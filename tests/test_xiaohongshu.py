import hashlib
import io
import json
from urllib.parse import urlsplit

import pytest
from PIL import Image
from yt_dlp import YoutubeDL
from yt_dlp.networking.common import Response

import engine
import media_album
import xiaohongshu
from video_formats import format_key


NOTE_ID = 'a0123456789abcdef0123456'
NOTE = f'https://www.xiaohongshu.com/discovery/item/{NOTE_ID}'
SHORT = 'https://xhslink.cn/o/fixture'
AVATAR = 'https://sns-avatar-qc.xhscdn.com/fixture!avatar'
CLIP = 'https://sns-video-qc.xhscdn.com/live.mp4'
MP4 = b'\0\0\0\x18ftypisomlive-photo-fixture'
with io.BytesIO() as image_bytes:
    Image.new('RGB', (80, 60)).save(image_bytes, format='JPEG')
    JPEG = image_bytes.getvalue()


def note_page(note, mobile):
    state = ({'noteData': {'data': {'noteData': note}}} if mobile else
             {'note': {'noteDetailMap': {NOTE_ID: {'note': note}}}})
    return ('<script>window.__INITIAL_STATE__=' + json.dumps(state) + '</script>').encode()


def response(url, body):
    return Response(io.BytesIO(body), url, {'Content-Length': str(len(body))}, status=200)


def test_share_normalization_keeps_only_access_parameters():
    assert engine.parse_input(f'分享 {NOTE}/?source=webshare&xsec_token=fixture=&xsec_source=pc_share。') == (
        'xiaohongshu', NOTE + '?xsec_token=fixture%3D&xsec_source=pc_share', False)
    assert engine.parse_input(f'App 分享 {SHORT}/。') == ('xiaohongshu', SHORT, True)
    for url in ('https://xhslink.cn/o/fixture/extra', f'{NOTE}0', f'{SHORT} https://example.test/'):
        with pytest.raises(engine.EngineError):
            engine.parse_input(url)


@pytest.mark.parametrize('mobile,kind,live_photo', [
    (False, 'normal', False), (True, 'normal', False), (True, 'video', False),
    (False, 'normal', True), (True, 'normal', True),
])
def test_album_uses_cdn_assets_for_preview_and_download(monkeypatch, tmp_path, mobile, kind, live_photo):
    page_url = NOTE + ('?xsec_source=app_share' if mobile else '?xsec_source=pc_share')
    images = [{'fileId': 'oss-sg/notes_pre_post/fixture'}] if mobile else [{
        'urlDefault': 'http://sns-webpic-qc.xhscdn.com/202611071755/' + 'a' * 32 + '/fixture!preview'}]
    if kind == 'video':
        images.append({'fileId': 'spectrum/another-slide'})
    if live_photo:
        clean_stream = {'masterUrl': CLIP.replace('https://', 'http://')}
        if mobile:
            clean_stream['streamDesc'] = 'WEB_LIVEPHOTO_19'
        images[0].update(livePhoto=True, stream={'h264': [
            {'streamDesc': 'MINI_APP_259', 'masterUrl': 'https://sns-video-qc.xhscdn.com/mini.mp4'},
            {'streamDesc': 'WEB_LIVEPHOTO_19', 'masterUrl': 'https://sns-video-qc.xhscdn.com/watermark.mp4', 'watermark': True},
            clean_stream,
        ]})
        images.append({'fileId': 'spectrum/still-slide', 'livePhoto': False})
    note = dict(noteId=NOTE_ID, type=kind, imageList=images, video={'consumer': {'originVideoKey': 'fixture'}},
                user={'userId': 'fixture-author', 'nickName' if mobile else 'nickname': '作者', 'avatar': AVATAR})
    requests = []

    def open_url(self, request):
        url = request if isinstance(request, str) else request.url
        requests.append(url)
        if url in (SHORT, page_url):
            return response(page_url, note_page(note, mobile))
        if url == CLIP:
            assert request.headers['Referer'] == 'https://www.xiaohongshu.com/'
            return response(url, MP4)
        assert url == AVATAR or urlsplit(url).hostname == 'sns-img-qc.xhscdn.com'
        return response(url, JPEG)

    monkeypatch.setattr(YoutubeDL, 'urlopen', open_url)
    monkeypatch.setattr(engine, 'load_default_cookies', lambda *_: pytest.fail('successful anonymous request read browser cookies'))
    preview = tmp_path / 'preview'
    stage = tmp_path / 'stage'
    preview.mkdir()
    stage.mkdir()
    events = []
    def emit(event, **value):
        events.append((event, value))
    input_url = SHORT if mobile else page_url
    engine.operate('identify_input', dict(input=input_url), emit)
    assert events[-1][1]['result']['kind'] == 'image_album'
    engine.operate('resolve_video', dict(input=input_url, preview_dir=str(preview)), emit)
    result = events[-1][1]['result']
    assert (result['resolver'], result['kind'], result['canonical_url']) == ('yt-dlp', 'image_album', page_url)
    assert len(result['images']) == len(images) and 'formats' not in result
    assert all((item['width'], item['height']) == (80, 60) for item in result['images'])
    assert (preview / result['images'][0]['preview_filename']).read_bytes() == JPEG
    assert result['author_profile'] == dict(id='fixture-author', nickname='作者', handle=None, kind='user')
    assert (preview / result['author_avatar_filename']).read_bytes() == JPEG
    assert result['author_avatar_bytes'] == len(JPEG) and AVATAR not in json.dumps(result)
    assert CLIP not in json.dumps(result) and CLIP not in requests
    engine.operate('download_video', dict(input=result['canonical_url'], resolver='yt-dlp', staging_dir=str(stage),
        expected=dict(platform='xiaohongshu', video_id=NOTE_ID, kind='image_album', image_count=len(images))), emit)
    assert events[-1][0] == 'download_finished'
    assert events[-1][1]['resolver'] == 'yt-dlp'
    assert (stage / 'album' / '001.jpeg').read_bytes() == JPEG
    files = events[-1][1]['files']
    assert len(list((stage / 'album').iterdir())) == len(images) + live_photo
    assert all((item['width'], item['height']) == (80, 60) for item in files)
    assert all('video' not in item for item in files[1:])
    assert events[-1][1]['bytes'] == len(JPEG) * len(images) + len(MP4) * live_photo
    if live_photo:
        assert files[0]['video'] == dict(filename='001.mp4', bytes=len(MP4), sha256=hashlib.sha256(MP4).hexdigest())
        assert (stage / 'album' / '001.mp4').read_bytes() == MP4
    else:
        assert 'video' not in files[0]
    assert page_url in requests
    assert requests.count(AVATAR) == 1


