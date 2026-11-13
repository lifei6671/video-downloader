"""Resolve a WeChat Channels share using Tencent's two web endpoints."""
import hashlib
import json
import math
import re
import secrets
import struct
import subprocess
import sys
import time
from http.cookiejar import Cookie
from urllib.parse import parse_qs, urlencode, urlsplit

from yt_dlp.networking import Request
from yt_dlp.networking.exceptions import HTTPError, RequestError
from yt_dlp.utils.networking import clean_proxies

from browser_cookies import load_default_cookies_for_url, scoped_jar
from local_media import display_dimensions
from network import GuardedRH, TargetError, validate_target

YUANBAO = 'https://yuanbao.tencent.com'
CHANNELS = 'https://channels.weixin.qq.com'
PARSE_URL = YUANBAO + '/api/weixin/get_parse_result'
FEED_URL = CHANNELS + '/finder-preview/api/feed/get_feed_info'
PLAY_PAGE = CHANNELS + '/finder-preview/pages/feed'
MEDIA_REFERER = 'https://weixin.qq.com/'
JSON_LIMIT = 1024 * 1024
MEDIA_PREFIX_BYTES = 128 * 1024
MAX_MP4_HEADER_BYTES = 8 * 1024 * 1024


class WeixinError(Exception):
    pass


def canonical_share(parts):
    if (parts.scheme != 'https' or parts.hostname != 'weixin.qq.com'
            or parts.username is not None or parts.password is not None
            or parts.port not in (None, 443)
            or not re.fullmatch(r'/sph/[A-Za-z0-9_-]{1,128}', parts.path)):
        raise WeixinError('INVALID_INPUT')
    return MEDIA_REFERER.rstrip('/') + parts.path


def manual_jar(cookies):
    jar = scoped_jar()
    for name, value in cookies.items():
        jar.set_cookie(Cookie(0, name, value, None, False,
                              'yuanbao.tencent.com', False, False, urlsplit(PARSE_URL).path,
                              True, True, None, True, None, None, {}))
    return jar


def parse_manual_cookie(raw):
    from engine import parse_cookie_header
    cookies = parse_cookie_header(raw)
    if any(not value.isascii() or any(ord(c) < 32 or ord(c) == 127 for c in value)
           for value in cookies.values()):
        raise WeixinError('CONFIG_INVALID')
    return manual_jar(cookies)


def post_json(target, url, payload, ua, proxy, logger, cookies=None):
    request = Request(url, data=json.dumps(payload).encode('utf-8'), headers={
        'Accept': 'application/json', 'Content-Type': 'application/json',
        'Origin': YUANBAO if target == 'weixin-yuanbao' else CHANNELS,
        'Referer': (YUANBAO if target == 'weixin-yuanbao' else CHANNELS) + '/',
        'User-Agent': ua,
    })
    proxies = {'all': proxy} if proxy else {}
    clean_proxies(proxies, request.headers)
    with GuardedRH(platform=target, proxies=proxies,
                   cookiejar=scoped_jar() if cookies is None else cookies,
                   timeout=15, logger=logger) as client:
        try:
            response = client.send(request)
        except HTTPError as error:
            # 401 is authentication only for the Yuanbao endpoint.
            error.response.close()
            if error.status == 401 and target == 'weixin-yuanbao':
                raise WeixinError('WEIXIN_LOGIN_REQUIRED') from None
            if error.status == 429:
                raise WeixinError('WEIXIN_RATE_LIMITED') from None
            if error.status in (401, 403, 404) and target == 'weixin-feed':
                raise WeixinError('WEIXIN_VIDEO_UNAVAILABLE') from None
            raise
        with response:
            raw = response.read(JSON_LIMIT + 1)
    if len(raw) <= JSON_LIMIT:
        try:
            value = json.loads(raw)
            if isinstance(value, dict):
                return value
        except (UnicodeError, ValueError):
            pass
    raise WeixinError('WEIXIN_UPSTREAM_CHANGED')


def play_params(playable):
    if not isinstance(playable, str) or len(playable) > 8192:
        raise WeixinError('WEIXIN_UPSTREAM_CHANGED')
    try:
        parts = urlsplit(playable)
        if (parts.scheme != 'https' or parts.hostname != 'channels.weixin.qq.com'
                or parts.path != '/finder-preview/pages/feed' or parts.username is not None
                or parts.password is not None or parts.port not in (None, 443)):
            raise ValueError()
        params = parse_qs(parts.query, max_num_fields=24, keep_blank_values=True)
        if any(len(params.get(key, [])) != 1 or not 0 < len(params[key][0]) <= 4096
               for key in ('eid', 'token')):
            raise ValueError()
    except ValueError:
        raise WeixinError('WEIXIN_UPSTREAM_CHANGED') from None
    return params['eid'][0], params['token'][0]


