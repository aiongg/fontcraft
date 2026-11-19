//! Designspace helpers (M9.36). Rows: `font.setMasterLocations`, `glyph.replaceWithInstance`,
//! `font.addVariation`, `font.mergeFontsToMasters`.
//!
//! | Command | What it does |
//! |---|---|
//! | Set Master Locations | maps the design coordinates on one axis through a table, or a scale and an offset |
//! | Adjust by Variation | replaces master layers with the glyph interpolated or extrapolated at another location |
//! | Add Variation | copies a master onto an axis (a new one, or one the font has) and runs commands on the copy |
//! | Merge Fonts to Masters | opens a new document with one master for each single-master font |
//!
//! Every coordinate here is a design (internal) coordinate.
//!
//! Set Master Locations moves the masters and the instances. When it moves the whole font (no
//! `masters` or `instances` list), the virtual masters, the intermediate layers, the alternate
//! layers' ranges and the internal side of the axis's Axis Mappings move through the same map,
//! so every source keeps its place relative to the masters.
//!
//! Merge Fonts to Masters places each font by its weight class (`wght`, the class value) and its
//! width class (`wdth`, the percentage the class stands for). An axis is made only for a class
//! the fonts differ in. The new font is a copy of the first font with the masters, metric, stem
//! and number definitions, glyph layers and kerning of all of them, and one instance per font at
//! its master.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_fontinfo::{MAX_MASTERS, MAX_METRIC_DEFS, MAX_STEM_DEFS, target};
use fontcraft_interp::GlyphSources;
use fontcraft_interp::designspace::{Designspace, Location};
use fontcraft_interp::instance::{InterpOptions, ModelKind, interpolate_layer};
use fontcraft_interp::params as iparams;
use fontcraft_model::plist::{Num, Value as PValue};
use fontcraft_model::{
    Axis, Font, Glyph, GlyphTable, KernContext, KernTable, Kerning, Layer, MAX_COORDINATE, MAX_GLYPHS, Master, MasterKerning, MetricDef, NumberDef,
    Source, StemDef, StyleClass,
};
use serde_json::{Map, Value, json};

use super::common::in_range;
use super::*;
use crate::EngineError;
use crate::params::{Params, kind};

/// Most points a `map` table holds.
pub const MAX_MAP_POINTS: usize = 1_024;
/// Smallest `scale` in size: a smaller one would merge locations that differ.
const MIN_SCALE: f64 = 1e-6;
/// Largest `scale` in size.
const MAX_SCALE: f64 = 1e6;
/// Most commands Add Variation runs on its new master.
pub const MAX_VARIATION_STEPS: usize = 1_000;
/// The text in Add Variation's command parameters that stands for the new master's id.
pub const MASTER_PLACEHOLDER: &str = "$master";
/// How deep into a step's parameters the placeholder is looked for.
const MAX_PLACEHOLDER_DEPTH: usize = 64;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.setMasterLocations",
            "Set Master Locations…",
            ["Font"],
            None,
            "{axis: tag|name, map?: [{from: number, to: number}] (at most 1024), scale?: number, offset?: number, masters?: [id|name|index], instances?: [id|name|index]} change the design coordinates on one axis in one step. `map` is a table: a coordinate equal to a `from` becomes its `to`, one between two entries is placed proportionally, one beyond the ends follows the nearest pair (a single entry shifts every coordinate); the `to` values must rise, or fall, with `from`. Otherwise each coordinate becomes coordinate × scale + offset (scale 1 and offset 0 when left out). Without `masters` and `instances` every master and instance moves, and so do the virtual masters, the intermediate layers, the alternate layers' ranges and the internal values of the axis's Axis Mappings; with either list only the masters and instances named move. A coordinate pushed beyond ±1000000 is refused → {axis, masters: [id], instances: [index], virtualMasters: number, layers: number, axisMappings: bool}",
            has_doc,
            set_master_locations
        ),
        cmd!(
            "glyph.replaceWithInstance",
            "Adjust by Variation",
            ["Glyph"],
            None,
            "{glyphs?: [name], master?: id|name|index, location: {axisTag: number}, relative?: bool = false} replace each glyph's master layer with the glyph interpolated, or extrapolated, at another design location: absolute coordinates, or offsets from the master's own location when `relative`; axes left out keep the master's value. The width, shapes, anchors and vertical metrics change; the layer keeps everything else. `master` defaults to the first master; without `glyphs`, every glyph with a layer for it. Locked glyphs, glyphs without a layer for the master and glyphs whose layers cannot interpolate are skipped with the reason; when none is left the command is refused → {master, location: {tag: number}, glyphs: [name], skipped: [{glyph, reason}]}",
            has_doc,
            replace_with_instance
        ),
        cmd!(
            "font.addVariation",
            "Add Variation…",
            ["Font"],
            None,
            "{axis: tag | {tag: string, name?: string}, location: number, from?: id|name|index, name?: string, commands?: [{command: id, params?: object}] (at most 1000)} copy a master (the first one by default) with its layers and kerning, and place the copy at `location` on the axis. An axis the font lacks is added first, with every other master, instance and virtual master at 0 on it. The copy is called `name`, else the original's name, the axis name and the location. Then each command runs, in order, with every text \"$master\" in its params replaced by the copy's id. One undo step; when a command fails, nothing changes → {master: {id, name, index}, axis: {tag, name, added}, results: [value]}",
            has_doc,
            add_variation
        ),
        cmd!(
            "font.mergeFontsToMasters",
            "Merge Fonts to Masters",
            ["Font"],
            None,
            "{fonts: [document title | path] (2 to 256), family?: string} open a new font with one master for each font given. Each font needs one master and the same units per em. Each is placed by its weight class (the `weightClass` parameter of its master or font, else its first static instance's class; 400 when it has none) on a `wght` axis and by its width class (as a percentage: 100 is normal) on a `wdth` axis; an axis is made only for a class the fonts differ in, and two fonts at the same place are refused. The new font is a copy of the first one with every font's master (metric, stem and number values matched by definition), glyph layers (background and special layers included), kerning and one static instance at its master; a glyph some fonts lack has no layer for their masters → {title, axes: [tag], masters: [{id, name, location: {tag: number}}], instances: number, glyphs: number}",
            always,
            merge_fonts_to_masters
        )
        .files(const { &[FileParam::read("fonts").document()] }),
    ]
}

