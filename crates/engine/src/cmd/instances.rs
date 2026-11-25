//! Export entry commands: the ten `font.instance.*` commands of M7, addressed through
//! `fontcraft_fontinfo::target`, classes within `fontcraft_fontinfo::WEIGHT_CLASS_RANGE` and
//! `WIDTH_CLASS_RANGE`.
//!
//! Rows: `font.instance.list`, `get`, `set`, `add`, `remove`, `reorder`, `setActive`,
//! `setStyleLinking`, `setWeightClass`, `setWidthClass`; `fontInfo.exports.orderIsCosmetic`,
//! `fontInfo.styleName.limits`, `fontInfo.weightClass.range`, `fontInfo.styleLinking.sameFamily`,
//! `instances.weightWidthClassIndependent`.
//!
//! Every edit validates all its parameters before it changes anything, and several targets get the
//! same change in one undo step. Warnings come from `fontcraft_fontinfo::warnings`: the edits
//! return those of the fields they set, and the queries give every entry's, by field. The weight
//! and width classes are stored apart from the axis coordinates: no command here derives one from
//! the other.

use fontcraft_fontinfo::{MAX_FIELD_BYTES, MAX_INSTANCES, WEIGHT_CLASS_RANGE, WIDTH_CLASS_RANGE, target, value, warnings as warn};
use fontcraft_model::axes::design_value;
use fontcraft_model::{Font, Instance, PropertyValue, StyleClass};
use serde_json::{Map, json};

use super::masters::moved_order;
use super::*;
use crate::params::{Params, kind};

/// The weight and width class an instance has when its source states none.
const DEFAULT_WEIGHT_CLASS: f64 = 400.0;
const DEFAULT_WIDTH_CLASS: f64 = 5.0;
/// The style name of a new instance.
const DEFAULT_STYLE_NAME: &str = "Regular";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.instance.list",
            "List Exports",
            [],
            None,
            "{type?: \"static\"|\"variable\", activeOnly?: bool} the export entries in list order → {instances: [{index, id, type, name, active, weightClass, widthClass, axes: {tag: number}, isBold, isItalic, linkStyle, warnings: [{field: 'styleName'|'styleLinking'|'weightClass', message}]}]}; weightClass and widthClass are 400 and 5 when the source states none; warnings are the cautions the edits give (an unsafe or long style name, a link to another family, a weight class between the standard steps)",
            has_doc,
            list
        ),
        cmd!(
            query "font.instance.get",
            "Get Export",
            [],
            None,
            "{instance: id|name|index} one export entry: the fields of font.instance.list plus visible, properties ({key: text, or {language: text} when localized}) and customParameters ([{name, value, enabled}]) → {instance: {…}}",
            has_doc,
            get
        ),
        cmd!(
            "font.instance.set",
            "Edit Export",
            [],
            None,
            "{instances: [id|name|index], active?: bool, styleName?: string, weightClass?: 1–1000, widthClass?: 1–9, axes?: {tag: number}, isBold?: bool, isItalic?: bool, linkStyle?: string (empty: no link)} write the given properties of every named entry as one undo step; axes sets the stored coordinates of the named axes only and leaves the classes alone; warnings cover the properties given (style name, style linking, weight class) → {changed: [keys], instances: [index], warnings: [string]}",
            has_doc,
            set
        ),
        cmd!(
            "font.instance.add",
            "Add Instance",
            [],
            None,
            "{styleName?: string, axes?: {tag: number}, weightClass?: 1–1000, widthClass?: 1–9} add an active static instance at the end of the list, named Regular unless a name is given, at the first master's coordinates unless axes are given; the classes are 400 and 5 unless given; warns about an unsafe or long style name and a weight class outside the standard steps → {index, name, warnings: [string]}",
            has_doc,
            add
        ),
        cmd!(
            "font.instance.remove",
            "Remove Export",
            [],
            None,
            "{instances: [id|name|index]} delete the named instances or variable font settings as one undo step → {removed: count}",
            has_doc,
            remove
        ),
        cmd!(
            "font.instance.reorder",
            "Reorder Exports",
            [],
            None,
            "{instances: [id|name|index] or instance: id|name|index, toIndex: integer} move export entries to another place in the list as one undo step; they end up next to each other in list order, the first of them at position toIndex, counted among the entries that stay; the order does not change any exported file → {index, indices}",
            has_doc,
            reorder
        ),
        cmd!(
            "font.instance.setActive",
            "Set Export Active",
            [],
            None,
            "{instances: [id|name|index], active: bool} turn the Active flag on or off; File › Export skips inactive entries → {instances: [index]}",
            has_doc,
            set_active
        ),
        cmd!(
            "font.instance.setStyleLinking",
            "Set Style Linking",
            [],
            None,
            "{instances: [id|name|index], isBold?: bool, isItalic?: bool, linkStyle?: string (empty: no link)} set the Bold and Italic checkboxes and the base style name of style linking; warns when the base instance of this font has another family name, since links work only within one family → {changed: [keys], instances: [index], warnings: [string]}",
            has_doc,
            set_style_linking
        ),
        cmd!(
            "font.instance.setWeightClass",
            "Set Weight Class",
            [],
            None,
            "{instances: [id|name|index], weightClass: 1–1000} set the Weight Class (100 to 900 are the standard steps); the axis coordinates do not change; warns about a value between or beyond the standard steps, which some applications refuse → {instances: [index], warnings: [string]}",
            has_doc,
            set_weight_class
        ),
        cmd!(
            "font.instance.setWidthClass",
            "Set Width Class",
            [],
            None,
            "{instances: [id|name|index], widthClass: 1–9} set the Width Class; the axis coordinates do not change → {instances: [index]}",
            has_doc,
            set_width_class
        ),
    ]
}

