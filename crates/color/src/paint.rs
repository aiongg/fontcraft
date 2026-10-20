//! The paint graph (M11.6 for solid layers, M11.7 for full-colour layers, M11.19 for
//! variation): one tree per colour glyph, read by Edit View, Font View, Text Preview, the SVG
//! writer and the COLR writer alike.
//!
//! Rows: `color.export.colrFromPaletteLayers`, `color.export.fullColorToColrV1`,
//! `color.export.strokeInColr`, `compile.colrGradientGeometry`, `color.layered.stackingOrder`,
//! `color.export.variableColr`, `color.export.colrGlyphComponent`.
//!
//! Contract (`docs/plans/M11.md` § 3):
//! - [`glyph_paint`] gives `None` for a glyph without vector colour layers (palette or
//!   full-colour) for the master. Otherwise a [`Paint::Layers`] with one entry per layer glyph,
//!   bottom first: each a [`Paint::Glyph`] whose [`ShapeRef`] names the shapes it fills.
//! - Consecutive shapes of one layer with equal paint attributes are merged into one
//!   [`ShapeRef`], as both oracles merge them; each [`ShapeRef`] becomes one layer glyph
//!   `<base>.color<i>`, numbered by its position.
//! - A stroke has no COLR paint: it becomes its own [`ShapeRef`] with a [`StrokeSpec`], above the
//!   fill, which the COLR writer expands to an outline and the SVG writer may draw as a stroke.
//!   The fill [`ShapeRef`] of a stroked path is the path's own outline, not its stroke.
//! - A master layer marked as full colour (fontc's test sources keep their paints there) paints
//!   like a full-colour layer, below the master's colour layers.
//! - A component of a glyph with full-colour layers, on a full-colour layer and without paint
//!   attributes of its own, is a [`Paint::ColrGlyph`] of that glyph moved by the component's
//!   transform, as glyphsLib writes it (`builder/color_layers.py`, Apache-2.0; see `NOTICE`).
//!   It is never merged with its neighbours, has no layer glyph, and takes a number among the
//!   layer glyphs ([`numbered_shape_refs`]). A reference that would lead back to its own glyph,
//!   or nest more than [`crate::MAX_PAINT_DEPTH`] deep, is drawn as an outline with a warning.
//! - Gradient geometry is in font units (the model's fractions of the shape's bounds resolved
//!   against the merged shapes' bounds). Alpha is kept as given; colours that are not palette
//!   references are added to the palettes by the CPAL writer, not here.
//! - Outlines are not drawn here: callers draw a [`ShapeRef`] with their own outline code, so
//!   this crate needs no path operations.
//! - Depth at most [`crate::MAX_PAINT_DEPTH`]; stops at most [`crate::MAX_STOPS`].
//! - Variation: a variable font varies a glyph's paint between masters whose paint graphs have
//!   the same structure ([`same_structure`], [`PaintVariation`]); an interpolated instance mixes
//!   its colour layers' paint attributes with the master weights ([`interpolate_paint_attrs`]).

use std::collections::HashMap;

use fontcraft_geom::{Affine, BezPath, Point, Rect, Shape as _};
use fontcraft_model::{Color, Component, Dimension, Extend, Font, Glyph, Gradient, GradientKind, Layer, PaletteEntry, Shape, ShapeAttrs};

use crate::layers::{ColorLayerKind, export_kind};
use crate::palette::FOREGROUND_INDEX;
use crate::{ColorError, MAX_LAYER_GLYPHS, Palettes};

/// A node of the paint graph.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    /// Paints drawn in order, the first at the bottom.
    Layers(Vec<Paint>),
    /// `paint` clipped to the outline of `shape`.
    Glyph {
        shape: ShapeRef,
        paint: Box<Paint>,
    },
    /// The paint graph of colour glyph `glyph`, moved by `transform`: a component of a glyph
    /// with full-colour layers, on a full-colour layer (`color.export.colrGlyphComponent`). It
    /// has no layer glyph. `shape` is that component; code that draws one glyph's layers
    /// without following other glyphs fills it with [`outline_fill`].
    ColrGlyph {
        glyph: String,
        transform: Affine,
        shape: ShapeRef,
    },
    Solid(ColorRef),
    /// A linear gradient from `p0` to `p1`, `p2` setting the rotation as in COLR version 1 (at
    /// right angles to `p0`–`p1` for an unrotated gradient).
    Linear {
        p0: Point,
        p1: Point,
        p2: Point,
        line: ColorLine,
    },
    /// A gradient between two circles.
    Radial {
        c0: Point,
        r0: f64,
        c1: Point,
        r1: f64,
        line: ColorLine,
    },
    /// A conic gradient around `center` from angle `start` to `end`, in degrees counter-clockwise
    /// from the positive x axis.
    Sweep {
        center: Point,
        start: f64,
        end: f64,
        line: ColorLine,
    },
}

/// A gradient's stops and what it paints beyond them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorLine {
    /// In offset order.
    pub stops: Vec<ColorStop>,
    pub extend: Extend,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub offset: f64,
    pub color: ColorRef,
}

/// A colour as the tables refer to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorRef {
    /// A palette row, with an alpha from 0 to 1 applied over the entry's own.
    Palette { index: u16, alpha: f64 },
    /// The text colour (palette index 65,535).
    Foreground { alpha: f64 },
    /// A colour that is not in the palettes: red, green, blue and alpha from 0 to 1.
    Rgba([f64; 4]),
}

/// Shapes of one layer that one layer glyph draws.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeRef {
    /// The layer's id within the glyph (a master layer's id for the masters of a layered font).
    pub layer: String,
    /// Indices into the layer's shapes (paths and components), in order.
    pub shapes: Vec<usize>,
    /// `Some` when this layer glyph is the shapes' stroke rather than their fill.
    pub stroke: Option<StrokeSpec>,
}

/// A stroke to expand, in font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeSpec {
    pub width: f64,
    /// The vertical size; the width when equal.
    pub height: f64,
    /// −1 inside, 0 centred, 1 outside (the version 4 convention).
    pub position: f64,
    /// 0 butt, 1 round, 2 square, 3 round inset.
    pub cap_start: u8,
    pub cap_end: u8,
    /// 0 miter, 1 round, 2 bevel, 3 round full.
    pub join: u8,
}

/// The fill of a shape on a full-colour layer with neither a fill colour nor a gradient: the
/// 50 % gray a new shape gets (`color.fullColor.defaultGrayFill`).
pub const DEFAULT_FILL: Color = Color::Rgba([0.5, 0.5, 0.5, 1.0]);
/// The colour of a stroke with a width and no stroke colour.
pub const DEFAULT_STROKE: Color = Color::Rgba([0.0, 0.0, 0.0, 1.0]);

/// The paint graph of `glyph` for `master_id`, colours resolved against `palettes` where a
/// writer needs them baked in. `None` when the glyph has no vector colour layers for the master.
///
/// A palette layer is one layer glyph of all its paths and components in its palette entry; a
/// layer without them is left out, as fontc leaves it out. A full-colour layer gives one layer
/// glyph per run of consecutive shapes with equal paint attributes, and for stroked paths a
/// stroke layer glyph above the run's fill. A component of a colour glyph on it paints that
/// glyph by reference ([`Paint::ColrGlyph`]) when [`colr_glyph_target`] allows it. A master layer
/// marked as full colour paints that way too, below the master's colour layers, as both oracles
/// read it. Intermediate colour layers are sources of their layer glyph's variation (M11.19),
/// not layers of their own.
pub fn glyph_paint(font: &Font, glyph: &Glyph, master_id: &str, palettes: &Palettes) -> Result<Option<Paint>, ColorError> {
    glyph_paint_warn(font, glyph, master_id, palettes, &mut Vec::new())
}

