"""Narrow detail-request and video-dimension patch for yt-dlp 2026.08.19's Douyin extractor.

Formula provenance and update/removal conditions: vendor/UPSTREAM.md.
The upstream parser/downloader are reused; per-address dimensions correct missing 4k URL-key metadata.
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
            query = {'aid': '6383', 'channel': 'channel_pc_web', 'detail_list': '1',
                     **(kwargs.get('query') or {})}
            query['uifid'] = cookie.value
            # Match browser URLSearchParams serialization for the signature.
            encoded = urlencode(query, safe='*').replace('~', '%7E')
            payload = f'{cookie.value}__A96D855A08C0A9707F8BEF0D9A527E4E_{encoded}'
            query['x-secsdk-web-signature'] = hashlib.md5(payload.encode()).hexdigest()
            kwargs['query'] = query
        return super()._download_json(url, video_id, *args, **kwargs)


    def _parse_aweme_video_app(self, detail):
        info = super()._parse_aweme_video_app(detail)
        # The upstream URL-key parser misses e.g. bytevc1_4k. Use each actual
        # play address's dimensions, never the parent video's upload dimensions.
        video = detail.get('video') or {}
        addresses = [video.get(k) for k in ('play_addr', 'play_addr_265', 'play_addr_h264', 'play_addr_bytevc1')]
        addresses += [gear.get('play_addr') for gear in video.get('bit_rate', []) if isinstance(gear, dict)]
        dimensions = {}
        for addr in addresses:
            if not isinstance(addr, dict):
                continue
            width, height = addr.get('width'), addr.get('height')
            if type(width) is int and type(height) is int and width > 0 and height > 0:
                for url in addr.get('url_list') or []:
                    dimensions[url] = (width, height)
        for fmt in info.get('formats', []):
            if fmt.get('url') in dimensions:
                fmt['width'], fmt['height'] = dimensions[fmt['url']]
        return info
