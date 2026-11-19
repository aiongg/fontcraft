//! The sidebar's, the inspector's and Selection Info's app layer: their shell commands and their projections.

use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::models::glyph_props::PropField;
use crate::app::{APP, FontCraftApp, Msg};
use crate::shell::dock::PanelId;

fn app() -> FontCraftApp {
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
    shell(&mut app, "fontView.show", json!({}));
    app
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
    app.status.clear();
    app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    app.drain();
}

fn path(text: &str) -> Vec<String> {
    text.split('/').map(str::to_string).collect()
}

/// Selecting a category narrows Font View and deselects the glyphs it hides; a bad path changes nothing; All shows
/// every glyph again.
#[test]
fn sidebar_entries_narrow_font_view_and_deselect_hidden_glyphs() {
    let mut app = app();
    let side = &app.models.sidebar;
    assert!(side.has_font && side.rows.iter().any(|r| r.kind == "all"), "{:?}", side.error);
    assert!(side.row_of(&path("categories/Letter")).is_some(), "the Letter category");
    let total = app.models.font_view.visible.len();
    shell(&mut app, "fontView.setSelection", json!({ "glyphs": ["A", "space"] }));

    shell(&mut app, "fontView.selectSidebarEntries", json!({ "entries": ["categories/Letter"] }));
    assert_eq!(app.status, "");
    assert_eq!(app.models.sidebar.selected, vec![path("categories/Letter")]);
    assert!(app.models.font_view.visible.len() < total);
    assert_eq!(app.view.font_view.selected, vec!["A".to_string()], "space is hidden, so it is deselected");

    shell(&mut app, "fontView.selectSidebarEntries", json!({ "entries": [""] }));
    assert!(!app.status.is_empty());
    assert_eq!(app.models.sidebar.selected, vec![path("categories/Letter")]);

    shell(&mut app, "fontView.selectSidebarEntries", json!({ "entries": ["all"] }));
    assert!(app.models.sidebar.selected.is_empty() && app.view.font_view.entries.is_none());
    assert_eq!(app.models.font_view.visible.len(), total);
}

/// The inspector describes the Font View selection; its switches change what it shows; Info for Selection shows the
/// Selection Info panel over the same glyphs.
#[test]
fn the_inspector_and_selection_info_follow_the_selection() {
    let mut app = app();
    shell(&mut app, "fontView.setSelection", json!({ "glyphs": ["A", "B"] }));
    let inspector = &app.models.inspector;
    assert_eq!(inspector.glyphs, vec!["A".to_string(), "B".to_string()]);
    assert!(inspector.field(PropField::Name).mixed);
    assert_eq!(inspector.metrics.len(), 3);

    shell(&mut app, "fontView.toggleMetricsOrientation", json!({}));
    shell(&mut app, "fontView.toggleInspectorDetails", json!({ "expanded": true }));
    shell(&mut app, "fontView.toggleInspector", json!({}));
    let inspector = &app.models.inspector;
    assert!(inspector.vertical && inspector.details && inspector.hidden);
    assert_eq!(inspector.metrics.len(), 4, "TSB, BSB, vertical origin, vertical width");

    assert!(app.models.selection_info.glyphs.is_empty(), "nothing is projected while the panel is hidden");
    shell(&mut app, "edit.infoForSelection", json!({}));
    assert_ne!(app.view.workspace.layout.panel_state(&PanelId::SelectionInfo), PanelState::Hidden);
    assert_eq!(app.models.selection_info.glyphs, vec!["A".to_string(), "B".to_string()]);
    shell(&mut app, "edit.infoForSelection", json!({ "open": false }));
    assert_eq!(app.view.workspace.layout.panel_state(&PanelId::SelectionInfo), PanelState::Hidden);
}

/// A typed vertical origin in the inspector reaches the engine as `spacing.setVerticalOrigin` and the field then
/// shows it.
#[test]
fn a_vertical_origin_edit_reaches_the_engine() {
    let mut app = app();
    shell(&mut app, "fontView.setSelection", json!({ "glyphs": ["A"] }));
    shell(&mut app, "fontView.toggleMetricsOrientation", json!({ "orientation": "vertical" }));
    let edit = crate::app::models::glyph_props::Edit::Metric(crate::app::models::glyph_props::MetricField::VertOrigin, "40".into());
    let msg = crate::app::models::glyph_props::command(&app.models.inspector, &edit).expect("a command");
    app.sent.push(msg);
    app.drain();
    assert_eq!(app.status, "");
    let origin = crate::panels::font_view::metrics::shown(&app.models.inspector, crate::app::models::glyph_props::MetricField::VertOrigin);
    assert_eq!(origin.text, "40");
}
