//! Our TrueType font programs: the `cvt ` layout, the `fpgm` function library and the `prep`
//! program of one master or instance.
//!
//! Contract (`docs/plans/M10.md` § 3 › Our TrueType programs). Written from the TrueType
//! instruction set specification and the handbook's description of the parameters; nothing is
//! taken from the reference's output, and byte parity with it is not a goal
//! (`hinting.tt.export.generatedPrograms`). Rows: `hinting.tt.param.ttfBlueFuzz`,
//! `hinting.tt.param.zoneRoundingThreshold`, `hinting.tt.param.overshootSuppressionBelowPpm`,
//! `hinting.tt.param.minimumDistance`, `hinting.tt.zones.align`, `hinting.tt.zones.deltas`,
//! `hinting.tt.stems.deltas`, `TTFFamilyZonesThreshold`, the `cvt `, `fpgm`, `prep` table rows.
//!
//! `cvt ` ([`CvtLayout`]): per TT zone two entries, its position (font units, never written at
//! run time) and its fitted row (written by `prep`); per TT stem its width and its fitted width;
//! then, when family zones apply, one position per family zone. A fitted entry starts equal to
//! its source, so a rasterizer that skips `prep` still sees the unrounded value.
//!
//! `prep`, once per pixel size: works out the rendering mode ([`S_GDI`]), whether overshoots
//! show ([`S_SHOW_OVERSHOOTS`]) and the minimum distance ([`S_MIN_DISTANCE`]); rounds every zone
//! that is not aligned (position + threshold, twice the threshold for `xHeight`, top zones only;
//! the family zone's position instead at sizes up to TTFFamilyZonesThreshold); places each
//! aligned zone at its target's row plus the rounded distance between them; applies the zone
//! deltas of the delta column; rounds every stem and applies its deltas under GDI ClearType only.
//!
//! `fpgm`: the functions below, numbered from 0, with their stack arguments listed bottom to top.
//! Glyph programs (`crate::tt_glyph`) set the projection and freedom vectors before calling the
//! ones that move points.
//!
//! | Number | Arguments | Effect |
//! |---|---|---|
//! | [`FN_ZONE_ROUND`] | fit, source, threshold | `cvt[fit] = round(cvt[source] + threshold)` |
//! | [`FN_ZONE_ALIGN`] | fit, position, target position, target fit | `cvt[fit] = cvt[target fit] + round(cvt[position] - cvt[target position])` |
//! | [`FN_ROUND`] | value | the value rounded half away from zero to whole pixels |
//! | [`FN_CVT_DELTA`] | cvt, ppm, amount | at that size, `cvt[cvt] += amount` |
//! | [`FN_ZONE_SNAP`] | point, zone position, zone fit | the point to the zone's row, plus its own rounded overshoot when overshoots show |
//! | [`FN_STEM`] | origin, target, stem width (−1: none), stem fit (−1: none), mode | the target at the stem's distance from the origin's current position |
//! | [`FN_POINT_DELTA`] | point, ppm, amount | at that size, the point moved by the amount |
//! | [`FN_POINT_ROUND`] | point, mode | the point rounded by the mode: nearest, up, down, kept (touched only) or nearest under GDI ClearType only |
//! | [`FN_ZONE_DISTANCE`] | point, zone position, zone fit | the point at its rounded original distance from the zone's row |
//! | [`FN_TRIPLE`] | hi1, lo2, hi2, lo3, hi3 | a triple hint: the middle stem (lo2, hi2), then the upper one (lo3, hi3), pushed up until at least a pixel separates each from the one below; `hi1` is the lower stem's upper edge |
//!
//! Amounts and positions on the stack are 26.6 pixels (64 = one pixel). Stem modes are
//! [`STEM_ROUND`] (the fitted width, or the rounded distance without a stem), [`STEM_UP`],
//! [`STEM_DOWN`], [`STEM_NO_ROUNDING`] and [`STEM_GDI_ONLY`] (rounded under GDI ClearType only);
//! every mode keeps at least the minimum distance.

use fontcraft_ttasm::{FunctionTable, ProgramLimits, analyse, assemble};

use crate::settings::{TtStem, TtZone};
use crate::{DeltaDir, HintSettings, HintingError, MAX_CVT_ENTRIES, Rounding};

/// `fpgm` function numbers (see the module table).
pub const FN_ZONE_ROUND: u8 = 0;
pub const FN_ZONE_ALIGN: u8 = 1;
pub const FN_ROUND: u8 = 2;
pub const FN_CVT_DELTA: u8 = 3;
pub const FN_ZONE_SNAP: u8 = 4;
pub const FN_STEM: u8 = 5;
pub const FN_POINT_DELTA: u8 = 6;
pub const FN_POINT_ROUND: u8 = 7;
pub const FN_ZONE_DISTANCE: u8 = 8;
pub const FN_TRIPLE: u8 = 9;
/// Functions `fpgm` defines.
pub const FUNCTION_COUNT: u16 = 10;
/// The stem function the autohinter adds after them ([`crate::tt_auto::autohint_fpgm`]): origin,
/// target, stem width entry (−1: none).
pub const FN_AUTO_STEM: u8 = 10;
/// The overshoot function the autohinter adds after it: a zone's overshoot at this size, unrounded
/// and signed, replaced by the one that shows.
pub const FN_AUTO_OVERSHOOT: u8 = 11;
/// [`FN_AUTO_STEM`] for a glyph of another zone group than the first: origin, target, stem width
/// entry (−1: none), the storage of the group's scaled x-height and of its fitted one.
pub const FN_AUTO_STEM_GROUP: u8 = 12;

