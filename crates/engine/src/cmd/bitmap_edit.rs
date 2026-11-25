//! Editing the bitmaps of strikes (M13.63): `bitmap.setPixels` changes one glyph's pixels, rows,
//! origin offset, references and, in a font without outlines, advance; `bitmap.transform` flips,
//! turns by quarter turns and skews bitmaps; `bitmap.regenerateGlyphs` draws chosen glyphs again
//! from the outlines; `bitmap.removeGlyphs` deletes bitmaps; `bitmap.setStrikeProperties` edits a
//! strike's name, size, resolution, line metrics and file properties.
//!
//! Each command builds the new bitmaps from the font as it is, then puts them in with one edit, so
//! a refusal changes nothing. The model's strike check, which refuses references that loop or nest
//! too deep, runs on the result.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};

use super::bitmap_strikes::{MAX_CALL_RASTER_PIXELS, RasterBudget, StrikeSpec, rasterize_glyphs, strike_spec, strike_specs};
use super::{CommandSpec, cmd, has_doc};
use crate::params::{Params, TargetKind};
use crate::{Result, Session};
use fontcraft_model::Font;
use fontcraft_model::strike::{self, BitmapReference, BitmapStrike, StrikeGlyph, StrikeProperty, StrikeValue};

/// Most pixels one `bitmap.setPixels` call lists.
const MAX_SET_PIXELS: usize = 1 << 20;
/// Largest skew step either way, in pixels.
const MAX_SKEW: i64 = 1_024;
/// Highest resolution a strike may state, in dots per inch.
const MAX_RESOLUTION: i64 = 100_000;
/// Longest name a message quotes, in characters.
const QUOTED: usize = 64;

/// A box `(left, bottom, right, top)` in pixels up from the baseline.
type PixelBox = (i64, i64, i64, i64);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "bitmap.setPixels",
            "Set Pixels",
            [],
            None,
            "{glyph, strike: {ppem: number, depth?: number (needed when the font has the size at several depths)}, rows?: [string] (the whole bitmap, top row first; one character a pixel: `.` off, `#` the darkest level, or a hex digit for the level; at 8 bits two characters a pixel: `..`, `##` or two hex digits), origin?: [left, bottom] (pixels from the glyph origin to the bitmap's bottom-left corner), pixels?: [{x, y, value}] (the pixel whose bottom-left corner is x pixels right of and y pixels above the glyph origin; value 0 clears, up to 1 at 1 bit and 3, 15 or 255 for grey; the bitmap grows to take a pixel that is set outside it), references?: [{glyph, x?: number, y?: number}] (replaces the glyph's references to other bitmaps of the strike, each drawn with its origin x pixels right and y up from this glyph's), advance?: number} changes one glyph's bitmap as one undo step, making it when the strike has none. Rows apply first, then the origin, then the pixels. In a font with outlines the advance is the glyph's width scaled to the strike and rounded, and `advance` is refused; in a font without outlines `advance` sets it, and a new bitmap starts from the scaled width. Refused, changing nothing: no change given, a glyph the font lacks, a strike the font lacks, a bitmap wider or taller than 1,024 pixels, rows of different lengths, a level above the strike's, more than 1,048,576 pixels, more than 64 references, a reference to the glyph itself or to a bitmap the strike lacks, references that lead back to the glyph or nest more than 8 deep → {glyph, strike, created, width, height, left, bottom, advance, references}",
            has_doc,
            set_pixels
        ),
        cmd!(
            "bitmap.transform",
            "Transform Bitmap",
            [],
            None,
            "{glyphs?: [name] (default: every bitmap of the strike), strike: {ppem: number, depth?: number (default: every depth of the size)}, op: \"flipH\" | \"flipV\" | \"rotate90cw\" | \"rotate90ccw\" | \"rotate180\" | \"skew\", skew?: [dx, dy] (op skew only: each row moves dx pixels right for every dy rows it lies above the baseline, rounded down; dx from -1024 to 1024, dy from 1 to 1024)} transforms bitmaps as one undo step. Flips mirror a bitmap within its box; quarter turns keep the box's centre, rounded down; skewing keeps the row on the baseline in place and widens the box. A glyph built from references has them drawn in first. Advances stay. Glyphs without a bitmap in the strike are skipped. Refused, changing nothing: a strike the font lacks, a glyph the font lacks, none of the glyphs with a bitmap, a skewed bitmap wider than 1,024 pixels → {strikes: [label], glyphs, skipped: [name]}",
            has_doc,
            transform
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "bitmap.regenerateGlyphs",
            "Regenerate Bitmap Glyphs…",
            ["Glyph"],
            None,
            "{glyphs?: [name] (default: every exported glyph), strikes?: [{ppem: number, depth?: number}] (default: all; without depth, every depth of the size)} draws the glyphs' bitmaps again from the first master's outlines, or from the placed PDF of a glyph without outlines, as bitmap.setStrikes fills a new strike, as one undo step. The new bitmaps replace hand edits and references; each keeps the code of the bitmap it replaces, and the strikes keep their metrics. Glyphs that are not exported are skipped. Refused, changing nothing: a font without strikes, a strike the font lacks, a glyph the font lacks, none of the glyphs exported, a bitmap wider or taller than 1,024 pixels, more than 67,108,864 pixels to draw for one strike or 268,435,456 for all, bitmaps over 64 MiB for one strike or 256 MiB for all the font's strikes → {strikes: [label], glyphs, skipped: [name], warnings?: [string]}",
            has_doc,
            regenerate_glyphs
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "bitmap.removeGlyphs",
            "Remove Bitmap Glyphs…",
            ["Glyph"],
            None,
            "{glyphs: [name], strikes?: [{ppem: number, depth?: number}] (default: all; without depth, every depth of the size)} deletes the glyphs' bitmaps from the strikes as one undo step; outlines and other strikes stay. References to a removed bitmap from the other glyphs of its strike are removed too, and those glyphs are listed. Refused, changing nothing: no glyphs, a glyph the font lacks, a font without strikes, a strike the font lacks, none of the glyphs with a bitmap in the strikes → {strikes: [label], removed, unlinked: [name]}",
            has_doc,
            remove_glyphs
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "bitmap.setStrikeProperties",
            "Set Strike Properties",
            [],
            None,
            "{strike: {ppem: number, depth?: number (needed when the font has the size at several depths)}, name?: string, pointSize?: number, resolution?: [x, y] (dots per inch; 0 for unknown), ascent?: number, descent?: number, properties?: [{name, value: string | whole number}] (replaces the strike's file properties, in order)} edits one strike's name, size in points, resolution, line metrics and the properties a BDF file carries, as one undo step. Refused, changing nothing: no change given, a strike the font lacks, an empty or repeated property name, more than 4,096 properties, a name or text over 65,536 bytes, a point size beyond 0..32767, a resolution beyond 0..100000, an ascent or descent beyond ±32767 → {strike, name, pointSize, resolution, ascent, descent, properties}",
            has_doc,
            set_strike_properties
        ),
    ]
}

