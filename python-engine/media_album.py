"""Preview and stage album media using the guarded yt-dlp transport."""
import hashlib
import math
from io import BytesIO

from PIL import Image
from yt_dlp.downloader.http import HttpFD
from yt_dlp.networking.exceptions import HTTPError

from author_profile import download_avatar


MAX_IMAGES = 64
MAX_IMAGE_BYTES = 32 * 1024 * 1024


def image_extension(body):
    if body.startswith(b'\xff\xd8\xff'):
        return 'jpeg'
    if body.startswith(b'\x89PNG\r\n\x1a\n'):
        return 'png'
    if body.startswith(b'RIFF') and body[8:12] == b'WEBP':
        return 'webp'
    if body.startswith((b'GIF87a', b'GIF89a')):
        return 'gif'
    if body[4:8] == b'ftyp':
        if body[8:12] in (b'avif', b'avis'):
            return 'avif'
        if body[8:12] in (b'heic', b'heix', b'hevc', b'hevx'):
            return 'heic'
    raise ValueError('INVALID_MEDIA_CONTENT')


def fill_image_size(choice, body):
    if choice['width'] and choice['height']:
        return
    try:
        with Image.open(BytesIO(body)) as image:
            width, height = image.size
            try:
                if image.getexif().get(274, 1) in (5, 6, 7, 8):
                    width, height = height, width
            except (OSError, SyntaxError, ValueError):
                pass  # Malformed EXIF does not invalidate the image dimensions.
    except (OSError, ValueError, Image.DecompressionBombError):
        return
    choice.update(width=width, height=height)


def preview_images(ydl, choices, stage):
    from engine import EngineError
    names = []
    for index, choice in enumerate(choices, 1):
        with ydl.urlopen(choice['hd_url']) as response:
            body = response.read(MAX_IMAGE_BYTES + 1)
        if len(body) > MAX_IMAGE_BYTES:
            raise EngineError('UNSUPPORTED_CONTENT', 'image_size')
        try:
            extension = image_extension(body)
        except ValueError:
            raise EngineError('INVALID_MEDIA_CONTENT', 'preview') from None
        if choice['url'] == choice['hd_url']:
            fill_image_size(choice, body)
        name = f'preview-{index:03d}.{extension}'
        (stage / name).write_bytes(body)
        names.append(name)
    return names


def download_images(ydl, choices, platform, video_id, resolver, stage, emit, *, metadata=None,
                    avatar_url=None, author_profile=None):
    from engine import EngineError
    album = stage / 'album'
    album.mkdir()
    files, downloaded = [], 0
    headers = {'User-Agent': ydl.params['http_headers']['User-Agent'],
               'Referer': 'https://www.' + platform + '.com/'}
    emit('progress', downloaded_bytes=0, total_bytes=None, speed_bps=None,
         completed_items=0, total_items=len(choices))
    for index, choice in enumerate(choices, 1):
        path = album / f'{index:03d}.image'

        def progress(data):
            if data.get('status') not in ('downloading', 'finished'):
                return
            count = data.get('downloaded_bytes', 0)
            if count > MAX_IMAGE_BYTES:
                raise EngineError('UNSUPPORTED_CONTENT', 'image_size')
            speed = data.get('speed')
            emit('progress', downloaded_bytes=downloaded + count, total_bytes=None,
                 speed_bps=speed if speed and math.isfinite(speed) else None,
                 completed_items=index - 1,
                 total_items=len(choices))

        # HTTP 404/410 can prove an original resource is unavailable; timeouts,
        # credentials and risk control must surface, not silently downgrade.
        url = choice['url']
        if choice['quality_source'] == 'original':
            try:
                with ydl.urlopen(url) as response:
                    response.read(1)
            except HTTPError as error:
                if error.status not in (404, 410):
                    raise
                url = choice['hd_url']
                choice['quality_source'] = 'high_definition'
                choice['width'], choice['height'] = choice['hd_width'], choice['hd_height']
        downloader = HttpFD(ydl, dict(ydl.params, max_filesize=MAX_IMAGE_BYTES))
        downloader.add_progress_hook(progress)
        success, _ = downloader.download(str(path), dict(url=url, http_headers=headers))
        if not success or not path.is_file():
            raise EngineError('IO_ERROR', 'image_download')
        body = path.read_bytes()
        if not body or len(body) > MAX_IMAGE_BYTES:
            raise EngineError('INVALID_MEDIA_CONTENT', 'image')
        try:
            extension = image_extension(body)
        except ValueError:
            raise EngineError('INVALID_MEDIA_CONTENT', 'image') from None
        fill_image_size(choice, body)
        final = path.with_suffix('.' + extension)
        path.rename(final)
        downloaded += len(body)
        files.append(dict(filename=final.name, bytes=len(body), sha256=hashlib.sha256(body).hexdigest(),
                          **{k: choice[k] for k in ('width', 'height', 'quality_source')}))
        if choice.get('video_url'):
            # Keep both originals. A clip failure must fail the album rather
            # than publish an apparently complete set of stills.
            clip = album / f'{index:03d}.mp4'
            clip_downloader = HttpFD(ydl, dict(ydl.params, max_filesize=MAX_IMAGE_BYTES))
            clip_downloader.add_progress_hook(progress)
            success, _ = clip_downloader.download(str(clip), dict(url=choice['video_url'], http_headers=headers))
            if not success or not clip.is_file():
                raise EngineError('IO_ERROR', 'live_photo_download')
            clip_body = clip.read_bytes()
            if (not 12 <= len(clip_body) <= MAX_IMAGE_BYTES
                    or clip_body[4:8] != b'ftyp'
                    or clip_body[8:12] in (b'avif', b'avis', b'heic', b'heix', b'hevc', b'hevx')):
                raise EngineError('INVALID_MEDIA_CONTENT', 'live_photo')
            files[-1]['video'] = dict(filename=clip.name, bytes=len(clip_body),
                                     sha256=hashlib.sha256(clip_body).hexdigest())
            downloaded += len(clip_body)
        emit('progress', downloaded_bytes=downloaded, total_bytes=None, speed_bps=None,
             completed_items=index, total_items=len(choices))
    emit('progress', downloaded_bytes=downloaded, total_bytes=downloaded, speed_bps=None,
         completed_items=len(files), total_items=len(files))
    emit('download_finished', platform=platform, video_id=video_id,
         kind='image_album', resolver=resolver, files=files, bytes=downloaded,
         **(download_avatar(ydl, avatar_url, stage) if avatar_url else {}),
         **({'author_profile': author_profile} if author_profile is not None else {}),
         **({'metadata': metadata} if metadata is not None else {}))
