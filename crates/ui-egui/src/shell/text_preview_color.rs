//! Colour glyphs in Text Preview (M11.17): the shown master or instance compiled with its colour
//! tables and its export switches, and each colour glyph drawn from that font
//! (`fontcraft_render::color`) in place of its outline.
//!
//! Rows: `color.preview.textPreview`.
//!
//! - Only a font with colour layers is compiled. The font is built on a thread of its own, at
//!   once when the window first needs it, then each time the document changes and editing has
//!   paused for [`REBUILD_PAUSE`] seconds; until then the last font of the same document and
//!   style is drawn. The style pop-up's instance is compiled as that instance, so its switches
//!   (`Export COLR Table`, `Color Palette for CPAL`…) apply; a master is compiled as the master.
//! - Palette 0 of the compiled font is drawn, which is the palette `Color Palette for CPAL`
//!   chooses. The text colour (palette index 65,535) is the preview's glyph colour.
//! - A glyph the compiled font has no colour for, one past the drawing limits, and one not yet
//!   drawn in a frame whose budget is spent are drawn as their outline, as before.
//! - When the colour font cannot be built, every glyph is drawn as its outline and the reason
//!   shows under the text, as it does when the preview's own font cannot be built.
//! - The glyphs drawn in colour are laid onto one texture of the area in view, made again only
//!   when what is drawn changes.
//! - What a glyph's colour drawing leaves out or draws plainer (an SVG filter drawn unfiltered,
//!   an image left out) and why a colour glyph is drawn as its outline show under the text, the
//!   first of them with the glyph's name and a count of the others ([`ColorPreview::warning`]),
//!   unless a build error or a shaping note shows there.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, pos2};
use fontcraft_engine::Session;
use fontcraft_engine::color::export::has_color_layers;
use fontcraft_engine::compile::{AssetsRef, Flavour, Options};
use fontcraft_model::{Font, Target};
use fontcraft_render::Image;
use fontcraft_render::color::{ColorOptions, ColorRenderer, FrameBudget, GlyphRaster, Painted, composite};

use super::text_preview::{REBUILD_PAUSE, build_options};

/// Seconds between looks at a build under way.
const BUILD_POLL: f64 = 0.03;

/// Most colour warnings kept in one frame, each glyph and reason once.
const MAX_WARNINGS: usize = 64;

/// What a colour font is built for: the document's id and revision, and the style compiled.
type Key = (u64, u64, String);

/// A compiled colour font.
struct Built {
    bytes: Arc<[u8]>,
    glyph_order: Vec<String>,
}

type BuildResult = Result<Arc<Built>, String>;
type Landing = Arc<Mutex<Option<BuildResult>>>;

/// What Text Preview keeps for its colour glyphs.
#[derive(Default)]
pub struct ColorPreview {
    /// Whether the document at this id and revision has colour layers.
    has_color: Option<((u64, u64), bool)>,
    built: Option<(Key, BuildResult)>,
    wanted: Option<(Key, f64)>,
    running: Option<(Key, Landing)>,
    renderer: Option<(Key, ColorRenderer)>,
    /// The font this frame wants, when the document has colour layers.
    current: Option<Key>,
    /// This frame: how glyphs are drawn (`None`: not in colour), the budget, the glyphs drawn in
    /// colour and where, and a hash of what was drawn.
    options: Option<ColorOptions>,
    budget: FrameBudget,
    frame: Vec<(Arc<GlyphRaster>, Pos2)>,
    names: Vec<Arc<str>>,
    /// This frame's warnings, one per glyph and reason, at most [`MAX_WARNINGS`].
    warnings: Vec<(Arc<str>, String)>,
    hasher: DefaultHasher,
    texture_key: Option<u64>,
    texture: Option<TextureHandle>,
    /// The last page laid out, the area it covers and its pixels per point.
    page: Option<(Image, Rect, f32)>,
}

