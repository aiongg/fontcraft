//! The components and corner components of the edited layer, over the glyph's outline.
//!
//! | Item | Drawn as |
//! |---|---|
//! | component aligned automatically | its outline in `component_aligned` (`components.autoAlignColors`) |
//! | component aligned horizontally only (the single Number component, or the horizontal setting) | its outline in `component_shiftable` |
//! | component placed by hand | its outline in `component_manual` |
//! | selected component | its outline in `selection`, wider |
//! | empty base layer, missing base glyph, circular reference | a crossed box with a label, `component_placeholder` (`components.placeholder*`) |
//! | nodes inside components while a tool drags | small rings, `component_node` (`components.nodesInsideComponentsSnap`) |
//! | corner component | a faint disc on its node, `corner_highlight` (`corner.highlightAndSelection`) |
//! | smart handle of a selected smart component | a grey dot, `smart_handle` (`smart.handlesFromPropertyAnchors`) |
//!
//! Outlines are resolved every frame from the font, so composites follow a base glyph while it is
//! being edited (`components.liveUpdateFromBaseGlyph`).

use std::collections::HashSet;

use egui::{Align2, FontId, Pos2, Shape, Stroke};
use fontcraft_engine::components::align::{AlignMode, component_alignment};
use fontcraft_engine::model::corner::{CornerTarget, corner_hint, is_corner};
use fontcraft_engine::model::smart::component_anchors;
use fontcraft_engine::model::{Font, Layer, Shape as ModelShape, SmartValue};
use fontcraft_geom::Shape as _;
use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_tools::{Selection, SmartAxis, SmartHandle, SmartSize};

use super::{Painting, flatten};

/// Most glyphs the circular-reference check visits for one component.
const MAX_REFERENCE_VISITS: usize = 4096;
/// Most components drawn one by one; past this the layer's outline alone shows them.
const MAX_DRAWN_COMPONENTS: usize = 2000;
/// Most selected components whose smart handles are resolved; each costs three interpolations
/// per property.
const MAX_HANDLE_COMPONENTS: usize = 64;
/// Two handle positions closer than this, in font units, count as one: the anchor does not move
/// with the property.
const SAME_POSITION: f64 = 1e-6;

/// Why a component cannot be drawn from its base glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placeholder {
    /// The base glyph's layer has no outline.
    Empty,
    /// No glyph has the base glyph's name.
    Missing,
    /// The base glyph contains, directly or through nesting, the glyph the component is in.
    Circular,
}

/// How a component is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Aligned,
    Shiftable,
    Manual,
    Placeholder(Placeholder),
}

/// The state of the component at shape index `shape` of `glyph`'s `layer`; `None` when that
/// shape is not a component.
pub fn state(font: &Font, glyph: &str, layer: &Layer, shape: usize) -> Option<State> {
    let Some(ModelShape::Component(c)) = layer.shapes.get(shape) else { return None };
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
            if let ModelShape::Component(c) = shape {
                stack.push(c.glyph.clone());
            }
        }
    }
    false
}

/// Draws the components of `glyph`'s `layer` and its corner components. `dragging` shows the
/// nodes inside the components as snap targets.
pub fn draw(c: &Painting, font: &Font, glyph: &str, layer: &Layer, selection: &Selection, dragging: bool) {
    let t = c.tokens;
    let to_screen = Affine::translate(c.view.origin.to_vec2()) * Affine::scale_non_uniform(c.view.scale(), -c.view.scale());
    let components = layer.shapes.iter().enumerate().filter(|(_, s)| matches!(s, ModelShape::Component(_))).take(MAX_DRAWN_COMPONENTS);
    for (i, shape) in components {
        let ModelShape::Component(component) = shape else { continue };
        let Some(state) = state(font, glyph, layer, i) else { continue };
        let selected = selection.shapes.contains(&i);
        let color = match state {
            _ if selected => t.selection,
            State::Aligned => t.component_aligned,
            State::Shiftable => t.component_shiftable,
            State::Manual => t.component_manual,
            State::Placeholder(_) => t.component_placeholder,
        };
        let width = if selected { t.component_outline_width * 1.5 } else { t.component_outline_width };
        if let State::Placeholder(kind) = state {
            placeholder(c, font, (&component.glyph, &layer.master), component.affine(), kind, Stroke::new(width, color));
            continue;
        }
        let Ok(outline) = font.component_outline(component, &layer.master) else { continue };
        for contour in flatten(&(to_screen * outline.clone())) {
            c.painter.add(Shape::line(contour, Stroke::new(width, color)));
        }
        if dragging {
            let ring = Stroke::new(1.0, t.component_node);
            for p in on_curve(&outline) {
                let at = pos(c.view.to_screen(p));
                if at.x.is_finite() && at.y.is_finite() {
                    c.painter.add(Shape::circle_stroke(at, t.handle_radius, ring));
                }
            }
        }
    }
    corners(c, layer);
    for h in smart_handles(font, layer, selection) {
        let at = pos(c.view.to_screen(h.at));
        if at.x.is_finite() && at.y.is_finite() {
            c.painter.add(Shape::circle_filled(at, t.anchor_size / 2.0, t.smart_handle));
        }
    }
}

