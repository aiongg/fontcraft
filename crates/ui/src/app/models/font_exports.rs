//! The Exports tab's view model: the export list, the selection, and the fields and rows of the selected entries.
//!
//! It is projected only while the Exports tab shows; `font.instance.placement` (the extrapolation warnings) runs then.

use craft::ui::contract::PlainData;
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::RowKind;
use fontcraft_engine::model::axes::design_value;
use fontcraft_engine::model::{Font, Instance, StyleClass};
use serde_json::{Value, json};

use super::ProjectCx;
use super::exported_names::{self, ExportedModel};
use super::font_info::{Editor, FieldWarning, MAX_SHOWN, MAX_WARNINGS, RowModel, field_warnings, rows};
use super::instance_preview::InstancePreview;
use crate::shell::font_info::Target;
use crate::shell::font_info_tabs::ParamProblem;

/// Most axes the Axes Coordinates rows list.
const MAX_AXES_SHOWN: usize = 64;
/// Most extrapolation warnings shown.
pub const MAX_WARNINGS_SHOWN: usize = 8;

/// The plus menu's commands. Add Instance is always there; the others are enabled once the engine has their command.
pub const PLUS_MENU: [&str; 4] =
    ["font.instance.add", "font.instance.addForEachMaster", "font.instance.toMaster", "font.instance.addVariableFontSetting"];

/// The standard weight and width classes the class menus list.
pub const WEIGHT_CLASSES: [u16; 9] = [100, 200, 300, 400, 500, 600, 700, 800, 900];
pub const WIDTH_CLASSES: [u16; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];
/// Class names older sources store instead of numbers: letters and digits only, lower case.
const WEIGHT_NAMES: &[(&str, u16)] = &[
    ("thin", 100),
    ("hairline", 100),
    ("extralight", 200),
    ("ultralight", 200),
    ("light", 300),
    ("normal", 400),
    ("regular", 400),
    ("medium", 500),
    ("semibold", 600),
    ("demibold", 600),
    ("bold", 700),
    ("extrabold", 800),
    ("ultrabold", 800),
    ("black", 900),
    ("heavy", 900),
];
const WIDTH_NAMES: &[(&str, u16)] = &[
    ("ultracondensed", 1),
    ("extracondensed", 2),
    ("condensed", 3),
    ("semicondensed", 4),
    ("medium", 5),
    ("mediumnormal", 5),
    ("normal", 5),
    ("semiexpanded", 6),
    ("expanded", 7),
    ("extraexpanded", 8),
    ("ultraexpanded", 9),
];
/// The class an instance has when its source states none (as `font.instance.list` reports it).
const DEFAULT_WEIGHT_CLASS: f64 = 400.0;
const DEFAULT_WIDTH_CLASS: f64 = 5.0;

/// What the Exports tab shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ExportsModel {
    pub entries: Vec<ExportEntry>,
    /// The selected entries, in list order; the first entry when none is selected.
    pub selected: Vec<usize>,
    /// Which plus menu commands the engine has, in [`PLUS_MENU`] order.
    pub plus: Vec<bool>,
    /// The selected entries' fields; `None` without entries.
    pub fields: Option<ExportFields>,
    /// The added General fields and the custom parameters of the selected entries.
    pub rows: Vec<RowModel>,
    pub params: Vec<RowModel>,
    /// What the export of the first selected entry writes; `None` until it is known.
    pub exported: Option<ExportedModel>,
    /// The entries that take a file name, PostScript name or style-linked slot another entry takes.
    pub conflicts: Vec<usize>,
    /// The first selected static instance in its own outlines.
    pub preview: Option<InstancePreview>,
}

/// One entry of the list.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ExportEntry {
    pub name: String,
    /// Exported with File › Export.
    pub active: bool,
    /// A variable font setting rather than a static instance.
    pub variable: bool,
}

/// A yes-or-no value of the selected entries: `mixed` when they differ.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq)]
pub struct Flag {
    pub on: bool,
    pub mixed: bool,
}

/// The built-in fields of the selected entries. A field whose entries differ is mixed; the static-only fields read
/// and edit the selected static instances.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ExportFields {
    /// The first selected entry is a variable font setting: the static-only fields do not show.
    pub variable: bool,
    pub active: Flag,
    pub name: String,
    pub name_mixed: bool,
    /// The selected static instances, which the static-only fields edit.
    pub statics: Vec<usize>,
    /// The class as a number; `None` when mixed or a name the tables do not know.
    pub weight: Option<f64>,
    pub width: Option<f64>,
    pub bold: Flag,
    pub italic: Flag,
    pub link_style: String,
    pub link_mixed: bool,
    /// One per font axis: its name and the shared coordinate (`None` when mixed).
    pub axes: Vec<(String, Option<f64>)>,
    /// The extrapolation warnings of the selected static instances.
    pub warnings: Vec<String>,
    /// The warnings `font.instance.list` gives for the selected entries' fields (`styleName`, `styleLinking`,
    /// `weightClass`), each once.
    pub field_warnings: Vec<FieldWarning>,
}

