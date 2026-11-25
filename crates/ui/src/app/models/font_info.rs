//! The Font Info panel's view model: the Font tab's fields, rows, axes and parameters, Other Settings and the note.
//!
//! [`rows`] turns the rows of a target (the font, masters or exports) into [`RowModel`]s with the editor each value
//! takes; the Masters and Exports tabs use it too.

use std::collections::HashMap;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::fontinfo::rows::{Row, RowKind, builtin_row, rows_of};
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_engine::fontinfo::{ParamKind, Scope, catalog, fields};
use fontcraft_engine::model::plist::{Num, Value};
use fontcraft_engine::model::{DEFAULT_LANGUAGE, Font};
use serde_json::{Value as Json, json};

use super::ProjectCx;
use crate::shell::font_info::{FontInfoTab, RowKey, RowsView, Target};

/// Most rows, axes, entries or languages a model holds; the rest are counted, not shown.
pub const MAX_SHOWN: usize = 256;
/// Most predefined axes the axis picker lists.
const MAX_PREDEFINED: usize = 64;

/// What the Font Info panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FontInfoModel {
    /// A font is open.
    pub open: bool,
    pub tab: FontInfoTab,
    pub font: FontTabModel,
    pub other: OtherModel,
    pub note: String,
    /// The font's masters as (id, name), for the parameters whose value names one.
    pub masters: Vec<(String, String)>,
    /// The Features tab; empty while another tab shows.
    pub features: super::font_features::FeaturesModel,
    /// The Exports tab; empty while another tab shows.
    pub exports: super::font_exports::ExportsModel,
    pub masters_tab: super::font_masters::MastersTabModel,
}

impl FontInfoModel {
    /// What the shown tab's rows act on and its rows in order, the added fields before the parameters; `None` on a tab
    /// without rows.
    pub fn shown_rows(&self) -> Option<(Target, Vec<&RowModel>)> {
        match self.tab {
            FontInfoTab::Font if self.open => Some((Target::font(), self.font.fields.iter().chain(&self.font.params).collect())),
            FontInfoTab::Masters if !self.masters_tab.selected.is_empty() => {
                let rows = self.masters_tab.name.iter().chain(&self.masters_tab.params);
                Some((self.masters_tab.target(), rows.collect()))
            }
            FontInfoTab::Exports if !self.exports.selected.is_empty() => {
                let target = Target { scope: Scope::Instance, indices: self.exports.selected.clone() };
                Some((target, self.exports.rows.iter().chain(&self.exports.params).collect()))
            }
            _ => None,
        }
    }

    /// Marks the rows `view` selects, on the tab of the scope they were selected in.
    fn mark_rows(&mut self, view: &RowsView) {
        let rows: Vec<&mut RowModel> = match (self.tab, view.scope) {
            (FontInfoTab::Font, Some(Scope::Font)) => self.font.fields.iter_mut().chain(&mut self.font.params).collect(),
            (FontInfoTab::Masters, Some(Scope::Master)) => self.masters_tab.name.iter_mut().chain(&mut self.masters_tab.params).collect(),
            (FontInfoTab::Exports, Some(Scope::Instance)) => self.exports.rows.iter_mut().chain(&mut self.exports.params).collect(),
            _ => return,
        };
        for row in rows {
            let key = row.key();
            row.mark = if view.rows.last() == Some(&key) {
                RowMark::Lead
            } else if view.rows.contains(&key) {
                RowMark::Selected
            } else {
                RowMark::None
            };
        }
    }
}

/// The Font tab.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FontTabModel {
    pub family_name: String,
    /// The warnings `font.info.get` gives for the General fields (`familyName`, `unitsPerEm`).
    pub warnings: Vec<FieldWarning>,
    pub units_per_em: u16,
    pub version_major: u16,
    pub version_minor: u16,
    pub date: Option<String>,
    /// The added General fields, in stored order.
    pub fields: Vec<RowModel>,
    pub params: Vec<RowModel>,
    pub axes: Vec<AxisModel>,
    /// The axes the name picker offers.
    pub predefined: Vec<PredefinedAxis>,
}

/// A Font Info warning and the field it shows under, as `font.info.get` and `font.instance.list` give it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FieldWarning {
    /// The field's key: `familyName`, `unitsPerEm`, `styleName`, `styleLinking` or `weightClass`.
    pub field: String,
    pub message: String,
}

/// Most warnings one model holds.
pub const MAX_WARNINGS: usize = 32;

