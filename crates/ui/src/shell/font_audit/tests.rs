#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use crate::app::{APP, FontCraftApp, Msg};
use crate::testing::TestApp;

/// Glyph A: one closed curve whose rightmost point lies between its nodes (a missing extreme, a warning with a
/// safe fix). Glyph B: an open line (a failure whose fix is not safe).
fn audited_session() -> Session {
    let mut session = Session::with_document();
    let nodes = json!([
        {"x": 100, "y": 100}, {"x": 300, "y": 100, "type": "offcurve"}, {"x": 300, "y": 300, "type": "offcurve"}, {"x": 100, "y": 300, "type": "curve"}
    ]);
    session.execute("path.addPath", &json!({"glyph": "A", "closed": true, "nodes": nodes})).unwrap();
    session.execute("path.addPath", &json!({"glyph": "B", "closed": false, "nodes": [{"x": 400, "y": 0}, {"x": 500, "y": 0}]})).unwrap();
    session
}

fn audited_app() -> FontCraftApp {
    FontCraftApp::with_edit_tab(audited_session(), Arc::new(Host::builder(APP).build()))
}

fn call(app: &mut FontCraftApp, id: &str, params: Value) {
    if let Err(e) = app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false) {
        panic!("{id}: {e}");
    }
}

fn refused(app: &mut FontCraftApp, id: &str, params: Value) -> String {
    app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false).expect_err(id)
}

fn panel(app: &FontCraftApp) -> Value {
    super::panel_json(app)
}

fn id_of(app: &FontCraftApp, check: &str, glyph: &str) -> String {
    let run = app.view.font_audit.run.clone().unwrap();
    run.items.iter().find(|i| i.check == check && i.glyph.as_deref() == Some(glyph)).unwrap_or_else(|| panic!("no {check} on {glyph}")).id.clone()
}

fn has(app: &FontCraftApp, check: &str, glyph: &str) -> bool {
    let run = app.view.font_audit.run.clone().unwrap();
    run.items.iter().any(|i| i.check == check && i.glyph.as_deref() == Some(glyph) && !i.ignored)
}

fn undo_len(app: &FontCraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

#[test]
fn show_panel_and_panel_run_list_what_audit_run_returns_and_filter_results_narrows_it() {
    let mut app = audited_app();
    assert!(crate::shell::commands::find("audit.showPanel").is_some_and(|(_, c)| c.menu == ["Window"]), "Window › Font Audit");
    call(&mut app, "audit.showPanel", json!({}));
    assert_eq!(panel(&app)["open"], true);

    call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
    let out = panel(&app);
    assert_eq!((out["profile"].clone(), out["run"]["kept"].clone()), (json!("outlines"), json!(true)));
    let kept = app.session.query("audit.results", &json!({"includeIgnored": true})).unwrap();
    let ids: Vec<&str> = kept["results"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    let run = app.view.font_audit.run.clone().unwrap();
    assert_eq!(run.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ids, "the panel lists the run the engine keeps");
    assert!(has(&app, "check.outlines.missingExtremes", "A") && has(&app, "check.outlines.openPath", "B"));
    assert!(!app.models.font_audit.groups.is_empty(), "the view model lists the results while the panel shows");

    call(&mut app, "audit.filterResults", json!({"minSeverity": "fail"}));
    let rows = panel(&app)["rows"].as_array().unwrap().clone();
    assert!(!rows.is_empty() && rows.iter().all(|r| r["severity"] == "fail"), "{rows:?}");
    call(&mut app, "audit.filterResults", json!({"minSeverity": "warn", "text": "MissingExtremes", "groupBy": "glyph"}));
    let out = panel(&app);
    assert_eq!(out["listed"], 1, "the search ignores case");
    assert_eq!(out["rows"][0]["glyph"], "A");

    let before = app.view.font_audit.filter.clone();
    refused(&mut app, "audit.filterResults", json!({"minSeverity": "worst"}));
    refused(&mut app, "audit.filterResults", json!({"groups": ["outlines", "colour"]}));
    refused(&mut app, "audit.showPanel", json!({"target": "everything"}));
    assert_eq!(app.view.font_audit.filter, before, "a refused command changes nothing");
    assert_eq!(app.view.font_audit.target, "font");
    call(&mut app, "audit.showPanel", json!({"visible": false}));
    assert_eq!(panel(&app)["open"], false);
}

#[test]
fn reveal_issue_shows_the_glyph_and_next_and_previous_issue_wrap() {
    let mut app = audited_app();
    call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
    call(&mut app, "audit.filterResults", json!({"minSeverity": "warn"}));
    let extreme = id_of(&app, "check.outlines.missingExtremes", "A");
    call(&mut app, "audit.revealIssue", json!({"result": extreme}));
    assert_eq!(app.view.glyph.as_deref(), Some("A"), "the glyph shows");
    assert_eq!(app.view.font_audit.current.as_deref(), Some(extreme.as_str()));
    assert_eq!(app.view.font_audit.selected, vec![extreme.clone()]);

    let listed = panel(&app)["listed"].as_u64().unwrap() as usize;
    assert!(listed >= 2);
    let mut seen = Vec::new();
    for _ in 0..listed {
        call(&mut app, "audit.nextIssue", json!({}));
        seen.push(app.view.font_audit.current.clone().unwrap());
    }
    assert_eq!(seen.last(), Some(&extreme), "Next comes back to the start after the last result");
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), listed, "Next visits every listed result once");
    call(&mut app, "audit.previousIssue", json!({}));
    call(&mut app, "audit.nextIssue", json!({}));
    assert_eq!(app.view.font_audit.current.as_deref(), Some(extreme.as_str()), "Previous and Next undo each other");

    call(&mut app, "audit.nextIssue", json!({"scope": "glyph"}));
    assert_eq!(app.view.glyph.as_deref(), Some("A"), "the glyph scope stays in the shown glyph");
    let run_id = app.view.font_audit.run.as_ref().and_then(|r| r.run_id.clone());
    call(&mut app, "audit.revealIssue", json!({"result": extreme, "runId": run_id}));
    assert!(refused(&mut app, "audit.revealIssue", json!({"result": extreme, "runId": "run0"})).contains("lists run"));
    refused(&mut app, "audit.revealIssue", json!({"result": "r999999"}));
    refused(&mut app, "audit.nextIssue", json!({"scope": "everywhere"}));
    call(&mut app, "audit.filterResults", json!({"text": "nothing matches this"}));
    refused(&mut app, "audit.nextIssue", json!({}));
}

