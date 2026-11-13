"""Run the Tencent share-link experiment and emit only sanitized observations."""
import argparse
import json
import re
import secrets
import subprocess
import sys
import tempfile
import time
from http.cookiejar import Cookie, DefaultCookiePolicy
from http.cookies import SimpleCookie
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'python-engine'))

from browser_cookies import load_default_cookies_for_url
from network import GuardedRH, validate_target
from yt_dlp.cookies import YDLLogger, YoutubeDLCookieJar
from yt_dlp.networking import Request
from yt_dlp.networking.exceptions import HTTPError

YUANBAO = 'https://yuanbao.tencent.com'
CHANNELS = 'https://channels.weixin.qq.com'
PARSE_URL = YUANBAO + '/api/weixin/get_parse_result'
FEED_URL = CHANNELS + '/finder-preview/api/feed/get_feed_info'
PLAY_PAGE = CHANNELS + '/finder-preview/pages/feed'
USER_AGENT = ('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 '
              '(KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36')
JSON_LIMIT = 1024 * 1024
MEDIA_LIMIT = 128 * 1024 * 1024


def jar():
    return YoutubeDLCookieJar(policy=DefaultCookiePolicy(
        strict_ns_domain=DefaultCookiePolicy.DomainStrictNonDomain))


def share_url(value):
    if not re.fullmatch(r'https://weixin\.qq\.com/sph/[A-Za-z0-9_-]{1,128}(?:[?#][^\s]*)?', value):
        raise ValueError('INVALID_SHARE_URL')
    return 'https://weixin.qq.com' + urlsplit(value).path


def manual_jar(path):
    # The file holds a Cookie header value, never a response or an arbitrary header.
    raw = path.read_text(encoding='utf-8-sig').rstrip('\r\n')
    if len(raw) > 16384 or any(ord(c) < 32 or ord(c) > 126 for c in raw):
        raise ValueError('INVALID_COOKIE_FILE')
    parsed = SimpleCookie()
    parsed.load(raw)
    if not parsed:
        raise ValueError('INVALID_COOKIE_FILE')
    result = jar()
    for key, morsel in parsed.items():
        result.set_cookie(Cookie(0, key, morsel.coded_value, None, False,
                                 'yuanbao.tencent.com', False, False, urlsplit(PARSE_URL).path,
                                 True, True, None, True, None, None, {}))
    return result


class ProbeLogger(YDLLogger):
    def debug(self, *args, **kwargs):
        pass

    info = warning = error = debug


def transport(target, cookies=None):
    return GuardedRH(platform=target, proxies={}, timeout=15,
                     cookiejar=jar() if cookies is None else cookies, logger=ProbeLogger())


def api(target, url, origin, payload, cookies=None):
    request = Request(url, data=json.dumps(payload).encode('utf-8'), headers={
        'Accept': 'application/json', 'Content-Type': 'application/json',
        'Origin': origin, 'Referer': origin + '/', 'User-Agent': USER_AGENT,
    })
    with transport(target, cookies) as client:
        try:
            response = client.send(request)
        except HTTPError as error:
            response = error.response
        with response:
            raw = response.read(JSON_LIMIT + 1)
            if len(raw) > JSON_LIMIT:
                return response.status, None, 'RESPONSE_TOO_LARGE'
            try:
                value = json.loads(raw)
            except (UnicodeError, ValueError):
                value = None
            return response.status, value if isinstance(value, dict) else None, (
                None if isinstance(value, dict) else 'INVALID_JSON')


def play_params(playable):
    if not isinstance(playable, str) or len(playable) > 8192:
        raise ValueError('INVALID_PLAYABLE_URL')
    parts = urlsplit(playable)
    if (parts.scheme != 'https' or parts.hostname != 'channels.weixin.qq.com'
            or parts.path != '/finder-preview/pages/feed' or parts.username is not None
            or parts.password is not None or parts.port not in (None, 443)):
        raise ValueError('INVALID_PLAYABLE_URL')
    params = parse_qs(parts.query, max_num_fields=24, keep_blank_values=True)
    if any(len(params.get(key, [])) != 1 or not 0 < len(params[key][0]) <= 4096
           for key in ('eid', 'token')):
        raise ValueError('INVALID_PLAYABLE_PARAMS')
    return params['eid'][0], params['token'][0]


