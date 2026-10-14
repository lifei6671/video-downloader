import socket

import pytest

import network


def test_direct_dns_is_pinned_and_never_resolved_twice(monkeypatch):
    lookups=[]
    connected=[]
    class Socket:
        def settimeout(self,value): pass
        def connect(self,address): connected.append(address)
        def close(self): pass
    def lookup(host,port,**kwargs):
        lookups.append((host,port))
        assert len(lookups)==1
        return [(socket.AF_INET,socket.SOCK_STREAM,6,'',('8.8.8.8',port))]
    monkeypatch.setattr(network.socket,'getaddrinfo',lookup)
    monkeypatch.setattr(network.socket,'socket',lambda *args:Socket())
    network.direct_connection(('v.tiktokcdn.com',443),timeout=1)
    assert lookups == [('v.tiktokcdn.com',443)]
    assert connected == [('8.8.8.8',443)]


@pytest.mark.parametrize('ip', ['8.8.8.8', '198.18.0.1', '100.64.0.1', 'fc00::63', '127.0.0.1', '::1'])
def test_direct_connection_accepts_system_and_tun_dns_addresses(ip, monkeypatch):
    family = socket.AF_INET6 if ':' in ip else socket.AF_INET
    target = (ip, 443, 0, 0) if family == socket.AF_INET6 else (ip, 443)
    connected = []
    class Socket:
        def settimeout(self, value): pass
        def connect(self, address): connected.append(address)
        def close(self): pass
    monkeypatch.setattr(network.socket, 'getaddrinfo', lambda *a, **kw: [
        (family, socket.SOCK_STREAM, 6, '', target)])
    monkeypatch.setattr(network.socket, 'socket', lambda *a: Socket())
    network.direct_connection(('www.youtube.com', 443))
    assert connected == [target]


def test_direct_connection_tries_resolved_addresses_in_order(monkeypatch):
    lookups, connected, closed = [], [], []
    addresses = [
        (socket.AF_INET6, socket.SOCK_STREAM, 6, '', ('fc00::63', 443, 0, 0)),
        (socket.AF_INET, socket.SOCK_STREAM, 6, '', ('198.18.0.1', 443))]
    class Socket:
        def settimeout(self, value): pass
        def connect(self, address):
            connected.append(address)
            if len(connected) == 1:
                raise OSError('fixture IPv6 route unavailable')
        def close(self): closed.append(True)
    def lookup(*args, **kwargs):
        lookups.append(args)
        return addresses
    monkeypatch.setattr(network.socket, 'getaddrinfo', lookup)
    monkeypatch.setattr(network.socket, 'socket', lambda *a: Socket())
    network.direct_connection(('www.youtube.com', 443))
    assert len(lookups) == 1
    assert connected == [entry[4] for entry in addresses] and len(closed) == 1


def test_yt_dlp_uses_only_guarded_handler(monkeypatch):
    import engine
    from yt_dlp import YoutubeDL
    observed={}
    def extract(self,url,download):
        observed['handlers']=list(self._request_director.handlers.values())
        observed['proxy']=self.proxies
        observed['plugins']=engine.plugin_dirs.value
        raise engine.EngineError('NOT_FOUND')
    monkeypatch.setenv('HTTP_PROXY','http://unexpected.invalid:1234')
    monkeypatch.setattr(YoutubeDL,'extract_info',extract)
    with pytest.raises(engine.EngineError):
        engine.operate('resolve_video',{'input':'https://www.tiktok.com/@u/video/123',
            'profiles':{'tiktok':{'proxy':None}}},lambda *a,**kw:None)
    assert len(observed['handlers'])==1
    assert isinstance(observed['handlers'][0],network.GuardedRH)
    assert not any(observed['handlers'][0].proxies.values())
    assert observed['plugins']==[]


def test_direct_guard_is_used_by_real_request_director(monkeypatch):
    import functools
    from yt_dlp import YoutubeDL
    from yt_dlp.networking.exceptions import RequestError
    connected = []
    def connect(address, *args, **kwargs):
        connected.append(address)
        raise OSError('fixture stops before opening a socket')
    monkeypatch.setattr(network, 'direct_connection', connect)
    with YoutubeDL({'proxy':'','quiet':True}) as ydl:
        ydl._request_director = ydl.build_request_director([functools.partial(network.GuardedRH,platform='tiktok')])
        with pytest.raises(RequestError):
            ydl.urlopen('http://v.tiktokcdn.com/video')
    assert connected == [('v.tiktokcdn.com', 80)]


def test_explicit_http_proxy_ignores_environment_and_system_bypass(monkeypatch, media_proxy):
    import functools
    import urllib.request
    from yt_dlp import YoutubeDL
    proxy, requests, _, _ = media_proxy
    monkeypatch.setenv('NO_PROXY', '*')
    monkeypatch.setattr(urllib.request, 'proxy_bypass', lambda host: True)
    monkeypatch.setattr(network, 'direct_connection', lambda *a, **kw: pytest.fail('unexpected direct route'))
    with YoutubeDL({'proxy': proxy, 'quiet': True}) as ydl:
        ydl._request_director = ydl.build_request_director([
            functools.partial(network.GuardedRH, platform='tiktok')])
        with ydl.urlopen('http://v.tiktokcdn.com/good.mp4') as response:
            assert response.read().startswith(b'\x00\x00\x00\x18ftyp')
    assert requests[0][0] == 'http://v.tiktokcdn.com/good.mp4'


def test_explicit_https_tunnel_ignores_bypass(monkeypatch):
    import functools
    import urllib.request
    from yt_dlp import YoutubeDL
    from yt_dlp.networking import _urllib
    from yt_dlp.networking.exceptions import RequestError
    connected = []
    def connect(address, *args, **kwargs):
        connected.append(address)
        raise OSError('test stops before socket')
    monkeypatch.setenv('NO_PROXY', '*')
    monkeypatch.setattr(urllib.request, 'proxy_bypass', lambda host: True)
    monkeypatch.setattr(_urllib, 'create_connection', connect)
    monkeypatch.setattr(network, 'direct_connection', lambda *a, **kw: pytest.fail('unexpected direct route'))
    with YoutubeDL({'proxy': 'http://127.0.0.1:3067', 'quiet': True}) as ydl:
        ydl._request_director = ydl.build_request_director([
            functools.partial(network.GuardedRH, platform='tiktok')])
        with pytest.raises(RequestError):
            ydl.urlopen('https://v.tiktokcdn.com/video')
    assert connected == [('127.0.0.1', 3067)]