// ── Shared ──────────────────────────────────────────────────────────────────────────────────────

/// The options instances interpolate with: the model the font's Font Type asks for.
fn interp_options(font: &Font) -> InterpOptions {
    let model = if font.info.settings.font_type.as_deref() == Some("variable") { ModelKind::Variable } else { ModelKind::Default };
    InterpOptions { model, ..InterpOptions::default() }
}

/// The index of the axis `name` gives by tag, else by name.
fn axis_index(p: &Params, font: &Font, key: &str, name: &str) -> Result<usize> {
    font.axes.iter().position(|a| a.tag == name).or_else(|| font.axes.iter().position(|a| a.name == name)).ok_or_else(|| {
        let tags: Vec<&str> = font.axes.iter().map(|a| a.tag.as_str()).collect();
        p.bad(format!("`{key}`: the font has no axis `{name}` (its axes: {})", if tags.is_empty() { "none".to_string() } else { tags.join(", ") }))
    })
}

/// A location as `{tag: value}`, in the font's axis order.
fn location_json(font: &Font, location: &[f64]) -> Value {
    Value::Object(font.axes.iter().zip(location).map(|(a, v)| (a.tag.clone(), json!(v))).collect::<Map<String, Value>>())
}

/// The masters a parameter names, each once; `None` when it is absent.
fn master_list(p: &Params, font: &Font, key: &str) -> Result<Option<Vec<usize>>> {
    let Some(raw) = p.get(key) else { return Ok(None) };
    if let Value::Array(a) = raw
        && a.len() > MAX_MASTERS
    {
        return Err(p.bad(format!("`{key}` holds {} targets; a font has at most {MAX_MASTERS} masters", a.len())));
    }
    let refs = target::parse_list(raw).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    target::masters(font, &refs).map(Some).map_err(|e| p.bad(format!("`{key}`: {e}")))
}

/// The one master a parameter names; `None` when it is absent.
fn one_master(p: &Params, font: &Font, key: &str) -> Result<Option<usize>> {
    let Some(raw) = p.get(key) else { return Ok(None) };
    let r = target::parse(raw).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    target::master(font, &r).map(Some).map_err(|e| p.bad(format!("`{key}`: {e}")))
}

// ── font.setMasterLocations ─────────────────────────────────────────────────────────────────────

/// How Set Master Locations moves a coordinate.
#[derive(Clone, Debug, PartialEq)]
pub enum CoordinateMap {
    /// `(from, to)` pairs: `from` strictly rising, `to` strictly rising or strictly falling.
    Table(Vec<(f64, f64)>),
    Linear {
        scale: f64,
        offset: f64,
    },
}

impl CoordinateMap {
    /// Where coordinate `v` goes.
    pub fn apply(&self, v: f64) -> f64 {
        match self {
            Self::Linear { scale, offset } => v * scale + offset,
            Self::Table(points) => table_value(points, v),
        }
    }

    /// Larger coordinates come out smaller: ranges swap their ends.
    pub fn reverses(&self) -> bool {
        match self {
            Self::Linear { scale, .. } => *scale < 0.0,
            Self::Table(points) => match (points.first(), points.last()) {
                (Some(a), Some(b)) => b.1 < a.1,
                _ => false,
            },
        }
    }
}

/// `v` through the table: an entry's own value exactly, proportionally between two entries, and
/// along the nearest pair beyond the ends. One entry shifts every value by `to − from`.
fn table_value(points: &[(f64, f64)], v: f64) -> f64 {
    if let Some(&(_, to)) = points.iter().find(|pt| pt.0 == v) {
        return to;
    }
    match points {
        [] => v,
        [(from, to)] => v + (to - from),
        _ => {
            let last_pair = points.len().saturating_sub(2);
            let i = points.iter().rposition(|pt| pt.0 <= v).unwrap_or(0).min(last_pair);
            let (Some(&(x0, y0)), Some(&(x1, y1))) = (points.get(i), points.get(i + 1)) else { return v };
            if x1 == x0 {
                return y0;
            }
            y0 + (v - x0) * (y1 - y0) / (x1 - x0)
        }
    }
}