pub fn project(cx: &mut ProjectCx<'_>) -> ExportsModel {
    let placement = cx.session.query("font.instance.placement", &json!({})).unwrap_or(Value::Null);
    let listed = cx.session.query("font.instance.list", &json!({})).unwrap_or(Value::Null);
    let Ok(font) = cx.session.font() else { return ExportsModel::default() };
    let count = font.instances.len();
    let selected = shown_selection(&cx.view.font_info.exports.selected, count);
    let entries = font
        .instances
        .iter()
        .take(MAX_SHOWN)
        .map(|i| ExportEntry { name: i.name.trim().to_string(), active: i.exports, variable: is_variable(i) })
        .collect();
    let plus = PLUS_MENU.iter().map(|id| fontcraft_engine::find_command(id).is_some() && (*id != "font.instance.toMaster" || count > 0)).collect();
    let target = Target { scope: Scope::Instance, indices: selected.clone() };
    let (fields, rows_, params) = if selected.is_empty() {
        (None, Vec::new(), Vec::new())
    } else {
        (fields(font, &selected, &placement, &listed), rows(font, &target, RowKind::Field), rows(font, &target, RowKind::Param))
    };
    let params = match selected.as_slice() {
        [one] => code_problems(cx, *one, params),
        _ => params,
    };
    let mut model = ExportsModel { entries, selected, plus, fields, rows: rows_, params, ..ExportsModel::default() };
    exported_names::place(&mut model, cx);
    let shown = model.fields.as_ref().and_then(|f| f.statics.first().copied());
    model.preview = shown.and_then(|at| super::instance_preview::project(cx, at));
    model
}

/// Most diagnostics of an instance compile read for the parameters' problems.
const MAX_CODE_DIAGNOSTICS: usize = 1_000;

/// `params`, the rows of the one selected instance `instance`, with the first problem the last instance compile
/// (`font.features.compile` with `instance`, run by `crate::shell::feature_work`) found in each parameter whose code is
/// the one it compiled.
fn code_problems(cx: &ProjectCx<'_>, instance: usize, mut params: Vec<RowModel>) -> Vec<RowModel> {
    let Some(found) = &cx.view.font_info.exports.problems else { return params };
    if cx.session.active().map(|d| d.id) != Some(found.doc) || found.instance != instance {
        return params;
    }
    for row in &mut params {
        if let Editor::Code { text, problem } = &mut row.editor
            && row.enabled
        {
            let param = found.params.iter().find(|p| p.key == row.key && p.occurrence == row.occurrence && p.text == *text);
            *problem = param.and_then(|p| p.problem.clone());
        }
    }
    params
}

/// The feature-code parameters of `instance` that are switched on.
pub fn code_params(font: &Font, instance: usize) -> Vec<ParamProblem> {
    let target = Target { scope: Scope::Instance, indices: vec![instance] };
    rows(font, &target, RowKind::Param)
        .into_iter()
        .filter(|r| r.enabled)
        .filter_map(|r| match r.editor {
            Editor::Code { text, .. } => Some(ParamProblem { key: r.key, occurrence: r.occurrence, text, problem: None }),
            _ => None,
        })
        .collect()
}

/// Puts the first problem `reply` (`font.features.compile` with `instance`) has for each of `params` on it.
pub fn place_problems(params: &mut [ParamProblem], reply: &Value) {
    let list = |key: &str| reply.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default().iter().take(MAX_CODE_DIAGNOSTICS);
    let refused = list("refused").map(|d| (d, 0));
    let errors = list("diagnostics")
        .filter(|d| d.get("severity").and_then(Value::as_str) == Some("error"))
        .map(|d| (d, d.get("line").and_then(Value::as_u64).unwrap_or(0)));
    for (d, line) in refused.chain(errors) {
        let at = d.get("param").and_then(|p| {
            let index = usize::try_from(p.get("index")?.as_u64()?).ok()?;
            Some((p.get("name")?.as_str()?, index))
        });
        let Some((name, index)) = at else { continue };
        let Some(param) = params.iter_mut().find(|p| p.key == name && p.occurrence == index) else { continue };
        if param.problem.is_none() {
            param.problem = Some((line, d.get("message").and_then(Value::as_str).unwrap_or_default().to_string()));
        }
    }
}

