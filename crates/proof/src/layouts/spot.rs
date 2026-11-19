//! The sheets of the reference table dumper (rows `proof`, `font.proof`): the glyph synopsis, the
//! `cmap` sheet, the vertical origins of `VORG`, and a text set as outlines; and the pieces the
//! feature sheet ([`super::features`]) shares with them: glyphs added on demand ([`Glyphs`]),
//! glyphs set in lines across pages ([`Flow`]) and one glyph with its marks ([`draw_glyph`]).
//!
//! What each sheet shows, its glyphs per page and its default glyph size (24 points) follow the
//! reference's proofs as run on our own fonts; positions inside a cell are our own. Every sheet
//! reads a [`Display`], so `show` and `hide` switch its em box, bounding box, baseline, advance
//! ticks, names and numbers.

use std::collections::{HashMap, HashSet};

use skrifa::raw::TableProvider;

use super::Display;
use super::tiles::Fit;
use crate::font::Face;
use crate::page::{self, Anchor, Document, GlyphStyle, Ink, Item, ShownGlyph};
use crate::{MAX_SHEET_GLYPHS, ProofError, ProofOptions};

/// Glyphs per page of the synopsis, as the reference (16 columns of 20 rows on a letter page).
pub(crate) const SYNOPSIS_PER_PAGE: usize = 320;
/// Glyph size of the line-set sheets and `vorg` when none is given, in points, as the reference.
pub(crate) const DEFAULT_SIZE: f64 = 24.0;
/// Glyph size of the text sheet when none is given, in points.
pub(crate) const TEXT_SIZE: f64 = 48.0;
/// Most glyphs one text sheet sets.
const MAX_TEXT_GLYPHS: usize = 100_000;
/// Smallest synopsis cell, in points.
const MIN_CELL: f64 = 6.0;
/// The share of a synopsis cell the glyph's em takes.
const SYNOPSIS_GLYPH: f64 = 0.6;
/// Space after each glyph of a line-set sheet, in ems of the glyph size (the reference's 10
/// points at 24).
const GAP: f64 = 10.0 / 24.0;
/// Label size of the line-set sheets and `vorg`, in ems of the glyph size.
const LABEL: f64 = 0.21;
/// The two rows a line-set sheet staggers its labels on, below the baseline, in ems.
const LABEL_ROW_1: f64 = 0.62;
const LABEL_ROW_2: f64 = 0.83;

/// The glyphs of a sheet that shows glyphs beyond the selection (feature results, text): each
/// glyph goes into [`Document::glyphs`] once, its outline counted against the proof's budget.
pub(crate) struct Glyphs {
    index: HashMap<u32, usize>,
    budget: usize,
}

impl Glyphs {
    pub(crate) fn new(doc: &Document) -> Self {
        let mut index = HashMap::new();
        let mut used = 0usize;
        for (i, g) in doc.glyphs.iter().enumerate() {
            index.entry(g.gid).or_insert(i);
            used = used.saturating_add(g.outline.els.len());
        }
        Glyphs { index, budget: crate::font::MAX_TOTAL_ELEMENTS.saturating_sub(used) }
    }

    /// The index in [`Document::glyphs`] of glyph `gid`, drawn now when it is not there yet.
    pub(crate) fn get(&mut self, face: &Face<'_>, doc: &mut Document, gid: u32) -> Result<usize, ProofError> {
        if let Some(i) = self.index.get(&gid) {
            return Ok(*i);
        }
        if doc.glyphs.len() >= MAX_SHEET_GLYPHS {
            return Err(ProofError::BadOptions(format!("a proof shows at most {MAX_SHEET_GLYPHS} different glyphs")));
        }
        let outline = face.outline(gid, &mut self.budget)?;
        let i = doc.glyphs.len();
        doc.glyphs.push(ShownGlyph { font: 0, gid, name: face.name(gid).to_string(), unicode: face.unicode(gid), outline });
        self.index.insert(gid, i);
        Ok(i)
    }
}

