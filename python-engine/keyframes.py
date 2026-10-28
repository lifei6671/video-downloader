"""Decode requested presentation-time samples from a registered local container."""
import re
import subprocess
import sys
from fractions import Fraction
from pathlib import Path

from PIL import Image
from local_tools import installed_tool

MAX_IMAGE_BYTES = 8 * 1024 * 1024
MAX_EDGE = 1280


class KeyframeError(Exception):
    pass


def _decode(ffmpeg, source, stage, label, requested, output=None):
    log = stage / f'{label}.log'
    seek = ['-seek_timestamp', '1', '-noaccurate_seek', '-ss', f'{requested / 1000:.3f}'] if output else []
    filters = f"select='gte(t,{requested / 1000:.3f})',showinfo"
    if output:
        filters += (f",scale=w='max(1,trunc(min(iw*if(gt(sar,0),sar,1),min({MAX_EDGE},{MAX_EDGE}*if(gt(dar,0),dar,a)))))':"
                    "h='max(1,trunc(ow/if(gt(dar,0),dar,a)))',setsar=1")
    with log.open('xb') as diagnostic:
        args = [ffmpeg, '-nostdin', '-hide_banner', '-nostats', '-loglevel', 'info', '-n',
                '-copyts', *seek, '-protocol_whitelist', 'file', '-format_whitelist', 'mov,matroska,webm',
                '-i', str(source), '-map', '0:V:0', '-an', '-sn', '-dn', '-vf', filters,
                '-frames:v', '1', '-fps_mode', 'passthrough']
        args += ['-c:v', 'png', '-threads', '1', str(output)] if output else ['-f', 'null', '-']
        result = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                stderr=diagnostic, timeout=20,
                                creationflags=0x08000000 if sys.platform == 'win32' else 0)
    if result.returncode or log.stat().st_size > 1024 * 1024:
        raise KeyframeError('KEYFRAME_FAILED')
    text = log.read_text(encoding='utf-8', errors='replace')
    time_base = re.search(r'config in time_base:\s*(\d+)/(\d+)', text)
    timestamp = re.search(r'\bn:\s*0\s+pts:\s*(-?\d+)\s+pts_time:', text)
    if not time_base or not timestamp:
        raise KeyframeError('KEYFRAME_RANGE_UNAVAILABLE')
    pts, numerator, denominator = int(timestamp[1]), int(time_base[1]), int(time_base[2])
    if pts < 0 or numerator <= 0 or denominator <= 0:
        raise KeyframeError('KEYFRAME_FAILED')
    return pts, numerator, denominator


def extract(params, emit):
    try:
        source, stage = Path(params['source']), Path(params['staging_dir'])
        points, end = params['positions_ms'], params.get('end_ms')
        if (not source.is_absolute() or source.suffix.lower() not in ('.mp4', '.mov', '.mkv', '.webm')
                or not stage.is_absolute() or not stage.is_dir() or stage.is_symlink()
                or not isinstance(points, list) or not 1 <= len(points) <= 9
                or any(type(t) is not int or not 0 <= t <= 2**53 - 1 for t in points)
                or points != sorted(set(points))
                or end is not None and (type(end) is not int or not points[-1] < end <= 2**53 - 1)):
            raise KeyframeError('KEYFRAME_RANGE_INVALID')
        if not source.is_file():
            raise KeyframeError('FILE_MISSING')
        ffmpeg = installed_tool('ffmpeg')
        if not ffmpeg:
            raise KeyframeError('FFMPEG_UNAVAILABLE')
        first, num, den = _decode(ffmpeg, source, stage, 'first', 0)
        if Fraction(first * num * 1000, den) > points[0]:
            raise KeyframeError('KEYFRAME_RANGE_UNAVAILABLE')
        frames = []
        for index, requested in enumerate(points):
            filename = f'frame-{index:03}.png'
            output = stage / filename
            # Keep pre-roll timestamps; select discards frames before the target.
            # Accurate input seeking offsets positive-start MP4 timestamps on some FFmpeg versions.
            pts, numerator, denominator = _decode(ffmpeg, source, stage, f'frame-{index:03}', requested, output)
            if not output.is_file():
                raise KeyframeError('KEYFRAME_RANGE_UNAVAILABLE')
            if output.is_symlink() or not 0 < output.stat().st_size <= MAX_IMAGE_BYTES:
                raise KeyframeError('KEYFRAME_FAILED')
            actual = Fraction(pts * numerator * 1000, denominator)
            if actual < requested or end is not None and actual >= end:
                raise KeyframeError('KEYFRAME_RANGE_UNAVAILABLE')
            with Image.open(output) as image:
                image.load()
                if image.format != 'PNG' or not 0 < image.width <= MAX_EDGE or not 0 < image.height <= MAX_EDGE:
                    raise KeyframeError('KEYFRAME_FAILED')
                histogram = image.convert('L').histogram()
                black = sum(histogram[:24]) * 100 >= image.width * image.height * 98
                width, height = image.size
            frames.append(dict(filename=filename, requested_ms=requested, pts=pts,
                               time_base_num=numerator, time_base_den=denominator,
                               width=width, height=height, black=black, bytes=output.stat().st_size))
            emit('keyframe_progress', completed=len(frames), total=len(points))
        return dict(frames=frames)
    except subprocess.TimeoutExpired:
        raise KeyframeError('TIMEOUT') from None
    except (KeyError, TypeError, ValueError, OSError, ZeroDivisionError):
        raise KeyframeError('KEYFRAME_FAILED') from None
