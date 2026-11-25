//! The TrueType autohinter: Snap and Stem instructions from the PostScript autohinter's zone
//! and stem analysis, compiled by the manual-instruction compiler (owner decision M10 § 7,
//! question 2 (a)). When TTFAutohint binary path names a ttfautohint program (desktop only), the
//! compiler runs that program on the finished font instead; that path is the compiler's.
//!
//! Rows: `autohint.tt.run`, `hint.tt.autohintGlyph`, `export.autohintTrueType`,
//! `hinting.tt.export.autohintIgnoresManual`, `hinting.tt.param.ttfAutohintOptions`,
//! `autohint.tt.option.*`.
//!
//! One glyph layer, in this order:
//!
//! 1. Vertical Snap on every on-curve node of each flat run that is a top extreme inside a top TT
//!    zone, or a bottom extreme inside a bottom zone (the zone widened by TTFBlueFuzz). Every
//!    node is snapped, not one per height: IUP gives an untouched point the move of a touched
//!    neighbour only when it lies at the end of the range its two touched neighbours span, and a
//!    node next to a slanted line moves along that line.
//! 2. Vertical Stem for every horizontal stem the PostScript autohinter finds (its `hstem`
//!    list, ghosts left out, hint substitution off so no two stems overlap): between the
//!    on-curve nodes on its two edges that lie closest together across, from the edge already
//!    touched to the other. Stems with a touched edge go first; when none is left, the lowest
//!    stem starts from its bottom edge, which the compiler rounds.
//! 3. The `left` and `right` points of the control instructions, each a one-point segment from
//!    its x plus the left offset to its x plus the right offset: a vertical Snap when it lies in
//!    a zone; else a vertical Stem from the nearest flat run above or below that overlaps the
//!    segment, within one and a half times the glyph's widest horizontal stem; else a rounded
//!    vertical Snap, an edge on the grid.
//! 4. Vertical Delta for the y shifts of the `touch` control instructions (before IUP, whole
//!    pixels: a shift of half a pixel or more moves one pixel).
//!
//! Nothing is hinted horizontally. The `point` shifts and the x shifts of `touch` run after
//! IUP, in eighths of a pixel ([`AutoHints::shifts`], [`shift_program`]); `nodir` points are
//! never instructed.
//!
//! The Stems compile to calls of the autohinter's own stem function ([`autohint_fpgm`]), which
//! takes the width by the stem width mode (natural, quantized or strong) of the rendering target.
//!
//! `prep` ([`autohint_prep`]) scales the zones so the x-height lands on the pixel grid, and
//! switches the glyph programs off above the hinting limit. At export the `xHeight` and
//! `capHeight` zones apply only when the font draws a letter ttfautohint measures its zones on
//! and the zone holds that letter's top, or is named after the letter's own height
//! ([`drop_unmeasured_zones`]); the x-height scaling needs a lower-case one.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fontcraft_autohint::{FdDict, HintInput, Options as PsOptions, StemSet, Zone, ZoneSet, hint_glyph};
use fontcraft_geom::{BezPath, PathEl};
use fontcraft_model::{Font, Layer, Shape};

use crate::control::{ControlInstructions, ControlKind};
use crate::tt::delta_column_key;
use crate::{DeltaDir, Direction, HintSettings, HintingError, NodeRef, PointMap, Rounding, StemRef, TtInstruction, TtKind, TtPoint, ZoneRef};

/// Most nodes of one layer the autohinter looks at (the TrueType point limit).
pub const MAX_AUTOHINT_NODES: usize = 65_535;
/// How far a node may lie from a stem edge or another node's height and still count as on it,
/// in font units.
const EDGE_TOLERANCE: f64 = 0.5;
/// Nodes of one stem edge compared when its two edges are paired, which bounds the pairing to
/// a fixed amount of work per stem.
const MAX_EDGE_CANDIDATES: usize = 256;
/// A shift of a control instruction smaller than this, in pixels, gives no Delta: a Delta cell
/// moves by whole steps.
const MIN_DELTA_SHIFT: f64 = 0.5;

/// How stems are rounded in one rendering mode (`--stem-width-mode` letters n, q, s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StemWidthMode {
    Natural,
    #[default]
    Quantized,
    Strong,
}

impl StemWidthMode {
    /// The mode's number in [`S_STEM_MODE`].
    fn number(self) -> u8 {
        match self {
            StemWidthMode::Natural => 0,
            StemWidthMode::Quantized => 1,
            StemWidthMode::Strong => 2,
        }
    }
}

/// The TTFAutohint options, with ttfautohint 1.8's defaults where the handbook gives none.
#[derive(Clone, Debug, PartialEq)]
pub struct TtAutohintOptions {
    pub hinting_range_min: u16,
    pub hinting_range_max: u16,
    /// 0: no limit.
    pub hinting_limit: u16,
    /// 0: off.
    pub increase_x_height: u16,
    pub x_height_snapping_exceptions: String,
    pub fallback_stem_width: Option<u16>,
    pub windows_compatibility: bool,
    pub adjust_subglyphs: bool,
    pub hint_composites: bool,
    pub symbol: bool,
    pub dehint: bool,
    pub no_info: bool,
    /// Detailed info: the version string gets the autohinter's options too.
    pub detailed_info: bool,
    pub ignore_restrictions: bool,
    pub ttfa_table: bool,
    pub default_script: String,
    pub fallback_script: String,
    pub fallback_scaling: bool,
    /// Grayscale, GDI ClearType, DirectWrite ClearType: `qsq` by default.
    pub stem_width_mode: [StemWidthMode; 3],
}

impl Default for TtAutohintOptions {
    fn default() -> Self {
        Self {
            hinting_range_min: 8,
            hinting_range_max: 50,
            hinting_limit: 200,
            increase_x_height: 14,
            x_height_snapping_exceptions: String::new(),
            fallback_stem_width: None,
            windows_compatibility: false,
            adjust_subglyphs: false,
            hint_composites: false,
            symbol: false,
            dehint: false,
            no_info: false,
            detailed_info: false,
            ignore_restrictions: false,
            ttfa_table: false,
            default_script: "latn".into(),
            fallback_script: "none".into(),
            fallback_scaling: false,
            stem_width_mode: [StemWidthMode::Quantized, StemWidthMode::Strong, StemWidthMode::Quantized],
        }
    }
}

/// The options our autohinter has no use for: accepted, checked and reported as ignored.
const IGNORED_OPTIONS: [&str; 10] = [
    "adjust-subglyphs",
    "default-script",
    "fallback-script",
    "fallback-scaling",
    "reference",
    "reference-index",
    "family-suffix",
    "control-file",
    "epoch",
    "verbose",
];
/// Longest option text read, and longest single value.
const MAX_OPTIONS_TEXT: usize = 4_096;
const MAX_OPTION_VALUE: usize = 256;
/// Size ranges in the x-height snapping exceptions.
pub const MAX_SNAPPING_EXCEPTIONS: usize = 256;
/// Highest PPM the options may name (PUSH takes signed words).
const MAX_OPTION_PPM: u16 = 32_767;

/// The long name of an option and whether it takes a value, from its long or short name.
fn option_name(name: &str) -> Option<(&'static str, bool)> {
    const TABLE: [(&str, &str, bool); 27] = [
        ("hinting-range-min", "l", true),
        ("hinting-range-max", "r", true),
        ("hinting-limit", "G", true),
        ("increase-x-height", "x", true),
        ("x-height-snapping-exceptions", "X", true),
        ("fallback-stem-width", "H", true),
        ("windows-compatibility", "W", false),
        ("adjust-subglyphs", "p", false),
        ("pre-hinting", "", false),
        ("composites", "c", false),
        ("symbol", "s", false),
        ("dehint", "d", false),
        ("no-info", "n", false),
        ("detailed-info", "I", false),
        ("ttfa-table", "t", false),
        ("default-script", "D", true),
        ("fallback-script", "f", true),
        ("fallback-scaling", "S", false),
        ("stem-width-mode", "a", true),
        ("strong-stem-width", "w", true),
        ("ignore-restrictions", "i", false),
        ("reference", "R", true),
        ("reference-index", "Z", true),
        ("family-suffix", "F", true),
        ("control-file", "m", true),
        ("epoch", "", true),
        ("verbose", "v", false),
    ];
    TABLE.iter().find(|(long, short, _)| *long == name || (!short.is_empty() && *short == name)).map(|(long, _, v)| (*long, *v))
}