def test_marked_live_photo_cannot_silently_fall_back_to_a_still():
    note = {'imageList': [{'fileId': 'fixture', 'livePhoto': True}]}
    with pytest.raises(engine.EngineError, match='UNSUPPORTED_CONTENT') as error:
        xiaohongshu.image_choices(note, media_album.MAX_IMAGES)
    assert error.value.stage == 'live_photo'


@pytest.mark.parametrize('manual', [False, True])
def test_video_cookie_fallback_and_watermarked_stream_exclusion(monkeypatch, tmp_path, manual):
    page_url = NOTE + '?xsec_source=pc_share'
    stream = dict(streamType=258, streamDesc='X264_MP4', videoCodec='h264', audioCodec='aac',
                  width=720, height=1280, size=123, masterUrl='http://sns-video-v6.xhscdn.com/fixture.mp4')
    mini = dict(stream, streamType=259, streamDesc='MINI_APP_259', width=1080, height=1920)
    note = dict(noteId=NOTE_ID, title='fixture', type='video', imageList=[{'fileId': 'cover'}],
                video={'media': {'stream': {'h264': [mini, stream]}}, 'consumer': {'originVideoKey': 'fixture'}},
                user={'userId': 'fixture-author', 'nickname': '作者', 'avatar': AVATAR})
    requests = []
    fallback = []

    def open_url(self, request):
        url = request if isinstance(request, str) else request.url
        if url == AVATAR or urlsplit(url).hostname == 'sns-img-qc.xhscdn.com':
            return response(url, JPEG)
        assert url == page_url
        scoped = [(c.domain, c.name, c.value) for c in self.cookiejar]
        requests.append(scoped)
        if not scoped:
            return response('https://www.xiaohongshu.com/login', b'')
        return response(page_url, note_page(note, False))

    def browser(jar, platform, _logger):
        fallback.append(platform)
        engine.add_cookies(jar, {'web_session': 'browser-fixture'}, platform)

    monkeypatch.setattr(YoutubeDL, 'urlopen', open_url)
    monkeypatch.setattr(engine, 'load_default_cookies', browser)
    params = {'input': page_url, 'preview_dir': str(tmp_path),
              'profiles': {'xiaohongshu': {'cookie': 'web_session=manual-fixture'} if manual else {}}}
    resolved = []
    engine.operate('resolve_video', params, lambda _, **value: resolved.append(value['result']))
    assert fallback == ([] if manual else ['xiaohongshu'])
    scoped = [('.xiaohongshu.com', 'web_session', 'manual-fixture' if manual else 'browser-fixture')]
    assert requests == ([scoped] if manual else [[], scoped])
    assert [f['width'] for f in resolved[0]['formats']] == [720]
    assert resolved[0]['author_profile']['id'] == 'fixture-author'
    assert (tmp_path / resolved[0]['author_avatar_filename']).read_bytes() == JPEG
    assert resolved[0]['author_avatar_bytes'] == len(JPEG)
    assert 'xhscdn' not in json.dumps(resolved)