/// Reads `map`, or `scale` and `offset`.
fn parse_map(p: &Params) -> Result<CoordinateMap> {
    let table = p.opt_array("map")?;
    let scale = p.opt_f64("scale")?;
    let offset = p.opt_f64("offset")?;
    match (table, scale, offset) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(p.bad("give `map`, or `scale` and `offset`, not both")),
        (None, None, None) => Err(p.bad("give `map`, or `scale` and `offset`")),
        (None, scale, offset) => {
            let scale = scale.unwrap_or(1.0);
            if !(MIN_SCALE..=MAX_SCALE).contains(&scale.abs()) {
                return Err(p.bad(format!("`scale` must be from {MIN_SCALE} to {MAX_SCALE} in size (positive or negative), got {scale}")));
            }
            let offset = offset.unwrap_or(0.0);
            if offset.abs() > 2.0 * MAX_COORDINATE {
                return Err(p.bad(format!("`offset` must be from -{} to {}", 2.0 * MAX_COORDINATE, 2.0 * MAX_COORDINATE)));
            }
            Ok(CoordinateMap::Linear { scale, offset })
        }
        (Some(list), None, None) => {
            if list.is_empty() {
                return Err(p.bad("`map` is empty"));
            }
            if list.len() > MAX_MAP_POINTS {
                return Err(p.bad(format!("`map` holds {} entries; at most {MAX_MAP_POINTS} are allowed", list.len())));
            }
            let mut points = Vec::with_capacity(list.len());
            for (i, item) in list.iter().enumerate() {
                let Value::Object(o) = item else {
                    return Err(p.bad(format!("`map[{i}]` must be an object {{from, to}}, got {}", kind(item))));
                };
                let number = |key: &str| o.get(key).and_then(Value::as_f64).filter(|v| in_range(*v));
                let (Some(from), Some(to)) = (number("from"), number("to")) else {
                    return Err(p.bad(format!("`map[{i}]` needs `from` and `to`, numbers from -{MAX_COORDINATE} to {MAX_COORDINATE}")));
                };
                points.push((from, to));
            }
            points.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let Some([a, _]) = points.windows(2).find(|w| matches!(w, [a, b] if a.0 == b.0)) {
                return Err(p.bad(format!("`map` gives coordinate {} twice", a.0)));
            }
            let rising = points.windows(2).all(|w| matches!(w, [a, b] if b.1 > a.1));
            let falling = points.windows(2).all(|w| matches!(w, [a, b] if b.1 < a.1));
            if !rising && !falling {
                return Err(p.bad("`map` must keep the coordinates in order: its `to` values must all rise, or all fall, as `from` rises"));
            }
            Ok(CoordinateMap::Table(points))
        }
    }
}

/// `v` through `map`, refused when the result is beyond the coordinate limit.
fn move_value(p: &Params, map: &CoordinateMap, v: f64, what: impl FnOnce() -> String) -> Result<f64> {
    let out = map.apply(v);
    if in_range(out) { Ok(out) } else { Err(p.bad(format!("{}: coordinate {v} would become {out}, beyond ±{MAX_COORDINATE}", what()))) }
}

/// Whether a layer has a coordinate or a range on axis `axis`.
fn layer_on_axis(layer: &Layer, axis: usize) -> bool {
    let a = &layer.info.attrs;
    a.coordinates.get(axis).is_some_and(Option::is_some) || a.axis_rules.get(axis).is_some_and(|r| r.min.is_some() || r.max.is_some())
}