/// [`glyph_paint`], adding to `warnings` a message for each component of a colour glyph that is
/// painted as its outline because [`colr_glyph_target`] refuses it.
pub fn glyph_paint_warn(
    font: &Font,
    glyph: &Glyph,
    master_id: &str,
    palettes: &Palettes,
    warnings: &mut Vec<String>,
) -> Result<Option<Paint>, ColorError> {
    let _ = palettes;
    MasterPaints::new(font, master_id).paint(glyph, warnings)
}

/// [`glyph_paint_warn`] for many glyphs of one master: what it learns about the glyphs that
/// components refer to is kept from one glyph to the next, so a glyph many others refer to is
/// read once, not once for each.
pub struct MasterPaints<'a>(Refs<'a>);

impl<'a> MasterPaints<'a> {
    pub fn new(font: &'a Font, master_id: &'a str) -> Self {
        MasterPaints(Refs { font, master: master_id, painted: HashMap::new(), reach: Reach::default() })
    }

    /// The paint graph of `glyph` in the master, as [`glyph_paint_warn`] gives it.
    pub fn paint(&mut self, glyph: &Glyph, warnings: &mut Vec<String>) -> Result<Option<Paint>, ColorError> {
        self.0.paint(glyph, warnings, 0)
    }
}

/// Paint graphs of one master, with what is known of the glyphs components refer to.
struct Refs<'a> {
    font: &'a Font,
    master: &'a str,
    /// Whether a glyph has a paint graph, for each glyph a component referred to.
    painted: HashMap<String, bool>,
    reach: Reach,
}

impl Refs<'_> {
    fn paint(&mut self, glyph: &Glyph, warnings: &mut Vec<String>, depth: usize) -> Result<Option<Paint>, ColorError> {
        let master_id = self.master;
        let mut layers = Vec::new();
        let painted =
            glyph.layers.iter().filter(|l| l.master == master_id).filter_map(|l| export_kind(l).map(|k| (l, k))).take(crate::MAX_COLOR_LAYERS);
        for (layer, kind) in painted {
            if !layer.info.attrs.coordinates.is_empty() {
                continue;
            }
            match kind {
                ColorLayerKind::Palette(entry) => {
                    let shapes = drawn_shapes(&layer.shapes);
                    if shapes.is_empty() {
                        continue;
                    }
                    let color = match entry {
                        PaletteEntry::Index(index) if index != FOREGROUND_INDEX => ColorRef::Palette { index, alpha: 1.0 },
                        PaletteEntry::Index(_) | PaletteEntry::Foreground => ColorRef::Foreground { alpha: 1.0 },
                    };
                    layers.push(Paint::Glyph {
                        shape: ShapeRef { layer: layer.id.clone(), shapes, stroke: None },
                        paint: Box::new(Paint::Solid(color)),
                    });
                }
                ColorLayerKind::FullColor => self.full_color_paints(glyph, layer, &mut layers, warnings, depth)?,
                ColorLayerKind::IColor(_) | ColorLayerKind::Svg => {}
            }
            if layers.len() > MAX_LAYER_GLYPHS {
                return Err(ColorError::LimitExceeded { what: "layer glyphs in one colour glyph", limit: MAX_LAYER_GLYPHS });
            }
        }
        Ok((!layers.is_empty()).then_some(Paint::Layers(layers)))
    }

    /// The paints of the full-colour `layer`, appended to `out` bottom first.
    fn full_color_paints(
        &mut self,
        glyph: &Glyph,
        layer: &Layer,
        out: &mut Vec<Paint>,
        warnings: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), ColorError> {
        let bad = |message: String| ColorError::BadPaint { glyph: glyph.name.clone(), message };
        let mut runs: Vec<Run> = Vec::new();
        for i in drawn_shapes(&layer.shapes) {
            let Some(shape) = layer.shapes.get(i) else { continue };
            if let Shape::Component(c) = shape
                && self.target(glyph, c, warnings, depth)
            {
                runs.push(Run::Colr { glyph: c.glyph.clone(), transform: c.affine(), shape: i });
                continue;
            }
            let look = look(shape).map_err(bad)?;
            match runs.last_mut() {
                Some(Run::Shapes(last, shapes)) if *last == look => shapes.push(i),
                _ => runs.push(Run::Shapes(look, vec![i])),
            }
        }
        for run in runs {
            let (look, shapes) = match run {
                Run::Shapes(look, shapes) => (look, shapes),
                Run::Colr { glyph, transform, shape } => {
                    out.push(Paint::ColrGlyph { glyph, transform, shape: ShapeRef { layer: layer.id.clone(), shapes: vec![shape], stroke: None } });
                    continue;
                }
            };
            if let Some(fill) = &look.fill {
                let paint = match fill {
                    Fill::Color(c) => Paint::Solid(*c),
                    Fill::Gradient(g) => gradient_paint(g, run_bounds(self.font, layer, &shapes).map_err(bad)?).map_err(bad)?,
                };
                out.push(Paint::Glyph { shape: ShapeRef { layer: layer.id.clone(), shapes: shapes.clone(), stroke: None }, paint: Box::new(paint) });
            }
            if let Some((spec, color)) = look.stroke {
                out.push(Paint::Glyph {
                    shape: ShapeRef { layer: layer.id.clone(), shapes, stroke: Some(spec) },
                    paint: Box::new(Paint::Solid(color)),
                });
            }
            if out.len() > MAX_LAYER_GLYPHS {
                return Err(ColorError::LimitExceeded { what: "layer glyphs in one colour glyph", limit: MAX_LAYER_GLYPHS });
            }
        }
        Ok(())
    }

    /// Whether component `c` of a full-colour layer of `base` paints its glyph by reference
    /// ([`colr_glyph_target`]). A component that qualifies but whose glyph does not export, leads
    /// back to `base`, lies deeper than [`crate::MAX_PAINT_DEPTH`] or paints nothing is drawn
    /// as its outline, with a warning.
    fn target(&mut self, base: &Glyph, c: &Component, warnings: &mut Vec<String>, depth: usize) -> bool {
        let font = self.font;
        let Some(target) = font.glyph(&c.glyph) else { return false };
        if !colr_glyph_target(font, c, self.master) {
            return false;
        }
        let why = if !target.export {
            Some(format!("`{}` does not export", c.glyph))
        } else if depth >= crate::MAX_PAINT_DEPTH || self.reach.leads_back(font, self.master, &c.glyph, &base.name) {
            Some(format!("its colour layers lead back to `{}` or nest more than {} deep", base.name, crate::MAX_PAINT_DEPTH))
        } else if !self.paints(target, depth + 1) {
            Some(format!("`{}` has no colour layers that paint", c.glyph))
        } else {
            None
        };
        match why {
            None => true,
            Some(why) => {
                warnings.push(format!("the component of `{}` on a colour layer is drawn as its outline: {why}", c.glyph));
                false
            }
        }
    }

    /// Whether `glyph` has a paint graph for the master.
    fn paints(&mut self, glyph: &Glyph, depth: usize) -> bool {
        if let Some(&known) = self.painted.get(&glyph.name) {
            return known;
        }
        // Warnings are given when the glyph itself is painted.
        let known = matches!(self.paint(glyph, &mut Vec::new(), depth), Ok(Some(_)));
        self.painted.insert(glyph.name.clone(), known);
        known
    }
}

/// Consecutive shapes of a full-colour layer that paint alike, or one component that paints its
/// glyph by reference.
enum Run {
    Shapes(Look, Vec<usize>),
    Colr { glyph: String, transform: Affine, shape: usize },
}

/// Whether component `c` on a full-colour layer of `master` is one glyphsLib writes as a
/// `PaintColrGlyph`: it has neither a fill colour nor a gradient of its own, and its glyph has a
/// full-colour layer in `master` (an intermediate layer does not count). The component's
/// transform moves the referenced paint.
pub fn colr_glyph_target(font: &Font, c: &Component, master: &str) -> bool {
    let a = &c.info.attrs;
    a.fill_color.is_none() && a.gradient.is_none() && font.glyph(&c.glyph).is_some_and(|g| has_full_color(g, master))
}

