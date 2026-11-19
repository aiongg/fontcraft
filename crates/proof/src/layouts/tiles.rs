//! The tile layouts `boxes`, `hints` and `digi`: each glyph in a tile (two side by side for
//! `digi`), drawn with the elements its [`Display`] turns on, in the reference script's order: em
//! box, glyph, horizontal and vertical stem hints, alignment zones, baseline, centre marks,
//! advance box, advance ticks, contour labels, point marks and labels, then a block of metrics
//! text under the glyph.
//!
//! Inside a tile the glyph square is 1000 tile units wide (one em) and as tall as the font's
//! ascender-to-descender band or the em box, whichever reaches further. Sizes and paddings of the
//! settings are in tile units, so they scale with the tile.

use skrifa::GlyphId;
use skrifa::raw::TableProvider;

use super::Display;
use super::hints::{FontHints, GlyphHints};
use crate::font::{El, Face};
use crate::page::{self, Anchor, Document, GlyphStyle, Ink, Item};
use crate::text::num;
use crate::{MAX_PAGES, MAX_SHEET_GLYPHS, ProofError, ProofOptions};

/// The width of the glyph square, in tile units.
const SQUARE: f64 = 1000.0;
/// Length of centre marks and advance ticks, in thousandths of an em.
const TICK: f64 = 10.0;
/// How far alignment zones reach past the em box, in thousandths of an em.
const ZONE_OFFSET: f64 = 5.0;
/// A tile labels the points of a glyph only up to this many.
const MAX_TILE_LABELS: usize = 400;
/// Widest a hint label is expected to be, in label sizes; it sets the room left of the square.
const HINT_LABEL_EMS: f64 = 9.0;

/// How big the tiles are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fit {
    /// The glyph square is this many points wide.
    Size(f64),
    /// As large as lets this many glyphs fit one page.
    PerPage(usize),
}

/// The space one copy of a glyph takes, in tile units.
struct Frame {
    h_pad: f64,
    v_pad: f64,
    /// Room above and below the glyph square for labels.
    band: f64,
    meta: f64,
}

impl Frame {
    fn of(d: &Display) -> Frame {
        let h_pad = d.h_padding + if d.hhints { d.point_label_size * HINT_LABEL_EMS } else { 0.0 };
        let band = if d.vhints || d.point_labels || d.contour_labels || d.blue_zones { d.point_label_size * 2.6 } else { 0.0 };
        let lines = [
            (d.meta_name, d.meta_name_size),
            (d.meta_bbox, d.meta_text_size),
            (d.meta_side_bearings, d.meta_text_size),
            (d.meta_v_side_bearings, d.meta_text_size),
            (d.meta_parts, d.meta_text_size),
            (d.meta_paths, d.meta_text_size),
            (d.meta_hints, d.meta_text_size),
        ];
        let text: f64 = lines.iter().filter(|(on, _)| *on).map(|(_, size)| size * 1.2).sum();
        let glyph = if d.meta_outline { d.meta_glyph_size } else { 0.0 };
        let meta = text.max(glyph);
        Frame { h_pad, v_pad: d.v_padding, band, meta }
    }

    fn width(&self) -> f64 {
        SQUARE + 2.0 * self.h_pad
    }

    fn height(&self, square_h: f64) -> f64 {
        2.0 * self.v_pad + 2.0 * self.band + square_h + self.meta
    }
}

