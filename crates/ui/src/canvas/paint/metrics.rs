//! Metric lines, alignment zones and their names, behind the outlines (View › Show Metrics, Show Metrics Names).
//! A metric whose glyph scope leaves out the edited glyph is not drawn (`fontInfo.metrics.scopeHidesLine`); the
//! projection drops it.

use egui::{Align2, FontId, Pos2, Rect, Stroke};
use fontcraft_geom::Point;

use super::super::layers::PaintCx;

pub fn paint(cx: &PaintCx<'_>) {
    let Some(glyph) = cx.glyph().filter(|_| cx.display.metrics) else { return };
    let (rect, t) = (cx.rect, cx.tokens);
    let y = |v: f64| cx.camera.to_screen(Point::new(0.0, v)).y;
    let x = |v: f64| cx.camera.to_screen(Point::new(v, 0.0)).x;
    for (lo, hi) in &glyph.zones {
        let band = Rect::from_x_y_ranges(rect.x_range(), y(*hi)..=y(*lo));
        cx.painter.rect_filled(band.intersect(rect), 0.0, t.zone_fill);
    }
    let stroke = Stroke::new(t.metric_line_width, t.metric_line);
    for (name, at) in &glyph.lines {
        let at_y = y(*at);
        cx.painter.line_segment([Pos2::new(rect.left(), at_y), Pos2::new(rect.right(), at_y)], stroke);
        if cx.display.metrics_names {
            let label = Pos2::new(rect.left() + t.label_gap, at_y - t.label_gap / 2.0);
            cx.painter.text(label, Align2::LEFT_BOTTOM, format!("{name} {at}"), FontId::proportional(t.label_size), t.metric_name);
        }
    }
    for side in [0.0, glyph.width] {
        cx.painter.line_segment([Pos2::new(x(side), y(glyph.descender)), Pos2::new(x(side), y(glyph.ascender))], stroke);
    }
}
