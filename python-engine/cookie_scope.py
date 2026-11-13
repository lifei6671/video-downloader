"""Build a cookie jar with strict host-only cookie handling."""
from http.cookiejar import DefaultCookiePolicy

from yt_dlp.cookies import YoutubeDLCookieJar


def scoped_jar():
    return YoutubeDLCookieJar(policy=DefaultCookiePolicy(
        strict_ns_domain=DefaultCookiePolicy.DomainStrictNonDomain))
