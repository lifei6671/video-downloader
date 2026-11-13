"""Read the signed author feed, emitting bounded metadata pages."""
import functools
import random
import re
import time
from datetime import UTC, datetime
from urllib.parse import urlsplit

from yt_dlp import YoutubeDL
from yt_dlp.networking import Request
from yt_dlp.networking.exceptions import HTTPError, RequestError

import dtk_adapter
from dtk.core.errors import DtkError
from dtk.platforms.douyin.parser import detect_risk_control
from network import GuardedRH, TargetError
from cookie_scope import scoped_jar

API_URL = 'https://www.douyin.com/aweme/v1/web/aweme/post/'
LOGIN_MARKERS = {'sessionid', 'sessionid_ss', 'sid_tt', 'sid_guard', 'uid_tt', 'sid_ucp_v1'}
MAX_PAGES = 1000


def profile_link(text):
    from engine import EngineError
    urls = re.findall(r'https?://[^\s<>"\u3000]+', text) if isinstance(text, str) and len(text) <= 4096 else []
    try:
        if len(urls) != 1:
            raise ValueError()
        parts = urlsplit(urls[0].rstrip('.,;!?，。；！）》)'))
        if parts.scheme != 'https' or parts.username or parts.password or parts.port not in (None, 443):
            raise ValueError()
        if parts.hostname == 'v.douyin.com' and re.fullmatch(r'/[A-Za-z0-9_-]{1,128}/?', parts.path):
            return parts.geturl(), None
        prefix = {'www.douyin.com': '/user/', 'douyin.com': '/user/',
                  'www.iesdouyin.com': '/share/user/'}.get(parts.hostname)
        if prefix and (match := re.fullmatch(prefix + r'([A-Za-z0-9_-]{10,256})/?', parts.path)):
            return parts.geturl(), match[1]
    except ValueError:
        pass
    raise EngineError('INVALID_AUTHOR_LINK', 'author_link')


def has_login(jar):
    return any(cookie.name in LOGIN_MARKERS and cookie.value
               for cookie in jar.get_cookies_for_url(API_URL))


def guarded_client(proxy, *, login_session=False):
    from engine import SafeLogger
    ydl = YoutubeDL(dict(quiet=True, logger=SafeLogger(), proxy=proxy or '',
                         socket_timeout=30, cachedir=False, allowed_extractors=[], js_runtimes={}))
    if login_session:
        ydl.cookiejar = scoped_jar()
    ydl._request_director = ydl.build_request_director([
        functools.partial(GuardedRH, platform='douyin')])
    return ydl


def expand_share(ydl, url, ua):
    from engine import EngineError, error_code
    try:
        with ydl.urlopen(Request(url, headers={'User-Agent': ua})) as response:
            return response.url
    except (RequestError, TargetError) as error:
        raise EngineError(error_code(error), 'author_link') from None


def identify_link(params, emit):
    from engine import EngineError, parse_input, profile_config
    text = params.get('input')
    try:
        url, author_id = profile_link(text)
    except EngineError as error:
        platform, url, short = parse_input(text)
        if platform != 'douyin' or not short:
            raise error
        author_id = None
    if author_id is None:
        ua, _, proxy = profile_config(params.get('profile', {}))
        with guarded_client(proxy) as ydl:
            url = expand_share(ydl, url, ua)
        try:
            _, author_id = profile_link(url)
        except EngineError:
            platform, canonical, short = parse_input(url)
            if platform != 'douyin' or short:
                raise EngineError('UNSUPPORTED_CONTENT', 'short_link')
            emit('identified', result=dict(kind='work', canonical_url=canonical))
            return
        if author_id is None:
            raise EngineError('UNSUPPORTED_CONTENT', 'short_link')
    emit('identified', result=dict(kind='author', author_id=author_id))


