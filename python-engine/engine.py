"""Single-video yt-dlp adapter. No business state, credentials on disk, or raw logs."""
import functools
import http.client
import math
import os
import re
import socket
import sys
from http.cookiejar import Cookie
from pathlib import Path
from urllib.parse import parse_qs, urlsplit, urlunsplit

from yt_dlp import YoutubeDL
from yt_dlp.globals import plugin_dirs
from yt_dlp.utils import DownloadError, ContentTooShortError, PostProcessingError
from yt_dlp.networking.exceptions import RequestError, HTTPError, IncompleteRead
from yt_dlp.networking import Request
from yt_dlp.version import __version__

from network import GuardedRH, TargetError, MediaRangeError
from douyin import DouyinIE
from browser_cookies import load_default_cookies
from local_tools import installed_tool, javascript_runtime
import runtime_dependencies
import local_cover
import local_video
import keyframes
import audio_excerpt
import transcription
import author_profile
import douyin_cover

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'vendor/dtk/upstream/src'))
import dtk_adapter
from video_formats import select_format, public_formats
from dtk.core.errors import DtkError

ENGINE_VERSION = '0.17.0'
PROTOCOL_VERSION = 3
YTDLP_VERSION = '2026.08.19'
DEFAULT_UA = ('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 '
              '(KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36 Edg/150.0.0.0')

# Disable discovery before constructing any YoutubeDL or extractor instance.
plugin_dirs.value = []


class EngineError(Exception):
    def __init__(self, code, stage='engine'):
        super().__init__(code)
        self.code, self.stage = code, stage


def error_code(error=None, message=''):
    pending, chain, seen = [error], [], set()
    while pending:
        item = pending.pop()
        if not isinstance(item, BaseException) or id(item) in seen:
            continue
        seen.add(id(item))
        chain.append(item)
        pending.extend([getattr(item, 'cause', None), item.__cause__, item.__context__])
        exc_info = getattr(item, 'exc_info', None)
        if isinstance(exc_info, tuple) and len(exc_info) == 3:
            pending.append(exc_info[1])
    for item in chain:
        if isinstance(item, (EngineError, TargetError)):
            return item.code
        if isinstance(item, MediaRangeError):
            return 'INCOMPLETE_MEDIA'
    for item in chain:
        if isinstance(item, HTTPError):
            return {401: 'ACCESS_DENIED', 403: 'ACCESS_DENIED', 404: 'NOT_FOUND',
                    429: 'RISK_CONTROL'}.get(item.status, 'NETWORK_ERROR')
        if isinstance(item, (IncompleteRead, http.client.IncompleteRead, ContentTooShortError)):
            return 'INCOMPLETE_MEDIA'
        if isinstance(item, (TimeoutError, socket.timeout)):
            return 'TIMEOUT'
        if isinstance(item, PostProcessingError):
            return 'MEDIA_MERGE_FAILED'
    if any(isinstance(item, RequestError) for item in chain):
        return 'NETWORK_ERROR'
    # yt-dlp sometimes discards the exception after exhausting its native retries.
    # Require an HTTP label and bounded three-digit code, never arbitrary digits.
    lower = (message or str(error or '')).lower()
    status = re.search(r'\bhttp(?:\s+error|\s+status(?:\s+code)?)?[:\s]+(401|403|404|429|500|502|503|504)\b', lower)
    if status:
        return {401: 'ACCESS_DENIED', 403: 'ACCESS_DENIED', 404: 'NOT_FOUND',
                429: 'RISK_CONTROL'}.get(int(status[1]), 'NETWORK_ERROR')
    if re.search(r'\b\d+ bytes read, \d+ more expected\b', lower):
        return 'INCOMPLETE_MEDIA'
    for words, code in ((('risk_control',), 'RISK_CONTROL'),
                        (('network_target_rejected',), 'INVALID_INPUT'),
                        (('requested format is not available',), 'FORMAT_UNAVAILABLE'),
                        (('fresh cookies', 'sign in', 'login required', 'uifid cookie is required'), 'ACCESS_DENIED'),
                        (('not found',), 'NOT_FOUND'), (('timed out', 'timeout'), 'TIMEOUT'),
                        (('incomplete', 'content too short'), 'INCOMPLETE_MEDIA'),
                        (('unable to download', 'connection', 'name resolution'), 'NETWORK_ERROR'),
                        (('no video formats',), 'UNSUPPORTED_CONTENT')):
        if any(word in lower for word in words):
            return code
    return 'UPSTREAM_CHANGED'


