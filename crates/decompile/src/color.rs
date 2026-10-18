//! Colour tables of a compiled font read back into the model (`color.open.compiledColorTables`).
//!
//! - `CPAL` becomes the font's `Color Palettes` parameter (lists of 0–255 integer lists), every
//!   palette in table order.
//! - `COLR` version 0 records become palette layers: one layer per record, bottom first, with the
//!   outline of the record's layer glyph and its palette entry (65,535 is the text colour).
//! - A `COLR` version 1 glyph whose paints are all solid palette colours at full opacity becomes
//!   palette layers in the same way. Any other version 1 glyph becomes one full-colour layer, one
//!   group of paths per layer glyph, filled with a palette colour or a linear, radial or conic
//!   gradient whose geometry is stored as fractions of the group's bounding box. Colours stay
//!   palette references, so the palettes keep every row.
//! - A glyph whose paints use anything else (transformations, blends, nested clips, a fill that
//!   no glyph clips) keeps its plain outline, and the warnings name it.
//! - The layer glyphs a colour glyph used are removed: an export makes them again from the
//!   layers. A layer glyph is kept when it has a code point, is a colour glyph itself, or a
//!   remaining glyph uses it as a component.
//! - `sbix`, `SVG ` and `CBDT` colour glyphs are not read; a warning says so.
//!
//! Every master gets the colour layers, each made from the layer glyph's layer of that master,
//! with the base glyph's width on that master.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::plist::{Array, Layout, Number, Value};
use fontcraft_model::{
    Color, ColorStop, Extend, Family, Font, Glyph, Gradient, GradientKind, Layer, Node, NodeKind, PaletteEntry, Path, Shape, ShapeAttrs,
};
use skrifa::MetadataProvider;
use skrifa::color::{Brush, ColorPainter, CompositeMode, Transform};
use skrifa::prelude::LocationRef;
use skrifa::raw::types::{BoundingBox, GlyphId, Tag};
use skrifa::raw::{FontRef, TableProvider};

use crate::Decompiled;

/// The tables [`import`] reads, which `lib.rs` leaves out of its "tables not read" warning.
pub const TABLES: &[&str] = &["COLR", "CPAL"];
/// Most palettes read from `CPAL`; the rest are left out with a warning.
pub const MAX_PALETTES: usize = 256;
/// Most colours in one palette (65,535 is the text colour's index).
pub const MAX_PALETTE_ROWS: usize = 65_534;
/// Most palette colours read in all, palettes times rows.
pub const MAX_PALETTE_COLORS: usize = 1 << 20;
/// Most colour layers of one glyph, or layer glyphs in one full-colour layer.
pub const MAX_COLOR_LAYERS: usize = 1_024;
/// Most stops of one gradient (`fontcraft_model::MAX_GRADIENT_STOPS`).
pub const MAX_STOPS: usize = fontcraft_model::MAX_GRADIENT_STOPS;
/// Most painter calls over every colour glyph; past it the remaining glyphs keep their outlines.
pub const MAX_PAINT_CALLS: usize = 4_000_000;
/// Most nodes copied into colour layers over the whole font.
pub const MAX_COPIED_NODES: usize = 10_000_000;
/// Gradient positions as fractions of a bounding box are kept within this much.
const MAX_FRACTION: f64 = 10_000.0;
/// A bounding box narrower than this is treated as having no width.
const MIN_EXTENT: f64 = 1e-6;
const COLOR_PALETTES: &str = "Color Palettes";
const FOREGROUND: u16 = 0xFFFF;
/// Colour glyph tables this module does not read, with what they hold.
const LOST_TABLES: &[([u8; 4], &str)] = &[(*b"sbix", "bitmap"), (*b"SVG ", "SVG"), (*b"CBDT", "bitmap")];

/// Reads the colour tables of `sfnt` into `d.font`, adding what was left out to `d.warnings`.
pub(crate) fn import(sfnt: &FontRef, d: &mut Decompiled) {
    for (tag, what) in LOST_TABLES {
        if sfnt.table_data(Tag::new(tag)).is_some() {
            let name = String::from_utf8_lossy(tag);
            d.warnings.push(format!(
                "the {what} colour glyphs of the `{}` table were not read: those glyphs open with their outlines only",
                name.trim_end()
            ));
        }
    }
    let rows = read_palettes(sfnt, d);
    read_colr(sfnt, d, rows);
}

