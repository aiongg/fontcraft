//! Display aids in front of the outline: the ghost outline, Tunni lines, curve tension, node indexes, curve lengths,
//! the metrics beam and the Lock Outline badge. The switches are `shell::overlays`.
//!
//! | Aid | Drawn as |
//! |---|---|
//! | ghost outline | the shapes a running drag changes, as they were when it began, `ghost` |
//! | Tunni lines | a dashed line joining the ends of a curve's handles and a dot where the lines through them meet, `tunni` |
//! | tension | each handle's length as a share of the distance to that meeting point, beside the handle, `tension` |
//! | curve length | the arc length beside the middle of each curve segment, `curve_length` |
//! | indexes | nodes, segments, paths, components and anchors numbered in the colour of what they number |
//! | metrics beam | a line across the canvas with the sidebearings measured on it, `beam` and `beam_label` |
//! | Lock Outline | a framed badge in the canvas's top right corner, `lock_badge` |

use egui::{Align2, Color32, Pos2, Rect, Shape, Stroke, StrokeKind};
use fontcraft_engine::model::Shape as ModelShape;
use fontcraft_geom::{CubicBez, ParamCurveArclen, Piece, Point, Segment, segments};

use super::tokens::OverlayTokens;
use super::{Drawn, num};
use crate::canvas::layers::PaintCx;

/// Most labels (indexes, tensions, lengths) drawn; a pathological glyph stops there.
const MAX_LABELS: usize = 20_000;
/// Accuracy of a curve length, in font units.
const LENGTH_ACCURACY: f64 = 1e-3;
/// Most path elements the ghost outline holds.
pub const MAX_GHOST_ELEMENTS: usize = 200_000;

/// Where a label sits beside its point.
#[derive(Clone, Copy)]
enum Offset {
    AboveRight,
    BelowRight,
    BelowLeft,
    Center,
}

pub fn aids(cx: &PaintCx<'_>) {
    aid_shapes(cx).paint(cx);
}

pub fn aid_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let t = OverlayTokens::standard();
    let data = cx.overlays.data;
    if let Some(ghost) = &data.ghost {
        out.shapes.extend(crate::canvas::paint::outline_shapes(cx, ghost, Stroke::new(t.ghost_width, t.ghost)));
    }
    let Some(glyph) = cx.glyph() else { return out };
    let sw = &data.switches;
    let mut budget = MAX_LABELS;
    let ix = sw.indexes;
    let mut component = 0usize;
    for (s, shape) in glyph.layer.shapes.iter().enumerate() {
        match shape {
            ModelShape::Path(path) => {
                let points = path.node_points();
                if ix.contours
                    && let Some(first) = points.first()
                {
                    label(cx, &mut out, first.point, Offset::BelowLeft, s.to_string(), t.index_contour, &mut budget);
                }
                for (k, seg) in segments(&points, path.closed).iter().enumerate() {
                    let cubic = match seg.pieces.as_slice() {
                        [Piece::Cubic(cb)] => Some(*cb),
                        _ => None,
                    };
                    if let Some(cb) = cubic.filter(|_| sw.tunni_lines) {
                        tunni(cx, &mut out, cb, &t);
                    }
                    if let Some(cb) = cubic.filter(|_| sw.curve_tension)
                        && let Some((first, second)) = fontcraft_engine::pathops::tension::tension(cb)
                    {
                        label(cx, &mut out, cb.p1, Offset::AboveRight, format!("{}%", num(first.round())), t.tension, &mut budget);
                        label(cx, &mut out, cb.p2, Offset::AboveRight, format!("{}%", num(second.round())), t.tension, &mut budget);
                    }
                    if sw.curve_length
                        && !seg.handles.is_empty()
                        && let Some(mid) = middle(seg)
                    {
                        let length: f64 = seg.pieces.iter().map(piece_length).sum();
                        label(cx, &mut out, mid, Offset::AboveRight, num((length * 10.0).round() / 10.0), t.curve_length, &mut budget);
                    }
                    if ix.segments
                        && let Some(mid) = middle(seg)
                    {
                        label(cx, &mut out, mid, Offset::BelowRight, k.to_string(), t.index_segment, &mut budget);
                    }
                }
                if ix.points {
                    for (n, node) in points.iter().enumerate() {
                        label(cx, &mut out, node.point, Offset::AboveRight, n.to_string(), t.index_point, &mut budget);
                    }
                }
            }
            ModelShape::Component(c) => {
                // The canvas model keeps one mark per component, in shape order.
                let mark = glyph.components.get(component);
                component += 1;
                if ix.components {
                    let at = mark
                        .filter(|m| !m.outline.elements().is_empty())
                        .map(|m| fontcraft_geom::Shape::bounding_box(&m.outline).center())
                        .unwrap_or_else(|| c.affine() * Point::ZERO);
                    label(cx, &mut out, at, Offset::Center, s.to_string(), t.index_component, &mut budget);
                }
            }
            _ => {}
        }
    }
    if ix.anchors {
        for (i, anchor) in glyph.layer.anchors.iter().enumerate() {
            label(cx, &mut out, Point::new(anchor.x, anchor.y), Offset::BelowRight, i.to_string(), t.index_anchor, &mut budget);
        }
    }
    if let Some(y) = data.beam {
        beam(cx, &mut out, glyph, y, &t);
    }
    if sw.lock_outline {
        let at = Pos2::new(cx.rect.right() - t.lock_badge_margin, cx.rect.top() + t.lock_badge_margin);
        out.label(at, Align2::RIGHT_TOP, cx.overlays.words.locked.as_str().to_string(), t.aid_label_size, t.lock_badge);
        // The frame around the badge's text, sized from the label's length.
        let width = cx.overlays.words.locked.as_str().to_string().chars().count() as f32 * t.aid_label_size * 0.6;
        let frame = Rect::from_min_max(Pos2::new(at.x - width, at.y), Pos2::new(at.x, at.y + t.aid_label_size * 1.3)).expand(t.aid_label_gap / 2.0);
        out.shapes.push(Shape::rect_stroke(frame, 2.0, Stroke::new(1.0, t.lock_badge), StrokeKind::Outside));
    }
    out
}

