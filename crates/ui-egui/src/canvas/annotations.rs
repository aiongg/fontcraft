//! Annotations on the edited layer, under View › Show Annotations (`view.showAnnotations`). The
//! geometry is the Annotation tool's ([`fontcraft_tools::annotate`]), so what is drawn is what
//! the tool hits.
//!
//! | Kind | Drawn as |
//! |---|---|
//! | text | a filled box from its position, as wide as its width, with the text wrapped inside |
//! | arrow | a line from its tail to its tip, with a head at the tip |
//! | circle | a ring of its diameter around its position |
//! | plus, minus | the sign at its position |
//! | a type FontCraft does not know | a dot and the type's name |
//! | selected | its handle (a filled dot) and a ring at its position, `selection` |

use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke, StrokeKind};
use fontcraft_geom::Point;
use fontcraft_model::{Annotation, Layer};
use fontcraft_tools::Selection;
use fontcraft_tools::annotate::{Kind, MARK_RADIUS, TEXT_LINE_HEIGHT, arrow_tail, diameter, handle, text_box, text_lines};

use super::Painting;
use crate::state::DisplayOptions;

/// Most annotations drawn; the model allows no more per layer.
const MAX_DRAWN: usize = fontcraft_model::MAX_ANNOTATIONS;
/// Inner margin of a note, in points.
const NOTE_PADDING: f32 = 3.0;
/// Length of an arrowhead's sides, in points.
const ARROW_HEAD: f32 = 10.0;

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The annotations of `layer`, over everything but the tool overlays.
pub fn draw(c: &Painting, layer: &Layer, selection: &Selection, display: &DisplayOptions) {
    if !display.annotations {
        return;
    }
    for (index, a) in layer.info.annotations.iter().enumerate().take(MAX_DRAWN) {
        let at = pos(c.view.to_screen(Point::new(a.x, a.y)));
        if !usable(at) {
            continue;
        }
        draw_one(c, a, at);
        if selection.annotations.contains(&index) {
            let stroke = Stroke::new(c.tokens.selection_width, c.tokens.selection);
            c.painter.circle_stroke(at, c.tokens.selection_ring_radius, stroke);
            if let Some(h) = handle(a, &c.view).map(|h| pos(c.view.to_screen(h))).filter(|h| usable(*h)) {
                c.painter.circle_filled(h, c.tokens.annotation_handle_radius, c.tokens.selection);
            }
        }
    }
}

fn draw_one(c: &Painting, a: &Annotation, at: Pos2) {
    let tokens = c.tokens;
    let stroke = Stroke::new(tokens.annotation_width, tokens.annotation);
    let r = MARK_RADIUS as f32;
    match Kind::of(a) {
        Kind::Text => {
            let (lo, hi) = text_box(a, &c.view);
            let (lo, hi) = (pos(c.view.to_screen(lo)), pos(c.view.to_screen(hi)));
            if !(usable(lo) && usable(hi)) {
                return;
            }
            let width = (hi.x - lo.x).max(1.0);
            let text = a.text.clone().unwrap_or_default();
            let galley = c.painter.layout(
                text,
                FontId::proportional(tokens.annotation_text_size),
                tokens.annotation_text,
                (width - 2.0 * NOTE_PADDING).max(1.0),
            );
            let lines_height = TEXT_LINE_HEIGHT as f32 * text_lines(a) as f32;
            let height = (galley.size().y + 2.0 * NOTE_PADDING).max(lines_height);
            let rect = Rect::from_min_size(lo, egui::vec2(width, height));
            c.painter.rect_filled(rect, 2.0, tokens.annotation_note_fill);
            c.painter.add(Shape::rect_stroke(rect, 2.0, Stroke::new(1.0, tokens.annotation), StrokeKind::Inside));
            c.painter.galley(Pos2::new(lo.x + NOTE_PADDING, lo.y + NOTE_PADDING), galley, tokens.annotation_text);
        }
        Kind::Arrow => {
            let tail = pos(c.view.to_screen(arrow_tail(a, &c.view)));
            if !usable(tail) {
                return;
            }
            c.painter.line_segment([tail, at], stroke);
            let (dx, dy) = (tail.x - at.x, tail.y - at.y);
            let len = (dx * dx + dy * dy).sqrt();
            if len > 0.0 {
                let (ux, uy) = (dx / len, dy / len);
                // The head's sides leave the tip at ±30° from the shaft.
                let (cos, sin) = (0.866_f32, 0.5_f32);
                for side in [1.0_f32, -1.0] {
                    let (hx, hy) = (ux * cos - uy * sin * side, uy * cos + ux * sin * side);
                    c.painter.line_segment([at, Pos2::new(at.x + hx * ARROW_HEAD, at.y + hy * ARROW_HEAD)], stroke);
                }
            }
        }
        Kind::Circle => {
            let radius = (diameter(a) / 2.0 * c.view.scale()) as f32;
            if radius.is_finite() {
                c.painter.circle_stroke(at, radius, stroke);
            }
        }
        Kind::Plus => {
            c.painter.line_segment([Pos2::new(at.x - r, at.y), Pos2::new(at.x + r, at.y)], stroke);
            c.painter.line_segment([Pos2::new(at.x, at.y - r), Pos2::new(at.x, at.y + r)], stroke);
        }
        Kind::Minus => {
            c.painter.line_segment([Pos2::new(at.x - r, at.y), Pos2::new(at.x + r, at.y)], stroke);
        }
        Kind::Other => {
            c.painter.circle_filled(at, tokens.annotation_handle_radius, tokens.annotation);
            let font = FontId::proportional(tokens.annotation_text_size);
            c.painter.text(Pos2::new(at.x + r, at.y), Align2::LEFT_CENTER, &a.kind, font, tokens.annotation);
        }
    }
}