class SafeLogger:
    def __init__(self):
        self.code = 'UPSTREAM_CHANGED'

    def debug(self, message, **kwargs):
        pass

    info = debug
    warning = debug

    def error(self, message, **kwargs):
        # Only stable, bounded codes leave this process. No upstream free text.
        self.code = error_code(sys.exception(), message)


def progress_number(value, maximum, *, integer=False, positive=False):
    if (type(value) not in (int, float) or not math.isfinite(value)
            or value < (1 if positive else 0) or value > maximum or (integer and value != int(value))):
        return None
    return int(value) if integer else value


class DownloadProgress:
    """Per-operation stream counters. Finished hooks describe streams, never the task."""
    def __init__(self, emit):
        self.emit, self.streams, self.merging = emit, {}, False

    def prepare(self, info):
        for fmt in info.get('requested_formats') or [info]:
            self.streams[str(fmt.get('format_id') or '')] = dict(count=0,
                total=progress_number(fmt.get('filesize'), 2**53 - 1, integer=True, positive=True),
                estimate=progress_number(fmt.get('filesize_approx'), 2**53 - 1, integer=True, positive=True))

    @property
    def transferred(self):
        return sum(s['count'] for s in self.streams.values())

    def __call__(self, data):
        if data.get('status') not in ('downloading', 'finished'):
            return
        key = str((data.get('info_dict') or {}).get('format_id') or '')
        if key not in self.streams:
            if len(self.streams) != 1:
                raise EngineError('ENGINE_PROTOCOL_ERROR', 'progress_stream')
            key = next(iter(self.streams))
        stream = self.streams[key]
        count = progress_number(data.get('downloaded_bytes'), 2**53 - 1, integer=True)
        if count is None:
            raise EngineError('ENGINE_PROTOCOL_ERROR', 'progress_bytes')
        stream['count'] = max(count, stream['count'])
        total = progress_number(data.get('total_bytes'), 2**53 - 1, integer=True, positive=True)
        estimate = progress_number(data.get('total_bytes_estimate'), 2**53 - 1, integer=True, positive=True)
        stream['total'] = total or stream['total']
        stream['estimate'] = estimate or stream['estimate']
        if stream['total'] is not None and stream['count'] > stream['total']:
            raise EngineError('INCOMPLETE_MEDIA', 'progress_bytes')
        totals = [s['total'] for s in self.streams.values()]
        estimates = [s['total'] or s['estimate'] for s in self.streams.values()]
        total = sum(totals) if all(totals) else None
        estimate = sum(estimates) if total is None and all(estimates) else None
        speed = progress_number(data.get('speed'), 10**12)
        eta = progress_number(data.get('eta'), 7 * 86400)
        if len(self.streams) > 1:
            eta = (max(0, (total or estimate) - self.transferred) / speed
                   if (total or estimate) and speed else None)
            eta = progress_number(eta, 7 * 86400)
        self.emit('progress', downloaded_bytes=self.transferred, total_bytes=total,
                  total_bytes_estimate=estimate, speed_bps=speed, eta_seconds=eta)

    def postprocess(self, data):
        if data.get('postprocessor') == 'Merger' and data.get('status') == 'started' and not self.merging:
            self.merging = True
            self.emit('progress', downloaded_bytes=self.transferred, total_bytes=None,
                      total_bytes_estimate=None, speed_bps=None, eta_seconds=None, phase='merging')