impl TtAutohintOptions {
    /// Reads the TTFAutohint options parameter (command-line options, long or short, a value
    /// after `=` or as the next word); returns the options and the long names of options given
    /// but ignored by our autohinter. An unknown option or a bad value is an error.
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), HintingError> {
        let bad = |message: String| HintingError::BadParameter { name: "TTFAutohint options".into(), message };
        if text.len() > MAX_OPTIONS_TEXT {
            return Err(HintingError::LimitExceeded { what: "bytes of TTFAutohint options", limit: MAX_OPTIONS_TEXT });
        }
        let mut o = Self::default();
        let mut ignored: Vec<String> = Vec::new();
        let mut words = text.split_whitespace();
        while let Some(word) = words.next() {
            let (raw, inline) = match word.strip_prefix("--") {
                Some(long) => match long.split_once('=') {
                    Some((n, v)) => (n, Some(v)),
                    None => (long, None),
                },
                None => match word.strip_prefix('-') {
                    Some(short) if !short.is_empty() => {
                        let split = short.char_indices().nth(1).map_or(short.len(), |(i, _)| i);
                        let (n, v) = short.split_at(split);
                        (n, (!v.is_empty()).then_some(v.trim_start_matches('=')))
                    }
                    _ => return Err(bad(format!("`{word}` is not an option"))),
                },
            };
            let Some((name, takes_value)) = option_name(raw) else { return Err(bad(format!("unknown option `{word}`"))) };
            let value = if takes_value {
                let v = match inline {
                    Some(v) => v,
                    None => words.next().ok_or_else(|| bad(format!("`{name}` needs a value")))?,
                };
                if v.len() > MAX_OPTION_VALUE {
                    return Err(bad(format!("the value of `{name}` is too long")));
                }
                v
            } else {
                if inline.is_some() {
                    return Err(bad(format!("`{name}` takes no value")));
                }
                ""
            };
            let ppm =
                |v: &str| v.parse::<u16>().ok().filter(|p| *p <= MAX_OPTION_PPM).ok_or_else(|| bad(format!("`{name}`: `{v}` is not a pixel size")));
            match name {
                "hinting-range-min" => o.hinting_range_min = ppm(value)?,
                "hinting-range-max" => o.hinting_range_max = ppm(value)?,
                "hinting-limit" => o.hinting_limit = ppm(value)?,
                "increase-x-height" => o.increase_x_height = ppm(value)?,
                "x-height-snapping-exceptions" => {
                    snapping_exceptions(value).map_err(|m| bad(format!("`{name}`: {m}")))?;
                    o.x_height_snapping_exceptions = value.to_string();
                }
                "fallback-stem-width" => {
                    o.fallback_stem_width = Some(
                        value
                            .parse::<u16>()
                            .ok()
                            .filter(|w| *w > 0)
                            .ok_or_else(|| bad(format!("`{name}`: `{value}` is not a width in font units")))?,
                    )
                }
                "windows-compatibility" => o.windows_compatibility = true,
                "adjust-subglyphs" | "pre-hinting" => o.adjust_subglyphs = true,
                "composites" => o.hint_composites = true,
                "symbol" => o.symbol = true,
                "dehint" => o.dehint = true,
                "no-info" => o.no_info = true,
                "detailed-info" => o.detailed_info = true,
                "ttfa-table" => o.ttfa_table = true,
                "default-script" | "fallback-script" => {
                    if value.is_empty() || value.len() > 8 || !value.chars().all(|c| c.is_ascii_alphanumeric()) {
                        return Err(bad(format!("`{name}`: `{value}` is not a script tag")));
                    }
                    if name == "default-script" { o.default_script = value.to_string() } else { o.fallback_script = value.to_string() }
                }
                "fallback-scaling" => o.fallback_scaling = true,
                "stem-width-mode" => {
                    let modes: Vec<StemWidthMode> = value
                        .chars()
                        .map(|c| match c {
                            'n' => Some(StemWidthMode::Natural),
                            'q' => Some(StemWidthMode::Quantized),
                            's' => Some(StemWidthMode::Strong),
                            _ => None,
                        })
                        .collect::<Option<_>>()
                        .ok_or_else(|| bad(format!("`{name}`: `{value}` is not three of n, q, s")))?;
                    o.stem_width_mode = modes.try_into().map_err(|_| bad(format!("`{name}`: `{value}` is not three of n, q, s")))?;
                }
                "strong-stem-width" => {
                    if !value.chars().all(|c| matches!(c, 'g' | 'G' | 'D')) {
                        return Err(bad(format!("`{name}`: `{value}` is not made of g, G and D")));
                    }
                    let mode = |c| if value.contains(c) { StemWidthMode::Strong } else { StemWidthMode::Quantized };
                    o.stem_width_mode = [mode('g'), mode('G'), mode('D')];
                }
                "ignore-restrictions" => o.ignore_restrictions = true,
                _ => {}
            }
            let reported = if matches!(name, "pre-hinting") { "adjust-subglyphs" } else { name };
            if IGNORED_OPTIONS.contains(&reported) && !ignored.iter().any(|i| i == reported) {
                ignored.push(reported.to_string());
            }
        }
        if o.hinting_range_min < 2 || o.hinting_range_min > o.hinting_range_max {
            return Err(bad(format!("the hint set range {} to {} is not a range of sizes from 2 PPM", o.hinting_range_min, o.hinting_range_max)));
        }
        if o.hinting_limit != 0 && o.hinting_limit < o.hinting_range_max {
            return Err(bad(format!("the hinting limit {} lies below the top of the hint set range {}", o.hinting_limit, o.hinting_range_max)));
        }
        if o.increase_x_height != 0 && o.increase_x_height < 6 {
            return Err(bad(format!("the x-height increase limit {} is neither 0 nor at least 6", o.increase_x_height)));
        }
        if o.no_info && o.detailed_info {
            return Err(bad("no-info and detailed-info exclude each other".into()));
        }
        Ok((o, ignored))
    }

    /// The options given that act only on an autohinted export, not on the instructions the
    /// commands store.
    pub fn export_only(&self) -> Vec<String> {
        let d = Self::default();
        [(self.stem_width_mode != d.stem_width_mode, "stem-width-mode"), (self.hint_composites, "composites")]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, name)| name.to_string())
            .collect()
    }

    /// The options that differ from the defaults, as long options, which
    /// [`TtAutohintOptions::parse`] reads back.
    pub fn to_text(&self) -> String {
        let d = Self::default();
        let mut out: Vec<String> = Vec::new();
        let mut num = |name: &str, v: u16, dv: u16| {
            if v != dv {
                out.push(format!("--{name}={v}"));
            }
        };
        num("hinting-range-min", self.hinting_range_min, d.hinting_range_min);
        num("hinting-range-max", self.hinting_range_max, d.hinting_range_max);
        num("hinting-limit", self.hinting_limit, d.hinting_limit);
        num("increase-x-height", self.increase_x_height, d.increase_x_height);
        if !self.x_height_snapping_exceptions.is_empty() {
            out.push(format!("--x-height-snapping-exceptions={}", self.x_height_snapping_exceptions.replace(' ', "")));
        }
        if let Some(w) = self.fallback_stem_width {
            out.push(format!("--fallback-stem-width={w}"));
        }
        for (on, name) in [
            (self.windows_compatibility, "windows-compatibility"),
            (self.adjust_subglyphs, "adjust-subglyphs"),
            (self.hint_composites, "composites"),
            (self.symbol, "symbol"),
            (self.dehint, "dehint"),
            (self.no_info, "no-info"),
            (self.detailed_info, "detailed-info"),
            (self.ignore_restrictions, "ignore-restrictions"),
            (self.ttfa_table, "ttfa-table"),
            (self.fallback_scaling, "fallback-scaling"),
        ] {
            if on {
                out.push(format!("--{name}"));
            }
        }
        if self.default_script != d.default_script {
            out.push(format!("--default-script={}", self.default_script));
        }
        if self.fallback_script != d.fallback_script {
            out.push(format!("--fallback-script={}", self.fallback_script));
        }
        if self.stem_width_mode != d.stem_width_mode {
            let letter = |m: &StemWidthMode| match m {
                StemWidthMode::Natural => 'n',
                StemWidthMode::Quantized => 'q',
                StemWidthMode::Strong => 's',
            };
            out.push(format!("--stem-width-mode={}", self.stem_width_mode.iter().map(letter).collect::<String>()));
        }
        out.join(" ")
    }
}

/// The size ranges of x-height snapping exceptions (`6-9, 12, 15-`; `-` alone: every size), as
/// inclusive (low, high) pairs.
pub fn snapping_exceptions(text: &str) -> Result<Vec<(u16, u16)>, String> {
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if out.len() >= MAX_SNAPPING_EXCEPTIONS {
            return Err(format!("more than {MAX_SNAPPING_EXCEPTIONS} sizes and ranges"));
        }
        let size = |v: &str, empty: u16| -> Result<u16, String> {
            if v.trim().is_empty() {
                return Ok(empty);
            }
            v.trim().parse::<u16>().ok().filter(|p| *p <= MAX_OPTION_PPM).ok_or_else(|| format!("`{v}` is not a pixel size"))
        };
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (size(a, 1)?, size(b, MAX_OPTION_PPM)?),
            None => {
                let p = size(part, 1)?;
                (p, p)
            }
        };
        if lo > hi {
            return Err(format!("`{part}` is not a range"));
        }
        out.push((lo, hi));
    }
    Ok(out)
}

/// What the autohinter adds to `prep` after our own program ([`crate::font_programs`]): the
/// zones rounded as ttfautohint rounds them, with the x-height scaling when the master has an
/// `xHeight` zone, the stem width mode of the rendering target ([`S_STEM_MODE`]), and the hinting
/// limit (glyph programs off above it).
///
/// X-height scaling, as the ttfautohint manual describes it and its output on our test fonts
/// shows: the top of the `xHeight` zone, overshoot included, is rounded to the pixel grid with a
/// preference for rounding up (up from 3/8 of a pixel above a row; from 3/16 at 6 PPM up to the
/// x-height increase limit), and every zone is then placed at its position scaled by the same
/// factor, rounded to the nearest row. Horizontal stems scale by that factor too. At the sizes of
/// the x-height snapping exceptions, and at every size without an `xHeight` zone or with
/// `scale_x_height` off ([`MeasuredZones::x_height`]), the factor is 1, so the zones are only
/// rounded. Zone and stem
/// deltas of our own `prep` are kept: each fitted entry moves by the difference between the
/// scaled row and the row our program gave it. Zones aligned to another zone keep their rounded
/// distance to it; zones in an alignment cycle, and those aligned to one, stay as our program
/// placed them.
///
/// Overshoots: a round extreme past half of a zone's size ([`crate::glyph_program`]) lies beyond
/// the zone's row by the zone's whole size at this size, scaled by the x-height factor and
/// replaced ([`autohint_fpgm`]): nothing under half a pixel, half a pixel up to three quarters,
/// from there whole pixels. `prep` leaves that row in the zone's overshoot entry.
///
/// Returns the `cvt ` entries to append after `layout`'s (each zone's overshoot edge, its position
/// plus its size, so the rasterizer scales it as it scales the zones; none when `cvt ` has no
/// room, which leaves out the x-height scaling and the overshoots) and the program to append to
/// `prep`. Storage: [`S_X_SCALED`] and [`S_X_FITTED`], after the font programs' own.
pub fn autohint_prep(
    options: &TtAutohintOptions,
    settings: &HintSettings,
    layout: &crate::CvtLayout,
    scale_x_height: bool,
) -> Result<(Vec<i16>, Vec<u8>), HintingError> {
    use std::fmt::Write;
    let mut t = stem_mode_program(options);
    let cvt = match layout.overshoot(0) {
        Some(_) => settings.tt_zones.iter().map(|z| (z.position + z.size).round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16).collect(),
        None => Vec::new(),
    };
    let exceptions = snapping_exceptions(&options.x_height_snapping_exceptions)
        .map_err(|message| HintingError::BadParameter { name: "TTFAutohint options".into(), message })?;
    t.push_str(&x_height_scaling(options, settings, layout, &exceptions, scale_x_height));
    t.push_str(&overshoot_rows(settings, layout));
    if options.hinting_limit > 0 {
        let _ = writeln!(t, "MPPEM[ ]\nPUSH {}\nGT[ ]\nIF[ ]\nPUSH 1 1\nINSTCTRL[ ]\nEIF[ ]", options.hinting_limit.min(MAX_OPTION_PPM));
    }
    Ok((cvt, fontcraft_ttasm::assemble(&t)?))
}

/// The `prep` text that stores the stem width mode of the rendering target: the grayscale mode
/// without ClearType, else the GDI or the DirectWrite ClearType mode.
fn stem_mode_program(options: &TtAutohintOptions) -> String {
    use crate::tt_font::S_GDI;
    let [gray, gdi, dw] = options.stem_width_mode.map(StemWidthMode::number);
    format!(
        "; stem width mode\nPUSH {S_STEM_MODE} 64\nGETINFO[ ]\nPUSH 0\nGT[ ]\nIF[ ]\nPUSH {S_GDI}\nRS[ ]\nIF[ ]\nPUSH {gdi}\nELSE[ ]\nPUSH {dw}\nEIF[ ]\nELSE[ ]\nPUSH {gray}\nEIF[ ]\nWS[ ]\n"
    )
}

