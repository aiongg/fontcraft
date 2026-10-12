//! Master commands: `font.master.list`, `.get`, `.set`, `.add`, `.duplicate`, `.remove`, `.reorder` and `.setIcon`, addressed through `fontcraft_fontinfo::target`.
//!
//! A master's data lives in three places: the [`Master`] itself, the layers tied to it in every
//! glyph (its master layer and the special layers whose `master` is its id), and its entries in
//! the kerning tables. Adding, duplicating and removing a master changes all three in one undo
//! step. A locked glyph refuses a change to its layers (`Session::edit`), so a font with a locked
//! glyph refuses adding, duplicating and removing masters until it is unlocked.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use fontcraft_fontinfo::{MAX_MASTERS, MAX_METRIC_DEFS, MAX_STEM_DEFS, target, value};
use fontcraft_model::{Font, Glyph, Layer, MAX_COORDINATE, MAX_NAME_LEN, Master, MetricDef, MetricValue, metric};
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// The 25 built-in master icons: a lowercase n at five weights and five widths. The normal width
/// is spelled by the weight alone, the others `Weight_Width`, as source files store them.
pub const MASTER_ICONS: [&str; 25] = [
    "Light_Condensed",
    "Light_SemiCondensed",
    "Light",
    "Light_SemiExtended",
    "Light_Extended",
    "Regular_Condensed",
    "Regular_SemiCondensed",
    "Regular",
    "Regular_SemiExtended",
    "Regular_Extended",
    "Medium_Condensed",
    "Medium_SemiCondensed",
    "Medium",
    "Medium_SemiExtended",
    "Medium_Extended",
    "SemiBold_Condensed",
    "SemiBold_SemiCondensed",
    "SemiBold",
    "SemiBold_SemiExtended",
    "SemiBold_Extended",
    "Bold_Condensed",
    "Bold_SemiCondensed",
    "Bold",
    "Bold_SemiExtended",
    "Bold_Extended",
];

/// The name a new master gets, and the icon a master without one is drawn with.
const DEFAULT_NAME: &str = "Regular";

/// Custom parameters whose value names a master; removing that master leaves them pointing at
/// nothing, so the removal reports them.
const PARAMS_NAMING_A_MASTER: [&str; 3] = ["Link Metrics With Master", "Get Hints From Master", "Variable Font Origin"];

/// Overshoot of the default alignment zones at 1000 units per em.
const DEFAULT_ZONE: f64 = 16.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.master.list",
            "List Masters",
            [],
            None,
            "{} the masters in list order → {masters: [{index, id, name, icon, axes: {tag: number}}]}",
            has_doc,
            list
        ),
        cmd!(
            query "font.master.get",
            "Get Master",
            [],
            None,
            "{master?: id|name|index, masters?: [id|name|index]} every property of one master, or of several at once: with several, a value they do not share is null → {ids, id, index, name, icon, visible, axes: {tag: number}, metrics: {metric: {position, zone}}, stems: {stem: number}, numbers: {name: number}, customParameters: [{name, value, disabled}]}",
            has_doc,
            get
        ),
        cmd!(
            "font.master.set",
            "Edit Master",
            [],
            None,
            "{masters: [id|name|index], name?: string, icon?: string, axes?: {tag: number}, metrics?: {metric: {position?: number, zone?: number}}, stems?: {stem: number}, numbers?: {name: number}} write the given properties on every master named, in one step. A metric is named by its name, else its kind (`ascender`, `x-height`…); `icon` is a built-in icon name or a glyph of the font → {masters: [id]}",
            has_doc,
            set
        ),
        cmd!(
            "font.master.add",
            "Add Master",
            [],
            None,
            "{name?: string = \"Regular\", axes?: {tag: number}} add a master at the end of the list, at the first master's axis coordinates unless `axes` gives others, with the default metric values and an empty layer in every glyph → {master: {index, id, name}}",
            has_doc,
            add
        ),
        cmd!(
            "font.master.duplicate",
            "Duplicate Selected",
            [],
            None,
            "{masters: [id|name|index]} copy each master right after itself, with the same name, its layers in every glyph (backgrounds and special layers included) and its kerning → {masters: [{index, id, of}]}",
            has_doc,
            duplicate
        ),
        cmd!(
            "font.master.remove",
            "Remove Master",
            [],
            None,
            "{masters: [id|name|index]} delete the masters with their layers in every glyph and their kerning; the last master is refused. Parameters that name a removed master are kept and listed → {removed: [id], warnings: [string]}",
            has_doc,
            remove
        ),
        cmd!(
            "font.master.reorder",
            "Reorder Masters",
            [],
            None,
            "{master: id|name|index, toIndex: integer} move a master to another place in the list → {master: id, index}",
            has_doc,
            reorder
        ),
        cmd!(
            "font.master.setIcon",
            "Set Master Icon",
            [],
            None,
            "{masters: [id|name|index], icon?: string, glyph?: glyphName} give the masters one of the 25 built-in icons (`Light`, `Bold_Condensed`… : Light, Regular, Medium, SemiBold or Bold, alone or with `_Condensed`, `_SemiCondensed`, `_SemiExtended` or `_Extended`), or a glyph of the font as their icon; give one of the two → {masters: [id], icon}",
            has_doc,
            set_icon
        ),
    ]
}

