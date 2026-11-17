//! DragonFruit's encoder for the Prusa SLA container: `.sl1`, `.sl1s`, `.slx`.
//!
//! PrusaSlicer writes one ZIP for all three machines (`store_sl1` in
//! `Slic3r/Biz/ResultExport/SLA/SL1.cpp`): `config.ini`, one 8-bit grayscale PNG
//! per layer named `<jobDir>NNNNN.png` (zero-based, five digits), and two
//! `thumbnail/` previews. What a file is *for* the firmware is the
//! `printerModel` in `config.ini`, so the machine comes from the printer profile's
//! `formatVersion` and the file extension follows it.
//!
//! The encoder takes the engine's RLE streaming path (`requires_png_layers() ==
//! false` plus a `RleStreamEncoder`): layers arrive as runs, each worker expands
//! one layer into its landscape frame, rotates it into the portrait image the
//! printer stores while building the PNG scanlines, and keeps only the encoded
//! PNG. Nothing ever holds every layer's pixels, which is what makes an 8K SLX job
//! the same shape as a 4K SL1 job.
//!
//! Registration is by folder convention: `scripts/generate-plugin-registry.mjs`
//! finds `pluginDefinition.ts`, `#[path]`-includes this file into the engine
//! crate, and the extension list in `slicing/formats.json` is what the format
//! definition advertises.

mod sl1_config;
mod sl1_png;
mod sl1_thumbnail;
mod sl1_types;

use std::io::{Cursor, Seek, Write};
use std::path::Path;
use std::sync::Arc;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::encoders::{FormatEncoder, RleStreamEncoder};
use crate::engine::SlicerV3Error;
use crate::rle::RleRun;
use crate::types::{LayerAreaStatsV3, RenderedLayersV3, SliceJobV3};

use self::sl1_config::{job_dir_name, Sl1Config};
use self::sl1_png::{encode_portrait_png, expand_runs};
use self::sl1_types::Sl1Model;

/// One encoder per extension the container is saved under.
///
/// They share the sink and the archive writer; only the default model differs, for
/// callers that hand the engine a format hint instead of a full printer profile
/// (the offline `encode` path). The app always reaches the `.sl1` encoder and names
/// the machine through `formatVersion`.
pub fn create_plugin_encoder() -> Vec<Box<dyn FormatEncoder>> {
    vec![
        Box::new(Sl1Encoder {
            extension: ".sl1",
            default_model: Sl1Model::OriginalSl1,
        }),
        Box::new(Sl1Encoder {
            extension: ".sl1s",
            default_model: Sl1Model::Sl1s,
        }),
        Box::new(Sl1Encoder {
            extension: ".slx",
            default_model: Sl1Model::Slx,
        }),
    ]
}

struct Sl1Encoder {
    extension: &'static str,
    default_model: Sl1Model,
}

impl Sl1Encoder {
    fn model_for(&self, job: &SliceJobV3) -> Sl1Model {
        Sl1Model::from_format_version(job.format_version.as_deref()).unwrap_or(self.default_model)
    }
}

/// The print as it accumulates: one encoded PNG per layer plus the per-layer areas.
struct Sl1RleStreamEncoder {
    job: SliceJobV3,
    model: Sl1Model,
    layers: Vec<Vec<u8>>,
    area_stats: Vec<LayerAreaStatsV3>,
}

impl Sl1RleStreamEncoder {
    fn new(job: &SliceJobV3, model: Sl1Model) -> Self {
        Self {
            job: job.clone(),
            model,
            layers: vec![Vec::new(); job.total_layers as usize],
            area_stats: Vec::new(),
        }
    }
}

/// One layer: runs in, portrait PNG out.
fn encode_layer(job: &SliceJobV3, runs: &[RleRun]) -> Result<Vec<u8>, SlicerV3Error> {
    let width = job.effective_render_width_px();
    let height = job.source_height_px;
    let landscape = expand_runs(width, height, runs);
    encode_portrait_png(width, height, &landscape, &job.png_compression_strategy)
}

impl RleStreamEncoder for Sl1RleStreamEncoder {
    fn consume_rle_layer(
        &mut self,
        layer_index: u32,
        runs: Vec<crate::rle::RleRun>,
    ) -> Result<(), SlicerV3Error> {
        // Encode now so the runs and the layer buffer are dropped immediately.
        let png = encode_layer(&self.job, &runs)?;
        self.layers[layer_index as usize] = png;
        Ok(())
    }

    fn set_area_stats(&mut self, stats: Vec<LayerAreaStatsV3>) {
        self.area_stats = stats;
    }

    fn parallel_encode_fn(
        &self,
    ) -> Option<
        Arc<dyn Fn(u32, &[crate::rle::RleRun]) -> Result<Vec<u8>, SlicerV3Error> + Send + Sync>,
    > {
        let job = self.job.clone();
        Some(Arc::new(move |_layer_index: u32, runs: &[RleRun]| {
            encode_layer(&job, runs)
        }))
    }

