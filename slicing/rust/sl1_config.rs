//! `config.ini`: the print settings the printer reads before it looks at a layer.
//!
//! PrusaSlicer writes this file as a flat `key = value` map (its own writer is
//! `std::map`, so the keys come out sorted) and the firmware matches on the names
//! exactly. The set below is the one PrusaSlicer and UVtools both write, minus the
//! fields that describe their own UI:
//!
//! - `printerModel` decides which machine the job is for; it is the only place the
//!   three models differ.
//! - `expTime` / `expTimeFirst` / `numFade` are the normal exposure, the initial
//!   exposure and how many layers fade between them.
//! - `layerHeight`, `jobDir` and `action` are the layer step, the name the layer
//!   images are prefixed with, and `print`.
//! - `numFast` / `numSlow` classify layers for the printer's time estimate;
//!   DragonFruit does not model the tilt, so every layer is reported as fast and
//!   the firmware's own motion profile decides how long it takes.
//! - `usedMaterial` is the cured volume in millilitres, derived from each layer's
//!   measured solid area rather than from the mesh, so supports and rafts are
//!   included.
//!
//! `config.json` (the tilt/exposure profile PrusaSlicer also writes) is
//! deliberately absent: it describes per-machine motion curves this plugin does
//! not model, and UVtools has shipped SL1S files without it for years. A made-up
//! curve would change how the vat tilts, which is not a guess worth making.

use std::path::Path;

use serde_json::Value;
use time::OffsetDateTime;

use crate::types::{LayerAreaStatsV3, SliceJobV3};

use super::sl1_types::Sl1Model;

/// What one `config.ini` says.
pub(super) struct Sl1Config {
    pub job_dir: String,
    pub layer_height_mm: f64,
    pub exposure_sec: f64,
    pub initial_exposure_sec: f64,
    /// Layers over which the exposure fades from the initial to the normal value.
    pub faded_layers: u32,
    /// `expUserProfile`: 0 fast, 1 slow, 2 custom.
    pub exp_user_profile: u32,
    pub total_layers: u32,
    /// Layers the firmware runs its fast tilt profile on.
    pub fast_layers: u32,
    /// Layers it runs the slow profile on.
    pub slow_layers: u32,
    pub used_material_ml: f64,
    pub material_name: String,
    pub slicer_version: String,
    pub timestamp: String,
}

impl Sl1Config {
    /// Read the job's settings for `model`.
    ///
    /// `area_stats` is the engine's per-layer solid area, which decides both the
    /// cured volume and the fast/slow layer split the printer's time estimate uses.
    pub(super) fn from_job(
        job: &SliceJobV3,
        model: Sl1Model,
        job_dir: &str,
        area_stats: &[LayerAreaStatsV3],
    ) -> Self {
        let metadata: Value = serde_json::from_str(&job.metadata_json).unwrap_or(Value::Null);
        let defaults = model.defaults();
        let layer_height_mm = number_at(&metadata, &["material", "layerHeightMm"])
            .unwrap_or(f64::from(job.layer_height_mm));
        let exposure_sec = number_at(&metadata, &["material", "normalExposureSec"])
            .unwrap_or(defaults.exposure_sec);
        let padded_digits = number_at(&metadata, &["material", "bottomExposureSec"])
            .unwrap_or(defaults.initial_exposure_sec);
        let faded_layers = number_at(&metadata, &["material", "bottomLayerCount"])
            .map(|value| value.round().clamp(0.0, 20.0) as u32)
            .unwrap_or(10);
        let exp_user_profile = match text_at(&metadata, &["prusa", "expUserProfile"])
            .or_else(|| {
                number_at(&metadata, &["prusa", "expUserProfile"])
                    .map(|value| value.round().to_string())
            })
            .as_deref()
        {
            Some("1") => 1,
            Some("2") => 2,
            _ => 0,
        };

        let (fast_layers, slow_layers) = fast_slow_split(job, faded_layers, area_stats);
        let surface_area_mm2: f64 = area_stats.iter().map(|stats| stats.total_solid_area_mm2).sum();
        let used_material_ml = surface_area_mm2 * layer_height_mm / 1000.0;

        Self {
            job_dir: job_dir.to_string(),
            layer_height_mm,
            exposure_sec,
            initial_exposure_sec: padded_digits,
            faded_layers,
            exp_user_profile,
            total_layers: job.total_layers,
            fast_layers,
            slow_layers,
            used_material_ml,
            material_name: text_at(&metadata, &["material", "name"]).unwrap_or_default(),
            slicer_version: text_at(&metadata, &["slicer", "version"])
                .filter(|version| !version.is_empty())
                .map(|version| format!("DragonFruit {version}"))
                .unwrap_or_else(|| "DragonFruit".to_string()),
            timestamp: timestamp(OffsetDateTime::now_utc()),
        }
    }

