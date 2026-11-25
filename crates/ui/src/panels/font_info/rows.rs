//! The shared rows of the Font Info panel: a section heading with its plus button, and one row per General field or
//! custom parameter, with its check box (parameters), its value editor (`param_value`) and its minus button. The row
//! draws; [`commands`] turns what the user did into the messages that run it (`font.info.set`,
//! `font.customParameters.set`, `font.info.removeField`…).
//!
//! With several masters or exports selected, the rows are those any of them holds. A row some of them lack shows how
//! many hold it: editing it adds it to the others, its check box and its minus act on those that hold it.
//!
//! A click on a row's label selects the row (`font.info.rows.select`): alone, with Cmd toggling it, with Shift adding
//! the rows between it and the one selected last. While rows are selected and no field or list has the keyboard, Copy,
//! Cut, Paste and Cmd+Delete act on them ([`takes_edits`]); a field or list that has it keeps its own.

use craft::behavior::IconId;
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::buttons::IconButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionResponse, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft_ext::ui::list::{CollectionEdits as _, EditCommand};
use egui::layers::ShapeIdx;
use egui::{Context, Id, Pos2, Rect, Sense, Ui};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::model::{DEFAULT_LANGUAGE, LANGUAGES};
use serde_json::{Map, Value as Json, json};

use super::FontInfoWords;
use super::param_value::{self, Edit};
use crate::app::Msg;
use crate::app::models::font_exports::sentence;
use crate::app::models::font_info::{Editor, FieldWarning, RowDialog, RowMark, RowModel, RowSort};
use crate::shell::font_info::{COPY_ROWS, CUT_ROWS, DELETE_ROWS, InfoDialog, PASTE_ROWS, RowKey, Target, open_params, select_rows};

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

fn claim_key() -> Id {
    Id::unique("fontcraftFontInfo.rows.claim")
}

/// Whether the selected rows take the edit commands now: they show, and nothing has the keyboard.
pub fn takes_edits(ctx: &Context) -> bool {
    let shown = ctx.data(|d| d.get_temp::<u64>(claim_key())).is_some_and(|pass| pass.saturating_add(1) >= ctx.cumulative_pass_nr());
    shown && ctx.memory(|m| m.focused().is_none())
}

/// The shell command an edit command runs on the selected rows.
pub fn edit_row_command(edit: EditCommand) -> Option<&'static str> {
    match edit {
        EditCommand::Copy => Some(COPY_ROWS),
        EditCommand::Cut => Some(CUT_ROWS),
        EditCommand::Paste => Some(PASTE_ROWS),
        EditCommand::Delete => Some(DELETE_ROWS),
        _ => None,
    }
}

/// The messages of the edit commands the list of `out` took: Copy and Paste act on the rows beside it, Delete runs
/// `remove`.
pub fn list_edits<K, E>(ui: &Ui, out: &CollectionResponse<K, E>, remove: Option<Msg>) -> Vec<Msg> {
    let taken = out.edit_commands(ui, &[EditCommand::Copy, EditCommand::Paste, EditCommand::Delete]);
    let message = |edit: EditCommand| match edit {
        EditCommand::Delete => remove.clone(),
        edit => edit_row_command(edit).map(|id| Msg::Shell(CommandRef::new(id))),
    };
    taken.into_iter().filter_map(message).collect()
}

/// The rows a Shift-click on the row at `at` adds: from the row selected last, or the clicked one alone when that is
/// not among `rows`. The clicked one comes last.
fn between(rows: &[RowModel], at: usize) -> Vec<RowKey> {
    let lead = rows.iter().position(|r| r.mark == RowMark::Lead).unwrap_or(at);
    let (low, high) = (lead.min(at), lead.max(at));
    let mut keys: Vec<RowKey> = rows.iter().skip(low).take(high.saturating_sub(low).saturating_add(1)).map(RowModel::key).collect();
    if lead > at {
        keys.reverse();
    }
    keys
}

/// The fill of a selected row, and the click on the row's label that selects it.
fn select(ui: &mut Ui, rows: &[RowModel], at: usize, places: (Rect, Rect), fill: ShapeIdx, target: &Target, sent: &mut Vec<Msg>) {
    let Some(row) = rows.get(at) else { return };
    let (whole, label) = places;
    if row.mark != RowMark::None {
        let soft = egui::Color32::from(Srgba::from(Env::of(ui).tokens.color.accent.soft));
        ui.painter().set(fill, egui::Shape::rect_filled(whole, 0.0, soft));
        let pass = ui.ctx().cumulative_pass_nr();
        ui.ctx().data_mut(|d| d.insert_temp(claim_key(), pass));
    }
    let id = Id::unique(("fontcraftFontInfo.rows.label", target.scope.name(), row.sort == RowSort::Param, row.key.as_str(), row.occurrence));
    let response = ui.interact(label.intersect(ui.clip_rect()), id, Sense::CLICK);
    if !response.clicked() {
        return;
    }
    // The rows take the edit commands while nothing has the keyboard.
    ui.memory_mut(|m| {
        if let Some(focused) = m.focused() {
            m.surrender_focus(focused);
        }
    });
    let mods = ui.input(|i| i.modifiers);
    let (keys, mode) = if mods.shift {
        (between(rows, at), "add")
    } else if mods.command {
        (vec![row.key()], "toggle")
    } else {
        (vec![row.key()], "set")
    };
    sent.push(Msg::Shell(select_rows(target.scope, &keys, mode)));
}

