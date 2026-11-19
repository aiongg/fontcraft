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
use crate::app::{APP, FontCraftApp, Msg};
use crate::dialogs::feature_picker::FeaturePicker;
use crate::shell::font_info::FontInfoTab;
use crate::shell::font_info_tabs::{EntryKey, EntryKind};

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
            diagnostics: vec![DiagnosticModel { id: 7, error: true, message: "unknown glyph".into(), line: 1, fixes: vec!["Remove".into()] }],
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

/// Typed code is committed with `font.features.setCode` when the field loses the keyboard; the automatic box, a Fix
/// button and a stylistic set name post their commands.
#[test]
fn features_entry_pane_posts_code_fixes_and_names() {
    let (mut fixture, posted) = panel(features_model());
    let code = fixture.by_label("Feature Code").expect("the code field");
    fixture.click(&code).key(Key::A, Modifiers::COMMAND).type_text("sub b by c;").settle();
    let notes = fixture.by_label("Notes").expect("the notes field");
    fixture.click(&notes).settle();
    let automatic = fixture.by_label("Generate Feature Automatically").expect("the box");
    fixture.click(&automatic).settle();
    let fix = fixture.by_label("Fix: Remove").expect("the Fix button");
    fixture.click(&fix).settle();
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
    let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
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
