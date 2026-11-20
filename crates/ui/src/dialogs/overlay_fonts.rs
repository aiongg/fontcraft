//! Font › Overlay Fonts… (`dialog.overlayFonts`): the sources `font.overlayFonts` stacks, bottom first, and a preview
//! of a few glyphs as the new font would draw them.
//!
//! | Control | Sets |
//! |---|---|
//! | one row per source | the open document, its colour (`#RRGGBBAA`), opacity, scale and offset |
//! | Add Source, Remove | a source more or less |
//! | Output | `colorLayers` (one colour-layered master) or `masters` (one master per source) |
//! | Preview Glyphs | the glyph names the preview draws, separated by spaces |
//! | Overlay | runs `font.overlayFonts`; the new document opens and the status line names it |
//!
//! The settings live in `ViewState::overlay_fonts`; the dialog shows a copy and hands each change back. After a change
//! [`refresh`] asks `font.overlayPreview` again, and the preview picture draws its SVG outlines in their colours. A
//! failed Overlay opens the dialog again with the message.

use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Picture, PictureSize, Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, WordSet as _};
use craft::ui::text::{NumberField, TextField};
use egui::{ColorImage, Ui};
use fontcraft_geom::{Affine, BezPath};
use fontcraft_render::{Style, render_path};
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};

craft::ui::words! {
    /// The words of the Overlay Fonts dialog.
    pub struct OverlayWords {
        title: "fontcraftOverlay.title" = "Overlay Fonts";
        font: "fontcraftOverlay.font" = "Font";
        color: "fontcraftOverlay.color" = "Colour";
        opacity: "fontcraftOverlay.opacity" = "Opacity";
        scale: "fontcraftOverlay.scale" = "Scale";
        offset_x: "fontcraftOverlay.offsetX" = "Offset X";
        offset_y: "fontcraftOverlay.offsetY" = "Offset Y";
        remove: "fontcraftOverlay.remove" = "Remove";
        add: "fontcraftOverlay.add" = "Add Source";
        output: "fontcraftOverlay.output" = "Output";
        color_layers: "fontcraftOverlay.colorLayers" = "Colour Layers";
        masters: "fontcraftOverlay.masters" = "Masters";
        glyphs: "fontcraftOverlay.glyphs" = "Preview Glyphs";
        preview: "fontcraftOverlay.preview" = "Preview";
        overlay: "fontcraftOverlay.overlay" = "Overlay";
        /// Before the new document's title in the status line.
        made: "fontcraftOverlay.made" = "Overlay Fonts made";
        /// After the new font's glyph count.
        glyph_count: "fontcraftOverlay.glyphCount" = "glyphs";
    }
}

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "dialog.overlayFonts",
    label: "Overlay Fonts…",
    menu: &["Font"],
    shortcut: None,
    params: "{layers?: [{font: index|title|path, master?, color?: \"#RRGGBBAA\", opacity?: 0 to 1, offset?: [x, y], scale?}] (default: one per open document, bottom first), output?: \"colorLayers\"|\"masters\" (default colorLayers), glyphs?: [names] (the preview's glyphs; default the shown glyph, else the font's first three), run?: bool} without run, open the dialog with these settings and preview them with font.overlayPreview; with run, run font.overlayFonts with the dialog's settings (or the ones given) and close it",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run: run_command };

/// Most sources the dialog offers; `font.overlayFonts` takes at most 256.
const MAX_SOURCES: usize = 256;
/// Most open documents the dialog starts with as sources.
const MAX_DEFAULT_SOURCES: usize = 8;
/// Most glyphs the preview draws.
const MAX_PREVIEW_GLYPHS: usize = 12;
/// Most characters of the Preview Glyphs field.
const MAX_GLYPHS_CHARS: usize = 1_000;
/// Most characters of a colour field.
const MAX_COLOR_CHARS: usize = 16;
/// The preview picture's size in pixels; the picture is fitted into its place.
const PICTURE_WIDTH: u32 = 640;
const PICTURE_HEIGHT: u32 = 160;
/// The colours new sources take in turn: data for the font's palette, not the window's look.
const SOURCE_COLORS: [&str; 6] = ["#000000FF", "#D32F2FFF", "#1976D2FF", "#388E3CFF", "#F9A825FF", "#7B1FA2FF"];
/// Mixed into the picture's texture key, so it does not meet another picture's.
const PICTURE_KEY: u64 = 0x6f76_6572_6c61_7900;

