//! Placed images and colour glyphs in Font View cells, and images dropped on cells (the same as
//! `glyph.addImage`). Package M11.16 (`docs/plans/M11.md` § 4); rows
//! `fontView.cell.imageDisplay`, `fontView.image.dropOnCell`, `fontView.image.showImageOption`,
//! `image.dragAndDrop` (cells), `color.preview.paletteDrivesFontView`.
//!
//! What a cell draws, bottom first:
//! - The image placed on the glyph's layer of Font View's master, while View › Show Image is on,
//!   and always when the layer has no paths and no components. It is drawn as Edit View draws it
//!   (its transform, crop and opacity), clipped to the cell. A file that is missing or cannot be
//!   decoded draws the missing-image marker, a kept-only format (PSD) the marker with its name.
//! - The glyph in colour, in place of its ordinary outline, when the glyph has colour layers in
//!   that master (palette and full-colour layers, else an svg layer, else the largest iColor
//!   image), or when the master has a Master Color: then the layers of every master with a
//!   Master Color are drawn, each filled in its colour, the first master on top (Master Color
//!   Dark on a dark canvas). Palette colours come from palette 0 of the master's palettes.
//!
//! Dropping image files on Font View ([`dropped`]): one file goes on the layer of Font View's
//! master of the glyph under the pointer; several files go on the glyphs their names match, as
//! Add Image does with several files.
//!
//! Cost, for fonts of thousands of glyphs (`docs/plans/M11.md` § 5: 500 image cells in 16 ms):
//! - Images are decoded by the same code as Edit View's (`canvas::images::decode`) and
//!   kept as thumbnails no larger than the cell (a power of two from [`MIN_THUMB_SIDE`] to
//!   [`MAX_THUMB_SIDE`] pixels), so a screenful of cells holds little texture memory. A frame
//!   decodes at most [`MAX_DECODES_PER_FRAME`] files, and none after it has decoded
//!   [`MAX_DECODE_PIXELS_PER_FRAME`] pixels (a large scan costs far more than a small file); the
//!   rest are drawn in later frames.
//! - Colour glyphs are rasterised by Edit View's colour drawing
//!   ([`crate::canvas::color::glyph_image`]) into one texture per glyph and cell size, kept
//!   while the glyph, its master, the palettes and the cell are unchanged. A frame makes at most
//!   [`MAX_COLOR_PER_FRAME`] of them.
//! - Both caches are bounded in entries and bytes and drop the least recently drawn entries.
//!   Switching documents empties them. When the window gets the focus back, each thumbnail is
//!   checked against its file's version ([`AssetStore::version`]: its modification time and
//!   size) as it is next drawn, and only a changed file is decoded again.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::epaint::Vertex;
use egui::{Color32, ColorImage, Mesh, Pos2, Rect, Shape, Stroke, TextureHandle, TextureId, TextureOptions, pos2, vec2};
use fontcraft_engine::color::layers::{self as kinds, ColorLayerKind};
use fontcraft_engine::color::{AssetStore, Rgba, palette};
use fontcraft_engine::imagery::ImageFormat;
use fontcraft_geom::{Affine, Point};
use fontcraft_model::{Font, Glyph, Image, LayerSide};
use fontcraft_tools::image::ImageGeom;
use serde_json::{Map, Value, json};

use super::images::Frame;
use crate::canvas::color::{self, Mode, Sources, SvgCache};
use crate::canvas::images::{CachedImage, DEFAULT_ALPHA, decode, shrink, side_empty, side_image};
use crate::theme::Tokens;

/// Most image thumbnails kept.
pub const MAX_THUMBS: usize = 4_096;
/// Most bytes of thumbnail pixels kept.
pub const MAX_THUMB_BYTES: usize = 128 << 20;
/// Smallest and largest thumbnail side, in pixels.
pub const MIN_THUMB_SIDE: u32 = 32;
pub const MAX_THUMB_SIDE: u32 = 1_024;
/// Most image files read and decoded in one frame.
pub const MAX_DECODES_PER_FRAME: usize = 4;
/// Decoded pixels after which a frame decodes no more files: about 16 Mpx, a few large scans.
pub const MAX_DECODE_PIXELS_PER_FRAME: u64 = 16 << 20;
/// Most colour glyph textures kept, and their bytes.
pub const MAX_SPRITES: usize = 4_096;
pub const MAX_SPRITE_BYTES: usize = 128 << 20;
/// Largest side of a colour glyph texture, in pixels.
pub const MAX_SPRITE_SIDE: u32 = 1_024;
/// Most colour glyph textures made in one frame.
pub const MAX_COLOR_PER_FRAME: usize = 32;

