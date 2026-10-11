//! Smart glyphs and smart components (`docs/spec/09-reusing-shapes.md` § 6): a smart glyph's
//! properties, the end of each property's range its layers draw (their poles), and the property
//! values of one smart component. `fontcraft_model::smart` draws the outlines from these.
//!
//! Properties are matched by name everywhere: the poles on the smart glyph's layers and the values
//! stored on its components name the property they belong to.

use fontcraft_model::smart::MAX_SMART_POLE_LAYERS;
use fontcraft_model::{Font, Glyph, Layer, MAX_COORDINATE, MAX_NAME_LEN, MAX_PART_PROPERTIES, PartProperty, Shape, SmartPole, SmartValue};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind, Targets};

/// The range a new property gets when none is given (`smart.propertyRangeRules`).
pub const DEFAULT_MIN: f64 = 0.0;
pub const DEFAULT_MAX: f64 = 100.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "smartGlyph.addProperty",
            "Add Smart Property",
            [],
            None,
            "{glyph: name, name: string (1 to 255 bytes, no control characters, not taken by another property of the glyph), min?: number (default 0), max?: number (default 100)} add a property to the glyph, making it a smart glyph. `min` must be below `max`; negative numbers are allowed. A range from 0 to 1 is an on/off switch → {glyph, property: {name, min, max, toggle}, index}",
            has_doc,
            add_property
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "smartGlyph.setProperty",
            "Edit Smart Property",
            [],
            None,
            "{glyph: name, property: name, name?: string (new name), min?: number, max?: number} rename a property or change its range; at least one of `name`, `min`, `max`. The range after the change must keep `min` below `max`. A rename carries the layers' poles and the values stored on every component of the glyph in the font to the new name → {glyph, property: {name, min, max, toggle}, renamedValues: count}",
            has_doc,
            set_property
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "smartGlyph.removeProperty",
            "Remove Smart Property",
            [],
            None,
            "{glyph: name, property: name} delete a property and its pole on every layer of the glyph. Values stored on components stay and are ignored → {glyph, removed: name, polesRemoved: count}",
            has_doc,
            remove_property
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "smartGlyph.setLayerPole",
            "Assign Layer to Property Value",
            [],
            None,
            "{glyph: name, layer: id | name, property: name, pole: \"min\" | \"max\" | \"none\"} say whether the layer draws the property's minimum, its maximum, or is unrelated to it. A layer is found by id, else by its name (a master layer's name is its master's); a name that several layers share is refused. One master may have at most 64 layers with poles → {glyph, layer: id, property, pole}",
            has_doc,
            set_layer_pole
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "smartGlyph.listProperties",
            "List Smart Properties",
            [],
            None,
            "{glyph: name} the glyph's properties and, for every layer, the pole it holds for each property → {glyph, smart: bool (the glyph has properties), properties: [{name, min, max, toggle}], layers: [{id, name, master, poles: [{property, pole: \"min\" | \"max\"}]}]} (`toggle`: the range runs from 0 to 1, an on/off switch)",
            has_doc,
            list_properties
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "smartComponent.setValue",
            "Set Smart Component Value",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", component: index (shape index on the layer), property: name, value: number} set one property value of a smart component. Any number within ±1,000,000 is accepted: values outside the property's range extrapolate. The component's glyph must have the property; a locked component is refused → {glyph, layer, component, property, value, min, max, extrapolates: bool}",
            has_doc,
            set_value
        )
        .targets(&[TargetKind::Layer]),
    ]
}

// ── Rules ───────────────────────────────────────────────────────────────────────────────────────

/// Whether a property is an on/off switch, shown as a checkbox in place of a slider: its range
/// runs from 0 to 1 (`smart.zeroOneRangeIsToggle`).
pub fn is_toggle(p: &PartProperty) -> bool {
    p.bottom_value == 0.0 && p.top_value == 1.0
}

