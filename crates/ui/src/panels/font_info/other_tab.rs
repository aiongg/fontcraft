//! The Other Settings tab: Grid Spacing, Subdivision, the Shift and Command keyboard increments, Use Custom Naming,
//! Disable Automatic Alignment, Keep Alternates Next to Base Glyph, File Format Version and Font Type. It shows what
//! `font.settings.get` answers, and every change posts `font.settings.set` with that one key.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Map, Value as Json, json};

use super::param_value::number_json;
use crate::app::Msg;
use crate::app::models::font_info::OtherModel;

craft::ui::words! {
    /// The words of the Other Settings tab.
    pub struct OtherWords {
        grid_spacing: "fontcraftFontInfoOther.gridSpacing" = "Grid Spacing";
        grid_spacing_tip: "fontcraftFontInfoOther.gridSpacingTip" = "The grid points snap to, in font units; 0 keeps coordinates unrounded";
        subdivision: "fontcraftFontInfoOther.subdivision" = "Subdivision";
        subdivision_tip: "fontcraftFontInfoOther.subdivisionTip" = "Divides the grid for finer steps";
        shift: "fontcraftFontInfoOther.shift" = "Shift Increment";
        shift_tip: "fontcraftFontInfoOther.shiftTip" = "How far arrow keys move or step values with Shift held";
        command: "fontcraftFontInfoOther.command" = "Command Increment";
        command_tip: "fontcraftFontInfoOther.commandTip" = "How far arrow keys move or step values with Command held";
        custom_naming: "fontcraftFontInfoOther.customNaming" = "Use Custom Naming";
        no_alignment: "fontcraftFontInfoOther.noAlignment" = "Disable Automatic Alignment";
        alternates: "fontcraftFontInfoOther.alternates" = "Keep Alternates Next to Base Glyph";
        format: "fontcraftFontInfoOther.format" = "File Format Version";
        format_2: "fontcraftFontInfoOther.format2" = "Version 2";
        format_3: "fontcraftFontInfoOther.format3" = "Version 3";
        format_4: "fontcraftFontInfoOther.format4" = "Version 4";
        font_type: "fontcraftFontInfoOther.fontType" = "Font Type";
        type_default: "fontcraftFontInfoOther.typeDefault" = "Default";
        type_variable: "fontcraftFontInfoOther.typeVariable" = "Variable";
    }
}

/// `font.settings.set` with one key.
pub fn set_command(key: &str, value: Json) -> Msg {
    let mut values = Map::new();
    values.insert(key.to_string(), value);
    Msg::run("font.settings.set", Json::Object(values))
}

pub fn view(ui: &mut Ui, model: &OtherModel, sent: &mut Vec<Msg>) {
    let w = Env::of(ui).words::<OtherWords>();
    let number = |key: &str| model.numbers.iter().find(|(k, _)| k == key).and_then(|(_, v)| *v);
    let numbers: [(&str, &Label, &Label); 4] = [
        ("gridSpacing", &w.grid_spacing, &w.grid_spacing_tip),
        ("gridSubdivision", &w.subdivision, &w.subdivision_tip),
        ("keyboardIncrementShift", &w.shift, &w.shift_tip),
        ("keyboardIncrementCmd", &w.command, &w.command_tip),
    ];
    for (key, label, tip) in numbers {
        Field::new(label.clone()).show(ui, |ui, _| {
            let out = NumberField::new(label.clone(), Bind::given(number(key))).tooltip(tip.clone()).show(ui);
            if let Some(Some(n)) = out.edit.events.committed() {
                sent.push(set_command(key, number_json(*n)));
            }
        });
    }
    let flags: [(&str, &Label); 3] =
        [("useCustomNaming", &w.custom_naming), ("disableAutomaticAlignment", &w.no_alignment), ("keepAlternatesNextToBase", &w.alternates)];
    for (key, label) in flags {
        let on = model.flags.iter().find(|(k, _)| k == key).is_some_and(|(_, v)| *v);
        if let Some(next) = Checkbox::new(Bind::given(on)).label(label.clone()).show(ui).events.committed() {
            sent.push(set_command(key, json!(next)));
        }
    }
    let formats = [Entry::new(2i64, w.format_2.clone()), Entry::new(3, w.format_3.clone()), Entry::new(4, w.format_4.clone())];
    Field::new(w.format.clone()).show(ui, |ui, _| {
        if let Some(picked) = Select::new(w.format.clone(), Bind::given(model.file_format_version), &formats).show(ui).events.committed() {
            sent.push(set_command("fileFormatVersion", json!(picked)));
        }
    });
    let types = [Entry::new("default".to_string(), w.type_default.clone()), Entry::new("variable".to_string(), w.type_variable.clone())];
    Field::new(w.font_type.clone()).show(ui, |ui, _| {
        if let Some(picked) = Select::new(w.font_type.clone(), Bind::given(model.font_type.clone()), &types).show(ui).events.committed() {
            sent.push(set_command("fontType", json!(picked)));
        }
    });
}
