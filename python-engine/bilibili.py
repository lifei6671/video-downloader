"""Canonical single-part Bilibili identities; extraction stays in pinned yt-dlp."""
import re
from urllib.parse import parse_qs, urlsplit, urlunsplit

HOSTS = ('www.bilibili.com', 'bilibili.com', 'm.bilibili.com')
VIDEO_ID = r'BV[0-9A-Za-z]{10}'


def parse_link(parts):
    from engine import EngineError
    if parts.hostname == 'b23.tv':
        if not re.fullmatch(r'/[0-9A-Za-z]+/?', parts.path):
            raise EngineError('UNSUPPORTED_CONTENT', 'input')
        return 'bilibili', 'https://b23.tv' + parts.path, True
    match = re.fullmatch(r'/video/(' + VIDEO_ID + r')/?', parts.path)
    pages = parse_qs(parts.query, keep_blank_values=True).get('p', ['1'])
    if not match or len(pages) != 1 or not re.fullmatch(r'[1-9][0-9]{0,3}', pages[0]):
        raise EngineError('UNSUPPORTED_CONTENT', 'input')
    return 'bilibili', f'https://www.bilibili.com/video/{match[1]}/?p={pages[0]}', False


def identity(url):
    parts = urlsplit(url)
    return parts.path.strip('/').split('/')[-1] + '_p' + parse_qs(parts.query)['p'][0]


def normalize_thumbnails(info):
    # The official extractor returns HTTP image URLs; the same image CDN serves HTTPS.
    def https(url):
        if not isinstance(url, str):
            return url
        try:
            parts = urlsplit(url)
            port = parts.port
        except ValueError:
            return url
        if (parts.scheme == 'http' and parts.hostname
                and any(parts.hostname == domain or parts.hostname.endswith('.' + domain)
                        for domain in ('hdslb.com', 'biliimg.com'))
                and not parts.username and not parts.password and port in (None, 80)):
            return urlunsplit(('https', parts.hostname, parts.path, parts.query, ''))
        return url
    info['thumbnail'] = https(info.get('thumbnail'))
    for thumbnail in info.get('thumbnails') or []:
        if isinstance(thumbnail, dict):
            thumbnail['url'] = https(thumbnail.get('url'))
