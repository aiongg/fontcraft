//! The menu bar, built from the command registry: a command with a menu path appears in the menu
//! by itself. Commands that only change view and window state belong to the shell and are listed
//! in [`UI_COMMANDS`]; everything else is an engine command.

use fontcraft_engine::{Session, command_specs};
use serde::Serialize;

/// Top-level menus, in bar order. A command whose first menu element is not listed here is
/// appended after them.
pub const MENU_ORDER: &[&str] = &["File", "Edit", "Glyph", "Path", "Filter", "View", "Script", "Window", "Help"];

/// Shell-only commands: `(id, label, menu path, shortcut, params doc)`.
pub const UI_COMMANDS: &[(&str, &str, &[&str], &str, &str)] = &[
    ("fontView.show", "Show Font View", &[], "Cmd+Alt+1", "{} leave Edit View and show the glyph grid"),
    ("fontView.setSelection", "Set Glyph Selection", &[], "", "{glyphs: [names]} select these glyphs in Font View (unknown names are an error)"),
    (
        "view.editSelectedGlyphs",
        "Edit Selected Glyphs",
        &[],
        "Cmd+Down",
        "{glyph?: name} open the glyph (default: the first selected one) in Edit View",
    ),
    ("view.zoomIn", "Zoom In", &["View"], "Cmd+Plus", "{} zoom Edit View in by one step"),
    ("view.zoomOut", "Zoom Out", &["View"], "Cmd+Minus", "{} zoom Edit View out by one step"),
    ("view.zoomToActualSize", "Zoom to Actual Size", &["View"], "Cmd+Alt+0", "{} one font unit per screen point"),
    ("view.showNodes", "Show Nodes", &["View"], "", "{show?: bool} show or hide nodes and handles in Edit View (omitted: toggle)"),
    ("view.showMetrics", "Show Metrics", &["View"], "Cmd+Shift+M", "{show?: bool} show or hide the metric lines in Edit View (omitted: toggle)"),
    ("window.palette", "Palette", &["Window"], "Cmd+Alt+P", "{show?: bool} show or hide the Palette sidebar (omitted: toggle)"),
];

pub fn is_ui_command(id: &str) -> bool {
    UI_COMMANDS.iter().any(|c| c.0 == id)
}

/// One entry of a menu.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuItem {
    pub command: &'static str,
    pub label: &'static str,
    /// Submenu path below the top-level menu.
    pub path: Vec<&'static str>,
    pub shortcut: &'static str,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Menu {
    pub title: &'static str,
    pub items: Vec<MenuItem>,
}

/// The whole menu tree for the session's current state (`ui.menu.list` on the control channel).
pub fn menu_tree(session: &Session) -> Vec<Menu> {
    let mut menus: Vec<Menu> = MENU_ORDER.iter().map(|title| Menu { title, items: Vec::new() }).collect();
    let mut add = |menu: &'static [&'static str], item: MenuItem| {
        let Some((title, _)) = menu.split_first() else { return };
        match menus.iter_mut().find(|m| m.title == *title) {
            Some(m) => m.items.push(item),
            None => menus.push(Menu { title, items: vec![item] }),
        }
    };
    for c in command_specs() {
        let info = c.info(session);
        let path = c.menu.get(1..).unwrap_or_default().to_vec();
        add(
            c.menu,
            MenuItem {
                command: c.id,
                label: c.label,
                path,
                shortcut: c.shortcut.unwrap_or(""),
                enabled: info.enabled,
                disabled_reason: info.disabled_reason,
            },
        );
    }
    for (id, label, menu, shortcut, _) in UI_COMMANDS {
        let path = menu.get(1..).unwrap_or_default().to_vec();
        add(menu, MenuItem { command: id, label, path, shortcut, enabled: true, disabled_reason: None });
    }
    menus.retain(|m| !m.items.is_empty());
    menus
}
