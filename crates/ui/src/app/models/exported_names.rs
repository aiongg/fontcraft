//! What the export of the first selected entry writes, read from the last `font.instance.names` reply
//! (`crate::shell::export_names`) into the Exports tab's model: the As Exported section, the marks of entries that take
//! the same name, and the tokens that name nothing under the rows that hold them.

use craft::ui::contract::PlainData;
use fontcraft_engine::model::Font;
use serde_json::Value;

use super::ProjectCx;
use super::font_exports::ExportsModel;
use super::font_info::{FieldWarning, MAX_SHOWN, MAX_WARNINGS};

/// Most characters of one name shown.
const MAX_TEXT_SHOWN: usize = 300;
/// Most entries a conflict or a style group names.
const MAX_NAMED: usize = 8;

/// What the export of one entry writes.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ExportedModel {
    /// Why the entry cannot be exported; the other fields are empty then.
    pub error: Option<String>,
    /// The file name without extension, in its Export Folder.
    pub file: String,
    /// The name records, as (ID, language tag or platform, text).
    pub names: Vec<ExportedName>,
    pub weight_class: Option<u64>,
    pub width_class: Option<u64>,
    /// The `fsSelection` flags by name.
    pub selection: Vec<String>,
    /// The style-linked family, the slot the entry takes, and the other exports linked to it as (slot, entry name).
    pub group: Option<StyleGroup>,
    /// A variable font's named instances: name, PostScript name, coordinates.
    pub named: Vec<NamedInstance>,
    /// A variable font's axis values, as (axis name, values in words).
    pub stat: Vec<(String, String)>,
    /// What other active entries take too.
    pub conflicts: Vec<ConflictLine>,
    /// Texts of the font or a master whose tokens name nothing, one sentence each.
    pub tokens: Vec<String>,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ExportedName {
    pub id: u64,
    /// Empty for the English Windows record.
    pub place: String,
    pub text: String,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct StyleGroup {
    pub family: String,
    pub style: String,
    pub others: Vec<(String, String)>,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct NamedInstance {
    pub name: String,
    pub postscript: String,
    pub location: String,
}

/// A value the entry shares with others.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ConflictLine {
    /// `fileName`, `postscriptName` or `styleLinking`.
    pub field: String,
    pub value: String,
    /// The entries that share it, by name.
    pub with: String,
}

/// Puts what the view state's reply says about the shown font into `model`. A reply of another document, or of another
/// number of exports, says nothing.
pub fn place(model: &mut ExportsModel, cx: &ProjectCx<'_>) {
    let (Some(known), Some(doc)) = (&cx.view.font_info.exports.names, cx.session.active()) else { return };
    if known.doc != doc.id {
        return;
    }
    let font: &Font = &doc.font;
    let entries = list(&known.reply, "instances");
    let left_out = known.reply.get("leftOut").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    if entries.len().saturating_add(left_out) != font.instances.len() {
        return;
    }
    model.conflicts = entries.iter().filter(|e| !list(e, "conflicts").is_empty()).filter_map(|e| index(e.get("index")?)).collect();
    let Some(first) = model.selected.first().copied() else { return };
    let Some(entry) = entries.iter().find(|e| e.get("index").and_then(index) == Some(first)) else { return };
    let mut shown = exported(font, entry, entries);
    for problem in list(entry, "tokens") {
        token_problem(model, &mut shown, font, problem);
    }
    model.exported = Some(shown);
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn index(v: &Value) -> Option<usize> {
    v.as_u64().and_then(|n| usize::try_from(n).ok())
}

fn entry_name(font: &Font, at: usize) -> String {
    match font.instances.get(at).map(|i| i.name.trim()) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => format!("#{at}"),
    }
}

fn cut(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT_SHOWN { s.to_string() } else { s.chars().take(MAX_TEXT_SHOWN).chain(['…']).collect() }
}

fn exported(font: &Font, entry: &Value, entries: &[Value]) -> ExportedModel {
    if let Some(error) = entry.get("error").and_then(Value::as_str) {
        return ExportedModel { error: Some(error.to_string()), ..ExportedModel::default() };
    }
    let names = list(entry, "names").iter().take(MAX_SHOWN).map(name).collect();
    let folder = text(entry, "folder");
    let file = text(entry, "fileName");
    let named = list(entry, "namedInstances").iter().take(MAX_SHOWN).map(|i| named_instance(i, list(entry, "axes"))).collect();
    let stat = entry.get("stat").map(|s| list(s, "axes").iter().take(MAX_SHOWN).map(stat_axis).collect()).unwrap_or_default();
    ExportedModel {
        error: None,
        file: if folder.is_empty() { file } else { format!("{folder}/{file}") },
        names,
        weight_class: entry.get("weightClass").and_then(Value::as_u64),
        width_class: entry.get("widthClass").and_then(Value::as_u64),
        selection: list(entry, "fsSelection").iter().filter_map(Value::as_str).map(str::to_string).collect(),
        group: entry.get("styleGroup").map(|g| style_group(font, entry, g, entries)),
        named,
        stat,
        conflicts: list(entry, "conflicts").iter().map(|c| conflict(font, c)).collect(),
        tokens: Vec::new(),
    }
}

fn name(r: &Value) -> ExportedName {
    let number = |key: &str| r.get(key).and_then(Value::as_u64).unwrap_or(0);
    let place = match (number("platform"), r.get("tag").and_then(Value::as_str)) {
        (3, Some(tag)) => tag.to_string(),
        (3, None) if number("language") == 0x409 => String::new(),
        (3, None) => format!("0x{:04X}", number("language")),
        (1, _) => "Mac".to_string(),
        (platform, _) => format!("{platform}/{}", number("encoding")),
    };
    ExportedName { id: number("id"), place, text: cut(&text(r, "text")) }
}

fn style_group(font: &Font, entry: &Value, group: &Value, entries: &[Value]) -> StyleGroup {
    let own = entry.get("index").and_then(index);
    let others = list(group, "entries")
        .iter()
        .filter_map(index)
        .filter(|at| Some(*at) != own)
        .take(MAX_NAMED)
        .map(|at| {
            let slot = entries.iter().find(|e| e.get("index").and_then(index) == Some(at)).and_then(|e| e.get("styleGroup"));
            (slot.map(|s| text(s, "style")).unwrap_or_default(), entry_name(font, at))
        })
        .collect();
    StyleGroup { family: text(group, "family"), style: text(group, "style"), others }
}

fn named_instance(i: &Value, axes: &[Value]) -> NamedInstance {
    let place = |(at, v): (usize, &Value)| {
        let n = v.as_f64().map(crate::panels::widgets::format_number).unwrap_or_default();
        format!("{} {n}", axes.get(at).and_then(Value::as_str).unwrap_or("?"))
    };
    let location: Vec<String> = list(i, "coordinates").iter().take(MAX_NAMED).enumerate().map(place).collect();
    NamedInstance { name: text(i, "name"), postscript: text(i, "postscriptName"), location: location.join(", ") }
}

fn stat_axis(axis: &Value) -> (String, String) {
    let number = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).map(crate::panels::widgets::format_number);
    let value = |v: &Value| {
        let mut line = format!("{} {}", text(v, "name"), number(v, "value").unwrap_or_default());
        if let (Some(min), Some(max)) = (number(v, "min"), number(v, "max")) {
            line.push_str(&format!(" ({min}–{max})"));
        }
        if let Some(linked) = number(v, "linkedValue") {
            line.push_str(&format!(" → {linked}"));
        }
        // An elidable value's name is left out of the composed style name: shown in brackets, as it is written nowhere.
        if v.get("elidable").and_then(Value::as_bool) == Some(true) { format!("[{line}]") } else { line }
    };
    let values: Vec<String> = list(axis, "values").iter().take(MAX_SHOWN).map(value).collect();
    (text(axis, "name"), values.join(", "))
}

