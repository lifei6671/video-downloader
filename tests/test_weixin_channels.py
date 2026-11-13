import json
from urllib.request import Request

import pytest

import weixin_channels as wx
from network import TargetError
from video_formats import public_formats, select_format

SHARE = 'https://weixin.qq.com/sph/Sample123'
MEDIA = 'https://finder.video.qq.com/example.mp4'


@pytest.fixture
def responses():
    return [
        {'code': 0, 'data': {'wx_export_id': 'fixture-export',
                             'playable_url': wx.PLAY_PAGE + '?eid=fixture-eid&token=fixture-token'}},
        {'errCode': 0, 'data': {'errMsg': {'type': 0}, 'authorInfo': {'nickname': '作者'},
                               'feedInfo': {'mediaType': 4, 'description': '作品', 'createtime': 1700000000,
                                            'videoUrl': MEDIA + '?variant=1',
                                            'h264VideoInfo': {'videoUrl': MEDIA + '?variant=1'},
                                            'h265VideoInfo': {'videoUrl': MEDIA + '?variant=2'}}}},
    ]


def mock_calls(monkeypatch, *responses):
    queue = iter(responses)
    calls = []

    def post(target, url, payload, ua, proxy, logger, cookies=None):
        calls.append((target, url, payload, cookies))
        result = next(queue)
        if isinstance(result, Exception):
            raise result
        return result

    monkeypatch.setattr(wx, 'post_json', post)
    return calls


def test_anonymous_success_never_uses_configured_cookie(monkeypatch, responses):
    calls = mock_calls(monkeypatch, *responses)
    info = wx.resolve(SHARE, 'UA', 'malformed cookie header', None, None)
    assert [call[0] for call in calls] == ['weixin-yuanbao', 'weixin-feed']
    assert all(call[3] is None for call in calls)
    assert calls[1][2] == {'baseReq': {'generalToken': 'fixture-token'}, 'exportId': 'fixture-eid'}
    assert info['title'] == '作品' and info['uploader'] == '作者'
    assert 'fixture-export' not in json.dumps(info)
    formats = public_formats(info)
    assert {f['codec'] for f in formats} == {'h264', 'h265'}
    assert all(f['width'] is None and f['filesize'] is None for f in formats)
    assert 'finder.video.qq.com' not in json.dumps(formats)


def test_manual_cookie_used_only_on_one_retry(monkeypatch, responses):
    calls = mock_calls(monkeypatch, wx.WeixinError('WEIXIN_LOGIN_REQUIRED'), *responses)
    wx.resolve(SHARE, 'UA', 'auth=test', None, None)
    assert [call[3] is None for call in calls] == [True, False, True]
    jar = calls[1][3]
    request = Request(wx.PARSE_URL)
    jar.add_cookie_header(request)
    assert request.get_header('Cookie') == 'auth=test'
    for url in (wx.FEED_URL, MEDIA, wx.YUANBAO + '/unrelated'):
        request = Request(url)
        jar.add_cookie_header(request)
        assert not request.has_header('Cookie')


def test_anonymous_authentication_failure_makes_only_one_request(monkeypatch):
    calls = mock_calls(monkeypatch, wx.WeixinError('WEIXIN_LOGIN_REQUIRED'))
    with pytest.raises(wx.WeixinError, match='^WEIXIN_LOGIN_REQUIRED$'):
        wx.resolve(SHARE, 'UA', '', None, None)
    assert len(calls) == 1 and calls[0][3] is None


def test_expired_cookie_reports_authentication_failure(monkeypatch):
    calls = mock_calls(monkeypatch, *[wx.WeixinError('WEIXIN_LOGIN_REQUIRED')] * 2)
    with pytest.raises(wx.WeixinError, match='^WEIXIN_COOKIE_EXPIRED$'):
        wx.resolve(SHARE, 'UA', 'auth=invalid', None, None)
    assert len(calls) == 2


