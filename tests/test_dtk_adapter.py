import hashlib
import io
import json
from datetime import UTC, datetime
from types import SimpleNamespace

import pytest

import engine
import dtk_adapter
import media_album
from yt_dlp import YoutubeDL
from cookie_scope import scoped_jar


def test_tiktok_uses_session_device_id_and_refreshed_signing_cookie(monkeypatch):
    from urllib.parse import parse_qs, urlsplit
    ua = engine.DEFAULT_UA
    requests = []
    fresh_token = 'fresh-' + 'a' * 128
    with YoutubeDL({'quiet': True, 'http_headers': {'User-Agent': ua}}) as ydl:
        ydl.cookiejar = scoped_jar()
        engine.add_cookies(ydl.cookiejar, {'msToken': 'old-token', 'sid_tt': 'synthetic-login'}, 'tiktok', login_session=True)

        def open_request(request):
            requests.append(request)
            if request.url == 'https://www.tiktok.com/':
                engine.add_cookies(ydl.cookiejar, {'msToken': fresh_token, 'tt_chain_token': 'synthetic-chain'}, 'tiktok', login_session=True)
                state = {'__DEFAULT_SCOPE__': {'webapp.app-context': {'wid': '1234567890123456789'}}}
                return io.BytesIO(('<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__">' + json.dumps(state) + '</script>').encode())
            query = parse_qs(urlsplit(request.url).query)
            assert query['device_id'] == ['1234567890123456789']
            assert query['msToken'] == [fresh_token]
            assert query['itemId'] == ['123']
            assert 'X-Gnarly' in query and 'X-Dynosaur' in query
            assert ydl.cookiejar.get_cookie_header(request.url).find('msToken=' + fresh_token) >= 0
            assert not ydl.cookiejar.get_cookie_header('https://p.tiktokcdn.com/image.webp')
            return io.BytesIO(b'{"statusCode":0}')

        monkeypatch.setattr(ydl, 'urlopen', open_request)
        assert dtk_adapter.request_json(ydl, dtk_adapter.TIKTOK, 'tiktok.content_detail', ua,
            {'msToken': 'old-token'}, None, item_id='123') == {'statusCode': 0}
    assert len(requests) == 2 and all(request.headers['User-Agent'] == ua for request in requests)
    assert all('Cookie' not in request.headers for request in requests)


@pytest.mark.parametrize('page', [b'challenge page', b'<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__">{}</script>', b'x' * (4 * 1024 * 1024 + 1)], ids=['missing', 'empty', 'oversized'])
def test_tiktok_missing_session_device_id_does_not_send_random_identity(monkeypatch, page):
    with YoutubeDL({'quiet': True}) as ydl:
        requests = []
        def open_request(request):
            requests.append(request.url)
            return io.BytesIO(page)
        monkeypatch.setattr(ydl, 'urlopen', open_request)
        with pytest.raises(engine.EngineError, match='UPSTREAM_CHANGED'):
            dtk_adapter.request_json(ydl, dtk_adapter.TIKTOK, 'tiktok.content_detail', engine.DEFAULT_UA,
                None, None, item_id='123')
    assert requests == ['https://www.tiktok.com/']


def test_tiktok_withheld_shop_video_is_unsupported_not_schema_failure(monkeypatch):
    monkeypatch.setattr(dtk_adapter, 'request_json', lambda *args, **kwargs: {'itemInfo': {'itemStruct': {
        'id': '123', 'desc': 'shop', 'isECVideo': 1, 'author': {'id': 'author', 'uniqueId': 'shop', 'nickname': 'Shop'},
        'video': {'playAddr': '', 'downloadAddr': ''}, 'stats': {}}}})
    with pytest.raises(engine.EngineError, match='UNSUPPORTED_CONTENT'):
        dtk_adapter.extract(None, 'tiktok', '123', engine.DEFAULT_UA, None, None)


