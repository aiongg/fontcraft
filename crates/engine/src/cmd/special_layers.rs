//! Special layer commands (M9.16): intermediate and alternate layers, Re-Interpolate, shape
//! order.
//!
//! Rows: `layers.setIntermediate`, `layers.clearIntermediate`, `layers.setAlternate`,
//! `layers.clearAlternate`, `layers.reInterpolate`, `glyph.setShapeOrder`, `glyph.swapShapes`.
//! The labels these layers are shown with are `fontcraft_interp::labels` (same package).
//!
//! Coordinates and ranges are internal (design) coordinates, keyed by axis tag. A change that
//! would leave a glyph unable to interpolate (two sources at one location, for one) is refused,
//! unless the glyph could not interpolate before it either.

use fontcraft_interp::instance::interpolate_layer;
use fontcraft_interp::labels::special_label;
use fontcraft_interp::{Designspace, GlyphSources, InterpOptions, Location, ModelKind, Source};
use fontcraft_model::{AxisRule, Font, Glyph, Layer, Shape};
use serde_json::{Map, Value, json};

use super::common::{Outcome, in_range, scope};
use super::*;
use crate::EngineError;
use crate::params::{MAX_TARGETS, Params, TargetKind, Targets, kind};

/// How far, relative to the value, a coordinate may lie beyond the masters' range and still count
/// as on its end (rounding in stored files).
const RANGE_SLACK: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layers.setIntermediate",
            "Intermediate",
            [],
            None,
            "{glyph: name, layer: id, coordinates: {tag: number | null}} make a layer that is not a master layer an intermediate layer at the given internal coordinates. An axis left out or set to null takes the coordinate of the layer's master. Each coordinate must lie within the range of the masters and virtual masters on its axis, and the layer must not land where another layer of the glyph already interpolates from. An alternate layer stays alternate as well → {glyph, layer, coordinates: {tag: number | null}, label}",
            has_doc,
            set_intermediate
        )
        .targets(&[TargetKind::Layer])
        .context(&["layers.row"]),
        cmd!(
            "layers.clearIntermediate",
            "Remove Intermediate Status",
            [],
            None,
            "{glyph: name, layer: id} remove an intermediate layer's coordinates, leaving a backup layer (or an alternate layer, when it has ranges too) → {glyph, layer, label}",
            has_doc,
            clear_intermediate
        )
        .targets(&[TargetKind::Layer])
        .context(&["layers.row"]),
        cmd!(
            "layers.setAlternate",
            "Alternate",
            [],
            None,
            "{glyph: name, layer: id, ranges: {tag: {min?: number | null, max?: number | null}}} make a layer an alternate layer that replaces its master's layer where every axis lies within its range, both bounds included. A missing or null bound is open; an axis left out has no bounds; `{}` makes the `[]` partner of a master layer that carries a range. Master layers take a range too, but not `{}`. An intermediate layer keeps its coordinates → {glyph, layer, ranges: {tag: {min, max}}, label}",
            has_doc,
            set_alternate
        )
        .targets(&[TargetKind::Layer])
        .context(&["layers.row"]),
        cmd!(
            "layers.clearAlternate",
            "Remove Alternate Status",
            [],
            None,
            "{glyph: name, layer: id} remove a layer's alternate ranges, leaving an ordinary master or backup layer (or an intermediate layer, when it has coordinates too) → {glyph, layer, label}",
            has_doc,
            clear_alternate
        )
        .targets(&[TargetKind::Layer])
        .context(&["layers.row"]),
        cmd!(
            "layers.reInterpolate",
            "Re-Interpolate",
            [],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), location?: {tag: number}} replace each layer's outline, anchors and width with the interpolation of the glyph's other layers at the layer's location: a master layer's master, an intermediate layer's coordinates, an alternate layer's place among the layers of its range. The layer itself is left out of the sources. `location` overrides axes of that location; a backup layer has none and needs `location`, the axes left out taking its master's values. Locations beyond the masters extrapolate. With `glyphs`, a glyph or layer that cannot be interpolated is skipped and named; with `glyph`, it is an error → {glyphs: [names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            re_interpolate
        )
        .targets(&[TargetKind::Layer])
        .context(&["layers.row"]),
        cmd!(
            "glyph.setShapeOrder",
            "Set Shape Order",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), order: [index] (every shape index of the layer once, in the new order)} reorder the layer's shapes, so that shapes at the same index match across the glyph's layers for interpolation. A list that is not a permutation of the layer's shape indices is refused → {glyph, layer, order: [index]}",
            has_doc,
            set_shape_order
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "glyph.swapShapes",
            "Swap Shapes",
            [],
            None,
            "{glyph: name, layer: id, a: index, b: index} exchange two shapes in the layer's shape order, as dragging one connecting line's end onto another's does in the Master Compatibility view. `a` and `b` must be different shapes of the layer → {glyph, layer, a, b}",
            has_doc,
            swap_shapes
        )
        .targets(&[TargetKind::Layer]),
    ]
}

