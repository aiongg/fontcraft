//! Text Preview shaping (rows `textPreview.shaping`, `textPreview.unicodeBidi`): one line of a
//! horizontal [`Layout`] shaped through the full preview font (`GSUB` and `GPOS`, the generated
//! kerning and marks included), as an application shapes text.
//!
//! The layout gives the lines (broken at UAX #14 opportunities on the widths
//! [`crate::measure`] shaped, or on the glyphs' own widths and the document's kerning until a
//! preview font is ready) and the UAX #9 level of every item. [`shape_line`] then shapes each run of one level with the
//! script's default features and nothing else, in the run's own direction, so a Latin word in
//! right-to-left text is shaped left to right; each run is split into script runs first. Advances
//! come live from the model, kerning and mark positions from the compiled font, so width edits
//! show at once and kerning, anchor and feature edits once the font is built again.
//!
//! A cluster's glyphs all belong to its first item; the other items of the cluster (a ligature's
//! components after the first, the marks shaped with their base) take no room and draw nothing.
//! Items the preview font cannot reach (a line break, a missing character, a glyph added since
//! the font was built) keep the glyph and advance the layout gave them and end a run. A shaping
//! error leaves its script run as laid out by code point, and the first message is kept.

use std::sync::Arc;

use fontcraft_model::Font;
use fontcraft_shaping::{Direction as ShapeDirection, ShapeRequest};
use fontcraft_spacing::kerning::Direction;
use fontcraft_spacing::metrics_source;

use crate::bidi;
use crate::layout::{Layout, Placed};
use crate::shape::PreviewShaper;
use crate::text::{Item, Text};

/// A shaped line: one [`Placed`] per item of the line, in text order, positioned as
/// [`Layout::placed`] is (glyph origins in font units, the line's baseline at
/// [`crate::Line::baseline`], left-to-right lines starting at x = 0 and right-to-left ones
/// ending there).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedLine {
    pub placed: Vec<Placed>,
    /// The sum of the shaped advances.
    pub length: f64,
    /// Why some of the line could not be shaped.
    pub error: Option<String>,
}

/// One item while the line is shaped: its glyphs in visual order with their advance and offset
/// (font units), and its advance along the line.
#[derive(Clone, Default)]
struct Cell {
    glyphs: Vec<(Arc<str>, f64, f64, f64)>,
    advance: f64,
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Shapes line `line` of `layout`, which [`crate::layout()`] made from `text` in `master` with
/// [`crate::Options::bidi`]. `None` for vertical text and a line the layout does not have.
pub fn shape_line(font: &Font, master: &str, text: &Text, layout: &Layout, line: usize, shaper: &PreviewShaper) -> Option<ShapedLine> {
    let info = layout.lines.get(line)?;
    let rtl = match layout.direction {
        Direction::Ltr => false,
        Direction::Rtl => true,
        Direction::Vertical => return None,
    };
    let paragraph = u8::from(rtl);
    let range = info.items.clone();
    let items = text.items();
    let level_of = |i: usize| layout.levels.as_ref().and_then(|l| l.get(i).copied()).unwrap_or(paragraph);

    let mut cells: Vec<Cell> = range
        .clone()
        .map(|i| {
            let p = layout.placed.get(i);
            let advance = p.map_or(0.0, |p| finite_or_zero(p.advance));
            let glyphs = p.and_then(|p| p.glyph.clone()).map(|g| vec![(g, advance, 0.0, 0.0)]).unwrap_or_default();
            Cell { glyphs, advance }
        })
        .collect();

    let source = metrics_source(font, master).map_or(master, |m| m.id.as_str());
    let live = |gid: u16| -> Option<i32> {
        let glyph = font.glyph(shaper.glyph_name(gid)?)?;
        let layer = glyph.layer(source).or_else(|| glyph.layer(master))?;
        layer.width.is_finite().then(|| layer.width.round() as i32)
    };

    let mut error: Option<String> = None;
    let mut run = Run::default();
    let mut run_level = paragraph;
    for i in range.clone() {
        let c = match items.get(i) {
            Some(Item::Glyph { name, typed }) => shaper.char_of(font, name, *typed),
            _ => None,
        };
        let level = level_of(i);
        if c.is_none() || level != run_level {
            run.shape(shaper, &live, run_level, range.start, &mut cells, &mut error);
            run_level = level;
        }
        if let Some(c) = c {
            run.push(i, c);
        }
    }
    run.shape(shaper, &live, run_level, range.start, &mut cells, &mut error);

    // The items in visual order, left to right.
    let order: Vec<usize> = match &layout.levels {
        Some(levels) => bidi::visual_order(range.clone(), levels, rtl),
        None if rtl => range.clone().rev().collect(),
        None => range.clone().collect(),
    };
    let mut left = vec![0.0f64; cells.len()];
    let mut length = 0.0f64;
    for &i in &order {
        let Some(k) = i.checked_sub(range.start) else { continue };
        if let Some(l) = left.get_mut(k) {
            *l = length;
        }
        length += cells.get(k).map_or(0.0, |c| c.advance);
    }
    let baseline = info.baseline;
    let placed = cells
        .into_iter()
        .zip(left)
        .map(|(cell, along)| {
            // Right-to-left lines end at x = 0.
            let x0 = if rtl { along - length } else { along };
            let offset = if rtl { length - along - cell.advance } else { along };
            let mut pen = x0;
            let mut at = cell.glyphs.into_iter().map(|(name, advance, dx, dy)| {
                let origin = (pen + dx, baseline + dy);
                pen += advance;
                (name, origin.0, origin.1)
            });
            let (glyph, x, y) = match at.next() {
                Some((name, x, y)) => (Some(name), x, y),
                None => (None, x0, baseline),
            };
            let extra: Vec<(Arc<str>, f64, f64)> = at.collect();
            Placed { glyph, x, y, advance: cell.advance, offset, kern: 0.0, line, extra, cursive: false }
        })
        .collect();
    Some(ShapedLine { placed, length, error })
}

/// Consecutive shapeable items of one level as text: each item is one character.
#[derive(Default)]
struct Run {
    text: String,
    /// (byte offset in `text`, item index), one per character.
    starts: Vec<(usize, usize)>,
}

impl Run {
    fn push(&mut self, item: usize, c: char) {
        self.starts.push((self.text.len(), item));
        self.text.push(c);
    }

