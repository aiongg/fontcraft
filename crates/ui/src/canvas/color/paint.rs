//! Drawing colour art: vector artwork rasterised by `fontcraft_render::color::artwork` into one texture per glyph and
//! zoom step, iColor images as textured quads. The canvas layers ([`paint`], [`text`]) and Font View cells draw
//! through [`draw`].
//!
//! Vector art is rasterised at the screen's scale rounded up to a half-octave step, so zooming draws the kept texture
//! until the next step. A picture that would take more than [`MAX_TEXTURE_PIXELS`] pixels, or a side longer than
//! [`MAX_TEXTURE_SIDE`], is rasterised at a smaller scale and drawn stretched. One glyph's drawing has a work budget
//! ([`MAX_PAINT_WORK`], more for glyphs drawn large); a glyph that needs more is drawn as one plain fill of its
//! outlines in the text colour, so a file of many large layers cannot stall a frame and no glyph is shown with layers
//! missing.

use craft::ui::primitives::TextureCache;
use egui::epaint::Vertex;
use egui::{Color32, ColorImage, Mesh, Painter, Pos2, Rect, Shape, TextureOptions, pos2};
pub use fontcraft_engine::color_drawing::raster::{MAX_GLYPH_PAINT_WORK, MAX_PAINT_WORK, PAINT_WORK_PER_PIXEL, rasterise};
use fontcraft_geom::{Affine, Point};
use fontcraft_render::color::artwork::{self, Item};

use super::{Bitmap, ColorArt, Content};
use crate::canvas::layers::PaintCx;

/// Most pixels of one colour texture.
pub const MAX_TEXTURE_PIXELS: f64 = 4_194_304.0;
/// Longest side of one colour texture, in pixels.
pub const MAX_TEXTURE_SIDE: f64 = 4096.0;

/// White is the identity tint: a texture's own colours show.
const UNTINTED: Color32 = Color32::WHITE;

/// The texture's pixels per font unit for a canvas at `scale` pixels per font unit, and its width and height.
pub fn texture_scale(scale: f64, bounds: fontcraft_geom::Rect) -> Option<(f64, u32, u32)> {
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    let step = (scale.log2() * 2.0).ceil() / 2.0;
    let mut s = step.exp2();
    let (w, h) = (bounds.width(), bounds.height());
    let over = ((w * s) * (h * s) / MAX_TEXTURE_PIXELS).sqrt().max(w * s / MAX_TEXTURE_SIDE).max(h * s / MAX_TEXTURE_SIDE);
    if over > 1.0 {
        s /= over;
    }
    let (pw, ph) = ((w * s).ceil().max(1.0), (h * s).ceil().max(1.0));
    (pw.is_finite() && ph.is_finite()).then_some((s, pw as u32, ph as u32))
}

/// Draws `art` with `painter`: `to_screen` maps its font units to screen points (y down); a half turn (the Space
/// Center upside down) turns the drawing with it.
/// `plain` is the text colour, the fill of vector art past its work budget.
pub fn draw(painter: &Painter, art: &ColorArt, to_screen: Affine, plain: [f32; 4]) {
    match &art.content {
        Content::Items(items) => draw_items(painter, art.key, items, to_screen, plain),
        Content::Bitmap { bitmap, place } => draw_bitmap(painter, bitmap, to_screen * *place),
    }
}

fn finite(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn screen(to_screen: Affine, p: Point) -> Pos2 {
    let q = to_screen * p;
    pos2(q.x as f32, q.y as f32)
}

fn draw_items(painter: &Painter, key: u64, items: &[Item], to_screen: Affine, plain: [f32; 4]) {
    let Some(b) = artwork::bounds(items) else { return };
    if !(b.width() > 0.0 && b.height() > 0.0) {
        return;
    }
    let ppp = f64::from(painter.ctx().pixels_per_point());
    let Some((scale, width, height)) = texture_scale(to_screen.determinant().abs().sqrt() * ppp, b) else { return };
    // The texture's corners, top left first, where they land on screen.
    let corners = [Point::new(b.x0, b.y1), Point::new(b.x1, b.y1), Point::new(b.x1, b.y0), Point::new(b.x0, b.y0)].map(|p| screen(to_screen, p));
    if !corners.iter().all(|p| finite(*p)) || !painter.clip_rect().intersects(Rect::from_points(&corners)) {
        return;
    }
    let image = || {
        let to_pixels = Affine::new([scale, 0.0, 0.0, -scale, -b.x0 * scale, b.y1 * scale]);
        let (pixels, _) = rasterise(items, to_pixels, width, height, plain);
        ColorImage::from_rgba_premultiplied([width as usize, height as usize], &pixels)
    };
    let texture = TextureCache::texture_with(painter.ctx(), ("fontcraftColor", key, scale.to_bits()), TextureOptions::LINEAR, image);
    textured_quad(painter, texture, corners);
}

/// A texture drawn on the quad `corners` (the texture's top left, top right, bottom right and bottom left on screen).
fn textured_quad(painter: &Painter, texture: egui::TextureId, corners: [Pos2; 4]) {
    let mut mesh = Mesh::with_texture(texture);
    let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    for (pos, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(Vertex { pos, uv, color: UNTINTED });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// An iColor image as a textured quad: `to_screen` maps the image's pixels (y up from its bottom left) to screen
/// points, so a rotated or slanted placement draws as placed.
fn draw_bitmap(painter: &Painter, bitmap: &Bitmap, to_screen: Affine) {
    let image = &bitmap.image;
    let (w, h) = (f64::from(image.width), f64::from(image.height));
    let corners = [Point::new(0.0, h), Point::new(w, h), Point::new(w, 0.0), Point::new(0.0, 0.0)].map(|p| screen(to_screen, p));
    if !corners.iter().all(|p| finite(*p)) || !painter.clip_rect().intersects(Rect::from_points(&corners)) {
        return;
    }
    let size = [image.width as usize, image.height as usize];
    let key = ("fontcraftColorBitmap", std::sync::Arc::as_ptr(image) as usize, image.width, image.height);
    let texture = TextureCache::texture_with(painter.ctx(), key, TextureOptions::LINEAR, || ColorImage::from_rgba_unmultiplied(size, &image.pixels));
    textured_quad(painter, texture, corners);
}

/// The edited glyph's colour picture, under its outline.
pub fn paint(cx: &PaintCx<'_>) {
    let Some(picture) = cx.model.color.as_ref() else { return };
    draw(cx.painter, &picture.art, cx.camera.affine(), picture.text);
}

/// The edit view's other glyphs in colour, under their outlines.
pub fn text(cx: &PaintCx<'_>) {
    if !cx.text.shown {
        return;
    }
    let to_screen = cx.camera.affine();
    for placed in &cx.text.colors {
        draw(cx.painter, &placed.art, to_screen * Affine::translate((placed.at[0], placed.at[1])), super::TEXT);
    }
}