fn has_full_color(g: &Glyph, master: &str) -> bool {
    g.layers.iter().any(|l| l.master == master && l.info.attrs.coordinates.is_empty() && export_kind(l) == Some(ColorLayerKind::FullColor))
}

/// The graph of the components that [`colr_glyph_target`] allows in one master, from each
/// glyph's full-colour layers to the glyphs they name, read as far as queries need it. Its
/// strongly connected components are found once (Tarjan's algorithm, without recursion), so
/// each glyph and component is looked at once per master however many references are checked.
#[derive(Default)]
struct Reach {
    ids: HashMap<String, usize>,
    names: Vec<String>,
    /// Per glyph id: the ids its components name, read when the glyph is visited.
    edges: Vec<Vec<usize>>,
    /// Per glyph id: its visit number, its lowest reachable visit number, whether it is on the
    /// stack, and its strongly connected component once that is complete.
    index: Vec<Option<usize>>,
    low: Vec<usize>,
    on_stack: Vec<bool>,
    comp: Vec<Option<usize>>,
    /// Per component: the longest chain of references from it to other components, capped
    /// just past [`crate::MAX_PAINT_DEPTH`].
    height: Vec<usize>,
    stack: Vec<usize>,
    visits: usize,
}

impl Reach {
    fn id(&mut self, name: &str) -> usize {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.edges.len();
        self.ids.insert(name.to_string(), id);
        self.names.push(name.to_string());
        self.edges.push(Vec::new());
        self.index.push(None);
        self.low.push(0);
        self.on_stack.push(false);
        self.comp.push(None);
        id
    }

    /// Whether the components lead from `from` back to `base`, which has a component of `from`,
    /// or nest more than [`crate::MAX_PAINT_DEPTH`] deep from `from`.
    fn leads_back(&mut self, font: &Font, master: &str, from: &str, base: &str) -> bool {
        let from = self.id(from);
        if self.index.get(from).copied().flatten().is_none() {
            self.visit(font, master, from);
        }
        let comp = self.comp.get(from).copied().flatten();
        // `base` names `from`, so `from` reaches `base` exactly when they share a component;
        // a `base` the walk from `from` never met is not reached.
        let same = self.ids.get(base).is_some_and(|&b| self.comp.get(b).copied().flatten() == comp);
        same || comp.and_then(|c| self.height.get(c)).is_some_and(|&h| h > crate::MAX_PAINT_DEPTH)
    }

    /// Tarjan's algorithm from `root`, which has not been visited.
    fn visit(&mut self, font: &Font, master: &str, root: usize) {
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some((u, pos)) = work.pop() {
            if pos == 0 {
                self.open(font, master, u);
            }
            let next = self.edges.get(u).and_then(|e| e.get(pos)).copied();
            if let Some(v) = next {
                work.push((u, pos + 1));
                match self.index.get(v).copied().flatten() {
                    None => work.push((v, 0)),
                    Some(iv) if self.on_stack.get(v).copied().unwrap_or(false) => self.lower(u, iv),
                    Some(_) => {}
                }
                continue;
            }
            let low = self.low.get(u).copied().unwrap_or(0);
            if self.index.get(u).copied().flatten() == Some(low) {
                self.close(u);
            }
            if let Some(&(parent, _)) = work.last() {
                self.lower(parent, low);
            }
        }
    }

    fn lower(&mut self, u: usize, to: usize) {
        if let Some(l) = self.low.get_mut(u) {
            *l = (*l).min(to);
        }
    }

    /// Numbers glyph `u` and reads the glyphs its components name.
    fn open(&mut self, font: &Font, master: &str, u: usize) {
        let visit = self.visits;
        self.visits = self.visits.saturating_add(1);
        if let (Some(i), Some(l), Some(s)) = (self.index.get_mut(u), self.low.get_mut(u), self.on_stack.get_mut(u)) {
            *i = Some(visit);
            *l = visit;
            *s = true;
        }
        self.stack.push(u);
        let Some(g) = self.names.get(u).and_then(|n| font.glyph(n)) else { return };
        let mut out = Vec::new();
        let layers = g
            .layers
            .iter()
            .filter(|l| l.master == master && l.info.attrs.coordinates.is_empty() && export_kind(l) == Some(ColorLayerKind::FullColor));
        for l in layers {
            for s in &l.shapes {
                if let Shape::Component(c) = s
                    && colr_glyph_target(font, c, master)
                {
                    out.push(self.id(&c.glyph));
                }
            }
        }
        if let Some(e) = self.edges.get_mut(u) {
            *e = out;
        }
    }

    /// Closes the strongly connected component whose first glyph is `u` and measures it.
    fn close(&mut self, u: usize) {
        let c = self.height.len();
        let mut members = Vec::new();
        while let Some(w) = self.stack.pop() {
            if let (Some(s), Some(k)) = (self.on_stack.get_mut(w), self.comp.get_mut(w)) {
                *s = false;
                *k = Some(c);
            }
            members.push(w);
            if w == u {
                break;
            }
        }
        // Every glyph these name is in this component or in one closed before it.
        let mut height = 0usize;
        for w in members {
            for v in self.edges.get(w).map_or(&[][..], Vec::as_slice) {
                if let Some(k) = self.comp.get(*v).copied().flatten().filter(|&k| k != c) {
                    height = height.max(self.height.get(k).copied().unwrap_or(0).saturating_add(1));
                }
            }
        }
        self.height.push(height.min(crate::MAX_PAINT_DEPTH + 1));
    }
}

/// What drawing code that follows no other glyph fills the `shape` of a [`Paint::ColrGlyph`]
/// with: [`DEFAULT_FILL`], as a shape without paint attributes is filled.
pub fn outline_fill() -> Paint {
    Paint::Solid(color_ref(DEFAULT_FILL))
}

/// What one shape of a full-colour layer paints; consecutive shapes that paint the same are one
/// run, as both oracles merge them.
#[derive(Clone, Debug, PartialEq)]
struct Look {
    fill: Option<Fill>,
    stroke: Option<(StrokeSpec, ColorRef)>,
}

#[derive(Clone, Debug, PartialEq)]
enum Fill {
    Color(ColorRef),
    Gradient(Gradient),
}

/// The paint of one shape. A gradient wins over the fill colour; a shape with neither is filled
/// with [`DEFAULT_FILL`] unless it is a stroke that is not also filled. Only paths have strokes.
/// A fill or stroke colour with no opacity paints nothing.
fn look(shape: &Shape) -> Result<Look, String> {
    let (attrs, stroke) = match shape {
        Shape::Path(p) => (&p.info.attrs, stroke_spec(&p.info.attrs)?),
        Shape::Component(c) => (&c.info.attrs, None),
        Shape::Image(_) | Shape::Raw(_) => return Ok(Look { fill: None, stroke: None }),
    };
    let fill = match (&attrs.gradient, attrs.fill_color) {
        (Some(g), _) => Some(Fill::Gradient(g.clone())),
        (None, Some(c)) => visible(&c).then(|| Fill::Color(color_ref(c))),
        (None, None) => (stroke.is_none() || attrs.fill).then(|| Fill::Color(color_ref(DEFAULT_FILL))),
    };
    let stroke = stroke.and_then(|spec| {
        let c = attrs.stroke_color.unwrap_or(DEFAULT_STROKE);
        visible(&c).then(|| (spec, color_ref(c)))
    });
    Ok(Look { fill, stroke })
}

