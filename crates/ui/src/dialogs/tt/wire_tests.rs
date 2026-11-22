//! The TrueType dialogs against their commands over the control channel. Each dialog's body is filled in a frame, and
//! what it posts runs in an app as the dialog module posts it; that app's font equals the font of an app given the
//! command over a TCP connection, with the parameters the command's documentation names. Cancel, and opening an
//! editor, leave the very font the app had.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use fontcraft_engine::hinting::settings::main_master;
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

use super::deltas;
use super::instructions::{self, InstructionsDialog, Press, Program, Request};
use super::zones::{self, Editor, Kind, List};
use crate::app::Msg;
use crate::control::tt_tests::{execute, wire};
use crate::testing::TestApp;

fn font(t: &TestApp) -> Arc<Font> {
    t.app.session.doc().map(|d| d.font.clone()).expect("a document")
}

fn master(t: &TestApp) -> String {
    main_master(&font(t))
}

/// Posts the dialog's messages in `t`, as the dialog module does, and runs a frame.
fn post(t: &mut TestApp, messages: Vec<Msg>) {
    t.app.sent.extend(messages);
    t.frame();
}

/// The font of a new app given `commands` over TCP, each with its parameters.
fn over_the_wire(commands: Vec<(&str, Value)>) -> Arc<Font> {
    let mut t = TestApp::new();
    let replies = wire(&mut t, commands.into_iter().map(|(command, params)| execute(command, params)).collect());
    for reply in &replies {
        assert_eq!(reply["ok"], true, "{reply}");
    }
    font(&t)
}

/// Shows the TTFZones or TTFStems editor body over `t`'s master `master`, does `act`, and returns the messages the
/// editor would post.
fn fill(t: &TestApp, kind: Kind, master: &str, act: impl FnOnce(&mut Fixture)) -> Vec<Msg> {
    let list = List::read(&font(t), kind, master).expect("the editor's list");
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut editor = Editor::default();
    let mut f = Fixture::new(move |ui| {
        let mut controls = Vec::new();
        zones::view(ui, &list, &mut editor, &mut controls);
        seen.borrow_mut().extend(controls.iter().filter_map(|c| list.command(c)).map(|(id, params)| Msg::run(id, params)));
    });
    f.settle();
    act(&mut f);
    posted.take()
}

fn press(f: &mut Fixture, id: egui::Id) {
    let node = f.node_of(id).expect("the control");
    f.click(&node).settle();
}

fn type_number(f: &mut Fixture, id: egui::Id, text: &str) {
    let node = f.node_of(id).expect("the field");
    f.click(&node).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
}

/// The TTFStems editor: plus, then a typed size on the new row, give what `hint.tt.addStemDefinition` and
/// `hint.tt.setStemDefinition` give over the wire.
#[test]
fn the_stems_editor_gives_what_its_commands_give_over_the_wire() {
    let mut t = TestApp::new();
    let m = master(&t);
    let added = fill(&t, Kind::Stems, &m, |f| press(f, zones::button_id(Kind::Stems, "plus")));
    assert_eq!(added.len(), 1, "{added:?}");
    post(&mut t, added);
    let list = List::read(&font(&t), Kind::Stems, &m).unwrap();
    let at = list.entries.len() - 1;
    let stem = list.entries[at].name.clone();
    let sized = fill(&t, Kind::Stems, &m, |f| type_number(f, zones::row_id(Kind::Stems, at, "size"), "120"));
    assert_eq!(sized.len(), 1, "{sized:?}");
    post(&mut t, sized);

    let wired = over_the_wire(vec![
        ("hint.tt.addStemDefinition", json!({"master": m, "orientation": "x", "size": 80})),
        ("hint.tt.setStemDefinition", json!({"master": m, "stem": stem, "size": 120})),
    ]);
    assert_eq!(*font(&t), *wired);
}

/// The Delta table of a TT zone: a click on a cell gives what `hint.tt.setZone {deltas}` gives over the wire.
#[test]
fn the_zone_delta_table_gives_what_set_zone_gives_over_the_wire() {
    let mut t = TestApp::new();
    let m = master(&t);
    let added = fill(&t, Kind::Zones, &m, |f| press(f, zones::button_id(Kind::Zones, "plus")));
    post(&mut t, added);
    let list = List::read(&font(&t), Kind::Zones, &m).unwrap();
    let at = list.entries.len() - 1;
    let zone = list.entries[at].name.clone();
    // The first column, by the key the editor stores for it.
    let (column, key) = (0, list.masters[0].key.clone());
    let cell = fill(&t, Kind::Zones, &m, |f| {
        f.room(egui::Vec2::new(800.0, 3000.0)).settle();
        press(f, zones::row_id(Kind::Zones, at, "deltas"));
        press(f, deltas::cell_id(column, 12));
    });
    assert_eq!(cell.len(), 1, "{cell:?}");
    post(&mut t, cell);

    let wired = over_the_wire(vec![
        ("hint.tt.addZone", json!({"master": m, "position": 0, "size": 16})),
        ("hint.tt.setZone", json!({"master": m, "zone": zone, "deltas": {key: {"12": "up"}}})),
    ]);
    assert_eq!(*font(&t), *wired);
}

