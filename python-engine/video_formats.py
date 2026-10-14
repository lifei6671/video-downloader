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


def candidates(context):
    formats = context.get('formats') or []
    separate_audio = any(f.get('vcodec') == 'none' and f.get('acodec') != 'none' for f in formats)
    choices = {}
    for fmt in formats:
        if (fmt.get('protocol') not in ('http', 'https') or fmt.get('ext') not in ('mp4', 'webm')
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


def select_format(context, selected=None):
    from engine import EngineError
    choices = candidates(context)
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


def public_formats(info):
    result = []
    for key, fmt in candidates(info):
        item = dict(format_id=key, container=fmt['ext'], codec=fmt.get('vcodec'))
        for field in ('width', 'height', 'filesize', 'filesize_approx', 'fps', 'tbr'):
            value = positive(fmt.get(field))
            if field in ('width', 'height', 'filesize') and value is not None and value != int(value):
                value = None
            item[field] = round(value) if value and field not in ('fps', 'tbr') else value
        result.append(item)
    return result