/// Glyph `index` of `doc` with its origin at `origin` and `scale` points per font unit, with
/// the marks `d` turns on: em box, bounding box, baseline and advance ticks.
pub(crate) fn draw_glyph(face: &Face<'_>, doc: &Document, d: &Display, index: usize, origin: [f64; 2], scale: f64, items: &mut Vec<Item>) {
    let Some(g) = doc.glyphs.get(index) else { return };
    let [x, y] = origin;
    let at = |fx: f64, fy: f64| [x + fx * scale, y - fy * scale];
    let adv = g.outline.advance;
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64, colour: [u8; 3], items: &mut Vec<Item>| {
        let [ax, ay] = at(x0.min(x1), y0.max(y1));
        items.push(Item::Rect { x: ax, y: ay, w: (x1 - x0).abs() * scale, h: (y1 - y0).abs() * scale, ink: Ink::Rgb(colour) });
    };
    let line = |a: [f64; 2], b: [f64; 2], colour: [u8; 3], items: &mut Vec<Item>| items.push(Item::Line { from: a, to: b, ink: Ink::Rgb(colour) });
    if d.em_box {
        rect(0.0, face.descender, face.upem, face.descender + face.upem, d.embox_colour, items);
    }
    if let (true, Some([x0, y0, x1, y1])) = (d.glyph_bbox, g.outline.bounds) {
        rect(x0, y0, x1, y1, d.embox_colour, items);
    }
    if d.baseline {
        line(at(0.0, 0.0), at(adv, 0.0), d.baseline_colour, items);
    }
    if d.outline {
        let style = if d.filled { GlyphStyle::Filled } else { GlyphStyle::Outlined };
        items.push(Item::Glyph { glyph: index, x, y, scale, style, primary: true });
    }
    if d.x_advance {
        let tick = face.upem * 0.04;
        for fx in [0.0, adv] {
            line(at(fx, -tick / 2.0), at(fx, tick / 2.0), d.x_advance_colour, items);
        }
        line(at(-tick, 0.0), at(0.0, 0.0), d.x_advance_colour, items);
        line(at(adv, 0.0), at(adv + tick, 0.0), d.x_advance_colour, items);
    }
}

/// The label under a glyph: its name, `@` and glyph id, and a code point, as far as the
/// elements `name` and `numeric-labels` allow; `None` when both are off.
pub(crate) fn glyph_label(d: &Display, g: &ShownGlyph, code: Option<u32>) -> Option<String> {
    let mut s = String::new();
    if d.meta_name {
        s.push_str(&g.name);
    }
    if d.numeric_labels {
        s.push_str(&format!("@{}", g.gid));
        if let Some(cp) = code {
            s.push_str(&format!(" {cp:04X}"));
        }
    }
    (!s.is_empty()).then_some(s)
}

/// Glyphs set in lines across pages, with room under each line for staggered labels (`cmap`,
/// features) or without it (text). Pages are added to the document as they fill.
pub(crate) struct Flow {
    page: [f64; 2],
    area: [f64; 4],
    /// Points per font unit.
    pub scale: f64,
    /// The em of every glyph, in points.
    pub size: f64,
    ascent: f64,
    line: f64,
    x: f64,
    top: f64,
    /// Glyphs placed on the current line, which picks the row of the next label.
    placed: usize,
    pub items: Vec<Item>,
}

impl Flow {
    pub(crate) fn new(face: &Face<'_>, page: [f64; 2], size: f64, labels: bool) -> Self {
        let area = page::content(page);
        let scale = size / face.upem;
        let ascent = face.ascender * scale;
        let below = if labels { size * (LABEL_ROW_2 + 0.45) } else { -face.descender * scale };
        let line = (ascent + below) * if labels { 1.0 } else { 1.15 };
        Flow { page, area, scale, size, ascent, line, x: area[0], top: area[1], placed: 0, items: Vec::new() }
    }

    fn bottom(&self) -> f64 {
        self.area[1] + self.area[3]
    }

    fn right(&self) -> f64 {
        self.area[0] + self.area[2]
    }

    /// Ends the current line.
    pub(crate) fn newline(&mut self) {
        if self.x > self.area[0] {
            self.skip_line();
        }
    }

    /// Moves down one line whether or not the current one holds anything.
    pub(crate) fn skip_line(&mut self) {
        self.top += self.line;
        self.x = self.area[0];
        self.placed = 0;
    }

    /// Puts what is drawn so far on a page and starts the next.
    pub(crate) fn page_break(&mut self, doc: &mut Document) -> Result<(), ProofError> {
        if !self.items.is_empty() {
            page::new_page(doc, self.page)?.items = std::mem::take(&mut self.items);
        }
        self.top = self.area[1];
        self.x = self.area[0];
        self.placed = 0;
        Ok(())
    }

    fn fit_height(&mut self, doc: &mut Document, height: f64) -> Result<(), ProofError> {
        if self.top + height > self.bottom() && self.top > self.area[1] {
            self.page_break(doc)?;
        }
        Ok(())
    }