/// The stroke `attrs` describe; `None` without a positive width.
fn stroke_spec(attrs: &ShapeAttrs) -> Result<Option<StrokeSpec>, String> {
    let Some(width) = dimension(&attrs.stroke_width, "stroke width")? else { return Ok(None) };
    if width <= 0.0 {
        return Ok(None);
    }
    Ok(Some(StrokeSpec {
        width,
        height: dimension(&attrs.stroke_height, "stroke height")?.unwrap_or(width),
        position: dimension(&attrs.stroke_pos, "stroke position")?.unwrap_or(0.0),
        cap_start: attrs.line_cap_start.unwrap_or(0),
        cap_end: attrs.line_cap_end.unwrap_or(0),
        join: attrs.line_join.unwrap_or(0),
    }))
}

/// A stroke size or position as a finite number; an expression that is not a plain number cannot
/// be evaluated here.
fn dimension(d: &Option<Dimension>, what: &str) -> Result<Option<f64>, String> {
    let n = match d {
        None => return Ok(None),
        Some(Dimension::Number(n)) => Some(*n),
        Some(Dimension::Expression(e)) => e.trim().parse::<f64>().ok(),
    };
    match n {
        Some(n) if n.is_finite() => Ok(Some(n)),
        _ => Err(format!("its {what} is not a number")),
    }
}

fn visible(c: &Color) -> bool {
    match c {
        Color::Rgba([_, _, _, a]) | Color::Palette { alpha: a, .. } => *a > 0.0,
    }
}

fn color_ref(c: Color) -> ColorRef {
    match c {
        Color::Rgba(rgba) => ColorRef::Rgba(rgba),
        Color::Palette { index, alpha } if index == FOREGROUND_INDEX => ColorRef::Foreground { alpha },
        Color::Palette { index, alpha } => ColorRef::Palette { index, alpha },
    }
}

/// The bounding box of the outlines of `shapes` of `layer`, curve extremes included, which is
/// what gradient fractions are fractions of (`compile.colrGradientGeometry`). The box of the
/// layer glyph these shapes make, so a compiled font opens back to the same fractions. All zero
/// without an outline.
fn run_bounds(font: &Font, layer: &Layer, shapes: &[usize]) -> Result<Rect, String> {
    let mut out: Option<Rect> = None;
    let mut add = |path: &BezPath| {
        if !path.elements().is_empty() {
            let b = path.bounding_box();
            out = Some(out.map_or(b, |o| o.union(b)));
        }
    };
    for s in shapes.iter().filter_map(|&i| layer.shapes.get(i)) {
        match s {
            Shape::Path(p) => add(&p.to_contour()?),
            Shape::Component(c) => {
                let path = font.outline(&c.glyph, &layer.master).map_err(|e| e.to_string())?;
                add(&(c.affine() * path));
            }
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    Ok(out.unwrap_or(Rect::ZERO))
}

/// `gradient` in font units over `bounds` (`compile.colrGradientGeometry`): points are fractions
/// of the box, radii fractions of its width. A linear gradient's third point is the start-to-end
/// vector turned −90° about the start. A radial gradient with both radii 0 (a version 3 file,
/// which stores none) is a circle around `start` from radius 0 to the farthest corner of the box,
/// as both oracles write it. Stops are sorted by offset and their offsets clamped to 0..1.
fn gradient_paint(gradient: &Gradient, bounds: Rect) -> Result<Paint, String> {
    if gradient.stops.len() > crate::MAX_STOPS {
        return Err(format!("a gradient has more than {} stops", crate::MAX_STOPS));
    }
    if !gradient.numbers().all(f64::is_finite) {
        return Err("a gradient has a value that is not a number".to_string());
    }
    let mut stops: Vec<ColorStop> =
        gradient.stops.iter().map(|s| ColorStop { offset: s.offset.clamp(0.0, 1.0), color: color_ref(s.color) }).collect();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let line = ColorLine { stops, extend: gradient.extend };
    let at = |p: Point| Point::new(bounds.x0 + bounds.width() * p.x, bounds.y0 + bounds.height() * p.y);
    Ok(match gradient.kind {
        GradientKind::Linear { start, end } => {
            let (p0, p1) = (at(start), at(end));
            Paint::Linear { p0, p1, p2: Point::new(p0.x + (p1.y - p0.y), p0.y - (p1.x - p0.x)), line }
        }
        GradientKind::Radial { start, end, start_radius, end_radius, .. } => {
            let c0 = at(start);
            if start_radius == 0.0 && end_radius == 0.0 {
                let corners = [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x0, bounds.y1), (bounds.x1, bounds.y1)];
                let r1 = corners.iter().map(|&(x, y)| (x - c0.x).hypot(y - c0.y)).fold(0.0, f64::max);
                Paint::Radial { c0, r0: 0.0, c1: c0, r1, line }
            } else {
                let w = bounds.width();
                Paint::Radial { c0, r0: (start_radius * w).abs(), c1: at(end), r1: (end_radius * w).abs(), line }
            }
        }
        GradientKind::Conic { center, start_angle, angle, .. } => {
            Paint::Sweep { center: at(center), start: start_angle, end: start_angle + angle, line }
        }
    })
}

/// The indices of the shapes in `shapes` that have an outline: paths and components.
pub fn drawn_shapes(shapes: &[Shape]) -> Vec<usize> {
    shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Path(_) | Shape::Component(_))).map(|(i, _)| i).collect()
}

/// Whether every paint in `paint` is solid, so the glyph fits COLR version 0
/// (`color.export.colrVersionChoice`).
///
/// A stack of solid layers whose colours are opaque palette references, the text colour, or
/// colours of their own (their alpha goes into `CPAL`). A palette reference with its own alpha
/// needs version 1, which has an alpha per paint.
pub fn is_solid_stack(paint: &Paint) -> bool {
    let solid_glyph = |p: &Paint| match p {
        Paint::Glyph { paint, .. } => match paint.as_ref() {
            Paint::Solid(ColorRef::Rgba(_)) => true,
            Paint::Solid(ColorRef::Palette { alpha, .. } | ColorRef::Foreground { alpha }) => *alpha >= 1.0,
            _ => false,
        },
        _ => false,
    };
    match paint {
        Paint::Layers(list) => list.iter().all(solid_glyph),
        other => solid_glyph(other),
    }
}

/// The layer glyphs `paint` needs, in order: the [`ShapeRef`] of every [`Paint::Glyph`].
/// Nothing below [`crate::MAX_PAINT_DEPTH`] is visited.
pub fn shape_refs(paint: &Paint) -> Vec<&ShapeRef> {
    numbered_shape_refs(paint).into_iter().map(|(_, s)| s).collect()
}

/// [`shape_refs`], each with the number its layer glyph `<base>.color<i>` takes: its place among
/// the glyph's layer paints, where a [`Paint::ColrGlyph`] takes a place too, as glyphsLib
/// numbers them. Without such references the numbers count from 0.
pub fn numbered_shape_refs(paint: &Paint) -> Vec<(usize, &ShapeRef)> {
    fn collect<'a>(paint: &'a Paint, depth: usize, next: &mut usize, out: &mut Vec<(usize, &'a ShapeRef)>) {
        if depth > crate::MAX_PAINT_DEPTH {
            return;
        }
        match paint {
            Paint::Layers(list) => list.iter().for_each(|p| collect(p, depth + 1, next, out)),
            Paint::Glyph { shape, paint } => {
                out.push((*next, shape));
                *next += 1;
                collect(paint, depth + 1, next, out);
            }
            Paint::ColrGlyph { .. } => *next += 1,
            Paint::Solid(_) | Paint::Linear { .. } | Paint::Radial { .. } | Paint::Sweep { .. } => {}
        }
    }
    let mut out = Vec::new();
    collect(paint, 0, &mut 0, &mut out);
    out
}

