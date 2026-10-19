//! Placed images in Edit View: decoded textures, the image drawn behind the outline, the
//! missing-image marker, the selected image with its knobs, the drags on it, a file dropped on
//! the canvas, and the View › Show Image and Reveal Image File commands. M11.14 draws iColor and
//! svg layers through the same cache; M11.16 draws Font View cells with it.
//!
//! Rows: `view.showImage`, `image.revealFile`, `image.alwaysShownForEmptyGlyphs`,
//! `image.directManipulation`, `image.lockedNotSelectable` (the gestures), `image.dragAndDrop`
//! (the canvas), `image.storedAsRelativePath` (the missing-image marker),
//! `editView.imageContextMenu` (with `shell::context_menu`), `infoBox.image` (with
//! `panels::info_box_image`).
//!
//! Drawing:
//! - The edited glyph's image is drawn behind the metric lines and the outline, at its opacity
//!   (`alpha`, 50 % when the file states none), cropped to its crop. While the background is
//!   edited, the background's image is drawn too. Images are shown while Show Image is on, and
//!   always on a side with no paths and no components.
//! - A file that is missing or cannot be read is drawn as the missing-image marker: a square
//!   with a cross at the image's position. A format kept as a reference but not decoded (PSD) is
//!   the same square with the format's name.
//! - PDF images are drawn from their first page at one point per unit of the image's own space,
//!   as Add Image places them.
//!
//! Selection and gestures (geometry in `fontcraft_tools::image`):
//! - A click with the Select tool on the image, where no node, segment, anchor, component or
//!   guide lies, selects the image and nothing else; a click elsewhere, or selecting anything
//!   else, deselects it. A locked image ignores clicks and drags.
//! - Dragging the image moves it; with Show Bounding Box on, its knobs scale it, and `Ctrl+Alt`
//!   on a knob crops it; with the image selected, the Rotate and Scale tools turn and scale it.
//!   Every drag is an interaction whose previews and final step run `image.setTransform` or
//!   `image.setCrop`, one undo step.
//!
//! Contract of the cache (`docs/plans/M11.md` § 3 › Limits):
//! - Bytes come from the session's asset store (`Session::assets`), never from a file opened
//!   here, and are decoded by `fontcraft_imagery` (PDF by `fontcraft_io_pdf`) within their
//!   limits.
//! - Textures are kept up to [`MAX_TEXTURE_BYTES`] of decoded pixels and [`MAX_ENTRIES`]
//!   entries, least recently used evicted. A file that is missing, cannot be decoded or is a
//!   kept-only format (PSD) is cached as such, so it is drawn as a marker without being read
//!   again every frame.
//! - The key's revision is a number the app raises when files on disk may have changed (here:
//!   when the window gets the focus back), so a changed or restored file is read again. An entry
//!   of an older revision of the same path is dropped when the new one is made. Switching
//!   documents empties the cache, as the same stored path may name another file.
//! - A texture larger than the GPU takes is shrunk by a whole factor before upload; the image's
//!   size in [`CachedImage::Ready`] stays its size in its own space.

use std::collections::HashMap;

use egui::epaint::Vertex;
use egui::{Color32, ColorImage, Mesh, Pos2, Rect, Shape, Stroke, TextureHandle, TextureOptions};
use fontcraft_engine::Session;
use fontcraft_engine::color::AssetStore;
use fontcraft_engine::imagery::{self, ImageFormat, Rgba8};
use fontcraft_geom::{Affine, Point};
use fontcraft_model::{Font, Image, Layer, LayerSide, Shape as ModelShape};
use fontcraft_tools::PointerKind;
use fontcraft_tools::image::{DragKind, ImageDrag, ImageGeom};
use serde_json::{Map, Value, json};

use super::Painting;
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};
use crate::state::{Tab, UiState};
use crate::tools_glue::{self, RawPointer, Space};

