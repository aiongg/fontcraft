//! Status Definitions (row `dialog.statusDefinitions`): the dialog of Font › Status Definitions… over
//! `font.setStatusDefinitions`.
//!
//! Its one field, `statuses`, is the whole list as OK sends it, each colour as `#RRGGBBAA`:
//! `[{code, name, color, isDefault}]`. It opens on the font's definitions. Each status is a row of its name, code,
//! colour, a Default checkbox (ticking one unticks the others) and Remove; Add Status adds one with the next free
//! code. Nothing changes while the dialog is open (run on OK): OK runs the command once, one undo step, and a list the
//! command refuses (a code or name used twice) keeps the dialog open with the message. Cancel changes nothing.

use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, FieldLayout, Show as _, ShowContainer as _, SideLabel};
use craft::ui::form::Field as FormField;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::{Response, Ui};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::status::{MAX_STATUS_CODE, MAX_STATUS_DEFINITIONS, StatusDef, definitions};
use serde_json::{Map, Value, json};

use super::fields::{Custom, Field};
use super::spec::{DialogSpec, Fields, Shaped};
use crate::app::models::color::{Rgba8, hex, parse_hex};
use crate::panels::attributes::color_cell;
use crate::panels::widgets::bare_number;

/// The field holding the list.
pub const STATUSES: &str = "statuses";
/// The colour a new status starts in.
const NEW_COLOR: Rgba8 = [0x88, 0x88, 0x88, 0xff];
/// Longest status name, in characters.
const MAX_NAME_CHARS: usize = 64;

craft::ui::words! {
    /// The words of the Status Definitions dialog.
    pub struct StatusWords {
        status: "fontcraftStatusDefinitions.status" = "Status {n}";
        name: "fontcraftStatusDefinitions.name" = "Name";
        code: "fontcraftStatusDefinitions.code" = "Code";
        color: "fontcraftStatusDefinitions.color" = "Color";
        default: "fontcraftStatusDefinitions.default" = "Default";
        remove: "fontcraftStatusDefinitions.remove" = "Remove";
        add: "fontcraftStatusDefinitions.add" = "Add Status";
        none: "fontcraftStatusDefinitions.none" = "No statuses are defined.";
    }
}

pub const SPECS: &[DialogSpec] = &[DialogSpec::new(
    "dialog.statusDefinitions",
    "Status Definitions",
    "font.setStatusDefinitions",
    Fields::Shaped(Shaped { fields, given, params, link }),
)
.run_on_ok()];

/// The list editor.
pub const STATUSES_KIND: Custom = Custom { name: "statusDefinitions", check, row: Some(row) };

/// One status as the field keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub code: i64,
    pub name: String,
    pub color: Rgba8,
    pub is_default: bool,
}

impl Entry {
    fn of(def: &StatusDef) -> Entry {
        let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        let [r, g, b, a] = def.color;
        Entry { code: def.code, name: def.name.clone(), color: [byte(r), byte(g), byte(b), byte(a)], is_default: def.is_default }
    }

    fn to_json(&self) -> Value {
        json!({"code": self.code, "name": self.name, "color": hex(self.color), "isDefault": self.is_default})
    }

    /// The entry as `font.setStatusDefinitions` takes it: the colour as four numbers from 0 to 1.
    fn to_param(&self) -> Value {
        let unit = |c: u8| f64::from(c) / 255.0;
        let [r, g, b, a] = self.color;
        json!({"code": self.code, "name": self.name, "color": [unit(r), unit(g), unit(b), unit(a)], "isDefault": self.is_default})
    }

    fn parse(v: &Value) -> Result<Entry, String> {
        let code = v.get("code").and_then(Value::as_i64).filter(|c| (0..=MAX_STATUS_CODE).contains(c));
        let code = code.ok_or_else(|| format!("`code` must be a whole number from 0 to {MAX_STATUS_CODE}"))?;
        let name = v.get("name").and_then(Value::as_str).ok_or("`name` must be text")?;
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("a name has at most {MAX_NAME_CHARS} characters"));
        }
        let color = v.get("color").and_then(Value::as_str).and_then(parse_hex).ok_or("`color` must be written #RRGGBB or #RRGGBBAA")?;
        let is_default = match v.get("isDefault") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err("`isDefault` must be true or false".into()),
        };
        Ok(Entry { code, name: name.to_string(), color, is_default })
    }
}

/// The entries of a `statuses` value.
pub fn entries(value: &Value) -> Result<Vec<Entry>, String> {
    let list = value.as_array().ok_or("the statuses are not a list")?;
    if list.len() > MAX_STATUS_DEFINITIONS {
        return Err(format!("at most {MAX_STATUS_DEFINITIONS} statuses can be defined"));
    }
    list.iter().enumerate().map(|(i, v)| Entry::parse(v).map_err(|e| format!("status {}: {e}", i + 1))).collect()
}

