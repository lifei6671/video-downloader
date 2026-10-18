import hashlib
import subprocess
from pathlib import Path

import pytest

import engine
import local_cover


@pytest.fixture
def stage(tmp_path):
    (tmp_path / 'video.mp4').write_bytes(b'complete local media')
    return dict(staging_dir=str(tmp_path), filename='video.mp4')


def test_extract_cover_dispatch_is_local_and_reuses_configured_runtime(monkeypatch, stage, tmp_path):
    monkeypatch.setattr(engine, 'parse_input', lambda *_: pytest.fail('no platform request'))
    monkeypatch.setattr(engine.runtime_dependencies, 'configure', lambda path: events.append(path))
    monkeypatch.setattr(local_cover, 'installed_tool', lambda tool: 'ffmpeg' if tool == 'ffmpeg' else None)
    events = []
    def run(args, **kwargs):
        assert args[args.index('-protocol_whitelist') + 1] == 'file'
        assert args[args.index('-format_whitelist') + 1] == 'mov,matroska,webm'
        assert args[args.index('-i') + 1] == str(tmp_path / 'video.mp4')
        assert kwargs['timeout'] == 10 and '-nostdin' in args
        Path(args[-1]).write_bytes(b'\xff\xd8\xffjpeg')
        return subprocess.CompletedProcess(args, 0)
    monkeypatch.setattr(local_cover.subprocess, 'run', run)
    original = (tmp_path / 'video.mp4').read_bytes()
    engine.operate('extract_cover', {**stage, 'runtime_directory': 'runtime'},
                   lambda event, **data: events.append((event, data)))
    assert events == ['runtime', ('cover_extracted', dict(cover_filename='cover', cover_bytes=7))]
    assert (tmp_path / 'video.mp4').read_bytes() == original


@pytest.mark.parametrize('mode', ['missing', 'error', 'timeout', 'empty', 'html', 'oversize', 'no_video'])
def test_optional_cover_failures(monkeypatch, stage, tmp_path, mode):
    monkeypatch.setattr(local_cover, 'installed_tool', lambda _: None if mode == 'missing' else 'ffmpeg')
    def run(args, **kwargs):
        if mode == 'timeout':
            raise subprocess.TimeoutExpired(args, 10)
        if mode == 'error':
            raise OSError('cannot execute')
        body = b'' if mode == 'empty' else b'<html>' if mode == 'html' else b'\xff\xd8\xffjpeg'
        if mode == 'oversize':
            body += b'x' * (10 * 1024 * 1024)
        Path(args[-1]).write_bytes(body)
        return subprocess.CompletedProcess(args, 1 if mode == 'no_video' else 0)
    monkeypatch.setattr(local_cover.subprocess, 'run', run)
    assert local_cover.extract(stage) == dict(cover_error='COVER_UNAVAILABLE')
    assert (tmp_path / 'video.mp4').read_bytes() == b'complete local media'


@pytest.mark.parametrize('filename', ['../outside.mp4', 'https://example.invalid/video.mp4', '/video.mp4', 'video.m3u8'])
def test_rejects_unowned_paths_and_playlists(monkeypatch, stage, filename):
    monkeypatch.setattr(local_cover, 'installed_tool', lambda _: pytest.fail('no process for invalid input'))
    assert local_cover.extract({**stage, 'filename': filename}) == dict(cover_error='COVER_UNAVAILABLE')


def test_real_ffmpeg_extracts_short_video_without_changing_original(tmp_path):
    ffmpeg = local_cover.installed_tool('ffmpeg')
    if not ffmpeg:
        pytest.skip('No runnable local FFmpeg')
    source = tmp_path / 'video.mp4'
    subprocess.run([ffmpeg, '-nostdin', '-loglevel', 'error', '-f', 'lavfi', '-i',
                    'color=c=red:s=320x240:r=25:d=0.08', '-c:v', 'mpeg4', str(source)],
                   check=True, timeout=15, capture_output=True)
    before = hashlib.sha256(source.read_bytes()).hexdigest()
    result = local_cover.extract(dict(staging_dir=str(tmp_path), filename='video.mp4'))
    assert result['cover_bytes'] == (tmp_path / 'cover').stat().st_size
    assert result['cover_filename'] == 'cover'
    decoded = subprocess.run([ffmpeg, '-nostdin', '-loglevel', 'error', '-i',
                              str(tmp_path / 'cover'), '-f', 'null', '-'],
                             capture_output=True, timeout=15)
    assert decoded.returncode == 0, decoded.stderr
    assert hashlib.sha256(source.read_bytes()).hexdigest() == before


@pytest.mark.parametrize('scene,expected_cover', [
    # Includes a fade-in; fixed first-frame extraction produced a black JPEG.
    ("color=c=red:s=320x240:r=25:d=3,fade=t=in:st=0:d=1.2", True),
    ("color=c=red:s=320x240:r=25:d=3,drawbox=color=black:t=fill:enable='lt(t,2)'", True),
    ('color=c=black:s=320x240:r=25:d=2', False),
    ('color=c=0x080808:s=320x240:r=25:d=2', False),
    # Black letterboxing must not hide a visible image in a dark scene.
    ('color=c=0x600000:s=320x120:r=25:d=1,pad=320:240:0:60:black', True),
    # The scan is bounded even if visible content starts after the limit.
    ("color=c=red:s=320x240:r=25:d=32,drawbox=color=black:t=fill:enable='lt(t,31)'", False),
])
def test_real_ffmpeg_skips_black_frames_and_limits_scan(tmp_path, scene, expected_cover):
    ffmpeg = local_cover.installed_tool('ffmpeg')
    if not ffmpeg:
        pytest.skip('No runnable local FFmpeg')
    source = tmp_path / 'video.mp4'
    subprocess.run([ffmpeg, '-nostdin', '-loglevel', 'error', '-f', 'lavfi', '-i',
                    scene, '-c:v', 'mpeg4', str(source)],
                   check=True, timeout=15, capture_output=True)
    before = hashlib.sha256(source.read_bytes()).hexdigest()
    result = local_cover.extract(dict(staging_dir=str(tmp_path), filename='video.mp4'))
    if expected_cover:
        assert result['cover_filename'] == 'cover'
        # Decode the actual JPEG and prove visible red pixels survived selection.
        decoded = subprocess.run([ffmpeg, '-nostdin', '-loglevel', 'error', '-i',
                                  str(tmp_path / 'cover'), '-vf', 'scale=16:16',
                                  '-frames:v', '1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
                                 capture_output=True, check=True, timeout=15)
        rgb = decoded.stdout
        assert len(rgb) == 16 * 16 * 3
        assert max(rgb[0::3]) > 60
        assert sum(rgb[0::3]) > sum(rgb[1::3]) + 1000
        assert result['cover_bytes'] == (tmp_path / 'cover').stat().st_size
    else:
        assert result == dict(cover_error='COVER_UNAVAILABLE')
        assert not (tmp_path / 'cover').exists()
    assert hashlib.sha256(source.read_bytes()).hexdigest() == before
