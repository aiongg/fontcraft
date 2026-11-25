//! Behaviour tests of the Features and Exports tabs of Font Info and the Add Feature picker.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::dialog::{DialogResult, DialogStack, StackDialog};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{Props, show};
use crate::app::models::font_exports::{ExportEntry, ExportFields, ExportsModel, Flag};
use crate::app::models::font_features::{DiagnosticModel, EntryPane, EntryRow, FeaturesModel, GroupModel};
use crate::app::models::font_info::FontInfoModel;
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::dialogs::feature_picker::FeaturePicker;
use crate::shell::font_info::FontInfoTab;
use crate::shell::font_info_tabs::{COMPILE, CompletionAsk, EntryKey, EntryKind, FeatureDraft, PreviewAsk};

const ROOM: egui::Vec2 = egui::vec2(1280.0, 900.0);

fn panel(model: FontInfoModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let model = Tracked::new(model);
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.room(ROOM).settle();
    (fixture, posted)
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn key(kind: EntryKind, name: &str, index: usize) -> EntryKey {
    EntryKey { kind, name: name.into(), index }
}

fn row(name: &str) -> EntryRow {
    EntryRow { name: name.into(), ..EntryRow::default() }
}

/// A Features tab with one class and the features `liga` and `ss01`; `ss01` is shown.
fn features_model() -> FontInfoModel {
    let shown = key(EntryKind::Feature, "ss01", 1);
    let features = FeaturesModel {
        groups: vec![
            GroupModel { kind: EntryKind::Prefix, ..GroupModel::default() },
            GroupModel { kind: EntryKind::Class, entries: vec![row("NewClass")], hidden: 0 },
            GroupModel { kind: EntryKind::Feature, entries: vec![row("liga"), EntryRow { error: true, ..row("ss01") }], hidden: 0 },
        ],
        selected: vec![shown.clone()],
        entry: Some(EntryPane {
            key: shown,
            code: "sub a by b;".into(),
            stylistic_set: true,
            labels: vec![("dflt".into(), "Swash".into())],
            diagnostics: vec![DiagnosticModel {
                id: 7,
                error: true,
                message: "unknown glyph".into(),
                line: 1,
                range: Some((4, 5)),
                fixes: vec!["Remove".into()],
            }],
            compiled: true,
            ..EntryPane::default()
        }),
        ..FeaturesModel::default()
    };
    FontInfoModel { open: true, tab: FontInfoTab::Features, features, ..FontInfoModel::default() }
}

/// The sidebar's plus buttons, the buttons under it and a click on an entry post their commands; the class plus picks
/// a free name and the feature plus opens the picker.
#[test]
fn features_sidebar_and_buttons_post_their_commands() {
    let (mut fixture, posted) = panel(features_model());
    for label in ["Add a class", "Add a feature", "Delete the selected entries", "Update", "Compile"] {
        let button = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} button"));
        fixture.click(&button).settle();
    }
    let liga = fixture.by_label("liga").expect("the liga entry");
    fixture.click(&liga).settle();
    let ss01 = json!({ "kind": "feature", "name": "ss01", "index": 1 });
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("font.features.addClass", json!({ "name": "NewClass1" })),
            shell("font.info.openDialog", json!({ "dialog": "featurePicker" })),
            Msg::run("font.features.remove", json!({ "entries": [ss01] })),
            Msg::run("font.features.update", json!({})),
            Msg::Shell(CommandRef::new(COMPILE)),
            shell("font.info.features.select", json!({ "entries": [{ "kind": "feature", "name": "liga", "index": 0 }] })),
        ]
    );
}

/// The feature code is coloured by class, and the range of the compile's diagnostic is underlined.
#[test]
fn feature_code_is_coloured_and_its_errors_underlined() {
    let (mut fixture, _) = panel(features_model());
    fixture.settle();
    let shapes = fixture.frame_shapes();
    let galley = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(t) if t.galley.job.text == "sub a by b;" => Some(t.galley.clone()),
            _ => None,
        })
        .expect("the code is painted");
    let sections = &galley.job.sections;
    let underlined: Vec<(usize, usize)> =
        sections.iter().filter(|s| s.format.underline.width > 0.0).map(|s| (s.byte_range.start.0, s.byte_range.end.0)).collect();
    assert_eq!(underlined, vec![(4, 5)]);
    let mut colors: Vec<egui::Color32> = sections.iter().map(|s| s.format.color).collect();
    colors.dedup();
    assert!(colors.len() > 1, "the keyword and the glyph names differ in colour: {colors:?}");
}

/// Typing a bad statement shows the underline of its check without a commit: the code as typed goes to the app, and the
/// check of exactly that text is underlined.
#[test]
fn a_typed_statement_is_underlined_from_its_check_without_a_commit() {
    let typed = "sub a by zz;";
    let mut model = features_model();
    if let Some(e) = model.features.entry.as_mut() {
        let found = DiagnosticModel { id: 0, error: true, message: "glyph not in font".into(), line: 1, range: Some((9, 11)), fixes: Vec::new() };
        e.check = Some((typed.into(), vec![found]));
    }
    let (mut fixture, posted) = panel(model);
    let code = fixture.by_label("Feature Code").expect("the code field");
    fixture.click(&code).key(Key::A, Modifiers::COMMAND).type_text(typed).settle();
    let draft = FeatureDraft { key: key(EntryKind::Feature, "ss01", 1), code: typed.into() };
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureDraft(Some(draft)))), "{:?}", posted.borrow());
    assert!(!posted.borrow().iter().any(|m| m.command_id() == Some("font.features.setCode")), "typing commits nothing");
    let galley = fixture
        .frame_shapes()
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(t) if t.galley.job.text == typed => Some(t.galley.clone()),
            _ => None,
        })
        .expect("the typed code is painted");
    let underlined: Vec<(usize, usize)> =
        galley.job.sections.iter().filter(|s| s.format.underline.width > 0.0).map(|s| (s.byte_range.start.0, s.byte_range.end.0)).collect();
    assert_eq!(underlined, vec![(9, 11)]);
}

/// A draft is checked once it has stood for the pause, on the font as it is, and its diagnostics reach the shown entry
/// while the font keeps its code; stored code is then compiled in the background.
#[test]
fn a_draft_is_checked_after_the_pause_and_stored_code_compiles_in_the_background() {
    use crate::shell::feature_work::step;
    use fontcraft_engine::feature_state::SessionFeaturesExt as _;
    // Inline jobs: each check and build finishes inside the step that starts it.
    let host = Host::builder(APP).jobs(Arc::new(craft::host::InlineJobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false, "code": "sub a by b;" })));
    app.sent.push(shell("font.info", json!({ "tab": "features" })));
    app.drain();
    for now in [1.0, 1.6, 1.7] {
        step(&mut app, now);
    }
    let compiled = app.session.feature_state().expect("the added feature is compiled");
    let font = Arc::clone(&app.session.doc().expect("a document").font);
    let draft = FeatureDraft { key: key(EntryKind::Feature, "liga", 0), code: "sub a by b;\nsub c by qq;".into() };
    app.sent.push(Msg::View(ViewMsg::FeatureDraft(Some(draft.clone()))));
    app.drain();
    assert!(step(&mut app, 10.0).is_some(), "the check waits for the pause");
    assert!(app.view.font_info.features.checked.is_none());
    step(&mut app, 10.6);
    step(&mut app, 10.7);
    app.drain();
    let entry = app.models.font_info.features.entry.clone().expect("the shown entry");
    let (code, found) = entry.check.expect("the check landed");
    assert_eq!(code, draft.code);
    assert_eq!(found.iter().map(|d| (d.error, d.line, d.range)).collect::<Vec<_>>(), vec![(true, 2, Some((21, 23)))]);
    assert!(Arc::ptr_eq(&font, &app.session.doc().expect("a document").font), "the check changes nothing");
    assert!(app.session.feature_state().is_some_and(|state| Arc::ptr_eq(&state, &compiled)), "nor compiles");

    app.sent.push(Msg::run("font.features.setCode", json!({ "kind": "feature", "name": "liga", "code": draft.code })));
    app.drain();
    step(&mut app, 20.0);
    step(&mut app, 20.6);
    step(&mut app, 20.7);
    let state = app.session.feature_state().expect("compiled in the background");
    assert!(state.has_errors());
    assert!(state.is_current(&app.session, app.session.doc().expect("a document")));
}

/// The Compile button's compile runs as a job with the button busy until it lands; an edit while it runs drops the
/// result and the code is compiled again once the document stands still.
#[test]
fn the_compile_button_is_busy_until_the_compile_of_the_font_as_it_is_lands() {
    use crate::shell::feature_work::step;
    use fontcraft_engine::feature_state::SessionFeaturesExt as _;
    let host = Host::builder(APP).jobs(Arc::new(craft::host::InlineJobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false, "code": "sub a by qq;" })));
    app.sent.push(shell("font.info", json!({ "tab": "features" })));
    app.drain();
    app.sent.push(Msg::Shell(CommandRef::new(COMPILE)));
    app.drain();
    assert!(app.models.font_info.features.compiling, "busy");
    assert!(app.session.feature_state().is_none(), "nothing has landed");
    app.sent.push(Msg::run("font.features.setNotes", json!({ "kind": "feature", "name": "liga", "notes": "x" })));
    app.drain();
    step(&mut app, 10.0);
    assert!(app.session.feature_state().is_none(), "the result of the revision before is dropped");
    assert!(app.view.font_info.features.compiling, "still busy");
    step(&mut app, 10.6);
    step(&mut app, 10.7);
    app.drain();
    let state = app.session.feature_state().expect("the compile landed");
    assert!(state.has_errors());
    assert!(!app.models.font_info.features.compiling);
    assert!(app.models.font_info.features.groups.iter().any(|g| g.entries.iter().any(|e| e.error)), "its errors show");
}

