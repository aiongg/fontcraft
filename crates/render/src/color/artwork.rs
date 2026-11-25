//! Glyph artwork from the source font: a glyph's vector colour layers drawn through the paint
//! graph, with the image-only effects of full-colour paths. Rows: `color.fullColor.imageOnlyEffects`,
//! `glyph.exportArtwork`, `filter.glyphAsImage`.
//!
//! [`collect`] turns a glyph's paint graph ([`fontcraft_color::paint`]) into [`Item`]s in font
//! units, bottom first; [`rasterize`] draws them into an image. References to other colour
//! glyphs are followed as the compiled font draws them. The caller supplies the outline code
//! (strokes expanded, components resolved, masks applied) and reads pattern image files, so this
//! crate needs neither path operations nor file access.
//!
//! The image-only effects of a path on a full-colour layer, which no font table can hold:
//!
//! | Effect | Drawn |
//! |---|---|
//! | Shadow | the path's silhouette (its fill and its stroke) moved by the offset, blurred, in the shadow colour, just below the path |
//! | Pattern Image | the image tiled from the glyph origin, one image pixel per font unit (a PDF page by its points), over the path's fill |
//! | Inner Shadow | inside the path: the outside of the path moved by the offset, blurred, in the shadow colour, over the fill and the pattern and below the stroke |
//!
//! The blur is a Gaussian whose standard deviation is half the blur value. A path that is only
//! stroked takes its pattern and inner shadow on the stroke. Effects of paths inside a
//! referenced colour glyph are not drawn.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_color::paint::{self, ColorLine, ColorRef, MasterPaints, Paint, ShapeRef};
use fontcraft_color::palette::{self, FOREGROUND_INDEX};
use fontcraft_color::{MAX_PAINT_DEPTH, Rgba};
use fontcraft_geom::{Affine, BezPath, Point, Rect, Shape as _, Vec2};
use fontcraft_imagery::Rgba8;
use fontcraft_model::{Color, Extend, Font, Glyph, Layer, Shadow, Shape, ShapeAttrs};

use super::filter::blur;
use crate::{Image, RenderError, Style, check_size, render_path};

/// Most fills one glyph is drawn with.
pub const MAX_DRAWABLES: usize = 8_192;
/// Most paint nodes one glyph's drawing visits, references followed included.
pub const MAX_PAINT_VISITS: usize = 65_536;
/// Most pixel operations one [`rasterize`] may take (a piece's pixel box times its passes,
/// summed over the pieces).
pub const MAX_ARTWORK_WORK: u64 = 1 << 32;
/// Most pixels a shadow is computed over, its blur margin included; a larger one is left out.
pub const MAX_EFFECT_PIXELS: u64 = 1 << 24;
/// Largest shadow blur and offset, in font units (as `attributes.addAttribute` accepts them).
pub const MAX_EFFECT_UNITS: f64 = 10_000.0;

/// A colour stop line with colours resolved: offsets in order, straight RGBA.
#[derive(Clone, Debug, PartialEq)]
pub struct Stops {
    pub at: Vec<(f64, [f32; 4])>,
    pub extend: Extend,
}

/// What fills an outline, in font units of the drawn glyph. Colours are straight RGBA.
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

impl Leaf {
    /// The colour when the leaf is one colour everywhere.
    pub fn solid(&self) -> Option<[f32; 4]> {
        match self {
            Leaf::Solid(c) => Some(*c),
            Leaf::Moved { leaf, .. } => match leaf.as_ref() {
                Leaf::Solid(c) => Some(*c),
                _ => None,
            },
            _ => None,
        }
    }
}

/// One fill: `leaf` within every outline of `clips`.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawable {
    pub clips: Vec<BezPath>,
    pub leaf: Leaf,
}

/// A shadow or inner shadow, in font units, its colour resolved (straight RGBA).
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub blur: f64,
    pub offset: (f64, f64),
    pub color: [f32; 4],
}

/// A decoded pattern image: `unit` font units per image pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub image: Arc<Rgba8>,
    pub unit: f64,
}

/// One piece of a glyph's artwork, in font units.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Fill(Drawable),
    /// A drop shadow cast by the union of `outlines`.
    Shadow {
        outlines: Vec<BezPath>,
        effect: Effect,
    },
    InnerShadow {
        outline: BezPath,
        effect: Effect,
    },
    Pattern {
        outline: BezPath,
        pattern: Pattern,
    },
}

