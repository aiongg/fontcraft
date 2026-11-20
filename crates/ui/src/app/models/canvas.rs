//! The canvas's view model: the edited glyph in font units, resolved once per engine revision.
//!
//! Composites, component outlines and states, the metric lines the glyph's scope leaves, the background and the
//! crossings of Extra Nodes take boolean operations and font lookups, so the projection does them; the paint layers
//! only map the result through the camera.

use std::collections::HashSet;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::cmd::stroke::{format_version, layer_outline, mask_mode, shapes_outline, stroke_style};
use fontcraft_engine::components::align::{AlignMode, component_alignment};
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::{Font, Glyph, Layer, LayerSide, Master, MetricDef, Shape, metric};
use fontcraft_engine::pathops::geometry_helpers::segment_crossings;
use fontcraft_geom::{BezPath, PathSeg, Piece, Point};

use super::ProjectCx;
use crate::shell::masters;

/// Most glyphs the circular-reference check visits for one component.
const MAX_REFERENCE_VISITS: usize = 4096;
/// Most components resolved one by one; past this the layer's outline alone shows them.
const MAX_COMPONENTS: usize = 2000;
/// Most outline elements kept for one background or other layer.
const MAX_ELEMENTS: usize = 200_000;
/// Most segments compared for Extra Nodes; a layer with more shows none.
const MAX_CROSSING_SEGMENTS: usize = 4000;
/// Two crossings closer than this, in font units, are one Extra Node.
const SAME_CROSSING: f64 = 1e-6;

/// What the canvas draws; `glyph` is `None` without an open font or glyph.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CanvasModel {
    pub glyph: Option<EditedGlyph>,
    /// The edited glyph in colour (`canvas::color`).
    pub color: Option<crate::canvas::color::ColorPicture>,
}

/// The edited glyph in its active layer, in font units.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct EditedGlyph {
    pub name: String,
    pub master: String,
    pub width: f64,
    /// The master's descender and ascender: the document's bottom and top.
    pub descender: f64,
    pub ascender: f64,
    /// The horizontal metric lines the glyph shows: name and height.
    pub lines: Vec<(String, f64)>,
    /// Alignment zones as (bottom, top).
    pub zones: Vec<(f64, f64)>,
    /// The edited side of the layer, as a layer: the background's shapes while the background is edited.
    #[plain_data(assume = "a model value: owned data, no interior mutability")]
    pub layer: Layer,
    /// The composite outline (strokes expanded, masks applied); the plain outline while the background is edited.
    #[plain_data(assume = "a kurbo path: owned points")]
    pub outline: BezPath,
    /// The paths as drawn, when strokes or masks make the composite differ from them.
    #[plain_data(assume = "a kurbo path: owned points")]
    pub source: Option<BezPath>,
    pub components: Vec<ComponentMark>,
    /// The other side of the layer: its outline, and its shapes for Show Nodes › In Background.
    #[plain_data(assume = "a model value and a kurbo path: owned data")]
    pub background: Option<(BezPath, Layer)>,
    /// The outlines of the glyph's other layers whose eye is on.
    #[plain_data(assume = "kurbo paths: owned points")]
    pub other_layers: Vec<BezPath>,
    /// Where paths of the edited side cross: Extra Nodes.
    pub extra_nodes: Vec<[f64; 2]>,
    /// The highlight bands of the corner and cap components: the parts of their host paths that fitting them put in.
    #[plain_data(assume = "kurbo paths: owned points")]
    pub attachments: Vec<BezPath>,
    /// The colour (RGBA from 0 to 1) of the status the glyph is in: its stored status, else the font's default.
    pub status: Option<[f64; 4]>,
}

/// One component of the edited side.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct ComponentMark {
    pub base: String,
    pub state: State,
    /// Its outline in the edited glyph's units; empty for a placeholder.
    #[plain_data(assume = "a kurbo path: owned points")]
    pub outline: BezPath,
    /// A placeholder's box corners in the edited glyph's units: bottom left, bottom right, top right, top left.
    pub placeholder: Option<[[f64; 2]; 4]>,
}