/// An edit stops the compile under way; the compile of the code as it is then lands.
#[test]
fn an_edit_stops_the_background_compile() {
    use crate::shell::feature_work::step;
    use fontcraft_engine::feature_state::SessionFeaturesExt as _;
    let jobs = Arc::new(craft::host::ManualJobs::new());
    let host = Host::builder(APP).jobs(Arc::<craft::host::ManualJobs>::clone(&jobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false, "code": "sub a by b;" })));
    app.sent.push(shell("font.info", json!({ "tab": "features" })));
    app.sent.push(Msg::Shell(CommandRef::new(COMPILE)));
    app.drain();
    let doc = app.session.doc().expect("a document");
    let (id, revision, master) = (doc.id, doc.revision, doc.font.masters.first().expect("a master").id.clone());
    app.sent.push(Msg::run("font.features.setCode", json!({ "kind": "feature", "name": "liga", "code": "sub b by a;" })));
    app.drain();
    step(&mut app, 10.0);
    jobs.run_all();
    assert!(app.session.preview_builds().get(id, revision, &master).is_none(), "the stopped build is not kept");
    step(&mut app, 10.1);
    assert!(app.session.feature_state().is_none(), "nothing has landed");
    step(&mut app, 10.7);
    jobs.run_all();
    step(&mut app, 10.8);
    let state = app.session.feature_state().expect("the compile landed");
    assert!(state.is_current(&app.session, app.session.doc().expect("a document")));
    assert!(app.session.target_check().is_some());
    assert!(!app.view.font_info.features.compiling);
}

/// The Exports tab compiles the shown instance's feature parameters as a job: the problem shows once it landed, goes
/// with the code it was found in, and a result for a revision before is dropped.
#[test]
fn the_exports_tab_shows_a_parameters_problem_once_its_compile_lands() {
    use crate::app::models::font_info::Editor;
    use crate::shell::feature_work::step;
    let host = Host::builder(APP).jobs(Arc::new(craft::host::InlineJobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    let param = |value: &str| json!({ "scope": "instance", "targets": ["Bold"], "name": "Replace Feature", "value": value });
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false, "code": "sub a by b;" })));
    app.sent.push(Msg::run("font.instance.add", json!({ "styleName": "Bold" })));
    app.sent.push(Msg::run("font.customParameters.add", param("liga; sub a by b;\nsub c by zzz;")));
    app.drain();
    let bold = app.session.font().expect("a font").instances.iter().position(|i| i.name == "Bold").expect("the instance");
    app.sent.push(shell("font.info", json!({ "tab": "exports" })));
    app.sent.push(shell("font.info.exports.select", json!({ "instances": [bold] })));
    app.drain();
    let problem = |app: &FontCraftApp| {
        app.models.font_info.exports.params.iter().find_map(|r| match &r.editor {
            Editor::Code { problem, .. } => problem.clone(),
            _ => None,
        })
    };
    step(&mut app, 10.0);
    app.drain();
    assert_eq!(problem(&app), None, "under way");
    step(&mut app, 10.1);
    app.drain();
    assert_eq!(problem(&app).map(|p| p.0), Some(2), "the line in the parameter's code");
    // The code changes while a compile of it runs: that compile is dropped, and the new code has no problem.
    app.sent.push(Msg::run("font.customParameters.set", param("liga; sub a by b;\nsub c by yyy;")));
    app.drain();
    assert_eq!(problem(&app), None, "the problem goes with its code");
    step(&mut app, 20.0);
    step(&mut app, 20.6);
    app.sent.push(Msg::run("font.customParameters.set", param("liga; sub a by b;")));
    app.drain();
    step(&mut app, 20.7);
    app.drain();
    assert_eq!(problem(&app), None);
    step(&mut app, 21.3);
    step(&mut app, 21.4);
    app.drain();
    assert_eq!(problem(&app), None);
    let known = app.view.font_info.exports.problems.clone().expect("the last compile landed");
    assert_eq!(known.revision, app.session.doc().expect("a document").revision);
}

/// Typing a name in the code asks for the names that fit at the caret.
#[test]
fn typing_a_name_asks_for_completions() {
    let (mut fixture, posted) = panel(features_model());
    let code = fixture.by_label("Feature Code").expect("the code field");
    fixture.click(&code).key(Key::A, Modifiers::COMMAND).type_text("sub a").settle();
    let ask = CompletionAsk { key: key(EntryKind::Feature, "ss01", 1), code: "sub a".into(), position: 5 };
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureCompletion(Some(ask)))), "{:?}", posted.borrow());
}

/// A snippet goes in place of the code field's selection, which the field keeps after the Snippets button takes the
/// keyboard.
#[test]
fn a_snippet_replaces_the_selected_code() {
    let mut model = features_model();
    model.features.snippets.built_in = vec!["Ligature".into()];
    let (mut fixture, posted) = panel(model);
    let code = fixture.by_label("Feature Code").expect("the code field");
    fixture.click(&code).key(Key::A, Modifiers::COMMAND).settle();
    let button = fixture.by_label("Snippets").expect("the Snippets button");
    fixture.click(&button).settle();
    let snippet = fixture.by_label("Ligature").expect("the snippet");
    fixture.click(&snippet).settle();
    let insert = posted.borrow().iter().find(|m| m.command_id() == Some("font.features.insertSnippet")).cloned();
    assert_eq!(
        insert,
        Some(Msg::run(
            "font.features.insertSnippet",
            json!({ "kind": "feature", "name": "ss01", "index": 1, "snippet": "Ligature", "position": 0, "end": 11 })
        ))
    );
}

/// Typed code is committed with `font.features.setCode` when the field loses the keyboard; the automatic box, a Fix
/// button and a stylistic set name post their commands.
#[test]
fn features_entry_pane_posts_code_fixes_and_names() {
    let (mut fixture, posted) = panel(features_model());
    let mark = fixture.by_label("Problems on line 1 (a fix is available)").expect("the line's icon");
    fixture.click(&mark).settle();
    let fix = fixture.by_label("Fix: Remove").expect("the Fix button");
    fixture.click(&fix).settle();
    let code = fixture.by_label("Feature Code").expect("the code field");
    fixture.click(&code).key(Key::A, Modifiers::COMMAND).type_text("sub b by c;").settle();
    let notes = fixture.by_label("Notes").expect("the notes field");
    fixture.click(&notes).settle();
    let automatic = fixture.by_label("Generate Feature Automatically").expect("the box");
    fixture.click(&automatic).settle();
    let minus = fixture.by_label("Remove this name").expect("the name's minus");
    fixture.click(&minus).settle();
    let entry = json!({ "kind": "feature", "name": "ss01", "index": 1 });
    let with = |k: &str, v: Value| {
        let mut p = entry.clone();
        p[k] = v;
        p
    };
    let got = posted.borrow().clone();
    assert!(got.contains(&Msg::run("font.features.setCode", with("code", json!("sub b by c;")))), "{got:?}");
    assert!(got.contains(&Msg::run("font.features.setAutomatic", with("automatic", json!(true)))), "{got:?}");
    assert!(got.contains(&Msg::run("font.features.applyFix", json!({ "error": 7, "fix": 0 }))), "{got:?}");
    assert!(got.contains(&Msg::run("font.features.removeLabel", json!({ "tag": "ss01", "index": 1, "language": "dflt" }))), "{got:?}");
}

/// A disabled entry's name is struck through: one rule across the middle of its row, none on the enabled rows.
#[test]
fn a_disabled_feature_is_struck_through() {
    let mut model = features_model();
    if let Some(group) = model.features.groups.iter_mut().find(|g| g.kind == EntryKind::Feature)
        && let Some(liga) = group.entries.first_mut()
    {
        liga.disabled = true;
    }
    let (mut fixture, _) = panel(model);
    let rules_in = |shapes: &[egui::Shape], row: egui::Rect| {
        shapes
            .iter()
            .filter(|shape| {
                matches!(shape, egui::Shape::LineSegment { points: [a, b], .. }
                    if row.contains(*a) && row.contains(*b) && (a.y - b.y).abs() < f32::EPSILON && (a.y - row.center().y).abs() < 1.0)
            })
            .count()
    };
    let liga = fixture.by_label("liga").expect("the liga entry").rect;
    let ss01 = fixture.by_label("ss01").expect("the ss01 entry").rect;
    let shapes = fixture.frame_shapes();
    assert_eq!(rules_in(&shapes, liga), 1);
    assert_eq!(rules_in(&shapes, ss01), 0);
}

/// Copy, Paste and Select All fired while a group's list has the keyboard focus reach the list, which posts the entry
/// commands; Cut stays with the document.
#[test]
fn edit_commands_reach_the_focused_sidebar_list() {
    use craft_ext::ui::list::{EditCommand, deliver};
    let (mut fixture, posted) = panel(features_model());
    let liga = fixture.by_label("liga").expect("the liga entry");
    fixture.click(&liga).settle();
    posted.borrow_mut().clear();
    let ctx = fixture.ctx().clone();
    for edit in [EditCommand::Copy, EditCommand::Paste, EditCommand::SelectAll] {
        assert!(deliver(&ctx, edit), "{edit:?}");
        fixture.settle();
    }
    assert!(!deliver(&ctx, EditCommand::Cut));
    let ss01 = json!({ "kind": "feature", "name": "ss01", "index": 1 });
    let all = json!([{ "kind": "feature", "name": "liga", "index": 0 }, ss01.clone()]);
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("font.features.copyEntries", json!({ "entries": [ss01] })),
            Msg::Shell(CommandRef::new("font.info.features.paste")),
            shell("font.info.features.select", json!({ "entries": all })),
        ]
    );
}