/// The `warnings` list of a `font.info.get` or `font.instance.list` entry.
pub fn field_warnings(entry: &Json) -> Vec<FieldWarning> {
    let read = |w: &Json| Some(FieldWarning { field: w.get("field")?.as_str()?.to_string(), message: w.get("message")?.as_str()?.to_string() });
    entry.get("warnings").and_then(Json::as_array).map(|list| list.iter().take(MAX_WARNINGS).filter_map(read).collect()).unwrap_or_default()
}

/// One design axis and the warnings `font.axes.list` gives for it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct AxisModel {
    pub name: String,
    pub tag: String,
    pub hidden: bool,
    pub warnings: Vec<String>,
}

/// An axis `font.axes.listPredefined` offers.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PredefinedAxis {
    pub name: String,
    pub tag: String,
    pub registered: bool,
}

/// Other Settings, as `font.settings.get` answers.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct OtherModel {
    /// The number settings by key (`gridSpacing`…); `None` when the answer has none.
    pub numbers: Vec<(String, Option<f64>)>,
    /// The yes-or-no settings by key (`useCustomNaming`…).
    pub flags: Vec<(String, bool)>,
    pub file_format_version: i64,
    pub font_type: String,
}

/// A General field or a custom parameter.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowSort {
    #[default]
    Field,
    Param,
}

/// One row of a target: a General field or a custom parameter, with the editor of its value.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct RowModel {
    pub sort: RowSort,
    /// The property key or the parameter name.
    pub key: String,
    /// Among the rows of this key, from 0.
    pub occurrence: usize,
    pub label: String,
    /// A parameter's checkbox; true for a field.
    pub enabled: bool,
    /// Built-in fields stay.
    pub removable: bool,
    /// The targets that hold the row hold different values.
    pub mixed: bool,
    /// The targets that hold the row, when others of the selection lack it; empty when all hold it.
    pub holders: Vec<usize>,
    pub mark: RowMark,
    pub editor: Editor,
    /// What is wrong with the value, one sentence each, shown under the row.
    pub warnings: Vec<String>,
}

impl RowModel {
    /// The row as `font.info.rows.select` and the row commands name it.
    pub fn key(&self) -> RowKey {
        RowKey { param: self.sort == RowSort::Param, key: self.key.clone(), index: self.occurrence }
    }
}

/// Whether a row is selected.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowMark {
    #[default]
    None,
    Selected,
    /// Selected last: Shift-click selects from here.
    Lead,
}

/// How a row edits its value.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub enum Editor {
    Bool(bool),
    Number(Option<f64>),
    Text(String),
    /// A list, one item per line.
    Lines(String),
    /// One of the parameter's values.
    Choice {
        choices: Vec<String>,
        value: String,
    },
    /// Any of the parameter's values.
    Choices {
        choices: Vec<String>,
        on: Vec<String>,
    },
    /// A master, by id; the value as stored when it names none.
    Master(String),
    Dict(Vec<DictEntry>),
    /// One text per language, as (language, text).
    Localized(Vec<(String, String)>),
    /// A localizable field: one text per language, as (language, text).
    Languages(Vec<(String, String)>),
    /// A value edited in a dialog of its own: a summary and an Edit button.
    Dialog {
        summary: String,
        dialog: RowDialog,
    },
    /// A value no editor shows.
    Summary(String),
    /// The `name; code` value of a parameter that writes feature code (Add or Replace Class, Prefix, Feature), and the
    /// first problem the instance compile found in it as (line in the code, message).
    Code {
        text: String,
        problem: Option<(u64, String)>,
    },
}

/// The parameters whose value is a name, a semicolon and feature code.
pub const FEATURE_CODE_PARAMS: [&str; 6] = ["Add Class", "Add Prefix", "Add Feature", "Replace Class", "Replace Prefix", "Replace Feature"];

impl Default for Editor {
    fn default() -> Self {
        Editor::Text(String::new())
    }
}

/// One entry of a dictionary value.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct DictEntry {
    pub key: String,
    /// The text of a text or number entry; `None` for an entry no field edits.
    pub text: Option<String>,
    /// The entry is a number: an edit that reads as one stays a number.
    pub number: bool,
    /// The entry's value as JSON, kept when another entry changes.
    pub json: String,
}

/// The dialog a row's Edit button opens.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowDialog {
    /// fsType, codePageRanges, unicodeRanges.
    Os2Bits,
    Panose,
    TtfStems,
    TtfZones,
    AxisLocation,
    VirtualMaster,
    AxisMappings,
    ColorPalettes,
    MasterColor,
}

/// The parameters whose value names one master, edited with a menu of the font's masters.
pub const NAMES_A_MASTER: &[&str] = &["Variable Font Origin", "Variation Font Origin", "Get Hints From Master", "Link Metrics With Master"];

