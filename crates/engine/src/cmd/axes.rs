//! Axis commands (M9.15): list, add, set, remove and reorder axes, the predefined axes, and the
//! axis locations of masters and instances.
//!
//! Rows: `font.axes.list`, `font.axes.add`, `font.axes.set`, `font.axes.remove`,
//! `font.axes.reorder`, `font.axes.listPredefined`, `font.master.setAxisLocation`,
//! `font.instance.setAxisLocation`, `fontInfo.axes.predefinedNameSetsTag`,
//! `fontInfo.axes.hidden`.
//!
//! Adding, removing or reordering an axis rewrites every location in the font (masters,
//! instances, intermediate coordinates, alternate ranges, the axis parameters) in the same undo
//! step.
//!
//! What each axis edit rewrites:
//!
//! | Data | Keyed by | Add | Remove | Reorder | Rename / retag |
//! |---|---|---|---|---|---|
//! | master and instance locations | axis order | 0 on the new axis | coordinate dropped | permuted | – |
//! | intermediate coordinates, alternate ranges | axis order | – (a missing entry inherits / is open) | entry dropped | permuted | – |
//! | `Axis Location` (masters, instances) | axis name | – | entry dropped | sorted | renamed |
//! | `Virtual Master` (font) | axis name | entry at 0 | entry dropped | sorted | renamed |
//! | `Axis Mappings` (font) | axis tag | – | entry dropped | sorted | retagged |
//!
//! Locations shorter than the axis list are read with the model's defaults
//! ([`fontcraft_model::axes::design_value`]) and written out in full, so an edit never changes
//! what a short location means. A variable font setting without a location keeps none.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_fontinfo::target;
use fontcraft_model::axes::{AXIS_LOCATION, AXIS_MAPPINGS, VIRTUAL_MASTER, design_value, number_value};
use fontcraft_model::plist::{Dict, Str, Value as Plist};
use fontcraft_model::{Axis, AxisRule, Font, Instance, MAX_AXIS_VALUES, MAX_COORDINATE, MAX_NAME_LEN, Params as ModelParams};
use serde_json::{Map, json};

use super::*;
use crate::params::{Params, kind};

/// The coordinate masters, instances and virtual masters get on a new axis.
const NEW_AXIS_VALUE: f64 = 0.0;

/// Most glyph names one warning lists.
const MAX_NAMED_IN_WARNING: usize = 10;