/// The picker adds the selected feature, automatic when Update can write it; a tag the font has cannot be added.
#[test]
fn feature_picker_adds_the_selected_feature_and_cancel_adds_nothing() {
    let mut picker = FeaturePicker::default();
    picker.fill(&json!({ "features": [
        { "tag": "aalt", "name": "Access All Alternates", "inFont": true },
        { "tag": "smcp", "name": "Small Capitals", "canGenerate": true },
    ] }));
    assert_eq!(picker.selection.items, vec![1], "the first feature the font lacks is selected");
    assert_eq!(picker.command(0), None);
    let answered = |press: &'static str| {
        let dialog = Rc::new(RefCell::new(FeaturePicker::default()));
        dialog.borrow_mut().fill(&json!({ "features": [{ "tag": "smcp", "name": "Small Capitals", "canGenerate": true }] }));
        let result = Rc::new(RefCell::new(None));
        let (held, out) = (Rc::clone(&dialog), Rc::clone(&result));
        let mut fixture = Fixture::new(move |ui| {
            if let Some(r) = held.borrow_mut().show(ui, &DialogStack::default()) {
                *out.borrow_mut() = Some(r);
            }
        });
        fixture.settle();
        let button = fixture.by_label(press).expect("the button");
        fixture.click(&button).settle();
        result.borrow_mut().take()
    };
    match answered("Add") {
        Some(DialogResult::Ok(msg)) => assert_eq!(msg, Msg::run("font.features.addFeature", json!({ "tag": "smcp", "automatic": true }))),
        other => panic!("Add answered {other:?}"),
    }
    assert!(matches!(answered("Cancel"), Some(DialogResult::Cancel)));
}

fn exports_model() -> FontInfoModel {
    let exports = ExportsModel {
        entries: vec![
            ExportEntry { name: "Regular".into(), active: true, variable: false },
            ExportEntry { name: "Wide".into(), active: false, variable: false },
        ],
        selected: vec![0],
        plus: vec![true, false, false, false],
        fields: Some(ExportFields {
            active: Flag { on: true, mixed: false },
            name: "Regular".into(),
            statics: vec![0],
            weight: Some(400.0),
            width: Some(5.0),
            axes: vec![("Weight".into(), Some(400.0))],
            warnings: vec!["Beyond the masters".into()],
            ..ExportFields::default()
        }),
        ..ExportsModel::default()
    };
    FontInfoModel { open: true, tab: FontInfoTab::Exports, exports, ..FontInfoModel::default() }
}

/// Two selected entries dragged past the third move together in one command.
#[test]
fn selected_exports_drag_together() {
    let mut model = exports_model();
    model.exports.entries.push(ExportEntry { name: "Black".into(), active: true, variable: false });
    model.exports.selected = vec![0, 1];
    let (mut fixture, posted) = panel(model);
    let first = fixture.by_label("Regular").expect("the first entry");
    let last = fixture.by_label("Black").expect("the last entry");
    let below = egui::pos2(last.center().x, last.rect.bottom() - last.rect.height() / 4.0);
    fixture.drag(&first, below - first.center()).settle();
    let want = Msg::run("font.instance.reorder", json!({ "instances": [0, 1], "toIndex": 1 }));
    assert!(posted.borrow().contains(&want), "{:?}", posted.borrow());
}

/// An axis dragged above another in the axis list posts the new order.
#[test]
fn axes_reorder_by_drag() {
    use crate::app::models::font_info::AxisModel;
    let mut model = FontInfoModel { open: true, ..FontInfoModel::default() };
    let axis = |name: &str, tag: &str| AxisModel { name: name.into(), tag: tag.into(), ..AxisModel::default() };
    model.font.axes = vec![axis("Weight", "wght"), axis("Width", "wdth")];
    let (mut fixture, posted) = panel(model);
    let first = fixture.by_label("Weight (wght)").expect("the first axis");
    let last = fixture.by_label("Width (wdth)").expect("the last axis");
    let above = egui::pos2(first.center().x, first.rect.top() + first.rect.height() / 4.0);
    fixture.drag(&last, above - last.center()).settle();
    assert!(posted.borrow().contains(&Msg::run("font.axes.reorder", json!({ "order": [1, 0] }))), "{:?}", posted.borrow());
}

/// At the panel's default size the Masters tab's fields scroll to their last section while the list stays, and Delete
/// on the focused list posts the removal the minus posts.
#[test]
fn the_masters_tab_scrolls_its_fields_under_a_list_that_stays() {
    use craft::ui::testing::pointer;

    use crate::app::models::font_masters::{MasterItem, MastersTabModel, MetricItem};
    use crate::app::models::layers::MasterIcon;
    let item = |name: &str| MasterItem { name: name.into(), icon: MasterIcon::Builtin { weight: 1, width: 2 } };
    let metric = |n: usize| MetricItem { label: format!("Metric {n}"), position: Some(0.0), zone: Some(0.0), ..MetricItem::default() };
    let masters_tab = MastersTabModel {
        masters: vec![item("Light"), item("Bold")],
        selected: vec![1],
        metrics: (0..24).map(metric).collect(),
        ..MastersTabModel::default()
    };
    let (mut fixture, posted) = panel(FontInfoModel { open: true, tab: FontInfoTab::Masters, masters_tab, ..FontInfoModel::default() });
    fixture.room(egui::vec2(780.0, 560.0)).settle();
    let before = fixture.by_label("Light").expect("the list's first entry").rect;
    let over = fixture.by_label("Metric 3").map_or(egui::pos2(500.0, 300.0), |t| t.rect.center());
    fixture.frame(vec![pointer::moved(over)]).frame(vec![pointer::wheel(-5000.0)]).settle();
    let plus = fixture.by_label("Add a number value").expect("the last section's plus").rect;
    let shown = fixture.shown();
    assert!(plus.bottom() <= shown.bottom() && plus.top() >= shown.top(), "the last plus at {plus:?}, the room {shown:?}");
    assert!(plus.right() <= 780.0, "the fields fit the panel's width: {plus:?}");
    let light = fixture.by_label("Light").expect("the list's first entry");
    assert_eq!(light.rect, before, "the list stays");
    posted.borrow_mut().clear();
    let bold = fixture.by_label("Bold").expect("the selected entry");
    fixture.click(&bold).key(Key::Delete, Modifiers::NONE).settle();
    assert!(posted.borrow().contains(&Msg::run("font.master.remove", json!({ "masters": [1] }))), "{:?}", posted.borrow());
}

/// The masters and exports a run adds or moves become the selection, so the next edit goes to them and not to their
/// originals.
#[test]
fn added_and_moved_entries_are_selected() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    let run = |app: &mut FontCraftApp, id: &str, params: Value| {
        app.sent.push(Msg::run(id, params));
        app.drain();
    };
    app.sent.push(shell("font.info", json!({ "tab": "masters" })));
    run(&mut app, "font.master.add", json!({ "name": "Bold" }));
    let count = app.session.font().expect("a font").masters.len();
    assert_eq!(app.models.font_info.masters_tab.selected, vec![count - 1], "the added master");
    run(&mut app, "font.master.duplicate", json!({ "masters": [0, count - 1] }));
    let copies = app.models.font_info.masters_tab.selected.clone();
    assert_eq!(copies, vec![1, count + 1], "the copies, each after its original");
    run(&mut app, "font.master.reorder", json!({ "masters": copies, "toIndex": 0 }));
    assert_eq!(app.models.font_info.masters_tab.selected, vec![0, 1], "the moved masters");
    run(&mut app, "font.master.remove", json!({ "masters": [0, 1] }));
    assert_eq!(app.models.font_info.masters_tab.selected, vec![0], "the first master after a removal");

    app.sent.push(shell("font.info.selectTab", json!({ "tab": "exports" })));
    run(&mut app, "font.instance.add", json!({}));
    let had = app.session.font().expect("a font").instances.len();
    run(&mut app, "font.instance.addForEachMaster", json!({}));
    let added: Vec<usize> = (had..app.session.font().expect("a font").instances.len()).collect();
    assert_eq!(app.models.font_info.exports.selected, added);
    run(&mut app, "font.instance.addVariableFontSetting", json!({}));
    assert_eq!(app.models.font_info.exports.selected, vec![had + added.len()]);
}

/// Three parameters of one export are copied and pasted on five others as one undo step, through the same commands the
/// Edit menu's Copy and Paste run; Cmd+Delete removes the selected rows.
#[test]
fn rows_copied_from_one_export_paste_on_five() {
    use crate::app::clipboard::{PasteKind, paste};
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    let send = |app: &mut FontCraftApp, msg: Msg| {
        app.sent.push(msg);
        app.drain();
        app.drain();
    };
    send(&mut app, shell("font.info", json!({ "tab": "exports" })));
    for _ in 0..6 {
        send(&mut app, Msg::run("font.instance.add", json!({})));
    }
    let first = app.session.font().expect("a font").instances.len() - 6;
    for (name, value) in [("Filter", "RemoveOverlap"), ("Filter", "AddExtremes"), ("fileName", "Sample")] {
        let params = json!({ "scope": "instance", "targets": [first], "name": name, "value": value });
        send(&mut app, Msg::run("font.customParameters.add", params));
    }
    send(&mut app, shell("font.info.exports.select", json!({ "instances": [first] })));
    let rows =
        json!([{ "kind": "param", "key": "Filter" }, { "kind": "param", "key": "Filter", "index": 1 }, { "kind": "param", "key": "fileName" }]);
    send(&mut app, shell("font.info.rows.select", json!({ "scope": "instance", "rows": rows })));
    send(&mut app, shell("font.info.rows.copy", json!({})));
    let text = app.clipboard.out.take().expect("the rows on the clipboard");
    let others: Vec<usize> = (first + 1..first + 6).collect();
    send(&mut app, shell("font.info.exports.select", json!({ "instances": others })));
    let counts =
        |app: &FontCraftApp| -> Vec<usize> { app.session.font().expect("a font").instances.iter().skip(first).map(|i| i.params.0.len()).collect() };
    for kind in [PasteKind::InfoRows, PasteKind::Paste] {
        paste(&mut app, &text, kind);
        app.drain();
        assert_eq!(counts(&app), vec![3; 6], "{kind:?}: {}", app.status);
        send(&mut app, Msg::run("edit.undo", json!({})));
        assert_eq!(counts(&app), vec![3, 0, 0, 0, 0, 0], "one undo step");
    }
    send(&mut app, shell("font.info.exports.select", json!({ "instances": [first] })));
    send(&mut app, shell("font.info.rows.delete", json!({})));
    assert_eq!(counts(&app), vec![0; 6]);
}

