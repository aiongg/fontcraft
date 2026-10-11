//! Font-wide metrics checks (M12.48): units per em, the italic angle and caret slope, the
//! advance-width summary fields, the monospace markers, the font bounding box, the vertical
//! metrics and the OS/2 strikeout and script fields of one compiled font. Runners take the
//! context of their scope ([`crate::BinaryCtx`]), the thresholds and a [`crate::Sink`]; their
//! registry entries go in `registry/metrics_font.rs`.
//!
//! The rules follow FontBakery 1.1.0 (https://github.com/fonttools/fontbakery, Apache License
//! 2.0, Copyright the FontBakery authors) and the single-face tests 12, 13, 15, 16 and 27 of
//! comparefamily in AFDKO 5.0.1 (https://github.com/adobe-type-tools/afdko, Apache License 2.0,
//! Copyright Adobe). Each check's row in `spec/inventory/21-qa-checks.toml` names its sources and
//! where this port departs from them.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_geom::{BezPath, Line, ParamCurve, Rect, Shape};
use read_fonts::ps::cff::dict::{Entry, entries};
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlinePen};
use skrifa::prelude::{LocationRef, Size};

use crate::{BinaryCtx, Hit, Location, Params, Sink, Threshold, num};

/// Most cmap mappings one check walks: one per Unicode code point.
const MAX_CMAP_MAPPINGS: usize = 0x11_0000;
/// Most top DICT entries read from a CFF table.
const MAX_DICT_ENTRIES: usize = 1_024;
/// Most glyph names one message lists.
const MAX_NAMES_IN_MESSAGE: usize = 20;
/// A font with more code points than this in the CJK blocks is a CJK font.
const CJK_MIN_CODE_POINTS: usize = 150;

/// The CJK blocks FontBakery counts (its `CJK_UNICODE_RANGES`).
const CJK_RANGES: &[(u32, u32)] = &[
    (0x1100, 0x11FF),
    (0x3040, 0x309F),
    (0x30A0, 0x30FF),
    (0x31F0, 0x31FF),
    (0x3100, 0x312F),
    (0x31A0, 0x31BF),
    (0x3130, 0x318F),
    (0x3200, 0x32FF),
    (0x3300, 0x33FF),
    (0xAC00, 0xD7AF),
    (0x4E00, 0x9FFF),
    (0x2E80, 0x2EFF),
    (0x2F00, 0x2FDF),
    (0x2FF0, 0x2FFF),
    (0x3400, 0x4DBF),
    (0x20000, 0x2A6DF),
    (0x3190, 0x319F),
    (0x31C0, 0x31EF),
    (0xF900, 0xFAFF),
    (0x2F800, 0x2FA1F),
];

/// `check.metrics.verticalMetricsScheme`: which house scheme the vertical metrics follow.
pub(crate) const SCHEME: Threshold = Threshold {
    name: "scheme",
    default: 0.0,
    min: 0.0,
    max: 2.0,
    doc: "The vertical-metrics scheme: 0 Google Fonts (its CJK scheme for CJK fonts), 1 Microsoft, 2 Type Network.",
};

// ---------------------------------------------------------------------------------------------
// Shared readers
// ---------------------------------------------------------------------------------------------

fn no_table(tag: &str) -> String {
    format!("the font has no readable '{tag}' table")
}

/// The location of one field of one table, with the source field behind it when the font was
/// compiled from a source in this run.
fn field(ctx: &BinaryCtx<'_>, table: &str, name: &str) -> Location {
    let key = format!("{table}.{name}");
    Location {
        table: Some(table.to_string()),
        field: Some(name.to_string()),
        source_field: ctx.source.and_then(|s| s.field(&key)).map(str::to_string),
        ..Location::default()
    }
}

/// A glyph's name: the source glyph, else the font's own name for it.
fn glyph_name(ctx: &BinaryCtx<'_>, gid: GlyphId) -> String {
    if let Some(name) = ctx.source.and_then(|s| s.glyph(gid.to_u32())) {
        return name.to_string();
    }
    ctx.font.glyph_names().get(gid).map(|n| n.as_str().to_string()).unwrap_or_else(|| format!("glyph {}", gid.to_u32()))
}

/// The advance of every glyph, as fontTools' `hmtx` lists them: glyphs past the long metrics
/// take the last long advance.
fn advances(font: &FontRef<'_>) -> Option<Vec<u16>> {
    let count = font.maxp().ok()?.num_glyphs();
    let hmtx = font.hmtx().ok()?;
    let long = hmtx.h_metrics();
    let last = long.last()?.advance();
    Some((0..usize::from(count)).map(|i| long.get(i).map_or(last, |m| m.advance())).collect())
}

/// Collects an outline into a path.
struct PathPen(BezPath);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// The unhinted outline of a glyph at the default location, components resolved.
fn outline(outlines: &OutlineGlyphCollection<'_>, gid: GlyphId) -> Option<BezPath> {
    let glyph = outlines.get(gid)?;
    let mut pen = PathPen(BezPath::new());
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
    Some(pen.0)
}

/// The exact bounds of a path (curve extremes, not control points); `None` when it is empty.
fn bounds(path: &BezPath) -> Option<Rect> {
    if path.elements().is_empty() {
        return None;
    }
    let r = path.bounding_box();
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()).then_some(r)
}

/// The exact bounds of the glyph mapped to `cp`, with its id.
fn char_bounds(font: &FontRef<'_>, outlines: &OutlineGlyphCollection<'_>, cp: u32) -> Option<(GlyphId, Option<Rect>)> {
    let gid = font.charmap().map(cp)?;
    Some((gid, outline(outlines, gid).as_ref().and_then(bounds)))
}

/// More than 150 code points in the CJK blocks (FontBakery's `is_cjk_font`).
fn is_cjk(font: &FontRef<'_>) -> bool {
    font.charmap()
        .mappings()
        .take(MAX_CMAP_MAPPINGS)
        .filter(|(cp, _)| CJK_RANGES.iter().any(|(a, b)| (*a..=*b).contains(cp)))
        .nth(CJK_MIN_CODE_POINTS)
        .is_some()
}

