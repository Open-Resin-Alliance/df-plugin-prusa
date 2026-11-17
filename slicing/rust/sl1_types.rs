//! The three machines that read the Prusa SLA container.
//!
//! They share one archive layout, one `config.ini` schema and one layer image
//! format (8-bit grayscale PNG, stored portrait); they differ in the
//! `printerModel` token the firmware matches on and in nothing the encoder
//! writes besides it. That token is what a file says it is for, so it comes from
//! the printer profile's `formatVersion` - the same value that selects the file
//! extension - and the encoder's own extension is only the fallback.

/// A machine that reads the container.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Sl1Model {
    OriginalSl1,
    Sl1s,
    Slx,
}

impl Sl1Model {
    /// The `printerModel` token the firmware matches on.
    pub(super) fn token(self) -> &'static str {
        match self {
            Self::OriginalSl1 => "SL1",
            Self::Sl1s => "SL1S",
            Self::Slx => "SLX",
        }
    }

    /// The machine a format version names, if it names one.
    ///
    /// `formatVersion` is the printer profile's own vocabulary (`sl1`, `sl1s`,
    /// `slx`) and is lower-cased here because profiles are allowed to be written
    /// by hand.
    pub(super) fn from_format_version(raw: Option<&str>) -> Option<Self> {
        match raw?.trim().to_ascii_lowercase().as_str() {
            "sl1" => Some(Self::OriginalSl1),
            "sl1s" => Some(Self::Sl1s),
            "slx" => Some(Self::Slx),
            _ => None,
        }
    }

    /// Stock material values for this machine, used when the material profile
    /// carries none.
    ///
    /// The SL1 numbers are PrusaSlicer's `*sl1_fast*` material profile at 0.05 mm
    /// and the SL1S numbers its 0.05 mm Prusament profile; a job without a material
    /// profile should still produce a file the machine can print rather than a
    /// zero-exposure one.
    pub(super) fn defaults(self) -> Sl1MaterialDefaults {
        match self {
            Self::OriginalSl1 => Sl1MaterialDefaults {
                exposure_sec: 12.0,
                initial_exposure_sec: 45.0,
            },
            // Prusa has not published SLX exposure data; the SL1S stock profile is
            // the closest published starting point and the material page lets the
            // user replace it.
            Self::Sl1s | Self::Slx => Sl1MaterialDefaults {
                exposure_sec: 2.2,
                initial_exposure_sec: 6.0,
            },
        }
    }
}

/// Exposures a machine falls back to when the job names none.
#[derive(Clone, Copy)]
pub(super) struct Sl1MaterialDefaults {
    pub exposure_sec: f64,
    pub initial_exposure_sec: f64,
}