/// Cut removes no row when the copy of all of them fails, and the rows selected in one document are none of another's.
#[test]
fn a_cut_that_copies_nothing_keeps_the_rows_and_another_document_has_none_selected() {
    use crate::app::models::font_info::RowMark;
    use fontcraft_engine::model::Font;
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    let send = |app: &mut FontCraftApp, msg: Msg| {
        app.sent.push(msg);
        app.drain();
        app.drain();
    };
    let held = |app: &FontCraftApp| app.session.font().expect("a font").info.params.0.len();
    let file_name = || json!({ "scope": "font", "name": "fileName", "value": "Sample" });
    send(&mut app, shell("font.info", json!({ "tab": "font" })));
    send(&mut app, Msg::run("font.customParameters.add", file_name()));
    send(&mut app, Msg::run("font.customParameters.add", json!({ "scope": "font", "name": "glyphOrder", "value": ["a"] })));
    let rows = json!([{ "kind": "param", "key": "fileName" }, { "kind": "param", "key": "glyphOrder" }]);
    send(&mut app, shell("font.info.rows.select", json!({ "scope": "font", "rows": rows })));
    assert!(app.models.font_info.font.params.iter().all(|r| r.mark != RowMark::None));
    // Behind the projection's back: the copy names a row the font no longer has.
    app.session.execute("font.customParameters.remove", &json!({ "scope": "font", "name": "glyphOrder" })).expect("removed");
    send(&mut app, shell("font.info.rows.cut", json!({})));
    assert_eq!((held(&app), app.clipboard.out.is_none()), (1, true), "{}", app.status);
    send(&mut app, shell("font.info.rows.cut", json!({})));
    assert_eq!((held(&app), app.clipboard.out.is_some()), (0, true), "{}", app.status);

    send(&mut app, Msg::run("edit.undo", json!({})));
    assert!(app.models.font_info.font.params.iter().any(|r| r.mark != RowMark::None), "the row is back, still selected");
    app.session.add_document(Font::new("Other Sans"), None).expect("a second document");
    send(&mut app, Msg::run("font.customParameters.add", file_name()));
    assert!(app.models.font_info.font.params.iter().all(|r| r.mark == RowMark::None));
    send(&mut app, shell("font.info.rows.delete", json!({})));
    assert_eq!(held(&app), 1, "no row is selected");
}

/// A parameter only the second of two selected masters holds shows with its holder; its minus acts on that master, and
/// editing it adds it to the other. A language change over several exports is one command.
#[test]
fn a_row_only_one_target_holds_shows_and_is_edited_on_both() {
    use super::param_value::Edit;
    use super::rows::{RowAction, commands};
    use crate::app::models::font_info::RowModel;
    use crate::shell::font_info::Target;
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(shell("font.info", json!({ "tab": "masters" })));
    app.sent.push(Msg::run("font.master.add", json!({ "name": "Bold" })));
    app.sent.push(Msg::run("font.customParameters.add", json!({ "scope": "master", "targets": [1], "name": "typoAscender", "value": 800 })));
    app.sent.push(shell("font.info.masters.select", json!({ "masters": [0, 1] })));
    app.drain();
    let (target, row) = {
        let tab = &app.models.font_info.masters_tab;
        (tab.target(), tab.params.iter().find(|r| r.key == "typoAscender").cloned().expect("the second master's row"))
    };
    assert_eq!((row.holders.clone(), row.mixed), (vec![1], false));
    let remove = json!({ "scope": "master", "targets": [1], "name": "typoAscender", "index": 0 });
    assert_eq!(commands(&target, &row, &RowAction::Remove), vec![Msg::run("font.customParameters.remove", remove)]);
    app.sent.extend(commands(&target, &row, &RowAction::Edit(Edit::Value(json!(750)))));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    let held: Vec<usize> = app.session.font().expect("a font").masters.iter().map(|m| m.params.0.len()).collect();
    assert_eq!(held, vec![1, 1]);

    let exports = Target { scope: fontcraft_engine::fontinfo::Scope::Instance, indices: vec![0, 1, 2, 3] };
    let names = RowModel { key: "styleNames".into(), ..RowModel::default() };
    let change = Edit::ChangeLanguage { from: "DEU".into(), to: "FRA".into() };
    let want = json!({ "scope": "instance", "targets": [0, 1, 2, 3], "key": "styleNames", "from": "DEU", "language": "FRA" });
    assert_eq!(commands(&exports, &names, &RowAction::Edit(change)), vec![Msg::run("font.info.setLocalizedValue", want)]);
}

/// Cmd-click adds an entry to the selection; Active, Style Name, an axis coordinate, Bold and minus post their
/// commands on the selected entries.
#[test]
fn exports_tab_selects_and_edits_through_its_commands() {
    let (mut fixture, posted) = panel(exports_model());
    assert!(fixture.texts().iter().any(|t| t.text == "Beyond the masters"), "the extrapolation warning shows");
    let wide = fixture.by_label("Wide").expect("the Wide entry");
    fixture.hold(Modifiers::COMMAND).click(&wide).hold(Modifiers::NONE).settle();
    let active = fixture.by_label("Active").expect("the Active box");
    fixture.click(&active).settle();
    let name = fixture.by_label("Style Name").expect("the Style Name field");
    fixture.click(&name).key(Key::A, Modifiers::COMMAND).type_text("Book").key(Key::Enter, Modifiers::NONE).settle();
    let weight = fixture.by_label("Weight").expect("the Weight coordinate");
    fixture.click(&weight).key(Key::A, Modifiers::COMMAND).type_text("350").key(Key::Enter, Modifiers::NONE).settle();
    let remove = fixture.by_label("Remove the selected exports").expect("the minus");
    fixture.click(&remove).settle();
    let got = posted.borrow().clone();
    for want in [
        shell("font.info.exports.select", json!({ "instances": [0, 1] })),
        Msg::run("font.instance.setActive", json!({ "instances": [0], "active": false })),
        Msg::run("font.instance.set", json!({ "instances": [0], "styleName": "Book" })),
        Msg::run("font.instance.setAxisLocation", json!({ "instances": [0], "location": [350.0] })),
        Msg::run("font.instance.remove", json!({ "instances": [0] })),
    ] {
        assert!(got.contains(&want), "{want:?} missing from {got:?}");
    }
}

/// The selection shell commands and the projections: the Features tab shows the selected entry with its code, the
/// Exports tab the selected instances' fields.
#[test]
fn the_tabs_project_the_selected_entries() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false })));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "ss01", "automatic": false })));
    app.sent.push(shell("font.info", json!({ "tab": "features" })));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    let features = &app.models.font_info.features;
    assert_eq!(features.selected.first().map(|k| k.name.as_str()), Some("liga"), "the first entry shows by default");
    app.sent.push(shell("font.info.features.select", json!({ "entries": [{ "kind": "feature", "name": "ss01", "index": 1 }] })));
    app.drain();
    let entry = app.models.font_info.features.entry.clone().expect("the shown entry");
    assert_eq!(entry.key, key(EntryKind::Feature, "ss01", 1));
    assert!(entry.stylistic_set);

    app.sent.push(Msg::run("font.instance.add", json!({})));
    app.sent.push(Msg::run("font.instance.add", json!({})));
    app.sent.push(shell("font.info.selectTab", json!({ "tab": "exports" })));
    app.sent.push(shell("font.info.exports.select", json!({ "instances": [1, 0, 9] })));
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    let exports = &app.models.font_info.exports;
    assert_eq!(exports.entries.len(), 2);
    assert_eq!(exports.selected, vec![0, 1], "an index the font lacks is dropped");
    assert!(exports.fields.is_some());
    assert_eq!(app.models.font_info.features, FeaturesModel::default(), "the Features tab is not projected while hidden");
}

/// The warnings `font.info.get` and `font.instance.list` give land in the model under their fields: the family name
/// on the Font tab, the weight class of the selected export on the Exports tab.
#[test]
fn font_info_warnings_project_by_field() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::run("font.info.set", json!({ "scope": "font", "values": { "familyName": "Sämple" } })));
    app.sent.push(Msg::run("font.instance.add", json!({})));
    app.sent.push(Msg::run("font.instance.setWeightClass", json!({ "instances": [0], "weightClass": 450 })));
    app.sent.push(shell("font.info.selectTab", json!({ "tab": "exports" })));
    app.drain();
    let font = &app.models.font_info.font;
    assert!(font.warnings.iter().any(|w| w.field == "familyName"), "{:?}", font.warnings);
    let fields = app.models.font_info.exports.fields.clone().expect("the selected export's fields");
    assert!(fields.field_warnings.iter().any(|w| w.field == "weightClass"), "{:?}", fields.field_warnings);
}

