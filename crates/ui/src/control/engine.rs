//! `engine.execute`, `engine.commands`, `document.inspect` and `control.methods`.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::{LATER, Method, method_names};
use crate::app::FontCraftApp;
use crate::app::catalog::TOOL_PREFIX;

pub const METHODS: &[(&str, Method)] =
    &[("engine.execute", execute), ("engine.commands", commands), ("document.inspect", document), ("control.methods", methods)];

/// The `command` of a request's params.
pub(super) fn command_id<'a>(method: &str, params: &'a Value) -> Result<&'a str, String> {
    params.get("command").and_then(Value::as_str).ok_or_else(|| format!("{method}: `command` (a command id) is required"))
}

/// The command a request names: `command` with its `params`.
pub(super) fn command_ref(method: &str, params: &Value) -> Result<CommandRef, String> {
    let id = command_id(method, params)?;
    let inner = params.get("params").cloned().filter(|p| !p.is_null()).unwrap_or_else(|| json!({}));
    Ok(CommandRef::with_params(id, inner))
}

/// `{command, params?}`: runs an engine command, a shell command or a tool's command, as a fired command runs, and
/// returns its result. A command that takes a path from the file picker needs the path here. The question a result
/// asks (pasted outline far outside the glyph, names the font has) opens only after the person's own run, not here.
fn execute(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let command = command_ref("engine.execute", params)?;
    app.run_request(FontCraftApp::route(command))
}

/// `{}` → `{commands: [{id, label, menu, shortcut, params, query, enabled, disabled_reason?}]}`; shell commands and
/// tools carry `ui: true`.
fn commands(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    let mut list: Vec<Value> =
        fontcraft_engine::command_specs().iter().filter_map(|spec| serde_json::to_value(spec.info(&app.session)).ok()).collect();
    for module in crate::shell::commands::MODULES {
        list.extend(
            module.commands.iter().map(
                |c| json!({"id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "params": c.params, "ui": true, "enabled": true}),
            ),
        );
    }
    list.extend(fontcraft_tools::tool::TOOLS.iter().map(|tool| {
        json!({"id": format!("{TOOL_PREFIX}{}", tool.id), "label": tool.label, "menu": [], "shortcut": null, "params": "{} make the tool current", "ui": true, "enabled": true})
    }));
    Ok(json!({ "commands": list }))
}

/// `{}` → the open documents (`Session::inspect`).
fn document(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    Ok(app.session.inspect())
}

/// `{}` → `{methods: [names], later: [{method, package}]}`: the methods this app answers, and those of packages
/// not built yet.
fn methods(_: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    let later: Vec<Value> = LATER.iter().map(|(method, package)| json!({"method": method, "package": package})).collect();
    Ok(json!({"methods": method_names(), "later": later}))
}
