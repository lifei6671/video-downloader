"""Bounded urllib transport for the pinned yt-dlp version, including redirects."""
import functools
import base64
import http.client
import re
import socket
import urllib.request
from urllib.parse import unquote, urlsplit

from yt_dlp.networking._urllib import (
    HTTPHandler, ProxyHandler, RedirectHandler, UrllibRH, _create_http_connection,
)
from yt_dlp.utils.networking import select_proxy
from yt_dlp.networking.common import Response
from yt_dlp.networking.exceptions import TransportError, IncompleteRead

# Platform web/API and video CDN suffixes used by the two pinned extractors.
# This is application policy, not a generated dtk asset or a general-site downloader.
DOMAINS = {
    'youtube': ('youtube.com', 'youtu.be', 'youtube-nocookie.com', 'googlevideo.com',
                'ytimg.com', 'youtubei.googleapis.com'),
    'douyin': ('douyin.com', 'iesdouyin.com', 'amemv.com', 'douyinvod.com',
               'bytecdn.cn', 'byteimg.com', 'douyinstatic.com', 'zjcdn.com', '365yg.com', 'douyinpic.com'),
    'tiktok': ('tiktok.com', 'tiktokv.com', 'tiktokcdn.com', 'tiktokcdn-us.com',
               'tiktokcdn-eu.com', 'tiktokv.us', 'ibytedtos.com', 'isnssdk.com',
               'musical.ly', 'muscdn.com'),
}


class TargetError(ValueError):
    def __init__(self, message='NETWORK_TARGET_REJECTED', *, code='INVALID_INPUT'):
        super().__init__(message)
        self.code = code


class MediaRangeError(ValueError):
    pass


class LengthCheckedResponse(Response):
    """Detect early EOF before HttpFD can treat a truncated chunk as NextFragment."""
    def __init__(self, response):
        super().__init__(response, response.url, response.headers, response.status,
                         response.reason, response.extensions)
        length = response.headers.get('Content-Length', '').strip()
        self.expected = int(length) if length.isdecimal() and not response.headers.get('Content-Encoding') else None
        if self.expected is None and response.status == 206 and not response.headers.get('Content-Encoding'):
            received = re.fullmatch(r'bytes (\d+)-(\d+)/(\d+)', response.headers.get('Content-Range', ''))
            if received:
                self.expected = int(received[2]) - int(received[1]) + 1
        self.received = 0

    def read(self, amt=None):
        data = super().read(amt)
        self.received += len(data)
        if (amt != 0 and (not data or self.fp.closed) and self.expected is not None
                and self.received < self.expected):
            self.close()
            raise IncompleteRead(self.received, self.expected - self.received)
        return data


def validate_target(url, platform):
    try:
        parts = urlsplit(url)
        host = parts.hostname or ''
        if (parts.scheme not in ('http', 'https') or parts.username or parts.password
                or parts.port not in (None, 80 if parts.scheme == 'http' else 443)):
            raise ValueError()
        # Reject Google's YouTube challenge redirect without misreporting the input link.
        # This classifies the failure; it does not allow requests to google.com.
        if (platform == 'youtube' and parts.scheme == 'https' and host == 'www.google.com'
                and parts.path in ('/sorry/index', '/sorry/')):
            raise TargetError('RISK_CONTROL', code='RISK_CONTROL')
        if not any(host == d or host.endswith('.' + d) for d in DOMAINS[platform]):
            raise ValueError()
    except TargetError:
        raise
    except (ValueError, KeyError):
        raise TargetError() from None


def direct_connection(address, timeout=socket._GLOBAL_DEFAULT_TIMEOUT, source_address=None):
    """Resolve once and use OS routing, including private addresses returned by TUN DNS."""
    host, port = address
    addresses = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    if not addresses:
        raise OSError('getaddrinfo returned no addresses')
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

    def http_response(self, request, response):
        if response.status == 206:
            requested = re.fullmatch(r'bytes=(\d+)-(\d*)', request.get_header('Range') or '')
            received = re.fullmatch(r'bytes (\d+)-(\d+)/(\d+)', response.headers.get('Content-Range', ''))
            valid = False
            if requested and received:
                start, end, total = map(int, received.groups())
                valid = (start == int(requested[1]) and start <= end < total
                         and (not requested[2] or end == min(int(requested[2]), total - 1)))
                length = response.headers.get('Content-Length')
                if valid and length is not None:
                    valid = bool(re.fullmatch(r'[0-9]+', length.strip())) and int(length) == end - start + 1
            if not valid:
                response.close()
                raise TransportError(cause=MediaRangeError('INCOMPLETE_MEDIA'))
        return response

    https_response = http_response


class GuardedHTTPHandler(HTTPHandler):
    def __init__(self, *, direct, **kwargs):
        super().__init__(**kwargs)
        self.direct = direct

    def connection(self, base, *args, **kwargs):
        connection = _create_http_connection(base, self._source_address, *args, **kwargs)
        if self.direct:
            connection._create_connection = direct_connection
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

    def _send(self, request):
        response = super()._send(request)
        return LengthCheckedResponse(response) if request.method == 'GET' else response

    def _create_instance(self, proxies, cookiejar, legacy_ssl_support=None):
        # Keep yt-dlp's HTTP decoding, SOCKS, cookie and redirect implementation.
        # Guard each URL hop and connect directly to the OS resolver's addresses.
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