/// A right click on a selected sidebar entry opens its menu: Disable, Copy and Paste post their commands on the
/// selected entries.
#[test]
fn the_sidebar_context_menu_disables_copies_and_pastes() {
    let (mut fixture, posted) = panel(features_model());
    let mut pick = |item: &str| {
        let at = fixture.by_label("ss01").expect("the ss01 entry").center();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
        fixture.frame(vec![egui::Event::PointerMoved(at), button(true)]).frame(vec![button(false)]).settle();
        let row = fixture.by_label(item).unwrap_or_else(|| panic!("the context menu's {item}"));
        fixture.click(&row).settle();
    };
    pick("Disable");
    pick("Copy");
    pick("Paste");
    let ss01 = json!([{ "kind": "feature", "name": "ss01", "index": 1 }]);
    let got = posted.borrow().clone();
    for want in [
        Msg::run("font.features.setDisabled", json!({ "entries": ss01, "disabled": true })),
        Msg::run("font.features.copyEntries", json!({ "entries": ss01 })),
        shell("font.info.features.paste", json!(null)),
    ] {
        assert!(got.contains(&want), "{want:?} missing from {got:?}");
    }
}

/// Copy puts `font.features.copyEntries`' text on the clipboard; Paste asks for the clipboard's text and pastes it with
/// `font.features.pasteEntries`, which adds the entries again.
#[test]
fn copied_feature_entries_paste_back() {
    use crate::app::clipboard::{self, PasteKind};
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false })));
    app.sent.push(Msg::run("font.features.setCode", json!({ "kind": "feature", "name": "liga", "code": "sub f i by f_i;" })));
    app.sent.push(Msg::run("font.features.copyEntries", json!({ "entries": [{ "kind": "feature", "name": "liga", "index": 0 }] })));
    app.drain();
    let text = app.clipboard.out.clone().expect("the copied entries on the clipboard");
    app.sent.push(Msg::Shell(CommandRef::new("font.info.features.paste")));
    app.drain();
    assert_eq!(app.clipboard.request, Some(PasteKind::FeatureEntries), "Paste asks for the clipboard's text");
    clipboard::paste(&mut app, &text, PasteKind::FeatureEntries);
    app.drain();
    assert!(app.status.is_empty(), "{}", app.status);
    assert_eq!(app.session.font().map(|f| f.features.features.len()).unwrap_or(0), 2, "the pasted feature joins the font");
}

/// An inserted snippet leaves the caret after itself: the shown entry carries the `caret` of
/// `font.features.insertSnippet`'s result until the code editor reports it placed.
#[test]
fn an_inserted_snippet_leaves_the_caret_after_it() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false })));
    app.sent.push(Msg::run("font.features.setCode", json!({ "kind": "feature", "name": "liga", "code": "sub a by b;" })));
    app.drain();
    let list = app.session.query("font.features.listSnippets", &json!({})).expect("the snippets");
    let first = list.get("builtIn").and_then(|b| b.get(0)).cloned().expect("a built-in snippet");
    let title = first.get("title").and_then(Value::as_str).expect("its title").to_string();
    let code = first.get("code").and_then(Value::as_str).expect("its code").to_string();
    app.sent.push(Msg::run(
        "font.features.insertSnippet",
        json!({ "kind": "feature", "name": "liga", "index": 0, "snippet": title, "position": 0, "end": 0 }),
    ));
    app.drain();
    let caret = app.view.font_info.features.caret.as_ref().map(|(key, at)| (key.name.clone(), *at));
    assert_eq!(caret, Some(("liga".to_string(), code.chars().count())));
    app.sent.push(Msg::View(ViewMsg::FeatureCaretPlaced));
    app.drain();
    assert_eq!(app.view.font_info.features.caret, None);
}

/// The painted code of the Features tab whose text is `code`, with where it was painted.
fn painted_code(fixture: &mut Fixture, code: &str) -> (egui::Pos2, Arc<egui::Galley>) {
    fixture
        .frame_shapes()
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(t) if t.galley.job.text == code => Some((t.pos, t.galley.clone())),
            _ => None,
        })
        .expect("the code is painted")
}

/// The features model showing `code` with the compile's `diagnostics`.
fn showing(code: &str, diagnostics: Vec<DiagnosticModel>) -> FontInfoModel {
    let mut model = features_model();
    if let Some(e) = model.features.entry.as_mut() {
        e.code = code.into();
        e.diagnostics = diagnostics;
    }
    model
}

/// The gutter marks the diagnostic's line with an icon beside that line, and no other; the list under the code is
/// gone, and the icon opens the details, whose Fix posts the same `font.features.applyFix` as the command takes.
#[test]
fn the_gutter_marks_the_diagnostics_line_and_its_icon_opens_the_fix() {
    let code = "sub a by b;\nsub c by d;";
    let found =
        DiagnosticModel { id: 3, error: true, message: "unknown glyph".into(), line: 2, range: Some((16, 17)), fixes: vec!["Use c.sc".into()] };
    let (mut fixture, posted) = panel(showing(code, vec![found]));
    assert!(fixture.by_label("Fix: Use c.sc").is_none(), "no list under the code");
    assert!(fixture.by_label("Problems on line 1").is_none());
    let mark = fixture.by_label("Problems on line 2 (a fix is available)").expect("an icon at line 2");
    let (pos, galley) = painted_code(&mut fixture, code);
    let second = galley.rows.get(1).expect("two rows").rect().translate(pos.to_vec2());
    assert!(second.y_range().contains(mark.rect.center().y), "the icon {:?} sits beside line 2 {second:?}", mark.rect);
    fixture.click(&mark).settle();
    let fix = fixture.by_label("Fix: Use c.sc").expect("the details show the Fix");
    fixture.click(&fix).settle();
    assert_eq!(*posted.borrow(), vec![Msg::run("font.features.applyFix", json!({ "error": 3, "fix": 0 }))]);
}

/// A line gets one mark: an error icon if any of its diagnostics is an error, a wrench only if one has a fix, and the
/// first message; the icon's name says when a fix is available.
#[test]
fn the_wrench_shows_only_where_a_fix_exists() {
    let d = |id: u64, error: bool, line: u64, fixes: &[&str]| DiagnosticModel {
        id,
        error,
        message: format!("problem {id}"),
        line,
        range: None,
        fixes: fixes.iter().map(|f| (*f).to_string()).collect(),
    };
    let found = vec![d(1, false, 3, &[]), d(2, true, 1, &[]), d(3, true, 3, &["Fix it"]), d(4, false, 0, &["Nowhere"])];
    let got = super::code_editor::marks(&found);
    let want = vec![
        super::code_editor::LineMark { line: 1, error: true, fixable: false, message: "problem 2" },
        super::code_editor::LineMark { line: 3, error: true, fixable: true, message: "problem 1" },
    ];
    assert_eq!(got, want);

    let (fixture, _) = panel(showing("sub a by b;\nsub c by d;\nsub e by f;", found.clone()));
    assert!(fixture.by_label("Problems on line 1").is_some(), "no fix, no wrench");
    assert!(fixture.by_label("Problems on line 1 (a fix is available)").is_none());
    assert!(fixture.by_label("Problems on line 3 (a fix is available)").is_some());
}

/// Each line of the code gets a number in the gutter; the rows a long line wraps onto get none.
#[test]
fn wrapped_lines_get_no_number() {
    let long = "sub a by b; ".repeat(30);
    let code = format!("{long}\nsub c by d;");
    let (mut fixture, _) = panel(showing(&code, Vec::new()));
    let shapes = fixture.frame_shapes();
    let (pos, galley) = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(t) if t.galley.job.text == code => Some((t.pos, t.galley.clone())),
            _ => None,
        })
        .expect("the code is painted");
    assert!(galley.rows.len() > 2, "the first line wraps: {} rows", galley.rows.len());
    let mut numbers: Vec<String> = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Text(t) if t.pos.x < pos.x && t.galley.job.text.parse::<u32>().is_ok() => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    numbers.sort();
    assert_eq!(numbers, vec!["1".to_string(), "2".to_string()]);
}

/// Measure: a frame of the Features tab showing a 256 KB entry with an error takes under 16 ms in a release build.
#[test]
#[ignore = "a measure: run with --release --ignored"]
fn a_frame_with_a_256_kb_entry_takes_under_16_ms() {
    let line = "sub a by b; # a statement of the measure\n";
    let code = line.repeat(256 * 1024 / line.len());
    let found = DiagnosticModel { id: 1, error: true, message: "unknown glyph".into(), line: 100, range: Some((4, 5)), fixes: vec!["Fix".into()] };
    let (mut fixture, _) = panel(showing(&code, vec![found]));
    let mut times: Vec<std::time::Duration> = (0..5)
        .map(|_| {
            let start = std::time::Instant::now();
            fixture.frame_shapes();
            start.elapsed()
        })
        .collect();
    times.sort();
    let median = times[2];
    eprintln!("a frame with a {} byte entry: {median:?} median of 5", code.len());
    if !cfg!(debug_assertions) {
        assert!(median < std::time::Duration::from_millis(16), "{median:?}");
    }
}

/// Alt-click on a glyph name in the code asks its preview, naming the characters it covers.
#[test]
fn alt_click_on_a_name_asks_its_preview() {
    let (mut fixture, posted) = panel(features_model());
    let (pos, galley) = painted_code(&mut fixture, "sub a by b;");
    let at = galley.pos_from_cursor(egui::text::CCursor::new(4)).translate(pos.to_vec2()).center() + egui::vec2(2.0, 0.0);
    fixture.hold(Modifiers::ALT).click_at(at).hold(Modifiers::NONE).settle();
    let ask = PreviewAsk { key: key(EntryKind::Feature, "ss01", 1), name: "a".into(), start: 4, end: 5, token: false };
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeaturePreview(Some(ask)))), "{:?}", posted.borrow());
}