    /// A line of text of `size` points across the page.
    pub(crate) fn heading(&mut self, doc: &mut Document, text: String, size: f64, ink: Ink) -> Result<(), ProofError> {
        self.newline();
        let height = size * 1.6;
        self.fit_height(doc, height + self.line)?;
        self.items.push(Item::Label { x: self.area[0], y: self.top + size, size, text, anchor: Anchor::Start, ink });
        self.top += height;
        Ok(())
    }

    /// Moves the pen by `dx` points along the line (kerning).
    pub(crate) fn shift(&mut self, dx: f64) {
        if dx.is_finite() {
            self.x += dx;
        }
    }

    /// Room for `width` points on the line, on a new line or page when the current one is
    /// full; returns the origin (x and baseline) where it starts.
    pub(crate) fn place(&mut self, doc: &mut Document, width: f64) -> Result<[f64; 2], ProofError> {
        if self.x > self.area[0] && self.x + width > self.right() {
            self.skip_line();
        }
        self.fit_height(doc, self.line)?;
        let origin = [self.x, self.top + self.ascent];
        self.x += width.max(0.0);
        Ok(origin)
    }

    /// The width a glyph of advance `advance` takes on the line, with the gap after it.
    pub(crate) fn cell(&self, advance: f64) -> f64 {
        advance.max(0.0) * self.scale + self.size * GAP
    }

    /// The label of a glyph placed at `origin`, on the next of the two label rows.
    pub(crate) fn label(&mut self, origin: [f64; 2], text: String, colour: [u8; 3]) {
        let row = if self.placed % 2 == 0 { LABEL_ROW_1 } else { LABEL_ROW_2 };
        self.placed += 1;
        let size = self.size * LABEL;
        self.items.push(Item::Label { x: origin[0], y: origin[1] + self.size * row, size, text, anchor: Anchor::Start, ink: Ink::Rgb(colour) });
    }

    /// A value under the glyph at `origin`, below both label rows.
    pub(crate) fn value(&mut self, origin: [f64; 2], text: String, colour: [u8; 3]) {
        let size = self.size * LABEL;
        let y = origin[1] + self.size * (LABEL_ROW_2 + 0.21);
        self.items.push(Item::Label { x: origin[0], y, size, text, anchor: Anchor::Start, ink: Ink::Rgb(colour) });
    }

    /// Puts the last page on the document.
    pub(crate) fn finish(mut self, doc: &mut Document) -> Result<(), ProofError> {
        self.page_break(doc)
    }
}

/// The synopsis: a grid of cells, each with glyph id and advance on top, the glyph, and its name
/// under it, in the order of the glyphs to show.
pub(crate) fn synopsis(face: &Face<'_>, doc: &mut Document, options: &ProofOptions, d: &Display, fit: Fit) -> Result<(), ProofError> {
    let [x0, y0, w, h] = page::content(options.page_size);
    let cell = match fit {
        Fit::Size(size) => size / SYNOPSIS_GLYPH,
        Fit::PerPage(n) => synopsis_cell(w, h, n)?,
    };
    let (cols, rows) = ((w / cell).floor(), (h / cell).floor());
    if !cell.is_finite() || cols < 1.0 || rows < 1.0 {
        return Err(ProofError::BadOptions("a synopsis cell of this size does not fit on the page".into()));
    }
    let (cols, rows) = (cols as usize, rows as usize);
    let per_page = cols.saturating_mul(rows);
    let count = doc.glyphs.len();
    if doc.pages.len().saturating_add(count.div_ceil(per_page)) > crate::MAX_PAGES {
        return Err(ProofError::BadOptions(format!("the proof would have more than {} pages: select fewer glyphs", crate::MAX_PAGES)));
    }
    let label = (cell * 0.12).min(9.0);
    let glyph_size = cell * SYNOPSIS_GLYPH;
    let (scale, base) = page::em_fit(face, glyph_size);
    let max_chars = ((cell / (label * 0.55)).floor() as usize).max(1);
    let colour = Ink::Rgb(d.meta_colour);
    for start in (0..count).step_by(per_page) {
        let mut items = Vec::new();
        for (slot, index) in (start..count.min(start + per_page)).enumerate() {
            let Some(g) = doc.glyphs.get(index) else { continue };
            let (cx, cy) = (x0 + (slot % cols) as f64 * cell, y0 + (slot / cols) as f64 * cell);
            items.push(Item::Rect { x: cx, y: cy, w: cell, h: cell, ink: Ink::Guide });
            let top = cy + label * 1.1;
            if d.numeric_labels {
                let gid = g.gid.to_string();
                items.push(Item::Label { x: cx + label * 0.3, y: top, size: label, text: gid, anchor: Anchor::Start, ink: colour });
                let adv = crate::text::num(g.outline.advance);
                items.push(Item::Label { x: cx + cell - label * 0.3, y: top, size: label, text: adv, anchor: Anchor::End, ink: colour });
            }
            if d.meta_name {
                let name = page::fit_text(&g.name, max_chars);
                items.push(Item::Label {
                    x: cx + label * 0.3,
                    y: cy + cell - label * 0.35,
                    size: label,
                    text: name,
                    anchor: Anchor::Start,
                    ink: colour,
                });
            }
            let origin = [cx + (cell - g.outline.advance * scale) / 2.0, cy + cell * 0.18 + base];
            draw_glyph(face, doc, d, index, origin, scale, &mut items);
        }
        page::new_page(doc, options.page_size)?.items = items;
    }
    Ok(())
}

