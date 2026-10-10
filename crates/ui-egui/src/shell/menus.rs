//! The menu bar, built from the command registry: a command with a menu path appears in the menu
//! by itself. Commands that only change view and window state belong to the shell and are listed
//! in [`UI_COMMANDS`]; everything else is an engine command.
//!
//! The tree is built when the document or the selection changes ([`MenuCache`]), not every frame.

use fontcraft_engine::{Session, command_specs};
use serde::Serialize;

use super::UI_COMMANDS;
use crate::state::UiState;

/// Top-level menus, in bar order. A command whose first menu element is not listed here is
/// appended after them.
pub const MENU_ORDER: &[&str] = &["File", "Edit", "Glyph", "Path", "Filter", "View", "Script", "Window", "Help"];

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
    for (id, label, menu, shortcut, _) in UI_COMMANDS.iter() {
        let path = menu.get(1..).unwrap_or_default().to_vec();
        add(menu, MenuItem { command: id, label, path, shortcut, enabled: true, disabled_reason: None });
    }
    menus.retain(|m| !m.items.is_empty());
    menus
}

/// The menu tree of the last frame, and what it was built from.
#[derive(Default)]
pub struct MenuCache {
    key: Option<u64>,
    tree: Vec<Menu>,
}

impl MenuCache {
    /// The menu tree, rebuilt only when the document, the interaction or the selection changed.
    pub fn tree(&mut self, session: &Session, state: &UiState) -> &[Menu] {
        let key = cache_key(session, state);
        if self.key != Some(key) {
            self.tree = menu_tree(session);
            self.key = Some(key);
        }
        &self.tree
    }
}

/// What a menu item's enablement can depend on: the active document and its revision, the
/// number of open documents, a running interaction, and the selections.
fn cache_key(session: &Session, state: &UiState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    session.active().map(|d| (d.id, d.revision)).hash(&mut h);
    session.documents().len().hash(&mut h);
    session.interaction().hash(&mut h);
    session.batch_is_idle().hash(&mut h);
    state.active_tab.hash(&mut h);
    state.selected_glyphs.hash(&mut h);
    if let Some(tab) = state.tab() {
        tab.edited.hash(&mut h);
        tab.glyphs.hash(&mut h);
        serde_json::to_string(&tab.selection).unwrap_or_default().hash(&mut h);
    }
    h.finish()
}

/// The items of `menu` grouped into submenus by their `path`, for drawing.
pub fn submenus(items: &[MenuItem]) -> Vec<(Option<&'static str>, Vec<&MenuItem>)> {
    let mut groups: Vec<(Option<&'static str>, Vec<&MenuItem>)> = Vec::new();
    for item in items {
        let key = item.path.first().copied();
        match groups.iter_mut().find(|(k, _)| *k == key && key.is_some()) {
            Some((_, list)) => list.push(item),
            None => groups.push((key, vec![item])),
        }
    }
    groups
}