/// Most bytes of decoded pixels kept as textures.
pub const MAX_TEXTURE_BYTES: usize = 512 << 20;
/// Most entries the cache keeps, textures and markers together.
pub const MAX_ENTRIES: usize = 4_096;
/// The opacity, in percent, of an image whose file states none (the file format's default).
pub const DEFAULT_ALPHA: f64 = 50.0;
/// Pixels per point a PDF page is drawn at: 288 pixels per inch, sharp at four times actual size.
pub const PDF_PIXELS_PER_POINT: f64 = 4.0;
/// Bytes of a file read for its pixel size when the cache has not decoded it.
const HEADER_BYTES: usize = 1 << 20;

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.showImage",
        "Show Image",
        &["View"],
        "",
        "{on?: bool} draw the images placed on glyph layers; a layer side with no paths and no components shows its image regardless (omitted: toggle)",
    ),
    (
        "image.revealFile",
        "Reveal Image File",
        &[],
        "",
        "{glyph?: name, layer?: id} show the file of the layer's image in the system's file manager; without `glyph`, the edited glyph's active layer on the side being edited → {path}",
    ),
];

/// The look of placed images.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageTokens {
    /// The missing-image marker's lines.
    pub marker: Color32,
    /// Side of the missing-image marker, in screen points.
    pub marker_size: f32,
    pub marker_width: f32,
    /// The selected image's outline (its colour is the selection colour).
    pub outline_width: f32,
}

impl ImageTokens {
    pub fn dark() -> Self {
        Self { marker: Color32::from_rgb(0xd0, 0x40, 0x40), marker_size: 24.0, marker_width: 1.5, outline_width: 1.5 }
    }
}

// ── the texture cache ───────────────────────────────────────────────────────────────────────────

/// Which file, at which revision.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    /// The stored path, as the asset store takes it.
    pub path: String,
    pub revision: u64,
}

/// What the cache holds for a key.
#[derive(Clone)]
pub enum CachedImage {
    /// A texture and the image's size in its own space: pixels, or points for a PDF (rounded).
    Ready { texture: TextureHandle, size: [u32; 2] },
    /// A format kept as a reference but not decoded.
    Unsupported(ImageFormat),
    /// The file could not be read or decoded; the message says why.
    Failed(String),
}

struct Entry {
    image: CachedImage,
    /// Decoded bytes the entry holds (0 unless ready).
    bytes: usize,
    /// The cache's use counter when the entry was last asked for.
    last_used: u64,
}

/// Decoded image textures.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<ImageKey, Entry>,
    bytes: usize,
    uses: u64,
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The image for `key`, decoded and uploaded on first use.
    pub fn get(&mut self, ctx: &egui::Context, assets: &dyn AssetStore, key: &ImageKey) -> Option<&CachedImage> {
        self.uses = self.uses.saturating_add(1);
        let uses = self.uses;
        if !self.entries.contains_key(key) {
            let (image, bytes) = load(ctx, assets, &key.path);
            let mut freed = 0;
            self.entries.retain(|k, e| {
                let keep = k.path != key.path;
                if !keep {
                    freed += e.bytes;
                }
                keep
            });
            self.bytes = self.bytes.saturating_sub(freed).saturating_add(bytes);
            self.entries.insert(key.clone(), Entry { image, bytes, last_used: uses });
            self.evict(key);
        }
        let entry = self.entries.get_mut(key)?;
        entry.last_used = uses;
        Some(&entry.image)
    }

    /// What the cache holds for `path` at its newest revision, without reading anything.
    pub fn peek(&self, path: &str) -> Option<&CachedImage> {
        self.entries.iter().filter(|(k, _)| k.path == path).max_by_key(|(k, _)| k.revision).map(|(_, e)| &e.image)
    }

    /// Drops the least recently used entries, never `keep`, until the cache is within its limits.
    fn evict(&mut self, keep: &ImageKey) {
        while self.bytes > MAX_TEXTURE_BYTES || self.entries.len() > MAX_ENTRIES {
            let oldest = self.entries.iter().filter(|(k, _)| *k != keep).min_by_key(|(_, e)| e.last_used).map(|(k, _)| k.clone());
            let Some(k) = oldest else { break };
            if let Some(e) = self.entries.remove(&k) {
                self.bytes = self.bytes.saturating_sub(e.bytes);
            }
        }
    }

    /// Decoded bytes held.
    pub fn len_bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Entries held, for tests.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Reads and decodes the file at `path` and uploads it; the entry and the bytes it holds.