def http_chunk_size(profile=None, platform=None):
    # A saved YouTube mode takes precedence over the legacy probe override.
    # None preserves per-stream extractor defaults; zero explicitly clears them.
    if profile is not None and 'http_chunk_size' in profile:
        value = profile['http_chunk_size']
        if (platform != 'youtube' or value is not None and
                (type(value) is not int or value != 0 and
                 (not 1048576 <= value <= 1073741824 or value % 1048576))):
            raise EngineError('CONFIG_INVALID', 'http_chunk_size')
        return value
    # Deployment/test override. Unset preserves extractor defaults (including YouTube chunks).
    value = os.environ.get('YINGQU_HTTP_CHUNK_SIZE', '0')
    if value not in ('0', '10485760'):
        raise EngineError('CONFIG_INVALID', 'http_chunk_size')
    return int(value)


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
            if host in ('youtube.com', 'www.youtube.com', 'm.youtube.com', 'youtu.be'):
                video_id = (parts.path.strip('/') if host == 'youtu.be' else
                            parse_qs(parts.query).get('v', [''])[0] if parts.path == '/watch' else
                            parts.path.rstrip('/').split('/')[-1] if re.fullmatch(r'/(?:shorts|embed)/[^/]+/?', parts.path) else '')
                if not re.fullmatch(r'[A-Za-z0-9_-]{11}', video_id):
                    raise EngineError('UNSUPPORTED_CONTENT', 'input')
                return 'youtube', 'https://www.youtube.com/watch?v=' + video_id, False
            raise EngineError('UNSUPPORTED_PLATFORM', 'input')
        short = host in ('v.douyin.com', 'vt.tiktok.com', 'vm.tiktok.com')
        pattern = r'/[A-Za-z0-9_-]+/?' if short else (
            r'/(?:video|note)/\d+/?' if platform == 'douyin' else r'/@[\w.-]+/(?:video|photo)/\d+/?')
        if not re.fullmatch(pattern, parts.path):
            raise EngineError('UNSUPPORTED_CONTENT', 'input')
        # Share tracking query is unnecessary for single-video identity.
        host = 'www.' + platform + '.com' if not short else host
        return platform, urlunsplit(('https', host, parts.path, '', '')), short
    except ValueError:
        raise EngineError('INVALID_INPUT', 'input') from None


def profile_config(profile):
    if not isinstance(profile, dict) or set(profile) - {'cookie', 'user_agent', 'proxy', 'http_chunk_size'}:
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


def cover_url(info):
    # Prefer Douyin's designed cover, then the existing resolution/origin ordering.
    candidates = [t for t in info.get('thumbnails', []) if isinstance(t, dict)
                  and isinstance(t.get('url'), str) and urlsplit(t['url']).scheme == 'https']
    if candidates:
        return max(enumerate(candidates), key=lambda item: (
            item[1].get('id') == 'cover_original_scale',
            (item[1].get('width') or 0) * (item[1].get('height') or 0),
            str(item[1].get('id', '')).startswith(('origin_cover', 'originCover')),
            item[1].get('preference') or 0, item[0]))[1]['url']
    url = info.get('thumbnail')
    return url if isinstance(url, str) and urlsplit(url).scheme == 'https' else None


def read_cover(ydl, url, *, optional=False):
    try:
        # Goes through the same guarded transport, proxy, cookies and timeout as the video.
        request = Request(url, extensions={'timeout': 5}) if optional else url
        with ydl.urlopen(request) as response:
            body = response.read(10 * 1024 * 1024 + 1)
        valid = (body.startswith(b'\xff\xd8\xff') or body.startswith(b'\x89PNG\r\n\x1a\n')
                 or (body.startswith(b'RIFF') and body[8:12] == b'WEBP'))
        if not valid or len(body) > 10 * 1024 * 1024:
            return None
        return body
    except (OSError, RequestError, TargetError, DownloadError):
        return None


def download_cover(ydl, info, stage):
    url = cover_url(info)
    body = read_cover(ydl, url) if url else None
    if body is None:
        return dict(cover_error='COVER_UNAVAILABLE')
    candidates = [candidate for candidate in info.get('_yingqu_douyin_covers', []) if candidate != url]
    if candidates:
        body = douyin_cover.upgrade(body, candidates, lambda candidate: read_cover(ydl, candidate, optional=True))
    try:
        partial = stage / 'cover.part'
        partial.write_bytes(body)
        partial.rename(stage / 'cover')
        return dict(cover_filename='cover', cover_bytes=len(body))
    except (OSError, RequestError, TargetError, DownloadError):
        return dict(cover_error='COVER_UNAVAILABLE')


