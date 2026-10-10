//! Metric lines, alignment zones and their names, behind the outlines.

use egui::{Align2, FontId, Pos2, Rect, Stroke};
use fontcraft_geom::Point;
use fontcraft_model::{Font, Master, metric};

use super::nodes::Metrics;

/// The master's horizontal metric lines, with their names, and its alignment zones.
pub fn of(font: &Font, master: &Master) -> (Vec<(String, f64)>, Metrics) {
    let lines: Vec<(String, f64)> = font
        .metrics
        .iter()
        .zip(master.metric_values.iter())
        .filter(|(d, v)| d.kind != metric::ITALIC_ANGLE && v.pos.is_finite())
        .map(|(d, v)| (d.kind.clone(), v.pos))
        .collect();
    let zones = font
        .alignment_zones(master)
        .iter()
        .filter(|z| z.pos.is_finite() && z.size.is_finite())
        .map(|z| if z.size >= 0.0 { (z.pos, z.pos + z.size) } else { (z.pos + z.size, z.pos) })
        .collect();
    let metrics = Metrics { lines: lines.iter().map(|(_, y)| *y).collect(), zones };
    (lines, metrics)
}

/// Draws zones as bands and metric lines across the canvas, the edited glyph's side lines from
/// `span.0` (descender) to `span.1` (ascender), and the names when `names` is set.
pub fn draw(c: &super::Painting, lines: &[(String, f64)], metrics: &Metrics, width: f64, span: (f64, f64), names: bool) {
    let (rect, tokens) = (c.rect, c.tokens);
    let y = |v: f64| c.view.to_screen(Point::new(0.0, v)).y as f32;
    let x = |v: f64| c.view.to_screen(Point::new(v, 0.0)).x as f32;
    for (lo, hi) in &metrics.zones {
        let band = Rect::from_x_y_ranges(rect.x_range(), y(*hi)..=y(*lo));
        c.painter.rect_filled(band.intersect(rect), 0.0, tokens.zone_fill);
    }
    let stroke = Stroke::new(tokens.metric_line_width, tokens.metric_line);
    for (name, at) in lines {
        let at_y = y(*at);
        c.painter.line_segment([Pos2::new(rect.left(), at_y), Pos2::new(rect.right(), at_y)], stroke);
        if names {
            let font = FontId::proportional(tokens.metric_name_size);
            c.painter.text(Pos2::new(rect.left() + 4.0, at_y - 2.0), Align2::LEFT_BOTTOM, format!("{name} {at}"), font, tokens.metric_name);
        }
    }
    for side in [0.0, width] {
        c.painter.line_segment([Pos2::new(x(side), y(span.0)), Pos2::new(x(side), y(span.1))], stroke);
    }
}
