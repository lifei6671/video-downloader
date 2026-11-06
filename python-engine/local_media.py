"""Read video attributes from a complete local file without rewriting it."""
import json
import math
import subprocess
import sys
from pathlib import Path

from local_tools import installed_tool


class MediaInfoError(Exception):
    pass


def probe(params):
    source = Path(params.get('source', ''))
    if not source.is_absolute() or source.is_symlink() or not source.is_file():
        raise MediaInfoError('VIDEO_METADATA_UNAVAILABLE')
    ffprobe = installed_tool('ffprobe')
    if not ffprobe:
        raise MediaInfoError('FFPROBE_UNAVAILABLE')
    try:
        result = subprocess.run([
            ffprobe, '-v', 'error', '-protocol_whitelist', 'file',
            '-show_entries', 'stream=codec_type,width,height,duration:stream_disposition=attached_pic:stream_side_data=rotation:format=duration',
            '-of', 'json', '-i', str(source),
        ], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            timeout=20, creationflags=0x08000000 if sys.platform == 'win32' else 0)
        if result.returncode:
            raise ValueError()
        metadata = json.loads(result.stdout)
        for stream in metadata.get('streams', []):
            if stream.get('codec_type') != 'video' or stream.get('disposition', {}).get('attached_pic'):
                continue
            width, height = stream.get('width'), stream.get('height')
            duration = stream.get('duration')
            if duration in (None, 'N/A'):
                duration = metadata.get('format', {}).get('duration')
            seconds = float(duration)
            if (type(width) is int and type(height) is int
                    and 0 < width <= 2147483647 and 0 < height <= 2147483647
                    and math.isfinite(seconds) and 0 < seconds <= (2**64 - 1) / 1000):
                rotation = next((item.get('rotation', 0) for item in stream.get('side_data_list', []) if 'rotation' in item), 0)
                if abs(round(float(rotation))) % 180 == 90:
                    width, height = height, width
                return dict(width=width, height=height, duration_ms=max(1, round(seconds * 1000)))
        raise ValueError()
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError):
        raise MediaInfoError('VIDEO_METADATA_UNAVAILABLE') from None
