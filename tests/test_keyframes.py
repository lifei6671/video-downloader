import subprocess
import sys
from fractions import Fraction
from pathlib import Path

import pytest
from PIL import Image
import keyframes
from local_tools import installed_tool


@pytest.fixture
def movie(tmp_path):
    ffmpeg = installed_tool('ffmpeg')
    if not ffmpeg:
        pytest.skip('FFmpeg unavailable')
    path = tmp_path / '源视频 中文😀.mp4'
    subprocess.run([ffmpeg, '-nostdin', '-v', 'error', '-f', 'lavfi', '-i', 'color=c=red:s=160x90:r=10:d=1',
                    '-f', 'lavfi', '-i', 'color=c=blue:s=160x90:r=10:d=1', '-filter_complex', '[0:v][1:v]concat=n=2:v=1:a=0',
                    '-c:v', 'libx264', '-g', '50', '-pix_fmt', 'yuv420p', str(path)], check=True, timeout=20,
                   creationflags=0x08000000 if sys.platform == 'win32' else 0)
    return path


def extract(source, stage, points, end=None):
    stage.mkdir()
    return keyframes.extract(dict(source=str(source.resolve()), staging_dir=str(stage.resolve()), positions_ms=points, end_ms=end), lambda *a, **k: None)


def test_real_samples_preserve_pts_and_decode_the_requested_nonkeyframes(movie, tmp_path):
    before = movie.read_bytes()
    stage = tmp_path / 'frames'
    result = extract(movie, stage, [350, 1050], 1500)['frames']
    actual = [Fraction(f['pts'] * f['time_base_num'] * 1000, f['time_base_den']) for f in result]
    assert actual == [400, 1100]
    assert [f['requested_ms'] for f in result] == [350, 1050]
    with Image.open(stage / result[0]['filename']) as image:
        red, green, blue = image.convert('RGB').getpixel((80, 45))
        assert red > 200 and blue < 20
    with Image.open(stage / result[1]['filename']) as image:
        red, green, blue = image.convert('RGB').getpixel((80, 45))
        assert blue > 200 and red < 20
    assert movie.read_bytes() == before


def test_positive_pts_uses_absolute_timestamps_and_rejects_head_tail(movie, tmp_path):
    shifted = tmp_path / '正PTS.mp4'
    subprocess.run([installed_tool('ffmpeg'), '-nostdin', '-v', 'error', '-i', str(movie), '-c', 'copy', '-output_ts_offset', '5', str(shifted)], check=True, timeout=20,
                   creationflags=0x08000000 if sys.platform == 'win32' else 0)
    result = extract(shifted, tmp_path / 'valid', [5350, 6050], 6500)['frames']
    assert [Fraction(f['pts'] * f['time_base_num'] * 1000, f['time_base_den']) for f in result] == [5400, 6100]
    for index, points in enumerate([[200], [9000]]):
        with pytest.raises(keyframes.KeyframeError, match='KEYFRAME_RANGE_UNAVAILABLE'):
            extract(shifted, tmp_path / f'invalid-{index}', points)


def test_black_frame_is_preserved_and_portrait_is_not_stretched(tmp_path):
    ffmpeg = installed_tool('ffmpeg')
    if not ffmpeg:
        pytest.skip('FFmpeg unavailable')
    source = tmp_path / 'black.mp4'
    subprocess.run([ffmpeg, '-nostdin', '-v', 'error', '-f', 'lavfi', '-i', 'color=c=black:s=90x160:r=10:d=1', '-c:v', 'mpeg4', str(source)], check=True, timeout=20,
                   creationflags=0x08000000 if sys.platform == 'win32' else 0)
    frame = extract(source, tmp_path / 'frames', [500])['frames'][0]
    assert (frame['width'], frame['height'], frame['black']) == (90, 160, True)


@pytest.mark.parametrize('sar,expected', [('2/1', (320, 90)), ('1/2', (80, 90))])
def test_non_square_pixels_preserve_display_aspect(tmp_path, sar, expected):
    ffmpeg = installed_tool('ffmpeg')
    if not ffmpeg:
        pytest.skip('FFmpeg unavailable')
    source = tmp_path / 'sar.mp4'
    subprocess.run([ffmpeg, '-nostdin', '-v', 'error', '-f', 'lavfi', '-i', 'color=c=blue:s=160x90:r=10:d=1',
                    '-vf', 'setsar=' + sar, '-c:v', 'mpeg4', str(source)], check=True, timeout=20,
                   creationflags=0x08000000 if sys.platform == 'win32' else 0)
    frame = extract(source, tmp_path / 'frames', [500])['frames'][0]
    assert (frame['width'], frame['height']) == expected


@pytest.mark.parametrize('points,end', [([], None), ([True], None), ([-1], None), ([1.2], None), ([0, 0], None), ([2, 1], None), ([0], 0), ([0], True), (list(range(10)), None)])
def test_invalid_ranges_do_not_start_a_process(tmp_path, monkeypatch, points, end):
    source = tmp_path / 'x.mp4'; source.write_bytes(b'x')
    monkeypatch.setattr(keyframes, 'installed_tool', lambda _: pytest.fail('invalid range must not start tools'))
    with pytest.raises(keyframes.KeyframeError, match='KEYFRAME_RANGE_INVALID'):
        extract(source, tmp_path / 'frames', points, end)


def test_missing_tool_source_and_decode_failures_are_explicit(movie, tmp_path, monkeypatch):
    with pytest.raises(keyframes.KeyframeError, match='FILE_MISSING'):
        extract(tmp_path / 'missing.mp4', tmp_path / 'missing', [0])
    original = keyframes.installed_tool
    monkeypatch.setattr(keyframes, 'installed_tool', lambda _: None)
    with pytest.raises(keyframes.KeyframeError, match='FFMPEG_UNAVAILABLE'):
        extract(movie, tmp_path / 'dependency', [0])
    monkeypatch.setattr(keyframes, 'installed_tool', original)
    corrupt = tmp_path / 'corrupt.mp4'; corrupt.write_bytes(b'not video')
    with pytest.raises(keyframes.KeyframeError, match='KEYFRAME_FAILED'):
        extract(corrupt, tmp_path / 'decode', [0])
    with pytest.raises(keyframes.KeyframeError, match='KEYFRAME_RANGE_UNAVAILABLE'):
        extract(movie, tmp_path / 'narrow', [350], 375)


def test_timeout_is_explicit_and_diagnostics_never_become_protocol_text(movie, tmp_path, monkeypatch):
    ffmpeg = installed_tool('ffmpeg')
    monkeypatch.setattr(keyframes, 'installed_tool', lambda _: ffmpeg)
    def timeout(*args, **kwargs):
        raise subprocess.TimeoutExpired(args[0], 20)
    monkeypatch.setattr(keyframes.subprocess, 'run', timeout)
    with pytest.raises(keyframes.KeyframeError, match='TIMEOUT'):
        extract(movie, tmp_path / 'timeout', [0])