    fn store_encoded_layer(&mut self, layer_index: u32, bytes: Vec<u8>) {
        self.layers[layer_index as usize] = bytes;
    }

    fn finalize_to_bytes(self: Box<Self>) -> Result<Vec<u8>, SlicerV3Error> {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        write_archive(
            &mut cursor,
            &self.job,
            self.model,
            &job_dir_name(None),
            &self.layers,
            &self.area_stats,
        )?;
        Ok(cursor.into_inner())
    }

    fn finalize_to_path(self: Box<Self>, output_path: &Path) -> Result<(), SlicerV3Error> {
        // The layer entries are named after the job, so the archive needs the
        // destination's stem; `finalize_to_bytes` has no path and uses `print`.
        let job_dir = job_dir_name(Some(output_path));
        let file = std::fs::File::create(output_path)?;
        let mut writer = std::io::BufWriter::new(file);
        write_archive(
            &mut writer,
            &self.job,
            self.model,
            &job_dir,
            &self.layers,
            &self.area_stats,
        )
    }
}

/// The archive itself: config, layers, previews.
fn write_archive<W: Write + Seek>(
    writer: W,
    job: &SliceJobV3,
    model: Sl1Model,
    job_dir: &str,
    layers: &[Vec<u8>],
    area_stats: &[LayerAreaStatsV3],
) -> Result<(), SlicerV3Error> {
    if layers.len() != job.total_layers as usize {
        return Err(SlicerV3Error::MissingRenderedLayerPayload(format!(
            "{} of {} layers were encoded",
            layers.len(),
            job.total_layers
        )));
    }
    if let Some(missing) = layers.iter().position(Vec::is_empty) {
        return Err(SlicerV3Error::MissingRenderedLayerPayload(format!(
            "layer {missing} was never encoded"
        )));
    }

    // PrusaSlicer's own statistics: the cured area of every layer, times the layer
    // step. Supports and rafts are part of the mask, so they are counted.
    let cured_area_mm2: f64 = area_stats
        .iter()
        .map(|stats| stats.total_solid_area_mm2)
        .sum();
    let config = Sl1Config::from_job(job, model, job_dir, cured_area_mm2);

    let mut zip = ZipWriter::new(writer);
    // Layer PNGs are already deflated; a second deflate pass only costs time.
    let layer_options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let text_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(i64::from(job.container_compression_level.min(9))));

    zip.start_file("config.ini", text_options)?;
    zip.write_all(config.render(model).as_bytes())?;

    for (index, png) in layers.iter().enumerate() {
        zip.start_file(format!("{job_dir}{index:05}.png"), layer_options)?;
        zip.write_all(png)?;
    }

    for (name, png) in sl1_thumbnail::thumbnails(job) {
        zip.start_file(name, layer_options)?;
        zip.write_all(&png)?;
    }

    zip.finish()?;
    Ok(())
}

/// Decode a layer PNG the engine rendered outside the streaming path.
///
/// Only the 8-bit grayscale layers DragonFruit produces are accepted; anything
/// else is refused rather than silently converted, because a wrong bit depth is a
/// wrong cure time.
fn decode_grayscale_layer(
    png_bytes: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, SlicerV3Error> {
    let decoder = png::Decoder::new(png_bytes);
    let mut reader = decoder
        .read_info()
        .map_err(|error| SlicerV3Error::Png(error.to_string()))?;
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| SlicerV3Error::Png(error.to_string()))?;
    if info.color_type != png::ColorType::Grayscale || info.bit_depth != png::BitDepth::Eight {
        return Err(SlicerV3Error::Png(format!(
            "expected an 8-bit grayscale layer, got {:?} {:?}",
            info.color_type, info.bit_depth
        )));
    }
    if info.width != width || info.height != height {
        return Err(SlicerV3Error::InvalidDimensions {
            width: info.width,
            height: info.height,
        });
    }
    pixels.truncate(info.buffer_size());
    Ok(pixels)
}

