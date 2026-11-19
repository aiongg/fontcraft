//! A colour glyph's paint graph turned into fills in font units: each fill is a leaf paint within the outlines of
//! the glyph nodes above it. References to other colour glyphs are followed as the compiled font draws them.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_engine::cmd::stroke::shapes_outline;
use fontcraft_engine::color::paint::{self, ColorLine, ColorRef, Paint, ShapeRef};
use fontcraft_engine::color::{MAX_PAINT_DEPTH, Rgba, palette};
use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_model::{Extend, Font, Glyph, Shape};

/// Most filled outlines one glyph is drawn with.
pub const MAX_DRAWABLES: usize = 8_192;

// ── content ─────────────────────────────────────────────────────────────────────────────────────

/// A colour stop line with colours resolved: offsets in order, straight RGBA.
#[derive(Clone, Debug, PartialEq)]
pub struct Stops {
    pub at: Vec<(f64, [f32; 4])>,
    pub extend: Extend,
}

/// What fills an outline, in font units of the glyph.
#[derive(Clone, Debug, PartialEq)]
pub enum Leaf {
    Solid([f32; 4]),
    /// `t = (p − p0) · v`: `v` is the gradient vector divided by its squared length.
    Linear {
        p0: Point,
        v: (f64, f64),
        stops: Stops,
    },
    Radial {
        c0: Point,
        r0: f64,
        c1: Point,
        r1: f64,
        stops: Stops,
    },
    Sweep {
        center: Point,
        start: f64,
        end: f64,
        stops: Stops,
    },
    /// `leaf` of a referenced colour glyph, read in that glyph's font units: `to_leaf` maps the
    /// drawn glyph's font units to them. Never nested: references compose their transforms.
    Moved {
        to_leaf: Affine,
        leaf: Box<Leaf>,
    },
}

/// One fill: `leaf` within every outline of `clips`.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawable {
    pub clips: Vec<BezPath>,
    pub leaf: Leaf,
}

/// A palette colour as straight RGBA from 0 to 1.
pub fn straight(c: Rgba) -> [f32; 4] {
    c.to_f64().map(|v| v as f32)
}

fn clamp01(v: f64) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 }
}

/// `c` resolved against palette `palette`; a row the palette lacks is drawn in the text colour.
fn resolve(c: ColorRef, palette: &[Rgba], text: [f32; 4]) -> [f32; 4] {
    let with = |[r, g, b, a]: [f32; 4], alpha: f64| [r, g, b, a * clamp01(alpha)];
    match c {
        ColorRef::Palette { index, alpha } => with(palette.get(usize::from(index)).map_or(text, |row| straight(*row)), alpha),
        ColorRef::Foreground { alpha } => with(text, alpha),
        ColorRef::Rgba(v) => v.map(clamp01),
    }
}

