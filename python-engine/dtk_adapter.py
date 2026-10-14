"""Pinned DTK parameter/signature/parser slice; all IO uses the existing guard."""
import asyncio
import hashlib
import json
import math
from pathlib import Path
from types import SimpleNamespace

import structlog
from yt_dlp.downloader.http import HttpFD
from yt_dlp.networking import Request
from yt_dlp.networking.exceptions import HTTPError


def discard_log(_logger, _method, _event):
    # stdout belongs to the JSONL bridge; upstream free text is never forwarded.
    raise structlog.DropEvent


structlog.configure(processors=[discard_log])

from dtk.core.errors import DtkError
from dtk.platforms.douyin.adapter import ADAPTER as DOUYIN
from dtk.platforms.tiktok.adapter import ADAPTER as TIKTOK
from dtk.signing.base import RequestSpec, SigningSession, StaticFingerprint
from dtk.signing.native.signer import NativeSigner
from dtk._desktop_revision import COMMIT, VERSION

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


def image_choices(content):
    """Known Douyin original_images shape, then the upstream full-size image list.

    Unknown present original shapes fail instead of silently claiming originals.
    Never choose download_url_list (watermark), thumbnails or rewrite CDN URLs.
    """
    from engine import EngineError
    images = content.media.images
    if not 1 <= len(images) <= MAX_IMAGES:
        raise EngineError('UNSUPPORTED_CONTENT', 'images')
    originals = (content.raw or {}).get('original_images') if str(content.platform) == 'douyin' else None
    if originals is not None and (not isinstance(originals, list) or len(originals) != len(images)):
        raise EngineError('UPSTREAM_CHANGED', 'original_images')
    result = []
    for index, image in enumerate(images):
        original = originals[index] if originals is not None else None
        if original is not None and not isinstance(original, dict):
            raise EngineError('UPSTREAM_CHANGED', 'original_image')
        urls = original.get('url_list') if original else None
        if urls is not None and (not isinstance(urls, list) or not urls
                                 or not all(isinstance(u, str) and u.startswith('https://') for u in urls)):
            raise EngineError('UPSTREAM_CHANGED', 'original_image')
        if original and urls is None:
            raise EngineError('UPSTREAM_CHANGED', 'original_image')
        result.append(dict(url=urls[0] if urls else image.url, hd_url=image.url,
                           width=original.get('width', image.width) if urls else image.width,
                           height=original.get('height', image.height) if urls else image.height,
                           quality_source='original' if urls else 'high_definition', hd_width=image.width, hd_height=image.height))
        for field in ('width', 'height', 'hd_width', 'hd_height'):
            value = result[-1][field]
            result[-1][field] = value if type(value) is int and 0 < value <= 4294967295 else None
    return result


def extract(ydl, platform, video_id, ua, cookies, proxy):
    from datetime import UTC, datetime
    from engine import EngineError
    adapter = DOUYIN if platform == 'douyin' else TIKTOK
    profile = adapter.profile_for(SimpleNamespace(
        user_agent=ua, platform='MacIntel' if 'Macintosh' in ua else 'Win32',
        screen='1920x1080', language='zh-CN', timezone='Asia/Shanghai',
        hardware_concurrency=8, device_memory=8))
    request = adapter.build_request(platform + '.content_detail', profile=profile,
        **({'aweme_id': video_id} if platform == 'douyin' else {'item_id': video_id}))
    spec = RequestSpec(request['method'], request['url'], request['params'], request['headers'])
    signed = asyncio.run(NativeSigner(adapter.platform, fill_ms_token=False).sign(
        spec, StaticFingerprint(ua, profile.browser_platform, profile.screen_width, profile.screen_height),
        SigningSession(cookies=cookies, proxy_url=proxy)))
    headers = {**dict(spec.headers), **dict(signed.headers), 'User-Agent': ua}
    with ydl.urlopen(Request(spec.url + '?' + signed.query, headers=headers)) as response:
        body = response.read(4 * 1024 * 1024 + 1)
    if len(body) > 4 * 1024 * 1024:
        raise EngineError('UPSTREAM_CHANGED', 'detail_size')
    try:
        content = adapter.parse_content(json.loads(body), fetched_at=datetime.now(UTC))
    except DtkError as error:
        code = {'UPSTREAM_RISK_CONTROL': 'RISK_CONTROL', 'CONTENT_PRIVATE': 'ACCESS_DENIED',
                'NOT_FOUND': 'NOT_FOUND', 'UNSUPPORTED_CONTENT': 'UNSUPPORTED_CONTENT'}.get(
                    str(error.code), 'UPSTREAM_CHANGED')
        raise EngineError(code, 'dtk') from None
    if content.content_id != video_id or content.is_deleted or content.is_private:
        raise EngineError('NOT_FOUND' if content.is_deleted else 'ACCESS_DENIED' if content.is_private
                          else 'UPSTREAM_CHANGED', 'identity')
    return content


