//! File › Import › Metrics…: one dialog with a tab for a metrics or AFM file
//! (`dialog.importMetricsFile`) and a tab for a font (`dialog.importMetricsFont`: a Glyphs or UFO
//! file, or another open document). Both run `file.importMetrics` once when Import is pressed,
//! as one undo step; Cancel changes nothing.
//!
//! The master field opens on the master the toolbar shows. Selected Glyphs limits the import to
//! the glyphs selected in Font View, and to every glyph when none is selected; unticked, it
//! passes an empty list, which is every glyph.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::form::{Field, Kind, MAX_CHOICES, MAX_PATH_TEXT};
use super::{Dialog, Fields, Mode, Shaped, Tab};
use crate::app::FontcraftApp;

/// The import dialog: opens on the file tab.
pub const DIALOG: Dialog = Dialog::new("dialog.importMetrics", "Import Metrics", "file.importMetrics", Fields::Tabs(&TABS));

/// The tabs, by source kind.
pub const TABS: [Tab; 2] = [Tab { id: "file", label: "Metrics or AFM File", dialog: FILE }, Tab { id: "font", label: "Font", dialog: FONT }];

/// A kerning-text or AFM file: kerning, kerning classes and glyph metrics.
pub const FILE: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Import", "Cancel"],
    ..Dialog::new(
        "dialog.importMetricsFile",
        "Import Metrics",
        "file.importMetrics",
        Fields::Shaped(Shaped { fields: file_fields, given: file_given, params: file_params, link: no_link }),
    )
};

/// A font source or an open document: kerning and kerning classes.
pub const FONT: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Import", "Cancel"],
    ..Dialog::new(
        "dialog.importMetricsFont",
        "Import Metrics",
        "file.importMetrics",
        Fields::Shaped(Shaped { fields: font_fields, given: font_given, params: font_params, link: no_link }),
    )
};

/// The import modes, as `file.importMetrics` names them, with their captions.
const MODES: [(&str, &str); 4] = [
    ("lsbAndWidth", "LSB and Width (RSB follows)"),
    ("rsbAndWidth", "RSB and Width (LSB follows)"),
    ("bothSidebearings", "Both Sidebearings (width follows)"),
    ("widthCentered", "Width, Outline Centred"),
];

/// The id of the master the toolbar shows: the active master of the showing tab, else Font
/// View's master.
pub fn current_master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    match app.state.tab() {
        Some(tab) => crate::canvas::view::active_master(font, tab).map(|m| m.id.clone()),
        None => crate::shell::font_view::master(app),
    }
}

/// A pop-up of the active document's masters by name; the value is the master id, the first
/// master at first.
pub fn master_field(session: &Session, key: &str, label: &str, doc: &str) -> Field {
    let masters: Vec<(String, String)> =
        session.font().map(|f| f.masters.iter().take(MAX_CHOICES).map(|m| (m.id.clone(), m.name.clone())).collect()).unwrap_or_default();
    Field {
        key: key.into(),
        label: label.into(),
        default: json!(masters.first().map(|(id, _)| id.clone()).unwrap_or_default()),
        kind: Kind::Choice { choices: masters.iter().map(|(id, _)| id.clone()).collect(), labels: masters.into_iter().map(|(_, n)| n).collect() },
        doc: doc.into(),
        group: None,
    }
}

fn path_field() -> Field {
    Field::text("file", "File", MAX_PATH_TEXT, "The path of the file to import from")
}

fn into_master(session: &Session) -> Field {
    master_field(session, "master", "Into Master", "The master the spacing and kerning go into")
}

fn selected_glyphs() -> Field {
    Field::bool("selectedGlyphs", "Selected Glyphs", false, "Only the glyphs selected in Font View, or every glyph when none is selected")
}

fn file_fields(session: &Session) -> Vec<Field> {
    vec![
        path_field(),
        into_master(session),
        Field::bool("importKerning", "Import Kerning", true, "Replace the master's kerning pairs with the file's"),
        Field::bool("importKerningGroups", "Import Kerning Classes", true, "Set the glyphs' kerning groups as the file gives them (AFM has none)"),
        Field::bool("importMetrics", "Import Metrics", false, "Set the glyphs' widths and sidebearings in the mode below"),
        Field::choice("mode", "Mode", &MODES, "lsbAndWidth", "Which of the file's metrics are taken; the others follow from the outline"),
        Field::bool(
            "keepAlignedComponents",
            "Keep Aligned Components",
            false,
            "Keep automatically aligned components aligned (no effect until automatic alignment exists)",
        ),
        selected_glyphs(),
    ]
}