fn stops(line: &ColorLine, palette: &[Rgba], text: [f32; 4]) -> Option<Stops> {
    let mut at: Vec<(f64, [f32; 4])> =
        line.stops.iter().filter(|s| s.offset.is_finite()).map(|s| (s.offset, resolve(s.color, palette, text))).collect();
    at.sort_by(|a, b| a.0.total_cmp(&b.0));
    (!at.is_empty()).then_some(Stops { at, extend: line.extend })
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The fill a leaf paint gives; `None` for a node that is not a leaf or cannot be drawn.
pub fn leaf(paint: &Paint, palette: &[Rgba], text: [f32; 4]) -> Option<Leaf> {
    match paint {
        Paint::Solid(c) => Some(Leaf::Solid(resolve(*c, palette, text))),
        Paint::Linear { p0, p1, p2, line } => {
            let stops = stops(line, palette, text)?;
            if !(finite(*p0) && finite(*p1) && finite(*p2)) {
                return None;
            }
            // Colour is constant along lines parallel to p0–p2: p1 projected onto the normal of
            // p0–p2 through p0 gives the gradient vector, as COLR version 1 defines it.
            let normal = (-(p2.y - p0.y), p2.x - p0.x);
            let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
            let nn = normal.0 * normal.0 + normal.1 * normal.1;
            let (vx, vy) = if nn > 1e-12 {
                let k = (dx * normal.0 + dy * normal.1) / nn;
                (normal.0 * k, normal.1 * k)
            } else {
                (dx, dy)
            };
            let len2 = vx * vx + vy * vy;
            if len2 <= 1e-12 {
                return stops.at.last().map(|(_, c)| Leaf::Solid(*c));
            }
            Some(Leaf::Linear { p0: *p0, v: (vx / len2, vy / len2), stops })
        }
        Paint::Radial { c0, r0, c1, r1, line } => {
            let ok = finite(*c0) && finite(*c1) && r0.is_finite() && r1.is_finite();
            ok.then(|| stops(line, palette, text)).flatten().map(|stops| Leaf::Radial { c0: *c0, r0: r0.abs(), c1: *c1, r1: r1.abs(), stops })
        }
        Paint::Sweep { center, start, end, line } => {
            let ok = finite(*center) && start.is_finite() && end.is_finite();
            ok.then(|| stops(line, palette, text)).flatten().map(|stops| Leaf::Sweep { center: *center, start: *start, end: *end, stops })
        }
        Paint::Layers(_) | Paint::Glyph { .. } | Paint::ColrGlyph { .. } => None,
    }
}

pub fn concat(parts: Vec<BezPath>) -> BezPath {
    parts.into_iter().fold(BezPath::new(), |mut all, p| {
        all.extend(p.elements().iter().copied());
        all
    })
}

/// The outline of the shapes `shape` names, as the COLR writer draws its layer glyph: on a
/// full-colour layer the fill without strokes, or the stroke alone, expanded.
fn shape_outline(font: &Font, glyph: &Glyph, shape: &ShapeRef) -> Option<BezPath> {
    let layer = glyph.layers.iter().find(|l| l.id == shape.layer)?;
    let mut shapes: Vec<Shape> = shape.shapes.iter().filter_map(|&i| layer.shapes.get(i).cloned()).collect();
    if layer.info.attrs.full_color {
        if shape.stroke.is_some() {
            shapes.retain(|s| matches!(s, Shape::Path(_)));
        }
        for s in &mut shapes {
            if let Shape::Path(p) = s {
                if shape.stroke.is_some() {
                    p.info.attrs.fill = false;
                } else {
                    p.info.attrs.stroke_width = None;
                }
            }
        }
    } else if shape.stroke.is_some() {
        return None;
    }
    shapes_outline(font, &shapes, &layer.master).ok().map(concat)
}

/// Most paint nodes one glyph's drawing visits, references followed included, so that
/// references fanning out to other references cannot multiply the work without bound.
pub const MAX_PAINT_VISITS: usize = 65_536;

/// Turns a glyph's paint graph into fills, following references to other colour glyphs
/// ([`Paint::ColrGlyph`]) as the compiled font draws them.
struct Collect<'f> {
    font: &'f Font,
    paints: paint::MasterPaints<'f>,
    /// The paint graphs of the glyphs referenced so far with their [`paint::nesting`], walked
    /// once per glyph however often it is referenced; `None` for a glyph without one.
    graphs: HashMap<String, Option<(Arc<Paint>, usize)>>,
    palette: &'f [Rgba],
    text: [f32; 4],
    out: Vec<Drawable>,
    /// The glyphs being drawn: the drawn glyph, then each glyph a reference is following.
    glyphs: Vec<String>,
    visits: usize,
}

