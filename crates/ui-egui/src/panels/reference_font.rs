//! Palette › Reference Font: fonts to draw against (`palette.referenceFont`). A path field adds a
//! font, the list selects one or removes it, a switch shows or hides its glyph behind the edited
//! outline, and two fields set a custom character and a language code. Every control runs
//! `view.setReferenceFont`, the module's shell command. `crate::canvas::reference_font` draws.
//!
//! A reference font is an untrusted file opened read-only through the engine's reading code
//! (`fontcraft_engine::io::open`, with its size and model limits). It never joins the session:
//! only the selected font is held, in [`ReferenceFontState`], and the file is not written.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use fontcraft_model::{Font, Glyph, Layer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Pending, palette, widgets};
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};

/// Fonts the list keeps.
pub const MAX_REFERENCE_FONTS: usize = 16;
/// Bytes of the custom character and language fields.
const MAX_FIELD_BYTES: usize = 64;

/// The reference fonts and how the selected one is shown.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReferenceFontState {
    /// The fonts added, as their paths, in the order they were added.
    pub paths: Vec<String>,
    /// The path of the font drawn.
    pub selected: Option<String>,
    /// The selected font's glyph is drawn behind the outline.
    pub show: bool,
    /// When not empty, its first character chooses the glyph instead of the edited glyph's.
    pub character: String,
    /// A language code; a glyph named like the found one plus `.locl` and the code in capitals
    /// is drawn instead when the font has one.
    pub language: String,
    /// The selected font as read. Empty after the state is restored, until the font is selected
    /// again.
    #[serde(skip)]
    pub loaded: Loaded,
}

/// A read reference font, shared between copies of the state.
#[derive(Clone, Default)]
pub struct Loaded(pub Option<Arc<Reference>>);

impl PartialEq for Loaded {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(r) => write!(f, "Loaded({})", r.path),
            None => write!(f, "Loaded(none)"),
        }
    }
}

/// A reference font: its path, the font, and its glyphs by code point.
pub struct Reference {
    pub path: String,
    pub font: Font,
    by_code: BTreeMap<u32, String>,
}

impl Reference {
    /// Reads the font at `path`.
    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code, unused_variables))]
    pub fn read(path: &str) -> Result<Self, String> {
        #[cfg(target_arch = "wasm32")]
        let font: Font = return Err(format!("{path}: the web app cannot read font files"));
        #[cfg(not(target_arch = "wasm32"))]
        let (font, _) = fontcraft_engine::io::open(Path::new(path), None).map_err(|e| e.to_string())?;
        let mut by_code = BTreeMap::new();
        for g in font.glyphs.iter() {
            for &u in &g.unicodes {
                by_code.entry(u).or_insert_with(|| g.name.clone());
            }
        }
        Ok(Self { path: path.to_string(), font, by_code })
    }

    /// The glyph to draw behind `edited`: by the custom character when one is set, else by the
    /// edited glyph's first code point, else by its name; then its localised form for `language`
    /// when the font has one.
    pub fn glyph_for(&self, edited: &Glyph, character: &str, language: &str) -> Option<&Glyph> {
        let code = character.chars().next().map(u32::from).or_else(|| edited.unicodes.first().copied());
        let base = match code {
            Some(u) => self.by_code.get(&u).and_then(|n| self.font.glyph(n)),
            None => self.font.glyph(&edited.name),
        }?;
        let lang = language.trim();
        if !lang.is_empty()
            && let Some(local) = self.font.glyph(&format!("{}.locl{}", base.name, lang.to_uppercase()))
        {
            return Some(local);
        }
        Some(base)
    }

    /// `glyph`'s layer in the font's first master, else its first layer.
    pub fn layer<'a>(&self, glyph: &'a Glyph) -> Option<&'a Layer> {
        let master = self.font.default_master().ok().map(|m| m.id.as_str());
        master.and_then(|m| glyph.layer(m)).or_else(|| glyph.layers.first())
    }
}