/// Storage set by `prep`: 1 when overshoots show at this size.
pub const S_SHOW_OVERSHOOTS: u8 = 0;
/// Storage set by `prep`: the minimum distance, 26.6 pixels.
pub const S_MIN_DISTANCE: u8 = 1;
/// Storage set by `prep`: 1 under GDI ClearType (ClearType on, symmetric smoothing off).
pub const S_GDI: u8 = 2;
// Locals of `FN_STEM`.
const S_WIDTH: u8 = 3;
const S_FIT: u8 = 4;
const S_MODE: u8 = 5;
pub(crate) const S_NEGATIVE: u8 = 6;
// Locals of `FN_TRIPLE`.
const S_LO2: u8 = 7;
const S_HI2: u8 = 8;
const S_LO3: u8 = 9;
const S_HI3: u8 = 10;

/// `FN_STEM` modes.
pub const STEM_ROUND: u8 = 0;
pub const STEM_UP: u8 = 1;
pub const STEM_DOWN: u8 = 2;
pub const STEM_NO_ROUNDING: u8 = 3;
pub const STEM_GDI_ONLY: u8 = 4;

/// The `FN_STEM` mode of an instruction's rounding.
pub fn stem_mode(rounding: Rounding) -> u8 {
    match rounding {
        Rounding::Round => STEM_ROUND,
        Rounding::Up => STEM_UP,
        Rounding::Down => STEM_DOWN,
        Rounding::NoRounding => STEM_NO_ROUNDING,
        Rounding::GdiOnly => STEM_GDI_ONLY,
    }
}

/// Zone and stem delta cells written to one `prep`; each costs a function call per size.
pub const MAX_PREP_DELTA_CELLS: usize = 8_192;
/// The rounding threshold is clamped to ± this many pixels (the handbook's values are near 0.1).
pub const MAX_ZONE_THRESHOLD_PX: f64 = 1.0;
/// The minimum distance is clamped to 0 to this many pixels.
pub const MAX_MINIMUM_DISTANCE_PX: f64 = 8.0;

/// Where each zone and stem sits in `cvt `.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CvtLayout {
    /// Per TT zone (in [`HintSettings::tt_zones`] order): the entries of its position and of its
    /// fitted row.
    pub zones: Vec<(u16, u16)>,
    /// Per TT zone: the span a point must lie in to belong to the zone, in font units, widened by
    /// TTFBlueFuzz on both sides (the glyph compiler's test for Snap Auto).
    pub spans: Vec<(i32, i32)>,
    /// Per TT stem (in [`HintSettings::tt_stems`] order): the entries of its width and of its
    /// fitted width.
    pub stems: Vec<(u16, u16)>,
    /// Per TT zone, when family zones apply: the entry of the family zone's position.
    pub family: Vec<u16>,
    /// Entries used in all, at most [`MAX_CVT_ENTRIES`].
    pub len: u16,
}

impl CvtLayout {
    /// The layout for `settings`: zones, stems, then the family zones, which count only with a
    /// TTFFamilyZonesThreshold and as many family zones as the master has zones.
    pub fn new(settings: &HintSettings, family_zones: &[TtZone]) -> Result<Self, HintingError> {
        let with_family = family_applies(settings, family_zones);
        let family = if with_family { family_zones.len() } else { 0 };
        let n = settings.tt_zones.len().saturating_mul(2).saturating_add(settings.tt_stems.len().saturating_mul(2)).saturating_add(family);
        let len = u16::try_from(n)
            .ok()
            .filter(|_| n <= MAX_CVT_ENTRIES)
            .ok_or(HintingError::LimitExceeded { what: "cvt entries", limit: MAX_CVT_ENTRIES })?;
        let mut next = 0u16;
        let mut take = || {
            let at = next;
            next = next.saturating_add(1);
            at
        };
        let zones = settings.tt_zones.iter().map(|_| (take(), take())).collect();
        let stems = settings.tt_stems.iter().map(|_| (take(), take())).collect();
        let family = (0..family).map(|_| take()).collect();
        let fuzz = finite(settings.tt_blue_fuzz, "TTFBlueFuzz")?;
        let spans = settings
            .tt_zones
            .iter()
            .map(|z| {
                let (a, b) = (finite(z.position, "a TT zone position")?, finite(z.position + z.size, "a TT zone size")?);
                Ok(((a.min(b) - fuzz).floor().clamp(-1e9, 1e9) as i32, (a.max(b) + fuzz).ceil().clamp(-1e9, 1e9) as i32))
            })
            .collect::<Result<_, HintingError>>()?;
        Ok(Self { zones, spans, stems, family, len })
    }

    pub fn zone(&self, index: usize) -> Option<(u16, u16)> {
        self.zones.get(index).copied()
    }

    pub fn stem(&self, index: usize) -> Option<(u16, u16)> {
        self.stems.get(index).copied()
    }

    /// The entry an autohinted build adds after the layout's own for the overshoot edge of zone
    /// `index` ([`crate::autohint_prep`]), one per zone; `None` when `cvt ` has no room for them.
    pub fn overshoot(&self, index: usize) -> Option<u16> {
        let room = usize::from(self.len).saturating_add(self.zones.len()) <= MAX_CVT_ENTRIES;
        let at = usize::from(self.len).saturating_add(index);
        u16::try_from(at).ok().filter(|_| room && index < self.zones.len())
    }

    /// The first zone whose span holds `y` (font units).
    pub fn zone_at(&self, y: f64) -> Option<usize> {
        self.spans.iter().position(|&(lo, hi)| f64::from(lo) <= y && y <= f64::from(hi))
    }
}

/// The font-wide TrueType programs of one master or instance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontPrograms {
    pub cvt: Vec<i16>,
    pub fpgm: Vec<u8>,
    pub prep: Vec<u8>,
    /// What `fpgm` and `prep` need of `maxp`; the compiler merges the glyph programs' limits.
    pub limits: ProgramLimits,
    pub layout: CvtLayout,
    /// Settings that were left out, for the export report (an alignment to a missing zone or in
    /// a cycle, family zones whose count does not match).
    pub warnings: Vec<String>,
}