// ── Shared ────────────────────────────────────────────────────────────────────────────────────

/// At most [`QUOTED`] characters of a name from the parameters, for a message.
fn quote(name: &str) -> String {
    name.chars().take(QUOTED).collect()
}

/// A whole number between `min` and `max`.
fn whole(v: &Value, min: i64, max: i64) -> Option<i64> {
    let n = v.as_f64().filter(|n| n.is_finite() && n.fract() == 0.0)?;
    (n >= min as f64 && n <= max as f64).then_some(n as i64)
}

/// A pair `[a, b]` of whole numbers between `min` and `max`.
fn pair(p: &Params<'_>, key: &str, min: i64, max: i64) -> Result<Option<(i64, i64)>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let bad = || p.bad(format!("`{key}` must be two whole numbers from {min} to {max}"));
    match v.as_array().map(Vec::as_slice) {
        Some([a, b]) => Ok(Some((whole(a, min, max).ok_or_else(bad)?, whole(b, min, max).ok_or_else(bad)?))),
        _ => Err(bad()),
    }
}

/// The `{ppem, depth?}` object under `key`, which is required.
fn strike_param(p: &Params<'_>, key: &str) -> Result<StrikeSpec> {
    match p.get(key) {
        Some(item) if item.is_object() => strike_spec(p, key, item),
        _ => Err(p.bad(format!("`{key}` is required: an object {{ppem, depth?}}"))),
    }
}

/// The one strike `spec` names: refused when the font lacks it, or has the size at several depths
/// and `spec` gives none.
fn one_strike(p: &Params<'_>, font: &Font, spec: StrikeSpec) -> Result<usize> {
    let mut found = font.bitmap_strikes.iter().enumerate().filter(|(_, s)| spec.matches(s)).map(|(i, _)| i);
    let Some(first) = found.next() else { return Err(p.bad(format!("the font has no strike {}", spec.label()))) };
    if found.next().is_some() {
        return Err(p.bad(format!("the font has strikes of {} pixels per em at several depths: give `depth`", spec.ppem)));
    }
    Ok(first)
}