/// Why a component cannot be drawn from its base glyph.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placeholder {
    /// The base glyph's layer has no outline.
    Empty,
    /// No glyph has the base glyph's name.
    Missing,
    /// The base glyph contains, directly or through nesting, the glyph the component is in.
    Circular,
}

/// How a component is shown (`components.autoAlignColors`).
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Aligned,
    /// Aligned horizontally only.
    Shiftable,
    Manual,
    Placeholder(Placeholder),
}

pub fn project(model: &mut Tracked<CanvasModel>, cx: &mut ProjectCx<'_>) {
    let glyph = super::glyph::shown_glyph(cx);
    let side = if cx.view.edit_background { LayerSide::Background } else { LayerSide::Foreground };
    let data = cx.session.glyph_data().ok();
    let edited = cx.session.font().ok().and_then(|font| Some((font, font.glyph(glyph.as_deref()?)?))).and_then(|(font, glyph)| {
        let layer = masters::shown_layer(font, &cx.view.layers, glyph)?;
        let master = masters::layer_master(font, &cx.view.layers, layer)?;
        edited_glyph(font, data.as_deref(), glyph, layer, master, side)
    });
    super::set(model, CanvasModel { glyph: edited, color: crate::canvas::color::project(cx) });
}

/// The canvas's picture of `glyph`'s layer `full`, editing its `side`, with the metric lines and zones of `master`.
pub fn edited_glyph(font: &Font, data: Option<&GlyphData>, glyph: &Glyph, full: &Layer, master: &Master, side: LayerSide) -> Option<EditedGlyph> {
    let layer = fontcraft_tools::side_view(full, side).into_owned();
    let (descender, ascender) = vertical_span(font, master);
    let hidden = data.map(|d| out_of_scope(font, d, glyph, &master.id)).unwrap_or_default();
    let (lines, zones) = metric_lines(font, master, &hidden);
    let plain = shapes_outline(font, &layer.shapes, &layer.master).map(concat).unwrap_or_else(|_| resolved(font, &layer.shapes, &layer.master));
    let (outline, source) = if side == LayerSide::Background {
        (plain, None)
    } else {
        match layer_outline(font, &layer) {
            Ok(parts) if styled(&layer, format_version(font)) => (concat(parts), Some(plain)),
            Ok(parts) => (concat(parts), None),
            Err(_) => (plain, None),
        }
    };
    let components = layer
        .shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, Shape::Component(_)))
        .take(MAX_COMPONENTS)
        .filter_map(|(i, _)| component_mark(font, &glyph.name, &layer, i))
        .collect();
    let background = layer.info.background.as_ref().map(|bg| {
        let shown = Layer { shapes: bg.shapes.clone(), ..Layer::new(layer.master.clone(), layer.width) };
        (resolved(font, &bg.shapes, &layer.master), shown)
    });
    let other_layers = glyph.layers.iter().filter(|l| l.id != full.id && l.info.visible).map(|l| resolved(font, &l.shapes, &l.master)).collect();
    Some(EditedGlyph {
        name: glyph.name.clone(),
        master: master.id.clone(),
        width: layer.width,
        descender,
        ascender,
        lines,
        zones,
        extra_nodes: extra_nodes(&layer),
        attachments: super::components::attachment_bands(font, &layer),
        status: fontcraft_engine::cmd::status::effective_status(font, &glyph.name).map(|def| def.color),
        layer,
        outline,
        source,
        components,
        background,
        other_layers,
    })
}

fn concat(parts: Vec<BezPath>) -> BezPath {
    parts.into_iter().fold(BezPath::new(), |mut all, p| {
        all.extend(p.elements().iter().copied());
        all
    })
}