pub fn project(model: &mut Tracked<FontInfoModel>, cx: &mut ProjectCx<'_>) {
    let tab = cx.view.font_info.tab;
    let predefined = predefined_axes(cx);
    let warnings = axis_warnings(cx);
    let settings = cx.session.query("font.settings.get", &json!({})).unwrap_or(Json::Null);
    let info_warnings = cx.session.query("font.info.get", &json!({"keys": ["warnings"]})).map(|reply| field_warnings(&reply)).unwrap_or_default();
    let features = if tab == FontInfoTab::Features { super::font_features::project(cx) } else { Default::default() };
    let exports = if tab == FontInfoTab::Exports { super::font_exports::project(cx) } else { Default::default() };
    let Ok(font) = cx.session.font() else {
        super::set(model, FontInfoModel { tab, ..FontInfoModel::default() });
        return;
    };
    let target = Target::font();
    let axes = font
        .axes
        .iter()
        .take(MAX_SHOWN)
        .enumerate()
        .map(|(at, a)| AxisModel {
            name: a.name.clone(),
            tag: a.tag.clone(),
            hidden: a.hidden,
            warnings: warnings.get(at).cloned().unwrap_or_default(),
        })
        .collect();
    let font_tab = FontTabModel {
        family_name: font.family_name.clone(),
        warnings: info_warnings,
        units_per_em: font.units_per_em,
        version_major: font.version_major,
        version_minor: font.version_minor,
        date: font.info.date.clone(),
        fields: rows(font, &target, RowKind::Field),
        params: rows(font, &target, RowKind::Param),
        axes,
        predefined,
    };
    let mut value = FontInfoModel {
        open: true,
        tab,
        font: font_tab,
        other: other(&settings),
        note: font.note.clone(),
        masters: font.masters.iter().take(MAX_SHOWN).map(|m| (m.id.clone(), m.name.clone())).collect(),
        features,
        exports,
        masters_tab: super::font_masters::project(font, &cx.view.font_info.masters),
    };
    value.mark_rows(&cx.view.font_info.rows);
    super::set(model, value);
}

/// The keys of the number and yes-or-no settings the Other Settings tab shows.
pub const SETTING_NUMBERS: [&str; 4] = ["gridSpacing", "gridSubdivision", "keyboardIncrementShift", "keyboardIncrementCmd"];
pub const SETTING_FLAGS: [&str; 3] = ["useCustomNaming", "disableAutomaticAlignment", "keepAlternatesNextToBase"];

fn other(settings: &Json) -> OtherModel {
    OtherModel {
        numbers: SETTING_NUMBERS.iter().map(|k| ((*k).to_string(), settings.get(*k).and_then(Json::as_f64).filter(|n| n.is_finite()))).collect(),
        flags: SETTING_FLAGS.iter().map(|k| ((*k).to_string(), settings.get(*k).and_then(Json::as_bool).unwrap_or(false))).collect(),
        file_format_version: settings.get("fileFormatVersion").and_then(Json::as_i64).unwrap_or(3),
        font_type: settings.get("fontType").and_then(Json::as_str).unwrap_or("default").to_string(),
    }
}

fn predefined_axes(cx: &mut ProjectCx<'_>) -> Vec<PredefinedAxis> {
    let reply = cx.session.query("font.axes.listPredefined", &json!({})).unwrap_or(Json::Null);
    let read = |a: &Json| {
        Some(PredefinedAxis {
            name: a.get("name")?.as_str()?.to_string(),
            tag: a.get("tag")?.as_str()?.to_string(),
            registered: a.get("registered").and_then(Json::as_bool).unwrap_or(false),
        })
    };
    reply.get("axes").and_then(Json::as_array).map(|axes| axes.iter().take(MAX_PREDEFINED).filter_map(read).collect()).unwrap_or_default()
}