def test_upstream_media_access_referer_reaches_video_downloader():
    item = live_album()
    item = item.model_copy(update={'kind': 'video',
        'media': item.media.model_copy(update={'video': item.media.images[0].video})})
    assert dtk_adapter.video_info(item)['http_headers'] == {'Referer': 'https://www.douyin.com/'}


def tiktok_video():
    return dtk_adapter.TIKTOK.parse_content({'itemInfo': {'itemStruct': {
        'id': '123', 'desc': 'video', 'author': {'id': 'author', 'uniqueId': 'name', 'nickname': 'Name'},
        'video': {'id': 'video', 'playAddr': 'https://v16-webapp-prime.tiktok.com/video.mp4?tk=tt_chain_token',
                  'width': 720, 'height': 1280, 'duration': 1}, 'stats': {}}}}, fetched_at=datetime.now(UTC))


def test_tiktok_bound_media_only_sends_chain_cookie_to_tiktok_domains():
    from network import TargetGuard
    from urllib.request import Request
    item = tiktok_video()
    assert item.media.access.session_bound
    with YoutubeDL({'quiet': True}) as ydl:
        ydl.cookiejar = scoped_jar()
        engine.add_cookies(ydl.cookiejar, {'tt_chain_token': 'synthetic-chain', 'sid_tt': 'synthetic-login',
            'sessionid': 'synthetic-session'}, 'tiktok', login_session=True)
        assert ydl.cookiejar.get_cookie_header(item.media.video.url) is None
        info = dtk_adapter.video_info(item, ydl)
        assert info['http_headers'] == {'Referer': 'https://www.tiktok.com/'}
        for url in [item.media.video.url, 'https://other.tiktok.com/redirect-target.mp4']:
            request = Request(url, headers={'Cookie': 'sid_tt=must-be-stripped'})
            TargetGuard('tiktok').https_request(request)
            ydl.cookiejar.add_cookie_header(request)
            assert request.get_header('Cookie') == 'tt_chain_token=synthetic-chain'
        for url in ['https://p.tiktokcdn.com/image.webp', 'https://v.tiktokv.com/video.mp4',
                    'https://www.tiktok.com.evil.test/video.mp4', 'http://v16-webapp-prime.tiktok.com/video.mp4']:
            assert not ydl.cookiejar.get_cookie_header(url)
        assert 'sid_tt=synthetic-login' in ydl.cookiejar.get_cookie_header('https://www.tiktok.com/')


def test_tiktok_bound_media_requires_the_current_session_chain():
    with YoutubeDL({'quiet': True}) as ydl:
        ydl.cookiejar = scoped_jar()
        with pytest.raises(engine.EngineError, match='ACCESS_DENIED'):
            dtk_adapter.video_info(tiktok_video(), ydl)


def content(originals=None):
    image = SimpleNamespace(url='http://v.fixture.douyinvod.com/image.webp', width=2160, height=3240)
    return SimpleNamespace(platform='douyin', content_id='123', raw={'original_images': originals},
                           media=SimpleNamespace(images=[image]))


def test_original_priority_and_hd_when_missing():
    original = {'url_list': ['https://p.douyinpic.com/original.png'], 'width': 4320, 'height': 6480}
    choice = dtk_adapter.image_choices(content([original]))[0]
    assert choice['url'] == original['url_list'][0]
    assert choice['quality_source'] == 'original' and choice['width'] == 4320
    hd = dtk_adapter.image_choices(content())[0]
    assert hd['url'].endswith('image.webp') and hd['quality_source'] == 'high_definition'


@pytest.mark.parametrize('platform', ['douyin', 'tiktok'])
@pytest.mark.parametrize('cover', [None, {'url_list': []},
    {'url_list': ['http://p.douyinpic.com/insecure']},
    {'url_list': [None, 'https://p.douyinpic.com/designed']}])
