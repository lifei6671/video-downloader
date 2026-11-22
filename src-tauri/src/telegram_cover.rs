//! Telegram preview derivatives. Original cover bytes remain in the backup unchanged.
use anyhow::{Context, Result};

pub(crate) const MAX_BYTES: usize = 10 * 1024 * 1024;

fn decode(body: &[u8]) -> Result<image::DynamicImage> {
    decode_limited(body, MAX_BYTES)
}
fn decode_limited(body: &[u8], max_bytes: usize) -> Result<image::DynamicImage> {
    anyhow::ensure!(
        !body.is_empty() && body.len() <= max_bytes,
        "COVER_UNAVAILABLE"
    );
    let mut reader = image::ImageReader::new(std::io::Cursor::new(body))
        .with_guessed_format()
        .context("COVER_UNAVAILABLE")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().context("COVER_UNAVAILABLE")
}

pub(crate) fn valid(body: &[u8]) -> bool {
    decode(body).is_ok()
}

/// Telegram thumbnails use a small JPEG; the full original is backed up separately.
pub(crate) fn thumbnail(body: &[u8]) -> Result<Vec<u8>> {
    let image = decode(body)?.thumbnail(320, 320).to_rgb8();
    let mut output = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 85)
        .encode_image(&image)
        .context("COVER_UNAVAILABLE")?;
    anyhow::ensure!(output.len() < 200 * 1024, "COVER_UNAVAILABLE");
    Ok(output)
}

/// Full-size Telegram photo preview; does not alter the original backup bytes.
pub(crate) fn photo(body: &[u8]) -> Result<Vec<u8>> {
    let original = decode_limited(body, 32 * 1024 * 1024)?;
    let mut image = original
        .thumbnail(original.width().min(2560), original.height().min(2560))
        .to_rgb8();
    // Native Telegram photos require an aspect ratio of at most 20:1.
    let width = image.width().max(image.height().div_ceil(20));
    let height = image.height().max(image.width().div_ceil(20));
    if (width, height) != (image.width(), image.height()) {
        let mut canvas = image::RgbImage::from_pixel(width, height, image::Rgb([255, 255, 255]));
        image::imageops::overlay(
            &mut canvas,
            &image,
            i64::from((width - image.width()) / 2),
            i64::from((height - image.height()) / 2),
        );
        image = canvas;
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90)
        .encode_image(&image)
        .context("COVER_UNAVAILABLE")?;
    anyhow::ensure!(bytes.len() <= MAX_BYTES, "COVER_UNAVAILABLE");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn album_photo_pads_long_and_wide_images_for_native_photo_limits() {
        for (width, height) in [(100, 3000), (3000, 100)] {
            let original = image::RgbImage::from_pixel(width, height, image::Rgb([0, 0, 0]));
            let mut body = std::io::Cursor::new(vec![]);
            original
                .write_to(&mut body, image::ImageFormat::Png)
                .unwrap();
            let preview = photo(body.get_ref()).unwrap();
            let decoded = image::load_from_memory(&preview).unwrap().to_rgb8();
            assert_eq!(
                (decoded.width(), decoded.height()),
                if height > width {
                    (128, 2560)
                } else {
                    (2560, 128)
                }
            );
            assert!(
                decoded.width() <= decoded.height() * 20
                    && decoded.height() <= decoded.width() * 20
            );
            assert!(decoded.get_pixel(decoded.width() / 2, decoded.height() / 2)[0] < 10);
            assert!(
                decoded.get_pixel(0, 0)[0] > 245,
                "padding preserves the whole image without cropping"
            );
        }
    }
    #[test]
    fn album_photo_previews_are_native_jpegs_without_changing_png_webp_or_jpeg_originals() {
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::WebP,
            image::ImageFormat::Jpeg,
        ] {
            let image = image::RgbImage::from_fn(512, 384, |x, y| {
                image::Rgb([(x % 251) as u8, (y % 251) as u8, 128])
            });
            let mut encoded = std::io::Cursor::new(vec![]);
            image.write_to(&mut encoded, format).unwrap();
            let original = encoded.into_inner();
            let before = original.clone();
            let jpeg = photo(&original).unwrap();
            assert_eq!(original, before);
            assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));
            let decoded = image::load_from_memory(&jpeg).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height()),
                (512, 384),
                "not a 320px thumbnail"
            );
        }
    }
    #[test]
    fn preview_is_a_small_jpeg_and_original_bytes_are_unchanged() {
        let original = image::RgbImage::from_fn(1600, 900, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 251) as u8, 90])
        });
        let mut body = std::io::Cursor::new(Vec::new());
        original
            .write_to(&mut body, image::ImageFormat::Png)
            .unwrap();
        let body = body.into_inner();
        let before = body.clone();
        let jpeg = thumbnail(&body).unwrap();
        assert_eq!(body, before);
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]) && jpeg.len() < 200 * 1024);
        let preview = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((preview.width(), preview.height()), (320, 180));
        assert!(valid(&body));
        assert!(!valid(b"\x89PNG\r\n\x1a\ninvalid"));
    }
}
