"""Canonical single-part Bilibili identities; extraction stays in pinned yt-dlp."""
import json
import re
from urllib.parse import parse_qs, urlsplit, urlunsplit

HOSTS = ('www.bilibili.com', 'bilibili.com', 'm.bilibili.com')
VIDEO_ID = r'BV[0-9A-Za-z]{10}'


def attach_author(ydl, info):
    from author_profile import normalize_bio, text
    from network import TargetError
    from yt_dlp.networking import Request
    from yt_dlp.networking.exceptions import RequestError
    from yt_dlp.utils import DownloadError
    mid = info.get('uploader_id')
    if not isinstance(mid, str) or not re.fullmatch(r'[1-9][0-9]{0,19}', mid):
        return
    try:
        with ydl.urlopen(Request(f'https://api.bilibili.com/x/web-interface/card?mid={mid}',
                                headers={'Referer': 'https://www.bilibili.com/'},
                                extensions={'timeout': 5})) as response:
            body = response.read(1024 * 1024 + 1)
        if len(body) > 1024 * 1024:
            return
        data = json.loads(body)
        card = data.get('data') if isinstance(data, dict) and data.get('code') == 0 else None
        card = card.get('card') if isinstance(card, dict) else None
        if not isinstance(card, dict) or str(card.get('mid')) != mid:
            return
        info['_yingqu_author'] = dict(id=mid, nickname=text(card.get('name')), kind='user')
        bio = normalize_bio(card.get('sign'))
        if bio is not None:
            info['_yingqu_author']['bio'] = bio
        avatar = card.get('face')
        if isinstance(avatar, str):
            image = {'thumbnail': avatar}
            normalize_thumbnails(image)
            info['_yingqu_author_avatar'] = image['thumbnail']
    except (OSError, ValueError, RequestError, TargetError, DownloadError):
        # Optional public profile failures must not fail video extraction/download.
        return


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
