//! Placing the items of a text: advances, kerning, lines and wrapping, the writing direction,
//! and hit testing.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use fontcraft_model::plist::Value;
use fontcraft_model::{Font, Glyph, Master, metric};
use fontcraft_spacing::kerning::{Direction, KernError, KernIndex, Table};
use fontcraft_spacing::metrics_source;

use crate::text::{Item, Text};

/// The master parameter that sets the distance between lines.
pub const LINE_HEIGHT_PARAM: &str = "EditView Line Height";
/// Lines wrap at this width unless the caller gives another.
pub const DEFAULT_LINE_WIDTH: f64 = 10_000.0;
/// Allowed line widths and line heights, in font units.
pub const LENGTH_RANGE: std::ops::RangeInclusive<f64> = 1.0..=1_000_000.0;
/// Allowed advances for a missing glyph.
pub const MISSING_ADVANCE_RANGE: std::ops::RangeInclusive<f64> = 0.0..=65_535.0;

/// How the layout applies kerning. Kerning and locked kerning lay text out the same way; they
/// differ in which shortcuts are live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KerningMode {
    None,
    #[default]
    Kerning,
    Locked,
}

impl KerningMode {
    pub const ALL: [KerningMode; 3] = [KerningMode::None, KerningMode::Kerning, KerningMode::Locked];

    pub fn name(self) -> &'static str {
        match self {
            KerningMode::None => "none",
            KerningMode::Kerning => "kerning",
            KerningMode::Locked => "locked",
        }
    }

    pub fn from_name(name: &str) -> Option<KerningMode> {
        KerningMode::ALL.into_iter().find(|m| m.name() == name)
    }

    /// Whether the layout applies kerning in this mode.
    pub fn shows_kerning(self) -> bool {
        self != KerningMode::None
    }
}

/// What [`layout`] does.
#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub direction: Direction,
    pub kerning: KerningMode,
    /// The length at which lines wrap (columns, in vertical text).
    pub line_width: f64,
    /// The distance between lines (columns); `None` for [`line_height`].
    pub line_height: Option<f64>,
    /// One line without wrapping; line breaks take no room (the preview line).
    pub single_line: bool,
    /// The advance of a missing character or glyph; `None` for half the em.
    pub missing_advance: Option<f64>,
    /// The kerning index of this font revision; `None` builds one when kerning is shown.
    pub index: Option<&'a KernIndex>,
    /// Glyphs shown in another layer than their layer of the master: glyph name → layer id. The
    /// width and vertical metrics come from that layer; a layer the glyph lacks is ignored.
    pub layers: Option<&'a BTreeMap<String, String>>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options {
            direction: Direction::Ltr,
            kerning: KerningMode::Kerning,
            line_width: DEFAULT_LINE_WIDTH,
            line_height: None,
            single_line: false,
            missing_advance: None,
            index: None,
            layers: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum LayoutError {
    #[error("the font has no master `{0}`")]
    UnknownMaster(String),
    #[error("`{name}` must be a finite number in {range}, not {value}")]
    BadOption { name: &'static str, value: f64, range: String },
    #[error(transparent)]
    Kern(#[from] KernError),
}

/// One item as laid out. Coordinates are font units with y up; the first line's baseline (the
/// first column's centre, in vertical text) is at 0, and lines follow downwards (columns
/// leftwards).
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The glyph drawn: a placeholder's resolved glyph; `None` for a line break, a missing
    /// character, a glyph the font lacks and a placeholder without a current glyph.
    pub glyph: Option<Arc<str>>,
    /// The glyph's origin.
    pub x: f64,
    pub y: f64,
    /// The advance along the line: the width, or the vertical width in vertical text.
    pub advance: f64,
    /// The distance along the line from its start to the start of this item's advance.
    pub offset: f64,
    /// The kerning applied between the previous item and this one.
    pub kern: f64,
    pub line: usize,
}

/// One line (column) of a layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Its items, a closing line break included.
    pub items: Range<usize>,
    /// The baseline's y; in vertical text the column centre's x.
    pub baseline: f64,
    /// The length of its content along the line.
    pub length: f64,
}

/// A laid-out text: one [`Placed`] per item, in text order, and its lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub placed: Vec<Placed>,
    pub lines: Vec<Line>,
    pub direction: Direction,
    pub line_height: f64,
    /// The master's ascender: lines reach from it down by one line height.
    pub ascender: f64,
    breaks: Vec<bool>,
}

