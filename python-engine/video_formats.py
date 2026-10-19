"""Single-file video choices; no media URLs in public data."""
import hashlib
import json
import math
import re


def positive(value):
    return value if type(value) in (int, float) and math.isfinite(value) and value > 0 else None


def format_key(fmt):
    # CDN mirrors get numeric suffixes. Signing tokens/URLs and sizes are not identities.
    name = re.sub(r'-\d+$', '', str(fmt.get('format_id') or ''))
    identity = [name, *(fmt.get(k) for k in ('ext', 'width', 'height', 'vcodec', 'acodec', 'tbr', 'fps'))]
    return hashlib.sha256(json.dumps(identity, separators=(',', ':')).encode()).hexdigest()[:24]


def candidates(context, merge=False):
    formats = context.get('formats') or []
    if merge:
        expanded = []
        for video in formats:
            if video.get('acodec') != 'none' or video.get('vcodec') in (None, 'none'):
                expanded.append(video)
                continue
            audio_ext = {'mp4': 'm4a', 'webm': 'webm'}.get(video.get('ext'))
            audio = next((a for a in reversed(formats) if a.get('vcodec') == 'none'
                          and a.get('acodec') not in (None, 'none') and not a.get('has_drm')
                          and a.get('protocol') in ('http', 'https') and a.get('ext') == audio_ext), None)
            if not audio:
                continue
            combined = {**video, 'format_id': f'{video["format_id"]}+{audio["format_id"]}',
                        'acodec': audio['acodec'], 'requested_formats': [video, audio],
                        'protocol': video.get('protocol', '') + '+' + audio['protocol'],
                        'requires_merge': True}
            for field in ('filesize', 'filesize_approx'):
                sizes = [positive(f.get(field)) for f in (video, audio)]
                combined[field] = sum(sizes) if all(sizes) else None
            expanded.append(combined)
        formats = expanded
    separate_audio = any(f.get('vcodec') == 'none' and f.get('acodec') != 'none' for f in formats)
    choices = {}
    for fmt in formats:
        protocols = str(fmt.get('protocol', '')).split('+')
        if (not all(p in ('http', 'https') for p in protocols) or (len(protocols) != 1 and not merge)
                or ('_yingqu_media_type' in fmt and fmt['_yingqu_media_type'] != 'muxed')
                or fmt.get('ext') not in ('mp4', 'webm')
                or fmt.get('vcodec') in ('none', 'bytevc2') or fmt.get('has_drm')
                or 'watermark' in str(fmt.get('format_note', '')).lower()
                or fmt.get('watermark') or (separate_audio and fmt.get('acodec') == 'none')):
            continue
        if fmt.get('acodec') == 'none' and any(f.get('acodec') != 'none' and f.get('vcodec') != 'none' for f in formats):
            continue
        # yt-dlp supplies worst -> best; keep its preferred CDN mirror within each choice.
        choices[format_key(fmt)] = fmt
    return sorted(choices.items(), key=lambda item: (
        (positive(item[1].get('width')) or 0) * (positive(item[1].get('height')) or 0),
        positive(item[1].get('fps')) or 0, positive(item[1].get('tbr')) or 0,
        positive(item[1].get('filesize')) or 0), reverse=True)


def select_format(context, selected=None, merge=False):
    from engine import EngineError
    choices = candidates(context, merge)
    if selected is not None:
        if not isinstance(selected, str) or not re.fullmatch(r'[a-f0-9]{24}', selected):
            raise EngineError('ENGINE_PROTOCOL_ERROR', 'format')
        choices = [item for item in choices if item[0] == selected]
        if not choices:
            raise EngineError('FORMAT_UNAVAILABLE', 'format')
    if not choices:
        raise EngineError('UNSUPPORTED_CONTENT', 'format')
    key, fmt = choices[0]
    yield {**fmt, '_yingqu_format_id': key}


def public_formats(info, merge=False):
    result = []
    for key, fmt in candidates(info, merge):
        item = dict(format_id=key, container=fmt['ext'], codec=fmt.get('vcodec'))
        if merge:
            item['requires_merge'] = bool(fmt.get('requires_merge'))
        for field in ('width', 'height', 'filesize', 'filesize_approx', 'fps', 'tbr'):
            value = positive(fmt.get(field))
            if field in ('width', 'height', 'filesize') and value is not None and value != int(value):
                value = None
            item[field] = round(value) if value and field not in ('fps', 'tbr') else value
        result.append(item)
    return result