/// The glyphs `paint` paints by reference ([`Paint::ColrGlyph`]), in order.
pub fn colr_glyph_refs(paint: &Paint) -> Vec<&str> {
    fn collect<'a>(paint: &'a Paint, depth: usize, out: &mut Vec<&'a str>) {
        if depth > crate::MAX_PAINT_DEPTH {
            return;
        }
        match paint {
            Paint::Layers(list) => list.iter().for_each(|p| collect(p, depth + 1, out)),
            Paint::Glyph { paint, .. } => collect(paint, depth + 1, out),
            Paint::ColrGlyph { glyph, .. } => out.push(glyph),
            Paint::Solid(_) | Paint::Linear { .. } | Paint::Radial { .. } | Paint::Sweep { .. } => {}
        }
    }
    let mut out = Vec::new();
    collect(paint, 0, &mut out);
    out
}

/// How many levels `paint` nests below itself, not following [`Paint::ColrGlyph`] references:
/// 0 for a leaf, one more than its deepest child for [`Paint::Layers`] and [`Paint::Glyph`].
/// Capped at one past [`crate::MAX_PAINT_DEPTH`]. Drawing code that inlines a reference uses it
/// to fall back to the outline before the inlined paint would pass its depth limit.
pub fn nesting(paint: &Paint) -> usize {
    fn walk(paint: &Paint, depth: usize) -> usize {
        if depth > crate::MAX_PAINT_DEPTH {
            return depth;
        }
        match paint {
            Paint::Layers(list) => list.iter().map(|p| walk(p, depth + 1)).max().unwrap_or(depth + 1),
            Paint::Glyph { paint, .. } => walk(paint, depth + 1),
            Paint::ColrGlyph { .. } | Paint::Solid(_) | Paint::Linear { .. } | Paint::Radial { .. } | Paint::Sweep { .. } => depth,
        }
    }
    walk(paint, 0)
}

/// The paints of a variable font's colour glyphs at the font's sources other than the default
/// master (`color.export.variableColr`).
///
/// Each master paints a glyph with its own [`glyph_paint`]; layer glyph `i` of a master matches
/// layer glyph `i` of the default master, as both oracles match colour layers by position. A
/// master whose paint differs in anything but its numbers ([`same_structure`]) does not vary the
/// glyph's paints. The COLR writer turns the numbers that differ into `PaintVar*` paints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintVariation {
    /// Normalized locations of the sources besides the default, one value per axis of the font.
    pub locations: Vec<Vec<f64>>,
    /// Each colour glyph's paint at each source, parallel to [`PaintVariation::locations`];
    /// `None` where the source does not vary it.
    pub glyphs: std::collections::BTreeMap<String, Vec<Option<Paint>>>,
    /// The layer glyphs the variable compile made source glyphs of before the outlines, so their
    /// outlines vary as any glyph's do. A COLR writer finds them by name.
    pub layer_glyphs: Vec<String>,
}

impl PaintVariation {
    /// The paints of `glyph` at the sources, parallel to [`PaintVariation::locations`].
    pub fn sources(&self, glyph: &str) -> &[Option<Paint>] {
        self.glyphs.get(glyph).map_or(&[], Vec::as_slice)
    }

    /// Whether some source paints `glyph` with numbers other than `default`'s.
    pub fn varies(&self, glyph: &str, default: &Paint) -> bool {
        let base = numbers(default);
        self.sources(glyph).iter().flatten().any(|p| numbers(p) != base)
    }
}

