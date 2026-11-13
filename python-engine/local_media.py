"""Read video attributes from a complete local file without rewriting it."""
import json
import math
import subprocess
import sys
import time
from pathlib import Path

from local_tools import installed_tool


class MediaInfoError(Exception):
    pass


def display_dimensions(stream):
    width, height = stream.get('width'), stream.get('height')
    if not all(type(value) is int and 0 < value <= 2147483647 for value in (width, height)):
        return None
    rotation = next((item['rotation'] for item in stream.get('side_data_list', [])
                     if 'rotation' in item), 0)
    return (height, width) if abs(round(float(rotation))) % 180 == 90 else (width, height)


def probe(params):
    source = Path(params.get('source', ''))
    if not source.is_absolute() or source.is_symlink() or not source.is_file():
        raise MediaInfoError('VIDEO_METADATA_UNAVAILABLE')
    ffprobe = installed_tool('ffprobe')
    if not ffprobe:
        raise MediaInfoError('FFPROBE_UNAVAILABLE')
    validate_mp4 = params.get('validate_mp4', False)
    entries = ('stream=codec_type,width,height,duration:stream_disposition=attached_pic:'
               'stream_side_data=rotation:format=duration')
    sample = []
    if validate_mp4:
        entries += ',format_name:stream=codec_name'
        sample = ['-select_streams', 'v']
    try:
        result = subprocess.run([
            ffprobe, '-v', 'error', '-protocol_whitelist', 'file',
            *sample, '-show_entries', entries,
            '-of', 'json', '-i', str(source),
        ], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE if validate_mp4 else subprocess.DEVNULL,
            timeout=20, creationflags=0x08000000 if sys.platform == 'win32' else 0)
        if result.returncode or (validate_mp4 and result.stderr):
            raise ValueError()
        metadata = json.loads(result.stdout)
        if validate_mp4 and 'mp4' not in metadata.get('format', {}).get('format_name', '').split(','):
            raise ValueError()
        for stream in metadata.get('streams', []):
            if stream.get('codec_type') != 'video' or stream.get('disposition', {}).get('attached_pic'):
                continue
            expected = {'h264': 'h264', 'h265': 'hevc'}.get(params.get('expected_codec'))
            if validate_mp4 and expected and stream.get('codec_name') != expected:
                continue
            dimensions = display_dimensions(stream)
            duration = stream.get('duration')
            if duration in (None, 'N/A'):
                duration = metadata.get('format', {}).get('duration')
            seconds = float(duration)
            if dimensions and math.isfinite(seconds) and 0 < seconds <= (2**64 - 1) / 1000:
                width, height = dimensions
                return dict(width=width, height=height, duration_ms=max(1, round(seconds * 1000)))
        raise ValueError()
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError):
        raise MediaInfoError('INCOMPLETE_MEDIA' if validate_mp4 else 'VIDEO_METADATA_UNAVAILABLE') from None


def validate_download(source, ffmpeg, heartbeat):
    """Decode all video/audio without generating an output file or frame listings."""
    deadline = time.monotonic() + 1800
    try:
        with subprocess.Popen([
            ffmpeg, '-nostdin', '-v', 'error', '-xerror', '-protocol_whitelist', 'file',
            '-i', str(source), '-map', '0:V', '-map', '0:a?', '-f', 'null', '-',
        ], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=0x08000000 if sys.platform == 'win32' else 0) as process:
            try:
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise MediaInfoError('TIMEOUT')
                    try:
                        if process.wait(timeout=min(20, remaining)):
                            raise MediaInfoError('INCOMPLETE_MEDIA')
                        return
                    except subprocess.TimeoutExpired:
                        heartbeat()
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
    except OSError:
        raise MediaInfoError('INCOMPLETE_MEDIA') from None
