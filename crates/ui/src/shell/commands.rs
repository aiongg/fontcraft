//! Shell commands: view and window state only, never the engine.
//!
//! Each module lists its own commands and the handler that runs them, and adds one line to [`MODULES`]. The catalog
//! puts them in menus and the keymap beside the engine commands; a fired shell command reaches its handler through
//! `Msg::Shell` in `drain`.

use craft::ui::runtime::commands::CommandRef;

use crate::app::FontCraftApp;

/// One shell command, described as an engine command is.
pub struct UiCommand {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement; empty: not in a menu.
    pub menu: &'static [&'static str],
    /// Default shortcut in the engine's notation (`Cmd+Alt+P`).
    pub shortcut: Option<&'static str>,
    /// Parameters, readable by people and agents.
    pub params: &'static str,
}

/// Runs one of a module's commands. An error goes to the status line.
pub type Handler = fn(&mut FontCraftApp, &CommandRef) -> Result<(), String>;

/// A module's commands and their handler.
pub struct Module {
    pub commands: &'static [UiCommand],
    pub run: Handler,
}

/// Every shell module.
pub const MODULES: &[Module] = &[
    super::window::MODULE,
    super::view::MODULE,
    super::font_view::MODULE,
    super::font_info::MODULE,
    super::font_info_tabs::MODULE,
    super::reference_font::MODULE,
    super::kerning_review::MODULE,
    super::glyph_info::MODULE,
    crate::dialogs::tt::SHELL,
    super::selection::MODULE,
    super::masters::MODULE,
    super::command_surfaces::MODULE,
    crate::dialogs::form::MODULE,
    crate::dialogs::export::MODULE,
    super::kerning_window::MODULE,
    super::kerning_tools::MODULE,
    super::sidebar::MODULE,
    super::palette::MODULE,
    super::palette_modules::MODULE,
    crate::canvas::text::commands::MODULE,
    super::space_center::MODULE,
    super::context_menu::MODULE,
    crate::app::close::MODULE,
    crate::app::open::MODULE,
    super::start::MODULE,
    super::settings::MODULE,
    super::url::MODULE,
    super::table_viewer::MODULE,
    super::compare_fonts::MODULE,
    super::compare_family::MODULE,
    super::overlays::MODULE,
    super::text_preview::MODULE,
    crate::canvas::hinting::commands::MODULE,
    crate::dialogs::actions::SHELL,
    crate::dialogs::color_palettes::MODULE,
    super::font_audit::MODULE,
    super::components::MODULE,
    crate::dialogs::glyph_picker::SHELL,
    super::font_info_masters::MODULE,
    super::compatibility::MODULE,
    crate::canvas::text::features::MODULE,
    crate::dialogs::sample_strings::MODULE,
    crate::dialogs::find_glyphs::MODULE,
    super::extensions::MODULE,
    super::project::MODULE,
    super::preview::MODULE,
];

/// The module and description of the shell command `id`.
pub fn find(id: &str) -> Option<(&'static Module, &'static UiCommand)> {
    MODULES.iter().find_map(|module| module.commands.iter().find(|c| c.id == id).map(|c| (module, c)))
}

/// Runs the shell command `command`.
pub fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let (module, _) = find(&command.id).ok_or_else(|| format!("no shell command `{}`", command.id))?;
    (module.run)(app, command)
}
