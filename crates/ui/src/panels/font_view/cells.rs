//! The outline of a grid cell. A glyph's contours overlap and hold counters, which a polygon fill cannot show, so
//! the outline is filled by the nonzero rule in horizontal bands: each band's spans become quads of one mesh, in the
//! surface's ink. A glyph with a colour drawing (`canvas::color::cell_color`) is drawn in colour instead.

use craft::ui::primitives::ContentPainterCx;
use egui::{Mesh, Pos2, Rect, Shape, pos2};

use crate::app::models::font_view::CellArt;
use crate::canvas::images::PlacedImage;
use crate::canvas::images::tokens::ImageTokens;

/// Most bands one cell is filled in; a taller cell takes bands of more than a point.
const MAX_BANDS: usize = 256;
/// Bands thinner than this, in points, are skipped.
const MIN_BAND: f32 = 0.01;
/// The share of the cell the font's ascender-to-descender height takes.
const FILL_SHARE: f32 = 0.8;
/// The status bar's height: this share of the cell, at least [`STATUS_MIN`] points.
const STATUS_SHARE: f32 = 0.06;
const STATUS_MIN: f32 = 2.0;

/// An edge in screen points, top end first, with its winding: +1 when the contour runs down it.
#[derive(Clone, Copy)]
struct Edge {
    top: Pos2,
    bottom: Pos2,
    winding: i32,
}

impl Edge {
    fn x_at(&self, y: f32) -> f32 {
        let y = y.clamp(self.top.y, self.bottom.y);
        let dy = self.bottom.y - self.top.y;
        if dy <= f32::EPSILON { self.top.x } else { self.top.x + (y - self.top.y) * (self.bottom.x - self.top.x) / dy }
    }
}

/// Where the outline lies in `rect`: the scale from font units to points and the screen position of the origin.
fn placement(art: &CellArt, ascender: f32, descender: f32, rect: Rect) -> Option<(f32, Pos2)> {
    place(art.width, ascender, descender, rect)
}

/// The scale and the glyph origin on screen of a glyph `width` wide in `rect`.
fn place(width: f32, ascender: f32, descender: f32, rect: Rect) -> Option<(f32, Pos2)> {
    let height = ascender - descender;
    if !(height.is_finite() && height > 0.0) {
        return None;
    }
    let mut scale = rect.height() * FILL_SHARE / height;
    if width > 0.0 && width * scale > rect.width() * FILL_SHARE {
        scale = rect.width() * FILL_SHARE / width;
    }
    let left = rect.center().x - width.max(0.0) * scale / 2.0;
    let baseline = rect.center().y + (ascender + descender) / 2.0 * scale;
    (scale.is_finite() && scale > 0.0).then_some((scale, pos2(left, baseline)))
}

/// Draws the glyph's placed image in `rect`, behind the glyph, placed as the glyph `width` wide is, clipped to the
/// cell.
pub fn paint_image(cx: &ContentPainterCx<'_>, rect: Rect, image: &PlacedImage, width: f32, ascender: f32, descender: f32) {
    let Some((scale, origin)) = place(width, ascender, descender, rect) else { return };
    let (s, x, y) = (f64::from(scale), f64::from(origin.x), f64::from(origin.y));
    let to_screen = fontcraft_geom::Affine::new([s, 0.0, 0.0, -s, x, y]);
    let tokens = ImageTokens::standard();
    let tokens = tokens.with_marker((rect.height() * tokens.cell_marker_share).min(tokens.marker_size));
    crate::canvas::images::draw(&cx.painter.with_clip_rect(rect), image, to_screen, &tokens);
}

/// Fills `art` in `rect` with the painter's ink, or draws the glyph in colour when it has a colour drawing.
pub fn paint(cx: &ContentPainterCx<'_>, rect: Rect, art: &CellArt, ascender: f32, descender: f32) {
    let Some((scale, origin)) = placement(art, ascender, descender, rect) else { return };
    if let Some(color) = &art.color {
        let (s, x, y) = (f64::from(scale), f64::from(origin.x), f64::from(origin.y));
        let to_screen = fontcraft_geom::Affine::new([s, 0.0, 0.0, -s, x, y]);
        crate::canvas::color::paint::draw(&cx.painter.with_clip_rect(rect), color, to_screen, crate::canvas::color::TEXT);
        return;
    }
    let to_screen = |[x, y]: [f32; 2]| pos2(origin.x + x * scale, origin.y - y * scale);
    let mut edges = Vec::new();
    for contour in &art.contours {
        let points: Vec<Pos2> = contour.iter().copied().map(to_screen).collect();
        for (i, &a) in points.iter().enumerate() {
            let b = points.get(i + 1).or(points.first()).copied().unwrap_or(a);
            if (a.y - b.y).abs() <= f32::EPSILON {
                continue;
            }
            edges.push(if a.y < b.y { Edge { top: a, bottom: b, winding: 1 } } else { Edge { top: b, bottom: a, winding: -1 } });
        }
    }
    let Some(mesh) = fill(&edges, rect, cx.ink()) else { return };
    cx.painter.add(Shape::mesh(mesh));
}