#[test]
fn fix_fixes_the_selected_result_after_another_run_replaced_the_kept_one() {
    let mut app = audited_app();
    call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
    let extreme = id_of(&app, "check.outlines.missingExtremes", "A");
    // Another run (live audit, an agent) replaces the engine's kept run.
    app.session.execute("audit.run", &json!({"profile": "universal"})).unwrap();
    let undo = undo_len(&app);
    call(&mut app, "audit.panel.setView", json!({"selected": [extreme], "current": extreme}));
    call(&mut app, "audit.panel.fix", json!({}));
    assert!(!has(&app, "check.outlines.missingExtremes", "A"), "the re-run no longer lists the fixed result");
    assert!(has(&app, "check.outlines.openPath", "B"), "other results stay");
    assert_eq!(undo_len(&app), undo + 1, "the fix is one undo step");
}

#[test]
fn ignore_hides_the_selected_result_and_show_ignored_lists_it_again() {
    let mut app = audited_app();
    call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
    let open = id_of(&app, "check.outlines.openPath", "B");
    call(&mut app, "audit.panel.setView", json!({"selected": [open], "reason": "a stroke glyph"}));
    call(&mut app, "audit.panel.ignore", json!({}));
    assert!(!has(&app, "check.outlines.openPath", "B"), "the ignored result is hidden");
    call(&mut app, "audit.filterResults", json!({"showIgnored": true}));
    let out = panel(&app);
    let row = out["rows"].as_array().unwrap().iter().find(|r| r["check"] == "check.outlines.openPath").cloned().unwrap();
    assert_eq!(row["ignored"], true);
    let entries = app.session.query("audit.listIgnored", &json!({})).unwrap();
    assert_eq!(entries["entries"][0]["reason"], "a stroke glyph");

    call(&mut app, "audit.panel.setView", json!({"selected": [row["id"]]}));
    call(&mut app, "audit.panel.unignore", json!({}));
    assert!(has(&app, "check.outlines.openPath", "B"), "Stop Ignoring lists it again");
}

