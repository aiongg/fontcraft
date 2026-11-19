//! Font View's app layer: its shell commands, its projection, the targets it gives fired commands, and what
//! follows the commands it runs.

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{Column, ViewMode, incompatible_notice};
use crate::app::{APP, FontCraftApp, Msg};

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

/// The shown glyphs' names, in display order.
fn shown(app: &FontCraftApp) -> Vec<String> {
    let m = &app.models.font_view;
    m.visible.iter().filter_map(|&i| m.name(i)).map(str::to_string).collect()
}

fn selected(app: &FontCraftApp) -> Vec<String> {
    let m = &app.models.font_view;
    m.selected.iter().filter_map(|&i| m.name(i)).map(str::to_string).collect()
}

/// Each shell command changes the view state the model shows; a refused one changes nothing and says why.
#[test]
fn shell_commands_change_what_the_model_shows() {
    let mut app = app();
    let m = &app.models.font_view;
    assert!(m.has_font && m.mode == ViewMode::Grid && m.cell_size == 64.0);
    assert_eq!((m.visible.len(), m.rows.len()), (54, 54));
    assert_eq!(shown(&app).first().map(String::as_str), Some(".notdef"), "glyph order");
    assert!(m.rows.iter().all(|r| r.art.is_some() && r.cells.is_empty()), "Grid View projects outlines only");

    shell(&mut app, "fontView.setViewMode", json!({"mode": "list"}));
    assert_eq!(app.models.font_view.mode, ViewMode::List);
    assert!(app.models.font_view.rows.iter().all(|r| r.art.is_none() && r.cells.len() == Column::ALL.len()));
    shell(&mut app, "fontView.setViewMode", json!({"mode": "table"}));
    assert!(app.status.contains("grid"), "{}", app.status);
    assert_eq!(app.models.font_view.mode, ViewMode::List, "a refused command changes nothing");

    shell(&mut app, "fontView.setCellSize", json!({"size": 1e9}));
    assert_eq!(app.models.font_view.cell_size, 512.0);
    shell(&mut app, "fontView.setCellSize", json!({"size": 2}));
    assert_eq!(app.models.font_view.cell_size, 16.0);

    // A second request on the same column reverses it; glyphs without a value go last; ID ascending is glyph order.
    shell(&mut app, "fontView.sortByColumn", json!({"column": "unicode"}));
    shell(&mut app, "fontView.sortByColumn", json!({"column": "unicode"}));
    let order = shown(&app);
    assert_eq!(order.get(..3), Some(&["z".to_string(), "y".into(), "x".into()][..]));
    assert_eq!(order.last().map(String::as_str), Some(".notdef"));
    shell(&mut app, "fontView.sortByColumn", json!({"column": "id"}));
    assert_eq!((app.models.font_view.sort, shown(&app).first().map(String::as_str)), (None, Some(".notdef")));
    shell(&mut app, "fontView.sortByColumn", json!({"column": "colour"}));
    assert!(app.status.contains("column"), "{}", app.status);

    shell(&mut app, "fontView.setColumns", json!({"order": ["note", "name"]}));
    assert_eq!(app.models.font_view.columns.get(..3), Some(&[Column::Note, Column::Name, Column::Id][..]));
    shell(&mut app, "fontView.setColumns", json!({"visible": ["unicode", "name"]}));
    assert_eq!(app.models.font_view.columns, [Column::Name, Column::Unicode], "shown in the stored order");
    shell(&mut app, "fontView.setColumns", json!({"visible": []}));
    assert!(!app.status.is_empty());

    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A", "B"]}));
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["C", "A"], "mode": "add"}));
    assert_eq!(selected(&app), ["A", "B", "C"]);
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["B"], "mode": "remove"}));
    assert_eq!(selected(&app), ["A", "C"]);
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["nope"]}));
    assert!(app.status.contains("nope"), "{}", app.status);
    assert_eq!(selected(&app), ["A", "C"]);

    shell(&mut app, "view.editSelectedGlyphs", json!({}));
    assert_eq!(app.view.glyph.as_deref(), Some("A"), "the selection opens in Edit View");
}

/// The search narrows the shown glyphs through the engine's query, and clearing it shows them all again.
#[test]
fn the_search_narrows_the_shown_glyphs() {
    let mut app = app();
    shell(&mut app, "fontView.setSearch", json!({"query": "Q"}));
    let found = shown(&app);
    assert!(found.contains(&"Q".to_string()) && found.len() < 54, "{found:?}");
    shell(&mut app, "fontView.setSearch", json!({"query": ""}));
    assert_eq!(shown(&app).len(), 54);
}

/// A command fired without targets while Font View shows acts on its selection; with Font View hidden, or given
/// targets, it stays as it was.
#[test]
fn fired_commands_take_the_font_view_selection() {
    let mut app = app();
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A", "B"]}));
    let fired = app.with_font_view_targets(CommandRef::new("path.correctPathDirection"));
    assert_eq!(fired.params["glyphs"], json!(["A", "B"]));
    assert!(fired.params["layer"].is_string(), "{}", fired.params);

    let given = CommandRef::with_params("path.correctPathDirection", json!({"glyphs": ["C"]}));
    assert_eq!(app.with_font_view_targets(given.clone()), given);

    let batch = app.with_font_view_targets(CommandRef::with_params("glyph.addComponent", json!({"component": "acutecomb"})));
    assert_eq!(batch.id, "command.batch");
    assert_eq!(batch.params["steps"].as_array().map(Vec::len), Some(2));

    shell(&mut app, "window.togglePanel", json!({"panel": "fontView"}));
    let hidden = app.with_font_view_targets(CommandRef::new("path.correctPathDirection"));
    assert!(hidden.params.is_null(), "{}", hidden.params);
}

/// A new glyph becomes the selection; a removed one leaves it.
#[test]
fn a_new_glyph_is_selected_and_a_removed_one_deselected() {
    let mut app = app();
    app.sent.push(Msg::run("glyph.newGlyph", json!({})));
    app.drain();
    assert_eq!(selected(&app), ["newGlyph"], "{}", app.status);
    app.sent.push(Msg::run("glyph.removeGlyph", json!({"glyphs": ["newGlyph"], "confirm": true})));
    app.drain();
    assert!(app.view.font_view.selected.is_empty(), "{}", app.status);
}

/// Tidy up and Round Coordinates name the glyphs they left incompatible; other commands and empty lists say nothing.
#[test]
fn the_incompatible_notice_names_the_glyphs() {
    let reply = json!({"glyphs": ["A", "B"], "incompatible": ["B"]});
    let notice = incompatible_notice("path.tidyUpPathsForAllMasters", &reply).expect("a notice");
    assert!(notice.ends_with(": B"), "{notice}");
    assert_eq!(incompatible_notice("path.roundCoordinates", &json!({"incompatible": []})), None);
    assert_eq!(incompatible_notice("path.removeOverlap", &reply), None);
}