/// The axes the name field offers, with the tag each sets: the registered OpenType axes, then
/// axes proposed for registration (`true` marks a registered one).
const PREDEFINED: [(&str, &str, bool); 14] = [
    ("Weight", "wght", true),
    ("Width", "wdth", true),
    ("Italic", "ital", true),
    ("Slant", "slnt", true),
    ("Optical Size", "opsz", true),
    ("Grade", "GRAD", false),
    ("Thick Stroke", "XOPQ", false),
    ("Thin Stroke", "YOPQ", false),
    ("Counter Width", "XTRA", false),
    ("Uppercase Height", "YTUC", false),
    ("Lowercase Height", "YTLC", false),
    ("Ascender Height", "YTAS", false),
    ("Descender Depth", "YTDE", false),
    ("Figure Height", "YTFI", false),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "font.axes.list", "List Axes", [], None, "{} the design axes in order → {axes: [{index, name, tag, hidden}]}", has_doc, list),
        cmd!(
            "font.axes.add",
            "Add Axis",
            [],
            None,
            "{name?: string, tag?: string, hidden?: bool = false} append a design axis. A predefined name sets its tag; without name and tag the axis is Weight (`wght`) when the font has none, else `Axis N` with a free private tag. Every master, instance and virtual master gets coordinate 0 on it; special layers inherit or stay open. A tag or name another axis has is refused → {axis: {index, name, tag, hidden}, warnings: [string]}",
            has_doc,
            add
        ),
        cmd!(
            "font.axes.set",
            "Edit Axis",
            [],
            None,
            "{axis: index|tag|name, name?: string, tag?: string, hidden?: bool} change an axis's name, tag (one to four printable ASCII characters) or Hidden flag. A predefined name given without `tag` sets the matching tag. Axis Location and Virtual Master entries follow a new name, Axis Mappings a new tag → {axis: {index, name, tag, hidden}, warnings: [string]}",
            has_doc,
            set
        ),
        cmd!(
            "font.axes.remove",
            "Remove Axis",
            [],
            None,
            "{axis: index|tag|name} delete an axis and its coordinate from every master, instance, virtual master, intermediate layer and alternate range, and its axis parameter entries. An alternate layer left without ranges stays an alternate layer `[]` → {removed: {index, name, tag, hidden}, warnings: [string]}",
            has_doc,
            remove
        ),
        cmd!(
            "font.axes.reorder",
            "Reorder Axes",
            [],
            None,
            "{order: [tag]} put the axes in this order, every tag once; every location, special-layer coordinate and range list is permuted with them, so no location changes → {axes: [tag]}",
            has_doc,
            reorder
        ),
        cmd!(
            query "font.axes.listPredefined",
            "List Predefined Axis Names",
            [],
            None,
            "{} the predefined axis names and the tag each sets: the registered axes, then those proposed for registration → {axes: [{name, tag, registered}]}",
            has_doc,
            list_predefined
        ),
        cmd!(
            "font.master.setAxisLocation",
            "Set Master Axes Coordinates",
            [],
            None,
            "{masters: [id|name|index], location: {tag: number}} place the masters in the designspace; axes not named keep their coordinates → {masters: [id], warnings: [string]}",
            has_doc,
            set_master_location
        ),
        cmd!(
            "font.instance.setAxisLocation",
            "Set Instance Axes Coordinates",
            [],
            None,
            "{instances: [id|name|index], location: {tag: number}} place the instances in the designspace; axes not named keep their coordinates. A location beyond the masters is extrapolated and named in the warnings → {instances: [index], warnings: [string]}",
            has_doc,
            set_instance_location
        ),
    ]
}

// ── Reading and checking ────────────────────────────────────────────────────────────────────────

fn axis_json(index: usize, a: &Axis) -> Value {
    json!({"index": index, "name": a.name, "tag": a.tag, "hidden": a.hidden})
}

/// The tag a predefined axis name sets, the name matched without regard to case.
fn predefined_tag(name: &str) -> Option<&'static str> {
    let name = name.trim();
    PREDEFINED.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)).map(|(_, t, _)| *t)
}

fn predefined_name(tag: &str) -> Option<&'static str> {
    PREDEFINED.iter().find(|(_, t, _)| *t == tag).map(|(n, _, _)| *n)
}

/// A tag as it is exported (four characters, padded with spaces), so `abc` and `abc ` collide.
fn tag_key(tag: &str) -> String {
    fontcraft_interp::designspace::export_tag(tag).unwrap_or_else(|_| tag.to_string())
}

fn check_name(p: &Params, name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(p.bad("`name` cannot be empty"));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`name` is longer than {MAX_NAME_LEN} bytes")));
    }
    if name.chars().any(char::is_control) {
        return Err(p.bad("`name` cannot hold control characters"));
    }
    Ok(())
}

fn check_tag(p: &Params, tag: &str) -> Result<()> {
    let printable = tag.chars().all(|c| (' '..='~').contains(&c));
    if tag.is_empty() || tag.len() > 4 || tag.starts_with(' ') || !printable {
        return Err(p.bad(format!("`tag` must be one to four printable ASCII characters, not starting with a space; got `{tag}`")));
    }
    Ok(())
}

/// Refuses a name or a tag another axis than `except` already has.
fn check_unique(p: &Params, font: &Font, name: &str, tag: &str, except: Option<usize>) -> Result<()> {
    let key = tag_key(tag);
    for (i, a) in font.axes.iter().enumerate() {
        if Some(i) == except {
            continue;
        }
        if a.name == name {
            return Err(p.bad(format!("the font already has an axis named `{name}`")));
        }
        if tag_key(&a.tag) == key {
            return Err(p.bad(format!("the font already has an axis with tag `{tag}` (`{}`)", a.name)));
        }
    }
    Ok(())
}

