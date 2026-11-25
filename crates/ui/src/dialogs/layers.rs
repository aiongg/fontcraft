//! The dialogs of commands over layers.
//!
//! - Assign Background (`dialog.assignBackground`): asks for the source (this font, another open document, or a font
//!   file) and runs `path.assignBackground` with `onUnmatchedMaster: "ask"`. When a master has no source master of its
//!   name, nothing changes and `dialog.assignBackgroundMasters` asks which source master each of them takes.
//! - Copy Layer to Layer (`dialog.copyLayerToLayer`) and Compare Layers (`dialog.compareLayers`): the two layers, each a
//!   master or a layer name. A name stands for the layer of that name in each glyph.
//!
//! After Compare Layers, Font View in front selects the glyphs that differ ([`after_run`]).

use std::collections::HashSet;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::cmd::background::assign_masters;
use fontcraft_engine::interp::labels::{brace_label, bracket_label};
use fontcraft_engine::model::{Font, Glyph};
use serde_json::{Map, Value, json};

use super::fields::{Field, Kind, MAX_CHOICES};
use super::spec::{After, DialogSpec, Fields, Shaped};
use crate::app::{FontCraftApp, Msg};

const ASSIGN: &str = "path.assignBackground";
const COPY: &str = "edit.copyLayerToLayer";
const COMPARE: &str = "font.compareLayers";

pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new(
        "dialog.assignBackground",
        "Assign Background",
        ASSIGN,
        Fields::Shaped(Shaped { fields: source_fields, given: source_given, params: source_params, link: |_, _| {} }),
    )
    .run_on_ok()
    .buttons("Assign", ""),
    DialogSpec::new("dialog.assignBackgroundMasters", "Assign Background: Masters", ASSIGN, Fields::TargetedShaped(master_fields, master_params))
        .run_on_ok()
        .after(After { command: ASSIGN, params: ask_masters })
        .buttons("Assign", "")
        .message("The source has no master of the same name as these. Choose the one each takes its outlines from."),
    DialogSpec::new("dialog.copyLayerToLayer", "Copy Layer to Layer", COPY, Fields::TargetedShaped(copy_fields, copy_params))
        .run_on_ok()
        .buttons("Copy", ""),
    DialogSpec::new("dialog.compareLayers", "Compare Layers", COMPARE, Fields::TargetedShaped(compare_fields, compare_params))
        .run_on_ok()
        .buttons("Compare", ""),
];

/// The source words of Assign Background besides a document's position.
const THIS_FONT: &str = "this";
const FILE: &str = "file";
/// Before a master's id in the key of its field.
const MASTER_KEY: &str = "master:";
/// Before a layer's id, and before a layer's name, in a layer choice.
const BY_ID: &str = "id:";
const BY_NAME: &str = "name:";
/// The largest tolerance Compare Layers offers.
const MAX_TOLERANCE: f64 = 1000.0;
/// Most glyph names the status line lists.
const MAX_LISTED: usize = 8;

fn choice(key: &str, label: &str, choices: Vec<(String, String)>, default: &str, doc: &str) -> Field {
    let (choices, labels): (Vec<String>, Vec<String>) = choices.into_iter().take(MAX_CHOICES).unzip();
    let default = if choices.iter().any(|c| c == default) { default.to_string() } else { choices.first().cloned().unwrap_or_default() };
    Field {
        key: key.into(),
        label: label.into(),
        kind: Kind::Choice { choices, labels },
        default: json!(default),
        doc: doc.into(),
        group: None,
        same_as: None,
    }
}

// ── Assign Background ───────────────────────────────────────────────────────────────────────────

fn source_fields(session: &Session) -> Vec<Field> {
    let active = session.doc().map(|d| d.id).ok();
    let mut sources = vec![(THIS_FONT.to_string(), "This Font".to_string()), (FILE.to_string(), "The File Below".to_string())];
    let others = session.documents().iter().enumerate().filter(|(_, d)| Some(d.id) != active);
    sources.extend(others.map(|(at, d)| (at.to_string(), d.title.clone())));
    vec![
        choice(
            "source",
            "Source",
            sources,
            THIS_FONT,
            "Where the outlines come from: this font (a copy of the current outlines), a font file or another open document",
        ),
        Field::open_file("file", "File", &["fontcraft", "glyphs", "otf", "ttf"], "The font file, when Source is The File Below"),
    ]
}