    /// The file as the printer reads it: sorted keys, one per line, `key = value`.
    pub(super) fn render(&self, model: Sl1Model) -> String {
        let mut lines: Vec<(String, String)> = vec![
            ("action".into(), "print".into()),
            ("expTime".into(), number(self.exposure_sec)),
            ("expTimeFirst".into(), number(self.initial_exposure_sec)),
            ("expUserProfile".into(), self.exp_user_profile.to_string()),
            ("fileCreationTimestamp".into(), self.timestamp.clone()),
            ("hollow".into(), "0".into()),
            ("jobDir".into(), self.job_dir.clone()),
            ("layerHeight".into(), number(self.layer_height_mm)),
            ("numFade".into(), self.faded_layers.to_string()),
            ("numFast".into(), self.fast_layers.to_string()),
            ("numSlow".into(), self.slow_layers.to_string()),
            ("printTime".into(), "0".into()),
            ("printerModel".into(), model.token().into()),
            ("printerVariant".into(), "default".into()),
            ("prusaSlicerVersion".into(), self.slicer_version.clone()),
            ("usedMaterial".into(), decimal(self.used_material_ml, 6)),
        ];
        if !self.material_name.is_empty() {
            lines.push(("materialName".into(), self.material_name.clone()));
        }
        lines.sort_by(|left, right| left.0.cmp(&right.0));

        let mut out = String::with_capacity(lines.len() * 32);
        for (key, value) in lines {
            out.push_str(&key);
            out.push_str(" = ");
            out.push_str(&value);
            out.push('\n');
        }
        out
    }
}

impl Sl1Config {
    /// `config.json`: the same values as `config.ini`, typed, plus the exposure
    /// profile PrusaSlicer writes alongside it.
    ///
    /// The SLX is the machine whose firmware reads this file; the SL1 and SL1S run
    /// without it (UVtools has written accepted SL1S files for years with only
    /// `config.ini`), so this is only emitted for the SLX.
    pub(super) fn render_json(&self, model: Sl1Model) -> String {
        let mut material_name = Value::Null;
        if !self.material_name.is_empty() {
            material_name = Value::String(self.material_name.clone());
        }
        let value = serde_json::json!({
            "action": "print",
            "expTime": self.exposure_sec,
            "expTimeFirst": self.initial_exposure_sec,
            "expUserProfile": self.exp_user_profile,
            "fileCreationTimestamp": self.timestamp,
            "hollow": 0,
            "jobDir": self.job_dir,
            "layerHeight": self.layer_height_mm,
            "materialName": material_name,
            "numFade": self.faded_layers,
            "numFast": self.fast_layers,
            "numSlow": self.slow_layers,
            "printTime": 0,
            "printerModel": model.token(),
            "printerVariant": "default",
            "prusaSlicerVersion": self.slicer_version,
            "usedMaterial": round_to(self.used_material_ml, 6),
            "version": 2,
            "exposure_profile": exposure_profile(),
        });
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    }
}

/// PrusaSlicer's fast/slow layer rule (`SLAPrintSteps::create_stats`): the first
/// `faded_layers + 3` layers are always fast, and a later layer is fast when its
/// area is at most 35 % of the display. The firmware uses the split for its time
/// estimate, which is why the two counts add up to the layer count.
fn fast_slow_split(
    job: &SliceJobV3,
    faded_layers: u32,
    area_stats: &[LayerAreaStatsV3],
) -> (u32, u32) {
    if area_stats.is_empty() {
        return (job.total_layers, 0);
    }
    let display_area_mm2 =
        f64::from(job.build_width_mm) * f64::from(job.build_depth_mm) * AREA_FILL_FRACTION;
    let always_fast = faded_layers + 3;
    let mut fast = 0u32;
    let mut slow = 0u32;
    for (index, stats) in area_stats.iter().enumerate() {
        if (index as u32) < always_fast || stats.total_solid_area_mm2 <= display_area_mm2 {
            fast += 1;
        } else {
            slow += 1;
        }
    }
    (fast, slow)
}

/// The material's area-fill threshold (`area_fill`), 35 % in every Prusa SLA preset.
const AREA_FILL_FRACTION: f64 = 0.35;

/// The tilt/exposure profile the SLX reads from `config.json`.
///
/// Lifted from a job PrusaSlicer 2.9.6 wrote for the SLX, because Prusa has not
/// published the SLX printer profile these values come from. They describe the
/// machine's peel, not the resin, so they are the same for every print; the
/// exposures in `config.ini` are what the user's material page controls.
fn exposure_profile() -> Value {
    serde_json::json!({
        "area_fill": 35,
        "printing_temperature": null,
        "below_area_fill": tilt_profile(7000),
        "above_area_fill": tilt_profile(10000),
    })
}

