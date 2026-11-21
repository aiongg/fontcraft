//! Placed images in Edit View: the image drawn behind the outline, the missing-image marker, the selected image with
//! its knobs, and the drags and keys on it.
//!
//! Rows: `view.showImage`, `image.alwaysShownForEmptyGlyphs`, `image.directManipulation`,
//! `image.lockedNotSelectable` (the gestures), `image.storedAsRelativePath` (the missing-image marker).
//!
//! Drawing ([`paint`], [`paint_selection`]; what to draw is worked out in [`refresh`] after each drain and kept in
//! `ToolView::images`):
//! - The edited glyph's image is drawn behind the metric lines and the outline, at its opacity (`alpha`, 50 % when the
//!   file states none), cropped to its crop. While the background is edited, the background's image is drawn over the
//!   foreground's. Images show while View › Show Image is on, and always on a side with no paths and no components.
//!   Text mode draws none.
//! - An iColor or svg layer's image is the layer's colour drawing (`canvas::color`), not drawn here.
//! - A file that is missing or cannot be read is drawn as the missing-image marker: a square with a cross at the
//!   image's position. A format kept as a reference but not decoded (PSD, SVG, PDF) is the same square with the
//!   format's name. A large file being decoded draws nothing until it lands ([`files`]).
//!
//! Selection and gestures (geometry in `fontcraft_tools::image`), seen before the toolbox ([`pointer`], [`key`]):
//! - A press with the Select tool on the image, where no node, segment, anchor, component or guide lies, selects the
//!   image and nothing else (`Tools::image`); a press elsewhere, or selecting anything else, deselects it. A locked
//!   image ignores presses and drags.
//! - Dragging the image moves it; with Show Bounding Box on, the selected image's knobs scale it, and `Ctrl+Alt` on a
//!   knob crops it; with the image selected and nothing else, the Rotate and Scale tools turn and scale it. Every drag
//!   is an interaction whose previews and final step run `image.setTransform` or `image.setCrop`, one undo step.
//! - With the image selected, the arrow keys run `selection.move` with `image: true` by the font's increments (plain,
//!   Shift, Cmd), and Delete or Backspace runs `edit.delete` with `image: true`. Commands run from menus, shortcuts and
//!   the Palette that take the `image` flag get it while the image is selected (`tools::targets::with_targets`).

pub mod files;
pub mod tokens;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;

use craft::ui::primitives::TextureCache;
use craft::ui::runtime::commands::CommandRef;
use egui::epaint::Vertex;
use egui::{Align2, ColorImage, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureOptions, pos2, vec2};
use fontcraft_engine::imagery::ImageFormat;
use fontcraft_geom::{Affine, Point};
use fontcraft_model::{Image, Layer, LayerSide, Shape as ModelShape};
use fontcraft_tools::image::{DragKind, ImageDrag, ImageGeom};
use fontcraft_tools::{Key, KeyEvent, PointerEvent, PointerKind, Selection, hit_test};
use serde_json::{Map, Value, json};

use self::files::{ImageFiles, Loaded, Pixels};
use self::tokens::ImageTokens;
use crate::app::{FontCraftApp, Msg, Preview};
use crate::canvas::layers::PaintCx;
use crate::canvas::tools::tokens::ToolTokens;
use crate::canvas::tools::{self, Target, apply};

/// The opacity, in percent, of an image whose file states none (the file format's default).
pub const DEFAULT_ALPHA: f64 = 50.0;

/// How one placed image is drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Look {
    /// Its pixels, and its size in pixels as stored (the image's own space).
    Pixels { pixels: Pixels, size: [u32; 2] },
    /// Being decoded: nothing is drawn yet.
    Pending,
    /// The missing-image marker.
    Missing,
    /// The marker with the name of a format kept as a reference.
    Kept(ImageFormat),
}

/// One placed image as the canvas or a Font View cell draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedImage {
    /// The image's own space to font units.
    pub place: Affine,
    /// The shown part in the image's own space; `None` when it cannot be worked out (the marker is drawn).
    pub visible: Option<fontcraft_geom::Rect>,
    /// From 0 to 1.
    pub alpha: f32,
    pub look: Look,
}

