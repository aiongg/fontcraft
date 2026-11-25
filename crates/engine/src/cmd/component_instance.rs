//! Select Component Instance (M6.24): point a component at another master or layer of its base
//! glyph, or at a location on the font's axes.
//!
//! Row: `spec/inventory/90-secondary.toml` (`component.setInstanceLocation`). The pin is the
//! component's `master_id` (a master's id, or the id of another layer of the base glyph); the
//! location is kept in its user data (`com.fontcraft.componentLocation`) in design coordinates.
//! `fontcraft_model::pinned` draws both.

use std::collections::BTreeMap;

use fontcraft_model::{Component, Font, Glyph, Layer, MAX_COORDINATE, Shape};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::{Params, TargetKind as K, Targets, kind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "component.setInstanceLocation",
            "Select Component Instance",
            [],
            None,
            "{glyph: name, layer?: id (default the first master's layer), component?: index (default the layer's only component), location?: {axisTag: design coordinate}, layerName?: master or layer of the base glyph, by name or id} draws the component from another place than the layer it is on. `layerName` pins it to that master or layer of its base glyph; `location` draws the base glyph interpolated there, on the axes it names (the other axes stay at the layer's master), and takes precedence over a pin. Each leaves the other as it is; an empty `location` takes the location away and an empty `layerName` the pin; omitting both takes both away. Exports write such a component as outlines. A location on an axis the font does not have, or one where the base glyph cannot be interpolated, is refused → {glyph, layer, component, pinned: layer id | null, location: {axisTag: number} | null}",
            has_doc,
            set_instance_location
        )
        .targets(&[K::Layer]),
    ]
}

fn component_at(layer: &Layer, i: usize) -> Option<&Component> {
    match layer.shapes.get(i) {
        Some(Shape::Component(c)) => Some(c),
        _ => None,
    }
}

/// The shape index `component` names, or the layer's only component.
fn component_index(p: &Params, layer: &Layer) -> Result<usize> {
    if let Some(v) = p.get("component") {
        let i = v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| p.bad(format!("`component` must be a whole number from 0, found {}", kind(v))))?;
        return match component_at(layer, i) {
            Some(_) => Ok(i),
            None => Err(p.bad(format!("`component`: shape {i} is not a component of the layer"))),
        };
    }
    let mut it = (0..layer.shapes.len()).filter(|&i| component_at(layer, i).is_some());
    match (it.next(), it.next()) {
        (Some(i), None) => Ok(i),
        (None, _) => Err(p.bad(format!("layer `{}` has no components", layer.id))),
        _ => Err(p.bad("the layer has several components: give `component`")),
    }
}

/// The id of the layer of `base` that `name` names: a layer id, else the master of that name,
/// else the layer of that name.
fn layer_named(p: &Params, font: &Font, base: &Glyph, name: &str) -> Result<String> {
    if let Some(l) = base.layers.iter().find(|l| l.id == name) {
        return Ok(l.id.clone());
    }
    let by_master: Vec<&Layer> = font.masters.iter().filter(|m| m.name == name).filter_map(|m| base.layer(&m.id)).collect();
    let by_name: Vec<&Layer> = base.layers.iter().filter(|l| !l.is_master_layer() && l.info.name.as_deref() == Some(name)).collect();
    let found = if by_master.is_empty() { by_name } else { by_master };
    match found.as_slice() {
        [l] => Ok(l.id.clone()),
        [] => Err(p.bad(format!("`layerName`: `{}` has no master or layer `{name}`", base.name))),
        _ => Err(p.bad(format!("`layerName`: `{}` has several layers `{name}`: give a layer id", base.name))),
    }
}

/// The location `location` gives; empty when it names no axis.
fn location(p: &Params, font: &Font, given: &Map<String, Value>) -> Result<BTreeMap<String, f64>> {
    let mut out = BTreeMap::new();
    for (tag, v) in given {
        if !font.axes.iter().any(|a| a.tag == *tag) {
            return Err(p.bad(format!("`location`: the font has no axis `{tag}`")));
        }
        let value = v
            .as_f64()
            .filter(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
            .ok_or_else(|| p.bad(format!("`location`: `{tag}` must be a number within ±{MAX_COORDINATE}")))?;
        out.insert(tag.clone(), value);
    }
    Ok(out)
}

fn set_instance_location(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("component.setInstanceLocation", v)?;
    let t = Targets::new(p.command(), p.str("glyph")?, p.opt_str("layer")?.map(str::to_string));
    let font = s.doc()?.font.clone();
    let r = t.resolve(&font)?;
    let i = component_index(&p, r.layer)?;
    let mut c = component_at(r.layer, i).cloned().ok_or_else(|| p.bad("the component does not exist"))?;
    let base = font.glyph(&c.glyph).ok_or_else(|| p.bad(format!("the component's glyph `{}` does not exist", c.glyph)))?;
    let (pin, at) = (p.opt_str("layerName")?, p.opt_object("location")?);
    if let Some(name) = pin {
        c.info.master_id = if name.is_empty() { None } else { Some(layer_named(&p, &font, base, name)?) };
    }
    if let Some(given) = at {
        c.set_location(&location(&p, &font, given)?);
    }
    if pin.is_none() && at.is_none() {
        c.info.master_id = None;
        c.set_location(&BTreeMap::new());
    }
    if at.is_some_and(|m| !m.is_empty()) {
        // A smart component is drawn by its values, so a location would do nothing.
        if !base.info.parts.is_empty() {
            return Err(p.bad(format!("`location`: `{}` is a smart glyph; set the component's smart values instead", base.name)));
        }
        fontcraft_model::pinned::component_paths(&font, &c, &r.layer.master).map_err(|e| p.bad(format!("`location`: {e}")))?;
    }
    let layer_id = r.layer.id.clone();
    let result = json!({ "glyph": t.glyph, "layer": layer_id, "component": i, "pinned": c.info.master_id, "location": c.location() });
    if component_at(r.layer, i) == Some(&c) {
        return Ok(result);
    }
    s.edit("Select Component Instance", |f| {
        t.edit_layer(f, |layer| {
            if let Some(Shape::Component(slot)) = layer.shapes.get_mut(i) {
                slot.clone_from(&c);
            }
            Ok(())
        })
    })?;
    Ok(result)
}
