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
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
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

    shell(&mut app, "view.showTab", json!({"index": 2}));
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

/// A dialog targets the view in front: the glyph opened in the edit view only while the edit view shows, Font View's
/// selection only while Font View shows.
#[test]
fn dialogs_target_the_view_in_front() {
    let mut app = app();
    assert!(!app.targets_chosen(), "no glyph opened, nothing selected");
    shell(&mut app, "view.editSelectedGlyphs", json!({"glyphs": ["A"]}));
    assert!(app.targets_chosen(), "the tab that opened shows A");

    shell(&mut app, "fontView.show", json!({}));
    assert!(!app.targets_chosen(), "A is behind Font View, which has nothing selected");
    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["B"]}));
    assert!(app.targets_chosen());
    assert_eq!(app.with_targets(CommandRef::new("path.correctPathDirection")).params["glyphs"], json!(["B"]));

    // Position 2 is the tab the window opened with, 3 the one A opened in.
    shell(&mut app, "view.showTab", json!({"index": 3}));
    assert!(app.targets_chosen(), "the edit view in front shows A");
    let fired = app.with_targets(CommandRef::new("path.correctPathDirection"));
    assert_ne!(fired.params["glyphs"], json!(["B"]), "Font View's selection behind the edit view is no target: {}", fired.params);
}

/// The glyph names in what firing `id` posts, or the status line when it posted nothing.
fn fired_on(app: &mut FontCraftApp, id: &str) -> Result<String, String> {
    app.status.clear();
    app.sent.clear();
    app.fire(CommandRef::new(id));
    match app.sent.as_slice() {
        [Msg::Run(command)] => Ok(command.params.to_string()),
        [] => Err(app.status.clone()),
        other => Ok(format!("{other:?}")),
    }
}

/// A glyph command fired with no glyph shown and nothing selected in Font View refuses and changes nothing, rather
/// than acting on the font's first glyph; with Font View's selection or a glyph shown it runs on those. A font-level
/// command still runs.
#[test]
fn a_glyph_command_fired_with_nothing_chosen_refuses() {
    let mut app = app();
    assert!(!app.targets_chosen());
    assert_eq!(fired_on(&mut app, "path.removeOverlap"), Err("Remove Overlap: select a glyph first".into()));
    app.drain();
    assert!(!app.has_unsaved_changes());
    let font_level = fired_on(&mut app, "font.info.syncVersion");
    assert!(font_level.is_ok(), "{font_level:?}");

    shell(&mut app, "fontView.setSelection", json!({"glyphs": ["A"]}));
    let selected = fired_on(&mut app, "path.removeOverlap");
    assert!(selected.as_ref().is_ok_and(|p| p.contains("\"A\"")), "{selected:?}");

    shell(&mut app, "view.editSelectedGlyphs", json!({"glyphs": ["B"]}));
    shell(&mut app, "fontView.setSelection", json!({"glyphs": []}));
    let shown = fired_on(&mut app, "path.removeOverlap");
    assert!(shown.as_ref().is_ok_and(|p| p.contains("\"B\"") && !p.contains(".notdef")), "{shown:?}");
}

/// `fontView.focusSearch`, and Edit › Find without parameters while Font View shows, ask the search field to take the
/// keyboard once; the field's answer clears the request. In the edit view nothing is asked.
#[test]
fn focus_search_asks_the_search_field_to_take_the_keyboard_once() {
    let mut app = app();
    shell(&mut app, "fontView.focusSearch", json!({}));
    assert!(app.models.font_view.focus_search, "{}", app.status);
    app.sent.push(Msg::View(crate::app::ViewMsg::FontViewSearchFocused));
    app.drain();
    assert!(!app.models.font_view.focus_search, "asked once");
    shell(&mut app, "edit.find", json!({}));
    assert!(app.models.font_view.focus_search && !app.view.text.find.open, "Find in Font View searches its glyphs");
    app.sent.push(Msg::View(crate::app::ViewMsg::FontViewSearchFocused));
    shell(&mut app, "view.showTab", json!({"index": 2}));
    assert!(!app.font_view_shows());
    shell(&mut app, "fontView.focusSearch", json!({}));
    assert!(!app.view.font_view.focus_search, "the edit view has no search field to focus");
}