// ── Helpers ─────────────────────────────────────────────────────────────────────────────────────

fn find_layer<'f>(p: &Params, font: &'f Font, glyph: &str, id: &str) -> Result<(&'f Glyph, &'f Layer)> {
    let g = font.glyph(glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
    let layer = g.layers.iter().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{id}`")))?;
    Ok((g, layer))
}

/// The font axis tagged `tag`.
fn font_axis(p: &Params, font: &Font, key: &str, tag: &str) -> Result<usize> {
    font.axes.iter().position(|a| a.tag == tag).ok_or_else(|| {
        let tags: Vec<&str> = font.axes.iter().map(|a| a.tag.as_str()).collect();
        p.bad(format!("`{key}` names the axis `{tag}`, which the font does not have (its axes: {})", tags.join(", ")))
    })
}

/// The designspace axis of font axis `i`.
fn designspace_axis(designspace: &Designspace, i: usize) -> Option<usize> {
    designspace.axes().iter().position(|a| a.font_axis == i)
}

/// A number from `v`: finite and within the model's coordinate limit.
fn coordinate(p: &Params, what: &str, v: &Value) -> Result<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|x| in_range(*x)).ok_or_else(|| p.bad(format!("`{what}` is not a usable number"))),
        other => Err(p.bad(format!("`{what}` must be a number, found {}", kind(other)))),
    }
}

/// `None` for null, else [`coordinate`].
fn opt_coordinate(p: &Params, what: &str, v: Option<&Value>) -> Result<Option<f64>> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => coordinate(p, what, v).map(Some),
    }
}

/// Whether the sources of `glyph` can be read.
fn sources_ok(font: &Font, glyph: &str) -> std::result::Result<(), String> {
    let designspace = Designspace::new(font).map_err(|e| e.to_string())?;
    GlyphSources::of(font, &designspace, glyph).map(|_| ()).map_err(|e| e.to_string())
}

/// Changes one layer's special attributes as one undo step. The change is refused when the
/// glyph's sources could be read before it and cannot after it.
fn edit_attrs(s: &mut Session, p: &Params, label: &str, glyph: &str, id: &str, f: impl FnOnce(&mut Layer)) -> Result<Option<String>> {
    let readable_before = sources_ok(s.font()?, glyph).is_ok();
    s.edit(label, |font| {
        {
            let mut g = font.glyph_mut(glyph).map_err(|_| p.bad(format!("there is no glyph `{glyph}`")))?;
            crate::check_unlocked(&g)?;
            let layer = g.layers.iter_mut().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{id}`")))?;
            f(layer);
        }
        if readable_before && let Err(reason) = sources_ok(font, glyph) {
            return Err(p.bad(format!("refused: glyph `{glyph}` could no longer interpolate: {reason}")));
        }
        let g = font.glyph(glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
        Ok(g.layers.iter().find(|l| l.id == id).and_then(|l| special_label(font, l)))
    })
}

// ── Intermediate layers ─────────────────────────────────────────────────────────────────────────

