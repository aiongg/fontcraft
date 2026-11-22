//! The shell commands of Font View's sidebar, its glyph inspector and the Selection Info panel. View state only;
//! the filters and the glyphs change through engine commands.
//!
//! The selected sidebar entries narrow Font View (`FontViewState::entries`); the engine combines them: categories add
//! up, scripts and languages add up, filters narrow each other, and sections narrow each other. Selected glyphs the
//! new view hides are deselected, so batch commands never act on glyphs the view does not show.

use std::collections::HashSet;

use craft::behavior::dock::PanelState;
use craft::ui::runtime::commands::CommandRef;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::{self, PanelId};
use crate::app::FontCraftApp;
use crate::app::sidebar::{MAX_ENTRIES, MAX_OPEN_STATES, entries_of, is_all, path_of, path_value};

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "fontView.selectSidebarEntries",
        label: "Select Sidebar Entries",
        menu: &[],
        shortcut: None,
        params: "{entries: [path], mode?: \"replace\" (default) | \"toggle\" | \"range\"} select the sidebar entries that filter Font View; a path is text such as all, categories/Letter/Uppercase, languages/latin/de or filters/Folder/Name, or a list of names. replace: these entries only; toggle: add each entry not selected, remove each selected one; range: one entry, and every entry of its section from the last one selected there to it. Selecting all shows every glyph. Selected glyphs the new view hides are deselected",
    },
    UiCommand {
        id: "fontView.setSidebarEntryOpen",
        label: "Open Sidebar Entry",
        menu: &[],
        shortcut: None,
        params: "{entry: path, open: bool} show or hide the entries inside a sidebar section, group or folder",
    },
    UiCommand {
        id: "fontView.toggleInspector",
        label: "Show / Hide Glyph Inspector",
        menu: &[],
        shortcut: None,
        params: "{visible?: bool (omitted: toggle)} show or hide the glyph inspector's fields at the foot of the sidebar",
    },
    UiCommand {
        id: "fontView.toggleInspectorDetails",
        label: "Show / Hide Extra Glyph Properties",
        menu: &[],
        shortcut: None,
        params: "{expanded?: bool (omitted: toggle)} show or hide the inspector rows for production name, script, category and subcategory",
    },
    UiCommand {
        id: "fontView.toggleMetricsOrientation",
        label: "Horizontal / Vertical Metrics",
        menu: &[],
        shortcut: None,
        params: "{orientation?: \"horizontal\" | \"vertical\" (omitted: switch)} show the horizontal metric fields of the glyph inspector (LSB, RSB, width) or the vertical ones (TSB, BSB, vertical origin, vertical width)",
    },
    UiCommand {
        id: "edit.infoForSelection",
        label: "Info for Selection",
        menu: &["Edit"],
        shortcut: Some("Cmd+Alt+I"),
        params: "{open?: bool (default true)} show the Selection Info panel on the selected glyphs (without Font View: the shown glyph); false hides it. In Edit View with one global guide selected alone, open the guide's scope rules instead",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    let flag = |key: &str| -> Result<Option<bool>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(format!("`{key}` must be true or false")),
        }
    };
    let side = &mut app.view.sidebar;
    match command.id.as_str() {
        "fontView.selectSidebarEntries" => select_entries(app, p),
        "fontView.setSidebarEntryOpen" => {
            let entry = p.get("entry").and_then(path_of).ok_or("`entry` must be a sidebar path")?;
            let open = flag("open")?.ok_or("`open` (true or false) is required")?;
            if side.open.len() >= MAX_OPEN_STATES && !side.open.contains_key(&entry) {
                side.open.clear();
            }
            side.open.insert(entry, open);
            Ok(())
        }
        "fontView.toggleInspector" => {
            let visible = flag("visible")?.unwrap_or(side.inspector_hidden);
            side.inspector_hidden = !visible;
            Ok(())
        }
        "fontView.toggleInspectorDetails" => {
            side.inspector_details = flag("expanded")?.unwrap_or(!side.inspector_details);
            Ok(())
        }
        "fontView.toggleMetricsOrientation" => {
            side.inspector_vertical = match p.get("orientation") {
                None | Some(Value::Null) => !side.inspector_vertical,
                Some(Value::String(s)) if s == "horizontal" => false,
                Some(Value::String(s)) if s == "vertical" => true,
                Some(_) => return Err("`orientation` must be \"horizontal\" or \"vertical\"".into()),
            };
            Ok(())
        }
        "edit.infoForSelection" => {
            let open = flag("open")?.unwrap_or(true);
            if open && let Some(given) = global_guide_scope(app) {
                let spec = crate::dialogs::find(crate::dialogs::guide_scope::DIALOG).ok_or("the guide scope dialog is missing")?;
                return crate::dialogs::form::open(app, &spec, &given).map(|_| ());
            }
            let shown = app.view.workspace.layout.panel_state(&PanelId::SelectionInfo) != PanelState::Hidden;
            if open != shown {
                dock::toggle(&mut app.view.workspace, PanelId::SelectionInfo);
            }
            Ok(())
        }
        other => Err(format!("no sidebar command `{other}`")),
    }
}