/// Checks a property range (`smart.propertyRangeRules`): finite numbers within the coordinate
/// limit, the minimum strictly below the maximum. Negative numbers are allowed.
pub fn check_range(min: f64, max: f64) -> std::result::Result<(), String> {
    for v in [min, max] {
        if !v.is_finite() || v.abs() > MAX_COORDINATE {
            return Err(format!("a property's range must lie within ±{MAX_COORDINATE}, got {v}"));
        }
    }
    if min >= max {
        return Err(format!("the minimum must be below the maximum, got {min} to {max}"));
    }
    Ok(())
}

/// Whether a property value extrapolates past the property's range (`smart.valuesExtrapolate`).
pub fn extrapolates(p: &PartProperty, value: f64) -> bool {
    value < p.bottom_value.min(p.top_value) || value > p.bottom_value.max(p.top_value)
}

fn property_json(p: &PartProperty) -> Value {
    json!({"name": p.name, "min": p.bottom_value, "max": p.top_value, "toggle": is_toggle(p)})
}

/// A property name read from `key`: 1 to [`MAX_NAME_LEN`] bytes, no control characters.
fn property_name<'a>(p: &Params<'a>, key: &str) -> Result<Option<&'a str>> {
    let Some(name) = p.opt_str(key)? else { return Ok(None) };
    if name.is_empty() || name.trim().is_empty() {
        return Err(p.bad(format!("`{key}` must not be empty")));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`{key}` is longer than {MAX_NAME_LEN} bytes")));
    }
    if name.chars().any(char::is_control) {
        return Err(p.bad(format!("`{key}` must not hold control characters")));
    }
    Ok(Some(name))
}

fn no_glyph(p: &Params, name: &str) -> EngineError {
    p.bad(format!("there is no glyph `{name}`"))
}

fn no_property(p: &Params, g: &Glyph, name: &str) -> EngineError {
    if g.info.parts.is_empty() {
        return p.bad(format!("glyph `{}` has no smart properties", g.name));
    }
    let names: Vec<&str> = g.info.parts.iter().map(|x| x.name.as_str()).collect();
    p.bad(format!("glyph `{}` has no property `{name}`; it has {}", g.name, names.join(", ")))
}

/// The name a layer shows in the Layers palette: its master's name for a master layer.
fn layer_name(font: &Font, l: &Layer) -> Option<String> {
    match font.master(&l.master) {
        Ok(m) if l.is_master_layer() => Some(m.name.clone()),
        _ => l.info.name.clone(),
    }
}

fn pole_str(top: bool) -> &'static str {
    if top { "max" } else { "min" }
}

// ── Properties ──────────────────────────────────────────────────────────────────────────────────

fn add_property(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("smartGlyph.addProperty", p)?;
    let glyph = p.str("glyph")?.to_string();
    let name = property_name(&p, "name")?.ok_or_else(|| p.bad("`name` is required"))?.to_string();
    let min = p.opt_f64("min")?.unwrap_or(DEFAULT_MIN);
    let max = p.opt_f64("max")?.unwrap_or(DEFAULT_MAX);
    check_range(min, max).map_err(|e| p.bad(e))?;
    let property =
        PartProperty { name: name.clone(), bottom_value: min, top_value: max, bottom_name: None, top_name: None, extra: Default::default() };
    let out = property_json(&property);
    let index = s.edit("Add Smart Property", |font| {
        let mut g = font.glyph_mut(&glyph).map_err(|_| no_glyph(&p, &glyph))?;
        crate::check_unlocked(&g)?;
        if g.info.parts.iter().any(|x| x.name == name) {
            return Err(p.bad(format!("glyph `{glyph}` already has a property `{name}`")));
        }
        if g.info.parts.len() >= MAX_PART_PROPERTIES {
            return Err(p.bad(format!("a smart glyph has at most {MAX_PART_PROPERTIES} properties")));
        }
        g.info.parts.push(property);
        Ok(g.info.parts.len().saturating_sub(1))
    })?;
    Ok(json!({"glyph": glyph, "property": out, "index": index}))
}