/// The name under a click keeps a leading `@` or backslash and stops at punctuation and spaces.
#[test]
fn the_clicked_name_takes_name_characters_on_both_sides() {
    use super::feature_previews::name_at;
    let code = "sub @Low' \\a.sc by [b-c];";
    let name = |at: usize| name_at(code, at).map(|(s, e)| code.chars().skip(s).take(e - s).collect::<String>());
    assert_eq!(name(6).as_deref(), Some("@Low"));
    assert_eq!(name(12).as_deref(), Some("\\a.sc"));
    assert_eq!(name(21).as_deref(), Some("b-c"));
    assert_eq!(name(9), None);
}

/// A class preview lists its members, or the anchors of its markClass statements, and names unknown members; a lookup
/// preview lists its rules and says when more follow.
#[test]
fn a_preview_lists_class_members_anchors_and_lookup_rules() {
    use super::feature_previews::{PreviewWords, preview_lines};
    use crate::app::models::font_features::PreviewModel;
    let words = PreviewWords::default();
    let class = PreviewModel {
        name: "@Top".into(),
        kind: "class".into(),
        glyphs: vec!["a".into(), "b".into()],
        total: 600,
        unknown: vec!["zz".into()],
        ..PreviewModel::default()
    };
    assert_eq!(preview_lines(&class, &words), ["Class @Top: 600 glyphs, first drawn: 2", "Not in the font: zz"]);
    let marks = PreviewModel {
        name: "@MARKS".into(),
        kind: "class".into(),
        glyphs: vec!["acutecomb".into()],
        total: 1,
        anchors: vec![("<anchor 0 500>".into(), "acutecomb".into())],
        ..PreviewModel::default()
    };
    assert_eq!(preview_lines(&marks, &words), ["Class @MARKS: 1 glyphs", "Anchor <anchor 0 500>: acutecomb"]);
    let lookup =
        PreviewModel { name: "Smallcaps".into(), kind: "lookup".into(), rules: vec!["sub a by a.sc;".into()], more: true, ..PreviewModel::default() };
    assert_eq!(preview_lines(&lookup, &words), ["Lookup Smallcaps", "sub a by a.sc;", "More rules follow"]);
}

/// Cmd+/ comments three selected lines, and again uncomments them; Tab and Shift+Tab indent and outdent a selection;
/// Return keeps the line's indentation; the bracket beside the caret finds its partner.
#[test]
fn editing_keys_comment_indent_and_keep_indentation() {
    use super::code_editor::{bracket_pair, indent, new_line, toggle_comments};
    use craft_ext::ui::text::TextSelection;
    let code = "sub a by b;\n\tsub c by d;\n\nsub e by f;";
    let all = TextSelection::new(0, code.chars().count());
    let commented = toggle_comments(code, all).expect("an edit");
    assert_eq!(commented.insert, "# sub a by b;\n\t# sub c by d;\n\n# sub e by f;");
    assert_eq!(toggle_comments(&commented.insert, TextSelection::new(0, commented.insert.chars().count())).expect("an edit").insert, code);
    let two = TextSelection::new(2, 14);
    assert_eq!(indent(code, two, true).expect("an edit").insert, "\tsub a by b;\n\t\tsub c by d;");
    assert_eq!(indent(code, two, false).expect("an edit").insert, "sub a by b;\nsub c by d;");
    let ret = new_line(code, TextSelection::caret_at(24));
    assert_eq!((ret.insert.as_str(), ret.select), ("\n\t", TextSelection::caret_at(26)));
    let braces = "feature liga {\n\tsub a by b;\n} liga;";
    assert_eq!(bracket_pair(braces, 14), vec![13, 28]);
    assert!(bracket_pair(braces, 3).is_empty());
}

/// Cmd+/ in the code field comments the selected lines in the code as typed, without a commit.
#[test]
fn cmd_slash_in_the_code_field_comments_the_lines() {
    let (mut fixture, posted) = panel(showing("sub a by b;\nsub c by d;", Vec::new()));
    let code = fixture.by_label("Feature Code").expect("the code field");
    fixture.click(&code).key(Key::A, Modifiers::COMMAND).settle();
    fixture.key(Key::Slash, Modifiers::COMMAND).settle();
    let draft = FeatureDraft { key: key(EntryKind::Feature, "ss01", 1), code: "# sub a by b;\n# sub c by d;".into() };
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureDraft(Some(draft)))), "{:?}", posted.borrow());
    assert!(!posted.borrow().iter().any(|m| m.command_id() == Some("font.features.setCode")), "the key commits nothing");
}

/// The arrows move the caret in the code field and Shift with them selects, also while a selection over lines has
/// Tab claimed for indenting (decision 0009: a field that keeps Tab loses its arrows to focus moves).
#[test]
fn arrows_move_the_caret_in_the_code_field() {
    let (mut fixture, posted) = panel(showing("sub a by b;\nsub c by d;", Vec::new()));
    let code = fixture.by_label("Feature Code").expect("the code field");
    fixture.click(&code).key(Key::A, Modifiers::COMMAND).settle();
    fixture.key(Key::ArrowLeft, Modifiers::NONE).key(Key::ArrowDown, Modifiers::NONE).key(Key::ArrowRight, Modifiers::NONE).settle();
    fixture.key(Key::ArrowRight, Modifiers::SHIFT).type_text("Z").settle();
    let draft = FeatureDraft { key: key(EntryKind::Feature, "ss01", 1), code: "sub a by b;\nsZb c by d;".into() };
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureDraft(Some(draft)))), "{:?}", posted.borrow());
}

/// Next selects the matches in turn and wraps past the last; Replace then replaces the selected one, the first.
#[test]
fn find_next_wraps_and_replace_takes_the_selected_match() {
    use crate::app::models::font_features::FindModel;
    use crate::shell::font_info_tabs::FindState;
    let code = "sub a by b;\nsub a by c;";
    let mut model = showing(code, Vec::new());
    let state = FindState { query: "a".into(), replacement: "x".into(), ..FindState::default() };
    if let Some(e) = model.features.entry.as_mut() {
        e.find = Some(FindModel { state, here: vec![(4, 5), (16, 17)], total: 2, ..FindModel::default() });
    }
    let (mut fixture, posted) = panel(model);
    for _ in 0..3 {
        let next = fixture.by_label("Next").expect("the Next button");
        fixture.click(&next).settle();
    }
    let replace = fixture.by_label("Replace").expect("the Replace button");
    fixture.click(&replace).settle();
    let expected = Msg::run(
        "font.features.replace",
        json!({ "kind": "feature", "name": "ss01", "index": 1, "query": "a", "matchCase": false, "wholeWord": false, "regex": false, "replacement": "x", "code": code, "start": 4 }),
    );
    assert!(posted.borrow().contains(&expected), "{:?}", posted.borrow());
}

/// A Replace Feature parameter is edited as coloured feature code, and the instance compile's problem shows under it
/// with an error icon and its line.
#[test]
fn a_feature_parameter_is_coloured_and_shows_its_compile_problem() {
    use crate::app::models::font_info::{Editor, RowModel, RowSort};
    let text = "liga; sub a by zz;";
    let mut model = exports_model();
    model.exports.params = vec![RowModel {
        sort: RowSort::Param,
        key: "Replace Feature".into(),
        label: "Replace Feature".into(),
        enabled: true,
        removable: true,
        editor: Editor::Code { text: text.into(), problem: Some((1, "glyph not in font".into())) },
        ..RowModel::default()
    }];
    let (mut fixture, _) = panel(model);
    assert!(fixture.by_label("Problem in this parameter's feature code").is_some(), "the error icon");
    let texts: Vec<String> = fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t == "Line 1: glyph not in font"), "{texts:?}");
    let (_, galley) = painted_code(&mut fixture, text);
    let mut colors: Vec<egui::Color32> = galley.job.sections.iter().map(|s| s.format.color).collect();
    colors.dedup();
    assert!(colors.len() > 1, "the code is coloured: {colors:?}");
}

/// At the panel's default size, with the find bar open, the find bar's controls each keep one line and the code field
/// spans the pane beside the gutter.
#[test]
fn the_find_bar_and_the_code_field_fit_the_panels_default_size() {
    use crate::app::models::font_features::FindModel;
    use crate::shell::font_info_tabs::FindState;
    let mut model = showing("sub a by b;", Vec::new());
    if let Some(e) = model.features.entry.as_mut() {
        e.find =
            Some(FindModel { state: FindState { query: "a".into(), ..FindState::default() }, here: vec![(4, 5)], total: 1, ..FindModel::default() });
    }
    let (mut fixture, _) = panel(model);
    let room = egui::vec2(780.0, 560.0);
    fixture.room(room).settle();
    let line = fixture.by_label("Next").expect("the Next button").rect.height();
    for label in ["Match Case", "Whole Word", "Regular Expression", "Close Find", "Replace All"] {
        let rect = fixture.by_label(label).unwrap_or_else(|| panic!("{label}")).rect;
        assert!(rect.height() <= line * 1.5 && rect.right() <= room.x, "{label}: {rect:?}");
    }
    let code = fixture.by_label("Feature Code").expect("the code field").rect;
    let next = fixture.by_label("Next").expect("the Next button").rect;
    assert!(code.width() > room.x / 2.0 && code.top() > next.bottom(), "{code:?}");
}