/// The style name: name ID 17, else name ID 2, preferring Windows records.
fn style_name(font: &FontRef<'_>) -> Option<String> {
    let name = font.name().ok()?;
    let data = name.string_data();
    let find = |id: u16| {
        let mut records: Vec<_> = name.name_record().iter().filter(|r| r.name_id().to_u16() == id).collect();
        records.sort_by_key(|r| r.platform_id() != 3);
        records.into_iter().find_map(|r| r.string(data).ok().map(|s| s.to_string()))
    };
    find(17).or_else(|| find(2))
}

/// Whether the font is an italic style: the fsSelection italic bit, or "Italic" in the style
/// name (name IDs 17 and 2); `None` when the font has neither OS/2 nor a style name.
fn style_is_italic(font: &FontRef<'_>) -> Option<bool> {
    let bit = font.os2().ok().map(|o| o.fs_selection().bits() & 1 != 0);
    let named = style_name(font).map(|s| s.contains("Italic"));
    match (bit, named) {
        (None, None) => None,
        (b, n) => Some(b.unwrap_or(false) || n.unwrap_or(false)),
    }
}

/// What the first top DICT of a `CFF ` table says.
struct CffTop {
    fixed_pitch: bool,
    bbox: [f64; 4],
    cid: bool,
}

fn cff_top(font: &FontRef<'_>) -> Option<CffTop> {
    let cff = font.cff().ok()?;
    let data = cff.top_dicts().get(0)?;
    let mut top = CffTop { fixed_pitch: false, bbox: [0.0; 4], cid: false };
    for entry in entries(data, None).take(MAX_DICT_ENTRIES) {
        match entry {
            Ok(Entry::IsFixedPitch(b)) => top.fixed_pitch = b,
            Ok(Entry::FontBbox(b)) => top.bbox = b.map(|v| v.to_f64()),
            Ok(Entry::Ros { .. } | Entry::FdArrayOffset(_)) => top.cid = true,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Some(top)
}

/// Python's `round` on a float: halves go to the even neighbour.
fn py_round(v: f64) -> f64 {
    v.round_ties_even()
}

/// comparefamily's `int(0.5 + v)`: add a half, then truncate toward zero.
fn half_up(v: f64) -> f64 {
    (0.5 + v).trunc()
}

// ---------------------------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------------------------

/// `check.metrics.unitsPerEm`.
pub(crate) fn units_per_em(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(head) = ctx.font.head() else {
        sink.skip(no_table("head"));
        return;
    };
    let upem = head.units_per_em();
    let place = field(ctx, "head", "unitsPerEm");
    if !(16..=16_384).contains(&upem) {
        let msg = format!("head.unitsPerEm is {upem}; it must be from 16 to 16384.");
        sink.push(Hit::new("out-of-range", msg).data("actual", f64::from(upem)).place(place));
    } else if !(upem.is_power_of_two() || upem == 1000 || upem == 2000) {
        let msg = format!("head.unitsPerEm is {upem}; a power of two, 1000 or 2000 suits every renderer.");
        sink.push(Hit::new("suboptimal", msg).data("actual", f64::from(upem)).place(place));
    }
}

/// The leftmost crossing of the horizontal line at `y` (or up to 18 units above it) with the
/// path, as FontBakery's italic-angle measure finds it.
fn leftmost_crossing(path: &BezPath, b: Rect, y: f64) -> Option<f64> {
    for adjust in (0..20).step_by(2) {
        let yy = y + f64::from(adjust);
        let line = Line::new((b.x0 - 100.0, yy), (b.x1 + 100.0, yy));
        let x = path
            .segments()
            .flat_map(|s| s.intersect_line(line))
            .map(|i| line.eval(i.line_t).x)
            .filter(|x| x.is_finite())
            .fold(f64::INFINITY, f64::min);
        if x.is_finite() {
            return Some(x);
        }
    }
    None
}

/// The lean measured on the first of bar, bracketleft, H and I that has an outline, in the
/// sign convention of post.italicAngle (negative leans right).
fn measured_lean(font: &FontRef<'_>, outlines: &OutlineGlyphCollection<'_>) -> Option<f64> {
    for cp in [0x7Cu32, 0x5B, 0x48, 0x49] {
        let Some(gid) = font.charmap().map(cp) else { continue };
        let Some(path) = outline(outlines, gid) else { continue };
        let Some(b) = bounds(&path) else { continue };
        let bottom = b.y0 + b.height() * 0.2;
        let top = b.y0 + b.height() * 0.8;
        let (Some(xb), Some(xt)) = (leftmost_crossing(&path, b, bottom), leftmost_crossing(&path, b, top)) else { continue };
        let angle = -(xt - xb).atan2(top - bottom).to_degrees();
        if angle.is_finite() {
            return Some(angle);
        }
    }
    None
}

/// `check.metrics.italicAngle`.
pub(crate) fn italic_angle(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(post) = ctx.font.post() else {
        sink.skip(no_table("post"));
        return;
    };
    let value = post.italic_angle().to_f64();
    let place = || field(ctx, "post", "italicAngle");
    let outlines = ctx.font.outline_glyphs();
    let lean = measured_lean(&ctx.font, &outlines);
    let hit = |code: &'static str, msg: String| Hit::new(code, msg).data("actual", value);
    match lean {
        None if value > 0.0 => {
            let msg = format!("post.italicAngle is {} (positive, a lean to the left); right-leaning italics use a negative angle.", num(value));
            sink.push(hit("positive", msg).place(place()));
        }
        Some(l) if l < 0.1 && value > 0.0 => {
            let msg = format!("post.italicAngle is {} (positive) but the outlines lean {}°.", num(value), num(l));
            sink.push(hit("positive", msg).data("measured", l).place(place()));
        }
        Some(l) if l > 0.1 && value < 0.0 => {
            let msg = format!("post.italicAngle is {} (negative) but the outlines lean {}° to the left.", num(value), num(l));
            sink.push(hit("negative", msg).data("measured", l).place(place()));
        }
        _ => {}
    }
    if value.abs() > 90.0 {
        sink.push(hit("over-90-degrees", format!("post.italicAngle is {}, more than 90°.", num(value))).place(place()));
    }
    if value.abs() > 30.0 {
        sink.push(hit("over-30-degrees", format!("post.italicAngle is {}, steeper than 30°; confirm it.", num(value))).place(place()));
    } else if value.abs() > 20.0 {
        sink.push(hit("over-20-degrees", format!("post.italicAngle is {}, steeper than 20°; confirm it.", num(value))).place(place()));
    }
    match style_is_italic(&ctx.font) {
        Some(true) if value == 0.0 => {
            sink.push(hit("zero-italic", "The font is an italic style but post.italicAngle is 0.".into()).place(place()));
        }
        Some(false) if value != 0.0 => {
            let msg = format!("The font is an upright style but post.italicAngle is {}.", num(value));
            sink.push(hit("non-zero-upright", msg).place(place()));
        }
        _ => {}
    }
    let empty: Vec<String> = [0x7Cu32, 0x5B, 0x48, 0x49]
        .into_iter()
        .filter_map(|cp| char_bounds(&ctx.font, &outlines, cp))
        .filter(|(_, b)| b.is_none())
        .map(|(gid, _)| glyph_name(ctx, gid))
        .collect();
    if !empty.is_empty() {
        let msg = format!("These glyphs have no outline, so the lean cannot be measured on them: {}.", empty.join(", "));
        sink.push(Hit::new("empty-glyphs", msg).without_fix());
    }
}

/// `check.metrics.italicBitVsAngle` (comparefamily single-face test 12).
pub(crate) fn italic_bit_vs_angle(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(head), Ok(post)) = (ctx.font.head(), ctx.font.post()) else {
        sink.skip("the font has no readable 'head' or 'post' table");
        return;
    };
    let italic = head.mac_style().bits() & 0x02 != 0;
    let angle = post.italic_angle().to_f64();
    let place = field(ctx, "head", "macStyle");
    if italic && angle.abs() <= 4.0 {
        let msg = format!("The italic bit of head.macStyle is set but post.italicAngle is only {}°.", num(angle));
        sink.push(Hit::new("bit-without-angle", msg).data("italicAngle", angle).place(place));
    } else if !italic && angle.abs() > 4.0 {
        let msg = format!("The italic bit of head.macStyle is clear but post.italicAngle is {}°.", num(angle));
        sink.push(Hit::new("angle-without-bit", msg).data("italicAngle", angle).place(place));
    }
}

/// `check.metrics.caretSlope`.
pub(crate) fn caret_slope(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(hhea), Ok(post), Ok(head)) = (ctx.font.hhea(), ctx.font.post(), ctx.font.head()) else {
        sink.skip("the font has no readable 'hhea', 'post' or 'head' table");
        return;
    };
    let rise = hhea.caret_slope_rise();
    let run = hhea.caret_slope_run();
    let place = field(ctx, "hhea", "caretSlopeRise");
    if rise == 0 {
        sink.push(Hit::new("zero-rise", "hhea.caretSlopeRise is 0; upright fonts use 1.").place(place));
        return;
    }
    let post_angle = post.italic_angle().to_f64();
    let hhea_angle = (-f64::from(run) / f64::from(rise)).atan().to_degrees();
    let upm = f64::from(head.units_per_em());
    let expected_run = py_round((-post_angle).to_radians().tan() * upm);
    let expected_rise = if expected_run == 0.0 { 1.0 } else { upm };
    if (post_angle - hhea_angle).abs() > 0.1 {
        let msg = format!(
            "hhea.caretSlopeRise {rise} and caretSlopeRun {run} give {}°, but post.italicAngle is {}°; expected rise {} and run {}.",
            num(hhea_angle),
            num(post_angle),
            num(expected_rise),
            num(expected_run)
        );
        let hit = Hit::new("mismatch", msg)
            .data("caretSlopeRise", f64::from(rise))
            .data("caretSlopeRun", f64::from(run))
            .data("expectedRise", expected_rise)
            .data("expectedRun", expected_run);
        sink.push(hit.place(place));
    }
}

