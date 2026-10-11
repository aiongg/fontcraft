//! Metric scopes in Edit View (`fontInfo.metrics.scopeHidesLine`): a metric whose definition has a
//! glyph scope (its `filter`, a glyph predicate evaluated with `fontcraft_glyphquery`) draws its
//! line and its alignment zone only in the glyphs the scope takes in. The edited glyph decides;
//! with no edited glyph, or a scope that does not read as a predicate, the metric shows.

use fontcraft_engine::Session;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, Master, MetricDef, metric};

use super::metrics;
use super::nodes::Metrics;
use crate::state::Tab;

/// The master's metric lines and zones as [`metrics::of`] gives them, without the metrics whose
/// scope leaves out the glyph `tab` edits.
pub fn of(session: &Session, font: &Font, master: &Master, tab: &Tab) -> (Vec<(String, f64)>, Metrics) {
    let hidden = hidden(session, font, master, tab);
    if hidden.is_empty() {
        return metrics::of(font, master);
    }
    shown(font, master, &hidden)
}

/// The indexes of the font's metrics whose scope leaves out the glyph `tab` edits.
pub fn hidden(session: &Session, font: &Font, master: &Master, tab: &Tab) -> Vec<usize> {
    if font.metrics.iter().all(|d| scope(d).is_none()) {
        return Vec::new();
    }
    let (Some(glyph), Ok(data)) = (tab.edited_glyph().and_then(|g| font.glyph(g)), session.glyph_data()) else { return Vec::new() };
    out_of_scope(font, &data, glyph, &master.id)
}

/// The indexes of the font's metrics whose scope leaves out `glyph`, its layer rules reading the
/// master `master`.
pub fn out_of_scope(font: &Font, data: &GlyphData, glyph: &Glyph, master: &str) -> Vec<usize> {
    let Ok(env) = fontcraft_glyphquery::Env::new(font, data, Some(master)) else { return Vec::new() };
    font.metrics
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            let Some(text) = scope(d) else { return false };
            let Ok(expr) = fontcraft_glyphquery::parse_predicate(text) else { return false };
            !fontcraft_glyphquery::guide_in_scope(Some(&expr), glyph, &env)
        })
        .map(|(i, _)| i)
        .collect()
}

fn scope(def: &MetricDef) -> Option<&str> {
    def.filter.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// The lines and zones of [`metrics::of`] without the metrics at `hidden`.
fn shown(font: &Font, master: &Master, hidden: &[usize]) -> (Vec<(String, f64)>, Metrics) {
    let defs = || font.metrics.iter().zip(master.metric_values.iter()).enumerate().filter(|(i, _)| !hidden.contains(i)).map(|(_, dv)| dv);
    let lines: Vec<(String, f64)> = defs()
        .filter(|(d, v)| d.kind != metric::ITALIC_ANGLE && v.pos.is_finite())
        .map(|(d, v)| (d.name.clone().unwrap_or_else(|| d.kind.clone()), v.pos))
        .collect();
    // The zones as `Font::alignment_zones` reads them: each metric's overshoot, then the master's own.
    let zones = defs()
        .filter(|(d, v)| d.kind != metric::ITALIC_ANGLE && v.over != 0.0)
        .map(|(_, v)| (v.pos, v.over))
        .chain(master.zones.iter().map(|z| (z.pos, z.size)))
        .filter(|(pos, size)| pos.is_finite() && size.is_finite())
        .map(|(pos, size)| if size >= 0.0 { (pos, pos + size) } else { (pos + size, pos) })
        .collect();
    let metrics = Metrics { lines: lines.iter().map(|(_, y)| *y).collect(), zones };
    (lines, metrics)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use egui::Shape;
    use serde_json::json;

    use crate::FontcraftApp;
    use crate::theme::Tokens;

    /// Runs one 1280 × 800 frame and counts the horizontal lines drawn in the metric-line colour
    /// across the whole canvas.
    fn metric_lines(app: &mut FontcraftApp) -> usize {
        let ctx = egui::Context::default();
        let line = Tokens::dark().metric_line;
        let mut count = 0;
        for _ in 0..2 {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| app.ui(ui));
            out.textures_delta.clear();
            let mut shapes: Vec<Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
            count = 0;
            while let Some(shape) = shapes.pop() {
                match shape {
                    Shape::Vec(list) => shapes.extend(list),
                    Shape::LineSegment { points, stroke }
                        if stroke.color == line && points[0].y == points[1].y && (points[1].x - points[0].x).abs() > 600.0 =>
                    {
                        count += 1
                    }
                    _ => {}
                }
            }
        }
        count
    }

    #[test]
    fn a_scoped_metric_hides_its_line_in_glyphs_outside_the_scope() {
        let mut app = FontcraftApp::default();
        app.run("path.addPath", &json!({"glyph": "A", "nodes": [{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 310, "y": 700}]})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [400, 250]})).unwrap();
        let plain = metric_lines(&mut app);
        assert!(plain >= 5, "the five default metric lines: {plain}");

        app.run("font.metrics.add", &json!({"category": "x-height", "name": "Small x", "scope": "name == 'B'"})).unwrap();
        app.run("font.master.setMetric", &json!({"masters": ["m01"], "metric": "Small x", "position": 420})).unwrap();
        assert_eq!(metric_lines(&mut app), plain, "a metric scoped to B shows no line in A");
        let (font, tab) = (app.session.font().unwrap(), app.state.tab().unwrap());
        let master = super::super::view::active_master(font, tab).unwrap();
        assert_eq!(super::hidden(&app.session, font, master, tab), vec![5]);
        let (lines, _) = super::of(&app.session, font, master, tab);
        assert!(lines.iter().all(|(_, y)| *y != 420.0), "{lines:?}");

        app.run("font.metrics.set", &json!({"metric": "Small x", "scope": "name == 'A'"})).unwrap();
        assert_eq!(metric_lines(&mut app), plain + 1, "a metric scoped to A shows its line in A");
        app.run("font.metrics.set", &json!({"metric": "Small x", "scope": ""})).unwrap();
        assert_eq!(metric_lines(&mut app), plain + 1, "a metric without a scope shows everywhere");
    }
}
