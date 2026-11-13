"""Author metadata only; keep remote avatar addresses inside the operation process."""
import re
from urllib.parse import urlsplit

from yt_dlp.extractor.tiktok import TikTokIE as UpstreamTikTokIE
from yt_dlp.networking import Request
from yt_dlp.networking.exceptions import RequestError
from yt_dlp.utils import DownloadError, ExtractorError
from yt_dlp.utils.traversal import traverse_obj

from network import TargetError

PUBLIC_FIELDS = ('follower_count', 'following_count', 'likes_count', 'age', 'region', 'ip_location')
DETAIL_FIELDS = ('bio', *PUBLIC_FIELDS, 'age_hidden')


def text(value):
    return str(value).strip()[:256] if type(value) in (str, int) and value else None


def normalize_bio(value):
    if isinstance(value, str):
        return value.strip()[:4096] or None
    return None


def public_integer(value):
    if isinstance(value, str) and re.fullmatch(r'[0-9]{1,16}', value):
        value = int(value)
    return value if type(value) is int and 0 <= value <= (1 << 53) - 1 else None


def public_label(value):
    if isinstance(value, str):
        value = value.strip()
        if value.lower() not in ('', '-', '-1', '0', 'unknown', 'null', '未知', '暂无'):
            return value[:120]
    return None


def public_details(platform, node, work=None):
    if not isinstance(node, dict):
        return {}
    work = work if isinstance(work, dict) else {}
    if node is work:
        node = {}
    sources = [(node, ('follower_count', 'following_count', 'total_favorited'))]
    if platform == 'tiktok':
        sources = [(work.get(key), ('followerCount', 'followingCount', 'heartCount'))
                   for key in ('authorStatsV2', 'authorStats')] + sources
    values = {}
    for source, names in sources:
        if not isinstance(source, dict):
            continue
        for field, name in zip(PUBLIC_FIELDS[:3], names):
            count = public_integer(source.get(name))
            if count is None and name == 'heartCount':
                count = public_integer(source.get('heart'))
            if count is not None:
                values.setdefault(field, count)
    age = public_integer(node.get('user_age') if platform == 'douyin' else node.get('age'))
    visible = node.get('show_age' if platform == 'douyin' else 'showAge')
    if isinstance(visible, str):
        visible = visible.strip().lower()
    if visible in (False, '0', 'false'):
        values['age_hidden'] = True
    elif age is not None and 1 <= age <= 130:
        values['age'] = age
    if platform == 'douyin':
        parts = dict.fromkeys(filter(None, (public_label(node.get(key)) for key in ('country', 'province', 'city'))))
        if parts:
            values['region'] = ' '.join(parts)[:120]
        if isinstance(node.get('ip_location'), str):
            ip_location = public_label(re.sub(r'^\s*IP\s*属地\s*[:：]?\s*', '', node['ip_location']))
            if ip_location:
                values['ip_location'] = ip_location
    elif (region := public_label(node.get('region'))) and region not in ('XX', 'ZZ') and re.fullmatch(r'[A-Z]{2}', region):
        values['region'] = region
    return values


def https_url(value):
    if not isinstance(value, str):
        return None
    try:
        parsed = urlsplit(value)
        return value if parsed.scheme == 'https' and parsed.hostname else None
    except ValueError:
        return None


def from_node(node, platform, work=None):
    node = node if isinstance(node, dict) else {}
    profile = dict(id=text(node.get('sec_uid') or node.get('uid')) if platform == 'douyin'
                   else text(node.get('id') or node.get('uid') or node.get('authorId')),
                   nickname=text(node.get('nickname')),
                   handle=text(node.get('unique_id') or node.get('uniqueId') or node.get('short_id') or node.get('author')),
                   kind='user')
    bio = normalize_bio(node.get('signature'))
    if bio is not None:
        profile['bio'] = bio
    profile.update(public_details(platform, node, work))
    avatar = None
    keys = ('avatar_larger', 'avatar_medium', 'avatar_300x300', 'avatar_thumb') if platform == 'douyin' else (
        'avatarLarger', 'avatarMedium', 'avatarThumb', 'avatar_larger', 'avatar_thumb')
    for key in keys:
        value = node.get(key)
        urls = (value.get('url_list') or []) if isinstance(value, dict) else [value]
        if not isinstance(urls, (list, tuple)):
            continue
        avatar = next((url for url in urls if https_url(url)), None)
        if avatar:
            break
    return profile, avatar