fn load(ctx: &egui::Context, assets: &dyn AssetStore, path: &str) -> (CachedImage, usize) {
    let bytes = match assets.read(path) {
        Ok(b) => b,
        Err(e) => return (CachedImage::Failed(e.to_string()), 0),
    };
    let (image, size) = match imagery::sniff(&bytes) {
        Err(e) => return (CachedImage::Failed(e.to_string()), 0),
        Ok(info) if info.format == ImageFormat::Pdf => match fontcraft_io_pdf::render_pdf(&bytes, PDF_PIXELS_PER_POINT, imagery::MAX_PIXELS) {
            Ok(r) => {
                let ppp = if r.pixels_per_point.is_finite() && r.pixels_per_point > 0.0 { r.pixels_per_point } else { 1.0 };
                let side = |px: u32| ((f64::from(px) / ppp).round().max(1.0)).min(f64::from(u32::MAX)) as u32;
                let size = [side(r.image.width), side(r.image.height)];
                (r.image, size)
            }
            Err(e) => return (CachedImage::Failed(e.to_string()), 0),
        },
        Ok(info) if info.format.decodable() => match imagery::decode_rgba(&bytes, imagery::MAX_PIXELS) {
            Ok(image) => {
                let size = [image.width, image.height];
                (image, size)
            }
            Err(e) => return (CachedImage::Failed(e.to_string()), 0),
        },
        Ok(info) => return (CachedImage::Unsupported(info.format), 0),
    };
    let max_side = u32::try_from(ctx.input(|i| i.max_texture_side)).unwrap_or(u32::MAX);
    let image = shrink(image, max_side);
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(image.pixels.len()) {
        return (CachedImage::Failed("the decoded image has the wrong number of pixels".into()), 0);
    }
    let color = ColorImage::from_rgba_unmultiplied([w, h], &image.pixels);
    let texture = ctx.load_texture(format!("placed-image:{path}"), color, TextureOptions::LINEAR);
    (CachedImage::Ready { texture, size }, image.pixels.len())
}

/// `image` shrunk by a whole factor so that neither side exceeds `max_side`, averaging each
/// block of pixels.
fn shrink(image: Rgba8, max_side: u32) -> Rgba8 {
    let (w, h) = (image.width, image.height);
    let longest = w.max(h);
    if max_side == 0 || longest <= max_side {
        return image;
    }
    let k = longest.div_ceil(max_side) as usize;
    let (w, h) = (w as usize, h as usize);
    let (nw, nh) = (w.div_ceil(k), h.div_ceil(k));
    let mut pixels = vec![0u8; nw * nh * 4];
    for (i, out) in pixels.chunks_exact_mut(4).enumerate() {
        let (ox, oy) = (i % nw, i / nw);
        let mut sum = [0u32; 4];
        let mut n = 0u32;
        for y in oy * k..((oy + 1) * k).min(h) {
            for x in ox * k..((ox + 1) * k).min(w) {
                let at = (y * w + x) * 4;
                if let Some(p) = image.pixels.get(at..at + 4) {
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u32::from(*v);
                    }
                    n += 1;
                }
            }
        }
        if n > 0 {
            for (o, s) in out.iter_mut().zip(sum) {
                *o = u8::try_from(s / n).unwrap_or(u8::MAX);
            }
        }
    }
    Rgba8 { width: u32::try_from(nw).unwrap_or(u32::MAX), height: u32::try_from(nh).unwrap_or(u32::MAX), pixels }
}

