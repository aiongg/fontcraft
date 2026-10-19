//! The derived values `tables.recalculate` writes (M12.61): each field is recomputed from the
//! font's other data the way the audit's checks expect it, then written with
//! [`fontcraft_tables::patch`]. Field names follow `table.field`; several fields computed together
//! share one name (`head.bbox` for the four box fields), and the single fields of such a group
//! are accepted for it.

use std::collections::BTreeSet;

use fontcraft_audit::data::{code_pages, unicode_ranges};
use fontcraft_tables::patch::{self, Os2Upgrade, PatchOptions, PatchValue};
use fontcraft_tables::sfnt::Flavour;
use fontcraft_tables::{SfntFont, Tag};
use serde_json::{Value, json};
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::tables::glyf::Glyph as TtGlyph;
use skrifa::raw::types::GlyphId;
use skrifa::raw::{FontRef, TableProvider};

use crate::{EngineError, Result};

/// The fields recomputed when the call names none: the ones a compiler derives.
pub(super) const DEFAULT_FIELDS: &[&str] = &[
    "glyf.bounds",
    "head.bbox",
    "hhea.advanceWidthMax",
    "hhea.extents",
    "OS/2.xAvgCharWidth",
    "OS/2.charIndex",
    "OS/2.ulUnicodeRange",
    "OS/2.ulCodePageRange",
];

/// Every field, in the order they are recomputed, so that each sees the fields it depends on.
const ORDER: &[&str] = &[
    "sfntVersion",
    "glyf.bounds",
    "head.bbox",
    "head.flags",
    "hhea.advanceWidthMax",
    "hhea.extents",
    "hhea.caretSlope",
    "post.underlineThickness",
    "post.isFixedPitch",
    "OS/2.version",
    "OS/2.fsSelection",
    "OS/2.xAvgCharWidth",
    "OS/2.charIndex",
    "OS/2.ulUnicodeRange",
    "OS/2.ulCodePageRange",
    "OS/2.yStrikeoutSize",
    "OS/2.yStrikeoutPosition",
    "OS/2.ySubscriptXSize",
    "OS/2.ySubscriptYSize",
    "OS/2.ySubscriptXOffset",
    "OS/2.ySubscriptYOffset",
    "OS/2.ySuperscriptXSize",
    "OS/2.ySuperscriptYSize",
    "OS/2.ySuperscriptXOffset",
    "OS/2.ySuperscriptYOffset",
    "OS/2.usWeightClass",
    "OS/2.panose",
    "gasp",
    "prep",
];

/// The instructions that switch on dropout control with smart dropout: PUSHW 0x01FF, SCANCTRL,
/// PUSHB 4, SCANTYPE (the sequence the hinting check looks for).
const SMART_DROPOUT: [u8; 7] = [0xb8, 0x01, 0xff, 0x85, 0xb0, 0x04, 0x8d];
/// The `gasp` behaviour for all sizes: grid-fit, grayscale, symmetric grid-fit and smoothing.
const GASP_ALL: u16 = 0x0F;
/// Most glyphs whose box change is listed one by one.
const MAX_LISTED: usize = 1_000;

/// The canonical name of a field: a group name for any field of a group; `None` for a field
/// `tables.recalculate` does not compute.
pub(super) fn canonical(name: &str) -> Option<&'static str> {
    let group = match name {
        "head.xMin" | "head.yMin" | "head.xMax" | "head.yMax" | "head.bbox" => "head.bbox",
        "hhea.caretSlopeRise" | "hhea.caretSlopeRun" | "hhea.caretSlope" => "hhea.caretSlope",
        "hhea.minLeftSideBearing" | "hhea.minRightSideBearing" | "hhea.xMaxExtent" | "hhea.extents" => "hhea.extents",
        "OS/2.ulUnicodeRange1" | "OS/2.ulUnicodeRange2" | "OS/2.ulUnicodeRange3" | "OS/2.ulUnicodeRange4" | "OS/2.ulUnicodeRange" => {
            "OS/2.ulUnicodeRange"
        }
        "OS/2.ulCodePageRange1" | "OS/2.ulCodePageRange2" | "OS/2.ulCodePageRange" => "OS/2.ulCodePageRange",
        "OS/2.usFirstCharIndex" | "OS/2.usLastCharIndex" | "OS/2.charIndex" => "OS/2.charIndex",
        "glyf" | "glyf.bounds" => "glyf.bounds",
        "sfntVersion" | "sfnt.version" => "sfntVersion",
        other => return ORDER.iter().copied().find(|f| *f == other),
    };
    Some(group)
}

