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
use fontcraft_geom::{Affine, BezPath, Point, Shape as _};
use fontcraft_render::color::artwork::{self, Drawable, Item, Leaf};

use super::{Bitmap, ColorArt, Content};
use crate::canvas::layers::PaintCx;

/// Most pixels of one colour texture.
pub const MAX_TEXTURE_PIXELS: f64 = 4_194_304.0;
/// Longest side of one colour texture, in pixels.
pub const MAX_TEXTURE_SIDE: f64 = 4096.0;
/// Pixel operations (a piece's pixel box times its passes, summed over the pieces) one glyph may take to draw: at
/// least this, or [`PAINT_WORK_PER_PIXEL`] per pixel of the glyph's own box when that is more, up to
/// [`MAX_GLYPH_PAINT_WORK`].
pub const MAX_PAINT_WORK: u64 = 64 << 20;
/// Pixel operations one pixel of a glyph's box allows, for glyphs drawn large.
pub const PAINT_WORK_PER_PIXEL: u64 = 256;
/// Most pixel operations one glyph may take to draw, however large it is drawn.
pub const MAX_GLYPH_PAINT_WORK: u64 = 1 << 30;

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

/// The pixel operations `items` take in a `width × height` image, roughly as the rasteriser counts them, and the
/// budget they get.
fn work(items: &[Item], to_pixels: Affine, width: u32, height: u32) -> (u64, u64) {
    let image = fontcraft_geom::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let area = |r: fontcraft_geom::Rect| {
        let r = to_pixels.transform_rect_bbox(r).intersect(image);
        let a = r.width().max(0.0) * r.height().max(0.0);
        if a.is_finite() { a.ceil() as u64 } else { u64::MAX }
    };
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let cost = items.iter().fold(0u64, |sum, item| {
        let Some(rect) = item.bounds() else { return sum };
        let passes = match item {
            Item::Fill(d) => count(d.clips.len()).saturating_add(1),
            Item::Shadow { outlines, .. } => count(outlines.len()).saturating_add(8),
            Item::InnerShadow { .. } => 10,
            Item::Pattern { .. } => 2,
        };
        sum.saturating_add(area(rect).saturating_mul(passes))
    });
    let whole = artwork::bounds(items).map_or(0, area);
    (cost, MAX_PAINT_WORK.max(PAINT_WORK_PER_PIXEL.saturating_mul(whole).min(MAX_GLYPH_PAINT_WORK)))
}

/// The outer outlines of `items` joined as one path, each turned to one winding so that layers wound in opposite
/// directions add up under the nonzero rule.
fn silhouette(items: &[Item]) -> BezPath {
    let mut all = BezPath::new();
    let mut add = |p: &BezPath| {
        let p = if p.area() < 0.0 { p.reverse_subpaths() } else { p.clone() };
        all.extend(p.elements().iter().copied());
    };
    for item in items {
        match item {
            Item::Fill(d) => d.clips.first().into_iter().for_each(&mut add),
            Item::Shadow { outlines, .. } => outlines.iter().for_each(&mut add),
            Item::InnerShadow { outline, .. } | Item::Pattern { outline, .. } => add(outline),
        }
    }
    all
}

/// `items` drawn into `width × height` premultiplied pixels; `to_pixels` maps font units to pixels. Past the work
/// budget, or when the artwork is refused, the outlines are filled in `plain`. Answers the pixels and whether the
/// artwork itself was drawn.
pub fn rasterise(items: &[Item], to_pixels: Affine, width: u32, height: u32, plain: [f32; 4]) -> (Vec<u8>, bool) {
    let blank = || vec![0u8; width as usize * height as usize * 4];
    let (cost, budget) = work(items, to_pixels, width, height);
    if cost <= budget
        && let Ok(image) = artwork::rasterize(items, to_pixels, width, height)
    {
        return (image.pixels, true);
    }
    let fill = Item::Fill(Drawable { clips: vec![silhouette(items)], leaf: Leaf::Solid(plain) });
    let pixels = artwork::rasterize(std::slice::from_ref(&fill), to_pixels, width, height).map_or_else(|_| blank(), |i| i.pixels);
    (pixels, false)
}

/// Draws `art` with `painter`: `to_screen` maps its font units to screen points (y down), without rotation.
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
    let (top_left, bottom_right) = (screen(to_screen, Point::new(b.x0, b.y1)), screen(to_screen, Point::new(b.x1, b.y0)));
    if !(finite(top_left) && finite(bottom_right)) || !painter.clip_rect().intersects(Rect::from_two_pos(top_left, bottom_right)) {
        return;
    }
    let image = || {
        let to_pixels = Affine::new([scale, 0.0, 0.0, -scale, -b.x0 * scale, b.y1 * scale]);
        let (pixels, _) = rasterise(items, to_pixels, width, height, plain);
        ColorImage::from_rgba_premultiplied([width as usize, height as usize], &pixels)
    };
    let texture = TextureCache::texture_with(painter.ctx(), ("fontcraftColor", key, scale.to_bits()), TextureOptions::LINEAR, image);
    painter.image(texture, Rect::from_two_pos(top_left, bottom_right), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), UNTINTED);
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
    let mut mesh = Mesh::with_texture(texture);
    let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    for (pos, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(Vertex { pos, uv, color: UNTINTED });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
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