def metadata(info, platform):
    video_id = info.get('id')
    if (info.get('_type', 'video') != 'video' or info.get('is_live') or info.get('live_status') == 'is_live'
            or not isinstance(video_id, str) or not re.fullmatch(
                r'[A-Za-z0-9_-]{11}' if platform == 'youtube' else r'\d{1,30}', video_id)):
        raise EngineError('UNSUPPORTED_CONTENT', 'detail')
    canonical = info.get('webpage_url')
    actual, canonical, short = parse_input(canonical)
    if actual != platform or short or input_identity(platform, canonical, 'auto', canonical)['video_id'] != video_id:
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
                author_profile=author_profile.metadata(info, platform),
                duration_ms=duration_ms, resolver='yt-dlp', kind='video',
                formats=public_formats(info, platform == 'youtube'), selected_format_id=info.get('_yingqu_format_id'), **selected)


def input_identity(platform, url, resolver, source_url):
    if resolver not in ('auto', 'yt-dlp', 'dtk'):
        raise EngineError('CONFIG_INVALID', 'resolver')
    album = '/note/' in url or '/photo/' in url
    actual = 'dtk' if resolver == 'dtk' or (resolver == 'auto' and album) else 'yt-dlp'
    if platform == 'youtube' and actual != 'yt-dlp':
        raise EngineError('UNSUPPORTED_CONTENT', 'resolver')
    if actual == 'yt-dlp' and album:
        raise EngineError('UNSUPPORTED_CONTENT', 'resolver')
    video_id = parse_qs(urlsplit(url).query)['v'][0] if platform == 'youtube' else url.rstrip('/').split('/')[-1]
    return dict(platform=platform, video_id=video_id, canonical_url=url,
                kind='image_album' if album else 'video', resolver=actual, source_url=source_url)