#[test]
fn a_run_in_steps_goes_on_once_a_frame_and_lists_what_audit_run_returns() {
    let mut session = audited_session();
    let whole = session.execute("audit.run", &json!({"profile": "outlines", "includeIgnored": true})).unwrap();
    let mut t = TestApp::with(session);
    t.ok("engine.execute", json!({"command": "audit.panel.run", "params": {"profile": "outlines", "inSteps": true}}));
    let started = t.ok("ui.inspect", json!({}))["fontAudit"].clone();
    assert!(started["progress"]["total"].as_u64().unwrap() > 3, "{started}");
    assert!(started["open"] == true, "the panel shows the run");
    t.frames(12);
    let done = t.ok("ui.inspect", json!({}))["fontAudit"].clone();
    assert_eq!(done["progress"], Value::Null, "the frames finished the run");
    assert_eq!(done["run"]["kept"], true);
    assert_eq!(done["run"]["results"].as_u64().unwrap() as usize, whole["results"].as_array().unwrap().len(), "the same results as one audit.run");
}

#[test]
fn cancel_lists_partial_results_and_changes_nothing() {
    let mut app = audited_app();
    call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
    let kept = panel(&app)["run"]["runId"].clone();
    let revision = app.session.doc().unwrap().revision;
    call(&mut app, "audit.panel.run", json!({"profile": "outlines", "inSteps": true}));
    // A new font's glyphs start .notdef, space, A: three layers reach A and stop before B.
    call(&mut app, "audit.panel.continue", json!({"budget": 3}));
    assert_eq!(panel(&app)["progress"]["done"], json!(3));
    call(&mut app, "audit.panel.cancel", json!({}));
    let out = panel(&app);
    assert_eq!(out["progress"], Value::Null);
    assert_eq!((out["run"]["partial"].clone(), out["run"]["kept"].clone()), (json!(true), json!(false)));
    let run = app.view.font_audit.run.clone().unwrap();
    assert!(!run.items.is_empty() && run.items.iter().all(|i| i.glyph.as_deref() != Some("B")), "only the first three layers were examined");
    assert_eq!(app.session.query("audit.results", &json!({})).unwrap()["runId"], kept, "the kept run is unchanged");
    assert_eq!(app.session.doc().unwrap().revision, revision, "the font is unchanged");
    assert!(refused(&mut app, "audit.panel.previewFix", json!({"results": [run.items[0].id]})).contains("engine keeps"));
    assert!(refused(&mut app, "audit.panel.cancel", json!({})).contains("in progress"));
    assert!(refused(&mut app, "audit.panel.continue", json!({})).contains("in progress"));
}

#[test]
fn preview_fix_lists_the_planned_edits_and_apply_makes_them() {
    let mut app = audited_app();
    call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
    let extreme = id_of(&app, "check.outlines.missingExtremes", "A");
    let revision = app.session.doc().unwrap().revision;
    let undo = undo_len(&app);
    call(&mut app, "audit.panel.previewFix", json!({"results": [extreme]}));
    let preview = app.view.font_audit.preview.clone().unwrap();
    assert!(preview.planned.iter().any(|p| p.contains("path.addExtremes")), "{preview:?}");
    assert_eq!(app.session.doc().unwrap().revision, revision, "a preview changes nothing");
    call(&mut app, "audit.panel.setView", json!({"selected": [extreme], "current": extreme}));
    assert!(app.models.font_audit.detail.as_ref().is_some_and(|d| d.previewed), "the detail area shows the preview");

    call(&mut app, "audit.panel.fix", json!({"preview": true}));
    assert!(!has(&app, "check.outlines.missingExtremes", "A"), "Apply fixed the previewed result");
    assert_eq!(undo_len(&app), undo + 1);
    assert!(app.view.font_audit.preview.is_none(), "the re-run drops the preview");
}