/// The dashed line between the ends of a cubic's handles and the dot where the lines through its handles meet.
fn tunni(cx: &PaintCx<'_>, out: &mut Drawn, cb: CubicBez, t: &OverlayTokens) {
    let (Some(a), Some(b)) = (screen(cx, cb.p1), screen(cx, cb.p2)) else { return };
    let stroke = Stroke::new(cx.tokens.handle_line_width, t.tunni);
    let dash = cx.tokens.handle_radius * 2.0;
    out.shapes.extend(Shape::dashed_line(&[a, b], stroke, dash, dash));
    if let Some(p) = fontcraft_tools::drag_aids::tunni_point(cb).and_then(|p| screen(cx, p)) {
        out.shapes.push(Shape::circle_filled(p, cx.tokens.handle_radius, t.tunni));
    }
}

fn piece_length(piece: &Piece) -> f64 {
    let length = match piece {
        Piece::Line(l) => l.arclen(LENGTH_ACCURACY),
        Piece::Quad(q) => q.arclen(LENGTH_ACCURACY),
        Piece::Cubic(cb) => cb.arclen(LENGTH_ACCURACY),
    };
    if length.is_finite() { length } else { 0.0 }
}

/// The point halfway through a segment's pieces.
fn middle(seg: &Segment) -> Option<Point> {
    let count = seg.pieces.len();
    let piece = seg.pieces.get(count / 2)?;
    Some(piece.eval(if count % 2 == 1 { 0.5 } else { 0.0 }))
}

/// The beam at height `y` with the sidebearings measured where it meets the outline, each centred over its gap.
fn beam(cx: &PaintCx<'_>, out: &mut Drawn, glyph: &crate::app::models::canvas::EditedGlyph, y: f64, t: &OverlayTokens) {
    let left_end = cx.camera.to_font(cx.rect.left_center());
    let right_end = cx.camera.to_font(cx.rect.right_center());
    if let (Some(a), Some(b)) = (screen(cx, Point::new(left_end.x.min(right_end.x), y)), screen(cx, Point::new(left_end.x.max(right_end.x), y))) {
        out.shapes.push(Shape::line_segment([a, b], Stroke::new(t.beam_width, t.beam)));
    }
    let Some((lsb, rsb)) = sidebearings_at(&glyph.outline, glyph.width, y) else { return };
    for (x, value) in [(lsb / 2.0, lsb), (glyph.width - rsb / 2.0, rsb)] {
        if let Some(at) = screen(cx, Point::new(x, y)) {
            out.label(at, Align2::CENTER_BOTTOM, num(value), t.beam_label_size, t.beam_label);
        }
    }
}

/// The left and right sidebearings of `outline` measured at height `y`: from 0 to the first crossing, and from the
/// last crossing to `width`. `None` where the line misses the outline.
pub fn sidebearings_at(outline: &fontcraft_geom::BezPath, width: f64, y: f64) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for contour in crate::canvas::paint::flatten(outline, fontcraft_geom::Affine::IDENTITY) {
        for pair in contour.windows(2) {
            let [a, b] = pair else { continue };
            let (ay, by) = (f64::from(a.y), f64::from(b.y));
            if (ay - y) * (by - y) > 0.0 || ay == by {
                continue;
            }
            let x = f64::from(a.x) + (y - ay) / (by - ay) * f64::from(b.x - a.x);
            if x.is_finite() {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
    }
    (lo <= hi).then(|| (lo, width - hi))
}

/// One label beside the font point `p`, while the label budget lasts. Labels off the canvas are skipped and do not
/// count.
fn label(cx: &PaintCx<'_>, out: &mut Drawn, p: Point, offset: Offset, text: String, color: Color32, budget: &mut usize) {
    let t = OverlayTokens::standard();
    if *budget == 0 {
        return;
    }
    let Some(at) = screen(cx, p) else { return };
    if !cx.rect.expand(t.aid_label_size * 4.0).contains(at) {
        return;
    }
    *budget -= 1;
    let d = t.aid_label_gap;
    let (at, align) = match offset {
        Offset::AboveRight => (at + egui::vec2(d, -d), Align2::LEFT_BOTTOM),
        Offset::BelowRight => (at + egui::vec2(d, d), Align2::LEFT_TOP),
        Offset::BelowLeft => (at + egui::vec2(-d, d), Align2::RIGHT_TOP),
        Offset::Center => (at, Align2::CENTER_CENTER),
    };
    out.label(at, align, text, t.aid_label_size, color);
}

fn screen(cx: &PaintCx<'_>, p: Point) -> Option<Pos2> {
    crate::canvas::paint::screen(cx, [p.x, p.y])
}