// ── the edited glyph's image ────────────────────────────────────────────────────────────────────

/// What a selection or a drag of an image belongs to: the tab, the glyph, the layer id and the
/// side.
type Target = (u64, String, String, LayerSide);

/// A drag on the image, between the press and the release.
#[derive(Clone)]
struct Dragging {
    drag: ImageDrag,
    target: Target,
    /// The press in screen points, for the drag threshold.
    from: Point,
    /// Whether the pointer has moved far enough for the interaction to begin.
    began: bool,
}

/// The images part of the app: the cache, the selected image and the drag in progress.
#[derive(Default)]
pub struct ImageUi {
    pub cache: ImageCache,
    /// The file revision of the cache keys.
    pub revision: u64,
    /// The window had the focus at the last frame.
    focused: bool,
    /// The document the cache holds images of.
    doc: Option<u64>,
    selected: Option<Target>,
    drag: Option<Dragging>,
}

/// The image of `layer` on `side`.
pub fn side_image(layer: &Layer, side: LayerSide) -> Option<&Image> {
    match side {
        LayerSide::Foreground => layer.info.image.as_ref(),
        LayerSide::Background => layer.info.background.as_ref()?.image.as_ref(),
    }
}

/// Whether `side` of `layer` has no paths and no components, so its image shows even while Show
/// Image is off.
pub fn side_empty(layer: &Layer, side: LayerSide) -> bool {
    let shapes = match side {
        LayerSide::Foreground => layer.shapes.as_slice(),
        LayerSide::Background => layer.info.background.as_ref().map_or(&[][..], |b| b.shapes.as_slice()),
    };
    !shapes.iter().any(|s| matches!(s, ModelShape::Path(_) | ModelShape::Component(_)))
}

/// Whether the image on `side` of `layer` is drawn.
pub fn shown(state: &UiState, layer: &Layer, side: LayerSide) -> bool {
    side_image(layer, side).is_some() && (state.display.show_image || side_empty(layer, side))
}

/// The edited layer and the side being edited; `None` in text mode or with no glyph.
fn edited<'f>(font: &'f Font, tab: &Tab) -> Option<(&'f Layer, LayerSide)> {
    if tab.text_mode {
        return None;
    }
    Some((super::view::active_layer(font, tab)?, tab.side()))
}

fn target_of(tab: &Tab, layer: &Layer, side: LayerSide) -> Option<Target> {
    Some((tab.id, tab.edited_glyph()?.to_string(), layer.id.clone(), side))
}

/// The targets image commands take for `t`: `glyph`, `layer` and, on the background, `side`.
fn command_targets(t: &Target) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("glyph".into(), json!(t.1));
    m.insert("layer".into(), json!(t.2));
    if t.3 == LayerSide::Background {
        m.insert("side".into(), json!("background"));
    }
    m
}

/// The size of the image at `path` in its own space: from the cache, else from its header.
fn size_of(images: &ImageUi, session: &Session, path: Option<&str>) -> Option<(f64, f64)> {
    let path = path?;
    if let Some(CachedImage::Ready { size: [w, h], .. }) = images.cache.peek(path) {
        return Some((f64::from(*w), f64::from(*h)));
    }
    let head = session.assets().read_head(path, HEADER_BYTES).ok()?;
    let info = imagery::sniff(&head).ok()?;
    (info.width > 0 && info.height > 0).then(|| (f64::from(info.width), f64::from(info.height)))
}

/// The edited side's image as the tools see it, whether it is shown, and its target.
fn current(app: &FontcraftApp) -> Option<(Target, ImageGeom, bool)> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    let (layer, side) = edited(font, tab)?;
    let image = side_image(layer, side)?;
    let geom = ImageGeom::new(image, size_of(&app.images, &app.session, image.path.as_deref()));
    Some((target_of(tab, layer, side)?, geom, shown(&app.state, layer, side)))
}