fn tilt_profile(delay_before_exposure_ms: u32) -> Value {
    serde_json::json!({
        "delay_before_exposure_ms": delay_before_exposure_ms,
        "delay_after_exposure_ms": 0,
        "delay_to_reflood_ms": 0,
        "tower_hop_height_nm": 0,
        "tower_profile": "layer22",
        "use_tilt": true,
        "tilt_down_initial_profile": "layer7360",
        "tilt_down_offset_steps": 0,
        "tilt_down_offset_delay_ms": 0,
        "tilt_down_finish_profile": "layer7360",
        "tilt_down_cycles": 1,
        "tilt_down_delay_ms": 0,
        "tilt_up_initial_profile": "layer7360",
        "tilt_up_offset_steps": 600,
        "tilt_up_offset_delay_ms": 0,
        "tilt_up_finish_profile": "layer7360",
        "tilt_up_cycles": 1,
        "tilt_up_delay_ms": 0,
        "dynamic_delay_before_profile": "neg0_2",
        "dynamic_delay_before_timeout_ms": 50000,
        "dynamic_tilt_down_profile": "disabled",
        "dynamic_tilt_up_profile": "disabled",
    })
}

/// The name the printer shows and the one every layer image is prefixed with.
///
/// The layer entries are `<jobDir>00000.png`, so the name ends up inside the
/// archive and on the printer's filesystem: keep it to ASCII letters, digits,
/// `-` and `_`, and fall back to `print` rather than writing an empty or
/// path-like name.
pub(super) fn job_dir_name(output_path: Option<&Path>) -> String {
    let stem = output_path
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .unwrap_or("print");
    let mut sanitized: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    sanitized.truncate(64);
    if sanitized.is_empty() || sanitized.chars().all(|character| character == '_') {
        "print".to_string()
    } else {
        sanitized
    }
}

fn timestamp(now: OffsetDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02} at {:02}:{:02}:{:02} UTC",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// Fixed-point with trailing zeros removed, the same shape PrusaSlicer writes
/// (`0.05`, `2.2`, `45`).
fn number(value: f64) -> String {
    decimal(value, 6)
}

fn round_to(value: f64, places: u32) -> f64 {
    let factor = 10f64.powi(places as i32);
    (value * factor).round() / factor
}

fn decimal(value: f64, places: usize) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut rendered = format!("{value:.places$}");
    if rendered.contains('.') {
        while rendered.ends_with('0') {
            rendered.pop();
        }
        if rendered.ends_with('.') {
            rendered.pop();
        }
    }
    rendered
}

fn value_at<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn number_at(root: &Value, path: &[&str]) -> Option<f64> {
    value_at(root, path).and_then(Value::as_f64)
}