/// One source: an open document and how it is stacked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    /// The document's position among the open documents, as text (the pop-up's value).
    pub font: String,
    pub color: String,
    pub opacity: Option<f64>,
    pub scale: Option<f64>,
    pub dx: Option<f64>,
    pub dy: Option<f64>,
}

impl Source {
    fn new(font: usize, at: usize) -> Source {
        let color = SOURCE_COLORS.get(at % SOURCE_COLORS.len()).copied().unwrap_or("#000000FF");
        Source { font: font.to_string(), color: color.into(), opacity: Some(1.0), scale: Some(1.0), dx: Some(0.0), dy: Some(0.0) }
    }

    /// The source as `font.overlayFonts` takes it.
    fn param(&self) -> Value {
        let mut m = Map::new();
        m.insert("font".into(), self.font.trim().parse::<u64>().map_or_else(|_| json!(self.font), |i| json!(i)));
        if !self.color.trim().is_empty() {
            m.insert("color".into(), json!(self.color.trim()));
        }
        if let Some(opacity) = self.opacity {
            m.insert("opacity".into(), json!(opacity));
        }
        if let Some(scale) = self.scale {
            m.insert("scale".into(), json!(scale));
        }
        m.insert("offset".into(), json!([self.dx.unwrap_or(0.0), self.dy.unwrap_or(0.0)]));
        Value::Object(m)
    }

    /// A source from a `layers` entry; `None` for an entry that is no object or names no font.
    fn from_param(entry: &Value, at: usize) -> Option<Source> {
        let font = match entry.get("font")? {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => return None,
        };
        let number = |key: &str| entry.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
        let offset = |i: usize| entry.get("offset").and_then(|o| o.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite());
        let mut source = Source::new(0, at);
        source.font = font;
        if let Some(color) = entry.get("color").and_then(Value::as_str) {
            source.color = color.chars().take(MAX_COLOR_CHARS).collect();
        }
        source.opacity = number("opacity").or(source.opacity);
        source.scale = number("scale").or(source.scale);
        source.dx = offset(0).or(source.dx);
        source.dy = offset(1).or(source.dy);
        Some(source)
    }
}

/// What the dialog sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// Bottom first.
    pub sources: Vec<Source>,
    /// `colorLayers` or `masters`.
    pub output: String,
    /// The preview's glyph names, separated by spaces.
    pub glyphs: String,
}

impl Settings {
    fn layers(&self) -> Vec<Value> {
        self.sources.iter().map(Source::param).collect()
    }

    /// The parameters of `font.overlayFonts`.
    pub fn command_params(&self) -> Value {
        json!({ "layers": self.layers(), "output": self.output })
    }

    /// The glyphs the preview draws.
    pub fn glyph_names(&self) -> Vec<String> {
        self.glyphs.split_whitespace().take(MAX_PREVIEW_GLYPHS).map(str::to_string).collect()
    }

    /// The parameters of `font.overlayPreview`.
    pub fn preview_params(&self) -> Value {
        json!({ "layers": self.layers(), "glyphs": self.glyph_names() })
    }
}

/// One source's outline of one glyph, as the preview draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewLayer {
    /// Unmultiplied RGBA.
    pub color: [u8; 4],
    /// SVG path data in the bottom source's units, y up.
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewGlyph {
    pub glyph: String,
    /// The new font's advance; `None` when no source has the glyph.
    pub width: Option<f64>,
    pub layers: Vec<PreviewLayer>,
}

/// `font.overlayPreview`'s answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayPreview {
    pub units_per_em: f64,
    pub glyphs: Vec<PreviewGlyph>,
}

impl OverlayPreview {
    fn from_reply(reply: &Value) -> OverlayPreview {
        let list = |v: Option<&Value>| v.and_then(Value::as_array).cloned().unwrap_or_default();
        let glyphs = list(reply.get("glyphs"))
            .iter()
            .take(MAX_PREVIEW_GLYPHS)
            .map(|g| PreviewGlyph {
                glyph: g.get("glyph").and_then(Value::as_str).unwrap_or_default().to_string(),
                width: g.get("width").and_then(Value::as_f64).filter(|w| w.is_finite()),
                layers: list(g.get("layers"))
                    .iter()
                    .take(MAX_SOURCES)
                    .filter_map(|l| {
                        let color = parse_color(l.get("color")?.as_str()?)?;
                        Some(PreviewLayer { color, path: l.get("path")?.as_str()?.to_string() })
                    })
                    .collect(),
            })
            .collect();
        OverlayPreview { units_per_em: reply.get("unitsPerEm").and_then(Value::as_f64).unwrap_or(0.0), glyphs }
    }
}

