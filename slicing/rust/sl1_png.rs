//! Layer images: RLE runs in, portrait 8-bit grayscale PNG out.
//!
//! The engine hands the streaming sink one layer as run-length encoded spans over
//! a landscape `width_px × height_px` frame - the same frame every other
//! DragonFruit format receives. The printers do not: they store each layer
//! portrait, `height_px × width_px`, with the plate's front-left corner at the
//! image's bottom-right (PrusaSlicer's layer rasterizer sets that up with
//! `flipXY` plus its default X mirror, which is why the profiles' `mirrorY` is
//! what makes the two machines agree). So every layer is rotated a quarter turn
//! counter-clockwise on its way into the PNG scanlines.
//!
//! The rotation is done by sweeping the source columns right to left with the
//! column's value vector kept as a run list over the source rows. A change to that
//! vector can only happen where a source span starts or ends, so the sweep patches
//! the run list at those boundaries and copies it out per column: the cost is the
//! number of span boundaries plus the size of the answer, not `width × height`
//! lookups. At 8K a layer's portrait runs take ~1 ms, against ~17 ms for reading
//! every pixel of the transposed frame, and no 33 MB frame is ever materialised.
//!
//! Two writers sit behind that sweep:
//!
//! - **Binary layers** (anti-aliasing off, every pixel 0 or 255) are re-expressed
//!   as portrait runs and handed to `crate::encode::encode_grayscale_png_from_rle`,
//!   the engine's O(runs) fixed-Huffman PNG writer, the same path Athena's layers
//!   take. At 8K a layer costs ~18 ms here against ~90 ms for a full-frame
//!   transpose, and it allocates no frame at all.
//! - **Anti-aliased layers** keep Sub-filtered libdeflate. A run encoder cannot
//!   express an edge gradient, so their scanlines are written one value at a time.

use crc32fast::Hasher as Crc32Hasher;
use libdeflater::{CompressionLvl, Compressor};

use crate::engine::SlicerV3Error;
use crate::rle::RleRun;

const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// One constant span of one landscape row.
#[derive(Clone, Copy)]
struct Span {
    start: u32,
    end: u32,
    value: u8,
}

