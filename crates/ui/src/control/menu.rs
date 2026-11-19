//! `ui.menu.list` and `ui.menu.invoke`: the menu bar the window shows, read and clicked.

use craft::menu::{ItemKind, Node};
use serde_json::{Value, json};

use super::Method;
use super::engine::command_ref;
use crate::app::FontCraftApp;

pub const METHODS: &[(&str, Method)] = &[("ui.menu.list", list), ("ui.menu.invoke", invoke)];

/// `{}` → `[{title, items: [{command, label, path, shortcut, enabled, disabled_reason?, checked?}]}]`: every menu
/// of the bar with its items, submenus flattened into `path`.
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
                let spec = fontcraft_engine::find_command(id);
                let shortcut = spec.and_then(|s| s.shortcut).or_else(|| crate::shell::commands::find(id).and_then(|(_, c)| c.shortcut));
                let mut entry = json!({
                    "command": id,
                    "label": item.label,
                    "path": path,
                    "shortcut": shortcut.unwrap_or(""),
                    "enabled": item.enabled,
                });
                if !item.enabled
                    && let Some(Err(reason)) = spec.map(|s| (s.enabled)(&app.session))
                {
                    super::set(&mut entry, "disabled_reason", json!(reason));
                }
                if let ItemKind::Check(on) | ItemKind::Radio(on) = item.kind {
                    super::set(&mut entry, "checked", json!(on));
                }
                out.push(entry);
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

/// Whether the menu bar has an item running `id`.
fn in_menu(nodes: &[Node], id: &str, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && nodes.iter().any(|node| match node {
            Node::Item(item) => item.command.id == id,
            Node::Submenu { children, .. } => in_menu(children, id, depth + 1),
            _ => false,
        })
}

/// `{command, params?}`: runs a menu item as a click on it does, and returns the command's result. A command that
/// takes its path from the file picker opens the picker and returns `{picker: true}`. An id in no menu is an
/// error.
fn invoke(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.menu.invoke";
    let command = command_ref(M, params)?;
    if !app.catalog.menu.menus.iter().any(|menu| in_menu(&menu.children, &command.id, 0)) {
        return Err(format!("{M}: `{}` is not a menu item", command.id));
    }
    app.run_now(FontCraftApp::route(command), true)
}
