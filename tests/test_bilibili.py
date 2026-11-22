import json
import pytest
from yt_dlp import YoutubeDL
from yt_dlp.networking import Request

import engine
from cookie_scope import scoped_jar
from network import TargetError, validate_target
from video_formats import public_formats
import bilibili

URL = 'https://www.bilibili.com/video/BV13XH56SEYt/'


@pytest.mark.parametrize('url,page', [(URL, 1), (URL + '?p=2&spm_id_from=ignored', 2),
    ('https://m.bilibili.com/video/BV13XH56SEYt?p=3', 3)])
def test_bilibili_identity_preserves_part_and_discards_tracking(url, page):
    platform, canonical, short = engine.parse_input(url)
    assert platform == 'bilibili' and not short
    assert canonical == URL + f'?p={page}'
    identity = engine.input_identity(platform, canonical, 'auto', url)
    assert identity['video_id'] == f'BV13XH56SEYt_p{page}'
    assert identity['resolver'] == 'yt-dlp'
    with pytest.raises(engine.EngineError):
        engine.input_identity(platform, canonical, 'dtk', url)


@pytest.mark.parametrize('url', [URL + '?p=0', URL + '?p=1&p=2', URL + '?p=../2',
    'https://www.bilibili.com/bangumi/play/ss123', 'https://www.bilibili.com/video/BVshort',
    'https://www.bilibili.com.evil.test/video/BV13XH56SEYt/',
    'https://user@www.bilibili.com/video/BV13XH56SEYt/'])
def test_bilibili_rejects_non_single_video_or_invalid_part(url):
    with pytest.raises(engine.EngineError): engine.parse_input(url)


def test_bilibili_short_link_is_resolved_inside_guarded_transport():
    assert engine.parse_input('分享 https://b23.tv/AbC123?share_source=copy') == (
        'bilibili', 'https://b23.tv/AbC123', True)


@pytest.mark.parametrize('login', [False, True])
def test_bilibili_cookie_reaches_official_api_but_not_media_or_other_platforms(login):
    jar = scoped_jar()
    engine.add_cookies(jar, {'SESSDATA': 'fixture-session'}, 'bilibili', login_session=login)
    import urllib.request
    for host, expected in [('www.bilibili.com', True), ('api.bilibili.com', True),
                           ('upos-sz-mirrorcos.bilivideo.com', False), ('www.youtube.com', False)]:
        request = urllib.request.Request('https://' + host + '/')
        jar.add_cookie_header(request)
        assert bool(request.get_header('Cookie')) == expected


def test_bilibili_network_boundaries_and_dash_selection(monkeypatch):
    monkeypatch.setattr(bilibili, 'attach_author', lambda *_: None)
    for host in ['www.bilibili.com', 'api.bilibili.com', 'i0.hdslb.com', 'v.bilivideo.com']:
        validate_target('https://' + host + '/fixture', 'bilibili')
    for host in ['bilivideo.com.evil.test', 'www.youtube.com', '127.0.0.1']:
        with pytest.raises(TargetError): validate_target('https://' + host + '/', 'bilibili')
    info = dict(id='BV13XH56SEYt_p1', webpage_url=URL + '?p=1', title='测试视频',
        thumbnail='http://i0.hdslb.com/bfs/archive/fixture.jpg',
        uploader='测试作者', uploader_id='123', duration=74, timestamp=1700000000,
        formats=[dict(format_id='video-1080', ext='mp4', vcodec='avc1', acodec='none',
                      width=1920, height=1080, protocol='https', url='https://v.bilivideo.com/video'),
                 dict(format_id='audio', ext='m4a', vcodec='none', acodec='mp4a',
                      protocol='https', url='https://v.bilivideo.com/audio')])
    observed = {}
    def extract(self, url, download=False):
        observed.update(self.params)
        return self.process_ie_result(dict(info), download=download)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: pytest.fail('Bilibili needs no JS runtime'))
    events = []
    engine.operate('resolve_video', {'input': URL}, lambda kind, **data: events.append((kind, data)))
    result = events[-1][1]['result']
    assert result['platform'] == 'bilibili' and result['video_id'] == info['id']
    assert result['cover_url'] == 'https://i0.hdslb.com/bfs/archive/fixture.jpg'
    assert result['formats'][0]['requires_merge'] and result['formats'][0]['height'] == 1080
    assert observed['noplaylist'] and observed['allowed_extractors'] == ['BiliBili$']
    assert 'bilivideo.com' not in json.dumps(result)


