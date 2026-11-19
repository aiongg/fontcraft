//! Methods about the app's own state.

use serde_json::{Value, json};

use super::Method;
use crate::app::FontCraftApp;

pub const METHODS: &[(&str, Method)] = &[("app.state", state)];

/// `{}` → `{documents, active, tool, status, unsaved}`.
fn state(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    Ok(json!({
        "documents": app.session.documents().len(),
        "active": app.session.active().map(|d| d.title.clone()),
        "tool": app.view.tool,
        "status": app.status,
        "unsaved": app.has_unsaved_changes(),
    }))
}
