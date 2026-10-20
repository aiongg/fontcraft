//! Text Preview shaping (rows `textPreview.shaping`, `textPreview.unicodeBidi`,
//! `textPreview.setLayoutOrientation`): one line or column of a [`Layout`] shaped through the full preview font (`GSUB` and `GPOS`, the generated
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
//! A vertical layout's columns are shaped top to bottom ([`vertical_features`]): `vert` (or
//! `vrt2` when the font has it), vertical kerning (`vkrn`) and mark positioning from the compiled
//! font, vertical advances and origins live from the model (`live.rs`). The model's own
//! vertical kerning, which the compiled font does not hold, adds the layout's kerning between
//! neighbours.
//!
//! A cluster's glyphs all belong to its first item; the other items of the cluster (a ligature's
//! components after the first, the marks shaped with their base) take no room and draw nothing.
//! Items the preview font cannot reach (a line break, a missing character, a glyph added since
//! the font was built) keep the glyph and advance the layout gave them and end a run. A shaping
//! error leaves its script run as laid out by code point, and the first message is kept.

use std::sync::Arc;

use fontcraft_model::Font;
use fontcraft_shaping::{Direction as ShapeDirection, FeatureSetting, ShapeRequest, Tag};
use fontcraft_spacing::kerning::Direction;

use crate::bidi;
use crate::layout::{Layout, Placed};
use crate::live::Live;
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

/// The features vertical text is shaped with beyond the shaper's defaults for vertical text
/// (which hold `vert` and the mark features): vertical kerning, and `vrt2` in place of `vert` when
/// the font has it, since `vrt2` holds every substitution of `vert`.
pub fn vertical_features(shaper: &PreviewShaper) -> Vec<FeatureSetting> {
    const VERT: Tag = Tag::new(b"vert");
    const VRT2: Tag = Tag::new(b"vrt2");
    let mut out = vec![FeatureSetting::global(Tag::new(b"vkrn"), true)];
    if shaper.features().contains(&VRT2) {
        out.push(FeatureSetting::global(VRT2, true));
        out.push(FeatureSetting::global(VERT, false));
    }
    out
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

/// Shapes line `line` (a column, in vertical text) of `layout`, which [`crate::layout()`] made
/// from `text` in `master` with [`crate::Options::bidi`]. `None` for a line the layout does not
/// have.
pub fn shape_line(font: &Font, master: &str, text: &Text, layout: &Layout, line: usize, shaper: &PreviewShaper) -> Option<ShapedLine> {
    let info = layout.lines.get(line)?;
    let vertical = layout.direction == Direction::Vertical;
    let rtl = layout.direction == Direction::Rtl;
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

    let live = Live::new(font, master, shaper);
    let features = if vertical { vertical_features(shaper) } else { Vec::new() };
    let how = How { live: &live, features: &features, vertical };

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
            run.shape(shaper, &how, run_level, range.start, &mut cells, &mut error);
            run_level = level;
        }
        if let Some(c) = c {
            run.push(i, c);
        }
    }
    run.shape(shaper, &how, run_level, range.start, &mut cells, &mut error);

    if vertical {
        return Some(place_column(cells, layout, range.start, line, info.baseline, error));
    }

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

/// A column from the top: each glyph's origin is the pen, at the vertical origin, plus its
/// offsets; the layout's kerning (the model's vertical kerning) stands before each item but the
/// first.
fn place_column(cells: Vec<Cell>, layout: &Layout, first: usize, line: usize, centre: f64, error: Option<String>) -> ShapedLine {
    let mut along = 0.0f64;
    let placed = cells
        .into_iter()
        .enumerate()
        .map(|(k, cell)| {
            let kern = if k == 0 { 0.0 } else { layout.placed.get(first + k).map_or(0.0, |p| finite_or_zero(p.kern)) };
            along += kern;
            let offset = along;
            let mut pen = -along;
            let mut at = cell.glyphs.into_iter().map(|(name, advance, dx, dy)| {
                let origin = (centre + dx, pen + dy);
                pen -= advance;
                (name, origin.0, origin.1)
            });
            let (glyph, x, y) = match at.next() {
                Some((name, x, y)) => (Some(name), x, y),
                None => (None, centre, -along),
            };
            let extra: Vec<(Arc<str>, f64, f64)> = at.collect();
            along += cell.advance;
            Placed { glyph, x, y, advance: cell.advance, offset, kern, line, extra, cursive: false }
        })
        .collect();
    ShapedLine { placed, length: along, error }
}

/// How runs are shaped: the live metrics, the features beyond the defaults, the orientation.
struct How<'a> {
    live: &'a Live<'a>,
    features: &'a [FeatureSetting],
    vertical: bool,
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
    fn shape(&mut self, shaper: &PreviewShaper, how: &How<'_>, level: u8, first: usize, cells: &mut [Cell], error: &mut Option<String>) {
        if self.text.is_empty() {
            return;
        }
        let direction = if how.vertical {
            ShapeDirection::TopToBottom
        } else if level % 2 == 1 {
            ShapeDirection::RightToLeft
        } else {
            ShapeDirection::LeftToRight
        };
        let live_advance = |gid: u16| how.live.advance(gid);
        let live_vertical = |gid: u16| how.live.vertical(gid);
        for script in fontcraft_shaping::script::itemize(&self.text) {
            let Some(slice) = self.text.get(script.range.clone()) else { continue };
            let request = ShapeRequest {
                text: slice,
                direction,
                script: Some(script.script),
                language: None,
                features: how.features,
                advances: Some(&live_advance),
                vertical: how.vertical.then_some(&live_vertical as &dyn Fn(u16) -> Option<fontcraft_shaping::VerticalMetrics>),
            };
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
                // Along the line: a vertical run's advances point down.
                let advance = if how.vertical { -f64::from(g.y_advance) } else { f64::from(g.x_advance) };
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