def feed_call(eid, token):
    query = urlencode({'_rid': f'{int(time.time()):x}-{secrets.token_hex(4)}', '_pageUrl': PLAY_PAGE})
    return FEED_URL + '?' + query, {'baseReq': {'generalToken': token}, 'exportId': eid}


def object_or_empty(value):
    return value if isinstance(value, dict) else {}


def text(value, maximum=4096):
    return value.strip()[:maximum] or None if isinstance(value, str) else None


def format_choices(feed):
    formats, seen = [], set()
    choices = [('h264', object_or_empty(feed.get('h264VideoInfo'))),
               ('h265', object_or_empty(feed.get('h265VideoInfo'))), ('default', feed)]
    flags = ('decodeKey', 'encryptKey', 'encrypted', 'drm', 'drmType')
    if any(feed.get(key) for key in flags):
        raise WeixinError('WEIXIN_MEDIA_PROTECTED')
    protected = {info.get('videoUrl') for _, info in choices
                 if isinstance(info.get('videoUrl'), str) and any(info.get(key) for key in flags)}
    for label, info in choices:
        url = info.get('videoUrl')
        if not isinstance(url, str) or not url or url in seen or url in protected:
            continue
        validate_target(url, 'weixin-media')
        seen.add(url)
        formats.append(dict(format_id='weixin-' + label, url=url, ext='mp4', protocol='https',
                            vcodec=label if label != 'default' else None,
                            _yingqu_compatibility={'h264': 2, 'h265': 1, 'default': 0}[label],
                            http_headers={'Referer': MEDIA_REFERER}))
    if not formats:
        raise WeixinError('WEIXIN_MEDIA_PROTECTED' if protected else 'WEIXIN_UPSTREAM_CHANGED')
    return formats


def leading_movie_end(prefix):
    """Locate the end of a leading MP4 movie header without scanning media data."""
    if prefix[4:8] != b'ftyp':
        return None
    offset = 0
    while offset + 8 <= len(prefix):
        size, kind = struct.unpack_from('>I4s', prefix, offset)
        header_size = 8
        if size == 1:
            header_size = 16
            if offset + header_size > len(prefix):
                return None
            size = struct.unpack_from('>Q', prefix, offset + 8)[0]
        if size < header_size or kind == b'mdat':
            return None
        offset += size
        if kind == b'moov':
            return offset
    return None


def media_range_request(fmt, start, end):
    return Request(fmt['url'], headers={**fmt['http_headers'],
                   'Range': f'bytes={start}-{end}', 'Accept-Encoding': 'identity'},
                   extensions={'timeout': 5})


def probe_media_metadata(ydl, formats, ffprobe):
    for fmt in formats:
        request = media_range_request(fmt, 0, MEDIA_PREFIX_BYTES - 1)
        try:
            with ydl.urlopen(request) as response:
                if response.status != 206 or response.headers.get('Content-Encoding', '').lower() not in ('', 'identity'):
                    continue
                # GuardedRH validates the requested range before handing us the response.
                total = response.headers.get('Content-Range', '').rpartition('/')[2]
                if re.fullmatch(r'[0-9]{1,16}', total) and 0 < int(total) < 2**53:
                    fmt['filesize'] = int(total)
                if not ffprobe:
                    continue
                prefix = response.read(MEDIA_PREFIX_BYTES)
            movie_end = leading_movie_end(prefix)
            if movie_end and movie_end > len(prefix):
                if movie_end > min(MAX_MP4_HEADER_BYTES, fmt.get('filesize', 0)):
                    continue
                with ydl.urlopen(media_range_request(fmt, len(prefix), movie_end - 1)) as response:
                    if (response.status != 206
                            or response.headers.get('Content-Encoding', '').lower() not in ('', 'identity')
                            or response.headers.get('Content-Range', '').rpartition('/')[2] != total):
                        continue
                    remaining = movie_end - len(prefix)
                    suffix = response.read(remaining)
                    if len(suffix) != remaining:
                        continue
                    prefix += suffix
            probe = subprocess.run([
                ffprobe, '-v', 'error', '-nofind_stream_info', '-protocol_whitelist', 'pipe',
                '-select_streams', 'V:0', '-show_entries',
                'stream=width,height,duration:stream_side_data=rotation:format=format_name',
                '-of', 'json', '-i', 'pipe:0',
            ], input=prefix, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=5,
                creationflags=0x08000000 if sys.platform == 'win32' else 0)
            if not probe.returncode:
                metadata = object_or_empty(json.loads(probe.stdout))
                streams = metadata.get('streams') or []
                if not streams or 'mp4' not in metadata.get('format', {}).get('format_name', '').split(','):
                    continue
                stream = streams[0]
                if dimensions := display_dimensions(stream):
                    fmt['width'], fmt['height'] = dimensions
                duration = float(stream.get('duration', 'nan'))
                if math.isfinite(duration) and 0 < duration <= (2**53 - 1) / 1000:
                    fmt['duration'] = duration
        except (RequestError, TargetError, OSError, ValueError, TypeError, OverflowError, subprocess.SubprocessError):
            # A signed URL or an MP4 with its metadata at the end can be unprobeable.
            continue