/// `CPAL` into `Color Palettes`; the number of rows read.
fn read_palettes(sfnt: &FontRef, d: &mut Decompiled) -> usize {
    if sfnt.cpal().is_err() {
        if sfnt.table_data(Tag::new(b"CPAL")).is_some() {
            d.warnings.push("the `CPAL` table could not be read: the colour palettes are lost".into());
        }
        return 0;
    }
    let all = sfnt.color_palettes();
    let count = usize::from(all.len());
    let mut palettes: Vec<Vec<Value>> = Vec::new();
    let mut rows = 0usize;
    for index in 0..all.len() {
        let Some(palette) = all.get(index) else {
            d.warnings.push(format!("palette {index} of the `CPAL` table could not be read; it and the later ones are left out"));
            break;
        };
        let colors = palette.colors();
        if palettes.is_empty() {
            rows = colors.len().min(MAX_PALETTE_ROWS);
            if colors.len() > MAX_PALETTE_ROWS {
                d.warnings.push(format!("the palettes have {} colours; only the first {MAX_PALETTE_ROWS} were read", colors.len()));
            }
        }
        if palettes.len() >= MAX_PALETTES || (palettes.len() + 1).saturating_mul(rows) > MAX_PALETTE_COLORS {
            d.warnings.push(format!("the `CPAL` table has {count} palettes; only the first {} were read", palettes.len()));
            break;
        }
        palettes.push(
            colors
                .iter()
                .take(rows)
                .map(|c| {
                    let n = |v: u8| Value::Number(Number::int(i64::from(v)));
                    Value::Array(Array {
                        items: vec![n(c.red()), n(c.green()), n(c.blue()), n(c.alpha())],
                        layout: Layout::Compact,
                        ..Array::default()
                    })
                })
                .collect(),
        );
    }
    if palettes.is_empty() || rows == 0 {
        return 0;
    }
    let value = Value::Array(Array {
        items: palettes.into_iter().map(|p| Value::Array(Array { items: p, layout: Layout::Multiline, ..Array::default() })).collect(),
        layout: Layout::Multiline,
        ..Array::default()
    });
    Arc::make_mut(&mut d.font.info).params.set(COLOR_PALETTES, value);
    rows
}

/// One stop of a gradient: offset, palette index, alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stop {
    offset: f64,
    index: u16,
    alpha: f64,
}

/// What one layer glyph is filled with, in font units.
#[derive(Clone, Debug, PartialEq)]
enum Fill {
    Solid { index: u16, alpha: f64 },
    Linear { p0: (f64, f64), p1: (f64, f64), stops: Vec<Stop>, extend: Extend },
    Radial { c0: (f64, f64), r0: f64, c1: (f64, f64), r1: f64, stops: Vec<Stop>, extend: Extend },
    Sweep { center: (f64, f64), start: f64, end: f64, stops: Vec<Stop>, extend: Extend },
}

/// A layer glyph and its fill: one layer of a colour glyph.
#[derive(Clone, Debug, PartialEq)]
struct Item {
    glyph: u32,
    fill: Fill,
}

fn unit(alpha: f32) -> f64 {
    let a = f64::from(alpha);
    if a.is_finite() { a.clamp(0.0, 1.0) } else { 1.0 }
}

fn finite(v: f32) -> f64 {
    let v = f64::from(v);
    if v.is_finite() { v } else { 0.0 }
}

fn extend(e: skrifa::color::Extend) -> Extend {
    match e {
        skrifa::color::Extend::Repeat => Extend::Repeat,
        skrifa::color::Extend::Reflect => Extend::Reflect,
        _ => Extend::Pad,
    }
}

fn stops(stops: &[skrifa::color::ColorStop]) -> Option<Vec<Stop>> {
    (stops.len() <= MAX_STOPS)
        .then(|| stops.iter().map(|s| Stop { offset: finite(s.offset), index: s.palette_index, alpha: unit(s.alpha) }).collect())
}

impl Fill {
    /// `None` for a gradient with more than [`MAX_STOPS`] stops.
    fn from_brush(brush: &Brush<'_>) -> Option<Fill> {
        let pt = |p: skrifa::raw::types::Point<f32>| (finite(p.x), finite(p.y));
        Some(match brush {
            Brush::Solid { palette_index, alpha } => Fill::Solid { index: *palette_index, alpha: unit(*alpha) },
            Brush::LinearGradient { p0, p1, color_stops, extend: e } => {
                Fill::Linear { p0: pt(*p0), p1: pt(*p1), stops: stops(color_stops)?, extend: extend(*e) }
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend: e } => Fill::Radial {
                c0: pt(*c0),
                r0: finite(*r0).abs(),
                c1: pt(*c1),
                r1: finite(*r1).abs(),
                stops: stops(color_stops)?,
                extend: extend(*e),
            },
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend: e } => {
                // The brush turns clockwise, with the font's angles and stops reversed when it
                // turned them around; the model counts counter-clockwise, as the font does.
                let (mut start, mut end) = (360.0 - finite(*start_angle), 360.0 - finite(*end_angle));
                let mut stops = stops(color_stops)?;
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                    stops.reverse();
                    for s in &mut stops {
                        s.offset = 1.0 - s.offset;
                    }
                }
                Fill::Sweep { center: pt(*c0), start, end, stops, extend: extend(*e) }
            }
        })
    }
}

/// Collects a colour glyph's paints as a list of layer glyphs and fills, and notes the first
/// paint that has no such form.
#[derive(Default)]
struct Collect {
    items: Vec<Item>,
    lost: Option<&'static str>,
    /// Clips, transformations and blend layers pushed and not yet popped, the root clip box aside.
    depth: usize,
    root_box: bool,
    calls: usize,
}

impl Collect {
    fn lose(&mut self, why: &'static str) {
        if self.lost.is_none() {
            self.lost = Some(why);
        }
    }
}

impl ColorPainter for Collect {
    fn push_transform(&mut self, _: Transform) {
        self.calls += 1;
        self.depth += 1;
    }

    fn pop_transform(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn push_clip_glyph(&mut self, _: GlyphId) {
        self.calls += 1;
        self.depth += 1;
    }

    fn push_clip_box(&mut self, _: BoundingBox<f32>) {
        self.calls += 1;
        // The clip box a version 1 glyph's clip list gives is pushed around the whole glyph.
        if self.depth == 0 && self.items.is_empty() && !self.root_box {
            self.root_box = true;
        } else {
            self.depth += 1;
        }
    }

    fn pop_clip(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        } else {
            self.root_box = false;
        }
    }

