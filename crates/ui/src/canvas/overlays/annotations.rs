//! Annotations on the edited layer, under View › Show Annotations (`view.showAnnotations`). The geometry is the
//! Annotation tool's ([`fontcraft_tools::annotate`]), so what is drawn is what the tool hits.
//!
//! | Kind | Drawn as |
//! |---|---|
//! | text | a filled box from its position, as wide as its width, with the text wrapped inside |
//! | arrow | a line from its tail to its tip, with a head at the tip |
//! | circle | a ring of its diameter around its position |
//! | plus, minus | the sign at its position |
//! | a type FontCraft does not know | a dot and the type's name |
//! | selected | its handle (a filled dot) and a ring at its position, `annotation_selected` |

use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke, StrokeKind};
use fontcraft_engine::model::Annotation;
use fontcraft_tools::View;
use fontcraft_tools::annotate::{Kind, MARK_RADIUS, arrow_tail, diameter, handle, text_box};

use super::Drawn;
use super::tokens::OverlayTokens;
use crate::canvas::layers::PaintCx;

/// Most annotations drawn.
const MAX_DRAWN: usize = fontcraft_engine::model::MAX_ANNOTATIONS;

fn view(cx: &PaintCx<'_>) -> View {
    crate::canvas::tools::apply::tools_view(&cx.camera)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The edited layer's annotations, while they show.
pub fn annotations(cx: &PaintCx<'_>) {
    let drawn = annotation_shapes(cx);
    drawn.paint(cx);
    // The notes' text, wrapped to their boxes.
    let t = OverlayTokens::standard();
    for (index, a, rect) in note_boxes(cx) {
        if cx.overlays.data.editing == Some(index) {
            continue;
        }
        let text = a.text.clone().unwrap_or_default();
        let galley = cx.painter.layout(
            text,
            FontId::proportional(t.annotation_text_size),
            t.annotation_text,
            (rect.width() - 2.0 * t.annotation_padding).max(1.0),
        );
        cx.painter.galley(rect.min + egui::vec2(t.annotation_padding, t.annotation_padding), galley, t.annotation_text);
    }
}

/// Each text note's index, annotation and box on screen.
pub fn note_boxes<'a>(cx: &'a PaintCx<'_>) -> Vec<(usize, &'a Annotation, Rect)> {
    let Some(glyph) = cx.glyph() else { return Vec::new() };
    if !cx.display.annotations {
        return Vec::new();
    }
    let v = view(cx);
    glyph
        .layer
        .info
        .annotations
        .iter()
        .enumerate()
        .take(MAX_DRAWN)
        .filter(|(_, a)| Kind::of(a) == Kind::Text)
        .filter_map(|(i, a)| {
            let (lo, hi) = text_box(a, &v);
            let (lo, hi) = (cx.camera.to_screen(lo), cx.camera.to_screen(hi));
            (usable(lo) && usable(hi)).then(|| (i, a, Rect::from_two_pos(lo, hi)))
        })
        .collect()
}

pub fn annotation_shapes(cx: &PaintCx<'_>) -> Drawn {
    let mut out = Drawn::default();
    let Some(glyph) = cx.glyph() else { return out };
    if !cx.display.annotations {
        return out;
    }
    let t = OverlayTokens::standard();
    let v = view(cx);
    let stroke = Stroke::new(t.annotation_width, t.annotation);
    let r = MARK_RADIUS as f32;
    for (index, a) in glyph.layer.info.annotations.iter().enumerate().take(MAX_DRAWN) {
        let at = cx.camera.to_screen(fontcraft_geom::Point::new(a.x, a.y));
        if !usable(at) {
            continue;
        }
        match Kind::of(a) {
            Kind::Text => {
                let (lo, hi) = text_box(a, &v);
                let (lo, hi) = (cx.camera.to_screen(lo), cx.camera.to_screen(hi));
                if usable(lo) && usable(hi) {
                    let rect = Rect::from_two_pos(lo, hi);
                    out.shapes.push(Shape::rect_filled(rect, 2.0, t.annotation_note_fill));
                    out.shapes.push(Shape::rect_stroke(rect, 2.0, Stroke::new(1.0, t.annotation), StrokeKind::Inside));
                }
            }
            Kind::Arrow => {
                let tail = cx.camera.to_screen(arrow_tail(a, &v));
                if !usable(tail) {
                    continue;
                }
                out.shapes.push(Shape::line_segment([tail, at], stroke));
                let dir = (tail - at).normalized();
                if dir.x.is_finite() && dir.y.is_finite() {
                    // The head's sides leave the tip at ±30° from the shaft.
                    for angle in [30.0_f32, -30.0] {
                        let side = egui::emath::Rot2::from_angle(angle.to_radians()) * dir;
                        out.shapes.push(Shape::line_segment([at, at + side * t.annotation_arrow_head], stroke));
                    }
                }
            }
            Kind::Circle => {
                let radius = (diameter(a) / 2.0 * cx.camera.scale()) as f32;
                if radius.is_finite() {
                    out.shapes.push(Shape::circle_stroke(at, radius, stroke));
                }
            }
            Kind::Plus => {
                out.shapes.push(Shape::line_segment([at - egui::vec2(r, 0.0), at + egui::vec2(r, 0.0)], stroke));
                out.shapes.push(Shape::line_segment([at - egui::vec2(0.0, r), at + egui::vec2(0.0, r)], stroke));
            }
            Kind::Minus => out.shapes.push(Shape::line_segment([at - egui::vec2(r, 0.0), at + egui::vec2(r, 0.0)], stroke)),
            Kind::Other => {
                out.shapes.push(Shape::circle_filled(at, t.annotation_handle_radius, t.annotation));
                out.label(at + egui::vec2(r, 0.0), Align2::LEFT_CENTER, a.kind.clone(), t.annotation_text_size, t.annotation);
            }
        }
        if cx.tools.selection.annotations.contains(&index) {
            let selected = Stroke::new(t.annotation_width, t.annotation_selected);
            out.shapes.push(Shape::circle_stroke(at, t.annotation_handle_radius * 2.0, selected));
            if let Some(h) = handle(a, &v).map(|p| cx.camera.to_screen(p)).filter(|p| usable(*p)) {
                out.shapes.push(Shape::circle_filled(h, t.annotation_handle_radius, t.annotation_selected));
            }
        }
    }
    out
}