/// The `fpgm` and `prep` of `settings`, with the `cvt ` values they work on.
///
/// `family_zones` are the font's TT zones (`settings::family_tt_zones`); `delta_column` is the
/// column of the zone and stem delta tables that applies: the master id for a master's build, the
/// instance name for an instance's. Imported programs (cvt Table, prep and fpgm Table Assembly)
/// are the compiler's business, not this function's.
pub fn font_programs(settings: &HintSettings, family_zones: &[TtZone], delta_column: &str) -> Result<FontPrograms, HintingError> {
    let (layout, cvt) = control_values(settings, family_zones)?;
    let mut warnings = Vec::new();
    if !family_zones.is_empty() && settings.tt_family_zones_threshold.is_some() && layout.family.is_empty() {
        warnings.push(format!(
            "the font has {} family TT zones and the master {} TT zones; family zones are left out",
            family_zones.len(),
            settings.tt_zones.len()
        ));
    }

    let fpgm = assemble(&fpgm_text())?;
    let prep = assemble(&prep_text(settings, &layout, delta_column, &mut warnings)?)?;
    let functions = FunctionTable::from_fpgm(&fpgm)?;
    let mut limits = analyse(&fpgm, &functions)?;
    limits.merge(&analyse(&prep, &functions)?);
    // `maxp.maxSizeOfInstructions` counts glyph programs only.
    limits.max_size_of_instructions = 0;
    Ok(FontPrograms { cvt, fpgm, prep, limits, layout, warnings })
}

/// The `cvt ` layout of `settings` and its values: each zone's position, each stem's width, each
/// family zone's position, fitted entries equal to their source. A variable font's `cvar` varies
/// these values between masters, whose layouts match since every master's zones and stems are
/// laid out on the main master's lists.
pub fn control_values(settings: &HintSettings, family_zones: &[TtZone]) -> Result<(CvtLayout, Vec<i16>), HintingError> {
    let layout = CvtLayout::new(settings, family_zones)?;
    let mut cvt = vec![0i16; usize::from(layout.len)];
    let mut set = |at: u16, v: f64, what: &str| -> Result<(), HintingError> {
        let v = finite(v, what)?.round();
        let v = (f64::from(i16::MIN)..=f64::from(i16::MAX))
            .contains(&v)
            .then_some(v as i16)
            .ok_or_else(|| HintingError::BadNumber { what: format!("{what} {v}") })?;
        if let Some(slot) = cvt.get_mut(usize::from(at)) {
            *slot = v;
        }
        Ok(())
    };
    for (z, &(pos, fit)) in settings.tt_zones.iter().zip(&layout.zones) {
        set(pos, z.position, "a TT zone position")?;
        set(fit, z.position, "a TT zone position")?;
    }
    for (s, &(width, fit)) in settings.tt_stems.iter().zip(&layout.stems) {
        set(width, s.size, "a TT stem size")?;
        set(fit, s.size, "a TT stem size")?;
    }
    for (z, &at) in family_zones.iter().zip(&layout.family) {
        set(at, z.position, "a family TT zone position")?;
    }
    Ok((layout, cvt))
}

fn family_applies(settings: &HintSettings, family_zones: &[TtZone]) -> bool {
    settings.tt_family_zones_threshold.is_some() && !family_zones.is_empty() && family_zones.len() == settings.tt_zones.len()
}

fn finite(v: f64, what: &str) -> Result<f64, HintingError> {
    v.is_finite().then_some(v).ok_or_else(|| HintingError::BadNumber { what: what.to_string() })
}

/// Pixels to 26.6, rounded.
fn f26dot6(px: f64) -> i32 {
    (px * 64.0).round() as i32
}