/// What an autohinted build adds to `fpgm` after our own functions: [`FN_AUTO_STEM`], which puts a
/// stem's target at its distance from the origin by the stem width mode, and
/// [`FN_AUTO_OVERSHOOT`]. The distance and the standard width (the TT stem the instruction uses)
/// and the overshoot are first scaled by the x-height factor.
///
/// The rules are fitted to the bar heights ttfautohint 1.8.4 gives synthetic fonts of one standard
/// width, at 9 to 100 PPM and widths of 0.1 to 15 pixels; its manual describes the modes only in
/// words. With `d` the distance and `s` the standard width, in pixels:
///
/// * natural: `d`.
/// * quantized: `d` when `s` is under 5/8; else `d` raised to 7/8, then `s` (at least 3/4) within
///   5/8 of it; else, under 3 pixels, the whole part of `d` plus 5/32 (a fraction from 5/32 to
///   1/2) or 27/32 (from 1/2 to 27/32), other fractions kept; from 3 pixels up `d` rounded.
/// * strong: the rounded `s` when `d` lies within 3/4 of it, else `d` rounded; at least 1.
///
/// The overshoots follow the rows ttfautohint gives synthetic fonts with overshoots of 0.5% to 3%
/// of the em at 8 to 200 PPM: an overshoot under 1/2 pixel does not show, one under 3/4 shows as
/// 1/2, and one from 3/4 up as the nearest whole number of pixels.
pub fn autohint_fpgm() -> Result<Vec<u8>, HintingError> {
    use crate::tt_font::{FN_AUTO_OVERSHOOT, FN_AUTO_STEM, S_NEGATIVE};
    let scale = format!(
        "PUSH {S_X_FITTED}\nRS[ ]\nPUSH {S_X_SCALED}\nRS[ ]\nNEQ[ ]\nIF[ ]\nPUSH {S_X_FITTED}\nRS[ ]\nMUL[ ]\nPUSH {S_X_SCALED}\nRS[ ]\nDIV[ ]\nEIF[ ]"
    );
    let std = format!("PUSH {S_STEM_STANDARD}\nRS[ ]");
    let (strong, quantized) = (StemWidthMode::Strong.number(), StemWidthMode::Quantized.number());
    Ok(fontcraft_ttasm::assemble(&format!(
        "\
; {FN_AUTO_STEM}: origin target width
PUSH {FN_AUTO_STEM}
FDEF[ ]
DUP[ ]
PUSH 0
GTEQ[ ]
IF[ ]
RCVT[ ]
ABS[ ]
{scale}
EIF[ ]
PUSH {S_STEM_STANDARD}
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
{scale}
PUSH {S_STEM_MODE}
RS[ ]
PUSH {strong}
EQ[ ]
IF[ ]
DUP[ ]
{std}
PUSH 0
GTEQ[ ]
IF[ ]
POP[ ]
{std}
PUSH 32
ADD[ ]
FLOOR[ ]
DUP[ ]
PUSH 3
CINDEX[ ]
SUB[ ]
ABS[ ]
PUSH 48
GTEQ[ ]
IF[ ]
POP[ ]
DUP[ ]
EIF[ ]
EIF[ ]
SWAP[ ]
POP[ ]
PUSH 32
ADD[ ]
FLOOR[ ]
PUSH 64
MAX[ ]
EIF[ ]
PUSH {S_STEM_MODE}
RS[ ]
PUSH {quantized}
EQ[ ]
{std}
PUSH 0
LT[ ]
{std}
PUSH 40
GTEQ[ ]
OR[ ]
AND[ ]
IF[ ]
PUSH 56
MAX[ ]
DUP[ ]
{std}
SUB[ ]
ABS[ ]
PUSH 40
LT[ ]
{std}
PUSH 0
GTEQ[ ]
AND[ ]
IF[ ]
POP[ ]
{std}
PUSH 48
MAX[ ]
ELSE[ ]
DUP[ ]
PUSH 192
LT[ ]
IF[ ]
; the whole part, then the fraction
DUP[ ]
FLOOR[ ]
SWAP[ ]
PUSH 2
CINDEX[ ]
SUB[ ]
DUP[ ]
PUSH 10
GT[ ]
PUSH 2
CINDEX[ ]
PUSH 54
LT[ ]
AND[ ]
IF[ ]
PUSH 32
LT[ ]
IF[ ]
PUSH 10
ELSE[ ]
PUSH 54
EIF[ ]
EIF[ ]
ADD[ ]
ELSE[ ]
PUSH 32
ADD[ ]
FLOOR[ ]
EIF[ ]
EIF[ ]
EIF[ ]
PUSH {S_NEGATIVE}
RS[ ]
IF[ ]
NEG[ ]
EIF[ ]
ROLL[ ]
SRP0[ ]
MSIRP[0]
ENDF[ ]

; {FN_AUTO_OVERSHOOT}: overshoot -> overshoot shown
PUSH {FN_AUTO_OVERSHOOT}
FDEF[ ]
DUP[ ]
PUSH 0
LT[ ]
PUSH {S_NEGATIVE}
SWAP[ ]
WS[ ]
ABS[ ]
{scale}
DUP[ ]
PUSH 32
LT[ ]
IF[ ]
POP[ ]
PUSH 0
ELSE[ ]
DUP[ ]
PUSH 48
LT[ ]
IF[ ]
POP[ ]
PUSH 32
ELSE[ ]
PUSH 32
ADD[ ]
FLOOR[ ]
EIF[ ]
EIF[ ]
PUSH {S_NEGATIVE}
RS[ ]
IF[ ]
NEG[ ]
EIF[ ]
ENDF[ ]
"
    ))?)
}

/// Storage written by [`autohint_prep`]: the top of the x-height zone at this size, unrounded,
/// and its rounded row (26.6 pixels).
pub const S_X_SCALED: u8 = 11;
pub const S_X_FITTED: u8 = 12;
/// Storage written by [`autohint_prep`]: the stem width mode of the rendering target
/// ([`StemWidthMode::number`]).
pub const S_STEM_MODE: u8 = 13;
/// A local of the autohinter's stem function ([`autohint_fpgm`]).
const S_STEM_STANDARD: u8 = 14;
/// What the x-height zone's scaled top gets added before it is floored to a row: 5/8 of a pixel,
/// and 13/16 from 6 PPM up to the x-height increase limit (26.6).
const X_HEIGHT_ROUND_UP: i32 = 40;
const X_HEIGHT_INCREASE_ROUND_UP: i32 = 52;

/// The `prep` text that turns each zone's overshoot entry into the row of its round extremes
/// ([`autohint_prep`]).
fn overshoot_rows(settings: &HintSettings, layout: &crate::CvtLayout) -> String {
    use crate::tt_font::FN_AUTO_OVERSHOOT;
    use std::fmt::Write;
    let mut t = String::new();
    if settings.tt_overshoot_suppression_below_ppm.is_some() {
        return t;
    }
    for (k, z) in settings.tt_zones.iter().enumerate().filter(|(_, z)| z.size != 0.0) {
        let (Some((pos, fit)), Some(end)) = (layout.zones.get(k).copied(), layout.overshoot(k)) else { continue };
        let _ = writeln!(
            t,
            "; overshoot of zone {}\nPUSH {end} {fit}\nRCVT[ ]\nPUSH {end}\nRCVT[ ]\nPUSH {pos}\nRCVT[ ]\nSUB[ ]\nPUSH {FN_AUTO_OVERSHOOT}\nCALL[ ]\nADD[ ]\nWCVTP[ ]",
            z.name.chars().filter(char::is_ascii_alphanumeric).collect::<String>()
        );
    }
    t
}

