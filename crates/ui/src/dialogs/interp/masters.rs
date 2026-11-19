//! The dialogs of Font Info's Masters tab. Each runs its command once, on OK; Cancel changes nothing.
//!
//! | Dialog | Command | Opens with |
//! |---|---|---|
//! | `fontInfo.addOtherFont` | `font.master.addFromOtherFont`: copies the ticked masters of the open documents | `{}` |
//! | `fontInfo.masterIconPicker` | `font.master.setIcon`: a built-in icon, or a glyph of the font when one is typed | `{masters: [index]}` |
//! | `fontInfo.addMetric` | `font.metrics.add`: category, name, scope | `{}` |
//! | `fontInfo.metricEditor` | `font.metrics.set`: the metric's category, name and scope | `{metric: index}` |
//! | `fontInfo.stemScope` | `font.stems.set`: the stem's glyph scope | `{stem: index}` |
//!
//! A scope is a glyph predicate, as smart filters write them (`script == "greek"`); an emptied name or scope removes
//! it. Add Other Font's checkbox words are `<document position>/<master index>`: `ui.dialog.set {field: "sources",
//! value: ["1/0"]}` ticks the first master of the second document.

use fontcraft_engine::Session;
use fontcraft_engine::cmd::masters::MASTER_ICONS;
use fontcraft_engine::model::{MAX_NAME_LEN, metric};
use serde_json::{Map, Value, json};

use super::super::fields::{Field, Kind, MAX_CHOICES};
use super::super::spec::{DialogSpec, Fields, Shaped};
use super::no_link;
use crate::app::models::font_masters::METRIC_CATEGORIES;

/// Longest scope the scope fields take, in characters.
const MAX_SCOPE: usize = 1024;

pub const ADD_OTHER_FONT: DialogSpec = DialogSpec::new(
    "fontInfo.addOtherFont",
    "Add Other Font",
    "font.master.addFromOtherFont",
    Fields::Shaped(Shaped { fields: other_font_fields, given: other_font_given, params: other_font_params, link: no_link }),
)
.run_on_ok();

pub const ICON_PICKER: DialogSpec = DialogSpec::new(
    "fontInfo.masterIconPicker",
    "Master Icon",
    "font.master.setIcon",
    Fields::Shaped(Shaped { fields: icon_fields, given: icon_given, params: icon_params, link: no_link }),
)
.run_on_ok();

pub const ADD_METRIC: DialogSpec = DialogSpec::new(
    "fontInfo.addMetric",
    "Add Metric",
    "font.metrics.add",
    Fields::Shaped(Shaped { fields: metric_fields, given: add_metric_given, params: add_metric_params, link: no_link }),
)
.run_on_ok()
.buttons("Add", "");

pub const METRIC_EDITOR: DialogSpec = DialogSpec::new(
    "fontInfo.metricEditor",
    "Metric",
    "font.metrics.set",
    Fields::Shaped(Shaped { fields: metric_fields, given: metric_given, params: metric_params, link: no_link }),
)
.run_on_ok();

pub const STEM_SCOPE: DialogSpec = DialogSpec::new(
    "fontInfo.stemScope",
    "Stem Scope",
    "font.stems.set",
    Fields::Shaped(Shaped { fields: scope_fields, given: stem_given, params: scope_params, link: no_link }),
)
.run_on_ok();

/// The masters of the open documents as `(word, label)` pairs, at most [`MAX_CHOICES`].
fn other_masters(session: &Session) -> Vec<(String, String)> {
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

fn other_font_fields(session: &Session) -> Vec<Field> {
    let (choices, labels): (Vec<String>, Vec<String>) = other_masters(session).into_iter().unzip();
    vec![Field {
        key: "sources".into(),
        label: "Masters".into(),
        kind: Kind::Multi { choices, labels },
        default: json!([]),
        doc: "The masters to copy into this font, each added at the end of its master list".into(),
        group: None,
    }]
}

fn other_font_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    session.font().map_err(|e| e.to_string())?;
    if other_masters(session).is_empty() {
        return Err("Add Other Font: no open font has a master".into());
    }
    Ok(given.clone())
}