/// Renames the values stored for `old` on the components of `base` in `shapes`; a value already
/// stored under `new` (left from a removed property) gives way. The number renamed.
fn rename_values(shapes: &mut [Shape], base: &str, old: &str, new: &str) -> usize {
    let mut n = 0;
    for shape in shapes {
        let Shape::Component(c) = shape else { continue };
        if c.glyph != base || !c.info.smart_values.iter().any(|v| v.property == old) {
            continue;
        }
        let values = &mut c.info.smart_values;
        values.retain(|v| v.property != new);
        for v in values.iter_mut().filter(|v| v.property == old) {
            v.property = new.to_string();
            n += 1;
        }
    }
    n
}

/// Whether a layer, foreground or background, holds a component of `base` with a value for `old`.
fn layer_has_values(l: &Layer, base: &str, old: &str) -> bool {
    let has = |shapes: &[Shape]| {
        shapes.iter().any(|s| matches!(s, Shape::Component(c) if c.glyph == base && c.info.smart_values.iter().any(|v| v.property == old)))
    };
    has(&l.shapes) || l.info.background.as_ref().is_some_and(|b| has(&b.shapes))
}

/// Carries the values stored for `old` on every component of `base` in the font to `new`. Only
/// glyphs that hold such a value are copied for editing. The number renamed.
fn rename_values_in_font(font: &mut Font, base: &str, old: &str, new: &str) -> usize {
    let users: Vec<usize> =
        font.glyphs.as_arcs().iter().enumerate().filter(|(_, g)| g.layers.iter().any(|l| layer_has_values(l, base, old))).map(|(i, _)| i).collect();
    let mut n = 0usize;
    for i in users {
        let Some(mut g) = font.glyphs.get_mut_at(i) else { continue };
        for l in g.layers.iter_mut() {
            n = n.saturating_add(rename_values(&mut l.shapes, base, old, new));
            if l.info.background.is_some()
                && let Some(b) = l.info.background.as_mut()
            {
                n = n.saturating_add(rename_values(&mut b.shapes, base, old, new));
            }
        }
    }
    n
}

fn set_property(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("smartGlyph.setProperty", p)?;
    let glyph = p.str("glyph")?.to_string();
    let old = p.str("property")?.to_string();
    let new_name = property_name(&p, "name")?.map(str::to_string);
    let (min, max) = (p.opt_f64("min")?, p.opt_f64("max")?);
    if new_name.is_none() && min.is_none() && max.is_none() {
        return Err(p.bad("give at least one of `name`, `min` and `max`"));
    }
    let (property, renamed) = s.edit("Edit Smart Property", |font| {
        let property = {
            let mut g = font.glyph_mut(&glyph).map_err(|_| no_glyph(&p, &glyph))?;
            crate::check_unlocked(&g)?;
            let i = g.info.parts.iter().position(|x| x.name == old).ok_or_else(|| no_property(&p, &g, &old))?;
            if let Some(new) = &new_name
                && *new != old
                && g.info.parts.iter().any(|x| x.name == *new)
            {
                return Err(p.bad(format!("glyph `{glyph}` already has a property `{new}`")));
            }
            let parts = &mut g.info.parts;
            let part = parts.get_mut(i).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no property `{old}`")))?;
            let (lo, hi) = (min.unwrap_or(part.bottom_value), max.unwrap_or(part.top_value));
            check_range(lo, hi).map_err(|e| p.bad(e))?;
            part.bottom_value = lo;
            part.top_value = hi;
            if let Some(new) = &new_name {
                part.name = new.clone();
            }
            let property = part.clone();
            if let Some(new) = new_name.as_ref().filter(|n| **n != old) {
                for l in g.layers.iter_mut() {
                    if !l.info.attrs.smart_poles.iter().any(|x| x.property == old) {
                        continue;
                    }
                    for pole in l.info.attrs.smart_poles.iter_mut().filter(|x| x.property == old) {
                        pole.property = new.clone();
                    }
                }
            }
            property
        };
        let renamed = match new_name.as_ref().filter(|n| **n != old) {
            Some(new) => rename_values_in_font(font, &glyph, &old, new),
            None => 0,
        };
        Ok((property, renamed))
    })?;
    Ok(json!({"glyph": glyph, "property": property_json(&property), "renamedValues": renamed}))
}

