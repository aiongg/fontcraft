//! The menu bar, built from the command registry: a command with a menu path appears in the menu
//! by itself. Commands that only change view and window state belong to the shell and are listed
//! in [`UI_COMMANDS`]; everything else is an engine command.
//!
//! An item is enabled when its command's `enabled` says so and, for a command that takes
//! targets, the Edit View selection supplies them (`tools_glue::selection_supplies`): with no
//! node selected, a command that acts on nodes is greyed out. Every on/off switch ([`SWITCHES`])
//! shows a check mark while on ([`checked`]). The tree is built when the document, the selection
//! or a switch changes ([`MenuCache`]), not every frame.

use fontcraft_engine::{Session, command_specs};
use serde::Serialize;

use super::UI_COMMANDS;
use crate::state::UiState;
use crate::tools_glue;

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
    /// A switch's state, drawn as a check mark while on; `None`: not a switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Menu {
    pub title: &'static str,
    pub items: Vec<MenuItem>,
    /// Items made from modules and macros ([`super::menus_dynamic`]), drawn after `items`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dynamic: Vec<super::menus_dynamic::DynamicItem>,
}

/// The whole menu tree for the session's current state (`ui.menu.list` on the control channel),
/// without the selection: only the commands' own `enabled` greys items out.
pub fn menu_tree(session: &Session) -> Vec<Menu> {
    build(session, None)
}

/// The menu tree the window shows: [`menu_tree`] with the items greyed out whose targets the
/// selection does not supply.
pub fn menu_tree_for(session: &Session, state: &UiState) -> Vec<Menu> {
    build(session, Some(state))
}

fn build(session: &Session, state: Option<&UiState>) -> Vec<Menu> {
    let mut menus: Vec<Menu> = MENU_ORDER.iter().map(|title| Menu { title, items: Vec::new(), dynamic: Vec::new() }).collect();
    let mut add = |menu: &'static [&'static str], item: MenuItem| {
        let Some((title, _)) = menu.split_first() else { return };
        match menus.iter_mut().find(|m| m.title == *title) {
            Some(m) => m.items.push(item),
            None => menus.push(Menu { title, items: vec![item], dynamic: Vec::new() }),
        }
    };
    for c in command_specs() {
        if c.menu.is_empty() {
            continue;
        }
        let info = c.info(session);
        let (enabled, disabled_reason) = match (info.enabled, state) {
            (true, Some(state)) => match tools_glue::selection_supplies(session, state, c) {
                Ok(()) => (true, None),
                Err(why) => (false, Some(why)),
            },
            _ => (info.enabled, info.disabled_reason),
        };
        let path = c.menu.get(1..).unwrap_or_default().to_vec();
        let checked = state.and_then(|s| checked(session, s, c.id));
        add(c.menu, MenuItem { command: c.id, label: c.label, path, shortcut: c.shortcut.unwrap_or(""), enabled, disabled_reason, checked });
    }
    for (id, label, menu, shortcut, _) in UI_COMMANDS.iter() {
        let path = menu.get(1..).unwrap_or_default().to_vec();
        let checked = state.and_then(|s| checked(session, s, id));
        add(menu, MenuItem { command: id, label, path, shortcut, enabled: true, disabled_reason: None, checked });
    }
    for m in &mut menus {
        m.dynamic =
            super::menus_dynamic::DynamicSection::of_menu(m.title).map(|s| super::menus_dynamic::items(session, state, s)).unwrap_or_default();
    }
    menus.retain(|m| !m.items.is_empty() || !m.dynamic.is_empty());
    menus
}

/// The menu commands that flip an on/off switch: each shows a check mark while its switch is on.
/// A command that only opens a window (its menu item never closes it) is not a switch.
pub const SWITCHES: &[&str] = &[
    "audit.toggleLiveAudit",
    "view.showNodes",
    "view.showNodesInForeground",
    "view.showNodesInBackground",
    "view.showExtraNodes",
    "view.showMetrics",
    "view.showMetricsNames",
    "view.showInfo",
    "view.showBackground",
    "view.showGuides",
    "view.showBoundingBox",
    "view.showAnnotations",
    "view.showAnchors",
    "view.fillPreview",
    "view.showMasterCompatibility",
    "view.showGroupMembers",
    "view.showHints",
    "view.showImage",
    "view.showTunniLines",
    "view.showCurveTension",
    "view.showCurveLength",
    "view.showIndexes",
    "view.showDetailsAcrossGlyphs",
    "view.lockOutline",
    "view.showMetricsBeam",
    "view.suggestHandles",
    "view.suggestLines",
    "view.suggestDistance",
    "view.suggestStems",
    "view.showStemWidths",
    "view.showCounterWidths",
    "view.showCornerAngles",
    "view.showQuickMeasurement",
    "view.showMeasurementLine",
    "view.setSpacingCutOff",
    "edit.keepLayerSelectionInSync",
    "path.editBackground",
    "path.togglePowerNudge",
    "window.palette",
    "window.previewPanel",
    "audit.showPanel",
];

