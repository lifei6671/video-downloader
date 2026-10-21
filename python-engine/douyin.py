"""Detail-request, dimensions and media-type patch for yt-dlp's Douyin extractor.

Formula provenance and update/removal conditions: vendor/UPSTREAM.md.
The upstream parser/downloader are reused; per-address dimensions correct missing 4k URL-key metadata.
"""
import hashlib
import json
from urllib.parse import urlencode

from yt_dlp.extractor.tiktok import DouyinIE as UpstreamDouyinIE
from yt_dlp.utils import ExtractorError, float_or_none, url_or_none
from yt_dlp.utils.traversal import traverse_obj
from video_formats import positive


def original_scale_cover(detail):
    for url in traverse_obj(detail, ('video', 'cover_original_scale', 'url_list', ..., {url_or_none})):
        if url.startswith('https://'):
            return url
    return None


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
        video = detail.get('video') or {}
        cover = original_scale_cover(detail)
        if cover:
            info['thumbnails'].append(dict(id='cover_original_scale', url=cover, preference=0))
        # The upstream URL-key parser misses e.g. bytevc1_4k. Use each actual
        # play address's dimensions, never the parent video's upload dimensions.
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

        # The upstream parser assumes AAC for every video address, including
        # video-only DASH. Classify each actual URL, not video['format'] (which
        # can be DASH even when the response also offers muxed MP4).
        media_types = {}
        audio_ids = set()
        for audio in video.get('bit_rate_audio') or []:
            meta = audio.get('audio_meta') if isinstance(audio, dict) else None
            if isinstance(meta, dict) and meta.get('media_type') == 'audio':
                if isinstance(meta.get('file_id'), str) and meta['file_id']:
                    audio_ids.add(meta['file_id'])
                for url in meta.get('url_list') or []:
                    media_types.setdefault(url, set()).add('audio')
        music = (detail.get('music') or {}).get('play_url') or {}
        for url in music.get('url_list') or []:
            media_types.setdefault(url, set()).add('audio')
        for gear in video.get('bit_rate') or []:
            if not isinstance(gear, dict):
                continue
            extra = gear.get('video_extra')
            if isinstance(extra, str):
                try:
                    extra = json.loads(extra)
                except ValueError:
                    extra = None
            kind = 'unknown'
            if isinstance(extra, dict) and extra.get('format') == gear.get('format'):
                # video_extra encodes these as numeric strings on the web API.
                audio_fields = [positive(float_or_none(extra.get(key)))
                                if type(extra.get(key)) in (str, int, float) else None
                                for key in ('audio_channels', 'audio_sample_rate')]
                if gear.get('format') == 'mp4' and all(audio_fields):
                    kind = 'muxed'
                elif (gear.get('format') == 'dash' and isinstance(extra.get('audio_file_id'), str)
                        and extra['audio_file_id'] in audio_ids):
                    kind = 'video'
            for url in (gear.get('play_addr') or {}).get('url_list') or []:
                media_types.setdefault(url, set()).add(kind)
        for fmt in info.get('formats', []):
            kinds = media_types.get(fmt.get('url'), set())
            kind = next(iter(kinds)) if len(kinds) == 1 else 'unknown'
            fmt['_yingqu_media_type'] = kind
            if kind == 'video':
                fmt['acodec'] = 'none'
            elif kind == 'audio':
                fmt['vcodec'] = 'none'
                fmt['width'] = fmt['height'] = None
            elif kind == 'unknown':
                fmt['acodec'] = None
        return info