/// The selected smart components of `layer` with their `Width` and `Height` properties (the
/// names matched exactly) and the component's extent at each end of them.
pub fn smart_sizes(font: &Font, layer: &Layer, selection: &Selection) -> Vec<SmartSize> {
    let mut out = Vec::new();
    for &shape in selection.shapes.iter().take(MAX_HANDLE_COMPONENTS) {
        let Some(ModelShape::Component(c)) = layer.shapes.get(shape) else { continue };
        let Some(base) = font.glyph(&c.glyph) else { continue };
        if base.info.parts.is_empty() {
            continue;
        }
        let axis = |name: &str, horizontal: bool| -> Option<SmartAxis> {
            let part = base.info.parts.iter().find(|p| p.name == name)?;
            let (min, max) = (part.bottom_value.min(part.top_value), part.bottom_value.max(part.top_value));
            let size = |value: f64| -> Option<f64> {
                let mut probe = c.clone();
                probe.info.smart_values.retain(|v| v.property != name);
                probe.info.smart_values.push(SmartValue { property: name.to_string(), value });
                let b = font.component_outline(&probe, &layer.master).ok()?.bounding_box();
                Some(if horizontal { b.width() } else { b.height() })
            };
            Some(SmartAxis { property: name.to_string(), min, max, size_at_min: size(min)?, size_at_max: size(max)? })
        };
        out.push(SmartSize { shape, width: axis("Width", true), height: axis("Height", false) });
    }
    out
}

/// The smart handles of the selected components of `layer`: for each property of a component's
/// smart glyph, the anchor of the same name (case included) when it moves between the
/// property's two ends.
pub fn smart_handles(font: &Font, layer: &Layer, selection: &Selection) -> Vec<SmartHandle> {
    let mut out = Vec::new();
    for &shape in selection.shapes.iter().take(MAX_HANDLE_COMPONENTS) {
        let Some(ModelShape::Component(c)) = layer.shapes.get(shape) else { continue };
        let Some(base) = font.glyph(&c.glyph) else { continue };
        for part in &base.info.parts {
            let anchor_at = |value: Option<f64>| -> Option<Point> {
                let mut probe = c.clone();
                if let Some(value) = value {
                    probe.info.smart_values.retain(|v| v.property != part.name);
                    probe.info.smart_values.push(SmartValue { property: part.name.clone(), value });
                }
                let anchors = component_anchors(font, &probe, &layer.master).ok()??;
                let a = anchors.iter().find(|a| a.name == part.name)?;
                Some(c.affine() * Point::new(a.x, a.y))
            };
            let (min, max) = (part.bottom_value.min(part.top_value), part.bottom_value.max(part.top_value));
            let (Some(at), Some(at_min), Some(at_max)) = (anchor_at(None), anchor_at(Some(min)), anchor_at(Some(max))) else { continue };
            if (at_max - at_min).hypot() < SAME_POSITION {
                continue;
            }
            out.push(SmartHandle { shape, property: part.name.clone(), at, at_min, at_max, min, max });
        }
    }
    out
}

/// A crossed box where the base glyph would be: its advance wide when it has one, half an em
/// otherwise, half an em tall, placed by the component's transform, with the problem named.
fn placeholder(c: &Painting, font: &Font, (base, master): (&str, &str), affine: Affine, kind: Placeholder, stroke: Stroke) {
    let em = f64::from(font.units_per_em);
    let advance = font.glyph(base).and_then(|g| g.layer(master)).map_or(0.0, |l| l.width);
    let w = if advance.is_finite() && advance > 0.0 { advance } else { em / 2.0 };
    let h = em / 2.0;
    let corners = [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)].map(|p| pos(c.view.to_screen(affine * p)));
    if !corners.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
        return;
    }
    let [a, b, d, e] = corners;
    c.painter.add(Shape::closed_line(corners.to_vec(), stroke));
    c.painter.add(Shape::line_segment([a, d], stroke));
    c.painter.add(Shape::line_segment([b, e], stroke));
    let label = match kind {
        Placeholder::Empty => format!("{base}: empty"),
        Placeholder::Missing => format!("{base}: no such glyph"),
        Placeholder::Circular => format!("{base}: circular reference"),
    };
    c.painter.text(e, Align2::LEFT_BOTTOM, label, FontId::proportional(c.tokens.metric_name_size), stroke.color);
}

/// A faint disc on the node of each corner component on a node.
fn corners(c: &Painting, layer: &Layer) {
    for hint in layer.info.hints.iter().filter(|h| is_corner(h)) {
        let Ok(Some(corner)) = corner_hint(hint) else { continue };
        let CornerTarget::Node { shape, node } = corner.target else { continue };
        let Some(ModelShape::Path(path)) = layer.shapes.get(shape) else { continue };
        // The hint counts a closed path's nodes with the start node last.
        let len = path.nodes.len();
        let index = if path.closed && len > 0 { (node + 1) % len } else { node };
        let Some(n) = path.nodes.get(index) else { continue };
        let at = pos(c.view.to_screen(n.point()));
        if at.x.is_finite() && at.y.is_finite() {
            c.painter.add(Shape::circle_filled(at, c.tokens.corner_highlight_radius, c.tokens.corner_highlight));
        }
    }
}

fn on_curve(path: &BezPath) -> Vec<Point> {
    path.elements()
        .iter()
        .filter_map(|el| match el {
            fontcraft_geom::PathEl::MoveTo(p)
            | fontcraft_geom::PathEl::LineTo(p)
            | fontcraft_geom::PathEl::QuadTo(_, p)
            | fontcraft_geom::PathEl::CurveTo(_, _, p) => Some(*p),
            fontcraft_geom::PathEl::ClosePath => None,
        })
        .collect()
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}