    fn fill(&mut self, _: Brush<'_>) {
        self.calls += 1;
        self.lose("a fill that is not clipped to one glyph");
    }

    fn fill_glyph(&mut self, glyph_id: GlyphId, brush_transform: Option<Transform>, brush: Brush<'_>) {
        self.calls += 1;
        if self.depth > 0 {
            return self.lose("a glyph drawn inside a clip, a transformation or a blend");
        }
        if brush_transform.is_some() {
            return self.lose("a transformed fill");
        }
        if self.items.len() >= MAX_COLOR_LAYERS {
            return self.lose("more than 1,024 layers");
        }
        match Fill::from_brush(&brush) {
            Some(fill) => self.items.push(Item { glyph: glyph_id.to_u32(), fill }),
            None => self.lose("a gradient with more than 256 stops"),
        }
    }

    fn push_layer(&mut self, _: CompositeMode) {
        self.calls += 1;
        self.depth += 1;
    }

    fn pop_layer(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

/// The base glyph ids of `COLR`, versions 0 and 1, each once, in id order.
fn base_glyphs(colr: &skrifa::raw::tables::colr::Colr<'_>) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    if let Some(Ok(records)) = colr.base_glyph_records() {
        out.extend(records.iter().map(|r| u32::from(r.glyph_id().to_u16())));
    }
    if let Some(Ok(list)) = colr.base_glyph_list() {
        out.extend(list.base_glyph_paint_records().iter().map(|r| u32::from(r.glyph_id().to_u16())));
    }
    out
}

/// `COLR` into colour layers, and the layer glyphs it used removed.
fn read_colr(sfnt: &FontRef, d: &mut Decompiled, rows: usize) {
    let colr = match sfnt.colr() {
        Ok(c) => c,
        Err(e) => {
            if sfnt.table_data(Tag::new(b"COLR")).is_some() {
                d.warnings.push(format!("the `COLR` table could not be read ({e}): the colour layers are lost"));
            }
            return;
        }
    };
    let glyphs = sfnt.color_glyphs();
    let mut calls = 0usize;
    let mut nodes = 0usize;
    let mut lost: Vec<(String, &'static str)> = Vec::new();
    let mut over_budget = false;
    let mut out_of_range = false;
    let mut colored: Vec<(String, Vec<Layer>)> = Vec::new();
    let mut used: HashSet<u32> = HashSet::new();
    let mut bases: HashSet<u32> = HashSet::new();
    let names: Vec<String> = d.font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
    for gid in base_glyphs(&colr) {
        let Some(name) = names.get(gid as usize) else { continue };
        bases.insert(gid);
        if calls > MAX_PAINT_CALLS || nodes > MAX_COPIED_NODES {
            over_budget = true;
            continue;
        }
        let Some(glyph) = glyphs.get(GlyphId::new(gid)) else { continue };
        let mut painter = Collect::default();
        let painted = glyph.paint(LocationRef::default(), &mut painter);
        calls = calls.saturating_add(painter.calls);
        if let Err(e) = painted {
            lost.push((
                name.clone(),
                if matches!(e, skrifa::color::PaintError::DepthLimitExceeded) { "paints nested too deep" } else { "unreadable paints" },
            ));
            continue;
        }
        if let Some(why) = painter.lost {
            lost.push((name.clone(), why));
            continue;
        }
        if painter.items.is_empty() {
            continue;
        }
        let uses_rows = |i: u16| i == FOREGROUND || usize::from(i) < rows;
        if !painter.items.iter().all(|it| fill_indices(&it.fill).all(uses_rows)) {
            out_of_range = true;
        }
        let layers = color_layers(&d.font, name, &names, &painter.items, &mut nodes);
        used.extend(painter.items.iter().map(|it| it.glyph));
        colored.push((name.clone(), layers));
    }
    if over_budget {
        d.warnings.push(format!(
            "the `COLR` table is too large to read whole (more than {MAX_PAINT_CALLS} paints or {MAX_COPIED_NODES} nodes): the later colour glyphs open with their outlines only"
        ));
    }
    if !lost.is_empty() {
        let shown: Vec<String> = lost.iter().take(5).map(|(g, why)| format!("`{g}` ({why})")).collect();
        let more = lost.len().saturating_sub(5);
        d.warnings.push(format!(
            "the colour of {} glyphs was not read, because their paints have no layer form: {}{}",
            lost.len(),
            shown.join(", "),
            if more > 0 { format!(" and {more} more") } else { String::new() }
        ));
    }
    if out_of_range {
        d.warnings.push("some colour layers use a palette colour the palettes do not have".into());
    }
    for (name, layers) in colored {
        if let Some(mut g) = d.font.glyphs.get_mut(&name) {
            g.layers.extend(layers);
        }
    }
    remove_layer_glyphs(&mut d.font, &names, &used, &bases);
}

fn fill_indices(fill: &Fill) -> Box<dyn Iterator<Item = u16> + '_> {
    match fill {
        Fill::Solid { index, .. } => Box::new(std::iter::once(*index)),
        Fill::Linear { stops, .. } | Fill::Radial { stops, .. } | Fill::Sweep { stops, .. } => Box::new(stops.iter().map(|s| s.index)),
    }
}

/// Whether `items` read as palette layers: every fill a solid colour at full opacity.
fn palette_form(items: &[Item]) -> bool {
    items.iter().all(|it| matches!(it.fill, Fill::Solid { alpha, .. } if alpha >= 1.0))
}

/// The colour layers of `base` for every master, made from `items`.
fn color_layers(font: &Font, base: &str, names: &[String], items: &[Item], nodes: &mut usize) -> Vec<Layer> {
    let Some(glyph) = font.glyph(base) else { return Vec::new() };
    let palette = palette_form(items);
    let mut out = Vec::new();
    let mut taken: HashSet<String> = glyph.layers.iter().map(|l| l.id.clone()).collect();
    for master in font.masters.iter() {
        let Some(width) = glyph.layers.iter().find(|l| l.id == master.id).map(|l| l.width) else { continue };
        let shapes_of = |gid: u32| -> Vec<Shape> {
            names
                .get(gid as usize)
                .and_then(|n| font.glyph(n))
                .and_then(|g| g.layers.iter().find(|l| l.id == master.id))
                .map(|l| l.shapes.clone())
                .unwrap_or_default()
        };
        let mut new_layer = |kind_name: String, entry: Option<PaletteEntry>, shapes: Vec<Shape>| {
            *nodes = nodes.saturating_add(shapes.iter().map(shape_nodes).sum());
            let mut l = Layer::new(master.id.as_str(), width);
            let mut n = 0usize;
            l.id = loop {
                let id = format!("{}.color{n}", master.id);
                n += 1;
                if taken.insert(id.clone()) {
                    break id;
                }
            };
            l.info.name = Some(kind_name);
            match entry {
                Some(e) => l.info.attrs.color_palette = Some(e),
                None => l.info.attrs.full_color = true,
            }
            l.shapes = shapes;
            l
        };
        if palette {
            for item in items {
                let Fill::Solid { index, .. } = item.fill else { continue };
                let (entry, label) = if index == FOREGROUND {
                    (PaletteEntry::Foreground, "Color *".to_string())
                } else {
                    (PaletteEntry::Index(index), format!("Color {index}"))
                };
                out.push(new_layer(label, Some(entry), shapes_of(item.glyph)));
            }
        } else {
            let mut shapes = Vec::new();
            for item in items {
                let mut group = shapes_of(item.glyph);
                let bounds = group_bounds(&group);
                for s in &mut group {
                    match s {
                        Shape::Path(p) => set_fill(&mut p.info.attrs, &item.fill, bounds),
                        Shape::Component(c) => set_fill(&mut c.info.attrs, &item.fill, bounds),
                        Shape::Image(_) | Shape::Raw(_) => {}
                    }
                }
                shapes.extend(group);
            }
            out.push(new_layer("Color".to_string(), None, shapes));
        }
    }
    out
}

fn shape_nodes(s: &Shape) -> usize {
    match s {
        Shape::Path(p) => p.nodes.len(),
        _ => 1,
    }
}

fn color(index: u16, alpha: f64) -> Color {
    Color::Palette { index, alpha }
}

/// A point as fractions of `bounds` (min x, min y, max x, max y), as a node whose
/// [`Node::point`] the gradient takes.
fn fraction(p: (f64, f64), bounds: [f64; 4]) -> Node {
    let [x0, y0, x1, y1] = bounds;
    let f = |v: f64, lo: f64, hi: f64| {
        let w = hi - lo;
        if w > MIN_EXTENT { ((v - lo) / w).clamp(-MAX_FRACTION, MAX_FRACTION) } else { 0.0 }
    };
    Node::new(f(p.0, x0, x1), f(p.1, y0, y1), NodeKind::Line)
}

fn set_fill(attrs: &mut ShapeAttrs, fill: &Fill, bounds: [f64; 4]) {
    let [x0, _, x1, _] = bounds;
    let width = x1 - x0;
    let radius = |r: f64| if width > MIN_EXTENT { (r / width).clamp(0.0, MAX_FRACTION) } else { 0.0 };
    let line = |stops: &[Stop]| stops.iter().map(|s| ColorStop { offset: s.offset, color: color(s.index, s.alpha) }).collect::<Vec<_>>();
    attrs.fill_color = None;
    attrs.gradient = None;
    match fill {
        Fill::Solid { index, alpha } => attrs.fill_color = Some(color(*index, *alpha)),
        Fill::Linear { p0, p1, stops, extend } => {
            attrs.gradient = Some(Gradient {
                kind: GradientKind::Linear { start: fraction(*p0, bounds).point(), end: fraction(*p1, bounds).point() },
                stops: line(stops),
                extend: *extend,
            });
        }
        Fill::Radial { c0, r0, c1, r1, stops, extend } => {
            attrs.gradient = Some(Gradient {
                kind: GradientKind::Radial {
                    start: fraction(*c0, bounds).point(),
                    end: fraction(*c1, bounds).point(),
                    start_radius: radius(*r0),
                    end_radius: radius(*r1),
                    angle: 0.0,
                },
                stops: line(stops),
                extend: *extend,
            });
        }
        Fill::Sweep { center, start, end, stops, extend } => {
            attrs.gradient = Some(Gradient {
                kind: GradientKind::Conic { center: fraction(*center, bounds).point(), start_angle: *start, angle: end - start, control_radius: 0.5 },
                stops: line(stops),
                extend: *extend,
            });
        }
    }
}

/// The bounding box of the paths in `shapes`, curve extremes included; all zero without paths.
fn group_bounds(shapes: &[Shape]) -> [f64; 4] {
    let mut b: Option<[f64; 4]> = None;
    for s in shapes {
        if let Shape::Path(p) = s {
            path_bounds(p, &mut b);
        }
    }
    b.unwrap_or([0.0; 4])
}

fn include(b: &mut Option<[f64; 4]>, (x, y): (f64, f64)) {
    if !(x.is_finite() && y.is_finite()) {
        return;
    }
    *b = Some(match *b {
        None => [x, y, x, y],
        Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
    });
}

fn lerp(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// A quadratic segment's end and its extremes on both axes.
fn quad_bounds(b: &mut Option<[f64; 4]>, p0: (f64, f64), c: (f64, f64), p1: (f64, f64)) {
    include(b, p1);
    for axis in 0..2 {
        let (a, m, z) = if axis == 0 { (p0.0, c.0, p1.0) } else { (p0.1, c.1, p1.1) };
        let denom = a - 2.0 * m + z;
        if denom.abs() > f64::EPSILON {
            let t = (a - m) / denom;
            if t > 0.0 && t < 1.0 {
                include(b, lerp(lerp(p0, c, t), lerp(c, p1, t), t));
            }
        }
    }
}

/// A cubic segment's end and its extremes on both axes.
fn cubic_bounds(b: &mut Option<[f64; 4]>, p0: (f64, f64), c0: (f64, f64), c1: (f64, f64), p1: (f64, f64)) {
    include(b, p1);
    let at = |t: f64| {
        let (a, bb, c) = (lerp(p0, c0, t), lerp(c0, c1, t), lerp(c1, p1, t));
        lerp(lerp(a, bb, t), lerp(bb, c, t), t)
    };
    for axis in 0..2 {
        let pick = |p: (f64, f64)| if axis == 0 { p.0 } else { p.1 };
        let (v0, v1, v2, v3) = (pick(p0), pick(c0), pick(c1), pick(p1));
        // The derivative over 3: a t² + b t + c.
        let a = -v0 + 3.0 * v1 - 3.0 * v2 + v3;
        let bq = 2.0 * (v0 - 2.0 * v1 + v2);
        let c = v1 - v0;
        let mut roots = Vec::with_capacity(2);
        if a.abs() < 1e-12 {
            if bq.abs() > 1e-12 {
                roots.push(-c / bq);
            }
        } else {
            let disc = bq * bq - 4.0 * a * c;
            if disc >= 0.0 {
                let s = disc.sqrt();
                roots.push((-bq + s) / (2.0 * a));
                roots.push((-bq - s) / (2.0 * a));
            }
        }
        for t in roots {
            if t > 0.0 && t < 1.0 {
                include(b, at(t));
            }
        }
    }
}

/// Adds the outline of `path` to `b`: lines, cubic curves, and quadratic splines with their
/// implied on-curve points.
fn path_bounds(path: &Path, b: &mut Option<[f64; 4]>) {
    let nodes = &path.nodes;
    let n = nodes.len();
    let at = |i: usize| nodes.get(i % n.max(1)).map(|nd| ((nd.x, nd.y), nd.kind));
    let Some(start) = nodes.iter().position(|nd| nd.kind != NodeKind::OffCurve) else {
        // A closed quadratic contour of off-curve points only: every on-curve point is implied.
        for i in 0..n {
            let (Some((prev, _)), Some((cur, _)), Some((next, _))) = (at(i + n - 1), at(i), at(i + 1)) else { continue };
            quad_bounds(b, lerp(prev, cur, 0.5), cur, lerp(cur, next, 0.5));
        }
        return;
    };
    let Some((first, _)) = at(start) else { return };
    include(b, first);
    let steps = if path.closed { n } else { n.saturating_sub(1 + start) };
    let mut prev = first;
    let mut offs: Vec<(f64, f64)> = Vec::new();
    for k in 1..=steps {
        let Some((p, kind)) = at(start + k) else { continue };
        if kind == NodeKind::OffCurve {
            offs.push(p);
            continue;
        }
        match (offs.as_slice(), kind) {
            ([], _) => include(b, p),
            ([c0, c1], NodeKind::Curve) => cubic_bounds(b, prev, *c0, *c1, p),
            (list, _) => {
                let mut s = prev;
                for (j, c) in list.iter().enumerate() {
                    let e = list.get(j + 1).map_or(p, |next| lerp(*c, *next, 0.5));
                    quad_bounds(b, s, *c, e);
                    s = e;
                }
            }
        }
        prev = p;
        offs.clear();
    }
}

/// Removes the layer glyphs `used` names, except base glyphs, glyphs with code points and glyphs
/// a remaining glyph uses as a component; the recorded glyph order follows.
fn remove_layer_glyphs(font: &mut Font, names: &[String], used: &HashSet<u32>, bases: &HashSet<u32>) {
    let mut remove: HashSet<&str> = used
        .iter()
        .filter(|gid| !bases.contains(*gid))
        .filter_map(|gid| names.get(*gid as usize))
        .map(String::as_str)
        .filter(|n| font.glyph(n).is_some_and(|g| g.unicodes.is_empty()))
        .collect();
    if remove.is_empty() {
        return;
    }
    // Every glyph that stays keeps the glyphs its components use, and theirs in turn.
    let index: HashMap<&str, &Glyph> = font.glyphs.as_arcs().iter().map(|g| (g.name.as_str(), g.as_ref())).collect();
    let mut work: Vec<&Glyph> = font.glyphs.as_arcs().iter().filter(|g| !remove.contains(g.name.as_str())).map(|g| g.as_ref()).collect();
    while let Some(g) = work.pop() {
        for l in &g.layers {
            for s in &l.shapes {
                if let Shape::Component(c) = s
                    && remove.remove(c.glyph.as_str())
                    && let Some(kept) = index.get(c.glyph.as_str())
                {
                    work.push(kept);
                }
            }
        }
    }
    let remove: HashSet<String> = remove.into_iter().map(str::to_string).collect();
    font.glyphs.retain(|g| !remove.contains(&g.name));
    // The glyph order recorded at open time.
    let extra = Arc::make_mut(&mut font.extra);
    if let Some(Value::Array(params)) = extra.get(Family::Glyphs, "customParameters").cloned() {
        let mut params = params;
        for p in &mut params.items {
            if let Value::Dict(d) = p
                && d.get("name").and_then(Value::as_str) == Some("glyphOrder")
                && let Some(Value::Array(order)) = d.get("value").cloned()
            {
                let mut order = order;
                order.items.retain(|v| v.as_str().is_none_or(|n| !remove.contains(n)));
                d.insert("value", Value::Array(order));
            }
        }
        extra.insert(Family::Glyphs, "customParameters", Value::Array(params));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use fontcraft_compile::{Options, compile_ttf};

    use super::*;

    fn int_rows(v: &Value) -> Vec<Vec<Vec<i64>>> {
        let list = |v: &Value| v.as_array().map(|a| a.items.clone()).unwrap_or_default();
        list(v)
            .iter()
            .map(|p| list(p).iter().map(|c| list(c).iter().map(|n| n.as_num().map_or(-1, |n| n.as_f64() as i64)).collect()).collect())
            .collect()
    }

    fn palettes_value(rows: &[Vec<[i64; 4]>]) -> Value {
        let colour = |c: &[i64; 4]| {
            Value::Array(Array { items: c.iter().map(|v| Value::Number(Number::int(*v))).collect(), layout: Layout::Compact, ..Array::default() })
        };
        Value::Array(Array {
            items: rows.iter().map(|p| Value::Array(Array { items: p.iter().map(colour).collect(), ..Array::default() })).collect(),
            ..Array::default()
        })
    }

    fn palette_layer(master: &str, id: &str, entry: PaletteEntry, rect: [f64; 4]) -> Layer {
        let mut l = Layer::new(master, 600.0);
        l.id = id.into();
        l.info.attrs.color_palette = Some(entry);
        l.shapes.push(Shape::Path(Path::rect(rect[0], rect[1], rect[2], rect[3])));
        l
    }

    #[test]
    fn a_palette_font_opens_back_to_the_same_palettes_and_palette_layers() {
        let mut f = Font::new("Colour Back");
        let master = f.masters[0].id.clone();
        let palettes = vec![vec![[255, 0, 0, 255], [0, 0, 255, 128]], vec![[0, 255, 0, 255], [10, 20, 30, 40]]];
        Arc::make_mut(&mut f.info).params.set(COLOR_PALETTES, palettes_value(&palettes));
        {
            let mut a = f.glyph_mut("A").unwrap();
            a.layers[0].width = 600.0;
            a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 500.0, 500.0)));
            a.layers.push(palette_layer(&master, "a-back", PaletteEntry::Index(1), [0.0, 0.0, 500.0, 500.0]));
            a.layers.push(palette_layer(&master, "a-front", PaletteEntry::Foreground, [100.0, 100.0, 400.0, 300.0]));
        }
        {
            let mut b = f.glyph_mut("B").unwrap();
            b.layers[0].width = 600.0;
            b.layers.push(palette_layer(&master, "b0", PaletteEntry::Index(0), [50.0, 0.0, 250.0, 700.0]));
        }
        let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;
        let d = crate::decompile(&bytes, None).unwrap();
        let back = &d.font;

        assert_eq!(
            int_rows(back.info.params.get(COLOR_PALETTES).unwrap()),
            palettes.iter().map(|p| p.iter().map(|c| c.to_vec()).collect()).collect::<Vec<Vec<Vec<i64>>>>()
        );
        let colour = |name: &str| -> Vec<(Option<PaletteEntry>, [f64; 4], f64)> {
            let g = back.glyph(name).unwrap();
            g.layers.iter().filter(|l| !l.is_master_layer()).map(|l| (l.info.attrs.color_palette, group_bounds(&l.shapes), l.width)).collect()
        };
        assert_eq!(
            colour("A"),
            vec![
                (Some(PaletteEntry::Index(1)), [0.0, 0.0, 500.0, 500.0], 600.0),
                (Some(PaletteEntry::Foreground), [100.0, 100.0, 400.0, 300.0], 600.0),
            ]
        );
        assert_eq!(colour("B"), vec![(Some(PaletteEntry::Index(0)), [50.0, 0.0, 250.0, 700.0], 600.0)]);
        // The layer glyphs are gone, from the glyphs and from the recorded glyph order.
        assert!(
            back.glyphs.as_arcs().iter().all(|g| !g.name.contains(".color")),
            "{:?}",
            back.glyphs.as_arcs().iter().map(|g| &g.name).collect::<Vec<_>>()
        );
        let order = format!("{:?}", back.extra.get(Family::Glyphs, "customParameters"));
        assert!(!order.contains(".color"), "{order}");
        assert!(!d.warnings.iter().any(|w| w.contains("COLR") || w.contains("CPAL")), "{:?}", d.warnings);
        back.check().unwrap();
    }