/// The filter sheet's Cancel posts nothing, and opening the TTFZones editor over the wire leaves the very font the app
/// had.
#[test]
fn the_zones_editor_cancel_and_opening_leave_the_font_pointer_equal() {
    let mut t = TestApp::new();
    let m = master(&t);
    let added = fill(&t, Kind::Zones, &m, |f| press(f, zones::button_id(Kind::Zones, "plus")));
    post(&mut t, added);
    let before = font(&t);
    let cancelled = fill(&t, Kind::Zones, &m, |f| {
        f.room(egui::Vec2::new(800.0, 3000.0)).settle();
        press(f, zones::row_id(Kind::Zones, 0, "filter"));
        let field = f.node_of(zones::button_id(Kind::Zones, "predicate")).expect("predicate field");
        f.click(&field).type_text("category == \"Letter\"").settle();
        press(f, zones::button_id(Kind::Zones, "cancel"));
    });
    assert!(cancelled.is_empty(), "{cancelled:?}");
    post(&mut t, cancelled);
    let replies = wire(&mut t, vec![execute("window.ttfZones", json!({})), execute("window.ttfStems", json!({}))]);
    assert!(replies.iter().all(|r| r["ok"] == true), "{replies:?}");
    t.frames(3);
    assert!(Arc::ptr_eq(&before, &font(&t)), "the font is the one the app had");
}

/// Shows the TrueType Instructions dialog body over `dialog`, presses `presses` in turn (setting the text area to
/// `text` after Edit), and returns the dialog.
fn instructions_after(dialog: InstructionsDialog, presses: &[Press], text: &str) -> InstructionsDialog {
    let dialog = Rc::new(RefCell::new(dialog));
    let shown = Rc::clone(&dialog);
    let mut f = Fixture::new(move |ui| {
        let mut d = shown.borrow_mut();
        let InstructionsDialog { program, draft, error, .. } = &mut *d;
        let pressed = instructions::view(ui, program, draft, error.as_deref());
        if let Some(p) = pressed {
            d.press(p);
        }
    });
    f.settle();
    for p in presses {
        let part = match p {
            Press::Edit => "edit",
            Press::Parse => "parse",
            Press::Cancel => "cancel",
        };
        press(&mut f, instructions::control_id(part));
        if *p == Press::Edit {
            dialog.borrow_mut().draft = Some(text.to_string());
            f.settle();
        }
    }
    drop(f);
    Rc::try_unwrap(dialog).ok().expect("the fixture is gone").into_inner()
}

/// The Instructions dialog: Edit, new text and Parse give what `hint.tt.setAssembly` gives over the wire; Edit then
/// Cancel posts nothing and leaves the very font the app had.
#[test]
fn the_instructions_dialog_gives_what_set_assembly_gives_and_cancel_changes_nothing() {
    let mut t = TestApp::new();
    let request = Request { target: json!({"glyph": "A"}), master: None };
    let open = |t: &TestApp| {
        let mut d = InstructionsDialog::new(request.clone());
        d.program = Program::read(&font(t), &request);
        d
    };

    let before = font(&t);
    let cancelled = instructions_after(open(&t), &[Press::Edit, Press::Cancel], "SVTCA[1]");
    assert_eq!(cancelled.draft, None);
    assert!(cancelled.outbox.is_empty(), "{:?}", cancelled.outbox);
    post(&mut t, cancelled.outbox);
    assert!(Arc::ptr_eq(&before, &font(&t)), "Cancel leaves the font the app had");

    let parsed = instructions_after(open(&t), &[Press::Edit, Press::Parse], "SVTCA[1]");
    assert_eq!(parsed.outbox.len(), 1, "{:?}", parsed.outbox);
    post(&mut t, parsed.outbox);
    let wired = over_the_wire(vec![("hint.tt.setAssembly", json!({"target": {"glyph": "A"}, "assembly": "SVTCA[1]"}))]);
    assert_eq!(*font(&t), *wired);
}
