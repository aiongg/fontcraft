//! The PostScript hint layers while View › Show Hints is on (`hinting.ps.manual.display`).
//!
//! | Part | Look |
//! |---|---|
//! | bar | across the canvas between the hint's edges: yellow for a horizontal stem, green for a vertical one; a ghost hint is a blue band on the inner side of its edge |
//! | badge | a gray box beside the hint with its origin and size (a ghost: its position and an arrow, up for a top ghost, down for a bottom one); orange while selected, ringed red when the hint has warnings |
//! | markers | blue, at the edges: a circle at the origin, a triangle at the other edge pointing away from it; filled when that end is linked to a node |
//!
//! The marks are placed by `fontcraft_tools::hint_drag::HintMarks`, the same placement the Select tool hits.

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, StrokeKind};
use fontcraft_geom::Point;
use fontcraft_tools::hint_drag::{End, HintKind, HintMarks};

use crate::canvas::layers::PaintCx;
use crate::canvas::tools::apply::tools_view;
use crate::panels::widgets::format_number;

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

/// The layer's marks, when they are shown.
fn marks<'a>(cx: &'a PaintCx<'_>) -> Option<&'a HintMarks> {
    let marks = &cx.hinting.model.ps;
    (marks.shown && !cx.edit_background).then_some(marks)
}

/// The bars, behind the outline.
pub fn bars(cx: &PaintCx<'_>) {
    cx.painter.extend(bar_shapes(cx));
}

/// The shapes of [`bars`].
pub fn bar_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(marks) = marks(cx) else { return Vec::new() };
    let t = cx.hinting.tokens;
    let mut out = Vec::new();
    for h in &marks.hints {
        let Some(origin) = h.origin else { continue };
        let at = |v: f64| if h.horizontal { cx.camera.to_screen(Point::new(0.0, v)).y } else { cx.camera.to_screen(Point::new(v, 0.0)).x };
        let a = at(origin);
        if !a.is_finite() {
            continue;
        }
        let (lo, hi, color) = match h.kind {
            HintKind::Stem => {
                let b = h.target().map_or(a, at);
                if !b.is_finite() {
                    continue;
                }
                (a.min(b), a.max(b), if h.horizontal { t.ps_horizontal } else { t.ps_vertical })
            }
            // The band lies inside the glyph: below a top edge, above a bottom one (screen y grows downwards).
            HintKind::GhostTop if h.horizontal => (a, a + t.ps_ghost_band, t.ps_ghost),
            HintKind::GhostBottom if h.horizontal => (a - t.ps_ghost_band, a, t.ps_ghost),
            HintKind::GhostTop | HintKind::GhostBottom => (a, a + t.ps_ghost_band, t.ps_ghost),
        };
        let rect = if h.horizontal {
            Rect::from_min_max(Pos2::new(cx.rect.left(), lo), Pos2::new(cx.rect.right(), hi))
        } else {
            Rect::from_min_max(Pos2::new(lo, cx.rect.top()), Pos2::new(hi, cx.rect.bottom()))
        };
        out.push(Shape::rect_filled(rect, 0.0, color));
    }
    out
}

/// The badges and markers, in front of the outline.
pub fn marks_layer(cx: &PaintCx<'_>) {
    let (shapes, labels) = mark_parts(cx);
    cx.painter.extend(shapes);
    let t = cx.hinting.tokens;
    for (at, text) in labels {
        cx.painter.text(at, Align2::CENTER_CENTER, text, FontId::proportional(t.ps_badge_text_size), t.ps_badge_text);
    }
}

/// The shapes of [`marks_layer`], without the badges' numbers.
pub fn mark_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    mark_parts(cx).0
}

/// The shapes of the marks, and the badges' numbers with their centres.
fn mark_parts(cx: &PaintCx<'_>) -> (Vec<Shape>, Vec<(Pos2, String)>) {
    let mut labels = Vec::new();
    let Some(marks) = marks(cx) else { return (Vec::new(), labels) };
    let t = cx.hinting.tokens;
    let view = tools_view(&cx.camera);
    let selected = &cx.tools.selection.hints;
    let r = t.marker_radius;
    let stroke = Stroke::new(t.ps_marker_width, t.ps_marker);
    let mut out = Vec::new();
    for h in &marks.hints {
        let Some(origin) = marks.marker(&view, h, End::Origin) else { continue };
        let target = marks.marker(&view, h, End::Target);
        if let Some(b) = marks.badge(&view, h) {
            let rect = Rect::from_min_max(pos(b.min), pos(b.max));
            let fill = if selected.contains(&h.index) { t.ps_badge_selected } else { t.ps_badge };
            out.push(Shape::rect_filled(rect, t.ps_badge_rounding, fill));
            if !h.warnings.is_empty() {
                out.push(Shape::rect_stroke(rect, t.ps_badge_rounding, Stroke::new(t.ps_warning_width, t.ps_warning), StrokeKind::Outside));
            }
            let centre = rect.center();
            match (h.kind, h.origin) {
                (HintKind::Stem, Some(o)) => {
                    let text = match h.size {
                        Some(s) => format!("{} / {}", format_number(o), format_number(s)),
                        None => format_number(o),
                    };
                    labels.push((centre, text));
                }
                (kind, Some(o)) => {
                    let arrow = Pos2::new(rect.right() - r - t.ps_badge_rounding, centre.y);
                    labels.push((Pos2::new(centre.x - r, centre.y), format_number(o)));
                    let up = kind == HintKind::GhostTop;
                    let (tip, base) = if up { (arrow.y - r * 0.8, arrow.y + r * 0.8) } else { (arrow.y + r * 0.8, arrow.y - r * 0.8) };
                    let points = vec![Pos2::new(arrow.x, tip), Pos2::new(arrow.x + r * 0.7, base), Pos2::new(arrow.x - r * 0.7, base)];
                    out.push(Shape::convex_polygon(points, t.ps_badge_text, Stroke::NONE));
                }
                _ => {}
            }
        }
        if h.origin_node.is_some() {
            out.push(Shape::circle_filled(pos(origin), r, t.ps_marker));
        } else {
            out.push(Shape::circle_stroke(pos(origin), r, stroke));
        }
        if let Some(to) = target.filter(|_| h.kind == HintKind::Stem) {
            let (dx, dy) = (to.x - origin.x, to.y - origin.y);
            let len = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = if len > 1e-9 && len.is_finite() {
                ((dx / len) as f32, (dy / len) as f32)
            } else if h.horizontal {
                (0.0, -1.0)
            } else {
                (1.0, 0.0)
            };
            let at = pos(to);
            let tip = Pos2::new(at.x + ux * r, at.y + uy * r);
            let left = Pos2::new(at.x - ux * r - uy * r, at.y - uy * r + ux * r);
            let right = Pos2::new(at.x - ux * r + uy * r, at.y - uy * r - ux * r);
            let fill = if h.target_node.is_some() { t.ps_marker } else { Color32::TRANSPARENT };
            out.push(Shape::convex_polygon(vec![tip, left, right], fill, stroke));
        }
    }
    (out, labels)
}
