//! The text's paint layers. Behind the glyphs: the kerned gaps, while the kerning mode shows kerning, and the
//! selection. With the glyphs: the text's other glyphs and the group members. In front: the text cursor, in text
//! mode when nothing is selected.

use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke};

use super::super::layers::PaintCx;
use super::super::paint::{fill_shapes, outline_shapes};
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
            out.push(Shape::rect_filled(r, 0.0, if *closes { cx.tokens.kerning_negative } else { cx.tokens.kerning_positive }));
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

/// The text's other glyphs, filled while Fill Preview is on and as outlines while it is off, and the group members
/// as outlines.
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
        if cx.display.fill_preview {
            out.extend(fill_shapes(cx, glyph.fill.as_ref().unwrap_or(&glyph.outline), cx.tokens.outline));
        } else {
            out.extend(outline_shapes(cx, &glyph.outline, Stroke::new(t.glyph_width, t.glyph)));
        }
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

pub fn metrics_labels(cx: &PaintCx<'_>) {
    let t = TextTokens::standard();
    let font = FontId::proportional(t.metrics_label_size);
    for (at, align, text) in metrics_label_places(cx) {
        cx.painter.text(at, align, text, font.clone(), t.metrics_label);
    }
}

/// Where each metrics label goes on screen (`spacing.metricsOverlayInTextMode`). Horizontal text: under the
/// descender, the left sidebearing at the glyph's left edge, the right one at its right edge, the width centred under
/// them; a glyph too narrow for both sidebearings shows only its width. Vertical text: along the column's left edge,
/// the top and bottom sidebearings at the advance's ends and the width between them. Glyphs narrower on screen than
/// `metrics_min_advance` get no labels.
pub fn metrics_label_places(cx: &PaintCx<'_>) -> Vec<(Pos2, Align2, String)> {
    let model = cx.text;
    if !model.shown || !model.text_mode {
        return Vec::new();
    }
    let t = TextTokens::standard();
    let (pad, row) = (t.metrics_label_pad, t.metrics_label_size + t.metrics_label_pad);
    // An estimate of a label's width; the painter lays the text out itself.
    let width_of = |s: &str| s.chars().count() as f32 * t.metrics_label_size * 0.6;
    let scale = cx.camera.to_screen((1.0, 0.0).into()).x - cx.camera.to_screen((0.0, 0.0).into()).x;
    let mut out = Vec::new();
    for label in &model.labels {
        let [x0, y0, x1, y1] = label.bounds;
        let a = cx.camera.to_screen((x0, y0).into());
        let b = cx.camera.to_screen((x1, y1).into());
        let r = Rect::from_two_pos(a, b);
        let along = if label.vertical { r.height() } else { r.width() };
        if !r.is_finite() || along < t.metrics_min_advance || !r.expand(row * 2.0).intersects(cx.rect) {
            continue;
        }
        let (first, second) = label.sides.map_or((String::new(), String::new()), |(a, b)| (number(a), number(b)));
        let width = number(label.advance);
        let (before, after) = ((label.overlap.0 as f32) * scale.abs(), (label.overlap.1 as f32) * scale.abs());
        if label.vertical {
            let x = r.left() + pad;
            out.push((Pos2::new(x, r.top() + before + pad), Align2::LEFT_TOP, first));
            out.push((Pos2::new(x, r.bottom() - after - pad), Align2::LEFT_BOTTOM, second));
            out.push((Pos2::new(x, r.center().y), Align2::LEFT_CENTER, width));
        } else {
            // The box runs from the descender up; its bottom edge on screen is the larger y.
            let y = r.bottom() + pad;
            let (start, end) = (r.left() + before + pad, r.right() - after - pad);
            if !first.is_empty() && width_of(&first) + width_of(&second) + pad <= end - start {
                out.push((Pos2::new(start, y), Align2::LEFT_TOP, first));
                out.push((Pos2::new(end, y), Align2::RIGHT_TOP, second));
            }
            out.push((Pos2::new(r.center().x, y + row), Align2::CENTER_TOP, width));
        }
    }
    out
}