// ── Addressing ──────────────────────────────────────────────────────────────────────────────────

/// The masters `masters` (a list or a single target) or `master` names, each once.
fn targets(p: &Params, font: &Font, single_key: Option<&str>) -> Result<Vec<usize>> {
    let raw = match (single_key.and_then(|k| p.get(k)), p.get("masters")) {
        (Some(_), Some(_)) => return Err(p.bad("give `master` or `masters`, not both")),
        (Some(v), None) | (None, Some(v)) => v,
        (None, None) => return Err(p.bad("`masters` is required")),
    };
    if let Value::Array(a) = raw {
        if a.is_empty() {
            return Err(p.bad("`masters` is empty"));
        }
        if a.len() > MAX_MASTERS {
            return Err(p.bad(format!("`masters` holds {} targets; a font has at most {MAX_MASTERS} masters", a.len())));
        }
    }
    let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
    target::masters(font, &refs).map_err(|e| p.bad(e.to_string()))
}

/// The one master `key` names.
fn one_target(p: &Params, font: &Font, key: &str) -> Result<usize> {
    let raw = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    let r = target::parse(raw).map_err(|e| p.bad(e.to_string()))?;
    target::master(font, &r).map_err(|e| p.bad(e.to_string()))
}

fn master_at<'f>(p: &Params, font: &'f Font, i: usize) -> Result<&'f Master> {
    font.masters.get(i).ok_or_else(|| p.bad(format!("there is no master #{i}")))
}

// ── Reading ─────────────────────────────────────────────────────────────────────────────────────

/// How a metric definition is named in `metrics` objects: its name, else its kind, with the
/// glyph filter in brackets when it has one.
fn metric_key(def: &MetricDef) -> String {
    let base = def.name.as_deref().unwrap_or(&def.kind);
    match &def.filter {
        Some(f) => format!("{base} [{f}]"),
        None => base.to_string(),
    }
}

/// How a stem definition is named: its name, else `#index` (format version 2 stems have none).
fn stem_key(name: &str, i: usize) -> String {
    if name.is_empty() { format!("#{i}") } else { name.to_string() }
}

fn axes_json(font: &Font, m: &Master) -> Value {
    let mut out = Map::new();
    for (i, a) in font.axes.iter().enumerate() {
        out.insert(a.tag.clone(), json!(m.location.get(i).copied().unwrap_or(0.0)));
    }
    Value::Object(out)
}

fn summary_json(font: &Font, i: usize, m: &Master) -> Value {
    json!({"index": i, "id": m.id, "name": m.name, "icon": m.icon, "axes": axes_json(font, m)})
}

