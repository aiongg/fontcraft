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
    super::reference_font::MODULE,
    super::kerning_review::MODULE,
    super::glyph_info::MODULE,
    crate::dialogs::tt::SHELL,
    super::selection::MODULE,
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