pub const COMMANDS: &[UiCommand] = &[(
    "view.setReferenceFont",
    "Set Reference Font",
    &["View"],
    "",
    "{path?: path (add the font and select it, or select it when it is listed; adding turns the display on), remove?: path (take a listed font off the list), show?: bool, character?: string (its first character chooses the glyph; empty: the edited glyph's), language?: string (a code such as \"TRK\"; empty: none)} the fonts drawn against: the selected font's glyph for the edited character is drawn behind the outline. The font is read, never changed; the list keeps at most 16 → {paths, selected, show, character, language, loaded: bool}",
)];

fn field(args: &Args, key: &str) -> Result<Option<String>, String> {
    let Some(v) = args.string(key)? else { return Ok(None) };
    if v.len() > MAX_FIELD_BYTES || v.chars().any(char::is_control) {
        return Err(args.err(format!("`{key}` must be at most {MAX_FIELD_BYTES} bytes without control characters")));
    }
    Ok(Some(v.to_string()))
}

fn path_param(args: &Args, key: &str) -> Result<Option<String>, String> {
    let Some(v) = args.string(key)? else { return Ok(None) };
    if v.trim().is_empty() || v.chars().any(char::is_control) {
        return Err(args.err(format!("`{key}` must be a file path")));
    }
    Ok(Some(v.to_string()))
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "view.setReferenceFont" {
        return Err(format!("unknown command `{id}`"));
    }
    let path = path_param(args, "path")?;
    let remove = path_param(args, "remove")?;
    let show = args.flag("show")?;
    let character = field(args, "character")?;
    let language = field(args, "language")?;
    if path.is_some() && remove.is_some() {
        return Err(args.err("give `path` or `remove`, not both"));
    }
    if path.is_none() && remove.is_none() && show.is_none() && character.is_none() && language.is_none() {
        return Err(args.err("give `path`, `remove`, `show`, `character` or `language`"));
    }
    let mut next = app.state.panels.reference_font.clone();
    if let Some(path) = path {
        let listed = next.paths.contains(&path);
        if !listed && next.paths.len() >= MAX_REFERENCE_FONTS {
            return Err(args.err(format!("the list holds at most {MAX_REFERENCE_FONTS} fonts; remove one first")));
        }
        let held = next.loaded.0.as_ref().is_some_and(|r| r.path == path);
        if !held {
            let read = Reference::read(&path).map_err(|e| args.err(e))?;
            next.loaded = Loaded(Some(Arc::new(read)));
        }
        if !listed {
            next.paths.push(path.clone());
            next.show = true;
        }
        next.selected = Some(path);
    }
    if let Some(remove) = remove {
        let before = next.paths.len();
        next.paths.retain(|p| *p != remove);
        if next.paths.len() == before {
            return Err(args.err(format!("`{remove}` is not in the list")));
        }
        if next.selected.as_deref() == Some(remove.as_str()) {
            next.selected = None;
            next.loaded = Loaded::default();
        }
    }
    if let Some(show) = show {
        next.show = show;
    }
    if let Some(c) = character {
        next.character = c;
    }
    if let Some(l) = language {
        next.language = l;
    }
    let reply = json!({
        "paths": next.paths,
        "selected": next.selected,
        "show": next.show,
        "character": next.character,
        "language": next.language,
        "loaded": next.loaded.0.is_some(),
    });
    app.state.panels.reference_font = next;
    Ok(reply)
}

