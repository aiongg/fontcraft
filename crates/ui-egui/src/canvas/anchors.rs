//! The edited layer's anchors, under View › Show Anchors (`view.showAnchors`).
//!
//! | Anchor | Drawn as (`anchors.shapeShowsMetricPosition`) |
//! |---|---|
//! | on a metric line | diamond, `anchor` |
//! | inside an alignment zone | square, `anchor` |
//! | anywhere else | circle, `anchor` |
//! | name starting with an underscore | the same shape, hollow |
//! | selected | a ring in `selection` and its name (`anchors.selection`) |

use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke, Vec2};
use fontcraft_engine::model::{Anchor, Layer};
use fontcraft_geom::Point;
use fontcraft_tools::Selection;

use super::Painting;
use super::nodes::Metrics;

/// How close, in font units, an anchor must be to a metric line to count as on it.
const ON_LINE: f64 = 1e-6;
/// Most anchors drawn; a layer holds far fewer.
const MAX_DRAWN_ANCHORS: usize = 10_000;

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

pub fn draw(c: &Painting, layer: &Layer, selection: &Selection, metrics: &Metrics) {
    let t = c.tokens;
    let half = t.anchor_size / 2.0;
    for anchor in layer.anchors.iter().take(MAX_DRAWN_ANCHORS) {
        let p = c.view.to_screen(Point::new(anchor.x, anchor.y));
        let at = Pos2::new(p.x as f32, p.y as f32);
        if !(at.x.is_finite() && at.y.is_finite()) {
            continue;
        }
        let hollow = anchor.name.starts_with('_');
        let (fill, stroke) = if hollow { (egui::Color32::TRANSPARENT, Stroke::new(2.0, t.anchor)) } else { (t.anchor, Stroke::NONE) };
        let shape = match marker(anchor, metrics) {
            Marker::Diamond => {
                let points = vec![at + Vec2::new(0.0, -half), at + Vec2::new(half, 0.0), at + Vec2::new(0.0, half), at + Vec2::new(-half, 0.0)];
                Shape::convex_polygon(points, fill, stroke)
            }
            Marker::Square => Shape::rect_filled(Rect::from_center_size(at, Vec2::splat(t.anchor_size * 0.8)), 0.0, fill),
            Marker::Circle => Shape::circle_filled(at, half * 0.9, fill),
        };
        let shape = match (shape, hollow) {
            (Shape::Rect(r), true) => Shape::rect_stroke(r.rect, 0.0, stroke, egui::StrokeKind::Middle),
            (Shape::Circle(circle), true) => Shape::circle_stroke(circle.center, circle.radius, stroke),
            (other, _) => other,
        };
        c.painter.add(shape);
        if selection.anchors.contains(&anchor.name) {
            c.painter.add(Shape::circle_stroke(at, t.selection_ring_radius, Stroke::new(t.selection_width, t.selection)));
            let label_at = at + Vec2::new(t.selection_ring_radius + 2.0, -t.selection_ring_radius);
            c.painter.text(label_at, Align2::LEFT_BOTTOM, &anchor.name, FontId::proportional(t.metric_name_size), t.anchor);
        }
    }
}