impl FormatEncoder for Sl1Encoder {
    fn output_format(&self) -> &'static str {
        self.extension
    }

    /// `usedMaterial` in `config.ini` is the cured area of every layer, which is
    /// the one number the engine has to measure rather than read from a profile.
    fn requires_area_stats(&self) -> bool {
        true
    }

    fn requires_png_layers(&self) -> bool {
        false
    }

    fn create_rle_stream_encoder(
        &self,
        job: &SliceJobV3,
    ) -> Result<Option<Box<dyn RleStreamEncoder>>, SlicerV3Error> {
        Ok(Some(Box::new(Sl1RleStreamEncoder::new(job, self.model_for(job)))))
    }

    fn estimate_encode_progress_units(&self, _rendered_layers: &RenderedLayersV3) -> u32 {
        1
    }

    /// Fallback for callers that hand over already-rendered PNG layers.
    fn encode_container(
        &self,
        job: &SliceJobV3,
        layer_pngs: &[Vec<u8>],
        layer_area_stats: &[LayerAreaStatsV3],
    ) -> Result<Vec<u8>, SlicerV3Error> {
        let width = job.effective_render_width_px();
        let height = job.source_height_px;
        let mut layers = Vec::with_capacity(layer_pngs.len());
        for png_bytes in layer_pngs {
            let landscape = decode_grayscale_layer(png_bytes, width, height)?;
            layers.push(encode_portrait_png(
                width,
                height,
                &landscape,
                &job.png_compression_strategy,
            )?);
        }

        let mut cursor = Cursor::new(Vec::<u8>::new());
        write_archive(
            &mut cursor,
            job,
            self.model_for(job),
            &job_dir_name(None),
            &layers,
            layer_area_stats,
        )?;
        Ok(cursor.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn job(total_layers: u32, format_version: &str) -> SliceJobV3 {
        SliceJobV3 {
            output_format: ".sl1".to_string(),
            format_version: Some(format_version.to_string()),
            source_width_px: 8,
            source_height_px: 4,
            width_px: 8,
            height_px: 4,
            build_width_mm: 128.0,
            build_depth_mm: 81.0,
            layer_height_mm: 0.05,
            total_layers,
            png_compression_strategy: "balanced".to_string(),
            metadata_json: r#"{"material":{"normalExposureSec":2.2,"bottomExposureSec":6}}"#
                .to_string(),
            ..SliceJobV3::default()
        }
    }

    fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip archive");
        let mut found = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("entry");
            let name = entry.name().expect("entry name").to_string();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).expect("entry data");
            found.push((name, data));
        }
        found
    }

    #[test]
    fn writes_config_layers_and_names_the_model() {
        let job = job(2, "sl1");
        let encoder = Sl1Encoder {
            extension: ".sl1",
            default_model: Sl1Model::OriginalSl1,
        };
        let png = encode_portrait_png(8, 4, &[0u8; 32], "balanced").expect("layer");
        let bytes = encoder
            .encode_container(&job, &[png.clone(), png], &[])
            .expect("archive");

        let entries = entries(&bytes);
        assert_eq!(
            entries.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
            vec!["config.ini", "print00000.png", "print00001.png"]
        );
        let config = String::from_utf8(entries[0].1.clone()).expect("config text");
        assert!(config.contains("printerModel = SL1\n"), "{config}");
        assert!(config.contains("jobDir = print\n"), "{config}");
        assert!(config.contains("expTime = 2.2\n"), "{config}");
        assert!(config.contains("action = print\n"), "{config}");
    }

    #[test]
    fn format_version_picks_the_model_and_the_extension_picks_it_when_absent() {
        let sl1s_job = job(1, "sl1s");
        let encoder = Sl1Encoder {
            extension: ".sl1",
            default_model: Sl1Model::OriginalSl1,
        };
        assert_eq!(encoder.model_for(&sl1s_job), Sl1Model::Sl1s);

        let mut no_version = job(1, "sl1");
        no_version.format_version = None;
        let slx_encoder = Sl1Encoder {
            extension: ".slx",
            default_model: Sl1Model::Slx,
        };
        assert_eq!(slx_encoder.model_for(&no_version), Sl1Model::Slx);
    }

    #[test]
    fn streaming_sink_rotates_layers_and_keeps_them_in_order() {
        let job = job(2, "sl1s");
        let encoder = Sl1Encoder {
            extension: ".sl1s",
            default_model: Sl1Model::Sl1s,
        };
        let mut sink = encoder
            .create_rle_stream_encoder(&job)
            .expect("sink")
            .expect("streaming sink");
        for index in 0..2 {
            sink.consume_rle_layer(
                index,
                vec![RleRun {
                    length: 32,
                    value: 255,
                }],
            )
            .expect("layer");
        }
        let bytes = sink.finalize_to_bytes().expect("archive");
        let entries = entries(&bytes);
        let config = String::from_utf8(entries[0].1.clone()).expect("config");
        assert!(config.contains("printerModel = SL1S\n"), "{config}");

        let (width, height) = {
            let decoder = png::Decoder::new(entries[1].1.as_slice());
            let reader = decoder.read_info().expect("layer png");
            let info = reader.info();
            (info.width, info.height)
        };
        assert_eq!((width, height), (4, 8), "layers are stored portrait");
    }

    #[test]
    fn a_missing_layer_fails_instead_of_shipping_an_empty_print() {
        let job = job(2, "sl1");
        let encoder = Sl1Encoder {
            extension: ".sl1",
            default_model: Sl1Model::OriginalSl1,
        };
        let bytes = encoder
            .encode_container(&job, &[encode_portrait_png(8, 4, &[0u8; 32], "balanced").unwrap()], &[]);
        assert!(matches!(
            bytes,
            Err(SlicerV3Error::MissingRenderedLayerPayload(_))
        ));
    }
}
