//! The zones and stems the autohinter works against: what a font gives ([`ZoneSet`],
//! [`StemSet`]) and the validated form the hinter reads ([`FdDict`]).
//!
//! Ported from otfautohint's `fdTools.py` (`FDDict`, `buildBlueLists`; AFDKO 5.0.1, Apache-2.0,
//! Copyright 2021 Adobe; see `NOTICE`). The types are fixed by M10.1; M10.13 fills
//! [`FdDict::new`] and the zone-list builder that [`crate::fontinfo`] shares.

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

/// The position keys of the four zone lists, with their overshoot keys, as otfautohint's fontinfo
/// names them (`kBlueValueKeys`, `kOtherBlueValueKeys`). The family lists use the same keys with
/// `Family` in front. The first BlueValues key is the baseline, the only bottom zone of that list.
pub(crate) const BLUE_KEYS: [(&str, &str); 7] = [
    ("BaselineYCoord", "BaselineOvershoot"),
    ("CapHeight", "CapOvershoot"),
    ("LcHeight", "LcOvershoot"),
    ("AscenderHeight", "AscenderOvershoot"),
    ("FigHeight", "FigOvershoot"),
    ("Height5", "Height5Overshoot"),
    ("Height6", "Height6Overshoot"),
];
pub(crate) const OTHER_BLUE_KEYS: [(&str, &str); 5] = [
    ("Baseline5", "Baseline5Overshoot"),
    ("Baseline6", "Baseline6Overshoot"),
    ("SuperiorBaseline", "SuperiorOvershoot"),
    ("OrdinalBaseline", "OrdinalOvershoot"),
    ("DescenderHeight", "DescenderOvershoot"),
];

/// Where the inactive zones of a font without usable BlueValues sit when the autohinter runs
/// anyway: far outside any glyph, so no edge falls into them. otfautohint puts them just outside
/// the font's bounding box, which this crate does not know.
pub const INACTIVE_ZONE_LOW: f64 = -32_768.0;
pub const INACTIVE_ZONE_HIGH: f64 = 32_767.0;
/// The stem width a direction without standard stems gets when the autohinter runs anyway
/// (otfautohint uses the em size: wider than any hint, so it changes nothing).
pub const DUMMY_STEM: f64 = 1000.0;

/// Which of the four zone lists a [`ZoneKey`] belongs to (`buildBlueLists`' index `i`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZoneList {
    Blue,
    OtherBlue,
    Family,
    OtherFamily,
}

/// One zone as `buildBlueLists` reads it: the position key's name, the position (the zone's flat
/// edge) and the overshoot (negative for a bottom zone).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ZoneKey {
    pub name: String,
    pub position: f64,
    pub overshoot: f64,
    /// The key is `BaselineYCoord` (or `FamilyBaselineYCoord`): the one bottom zone allowed in
    /// the BlueValues lists.
    pub baseline: bool,
}