def test_video_cover_priority_is_douyin_only_and_falls_back(platform, cover):
    legacy = 'https://p.douyinpic.com/legacy'
    stream = SimpleNamespace(url='https://v.fixture.douyinvod.com/video.mp4', watermark=False,
                             width=1920, height=1080, size_bytes=10000, bitrate=None)
    item = SimpleNamespace(platform=platform, content_id='123', kind='video',
        raw={'video': {'cover_original_scale': cover}}, web_url='https://www.douyin.com/video/123',
        title='fixture', duration_ms=1000, music=None, created_at=None,
        author=SimpleNamespace(uid='author-id', unique_id='fixture', nickname='fixture', signature=None, avatar=None),
        media=SimpleNamespace(video=stream, streams=[], covers=[SimpleNamespace(url=legacy)]))
    expected = ('https://p.douyinpic.com/designed' if platform == 'douyin' and cover and
                'https://p.douyinpic.com/designed' in cover.get('url_list', []) else legacy)
    assert dtk_adapter.metadata(item)['cover_url'] == expected
    assert engine.cover_url(dtk_adapter.video_info(item)) == expected
    assert dtk_adapter.video_info(item)['_yingqu_douyin_covers'] == ([expected] if expected.endswith('/designed') else [])
    item.media.covers = []
    expected = expected if expected != legacy else None
    assert dtk_adapter.metadata(item)['cover_url'] == expected
    assert engine.cover_url(dtk_adapter.video_info(item)) == expected


@pytest.mark.parametrize('original', [{'wrong': 'shape'}, {'url_list': []}, {'url_list': ['file:///private']}])
def test_present_invalid_original_cannot_silently_downgrade(original):
    with pytest.raises(engine.EngineError, match='UPSTREAM_CHANGED'):
        dtk_adapter.image_choices(content([original]))


def test_real_http_downloader_preserves_image_bytes_and_progress(tmp_path, media_proxy):
    import functools
    from network import GuardedRH
    proxy, requests, _, _ = media_proxy
    events = []
    with YoutubeDL(dict(proxy=proxy, quiet=True, noprogress=True, retries=0, continuedl=False,
                        logger=engine.SafeLogger(), http_headers={'User-Agent': engine.DEFAULT_UA})) as ydl:
        ydl._request_director = ydl.build_request_director([functools.partial(GuardedRH, platform='douyin')])
        media_album.download_images(ydl, dtk_adapter.image_choices(content()), 'douyin', '123', 'dtk', tmp_path, lambda event_type, **value: events.append((event_type, value)))
    body = (tmp_path / 'album/001.webp').read_bytes()
    assert body == b'RIFF' + b'\x10\x00\x00\x00WEBPfixture'
    result = events[-1][1]
    assert result['files'][0]['sha256'] == hashlib.sha256(body).hexdigest()
    assert result['files'][0]['quality_source'] == 'high_definition'
    assert result['bytes'] == len(body)
    progress = [value for kind, value in events if kind == 'progress']
    assert all(v['total_bytes'] is None for v in progress[:-1])
    assert progress[-1]['total_bytes'] == len(body) and progress[-1]['completed_items'] == 1
    assert all('url' not in str(v) for _, v in events)
    assert not list(tmp_path.rglob('*.part'))
    assert not any('Cookie' in headers for _, headers in requests)


def test_image_format_is_detected_without_transcoding():
    assert media_album.image_extension(b'\xff\xd8\xfforiginal') == 'jpeg'
    assert media_album.image_extension(b'\x89PNG\r\n\x1a\noriginal') == 'png'
    assert media_album.image_extension(b'GIF89aoriginal') == 'gif'
    assert media_album.image_extension(b'\x00\x00\x00\x18ftypavif') == 'avif'
    assert media_album.image_extension(b'\x00\x00\x00\x18ftypheic') == 'heic'
    with pytest.raises(ValueError):
        media_album.image_extension(b'<html>not an image</html>')


