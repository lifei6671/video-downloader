"""Decode an explicitly requested, bounded local audio range without persisting it."""
import re
import subprocess
import sys
import wave
from pathlib import Path

from local_tools import installed_tool

SAMPLE_RATE = 16000
MAX_WAV_BYTES = 15 * SAMPLE_RATE * 2 + 4096


class AudioError(Exception):
    pass


def extract(params):
    try:
        source, stage = Path(params['source']), Path(params['staging_dir'])
        start, end = params['start_ms'], params['end_ms']
        if (not source.is_absolute() or source.suffix.lower() not in ('.mp4', '.mov', '.mkv', '.webm')
                or not stage.is_absolute() or not stage.is_dir() or stage.is_symlink()
                or type(start) is not int or type(end) is not int
                or not 0 <= start < end <= 2**53 - 1 or end - start > 15000):
            raise AudioError('INVALID_INPUT')
        if not source.is_file():
            raise AudioError('FILE_MISSING')
        ffmpeg = installed_tool('ffmpeg')
        if not ffmpeg:
            raise AudioError('FFMPEG_UNAVAILABLE')
        output, log = stage / 'excerpt.wav', stage / 'decode.log'
        filters = f'atrim=start={start / 1000:.3f}:end={end / 1000:.3f},aresample={SAMPLE_RATE},ashowinfo,asetpts=PTS-STARTPTS'
        with log.open('xb') as diagnostic:
            result = subprocess.run([
                ffmpeg, '-nostdin', '-hide_banner', '-nostats', '-loglevel', 'info', '-n', '-copyts',
                '-seek_timestamp', '1', '-noaccurate_seek', '-ss', f'{start / 1000:.3f}',
                '-protocol_whitelist', 'file', '-format_whitelist', 'mov,matroska,webm', '-i', str(source),
                '-map', '0:a:0', '-vn', '-sn', '-dn', '-af', filters,
                '-t', f'{(end - start) / 1000:.3f}', '-ar', str(SAMPLE_RATE), '-ac', '1', '-c:a', 'pcm_s16le', str(output),
            ], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=diagnostic, timeout=20,
                creationflags=0x08000000 if sys.platform == 'win32' else 0)
        if log.stat().st_size > 1024 * 1024:
            raise AudioError('AUDIO_FAILED')
        text = log.read_text(encoding='utf-8', errors='replace')
        if result.returncode:
            if 'matches no streams' in text:
                raise AudioError('AUDIO_NO_TRACK')
            raise AudioError('AUDIO_FAILED')
        if not output.is_file() or output.is_symlink() or output.stat().st_size > MAX_WAV_BYTES:
            raise AudioError('AUDIO_FAILED')
        with wave.open(str(output), 'rb') as audio:
            frames = audio.getnframes()
            if audio.getnchannels() != 1 or audio.getsampwidth() != 2 or audio.getframerate() != SAMPLE_RATE:
                raise AudioError('AUDIO_FAILED')
        first = re.search(r'\bn:0\s+pts:(-?\d+)\s+pts_time:', text)
        if not first or not 0 < frames <= 15 * SAMPLE_RATE:
            raise AudioError('AUDIO_RANGE_UNAVAILABLE')
        start_sample = int(first[1])
        if start_sample < 0 or start_sample * 1000 < start * SAMPLE_RATE or (start_sample + frames) * 1000 > end * SAMPLE_RATE:
            raise AudioError('AUDIO_RANGE_UNAVAILABLE')
        return dict(filename='excerpt.wav', frames=frames, start_sample=start_sample, sample_rate=SAMPLE_RATE)
    except subprocess.TimeoutExpired:
        raise AudioError('AUDIO_TIMEOUT') from None
    except (KeyError, TypeError, ValueError, OSError, wave.Error):
        raise AudioError('AUDIO_FAILED') from None