/// `buildBlueLists` for one list: each zone turned into a [`BlueValue`], checked for the sign
/// of its overshoot, sorted, and checked for overlaps and the `2 × fuzz + 1` separation. The
/// error is the message without its context.
pub(crate) fn build_pairs(list: ZoneList, zones: &[ZoneKey], fuzz: f64, dict_name: &str) -> Result<Vec<BlueValue>, String> {
    let mut pairs = Vec::with_capacity(zones.len());
    for z in zones {
        if !z.position.is_finite() || !z.overshoot.is_finite() {
            return Err(format!("zone {} is not a usable number", z.name));
        }
        if list == ZoneList::Blue && z.baseline && z.overshoot > 0.0 {
            return Err(format!("the baseline zone {} must be a bottom zone: its overshoot must be negative, not positive", z.name));
        }
        let (top, bottom, is_bottom) = if z.overshoot < 0.0 {
            if matches!(list, ZoneList::Blue | ZoneList::Family) && !z.baseline {
                return Err(format!("zone {} is a top zone, so its overshoot ({}) must be positive", z.name, z.overshoot));
            }
            (z.position, z.position + z.overshoot, true)
        } else {
            if list == ZoneList::OtherBlue && z.overshoot > 0.0 {
                return Err(format!("zone {} is a bottom zone, so its overshoot ({}) must be negative", z.name, z.overshoot));
            }
            (z.position + z.overshoot, z.position, false)
        };
        pairs.push(BlueValue { top, bottom, name: z.name.clone(), dict_name: dict_name.to_string(), is_bottom });
    }
    pairs.sort_by(|a, b| {
        a.top
            .total_cmp(&b.top)
            .then(a.bottom.total_cmp(&b.bottom))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.dict_name.cmp(&b.dict_name))
            .then(a.is_bottom.cmp(&b.is_bottom))
    });
    let buffer = 2.0 * fuzz + 1.0;
    for w in pairs.windows(2) {
        let [prev, pair] = w else { continue };
        if prev.top > pair.bottom {
            return Err(format!("the top of zone {} at {} overlaps zone {} with its bottom at {}", prev.name, prev.top, pair.name, pair.bottom));
        }
        if (pair.bottom - prev.top).abs() < buffer {
            return Err(format!(
                "the top of zone {} at {} is closer than {} units (2 × BlueFuzz + 1) to zone {} with its bottom at {}",
                prev.name, prev.top, buffer, pair.name, pair.bottom
            ));
        }
    }
    Ok(pairs)
}

/// Python's `int()` on a number: towards zero.
fn whole(v: f64) -> f64 {
    v.trunc()
}

/// The zones of one list as the Private dict gives them, in otfautohint's reading
/// (`getPrivateFDDict`): sorted by bottom edge, at most as many as the list has keys; in a
/// BlueValues list the first is the baseline (a bottom zone), the others top zones; every zone of
/// an OtherBlues list is a bottom zone. Values are truncated to whole units as `setInfo` does.
fn private_zones(zones: &[Zone], other: bool, family: bool) -> Result<Vec<ZoneKey>, AutohintError> {
    let mut sorted: Vec<&Zone> = zones.iter().collect();
    if sorted.iter().any(|z| !z.bottom.is_finite() || !z.top.is_finite()) {
        return Err(AutohintError::BadZones("a zone edge is not a usable number".into()));
    }
    sorted.sort_by(|a, b| a.bottom.total_cmp(&b.bottom).then(a.top.total_cmp(&b.top)));
    let keys: &[(&str, &str)] = if other { &OTHER_BLUE_KEYS } else { &BLUE_KEYS };
    let prefix = if family { "Family" } else { "" };
    Ok(sorted
        .into_iter()
        .zip(keys)
        .enumerate()
        .map(|(i, (z, (key, _)))| {
            let name = if z.name.is_empty() { format!("{prefix}{key}") } else { z.name.clone() };
            let bottom_zone = other || i == 0;
            if bottom_zone {
                ZoneKey { name, position: whole(z.top), overshoot: whole(z.bottom - z.top), baseline: !other && i == 0 }
            } else {
                ZoneKey { name, position: whole(z.bottom), overshoot: whole(z.top - z.bottom), baseline: false }
            }
        })
        .collect())
}

/// One direction's standard stems as `getPrivateFDDict` reads them: sorted and truncated; with
/// none (or a single width below 1) the dummy width, when that is allowed.
fn private_stems(stems: &[f64], dir: &str, allow_no_blues: bool) -> Result<Vec<f64>, AutohintError> {
    if stems.len() > MAX_ZONES {
        return Err(AutohintError::LimitExceeded { what: "standard stems in one direction", limit: MAX_ZONES });
    }
    if stems.iter().any(|s| !s.is_finite()) {
        return Err(AutohintError::BadStems(format!("a {dir} stem width is not a usable number")));
    }
    if stems.is_empty() && !allow_no_blues {
        return Err(AutohintError::BadStems(format!("there is no {dir} standard stem")));
    }
    let mut sorted = stems.to_vec();
    sorted.sort_by(f64::total_cmp);
    if matches!(sorted.as_slice(), [] | [_]) && sorted.first().is_none_or(|s| *s < 1.0) {
        return Ok(vec![DUMMY_STEM]);
    }
    Ok(sorted.into_iter().map(whole).collect())
}

