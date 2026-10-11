//! Rename Kerning Group: the question asked after a group name is edited in the Kerning window.
//! All Pairs runs `kerning.renameGroup` with scope `font`, which renames the group on every
//! glyph and in every pair; This Pair runs it with scope `pair`, which points only the selected
//! pair's side at the new name. Cancel renames nothing.
//!
//! The Kerning window opens it with the command's parameters (`side`, `from`, `to`, `pair`,
//! `master`, `direction`); `to` stays editable. Nothing runs until a button is pressed.

use fontcraft_spacing::kerning::MAX_GROUP_NAME_LEN;
use serde_json::{Map, Value};

use super::form::Field;
use super::{Dialog, Fields, Mode, Third};

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["All Pairs", "Cancel"],
    third: Some(Third { id: "thisPair", caption: "This Pair", key: "scope", ok: "font", value: "pair", ok_caption: None }),
    message: "Rename the group in the whole font, or only for the selected pair?",
    ..Dialog::new("dialog.renameKerningGroup", "Rename Kerning Group", "kerning.renameGroup", Fields::Targeted(fields))
};

fn fields(_: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let text = |key: &str| targets.get(key).and_then(Value::as_str);
    let side = match text("side") {
        Some(side @ ("left" | "right")) => side,
        _ => return Err("Rename Kerning Group: `side` must be \"left\" or \"right\"".into()),
    };
    let Some(from) = text("from").map(|f| f.strip_prefix('@').unwrap_or(f)).filter(|f| !f.is_empty()) else {
        return Err("Rename Kerning Group: `from` names the group to rename".into());
    };
    let shown: String = from.chars().take(MAX_GROUP_NAME_LEN).collect();
    Ok(vec![
        Field::text("to", &format!("New name for the {side} group @{shown}"), MAX_GROUP_NAME_LEN, "the group's new name, without @"),
        Field::choice(
            "scope",
            "",
            &[("font", "All Pairs"), ("pair", "This Pair")],
            "font",
            "font renames the group on every glyph and in every pair of every master; pair points only the selected pair's side at the new name",
        ),
    ])
}
