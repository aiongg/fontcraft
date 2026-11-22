//! Selection Info as the user drives it: each field M4.19 adds runs its command once on every described glyph.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::contract::Tracked;
use craft::ui::testing::Fixture;
use serde_json::json;

use super::{Props, show};
use crate::app::Msg;
use crate::panels::font_view::inspector::tests::{commit, three};

/// The note, the metrics keys and the kerning groups each run their command on the three glyphs.
#[test]
fn each_field_runs_its_command_on_every_glyph() {
    let model = Tracked::new(three(false));
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    commit(&mut fixture, "Left Metrics Key", "=n");
    commit(&mut fixture, "Width Metrics Key", "=H");
    commit(&mut fixture, "Right Group", "O");
    commit(&mut fixture, "Case", "upper");
    let glyphs = json!(["A", "B", "C"]);
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("spacing.setLeftSidebearing", json!({ "glyphs": glyphs, "layer": "m01", "value": "=n" })),
            Msg::run("spacing.setWidth", json!({ "glyphs": glyphs, "layer": "m01", "value": "=H" })),
            Msg::run("kerning.setRightGroup", json!({ "glyphs": glyphs, "group": "O" })),
            Msg::run("glyph.setCase", json!({ "glyphs": glyphs, "case": "upper" })),
        ]
    );
}

/// The other fields M4.19 lists run their setter once on all three glyphs, a mixed field (Left Group) included.
#[test]
fn the_remaining_fields_run_their_setter_on_every_glyph() {
    let model = Tracked::new(three(false));
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    commit(&mut fixture, "Right Metrics Key", "=o");
    commit(&mut fixture, "Left Group", "H");
    commit(&mut fixture, "Direction", "rtl");
    commit(&mut fixture, "Production Name", "uni0041");
    commit(&mut fixture, "Subcategory", "Uppercase");
    let glyphs = json!(["A", "B", "C"]);
    assert_eq!(
        *posted.borrow(),
        vec![
            Msg::run("spacing.setRightSidebearing", json!({ "glyphs": glyphs, "layer": "m01", "value": "=o" })),
            Msg::run("kerning.setLeftGroup", json!({ "glyphs": glyphs, "group": "H" })),
            Msg::run("glyph.setDirection", json!({ "glyphs": glyphs, "direction": "rtl" })),
            Msg::run("glyph.setProductionName", json!({ "glyphs": glyphs, "productionName": "uni0041" })),
            Msg::run("glyph.setCategory", json!({ "glyphs": glyphs, "subCategory": "Uppercase" })),
        ]
    );
}