/// The strikes `specs` name, each once, in font order; every strike when `None`.
fn chosen_strikes(p: &Params<'_>, font: &Font, specs: Option<Vec<StrikeSpec>>) -> Result<Vec<usize>> {
    if font.bitmap_strikes.is_empty() {
        return Err(p.bad("the font has no bitmap strikes"));
    }
    let Some(specs) = specs else { return Ok((0..font.bitmap_strikes.len()).collect()) };
    for spec in &specs {
        if !font.bitmap_strikes.iter().any(|s| spec.matches(s)) {
            return Err(p.bad(format!("the font has no strike {}", spec.label())));
        }
    }
    Ok(font.bitmap_strikes.iter().enumerate().filter(|(_, s)| specs.iter().any(|spec| spec.matches(s))).map(|(i, _)| i).collect())
}

fn labels(font: &Font, indices: &[usize]) -> Vec<String> {
    indices.iter().filter_map(|i| font.bitmap_strikes.get(*i)).map(BitmapStrike::label).collect()
}

/// Whether any glyph has a path or component on any layer: a bitmap's advance then follows its glyph's width.
pub fn has_outlines(font: &Font) -> bool {
    font.glyphs.iter().any(|g| g.layers.iter().any(|l| !l.shapes.is_empty()))
}

/// `glyph`'s width on the first master, scaled to `ppem` pixels per em and rounded.
fn scaled_advance(font: &Font, glyph: &str, ppem: u16) -> Result<i32> {
    let master = font.default_master()?;
    let width = font.glyph(glyph).and_then(|g| g.layer(&master.id)).map_or(0.0, |l| l.width);
    let v = (width * f64::from(ppem) / f64::from(font.units_per_em.max(1))).round();
    let limit = f64::from(strike::MAX_STRIKE_COORDINATE);
    Ok(if v.is_finite() { v.clamp(-limit, limit) as i32 } else { 0 })
}

fn union(a: Option<PixelBox>, (l, b, r, t): PixelBox) -> PixelBox {
    match a {
        Some((al, ab, ar, at)) => (al.min(l), ab.min(b), ar.max(r), at.max(t)),
        None => (l, b, r, t),
    }
}

/// Refuses a box wider or taller than a bitmap can be.
fn fits(p: &Params<'_>, what: &str, (l, b, r, t): PixelBox) -> Result<()> {
    let side = i64::from(strike::MAX_STRIKE_BITMAP_SIDE);
    if r - l > side || t - b > side {
        return Err(p.bad(format!("{what} would be {} × {} pixels; a bitmap is at most {side} on a side", r - l, t - b)));
    }
    Ok(())
}

/// `g` with its box moved to `to`, which must fit a bitmap; pixels outside `to` are lost.
fn rebox(g: &StrikeGlyph, depth: u8, (left, bottom, right, top): PixelBox) -> StrikeGlyph {
    let mut out = StrikeGlyph {
        width: u16::try_from(right - left).unwrap_or(0),
        height: u16::try_from(top - bottom).unwrap_or(0),
        left: i32::try_from(left).unwrap_or(0),
        bottom: i32::try_from(bottom).unwrap_or(0),
        data: Vec::new(),
        ..g.clone()
    };
    out.data = vec![0; out.stride(depth) * usize::from(out.height)];
    let old_top = i64::from(g.bottom) + i64::from(g.height);
    for row in 0..g.height {
        for col in 0..g.width {
            let level = g.pixel(depth, col, row);
            if level == 0 {
                continue;
            }
            let x = i64::from(g.left) + i64::from(col) - left;
            let y = top - old_top + i64::from(row);
            if let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) {
                out.set_pixel(depth, x, y, level);
            }
        }
    }
    out
}

/// Puts each strike's new bitmaps in place of the glyphs' old ones; a glyph the strike has no
/// bitmap for gets it at the end, in the order given.
fn put_bitmaps(font: &mut Font, changes: Vec<(usize, Vec<StrikeGlyph>)>) {
    let strikes = Arc::make_mut(&mut font.bitmap_strikes);
    for (i, list) in changes {
        let Some(strike) = strikes.get_mut(i) else { continue };
        let order: Vec<String> = list.iter().map(|g| g.glyph.clone()).collect();
        let mut by_name: HashMap<String, StrikeGlyph> = list.into_iter().map(|g| (g.glyph.clone(), g)).collect();
        for g in strike.glyphs.iter_mut() {
            if let Some(new) = by_name.remove(&g.glyph) {
                *g = new;
            }
        }
        for name in order {
            if let Some(g) = by_name.remove(&name) {
                strike.glyphs.push(g);
            }
        }
    }
}

// ── bitmap.setPixels ──────────────────────────────────────────────────────────────────────────