/// Which thumbnail: the stored path and the thumbnail's largest side.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ThumbKey {
    path: String,
    side: u32,
}

struct Thumb {
    image: CachedImage,
    bytes: usize,
    used: u64,
    /// The file's version when it was decoded; `None` when the store cannot tell.
    version: Option<u64>,
    /// The focus revision at which `version` was last found current.
    checked: u64,
}

/// A glyph's colour drawing as a cell keeps it.
enum Drawn {
    Texture(TextureHandle),
    /// An iColor image, drawn through the thumbnails.
    Bitmap(Arc<Image>),
    /// The glyph has colour layers but they draw nothing: its outline is drawn as usual.
    Nothing,
}

struct Sprite {
    master: String,
    glyph: Arc<Glyph>,
    doc: (u64, u64),
    /// Its layers hold components, whose outlines depend on other glyphs.
    components: bool,
    stamp: u64,
    side: u32,
    frame: Frame,
    drawn: Drawn,
    bytes: usize,
    used: u64,
}

/// What decides how every colour glyph of a frame looks.
type StyleKey = ((u64, u64), String, bool, u64, Color32);

/// Font View's images and colour glyphs: thumbnails, colour textures and the svg files read.
#[derive(Default)]
pub struct CellArt {
    thumbs: HashMap<ThumbKey, Thumb>,
    thumb_bytes: usize,
    sprites: HashMap<String, Sprite>,
    sprite_bytes: usize,
    svgs: SvgCache,
    /// The document the caches hold.
    doc: Option<u64>,
    /// Raised when the window gets the focus back, so changed files are read again.
    revision: u64,
    /// Pixels decoded in this frame.
    decoded_pixels: u64,
    focused: bool,
    /// The frame counter, for least-recently-drawn eviction.
    frame: u64,
    decoded: usize,
    coloured: usize,
    /// The layered-font mode and the style stamp of the last frame, with what they came from.
    style: Option<(StyleKey, Option<Mode>, u64)>,
}

/// What every cell of one frame shares.
pub struct ArtFrame {
    assets: Arc<dyn AssetStore>,
    doc: (u64, u64),
    master: String,
    frame: Frame,
    /// Set when Font View's master has a Master Color: every glyph is drawn in its masters'
    /// colours.
    layered: Option<Mode>,
    stamp: u64,
    revision: u64,
    show_image: bool,
    text: Color32,
    marker: Color32,
    marker_size: f32,
    marker_width: f32,
}

/// What [`CellArt::paint`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Painted {
    /// The glyph is drawn in colour, or will be next frame: its ordinary outline is not drawn.
    pub colour: bool,
    /// Something waits for the next frame.
    pub deferred: bool,
}

