"""Xiaohongshu note pages and media candidates for the pinned yt-dlp runtime."""
import re
from urllib.parse import parse_qsl, quote, urlencode, urlsplit, urlunsplit

from yt_dlp.extractor.xiaohongshu import XiaoHongShuIE
from yt_dlp.networking import Request
from yt_dlp.utils import float_or_none, int_or_none, js_to_json
from yt_dlp.utils.networking import std_headers
from yt_dlp.utils.traversal import traverse_obj

from author_profile import https_url, text
from video_formats import format_key, positive

PLATFORM = 'xiaohongshu'
NOTE_HOSTS = ('xiaohongshu.com', 'www.xiaohongshu.com')
SHORT_HOSTS = ('xhslink.cn', 'www.xhslink.cn', 'xhslink.com', 'www.xhslink.com')
NOTE_PATH = r'/(?:discovery/item|explore)/([0-9a-f]{24})/?'
MOBILE_UA = ('Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 '
             '(KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36')


def parse_link(parts):
    from engine import EngineError
    short = parts.hostname in SHORT_HOSTS
    pattern = r'/(?:[ao]/)?[A-Za-z0-9_-]+/?' if short else NOTE_PATH
    if not re.fullmatch(pattern, parts.path):
        raise EngineError('UNSUPPORTED_CONTENT', 'input')
    # Only the two share parameters needed for fetching a note are replayed.
    query = urlencode([(key, value) for key, value in parse_qsl(parts.query)
                       if key in ('xsec_token', 'xsec_source')]) if not short else ''
    host = parts.hostname if short else 'www.xiaohongshu.com'
    return PLATFORM, urlunsplit(('https', host, parts.path.rstrip('/'), query, '')), short


def user_agent(url, short, configured, default):
    if configured != default:
        return configured
    return MOBILE_UA if short or ('xsec_source', 'app_share') in parse_qsl(urlsplit(url).query) else std_headers['User-Agent']


def fetch_note(ydl, url, short):
    from engine import EngineError
    with ydl.urlopen(Request(url)) as response:
        final = urlsplit(response.url)
        if final.hostname not in NOTE_HOSTS:
            raise EngineError('UNSUPPORTED_CONTENT', 'short_link')
        if final.path.rstrip('/') == '/login':
            raise EngineError('ACCESS_DENIED', 'detail')
        canonical = parse_link(final)[1]
        video_id = urlsplit(canonical).path.rsplit('/', 1)[-1]
        if not short and urlsplit(url).path.rsplit('/', 1)[-1] != video_id:
            raise EngineError('UPSTREAM_CHANGED', 'identity')
        body = response.read(8 * 1024 * 1024 + 1)
    if len(body) > 8 * 1024 * 1024:
        raise EngineError('UPSTREAM_CHANGED', 'detail_size')
    state = XiaoHongShuIE(ydl)._search_json(
        r'window\.__INITIAL_STATE__\s*=', body.decode('utf-8', 'replace'), 'initial state', video_id,
        transform_source=js_to_json, fatal=False, default={})
    note = traverse_obj(state, ('note', 'noteDetailMap', video_id, 'note', {dict}),
                        ('noteData', 'data', 'noteData', {dict}))
    if not note or note.get('noteId') != video_id:
        raise EngineError('UPSTREAM_CHANGED', 'detail')
    if note.get('type') not in ('video', 'normal'):
        raise EngineError('UNSUPPORTED_CONTENT', 'detail')
    # A long link may carry the token even when a redirect omits it.
    if not urlsplit(canonical).query and not short:
        canonical = url
    return canonical, note


def metadata(note, canonical):
    user = note.get('user') if isinstance(note.get('user'), dict) else {}
    result = dict(platform=PLATFORM, video_id=note['noteId'], canonical_url=canonical,
                resolver='yt-dlp', title=text(note.get('title')), author=text(user.get('nickname') or user.get('nickName')),
                author_profile=dict(id=text(user.get('userId')), nickname=text(user.get('nickname') or user.get('nickName')),
                                    handle=None, kind='user'), cover_url=None)
    # The note detail reports publication time in milliseconds.
    timestamp = note.get('time')
    if type(timestamp) is int and 1_000_000_000_000 <= timestamp < 253_402_300_800_000:
        result['published_at'] = timestamp // 1000
    return result


def avatar_url(note):
    return traverse_obj(note, ('user', 'avatar', {https_url}))


def image_url(image):
    file_id = image.get('fileId')
    if not file_id:
        # Desktop shares can omit fileId. Their signed webpic path contains the asset ID.
        value = image.get('urlDefault')
        if isinstance(value, str):
            parts = urlsplit(value)
            match = (re.fullmatch(r'/\d{10,14}/[0-9a-f]{32}/([^!]+)(?:![^/]*)?', parts.path)
                     if (parts.hostname or '').endswith('.xhscdn.com') else None)
            if match:
                file_id = match[1]
    if isinstance(file_id, str) and re.fullmatch(r'(?:[A-Za-z0-9_-]+/){0,3}[A-Za-z0-9_-]+', file_id):
        return f'https://sns-img-qc.xhscdn.com/{quote(file_id)}?imageView2/2/w/1920/format/jpg'
    return None


