import json
import subprocess
import sys
from pathlib import Path

import pytest

import engine
import local_video


@pytest.fixture
def media(tmp_path):
    source = tmp_path / '本地视频.mp4'
    source.write_bytes(b'original local media')
    stage = tmp_path / 'stage'
    stage.mkdir()
    return dict(source=str(source), staging_dir=str(stage))


def mock_probe(monkeypatch, data, code=0):
    monkeypatch.setattr(local_video, 'installed_tool', lambda _: 'ffprobe')
    def run(args, **kwargs):
        assert args[args.index('-protocol_whitelist') + 1] == 'file'
        assert args[args.index('-format_whitelist') + 1] == 'mov,matroska,webm'
        assert kwargs['timeout'] == 20 and kwargs['stdin'] == subprocess.DEVNULL
        kwargs['stdout'].write(json.dumps(data).encode())
        return subprocess.CompletedProcess(args, code)
    monkeypatch.setattr(local_video.subprocess, 'run', run)


def test_probe_returns_only_verified_metadata_and_does_not_modify_video(monkeypatch, media):
    mock_probe(monkeypatch, dict(streams=[dict(codec_type='video', width=1280, height=720)],
                                 format=dict(duration='3.125')))
    monkeypatch.setattr(engine.runtime_dependencies, 'configure', lambda _: None)
    events = []
    engine.operate('probe_local_video', media, lambda event, **data: events.append((event, data)))
    assert events == [('local_video_probed', dict(width=1280, height=720, duration_ms=3125))]
    assert Path(media['source']).read_bytes() == b'original local media'


@pytest.mark.parametrize('data', [
    dict(streams=[]),
    dict(streams=[dict(codec_type='audio')]),
    dict(streams=[dict(codec_type='video', width=10, height=10, disposition=dict(attached_pic=1))]),
    dict(streams=[dict(codec_type='video', width=True, height=10)]),
    dict(streams=[dict(codec_type='video', width=0, height=10)]),
    dict(streams=[dict(codec_type='video', width=10, height=10)], format=dict(duration='nan')),
    dict(streams=[dict(codec_type='video', width=10, height=10)], format=dict(duration='-1')),
])
def test_invalid_media_is_rejected(monkeypatch, media, data):
    mock_probe(monkeypatch, data)
    with pytest.raises(local_video.ProbeError, match='LOCAL_VIDEO_INVALID'):
        local_video.probe(media)


def test_unknown_duration_remains_unknown(monkeypatch, media):
    mock_probe(monkeypatch, dict(streams=[dict(codec_type='video', width=100, height=200)]))
    assert local_video.probe(media)['duration_ms'] is None


@pytest.mark.parametrize('mode', ['missing', 'failed', 'timeout', 'malformed'])
def test_tool_errors_are_explicit(monkeypatch, media, mode):
    monkeypatch.setattr(local_video, 'installed_tool', lambda _: None if mode == 'missing' else 'ffprobe')
    def run(args, **kwargs):
        if mode == 'timeout':
            raise subprocess.TimeoutExpired(args, 20)
        kwargs['stdout'].write(b'not-json' if mode == 'malformed' else b'{}')
        return subprocess.CompletedProcess(args, 1 if mode == 'failed' else 0)
    monkeypatch.setattr(local_video.subprocess, 'run', run)
    expected = 'FFMPEG_UNAVAILABLE' if mode == 'missing' else 'TIMEOUT' if mode == 'timeout' else 'LOCAL_VIDEO_INVALID'
    with pytest.raises(local_video.ProbeError, match=expected):
        local_video.probe(media)


def test_playlist_and_relative_inputs_do_not_start_tools(monkeypatch, media, tmp_path):
    playlist = tmp_path / 'video.m3u8'
    playlist.write_text('https://example.invalid/video')
    monkeypatch.setattr(local_video, 'installed_tool', lambda _: pytest.fail('must not launch tools'))
    for source in [str(playlist), 'relative.mp4', str(tmp_path), 'https://example.invalid/video.mp4']:
        with pytest.raises(local_video.ProbeError, match='LOCAL_VIDEO_INVALID'):
            local_video.probe({**media, 'source': source})


def test_real_local_video_and_corrupt_container(tmp_path):
    from local_tools import installed_tool
    ffmpeg = installed_tool('ffmpeg')
    ffprobe = installed_tool('ffprobe')
    if not ffmpeg or not ffprobe:
        pytest.skip('No local FFmpeg/FFprobe')
    source = tmp_path / 'sample.mp4'
    subprocess.run([ffmpeg, '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
                    'color=c=blue:s=64x48:r=10', '-t', '0.2', '-an', '-c:v', 'mpeg4', str(source)],
                   check=True, timeout=15, creationflags=0x08000000 if sys.platform == 'win32' else 0)
    original = source.read_bytes()
    stage = tmp_path / 'stage'
    stage.mkdir()
    result = local_video.probe(dict(source=str(source), staging_dir=str(stage)))
    assert result['width'] == 64 and result['height'] == 48 and result['duration_ms'] > 0
    assert source.read_bytes() == original
    (stage / 'probe.json').unlink()
    source.write_bytes(b'not a video')
    with pytest.raises(local_video.ProbeError, match='LOCAL_VIDEO_INVALID'):
        local_video.probe(dict(source=str(source), staging_dir=str(stage)))