impl FdDict {
    /// Validates zones and stems into a dictionary named `name`, as `getPrivateFDDict` and
    /// `buildBlueLists` do. The BlueValues are sorted by bottom edge; the lowest is the baseline
    /// zone (a bottom zone, its flat edge at `top`) and the others are top zones, which must have
    /// a positive size; every OtherBlues zone is a bottom zone. At most 7 BlueValues and 5
    /// OtherBlues zones are read (the keys otfautohint has; the CFF limits too). Sorted zones
    /// must not overlap and must be at least `2 × BlueFuzz + 1` apart. Values are truncated to
    /// whole units. More than [`MAX_ZONES`] zones in one list is an error. Fewer than two
    /// BlueValues zones, or a direction without stems, is an error unless `allow_no_blues`; then
    /// the zones are two inactive ones ([`INACTIVE_ZONE_LOW`], [`INACTIVE_ZONE_HIGH`]) and the
    /// stems [`DUMMY_STEM`].
    pub fn new(name: &str, zones: &ZoneSet, stems: &StemSet, allow_no_blues: bool) -> Result<Self, AutohintError> {
        for list in [&zones.blue_values, &zones.other_blues, &zones.family_blues, &zones.family_other_blues] {
            if list.len() > MAX_ZONES {
                return Err(AutohintError::LimitExceeded { what: "alignment zones in one list", limit: MAX_ZONES });
            }
        }
        if !zones.blue_fuzz.is_finite() {
            return Err(AutohintError::BadZones("BlueFuzz is not a usable number".into()));
        }
        let fuzz = whole(zones.blue_fuzz);
        let blues = if zones.blue_values.len() < 2 {
            if !allow_no_blues {
                return Err(AutohintError::BadZones("fewer than 4 BlueValues: the baseline zone and at least one top zone are needed".into()));
            }
            vec![
                ZoneKey { name: "BaselineYCoord".into(), position: INACTIVE_ZONE_LOW, overshoot: 0.0, baseline: true },
                ZoneKey { name: "CapHeight".into(), position: INACTIVE_ZONE_HIGH, overshoot: 0.0, baseline: false },
            ]
        } else {
            private_zones(&zones.blue_values, false, false)?
        };
        let baseline = blues.first().map(|z| (z.position, z.overshoot));
        let build = |list, keys: &[ZoneKey]| build_pairs(list, keys, fuzz, name).map_err(AutohintError::BadZones);
        Ok(Self {
            dict_name: name.to_string(),
            blue_pairs: build(ZoneList::Blue, &blues)?,
            other_blue_pairs: build(ZoneList::OtherBlue, &private_zones(&zones.other_blues, true, false)?)?,
            family_pairs: build(ZoneList::Family, &private_zones(&zones.family_blues, false, true)?)?,
            other_family_pairs: build(ZoneList::OtherFamily, &private_zones(&zones.family_other_blues, true, true)?)?,
            blue_fuzz: fuzz,
            dominant_h: private_stems(&stems.h, "horizontal", allow_no_blues)?,
            dominant_v: private_stems(&stems.v, "vertical", allow_no_blues)?,
            baseline,
            ..Self::default()
        })
    }