/// Draws `rows` of `target` and posts what the user did.
pub fn rows(ui: &mut Ui, rows: &[RowModel], target: &Target, masters: &[(String, String)], sent: &mut Vec<Msg>) {
    let languages = if rows.iter().any(|r| matches!(r.editor, Editor::Languages(_))) { language_entries() } else { Vec::new() };
    for (at, row) in rows.iter().enumerate() {
        let fill = ui.painter().add(egui::Shape::Noop);
        let (action, places) = draw(ui, row, masters, &languages, target.indices.len());
        if let Some(action) = action {
            sent.extend(commands(target, row, &action));
        }
        // A run that only measures shows nothing to click.
        if !ui.is_sizing_pass() {
            select(ui, rows, at, places, fill, target, sent);
        }
    }
}

/// One row: the side label, then the check box, the editor and the minus button.
pub fn one(ui: &mut Ui, row: &RowModel, masters: &[(String, String)], languages: &[Entry<String>]) -> Option<RowAction> {
    draw(ui, row, masters, languages, 1).0
}

/// Draws one row of `total` targets; what the user did, and the rects of the row and of its label.
fn draw(ui: &mut Ui, row: &RowModel, masters: &[(String, String)], languages: &[Entry<String>], total: usize) -> (Option<RowAction>, (Rect, Rect)) {
    let words = Env::of(ui).words::<FontInfoWords>();
    let mut action = None;
    let mut body = ui.available_rect_before_wrap();
    // The minus of a localizable field's last language removes the field.
    let minus = row.removable && !matches!(&row.editor, Editor::Languages(l) if !l.is_empty());
    let salt = (row.sort == RowSort::Param, row.key.as_str(), row.occurrence);
    let field = Field::new(row.label.clone()).id_salt(salt).show(ui, |ui, _| {
        body = ui.available_rect_before_wrap();
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
            if !row.holders.is_empty() {
                let held = format!("{}/{total}", row.holders.len());
                Text::new(held).shade(Shade::Muted).tooltip(words.some_lack.clone()).show(ui);
            }
            if minus && IconButton::new(IconId::new("minus")).label(words.remove_row.clone()).tooltip(words.remove_row.clone()).show(ui).clicked() {
                action = Some(RowAction::Remove);
            }
        });
    });
    let whole = field.output.rect;
    let label = if body.left() > whole.left() + 1.0 {
        Rect::from_min_max(whole.min, Pos2::new(body.left(), whole.bottom()))
    } else if body.right() + 1.0 < whole.right() {
        Rect::from_min_max(Pos2::new(body.right(), whole.top()), whole.max)
    } else {
        Rect::from_min_max(whole.min, Pos2::new(whole.right(), body.top().clamp(whole.top(), whole.bottom())))
    };
    (action, (whole, label))
}

/// The messages a row action posts on `target`.
pub fn commands(target: &Target, row: &RowModel, action: &RowAction) -> Vec<Msg> {
    let holders = if row.holders.is_empty() { target.clone() } else { Target { scope: target.scope, indices: row.holders.clone() } };
    let param = |target: &Target, extra: Vec<(&str, Json)>| {
        let mut all = vec![("name", json!(row.key)), ("index", json!(row.occurrence))];
        all.extend(extra);
        with(target.params(), all)
    };
    let localized = |id: &str, target: &Target, extra: Vec<(&str, Json)>| -> Vec<Msg> {
        let mut all = vec![("key", json!(row.key))];
        all.extend(extra);
        vec![Msg::run(id, with(target.params(), all))]
    };
    match (action, row.sort) {
        (RowAction::SetEnabled(_), RowSort::Field) => Vec::new(),
        (RowAction::SetEnabled(on), RowSort::Param) => vec![Msg::run("font.customParameters.set", param(&holders, vec![("enabled", json!(on))]))],
        (RowAction::Remove, RowSort::Param) => vec![Msg::run("font.customParameters.remove", param(&holders, Vec::new()))],
        (RowAction::Remove, RowSort::Field) => vec![Msg::run("font.info.removeField", with(holders.params(), vec![("keys", json!([row.key]))]))],
        (RowAction::Edit(Edit::Value(v)), RowSort::Param) => {
            let mut extra = vec![("value", v.clone())];
            if !row.holders.is_empty() {
                extra.push(("addMissing", json!(true)));
            }
            vec![Msg::run("font.customParameters.set", param(target, extra))]
        }
        (RowAction::Edit(Edit::Value(v)), RowSort::Field) => {
            let mut values = Map::new();
            values.insert(row.key.clone(), v.clone());
            vec![Msg::run("font.info.set", with(target.params(), vec![("values", Json::Object(values))]))]
        }
        (RowAction::Edit(Edit::Language { language, text }), _) => {
            localized("font.info.setLocalizedValue", target, vec![("language", json!(language)), ("value", json!(text))])
        }
        (RowAction::Edit(Edit::AddLanguage(language)), _) => {
            let extra = vec![("language", json!(language)), ("value", json!("")), ("onlyMissing", json!(true))];
            localized("font.info.setLocalizedValue", target, extra)
        }
        (RowAction::Edit(Edit::RemoveLanguage(language)), _) => {
            localized("font.info.removeLocalizedValue", &holders, vec![("language", json!(language))])
        }
        (RowAction::Edit(Edit::ChangeLanguage { from, to }), _) => {
            localized("font.info.setLocalizedValue", &holders, vec![("from", json!(from)), ("language", json!(to))])
        }
        (RowAction::Edit(Edit::Dialog(dialog)), _) => dialog_message(&holders, row, *dialog).into_iter().collect(),
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