/// The outline of `shapes` on `master`, components resolved. Shapes that cannot be resolved (a missing component
/// glyph, a cycle, bad coordinates) are left out.
fn resolved(font: &Font, shapes: &[Shape], master: &str) -> BezPath {
    let mut out = BezPath::new();
    for shape in shapes {
        let part = match shape {
            Shape::Path(p) => p.to_bez().ok(),
            Shape::Component(c) => font.component_outline(c, master).ok(),
            _ => None,
        };
        let Some(part) = part else { continue };
        if out.elements().len().saturating_add(part.elements().len()) > MAX_ELEMENTS {
            break;
        }
        out.extend(part);
    }
    out
}

/// Whether `layer` has a stroked path or a mask shape.
fn styled(layer: &Layer, version: u32) -> bool {
    layer.shapes.iter().any(|shape| match shape {
        Shape::Path(p) => mask_mode(&p.info.attrs, version).is_some() || !matches!(stroke_style(&p.info.attrs), Ok(None)),
        Shape::Component(c) => mask_mode(&c.info.attrs, version).is_some(),
        Shape::Image(_) | Shape::Raw(_) => false,
    })
}

/// Descender and ascender of a master, in font units.
pub fn vertical_span(font: &Font, master: &Master) -> (f64, f64) {
    let upm = f64::from(font.units_per_em);
    let ascender = font.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let descender = font.metric_pos(master, metric::DESCENDER, -upm * 0.2);
    if ascender > descender { (descender, ascender) } else { (-upm * 0.2, upm * 0.8) }
}

/// Named metric lines and (bottom, top) zones.
type MetricLines = (Vec<(String, f64)>, Vec<(f64, f64)>);

/// The master's horizontal metric lines and alignment zones, without the metrics at `hidden`. The zones are read as
/// `Font::alignment_zones` reads them: each metric's overshoot, then the master's own.
fn metric_lines(font: &Font, master: &Master, hidden: &[usize]) -> MetricLines {
    let defs = || font.metrics.iter().zip(master.metric_values.iter()).enumerate().filter(|(i, _)| !hidden.contains(i)).map(|(_, dv)| dv);
    let lines = defs()
        .filter(|(d, v)| d.kind != metric::ITALIC_ANGLE && v.pos.is_finite())
        .map(|(d, v)| (d.name.clone().unwrap_or_else(|| d.kind.clone()), v.pos))
        .collect();
    let zones = defs()
        .filter(|(d, v)| d.kind != metric::ITALIC_ANGLE && v.over != 0.0)
        .map(|(_, v)| (v.pos, v.over))
        .chain(master.zones.iter().map(|z| (z.pos, z.size)))
        .filter(|(pos, size)| pos.is_finite() && size.is_finite())
        .map(|(pos, size)| if size >= 0.0 { (pos, pos + size) } else { (pos + size, pos) })
        .collect();
    (lines, zones)
}

/// The indexes of the font's metrics whose glyph scope (`fontInfo.metrics.scopeHidesLine`) leaves out `glyph`. A
/// scope that does not read as a predicate shows its metric.
pub fn out_of_scope(font: &Font, data: &GlyphData, glyph: &Glyph, master: &str) -> Vec<usize> {
    if font.metrics.iter().all(|d| scope(d).is_none()) {
        return Vec::new();
    }
    let Ok(env) = fontcraft_glyphquery::Env::new(font, data, Some(master)) else { return Vec::new() };
    font.metrics
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            let Some(text) = scope(d) else { return false };
            let Ok(expr) = fontcraft_glyphquery::parse_predicate(text) else { return false };
            !fontcraft_glyphquery::guide_in_scope(Some(&expr), glyph, &env)
        })
        .map(|(i, _)| i)
        .collect()
}