/// The state of the switch the command `id` flips, for its check mark; `None` for a command
/// that is not one of the [`SWITCHES`]. Path › Edit Background reads the showing tab (off with
/// no tab showing); Power Nudge reads the user setting.
pub fn checked(session: &Session, state: &UiState, id: &str) -> Option<bool> {
    let mut display = state.display;
    if let Some(on) = super::view::flag(&mut display, id) {
        return Some(*on);
    }
    let aids = &state.panels.edit_aids;
    let mut suggest = aids.suggest;
    if let Some(on) = suggest.get_mut(id) {
        return Some(*on);
    }
    let meter = &state.panels.autometer;
    Some(match id {
        "view.showMasterCompatibility" => state.display.master_compatibility,
        "edit.keepLayerSelectionInSync" => state.display.keep_layer_selection_in_sync,
        "view.showGroupMembers" => state.display.group_members,
        "view.showHints" => state.display.hints,
        "view.showImage" => state.display.show_image,
        "view.showTunniLines" => aids.tunni_lines,
        "view.showCurveTension" => aids.curve_tension,
        "view.showCurveLength" => aids.curve_length,
        "view.showIndexes" => aids.indexes.any(),
        "view.showDetailsAcrossGlyphs" => aids.details_across_glyphs,
        "view.lockOutline" => aids.lock_outline,
        "view.showMetricsBeam" => aids.beam,
        "view.showStemWidths" => meter.stem_widths,
        "view.showCounterWidths" => meter.counter_widths,
        "view.showCornerAngles" => meter.corner_angles,
        "view.showQuickMeasurement" => meter.quick_measurement,
        "view.showMeasurementLine" => state.panels.measurement.on,
        "view.setSpacingCutOff" => state.panels.space_center.cut_off.on,
        "path.editBackground" => state.tab().is_some_and(|t| t.edit_background),
        "path.togglePowerNudge" => session.settings().get_bool(fontcraft_tools::drag_aids::POWER_NUDGE_SETTING).unwrap_or(false),
        "window.palette" => state.show_palette,
        "window.previewPanel" => state.panels.preview.panel_open,
        "audit.showPanel" => state.panels.font_audit.open,
        "audit.toggleLiveAudit" => state.audit.live,
        _ => return None,
    })
}

/// The menu tree of the last frame, and what it was built from.
#[derive(Default)]
pub struct MenuCache {
    key: Option<u64>,
    tree: Vec<Menu>,
    builds: u64,
}

impl MenuCache {
    /// The menu tree, rebuilt only when the document, the interaction, the selection or a switch
    /// changed.
    pub fn tree(&mut self, session: &Session, state: &UiState) -> &[Menu] {
        let key = cache_key(session, state);
        if self.key != Some(key) {
            self.tree = menu_tree_for(session, state);
            self.key = Some(key);
            self.builds = self.builds.saturating_add(1);
        }
        &self.tree
    }

    /// How many times the tree has been built.
    pub fn builds(&self) -> u64 {
        self.builds
    }
}

/// What a menu item's enablement and check mark can depend on: the active document and its
/// revision, the number of open documents, a running interaction, the selections, the path focus
/// and the state of every switch.
fn cache_key(session: &Session, state: &UiState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    session.active().map(|d| (d.id, d.revision)).hash(&mut h);
    session.documents().len().hash(&mut h);
    session.interaction().hash(&mut h);
    session.batch_is_idle().hash(&mut h);
    session.extensions().revision().hash(&mut h);
    state.active_tab.hash(&mut h);
    state.selected_glyphs.hash(&mut h);
    for id in SWITCHES {
        checked(session, state, id).hash(&mut h);
    }
    super::menus_dynamic::cache_key(state).hash(&mut h);
    if let Some(tab) = state.tab() {
        tab.text.revision().hash(&mut h);
        tab.focus.hash(&mut h);
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
