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
    super::tabs::MODULE,
    super::font_info::MODULE,
    super::font_info_tabs::MODULE,
    super::reference_font::MODULE,
    super::kerning_review::MODULE,
    super::glyph_info::MODULE,
    crate::dialogs::tt::SHELL,
    super::selection::MODULE,
    super::masters::MODULE,
    super::command_surfaces::MODULE,
    super::macro_editor::MODULE,
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
    crate::app::file_questions::MODULE,
    crate::app::open_report::MODULE,
    super::start::MODULE,
    super::images::MODULE,
    super::settings::MODULE,
    super::url::MODULE,
    super::table_viewer::MODULE,
    super::compare_fonts::MODULE,
    super::compare_family::MODULE,
    super::compare_in_list::MODULE,
    super::overlays::MODULE,
    super::text_preview::MODULE,
    crate::canvas::hinting::commands::MODULE,
    crate::canvas::hoi::MODULE,
    crate::dialogs::actions::SHELL,
    crate::dialogs::color_palettes::MODULE,
    super::font_audit::MODULE,
    super::components::MODULE,
    crate::dialogs::glyph_picker::SHELL,
    super::font_info_masters::MODULE,
    super::show_all_masters::MODULE,
    super::compatibility::MODULE,
    crate::canvas::text::features::MODULE,
    crate::dialogs::sample_strings::MODULE,
    crate::dialogs::find_glyphs::MODULE,
    super::extensions::MODULE,
    super::extension_manager::MODULE,
    super::project::MODULE,
    super::preview::MODULE,
    super::axis_sweep::MODULE,
    crate::dialogs::overlay_fonts::MODULE,
    super::shape_order::MODULE,
    super::bitmap_editor::MODULE,
    crate::dialogs::axis_mappings::MODULE,
    crate::dialogs::cross_axis::MODULE,
    crate::dialogs::print::MODULE,
    super::encoding::MODULE,
    super::getters::MODULE,
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

/// Keys the window takes for a shell command in one place only, or as the first of a numbered series
/// ([`super::numbered_keys`]). A command's own `shortcut` is bound everywhere, so these stay out of it; the inventory
/// check reads them through [`shown_shortcut`].
///
/// - Escape: text mode and edit mode in the edit view (`canvas::text::input`).
/// - Comma, period and X: the TrueType Instructor's keys, run through the tool (`fontcraft_tools::tt_instructor`).
/// - Up and Down: the Kerning window's list moves its lead row, and the panel selects that row and shows its pair,
///   as the commands do.
/// - Cmd+1 and Cmd+Alt+2: the first keys of the master and tab series.
pub const SCOPED_KEYS: &[(&str, &str)] = &[
    ("editView.toggleTextMode", "Escape"),
    ("hint.tt.decreasePixelSize", ","),
    ("hint.tt.increasePixelSize", "."),
    ("hint.tt.toggleDirection", "X"),
    ("kerning.selectNextPair", "Down"),
    ("kerning.selectPreviousPair", "Up"),
    ("view.selectMaster", "Cmd+1"),
    ("view.showTab", "Cmd+Alt+2"),
];

/// The shortcut people know `command` by: its own, else its key from [`SCOPED_KEYS`].
pub fn shown_shortcut(command: &UiCommand) -> Option<&'static str> {
    command.shortcut.or_else(|| SCOPED_KEYS.iter().find(|(id, _)| *id == command.id).map(|(_, key)| *key))
}

#[cfg(test)]
mod tests {
    use super::{SCOPED_KEYS, find};
    use crate::shell::numbered_keys;

    #[test]
    fn scoped_keys_name_shell_commands_without_a_global_shortcut() {
        for (id, _) in SCOPED_KEYS {
            let found = find(id).map(|(_, command)| command.shortcut);
            assert_eq!(found, Some(None), "{id}: a shell command with no shortcut of its own");
        }
        let numbered = numbered_keys::all();
        for id in ["view.selectMaster", "view.showTab"] {
            let first = numbered.iter().find(|key| key.command.id == id).map(|key| key.shortcut.as_str());
            let scoped = SCOPED_KEYS.iter().find(|(scoped, _)| *scoped == id).map(|(_, key)| *key);
            assert_eq!(scoped, first, "{id}: the first key of its numbered series");
        }
    }
}
