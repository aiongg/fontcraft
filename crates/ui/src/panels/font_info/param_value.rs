//! The value editor of each row ([`Editor`]): a check box, a number or text field, a field of several lines, a menu
//! of the parameter's values, check boxes for a list of them, a menu of the font's masters, a field per dictionary
//! entry or language, or a summary with an Edit button for a value that has a dialog of its own.
//!
//! An edit leaves as an [`Edit`]: the row turns it into a command. A typed parameter value goes through the check
//! the command runs (`fontcraft_fontinfo::value::check`) first: a refused value stays in its field, its message shows
//! under the row, and nothing is sent.

use craft::behavior::IconId;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Validity};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label, Message};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use fontcraft_engine::fontinfo::{catalog, value as fv};
use serde_json::{Map, Value as Json, json};

use super::FontInfoWords;
use crate::app::models::font_info::{DictEntry, Editor, RowDialog, RowModel, RowSort};

/// Lines the list field shows.
const LIST_LINES: usize = 4;

/// What the user did to a row's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// A new value, as JSON.
    Value(Json),
    /// The Edit button of a value with a dialog.
    Dialog(RowDialog),
    /// New text for one language of a localizable field.
    Language {
        language: String,
        text: String,
    },
    /// Another language picked for one of a localizable field's texts.
    ChangeLanguage {
        from: String,
        to: String,
    },
    AddLanguage(String),
    RemoveLanguage(String),
}

/// Draws the editor of `row`; `masters` are the font's masters as (id, name). Returns what the user did.
pub fn editor(ui: &mut Ui, row: &RowModel, masters: &[(String, String)], language_list: &[Entry<String>]) -> Option<Edit> {
    let words = Env::of(ui).words::<FontInfoWords>();
    let label: Label = row.label.clone().into();
    let name = row.key.as_str();
    let param = row.sort == RowSort::Param;
    let mixed = row.mixed;
    match &row.editor {
        Editor::Bool(on) => {
            let out = Checkbox::new(Bind::given(*on && !mixed)).label(label).show(ui);
            out.events.committed().map(|on| Edit::Value(json!(on)))
        }
        Editor::Number(value) => {
            let shown = if mixed { None } else { *value };
            let check = param.then_some(name);
            let bind = Bind::given(shown).check(move |v: &Option<f64>| v.map_or(Validity::Ok, |n| validity(check, &number_json(n))));
            let out = NumberField::new(label, bind).show(ui);
            let Some(Some(n)) = out.edit.events.committed() else { return None };
            let typed = number_json(*n);
            (!param || refusal(name, &typed).is_none()).then_some(Edit::Value(typed))
        }
        Editor::Text(text) => {
            text_field(ui, label, if mixed { "" } else { text }, param.then_some(name), mixed, &words.multiple).map(|t| Edit::Value(Json::String(t)))
        }
        Editor::Lines(text) => {
            let shown = if mixed { String::new() } else { text.clone() };
            let check = param.then_some(name);
            let bind = Bind::given(shown).check(move |t: &String| validity(check, &Json::String(t.clone())));
            let out = TextField::new(label, bind).lines(LIST_LINES).show(ui);
            let typed = out.events.committed()?;
            (check.is_none_or(|n| refusal(n, &Json::String(typed.clone())).is_none())).then(|| Edit::Value(Json::String(typed.clone())))
        }
        Editor::Choice { choices, value } => {
            let entries: Vec<Entry<String>> = choices.iter().map(|c| Entry::new(c.clone(), c.clone())).collect();
            let out = Select::new(label, Bind::given(value.clone()), &entries).placeholder(words.choose.clone()).show(ui);
            out.events.committed().map(|picked| Edit::Value(choice_json(picked)))
        }
        Editor::Choices { choices, on } => {
            let mut edit = None;
            Stack::new().show(ui, |ui, _| {
                for choice in choices {
                    let checked = on.iter().any(|o| o.eq_ignore_ascii_case(choice));
                    let out = Checkbox::new(Bind::given(checked)).label(choice.clone()).id_salt((name, choice.as_str())).show(ui);
                    if let Some(now) = out.events.committed() {
                        let list: Vec<&String> =
                            choices.iter().filter(|c| if *c == choice { *now } else { on.iter().any(|o| o.eq_ignore_ascii_case(c)) }).collect();
                        edit = Some(Edit::Value(json!(list)));
                    }
                }
            });
            edit
        }
        Editor::Master(id) => {
            let entries: Vec<Entry<String>> = masters.iter().map(|(id, name)| Entry::new(id.clone(), name.clone())).collect();
            let out = Select::new(label, Bind::given(id.clone()), &entries).placeholder(words.choose.clone()).show(ui);
            out.events.committed().map(|picked| Edit::Value(Json::String(picked.clone())))
        }
        Editor::Dict(entries) => dict(ui, name, entries, mixed, &words),
        Editor::Localized(entries) => localized(ui, name, entries, mixed, &words),
        Editor::Languages(entries) => languages(ui, name, entries, language_list, mixed, &words),
        Editor::Dialog { summary, dialog } => {
            let mut edit = None;
            Row::new().show(ui, |ui, _| {
                Text::new(if mixed { words.multiple.clone() } else { summary.clone().into() }).shade(Shade::Muted).show(ui);
                if Button::new(words.edit.clone()).tooltip(words.edit_tip.clone()).id_salt((name, "dialog")).show(ui).clicked() {
                    edit = Some(Edit::Dialog(*dialog));
                }
            });
            edit
        }
        Editor::Summary(text) => {
            Text::new(text.clone()).shade(Shade::Muted).show(ui);
            None
        }
    }
}