/// Whether the edited side's image is selected.
pub fn is_selected(app: &FontcraftApp) -> bool {
    match (&app.images.selected, current(app)) {
        (Some(s), Some((t, geom, shown))) => *s == t && shown && !geom.locked,
        _ => false,
    }
}

/// The targets of the selected image's commands, while one is selected.
pub fn selected_targets(app: &FontcraftApp) -> Option<Map<String, Value>> {
    is_selected(app).then(|| app.images.selected.as_ref().map(command_targets)).flatten()
}

/// Selects the edited side's image and nothing else.
fn select(app: &mut FontcraftApp, target: Target) {
    app.images.selected = Some(target);
    if let Some(tab) = app.state.tab_mut() {
        tab.selection.clear();
    }
}

// ── drawing ─────────────────────────────────────────────────────────────────────────────────────

/// Draws the edited glyph's images behind the outline, the selected image's outline and knobs,
/// and queues Add Image for files dropped on the canvas.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &Painting,
    ctx: &egui::Context,
    images: &mut ImageUi,
    session: &Session,
    state: &UiState,
    tab: &Tab,
    font: &Font,
    pending: &mut Vec<(&'static str, Value)>,
) {
    let doc = session.active().map(|d| d.id);
    if doc != images.doc {
        images.cache.clear();
        images.doc = doc;
    }
    let focused = ctx.input(|i| i.focused);
    if focused && !images.focused {
        images.revision = images.revision.wrapping_add(1);
    }
    images.focused = focused;
    let Some((layer, side)) = edited(font, tab) else {
        images.selected = None;
        return;
    };
    dropped(c, ctx, tab, layer, side, pending);
    let target = target_of(tab, layer, side);
    let edited_image = side_image(layer, side);
    if images.selected.is_some()
        && (images.selected != target || !tab.selection.is_empty() || edited_image.is_none_or(|i| i.locked) || !shown(state, layer, side))
    {
        images.selected = None;
    }
    let v = c.view;
    let to_screen = Affine::translate(v.origin.to_vec2()) * Affine::scale_non_uniform(v.scale(), -v.scale());
    let assets = session.assets();
    let mut sides = vec![LayerSide::Foreground];
    if side == LayerSide::Background {
        sides.push(LayerSide::Background);
    }
    for s in sides {
        if let Some(image) = side_image(layer, s).filter(|_| shown(state, layer, s)) {
            draw_layer_image(c, &mut images.cache, ctx, assets.as_ref(), image, to_screen, images.revision);
        }
    }
    if images.selected.is_some()
        && let Some(image) = edited_image
    {
        let size = match image.path.as_deref().and_then(|p| images.cache.peek(p)) {
            Some(CachedImage::Ready { size: [w, h], .. }) => Some((f64::from(*w), f64::from(*h))),
            _ => None,
        };
        draw_selection(c, &ImageGeom::new(image, size), to_screen, state.display.bounding_box);
    }
}

/// Draws `image` of the edited layer: `to_screen` maps font units to the screen; `revision` is
/// the app's file revision. A missing or undecodable file is drawn as a marker.
pub fn draw_layer_image(
    painting: &Painting,
    cache: &mut ImageCache,
    ctx: &egui::Context,
    assets: &dyn AssetStore,
    image: &Image,
    to_screen: Affine,
    revision: u64,
) {
    let place = to_screen * image.transform.affine();
    let Some(path) = image.path.as_deref() else {
        return marker(painting, place, None);
    };
    let key = ImageKey { path: path.to_string(), revision };
    match cache.get(ctx, assets, &key).cloned() {
        Some(CachedImage::Ready { texture, size: [w, h] }) => {
            let geom = ImageGeom::new(image, Some((f64::from(w), f64::from(h))));
            let Some(r) = geom.visible() else { return marker(painting, place, None) };
            let alpha = image.alpha.filter(|a| a.is_finite()).unwrap_or(DEFAULT_ALPHA).clamp(0.0, 100.0) / 100.0;
            let color = Color32::WHITE.gamma_multiply(alpha as f32);
            let (fw, fh) = (f64::from(w), f64::from(h));
            let mut mesh = Mesh::with_texture(texture.id());
            for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)] {
                let p = place * Point::new(x, y);
                // Texture rows run top to bottom; the image's own y runs up.
                let uv = egui::pos2((x / fw) as f32, (1.0 - y / fh) as f32);
                mesh.vertices.push(Vertex { pos: Pos2::new(p.x as f32, p.y as f32), uv, color });
            }
            if mesh.vertices.iter().all(|v| v.pos.x.is_finite() && v.pos.y.is_finite()) {
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painting.painter.add(Shape::mesh(mesh));
            }
        }
        Some(CachedImage::Unsupported(format)) => marker(painting, place, Some(format)),
        Some(CachedImage::Failed(_)) | None => marker(painting, place, None),
    }
}