impl CellArt {
    /// Call once per frame before drawing cells.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        ctx: &egui::Context,
        assets: Arc<dyn AssetStore>,
        doc: (u64, u64),
        font: &Font,
        master: &str,
        show_image: bool,
        tokens: &Tokens,
    ) -> ArtFrame {
        if self.doc != Some(doc.0) {
            self.thumbs.clear();
            self.thumb_bytes = 0;
            self.sprites.clear();
            self.sprite_bytes = 0;
            self.svgs.clear();
            self.style = None;
            self.doc = Some(doc.0);
        }
        let focused = ctx.input(|i| i.focused);
        if focused && !self.focused {
            self.revision = self.revision.wrapping_add(1);
        }
        self.focused = focused;
        self.frame = self.frame.wrapping_add(1);
        self.decoded = 0;
        self.decoded_pixels = 0;
        self.coloured = 0;
        let text = tokens.color.text_color;
        let key: StyleKey = (doc, master.to_string(), tokens.dark, self.revision, text);
        let (layered, stamp) = match &self.style {
            Some((k, layered, stamp)) if *k == key => (layered.clone(), *stamp),
            _ => {
                let layered = layered_mode(font, master, tokens.dark);
                let palette = palette::read(font, Some(master)).ok().and_then(|p| p.palette(0).map(<[Rgba]>::to_vec)).unwrap_or_default();
                let mut h = DefaultHasher::new();
                (format!("{layered:?}"), palette, text.to_array(), self.revision).hash(&mut h);
                let stamp = h.finish();
                self.style = Some((key, layered.clone(), stamp));
                (layered, stamp)
            }
        };
        ArtFrame {
            assets,
            doc,
            master: master.to_string(),
            frame: Frame::of(font, master),
            layered,
            stamp,
            revision: self.revision,
            show_image,
            text,
            marker: tokens.images.marker,
            marker_size: tokens.images.marker_size,
            marker_width: tokens.images.marker_width,
        }
    }

    /// Image files read in this frame (tests read it).
    pub fn decoded(&self) -> usize {
        self.decoded
    }

    /// Thumbnails held.
    pub fn thumbs(&self) -> usize {
        self.thumbs.len()
    }

    /// Colour glyph textures held, those that draw nothing included.
    pub fn sprites(&self) -> usize {
        self.sprites.len()
    }

    /// Draws `glyph`'s placed image and its colour drawing into `rect`, the square that holds its
    /// outline. The caller draws the ordinary outline unless [`Painted::colour`] says otherwise.
    pub fn paint(&mut self, f: &ArtFrame, painter: &egui::Painter, font: &Font, glyph: &Arc<Glyph>, rect: Rect) -> Painted {
        let mut out = Painted::default();
        let Some(layer) = glyph.layer(&f.master) else { return out };
        let ppp = painter.ctx().pixels_per_point();
        let px = (rect.width() * ppp).round().clamp(1.0, f32::from(u16::MAX)) as u32;
        let to_screen = cell_transform(f.frame, layer.width, rect);
        if !to_screen.as_coeffs().iter().all(|v| v.is_finite()) {
            return out;
        }
        let painter = painter.with_clip_rect(rect);
        if let Some(image) = side_image(layer, LayerSide::Foreground).filter(|_| f.show_image || side_empty(layer, LayerSide::Foreground)) {
            out.deferred |= self.image(f, &painter, image, to_screen, px, rect);
        }
        if f.layered.is_some() || has_colour_layers(glyph, &f.master) {
            let c = self.colour(f, &painter, font, glyph, layer.width, to_screen, rect, px);
            out.colour = c.colour;
            out.deferred |= c.deferred;
        }
        out
    }

    /// Draws `image` placed by `to_screen` (font units to points). Returns true when its file
    /// waits for a later frame.
    fn image(&mut self, f: &ArtFrame, painter: &egui::Painter, image: &Image, to_screen: Affine, px: u32, cell: Rect) -> bool {
        let place = to_screen * image.transform.affine();
        let Some(path) = image.path.as_deref() else {
            marker(painter, f, cell, place, None);
            return false;
        };
        let key = ThumbKey { path: path.to_string(), side: px.clamp(MIN_THUMB_SIDE, MAX_THUMB_SIDE).next_power_of_two() };
        // After the focus comes back, a thumbnail whose file is unchanged is kept.
        if let Some(thumb) = self.thumbs.get_mut(&key)
            && thumb.checked != f.revision
        {
            let version = f.assets.version(path);
            if version.is_some() && version == thumb.version {
                thumb.checked = f.revision;
            } else if let Some(stale) = self.thumbs.remove(&key) {
                self.thumb_bytes = self.thumb_bytes.saturating_sub(stale.bytes);
            }
        }
        if !self.thumbs.contains_key(&key) {
            if self.decoded >= MAX_DECODES_PER_FRAME || self.decoded_pixels >= MAX_DECODE_PIXELS_PER_FRAME {
                return true;
            }
            self.decoded += 1;
            // Taken before the read, so a change made during it is seen at the next check.
            let version = f.assets.version(path);
            let (image, bytes, pixels) = thumbnail(painter.ctx(), f.assets.as_ref(), path, key.side);
            self.decoded_pixels = self.decoded_pixels.saturating_add(pixels);
            // The other sizes made from another version of the file are stale.
            let mut freed = 0;
            self.thumbs.retain(|k, t| {
                let keep = k.path != key.path || (version.is_some() && t.version == version);
                if !keep {
                    freed += t.bytes;
                }
                keep
            });
            self.thumb_bytes = self.thumb_bytes.saturating_sub(freed).saturating_add(bytes);
            self.thumbs.insert(key.clone(), Thumb { image, bytes, used: self.frame, version, checked: f.revision });
            evict(&mut self.thumbs, &mut self.thumb_bytes, MAX_THUMBS, MAX_THUMB_BYTES, &key, |t| t.used, |t| t.bytes);
        }
        let frame = self.frame;
        let Some(thumb) = self.thumbs.get_mut(&key) else { return false };
        thumb.used = frame;
        match &thumb.image {
            CachedImage::Ready { texture, size } => draw_texture(painter, texture.id(), *size, image, place),
            CachedImage::Unsupported(format) => marker(painter, f, cell, place, Some(*format)),
            CachedImage::Failed(_) => marker(painter, f, cell, place, None),
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn colour(
        &mut self,
        f: &ArtFrame,
        painter: &egui::Painter,
        font: &Font,
        glyph: &Arc<Glyph>,
        width: f64,
        to_screen: Affine,
        cell: Rect,
        px: u32,
    ) -> Painted {
        let side = px.min(MAX_SPRITE_SIDE);
        let valid = self.sprites.get(glyph.name.as_str()).is_some_and(|e| {
            e.master == f.master
                && e.stamp == f.stamp
                && e.side == side
                && e.frame == f.frame
                && (e.doc == f.doc || (e.doc.0 == f.doc.0 && Arc::ptr_eq(&e.glyph, glyph) && !e.components))
        });
        if !valid {
            if self.coloured >= MAX_COLOR_PER_FRAME {
                return Painted { colour: true, deferred: true };
            }
            self.coloured += 1;
            let mode = f.layered.clone().unwrap_or_else(|| Mode::Layers { master: f.master.clone(), kind: ColorLayerKind::FullColor });
            let mut src = Sources { assets: f.assets.as_ref(), svgs: &mut self.svgs, revision: f.revision, text: f.text };
            let (drawn, bytes) = match colour_image(font, &glyph.name, &mode, &mut src, f.frame, width, side) {
                Some(image) => {
                    let bytes = image.pixels.len().saturating_mul(4);
                    (Drawn::Texture(painter.ctx().load_texture(format!("fontView.color:{}", glyph.name), image, TextureOptions::LINEAR)), bytes)
                }
                None => match icolor_image(glyph, &f.master) {
                    Some(image) => (Drawn::Bitmap(Arc::new(image)), 0),
                    None => (Drawn::Nothing, 0),
                },
            };
            let components = glyph.layers.iter().any(|l| l.shapes.iter().any(|s| matches!(s, fontcraft_model::Shape::Component(_))));
            let sprite = Sprite {
                master: f.master.clone(),
                glyph: glyph.clone(),
                doc: f.doc,
                components,
                stamp: f.stamp,
                side,
                frame: f.frame,
                drawn,
                bytes,
                used: self.frame,
            };
            if let Some(old) = self.sprites.insert(glyph.name.clone(), sprite) {
                self.sprite_bytes = self.sprite_bytes.saturating_sub(old.bytes);
            }
            self.sprite_bytes = self.sprite_bytes.saturating_add(bytes);
            evict(&mut self.sprites, &mut self.sprite_bytes, MAX_SPRITES, MAX_SPRITE_BYTES, &glyph.name, |s| s.used, |s| s.bytes);
        }
        let frame = self.frame;
        let Some(entry) = self.sprites.get_mut(glyph.name.as_str()) else { return Painted::default() };
        entry.used = frame;
        entry.doc = f.doc;
        let bitmap = match &entry.drawn {
            Drawn::Texture(texture) => {
                painter.image(texture.id(), cell, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                return Painted { colour: true, deferred: false };
            }
            Drawn::Bitmap(image) => image.clone(),
            Drawn::Nothing => return Painted::default(),
        };
        let deferred = self.image(f, painter, &bitmap, to_screen, px, cell);
        Painted { colour: true, deferred }
    }
}

/// The colour drawing of glyph `name` in `mode` on a square of `side` pixels, placed as a cell
/// places its outline (`width` is the advance width); `None` when it has no visible pixel.
pub(crate) fn colour_image(font: &Font, name: &str, mode: &Mode, src: &mut Sources, frame: Frame, width: f64, side: u32) -> Option<ColorImage> {
    let to_pixels = cell_transform(frame, width, Rect::from_min_size(Pos2::ZERO, vec2(side as f32, side as f32)));
    if !to_pixels.as_coeffs().iter().all(|v| v.is_finite()) || side == 0 {
        return None;
    }
    color::glyph_image(font, name, mode, None, src, side, side, to_pixels).filter(|image| image.pixels.iter().any(|p| p.a() > 0))
}

/// Font units to the square `rect`: the vertical extent from descender to ascender fills its
/// height, the advance width is centred (as the cell's outline texture places it).
fn cell_transform(frame: Frame, width: f64, rect: Rect) -> Affine {
    let height = (frame.ascender - frame.descender).max(1.0);
    let s = f64::from(rect.width()) / height;
    let left = f64::from(rect.center().x) - width * s / 2.0;
    Affine::new([s, 0.0, 0.0, -s, left, f64::from(rect.min.y) + frame.ascender * s])
}

/// The layered-font mode when `master` has a Master Color: the masters with one, bottom first,
/// so the first master is on top.
fn layered_mode(font: &Font, master: &str, dark: bool) -> Option<Mode> {
    font.master(master).ok().and_then(|m| color::master_color(m, dark))?;
    let masters = font.masters.iter().rev().filter_map(|m| color::master_color(m, dark).map(|c| (m.id.clone(), c))).collect();
    Some(Mode::Layered { masters })
}

/// Whether `glyph` has a colour layer in `master`.
fn has_colour_layers(glyph: &Glyph, master: &str) -> bool {
    glyph.layers.iter().any(|l| l.master == master && kinds::export_kind(l).is_some())
}

/// The image of `glyph`'s largest iColor layer in `master` that has a file, drawn whole.
fn icolor_image(glyph: &Glyph, master: &str) -> Option<Image> {
    kinds::color_layers(glyph, master)
        .into_iter()
        .filter(|(l, _)| l.info.attrs.coordinates.is_empty())
        .filter_map(|(l, k)| match k {
            ColorLayerKind::IColor(size) => l.info.image.as_ref().filter(|i| i.path.is_some()).map(|i| (size, i)),
            _ => None,
        })
        .max_by_key(|(size, _)| *size)
        .map(|(_, image)| Image { alpha: Some(100.0), ..image.clone() })
}

/// Decodes the file at `path` into a texture whose longest side is at most `side` pixels: the
/// entry, the bytes it holds and the pixels decoded.
fn thumbnail(ctx: &egui::Context, assets: &dyn AssetStore, path: &str, side: u32) -> (CachedImage, usize, u64) {
    let (image, size) = match decode(assets, path) {
        Ok(decoded) => decoded,
        Err(entry) => return (entry, 0, 0),
    };
    let pixels = u64::from(image.width).saturating_mul(u64::from(image.height));
    let max_side = u32::try_from(ctx.input(|i| i.max_texture_side)).unwrap_or(u32::MAX);
    let image = shrink(image, side.min(max_side));
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(image.pixels.len()) {
        return (CachedImage::Failed("the decoded image has the wrong number of pixels".into()), 0, pixels);
    }
    let color = ColorImage::from_rgba_unmultiplied([w, h], &image.pixels);
    let texture = ctx.load_texture(format!("fontView.image:{path}"), color, TextureOptions::LINEAR);
    (CachedImage::Ready { texture, size }, image.pixels.len(), pixels)
}

/// Drops the least recently drawn entries of `map`, never `keep`, until it is within `max_len`
/// entries and `max_bytes`.
fn evict<K: Clone + Eq + Hash, V>(
    map: &mut HashMap<K, V>,
    bytes: &mut usize,
    max_len: usize,
    max_bytes: usize,
    keep: &K,
    used: impl Fn(&V) -> u64,
    size: impl Fn(&V) -> usize,
) {
    while *bytes > max_bytes || map.len() > max_len {
        let oldest = map.iter().filter(|(k, _)| *k != keep).min_by_key(|(_, v)| used(v)).map(|(k, _)| k.clone());
        let Some(k) = oldest else { break };
        if let Some(v) = map.remove(&k) {
            *bytes = bytes.saturating_sub(size(&v));
        }
    }
}

/// Draws the texture of an image whose own size is `size`, placed by `place` (its own space to
/// points), cropped and at its opacity, as Edit View draws it.
fn draw_texture(painter: &egui::Painter, texture: TextureId, size: [u32; 2], image: &Image, place: Affine) {
    let (fw, fh) = (f64::from(size[0]), f64::from(size[1]));
    let geom = ImageGeom::new(image, Some((fw, fh)));
    let Some(r) = geom.visible() else { return };
    let alpha = image.alpha.filter(|a| a.is_finite()).unwrap_or(DEFAULT_ALPHA).clamp(0.0, 100.0) / 100.0;
    let color = Color32::WHITE.gamma_multiply(alpha as f32);
    let mut mesh = Mesh::with_texture(texture);
    for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)] {
        let p = place * Point::new(x, y);
        // Texture rows run top to bottom; the image's own y runs up.
        let uv = pos2((x / fw) as f32, (1.0 - y / fh) as f32);
        mesh.vertices.push(Vertex { pos: Pos2::new(p.x as f32, p.y as f32), uv, color });
    }
    if mesh.vertices.iter().all(|v| v.pos.x.is_finite() && v.pos.y.is_finite()) {
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
    }
}

