"""Author metadata only; keep remote avatar addresses inside the operation process."""
import re
from urllib.parse import urlsplit

from yt_dlp.extractor.tiktok import TikTokIE as UpstreamTikTokIE
from yt_dlp.networking import Request
from yt_dlp.networking.exceptions import RequestError
from yt_dlp.utils import DownloadError, ExtractorError
from yt_dlp.utils.traversal import traverse_obj

from network import TargetError


def text(value):
    return str(value).strip()[:256] if type(value) in (str, int) and value else None


def https_url(value):
    if not isinstance(value, str):
        return None
    try:
        parsed = urlsplit(value)
        return value if parsed.scheme == 'https' and parsed.hostname else None
    except ValueError:
        return None


def from_node(node, platform):
    node = node if isinstance(node, dict) else {}
    profile = dict(id=text(node.get('sec_uid') or node.get('uid')) if platform == 'douyin'
                   else text(node.get('id') or node.get('uid') or node.get('authorId')),
                   nickname=text(node.get('nickname')),
                   handle=text(node.get('unique_id') or node.get('uniqueId') or node.get('short_id') or node.get('author')),
                   kind='user')
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


def attach(info, node, platform):
    info['_yingqu_author'], info['_yingqu_author_avatar'] = from_node(node, platform)
    return info


class TikTokIE(UpstreamTikTokIE):
    def _parse_aweme_video_web(self, detail, *args, **kwargs):
        info = super()._parse_aweme_video_web(detail, *args, **kwargs)
        node = next((detail[key] for key in ('authorInfo', 'author') if isinstance(detail.get(key), dict)), detail)
        return attach(info, node, 'tiktok')

    def _parse_aweme_video_app(self, detail):
        return attach(super()._parse_aweme_video_app(detail), detail.get('author'), 'tiktok')


def metadata(info, platform):
    result = dict(id=text(info.get('channel_id') if platform == 'youtube' else info.get('uploader_id')),
                nickname=text(info.get('channel') or info.get('uploader')),
                handle=text(info.get('uploader_id') if platform == 'youtube' else info.get('uploader')),
                kind='channel' if platform == 'youtube' else 'user')
    if isinstance(info.get('_yingqu_author'), dict):
        result = {key: info['_yingqu_author'].get(key) or value for key, value in result.items()}
    return result


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