/// A one-line text field; with a parameter `check`, the value goes through the command's check first.
fn text_field(ui: &mut Ui, label: Label, shown: &str, check: Option<&str>, mixed: bool, multiple: &Label) -> Option<String> {
    let bind = Bind::given(shown.to_string()).check(move |t: &String| validity(check, &Json::String(t.clone())));
    let mut field = TextField::new(label, bind);
    if mixed {
        field = field.placeholder(multiple.clone());
    }
    let typed = field.show(ui).events.committed()?.clone();
    check.is_none_or(|n| refusal(n, &Json::String(typed.clone())).is_none()).then_some(typed)
}

/// What the command's check says about `value` for the parameter `name`; always fine without a name.
fn validity<T>(name: Option<&str>, value: &Json) -> Validity<T> {
    match name.and_then(|n| refusal(n, value)) {
        Some(message) => Validity::Refuse(Message::new(message)),
        None => Validity::Ok,
    }
}

/// Why `font.customParameters.set` would refuse `value` for the parameter `name`, if it would.
pub fn refusal(name: &str, value: &Json) -> Option<String> {
    let checked = fv::from_json(name, value).map_err(|e| e.to_string()).and_then(|v| match catalog::lookup(name) {
        Some(spec) => fv::check(spec, &v).map(|_| ()).map_err(|e| e.to_string()),
        None => Ok(()),
    });
    checked.err()
}

/// A number as the property list holds it: an integer when it is whole.
pub fn number_json(n: f64) -> Json {
    if n.fract() == 0.0 && n.abs() < 9.0e15 { json!(n as i64) } else { json!(n) }
}

/// A picked choice: a whole number stays a number.
fn choice_json(choice: &str) -> Json {
    choice.trim().parse::<i64>().map_or_else(|_| Json::String(choice.to_string()), |n| json!(n))
}

