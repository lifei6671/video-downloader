import io
import json
from types import SimpleNamespace
from urllib.request import Request

import pytest
import engine
from network import TargetGuard


HINTS = {'sec-ch-ua-platform': '"Windows"', 'sec-ch-ua-mobile': '?0',
         'sec-ch-ua': '"Microsoft Edge";v="149"'}


@pytest.mark.parametrize('platform,official,cdn', [
    ('bilibili', 'api.bilibili.com', 'v.bilivideo.com'),
    ('douyin', 'www.douyin.com', 'v.douyinvod.com'),
    ('tiktok', 'www.tiktok.com', 'v.tiktokcdn.com'),
    ('xiaohongshu', 'www.xiaohongshu.com', 'v.xhscdn.com'),
    ('youtube', 'www.youtube.com', 'v.googlevideo.com'),
])
def test_login_client_hints_reach_first_party_only_even_after_redirect(platform, official, cdn):
    guard = TargetGuard(platform, HINTS)
    first = Request('https://' + official + '/fixture')
    guard.http_request(first)
    assert first.get_header('Sec-ch-ua-platform') == '"Windows"'
    redirected = Request('https://' + cdn + '/media', headers=dict(first.header_items()))
    guard.http_request(redirected)
    assert not any(name.lower().startswith('sec-ch-ua') for name, _ in redirected.header_items())


def test_yuanbao_browser_identity_does_not_reach_video_or_avatar_cdn():
    request = Request('https://yuanbao.tencent.com/api/weixin/get_parse_result')
    TargetGuard('weixin-yuanbao', HINTS).http_request(request)
    assert request.get_header('Sec-ch-ua-platform') == '"Windows"'
    request = Request('https://finder.video.qq.com/media', headers=dict(request.header_items()))
    TargetGuard('weixin-media', HINTS).http_request(request)
    assert request.get_header('Sec-ch-ua-platform') is None


@pytest.mark.parametrize('extra', [
    {'client_hints': {'Cookie': 'secret'}},
    {'client_hints': {'sec-ch-ua': 'edge\r\nCookie: secret'}},
    {'browser_environment': {'screen_width': True}},
    {'browser_environment': {'hardware_concurrency': 0}},
    {'browser_environment': {'device_memory': float('nan')}},
    {'browser_environment': {'platform': 'Win32\n'}},
])
def test_engine_validates_browser_identity_at_protocol_boundary(extra):
    with pytest.raises(engine.EngineError, match='CONFIG_INVALID'):
        engine.profile_config({'user_agent': 'Native UA', **extra})


def test_dtk_uses_observed_browser_environment_without_altering_signed_request(monkeypatch):
    import dtk_adapter
    observed = {}
    class Adapter:
        platform = 'douyin'
        def profile_for(self, source):
            observed.update(vars(source))
            return SimpleNamespace(browser_platform=source.platform, screen_width=2560, screen_height=1440)
        def build_request(self, endpoint, **kwargs):
            return {'method': 'GET', 'url': 'https://www.douyin.com/fixture', 'params': {}, 'headers': {}}
    class Signer:
        def __init__(self, *args, **kwargs): pass
        async def sign(self, *args): return SimpleNamespace(headers={}, query='fixture=1')
    class Client:
        _yingqu_browser_environment = {'platform': 'Linux x86_64', 'language': 'en-US',
            'timezone': 'Europe/London', 'screen_width': 2560, 'screen_height': 1440,
            'hardware_concurrency': 24, 'device_memory': 4.0}
        def urlopen(self, request):
            assert request.headers['User-Agent'] == 'Native UA'
            return io.BytesIO(json.dumps({'status_code': 0}).encode())
    monkeypatch.setattr(dtk_adapter, 'NativeSigner', Signer)
    dtk_adapter.request_json(Client(), Adapter(), 'fixture', 'Native UA', {}, None)
    assert observed == dict(user_agent='Native UA', platform='Linux x86_64', language='en-US',
        timezone='Europe/London', screen='2560x1440', hardware_concurrency=24, device_memory=4)