/// At the panel's default size with the find bar open, the notes field under the code field can be reached: it lies in
/// the room, or the tab scrolls to it.
#[test]
fn the_notes_field_is_reachable_at_the_panels_default_size_with_the_find_bar_open() {
    use craft::ui::runtime::access::Role;
    use craft::ui::testing::pointer;

    use crate::app::models::font_features::FindModel;
    use crate::shell::font_info_tabs::FindState;
    let mut model = showing("sub a by b;", Vec::new());
    if let Some(e) = model.features.entry.as_mut() {
        e.find =
            Some(FindModel { state: FindState { query: "a".into(), ..FindState::default() }, here: vec![(4, 5)], total: 1, ..FindModel::default() });
    }
    let (mut fixture, _) = panel(model);
    let room = egui::vec2(780.0, 560.0);
    fixture.room(room).settle();
    // The notes field is the lowest text field of the pane.
    let notes = |fixture: &Fixture| {
        let mut fields = fixture.by_role(Role::MULTILINE_TEXT_INPUT);
        fields.extend(fixture.by_role(Role::TEXT_INPUT));
        fields.into_iter().max_by(|a, b| a.rect.top().total_cmp(&b.rect.top())).expect("the notes field")
    };
    if notes(&fixture).rect.bottom() > fixture.shown().bottom() {
        let over = fixture.by_label("Generate Feature Automatically").expect("the automatic check box").rect.center();
        fixture.frame(vec![pointer::moved(over)]).frame(vec![pointer::wheel(-2000.0)]).settle();
    }
    let shown = fixture.shown();
    let rect = notes(&fixture).rect;
    assert!(rect.bottom() <= shown.bottom() && rect.top() >= shown.top(), "the notes field at {rect:?}, the room {shown:?}");
    let code = fixture.by_label("Feature Code").expect("the code field").rect;
    assert!(code.width() > room.x / 2.0, "the code field keeps the pane's width: {code:?}");
}

/// Next selects a match while the query keeps the keyboard: the code shows that match on a fill, and only that one.
#[test]
fn next_shows_the_selected_match_while_the_query_has_the_keyboard() {
    use crate::app::models::font_features::FindModel;
    use crate::shell::font_info_tabs::FindState;
    let code = "sub a by b;\nsub a by c;";
    let mut model = showing(code, Vec::new());
    if let Some(e) = model.features.entry.as_mut() {
        let state = FindState { query: "a".into(), ..FindState::default() };
        e.find = Some(FindModel { state, here: vec![(4, 5), (16, 17)], total: 2, ..FindModel::default() });
    }
    let (mut fixture, _) = panel(model);
    let filled = |fixture: &mut Fixture| -> Vec<String> {
        let (_, galley) = painted_code(fixture, code);
        let job = &galley.job;
        job.sections
            .iter()
            .filter(|s| s.format.background != egui::Color32::TRANSPARENT)
            .filter_map(|s| job.text.get(s.byte_range.start.0..s.byte_range.end.0).map(str::to_string))
            .collect()
    };
    assert!(filled(&mut fixture).is_empty(), "no match is selected yet");
    let next = fixture.by_label("Next").expect("the Next button");
    fixture.click(&next).settle();
    assert_eq!(filled(&mut fixture), vec!["a".to_string()]);
}

/// The Features tab with 80 features, the last of them automatic and with an error.
fn many_entries() -> FontInfoModel {
    let mut model = features_model();
    if let Some(group) = model.features.groups.iter_mut().find(|g| g.kind == EntryKind::Feature) {
        group.entries = (0..80).map(|n| row(&format!("f{n:03}"))).collect();
        if let Some(last) = group.entries.last_mut() {
            last.automatic = true;
            last.error = true;
        }
    }
    model
}

/// The sidebar scrolls on its own: with 80 entries scrolled to the end the code field stands where it stood, and the
/// Compile button under the sidebar stays in view.
#[test]
fn the_code_stays_in_view_while_the_sidebar_scrolls() {
    use craft::ui::testing::pointer;
    let (mut fixture, _) = panel(many_entries());
    let before = fixture.by_label("Feature Code").expect("the code field").rect;
    let over = fixture.by_label("f000").expect("the first entry").rect.center();
    fixture.frame(vec![pointer::moved(over)]).frame(vec![pointer::wheel(-4000.0)]).settle();
    let last = fixture.by_label("f079").expect("the last entry").rect;
    let shown = fixture.shown();
    assert!(last.bottom() <= shown.bottom() && last.top() >= shown.top(), "the sidebar scrolled to its end: {last:?}");
    assert_eq!(fixture.by_label("Feature Code").expect("the code field").rect, before);
    let compile = fixture.by_label("Compile").expect("the Compile button").rect;
    assert!(compile.bottom() <= shown.bottom(), "{compile:?}");
}

/// The code field fills the height the rest of the entry pane leaves, and a drag of the line over the notes asks for
/// another height of the notes.
#[test]
fn the_code_fills_the_pane_and_the_notes_line_drags() {
    let (mut fixture, posted) = panel(features_model());
    let code = fixture.by_label("Feature Code").expect("the code field").rect;
    assert!(code.height() > ROOM.y / 2.0, "one line of code fills the pane: {code:?}");
    let line = fixture.by_label("Resize Notes").expect("the line over the notes").rect.center();
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let to = line - egui::vec2(0.0, 60.0);
    fixture.frame(vec![egui::Event::PointerMoved(line)]).frame(vec![button(line, true)]);
    fixture.frame(vec![egui::Event::PointerMoved(to)]).frame(vec![button(to, false)]).settle();
    let heights: Vec<f32> =
        posted.borrow().iter().filter_map(|m| if let Msg::View(ViewMsg::FeatureNotesHeight(h)) = m { Some(*h) } else { None }).collect();
    assert!(heights.last().is_some_and(|h| *h > 100.0), "{heights:?}");
}

/// The update button of an automatic row and the error icon post their commands; neither changes the selection.
#[test]
fn a_rows_update_button_and_error_icon_post_their_commands() {
    let mut model = features_model();
    if let Some(liga) = model.features.groups.iter_mut().find(|g| g.kind == EntryKind::Feature).and_then(|g| g.entries.first_mut()) {
        liga.automatic = true;
    }
    let (mut fixture, posted) = panel(model);
    for label in ["Regenerate this entry", "Show the error the last compile found here"] {
        let part = fixture.by_label(label).unwrap_or_else(|| panic!("the part {label}"));
        fixture.click(&part).settle();
    }
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("font.features.updateEntry", json!({ "kind": "feature", "name": "liga", "index": 0 })),
            Msg::run("font.features.jumpToError", json!({ "kind": "feature", "name": "ss01", "index": 1 })),
        ]
    );
}

/// `font.features.jumpToError` shows the entry with the place of its error for the code editor, which selects it and
/// reports it done; Spec opens the feature's address.
#[test]
fn a_jump_selects_the_entry_and_the_faulty_code_and_spec_opens_its_address() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static OPENED: AtomicBool = AtomicBool::new(false);
    fn opened(url: &str) -> Result<(), String> {
        OPENED.store(url.starts_with("https://"), Ordering::SeqCst);
        Ok(())
    }
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "calt", "automatic": false })));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false })));
    app.sent.push(Msg::run("font.features.setCode", json!({ "kind": "feature", "name": "liga", "code": "# é\nsub A by nonesuch;" })));
    app.sent.push(shell("font.info", json!({ "tab": "features" })));
    app.sent.push(Msg::run("font.features.compile", json!({})));
    app.drain();
    let index = app.session.font().ok().and_then(|f| f.features.features.iter().position(|e| e.name == "liga")).expect("liga");
    app.sent.push(Msg::run("font.features.jumpToError", json!({ "kind": "feature", "name": "liga", "index": index })));
    app.drain();
    let entry = app.models.font_info.features.entry.clone().expect("the shown entry");
    assert_eq!(entry.key.name, "liga");
    let (start, end) = entry.jump.expect("the place of the error");
    let picked: String = entry.code.chars().skip(start).take(end - start).collect();
    assert!(picked.contains("nonesuch") || picked == "A", "{picked:?}");

    let (_fixture, posted) =
        panel(FontInfoModel { open: true, tab: FontInfoTab::Features, features: app.models.font_info.features.clone(), ..FontInfoModel::default() });
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeatureJumped)), "the editor takes the jump");
    app.sent.push(Msg::View(ViewMsg::FeatureJumped));
    app.opener = craft_ext::platform::open::Opener::new(Some(opened), None);
    app.sent.push(Msg::run("font.features.openSpec", json!({ "tag": "liga" })));
    app.drain();
    assert_eq!(app.view.font_info.features.jump, None);
    assert!(OPENED.load(Ordering::SeqCst), "Spec opened the address");
}

/// The filter keeps the entries whose name or code holds its text, ignoring case, and changes nothing in the font; the
/// field posts what is typed.
#[test]
fn the_filter_hides_rows_and_changes_no_data() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    for (tag, code) in [("calt", "sub A by B;"), ("liga", "# Nothing here")] {
        app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": tag, "automatic": false })));
        app.sent.push(Msg::run("font.features.setCode", json!({ "kind": "feature", "name": tag, "code": code })));
    }
    app.sent.push(shell("font.info", json!({ "tab": "features" })));
    app.drain();
    let revision = app.session.active().map(|d| d.revision);
    let listed = |app: &FontCraftApp| -> Vec<String> {
        let groups = &app.models.font_info.features.groups;
        groups.iter().flat_map(|g| g.entries.iter()).filter(|e| !e.filtered).map(|e| e.name.clone()).collect()
    };
    app.sent.push(Msg::View(ViewMsg::FeatureFilter("SUB a".into())));
    app.drain();
    assert_eq!(listed(&app), vec!["calt".to_string()], "matched in its code");
    app.sent.push(Msg::View(ViewMsg::FeatureFilter("lig".into())));
    app.drain();
    assert_eq!(listed(&app), vec!["liga".to_string()], "matched in its name");
    assert_eq!(app.session.active().map(|d| d.revision), revision);

    let mut model = features_model();
    model.features.filter = "liga".into();
    if let Some(ss01) = model.features.groups.iter_mut().find(|g| g.kind == EntryKind::Feature).and_then(|g| g.entries.last_mut()) {
        ss01.filtered = true;
    }
    let (fixture, posted) = panel(model);
    let (liga, ss01) = (fixture.by_label("liga").expect("the row"), fixture.by_label("ss01").expect("the entry shown"));
    assert!(ss01.rect.min.x > liga.rect.max.x, "the row is hidden, its entry stays shown");
    let selects = |m: &Msg| matches!(m, Msg::Shell(c) if c.id == "font.info.features.select");
    assert!(!posted.borrow().iter().any(selects), "{:?}", posted.borrow());
}

