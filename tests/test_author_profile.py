import io
import json

import pytest
from yt_dlp import YoutubeDL
from yt_dlp.extractor.tiktok import TikTokIE as UpstreamTikTokIE
from yt_dlp.extractor.youtube import YoutubeTabIE

import author_profile
import engine
from cookie_scope import scoped_jar
from douyin_author import API_URL, guarded_client
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


def test_embedded_login_cookies_stay_on_their_exact_hosts():
    with YoutubeDL({'quiet': True}) as ydl:
        ydl.cookiejar = scoped_jar()
        for platform, sibling in [('douyin', 'sso.douyin.com'), ('xiaohongshu', 'edith.xiaohongshu.com'),
                                  ('tiktok', 'vm.tiktok.com')]:
            host = f'www.{platform}.com'
            engine.add_cookies(ydl.cookiejar, {'sid_tt': 'synthetic'}, platform, login_session=True)
            assert ydl.cookiejar.get_cookie_header(f'https://{host}/') == 'sid_tt=synthetic'
            assert not ydl.cookiejar.get_cookie_header(f'https://{sibling}/')
            assert not ydl.cookiejar.get_cookie_header(f'https://sibling.{host}/')
            assert not ydl.cookiejar.get_cookie_header(f'http://{host}/')
        extractor = author_profile.TikTokIE(ydl, login_session=True)
        extractor._set_cookie('media.tiktokcdn.com', 'sid_tt', 'synthetic')
        assert not ydl.cookiejar.get_cookie_header('https://media.tiktokcdn.com/video')

    with guarded_client(None, login_session=True) as ydl:
        engine.add_cookies(ydl.cookiejar, {'sessionid': 'synthetic'}, 'douyin', login_session=True)
        handler, = ydl._request_director.handlers.values()
        assert handler.cookiejar.get_cookie_header(API_URL) == 'sessionid=synthetic'


def test_tiktok_wrapper_preserves_upstream_result_and_captures_author(monkeypatch):
    monkeypatch.setattr(UpstreamTikTokIE, '_parse_aweme_video_web', lambda *a, **k: {'id': '123'})
    info = author_profile.TikTokIE()._parse_aweme_video_web(
        {'author': {'id': '456', 'uniqueId': 'name', 'nickname': '昵称', 'signature': ' TikTok 简介 ',
                    'avatarThumb': 'https://p.tiktokcdn.com/a'}},
        'https://www.tiktok.com/@name/video/123', '123')
    assert info['id'] == '123'
    profile = author_profile.metadata(info, 'tiktok')
    assert profile['nickname'] == '昵称'
    assert profile['bio'] == 'TikTok 简介'


@pytest.mark.parametrize('signature,bio', [(' 抖音简介 ', '抖音简介'), (' ', None), (None, None), (123, None)])
def test_optional_signature_is_a_text_bio(signature, bio):
    info = author_profile.attach({}, {'uid': '123', 'signature': signature}, 'douyin')
    profile = author_profile.metadata(info, 'douyin')
    assert profile.get('bio') == bio
    assert ('bio' in profile) == (bio is not None)


def test_douyin_optional_details_keep_zero_and_separate_profile_region_from_ip_label():
    node = {'sec_uid': 'MS4wLjABfixture', 'follower_count': '12345', 'following_count': 0,
            'total_favorited': 456, 'user_age': 28, 'country': '中国', 'province': '上海',
            'city': '上海', 'ip_location': 'IP属地：浙江'}
    profile = author_profile.metadata(author_profile.attach({}, node, 'douyin'), 'douyin')
    assert author_profile.downloaded_profile({'author_profile': profile}) == {
        'id': 'MS4wLjABfixture', 'follower_count': 12345, 'following_count': 0,
        'likes_count': 456, 'age': 28, 'region': '中国 上海', 'ip_location': '浙江'}
    assert author_profile.public_details('douyin', {
        'follower_count': True, 'total_favorited': -1, 'user_age': 28, 'show_age': False,
        'birthday': '2000-01-01', 'province': '未知', 'ip_location': 'IP属地：'}) == {'age_hidden': True}


def test_tiktok_uses_author_counters_field_by_field(monkeypatch):
    monkeypatch.setattr(UpstreamTikTokIE, '_parse_aweme_video_web', lambda *a, **k: {'id': '123'})
    info = author_profile.TikTokIE()._parse_aweme_video_web({
        'author': {'id': '456', 'region': 'US', 'age': 22, 'showAge': 'FALSE'},
        'authorStatsV2': {'followerCount': '0', 'heartCount': '325'},
        'authorStats': {'followerCount': 99, 'followingCount': 20, 'heartCount': 999},
        'stats': {'diggCount': 77},
    })
    profile = author_profile.metadata(info, 'tiktok')
    assert author_profile.downloaded_profile({'author_profile': profile}) == {
        'id': '456', 'follower_count': 0, 'following_count': 20, 'likes_count': 325,
        'region': 'US', 'age_hidden': True}
    assert author_profile.public_details('tiktok', {'id': '456', 'region': 'ZZ'},
                                         {'stats': {'diggCount': 77}, 'region': 'GB'}) == {}
    root = {'authorId': '456', 'region': 'GB', 'age': 22, 'authorStats': {'heart': 325}}
    info = author_profile.TikTokIE()._parse_aweme_video_web(root)
    assert author_profile.downloaded_profile({'author_profile': author_profile.metadata(info, 'tiktok')}) == {
        'id': '456', 'likes_count': 325}


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