@pytest.mark.parametrize('reply, expected', [
    ({'code': 0, 'data': {'wx_export_id': '', 'playable_url': ''}}, 'WEIXIN_VIDEO_UNAVAILABLE'),
    ({'code': 0, 'data': {}}, 'WEIXIN_UPSTREAM_CHANGED'),
    ({'code': 0, 'data': {'wx_export_id': 'id', 'playable_url': 'https://evil.test/?eid=e&token=t'}},
     'WEIXIN_UPSTREAM_CHANGED'),
])
def test_empty_content_and_schema_changes(monkeypatch, reply, expected):
    calls = mock_calls(monkeypatch, reply)
    with pytest.raises(wx.WeixinError, match='^' + expected + '$'):
        wx.resolve(SHARE, 'UA', {}, None, None)
    assert len(calls) == 1


def test_invalid_cdn_and_protected_duplicate(monkeypatch, responses):
    feed = responses[1]['data']['feedInfo']
    feed['h265VideoInfo']['videoUrl'] = 'https://127.0.0.1/example.mp4'
    mock_calls(monkeypatch, *responses)
    with pytest.raises(TargetError):
        wx.resolve(SHARE, 'UA', {}, None, None)
    feed['h264VideoInfo']['decodeKey'] = 'fake'
    del feed['h265VideoInfo']
    mock_calls(monkeypatch, *responses)
    with pytest.raises(wx.WeixinError, match='WEIXIN_MEDIA_PROTECTED'):
        wx.resolve(SHARE, 'UA', {}, None, None)


def test_api_proxy_resolves_dns_through_socks5(monkeypatch):
    from yt_dlp.networking.exceptions import HTTPError

    class Handler:
        def __init__(self, **kwargs):
            assert kwargs['proxies'] == {'all': 'socks5h://127.0.0.1:1080'}

        def __enter__(self):
            return self

        def __exit__(self, *args):
            pass

        def send(self, request):
            from yt_dlp.networking.common import Response
            from io import BytesIO
            raise HTTPError(Response(BytesIO(b''), wx.PARSE_URL, {}, 401))

    monkeypatch.setattr(wx, 'GuardedRH', Handler)
    with pytest.raises(wx.WeixinError, match='^WEIXIN_LOGIN_REQUIRED$'):
        wx.post_json('weixin-yuanbao', wx.PARSE_URL, {}, 'UA', 'socks5://127.0.0.1:1080', None)


def test_engine_identifies_online_and_rejects_changed_identity_or_format(monkeypatch, tmp_path, responses):
    import engine

    args = {'input': SHARE + '?tracking=ignored', 'report_resolve_progress': True}
    events = []
    emit = lambda event, **data: events.append((event, data))
    mock_calls(monkeypatch, *responses)
    engine.operate('identify_input', args, emit)
    identity = events[-1][1]['result']
    assert identity['canonical_url'] == SHARE and identity['resolver'] == 'weixin'
    assert [data['phase'] for event, data in events if event == 'resolve_progress'] == [
        'loading_configuration', 'resolving_link']
    assert identity['video_id'] != 'Sample123'
    mock_calls(monkeypatch, *responses)
    engine.operate('resolve_video', args, emit)
    metadata = events[-1][1]['result']
    assert metadata['selected_format_id'] == metadata['formats'][0]['format_id']
    assert metadata['formats'][0]['codec'] == 'h264'
    assert 'finder.video.qq.com' not in json.dumps(metadata)
    monkeypatch.setattr(engine, 'installed_tool', lambda _: 'ffprobe')
    monkeypatch.setattr(engine, 'YoutubeDL', lambda *_: pytest.fail('started transfer for wrong identity or format'))
    mock_calls(monkeypatch, *responses)
    download = args | {'staging_dir': str(tmp_path), 'expected': identity | {'video_id': 'b' * 64}}
    with pytest.raises(engine.EngineError, match='UPSTREAM_CHANGED'):
        engine.operate('download_video', download, emit)
    mock_calls(monkeypatch, *responses)
    with pytest.raises(engine.EngineError, match='FORMAT_UNAVAILABLE'):
        engine.operate('download_video', download | {'expected': identity, 'format_id': '0' * 24}, emit)


