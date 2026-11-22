//! Rename Glyphs by Name List (row `dialog.renameGlyphsByNamelist`): the dialog of Font › Rename Glyphs by Name List…
//! over `font.renameGlyphsByNamelist`.
//!
//! | Field | Parameter |
//! |---|---|
//! | Names From: A File, Pasted Text, Nice Names, Production Names | which of `namelist` or `text` is sent |
//! | Name List File | `namelist`, a file's path |
//! | Name List | `text`, the lines of a name list |
//! | Update References | `updateReferences` |
//! | Renames | none: what OK will rename, old name, new name and the problem of a clash ([`preview`]) |
//!
//! Nothing changes while the dialog is open (run on OK): the Renames list is read from the font after every change
//! with `cmd::encodings::preview_renames`, the function the command itself checks with, so the list shows exactly what
//! OK does. A list with a problem is refused whole, and the dialog stays open with the message. From the menu it
//! renames the glyphs selected in Font View; with none selected, and without `glyphs` in `dialog.open`'s params, it
//! renames every glyph of the font (never only the glyph the edit view shows).

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, keyed};
use craft::ui::contract::{ColumnSpec, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::table::Table;
use egui::{Response, Ui};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::encodings::{NameSource, PlannedRename, preview_renames};
use serde_json::{Map, Value, json};

use super::fields::{Custom, Field};
use super::spec::{DialogSpec, Fields, Shaped};

/// The command the dialog runs.
pub const COMMAND: &str = "font.renameGlyphsByNamelist";
/// The field keys.
pub const SOURCE: &str = "source";
pub const FILE: &str = "file";
pub const TEXT: &str = "text";
pub const UPDATE: &str = "updateReferences";
pub const PREVIEW: &str = "preview";
/// Longest pasted name list, in characters.
const MAX_TEXT_CHARS: usize = 1_000_000;
/// Most renames the Renames list holds; the count above it says how many there are in all.
pub const MAX_PREVIEW_ROWS: usize = 2_000;
/// Rows the list shows before it scrolls.
const LIST_ROWS: u16 = 10;

const FROM: ColumnKey = ColumnKey::new("from");
const TO: ColumnKey = ColumnKey::new("to");
const PROBLEM: ColumnKey = ColumnKey::new("problem");

craft::ui::words! {
    /// The words of the Renames list.
    pub struct RenameWords {
        renames: "fontcraftRenameByNamelist.renames" = "Renames";
        from: "fontcraftRenameByNamelist.from" = "Glyph";
        to: "fontcraftRenameByNamelist.to" = "New Name";
        problem: "fontcraftRenameByNamelist.problem" = "Problem";
        nothing: "fontcraftRenameByNamelist.nothing" = "No glyph would be renamed.";
        count: "fontcraftRenameByNamelist.count" = "{count} glyphs would be renamed; the first {shown} are listed.";
    }
}

pub const SPECS: &[DialogSpec] = &[DialogSpec::new(
    "dialog.renameGlyphsByNamelist",
    "Rename Glyphs by Name List",
    COMMAND,
    Fields::Shaped(Shaped { fields, given, params, link }),
)
.run_on_ok()
.buttons("Rename", "")
.annotate(preview)];

/// The Renames list: the value is `{renames: [{from, to, problem}], count, error}`, filled by [`preview`].
pub const PREVIEW_KIND: Custom = Custom { name: "renamePreview", check, row: Some(row) };

fn fields(_: &Session) -> Vec<Field> {
    let sources = [("file", "A File"), ("text", "Pasted Text"), ("nice", "Nice Names"), ("production", "Production Names")];
    vec![
        Field::choice(
            SOURCE,
            "Names From",
            &sources,
            "file",
            "Where the new names come from: a name-list file, pasted lines, or the glyph data's nice or production names",
        ),
        Field::open_file(FILE, "Name List File", &["nam", "txt"], "A name-list file: lines 0x0041 A, U+0041 A or A;0041"),
        Field::text_area(TEXT, "Name List", MAX_TEXT_CHARS, 6, "The lines of a name list, pasted: 0x0041 A, U+0041 A or A;0041"),
        Field::bool(UPDATE, "Update References", true, "Components, metrics keys, kerning, groups and the glyph order follow the new names"),
        Field::custom(
            PREVIEW,
            "Renames",
            PREVIEW_KIND,
            Value::Null,
            json!({"renames": [], "count": 0, "error": null}),
            "What OK renames: each glyph's old and new name, and why a rename cannot be made",
        ),
    ]
}

/// `dialog.open`'s params in the command's shape: `namelist` (nice, production or a path) or `text` picks the
/// source; `glyphs` passes through, and without it every glyph is renamed.
fn given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (key, value) in p {
        match key.as_str() {
            "namelist" => match value.as_str() {
                Some(list @ ("nice" | "production")) => {
                    out.insert(SOURCE.into(), json!(list));
                }
                Some(path) => {
                    out.insert(SOURCE.into(), json!("file"));
                    out.insert(FILE.into(), json!(path));
                }
                None => return Err("`namelist` must be a path, nice or production".into()),
            },
            TEXT => {
                out.insert(SOURCE.into(), json!("text"));
                out.insert(TEXT.into(), value.clone());
            }
            SOURCE | FILE | UPDATE | "glyphs" => {
                out.insert(key.clone(), value.clone());
            }
            other => return Err(format!("unknown parameter `{other}`: the dialog takes namelist, text, glyphs and updateReferences")),
        }
    }
    // A null `glyphs` is every glyph: it keeps the dialog from taking the selection as its targets.
    out.entry("glyphs").or_insert(Value::Null);
    Ok(out)
}

fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    match v.get(SOURCE).and_then(Value::as_str).unwrap_or("file") {
        "text" => {
            out.insert(TEXT.into(), v.get(TEXT).cloned().unwrap_or_else(|| json!("")));
        }
        list @ ("nice" | "production") => {
            out.insert("namelist".into(), json!(list));
        }
        _ => {
            if let Some(path) = v.get(FILE) {
                out.insert("namelist".into(), path.clone());
            }
        }
    }
    out.insert(UPDATE.into(), v.get(UPDATE).cloned().unwrap_or(json!(true)));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

/// The Renames list for the command's parameters `params`: the renames OK would make, at most [`MAX_PREVIEW_ROWS`],
/// how many there are, and why the source cannot be used.
pub fn preview(session: &Session, params: &Value) -> Map<String, Value> {
    let shown = match planned(session, params) {
        Ok(list) => {
            let count = list.len();
            let rows: Vec<Value> =
                list.into_iter().take(MAX_PREVIEW_ROWS).map(|r| json!({"from": r.from, "to": r.to, "problem": r.problem})).collect();
            json!({"renames": rows, "count": count, "error": null})
        }
        Err(e) => json!({"renames": [], "count": 0, "error": e}),
    };
    let mut out = Map::new();
    out.insert(PREVIEW.into(), shown);
    out
}

fn planned(session: &Session, params: &Value) -> Result<Vec<PlannedRename>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    let data = session.glyph_data().map_err(|e| e.to_string())?;
    let source = match (params.get("namelist").and_then(Value::as_str), params.get(TEXT).and_then(Value::as_str)) {
        (_, Some(text)) if text.trim().is_empty() => return Err("Paste the lines of a name list.".into()),
        (_, Some(text)) => NameSource::Text(text),
        (Some(list), None) => NameSource::List(list),
        (None, None) => return Err("Choose a name-list file.".into()),
    };
    let targets: Option<Vec<String>> =
        params.get("glyphs").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect());
    preview_renames(font, &data, source, targets.as_deref()).map_err(|e| e.to_string())?
}

fn check(field: &Field, _: &Value) -> Result<Value, String> {
    Err(format!("`{}` shows what OK will rename; it is not set", field.key))
}

/// One rename as the list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Shown {
    from: String,
    to: String,
    problem: String,
}