def metadata(content):
    result = dict(platform=str(content.platform), video_id=content.content_id,
                  canonical_url=content.web_url, resolver='dtk', kind=str(content.kind),
                  title=content.title, author=content.author.unique_id or content.author.nickname,
                  duration_ms=content.duration_ms, cover_url=None)
    if content.music:
        result['music'] = dict(title=content.music.title, author=content.music.author)
    if str(content.kind) == 'image_album':
        result['images'] = [{k: choice[k] for k in ('width', 'height', 'quality_source')}
                            for choice in image_choices(content)]
    else:
        stream = content.media.video
        result.update(width=stream.width, height=stream.height, filesize=stream.size_bytes,
                      filesize_approx=None,
                      cover_url=content.media.covers[0].url if content.media.covers else None)
    return result


def preview_images(ydl, content, stage):
    from engine import EngineError
    names = []
    for index, choice in enumerate(image_choices(content), 1):
        with ydl.urlopen(choice['hd_url']) as response:
            body = response.read(MAX_IMAGE_BYTES + 1)
        if len(body) > MAX_IMAGE_BYTES:
            raise EngineError('UNSUPPORTED_CONTENT', 'image_size')
        try:
            extension = image_extension(body)
        except ValueError:
            raise EngineError('INVALID_MEDIA_CONTENT', 'preview') from None
        name = f'preview-{index:03d}.{extension}'
        (stage / name).write_bytes(body)
        names.append(name)
    return names


def download_images(ydl, content, stage, emit):
    from engine import EngineError
    album = stage / 'album'
    album.mkdir()
    files, downloaded = [], 0
    choices = image_choices(content)
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
                 completed_items=index if data['status'] == 'finished' else index - 1,
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
        success, _ = downloader.download(str(path), dict(url=url,
            http_headers={'User-Agent': ydl.params['http_headers']['User-Agent'],
                          'Referer': 'https://www.' + str(content.platform) + '.com/'}))
        if not success or not path.is_file():
            raise EngineError('IO_ERROR', 'image_download')
        body = path.read_bytes()
        if not body or len(body) > MAX_IMAGE_BYTES:
            raise EngineError('INVALID_MEDIA_CONTENT', 'image')
        try:
            extension = image_extension(body)
        except ValueError:
            raise EngineError('INVALID_MEDIA_CONTENT', 'image') from None
        final = path.with_suffix('.' + extension)
        path.rename(final)
        downloaded += len(body)
        files.append(dict(filename=final.name, bytes=len(body), sha256=hashlib.sha256(body).hexdigest(),
                          **{k: choice[k] for k in ('width', 'height', 'quality_source')}))
    emit('progress', downloaded_bytes=downloaded, total_bytes=downloaded, speed_bps=None,
         completed_items=len(files), total_items=len(files))
    emit('download_finished', platform=str(content.platform), video_id=content.content_id,
         kind='image_album', resolver='dtk', files=files, bytes=downloaded)


def video_info(content):
    """Expose only DTK's single-file streams to our existing yt-dlp selector."""
    from urllib.parse import urlsplit
    from engine import EngineError
    formats = []
    for index, stream in enumerate([content.media.video, *content.media.streams]):
        if stream is None or stream.watermark:
            continue
        formats.append(dict(format_id=f'dtk-{index}', url=stream.url, ext='mp4',
                            protocol=urlsplit(stream.url).scheme, vcodec=None, acodec=None,
                            width=stream.width, height=stream.height, filesize=stream.size_bytes,
                            tbr=stream.bitrate / 1000 if stream.bitrate else None))
    if not formats:
        raise EngineError('UNSUPPORTED_CONTENT', 'format')
    return dict(id=content.content_id, webpage_url=content.web_url, title=content.title,
                uploader=content.author.unique_id or content.author.nickname,
                duration=content.duration_ms / 1000 if content.duration_ms else None,
                thumbnail=content.media.covers[0].url if content.media.covers else None,
                extractor='dtk', extractor_key='DTK', formats=formats)
