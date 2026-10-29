import subprocess
import sys
import wave
from pathlib import Path

import pytest
import audio_excerpt
from local_tools import installed_tool


@pytest.fixture
def movie(tmp_path):
    ffmpeg = installed_tool('ffmpeg')
    if not ffmpeg:
        pytest.skip('FFmpeg unavailable')
    path = tmp_path / '短音频 中文.mp4'
    subprocess.run([ffmpeg, '-nostdin', '-v', 'error', '-f', 'lavfi', '-i', 'color=s=64x48:r=10:d=18',
                    '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000:duration=18',
                    '-c:v', 'libx264', '-c:a', 'aac', '-shortest', str(path)], check=True, timeout=20,
                   creationflags=0x08000000 if sys.platform == 'win32' else 0)
    return path


def extract(source, stage, start, end):
    stage.mkdir()
    return audio_excerpt.extract(dict(source=str(source.resolve()), staging_dir=str(stage.resolve()), start_ms=start, end_ms=end))


@pytest.mark.parametrize('start,end', [(0, 15000), (500, 1350), (12000, 17000)])
def test_real_wav_format_duration_and_requested_media_clock(movie, tmp_path, start, end):
    before = movie.read_bytes()
    stage = tmp_path / 'audio'
    result = extract(movie, stage, start, end)
    with wave.open(str(stage / result['filename']), 'rb') as audio:
        assert (audio.getframerate(), audio.getnchannels(), audio.getsampwidth()) == (16000, 1, 2)
        assert audio.getnframes() == (end - start) * 16
        assert any(audio.readframes(audio.getnframes()))
    assert result['start_sample'] == start * 16
    assert (stage / 'excerpt.wav').stat().st_size <= audio_excerpt.MAX_WAV_BYTES
    assert movie.read_bytes() == before


def test_positive_pts_reports_actual_audio_range(movie, tmp_path):
    path = tmp_path / 'offset.mp4'
    subprocess.run([installed_tool('ffmpeg'), '-nostdin', '-v', 'error', '-i', str(movie), '-c', 'copy',
                    '-output_ts_offset', '1.5', str(path)], check=True, timeout=20,
                   creationflags=0x08000000 if sys.platform == 'win32' else 0)
    result = extract(path, tmp_path / 'offset', 0, 2000)
    assert 23000 <= result['start_sample'] <= 24000  # AAC priming can precede 1500 ms.
    assert result['start_sample'] + result['frames'] == 32000


def test_no_audio_and_unavailable_range_fail_explicitly(movie, tmp_path):
    path = tmp_path / 'silent.mp4'
    subprocess.run([installed_tool('ffmpeg'), '-nostdin', '-v', 'error', '-i', str(movie), '-an', '-c:v', 'copy', str(path)],
                   check=True, timeout=20, creationflags=0x08000000 if sys.platform == 'win32' else 0)
    with pytest.raises(audio_excerpt.AudioError, match='AUDIO_NO_TRACK'):
        extract(path, tmp_path / 'silent', 0, 1000)
    with pytest.raises(audio_excerpt.AudioError, match='AUDIO_RANGE_UNAVAILABLE'):
        extract(movie, tmp_path / 'beyond', 19000, 20000)


@pytest.mark.parametrize('start,end', [(0, 15001), (10, 10), (-1, 10), (True, 10), (0, 2**53)])
def test_invalid_ranges_are_rejected(movie, tmp_path, start, end):
    with pytest.raises(audio_excerpt.AudioError, match='INVALID_INPUT'):
        extract(movie, tmp_path / 'invalid', start, end)
