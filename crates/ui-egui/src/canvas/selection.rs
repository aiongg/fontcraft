//! The selection in Edit View, and what the pointer tools ask the canvas to draw.
//!
//! | Item | Drawn as |
//! |---|---|
//! | selected node or handle | ring around it, `selection` |
//! | selected segment (both ends, or one of its handles, selected) | the segment over the outline, `selection` |
//! | marquee, Zoom tool rectangle | filled rectangle with an edge, `marquee_fill` and `marquee_stroke` |
//! | lasso | closed line, `marquee_stroke` |
//! | snapping line, snap target | line and ring, `snap_line` |
//! | point under the pointer | ring, `hover_point`; `locked_point` when it cannot be moved |
//!
//! The handbook does not say how a selection looks; these are our choices.

use egui::{Pos2, Shape, Stroke};
use fontcraft_geom::{Point, segments};
use fontcraft_model::Layer;
use fontcraft_tools::{Overlay, OverlayKind, Selection, View};

use crate::theme::Tokens;

/// Points sampled along each piece of a selected segment.
const SEGMENT_SAMPLES: usize = 24;

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The selected segments, drawn under the nodes.
pub fn segment_shapes(layer: &Layer, selection: &Selection, view: View, tokens: &Tokens, focus: Option<&[usize]>) -> Vec<Shape> {
    let mut out = Vec::new();
    if selection.points.is_empty() {
        return out;
    }
    let stroke = Stroke::new(tokens.selection_width, tokens.selection);
    for (index, shape) in layer.shapes.iter().enumerate() {
        let fontcraft_model::Shape::Path(path) = shape else { continue };
        if focus.is_some_and(|f| !f.contains(&index)) {
            continue;
        }
        let selected = |n: usize| selection.points.contains(&(index, n));
        for segment in segments(&path.node_points(), path.closed) {
            if !(selected(segment.start) && selected(segment.end) || segment.handles.iter().any(|&h| selected(h))) {
                continue;
            }
            let mut line: Vec<Pos2> = Vec::with_capacity(segment.pieces.len() * SEGMENT_SAMPLES + 1);
            for piece in &segment.pieces {
                line.extend((0..=SEGMENT_SAMPLES).map(|k| pos(view.to_screen(piece.eval(k as f64 / SEGMENT_SAMPLES as f64)))));
            }
            if line.len() > 1 && line.iter().all(|p| usable(*p)) {
                out.push(Shape::line(line, stroke));
            }
        }
    }
    out
}

/// Rings around the selected nodes and handles, drawn over them.
pub fn point_shapes(layer: &Layer, selection: &Selection, view: View, tokens: &Tokens, focus: Option<&[usize]>) -> Vec<Shape> {
    let stroke = Stroke::new(tokens.selection_width, tokens.selection);
    selection
        .points
        .iter()
        .filter(|(s, _)| focus.is_none_or(|f| f.contains(s)))
        .filter_map(|&(s, n)| match layer.shapes.get(s)? {
            fontcraft_model::Shape::Path(p) => p.nodes.get(n),
            _ => None,
        })
        .map(|node| pos(view.to_screen(node.point())))
        .filter(|p| usable(*p))
        .map(|at| Shape::circle_stroke(at, tokens.selection_ring_radius, stroke))
        .collect()
}

/// The shapes for the active tool's overlays.
pub fn overlay_shapes(overlays: &[Overlay], view: View, tokens: &Tokens) -> Vec<Shape> {
    let mut out = Vec::new();
    for overlay in overlays {
        match overlay {
            Overlay::Polygon { kind, points } => {
                let screen: Vec<Pos2> = points.iter().map(|p| pos(view.to_screen(*p))).collect();
                if screen.len() < 2 || !screen.iter().all(|p| usable(*p)) {
                    continue;
                }
                let edge = Stroke::new(1.0, tokens.marquee_stroke);
                match kind {
                    // A marquee and a zoom rectangle are convex; a lasso need not be.
                    OverlayKind::Marquee | OverlayKind::ZoomRect => out.push(Shape::convex_polygon(screen, tokens.marquee_fill, edge)),
                    _ => out.push(Shape::closed_line(screen, edge)),
                }
            }
            Overlay::Line { kind, from, to } => {
                let (a, b) = (pos(view.to_screen(*from)), pos(view.to_screen(*to)));
                if usable(a) && usable(b) {
                    let stroke = match kind {
                        OverlayKind::KnifeLine => Stroke::new(tokens.selection_width, tokens.knife_line),
                        _ => Stroke::new(tokens.snap_line_width, tokens.snap_line),
                    };
                    out.push(Shape::line_segment([a, b], stroke));
                }
            }
            Overlay::Marker { kind, at } => {
                let at = pos(view.to_screen(*at));
                if !usable(at) {
                    continue;
                }
                let color = match kind {
                    OverlayKind::SnapTarget | OverlayKind::SnapLine => tokens.snap_line,
                    OverlayKind::HoverLocked => tokens.locked_point,
                    _ => tokens.hover_point,
                };
                out.push(Shape::circle_stroke(at, tokens.marker_radius, Stroke::new(tokens.selection_width, color)));
            }
        }
    }
    out
}
