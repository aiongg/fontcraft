//! Reference Font (`view.setReferenceFont`): fonts to draw against. The list keeps the fonts added; the selected one
//! is read and held in [`ReferenceFontState::loaded`], and its glyph for the edited character is drawn behind the
//! outline while `show` is on (the canvas layer reads [`Reference::glyph_for`]).
//!
//! A reference font is an untrusted file opened read-only through the engine's reading code
//! (`fontcraft_engine::io::open`, with its size and model limits). It never joins the session, and the file is never
//! written.

use std::collections::BTreeMap;
use std::sync::Arc;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::{Font, Glyph, Layer};
use serde_json::Value;

use super::commands::{Module, UiCommand};
use super::params::{flag, string};
use crate::app::FontCraftApp;

/// Fonts the list keeps.
pub const MAX_REFERENCE_FONTS: usize = 16;
/// Bytes of the custom character and language fields.
const MAX_FIELD_BYTES: usize = 64;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "view.setReferenceFont",
    label: "Set Reference Font",
    menu: &[],
    shortcut: None,
    params: "{path?: path (add the font and select it, or select it when it is listed; adding turns the display on), remove?: path (take a listed font off the list), show?: bool, character?: string (its first character chooses the glyph; empty: the edited glyph's), language?: string (a code such as \"TRK\"; empty: none)} the fonts drawn against: the selected font's glyph for the edited character is drawn behind the outline. The font is read, never changed; the list keeps at most 16",
}];

/// The reference fonts and how the selected one is shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReferenceFontState {
    /// The fonts added, as their paths, in the order they were added.
    pub paths: Vec<String>,
    /// The path of the font drawn.
    pub selected: Option<String>,
    /// The selected font's glyph is drawn behind the outline.
    pub show: bool,
    /// When not empty, its first character chooses the glyph instead of the edited glyph's.
    pub character: String,
    /// A language code: a glyph named like the found one plus `.locl` and the code in capitals is drawn instead when
    /// the font has one.
    pub language: String,
    /// The selected font as read.
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
        let (font, _) = fontcraft_engine::io::open(std::path::Path::new(path), None).map_err(|e| e.to_string())?;
        let mut by_code = BTreeMap::new();
        for g in font.glyphs.iter() {
            for &u in &g.unicodes {
                by_code.entry(u).or_insert_with(|| g.name.clone());
            }
        }
        Ok(Self { path: path.to_string(), font, by_code })
    }

    /// The glyph to draw behind `edited`: by the custom character when one is set, else by the edited glyph's first
    /// code point, else by its name; then its localised form for `language` when the font has one.
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

fn field(p: &Value, key: &str) -> Result<Option<String>, String> {
    let Some(v) = string(p, key)? else { return Ok(None) };
    if v.len() > MAX_FIELD_BYTES || v.chars().any(char::is_control) {
        return Err(format!("`{key}` must be at most {MAX_FIELD_BYTES} bytes without control characters"));
    }
    Ok(Some(v.to_string()))
}

fn path_param(p: &Value, key: &str) -> Result<Option<String>, String> {
    let Some(v) = string(p, key)? else { return Ok(None) };
    if v.trim().is_empty() || v.chars().any(char::is_control) {
        return Err(format!("`{key}` must be a file path"));
    }
    Ok(Some(v.to_string()))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "view.setReferenceFont" => {
            app.view.reference_font = set(&app.view.reference_font, &command.params)?;
            Ok(())
        }
        other => Err(format!("no reference font command `{other}`")),
    }
}

/// `state` after `view.setReferenceFont` with `p`. Everything is checked, and the font read, before anything changes.
pub fn set(state: &ReferenceFontState, p: &Value) -> Result<ReferenceFontState, String> {
    let path = path_param(p, "path")?;
    let remove = path_param(p, "remove")?;
    let show = flag(p, "show")?;
    let character = field(p, "character")?;
    let language = field(p, "language")?;
    if path.is_some() && remove.is_some() {
        return Err("give `path` or `remove`, not both".into());
    }
    if path.is_none() && remove.is_none() && show.is_none() && character.is_none() && language.is_none() {
        return Err("give `path`, `remove`, `show`, `character` or `language`".into());
    }
    let mut next = state.clone();
    if let Some(path) = path {
        let listed = next.paths.contains(&path);
        if !listed && next.paths.len() >= MAX_REFERENCE_FONTS {
            return Err(format!("the list holds at most {MAX_REFERENCE_FONTS} fonts; remove one first"));
        }
        if !next.loaded.0.as_ref().is_some_and(|r| r.path == path) {
            next.loaded = Loaded(Some(Arc::new(Reference::read(&path)?)));
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
            return Err(format!("`{}` is not in the list", super::params::quoted(&remove)));
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
    Ok(next)
}