fn set_pixels(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.setPixels", v)?;
    let font = s.font()?;
    let name = p.str("glyph")?;
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("there is no glyph `{}`", quote(name))));
    }
    let index = one_strike(&p, font, strike_param(&p, "strike")?)?;
    let Some(target) = font.bitmap_strikes.get(index) else { return Err(p.bad("the strike is gone")) };
    let depth = target.depth;
    let c = i64::from(strike::MAX_STRIKE_COORDINATE);
    let rows = p.opt_array("rows")?;
    let origin = pair(&p, "origin", -c, c)?;
    let pixels = p.opt_array("pixels")?;
    let references = p.opt_array("references")?;
    let advance = p.opt_int("advance", -c, c)?;
    if rows.is_none() && origin.is_none() && pixels.is_none() && references.is_none() && advance.is_none() {
        return Err(p.bad("give `rows`, `origin`, `pixels`, `references` or `advance`"));
    }
    let outlines = has_outlines(font);
    if advance.is_some() && outlines {
        return Err(p.bad("the font has outlines, so a bitmap's advance follows its glyph's width; change the width instead"));
    }
    let created = target.glyph(name).is_none();
    let mut bitmap = target.glyph(name).cloned().unwrap_or_else(|| StrikeGlyph::empty(name, 0));
    if let Some(rows) = rows {
        let (width, height, data) = parse_rows(&p, rows, depth)?;
        bitmap.width = width;
        bitmap.height = height;
        bitmap.data = data;
    }
    if let Some((left, bottom)) = origin {
        bitmap.left = i32::try_from(left).unwrap_or(0);
        bitmap.bottom = i32::try_from(bottom).unwrap_or(0);
    }
    if let Some(pixels) = pixels {
        set_listed_pixels(&p, &mut bitmap, depth, pixels)?;
    }
    if let Some(items) = references {
        bitmap.references = parse_references(&p, target, name, items)?;
    }
    match advance {
        Some(a) => bitmap.advance = i32::try_from(a).unwrap_or(0),
        None if outlines || created => bitmap.advance = scaled_advance(font, name, target.ppem)?,
        None => {}
    }
    let out = json!({
        "glyph": name,
        "strike": target.label(),
        "created": created,
        "width": bitmap.width,
        "height": bitmap.height,
        "left": bitmap.left,
        "bottom": bitmap.bottom,
        "advance": bitmap.advance,
        "references": bitmap.references.len(),
    });
    s.edit("Set Pixels", |font| {
        put_bitmaps(font, vec![(index, vec![bitmap])]);
        Ok(())
    })?;
    Ok(out)
}

/// `rows` as a bitmap's width, height and packed pixels at `depth` bits.
fn parse_rows(p: &Params<'_>, rows: &[Value], depth: u8) -> Result<(u16, u16, Vec<u8>)> {
    let side = usize::from(strike::MAX_STRIKE_BITMAP_SIDE);
    if rows.len() > side {
        return Err(p.bad(format!("`rows` holds more than {side} rows")));
    }
    // Characters a pixel: two hex digits at 8 bits, one below.
    let per = if depth == 8 { 2 } else { 1 };
    let top = strike::max_level(depth);
    let mut texts = Vec::with_capacity(rows.len());
    let mut width = None;
    for (i, row) in rows.iter().enumerate() {
        let Some(text) = row.as_str() else { return Err(p.bad(format!("`rows`: row {i} is not a string"))) };
        if !text.is_ascii() || text.len() > side * per || text.len() % per != 0 {
            return Err(p.bad(format!("`rows`: row {i} must hold at most {side} pixels of {per} character(s) each, from `.`, `#` and hex digits")));
        }
        let w = text.len() / per;
        match width {
            None => width = Some(w),
            Some(first) if first != w => return Err(p.bad(format!("`rows`: row {i} is {w} pixels long and row 0 is {first}"))),
            Some(_) => {}
        }
        texts.push(text.as_bytes());
    }
    let width = width.unwrap_or(0);
    if width == 0 {
        return Ok((0, 0, Vec::new()));
    }
    // Both within 1..=1024 by the tests above.
    let mut g = StrikeGlyph::empty("", 0);
    g.width = u16::try_from(width).unwrap_or(0);
    g.height = u16::try_from(texts.len()).unwrap_or(0);
    g.data = vec![0; g.stride(depth) * usize::from(g.height)];
    for (y, text) in texts.iter().enumerate() {
        for x in 0..width {
            let token = text.get(x * per..x * per + per).unwrap_or(&[]);
            let level = match token {
                [b'.'] | [b'.', b'.'] => Some(0),
                [b'#'] | [b'#', b'#'] => Some(top),
                [a] => hex(*a),
                [a, b] => hex(*a).zip(hex(*b)).map(|(h, l)| h * 16 + l),
                _ => None,
            };
            let Some(level) = level.filter(|l| *l <= top) else {
                return Err(p.bad(format!("`rows`: row {y}, pixel {x} is not `.`, `#` or a level from 0 to {top}")));
            };
            if let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) {
                g.set_pixel(depth, x, y, level);
            }
        }
    }
    Ok((g.width, g.height, g.data))
}

