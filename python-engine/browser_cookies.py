"""Read only the OS default browser; never persist its cookies or try other browsers."""
import plistlib
import sys
from pathlib import Path

from yt_dlp.cookies import extract_cookies_from_browser


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