/// The `prep` text of the x-height scaling ([`autohint_prep`]). With `scale_x_height` off, and
/// without a usable `xHeight` zone or room in `cvt ` for its overshoot entry, the factor is 1: the
/// zones are only rounded to the nearest row.
fn x_height_scaling(
    options: &TtAutohintOptions,
    settings: &HintSettings,
    layout: &crate::CvtLayout,
    exceptions: &[(u16, u16)],
    scale_x_height: bool,
) -> String {
    use crate::tt_font::{FN_ROUND, MAX_ZONE_THRESHOLD_PX};
    use std::fmt::Write;
    let zones = &settings.tt_zones;
    let word = |v: f64| -> Option<i32> { (v.is_finite() && (1.0..=32_767.0).contains(&v.round())).then(|| v.round() as i32) };
    // The overshoot entry of the x-height zone. A zone at or below the baseline cannot scale the
    // others.
    let top = zones
        .iter()
        .enumerate()
        .find(|(_, z)| scale_x_height && z.name == "xHeight" && z.align.is_none() && z.size >= 0.0)
        .filter(|(_, x)| word(x.position).is_some() && word(x.position + x.size).is_some())
        .and_then(|(k, _)| layout.overshoot(k));
    let in_range = |lo: u16, hi: u16| format!("MPPEM[ ]\nPUSH {lo}\nGTEQ[ ]\nMPPEM[ ]\nPUSH {hi}\nLTEQ[ ]\nAND[ ]\nIF[ ]");
    // The cvt entry our program rounded zone `k` from: the family zone up to the family
    // threshold, else the zone's own position.
    let source = |k: usize| -> Option<String> {
        let (pos, _) = layout.zones.get(k).copied()?;
        Some(match (settings.tt_family_zones_threshold.map(|p| p.min(MAX_OPTION_PPM)), layout.family.get(k)) {
            (Some(until), Some(family)) => format!("MPPEM[ ]\nPUSH {until}\nLTEQ[ ]\nIF[ ]\nPUSH {family}\nELSE[ ]\nPUSH {pos}\nEIF[ ]"),
            _ => format!("PUSH {pos}"),
        })
    };
    // The value on the stack scaled by the factor and rounded; only rounded at a factor of 1,
    // which MUL and DIV would leave up to 1/64 pixel short.
    let scaled = format!(
        "PUSH {S_X_FITTED}\nRS[ ]\nPUSH {S_X_SCALED}\nRS[ ]\nNEQ[ ]\nIF[ ]\nPUSH {S_X_FITTED}\nRS[ ]\nMUL[ ]\nPUSH {S_X_SCALED}\nRS[ ]\nDIV[ ]\nEIF[ ]\nPUSH {FN_ROUND}\nCALL[ ]"
    );

    let mut t = String::new();
    if let Some(entry) = top {
        let _ = writeln!(t, "; x-height scaling\nPUSH {S_X_SCALED} {entry}\nRCVT[ ]\nWS[ ]");
        let _ = writeln!(t, "PUSH {S_X_FITTED} {S_X_SCALED}\nRS[ ]\nPUSH {X_HEIGHT_ROUND_UP}\nADD[ ]");
    } else {
        let _ = writeln!(t, "; no x-height scaling\nPUSH {S_X_SCALED} 64\nWS[ ]\nPUSH {S_X_FITTED} 64");
    }
    if top.is_some() && options.increase_x_height >= 6 {
        let _ = writeln!(
            t,
            "{}\nPUSH {}\nADD[ ]\nEIF[ ]",
            in_range(6, options.increase_x_height.min(MAX_OPTION_PPM)),
            X_HEIGHT_INCREASE_ROUND_UP - X_HEIGHT_ROUND_UP
        );
    }
    if top.is_some() {
        t.push_str("FLOOR[ ]\n");
        for &(lo, hi) in exceptions {
            let _ = writeln!(t, "{}\nPOP[ ]\nPUSH {S_X_SCALED}\nRS[ ]\nEIF[ ]", in_range(lo, hi));
        }
    }
    let _ = writeln!(t, "WS[ ]\nPUSH {S_X_SCALED}\nRS[ ]\nPUSH 0\nGT[ ]\nPUSH {S_X_FITTED}\nRS[ ]\nPUSH 0\nGT[ ]\nAND[ ]\nIF[ ]");

    // Alignment depth of each zone: 0 for a zone rounded on its own, `None` in or below a cycle.
    let target = |k: usize| -> Option<usize> {
        let name = zones.get(k)?.align.as_deref()?;
        zones.iter().position(|z| z.name == name).filter(|&j| j != k)
    };
    let depth: Vec<Option<usize>> = (0..zones.len())
        .map(|start| {
            let mut k = start;
            for d in 0..=zones.len() {
                match target(k) {
                    Some(j) => k = j,
                    None => return Some(d),
                }
            }
            None
        })
        .collect();
    let mut aligned: Vec<(usize, usize, usize)> =
        (0..zones.len()).filter_map(|k| Some((depth.get(k).copied().flatten().filter(|d| *d > 0)?, k, target(k)?))).collect();
    aligned.sort();
    // FN_ZONE_ALIGN's placement of zone `k` on zone `j`: the target's row plus the rounded distance.
    let placement = |k: usize, j: usize| -> Option<String> {
        let ((pos, _), (tpos, tfit)) = (layout.zones.get(k).copied()?, layout.zones.get(j).copied()?);
        Some(format!("PUSH {tfit}\nRCVT[ ]\nPUSH {pos}\nRCVT[ ]\nPUSH {tpos}\nRCVT[ ]\nSUB[ ]\nPUSH {FN_ROUND}\nCALL[ ]\nADD[ ]"))
    };
    // Aligned zones lose their placement first, dependents before their targets, which keeps
    // only their deltas; they get it back from the scaled targets at the end.
    for &(_, k, j) in aligned.iter().rev() {
        let (Some((_, fit)), Some(p)) = (layout.zones.get(k).copied(), placement(k, j)) else { continue };
        let _ = writeln!(t, "PUSH {fit} {fit}\nRCVT[ ]\n{p}\nSUB[ ]\nWCVTP[ ]");
    }
    let threshold = if settings.tt_zone_rounding_threshold.is_finite() {
        settings.tt_zone_rounding_threshold.clamp(-MAX_ZONE_THRESHOLD_PX, MAX_ZONE_THRESHOLD_PX)
    } else {
        0.0
    };
    for (k, z) in zones.iter().enumerate() {
        if depth.get(k).copied().flatten() != Some(0) {
            continue;
        }
        let (Some((_, fit)), Some(src)) = (layout.zones.get(k).copied(), source(k)) else { continue };
        // The rounding of our own `prep` (`FN_ZONE_ROUND`), replaced by the scaled row.
        let times = if z.name == "xHeight" { 2.0 } else { 1.0 };
        let thr = if z.size > 0.0 { (threshold * times * 64.0).round() as i32 } else { 0 };
        let _ = writeln!(
            t,
            "; zone {k}\nPUSH {fit} {fit}\nRCVT[ ]\n{src}\nRCVT[ ]\n{scaled}\n{src}\nRCVT[ ]\nPUSH {}\nADD[ ]\nFLOOR[ ]\nSUB[ ]\nADD[ ]\nWCVTP[ ]",
            thr + 32
        );
    }
    for &(_, k, j) in &aligned {
        let (Some((_, fit)), Some(p)) = (layout.zones.get(k).copied(), placement(k, j)) else { continue };
        let _ = writeln!(t, "PUSH {fit} {fit}\nRCVT[ ]\n{p}\nADD[ ]\nWCVTP[ ]");
    }
    for (k, (s, &(width, fit))) in settings.tt_stems.iter().zip(&layout.stems).enumerate() {
        if s.horizontal {
            let _ = writeln!(
                t,
                "; stem {k}\nPUSH {fit} {fit}\nRCVT[ ]\nPUSH {width}\nRCVT[ ]\n{scaled}\nPUSH {width}\nRCVT[ ]\nPUSH 32\nADD[ ]\nFLOOR[ ]\nSUB[ ]\nADD[ ]\nWCVTP[ ]"
            );
        }
    }
    t.push_str("EIF[ ]\n");
    t
}

/// The letters the ttfautohint manual lists for its Latin x-height zone and cap-height zone.
const X_HEIGHT_LETTERS: [char; 8] = ['u', 'v', 'x', 'z', 'o', 'e', 's', 'c'];
const CAP_HEIGHT_LETTERS: [char; 8] = ['T', 'H', 'E', 'Z', 'O', 'C', 'Q', 'S'];

/// What [`drop_unmeasured_zones`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeasuredZones {
    /// The zones left out.
    pub left_out: Vec<String>,
    /// The font draws a letter of the x-height zone, so its zones scale with the x-height
    /// ([`autohint_prep`]).
    pub x_height: bool,
}

/// Leaves out of `settings` the zones the autohinter has no letters for, as ttfautohint, which
/// measures its zones on those letters, finds none: the `xHeight` zone when no glyph for u, v, x,
/// z, o, e, s or c has an outline on the master, the `capHeight` zone when none for T, H, E, Z, O,
/// C, Q or S has. A zone that holds the top of a letter of the other list stays, since ttfautohint
/// measures a zone at that height (a font whose capitals are as tall as its x-height).
pub fn drop_unmeasured_zones(font: &Font, settings: &mut HintSettings) -> MeasuredZones {
    let tops = |letters: &[char]| -> Vec<f64> {
        font.glyphs
            .iter()
            .filter(|g| g.unicodes.iter().any(|u| char::from_u32(*u).is_some_and(|c| letters.contains(&c))))
            .filter(|g| g.layer(&settings.master).is_some_and(|l| !l.shapes.is_empty()))
            .map(|g| font.outline_of(g, &settings.master).ok().and_then(|o| outline_top(&o)).unwrap_or(f64::NAN))
            .collect()
    };
    let (lower, upper) = (tops(&X_HEIGHT_LETTERS), tops(&CAP_HEIGHT_LETTERS));
    let fuzz = if settings.tt_blue_fuzz.is_finite() { settings.tt_blue_fuzz.max(0.0) } else { 0.0 };
    let mut gone = Vec::new();
    for (name, own, other) in [("xHeight", &lower, &upper), ("capHeight", &upper, &lower)] {
        let Some(z) = settings.tt_zones.iter().find(|z| z.name == name) else { continue };
        let (lo, hi) = (z.position.min(z.position + z.size) - fuzz, z.position.max(z.position + z.size) + fuzz);
        if own.is_empty() && !other.iter().any(|y| *y >= lo && *y <= hi) {
            gone.push(name.to_string());
        }
    }
    settings.tt_zones.retain(|z| !gone.contains(&z.name));
    MeasuredZones { left_out: gone, x_height: !lower.is_empty() }
}

/// Prepares `settings` for an autohinted build: leaves out the zones without letters
/// ([`drop_unmeasured_zones`]) and, when the master has no horizontal TT stem, adds one of the
/// standard width ([`standard_stem`]). Every master of a variable font gets the same entries.
pub fn prepare_settings(font: &Font, settings: &mut HintSettings, options: &TtAutohintOptions) -> MeasuredZones {
    let measured = drop_unmeasured_zones(font, settings);
    if !settings.tt_stems.iter().any(|s| s.horizontal && s.size.is_finite() && s.size > 0.0) {
        let size = standard_stem(font, settings, options);
        settings.tt_stems.push(crate::TtStem { name: "autohint standard".into(), horizontal: true, size, ..Default::default() });
    }
    measured
}

/// The standard horizontal stem width of a master without one, as the ttfautohint manual derives
/// it: the width of the thinnest horizontal stem of `o`; without a drawn `o`, and for a symbol
/// font, the fallback stem width (by default 50 units at 2048 units per em).
fn standard_stem(font: &Font, settings: &HintSettings, options: &TtAutohintOptions) -> f64 {
    let fallback = options.fallback_stem_width.map_or((f64::from(font.units_per_em) * 50.0 / 2048.0).round(), f64::from);
    let fallback = if fallback.is_finite() && fallback >= 1.0 { fallback } else { 1.0 };
    if options.symbol {
        return fallback;
    }
    let measured = || -> Option<f64> {
        let g = font.glyphs.iter().find(|g| g.unicodes.contains(&u32::from('o')))?;
        let layer = g.layer(&settings.master)?;
        let control = ControlInstructions::default();
        let input = AutoGlyph {
            glyph: &g.name,
            layer,
            settings,
            zones: None,
            stems: Some(&[]),
            options,
            control: &control,
            points: None,
            delta_column: None,
        };
        let (edges, _) = ps_stems(&input, &[], &mut Vec::new());
        edges.iter().map(|(lo, hi)| (hi - lo).round()).filter(|w| *w >= 1.0 && w.is_finite()).reduce(f64::min)
    };
    measured().unwrap_or(fallback)
}

/// The highest point where a segment of `outline` starts or ends; `None` without segments.
fn outline_top(outline: &BezPath) -> Option<f64> {
    outline
        .elements()
        .iter()
        .filter_map(|el| match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(p.y),
            PathEl::ClosePath => None,
        })
        .reduce(f64::max)
}

