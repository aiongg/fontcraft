//! Outline repairs on compiled fonts (M12.28; rows `outlines remove-overlaps`, `direction`,
//! `extremes`, `round`, `decompose` and their `outlines.*` commands). Oracles: fontTools
//! `ttLib.removeOverlaps`, checkoutlinesufo `-e`, ttfdecomponentizer. Untouched tables are copied
//! byte for byte.
//!
//! The geometry is the editor's (`fontcraft_pathops`, decision D2: one implementation per
//! action); this module reads and writes the `glyf` outlines around it. Rules, by action:
//!
//! | Action | What changes |
//! |---|---|
//! | Remove overlaps | As `ttLib.removeOverlaps`: simple glyphs first, then composites by component depth. A simple glyph with contours, or a composite whose components overlap one another, is drawn with its components decomposed and merged (nonzero rule, points on whole units). It is replaced only when the merged contours differ from the drawn ones, and keeps the drawn winding. Contours of a replaced glyph whose bounding box is smaller than `min_area` square units are dropped. Without `keep_hinting` every processed glyph loses its instructions; the font-wide hint tables stay, as in the oracle |
//! | Correct direction | Contours run as the flavour says and start at their lowest, then leftmost, on-curve point, sorted by their bounds (`path.correctPathDirection`), or are reversed in place. Composites are left alone |
//! | Add extremes | `path.addExtremes` on each contour, new points on whole units |
//! | Round | Point coordinates, component offsets and advance widths to multiples of `grid`; with `tolerance`, only values that close to a grid position. Instructions stay (no point moves to another index) |
//! | Decompose | As ttfdecomponentizer: each composite becomes a simple glyph whose contours start at their first on-curve point; or only flattened one level (`nested_only`), or decomposed only when a direct component has a 2×2 transform (`transformed_only`) |
//!
//! A glyph whose geometry changed loses its instructions and gets its left side bearing set to
//! its `xMin`. Afterwards the composite bounding boxes, `head` (bounds, `indexToLocFormat`),
//! `maxp` and `hhea` are recalculated the way fontTools recalculates them on saving; a table whose
//! values come out the same keeps its bytes. With nothing changed the font comes back as it was.
//!
//! CFF-flavoured fonts are refused for now, and so are variable TrueType fonts for every action
//! but rounding: `gvar` would need its deltas rewritten.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use fontcraft_geom::{BezPath, Point};
use fontcraft_pathops as po;
use read_fonts::tables::glyf::SimpleGlyph as RawSimple;
use read_fonts::tables::post::Post;
use read_fonts::types::GlyphId16;
use read_fonts::{FontData, FontRead};

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Deepest component nesting followed; deeper nesting (or a cycle) fails the glyph.
pub const MAX_COMPONENT_DEPTH: usize = 32;
/// Most component references visited while drawing one glyph.
pub const MAX_COMPONENT_VISITS: usize = 1_000_000;
/// Most component references visited in all while working on one font: drawing every composite
/// of a font for `maxp`, or finding nesting depths, stops there.
pub const MAX_FONT_COMPONENT_VISITS: usize = 20_000_000;
const FONT_VISITS_MESSAGE: &str = "the font's composites refer to components more than 20,000,000 times in all";
/// Most points in one glyph, the `glyf` format's limit.
pub const MAX_GLYPH_POINTS: usize = 65_535;
/// Composites with more components skip the pairwise overlap test and are merged whole, the
/// glyph changing only when the merge changes something.
pub const MAX_PAIRWISE_COMPONENTS: usize = 64;
/// Coarsest grid [`OutlineAction::Round`] accepts, in units.
pub const MAX_GRID: f64 = 1_000.0;
/// Largest `min_area`, in square units.
pub const MAX_MIN_AREA: f64 = 1.0e12;
/// The default `min_area` of overlap removal (checkoutlinesufo's).
pub const DEFAULT_MIN_AREA: f64 = 25.0;
/// The tolerance quadratic conversion uses when a merged contour comes back with cubic pieces.
const QUADRATIC_TOLERANCE: f64 = 1.0;

/// One repair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineAction {
    RemoveOverlaps,
    CorrectDirection,
    AddExtremes,
    Round,
    Decompose,
}

/// The winding [`OutlineAction::CorrectDirection`] gives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Winding {
    /// The convention of the font's outline format: TrueType for `glyf`.
    #[default]
    Auto,
    /// Outer contours counter-clockwise, counters clockwise.
    PostScript,
    /// Outer contours clockwise, counters counter-clockwise.
    TrueType,
    /// Every contour reversed, keeping its start point.
    Reverse,
}

/// What [`repair`] does.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineOptions {
    pub action: OutlineAction,
    /// Glyphs to change; empty changes every glyph.
    pub glyphs: Vec<String>,
    /// Glyphs left alone even when `glyphs` names them.
    pub exclude_glyphs: Vec<String>,
    /// Remove overlaps: keep the instructions of glyphs that do not change.
    pub keep_hinting: bool,
    /// Leave a glyph that cannot be done as it was and report it, instead of failing.
    pub ignore_errors: bool,
    /// Remove overlaps: drop result contours whose bounding box is smaller (square units).
    pub min_area: f64,
    /// Correct direction: the convention.
    pub winding: Winding,
    /// Add extremes: [`po::MIN_EXTREME_SEGMENT`] (the editor's) or 0 (every extreme).
    pub min_segment: f64,
    /// Round: the grid, in units.
    pub grid: f64,
    /// Round: only values within this distance of a grid position; `None` rounds every value.
    pub tolerance: Option<f64>,
    /// Decompose: only flatten components of components.
    pub nested_only: bool,
    /// Decompose: only glyphs with a component whose 2×2 transform is not the identity.
    pub transformed_only: bool,
}

impl OutlineOptions {
    /// `action` on every glyph with the defaults of the command line.
    pub fn new(action: OutlineAction) -> Self {
        Self {
            action,
            glyphs: Vec::new(),
            exclude_glyphs: Vec::new(),
            keep_hinting: false,
            ignore_errors: false,
            min_area: DEFAULT_MIN_AREA,
            winding: Winding::Auto,
            min_segment: po::MIN_EXTREME_SEGMENT,
            grid: 1.0,
            tolerance: None,
            nested_only: false,
            transformed_only: false,
        }
    }
}

/// The changed font and the glyphs that changed (shared with [`crate::curves`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineResult {
    pub font: SfntFont,
    pub changed: Vec<String>,
}

/// A glyph left as it was, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphFailure {
    pub glyph: String,
    pub reason: String,
}

/// Everything [`repair_report`] found out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineReport {
    pub result: OutlineResult,
    /// Glyphs left alone because they could not be done (only with `ignore_errors`).
    pub failed: Vec<GlyphFailure>,
    /// Add extremes: points added, by glyph.
    pub points_added: BTreeMap<String, usize>,
    /// Round: coordinates, offsets and advances that moved.
    pub values_changed: usize,
}

/// Applies one repair.
pub fn repair(font: &SfntFont, options: &OutlineOptions) -> Result<OutlineResult> {
    repair_report(font, options).map(|r| r.result)
}