/// Encode a layer straight from its landscape runs.
///
/// Output is `height × width` (portrait), 8-bit grayscale, non-interlaced;
/// `binary` selects the run writer over the gradient writer.
pub(super) fn encode_portrait_png_from_runs(
    width: u32,
    height: u32,
    runs: &[RleRun],
    png_compression_strategy: &str,
    binary: bool,
) -> Result<Vec<u8>, SlicerV3Error> {
    if width == 0 || height == 0 {
        return Err(SlicerV3Error::InvalidDimensions { width, height });
    }

    if binary {
        let (rows, _covered_columns) = row_spans(width as usize, height as usize, runs);
        let portrait = portrait_runs(width as usize, height as usize, &rows);
        return crate::encode::encode_grayscale_png_from_rle(
            height,
            width,
            &portrait,
            png_compression_strategy,
            true,
        );
    }

    let (rows, covered_columns) = row_spans(width as usize, height as usize, runs);
    let filtered = portrait_filtered(
        width as usize,
        height as usize,
        &rows,
        &covered_columns,
        !matches!(png_compression_strategy, "fastest"),
    );
    let compressed = zlib_compress(&filtered, png_compression_level(png_compression_strategy))?;

    let mut out = Vec::with_capacity(8 + 25 + 12 + compressed.len() + 12);
    out.extend_from_slice(&PNG_SIG);
    write_ihdr(&mut out, height, width);
    write_chunk(&mut out, b"IDAT", &compressed);
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Group the landscape runs by row and mark the columns any non-zero run covers.
fn row_spans(width: usize, height: usize, runs: &[RleRun]) -> (Vec<Vec<Span>>, Vec<bool>) {
    let mut rows: Vec<Vec<Span>> = vec![Vec::new(); height];
    let mut covered_columns = vec![false; width];
    let mut offset = 0usize;
    for run in runs {
        let mut remaining = run.length as usize;
        while remaining > 0 {
            let row = offset / width;
            if row >= height {
                break;
            }
            let column = offset % width;
            let take = remaining.min(width - column);
            rows[row].push(Span {
                start: column as u32,
                end: (column + take) as u32,
                value: run.value,
            });
            if run.value != 0 {
                covered_columns[column..column + take].fill(true);
            }
            offset += take;
            remaining -= take;
        }
    }
    (rows, covered_columns)
}

/// The portrait image's runs, by sweeping the source columns right to left.
///
/// The column vector changes only where a source span starts or ends, so it is
/// kept as a run list over the source rows and patched at those events instead of
/// being re-read row by row for every column: a sweep over 7680 columns costs the
/// number of span boundaries, not 7680 × 4320 lookups. Each column then copies
/// its (few) runs into the output, which is the size of the answer.
fn portrait_runs(width: usize, height: usize, rows: &[Vec<Span>]) -> Vec<RleRun> {
    // `events[start]` lists the rows whose covering span ends when the sweep moves
    // from column `start` to `start - 1`, with the value they fall back to.
    let mut events: Vec<Vec<(u32, u8)>> = vec![Vec::new(); width];
    for (row, spans) in rows.iter().enumerate() {
        for index in 0..spans.len() {
            let start = spans[index].start as usize;
            if start == 0 {
                continue;
            }
            let previous = if index > 0 && spans[index - 1].end as usize >= start {
                spans[index - 1].value
            } else {
                0
            };
            events[start].push((row as u32, previous));
        }
    }

    // The column at the right edge: each row's last span, if it reaches it.
    let mut values = vec![0u8; height];
    for (row, spans) in rows.iter().enumerate() {
        values[row] = match spans.last() {
            Some(span) if span.end as usize >= width => span.value,
            _ => 0,
        };
    }
    let mut runs: Vec<(u32, u32, u8)> = Vec::with_capacity(height);
    let mut start = 0u32;
    for row in 1..height {
        if values[row] != values[row - 1] {
            runs.push((start, row as u32, values[row - 1]));
            start = row as u32;
        }
    }
    runs.push((start, height as u32, values[height - 1]));

    let mut out = Vec::with_capacity(width * 2);
    for column in (0..width).rev() {
        out.extend(runs.iter().map(|&(from, to, value)| RleRun {
            length: to - from,
            value,
        }));
        if column > 0 {
            for &(row, previous) in &events[column] {
                set_column_value(&mut values, &mut runs, row, previous);
            }
        }
    }
    out
}

/// Replace one entry of the column vector, keeping the run list over rows in sync.
fn set_column_value(values: &mut [u8], runs: &mut Vec<(u32, u32, u8)>, row: u32, next: u8) {
    if values[row as usize] == next {
        return;
    }
    values[row as usize] = next;

    let index = runs.partition_point(|run| run.1 <= row);
    let (from, to, value) = runs[index];
    let mut pieces: Vec<(u32, u32, u8)> = Vec::with_capacity(3);
    if from < row {
        pieces.push((from, row, value));
    }
    pieces.push((row, row + 1, next));
    if row + 1 < to {
        pieces.push((row + 1, to, value));
    }
    runs.splice(index..=index, pieces);

    // Re-merge the window the split touched, so runs never fragment.
    let mut index = index.saturating_sub(1);
    while index + 1 < runs.len() {
        if runs[index].1 == runs[index + 1].0 && runs[index].2 == runs[index + 1].2 {
            let end = runs[index + 1].1;
            runs[index].1 = end;
            runs.remove(index + 1);
        } else {
            index += 1;
        }
    }
}

/// The portrait image's PNG scanlines: one filter byte plus the row, in order.
fn portrait_filtered(
    width: usize,
    height: usize,
    rows: &[Vec<Span>],
    covered_columns: &[bool],
    use_sub: bool,
) -> Vec<u8> {
    let row_bytes = 1 + height;
    let mut filtered = vec![0u8; row_bytes * width];
    let mut cursors: Vec<usize> = rows.iter().map(Vec::len).collect();
    let mut row_values = vec![0u8; height];
    for output_row in 0..width {
        let column = width - 1 - output_row;
        let target = &mut filtered[output_row * row_bytes..(output_row + 1) * row_bytes];
        target[0] = if use_sub { 1 } else { 0 };
        if !covered_columns[column] {
            continue;
        }

        for source_row in 0..height {
            let spans = &rows[source_row];
            let mut cursor = cursors[source_row];
            while cursor > 0 && spans[cursor - 1].start as usize > column {
                cursor -= 1;
            }
            cursors[source_row] = cursor;
            row_values[source_row] =
                if cursor > 0 && (spans[cursor - 1].end as usize) > column {
                    spans[cursor - 1].value
                } else {
                    0
                };
        }

        if use_sub {
            target[1] = row_values[0];
            for source_row in 1..height {
                target[source_row + 1] =
                    row_values[source_row].wrapping_sub(row_values[source_row - 1]);
            }
        } else {
            target[1..].copy_from_slice(&row_values);
        }
    }
    filtered
}

/// A flat pixel row stream as runs, for the fallback that receives PNG layers.
pub(super) fn runs_from_pixels(pixels: &[u8]) -> Vec<RleRun> {
    let mut runs = Vec::new();
    let Some(&first) = pixels.first() else {
        return runs;
    };
    let mut value = first;
    let mut length = 1u32;
    for &next in &pixels[1..] {
        if next == value {
            length += 1;
        } else {
            runs.push(RleRun { length, value });
            value = next;
            length = 1;
        }
    }
    runs.push(RleRun { length, value });
    runs
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
        // 4 wide x 2 tall landscape, one run per pixel, all distinct.
        let runs: Vec<RleRun> = (1..=8)
            .map(|value| RleRun { length: 1, value })
            .collect();
        let png = encode_portrait_png_from_runs(4, 2, &runs, "balanced", false).expect("encode");
        let (width, height, pixels) = decode_grayscale(&png);
        assert_eq!((width, height), (2, 4));
        // A quarter turn CCW: the right-hand column becomes the top row.
        assert_eq!(pixels, vec![4, 8, 3, 7, 2, 6, 1, 5]);
    }

    #[test]
    fn binary_layers_take_the_run_writer_and_rotate_the_same_way() {
        // Landscape: [0, 255, 0, 255] over [255, 0, 255, 0].
        let runs = [
            RleRun { length: 1, value: 0 },
            RleRun { length: 1, value: 255 },
            RleRun { length: 1, value: 0 },
            RleRun { length: 2, value: 255 },
            RleRun { length: 1, value: 0 },
            RleRun { length: 1, value: 255 },
            RleRun { length: 1, value: 0 },
        ];
        let png = encode_portrait_png_from_runs(4, 2, &runs, "balanced", true).expect("encode");
        let (width, height, pixels) = decode_grayscale(&png);
        assert_eq!((width, height), (2, 4));
        assert_eq!(pixels, vec![255, 0, 0, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn the_two_writers_agree_on_a_binary_layer() {
        let runs = [
            RleRun { length: 2, value: 0 },
            RleRun { length: 2, value: 255 },
            RleRun { length: 1, value: 255 },
            RleRun { length: 3, value: 0 },
        ];
        let binary = encode_portrait_png_from_runs(4, 2, &runs, "balanced", true).expect("binary");
        let gradient = encode_portrait_png_from_runs(4, 2, &runs, "balanced", false).expect("gradient");
        assert_eq!(decode_grayscale(&binary), decode_grayscale(&gradient));
    }

    #[test]
    fn an_uncovered_column_is_emitted_black_without_a_walk() {
        // Row 0: [10, 10, 200, 200]; row 1: [10, 200, 200, 200].
        let runs = [
            RleRun { length: 2, value: 10 },
            RleRun { length: 2, value: 200 },
            RleRun { length: 1, value: 10 },
            RleRun { length: 3, value: 200 },
        ];
        let png = encode_portrait_png_from_runs(4, 2, &runs, "balanced", false).expect("encode");
        let (width, height, pixels) = decode_grayscale(&png);
        assert_eq!((width, height), (2, 4));
        assert_eq!(pixels, vec![200, 200, 200, 200, 10, 200, 10, 10]);

        // A layer with no non-zero run is all black, whatever its size.
        let png = encode_portrait_png_from_runs(8, 4, &[RleRun { length: 32, value: 0 }], "balanced", true)
            .expect("empty");
        let (_, _, pixels) = decode_grayscale(&png);
        assert!(pixels.iter().all(|&value| value == 0));
    }

    #[test]
    fn a_short_stream_leaves_the_rest_black() {
        // One run covers the first row only; the second row must come out black.
        let runs = [RleRun { length: 4, value: 255 }];
        let png = encode_portrait_png_from_runs(4, 2, &runs, "balanced", true).expect("encode");
        let (_, _, pixels) = decode_grayscale(&png);
        assert_eq!(pixels, vec![255, 0, 255, 0, 255, 0, 255, 0]);
    }

    #[test]
    fn runs_from_pixels_matches_the_stream_the_engine_sends() {
        let runs = runs_from_pixels(&[0, 0, 5, 5, 5, 255]);
        assert_eq!(runs.len(), 3);
        assert_eq!((runs[0].length, runs[0].value), (2, 0));
        assert_eq!((runs[1].length, runs[1].value), (3, 5));
        assert_eq!((runs[2].length, runs[2].value), (1, 255));
    }

    #[test]
    fn rejects_a_zero_sized_frame() {
        let error = encode_portrait_png_from_runs(0, 4, &[], "balanced", true).expect_err("empty");
        assert!(matches!(error, SlicerV3Error::InvalidDimensions { .. }));
    }
}