    fn layer_font() -> (Font, Vec<String>) {
        let mut f = Font::new("Paints");
        let master = f.masters[0].id.clone();
        let mut lg = Glyph::new("base.color0");
        let mut l = Layer::new(master.as_str(), 0.0);
        l.shapes.push(Shape::Path(Path::rect(100.0, 0.0, 300.0, 200.0)));
        lg.layers.push(l);
        f.glyphs.push(lg);
        let mut base = Glyph::new("base");
        base.layers.push(Layer::new(master.as_str(), 700.0));
        f.glyphs.push(base);
        let names = f.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
        (f, names)
    }

    #[test]
    fn version_one_paints_become_a_full_colour_layer_with_gradients_in_bounding_box_fractions() {
        let (f, names) = layer_font();
        let lg = u32::try_from(names.iter().position(|n| n == "base.color0").unwrap()).unwrap();
        let stops = vec![Stop { offset: 0.0, index: 0, alpha: 1.0 }, Stop { offset: 1.0, index: FOREGROUND, alpha: 0.5 }];
        let mut nodes = 0;
        let mut fill = |fill: Fill| -> ShapeAttrs {
            let layers = color_layers(&f, "base", &names, &[Item { glyph: lg, fill }], &mut nodes);
            assert_eq!(layers.len(), 1);
            let l = &layers[0];
            assert!(l.info.attrs.full_color && l.info.attrs.color_palette.is_none());
            assert_eq!(l.width, 700.0);
            let Shape::Path(p) = &l.shapes[0] else { panic!("a path") };
            p.info.attrs.clone()
        };

        let linear = fill(Fill::Linear { p0: (100.0, 0.0), p1: (300.0, 200.0), stops: stops.clone(), extend: Extend::Reflect });
        let g = linear.gradient.unwrap();
        let GradientKind::Linear { start, end } = g.kind else { panic!("{:?}", g.kind) };
        assert_eq!((start.x, start.y, end.x, end.y), (0.0, 0.0, 1.0, 1.0));
        assert_eq!(g.extend, Extend::Reflect);
        assert_eq!(g.stops[1].color, Color::Palette { index: FOREGROUND, alpha: 0.5 });
        assert!(linear.fill_color.is_none());

        let radial = fill(Fill::Radial { c0: (200.0, 100.0), r0: 0.0, c1: (200.0, 100.0), r1: 100.0, stops: stops.clone(), extend: Extend::Pad });
        let GradientKind::Radial { start, end_radius, .. } = radial.gradient.unwrap().kind else { panic!("radial") };
        assert_eq!((start.x, start.y, end_radius), (0.5, 0.5, 0.5));

        let sweep = fill(Fill::Sweep { center: (100.0, 200.0), start: 30.0, end: 120.0, stops, extend: Extend::Pad });
        let GradientKind::Conic { center, start_angle, angle, .. } = sweep.gradient.unwrap().kind else { panic!("conic") };
        assert_eq!((center.x, center.y, start_angle, angle), (0.0, 1.0, 30.0, 90.0));

        // A translucent solid fill keeps its alpha in a full-colour layer.
        let solid = fill(Fill::Solid { index: 1, alpha: 0.25 });
        assert_eq!(solid.fill_color, Some(Color::Palette { index: 1, alpha: 0.25 }));

        // Opaque solid fills only: palette layers, one per layer glyph.
        let items =
            [Item { glyph: lg, fill: Fill::Solid { index: 1, alpha: 1.0 } }, Item { glyph: lg, fill: Fill::Solid { index: FOREGROUND, alpha: 1.0 } }];
        let layers = color_layers(&f, "base", &names, &items, &mut nodes);
        let entries: Vec<_> = layers.iter().map(|l| (l.info.attrs.color_palette, l.info.name.clone())).collect();
        assert_eq!(entries, vec![(Some(PaletteEntry::Index(1)), Some("Color 1".into())), (Some(PaletteEntry::Foreground), Some("Color *".into()))]);
        assert_ne!(layers[0].id, layers[1].id);
    }