fn master_json(font: &Font, i: usize, m: &Master) -> Value {
    let mut metrics = Map::new();
    for (k, def) in font.metrics.iter().enumerate() {
        let v = m.metric_values.get(k).copied().unwrap_or_default();
        metrics.entry(metric_key(def)).or_insert_with(|| json!({"position": v.pos, "zone": v.over}));
    }
    let mut stems = Map::new();
    for (k, def) in font.stems.iter().enumerate() {
        stems.entry(stem_key(&def.name, k)).or_insert_with(|| json!(m.stem_values.get(k).copied().unwrap_or(0.0)));
    }
    let mut numbers = Map::new();
    for (k, def) in font.numbers.iter().enumerate() {
        numbers.entry(def.name.clone()).or_insert_with(|| json!(m.number_values.get(k).copied().unwrap_or(0.0)));
    }
    let params: Vec<Value> =
        m.params.0.iter().map(|prm| json!({"name": prm.name, "value": value::to_json(&prm.value), "disabled": prm.disabled})).collect();
    json!({
        "id": m.id,
        "index": i,
        "name": m.name,
        "icon": m.icon,
        "visible": m.visible,
        "axes": axes_json(font, m),
        "metrics": metrics,
        "stems": stems,
        "numbers": numbers,
        "customParameters": params,
    })
}

/// The value two masters share: equal values stay, objects are merged key by key, anything that
/// differs (or is missing on one side) becomes null.
fn shared(a: &Value, b: &Value) -> Value {
    if a == b {
        return a.clone();
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            let mut out = Map::new();
            for k in keys {
                let v = match (x.get(k), y.get(k)) {
                    (Some(x), Some(y)) => shared(x, y),
                    _ => Value::Null,
                };
                out.insert(k.clone(), v);
            }
            Value::Object(out)
        }
        _ => Value::Null,
    }
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    Params::new("font.master.list", p)?;
    let font = s.font()?;
    let masters: Vec<Value> = font.masters.iter().enumerate().map(|(i, m)| summary_json(font, i, m)).collect();
    Ok(json!({"masters": masters}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.master.get", p)?;
    let font = s.font()?;
    let indices = targets(&p, font, Some("master"))?;
    let mut ids = Vec::with_capacity(indices.len());
    let mut merged: Option<Value> = None;
    for &i in &indices {
        let m = master_at(&p, font, i)?;
        ids.push(m.id.clone());
        let v = master_json(font, i, m);
        merged = Some(match merged {
            None => v,
            Some(prev) => shared(&prev, &v),
        });
    }
    let mut out = merged.ok_or_else(|| p.bad("`masters` is empty"))?;
    if let Some(o) = out.as_object_mut() {
        o.insert("ids".into(), json!(ids));
    }
    Ok(out)
}

// ── Checks on new values ────────────────────────────────────────────────────────────────────────

fn check_name(p: &Params, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(p.bad("a master name cannot be empty"));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("a master name is at most {MAX_NAME_LEN} bytes long")));
    }
    if name.chars().any(char::is_control) {
        return Err(p.bad("a master name cannot hold control characters"));
    }
    Ok(())
}

/// A built-in icon name or the name of a glyph of the font.
fn check_icon(p: &Params, font: &Font, icon: &str) -> Result<()> {
    if MASTER_ICONS.contains(&icon) || font.glyph(icon).is_some() {
        Ok(())
    } else {
        Err(p.bad(format!("`{icon}` is neither a built-in master icon nor a glyph of the font")))
    }
}

fn finite_coordinate(p: &Params, what: &str, v: &Value) -> Result<f64> {
    match v.as_f64() {
        Some(n) if n.is_finite() && n.abs() <= MAX_COORDINATE => Ok(n),
        _ => Err(p.bad(format!("{what} must be a finite number within ±{MAX_COORDINATE}"))),
    }
}

