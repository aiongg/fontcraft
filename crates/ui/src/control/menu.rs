//! `ui.menu.list` and `ui.menu.invoke`: the menu bar the window shows, read and clicked.

use craft::menu::{ItemKind, Node};
use serde_json::{Value, json};

use super::Method;
use super::engine::command_ref;
use crate::app::FontCraftApp;
use crate::app::menu_alternates::alternate_label;

pub const METHODS: &[(&str, Method)] = &[("ui.menu.list", list), ("ui.menu.invoke", invoke)];

/// `{}` → `[{title, items: [{command, label, path, shortcut, enabled, disabled_reason?, checked?, alternate?,
/// alternateOf?}]}]`: every menu of the bar with its items, submenus flattened into `path`. A row with an Option
/// alternate names it in `alternate`, and the alternate follows it as an item of its own with `alternateOf`: what the
/// row reads and runs while Alt is held.
fn list(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    let menus: Vec<Value> = app
        .catalog
        .menu
        .menus
        .iter()
        .map(|menu| {
            let mut items = Vec::new();
            flatten(app, &menu.children, &mut Vec::new(), &mut items);
            json!({"title": menu.title, "items": items})
        })
        .collect();
    Ok(Value::Array(menus))
}

/// Most submenu levels read; the menus are built from command paths, which are short.
const MAX_DEPTH: usize = 8;

fn flatten(app: &FontCraftApp, nodes: &[Node], path: &mut Vec<String>, out: &mut Vec<Value>) {
    if path.len() > MAX_DEPTH {
        return;
    }
    for node in nodes {
        match node {
            Node::Item(item) => {
                let id = item.command.id.as_str();
                let mut entry = menu_entry(app, id, &item.label, path, item.enabled, item.tooltip.as_deref());
                if let ItemKind::Check(on) | ItemKind::Radio(on) = item.kind {
                    super::set(&mut entry, "checked", json!(on));
                }
                let alternate = item.alternate.as_ref().map(|a| a.command.id.as_str());
                if let Some(alt) = alternate {
                    super::set(&mut entry, "alternate", json!(alt));
                }
                out.push(entry);
                if let Some(alt) = alternate {
                    let label = alternate_label(alt).unwrap_or(alt);
                    let mut held = menu_entry(app, alt, label, path, item.enabled, item.tooltip.as_deref());
                    super::set(&mut held, "alternateOf", json!(id));
                    out.push(held);
                }
            }
            Node::Submenu { label, children, .. } => {
                path.push(label.clone());
                flatten(app, children, path, out);
                path.pop();
            }
            _ => {}
        }
    }
}

/// One item of the `ui.menu.list` reply: the command `id` under `label` at `path`, greyed out unless `enabled`; a
/// greyed item says why (the engine's reason, else the row's `tooltip`).
fn menu_entry(app: &FontCraftApp, id: &str, label: &str, path: &[String], enabled: bool, tooltip: Option<&str>) -> Value {
    let spec = fontcraft_engine::find_command(id);
    let built_in = spec.and_then(|s| s.shortcut).or_else(|| crate::shell::commands::find(id).and_then(|(_, c)| c.shortcut));
    let shortcut = app.catalog.shortcut(id, built_in);
    let check = spec.map(|s| (s.enabled)(&app.session));
    let enabled = enabled && !matches!(check, Some(Err(_)));
    let mut entry = json!({"command": id, "label": label, "path": path, "shortcut": shortcut.unwrap_or_default(), "enabled": enabled});
    if !enabled {
        // A row the engine allows is greyed out by the selection, which put why in its tooltip.
        let reason = match check {
            Some(Err(reason)) => Some(reason),
            _ => tooltip.map(str::to_owned),
        };
        if let Some(reason) = reason {
            super::set(&mut entry, "disabled_reason", json!(reason));
        }
    }
    entry
}

/// Whether the menu bar has an item running `id`.
fn in_menu(nodes: &[Node], id: &str, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && nodes.iter().any(|node| match node {
            Node::Item(item) => item.command.id == id || item.alternate.as_ref().is_some_and(|a| a.command.id == id),
            Node::Submenu { children, .. } => in_menu(children, id, depth + 1),
            _ => false,
        })
}

/// `{command, params?}`: runs a menu item as a click on it does ([`FontCraftApp::fired`]), and returns the command's
/// result. A click that opens a dialog instead of running returns `{dialog: id}`; one that asks a question of its own
/// (unsaved changes, a glyph picker) or edits text returns null. A command that takes its path from the file picker
/// opens the picker and returns `{picker: true}`. A row's Option alternate is invoked by its own id, as a click with
/// Alt held runs it. An id in no menu is an error.
fn invoke(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.menu.invoke";
    let command = command_ref(M, params)?;
    if !app.catalog.menu.menus.iter().any(|menu| in_menu(&menu.children, &command.id, 0)) {
        return Err(format!("{M}: `{}` is not a menu item", command.id));
    }
    // What views posted before the click applies first, as the click's frame would.
    app.drain();
    let before = app.dialogs.open.as_ref().map(|d| d.id.clone());
    let result = match app.fired(command) {
        Ok(Some(msg)) => return app.run_now(msg, true),
        Ok(None) => match app.dialogs.open.as_ref().map(|d| d.id.clone()) {
            Some(id) if before.as_ref() != Some(&id) => Ok(json!({ "dialog": id })),
            _ => Ok(Value::Null),
        },
        Err(error) => {
            app.status = error.clone();
            Err(error)
        }
    };
    app.drain();
    result
}