fn set_master_locations(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.setMasterLocations", v)?;
    let font = s.font()?;
    let axis = axis_index(&p, font, "axis", p.str("axis")?)?;
    let map = parse_map(&p)?;
    let named_masters = master_list(&p, font, "masters")?;
    let named_instances = match p.get("instances") {
        None => None,
        Some(raw) => {
            let refs = target::parse_list(raw).map_err(|e| p.bad(format!("`instances`: {e}")))?;
            Some(target::instances(font, &refs).map_err(|e| p.bad(format!("`instances`: {e}")))?)
        }
    };
    let whole = named_masters.is_none() && named_instances.is_none();
    let masters = named_masters.unwrap_or_else(|| (0..font.masters.len()).collect());
    let instances = named_instances.unwrap_or_else(|| (0..font.instances.len()).collect());
    let (tag, axis_name) = font.axes.get(axis).map(|a| (a.tag.clone(), a.name.clone())).unwrap_or_default();
    let reverses = map.reverses();

    let (master_ids, virtual_masters, layers, mapped) = s.edit("Set Master Locations", |font| {
        let mut master_ids = Vec::with_capacity(masters.len());
        let list = Arc::make_mut(&mut font.masters);
        for &i in &masters {
            let Some(m) = list.get_mut(i) else { continue };
            let label = if m.name.is_empty() { m.id.clone() } else { m.name.clone() };
            if let Some(slot) = m.location.get_mut(axis) {
                *slot = move_value(&p, &map, *slot, || format!("master `{label}`"))?;
            }
            master_ids.push(m.id.clone());
        }
        let list = Arc::make_mut(&mut font.instances);
        for &i in &instances {
            let Some(inst) = list.get_mut(i) else { continue };
            let label = if inst.name.is_empty() { format!("#{i}") } else { inst.name.clone() };
            if let Some(slot) = inst.location.get_mut(axis) {
                *slot = move_value(&p, &map, *slot, || format!("instance `{label}`"))?;
            }
        }
        let (mut virtual_masters, mut layers, mut mapped) = (0usize, 0usize, false);
        if !whole {
            return Ok((master_ids, virtual_masters, layers, mapped));
        }

        let entries = iparams::virtual_masters(font).map_err(|e| p.bad(e.to_string()))?;
        let mut moved = entries.clone();
        for (n, entry) in moved.iter_mut().enumerate() {
            for (name, value) in entry.coordinates.iter_mut().filter(|(name, _)| *name == axis_name) {
                *value = move_value(&p, &map, *value, || format!("virtual master {} ({name})", n + 1))?;
                virtual_masters += 1;
            }
        }
        if moved != entries {
            let params = iparams::set_virtual_masters(&font.info.params, &moved).map_err(|e| p.bad(e.to_string()))?;
            Arc::make_mut(&mut font.info).params = params;
        }

        let mappings = iparams::axis_mappings(font).map_err(|e| p.bad(e.to_string()))?;
        if let Some((_, points)) = mappings.iter().find(|(t, _)| *t == tag) {
            let points = points
                .iter()
                .map(|&(external, internal)| -> Result<(f64, f64)> {
                    Ok((external, move_value(&p, &map, internal, || format!("the Axis Mappings point at {external}"))?))
                })
                .collect::<Result<Vec<_>>>()?;
            let params = iparams::set_axis_mappings(&font.info.params, &tag, &points).map_err(|e| p.bad(e.to_string()))?;
            Arc::make_mut(&mut font.info).params = params;
            mapped = true;
        }

        for gi in 0..font.glyphs.len() {
            let touched = font.glyphs.as_arcs().get(gi).is_some_and(|g| g.layers.iter().any(|l| layer_on_axis(l, axis)));
            if !touched {
                continue;
            }
            let Some(mut g) = font.glyphs.get_mut_at(gi) else { continue };
            let glyph = g.name.clone();
            for layer in g.layers.iter_mut().filter(|l| layer_on_axis(l, axis)) {
                let what = || format!("a special layer of glyph `{glyph}`");
                let attrs = &mut layer.info.attrs;
                if let Some(Some(value)) = attrs.coordinates.get_mut(axis) {
                    *value = move_value(&p, &map, *value, what)?;
                }
                if let Some(rule) = attrs.axis_rules.get_mut(axis) {
                    let min = rule.min.map(|v| move_value(&p, &map, v, what)).transpose()?;
                    let max = rule.max.map(|v| move_value(&p, &map, v, what)).transpose()?;
                    (rule.min, rule.max) = if reverses { (max, min) } else { (min, max) };
                }
                layers += 1;
            }
        }
        Ok((master_ids, virtual_masters, layers, mapped))
    })?;
    Ok(json!({
        "axis": tag,
        "masters": master_ids,
        "instances": instances,
        "virtualMasters": virtual_masters,
        "layers": layers,
        "axisMappings": mapped,
    }))
}

// ── glyph.replaceWithInstance ───────────────────────────────────────────────────────────────────

fn replace_with_instance(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.replaceWithInstance", v)?;
    let font = s.font()?;
    let index = one_master(&p, font, "master")?.unwrap_or(0);
    let master = font.masters.get(index).ok_or_else(|| p.bad("the font has no masters"))?;
    let master_id = master.id.clone();
    let ds = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
    let base = ds.master(&master_id).ok_or_else(|| p.bad(format!("master `{}` takes no part in interpolation", master.name)))?.location.clone();
    let given = p.opt_object("location")?.ok_or_else(|| p.bad("`location` is required"))?;
    if given.is_empty() {
        return Err(p.bad("`location` names no axis"));
    }
    let relative = p.bool_or("relative", false)?;
    let mut values = base.0;
    for (tag, value) in given {
        let (i, _) = ds.axis(tag).ok_or_else(|| p.bad(format!("`location`: the font has no axis `{tag}`")))?;
        let n = value
            .as_f64()
            .filter(|n| in_range(*n))
            .ok_or_else(|| p.bad(format!("`location.{tag}` must be a number from -{MAX_COORDINATE} to {MAX_COORDINATE}, got {}", kind(value))))?;
        let slot = values.get_mut(i).ok_or_else(|| p.bad(format!("the master has no coordinate on axis `{tag}`")))?;
        let new = if relative { *slot + n } else { n };
        if !in_range(new) {
            return Err(p.bad(format!("`location.{tag}`: the master's coordinate plus {n} is beyond ±{MAX_COORDINATE}")));
        }
        *slot = new;
    }
    let location = Location(values);
    ds.to_normalized(&location).map_err(|e| p.bad(e.to_string()))?;
    let names: Vec<String> = match p.opt_glyphs("glyphs", font)? {
        Some(list) if list.is_empty() => return Err(p.bad("`glyphs` is empty")),
        Some(list) => list,
        None => font.glyphs.iter().filter(|g| g.layer(&master_id).is_some()).map(|g| g.name.clone()).collect(),
    };
    let options = interp_options(font);
    let mut replaced: Vec<(String, Layer)> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for name in &names {
        let mut skip = |reason: String| skipped.push(json!({"glyph": name, "reason": reason}));
        let Some(glyph) = font.glyph(name) else { continue };
        if glyph.info.locked {
            skip("the glyph is locked".into());
            continue;
        }
        let Some(layer) = glyph.layer(&master_id) else {
            skip(format!("the glyph has no layer for master `{}`", master.name));
            continue;
        };
        let sources = match GlyphSources::of(font, &ds, name) {
            Ok(sources) => sources,
            Err(e) => {
                skip(e.to_string());
                continue;
            }
        };
        match interpolate_layer(font, &sources, &location, &options) {
            Ok(new) => {
                let mut out = layer.clone();
                out.width = new.width;
                out.shapes = new.shapes;
                out.anchors = new.anchors;
                out.info.vert_width = new.info.vert_width;
                out.info.vert_origin = new.info.vert_origin;
                replaced.push((name.clone(), out));
            }
            Err(e) => skip(e.to_string()),
        }
    }
    if replaced.is_empty() {
        let why =
            skipped.first().and_then(|v| v.get("reason")).and_then(Value::as_str).unwrap_or("the font has no glyph with a layer for the master");
        return Err(p.bad(format!("no glyph was adjusted: {why}")));
    }
    let location_out = location_json(font, &location.0);
    let glyphs: Vec<String> = replaced.iter().map(|(n, _)| n.clone()).collect();
    s.edit("Adjust by Variation", |font| {
        for (name, layer) in replaced {
            let mut g = font.glyph_mut(&name)?;
            if let Some(slot) = g.layer_mut(&layer.id) {
                *slot = layer;
            }
        }
        Ok(())
    })?;
    Ok(json!({"master": master_id, "location": location_out, "glyphs": glyphs, "skipped": skipped}))
}

