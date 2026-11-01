//! Telegram preview derivatives. Original cover bytes remain in the backup unchanged.
use anyhow::{Context, Result};

pub(crate) const MAX_BYTES: usize = 10 * 1024 * 1024;

fn decode(body: &[u8]) -> Result<image::DynamicImage> {
    anyhow::ensure!(
        !body.is_empty() && body.len() <= MAX_BYTES,
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

#[cfg(test)]
mod tests {
    use super::*;
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