fn remove_property(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("smartGlyph.removeProperty", p)?;
    let glyph = p.str("glyph")?.to_string();
    let name = p.str("property")?.to_string();
    let poles = s.edit("Remove Smart Property", |font| {
        let mut g = font.glyph_mut(&glyph).map_err(|_| no_glyph(&p, &glyph))?;
        crate::check_unlocked(&g)?;
        let i = g.info.parts.iter().position(|x| x.name == name).ok_or_else(|| no_property(&p, &g, &name))?;
        g.info.parts.remove(i);
        let mut poles = 0usize;
        for l in g.layers.iter_mut() {
            let before = l.info.attrs.smart_poles.len();
            if !l.info.attrs.smart_poles.iter().any(|x| x.property == name) {
                continue;
            }
            l.info.attrs.smart_poles.retain(|x| x.property != name);
            poles = poles.saturating_add(before.saturating_sub(l.info.attrs.smart_poles.len()));
        }
        Ok(poles)
    })?;
    Ok(json!({"glyph": glyph, "removed": name, "polesRemoved": poles}))
}

// ── Layers ──────────────────────────────────────────────────────────────────────────────────────

/// The index in `g.layers` of the layer `key` names: by id, else by its palette name, which must
/// be unique.
fn find_layer(p: &Params, font: &Font, g: &Glyph, key: &str) -> Result<usize> {
    if let Some(i) = g.layers.iter().position(|l| l.id == key) {
        return Ok(i);
    }
    let mut named = g.layers.iter().enumerate().filter(|(_, l)| layer_name(font, l).as_deref() == Some(key)).map(|(i, _)| i);
    match (named.next(), named.next()) {
        (Some(i), None) => Ok(i),
        (Some(_), Some(_)) => Err(p.bad(format!("glyph `{}` has several layers named `{key}`: give the layer's id", g.name))),
        (None, _) => Err(p.bad(format!("glyph `{}` has no layer with the id or name `{key}`", g.name))),
    }
}

