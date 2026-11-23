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
use crate::shell::font_info_tabs::{CompletionAsk, EntryKey, EntryKind, FeatureDraft, PreviewAsk};

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
        master: Some("m01".into()),
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
            Msg::run("font.features.compile", json!({ "master": "m01" })),
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
    assert!(app.session.feature_state().is_none(), "nor compiles");

    app.sent.push(Msg::run("font.features.setCode", json!({ "kind": "feature", "name": "liga", "code": draft.code })));
    app.drain();
    step(&mut app, 20.0);
    step(&mut app, 20.6);
    step(&mut app, 20.7);
    let state = app.session.feature_state().expect("compiled in the background");
    assert!(state.has_errors());
    assert!(state.is_current(&app.session, app.session.doc().expect("a document")));
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
    let ask = PreviewAsk { key: key(EntryKind::Feature, "ss01", 1), name: "a".into(), start: 4, end: 5 };
    assert!(posted.borrow().contains(&Msg::View(ViewMsg::FeaturePreview(Some(ask)))), "{:?}", posted.borrow());
}

/// The name under a click keeps a leading `@` or backslash and stops at punctuation and spaces.
#[test]
fn the_clicked_name_takes_name_characters_on_both_sides() {
    use super::code_editor::name_at;
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
    use super::code_editor::{CodeEditorWords, preview_lines};
    use crate::app::models::font_features::PreviewModel;
    let words = CodeEditorWords::default();
    let class = PreviewModel {
        name: "@Top".into(),
        kind: "class".into(),
        glyphs: vec!["a".into(), "b".into()],
        unknown: vec!["zz".into()],
        ..PreviewModel::default()
    };
    assert_eq!(preview_lines(&class, &words), ["Class @Top: 2 glyphs", "a b", "Not in the font: zz"]);
    let marks = PreviewModel {
        name: "@MARKS".into(),
        kind: "class".into(),
        glyphs: vec!["acutecomb".into()],
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