def live_photo_url(image):
    for stream in traverse_obj(image, ('stream', 'h264', ..., {dict})):
        # Desktop note pages omit the stream description for the same clip.
        if stream.get('streamDesc') not in (None, 'WEB_LIVEPHOTO_19') or stream.get('watermark'):
            continue
        url = stream.get('masterUrl')
        if isinstance(url, str) and url.startswith('http://'):
            url = 'https://' + url[len('http://'):]
        if url := https_url(url):
            return url
    return None


def image_choices(note, max_images):
    from engine import EngineError
    images = note.get('imageList')
    if not isinstance(images, list) or not 1 <= len(images) <= max_images:
        raise EngineError('UNSUPPORTED_CONTENT', 'images')
    choices = []
    for image in images:
        url = image_url(image) if isinstance(image, dict) else None
        if not url:
            raise EngineError('UPSTREAM_CHANGED', 'images')
        choice = dict(url=url, hd_url=url, width=None, height=None, quality_source='high_definition')
        if image.get('livePhoto') is True:
            clip = live_photo_url(image)
            if not clip:
                raise EngineError('UNSUPPORTED_CONTENT', 'live_photo')
            choice['video_url'] = clip
        choices.append(choice)
    return choices


def is_album(note):
    if note['type'] == 'normal':
        return True
    images = note.get('imageList')
    # Photo slideshows carry a generated video alongside the individual images.
    if not isinstance(images, list) or len(images) < 2:
        return False
    urls = {image_url(image) for image in images if isinstance(image, dict)} - {None}
    return len(urls) > 1


def video_info(ydl, note, canonical, selected_format=None):
    from engine import EngineError
    formats = []
    for stream in traverse_obj(note, ('video', 'media', 'stream', ..., ..., {dict})):
        description = str(stream.get('streamDesc') or '').upper()
        stream_type = int_or_none(stream.get('streamType'))
        # Mini App streams (259/309) may carry the platform watermark.
        if (stream_type in (259, 309) or 'MINI_APP' in description
                or 'WATERMARK' in description or stream.get('watermark')):
            continue
        if stream_type not in (258, 301) and not re.match(r'^(?:X264|X265|AV1)_MP4(?:_|$)', description):
            continue
        if stream.get('audioCodec') == 'none' or stream.get('videoCodec') in ('none', 'bytevc2'):
            continue
        url = stream.get('masterUrl')
        url = https_url(url.replace('http://', 'https://', 1)) if isinstance(url, str) else None
        if url:
            # yt-dlp otherwise assigns a position that can change when variants disappear.
            fmt = dict(url=url, ext='mp4', format_id=f'xhs_{stream_type}' if stream_type is not None else 'xhs_web',
                       width=int_or_none(stream.get('width')),
                       height=int_or_none(stream.get('height')), fps=int_or_none(stream.get('fps')),
                       vcodec=stream.get('videoCodec'), acodec=stream.get('audioCodec'),
                       tbr=int_or_none(stream.get('avgBitrate'), scale=1000), filesize=int_or_none(stream.get('size')))
            if duration := positive(float_or_none(stream.get('duration'), scale=1000)):
                fmt['duration'] = duration
            formats.append(fmt)
    original_format = dict(ext='mp4', format_id='direct')
    if not formats or selected_format == format_key(original_format):
        origin = traverse_obj(note, ('video', 'consumer', 'originVideoKey', {str}))
        if origin and re.fullmatch(r'(?:[A-Za-z0-9_-]+/){0,3}[A-Za-z0-9_-]+', origin):
            # Like the upstream extractor, use GET because HEAD can reject valid originals.
            with ydl.urlopen(f'https://sns-video-bd.xhscdn.com/{origin}') as response:
                formats.append(dict(original_format, url=response.url,
                                    filesize=int_or_none(response.headers.get('Content-Length'))))
    if not formats:
        raise EngineError('UNSUPPORTED_CONTENT', 'format')
    base = metadata(note, canonical)
    images = traverse_obj(note, ('imageList', ..., {dict}))
    cover = image_url(images[0]) if images else None
    # Stream durations are milliseconds, while the note-level fields are seconds.
    duration = next((fmt['duration'] for fmt in formats if 'duration' in fmt), None)
    if duration is None:
        duration = traverse_obj(note, ('video', 'capa', 'duration', {float_or_none}, {positive}),
                                ('video', 'media', 'video', 'duration', {float_or_none}, {positive}))
    return dict(id=base['video_id'], webpage_url=canonical, title=base['title'], uploader=base['author'],
                timestamp=base.get('published_at'),
                _yingqu_author=base['author_profile'], _yingqu_author_avatar=avatar_url(note), formats=formats,
                duration=duration,
                thumbnails=[{'url': cover}] if cover else [], extractor=PLATFORM, extractor_key='XiaoHongShu')