/// The largest synopsis cell at which `n` cells fit the area `w` by `h`.
fn synopsis_cell(w: f64, h: f64, n: usize) -> Result<f64, ProofError> {
    let fits = |c: f64| ((w / c).floor() * (h / c).floor()) >= n as f64;
    if n == 0 || n > MAX_SHEET_GLYPHS || !fits(MIN_CELL) {
        return Err(ProofError::BadOptions(format!("{n} synopsis cells do not fit on one page of this size")));
    }
    let (mut lo, mut hi) = (MIN_CELL, w.min(h).max(MIN_CELL));
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

/// The `cmap` sheet: every code point that maps to one of the glyphs to show, in code-point
/// order, as its glyph with advance ticks and the label `name@gid code`. A glyph mapped from
/// several code points appears once for each.
pub(crate) fn cmap(face: &Face<'_>, doc: &mut Document, options: &ProofOptions, d: &Display, gids: &[u32], size: f64) -> Result<(), ProofError> {
    let selected: HashSet<u32> = gids.iter().copied().collect();
    let entries: Vec<(u32, u32)> = face.mappings.iter().copied().filter(|(_, gid)| selected.contains(gid)).collect();
    if entries.is_empty() {
        return Err(ProofError::BadOptions("none of the glyphs to show is mapped in cmap".into()));
    }
    let mut glyphs = Glyphs::new(doc);
    let mut flow = Flow::new(face, options.page_size, size, true);
    for (cp, gid) in entries {
        let index = glyphs.get(face, doc, gid)?;
        let Some(g) = doc.glyphs.get(index) else { continue };
        let label = glyph_label(d, g, Some(cp));
        let width = flow.cell(g.outline.advance);
        let origin = flow.place(doc, width)?;
        draw_glyph(face, doc, d, index, origin, flow.scale, &mut flow.items);
        if let Some(text) = label {
            flow.label(origin, text, d.meta_colour);
        }
    }
    flow.finish(doc)
}

/// The vertical-origin sheet: each glyph in its vertical em box, from its vertical origin down
/// by its vertical advance, with ticks at the middle of the top and bottom edges and the label
/// `name@gid`, advance and origin. A font with `VORG` records for some of the glyphs to show
/// shows those; otherwise every glyph to show, with the origin from `vmtx` or the em box.
pub(crate) fn vorg(face: &Face<'_>, doc: &mut Document, options: &ProofOptions, d: &Display, size: f64) -> Result<(), ProofError> {
    let records: HashSet<u32> =
        face.font.vorg().map(|v| v.vert_origin_y_metrics().iter().map(|m| m.glyph_index().to_u32()).collect()).unwrap_or_default();
    let count = doc.glyphs.len();
    let mut shown: Vec<usize> = (0..count).filter(|i| doc.glyphs.get(*i).is_some_and(|g| records.contains(&g.gid))).collect();
    if shown.is_empty() {
        shown = (0..count).collect();
    }
    let [x0, y0, w, h] = page::content(options.page_size);
    let label = size * LABEL;
    let scale = size / face.upem;
    let (cell_w, cell_h) = (size * 1.4, size * 1.25 + label * 2.8);
    let (cols, rows) = ((w / cell_w).floor(), (h / cell_h).floor());
    if cols < 1.0 || rows < 1.0 {
        return Err(ProofError::BadOptions(format!("a glyph of {size} points does not fit on the page")));
    }
    let (cols, rows) = (cols as usize, rows as usize);
    let per_page = cols.saturating_mul(rows);
    if doc.pages.len().saturating_add(shown.len().div_ceil(per_page)) > crate::MAX_PAGES {
        return Err(ProofError::BadOptions(format!("the proof would have more than {} pages: select fewer glyphs", crate::MAX_PAGES)));
    }
    let mut flat = d.clone();
    flat.em_box = false;
    flat.x_advance = false;
    for chunk in shown.chunks(per_page) {
        let mut items = Vec::new();
        for (slot, &index) in chunk.iter().enumerate() {
            let Some(g) = doc.glyphs.get(index) else { continue };
            let (cx, cy) = (x0 + (slot % cols) as f64 * cell_w, y0 + (slot / cols) as f64 * cell_h);
            let y_max = g.outline.bounds.map_or(0.0, |b| b[3]);
            let (v_origin, v_adv) = super::tiles::vertical(face, g.gid, y_max, face.descender);
            // The box's top edge, the vertical origin, sits a little below the cell's top.
            let origin = [cx, cy + size * 0.1 + v_origin * scale];
            let at = |fx: f64, fy: f64| [origin[0] + fx * scale, origin[1] - fy * scale];
            let line =
                |a: [f64; 2], b: [f64; 2], colour: [u8; 3], items: &mut Vec<Item>| items.push(Item::Line { from: a, to: b, ink: Ink::Rgb(colour) });
            if d.em_box {
                let [ax, ay] = at(0.0, v_origin);
                items.push(Item::Rect { x: ax, y: ay, w: face.upem * scale, h: v_adv * scale, ink: Ink::Rgb(d.embox_colour) });
            }
            draw_glyph(face, doc, &flat, index, origin, scale, &mut items);
            if d.x_advance {
                let (mid, tick) = (face.upem / 2.0, face.upem * 0.04);
                for (y, out) in [(v_origin, tick), (v_origin - v_adv, -tick)] {
                    line(at(mid, y), at(mid, y + out), d.x_advance_colour, &mut items);
                    line(at(mid - tick / 2.0, y), at(mid + tick / 2.0, y), d.x_advance_colour, &mut items);
                }
            }
            let below = cy + size * 0.1 + v_adv.max(face.upem) * scale;
            let ink = Ink::Rgb(d.meta_colour);
            if let Some(text) = glyph_label(d, g, None) {
                items.push(Item::Label { x: cx, y: below + label * 1.2, size: label, text, anchor: Anchor::Start, ink });
            }
            if d.numeric_labels {
                let text = format!("adv {}  y {}", crate::text::num(v_adv), crate::text::num(v_origin));
                items.push(Item::Label { x: cx, y: below + label * 2.4, size: label, text, anchor: Anchor::Start, ink });
            }
        }
        page::new_page(doc, options.page_size)?.items = items;
    }
    Ok(())
}

/// The text sheet: `text` set as outlines, each character's glyph from `cmap` (`.notdef` when
/// it has none) with the font's advances and pair kerning, a new line at each line break and
/// wherever the next glyph would pass the right margin. An empty text sets the glyphs to show.
pub(crate) fn text(
    face: &Face<'_>,
    doc: &mut Document,
    options: &ProofOptions,
    d: &Display,
    gids: &[u32],
    text: &str,
    size: f64,
) -> Result<(), ProofError> {
    let lines: Vec<Vec<u32>> = if text.trim().is_empty() {
        vec![gids.to_vec()]
    } else {
        text.lines().map(|l| l.chars().filter(|c| !c.is_control()).map(|c| face.glyph_for(c).unwrap_or(0)).collect()).collect()
    };
    let total = lines.iter().fold(0usize, |n, l| n.saturating_add(l.len()));
    if total > MAX_TEXT_GLYPHS {
        return Err(ProofError::BadOptions(format!("the text sets more than {MAX_TEXT_GLYPHS} glyphs")));
    }
    if total == 0 {
        return Err(ProofError::BadOptions("there is no text to set".into()));
    }
    let mut glyphs = Glyphs::new(doc);
    let mut kerning = crate::kern::Kerning::new(&face.font);
    let mut flow = Flow::new(face, options.page_size, size, false);
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            flow.skip_line();
        }
        let mut previous: Option<u32> = None;
        for &gid in line {
            let index = glyphs.get(face, doc, gid)?;
            let advance = doc.glyphs.get(index).map_or(0.0, |g| g.outline.advance);
            if let Some(left) = previous {
                flow.shift(kerning.pair(left, gid) * flow.scale);
            }
            let origin = flow.place(doc, advance.max(0.0) * flow.scale)?;
            draw_glyph(face, doc, d, index, origin, flow.scale, &mut flow.items);
            previous = Some(gid);
        }
    }
    flow.finish(doc)
}