fn hex(c: u8) -> Option<u8> {
    char::from(c).to_digit(16).and_then(|d| u8::try_from(d).ok())
}

/// Sets the `pixels` items of `bitmap`, growing its box to take each pixel set outside it.
fn set_listed_pixels(p: &Params<'_>, bitmap: &mut StrikeGlyph, depth: u8, items: &[Value]) -> Result<()> {
    if items.len() > MAX_SET_PIXELS {
        return Err(p.bad(format!("`pixels` lists more than {MAX_SET_PIXELS} pixels")));
    }
    let top = strike::max_level(depth);
    let c = i64::from(strike::MAX_STRIKE_COORDINATE);
    let mut listed = Vec::with_capacity(items.len());
    let mut grown = bitmap.pixel_box();
    for (i, item) in items.iter().enumerate() {
        let field = |key: &str, min: i64, max: i64| item.get(key).and_then(|v| whole(v, min, max));
        let (Some(x), Some(y), Some(value)) = (field("x", -c, c), field("y", -c, c), field("value", 0, i64::from(top))) else {
            return Err(p.bad(format!("`pixels`: item {i} needs whole numbers `x` and `y` within ±{c} and `value` from 0 to {top}")));
        };
        let value = u8::try_from(value).unwrap_or(0);
        if value > 0 {
            grown = Some(union(grown, (x, y, x + 1, y + 1)));
        }
        listed.push((x, y, value));
    }
    if let Some(b) = grown
        && grown != bitmap.pixel_box()
    {
        fits(p, &format!("the bitmap of `{}`", bitmap.glyph), b)?;
        *bitmap = rebox(bitmap, depth, b);
    }
    // Only clearing, on a bitmap without pixels: nothing to do.
    let Some((left, _, _, top_edge)) = bitmap.pixel_box() else { return Ok(()) };
    for (x, y, value) in listed {
        if let (Ok(col), Ok(row)) = (u16::try_from(x - left), u16::try_from(top_edge - 1 - y)) {
            bitmap.set_pixel(depth, col, row, value);
        }
    }
    Ok(())
}

/// The `references` items for `name`'s bitmap in `strike`.
fn parse_references(p: &Params<'_>, strike: &BitmapStrike, name: &str, items: &[Value]) -> Result<Vec<BitmapReference>> {
    if items.len() > strike::MAX_BITMAP_REFERENCES {
        return Err(p.bad(format!("`references` lists more than {} references", strike::MAX_BITMAP_REFERENCES)));
    }
    let c = i64::from(strike::MAX_STRIKE_COORDINATE);
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(glyph) = item.get("glyph").and_then(Value::as_str) else {
            return Err(p.bad(format!("`references`: item {i} needs `glyph`, the bitmap to draw")));
        };
        if glyph == name {
            return Err(p.bad(format!("`references`: the bitmap of `{}` cannot refer to itself", quote(name))));
        }
        if strike.glyph(glyph).is_none() {
            return Err(p.bad(format!("`references`: strike {} has no bitmap for `{}`", strike.label(), quote(glyph))));
        }
        let offset = |key: &str| match item.get(key) {
            None | Some(Value::Null) => Some(0),
            Some(v) => whole(v, -c, c),
        };
        let (Some(x), Some(y)) = (offset("x"), offset("y")) else {
            return Err(p.bad(format!("`references`: item {i}: `x` and `y` must be whole numbers within ±{c}")));
        };
        out.push(BitmapReference { glyph: glyph.to_string(), x: i32::try_from(x).unwrap_or(0), y: i32::try_from(y).unwrap_or(0) });
    }
    Ok(out)
}

// ── bitmap.transform ──────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    FlipH,
    FlipV,
    Rotate90Cw,
    Rotate90Ccw,
    Rotate180,
    /// Rows move `.0` pixels right for every `.1` rows above the baseline.
    Skew(i64, i64),
}