/// What the tag rules (`axes.tagRules`) say about a tag the user chose.
fn tag_warnings(tag: &str) -> Vec<String> {
    let mut out = Vec::new();
    if tag.len() < 4 {
        out.push(format!("the tag `{tag}` has fewer than four characters; it is padded with spaces at export"));
    }
    let registered = PREDEFINED.iter().any(|(_, t, r)| *r && *t == tag);
    let capitals = tag.len() == 4 && tag.chars().all(|c| c.is_ascii_uppercase());
    if !registered && !capitals {
        out.push(format!(
            "`{tag}` is not a registered axis tag; a private axis should use four capital letters so it cannot clash with an axis registered later"
        ));
    }
    out
}

fn name_taken(font: &Font, name: &str) -> bool {
    font.axes.iter().any(|a| a.name == name)
}

fn tag_taken(font: &Font, tag: &str) -> bool {
    let key = tag_key(tag);
    font.axes.iter().any(|a| tag_key(&a.tag) == key)
}

/// `Axis N`, N the new axis's place in the list, or the next free number.
fn free_axis_name(font: &Font) -> String {
    (font.axes.len() + 1..=font.axes.len() + 1 + MAX_AXIS_VALUES)
        .map(|n| format!("Axis {n}"))
        .find(|n| !name_taken(font, n))
        .unwrap_or_else(|| format!("Axis {}", font.axes.len() + 1))
}

/// The first free private tag of `AXSA`, `AXSB`, … `AXZZ`: four capital letters, as the tag rules
/// ask of a private axis.
fn free_private_tag(font: &Font) -> String {
    ('S'..='Z').flat_map(|a| ('A'..='Z').map(move |b| format!("AX{a}{b}"))).find(|t| !tag_taken(font, t)).unwrap_or_else(|| "AXSA".to_string())
}

/// The axis `axis` names: an index, else a tag, else a name.
fn axis_index(p: &Params, font: &Font) -> Result<usize> {
    let raw = p.get("axis").ok_or_else(|| p.bad("`axis` is required"))?;
    if font.axes.is_empty() {
        return Err(p.bad("the font has no axes"));
    }
    match raw {
        Value::Number(_) => {
            let last = i64::try_from(font.axes.len().saturating_sub(1)).unwrap_or(i64::MAX);
            let i = p.opt_int("axis", 0, last)?.ok_or_else(|| p.bad("`axis` is required"))?;
            usize::try_from(i).map_err(|_| p.bad("`axis` must not be negative"))
        }
        Value::String(key) => font
            .axes
            .iter()
            .position(|a| a.tag == *key)
            .or_else(|| font.axes.iter().position(|a| a.name == *key))
            .ok_or_else(|| p.bad(format!("the font has no axis `{key}`"))),
        other => Err(p.bad(format!("`axis` must be an index, a tag or a name, got {}", kind(other)))),
    }
}

/// `location: {tag: number}` as (axis index, coordinate) pairs: at least one, every tag an axis
/// of the font, every coordinate finite and within ±[`MAX_COORDINATE`].
fn read_location(p: &Params, font: &Font) -> Result<Vec<(usize, f64)>> {
    let map: &Map<String, Value> = p.opt_object("location")?.ok_or_else(|| p.bad("`location` is required"))?;
    if map.is_empty() {
        return Err(p.bad("`location` names no axis"));
    }
    if map.len() > font.axes.len() {
        return Err(p.bad(format!("`location` names {} axes; the font has {}", map.len(), font.axes.len())));
    }
    let mut out = Vec::with_capacity(map.len());
    for (tag, v) in map {
        let i = font.axes.iter().position(|a| a.tag == *tag).ok_or_else(|| p.bad(format!("the font has no axis `{tag}`")))?;
        match v.as_f64() {
            Some(n) if n.is_finite() && n.abs() <= MAX_COORDINATE => out.push((i, n)),
            _ => return Err(p.bad(format!("`location.{tag}` must be a finite number within ±{MAX_COORDINATE}, got {}", kind(v)))),
        }
    }
    Ok(out)
}

