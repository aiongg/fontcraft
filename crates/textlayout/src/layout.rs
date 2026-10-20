//! Placing the items of a text: advances, kerning, lines and wrapping, the writing direction,
//! and hit testing.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use fontcraft_model::plist::Value;
use fontcraft_model::{Font, Glyph, Master, metric};
use fontcraft_spacing::kerning::{Direction, KernError, KernIndex, Table};
use fontcraft_spacing::metrics_source;

use crate::bidi::{self, Embedding};
use crate::cursive;
use crate::linebreak::{self, Break};
use crate::measure::{self, Measure};
use crate::shape::{Shaping, shape_items};
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
    /// Horizontal text only: order runs of the other direction by the Unicode Bidirectional
    /// Algorithm ([`crate::bidi`]), so a left-to-right word in right-to-left text keeps its
    /// letters in order. Off, every item follows `direction`.
    pub bidi: bool,
    /// With `bidi`: ranges of items with their own base direction.
    pub embeddings: &'a [Embedding],
    /// Edit View: the substitutions of the compiled preview font, applied before the items are
    /// placed ([`crate::shape`]). Not combined with `bidi`, which it ignores.
    pub shaping: Option<Shaping<'a>>,
    /// Edit View: in right-to-left text, attach each glyph's `entry` anchor to the `exit` anchor
    /// of the glyph before it ([`crate::cursive`]).
    pub cursive: bool,
    /// Text Preview: lines break at the opportunities of the Unicode Line Breaking Algorithm
    /// ([`crate::linebreak`]), and a word wider than the line breaks between two clusters. Off,
    /// they break after the last space of the line, else before the glyph that overflows.
    pub break_opportunities: bool,
    /// Text Preview, horizontal text: the items are measured through the full preview font
    /// ([`crate::measure`]) before lines break, so kerning and substitutions decide where a line
    /// ends. Measured items do not kern from the document's kerning.
    pub measure: Option<Measure<'a>>,
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
            bidi: false,
            embeddings: &[],
            shaping: None,
            cursive: false,
            break_opportunities: false,
            measure: None,
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
    /// The kerning applied between the previous item and this one; with [`Placed::cursive`], the
    /// shift that cursive attachment applied instead.
    pub kern: f64,
    pub line: usize,
    /// Shaping made this item several glyphs: the ones after [`Placed::glyph`], each with its
    /// origin.
    pub extra: Vec<(Arc<str>, f64, f64)>,
    /// The glyph is attached to the one before it by cursive attachment, which replaces the
    /// kerning between them.
    pub cursive: bool,
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
    /// Why some of the text could not be shaped (it is laid out by code point there).
    pub shaping_error: Option<String>,
    /// With [`Options::bidi`]: the UAX #9 level of every item, each line's trailing blanks reset
    /// to the paragraph's; `None` when every item runs in the layout's direction.
    pub levels: Option<Vec<u8>>,
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
    /// The kerning id of the item's first glyph (its left side) and of its last glyph (its right
    /// side); they differ only when shaping made the item several glyphs.
    kern_id: Option<usize>,
    kern_last: Option<usize>,
    /// The glyphs after the first and their advances, when shaping made the item several glyphs.
    extra: Vec<(Arc<str>, f64)>,
    /// Shaping merged the item's glyph into an earlier item's.
    merged: bool,
    /// The first glyph's `entry` anchor and the last glyph's `exit` anchor, for cursive
    /// attachment.
    entry: Option<(f64, f64)>,
    exit: Option<(f64, f64)>,
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
    let mut levels = (options.bidi && direction != Direction::Vertical)
        .then(|| bidi::levels(text.items(), direction == Direction::Rtl, options.embeddings))
        .flatten();
    // A run of the other direction kerns from that direction's table.
    let other: Option<Table<'_>> = match (&levels, index) {
        (Some(_), Some(i)) => Some(i.table(if direction == Direction::Rtl { Direction::Ltr } else { Direction::Rtl }, master)?),
        _ => None,
    };

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
        let (entry, exit) = if options.cursive { cursive::anchors(layer) } else { (None, None) };
        Metrics {
            glyph: Some(name),
            kern_id,
            kern_last: kern_id,
            extra: Vec::new(),
            merged: false,
            entry,
            exit,
            advance,
            width,
            top: asc - vo,
            is_break: false,
            is_space,
        }
    };
    let blank = |is_break: bool, advance: f64, is_space: bool| Metrics {
        glyph: None,
        kern_id: None,
        kern_last: None,
        extra: Vec::new(),
        merged: false,
        entry: None,
        exit: None,
        advance,
        width: advance,
        top: 0.0,
        is_break,
        is_space,
    };
    let mut items: Vec<Metrics> = text
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

    // Shaping: an item shows the glyphs the preview font's substitutions give it.
    let mut shaping_error = None;
    if let Some(shaping) = options.shaping.as_ref() {
        let (shaped, error) = shape_items(font, text, direction, shaping);
        shaping_error = error;
        for (m, glyphs) in items.iter_mut().zip(shaped) {
            let Some(glyphs) = glyphs else { continue };
            let mut found = glyphs.into_iter().filter_map(|name| font.glyph(&name).map(|g| metrics_of(name.clone(), g)));
            let Some(mut first) = found.next() else {
                *m = Metrics { merged: true, ..blank(false, 0.0, false) };
                continue;
            };
            first.is_space = m.is_space;
            for more in found {
                first.advance += more.advance;
                first.kern_last = more.kern_id;
                first.exit = more.exit;
                if let Some(name) = more.glyph {
                    first.extra.push((name, more.advance));
                }
            }
            *m = first;
        }
    }
    // Text Preview: break opportunities, and the items measured by shaping.
    let opportunities: Option<Vec<Break>> =
        (options.break_opportunities || options.measure.is_some()).then(|| linebreak::item_opportunities(text.items()));
    let mut measured: Vec<bool> = Vec::new();
    let mut joined: Vec<bool> = Vec::new();
    if let (Some(m), Some(breaks)) = (options.measure.as_ref(), opportunities.as_deref())
        && direction != Direction::Vertical
    {
        let shaped = measure::measure(font, master, text.items(), levels.as_deref(), u8::from(direction == Direction::Rtl), breaks, m);
        if shaping_error.is_none() {
            shaping_error = shaped.error;
        }
        for (item, advance) in items.iter_mut().zip(&shaped.advances) {
            if let Some(a) = advance {
                item.advance = finite_or_zero(f64::from(*a));
                item.width = item.advance;
            }
        }
        measured = shaped.advances.iter().map(Option::is_some).collect();
        joined = shaped.joined;
    }
    // The nearest item before each one that is not merged into another: kerning and cursive
    // attachment reach across a ligature's merged components.
    let mut before: Vec<Option<usize>> = Vec::with_capacity(items.len());
    let mut last = None;
    for (i, m) in items.iter().enumerate() {
        before.push(last);
        if !m.merged {
            last = Some(i);
        }
    }
    let attaches = options.cursive && direction == Direction::Rtl;

    // The cursive attachment of item `i` to the item before it: the shift along the line that
    // puts its entry anchor on that item's exit anchor, and the rise from that anchor to its own.
    let attachment = |i: usize| -> Option<(f64, f64)> {
        if !attaches || items.get(i)?.merged {
            return None;
        }
        let p = before.get(i).copied().flatten()?;
        let (Some(exit), Some(entry)) = (items.get(p)?.exit, items.get(i)?.entry) else { return None };
        let between = items.get(p + 1..i).map_or(0.0, |merged| merged.iter().map(|m| m.advance).sum::<f64>());
        Some((cursive::shift(exit, entry, items.get(i)?.advance) - between, exit.1 - entry.1))
    };
    // Kerning between item `i` and the one before it, when both are glyphs (and, with bidi, at
    // the same level: two such items stand side by side in the same run). Cursive attachment
    // replaces it.
    let kern = |i: usize, levels: Option<&[u8]>| {
        if let Some((shift, _)) = attachment(i) {
            return shift;
        }
        let Some(p) = (if options.shaping.is_some() { before.get(i).copied().flatten() } else { i.checked_sub(1) }) else { return 0.0 };
        // The shaped widths hold the compiled kerning already.
        if measured.get(i).copied().unwrap_or(false) && measured.get(p).copied().unwrap_or(false) {
            return 0.0;
        }
        let table = match levels {
            None => table,
            Some(levels) => {
                let level = levels.get(i);
                if levels.get(p) != level {
                    None
                } else if level.is_some_and(|l| (l % 2 == 1) == (direction == Direction::Rtl)) {
                    table
                } else {
                    other
                }
            }
        };
        match (table, items.get(p).and_then(|m| m.kern_last), items.get(i).and_then(|m| m.kern_id)) {
            (Some(t), Some(a), Some(b)) => t.lookup(a, b).map_or(0.0, |applied| finite_or_zero(applied.value)),
            _ => 0.0,
        }
    };
    let kerns: Vec<f64> = (0..items.len()).map(|i| kern(i, levels.as_deref())).collect();

    // Lines: greedy. A line breaks after the last space of a line that overflows, else before the
    // glyph that overflows; with break opportunities, at the last opportunity, else before the
    // cluster that overflows. A line keeps at least one item (one cluster), and a space may hang
    // past the width.
    let n = items.len();
    let wrap = !options.single_line;
    let breaks = opportunities.as_deref();
    // Whether item `i` belongs to the cluster of the item before it.
    let continues = |i: usize| i.checked_sub(1).and_then(|p| breaks?.get(p)) == Some(&Break::Never) || joined.get(i).copied().unwrap_or(false);
    let mut offsets = vec![0.0; n];
    let mut used = vec![0.0; n];
    let mut line_of = vec![0usize; n];
    let mut lines: Vec<(Range<usize>, f64)> = Vec::new();
    let (mut i, mut start, mut pen, mut last_space) = (0usize, 0usize, 0.0f64, None::<usize>);
    // With break opportunities: where the line may last end, and where the cluster of the last
    // item placed begins.
    let (mut last_opportunity, mut cluster) = (None::<usize>, 0usize);
    while let Some(m) = items.get(i) {
        if m.is_break && wrap {
            if let (Some(o), Some(l)) = (offsets.get_mut(i), line_of.get_mut(i)) {
                *o = pen;
                *l = lines.len();
            }
            lines.push((start..i + 1, pen));
            (start, pen, last_space, last_opportunity) = (i + 1, 0.0, None, None);
            i += 1;
            continue;
        }
        let k = if i == start { 0.0 } else { kerns.get(i).copied().unwrap_or(0.0) };
        let end = pen + k + m.advance;
        if wrap && end > width_limit && i > start && !m.is_space {
            let b = match breaks {
                None => last_space.map_or(i, |s| s + 1),
                Some(_) => last_opportunity.unwrap_or(if continues(i) { cluster } else { i }),
            };
            if b > start {
                let length = b.checked_sub(1).and_then(|p| Some(offsets.get(p)? + items.get(p)?.advance)).unwrap_or(pen);
                lines.push((start..b, length));
                (start, pen, last_space, last_opportunity) = (b, 0.0, None, None);
                i = b;
                continue;
            }
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
        if i == start || !continues(i) {
            cluster = i;
        }
        match breaks.and_then(|b| b.get(i)) {
            Some(Break::Allowed) => last_opportunity = Some(i + 1),
            // A hard break other than a line break item (U+2028, a vertical tab).
            Some(Break::Mandatory) if wrap => {
                lines.push((start..i + 1, pen));
                (start, pen, last_space, last_opportunity) = (i + 1, 0.0, None, None);
            }
            _ => {}
        }
        i += 1;
    }
    lines.push((start..n, pen));

    // With bidi, each line's trailing blanks take the paragraph's level, which only the line
    // breaks above decide; their kerns are looked up again from those levels, so every kern
    // stands between two glyphs of the same run. Then the items move to their visual order, a
    // kern between the two glyphs it was looked up for (side by side, in either order), and the
    // line's length is the sum of its advances and kerns as placed.
    if let Some(levels) = levels.as_mut() {
        let rtl = direction == Direction::Rtl;
        let blank: Vec<bool> = items.iter().map(|m| m.is_space || m.is_break).collect();
        for (range, length) in &mut lines {
            let trailing = bidi::reset_trailing(range.clone(), levels, &blank, rtl);
            for i in trailing.max(range.start + 1)..range.end {
                if let Some(u) = used.get_mut(i) {
                    *u = kern(i, Some(levels));
                }
            }
            let order = bidi::visual_order(range.clone(), levels, rtl);
            let mut along = 0.0;
            let mut prev: Option<usize> = None;
            for &i in &order {
                let gap = match prev {
                    Some(a) if a + 1 == i => used.get(i).copied().unwrap_or(0.0),
                    Some(a) if i + 1 == a => used.get(a).copied().unwrap_or(0.0),
                    _ => 0.0,
                };
                along += gap;
                if let Some(o) = offsets.get_mut(i) {
                    *o = along;
                }
                along += items.get(i).map_or(0.0, |m| m.advance);
                prev = Some(i);
            }
            *length = along;
            if rtl {
                for &i in &order {
                    let advance = items.get(i).map_or(0.0, |m| m.advance);
                    if let Some(o) = offsets.get_mut(i) {
                        *o = along - *o - advance;
                    }
                }
            }
        }
    }

    // `+ 0.0` keeps the first line at 0 rather than −0.
    let base = |line: usize| -(line as f64 * lh) + 0.0;
    // Cursive rises add up along a chain of attached glyphs on one line.
    let mut rises = vec![0.0f64; n];
    let mut attached = vec![false; n];
    if attaches {
        for i in 0..n {
            let Some((_, rise)) = attachment(i) else { continue };
            let Some(p) = before.get(i).copied().flatten() else { continue };
            if line_of.get(p) != line_of.get(i) || line_of.get(i).and_then(|l| lines.get(*l)).is_some_and(|(r, _)| r.start == i) {
                continue;
            }
            let from = rises.get(p).copied().unwrap_or(0.0);
            if let (Some(r), Some(a)) = (rises.get_mut(i), attached.get_mut(i)) {
                *r = finite_or_zero(from + rise);
                *a = true;
            }
        }
    }
    let placed = items
        .iter()
        .zip(offsets.iter().zip(used.iter()).zip(line_of.iter()))
        .zip(rises.iter().zip(attached.iter()))
        .map(|((m, ((offset, kern), line)), (rise, cursive))| {
            let (x, y) = match direction {
                Direction::Ltr => (*offset, base(*line) + rise),
                Direction::Rtl => (-offset - m.advance, base(*line) + rise),
                Direction::Vertical => (base(*line) - m.width / 2.0, -offset - m.top),
            };
            // The first glyph of several stands at the start of the item's advance along the
            // line: on the right in right-to-left text, at the top in vertical text.
            let first_advance = m.advance - m.extra.iter().map(|(_, a)| a).sum::<f64>();
            let (x, y) = match direction {
                Direction::Rtl if !m.extra.is_empty() => (x + m.advance - first_advance, y),
                _ => (x, y),
            };
            let mut along = first_advance;
            let extra = m
                .extra
                .iter()
                .map(|(name, advance)| {
                    let at = match direction {
                        Direction::Ltr => (x + along, y),
                        Direction::Rtl => (x - along + first_advance - advance, y),
                        Direction::Vertical => (x, y - along),
                    };
                    along += advance;
                    (name.clone(), at.0, at.1)
                })
                .collect();
            Placed { glyph: m.glyph.clone(), x, y, advance: m.advance, offset: *offset, kern: *kern, line: *line, extra, cursive: *cursive }
        })
        .collect();
    let lines = lines.into_iter().enumerate().map(|(l, (items, length))| Line { items, baseline: base(l), length }).collect();
    let ascender = font.metric_pos(drawn, metric::ASCENDER, upm * 0.8);
    let breaks = items.iter().map(|m| m.is_break && wrap).collect();
    Ok(Layout { placed, lines, direction, line_height: lh, ascender, shaping_error, levels, breaks })
}