fn set_layer_pole(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("smartGlyph.setLayerPole", p)?;
    let glyph = p.str("glyph")?.to_string();
    let key = p.str("layer")?.to_string();
    let property = p.str("property")?.to_string();
    let pole = match p.str("pole")? {
        "min" => Some(false),
        "max" => Some(true),
        "none" => None,
        other => return Err(p.bad(format!("`pole` must be \"min\", \"max\" or \"none\", got \"{other}\""))),
    };
    let id = s.edit("Assign Layer to Property Value", |font| {
        let (index, master) = {
            let g = font.glyph(&glyph).ok_or_else(|| no_glyph(&p, &glyph))?;
            crate::check_unlocked(g)?;
            if !g.info.parts.iter().any(|x| x.name == property) {
                return Err(no_property(&p, g, &property));
            }
            let i = find_layer(&p, font, g, &key)?;
            let layer = g.layers.get(i).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{key}`")))?;
            if pole.is_some() && layer.info.attrs.smart_poles.is_empty() {
                let poles = g.layers.iter().filter(|l| l.master == layer.master && !l.info.attrs.smart_poles.is_empty()).count();
                if poles >= MAX_SMART_POLE_LAYERS {
                    return Err(p.bad(format!("master `{}` already has {MAX_SMART_POLE_LAYERS} smart layers in glyph `{glyph}`", layer.master)));
                }
            }
            (i, layer.master.clone())
        };
        let mut g = font.glyph_mut(&glyph).map_err(|_| no_glyph(&p, &glyph))?;
        let layer = g.layers.get_mut(index).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{key}` in master `{master}`")))?;
        let current = layer.info.attrs.smart_poles.iter().find(|x| x.property == property).map(|x| x.top);
        if current == pole {
            return Ok(layer.id.clone());
        }
        let poles = &mut layer.info.attrs.smart_poles;
        match (poles.iter_mut().find(|x| x.property == property), pole) {
            (Some(x), Some(top)) => x.top = top,
            (Some(_), None) => poles.retain(|x| x.property != property),
            (None, Some(top)) => poles.push(SmartPole { property: property.clone(), top }),
            (None, None) => {}
        }
        Ok(layer.id.clone())
    })?;
    Ok(json!({"glyph": glyph, "layer": id, "property": property, "pole": pole.map_or("none", pole_str)}))
}

fn list_properties(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("smartGlyph.listProperties", p)?;
    let name = p.str("glyph")?;
    let font = s.font()?;
    let g = font.glyph(name).ok_or_else(|| no_glyph(&p, name))?;
    let properties: Vec<Value> = g.info.parts.iter().map(property_json).collect();
    let layers: Vec<Value> = g
        .layers
        .iter()
        .map(|l| {
            let poles: Vec<Value> = l.info.attrs.smart_poles.iter().map(|x| json!({"property": x.property, "pole": pole_str(x.top)})).collect();
            json!({"id": l.id, "name": layer_name(font, l), "master": l.master, "poles": poles})
        })
        .collect();
    Ok(json!({"glyph": g.name, "smart": !g.info.parts.is_empty(), "properties": properties, "layers": layers}))
}

// ── Components ──────────────────────────────────────────────────────────────────────────────────

fn set_value(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("smartComponent.setValue", p)?;
    let t = Targets::parse(&p)?;
    let index = p.opt_int("component", 0, i64::from(u32::MAX))?.ok_or_else(|| p.bad("`component` is required"))?;
    let index = usize::try_from(index).map_err(|_| p.bad("`component` is out of range"))?;
    let property = p.str("property")?.to_string();
    let value = p.f64("value")?;
    if value.abs() > MAX_COORDINATE {
        return Err(p.bad(format!("`value` must lie within ±{MAX_COORDINATE}, got {value}")));
    }
    let (layer, part) = s.edit("Set Smart Component Value", |font| {
        let (layer, part) = {
            let r = t.resolve(font)?;
            let c = match r.parts.shapes.get(index) {
                Some(Shape::Component(c)) => c,
                Some(_) => return Err(p.bad(format!("shape {index} of glyph `{}` is not a component", t.glyph))),
                None => return Err(p.bad(format!("`component`: {index} does not resolve: the layer has {} shapes", r.parts.shapes.len()))),
            };
            if c.info.locked {
                return Err(p.bad(format!("component {index} is locked: unlock it first")));
            }
            let base = font.glyph(&c.glyph).ok_or_else(|| p.bad(format!("component {index} refers to `{}`, which is not in the font", c.glyph)))?;
            let part = base.info.parts.iter().find(|x| x.name == property).cloned().ok_or_else(|| no_property(&p, base, &property))?;
            (r.layer.id.clone(), part)
        };
        t.edit_layer(font, |l| {
            let Some(Shape::Component(c)) = l.parts_mut(t.side).shapes.get_mut(index) else {
                return Err(p.bad(format!("shape {index} of glyph `{}` is not a component", t.glyph)));
            };
            let values = &mut c.info.smart_values;
            if let Some(v) = values.iter_mut().find(|v| v.property == property) {
                v.value = value;
            } else if values.len() >= MAX_PART_PROPERTIES {
                return Err(p.bad(format!("component {index} already stores {MAX_PART_PROPERTIES} property values")));
            } else {
                values.push(SmartValue { property: property.clone(), value });
            }
            Ok(())
        })?;
        Ok((layer, part))
    })?;
    Ok(json!({
        "glyph": t.glyph,
        "layer": layer,
        "component": index,
        "property": property,
        "value": value,
        "min": part.bottom_value,
        "max": part.top_value,
        "extrapolates": extrapolates(&part, value),
    }))
}