/// `old` in full (one coordinate per axis) with `values` written in.
fn placed(old: &[f64], axis_count: usize, values: &[(usize, f64)]) -> Vec<f64> {
    let mut next: Vec<f64> = (0..axis_count).map(|i| design_value(old, i)).collect();
    for &(i, v) in values {
        if let Some(slot) = next.get_mut(i) {
            *slot = v;
        }
    }
    next
}

fn is_setting(inst: &Instance) -> bool {
    inst.kind.as_deref() == Some("variable")
}

/// The bits of a location, with −0 and 0 the same, for grouping equal locations.
fn location_key(location: &[f64], axis_count: usize) -> Vec<u64> {
    (0..axis_count).map(|i| design_value(location, i)).map(|v| if v == 0.0 { 0 } else { v.to_bits() }).collect()
}

/// One warning for each group of masters that share a location.
fn shared_location_warnings(font: &Font) -> Vec<String> {
    let n = font.axes.len();
    let mut groups: Vec<(Vec<u64>, Vec<&str>)> = Vec::new();
    let mut at: HashMap<Vec<u64>, usize> = HashMap::new();
    for m in font.masters.iter() {
        let key = location_key(&m.location, n);
        match at.get(&key).and_then(|&g| groups.get_mut(g)) {
            Some((_, names)) => names.push(&m.name),
            None => {
                at.insert(key.clone(), groups.len());
                groups.push((key, vec![m.name.as_str()]));
            }
        }
    }
    groups
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(_, names)| {
            format!(
                "masters {} share one location; interpolation needs them apart",
                names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
            )
        })
        .collect()
}

// ── Rewriting locations and parameters ──────────────────────────────────────────────────────────

/// Where each axis of the edited list comes from: `Some(old index)`, or `None` for a new axis.
struct Remap(Vec<Option<usize>>);

impl Remap {
    fn location(&self, old: &[f64]) -> Vec<f64> {
        self.0.iter().map(|o| o.map_or(NEW_AXIS_VALUE, |i| design_value(old, i))).collect()
    }

    /// An intermediate layer's coordinates; a missing entry inherits the master's value, so
    /// trailing ones are left out. A layer that had coordinates keeps at least one entry and so
    /// stays an intermediate layer.
    fn coordinates(&self, old: &[Option<f64>]) -> Vec<Option<f64>> {
        if old.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<Option<f64>> = self.0.iter().map(|o| o.and_then(|i| old.get(i).copied().flatten())).collect();
        while out.len() > 1 && out.last().is_some_and(Option::is_none) {
            out.pop();
        }
        if out.is_empty() {
            out.push(None);
        }
        out
    }

    /// An alternate layer's ranges; a missing entry is open. A layer that had ranges keeps at
    /// least one, so it stays an alternate layer (`[]` when every bound is gone).
    fn rules(&self, old: &[AxisRule]) -> Vec<AxisRule> {
        if old.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<AxisRule> = self.0.iter().map(|o| o.and_then(|i| old.get(i).copied()).unwrap_or_default()).collect();
        while out.len() > 1 && out.last().is_some_and(|r| *r == AxisRule::default()) {
            out.pop();
        }
        if out.is_empty() {
            out.push(AxisRule::default());
        }
        out
    }
}

/// Rewrites the master and instance locations, and with `layers` the special layers'
/// coordinates and ranges. Glyphs whose layers do not change stay shared.
fn apply_remap(font: &mut Font, remap: &Remap, layers: bool) {
    for m in Arc::make_mut(&mut font.masters).iter_mut() {
        m.location = remap.location(&m.location);
    }
    if !font.instances.is_empty() {
        for inst in Arc::make_mut(&mut font.instances).iter_mut() {
            if is_setting(inst) && inst.location.is_empty() {
                continue;
            }
            inst.location = remap.location(&inst.location);
        }
    }
    if !layers {
        return;
    }
    for gi in 0..font.glyphs.len() {
        let Some(g) = font.glyphs.as_arcs().get(gi) else { continue };
        let changes: Vec<(usize, Vec<Option<f64>>, Vec<AxisRule>)> = g
            .layers
            .iter()
            .enumerate()
            .filter_map(|(li, l)| {
                let a = &l.info.attrs;
                if a.coordinates.is_empty() && a.axis_rules.is_empty() {
                    return None;
                }
                let c = remap.coordinates(&a.coordinates);
                let r = remap.rules(&a.axis_rules);
                (c != a.coordinates || r != a.axis_rules).then_some((li, c, r))
            })
            .collect();
        if changes.is_empty() {
            continue;
        }
        if let Some(mut g) = font.glyphs.get_mut_at(gi) {
            for (li, c, r) in changes {
                if let Some(l) = g.layers.get_mut(li) {
                    l.info.attrs.coordinates = c;
                    l.info.attrs.axis_rules = r;
                }
            }
        }
    }
}

