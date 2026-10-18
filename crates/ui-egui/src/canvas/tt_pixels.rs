//! The Instructor's instance and grid-fitted outlines and the pixel backdrop.
//!
//! Rows: `hinting.tt.tool.instanceOutline`, `hinting.tt.tool.gridFittedOutline`,
//! `hinting.tt.tool.pixelBackdrop`, `hint.tt.showPixelPreview`, `hint.tt.setPixelSize`,
//! `hint.tt.setRenderingIntent`.
//!
//! While the TrueType Instructor is active, Edit View draws behind the edited outline:
//!
//! | Layer | Drawn from | When |
//! |---|---|---|
//! | pixel backdrop, grey coverage | the grid-fitted outline rasterised at the pixel size | always |
//! | blue outline | the previewed instance, unhinted | the previewed instance is not at the edited master, and Show Pixel Preview is on |
//! | red outline | the previewed instance after its instructions ran, grid-fitted | the glyph has a glyph program, and Show Pixel Preview is on |
//!
//! The preview font is the previewed instance compiled as TrueType in memory through the
//! export's compiler, holding only the edited glyph, the glyphs its components and corner
//! components use, and `.notdef`, without layout tables. Its `fpgm`, `prep`, `cvt ` and glyph
//! program are the ones a full export writes, because they depend on the masters' TrueType zones
//! and stems and on the glyph alone. `fontcraft_render::hinted` runs them at the pixel size in
//! the rendering intent.
//!
//! The preview font is rebuilt when the document revision, the glyph, the edited master or the
//! previewed instance change; its outlines and pixels per pixel size and intent are kept beside
//! it, and the backdrop texture in egui's memory. A frame that changes nothing only transforms
//! and draws them.
//!
//! What other packages read: [`InstructorState::intent`] ([`RenderingIntent`], set by
//! `hint.tt.setRenderingIntent`; [`RenderingIntent::hinted`] gives the renderer's intent and
//! [`RenderingIntent::as_str`] the `intent` parameter of `hint.tt.renderPreview`),
//! [`InstructorState::ppm`] (set by `hint.tt.setPixelSize`) and [`preview_of`].

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{Color32, ColorImage, Pos2, Rect, Shape, Stroke, TextureHandle, TextureOptions, pos2};
use fontcraft_engine::Session;
use fontcraft_engine::compile::{self, DataRef, Flavour, Options};
use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_model::{Font, GlyphTable, MAX_COMPONENT_DEPTH, Shape as ModelShape, Target};
use fontcraft_render::hinted::{HintedRenderer, Intent};
use fontcraft_tools::tt_instructor::ID as INSTRUCTOR;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::tt_badges::{InstructorState, MAX_PPM, MIN_PPM, clamp_ppm};
use super::{Painting, Scene};
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};

/// The `instance` value of `hint.tt.showPixelPreview` that previews the edited master.
pub const MASTER: &str = "master";
/// Pixel sizes and intents whose preview is kept for one preview font.
const MAX_CACHED_PREVIEWS: usize = 16;
/// Most glyphs the preview font takes from the source: the edited glyph and what it uses.
const MAX_PREVIEW_GLYPHS: usize = 1_000;

/// The rendering a preview imitates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderingIntent {
    /// Grayscale anti-aliasing.
    #[default]
    Grayscale,
    /// GDI ClearType: horizontal subpixels, no vertical oversampling.
    ClearType,
    /// DirectWrite ClearType: horizontal subpixels, smoothed vertically.
    DirectWrite,
}

impl RenderingIntent {
    pub const ALL: [RenderingIntent; 3] = [RenderingIntent::Grayscale, RenderingIntent::ClearType, RenderingIntent::DirectWrite];