@pytest.mark.parametrize('status', [404, 403])
def test_original_unavailable_downgrades_but_auth_failure_does_not(status, tmp_path, media_proxy):
    import functools
    import io
    from network import GuardedRH
    from yt_dlp.networking import Response
    from yt_dlp.networking.exceptions import HTTPError
    proxy, _, _, _ = media_proxy
    original = 'https://p.douyinpic.com/original.png'
    events = []
    with YoutubeDL(dict(proxy=proxy, quiet=True, noprogress=True, retries=0, continuedl=False,
                        logger=engine.SafeLogger(), http_headers={'User-Agent': engine.DEFAULT_UA})) as ydl:
        ydl._request_director = ydl.build_request_director([functools.partial(GuardedRH, platform='douyin')])
        urlopen = ydl.urlopen
        def unavailable(url):
            if url == original:
                raise HTTPError(Response(io.BytesIO(), original, {}, status=status))
            return urlopen(url)
        ydl.urlopen = unavailable
        item = content([{'url_list': [original], 'width': 4320, 'height': 6480}])
        emit = lambda event_type, **value: events.append((event_type, value))
        if status == 403:
            with pytest.raises(HTTPError):
                media_album.download_images(ydl, dtk_adapter.image_choices(item), 'douyin', '123', 'dtk', tmp_path, emit)
            assert not list((tmp_path / 'album').iterdir())
        else:
            media_album.download_images(ydl, dtk_adapter.image_choices(item), 'douyin', '123', 'dtk', tmp_path, emit)
            file = events[-1][1]['files'][0]
            assert file['quality_source'] == 'high_definition' and file['width'] == 2160


def test_dtk_video_uses_same_guarded_single_file_downloader(monkeypatch, tmp_path, media_proxy):
    from conftest import BODY
    proxy, _, _, _ = media_proxy
    stream = SimpleNamespace(url='http://v.fixture.douyinvod.com/good.mp4', watermark=False,
                             width=1280, height=720, size_bytes=len(BODY), bitrate=None)
    item = SimpleNamespace(platform='douyin', content_id='123', kind='video', raw={},
        web_url='https://www.douyin.com/video/123', title='fixture', duration_ms=1000, music=None,
        created_at=datetime.fromtimestamp(1700000000, UTC),
        author=SimpleNamespace(uid='author-id',unique_id='fixture',nickname='fixture',signature='DTK 简介',avatar=None),
        media=SimpleNamespace(video=stream,streams=[stream],covers=[]))
    monkeypatch.setattr(dtk_adapter,'extract',lambda *args:item)
    from yt_dlp import YoutubeDL
    original_process = YoutubeDL.process_info
    def process(self, info):
        assert info['http_headers']['Referer'] == 'https://www.douyin.com/'
        return original_process(self, info)
    monkeypatch.setattr(YoutubeDL, 'process_info', process)
    events=[]
    engine.operate('resolve_video', dict(input=item.web_url, resolver='dtk', profiles={'douyin':{'proxy':proxy}}),
                   lambda event_type, **value: events.append((event_type, value)))
    assert events[-1][1]['result']['published_at'] == 1700000000
    engine.operate('download_video',dict(input=item.web_url,resolver='dtk',
        profiles={'douyin':{'proxy':proxy}},expected={'platform':'douyin','video_id':'123','kind':'video'},
        staging_dir=str(tmp_path)),lambda event_type,**value:events.append((event_type,value)))
    assert (tmp_path/'video.mp4').read_bytes()==BODY
    assert events[-1][0]=='download_finished' and events[-1][1]['bytes']==len(BODY)
    assert events[-1][1]['author_profile'] == {'id': 'author-id', 'bio': 'DTK 简介'}