/// `check.metrics.maxAdvanceWidth`.
pub(crate) fn max_advance_width(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(hhea), Some(adv)) = (ctx.font.hhea(), advances(&ctx.font)) else {
        sink.skip("the font has no readable 'hhea', 'hmtx' or 'maxp' table");
        return;
    };
    let max = adv.iter().copied().max().unwrap_or(0);
    let actual = hhea.advance_width_max().to_u16();
    if actual != max {
        let msg = format!("hhea.advanceWidthMax is {actual} but the largest advance in hmtx is {max}.");
        let hit = Hit::new("mismatch", msg).data("actual", f64::from(actual)).data("expected", f64::from(max));
        sink.push(hit.place(field(ctx, "hhea", "advanceWidthMax")));
    }
}

/// Weights of the OS/2 version 0 to 2 average: a to z, then space.
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

/// `check.metrics.xAvgCharWidth`.
pub(crate) fn x_avg_char_width(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Some(adv)) = (ctx.font.os2(), advances(&ctx.font)) else {
        sink.skip("the font has no readable 'OS/2', 'hmtx' or 'maxp' table");
        return;
    };
    let current = f64::from(os2.x_avg_char_width());
    let (expected, rule) = if os2.version() >= 3 {
        let widths: Vec<f64> = adv.iter().filter(|w| **w > 0).map(|w| f64::from(*w)).collect();
        if widths.is_empty() {
            sink.skip("no glyph has a non-zero advance");
            return;
        }
        (py_round(widths.iter().sum::<f64>() / widths.len() as f64), "the average of the non-zero advances")
    } else {
        let charmap = ctx.font.charmap();
        let mut sum = 0.0;
        for (cp, weight) in XAVG_WEIGHTS {
            let Some(w) = charmap.map(cp).and_then(|g| adv.get(g.to_u32() as usize)) else {
                sink.skip("the font lacks a lowercase letter a to z or the space, which the version 0 to 2 average needs");
                return;
            };
            sum += f64::from(*w) * f64::from(weight);
        }
        (half_up(sum / 1000.0), "the weighted average of a to z and the space")
    };
    let difference = (current - expected).abs();
    if difference <= 1.0 {
        return;
    }
    let code = if difference < 10.0 { "close" } else { "wrong" };
    let msg = format!("OS/2.xAvgCharWidth is {} but {rule} is {}.", num(current), num(expected));
    let hit = Hit::new(code, msg).data("actual", current).data("expected", expected);
    sink.push(hit.place(field(ctx, "OS/2", "xAvgCharWidth")));
}

