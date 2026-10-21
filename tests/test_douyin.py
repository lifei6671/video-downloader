from http.cookies import SimpleCookie
import copy
import json
from pathlib import Path
from urllib.parse import urlsplit

import pytest
from yt_dlp import YoutubeDL
from yt_dlp.utils import ExtractorError

import engine
import network
from douyin import DouyinIE, UpstreamDouyinIE
from video_formats import candidates, format_key, public_formats, select_format


DETAIL = 'https://www.douyin.com/aweme/v1/web/aweme/detail/'


@pytest.mark.parametrize('name', ['UIFID', 'UIFID_TEMP'])
def test_signature_matches_browser_serialization(name, monkeypatch):
    # Synthetic vector generated independently with Node URLSearchParams + crypto.
    cookies = SimpleCookie()
    cookies[name] = 'fixture~*+ /'
    extractor = DouyinIE()
    monkeypatch.setattr(extractor, '_get_cookies', lambda url: cookies)
    captured = {}
    def download(self, url, video_id, *args, **kwargs):
        captured.update(kwargs)
        return {'aweme_detail': {'aweme_id': video_id}}
    monkeypatch.setattr(UpstreamDouyinIE, '_download_json', download)
    original = {'aweme_id': '123'}
    assert extractor._download_json(DETAIL, '123', query=original)['aweme_detail']['aweme_id'] == '123'
    assert captured['query'] == {
        'aid': '6383', 'channel': 'channel_pc_web', 'detail_list': '1',
        'aweme_id': '123', 'uifid': 'fixture~*+ /',
        'x-secsdk-web-signature': 'bdbb9b87ca47022ef8f60d2812f50ad8',
    }
    assert original == {'aweme_id': '123'}


def test_missing_uifid_fails_before_request(monkeypatch):
    extractor = DouyinIE()
    monkeypatch.setattr(extractor, '_get_cookies', lambda url: SimpleCookie())
    monkeypatch.setattr(UpstreamDouyinIE, '_download_json',
                        lambda *a, **kw: pytest.fail('must not send unsigned request'))
    with pytest.raises(ExtractorError, match='UIFID cookie is required'):
        extractor._download_json(DETAIL, '123', query={'aweme_id': '123'})
    logger = engine.SafeLogger()
    logger.error('Douyin UIFID cookie is required')
    assert logger.code == 'ACCESS_DENIED'


def test_patch_only_changes_exact_detail_endpoint(monkeypatch):
    extractor = DouyinIE()
    monkeypatch.setattr(extractor, '_get_cookies', lambda url: pytest.fail('must not access cookies'))
    monkeypatch.setattr(UpstreamDouyinIE, '_download_json', lambda self, url, video_id, **kw: kw)
    for url in ['https://www.tiktok.com/api/item/detail/', 'https://www.douyin.com/other/']:
        assert extractor._download_json(url, '123', query={'id': '123'}) == {'query': {'id': '123'}}


@pytest.mark.parametrize('platform', ['douyin', 'tiktok'])
def test_production_registers_patch_only_for_douyin(platform, monkeypatch):
    def extract(self, url, download):
        assert isinstance(self.get_info_extractor('Douyin'), DouyinIE) == (platform == 'douyin')
        raise engine.EngineError('NOT_FOUND')
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    url = ('https://www.douyin.com/video/123' if platform == 'douyin'
           else 'https://www.tiktok.com/@u/video/123')
    with pytest.raises(engine.EngineError, match='NOT_FOUND'):
        engine.operate('resolve_video', {'input': url}, lambda *a, **kw: None)


def test_observed_douyin_cdn_stays_platform_scoped():
    network.validate_target('https://v26-default.365yg.com/video', 'douyin')
    for url, platform in [
        ('https://v26-default.365yg.com/video', 'tiktok'),
        ('https://365yg.com.attacker.invalid/video', 'douyin'),
        ('https://fake365yg.com/video', 'douyin'),
    ]:
        with pytest.raises(network.TargetError):
            network.validate_target(url, platform)


def test_wrapped_target_rejection_is_not_reported_as_connectivity(monkeypatch):
    from yt_dlp.networking.exceptions import RequestError
    def extract(*args, **kwargs):
        raise RequestError(cause=network.TargetError('NETWORK_TARGET_REJECTED'))
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    with pytest.raises(engine.EngineError, match='INVALID_INPUT') as caught:
        engine.operate('resolve_video', {'input': 'https://www.douyin.com/video/123'},
                       lambda *a, **kw: None)
    assert caught.value.stage == 'network_target'


