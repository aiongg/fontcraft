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