impl Item {
    /// The area the piece may paint, in font units; `None` when it paints nothing.
    pub fn bounds(&self) -> Option<Rect> {
        let of = |p: &BezPath| (!p.elements().is_empty()).then(|| p.bounding_box());
        match self {
            Item::Fill(d) => d.clips.first().and_then(of),
            Item::Shadow { outlines, effect } => {
                let all = outlines.iter().filter_map(of).reduce(|a, b| a.union(b))?;
                let reach = reach(effect.blur);
                Some((all + Vec2::new(effect.offset.0, effect.offset.1)).inflate(reach, reach))
            }
            Item::InnerShadow { outline, .. } | Item::Pattern { outline, .. } => of(outline),
        }
    }
}

/// The area `items` may paint, in font units.
pub fn bounds(items: &[Item]) -> Option<Rect> {
    items.iter().filter_map(Item::bounds).filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())).reduce(|a, b| a.union(b))
}

/// How far a blur of `blur` font units reaches: three standard deviations.
fn reach(blur: f64) -> f64 {
    blur.max(0.0) * 1.5
}

/// Makes the outline of some shapes on a master.
pub type OutlineOf<'a> = dyn Fn(&Font, &[Shape], &str) -> Option<BezPath> + 'a;

/// Where outlines and pattern images come from.
pub struct Sources<'a> {
    /// The outline of `shapes` on master `master` as the canvas draws them (strokes expanded,
    /// components resolved, masks applied); `None` when it cannot be made.
    pub outline: &'a OutlineOf<'a>,
    /// The pattern image a path's attributes name, decoded; `None` when they name none or it
    /// cannot be read.
    pub pattern: &'a mut dyn FnMut(&ShapeAttrs) -> Option<Pattern>,
    /// The text colour (palette index 65,535), straight RGBA.
    pub text: [f32; 4],
    /// The palette the colours come from, from 0; one the font lacks gives the text colour.
    pub palette: usize,
}

// ── the paint graph ─────────────────────────────────────────────────────────────────────────────

fn clamp01(v: f64) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 }
}

fn straight(c: Rgba) -> [f32; 4] {
    c.to_f64().map(|v| v as f32)
}

/// `c` resolved against `palette`; a row the palette lacks is drawn in the text colour.
fn resolve(c: ColorRef, palette: &[Rgba], text: [f32; 4]) -> [f32; 4] {
    let with = |[r, g, b, a]: [f32; 4], alpha: f64| [r, g, b, a * clamp01(alpha)];
    match c {
        ColorRef::Palette { index, alpha } => with(palette.get(usize::from(index)).map_or(text, |row| straight(*row)), alpha),
        ColorRef::Foreground { alpha } => with(text, alpha),
        ColorRef::Rgba(v) => v.map(clamp01),
    }
}

