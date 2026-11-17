//! Layer images: RLE runs in, portrait 8-bit grayscale PNG out.
//!
//! The engine hands the streaming sink one layer as run-length encoded spans over
//! a landscape `width_px × height_px` frame - the same frame every other
//! DragonFruit format receives. The printers do not: they store each layer
//! portrait, `height_px × width_px`, with the plate's front-left corner at the
//! image's bottom-right (PrusaSlicer's layer rasterizer sets that up with
//! `flipXY` plus its default X mirror, which is why the profiles' `mirrorY` is
//! what makes the two machines agree). So every layer is expanded once and
//! rotated a quarter turn counter-clockwise while its PNG scanlines are built.
//!
//! The rotation is folded into the filter pass: no portrait pixel buffer is ever
//! materialised, only the filtered scanlines that are about to be deflated. At
//! 8K that is the difference between one 33 MB buffer per worker and two. The PNG
//! assembly below mirrors `crate::encode`'s grayscale writer - signature, IHDR,
//! one IDAT, IEND, Sub filtering unless the job asked for `fastest` - because the
//! engine's helper writes rows in source order and cannot express the rotation.
//!
//! `crc32fast` and `libdeflater` are engine dependencies, so this module adds no
//! crate to the build.

use crc32fast::Hasher as Crc32Hasher;
use libdeflater::{CompressionLvl, Compressor};

use crate::engine::SlicerV3Error;
use crate::rle::RleRun;

const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Expand a layer's runs into its `width * height` landscape pixels.
///
/// Runs arrive row-major over the whole frame and never span more than one layer.
/// A short stream leaves the remaining pixels black, which is what an
/// under-delivered layer should print as - the engine refuses a missing layer, so
/// this is only the tail of the last row.
pub(super) fn expand_runs(width: u32, height: u32, runs: &[RleRun]) -> Vec<u8> {
    let total = width as usize * height as usize;
    let mut pixels = vec![0u8; total];
    let mut offset = 0usize;
    for run in runs {
        if offset >= total {
            break;
        }
        let end = (offset + run.length as usize).min(total);
        pixels[offset..end].fill(run.value);
        offset = end;
    }
    pixels
}

/// Encode one landscape layer whose pixels are already expanded.
///
/// Output is `height × width` (portrait), 8-bit grayscale, non-interlaced.
pub(super) fn encode_portrait_png(
    width: u32,
    height: u32,
    landscape: &[u8],
    png_compression_strategy: &str,
) -> Result<Vec<u8>, SlicerV3Error> {
    if width == 0 || height == 0 {
        return Err(SlicerV3Error::InvalidDimensions { width, height });
    }
    let expected = width as usize * height as usize;
    if landscape.len() != expected {
        return Err(SlicerV3Error::Png(format!(
            "layer buffer is {} bytes, expected {expected} for {width}x{height}",
            landscape.len()
        )));
    }

    let out_width = height as usize;
    let out_height = width as usize;
    let use_sub = !matches!(png_compression_strategy, "fastest");
    let row_bytes = 1 + out_width;
    let mut filtered = vec![0u8; row_bytes * out_height];

    // new[row = r][col = c] = landscape[row = c][col = width - 1 - r]: a quarter
    // turn counter-clockwise. The read runs down a column of the source, so each
    // output row is one strided walk.
    for r in 0..out_height {
        let source_column = width as usize - 1 - r;
        let row = &mut filtered[r * row_bytes..(r + 1) * row_bytes];
        row[0] = if use_sub { 1 } else { 0 };
        if use_sub {
            let mut previous = 0u8;
            for c in 0..out_width {
                let value = landscape[c * width as usize + source_column];
                row[c + 1] = value.wrapping_sub(previous);
                previous = value;
            }
        } else {
            for c in 0..out_width {
                row[c + 1] = landscape[c * width as usize + source_column];
            }
        }
    }

    let compressed = zlib_compress(&filtered, png_compression_level(png_compression_strategy))?;
    let mut out = Vec::with_capacity(8 + 25 + 12 + compressed.len() + 12);
    out.extend_from_slice(&PNG_SIG);
    write_ihdr(&mut out, out_width as u32, out_height as u32);
    write_chunk(&mut out, b"IDAT", &compressed);
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Same ladder `crate::encode` uses for the other formats' layers.
fn png_compression_level(strategy: &str) -> CompressionLvl {
    let level = match strategy {
        "fastest" => 1,
        "balanced" => 3,
        "smallest" => 6,
        "optimal" => 9,
        _ => 1,
    };
    CompressionLvl::new(level).unwrap_or(CompressionLvl::best())
}

fn zlib_compress(data: &[u8], level: CompressionLvl) -> Result<Vec<u8>, SlicerV3Error> {
    let mut compressor = Compressor::new(level);
    let bound = compressor.zlib_compress_bound(data.len());
    let mut buffer = vec![0u8; bound];
    let written = compressor
        .zlib_compress(data, &mut buffer)
        .map_err(|error| SlicerV3Error::Png(error.to_string()))?;
    buffer.truncate(written);
    Ok(buffer)
}

fn chunk_crc32(kind: &[u8; 4], data: &[u8]) -> u32 {
    let mut hasher = Crc32Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    hasher.finalize()
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&chunk_crc32(kind, data).to_be_bytes());
}

fn write_ihdr(out: &mut Vec<u8>, width: u32, height: u32) {
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 0; // grayscale
    ihdr[10] = 0; // deflate
    ihdr[11] = 0; // adaptive filtering
    ihdr[12] = 0; // no interlace
    write_chunk(out, b"IHDR", &ihdr);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_grayscale(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(png_bytes);
        let mut reader = decoder.read_info().expect("png header");
        let mut buffer = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).expect("png frame");
        assert_eq!(info.color_type, png::ColorType::Grayscale);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        buffer.truncate(info.buffer_size());
        (info.width, info.height, buffer)
    }

    #[test]
    fn rotates_a_landscape_layer_counter_clockwise() {
        // 4 wide x 2 tall landscape, each value distinct.
        let landscape = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let png = encode_portrait_png(4, 2, &landscape, "balanced").expect("encode");
        let (width, height, pixels) = decode_grayscale(&png);
        assert_eq!((width, height), (2, 4));
        // A quarter turn CCW: the right-hand column becomes the top row.
        assert_eq!(pixels, vec![4, 8, 3, 7, 2, 6, 1, 5]);
    }

    #[test]
    fn expansive_runs_fill_the_frame_and_leave_the_tail_black() {
        let pixels = expand_runs(
            3,
            2,
            &[
                RleRun { length: 2, value: 255 },
                RleRun { length: 1, value: 17 },
            ],
        );
        assert_eq!(pixels, vec![255, 255, 17, 0, 0, 0]);
    }

    #[test]
    fn rejects_a_buffer_that_does_not_match_the_frame() {
        let error = encode_portrait_png(4, 2, &[0u8; 7], "balanced").expect_err("short buffer");
        assert!(matches!(error, SlicerV3Error::Png(_)));
    }

    #[test]
    fn fastest_strategy_skips_sub_filtering_but_still_round_trips() {
        let landscape = vec![0, 255, 0, 255, 255, 0, 255, 0];
        let png = encode_portrait_png(4, 2, &landscape, "fastest").expect("encode");
        let (width, height, pixels) = decode_grayscale(&png);
        assert_eq!((width, height), (2, 4));
        assert_eq!(pixels, vec![255, 0, 0, 255, 255, 0, 0, 255]);
    }
}