fn transform(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.transform", v)?;
    let font = s.font()?;
    let spec = strike_param(&p, "strike")?;
    let indices: Vec<usize> = font.bitmap_strikes.iter().enumerate().filter(|(_, s)| spec.matches(s)).map(|(i, _)| i).collect();
    if indices.is_empty() {
        return Err(p.bad(format!("the font has no strike {}", spec.label())));
    }
    let op = match p.str("op")? {
        "flipH" => Op::FlipH,
        "flipV" => Op::FlipV,
        "rotate90cw" => Op::Rotate90Cw,
        "rotate90ccw" => Op::Rotate90Ccw,
        "rotate180" => Op::Rotate180,
        "skew" => {
            let Some((dx, dy)) = pair(&p, "skew", -MAX_SKEW, MAX_SKEW)? else { return Err(p.bad("op skew needs `skew`: [dx, dy]")) };
            if dy < 1 {
                return Err(p.bad(format!("`skew`: dy, the rows per step, must be from 1 to {MAX_SKEW}")));
            }
            Op::Skew(dx, dy)
        }
        other => {
            return Err(p.bad(format!("`op` must be flipH, flipV, rotate90cw, rotate90ccw, rotate180 or skew, got `{}`", quote(other))));
        }
    };
    if !matches!(op, Op::Skew(..)) && p.get("skew").is_some() {
        return Err(p.bad("`skew` goes with op skew"));
    }
    let only = p.opt_glyphs("glyphs", font)?;
    let wanted: Option<HashSet<&str>> = only.as_ref().map(|l| l.iter().map(String::as_str).collect());
    let mut changes = Vec::with_capacity(indices.len());
    let mut done: HashSet<String> = HashSet::new();
    let mut count = 0usize;
    for &i in &indices {
        let Some(strike) = font.bitmap_strikes.get(i) else { continue };
        let flat = strike.flattened().map_err(|e| p.bad(format!("strike {}: {e}", strike.label())))?;
        let source = flat.as_ref().unwrap_or(strike);
        let mut list = Vec::new();
        for g in source.glyphs.iter().filter(|g| wanted.as_ref().is_none_or(|w| w.contains(g.glyph.as_str()))) {
            let new = transformed(&p, &strike.label(), g, source.depth, op)?;
            if wanted.is_some() {
                done.insert(g.glyph.clone());
            }
            list.push(new);
        }
        count += list.len();
        changes.push((i, list));
    }
    if count == 0 {
        return Err(p.bad(format!("none of the glyphs has a bitmap in strike {}", spec.label())));
    }
    let skipped: Vec<&String> = only.iter().flatten().filter(|n| !done.contains(n.as_str())).collect();
    let out = json!({ "strikes": labels(font, &indices), "glyphs": count, "skipped": skipped });
    s.edit("Transform Bitmap", |font| {
        put_bitmaps(font, changes);
        Ok(())
    })?;
    Ok(out)
}

/// `g`, a bitmap without references in strike `label`, flipped, turned or skewed by `op` at
/// `depth` bits per pixel.
fn transformed(p: &Params<'_>, label: &str, g: &StrikeGlyph, depth: u8, op: Op) -> Result<StrikeGlyph> {
    let (w, h) = (i64::from(g.width), i64::from(g.height));
    let (left, bottom) = (i64::from(g.left), i64::from(g.bottom));
    // The new box, and where an old pixel (column, row from the top) goes.
    let shift = |row: i64| match op {
        Op::Skew(dx, dy) => (bottom + h - 1 - row).saturating_mul(dx).checked_div_euclid(dy).unwrap_or(0),
        _ => 0,
    };
    let (shift_min, shift_max) = (0..h).map(&shift).fold((i64::MAX, i64::MIN), |(lo, hi), s| (lo.min(s), hi.max(s)));
    let (shift_min, shift_max) = if h == 0 { (0, 0) } else { (shift_min, shift_max) };
    let (new_w, new_h, new_left, new_bottom) = match op {
        Op::FlipH | Op::FlipV | Op::Rotate180 => (w, h, left, bottom),
        Op::Rotate90Cw | Op::Rotate90Ccw => (h, w, left + (w - h).div_euclid(2), bottom + (h - w).div_euclid(2)),
        Op::Skew(..) => (w + shift_max - shift_min, h, left + shift_min, bottom),
    };
    let to = |col: i64, row: i64| match op {
        Op::FlipH => (w - 1 - col, row),
        Op::FlipV => (col, h - 1 - row),
        Op::Rotate180 => (w - 1 - col, h - 1 - row),
        Op::Rotate90Cw => (h - 1 - row, col),
        Op::Rotate90Ccw => (row, w - 1 - col),
        Op::Skew(..) => (col + shift(row) - shift_min, row),
    };
    if w == 0 || h == 0 {
        return Ok(g.clone());
    }
    fits(p, &format!("strike {label}: the bitmap of `{}`", g.glyph), (new_left, new_bottom, new_left + new_w, new_bottom + new_h))?;
    let mut out = StrikeGlyph {
        width: u16::try_from(new_w).unwrap_or(0),
        height: u16::try_from(new_h).unwrap_or(0),
        left: i32::try_from(new_left).unwrap_or(g.left),
        bottom: i32::try_from(new_bottom).unwrap_or(g.bottom),
        data: Vec::new(),
        ..g.clone()
    };
    out.data = vec![0; out.stride(depth) * usize::from(out.height)];
    for row in 0..g.height {
        for col in 0..g.width {
            let level = g.pixel(depth, col, row);
            if level == 0 {
                continue;
            }
            let (x, y) = to(i64::from(col), i64::from(row));
            if let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) {
                out.set_pixel(depth, x, y, level);
            }
        }
    }
    Ok(out)
}