fn model_color(c: &Color, palette: &[Rgba], text: [f32; 4]) -> [f32; 4] {
    match *c {
        Color::Rgba(v) => v.map(clamp01),
        Color::Palette { index, alpha } if index == FOREGROUND_INDEX => resolve(ColorRef::Foreground { alpha }, palette, text),
        Color::Palette { index, alpha } => resolve(ColorRef::Palette { index, alpha }, palette, text),
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

/// The fill a leaf paint gives; `None` for a node that is not a leaf or cannot be drawn. Palette
/// rows `palette` lacks, and the foreground, take `text`.
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

/// The shapes of `layer` at `indices` as the COLR writer draws its layer glyph: on a
/// full-colour layer the fill without strokes, or the stroke alone (`stroke`). `None` for a
/// stroke on any other layer.
fn drawn_shapes(layer: &Layer, indices: &[usize], stroke: bool) -> Option<Vec<Shape>> {
    let mut shapes: Vec<Shape> = indices.iter().filter_map(|&i| layer.shapes.get(i).cloned()).collect();
    if layer.info.attrs.full_color {
        if stroke {
            shapes.retain(|s| matches!(s, Shape::Path(_)));
        }
        for s in &mut shapes {
            if let Shape::Path(p) = s {
                if stroke {
                    p.info.attrs.fill = false;
                } else {
                    p.info.attrs.stroke_width = None;
                }
            }
        }
    } else if stroke {
        return None;
    }
    Some(shapes)
}

type Outline<'a> = &'a dyn Fn(&Font, &[Shape], &str) -> Option<BezPath>;

fn outline_of(font: &Font, outline: Outline, layer: &Layer, indices: &[usize], stroke: bool) -> Option<BezPath> {
    let shapes = drawn_shapes(layer, indices, stroke)?;
    outline(font, &shapes, &layer.master).filter(|p| !p.elements().is_empty())
}

/// Turns a glyph's paint graph into fills, following references to other colour glyphs.
struct Collect<'f> {
    font: &'f Font,
    outline: Outline<'f>,
    paints: MasterPaints<'f>,
    /// The paint graphs of the glyphs referenced so far with their [`paint::nesting`]; `None`
    /// for a glyph without one.
    graphs: HashMap<String, Option<(Arc<Paint>, usize)>>,
    palette: &'f [Rgba],
    text: [f32; 4],
    /// The fills, each with the shapes of the drawn glyph itself it fills (`None` within a
    /// referenced glyph).
    out: Vec<(Drawable, Option<ShapeRef>)>,
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
                let Some(outline) = self.shape_outline(glyph, shape) else { return };
                clips.push(placed(outline, place));
                match leaf(paint, self.palette, self.text) {
                    Some(l) => {
                        let source = (self.glyphs.len() == 1).then(|| shape.clone());
                        self.push(clips, l, place, source);
                    }
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
                let Some(outline) = self.shape_outline(glyph, shape) else { return };
                clips.push(placed(outline, place));
                if let Some(l) = leaf(&paint::outline_fill(), self.palette, self.text) {
                    self.push(clips, l, place, None);
                }
                clips.pop();
            }
            other => {
                if !clips.is_empty()
                    && let Some(l) = leaf(other, self.palette, self.text)
                {
                    self.push(clips, l, place, None);
                }
            }
        }
    }

    fn shape_outline(&self, glyph: &Glyph, shape: &ShapeRef) -> Option<BezPath> {
        let layer = glyph.layers.iter().find(|l| l.id == shape.layer)?;
        outline_of(self.font, self.outline, layer, &shape.shapes, shape.stroke.is_some())
    }

    fn push(&mut self, clips: &[BezPath], leaf: Leaf, place: Affine, source: Option<ShapeRef>) {
        let leaf = if place == Affine::IDENTITY { leaf } else { Leaf::Moved { to_leaf: place.inverse(), leaf: Box::new(leaf) } };
        self.out.push((Drawable { clips: clips.to_vec(), leaf }, source));
    }

    /// The paint graph a reference to `name` at `depth` draws, or `None` when it is drawn as
    /// its component's outline, as the compile falls back: the glyph has no graph, is already
    /// being drawn (a cycle), or references or its graph would nest past [`MAX_PAINT_DEPTH`].
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

/// Palette `index` of `master`'s palettes (the font's when the master has none).
fn palette_at(font: &Font, master: &str, index: usize) -> Vec<Rgba> {
    palette::read(font, Some(master)).ok().and_then(|p| p.palette(index).map(<[Rgba]>::to_vec)).unwrap_or_default()
}

fn effect(shadow: &Shadow, palette: &[Rgba], text: [f32; 4]) -> Option<Effect> {
    let [x, y] = shadow.offset;
    if !(shadow.blur.is_finite() && x.is_finite() && y.is_finite()) {
        return None;
    }
    let clamp = |v: f64| v.clamp(-MAX_EFFECT_UNITS, MAX_EFFECT_UNITS);
    let color = model_color(&shadow.color, palette, text);
    (color[3] > 0.0).then_some(Effect { blur: shadow.blur.clamp(0.0, MAX_EFFECT_UNITS), offset: (clamp(x), clamp(y)), color })
}

/// The artwork of `glyph` in `master`: its vector colour layers (palette and full-colour)
/// through the paint graph, with the image-only effects of its full-colour paths. `None` when
/// the glyph has no vector colour layers in the master.
pub fn collect(font: &Font, glyph: &Glyph, master: &str, src: &mut Sources) -> Option<Vec<Item>> {
    let palette = palette_at(font, master, src.palette);
    let mut paints = MasterPaints::new(font, master);
    let graph = paints.paint(glyph, &mut Vec::new()).ok()??;
    let mut c = Collect {
        font,
        outline: src.outline,
        paints,
        graphs: HashMap::new(),
        palette: &palette,
        text: src.text,
        out: Vec::new(),
        glyphs: vec![glyph.name.clone()],
        visits: 0,
    };
    c.collect(glyph, &graph, &mut Vec::new(), Affine::IDENTITY, 0);
    let drawn = c.out;
    Some(with_effects(font, glyph, drawn, &palette, src))
}