def live_album(avatar_url=None):
    # Real vendored parser, with a mixed live/still album. Post-level video is
    # intentionally unrelated; only the per-slide clip may be downloaded.
    return dtk_adapter.DOUYIN.parse_content({'aweme_detail': {
        'aweme_id': '123', 'desc': 'live fixture', 'status': {}, 'statistics': {}, 'create_time': 1700000000,
        'author': {'sec_uid': 'fixture-author', 'nickname': 'fixture', 'signature': 'DTK 简介',
                   'follower_count': 0, 'total_favorited': 321, 'city': '上海',
                   'avatar_thumb': {'url_list': [avatar_url]} if avatar_url else None},
        'video': {'play_addr': {'url_list': ['http://v.fixture.douyinvod.com/music.mp4']}},
        'images': [
            {'url_list': ['http://v.fixture.douyinvod.com/image.webp'], 'width': 1080, 'height': 1920,
             'live_photo_type': 1, 'video': {'width': 720, 'height': 1280,
                 'play_addr': {'url_list': ['http://v.fixture.douyinvod.com/live.mp4']}}},
            {'url_list': ['http://v.fixture.douyinvod.com/image.webp'], 'width': 1080, 'height': 1920}]}},
        fetched_at=datetime.now(UTC))


@pytest.mark.parametrize('kind', ['video', 'image_album'])
@pytest.mark.parametrize(('fetch', 'valid'), [(True, True), (True, False), (False, True)])
def test_batch_download_stages_optional_avatar_without_adding_to_media(
        kind, fetch, valid, monkeypatch, tmp_path, media_proxy):
    import io
    from conftest import BODY
    from yt_dlp.networking import Response

    proxy, _, _, _ = media_proxy
    avatar_url = 'https://p.douyinpic.com/fixture-avatar?token=fixture'
    body = b'\x89PNG\r\n\x1a\nfixture' if valid else b'<html>unavailable</html>'
    if kind == 'image_album':
        item = live_album(avatar_url)
    else:
        stream = SimpleNamespace(url='http://v.fixture.douyinvod.com/good.mp4', watermark=False,
                                 width=1280, height=720, size_bytes=len(BODY), bitrate=None)
        item = SimpleNamespace(platform='douyin', content_id='123', kind=kind, raw={},
            web_url='https://www.douyin.com/video/123', title='fixture', duration_ms=1000, music=None, created_at=None,
            author=SimpleNamespace(uid='fixture-author', sec_uid='fixture-author',
                                   unique_id='fixture', nickname='fixture', signature='DTK 简介',
                                   avatar=SimpleNamespace(url=avatar_url)),
            media=SimpleNamespace(video=stream, streams=[], covers=[]))
    monkeypatch.setattr(dtk_adapter, 'extract', lambda *_args: item)
    urlopen = YoutubeDL.urlopen
    avatar_requests = []

    def open_url(self, request):
        if getattr(request, 'url', None) == avatar_url:
            avatar_requests.append(request.url)
            return Response(io.BytesIO(body), avatar_url, {})
        return urlopen(self, request)

    monkeypatch.setattr(YoutubeDL, 'urlopen', open_url)
    events = []
    engine.operate('download_video', dict(input=item.web_url, resolver='dtk',
        profiles={'douyin': {'proxy': proxy, 'cookie': 'sessionid=fixture'}},
        expected={'platform': 'douyin', 'video_id': '123', 'kind': kind, 'author_id': 'fixture-author'},
        fetch_author_avatar=fetch, staging_dir=str(tmp_path)),
        lambda event, **value: events.append((event, value)))
    event, result = events[-1]
    assert event == 'download_finished' and avatar_url not in str(result)
    assert result['metadata']['author_profile']['bio'] == 'DTK 简介'
    assert avatar_requests == ([avatar_url] if fetch else [])
    if fetch and valid:
        assert result['author_avatar_filename'] == 'author-avatar'
        assert result['author_avatar_bytes'] == len(body)
        assert (tmp_path / 'author-avatar').read_bytes() == body
    else:
        assert 'author_avatar_filename' not in result
        assert not (tmp_path / 'author-avatar').exists()
    media = (tmp_path / 'album').iterdir() if kind == 'image_album' else [tmp_path / 'video.mp4']
    assert result['bytes'] == sum(path.stat().st_size for path in media)