/// Applies one repair and reports per glyph.
pub fn repair_report(font: &SfntFont, options: &OutlineOptions) -> Result<OutlineReport> {
    check_options(options)?;
    if font.get(*b"glyf").is_none() {
        if font.is_cff() {
            return Err(TablesError::Unsupported("outline repairs on CFF outlines; TrueType (`glyf`) outlines only".into()));
        }
        return Err(TablesError::MissingTable("glyf".into()));
    }
    if font.get(*b"gvar").is_some() && options.action != OutlineAction::Round {
        return Err(TablesError::Unsupported("changing the outlines of a variable font: instantiate it first".into()));
    }
    let mut work = Work::read(font)?;
    let selected = work.selection(options)?;
    let mut report = Report::default();
    match options.action {
        OutlineAction::RemoveOverlaps => work.remove_overlaps(&selected, options, &mut report)?,
        OutlineAction::CorrectDirection => work.correct_direction(&selected, options, &mut report)?,
        OutlineAction::AddExtremes => work.add_extremes(&selected, options, &mut report)?,
        OutlineAction::Round => work.round(&selected, options, &mut report)?,
        OutlineAction::Decompose => work.decompose(&selected, options, &mut report)?,
    }
    let changed: Vec<String> = report.changed.iter().filter_map(|&gid| work.names.get(gid).cloned()).collect();
    let font = work.finish(font)?;
    Ok(OutlineReport {
        result: OutlineResult { font, changed },
        failed: report.failed,
        points_added: report.points_added,
        values_changed: report.values_changed,
    })
}

fn check_options(o: &OutlineOptions) -> Result<()> {
    let bad = |m: &str| Err(TablesError::BadOptions(m.into()));
    if !(o.min_area.is_finite() && (0.0..=MAX_MIN_AREA).contains(&o.min_area)) {
        return bad("the minimum area must be a number from 0 to 10^12");
    }
    if !(o.grid.is_finite() && o.grid >= po::MIN_GRID && o.grid <= MAX_GRID) {
        return bad("the grid must be a positive number of units, at most 1000");
    }
    if o.tolerance.is_some_and(|t| !(t.is_finite() && t >= 0.0)) {
        return bad("the rounding tolerance must be a number of units, 0 or more");
    }
    if !o.min_segment.is_finite() || o.min_segment < 0.0 {
        return bad("the minimum segment must be a number of units, 0 or more");
    }
    if o.min_segment != 0.0 && o.min_segment != po::MIN_EXTREME_SEGMENT {
        return Err(TablesError::Unsupported(format!(
            "a minimum segment other than {} units (the editor's) or 0 (every extreme)",
            po::MIN_EXTREME_SEGMENT
        )));
    }
    if o.nested_only && o.transformed_only {
        return bad("give nested-only or transformed-only, not both");
    }
    let names = o.glyphs.len().saturating_add(o.exclude_glyphs.len());
    if names > crate::limits::MAX_GLYPHS.saturating_mul(2) {
        return Err(TablesError::TooLarge { what: "the glyph list", limit: crate::limits::MAX_GLYPHS });
    }
    Ok(())
}

// ── Glyph data ──────────────────────────────────────────────────────────────────────────────────

/// A point of a `glyf` outline, in font units; whole numbers except while a transform is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pt {
    x: f64,
    y: f64,
    on: bool,
}

type Outline = Vec<Vec<Pt>>;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    Offset(f64, f64),
    /// Point `base` of the glyph so far meets point `component` of the component.
    Points(u16, u16),
}

/// A component record. `m` is the 2×2 transform in file order (`xx`, `yx`, `xy`, `yy`):
/// `x' = xx·x + xy·y`, `y' = yx·x + yy·y`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Comp {
    flags: u16,
    gid: u16,
    anchor: Anchor,
    m: [f64; 4],
}

const IDENTITY: [f64; 4] = [1.0, 0.0, 0.0, 1.0];

const ARG_WORDS: u16 = 0x0001;
const ARGS_XY: u16 = 0x0002;
const ROUND_XY: u16 = 0x0004;
const HAVE_SCALE: u16 = 0x0008;
const MORE: u16 = 0x0020;
const XY_SCALE: u16 = 0x0040;
const TWO_BY_TWO: u16 = 0x0080;
const HAVE_INSTRUCTIONS: u16 = 0x0100;
const USE_MY_METRICS: u16 = 0x0200;
const OVERLAP_COMPOUND: u16 = 0x0400;
const SCALED_OFFSET: u16 = 0x0800;
const UNSCALED_OFFSET: u16 = 0x1000;
/// Flags a rewritten record keeps; the others follow from its data.
const KEPT_FLAGS: u16 = ROUND_XY | USE_MY_METRICS | OVERLAP_COMPOUND | SCALED_OFFSET | UNSCALED_OFFSET;

#[derive(Clone, Debug, PartialEq)]
enum Parsed {
    Empty,
    Simple {
        outline: Outline,
        instructions: Vec<u8>,
        /// Offset of the instruction length field in the glyph's bytes.
        instr_at: usize,
    },
    Composite {
        comps: Vec<Comp>,
        instructions: Vec<u8>,
        /// Offset of each record's flags, and of the instructions' length field (or the end).
        flag_at: Vec<usize>,
        instr_at: usize,
    },
    Bad(String),
}

#[derive(Clone, Debug)]
struct Glyph {
    bytes: Vec<u8>,
    parsed: Parsed,
    /// The geometry changed: the left side bearing follows `xMin`.
    moved: bool,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    if let Some(slot) = at.checked_add(2).and_then(|end| b.get_mut(at..end)) {
        slot.copy_from_slice(&v.to_be_bytes());
    }
}

fn f2dot14(v: u16) -> f64 {
    f64::from(v as i16) / 16384.0
}

/// fontTools' `otRound`: halves go up.
fn ot_round(v: f64) -> f64 {
    if v.is_finite() { (v + 0.5).floor() } else { 0.0 }
}

fn parse(bytes: &[u8]) -> Parsed {
    if bytes.is_empty() {
        return Parsed::Empty;
    }
    let Some(contours) = i16_at(bytes, 0) else { return Parsed::Bad("the glyph header is cut short".into()) };
    let parsed = if contours >= 0 { parse_simple(bytes) } else { parse_composite(bytes) };
    parsed.unwrap_or_else(Parsed::Bad)
}

fn parse_simple(bytes: &[u8]) -> std::result::Result<Parsed, String> {
    let g = RawSimple::read(FontData::new(bytes)).map_err(|e| format!("unreadable glyph: {e}"))?;
    let points: Vec<_> = g.points().collect();
    if points.len() > MAX_GLYPH_POINTS {
        return Err("the glyph has too many points".into());
    }
    let mut outline = Vec::new();
    let mut start = 0usize;
    for end in g.end_pts_of_contours() {
        let end = usize::from(end.get());
        let contour = points.get(start..=end).ok_or("the glyph's contour ends do not match its points")?;
        outline.push(contour.iter().map(|p| Pt { x: f64::from(p.x), y: f64::from(p.y), on: p.on_curve }).collect());
        start = end.saturating_add(1);
    }
    let instr_at = g.instructions_byte_range().start.saturating_sub(2);
    Ok(Parsed::Simple { outline, instructions: g.instructions().to_vec(), instr_at })
}