    #[test]
    fn paints_without_a_layer_form_are_reported_not_guessed() {
        let solid = || Brush::Solid { palette_index: 0, alpha: 1.0 };
        // The clip box around a whole glyph is fine.
        let mut c = Collect::default();
        c.push_clip_box(BoundingBox { x_min: 0.0, y_min: 0.0, x_max: 10.0, y_max: 10.0 });
        c.fill_glyph(GlyphId::new(3), None, solid());
        c.pop_clip();
        assert_eq!((c.items.len(), c.lost), (1, None));
        // A blend, a nested clip or an unclipped fill is not a layer.
        let mut c = Collect::default();
        c.push_layer(CompositeMode::SrcOver);
        c.fill_glyph(GlyphId::new(3), None, solid());
        assert!(c.lost.is_some());
        let mut c = Collect::default();
        c.push_clip_glyph(GlyphId::new(2));
        c.fill(solid());
        assert!(c.lost.is_some());
    }

    #[test]
    fn path_bounds_include_curve_extremes() {
        let n = Node::new;
        let cubic = Path::new(
            true,
            vec![n(0.0, 0.0, NodeKind::Line), n(0.0, 100.0, NodeKind::OffCurve), n(100.0, 100.0, NodeKind::OffCurve), n(100.0, 0.0, NodeKind::Curve)],
        );
        assert_eq!(group_bounds(&[Shape::Path(cubic)]), [0.0, 0.0, 100.0, 75.0]);
        let quad = Path::new(true, vec![n(0.0, 0.0, NodeKind::Line), n(50.0, 100.0, NodeKind::OffCurve), n(100.0, 0.0, NodeKind::QCurve)]);
        assert_eq!(group_bounds(&[Shape::Path(quad)]), [0.0, 0.0, 100.0, 50.0]);
    }