/// The function library. Rounding is `FLOOR(x + 32)` rather than `ROUND`, so it does not depend
/// on the round state a glyph program leaves behind.
fn fpgm_text() -> String {
    format!(
        "\
; {FN_ZONE_ROUND}: fit source threshold
PUSH {FN_ZONE_ROUND}
FDEF[ ]
SWAP[ ]
RCVT[ ]
ADD[ ]
PUSH 32
ADD[ ]
FLOOR[ ]
WCVTP[ ]
ENDF[ ]

; {FN_ZONE_ALIGN}: fit position target_position target_fit
PUSH {FN_ZONE_ALIGN}
FDEF[ ]
RCVT[ ]
SWAP[ ]
RCVT[ ]
ROLL[ ]
RCVT[ ]
SWAP[ ]
SUB[ ]
PUSH {FN_ROUND}
CALL[ ]
ADD[ ]
WCVTP[ ]
ENDF[ ]

; {FN_ROUND}: value -> value rounded half away from zero
PUSH {FN_ROUND}
FDEF[ ]
DUP[ ]
PUSH 0
LT[ ]
IF[ ]
NEG[ ]
PUSH 32
ADD[ ]
FLOOR[ ]
NEG[ ]
ELSE[ ]
PUSH 32
ADD[ ]
FLOOR[ ]
EIF[ ]
ENDF[ ]

; {FN_CVT_DELTA}: cvt ppm amount
PUSH {FN_CVT_DELTA}
FDEF[ ]
SWAP[ ]
MPPEM[ ]
EQ[ ]
IF[ ]
SWAP[ ]
DUP[ ]
RCVT[ ]
ROLL[ ]
ADD[ ]
WCVTP[ ]
ELSE[ ]
POP[ ]
POP[ ]
EIF[ ]
ENDF[ ]

; {FN_ZONE_SNAP}: point position fit
PUSH {FN_ZONE_SNAP}
FDEF[ ]
RCVT[ ]
SWAP[ ]
RCVT[ ]
PUSH 3
CINDEX[ ]
GC[1]
SWAP[ ]
SUB[ ]
PUSH {S_SHOW_OVERSHOOTS}
RS[ ]
IF[ ]
PUSH {FN_ROUND}
CALL[ ]
ELSE[ ]
POP[ ]
PUSH 0
EIF[ ]
ADD[ ]
SCFS[ ]
ENDF[ ]

; {FN_STEM}: origin target width fit mode
PUSH {FN_STEM}
FDEF[ ]
PUSH {S_MODE}
SWAP[ ]
WS[ ]
PUSH {S_FIT}
SWAP[ ]
WS[ ]
PUSH {S_WIDTH}
SWAP[ ]
WS[ ]
; the original distance, its sign and size
DUP[ ]
GC[1]
PUSH 3
CINDEX[ ]
GC[1]
SUB[ ]
DUP[ ]
PUSH 0
LT[ ]
PUSH {S_NEGATIVE}
SWAP[ ]
WS[ ]
ABS[ ]
; the stem's width, or the distance without a stem
PUSH {S_WIDTH}
RS[ ]
DUP[ ]
PUSH 0
LT[ ]
IF[ ]
POP[ ]
ELSE[ ]
SWAP[ ]
POP[ ]
RCVT[ ]
EIF[ ]
; GDI-only: round under GDI ClearType, else keep
PUSH {S_MODE}
RS[ ]
PUSH {STEM_GDI_ONLY}
EQ[ ]
IF[ ]
PUSH {S_MODE}
PUSH {S_GDI}
RS[ ]
IF[ ]
PUSH {STEM_ROUND}
ELSE[ ]
PUSH {STEM_NO_ROUNDING}
EIF[ ]
WS[ ]
EIF[ ]
PUSH {S_MODE}
RS[ ]
PUSH {STEM_ROUND}
EQ[ ]
IF[ ]
PUSH {S_FIT}
RS[ ]
DUP[ ]
PUSH 0
LT[ ]
IF[ ]
POP[ ]
PUSH 32
ADD[ ]
FLOOR[ ]
ELSE[ ]
SWAP[ ]
POP[ ]
RCVT[ ]
EIF[ ]
EIF[ ]
PUSH {S_MODE}
RS[ ]
PUSH {STEM_UP}
EQ[ ]
IF[ ]
CEILING[ ]
EIF[ ]
PUSH {S_MODE}
RS[ ]
PUSH {STEM_DOWN}
EQ[ ]
IF[ ]
FLOOR[ ]
EIF[ ]
PUSH {S_MIN_DISTANCE}
RS[ ]
MAX[ ]
PUSH {S_NEGATIVE}
RS[ ]
IF[ ]
NEG[ ]
EIF[ ]
ROLL[ ]
SRP0[ ]
MSIRP[0]
ENDF[ ]

; {FN_POINT_DELTA}: point ppm amount
PUSH {FN_POINT_DELTA}
FDEF[ ]
SWAP[ ]
MPPEM[ ]
EQ[ ]
IF[ ]
SWAP[ ]
DUP[ ]
GC[0]
ROLL[ ]
ADD[ ]
SCFS[ ]
ELSE[ ]
POP[ ]
POP[ ]
EIF[ ]
ENDF[ ]

; {FN_POINT_ROUND}: point mode
PUSH {FN_POINT_ROUND}
FDEF[ ]
DUP[ ]
PUSH {STEM_GDI_ONLY}
EQ[ ]
IF[ ]
POP[ ]
PUSH {S_GDI}
RS[ ]
IF[ ]
PUSH {STEM_ROUND}
ELSE[ ]
PUSH {STEM_NO_ROUNDING}
EIF[ ]
EIF[ ]
DUP[ ]
PUSH {STEM_NO_ROUNDING}
EQ[ ]
IF[ ]
POP[ ]
MDAP[0]
ELSE[ ]
SWAP[ ]
DUP[ ]
GC[0]
ROLL[ ]
DUP[ ]
PUSH {STEM_UP}
EQ[ ]
IF[ ]
POP[ ]
CEILING[ ]
ELSE[ ]
PUSH {STEM_DOWN}
EQ[ ]
IF[ ]
FLOOR[ ]
ELSE[ ]
PUSH {FN_ROUND}
CALL[ ]
EIF[ ]
EIF[ ]
SCFS[ ]
EIF[ ]
ENDF[ ]

; {FN_ZONE_DISTANCE}: point position fit
PUSH {FN_ZONE_DISTANCE}
FDEF[ ]
RCVT[ ]
SWAP[ ]
RCVT[ ]
PUSH 3
CINDEX[ ]
GC[1]
SWAP[ ]
SUB[ ]
PUSH {FN_ROUND}
CALL[ ]
ADD[ ]
SCFS[ ]
ENDF[ ]

; {FN_TRIPLE}: hi1 lo2 hi2 lo3 hi3
PUSH {FN_TRIPLE}
FDEF[ ]
PUSH {S_HI3}
SWAP[ ]
WS[ ]
PUSH {S_LO3}
SWAP[ ]
WS[ ]
PUSH {S_HI2}
SWAP[ ]
WS[ ]
PUSH {S_LO2}
SWAP[ ]
WS[ ]
; what the middle stem must rise: one pixel less the gap above the lower stem
GC[0]
PUSH {S_LO2}
RS[ ]
GC[0]
SUB[ ]
PUSH 64
ADD[ ]
DUP[ ]
PUSH 0
GT[ ]
IF[ ]
PUSH {S_LO2}
RS[ ]
PUSH {S_HI2}
RS[ ]
ROLL[ ]
PUSH 2
SLOOP[ ]
SHPIX[ ]
ELSE[ ]
POP[ ]
EIF[ ]
; the same for the upper stem over the middle one
PUSH {S_HI2}
RS[ ]
GC[0]
PUSH {S_LO3}
RS[ ]
GC[0]
SUB[ ]
PUSH 64
ADD[ ]
DUP[ ]
PUSH 0
GT[ ]
IF[ ]
PUSH {S_LO3}
RS[ ]
PUSH {S_HI3}
RS[ ]
ROLL[ ]
PUSH 2
SLOOP[ ]
SHPIX[ ]
ELSE[ ]
POP[ ]
EIF[ ]
ENDF[ ]
"
    )
}