/// With Edit View showing and one global guide selected alone on the canvas: the guide's address and its stored
/// filter, which the guide scope dialog opens with (`guides.globalScopeFilter`).
fn global_guide_scope(app: &FontCraftApp) -> Option<Map<String, Value>> {
    if app.view.tabs.font_view_shows() {
        return None;
    }
    let selection = &app.tools.selection;
    let mut guides = selection.guides.iter();
    let (&r, None) = (guides.next()?, guides.next()) else { return None };
    if r.scope != fontcraft_tools::selection::GuideScope::Global || !selection.points.is_empty() || !selection.shapes.is_empty() {
        return None;
    }
    let font = app.session.font().ok()?;
    let scene = crate::canvas::tools::scene(font, &app.view, app.tools.focus.as_ref())?;
    let g = fontcraft_tools::guide_drag::guide(&scene, r)?;
    let mut given = scene.target();
    given.insert("guide".into(), json!({"scope": r.scope.as_str(), "index": r.index}));
    given.insert("filter".into(), json!(g.filter.clone().unwrap_or_default()));
    Some(given)
}

fn select_entries(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let list = p.get("entries").and_then(Value::as_array).ok_or("`entries` must be a list of sidebar paths")?;
    if list.len() > MAX_ENTRIES {
        return Err(format!("`entries` holds {} paths; at most {MAX_ENTRIES} are allowed", list.len()));
    }
    let given: Vec<Vec<String>> = list
        .iter()
        .map(|v| path_of(v).ok_or_else(|| format!("`{v}` is not a sidebar path: text such as categories/Letter, or a list of names")))
        .collect::<Result<_, _>>()?;
    let current = entries_of(app.view.font_view.entries.as_ref());
    let next = match p.get("mode").and_then(Value::as_str).unwrap_or("replace") {
        "replace" => given.clone(),
        "toggle" => {
            let mut next = current;
            for path in &given {
                match next.iter().position(|c| c == path) {
                    Some(i) => {
                        next.remove(i);
                    }
                    None => next.push(path.clone()),
                }
            }
            next
        }
        "range" => {
            let [clicked] = given.as_slice() else { return Err("`range` takes one entry: the one Shift-clicked".into()) };
            range(app, &current, clicked)
        }
        other => return Err(format!("`mode` must be \"replace\", \"toggle\" or \"range\", got \"{other}\"")),
    };
    // All is the absence of a selection: choosing it drops the rest.
    let mut next = if given.iter().any(|p| is_all(p)) { Vec::new() } else { next };
    next.retain(|p| !is_all(p));
    let mut seen = HashSet::new();
    next.retain(|p| seen.insert(p.clone()));
    if next.len() > MAX_ENTRIES {
        return Err(format!("at most {MAX_ENTRIES} sidebar entries can be selected at once"));
    }
    apply(app, next)
}

/// Shift-click on `clicked`: the entries of its section from the last one selected there to it, in the order the
/// sidebar lists them, with the other sections' entries kept. Without a selected entry in that section, the clicked
/// one is added.
fn range(app: &FontCraftApp, current: &[Vec<String>], clicked: &[String]) -> Vec<Vec<String>> {
    let section = clicked.first().cloned().unwrap_or_default();
    let mut next = current.to_vec();
    let Some(anchor) = current.iter().rev().find(|c| c.first() == Some(&section)).cloned() else {
        next.push(clicked.to_vec());
        return next;
    };
    let order = app.models.sidebar.section_order(&section);
    let (Some(a), Some(b)) = (order.iter().position(|p| *p == anchor), order.iter().position(|p| p == clicked)) else {
        next.push(clicked.to_vec());
        return next;
    };
    let span = order.get(a.min(b)..=a.max(b)).unwrap_or_default();
    next.retain(|c| c.first() != Some(&section));
    next.extend(span.iter().filter(|p| **p != anchor).cloned());
    // The anchor goes last, so it stays the anchor for the next Shift-click.
    next.push(anchor);
    next
}

/// Makes `entries` Font View's sidebar selection once the engine has accepted the query they make, and deselects
/// the glyphs the view no longer shows. A refused query changes nothing.
fn apply(app: &mut FontCraftApp, entries: Vec<Vec<String>>) -> Result<(), String> {
    let value = (!entries.is_empty()).then(|| Value::Array(entries.iter().map(|p| path_value(p)).collect()));
    let mut query = Map::new();
    if let Some(v) = &value {
        query.insert("entries".into(), v.clone());
    }
    if let Some(search) = app.view.font_view.search.to_json() {
        query.insert("search".into(), search);
    }
    if !query.is_empty() {
        let master = app.session.font().ok().and_then(|f| crate::shell::masters::current_master(f, &app.view.layers)).map(|m| m.id.clone());
        if let Some(master) = master {
            query.insert("master".into(), json!(master));
        }
        let result = app.session.query("fontView.queryGlyphs", &Value::Object(query)).map_err(|e| e.to_string())?;
        let shown: HashSet<&str> =
            result.get("glyphs").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let fv = &mut app.view.font_view;
        fv.selected.retain(|n| shown.contains(n.as_str()));
        if fv.anchor.as_ref().is_some_and(|a| !shown.contains(a.as_str())) {
            fv.anchor = None;
        }
    }
    app.view.font_view.entries = value;
    Ok(())
}
