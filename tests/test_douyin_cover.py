from io import BytesIO

import pytest
from PIL import Image, ImageDraw

import douyin_cover
import engine
from douyin import cover_candidates


def picture():
    image = Image.new('RGB', (640, 960), '#426b87')
    draw = ImageDraw.Draw(image)
    draw.rectangle((40, 80, 410, 330), fill='#ebc783')
    draw.ellipse((220, 320, 600, 800), fill='#723a45')
    draw.line((0, 900, 640, 400), fill='white', width=18)
    return image


def encode(image, size=None, fmt='JPEG', **kwargs):
    out = BytesIO()
    if size:
        image = image.resize(size, Image.Resampling.LANCZOS)
    image.save(out, format=fmt, **kwargs)
    return out.getvalue()


def test_largest_matching_static_image_preserves_original_bytes():
    image = picture()
    original = encode(image, (160, 240), quality=82)
    medium = encode(image, (320, 480), quality=90)
    large = encode(image, fmt='WEBP', quality=93)
    other = encode(Image.new('RGB', image.size, 'red'))
    choices = dict(medium=medium, large=large, other=other, small=original)
    assert douyin_cover.upgrade(original, choices, choices.get) == large


@pytest.mark.parametrize('kind', ['different', 'text', 'crop', 'animated', 'transparent', 'invalid', 'missing', 'smaller'])
def test_uncertain_candidates_keep_original(kind):
    image = picture()
    original = encode(image, (160, 240))
    if kind == 'different':
        image = image.transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    elif kind == 'text':
        ImageDraw.Draw(image).text((100, 180), 'OTHER COVER', fill='black', font_size=35)
    elif kind == 'crop':
        image = image.crop((30, 30, 610, 900)).resize(image.size)
    elif kind == 'transparent':
        image = image.convert('RGBA')
    if kind == 'animated':
        candidate = encode(image, fmt='WEBP', save_all=True, append_images=[Image.new('RGB', image.size, 'red')], duration=100)
    elif kind == 'transparent':
        candidate = encode(image, fmt='PNG')
    elif kind == 'invalid':
        candidate = b'\xff\xd8\xffcorrupt'
    elif kind == 'missing':
        candidate = None
    elif kind == 'smaller':
        candidate = encode(image, (80, 120))
    else:
        candidate = encode(image)
    assert douyin_cover.upgrade(original, ['candidate'], lambda _: candidate) == original


def test_undecodable_reference_never_requests_candidates():
    original = b'\xff\xd8\xfflegacy'
    def forbidden(_):
        raise AssertionError('cannot compare with an undecodable reference')
    assert douyin_cover.upgrade(original, ['candidate'], forbidden) == original


def test_oversized_decoded_image_is_rejected(monkeypatch):
    # Small encoded file, large declared dimensions: reject before allocating pixels.
    class Header:
        width, height = 5000, 5000
        def __enter__(self): return self
        def __exit__(self, *_): pass
    monkeypatch.setattr(Image, 'open', lambda *args, **kwargs: Header())
    assert douyin_cover.static_image(b'header') is None


def test_candidates_are_bounded_deduplicated_and_exclude_blurred_cover():
    assert cover_candidates({'video': {
        'cover_original_scale': {'url_list': ['https://p.douyinpic.com/base']},
        'dynamic_cover': {'url_list': ['http://insecure', 'https://p.douyinpic.com/hd', 'https://p.douyinpic.com/mirror']},
        'origin_cover': {'url_list': ['https://p.douyinpic.com/base']},
        'gaussian_cover': {'url_list': ['https://p.douyinpic.com/blurred']},
        'cover': {'url_list': [None]},
    }}) == ['https://p.douyinpic.com/base', 'https://p.douyinpic.com/hd']


@pytest.mark.parametrize('failure', [False, True])
def test_download_integration_and_candidate_failure_fallback(tmp_path, failure):
    image = picture()
    original, hd = encode(image, (160, 240)), encode(image)
    calls = []
    class Ydl:
        def urlopen(self, request):
            calls.append(request)
            if isinstance(request, str):
                assert request == 'https://p.douyinpic.com/base'
                return BytesIO(original)
            assert request.extensions['timeout'] == 5
            if failure:
                raise engine.RequestError('candidate unavailable')
            return BytesIO(hd)
    info = {'thumbnail': 'https://p.douyinpic.com/base',
            '_yingqu_douyin_covers': ['https://p.douyinpic.com/base', 'https://p.douyinpic.com/hd']}
    result = engine.download_cover(Ydl(), info, tmp_path)
    expected = original if failure else hd
    assert (tmp_path / 'cover').read_bytes() == expected
    assert result == {'cover_filename': 'cover', 'cover_bytes': len(expected)}
    assert len(calls) == 2


def test_other_platform_without_candidates_keeps_existing_choice(tmp_path):
    original = encode(picture(), (160, 240))
    class Ydl:
        def urlopen(self, url):
            assert url == 'https://p.byteimg.com/existing'
            return BytesIO(original)
    assert engine.download_cover(Ydl(), {'thumbnail': 'https://p.byteimg.com/existing'}, tmp_path)['cover_bytes'] == len(original)
    assert (tmp_path / 'cover').read_bytes() == original