/// Is the font monospaced by FontBakery's rule? At least 80% of the ASCII characters have
/// glyphs: 80% of those glyphs share one width. Otherwise the glyphs of printable characters,
/// marks left out, have at most two non-zero widths.
fn seems_monospaced(font: &FontRef<'_>, adv: &[u16]) -> bool {
    let charmap = font.charmap();
    let ascii: Vec<u32> = (32u32..127).filter_map(|c| charmap.map(c)).map(|g| g.to_u32()).collect();
    if ascii.len() as f64 > 0.8 * 95.0 {
        let unique: BTreeSet<u32> = ascii.into_iter().collect();
        let widths: Vec<u16> = unique.iter().filter_map(|g| adv.get(*g as usize).copied()).filter(|w| *w != 0).collect();
        let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
        for w in &widths {
            *counts.entry(*w).or_default() += 1;
        }
        let top = counts.values().copied().max().unwrap_or(0);
        return top > 0 && top as f64 >= widths.len() as f64 * 0.8;
    }
    let marks = font.gdef().ok().and_then(|g| g.glyph_class_def()).and_then(Result::ok);
    let mut widths = BTreeSet::new();
    for (cp, gid) in charmap.mappings().take(MAX_CMAP_MAPPINGS) {
        let control = cp < 0x20 || (0x7F..=0x9F).contains(&cp);
        let separator = cp == 0x2028 || cp == 0x2029;
        let format = matches!(cp, 0xAD | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0xFEFF);
        let private = (0xE000..=0xF8FF).contains(&cp) || cp >= 0xF_0000;
        if control || separator || format || private || marks.as_ref().is_some_and(|c| c.get(gid) == 3) {
            continue;
        }
        if let Some(w) = adv.get(gid.to_u32() as usize).copied().filter(|w| *w != 0) {
            widths.insert(w);
            if widths.len() > 2 {
                return false;
            }
        }
    }
    true
}

/// PANOSE says monospaced: Latin Text with proportion 9, or Latin Hand Written or Latin Symbol
/// with spacing 3.
fn panose_monospaced(panose: &[u8]) -> bool {
    match (panose.first(), panose.get(3)) {
        (Some(2), Some(p)) => *p == 9,
        (Some(3 | 5), Some(p)) => *p == 3,
        _ => false,
    }
}

/// `check.metrics.monospaceFlags`.
pub(crate) fn monospace_flags(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(hhea), Ok(os2), Ok(post), Some(adv)) = (ctx.font.hhea(), ctx.font.os2(), ctx.font.post(), advances(&ctx.font)) else {
        sink.skip("the font has no readable 'hhea', 'hmtx', 'OS/2', 'post' or 'maxp' table");
        return;
    };
    let fixed = post.is_fixed_pitch();
    let panose = os2.panose_10();
    if seems_monospaced(&ctx.font, &adv) {
        let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
        for w in adv.iter().filter(|w| **w != 0) {
            *counts.entry(*w).or_default() += 1;
        }
        let common = counts.iter().max_by_key(|(w, n)| (**n, std::cmp::Reverse(**w))).map_or(0, |(w, _)| *w);
        let h_metrics = hhea.number_of_h_metrics();
        if h_metrics != 3 {
            let msg = format!("The font is monospaced; hhea.numberOfHMetrics is {h_metrics}, and 3 is recommended.");
            sink.push(
                Hit::new("number-of-hmetrics", msg).data("actual", f64::from(h_metrics)).place(field(ctx, "hhea", "numberOfHMetrics")).without_fix(),
            );
        }
        if !panose_monospaced(panose) {
            let msg = "The font is monospaced but its PANOSE values do not say so (proportion 9 for Latin Text, spacing 3 for Latin Hand Written and Latin Symbol).";
            sink.push(Hit::new("mono-bad-panose", msg).place(field(ctx, "OS/2", "panose")));
        }
        let outliers: Vec<usize> = adv
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, w)| **w != 0 && **w != common)
            .map(|(g, _)| g)
            .filter(|g| {
                let name = glyph_name(ctx, GlyphId::new(*g as u32));
                name != ".null" && name != "NULL"
            })
            .collect();
        if !outliers.is_empty() {
            let names: Vec<String> = outliers.iter().take(MAX_NAMES_IN_MESSAGE).map(|g| glyph_name(ctx, GlyphId::new(*g as u32))).collect();
            let more = if outliers.len() > MAX_NAMES_IN_MESSAGE { ", …" } else { "" };
            let msg =
                format!("The font is monospaced ({common} units) but {} glyphs have another width: {}{more}.", outliers.len(), names.join(", "));
            sink.push(
                Hit::new("mono-outliers", msg)
                    .data("count", outliers.len() as f64)
                    .data("width", f64::from(common))
                    .place(field(ctx, "hmtx", "advanceWidth"))
                    .without_fix(),
            );
        } else if fixed == 0 {
            let msg = "The font is monospaced but post.isFixedPitch is 0.";
            sink.push(Hit::new("mono-bad-is-fixed-pitch", msg).place(field(ctx, "post", "isFixedPitch")));
        }
    } else {
        if fixed != 0 {
            let msg = format!("The font is not monospaced but post.isFixedPitch is {fixed}.");
            sink.push(Hit::new("bad-is-fixed-pitch", msg).data("actual", f64::from(fixed)).place(field(ctx, "post", "isFixedPitch")));
        }
        if panose.get(3) == Some(&9) {
            let msg = "The font is not monospaced but its PANOSE proportion is 9 (monospaced).";
            sink.push(Hit::new("bad-panose", msg).place(field(ctx, "OS/2", "panose")));
        }
    }
    if let Some(top) = cff_top(&ctx.font)
        && top.fixed_pitch != (fixed != 0)
    {
        let msg = format!(
            "post.isFixedPitch says the font is {}monospaced but the CFF isFixedPitch says the opposite.",
            if fixed != 0 { "" } else { "not " }
        );
        sink.push(Hit::new("cff-is-fixed-pitch", msg).place(field(ctx, "CFF ", "isFixedPitch")));
    }
}