/// One glyph layer to autohint.
pub struct AutoGlyph<'a> {
    pub glyph: &'a str,
    /// The layer whose outline is analysed and whose nodes the instructions name.
    pub layer: &'a Layer,
    pub settings: &'a HintSettings,
    /// The TT zones and stems that apply to the glyph (indexes into the settings' lists);
    /// `None`: all of them.
    pub zones: Option<&'a [usize]>,
    pub stems: Option<&'a [usize]>,
    pub options: &'a TtAutohintOptions,
    pub control: &'a ControlInstructions,
    /// The exported point numbers, which control instructions address; `None`: points count
    /// the layer's nodes, paths in order.
    pub points: Option<&'a dyn PointMap>,
    /// The Delta column the `touch` and `point` corrections are written for; `None`: none.
    pub delta_column: Option<&'a str>,
}

/// The autohinter's result for one glyph layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoHints {
    pub instructions: Vec<TtInstruction>,
    /// The control instructions' corrections that run after IUP (`point` shifts, and the x
    /// shifts of `touch`), which no Instructor instruction holds: the export writes them with
    /// [`shift_program`], the commands that store instructions with [`shifts_as_deltas`].
    pub shifts: Vec<PointShift>,
    /// What the analysis skipped or could not use, for the report.
    pub warnings: Vec<String>,
}

/// A move of one node along one axis by `amount` (26.6 pixels, a multiple of an eighth of a
/// pixel, at most one pixel either way) at each of `ppems`.
#[derive(Clone, Debug, PartialEq)]
pub struct PointShift {
    pub node: NodeRef,
    pub direction: Direction,
    pub ppems: Vec<u16>,
    pub amount: i32,
}

/// TrueType instructions for one glyph's main-master layer, replacing its manual ones.
pub fn autohint_tt(
    font: &Font,
    glyph: &str,
    settings: &HintSettings,
    options: &TtAutohintOptions,
    control: &ControlInstructions,
) -> Result<Vec<TtInstruction>, HintingError> {
    let g = font.glyph(glyph).ok_or_else(|| HintingError::MissingGlyph(glyph.to_string()))?;
    let layer =
        g.layer(&settings.main_master).ok_or_else(|| HintingError::MissingLayer { glyph: glyph.to_string(), layer: settings.main_master.clone() })?;
    let zones = settings.tt_zones_for(font, glyph);
    let stems = settings.tt_stems_for(font, glyph);
    let column = font.master(&settings.main_master).ok().map(|m| delta_column_key(&m.location));
    let input = AutoGlyph {
        glyph,
        layer,
        settings,
        zones: Some(&zones),
        stems: Some(&stems),
        options,
        control,
        points: None,
        delta_column: column.as_deref(),
    };
    let h = autohint_glyph(&input)?;
    let mut list = h.instructions;
    list.extend(shifts_as_deltas(&h.shifts, column.as_deref(), &mut Vec::new()));
    Ok(list)
}

/// A node of the layer as the analysis sees it.
#[derive(Clone, Copy, Debug)]
struct Pt {
    r: NodeRef,
    x: f64,
    y: f64,
    on: bool,
}

/// TrueType instructions for one glyph layer (the module doc lists what it writes).
pub fn autohint_glyph(input: &AutoGlyph) -> Result<AutoHints, HintingError> {
    let mut out = AutoHints::default();
    if input.options.dehint {
        return Ok(out);
    }
    let paths = layer_paths(input.layer, input.glyph)?;
    let all: Vec<Pt> = paths.iter().flatten().copied().collect();
    let control = GlyphControl::new(input, &paths, &mut out.warnings);

    let settings = input.settings;
    let zone_ids: Vec<usize> = match input.zones {
        Some(z) => z.to_vec(),
        None => (0..settings.tt_zones.len()).collect(),
    };
    let fuzz = if settings.tt_blue_fuzz.is_finite() { settings.tt_blue_fuzz.max(0.0) } else { 0.0 };
    // (bottom, top, is top zone) of each zone in force, overshoot included.
    let zones: Vec<(f64, f64, bool)> = zone_ids
        .iter()
        .filter_map(|i| settings.tt_zones.get(*i))
        .filter(|z| z.size != 0.0 && z.size.is_finite() && z.position.is_finite())
        .map(|z| (z.position.min(z.position + z.size), z.position.max(z.position + z.size), z.size > 0.0))
        .collect();

    let mut touched_y: BTreeSet<NodeRef> = BTreeSet::new();
    let mut list = Vec::new();

    // 1. Extremes in zones.
    for path in &paths {
        for (top, p) in extremes(path) {
            if control.nodir.contains(&p.r) {
                continue;
            }
            let inside = zones.iter().any(|&(lo, hi, is_top)| is_top == top && p.y >= lo - fuzz && p.y <= hi + fuzz);
            if inside && touched_y.insert(p.r) {
                list.push(TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, TtPoint::Node(p.r)));
            }
        }
    }

    // 2. Stems from the PostScript analysis.
    let stems = ps_stems(input, &zones, &mut out.warnings);
    let usable = |p: &Pt| p.on && !control.nodir.contains(&p.r);
    // Heights only, as the reference's autohinter does: a rasterizer in ClearType modes ignores
    // most horizontal moves, and leaving them out keeps the programs small.
    {
        let (direction, edges, touched) = (Direction::Vertical, &stems.0, &mut touched_y);
        let at = |v: f64| -> Vec<Pt> {
            all.iter()
                .filter(|p| usable(p) && (if direction == Direction::Vertical { p.y } else { p.x } - v).abs() <= EDGE_TOLERANCE)
                .copied()
                .collect()
        };
        let mut pending: Vec<(Pt, Pt)> = edges.iter().filter_map(|&(lo, hi)| closest_pair(&at(lo), &at(hi), direction)).collect();
        pending.retain(|(a, b)| a.r != b.r);
        while !pending.is_empty() {
            let anchored = pending.iter().position(|(a, b)| touched.contains(&a.r) || touched.contains(&b.r));
            let (a, b) = match anchored {
                Some(i) => pending.remove(i),
                None => pending.remove(0),
            };
            let (origin, target) = match (touched.contains(&a.r), touched.contains(&b.r)) {
                (true, true) => continue,
                (false, true) => (b, a),
                _ => (a, b),
            };
            let mut t = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, direction, TtPoint::Node(target.r));
            t.origin = Some(TtPoint::Node(origin.r));
            touched.insert(origin.r);
            touched.insert(target.r);
            list.push(t);
        }
    }

    // 3. The control instructions' left and right points: one-point segments, hinted as edges.
    let widest = settings
        .tt_stems
        .iter()
        .filter(|s| s.horizontal && s.size.is_finite())
        .map(|s| s.size)
        .chain(stems.0.iter().map(|(lo, hi)| hi - lo))
        .fold(0.0f64, f64::max);
    let runs = flat_runs(&paths);
    for &(r, a, b) in &control.segments {
        if control.nodir.contains(&r) || touched_y.contains(&r) {
            continue;
        }
        let Some(p) = all.iter().find(|q| q.r == r).copied() else { continue };
        if zones.iter().any(|&(lo, hi, _)| p.y >= lo - fuzz && p.y <= hi + fuzz) {
            touched_y.insert(r);
            list.push(TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, TtPoint::Node(r)));
            continue;
        }
        match segment_partner(&runs, &p, a, b, widest * SEGMENT_STEM_REACH, &touched_y, &control.nodir) {
            Some(q) => {
                let mut t = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Vertical, TtPoint::Node(r));
                t.origin = Some(TtPoint::Node(q.r));
                touched_y.insert(q.r);
                list.push(t);
            }
            None => list.push(TtInstruction::new(TtKind::Snap { zone: ZoneRef::NoZone }, Direction::Vertical, TtPoint::Node(r))),
        }
        touched_y.insert(r);
    }

    // 4. The `touch` corrections.
    list.extend(control.deltas(input.delta_column, &mut out.warnings));
    out.shifts = control.after_iup;
    if list.len() > crate::MAX_LAYER_HINTS {
        return Err(HintingError::LimitExceeded { what: "instructions on a layer", limit: crate::MAX_LAYER_HINTS });
    }
    out.instructions = list;
    Ok(out)
}

/// The layer's closed paths as nodes, model order; open paths are left out. Every coordinate
/// is checked finite.
fn layer_paths(layer: &Layer, glyph: &str) -> Result<Vec<Vec<Pt>>, HintingError> {
    let mut out = Vec::new();
    let mut count = 0usize;
    for (pi, path) in layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).enumerate() {
        count = count.saturating_add(path.nodes.len());
        if count > MAX_AUTOHINT_NODES {
            return Err(HintingError::LimitExceeded { what: "nodes in a glyph to autohint", limit: MAX_AUTOHINT_NODES });
        }
        if path.nodes.iter().any(|n| !n.x.is_finite() || !n.y.is_finite()) {
            return Err(HintingError::Glyph { glyph: glyph.to_string(), message: "a node's coordinate is not a number".into() });
        }
        if !path.closed || path.nodes.len() < 2 {
            out.push(Vec::new());
            continue;
        }
        out.push(path.nodes.iter().enumerate().map(|(ni, n)| Pt { r: NodeRef::new(pi, ni), x: n.x, y: n.y, on: n.on_curve() }).collect());
    }
    Ok(out)
}

/// The y extremes of one closed path: the on-curve nodes of each flat run whose neighbours on
/// both sides lie below it (`true`, a top) or above it (`false`). A run made of off-curve nodes
/// only gives none.
fn extremes(path: &[Pt]) -> Vec<(bool, Pt)> {
    let n = path.len();
    let same = |a: &Pt, b: &Pt| (a.y - b.y).abs() <= EDGE_TOLERANCE;
    // Start right after a node whose height differs from the next one's, so no run wraps.
    let Some(start) = (0..n).find(|&i| match (path.get(i), path.get((i + 1) % n)) {
        (Some(a), Some(b)) => !same(a, b),
        _ => false,
    }) else {
        return Vec::new();
    };
    let at = |k: usize| path.get((start + 1 + k) % n);
    let mut out = Vec::new();
    let mut k = 0;
    while k < n {
        let Some(first) = at(k) else { break };
        let mut end = k;
        while end + 1 < n && at(end + 1).is_some_and(|p| same(first, p)) {
            end += 1;
        }
        if let (Some(b), Some(a)) = (at((k + n - 1) % n), at((end + 1) % n)) {
            let top = b.y < first.y && a.y < first.y;
            let bottom = b.y > first.y && a.y > first.y;
            if top || bottom {
                out.extend((k..=end).filter_map(at).filter(|p| p.on).map(|p| (top, *p)));
            }
        }
        k = end + 1;
    }
    out
}