// ── font.addVariation ───────────────────────────────────────────────────────────────────────────

/// `value` with every text equal to [`MASTER_PLACEHOLDER`] replaced by `id`.
fn put_master(value: &mut Value, id: &str, depth: usize) {
    if depth > MAX_PLACEHOLDER_DEPTH {
        return;
    }
    match value {
        Value::String(text) if *text == MASTER_PLACEHOLDER => *text = id.to_string(),
        Value::Array(items) => items.iter_mut().for_each(|v| put_master(v, id, depth + 1)),
        Value::Object(map) => map.values_mut().for_each(|v| put_master(v, id, depth + 1)),
        _ => {}
    }
}

fn add_variation(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.addVariation", v)?;
    let font = s.font()?;
    let from = one_master(&p, font, "from")?.unwrap_or(0);
    let original = font.masters.get(from).ok_or_else(|| p.bad("the font has no masters"))?;
    let (tag, axis_name) = match p.get("axis") {
        None => return Err(p.bad("`axis` is required")),
        Some(Value::String(t)) => (t.clone(), None),
        Some(Value::Object(o)) => {
            let tag = o.get("tag").and_then(Value::as_str).ok_or_else(|| p.bad("`axis.tag` is required"))?;
            let name = match o.get("name") {
                None | Some(Value::Null) => None,
                Some(Value::String(n)) => Some(n.trim().to_string()),
                Some(other) => return Err(p.bad(format!("`axis.name` must be a string, got {}", kind(other)))),
            };
            (tag.to_string(), name)
        }
        Some(other) => return Err(p.bad(format!("`axis` must be a tag or {{tag, name?}}, got {}", kind(other)))),
    };
    let location = p.f64("location")?;
    if !in_range(location) {
        return Err(p.bad(format!("`location` must be from -{MAX_COORDINATE} to {MAX_COORDINATE}")));
    }
    let name = p.opt_str("name")?.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let steps = p.opt_array("commands")?.map(Vec::as_slice).unwrap_or_default();
    if steps.len() > MAX_VARIATION_STEPS {
        return Err(p.bad(format!("`commands` holds {} entries; at most {MAX_VARIATION_STEPS} are allowed", steps.len())));
    }
    for (i, step) in steps.iter().enumerate() {
        match step.get("command").and_then(Value::as_str) {
            None => return Err(p.bad(format!("`commands[{i}]` has no `command`"))),
            Some(id @ ("edit.undo" | "edit.redo")) => return Err(p.bad(format!("`commands[{i}]`: `{id}` cannot run inside Add Variation"))),
            Some(id) if find_command(id).is_none() => return Err(p.bad(format!("`commands[{i}]`: there is no command `{id}`"))),
            Some(_) => {}
        }
        if let Some(params) = step.get("params")
            && !params.is_object()
            && !params.is_null()
        {
            return Err(p.bad(format!("`commands[{i}].params` must be an object, got {}", kind(params))));
        }
    }
    let existing = font.axes.iter().position(|a| a.tag == tag);
    if let (Some(i), Some(n)) = (existing, &axis_name)
        && let Some(axis) = font.axes.get(i)
        && axis.name != *n
    {
        return Err(p.bad(format!("the font's axis `{tag}` is called `{}`, not `{n}`", axis.name)));
    }
    let start = existing.and_then(|i| original.location.get(i).copied()).unwrap_or(0.0);
    if start == location {
        return Err(p.bad(format!("master `{}` is already at {location} on `{tag}`: give another location", original.name)));
    }
    let (from_id, from_name) = (original.id.clone(), original.name.clone());

    super::audit::one_step(s, "Add Variation", |s| {
        if existing.is_none() {
            let mut axis = Map::new();
            axis.insert("tag".into(), json!(tag));
            if let Some(n) = &axis_name {
                axis.insert("name".into(), json!(n));
            }
            s.execute("font.axes.add", &Value::Object(axis))?;
        }
        let copy = s.execute("font.master.duplicate", &json!({"masters": [{"id": from_id}]}))?;
        let id = copy.pointer("/masters/0/id").and_then(Value::as_str).map(str::to_string);
        let index = copy.pointer("/masters/0/index").and_then(Value::as_u64);
        let (Some(id), Some(index)) = (id, index) else {
            return Err(EngineError::Internal("copying the master reported no id".into()));
        };
        let axis_label = s.font()?.axes.iter().find(|a| a.tag == tag).map_or_else(|| tag.clone(), |a| a.name.clone());
        let new_name = name.clone().unwrap_or_else(|| format!("{from_name} {axis_label} {location}"));
        let mut at = Map::new();
        at.insert(tag.clone(), json!(location));
        s.execute("font.master.setAxisLocation", &json!({"masters": [{"id": id}], "location": at}))?;
        s.execute("font.master.set", &json!({"masters": [{"id": id}], "name": new_name}))?;
        let mut results = Vec::with_capacity(steps.len());
        for (i, step) in steps.iter().enumerate() {
            let command = step.get("command").and_then(Value::as_str).unwrap_or_default();
            let mut params = step.get("params").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!({}));
            put_master(&mut params, &id, 0);
            // Each step is a request of its own while automation roots are in force.
            if let (Some(roots), Some(spec)) = (crate::file_access::current(), find_command(command)) {
                params = crate::file_access::command::check(s, spec, &params, &roots)
                    .map_err(|e| p.bad(format!("`commands[{i}]` (`{command}`) failed: {e}")))?;
            }
            let value = s.execute(command, &params).map_err(|e| p.bad(format!("`commands[{i}]` (`{command}`) failed: {e}")))?;
            results.push(value);
        }
        Ok(json!({
            "master": {"id": id, "name": new_name, "index": index},
            "axis": {"tag": tag, "name": axis_label, "added": existing.is_none()},
            "results": results,
        }))
    })
}