def operate(method, params, emit):
    if not isinstance(params, dict):
        raise EngineError('ENGINE_PROTOCOL_ERROR', 'protocol')
    try:
        runtime_dependencies.configure(params.get('runtime_directory'))
        if method == 'get_audio_excerpt':
            emit('audio_excerpt', **audio_excerpt.extract(params))
            return
        if method == 'extract_keyframes':
            emit('keyframes_extracted', **keyframes.extract(params, emit))
            return
        if method == 'probe_local_video':
            emit('local_video_probed', **local_video.probe(params))
            return
        if method == 'transcribe_video':
            emit('transcribed', **transcription.transcribe(params, emit))
            return
        if method == 'extract_cover':
            emit('cover_extracted', **local_cover.extract(params))
            return
        if method in ('check_dependencies', 'install_dependency'):
            result = (runtime_dependencies.status() if method == 'check_dependencies' else
                      runtime_dependencies.install(params.get('tool'), params.get('staging_dir'), emit, params.get('variant')))
            emit('dependencies', result=result)
            return
    except transcription.TranscriptionError as error:
        raise EngineError(str(error), 'transcription') from None
    except keyframes.KeyframeError as error:
        raise EngineError(str(error), 'keyframes') from None
    except audio_excerpt.AudioError as error:
        raise EngineError(str(error), 'audio') from None
    except local_video.ProbeError as error:
        raise EngineError(str(error), 'local_video') from None
    except runtime_dependencies.DependencyError as error:
        raise EngineError(str(error), 'dependencies') from None
    platform, url, short = parse_input(params.get('input'))
    source_url = url
    resolver = params.get('resolver', 'auto')
    identity = None if short else input_identity(platform, url, resolver, source_url)
    if method == 'identify_input' and identity:
        emit('identified', result=identity)
        return
    profiles = params.get('profiles', {})
    if not isinstance(profiles, dict):
        raise EngineError('CONFIG_INVALID', 'config')
    ua, cookies, proxy = profile_config(profiles.get(platform, {}))
    chunk_size = http_chunk_size(profiles.get(platform, {}), platform)
    chunk_configured = 'http_chunk_size' in profiles.get(platform, {})
    downloading = method == 'download_video'
    expected = params.get('expected')
    if resolver not in ('auto', 'yt-dlp', 'dtk'):
        raise EngineError('CONFIG_INVALID', 'resolver')
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

    preview = None
    if not downloading and params.get('preview_dir'):
        preview = Path(params['preview_dir'])
        if not preview.is_absolute() or not preview.is_dir() or preview.is_symlink() or any(preview.iterdir()):
            raise EngineError('IO_ERROR', 'preview')

    audio_context = {}

    def match_filter(info, *, incomplete=False):
        if info.get('_type', 'video') in ('playlist', 'multi_video') or info.get('is_live'):
            raise EngineError('UNSUPPORTED_CONTENT', 'detail')
        if downloading and info.get('id') and info['id'] != expected['video_id']:
            raise EngineError('UPSTREAM_CHANGED', 'identity')
        # The pinned selector callback only receives formats, not top-level language.
        # match_filter sees the complete info before that callback runs.
        for field in ('original_language', 'language'):
            if isinstance(info.get(field), str) and info[field]:
                audio_context[field] = info[field]
        return None

    progress = DownloadProgress(emit)

    logger = SafeLogger()
    options = dict(proxy=proxy or '', http_headers={'User-Agent': ua},
        logger=logger, quiet=True, noprogress=True, cachedir=False, noplaylist=True,
        allowed_extractors=(['Douyin$'] if platform == 'douyin' else ['Youtube$'] if platform == 'youtube'
                            else ['TikTok$', r'vm\.tiktok$']),
        format=lambda context: select_format({**context, **audio_context}, params.get('format_id') if downloading else None, platform == 'youtube'), match_filter=match_filter, progress_hooks=[progress] if downloading else [],
        postprocessor_hooks=[progress.postprocess],
        outtmpl=str(stage / 'video.%(ext)s') if stage else 'unused.%(ext)s',
        overwrites=False, continuedl=False, socket_timeout=30, retries=1,
        retry_sleep_functions={'http': lambda n: 1}, http_chunk_size=chunk_size or 0,
        extractor_retries=0, fragment_retries=0, file_access_retries=0,
        postprocessors=[], fixup='never', external_downloader=None,
        js_runtimes={}, remote_components=set())
    if platform == 'youtube':
        options['js_runtimes'] = javascript_runtime()
        if not options['js_runtimes']:
            raise EngineError('JS_RUNTIME_UNAVAILABLE', 'runtime')
        options['ffmpeg_location'] = installed_tool('ffmpeg')
    try:
        with YoutubeDL(options) as ydl:
            cookie_warning = None
            if cookies:
                add_cookies(ydl.cookiejar, cookies, platform)
            else:
                cookie_warning = load_default_cookies(ydl.cookiejar, platform, logger)
                cookies = {c.name: c.value for c in ydl.cookiejar if not c.is_expired()}
            if platform == 'douyin':
                ydl.add_info_extractor(DouyinIE())
            elif platform == 'tiktok':
                ydl.add_info_extractor(author_profile.TikTokIE())
            ydl._request_director = ydl.build_request_director(
                [functools.partial(GuardedRH, platform=platform)])
            if short:
                with ydl.urlopen(url) as response:
                    resolved_platform, url, still_short = parse_input(response.url)
                if resolved_platform != platform or still_short:
                    raise EngineError('UNSUPPORTED_CONTENT', 'short_link')
            identity = identity or input_identity(platform, url, resolver, source_url)
            actual_resolver = identity['resolver']
            if method == 'identify_input':
                emit('identified', result=identity)
                return
            content = None
            if actual_resolver == 'dtk':
                video_id = url.rstrip('/').split('/')[-1]
                content = dtk_adapter.extract(ydl, platform, video_id, ua, cookies, proxy)
                if str(content.kind) == 'image_album':
                    result = dtk_adapter.metadata(content)
                    result['source_url'] = source_url
                    if cookie_warning:
                        result['cookie_warning'] = cookie_warning
                    if downloading:
                        if (result['video_id'] != expected['video_id']
                                or expected.get('kind', 'video') != 'image_album'
                                or len(result['images']) != expected.get('image_count')):
                            raise EngineError('UPSTREAM_CHANGED', 'identity')
                        # Album counters keep their existing conservative retry policy.
                        ydl.params['retries'] = 0
                        dtk_adapter.download_images(ydl, content, stage, emit)
                    else:
                        if preview:
                            names = dtk_adapter.preview_images(ydl, content, preview)
                            for image, name in zip(result['images'], names):
                                image['preview_filename'] = name
                            result.update(author_profile.download_avatar(ydl,
                                content.author.avatar.url if content.author.avatar else None, preview))
                        emit('resolved', result=result)
                    return
                info = ydl.process_ie_result(dtk_adapter.video_info(content), download=False)
            else:
                info = ydl.extract_info(url, download=False)
            if not isinstance(info, dict):
                raise EngineError('UPSTREAM_CHANGED', 'detail')
            # Selected stream fields can overwrite top-level language with None.
            # Keep the same captured preference for the public choices and selector.
            result = metadata({**info, **audio_context}, platform)
            result['resolver'] = actual_resolver
            result['source_url'] = source_url
            if cookie_warning:
                result['cookie_warning'] = cookie_warning
            if content and content.music:
                result['music'] = dict(title=content.music.title, author=content.music.author)
            if not downloading:
                if preview:
                    result.update(download_cover(ydl, info, preview))
                    result.update(author_profile.download_avatar(ydl,
                        author_profile.avatar_url(ydl, info, platform), preview))
                emit('resolved', result=result)
                return
            if result['video_id'] != expected['video_id'] or expected.get('kind', 'video') != 'video':
                raise EngineError('UPSTREAM_CHANGED', 'identity')
            # The validated single-file format and identity remain in this process;
            # start media I/O only after validation, never forward a URL to Rust.
            if platform == 'douyin':
                # Web 4K CDN variants require the platform root Referer.
                info.setdefault('http_headers', {})['Referer'] = 'https://www.douyin.com/'
            if info.get('requested_formats') and not options.get('ffmpeg_location'):
                raise EngineError('FFMPEG_UNAVAILABLE', 'merge')
            if info.get('requested_formats'):
                from yt_dlp.postprocessor.ffmpeg import FFmpegMergerPP
                if not FFmpegMergerPP(ydl).available:
                    raise EngineError('FFMPEG_UNAVAILABLE', 'merge')
            if chunk_size is not None and (chunk_configured or 'YINGQU_HTTP_CHUNK_SIZE' in os.environ):
                # Global 0 falls back to extractor downloader_options in HttpFD.
                # Apply the explicit mode to the selected streams only.
                for fmt in info.get('requested_formats') or [info]:
                    fmt.setdefault('downloader_options', {})['http_chunk_size'] = options['http_chunk_size']
            progress.prepare(info)
            ydl.process_info(info)
            file = Path(ydl.prepare_filename(info))
            if file.parent != stage or file.is_symlink() or not file.is_file() or file.stat().st_size == 0:
                raise EngineError('IO_ERROR', 'output')
            emit('download_finished', platform=platform, video_id=result['video_id'],
                 filename=file.name, bytes=file.stat().st_size, container=info['ext'],
                 format_id=result['selected_format_id'], width=result['width'], height=result['height'],
                 **({'transferred_bytes': progress.transferred} if platform == 'youtube' else {}),
                 **download_cover(ydl, info, stage))
    except DtkError as error:
        raise EngineError('UPSTREAM_CHANGED', 'dtk_signing') from None
    except DownloadError as error:
        code = error_code(error)
        if code == 'UPSTREAM_CHANGED':
            code = 'MEDIA_MERGE_FAILED' if progress.merging else logger.code
        if code == 'ACCESS_DENIED' and cookie_warning:
            code = 'BROWSER_COOKIES_UNAVAILABLE'
        raise EngineError(code, 'yt_dlp') from None
    except TargetError as error:
        raise EngineError(error.code, 'network_target') from None
    except RequestError as error:
        if isinstance(error.cause, TargetError):
            raise EngineError(error.cause.code, 'network_target') from None
        raise EngineError(error_code(error), 'request') from None


def check_version():
    import dtk
    if __version__ != YTDLP_VERSION or dtk.__version__ != dtk_adapter.VERSION:
        raise EngineError('ENGINE_UNAVAILABLE', 'version')