/// `{tag: number}` read against the font's axes: (axis index, value).
fn read_axes(p: &Params, font: &Font) -> Result<Option<Vec<(usize, f64)>>> {
    let Some(obj) = p.opt_object("axes")? else { return Ok(None) };
    if obj.len() > font.axes.len() {
        return Err(p.bad(format!("`axes` names {} axes; the font has {}", obj.len(), font.axes.len())));
    }
    let mut out = Vec::with_capacity(obj.len());
    for (tag, v) in obj {
        let i = font.axes.iter().position(|a| a.tag == *tag).ok_or_else(|| p.bad(format!("the font has no axis `{tag}`")))?;
        out.push((i, finite_coordinate(p, &format!("axis `{tag}`"), v)?));
    }
    Ok(Some(out))
}

/// One metric's new position and zone, by definition index.
struct MetricEdit {
    index: usize,
    position: Option<f64>,
    zone: Option<f64>,
}

fn read_metrics(p: &Params, font: &Font) -> Result<Option<Vec<MetricEdit>>> {
    let Some(obj) = p.opt_object("metrics")? else { return Ok(None) };
    if obj.len() > MAX_METRIC_DEFS {
        return Err(p.bad(format!("`metrics` names {} metrics; a font has at most {MAX_METRIC_DEFS}", obj.len())));
    }
    let mut out = Vec::with_capacity(obj.len());
    for (key, v) in obj {
        let index = font.metrics.iter().position(|d| metric_key(d) == *key).ok_or_else(|| p.bad(format!("the font has no metric `{key}`")))?;
        let Value::Object(fields) = v else {
            return Err(p.bad(format!("metric `{key}` takes {{position?, zone?}}")));
        };
        if let Some(other) = fields.keys().find(|k| *k != "position" && *k != "zone") {
            return Err(p.bad(format!("metric `{key}` has no field `{other}`; it takes position and zone")));
        }
        let position = fields.get("position").map(|v| finite_coordinate(p, &format!("the position of `{key}`"), v)).transpose()?;
        let zone = fields.get("zone").map(|v| finite_coordinate(p, &format!("the zone of `{key}`"), v)).transpose()?;
        if position.is_none() && zone.is_none() {
            return Err(p.bad(format!("metric `{key}` needs a position or a zone")));
        }
        out.push(MetricEdit { index, position, zone });
    }
    Ok(Some(out))
}

/// `{name: number}` matched against definition names: (definition index, value).
fn read_named(p: &Params, key: &str, names: &[String], limit: usize, what: &str) -> Result<Option<Vec<(usize, f64)>>> {
    let Some(obj) = p.opt_object(key)? else { return Ok(None) };
    if obj.len() > limit {
        return Err(p.bad(format!("`{key}` names {} {what}s; a font has at most {limit}", obj.len())));
    }
    let mut out = Vec::with_capacity(obj.len());
    for (name, v) in obj {
        let i = names.iter().position(|n| n == name).ok_or_else(|| p.bad(format!("the font has no {what} `{name}`")))?;
        out.push((i, finite_coordinate(p, &format!("{what} `{name}`"), v)?));
    }
    Ok(Some(out))
}

/// Sets `values[i]`, growing the list with zeros to reach it.
fn put(values: &mut Vec<f64>, i: usize, v: f64) {
    if values.len() <= i {
        values.resize(i + 1, 0.0);
    }
    if let Some(slot) = values.get_mut(i) {
        *slot = v;
    }
}