// ── font.mergeFontsToMasters ────────────────────────────────────────────────────────────────────

/// An open document named by title or path, else a font file read from `r`.
fn font_ref(s: &Session, r: &str) -> Result<Arc<Font>> {
    if let Some(d) = s.documents().iter().find(|d| d.title == r || d.path.as_deref().is_some_and(|p| p == std::path::Path::new(r))) {
        return Ok(d.font.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (font, _) = crate::io::open(std::path::Path::new(r), None)?;
        Ok(Arc::new(font))
    }
    #[cfg(target_arch = "wasm32")]
    Err(crate::EngineError::Io(format!("no open document is called `{r}`")))
}

fn merge_fonts_to_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.mergeFontsToMasters", v)?;
    if let Some(Value::Array(a)) = p.get("fonts")
        && a.len() > MAX_MASTERS
    {
        return Err(p.bad(format!("`fonts` names {} fonts; a font has at most {MAX_MASTERS} masters", a.len())));
    }
    let refs = p.strings("fonts")?;
    if refs.len() < 2 {
        return Err(p.bad("`fonts` must name at least two fonts"));
    }
    let family = p.opt_str("family")?.map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
    let mut fonts = Vec::with_capacity(refs.len());
    for r in &refs {
        fonts.push((r.to_string(), font_ref(s, r)?));
    }
    let mut merged = merge_fonts(&fonts).map_err(|e| p.bad(e))?;
    if let Some(f) = family {
        merged.family_name = f;
    }
    let axes: Vec<&str> = merged.axes.iter().map(|a| a.tag.as_str()).collect();
    let masters: Vec<Value> =
        merged.masters.iter().map(|m| json!({"id": m.id, "name": m.name, "location": location_json(&merged, &m.location)})).collect();
    let mut out = json!({"axes": axes, "masters": masters, "instances": merged.instances.len(), "glyphs": merged.glyphs.len()});
    s.add_document(merged, None)?;
    let title = s.doc()?.title.clone();
    if let Some(o) = out.as_object_mut() {
        o.insert("title".into(), json!(title));
    }
    Ok(out)
}

/// A number from a parameter value that may be written as text (`"600"`).
fn param_number(v: &PValue) -> Option<f64> {
    let n = match v {
        PValue::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        other => other.as_num().map(Num::as_f64)?,
    };
    n.is_finite().then_some(n)
}

/// Where a single-master font sits on a weight (`wght`: the weight class) or width (`wdth`: the
/// percentage its width class stands for) axis: the `weightClass` or `widthClass` parameter of
/// its master, else of the font, else its first static instance's class; 400 and 100 when it
/// states none.
fn style_position(font: &Font, master: &Master, tag: &str) -> f64 {
    let wght = tag == "wght";
    let param = if wght { "weightClass" } else { "widthClass" };
    let from_params = |params: &fontcraft_model::Params| -> Option<f64> {
        let class = params.get(param).and_then(param_number)?.trunc();
        if wght { (1.0..=1000.0).contains(&class).then_some(class) } else { fontcraft_model::axes::width_class_value(class) }
    };
    if let Some(v) = from_params(&master.params).or_else(|| from_params(&font.info.params)) {
        return v;
    }
    match font.instances.iter().find(|i| i.kind.is_none()) {
        Some(inst) => fontcraft_model::axes::instance_class_value(inst, tag),
        None if wght => 400.0,
        None => 100.0,
    }
}