/// Field values from the command's parameters: `font` picks the document, `file` the file.
fn source_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = given.clone();
    let source = match (out.remove("font"), out.get("file")) {
        (Some(Value::Number(at)), _) => at.to_string(),
        (Some(Value::String(title)), _) => {
            let at = session.documents().iter().position(|d| d.title == title).ok_or_else(|| format!("no open document is called `{title}`"))?;
            at.to_string()
        }
        (Some(_), _) => return Err("`font` must be an open document's position or title".into()),
        (None, Some(_)) => FILE.to_string(),
        (None, None) => THIS_FONT.to_string(),
    };
    out.entry("source").or_insert_with(|| json!(source));
    Ok(out)
}

fn source_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    match values.get("source").and_then(Value::as_str) {
        Some(FILE) => {
            // An empty path reaches the command, which says it cannot read it: the font itself is not what was asked.
            out.insert("file".into(), values.get("file").cloned().unwrap_or_else(|| json!("")));
        }
        Some(at) => {
            if let Ok(at) = at.parse::<u64>() {
                out.insert("font".into(), json!(at));
            }
        }
        None => {}
    }
    out.insert("onUnmatchedMaster".into(), json!("ask"));
    out
}

/// The parameters to ask for the masters with, when `path.assignBackground` ran with `request` and returned masters
/// without one of their name in the source.
fn ask_masters(request: &Value, result: &Value) -> Option<Map<String, Value>> {
    result.get("unmatched")?.as_array().filter(|list| !list.is_empty())?;
    let mut given = request.as_object()?.clone();
    given.remove("onUnmatchedMaster");
    Some(given)
}

fn master_fields(session: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let (pairs, sources) = assign_masters(session, &Value::Object(targets.clone())).map_err(|e| e.to_string())?;
    let only = targets.get("master").and_then(Value::as_str);
    let fields: Vec<Field> = pairs
        .into_iter()
        .filter(|pair| !pair.by_name && only.is_none_or(|id| id == pair.master))
        .take(super::fields::MAX_FIELDS)
        .map(|pair| {
            let key = format!("{MASTER_KEY}{}", pair.master);
            choice(&key, &pair.name, sources.clone(), &pair.from, "The source master this master's backgrounds are copied from")
        })
        .collect();
    if fields.is_empty() { Err("every master has a source master of its name".into()) } else { Ok(fields) }
}

fn master_params(values: &Map<String, Value>) -> Map<String, Value> {
    let map: Map<String, Value> = values.iter().filter_map(|(k, v)| Some((k.strip_prefix(MASTER_KEY)?.to_string(), v.clone()))).collect();
    Map::from_iter([("masterMap".to_string(), Value::Object(map))])
}

// ── Copy Layer to Layer and Compare Layers ──────────────────────────────────────────────────────

/// The layers the glyphs of `targets` (every glyph without `glyphs`) offer: the masters, the named layers, and the
/// special layers without a name when there is one glyph.
fn layer_choices(font: &Font, targets: &Map<String, Value>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = font.masters.iter().map(|m| (format!("{BY_ID}{}", m.id), m.name.clone())).collect();
    let named: Option<Vec<&str>> = targets.get("glyphs").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).collect());
    let glyphs: Vec<&Glyph> = match &named {
        Some(names) => names.iter().filter_map(|name| font.glyph(name).map(|g| &**g)).collect(),
        None => font.glyphs.iter().map(|g| &**g).collect(),
    };
    let mut seen: HashSet<&str> = HashSet::new();
    let one = glyphs.len() == 1;
    for layer in glyphs.iter().flat_map(|g| g.layers.iter()).filter(|l| !l.is_master_layer()) {
        if out.len() >= MAX_CHOICES {
            break;
        }
        match layer.info.name.as_deref().filter(|name| !name.is_empty()) {
            Some(name) if seen.insert(name) => out.push((format!("{BY_NAME}{name}"), name.to_string())),
            Some(_) => {}
            None if one => {
                let labels = [bracket_label(font, layer), brace_label(font, layer)];
                let label = labels.into_iter().flatten().collect::<Vec<_>>().join(" ");
                out.push((format!("{BY_ID}{}", layer.id), if label.is_empty() { layer.id.clone() } else { label }));
            }
            None => {}
        }
    }
    out
}

