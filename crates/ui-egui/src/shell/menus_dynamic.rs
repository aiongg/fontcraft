//! Menu items that come from the installed modules and the Macros folder rather than from the
//! command registry. Each item invokes a static command with parameters naming the module or
//! macro, so the registry stays fixed:
//!
//! | Menu | Items | Command |
//! |---|---|---|
//! | Script | one per macro of the Macros folder: folders first, then titles in natural order; subfolders as submenus; the description as tooltip; unreadable files greyed out | `script.run {"macro": path, glyphs?, layers?}` |
//! | Filter | one per filter module, by name, below a separator | `plugin.runFilter {"id", …targets}` with the module's defaults |
//! | View | one Show entry per reporter module, by name, checked while shown | `view.showReporter {"id", "on"}` |
//!
//! The selection becomes the targets (`fontView.batch.pluginsAndScripts`): Font View's selected
//! glyphs as `glyphs` (filters run on Font View's master), or in Edit View the edited glyph and
//! layer with the selected nodes ([`filter_targets`]).
//!
//! [`super::menus`] attaches the items of each section to its menu ([`Menu::dynamic`](super::menus::Menu::dynamic))
//! and rebuilds them when the extension revision, the selection or [`cache_key`] changes; the
//! menu bar draws them after the static items ([`draw`]). Rows: `script.menu.mirrorsFolder`,
//! `script.menu.subfolders`, `script.menu.tooltip`, `view.reporterPluginsInViewMenu`,
//! `filter.menu.pluginFiltersListed`, `fontView.batch.pluginsAndScripts`.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use fontcraft_engine::Session;
use fontcraft_engine::extensions::MAX_MACRO_FOLDER_DEPTH;
use fontcraft_engine::macros::natural_cmp;
use fontcraft_engine::plugins::{Installed, Kind};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::panels::Pending;
use crate::state::UiState;
use crate::tools_glue;

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
    match section {
        DynamicSection::Script => script_items(session, state),
        DynamicSection::Filter => filter_items(session, state),
        DynamicSection::ViewReporters => reporter_items(session, state),
    }
}

/// What the items depend on that the menu cache does not hash otherwise: the shown reporters,
/// the edited glyph, master, layers and side, the text selection, and Font View's master.
pub fn cache_key(state: &UiState) -> u64 {
    let mut h = DefaultHasher::new();
    state.panels.extensions.reporters.hash(&mut h);
    state.font_view.master.hash(&mut h);
    if let Some(tab) = state.tab() {
        tab.edited_glyph().hash(&mut h);
        tab.master.hash(&mut h);
        tab.layers.hash(&mut h);
        tab.edit_background.hash(&mut h);
        tab.text_mode.hash(&mut h);
        let selection = tab.text.selection();
        (selection.start, selection.end).hash(&mut h);
    }
    h.finish()
}

/// The targets a filter module's item hands `plugin.runFilter`: in Edit View the edited glyph and
/// layer with the selected nodes (`glyph`, `layer`, `points`), in text mode the selected glyphs in
/// the tab's master (`glyphs`, `layer`), in Font View the selected glyphs in Font View's master
/// (`fontView.batch.pluginsAndScripts`). `None` when there is nothing to act on.
pub fn filter_targets(session: &Session, state: &UiState) -> Option<Map<String, Value>> {
    if state.tab().is_some() {
        let spec = fontcraft_engine::find_command(RUN_FILTER)?;
        return tools_glue::selection_targets(session, state, spec);
    }
    if state.selected_glyphs.is_empty() {
        return None;
    }
    let font = session.font().ok()?;
    let mut m = Map::new();
    m.insert("glyphs".into(), json!(state.selected_glyphs));
    if let Some(master) = crate::shell::font_view::cache::resolve_master(font, state.font_view.master.as_deref()) {
        m.insert("layer".into(), json!(master));
    }
    Some(m)
}

/// The built-in arguments a macro's item hands `script.run`: the glyphs [`filter_targets`] names
/// as `glyphs`, and in Edit View their layer as `layers`. Font View passes only its selection.
fn macro_targets(session: &Session, state: &UiState) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(t) = filter_targets(session, state) else { return out };
    match (t.get("glyph"), t.get("glyphs")) {
        (Some(g), _) => {
            out.insert("glyphs".into(), json!([g]));
        }
        (None, Some(gs)) => {
            out.insert("glyphs".into(), gs.clone());
        }
        (None, None) => {}
    }
    if state.tab().is_some()
        && let Some(layer) = t.get("layer")
    {
        out.insert("layers".into(), json!([layer]));
    }
    out
}

const RUN_FILTER: &str = "plugin.runFilter";

/// The Script menu: one item per macro of the Macros folder in the listing's order (folders
/// first, then titles in natural order), subfolders as submenus, the description as tooltip.
/// A file that cannot be read is listed greyed out with the reason.
fn script_items(session: &Session, state: Option<&UiState>) -> Vec<DynamicItem> {
    let targets = state.map(|s| macro_targets(session, s)).unwrap_or_default();
    session
        .extensions()
        .macros
        .iter()
        .map(|e| {
            let mut path: Vec<String> = e.path.split('/').map(str::to_string).collect();
            path.pop();
            path.truncate(MAX_MACRO_FOLDER_DEPTH);
            let mut params = targets.clone();
            params.insert("macro".into(), json!(e.path));
            let tooltip = match &e.error {
                Some(error) => Some(format!("This macro cannot be read: {error}")),
                None => Some(e.description.clone()).filter(|d| !d.trim().is_empty()),
            };
            DynamicItem {
                command: "script.run",
                params: Value::Object(params),
                label: e.title.clone(),
                path,
                tooltip,
                enabled: e.error.is_none(),
                checked: None,
            }
        })
        .collect()
}