fn scope(def: &MetricDef) -> Option<&str> {
    def.filter.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// The state of the component at shape index `shape` of `glyph`'s `layer`; `None` when that shape is not a
/// component.
pub fn state(font: &Font, glyph: &str, layer: &Layer, shape: usize) -> Option<State> {
    let Some(Shape::Component(c)) = layer.shapes.get(shape) else { return None };
    if font.glyph(&c.glyph).is_none() {
        return Some(State::Placeholder(Placeholder::Missing));
    }
    if reaches(font, &c.glyph, glyph, &layer.master) {
        return Some(State::Placeholder(Placeholder::Circular));
    }
    match font.component_outline(c, &layer.master) {
        Ok(outline) if outline.elements().is_empty() => return Some(State::Placeholder(Placeholder::Empty)),
        Ok(_) => {}
        // Any other failure (a cycle further down, an outline past the limits) is a bad reference.
        Err(_) => return Some(State::Placeholder(Placeholder::Circular)),
    }
    Some(match component_alignment(font, layer, shape) {
        Some(AlignMode::Full) => State::Aligned,
        Some(AlignMode::Horizontal) => State::Shiftable,
        None => State::Manual,
    })
}

/// Whether `from`'s layer of `master`, through its components, contains `target` or loops.
fn reaches(font: &Font, from: &str, target: &str, master: &str) -> bool {
    let mut stack = vec![from.to_string()];
    let mut seen = HashSet::new();
    while let Some(name) = stack.pop() {
        if name == target || seen.len() >= MAX_REFERENCE_VISITS {
            return true;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(layer) = font.glyph(&name).and_then(|g| g.layer(master)) else { continue };
        for shape in &layer.shapes {
            if let Shape::Component(c) = shape {
                stack.push(c.glyph.clone());
            }
        }
    }
    false
}

fn component_mark(font: &Font, glyph: &str, layer: &Layer, shape: usize) -> Option<ComponentMark> {
    let Some(Shape::Component(component)) = layer.shapes.get(shape) else { return None };
    let state = state(font, glyph, layer, shape)?;
    let base = component.glyph.clone();
    if let State::Placeholder(_) = state {
        // A box as wide as the base glyph's advance (half an em without one) and half an em tall.
        let em = f64::from(font.units_per_em);
        let advance = font.glyph(&base).and_then(|g| g.layer(&layer.master)).map_or(0.0, |l| l.width);
        let w = if advance.is_finite() && advance > 0.0 { advance } else { em / 2.0 };
        let h = em / 2.0;
        let affine = component.affine();
        let corner = |x: f64, y: f64| {
            let p = affine * Point::new(x, y);
            [p.x, p.y]
        };
        let placeholder = Some([corner(0.0, 0.0), corner(w, 0.0), corner(w, h), corner(0.0, h)]);
        return Some(ComponentMark { base, state, outline: BezPath::new(), placeholder });
    }
    let outline = font.component_outline(component, &layer.master).ok()?;
    Some(ComponentMark { base, state, outline, placeholder: None })
}

/// Where the paths of `layer` cross one another or themselves, away from their nodes: Extra Nodes.
pub fn extra_nodes(layer: &Layer) -> Vec<[f64; 2]> {
    let mut segments: Vec<Vec<Piece>> = Vec::new();
    for shape in &layer.shapes {
        let Shape::Path(path) = shape else { continue };
        let Ok(bez) = path.to_bez() else { continue };
        for seg in bez.segments() {
            if segments.len() >= MAX_CROSSING_SEGMENTS {
                return Vec::new();
            }
            segments.push(vec![match seg {
                PathSeg::Line(l) => Piece::Line(l),
                PathSeg::Quad(q) => Piece::Quad(q),
                PathSeg::Cubic(c) => Piece::Cubic(c),
            }]);
        }
    }
    let Ok(crossings) = segment_crossings(&segments) else { return Vec::new() };
    let mut out: Vec<[f64; 2]> = Vec::new();
    for (pieces, ts) in segments.iter().zip(&crossings) {
        let Some(piece) = pieces.first() else { continue };
        // A crossing at a segment's end is one of its nodes.
        for t in ts.iter().filter(|t| **t > SAME_CROSSING && **t < 1.0 - SAME_CROSSING) {
            let p = piece.eval(*t);
            if !out.iter().any(|q| (q[0] - p.x).abs() <= SAME_CROSSING * 1e3 && (q[1] - p.y).abs() <= SAME_CROSSING * 1e3) {
                out.push([p.x, p.y]);
            }
        }
    }
    out
}