/// The two layer fields, the second on the second layer.
fn layer_pair(session: &Session, targets: &Map<String, Value>, fields: [(&str, &str, &str); 2]) -> Result<Vec<Field>, String> {
    let choices = layer_choices(session.font().map_err(|e| e.to_string())?, targets);
    let [first, second] = choices.as_slice().first_chunk::<2>().ok_or("there is only one layer")?;
    let defaults = [first.0.clone(), second.0.clone()];
    Ok(fields.iter().zip(&defaults).map(|((key, label, doc), default)| choice(key, label, choices.clone(), default, doc)).collect())
}

/// The layer choice `values` holds under `key` as the parameter `id`, or `name` for a layer name.
fn layer_param(values: &Map<String, Value>, key: &str, id: &str, name: &str, out: &mut Map<String, Value>) {
    let Some(value) = values.get(key).and_then(Value::as_str) else { return };
    if let Some(layer) = value.strip_prefix(BY_ID) {
        out.insert(id.into(), json!(layer));
    } else if let Some(layer) = value.strip_prefix(BY_NAME) {
        out.insert(name.into(), json!(layer));
    }
}

fn copy_fields(session: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let pair = [
        ("fromLayer", "From", "The layer whose paths and components are copied"),
        ("toLayer", "To", "The layer they go to; a glyph without one of the two is skipped"),
    ];
    let mut fields = layer_pair(session, targets, pair)?;
    fields.push(Field::choice(
        "mode",
        "Mode",
        &[("replace", "Replace"), ("append", "Add")],
        "replace",
        "Replace the paths and components of the second layer, or add to them",
    ));
    Ok(fields)
}

fn copy_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    layer_param(values, "fromLayer", "from", "fromName", &mut out);
    layer_param(values, "toLayer", "to", "toName", &mut out);
    if let Some(mode) = values.get("mode") {
        out.insert("mode".into(), mode.clone());
    }
    out
}

fn compare_fields(session: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let pair = [("firstLayer", "Layer", "The first layer"), ("secondLayer", "Compare With", "The layer it is compared with")];
    let mut fields = layer_pair(session, targets, pair)?;
    fields.push(Field::number("tolerance", "Tolerance", 0.0, MAX_TOLERANCE, 0.0, "How far positions and widths may differ, in units"));
    Ok(fields)
}

fn compare_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    layer_param(values, "firstLayer", "layerA", "nameA", &mut out);
    layer_param(values, "secondLayer", "layerB", "nameB", &mut out);
    if let Some(tolerance) = values.get("tolerance") {
        out.insert("tolerance".into(), tolerance.clone());
    }
    out
}

/// After the person ran Compare Layers: Font View in front selects the glyphs that differ and scrolls to the first; the
/// status line names them.
pub fn after_run(app: &mut FontCraftApp, command: &str, result: &Value) {
    if command != COMPARE {
        return;
    }
    let names: Vec<&str> =
        result.get("different").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if names.is_empty() {
        app.status = "Compare Layers: no glyph differs".into();
        return;
    }
    if app.font_view_shows() {
        let select = CommandRef::with_params("fontView.setSelection", json!({"glyphs": names, "reveal": true}));
        app.sent.push(Msg::Shell(select));
    }
    let listed = names.iter().take(MAX_LISTED).copied().collect::<Vec<_>>().join(", ");
    let more = if names.len() > MAX_LISTED { ", …" } else { "" };
    let noun = if names.len() == 1 { "glyph differs" } else { "glyphs differ" };
    app.status = format!("Compare Layers: {} {noun}: {listed}{more}", names.len());
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
#[path = "layers_tests.rs"]
mod tests;