fn to_value(list: &[Entry]) -> Value {
    Value::Array(list.iter().map(Entry::to_json).collect())
}

fn fields(session: &Session) -> Vec<Field> {
    let defs = session.font().map(definitions).unwrap_or_default();
    let list: Vec<Entry> = defs.iter().map(Entry::of).collect();
    vec![Field::custom(
        STATUSES,
        "",
        STATUSES_KIND,
        Value::Null,
        to_value(&list),
        "The font's statuses: [{code, name, color (#RRGGBBAA), isDefault}]; codes and names unique, at most one default",
    )]
}

fn given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if let Some(key) = p.keys().find(|k| k.as_str() != STATUSES) {
        return Err(format!("unknown parameter `{key}`: the dialog takes `{STATUSES}`"));
    }
    Ok(p.clone())
}

fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let list = v.get(STATUSES).map(entries).and_then(Result::ok).unwrap_or_default();
    let mut out = Map::new();
    out.insert(STATUSES.into(), Value::Array(list.iter().map(Entry::to_param).collect()));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

fn check(field: &Field, value: &Value) -> Result<Value, String> {
    entries(value).map(|list| to_value(&list)).map_err(|e| format!("`{}`: {e}", field.key))
}

/// A new status after `list`: the next free code and a name no other status has.
pub fn added(list: &[Entry]) -> Entry {
    let code = list.iter().map(|e| e.code).max().map_or(0, |c| c.saturating_add(1)).min(MAX_STATUS_CODE);
    let mut n = list.len() + 1;
    let name = loop {
        let name = format!("Status {n}");
        if !list.iter().any(|e| e.name == name) || n > MAX_STATUS_DEFINITIONS * 2 {
            break name;
        }
        n += 1;
    };
    Entry { code, name, color: NEW_COLOR, is_default: false }
}

/// One edit the rows made.
enum Edit {
    Name(usize, String),
    Code(usize, i64),
    Color(usize, Rgba8),
    Default(usize),
    Remove(usize),
    Add,
}

fn apply(mut list: Vec<Entry>, edit: Edit) -> Vec<Entry> {
    match edit {
        Edit::Name(i, name) => {
            if let Some(e) = list.get_mut(i) {
                e.name = name.trim().chars().take(MAX_NAME_CHARS).collect();
            }
        }
        Edit::Code(i, code) => {
            if let Some(e) = list.get_mut(i) {
                e.code = code.clamp(0, MAX_STATUS_CODE);
            }
        }
        Edit::Color(i, color) => {
            if let Some(e) = list.get_mut(i) {
                e.color = color;
            }
        }
        Edit::Default(i) => {
            let was = list.get(i).is_some_and(|e| e.is_default);
            for (at, e) in list.iter_mut().enumerate() {
                e.is_default = at == i && !was;
            }
        }
        Edit::Remove(i) => {
            if i < list.len() {
                list.remove(i);
            }
        }
        Edit::Add => {
            if list.len() < MAX_STATUS_DEFINITIONS {
                let next = added(&list);
                list.push(next);
            }
        }
    }
    list
}