impl Layout {
    /// The cursor index nearest to a point: the line (column) under it, then the item boundary
    /// nearest along that line.
    pub fn hit(&self, x: f64, y: f64) -> usize {
        let lh = self.line_height;
        let (band, along) = match self.direction {
            Direction::Ltr => ((self.ascender - y) / lh, x),
            Direction::Rtl => ((self.ascender - y) / lh, -x),
            Direction::Vertical => ((0.5 * lh - x) / lh, -y),
        };
        let last = self.lines.len().saturating_sub(1);
        let index = if band.is_finite() && band > 0.0 { (band.floor() as usize).min(last) } else { 0 };
        let Some(line) = self.lines.get(index) else { return 0 };
        for i in line.items.clone() {
            let (Some(p), Some(is_break)) = (self.placed.get(i), self.breaks.get(i)) else { break };
            if *is_break || along < p.offset + p.advance / 2.0 {
                return i;
            }
        }
        line.items.end
    }

    /// Where the cursor at `index` stands: on the baseline (horizontal text) or on the column
    /// centre at the top of the next advance (vertical text). `index` is clamped to the text.
    pub fn caret(&self, index: usize) -> (f64, f64) {
        let (line, along) = match self.placed.get(index) {
            Some(p) => (p.line, p.offset),
            None => (self.lines.len().saturating_sub(1), self.lines.last().map_or(0.0, |l| l.length)),
        };
        let base = self.lines.get(line).map_or(0.0, |l| l.baseline);
        match self.direction {
            Direction::Ltr => (along, base),
            Direction::Rtl => (-along, base),
            Direction::Vertical => (base, -along),
        }
    }
}

/// The distance between lines in a master: its `EditView Line Height` parameter, else its
/// ascender minus its descender plus a tenth of the em.
pub fn line_height(font: &Font, master: &Master) -> f64 {
    let upm = f64::from(font.units_per_em);
    let param = master.params.get(LINE_HEIGHT_PARAM).and_then(|v| match v {
        Value::Number(n) => Some(n.value().as_f64()),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok(),
        _ => None,
    });
    let computed = font.metric_pos(master, metric::ASCENDER, upm * 0.8) - font.metric_pos(master, metric::DESCENDER, -upm * 0.2) + upm * 0.1;
    [param, Some(computed)].into_iter().flatten().find(|h| LENGTH_RANGE.contains(h)).unwrap_or(upm * 1.1)
}

fn check(name: &'static str, value: f64, range: &std::ops::RangeInclusive<f64>) -> Result<f64, LayoutError> {
    if range.contains(&value) {
        Ok(value)
    } else {
        Err(LayoutError::BadOption { name, value, range: format!("{} to {}", range.start(), range.end()) })
    }
}