fn set_intermediate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.setIntermediate", value)?;
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("layer")?.to_string();
    let given = p.opt_object("coordinates")?.ok_or_else(|| p.bad("`coordinates` is required: {tag: number or null}"))?;
    let font = s.font()?;
    if font.axes.is_empty() {
        return Err(p.bad("the font has no axes: an intermediate layer needs at least one"));
    }
    let (_, layer) = find_layer(&p, font, &glyph, &id)?;
    if layer.is_master_layer() {
        return Err(p.bad("a master layer cannot be an intermediate layer: add a layer under the master and make that one intermediate"));
    }
    let designspace = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
    let mut coords: Vec<Option<f64>> = vec![None; font.axes.len()];
    for (tag, v) in given {
        let what = format!("coordinates.{tag}");
        let i = font_axis(&p, font, "coordinates", tag)?;
        let value = opt_coordinate(&p, &what, Some(v))?;
        if let (Some(x), Some(d)) = (value, designspace_axis(&designspace, i)) {
            let values = designspace.masters().iter().map(|m| &m.location).chain(designspace.virtual_masters().iter().map(|v| &v.location));
            let (lo, hi) = values.filter_map(|l| l.0.get(d).copied()).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
            let slack = RANGE_SLACK * (1.0 + x.abs());
            if lo <= hi && (x < lo - slack || x > hi + slack) {
                return Err(p.bad(format!("`{what}` is {x}, outside the range of the masters on that axis ({lo} to {hi})")));
            }
        }
        if let Some(slot) = coords.get_mut(i) {
            *slot = value;
        }
    }
    let echo: Map<String, Value> = font.axes.iter().zip(&coords).map(|(a, c)| (a.tag.clone(), json!(c))).collect();
    let label = edit_attrs(s, &p, "Intermediate", &glyph, &id, |layer| layer.info.attrs.coordinates = coords)?;
    Ok(json!({"glyph": glyph, "layer": id, "coordinates": echo, "label": label}))
}

fn clear_intermediate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.clearIntermediate", value)?;
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("layer")?.to_string();
    let (_, layer) = find_layer(&p, s.font()?, &glyph, &id)?;
    if layer.info.attrs.coordinates.is_empty() {
        return Err(p.bad(format!("layer `{id}` of glyph `{glyph}` is not an intermediate layer")));
    }
    let label = edit_attrs(s, &p, "Remove Intermediate Status", &glyph, &id, |layer| layer.info.attrs.coordinates.clear())?;
    Ok(json!({"glyph": glyph, "layer": id, "label": label}))
}

// ── Alternate layers ────────────────────────────────────────────────────────────────────────────

fn set_alternate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.setAlternate", value)?;
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("layer")?.to_string();
    let given = p.opt_object("ranges")?.ok_or_else(|| p.bad("`ranges` is required: {tag: {min?, max?}}, or {} for the `[]` partner"))?;
    let font = s.font()?;
    if font.axes.is_empty() {
        return Err(p.bad("the font has no axes: an alternate layer needs at least one"));
    }
    let (_, layer) = find_layer(&p, font, &glyph, &id)?;
    let mut rules = vec![AxisRule::default(); font.axes.len()];
    for (tag, v) in given {
        let i = font_axis(&p, font, "ranges", tag)?;
        let Value::Object(range) = v else {
            return Err(p.bad(format!("`ranges.{tag}` must be an object {{min?, max?}}, found {}", kind(v))));
        };
        if let Some(key) = range.keys().find(|k| *k != "min" && *k != "max") {
            return Err(p.bad(format!("`ranges.{tag}` takes `min` and `max`, not `{key}`")));
        }
        let min = opt_coordinate(&p, &format!("ranges.{tag}.min"), range.get("min"))?;
        let max = opt_coordinate(&p, &format!("ranges.{tag}.max"), range.get("max"))?;
        if let (Some(lo), Some(hi)) = (min, max)
            && lo > hi
        {
            return Err(p.bad(format!("`ranges.{tag}` runs from {lo} down to {hi}: the minimum must not be above the maximum")));
        }
        if let Some(slot) = rules.get_mut(i) {
            *slot = AxisRule { min, max };
        }
    }
    let bounded = rules.iter().any(|r| r.min.is_some() || r.max.is_some());
    if layer.is_master_layer() && !bounded {
        return Err(p.bad("a master layer needs a range: the `[]` partner is another layer under the master"));
    }
    let echo: Map<String, Value> = font.axes.iter().zip(&rules).map(|(a, r)| (a.tag.clone(), json!({"min": r.min, "max": r.max}))).collect();
    let label = edit_attrs(s, &p, "Alternate", &glyph, &id, |layer| layer.info.attrs.axis_rules = rules)?;
    Ok(json!({"glyph": glyph, "layer": id, "ranges": echo, "label": label}))
}