def test_preview_media_metadata_preserves_download_format_id(monkeypatch, responses):
    from io import BytesIO
    from subprocess import CompletedProcess

    feed = responses[1]['data']['feedInfo']
    info = {'formats': wx.format_choices(feed)}
    before = {fmt['codec']: fmt['format_id'] for fmt in public_formats(info)}

    class Partial(BytesIO):
        status = 206
        headers = {'Content-Range': 'bytes 0-131071/5112296'}

    class Client:
        def urlopen(self, request):
            assert request.headers['Range'] == 'bytes=0-131071'
            return Partial(b'fixture-prefix')

    def probe(command, **kwargs):
        assert kwargs['input'] == b'fixture-prefix'
        assert command[-1] == 'pipe:0'
        assert '-nofind_stream_info' in command
        return CompletedProcess(command, 0, b'{"format":{"format_name":"mov,mp4"},"streams":'
                                b'[{"width":1280,"height":720,"duration":"126.583333",'
                                b'"side_data_list":[{"rotation":90}]}]}')

    monkeypatch.setattr(wx.subprocess, 'run', probe)
    wx.probe_media_metadata(Client(), info['formats'], 'ffprobe')
    for fmt in public_formats(info):
        assert (fmt['width'], fmt['height']) == (720, 1280)
        assert fmt['filesize'] == 5112296
        assert fmt['format_id'] == before[fmt['codec']]
        assert next(select_format({'formats': wx.format_choices(feed)}, fmt['format_id']))
    assert all(fmt['duration'] == 126.583333 for fmt in info['formats'])

    without_ffprobe = wx.format_choices(feed)
    wx.probe_media_metadata(Client(), without_ffprobe, None)
    assert all(fmt['filesize'] == 5112296 and 'duration' not in fmt for fmt in without_ffprobe)


def test_author_avatar_requires_the_observed_cdn(monkeypatch, responses):
    responses[1]['data']['authorInfo']['headImgUrl'] = 'https://wx.qlogo.cn.evil.test/image'
    responses[0]['data']['author_icon'] = 'https://wx.qlogo.cn/image'
    mock_calls(monkeypatch, *responses)
    info = wx.resolve(SHARE, 'UA', '', None, None)
    assert info['_yingqu_author_avatar'] == 'https://wx.qlogo.cn/image'
    with pytest.raises(TargetError):
        wx.validate_target(info['_yingqu_author_avatar'], 'weixin-media')


@pytest.mark.parametrize('within_limit', [True, False])
def test_preview_reads_missing_mp4_header_bytes_only(monkeypatch, responses, within_limit):
    import struct
    from io import BytesIO
    from subprocess import CompletedProcess

    first_length = wx.MEDIA_PREFIX_BYTES
    movie_end = first_length + 12 if within_limit else wx.MAX_MP4_HEADER_BYTES + 1
    first = struct.pack('>I4s4sI4s', 20, b'ftyp', b'isom', 0, b'isom')
    first += struct.pack('>I4s', movie_end - len(first), b'moov')
    first = first.ljust(first_length, b'\0')
    suffix = b'\0' * 12
    total = movie_end + 1024
    ranges = []
    fmt = wx.format_choices(responses[1]['data']['feedInfo'])[0]

    class Client:
        def urlopen(self, request):
            ranges.append(request.headers['Range'])
            start, end = (0, first_length - 1) if len(ranges) == 1 else (first_length, movie_end - 1)
            response = BytesIO(first if start == 0 else suffix)
            response.status = 206
            response.headers = {'Content-Range': f'bytes {start}-{end}/{total}'}
            return response

    def probe(command, **kwargs):
        assert within_limit and kwargs['input'] == first + suffix
        return CompletedProcess(command, 0, b'{"format":{"format_name":"mov,mp4"},"streams":'
                                b'[{"width":1024,"height":576,"duration":"5628.745687"}]}')

    monkeypatch.setattr(wx.subprocess, 'run', probe)
    wx.probe_media_metadata(Client(), [fmt], 'ffprobe')
    assert fmt['filesize'] == total
    assert ranges == [f'bytes=0-{first_length - 1}'] + (
        [f'bytes={first_length}-{movie_end - 1}'] if within_limit else [])
    assert fmt.get('duration') == (5628.745687 if within_limit else None)
    assert (fmt.get('width'), fmt.get('height')) == ((1024, 576) if within_limit else (None, None))