def test_bilibili_only_upgrades_official_image_cdn_urls_to_https():
    info = {'thumbnail': 'http://i0.hdslb.com/cover.jpg', 'thumbnails': [
        {'url':'http://i0.hdslb.com.evil.test/cover.jpg'},
        {'url':'http://user:password@i0.hdslb.com/cover.jpg'},
        {'url':'http://i0.hdslb.com:8080/cover.jpg'}]}
    original = [dict(t) for t in info['thumbnails']]
    bilibili.normalize_thumbnails(info)
    assert info['thumbnail'] == 'https://i0.hdslb.com/cover.jpg'
    assert info['thumbnails'] == original


def test_bilibili_official_mcdn_https_port_is_scoped_to_media():
    url = 'https://xy211x154x243x196xy.mcdn.bilivideo.cn:8082/media.m4s'
    validate_target(url, 'bilibili')
    for rejected in [url.replace('https:', 'http:'), url.replace(':8082', ':8083'),
                     url.replace('.mcdn.bilivideo.cn', '.mcdn.bilivideo.cn.evil.test'),
                     url.replace('.mcdn.bilivideo.cn', '.bilivideo.cn'),
                     url.replace('xy211x154x243x196xy.mcdn.bilivideo.cn', 'api.bilibili.com'),
                     url.replace('https://', 'https://user:password@')]:
        with pytest.raises(TargetError): validate_target(rejected, 'bilibili')
    for platform in ['youtube', 'douyin', 'tiktok', 'xiaohongshu']:
        with pytest.raises(TargetError): validate_target(url, platform)
    jar = scoped_jar()
    engine.add_cookies(jar, {'SESSDATA': 'fixture-session'}, 'bilibili')
    import urllib.request
    request = urllib.request.Request(url)
    jar.add_cookie_header(request)
    assert request.get_header('Cookie') is None


def test_bilibili_author_card_supplies_bio_and_avatar_without_persisting_url():
    import io
    import author_profile
    class Client:
        def urlopen(self, request):
            assert request.url == 'https://api.bilibili.com/x/web-interface/card?mid=123'
            assert request.extensions['timeout'] == 5
            return io.BytesIO(json.dumps({'code': 0, 'data': {'card': {
                'mid': '123', 'name': '作者', 'face': 'http://i0.hdslb.com/avatar.jpg',
                'sign': ' 简介第一行\n第二行 '}}}).encode())
    info = {'uploader_id': '123', 'uploader': '原昵称'}
    bilibili.attach_author(Client(), info)
    profile = author_profile.metadata(info, 'bilibili')
    assert profile['id'] == '123' and profile['nickname'] == '作者'
    assert profile['bio'] == '简介第一行\n第二行'
    assert author_profile.avatar_url(None, info, 'bilibili') == 'https://i0.hdslb.com/avatar.jpg'
    assert 'https://' not in json.dumps(profile)
    assert author_profile.downloaded_profile({'author_profile': profile}) == {
        'id': '123', 'bio': '简介第一行\n第二行'}
    assert author_profile.downloaded_profile({'platform': 'bilibili', 'author_profile': {'id': '123'}}) == {'id': '123'}


@pytest.mark.parametrize('payload', [b'<html>blocked</html>', b'x' * (1024 * 1024 + 1),
    b'{"code":-403}', b'{"code":0,"data":{"card":{"mid":456,"sign":"wrong"}}}',
    b'{"code":0,"data":{"card":[]}}'], ids=['html', 'oversized', 'denied', 'wrong-author', 'bad-card'])
def test_bilibili_optional_author_failures_keep_video_metadata(payload):
    import io
    class Client:
        def urlopen(self, request): return io.BytesIO(payload)
    info = {'uploader_id': '123', 'uploader': '原昵称'}
    bilibili.attach_author(Client(), info)
    assert info == {'uploader_id': '123', 'uploader': '原昵称'}


def test_bilibili_author_request_failure_is_optional_and_invalid_ids_are_not_queried():
    from yt_dlp.networking.exceptions import TransportError
    class Client:
        def urlopen(self, request): raise TransportError('fixture unavailable')
    bilibili.attach_author(Client(), {'uploader_id': '123'})
    for mid in [None, '../123', '123?other=1', True]:
        bilibili.attach_author(None, {'uploader_id': mid})