/// `check.metrics.fontBBox` (comparefamily single-face test 15).
pub(crate) fn font_bbox(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(head) = ctx.font.head() else {
        sink.skip(no_table("head"));
        return;
    };
    let upm = f64::from(head.units_per_em());
    let bbox = [head.x_min(), head.y_min(), head.x_max(), head.y_max()].map(f64::from);
    let fields = ["xMin", "yMin", "xMax", "yMax"];
    for (i, (v, name)) in bbox.iter().zip(fields).enumerate() {
        let limit = if i % 2 == 0 { 2.0 * upm } else { 1.5 * upm };
        if v.abs() > limit {
            let msg = format!("head.{name} is {}, beyond {} units from the origin.", num(*v), num(limit));
            sink.push(Hit::new("improbable", msg).data("actual", *v).data("limit", limit).place(field(ctx, "head", name)));
        }
    }
    if let Some(top) = cff_top(&ctx.font)
        && bbox.iter().zip(top.bbox).any(|(h, c)| (h - c).abs() > 1.0)
    {
        let show = |b: [f64; 4]| b.map(num).join(" ");
        let msg = format!("The head box ({}) differs from the CFF FontBBox ({}).", show(bbox), show(top.bbox));
        sink.push(Hit::new("head-cff-differ", msg).place(field(ctx, "CFF ", "FontBBox")));
    }
}

/// `check.metrics.typoMetricsVsEm` (comparefamily single-face test 16).
pub(crate) fn typo_metrics_vs_em(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Ok(head)) = (ctx.font.os2(), ctx.font.head()) else {
        sink.skip("the font has no readable 'OS/2' or 'head' table");
        return;
    };
    let upm = i32::from(head.units_per_em());
    let height = i32::from(os2.s_typo_ascender()) - i32::from(os2.s_typo_descender());
    if height != upm {
        let msg = format!(
            "OS/2.sTypoAscender {} minus sTypoDescender {} is {height}, not the em size {upm}.",
            os2.s_typo_ascender(),
            os2.s_typo_descender()
        );
        sink.push(Hit::new("typo-height", msg).data("actual", f64::from(height)).data("expected", f64::from(upm)).place(field(
            ctx,
            "OS/2",
            "sTypoAscender",
        )));
    }
    if cff_top(&ctx.font).is_some_and(|t| t.cid) && i32::from(os2.s_typo_line_gap()) != upm {
        let msg = format!("This CID-keyed font has OS/2.sTypoLineGap {}; the convention is the em size {upm}.", os2.s_typo_line_gap());
        sink.push(Hit::new("cid-line-gap", msg).data("actual", f64::from(os2.s_typo_line_gap())).place(field(ctx, "OS/2", "sTypoLineGap")));
    }
}

/// `check.metrics.hheaMatchesTypo`.
pub(crate) fn hhea_matches_typo(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Ok(hhea)) = (ctx.font.os2(), ctx.font.hhea()) else {
        sink.skip("the font has no readable 'OS/2' or 'hhea' table");
        return;
    };
    if is_cjk(&ctx.font) {
        sink.skip("the font is a CJK font");
        return;
    }
    let pairs = [
        ("ascender", "ascender", hhea.ascender().to_i16(), "sTypoAscender", os2.s_typo_ascender()),
        ("descender", "descender", hhea.descender().to_i16(), "sTypoDescender", os2.s_typo_descender()),
        ("line-gap", "lineGap", hhea.line_gap().to_i16(), "sTypoLineGap", os2.s_typo_line_gap()),
    ];
    for (code, hname, h, tname, t) in pairs {
        if h != t {
            let msg = format!("hhea.{hname} is {h} but OS/2.{tname} is {t}.");
            sink.push(Hit::new(code, msg).data("actual", f64::from(h)).data("expected", f64::from(t)).place(field(ctx, "hhea", hname)));
        }
    }
}

/// The lowest and highest glyph extents as FontBakery reads them: the head box for CFF fonts,
/// the glyph headers (with 0) for TrueType ones, with the glyph that reaches each.
fn y_extents(font: &FontRef<'_>) -> Option<((i16, Option<GlyphId>), (i16, Option<GlyphId>))> {
    let head = font.head().ok()?;
    let otto = font.table_directory.sfnt_version() == 0x4F54_544F;
    let glyf = font.glyf().ok();
    let loca = font.loca(Some(head.index_to_loc_format() == 1)).ok();
    let (Some(glyf), Some(loca), false) = (glyf, loca, otto) else {
        return Some(((head.y_min(), None), (head.y_max(), None)));
    };
    let count = font.maxp().ok()?.num_glyphs();
    let (mut lo, mut hi) = ((0i16, None), (0i16, None));
    for g in 0..u32::from(count) {
        let gid = GlyphId::new(g);
        let Ok(Some(glyph)) = loca.get_glyf(gid, &glyf) else { continue };
        if glyph.y_min() < lo.0 {
            lo = (glyph.y_min(), Some(gid));
        }
        if glyph.y_max() > hi.0 {
            hi = (glyph.y_max(), Some(gid));
        }
    }
    Some((lo, hi))
}

/// `check.metrics.winAscentDescent`.
pub(crate) fn win_ascent_descent(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Some(((y_min, low), (y_max, high)))) = (ctx.font.os2(), y_extents(&ctx.font)) else {
        sink.skip("the font has no readable 'OS/2' or 'head' table");
        return;
    };
    if is_cjk(&ctx.font) {
        sink.skip("the font is a CJK font");
        return;
    }
    let ascent = i32::from(os2.us_win_ascent());
    let descent = i32::from(os2.us_win_descent());
    let (top, bottom) = (i32::from(y_max), i32::from(y_min).abs());
    let with_glyph = |mut place: Location, gid: Option<GlyphId>| {
        if let Some(g) = gid {
            place.glyph = glyph_name(ctx, g);
        }
        place
    };
    let mut report = |code: &'static str, name: &str, actual: i32, expected: i32, msg: String, gid: Option<GlyphId>| {
        let hit = Hit::new(code, msg).data("actual", f64::from(actual)).data("expected", f64::from(expected));
        sink.push(hit.place(with_glyph(field(ctx, "OS/2", name), gid)));
    };
    if ascent < top {
        report("ascent", "usWinAscent", ascent, top, format!("OS/2.usWinAscent is {ascent} but the tallest glyph reaches {top}."), high);
    }
    if ascent > top * 2 {
        report("ascent", "usWinAscent", ascent, top, format!("OS/2.usWinAscent is {ascent}, more than twice the font's height {top}."), high);
    }
    if descent < bottom {
        report("descent", "usWinDescent", descent, bottom, format!("OS/2.usWinDescent is {descent} but the lowest glyph reaches {}.", -bottom), low);
    }
    if descent > bottom * 2 {
        report(
            "descent",
            "usWinDescent",
            descent,
            bottom,
            format!("OS/2.usWinDescent is {descent}, more than twice the font's depth {bottom}."),
            low,
        );
    }
}

