//! `ui.text`: typing as the keyboard does. The open dialog's text field that has the keyboard takes the text first,
//! then any other text field that has it (queued on its window); else, in text mode, the edit view's text takes it at
//! the cursor, in place of the selection.

use serde_json::{Value, json};

use super::Method;
use crate::app::FontCraftApp;

/// Most characters one request types.
const MAX_CHARS: usize = 4096;

pub const METHODS: &[(&str, Method)] = &[("ui.text", text)];

/// `{text}` → `{field: true}` when a dialog field or a focused text field took it, else `{typed: bool}` (false outside text mode).
fn text(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.text";
    let text = params.get("text").and_then(Value::as_str).ok_or_else(|| format!("{M}: `text` is required"))?;
    if text.chars().nth(MAX_CHARS).is_some() {
        return Err(format!("{M}: `text` holds at most {MAX_CHARS} characters"));
    }
    // The question about unsaved changes is a modal with no text field: nothing behind it is typed into.
    if app.closing.is_open() {
        return Ok(json!({"typed": false, "closePrompt": true}));
    }
    if app.dialogs.open.is_some() {
        let reply = super::dialogs::text(app, params).map_err(|e| format!("{M}: {e}"))?;
        if reply.get("field").and_then(Value::as_bool) == Some(true) {
            return Ok(reply);
        }
    }
    if let Some(host) = app.control.window.clone().filter(craft::ui::runtime::windows::WindowHost::typing) {
        host.inject_text(host.key_window(), text);
        app.control.queued += 1;
        return Ok(json!({"field": true}));
    }
    if !crate::canvas::text::text_mode(&app.view) {
        return Ok(json!({"typed": false}));
    }
    app.drain();
    let result = crate::canvas::text::input::type_chars(app, text);
    app.drain();
    result.map(|()| json!({"typed": true})).map_err(|e| format!("{M}: {e}"))
}
