//! The shared rows of the Font Info panel: a section heading with its plus button, and one row per General field or
//! custom parameter, with its check box (parameters), its value editor (`param_value`) and its minus button. The row
//! draws; [`commands`] turns what the user did into the messages that run it (`font.info.set`,
//! `font.customParameters.set`, `font.info.removeField`…).

use craft::behavior::IconId;
use craft::theme::variant::Tone;
use craft::ui::buttons::IconButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::Ui;
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::model::{DEFAULT_LANGUAGE, LANGUAGES};
use serde_json::{Map, Value as Json, json};

use super::FontInfoWords;
use super::param_value::{self, Edit};
use crate::app::Msg;
use crate::app::models::font_exports::sentence;
use crate::app::models::font_info::{FieldWarning, RowDialog, RowModel, RowSort};
use crate::shell::font_info::{InfoDialog, Target, open_params};

/// What the user did to a row.
#[derive(Clone, Debug, PartialEq)]
pub enum RowAction {
    Edit(Edit),
    /// The parameter's check box.
    SetEnabled(bool),
    /// The minus button.
    Remove,
}

/// A section titled `title` whose header holds a plus button named `plus`; true when the plus was clicked. `salt`
/// keeps the sections of one tab apart.
pub fn section(ui: &mut Ui, salt: &str, title: Label, plus: Label, body: impl FnOnce(&mut Ui)) -> bool {
    let mut clicked = false;
    let mut body = Some(body);
    Section::new(title)
        .id_salt(salt)
        .controls(|ui, _| {
            clicked |= IconButton::new(IconId::new("plus")).label(plus.clone()).tooltip(plus.clone()).id_salt((salt, "plus")).show(ui).clicked();
        })
        .show(ui, |ui, _| {
            if let Some(body) = body.take() {
                body(ui);
            }
        });
    clicked
}

/// The warnings of `list` that belong under the field `field`, one line each.
pub fn warnings(ui: &mut Ui, list: &[FieldWarning], field: &str) {
    for warning in list.iter().filter(|w| w.field == field) {
        Text::new(sentence(&warning.message)).tone(Tone::Warn).show(ui);
    }
}

/// The languages a localizable field may take: the default language, then the language tags the name table knows.
pub fn language_entries() -> Vec<Entry<String>> {
    std::iter::once(DEFAULT_LANGUAGE).chain(LANGUAGES.iter().map(|(tag, _)| *tag)).map(|tag| Entry::new(tag.to_string(), tag)).collect()
}

/// Draws `rows` of `target` and posts what the user did.
pub fn rows(ui: &mut Ui, rows: &[RowModel], target: &Target, masters: &[(String, String)], sent: &mut Vec<Msg>) {
    let languages =
        if rows.iter().any(|r| matches!(r.editor, crate::app::models::font_info::Editor::Languages(_))) { language_entries() } else { Vec::new() };
    for row in rows {
        if let Some(action) = one(ui, row, masters, &languages) {
            sent.extend(commands(target, row, &action));
        }
    }
}

/// One row: the side label, then the check box, the editor and the minus button.
pub fn one(ui: &mut Ui, row: &RowModel, masters: &[(String, String)], languages: &[Entry<String>]) -> Option<RowAction> {
    let words = Env::of(ui).words::<FontInfoWords>();
    let mut action = None;
    let salt = (row.sort == RowSort::Param, row.key.as_str(), row.occurrence);
    Field::new(row.label.clone()).id_salt(salt).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            if row.sort == RowSort::Param {
                let out = Checkbox::new(Bind::given(row.enabled)).label(words.use_parameter.clone()).tooltip(words.use_parameter.clone()).show(ui);
                if let Some(on) = out.events.committed() {
                    action = Some(RowAction::SetEnabled(*on));
                }
            }
            if let Some(edit) = param_value::editor(ui, row, masters, languages) {
                action = Some(RowAction::Edit(edit));
            }
            if row.removable
                && IconButton::new(IconId::new("minus")).label(words.remove_row.clone()).tooltip(words.remove_row.clone()).show(ui).clicked()
            {
                action = Some(RowAction::Remove);
            }
        });
    });
    action
}

