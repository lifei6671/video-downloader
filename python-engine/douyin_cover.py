"""Conservative same-picture upgrades; keep the original bytes on uncertainty."""
from io import BytesIO

from PIL import Image, ImageChops, ImageOps, ImageStat


def static_image(body):
    try:
        with Image.open(BytesIO(body), formats=('JPEG', 'PNG', 'WEBP')) as image:
            if image.width * image.height > 20_000_000 or getattr(image, 'n_frames', 1) != 1:
                return None
            image = ImageOps.exif_transpose(image)
            # Transparent pixels cannot establish the displayed background.
            if 'A' in image.getbands() or 'transparency' in image.info:
                return None
            return image.convert('RGB')
    except (OSError, ValueError, Image.DecompressionBombError):
        return None


def same_picture(reference, candidate):
    if abs((candidate.width / candidate.height) / (reference.width / reference.height) - 1) > 0.01:
        return False
    size = (128, 128)
    difference = ImageChops.difference(reference.resize(size, Image.Resampling.LANCZOS),
                                     candidate.resize(size, Image.Resampling.LANCZOS))
    # Require both close overall colors and close local regions. A perceptual
    # hash alone can accept a different frame or a changed text overlay.
    if max(ImageStat.Stat(difference).mean) > 4:
        return False
    return all(max(ImageStat.Stat(difference.crop((x, y, x + 16, y + 16))).mean) <= 10
               for y in range(0, 128, 16) for x in range(0, 128, 16))


def upgrade(original, urls, read):
    reference = static_image(original)
    if reference is None:
        return original
    best, area = original, reference.width * reference.height
    for url in urls:
        body = read(url)
        candidate = static_image(body) if body else None
        if (candidate is not None and candidate.width >= reference.width
                and candidate.height >= reference.height
                and candidate.width * candidate.height > area
                and same_picture(reference, candidate)):
            best, area = body, candidate.width * candidate.height
    return best