/// `check.metrics.lineGaps`.
pub(crate) fn line_gaps(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Ok(hhea)) = (ctx.font.os2(), ctx.font.hhea()) else {
        sink.skip("the font has no readable 'OS/2' or 'hhea' table");
        return;
    };
    let gap = hhea.line_gap().to_i16();
    if gap != 0 {
        sink.push(
            Hit::new("hhea-line-gap", format!("hhea.lineGap is {gap}, not 0.")).data("actual", f64::from(gap)).place(field(ctx, "hhea", "lineGap")),
        );
    }
    let typo = os2.s_typo_line_gap();
    if typo != 0 {
        sink.push(Hit::new("typo-line-gap", format!("OS/2.sTypoLineGap is {typo}, not 0.")).data("actual", f64::from(typo)).place(field(
            ctx,
            "OS/2",
            "sTypoLineGap",
        )));
    }
}

/// `check.metrics.useTypoMetrics`.
pub(crate) fn use_typo_metrics(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(os2) = ctx.font.os2() else {
        sink.skip(no_table("OS/2"));
        return;
    };
    let set = os2.fs_selection().bits() & 0x80 != 0;
    let version = os2.version();
    let place = field(ctx, "OS/2", "fsSelection");
    if set && version < 4 {
        let msg = format!("OS/2.fsSelection bit 7 (USE_TYPO_METRICS) is set but the OS/2 table is version {version}; the bit needs version 4.");
        sink.push(Hit::new("set-below-version-4", msg).data("version", f64::from(version)).place(place));
    } else if !set && !is_cjk(&ctx.font) {
        sink.push(Hit::new("not-set", "OS/2.fsSelection bit 7 (USE_TYPO_METRICS) is not set.").place(place));
    }
}

/// `check.metrics.typoAscenderVsAgrave`.
pub(crate) fn typo_ascender_vs_agrave(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(os2) = ctx.font.os2() else {
        sink.skip(no_table("OS/2"));
        return;
    };
    let outlines = ctx.font.outline_glyphs();
    let Some((gid, Some(b))) = char_bounds(&ctx.font, &outlines, 0xC0) else {
        sink.skip("the font has no Agrave with an outline");
        return;
    };
    let typo = f64::from(os2.s_typo_ascender());
    if typo < b.y1 {
        let name = glyph_name(ctx, gid);
        let msg = format!("OS/2.sTypoAscender is {} but {name} reaches {}.", num(typo), num(b.y1));
        let mut place = field(ctx, "OS/2", "sTypoAscender");
        place.glyph = name;
        sink.push(Hit::new("below-agrave", msg).data("actual", typo).data("expected", b.y1).place(place));
    }
}

/// `check.metrics.capsVerticallyCentred`.
pub(crate) fn caps_vertically_centred(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(hhea), Ok(head)) = (ctx.font.hhea(), ctx.font.head()) else {
        sink.skip("the font has no readable 'hhea' or 'head' table");
        return;
    };
    let outlines = ctx.font.outline_glyphs();
    let mut tops = Vec::new();
    let mut bottoms = Vec::new();
    for c in "ABCDEHIMOSTX".chars() {
        let Some((_, Some(b))) = char_bounds(&ctx.font, &outlines, u32::from(c)) else {
            sink.skip("the font lacks one of the capitals A B C D E H I M O S T X, or its outline");
            return;
        };
        tops.push(b.y1);
        bottoms.push(b.y0);
    }
    let n = tops.len() as f64;
    let cap = tops.iter().sum::<f64>() / n;
    let low = bottoms.iter().sum::<f64>() / n;
    let margin = 1.2 * f64::from(head.units_per_em()) * 0.18;
    let top_space = f64::from(hhea.ascender().to_i16()) - cap;
    let bottom_space = f64::from(hhea.descender().to_i16()).abs() + low;
    let difference = (top_space - bottom_space).abs();
    if difference > margin {
        let msg = format!(
            "The capitals have {} units above them to hhea.ascender and {} below them to hhea.descender; the difference exceeds {}.",
            num(top_space),
            num(bottom_space),
            num(margin)
        );
        let hit = Hit::new("not-centred", msg).data("above", top_space).data("below", bottom_space).data("margin", margin);
        sink.push(hit.place(field(ctx, "hhea", "ascender")));
    }
}

/// The vertical metrics every scheme reads.
struct Vertical {
    upm: f64,
    typo: [i16; 3],
    hhea: [i16; 3],
    win: [u16; 2],
    use_typo: bool,
    cap_height: Option<i16>,
}

/// `check.metrics.verticalMetricsScheme`.
pub(crate) fn vertical_metrics_scheme(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Ok(hhea), Ok(head)) = (ctx.font.os2(), ctx.font.hhea(), ctx.font.head()) else {
        sink.skip("the font has no readable 'OS/2', 'hhea' or 'head' table");
        return;
    };
    let v = Vertical {
        upm: f64::from(head.units_per_em()),
        typo: [os2.s_typo_ascender(), os2.s_typo_descender(), os2.s_typo_line_gap()],
        hhea: [hhea.ascender().to_i16(), hhea.descender().to_i16(), hhea.line_gap().to_i16()],
        win: [os2.us_win_ascent(), os2.us_win_descent()],
        use_typo: os2.fs_selection().bits() & 0x80 != 0,
        cap_height: os2.s_cap_height(),
    };
    if v.upm <= 0.0 {
        sink.skip("head.unitsPerEm is 0");
        return;
    }
    let scheme = params.get(&SCHEME).round();
    if scheme == 1.0 {
        microsoft_scheme(ctx, &v, sink);
    } else if scheme == 2.0 {
        type_network_scheme(ctx, &v, sink);
    } else if is_cjk(&ctx.font) {
        google_cjk_scheme(ctx, &v, sink);
    } else {
        google_scheme(ctx, &v, sink);
    }
}

fn push_field(ctx: &BinaryCtx<'_>, sink: &mut Sink, code: &'static str, table: &str, name: &str, msg: String) {
    sink.push(Hit::new(code, msg).place(field(ctx, table, name)));
}

