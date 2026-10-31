//! Image codecs are pure Rust: loading/saving does not need a desktop service.
use anyhow::{Context, Result};
use gdk_pixbuf::Pixbuf;
use image::ImageEncoder;
use std::{
    io::{BufRead, Seek, Write},
    path::Path,
};

fn decode(reader: impl BufRead + Seek) -> Result<Pixbuf> {
    let rgba = image::io::Reader::new(reader)
        .with_guessed_format()?
        .decode()?
        .into_rgba8();
    from_rgba(rgba)
}

pub fn from_rgba(rgba: image::RgbaImage) -> Result<Pixbuf> {
    let (width, height) = rgba.dimensions();
    crate::i18n::ensure!(
        width > 0 && height > 0 && width <= i32::MAX as u32 / 4 && height <= i32::MAX as u32,
        "Ungültige Bildgröße"
    );
    Ok(Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba.into_raw()),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        width as i32,
        height as i32,
        width as i32 * 4,
    ))
}

/// Clipboard transfers are decoded on a worker. Check dimensions before any
/// decompression/allocation, including the Cairo limit used by the editor.
pub fn decode_import(bytes: &[u8]) -> Result<image::RgbaImage> {
    decode_bounded(bytes, 32 * 1024 * 1024, 40_000_000)
}

pub fn decode_layer(bytes: &[u8]) -> Result<image::RgbaImage> {
    decode_bounded(bytes, 128 * 1024 * 1024, 64_000_000)
}

fn decode_bounded(bytes: &[u8], max_bytes: usize, max_pixels: u64) -> Result<image::RgbaImage> {
    crate::i18n::ensure!(
        bytes.len() <= max_bytes,
        "Die Bilddatei ist zu groß (maximal {} MiB)",
        max_bytes / 1024 / 1024
    );
    let reader = || image::io::Reader::new(std::io::Cursor::new(bytes)).with_guessed_format();
    let (width, height) = reader()?
        .into_dimensions()
        .context(crate::i18n::tr("Kein lesbares PNG- oder JPEG-Bild"))?;
    crate::i18n::ensure!(width > 0 && height > 0 && width <= 32767 && height <= 32767
        && u64::from(width) * u64::from(height) <= max_pixels,
        "Das Bild überschreitet die Importgrenze: maximal {} Millionen Pixel und 32.767 Pixel je Seite.", max_pixels / 1_000_000);
    Ok(reader()?.decode()?.into_rgba8())
}

pub fn open(path: &Path) -> Result<Pixbuf> {
    decode(std::io::BufReader::new(std::fs::File::open(path)?)).with_context(|| {
        crate::i18n::format!("Bild konnte nicht geöffnet werden: {}", path.display())
    })
}

pub fn from_bytes(bytes: &[u8]) -> Result<Pixbuf> {
    decode(std::io::Cursor::new(bytes))
}

pub fn write_png(image: &Pixbuf, writer: impl Write) -> Result<()> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let channels = image.n_channels() as usize;
    let stride = image.rowstride() as usize;
    let data = image.read_pixel_bytes();
    let mut packed = Vec::with_capacity(width * height * channels);
    for y in 0..height {
        packed.extend_from_slice(&data.as_ref()[y * stride..y * stride + width * channels]);
    }
    image::codecs::png::PngEncoder::new(writer).write_image(
        &packed,
        width as u32,
        height as u32,
        if image.has_alpha() {
            image::ColorType::Rgba8
        } else {
            image::ColorType::Rgb8
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_preserves_long_page_footer_and_alpha() {
        let rgba = image::RgbaImage::from_fn(32, 2400, |_, y| {
            image::Rgba(if y < 2390 {
                [240, 245, 250, 255]
            } else {
                [20, 80, 180, 128]
            })
        });
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&rgba, 32, 2400, image::ColorType::Rgba8)
            .unwrap();
        let decoded = decode_import(&png).unwrap();
        assert_eq!(decoded.dimensions(), (32, 2400));
        assert_eq!(decoded.get_pixel(31, 2399).0, [20, 80, 180, 128]);
        assert!(decode_import(b"https://example.invalid/not-an-image").is_err());
        assert!(decode_import(&png[..png.len() / 2]).is_err());
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpg)
            .encode(&[220, 80, 20], 1, 1, image::ColorType::Rgb8)
            .unwrap();
        assert_eq!(decode_import(&jpg).unwrap().dimensions(), (1, 1));
    }

    #[test]
    fn import_rejects_oversized_headers_before_decompression() {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[0, 0, 0, 255], 1, 1, image::ColorType::Rgba8)
            .unwrap();
        for (w, h) in [(1_u32, 32768_u32), (10000, 10000)] {
            let mut large = png.clone();
            large[16..20].copy_from_slice(&w.to_be_bytes());
            large[20..24].copy_from_slice(&h.to_be_bytes());
            let mut crc = !0_u32;
            for byte in &large[12..29] {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(crc & 1));
                }
            }
            large[29..33].copy_from_slice(&(!crc).to_be_bytes());
            assert_eq!(decode_import(&large).unwrap_err().to_string(), crate::i18n::format!(
                "Das Bild überschreitet die Importgrenze: maximal {} Millionen Pixel und 32.767 Pixel je Seite.", 40));
        }
        assert!(decode_import(&vec![0; 32 * 1024 * 1024 + 1])
            .unwrap_err()
            .to_string()
            .contains("32 MiB"));
    }
}