    /// Every zone the hinter treats as a band: BlueValues then OtherBlues.
    pub fn bands(&self) -> impl Iterator<Item = &BlueValue> {
        self.blue_pairs.iter().chain(&self.other_blue_pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(bottom: f64, top: f64) -> Zone {
        Zone { bottom, top, is_bottom: false, name: String::new() }
    }

    fn zones(blues: &[(f64, f64)], others: &[(f64, f64)], fuzz: f64) -> ZoneSet {
        ZoneSet {
            blue_values: blues.iter().map(|(b, t)| zone(*b, *t)).collect(),
            other_blues: others.iter().map(|(b, t)| zone(*b, *t)).collect(),
            blue_fuzz: fuzz,
            ..ZoneSet::default()
        }
    }

    fn stems(h: &[f64], v: &[f64]) -> StemSet {
        StemSet { h: h.to_vec(), v: v.to_vec() }
    }

    fn pair(top: f64, bottom: f64, name: &str, is_bottom: bool) -> BlueValue {
        BlueValue { top, bottom, name: name.into(), dict_name: "m".into(), is_bottom }
    }

    #[test]
    fn fd_dict_new_reads_zones_and_stems_as_otfautohint_reads_a_private_dict() {
        // The Private dict BlueValues [-12 0 500 512 700 712], OtherBlues [-260 -250], StemSnapH
        // [70.9], StemSnapV [80]: otfautohint's FDDict has the pairs below (values truncated).
        let z = zones(&[(700.0, 712.0), (-12.7, 0.0), (500.0, 512.0)], &[(-260.0, -250.0)], 1.0);
        let d = FdDict::new("m", &z, &stems(&[70.9], &[80.0]), false).unwrap();
        assert_eq!(
            d.blue_pairs,
            vec![pair(0.0, -12.0, "BaselineYCoord", true), pair(512.0, 500.0, "CapHeight", false), pair(712.0, 700.0, "LcHeight", false)]
        );
        assert_eq!(d.other_blue_pairs, vec![pair(-250.0, -260.0, "Baseline5", true)]);
        assert_eq!((d.dominant_h.clone(), d.dominant_v.clone(), d.blue_fuzz, d.baseline), (vec![70.0], vec![80.0], 1.0, Some((0.0, -12.0))));
    }

    #[test]
    fn fd_dict_new_refuses_what_otfautohint_refuses_unless_told_otherwise() {
        let ok_stems = stems(&[70.0], &[80.0]);
        let bad = |z: ZoneSet, s: &StemSet| FdDict::new("m", &z, s, false);
        assert!(matches!(bad(zones(&[(-12.0, 0.0)], &[], 1.0), &ok_stems), Err(AutohintError::BadZones(_))), "fewer than 4 BlueValues");
        assert!(
            matches!(bad(zones(&[(-12.0, 0.0), (500.0, 520.0), (510.0, 530.0)], &[], 1.0), &ok_stems), Err(AutohintError::BadZones(_))),
            "overlap"
        );
        assert!(
            matches!(bad(zones(&[(-12.0, 0.0), (500.0, 512.0), (514.0, 530.0)], &[], 1.0), &ok_stems), Err(AutohintError::BadZones(_))),
            "closer than 2 × 1 + 1"
        );
        assert!(bad(zones(&[(-12.0, 0.0), (500.0, 512.0), (515.0, 530.0)], &[], 1.0), &ok_stems).is_ok(), "3 units apart is enough");
        assert!(matches!(bad(zones(&[(-12.0, 0.0), (500.0, 512.0)], &[], 1.0), &stems(&[], &[80.0])), Err(AutohintError::BadStems(_))));
        assert!(matches!(bad(zones(&[(-12.0, f64::NAN), (500.0, 512.0)], &[], 1.0), &ok_stems), Err(AutohintError::BadZones(_))));
        let many = ZoneSet { blue_values: vec![zone(0.0, 0.0); MAX_ZONES + 1], ..ZoneSet::default() };
        assert!(matches!(bad(many, &ok_stems), Err(AutohintError::LimitExceeded { .. })));

        let d = FdDict::new("m", &ZoneSet::default(), &StemSet::default(), true).unwrap();
        let tops: Vec<f64> = d.blue_pairs.iter().map(|p| p.top).collect();
        assert_eq!(tops, vec![INACTIVE_ZONE_LOW, INACTIVE_ZONE_HIGH], "inactive zones outside every glyph");
        assert_eq!((d.dominant_h, d.dominant_v), (vec![DUMMY_STEM], vec![DUMMY_STEM]));
    }
}
