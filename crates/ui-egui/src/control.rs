//! The control protocol: how agents drive the running app (`docs/control-protocol.md`).
//!
//! One JSON request in, one JSON reply out: `{"id": 1, "method": "engine.execute", "params":
//! {"command": "glyph.newGlyph", "params": {}}}` → `{"id": 1, "ok": true, "result": {…}}` or
//! `{"id": 1, "ok": false, "error": "…"}`. The transport (a loopback TCP port, one JSON object per
//! line) lives in the desktop app; this module is the dispatch and has no I/O.

use fontcraft_engine::guard;
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::menus;

/// The methods [`handle`] answers.
pub const METHODS: &[(&str, &str)] = &[
    ("engine.execute", "{command, params?} run any engine or UI command"),
    ("engine.commands", "{} every command with label, menu, shortcut, params doc, enablement"),
    ("document.inspect", "{} the open documents: title, path, modified, revision, glyph count, history"),
    ("ui.inspect", "{} the UI state: selection, the glyph being edited, zoom, visible aids, status text"),
    ("ui.menu.list", "{} the menu tree with enablement"),
    ("ui.menu.invoke", "{command, params?} run a menu item (the same as engine.execute; an id that is in no menu is an error)"),
    ("control.methods", "{} this list"),
];

/// Answers one request. Never panics: a bug becomes an error reply and the app keeps running.
pub fn handle(app: &mut FontcraftApp, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let reply = |result: Result<Value, String>| match result {
        Ok(result) => json!({"id": id, "ok": true, "result": result}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    };
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return reply(Err("a request is a JSON object with a string `method`".into()));
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    match guard::catch_panic(|| dispatch(app, method, &params)) {
        Ok(result) => reply(result),
        Err(panic) => reply(Err(format!("internal error (please report it): `{method}` failed unexpectedly: {panic}"))),
    }
}

fn dispatch(app: &mut FontcraftApp, method: &str, params: &Value) -> Result<Value, String> {
    let command = || params.get("command").and_then(Value::as_str).ok_or_else(|| format!("{method}: `command` (a command id) is required"));
    let inner = || params.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "engine.execute" => app.run(command()?, &inner()),
        "engine.commands" => Ok(app.commands()),
        "document.inspect" => Ok(app.session.inspect()),
        "ui.inspect" => serde_json::to_value(&app.state).map_err(|e| e.to_string()),
        "ui.menu.list" => serde_json::to_value(menus::menu_tree(&app.session)).map_err(|e| e.to_string()),
        "ui.menu.invoke" => {
            let id = command()?;
            if !menus::menu_tree(&app.session).iter().any(|m| m.items.iter().any(|i| i.command == id)) {
                return Err(format!("{method}: `{id}` is not a menu item"));
            }
            app.run(id, &inner())
        }
        "control.methods" => Ok(json!(METHODS.iter().map(|(m, doc)| json!({"method": m, "params": doc})).collect::<Vec<_>>())),
        other => Err(format!("unknown method `{other}` (see control.methods)")),
    }
}