/// The `prep` program; see the module documentation for what it does in which order.
/// Largest size a `prep` comparison pushes.
const MAX_PPM_WORD: u16 = 32_767;

fn prep_text(settings: &HintSettings, layout: &CvtLayout, column: &str, warnings: &mut Vec<String>) -> Result<String, HintingError> {
    use std::fmt::Write;
    let mut t = String::new();
    // `write!` to a String cannot fail.
    let line = |t: &mut String, s: std::fmt::Arguments<'_>| {
        let _ = t.write_fmt(s);
        t.push('\n');
    };

    line(&mut t, format_args!("; GDI ClearType: ClearType on and symmetric smoothing off"));
    line(&mut t, format_args!("PUSH {S_GDI} 64\nGETINFO[ ]\nPUSH 0\nGT[ ]\nPUSH 2048\nGETINFO[ ]\nPUSH 0\nEQ[ ]\nAND[ ]\nWS[ ]"));
    // The settings take sizes up to 65535 PPM; PUSH takes signed words, and no device reaches
    // 32767 PPM.
    let ppm_word = |ppm: u16| ppm.min(MAX_PPM_WORD);
    match settings.tt_overshoot_suppression_below_ppm.map(ppm_word) {
        Some(ppm) => line(&mut t, format_args!("PUSH {S_SHOW_OVERSHOOTS}\nMPPEM[ ]\nPUSH {ppm}\nGTEQ[ ]\nWS[ ]")),
        None => line(&mut t, format_args!("PUSH {S_SHOW_OVERSHOOTS} 0\nWS[ ]")),
    }
    let min_distance = finite(settings.tt_minimum_distance, "TTFMinimumDistance")?.clamp(0.0, MAX_MINIMUM_DISTANCE_PX);
    line(&mut t, format_args!("PUSH {S_MIN_DISTANCE} {}\nWS[ ]", f26dot6(min_distance)));

    let threshold = finite(settings.tt_zone_rounding_threshold, "TTFZoneRoundingThreshold")?.clamp(-MAX_ZONE_THRESHOLD_PX, MAX_ZONE_THRESHOLD_PX);
    let mut cells = 0usize;
    let mut deltas = |t: &mut String, at: u16, d: &crate::Deltas| -> Result<(), HintingError> {
        for (&ppm, dir) in d.get(column).into_iter().flatten() {
            cells += 1;
            if cells > MAX_PREP_DELTA_CELLS {
                return Err(HintingError::LimitExceeded { what: "zone and stem delta cells", limit: MAX_PREP_DELTA_CELLS });
            }
            let amount = if *dir == DeltaDir::Up { 64 } else { -64 };
            line(t, format_args!("PUSH {at} {ppm} {amount} {FN_CVT_DELTA}\nCALL[ ]"));
        }
        Ok(())
    };

    let (order, targets) = zone_order(&settings.tt_zones, warnings);
    for k in order {
        let (Some(z), Some(&(pos, fit))) = (settings.tt_zones.get(k), layout.zones.get(k)) else { continue };
        match targets.get(k).copied().flatten() {
            Some(j) => {
                let Some(&(target_pos, target_fit)) = layout.zones.get(j) else { continue };
                line(&mut t, format_args!("; zone {k} aligned to zone {j}\nPUSH {fit} {pos} {target_pos} {target_fit} {FN_ZONE_ALIGN}\nCALL[ ]"));
            }
            None => {
                let times = if z.name == "xHeight" { 2.0 } else { 1.0 };
                let thr = if z.size > 0.0 { f26dot6(threshold * times) } else { 0 };
                match (settings.tt_family_zones_threshold.map(ppm_word), layout.family.get(k)) {
                    (Some(until), Some(family)) => line(
                        &mut t,
                        format_args!(
                            "; zone {k}\nMPPEM[ ]\nPUSH {until}\nLTEQ[ ]\nIF[ ]\nPUSH {fit} {family} {thr}\nELSE[ ]\nPUSH {fit} {pos} {thr}\nEIF[ ]\nPUSH {FN_ZONE_ROUND}\nCALL[ ]"
                        ),
                    ),
                    _ => line(&mut t, format_args!("; zone {k}\nPUSH {fit} {pos} {thr} {FN_ZONE_ROUND}\nCALL[ ]")),
                }
            }
        }
        deltas(&mut t, fit, &z.deltas)?;
    }

    for (k, (s, &(width, fit))) in settings.tt_stems.iter().zip(&layout.stems).enumerate() {
        line(&mut t, format_args!("; stem {k}\nPUSH {fit} {width} 0 {FN_ZONE_ROUND}\nCALL[ ]"));
        if stem_has_cells(s, column) {
            line(&mut t, format_args!("PUSH {S_GDI}\nRS[ ]\nIF[ ]"));
            deltas(&mut t, fit, &s.deltas)?;
            line(&mut t, format_args!("EIF[ ]"));
        }
    }
    Ok(t)
}

fn stem_has_cells(s: &TtStem, column: &str) -> bool {
    s.deltas.get(column).is_some_and(|c| !c.is_empty())
}

