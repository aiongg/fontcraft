//! The selection and what the active tool asks the canvas to draw, as paint layers.
//!
//! | Item | Drawn as |
//! |---|---|
//! | selected segment (both ends, or one of its handles, selected) | the segment over the outline, under the nodes, `selection` |
//! | selected node or handle | ring around it, over the nodes, `selection` |
//! | marquee, Zoom tool rectangle | filled rectangle with an edge, `marquee_fill` and `marquee_stroke` |
//! | lasso | closed line, `marquee_stroke` |
//! | snapping line, snap target | line and ring, `snap_line` |
//! | point under the pointer | ring, `hover_point`; `locked_point` when it cannot be moved |
//! | knife line, measurement ruler, drawing preview | line, `knife_line`, `measure_ruler`, `draw_preview` |
//! | handle a drawing tool pulls out | handle line and dot, as the outline's handles |

use egui::{Pos2, Shape, Stroke};
use fontcraft_engine::model::Shape as ModelShape;
use fontcraft_geom::{Point, segments as path_segments};
use fontcraft_tools::{Overlay, OverlayKind};

use super::tokens::ToolTokens;
use crate::canvas::layers::PaintCx;

/// Points sampled along each piece of a selected segment.
const SEGMENT_SAMPLES: usize = 24;

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn shown(cx: &PaintCx<'_>, shape: usize) -> bool {
    cx.tools.focus.as_ref().is_none_or(|f| f.contains(&shape))
}

/// The selected segments, under the nodes.
pub fn segments(cx: &PaintCx<'_>) {
    cx.painter.extend(segment_shapes(cx));
}

pub fn segment_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let mut out = Vec::new();
    let selection = &cx.tools.selection;
    let Some(glyph) = cx.glyph() else { return out };
    if selection.points.is_empty() || !cx.display.nodes {
        return out;
    }
    let t = ToolTokens::standard();
    let stroke = Stroke::new(t.selection_width, t.selection);
    for (index, shape) in glyph.layer.shapes.iter().enumerate() {
        let ModelShape::Path(path) = shape else { continue };
        if !shown(cx, index) {
            continue;
        }
        let selected = |n: usize| selection.points.contains(&(index, n));
        for segment in path_segments(&path.node_points(), path.closed) {
            if !(selected(segment.start) && selected(segment.end) || segment.handles.iter().any(|&h| selected(h))) {
                continue;
            }
            let mut line: Vec<Pos2> = Vec::with_capacity(segment.pieces.len() * (SEGMENT_SAMPLES + 1));
            for piece in &segment.pieces {
                line.extend((0..=SEGMENT_SAMPLES).map(|k| cx.camera.to_screen(piece.eval(k as f64 / SEGMENT_SAMPLES as f64))));
            }
            if line.len() > 1 && line.iter().all(|p| usable(*p)) {
                out.push(Shape::line(line, stroke));
            }
        }
    }
    out
}

/// Rings around the selected nodes and handles, over them.
pub fn selection(cx: &PaintCx<'_>) {
    cx.painter.extend(selection_shapes(cx));
}

pub fn selection_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(glyph) = cx.glyph() else { return Vec::new() };
    if !cx.display.nodes {
        return Vec::new();
    }
    let t = ToolTokens::standard();
    let stroke = Stroke::new(t.selection_width, t.selection);
    cx.tools
        .selection
        .points
        .iter()
        .filter(|(s, _)| shown(cx, *s))
        .filter_map(|&(s, n)| match glyph.layer.shapes.get(s)? {
            ModelShape::Path(p) => p.nodes.get(n),
            _ => None,
        })
        .map(|node| cx.camera.to_screen(node.point()))
        .filter(|p| usable(*p))
        .map(|at| Shape::circle_stroke(at, t.selection_ring_radius, stroke))
        .collect()
}

/// The active tool's overlays, over everything else.
pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(overlay_shapes(cx));
}

pub fn overlay_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let t = ToolTokens::standard();
    let to_screen = |p: Point| cx.camera.to_screen(p);
    let mut out = Vec::new();
    for overlay in &cx.tools.overlays {
        match overlay {
            Overlay::Polygon { kind, points } => {
                let screen: Vec<Pos2> = points.iter().map(|p| to_screen(*p)).collect();
                if screen.len() < 2 || !screen.iter().all(|p| usable(*p)) {
                    continue;
                }
                let edge = Stroke::new(t.snap_line_width, t.marquee_stroke);
                match kind {
                    // A marquee and a zoom rectangle are convex; a lasso need not be.
                    OverlayKind::Marquee | OverlayKind::ZoomRect => out.push(Shape::convex_polygon(screen, t.marquee_fill, edge)),
                    _ => out.push(Shape::closed_line(screen, edge)),
                }
            }
            Overlay::Line { kind, from, to } => {
                let (a, b) = (to_screen(*from), to_screen(*to));
                if !(usable(a) && usable(b)) {
                    continue;
                }
                let stroke = match kind {
                    OverlayKind::KnifeLine => Stroke::new(t.selection_width, t.knife_line),
                    OverlayKind::Ruler => Stroke::new(t.measure_ruler_width, t.measure_ruler),
                    OverlayKind::DrawPreview => Stroke::new(cx.tokens.outline_width, t.draw_preview),
                    // A handle being pulled out looks like the handles of the outline.
                    OverlayKind::DrawHandle => Stroke::new(cx.tokens.handle_line_width, cx.tokens.handle_line),
                    _ => Stroke::new(t.snap_line_width, t.snap_line),
                };
                out.push(Shape::line_segment([a, b], stroke));
            }
            Overlay::Marker { kind, at } => {
                let at = to_screen(*at);
                if !usable(at) {
                    continue;
                }
                if *kind == OverlayKind::DrawHandle {
                    out.push(Shape::circle_filled(at, cx.tokens.handle_radius, cx.tokens.node_handle));
                    continue;
                }
                let color = match kind {
                    OverlayKind::SnapTarget | OverlayKind::SnapLine => t.snap_line,
                    OverlayKind::HoverLocked => t.locked_point,
                    _ => t.hover_point,
                };
                out.push(Shape::circle_stroke(at, t.marker_radius, Stroke::new(t.selection_width, color)));
            }
            // Text needs the fonts: `canvas::badges` paints these.
            Overlay::Badge { .. } => {}
        }
    }
    out
}
