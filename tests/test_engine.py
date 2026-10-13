import json
from http.cookiejar import CookieJar
from urllib.request import Request

import pytest

import engine
from network import DOMAINS, TargetError, validate_target


@pytest.mark.parametrize('text,platform', [
    ('分享 https://v.douyin.com/abc/ 复制打开', 'douyin'),
    ('https://vt.tiktok.com/abc/', 'tiktok'),
    ('https://www.tiktok.com/@author/video/123?tracking=foo', 'tiktok'),
    ('https://www.douyin.com/video/123', 'douyin')])
def test_single_input(text, platform):
    actual, url, short = engine.parse_input(text)
    assert actual == platform
    assert '?' not in url


@pytest.mark.parametrize('text', [
    '8.20 复制打开抖音，看看【萝卜说剧的作品】口号要致富偷粮库，奇葩匪首自诩智多星 # 警匪片 ... https://v.douyin.com/rgVESAi-rA8/ NWZ:/ N@w.sE :9pm 02/19',
    '视频分析\nhttps://v.douyin.com/rgVESAi-rA8/。\n复制打开抖音',
])
def test_share_copy_extracts_hyphenated_short_link(text):
    assert engine.parse_input(text) == ('douyin', 'https://v.douyin.com/rgVESAi-rA8/', True)


@pytest.mark.parametrize('text', [
    '分享 https://v.douyin.com/a-b/ https://v.douyin.com/c-d/',
    '分享 https://v.douyin.com.attacker.invalid/a-b/',
    '分享 https://v.douyin.com/a-b/extra/',
])
def test_share_copy_still_rejects_multiple_or_invalid_links(text):
    with pytest.raises(engine.EngineError):
        engine.parse_input(text)


@pytest.mark.parametrize('text', ['', 'hello', 'https://example.com/video/123',
    'https://v.douyin.com/a/ https://vt.tiktok.com/b/',
    'https://www.tiktok.com/@user', 'https://www.tiktok.com/@user/live',
    'https://user:password@www.douyin.com/video/123',
    'https://www.douyin.com:444/video/123'])
def test_invalid_input(text):
    with pytest.raises(engine.EngineError):
        engine.parse_input(text)


@pytest.mark.parametrize('profile', [{'cookie':'bad'}, {'cookie':'a=b\r\nX:c'},
    {'proxy':'file:///x'}, {'proxy':'http://127.0.0.1'}, {'proxy':42},
    {'user_agent':''}, {'fingerprint':'windows-chrome'}])
def test_invalid_config(profile):
    with pytest.raises(engine.EngineError, match='CONFIG_INVALID'):
        engine.profile_config(profile)


def test_cookie_scope_and_profile_isolation():
    jar = CookieJar()
    ua, cookies, proxy = engine.profile_config({'cookie':'session=private; token=a=b',
                                               'user_agent':'my browser', 'proxy':None})
    engine.add_cookies(jar, cookies, 'tiktok')
    assert ua == 'my browser' and proxy is None
    for url, present in [('https://www.tiktok.com/', True), ('http://www.tiktok.com/',False),
                         ('https://www.douyin.com/',False),('https://v.tiktokcdn.com/',False)]:
        req = Request(url)
        jar.add_cookie_header(req)
        assert bool(req.get_header('Cookie')) is present
    assert all(c.discard for c in jar)


def test_format_contract():
    audio = dict(protocol='https',ext='mp4',vcodec='none',acodec='aac')
    video = dict(protocol='https',ext='mp4',vcodec='h264',acodec='none')
    combined = dict(protocol='https',ext='mp4',vcodec='h264',acodec='aac')
    assert list(engine.select_format({'formats':[combined, video, audio]})) == [combined]
    assert list(engine.select_format({'formats':[video]})) == [video]
    for formats in [[audio,video], [{**combined,'protocol':'m3u8_native'}],
                    [{**combined,'has_drm':True}], [audio]]:
        with pytest.raises(engine.EngineError,match='UNSUPPORTED_CONTENT'):
            list(engine.select_format({'formats':formats}))


@pytest.mark.parametrize('url', ['http://127.0.0.1/v','https://eviltiktokcdn.com/v',
    'https://tiktokcdn.com.evil.org/v','https://u:p@tiktokcdn.com/v',
    'https://tiktokcdn.com:444/v','file:///secret'])
def test_target_rejection(url):
    with pytest.raises(TargetError):
        validate_target(url, 'tiktok')


def test_domains_are_platform_specific():
    validate_target('https://v1.douyinvod.com/v','douyin')
    with pytest.raises(TargetError):
        validate_target('https://v1.douyinvod.com/v','tiktok')


def test_metadata_never_exposes_media_url():
    value = engine.metadata({'id':'123','webpage_url':'https://www.tiktok.com/@u/video/123',
        'url':'https://cdn.invalid/?secret=private','http_headers':{'Cookie':'private'},
        'formats':[{'url':'private'}], 'title':'title'},'tiktok')
    assert 'private' not in json.dumps(value)
    assert value['duration_ms'] is None


def test_metadata_rejects_playlist_and_identity():
    for info in [{'_type':'playlist'}, {'id':'123','is_live':True},
                 {'id':'123','webpage_url':'https://www.tiktok.com/@u/video/456'}]:
        with pytest.raises(engine.EngineError):
            engine.metadata(info,'tiktok')


def test_pinned_version_and_no_old_modules():
    import sys
    engine.check_version()
    assert not any(name.startswith(('dtk','wreq','httpx','pydantic','structlog')) for name in sys.modules)


def test_cover_prefers_highest_resolution_and_downloads_to_stage(tmp_path):
    import io
    body = b'\x89PNG\r\n\x1a\nfixture'
    info = {'thumbnails': [{'url': 'https://p.byteimg.com/small', 'width': 100, 'height': 100},
                           {'url': 'https://p.byteimg.com/large', 'width': 1080, 'height': 1920}]}
    class Ydl:
        def urlopen(self, url):
            assert url == 'https://p.byteimg.com/large'
            return io.BytesIO(body)
    assert engine.download_cover(Ydl(), info, tmp_path) == {'cover_filename': 'cover', 'cover_bytes': len(body)}
    assert (tmp_path / 'cover').read_bytes() == body
    assert not (tmp_path / 'cover.part').exists()


@pytest.mark.parametrize('body', [b'<html>not an image</html>', b'\xff\xd8\xff' + b'x' * (10 * 1024 * 1024)], ids=['html', 'oversized'])
def test_cover_rejects_invalid_or_oversized_content(tmp_path, body):
    import io
    class Ydl:
        def urlopen(self, url):
            return io.BytesIO(body)
    assert engine.download_cover(Ydl(), {'thumbnail': 'https://p.byteimg.com/x'}, tmp_path) == {'cover_error': 'COVER_UNAVAILABLE'}
    assert not list(tmp_path.iterdir())


def test_cover_failure_does_not_fail_video(tmp_path):
    class Ydl:
        def urlopen(self, url):
            raise engine.TargetError('blocked')
    assert engine.download_cover(Ydl(), {'thumbnail': 'https://invalid/x'}, tmp_path) == {'cover_error': 'COVER_UNAVAILABLE'}


def test_cover_prefers_original_when_upstream_omits_dimensions():
    assert engine.cover_url({'thumbnails': [
        {'id': 'origin_cover', 'url': 'https://p.byteimg.com/original'},
        {'id': 'cover', 'url': 'https://p.byteimg.com/preview'},
    ]}) == 'https://p.byteimg.com/original'