def test_live_album_preserves_stills_and_original_clips(monkeypatch, tmp_path, media_proxy, media_bodies):
    import copy
    from conftest import BODY
    proxy, requests, _, _ = media_proxy
    media_bodies['live'] = BODY
    item = live_album()
    original = copy.deepcopy(item)
    monkeypatch.setattr(dtk_adapter, 'extract', lambda *args: item)
    preview = tmp_path / 'preview'
    preview.mkdir()
    params = dict(input=item.web_url, resolver='dtk', profiles={'douyin': {'proxy': proxy}})
    events = []
    emit = lambda event, **value: events.append((event, value))
    engine.operate('resolve_video', {**params, 'preview_dir': str(preview)}, emit)
    metadata = events[-1][1]['result']
    assert len(metadata['images']) == 2
    assert metadata['author_profile']['bio'] == 'DTK 简介'
    assert metadata['author_profile']['follower_count'] == 0
    assert metadata['published_at'] == 1700000000
    assert len(list(preview.glob('preview-*.webp'))) == 2
    stage = tmp_path / 'stage'
    stage.mkdir()
    events.clear()
    engine.operate('download_video', {**params, 'expected': {
        'platform': 'douyin', 'video_id': '123', 'kind': 'image_album', 'image_count': 2},
        'staging_dir': str(stage)}, emit)
    assert events[-1][0] == 'download_finished'
    result = events[-1][1]
    assert result['author_profile'] == {'id': 'fixture-author', 'bio': 'DTK 简介',
                                        'follower_count': 0, 'likes_count': 321, 'region': '上海'}
    assert {p.name for p in (stage / 'album').iterdir()} == {'001.webp', '001.mp4', '002.webp'}
    assert (stage / 'album/001.mp4').read_bytes() == BODY
    assert result['files'][0]['video'] == dict(filename='001.mp4', bytes=len(BODY), sha256=hashlib.sha256(BODY).hexdigest())
    assert 'video' not in result['files'][1]
    assert result['bytes'] == sum(p.stat().st_size for p in (stage / 'album').iterdir())
    progress = [value for kind, value in events if kind == 'progress']
    assert [p['downloaded_bytes'] for p in progress] == sorted(p['downloaded_bytes'] for p in progress)
    assert progress[-1]['completed_items'] == 2
    assert not any('/music.mp4' in url for url, _ in requests)
    assert 'url' not in str(result) and item == original


@pytest.mark.parametrize('mode', ['html', 'status404', 'truncated', 'avif'])
def test_live_clip_failure_never_finishes_album(mode, tmp_path, media_proxy, media_bodies):
    import functools
    from network import GuardedRH
    from yt_dlp.networking.exceptions import HTTPError
    from yt_dlp.utils import DownloadError
    proxy, _, _, _ = media_proxy
    media_bodies['avif'] = b'\x00\x00\x00\x18ftypavif' + b'fixture'
    item = live_album()
    clip = item.media.images[0].video.model_copy(update={'url': f'http://v.fixture.douyinvod.com/{mode}.mp4'})
    item.media.images[0] = item.media.images[0].model_copy(update={'video': clip})
    events = []
    with YoutubeDL(dict(proxy=proxy, quiet=True, noprogress=True, retries=0, continuedl=False,
                        logger=engine.SafeLogger(), http_headers={'User-Agent': engine.DEFAULT_UA})) as ydl:
        ydl._request_director = ydl.build_request_director([functools.partial(GuardedRH, platform='douyin')])
        with pytest.raises((engine.EngineError, HTTPError, DownloadError)):
            media_album.download_images(ydl, dtk_adapter.image_choices(item), 'douyin', '123', 'dtk', tmp_path, lambda kind, **value: events.append((kind, value)))
    assert not any(kind == 'download_finished' for kind, _ in events)
    assert not any(value.get('completed_items') for _, value in events)
