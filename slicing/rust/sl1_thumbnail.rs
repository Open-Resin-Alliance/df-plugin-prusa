//! The two preview images the printer's file browser shows.
//!
//! PrusaSlicer writes `thumbnail/thumbnail400x400.png` and
//! `thumbnail/thumbnail800x480.png`, both RGBA, and those are the sizes the
//! printer asks for. DragonFruit captures one export thumbnail at whatever size
//! the viewport had, so it is decoded once, fitted into each box without
//! upscaling and centred on a transparent canvas of exactly that size - a
//! picture renamed to a size it is not is how a printer ends up with no preview.
//!
//! Thumbnails are decoration: a capture that cannot be decoded leaves the archive
//! without them rather than failing the print.

use base64::Engine;

use crate::engine::SlicerV3Error;
use crate::types::SliceJobV3;

/// The boxes PrusaSlicer asks the printer for, largest last.
const THUMBNAIL_SIZES: [(u32, u32); 2] = [(400, 400), (800, 480)];

/// The captured scene thumbnail, rendered at each size the printer reads.
pub(super) fn thumbnails(job: &SliceJobV3) -> Vec<(String, Vec<u8>)> {
    let Some((src_width, src_height, src_pixels)) = decoded_capture(job) else {
        return Vec::new();
    };

    THUMBNAIL_SIZES
        .iter()
        .filter_map(|&(width, height)| {
            let canvas = fit_centered(&src_pixels, src_width, src_height, width, height);
            encode_rgba_png(width, height, &canvas)
                .ok()
                .map(|png| (format!("thumbnail/thumbnail{width}x{height}.png"), png))
        })
        .collect()
}

fn decoded_capture(job: &SliceJobV3) -> Option<(u32, u32, Vec<u8>)> {
    let encoded = job.export_thumbnail_png_base64.as_deref()?.trim();
    if encoded.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    decode_rgba(&bytes)
}

/// Nearest-neighbour scale into `box_width × box_height`, centred on a transparent
/// canvas; never upscales past the capture's own size.
fn fit_centered(
    source: &[u8],
    src_width: u32,
    src_height: u32,
    box_width: u32,
    box_height: u32,
) -> Vec<u8> {
    let mut canvas = vec![0u8; box_width as usize * box_height as usize * 4];
    if src_width == 0 || src_height == 0 {
        return canvas;
    }

    let (fit_width, fit_height) = fit(src_width, src_height, box_width, box_height);
    let offset_x = (box_width - fit_width) / 2;
    let offset_y = (box_height - fit_height) / 2;

    for y in 0..fit_height {
        let source_y = (y as u64 * src_height as u64 / fit_height as u64) as u32;
        for x in 0..fit_width {
            let source_x = (x as u64 * src_width as u64 / fit_width as u64) as u32;
            let source_index = ((source_y as usize * src_width as usize) + source_x as usize) * 4;
            let target_index = (((offset_y + y) as usize * box_width as usize)
                + (offset_x + x) as usize)
                * 4;
            canvas[target_index..target_index + 4]
                .copy_from_slice(&source[source_index..source_index + 4]);
        }
    }

    canvas
}

/// Largest size that fits the box without upscaling.
fn fit(src_width: u32, src_height: u32, box_width: u32, box_height: u32) -> (u32, u32) {
    if src_width <= box_width && src_height <= box_height {
        return (src_width, src_height);
    }
    let by_width = (u64::from(box_width) * u64::from(src_height)) / u64::from(src_width);
    if by_width <= u64::from(box_height) {
        (box_width, by_width.max(1) as u32)
    } else {
        let by_height = (u64::from(box_height) * u64::from(src_width)) / u64::from(src_height);
        (by_height.max(1) as u32, box_height)
    }
}

fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let (width, height) = (info.width, info.height);
    buffer.truncate(info.buffer_size());

    let pixels = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Grayscale => buffer
            .iter()
            .flat_map(|&value| [value, value, value, 255])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some((width, height, pixels))
}

fn encode_rgba_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, SlicerV3Error> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| SlicerV3Error::Png(error.to_string()))?;
        writer
            .write_image_data(pixels)
            .map_err(|error| SlicerV3Error::Png(error.to_string()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_capture(width: u32, height: u32) -> Vec<u8> {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            let pixels: Vec<u8> = (0..width * height)
                .flat_map(|_| [200u8, 100, 50, 255])
                .collect();
            writer.write_image_data(&pixels).expect("pixels");
        }
        png_bytes
    }

    #[test]
    fn fits_without_upscaling_and_centres() {
        let canvas = fit_centered(&[1, 2, 3, 255, 4, 5, 6, 255], 2, 1, 4, 4);
        // 2x1 inside 4x4: two pixels wide, one tall, centred at (1, 1).
        assert_eq!(&canvas[0..16], &[0u8; 16]);
        let row = (1 * 4 + 1) * 4;
        assert_eq!(&canvas[row..row + 8], &[1u8, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn thumbnails_carry_the_sizes_the_printer_reads() {
        let mut job = SliceJobV3::default();
        job.export_thumbnail_png_base64 = Some(
            base64::engine::general_purpose::STANDARD.encode(solid_capture(1600, 960)),
        );
        let entries = thumbnails(&job);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "thumbnail/thumbnail400x400.png",
                "thumbnail/thumbnail800x480.png"
            ]
        );
        for (name, png_bytes) in entries {
            let (width, height) = decode_rgba(&png_bytes)
                .map(|(width, height, _)| (width, height))
                .expect("decodable thumbnail");
            assert!(name.contains(&format!("{width}x{height}")));
        }
    }

    #[test]
    fn a_job_without_a_capture_gets_no_thumbnails() {
        assert!(thumbnails(&SliceJobV3::default()).is_empty());
    }
}