// ---------- reading ----------

fn list(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.list", v)?;
    let only_type = p.opt_str("type")?;
    if let Some(t) = only_type
        && t != "static"
        && t != "variable"
    {
        return Err(p.bad(format!("`type` must be \"static\" or \"variable\", got `{t}`")));
    }
    let active_only = p.bool_or("activeOnly", false)?;
    let font = s.font()?;
    let items: Vec<Value> = font
        .instances
        .iter()
        .enumerate()
        .filter(|(_, inst)| only_type.is_none_or(|t| type_of(inst) == t))
        .filter(|(_, inst)| !active_only || inst.exports)
        .map(|(i, inst)| summary(font, i, inst))
        .collect();
    Ok(json!({ "instances": items }))
}

fn get(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.get", v)?;
    let font = s.font()?;
    let i = one_target(&p, font, "instance")?;
    let inst = font.instances.get(i).ok_or_else(|| p.bad(format!("there is no instance #{i}")))?;
    let mut out = summary(font, i, inst);
    if let Value::Object(map) = &mut out {
        map.insert("visible".into(), json!(inst.visible));
        map.insert("properties".into(), properties_json(inst));
        map.insert("customParameters".into(), params_json(inst));
    }
    Ok(json!({ "instance": out }))
}

/// The fields `font.instance.list` gives for one entry.
fn summary(font: &Font, index: usize, inst: &Instance) -> Value {
    json!({
        "index": index,
        "id": inst.id,
        "type": type_of(inst),
        "name": inst.name,
        "active": inst.exports,
        "weightClass": class_json(inst.weight_class.as_ref(), DEFAULT_WEIGHT_CLASS),
        "widthClass": class_json(inst.width_class.as_ref(), DEFAULT_WIDTH_CLASS),
        "axes": axes_json(font, &inst.location),
        "isBold": inst.is_bold,
        "isItalic": inst.is_italic,
        "linkStyle": inst.link_style,
        "warnings": super::font::warnings_json(warn::instance(font, inst)),
    })
}

/// `static` for a static instance, else the kind the source names (`variable`).
fn type_of(inst: &Instance) -> &str {
    inst.kind.as_deref().unwrap_or("static")
}

/// A class as stored: a number, or the name an older source writes (`Bold`).
fn class_json(class: Option<&StyleClass>, default: f64) -> Value {
    match class {
        None => number_json(default),
        Some(StyleClass::Number(n)) => number_json(*n),
        Some(StyleClass::Name(name)) => json!(name),
    }
}

/// Whole numbers as JSON integers, so `400` reads back as `400`, not `400.0`.
fn number_json(n: f64) -> Value {
    const EXACT: f64 = 9_007_199_254_740_992.0;
    if n.is_finite() && n.fract() == 0.0 && n.abs() < EXACT { json!(n as i64) } else { json!(n) }
}