fn clear_alternate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.clearAlternate", value)?;
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("layer")?.to_string();
    let (_, layer) = find_layer(&p, s.font()?, &glyph, &id)?;
    if layer.info.attrs.axis_rules.is_empty() {
        return Err(p.bad(format!("layer `{id}` of glyph `{glyph}` is not an alternate layer")));
    }
    let label = edit_attrs(s, &p, "Remove Alternate Status", &glyph, &id, |layer| layer.info.attrs.axis_rules.clear())?;
    Ok(json!({"glyph": glyph, "layer": id, "label": label}))
}

// ── Re-Interpolate ──────────────────────────────────────────────────────────────────────────────

/// The `location` parameter: (designspace axis, internal coordinate) pairs.
fn location_overrides(p: &Params, font: &Font, designspace: &Designspace) -> Result<Vec<(usize, f64)>> {
    let Some(given) = p.opt_object("location")? else { return Ok(Vec::new()) };
    let mut out = Vec::with_capacity(given.len().min(font.axes.len()));
    for (tag, v) in given {
        let i = font_axis(p, font, "location", tag)?;
        let d = designspace_axis(designspace, i).ok_or_else(|| p.bad(format!("the axis `{tag}` does not take part in interpolation")))?;
        out.push((d, coordinate(p, &format!("location.{tag}"), v)?));
    }
    Ok(out)
}

/// The variation model the font's Font Type setting asks for.
fn interp_options(font: &Font) -> InterpOptions {
    let model = if font.info.settings.font_type.as_deref() == Some("variable") { ModelKind::Variable } else { ModelKind::Default };
    InterpOptions { model, ..InterpOptions::default() }
}

/// Layer `id` of `glyph` with its drawing replaced by the interpolation of the glyph's other
/// sources at its location; the reason when it cannot be.
fn reinterpolated(
    font: &Font,
    designspace: &Designspace,
    glyph: &Glyph,
    id: &str,
    given: &[(usize, f64)],
    options: &InterpOptions,
) -> std::result::Result<Layer, String> {
    let layer = glyph.layers.iter().find(|l| l.id == id).ok_or_else(|| "the glyph has no such layer".to_string())?;
    let a = &layer.info.attrs;
    if a.color_palette.is_some() || a.full_color || a.svg || a.sbix_size.is_some() || !a.smart_poles.is_empty() {
        return Err("colour layers and smart-glyph layers are not interpolated".into());
    }
    let sources = GlyphSources::of(font, designspace, &glyph.name).map_err(|e| e.to_string())?;
    let without = |list: &[Source]| -> Vec<Source> { list.iter().filter(|s| s.layer_id != id).cloned().collect() };
    let only = |main: Vec<Source>| GlyphSources { glyph: glyph.name.clone(), main, ..GlyphSources::default() };
    let in_set = sources.alternates.iter().find_map(|set| set.sources.iter().find(|s| s.layer_id == id).map(|s| (set, s)));
    let (base, others) = if let Some(src) = sources.main.iter().find(|s| s.layer_id == id) {
        (src.location.clone(), only(without(&sources.main)))
    } else if let Some((set, src)) = in_set {
        (src.location.clone(), only(without(&set.sources)))
    } else {
        if given.is_empty() {
            return Err("a backup layer has no location of its own: give `location`".into());
        }
        let master = designspace.master(&layer.master).ok_or_else(|| format!("the layer's master `{}` does not interpolate", layer.master))?;
        (master.location.clone(), sources.clone())
    };
    let mut location: Location = base;
    for &(i, v) in given {
        if let Some(slot) = location.0.get_mut(i) {
            *slot = v;
        }
    }
    if others.main.is_empty() {
        return Err("the glyph has no other layer to interpolate from".into());
    }
    let new = interpolate_layer(font, &others, &location, options).map_err(|e| e.to_string())?;
    let mut out = layer.clone();
    out.width = new.width;
    out.shapes = new.shapes;
    out.anchors = new.anchors;
    out.info.vert_width = new.info.vert_width;
    out.info.vert_origin = new.info.vert_origin;
    Ok(out)
}

