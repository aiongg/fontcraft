//! The glyph inspector as the user drives it: typed values reach every selected glyph through the field's command,
//! and the orientation switch and the disclosures post their shell commands.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use egui::{Key, Modifiers};
use serde_json::json;

use super::show;
use crate::app::Msg;
use crate::app::models::glyph_props::{GlyphPropsModel, PropField, Shown};

fn same(text: &str) -> Shown {
    Shown { text: text.into(), mixed: false }
}

fn mixed() -> Shown {
    Shown { text: String::new(), mixed: true }
}

/// Three glyphs whose metrics, left group and export differ.
pub(crate) fn three(vertical: bool) -> GlyphPropsModel {
    let mut fields = vec![Shown::default(); PropField::ALL.len()];
    if let Some(f) = fields.get_mut(PropField::Name.index()) {
        *f = mixed();
    }
    if let Some(f) = fields.get_mut(PropField::LeftGroup.index()) {
        *f = mixed();
    }
    if let Some(f) = fields.get_mut(PropField::RightGroup.index()) {
        *f = same("o");
    }
    let metrics = if vertical { vec![same("50"), mixed(), same("0"), same("1000")] } else { vec![mixed(), same("40"), same("600")] };
    GlyphPropsModel {
        has_font: true,
        glyphs: vec!["A".into(), "B".into(), "C".into()],
        master: "m01".into(),
        fields,
        metrics,
        export: None,
        tags: Some(vec!["base".into()]),
        sort_name_stored: Some(false),
        vertical,
        ..GlyphPropsModel::default()
    }
}

fn open(model: GlyphPropsModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, &model, &mut sent);
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    (fixture, posted)
}

/// Types `text` into the field labelled `label` and presses Return.
pub(crate) fn commit(fixture: &mut Fixture, label: &str, text: &str) {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
    fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
}

/// Typing into O and H runs `spacing.setVerticalOrigin` and `spacing.setVerticalWidth` on every glyph in Font
/// View's master: a number as a number, a key as text.
#[test]
fn typing_into_o_and_h_runs_the_vertical_metric_commands() {
    let (mut fixture, posted) = open(three(true));
    commit(&mut fixture, "Vertical Origin", "40");
    commit(&mut fixture, "Vertical Width", "=H");
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("spacing.setVerticalOrigin", json!({ "glyphs": ["A", "B", "C"], "layer": "m01", "value": 40.0 })),
            Msg::run("spacing.setVerticalWidth", json!({ "glyphs": ["A", "B", "C"], "layer": "m01", "value": "=H" })),
        ]
    );
}

/// On three glyphs with mixed values, a typed LSB, kerning group or tag goes to all three in one command, and the
/// mixed Exports box turns export on for all of them.
#[test]
fn edits_on_three_glyphs_reach_all_of_them() {
    let (mut fixture, posted) = open(three(false));
    commit(&mut fixture, "LSB", "=n");
    commit(&mut fixture, "Left Group", "H");
    commit(&mut fixture, "Tags", "round,");
    let exports = fixture.by_label("Exports").expect("the Exports box");
    fixture.click(&exports).settle();
    let glyphs = json!(["A", "B", "C"]);
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("spacing.setLeftSidebearing", json!({ "glyphs": glyphs, "layer": "m01", "value": "=n" })),
            Msg::run("kerning.setLeftGroup", json!({ "glyphs": glyphs, "group": "H" })),
            Msg::run("glyph.addTag", json!({ "glyphs": glyphs, "tag": "round" })),
            Msg::run("glyph.setExport", json!({ "glyphs": glyphs, "export": true })),
        ]
    );
}

/// The orientation switch and the extra rows' disclosure post their shell commands; the Name field takes nothing
/// with several glyphs.
#[test]
fn the_switches_post_their_shell_commands() {
    let (mut fixture, posted) = open(three(false));
    let switch = fixture.by_label("Show Vertical Metrics").expect("the orientation switch");
    fixture.click(&switch).settle();
    let details = fixture.by_label("Show Extra Glyph Properties").expect("the details disclosure");
    fixture.click(&details).settle();
    commit(&mut fixture, "Name", "X");
    let shell = |id: &str, params| Msg::Shell(CommandRef::with_params(id, params));
    assert_eq!(
        *posted.borrow(),
        vec![
            shell("fontView.toggleMetricsOrientation", json!({ "orientation": "vertical" })),
            shell("fontView.toggleInspectorDetails", json!({ "expanded": true })),
        ]
    );
}
