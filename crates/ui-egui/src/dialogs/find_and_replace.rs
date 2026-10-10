//! Edit › Find › Find and Replace…: renames the selected glyphs with `edit.findAndReplace`.
//! Nothing runs until Replace, which renames them all or, when a result is taken or not a valid
//! name, none of them and keeps the dialog open with the problems.

use fontcraft_model::MAX_NAME_LEN;

use super::form::Field;
use super::{Dialog, Fields, Mode};

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Replace", "Cancel"],
    ..Dialog::new("dialog.findAndReplaceGlyphNames", "Find and Replace", "edit.findAndReplace", Fields::Fixed(fields))
};

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![
        Field::text("find", "Find", MAX_NAME_LEN, "The text replaced in each name; empty appends Replace to every name"),
        Field::text("replace", "Replace", MAX_NAME_LEN, "What takes its place; with Regex, \\1 to \\9 name the expression's groups"),
        Field::bool("matchCase", "Match Case", true, "Find matches only the same capitals and small letters"),
        Field::bool("regex", "Regex", false, "Find is a regular expression"),
    ]
}