/// The OS/2 width class (1 to 9) whose percentage is `percent`.
fn width_class_code(percent: f64) -> Option<f64> {
    (1..=9u8).map(f64::from).find(|c| fontcraft_model::axes::width_class_value(*c) == Some(percent))
}

/// One font being merged.
struct MergeSource<'f> {
    label: &'f str,
    font: &'f Font,
    master: &'f Master,
    weight: f64,
    width: f64,
}

fn same_metric(a: &MetricDef, b: &MetricDef) -> bool {
    a.kind == b.kind && a.name == b.name && a.filter == b.filter
}

fn same_stem(a: &StemDef, b: &StemDef) -> bool {
    a.name == b.name && a.horizontal == b.horizontal && a.filter == b.filter
}

fn same_number(a: &NumberDef, b: &NumberDef) -> bool {
    a.name == b.name
}

/// The definitions of the first font with those of the later fonts it lacks appended.
fn merged_defs<T: Clone>(sources: &[MergeSource<'_>], defs: impl Fn(&Font) -> &[T], same: impl Fn(&T, &T) -> bool) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for src in sources {
        for d in defs(src.font) {
            if !out.iter().any(|x| same(x, d)) {
                out.push(d.clone());
            }
        }
    }
    out
}

/// A master's values re-ordered for the merged definitions: a definition its font lacks has the
/// default value.
fn values_for<T, V: Copy + Default>(merged: &[T], own: &[T], values: &[V], same: impl Fn(&T, &T) -> bool) -> Vec<V> {
    merged.iter().map(|d| own.iter().position(|x| same(x, d)).and_then(|i| values.get(i).copied()).unwrap_or_default()).collect()
}

/// A layer id of the 8-4-4-4-12 hex form, new in `glyph`. It depends only on `seed`, so a replay
/// gives the same id.
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

fn kern_direction(k: &Kerning, i: usize) -> &KernTable {
    match i {
        0 => &k.ltr,
        1 => &k.rtl,
        _ => &k.vertical,
    }
}

