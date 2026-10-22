"""Single-file video choices; no media URLs in public data."""
import hashlib
import json
import math
import re


def positive(value):
    return value if type(value) in (int, float) and math.isfinite(value) and value > 0 else None


def normalized_fps(value):
    value = positive(value)
    if value is None:
        return None
    for target in (24, 25, 30, 48, 50, 60, 100, 120, 144, 240):
        if abs(value - target) <= max(.05, target * .0011):
            return target
    return round(value, 3)


def codec_name(value):
    if not isinstance(value, str) or not value or value == 'none':
        return None
    name = value.lower()
    for prefixes, label in ((('avc', 'h264', 'h.264'), 'h264'),
                            (('hev', 'hvc', 'h265', 'h.265'), 'h265'),
                            (('av01', 'av1'), 'av1')):
        if name.startswith(prefixes):
            return label
    return value


def format_key(fmt):
    # CDN mirrors get numeric suffixes. Signing tokens/URLs and sizes are not identities.
    name = re.sub(r'-\d+$', '', str(fmt.get('format_id') or ''))
    identity = [name, *(fmt.get(k) for k in ('ext', 'width', 'height', 'vcodec', 'acodec', 'tbr', 'fps'))]
    if fmt.get('requested_formats'):
        # Both stream identities and the actual audio language must survive re-extraction.
        # Normalize CDN mirror suffixes independently, never strip language metadata.
        identity = ['streams-v2', *[format_key(f) for f in fmt['requested_formats']]]
    if fmt.get('vcodec') == 'none':
        identity.extend(['language', fmt.get('language')])
        identity.extend([fmt.get('audio_channels'), fmt.get('asr')])
    return hashlib.sha256(json.dumps(identity, separators=(',', ':')).encode()).hexdigest()[:24]


def usable(fmt):
    return (fmt.get('protocol') in ('http', 'https') and not fmt.get('has_drm')
            and not fmt.get('watermark') and 'watermark' not in str(fmt.get('format_note', '')).lower())


def audio_rank(audio, context):
    preference = audio.get('language_preference')
    preference = preference if type(preference) in (int, float) and math.isfinite(preference) else -2
    language = audio.get('language') or ''
    preferred = context.get('original_language') or context.get('language')
    return (preference, bool(preferred and language == preferred),
            audio.get('is_default') is True or '(default)' in str(audio.get('format_note', '')).lower(),
            positive(audio.get('abr')) or positive(audio.get('tbr')) or 0,
            positive(audio.get('audio_channels')) or 0, positive(audio.get('asr')) or 0,
            language, str(audio.get('format_id') or ''))


def candidates(context, merge=False):
    formats = context.get('formats') or []
    if merge:
        expanded = []
        for video in formats:
            if video.get('acodec') != 'none' or video.get('vcodec') in (None, 'none'):
                expanded.append(video)
                continue
            audio_ext = {'mp4': 'm4a', 'webm': 'webm'}.get(video.get('ext'))
            audio_codecs = ('mp4a', 'aac', 'alac') if audio_ext == 'm4a' else ('opus', 'vorbis')
            compatible = [a for a in formats if a.get('vcodec') == 'none'
                          and a.get('acodec') not in (None, 'none') and not a.get('has_drm')
                          and usable(a) and a.get('ext') == audio_ext
                          and str(a['acodec']).lower().startswith(audio_codecs)]
            video_compatible = (video.get('ext') != 'webm' or
                                str(video.get('vcodec', '')).lower().startswith(('vp8', 'vp9', 'vp09', 'av1', 'av01')))
            audio = max(compatible, key=lambda a: audio_rank(a, context), default=None) if usable(video) and video_compatible else None
            if not audio:
                continue
            combined = {**video, 'format_id': f'{video["format_id"]}+{audio["format_id"]}',
                        'acodec': audio['acodec'], 'requested_formats': [video, audio],
                        'protocol': video.get('protocol', '') + '+' + audio['protocol'],
                        'requires_merge': True}
            for field in ('filesize', 'filesize_approx'):
                sizes = [positive(f.get(field)) if field == 'filesize' else
                         positive(f.get('filesize')) or positive(f.get('filesize_approx')) for f in (video, audio)]
                combined[field] = sum(sizes) if all(sizes) else None
            if combined['filesize'] is not None:
                combined['filesize_approx'] = None
            expanded.append(combined)
        formats = expanded
    choices = {}
    for fmt in formats:
        protocols = str(fmt.get('protocol', '')).split('+')
        if (not all(p in ('http', 'https') for p in protocols) or (len(protocols) != 1 and not merge)
                or ('_yingqu_media_type' in fmt and fmt['_yingqu_media_type'] != 'muxed')
                or fmt.get('ext') not in ('mp4', 'webm')
                or fmt.get('vcodec') in ('none', 'bytevc2') or fmt.get('has_drm')
                or 'watermark' in str(fmt.get('format_note', '')).lower()
                or fmt.get('watermark') or fmt.get('acodec') == 'none'):
            continue
        if fmt.get('acodec') == 'none' and any(f.get('acodec') != 'none' and f.get('vcodec') != 'none' for f in formats):
            continue
        # yt-dlp supplies worst -> best; keep its preferred CDN mirror within each choice.
        choices[format_key(fmt)] = fmt
    return sorted(choices.items(), key=lambda item: (
        (positive(item[1].get('width')) or 0) * (positive(item[1].get('height')) or 0),
        normalized_fps(item[1].get('fps')) or 0, positive(item[1].get('tbr')) or 0,
        positive(item[1].get('filesize')) or 0, item[0]), reverse=True)


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
        item = dict(format_id=key, container=fmt['ext'], codec=codec_name(fmt.get('vcodec')))
        if merge:
            item['requires_merge'] = bool(fmt.get('requires_merge'))
        for field in ('width', 'height', 'filesize', 'filesize_approx', 'fps', 'tbr'):
            value = positive(fmt.get(field))
            if field in ('width', 'height', 'filesize') and value is not None and value != int(value):
                value = None
            item[field] = round(value) if value and field not in ('fps', 'tbr') else value
        item['fps'] = normalized_fps(fmt.get('fps'))
        if item['filesize'] is not None:
            item['filesize_approx'] = None
        result.append(item)
    return result