fn row(ui: &mut Ui, rows: &mut DialogFields, _: &mut CustomContext, field: &Field) -> Response {
    let words = Env::of(ui).words::<StatusWords>();
    let list: Vec<Entry> = match rows.get(&field.key) {
        Some(FieldValue::Json(v)) => entries(v).unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut edit = None;
    let shown = Stack::new().show(ui, |ui, _| {
        if list.is_empty() {
            let _ = craft::ui::display::Text::new(words.none.clone()).shade(craft::ui::display::Shade::Muted).show(ui);
        }
        for (i, e) in list.iter().enumerate() {
            let caption = Label::from(words.status.as_str().replace("{n}", &(i + 1).to_string()));
            let _ = FormField::new(caption).show(ui, |ui, _| {
                let _ = Row::new().wrap(true).show(ui, |ui, cx| {
                    let out = TextField::new(words.name.clone(), Bind::given(e.name.clone())).show(ui);
                    if let Some(text) = out.events.committed()
                        && text.trim() != e.name
                    {
                        edit = Some(Edit::Name(i, text.clone()));
                    }
                    if let Some(code) = bare_number(ui, words.code.clone(), Some(e.code as f64))
                        && code.is_finite()
                        && code.fract() == 0.0
                    {
                        edit = Some(Edit::Code(i, code as i64));
                    }
                    let (picked, _) = color_cell(ui, &words.color, Some(e.color), ("fontcraftStatusDefinitions.color", i));
                    if let Some(c) = picked {
                        edit = Some(Edit::Color(i, c));
                    }
                    // A box in the row's label scope draws only its mark, so its word comes from the row's label step.
                    let mut on = e.is_default;
                    let side = SideLabel::new().layout(FieldLayout::After);
                    let _ =
                        cx.labelled(ui, ("fontcraftStatusDefinitions.default", i), words.default.clone(), side, Checkbox::new(Bind::bound(&mut on)));
                    if on != e.is_default {
                        edit = Some(Edit::Default(i));
                    }
                    if Button::new(words.remove.clone()).show(ui).clicked() {
                        edit = Some(Edit::Remove(i));
                    }
                });
            });
        }
        if list.len() < MAX_STATUS_DEFINITIONS && Button::new(words.add.clone()).show(ui).clicked() {
            edit = Some(Edit::Add);
        }
    });
    if let Some(edit) = edit {
        rows.set(field.key.clone(), FieldValue::Json(to_value(&apply(list, edit))));
    }
    shown.output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use fontcraft_engine::cmd::status::definitions;
    use serde_json::json;

    use super::{Edit, Entry, apply};
    use crate::app::{APP, FontCraftApp, Msg};
    use crate::dialogs::form;

    fn app() -> FontCraftApp {
        FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()))
    }

    /// OK sends the list as the command takes it; the font then holds what the command alone would make. Cancel
    /// changes nothing.
    #[test]
    fn the_dialog_equals_its_command_and_cancel_changes_nothing() {
        let list = json!([
            {"code": 0, "name": "Draft", "color": "#FF0000FF", "isDefault": true},
            {"code": 1, "name": "Final", "color": "#00FF00FF", "isDefault": false},
        ]);
        let mut by_dialog = app();
        let open = json!({"command": "font.setStatusDefinitions", "params": {}});
        let shown = form::open_command(&mut by_dialog, &open).unwrap();
        assert_eq!(shown["fields"][0]["value"], json!([]), "a font without definitions opens on an empty list");
        form::set(&mut by_dialog, "statuses", &list).unwrap();
        form::cancel(&mut by_dialog).unwrap();
        assert!(definitions(by_dialog.session.font().unwrap()).is_empty(), "Cancel changes nothing");

        form::open_command(&mut by_dialog, &open).unwrap();
        form::set(&mut by_dialog, "statuses", &list).unwrap();
        form::confirm(&mut by_dialog).unwrap();
        let mut by_command = app();
        let statuses = json!([
            {"code": 0, "name": "Draft", "color": [1.0, 0.0, 0.0, 1.0], "isDefault": true},
            {"code": 1, "name": "Final", "color": [0.0, 1.0, 0.0, 1.0], "isDefault": false},
        ]);
        by_command.run_now(Msg::run("font.setStatusDefinitions", json!({"statuses": statuses})), false).unwrap();
        assert_eq!(definitions(by_dialog.session.font().unwrap()), definitions(by_command.session.font().unwrap()));

        // It opens again on the font's list.
        let shown = form::open_command(&mut by_dialog, &open).unwrap();
        assert_eq!(shown["fields"][0]["value"], list);
        form::cancel(&mut by_dialog).unwrap();
    }

    /// A list the command refuses keeps the dialog open and the font as it was.
    #[test]
    fn a_refused_list_keeps_the_dialog_open() {
        let mut app = app();
        form::open_command(&mut app, &json!({"command": "font.setStatusDefinitions", "params": {}})).unwrap();
        let twice = json!([{"code": 0, "name": "A", "color": "#000000FF"}, {"code": 0, "name": "B", "color": "#000000FF"}]);
        form::set(&mut app, "statuses", &twice).unwrap();
        assert!(form::confirm(&mut app).is_err());
        assert!(app.dialogs.open.is_some());
        assert!(definitions(app.session.font().unwrap()).is_empty());
    }

    /// Add Status takes the next code and a free name; Default keeps at most one default.
    #[test]
    fn the_rows_add_and_keep_one_default() {
        let list = apply(Vec::new(), Edit::Add);
        let list = apply(list, Edit::Add);
        assert_eq!(list.iter().map(|e| (e.code, e.name.as_str())).collect::<Vec<_>>(), vec![(0, "Status 1"), (1, "Status 2")]);
        let list = apply(list, Edit::Default(0));
        let list = apply(list, Edit::Default(1));
        assert_eq!(list.iter().map(|e: &Entry| e.is_default).collect::<Vec<_>>(), vec![false, true]);
        let list = apply(list, Edit::Default(1));
        assert!(list.iter().all(|e| !e.is_default), "ticking the default again unticks it");
    }
}