def feed_request(eid, token):
    query = urlencode({'_rid': f'{int(time.time()):x}-{secrets.token_hex(4)}', '_pageUrl': PLAY_PAGE})
    return api('weixin-feed', FEED_URL + '?' + query, CHANNELS,
               {'baseReq': {'generalToken': token}, 'exportId': eid})


def as_object(value):
    return value if isinstance(value, dict) else {}


def integer(value):
    return value if type(value) is int else None


def protection_fields(value):
    names = {'decodekey', 'encryptkey', 'encrypted', 'encryption', 'drm', 'drmtype'}
    if isinstance(value, dict):
        return (names & {key.lower() for key in value}) | set().union(
            *(protection_fields(item) for item in value.values()))
    if isinstance(value, list):
        return set().union(*(protection_fields(item) for item in value))
    return set()


def download_media(url):
    validate_target(url, 'weixin-media')
    with tempfile.TemporaryDirectory(prefix='weixin-probe-') as tmp:
        path = Path(tmp) / 'sample.mp4'
        started = time.monotonic()
        with transport('weixin-media') as client:
            request = Request(url, headers={'User-Agent': USER_AGENT,
                                            'Referer': 'https://weixin.qq.com/'})
            try:
                response = client.send(request)
            except HTTPError as error:
                with error.response as response:
                    return {'status': 'MEDIA_HTTP_ERROR', 'http': response.status}
            with response, path.open('wb') as output:
                content_type = response.headers.get('Content-Type', '').split(';', 1)[0].lower()
                if response.status != 200 or content_type in ('text/html', 'application/json'):
                    return {'status': 'MEDIA_RESPONSE_REJECTED', 'http': response.status}
                size = 0
                while chunk := response.read(65536):
                    size += len(chunk)
                    if size > MEDIA_LIMIT or time.monotonic() - started > 120:
                        return {'status': 'MEDIA_PROBE_LIMIT'}
                    output.write(chunk)
                http = response.status
                redirected = response.url != url
        probe = subprocess.run(
            ['ffprobe', '-v', 'error', '-protocol_whitelist', 'file', '-show_entries',
             'format=format_name:stream=codec_name,codec_type,width,height', '-of', 'json', str(path)],
            capture_output=True, timeout=20, check=False)
        if not size or probe.returncode:
            return {'status': 'MEDIA_PROBE_FAILED', 'http': http, 'bytes': size}
        result = json.loads(probe.stdout)
        tracks = result.get('streams', [])
        if not any(track.get('codec_type') == 'video' for track in tracks):
            return {'status': 'MEDIA_NO_VIDEO_TRACK', 'http': http, 'bytes': size}
        decoded = subprocess.run(
            ['ffmpeg', '-nostdin', '-v', 'error', '-xerror', '-protocol_whitelist', 'file', '-i', str(path),
             '-map', '0:v:0', '-map', '0:a:0?', '-f', 'null', '-'],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120, check=False)
        return {'status': 'PLAYABLE' if not decoded.returncode else 'MEDIA_DECODE_FAILED',
                'http': http, 'redirected': redirected, 'bytes': size,
                'container': result['format']['format_name'], 'tracks': tracks}


