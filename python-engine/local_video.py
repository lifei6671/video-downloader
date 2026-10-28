"""Probe a user-selected local container without network access or modification."""
import json
import math
import subprocess
import sys
from pathlib import Path

from local_tools import installed_tool


class ProbeError(Exception):
    pass


def probe(params):
    try:
        source = Path(params['source'])
        stage = Path(params['staging_dir'])
        if (not source.is_absolute() or not source.is_file() or source.stat().st_size == 0
                or source.suffix.lower() not in ('.mp4', '.mov', '.mkv', '.webm')
                or not stage.is_absolute() or not stage.is_dir() or stage.is_symlink()):
            raise ProbeError('LOCAL_VIDEO_INVALID')
        ffprobe = installed_tool('ffprobe')
        if not ffprobe:
            raise ProbeError('FFMPEG_UNAVAILABLE')
        output = stage / 'probe.json'
        with output.open('xb') as stream:
            result = subprocess.run([
                ffprobe, '-v', 'error', '-protocol_whitelist', 'file',
                '-format_whitelist', 'mov,matroska,webm',
                '-show_entries', 'format=duration:stream=codec_type,width,height:stream_disposition=attached_pic',
                '-of', 'json', str(source),
            ], stdin=subprocess.DEVNULL, stdout=stream, stderr=subprocess.DEVNULL,
                timeout=20, creationflags=0x08000000 if sys.platform == 'win32' else 0)
        if result.returncode or output.stat().st_size > 1024 * 1024:
            raise ProbeError('LOCAL_VIDEO_INVALID')
        data = json.loads(output.read_text(encoding='utf-8'))
        video = next((v for v in data.get('streams', []) if v.get('codec_type') == 'video'
                      and not v.get('disposition', {}).get('attached_pic')), None)
        if (not video or type(video.get('width')) is not int or type(video.get('height')) is not int
                or not 0 < video['width'] <= 2**32 - 1 or not 0 < video['height'] <= 2**32 - 1):
            raise ProbeError('LOCAL_VIDEO_INVALID')
        duration = data.get('format', {}).get('duration')
        milliseconds = None
        if duration not in (None, 'N/A'):
            duration = float(duration)
            if not math.isfinite(duration) or duration <= 0 or duration * 1000 > 2**63 - 1:
                raise ProbeError('LOCAL_VIDEO_INVALID')
            milliseconds = max(1, round(duration * 1000))
        return dict(width=video['width'], height=video['height'], duration_ms=milliseconds)
    except subprocess.TimeoutExpired:
        raise ProbeError('TIMEOUT') from None
    except (KeyError, TypeError, ValueError, OSError, StopIteration):
        raise ProbeError('LOCAL_VIDEO_INVALID') from None