/// Whether `b` paints like `a` apart from its numbers: the same nodes, layer glyphs that are
/// fills or strokes alike, the same palette rows and colours of their own, the same extend modes
/// and number of stops. Such paints vary into each other; `CPAL` colours cannot vary.
pub fn same_structure(a: &Paint, b: &Paint) -> bool {
    fn same_color(a: &ColorRef, b: &ColorRef) -> bool {
        match (a, b) {
            (ColorRef::Palette { index: i, .. }, ColorRef::Palette { index: j, .. }) => i == j,
            (ColorRef::Foreground { .. }, ColorRef::Foreground { .. }) => true,
            (ColorRef::Rgba(x), ColorRef::Rgba(y)) => x == y,
            _ => false,
        }
    }
    fn same_line(a: &ColorLine, b: &ColorLine) -> bool {
        a.extend == b.extend && a.stops.len() == b.stops.len() && a.stops.iter().zip(&b.stops).all(|(s, t)| same_color(&s.color, &t.color))
    }
    fn go(a: &Paint, b: &Paint, depth: usize) -> bool {
        if depth > crate::MAX_PAINT_DEPTH {
            return false;
        }
        match (a, b) {
            (Paint::Layers(x), Paint::Layers(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| go(p, q, depth + 1)),
            (Paint::Glyph { shape: s, paint: p }, Paint::Glyph { shape: t, paint: q }) => {
                s.stroke.is_some() == t.stroke.is_some() && go(p, q, depth + 1)
            }
            (Paint::ColrGlyph { glyph: g, .. }, Paint::ColrGlyph { glyph: h, .. }) => g == h,
            (Paint::Solid(c), Paint::Solid(d)) => same_color(c, d),
            (Paint::Linear { line: l, .. }, Paint::Linear { line: m, .. })
            | (Paint::Radial { line: l, .. }, Paint::Radial { line: m, .. })
            | (Paint::Sweep { line: l, .. }, Paint::Sweep { line: m, .. }) => same_line(l, m),
            _ => false,
        }
    }
    go(a, b, 0)
}

/// The numbers of `paint` a variable font can vary, in tree order: the alpha of each palette
/// reference, gradient geometry, and each stop's offset and alpha. Nothing below
/// [`crate::MAX_PAINT_DEPTH`] is read.
pub fn numbers(paint: &Paint) -> Vec<f64> {
    fn alpha(c: &ColorRef, out: &mut Vec<f64>) {
        match c {
            ColorRef::Palette { alpha, .. } | ColorRef::Foreground { alpha } => out.push(*alpha),
            ColorRef::Rgba(_) => {}
        }
    }
    fn line(l: &ColorLine, out: &mut Vec<f64>) {
        for s in &l.stops {
            out.push(s.offset);
            alpha(&s.color, out);
        }
    }
    fn go(paint: &Paint, depth: usize, out: &mut Vec<f64>) {
        if depth > crate::MAX_PAINT_DEPTH {
            return;
        }
        match paint {
            Paint::Layers(list) => list.iter().for_each(|p| go(p, depth + 1, out)),
            Paint::Glyph { paint, .. } => go(paint, depth + 1, out),
            // The default master's transform is written; it does not vary.
            Paint::ColrGlyph { .. } => {}
            Paint::Solid(c) => alpha(c, out),
            Paint::Linear { p0, p1, p2, line: l } => {
                out.extend([p0.x, p0.y, p1.x, p1.y, p2.x, p2.y]);
                line(l, out);
            }
            Paint::Radial { c0, r0, c1, r1, line: l } => {
                out.extend([c0.x, c0.y, *r0, c1.x, c1.y, *r1]);
                line(l, out);
            }
            Paint::Sweep { center, start, end, line: l } => {
                out.extend([center.x, center.y, *start, *end]);
                line(l, out);
            }
        }
    }
    let mut out = Vec::new();
    go(paint, 0, &mut out);
    out
}

/// The paint attributes of `out` (a colour layer of an interpolated instance) interpolated from
/// `sources`, the same layer at each master with the master's weight: fill and stroke colours
/// and gradients (geometry, stops, colours). A shape whose attributes differ in kind between
/// the sources (a colour against a gradient, other palette rows, other stop counts) keeps what
/// `out` has, which is the heaviest master's. Colour components are kept within 0 to 1.
pub fn interpolate_paint_attrs(out: &mut [Shape], sources: &[(&[Shape], f64)]) {
    for (i, shape) in out.iter_mut().enumerate() {
        let Some(target) = attrs_mut(shape) else { continue };
        let Some(attrs) = sources.iter().map(|(shapes, w)| shapes.get(i).and_then(shape_attrs).map(|a| (a, *w))).collect::<Option<Vec<_>>>() else {
            continue;
        };
        if let Some(c) = mix_colors(attrs.iter().map(|(a, w)| a.fill_color.map(|c| (c, *w)))) {
            target.fill_color = Some(c);
        }
        if let Some(c) = mix_colors(attrs.iter().map(|(a, w)| a.stroke_color.map(|c| (c, *w)))) {
            target.stroke_color = Some(c);
        }
        if let Some(g) = attrs.iter().map(|(a, w)| a.gradient.as_ref().map(|g| (g, *w))).collect::<Option<Vec<_>>>().and_then(|g| mix_gradients(&g)) {
            target.gradient = Some(g);
        }
    }
}

fn shape_attrs(shape: &Shape) -> Option<&ShapeAttrs> {
    match shape {
        Shape::Path(p) => Some(&p.info.attrs),
        Shape::Component(c) => Some(&c.info.attrs),
        Shape::Image(_) | Shape::Raw(_) => None,
    }
}

fn attrs_mut(shape: &mut Shape) -> Option<&mut ShapeAttrs> {
    match shape {
        Shape::Path(p) => Some(&mut p.info.attrs),
        Shape::Component(c) => Some(&mut c.info.attrs),
        Shape::Image(_) | Shape::Raw(_) => None,
    }
}

/// The weighted sum of colours of one kind (all RGBA, or all the same palette row); `None` when
/// a source has none, the kinds differ, or the sum is not a number.
fn mix_colors(colors: impl Iterator<Item = Option<(Color, f64)>>) -> Option<Color> {
    let colors: Vec<(Color, f64)> = colors.collect::<Option<Vec<_>>>()?;
    let (first, _) = colors.first()?;
    let unit = |v: f64| v.is_finite().then(|| v.clamp(0.0, 1.0));
    match *first {
        Color::Rgba(_) => {
            let mut sum = [0.0; 4];
            for (c, w) in &colors {
                let Color::Rgba(rgba) = c else { return None };
                sum.iter_mut().zip(rgba).for_each(|(s, v)| *s += v * w);
            }
            let [r, g, b, a] = sum;
            Some(Color::Rgba([unit(r)?, unit(g)?, unit(b)?, unit(a)?]))
        }
        Color::Palette { index, .. } => {
            let mut alpha = 0.0;
            for (c, w) in &colors {
                match c {
                    Color::Palette { index: i, alpha: a } if *i == index => alpha += a * w,
                    _ => return None,
                }
            }
            Some(Color::Palette { index, alpha: unit(alpha)? })
        }
    }
}

/// The weighted sum of gradients of one kind with the same number of stops; `None` otherwise.
fn mix_gradients(gradients: &[(&Gradient, f64)]) -> Option<Gradient> {
    let (first, _) = gradients.first()?;
    let n = first.numbers().count();
    if gradients.iter().any(|(g, _)| {
        std::mem::discriminant(&g.kind) != std::mem::discriminant(&first.kind) || g.stops.len() != first.stops.len() || g.numbers().count() != n
    }) {
        return None;
    }
    let mut sum = vec![0.0; n];
    for (g, w) in gradients {
        sum.iter_mut().zip(g.numbers()).for_each(|(s, v)| *s += v * w);
    }
    if !sum.iter().all(|v| v.is_finite()) {
        return None;
    }
    let mut it = sum.into_iter();
    let mut next = || it.next().unwrap_or(0.0);
    let kind = match first.kind {
        GradientKind::Linear { .. } => {
            let start = Point::new(next(), next());
            GradientKind::Linear { start, end: Point::new(next(), next()) }
        }
        GradientKind::Radial { .. } => {
            let (start, end) = (Point::new(next(), next()), Point::new(next(), next()));
            GradientKind::Radial { start, end, start_radius: next(), end_radius: next(), angle: next() }
        }
        GradientKind::Conic { .. } => {
            let center = Point::new(next(), next());
            GradientKind::Conic { center, start_angle: next(), angle: next(), control_radius: next() }
        }
    };
    let mut stops = Vec::with_capacity(first.stops.len());
    for (k, stop) in first.stops.iter().enumerate() {
        let offset = next();
        // The colour is mixed on its own below, so its numbers are skipped here.
        for _ in stop.color.numbers() {
            next();
        }
        let color = mix_colors(gradients.iter().map(|(g, w)| g.stops.get(k).map(|s| (s.color, *w))))?;
        stops.push(fontcraft_model::ColorStop { offset: offset.clamp(0.0, 1.0), color });
    }
    Some(Gradient { kind, stops, extend: first.extend })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    use fontcraft_model::ColorStop as Stop;

    use super::*;

    fn path(rect: [f64; 4], style: impl FnOnce(&mut ShapeAttrs)) -> Shape {
        let mut p = fontcraft_model::Path::rect(rect[0], rect[1], rect[2], rect[3]);
        style(&mut p.info.attrs);
        Shape::Path(p)
    }

    fn colour_layer(master: &str, id: &str, shapes: Vec<Shape>) -> Layer {
        let mut l = Layer::new(master, 600.0);
        l.id = id.into();
        l.info.attrs.full_color = true;
        l.shapes = shapes;
        l
    }

    fn glyph_with(layers: impl FnOnce(&str) -> Vec<Layer>) -> (Font, Glyph) {
        let font = Font::new("Paints");
        let master = font.masters[0].id.clone();
        let mut g = Glyph::new("A");
        g.layers.push(Layer::new(master.as_str(), 600.0));
        g.layers.extend(layers(&master));
        (font, g)
    }

    fn paint_of(font: &Font, g: &Glyph) -> Result<Option<Paint>, ColorError> {
        glyph_paint(font, g, &font.masters[0].id, &Palettes::default())
    }

    fn solid(layer: &str, shapes: Vec<usize>, stroke: Option<StrokeSpec>, color: ColorRef) -> Paint {
        Paint::Glyph { shape: ShapeRef { layer: layer.into(), shapes, stroke }, paint: Box::new(Paint::Solid(color)) }
    }

    #[test]
    fn full_colour_layers_merge_runs_split_strokes_and_place_gradients_in_font_units() {
        const RED: [f64; 4] = [1.0, 0.0, 0.0, 1.0];
        let gradient = Gradient {
            kind: GradientKind::Linear { start: Point::new(0.25, 0.5), end: Point::new(0.75, 1.0) },
            // Out of order: written sorted by offset.
            stops: vec![
                Stop { offset: 1.0, color: Color::Palette { index: 0, alpha: 1.0 } },
                Stop { offset: 0.0, color: Color::Palette { index: 1, alpha: 0.5 } },
            ],
            extend: Extend::Reflect,
        };
        let shapes = vec![
            path([0.0, 0.0, 100.0, 100.0], |a| a.fill_color = Some(Color::Rgba(RED))),
            path([200.0, 0.0, 300.0, 100.0], |a| a.fill_color = Some(Color::Rgba(RED))),
            path([100.0, 100.0, 500.0, 400.0], |a| a.gradient = Some(gradient.clone())),
            path([0.0, 0.0, 50.0, 50.0], |a| {
                a.stroke_width = Some(Dimension::Number(10.0));
                a.stroke_pos = Some(Dimension::Number(1.0));
                a.stroke_color = Some(Color::Palette { index: FOREGROUND_INDEX, alpha: 1.0 });
            }),
            path([0.0, 0.0, 50.0, 50.0], |a| a.fill_color = Some(Color::Rgba([0.0, 0.0, 0.0, 0.0]))),
            path([0.0, 0.0, 50.0, 50.0], |_| {}),
            path([0.0, 0.0, 400.0, 300.0], |a| {
                a.fill_color = Some(Color::Rgba(RED));
                a.stroke_width = Some(Dimension::Number(4.0));
                a.line_join = Some(1);
            }),
        ];
        let (font, g) = glyph_with(|m| vec![colour_layer(m, "c", shapes)]);
        let paint = paint_of(&font, &g).unwrap().unwrap();
        let spec = |width: f64, position: f64, join: u8| StrokeSpec { width, height: width, position, cap_start: 0, cap_end: 0, join };
        let expected = Paint::Layers(vec![
            solid("c", vec![0, 1], None, ColorRef::Rgba(RED)),
            Paint::Glyph {
                shape: ShapeRef { layer: "c".into(), shapes: vec![2], stroke: None },
                paint: Box::new(Paint::Linear {
                    // Fractions of the path's box (100, 100)–(500, 400); the third point is the
                    // start-to-end vector turned −90° about the start.
                    p0: Point::new(200.0, 250.0),
                    p1: Point::new(400.0, 400.0),
                    p2: Point::new(350.0, 50.0),
                    line: ColorLine {
                        stops: vec![
                            ColorStop { offset: 0.0, color: ColorRef::Palette { index: 1, alpha: 0.5 } },
                            ColorStop { offset: 1.0, color: ColorRef::Palette { index: 0, alpha: 1.0 } },
                        ],
                        extend: Extend::Reflect,
                    },
                }),
            },
            // A stroke without a fill paints only its stroke, in the text colour here.
            solid("c", vec![3], Some(spec(10.0, 1.0, 0)), ColorRef::Foreground { alpha: 1.0 }),
            // The transparent fill paints nothing; a path without paint attributes is gray.
            solid("c", vec![5], None, ColorRef::Rgba([0.5, 0.5, 0.5, 1.0])),
            // A filled, stroked path: the fill, then its stroke above it in black.
            solid("c", vec![6], None, ColorRef::Rgba(RED)),
            solid("c", vec![6], Some(spec(4.0, 0.0, 1)), ColorRef::Rgba([0.0, 0.0, 0.0, 1.0])),
        ]);
        assert_eq!(paint, expected);
        assert_eq!(shape_refs(&paint).len(), 6, "one layer glyph per paint");
        assert!(!is_solid_stack(&paint), "a gradient needs COLR version 1");

        // A version 3 radial gradient (no radii) is a circle around its start out to the farthest
        // corner of the box.
        let radial = Gradient {
            kind: GradientKind::Radial { start: Point::new(0.25, 0.5), end: Point::new(0.25, 0.5), start_radius: 0.0, end_radius: 0.0, angle: 0.0 },
            stops: gradient.stops.clone(),
            extend: Extend::Pad,
        };
        let (font, g) = glyph_with(|m| vec![colour_layer(m, "c", vec![path([0.0, 0.0, 400.0, 300.0], |a| a.gradient = Some(radial))])]);
        let Some(Paint::Layers(list)) = paint_of(&font, &g).unwrap() else { panic!("one layer") };
        let Paint::Glyph { paint, .. } = &list[0] else { panic!("a glyph paint") };
        let Paint::Radial { c0, r0, c1, r1, .. } = paint.as_ref() else { panic!("a radial gradient: {paint:?}") };
        assert_eq!((*c0, *r0, *c1), (Point::new(100.0, 150.0), 0.0, Point::new(100.0, 150.0)));
        assert!((r1 - 300f64.hypot(150.0)).abs() < 1e-9, "{r1}");

        // A stroke width that is not a number is refused with the glyph's name.
        let (font, g) = glyph_with(|m| {
            vec![colour_layer(m, "c", vec![path([0.0, 0.0, 10.0, 10.0], |a| a.stroke_width = Some(Dimension::Expression("w * 2".into())))])]
        });
        assert!(matches!(paint_of(&font, &g), Err(ColorError::BadPaint { glyph, .. }) if glyph == "A"));
    }

    #[test]
    fn solid_stacks_fit_version_0_and_a_full_colour_master_layer_paints_first() {
        let opaque = Paint::Layers(vec![
            solid("a", vec![0], None, ColorRef::Rgba([0.1, 0.2, 0.3, 0.4])),
            solid("b", vec![0], None, ColorRef::Palette { index: 2, alpha: 1.0 }),
            solid("b", vec![0], None, ColorRef::Foreground { alpha: 1.0 }),
        ]);
        assert!(is_solid_stack(&opaque), "a colour's own alpha goes into CPAL");
        let translucent = Paint::Layers(vec![solid("b", vec![0], None, ColorRef::Palette { index: 2, alpha: 0.5 })]);
        assert!(!is_solid_stack(&translucent), "a palette reference's alpha needs version 1");

        let (font, mut g) = glyph_with(|m| {
            let mut palette = Layer::new(m, 600.0);
            palette.id = "p".into();
            palette.info.attrs.color_palette = Some(PaletteEntry::Index(0));
            palette.shapes.push(path([0.0, 0.0, 10.0, 10.0], |_| {}));
            vec![palette]
        });
        g.layers[0].info.attrs.full_color = true;
        g.layers[0].shapes.push(path([0.0, 0.0, 20.0, 20.0], |a| a.fill_color = Some(Color::Palette { index: 1, alpha: 1.0 })));
        let master = font.masters[0].id.clone();
        let paint = paint_of(&font, &g).unwrap().unwrap();
        assert_eq!(
            paint,
            Paint::Layers(vec![
                solid(&master, vec![0], None, ColorRef::Palette { index: 1, alpha: 1.0 }),
                solid("p", vec![0], None, ColorRef::Palette { index: 0, alpha: 1.0 }),
            ])
        );
        assert!(is_solid_stack(&paint));
    }

    #[test]
    fn paints_vary_only_in_their_numbers_and_instances_mix_paint_attributes() {
        let line = |alpha: f64, extend: Extend| ColorLine {
            stops: vec![
                ColorStop { offset: 0.0, color: ColorRef::Palette { index: 0, alpha } },
                ColorStop { offset: 1.0, color: ColorRef::Rgba([0.0, 0.0, 1.0, 1.0]) },
            ],
            extend,
        };
        let linear = |x: f64, alpha: f64, extend: Extend| {
            Paint::Layers(vec![Paint::Glyph {
                shape: ShapeRef { layer: format!("l{x}"), shapes: vec![0], stroke: None },
                paint: Box::new(Paint::Linear {
                    p0: Point::new(x, 0.0),
                    p1: Point::new(100.0, 0.0),
                    p2: Point::new(0.0, -100.0),
                    line: line(alpha, extend),
                }),
            }])
        };
        let default = linear(0.0, 1.0, Extend::Pad);
        // Another layer id and other numbers: the same structure.
        assert!(same_structure(&default, &linear(10.0, 0.5, Extend::Pad)));
        assert_eq!(numbers(&linear(10.0, 0.5, Extend::Pad)), [10.0, 0.0, 100.0, 0.0, 0.0, -100.0, 0.0, 0.5, 1.0]);
        assert!(!same_structure(&default, &linear(0.0, 1.0, Extend::Reflect)), "another extend mode");
        assert!(!same_structure(&default, &solid("l", vec![0], None, ColorRef::Palette { index: 0, alpha: 1.0 })));

        let mut variation = PaintVariation { locations: vec![vec![1.0]], ..PaintVariation::default() };
        variation.glyphs.insert("A".into(), vec![Some(linear(0.0, 1.0, Extend::Pad))]);
        assert!(!variation.varies("A", &default), "equal numbers do not vary");
        variation.glyphs.insert("A".into(), vec![Some(linear(10.0, 1.0, Extend::Pad))]);
        assert!(variation.varies("A", &default));
        assert!(!variation.varies("B", &default), "a glyph without sources");

        // Halfway between two masters: colours, gradient positions and stops are mixed; a shape
        // whose fill differs in kind keeps the heaviest master's.
        let gradient = |x: f64, alpha: f64| Gradient {
            kind: GradientKind::Linear { start: Point::new(x, 0.0), end: Point::new(1.0, 1.0) },
            stops: vec![
                Stop { offset: x, color: Color::Palette { index: 1, alpha } },
                Stop { offset: 1.0, color: Color::Rgba([1.0, 0.0, 0.0, 1.0]) },
            ],
            extend: Extend::Pad,
        };
        let master = |x: f64, alpha: f64, fill: Color| {
            vec![path([0.0, 0.0, 10.0, 10.0], |a| a.gradient = Some(gradient(x, alpha))), path([0.0, 0.0, 10.0, 10.0], |a| a.fill_color = Some(fill))]
        };
        let light = master(0.0, 1.0, Color::Rgba([0.0, 0.0, 0.0, 1.0]));
        let bold = master(0.5, 0.5, Color::Palette { index: 0, alpha: 1.0 });
        let mut out = bold.clone();
        interpolate_paint_attrs(&mut out, &[(&light, 0.5), (&bold, 0.5)]);
        let Shape::Path(p) = &out[0] else { panic!("a path") };
        assert_eq!(p.info.attrs.gradient, Some(gradient(0.25, 0.75)));
        let Shape::Path(p) = &out[1] else { panic!("a path") };
        assert_eq!(p.info.attrs.fill_color, Some(Color::Palette { index: 0, alpha: 1.0 }), "kinds differ: the heaviest master's");
    }

    #[test]
    fn components_of_colour_glyphs_paint_them_by_reference_without_cycles() {
        const RED: [f64; 4] = [1.0, 0.0, 0.0, 1.0];
        let mut font = Font::new("References");
        let master = font.masters[0].id.clone();
        let red = || path([0.0, 0.0, 400.0, 400.0], |a| a.fill_color = Some(Color::Rgba(RED)));
        let component = |glyph: &str, affine: [f64; 6]| {
            let mut c = Component::new(glyph, 0.0, 0.0);
            c.transform = fontcraft_model::Transform::Matrix(affine);
            Shape::Component(c)
        };
        let moved = [1.0, 0.0, 0.0, 1.0, 100.0, 50.0];
        let mut tinted = Component::new("A", 0.0, 0.0);
        tinted.info.attrs.fill_color = Some(Color::Rgba(RED));
        let layers: [(&str, Vec<Shape>); 7] = [
            ("A", vec![red()]),
            // A path, a reference, then a component that is no reference: its glyph has no
            // colour layers, so it merges with the path after it.
            ("B", vec![red(), component("A", moved), component("Z", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), path([0.0, 0.0, 9.0, 9.0], |_| {})]),
            ("C", vec![Shape::Component(tinted)]),
            ("D", vec![component("D", moved)]),
            ("E", vec![component("F", moved)]),
            ("F", vec![red(), component("E", moved)]),
            ("G", vec![component("H", moved)]),
        ];
        for (name, shapes) in layers {
            let mut g = font.glyph_mut(name).unwrap();
            g.layers.push(colour_layer(&master, &format!("{name}-c"), shapes));
        }
        let mut h = font.glyph_mut("H").unwrap();
        h.layers.push(colour_layer(&master, "H-c", vec![red()]));
        h.export = false;
        drop(h);
        let paint = |name: &str| {
            let mut warnings = Vec::new();
            let g = font.glyph(name).unwrap();
            let p = glyph_paint_warn(&font, g, &master, &Palettes::default(), &mut warnings).unwrap().unwrap();
            (p, warnings)
        };

        let (b, warnings) = paint("B");
        let grey = color_ref(DEFAULT_FILL);
        let want = Paint::Layers(vec![
            solid("B-c", vec![0], None, ColorRef::Rgba(RED)),
            Paint::ColrGlyph {
                glyph: "A".into(),
                transform: Affine::new(moved),
                shape: ShapeRef { layer: "B-c".into(), shapes: vec![1], stroke: None },
            },
            solid("B-c", vec![2, 3], None, grey),
        ]);
        assert_eq!(b, want);
        assert!(warnings.is_empty(), "{warnings:?}");
        // The reference takes number 1, so the layer glyphs are B.color0 and B.color2, as glyphsLib
        // numbers them.
        assert_eq!(numbered_shape_refs(&b).iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(colr_glyph_refs(&b), ["A"]);
        assert!(!is_solid_stack(&b));

        // A component with a fill colour of its own paints its outline.
        let (c, _) = paint("C");
        assert_eq!(c, Paint::Layers(vec![solid("C-c", vec![0], None, ColorRef::Rgba(RED))]));

        // A glyph that refers to itself, two that refer to each other and a glyph that does not
        // export are drawn as outlines, with a warning.
        for (name, why) in [("D", "lead back"), ("E", "lead back"), ("F", "lead back"), ("G", "does not export")] {
            let (p, warnings) = paint(name);
            assert!(colr_glyph_refs(&p).is_empty(), "{name}: {p:?}");
            assert!(warnings.iter().any(|w| w.contains(why)), "{name}: {warnings:?}");
        }
        let (d, _) = paint("D");
        assert_eq!(d, Paint::Layers(vec![solid("D-c", vec![0], None, grey)]));
    }

    #[test]
    fn references_through_a_shared_glyph_take_linear_work() {
        // `hub` refers to 5,000 colour glyphs and 5,000 glyphs refer to `hub`. Each reference
        // checks whether it leads back; walking the hub's references once per check is 25
        // million steps, once per master is 10,000.
        let mut font = Font::new("Hub");
        let master = font.masters[0].id.clone();
        let red = || path([0.0, 0.0, 400.0, 400.0], |a| a.fill_color = Some(Color::Rgba([1.0, 0.0, 0.0, 1.0])));
        let add = |font: &mut Font, name: &str, shapes: Vec<Shape>| {
            let mut g = Glyph::new(name);
            g.layers.push(colour_layer(&master, &format!("{name}-c"), shapes));
            font.glyphs.push(g);
        };
        let n = 5_000;
        for i in 0..n {
            add(&mut font, &format!("leaf{i}"), vec![red()]);
        }
        add(&mut font, "hub", (0..n).map(|i| Shape::Component(Component::new(format!("leaf{i}"), 0.0, 0.0))).collect());
        for i in 0..n {
            add(&mut font, &format!("user{i}"), vec![Shape::Component(Component::new("hub", 0.0, 0.0))]);
        }
        // A chain of 70 references: one that leads more than 64 deep is drawn as an outline.
        for i in 0..70 {
            add(&mut font, &format!("link{i}"), vec![Shape::Component(Component::new(format!("link{}", i + 1), 0.0, 0.0))]);
        }
        add(&mut font, "link70", vec![red()]);

        let start = std::time::Instant::now();
        let mut paints = MasterPaints::new(&font, &master);
        for i in 0..n {
            let mut warnings = Vec::new();
            let p = paints.paint(font.glyph(&format!("user{i}")).unwrap(), &mut warnings).unwrap().unwrap();
            assert_eq!(colr_glyph_refs(&p), ["hub"]);
            assert!(warnings.is_empty(), "{warnings:?}");
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "{:?}", start.elapsed());

        let mut warnings = Vec::new();
        let deep = paints.paint(font.glyph("link0").unwrap(), &mut warnings).unwrap().unwrap();
        assert!(colr_glyph_refs(&deep).is_empty(), "{deep:?}");
        assert!(warnings.iter().any(|w| w.contains("nest more than")), "{warnings:?}");
        let shallow = paints.paint(font.glyph("link10").unwrap(), &mut Vec::new()).unwrap().unwrap();
        assert_eq!(colr_glyph_refs(&shallow), ["link11"]);
    }
}