impl PlacedImage {
    /// `image` as drawn from what the cache knows of its file.
    pub fn new(image: &Image, loaded: Option<&Loaded>) -> PlacedImage {
        let size = loaded.and_then(Loaded::size).map(|[w, h]| (f64::from(w), f64::from(h)));
        let geom = ImageGeom::new(image, size);
        let look = match loaded {
            Some(Loaded::Ready { pixels, size }) => Look::Pixels { pixels: pixels.clone(), size: *size },
            Some(Loaded::Pending { .. }) => Look::Pending,
            Some(Loaded::Unsupported { format, .. }) => Look::Kept(*format),
            Some(Loaded::Failed(_)) | None => Look::Missing,
        };
        let alpha = image.alpha.filter(|a| a.is_finite()).unwrap_or(DEFAULT_ALPHA).clamp(0.0, 100.0) / 100.0;
        PlacedImage { place: geom.affine, visible: geom.visible(), alpha: alpha as f32, look }
    }
}

/// What the image layers paint: the edited glyph's images, bottom first, and the selected image's geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImagePaint {
    pub placed: Vec<PlacedImage>,
    pub selected: Option<ImageGeom>,
}

/// A drag on the image, between the press and the release.
#[derive(Clone, Debug)]
struct Dragging {
    drag: ImageDrag,
    target: Target,
    /// The press on screen, for the drag threshold.
    from: Point,
    /// Whether the pointer moved far enough for the interaction to begin.
    began: bool,
}

/// The images part of the app: the decoded files and the drag in progress. Whether the image is selected is
/// `Tools::image`, so it follows the tools' target.
#[derive(Default)]
pub struct Images {
    pub files: ImageFiles,
    drag: Option<Dragging>,
}

/// The image of `layer` on `side`.
pub fn side_image(layer: &Layer, side: LayerSide) -> Option<&Image> {
    match side {
        LayerSide::Foreground => layer.info.image.as_ref(),
        LayerSide::Background => layer.info.background.as_ref()?.image.as_ref(),
    }
}

/// Whether `side` of `layer` has no paths and no components, so its image shows even while Show Image is off.
pub fn side_empty(layer: &Layer, side: LayerSide) -> bool {
    let shapes = match side {
        LayerSide::Foreground => layer.shapes.as_slice(),
        LayerSide::Background => layer.info.background.as_ref().map_or(&[][..], |b| b.shapes.as_slice()),
    };
    !shapes.iter().any(|s| matches!(s, ModelShape::Path(_) | ModelShape::Component(_)))
}

/// Whether the image on `side` of `layer` is drawn while Show Image is `show_image`.
pub fn shown(show_image: bool, layer: &Layer, side: LayerSide) -> bool {
    side_image(layer, side).is_some() && (show_image || side_empty(layer, side))
}

/// Whether an iColor or svg layer draws its image as its colour drawing.
fn drawn_as_color(layer: &Layer) -> bool {
    use fontcraft_engine::color::layers::{self as kinds, ColorLayerKind};
    matches!(kinds::kind(layer), Some(ColorLayerKind::IColor(_) | ColorLayerKind::Svg))
}

/// The targets image commands take for `t`: `glyph`, `layer` and, on the background, `side`.
pub fn command_targets(t: &Target) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("glyph".into(), json!(t.glyph));
    m.insert("layer".into(), json!(t.layer));
    if t.side == LayerSide::Background {
        m.insert("side".into(), json!("background"));
    }
    m
}

/// The edited side's image as the tools see it, whether it is shown, and its target; `None` in text mode, without an
/// edited glyph or without an image on the edited side.
pub fn current(app: &FontCraftApp) -> Option<(Target, ImageGeom, bool)> {
    if crate::canvas::text::text_mode(&app.view) {
        return None;
    }
    let target = tools::target(&app.session, &app.view)?;
    let font = app.session.font().ok()?;
    let layer = tools::active_layer(font, &app.view)?;
    let image = side_image(layer, target.side)?;
    let size = image.path.as_deref().and_then(|p| app.images.files.peek(p)).and_then(Loaded::size).map(|[w, h]| (f64::from(w), f64::from(h)));
    let geom = ImageGeom::new(image, size);
    let on = shown(app.view.display.image, layer, target.side);
    Some((target, geom, on))
}

/// Whether the edited side's image is selected: shown, unlocked and chosen.
pub fn is_selected(app: &FontCraftApp) -> bool {
    app.tools.image && current(app).is_some_and(|(_, geom, on)| on && !geom.locked)
}

