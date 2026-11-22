//! `ui.inspect`: the UI state an agent checks after acting.
//!
//! The skeleton reports the tool, the shown glyph, the panels, the canvas view, the status line and the input
//! still waiting. A package whose state agents need adds a function to [`INSPECT`] that sets its own keys.

use serde_json::{Map, Value, json};

use super::Method;
use crate::app::FontCraftApp;
use crate::shell::dock::PanelId;

pub const METHODS: &[(&str, Method)] = &[("ui.inspect", inspect)];

/// Adds a package's keys to the `ui.inspect` reply.
pub type Inspect = fn(&FontCraftApp, &mut Map<String, Value>);

/// Every package's `ui.inspect` keys, one line per package.
pub const INSPECT: &[Inspect] = &[
    crate::shell::kerning_window::inspect,
    crate::canvas::text::inspect,
    crate::shell::space_center::inspect,
    crate::app::close::inspect,
    crate::shell::start::inspect,
    crate::shell::settings::inspect,
    crate::shell::table_viewer::inspect,
    crate::shell::compare_fonts::inspect,
    crate::shell::compare_family::inspect,
    crate::shell::compare_in_list::inspect,
    crate::shell::text_preview::inspect,
    crate::dialogs::actions::inspect,
    crate::shell::font_audit::inspect,
    crate::shell::kerning_tools::inspect,
    crate::shell::shape_order::inspect,
    crate::shell::bitmap_editor::inspect,
    crate::shell::tabs::inspect,
    crate::dialogs::axis_mappings::inspect,
    crate::dialogs::cross_axis::inspect,
    crate::shell::getters::inspect,
];

/// `{}` → `{tool, glyph, panels, canvas: {zoom, center, rect}, status, closing, unsaved, pendingInput}`, plus
/// the keys of [`INSPECT`].
fn inspect(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    let canvas = &app.view.canvas;
    let rect = canvas.rect;
    let rect = if rect.is_finite() { json!([rect.min.x, rect.min.y, rect.max.x, rect.max.y]) } else { Value::Null };
    let panels: Vec<&str> = PanelId::ALL.iter().filter(|p| shown(app, **p)).map(|p| p.id()).collect();
    let pending = app.control.window.as_ref().is_some_and(|w| !w.injected_idle());
    let mut state = Map::new();
    state.insert("tool".into(), json!(app.view.tool));
    state.insert("glyph".into(), json!(app.models.glyph_metrics.glyph));
    state.insert("panels".into(), json!(panels));
    state.insert("canvas".into(), json!({"zoom": canvas.zoom, "center": [canvas.center.x, canvas.center.y], "rect": rect}));
    state.insert("status".into(), json!(app.status));
    state.insert("closing".into(), json!(format!("{:?}", app.closing)));
    state.insert("unsaved".into(), json!(app.has_unsaved_changes()));
    state.insert("pendingInput".into(), json!(pending));
    for add in INSPECT {
        add(app, &mut state);
    }
    Ok(Value::Object(state))
}

/// Whether `panel` shows in the dock.
pub(super) fn shown(app: &FontCraftApp, panel: PanelId) -> bool {
    app.view.workspace.layout.panel_state(&panel) != craft::behavior::dock::PanelState::Hidden
}