/// Lays out the glyphs of `doc` in tiles, one copy per entry of `copies`.
pub(crate) fn sheet(face: &Face<'_>, doc: &mut Document, options: &ProofOptions, copies: &[Display], fit: Fit) -> Result<(), ProofError> {
    let hints =
        if copies.iter().any(|d| d.hhints || d.vhints || d.blue_zones || d.meta_hints) { super::hints::read(face) } else { FontHints::default() };
    let upem = face.upem;
    let base = match options.baseline {
        Some(b) if b.is_finite() && b.abs() <= upem * 4.0 => b,
        Some(b) => return Err(ProofError::BadOptions(format!("baseline {b} must be within four ems of 0"))),
        None if copies.len() > 1 || hints.cid => -0.12 * upem,
        None => 0.0,
    };
    // The vertical band the glyph square shows, in font units.
    let (lo, hi) = (base.min(face.descender), (base + upem).max(face.ascender));
    let square_h = (hi - lo) * SQUARE / upem;
    let frames: Vec<Frame> = copies.iter().map(Frame::of).collect();
    let unit_w: f64 = frames.iter().map(Frame::width).sum();
    let unit_h = frames.iter().map(|f| f.height(square_h)).fold(0.0, f64::max);
    let [x0, y0, w, h] = page::content(options.page_size);
    let shape = |s: f64| {
        if !(s.is_finite() && s > 0.0) {
            return (0usize, 0usize);
        }
        ((w / (unit_w * s)).floor().clamp(0.0, 1e6) as usize, (h / (unit_h * s)).floor().clamp(0.0, 1e6) as usize)
    };
    let (s, cols, per_page) = match fit {
        Fit::Size(size) => {
            let s = size / SQUARE;
            let (c, r) = shape(s);
            if c.saturating_mul(r) == 0 {
                return Err(ProofError::BadOptions(format!("a glyph of {size} points does not fit on the page in this layout")));
            }
            (s, c, c.saturating_mul(r))
        }
        Fit::PerPage(n) => {
            let fits = |s: f64| {
                let (c, r) = shape(s);
                c.saturating_mul(r) >= n
            };
            let (mut a, mut b) = (page::MIN_SIZE / SQUARE, (w / unit_w).min(h / unit_h));
            if n == 0 || n > MAX_SHEET_GLYPHS || !fits(a) {
                return Err(ProofError::BadOptions(format!("{n} glyphs do not fit on one page in this layout")));
            }
            for _ in 0..60 {
                let mid = (a + b) / 2.0;
                if fits(mid) {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            (a, shape(a).0.max(1), n)
        }
    };
    let count = doc.glyphs.len();
    if doc.pages.len().saturating_add(count.div_ceil(per_page)) > MAX_PAGES {
        return Err(ProofError::BadOptions(format!("the proof would have more than {MAX_PAGES} pages: select fewer glyphs or a smaller size")));
    }
    let geometry = Geometry { s, lo, hi, base };
    for start in (0..count).step_by(per_page) {
        let mut items = Vec::new();
        for (slot, index) in (start..count.min(start + per_page)).enumerate() {
            let (col, row) = (slot % cols, slot / cols);
            let mut ox = x0 + col as f64 * unit_w * s;
            let oy = y0 + row as f64 * unit_h * s;
            for (copy, (d, frame)) in copies.iter().zip(&frames).enumerate() {
                let tile = Tile { d, frame, ox, oy, s, primary: copy == 0 };
                tile.draw(face, doc, index, &hints, &options.hint_groups, &geometry, &mut items);
                ox += frame.width() * s;
            }
        }
        page::new_page(doc, options.page_size)?.items = items;
    }
    Ok(())
}

/// What every tile of a sheet shares.
struct Geometry {
    /// Points per tile unit.
    s: f64,
    /// The bottom and top of the glyph square, in font units.
    lo: f64,
    hi: f64,
    /// The bottom of the em box, in font units.
    base: f64,
}

struct Tile<'a> {
    d: &'a Display,
    frame: &'a Frame,
    /// Top left of the tile, in points.
    ox: f64,
    oy: f64,
    /// Points per tile unit.
    s: f64,
    primary: bool,
}

impl Tile<'_> {
    #[allow(clippy::too_many_arguments)]
    fn draw(&self, face: &Face<'_>, doc: &Document, index: usize, hints: &FontHints, groups: &[usize], g: &Geometry, items: &mut Vec<Item>) {
        let Some(glyph) = doc.glyphs.get(index) else { return };
        let (d, s) = (self.d, g.s);
        let upem = face.upem;
        let f = s * SQUARE / upem;
        let sx = self.ox + self.frame.h_pad * s;
        let sy = self.oy + (self.frame.v_pad + self.frame.band) * s;
        let at = |fx: f64, fy: f64| [sx + fx * f, sy + (g.hi - fy) * f];
        let ls = d.point_label_size * s;
        let em = upem / 1000.0;
        let adv = glyph.outline.advance;
        let base = g.base;
        let top = base + upem;
        let empty = GlyphHints::default();
        let gh = hints.glyph(glyph.gid).unwrap_or(&empty);
        let bounds = glyph.outline.bounds;
        let (v_origin, v_adv) = vertical(face, glyph.gid, bounds.map_or(0.0, |[_, _, _, y_max]| y_max), base);
        let label = |x: f64, y: f64, size: f64, text: String, anchor: Anchor, colour: [u8; 3], items: &mut Vec<Item>| {
            items.push(Item::Label { x, y, size, text, anchor, ink: Ink::Rgb(colour) });
        };
        let line =
            |a: [f64; 2], b: [f64; 2], colour: [u8; 3], items: &mut Vec<Item>| items.push(Item::Line { from: a, to: b, ink: Ink::Rgb(colour) });
        let rect = |fx0: f64, fy0: f64, fx1: f64, fy1: f64, colour: [u8; 3], items: &mut Vec<Item>| {
            let [ax, ay] = at(fx0.min(fx1), fy0.max(fy1));
            items.push(Item::Rect { x: ax, y: ay, w: (fx1 - fx0).abs() * f, h: (fy1 - fy0).abs() * f, ink: Ink::Rgb(colour) });
        };

        if d.em_box {
            rect(0.0, base, upem, top, d.embox_colour, items);
        }
        if d.outline {
            let [x, y] = at(0.0, 0.0);
            let style = if d.filled { GlyphStyle::Filled } else { GlyphStyle::Outlined };
            items.push(Item::Glyph { glyph: index, x, y, scale: f, style, primary: self.primary });
        }
        let shown = |i: usize| groups.is_empty() || gh.groups.is_empty() || groups.iter().any(|k| gh.groups.get(*k).is_some_and(|g| g.contains(&i)));
        let in_groups = |i: usize| {
            if gh.groups.len() < 2 {
                return None;
            }
            let list: Vec<String> = gh.groups.iter().enumerate().filter(|(_, g)| g.contains(&i)).map(|(k, _)| k.to_string()).collect();
            (!list.is_empty()).then(|| format!("in groups {}", list.join(" ")))
        };
        for horizontal in [true, false] {
            if (horizontal && !d.hhints) || (!horizontal && !d.vhints) {
                continue;
            }
            let mut offset = (if horizontal { d.hhint_offset } else { d.vhint_offset }) * em;
            let mut previous: Option<f64> = None;
            for (k, (i, stem)) in gh.stems.iter().enumerate().filter(|(_, st)| st.horizontal == horizontal).enumerate() {
                if !shown(i) {
                    continue;
                }
                let (lo_e, hi_e) = (stem.pos, stem.pos + stem.width);
                let overlap = previous.is_some_and(|p| p >= lo_e);
                let colour = if overlap {
                    offset += 2.0 * em;
                    d.hint_overlap_colour
                } else {
                    d.hint_colour
                };
                let text_colour = colour.map(|c| (f64::from(c) * 0.6).round() as u8);
                let text = format!("{}({}, {})  {}", if horizontal { 'h' } else { 'v' }, num(lo_e), num(hi_e), num(stem.width));
                let groups_text = in_groups(i);
                if horizontal {
                    let (x0, x1) = (-offset, upem + offset);
                    rect(x0, lo_e, x1, hi_e, colour, items);
                    let y = at(0.0, lo_e)[1] - 0.2 * ls;
                    let (x, anchor) = if k % 2 == 1 { (at(x1, 0.0)[0] + 0.2 * ls, Anchor::Start) } else { (at(x0, 0.0)[0] - 0.2 * ls, Anchor::End) };
                    label(x, y, ls, text, anchor, text_colour, items);
                    if let Some(t) = groups_text {
                        label(x, y - 1.2 * ls, ls, t, anchor, text_colour, items);
                    }
                } else {
                    let (y0, y1) = (base - offset, top + offset);
                    rect(lo_e, y0, hi_e, y1, colour, items);
                    let x = at(lo_e.min(hi_e), 0.0)[0];
                    let (y, step) = if k % 2 == 1 { (at(0.0, y1)[1] - 0.2 * ls, -1.2 * ls) } else { (at(0.0, y0)[1] + 1.2 * ls, 1.2 * ls) };
                    label(x, y, ls, text, Anchor::Start, text_colour, items);
                    if let Some(t) = groups_text {
                        label(x, y + step, ls, t, Anchor::Start, text_colour, items);
                    }
                }
                previous = Some(hi_e);
            }
        }
        if d.blue_zones {
            let (x0, x1) = (-ZONE_OFFSET * em, upem + ZONE_OFFSET * em);
            let text_colour = d.zone_colour.map(|c| (f64::from(c) * 0.5).round() as u8);
            for (i, &(z0, z1)) in hints.zones.iter().enumerate() {
                rect(x0, z0, x1, z1, d.zone_colour, items);
                let text = format!("<{}, {}> {}", num(z0), num(z1), num(z1 - z0));
                let y = at(0.0, z1)[1] - 0.2 * ls;
                let (x, anchor) = if i % 2 == 0 { (at(x1, 0.0)[0], Anchor::End) } else { (at(x0, 0.0)[0], Anchor::Start) };
                label(x, y, ls, text, anchor, text_colour, items);
            }
        }
        // As the reference script, at the `baseline` value: the em box's bottom edge.
        if d.baseline {
            line(at(0.0, base), at(upem, base), d.baseline_colour, items);
        }
        let (y_start, height) = if d.meta_v_side_bearings { (v_origin - v_adv, v_adv) } else { (base, upem) };
        let tick = TICK * em;
        if d.center_marks || d.center_marks_with_box {
            let mid = base + upem / 2.0;
            let x2 = adv / 2.0;
            line(at(-tick, mid), at(0.0, mid), d.x_advance_colour, items);
            line(at(adv, mid), at(adv + tick, mid), d.x_advance_colour, items);
            line(at(x2, y_start + height), at(x2, y_start + height + tick), d.x_advance_colour, items);
            line(at(x2, y_start - tick), at(x2, y_start), d.x_advance_colour, items);
        }
        if d.glyph_box || d.center_marks_with_box {
            rect(0.0, y_start, adv, y_start + height, d.embox_colour, items);
        }
        if d.x_advance {
            for x in [0.0, adv] {
                line(at(x, -tick / 2.0), at(x, tick / 2.0), d.x_advance_colour, items);
            }
            line(at(0.0, 0.0), at(tick * 2.0 / 3.0, 0.0), d.x_advance_colour, items);
            line(at(adv - tick * 2.0 / 3.0, 0.0), at(adv, 0.0), d.x_advance_colour, items);
        }
        if d.contour_labels {
            let mut contour = 0usize;
            for el in &glyph.outline.els {
                if let El::Move(x, y) = *el {
                    let [px, py] = at(f64::from(x), f64::from(y));
                    label(px + 0.3 * ls, py - 0.3 * ls, ls, format!("P {contour}"), Anchor::Start, d.contour_label_colour, items);
                    contour += 1;
                }
            }
        }
        if d.point_marks || d.bcp_marks || d.point_labels {
            self.points(&glyph.outline.els, &at, ls, items);
        }
        let meta_top = sy + (g.hi - g.lo) * f + self.frame.band * s;
        self.meta(face, index, glyph, gh, hints.cid, [sx, meta_top], (v_origin, v_adv), items);
    }

    /// Point marks, off-curve marks with their handles, and point labels.
    fn points(&self, els: &[El], at: &impl Fn(f64, f64) -> [f64; 2], ls: f64, items: &mut Vec<Item>) {
        let d = self.d;
        let ink = Ink::Rgb(d.point_label_colour);
        let labels = d.point_labels && els.len() <= MAX_TILE_LABELS;
        let mut index = 0usize;
        let mut current: Option<[f64; 2]> = None;
        let mut on = |kind: &str, x: f32, y: f32, items: &mut Vec<Item>| {
            let (fx, fy) = (f64::from(x), f64::from(y));
            let [px, py] = at(fx, fy);
            if d.point_marks {
                items.push(Item::Point { x: px, y: py, on_curve: true, ink });
            }
            if labels {
                let mut text = String::new();
                if d.label_type {
                    text.push_str(kind);
                }
                if d.label_index {
                    text.push_str(&format!(" {index}"));
                }
                if d.label_position {
                    text.push_str(&format!(" ({}, {})", num(fx), num(fy)));
                }
                let text = text.trim().to_string();
                if !text.is_empty() {
                    items.push(Item::Label { x: px + 0.4 * ls, y: py - 0.4 * ls, size: ls, text, anchor: Anchor::Start, ink });
                }
            }
            index += 1;
            [fx, fy]
        };
        let off = |from: [f64; 2], c: (f32, f32), items: &mut Vec<Item>| {
            if d.bcp_marks {
                let [px, py] = at(f64::from(c.0), f64::from(c.1));
                items.push(Item::Line { from: at(from[0], from[1]), to: [px, py], ink });
                items.push(Item::Point { x: px, y: py, on_curve: false, ink });
            }
        };
        for el in els {
            match *el {
                El::Move(x, y) => current = Some(on("m", x, y, items)),
                El::Line(x, y) => current = Some(on("l", x, y, items)),
                El::Quad(cx, cy, x, y) => {
                    if let Some(c) = current {
                        off(c, (cx, cy), items);
                    }
                    off([f64::from(x), f64::from(y)], (cx, cy), items);
                    current = Some(on("q", x, y, items));
                }
                El::Cubic(ax, ay, bx, by, x, y) => {
                    if let Some(c) = current {
                        off(c, (ax, ay), items);
                    }
                    off([f64::from(x), f64::from(y)], (bx, by), items);
                    current = Some(on("c", x, y, items));
                }
                El::Close => {}
            }
        }
    }

    /// The metrics text under the glyph square, and the small filled copy of the glyph.
    #[allow(clippy::too_many_arguments)]
    fn meta(
        &self,
        face: &Face<'_>,
        index: usize,
        glyph: &page::ShownGlyph,
        gh: &GlyphHints,
        cid: bool,
        [x, top]: [f64; 2],
        (v_origin, v_adv): (f64, f64),
        items: &mut Vec<Item>,
    ) {
        let d = self.d;
        let s = self.s;
        let adv = glyph.outline.advance;
        let [bx0, by0, bx1, by1] = glyph.outline.bounds.unwrap_or([0.0, 0.0, 0.0, 0.0]);
        let mut lines: Vec<(f64, String)> = Vec::new();
        if d.meta_name {
            let name = match glyph.name.strip_prefix("cid") {
                Some(rest) if cid && rest.chars().all(|c| c.is_ascii_digit()) => format!("CID {rest}"),
                _ => glyph.name.clone(),
            };
            lines.push((d.meta_name_size, name));
        }
        if d.meta_bbox {
            lines.push((d.meta_text_size, format!("min = {}, {}  max = {}, {}", num(bx0), num(by0), num(bx1), num(by1))));
        }
        if d.meta_side_bearings {
            let (lsb, rsb) = if glyph.outline.bounds.is_some() { (bx0, adv - bx1) } else { (0.0, adv) };
            lines.push((d.meta_text_size, format!("L = {}, R = {} Width = {}", num(lsb), num(rsb), num(adv))));
        }
        if d.meta_v_side_bearings {
            let (tsb, bsb) = if glyph.outline.bounds.is_some() { (v_origin - by1, by0 - (v_origin - v_adv)) } else { (0.0, v_adv) };
            lines.push((d.meta_text_size, format!("T = {}, B = {}, vAdv = {}", num(tsb), num(bsb), num(v_adv))));
        }
        let (mut moves, mut straight, mut curves) = (0usize, 0usize, 0usize);
        for el in &glyph.outline.els {
            match el {
                El::Move(..) => moves += 1,
                El::Line(..) => straight += 1,
                El::Quad(..) | El::Cubic(..) => curves += 1,
                El::Close => {}
            }
        }
        if d.meta_parts {
            let total = moves + straight + curves;
            lines.push((d.meta_text_size, format!("Parts: m = {moves}, l = {straight}, c = {curves}, total = {total}")));
        }
        if d.meta_paths {
            lines.push((d.meta_text_size, format!("Paths:  {moves}")));
        }
        if d.meta_hints {
            let h = gh.stems.iter().filter(|st| st.horizontal).count();
            let v = gh.stems.len() - h;
            lines.push((d.meta_text_size, format!("Hints:  {} horiz: {h}, vert: {v}", h + v)));
        }
        let mut y = top;
        for (size, text) in lines {
            y += size * 1.2 * s;
            items.push(Item::Label { x, y: y - 0.2 * size * s, size: size * s, text, anchor: Anchor::Start, ink: Ink::Rgb(d.meta_colour) });
        }
        if d.meta_outline && face.upem > 0.0 {
            let k = d.meta_glyph_size * s / face.upem;
            let gx = x + (SQUARE - d.meta_glyph_size) * s;
            items.push(Item::Glyph { glyph: index, x: gx, y: top + face.ascender * k, scale: k, style: GlyphStyle::Filled, primary: false });
        }
    }
}

/// The vertical origin and advance of glyph `gid`: from `VORG` and `vmtx` when the font has them,
/// else the top of the em box and one em, as the reference.
fn vertical(face: &Face<'_>, gid: u32, y_max: f64, base: f64) -> (f64, f64) {
    let id = GlyphId::new(gid);
    let vmtx = face.font.vmtx().ok();
    let advance = vmtx.as_ref().and_then(|t| t.advance(id)).map(f64::from).unwrap_or(face.upem);
    let origin = match (face.font.vorg(), vmtx.as_ref().and_then(|t| t.side_bearing(id))) {
        (Ok(vorg), _) => f64::from(vorg.vertical_origin_y(id)),
        (Err(_), Some(tsb)) => y_max + f64::from(tsb),
        (Err(_), None) => face.upem + base,
    };
    (origin, advance)
}
