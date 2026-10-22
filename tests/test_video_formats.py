import json

import pytest

import engine
from video_formats import candidates, format_key, public_formats, select_format


def fmt(name, width, height, size, **extra):
    return dict(format_id=name, protocol='https', ext='mp4', vcodec='h265', acodec='aac',
                width=width, height=height, filesize=size, url='https://v.douyinvod.com/?sign=private', **extra)


def test_real_resolution_wins_over_upstream_order_and_codec():
    # Upstream places an unrecognized 4k URL key before familiar 1080p variants.
    high = fmt('adapt_lowest_4_1-0', 2160, 3840, 6290708)
    low = fmt('1080p-0', 1080, 1920, 3304911)
    selected = list(select_format({'formats': [high, low]}))[0]
    assert selected['url'] == high['url'] and selected['width'] == 2160
    assert selected['_yingqu_format_id'] == format_key(high)
    choices = public_formats({'formats': [high, low]})
    assert choices[0]['filesize'] == 6290708
    assert 'private' not in json.dumps(choices) and 'url' not in choices[0]


def test_mirrors_are_one_choice_and_signing_changes_do_not_change_selection():
    low = fmt('720p-0', 720, 1280, 2000)
    mirror = {**low, 'format_id': '720p-2', 'url': 'https://v2.douyinvod.com/?sign=fresh'}
    high = fmt('2160p-0', 2160, 3840, 6000)
    assert len(candidates({'formats': [low, mirror, high]})) == 2
    chosen = list(select_format({'formats': [high, mirror]}, format_key(low)))[0]
    assert chosen['url'] == mirror['url'] and chosen['width'] == 720
    assert format_key(low) == format_key(mirror)


def test_selection_missing_or_invalid_never_downgrades():
    low = fmt('720p-0', 720, 1280, 2000)
    high = fmt('2160p-0', 2160, 3840, 6000)
    with pytest.raises(engine.EngineError, match='FORMAT_UNAVAILABLE'):
        list(select_format({'formats': [low]}, format_key(high)))
    with pytest.raises(engine.EngineError, match='ENGINE_PROTOCOL_ERROR'):
        list(select_format({'formats': [low]}, 'https://attacker.invalid/video'))


def test_watermarked_drm_and_unplayable_choices_are_not_offered():
    safe = fmt('720p-0', 720, 1280, 2000)
    choices = public_formats({'formats': [safe, {**safe, 'format_note': 'Download video, watermarked'},
        {**safe, 'has_drm': True}, {**safe, 'vcodec': 'bytevc2'}]})
    assert len(choices) == 1
    assert choices[0]['format_id'] == format_key(safe)


def test_unknown_size_remains_unknown_without_fabricated_estimates():
    value = public_formats({'formats': [fmt('720p-0', 720, 1280, None)]})[0]
    assert value['filesize'] is None and value['filesize_approx'] is None


@pytest.mark.parametrize('fps,expected', [(23.976, 24), (29.969, 30), (29.97, 30), (59.94, 60), (27, 27)])
def test_fps_display_normalization_keeps_persisted_identity(fps, expected):
    value = fmt('same-0', 1920, 1080, None, fps=fps)
    assert public_formats({'formats': [value]})[0]['fps'] == expected
    # Display normalization must not silently rewrite the existing single-stream key.
    import hashlib
    identity = ['same', 'mp4', 1920, 1080, 'h265', 'aac', None, fps]
    assert format_key(value) == hashlib.sha256(json.dumps(identity, separators=(',', ':')).encode()).hexdigest()[:24]
    assert format_key({**value, 'language': 'en'}) == format_key(value)


def test_real_codec_bitrate_and_resolution_variants_are_retained():
    base = fmt('base', 1920, 1080, 1000, fps=29.97)
    variants = [{**base, 'vcodec': codec, 'tbr': rate}
                for codec in ('avc1.640028', 'hvc1.1.6.L120', 'av01.0.08M.08') for rate in (1000, 2000)]
    variants.append(fmt('4k', 3840, 2160, 4000))
    public = public_formats({'formats': variants})
    assert len(public) == 7 and public[0]['width'] == 3840
    assert {f['codec'] for f in public} == {'h264', 'h265', 'av1'}


@pytest.mark.parametrize('size,estimate,expected', [(100, 110, (100, None)), (None, 110, (None, 110)),
    (None, None, (None, None)), (float('inf'), -1, (None, None)), (100.5, 120, (None, 120))])
def test_sizes_remain_exact_estimated_or_unknown(size, estimate, expected):
    public = public_formats({'formats': [fmt('size', 1280, 720, size, filesize_approx=estimate)]})[0]
    assert (public['filesize'], public['filesize_approx']) == expected