/// What one item contributes before line breaking.
struct Metrics {
    glyph: Option<Arc<str>>,
    kern_id: Option<usize>,
    advance: f64,
    width: f64,
    /// From the glyph's origin up to the top of its vertical advance box.
    top: f64,
    is_break: bool,
    is_space: bool,
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// Lays `text` out in `master`. Widths and vertical metrics come from the layer
/// `options.layers` picks for a glyph, else from the master whose metrics `master` shows (linked
/// metrics); placeholders show the text's current glyph.
pub fn layout(font: &Font, master: &str, text: &Text, options: &Options<'_>) -> Result<Layout, LayoutError> {
    let drawn = font.masters.iter().find(|m| m.id == master).ok_or_else(|| LayoutError::UnknownMaster(master.to_string()))?;
    let source = metrics_source(font, master).unwrap_or(drawn);
    let upm = f64::from(font.units_per_em);
    let width_limit = check("lineWidth", options.line_width, &LENGTH_RANGE)?;
    let lh = match options.line_height {
        Some(h) => check("lineHeight", h, &LENGTH_RANGE)?,
        None => line_height(font, drawn),
    };
    let missing = check("missingAdvance", options.missing_advance.unwrap_or(upm * 0.5), &MISSING_ADVANCE_RANGE)?;
    let direction = options.direction;
    let (asc, desc) = (font.metric_pos(source, metric::ASCENDER, upm * 0.8), font.metric_pos(source, metric::DESCENDER, -upm * 0.2));

    let built;
    let index = match (options.kerning.shows_kerning(), options.index) {
        (false, _) => None,
        (true, Some(index)) => Some(index),
        (true, None) => {
            built = KernIndex::new(font);
            Some(&built)
        }
    };
    let table: Option<Table<'_>> = index.map(|i| i.table(direction, master)).transpose()?;

    let current = text.current_glyph().and_then(|n| font.glyph(n).map(|g| (Arc::<str>::from(n), g)));
    let metrics_of = |name: Arc<str>, glyph: &Glyph| {
        let picked = options.layers.and_then(|l| l.get(&*name)).and_then(|id| glyph.layers.iter().find(|l| l.id == *id));
        let layer = picked.or_else(|| glyph.layer(&source.id)).or_else(|| glyph.layer(master));
        let width = finite_or_zero(layer.map_or(0.0, |l| l.width));
        let vo = finite_or_zero(layer.and_then(|l| l.info.vert_origin).unwrap_or(0.0));
        let vw = finite_or_zero(layer.and_then(|l| l.info.vert_width).unwrap_or(asc - desc));
        let advance = if direction == Direction::Vertical { vw } else { width };
        let kern_id = index.and_then(|i| i.glyph_id(&name));
        let is_space = glyph.unicodes.first().is_some_and(|u| char::from_u32(*u).is_some_and(char::is_whitespace));
        Metrics { glyph: Some(name), kern_id, advance, width, top: asc - vo, is_break: false, is_space }
    };
    let blank =
        |is_break: bool, advance: f64, is_space: bool| Metrics { glyph: None, kern_id: None, advance, width: advance, top: 0.0, is_break, is_space };
    let items: Vec<Metrics> = text
        .items()
        .iter()
        .map(|item| match item {
            Item::Glyph { name, typed } => match font.glyph(name) {
                Some(g) => {
                    let mut m = metrics_of(name.clone(), g);
                    m.is_space |= typed.is_some_and(char::is_whitespace);
                    m
                }
                None => blank(false, missing, false),
            },
            Item::Placeholder => match &current {
                Some((name, g)) => metrics_of(name.clone(), g),
                None => blank(false, missing, false),
            },
            Item::Missing(c) => blank(false, missing, c.is_whitespace()),
            Item::LineBreak => blank(true, 0.0, false),
        })
        .collect();

    // Kerning with the previous item, when both are glyphs.
    let kerns: Vec<f64> = (0..items.len())
        .map(|i| {
            let prev = i.checked_sub(1).and_then(|p| items.get(p)).and_then(|m| m.kern_id);
            let this = items.get(i).and_then(|m| m.kern_id);
            match (table, prev, this) {
                (Some(t), Some(a), Some(b)) => t.lookup(a, b).map_or(0.0, |applied| finite_or_zero(applied.value)),
                _ => 0.0,
            }
        })
        .collect();

    // Lines: greedy, breaking after the last space of a line that overflows, else before the
    // glyph that overflows. A line keeps at least one item, and a space may hang past the width.
    let n = items.len();
    let wrap = !options.single_line;
    let mut offsets = vec![0.0; n];
    let mut used = vec![0.0; n];
    let mut line_of = vec![0usize; n];
    let mut lines: Vec<(Range<usize>, f64)> = Vec::new();
    let (mut i, mut start, mut pen, mut last_space) = (0usize, 0usize, 0.0f64, None::<usize>);
    while let Some(m) = items.get(i) {
        if m.is_break && wrap {
            if let (Some(o), Some(l)) = (offsets.get_mut(i), line_of.get_mut(i)) {
                *o = pen;
                *l = lines.len();
            }
            lines.push((start..i + 1, pen));
            (start, pen, last_space) = (i + 1, 0.0, None);
            i += 1;
            continue;
        }
        let k = if i == start { 0.0 } else { kerns.get(i).copied().unwrap_or(0.0) };
        let end = pen + k + m.advance;
        if wrap && end > width_limit && i > start && !m.is_space {
            let b = last_space.map_or(i, |s| s + 1);
            let length = b.checked_sub(1).and_then(|p| Some(offsets.get(p)? + items.get(p)?.advance)).unwrap_or(pen);
            lines.push((start..b, length));
            (start, pen, last_space) = (b, 0.0, None);
            i = b;
            continue;
        }
        if let (Some(o), Some(u), Some(l)) = (offsets.get_mut(i), used.get_mut(i), line_of.get_mut(i)) {
            *o = pen + k;
            *u = k;
            *l = lines.len();
        }
        pen = end;
        if m.is_space {
            last_space = Some(i);
        }
        i += 1;
    }
    lines.push((start..n, pen));

    // `+ 0.0` keeps the first line at 0 rather than −0.
    let base = |line: usize| -(line as f64 * lh) + 0.0;
    let placed = items
        .iter()
        .zip(offsets.iter().zip(used.iter()).zip(line_of.iter()))
        .map(|(m, ((offset, kern), line))| {
            let (x, y) = match direction {
                Direction::Ltr => (*offset, base(*line)),
                Direction::Rtl => (-offset - m.advance, base(*line)),
                Direction::Vertical => (base(*line) - m.width / 2.0, -offset - m.top),
            };
            Placed { glyph: m.glyph.clone(), x, y, advance: m.advance, offset: *offset, kern: *kern, line: *line }
        })
        .collect();
    let lines = lines.into_iter().enumerate().map(|(l, (items, length))| Line { items, baseline: base(l), length }).collect();
    let ascender = font.metric_pos(drawn, metric::ASCENDER, upm * 0.8);
    Ok(Layout { placed, lines, direction, line_height: lh, ascender, breaks: items.iter().map(|m| m.is_break && wrap).collect() })
}