impl Collect<'_> {
    /// The fills of `paint` of `glyph`, bottom first, each within the outlines of the glyph
    /// nodes above it. `place` maps `glyph`'s font units to the drawn glyph's.
    fn collect(&mut self, glyph: &Glyph, paint: &Paint, clips: &mut Vec<BezPath>, place: Affine, depth: usize) {
        if depth > MAX_PAINT_DEPTH || self.out.len() >= MAX_DRAWABLES || self.visits >= MAX_PAINT_VISITS {
            return;
        }
        self.visits += 1;
        match paint {
            Paint::Layers(list) => {
                for p in list {
                    self.collect(glyph, p, clips, place, depth + 1);
                }
            }
            Paint::Glyph { shape, paint } => {
                let Some(outline) = shape_outline(self.font, glyph, shape) else { return };
                clips.push(placed(outline, place));
                match leaf(paint, self.palette, self.text) {
                    Some(l) => self.push(clips, l, place),
                    None => self.collect(glyph, paint, clips, place, depth + 1),
                }
                clips.pop();
            }
            Paint::ColrGlyph { glyph: name, transform, shape } => {
                let font = self.font;
                let to = place * *transform;
                let target = self.graph(name, depth).filter(|_| invertible(to)).zip(font.glyph(name));
                if let Some((graph, target)) = target {
                    self.glyphs.push(name.clone());
                    self.collect(target, &graph, clips, to, depth + 1);
                    self.glyphs.pop();
                    return;
                }
                let Some(outline) = shape_outline(self.font, glyph, shape) else { return };
                clips.push(placed(outline, place));
                if let Some(l) = leaf(&paint::outline_fill(), self.palette, self.text) {
                    self.push(clips, l, place);
                }
                clips.pop();
            }
            other => {
                if !clips.is_empty()
                    && let Some(l) = leaf(other, self.palette, self.text)
                {
                    self.push(clips, l, place);
                }
            }
        }
    }

    fn push(&mut self, clips: &[BezPath], leaf: Leaf, place: Affine) {
        let leaf = if place == Affine::IDENTITY { leaf } else { Leaf::Moved { to_leaf: place.inverse(), leaf: Box::new(leaf) } };
        self.out.push(Drawable { clips: clips.to_vec(), leaf });
    }

    /// The paint graph a reference to `name` at `depth` draws, or `None` when it is drawn as
    /// its component's outline, as the compile falls back: the glyph has no graph, is already
    /// being drawn (a cycle), references nest more than [`MAX_PAINT_DEPTH`] deep, or its graph
    /// would nest past [`MAX_PAINT_DEPTH`].
    fn graph(&mut self, name: &str, depth: usize) -> Option<Arc<Paint>> {
        if self.glyphs.len() > MAX_PAINT_DEPTH || self.glyphs.iter().any(|g| g == name) {
            return None;
        }
        let (graph, nesting) = match self.graphs.get(name) {
            Some(known) => known.clone(),
            None => {
                let font = self.font;
                let made = font.glyph(name).and_then(|g| self.paints.paint(g, &mut Vec::new()).ok().flatten()).map(|graph| {
                    let nesting = paint::nesting(&graph);
                    (Arc::new(graph), nesting)
                });
                self.graphs.insert(name.to_string(), made.clone());
                made
            }
        }?;
        ((depth + 1).saturating_add(nesting) <= MAX_PAINT_DEPTH).then_some(graph)
    }
}

/// `outline` moved by `place`.
fn placed(mut outline: BezPath, place: Affine) -> BezPath {
    if place != Affine::IDENTITY {
        outline.apply_affine(place);
    }
    outline
}

/// Whether `a` maps the plane onto itself with finite numbers, so its inverse exists.
fn invertible(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite()) && a.determinant().abs() > 1e-12
}

/// The fills of `glyph`'s colour layers in `master`, palette colours from palette 0 of the master's palettes (the
/// font's when the master has none); `text` is the text colour (`*`).
pub fn vector(font: &Font, glyph: &Glyph, master: &str, text: [f32; 4]) -> Option<Vec<Drawable>> {
    let palettes = palette::read(font, Some(master)).unwrap_or_default();
    let palette = palettes.palette(0).map(<[Rgba]>::to_vec).unwrap_or_default();
    let mut paints = paint::MasterPaints::new(font, master);
    let graph = paints.paint(glyph, &mut Vec::new()).ok()??;
    let mut c =
        Collect { font, paints, graphs: HashMap::new(), palette: &palette, text, out: Vec::new(), glyphs: vec![glyph.name.clone()], visits: 0 };
    c.collect(glyph, &graph, &mut Vec::new(), Affine::IDENTITY, 0);
    Some(c.out)
}
