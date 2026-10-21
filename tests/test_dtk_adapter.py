import hashlib
from types import SimpleNamespace

import pytest

import engine
import dtk_adapter
from yt_dlp import YoutubeDL


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
        title='fixture', duration_ms=1000, music=None,
        author=SimpleNamespace(unique_id='fixture', nickname='fixture'),
        media=SimpleNamespace(video=stream, streams=[], covers=[SimpleNamespace(url=legacy)]))
    expected = ('https://p.douyinpic.com/designed' if platform == 'douyin' and cover and
                'https://p.douyinpic.com/designed' in cover.get('url_list', []) else legacy)
    assert dtk_adapter.metadata(item)['cover_url'] == expected
    assert engine.cover_url(dtk_adapter.video_info(item)) == expected
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
        dtk_adapter.download_images(ydl, content(), tmp_path, lambda event_type, **value: events.append((event_type, value)))
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
    assert dtk_adapter.image_extension(b'\xff\xd8\xfforiginal') == 'jpeg'
    assert dtk_adapter.image_extension(b'\x89PNG\r\n\x1a\noriginal') == 'png'
    assert dtk_adapter.image_extension(b'GIF89aoriginal') == 'gif'
    assert dtk_adapter.image_extension(b'\x00\x00\x00\x18ftypavif') == 'avif'
    assert dtk_adapter.image_extension(b'\x00\x00\x00\x18ftypheic') == 'heic'
    with pytest.raises(ValueError):
        dtk_adapter.image_extension(b'<html>not an image</html>')


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
                dtk_adapter.download_images(ydl, item, tmp_path, emit)
            assert not list((tmp_path / 'album').iterdir())
        else:
            dtk_adapter.download_images(ydl, item, tmp_path, emit)
            file = events[-1][1]['files'][0]
            assert file['quality_source'] == 'high_definition' and file['width'] == 2160


def test_dtk_video_uses_same_guarded_single_file_downloader(monkeypatch, tmp_path, media_proxy):
    from conftest import BODY
    proxy, _, _, _ = media_proxy
    stream = SimpleNamespace(url='http://v.fixture.douyinvod.com/good.mp4', watermark=False,
                             width=1280, height=720, size_bytes=len(BODY), bitrate=None)
    item = SimpleNamespace(platform='douyin', content_id='123', kind='video', raw={},
        web_url='https://www.douyin.com/video/123', title='fixture', duration_ms=1000, music=None,
        author=SimpleNamespace(unique_id='fixture',nickname='fixture'),
        media=SimpleNamespace(video=stream,streams=[stream],covers=[]))
    monkeypatch.setattr(dtk_adapter,'extract',lambda *args:item)
    from yt_dlp import YoutubeDL
    original_process = YoutubeDL.process_info
    def process(self, info):
        assert info['http_headers']['Referer'] == 'https://www.douyin.com/'
        return original_process(self, info)
    monkeypatch.setattr(YoutubeDL, 'process_info', process)
    events=[]
    engine.operate('download_video',dict(input=item.web_url,resolver='dtk',
        profiles={'douyin':{'proxy':proxy}},expected={'platform':'douyin','video_id':'123','kind':'video'},
        staging_dir=str(tmp_path)),lambda event_type,**value:events.append((event_type,value)))
    assert (tmp_path/'video.mp4').read_bytes()==BODY
    assert events[-1][0]=='download_finished' and events[-1][1]['bytes']==len(BODY)
