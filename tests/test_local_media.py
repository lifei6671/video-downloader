import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

import engine
import local_media
import local_tools


def test_probe_dispatch_uses_local_tools_and_never_resolves_a_platform(monkeypatch, tmp_path):
    source = tmp_path / 'original.mov'
    source.write_bytes(b'original fixture')
    events = []
    monkeypatch.setattr(engine, 'parse_input', lambda *_: pytest.fail('not a platform request'))
    monkeypatch.setattr(engine.runtime_dependencies, 'configure', lambda root: events.append(root))
    monkeypatch.setattr(local_media, 'installed_tool', lambda tool: 'ffprobe' if tool == 'ffprobe' else None)

    def run(args, **kwargs):
        assert args[args.index('-protocol_whitelist') + 1] == 'file'
        assert args[-1] == str(source)
        assert kwargs['timeout'] == 20
        return subprocess.CompletedProcess(args, 0, json.dumps({
            'streams': [{'codec_type': 'video', 'width': 1920, 'height': 1080, 'duration': 'N/A', 'side_data_list': [{'rotation': -90}]}],
            'format': {'duration': '2.5'},
        }).encode())

    monkeypatch.setattr(local_media.subprocess, 'run', run)
    engine.operate('probe_video', {'source': str(source), 'runtime_directory': 'runtime'},
                   lambda event, **data: events.append((event, data)))
    assert events == ['runtime', ('video_metadata', dict(width=1080, height=1920, duration_ms=2500))]
    assert source.read_bytes() == b'original fixture'


@pytest.mark.parametrize('result', [
    {'streams': [], 'format': {'duration': '1'}},
    {'streams': [{'codec_type': 'video', 'width': 0, 'height': 720}], 'format': {'duration': '1'}},
    {'streams': [{'codec_type': 'video', 'width': 1280, 'height': 720}], 'format': {'duration': 'nan'}},
    {'streams': [{'codec_type': 'video', 'width': 1280, 'height': 720, 'disposition': {'attached_pic': 1}}], 'format': {'duration': '1'}},
])
def test_missing_or_invalid_attributes_are_not_invented(monkeypatch, tmp_path, result):
    source = tmp_path / 'clip.mkv'
    source.write_bytes(b'fixture')
    monkeypatch.setattr(local_media, 'installed_tool', lambda _: 'ffprobe')
    monkeypatch.setattr(local_media.subprocess, 'run', lambda *a, **k: subprocess.CompletedProcess(a, 0, json.dumps(result).encode()))
    with pytest.raises(local_media.MediaInfoError, match='^VIDEO_METADATA_UNAVAILABLE$'):
        local_media.probe({'source': str(source)})


def test_missing_ffprobe_is_an_actionable_dependency_error(monkeypatch, tmp_path):
    source = tmp_path / 'clip.mov'
    source.write_bytes(b'fixture')
    monkeypatch.setattr(local_media, 'installed_tool', lambda _: None)
    with pytest.raises(local_media.MediaInfoError, match='^FFPROBE_UNAVAILABLE$'):
        local_media.probe({'source': str(source)})


@pytest.mark.parametrize('extension,codec', [('mp4', 'libx264'), ('mov', 'libx264'), ('mkv', 'libx264'), ('webm', 'libvpx')])
def test_real_probe_preserves_original_video_bytes(tmp_path, extension, codec):
    ffmpeg, ffprobe = local_tools.installed_tool('ffmpeg'), local_tools.installed_tool('ffprobe')
    if not ffmpeg or not ffprobe:
        pytest.skip('existing FFmpeg/FFprobe unavailable')
    source = tmp_path / ('original.' + extension)
    subprocess.run([ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'testsrc=size=32x24:rate=5',
                    '-t', '0.6', '-an', '-c:v', codec, '-pix_fmt', 'yuv420p', str(source)],
                   check=True, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                   timeout=20, creationflags=0x08000000 if sys.platform == 'win32' else 0)
    before = hashlib.sha256(source.read_bytes()).hexdigest()
    result = local_media.probe({'source': str(source)})
    assert result['width'] == 32 and result['height'] == 24 and 400 <= result['duration_ms'] <= 1000
    assert hashlib.sha256(source.read_bytes()).hexdigest() == before
    if extension == 'mov':
        # Verify the real JSON-lines entry point, version handshake and terminal result.
        root = Path(__file__).resolve().parents[1]
        env = dict(os.environ, PYTHONPATH=os.pathsep.join([str(root / 'python-engine'), str(root / 'vendor/dtk/upstream/src')]))
        with subprocess.Popen([sys.executable, str(root / 'python-engine/bridge.py')], stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env,
                              creationflags=0x08000000 if sys.platform == 'win32' else 0) as process:
            try:
                ready = json.loads(process.stdout.readline())
                assert ready['engine_version'] == '0.14.5'
                process.stdin.write((json.dumps({'protocol_version': 3, 'id': 'local-probe', 'method': 'probe_video',
                                                'params': {'source': str(source)}}) + '\n').encode())
                process.stdin.flush()
                metadata = json.loads(process.stdout.readline())
                assert metadata['type'] == 'video_metadata' and metadata['width'] == 32
                assert process.wait(timeout=30) == 0 and process.stdout.read() == b''
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