/// The other open documents as a pop-up: the value is the document's index, empty for a file.
fn document_field(session: &Session) -> Field {
    let active = session.doc().map(|d| d.id).ok();
    let mut choices = vec![String::new()];
    let mut labels = vec!["The file below".to_string()];
    for (i, d) in session.documents().iter().enumerate().filter(|(_, d)| Some(d.id) != active).take(MAX_CHOICES.saturating_sub(1)) {
        choices.push(i.to_string());
        labels.push(d.title.clone());
    }
    Field {
        key: "sourceDocument".into(),
        label: "Source".into(),
        kind: Kind::Choice { choices, labels },
        default: json!(""),
        doc: "Another open document to import from, or a .glyphs or UFO file".into(),
        group: None,
    }
}

fn font_fields(session: &Session) -> Vec<Field> {
    vec![
        document_field(session),
        path_field(),
        Field::text("sourceMaster", "Source Master", 255, "The id or name of the source's master; empty: its first master"),
        into_master(session),
        // Its own key, so the file tab's ticked box does not carry over to a source that is often a
        // .glyphs file.
        Field::bool(
            "kerningValues",
            "Import Kerning",
            false,
            "Replace the master's kerning pairs with the source master's (not from a .glyphs file)",
        ),
        Field::bool("importKerningGroups", "Import Kerning Classes", true, "Copy the kerning groups of the glyphs of the same name"),
        selected_glyphs(),
    ]
}

fn no_link(_: &mut Map<String, Value>, _: &str) {}

/// `glyphs: []` (every glyph) unless Selected Glyphs is ticked, when the selection the dialog
/// opened with stays.
fn glyph_scope(values: &Map<String, Value>, out: &mut Map<String, Value>) {
    if values.get("selectedGlyphs") != Some(&Value::Bool(true)) {
        out.insert("glyphs".into(), json!([]));
    }
}

/// A text field's value as a parameter: left out when empty.
fn text_param(values: &Map<String, Value>, key: &str, out: &mut Map<String, Value>) {
    if let Some(t) = values.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
        out.insert(key.into(), json!(t));
    }
}

fn copy(values: &Map<String, Value>, keys: &[&str], out: &mut Map<String, Value>) {
    for key in keys {
        if let Some(v) = values.get(*key) {
            out.insert((*key).into(), v.clone());
        }
    }
}

fn file_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    text_param(values, "file", &mut out);
    copy(values, &["master", "importKerning", "importKerningGroups", "keepAlignedComponents"], &mut out);
    let metrics = values.get("importMetrics") == Some(&Value::Bool(true));
    out.insert("importMetrics".into(), if metrics { values.get("mode").cloned().unwrap_or(json!("lsbAndWidth")) } else { json!(false) });
    glyph_scope(values, &mut out);
    out
}

/// Field values from parameters in the command's shape: `importMetrics: "mode"` ticks Import
/// Metrics and picks the mode; a `glyphs` list ticks Selected Glyphs and stays the target.
fn file_given(_: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = given.clone();
    match given.get("importMetrics") {
        Some(Value::String(mode)) => {
            out.insert("importMetrics".into(), json!(true));
            out.insert("mode".into(), json!(mode));
        }
        Some(Value::Bool(_)) | None => {}
        Some(_) => return Err("`importMetrics` must be false or a mode name".into()),
    }
    given_scope(given, &mut out);
    Ok(out)
}

fn given_scope(given: &Map<String, Value>, out: &mut Map<String, Value>) {
    if let Some(list) = given.get("glyphs").and_then(Value::as_array) {
        out.insert("selectedGlyphs".into(), json!(!list.is_empty()));
    }
}

fn font_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    match values.get("sourceDocument").and_then(Value::as_str).and_then(|i| i.parse::<u64>().ok()) {
        Some(i) => {
            out.insert("sourceDocument".into(), json!(i));
        }
        None => text_param(values, "file", &mut out),
    }
    text_param(values, "sourceMaster", &mut out);
    copy(values, &["master", "importKerningGroups"], &mut out);
    out.insert("importKerning".into(), json!(values.get("kerningValues") == Some(&Value::Bool(true))));
    glyph_scope(values, &mut out);
    out
}

fn font_given(_: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = given.clone();
    match given.get("sourceDocument") {
        Some(Value::Number(n)) => {
            out.insert("sourceDocument".into(), json!(n.to_string()));
        }
        Some(Value::String(_)) | None => {}
        Some(_) => return Err("`sourceDocument` must be the index of an open document".into()),
    }
    if let Some(v) = out.remove("importKerning") {
        out.insert("kerningValues".into(), v);
    }
    given_scope(given, &mut out);
    Ok(out)
}