/// Where the status bar of a cell drawn in `rect` lies: along its foot, the cell's width.
pub fn status_bar(rect: Rect) -> Rect {
    let height = (rect.height() * STATUS_SHARE).max(STATUS_MIN).min(rect.height().max(0.0));
    Rect::from_min_max(pos2(rect.left(), rect.bottom() - height), rect.max)
}

/// Draws the status bar in the status colour `[r, g, b, a]`.
pub fn paint_status(cx: &ContentPainterCx<'_>, rect: Rect, [r, g, b, a]: [u8; 4]) {
    cx.painter.rect_filled(status_bar(rect), 0.0, egui::Color32::from_rgba_unmultiplied(r, g, b, a));
}

/// The nonzero fill of `edges` inside `rect` as one mesh; `None` when nothing is filled.
fn fill(edges: &[Edge], rect: Rect, color: egui::Color32) -> Option<Mesh> {
    let top = edges.iter().map(|e| e.top.y).fold(f32::INFINITY, f32::min).max(rect.top());
    let bottom = edges.iter().map(|e| e.bottom.y).fold(f32::NEG_INFINITY, f32::max).min(rect.bottom());
    if !(top.is_finite() && bottom.is_finite() && bottom - top > MIN_BAND) {
        return None;
    }
    let bands = ((bottom - top).ceil() as usize).clamp(1, MAX_BANDS);
    let step = (bottom - top) / bands as f32;
    let mut mesh = Mesh::default();
    let mut crossing: Vec<(f32, f32, f32, i32)> = Vec::new();
    for band in 0..bands {
        let (y0, y1) = (top + band as f32 * step, top + (band + 1) as f32 * step);
        let mid = (y0 + y1) / 2.0;
        crossing.clear();
        crossing.extend(edges.iter().filter(|e| e.top.y <= mid && e.bottom.y > mid).map(|e| (e.x_at(mid), e.x_at(y0), e.x_at(y1), e.winding)));
        crossing.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut start: Option<(f32, f32)> = None;
        for &(_, x0, x1, w) in &crossing {
            let was = winding;
            winding += w;
            match (was == 0, winding == 0) {
                (true, false) => start = Some((x0, x1)),
                (false, true) => {
                    if let Some((l0, l1)) = start.take() {
                        quad(&mut mesh, [pos2(l0, y0), pos2(x0, y0), pos2(x1, y1), pos2(l1, y1)], color);
                    }
                }
                _ => {}
            }
        }
    }
    (!mesh.is_empty()).then_some(mesh)
}

fn quad(mesh: &mut Mesh, corners: [Pos2; 4], color: egui::Color32) {
    let Ok(base) = u32::try_from(mesh.vertices.len()) else { return };
    for p in corners {
        mesh.colored_vertex(p, color);
    }
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Rect, pos2};

    use super::{Edge, fill};

    fn square(x0: f32, y0: f32, x1: f32, y1: f32, winding: i32) -> [Edge; 2] {
        // The two vertical sides; the horizontal ones add nothing to a band fill.
        let (down, up) =
            (Edge { top: pos2(x1, y0), bottom: pos2(x1, y1), winding }, Edge { top: pos2(x0, y0), bottom: pos2(x0, y1), winding: -winding });
        [down, up]
    }

    /// A counter running the other way stays empty: the band across the middle of an O has two spans.
    #[test]
    fn a_counter_is_left_unfilled() {
        let mut edges = square(0.0, 0.0, 30.0, 30.0, 1).to_vec();
        edges.extend(square(10.0, 10.0, 20.0, 20.0, -1));
        let mesh = fill(&edges, Rect::from_min_max(pos2(0.0, 0.0), pos2(30.0, 30.0)), Color32::WHITE).expect("filled");
        let middle: Vec<f32> = mesh.vertices.iter().filter(|v| (v.pos.y - 15.0).abs() < 0.6).map(|v| v.pos.x).collect();
        assert!(!middle.is_empty());
        assert!(middle.iter().all(|x| *x <= 10.0 + 1e-3 || *x >= 20.0 - 1e-3), "{middle:?}");
    }
}