def test_douyin_cover_domain_is_platform_scoped():
    from network import validate_target, TargetError
    validate_target('https://p81-sign.douyinpic.com/image', 'douyin')
    for url, platform in [('https://p81-sign.douyinpic.com/image', 'tiktok'),
                          ('https://douyinpic.com.attacker.invalid/image', 'douyin'),
                          ('https://fakedouyinpic.com/image', 'douyin')]:
        with pytest.raises(TargetError):
            validate_target(url, platform)


def test_each_play_address_dimensions_override_upload_and_unparsed_4k(monkeypatch):
    monkeypatch.setattr(UpstreamDouyinIE, '_parse_aweme_video_app', lambda *a: {
        'formats': [{'url': 'https://v.douyinvod.com/4k', 'width': None, 'height': None},
                    {'url': 'https://v.douyinvod.com/720', 'width': 2160, 'height': 3840}]})
    value = DouyinIE()._parse_aweme_video_app({'video': {
        'width': 2160, 'height': 3840,
        'play_addr': {'url_list': ['https://v.douyinvod.com/720'], 'width': 720, 'height': 1280},
        'bit_rate': [{'play_addr': {'url_list': ['https://v.douyinvod.com/4k'], 'width': 2160, 'height': 3840}}]}})
    assert [(f['width'], f['height']) for f in value['formats']] == [(2160, 3840), (720, 1280)]


@pytest.fixture
def media_detail():
    # Synthetic shape of the observed MP4/DASH response; no signed URLs or credentials.
    base = 'https://v.fixture.douyinvod.com/'
    muxed = dict(url_list=[base + 'muxed.mp4', base + 'mirror.mp4'],
                 width=1920, height=1080, data_size=10000, url_key='v1_bytevc1_1080p_400000')
    video = dict(url_list=[base + 'video.mp4'], width=1920, height=1080,
                 data_size=8000, url_key='v1_bytevc1_1080p_320000')
    return {'aweme_id': '123', 'desc': 'synthetic', 'video': {
        'format': 'dash', 'width': 1920, 'height': 1080, 'duration': 196000,
        'play_addr': muxed,
        'play_addr_h264': {'url_list': [base + 'original.mp3']},
        'play_addr_bytevc1': {'url_list': [base + 'audio.mp4']},
        'bit_rate': [
            {'format': 'mp4', 'play_addr': muxed, 'FPS': 24, 'is_h265': 1,
             'video_extra': json.dumps({'format': 'mp4', 'audio_channels': '2.0',
                                        'audio_sample_rate': '44100'})},
            {'format': 'dash', 'play_addr': video, 'FPS': 24, 'is_h265': 1,
             'video_extra': json.dumps({'format': 'dash', 'audio_file_id': 'audio-id'})}],
        'bit_rate_audio': [{'audio_meta': {'file_id': 'audio-id', 'media_type': 'audio',
                                           'url_list': [base + 'audio.mp4']}}]},
        'music': {'play_url': {'url_list': [base + 'original.mp3']}}}



def parse_media_detail(detail):
    with YoutubeDL({'quiet': True, 'logger': engine.SafeLogger()}) as ydl:
        info = DouyinIE(ydl)._parse_aweme_video_app(detail)
    # YoutubeDL adds this after extraction, before calling the format selector.
    for fmt in info['formats']:
        fmt['protocol'] = urlsplit(fmt['url']).scheme
    return info


@pytest.mark.parametrize('cover', [None, {}, {'url_list': []},
    {'url_list': [None, '', 'http://p.douyinpic.com/insecure']},
    {'url_list': [None, 'https://p.douyinpic.com/designed']}])
def test_original_scale_cover_priority_and_legacy_fallback(media_detail, cover):
    video = media_detail['video']
    video['cover'] = {'url_list': ['https://p.douyinpic.com/preview']}
    video['origin_cover'] = {'url_list': ['https://p.douyinpic.com/origin']}
    if cover is not None:
        video['cover_original_scale'] = cover
    original = copy.deepcopy(media_detail)
    info = parse_media_detail(media_detail)
    expected = ('https://p.douyinpic.com/designed' if cover and
                'https://p.douyinpic.com/designed' in cover.get('url_list', [])
                else 'https://p.douyinpic.com/origin')
    assert engine.cover_url(info) == expected
    assert media_detail == original


