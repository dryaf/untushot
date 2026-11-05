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

/// Four-connected bucket fill, comparing every pixel to the seed color.
/// Tolerance is a percentage of the maximum premultiplied RGBA channel distance.
/// A scanline work list avoids recursion
/// and keeps large blank areas cheap. The returned layer contains only filled pixels.
pub fn flood_fill(
    source: &Pixbuf,
    x: usize,
    y: usize,
    color: [u8; 4],
    tolerance: u8,
) -> Result<Option<(Pixbuf, crate::geometry::Rect)>> {
    let (w, h) = (source.width() as usize, source.height() as usize);
    if x >= w || y >= h {
        return Ok(None);
    }
    let bytes = source.read_pixel_bytes();
    let channels = source.n_channels() as usize;
    let stride = source.rowstride() as usize;
    let pixel = |x: usize, y: usize| {
        let offset = y * stride + x * channels;
        let p = &bytes.as_ref()[offset..offset + channels];
        let alpha = if channels == 4 { p[3] } else { 255 };
        if alpha == 0 {
            [0; 4]
        } else {
            [p[0], p[1], p[2], alpha]
        }
    };
    let target = pixel(x, y);
    if target == color && tolerance == 0 {
        return Ok(None);
    }
    let premultiply = |p: [u8; 4]| {
        let a = u16::from(p[3]);
        [
            ((u16::from(p[0]) * a + 127) / 255) as u8,
            ((u16::from(p[1]) * a + 127) / 255) as u8,
            ((u16::from(p[2]) * a + 127) / 255) as u8,
            p[3],
        ]
    };
    let threshold = (u16::from(tolerance.min(100)) * 255 / 100) as u8;
    let seed = premultiply(target);
    let matches = |x, y| {
        let p = pixel(x, y);
        if tolerance == 0 {
            p == target
        } else {
            premultiply(p)
                .iter()
                .zip(seed)
                .all(|(a, b)| a.abs_diff(b) <= threshold)
        }
    };
    let mut changed = false;
    let mut filled = vec![false; w * h];
    let mut pending = vec![(x, y)];
    let (mut left, mut top, mut right, mut bottom) = (x, y, x, y);
    while let Some((x, y)) = pending.pop() {
        if filled[y * w + x] {
            continue;
        }
        let mut start = x;
        while start > 0 && !filled[y * w + start - 1] && matches(start - 1, y) {
            start -= 1;
        }
        let mut end = x;
        while end + 1 < w && !filled[y * w + end + 1] && matches(end + 1, y) {
            end += 1;
        }
        filled[y * w + start..=y * w + end].fill(true);
        changed |= (start..=end).any(|column| pixel(column, y) != color);
        left = left.min(start);
        right = right.max(end);
        top = top.min(y);
        bottom = bottom.max(y);
        for adjacent in [y.checked_sub(1), (y + 1 < h).then_some(y + 1)]
            .into_iter()
            .flatten()
        {
            let mut run = false;
            for column in start..=end {
                let matched = !filled[adjacent * w + column] && matches(column, adjacent);
                if matched && !run {
                    pending.push((column, adjacent));
                }
                run = matched;
            }
        }
    }
    if !changed {
        return Ok(None);
    }
    let width = right - left + 1;
    let height = bottom - top + 1;
    let image = image::RgbaImage::from_fn(width as u32, height as u32, |x, y| {
        image::Rgba(if filled[(top + y as usize) * w + left + x as usize] {
            color
        } else {
            [0; 4]
        })
    });
    Ok(Some((
        from_rgba(image)?,
        crate::geometry::Rect::new(left as f64, top as f64, width as f64, height as f64),
    )))
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
    fn tolerance_is_seed_relative_and_includes_the_threshold() {
        let source = from_rgba(image::RgbaImage::from_fn(6, 1, |x, _| {
            image::Rgba(
                [
                    [100, 100, 100, 255],
                    [125, 100, 100, 255],
                    [126, 100, 100, 255],
                    [140, 100, 100, 255],
                    [100, 100, 100, 255],
                    [0, 0, 0, 0],
                ][x as usize],
            )
        }))
        .unwrap();
        let red = [255, 0, 0, 255];
        for (tolerance, width) in [(0, 1.), (10, 2.), (11, 3.), (100, 6.)] {
            let (_, bounds) = flood_fill(&source, 0, 0, red, tolerance).unwrap().unwrap();
            assert_eq!(bounds.w, width);
        }
        // Equal seed/replacement must still recolor similar neighbors.
        let (_, bounds) = flood_fill(&source, 0, 0, [100, 100, 100, 255], 10)
            .unwrap()
            .unwrap();
        assert_eq!(bounds.w, 2.);
        let same = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 10, 10).unwrap();
        same.fill(0xff0000ff);
        assert!(flood_fill(&same, 0, 0, red, 100).unwrap().is_none());
    }

    #[test]
    fn tolerance_handles_alpha_and_invisible_rgb() {
        let source = from_rgba(image::RgbaImage::from_fn(4, 1, |x, _| {
            image::Rgba(
                [
                    [255, 20, 30, 0],
                    [0, 255, 0, 0],
                    [255, 255, 255, 25],
                    [0, 0, 0, 255],
                ][x as usize],
            )
        }))
        .unwrap();
        for (tolerance, width) in [(0, 2.), (9, 2.), (10, 3.), (99, 3.), (100, 4.)] {
            let (_, bounds) = flood_fill(&source, 0, 0, [255, 0, 0, 255], tolerance)
                .unwrap()
                .unwrap();
            assert_eq!(bounds.w, width);
        }
    }

    #[test]
    fn bucket_fill_respects_boundaries_diagonals_alpha_and_row_padding() {
        let mut source = image::RgbaImage::from_pixel(7, 7, image::Rgba([255, 255, 255, 255]));
        for y in 0..7 {
            source.put_pixel(3, y, image::Rgba([0, 0, 0, 255]));
        }
        source.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        let source = from_rgba(source).unwrap();
        let (fill, bounds) = flood_fill(&source, 1, 1, [255, 0, 0, 255], 0)
            .unwrap()
            .unwrap();
        assert_eq!(bounds, crate::geometry::Rect::new(0., 0., 3., 7.));
        assert_eq!(&fill.read_pixel_bytes().as_ref()[..4], &[0, 0, 0, 0]);
        assert_eq!(&fill.read_pixel_bytes().as_ref()[4..8], &[255, 0, 0, 255]);
        assert!(flood_fill(&source, 7, 0, [0; 4], 0).unwrap().is_none());
        let source = from_rgba(image::RgbaImage::from_fn(2, 2, |x, y| {
            image::Rgba(if x == y {
                [123, 44, 66, 0]
            } else {
                [0, 0, 0, 128]
            })
        }))
        .unwrap();
        let (_, bounds) = flood_fill(&source, 0, 0, [1, 2, 3, 255], 0)
            .unwrap()
            .unwrap();
        assert_eq!(bounds, crate::geometry::Rect::new(0., 0., 1., 1.));
        // RGB pixbuf rows may be padded; the last row and column must still fill.
        let source = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 301, 200).unwrap();
        source.fill(0xffffffff);
        let (fill, bounds) = flood_fill(&source, 300, 199, [0, 0, 0, 255], 0)
            .unwrap()
            .unwrap();
        assert_eq!(bounds, crate::geometry::Rect::new(0., 0., 301., 200.));
        assert_eq!(fill.read_pixel_bytes().as_ref().len(), 301 * 200 * 4);
    }

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