fn parse_composite(b: &[u8]) -> std::result::Result<Parsed, String> {
    let cut = || "the composite glyph is cut short".to_string();
    let mut at = 10usize;
    let mut comps = Vec::new();
    let mut flag_at = Vec::new();
    let mut have_instructions = false;
    loop {
        let flags = u16_at(b, at).ok_or_else(cut)?;
        let gid = u16_at(b, at.saturating_add(2)).ok_or_else(cut)?;
        flag_at.push(at);
        at = at.saturating_add(4);
        let xy = flags & ARGS_XY != 0;
        let (a1, a2) = if flags & ARG_WORDS != 0 {
            let r = (u16_at(b, at).ok_or_else(cut)?, u16_at(b, at.saturating_add(2)).ok_or_else(cut)?);
            at = at.saturating_add(4);
            if xy { (f64::from(r.0 as i16), f64::from(r.1 as i16)) } else { (f64::from(r.0), f64::from(r.1)) }
        } else {
            let r = (*b.get(at).ok_or_else(cut)?, *b.get(at.saturating_add(1)).ok_or_else(cut)?);
            at = at.saturating_add(2);
            if xy { (f64::from(r.0 as i8), f64::from(r.1 as i8)) } else { (f64::from(r.0), f64::from(r.1)) }
        };
        let anchor = if xy { Anchor::Offset(a1, a2) } else { Anchor::Points(a1 as u16, a2 as u16) };
        let f = |k: usize| u16_at(b, at.saturating_add(2 * k)).map(f2dot14).ok_or_else(cut);
        let m = if flags & HAVE_SCALE != 0 {
            let s = f(0)?;
            at = at.saturating_add(2);
            [s, 0.0, 0.0, s]
        } else if flags & XY_SCALE != 0 {
            let m = [f(0)?, 0.0, 0.0, f(1)?];
            at = at.saturating_add(4);
            m
        } else if flags & TWO_BY_TWO != 0 {
            let m = [f(0)?, f(1)?, f(2)?, f(3)?];
            at = at.saturating_add(8);
            m
        } else {
            IDENTITY
        };
        comps.push(Comp { flags, gid, anchor, m });
        have_instructions |= flags & HAVE_INSTRUCTIONS != 0;
        if flags & MORE == 0 {
            break;
        }
    }
    let instructions = if have_instructions {
        let n = usize::from(u16_at(b, at).ok_or_else(cut)?);
        b.get(at.saturating_add(2)..at.saturating_add(2).saturating_add(n)).ok_or_else(cut)?.to_vec()
    } else {
        Vec::new()
    };
    Ok(Parsed::Composite { comps, instructions, flag_at, instr_at: at })
}

/// The glyph's bytes without instructions; `None` when it has none.
fn strip_instructions(g: &Glyph) -> Option<Vec<u8>> {
    match &g.parsed {
        Parsed::Simple { instructions, instr_at, .. } if !instructions.is_empty() => {
            let mut out = g.bytes.get(..*instr_at)?.to_vec();
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(g.bytes.get(instr_at.saturating_add(2).saturating_add(instructions.len())..)?);
            Some(out)
        }
        Parsed::Composite { flag_at, instr_at, .. } if flag_at.iter().any(|&at| u16_at(&g.bytes, at).is_some_and(|f| f & HAVE_INSTRUCTIONS != 0)) => {
            let mut out = g.bytes.get(..*instr_at)?.to_vec();
            for &at in flag_at {
                let flags = u16_at(&out, at)?;
                put_u16(&mut out, at, flags & !HAVE_INSTRUCTIONS);
            }
            Some(out)
        }
        _ => None,
    }
}

/// A simple glyph's bytes: the points rounded to whole units, contours of no points left out.
fn encode_simple(outline: &[Vec<Pt>], instructions: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let contours: Vec<Vec<(i16, i16, bool)>> = outline
        .iter()
        .filter(|c| !c.is_empty())
        .map(|c| c.iter().map(|p| Ok((coord(p.x)?, coord(p.y)?, p.on))).collect::<std::result::Result<Vec<_>, String>>())
        .collect::<std::result::Result<_, _>>()?;
    let total: usize = contours.iter().map(Vec::len).sum();
    if total == 0 {
        return Ok(Vec::new());
    }
    if total > MAX_GLYPH_POINTS || contours.len() > i16::MAX as usize {
        return Err("the glyph would have more points than a TrueType glyph can hold".into());
    }
    if instructions.len() > usize::from(u16::MAX) {
        return Err("the glyph's instructions are too long".into());
    }
    let all = || contours.iter().flatten();
    let bbox = [
        all().map(|p| p.0).min().unwrap_or(0),
        all().map(|p| p.1).min().unwrap_or(0),
        all().map(|p| p.0).max().unwrap_or(0),
        all().map(|p| p.1).max().unwrap_or(0),
    ];
    let mut out = Vec::with_capacity(total.saturating_mul(5).saturating_add(16));
    out.extend_from_slice(&(contours.len() as i16).to_be_bytes());
    for v in bbox {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut end = 0usize;
    for c in &contours {
        end = end.saturating_add(c.len());
        out.extend_from_slice(&(end.saturating_sub(1) as u16).to_be_bytes());
    }
    out.extend_from_slice(&(instructions.len() as u16).to_be_bytes());
    out.extend_from_slice(instructions);
    let (mut flags, mut xs, mut ys) = (Vec::with_capacity(total), Vec::new(), Vec::new());
    let (mut px, mut py) = (0i32, 0i32);
    for &(x, y, on) in all() {
        let mut flag = u8::from(on);
        flag |= delta(i32::from(x) - px, 0x02, 0x10, &mut xs);
        flag |= delta(i32::from(y) - py, 0x04, 0x20, &mut ys);
        (px, py) = (i32::from(x), i32::from(y));
        flags.push(flag);
    }
    let mut i = 0usize;
    while let Some(&flag) = flags.get(i) {
        let run = flags.iter().skip(i + 1).take(255).take_while(|&&f| f == flag).count();
        if run > 0 {
            out.push(flag | 0x08);
            out.push(run as u8);
        } else {
            out.push(flag);
        }
        i = i.saturating_add(run).saturating_add(1);
    }
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
    Ok(out)
}

/// Writes one coordinate delta and returns its flag bits.
fn delta(d: i32, short: u8, same_or_positive: u8, out: &mut Vec<u8>) -> u8 {
    if d == 0 {
        same_or_positive
    } else if (-255..=255).contains(&d) {
        out.push(d.unsigned_abs() as u8);
        short | if d > 0 { same_or_positive } else { 0 }
    } else {
        out.extend_from_slice(&(d as i16).to_be_bytes());
        0
    }
}

fn coord(v: f64) -> std::result::Result<i16, String> {
    let r = ot_round(v);
    if (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&r) {
        Ok(r as i16)
    } else {
        Err(format!("a coordinate ({v}) is beyond what a TrueType glyph can hold"))
    }
}

fn f2dot14_bits(v: f64) -> std::result::Result<u16, String> {
    let r = (v * 16384.0).round();
    if (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&r) {
        Ok(r as i16 as u16)
    } else {
        Err(format!("a component transform value ({v}) is outside the range a TrueType glyph can hold (-2 to 2)"))
    }
}

/// A composite glyph's bytes.
fn encode_composite(comps: &[Comp], instructions: &[u8], bbox: [i16; 4]) -> std::result::Result<Vec<u8>, String> {
    if instructions.len() > usize::from(u16::MAX) {
        return Err("the glyph's instructions are too long".into());
    }
    let mut out = Vec::with_capacity(comps.len().saturating_mul(16).saturating_add(12));
    out.extend_from_slice(&(-1i16).to_be_bytes());
    for v in bbox {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let count = comps.len();
    for (i, c) in comps.iter().enumerate() {
        let mut flags = c.flags & KEPT_FLAGS;
        let mut args = Vec::with_capacity(4);
        match c.anchor {
            Anchor::Offset(x, y) => {
                flags |= ARGS_XY;
                let (x, y) = (coord(x)?, coord(y)?);
                if (-128..=127).contains(&x) && (-128..=127).contains(&y) {
                    args.extend_from_slice(&[x as i8 as u8, y as i8 as u8]);
                } else {
                    flags |= ARG_WORDS;
                    args.extend_from_slice(&x.to_be_bytes());
                    args.extend_from_slice(&y.to_be_bytes());
                }
            }
            Anchor::Points(a, b) => {
                if a <= 255 && b <= 255 {
                    args.extend_from_slice(&[a as u8, b as u8]);
                } else {
                    flags |= ARG_WORDS;
                    args.extend_from_slice(&a.to_be_bytes());
                    args.extend_from_slice(&b.to_be_bytes());
                }
            }
        }
        let [xx, yx, xy, yy] = c.m;
        let transform: Vec<f64> = if c.m == IDENTITY {
            Vec::new()
        } else if yx == 0.0 && xy == 0.0 && xx == yy {
            flags |= HAVE_SCALE;
            vec![xx]
        } else if yx == 0.0 && xy == 0.0 {
            flags |= XY_SCALE;
            vec![xx, yy]
        } else {
            flags |= TWO_BY_TWO;
            vec![xx, yx, xy, yy]
        };
        if i + 1 < count {
            flags |= MORE;
        } else if !instructions.is_empty() {
            flags |= HAVE_INSTRUCTIONS;
        }
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(&c.gid.to_be_bytes());
        out.extend_from_slice(&args);
        for v in transform {
            out.extend_from_slice(&f2dot14_bits(v)?.to_be_bytes());
        }
    }
    if !instructions.is_empty() {
        out.extend_from_slice(&(instructions.len() as u16).to_be_bytes());
        out.extend_from_slice(instructions);
    }
    Ok(out)
}

// ── Conversions to and from the editor's contours ───────────────────────────────────────────────

fn to_po(contour: &[Pt]) -> po::Contour {
    let n = contour.len();
    let nodes = contour
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let prev_off = contour.get((i + n - 1) % n).is_some_and(|q| !q.on);
            let kind = match (p.on, prev_off) {
                (false, _) => po::NodeKind::OffCurve,
                (true, true) => po::NodeKind::QCurve,
                (true, false) => po::NodeKind::Line,
            };
            po::Node::new(Point::new(p.x, p.y), kind)
        })
        .collect();
    po::Contour::new(true, nodes)
}