/// A step that does not apply to this font, and why.
struct NotApplicable(String);

type Step = std::result::Result<SfntFont, NotApplicable>;

fn na(why: impl Into<String>) -> Step {
    Err(NotApplicable(why.into()))
}

fn tables_err(e: fontcraft_tables::TablesError) -> EngineError {
    EngineError::Tables(e)
}

/// What the recomputation reads, from the font as it is at that step.
struct View {
    bytes: Vec<u8>,
}

impl View {
    fn new(font: &SfntFont) -> Result<Self> {
        Ok(Self { bytes: fontcraft_tables::write(font, &fontcraft_tables::WriteOptions::default())? })
    }

    fn font(&self) -> Result<FontRef<'_>> {
        FontRef::new(&self.bytes).map_err(|e| EngineError::Io(format!("the font cannot be read: {e}")))
    }
}

/// Every glyph's advance, as `hmtx` gives it.
fn advances(f: &FontRef<'_>) -> Option<Vec<u16>> {
    let count = f.maxp().ok()?.num_glyphs();
    let hmtx = f.hmtx().ok()?;
    let long = hmtx.h_metrics();
    let last = long.last()?.advance();
    Some((0..usize::from(count)).map(|i| long.get(i).map_or(last, |m| m.advance())).collect())
}

fn lsb(f: &FontRef<'_>, gid: usize) -> i16 {
    let gid = u32::try_from(gid).unwrap_or(u32::MAX);
    f.hmtx().ok().and_then(|h| h.side_bearing(GlyphId::new(gid))).unwrap_or(0)
}

/// Bounds of an outline: every point (`control`, the TrueType convention for stored boxes) or
/// the exact extent of the curves.
struct BoundsPen {
    control: bool,
    at: (f64, f64),
    bounds: Option<[f64; 4]>,
}

impl BoundsPen {
    fn new(control: bool) -> Self {
        Self { control, at: (0.0, 0.0), bounds: None }
    }

    fn add(&mut self, x: f64, y: f64) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        self.bounds = Some(match self.bounds {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }

    /// Adds the curve point at each `t` in (0, 1) where one coordinate has an extremum.
    fn extrema(&mut self, ts: impl IntoIterator<Item = f64>, at: impl Fn(f64) -> (f64, f64)) {
        for t in ts {
            if t > 0.0 && t < 1.0 {
                let (x, y) = at(t);
                self.add(x, y);
            }
        }
    }
}

/// The roots in (0, 1) of a t² + b t + c.
fn roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() < 1e-12 {
        return if b.abs() < 1e-12 { Vec::new() } else { vec![-c / b] };
    }
    let d = b * b - 4.0 * a * c;
    if d < 0.0 {
        return Vec::new();
    }
    let s = d.sqrt();
    vec![(-b + s) / (2.0 * a), (-b - s) / (2.0 * a)]
}

