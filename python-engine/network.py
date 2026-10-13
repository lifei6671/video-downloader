"""Bounded urllib transport for the pinned yt-dlp version, including redirects."""
import functools
import base64
import http.client
import ipaddress
import socket
import urllib.request
from urllib.parse import unquote, urlsplit

from yt_dlp.networking._urllib import (
    HTTPHandler, ProxyHandler, RedirectHandler, UrllibRH, _create_http_connection,
)
from yt_dlp.utils.networking import select_proxy

# Platform web/API and video CDN suffixes used by the two pinned extractors.
# This is application policy, not a generated dtk asset or a general-site downloader.
DOMAINS = {
    'douyin': ('douyin.com', 'iesdouyin.com', 'amemv.com', 'douyinvod.com',
               'bytecdn.cn', 'byteimg.com', 'douyinstatic.com', 'zjcdn.com', '365yg.com', 'douyinpic.com'),
    'tiktok': ('tiktok.com', 'tiktokv.com', 'tiktokcdn.com', 'tiktokcdn-us.com',
               'tiktokcdn-eu.com', 'tiktokv.us', 'ibytedtos.com', 'isnssdk.com',
               'musical.ly', 'muscdn.com'),
}


class TargetError(ValueError):
    pass


def validate_target(url, platform):
    try:
        parts = urlsplit(url)
        host = parts.hostname or ''
        if (parts.scheme not in ('http', 'https') or parts.username or parts.password
                or parts.port not in (None, 80 if parts.scheme == 'http' else 443)
                or not any(host == d or host.endswith('.' + d) for d in DOMAINS[platform])):
            raise ValueError()
    except (ValueError, KeyError):
        raise TargetError('NETWORK_TARGET_REJECTED') from None


def public_connection(address, timeout=socket._GLOBAL_DEFAULT_TIMEOUT, source_address=None):
    """Resolve once, validate every address, then connect to those exact sockaddr values."""
    host, port = address
    addresses = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    if not addresses or any(not ipaddress.ip_address(a[4][0]).is_global for a in addresses):
        raise TargetError('NETWORK_TARGET_REJECTED')
    last_error = None
    for family, kind, proto, _, sockaddr in addresses:
        connection = socket.socket(family, kind, proto)
        try:
            if timeout is not socket._GLOBAL_DEFAULT_TIMEOUT:
                connection.settimeout(timeout)
            if source_address:
                connection.bind(source_address)
            connection.connect(sockaddr)
            return connection
        except OSError as error:
            last_error = error
            connection.close()
    raise last_error


class TargetGuard(urllib.request.BaseHandler):
    handler_order = 50

    def __init__(self, platform):
        self.platform = platform

    def http_request(self, request):
        validate_target(request.full_url, self.platform)
        # Authentication belongs to the scoped CookieJar or configured proxy only.
        request.remove_header('Authorization')
        request.remove_header('Cookie')
        return request

    https_request = http_request


class GuardedHTTPHandler(HTTPHandler):
    def __init__(self, *, direct, **kwargs):
        super().__init__(**kwargs)
        self.direct = direct

    def connection(self, base, *args, **kwargs):
        connection = _create_http_connection(base, self._source_address, *args, **kwargs)
        if self.direct:
            connection._create_connection = public_connection
        return connection

    def http_open(self, request):
        base = self._make_conn_class(http.client.HTTPConnection, request)
        return self.do_open(functools.partial(self.connection, base), request)

    def https_open(self, request):
        base = self._make_conn_class(http.client.HTTPSConnection, request)
        return self.do_open(functools.partial(self.connection, base), request, context=self._context)


class ExplicitProxyHandler(ProxyHandler):
    def proxy_open(self, request):
        proxy = select_proxy(request.full_url, self.proxies)
        if proxy is None or urlsplit(proxy).scheme != 'http':
            return super().proxy_open(request)
        # Profiles specify the route explicitly. urllib's proxy_bypass (including
        # NO_PROXY and Windows system exclusions) must not silently select direct.
        parts = urlsplit(proxy)
        if parts.username is not None and parts.password is not None:
            credentials = f'{unquote(parts.username)}:{unquote(parts.password)}'
            token = base64.b64encode(credentials.encode()).decode('ascii')
            request.add_header('Proxy-authorization', 'Basic ' + token)
        request.set_proxy(parts.netloc.rsplit('@', 1)[-1], 'http')
        return None


class GuardedRH(UrllibRH):
    _SUPPORTED_URL_SCHEMES = ('http', 'https')
    _SUPPORTED_PROXY_SCHEMES = ('http', 'socks5', 'socks5h')  # yt-dlp normalizes socks5 to socks5h

    def __init__(self, *, platform, **kwargs):
        super().__init__(**kwargs)
        self.platform = platform

    def _create_instance(self, proxies, cookiejar, legacy_ssl_support=None):
        # Keep yt-dlp's HTTP decoding, SOCKS, cookie and redirect implementation.
        # Only add a per-hop guard and pin direct sockets to validated DNS results.
        opener = urllib.request.OpenerDirector()
        for handler in (
            TargetGuard(self.platform), ExplicitProxyHandler(proxies),
            GuardedHTTPHandler(direct=not any(proxies.values()),
                context=self._make_sslcontext(legacy_ssl_support=legacy_ssl_support)),
            urllib.request.HTTPCookieProcessor(cookiejar),
            urllib.request.UnknownHandler(), urllib.request.HTTPDefaultErrorHandler(),
            urllib.request.HTTPErrorProcessor(), RedirectHandler(),
        ):
            opener.add_handler(handler)
        opener.addheaders = []
        return opener