/// The style the preview compiles: the chosen instance when the style pop-up names one (and no
/// master of that name), else the shown master.
fn target_of(font: &Font, style: Option<&str>, master: &str) -> (String, Target) {
    if let Some(name) = style
        && !font.masters.iter().any(|m| m.name == name)
        && let Some(i) = font
            .instances
            .iter()
            .position(|i| i.name == name && i.kind.as_deref() != Some("variable") && font.masters.iter().any(|m| m.location == i.location))
    {
        return (format!("instance {i}"), Target::Instance(i));
    }
    (format!("master {master}"), Target::Master(master.to_string()))
}

/// The options a colour build compiles with: the preview's, the document's images and svg
/// files, no layout tables and no overlap removal, which a colour drawing does not need.
fn colour_options(session: &Session) -> Options {
    let mut options = build_options(session);
    options.assets = AssetsRef(Some(session.assets()));
    options.skip_features = true;
    options.remove_overlap = false;
    options
}

fn build(font: &Font, target: &Target, options: &Options) -> BuildResult {
    let compiled = fontcraft_engine::compile::compile(font, target, Flavour::TrueType, options)
        .map_err(|e| format!("The colour glyphs are drawn as outlines: {e}"))?;
    Ok(Arc::new(Built { bytes: Arc::from(compiled.bytes), glyph_order: compiled.glyph_order }))
}

/// Builds on a thread of its own (here, on the web or when no thread can be started); the
/// result lands in the returned slot.
fn start(font: Arc<Font>, target: Target, options: Options) -> Landing {
    let landing: Landing = Arc::default();
    let out = landing.clone();
    let work = move || {
        let result = fontcraft_engine::guard::catch_panic(|| build(&font, &target, &options))
            .unwrap_or_else(|e| Err(format!("The colour glyphs are drawn as outlines: the font could not be built ({e})")));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let work = Arc::new(Mutex::new(Some(work)));
        let in_thread = work.clone();
        let spawned = std::thread::Builder::new().name("text preview colour font".into()).spawn(move || {
            if let Some(w) = in_thread.lock().ok().and_then(|mut w| w.take()) {
                w();
            }
        });
        if spawned.is_err()
            && let Some(w) = work.lock().ok().and_then(|mut w| w.take())
        {
            w();
        }
    }
    #[cfg(target_arch = "wasm32")]
    work();
    landing
}