/// The targets of the selected image's commands, while one is selected.
pub fn selected_targets(app: &FontCraftApp) -> Option<Map<String, Value>> {
    if !is_selected(app) {
        return None;
    }
    current(app).map(|(t, _, _)| command_targets(&t))
}

/// Selects the edited side's image and nothing else.
pub fn select(app: &mut FontCraftApp) {
    app.tools.image = true;
    app.tools.selection = Selection::new();
    app.tools.refresh();
}

/// Takes in decoded files, drops a selection that no longer applies and works out what the image layers paint. Runs
/// after each drain.
pub fn refresh(app: &mut FontCraftApp) {
    app.images.files.follow(app.session.active().map(|d| d.id));
    app.images.files.collect();
    let keep = app.tools.image && app.tools.selection.is_empty() && current(app).is_some_and(|(_, geom, on)| on && !geom.locked);
    if app.tools.image && !keep {
        app.tools.image = false;
    }
    let painted = painted(app);
    if app.tools.view.images != painted {
        app.tools.view.images = painted;
    }
}

/// When the window gets the focus back, image files are read again: they may have changed on disk.
pub fn logic(app: &mut FontCraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) {
        app.images.files.reread();
    }
}

fn painted(app: &mut FontCraftApp) -> ImagePaint {
    let Some(target) = (!crate::canvas::text::text_mode(&app.view)).then(|| tools::target(&app.session, &app.view)).flatten() else {
        return ImagePaint::default();
    };
    let assets = app.session.assets();
    let Ok(font) = app.session.font() else { return ImagePaint::default() };
    let Some(layer) = tools::active_layer(font, &app.view) else { return ImagePaint::default() };
    let mut sides = vec![LayerSide::Foreground];
    if target.side == LayerSide::Background {
        sides.push(LayerSide::Background);
    }
    let show_image = app.view.display.image;
    let mut placed = Vec::new();
    for side in sides {
        if side == LayerSide::Foreground && drawn_as_color(layer) {
            continue;
        }
        let Some(image) = side_image(layer, side).filter(|_| shown(show_image, layer, side)) else { continue };
        let loaded = image.path.as_deref().map(|p| app.images.files.get(&assets, p, Some(&*app.host), app.window.as_ref()));
        placed.push(PlacedImage::new(image, loaded.as_ref()));
    }
    let selected = app.tools.image.then(|| side_image(layer, target.side)).flatten().map(|image| {
        let size = image.path.as_deref().and_then(|p| app.images.files.peek(p)).and_then(Loaded::size).map(|[w, h]| (f64::from(w), f64::from(h)));
        ImageGeom::new(image, size)
    });
    ImagePaint { placed, selected }
}

// ── drawing ─────────────────────────────────────────────────────────────────────────────────────────────────────────

/// The paint layer behind the metric lines: the edited glyph's images.
pub fn paint(cx: &PaintCx<'_>) {
    let t = ImageTokens::standard();
    let to_screen = cx.camera.affine();
    for image in &cx.tools.images.placed {
        draw(cx.painter, image, to_screen, &t);
    }
}

fn screen(to_screen: Affine, p: Point) -> Pos2 {
    let q = to_screen * p;
    pos2(q.x as f32, q.y as f32)
}

fn finite(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// Draws `image`: `to_screen` maps font units to screen points (y down).
pub fn draw(painter: &Painter, image: &PlacedImage, to_screen: Affine, t: &ImageTokens) {
    let place = to_screen * image.place;
    match &image.look {
        Look::Pixels { pixels, size: [w, h] } => {
            let Some(r) = image.visible else { return marker(painter, place, None, t) };
            let (fw, fh) = (f64::from(*w), f64::from(*h));
            if fw <= 0.0 || fh <= 0.0 {
                return marker(painter, place, None, t);
            }
            let p = &pixels.0;
            let key = ("fontcraftPlacedImage", std::sync::Arc::as_ptr(p) as usize, p.width, p.height);
            let texture = TextureCache::texture_with(painter.ctx(), key, TextureOptions::LINEAR, || {
                ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.pixels)
            });
            let color = egui::Color32::WHITE.gamma_multiply(image.alpha);
            let mut mesh = Mesh::with_texture(texture);
            for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)] {
                // Texture rows run top to bottom; the image's own y runs up.
                let uv = pos2((x / fw) as f32, (1.0 - y / fh) as f32);
                mesh.vertices.push(Vertex { pos: screen(place, Point::new(x, y)), uv, color });
            }
            if !mesh.vertices.iter().all(|v| finite(v.pos)) {
                return;
            }
            let bounds = Rect::from_points(&mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>());
            if !painter.clip_rect().intersects(bounds) {
                return;
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(Shape::mesh(mesh));
        }
        Look::Pending => {}
        Look::Missing => marker(painter, place, None, t),
        Look::Kept(format) => marker(painter, place, Some(*format), t),
    }
}

