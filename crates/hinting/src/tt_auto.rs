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
//! 3. Vertical Delta for the `touch` and `point` control instructions.
//!
//! Nothing is hinted horizontally except the `left` and `right` points of the control
//! instructions.
//!
//! Control instructions: `nodir` points are never instructed; `left` and `right` points get a
//! rounded horizontal Snap, which makes them an edge the hinting keeps on the grid.

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
const IGNORED_OPTIONS: [&str; 15] = [
    "hinting-range-min",
    "hinting-range-max",
    "adjust-subglyphs",
    "composites",
    "default-script",
    "fallback-script",
    "fallback-scaling",
    "stem-width-mode",
    "ignore-restrictions",
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
            let reported = if matches!(name, "pre-hinting") {
                "adjust-subglyphs"
            } else if name == "strong-stem-width" {
                "stem-width-mode"
            } else {
                name
            };
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

/// What the TTFAutohint options add to `prep` after our own program ([`crate::font_programs`]):
/// the hinting limit (glyph programs off above it), the x-height increase (the `xHeight` zone
/// rounded up from an eighth of a pixel at 6 PPM up to the limit) and the x-height snapping
/// exceptions (the `xHeight` zone unrounded at those sizes). Empty when none applies.
pub fn autohint_prep(options: &TtAutohintOptions, settings: &HintSettings, layout: &crate::CvtLayout) -> Result<Vec<u8>, HintingError> {
    use std::fmt::Write;
    let mut t = String::new();
    let x_height: Vec<(u16, u16)> = settings
        .tt_zones
        .iter()
        .zip(&layout.zones)
        .filter(|(z, _)| z.name == "xHeight" && z.size > 0.0 && z.align.is_none())
        .map(|(_, cells)| *cells)
        .collect();
    let in_range = |lo: u16, hi: u16| format!("MPPEM[ ]\nPUSH {lo}\nGTEQ[ ]\nMPPEM[ ]\nPUSH {hi}\nLTEQ[ ]\nAND[ ]\nIF[ ]");
    if options.increase_x_height >= 6 {
        for (pos, fit) in &x_height {
            let _ = writeln!(
                t,
                "{}\nPUSH {fit} {pos} {X_HEIGHT_INCREASE_THRESHOLD} {}\nCALL[ ]\nEIF[ ]",
                in_range(6, options.increase_x_height.min(MAX_OPTION_PPM)),
                crate::tt_font::FN_ZONE_ROUND
            );
        }
    }
    let exceptions = snapping_exceptions(&options.x_height_snapping_exceptions)
        .map_err(|message| HintingError::BadParameter { name: "TTFAutohint options".into(), message })?;
    for (lo, hi) in exceptions {
        for (pos, fit) in &x_height {
            let _ = writeln!(t, "{}\nPUSH {fit} {pos}\nRCVT[ ]\nWCVTP[ ]\nEIF[ ]", in_range(lo, hi));
        }
    }
    if options.hinting_limit > 0 {
        let _ = writeln!(t, "MPPEM[ ]\nPUSH {}\nGT[ ]\nIF[ ]\nPUSH 1 1\nINSTCTRL[ ]\nEIF[ ]", options.hinting_limit.min(MAX_OPTION_PPM));
    }
    if t.is_empty() {
        return Ok(Vec::new());
    }
    Ok(fontcraft_ttasm::assemble(&t)?)
}

/// The rounding threshold of the `xHeight` zone at the sizes the x-height increase covers, 26.6
/// pixels: the x-height takes the row above once it lies an eighth of a pixel above the one
/// below.
const X_HEIGHT_INCREASE_THRESHOLD: i32 = 24;

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
    /// What the analysis skipped or could not use, for the report.
    pub warnings: Vec<String>,
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
    Ok(autohint_glyph(&input)?.instructions)
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
    let mut touched_x: BTreeSet<NodeRef> = BTreeSet::new();
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

    // Control instructions: left and right edges, then the corrections.
    for r in &control.edges {
        if touched_x.insert(*r) {
            list.push(TtInstruction::new(TtKind::Snap { zone: ZoneRef::NoZone }, Direction::Horizontal, TtPoint::Node(*r)));
        }
    }
    list.extend(control.deltas(input.delta_column, &mut out.warnings));
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
    edges: Vec<NodeRef>,
    /// (node, PPMs, y shift in pixels) of each `touch` and `point` correction, in text order.
    shifts: Vec<(NodeRef, Vec<u16>, f64)>,
}

impl GlyphControl {
    fn new(input: &AutoGlyph, paths: &[Vec<Pt>], warnings: &mut Vec<String>) -> Self {
        let mut out = Self::default();
        let lines: Vec<_> = input.control.lines.iter().filter(|l| l.font_index == 0 && l.glyph == input.glyph).collect();
        if lines.is_empty() {
            return out;
        }
        // Exported point number → node.
        let mut by_point: HashMap<u32, NodeRef> = HashMap::new();
        let mut k = 0u32;
        for p in paths.iter().flatten() {
            let n = match input.points {
                Some(map) => map.point(p.r).map(u32::from),
                None => Some(k),
            };
            k = k.saturating_add(1);
            if let Some(n) = n {
                by_point.entry(n).or_insert(p.r);
            }
        }
        for line in lines {
            let mut nodes = Vec::with_capacity(line.points.len());
            for i in &line.points {
                match by_point.get(i) {
                    Some(r) => nodes.push(*r),
                    None => warnings.push(format!("control instructions, line {}: glyph `{}` has no point {i}", line.line, input.glyph)),
                }
            }
            match &line.kind {
                ControlKind::NoDir => out.nodir.extend(nodes),
                ControlKind::Left | ControlKind::Right => out.edges.extend(nodes),
                ControlKind::Touch { x_shift, y_shift } | ControlKind::Point { x_shift, y_shift } => {
                    if *x_shift != 0.0 {
                        warnings
                            .push(format!("control instructions, line {}: horizontal shifts are not applied by the built-in autohinter", line.line));
                    }
                    for r in nodes {
                        out.shifts.push((r, line.ppems.clone(), *y_shift));
                    }
                }
            }
        }
        out
    }

    /// One vertical Delta per corrected node, cells by the sign of its shift.
    fn deltas(&self, column: Option<&str>, warnings: &mut Vec<String>) -> Vec<TtInstruction> {
        if self.shifts.is_empty() {
            return Vec::new();
        }
        let Some(column) = column else {
            warnings.push("control instructions: the corrections have no Delta column here and were left out".into());
            return Vec::new();
        };
        let mut per_node: BTreeMap<NodeRef, BTreeMap<u16, DeltaDir>> = BTreeMap::new();
        for (r, ppems, shift) in &self.shifts {
            if shift.abs() < MIN_DELTA_SHIFT {
                continue;
            }
            let dir = if *shift > 0.0 { DeltaDir::Up } else { DeltaDir::Down };
            let cells = per_node.entry(*r).or_default();
            for p in ppems.iter().filter(|p| (1..=crate::MAX_DELTA_PPM).contains(*p)) {
                cells.insert(*p, dir);
            }
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_read_long_and_short_forms_and_report_the_ignored_ones() {
        let (o, ignored) =
            TtAutohintOptions::parse("--hinting-limit=120 -x 0 --x-height-snapping-exceptions 8-9,12 -W -t --stem-width-mode=nnn -c -H50").unwrap();
        assert_eq!((o.hinting_limit, o.increase_x_height, o.fallback_stem_width), (120, 0, Some(50)));
        assert!(o.windows_compatibility && o.ttfa_table && o.hint_composites);
        assert_eq!(ignored, ["stem-width-mode", "composites"]);
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
}