#[test]
fn export_report_writes_the_listed_run() {
    let mut app = audited_app();
    call(&mut app, "audit.panel.run", json!({"profile": "outlines"}));
    let listed = panel(&app)["run"].clone();
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-audit-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let js = dir.join("report.json");
    call(&mut app, "audit.panel.exportReport", json!({"path": js.to_string_lossy(), "format": "json"}));
    let written: Value = serde_json::from_slice(&std::fs::read(&js).unwrap()).unwrap();
    assert_eq!(written["runId"], listed["runId"]);
    assert_eq!(written["results"].as_array().unwrap().len(), listed["results"].as_u64().unwrap() as usize);
    assert!(refused(&mut app, "audit.panel.exportReport", json!({})).contains("`path`"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── The Audit Profile dialog ────────────────────────────────────────────────────────────────────

fn app_with_house() -> FontCraftApp {
    let mut app = audited_app();
    app.session.execute("audit.profile.save", &json!({"profile": "house", "label": "House", "include": ["outlines"]})).unwrap();
    app
}

fn shown(app: &mut FontCraftApp, id: &str) -> Value {
    app.session.query("audit.profile.show", &json!({"profile": id})).unwrap()
}

#[test]
fn edit_profile_and_close_profile_dialog_save_what_audit_profile_save_saves() {
    let mut app = app_with_house();
    call(&mut app, "audit.editProfile", json!({"profile": "house"}));
    let d = app.view.font_audit.draft.clone().unwrap();
    assert_eq!((d.id.as_str(), d.store.as_str(), d.read_only), ("house", "user", false));
    assert_eq!(d.include, vec!["outlines".to_string()]);
    assert_eq!(json!(d.count()), shown(&mut app, "outlines")["resolved"]["count"], "the draft runs what its include runs");

    let first = d.catalogue.iter().find(|c| d.runs(&c.id)).unwrap().id.clone();
    let mut edited = d.clone();
    edited.toggle(&first);
    app.view.font_audit.draft = Some(edited.clone());
    assert_eq!(shown(&mut app, "house")["exclude"], json!([]), "nothing is stored before Save");
    call(&mut app, "audit.closeProfileDialog", json!({"action": "save"}));
    assert!(app.view.font_audit.draft.is_none(), "Save closes the dialog");

    let mut direct = app_with_house();
    direct.session.execute("audit.profile.save", &edited.save_params()).unwrap();
    assert_eq!(shown(&mut app, "house"), shown(&mut direct, "house"), "the dialog saves what the command saves");
}

#[test]
fn a_built_in_profile_opens_read_only_and_duplicate_saves_a_user_profile_that_includes_it() {
    let mut app = audited_app();
    call(&mut app, "audit.editProfile", json!({"profile": "outlines"}));
    refused(&mut app, "audit.closeProfileDialog", json!({"action": "save"}));
    assert!(app.view.font_audit.draft.is_some(), "a refused save keeps the dialog open");

    let copy = app.view.font_audit.draft.as_ref().unwrap().duplicate();
    assert_eq!((copy.id.as_str(), copy.read_only, copy.store.as_str()), ("outlines-copy", false, "user"));
    app.view.font_audit.draft = Some(copy);
    call(&mut app, "audit.closeProfileDialog", json!({"action": "save"}));
    let saved = shown(&mut app, "outlines-copy");
    assert_eq!((saved["include"].clone(), saved["store"].clone()), (json!(["outlines"]), json!("user")));

    call(&mut app, "audit.editProfile", json!({"profile": "outlines-copy"}));
    call(&mut app, "audit.closeProfileDialog", json!({"action": "cancel"}));
    assert!(app.session.query("audit.profile.show", &json!({"profile": "outlines-copy"})).is_ok(), "Cancel changes nothing");
    call(&mut app, "audit.editProfile", json!({"profile": "outlines-copy"}));
    call(&mut app, "audit.closeProfileDialog", json!({"action": "delete"}));
    assert!(app.session.query("audit.profile.show", &json!({"profile": "outlines-copy"})).is_err());
}

#[test]
fn toggling_a_check_uses_checks_and_exclude_as_little_as_it_can() {
    let mut app = app_with_house();
    call(&mut app, "audit.editProfile", json!({"profile": "house"}));
    let mut d = app.view.font_audit.draft.clone().unwrap();
    let on = d.catalogue.iter().find(|c| d.runs(&c.id)).unwrap().id.clone();
    let off = d.catalogue.iter().find(|c| !d.runs(&c.id)).unwrap().id.clone();
    d.toggle(&on);
    d.toggle(&off);
    assert_eq!((d.exclude.clone(), d.checks.clone()), (vec![on.clone()], vec![off.clone()]));
    d.toggle(&on);
    d.toggle(&off);
    assert!(d.exclude.is_empty() && d.checks.is_empty(), "turning them back takes the entries out again");
    d.set_override(&on, "fail");
    assert_eq!(d.save_params()["overrides"], json!([{"check": on, "severity": "fail"}]));
    d.set_override(&on, "");
    assert_eq!(d.save_params()["overrides"], json!([]));
}

#[test]
fn the_profile_dialog_shows_in_the_window_and_closes_with_the_command() {
    let mut t = TestApp::with(audited_session());
    t.ok("engine.execute", json!({"command": "audit.editProfile", "params": {"profile": "outlines"}}));
    t.frames(3);
    let shown = t.ok("ui.inspect", json!({}));
    assert_eq!(shown["auditProfile"]["readOnly"], true, "{shown}");
    t.ok("engine.execute", json!({"command": "audit.closeProfileDialog", "params": {"action": "cancel"}}));
    t.frames(3);
    assert_eq!(t.ok("ui.inspect", json!({}))["auditProfile"], Value::Null);
}
