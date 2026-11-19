//! The text's paint layers. Behind the glyphs: the kerned gaps, while the kerning mode shows kerning, and the
//! selection. With the glyphs: the text's other glyphs and the group members. In front: the text cursor, in text
//! mode when nothing is selected.

use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke};

use super::super::layers::PaintCx;
use super::super::paint::outline_shapes;
use super::tokens::TextTokens;

/// The screen rectangle of `[x0, y0, x1, y1]` in the edited glyph's font units.
fn screen_rect(cx: &PaintCx<'_>, [x0, y0, x1, y1]: [f64; 4]) -> Option<Rect> {
    let a = cx.camera.to_screen((x0, y0).into());
    let b = cx.camera.to_screen((x1, y1).into());
    let rect = Rect::from_two_pos(a, b);
    (rect.is_finite() && rect.intersects(cx.rect)).then_some(rect)
}

pub fn behind(cx: &PaintCx<'_>) {
    cx.painter.extend(behind_shapes(cx));
}

/// The kerned gaps and the selection.
pub fn behind_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let model = cx.text;
    if !model.shown {
        return Vec::new();
    }
    let t = TextTokens::standard();
    let mut out = Vec::new();
    for (band, closes) in &model.kerned {
        if let Some(r) = screen_rect(cx, *band) {
            out.push(Shape::rect_filled(r, 0.0, if *closes { t.kerning_negative } else { t.kerning_positive }));
        }
    }
    if model.text_mode {
        for band in &model.selection {
            if let Some(r) = screen_rect(cx, *band) {
                out.push(Shape::rect_filled(r, 0.0, t.selection));
            }
        }
    }
    out
}

pub fn glyphs(cx: &PaintCx<'_>) {
    cx.painter.extend(glyph_shapes(cx));
}

/// The text's other glyphs and the group members, as outlines.
pub fn glyph_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let model = cx.text;
    if !model.shown {
        return Vec::new();
    }
    let t = TextTokens::standard();
    let mut out = Vec::new();
    for member in &model.members {
        out.extend(outline_shapes(cx, member, Stroke::new(t.group_members_width, t.group_members)));
    }
    for glyph in &model.glyphs {
        out.extend(outline_shapes(cx, &glyph.outline, Stroke::new(t.glyph_width, t.glyph)));
    }
    out
}

pub fn cursor(cx: &PaintCx<'_>) {
    cx.painter.extend(cursor_shapes(cx));
}

/// The text cursor.
pub fn cursor_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let model = cx.text;
    let Some([a, b]) = model.cursor.filter(|_| model.shown && model.text_mode) else { return Vec::new() };
    let t = TextTokens::standard();
    let to = |[x, y]: [f64; 2]| -> Pos2 { cx.camera.to_screen((x, y).into()) };
    let (a, b) = (to(a), to(b));
    if !(a.is_finite() && b.is_finite()) {
        return Vec::new();
    }
    vec![Shape::line_segment([a, b], Stroke::new(t.cursor_width, t.cursor))]
}

pub fn measurement(cx: &PaintCx<'_>) {
    cx.painter.extend(measurement_shapes(cx));
    let t = TextTokens::standard();
    let font = FontId::proportional(t.measurement_label_size);
    for (at, value) in measurement_labels(cx) {
        cx.painter.text(at, Align2::CENTER_BOTTOM, number(value), font.clone(), t.measurement_label);
    }
}

/// Where a glyph's measured advance shows on screen: its left and right edge and its bottom and top.
fn measured_edges(cx: &PaintCx<'_>, g: &crate::app::models::text::MeasuredGlyph) -> Option<(f32, f32, f32, f32)> {
    let screen = |x: f64, y: f64| cx.camera.to_screen((x, y).into());
    let (top, bottom) = (screen(g.x0, g.top).y, screen(g.x0, g.bottom).y);
    let (x0, x1) = (screen(g.x0, g.y).x, screen(g.x1, g.y).x);
    let finite = x0.is_finite() && x1.is_finite() && top.is_finite() && bottom.is_finite();
    let visible = x1 >= cx.rect.left() && x0 <= cx.rect.right() && bottom >= cx.rect.top() && top <= cx.rect.bottom();
    (finite && visible).then_some((x0, x1, top, bottom))
}

/// The measurement line: a rule across the canvas at the line's height on every line of the text, and thin lines at
/// the edges of every advance.
pub fn measurement_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(line) = cx.text.measurement.as_ref().filter(|_| cx.text.shown) else { return Vec::new() };
    let t = TextTokens::standard();
    let (left, right) = (cx.rect.left(), cx.rect.right());
    let mut out = Vec::new();
    for &rule in &line.rules {
        let y = cx.camera.to_screen((0.0, rule).into()).y;
        if (cx.rect.top()..=cx.rect.bottom()).contains(&y) {
            out.push(Shape::line_segment([Pos2::new(left, y), Pos2::new(right, y)], Stroke::new(t.measurement_width, t.measurement)));
        }
    }
    let advance = Stroke::new(t.measurement_advance_width, t.measurement_advance);
    for g in &line.glyphs {
        let Some((x0, x1, top, bottom)) = measured_edges(cx, g) else { continue };
        for x in [x0, x1] {
            out.push(Shape::line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], advance));
        }
    }
    out
}

/// Each glyph's sidebearings on the measurement line, centred over the length they measure, where the glyph's
/// advance leaves room for them: the bottom centre of each number on screen and the number.
pub fn measurement_labels(cx: &PaintCx<'_>) -> Vec<(Pos2, f64)> {
    let Some(line) = cx.text.measurement.as_ref().filter(|_| cx.text.shown) else { return Vec::new() };
    let t = TextTokens::standard();
    let screen = |x: f64, y: f64| cx.camera.to_screen((x, y).into());
    let mut out = Vec::new();
    for g in &line.glyphs {
        let Some((x0, x1, _, _)) = measured_edges(cx, g) else { continue };
        let Some((lsb, rsb)) = g.sides.filter(|_| x1 - x0 >= t.measurement_min_advance) else { continue };
        let y = screen(g.x0, g.y).y - t.measurement_label_gap;
        for (from, to, value) in [(g.x0, g.x0 + lsb, lsb), (g.x1 - rsb, g.x1, rsb)] {
            out.push((Pos2::new((screen(from, g.y).x + screen(to, g.y).x) / 2.0, y), value));
        }
    }
    out
}

/// A measured length as the line prints it: whole units without a fraction, else one decimal.
pub fn number(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 { format!("{rounded:.0}") } else { format!("{rounded:.1}") }
}