/// The editor's contours as `glyf` contours; cubic pieces are converted to quadratic splines.
fn from_po(contours: &[po::Contour]) -> std::result::Result<Outline, String> {
    let mut out = Vec::with_capacity(contours.len());
    for c in contours {
        if c.nodes.iter().any(|n| n.kind == po::NodeKind::Curve) {
            let bez = c.to_bez().map_err(|e| e.to_string())?;
            let quadratic = fontcraft_compile::outline::to_quadratic(&bez, QUADRATIC_TOLERANCE);
            for q in po::contours_from_bez(&quadratic) {
                out.push(q.nodes.iter().map(|n| Pt { x: n.point.x, y: n.point.y, on: n.kind != po::NodeKind::OffCurve }).collect());
            }
        } else {
            out.push(c.nodes.iter().map(|n| Pt { x: n.point.x, y: n.point.y, on: n.kind != po::NodeKind::OffCurve }).collect());
        }
    }
    Ok(out)
}

fn bez_of(outline: &[Vec<Pt>]) -> std::result::Result<BezPath, String> {
    let mut path = BezPath::new();
    for c in outline.iter().filter(|c| c.len() > 1) {
        for el in to_po(c).to_bez().map_err(|e| e.to_string())?.elements() {
            path.push(*el);
        }
    }
    Ok(path)
}

/// Twice the signed area of the outline's point polygons: negative when it runs clockwise.
fn signed_area(outline: &[Vec<Pt>]) -> f64 {
    let mut sum = 0.0;
    for c in outline {
        let n = c.len();
        for (i, p) in c.iter().enumerate() {
            if let Some(q) = c.get((i + 1) % n.max(1)) {
                sum += p.x * q.y - q.x * p.y;
            }
        }
    }
    sum
}

/// The contour run the other way from the same start point.
fn reversed(c: &[Pt]) -> Vec<Pt> {
    let mut out = Vec::with_capacity(c.len());
    out.extend(c.first().copied());
    out.extend(c.iter().skip(1).rev().copied());
    out
}

/// The contour started at its first on-curve point, as fontTools draws it.
fn from_first_on_curve(c: &[Pt]) -> Vec<Pt> {
    match c.iter().position(|p| p.on) {
        Some(k) => c.iter().skip(k).chain(c.iter().take(k)).copied().collect(),
        None => c.to_vec(),
    }
}

type Key = Vec<(i64, i64, bool)>;

fn key(c: &[Pt]) -> Key {
    c.iter().map(|p| (ot_round(p.x) as i64, ot_round(p.y) as i64, p.on)).collect()
}

fn same_ordered(a: &[Vec<Pt>], b: &[Vec<Pt>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| key(x) == key(y))
}

fn same_unordered(a: &[Vec<Pt>], b: &[Vec<Pt>]) -> bool {
    a.iter().map(|c| key(c)).collect::<BTreeSet<_>>() == b.iter().map(|c| key(c)).collect::<BTreeSet<_>>()
}

fn area_of_bounds(c: &[Pt]) -> f64 {
    let xs = c.iter().map(|p| p.x);
    let ys = c.iter().map(|p| p.y);
    let w = xs.clone().fold(f64::NEG_INFINITY, f64::max) - xs.fold(f64::INFINITY, f64::min);
    let h = ys.clone().fold(f64::NEG_INFINITY, f64::max) - ys.fold(f64::INFINITY, f64::min);
    if w.is_finite() && h.is_finite() { w * h } else { 0.0 }
}

/// Runs a geometry job with the editor's time limit; a timeout or a panic becomes a message.
fn limited<T: Send + 'static>(f: impl FnOnce() -> std::result::Result<T, po::PathOpError> + Send + 'static) -> std::result::Result<T, String> {
    po::run_with_time_limit(po::TIME_LIMIT, f).map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

// ── The font being repaired ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Report {
    changed: BTreeSet<usize>,
    failed: Vec<GlyphFailure>,
    points_added: BTreeMap<String, usize>,
    values_changed: usize,
}

struct Work {
    glyphs: Vec<Glyph>,
    names: Vec<String>,
    /// `(advance, lsb)` per glyph, as `hmtx` stores them.
    metrics: Vec<(u16, i16)>,
    number_of_h_metrics: usize,
    advances_changed: bool,
    bytes_changed: bool,
    /// Component references visited so far, against [`MAX_FONT_COMPONENT_VISITS`].
    font_visits: Cell<usize>,
}

impl Work {
    fn read(font: &SfntFont) -> Result<Self> {
        let head = font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?;
        let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
        let loca = font.get(*b"loca").ok_or_else(|| TablesError::MissingTable("loca".into()))?;
        let glyf = font.get(*b"glyf").unwrap_or_default();
        let count = usize::from(u16_at(maxp, 4).ok_or_else(|| TablesError::invalid("the maxp table is cut short"))?);
        let long = match i16_at(head, 50) {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(TablesError::invalid("the head table's indexToLocFormat is not 0 or 1")),
        };
        let offset = |i: usize| -> Option<usize> {
            if long { u32_at(loca, i.checked_mul(4)?).map(|v| v as usize) } else { u16_at(loca, i.checked_mul(2)?).map(|v| usize::from(v) * 2) }
        };
        let mut glyphs = Vec::with_capacity(count);
        for gid in 0..count {
            let (start, end) =
                offset(gid).zip(offset(gid + 1)).ok_or_else(|| TablesError::invalid("the loca table is shorter than the glyph count"))?;
            let bytes = if start >= end {
                Vec::new()
            } else {
                glyf.get(start..end).ok_or_else(|| TablesError::invalid(format!("glyph {gid} lies outside the glyf table")))?.to_vec()
            };
            let parsed = parse(&bytes);
            glyphs.push(Glyph { bytes, parsed, moved: false });
        }
        let names = glyph_names(font, count);
        let hhea = font.get(*b"hhea").ok_or_else(|| TablesError::MissingTable("hhea".into()))?;
        let hmtx = font.get(*b"hmtx").ok_or_else(|| TablesError::MissingTable("hmtx".into()))?;
        let number_of_h_metrics = usize::from(u16_at(hhea, 34).ok_or_else(|| TablesError::invalid("the hhea table is cut short"))?).min(count);
        if number_of_h_metrics == 0 && count > 0 {
            return Err(TablesError::invalid("the hhea table gives no horizontal metrics"));
        }
        let mut metrics = Vec::with_capacity(count);
        let mut last_advance = 0u16;
        for gid in 0..count {
            let m = if gid < number_of_h_metrics {
                let at = gid.saturating_mul(4);
                let advance = u16_at(hmtx, at).ok_or_else(|| TablesError::invalid("the hmtx table is cut short"))?;
                last_advance = advance;
                (advance, i16_at(hmtx, at.saturating_add(2)).ok_or_else(|| TablesError::invalid("the hmtx table is cut short"))?)
            } else {
                let at = number_of_h_metrics.saturating_mul(4).saturating_add((gid - number_of_h_metrics).saturating_mul(2));
                // A font may leave the trailing side bearings out; they read as 0.
                (last_advance, i16_at(hmtx, at).unwrap_or(0))
            };
            metrics.push(m);
        }
        Ok(Self { glyphs, names, metrics, number_of_h_metrics, advances_changed: false, bytes_changed: false, font_visits: Cell::new(0) })
    }