/// `{sources: [{font, master}]}` from the ticked words.
fn other_font_params(values: &Map<String, Value>) -> Map<String, Value> {
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

fn icon_fields(_: &Session) -> Vec<Field> {
    let names: Vec<String> = MASTER_ICONS.iter().map(|n| (*n).to_string()).collect();
    let labels = names.iter().map(|n| n.replace('_', " ")).collect();
    vec![
        Field {
            key: "icon".into(),
            label: "Icon".into(),
            kind: Kind::Choice { choices: names, labels },
            default: json!(MASTER_ICONS.get(1).copied().unwrap_or_default()),
            doc: "One of the built-in icons: a lowercase n at five weights and five widths".into(),
            group: None,
        },
        Field::text("glyph", "Glyph", MAX_NAME_LEN, "A glyph of the font to use as the icon instead; empty uses the built-in icon"),
    ]
}

fn icon_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    let masters = given.get("masters").and_then(Value::as_array).filter(|m| !m.is_empty()).ok_or("Master Icon: name the masters with `masters`")?;
    let mut out = given.clone();
    if !out.contains_key("icon") && !out.contains_key("glyph") {
        let first = masters.first().and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).and_then(|i| font.masters.get(i));
        match first.and_then(|m| m.icon.clone()) {
            Some(name) if MASTER_ICONS.contains(&name.as_str()) => {
                out.insert("icon".into(), json!(name));
            }
            Some(glyph) => {
                out.insert("glyph".into(), json!(glyph));
            }
            None => {}
        }
    }
    Ok(out)
}

/// The glyph when one is typed, else the built-in icon.
fn icon_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    match values.get("glyph").and_then(Value::as_str).map(str::trim).filter(|g| !g.is_empty()) {
        Some(glyph) => m.insert("glyph".into(), json!(glyph)),
        None => m.insert("icon".into(), values.get("icon").cloned().unwrap_or(Value::Null)),
    };
    m
}

fn metric_fields(_: &Session) -> Vec<Field> {
    vec![
        Field::choice("category", "Category", &METRIC_CATEGORIES, metric::ASCENDER, "The metric's kind; a metric known only by its name has none"),
        Field::text("name", "Name", MAX_NAME_LEN, "A name that tells the metric apart from others of its kind; empty for none"),
        Field::text("scope", "Scope", MAX_SCOPE, "The glyphs the metric applies to, as a glyph predicate (script == \"greek\"); empty for all"),
    ]
}

/// The new metric's category: the first the font has no plain metric of.
fn add_metric_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    let mut out = given.clone();
    if !out.contains_key("category") {
        let plain = |kind: &str| font.metrics.iter().any(|m| m.kind == kind && m.name.is_none() && m.filter.is_none());
        let category = METRIC_CATEGORIES.iter().map(|(k, _)| *k).find(|k| !k.is_empty() && !plain(k)).unwrap_or(metric::ASCENDER);
        out.insert("category".into(), json!(category));
    }
    Ok(out)
}

fn index_of(given: &Map<String, Value>, key: &str, count: usize, title: &str) -> Result<usize, String> {
    let at = given
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .ok_or_else(|| format!("{title}: name the {key} by its index with `{key}`"))?;
    if at >= count {
        return Err(format!("{title}: the font has {count} {key}s; there is none with index {at}"));
    }
    Ok(at)
}

fn metric_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    let at = index_of(given, "metric", font.metrics.len(), "Metric")?;
    let mut out = given.clone();
    if let Some(def) = font.metrics.get(at) {
        // A stored category the menu lacks keeps its spelling only when the menu has it; else the field starts empty.
        let category = METRIC_CATEGORIES.iter().find(|(k, _)| k.eq_ignore_ascii_case(&def.kind)).map_or("", |(k, _)| k);
        out.entry("category").or_insert_with(|| json!(category));
        out.entry("name").or_insert_with(|| json!(def.name.clone().unwrap_or_default()));
        out.entry("scope").or_insert_with(|| json!(def.filter.clone().unwrap_or_default()));
    }
    Ok(out)
}

/// A new metric's parameters: an empty name or scope is left out.
fn add_metric_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = metric_params(values);
    m.retain(|k, v| k == "category" || v.as_str().is_some_and(|t| !t.is_empty()));
    m
}

/// The fields as the command's parameters, trimmed.
fn metric_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    for key in ["category", "name", "scope"] {
        let text = values.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default();
        m.insert(key.into(), json!(text));
    }
    m
}

fn scope_fields(_: &Session) -> Vec<Field> {
    vec![Field::text("scope", "Scope", MAX_SCOPE, "The glyphs the stem applies to, as a glyph predicate (script == \"greek\"); empty for all")]
}

fn stem_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    let at = index_of(given, "stem", font.stems.len(), "Stem Scope")?;
    let mut out = given.clone();
    if let Some(def) = font.stems.get(at) {
        out.entry("scope").or_insert_with(|| json!(def.filter.clone().unwrap_or_default()));
    }
    Ok(out)
}

fn scope_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("scope".into(), json!(values.get("scope").and_then(Value::as_str).map(str::trim).unwrap_or_default()));
    m
}