fn row(ui: &mut Ui, rows: &mut DialogFields, _: &mut CustomContext, field: &Field) -> Response {
    let words = Env::of(ui).words::<RenameWords>();
    let value = match rows.get(&field.key) {
        Some(FieldValue::Json(v)) => v.clone(),
        _ => Value::Null,
    };
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or_default().to_string();
    let renames: Vec<Shown> = value
        .get("renames")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(|r| Shown { from: text(r.get("from")), to: text(r.get("to")), problem: text(r.get("problem")) }).collect())
        .unwrap_or_default();
    let count = value.get("count").and_then(Value::as_u64).unwrap_or(0);
    let error = value.get("error").and_then(Value::as_str).map(str::to_string);
    let shown = Stack::new().show(ui, |ui, _| {
        if let Some(e) = &error {
            let _ = Text::new(Label::from(e.clone())).shade(Shade::Muted).show(ui);
            return;
        }
        if renames.is_empty() {
            let _ = Text::new(words.nothing.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        if count > renames.len() as u64 {
            let line = words.count.as_str().replace("{count}", &count.to_string()).replace("{shown}", &renames.len().to_string());
            let _ = Text::new(Label::from(line)).shade(Shade::Muted).show(ui);
        }
        let columns = vec![
            ColumnSpec::new(FROM, words.from.clone(), ColumnWidth::Content),
            ColumnSpec::new(TO, words.to.clone(), ColumnWidth::Content),
            ColumnSpec::new(PROBLEM, words.problem.clone(), ColumnWidth::Content),
        ];
        let source = keyed(&renames, |r| r.from.clone()).with_text(|r| Cow::Owned(r.from.clone()));
        let _ = Table::<String>::new(words.renames.clone(), columns).max_rows(LIST_ROWS).striped(true).show_items(ui, &source, &mut |from, icx| {
            let Some(r) = renames.iter().find(|r| r.from == *from) else { return ListRow::new(from.clone()) };
            let mut list_row = ListRow::new(r.from.clone()).cells(icx.cells()).cell(TO, r.to.clone()).cell(PROBLEM, r.problem.clone());
            // A rename that cannot be made: its old and new names are muted beside the problem.
            if !r.problem.is_empty() {
                list_row = list_row.cell_muted(TO);
            }
            list_row
        });
    });
    shown.output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg};
    use crate::dialogs::form;

    /// The new font: A to Z and a to z, each at its code point.
    fn app() -> FontCraftApp {
        FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()))
    }

    fn names(app: &FontCraftApp) -> Vec<String> {
        app.session.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect()
    }

    /// The pasted list previews its renames with nothing changed; OK renames exactly as the command does; Cancel
    /// changes nothing.
    #[test]
    fn the_dialog_previews_and_equals_its_command() {
        let mut by_dialog = app();
        let before = names(&by_dialog);
        let open = json!({"command": "font.renameGlyphsByNamelist", "params": {"text": "0x0041 Alpha\n"}});
        let shown = form::open_command(&mut by_dialog, &open).unwrap();
        let preview = shown["fields"].as_array().unwrap().iter().find(|f| f["key"] == "preview").unwrap().clone();
        assert_eq!(preview["value"]["renames"], json!([{"from": "A", "to": "Alpha", "problem": null}]), "{shown}");
        assert_eq!(names(&by_dialog), before, "nothing changes while the dialog is open");
        form::cancel(&mut by_dialog).unwrap();
        assert_eq!(names(&by_dialog), before, "Cancel changes nothing");

        form::open_command(&mut by_dialog, &open).unwrap();
        form::confirm(&mut by_dialog).unwrap();
        let mut by_command = app();
        by_command.run_now(Msg::run("font.renameGlyphsByNamelist", json!({"text": "0x0041 Alpha\n"})), false).unwrap();
        assert_eq!(names(&by_dialog), names(&by_command));
        assert!(names(&by_dialog).contains(&"Alpha".to_string()));
    }

    /// A name that is taken is marked in the preview, and OK is refused with the dialog left open and the font as
    /// it was.
    #[test]
    fn a_clash_is_marked_and_refused() {
        let mut app = app();
        let before = names(&app);
        let open = json!({"command": "font.renameGlyphsByNamelist", "params": {}});
        form::open_command(&mut app, &open).unwrap();
        form::set(&mut app, "source", &json!("text")).unwrap();
        let shown = form::set(&mut app, "text", &json!("0x0041 B\n")).unwrap();
        let preview = shown["fields"].as_array().unwrap().iter().find(|f| f["key"] == "preview").unwrap().clone();
        assert_eq!(preview["value"]["renames"][0]["problem"], json!("the name is taken"), "{shown}");
        assert!(form::confirm(&mut app).is_err(), "a clash refuses the whole list");
        assert!(app.dialogs.open.is_some(), "the dialog stays open with the message");
        assert_eq!(names(&app), before);
        form::cancel(&mut app).unwrap();
    }
}