    /// The glyph ids `options` names, in glyph order.
    fn selection(&self, options: &OutlineOptions) -> Result<Vec<usize>> {
        let index: BTreeMap<&str, usize> = self.names.iter().enumerate().rev().map(|(i, n)| (n.as_str(), i)).collect();
        let look =
            |name: &String| index.get(name.as_str()).copied().ok_or_else(|| TablesError::BadOptions(format!("the font has no glyph named `{name}`")));
        let mut chosen: BTreeSet<usize> =
            if options.glyphs.is_empty() { (0..self.glyphs.len()).collect() } else { options.glyphs.iter().map(look).collect::<Result<_>>()? };
        for name in &options.exclude_glyphs {
            chosen.remove(&look(name)?);
        }
        Ok(chosen.into_iter().collect())
    }

    fn name(&self, gid: usize) -> String {
        self.names.get(gid).cloned().unwrap_or_else(|| format!("glyph{gid:05}"))
    }

    /// Records a failure, or fails the whole run without `ignore_errors`.
    fn fail(&self, gid: usize, reason: String, options: &OutlineOptions, report: &mut Report) -> Result<()> {
        let glyph = self.name(gid);
        if !options.ignore_errors {
            return Err(TablesError::Invalid(format!("glyph `{glyph}`: {reason}")));
        }
        report.failed.push(GlyphFailure { glyph, reason });
        Ok(())
    }

    /// Replaces a glyph by a simple glyph with `outline` and `instructions`.
    fn set_simple(&mut self, gid: usize, outline: Outline, instructions: Vec<u8>) -> std::result::Result<(), String> {
        let bytes = encode_simple(&outline, &instructions)?;
        let parsed = parse(&bytes);
        if let Parsed::Bad(why) = parsed {
            return Err(why);
        }
        if let Some(g) = self.glyphs.get_mut(gid) {
            *g = Glyph { bytes, parsed, moved: true };
            self.bytes_changed = true;
        }
        Ok(())
    }

    fn set_composite(&mut self, gid: usize, comps: Vec<Comp>, instructions: Vec<u8>) -> std::result::Result<(), String> {
        let bbox = self.glyphs.get(gid).and_then(|g| bbox_in_header(&g.bytes)).unwrap_or([0; 4]);
        let bytes = encode_composite(&comps, &instructions, bbox)?;
        let parsed = parse(&bytes);
        if let Parsed::Bad(why) = parsed {
            return Err(why);
        }
        if let Some(g) = self.glyphs.get_mut(gid) {
            *g = Glyph { bytes, parsed, moved: true };
            self.bytes_changed = true;
        }
        Ok(())
    }

    fn strip(&mut self, gid: usize) {
        if let Some(g) = self.glyphs.get_mut(gid)
            && let Some(bytes) = strip_instructions(g)
        {
            g.parsed = parse(&bytes);
            g.bytes = bytes;
            self.bytes_changed = true;
        }
    }

    /// The glyph's outline with its components decomposed.
    fn draw(&self, gid: usize) -> std::result::Result<Outline, String> {
        let mut visits = 0usize;
        self.draw_at(gid, 0, &mut visits)
    }

    fn draw_at(&self, gid: usize, depth: usize, visits: &mut usize) -> std::result::Result<Outline, String> {
        let g = self.glyphs.get(gid).ok_or_else(|| format!("a component refers to glyph {gid}, which the font does not have"))?;
        match &g.parsed {
            Parsed::Empty => Ok(Vec::new()),
            Parsed::Simple { outline, .. } => Ok(outline.clone()),
            Parsed::Bad(why) => Err(why.clone()),
            Parsed::Composite { comps, .. } => {
                let parts = self.parts_at(comps, depth, visits)?;
                Ok(parts.into_iter().flatten().collect())
            }
        }
    }

    /// Each component of a composite drawn in place.
    fn parts_at(&self, comps: &[Comp], depth: usize, visits: &mut usize) -> std::result::Result<Vec<Outline>, String> {
        if depth >= MAX_COMPONENT_DEPTH {
            return Err(format!("components are nested more than {MAX_COMPONENT_DEPTH} deep, or refer to one another"));
        }
        let mut parts: Vec<Outline> = Vec::with_capacity(comps.len());
        let mut points = 0usize;
        for c in comps {
            *visits = visits.saturating_add(1);
            if *visits > MAX_COMPONENT_VISITS {
                return Err(format!("drawing the glyph visits more than {MAX_COMPONENT_VISITS} components"));
            }
            if !self.visit() {
                return Err(FONT_VISITS_MESSAGE.to_string());
            }
            let child = self.draw_at(usize::from(c.gid), depth + 1, visits)?;
            let [xx, yx, xy, yy] = c.m;
            let mut placed: Outline =
                child.iter().map(|ct| ct.iter().map(|p| Pt { x: xx * p.x + xy * p.y, y: yx * p.x + yy * p.y, on: p.on }).collect()).collect();
            let (dx, dy) = match c.anchor {
                Anchor::Offset(x, y) if c.flags & SCALED_OFFSET != 0 && c.flags & UNSCALED_OFFSET == 0 => (xx * x + xy * y, yx * x + yy * y),
                Anchor::Offset(x, y) => (x, y),
                Anchor::Points(base, own) => {
                    let base_pt =
                        parts.iter().flatten().flatten().nth(usize::from(base)).ok_or("a component is placed by a point the glyph does not have")?;
                    let own_pt = placed.iter().flatten().nth(usize::from(own)).ok_or("a component is placed by a point it does not have")?;
                    (base_pt.x - own_pt.x, base_pt.y - own_pt.y)
                }
            };
            for p in placed.iter_mut().flatten() {
                p.x += dx;
                p.y += dy;
            }
            points = points.saturating_add(placed.iter().map(Vec::len).sum::<usize>());
            if points > MAX_GLYPH_POINTS {
                return Err("the decomposed glyph has more points than a TrueType glyph can hold".into());
            }
            parts.push(placed);
        }
        Ok(parts)
    }

    /// Component nesting depth: 0 for a simple glyph, 1 for a composite of simple glyphs.
    /// Counts one component reference against the font-wide budget; false once it is spent.
    fn visit(&self) -> bool {
        let n = self.font_visits.get().saturating_add(1);
        self.font_visits.set(n);
        n <= MAX_FONT_COMPONENT_VISITS
    }