impl ColorPreview {
    /// Starts a frame: builds or rebuilds the colour font when needed, and decides whether this
    /// frame draws in colour. `scale` is pixels per font unit; `text` the glyph colour.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        session: &Session,
        font: &Font,
        style: Option<&str>,
        master: &str,
        doc: (u64, u64),
        now: f64,
        ctx: &egui::Context,
        scale: f64,
        text: Color32,
    ) {
        self.frame.clear();
        self.names.clear();
        self.warnings.clear();
        self.hasher = DefaultHasher::new();
        self.budget = FrameBudget::frame();
        self.options = None;
        let colour = match self.has_color {
            Some((d, c)) if d == doc => c,
            _ => {
                let c = has_color_layers(font);
                self.has_color = Some((doc, c));
                c
            }
        };
        if !colour {
            self.renderer = None;
            self.built = None;
            self.current = None;
            return;
        }
        let (label, target) = target_of(font, style, master);
        let key: Key = (doc.0, doc.1, label);
        self.current = Some(key.clone());
        self.step(&key, target, now, session, ctx);
        if let Some((k, _)) = &self.renderer
            && k.0 == key.0
            && k.2 == key.2
        {
            k.hash(&mut self.hasher);
            scale.to_bits().hash(&mut self.hasher);
            text.to_array().hash(&mut self.hasher);
            let [r, g, b, a] = text.to_srgba_unmultiplied();
            self.options = Some(ColorOptions { scale, palette: 0, text_color: [r, g, b, a] });
        }
    }

    /// Takes the result of a finished build.
    fn collect(&mut self) {
        let Some((key, landing)) = &self.running else { return };
        let done = landing.lock().ok().and_then(|mut l| l.take());
        if let Some(result) = done {
            self.built = Some((key.clone(), result));
            self.running = None;
        }
    }

    fn step(&mut self, key: &Key, target: Target, now: f64, session: &Session, ctx: &egui::Context) {
        self.collect();
        let fresh = self.built.as_ref().is_some_and(|(k, _)| k == key);
        let under_way = self.running.as_ref().is_some_and(|(k, _)| k == key);
        if !fresh && !under_way {
            if self.wanted.as_ref().is_none_or(|(k, _)| k != key) {
                self.wanted = Some((key.clone(), now));
            }
            let since = self.wanted.as_ref().map_or(0.0, |(_, at)| now - at);
            let editing = self.renderer.as_ref().is_some_and(|(k, _)| k.0 == key.0 && k.2 == key.2);
            let wait = if editing { REBUILD_PAUSE - since } else { 0.0 };
            if self.running.is_none() && wait <= 0.0 {
                if let Some(doc) = session.active() {
                    self.running = Some((key.clone(), start(doc.font.clone(), target, colour_options(session))));
                    self.collect();
                }
            } else if wait > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.clamp(BUILD_POLL, REBUILD_PAUSE)));
            }
        }
        if self.running.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(BUILD_POLL));
        }
        // A finished build becomes the renderer; one that failed for this style draws outlines.
        let made = match &self.built {
            Some((k, Ok(built))) if self.renderer.as_ref().is_none_or(|(r, _)| r != k) => Some((k.clone(), built.clone())),
            _ => None,
        };
        if let Some((k, built)) = made {
            match ColorRenderer::new(built.bytes.clone(), &built.glyph_order) {
                Ok(r) => self.renderer = Some((k, r)),
                Err(e) => self.built = Some((k, Err(format!("The colour glyphs are drawn as outlines: {e}")))),
            }
        }
        if matches!(&self.built, Some((k, Err(_))) if k.0 == key.0 && k.2 == key.2) {
            self.renderer = None;
        }
    }

    /// Glyph `name` with its origin at screen point `at`: true when it is drawn in colour, and
    /// the caller leaves its outline out.
    pub fn take(&mut self, name: &str, at: Pos2) -> bool {
        let (Some(options), Some((_, renderer))) = (self.options.as_ref(), self.renderer.as_mut()) else { return false };
        let Some(gid) = renderer.glyph_id(name) else { return false };
        match renderer.glyph(gid, options, &mut self.budget) {
            Painted::Color(raster) => {
                for w in &raster.warnings {
                    self.note(name, w);
                }
                (gid, at.x.to_bits(), at.y.to_bits()).hash(&mut self.hasher);
                self.frame.push((raster, at));
                self.names.push(Arc::from(name));
                true
            }
            Painted::Deferred => {
                (gid, u32::MAX).hash(&mut self.hasher);
                false
            }
            Painted::Refused(why) => {
                self.note(name, &format!("drawn as its outline: {why}"));
                false
            }
            Painted::Plain => false,
        }
    }

    fn note(&mut self, glyph: &str, warning: &str) {
        if self.warnings.len() < MAX_WARNINGS && !self.warnings.iter().any(|(g, w)| &**g == glyph && w == warning) {
            self.warnings.push((Arc::from(glyph), warning.to_string()));
        }
    }

    /// The first warning of this frame's colour glyphs, with the glyph's name and how many more
    /// there are (each glyph and reason once); Text Preview shows it under the text.
    pub fn warning(&self) -> Option<String> {
        let (glyph, first) = self.warnings.first()?;
        Some(match self.warnings.len() - 1 {
            0 => format!("{glyph}: {first}"),
            1 => format!("{glyph}: {first} (and 1 more colour warning)"),
            more => format!("{glyph}: {first} (and {more} more colour warnings)"),
        })
    }

    /// A hash of what this frame draws in colour, for the outline texture's key: a glyph that
    /// turns to colour leaves the outlines.
    pub fn key(&self) -> u64 {
        self.hasher.finish()
    }

    /// Draws this frame's colour glyphs over `clip` (the area in view) at `ppp` pixels per point.
    pub fn paint(&mut self, ctx: &egui::Context, painter: &egui::Painter, clip: Rect, ppp: f32) {
        if self.budget.deferred() > 0 {
            ctx.request_repaint();
        }
        if self.frame.is_empty() {
            self.texture_key = None;
            self.texture = None;
            self.page = None;
            return;
        }
        let (w, h) = ((clip.width() * ppp).round(), (clip.height() * ppp).round());
        if !(w >= 1.0 && h >= 1.0) {
            return;
        }
        let mut hasher = self.hasher.clone();
        for v in [clip.min.x, clip.min.y, w, h, ppp] {
            v.to_bits().hash(&mut hasher);
        }
        let key = hasher.finish();
        if self.texture_key != Some(key) {
            self.texture_key = Some(key);
            self.texture = None;
            self.page = None;
            let (w, h) = (w as u32, h as u32);
            let Ok(mut page) = Image::from_rgba_premultiplied(w, h, vec![0; w as usize * h as usize * 4]) else { return };
            for (raster, at) in &self.frame {
                let x = (f64::from(at.x - clip.min.x) * f64::from(ppp)).round() as i64;
                let y = (f64::from(at.y - clip.min.y) * f64::from(ppp)).round() as i64;
                composite(&mut page, raster, x, y);
            }
            let colour = ColorImage::from_rgba_premultiplied([w as usize, h as usize], &page.pixels);
            self.texture = Some(ctx.load_texture("text-preview-color", colour, TextureOptions::LINEAR));
            self.page = Some((page, clip, ppp));
        }
        if let Some(texture) = &self.texture {
            painter.image(texture.id(), clip, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }

    /// The glyphs the last frame drew in colour, in text order.
    pub fn drawn(&self) -> &[Arc<str>] {
        &self.names
    }

    /// The last page of colour glyphs, the area it covers and its pixels per point.
    pub fn page(&self) -> Option<&(Image, Rect, f32)> {
        self.page.as_ref()
    }

    /// A colour font is being built.
    pub fn building(&self) -> bool {
        self.running.is_some()
    }

    /// Whether a colour font is ready to draw with.
    pub fn ready(&self) -> bool {
        self.renderer.is_some()
    }

    /// Why the colour font of the shown document and style could not be built; Text Preview
    /// shows it under the text, which is drawn in outlines.
    pub fn error(&self) -> Option<&str> {
        match (&self.built, &self.current) {
            (Some((k, Err(e))), Some(c)) if k.0 == c.0 && k.2 == c.2 => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use fontcraft_engine::color::layers;
    use fontcraft_engine::imagery::{Rgba8, to_png};
    use fontcraft_geom::Affine;
    use fontcraft_render::color::ColorKind;
    use serde_json::{Value, json};

    use super::*;
    use crate::FontcraftApp;
    use crate::canvas::color::{self as canvas_color, Mode, Sources, SvgCache};
    use crate::control;

    struct Window {
        ctx: egui::Context,
        time: Cell<f64>,
    }

    impl Window {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|s| s.animation_time = 0.0);
            Self { ctx, time: Cell::new(0.0) }
        }

        fn frame(&self, app: &mut FontcraftApp, dt: f64) {
            self.time.set(self.time.get() + dt);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                time: Some(self.time.get()),
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
            output.textures_delta.clear();
        }

        /// Frames past the rebuild pause until no colour font is being built and one is ready.
        fn settle(&self, app: &mut FontcraftApp) {
            for n in 0..1_000 {
                self.frame(app, REBUILD_PAUSE + 0.1);
                let c = &app.state.panels.text_preview.cache.color;
                if n >= 3 && !c.building() && (c.ready() || c.error().is_some()) {
                    // One more frame draws with the font that just landed.
                    self.frame(app, 0.01);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("the colour font was not built");
        }
    }

    fn exec(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
        assert_eq!(r["ok"], true, "{command} {r}");
        r["result"].clone()
    }

    fn master(app: &FontcraftApp) -> String {
        app.session.font().unwrap().masters[0].id.clone()
    }

    fn colour_layer(app: &FontcraftApp, glyph: &str) -> String {
        app.session.font().unwrap().glyph(glyph).unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone()
    }

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-text-preview-color-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Glyph `name`, a square (100,100)–(300,300), with a copy of its master layer converted by
    /// `convert`.
    fn add_colour_glyph(app: &mut FontcraftApp, name: &str, convert: &str) -> String {
        if app.session.font().unwrap().glyph(name).is_none() {
            exec(app, "glyph.newGlyph", json!({"name": name}));
        }
        exec(app, "path.addRectangle", json!({"glyph": name, "x": 100, "y": 100, "width": 200, "height": 200}));
        let m = master(app);
        exec(app, "layers.addBackupLayer", json!({"glyphs": [name], "master": m}));
        let layer = colour_layer(app, name);
        exec(app, convert, json!({"glyph": name, "layer": layer}));
        layer
    }

    /// A red palette glyph A and a full-colour glyph B with a blue gradient and a black stroke.
    fn colour_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        exec(&mut app, "colorPalette.addColor", json!({"color": "#FF0000"}));
        add_colour_glyph(&mut app, "A", "layer.convertToColorPalette");
        let b = add_colour_glyph(&mut app, "B", "layer.convertToColor");
        exec(
            &mut app,
            "attributes.addGradient",
            json!({"glyph": "B", "layer": b, "stops": [{"offset": 0, "color": "#0000FFFF"}, {"offset": 1, "color": "#00FF00FF"}]}),
        );
        exec(&mut app, "attributes.setStrokeWidth", json!({"glyph": "B", "layer": b, "width": 20}));
        if app.session.font().unwrap().glyph("C").is_none() {
            exec(&mut app, "glyph.newGlyph", json!({"name": "C", "unicodes": ["0043"]}));
        }
        exec(&mut app, "path.addRectangle", json!({"glyph": "C", "x": 100, "y": 100, "width": 200, "height": 200}));
        app
    }

    fn compiled(app: &FontcraftApp) -> ColorRenderer {
        let font = app.session.font().unwrap();
        let options = Options { assets: AssetsRef(Some(app.session.assets())), skip_features: true, remove_overlap: false, ..Options::default() };
        let built = build(font, &Target::Master(master(app)), &options).unwrap();
        ColorRenderer::new(built.bytes.clone(), &built.glyph_order).unwrap()
    }

    /// Glyph `name` from the compiled font, 400 × 400 pixels, one per unit, origin at the bottom
    /// left.
    fn rendered(r: &mut ColorRenderer, name: &str, scale: f64) -> (Image, ColorKind) {
        let gid = r.glyph_id(name).unwrap();
        let options = ColorOptions { scale, palette: 0, text_color: [0, 0, 0, 255] };
        let Painted::Color(raster) = r.glyph(gid, &options, &mut FrameBudget::frame()) else { panic!("{name} is not drawn in colour") };
        let mut page = Image::from_rgba_premultiplied(400, 400, vec![0; 400 * 400 * 4]).unwrap();
        composite(&mut page, &raster, 0, 400);
        (page, raster.kind)
    }

    /// The Edit View drawing of `name`'s colour layer, on the same pixel grid.
    fn edit_view(app: &FontcraftApp, name: &str) -> Image {
        let font = app.session.font().unwrap();
        let id = colour_layer(app, name);
        let layer = font.glyph(name).unwrap().layers.iter().find(|l| l.id == id).unwrap();
        let mode = Mode::Layers { master: master(app), kind: layers::kind(layer).unwrap() };
        let assets = app.session.assets();
        let mut svgs = SvgCache::default();
        let mut src = Sources { assets: assets.as_ref(), svgs: &mut svgs, revision: 0, text: Color32::BLACK };
        let to_pixels = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 400.0]);
        let image = canvas_color::glyph_image(font, name, &mode, Some(layer), &mut src, 400, 400, to_pixels).unwrap();
        let pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
        Image::from_rgba_premultiplied(400, 400, pixels).unwrap()
    }

    /// Pixels whose channels differ by more than 24, and pixels with ink in `a`.
    fn differences(a: &Image, b: &Image) -> (usize, usize) {
        let pa = a.pixels.as_chunks::<4>().0.iter();
        let pb = b.pixels.as_chunks::<4>().0.iter();
        let differ = pa.clone().zip(pb).filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 24)).count();
        (differ, pa.filter(|p| p[3] > 0).count())
    }

    fn at(image: &Image, x: u32, y_up: u32) -> [u8; 4] {
        image.pixel(x, 399 - y_up).unwrap()
    }

    #[test]
    fn color_preview_text_preview_compiled_glyphs_equal_the_edit_view_drawing() {
        let app = colour_app();
        let mut r = compiled(&app);
        for name in ["A", "B"] {
            let (mine, kind) = rendered(&mut r, name, 1.0);
            assert_eq!(kind, ColorKind::Colr);
            let theirs = edit_view(&app, name);
            let (differ, ink) = differences(&mine, &theirs);
            assert!(ink > 30_000, "{name} draws its square: {ink} pixels");
            assert!(differ * 100 <= 400 * 400, "{name}: {differ} of 160,000 pixels differ from the Edit View drawing");
        }
        let (a, _) = rendered(&mut r, "A", 1.0);
        assert_eq!(at(&a, 200, 200), [255, 0, 0, 255], "palette row 0");
        assert_eq!(at(&a, 50, 50)[3], 0, "nothing outside the outline");
        let (b, _) = rendered(&mut r, "B", 1.0);
        let (left, right) = (at(&b, 130, 200), at(&b, 270, 200));
        assert!(left[2] > 150 && right[1] > 150, "the gradient runs from blue to green: {left:?} {right:?}");
        assert_eq!(at(&b, 100, 200), [0, 0, 0, 255], "the black stroke on the edge, above the fill");
        // The glyph without colour layers is no colour glyph.
        let c = r.glyph_id("C").unwrap();
        assert_eq!(r.kind(c), None);
    }

    #[test]
    fn color_preview_text_preview_draws_sbix_images_and_svg_documents() {
        let dir = folder("kinds");
        let png = dir.join("E.png");
        std::fs::write(&png, to_png(&Rgba8 { width: 64, height: 32, pixels: [255, 0, 0, 255].repeat(64 * 32) }).unwrap()).unwrap();
        let svg = dir.join("F.svg");
        std::fs::write(
            &svg,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100" fill="#00FF00"/></svg>"##,
        )
        .unwrap();
        let mut app = FontcraftApp::default();
        exec(&mut app, "glyph.newGlyph", json!({"name": "E.icolor"}));
        let m = master(&app);
        exec(&mut app, "layers.addBackupLayer", json!({"glyphs": ["E.icolor"], "master": m}));
        let e = colour_layer(&app, "E.icolor");
        exec(&mut app, "layer.convertToIColor", json!({"glyph": "E.icolor", "layer": e, "size": 125}));
        exec(&mut app, "layer.setIColorImage", json!({"glyph": "E.icolor", "layer": e, "path": png.to_string_lossy()}));
        let f = add_colour_glyph(&mut app, "F", "layer.convertToSvg");
        exec(&mut app, "layer.setSvgImage", json!({"glyph": "F", "layer": f, "path": svg.to_string_lossy()}));
        let mut r = compiled(&app);

        // 125 pixels per em on a 1000-unit em: one image pixel per sprite pixel.
        let (image, kind) = rendered(&mut r, "E.icolor", 0.125);
        assert_eq!(kind, ColorKind::Sbix);
        assert_eq!(at(&image, 0, 0), [255, 0, 0, 255], "the image's bottom left at the origin");
        assert_eq!(at(&image, 63, 31), [255, 0, 0, 255]);
        assert_eq!(at(&image, 64, 0)[3], 0);
        assert_eq!(at(&image, 0, 32)[3], 0);
        assert_eq!(image.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count(), 64 * 32);

        let (mine, kind) = rendered(&mut r, "F", 1.0);
        assert_eq!(kind, ColorKind::Svg);
        assert_eq!(at(&mine, 50, 50), [0, 255, 0, 255], "the file's bottom left at the layer origin, a unit per unit");
        assert_eq!(at(&mine, 150, 150)[3], 0);
        let (differ, ink) = differences(&mine, &edit_view(&app, "F"));
        assert!(ink > 9_000 && differ * 100 <= 400 * 400, "{differ} pixels differ, {ink} drawn");
    }

    #[test]
    fn color_preview_text_preview_shows_colour_and_the_fallback_outlines_without_colr() {
        let mut app = colour_app();
        exec(&mut app, "window.textPreview", json!({}));
        exec(&mut app, "textPreview.setText", json!({"text": "ABC"}));
        let w = Window::new();
        w.settle(&mut app);
        let tp = &app.state.panels.text_preview;
        let drawn: Vec<&str> = tp.cache.color.drawn().iter().map(|n| &**n).collect();
        assert_eq!(drawn, ["A", "B"], "C has no colour layers: {:?}", tp.cache.color.error());
        let names: Vec<&str> = tp.cache.drawn.glyphs.iter().map(|(n, _)| &**n).collect();
        assert_eq!(names, ["A", "B", "C"]);
        // A's square centre on the colour page is palette red.
        let (page, clip, ppp) = tp.cache.color.page().unwrap();
        let origin = tp.cache.drawn.glyphs[0].1;
        let scale = (tp.size / f64::from(app.session.font().unwrap().units_per_em)) as f32;
        let centre = origin + egui::vec2(200.0 * scale, -200.0 * scale);
        let (x, y) = (((centre.x - clip.min.x) * ppp) as u32, ((centre.y - clip.min.y) * ppp) as u32);
        assert_eq!(page.pixel(x, y), Some([255, 0, 0, 255]));

        // Export COLR Table off: the compiled font has no COLR, so every glyph is an outline.
        exec(&mut app, "font.customParameters.add", json!({"scope": "font", "name": "Export COLR Table", "value": false}));
        w.settle(&mut app);
        let tp = &app.state.panels.text_preview;
        assert!(tp.cache.color.drawn().is_empty(), "{:?}", tp.cache.color.drawn());
        assert!(tp.cache.color.page().is_none());
        assert_eq!(tp.cache.drawn.glyphs.len(), 3, "the glyphs are still drawn, as outlines");
    }

    #[test]
    fn color_preview_text_preview_shows_what_a_colour_glyph_draws_plainer() {
        // An svg file whose filter has a kernel past the limit: drawn in colour, unfiltered, with
        // the reason under the text once although the glyph shows twice.
        let dir = folder("warnings");
        let svg = dir.join("F.svg");
        let kernel = vec!["1"; 17 * 17].join(" ");
        std::fs::write(
            &svg,
            format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><filter id="f"><feConvolveMatrix order="17" kernelMatrix="{kernel}"/></filter><rect width="100" height="100" fill="#00FF00" filter="url(#f)"/></svg>"##
            ),
        )
        .unwrap();
        let mut app = FontcraftApp::default();
        let f = add_colour_glyph(&mut app, "F", "layer.convertToSvg");
        exec(&mut app, "layer.setSvgImage", json!({"glyph": "F", "layer": f, "path": svg.to_string_lossy()}));
        exec(&mut app, "window.textPreview", json!({}));
        exec(&mut app, "textPreview.setText", json!({"text": "FF"}));
        let w = Window::new();
        w.settle(&mut app);
        let tp = &app.state.panels.text_preview;
        assert_eq!(tp.cache.color.drawn().len(), 2, "{:?}", tp.cache.color.error());
        let message = tp.cache.drawn.message.as_deref().unwrap_or_default();
        assert!(message.starts_with("F: ") && message.contains("kernel") && !message.contains("more"), "{message}");
    }

    #[test]
    fn color_preview_text_preview_shows_why_the_colour_font_was_not_built() {
        let mut app = colour_app();
        exec(&mut app, "window.textPreview", json!({}));
        exec(&mut app, "textPreview.setText", json!({"text": "AB"}));
        let w = Window::new();
        w.settle(&mut app);
        assert_eq!(app.state.panels.text_preview.cache.drawn.message, None);
        // The build for the shown document and style failed.
        let c = &mut app.state.panels.text_preview.cache.color;
        let key = c.current.clone().unwrap();
        let reason = "The colour glyphs are drawn as outlines: the font could not be built";
        c.built = Some((key, Err(reason.to_string())));
        w.frame(&mut app, 0.01);
        let tp = &app.state.panels.text_preview;
        assert!(tp.cache.color.drawn().is_empty(), "every glyph is an outline");
        assert_eq!(tp.cache.drawn.glyphs.len(), 2);
        assert_eq!(tp.cache.drawn.message.as_deref(), Some(reason), "the reason shows under the text");
    }

    #[test]
    fn color_preview_text_preview_spreads_first_drawings_over_frames() {
        let app = colour_app();
        let mut r = compiled(&app);
        let options = ColorOptions { scale: 1.0, palette: 0, text_color: [0, 0, 0, 255] };
        let (a, b) = (r.glyph_id("A").unwrap(), r.glyph_id("B").unwrap());
        let mut budget = FrameBudget::new(1);
        assert!(matches!(r.glyph(a, &options, &mut budget), Painted::Color(_)), "the first glyph of a frame always starts");
        assert_eq!(r.glyph(b, &options, &mut budget), Painted::Deferred);
        assert!(matches!(r.glyph(a, &options, &mut budget), Painted::Color(_)), "a drawn glyph comes from the cache");
        assert_eq!(budget.deferred(), 1);
        let mut next = FrameBudget::new(1);
        assert!(matches!(r.glyph(b, &options, &mut next), Painted::Color(_)));
        assert_eq!(r.cached().0, 2);
        // A size past the sprite limit is refused, and the glyph drawn as an outline.
        let huge = ColorOptions { scale: 30.0, ..options };
        assert!(matches!(r.glyph(a, &huge, &mut FrameBudget::frame()), Painted::Refused(_)));
    }

    #[test]
    #[ignore = "speed measure"]
    fn color_preview_text_preview_frame_speed() {
        let mut app = FontcraftApp::default();
        let names: Vec<String> = (0..50).map(|i| format!("g{i}")).collect();
        let mut text = String::new();
        for (i, name) in names.iter().enumerate() {
            let cp = 0xE000 + i as u32;
            exec(&mut app, "glyph.newGlyph", json!({"name": name, "unicodes": [format!("{cp:04X}")]}));
            let layer = add_colour_glyph(&mut app, name, "layer.convertToColor");
            exec(&mut app, "attributes.addGradient", json!({"glyph": name, "layer": layer}));
            exec(&mut app, "attributes.setStrokeWidth", json!({"glyph": name, "layer": layer, "width": 12}));
        }
        for i in 0..2_000u32 {
            text.push(char::from_u32(0xE000 + i % 50).unwrap());
            if i % 40 == 39 {
                text.push(' ');
            }
        }
        exec(&mut app, "window.textPreview", json!({}));
        exec(&mut app, "textPreview.setText", json!({"text": text}));
        let w = Window::new();
        w.settle(&mut app);
        // Frames until every colour glyph in view has been drawn once.
        let started = std::time::Instant::now();
        let (mut frames, mut last) = (0, usize::MAX);
        while frames < 500 {
            w.frame(&mut app, 0.01);
            frames += 1;
            let shown = app.state.panels.text_preview.cache.color.drawn().len();
            if shown == last && shown > 0 {
                break;
            }
            last = shown;
        }
        let cold = started.elapsed();
        for _ in 0..10 {
            w.frame(&mut app, 0.01);
        }
        let runs = 20;
        let started = std::time::Instant::now();
        for _ in 0..runs {
            w.frame(&mut app, 0.01);
        }
        let steady = started.elapsed() / runs;
        let shown = app.state.panels.text_preview.cache.color.drawn().len();
        println!("a page of colour text ({shown} colour glyphs in view): until drawn {cold:?} over {frames} frames, steady {steady:?}");
        assert!(shown > 100, "a page of colour glyphs: {shown}");
        if !cfg!(debug_assertions) {
            assert!(steady.as_secs_f64() < 0.016, "steady {steady:?}");
        }
    }
}