/// The missing-image marker at the image's origin, sized to the cell: a square with a cross, or
/// with the format's name for a format kept as a reference.
fn marker(painter: &egui::Painter, f: &ArtFrame, cell: Rect, place: Affine, format: Option<ImageFormat>) {
    let at = place * Point::ZERO;
    if !at.x.is_finite() || !at.y.is_finite() {
        return;
    }
    let s = f.marker_size.min(cell.width() / 3.0).max(1.0);
    let rect = Rect::from_min_size(Pos2::new(at.x as f32, at.y as f32 - s), vec2(s, s));
    let stroke = Stroke::new(f.marker_width, f.marker);
    painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
    match format {
        Some(format) => {
            let name = format!("{format:?}").to_uppercase();
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(s * 0.35), f.marker);
        }
        None => {
            painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
            painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
        }
    }
}

/// Queues Add Image for image files dropped on Font View (`area`, in window points): one file
/// goes on the layer of `master` of the glyph whose cell or row (`cells`, as laid out this
/// frame) is under the pointer; several go on the glyphs their names match.
pub fn dropped(ctx: &egui::Context, area: Rect, cells: &[(String, Rect)], master: &str, pending: &mut Vec<(&'static str, Value)>) {
    let (files, at) = ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.hover_pos()));
    if files.is_empty() || at.is_some_and(|p| !area.contains(p)) {
        return;
    }
    let paths: Vec<String> = files
        .iter()
        .map(|f| f.path())
        // The web build gives a file name only; its bytes would need the memory store.
        .filter(|p| p.is_absolute())
        .map(|p| p.to_string_lossy().into_owned())
        .take(fontcraft_engine::assets::MAX_ADD_IMAGE_FILES)
        .collect();
    let mut m = Map::new();
    match paths.as_slice() {
        [] => return,
        [one] => {
            let Some(p) = at else { return };
            let Some((glyph, _)) = cells.iter().find(|(_, r)| r.contains(p)) else { return };
            m.insert("files".into(), json!([one]));
            m.insert("glyphs".into(), json!([glyph]));
        }
        many => {
            m.insert("files".into(), json!(many));
        }
    }
    m.insert("layer".into(), json!(master));
    pending.push(("glyph.addImage", Value::Object(m)));
}

#[cfg(test)]
#[path = "cell_images_tests.rs"]
mod tests;