/// The missing-image marker at the image's position (`place` maps the image's own space to the
/// screen): a square with a cross, or with the format's name for a format kept as a reference.
fn marker(c: &Painting, place: Affine, format: Option<ImageFormat>) {
    let t = &c.tokens.images;
    let at = place * Point::ZERO;
    if !at.x.is_finite() || !at.y.is_finite() {
        return;
    }
    let s = t.marker_size;
    let min = Pos2::new(at.x as f32, at.y as f32 - s);
    let rect = Rect::from_min_size(min, egui::vec2(s, s));
    let stroke = Stroke::new(t.marker_width, t.marker);
    c.painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
    match format {
        Some(f) => {
            let name = format!("{f:?}").to_uppercase();
            c.painter.text(rect.center(), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(s * 0.35), t.marker);
        }
        None => {
            c.painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
            c.painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
        }
    }
}

/// The selected image's outline, and its knobs while Show Bounding Box is on.
fn draw_selection(c: &Painting, geom: &ImageGeom, to_screen: Affine, knobs: bool) {
    let screen = |p: Point| {
        let q = to_screen * p;
        Pos2::new(q.x as f32, q.y as f32)
    };
    let Some(corners) = geom.corners() else { return };
    let points: Vec<Pos2> = corners.iter().map(|&p| screen(p)).collect();
    if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return;
    }
    let t = c.tokens;
    c.painter.add(Shape::closed_line(points, Stroke::new(t.images.outline_width, t.selection)));
    if knobs {
        for (_, k) in geom.knobs() {
            let rect = Rect::from_center_size(screen(k), egui::vec2(t.knob_size, t.knob_size));
            c.painter.rect_filled(rect, 0.0, t.bounding_box);
        }
    }
}

/// Queues Add Image for image files dropped on the canvas: one file goes on the edited glyph's
/// active layer (on the side being edited); several go on the glyphs their names match.
fn dropped(c: &Painting, ctx: &egui::Context, tab: &Tab, layer: &Layer, side: LayerSide, pending: &mut Vec<(&'static str, Value)>) {
    let (files, at) = ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.hover_pos()));
    if files.is_empty() || at.is_some_and(|p| !c.rect.contains(p)) {
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
    let Some(glyph) = tab.edited_glyph() else { return };
    let mut m = Map::new();
    match paths.as_slice() {
        [] => return,
        [one] => {
            m.insert("files".into(), json!([one]));
            m.insert("glyphs".into(), json!([glyph]));
            m.insert("layer".into(), json!(layer.id));
            if side == LayerSide::Background {
                m.insert("side".into(), json!("background"));
            }
        }
        many => {
            m.insert("files".into(), json!(many));
            m.insert("layer".into(), json!(layer.master));
        }
    }
    pending.push(("glyph.addImage", Value::Object(m)));
}

// ── pointer input ───────────────────────────────────────────────────────────────────────────────