/// An axis edit as the axis parameters see it.
enum ParamEdit<'a> {
    Add {
        name: &'a str,
    },
    Remove {
        name: &'a str,
        tag: &'a str,
    },
    /// The axes' names and tags in their new order.
    Reorder {
        names: &'a [String],
        tags: &'a [String],
    },
    Rename {
        from_name: &'a str,
        to_name: &'a str,
        from_tag: &'a str,
        to_tag: &'a str,
    },
}

fn axis_of(item: &Plist) -> Option<&str> {
    item.as_dict().and_then(|d| d.get("Axis")).and_then(Plist::as_str)
}

/// Edits an `Axis Location` or `Virtual Master` value (a list of `{Axis, Location}`, by axis
/// name). Returns whether the edit emptied it. A value of another shape is left alone.
fn edit_location_list(value: &mut Plist, edit: &ParamEdit, is_virtual: bool) -> bool {
    let Plist::Array(list) = value else { return false };
    let was_empty = list.items.is_empty();
    match edit {
        ParamEdit::Add { name } => {
            if is_virtual
                && !list.items.iter().any(|i| axis_of(i) == Some(*name))
                && let Some(v) = number_value(NEW_AXIS_VALUE)
            {
                let mut d = Dict::new();
                d.insert("Axis", Plist::String(Str::new(*name)));
                d.insert("Location", v);
                list.items.push(Plist::Dict(d));
            }
        }
        ParamEdit::Remove { name, .. } => list.items.retain(|i| axis_of(i) != Some(*name)),
        ParamEdit::Reorder { names, .. } => {
            list.items.sort_by_key(|i| axis_of(i).and_then(|a| names.iter().position(|n| n == a)).unwrap_or(usize::MAX));
        }
        ParamEdit::Rename { from_name, to_name, .. } => {
            if from_name != to_name {
                for item in &mut list.items {
                    if let Plist::Dict(d) = item
                        && d.get("Axis").and_then(Plist::as_str) == Some(*from_name)
                    {
                        d.insert("Axis", Plist::String(Str::new(*to_name)));
                    }
                }
            }
        }
    }
    !was_empty && list.items.is_empty()
}

/// Edits an `Axis Mappings` value (`{tag: {external: internal}}`). Returns whether the edit
/// emptied it.
fn edit_mappings(value: &mut Plist, edit: &ParamEdit) -> bool {
    let Plist::Dict(d) = value else { return false };
    let was_empty = d.entries.is_empty();
    match edit {
        ParamEdit::Add { .. } => {}
        ParamEdit::Remove { tag, .. } => d.entries.retain(|(k, _)| k.as_str() != *tag),
        ParamEdit::Reorder { tags, .. } => {
            d.entries.sort_by_key(|(k, _)| tags.iter().position(|t| t == k.as_str()).unwrap_or(usize::MAX));
        }
        ParamEdit::Rename { from_tag, to_tag, .. } => {
            if from_tag != to_tag {
                for (k, _) in &mut d.entries {
                    if k.as_str() == *from_tag {
                        *k = Str::new(*to_tag);
                    }
                }
            }
        }
    }
    !was_empty && d.entries.is_empty()
}