    /// The position in `starts` of the character at byte `at` (the last one starting at or
    /// before it); `starts.len()` past the end.
    fn local(&self, at: usize) -> usize {
        if at >= self.text.len() {
            return self.starts.len();
        }
        self.starts.partition_point(|(b, _)| *b <= at).saturating_sub(1)
    }

    /// Shapes the run at `level` into `cells` (indexed from item `first`) and empties it.
    fn shape(
        &mut self,
        shaper: &PreviewShaper,
        live: &dyn Fn(u16) -> Option<i32>,
        level: u8,
        first: usize,
        cells: &mut [Cell],
        error: &mut Option<String>,
    ) {
        if self.text.is_empty() {
            return;
        }
        let direction = if level % 2 == 1 { ShapeDirection::RightToLeft } else { ShapeDirection::LeftToRight };
        for script in fontcraft_shaping::script::itemize(&self.text) {
            let Some(slice) = self.text.get(script.range.clone()) else { continue };
            let request = ShapeRequest { text: slice, direction, script: Some(script.script), language: None, features: &[], advances: Some(live) };
            let glyphs = match fontcraft_shaping::shape(shaper.shaping_font(), &request) {
                Ok(g) => g,
                Err(e) => {
                    error.get_or_insert_with(|| e.to_string());
                    continue;
                }
            };
            // Every item of the script run gives up its own glyph; each cluster's glyphs then go
            // to the cluster's first item.
            let span = self.local(script.range.start)..self.local(script.range.end);
            for local in span {
                if let Some(cell) = self.starts.get(local).and_then(|(_, item)| cells.get_mut(item.wrapping_sub(first))) {
                    *cell = Cell::default();
                }
            }
            for g in glyphs {
                let local = self.local(script.range.start + g.cluster as usize);
                let Some(cell) = self.starts.get(local).and_then(|(_, item)| cells.get_mut(item.wrapping_sub(first))) else { continue };
                let advance = f64::from(g.x_advance);
                cell.advance += advance;
                if let Some(name) = shaper.glyph_name(g.glyph) {
                    cell.glyphs.push((name.clone(), advance, f64::from(g.x_offset), f64::from(g.y_offset)));
                }
            }
        }
        self.text.clear();
        self.starts.clear();
    }
}
