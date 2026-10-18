//! The shell: menus, context menus, keymap, tab strip, status bar, Font View, Palette, the tool
//! and selection commands, and the shell commands.
//!
//! Shell commands change view and window state only; everything else is an engine command. Each
//! module lists its own commands ([`MODULES`]), so a package adds a module instead of editing a
//! shared list.

pub mod actions;
pub mod annotation;
pub mod attach;
pub mod audit;
pub mod clipboard;
pub mod command_console;
pub mod compare_family;
pub mod compare_fonts;
pub mod components;
pub mod context_menu;
pub mod drawing;
pub mod edit_aids;
pub mod extensions;
pub mod font_info;
pub mod font_view;
pub mod glyph_info;
pub mod item_state;
pub mod kerning_review;
pub mod kerning_window;
pub mod keymap;
pub mod master_switcher;
pub mod masters;
pub mod measurement;
pub mod menus;
pub mod menus_dynamic;
pub mod palette;
pub mod pickers;
pub mod preview;
pub mod preview_panel;
pub mod selection;
pub mod space_center;
pub mod start_window;
pub mod status_bar;
pub mod tabs;
pub mod text;
pub mod text_preview;
pub mod toolbar;
pub mod transform;
pub mod url;
pub mod view;

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::app::FontcraftApp;

/// A shell command: `(id, label, menu path, shortcut, params doc)`.
pub type UiCommand = (&'static str, &'static str, &'static [&'static str], &'static str, &'static str);

/// Runs one of a module's commands.
pub type Handler = fn(&mut FontcraftApp, &str, &Args) -> Result<Value, String>;

/// Every shell module: its commands and the function that runs them.
pub const MODULES: &[(&[UiCommand], Handler)] = &[
    (font_view::COMMANDS, font_view::run),
    (tabs::COMMANDS, tabs::run),
    (view::COMMANDS, view::run),
    (palette::COMMANDS, palette::run),
    (selection::COMMANDS, selection::run),
    (crate::dialogs::COMMANDS, crate::dialogs::run),
    (transform::COMMANDS, transform::run),
    (crate::panels::COMMANDS, crate::panels::run),
    (annotation::COMMANDS, annotation::run),
    (drawing::COMMANDS, drawing::run),
    (audit::COMMANDS, audit::run),
    (masters::COMMANDS, masters::run),
    (font_view::inspector::COMMANDS, font_view::inspector::run),
    (font_view::sidebar::COMMANDS, font_view::sidebar::run),
    (glyph_info::COMMANDS, glyph_info::run),
    (start_window::COMMANDS, start_window::run),
    (text::COMMANDS, text::run),
    (crate::canvas::group_members::COMMANDS, crate::canvas::group_members::run),
    (crate::canvas::compatibility::COMMANDS, crate::canvas::compatibility::run),
    (kerning_window::COMMANDS, kerning_window::run),
    (font_view::metrics::COMMANDS, font_view::metrics::run),
    (preview::COMMANDS, preview::run),
    (measurement::COMMANDS, measurement::run),
    (text_preview::COMMANDS, text_preview::run),
    (crate::dialogs::sample_strings::COMMANDS, crate::dialogs::sample_strings::run),
    (crate::dialogs::find_glyphs::COMMANDS, crate::dialogs::find_glyphs::run),
    (font_info::COMMANDS, font_info::run),
    (extensions::COMMANDS, extensions::run),
    (command_console::COMMANDS, command_console::run),
    (actions::COMMANDS, actions::run),
    (space_center::COMMANDS, space_center::run),
    (kerning_review::COMMANDS, kerning_review::run),
    (edit_aids::COMMANDS, edit_aids::run),
    (attach::COMMANDS, attach::run),
    (components::COMMANDS, components::run),
    (crate::panels::reference_font::COMMANDS, crate::panels::reference_font::run),
    (crate::panels::selections::COMMANDS, crate::panels::selections::run),
    (url::COMMANDS, url::run),
    (font_view::encoding::COMMANDS, font_view::encoding::run),
    (crate::dialogs::compare_fonts::COMMANDS, crate::dialogs::compare_fonts::run),
];

/// The shell commands' parameters that name files, declared as engine commands declare theirs
/// (`fontcraft_engine::cmd::FileParam`), so the automation roots
/// (`fontcraft_engine::file_access`) check them. Every shell command whose parameters name a file is listed here.
pub const UI_FILE_PARAMS: &[(&str, &[fontcraft_engine::cmd::FileParam])] = &[
    ("file.showInFinder", &[fontcraft_engine::cmd::FileParam::read("path")]),
    ("view.setReferenceFont", &[fontcraft_engine::cmd::FileParam::read("path")]),
    ("view.filterByEncodingFile", &[fontcraft_engine::cmd::FileParam::read("path")]),
];

/// The file parameters [`UI_FILE_PARAMS`] declares for the shell command `id`.
pub fn ui_file_params(id: &str) -> &'static [fontcraft_engine::cmd::FileParam] {
    UI_FILE_PARAMS.iter().find(|(c, _)| *c == id).map_or(&[], |(_, f)| f)
}