/// One multi-master font from single-master fonts (`(label, font)`, the label naming the font in
/// messages), as Merge Fonts to Masters builds it; the first problem when they cannot merge.
pub fn merge_fonts(fonts: &[(String, Arc<Font>)]) -> std::result::Result<Font, String> {
    let Some((_, base)) = fonts.first() else { return Err("there are no fonts to merge".into()) };
    if fonts.len() > MAX_MASTERS {
        return Err(format!("a font has at most {MAX_MASTERS} masters"));
    }
    let mut sources: Vec<MergeSource> = Vec::with_capacity(fonts.len());
    for (label, font) in fonts {
        let master = match font.masters.as_slice() {
            [m] => m,
            list => return Err(format!("`{label}` has {} masters: each font to merge needs exactly one", list.len())),
        };
        if font.units_per_em != base.units_per_em {
            return Err(format!("`{label}` has {} units per em and the first font {}", font.units_per_em, base.units_per_em));
        }
        let (weight, width) = (style_position(font, master, "wght"), style_position(font, master, "wdth"));
        sources.push(MergeSource { label, font, master, weight, width });
    }
    let differ = |f: fn(&MergeSource<'_>) -> f64| sources.iter().any(|s| sources.first().is_some_and(|first| f(s) != f(first)));
    let (has_weight, has_width) = (differ(|s| s.weight), differ(|s| s.width));
    if !has_weight && !has_width {
        return Err("the fonts have the same weight class and width class: nothing places them apart in a designspace".into());
    }
    for (i, a) in sources.iter().enumerate() {
        if let Some(b) = sources.iter().skip(i + 1).find(|b| b.weight == a.weight && b.width == a.width) {
            return Err(format!("`{}` and `{}` have the same weight class and width class", a.label, b.label));
        }
    }
    let mut axes = Vec::new();
    if has_weight {
        axes.push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
    }
    if has_width {
        axes.push(Axis { name: "Width".into(), tag: "wdth".into(), ..Default::default() });
    }
    let location = |s: &MergeSource<'_>| -> Vec<f64> {
        let mut l = Vec::with_capacity(2);
        if has_weight {
            l.push(s.weight);
        }
        if has_width {
            l.push(s.width);
        }
        l
    };

    let metrics = merged_defs(&sources, |f| f.metrics.as_slice(), same_metric);
    let stems = merged_defs(&sources, |f| f.stems.as_slice(), same_stem);
    let numbers = merged_defs(&sources, |f| f.numbers.as_slice(), same_number);
    if metrics.len() > MAX_METRIC_DEFS {
        return Err(format!("the fonts define {} metrics together; a font has at most {MAX_METRIC_DEFS}", metrics.len()));
    }
    if stems.len() > MAX_STEM_DEFS {
        return Err(format!("the fonts define {} stems together; a font has at most {MAX_STEM_DEFS}", stems.len()));
    }

    // Master ids: a font keeps its own unless an earlier font took it or a special layer uses it.
    let layer_ids: HashSet<&str> = sources
        .iter()
        .flat_map(|s| s.font.glyphs.iter().flat_map(|g| g.layers.iter().filter(|l| !l.is_master_layer()).map(|l| l.id.as_str())))
        .collect();
    let mut ids: Vec<String> = Vec::with_capacity(sources.len());
    for src in &sources {
        let free = |id: &str| !ids.iter().any(|x| x == id) && !layer_ids.contains(id);
        let id = if free(&src.master.id) {
            src.master.id.clone()
        } else {
            (1..=10_000u32).map(|n| format!("m{n:02}")).find(|id| free(id)).ok_or("could not find a free master id")?
        };
        ids.push(id);
    }

    let mut masters = Vec::with_capacity(sources.len());
    for (src, id) in sources.iter().zip(&ids) {
        let mut m = src.master.clone();
        m.id = id.clone();
        m.location = location(src);
        m.metric_values = values_for(metrics.as_slice(), src.font.metrics.as_slice(), src.master.metric_values.as_slice(), same_metric);
        m.stem_values = values_for(stems.as_slice(), src.font.stems.as_slice(), src.master.stem_values.as_slice(), same_stem);
        m.number_values = values_for(numbers.as_slice(), src.font.numbers.as_slice(), src.master.number_values.as_slice(), same_number);
        // The master belongs to the new document, not to the file it came from.
        crate::linked::unmark(&mut m.user_data);
        masters.push(m);
    }

    let mut order: Vec<&str> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for src in &sources {
        for g in src.font.glyphs.iter() {
            if seen.insert(g.name.as_str()) {
                order.push(g.name.as_str());
            }
        }
        if order.len() > MAX_GLYPHS {
            return Err(format!("the fonts have more than {MAX_GLYPHS} glyphs together"));
        }
    }
    let mut glyphs: Vec<Arc<Glyph>> = Vec::with_capacity(order.len());
    for name in order {
        let mut merged: Option<Glyph> = None;
        for (src, id) in sources.iter().zip(&ids) {
            let Some(g) = src.font.glyph(name) else { continue };
            let out = merged.get_or_insert_with(|| Glyph { layers: Vec::new(), ..Glyph::clone(g) });
            for layer in g.layers.iter().filter(|l| l.master == src.master.id) {
                let mut layer = layer.clone();
                if layer.is_master_layer() {
                    layer.id = id.clone();
                } else if out.layers.iter().any(|l| l.id == layer.id) || ids.contains(&layer.id) {
                    let seed = format!("{name}|{}|{id}", layer.id);
                    layer.id = new_layer_id(out, &seed).ok_or_else(|| format!("could not find a free layer id in glyph `{name}`"))?;
                }
                layer.master = id.clone();
                out.layers.push(layer);
            }
        }
        if let Some(g) = merged {
            glyphs.push(Arc::new(g));
        }
    }

    let mut kerning = Kerning::default();
    for (i, slot) in [&mut kerning.ltr, &mut kerning.rtl, &mut kerning.vertical].into_iter().enumerate() {
        let entries: Vec<MasterKerning> = sources
            .iter()
            .zip(&ids)
            .flat_map(|(src, id)| {
                kern_direction(&src.font.kerning, i)
                    .0
                    .iter()
                    .filter(move |m| m.master == src.master.id)
                    .map(move |m| MasterKerning { master: id.clone(), firsts: m.firsts.clone() })
            })
            .collect();
        *slot = KernTable::new(entries);
    }
    for (src, id) in sources.iter().zip(&ids) {
        for c in &src.font.kerning.context {
            let values: Vec<(String, f64)> = c.values.iter().filter(|(m, _)| *m == src.master.id).map(|(_, v)| (id.clone(), *v)).collect();
            if values.is_empty() {
                continue;
            }
            match kerning.context.iter_mut().find(|x| x.context == c.context) {
                Some(x) => x.values.extend(values),
                None => kerning.context.push(KernContext { context: c.context.clone(), values }),
            }
        }
    }

    let mut instance_ids: HashSet<String> = HashSet::new();
    let mut instances = Vec::with_capacity(sources.len());
    for (src, m) in sources.iter().zip(&masters) {
        let mut inst = src.font.instances.iter().find(|i| i.kind.is_none()).cloned().unwrap_or_default();
        if inst.name.is_empty() {
            inst.name = m.name.clone();
        }
        inst.location = m.location.clone();
        inst.interpolation.clear();
        inst.manual_interpolation = false;
        if fontcraft_model::axes::instance_class_value(&inst, "wght") != src.weight {
            inst.params.remove("weightClass");
            inst.weight_class = Some(StyleClass::Number(src.weight));
        }
        if fontcraft_model::axes::instance_class_value(&inst, "wdth") != src.width {
            inst.params.remove("widthClass");
            inst.width_class = width_class_code(src.width).map(StyleClass::Number);
        }
        if let Some(id) = &inst.id
            && !instance_ids.insert(id.clone())
        {
            inst.id = None;
        }
        instances.push(inst);
    }

    let mut out = Font::clone(base);
    // A font with several masters that no single input file describes.
    out.source = Source::default();
    out.axes = Arc::new(axes);
    out.metrics = Arc::new(metrics);
    out.stems = Arc::new(stems);
    out.numbers = Arc::new(numbers);
    out.masters = Arc::new(masters);
    out.instances = Arc::new(instances);
    // Each instance carries its own class; a font-wide one would give every instance the first font's.
    let info = Arc::make_mut(&mut out.info);
    info.params.remove("weightClass");
    info.params.remove("widthClass");
    out.glyphs = GlyphTable::from_glyphs(glyphs);
    out.kerning = Arc::new(kerning);
    Ok(out)
}