/// Draws the section.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let state = app.state.panels.reference_font.clone();
    ui.horizontal(|ui| {
        ui.label("Font file");
        if let Some(path) = widgets::text(ui, widgets::id("referenceFont.add"), "", 160.0)
            && !path.trim().is_empty()
        {
            pending.push(("view.setReferenceFont", json!({ "path": path.trim() })));
        }
    });
    if state.paths.is_empty() {
        palette::note(app, ui, "Add a font file to draw against");
    }
    for path in &state.paths {
        let selected = state.selected.as_deref() == Some(path.as_str());
        let name = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
        ui.horizontal(|ui| {
            if widgets::button(ui, widgets::id(("referenceFont.select", path)), name, path, true, selected) {
                pending.push(("view.setReferenceFont", json!({ "path": path })));
            }
            if widgets::flat_button(ui, widgets::id(("referenceFont.remove", path)), "Remove", "Take this font off the list") {
                pending.push(("view.setReferenceFont", json!({ "remove": path })));
            }
        });
    }
    if state.selected.is_some() && state.loaded.0.is_none() {
        palette::note(app, ui, "Click the font to read it again");
    }
    ui.horizontal(|ui| {
        if widgets::button(ui, widgets::id("referenceFont.show"), "Show", "Draw the reference glyph behind the outline", true, state.show) {
            pending.push(("view.setReferenceFont", json!({ "show": !state.show })));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Character");
        if let Some(c) = widgets::text(ui, widgets::id("referenceFont.character"), &state.character, 48.0) {
            pending.push(("view.setReferenceFont", json!({ "character": c })));
        }
        ui.label("Language");
        if let Some(l) = widgets::text(ui, widgets::id("referenceFont.language"), &state.language, 48.0) {
            pending.push(("view.setReferenceFont", json!({ "language": l })));
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Shape};
    use serde_json::json;

    use crate::FontcraftApp;

    fn frame(ctx: &egui::Context, app: &mut FontcraftApp) -> Vec<Shape> {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
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

    /// Line strokes drawn in the reference font's colour.
    fn reference_strokes(shapes: &[Shape], app: &FontcraftApp) -> Vec<Vec<Pos2>> {
        let colour = app.tokens.reference_font_outline;
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(colour) => Some(p.points.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_compiled_fonts_glyph_is_drawn_behind_the_edited_one() {
        // The reference: a compiled font whose A is a 500-unit square, and whose A.loclTRK is a
        // 200-unit one.
        let dir = std::env::temp_dir().join(format!("fontcraft-reference-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ttf = dir.join("Reference.ttf");
        let mut src = FontcraftApp::default();
        src.run("path.addRectangle", &json!({"glyph": "A", "x": 0, "y": 0, "width": 500, "height": 500})).unwrap();
        src.run("glyph.newGlyph", &json!({"name": "A.loclTRK"})).unwrap();
        src.run("path.addRectangle", &json!({"glyph": "A.loclTRK", "x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
        src.run("file.exportStatic", &json!({"destination": ttf})).unwrap();

        // The edited font's A is empty and mapped to U+0041 like the reference's.
        let mut app = FontcraftApp::default();
        let a = app.session.font().unwrap().glyph("A").unwrap().unicodes.clone();
        assert_eq!(a, vec![0x41], "the new font maps A");
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [250, 250]})).unwrap();
        let ctx = egui::Context::default();
        assert!(reference_strokes(&frame(&ctx, &mut app), &app).is_empty(), "nothing is drawn before a font is added");

        let r = app.run("view.setReferenceFont", &json!({"path": ttf})).unwrap();
        assert_eq!((r["loaded"].as_bool(), r["show"].as_bool()), (Some(true), Some(true)), "{r}");
        frame(&ctx, &mut app);
        let strokes = reference_strokes(&frame(&ctx, &mut app), &app);
        assert_eq!(strokes.len(), 1, "one contour: the reference A");
        let width = |s: &[Pos2]| {
            let xs: Vec<f32> = s.iter().map(|p| p.x).collect();
            xs.iter().copied().fold(f32::MIN, f32::max) - xs.iter().copied().fold(f32::MAX, f32::min)
        };
        let big = width(&strokes[0]);

        // The language code picks the localised form, which is smaller.
        app.run("view.setReferenceFont", &json!({"language": "trk"})).unwrap();
        let strokes = reference_strokes(&frame(&ctx, &mut app), &app);
        assert_eq!(strokes.len(), 1);
        assert!((width(&strokes[0]) * 2.5 - big).abs() < 2.0, "A.loclTRK is 200 units where A is 500");

        // Hidden, nothing; a file that is not a font is refused and changes nothing.
        app.run("view.setReferenceFont", &json!({"show": false})).unwrap();
        assert!(reference_strokes(&frame(&ctx, &mut app), &app).is_empty());
        let junk = dir.join("junk.ttf");
        std::fs::write(&junk, b"not a font").unwrap();
        let before = app.state.panels.reference_font.clone();
        assert!(app.run("view.setReferenceFont", &json!({"path": junk})).is_err());
        assert_eq!(app.state.panels.reference_font, before);
        let r = app.run("view.setReferenceFont", &json!({"remove": ttf})).unwrap();
        assert_eq!((r["paths"].clone(), r["loaded"].as_bool()), (json!([]), Some(false)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