/// `#RRGGBB` or `#RRGGBBAA` as unmultiplied RGBA.
fn parse_color(text: &str) -> Option<[u8; 4]> {
    let hex = text.strip_prefix('#')?;
    let part = |i: usize| hex.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok());
    match hex.len() {
        6 => Some([part(0)?, part(2)?, part(4)?, u8::MAX]),
        8 => Some([part(0)?, part(2)?, part(4)?, part(6)?]),
        _ => None,
    }
}

/// The dialog's state in the app layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayState {
    pub open: bool,
    pub settings: Settings,
    pub preview: OverlayPreview,
    /// Why the last preview or Overlay failed.
    pub error: Option<String>,
    /// Moves with every new preview.
    pub revision: u64,
    /// The `font.overlayPreview` parameters the preview was made for, as text.
    previewed: Option<String>,
}

/// The settings the dialog opens with: one source per open document, and the shown glyph (else the active font's
/// first three) to preview.
fn defaults(app: &FontCraftApp) -> Settings {
    let sources = (0..app.session.documents().len().min(MAX_DEFAULT_SOURCES)).map(|i| Source::new(i, i)).collect();
    let glyphs = match &app.view.glyph {
        Some(glyph) => glyph.clone(),
        None => app.session.font().map(|f| f.glyphs.iter().take(3).map(|g| g.name.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default(),
    };
    Settings { sources, output: "colorLayers".into(), glyphs }
}

/// `params`' settings over `base`: `layers`, `output` and `glyphs` replace what they name.
fn given(params: &Value, mut base: Settings) -> Settings {
    if let Some(layers) = params.get("layers").and_then(Value::as_array) {
        base.sources = layers.iter().take(MAX_SOURCES).enumerate().filter_map(|(at, e)| Source::from_param(e, at)).collect();
    }
    if let Some(output) = params.get("output").and_then(Value::as_str) {
        base.output = output.to_string();
    }
    match params.get("glyphs") {
        Some(Value::Array(names)) => base.glyphs = names.iter().filter_map(Value::as_str).take(MAX_PREVIEW_GLYPHS).collect::<Vec<_>>().join(" "),
        Some(Value::String(text)) => base.glyphs = text.chars().take(MAX_GLYPHS_CHARS).collect(),
        _ => {}
    }
    base
}

fn run_command(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = &command.params;
    if params.get("run").and_then(Value::as_bool) == Some(true) {
        let settings = given(params, app.view.overlay_fonts.settings.clone());
        app.view.overlay_fonts.settings = settings.clone();
        return match app.run_command("font.overlayFonts", settings.command_params()) {
            Ok(reply) => {
                app.view.overlay_fonts.open = false;
                app.view.overlay_fonts.error = None;
                let words = OverlayWords::english();
                let title = reply.get("title").and_then(Value::as_str).unwrap_or_default();
                let count = reply.get("glyphs").and_then(Value::as_u64).unwrap_or(0);
                app.status = format!("{} {title} ({count} {})", words.made.as_str(), words.glyph_count.as_str());
                Ok(())
            }
            Err(error) => {
                app.view.overlay_fonts.open = true;
                app.view.overlay_fonts.error = Some(error.clone());
                Err(error)
            }
        };
    }
    let settings = given(params, defaults(app));
    app.view.overlay_fonts = OverlayState { open: true, settings, ..Default::default() };
    refresh(app);
    Ok(())
}

/// Asks `font.overlayPreview` for the open dialog's settings, unless the preview already shows them.
pub fn refresh(app: &mut FontCraftApp) {
    let state = &app.view.overlay_fonts;
    if !state.open {
        return;
    }
    let params = state.settings.preview_params();
    let key = params.to_string();
    if state.previewed.as_deref() == Some(key.as_str()) {
        return;
    }
    let result = if state.settings.sources.is_empty() || state.settings.glyph_names().is_empty() {
        Ok(OverlayPreview::default())
    } else {
        app.session.query("font.overlayPreview", &params).map(|reply| OverlayPreview::from_reply(&reply)).map_err(|e| e.to_string())
    };
    let state = &mut app.view.overlay_fonts;
    match result {
        Ok(preview) => {
            state.preview = preview;
            state.error = None;
        }
        Err(error) => {
            state.preview = OverlayPreview::default();
            state.error = Some(error);
        }
    }
    state.previewed = Some(key);
    state.revision = state.revision.wrapping_add(1);
}

/// The preview's glyphs side by side on `paper` (premultiplied RGBA), each source's outline in its colour, bottom
/// source first.
pub fn picture(preview: &OverlayPreview, paper: [u8; 4]) -> ColorImage {
    let (w, h) = (PICTURE_WIDTH, PICTURE_HEIGHT);
    let mut pixels: Vec<u8> = std::iter::repeat_n(paper, (w * h) as usize).flatten().collect();
    let upm = if preview.units_per_em.is_finite() && preview.units_per_em > 0.0 { preview.units_per_em } else { 1000.0 };
    let advance = |g: &PreviewGlyph| g.width.filter(|w| *w > 0.0).unwrap_or(upm);
    let total: f64 = preview.glyphs.iter().map(advance).sum();
    let (width, height) = (f64::from(w), f64::from(h));
    let margin = height * 0.05;
    let s = (height * 0.7 / upm).min((width - 2.0 * margin) / total.max(1.0));
    if s.is_finite() && s > 0.0 {
        let baseline = height * 0.78;
        let mut x = margin + ((width - 2.0 * margin) - total * s).max(0.0) / 2.0;
        for glyph in &preview.glyphs {
            for layer in &glyph.layers {
                let Ok(path) = BezPath::from_svg(&layer.path) else { continue };
                let to_pixels = Affine::translate((x, baseline)) * Affine::scale_non_uniform(s, -s);
                let style = Style { ink: layer.color, background: [0, 0, 0, 0], anti_alias: true };
                if let Ok(image) = render_path(&path, to_pixels, w, h, &style) {
                    over(&mut pixels, &image.pixels);
                }
            }
            x += advance(glyph) * s;
        }
    }
    ColorImage::from_rgba_premultiplied([w as usize, h as usize], &pixels)
}

/// Puts premultiplied `src` over `dst`, pixel by pixel.
fn over(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let alpha = u16::from(s.get(3).copied().unwrap_or(0));
        for (dc, sc) in d.iter_mut().zip(s) {
            let mixed = u16::from(*sc) + u16::from(*dc) * (255 - alpha) / 255;
            *dc = u8::try_from(mixed.min(255)).unwrap_or(u8::MAX);
        }
    }
}

/// The dialog: a copy of the settings, the documents to choose from and the preview to show.
pub struct OverlayDialog {
    id: DialogId,
    settings: Settings,
    documents: Vec<Entry<String>>,
    preview: OverlayPreview,
    error: Option<String>,
    revision: u64,
    paper: [u8; 4],
    open: bool,
}

impl OverlayDialog {
    fn new(app: &FontCraftApp) -> OverlayDialog {
        let state = &app.view.overlay_fonts;
        let documents =
            app.session.documents().iter().enumerate().take(MAX_SOURCES).map(|(i, d)| Entry::new(i.to_string(), d.title.clone())).collect();
        OverlayDialog {
            id: dialog_id("font.overlayFonts"),
            settings: state.settings.clone(),
            documents,
            preview: state.preview.clone(),
            error: state.error.clone(),
            revision: state.revision,
            paper: crate::panels::preview::PreviewTokens::standard().paper.to_array(),
            open: true,
        }
    }
}

impl StackDialog for OverlayDialog {
    type Result = Settings;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Settings>> {
        let words = Env::of(ui).words::<OverlayWords>();
        let outputs = [Entry::new("colorLayers".to_string(), words.color_layers.clone()), Entry::new("masters".to_string(), words.masters.clone())];
        let OverlayDialog { settings, documents, preview, error, revision, paper, open, .. } = self;
        let can_run = !settings.sources.is_empty();
        let mut shown_value = false;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut shown_value))
            .opened(Bind::bound(open))
            .ok_label(words.overlay.clone())
            .ok_enabled(can_run)
            .show(ui, |ui, _, _| {
                let mut remove = None;
                for (at, source) in settings.sources.iter_mut().enumerate() {
                    let _ = Row::new().show(ui, |ui, _| {
                        Select::new(words.font.clone(), Bind::bound(&mut source.font), documents.as_slice())
                            .id_salt(("fontcraftOverlay.font", at))
                            .show(ui);
                        TextField::new(words.color.clone(), Bind::bound(&mut source.color))
                            .limit(MAX_COLOR_CHARS)
                            .id_salt(("fontcraftOverlay.color", at))
                            .show(ui);
                        let numbers = [
                            (&words.opacity, &mut source.opacity, "fontcraftOverlay.opacity"),
                            (&words.scale, &mut source.scale, "fontcraftOverlay.scale"),
                            (&words.offset_x, &mut source.dx, "fontcraftOverlay.offsetX"),
                            (&words.offset_y, &mut source.dy, "fontcraftOverlay.offsetY"),
                        ];
                        for (label, value, salt) in numbers {
                            NumberField::new(label.clone(), Bind::bound(value)).id_salt((salt, at)).show(ui);
                        }
                        if Button::new(words.remove.clone()).id_salt(("fontcraftOverlay.remove", at)).show(ui).clicked() {
                            remove = Some(at);
                        }
                    });
                }
                if let Some(at) = remove.filter(|at| *at < settings.sources.len()) {
                    settings.sources.remove(at);
                }
                if settings.sources.len() < MAX_SOURCES && Button::new(words.add.clone()).show(ui).clicked() {
                    let at = settings.sources.len();
                    settings.sources.push(Source::new(0, at));
                }
                Segmented::new(words.output.clone(), Bind::bound(&mut settings.output), &outputs).show(ui);
                TextField::new(words.glyphs.clone(), Bind::bound(&mut settings.glyphs)).limit(MAX_GLYPHS_CHARS).show(ui);
                let image = || picture(preview, *paper);
                Picture::new(words.preview.clone(), PICTURE_KEY ^ *revision, &image)
                    .aspect(ThumbAspect::new(4, 1))
                    .extent(PictureSize::Thumb(ThumbSize::Large))
                    .show(ui);
                if let Some(error) = error {
                    Text::new(error.clone()).shade(Shade::Muted).show(ui);
                }
                true
            });
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(settings.clone())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the dialog while the app state asks for it, hands its changes to the app state (which previews them), gives
/// it each new preview, and posts Overlay.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(answer) = host.take::<OverlayDialog>() {
        app.view.overlay_fonts.open = false;
        if let DialogResult::Ok(settings) = answer {
            app.view.overlay_fonts.settings = settings;
            app.sent.push(Msg::Shell(CommandRef::with_params("dialog.overlayFonts", json!({ "run": true }))));
        }
    }
    if !app.view.overlay_fonts.open {
        return;
    }
    let Some(dialog) = host.get_mut::<OverlayDialog>() else {
        host.open(OverlayDialog::new(app));
        return;
    };
    if dialog.settings != app.view.overlay_fonts.settings {
        app.view.overlay_fonts.settings = dialog.settings.clone();
        refresh(app);
    }
    let state = &app.view.overlay_fonts;
    if dialog.revision != state.revision {
        dialog.preview = state.preview.clone();
        dialog.error = state.error.clone();
        dialog.revision = state.revision;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::picture;
    use crate::app::{APP, FontCraftApp, Msg};

    fn shell(params: serde_json::Value) -> Msg {
        Msg::Shell(CommandRef::with_params("dialog.overlayFonts", params))
    }

    /// Overlay Fonts previews the stacked outlines in their colours, and Overlay makes the new document.
    #[test]
    fn overlay_fonts_previews_then_makes_the_font() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.session.execute("glyph.newGlyph", &json!({"name": "A"})).ok();
        app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 0, "width": 300, "height": 500})).unwrap();
        let layers = json!([{"font": 0, "color": "#FF0000FF"}, {"font": 0, "color": "#0000FF80", "offset": [40, 0]}]);
        app.sent.push(shell(json!({"layers": layers, "glyphs": ["A"]})));
        app.drain();

        let state = app.view.overlay_fonts.clone();
        assert!(state.open, "{}", app.status);
        assert_eq!(state.error, None);
        let glyph = state.preview.glyphs.first().expect("A is previewed");
        assert_eq!(glyph.layers.len(), 2);
        assert_eq!(glyph.layers[1].color, [0, 0, 255, 128]);
        let paper = [255, 255, 255, 255];
        assert!(picture(&state.preview, paper).pixels.iter().any(|p| p.to_array() != paper), "the outlines are drawn");

        let before = app.session.documents().len();
        app.sent.push(shell(json!({"run": true})));
        app.drain();
        assert_eq!(app.session.documents().len(), before + 1, "{}", app.status);
        assert!(!app.view.overlay_fonts.open);
        assert!(app.status.starts_with("Overlay Fonts made"), "{}", app.status);
    }
}
