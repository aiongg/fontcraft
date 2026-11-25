//! Methods on the open form dialog (`docs/control-protocol.md`, Dialogs). They act on the dialog the app holds
//! (`dialogs::form`), the one the window shows, so an agent and the person see the same dialog.

use serde_json::{Value, json};

use super::Method;
use crate::app::FontCraftApp;
use crate::dialogs::form::{self, DialogKey};

pub const METHODS: &[(&str, Method)] = &[
    ("ui.dialog.inspect", inspect),
    ("ui.dialog.set", set),
    ("ui.dialog.confirm", confirm),
    ("ui.dialog.cancel", cancel),
    ("ui.dialog.click", click),
    ("ui.dialog.key", key),
    ("ui.dialog.text", text),
];

/// `{}` → the open dialog, or `{open: false}`.
fn inspect(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    Ok(form::inspect(app))
}

/// `{field, value}` → the `ui.dialog.inspect` reply.
fn set(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let field = params.get("field").and_then(Value::as_str).ok_or("`field` is required")?;
    let value = params.get("value").ok_or("`value` is required")?;
    form::set(app, field, value)
}

/// `{}` → `{confirmed, changed, command, params}`, or `{confirmed, command, params, result}` for a run-on-OK dialog.
fn confirm(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    form::confirm(app)
}

/// `{}` → `{cancelled}`.
fn cancel(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    form::cancel(app)
}

/// `{id}`: presses `ok`, `cancel`, the third button, a tab, an action or a field (`form::click`). `ui.click {id}`
/// comes here.
pub fn click(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let id = params.get("id").and_then(Value::as_str).ok_or("`id` is required")?;
    form::click(app, id)
}

/// `{key, shift?}`: a key for the open dialog (`form::key`) → `{handled, open, error?}`. `handled` is false for a key
/// the dialog does not take or when no dialog is open; `ui.key` comes here first.
pub fn key(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let name = params.get("key").and_then(Value::as_str).ok_or("`key` is required")?;
    let shift = params.get("shift").and_then(Value::as_bool).unwrap_or(false);
    let key = DialogKey::parse(name).filter(|_| !app.closing.is_open());
    let Some(key) = key else { return Ok(json!({"handled": false, "open": app.dialogs.open.is_some()})) };
    let Some(result) = form::key(app, key, shift) else { return Ok(json!({"handled": false, "open": false})) };
    let open = app.dialogs.open.is_some();
    Ok(match result {
        Ok(_) => json!({"handled": true, "open": open}),
        Err(e) => json!({"handled": true, "open": open, "error": e}),
    })
}

/// `{text}`: types into the dialog's text field that has the keyboard → `{field}`, false when none has it; `ui.text`
/// comes here first.
pub fn text(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let text = params.get("text").and_then(Value::as_str).ok_or("`text` is required")?;
    Ok(json!({"field": !app.closing.is_open() && form::type_text(app, text)}))
}