// ── bitmap.regenerateGlyphs ───────────────────────────────────────────────────────────────────

fn regenerate_glyphs(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.regenerateGlyphs", v)?;
    let assets = s.assets();
    let font = s.font()?;
    let indices = chosen_strikes(&p, font, strike_specs(&p, "strikes")?)?;
    let only = p.opt_glyphs("glyphs", font)?;
    let wanted: Option<HashSet<String>> = only.as_ref().map(|l| l.iter().cloned().collect());
    let redrawn = |name: &str| font.glyph(name).is_some_and(|g| g.export) && wanted.as_ref().is_none_or(|w| w.contains(name));
    // The bytes the font's strikes keep beside the bitmaps drawn again.
    let mut kept = 0usize;
    for (i, strike) in font.bitmap_strikes.iter().enumerate() {
        let chosen = indices.contains(&i);
        for g in strike.glyphs.iter().filter(|g| !(chosen && redrawn(&g.glyph))) {
            kept = kept.saturating_add(g.data.len());
        }
    }
    let mut budget = RasterBudget { pixels: MAX_CALL_RASTER_PIXELS, bytes: strike::MAX_ALL_STRIKES_BYTES.saturating_sub(kept) };
    let mut warnings = Vec::new();
    // Every strike is measured before any is drawn, so a refusal comes before the work.
    for &i in &indices {
        let Some(strike) = font.bitmap_strikes.get(i) else { continue };
        rasterize_glyphs(font, assets.as_ref(), &p, (strike.ppem, strike.depth), wanted.as_ref(), &mut budget, false, &mut warnings)?;
    }
    let mut changes = Vec::with_capacity(indices.len());
    let mut count = 0usize;
    for &i in &indices {
        let Some(strike) = font.bitmap_strikes.get(i) else { continue };
        let drawn = rasterize_glyphs(
            font,
            assets.as_ref(),
            &p,
            (strike.ppem, strike.depth),
            wanted.as_ref(),
            &mut RasterBudget::unlimited(),
            true,
            &mut warnings,
        )?;
        let codes: HashMap<&str, u32> = strike.glyphs.iter().filter_map(|g| g.code.map(|c| (g.glyph.as_str(), c))).collect();
        let mut list = drawn.glyphs;
        for g in list.iter_mut() {
            g.code = codes.get(g.glyph.as_str()).copied();
        }
        count += list.len();
        changes.push((i, list));
    }
    if count == 0 {
        return Err(p.bad("none of the glyphs is exported, so there is nothing to draw"));
    }
    let skipped: Vec<&String> = only.iter().flatten().filter(|n| !font.glyph(n).is_some_and(|g| g.export)).collect();
    let mut out = json!({ "strikes": labels(font, &indices), "glyphs": count, "skipped": skipped });
    if !warnings.is_empty()
        && let Some(o) = out.as_object_mut()
    {
        o.insert("warnings".into(), json!(warnings));
    }
    s.edit("Regenerate Bitmap Glyphs", |font| {
        put_bitmaps(font, changes);
        Ok(())
    })?;
    Ok(out)
}

// ── bitmap.removeGlyphs ───────────────────────────────────────────────────────────────────────

fn remove_glyphs(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.removeGlyphs", v)?;
    let font = s.font()?;
    let Some(names) = p.opt_glyphs("glyphs", font)? else { return Err(p.bad("give `glyphs`, the glyphs whose bitmaps to remove")) };
    let indices = chosen_strikes(&p, font, strike_specs(&p, "strikes")?)?;
    let gone: HashSet<String> = names.into_iter().collect();
    let removed: usize = indices
        .iter()
        .filter_map(|i| font.bitmap_strikes.get(*i))
        .map(|strike| strike.glyphs.iter().filter(|g| gone.contains(&g.glyph)).count())
        .sum();
    if removed == 0 {
        return Err(p.bad("none of the glyphs has a bitmap in the chosen strikes"));
    }
    let strikes = labels(font, &indices);
    let unlinked = s.edit("Remove Bitmap Glyphs", |font| {
        let all = Arc::make_mut(&mut font.bitmap_strikes);
        let mut unlinked = Vec::new();
        let mut listed = HashSet::new();
        for i in indices {
            let Some(strike) = all.get_mut(i) else { continue };
            strike.glyphs.retain(|g| !gone.contains(&g.glyph));
            for g in strike.glyphs.iter_mut() {
                let before = g.references.len();
                g.references.retain(|r| !gone.contains(&r.glyph));
                if g.references.len() != before && listed.insert(g.glyph.clone()) {
                    unlinked.push(g.glyph.clone());
                }
            }
        }
        Ok(unlinked)
    })?;
    Ok(json!({ "strikes": strikes, "removed": removed, "unlinked": unlinked }))
}