/// The installed modules of `kind`, in the order of their names.
fn modules_of(session: &Session, kind: Kind) -> Vec<&Installed> {
    let mut list: Vec<&Installed> = session.extensions().registry.list().iter().filter(|m| m.plugin.kind() == kind).collect();
    list.sort_by(|a, b| {
        let (a, b) = (a.plugin.manifest(), b.plugin.manifest());
        natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id))
    });
    list
}

/// The module's description as a tooltip, if it has one.
fn description(m: &Installed) -> Option<String> {
    Some(m.plugin.manifest().description.clone()).filter(|d| !d.trim().is_empty())
}

/// The Filter menu: one item per filter module, run with its default parameters on what the
/// selection gives ([`filter_targets`]). A module that cannot run now (turned off, safe mode)
/// or that has nothing to act on is greyed out with the reason as tooltip.
fn filter_items(session: &Session, state: Option<&UiState>) -> Vec<DynamicItem> {
    let ext = session.extensions();
    let targets = state.and_then(|s| filter_targets(session, s));
    modules_of(session, Kind::Filter)
        .into_iter()
        .map(|m| {
            let id = m.plugin.id();
            let mut params = targets.clone().unwrap_or_default();
            params.insert("id".into(), json!(id));
            let why = match (ext.runnable(id, Kind::Filter), &targets) {
                (Err(why), _) => Some(why),
                (Ok(_), None) if state.is_some() => Some("Select glyphs or edit a glyph first".to_string()),
                _ => None,
            };
            DynamicItem {
                command: RUN_FILTER,
                params: Value::Object(params),
                label: m.plugin.manifest().name.clone(),
                path: Vec::new(),
                enabled: why.is_none(),
                tooltip: why.or_else(|| description(m)),
                checked: None,
            }
        })
        .collect()
}

/// The View menu: one Show entry per reporter module, checked while it is shown. A module that
/// cannot run now keeps its entry, so its switch can still be turned off; the tooltip says why
/// nothing is drawn.
fn reporter_items(session: &Session, state: Option<&UiState>) -> Vec<DynamicItem> {
    let ext = session.extensions();
    modules_of(session, Kind::Reporter)
        .into_iter()
        .map(|m| {
            let id = m.plugin.id();
            let shown = state.is_some_and(|s| s.panels.extensions.reporters.iter().any(|r| r == id));
            DynamicItem {
                command: "view.showReporter",
                params: json!({"id": id, "on": !shown}),
                label: format!("Show {}", m.plugin.manifest().name),
                path: Vec::new(),
                tooltip: ext.runnable(id, Kind::Reporter).err().or_else(|| description(m)),
                enabled: true,
                checked: Some(shown),
            }
        })
        .collect()
}

/// Draws `items` at the end of a menu, with a separator before them; items with a `path` go into
/// submenus. A click queues the item's command with its parameters.
pub fn draw(ui: &mut egui::Ui, items: &[DynamicItem], pending: &mut Pending) {
    if items.is_empty() {
        return;
    }
    ui.separator();
    let all: Vec<&DynamicItem> = items.iter().collect();
    draw_level(ui, &all, 0, pending);
}

/// Draws the items below the submenu `depth` levels down: a submenu for each folder at this
/// level, in the order of the first item in it, and the items with no folder left.
fn draw_level(ui: &mut egui::Ui, items: &[&DynamicItem], depth: usize, pending: &mut Pending) {
    enum Entry<'a> {
        Folder(&'a str, Vec<&'a DynamicItem>),
        Item(&'a DynamicItem),
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut folders: HashMap<&str, usize> = HashMap::new();
    for &item in items {
        match item.path.get(depth) {
            Some(folder) if depth < MAX_MACRO_FOLDER_DEPTH => match folders.get(folder.as_str()).and_then(|&at| entries.get_mut(at)) {
                Some(Entry::Folder(_, list)) => list.push(item),
                _ => {
                    folders.insert(folder.as_str(), entries.len());
                    entries.push(Entry::Folder(folder.as_str(), vec![item]));
                }
            },
            _ => entries.push(Entry::Item(item)),
        }
    }
    for entry in entries {
        match entry {
            Entry::Folder(name, list) => {
                ui.menu_button(name, |ui| draw_level(ui, &list, depth + 1, pending));
            }
            Entry::Item(item) => draw_item(ui, item, pending),
        }
    }
}

fn draw_item(ui: &mut egui::Ui, item: &DynamicItem, pending: &mut Pending) {
    let label = match item.checked {
        Some(true) => format!("✓ {}", item.label),
        _ => item.label.clone(),
    };
    let response = ui.add_enabled(item.enabled, egui::Button::new(label));
    let response = match &item.tooltip {
        Some(tip) if item.enabled => response.on_hover_text(tip),
        Some(tip) => response.on_disabled_hover_text(tip),
        None => response,
    };
    if response.clicked() {
        let params = if item.params.is_null() { json!({}) } else { item.params.clone() };
        pending.push((item.command, params));
        ui.close();
    }
}