/// The messages a row action posts on `target`.
pub fn commands(target: &Target, row: &RowModel, action: &RowAction) -> Vec<Msg> {
    let param = |extra: Vec<(&str, Json)>| {
        let mut all = vec![("name", json!(row.key)), ("index", json!(row.occurrence))];
        all.extend(extra);
        with(target.params(), all)
    };
    let each = |id: &str, extra: Vec<(&str, Json)>| -> Vec<Msg> {
        let mut all = vec![("key", json!(row.key))];
        all.extend(extra);
        target.each().into_iter().map(|p| Msg::run(id, with(p, all.clone()))).collect()
    };
    match (action, row.sort) {
        (RowAction::SetEnabled(_), RowSort::Field) => Vec::new(),
        (RowAction::SetEnabled(on), RowSort::Param) => vec![Msg::run("font.customParameters.set", param(vec![("enabled", json!(on))]))],
        (RowAction::Remove, RowSort::Param) => vec![Msg::run("font.customParameters.remove", param(Vec::new()))],
        (RowAction::Remove, RowSort::Field) => vec![Msg::run("font.info.removeField", with(target.params(), vec![("keys", json!([row.key]))]))],
        (RowAction::Edit(Edit::Value(v)), RowSort::Param) => vec![Msg::run("font.customParameters.set", param(vec![("value", v.clone())]))],
        (RowAction::Edit(Edit::Value(v)), RowSort::Field) => {
            let mut values = Map::new();
            values.insert(row.key.clone(), v.clone());
            vec![Msg::run("font.info.set", with(target.params(), vec![("values", Json::Object(values))]))]
        }
        (RowAction::Edit(Edit::Language { language, text }), _) => {
            each("font.info.setLocalizedValue", vec![("language", json!(language)), ("value", json!(text))])
        }
        (RowAction::Edit(Edit::AddLanguage(language)), _) => {
            each("font.info.setLocalizedValue", vec![("language", json!(language)), ("value", json!(""))])
        }
        (RowAction::Edit(Edit::RemoveLanguage(language)), _) => each("font.info.removeLocalizedValue", vec![("language", json!(language))]),
        (RowAction::Edit(Edit::ChangeLanguage { from, to }), _) => {
            let text = match &row.editor {
                crate::app::models::font_info::Editor::Languages(l) => l.iter().find(|(l, _)| l == from).map(|(_, t)| t.clone()).unwrap_or_default(),
                _ => String::new(),
            };
            let mut out = each("font.info.setLocalizedValue", vec![("language", json!(to)), ("value", json!(text))]);
            out.extend(each("font.info.removeLocalizedValue", vec![("language", json!(from))]));
            out
        }
        (RowAction::Edit(Edit::Dialog(dialog)), _) => dialog_message(target, row, *dialog).into_iter().collect(),
    }
}

/// The message that opens a value's dialog on `target`.
pub fn dialog_message(target: &Target, row: &RowModel, dialog: RowDialog) -> Option<Msg> {
    let shell = |id: &str, params: Json| Msg::Shell(CommandRef::with_params(id, params));
    let open = |dialog: &str, params: Json| shell("dialog.open", json!({"dialog": dialog, "params": params}));
    Some(match dialog {
        RowDialog::Os2Bits => {
            shell("font.info.openDialog", open_params(&InfoDialog::Bits { target: target.clone(), name: row.key.clone(), index: row.occurrence }))
        }
        RowDialog::Panose => shell("font.info.openDialog", open_params(&InfoDialog::Panose { target: target.clone(), index: row.occurrence })),
        RowDialog::TtfStems => shell("window.ttfStems", json!({"master": target.first()})),
        RowDialog::TtfZones if target.scope == Scope::Font => shell("window.ttfZones", json!({"family": true})),
        RowDialog::TtfZones => shell("window.ttfZones", json!({"master": target.first()})),
        RowDialog::AxisLocation => open("dialog.axisLocation", json!({"scope": target.scope.name(), "targets": target.first()})),
        RowDialog::VirtualMaster => open("dialog.virtualMaster", json!({"index": row.occurrence})),
        RowDialog::AxisMappings if target.scope == Scope::Instance => {
            shell("axisMappings.showDialog", json!({"variableFontSetting": target.first()}))
        }
        RowDialog::AxisMappings => shell("axisMappings.showDialog", json!({})),
        RowDialog::ColorPalettes => shell("colorPalette.edit", json!({})),
        RowDialog::MasterColor => open("dialog.masterColorPicker", json!({"targets": target.indices, "name": row.key})),
    })
}

/// `p` with the `extra` entries added, as a JSON object.
fn with(mut p: Map<String, Json>, extra: Vec<(&str, Json)>) -> Json {
    for (k, v) in extra {
        p.insert(k.to_string(), v);
    }
    Json::Object(p)
}