/// `drawn` as items, with each full-colour path's shadow just below its first fill or stroke,
/// and its pattern and inner shadow just above its fill (its stroke when it is only stroked).
fn with_effects(font: &Font, glyph: &Glyph, drawn: Vec<(Drawable, Option<ShapeRef>)>, palette: &[Rgba], src: &mut Sources) -> Vec<Item> {
    // Whether each shape is filled and whether it is stroked.
    let mut painted: HashMap<(&str, usize), (bool, bool)> = HashMap::new();
    for s in drawn.iter().filter_map(|(_, s)| s.as_ref()) {
        for &i in &s.shapes {
            let e = painted.entry((s.layer.as_str(), i)).or_default();
            if s.stroke.is_some() {
                e.1 = true;
            } else {
                e.0 = true;
            }
        }
    }
    let painted: HashMap<(String, usize), (bool, bool)> = painted.into_iter().map(|((l, i), v)| ((l.to_string(), i), v)).collect();
    let mut shadowed: HashSet<(String, usize)> = HashSet::new();
    let mut items = Vec::with_capacity(drawn.len());
    for (d, source) in drawn {
        let Some(s) = source else {
            items.push(Item::Fill(d));
            continue;
        };
        let Some(layer) = glyph.layers.iter().find(|l| l.id == s.layer).filter(|l| l.info.attrs.full_color) else {
            items.push(Item::Fill(d));
            continue;
        };
        let stroke = s.stroke.is_some();
        let paths: Vec<(usize, &ShapeAttrs)> = s
            .shapes
            .iter()
            .filter_map(|&i| match layer.shapes.get(i) {
                Some(Shape::Path(p)) => Some((i, &p.info.attrs)),
                _ => None,
            })
            .collect();
        for &(i, attrs) in &paths {
            let Some(shadow) = &attrs.shadow else { continue };
            if !shadowed.insert((s.layer.clone(), i)) {
                continue;
            }
            let Some(effect) = effect(shadow, palette, src.text) else { continue };
            let (filled, stroked) = painted.get(&(s.layer.clone(), i)).copied().unwrap_or_default();
            let outlines: Vec<BezPath> = [(filled, false), (stroked, true)]
                .into_iter()
                .filter(|(on, _)| *on)
                .filter_map(|(_, as_stroke)| outline_of(font, src.outline, layer, &[i], as_stroke))
                .collect();
            if !outlines.is_empty() {
                items.push(Item::Shadow { outlines, effect });
            }
        }
        items.push(Item::Fill(d));
        for &(i, attrs) in &paths {
            let (filled, _) = painted.get(&(s.layer.clone(), i)).copied().unwrap_or_default();
            // On the fill, or on the stroke of a path that is not filled.
            if stroke == filled {
                continue;
            }
            let inner = attrs.shadow_in.as_ref().and_then(|e| effect(e, palette, src.text));
            let pattern = (src.pattern)(attrs);
            if inner.is_none() && pattern.is_none() {
                continue;
            }
            let Some(outline) = outline_of(font, src.outline, layer, &[i], stroke) else { continue };
            if let Some(pattern) = pattern {
                items.push(Item::Pattern { outline: outline.clone(), pattern });
            }
            if let Some(effect) = inner {
                items.push(Item::InnerShadow { outline, effect });
            }
        }
    }
    items
}

// ── rasterising ─────────────────────────────────────────────────────────────────────────────────

const MASK: Style = Style { ink: [255, 255, 255, 255], background: [0, 0, 0, 0], anti_alias: true };

/// Premultiplied RGBA pixels, rows top to bottom.
struct Px<'a> {
    data: &'a mut [u8],
    width: u32,
    height: u32,
}