impl OutlinePen for BoundsPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.at = (f64::from(x), f64::from(y));
        self.add(self.at.0, self.at.1);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (p0, c, p1) = (self.at, (f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
        if self.control {
            self.add(c.0, c.1);
        } else {
            let at = |t: f64| {
                let u = 1.0 - t;
                (u * u * p0.0 + 2.0 * u * t * c.0 + t * t * p1.0, u * u * p0.1 + 2.0 * u * t * c.1 + t * t * p1.1)
            };
            let tx = roots(0.0, 2.0 * (p0.0 - 2.0 * c.0 + p1.0), 2.0 * (c.0 - p0.0));
            let ty = roots(0.0, 2.0 * (p0.1 - 2.0 * c.1 + p1.1), 2.0 * (c.1 - p0.1));
            self.extrema(tx.into_iter().chain(ty), at);
        }
        self.move_to(x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let p0 = self.at;
        let (c0, c1, p1) = ((f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
        if self.control {
            self.add(c0.0, c0.1);
            self.add(c1.0, c1.1);
        } else {
            let at = |t: f64| {
                let u = 1.0 - t;
                let f = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
                (f(p0.0, c0.0, c1.0, p1.0), f(p0.1, c0.1, c1.1, p1.1))
            };
            // The derivative of a cubic Bézier, divided by 3: a t² + b t + c.
            let coef = |a: f64, b: f64, c: f64, d: f64| (-a + 3.0 * b - 3.0 * c + d, 2.0 * (a - 2.0 * b + c), b - a);
            let (ax, bx, cx) = coef(p0.0, c0.0, c1.0, p1.0);
            let (ay, by, cy) = coef(p0.1, c0.1, c1.1, p1.1);
            self.extrema(roots(ax, bx, cx).into_iter().chain(roots(ay, by, cy)), at);
        }
        self.move_to(x, y);
    }
    fn close(&mut self) {}
}

/// The drawn bounds of a glyph, unhinted, at the default location.
fn drawn(f: &FontRef<'_>, gid: u32, control: bool) -> Option<[f64; 4]> {
    let glyph = f.outline_glyphs().get(GlyphId::new(gid))?;
    let mut pen = BoundsPen::new(control);
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
    pen.bounds
}

fn int_box(b: [f64; 4]) -> Result<[i16; 4]> {
    let [x0, y0, x1, y1] = b;
    let [x0, y0, x1, y1] = [x0.floor(), y0.floor(), x1.ceil(), y1.ceil()];
    let mut out = [0i16; 4];
    for (slot, v) in out.iter_mut().zip([x0, y0, x1, y1]) {
        if !(-32_768.0..=32_767.0).contains(&v) {
            return Err(EngineError::Io(format!("a glyph reaches {v}, beyond the 16-bit coordinate range")));
        }
        *slot = v as i16;
    }
    Ok(out)
}

/// Each glyph's box: from `glyf` points for TrueType outlines (composites drawn), the exact
/// curve extent for CFF. `None` for glyphs without an outline.
fn glyph_boxes(f: &FontRef<'_>) -> Result<Vec<Option<[i16; 4]>>> {
    let count = f.maxp().map_err(|e| EngineError::Io(format!("the maxp table cannot be read: {e}")))?.num_glyphs();
    let mut out = Vec::with_capacity(usize::from(count));
    match (f.loca(None), f.glyf()) {
        (Ok(loca), Ok(glyf)) => {
            for gid in 0..u32::from(count) {
                let b = match loca.get_glyf(GlyphId::new(gid), &glyf) {
                    Ok(Some(TtGlyph::Simple(s))) => {
                        let mut pen = BoundsPen::new(true);
                        s.points().for_each(|p| pen.add(f64::from(p.x), f64::from(p.y)));
                        pen.bounds
                    }
                    Ok(Some(TtGlyph::Composite(_))) => drawn(f, gid, true),
                    _ => None,
                };
                out.push(b.map(int_box).transpose()?);
            }
        }
        _ => {
            for gid in 0..u32::from(count) {
                out.push(drawn(f, gid, false).map(int_box).transpose()?);
            }
        }
    }
    Ok(out)
}

/// The boxes stored in `glyf`, for glyphs with outline data.
fn stored_boxes(f: &FontRef<'_>) -> Option<Vec<Option<[i16; 4]>>> {
    let (loca, glyf, maxp) = (f.loca(None).ok()?, f.glyf().ok()?, f.maxp().ok()?);
    Some(
        (0..u32::from(maxp.num_glyphs()))
            .map(|gid| match loca.get_glyf(GlyphId::new(gid), &glyf) {
                Ok(Some(g)) => Some([g.x_min(), g.y_min(), g.x_max(), g.y_max()]),
                _ => None,
            })
            .collect(),
    )
}

/// `round` as Python 3 does it: halves to even.
fn py_round(v: f64) -> f64 {
    v.round_ties_even()
}

fn half_up(v: f64) -> f64 {
    (v + 0.5).floor()
}

fn upm(f: &FontRef<'_>) -> Result<f64> {
    Ok(f64::from(f.head().map_err(|e| EngineError::Io(format!("the head table cannot be read: {e}")))?.units_per_em()))
}

fn code_points(f: &FontRef<'_>) -> BTreeSet<u32> {
    f.charmap().mappings().take(fontcraft_tables::limits::MAX_CODE_POINTS).filter(|(_, g)| g.to_u32() != 0).map(|(cp, _)| cp).collect()
}

fn glyph_name(f: &FontRef<'_>, gid: usize) -> Option<String> {
    let post = f.post().ok()?;
    let gid = u16::try_from(gid).ok()?;
    post.glyph_name(skrifa::raw::types::GlyphId16::new(gid)).map(str::to_string)
}

/// Every glyph but `.notdef`, `.null` and zero-width ones has one advance.
fn monospaced(f: &FontRef<'_>, adv: &[u16]) -> bool {
    let widths: BTreeSet<u16> = adv
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, w)| **w != 0)
        .filter(|(g, _)| !matches!(glyph_name(f, *g).as_deref(), Some(".null" | "NULL")))
        .map(|(_, w)| *w)
        .collect();
    widths.len() == 1
}

/// What the call changed, one entry per field.
pub(super) struct Changes(pub(super) Vec<Value>);

impl Changes {
    fn push(&mut self, field: &str, old: Value, new: Value) {
        self.0.push(json!({"field": field, "old": old, "new": new}));
    }
}

fn value_json(v: &PatchValue) -> Value {
    match v {
        PatchValue::Number(n) => json!(n),
        PatchValue::Text(t) => json!(t),
    }
}

/// Sets the fields of one table that differ, recording each change.
fn set(font: &SfntFont, table: Tag, fields: &[(&str, PatchValue)], changes: &mut Changes) -> Result<SfntFont> {
    let mut changed = Vec::new();
    for (name, new) in fields {
        let old = patch::read_field(font, table, name)?;
        if old != *new {
            let label = format!("{}.{name}", fontcraft_tables::sfnt::tag_str(table).trim_end());
            changes.push(&label, value_json(&old), value_json(new));
            changed.push(((*name).to_string(), new.clone()));
        }
    }
    if changed.is_empty() {
        return Ok(font.clone());
    }
    patch::patch_table(font, &PatchOptions { table, fields: changed }).map_err(tables_err)
}

fn num(v: f64) -> PatchValue {
    PatchValue::Number(v)
}

fn has(font: &SfntFont, tag: &Tag) -> bool {
    font.get(*tag).is_some()
}

fn os2_upgrade(view: &View) -> Result<Os2Upgrade> {
    let f = view.font()?;
    let points = code_points(&f);
    let pages = code_pages::bits_for(|cp| points.contains(&cp));
    let boxes = glyph_boxes(&f)?;
    let top = |c: char| f.charmap().map(c).and_then(|g| boxes.get(g.to_u32() as usize).copied().flatten()).map_or(0, |[_, _, _, y1]| y1);
    let max_context = fontcraft_tables::layout_install::max_context(&fontcraft_tables::read(&view.bytes, 0)?).unwrap_or(0);
    Ok(Os2Upgrade {
        code_page_range: [pages as u32, (pages >> 32) as u32],
        x_height: top('x'),
        cap_height: top('H'),
        default_char: 0,
        break_char: 0x20,
        max_context,
    })
}

fn os2_version(font: &SfntFont) -> Option<u16> {
    font.get(*b"OS/2").and_then(|d| d.get(..2)).map(|b| u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
}

/// Recomputes one field group.
fn step(field: &str, font: &SfntFont, changes: &mut Changes) -> Result<Step> {
    let view = View::new(font)?;
    let f = view.font()?;
    Ok(match field {
        "sfntVersion" => {
            let want = if has(font, b"glyf") {
                Flavour::TrueType
            } else if font.is_cff() {
                Flavour::Cff
            } else {
                return Ok(na("the font has neither TrueType nor CFF outlines"));
            };
            let mut out = font.clone();
            if out.flavour != want {
                let text = |fl: Flavour| format!("0x{:08X}", fl.sfnt_version());
                changes.push("sfntVersion", json!(text(out.flavour)), json!(text(want)));
                out.flavour = want;
            }
            Ok(out)
        }
        "glyf.bounds" => {
            let Some(stored) = stored_boxes(&f) else { return Ok(na("the font has no TrueType outlines")) };
            let fresh = glyph_boxes(&f)?;
            let mut set_boxes = Vec::new();
            for (gid, (old, new)) in stored.iter().zip(&fresh).enumerate() {
                if let (Some(old), Some(new)) = (old, new)
                    && old != new
                {
                    if set_boxes.len() < MAX_LISTED {
                        changes.push(&format!("glyf.bounds[{gid}]"), json!(old), json!(new));
                    }
                    set_boxes.push((u16::try_from(gid).unwrap_or(u16::MAX), *new));
                }
            }
            if set_boxes.is_empty() {
                return Ok(Ok(font.clone()));
            }
            Ok(patch::set_glyph_bounds(font, &set_boxes).map_err(tables_err)?)
        }
        "head.bbox" => {
            let boxes = if has(font, b"glyf") { stored_boxes(&f).unwrap_or_default() } else { glyph_boxes(&f)? };
            let mut all: Option<[i16; 4]> = None;
            for b in boxes.into_iter().flatten() {
                all = Some(match all {
                    None => b,
                    Some([ax0, ay0, ax1, ay1]) => {
                        let [bx0, by0, bx1, by1] = b;
                        [ax0.min(bx0), ay0.min(by0), ax1.max(bx1), ay1.max(by1)]
                    }
                });
            }
            let [x0, y0, x1, y1] = all.unwrap_or([0; 4]).map(f64::from);
            Ok(set(font, *b"head", &[("xMin", num(x0)), ("yMin", num(y0)), ("xMax", num(x1)), ("yMax", num(y1))], changes)?)
        }
        "head.flags" => {
            if !has(font, b"fpgm") {
                return Ok(na("the font has no TrueType instructions"));
            }
            let PatchValue::Number(flags) = patch::read_field(font, *b"head", "flags")? else { return Ok(na("head.flags is not a number")) };
            Ok(set(font, *b"head", &[("flags", num(f64::from(flags as u16 | 0x0008)))], changes)?)
        }
        "hhea.advanceWidthMax" => {
            let Some(adv) = advances(&f) else { return Ok(na("the hmtx table cannot be read")) };
            let max = adv.iter().copied().max().unwrap_or(0);
            Ok(set(font, *b"hhea", &[("advanceWidthMax", num(f64::from(max)))], changes)?)
        }
        "hhea.extents" => {
            let Some(adv) = advances(&f) else { return Ok(na("the hmtx table cannot be read")) };
            let boxes = if has(font, b"glyf") { stored_boxes(&f).unwrap_or_default() } else { glyph_boxes(&f)? };
            let (mut min_lsb, mut min_rsb, mut max_extent) = (i32::MAX, i32::MAX, i32::MIN);
            for (gid, b) in boxes.iter().enumerate() {
                let Some(b) = b else { continue };
                let [x0, _, x1, _] = *b;
                let left = i32::from(lsb(&f, gid));
                let extent = left + i32::from(x1) - i32::from(x0);
                min_lsb = min_lsb.min(left);
                min_rsb = min_rsb.min(i32::from(adv.get(gid).copied().unwrap_or(0)) - extent);
                max_extent = max_extent.max(extent);
            }
            if max_extent == i32::MIN {
                (min_lsb, min_rsb, max_extent) = (0, 0, 0);
            }
            let fields = [
                ("minLeftSideBearing", num(f64::from(min_lsb))),
                ("minRightSideBearing", num(f64::from(min_rsb))),
                ("xMaxExtent", num(f64::from(max_extent))),
            ];
            Ok(set(font, *b"hhea", &fields, changes)?)
        }
        "hhea.caretSlope" => {
            let angle = f.post().map_err(|e| EngineError::Io(format!("the post table cannot be read: {e}")))?.italic_angle().to_f64();
            let upm = upm(&f)?;
            let run = py_round((-angle).to_radians().tan() * upm);
            let rise = if run == 0.0 { 1.0 } else { upm };
            Ok(set(font, *b"hhea", &[("caretSlopeRise", num(rise)), ("caretSlopeRun", num(run))], changes)?)
        }
        "post.underlineThickness" => {
            let PatchValue::Number(now) = patch::read_field(font, *b"post", "underlineThickness")? else { return Ok(na("not a number")) };
            if now != 0.0 {
                return Ok(Ok(font.clone()));
            }
            Ok(set(font, *b"post", &[("underlineThickness", num(half_up(upm(&f)? * 0.05)))], changes)?)
        }
        "post.isFixedPitch" => {
            let Some(adv) = advances(&f) else { return Ok(na("the hmtx table cannot be read")) };
            let fixed = if monospaced(&f, &adv) { 1.0 } else { 0.0 };
            Ok(set(font, *b"post", &[("isFixedPitch", num(fixed))], changes)?)
        }
        "OS/2.version" | "OS/2.fsSelection" => {
            let Some(old) = os2_version(font) else { return Ok(na("the font has no OS/2 table")) };
            let mut out = if old < 4 {
                let up = patch::raise_os2_version(font, 4, &os2_upgrade(&view)?).map_err(tables_err)?;
                changes.push("OS/2.version", json!(old), json!(4));
                // From version 3 on the average width counts every glyph.
                if old < 3 { step("OS/2.xAvgCharWidth", &up, changes)?.unwrap_or(up) } else { up }
            } else {
                font.clone()
            };
            if field == "OS/2.fsSelection" {
                let PatchValue::Number(bits) = patch::read_field(&out, *b"OS/2", "fsSelection")? else { return Ok(na("not a number")) };
                out = set(&out, *b"OS/2", &[("fsSelection", num(f64::from(bits as u16 | 0x0080)))], changes)?;
            }
            Ok(out)
        }
        "OS/2.xAvgCharWidth" => {
            let (Some(version), Some(adv)) = (os2_version(font), advances(&f)) else { return Ok(na("the OS/2 or hmtx table cannot be read")) };
            let avg = if version >= 3 {
                let widths: Vec<f64> = adv.iter().filter(|w| **w > 0).map(|w| f64::from(*w)).collect();
                if widths.is_empty() {
                    return Ok(na("no glyph has a non-zero advance"));
                }
                py_round(widths.iter().sum::<f64>() / widths.len() as f64)
            } else {
                let charmap = f.charmap();
                let mut sum = 0.0;
                for (cp, weight) in XAVG_WEIGHTS {
                    let Some(w) = charmap.map(cp).and_then(|g| adv.get(g.to_u32() as usize)) else {
                        return Ok(na("the font lacks a letter a to z or the space, which the version 0 to 2 average needs"));
                    };
                    sum += f64::from(*w) * f64::from(weight);
                }
                half_up(sum / 1000.0)
            };
            Ok(set(font, *b"OS/2", &[("xAvgCharWidth", num(avg))], changes)?)
        }
        "OS/2.charIndex" => {
            let points = code_points(&f);
            let (first, last) = (points.first().copied().unwrap_or(0).min(0xFFFF), points.last().copied().unwrap_or(0).min(0xFFFF));
            Ok(set(font, *b"OS/2", &[("usFirstCharIndex", num(f64::from(first))), ("usLastCharIndex", num(f64::from(last)))], changes)?)
        }
        "OS/2.ulUnicodeRange" => {
            let bits = unicode_ranges::bits_for(code_points(&f));
            let word = |i: u32| num(f64::from((bits >> (32 * i)) as u32));
            let fields = [("ulUnicodeRange1", word(0)), ("ulUnicodeRange2", word(1)), ("ulUnicodeRange3", word(2)), ("ulUnicodeRange4", word(3))];
            Ok(set(font, *b"OS/2", &fields, changes)?)
        }
        "OS/2.ulCodePageRange" => {
            if os2_version(font).is_none_or(|v| v < 1) {
                return Ok(na("OS/2 version 0 has no code page fields; recalculate OS/2.version first"));
            }
            let points = code_points(&f);
            let computed = code_pages::bits_for(|cp| points.contains(&cp));
            let stored = match (patch::read_field(font, *b"OS/2", "ulCodePageRange1")?, patch::read_field(font, *b"OS/2", "ulCodePageRange2")?) {
                (PatchValue::Number(a), PatchValue::Number(b)) => code_pages::from_fields(a as u32, b as u32),
                _ => 0,
            };
            // Bits 30 and 31 (OEM and symbol character sets) are the designer's statement.
            let keep = (1u64 << 30) | (1u64 << 31);
            let bits = (computed & !keep) | (stored & keep);
            Ok(set(
                font,
                *b"OS/2",
                &[("ulCodePageRange1", num(f64::from(bits as u32))), ("ulCodePageRange2", num(f64::from((bits >> 32) as u32)))],
                changes,
            )?)
        }
        "OS/2.usWeightClass" => {
            let axes = fontcraft_tables::variations::fvar_axes(font).unwrap_or_default();
            let Some(wght) = axes.iter().find(|a| &a.tag == b"wght") else { return Ok(na("the font has no wght axis")) };
            Ok(set(font, *b"OS/2", &[("usWeightClass", num(wght.default.trunc().clamp(1.0, 1000.0)))], changes)?)
        }
        "OS/2.panose" => {
            let Some(adv) = advances(&f) else { return Ok(na("the hmtx table cannot be read")) };
            let PatchValue::Text(text) = patch::read_field(font, *b"OS/2", "panose")? else { return Ok(na("not text")) };
            let mut p: Vec<u8> = text.split(' ').filter_map(|s| s.parse().ok()).collect();
            p.resize(10, 0);
            if monospaced(&f, &adv) {
                match p.first().copied() {
                    Some(3 | 5) => p.get_mut(3).into_iter().for_each(|v| *v = 3),
                    _ => {
                        p.get_mut(0).into_iter().for_each(|v| *v = 2);
                        p.get_mut(3).into_iter().for_each(|v| *v = 9);
                    }
                }
            } else if p.get(3) == Some(&9) {
                p.get_mut(3).into_iter().for_each(|v| *v = 0);
            }
            let text = p.iter().map(u8::to_string).collect::<Vec<_>>().join(" ");
            Ok(set(font, *b"OS/2", &[("panose", PatchValue::Text(text))], changes)?)
        }
        "gasp" => {
            if !has(font, b"glyf") {
                return Ok(na("the font has no TrueType outlines"));
            }
            let table = patch::gasp_table(&[(0xFFFF, GASP_ALL)]).map_err(tables_err)?;
            let mut out = font.clone();
            if out.get(*b"gasp") != Some(table.as_slice()) {
                changes.push("gasp", json!(out.get(*b"gasp").map(<[u8]>::len)), json!("one range for all sizes, flags 0x0F"));
                out.insert(*b"gasp", table);
            }
            Ok(out)
        }
        "prep" => {
            if !has(font, b"glyf") {
                return Ok(na("the font has no TrueType outlines"));
            }
            let (out, added) = patch::append_prep(font, &SMART_DROPOUT, 1).map_err(tables_err)?;
            if added {
                changes.push("prep", json!(null), json!("smart dropout control"));
            }
            Ok(out)
        }
        strike if strike.starts_with("OS/2.y") => {
            let (Ok(head), Ok(post), Ok(os2)) = (f.head(), f.post(), f.os2()) else { return Ok(na("the head, post or OS/2 table cannot be read")) };
            let upm = f64::from(head.units_per_em());
            let x_height = os2.sx_height().map(f64::from).filter(|h| *h > 0.0);
            let sub_y_offset = half_up(upm * 0.075);
            let sup_y_offset = half_up(upm * 0.35);
            let angle = post.italic_angle().to_f64();
            let tan = (-angle).to_radians().tan();
            let value = match strike.trim_start_matches("OS/2.") {
                "yStrikeoutSize" => f64::from(post.underline_thickness().to_i16()),
                "yStrikeoutPosition" => x_height.map_or_else(|| half_up(upm * 0.22), |h| half_up(h * 0.6)),
                "ySubscriptXSize" | "ySuperscriptXSize" => half_up(upm * 0.65),
                "ySubscriptYSize" | "ySuperscriptYSize" => half_up(upm * 0.60),
                "ySubscriptYOffset" => sub_y_offset,
                "ySuperscriptYOffset" => sup_y_offset,
                "ySubscriptXOffset" => {
                    if angle == 0.0 {
                        0.0
                    } else {
                        -half_up(sub_y_offset * tan)
                    }
                }
                "ySuperscriptXOffset" => {
                    if angle == 0.0 {
                        0.0
                    } else {
                        half_up(sup_y_offset * tan)
                    }
                }
                _ => return Ok(na("not a strikeout or script field")),
            };
            if !value.is_finite() {
                return Ok(na("the italic angle gives no finite offset"));
            }
            Ok(set(font, *b"OS/2", &[(strike.trim_start_matches("OS/2."), num(value))], changes)?)
        }
        _ => na(format!("`{field}` is not a field this command recomputes")),
    })
}

/// Weights of the OS/2 version 0 to 2 average: a to z, then the space.
const XAVG_WEIGHTS: [(u32, u32); 27] = [
    (0x61, 64),
    (0x62, 14),
    (0x63, 27),
    (0x64, 35),
    (0x65, 100),
    (0x66, 20),
    (0x67, 14),
    (0x68, 42),
    (0x69, 63),
    (0x6A, 3),
    (0x6B, 6),
    (0x6C, 35),
    (0x6D, 20),
    (0x6E, 56),
    (0x6F, 56),
    (0x70, 17),
    (0x71, 4),
    (0x72, 49),
    (0x73, 56),
    (0x74, 71),
    (0x75, 31),
    (0x76, 10),
    (0x77, 18),
    (0x78, 3),
    (0x79, 18),
    (0x7A, 2),
    (0x20, 166),
];

/// The font with `fields` recomputed (the defaults when `None`) and then `values` written as
/// given. A named field that does not apply to the font is an error; a default one that does
/// not apply is left out.
pub(super) fn recalculate(font: &SfntFont, fields: Option<&[&'static str]>, values: &[(String, PatchValue)]) -> Result<(SfntFont, Vec<Value>)> {
    let wanted: Vec<&'static str> = fields.unwrap_or(DEFAULT_FIELDS).to_vec();
    let mut changes = Changes(Vec::new());
    let mut out = font.clone();
    for field in ORDER.iter().copied().filter(|f| wanted.contains(f)) {
        match step(field, &out, &mut changes)? {
            Ok(next) => out = next,
            Err(NotApplicable(why)) if fields.is_some() => {
                return Err(EngineError::BadParams { command: "tables.recalculate".into(), message: format!("`{field}`: {why}") });
            }
            Err(NotApplicable(_)) => {}
        }
    }
    for (name, value) in values {
        let Some((table, field)) = name.split_once('.') else {
            return Err(EngineError::BadParams {
                command: "tables.recalculate".into(),
                message: format!("`values` key `{name}` is not `table.field`"),
            });
        };
        let tag = fontcraft_tables::sfnt::parse_tag(table).map_err(tables_err)?;
        out = set(&out, tag, &[(field, value.clone())], &mut changes)?;
    }
    Ok((out, changes.0))
}
