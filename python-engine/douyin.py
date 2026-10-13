"""Narrow request-signature patch for yt-dlp 2026.08.19's Douyin extractor.

Formula provenance and update/removal conditions: vendor/UPSTREAM.md.
The upstream extractor, parser and downloader otherwise remain unchanged.
"""
import hashlib
from urllib.parse import urlencode

from yt_dlp.extractor.tiktok import DouyinIE as UpstreamDouyinIE
from yt_dlp.utils import ExtractorError


class DouyinIE(UpstreamDouyinIE):
    def _download_json(self, url, video_id, *args, **kwargs):
        if url == 'https://www.douyin.com/aweme/v1/web/aweme/detail/':
            cookies = self._get_cookies(self._WEBPAGE_HOST)
            cookie = cookies.get('UIFID') or cookies.get('UIFID_TEMP')
            if not cookie or not cookie.value:
                raise ExtractorError('Douyin UIFID cookie is required', expected=True)
            query = dict(kwargs.get('query') or {})
            query['uifid'] = cookie.value
            # Match browser URLSearchParams serialization for the signature.
            encoded = urlencode(query, safe='*').replace('~', '%7E')
            payload = f'{cookie.value}__A96D855A08C0A9707F8BEF0D9A527E4E_{encoded}'
            query['x-secsdk-web-signature'] = hashlib.md5(payload.encode()).hexdigest()
            kwargs['query'] = query
        return super()._download_json(url, video_id, *args, **kwargs)