/// The two nodes, one from each edge, that lie closest together across the stem (same path
/// first), as (low edge, high edge).
fn closest_pair(lo: &[Pt], hi: &[Pt], direction: Direction) -> Option<(Pt, Pt)> {
    let across = |a: &Pt, b: &Pt| if direction == Direction::Vertical { (a.x - b.x).abs() } else { (a.y - b.y).abs() };
    let mut best: Option<((bool, f64), Pt, Pt)> = None;
    for a in lo.iter().take(MAX_EDGE_CANDIDATES) {
        for b in hi.iter().take(MAX_EDGE_CANDIDATES) {
            let key = (a.r.path != b.r.path, across(a, b));
            if best.as_ref().is_none_or(|(k, _, _)| (!key.0 && k.0) || (key.0 == k.0 && key.1 < k.1)) {
                best = Some((key, *a, *b));
            }
        }
    }
    best.map(|(_, a, b)| (a, b))
}

/// How far from a `left` or `right` point, as a multiple of the glyph's widest horizontal stem,
/// another segment may lie and still form a stem with it.
const SEGMENT_STEM_REACH: f64 = 1.5;

/// The flat stretches of the closed paths: (left x, right x, y, its on-curve nodes) of each run
/// of on-curve nodes at one height, a single on-curve node counting as a run of its own.
fn flat_runs(paths: &[Vec<Pt>]) -> Vec<(f64, f64, f64, Vec<Pt>)> {
    let mut out = Vec::new();
    for path in paths {
        let n = path.len();
        for (i, p) in path.iter().enumerate() {
            if !p.on {
                continue;
            }
            out.push((p.x, p.x, p.y, vec![*p]));
            if let Some(q) = path.get((i + 1) % n).filter(|q| q.on && q.r != p.r && (q.y - p.y).abs() <= EDGE_TOLERANCE) {
                out.push((p.x.min(q.x), p.x.max(q.x), p.y, vec![*p, *q]));
            }
        }
    }
    out
}

/// The node a `left` or `right` point at `p`, whose segment spans `a` to `b`, forms a stem with:
/// a node of the nearest flat run above or below that overlaps the segment, within `reach` font
/// units, a node already touched first. `None` when no run qualifies.
fn segment_partner(
    runs: &[(f64, f64, f64, Vec<Pt>)],
    p: &Pt,
    a: f64,
    b: f64,
    reach: f64,
    touched: &BTreeSet<NodeRef>,
    nodir: &BTreeSet<NodeRef>,
) -> Option<Pt> {
    let mut best: Option<(f64, bool, Pt)> = None;
    for (lo, hi, y, nodes) in runs {
        let d = (y - p.y).abs();
        if d <= EDGE_TOLERANCE || d > reach || *hi < a || *lo > b || nodes.iter().any(|q| q.r == p.r) {
            continue;
        }
        let pick = nodes.iter().filter(|q| !nodir.contains(&q.r)).min_by(|q, r| {
            (!touched.contains(&q.r), (q.x - p.x).abs())
                .partial_cmp(&(!touched.contains(&r.r), (r.x - p.x).abs()))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let Some(q) = pick.copied() else { continue };
        let key = (d, !touched.contains(&q.r));
        if best.as_ref().is_none_or(|(bd, bt, _)| key.0 < *bd - EDGE_TOLERANCE || ((key.0 - bd).abs() <= EDGE_TOLERANCE && !key.1 && *bt)) {
            best = Some((key.0, key.1, q));
        }
    }
    best.map(|(_, _, q)| q)
}

/// The PostScript autohinter's stems of the layer: horizontal stems as (bottom, top) and
/// vertical stems as (left, right), ghosts left out. Only the horizontal ones are used.
type StemEdges = (Vec<(f64, f64)>, Vec<(f64, f64)>);

fn ps_stems(input: &AutoGlyph, zones: &[(f64, f64, bool)], warnings: &mut Vec<String>) -> StemEdges {
    let contours: Vec<BezPath> = input
        .layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) if p.closed && p.nodes.len() > 1 => p.to_bez().ok(),
            _ => None,
        })
        .map(|b| cubic(&b))
        .collect();
    if contours.is_empty() {
        return StemEdges::default();
    }
    let settings = input.settings;
    let stem_ids: Vec<usize> = match input.stems {
        Some(s) => s.to_vec(),
        None => (0..settings.tt_stems.len()).collect(),
    };
    let mut stems = StemSet::default();
    for s in stem_ids.iter().filter_map(|i| settings.tt_stems.get(*i)).filter(|s| s.size.is_finite() && s.size > 0.0) {
        if s.horizontal { stems.h.push(s.size) } else { stems.v.push(s.size) }
    }
    if stems.v.is_empty() {
        stems.v.extend(input.options.fallback_stem_width.map(f64::from));
    }
    let Some(dict) = fd_dict(input.glyph, zones, &stems) else {
        warnings.push(format!("glyph `{}`: its zones and stems cannot be used for the stem analysis", input.glyph));
        return StemEdges::default();
    };
    let options = PsOptions { no_flex: true, no_hint_sub: true, allow_no_blues: true, explicit_glyphs: true, ..PsOptions::default() };
    let hints = match hint_glyph(&HintInput { name: input.glyph, contours: &contours, without_overlap: None, dict: &dict, options: &options }) {
        Ok(h) => h,
        Err(e) => {
            warnings.push(format!("glyph `{}`: the stem analysis failed ({e}); only its zones were hinted", input.glyph));
            return StemEdges::default();
        }
    };
    let pairs = |list: &[fontcraft_autohint::Stem]| {
        list.iter().filter(|s| s.rt > s.lb && s.lb.is_finite() && s.rt.is_finite()).map(|s| (s.lb, s.rt)).collect()
    };
    (pairs(&hints.h), pairs(&hints.v))
}

/// `path` with every quadratic segment raised to a cubic, as the PostScript analysis reads it.
fn cubic(path: &BezPath) -> BezPath {
    let mut out = Vec::with_capacity(path.elements().len());
    let mut current = fontcraft_geom::Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::QuadTo(q, p) => {
                out.push(PathEl::CurveTo(current + (q - current) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p));
                current = p;
            }
            PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::CurveTo(_, _, p) => {
                out.push(*el);
                current = p;
            }
            PathEl::ClosePath => out.push(*el),
        }
    }
    BezPath::from_vec(out)
}

/// The analysis' zone set: the highest bottom zone is the baseline zone, with the top zones in
/// BlueValues; the other bottom zones are OtherBlues. When those cannot be used, the analysis
/// runs without zones, then without stems too.
fn fd_dict(name: &str, zones: &[(f64, f64, bool)], stems: &StemSet) -> Option<FdDict> {
    let mut bottoms: Vec<Zone> = Vec::new();
    let mut tops: Vec<Zone> = Vec::new();
    for &(bottom, top, is_top) in zones {
        let list = if is_top { &mut tops } else { &mut bottoms };
        if !list.iter().any(|z| z.bottom == bottom && z.top == top) {
            list.push(Zone { bottom, top, is_bottom: !is_top, name: String::new() });
        }
    }
    bottoms.sort_by(|a, b| b.top.total_cmp(&a.top));
    tops.sort_by(|a, b| a.bottom.total_cmp(&b.bottom));
    let mut blue_values: Vec<Zone> = Vec::new();
    let mut other_blues = Vec::new();
    for (i, z) in bottoms.into_iter().enumerate() {
        if i == 0 { blue_values.push(z) } else { other_blues.push(z) }
    }
    blue_values.extend(tops);
    other_blues.reverse();
    let set = ZoneSet { blue_values, other_blues, blue_fuzz: 0.0, ..ZoneSet::default() };
    [(&set, stems), (&ZoneSet::default(), stems), (&ZoneSet::default(), &StemSet::default())]
        .into_iter()
        .find_map(|(z, s)| FdDict::new(name, z, s, true).ok())
}

/// The control instructions of one glyph, resolved to nodes.
#[derive(Default)]
struct GlyphControl {
    nodir: BTreeSet<NodeRef>,
    /// (node, segment start, segment end) of each `left` and `right` point: its x plus the
    /// offsets, font units, start not after end.
    segments: Vec<(NodeRef, f64, f64)>,
    /// (node, PPMs, y shift in pixels) of each `touch` correction, in text order.
    touch: Vec<(NodeRef, Vec<u16>, f64)>,
    /// The corrections written after IUP: `point` shifts and the x shifts of `touch`.
    after_iup: Vec<PointShift>,
}

impl GlyphControl {
    fn new(input: &AutoGlyph, paths: &[Vec<Pt>], warnings: &mut Vec<String>) -> Self {
        let mut out = Self::default();
        let lines: Vec<_> = input.control.lines.iter().filter(|l| l.font_index == 0 && l.glyph == input.glyph).collect();
        if lines.is_empty() {
            return out;
        }
        // Exported point number → node.
        let mut by_point: HashMap<u32, Pt> = HashMap::new();
        let mut k = 0u32;
        for p in paths.iter().flatten() {
            let n = match input.points {
                Some(map) => map.point(p.r).map(u32::from),
                None => Some(k),
            };
            k = k.saturating_add(1);
            if let Some(n) = n {
                by_point.entry(n).or_insert(*p);
            }
        }
        let mut cells_left = MAX_CONTROL_CELLS;
        let mut over = false;
        for line in lines {
            let mut nodes = Vec::with_capacity(line.points.len());
            for i in &line.points {
                match by_point.get(i) {
                    Some(p) => nodes.push(*p),
                    None => warnings.push(format!("control instructions, line {}: glyph `{}` has no point {i}", line.line, input.glyph)),
                }
            }
            match &line.kind {
                ControlKind::NoDir => out.nodir.extend(nodes.iter().map(|p| p.r)),
                ControlKind::Left { left_offset, right_offset } | ControlKind::Right { left_offset, right_offset } => {
                    let (a, b) = (f64::from(*left_offset.min(right_offset)), f64::from(*left_offset.max(right_offset)));
                    for p in nodes {
                        if out.segments.len() >= MAX_CONTROL_SEGMENTS {
                            warnings.push(format!(
                                "control instructions, line {}: glyph `{}` has more than {MAX_CONTROL_SEGMENTS} left and right points; the rest were left out",
                                line.line, input.glyph
                            ));
                            break;
                        }
                        out.segments.push((p.r, p.x + a, p.x + b));
                    }
                }
                // Each node holds its own copy of the sizes, once for a `touch` and once for each
                // axis of a `point`.
                ControlKind::Touch { .. } | ControlKind::Point { .. }
                    if !take_cells(&mut cells_left, nodes.len().saturating_mul(line.ppems.len().max(1)).saturating_mul(2)) =>
                {
                    if !over {
                        warnings.push(format!(
                            "control instructions, line {}: glyph `{}` has more than {MAX_CONTROL_CELLS} correction cells (points times sizes); the rest were left out",
                            line.line, input.glyph
                        ));
                        over = true;
                    }
                }
                ControlKind::Touch { x_shift, y_shift } => {
                    for p in &nodes {
                        out.touch.push((p.r, line.ppems.clone(), *y_shift));
                    }
                    out.after_iup.extend(shifts(&nodes, Direction::Horizontal, *x_shift, &line.ppems));
                }
                ControlKind::Point { x_shift, y_shift } => {
                    out.after_iup.extend(shifts(&nodes, Direction::Horizontal, *x_shift, &line.ppems));
                    out.after_iup.extend(shifts(&nodes, Direction::Vertical, *y_shift, &line.ppems));
                }
            }
        }
        out
    }