/// The missing-image marker at the image's position (`place` maps the image's own space to the screen): a square
/// with a cross, or with the format's name for a format kept as a reference.
fn marker(painter: &Painter, place: Affine, format: Option<ImageFormat>, t: &ImageTokens) {
    let at = screen(place, Point::ZERO);
    if !finite(at) {
        return;
    }
    let s = t.marker_size;
    let rect = Rect::from_min_size(pos2(at.x, at.y - s), vec2(s, s));
    let stroke = Stroke::new(t.marker_width, t.marker);
    painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Inside);
    match format {
        Some(f) => {
            let name = format!("{f:?}").to_uppercase();
            painter.text(rect.center(), Align2::CENTER_CENTER, name, FontId::proportional(s * t.marker_text), t.marker);
        }
        None => {
            painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
            painter.line_segment([rect.right_top(), rect.left_bottom()], stroke);
        }
    }
}

/// The paint layer under the selection: the selected image's outline, and its knobs while Show Bounding Box is on.
pub fn paint_selection(cx: &PaintCx<'_>) {
    let Some(geom) = cx.tools.images.selected.as_ref() else { return };
    let Some(corners) = geom.corners() else { return };
    let to_screen = cx.camera.affine();
    let points: Vec<Pos2> = corners.iter().map(|&p| screen(to_screen, p)).collect();
    if !points.iter().all(|p| finite(*p)) {
        return;
    }
    let t = ImageTokens::standard();
    let colour = ToolTokens::standard().selection;
    cx.painter.add(Shape::closed_line(points, Stroke::new(t.outline_width, colour)));
    if cx.display.bounding_box {
        for (_, k) in geom.knobs() {
            let at = screen(to_screen, k);
            if finite(at) {
                cx.painter.rect_filled(Rect::from_center_size(at, vec2(t.knob_size, t.knob_size)), 0.0, colour);
            }
        }
    }
}

// ── pointer input ───────────────────────────────────────────────────────────────────────────────────────────────────

/// A pointer event for the placed image, before the toolbox sees it: a press that selects the image or starts a drag
/// on it, and the drags and the release that follow. `None` when the image does not take the event; else the errors.
pub fn pointer(app: &mut FontCraftApp, ev: PointerEvent) -> Option<Vec<String>> {
    match ev.kind {
        PointerKind::Down => press(app, ev),
        PointerKind::Drag if app.images.drag.is_some() => Some(drag(app, ev, false)),
        PointerKind::Up if app.images.drag.is_some() => Some(drag(app, ev, true)),
        _ => None,
    }
}

fn press(app: &mut FontCraftApp, ev: PointerEvent) -> Option<Vec<String>> {
    if let Some(d) = app.images.drag.take()
        && d.began
    {
        let _ = app.apply_msg(Msg::Preview(Preview::Cancel));
    }
    apply::sync(app);
    let Some((target, geom, on)) = current(app) else {
        app.tools.image = false;
        return None;
    };
    let view = apply::tools_view(&apply::camera(app));
    let tolerance = f64::from(ToolTokens::standard().hit_tolerance);
    let selected = is_selected(app);
    let tool = app.tools.toolbox.active_id().to_string();
    match tool.as_str() {
        "select" if on && !geom.locked => {
            if selected
                && app.view.display.bounding_box
                && let Some(knob) = geom.knob_at(ev.pos, &view, tolerance)
            {
                let kind = if ev.mods.ctrl && ev.mods.alt { DragKind::Crop(knob) } else { DragKind::Knob(knob) };
                start(app, kind, ev.pos, geom, target);
                return Some(Vec::new());
            }
            // The layer's own objects lie above the image.
            let object = {
                let font = app.session.font().ok()?;
                let scene = tools::scene(font, &app.view, app.tools.focus.as_ref())?;
                hit_test(&scene, &view, ev.pos, tolerance).is_some()
            };
            if object || !geom.contains(ev.pos) {
                app.tools.image = false;
                return None;
            }
            select(app);
            start(app, DragKind::Move, ev.pos, geom, target);
            Some(Vec::new())
        }
        "rotate" | "scale" if selected && app.tools.selection.is_empty() => {
            let kind = if tool == "rotate" { DragKind::Rotate } else { DragKind::Scale };
            start(app, kind, ev.pos, geom, target);
            Some(Vec::new())
        }
        _ => None,
    }
}

