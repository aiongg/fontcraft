//! The Edit View camera: where a tab's line of glyphs sits on the canvas.
//!
//! A tab's [`TabView`] holds a scale and the line point at the canvas centre. The tools crate's
//! [`fontcraft_tools::View`] is the transform for the edited glyph, whose origin sits at its
//! offset in the line; [`tools_view`] turns one into the other, so the canvas and the tools share
//! one transform.

use fontcraft_geom::Point;
use fontcraft_model::{Font, Master, metric};
use fontcraft_tools::View;

use crate::state::{Tab, TabView};

/// Where each glyph of a tab starts on the line, and its advance width (0 for a missing glyph).
pub fn layout(font: &Font, master: &str, glyphs: &[String]) -> Vec<(f64, f64)> {
    let mut x = 0.0;
    glyphs
        .iter()
        .map(|name| {
            let width = font.glyph(name).and_then(|g| g.layer(master)).map_or(0.0, |l| if l.width.is_finite() { l.width.max(0.0) } else { 0.0 });
            let at = x;
            x += width;
            (at, width)
        })
        .collect()
}

/// The first master: the layer every tab edits until master switching exists.
pub fn active_master(font: &Font) -> Option<&Master> {
    font.masters.first()
}

/// Where the edited glyph starts on the line.
pub fn edited_offset(font: &Font, tab: &Tab) -> f64 {
    let Some(master) = active_master(font) else { return 0.0 };
    layout(font, &master.id, &tab.glyphs).get(tab.edited).map_or(0.0, |(x, _)| *x)
}

/// The edited glyph's transform on a canvas whose centre is `center` (screen points).
pub fn tools_view(view: &TabView, offset: f64, center: Point) -> View {
    let s = TabView::clamped_scale(view.scale);
    View::new(s, Point::new(center.x + (offset - view.center[0]) * s, center.y + view.center[1] * s))
}

/// Descender and ascender of a master, in font units.
pub fn vertical_span(font: &Font, master: &Master) -> (f64, f64) {
    let upm = f64::from(font.units_per_em);
    let ascender = font.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let descender = font.metric_pos(master, metric::DESCENDER, -upm * 0.2);
    if ascender > descender { (descender, ascender) } else { (-upm * 0.2, upm * 0.8) }
}

/// The camera that fits the edited glyph's descender-to-ascender span (and its width, when that
/// is the tighter limit) into a canvas of `canvas` points, leaving `margin` (a fraction) free on
/// every side.
pub fn fit(font: &Font, tab: &Tab, canvas: [f64; 2], margin: f64) -> TabView {
    let Some(master) = active_master(font) else { return TabView::default() };
    let (descender, ascender) = vertical_span(font, master);
    let (offset, width) = layout(font, &master.id, &tab.glyphs).get(tab.edited).copied().unwrap_or((0.0, 0.0));
    let mut v = zoom_to_rect([offset, descender, width, ascender - descender], canvas, margin);
    if width <= 0.0 {
        v.center[0] = offset;
    }
    v
}

/// The camera that shows the line rectangle `[x, y, width, height]` as large as fits.
pub fn zoom_to_rect(rect: [f64; 4], canvas: [f64; 2], margin: f64) -> TabView {
    let [x, y, w, h] = rect;
    let room = 1.0 - 2.0 * margin.clamp(0.0, 0.45);
    let fits = |space: f64, size: f64| if size > 0.0 && space > 0.0 { space * room / size } else { f64::INFINITY };
    let scale = fits(canvas[0], w).min(fits(canvas[1], h));
    TabView { scale: TabView::clamped_scale(if scale.is_finite() { scale } else { 1.0 }), center: [x + w / 2.0, y + h / 2.0] }
}