    /// One vertical Delta per node with `touch` corrections, cells by the sign of its shift.
    fn deltas(&self, column: Option<&str>, warnings: &mut Vec<String>) -> Vec<TtInstruction> {
        if self.touch.is_empty() {
            return Vec::new();
        }
        let Some(column) = column else {
            warnings.push("control instructions: the corrections have no Delta column here and were left out".into());
            return Vec::new();
        };
        let mut per_node: BTreeMap<NodeRef, BTreeMap<u16, DeltaDir>> = BTreeMap::new();
        for (r, ppems, shift) in &self.touch {
            if shift.abs() < MIN_DELTA_SHIFT {
                continue;
            }
            let dir = if *shift > 0.0 { DeltaDir::Up } else { DeltaDir::Down };
            let cells = per_node.entry(*r).or_default();
            for p in ppems.iter().filter(|p| (1..=crate::MAX_DELTA_PPM).contains(*p)) {
                cells.insert(*p, dir);
            }
        }
        delta_instructions(per_node, column)
    }
}

/// Vertical Delta instructions, one per node, in the Delta column `column`.
fn delta_instructions(per_node: BTreeMap<NodeRef, BTreeMap<u16, DeltaDir>>, column: &str) -> Vec<TtInstruction> {
    per_node
        .into_iter()
        .filter(|(_, cells)| !cells.is_empty())
        .map(|(r, cells)| {
            let mut t =
                TtInstruction::new(TtKind::Delta { cells: BTreeMap::from([(column.to_string(), cells)]) }, Direction::Vertical, TtPoint::Node(r));
            t.rounding = Rounding::NoRounding;
            t
        })
        .collect()
}

/// The shifts of `nodes` along one axis, in eighths of a pixel; none for a shift that rounds to 0.
fn shifts(nodes: &[Pt], direction: Direction, pixels: f64, ppems: &[u16]) -> Vec<PointShift> {
    let eighths = if pixels.is_finite() { (pixels.clamp(-1.0, 1.0) * 8.0).round() as i32 } else { 0 };
    if eighths == 0 {
        return Vec::new();
    }
    let ppems: Vec<u16> = ppems.iter().copied().filter(|p| (1..=crate::MAX_DELTA_PPM).contains(p)).collect();
    nodes.iter().map(|p| PointShift { node: p.r, direction, ppems: ppems.clone(), amount: eighths * 8 }).collect()
}

/// Takes `n` from `left`; `false` (and nothing left) when there is not enough.
fn take_cells(left: &mut usize, n: usize) -> bool {
    match left.checked_sub(n) {
        Some(rest) => {
            *left = rest;
            true
        }
        None => {
            *left = 0;
            false
        }
    }
}

/// Most cells (a point at a size, per axis) of one glyph's `touch` and `point` corrections the
/// autohinter holds; the program after IUP takes [`MAX_SHIFT_CELLS`] of them.
const MAX_CONTROL_CELLS: usize = 1 << 16;
/// Most `left` and `right` points of one glyph the autohinter looks at; each costs a pass over
/// the glyph's nodes.
const MAX_CONTROL_SEGMENTS: usize = 256;
/// Most shift cells (node and size) written after IUP in one glyph program.
pub const MAX_SHIFT_CELLS: usize = 8_192;

/// The program the export appends to a glyph's autohinted program, after its IUP: each shift as a
/// move of its point at each of its sizes. Empty when there are none; shifts whose node has no
/// exported point are left out.
pub fn shift_program(shifts: &[PointShift], points: &dyn PointMap) -> Result<Vec<u8>, HintingError> {
    use std::fmt::Write;
    let mut t = String::new();
    let mut cells = 0usize;
    for direction in [Direction::Vertical, Direction::Horizontal] {
        let mut axis_set = false;
        for s in shifts.iter().filter(|s| s.direction == direction) {
            let Some(point) = points.point(s.node) else { continue };
            for ppm in &s.ppems {
                cells += 1;
                if cells > MAX_SHIFT_CELLS {
                    return Err(HintingError::LimitExceeded { what: "control-instruction shift cells in a glyph", limit: MAX_SHIFT_CELLS });
                }
                if !axis_set {
                    t.push_str(if direction == Direction::Vertical { "SVTCA[0]\n" } else { "SVTCA[1]\n" });
                    axis_set = true;
                }
                let _ = writeln!(t, "PUSH {point} {ppm} {} {}\nCALL[ ]", s.amount.clamp(-64, 64), crate::tt_font::FN_POINT_DELTA);
            }
        }
    }
    if t.is_empty() {
        return Ok(Vec::new());
    }
    Ok(fontcraft_ttasm::assemble(&t)?)
}