fn axes_json(font: &Font, location: &[f64]) -> Value {
    let mut map = Map::new();
    for (i, axis) in font.axes.iter().enumerate() {
        map.entry(axis.tag.clone()).or_insert_with(|| number_json(design_value(location, i)));
    }
    Value::Object(map)
}

fn properties_json(inst: &Instance) -> Value {
    let mut map = Map::new();
    for prop in &inst.properties.0 {
        let value = match &prop.value {
            PropertyValue::Single(text) => json!(text),
            PropertyValue::Localized(list) => Value::Object(list.iter().map(|l| (l.language.clone(), json!(l.value))).collect()),
        };
        map.entry(prop.key.clone()).or_insert(value);
    }
    Value::Object(map)
}

fn params_json(inst: &Instance) -> Value {
    Value::Array(
        inst.params.0.iter().map(|param| json!({"name": param.name, "value": value::to_json(&param.value), "enabled": !param.disabled})).collect(),
    )
}

// ---------- targets and values ----------

/// The instances a `key` parameter names, each once; at least one.
fn many_targets(p: &Params, font: &Font, key: &str) -> Result<Vec<usize>> {
    let raw = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
    if refs.is_empty() {
        return Err(p.bad(format!("`{key}` names no instance")));
    }
    target::instances(font, &refs).map_err(|e| p.bad(e.to_string()))
}

/// The one instance a `key` parameter names.
fn one_target(p: &Params, font: &Font, key: &str) -> Result<usize> {
    let raw = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    let r = target::parse(raw).map_err(|e| p.bad(e.to_string()))?;
    target::instance(font, &r).map_err(|e| p.bad(e.to_string()))
}

fn opt_bool(p: &Params, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => Err(p.bad(format!("`{key}` must be true or false, got {}", kind(other)))),
    }
}

fn opt_weight_class(p: &Params) -> Result<Option<u16>> {
    class_param(p, "weightClass", &WEIGHT_CLASS_RANGE)
}

fn opt_width_class(p: &Params) -> Result<Option<u16>> {
    class_param(p, "widthClass", &WIDTH_CLASS_RANGE)
}

fn class_param(p: &Params, key: &str, range: &std::ops::RangeInclusive<u16>) -> Result<Option<u16>> {
    let Some(n) = p.opt_int(key, i64::from(*range.start()), i64::from(*range.end()))? else { return Ok(None) };
    u16::try_from(n).map(Some).map_err(|_| p.bad(format!("`{key}` is out of range")))
}

/// A style name: not empty, within the field limit.
fn opt_style_name(p: &Params) -> Result<Option<String>> {
    let Some(name) = p.opt_str("styleName")? else { return Ok(None) };
    if name.trim().is_empty() {
        return Err(p.bad("`styleName` cannot be empty"));
    }
    if name.len() > MAX_FIELD_BYTES {
        return Err(p.bad(format!("`styleName` is longer than {MAX_FIELD_BYTES} bytes")));
    }
    Ok(Some(name.to_string()))
}

/// The base style of style linking: `Some(None)` clears the link (an empty string).
fn opt_link_style(p: &Params) -> Result<Option<Option<String>>> {
    let Some(name) = p.opt_str("linkStyle")? else { return Ok(None) };
    if name.len() > MAX_FIELD_BYTES {
        return Err(p.bad(format!("`linkStyle` is longer than {MAX_FIELD_BYTES} bytes")));
    }
    let name = name.trim();
    Ok(Some((!name.is_empty()).then(|| name.to_string())))
}

/// `axes: {tag: number}` as (axis index, coordinate) pairs; every tag must be one of the font's
/// axes and every coordinate finite.
fn opt_axes(p: &Params, font: &Font) -> Result<Vec<(usize, f64)>> {
    let Some(map) = p.opt_object("axes")? else { return Ok(Vec::new()) };
    let mut out = Vec::with_capacity(map.len().min(font.axes.len()));
    for (tag, v) in map {
        let i = font.axes.iter().position(|a| &a.tag == tag).ok_or_else(|| p.bad(format!("the font has no axis `{tag}`")))?;
        let n = v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| p.bad(format!("`axes.{tag}` must be a finite number, got {}", kind(v))))?;
        out.push((i, n));
    }
    Ok(out)
}