    /// The name commands use: `grayscale`, `cleartype` or `directwrite`.
    pub fn as_str(self) -> &'static str {
        match self {
            RenderingIntent::Grayscale => "grayscale",
            RenderingIntent::ClearType => "cleartype",
            RenderingIntent::DirectWrite => "directwrite",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.as_str() == name)
    }

    /// The label a pop-up shows.
    pub fn label(self) -> &'static str {
        match self {
            RenderingIntent::Grayscale => "Grayscale",
            RenderingIntent::ClearType => "ClearType",
            RenderingIntent::DirectWrite => "DirectWrite",
        }
    }

    /// The hinted renderer's intent.
    pub fn hinted(self) -> Intent {
        match self {
            RenderingIntent::Grayscale => Intent::Grayscale,
            RenderingIntent::ClearType => Intent::GdiClearType,
            RenderingIntent::DirectWrite => Intent::DirectWrite,
        }
    }
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "hint.tt.showPixelPreview",
        "Show Pixel Preview",
        &[],
        "",
        "{on?: bool (omitted: toggle, or on when `instance` is given), instance?: name (an instance by name, `#n` for the instance at position n counted from 0, or \"master\" for the edited master)} show or hide the TrueType Instructor's blue outline (the previewed instance before hinting, not drawn when that instance sits at the edited master) and red outline (the previewed instance after its instructions ran, grid-fitted at the pixel size in the rendering intent), and choose the previewed instance. The pixel backdrop stays → {on, instance}",
    ),
    (
        "hint.tt.setPixelSize",
        "Set Preview Pixel Size",
        &[],
        "",
        "{ppm: whole number 4 to 400} the pixels per em of the TrueType Instructor's grid-fitted outline and pixel backdrop; one pixel is units per em / ppm font units → {ppm}",
    ),
    (
        "hint.tt.setRenderingIntent",
        "Set Rendering Intent",
        &[],
        "",
        "{intent: \"grayscale\" | \"cleartype\" | \"directwrite\"} the rendering the TrueType Instructor's grid-fitted outline, its pixel backdrop and the hinting preview imitate: grayscale anti-aliasing, GDI ClearType (horizontal subpixels, no vertical smoothing) or DirectWrite (horizontal subpixels, smoothed vertically). A font's programs see it through GETINFO → {intent}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "hint.tt.showPixelPreview" => {
            let given = args.string("instance")?;
            let instance = match given {
                None => app.state.tt_instructor.instance.clone(),
                Some(MASTER) => None,
                Some(name) => {
                    let font = app.session.font().map_err(|e| e.to_string())?;
                    instance_index(font, name).ok_or_else(|| args.err(format!("the font has no instance `{name}`")))?;
                    Some(name.to_string())
                }
            };
            let s = &mut app.state.tt_instructor;
            let on = match (args.flag("on")?, given) {
                (Some(on), _) => on,
                (None, Some(_)) => true,
                (None, None) => !s.pixel_preview,
            };
            s.pixel_preview = on;
            s.instance = instance;
            Ok(json!({"on": on, "instance": s.instance.as_deref().unwrap_or(MASTER)}))
        }
        "hint.tt.setPixelSize" => {
            let ppm = args.number("ppm")?.ok_or_else(|| args.err("`ppm` is required"))?;
            if ppm.fract() != 0.0 || ppm < f64::from(MIN_PPM) || ppm > f64::from(MAX_PPM) {
                return Err(args.err(format!("`ppm`: {ppm} is not a whole pixel size from {MIN_PPM} to {MAX_PPM}")));
            }
            let s = &mut app.state.tt_instructor;
            s.ppm = clamp_ppm(ppm as i64);
            Ok(json!({"ppm": s.ppm}))
        }
        "hint.tt.setRenderingIntent" => {
            let name = args.string("intent")?.ok_or_else(|| args.err("`intent` is required"))?;
            let intent = RenderingIntent::parse(name)
                .ok_or_else(|| args.err(format!("unknown intent `{name}`; the intents are grayscale, cleartype and directwrite")))?;
            app.state.tt_instructor.intent = intent;
            Ok(json!({"intent": intent.as_str()}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// The position in the font's instance list of the instance `name` names: by name, or `#n`.
pub fn instance_index(font: &Font, name: &str) -> Option<usize> {
    match name.strip_prefix('#') {
        Some(n) => n.parse::<usize>().ok().filter(|&i| i < font.instances.len()),
        None => font.instances.iter().position(|i| i.name == name),
    }
}

/// What the preview shows: the font target, and whether it sits at the edited master (then the
/// blue outline would lie under the black one and is left out).
#[derive(Clone, Debug, PartialEq)]
pub struct Previewed {
    pub target: Target,
    pub at_master: bool,
}

/// The previewed instance `instance` (as [`InstructorState::instance`] holds it) for an edit of
/// master `master`.
pub fn previewed(font: &Font, master: &str, instance: Option<&str>) -> Result<Previewed, String> {
    let Some(name) = instance else { return Ok(Previewed { target: Target::Master(master.to_string()), at_master: true }) };
    let index = instance_index(font, name).ok_or_else(|| format!("the font has no instance `{name}`"))?;
    let at_master = font.instances.get(index).and_then(|i| font.master_at(&i.location)).is_some_and(|m| m.id == master);
    Ok(Previewed { target: Target::Instance(index), at_master })
}

/// Pixel coverage at one size, placed in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    /// Coverage per pixel, rows from the top.
    pub alpha: Vec<u8>,
    /// The left edge, in pixels from the glyph origin.
    pub left: i32,
    /// The top edge, in pixels above the baseline.
    pub top: i32,
    /// Font units per pixel: units per em / pixel size.
    pub unit: f64,
}

impl Pixels {
    /// The area the pixels cover, in font units: (left, bottom) and (right, top).
    pub fn bounds(&self) -> (Point, Point) {
        let (left, top) = (f64::from(self.left), f64::from(self.top));
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        (Point::new(left * self.unit, (top - h) * self.unit), Point::new((left + w) * self.unit, top * self.unit))
    }
}

/// The Instructor's preview of the edited glyph at one pixel size and intent, in font units with
/// y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PixelPreview {
    /// The previewed instance before hinting; `None` when it sits at the edited master.
    pub instance: Option<BezPath>,
    /// The grid-fitted outline; `None` for a glyph without a glyph program.
    pub fitted: Option<BezPath>,
    pub pixels: Pixels,
    /// Why the glyph was drawn unhinted, when it was.
    pub message: Option<String>,
}

/// The preview font: the bytes and the edited glyph's id in it.
#[derive(Clone, Debug)]
pub struct PreviewFont {
    pub bytes: Arc<[u8]>,
    pub gid: u32,
    pub units_per_em: f64,
}

/// The names of `glyph` and of every glyph its layers use as components or corner components,
/// and `.notdef` when the font has one.
fn glyph_closure(font: &Font, glyph: &str) -> Result<BTreeSet<String>, String> {
    let mut wanted = BTreeSet::new();
    let mut frontier = vec![(glyph.to_string(), 0usize)];
    if font.glyph(".notdef").is_some() {
        frontier.push((".notdef".to_string(), 0));
    }
    while let Some((name, depth)) = frontier.pop() {
        if depth > MAX_COMPONENT_DEPTH || wanted.contains(&name) {
            continue;
        }
        let Some(g) = font.glyph(&name) else { continue };
        if wanted.len() >= MAX_PREVIEW_GLYPHS {
            return Err(format!("`{glyph}` uses more than {MAX_PREVIEW_GLYPHS} glyphs"));
        }
        for layer in &g.layers {
            for shape in &layer.shapes {
                if let ModelShape::Component(c) = shape {
                    frontier.push((c.glyph.clone(), depth + 1));
                }
            }
            for hint in &layer.info.hints {
                if let Ok(Some(corner)) = fontcraft_model::corner::corner_hint(hint) {
                    frontier.push((corner.name, depth + 1));
                }
            }
        }
        wanted.insert(name);
    }
    Ok(wanted)
}

/// `font` at `target` compiled as TrueType with only `glyph` and the glyphs it uses.
pub fn preview_font(font: &Font, target: &Target, glyph: &str, options: &Options) -> Result<PreviewFont, String> {
    if font.glyph(glyph).is_none() {
        return Err(format!("the font has no glyph `{glyph}`"));
    }
    let wanted = glyph_closure(font, glyph)?;
    let mut subset = font.clone();
    subset.glyphs = GlyphTable::from_glyphs(font.glyphs.iter().filter(|g| wanted.contains(&g.name)).cloned().collect());
    let options = Options { skip_features: true, ..options.clone() };
    let compiled = compile::compile(&subset, target, Flavour::TrueType, &options).map_err(|e| e.to_string())?;
    let gid = compiled
        .glyph_order
        .iter()
        .position(|g| g == glyph)
        .and_then(|i| u32::try_from(i).ok())
        .ok_or_else(|| format!("`{glyph}` is not in the exported font"))?;
    Ok(PreviewFont { bytes: compiled.bytes.into(), gid, units_per_em: f64::from(font.units_per_em.max(1)) })
}

/// A preview font with its renderers: one unhinted, one per intent asked for.
struct Built {
    font: PreviewFont,
    plain: HintedRenderer,
    hinted: Vec<(RenderingIntent, HintedRenderer)>,
}

impl Built {
    fn new(font: PreviewFont) -> Result<Self, String> {
        let plain = HintedRenderer::new(font.bytes.clone(), Intent::Grayscale).map_err(|e| e.to_string())?.without_hinting();
        Ok(Self { font, plain, hinted: Vec::new() })
    }

    /// The renderer of `intent`; it keeps the font's hinting state per pixel size.
    fn renderer(&mut self, intent: RenderingIntent) -> Result<HintedRenderer, String> {
        if let Some((_, r)) = self.hinted.iter().find(|(i, _)| *i == intent) {
            return Ok(r.clone());
        }
        let r = HintedRenderer::new(self.font.bytes.clone(), intent.hinted()).map_err(|e| e.to_string())?;
        self.hinted.push((intent, r.clone()));
        Ok(r)
    }

    fn render(&mut self, ppm: u16, intent: RenderingIntent, at_master: bool) -> Result<PixelPreview, String> {
        let unit = self.font.units_per_em / f64::from(ppm.max(1));
        let to_units = Affine::scale(unit);
        let gid = self.font.gid;
        let hinted = self.renderer(intent)?;
        let fitted = hinted.outline(gid, ppm).map_err(|e| e.to_string())?;
        let bitmap = hinted.pixels(gid, ppm).map_err(|e| e.to_string())?;
        let instance = if at_master { None } else { Some(to_units * self.plain.outline(gid, ppm).map_err(|e| e.to_string())?.path) };
        // Black ink: the coverage is the alpha.
        let alpha = bitmap.image.pixels.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        let pixels = Pixels { width: bitmap.image.width, height: bitmap.image.height, alpha, left: bitmap.left, top: bitmap.top, unit };
        let fitted_path = hinted.has_instructions(gid).then(|| to_units * fitted.path);
        Ok(PixelPreview { instance, fitted: fitted_path, pixels, message: fitted.message })
    }
}

/// What a preview font is built from.
#[derive(Clone, Debug, PartialEq)]
struct FontKey {
    doc: (u64, u64),
    glyph: String,
    master: String,
    target: Target,
}

type SizeKey = (u16, RenderingIntent);

struct Cached {
    key: FontKey,
    built: Result<Built, String>,
    previews: Vec<(SizeKey, Result<Arc<PixelPreview>, String>)>,
}

thread_local! {
    /// The last preview font and its previews: compiling runs the export's outline preparation
    /// and the hinting programs, too slow for every frame.
    static CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
}

/// The Instructor's preview of `glyph` in an edit of master `master`, for the previewed
/// instance, pixel size and intent of `state`. `doc` is the document's id and revision.
pub fn preview_of(
    session: &Session,
    font: &Font,
    doc: (u64, u64),
    master: &str,
    glyph: &str,
    state: &InstructorState,
) -> Result<Arc<PixelPreview>, String> {
    let previewed = previewed(font, master, state.instance.as_deref())?;
    let ppm = clamp_ppm(i64::from(state.ppm));
    let size: SizeKey = (ppm, state.intent);
    let key = FontKey { doc, glyph: glyph.to_string(), master: master.to_string(), target: previewed.target.clone() };
    CACHE.with_borrow_mut(|cache| {
        if !cache.as_ref().is_some_and(|c| c.key == key) {
            let options = Options { glyph_data: DataRef(session.glyph_data().ok()), ..Options::default() };
            let built = preview_font(font, &previewed.target, glyph, &options).and_then(Built::new);
            *cache = Some(Cached { key, built, previews: Vec::new() });
        }
        let Some(cached) = cache.as_mut() else { return Err("the preview font was not kept".to_string()) };
        if let Some((_, found)) = cached.previews.iter().find(|(k, _)| *k == size) {
            return found.clone();
        }
        let made = match &mut cached.built {
            Ok(built) => built.render(ppm, state.intent, previewed.at_master).map(Arc::new),
            Err(e) => Err(e.clone()),
        };
        if cached.previews.len() >= MAX_CACHED_PREVIEWS {
            drop(cached.previews.remove(0));
        }
        cached.previews.push((size, made.clone()));
        made
    })
}

/// The backdrop texture and the preview it shows, kept in egui's memory.
#[derive(Clone, Default)]
struct Backdrop {
    preview: Option<Arc<PixelPreview>>,
    colour: Option<Color32>,
    texture: Option<TextureHandle>,
}

fn backdrop_id() -> egui::Id {
    egui::Id::new("fontcraft.tt_pixels.backdrop")
}

/// The texture the backdrop drew last.
#[cfg(test)]
pub(crate) fn texture_id(ctx: &egui::Context) -> Option<egui::TextureId> {
    ctx.data(|d| d.get_temp::<Backdrop>(backdrop_id())).and_then(|b| b.texture.map(|t| t.id()))
}

/// The backdrop's texture for `preview`, made again only when the preview or the colour changed.
fn texture(ctx: &egui::Context, preview: &Arc<PixelPreview>, colour: Color32) -> Option<TextureHandle> {
    let mut cache = ctx.data(|d| d.get_temp::<Backdrop>(backdrop_id())).unwrap_or_default();
    let fresh = cache.preview.as_ref().is_some_and(|p| Arc::ptr_eq(p, preview)) && cache.colour == Some(colour);
    if !fresh {
        let p = &preview.pixels;
        let [r, g, b, a] = colour.to_srgba_unmultiplied();
        let mut rgba = Vec::with_capacity(p.alpha.len().saturating_mul(4));
        for &coverage in &p.alpha {
            let alpha = (u16::from(a) * u16::from(coverage) / 255) as u8;
            rgba.extend_from_slice(&[r, g, b, alpha]);
        }
        let (w, h) = (p.width as usize, p.height as usize);
        cache.texture = (!rgba.is_empty() && rgba.len() == w.saturating_mul(h).saturating_mul(4))
            .then(|| ctx.load_texture("tt-pixel-backdrop", ColorImage::from_rgba_unmultiplied([w, h], &rgba), TextureOptions::NEAREST));
        cache.preview = Some(preview.clone());
        cache.colour = Some(colour);
        ctx.data_mut(|d| d.insert_temp(backdrop_id(), cache.clone()));
    }
    cache.texture
}

/// Draws the backdrop and the blue and red outlines of the edited glyph, behind its outline,
/// while the Instructor is active.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene) {
    if scene.tool != INSTRUCTOR || scene.tab.edit_background {
        return;
    }
    let Some(glyph) = scene.tab.edited_glyph() else { return };
    let state = &scene.state.tt_instructor;
    let Ok(preview) = preview_of(scene.session, scene.font, scene.doc, &scene.layer.master, glyph, state) else { return };
    let t = &c.tokens.tt;
    let screen = |p: Point| {
        let s = c.view.to_screen(p);
        Pos2::new(s.x as f32, s.y as f32)
    };
    let (low, high) = preview.pixels.bounds();
    let area = Rect::from_two_pos(screen(low), screen(high));
    if area.is_finite()
        && area.intersects(c.rect)
        && let Some(texture) = texture(ui.ctx(), &preview, t.pixel)
    {
        c.painter.image(texture.id(), area, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    if !state.pixel_preview {
        return;
    }
    let to_screen = Affine::translate(c.view.origin.to_vec2()) * Affine::scale_non_uniform(c.view.scale(), -c.view.scale());
    for (path, colour) in [(&preview.instance, t.instance_outline), (&preview.fitted, t.fitted_outline)] {
        let Some(path) = path else { continue };
        for contour in super::flatten(&(to_screen * path.clone())) {
            c.painter.add(Shape::line(contour, Stroke::new(t.preview_outline_width, colour)));
        }
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_engine::model::{Instance, Path};

    use super::*;

    /// One frame of `app` in a 1280 × 800 window; the shapes it drew, nested lists flattened.
    fn frame(ctx: &egui::Context, app: &mut FontcraftApp) -> Vec<Shape> {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        let mut flat = Vec::new();
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                other => flat.push(other),
            }
        }
        flat
    }

    fn strokes(shapes: &[Shape], colour: Color32) -> usize {
        shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(colour))).count()
    }

    /// Glyph `A`, a bar 100 to 200 by 0 to 700 in master `m01` and 100 to 300 in master `m02`
    /// (at 900), with its four nodes snapped vertically; instances `Black` at `m02` and
    /// `Regular` at `m01`. Edited in `m01` with the Instructor at actual size.
    fn app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.session
            .edit("setup", |font| {
                let regular = font.masters[0].location.clone();
                let mut m = font.masters[0].clone();
                m.id = "m02".into();
                m.name = "Black".into();
                m.location = vec![900.0];
                Arc::make_mut(&mut font.masters).push(m);
                let instances = Arc::make_mut(&mut font.instances);
                instances.push(Instance { name: "Black".into(), location: vec![900.0], ..Instance::default() });
                instances.push(Instance { name: "Regular".into(), location: regular, ..Instance::default() });
                let mut g = font.glyph_mut("A")?;
                let layer = g.layer_mut("m01").unwrap();
                layer.shapes = vec![ModelShape::Path(Path::rect(100.0, 0.0, 200.0, 700.0))];
                let mut black = layer.clone();
                black.id = "m02".into();
                black.master = "m02".into();
                black.shapes = vec![ModelShape::Path(Path::rect(100.0, 0.0, 300.0, 700.0))];
                g.layers.push(black);
                Ok(())
            })
            .unwrap();
        app.run("hint.tt.snapPoint", &json!({"glyph": "A", "nodes": [[0, 0], [0, 1], [0, 2], [0, 3]], "direction": "vertical"})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [300, 350]})).unwrap();
        app.run("tool.select", &json!({"tool": INSTRUCTOR})).unwrap();
        app
    }

    fn preview(app: &FontcraftApp) -> Arc<PixelPreview> {
        let font = app.session.font().unwrap();
        let doc = app.session.active().map(|d| (d.id, d.revision)).unwrap();
        preview_of(&app.session, font, doc, "m01", "A", &app.state.tt_instructor).unwrap()
    }

    /// The y values of `path`'s points.
    fn ys(path: &BezPath) -> Vec<f64> {
        path.elements().iter().filter_map(|el| el.end_point()).map(|p| p.y).collect()
    }

    /// The median of `n` timed runs of `f`.
    fn median(n: usize, mut f: impl FnMut()) -> std::time::Duration {
        let mut times: Vec<_> = (0..n)
            .map(|_| {
                let start = std::time::Instant::now();
                f();
                start.elapsed()
            })
            .collect();
        times.sort();
        times[n / 2]
    }

    /// `cargo test -p fontcraft-ui-egui instructor_timings -- --ignored --nocapture`.
    #[test]
    #[ignore = "prints timings"]
    fn instructor_timings() {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&ctx, &mut app);
        let cached = median(21, || drop(frame(&ctx, &mut app)));
        let mut fitted: Vec<_> = (0..21)
            .map(|i| {
                let rounding = if i % 2 == 0 { "up" } else { "down" };
                app.run("hint.tt.setRounding", &json!({"glyph": "A", "instructions": [0], "rounding": rounding})).unwrap();
                let start = std::time::Instant::now();
                assert!(preview(&app).fitted.is_some());
                start.elapsed()
            })
            .collect();
        fitted.sort();
        let fitted = fitted[10];
        println!("Instructor cached frame {cached:?}; grid-fitted outline after an instruction edit {fitted:?}");
    }

    #[test]
    fn frame_shows_backdrop_and_blue_and_red_outlines_at_two_pixel_sizes() {
        let mut app = app();
        let ctx = egui::Context::default();
        let t = app.tokens.tt.clone();
        assert_eq!(app.run("hint.tt.showPixelPreview", &json!({"instance": "Black"})).unwrap(), json!({"on": true, "instance": "Black"}));
        let mut areas = Vec::new();
        for ppm in [12, 24] {
            app.run("hint.tt.setPixelSize", &json!({"ppm": ppm})).unwrap();
            frame(&ctx, &mut app);
            let shapes = frame(&ctx, &mut app);
            assert!(strokes(&shapes, t.instance_outline) >= 1, "{ppm}: the blue outline of the Black instance");
            assert!(strokes(&shapes, t.fitted_outline) >= 1, "{ppm}: the red grid-fitted outline");
            assert!(strokes(&shapes, app.tokens.outline) >= 1, "{ppm}: the black master outline");
            let texture = texture_id(&ctx).expect("a backdrop texture");
            let mesh = shapes.iter().find(|s| matches!(s, Shape::Mesh(m) if m.texture_id == texture)).expect("the backdrop is drawn");
            areas.push(mesh.visual_bounding_rect());

            let p = preview(&app);
            let unit = 1000.0 / f64::from(ppm);
            assert_eq!(p.pixels.unit, unit);
            assert_eq!(p.pixels.alpha.len(), (p.pixels.width * p.pixels.height) as usize);
            assert!(p.pixels.alpha.iter().any(|&a| a > 0), "{ppm}: the bar covers pixels");
            // Snapped nodes land on whole pixels.
            let fitted = p.fitted.as_ref().expect("A has a glyph program");
            assert!(ys(fitted).iter().all(|y| ((y / unit) - (y / unit).round()).abs() < 1e-6), "{ppm}: {:?}", ys(fitted));
            // The blue outline is the Black instance: wider than the master's bar.
            let blue = p.instance.as_ref().unwrap();
            let right = blue.elements().iter().filter_map(|el| el.end_point()).map(|p| p.x).fold(f64::MIN, f64::max);
            assert!((right - 300.0).abs() < 1.0, "{ppm}: {right}");
        }
        assert_ne!(areas[0], areas[1], "the backdrop follows the pixel size");

        // The Regular instance sits at the edited master: no blue outline.
        app.run("hint.tt.showPixelPreview", &json!({"instance": "Regular"})).unwrap();
        frame(&ctx, &mut app);
        let shapes = frame(&ctx, &mut app);
        assert_eq!(strokes(&shapes, t.instance_outline), 0);
        assert!(strokes(&shapes, t.fitted_outline) >= 1);
        assert!(preview(&app).instance.is_none());

        // Show Pixel Preview off hides both outlines; the backdrop stays.
        assert_eq!(app.run("hint.tt.showPixelPreview", &json!({})).unwrap(), json!({"on": false, "instance": "Regular"}));
        frame(&ctx, &mut app);
        let shapes = frame(&ctx, &mut app);
        assert_eq!(strokes(&shapes, t.fitted_outline) + strokes(&shapes, t.instance_outline), 0);
        let texture = texture_id(&ctx).unwrap();
        assert!(shapes.iter().any(|s| matches!(s, Shape::Mesh(m) if m.texture_id == texture)));

        // Another tool draws none of it.
        app.run("tool.select", &json!({"tool": "select"})).unwrap();
        app.run("hint.tt.showPixelPreview", &json!({"on": true})).unwrap();
        let shapes = frame(&ctx, &mut app);
        assert_eq!(strokes(&shapes, t.fitted_outline), 0);
        assert!(!shapes.iter().any(|s| matches!(s, Shape::Mesh(m) if m.texture_id == texture)));
    }

    #[test]
    fn a_glyph_without_instructions_has_no_red_outline() {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "B", "x": 100, "y": 0, "width": 100, "height": 700})).unwrap();
        let font = app.session.font().unwrap();
        let doc = app.session.active().map(|d| (d.id, d.revision)).unwrap();
        let master = font.masters[0].id.clone();
        let p = preview_of(&app.session, font, doc, &master, "B", &app.state.tt_instructor).unwrap();
        assert!(p.fitted.is_none() && p.instance.is_none());
        assert!(p.pixels.alpha.iter().any(|&a| a > 0), "the backdrop still shows the glyph");
    }

    #[test]
    fn intent_pixel_size_and_instance_commands_check_their_values() {
        let mut app = app();
        for intent in ["cleartype", "directwrite", "grayscale"] {
            assert_eq!(app.run("hint.tt.setRenderingIntent", &json!({"intent": intent})).unwrap(), json!({"intent": intent}));
            assert_eq!(app.state.tt_instructor.intent.as_str(), intent);
            assert!(preview(&app).fitted.is_some(), "{intent}");
        }
        assert_eq!(RenderingIntent::ClearType.hinted(), Intent::GdiClearType);
        let before = app.state.tt_instructor.clone();
        for bad in [
            ("hint.tt.setRenderingIntent", json!({"intent": "subpixel"})),
            ("hint.tt.setRenderingIntent", json!({})),
            ("hint.tt.setPixelSize", json!({"ppm": 3})),
            ("hint.tt.setPixelSize", json!({"ppm": 401})),
            ("hint.tt.setPixelSize", json!({"ppm": 12.5})),
            ("hint.tt.setPixelSize", json!({"ppm": "12"})),
            ("hint.tt.setPixelSize", json!({})),
            ("hint.tt.showPixelPreview", json!({"instance": "Hairline"})),
            ("hint.tt.showPixelPreview", json!({"instance": "#9"})),
            ("hint.tt.showPixelPreview", json!({"on": "yes"})),
        ] {
            assert!(app.run(bad.0, &bad.1).is_err(), "{} {}", bad.0, bad.1);
            assert_eq!(app.state.tt_instructor, before, "a refused {} changes nothing", bad.0);
        }
        for ppm in [MIN_PPM, MAX_PPM] {
            assert_eq!(app.run("hint.tt.setPixelSize", &json!({"ppm": ppm})).unwrap(), json!({"ppm": ppm}));
        }
        assert_eq!(app.run("hint.tt.showPixelPreview", &json!({"instance": "#0", "on": false})).unwrap(), json!({"on": false, "instance": "#0"}));
        assert_eq!(app.run("hint.tt.showPixelPreview", &json!({"instance": MASTER})).unwrap(), json!({"on": true, "instance": MASTER}));
        assert_eq!(app.state.tt_instructor.instance, None);
    }
}