def test_mobile_page_rejects_a_different_note(monkeypatch):
    monkeypatch.setattr(YoutubeDL, 'urlopen', lambda *_: response(NOTE, note_page(
        dict(noteId='b' * 24, type='normal', imageList=[]), True)))
    with YoutubeDL({'quiet': True}) as ydl, pytest.raises(engine.EngineError, match='UPSTREAM_CHANGED'):
        xiaohongshu.fetch_note(ydl, SHORT, True)


def test_original_video_is_only_used_when_clean_web_formats_are_missing(monkeypatch):
    url = 'https://sns-video-bd.xhscdn.com/spectrum/original'
    mini = dict(streamType=259, streamDesc='MINI_APP_259', masterUrl='https://sns-video-v6.xhscdn.com/fixture.mp4')
    note = dict(noteId=NOTE_ID, type='video', video={
        'consumer': {'originVideoKey': 'spectrum/original'}, 'media': {'stream': {'h264': [mini]}},
    })
    def open_url(_, requested):
        assert requested == url
        return response(url, b'fixture')
    monkeypatch.setattr(YoutubeDL, 'urlopen', open_url)
    with YoutubeDL({'quiet': True}) as ydl:
        original = dict(url=url, ext='mp4', format_id='direct', filesize=7)
        assert xiaohongshu.video_info(ydl, note, NOTE)['formats'] == [original]
        note['video']['media']['stream']['h264'].append(dict(mini, streamType=258, streamDesc='X264_MP4',
            width=720, height=1280, videoCodec='h264', audioCodec='aac'))
        assert [fmt['format_id'] for fmt in xiaohongshu.video_info(ydl, note, NOTE)['formats']] == ['xhs_258']
        requested = format_key(original)
        info = ydl.process_ie_result(xiaohongshu.video_info(ydl, note, NOTE, requested), download=False)
        assert next(engine.select_format(info, requested))['url'] == url


def test_bad_exif_does_not_discard_valid_album_dimensions():
    with io.BytesIO() as image_bytes:
        Image.new('RGB', (80, 60)).save(image_bytes, format='JPEG', dpi=(72, 72), exif=b'Exif\0\0broken TIFF')
        body = image_bytes.getvalue()
    with Image.open(io.BytesIO(body)) as image:
        image.load()
    choice = dict(width=None, height=None)
    media_album.fill_image_size(choice, body)
    assert choice == dict(width=80, height=60)


def test_video_format_identity_is_stable_when_a_variant_disappears():
    hd = dict(streamType=258, streamDesc='X264_MP4', videoCodec='h264', audioCodec='aac',
              width=720, height=1280, masterUrl='https://sns-video-v6.xhscdn.com/hd.mp4')
    full_hd = dict(hd, width=1080, height=1920, masterUrl='https://sns-video-v6.xhscdn.com/full-hd.mp4')
    streams = [hd, full_hd]
    note = dict(noteId=NOTE_ID, title='fixture', type='video', video={'media': {'stream': {'h264': streams}}})
    with YoutubeDL({'quiet': True, 'format': engine.select_format}) as ydl:
        first = ydl.process_ie_result(xiaohongshu.video_info(ydl, note, NOTE), download=False)
        streams.remove(hd)
        full_hd['masterUrl'] += '?refreshed=fixture'
        second = ydl.process_ie_result(xiaohongshu.video_info(ydl, note, NOTE), download=False)
    selected = first['_yingqu_format_id']
    assert second['_yingqu_format_id'] == selected
    assert next(engine.select_format(second, selected))['width'] == 1080


@pytest.mark.parametrize('stream_ms,capa_seconds,media_seconds,expected_ms', [
    (7800, 7, 8, 7800),
    (82320, 82, 83, 82320),
    (None, 18, 19, 18000),
    (0, None, 19, 19000),
    (None, None, None, None),
])
def test_video_duration_uses_stream_milliseconds_or_note_seconds(stream_ms, capa_seconds, media_seconds, expected_ms):
    stream = dict(streamType=258, streamDesc='X264_MP4', videoCodec='h264', audioCodec='aac',
                  duration=stream_ms, masterUrl='https://sns-video-v6.xhscdn.com/fixture.mp4')
    note = dict(noteId=NOTE_ID, title='fixture', type='video', video={
        'capa': {'duration': capa_seconds},
        'media': {'video': {'duration': media_seconds}, 'stream': {'h264': [stream]}},
    })
    with YoutubeDL({'quiet': True, 'format': engine.select_format}) as ydl:
        info = ydl.process_ie_result(xiaohongshu.video_info(ydl, note, NOTE), download=False)
    assert engine.metadata(info, 'xiaohongshu')['duration_ms'] == expected_ms