@pytest.mark.parametrize('numeric', [False, True])
def test_real_upstream_parser_classifies_each_url_and_offers_only_muxed(media_detail, numeric):
    if numeric:
        media_detail['video']['bit_rate'][0]['video_extra'] = {
            'format': 'mp4', 'audio_channels': 2, 'audio_sample_rate': 44100}
    original = copy.deepcopy(media_detail)
    info = parse_media_detail(media_detail)
    formats = {f['url'].rsplit('/', 1)[-1]: f for f in info['formats']}
    assert formats['muxed.mp4']['_yingqu_media_type'] == 'muxed'
    assert formats['mirror.mp4']['_yingqu_media_type'] == 'muxed'
    assert formats['video.mp4']['acodec'] == 'none'
    for name in ('audio.mp4', 'original.mp3'):
        assert formats[name]['vcodec'] == 'none'
        assert formats[name]['width'] is None and formats[name]['height'] is None
    assert len(public_formats(info)) == 1
    assert list(select_format(info))[0]['_yingqu_media_type'] == 'muxed'
    assert '_yingqu_media_type' not in json.dumps(public_formats(info))
    assert media_detail == original


@pytest.mark.parametrize('extra', [None, '', '{bad', [],
    {'format': 'dash', 'audio_channels': 2, 'audio_sample_rate': 44100},
    {'format': 'mp4', 'audio_channels': 0, 'audio_sample_rate': 44100},
    {'format': 'mp4', 'audio_channels': True, 'audio_sample_rate': 44100},
    {'format': 'mp4', 'audio_channels': 'bad', 'audio_sample_rate': '44100'},
    {'format': 'mp4', 'audio_channels': 'NaN', 'audio_sample_rate': '44100'},
    {'format': 'mp4', 'audio_channels': 2, 'audio_sample_rate': float('inf')}])
def test_missing_or_conflicting_audio_evidence_never_defaults_to_muxed(media_detail, extra):
    media_detail['video']['bit_rate'][0]['video_extra'] = extra
    info = parse_media_detail(media_detail)
    assert public_formats(info) == []
    with pytest.raises(engine.EngineError, match='UNSUPPORTED_CONTENT'):
        list(select_format(info))


def test_conflicting_url_and_unmatched_address_remain_unknown(media_detail):
    video = media_detail['video']
    video['bit_rate'][1]['play_addr'] = video['play_addr']
    video['play_addr_265'] = {'url_list': ['https://v.fixture.douyinvod.com/unmatched.mp4']}
    # This address is included by the upstream parser.
    video['download_addr'] = video['play_addr_265']
    info = parse_media_detail(media_detail)
    assert public_formats(info) == []
    assert all(f['_yingqu_media_type'] != 'muxed' for f in info['formats'])


def test_old_silent_selection_is_unavailable_without_downgrade(media_detail):
    info = parse_media_detail(media_detail)
    video = next(f for f in info['formats'] if f['_yingqu_media_type'] == 'video')
    old_id = format_key({**video, 'acodec': 'aac'})
    with pytest.raises(engine.EngineError, match='FORMAT_UNAVAILABLE'):
        list(select_format(info, old_id))
    info['formats'] = [video]
    assert candidates(info) == []


def test_resolve_and_download_reextract_use_same_muxed_candidates(media_detail, monkeypatch, tmp_path):
    requests = []
    def detail(self, url, video_id, *args, **kwargs):
        requests.append(video_id)
        return {'aweme_detail': copy.deepcopy(media_detail)}
    monkeypatch.setattr(DouyinIE, '_download_json', detail)
    monkeypatch.setattr(engine, 'download_cover', lambda *a: {})
    def download(self, info):
        assert info['_yingqu_media_type'] == 'muxed'
        assert info['url'].rsplit('/', 1)[-1] in ('muxed.mp4', 'mirror.mp4')
        Path(self.prepare_filename(info)).write_bytes(b'fixture media')
    monkeypatch.setattr(YoutubeDL, 'process_info', download)
    params = {'input': 'https://www.douyin.com/video/123', 'resolver': 'yt-dlp',
              'profiles': {'douyin': {'cookie': 'UIFID=synthetic'}}}
    events = []
    engine.operate('resolve_video', params, lambda kind, **kw: events.append((kind, kw)))
    result = events[-1][1]['result']
    assert len(result['formats']) == 1
    selected = result['selected_format_id']
    events.clear()
    engine.operate('download_video', {**params, 'expected': result, 'format_id': selected,
                   'staging_dir': str(tmp_path)}, lambda kind, **kw: events.append((kind, kw)))
    assert requests == ['123', '123']
    assert events[-1][0] == 'download_finished'
    assert events[-1][1]['format_id'] == selected
