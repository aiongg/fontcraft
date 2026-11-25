//! Colour content as pixels: vector artwork within a work budget, iColor images placed, and both laid over an image.
//!
//! One glyph's drawing has a work budget ([`MAX_PAINT_WORK`], more for glyphs drawn large); a glyph that needs more is
//! drawn as one plain fill of its outlines, so a file of many large layers cannot stall a frame and no glyph is shown
//! with layers missing.

use fontcraft_geom::{Affine, BezPath, Rect, Shape as _};
use fontcraft_render::Image;
use fontcraft_render::color::artwork::{self, Drawable, Item, Leaf};
use fontcraft_render::color::{composite_image, draw_bitmap};

use super::{Content, PlacedColor};

/// Pixel operations (a piece's pixel box times its passes, summed over the pieces) one glyph may take to draw: at
/// least this, or [`PAINT_WORK_PER_PIXEL`] per pixel of the glyph's own box when that is more, up to
/// [`MAX_GLYPH_PAINT_WORK`].
pub const MAX_PAINT_WORK: u64 = 64 << 20;
/// Pixel operations one pixel of a glyph's box allows, for glyphs drawn large.
pub const PAINT_WORK_PER_PIXEL: u64 = 256;
/// Most pixel operations one glyph may take to draw, however large it is drawn.
pub const MAX_GLYPH_PAINT_WORK: u64 = 1 << 30;

/// The pixel operations `items` take in a `width × height` image, roughly as the rasteriser counts them, and the
/// budget they get.
fn work(items: &[Item], to_pixels: Affine, width: u32, height: u32) -> (u64, u64) {
    let image = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let area = |r: Rect| {
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

/// Lays each drawing of `colors` over `image`, a picture of the line they are placed on: `to_pixels` maps the line's
/// font units to pixels. A drawing outside the image is passed over. `plain` is the colour of a glyph past its work
/// budget, straight RGBA.
pub fn draw_over(image: &mut Image, colors: &[PlacedColor], to_pixels: Affine, plain: [f32; 4]) {
    for placed in colors {
        let place = to_pixels * Affine::translate((placed.at[0], placed.at[1]));
        match &placed.art.content {
            Content::Items(items) => items_over(image, items, place, plain),
            Content::Bitmap { bitmap, place: own } => draw_bitmap(image, &bitmap.image, place * *own),
        }
    }
}

fn items_over(image: &mut Image, items: &[Item], to_pixels: Affine, plain: [f32; 4]) {
    let Some(bounds) = artwork::bounds(items) else { return };
    let b = to_pixels.transform_rect_bbox(bounds);
    if ![b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) {
        return;
    }
    // Rounded out, so the picture is a whole number of pixels and falls on the image's own.
    let (x0, y0) = (b.x0.max(0.0).floor(), b.y0.max(0.0).floor());
    let (x1, y1) = (b.x1.min(f64::from(image.width)).ceil(), b.y1.min(f64::from(image.height)).ceil());
    if !(x1 > x0 && y1 > y0) {
        return;
    }
    let (width, height) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let (pixels, _) = rasterise(items, Affine::translate((-x0, -y0)) * to_pixels, width, height, plain);
    if let Ok(art) = Image::from_rgba_premultiplied(width, height, pixels) {
        composite_image(image, &art, x0 as i64, y0 as i64);
    }
}