/// `params` rewritten for `edit`, switched-off parameters included; a parameter the edit empties
/// is removed. `None` when nothing changes.
fn edit_params(params: &ModelParams, edit: &ParamEdit) -> Option<ModelParams> {
    let touched = |name: &str| name == AXIS_LOCATION || name == VIRTUAL_MASTER || name == AXIS_MAPPINGS;
    if !params.0.iter().any(|p| touched(&p.name)) {
        return None;
    }
    let mut out = params.clone();
    let emptied: Vec<bool> = out
        .0
        .iter_mut()
        .map(|prm| match prm.name.as_str() {
            AXIS_LOCATION => edit_location_list(&mut prm.value, edit, false),
            VIRTUAL_MASTER => edit_location_list(&mut prm.value, edit, true),
            AXIS_MAPPINGS => edit_mappings(&mut prm.value, edit),
            _ => false,
        })
        .collect();
    let mut gone = emptied.into_iter();
    out.0.retain(|_| !gone.next().unwrap_or(false));
    (out != *params).then_some(out)
}

/// Applies `edit` to the parameters of the font, every master and every instance.
fn rewrite_params(font: &mut Font, edit: &ParamEdit) {
    if let Some(next) = edit_params(&font.info.params, edit) {
        Arc::make_mut(&mut font.info).params = next;
    }
    let masters: Vec<(usize, ModelParams)> =
        font.masters.iter().enumerate().filter_map(|(i, m)| edit_params(&m.params, edit).map(|p| (i, p))).collect();
    if !masters.is_empty() {
        let list = Arc::make_mut(&mut font.masters);
        for (i, prm) in masters {
            if let Some(m) = list.get_mut(i) {
                m.params = prm;
            }
        }
    }
    let instances: Vec<(usize, ModelParams)> =
        font.instances.iter().enumerate().filter_map(|(i, inst)| edit_params(&inst.params, edit).map(|p| (i, p))).collect();
    if !instances.is_empty() {
        let list = Arc::make_mut(&mut font.instances);
        for (i, prm) in instances {
            if let Some(inst) = list.get_mut(i) {
                inst.params = prm;
            }
        }
    }
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn list(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("font.axes.list", v)?;
    let font = s.font()?;
    Ok(json!({"axes": font.axes.iter().enumerate().map(|(i, a)| axis_json(i, a)).collect::<Vec<_>>()}))
}

fn list_predefined(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("font.axes.listPredefined", v)?;
    s.font()?;
    Ok(json!({"axes": PREDEFINED.iter().map(|(n, t, r)| json!({"name": n, "tag": t, "registered": r})).collect::<Vec<_>>()}))
}

fn add(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.axes.add", v)?;
    let font = s.font()?;
    if font.axes.len() >= MAX_AXIS_VALUES {
        return Err(p.bad(format!("a font has at most {MAX_AXIS_VALUES} axes")));
    }
    let name = p.opt_str("name")?.map(str::trim);
    let tag = p.opt_str("tag")?;
    let hidden = p.bool_or("hidden", false)?;
    if let Some(n) = name {
        check_name(&p, n)?;
    }
    if let Some(t) = tag {
        check_tag(&p, t)?;
    }
    let (name, tag) = match (name, tag) {
        (Some(n), Some(t)) => (n.to_string(), t.to_string()),
        (Some(n), None) => (n.to_string(), predefined_tag(n).map_or_else(|| free_private_tag(font), str::to_string)),
        (None, Some(t)) => (predefined_name(t).filter(|n| !name_taken(font, n)).map_or_else(|| free_axis_name(font), str::to_string), t.to_string()),
        (None, None) if !tag_taken(font, "wght") && !name_taken(font, "Weight") => ("Weight".to_string(), "wght".to_string()),
        (None, None) => (free_axis_name(font), free_private_tag(font)),
    };
    check_unique(&p, font, &name, &tag, None)?;
    let warnings = tag_warnings(&tag);
    let index = font.axes.len();
    let axis = Axis { name: name.clone(), tag, hidden, ..Default::default() };
    let out = axis_json(index, &axis);
    let remap = Remap((0..index).map(Some).chain(std::iter::once(None)).collect());
    s.edit("Add Axis", |font| {
        apply_remap(font, &remap, false);
        Arc::make_mut(&mut font.axes).push(axis);
        rewrite_params(font, &ParamEdit::Add { name: &name });
        Ok(())
    })?;
    Ok(json!({"axis": out, "warnings": warnings}))
}

fn set(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.axes.set", v)?;
    let font = s.font()?;
    let i = axis_index(&p, font)?;
    let old = font.axes.get(i).ok_or_else(|| p.bad(format!("there is no axis #{i}")))?;
    let name = p.opt_str("name")?.map(str::trim);
    let tag = p.opt_str("tag")?;
    let hidden = match p.get("hidden") {
        Some(_) => Some(p.bool_or("hidden", false)?),
        None => None,
    };
    if name.is_none() && tag.is_none() && hidden.is_none() {
        return Err(p.bad("give at least one of name, tag, hidden"));
    }
    if let Some(n) = name {
        check_name(&p, n)?;
    }
    if let Some(t) = tag {
        check_tag(&p, t)?;
    }
    let new_name = name.map_or_else(|| old.name.clone(), str::to_string);
    // A predefined name sets its tag (`fontInfo.axes.predefinedNameSetsTag`); a free-form name
    // leaves the tag as it was.
    let new_tag = match (tag, name.and_then(predefined_tag)) {
        (Some(t), _) => t.to_string(),
        (None, Some(t)) => t.to_string(),
        (None, None) => old.tag.clone(),
    };
    check_unique(&p, font, &new_name, &new_tag, Some(i))?;
    let warnings = if new_tag == old.tag { Vec::new() } else { tag_warnings(&new_tag) };
    let (old_name, old_tag) = (old.name.clone(), old.tag.clone());
    let new_hidden = hidden.unwrap_or(old.hidden);
    let next = Axis { name: new_name.clone(), tag: new_tag.clone(), hidden: new_hidden, ..old.clone() };
    let out = axis_json(i, &next);
    s.edit("Edit Axis", |font| {
        if let Some(a) = Arc::make_mut(&mut font.axes).get_mut(i) {
            *a = next;
        }
        rewrite_params(font, &ParamEdit::Rename { from_name: &old_name, to_name: &new_name, from_tag: &old_tag, to_tag: &new_tag });
        Ok(())
    })?;
    Ok(json!({"axis": out, "warnings": warnings}))
}

fn remove(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.axes.remove", v)?;
    let font = s.font()?;
    let i = axis_index(&p, font)?;
    let axis = font.axes.get(i).cloned().ok_or_else(|| p.bad(format!("there is no axis #{i}")))?;
    let remap = Remap((0..font.axes.len()).filter(|&k| k != i).map(Some).collect());
    // Intermediate layers whose only own coordinate was on this axis end up at their master's
    // location.
    let collapsed: Vec<String> = font
        .glyphs
        .iter()
        .filter(|g| {
            g.layers.iter().any(|l| {
                let c = &l.info.attrs.coordinates;
                c.iter().any(Option::is_some) && remap.coordinates(c).iter().all(Option::is_none)
            })
        })
        .map(|g| g.name.clone())
        .collect();
    s.edit("Remove Axis", |font| {
        apply_remap(font, &remap, true);
        let axes = Arc::make_mut(&mut font.axes);
        if i < axes.len() {
            axes.remove(i);
        }
        rewrite_params(font, &ParamEdit::Remove { name: &axis.name, tag: &axis.tag });
        Ok(())
    })?;
    let mut warnings = shared_location_warnings(s.font()?);
    if !collapsed.is_empty() {
        let mut names: Vec<String> = collapsed.iter().take(MAX_NAMED_IN_WARNING).map(|n| format!("`{n}`")).collect();
        if collapsed.len() > MAX_NAMED_IN_WARNING {
            names.push(format!("and {} more", collapsed.len() - MAX_NAMED_IN_WARNING));
        }
        warnings.push(format!("intermediate layers of {} now sit at their masters' locations", names.join(", ")));
    }
    Ok(json!({"removed": axis_json(i, &axis), "warnings": warnings}))
}

fn reorder(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.axes.reorder", v)?;
    let font = s.font()?;
    let order = p.strings("order")?;
    let n = font.axes.len();
    if order.len() != n {
        return Err(p.bad(format!("`order` must list each of the font's {n} axis tags once; it has {} entries", order.len())));
    }
    let mut perm: Vec<usize> = Vec::with_capacity(n);
    for t in &order {
        let i = font.axes.iter().position(|a| a.tag == *t).ok_or_else(|| p.bad(format!("the font has no axis `{t}`")))?;
        if perm.contains(&i) {
            return Err(p.bad(format!("`order` names axis `{t}` twice")));
        }
        perm.push(i);
    }
    let tags: Vec<String> = perm.iter().filter_map(|&i| font.axes.get(i).map(|a| a.tag.clone())).collect();
    if perm.iter().enumerate().all(|(k, &i)| k == i) {
        return Ok(json!({"axes": tags}));
    }
    let names: Vec<String> = perm.iter().filter_map(|&i| font.axes.get(i).map(|a| a.name.clone())).collect();
    let remap = Remap(perm.iter().map(|&i| Some(i)).collect());
    s.edit("Reorder Axes", |font| {
        let next: Vec<Axis> = perm.iter().filter_map(|&i| font.axes.get(i).cloned()).collect();
        apply_remap(font, &remap, true);
        font.axes = Arc::new(next);
        rewrite_params(font, &ParamEdit::Reorder { names: &names, tags: &tags });
        Ok(())
    })?;
    Ok(json!({"axes": tags}))
}

fn set_master_location(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.master.setAxisLocation", v)?;
    let font = s.font()?;
    let raw = p.get("masters").ok_or_else(|| p.bad("`masters` is required"))?;
    let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
    let indices = target::masters(font, &refs).map_err(|e| p.bad(e.to_string()))?;
    let values = read_location(&p, font)?;
    let n = font.axes.len();
    let ids: Vec<String> = indices.iter().filter_map(|&i| font.masters.get(i).map(|m| m.id.clone())).collect();
    s.edit("Set Master Axes Coordinates", |font| {
        let masters = Arc::make_mut(&mut font.masters);
        for &i in &indices {
            if let Some(m) = masters.get_mut(i) {
                m.location = placed(&m.location, n, &values);
            }
        }
        Ok(())
    })?;
    let warnings = shared_location_warnings(s.font()?);
    Ok(json!({"masters": ids, "warnings": warnings}))
}

fn set_instance_location(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.setAxisLocation", v)?;
    let font = s.font()?;
    let raw = p.get("instances").ok_or_else(|| p.bad("`instances` is required"))?;
    let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
    let indices = target::instances(font, &refs).map_err(|e| p.bad(e.to_string()))?;
    let values = read_location(&p, font)?;
    let n = font.axes.len();
    s.edit("Set Instance Axes Coordinates", |font| {
        let instances = Arc::make_mut(&mut font.instances);
        for &i in &indices {
            if let Some(inst) = instances.get_mut(i) {
                inst.location = placed(&inst.location, n, &values);
            }
        }
        Ok(())
    })?;
    let font = s.font()?;
    let warnings: Vec<String> = indices
        .iter()
        .filter_map(|&i| font.instances.get(i))
        .filter(|inst| !is_setting(inst))
        .filter_map(|inst| outside_warning(font, inst))
        .collect();
    Ok(json!({"instances": indices, "warnings": warnings}))
}

/// A warning naming the axes on which `inst` lies beyond every master: the reference
/// extrapolates such an instance.
fn outside_warning(font: &Font, inst: &Instance) -> Option<String> {
    let outside: Vec<&str> = font
        .axes
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let v = design_value(&inst.location, *i);
            let (lo, hi) = font
                .masters
                .iter()
                .map(|m| design_value(&m.location, *i))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
            lo <= hi && (v < lo || v > hi)
        })
        .map(|(_, a)| a.tag.as_str())
        .collect();
    (!outside.is_empty()).then(|| format!("instance `{}` lies beyond the masters on {}; it is extrapolated", inst.name, outside.join(", ")))
}
