from http.cookies import SimpleCookie

import pytest
from yt_dlp import YoutubeDL
from yt_dlp.utils import ExtractorError

import engine
import network
from douyin import DouyinIE, UpstreamDouyinIE


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
        'aweme_id': '123', 'uifid': 'fixture~*+ /',
        'x-secsdk-web-signature': 'a949734dd5a7b81cfe506822d8063af7',
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