/// A pointer event for the placed image, before the tools see it: a press that selects the
/// image or starts a drag on it, and the drag and release that follow. `None` when the image
/// does not take the event, so it goes on to the active tool.
pub fn pointer(app: &mut FontcraftApp, ev: RawPointer) -> Option<Vec<String>> {
    match ev.kind {
        PointerKind::Down => press(app, ev),
        PointerKind::Drag if app.images.drag.is_some() => Some(drag(app, ev, false)),
        PointerKind::Up if app.images.drag.is_some() => Some(drag(app, ev, true)),
        _ => None,
    }
}

fn font_point(view: &fontcraft_tools::View, ev: &RawPointer) -> Point {
    match ev.space {
        Space::Font => ev.at,
        Space::Screen => view.to_font(ev.at),
    }
}

fn press(app: &mut FontcraftApp, ev: RawPointer) -> Option<Vec<String>> {
    if app.images.drag.is_some() {
        cancel(app);
    }
    let Some((target, geom, shown)) = current(app) else {
        app.images.selected = None;
        return None;
    };
    let view = tools_glue::current_view(app)?;
    let pos = font_point(&view, &ev);
    let selected = is_selected(app);
    let settings = app.session.font().ok().map(|f| tools_glue::settings(&app.tokens, f))?;
    match app.tools.toolbox.active_id() {
        "select" if shown => {
            if selected
                && app.state.display.bounding_box
                && let Some(knob) = geom.knob_at(pos, &view, settings.hit_tolerance)
            {
                let kind = if ev.mods.ctrl && ev.mods.alt { DragKind::Crop(knob) } else { DragKind::Knob(knob) };
                start(app, kind, pos, &view, geom, target);
                return Some(Vec::new());
            }
            // The layer's own objects lie above the image.
            if !matches!(tools_glue::hit_at(app, ev.at, ev.space), Ok((_, None))) {
                app.images.selected = None;
                return None;
            }
            if geom.contains(pos) {
                select(app, target.clone());
                start(app, DragKind::Move, pos, &view, geom, target);
                return Some(Vec::new());
            }
            app.images.selected = None;
            None
        }
        "rotate" | "scale" if selected && app.state.tab().is_some_and(|t| t.selection.is_empty()) => {
            let kind = if app.tools.toolbox.active_id() == "rotate" { DragKind::Rotate } else { DragKind::Scale };
            start(app, kind, pos, &view, geom, target);
            Some(Vec::new())
        }
        _ => None,
    }
}

fn start(app: &mut FontcraftApp, kind: DragKind, pos: Point, view: &fontcraft_tools::View, geom: ImageGeom, target: Target) {
    let drag = ImageDrag::new(kind, pos, geom, command_targets(&target));
    app.images.drag = Some(Dragging { drag, target, from: view.to_screen(pos), began: false });
}

/// A drag event, or the release when `up`: previews the drag's command, and on the release
/// commits it as one undo step.
fn drag(app: &mut FontcraftApp, ev: RawPointer, up: bool) -> Vec<String> {
    let Some(mut d) = app.images.drag.take() else { return Vec::new() };
    let now = current(app).map(|c| c.0);
    let Some(view) = tools_glue::current_view(app).filter(|_| now.as_ref() == Some(&d.target)) else {
        if d.began && app.session.interaction().is_some() {
            let _ = app.session.cancel();
        }
        return Vec::new();
    };
    let pos = font_point(&view, &ev);
    let mut errors = Vec::new();
    if !d.began {
        let threshold = app.session.font().map_or(0.0, |f| tools_glue::settings(&app.tokens, f).drag_threshold);
        if (view.to_screen(pos) - d.from).hypot() < threshold {
            if !up {
                app.images.drag = Some(d);
            }
            return errors;
        }
        if let Err(e) = app.session.begin(d.drag.label()) {
            let e = e.to_string();
            app.state.status = e.clone();
            return vec![e];
        }
        d.began = true;
    }
    if let Some(inv) = d.drag.invocation(pos, ev.mods)
        && let Err(e) = app.session.preview(&inv.id, &inv.params)
    {
        let e = e.to_string();
        app.state.status = e.clone();
        errors.push(e);
    }
    if !up {
        app.images.drag = Some(d);
        return errors;
    }
    match app.session.commit() {
        Ok(true) => app.state.status = format!("{}: done", d.drag.label()),
        Ok(false) => {}
        Err(e) => {
            let e = e.to_string();
            app.state.status = e.clone();
            errors.push(e);
        }
    }
    errors
}