fn start(app: &mut FontCraftApp, kind: DragKind, pos: Point, geom: ImageGeom, target: Target) {
    let view = apply::tools_view(&apply::camera(app));
    let drag = ImageDrag::new(kind, pos, geom, command_targets(&target));
    app.images.drag = Some(Dragging { drag, target, from: view.to_screen(pos), began: false });
}

/// A drag event, or the release when `up`: previews the drag's command, and on the release commits it as one undo
/// step.
fn drag(app: &mut FontCraftApp, ev: PointerEvent, up: bool) -> Vec<String> {
    let Some(mut d) = app.images.drag.take() else { return Vec::new() };
    if tools::target(&app.session, &app.view).as_ref() != Some(&d.target) {
        if d.began {
            let _ = app.apply_msg(Msg::Preview(Preview::Cancel));
        }
        return Vec::new();
    }
    let view = apply::tools_view(&apply::camera(app));
    if !d.began {
        if (view.to_screen(ev.pos) - d.from).hypot() < f64::from(ToolTokens::standard().drag_threshold) {
            if !up {
                app.images.drag = Some(d);
            }
            return Vec::new();
        }
        if let Err(e) = app.apply_msg(Msg::Preview(Preview::Begin(d.drag.label().into()))) {
            return vec![e];
        }
        d.began = true;
    }
    let mut errors = Vec::new();
    if let Some(inv) = d.drag.invocation(ev.pos, ev.mods)
        && let Err(e) = app.apply_msg(Msg::Preview(Preview::Update(CommandRef::with_params(inv.id, inv.params))))
    {
        errors.push(e);
    }
    if !up {
        app.images.drag = Some(d);
        return errors;
    }
    if let Err(e) = app.apply_msg(Msg::Preview(Preview::Commit)) {
        errors.push(e);
    }
    errors
}

// ── keys ────────────────────────────────────────────────────────────────────────────────────────────────────────────

/// A key for the selected image, before the toolbox sees it: an arrow moves the image by the font's increment
/// (Shift: the big one, Cmd: the huge one) and Delete or Backspace removes it, through the commands nodes use. `None`
/// when no image is selected or the key is not one of these.
pub fn key(app: &mut FontCraftApp, ev: &KeyEvent) -> Option<Vec<String>> {
    if !ev.pressed || app.images.drag.is_some() || app.tools.toolbox.busy() {
        return None;
    }
    let mut params = selected_targets(app)?;
    let m = ev.mods;
    let id = match ev.key {
        Key::Left | Key::Right | Key::Up | Key::Down => {
            let font = app.session.font().ok()?;
            let [plain, big, huge] = fontcraft_tools::ToolSettings::default().with_font(font).increments;
            let step = if m.cmd {
                huge
            } else if m.shift {
                big
            } else {
                plain
            };
            if !step.is_finite() {
                return None;
            }
            let (dx, dy) = match ev.key {
                Key::Left => (-step, 0.0),
                Key::Right => (step, 0.0),
                Key::Up => (0.0, step),
                _ => (0.0, -step),
            };
            params.insert("dx".into(), json!(dx));
            params.insert("dy".into(), json!(dy));
            "selection.move"
        }
        Key::Delete | Key::Backspace if !(m.ctrl || m.alt || m.shift || m.cmd) => "edit.delete",
        _ => return None,
    };
    params.insert("image".into(), json!(true));
    let msg = FontCraftApp::route(CommandRef::with_params(id, Value::Object(params)));
    Some(app.apply_msg(msg).err().into_iter().collect())
}