/// The warnings of each axis, in axis order.
fn axis_warnings(cx: &mut ProjectCx<'_>) -> Vec<Vec<String>> {
    let reply = cx.session.query("font.axes.list", &json!({})).unwrap_or(Json::Null);
    let read = |a: &Json| -> Vec<String> {
        a.get("warnings").and_then(Json::as_array).map(|w| w.iter().filter_map(Json::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    reply.get("axes").and_then(Json::as_array).map(|axes| axes.iter().take(MAX_SHOWN).map(read).collect()).unwrap_or_default()
}

/// The rows of `kind` any of `target`'s masters or exports holds, in stored order: the first one's, then those only a
/// later one holds. A row is mixed where the targets that hold it hold different values.
pub fn rows(font: &Font, target: &Target, kind: RowKind) -> Vec<RowModel> {
    let indices: Vec<usize> = if target.below_font() && !target.indices.is_empty() { target.indices.clone() } else { vec![target.first()] };
    let masters: Vec<(&str, &str)> = font.masters.iter().map(|m| (m.id.as_str(), m.name.as_str())).collect();
    let mut place: HashMap<(String, usize), usize> = HashMap::new();
    // The row of the first target that holds it, its occurrence, its holders, and whether they disagree.
    let mut found: Vec<(Row, usize, Vec<usize>, bool)> = Vec::new();
    for &at in &indices {
        let mut seen: HashMap<String, usize> = HashMap::new();
        for row in rows_of(font, target.scope, at).into_iter().filter(|r| r.kind == kind) {
            let count = seen.entry(row.key.clone()).or_insert(0);
            let occurrence = *count;
            *count = count.saturating_add(1);
            let key = (row.key.clone(), occurrence);
            if let Some(held) = place.get(&key).copied() {
                if let Some((first, _, holders, mixed)) = found.get_mut(held) {
                    holders.push(at);
                    *mixed |= first.value != row.value;
                }
            } else if found.len() < MAX_SHOWN {
                place.insert(key, found.len());
                found.push((row, occurrence, vec![at], false));
            }
        }
    }
    let model = |(row, occurrence, holders, mixed): (Row, usize, Vec<usize>, bool)| {
        let mut model = row_model(&row, target.scope, &masters);
        model.occurrence = occurrence;
        model.mixed = mixed;
        if holders.len() < indices.len() {
            model.holders = holders;
        }
        model
    };
    found.into_iter().map(model).collect()
}

/// One built-in field of `target` (a master's `name`, an instance's `weightClass`…); `None` when it does not set it.
pub fn builtin(font: &Font, target: &Target, key: &str) -> Option<RowModel> {
    let row = builtin_row(font, target.scope, target.first(), key)?;
    let mixed = target.indices.iter().skip(1).any(|at| builtin_row(font, target.scope, *at, key).is_none_or(|r| r.value != row.value));
    let mut model = row_model(&row, target.scope, &[]);
    model.mixed = mixed;
    Some(model)
}

fn row_model(row: &Row, scope: Scope, masters: &[(&str, &str)]) -> RowModel {
    match row.kind {
        RowKind::Field => {
            let field = fields::field(scope, &row.key);
            let localizable = field.map_or(matches!(row.value, Value::Dict(_)), |f| f.localizable);
            let built_in = field.is_some_and(|f| f.built_in);
            let editor = if localizable {
                Editor::Languages(languages_of(&row.value))
            } else if built_in && matches!(row.value, Value::Number(_)) {
                Editor::Number(fv::number(&row.value))
            } else {
                Editor::Text(plain_text(&row.value).unwrap_or_default())
            };
            RowModel {
                sort: RowSort::Field,
                key: row.key.clone(),
                occurrence: 0,
                label: field.map_or_else(|| row.key.clone(), |f| f.label.to_string()),
                enabled: true,
                removable: !built_in,
                mixed: false,
                holders: Vec::new(),
                mark: RowMark::None,
                editor,
                warnings: Vec::new(),
            }
        }
        RowKind::Param => RowModel {
            sort: RowSort::Param,
            key: row.key.clone(),
            occurrence: 0,
            label: row.key.clone(),
            enabled: row.enabled,
            removable: true,
            mixed: false,
            holders: Vec::new(),
            mark: RowMark::None,
            editor: param_editor(&row.key, &row.value, scope, masters),
            warnings: Vec::new(),
        },
    }
}

/// The dialog that edits the parameter `name` at `scope`, if its value has one.
pub fn row_dialog(name: &str, scope: Scope) -> Option<RowDialog> {
    Some(match (name, scope) {
        ("fsType" | "codePageRanges" | "unicodeRanges", _) => RowDialog::Os2Bits,
        ("panose", _) => RowDialog::Panose,
        ("TTFStems", Scope::Master) => RowDialog::TtfStems,
        ("TTFZones", Scope::Master) => RowDialog::TtfZones,
        ("Axis Location", Scope::Master | Scope::Instance) => RowDialog::AxisLocation,
        ("Virtual Master", Scope::Font) => RowDialog::VirtualMaster,
        ("Axis Mappings", Scope::Font | Scope::Instance) => RowDialog::AxisMappings,
        ("Color Palettes", Scope::Font) => RowDialog::ColorPalettes,
        ("Master Color" | "Master Color Dark", Scope::Master) => RowDialog::MasterColor,
        _ => return None,
    })
}

fn param_editor(name: &str, value: &Value, scope: Scope, masters: &[(&str, &str)]) -> Editor {
    if let Some(dialog) = row_dialog(name, scope) {
        return Editor::Dialog { summary: summary(value), dialog };
    }
    if FEATURE_CODE_PARAMS.contains(&name)
        && let Some(text) = plain_text(value)
    {
        return Editor::Code { text, problem: None };
    }
    if NAMES_A_MASTER.contains(&name) {
        let text = plain_text(value).unwrap_or_default();
        let id = masters.iter().find(|(id, n)| *id == text || *n == text).map_or(text, |(id, _)| (*id).to_string());
        return Editor::Master(id);
    }
    let spec = catalog::lookup(name);
    let kind = spec.map_or_else(|| kind_of(value), |s| s.kind);
    let choices: Vec<String> = spec.map(|s| s.choices.iter().map(|c| (*c).to_string()).collect()).unwrap_or_default();
    match kind {
        ParamKind::Number | ParamKind::String if !choices.is_empty() => Editor::Choice { value: plain_text(value).unwrap_or_default(), choices },
        ParamKind::List if !choices.is_empty() => match fv::lines(value) {
            Some(on) => {
                let on = on
                    .into_iter()
                    .map(|item| choices.iter().find(|c| c.eq_ignore_ascii_case(item.trim())).cloned().unwrap_or_else(|| item.trim().to_string()))
                    .collect();
                Editor::Choices { choices, on }
            }
            None => list_choices(value, choices),
        },
        ParamKind::Bool => Editor::Bool(fv::flag(value).unwrap_or(false)),
        ParamKind::Number => Editor::Number(fv::number(value)),
        ParamKind::String => plain_text(value).map_or_else(|| Editor::Summary(summary(value)), Editor::Text),
        ParamKind::List => fv::lines(value).map_or_else(|| Editor::Summary(summary(value)), |lines| Editor::Lines(lines.join("\n"))),
        ParamKind::Dict => dict(value),
        ParamKind::Localized => fv::localized(value).map_or_else(
            || Editor::Summary(summary(value)),
            |mut l| {
                l.truncate(MAX_SHOWN);
                Editor::Localized(l)
            },
        ),
    }
}

/// A list of choices stored as numbers (`[1, 2]`): each item's text.
fn list_choices(value: &Value, choices: Vec<String>) -> Editor {
    let Value::Array(a) = value else { return Editor::Summary(summary(value)) };
    let on = a.items.iter().take(MAX_SHOWN).filter_map(plain_text).collect();
    Editor::Choices { choices, on }
}

fn dict(value: &Value) -> Editor {
    let Value::Dict(d) = value else { return Editor::Summary(summary(value)) };
    Editor::Dict(
        d.entries
            .iter()
            .take(MAX_SHOWN)
            .map(|(key, entry)| DictEntry {
                key: key.as_str().to_string(),
                text: plain_text(entry),
                number: matches!(entry, Value::Number(_)),
                json: fv::to_json(entry).to_string(),
            })
            .collect(),
    )
}

/// The kind of editor for a value of a parameter the catalogue does not know.
fn kind_of(value: &Value) -> ParamKind {
    match value {
        Value::Bool(_) => ParamKind::Bool,
        Value::Number(_) => ParamKind::Number,
        Value::Array(_) => ParamKind::List,
        Value::Dict(_) => ParamKind::Dict,
        _ => ParamKind::String,
    }
}

/// A text, a number or a flag as one line of text; `None` for other values.
pub fn plain_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.as_str().to_string()),
        Value::Number(n) => Some(match n.value() {
            Num::Int(i) => i.to_string(),
            Num::Float(f) => f.to_string(),
        }),
        Value::Bool(b) => Some(if *b { "1".into() } else { "0".into() }),
        _ => None,
    }
}

/// A short summary of a value: its text, or how many items or entries it holds.
pub fn summary(value: &Value) -> String {
    match value {
        Value::Array(a) => a.items.iter().take(MAX_SHOWN).map(|v| plain_text(v).unwrap_or_else(|| "…".into())).collect::<Vec<_>>().join(", "),
        Value::Dict(d) => d.entries.iter().take(MAX_SHOWN).map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", "),
        Value::Data(_) => String::new(),
        other => plain_text(other).unwrap_or_default(),
    }
}

/// The languages of a localizable field's value: a dictionary from language to text, or one text for the default
/// language.
pub fn languages_of(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Dict(d) => d.entries.iter().take(MAX_SHOWN).map(|(k, v)| (k.as_str().to_string(), plain_text(v).unwrap_or_default())).collect(),
        other => vec![(DEFAULT_LANGUAGE.to_string(), plain_text(other).unwrap_or_default())],
    }
}