    /// The nesting depth of `gid`; once the font-wide visit budget is spent, the depth reached so
    /// far.
    fn depth(&self, gid: usize, level: usize) -> usize {
        if !self.visit() {
            return level;
        }
        match self.glyphs.get(gid).map(|g| &g.parsed) {
            Some(Parsed::Composite { comps, .. }) if level < MAX_COMPONENT_DEPTH => {
                1 + comps.iter().map(|c| self.depth(usize::from(c.gid), level + 1)).max().unwrap_or(0)
            }
            Some(Parsed::Composite { .. }) => level,
            _ => 0,
        }
    }

    // ── The actions ─────────────────────────────────────────────────────────────────────────────

    fn remove_overlaps(&mut self, selected: &[usize], options: &OutlineOptions, report: &mut Report) -> Result<()> {
        let mut order: Vec<(usize, usize)> = selected.iter().map(|&gid| (self.depth(gid, 0), gid)).collect();
        order.sort_unstable();
        for (_, gid) in order {
            let take_part = match self.glyphs.get(gid).map(|g| &g.parsed) {
                Some(Parsed::Simple { outline, .. }) => !outline.is_empty(),
                Some(Parsed::Composite { comps, .. }) => match self.components_overlap(comps) {
                    Ok(overlap) => overlap,
                    Err(why) => {
                        self.fail(gid, why, options, report)?;
                        continue;
                    }
                },
                Some(Parsed::Bad(why)) => {
                    let why = why.clone();
                    self.fail(gid, why, options, report)?;
                    continue;
                }
                _ => false,
            };
            if take_part {
                match self.merged(gid, options.min_area) {
                    Ok(Some(outline)) => {
                        if let Err(why) = self.set_simple(gid, outline, Vec::new()) {
                            self.fail(gid, why, options, report)?;
                        } else {
                            report.changed.insert(gid);
                        }
                        continue;
                    }
                    Ok(None) => {}
                    Err(why) => {
                        self.fail(gid, why, options, report)?;
                        continue;
                    }
                }
            }
            if !options.keep_hinting {
                self.strip(gid);
            }
        }
        Ok(())
    }