// ── Editing ─────────────────────────────────────────────────────────────────────────────────────

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.master.set", p)?;
    let font = s.font()?;
    let indices = targets(&p, font, None)?;
    let name = p.opt_str("name")?;
    if let Some(n) = name {
        check_name(&p, n)?;
    }
    let icon = p.opt_str("icon")?;
    if let Some(i) = icon {
        check_icon(&p, font, i)?;
    }
    let axes = read_axes(&p, font)?;
    let metrics = read_metrics(&p, font)?;
    let stem_names: Vec<String> = font.stems.iter().enumerate().map(|(i, d)| stem_key(&d.name, i)).collect();
    let stems = read_named(&p, "stems", &stem_names, MAX_STEM_DEFS, "stem")?;
    let number_names: Vec<String> = font.numbers.iter().map(|d| d.name.clone()).collect();
    let numbers = read_named(&p, "numbers", &number_names, MAX_METRIC_DEFS, "number")?;
    if name.is_none() && icon.is_none() && axes.is_none() && metrics.is_none() && stems.is_none() && numbers.is_none() {
        return Err(p.bad("give at least one of name, icon, axes, metrics, stems, numbers"));
    }
    let axis_count = font.axes.len();
    let ids: Vec<String> = indices.iter().filter_map(|i| font.masters.get(*i).map(|m| m.id.clone())).collect();
    s.edit("Edit Master", |font| {
        let masters = Arc::make_mut(&mut font.masters);
        for &i in &indices {
            let Some(m) = masters.get_mut(i) else { continue };
            if let Some(n) = name {
                m.name = n.to_string();
            }
            if let Some(ic) = icon {
                m.icon = Some(ic.to_string());
            }
            if let Some(axes) = &axes {
                if m.location.len() < axis_count {
                    m.location.resize(axis_count, 0.0);
                }
                for &(a, v) in axes {
                    put(&mut m.location, a, v);
                }
            }
            if let Some(metrics) = &metrics {
                for e in metrics {
                    if m.metric_values.len() <= e.index {
                        m.metric_values.resize(e.index + 1, MetricValue::default());
                    }
                    if let Some(slot) = m.metric_values.get_mut(e.index) {
                        if let Some(pos) = e.position {
                            slot.pos = pos;
                        }
                        if let Some(zone) = e.zone {
                            slot.over = zone;
                        }
                    }
                }
            }
            for &(k, v) in stems.iter().flatten() {
                put(&mut m.stem_values, k, v);
            }
            for &(k, v) in numbers.iter().flatten() {
                put(&mut m.number_values, k, v);
            }
        }
        Ok(())
    })?;
    Ok(json!({"masters": ids}))
}

fn set_icon(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.master.setIcon", p)?;
    let font = s.font()?;
    let indices = targets(&p, font, None)?;
    let icon = match (p.opt_str("icon")?, p.opt_str("glyph")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `icon` or `glyph`, not both")),
        (None, None) => return Err(p.bad("give `icon` (a built-in icon) or `glyph` (a glyph of the font)")),
        (Some(icon), None) => {
            if !MASTER_ICONS.contains(&icon) {
                return Err(p.bad(format!("`{icon}` is not a built-in master icon; the icons are {}", MASTER_ICONS.join(", "))));
            }
            icon
        }
        (None, Some(glyph)) => {
            if font.glyph(glyph).is_none() {
                return Err(p.bad(format!("there is no glyph `{glyph}`")));
            }
            glyph
        }
    };
    let ids: Vec<String> = indices.iter().filter_map(|i| font.masters.get(*i).map(|m| m.id.clone())).collect();
    s.edit("Set Master Icon", |font| {
        let masters = Arc::make_mut(&mut font.masters);
        for &i in &indices {
            if let Some(m) = masters.get_mut(i) {
                m.icon = Some(icon.to_string());
            }
        }
        Ok(())
    })?;
    Ok(json!({"masters": ids, "icon": icon}))
}

/// Every id a new master or layer must avoid: the masters' ids and every layer id of every glyph.
fn taken_ids(font: &Font) -> HashSet<String> {
    let mut out: HashSet<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    for g in font.glyphs.iter() {
        out.extend(g.layers.iter().map(|l| l.id.clone()));
    }
    out
}

/// The first free id of the form `m01`, `m02`…, the form a new font's master id takes.
fn new_master_id(p: &Params, taken: &HashSet<String>) -> Result<String> {
    (1..=10_000u32).map(|n| format!("m{n:02}")).find(|id| !taken.contains(id)).ok_or_else(|| p.bad("could not find a free master id"))
}