fn re_interpolate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.reInterpolate", value)?;
    let font = s.font()?;
    let scope = scope(&p, font)?;
    let designspace = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
    let given = location_overrides(&p, font, &designspace)?;
    let options = interp_options(font);
    let mut out = Outcome::default();
    let mut replaced: Vec<(String, Layer)> = Vec::new();
    for name in &scope.glyphs {
        let g = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        if g.info.locked {
            if scope.single {
                return Err(EngineError::Locked(name.clone()));
            }
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        let mut changed = false;
        for id in &scope.layers {
            match reinterpolated(font, &designspace, g, id, &given, &options) {
                Ok(layer) => {
                    replaced.push((name.clone(), layer));
                    changed = true;
                }
                Err(reason) if scope.single => return Err(p.bad(format!("layer `{id}` of glyph `{name}`: {reason}"))),
                Err(reason) => out.skip(name, Some(id.as_str()), reason),
            }
        }
        if changed {
            out.glyphs.push(name.clone());
        }
    }
    if !replaced.is_empty() {
        s.edit("Re-Interpolate", |font| {
            for (name, layer) in replaced {
                let mut g = font.glyph_mut(&name)?;
                if let Some(slot) = g.layers.iter_mut().find(|l| l.id == layer.id) {
                    *slot = layer;
                }
            }
            Ok(())
        })?;
    }
    Ok(out.to_json())
}

// ── Shape order ─────────────────────────────────────────────────────────────────────────────────

/// A shape index read from `v`.
fn shape_index(p: &Params, key: &str, v: &Value) -> Result<usize> {
    v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad(format!("`{key}` must hold whole numbers from 0, found {v}")))
}

fn set_shape_order(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("glyph.setShapeOrder", value)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes `order`, not target lists"));
    }
    let list = p.opt_array("order")?.ok_or_else(|| p.bad("`order` is required: every shape index once, in the new order"))?;
    if list.len() > MAX_TARGETS {
        return Err(p.bad(format!("`order` holds {} entries; at most {MAX_TARGETS} are allowed", list.len())));
    }
    let order: Vec<usize> = list.iter().map(|v| shape_index(&p, "order", v)).collect::<Result<_>>()?;
    let count = t.resolve(s.font()?)?.parts.shapes.len();
    let mut seen = vec![false; count];
    for &i in &order {
        match seen.get_mut(i) {
            Some(slot) if !*slot => *slot = true,
            Some(_) => return Err(p.bad(format!("`order` is not a permutation: it names shape {i} twice"))),
            None => return Err(p.bad(format!("`order` is not a permutation: shape {i} does not resolve, the layer has {count} shapes"))),
        }
    }
    if order.len() != count {
        return Err(p.bad(format!("`order` is not a permutation: it must name all {count} shapes of the layer, got {}", order.len())));
    }
    s.edit("Set Shape Order", |font| {
        t.edit_layer(font, |layer| {
            let shapes = layer.parts_mut(t.side).shapes;
            let mut slots: Vec<Option<Shape>> = std::mem::take(shapes).into_iter().map(Some).collect();
            for &i in &order {
                let shape = slots.get_mut(i).and_then(Option::take).ok_or_else(|| p.bad(format!("shape {i} does not resolve")))?;
                shapes.push(shape);
            }
            Ok(())
        })
    })?;
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "order": order}))
}

fn swap_shapes(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("glyph.swapShapes", value)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes `a` and `b`, not target lists"));
    }
    let a = shape_index(&p, "a", p.get("a").ok_or_else(|| p.bad("`a` is required"))?)?;
    let b = shape_index(&p, "b", p.get("b").ok_or_else(|| p.bad("`b` is required"))?)?;
    let count = t.resolve(s.font()?)?.parts.shapes.len();
    if let Some(i) = [a, b].into_iter().find(|i| *i >= count) {
        return Err(p.bad(format!("shape {i} does not resolve: the layer has {count} shapes")));
    }
    if a == b {
        return Err(p.bad(format!("`a` and `b` are both shape {a}: there is nothing to swap")));
    }
    s.edit("Swap Shapes", |font| {
        t.edit_layer(font, |layer| {
            let shapes = layer.parts_mut(t.side).shapes;
            if a.max(b) >= shapes.len() {
                return Err(p.bad("the layer changed: a shape does not resolve"));
            }
            shapes.swap(a, b);
            Ok(())
        })
    })?;
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "a": a, "b": b}))
}