/// The zone `k` is aligned to, when that zone exists and is another one.
fn aligned_to(zones: &[TtZone], k: usize) -> Option<usize> {
    let name = zones.get(k)?.align.as_deref()?;
    zones.iter().position(|z| z.name == name).filter(|&j| j != k)
}

/// Each zone's alignment target, with alignments to a missing zone and the link that closes a
/// cycle left out (that zone is then rounded on its own) and reported, and the zone indexes with
/// every target before the zones aligned to it. At most quadratic in the zone count
/// ([`crate::MAX_TT_ZONES`]).
fn zone_order(zones: &[TtZone], warnings: &mut Vec<String>) -> (Vec<usize>, Vec<Option<usize>>) {
    let n = zones.len();
    let mut target: Vec<Option<usize>> = (0..n).map(|k| aligned_to(zones, k)).collect();
    for (z, t) in zones.iter().zip(&target) {
        if let (Some(name), None) = (z.align.as_deref(), t) {
            warnings.push(format!("TT zone `{}` is aligned to `{name}`, which is not another zone of the master", z.name));
        }
    }
    for start in 0..n {
        let mut seen = vec![start];
        let mut k = start;
        while let Some(j) = target.get(k).copied().flatten() {
            if seen.contains(&j) {
                if let Some(z) = zones.get(k) {
                    warnings.push(format!("TT zone `{}` closes an alignment cycle; it is rounded on its own", z.name));
                }
                if let Some(t) = target.get_mut(k) {
                    *t = None;
                }
                break;
            }
            seen.push(j);
            k = j;
        }
    }
    let mut placed = vec![false; n];
    let mut order = Vec::with_capacity(n);
    for start in 0..n {
        let mut chain = Vec::new();
        let mut k = Some(start);
        while let Some(c) = k.filter(|&c| !placed.get(c).copied().unwrap_or(true)) {
            chain.push(c);
            k = target.get(c).copied().flatten();
        }
        for &c in chain.iter().rev() {
            if let Some(p) = placed.get_mut(c) {
                *p = true;
            }
            order.push(c);
        }
    }
    (order, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tt_testfont::{font, gdi, grayscale, hinted_y, mono};
    use crate::{Deltas, settings::DEFAULT_MINIMUM_DISTANCE, settings::DEFAULT_ZONE_ROUNDING_THRESHOLD};

    const COLUMN: &str = "m01";

    fn zone(name: &str, position: f64, size: f64) -> TtZone {
        TtZone { name: name.to_string(), position, size, ..TtZone::default() }
    }

    fn stem(size: f64) -> TtStem {
        TtStem { name: format!("stem {size}"), horizontal: true, size, ..TtStem::default() }
    }

    fn cells(ppm: u16, dir: DeltaDir) -> Deltas {
        Deltas::from([(COLUMN.to_string(), [(ppm, dir)].into())])
    }

    fn settings(zones: Vec<TtZone>, stems: Vec<TtStem>) -> HintSettings {
        HintSettings {
            master: COLUMN.to_string(),
            tt_zones: zones,
            tt_stems: stems,
            tt_blue_fuzz: 1.0,
            tt_zone_rounding_threshold: 0.0,
            tt_minimum_distance: DEFAULT_MINIMUM_DISTANCE,
            ..HintSettings::default()
        }
    }

    fn programs(s: &HintSettings) -> FontPrograms {
        font_programs(s, &[], COLUMN).unwrap()
    }

    fn asm(text: &str) -> Vec<u8> {
        assemble(text).unwrap()
    }

    /// A glyph program snapping point `p` to zone `z` for each pair.
    fn snaps(p: &FontPrograms, pairs: &[(usize, usize)]) -> Vec<u8> {
        let mut t = String::from("SVTCA[0]\n");
        for &(point, z) in pairs {
            let (pos, fit) = p.layout.zone(z).unwrap();
            t.push_str(&format!("PUSH {point} {pos} {fit} {FN_ZONE_SNAP}\nCALL[ ]\n"));
        }
        asm(&t)
    }

    /// The y of point `point` (in pixels) after snapping it to zone `z`, at `ppem`.
    fn snapped(s: &HintSettings, points: &[(i16, i16)], pairs: &[(usize, usize)], ppem: u16) -> Vec<f32> {
        let p = programs(s);
        let f = font(&p, points, &snaps(&p, pairs));
        let y = hinted_y(&f, ppem, mono());
        pairs.iter().map(|&(point, _)| y[point]).collect()
    }

    #[test]
    fn the_handbooks_rounding_example() {
        // At 10 PPM and 1000 units: small caps at 6.45 px, x-height at 5.25 px.
        let zones = vec![zone("smallCaps", 645.0, 15.0), zone("xHeight", 525.0, 15.0)];
        let points = [(0, 645), (100, 525), (200, 0)];
        let pairs = [(0, 0), (1, 1)];
        let mut s = settings(zones, vec![]);
        s.tt_zone_rounding_threshold = 0.2;
        assert_eq!(snapped(&s, &points, &pairs, 10), [7.0, 6.0]);
        s.tt_zone_rounding_threshold = 0.0;
        assert_eq!(snapped(&s, &points, &pairs, 10), [6.0, 5.0]);
        // The documented default, 6/64 px, already lifts 6.45 px (6.54) but not 5.25 px (5.44).
        s.tt_zone_rounding_threshold = DEFAULT_ZONE_ROUNDING_THRESHOLD;
        assert_eq!(snapped(&s, &points, &pairs, 10), [7.0, 5.0]);
        // Bottom zones get no threshold: -5.25 px stays at -5.
        let mut s = settings(vec![zone("descender", -525.0, -15.0)], vec![]);
        s.tt_zone_rounding_threshold = 0.2;
        assert_eq!(snapped(&s, &[(0, -525), (100, 0), (200, 0)], &[(0, 0)], 10), [-5.0]);
    }

    #[test]
    fn an_aligned_zone_keeps_the_rounded_distance_to_its_target() {
        let points = [(0, 645), (100, 675), (200, 0)];
        let pairs = [(0, 0), (1, 1)];
        // 6.45 px and 6.75 px: 0.3 px apart, so the same row, though 6.75 alone rounds to 7.
        let mut b = zone("smallCaps", 675.0, 15.0);
        let alone = settings(vec![zone("capHeight", 645.0, 15.0), b.clone()], vec![]);
        assert_eq!(snapped(&alone, &points, &pairs, 10), [6.0, 7.0]);
        b.align = Some("capHeight".to_string());
        let aligned = settings(vec![zone("capHeight", 645.0, 15.0), b.clone()], vec![]);
        assert_eq!(snapped(&aligned, &points, &pairs, 10), [6.0, 6.0]);
        // 6.55 px and 7.06 px: half a pixel or more apart, so the next row up, where 7.06 alone
        // rounds to 7. The target comes after the aligned zone in the list.
        b.position = 706.0;
        let s = settings(vec![b, zone("capHeight", 655.0, 15.0)], vec![]);
        assert_eq!(snapped(&s, &[(0, 655), (100, 706), (200, 0)], &[(0, 1), (1, 0)], 10), [7.0, 8.0]);
    }

    #[test]
    fn a_zone_delta_moves_the_zone_at_its_size_only() {
        // 6.2 px at 10 PPM, 5.58 at 9, 7.44 at 12.
        let mut z = zone("xHeight", 620.0, 15.0);
        z.deltas = cells(10, DeltaDir::Up);
        z.deltas.insert("another master".to_string(), [(12, DeltaDir::Up)].into());
        let s = settings(vec![z], vec![]);
        let points = [(0, 620), (100, 0), (200, 0)];
        assert_eq!(snapped(&s, &points, &[(0, 0)], 10), [7.0]);
        assert_eq!(snapped(&s, &points, &[(0, 0)], 9), [6.0]);
        assert_eq!(snapped(&s, &points, &[(0, 0)], 12), [7.0]);
    }

    #[test]
    fn overshoots_collapse_below_the_suppression_size() {
        // A zone at 500 with an overshoot to 516: at 40 PPM 20 px and 20.64 px, at 60 PPM 30 px
        // and 30.96 px.
        let mut s = settings(vec![zone("xHeight", 500.0, 20.0)], vec![]);
        let points = [(0, 516), (100, 0), (200, 0)];
        assert_eq!(snapped(&s, &points, &[(0, 0)], 60), [30.0], "without the parameter, zones flatten");
        s.tt_overshoot_suppression_below_ppm = Some(60);
        assert_eq!(snapped(&s, &points, &[(0, 0)], 40), [20.0]);
        assert_eq!(snapped(&s, &points, &[(0, 0)], 60), [31.0]);
    }

    /// The target's y after a Stem from a point at 100 to one at 100 + `width` units, at 10 PPM.
    fn stem_target(s: &HintSettings, mode: u8, with_stem: bool, target: crate::tt_testfont::Target) -> f32 {
        let p = programs(s);
        let (w, f) = if with_stem { p.layout.stem(0).map(|(w, f)| (i32::from(w), i32::from(f))).unwrap() } else { (-1, -1) };
        let width = s.tt_stems.first().map(|t| t.size as i16).unwrap_or(30);
        let program = asm(&format!("SVTCA[0]\nPUSH 0 1 {w} {f} {mode} {FN_STEM}\nCALL[ ]"));
        let fnt = font(&p, &[(0, 100), (100, 100 + width), (200, 0)], &program);
        hinted_y(&fnt, 10, target)[1]
    }

    #[test]
    fn a_stem_is_never_drawn_shorter_than_the_minimum_distance() {
        // A stem of 30 units is 0.3 px at 10 PPM; the origin sits at 1 px.
        let mut s = settings(vec![], vec![stem(30.0)]);
        assert_eq!(stem_target(&s, STEM_ROUND, true, mono()), 1.25);
        assert_eq!(stem_target(&s, STEM_DOWN, true, mono()), 1.25);
        assert_eq!(stem_target(&s, STEM_UP, true, mono()), 2.0);
        assert!((stem_target(&s, STEM_NO_ROUNDING, true, mono()) - 1.3).abs() < 1.5 / 64.0);
        assert_eq!(stem_target(&s, STEM_DOWN, false, mono()), 1.25, "No Stem keeps the minimum too");
        s.tt_minimum_distance = 0.5;
        assert_eq!(stem_target(&s, STEM_ROUND, true, mono()), 1.5);
    }

    #[test]
    fn stem_deltas_and_gdi_only_rounding_act_under_gdi_cleartype_only() {
        // A stem of 100 units is 1 px at 10 PPM; a delta adds a pixel there.
        let mut t = stem(100.0);
        t.deltas = cells(10, DeltaDir::Up);
        let s = settings(vec![], vec![t]);
        assert_eq!(stem_target(&s, STEM_ROUND, true, gdi()), 3.0);
        assert_eq!(stem_target(&s, STEM_ROUND, true, grayscale()), 2.0);
        assert_eq!(stem_target(&s, STEM_ROUND, true, mono()), 2.0);
        // 130 units, 1.3 px: rounded to 1 px under GDI ClearType, kept elsewhere.
        let s = settings(vec![], vec![stem(130.0)]);
        assert_eq!(stem_target(&s, STEM_GDI_ONLY, true, gdi()), 2.0);
        assert!((stem_target(&s, STEM_GDI_ONLY, true, grayscale()) - 2.3).abs() < 1.5 / 64.0);
    }

    #[test]
    fn sizes_past_the_signed_word_range_still_build() {
        let mut s = settings(vec![zone("xHeight", 500.0, 20.0)], vec![]);
        s.tt_overshoot_suppression_below_ppm = Some(40_000);
        s.tt_family_zones_threshold = Some(40_000);
        let p = font_programs(&s, &[zone("xHeight", 500.0, 20.0)], COLUMN).unwrap();
        let points = [(0, 516), (100, 0), (200, 0)];
        let f = font(&p, &points, &snaps(&p, &[(0, 0)]));
        assert_eq!(hinted_y(&f, 60, mono())[0], 30.0, "overshoots stay suppressed at every size");
    }

    #[test]
    fn family_zones_replace_the_masters_up_to_their_threshold() {
        let mut s = settings(vec![zone("capHeight", 645.0, 15.0)], vec![]);
        s.tt_family_zones_threshold = Some(12);
        let family = [zone("capHeight", 700.0, 15.0)];
        let p = font_programs(&s, &family, COLUMN).unwrap();
        assert_eq!(p.layout.family.len(), 1);
        let f = font(&p, &[(0, 645), (100, 0), (200, 0)], &snaps(&p, &[(0, 0)]));
        assert_eq!(hinted_y(&f, 10, mono())[0], 7.0, "the family zone's 7 px, not the master's 6.45");
        assert_eq!(hinted_y(&f, 20, mono())[0], 13.0, "the master's 12.9 px");
        // A family list of another length is left out, with a warning.
        let p = font_programs(&s, &[family[0].clone(), zone("xHeight", 500.0, 15.0)], COLUMN).unwrap();
        assert!(p.layout.family.is_empty() && p.warnings.len() == 1);
    }

    #[test]
    fn every_option_runs_within_the_analysed_limits_at_every_size_and_intent() {
        let mut zones =
            vec![zone("baseline", 0.0, -15.0), zone("xHeight", 500.0, 12.0), zone("capHeight", 700.0, 15.0), zone("ascender", 760.0, 15.0)];
        zones[3].align = Some("capHeight".to_string());
        zones[1].deltas = cells(14, DeltaDir::Down);
        // A cycle and a missing target: reported, rounded on their own.
        let mut a = zone("a", 300.0, 10.0);
        a.align = Some("b".to_string());
        let mut b = zone("b", 320.0, 10.0);
        b.align = Some("a".to_string());
        let mut c = zone("c", 340.0, 10.0);
        c.align = Some("nowhere".to_string());
        zones.extend([a, b, c]);
        let mut t = stem(80.0);
        t.deltas = cells(20, DeltaDir::Down);
        let mut s = settings(zones, vec![t, stem(30.0)]);
        s.tt_overshoot_suppression_below_ppm = Some(30);
        s.tt_zone_rounding_threshold = DEFAULT_ZONE_ROUNDING_THRESHOLD;
        s.tt_family_zones_threshold = Some(16);
        let family: Vec<_> = s.tt_zones.iter().map(|z| zone(&z.name, z.position + 10.0, z.size)).collect();
        let p = font_programs(&s, &family, COLUMN).unwrap();
        assert_eq!(p.warnings.len(), 2, "{:?}", p.warnings);
        assert_eq!((p.limits.max_function_defs, p.limits.max_storage, p.limits.max_twilight_points), (FUNCTION_COUNT, 11, 0));
        assert_eq!(usize::from(p.layout.len), p.cvt.len());
        assert_eq!(p.layout.zone_at(-16.0), Some(0), "TTFBlueFuzz widens the span by 1");
        assert_eq!(p.layout.zone_at(-17.0), None);

        let mut text = format!("SVTCA[0]\nPUSH 0 {} {} {FN_ZONE_SNAP}\nCALL[ ]\n", p.layout.zones[1].0, p.layout.zones[1].1);
        for mode in [STEM_ROUND, STEM_UP, STEM_DOWN, STEM_NO_ROUNDING, STEM_GDI_ONLY] {
            let (w, f) = p.layout.stems[0];
            text.push_str(&format!("PUSH 0 1 {w} {f} {mode} {FN_STEM}\nCALL[ ]\nPUSH 1 0 -1 -1 {mode} {FN_STEM}\nCALL[ ]\n"));
        }
        text.push_str(&format!("PUSH 2 14 64 {FN_POINT_DELTA}\nCALL[ ]\nIUP[0]\nIUP[1]"));
        let fnt = font(&p, &[(0, 506), (100, 420), (200, 0), (300, 300)], &asm(&text));
        for ppm in 8..=72 {
            for target in [mono(), grayscale(), gdi()] {
                let y = hinted_y(&fnt, ppm, target);
                assert!(y.iter().all(|v| v.is_finite()));
            }
        }
    }

    #[test]
    fn out_of_range_values_are_errors_not_panics() {
        let s = settings(vec![zone("x", 1e9, 10.0)], vec![]);
        assert!(matches!(font_programs(&s, &[], COLUMN), Err(HintingError::BadNumber { .. })));
        let s = settings(vec![zone("x", f64::NAN, 10.0)], vec![]);
        assert!(font_programs(&s, &[], COLUMN).is_err());
        let mut s = settings(vec![], vec![]);
        s.tt_minimum_distance = f64::INFINITY;
        assert!(font_programs(&s, &[], COLUMN).is_err());
        let many: Vec<_> = (0..MAX_CVT_ENTRIES).map(|i| zone(&i.to_string(), 0.0, 10.0)).collect();
        assert!(matches!(font_programs(&settings(many, vec![]), &[], COLUMN), Err(HintingError::LimitExceeded { .. })));
    }
}
