//! The dialog that adds a master from another open font (M9.23).
//!
//! Rows: `fontInfo.addOtherFont`.
//!
//! Font Info › Masters › + › Add Other Font… lists every master of every open document, the
//! active one included, as checkboxes. OK copies the ticked masters into the active document
//! through `font.master.addFromOtherFont`, as one undo step; Cancel adds nothing. Nothing runs
//! before OK, as the new masters would not show behind the dialog anyway.
//!
//! Each checkbox's word is `<document position>/<master index>`, the positions in the session's
//! document list; `ui.dialog.set {field: "sources", value: ["1/0"]}` ticks the first master of
//! the second document. The labels name the document and the master.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::axis_location::no_link;
use super::form::{Field, Kind, MAX_CHOICES};
use super::{Dialog, Fields, Mode, Shaped};

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    ..Dialog::new(
        "fontInfo.addOtherFont",
        "Add Other Font",
        "font.master.addFromOtherFont",
        Fields::Shaped(Shaped { fields, given, params, link: no_link }),
    )
};

/// The masters of the open documents as `(word, label)` pairs, at most [`MAX_CHOICES`].
fn choices(session: &Session) -> Vec<(String, String)> {
    session
        .documents()
        .iter()
        .enumerate()
        .flat_map(|(d, doc)| {
            doc.font.masters.iter().enumerate().map(move |(m, master)| (format!("{d}/{m}"), format!("{} — {}", doc.title, master.name)))
        })
        .take(MAX_CHOICES)
        .collect()
}

fn fields(session: &Session) -> Vec<Field> {
    let (words, labels): (Vec<String>, Vec<String>) = choices(session).into_iter().unzip();
    vec![Field {
        key: "sources".into(),
        label: "Masters".into(),
        kind: Kind::Multi { choices: words, labels },
        default: json!([]),
        doc: "The masters to copy into this font, each added at the end of its master list".into(),
        group: None,
    }]
}

fn given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    session.font().map_err(|e| e.to_string())?;
    if choices(session).is_empty() {
        return Err("Add Other Font: no open font has a master".into());
    }
    Ok(given.clone())
}

/// `{sources: [{font, master}]}` from the ticked words.
fn params(values: &Map<String, Value>) -> Map<String, Value> {
    let sources: Vec<Value> = values
        .get("sources")
        .and_then(Value::as_array)
        .map(|words| {
            words
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|w| w.split_once('/'))
                .filter_map(|(d, m)| Some(json!({"font": d.parse::<u64>().ok()?, "master": m.parse::<u64>().ok()?})))
                .collect()
        })
        .unwrap_or_default();
    let mut m = Map::new();
    m.insert("sources".into(), Value::Array(sources));
    m
}
