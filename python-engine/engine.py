"""Single-video yt-dlp adapter. No business state, credentials on disk, or raw logs."""
import functools
import math
import re
from http.cookiejar import Cookie
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

from yt_dlp import YoutubeDL
from yt_dlp.globals import plugin_dirs
from yt_dlp.utils import DownloadError
from yt_dlp.networking.exceptions import RequestError
from yt_dlp.version import __version__

from network import GuardedRH, TargetError
from douyin import DouyinIE

ENGINE_VERSION = '0.2.1'
YTDLP_VERSION = '2026.08.19'
DEFAULT_UA = ('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 '
              '(KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36 Edg/150.0.0.0')

# Disable discovery before constructing any YoutubeDL or extractor instance.
plugin_dirs.value = []


class EngineError(Exception):
    def __init__(self, code, stage='engine'):
        super().__init__(code)
        self.code, self.stage = code, stage


class SafeLogger:
    def __init__(self):
        self.code = 'UPSTREAM_CHANGED'

    def debug(self, message):
        pass

    info = debug
    warning = debug

    def error(self, message):
        # Only stable, bounded codes leave this process. No upstream free text.
        lower = message.lower()
        if 'network_target_rejected' in lower:
            self.code = 'INVALID_INPUT'
        elif ('403' in lower or '401' in lower or 'fresh cookies' in lower
              or 'login' in lower or 'uifid cookie is required' in lower):
            self.code = 'ACCESS_DENIED'
        elif '404' in lower or 'not found' in lower:
            self.code = 'NOT_FOUND'
        elif 'timed out' in lower:
            self.code = 'TIMEOUT'
        elif 'unable to download' in lower or 'connection' in lower or 'incomplete' in lower:
            self.code = 'NETWORK_ERROR'
        elif 'requested format is not available' in lower or 'no video formats' in lower:
            self.code = 'UNSUPPORTED_CONTENT'


def parse_input(text):
    if not isinstance(text, str):
        raise EngineError('INVALID_INPUT', 'input')
    urls = re.findall(r'https?://[^\s<>"\u3000]+', text)
    if len(urls) != 1:
        raise EngineError('INVALID_INPUT', 'input')
    url = urls[0].rstrip('.,;!?，。；！）》)')
    try:
        parts = urlsplit(url)
        if parts.username or parts.password or parts.port not in (None, 80 if parts.scheme == 'http' else 443):
            raise ValueError()
        host = parts.hostname
        platform = ('douyin' if host in ('douyin.com', 'www.douyin.com', 'v.douyin.com')
                    else 'tiktok' if host in ('www.tiktok.com', 'tiktok.com', 'vt.tiktok.com', 'vm.tiktok.com')
                    else None)
        if not platform:
            raise EngineError('UNSUPPORTED_PLATFORM', 'input')
        short = host in ('v.douyin.com', 'vt.tiktok.com', 'vm.tiktok.com')
        pattern = r'/[A-Za-z0-9_-]+/?' if short else (
            r'/video/\d+/?' if platform == 'douyin' else r'/@[\w.-]+/video/\d+/?')
        if not re.fullmatch(pattern, parts.path):
            raise EngineError('UNSUPPORTED_CONTENT', 'input')
        # Share tracking query is unnecessary for single-video identity.
        host = 'www.' + platform + '.com' if not short else host
        return platform, urlunsplit(('https', host, parts.path, '', '')), short
    except ValueError:
        raise EngineError('INVALID_INPUT', 'input') from None