fn hhea_sum(v: &Vertical) -> f64 {
    let [asc, desc, gap] = v.hhea.map(f64::from);
    (asc + desc.abs() + gap) / v.upm
}

fn line_gaps_zero(ctx: &BinaryCtx<'_>, v: &Vertical, sink: &mut Sink) {
    if v.typo[2] != 0 {
        push_field(ctx, sink, "typo-line-gap", "OS/2", "sTypoLineGap", format!("OS/2.sTypoLineGap is {}; the scheme needs 0.", v.typo[2]));
    }
    if v.hhea[2] != 0 {
        push_field(ctx, sink, "hhea-line-gap", "hhea", "lineGap", format!("hhea.lineGap is {}; the scheme needs 0.", v.hhea[2]));
    }
}

fn google_scheme(ctx: &BinaryCtx<'_>, v: &Vertical, sink: &mut Sink) {
    line_gaps_zero(ctx, v, sink);
    let sum = hhea_sum(v);
    let total = num(sum * v.upm);
    if sum < 1.2 {
        push_field(
            ctx,
            sink,
            "hhea-range",
            "hhea",
            "ascender",
            format!("hhea ascender, |descender| and line gap add up to {total}; the scheme needs at least 120% of the em."),
        );
    } else if sum > 2.0 {
        push_field(
            ctx,
            sink,
            "hhea-range",
            "hhea",
            "ascender",
            format!("hhea ascender, |descender| and line gap add up to {total}; the scheme allows at most 200% of the em."),
        );
    } else if sum > 1.5 {
        push_field(
            ctx,
            sink,
            "hhea-range-high",
            "hhea",
            "ascender",
            format!("hhea ascender, |descender| and line gap add up to {total}; 120% to 150% of the em is recommended."),
        );
    }
    if v.typo[0] < 0 {
        push_field(ctx, sink, "typo-ascender", "OS/2", "sTypoAscender", format!("OS/2.sTypoAscender is {}; it must be positive.", v.typo[0]));
    }
    if v.hhea[0] <= 0 {
        push_field(ctx, sink, "hhea-ascender", "hhea", "ascender", format!("hhea.ascender is {}; it must be positive.", v.hhea[0]));
    }
    if v.win[0] == 0 {
        push_field(ctx, sink, "win-ascent", "OS/2", "usWinAscent", "OS/2.usWinAscent is 0; it must be positive.".into());
    }
    if v.typo[1] > 0 {
        push_field(ctx, sink, "typo-descender", "OS/2", "sTypoDescender", format!("OS/2.sTypoDescender is {}; it must be 0 or negative.", v.typo[1]));
    }
    if v.hhea[1] > 0 {
        push_field(ctx, sink, "hhea-descender", "hhea", "descender", format!("hhea.descender is {}; it must be 0 or negative.", v.hhea[1]));
    }
}

fn google_cjk_scheme(ctx: &BinaryCtx<'_>, v: &Vertical, sink: &mut Sink) {
    if v.use_typo {
        push_field(
            ctx,
            sink,
            "cjk-use-typo-metrics",
            "OS/2",
            "fsSelection",
            "The CJK scheme needs OS/2.fsSelection bit 7 (USE_TYPO_METRICS) clear.".into(),
        );
    }
    let asc = py_round(v.upm * 0.88);
    let desc = py_round(v.upm * -0.12);
    if f64::from(v.typo[0]) != asc {
        push_field(
            ctx,
            sink,
            "cjk-typo-ascender",
            "OS/2",
            "sTypoAscender",
            format!("OS/2.sTypoAscender is {}; the CJK scheme needs {}.", v.typo[0], num(asc)),
        );
    }
    if f64::from(v.typo[1]) != desc {
        push_field(
            ctx,
            sink,
            "cjk-typo-descender",
            "OS/2",
            "sTypoDescender",
            format!("OS/2.sTypoDescender is {}; the CJK scheme needs {}.", v.typo[1], num(desc)),
        );
    }
    line_gaps_zero(ctx, v, sink);
    hhea_equals_win(ctx, v, sink);
    let sum = hhea_sum(v);
    if !(sum > 1.1 && sum <= 1.5) {
        push_field(
            ctx,
            sink,
            "cjk-hhea-range",
            "hhea",
            "ascender",
            format!("hhea ascender, |descender| and line gap add up to {}; 110% to 150% of the em is recommended.", num(sum * v.upm)),
        );
    }
}

fn hhea_equals_win(ctx: &BinaryCtx<'_>, v: &Vertical, sink: &mut Sink) {
    if i32::from(v.hhea[0]) != i32::from(v.win[0]) {
        push_field(
            ctx,
            sink,
            "hhea-win-ascent",
            "hhea",
            "ascender",
            format!("hhea.ascender {} differs from OS/2.usWinAscent {}.", v.hhea[0], v.win[0]),
        );
    }
    if i32::from(v.hhea[1]).abs() != i32::from(v.win[1]) {
        push_field(
            ctx,
            sink,
            "hhea-win-descent",
            "hhea",
            "descender",
            format!("|hhea.descender| {} differs from OS/2.usWinDescent {}.", i32::from(v.hhea[1]).abs(), v.win[1]),
        );
    }
}

fn microsoft_scheme(ctx: &BinaryCtx<'_>, v: &Vertical, sink: &mut Sink) {
    if v.use_typo {
        let pairs = [
            ("hhea-typo-ascender", "ascender", "sTypoAscender", 0),
            ("hhea-typo-descender", "descender", "sTypoDescender", 1),
            ("hhea-typo-line-gap", "lineGap", "sTypoLineGap", 2),
        ];
        for (code, h, t, i) in pairs {
            let (Some(a), Some(b)) = (v.hhea.get(i), v.typo.get(i)) else { continue };
            if a != b {
                push_field(ctx, sink, code, "hhea", h, format!("USE_TYPO_METRICS is set, so hhea.{h} {a} must equal OS/2.{t} {b}."));
            }
        }
    } else {
        hhea_equals_win(ctx, v, sink);
    }
}