/// The entries the tab acts on: the selected ones the font has, else the first.
pub fn shown_selection(wanted: &[usize], count: usize) -> Vec<usize> {
    let mut out: Vec<usize> = wanted.iter().copied().filter(|i| *i < count).collect();
    out.sort_unstable();
    out.dedup();
    if out.is_empty() && count > 0 {
        out.push(0);
    }
    out
}

/// Whether the entry is a variable font setting rather than a static instance.
pub fn is_variable(instance: &Instance) -> bool {
    instance.kind.as_deref() == Some("variable")
}

/// Whether the entries in `all` differ in what `f` reads.
fn differs<T: PartialEq>(all: &[&Instance], f: impl Fn(&Instance) -> T) -> bool {
    let mut values = all.iter().map(|i| f(i));
    match values.next() {
        Some(first) => values.any(|v| v != first),
        None => false,
    }
}

fn flag(all: &[&Instance], f: impl Fn(&Instance) -> bool) -> Flag {
    Flag { on: all.first().is_some_and(|i| f(i)), mixed: differs(all, f) }
}

/// The built-in fields of the `selected` instances of `font`; `placement` is `font.instance.placement`'s result (the
/// extrapolation warnings), `listed` is `font.instance.list`'s (the field warnings).
pub fn fields(font: &Font, selected: &[usize], placement: &Value, listed: &Value) -> Option<ExportFields> {
    let all: Vec<&Instance> = selected.iter().filter_map(|i| font.instances.get(*i)).collect();
    let first = *all.first()?;
    let statics: Vec<usize> = selected.iter().copied().filter(|i| font.instances.get(*i).is_some_and(|x| !is_variable(x))).collect();
    let stat: Vec<&Instance> = statics.iter().filter_map(|i| font.instances.get(*i)).collect();
    let class = |read: &dyn Fn(&Instance) -> Option<f64>| {
        let mixed = differs(&stat, |i| read(i).map(f64::to_bits));
        stat.first().and_then(|i| read(i)).filter(|_| !mixed)
    };
    let weight = class(&|i: &Instance| class_value(i.weight_class.as_ref(), WEIGHT_NAMES, DEFAULT_WEIGHT_CLASS));
    let width = class(&|i: &Instance| class_value(i.width_class.as_ref(), WIDTH_NAMES, DEFAULT_WIDTH_CLASS));
    let stat_first = stat.first().copied();
    let axes = font
        .axes
        .iter()
        .take(MAX_AXES_SHOWN)
        .enumerate()
        .map(|(at, axis)| {
            let mixed = differs(&stat, |i| design_value(&i.location, at).to_bits());
            (axis.name.clone(), stat_first.filter(|_| !mixed).map(|i| design_value(&i.location, at)))
        })
        .collect();
    let warnings = placement
        .get("instances")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| e.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).is_some_and(|i| statics.contains(&i)))
        .filter_map(|e| e.get("warning").and_then(Value::as_str).map(sentence))
        .take(MAX_WARNINGS_SHOWN.saturating_add(1))
        .collect();
    let mut by_field: Vec<FieldWarning> = Vec::new();
    let of_selected = listed
        .get("instances")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| e.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).is_some_and(|i| selected.contains(&i)));
    for warning in of_selected.flat_map(field_warnings) {
        if by_field.len() >= MAX_WARNINGS {
            break;
        }
        if !by_field.contains(&warning) {
            by_field.push(warning);
        }
    }
    Some(ExportFields {
        variable: is_variable(first),
        active: flag(&all, |i| i.exports),
        name: first.name.clone(),
        name_mixed: differs(&all, |i| i.name.clone()),
        weight,
        width,
        bold: flag(&stat, |i| i.is_bold),
        italic: flag(&stat, |i| i.is_italic),
        link_style: stat_first.and_then(|i| i.link_style.clone()).unwrap_or_default(),
        link_mixed: differs(&stat, |i| i.link_style.clone()),
        statics,
        axes,
        warnings,
        field_warnings: by_field,
    })
}

/// A class as the field shows it: its number, a known name's number, or the default when the source states none.
/// `None` for a name the tables do not know.
pub fn class_value(class: Option<&StyleClass>, names: &[(&str, u16)], default: f64) -> Option<f64> {
    match class {
        None => Some(default),
        Some(StyleClass::Number(n)) => n.is_finite().then_some(*n),
        Some(StyleClass::Name(name)) => {
            let key: String = name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
            names.iter().find(|(k, _)| *k == key).map(|(_, n)| f64::from(*n))
        }
    }
}

/// `text` with its first letter capitalised, as a line on its own reads.
pub fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}