/// One field per entry whose value is a text or a number; the new dictionary when one changed.
fn dict(ui: &mut Ui, name: &str, entries: &[DictEntry], mixed: bool, words: &FontInfoWords) -> Option<Edit> {
    if mixed {
        Text::new(words.multiple.clone()).shade(Shade::Muted).show(ui);
        return None;
    }
    let mut changed = None;
    Stack::new().show(ui, |ui, _| {
        for (at, entry) in entries.iter().enumerate() {
            match &entry.text {
                Some(text) => {
                    let out = TextField::new(entry.key.clone(), Bind::given(text.clone())).id_salt((name, "entry", at)).show(ui);
                    if let Some(typed) = out.events.committed() {
                        let value = match entry.number {
                            true => {
                                typed.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| Json::String(typed.clone()), number_json)
                            }
                            false => Json::String(typed.clone()),
                        };
                        changed = Some((at, value));
                    }
                }
                None => {
                    Text::new(format!("{}: {}", entry.key, entry.json)).shade(Shade::Muted).show(ui);
                }
            }
        }
    });
    let (at, new) = changed?;
    let mut out = Map::new();
    for (i, entry) in entries.iter().enumerate() {
        let value = if i == at { new.clone() } else { serde_json::from_str(&entry.json).unwrap_or(Json::Null) };
        out.insert(entry.key.clone(), value);
    }
    let value = Json::Object(out);
    refusal(name, &value).is_none().then_some(Edit::Value(value))
}

/// One field per language of a localized parameter; the new list of `{language, value}` entries when one changed.
fn localized(ui: &mut Ui, name: &str, entries: &[(String, String)], mixed: bool, words: &FontInfoWords) -> Option<Edit> {
    if mixed {
        Text::new(words.multiple.clone()).shade(Shade::Muted).show(ui);
        return None;
    }
    let mut changed = None;
    Stack::new().show(ui, |ui, _| {
        for (at, (language, text)) in entries.iter().enumerate() {
            let out = TextField::new(language.clone(), Bind::given(text.clone())).id_salt((name, "language", at)).show(ui);
            if let Some(typed) = out.events.committed() {
                changed = Some((at, typed.clone()));
            }
        }
    });
    let (at, new) = changed?;
    let items: Vec<Json> = entries
        .iter()
        .enumerate()
        .map(|(i, (language, text))| json!({"language": language, "value": if i == at { new.as_str() } else { text.as_str() }}))
        .collect();
    Some(Edit::Value(Json::Array(items)))
}

/// One line per language of a localizable field: the language menu, the text and the minus button; then the plus
/// button that adds the next free language.
fn languages(ui: &mut Ui, name: &str, entries: &[(String, String)], all: &[Entry<String>], mixed: bool, words: &FontInfoWords) -> Option<Edit> {
    let mut edit = None;
    let next_free = all.iter().filter_map(|e| e.value()).find(|l| !entries.iter().any(|(e, _)| e == *l)).cloned();
    Stack::new().show(ui, |ui, _| {
        for (at, (language, text)) in entries.iter().enumerate() {
            Row::new().id_salt((name, "language", at)).show(ui, |ui, _| {
                let out = Select::new(words.language.clone(), Bind::given(language.clone()), all).searchable().show(ui);
                if let Some(to) = out.events.committed()
                    && to != language
                {
                    edit = Some(Edit::ChangeLanguage { from: language.clone(), to: to.clone() });
                }
                let mut field = TextField::new(language.clone(), Bind::given(if mixed { String::new() } else { text.clone() }));
                if mixed {
                    field = field.placeholder(words.multiple.clone());
                }
                if let Some(typed) = field.show(ui).events.committed() {
                    edit = Some(Edit::Language { language: language.clone(), text: typed.clone() });
                }
                if IconButton::new(IconId::new("minus"))
                    .label(words.remove_language.clone())
                    .tooltip(words.remove_language.clone())
                    .show(ui)
                    .clicked()
                {
                    edit = Some(Edit::RemoveLanguage(language.clone()));
                }
            });
        }
        if let Some(free) = &next_free
            && IconButton::new(IconId::new("plus"))
                .label(words.add_language.clone())
                .tooltip(words.add_language.clone())
                .id_salt((name, "add"))
                .show(ui)
                .clicked()
        {
            edit = Some(Edit::AddLanguage(free.clone()));
        }
    });
    edit
}
