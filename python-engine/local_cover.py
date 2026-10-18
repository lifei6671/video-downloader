"""Optional cover extraction from a complete, task-owned local video."""
import subprocess
import sys
from pathlib import Path

from local_tools import installed_tool


def extract(params):
    try:
        stage = Path(params['staging_dir'])
        name = params['filename']
        if (not stage.is_absolute() or not stage.is_dir() or stage.is_symlink()
                or name not in ('video.mp4', 'video.mov', 'video.mkv', 'video.webm')):
            raise ValueError()
        source = stage / name
        if source.is_symlink() or not source.is_file() or source.stat().st_size == 0:
            raise ValueError()
        ffmpeg = installed_tool('ffmpeg')
        if not ffmpeg:
            return dict(cover_error='COVER_UNAVAILABLE')
        cover = stage / 'cover'
        # No network protocols or playlist demuxers: decode only the completed
        # local container. The original video is opened as input and never edited.
        result = subprocess.run([
            ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'error', '-n',
            '-protocol_whitelist', 'file', '-format_whitelist', 'mov,matroska,webm',
            '-t', '30', '-i', str(source), '-map', '0:v:0', '-frames:v', '1',
            # Keep the first frame (including sub-second clips), then sample at
            # most twice per second. Reject frames with >=98% near-black pixels.
            '-vf', "select='isnan(prev_selected_t)+gte(t-prev_selected_t,0.5)',"
            "format=yuv420p,blackframe=amount=0:threshold=24,"
            "metadata=mode=select:key=lavfi.blackframe.pblack:value=98:function=less,"
            "scale=w='min(1280,iw)':h='min(1280,ih)':force_original_aspect_ratio=decrease",
            '-c:v', 'mjpeg', '-q:v', '3', '-f', 'image2', '-update', '1', str(cover),
        ], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            timeout=10, creationflags=0x08000000 if sys.platform == 'win32' else 0)
        size = cover.stat().st_size
        if result.returncode or cover.is_symlink() or not 0 < size <= 10 * 1024 * 1024:
            raise ValueError()
        with cover.open('rb') as image:
            if image.read(3) != b'\xff\xd8\xff':
                raise ValueError()
        return dict(cover_filename='cover', cover_bytes=size)
    except (KeyError, TypeError, ValueError, OSError, subprocess.SubprocessError):
        # Covers are optional. Partial output stays in the owned staging folder
        # and is discarded by the caller together with the operation's staging.
        return dict(cover_error='COVER_UNAVAILABLE')