fn text_at(root: &Value, path: &[&str]) -> Option<String> {
    match value_at(root, path)? {
        Value::String(text) => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(metadata: &str) -> SliceJobV3 {
        SliceJobV3 {
            source_width_px: 2560,
            source_height_px: 1620,
            build_width_mm: 128.0,
            build_depth_mm: 81.0,
            layer_height_mm: 0.05,
            total_layers: 100,
            metadata_json: metadata.to_string(),
            ..SliceJobV3::default()
        }
    }

    #[test]
    fn renders_the_keys_the_firmware_reads() {
        let job = job(
            r#"{"material":{"layerHeightMm":0.025,"normalExposureSec":2.5,
                "bottomExposureSec":7,"bottomLayerCount":12,"name":"Prusament Model Grey"},
                "prusa":{"expUserProfile":"1"},"slicer":{"version":"0.1.15"}}"#,
        );
        let config = Sl1Config::from_job(&job, Sl1Model::Sl1s, "bench", &[]);
        let ini = config.render(Sl1Model::Sl1s);

        assert!(ini.contains("layerHeight = 0.025\n"), "{ini}");
        assert!(ini.contains("expTime = 2.5\n"), "{ini}");
        assert!(ini.contains("expTimeFirst = 7\n"), "{ini}");
        assert!(ini.contains("numFade = 12\n"), "{ini}");
        assert!(ini.contains("numFast = 100\n"), "{ini}");
        assert!(ini.contains("printerModel = SL1S\n"), "{ini}");
        assert!(ini.contains("printerVariant = default\n"), "{ini}");
        assert!(ini.contains("expUserProfile = 1\n"), "{ini}");
        assert!(ini.contains("jobDir = bench\n"), "{ini}");
        assert!(ini.contains("action = print\n"), "{ini}");
        assert!(ini.contains("materialName = Prusament Model Grey\n"), "{ini}");
        assert!(ini.contains("prusaSlicerVersion = DragonFruit 0.1.15\n"), "{ini}");
    }

    #[test]
    fn falls_back_to_the_models_stock_exposures() {
        let job = job("{}");
        let sl1 = Sl1Config::from_job(&job, Sl1Model::OriginalSl1, "print", &[]).render(Sl1Model::OriginalSl1);
        assert!(sl1.contains("expTime = 12\n"), "{sl1}");
        assert!(sl1.contains("expTimeFirst = 45\n"), "{sl1}");
        assert!(sl1.contains("printerModel = SL1\n"), "{sl1}");

        let slx = Sl1Config::from_job(&job, Sl1Model::Slx, "print", &[]).render(Sl1Model::Slx);
        assert!(slx.contains("expTime = 2.2\n"), "{slx}");
        assert!(slx.contains("printerModel = SLX\n"), "{slx}");
    }

    #[test]
    fn caps_the_fade_at_the_printers_limit_and_never_exceeds_the_layer_count() {
        let job = job(r#"{"material":{"bottomLayerCount":40}}"#);
        let config = Sl1Config::from_job(&job, Sl1Model::Sl1s, "print", &[]);
        let ini = config.render(Sl1Model::Sl1s);
        assert!(ini.contains("numFade = 20\n"), "{ini}");
        assert!(ini.contains("numFast = 100\n"), "{ini}");
    }

    #[test]
    fn used_material_is_the_cured_volume() {
        // One full 128x81 mm plate at 0.05 mm is 128*81*0.05 mm3 = 518.4 mm3 = 0.5184 ml.
        let job = job("{}");
        let stats = [LayerAreaStatsV3 {
            total_solid_area_mm2: 128.0 * 81.0,
            ..LayerAreaStatsV3::default()
        }];
        let config = Sl1Config::from_job(&job, Sl1Model::Sl1s, "print", &stats);
        let ini = config.render(Sl1Model::Sl1s);
        assert!(ini.contains("usedMaterial = 0.5184\n"), "{ini}");
    }

    #[test]
    fn the_fast_slow_split_follows_prusaslicers_area_rule() {
        let stats_for = |area_mm2: f64, layers: usize| -> Vec<LayerAreaStatsV3> {
            (0..layers)
                .map(|_| LayerAreaStatsV3 {
                    total_solid_area_mm2: area_mm2,
                    ..LayerAreaStatsV3::default()
                })
                .collect()
        };
        let render = |stats: Vec<LayerAreaStatsV3>, layers: u32| {
            Sl1Config::from_job(
                &SliceJobV3 {
                    total_layers: layers,
                    build_width_mm: 220.0,
                    build_depth_mm: 125.0,
                    ..job("{\"material\":{\"bottomLayerCount\":1}}")
                },
                Sl1Model::Slx,
                "print",
                &stats,
            )
            .render(Sl1Model::Slx)
        };

        // Layers inside `faded + 3` are fast whatever their area.
        let all_fast = render(stats_for(20_000.0, 4), 4);
        assert!(all_fast.contains("numFast = 4\n"), "{all_fast}");
        assert!(all_fast.contains("numSlow = 0\n"), "{all_fast}");

        // Past that window, 20 000 mm2 is over 35 % of 220 x 125 mm and 2 000 is not.
        let split = render(stats_for(20_000.0, 8), 8);
        assert!(split.contains("numFast = 4\n"), "{split}");
        assert!(split.contains("numSlow = 4\n"), "{split}");
        let light = render(stats_for(2_000.0, 8), 8);
        assert!(light.contains("numFast = 8\n"), "{light}");
        assert!(light.contains("numSlow = 0\n"), "{light}");
    }

    #[test]
    fn the_slx_config_json_carries_the_exposure_profile() {
        let job = job("{}");
        let stats = [LayerAreaStatsV3 {
            total_solid_area_mm2: 100.0,
            ..LayerAreaStatsV3::default()
        }];
        let config = Sl1Config::from_job(&job, Sl1Model::Slx, "print", &stats);
        let json: serde_json::Value =
            serde_json::from_str(&config.render_json(Sl1Model::Slx)).expect("valid json");
        assert_eq!(json["printerModel"], "SLX");
        assert_eq!(json["version"], 2);
        assert_eq!(json["exposure_profile"]["area_fill"], 35);
        assert_eq!(
            json["exposure_profile"]["below_area_fill"]["tilt_up_offset_steps"],
            600
        );
        // PrusaSlicer's `surface_area` is the mesh's surface, which an encoder
        // does not have; writing the layer-area sum there would be a different
        // quantity under the same name.
        assert!(json.get("surface_area").is_none());
    }

    #[test]
    fn job_dir_is_ascii_and_never_empty() {
        assert_eq!(job_dir_name(Some(Path::new("/tmp/Bench part.sl1s"))), "Bench_part");
        assert_eq!(job_dir_name(Some(Path::new("/tmp/打印.sl1"))), "print");
        assert_eq!(job_dir_name(None), "print");
    }
}