fn conflict(font: &Font, c: &Value) -> ConflictLine {
    let mut with: Vec<String> = list(c, "entries").iter().filter_map(index).take(MAX_NAMED).map(|at| entry_name(font, at)).collect();
    if list(c, "entries").len() > MAX_NAMED {
        with.push("…".to_string());
    }
    ConflictLine { field: text(c, "field"), value: text(c, "value"), with: with.join(", ") }
}

/// Puts one entry of `tokens` under the row that holds the text, or with the section's when no row of the tab does.
fn token_problem(model: &mut ExportsModel, shown: &mut ExportedModel, font: &Font, problem: &Value) {
    let tokens: Vec<String> = list(problem, "tokens").iter().filter_map(Value::as_str).take(MAX_NAMED).map(|t| format!("{{{{{{{t}}}}}}}")).collect();
    let message = format!("nothing is named {}; it is written as it stands", tokens.join(", "));
    let (key, param) = (text(problem, "key"), problem.get("param").and_then(Value::as_bool).unwrap_or(false));
    let occurrence = problem.get("index").and_then(index).unwrap_or(0);
    match problem.get("scope").and_then(Value::as_str) {
        Some("instance") => {
            let rows = if param { &mut model.params } else { &mut model.rows };
            if let Some(row) = rows.iter_mut().find(|r| r.key == key && r.occurrence == occurrence) {
                row.warnings.push(message);
                return;
            }
            if let Some(fields) = model.fields.as_mut().filter(|f| f.field_warnings.len() < MAX_WARNINGS) {
                fields.field_warnings.push(FieldWarning { field: key, message });
            }
        }
        scope => {
            let place = match (scope, problem.get("master").and_then(Value::as_str)) {
                (Some("master"), Some(id)) => font.masters.iter().find(|m| m.id == id).map_or_else(|| id.to_string(), |m| m.name.clone()),
                _ => "Font".to_string(),
            };
            shown.tokens.push(format!("{place} › {key}: {message}"));
        }
    }
}
