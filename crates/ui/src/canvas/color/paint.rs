//! The colour layer: the edited glyph's colour picture as one texture under its outline.
//!
//! The picture is rasterised at the canvas's scale rounded up to a half-octave step, so zooming draws the kept texture
//! until the next step. A picture that would take more than [`MAX_TEXTURE_PIXELS`] pixels, or a side longer than
//! [`MAX_TEXTURE_SIDE`], is rasterised at a smaller scale and drawn stretched.

use craft::ui::primitives::TextureCache;
use egui::{Color32, ColorImage, Pos2, Rect, TextureOptions};
use fontcraft_geom::{Affine, Point, Shape as _};

use super::raster::{Pixels, paint_drawables};
use super::{ColorPicture, content::Drawable};
use crate::canvas::layers::PaintCx;

/// Most pixels of one colour texture.
pub const MAX_TEXTURE_PIXELS: f64 = 4_194_304.0;
/// Longest side of one colour texture, in pixels.
pub const MAX_TEXTURE_SIDE: f64 = 4096.0;

/// The box of the fills' outer outlines, in font units; `None` when it is empty or not finite.
fn bounds(fills: &[Drawable]) -> Option<fontcraft_geom::Rect> {
    let b = fills.iter().filter_map(|d| d.clips.first()).filter(|c| !c.elements().is_empty()).map(|c| c.bounding_box()).reduce(|a, b| a.union(b))?;
    let finite = [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite());
    (finite && b.width() > 0.0 && b.height() > 0.0).then_some(b)
}

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

/// The picture rasterised at `scale` pixels per font unit into `width × height` premultiplied pixels, the box's top
/// left at the image's top left.
pub fn rasterise(picture: &ColorPicture, bounds: fontcraft_geom::Rect, scale: f64, width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0u8; width as usize * height as usize * 4];
    let to_pixels = Affine::new([scale, 0.0, 0.0, -scale, -bounds.x0 * scale, bounds.y1 * scale]);
    paint_drawables(&mut Pixels { data: &mut data, width, height }, &picture.fills, to_pixels, picture.text);
    data
}

pub fn paint(cx: &PaintCx<'_>) {
    let Some(picture) = cx.model.color.as_ref() else { return };
    let Some(b) = bounds(&picture.fills) else { return };
    let ppp = f64::from(cx.painter.ctx().pixels_per_point());
    let Some((scale, width, height)) = texture_scale(cx.camera.scale() * ppp, b) else { return };
    let key = ("fontcraftColor", picture.key, scale.to_bits());
    let image = || {
        let pixels = rasterise(picture, b, scale, width, height);
        ColorImage::from_rgba_premultiplied([width as usize, height as usize], &pixels)
    };
    let texture = TextureCache::texture_with(cx.painter.ctx(), key, TextureOptions::LINEAR, image);
    let (top_left, bottom_right) = (cx.camera.to_screen(Point::new(b.x0, b.y1)), cx.camera.to_screen(Point::new(b.x1, b.y0)));
    if !(top_left.x.is_finite() && top_left.y.is_finite() && bottom_right.x.is_finite() && bottom_right.y.is_finite()) {
        return;
    }
    // White is the identity tint: the texture's own colours show.
    cx.painter.image(texture, Rect::from_two_pos(top_left, bottom_right), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
}
