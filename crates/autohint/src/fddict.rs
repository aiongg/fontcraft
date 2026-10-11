//! The zones and stems the autohinter works against: what a font gives ([`ZoneSet`],
//! [`StemSet`]) and the validated form the hinter reads ([`FdDict`]).
//!
//! Ported from otfautohint's `fdTools.py` (`FDDict`, `buildBlueLists`; AFDKO 5.0.1, Apache-2.0,
//! Copyright 2021 Adobe; see `NOTICE`). The types are fixed by M10.1; M10.13 fills
//! [`FdDict::new`].

use crate::{AutohintError, MAX_ZONES};

/// One alignment zone as a font gives it. A bottom zone (`is_bottom`) has its flat edge at
/// `top` and its overshoot below; a top zone the reverse.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zone {
    pub bottom: f64,
    pub top: f64,
    pub is_bottom: bool,
    /// The zone's name in reports (`BaselineYCoord`, `CapHeight`…, or the metric's name).
    pub name: String,
}

/// The alignment zones of a master or override set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneSet {
    /// BlueValues: the baseline zone and the top zones.
    pub blue_values: Vec<Zone>,
    /// OtherBlues: bottom zones below the baseline.
    pub other_blues: Vec<Zone>,
    /// FamilyBlues and FamilyOtherBlues.
    pub family_blues: Vec<Zone>,
    pub family_other_blues: Vec<Zone>,
    /// BlueFuzz: zones closer than `2 × fuzz + 1` are refused.
    pub blue_fuzz: f64,
}

/// The standard stems of a master or override set (StemSnapH / StemSnapV, else StdHW / StdVW;
/// otfautohint's `DominantH` / `DominantV`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StemSet {
    pub h: Vec<f64>,
    pub v: Vec<f64>,
}

/// A validated zone (`BlueValue`: top, bottom, key name, dictionary name, bottom flag).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlueValue {
    pub top: f64,
    pub bottom: f64,
    pub name: String,
    pub dict_name: String,
    pub is_bottom: bool,
}

/// The settings the hinter reads for a glyph (`FDDict`).
#[derive(Clone, Debug, PartialEq)]
pub struct FdDict {
    /// `DictName`: the override set's name, or the font's.
    pub dict_name: String,
    /// `FontName`: the master's name, for reports on several masters.
    pub font_name: Option<String>,
    /// `BlueValuesPairs` and `OtherBlueValuesPairs`, sorted.
    pub blue_pairs: Vec<BlueValue>,
    pub other_blue_pairs: Vec<BlueValue>,
    /// `FamilyValuesPairs` and `OtherFamilyValuesPairs`.
    pub family_pairs: Vec<BlueValue>,
    pub other_family_pairs: Vec<BlueValue>,
    pub blue_fuzz: f64,
    /// `DominantH`, `DominantV`.
    pub dominant_h: Vec<f64>,
    pub dominant_v: Vec<f64>,
    /// `FlexOK`.
    pub flex_ok: bool,
    /// `LanguageGroup` 1 (CJK).
    pub language_group: u8,
    /// `OrigEmSqUnits`.
    pub em_units: Option<f64>,
    /// `VCounterChars`, `HCounterChars`: counter-hinted glyphs of this set, beyond the options'.
    pub v_counter_chars: Vec<String>,
    pub h_counter_chars: Vec<String>,
    /// The baseline zone (`BaselineYCoord`, `BaselineOvershoot`), which otfautohint requires
    /// unless told otherwise.
    pub baseline: Option<(f64, f64)>,
}

impl Default for FdDict {
    fn default() -> Self {
        Self {
            dict_name: String::new(),
            font_name: None,
            blue_pairs: Vec::new(),
            other_blue_pairs: Vec::new(),
            family_pairs: Vec::new(),
            other_family_pairs: Vec::new(),
            blue_fuzz: 0.0,
            dominant_h: Vec::new(),
            dominant_v: Vec::new(),
            flex_ok: true,
            language_group: 0,
            em_units: None,
            v_counter_chars: Vec::new(),
            h_counter_chars: Vec::new(),
            baseline: None,
        }
    }
}

impl FdDict {
    /// Validates zones and stems into a dictionary named `name`, as `buildBlueLists` does: the
    /// baseline zone must exist and be a bottom zone, top zones must have positive size and
    /// bottom zones negative, sorted zones must not overlap and must be at least
    /// `2 × BlueFuzz + 1` apart. More than [`MAX_ZONES`] zones in one list is an error. With
    /// `allow_no_blues` a missing baseline or empty stems pass.
    pub fn new(name: &str, zones: &ZoneSet, stems: &StemSet, allow_no_blues: bool) -> Result<Self, AutohintError> {
        for list in [&zones.blue_values, &zones.other_blues, &zones.family_blues, &zones.family_other_blues] {
            if list.len() > MAX_ZONES {
                return Err(AutohintError::LimitExceeded { what: "alignment zones in one list", limit: MAX_ZONES });
            }
        }
        // M10.13: buildBlueLists and the stem checks.
        let _ = allow_no_blues;
        Ok(Self {
            dict_name: name.to_string(),
            dominant_h: stems.h.clone(),
            dominant_v: stems.v.clone(),
            blue_fuzz: zones.blue_fuzz,
            ..Self::default()
        })
    }

    /// Every zone the hinter treats as a band: BlueValues then OtherBlues.
    pub fn bands(&self) -> impl Iterator<Item = &BlueValue> {
        self.blue_pairs.iter().chain(&self.other_blue_pairs)
    }
}