fn type_network_scheme(ctx: &BinaryCtx<'_>, v: &Vertical, sink: &mut Sink) {
    let hhea_desc = i32::from(v.hhea[1]).abs();
    if v.use_typo {
        if v.hhea[0] != v.typo[0] {
            push_field(
                ctx,
                sink,
                "hhea-typo-ascender",
                "hhea",
                "ascender",
                format!("hhea.ascender {} must equal OS/2.sTypoAscender {}.", v.hhea[0], v.typo[0]),
            );
        } else if hhea_desc != i32::from(v.typo[1]).abs() {
            push_field(
                ctx,
                sink,
                "hhea-typo-descender",
                "hhea",
                "descender",
                format!("hhea.descender {} must equal OS/2.sTypoDescender {}.", v.hhea[1], v.typo[1]),
            );
        } else if v.typo[2] != 0 {
            push_field(ctx, sink, "typo-line-gap", "OS/2", "sTypoLineGap", format!("OS/2.sTypoLineGap is {}; the scheme needs 0.", v.typo[2]));
        } else if v.hhea[2] != 0 {
            push_field(ctx, sink, "hhea-line-gap", "hhea", "lineGap", format!("hhea.lineGap is {}; the scheme needs 0.", v.hhea[2]));
        }
    } else {
        push_field(
            ctx,
            sink,
            "use-typo-metrics-recommended",
            "OS/2",
            "fsSelection",
            "OS/2.fsSelection bit 7 (USE_TYPO_METRICS) is clear; the scheme recommends setting it and making hhea equal the typo metrics.".into(),
        );
        let typo_sum = i32::from(v.typo[0]) + i32::from(v.typo[1]).abs() + i32::from(v.typo[2]);
        let hhea_total = i32::from(v.hhea[0]) + hhea_desc + i32::from(v.hhea[2]);
        if v.hhea[0] == v.typo[0] && hhea_desc == i32::from(v.win[1]) {
            push_field(
                ctx,
                sink,
                "use-typo-metrics-off",
                "OS/2",
                "fsSelection",
                "hhea follows the typo metrics but USE_TYPO_METRICS is clear.".into(),
            );
        } else if i32::from(v.hhea[0]) != i32::from(v.win[0]) {
            push_field(
                ctx,
                sink,
                "hhea-win-ascent",
                "hhea",
                "ascender",
                format!("hhea.ascender {} must equal OS/2.usWinAscent {}.", v.hhea[0], v.win[0]),
            );
        } else if hhea_desc != i32::from(v.win[1]) {
            push_field(
                ctx,
                sink,
                "hhea-win-descent",
                "hhea",
                "descender",
                format!("|hhea.descender| {hhea_desc} must equal OS/2.usWinDescent {}.", v.win[1]),
            );
        } else if typo_sum != hhea_total {
            push_field(
                ctx,
                sink,
                "typo-hhea-sum",
                "OS/2",
                "sTypoAscender",
                format!("The typo metrics add up to {typo_sum} but hhea adds up to {hhea_total}."),
            );
        }
    }
    if let Some(cap) = v.cap_height {
        let above = i32::from(v.hhea[0]) - i32::from(cap);
        let below = hhea_desc;
        let larger = above.max(below);
        if larger > 0 {
            let ratio = f64::from((above - below).abs()) / f64::from(larger);
            if ratio >= 0.3 {
                let msg = format!("Text sits off centre on the web: {above} units above the capitals, {below} below the baseline.");
                sink.push(Hit::new("uncentred", msg).data("ratio", ratio).place(field(ctx, "hhea", "ascender")));
            }
        }
    }
}

/// `check.metrics.strikeoutAndScripts` (comparefamily single-face test 27; the zero sizes after
/// Type Network's checks).
pub(crate) fn strikeout_and_scripts(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Ok(head), Ok(post)) = (ctx.font.os2(), ctx.font.head(), ctx.font.post()) else {
        sink.skip("the font has no readable 'OS/2', 'head' or 'post' table");
        return;
    };
    let underline = post.underline_thickness().to_i16();
    if os2.y_strikeout_size() == 0 {
        push_field(ctx, sink, "strikeout-size-zero", "OS/2", "yStrikeoutSize", "OS/2.yStrikeoutSize is 0, so no strikeout is drawn.".into());
    }
    if underline == 0 {
        push_field(
            ctx,
            sink,
            "underline-thickness-zero",
            "post",
            "underlineThickness",
            "post.underlineThickness is 0, so no underline is drawn.".into(),
        );
    }
    let upm = f64::from(head.units_per_em());
    let x_height = os2.sx_height().map(f64::from).filter(|h| *h > 0.0);
    let strike_pos = x_height.map_or_else(|| half_up(upm * 0.22), |h| half_up(h * 0.6));
    let script_x = half_up(upm * 0.65);
    let script_y = half_up(upm * 0.60);
    let sub_y_offset = half_up(upm * 0.075);
    let sup_y_offset = half_up(upm * 0.35);
    let angle = post.italic_angle().to_f64();
    let (sub_x_offset, sup_x_offset) = if angle == 0.0 {
        (0.0, 0.0)
    } else {
        let tan = (-angle).to_radians().tan();
        (-half_up(sub_y_offset * tan), half_up(sup_y_offset * tan))
    };
    let fields = [
        ("yStrikeoutSize", os2.y_strikeout_size(), f64::from(underline)),
        ("yStrikeoutPosition", os2.y_strikeout_position(), strike_pos),
        ("ySubscriptXSize", os2.y_subscript_x_size(), script_x),
        ("ySubscriptYSize", os2.y_subscript_y_size(), script_y),
        ("ySubscriptXOffset", os2.y_subscript_x_offset(), sub_x_offset),
        ("ySubscriptYOffset", os2.y_subscript_y_offset(), sub_y_offset),
        ("ySuperscriptXSize", os2.y_superscript_x_size(), script_x),
        ("ySuperscriptYSize", os2.y_superscript_y_size(), script_y),
        ("ySuperscriptXOffset", os2.y_superscript_x_offset(), sup_x_offset),
        ("ySuperscriptYOffset", os2.y_superscript_y_offset(), sup_y_offset),
    ];
    for (name, actual, expected) in fields {
        let actual = f64::from(actual);
        if expected.is_finite() && (actual - expected).abs() > 5.0 {
            let msg = format!("OS/2.{name} is {} but the default derived from the font's metrics is {}.", num(actual), num(expected));
            let hit = Hit::new("differs-from-default", msg).data("actual", actual).data("expected", expected);
            sink.push(hit.place(field(ctx, "OS/2", name)));
        }
    }
}

#[cfg(test)]
mod tests;