/// A layer id in the 8-4-4-4-12 hex form layer ids usually take, new in `glyph`. It depends only
/// on `seed`, so replaying a journal gives the same id.
fn new_layer_id(glyph: &Glyph, seed: &str) -> Option<String> {
    fn fnv(basis: u64, text: &str) -> u64 {
        text.bytes().fold(basis, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3))
    }
    (0u32..10_000).find_map(|n| {
        let text = format!("{seed}#{n}");
        let (a, b) = (fnv(0xCBF2_9CE4_8422_2325, &text), fnv(0x6C62_272E_07BB_0142, &text));
        let id = format!("{:08X}-{:04X}-{:04X}-{:04X}-{:012X}", a >> 32, (a >> 16) & 0xFFFF, a & 0xFFFF, b >> 48, b & 0xFFFF_FFFF_FFFF);
        (!glyph.layers.iter().any(|l| l.id == id)).then_some(id)
    })
}

/// A new master's value for a metric kind: the default metrics at 1000 units per em (ascender
/// 800, cap height 700, x-height 500, descender −200, zones 16 and −16), scaled to the font's
/// units per em and rounded.
fn default_metric(font: &Font, def: &MetricDef) -> MetricValue {
    let scale = f64::from(font.units_per_em) / 1000.0;
    let (pos, over) = match def.kind.as_str() {
        metric::ASCENDER => (800.0, DEFAULT_ZONE),
        metric::CAP_HEIGHT => (700.0, DEFAULT_ZONE),
        metric::X_HEIGHT => (500.0, DEFAULT_ZONE),
        metric::BASELINE => (0.0, -DEFAULT_ZONE),
        metric::DESCENDER => (-200.0, -DEFAULT_ZONE),
        _ => (0.0, 0.0),
    };
    MetricValue { pos: (pos * scale).round(), over: (over * scale).round() }
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.master.add", p)?;
    let font = s.font()?;
    if font.masters.len() >= MAX_MASTERS {
        return Err(p.bad(format!("a font has at most {MAX_MASTERS} masters")));
    }
    let name = p.opt_str("name")?.unwrap_or(DEFAULT_NAME);
    check_name(&p, name)?;
    let axes = read_axes(&p, font)?;
    let id = new_master_id(&p, &taken_ids(font))?;
    let first = font.default_master()?;
    let mut location: Vec<f64> = (0..font.axes.len()).map(|i| first.location.get(i).copied().unwrap_or(0.0)).collect();
    for &(a, v) in axes.iter().flatten() {
        put(&mut location, a, v);
    }
    let master = Master {
        id: id.clone(),
        name: name.to_string(),
        location,
        metric_values: font.metrics.iter().map(|d| default_metric(font, d)).collect(),
        stem_values: vec![0.0; font.stems.len()],
        number_values: vec![0.0; font.numbers.len()],
        ..Default::default()
    };
    let first_id = first.id.clone();
    let index = font.masters.len();
    s.edit("Add Master", |font| {
        Arc::make_mut(&mut font.masters).push(master);
        for gi in 0..font.glyphs.len() {
            let Some(mut g) = font.glyphs.get_mut_at(gi) else { continue };
            let width = g.layers.iter().find(|l| l.id == first_id).map_or(0.0, |l| l.width);
            g.layers.push(Layer::new(id.as_str(), width));
        }
        Ok(())
    })?;
    Ok(json!({"master": {"index": index, "id": id, "name": name}}))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.master.duplicate", p)?;
    let font = s.font()?;
    let indices = targets(&p, font, None)?;
    if font.masters.len() + indices.len() > MAX_MASTERS {
        return Err(p.bad(format!("a font has at most {MAX_MASTERS} masters")));
    }
    let mut taken = taken_ids(font);
    // (original id, copy id), in the order given.
    let mut pairs = Vec::with_capacity(indices.len());
    for &i in &indices {
        let original = master_at(&p, font, i)?.id.clone();
        let copy = new_master_id(&p, &taken)?;
        taken.insert(copy.clone());
        pairs.push((original, copy));
    }
    let made = s.edit("Duplicate Masters", |font| {
        let mut made = Vec::with_capacity(pairs.len());
        for (original, copy) in &pairs {
            let masters = Arc::make_mut(&mut font.masters);
            let Some(at) = masters.iter().position(|m| m.id == *original) else { continue };
            let Some(source) = masters.get(at) else { continue };
            let mut m = source.clone();
            m.id = copy.clone();
            // The copy is the document's own master: with the mark kept, saving would drop it.
            crate::linked::unmark(&mut m.user_data);
            masters.insert(at + 1, m);
            made.push(json!({"index": at + 1, "id": copy, "of": original}));
            copy_layers(font, original, copy)?;
            copy_kerning(font, original, copy);
        }
        // Later copies can shift earlier ones down the list: report where each one ended up.
        for v in &mut made {
            let at = v.get("id").and_then(Value::as_str).and_then(|id| font.masters.iter().position(|m| m.id == id));
            if let (Some(i), Some(o)) = (at, v.as_object_mut()) {
                o.insert("index".into(), json!(i));
            }
        }
        Ok(made)
    })?;
    Ok(json!({"masters": made}))
}