def run_sample(url, cookies, download, identities):
    status, body, problem = api('weixin-yuanbao', PARSE_URL, YUANBAO,
                                {'type': 'video_channel_url', 'url': url, 'scene': 1}, cookies)
    body = as_object(body)
    data = as_object(body.get('data'))
    result = {'yuanbao_http': status, 'yuanbao_code': integer(body.get('code'))}
    if status == 401:
        code = as_object(body.get('error')).get('code')
        result['authentication_error_code_shape'] = 'string' if isinstance(code, str) else 'other'
        # Print only recognized fixed codes, never error messages or arbitrary field values.
        for known in ('Unauthorized', 'UNAUTHORIZED', 'AUTHENTICATION_REQUIRED', 'LOGIN_REQUIRED',
                      'unauthorized', 'NOT_LOGIN', 'UNAUTHENTICATED', 'login_required',
                      'unlogin', 'not_login', 'AUTH_FAILED'):
            if code == known:
                result['authentication_error_code'] = known
        return result | {'status': 'AUTH_REQUIRED'}
    if status == 429:
        return result | {'status': 'RATE_LIMITED'}
    if status != 200 or problem:
        return result | {'status': problem or 'YUANBAO_HTTP_ERROR'}
    result['yuanbao_fields'] = [k for k in ('wx_export_id', 'cover_url', 'desc', 'author', 'playable_url') if k in data]
    if not data.get('wx_export_id') or not data.get('playable_url'):
        return result | {'status': 'EMPTY_RESULT' if result['yuanbao_code'] == 0 else 'YUANBAO_API_ERROR'}
    eid, token = play_params(data['playable_url'])
    status, body, problem = feed_request(eid, token)
    body = as_object(body)
    details = as_object(body.get('data'))
    feed = as_object(details.get('feedInfo'))
    result.update(feed_http=status, feed_code=integer(body.get('errCode')),
                  feed_error_type=integer(as_object(details.get('errMsg')).get('type')))
    if not 200 <= status < 300 or problem or result['feed_code'] != 0 or not feed:
        return result | {'status': problem or 'FEED_API_ERROR'}
    result.update(status='FEED_RESOLVED', media_type=integer(feed.get('mediaType')),
                  feed_fields=[k for k in ('description', 'coverUrl', 'createtime', 'duration',
                                           'picInfo', 'videoUrl', 'h264VideoInfo', 'h265VideoInfo') if k in feed],
                  protection_fields=sorted(protection_fields(data) | protection_fields(details)))
    identities.append((data['wx_export_id'], eid, as_object(details.get('sceneInfo')).get('dynamicExportId')))
    candidates = []
    seen = set()
    for field, info in [('h264', as_object(feed.get('h264VideoInfo'))),
                        ('h265', as_object(feed.get('h265VideoInfo'))), ('default', feed)]:
        media_url = info.get('videoUrl')
        if not isinstance(media_url, str) or not media_url:
            continue
        candidate = {'field': field}
        try:
            validate_target(media_url, 'weixin-media')
            candidate['host'] = urlsplit(media_url).hostname
            if media_url in seen:
                candidate['status'] = 'DUPLICATE_URL'
            elif download:
                candidate.update(download_media(media_url))
            else:
                candidate['status'] = 'NOT_DOWNLOADED'
            seen.add(media_url)
        except Exception as error:
            candidate['status'] = type(error).__name__
        candidates.append(candidate)
    return result | {'candidates': candidates}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('urls', nargs='+')
    parser.add_argument('--cookie-file', type=Path)
    parser.add_argument('--browser', action='store_true')
    parser.add_argument('--download', action='store_true', help='download and decode authorized samples')
    parser.add_argument('--compare-first-two', action='store_true',
                        help='compare identities of the first two URLs for the same work')
    args = parser.parse_args()
    try:
        urls = [share_url(value) for value in args.urls]
        manual = manual_jar(args.cookie_file) if args.cookie_file else None
    except Exception as error:
        print(json.dumps({'status': 'INVALID_INPUT', 'error_type': type(error).__name__}))
        return 2
    browser = None
    if args.browser:
        name, status, browser = load_default_cookies_for_url(PARSE_URL)
        print(json.dumps({'mode': 'browser', 'browser': name, 'status': status}), flush=True)
    comparisons = {}
    for index, url in enumerate(urls, 1):
        for mode, cookies in [('anonymous', None), ('manual', manual), ('browser', browser)]:
            if mode != 'anonymous' and cookies is None:
                continue
            try:
                identities = []
                result = run_sample(url, cookies, args.download, identities)
                if identities:
                    comparisons.setdefault((index, mode), identities[0])
            except Exception as error:
                result = {'status': 'PROBE_ERROR', 'error_type': type(error).__name__}
            print(json.dumps({'sample': index, 'mode': mode, **result}, ensure_ascii=True), flush=True)
    if args.compare_first_two:
        for mode in ('anonymous', 'manual', 'browser'):
            left, right = comparisons.get((1, mode)), comparisons.get((2, mode))
            if left and right:
                print(json.dumps({'mode': mode, 'identity_equal': {
                    name: a == b if a and b else None for name, a, b in zip(
                        ('wx_export_id', 'eid', 'dynamicExportId'), left, right)}}), flush=True)
    return 0


if __name__ == '__main__':
    sys.exit(main())