def parse_page(payload, author_id):
    from engine import EngineError
    stage = 'author_list'
    if not isinstance(payload, dict):
        raise EngineError('UPSTREAM_CHANGED', stage)
    if detect_risk_control(payload):
        raise EngineError('RISK_CONTROL', stage)
    if type(payload.get('status_code')) is not int or payload['status_code'] != 0:
        raise EngineError('UPSTREAM_CHANGED', stage)
    more = payload.get('has_more')
    if type(more) not in (int, bool) or more not in (0, 1) or not isinstance(payload.get('aweme_list'), list):
        raise EngineError('UPSTREAM_CHANGED', stage)
    if len(payload['aweme_list']) > 200:
        raise EngineError('AUTHOR_SCAN_LIMIT', stage)
    own_or_unverified, skipped = [], 0
    for item in payload['aweme_list']:
        author = item.get('author') if isinstance(item, dict) else None
        sec_uid = author.get('sec_uid') if isinstance(author, dict) else None
        if isinstance(sec_uid, str) and re.fullmatch(r'[A-Za-z0-9_-]{10,256}', sec_uid) and sec_uid != author_id:
            skipped += 1
        else:
            own_or_unverified.append(item)
    try:
        page = dtk_adapter.DOUYIN.parse_author_posts(
            {**payload, 'aweme_list': own_or_unverified}, fetched_at=datetime.now(UTC))
    except DtkError as error:
        raise EngineError('RISK_CONTROL' if str(error.code) == 'UPSTREAM_RISK_CONTROL'
                          else 'UPSTREAM_CHANGED', stage) from None
    items = []
    for post in page.items:
        if (not re.fullmatch(r'[A-Za-z0-9_-]{10,256}', post.author.sec_uid or '')
                or not re.fullmatch(r'[0-9]{1,30}', post.content_id)):
            raise EngineError('UPSTREAM_CHANGED', stage)
        kind = str(post.kind)
        if post.author.sec_uid != author_id or post.is_private or post.is_deleted or kind not in ('video', 'image_album'):
            skipped += 1
            continue
        items.append(dict(video_id=post.content_id, kind=kind, title=(post.title or '')[:512],
                          author=(post.author.nickname or '')[:256], author_id=post.author.sec_uid,
                          published_at=int(post.created_at.timestamp()) if post.created_at else None))
    return page, items, skipped


def scan(params, emit):
    from engine import EngineError, add_cookies, error_code, profile_config
    url, author_id = profile_link(params.get('input'))
    cursor = params.get('cursor')
    completed = params.get('completed_pages', 0)
    if (type(completed) is not int or not 0 <= completed < MAX_PAGES
            or (completed == 0 and cursor is not None)
            or (completed > 0 and (not isinstance(cursor, str)
                                   or not re.fullmatch(r'[0-9]{1,32}', cursor) or cursor == '0'))):
        raise EngineError('AUTHOR_CURSOR_INVALID', 'author_list')
    profile = params['profile']
    ua, cookies, proxy = profile_config(profile)
    login_session = profile.get('login_session', False)
    with guarded_client(proxy, login_session=login_session) as ydl:
        if author_id is None:
            _, author_id = profile_link(expand_share(ydl, url, ua))
            if author_id is None:
                raise EngineError('INVALID_AUTHOR_LINK', 'author_link')
        add_cookies(ydl.cookiejar, cookies, 'douyin', login_session=login_session)
        if not has_login(ydl.cookiejar):
            raise EngineError('AUTHOR_LOGIN_REQUIRED', 'author_list')

        started = time.monotonic() if cursor is not None else None
        cursors = {cursor} if cursor is not None else set()
        for number in range(completed + 1, MAX_PAGES + 1):
            for attempt in range(2):
                if started is not None:
                    wait = max(0, 8.5 - (time.monotonic() - started))
                    if attempt == 0:
                        wait += random.randint(500, 1200) / 1000
                    time.sleep(wait)
                started = time.monotonic()
                try:
                    payload = dtk_adapter.request_json(
                        ydl, dtk_adapter.DOUYIN, 'douyin.author_posts', ua, None, proxy,
                        sec_user_id=author_id, cursor=cursor, count=20)
                except HTTPError as error:
                    code = 'AUTHOR_LOGIN_REQUIRED' if error.status == 401 else error_code(error)
                    raise EngineError(code, 'author_list') from None
                except RequestError as error:
                    if attempt or error_code(error) not in ('TIMEOUT', 'NETWORK_ERROR'):
                        raise EngineError(error_code(error), 'author_list') from None
                except (ValueError, DtkError, TargetError) as error:
                    raise EngineError(error_code(error), 'author_list') from None
                else:
                    break
            page, items, skipped = parse_page(payload, author_id)
            if page.has_more and (not re.fullmatch(r'[0-9]{1,32}', page.cursor or '')
                                  or page.cursor == '0' or page.cursor in cursors):
                raise EngineError('AUTHOR_CURSOR_INVALID', 'author_list')
            emit('author_scan_page', author_id=author_id, page=number, items=items,
                 skipped=skipped, has_more=page.has_more, next_cursor=page.cursor)
            if not page.has_more:
                emit('author_scan_finished', result={'pages': number})
                return
            cursor = page.cursor
            cursors.add(cursor)
        raise EngineError('AUTHOR_SCAN_LIMIT', 'author_list')