/// Copies every layer tied to `original` in every glyph and ties the copies to `copy`: the master
/// layer becomes the copy's master layer, the special layers get new ids.
fn copy_layers(font: &mut Font, original: &str, copy: &str) -> Result<()> {
    for gi in 0..font.glyphs.len() {
        let has = font.glyphs.as_arcs().get(gi).is_some_and(|g| g.layers.iter().any(|l| l.master == original));
        if !has {
            continue;
        }
        let Some(mut g) = font.glyphs.get_mut_at(gi) else { continue };
        let tied: Vec<Layer> = g.layers.iter().filter(|l| l.master == original).cloned().collect();
        for mut l in tied {
            if l.is_master_layer() {
                l.id = copy.to_string();
            } else {
                let seed = format!("{}|{}|{copy}", g.name, l.id);
                l.id = new_layer_id(&g, &seed).ok_or_else(|| EngineError::BadParams {
                    command: "font.master.duplicate".into(),
                    message: format!("could not find a free layer id in glyph `{}`", g.name),
                })?;
            }
            l.master = copy.to_string();
            g.layers.push(l);
        }
    }
    Ok(())
}

/// Copies `original`'s kerning, in every direction and every context, to `copy`.
fn copy_kerning(font: &mut Font, original: &str, copy: &str) {
    let k = &font.kerning;
    let has = [&k.ltr, &k.rtl, &k.vertical].iter().any(|t| t.0.iter().any(|m| m.master == original))
        || k.context.iter().any(|c| c.values.iter().any(|(m, _)| m == original));
    if !has {
        return;
    }
    let k = Arc::make_mut(&mut font.kerning);
    for table in [&mut k.ltr, &mut k.rtl, &mut k.vertical] {
        if let Some(entry) = table.0.iter().find(|m| m.master == original) {
            let mut e = entry.clone();
            e.master = copy.to_string();
            table.0.push(e);
        }
    }
    for c in &mut k.context {
        if let Some(v) = c.values.iter().find(|(m, _)| m == original).map(|(_, v)| *v) {
            c.values.push((copy.to_string(), v));
        }
    }
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.master.remove", p)?;
    let font = s.font()?;
    let indices = targets(&p, font, None)?;
    if indices.len() >= font.masters.len() {
        return Err(p.bad("a font needs at least one master: the last master cannot be removed"));
    }
    let gone: Vec<&Master> = indices.iter().filter_map(|i| font.masters.get(*i)).collect();
    let ids: BTreeSet<String> = gone.iter().map(|m| m.id.clone()).collect();
    let warnings = naming_params(font, &gone);
    let ordered: Vec<String> = gone.iter().map(|m| m.id.clone()).collect();
    s.edit("Remove Master", |font| {
        Arc::make_mut(&mut font.masters).retain(|m| !ids.contains(&m.id));
        for gi in 0..font.glyphs.len() {
            let has = font.glyphs.as_arcs().get(gi).is_some_and(|g| g.layers.iter().any(|l| ids.contains(&l.master)));
            if !has {
                continue;
            }
            if let Some(mut g) = font.glyphs.get_mut_at(gi) {
                g.layers.retain(|l| !ids.contains(&l.master));
            }
        }
        let k = &font.kerning;
        let kerned = [&k.ltr, &k.rtl, &k.vertical].iter().any(|t| t.0.iter().any(|m| ids.contains(&m.master)))
            || k.context.iter().any(|c| c.values.iter().any(|(m, _)| ids.contains(m)));
        if kerned {
            let k = Arc::make_mut(&mut font.kerning);
            for table in [&mut k.ltr, &mut k.rtl, &mut k.vertical] {
                table.0.retain(|m| !ids.contains(&m.master));
            }
            for c in &mut k.context {
                c.values.retain(|(m, _)| !ids.contains(m));
            }
            k.context.retain(|c| !c.values.is_empty());
        }
        if font.instances.iter().any(|i| i.interpolation.iter().any(|(m, _)| ids.contains(m))) {
            for inst in Arc::make_mut(&mut font.instances).iter_mut() {
                inst.interpolation.retain(|(m, _)| !ids.contains(m));
            }
        }
        Ok(())
    })?;
    Ok(json!({"removed": ordered, "warnings": warnings}))
}