def attach(info, node, platform, work=None):
    info['_yingqu_author'], info['_yingqu_author_avatar'] = from_node(node, platform, work)
    return info


class TikTokIE(UpstreamTikTokIE):
    def __init__(self, downloader=None, *, login_session=False):
        super().__init__(downloader)
        self._login_session = login_session

    def _set_cookie(self, domain, name, value, *args, **kwargs):
        # Upstream copies sid_tt to its API and media hosts. Keep embedded credentials web-only.
        if self._login_session and name == 'sid_tt' and domain != 'www.tiktok.com':
            return
        return super()._set_cookie(domain, name, value, *args, **kwargs)

    def _parse_aweme_video_web(self, detail, *args, **kwargs):
        info = super()._parse_aweme_video_web(detail, *args, **kwargs)
        node = next((detail[key] for key in ('authorInfo', 'author') if isinstance(detail.get(key), dict)), detail)
        return attach(info, node, 'tiktok', detail)

    def _parse_aweme_video_app(self, detail):
        return attach(super()._parse_aweme_video_app(detail), detail.get('author'), 'tiktok', detail)


def metadata(info, platform):
    result = dict(id=text(info.get('channel_id') if platform == 'youtube' else info.get('uploader_id')),
                nickname=text(info.get('channel') or info.get('uploader')),
                handle=text(info.get('uploader_id') if platform == 'youtube' else info.get('uploader')),
                kind='channel' if platform == 'youtube' else 'user')
    if isinstance(info.get('_yingqu_author'), dict):
        extracted = info['_yingqu_author']
        result = {key: extracted.get(key) or value for key, value in result.items()}
        if platform in ('douyin', 'tiktok'):
            result.update({key: extracted[key] for key in DETAIL_FIELDS if key in extracted})
    return result


def downloaded_profile(video):
    profile = video.get('author_profile') or {}
    details = {key: profile[key] for key in DETAIL_FIELDS if profile.get(key) is not None}
    if profile.get('id') and details:
        return {'id': profile['id'], **details}
    return None


def avatar_url(ydl, info, platform):
    if platform != 'youtube':
        return info.get('_yingqu_author_avatar')
    channel_id = info.get('channel_id')
    # Only the verified channel identity from the single video's result may be queried.
    if not isinstance(channel_id, str) or not re.fullmatch(r'UC[A-Za-z0-9_-]{22}', channel_id):
        return None
    from yt_dlp.extractor.youtube import YoutubeTabIE
    extractor = YoutubeTabIE(ydl)
    url = f'https://www.youtube.com/channel/{channel_id}/about'
    try:
        # Extract the channel header only, without enumerating or downloading its videos.
        webpage = extractor._download_webpage(Request(url, extensions={'timeout': 5}), channel_id, fatal=False)
        if not webpage:
            return None
        data = extractor.extract_yt_initial_data(channel_id, webpage, fatal=False)
        renderer = traverse_obj(data, ('metadata', 'channelMetadataRenderer', {dict})) or {}
        if renderer.get('externalId') != channel_id:
            return None
        thumbnails = extractor._extract_thumbnails(renderer, 'avatar')
        return next((t['url'] for t in reversed(thumbnails)
                     if isinstance(t, dict) and https_url(t.get('url'))), None)
    except (ExtractorError, DownloadError, RequestError, TargetError, OSError, ValueError):
        return None


def download_avatar(ydl, url, stage):
    if not https_url(url):
        return {}
    try:
        with ydl.urlopen(Request(url, extensions={'timeout': 5})) as response:
            body = response.read(2 * 1024 * 1024 + 1)
        valid = (body.startswith(b'\xff\xd8\xff') or body.startswith(b'\x89PNG\r\n\x1a\n')
                 or (body.startswith(b'RIFF') and body[8:12] == b'WEBP'))
        if not valid or len(body) > 2 * 1024 * 1024:
            return {}
        partial = stage / 'author-avatar.part'
        partial.write_bytes(body)
        partial.rename(stage / 'author-avatar')
        return dict(author_avatar_filename='author-avatar', author_avatar_bytes=len(body))
    except (OSError, RequestError, TargetError, DownloadError, ValueError):
        return {}