    /// Does any component of the composite overlap another one?
    fn components_overlap(&self, comps: &[Comp]) -> std::result::Result<bool, String> {
        if comps.len() < 2 {
            return Ok(false);
        }
        if comps.len() > MAX_PAIRWISE_COMPONENTS {
            return Ok(true);
        }
        let mut visits = 0usize;
        let parts = self.parts_at(comps, 1, &mut visits)?;
        let paths = parts.iter().map(|p| bez_of(p)).collect::<std::result::Result<Vec<_>, _>>()?;
        limited(move || {
            for (i, a) in paths.iter().enumerate() {
                for b in paths.iter().skip(i + 1) {
                    if !po::intersection(std::slice::from_ref(a), std::slice::from_ref(b))?.is_empty() {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        })
    }

    /// The glyph drawn and merged; `None` when merging changes nothing.
    fn merged(&self, gid: usize, min_area: f64) -> std::result::Result<Option<Outline>, String> {
        let drawn = self.draw(gid)?;
        let contours: Vec<po::Contour> = drawn.iter().map(|c| to_po(c)).collect();
        let options = po::OverlapOptions { grid: 1.0, open_paths: po::OpenPaths::Remove };
        let result = limited(move || po::remove_overlap(&contours, options))?;
        let mut merged = from_po(&result)?;
        if signed_area(&drawn) < 0.0 {
            merged = merged.iter().map(|c| reversed(c)).collect();
        }
        if same_unordered(&drawn, &merged) {
            return Ok(None);
        }
        merged.retain(|c| area_of_bounds(c) >= min_area);
        Ok(Some(merged))
    }

    fn correct_direction(&mut self, selected: &[usize], options: &OutlineOptions, report: &mut Report) -> Result<()> {
        for &gid in selected {
            let outline = match self.glyphs.get(gid).map(|g| &g.parsed) {
                Some(Parsed::Simple { outline, .. }) if !outline.is_empty() => outline.clone(),
                _ => continue,
            };
            let result = match options.winding {
                Winding::Reverse => Ok(outline.iter().map(|c| reversed(c)).collect()),
                Winding::PostScript => directed(&outline, false),
                Winding::Auto | Winding::TrueType => directed(&outline, true),
            };
            match result {
                Ok(result) if !same_ordered(&outline, &result) => match self.set_simple(gid, result, Vec::new()) {
                    Ok(()) => {
                        report.changed.insert(gid);
                    }
                    Err(why) => self.fail(gid, why, options, report)?,
                },
                Ok(_) => {}
                Err(why) => self.fail(gid, why, options, report)?,
            }
        }
        Ok(())
    }

    fn add_extremes(&mut self, selected: &[usize], options: &OutlineOptions, report: &mut Report) -> Result<()> {
        let extremes = po::ExtremesOptions { force: options.min_segment == 0.0, grid: 1.0 };
        for &gid in selected {
            let outline = match self.glyphs.get(gid).map(|g| &g.parsed) {
                Some(Parsed::Simple { outline, .. }) if !outline.is_empty() => outline.clone(),
                _ => continue,
            };
            let contours: Vec<po::Contour> = outline.iter().map(|c| to_po(c)).collect();
            let done = limited(move || {
                contours
                    .iter()
                    .map(|c| if c.nodes.len() > 1 { po::add_extremes(c, None, extremes) } else { Ok(c.clone()) })
                    .collect::<std::result::Result<Vec<_>, _>>()
            });
            match done.and_then(|c| from_po(&c)) {
                Ok(result) if !same_ordered(&outline, &result) => {
                    let before: usize = outline.iter().map(Vec::len).sum();
                    let after: usize = result.iter().map(Vec::len).sum();
                    match self.set_simple(gid, result, Vec::new()) {
                        Ok(()) => {
                            report.changed.insert(gid);
                            report.points_added.insert(self.name(gid), after.saturating_sub(before));
                        }
                        Err(why) => self.fail(gid, why, options, report)?,
                    }
                }
                Ok(_) => {}
                Err(why) => self.fail(gid, why, options, report)?,
            }
        }
        Ok(())
    }

    fn round(&mut self, selected: &[usize], options: &OutlineOptions, report: &mut Report) -> Result<()> {
        let (grid, tolerance) = (options.grid, options.tolerance);
        let r = |v: f64| {
            let g = (v / grid).round() * grid;
            if tolerance.is_none_or(|t| (v - g).abs() <= t) { ot_round(g) } else { v }
        };
        for &gid in selected {
            let mut moved = 0usize;
            let parsed = self.glyphs.get(gid).map(|g| g.parsed.clone());
            let result = match parsed {
                Some(Parsed::Simple { outline, instructions, .. }) => {
                    let rounded: Outline = outline
                        .iter()
                        .map(|c| {
                            c.iter()
                                .map(|p| {
                                    let q = Pt { x: r(p.x), y: r(p.y), on: p.on };
                                    moved += usize::from(q.x != p.x) + usize::from(q.y != p.y);
                                    q
                                })
                                .collect()
                        })
                        .collect();
                    if moved > 0 { self.set_simple(gid, rounded, instructions) } else { Ok(()) }
                }
                Some(Parsed::Composite { comps, instructions, .. }) => {
                    let rounded: Vec<Comp> = comps
                        .iter()
                        .map(|c| match c.anchor {
                            Anchor::Offset(x, y) => {
                                let (nx, ny) = (r(x), r(y));
                                moved += usize::from(nx != x) + usize::from(ny != y);
                                Comp { anchor: Anchor::Offset(nx, ny), ..*c }
                            }
                            Anchor::Points(..) => *c,
                        })
                        .collect();
                    if moved > 0 { self.set_composite(gid, rounded, instructions) } else { Ok(()) }
                }
                _ => Ok(()),
            };
            if let Err(why) = result {
                self.fail(gid, why, options, report)?;
                continue;
            }
            if let Some(m) = self.metrics.get_mut(gid) {
                let advance = r(f64::from(m.0)).clamp(0.0, f64::from(u16::MAX)) as u16;
                if advance != m.0 {
                    m.0 = advance;
                    moved += 1;
                    self.advances_changed = true;
                }
            }
            if moved > 0 {
                report.changed.insert(gid);
                report.values_changed = report.values_changed.saturating_add(moved);
            }
        }
        Ok(())
    }

    fn decompose(&mut self, selected: &[usize], options: &OutlineOptions, report: &mut Report) -> Result<()> {
        // Every glyph is decomposed from the input's outlines, as in ttfdecomponentizer, where
        // drawing a composite always reaches the simple glyphs at the bottom.
        let input = Work {
            glyphs: self.glyphs.clone(),
            names: Vec::new(),
            metrics: Vec::new(),
            number_of_h_metrics: 0,
            advances_changed: false,
            bytes_changed: false,
            font_visits: Cell::new(0),
        };
        for &gid in selected {
            let Some(Parsed::Composite { comps, .. }) = self.glyphs.get(gid).map(|g| &g.parsed) else { continue };
            let comps = comps.clone();
            let result = if options.nested_only {
                if comps.iter().all(|c| !matches!(input.glyphs.get(usize::from(c.gid)).map(|g| &g.parsed), Some(Parsed::Composite { .. }))) {
                    continue;
                }
                input.flattened(&comps, 0).and_then(|flat| self.set_composite(gid, flat, Vec::new()))
            } else {
                if options.transformed_only && comps.iter().all(|c| c.m == IDENTITY) {
                    continue;
                }
                input.draw(gid).and_then(|outline| self.set_simple(gid, outline.iter().map(|c| from_first_on_curve(c)).collect(), Vec::new()))
            };
            match result {
                Ok(()) => {
                    report.changed.insert(gid);
                }
                Err(why) => self.fail(gid, why, options, report)?,
            }
        }
        Ok(())
    }

    /// The components with every composite among them replaced by its own components, placed.
    fn flattened(&self, comps: &[Comp], depth: usize) -> std::result::Result<Vec<Comp>, String> {
        if depth >= MAX_COMPONENT_DEPTH {
            return Err(format!("components are nested more than {MAX_COMPONENT_DEPTH} deep, or refer to one another"));
        }
        let mut out = Vec::with_capacity(comps.len());
        for c in comps {
            let inner = match self.glyphs.get(usize::from(c.gid)).map(|g| &g.parsed) {
                Some(Parsed::Composite { comps, .. }) => self.flattened(comps, depth + 1)?,
                _ => {
                    out.push(*c);
                    continue;
                }
            };
            let Anchor::Offset(ox, oy) = c.anchor else { return Err("a component placed by point numbers cannot be flattened".into()) };
            let [a, b, cc, d] = c.m;
            let outer_offset =
                if c.flags & SCALED_OFFSET != 0 && c.flags & UNSCALED_OFFSET == 0 { (a * ox + cc * oy, b * ox + d * oy) } else { (ox, oy) };
            for i in inner {
                let Anchor::Offset(ix, iy) = i.anchor else { return Err("a component placed by point numbers cannot be flattened".into()) };
                let [ia, ib, ic, id] = i.m;
                let inner_offset =
                    if i.flags & SCALED_OFFSET != 0 && i.flags & UNSCALED_OFFSET == 0 { (ia * ix + ic * iy, ib * ix + id * iy) } else { (ix, iy) };
                let m = [a * ia + cc * ib, b * ia + d * ib, a * ic + cc * id, b * ic + d * id];
                let offset = (a * inner_offset.0 + cc * inner_offset.1 + outer_offset.0, b * inner_offset.0 + d * inner_offset.1 + outer_offset.1);
                if out.len() >= MAX_COMPONENT_VISITS {
                    return Err("flattening gives too many components".into());
                }
                let flags = (c.flags & (USE_MY_METRICS | ROUND_XY)) | (i.flags & OVERLAP_COMPOUND);
                out.push(Comp { flags, gid: i.gid, anchor: Anchor::Offset(ot_round(offset.0), ot_round(offset.1)), m });
            }
        }
        Ok(out)
    }

    // ── Writing ─────────────────────────────────────────────────────────────────────────────────

    /// The font with the new glyphs and every table that depends on them.
    fn finish(mut self, font: &SfntFont) -> Result<SfntFont> {
        if !self.bytes_changed && !self.advances_changed {
            return Ok(font.clone());
        }
        // Composite bounds follow their components, as fontTools recalculates them.
        let mut bounds: Vec<Option<[i16; 4]>> = Vec::with_capacity(self.glyphs.len());
        for gid in 0..self.glyphs.len() {
            let b = match self.glyphs.get(gid).map(|g| &g.parsed) {
                Some(Parsed::Composite { .. }) => match self.draw(gid) {
                    Ok(outline) => outline_bounds(&outline).or(Some([0; 4])),
                    Err(_) => self.glyphs.get(gid).and_then(|g| bbox_in_header(&g.bytes)),
                },
                Some(Parsed::Simple { .. } | Parsed::Bad(_)) => self.glyphs.get(gid).and_then(|g| bbox_in_header(&g.bytes)),
                _ => None,
            };
            bounds.push(b);
        }
        for (g, b) in self.glyphs.iter_mut().zip(&bounds) {
            if let (Parsed::Composite { .. }, Some(b)) = (&g.parsed, b)
                && bbox_in_header(&g.bytes) != Some(*b)
            {
                for (k, v) in b.iter().enumerate() {
                    put_u16(&mut g.bytes, 2 + 2 * k, *v as u16);
                }
                g.moved = true;
            }
        }
        // Left side bearings of moved glyphs follow their xMin.
        for ((g, b), m) in self.glyphs.iter().zip(&bounds).zip(self.metrics.iter_mut()) {
            if let (true, Some(b)) = (g.moved, b) {
                m.1 = b[0];
            }
        }
        let mut out = font.clone();
        let (glyf, loca, long) = build_glyf(&self.glyphs)?;
        out.insert(*b"glyf", glyf);
        out.insert(*b"loca", loca);
        let has_outline: Vec<bool> = self.glyphs.iter().map(|g| !g.bytes.is_empty() && i16_at(&g.bytes, 0).is_some_and(|n| n != 0)).collect();

        if let Some(head) = font.get(*b"head") {
            let mut head = head.to_vec();
            put_u16(&mut head, 50, u16::from(long));
            let drawn: Vec<[i16; 4]> = bounds.iter().zip(&has_outline).filter_map(|(b, &has)| b.filter(|_| has)).collect();
            let fb = if drawn.is_empty() {
                [0; 4]
            } else {
                [
                    drawn.iter().map(|b| b[0]).min().unwrap_or(0),
                    drawn.iter().map(|b| b[1]).min().unwrap_or(0),
                    drawn.iter().map(|b| b[2]).max().unwrap_or(0),
                    drawn.iter().map(|b| b[3]).max().unwrap_or(0),
                ]
            };
            for (k, v) in fb.iter().enumerate() {
                put_u16(&mut head, 36 + 2 * k, *v as u16);
            }
            let all_lsb_is_xmin = bounds.iter().zip(&has_outline).zip(&self.metrics).all(|((b, &has), m)| !has || b.is_none_or(|b| b[0] == m.1));
            if all_lsb_is_xmin && let Some(flags) = u16_at(&head, 16) {
                put_u16(&mut head, 16, flags | 0x2);
            }
            out.insert(*b"head", head);
        }
        if let Some(hmtx) = font.get(*b"hmtx") {
            out.insert(*b"hmtx", self.hmtx(hmtx));
        }
        if let Some(hhea) = font.get(*b"hhea") {
            let mut hhea = hhea.to_vec();
            let widths: Vec<(usize, i32)> = bounds
                .iter()
                .zip(&has_outline)
                .enumerate()
                .filter_map(|(gid, (b, &has))| b.filter(|_| has).map(|b| (gid, i32::from(b[2]) - i32::from(b[0]))))
                .collect();
            let advance_max = self.metrics.iter().map(|m| m.0).max().unwrap_or(0);
            put_u16(&mut hhea, 10, advance_max);
            let (mut min_lsb, mut min_rsb, mut max_extent) = (i32::MAX, i32::MAX, i32::MIN);
            for (gid, width) in &widths {
                let Some(&(advance, lsb)) = self.metrics.get(*gid) else { continue };
                min_lsb = min_lsb.min(i32::from(lsb));
                min_rsb = min_rsb.min(i32::from(advance) - i32::from(lsb) - width);
                max_extent = max_extent.max(i32::from(lsb) + width);
            }
            if widths.is_empty() {
                (min_lsb, min_rsb, max_extent) = (0, 0, 0);
            }
            for (at, v) in [(12, min_lsb), (14, min_rsb), (16, max_extent)] {
                put_u16(&mut hhea, at, v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16);
            }
            out.insert(*b"hhea", hhea);
        }
        if let Some(maxp) = font.get(*b"maxp").filter(|m| u32_at(m, 0) == Some(0x0001_0000) && m.len() >= 32) {
            out.insert(*b"maxp", self.maxp(maxp)?);
        }
        Ok(out)
    }

    fn hmtx(&self, old: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(old.len());
        for (gid, (advance, lsb)) in self.metrics.iter().enumerate() {
            if gid < self.number_of_h_metrics {
                out.extend_from_slice(&advance.to_be_bytes());
            }
            out.extend_from_slice(&lsb.to_be_bytes());
        }
        // Anything after the metrics the font had stays.
        if let Some(rest) = old.get(out.len()..) {
            out.extend_from_slice(rest);
        }
        out
    }

    fn maxp(&self, old: &[u8]) -> Result<Vec<u8>> {
        self.font_visits.set(0);
        let mut out = old.to_vec();
        let (mut points, mut contours, mut comp_points, mut comp_contours, mut elements, mut depth) =
            (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        for (gid, g) in self.glyphs.iter().enumerate() {
            match &g.parsed {
                Parsed::Simple { outline, .. } if !outline.is_empty() => {
                    points = points.max(outline.iter().map(Vec::len).sum());
                    contours = contours.max(outline.len());
                }
                Parsed::Composite { comps, .. } => {
                    if let Ok(outline) = self.draw(gid) {
                        comp_points = comp_points.max(outline.iter().map(Vec::len).sum());
                        comp_contours = comp_contours.max(outline.len());
                    }
                    elements = elements.max(comps.len());
                    depth = depth.max(self.depth(gid, 0));
                }
                _ => {}
            }
        }
        if self.font_visits.get() > MAX_FONT_COMPONENT_VISITS {
            return Err(TablesError::invalid(format!("cannot recompute maxp: {FONT_VISITS_MESSAGE}")));
        }
        let clamp = |v: usize| v.min(usize::from(u16::MAX)) as u16;
        for (at, v) in [(6, points), (8, contours), (10, comp_points), (12, comp_contours), (28, elements), (30, depth)] {
            put_u16(&mut out, at, clamp(v));
        }
        Ok(out)
    }
}

fn bbox_in_header(bytes: &[u8]) -> Option<[i16; 4]> {
    Some([i16_at(bytes, 2)?, i16_at(bytes, 4)?, i16_at(bytes, 6)?, i16_at(bytes, 8)?])
}

fn outline_bounds(outline: &[Vec<Pt>]) -> Option<[i16; 4]> {
    let mut it = outline.iter().flatten();
    let first = it.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in it {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    let c = |v: f64| ot_round(v).clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    Some([c(x0), c(y0), c(x1), c(y1)])
}

/// `glyf` and `loca` from the glyphs' bytes, with fontTools' layout: glyphs unpadded unless padding
/// odd ones to even lengths lets `loca` use short offsets. Returns whether `loca` is long.
fn build_glyf(glyphs: &[Glyph]) -> Result<(Vec<u8>, Vec<u8>, bool)> {
    let mut total = glyphs
        .iter()
        .try_fold(0usize, |sum, g| sum.checked_add(g.bytes.len()))
        .ok_or(TablesError::TooLarge { what: "the glyf table", limit: crate::limits::MAX_SFNT })?;
    let odd = glyphs.iter().filter(|g| g.bytes.len() % 2 == 1).count();
    let pad = total < 0x20000 && odd > 0 && total.saturating_add(odd) < 0x20000;
    if pad {
        total = total.saturating_add(odd);
    }
    if total > crate::limits::MAX_SFNT {
        return Err(TablesError::TooLarge { what: "the glyf table", limit: crate::limits::MAX_SFNT });
    }
    let mut glyf = Vec::with_capacity(total);
    let mut offsets = Vec::with_capacity(glyphs.len().saturating_add(1));
    for g in glyphs {
        offsets.push(glyf.len());
        glyf.extend_from_slice(&g.bytes);
        if pad && g.bytes.len() % 2 == 1 {
            glyf.push(0);
        }
    }
    offsets.push(glyf.len());
    let long = !(glyf.len() < 0x20000 && offsets.iter().all(|o| o % 2 == 0));
    let mut loca = Vec::with_capacity(offsets.len().saturating_mul(if long { 4 } else { 2 }));
    for o in offsets {
        if long {
            loca.extend_from_slice(&(o as u32).to_be_bytes());
        } else {
            loca.extend_from_slice(&((o / 2) as u16).to_be_bytes());
        }
    }
    if glyf.is_empty() {
        // An empty glyf table is refused by some readers; fontTools writes one zero byte.
        glyf.push(0);
    }
    Ok((glyf, loca, long))
}

/// Glyph names from `post`, made unique; `glyphNNNNN` where the font has none.
fn glyph_names(font: &SfntFont, count: usize) -> Vec<String> {
    let post = font.get(*b"post").and_then(|b| Post::read(FontData::new(b)).ok());
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(count);
    for gid in 0..count {
        let base = post
            .as_ref()
            .and_then(|p| p.glyph_name(GlyphId16::new(gid as u16)))
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("glyph{gid:05}"));
        let mut name = base.clone();
        let mut n = 1usize;
        while !seen.insert(name.clone()) {
            name = format!("{base}#{n}");
            n = n.saturating_add(1);
        }
        out.push(name);
    }
    out
}

/// The outline with the editor's direction rules; `truetype` reverses the result.
fn directed(outline: &[Vec<Pt>], truetype: bool) -> std::result::Result<Outline, String> {
    let (strays, closed): (Vec<Vec<Pt>>, Vec<Vec<Pt>>) = outline.iter().cloned().partition(|c| c.len() < 2);
    let contours: Vec<po::Contour> = closed.iter().map(|c| to_po(c)).collect();
    let directed = limited(move || po::correct_direction(&contours))?;
    let mut out = from_po(&directed.contours)?;
    if truetype {
        out = out.iter().map(|c| reversed(c)).collect();
    }
    out.extend(strays);
    Ok(out)
}

#[cfg(test)]
#[path = "outlines_tests.rs"]
mod tests;