/// One warning for each parameter, at any level, whose value names one of `gone` by id or name.
fn naming_params(font: &Font, gone: &[&Master]) -> Vec<String> {
    let names_one = |prm: &fontcraft_model::Param| -> Option<String> {
        if !PARAMS_NAMING_A_MASTER.contains(&prm.name.as_str()) {
            return None;
        }
        let text = value::text(&prm.value)?;
        gone.iter().find(|m| m.id == text || (!m.name.is_empty() && m.name == text)).map(|m| m.name.clone())
    };
    let mut out = Vec::new();
    for prm in &font.info.params.0 {
        if let Some(m) = names_one(prm) {
            out.push(format!("the font's parameter `{}` names the removed master `{m}`", prm.name));
        }
    }
    for owner in font.masters.iter().filter(|m| !gone.iter().any(|g| g.id == m.id)) {
        for prm in &owner.params.0 {
            if let Some(m) = names_one(prm) {
                out.push(format!("master `{}`'s parameter `{}` names the removed master `{m}`", owner.name, prm.name));
            }
        }
    }
    for inst in font.instances.iter() {
        for prm in &inst.params.0 {
            if let Some(m) = names_one(prm) {
                out.push(format!("export `{}`'s parameter `{}` names the removed master `{m}`", inst.name, prm.name));
            }
        }
    }
    out
}

fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.master.reorder", p)?;
    let font = s.font()?;
    let from = one_target(&p, font, "master")?;
    let last = font.masters.len().saturating_sub(1);
    let to = p.opt_int("toIndex", 0, i64::try_from(last).unwrap_or(i64::MAX))?.ok_or_else(|| p.bad("`toIndex` is required"))?;
    let to = usize::try_from(to).map_err(|_| p.bad("`toIndex` must not be negative"))?;
    let id = master_at(&p, font, from)?.id.clone();
    s.edit("Reorder Masters", |font| {
        let masters = Arc::make_mut(&mut font.masters);
        if from < masters.len() && to < masters.len() {
            let m = masters.remove(from);
            masters.insert(to, m);
        }
        Ok(())
    })?;
    Ok(json!({"master": id, "index": to}))
}