/// Writes `axes` into `location`, filling the axes it leaves out with their current (or default)
/// coordinates so the location stays parallel to the font's axes.
fn place(location: &mut Vec<f64>, axis_count: usize, axes: &[(usize, f64)]) {
    if axes.is_empty() {
        return;
    }
    let mut next: Vec<f64> = (0..axis_count).map(|i| design_value(location, i)).collect();
    for (i, n) in axes {
        if let Some(slot) = next.get_mut(*i) {
            *slot = *n;
        }
    }
    *location = next;
}

// ---------- editing ----------

/// The built-in properties `font.instance.set` and the narrower commands write.
#[derive(Default)]
struct Changes {
    active: Option<bool>,
    style_name: Option<String>,
    weight_class: Option<u16>,
    width_class: Option<u16>,
    axes: Vec<(usize, f64)>,
    is_bold: Option<bool>,
    is_italic: Option<bool>,
    link_style: Option<Option<String>>,
}

impl Changes {
    fn keys(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let mut note = |on: bool, key| {
            if on {
                out.push(key);
            }
        };
        note(self.active.is_some(), "active");
        note(self.style_name.is_some(), "styleName");
        note(self.weight_class.is_some(), "weightClass");
        note(self.width_class.is_some(), "widthClass");
        note(!self.axes.is_empty(), "axes");
        note(self.is_bold.is_some(), "isBold");
        note(self.is_italic.is_some(), "isItalic");
        note(self.link_style.is_some(), "linkStyle");
        out
    }

    fn apply(&self, inst: &mut Instance, axis_count: usize) {
        if let Some(a) = self.active {
            inst.exports = a;
        }
        if let Some(name) = &self.style_name {
            inst.name.clone_from(name);
        }
        if let Some(w) = self.weight_class {
            inst.weight_class = Some(StyleClass::Number(f64::from(w)));
        }
        if let Some(w) = self.width_class {
            inst.width_class = Some(StyleClass::Number(f64::from(w)));
        }
        place(&mut inst.location, axis_count, &self.axes);
        if let Some(b) = self.is_bold {
            inst.is_bold = b;
        }
        if let Some(i) = self.is_italic {
            inst.is_italic = i;
        }
        if let Some(link) = &self.link_style {
            inst.link_style.clone_from(link);
        }
    }
}

/// What an edit of several instances did.
struct Edited {
    changed: Vec<&'static str>,
    instances: Vec<usize>,
    warnings: Vec<String>,
}

impl Edited {
    fn full(self) -> Value {
        json!({ "changed": self.changed, "instances": self.instances, "warnings": self.warnings })
    }

    fn targets_only(self) -> Value {
        json!({ "instances": self.instances })
    }

    fn with_warnings(self) -> Value {
        json!({ "instances": self.instances, "warnings": self.warnings })
    }
}

/// Applies `changes` to the instances `key` names as one undo step, with the warnings of the
/// edited entries.
fn edit_instances(s: &mut Session, p: &Params, key: &str, label: &str, changes: &Changes) -> Result<Edited> {
    let keys = changes.keys();
    if keys.is_empty() {
        return Err(p.bad("nothing to change: give at least one property"));
    }
    let targets = many_targets(p, s.font()?, key)?;
    let warnings = s.edit(label, |font| {
        let axis_count = font.axes.len();
        let instances = std::sync::Arc::make_mut(&mut font.instances);
        for &i in &targets {
            let inst = instances.get_mut(i).ok_or_else(|| p.bad(format!("there is no instance #{i}")))?;
            changes.apply(inst, axis_count);
        }
        let font: &Font = font;
        let mut warnings = Vec::new();
        for &i in &targets {
            let Some(inst) = font.instances.get(i) else { continue };
            if changes.style_name.is_some() {
                warnings.extend(warn::style_name(font, inst));
            }
            if changes.is_bold.is_some() || changes.is_italic.is_some() || changes.link_style.is_some() {
                warnings.extend(warn::style_link(font, inst));
            }
            if changes.weight_class.is_some() {
                warnings.extend(warn::instance_weight_class(inst));
            }
        }
        // The class warning names no instance, so several targets would repeat it.
        warnings.dedup();
        Ok(warnings)
    })?;
    Ok(Edited { changed: keys, instances: targets, warnings })
}