    /// What FontCraft's COLR version 1 writer makes of a full-colour layer opens back to the same
    /// paints: gradient positions are fractions of the curve bounds of each layer glyph's outline
    /// on both sides.
    #[test]
    fn a_full_colour_layer_with_gradients_survives_compile_and_open() {
        let mut f = Font::new("Gradient Back");
        let master = f.masters[0].id.clone();
        Arc::make_mut(&mut f.info).params.set(COLOR_PALETTES, palettes_value(&[vec![[255, 0, 0, 255], [0, 0, 255, 255]]]));
        let linear = |start: (f64, f64), end: (f64, f64), first: u16| Gradient {
            kind: GradientKind::Linear { start: fontcraft_geom::Point::new(start.0, start.1), end: fontcraft_geom::Point::new(end.0, end.1) },
            stops: vec![ColorStop { offset: 0.0, color: color(first, 1.0) }, ColorStop { offset: 1.0, color: color(1 - first, 0.5) }],
            extend: Extend::Reflect,
        };
        let shaded = linear((0.25, 0.5), (0.75, 1.0), 0);
        // A curve whose top (75) lies below its control points (100).
        let n = Node::new;
        let curved = linear((0.0, 0.0), (1.0, 1.0), 1);
        let conic = Gradient {
            kind: GradientKind::Conic { center: fontcraft_geom::Point::new(0.5, 0.5), start_angle: 30.0, angle: 90.0, control_radius: 0.5 },
            stops: vec![ColorStop { offset: 0.0, color: color(0, 1.0) }, ColorStop { offset: 1.0, color: color(1, 0.5) }],
            extend: Extend::Pad,
        };
        let shapes = {
            let mut a = Path::rect(100.0, 100.0, 500.0, 400.0);
            a.info.attrs.gradient = Some(shaded.clone());
            let mut b = Path::rect(0.0, 0.0, 100.0, 100.0);
            b.info.attrs.fill_color = Some(color(1, 1.0));
            let mut c = Path::new(
                true,
                vec![
                    n(0.0, 0.0, NodeKind::Line),
                    n(0.0, 100.0, NodeKind::OffCurve),
                    n(100.0, 100.0, NodeKind::OffCurve),
                    n(100.0, 0.0, NodeKind::Curve),
                ],
            );
            c.info.attrs.gradient = Some(curved.clone());
            let mut d = Path::rect(200.0, 0.0, 400.0, 200.0);
            d.info.attrs.gradient = Some(conic.clone());
            vec![Shape::Path(a), Shape::Path(b), Shape::Path(c), Shape::Path(d)]
        };
        {
            let mut a = f.glyph_mut("A").unwrap();
            a.layers[0].width = 600.0;
            a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 500.0, 400.0)));
            let mut l = Layer::new(master.as_str(), 600.0);
            l.id = "a-colour".into();
            l.info.attrs.full_color = true;
            l.shapes = shapes;
            a.layers.push(l);
        }
        let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;
        let d = crate::decompile(&bytes, None).unwrap();
        let back = d.font.glyph("A").unwrap();
        let colour: Vec<&Layer> = back.layers.iter().filter(|l| !l.is_master_layer()).collect();
        assert_eq!(colour.len(), 1, "one full-colour layer");
        assert!(colour[0].info.attrs.full_color);
        let paths: Vec<(&ShapeAttrs, [f64; 4])> = colour[0]
            .shapes
            .iter()
            .map(|s| match s {
                Shape::Path(p) => (&p.info.attrs, group_bounds(std::slice::from_ref(s))),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(paths.len(), 4);
        assert_eq!(paths[0].1, [100.0, 100.0, 500.0, 400.0]);
        assert_eq!((paths[0].0.gradient.as_ref(), paths[0].0.fill_color), (Some(&shaded), None));
        assert_eq!((paths[1].0.gradient.as_ref(), paths[1].0.fill_color), (None, Some(color(1, 1.0))));
        let Some(Gradient { kind: GradientKind::Linear { start, end }, stops, extend }) = &paths[2].0.gradient else { panic!("{:?}", paths[2].0) };
        let near = |a: f64, b: f64| (a - b).abs() < 0.02;
        assert!(near(start.x, 0.0) && near(start.y, 0.0) && near(end.x, 1.0) && near(end.y, 1.0), "{start:?} {end:?}");
        assert_eq!((stops, *extend), (&curved.stops, Extend::Reflect));
        // The sweep keeps its counter-clockwise direction and its stop order.
        let Some(Gradient { kind: GradientKind::Conic { center, start_angle, angle, .. }, stops, extend }) = &paths[3].0.gradient else {
            panic!("{:?}", paths[3].0)
        };
        assert!(near(center.x, 0.5) && near(center.y, 0.5), "{center:?}");
        assert!((start_angle - 30.0).abs() < 0.05 && (angle - 90.0).abs() < 0.05, "{start_angle} {angle}");
        assert_eq!((stops, *extend), (&conic.stops, Extend::Pad));
    }
}