def resolve(url, ua, cookies, proxy, logger):
    payload = {'type': 'video_channel_url', 'url': url, 'scene': 1}

    def parse(cookiejar=None):
        return post_json('weixin-yuanbao', PARSE_URL, payload, ua, proxy, logger, cookiejar)

    try:
        result = parse()
    except WeixinError as error:
        if str(error) != 'WEIXIN_LOGIN_REQUIRED':
            raise
        if cookies:
            auth = parse_manual_cookie(cookies)
        else:
            _, status, auth = load_default_cookies_for_url(PARSE_URL)
            if auth is None:
                code = ('WEIXIN_LOGIN_REQUIRED' if status == 'NO_APPLICABLE_COOKIES' else
                        'WEIXIN_COOKIE_EXPIRED' if status == 'COOKIES_EXPIRED' else
                        'BROWSER_COOKIES_UNAVAILABLE')
                raise WeixinError(code) from None
        try:
            result = parse(auth)
        except WeixinError as retry:
            if str(retry) == 'WEIXIN_LOGIN_REQUIRED':
                raise WeixinError('WEIXIN_COOKIE_EXPIRED') from None
            raise
    data = object_or_empty(result.get('data'))
    export = data.get('wx_export_id')
    playable = data.get('playable_url')
    if type(result.get('code')) is not int or result['code'] != 0:
        raise WeixinError('WEIXIN_UPSTREAM_CHANGED')
    if export == '' and playable == '':
        raise WeixinError('WEIXIN_VIDEO_UNAVAILABLE')
    if not isinstance(export, str) or not 0 < len(export) <= 4096 or not export.strip():
        raise WeixinError('WEIXIN_UPSTREAM_CHANGED')
    endpoint, payload = feed_call(*play_params(playable))
    result = post_json('weixin-feed', endpoint, payload, ua, proxy, logger)
    details = object_or_empty(result.get('data'))
    err_type = object_or_empty(details.get('errMsg')).get('type')
    if err_type == 4:
        raise WeixinError('WEIXIN_VIDEO_UNAVAILABLE')
    feed = object_or_empty(details.get('feedInfo'))
    if (type(result.get('errCode')) is not int or result['errCode'] != 0
            or type(err_type) is not int or err_type != 0 or not feed):
        raise WeixinError('WEIXIN_UPSTREAM_CHANGED')
    if feed.get('mediaType') != 4:
        raise WeixinError('UNSUPPORTED_CONTENT')
    cover = feed.get('coverUrl') or data.get('cover_url')
    if isinstance(cover, str) and cover:
        try:
            validate_target(cover, 'weixin-media')
        except TargetError:
            cover = None
    else:
        cover = None
    author = object_or_empty(details.get('authorInfo'))
    avatar = None
    for candidate in (author.get('headImgUrl'), data.get('author_icon')):
        if isinstance(candidate, str) and candidate:
            try:
                validate_target(candidate, 'weixin-avatar')
            except TargetError:
                continue
            avatar = candidate
            break
    return dict(id=hashlib.sha256(('weixin:export:' + export).encode()).hexdigest(),
                webpage_url=url, title=text(feed.get('description')) or text(data.get('desc')),
                uploader=text(author.get('nickname'), 512) or text(data.get('author'), 512),
                timestamp=feed.get('createtime'), thumbnail=cover, _yingqu_author_avatar=avatar,
                formats=format_choices(feed))