/// Ends a drag in progress and puts the image back as it was at the press.
fn cancel(app: &mut FontcraftApp) {
    if let Some(d) = app.images.drag.take()
        && d.began
        && app.session.interaction().is_some()
    {
        let _ = app.session.cancel();
    }
}

// ── the context menu ────────────────────────────────────────────────────────────────────────────

/// Whether a right-click at `at` (font units) lies on the edited side's image, locked or not, so
/// the menu shows the image section. An unlocked image becomes the selection.
pub fn context_hit(app: &mut FontcraftApp, at: Point) -> bool {
    let Some((target, geom, shown)) = current(app) else { return false };
    if !shown || !geom.covers(at) {
        return false;
    }
    if !geom.locked {
        select(app, target);
    }
    true
}

/// Whether the image section shows item `id`: Unlock Image on a locked image, the crop items on
/// an unlocked one.
pub fn menu_visible(session: &Session, state: &UiState, id: &str) -> bool {
    let image = session.font().ok().zip(state.tab()).and_then(|(font, tab)| {
        let (layer, side) = edited(font, tab)?;
        side_image(layer, side).map(|i| i.locked)
    });
    match (id, image) {
        (_, None) => false,
        ("image.unlock", Some(locked)) => locked,
        (_, Some(locked)) => !locked,
    }
}

// ── commands ────────────────────────────────────────────────────────────────────────────────────

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "view.showImage" => {
            let on = args.toggle(app.state.display.show_image)?;
            app.state.display.show_image = on;
            Ok(json!({ "on": on }))
        }
        "image.revealFile" => reveal(app, args),
        _ => Err(args.err(format!("unknown command `{id}`"))),
    }
}

fn reveal(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tab();
    let given = args.string("glyph")?;
    let glyph = match given {
        Some(g) => g.to_string(),
        None => tab.and_then(|t| t.edited_glyph()).map(str::to_string).ok_or_else(|| args.err("give `glyph`: no glyph is being edited"))?,
    };
    let g = font.glyph(&glyph).ok_or_else(|| args.err(format!("there is no glyph `{glyph}`")))?;
    let edited_layer = tab.filter(|t| t.edited_glyph() == Some(glyph.as_str())).and_then(|t| super::view::active_layer(font, t));
    let layer = match args.string("layer")? {
        Some(id) => g.layers.iter().find(|l| l.id == id).ok_or_else(|| args.err(format!("glyph `{glyph}` has no layer `{id}`")))?,
        None => edited_layer
            .or_else(|| font.masters.first().and_then(|m| g.layer(&m.id)))
            .ok_or_else(|| args.err(format!("glyph `{glyph}` has no layer in the first master")))?,
    };
    let side = match (given, tab) {
        (None, Some(t)) => t.side(),
        _ => LayerSide::Foreground,
    };
    let image = side_image(layer, side).ok_or_else(|| args.err(format!("layer `{}` of glyph `{glyph}` has no image", layer.id)))?;
    let stored = image.path.as_deref().ok_or_else(|| args.err("the image names no file"))?;
    let file = app.session.asset_file(stored).ok_or_else(|| args.err(format!("the image file `{stored}` cannot be found")))?;
    if !file.exists() {
        return Err(args.err(format!("the image file `{}` does not exist", file.display())));
    }
    app.run("file.showInFinder", &json!({ "path": file.to_string_lossy() }))
}