/// Every shell command, in module order.
pub static UI_COMMANDS: LazyLock<Vec<UiCommand>> = LazyLock::new(|| MODULES.iter().flat_map(|(c, _)| c.iter().copied()).collect());

/// Draws the shell's floating windows, one call per window. Their buttons queue commands in
/// `pending`.
pub fn windows(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    glyph_info::show(app, ui, pending);
    start_window::show(app, ui, pending);
    kerning_window::show(app, ui, pending);
    preview_panel::show(app, ui, pending);
    text_preview::show(app, ui, pending);
    crate::dialogs::sample_strings::show(app, ui, pending);
    crate::dialogs::find_glyphs::show(app, ui, pending);
    font_info::show(app, ui, pending);
    extensions::show(app, ui, pending);
    command_console::show(app, ui, pending);
    actions::show(app, ui, pending);
    space_center::show(app, ui, pending);
    kerning_review::show(app, ui, pending);
    crate::dialogs::compare_fonts::show(app, ui, pending);
}

pub fn is_ui_command(id: &str) -> bool {
    UI_COMMANDS.iter().any(|c| c.0 == id)
}

/// Runs a shell command. The caller catches panics.
pub(crate) fn run(app: &mut FontcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    let args = Args::new(id, params)?;
    let (_, handler) = MODULES.iter().find(|(c, _)| c.iter().any(|c| c.0 == id)).ok_or_else(|| format!("unknown command `{id}`"))?;
    handler(app, id, &args)
}

/// A shell command's parameters, with typed readers whose errors name the command and the key.
pub struct Args<'a> {
    id: &'a str,
    obj: Option<&'a Map<String, Value>>,
}

impl<'a> Args<'a> {
    pub fn new(id: &'a str, params: &'a Value) -> Result<Self, String> {
        match params {
            Value::Null => Ok(Self { id, obj: None }),
            Value::Object(m) => Ok(Self { id, obj: Some(m) }),
            _ => Err(format!("{id}: params must be a JSON object")),
        }
    }

    /// An error message about this command.
    pub fn err(&self, message: impl std::fmt::Display) -> String {
        format!("{}: {message}", self.id)
    }

    /// The value of `key`; `null` counts as absent.
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.obj.and_then(|m| m.get(key)).filter(|v| !v.is_null())
    }

    /// `on` for a display toggle: the value given, or the opposite of `current` when absent.
    pub fn toggle(&self, current: bool) -> Result<bool, String> {
        match self.get("on") {
            None => Ok(!current),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(self.err("`on` must be true or false")),
        }
    }

    /// A finite number.
    pub fn number(&self, key: &str) -> Result<Option<f64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_f64().filter(|n| n.is_finite()).map(Some).ok_or_else(|| self.err(format!("`{key}` must be a number"))),
        }
    }

    /// A whole number of zero or more.
    pub fn index(&self, key: &str) -> Result<Option<u64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_u64().map(Some).ok_or_else(|| self.err(format!("`{key}` must be a whole number"))),
        }
    }

    /// A list of numbers of exactly `N` entries.
    pub fn numbers<const N: usize>(&self, key: &str) -> Result<Option<[f64; N]>, String> {
        let Some(v) = self.get(key) else { return Ok(None) };
        let bad = || self.err(format!("`{key}` must be a list of {N} numbers"));
        let list = v.as_array().filter(|a| a.len() == N).ok_or_else(bad)?;
        let mut out = [0.0; N];
        for (slot, item) in out.iter_mut().zip(list) {
            *slot = item.as_f64().filter(|n| n.is_finite()).ok_or_else(bad)?;
        }
        Ok(Some(out))
    }

    /// A list of strings of at most `max` entries.
    pub fn strings(&self, key: &str, max: usize) -> Result<Option<Vec<String>>, String> {
        let Some(v) = self.get(key) else { return Ok(None) };
        let bad = || self.err(format!("`{key}` must be a list of names"));
        let list = v.as_array().ok_or_else(bad)?;
        if list.len() > max {
            return Err(self.err(format!("`{key}` holds at most {max} names")));
        }
        list.iter().map(|n| n.as_str().map(str::to_string).ok_or_else(bad)).collect::<Result<Vec<_>, _>>().map(Some)
    }

    /// `true` or `false`.
    pub fn flag(&self, key: &str) -> Result<Option<bool>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_bool().map(Some).ok_or_else(|| self.err(format!("`{key}` must be true or false"))),
        }
    }

    /// All parameters as one JSON object (`null` when none were given).
    pub fn value(&self) -> Value {
        self.obj.map_or(Value::Null, |m| Value::Object(m.clone()))
    }

    /// A string.
    pub fn string(&self, key: &str) -> Result<Option<&'a str>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| self.err(format!("`{key}` must be a string"))),
        }
    }
}