/// Measure: a frame of the Features tab with 2,000 entries takes under 16 ms in a release build.
#[test]
#[ignore = "a measure: run with --release --ignored"]
fn a_frame_with_2000_entries_takes_under_16_ms() {
    let mut model = features_model();
    if let Some(group) = model.features.groups.iter_mut().find(|g| g.kind == EntryKind::Feature) {
        group.entries = (0..2_000).map(|n| EntryRow { title: "A Registered Name".into(), automatic: true, ..row(&format!("f{n:04}")) }).collect();
    }
    let (mut fixture, _) = panel(model);
    let mut times: Vec<std::time::Duration> = (0..5)
        .map(|_| {
            let start = std::time::Instant::now();
            fixture.frame_shapes();
            start.elapsed()
        })
        .collect();
    times.sort();
    eprintln!("a frame with 2,000 entries: {:?} median of 5", times[2]);
    if !cfg!(debug_assertions) {
        assert!(times[2] < std::time::Duration::from_millis(16), "{:?}", times[2]);
    }
}

/// Every command that changes feature entries, and an undo, starts the background compile once the document stands still;
/// the notes do not.
#[test]
fn each_change_of_the_entries_starts_a_compile() {
    use crate::shell::feature_work::step;
    use fontcraft_engine::feature_state::SessionFeaturesExt as _;
    let host = Host::builder(APP).jobs(Arc::new(craft::host::InlineJobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    let liga = || json!({ "kind": "feature", "name": "liga", "index": 0 });
    let changes = [
        Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false, "code": "sub a by b;" })),
        Msg::run("font.features.addClass", json!({ "name": "Some", "code": "a b" })),
        Msg::run("font.features.setCode", json!({ "kind": "feature", "name": "liga", "code": "sub a by c;" })),
        Msg::run("font.features.setDisabled", json!({ "entries": [liga()], "disabled": true })),
        Msg::run("edit.undo", json!({})),
        Msg::run("font.features.remove", json!({ "entries": [liga()] })),
    ];
    let mut now = 10.0;
    step(&mut app, now);
    for change in changes {
        app.sent.push(change.clone());
        app.drain();
        app.session.active().map(|d| d.id).into_iter().for_each(|id| app.session.clear_feature_state(id));
        for pause in [0.0, 0.6, 0.7] {
            step(&mut app, now + pause);
        }
        now += 10.0;
        assert!(app.session.feature_state().is_some(), "{change:?} starts a compile");
        assert!(app.session.target_check().is_some(), "{change:?} checks every target");
    }
    app.sent.push(Msg::run("font.features.addPrefix", json!({})));
    app.drain();
    for pause in [0.0, 0.6, 0.7] {
        step(&mut app, now + pause);
    }
    let before = app.session.feature_state().expect("compiled");
    app.sent.push(Msg::run("font.features.setNotes", json!({ "kind": "class", "name": "Some", "notes": "x" })));
    app.drain();
    now += 10.0;
    for pause in [0.0, 0.6, 0.7] {
        step(&mut app, now + pause);
    }
    assert!(app.session.feature_state().is_some_and(|state| Arc::ptr_eq(&state, &before)));
}

/// What a compile found outside the entries shows over the code with its targets, the sidebar says that the code changed
/// since, and Compile and Update report in the status line.
#[test]
fn problems_outside_the_entries_show_and_compile_and_update_give_a_notice() {
    let mut model = features_model();
    model.features.stale = true;
    model.features.unplaced =
        vec![DiagnosticModel { id: u64::MAX, error: true, message: "mark lookup is broken (variable font)".into(), ..DiagnosticModel::default() }];
    let (mut fixture, _) = panel(model);
    let painted: Vec<String> = fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| if let egui::Shape::Text(t) = shape { Some(t.galley.job.text.clone()) } else { None })
        .collect();
    for text in ["mark lookup is broken (variable font)", "Changed since the last compile"] {
        assert!(painted.iter().any(|p| p == text), "{text}: {painted:?}");
    }

    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::run("font.features.addFeature", json!({ "tag": "liga", "automatic": false, "code": "sub a by nonesuch;" })));
    app.sent.push(Msg::run("font.features.compile", json!({})));
    app.drain();
    assert_eq!(app.status, "Compile: 1 error");
    app.sent.push(Msg::run("font.features.update", json!({})));
    app.drain();
    assert!(app.status.starts_with("Update: "), "{}", app.status);
}

/// The Exports tab shows what the shown entry's export writes once the query landed, marks the entries of one name, and
/// puts a token that names nothing under its row.
#[test]
fn the_exports_tab_shows_the_exported_names_and_marks_the_entries_that_share_one() {
    use crate::shell::export_names::step;
    let host = Host::builder(APP).jobs(Arc::new(craft::host::InlineJobs)).build();
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(host));
    app.sent.push(Msg::run("font.instance.add", json!({ "styleName": "Twin" })));
    app.sent.push(Msg::run("font.instance.add", json!({ "styleName": "Twin" })));
    app.drain();
    let all = app.session.font().expect("a font").instances.len();
    let (first, second) = (all - 2, all - 1);
    let designer = json!({ "scope": "instance", "targets": [second], "values": { "designers": "{{{no such}}}" } });
    app.sent.push(Msg::run("font.info.set", designer));
    app.sent.push(shell("font.info", json!({ "tab": "exports" })));
    app.sent.push(shell("font.info.exports.select", json!({ "instances": [second] })));
    app.drain();
    assert!(app.models.font_info.exports.exported.is_none(), "not known yet");
    step(&mut app, 10.0);
    step(&mut app, 10.1);
    app.drain();
    let model = app.models.font_info.exports.clone();
    assert_eq!(model.conflicts, vec![first, second]);
    let exported = model.exported.clone().expect("the names");
    let name = |id: u64| exported.names.iter().find(|n| n.id == id && n.place.is_empty()).map(|n| n.text.clone());
    assert_eq!(name(17).as_deref(), Some("Twin"));
    assert!(exported.conflicts.iter().any(|c| c.field == "fileName" && c.with == "Twin"), "{:?}", exported.conflicts);
    let row = model.rows.iter().find(|r| r.key == "designers").expect("the row");
    assert!(row.warnings.iter().any(|w| w.contains("{{{no such}}}")), "{:?}", row.warnings);
    // An edit leaves the last answer until the next lands, which a removed entry does not.
    app.sent.push(Msg::run("font.instance.remove", json!({ "instances": [first] })));
    app.drain();
    assert!(app.models.font_info.exports.exported.is_none(), "another number of entries");
    step(&mut app, 20.0);
    step(&mut app, 20.6);
    step(&mut app, 20.7);
    app.drain();
    assert!(app.models.font_info.exports.conflicts.is_empty());

    let mut shown = FontInfoModel { open: true, tab: FontInfoTab::Exports, exports: model, ..FontInfoModel::default() };
    shown.exports.params.clear();
    let (fixture, _) = panel(shown);
    assert!(fixture.texts().iter().any(|t| t.text.contains("As Exported")), "the section shows");
}

/// The Exports tab draws the selected instance in its own outlines: those of the engine's instance view, side by side,
/// with the glyphs of Instance Preview.
#[test]
fn the_instance_preview_is_drawn_in_the_instances_own_outlines() {
    use fontcraft_engine::interp_cache::SessionInterpExt as _;
    let mut s = Session::with_document();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    let ids: Vec<String> = {
        run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
        run("font.master.add", json!({}));
        run("font.master.setAxisLocation", json!({"masters": [0], "location": [100]}));
        run("font.master.setAxisLocation", json!({"masters": [1], "location": [300]}));
        run("font.instance.add", json!({"styleName": "Mid", "axes": {"wght": 200}}));
        s.font().expect("a font").masters.iter().map(|m| m.id.clone()).collect()
    };
    for (id, width) in ids.iter().zip([100, 300]) {
        let p = json!({"glyph": "A", "layer": id, "x": 0, "y": 0, "width": width, "height": 500});
        s.execute("path.addRectangle", &p).expect("a rectangle");
    }
    let mid = s.font().expect("a font").instances.iter().position(|i| i.name == "Mid").expect("the instance");
    let p = json!({"scope": "instance", "targets": [mid], "name": "Instance Preview", "value": ["A", "nowhere"]});
    s.execute("font.customParameters.add", &p).expect("the parameter");
    let view = s.instance_view(mid, &["A".to_string()]).expect("the instance's view");
    let master = view.masters.first().expect("its master").id.clone();
    let drawn = view.outline("A", &master).expect("its outline");

    let mut app = FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()));
    app.sent.push(shell("font.info", json!({ "tab": "exports" })));
    app.sent.push(shell("font.info.exports.select", json!({ "instances": [mid] })));
    app.drain();
    let preview = app.models.font_info.exports.preview.clone().expect("the preview");
    assert_eq!(preview.outline.elements(), drawn.elements());
    let (fixture, _) = panel((*app.models.font_info).clone());
    assert!(fixture.by_label("Instance Preview").is_some(), "it draws");
}