fn set(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.set", v)?;
    let changes = Changes {
        active: opt_bool(&p, "active")?,
        style_name: opt_style_name(&p)?,
        weight_class: opt_weight_class(&p)?,
        width_class: opt_width_class(&p)?,
        axes: opt_axes(&p, s.font()?)?,
        is_bold: opt_bool(&p, "isBold")?,
        is_italic: opt_bool(&p, "isItalic")?,
        link_style: opt_link_style(&p)?,
    };
    edit_instances(s, &p, "instances", "Edit Export", &changes).map(Edited::full)
}

fn set_active(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.setActive", v)?;
    let active = opt_bool(&p, "active")?.ok_or_else(|| p.bad("`active` is required"))?;
    edit_instances(s, &p, "instances", "Set Export Active", &Changes { active: Some(active), ..Changes::default() }).map(Edited::targets_only)
}

fn set_style_linking(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.setStyleLinking", v)?;
    let changes =
        Changes { is_bold: opt_bool(&p, "isBold")?, is_italic: opt_bool(&p, "isItalic")?, link_style: opt_link_style(&p)?, ..Changes::default() };
    edit_instances(s, &p, "instances", "Set Style Linking", &changes).map(Edited::full)
}

fn set_weight_class(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.setWeightClass", v)?;
    let class = opt_weight_class(&p)?.ok_or_else(|| p.bad("`weightClass` is required"))?;
    edit_instances(s, &p, "instances", "Set Weight Class", &Changes { weight_class: Some(class), ..Changes::default() }).map(Edited::with_warnings)
}

fn set_width_class(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.setWidthClass", v)?;
    let class = opt_width_class(&p)?.ok_or_else(|| p.bad("`widthClass` is required"))?;
    edit_instances(s, &p, "instances", "Set Width Class", &Changes { width_class: Some(class), ..Changes::default() }).map(Edited::targets_only)
}

fn add(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.add", v)?;
    let name = opt_style_name(&p)?.unwrap_or_else(|| DEFAULT_STYLE_NAME.to_string());
    let weight_class = opt_weight_class(&p)?;
    let width_class = opt_width_class(&p)?;
    let axes = opt_axes(&p, s.font()?)?;
    s.edit("Add Instance", |font| {
        if font.instances.len() >= MAX_INSTANCES {
            return Err(p.bad(format!("a font holds at most {MAX_INSTANCES} export entries")));
        }
        let axis_count = font.axes.len();
        let mut location: Vec<f64> = match font.masters.first() {
            Some(m) => (0..axis_count).map(|i| design_value(&m.location, i)).collect(),
            None => (0..axis_count).map(|i| design_value(&[], i)).collect(),
        };
        place(&mut location, axis_count, &axes);
        let inst = Instance {
            name,
            location,
            weight_class: weight_class.map(|w| StyleClass::Number(f64::from(w))),
            width_class: width_class.map(|w| StyleClass::Number(f64::from(w))),
            ..Instance::default()
        };
        let mut warnings = warn::style_name(font, &inst);
        warnings.extend(warn::instance_weight_class(&inst));
        let out = json!({ "index": font.instances.len(), "name": inst.name, "warnings": warnings });
        std::sync::Arc::make_mut(&mut font.instances).push(inst);
        Ok(out)
    })
}

fn remove(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.remove", v)?;
    let mut targets = many_targets(&p, s.font()?, "instances")?;
    targets.sort_unstable();
    let count = targets.len();
    s.edit("Remove Export", |font| {
        let instances = std::sync::Arc::make_mut(&mut font.instances);
        for &i in targets.iter().rev() {
            if i >= instances.len() {
                return Err(p.bad(format!("there is no instance #{i}")));
            }
            instances.remove(i);
        }
        Ok(())
    })?;
    Ok(json!({ "removed": count }))
}

fn reorder(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.reorder", v)?;
    let font = s.font()?;
    let moved = match (p.get("instance"), p.get("instances")) {
        (Some(_), Some(_)) => return Err(p.bad("give `instance` or `instances`, not both")),
        (Some(_), None) => vec![one_target(&p, font, "instance")?],
        (None, _) => many_targets(&p, font, "instances")?,
    };
    let order = moved_order(&p, font.instances.len(), moved)?;
    if order.changes() {
        s.edit("Reorder Exports", |font| {
            font.instances = std::sync::Arc::new(order.order.iter().filter_map(|i| font.instances.get(*i).cloned()).collect());
            Ok(())
        })?;
    }
    Ok(json!({ "index": order.to, "indices": order.indices() }))
}