def profile_config(profile):
    if not isinstance(profile, dict) or set(profile) - {'cookie', 'user_agent', 'proxy'}:
        raise EngineError('CONFIG_INVALID', 'config')
    ua, cookie, proxy = profile.get('user_agent', DEFAULT_UA), profile.get('cookie', ''), profile.get('proxy')
    if (not isinstance(ua, str) or not ua or not isinstance(cookie, str)
            or any(c in ua + cookie for c in '\r\n')):
        raise EngineError('CONFIG_INVALID', 'config')
    if proxy is not None:
        try:
            p = urlsplit(proxy)
            if (p.scheme not in ('http', 'socks5') or not p.hostname or not p.port
                    or p.path not in ('', '/') or p.query or p.fragment):
                raise ValueError()
        except (ValueError, TypeError, AttributeError):
            raise EngineError('CONFIG_INVALID', 'config') from None
    cookies = {}
    for item in cookie.split(';'):
        if not item.strip():
            continue
        name, sep, value = item.strip().partition('=')
        if not sep or not re.fullmatch(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+", name):
            raise EngineError('CONFIG_INVALID', 'config')
        cookies[name] = value
    return ua, cookies, proxy


def add_cookies(jar, cookies, platform):
    for name, value in cookies.items():
        jar.set_cookie(Cookie(0, name, value, None, False, '.' + platform + '.com', True,
                             True, '/', True, True, None, True, None, None, {}))


def select_format(context):
    # yt-dlp supplies formats sorted worst -> best. No segmented or external downloader.
    for fmt in reversed(context['formats']):
        if (fmt.get('protocol') in ('http', 'https') and fmt.get('ext') in ('mp4', 'webm')
                and fmt.get('vcodec') != 'none' and fmt.get('acodec') != 'none'
                and not fmt.get('has_drm')):
            yield fmt
            return
    # A truly silent video may have no audio format at all. Do not discard a separate audio track.
    if not any(f.get('vcodec') == 'none' and f.get('acodec') != 'none' for f in context['formats']):
        for fmt in reversed(context['formats']):
            if (fmt.get('protocol') in ('http', 'https') and fmt.get('ext') in ('mp4', 'webm')
                    and fmt.get('vcodec') != 'none' and not fmt.get('has_drm')):
                yield fmt
                return
    raise EngineError('UNSUPPORTED_CONTENT', 'format')


def cover_url(info):
    # yt-dlp sorts thumbnails from least to most preferred; use dimensions when available.
    candidates = [t for t in info.get('thumbnails', []) if isinstance(t, dict)
                  and isinstance(t.get('url'), str) and urlsplit(t['url']).scheme == 'https']
    if candidates:
        return max(enumerate(candidates), key=lambda item: (
            (item[1].get('width') or 0) * (item[1].get('height') or 0),
            str(item[1].get('id', '')).startswith(('origin_cover', 'originCover')),
            item[1].get('preference') or 0, item[0]))[1]['url']
    url = info.get('thumbnail')
    return url if isinstance(url, str) and urlsplit(url).scheme == 'https' else None


def download_cover(ydl, info, stage):
    url = cover_url(info)
    if not url:
        return dict(cover_error='COVER_UNAVAILABLE')
    try:
        # Goes through the same guarded transport, proxy, cookies and timeout as the video.
        with ydl.urlopen(url) as response:
            body = response.read(10 * 1024 * 1024 + 1)
        valid = (body.startswith(b'\xff\xd8\xff') or body.startswith(b'\x89PNG\r\n\x1a\n')
                 or (body.startswith(b'RIFF') and body[8:12] == b'WEBP'))
        if not valid or len(body) > 10 * 1024 * 1024:
            return dict(cover_error='COVER_UNAVAILABLE')
        partial = stage / 'cover.part'
        partial.write_bytes(body)
        partial.rename(stage / 'cover')
        return dict(cover_filename='cover', cover_bytes=len(body))
    except (OSError, RequestError, TargetError, DownloadError):
        return dict(cover_error='COVER_UNAVAILABLE')


def metadata(info, platform):
    video_id = info.get('id')
    if (info.get('_type', 'video') != 'video' or info.get('is_live') or info.get('live_status') == 'is_live'
            or not isinstance(video_id, str) or not re.fullmatch(r'\d{1,30}', video_id)):
        raise EngineError('UNSUPPORTED_CONTENT', 'detail')
    canonical = info.get('webpage_url')
    actual, canonical, short = parse_input(canonical)
    if actual != platform or short or canonical.rstrip('/').split('/')[-1] != video_id:
        raise EngineError('UPSTREAM_CHANGED', 'detail')
    duration = info.get('duration')
    duration_ms = (round(duration * 1000) if isinstance(duration, (float, int))
                   and math.isfinite(duration) and duration >= 0 else None)
    cover = cover_url(info)
    selected = {}
    for field in ('width', 'height', 'filesize', 'filesize_approx'):
        value = info.get(field)
        valid = (type(value) in (int, float) and math.isfinite(value) and value > 0
                 and (field == 'filesize_approx' or value == int(value)))
        selected[field] = max(1, round(value)) if valid else None
    return dict(platform=platform, video_id=video_id, canonical_url=canonical,
                title=info.get('title'), author=info.get('uploader'), cover_url=cover,
                duration_ms=duration_ms, **selected)


def operate(method, params, emit):
    if not isinstance(params, dict):
        raise EngineError('ENGINE_PROTOCOL_ERROR', 'protocol')
    platform, url, short = parse_input(params.get('input'))
    profiles = params.get('profiles', {})
    if not isinstance(profiles, dict):
        raise EngineError('CONFIG_INVALID', 'config')
    ua, cookies, proxy = profile_config(profiles.get(platform, {}))
    downloading = method == 'download_video'
    expected = params.get('expected')
    stage = None
    if downloading:
        if (not isinstance(expected, dict) or expected.get('platform') != platform
                or not isinstance(expected.get('video_id'), str)):
            raise EngineError('ENGINE_PROTOCOL_ERROR', 'protocol')
        try:
            stage = Path(params['staging_dir'])
            if not stage.is_absolute() or not stage.is_dir() or stage.is_symlink() or any(stage.iterdir()):
                raise ValueError()
        except (KeyError, TypeError, ValueError, OSError):
            raise EngineError('IO_ERROR', 'staging') from None

    def match_filter(info, *, incomplete=False):
        if info.get('_type', 'video') in ('playlist', 'multi_video') or info.get('is_live'):
            raise EngineError('UNSUPPORTED_CONTENT', 'detail')
        if downloading and info.get('id') and info['id'] != expected['video_id']:
            raise EngineError('UPSTREAM_CHANGED', 'identity')
        return None

    def progress(data):
        if data.get('status') in ('downloading', 'finished'):
            total = data.get('total_bytes')
            speed = data.get('speed')
            emit('progress', downloaded_bytes=data.get('downloaded_bytes', 0),
                 total_bytes=total, speed_bps=speed if speed and math.isfinite(speed) else None)

    logger = SafeLogger()
    options = dict(proxy=proxy or '', http_headers={'User-Agent': ua},
        logger=logger, quiet=True, noprogress=True, cachedir=False, noplaylist=True,
        allowed_extractors=['Douyin$'] if platform == 'douyin' else ['TikTok$', r'vm\.tiktok$'],
        format=select_format, match_filter=match_filter, progress_hooks=[progress] if downloading else [],
        outtmpl=str(stage / 'video.%(ext)s') if stage else 'unused.%(ext)s',
        overwrites=False, continuedl=False, socket_timeout=30, retries=0,
        extractor_retries=0, fragment_retries=0, file_access_retries=0,
        postprocessors=[], fixup='never', external_downloader=None,
        js_runtimes={}, remote_components=set())
    try:
        with YoutubeDL(options) as ydl:
            add_cookies(ydl.cookiejar, cookies, platform)
            if platform == 'douyin':
                ydl.add_info_extractor(DouyinIE())
            ydl._request_director = ydl.build_request_director(
                [functools.partial(GuardedRH, platform=platform)])
            if platform == 'douyin' and short:
                with ydl.urlopen(url) as response:
                    resolved_platform, url, still_short = parse_input(response.url)
                if resolved_platform != platform or still_short:
                    raise EngineError('UNSUPPORTED_CONTENT', 'short_link')
            # Both operations re-extract. Nothing persists signed URLs or the raw info dict.
            info = ydl.extract_info(url, download=False)
            if not isinstance(info, dict):
                raise EngineError('UPSTREAM_CHANGED', 'detail')
            result = metadata(info, platform)
            if not downloading:
                emit('resolved', result=result)
                return
            if result['video_id'] != expected['video_id']:
                raise EngineError('UPSTREAM_CHANGED', 'identity')
            # The validated single-file format and identity remain in this process;
            # start media I/O only after validation, never forward a URL to Rust.
            ydl.process_info(info)
            file = Path(ydl.prepare_filename(info))
            if file.parent != stage or file.is_symlink() or not file.is_file() or file.stat().st_size == 0:
                raise EngineError('IO_ERROR', 'output')
            emit('download_finished', platform=platform, video_id=result['video_id'],
                 filename=file.name, bytes=file.stat().st_size, container=info['ext'],
                 **download_cover(ydl, info, stage))
    except DownloadError:
        raise EngineError(logger.code, 'yt_dlp') from None
    except TargetError:
        raise EngineError('INVALID_INPUT', 'network_target') from None
    except RequestError as error:
        if isinstance(error.cause, TargetError):
            raise EngineError('INVALID_INPUT', 'network_target') from None
        raise EngineError('NETWORK_ERROR', 'request') from None


def check_version():
    if __version__ != YTDLP_VERSION:
        raise EngineError('ENGINE_UNAVAILABLE', 'version')