impl Px<'_> {
    /// Lays `color` (straight) at `coverage` over the pixel at `(x, y)`.
    fn blend(&mut self, x: i64, y: i64, color: [f32; 4], coverage: f32) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let [r, g, b, a] = color;
        let alpha = (a * coverage).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        let Some(px) = self.data.get_mut(at..at + 4) else { return };
        let keep = 1.0 - alpha;
        for (d, s) in px.iter_mut().zip([r * alpha, g * alpha, b * alpha, alpha]) {
            let v = s * 255.0 + f32::from(*d) * keep;
            *d = v.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// `t` within 0 to 1 as the extend mode says.
fn extend(t: f64, mode: Extend) -> f64 {
    match mode {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => t - t.floor(),
        Extend::Reflect => {
            let m = t.rem_euclid(2.0);
            if m > 1.0 { 2.0 - m } else { m }
        }
    }
}

fn sample(stops: &Stops, t: f64) -> [f32; 4] {
    let t = if t.is_finite() { extend(t, stops.extend) } else { 0.0 };
    let (Some(first), Some(last)) = (stops.at.first(), stops.at.last()) else { return [0.0; 4] };
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = stops.at.partition_point(|(o, _)| *o <= t);
    match (stops.at.get(i.wrapping_sub(1)), stops.at.get(i)) {
        (Some((o0, c0)), Some((o1, c1))) if o1 > o0 => {
            let k = ((t - o0) / (o1 - o0)) as f32;
            let mut out = [0.0; 4];
            for ((o, a), b) in out.iter_mut().zip(c0).zip(c1) {
                *o = a + (b - a) * k;
            }
            out
        }
        (Some((_, c)), _) | (_, Some((_, c))) => *c,
        _ => last.1,
    }
}

/// The colour of `leaf` at font point `p`; `None` where a radial gradient paints nothing.
fn eval(leaf: &Leaf, p: Point) -> Option<[f32; 4]> {
    match leaf {
        Leaf::Solid(c) => Some(*c),
        Leaf::Linear { p0, v, stops } => Some(sample(stops, (p.x - p0.x) * v.0 + (p.y - p0.y) * v.1)),
        Leaf::Radial { c0, r0, c1, r1, stops } => {
            // The largest t with |p − c(t)| = r(t) and r(t) ≥ 0, c and r moving from the start
            // circle to the end circle.
            let (cdx, cdy, dr) = (c1.x - c0.x, c1.y - c0.y, r1 - r0);
            let (pdx, pdy) = (p.x - c0.x, p.y - c0.y);
            let a = cdx * cdx + cdy * cdy - dr * dr;
            let b = pdx * cdx + pdy * cdy + r0 * dr;
            let c = pdx * pdx + pdy * pdy - r0 * r0;
            let r = |t: f64| r0 + t * dr;
            let t = if a.abs() < 1e-9 {
                (b.abs() > 1e-12).then(|| c / (2.0 * b)).filter(|t| r(*t) >= 0.0)
            } else {
                let disc = b * b - a * c;
                if disc < 0.0 {
                    None
                } else {
                    let s = disc.sqrt();
                    let (t1, t2) = ((b + s) / a, (b - s) / a);
                    let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
                    [hi, lo].into_iter().find(|t| r(*t) >= 0.0)
                }
            }?;
            Some(sample(stops, t))
        }
        Leaf::Sweep { center, start, end, stops } => {
            let span = end - start;
            if span.abs() < 1e-9 {
                return stops.at.last().map(|(_, c)| *c);
            }
            let angle = (p.y - center.y).atan2(p.x - center.x).to_degrees().rem_euclid(360.0);
            Some(sample(stops, (angle - start) / span))
        }
        Leaf::Moved { to_leaf, leaf } => match leaf.as_ref() {
            Leaf::Moved { .. } => None,
            inner => eval(inner, *to_leaf * p),
        },
    }
}

/// A pixel box: left, top, right, bottom; may reach outside the image.
type PixBox = (i64, i64, i64, i64);

/// The whole pixels `rect` (in pixels) touches, within `limit`; `None` when that is empty or
/// not finite.
fn pix_box(rect: Rect, limit: Rect) -> Option<PixBox> {
    let r = rect.intersect(limit);
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let b = (r.x0.floor() as i64, r.y0.floor() as i64, r.x1.ceil() as i64, r.y1.ceil() as i64);
    (b.2 > b.0 && b.3 > b.1).then_some(b)
}

fn area((x0, y0, x1, y1): PixBox) -> u64 {
    u64::try_from(x1.saturating_sub(x0)).unwrap_or(0).saturating_mul(u64::try_from(y1.saturating_sub(y0)).unwrap_or(0))
}

/// The coverage of `outline` mapped by `to_pixels` over the box `b`, row by row; `None` when the
/// box is too large to render.
fn coverage(outline: &BezPath, to_pixels: Affine, b: PixBox) -> Option<Vec<f32>> {
    let (w, h) = (u32::try_from(b.2 - b.0).ok()?, u32::try_from(b.3 - b.1).ok()?);
    let shift = Affine::translate((-(b.0 as f64), -(b.1 as f64))) * to_pixels;
    let mask = render_path(outline, shift, w, h, &MASK).ok()?;
    Some(mask.pixels.as_chunks::<4>().0.iter().map(|p| f32::from(p[3]) / 255.0).collect())
}

/// What one rasterisation draws over: the image, and how its font units map to it.
struct Canvas {
    to_pixels: Affine,
    inverse: Affine,
    /// Pixels per font unit.
    scale: f64,
    image: Rect,
}

impl Canvas {
    /// The pixel box an effect is computed over: `rect` (font units) inflated by `margin`
    /// pixels, within the image inflated by the same margin.
    fn effect_box(&self, rect: Rect, margin: (f64, f64)) -> Option<PixBox> {
        let r = self.to_pixels.transform_rect_bbox(rect).inflate(margin.0, margin.1);
        let b = pix_box(r, self.image.inflate(margin.0, margin.1))?;
        (area(b) <= MAX_EFFECT_PIXELS).then_some(b)
    }

    /// `offset` (font units) in pixels.
    fn offset(&self, (x, y): (f64, f64)) -> Vec2 {
        self.to_pixels * Point::new(x, y) - self.to_pixels * Point::ZERO
    }

    /// Pixels of the blur's standard deviation, and the margin it needs.
    fn sigma(&self, blur: f64) -> (f64, f64) {
        let sigma = blur.max(0.0) / 2.0 * self.scale;
        (sigma, (sigma * 3.0).ceil())
    }
}

/// The pixel operations `item` takes, roughly: its pixel box times its passes.
fn work(item: &Item, c: &Canvas) -> u64 {
    let Some(rect) = item.bounds() else { return 0 };
    let Some(b) = pix_box(c.to_pixels.transform_rect_bbox(rect), c.image) else { return 0 };
    let passes: u64 = match item {
        Item::Fill(d) => u64::try_from(d.clips.len()).unwrap_or(u64::MAX).saturating_add(1),
        Item::Shadow { outlines, .. } => u64::try_from(outlines.len()).unwrap_or(u64::MAX).saturating_add(8),
        Item::InnerShadow { .. } => 10,
        Item::Pattern { .. } => 2,
    };
    area(b).saturating_mul(passes)
}

/// Draws `items` into a new transparent `width × height` image; `to_pixels` maps font units to
/// pixels. Refused when the drawing would take more than [`MAX_ARTWORK_WORK`].
pub fn rasterize(items: &[Item], to_pixels: Affine, width: u32, height: u32) -> Result<Image, RenderError> {
    check_size(u64::from(width), u64::from(height))?;
    if !invertible(to_pixels) {
        return Err(RenderError::Model("the drawing transform cannot be inverted".into()));
    }
    let canvas = Canvas {
        to_pixels,
        inverse: to_pixels.inverse(),
        scale: to_pixels.determinant().abs().sqrt(),
        image: Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
    };
    let cost = items.iter().map(|i| work(i, &canvas)).fold(0u64, u64::saturating_add);
    if cost > MAX_ARTWORK_WORK {
        return Err(RenderError::Model("the picture needs more drawing work than allowed; choose fewer pixels per em".into()));
    }
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let mut px = Px { data: &mut data, width, height };
    for item in items {
        match item {
            Item::Fill(d) => paint_fill(&mut px, d, &canvas),
            Item::Shadow { outlines, effect } => paint_shadow(&mut px, outlines, effect, &canvas),
            Item::InnerShadow { outline, effect } => paint_inner_shadow(&mut px, outline, effect, &canvas),
            Item::Pattern { outline, pattern } => paint_pattern(&mut px, outline, pattern, &canvas),
        }
    }
    Ok(Image { width, height, pixels: data })
}

/// `leaf` painted over every pixel of a new `width × height` image (for a PDF, which clips it
/// to the outlines itself).
pub fn leaf_image(leaf: &Leaf, to_pixels: Affine, width: u32, height: u32) -> Result<Image, RenderError> {
    check_size(u64::from(width), u64::from(height))?;
    if !invertible(to_pixels) {
        return Err(RenderError::Model("the drawing transform cannot be inverted".into()));
    }
    let inverse = to_pixels.inverse();
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let mut px = Px { data: &mut data, width, height };
    for y in 0..i64::from(height) {
        for x in 0..i64::from(width) {
            if let Some(color) = eval(leaf, inverse * Point::new(x as f64 + 0.5, y as f64 + 0.5)) {
                px.blend(x, y, color, 1.0);
            }
        }
    }
    Ok(Image { width, height, pixels: data })
}

fn paint_fill(px: &mut Px, d: &Drawable, c: &Canvas) {
    let Some(first) = d.clips.first().filter(|f| !f.elements().is_empty()) else { return };
    let Some(b) = pix_box(c.to_pixels.transform_rect_bbox(first.bounding_box()), c.image) else { return };
    let w = b.2 - b.0;
    let mut cov = vec![1.0f32; usize::try_from(area(b)).unwrap_or(0)];
    for clip in &d.clips {
        let Some(mask) = coverage(clip, c.to_pixels, b) else { return };
        for (a, m) in cov.iter_mut().zip(mask) {
            *a *= m;
        }
    }
    for (i, a) in cov.iter().enumerate() {
        if *a <= 0.0 {
            continue;
        }
        let (x, y) = (b.0 + i as i64 % w, b.1 + i as i64 / w);
        let Some(color) = eval(&d.leaf, c.inverse * Point::new(x as f64 + 0.5, y as f64 + 0.5)) else { continue };
        px.blend(x, y, color, *a);
    }
}

/// `alpha` (one value per pixel of `b`) blurred by `sigma` pixels.
fn blurred(alpha: &[f32], b: PixBox, sigma: f64) -> Vec<f32> {
    let (w, h) = (usize::try_from(b.2 - b.0).unwrap_or(0), usize::try_from(b.3 - b.1).unwrap_or(0));
    let mut buf: Vec<u8> = alpha.iter().flat_map(|a| [0, 0, 0, (a * 255.0).round().clamp(0.0, 255.0) as u8]).collect();
    blur(&mut buf, w, h, (sigma, sigma));
    buf.as_chunks::<4>().0.iter().map(|p| f32::from(p[3]) / 255.0).collect()
}

fn paint_shadow(px: &mut Px, outlines: &[BezPath], e: &Effect, c: &Canvas) {
    let Some(rect) = outlines.iter().filter(|o| !o.elements().is_empty()).map(|o| o.bounding_box()).reduce(|a, b| a.union(b)) else {
        return;
    };
    let (sigma, margin) = c.sigma(e.blur);
    let moved = Affine::translate(c.offset(e.offset)) * c.to_pixels;
    let r = moved.transform_rect_bbox(rect).inflate(margin, margin);
    let Some(b) = pix_box(r, c.image.inflate(margin, margin)).filter(|b| area(*b) <= MAX_EFFECT_PIXELS) else { return };
    let mut alpha = vec![0.0f32; usize::try_from(area(b)).unwrap_or(0)];
    for o in outlines {
        let Some(mask) = coverage(o, moved, b) else { continue };
        for (a, m) in alpha.iter_mut().zip(mask) {
            *a = a.max(m);
        }
    }
    let alpha = blurred(&alpha, b, sigma);
    let w = b.2 - b.0;
    for (i, a) in alpha.iter().enumerate() {
        if *a > 0.0 {
            px.blend(b.0 + i as i64 % w, b.1 + i as i64 / w, e.color, *a);
        }
    }
}

fn paint_inner_shadow(px: &mut Px, outline: &BezPath, e: &Effect, c: &Canvas) {
    if outline.elements().is_empty() {
        return;
    }
    let (sigma, margin) = c.sigma(e.blur);
    let o = c.offset(e.offset);
    // The outside of the moved outline is clear beyond the box: the box reaches far enough past
    // the outline that the blur cannot carry that into it.
    let Some(b) = c.effect_box(outline.bounding_box(), (margin + o.x.abs(), margin + o.y.abs())) else { return };
    let (Some(inside), Some(moved)) = (coverage(outline, c.to_pixels, b), coverage(outline, Affine::translate(o) * c.to_pixels, b)) else {
        return;
    };
    let outside: Vec<f32> = moved.iter().map(|m| 1.0 - m).collect();
    let shade = blurred(&outside, b, sigma);
    let w = b.2 - b.0;
    for (i, (m, s)) in inside.iter().zip(shade).enumerate() {
        let a = m * s;
        if a > 0.0 {
            px.blend(b.0 + i as i64 % w, b.1 + i as i64 / w, e.color, a);
        }
    }
}

fn paint_pattern(px: &mut Px, outline: &BezPath, pattern: &Pattern, c: &Canvas) {
    let image = &pattern.image;
    let (iw, ih) = (u64::from(image.width), u64::from(image.height));
    let unit = pattern.unit;
    if outline.elements().is_empty()
        || iw == 0
        || ih == 0
        || !(unit.is_finite() && unit > 0.0)
        || (image.pixels.len() as u64) < iw.saturating_mul(ih).saturating_mul(4)
    {
        return;
    }
    let Some(b) = pix_box(c.to_pixels.transform_rect_bbox(outline.bounding_box()), c.image) else { return };
    let Some(cov) = coverage(outline, c.to_pixels, b) else { return };
    let w = b.2 - b.0;
    for (i, a) in cov.iter().enumerate() {
        if *a <= 0.0 {
            continue;
        }
        let (x, y) = (b.0 + i as i64 % w, b.1 + i as i64 / w);
        let p = c.inverse * Point::new(x as f64 + 0.5, y as f64 + 0.5);
        let (u, v) = ((p.x / unit).floor(), (p.y / unit).floor());
        if !(u.is_finite() && v.is_finite()) {
            continue;
        }
        // The image's bottom row sits on the baseline; rows are stored top first.
        let col = u.rem_euclid(iw as f64) as u64;
        let row = ih - 1 - (v.rem_euclid(ih as f64) as u64).min(ih - 1);
        let at = usize::try_from((row * iw + col.min(iw - 1)) * 4).unwrap_or(usize::MAX);
        let Some(&[r, g, bl, al]) = image.pixels.get(at..at.saturating_add(4)).and_then(|s| <&[u8; 4]>::try_from(s).ok()) else { continue };
        px.blend(x, y, [r, g, bl, al].map(|v| f32::from(v) / 255.0), *a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x0, y0));
        p.line_to((x1, y0));
        p.line_to((x1, y1));
        p.line_to((x0, y1));
        p.close_path();
        p
    }

    /// One pixel per unit, y up, 100 units high.
    fn flip() -> Affine {
        Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 100.0])
    }

    fn alpha(img: &Image, x: u32, y: u32) -> u8 {
        img.pixel(x, y).map_or(0, |p| p[3])
    }

    #[test]
    fn a_shadow_lies_below_and_offset_and_an_inner_shadow_stays_inside() {
        let fill = Item::Fill(Drawable { clips: vec![square(20.0, 40.0, 60.0, 80.0)], leaf: Leaf::Solid([0.0, 0.0, 1.0, 1.0]) });
        let effect = Effect { blur: 0.0, offset: (20.0, -20.0), color: [0.0, 0.0, 0.0, 1.0] };
        let shadow = Item::Shadow { outlines: vec![square(20.0, 40.0, 60.0, 80.0)], effect: effect.clone() };
        let img = rasterize(&[shadow, fill.clone()], flip(), 100, 100).unwrap();
        // Units (70, 30) lie in the shadow only: pixel (70, 70).
        assert_eq!(img.pixel(70, 70), Some([0, 0, 0, 255]));
        // The fill covers the shadow where both lie.
        assert_eq!(img.pixel(50, 50), Some([0, 0, 255, 255]));
        assert_eq!(alpha(&img, 10, 10), 0);

        let inner = Item::InnerShadow { outline: square(20.0, 40.0, 60.0, 80.0), effect };
        let img = rasterize(&[fill, inner], flip(), 100, 100).unwrap();
        // The shadow falls along the top left edges inside the square, not outside it.
        assert_eq!(img.pixel(25, 25), Some([0, 0, 0, 255]), "top left inside: in the shade");
        assert_eq!(img.pixel(55, 55), Some([0, 0, 255, 255]), "bottom right inside: the fill");
        assert_eq!(alpha(&img, 10, 10), 0, "outside: nothing");
    }

    #[test]
    fn a_pattern_tiles_from_the_origin_within_its_outline() {
        // Two by one pixels: red then green.
        let image = Arc::new(Rgba8 { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 255, 0, 255] });
        let item = Item::Pattern { outline: square(0.0, 0.0, 50.0, 50.0), pattern: Pattern { image, unit: 10.0 } };
        let img = rasterize(&[item], flip(), 100, 100).unwrap();
        assert_eq!(img.pixel(5, 95), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(15, 95), Some([0, 255, 0, 255]));
        assert_eq!(img.pixel(25, 55), Some([255, 0, 0, 255]), "repeats across and up");
        assert_eq!(alpha(&img, 75, 95), 0, "nothing outside the outline");
    }
}