/// The shifts of `shifts` as the Instructor stores them: a vertical Delta at each size, one cell
/// up or down by the sign of a shift of at least half a pixel. Horizontal shifts and smaller ones
/// give none and are reported.
pub fn shifts_as_deltas(shifts: &[PointShift], column: Option<&str>, warnings: &mut Vec<String>) -> Vec<TtInstruction> {
    if shifts.is_empty() {
        return Vec::new();
    }
    let Some(column) = column else {
        warnings.push("control instructions: the corrections have no Delta column here and were left out".into());
        return Vec::new();
    };
    let mut per_node: BTreeMap<NodeRef, BTreeMap<u16, DeltaDir>> = BTreeMap::new();
    let mut dropped = false;
    for s in shifts {
        if s.direction != Direction::Vertical || s.amount.abs() < 32 {
            dropped = true;
            continue;
        }
        let dir = if s.amount > 0 { DeltaDir::Up } else { DeltaDir::Down };
        let cells = per_node.entry(s.node).or_default();
        for p in &s.ppems {
            cells.insert(*p, dir);
        }
    }
    if dropped {
        warnings.push("control instructions: horizontal shifts and shifts under half a pixel are written only by an autohinted export, not stored as Delta instructions".into());
    }
    delta_instructions(per_node, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tt_testfont as tt;

    #[test]
    fn options_read_long_and_short_forms_and_report_the_ignored_ones() {
        let (o, ignored) =
            TtAutohintOptions::parse("--hinting-limit=120 -x 0 --x-height-snapping-exceptions 8-9,12 -W -t --stem-width-mode=nnn -c -H50").unwrap();
        assert_eq!((o.hinting_limit, o.increase_x_height, o.fallback_stem_width), (120, 0, Some(50)));
        assert!(o.windows_compatibility && o.ttfa_table && o.hint_composites);
        assert!(ignored.is_empty(), "{ignored:?}");
        assert_eq!(o.export_only(), ["stem-width-mode", "composites"]);
        assert_eq!(TtAutohintOptions::parse("-p -i -l 10 -r 20 -D grek").unwrap().1, ["adjust-subglyphs", "default-script"]);
        assert_eq!(TtAutohintOptions::parse(&o.to_text()).unwrap().0, o, "{}", o.to_text());
        assert_eq!(TtAutohintOptions::default().to_text(), "");
        assert_eq!(snapping_exceptions("-").unwrap(), [(1, MAX_OPTION_PPM)]);
        for bad in [
            "--frobnicate",
            "--hinting-limit",
            "--hinting-limit=lots",
            "--hinting-limit=40",
            "-l 60 -r 50",
            "--increase-x-height=3",
            "--dehint=yes",
            "--stem-width-mode=qq",
            "--no-info --detailed-info",
            "--x-height-snapping-exceptions=9-3",
            "--default-script=no/tag",
            "plain",
        ] {
            assert!(TtAutohintOptions::parse(bad).is_err(), "{bad}");
        }
        assert!(TtAutohintOptions::parse(&"-W ".repeat(5_000)).is_err());
    }

    /// A slab 0 to 100 high and a wedge whose lowest point (point 5) hangs at 160, outside every
    /// zone of a new font's master, with a horizontal TT stem of 100.
    fn slab_and_wedge() -> (Font, HintSettings) {
        use fontcraft_model::{Node, NodeKind, Path};
        let mut font = Font::new("Edges");
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        font.glyph_mut("x").unwrap().layer_mut("m01").unwrap().shapes = vec![
            Shape::Path(Path::rect(0.0, 0.0, 260.0, 100.0)),
            Shape::Path(Path::new(true, vec![n(250.0, 400.0), n(300.0, 160.0), n(350.0, 400.0)])),
        ];
        let mut settings = HintSettings::for_master(&font, "m01").unwrap();
        settings.tt_stems.push(crate::TtStem { name: "bar".into(), horizontal: true, size: 100.0, ..Default::default() });
        (font, settings)
    }

    fn hints_for(font: &Font, settings: &HintSettings, control: &str) -> AutoHints {
        let control = crate::control::parse(control).unwrap();
        let options = TtAutohintOptions::default();
        let layer = font.glyph("x").unwrap().layer("m01").unwrap();
        let input = AutoGlyph {
            glyph: "x",
            layer,
            settings,
            zones: None,
            stems: None,
            options: &options,
            control: &control,
            points: None,
            delta_column: Some("m01"),
        };
        autohint_glyph(&input).unwrap()
    }

    fn on(list: &[TtInstruction], node: NodeRef) -> Vec<&TtInstruction> {
        list.iter().filter(|t| t.target == TtPoint::Node(node)).collect()
    }

    #[test]
    fn left_and_right_points_become_edges_whose_offsets_reach_a_neighbouring_segment() {
        let (font, settings) = slab_and_wedge();
        let tip = NodeRef::new(1, 1);
        assert!(on(&hints_for(&font, &settings, "").instructions, tip).is_empty(), "the tip is no extreme in a zone");

        // A one-point segment over nothing: an edge rounded to the grid.
        let h = hints_for(&font, &settings, "x left 5");
        let t = on(&h.instructions, tip);
        assert_eq!(t.len(), 1, "{:?}", h.instructions);
        assert_eq!((&t[0].kind, t[0].direction), (&TtKind::Snap { zone: ZoneRef::NoZone }, Direction::Vertical));

        // Offsets that reach over the slab's top make a stem with it.
        let h = hints_for(&font, &settings, "x right 5 (-60, 0)");
        let t = on(&h.instructions, tip);
        assert_eq!(t.len(), 1, "{:?}", h.instructions);
        assert!(matches!(t[0].kind, TtKind::Stem { .. }) && t[0].direction == Direction::Vertical, "{:?}", t[0]);
        match t[0].origin {
            Some(TtPoint::Node(r)) => assert!(r.path == 0 && matches!(r.node, 2 | 3), "the origin is on the slab's top: {r:?}"),
            ref other => panic!("{other:?}"),
        }

        // `nodir` wins over a direction.
        let h = hints_for(&font, &settings, "x left 5\nx nodir 5");
        assert!(on(&h.instructions, tip).is_empty());
    }

    #[test]
    fn corrections_past_the_cell_budget_are_left_out_with_a_warning() {
        // 300 lines moving all seven points at 255 sizes: about a million point and size cells.
        let (font, settings) = slab_and_wedge();
        let control = "x point 0-6 xshift 1 yshift 1 @ 1-255\n".repeat(300);
        let h = hints_for(&font, &settings, &control);
        let cells: usize = h.shifts.iter().map(|s| s.ppems.len()).sum();
        assert!(cells > 0 && cells <= MAX_CONTROL_CELLS, "{cells}");
        assert_eq!(h.warnings.iter().filter(|w| w.contains("correction cells")).count(), 1, "{:?}", h.warnings);
    }

    #[test]
    fn point_shifts_and_x_shifts_run_after_iup_in_eighths_of_a_pixel() {
        let (font, settings) = slab_and_wedge();
        let tip = NodeRef::new(1, 1);
        let h = hints_for(&font, &settings, "x point 5 xshift 0.3 yshift -0.5 @ 12; x touch 5 xshift -1 yshift 1 @ 9-10");
        let shifts: Vec<(Direction, i32, &[u16])> = h.shifts.iter().map(|s| (s.direction, s.amount, s.ppems.as_slice())).collect();
        assert_eq!(
            shifts,
            [(Direction::Horizontal, 16, &[12][..]), (Direction::Vertical, -32, &[12][..]), (Direction::Horizontal, -64, &[9, 10][..])]
        );
        assert!(h.shifts.iter().all(|s| s.node == tip));
        // The touch y shift is a Delta before IUP.
        assert!(on(&h.instructions, tip).iter().any(|t| matches!(t.kind, TtKind::Delta { .. })), "{:?}", h.instructions);

        let points = crate::VecPointMap { paths: vec![(0..4).map(Some).collect(), (4..7).map(Some).collect()], count: 7 };
        let program = shift_program(&h.shifts, &points).unwrap();
        let text = fontcraft_ttasm::disassemble(&program).unwrap();
        assert!(text.contains("SVTCA[0]") && text.contains("SVTCA[1]"), "{text}");

        // Stored as Instructor instructions, only the vertical shift of half a pixel or more stays.
        let mut warnings = Vec::new();
        let deltas = shifts_as_deltas(&h.shifts, Some("m01"), &mut warnings);
        assert_eq!(deltas.len(), 1);
        assert!(matches!(&deltas[0].kind, TtKind::Delta { cells } if cells["m01"] == BTreeMap::from([(12u16, DeltaDir::Down)])));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn x_height_scaling_needs_an_x_height_zone_and_a_cvt_entry_for_its_top() {
        let (font, settings) = slab_and_wedge();
        let layout = crate::CvtLayout::new(&settings, &[]).unwrap();
        let (cvt, prep) = autohint_prep(&TtAutohintOptions::default(), &settings, &layout, true).unwrap();
        assert_eq!(cvt.len(), settings.tt_zones.len());
        let x = settings.tt_zones.iter().position(|z| z.name == "xHeight").unwrap();
        assert_eq!(cvt[x], 516, "the new font's x-height 500 with its overshoot of 16");
        let (_, unscaled) = autohint_prep(&TtAutohintOptions::default(), &settings, &layout, false).unwrap();
        assert!(unscaled.len() < prep.len());
        let mut flat = settings.clone();
        flat.tt_zones.retain(|z| z.name != "xHeight");
        let layout = crate::CvtLayout::new(&flat, &[]).unwrap();
        let [with, without] = [true, false].map(|on| autohint_prep(&TtAutohintOptions::default(), &flat, &layout, on).unwrap().1);
        assert_eq!(with, without, "no x-height zone, no scaling");
        let _ = font;
    }

    /// A font whose only letter is `name`, a box `height` units tall.
    fn one_letter(name: &str, height: f64) -> Font {
        let mut font = Font::new("Letters");
        font.glyph_mut(name).unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(fontcraft_model::Path::rect(0.0, 0.0, 400.0, height)));
        font
    }

    #[test]
    fn zones_without_their_letters_are_left_out() {
        let font = Font::new("Letters");
        let mut settings = HintSettings::for_master(&font, "m01").unwrap();
        let before = settings.tt_zones.len();
        // A new font's letters have no outlines yet.
        let measured = drop_unmeasured_zones(&font, &mut settings);
        assert_eq!((measured.left_out, measured.x_height), (vec!["xHeight".to_string(), "capHeight".to_string()], false));
        assert_eq!(settings.tt_zones.len(), before - 2);

        let font = one_letter("e", 500.0);
        let mut settings = HintSettings::for_master(&font, "m01").unwrap();
        let measured = drop_unmeasured_zones(&font, &mut settings);
        assert_eq!((measured.left_out, measured.x_height), (vec!["capHeight".to_string()], true));
        assert!(settings.tt_zones.iter().any(|z| z.name == "xHeight"));
    }

    #[test]
    fn a_capital_as_tall_as_the_x_height_is_snapped_without_x_height_scaling() {
        // ttfautohint measures its capital zone on C here: 4.5 px goes up to 5 at 9 PPM, and at
        // 12 PPM 6.24 px (a height of 520) stays on 6, where the x-height would go up to 7.
        let top = |name: &str, height: f64, ppem: u16| {
            let mut font = one_letter(name, height);
            for v in &mut std::sync::Arc::make_mut(&mut font.masters)[0].metric_values {
                if v.pos == 500.0 {
                    v.pos = height;
                }
            }
            let built = tt::autohinted(&font, name, &TtAutohintOptions::default());
            tt::hinted_y(&built, ppem, tt::grayscale()).into_iter().fold(0.0f32, f32::max)
        };
        for (ppem, row) in [(9, 5.0), (10, 5.0), (11, 6.0), (13, 7.0), (29, 15.0)] {
            assert_eq!(top("C", 500.0, ppem), row, "{ppem} PPM");
        }
        assert_eq!(top("C", 520.0, 12), 6.0);
        assert_eq!(top("x", 520.0, 12), 7.0);
    }

    /// A closed contour of quadratic curves through the four extremes of a box.
    fn ring(x0: f64, y0: f64, x1: f64, y1: f64) -> fontcraft_model::Path {
        use fontcraft_model::{Node, NodeKind, Path};
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let corners = [(x1, y0), (x1, y1), (x0, y1), (x0, y0)];
        let extremes = [(x1, cy), (cx, y1), (x0, cy), (cx, y0)];
        let nodes =
            corners.iter().zip(extremes).flat_map(|(&(a, b), (x, y))| [Node::new(a, b, NodeKind::OffCurve), Node::new(x, y, NodeKind::QCurve)]);
        Path::new(true, nodes.collect())
    }

    #[test]
    fn overshoots_show_from_half_a_pixel_on_round_extremes() {
        // ttfautohint 1.8.4's rows for an x-height of 500 and an o from -16 to 516.
        let mut font = one_letter("x", 500.0);
        font.glyph_mut("o").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Path(ring(50.0, -16.0, 550.0, 516.0)));
        let options = TtAutohintOptions::default();
        let (round, flat) = (tt::autohinted(&font, "o", &options), tt::autohinted(&font, "x", &options));
        let ends = |f: &[u8], ppem| {
            let y = tt::hinted_y(f, ppem, tt::grayscale());
            (y.iter().copied().fold(f32::MAX, f32::min), y.iter().copied().fold(f32::MIN, f32::max))
        };
        for (ppem, bottom, top, x) in [
            (12, 0.0, 7.0, 7.0),
            (28, 0.0, 15.0, 15.0),
            (32, -0.5, 16.5, 16.0),
            (40, -0.5, 20.5, 20.0),
            (60, -1.0, 31.0, 30.0),
            (100, -2.0, 52.0, 50.0),
        ] {
            assert_eq!(ends(&round, ppem), (bottom, top), "o at {ppem} PPM");
            assert_eq!(ends(&flat, ppem), (0.0, x), "x at {ppem} PPM");
        }
    }

    /// The height in pixels of an E's bar on the baseline, `width` units thick, with a standard
    /// width of 80. The font has no x-height letter, so nothing is scaled.
    fn bar(width: f64, modes: &str, ppem: u16, target: tt::Target) -> f32 {
        use fontcraft_model::{Node, NodeKind, Path};
        let mut font = Font::new("Bars");
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let e = vec![n(50.0, 0.0), n(550.0, 0.0), n(550.0, width), n(200.0, width), n(200.0, 700.0), n(50.0, 700.0)];
        font.glyph_mut("E").unwrap().layer_mut("m01").unwrap().shapes = vec![Shape::Path(Path::new(true, e))];
        let options = TtAutohintOptions::parse(&format!("--symbol --fallback-stem-width=80 --stem-width-mode={modes}")).unwrap().0;
        let y = tt::hinted_y(&tt::autohinted(&font, "E", &options), ppem, target);
        y[2] - y[0]
    }

    #[test]
    fn stem_widths_follow_the_mode_of_the_rendering_target_as_ttfautohints_do() {
        let near = |got: f32, want: f32, what: &str| assert!((got - want).abs() <= 1.0 / 32.0, "{what}: {got}, not {want}");
        // ttfautohint 1.8.4's bar heights for a standard width of 80 at 20 PPM (1.6 pixels).
        for (width, natural, quantized, strong) in
            [(40.0, 0.80, 0.875, 1.0), (60.0, 1.20, 1.60, 1.0), (70.0, 1.40, 1.60, 2.0), (120.0, 2.40, 2.16, 2.0), (150.0, 3.0, 3.0, 3.0)]
        {
            for (modes, want) in [("nnn", natural), ("qqq", quantized), ("sss", strong)] {
                near(bar(width, modes, 20, tt::grayscale()), want, &format!("{width} units, {modes}"));
            }
        }
        // Grayscale without ClearType, GDI ClearType, DirectWrite ClearType.
        near(bar(80.0, "snn", 20, tt::mono()), 2.0, "the first letter");
        near(bar(80.0, "nsn", 20, tt::gdi()), 2.0, "the second letter");
        near(bar(80.0, "nns", 20, tt::grayscale()), 2.0, "the third letter");
    }
}