// ── bitmap.setStrikeProperties ────────────────────────────────────────────────────────────────

fn set_strike_properties(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("bitmap.setStrikeProperties", v)?;
    let font = s.font()?;
    let index = one_strike(&p, font, strike_param(&p, "strike")?)?;
    let name = p.opt_str("name")?;
    if name.is_some_and(|n| n.len() > strike::MAX_STRIKE_STRING) {
        return Err(p.bad(format!("`name` is longer than {} bytes", strike::MAX_STRIKE_STRING)));
    }
    let point_size = p.opt_f64("pointSize")?;
    let most = f64::from(strike::MAX_STRIKE_COORDINATE);
    if point_size.is_some_and(|v| !(0.0..=most).contains(&v)) {
        return Err(p.bad(format!("`pointSize` must be from 0 to {most}")));
    }
    let resolution = pair(&p, "resolution", 0, MAX_RESOLUTION)?;
    let c = i64::from(strike::MAX_STRIKE_COORDINATE);
    let ascent = p.opt_int("ascent", -c, c)?;
    let descent = p.opt_int("descent", -c, c)?;
    let properties = match p.opt_array("properties")? {
        Some(items) => Some(parse_properties(&p, items)?),
        None => None,
    };
    if name.is_none() && point_size.is_none() && resolution.is_none() && ascent.is_none() && descent.is_none() && properties.is_none() {
        return Err(p.bad("give `name`, `pointSize`, `resolution`, `ascent`, `descent` or `properties`"));
    }
    s.edit("Strike Properties", |font| {
        let strikes = Arc::make_mut(&mut font.bitmap_strikes);
        let Some(strike) = strikes.get_mut(index) else { return Err(p.bad("the strike is gone")) };
        if let Some(name) = name {
            strike.name = name.to_string();
        }
        if let Some(size) = point_size {
            strike.point_size = size;
        }
        if let Some((x, y)) = resolution {
            strike.resolution = [u32::try_from(x).unwrap_or(0), u32::try_from(y).unwrap_or(0)];
        }
        if let Some(a) = ascent {
            strike.ascent = i32::try_from(a).unwrap_or(0);
        }
        if let Some(d) = descent {
            strike.descent = i32::try_from(d).unwrap_or(0);
        }
        if let Some(list) = properties {
            strike.properties = list;
        }
        Ok(json!({
            "strike": strike.label(),
            "name": strike.name,
            "pointSize": strike.point_size,
            "resolution": strike.resolution,
            "ascent": strike.ascent,
            "descent": strike.descent,
            "properties": strike.properties.len(),
        }))
    })
}

/// The `properties` items: names unique and not empty, values text or whole numbers.
fn parse_properties(p: &Params<'_>, items: &[Value]) -> Result<Vec<StrikeProperty>> {
    let most = strike::MAX_STRIKE_STRING;
    if items.len() > strike::MAX_STRIKE_PROPERTIES {
        return Err(p.bad(format!("`properties` lists more than {} properties", strike::MAX_STRIKE_PROPERTIES)));
    }
    let mut seen = HashSet::with_capacity(items.len());
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(name) = item.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty() && n.len() <= most) else {
            return Err(p.bad(format!("`properties`: item {i} needs a `name` of 1 to {most} bytes")));
        };
        if !seen.insert(name) {
            return Err(p.bad(format!("`properties`: `{}` is given twice", quote(name))));
        }
        let value = match item.get("value") {
            Some(Value::String(t)) if t.len() <= most => StrikeValue::Text(t.clone()),
            Some(Value::Number(n)) if n.as_i64().is_some() => StrikeValue::Integer(n.as_i64().unwrap_or(0)),
            _ => return Err(p.bad(format!("`properties`: `{}` needs a `value`: text of at most {most} bytes or a whole number", quote(name)))),
        };
        out.push(StrikeProperty { name: name.to_string(), value });
    }
    Ok(out)
}
