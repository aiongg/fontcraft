//! Menu items that come from the installed modules and the Macros folder rather than from the
//! command registry (M13.6 fills [`items`]). Each item invokes a static command with parameters
//! naming the module or macro, so the registry stays fixed: a filter module runs
//! `plugin.runFilter {"id"}`, a reporter toggles `view.showReporter {"id"}`, a macro runs
//! `script.run {"macro": path}` with the Font View selection as `glyphs`.
//!
//! [`super::menus`] attaches the items of each section to its menu ([`Menu::dynamic`](super::menus::Menu::dynamic))
//! and rebuilds them when the extension revision changes; the menu bar draws them after the
//! static items ([`draw`]). Rows: `script.menu.mirrorsFolder`, `script.menu.subfolders`,
//! `script.menu.tooltip`, `view.reporterPluginsInViewMenu`, `filter.menu.pluginFiltersListed`,
//! `fontView.batch.pluginsAndScripts`.

use fontcraft_engine::Session;
use serde::Serialize;
use serde_json::{Value, json};

use crate::panels::Pending;
use crate::state::UiState;

/// A part of a menu filled from extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DynamicSection {
    /// The Script menu: the Macros folder, subfolders as submenus.
    Script,
    /// The Filter menu: installed filter modules, below a separator.
    Filter,
    /// The View menu: one Show entry per reporter module.
    ViewReporters,
}

impl DynamicSection {
    /// The section shown in the top-level menu `title`, if any.
    pub fn of_menu(title: &str) -> Option<DynamicSection> {
        match title {
            "Script" => Some(DynamicSection::Script),
            "Filter" => Some(DynamicSection::Filter),
            "View" => Some(DynamicSection::ViewReporters),
            _ => None,
        }
    }
}

/// One menu item made from a module or a macro.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DynamicItem {
    /// The static command the item runs.
    pub command: &'static str,
    /// The parameters it runs the command with.
    pub params: Value,
    pub label: String,
    /// Submenu path below the top-level menu (Macros subfolders).
    pub path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    pub enabled: bool,
    /// A check mark (a shown reporter); `None`: no check box.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

/// The items of `section` for the session's extensions; `state` (when given) supplies the
/// selection that becomes `glyphs` and which reporters are shown.
pub fn items(session: &Session, state: Option<&UiState>, section: DynamicSection) -> Vec<DynamicItem> {
    let _ = (session, state, section);
    // M13.6.
    Vec::new()
}

/// Draws `items` at the end of a menu, with a separator before them; a click queues the item's
/// command with its parameters.
pub fn draw(ui: &mut egui::Ui, items: &[DynamicItem], pending: &mut Pending) {
    if items.is_empty() {
        return;
    }
    ui.separator();
    // M13.6: items with a `path` go into submenus.
    for item in items {
        let label = match item.checked {
            Some(true) => format!("✓ {}", item.label),
            _ => item.label.clone(),
        };
        let response = ui.add_enabled(item.enabled, egui::Button::new(label));
        let response = match &item.tooltip {
            Some(tip) => response.on_hover_text(tip),
            None => response,
        };
        if response.clicked() {
            let params = if item.params.is_null() { json!({}) } else { item.params.clone() };
            pending.push((item.command, params));
            ui.close();
        }
    }
}
