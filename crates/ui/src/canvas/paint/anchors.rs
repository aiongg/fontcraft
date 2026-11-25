//! The edited layer's anchors, under View › Show Anchors.
//!
//! | Anchor | Drawn as (`anchors.shapeShowsMetricPosition`) |
//! |---|---|
//! | on a metric line | diamond, `anchor` |
//! | inside an alignment zone | square, `anchor` |
//! | elsewhere | circle, `anchor` |
//! | name starting with `_` | the same shape hollow |
//! | selected | a ring in the tools' `selection` and the anchor's name (`anchors.selection`) |

use egui::{Align2, Color32, FontId, Rect, Shape, Stroke, StrokeKind, Vec2};
use fontcraft_engine::model::Anchor;

use super::super::layers::PaintCx;
use super::nodes::Metrics;
use super::screen;
use crate::canvas::tools::tokens::ToolTokens;

/// How close, in font units, an anchor must be to a metric line to count as on it.
const ON_LINE: f64 = 1e-6;
/// Most anchors drawn; a layer holds far fewer.
const MAX_DRAWN_ANCHORS: usize = 10_000;
/// Most anchor names drawn, and most characters of each.
const MAX_DRAWN_NAMES: usize = 500;
const MAX_NAME_CHARS: usize = 120;

/// Where an anchor sits relative to the metric lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Diamond,
    Square,
    Circle,
}

/// The marker of `anchor` among `metrics`.
pub fn marker(anchor: &Anchor, metrics: &Metrics) -> Marker {
    if metrics.lines.iter().any(|y| (anchor.y - y).abs() < ON_LINE) {
        Marker::Diamond
    } else if metrics.zones.iter().any(|&(bottom, top)| anchor.y >= bottom.min(top) && anchor.y <= bottom.max(top)) {
        Marker::Square
    } else {
        Marker::Circle
    }
}

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(glyph) = cx.glyph().filter(|_| cx.display.anchors) else { return Vec::new() };
    let t = cx.tokens;
    let metrics = Metrics::of(glyph);
    let half = t.anchor_size / 2.0;
    let tool = ToolTokens::standard();
    let mut names = 0usize;
    let mut out = Vec::new();
    for anchor in glyph.layer.anchors.iter().take(MAX_DRAWN_ANCHORS) {
        let Some(at) = screen(cx, [anchor.x, anchor.y]) else { continue };
        let hollow = anchor.name.starts_with('_');
        let stroke = Stroke::new(t.handle_line_width * 2.0, t.anchor);
        let fill = if hollow { Color32::TRANSPARENT } else { t.anchor };
        out.push(match (marker(anchor, &metrics), hollow) {
            (Marker::Diamond, _) => {
                let points = vec![at + Vec2::new(0.0, -half), at + Vec2::new(half, 0.0), at + Vec2::new(0.0, half), at + Vec2::new(-half, 0.0)];
                Shape::convex_polygon(points, fill, if hollow { stroke } else { Stroke::NONE })
            }
            (Marker::Square, false) => Shape::rect_filled(Rect::from_center_size(at, Vec2::splat(t.anchor_size * 0.8)), 0.0, fill),
            (Marker::Square, true) => {
                Shape::rect_stroke(Rect::from_center_size(at, Vec2::splat(t.anchor_size * 0.8)), 0.0, stroke, StrokeKind::Middle)
            }
            (Marker::Circle, false) => Shape::circle_filled(at, half * 0.9, fill),
            (Marker::Circle, true) => Shape::circle_stroke(at, half * 0.9, stroke),
        });
        if !cx.tools.selection.anchors.contains(&anchor.name) {
            continue;
        }
        out.push(Shape::circle_stroke(at, tool.selection_ring_radius, Stroke::new(tool.selection_width, tool.selection)));
        if names < MAX_DRAWN_NAMES {
            names += 1;
            let name: String = anchor.name.chars().take(MAX_NAME_CHARS).collect();
            let galley = cx.painter.layout_no_wrap(name, FontId::proportional(t.label_size), t.anchor);
            let corner = at + Vec2::new(tool.selection_ring_radius + t.label_gap, -tool.selection_ring_radius);
            out.push(Shape::galley(Align2::LEFT_BOTTOM.anchor_size(corner, galley.size()).min, galley, t.anchor));
        }
    }
    out
}
