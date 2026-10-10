//! Filled glyphs on the canvas. egui fills only convex shapes, so filled outlines (with their
//! counters) are rasterised by the `render` crate, which fills closed contours only, into one
//! canvas-sized texture, rebuilt only when the document, the tab or the camera changes.

use std::hash::{Hash, Hasher};

use egui::{Color32, ColorImage, Rect, Stroke, TextureHandle, TextureOptions, pos2};
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::Font;
use fontcraft_render::{Style, render_path};

use super::{Painting, flatten};
use crate::state::Tab;

/// The last fill texture and what it was drawn from.
#[derive(Default)]
pub struct FillCache {
    key: Option<u64>,
    texture: Option<TextureHandle>,
}

/// What to fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fill {
    /// The document's id and revision, so an edit redraws the texture.
    pub doc: (u64, u64),
    /// Fill the glyphs other than the edited one (else they are drawn as thin outlines).
    pub others: bool,
    /// Fill the edited glyph too.
    pub edited: bool,
}

/// Draws the tab's glyphs (`line` holds their offsets and widths) as `what` says.
pub fn draw(c: &Painting, cache: &mut FillCache, font: &Font, master: &str, tab: &Tab, line: &[(f64, f64)], what: Fill) {
    let offset = line.get(tab.edited).map_or(0.0, |(x, _)| *x);
    let mut filled = BezPath::new();
    for (i, (name, (x, _))) in tab.glyphs.iter().zip(line).enumerate() {
        let edited = i == tab.edited;
        let Ok(mut outline) = font.outline(name, master) else { continue };
        outline.apply_affine(Affine::translate((x - offset, 0.0)));
        if (edited && what.edited) || (!edited && what.others) {
            filled.extend(outline.elements().iter().copied());
        } else if !edited {
            let to_screen = Affine::translate(c.view.origin.to_vec2()) * Affine::scale_non_uniform(c.view.scale(), -c.view.scale());
            for contour in flatten(&(to_screen * outline)) {
                c.painter.add(egui::Shape::line(contour, Stroke::new(c.tokens.metric_line_width, c.tokens.glyph_fill)));
            }
        }
    }
    if filled.elements().is_empty() {
        return;
    }
    let ppp = c.painter.ctx().pixels_per_point();
    let (w, h) = ((c.rect.width() * ppp).round(), (c.rect.height() * ppp).round());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (what, &tab.glyphs, tab.edited).hash(&mut hasher);
    for v in [
        c.view.scale(),
        c.view.origin.x,
        c.view.origin.y,
        f64::from(c.rect.min.x),
        f64::from(c.rect.min.y),
        f64::from(w),
        f64::from(h),
        f64::from(ppp),
    ] {
        v.to_bits().hash(&mut hasher);
    }
    c.tokens.glyph_fill.to_array().hash(&mut hasher);
    let key = hasher.finish();
    if cache.key != Some(key) {
        cache.key = Some(key);
        cache.texture = rasterise(c, &filled, w, h, ppp);
    }
    if let Some(texture) = &cache.texture {
        c.painter.image(texture.id(), c.rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The filled outline as a texture the size of the canvas in pixels; `None` when it cannot be
/// drawn (an empty or oversized canvas).
fn rasterise(c: &Painting, outline: &BezPath, w: f32, h: f32, ppp: f32) -> Option<TextureHandle> {
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    let (width, height) = (w as u32, h as u32);
    let to_pixels = Affine::scale(f64::from(ppp))
        * Affine::translate((-f64::from(c.rect.min.x), -f64::from(c.rect.min.y)))
        * Affine::translate(c.view.origin.to_vec2())
        * Affine::scale_non_uniform(c.view.scale(), -c.view.scale());
    let [r, g, b, a] = c.tokens.glyph_fill.to_srgba_unmultiplied();
    let style = Style { ink: [r, g, b, a], background: [0, 0, 0, 0], anti_alias: true };
    let image = render_path(outline, to_pixels, width, height, &style).ok()?;
    let color = ColorImage::from_rgba_premultiplied([width as usize, height as usize], &image.pixels);
    Some(c.painter.ctx().load_texture("edit-view-fill", color, TextureOptions::LINEAR))
}
