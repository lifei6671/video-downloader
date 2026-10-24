import io
import json

import pytest
from yt_dlp import YoutubeDL
from yt_dlp.extractor.tiktok import TikTokIE as UpstreamTikTokIE
from yt_dlp.extractor.youtube import YoutubeTabIE

import author_profile
import engine
from network import TargetError, validate_target


@pytest.mark.parametrize('platform,node,expected', [
    ('douyin', {'uid': '123', 'sec_uid': 'MS4w.author', 'unique_id': 'handle', 'nickname': '作者昵称',
                'avatar_larger': {'url_list': ['https://p.douyinpic.com/avatar.webp']}}, 'MS4w.author'),
    ('tiktok', {'id': '456', 'secUid': 'secondary', 'uniqueId': 'handle', 'nickname': '作者昵称',
                'avatarLarger': 'https://p.tiktokcdn.com/avatar.webp'}, '456'),
])
def test_author_identity_nickname_and_avatar_are_separate(platform, node, expected):
    info = author_profile.attach({}, node, platform)
    profile = author_profile.metadata(info, platform)
    assert profile == dict(id=expected, nickname='作者昵称', handle='handle', kind='user')
    assert 'avatar.webp' in author_profile.avatar_url(None, info, platform)
    assert 'https://' not in json.dumps(profile)


def test_tiktok_wrapper_preserves_upstream_result_and_captures_author(monkeypatch):
    monkeypatch.setattr(UpstreamTikTokIE, '_parse_aweme_video_web', lambda *a, **k: {'id': '123'})
    info = author_profile.TikTokIE()._parse_aweme_video_web(
        {'author': {'id': '456', 'uniqueId': 'name', 'nickname': '昵称', 'avatarThumb': 'https://p.tiktokcdn.com/a'}},
        'https://www.tiktok.com/@name/video/123', '123')
    assert info['id'] == '123'
    assert author_profile.metadata(info, 'tiktok')['nickname'] == '昵称'


def test_youtube_uses_channel_id_not_handle_for_identity():
    profile = author_profile.metadata(dict(channel_id='UC' + 'a' * 22, uploader_id='@handle',
                                           channel='频道昵称', uploader='频道昵称'), 'youtube')
    assert profile == dict(id='UC' + 'a' * 22, nickname='频道昵称', handle='@handle', kind='channel')


def test_root_shaped_tiktok_and_incomplete_author_preserve_extracted_identity():
    detail = {'author': 'handle', 'authorId': '123', 'nickname': 'Display Name'}
    info = author_profile.TikTokIE()._parse_aweme_video_web(detail, 'https://www.tiktok.com/@handle/video/1', '1', extract_flat=True)
    assert author_profile.metadata(info, 'tiktok') == dict(id='123', nickname='Display Name', handle='handle', kind='user')
    info['_yingqu_author'] = {'kind': 'user'}
    assert author_profile.metadata(info, 'tiktok')['id'] == '123'
    assert author_profile.metadata(info, 'tiktok')['nickname'] == 'Display Name'


@pytest.mark.parametrize('avatar', [{'url_list': ['https://[']}, {'url_list': 123}, None])
def test_malformed_optional_avatar_does_not_break_author_or_video(avatar, tmp_path):
    profile, url = author_profile.from_node({'uid': '123', 'nickname': '昵称', 'avatar_larger': avatar}, 'douyin')
    assert profile['id'] == '123' and profile['nickname'] == '昵称' and url is None
    assert author_profile.download_avatar(None, 'https://[', tmp_path) == {}


def test_youtube_avatar_reads_only_matching_channel_header(monkeypatch):
    channel = 'UC' + 'a' * 22
    data = {'metadata': {'channelMetadataRenderer': {'externalId': channel,
            'avatar': {'thumbnails': [{'url': 'https://yt3.ggpht.com/avatar=s88', 'width': 88}]}}}}
    calls = []
    def webpage(self, request, video_id, **kwargs):
        calls.append((request.url, request.extensions))
        return '<html></html>'
    monkeypatch.setattr(YoutubeTabIE, '_download_webpage', webpage)
    monkeypatch.setattr(YoutubeTabIE, 'extract_yt_initial_data', lambda *a, **k: data)
    with YoutubeDL({'quiet': True}) as ydl:
        assert author_profile.avatar_url(ydl, {'channel_id': channel}, 'youtube') == 'https://yt3.ggpht.com/avatar=s88'
        data['metadata']['channelMetadataRenderer']['externalId'] = 'UC' + 'b' * 22
        assert author_profile.avatar_url(ydl, {'channel_id': channel}, 'youtube') is None
        assert author_profile.avatar_url(ydl, {'channel_id': 'https://evil.test'}, 'youtube') is None
    assert len(calls) == 2
    assert all(url == f'https://www.youtube.com/channel/{channel}/about' and ext['timeout'] == 5
               for url, ext in calls)


@pytest.mark.parametrize('body,accepted', [(b'\x89PNG\r\n\x1a\nfixture', True),
    (b'<html>login</html>', False), (b'\xff\xd8\xff' + b'x' * (2 * 1024 * 1024), False)], ids=['png', 'html', 'oversized'])
def test_avatar_staging_is_bounded_and_never_returns_remote_url(tmp_path, body, accepted):
    class Ydl:
        def urlopen(self, request):
            assert request.extensions['timeout'] == 5
            return io.BytesIO(body)
    result = author_profile.download_avatar(Ydl(), 'https://p.douyinpic.com/avatar?secret=token', tmp_path)
    assert bool(result) == accepted
    assert 'secret' not in json.dumps(result)
    assert list(tmp_path.iterdir()) == ([tmp_path / 'author-avatar'] if accepted else [])
    if accepted:
        assert (tmp_path / 'author-avatar').read_bytes() == body


def test_avatar_network_failure_is_optional(tmp_path):
    class Ydl:
        def urlopen(self, request):
            raise TargetError()
    assert author_profile.download_avatar(Ydl(), 'https://evil.test/a', tmp_path) == {}
    assert author_profile.from_node({'avatar_larger': {'url_list': None}}, 'douyin')[1] is None


def test_youtube_avatar_hosts_stay_platform_scoped():
    validate_target('https://yt3.ggpht.com/a', 'youtube')
    validate_target('https://yt3.googleusercontent.com/a', 'youtube')
    for url, platform in [('https://yt3.ggpht.com/a', 'douyin'),
                          ('https://other.googleusercontent.com/a', 'youtube')]:
        with pytest.raises(TargetError):
            validate_target(url, platform)
