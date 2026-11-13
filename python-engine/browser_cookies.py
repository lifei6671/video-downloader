"""Read only the OS default browser; never persist its cookies or try other browsers."""
import plistlib
import sys
from http.cookiejar import DefaultCookiePolicy
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import Request

from yt_dlp.cookies import YDLLogger, YoutubeDLCookieJar, extract_cookies_from_browser


def default_browser():
    if sys.platform == 'win32':
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                r'Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice') as key:
            identity = winreg.QueryValueEx(key, 'ProgId')[0].lower()
        prefixes = {'chromehtml': 'chrome', 'msedgehtm': 'edge', 'firefoxurl': 'firefox',
                    'bravehtml': 'brave', 'operastable': 'opera'}
    elif sys.platform == 'darwin':
        file = Path.home() / 'Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist'
        with file.open('rb') as stream:
            handlers = plistlib.load(stream).get('LSHandlers', [])
        identity = next((h.get('LSHandlerRoleAll', '').lower() for h in handlers
                         if h.get('LSHandlerURLScheme') == 'https'), 'com.apple.safari')
        prefixes = {'com.google.chrome': 'chrome', 'com.microsoft.edgemac': 'edge',
                    'org.mozilla.firefox': 'firefox', 'com.brave.browser': 'brave',
                    'com.operasoftware.opera': 'opera', 'com.apple.safari': 'safari'}
        return prefixes.get(identity)
    else:
        return None
    return next((browser for prefix, browser in prefixes.items() if identity.startswith(prefix)), None)


def load_default_cookies(jar, platform, logger):
    try:
        browser = default_browser()
        if not browser:
            return 'BROWSER_COOKIES_UNAVAILABLE'
        source = extract_cookies_from_browser(browser, logger=logger)
        domains = (platform + '.com',)
        # Authentication cookies must never cross platform boundaries.
        copied = 0
        for cookie in source:
            domain = cookie.domain.lstrip('.').lower()
            if not cookie.is_expired() and any(domain == d or domain.endswith('.' + d) for d in domains):
                jar.set_cookie(cookie)
                copied += 1
        return None if copied else 'BROWSER_COOKIES_UNAVAILABLE'
    except Exception:
        # OS, keychain, locked SQLite and unsupported encryption errors contain private paths.
        return 'BROWSER_COOKIES_UNAVAILABLE'


class ScopedCookieLogger(YDLLogger):
    """Keep browser paths and keychain diagnostics out of the protocol."""
    def __init__(self):
        super().__init__()
        self.failure = None

    def warning(self, message, *args, **kwargs):
        message = str(message).lower()
        if 'could not copy' in message:
            self.failure = 'COOKIE_DATABASE_COPY_FAILED'
        elif any(word in message for word in ('decrypt', 'dpapi', 'app-bound')):
            self.failure = 'COOKIE_DECRYPT_FAILED'

    error = warning


def scoped_jar():
    return YoutubeDLCookieJar(policy=DefaultCookiePolicy(
        strict_ns_domain=DefaultCookiePolicy.DomainStrictNonDomain))


def load_default_cookies_for_url(url):
    """Read default-browser cookies applicable to this HTTPS request only."""
    browser = None
    logger = ScopedCookieLogger()
    try:
        browser = default_browser()
        if not browser:
            return browser, 'BROWSER_UNRECOGNIZED', None
        source = extract_cookies_from_browser(browser, logger=logger)
        host = urlsplit(url).hostname
        jar = scoped_jar()
        expired = False
        for cookie in source:
            domain = cookie.domain.lstrip('.').lower()
            # Chromium's yt-dlp adapter sets domain_specified for host-only cookies too.
            if domain == host or (cookie.domain_initial_dot and host.endswith('.' + domain)):
                expired |= cookie.is_expired()
                jar.set_cookie(cookie)
        request = Request(url)
        jar.add_cookie_header(request)
        if request.has_header('Cookie'):
            return browser, 'COOKIES_LOADED', jar
        return browser, logger.failure or ('COOKIES_EXPIRED' if expired else 'NO_APPLICABLE_COOKIES'), None
    except Exception as error:
        # yt-dlp's default error path can raise without calling the logger.
        logger.error(error)
        return browser, logger.failure or 'BROWSER_COOKIE_READ_FAILED', None
