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

use std::collections::HashSet;

use fontcraft_hinting::Renumbered;
use fontcraft_interp::instance::interpolate_layer;
use fontcraft_interp::labels::{layer_title, special_label};
use fontcraft_interp::sources::set_inactive;
use fontcraft_interp::{Designspace, GlyphSources, InterpOptions, Location, ModelKind, RangeSet, Source};
use fontcraft_model::{AxisRule, Font, Glyph, Layer, LayerAttrs, LayerSide, Shape};
use serde_json::{Map, Value, json};

use super::common::{Outcome, in_range, layer_ids, scope};
use super::shared::renumber_hints;
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
            "{glyph?: name, layer?: id | layers?: [ids of the glyph], allMasters?: bool (default false), glyphs?: [names] (default: the glyph), coordinates: {tag: number | null}} make layers that are not master layers intermediate layers at the given internal coordinates. An axis left out or set to null takes the coordinate of the layer's master. Each coordinate must lie within the range of the masters and virtual masters on its axis, and no layer may land where another layer of the glyph already interpolates from. An alternate layer stays alternate as well. With `allMasters`, each master of the `glyphs` whose own coordinates complete the given ones to a place the glyph has no source at also gets an intermediate layer, a copy of its master layer; `glyph` and `layer` can then be left out. One undo step → {glyph, layer, coordinates: {tag: number | null}, label, layers: [{glyph, layer, label, added: bool}]}; `glyph`, `layer` and `label` are the first's",
            has_doc,
            set_intermediate
        )
        .targets(&[TargetKind::Layer])
        .context(&["layers.row"]),
        cmd!(
            "layers.addIntermediate",
            "Add Source",
            [],
            None,
            "{glyph: name, location: {tag: number}, master?: id (default: the nearest master the glyph has a layer for)} add an intermediate layer at a location in internal coordinates, drawn as the glyph interpolates there; an axis left out takes the master's coordinate. The location must lie within the range of the masters and virtual masters, and no other layer of the glyph may interpolate from it → {glyph, layer, master, coordinates: {tag: number}, label}",
            has_doc,
            add_intermediate
        ),
        cmd!(
            "layers.clearIntermediate",
            "Remove Intermediate Status",
            [],
            None,
            "{glyph: name, layer: id | layers: [ids]} remove the coordinates of the intermediate layers among these, leaving backup layers (or alternate layers, where they have ranges too); refused when none is one → {glyph, layer, label, layers: [{glyph, layer, label, added: false}]}",
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
            "{glyph?: name, layer?: id | layers?: [ids of the glyph], allMasters?: bool (default false), glyphs?: [names] (default: the glyph), ranges: {tag: {min?: number | null, max?: number | null}}} make layers alternate layers that replace their masters' layers where every axis lies within its range, both bounds included. A missing or null bound is open; an axis left out has no bounds; `{}` makes the `[]` partner of a master layer that carries a range. Master layers take a range too, but not `{}`. An intermediate layer keeps its coordinates. With `allMasters`, each master of the `glyphs` without a layer of that range at its own place also gets one, a copy of its master layer (with `{}`: each master whose layer carries a range and has no `[]` partner); `glyph` and `layer` can then be left out. One undo step → {glyph, layer, ranges: {tag: {min, max}}, label, layers: [{glyph, layer, label, added: bool}]}; `glyph`, `layer` and `label` are the first's",
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
            "{glyph: name, layer: id | layers: [ids]} remove the ranges of the alternate layers among these, leaving ordinary master or backup layers (or intermediate layers, where they have coordinates too); refused when none is one → {glyph, layer, label, layers: [{glyph, layer, label, added: false}]}",
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
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", order: [index] (every shape index of the layer once, in the new order)} reorder the layer's shapes (paths and components), so that shapes at the same index match across the glyph's layers for interpolation. Hints follow the paths they point at. A list that is not a permutation of the layer's shape indices is refused → {glyph, layer, order: [index]}",
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

/// What a command that gives or takes a special status acts on.
struct SpecialTargets {
    /// The glyph whose `layers` are named.
    glyph: Option<String>,
    layers: Vec<String>,
    /// The glyphs whose masters get a layer (`allMasters`); empty without it.
    all_masters_of: Vec<String>,
}

/// Reads `glyph` with `layer` or `layers` and, where the command `adds` layers, `allMasters` with `glyphs`. Every layer
/// named is one of the glyph's.
fn special_targets(p: &Params, font: &Font, adds: bool) -> Result<SpecialTargets> {
    let glyph = p.opt_str("glyph")?.map(str::to_string);
    if let Some(name) = &glyph
        && font.glyph(name).is_none()
    {
        return Err(p.bad(format!("there is no glyph `{name}`")));
    }
    let named = p.get("layer").is_some() || p.get("layers").is_some();
    let layers = if named { layer_ids(p, font)? } else { Vec::new() };
    let all_masters = adds && p.bool_or("allMasters", false)?;
    if named || !all_masters {
        let name = glyph.as_deref().ok_or_else(|| p.bad("`glyph` is required"))?;
        if layers.is_empty() && !all_masters {
            return Err(p.bad("`layer` or `layers` is required"));
        }
        for id in &layers {
            find_layer(p, font, name, id)?;
        }
    }
    let all_masters_of = if all_masters {
        match p.opt_glyphs("glyphs", font)? {
            Some(glyphs) => glyphs,
            None => vec![glyph.clone().ok_or_else(|| p.bad("`glyph` or `glyphs` is required"))?],
        }
    } else {
        Vec::new()
    };
    Ok(SpecialTargets { glyph, layers, all_masters_of })
}

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
    sources_in(font, &designspace, glyph)
}

/// [`sources_ok`] in a designspace already read.
fn sources_in(font: &Font, designspace: &Designspace, glyph: &str) -> std::result::Result<(), String> {
    GlyphSources::of(font, designspace, glyph).map(|_| ()).map_err(|e| e.to_string())
}

/// What changes in one glyph: the layers that take the change, and layers added to it.
struct Change {
    glyph: String,
    set: Vec<String>,
    add: Vec<Layer>,
}

/// Changes special attributes as one undo step: `f` on each layer of every change, then its new layers after the last
/// of their master. Refused when the sources of a glyph could be read before it and cannot after it. → a row per layer.
fn edit_attrs(s: &mut Session, p: &Params, label: &str, changes: Vec<Change>, f: impl Fn(&mut Layer)) -> Result<Vec<Value>> {
    // Attributes of layers move no axis and no master, so the designspace stays as it is.
    let designspace = Designspace::new(s.font()?);
    let readable = |font: &Font, glyph: &str| match &designspace {
        Ok(d) => sources_in(font, d, glyph),
        Err(e) => Err(e.to_string()),
    };
    s.edit(label, |font| {
        let mut rows = Vec::new();
        for Change { glyph, set, add } in changes {
            let readable_before = readable(font, &glyph).is_ok();
            let mut done: Vec<(String, bool)> = Vec::with_capacity(set.len().saturating_add(add.len()));
            {
                let mut g = font.glyph_mut(&glyph).map_err(|_| p.bad(format!("there is no glyph `{glyph}`")))?;
                crate::check_unlocked(&g)?;
                if !add.is_empty() && g.layers.len().saturating_add(add.len()) > super::layers::MAX_LAYERS {
                    let most = super::layers::MAX_LAYERS;
                    return Err(p.bad(format!("glyph `{glyph}` has {} layers; at most {most} are allowed", g.layers.len())));
                }
                for id in set {
                    let layer = g.layers.iter_mut().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{id}`")))?;
                    f(layer);
                    done.push((id, false));
                }
                for layer in add {
                    done.push((layer.id.clone(), true));
                    let at = g.layers.iter().rposition(|l| l.master == layer.master).map_or(g.layers.len(), |i| i.saturating_add(1));
                    g.layers.insert(at, layer);
                }
            }
            if readable_before && let Err(reason) = readable(font, &glyph) {
                return Err(p.bad(format!("refused: glyph `{glyph}` could no longer interpolate: {reason}")));
            }
            let g = font.glyph(&glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
            for (id, added) in done {
                let label = g.layers.iter().find(|l| l.id == id).and_then(|l| special_label(font, l));
                rows.push(json!({"glyph": glyph, "layer": id, "label": label, "added": added}));
            }
        }
        Ok(rows)
    })
}

/// The reply of a set or clear command: the first row's keys, `extra`, and every row.
fn special_reply(rows: Vec<Value>, extra: Option<(&str, Map<String, Value>)>) -> Value {
    let mut out = Map::new();
    if let Some(Value::Object(first)) = rows.first() {
        for key in ["glyph", "layer", "label"] {
            out.insert(key.into(), first.get(key).cloned().unwrap_or(Value::Null));
        }
    }
    if let Some((key, map)) = extra {
        out.insert(key.into(), Value::Object(map));
    }
    out.insert("layers".into(), Value::Array(rows));
    Value::Object(out)
}

/// The change of the layers named by `t` that `wanted` picks, and an empty one for each other glyph `allMasters` names.
fn named(font: &Font, t: &SpecialTargets, wanted: impl Fn(&Layer) -> bool) -> Vec<Change> {
    let mut out = Vec::new();
    if let Some(g) = t.glyph.as_deref().and_then(|name| font.glyph(name)) {
        let set: Vec<String> = g.layers.iter().filter(|l| t.layers.contains(&l.id) && wanted(l)).map(|l| l.id.clone()).collect();
        if !set.is_empty() || t.all_masters_of.contains(&g.name) {
            out.push(Change { glyph: g.name.clone(), set, add: Vec::new() });
        }
    }
    let others = t.all_masters_of.iter().filter(|name| t.glyph.as_ref() != Some(*name));
    out.extend(others.map(|glyph| Change { glyph: glyph.clone(), set: Vec::new(), add: Vec::new() }));
    out
}

/// A copy of `master_layer` under the same master, hidden, without special attributes, to be given some.
fn copy_of(p: &Params, g: &Glyph, added: &[Layer], master_layer: &Layer, seed: &str) -> Result<Layer> {
    let count = g.layers.len().saturating_add(added.len());
    let seed = format!("{}|{seed}|{}|{count}", g.name, master_layer.id);
    let taken = |id: &str| g.layers.iter().chain(added).any(|l| l.id == id);
    let id = super::layers::new_layer_id(&seed, taken).ok_or_else(|| p.bad("could not find a free layer id"))?;
    let mut layer = master_layer.clone();
    layer.id = id;
    layer.info.name = None;
    layer.info.attrs = LayerAttrs::default();
    layer.info.visible = false;
    set_inactive(&mut layer, false);
    Ok(layer)
}

/// Refuses a change that leaves everything as it is.
fn something_to_do(p: &Params, changes: Vec<Change>) -> Result<Vec<Change>> {
    let changes: Vec<Change> = changes.into_iter().filter(|c| !c.set.is_empty() || !c.add.is_empty()).collect();
    if changes.is_empty() {
        return Err(p.bad("every master already has such a layer"));
    }
    Ok(changes)
}

// ── Intermediate layers ─────────────────────────────────────────────────────────────────────────

fn set_intermediate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.setIntermediate", value)?;
    let given = p.opt_object("coordinates")?.ok_or_else(|| p.bad("`coordinates` is required: {tag: number or null}"))?;
    let font = s.font()?;
    if font.axes.is_empty() {
        return Err(p.bad("the font has no axes: an intermediate layer needs at least one"));
    }
    let targets = special_targets(&p, font, true)?;
    let mut changes = named(font, &targets, |_| true);
    let is_master = |c: &Change| font.glyph(&c.glyph).is_some_and(|g| g.layers.iter().any(|l| l.is_master_layer() && c.set.contains(&l.id)));
    if changes.iter().any(is_master) {
        return Err(p.bad("a master layer cannot be an intermediate layer: add a layer under the master and make that one intermediate"));
    }
    let designspace = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
    let mut coords: Vec<Option<f64>> = vec![None; font.axes.len()];
    for (tag, v) in given {
        let what = format!("coordinates.{tag}");
        let i = font_axis(&p, font, "coordinates", tag)?;
        let value = opt_coordinate(&p, &what, Some(v))?;
        if let (Some(x), Some(d)) = (value, designspace_axis(&designspace, i)) {
            within_masters(&p, &designspace, d, &what, x)?;
        }
        if let Some(slot) = coords.get_mut(i) {
            *slot = value;
        }
    }
    if !targets.all_masters_of.is_empty() {
        let of: HashSet<&str> = targets.all_masters_of.iter().map(String::as_str).collect();
        for change in changes.iter_mut().filter(|c| of.contains(c.glyph.as_str())) {
            change.add = intermediates_of_masters(&p, font, &designspace, change, &coords)?;
        }
        changes = something_to_do(&p, changes)?;
    }
    let echo: Map<String, Value> = font.axes.iter().zip(&coords).map(|(a, c)| (a.tag.clone(), json!(c))).collect();
    let rows = edit_attrs(s, &p, "Intermediate", changes, |layer| layer.info.attrs.coordinates.clone_from(&coords))?;
    Ok(special_reply(rows, Some(("coordinates", echo))))
}

/// Whether `a` and `b` are the same place.
fn same_place(a: &Location, b: &Location) -> bool {
    a.0.len() == b.0.len() && a.0.iter().zip(&b.0).all(|(a, b)| (a - b).abs() <= RANGE_SLACK * (1.0 + b.abs()))
}

/// An intermediate layer at `coords` for each master of the glyph of `change` that would put one where the glyph has no
/// source yet, the layers the change sets counted as already there. Masters whose coordinates complete `coords` to the
/// same place get one layer, under the first of them.
fn intermediates_of_masters(p: &Params, font: &Font, designspace: &Designspace, change: &Change, coords: &[Option<f64>]) -> Result<Vec<Layer>> {
    let glyph = &change.glyph;
    let g = font.glyph(glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
    if let Some(l) = g.layers.iter().find(|l| change.set.contains(&l.id) && !l.info.attrs.axis_rules.is_empty()) {
        return Err(
            p.bad(format!("`allMasters` adds plain intermediate layers, and {} of glyph `{glyph}` is an alternate layer", layer_title(font, l)))
        );
    }
    let at = |base: &Location| {
        let mut location = base.clone();
        for (i, c) in coords.iter().enumerate() {
            if let (Some(c), Some(slot)) = (c, designspace_axis(designspace, i).and_then(|d| location.0.get_mut(d))) {
                *slot = *c;
            }
        }
        location
    };
    let sources = GlyphSources::of(font, designspace, glyph).map_err(|e| p.bad(format!("glyph `{glyph}` cannot interpolate: {e}")))?;
    let mut taken: Vec<Location> =
        sources.main.iter().chain(&sources.inactive).filter(|s| !change.set.contains(&s.layer_id)).map(|s| s.location.clone()).collect();
    let set = g.layers.iter().filter(|l| change.set.contains(&l.id));
    taken.extend(set.filter_map(|l| designspace.master(&l.master)).map(|m| at(&m.location)));
    let mut added: Vec<Layer> = Vec::new();
    for m in designspace.masters() {
        let Some(master_layer) = g.layer(&m.id) else { continue };
        let location = at(&m.location);
        if taken.iter().any(|l| same_place(l, &location)) {
            continue;
        }
        let mut layer = copy_of(p, g, &added, master_layer, "intermediate")?;
        layer.info.attrs.coordinates = coords.to_vec();
        added.push(layer);
        taken.push(location);
    }
    Ok(added)
}

/// Refuses a coordinate `x` on designspace axis `d` beyond the masters and virtual masters.
fn within_masters(p: &Params, designspace: &Designspace, d: usize, what: &str, x: f64) -> Result<()> {
    let values = designspace.masters().iter().map(|m| &m.location).chain(designspace.virtual_masters().iter().map(|v| &v.location));
    let (lo, hi) = values.filter_map(|l| l.0.get(d).copied()).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let slack = RANGE_SLACK * (1.0 + x.abs());
    if lo <= hi && (x < lo - slack || x > hi + slack) {
        return Err(p.bad(format!("`{what}` is {x}, outside the range of the masters on that axis ({lo} to {hi})")));
    }
    Ok(())
}

fn add_intermediate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.addIntermediate", value)?;
    let glyph = p.str("glyph")?.to_string();
    let font = s.font()?;
    if font.axes.is_empty() {
        return Err(p.bad("the font has no axes: an intermediate layer needs at least one"));
    }
    let g = font.glyph(&glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
    crate::check_unlocked(g)?;
    if g.layers.len() >= super::layers::MAX_LAYERS {
        return Err(p.bad(format!("glyph `{glyph}` has {} layers; at most {} are allowed", g.layers.len(), super::layers::MAX_LAYERS)));
    }
    let designspace = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
    if !matches!(p.get("location"), Some(Value::Object(o)) if !o.is_empty()) {
        return Err(p.bad("`location` is required: {tag: number}"));
    }
    let given = location_overrides(&p, font, &designspace)?;
    for &(d, x) in &given {
        let tag = designspace.axes().get(d).map_or("", |a| a.tag.as_str());
        within_masters(&p, &designspace, d, &format!("location.{tag}"), x)?;
    }
    let at = |base: &Location| {
        let mut location = base.clone();
        for &(d, x) in &given {
            if let Some(slot) = location.0.get_mut(d) {
                *slot = x;
            }
        }
        location
    };
    let drawn = |id: &str| g.layer(id).is_some();
    let master = match p.opt_str("master")? {
        Some(id) => {
            let master = designspace.master(id).ok_or_else(|| p.bad(format!("there is no master `{id}` that interpolates")))?;
            if !drawn(id) {
                return Err(p.bad(format!("glyph `{glyph}` has no layer of master `{id}`")));
            }
            master
        }
        None => {
            // How far the location is from each master, in normalized coordinates; the first of the nearest.
            let distance = |base: &Location| -> Option<f64> {
                let (a, b) = (designspace.to_normalized(base).ok()?, designspace.to_normalized(&at(base)).ok()?);
                Some(a.0.iter().zip(&b.0).map(|(x, y)| (x - y) * (x - y)).sum())
            };
            let candidates = designspace.masters().iter().filter(|m| drawn(&m.id));
            let nearest = candidates.filter_map(|m| Some((distance(&m.location)?, m))).min_by(|a, b| a.0.total_cmp(&b.0));
            nearest.map(|(_, m)| m).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no master layer to interpolate from")))?
        }
    };
    let location = at(&master.location);
    let sources = GlyphSources::of(font, &designspace, &glyph).map_err(|e| p.bad(format!("glyph `{glyph}` cannot interpolate: {e}")))?;
    if let Some(source) = sources.main.iter().chain(&sources.inactive).find(|s| same_place(&s.location, &location)) {
        let title = g.layers.iter().find(|l| l.id == source.layer_id).map_or_else(|| source.layer_id.clone(), |l| layer_title(font, l));
        return Err(p.bad(format!("glyph `{glyph}` already has a source there: {title}")));
    }
    let new = interpolate_layer(font, &sources, &location, &interp_options(font)).map_err(|e| p.bad(format!("glyph `{glyph}`: {e}")))?;
    let seed = format!("{glyph}|intermediate|{:?}|{}", location.0, g.layers.len());
    let id = super::layers::new_layer_id(&seed, |id| g.layers.iter().any(|l| l.id == id)).ok_or_else(|| p.bad("could not find a free layer id"))?;
    let mut layer = Layer::new(master.id.as_str(), new.width);
    layer.id.clone_from(&id);
    layer.shapes = new.shapes;
    layer.anchors = new.anchors;
    layer.info.vert_width = new.info.vert_width;
    layer.info.vert_origin = new.info.vert_origin;
    let mut coords: Vec<Option<f64>> = vec![None; font.axes.len()];
    for (axis, v) in designspace.axes().iter().zip(&location.0) {
        if let Some(slot) = coords.get_mut(axis.font_axis) {
            *slot = Some(*v);
        }
    }
    let echo: Map<String, Value> = font.axes.iter().zip(&coords).filter_map(|(a, c)| c.map(|c| (a.tag.clone(), json!(c)))).collect();
    layer.info.attrs.coordinates = coords;
    let master = master.id.clone();
    let label = special_label(font, &layer);
    s.edit("Add Source", |font| {
        {
            let mut g = font.glyph_mut(&glyph)?;
            let at = g.layers.iter().rposition(|l| l.master == master).map_or(g.layers.len(), |i| i.saturating_add(1));
            g.layers.insert(at, layer);
        }
        sources_ok(font, &glyph).map_err(|reason| p.bad(format!("refused: glyph `{glyph}` could no longer interpolate: {reason}")))
    })?;
    Ok(json!({"glyph": glyph, "layer": id, "master": master, "coordinates": echo, "label": label}))
}

fn clear_intermediate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.clearIntermediate", value)?;
    let font = s.font()?;
    let targets = special_targets(&p, font, false)?;
    let changes = named(font, &targets, |l| !l.info.attrs.coordinates.is_empty());
    if changes.is_empty() {
        return Err(p.bad(format!("not an intermediate layer: {}", targets.layers.join(", "))));
    }
    let rows = edit_attrs(s, &p, "Remove Intermediate Status", changes, |layer| layer.info.attrs.coordinates.clear())?;
    Ok(special_reply(rows, None))
}

// ── Alternate layers ────────────────────────────────────────────────────────────────────────────

fn set_alternate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.setAlternate", value)?;
    let given = p.opt_object("ranges")?.ok_or_else(|| p.bad("`ranges` is required: {tag: {min?, max?}}, or {} for the `[]` partner"))?;
    let font = s.font()?;
    if font.axes.is_empty() {
        return Err(p.bad("the font has no axes: an alternate layer needs at least one"));
    }
    let targets = special_targets(&p, font, true)?;
    let mut changes = named(font, &targets, |_| true);
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
    let is_master = |c: &Change| font.glyph(&c.glyph).is_some_and(|g| g.layers.iter().any(|l| l.is_master_layer() && c.set.contains(&l.id)));
    if !bounded && changes.iter().any(is_master) {
        return Err(p.bad("a master layer needs a range: the `[]` partner is another layer under the master"));
    }
    let echo: Map<String, Value> = font.axes.iter().zip(&rules).map(|(a, r)| (a.tag.clone(), json!({"min": r.min, "max": r.max}))).collect();
    if !targets.all_masters_of.is_empty() {
        let designspace = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
        let of: HashSet<&str> = targets.all_masters_of.iter().map(String::as_str).collect();
        for change in changes.iter_mut().filter(|c| of.contains(c.glyph.as_str())) {
            change.add = alternates_of_masters(&p, font, &designspace, change, &rules)?;
        }
        changes = something_to_do(&p, changes)?;
    }
    let rows = edit_attrs(s, &p, "Alternate", changes, |layer| layer.info.attrs.axis_rules.clone_from(&rules))?;
    Ok(special_reply(rows, Some(("ranges", echo))))
}

/// An alternate layer of `rules` for each master of the glyph of `change` that has none at its own place, the layers
/// the change sets counted. Without bounds, for each master whose layer carries a range and has no `[]` partner.
fn alternates_of_masters(p: &Params, font: &Font, designspace: &Designspace, change: &Change, rules: &[AxisRule]) -> Result<Vec<Layer>> {
    let glyph = &change.glyph;
    let g = font.glyph(glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
    let has_bound = |rules: &[AxisRule]| rules.iter().any(|r| r.min.is_some() || r.max.is_some());
    let wanted = RangeSet::from_rules(designspace, rules).map_err(|e| p.bad(e.to_string()))?;
    let carries = |l: &Layer| {
        let a = &l.info.attrs;
        if !a.coordinates.is_empty() {
            return false;
        }
        if change.set.contains(&l.id) {
            return true;
        }
        !a.axis_rules.is_empty() && RangeSet::from_rules(designspace, &a.axis_rules).is_ok_and(|r| r == wanted)
    };
    let mut added: Vec<Layer> = Vec::new();
    for m in designspace.masters() {
        let Some(master_layer) = g.layer(&m.id) else { continue };
        let partner_of_none = !has_bound(rules) && !has_bound(&master_layer.info.attrs.axis_rules);
        if partner_of_none || g.layers.iter().any(|l| l.master == m.id && carries(l)) {
            continue;
        }
        let mut layer = copy_of(p, g, &added, master_layer, "alternate")?;
        layer.info.attrs.axis_rules = rules.to_vec();
        added.push(layer);
    }
    Ok(added)
}

fn clear_alternate(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("layers.clearAlternate", value)?;
    let font = s.font()?;
    let targets = special_targets(&p, font, false)?;
    let changes = named(font, &targets, |l| !l.info.attrs.axis_rules.is_empty());
    if changes.is_empty() {
        return Err(p.bad(format!("not an alternate layer: {}", targets.layers.join(", "))));
    }
    let rows = edit_attrs(s, &p, "Remove Alternate Status", changes, |layer| layer.info.attrs.axis_rules.clear())?;
    Ok(special_reply(rows, None))
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
    } else if let Some(src) = sources.inactive.iter().find(|s| s.layer_id == id) {
        (src.location.clone(), sources.clone())
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
    reorder_shapes(s, &p, &t, "Set Shape Order", &order)?;
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "order": order}))
}

/// Puts the target layer's shapes in `order`, a permutation of their indices, as one undo step named `label`.
fn reorder_shapes(s: &mut Session, p: &Params, t: &Targets, label: &str, order: &[usize]) -> Result<()> {
    let done = s.edit(label, |font| {
        let mut done = Renumbered::default();
        t.edit_layer(font, |layer| {
            let hinted = match t.side {
                LayerSide::Foreground => !layer.info.hints.is_empty(),
                LayerSide::Background => layer.info.background.as_ref().is_some_and(|b| !b.hints.is_empty()),
            };
            let before = hinted.then(|| layer.clone());
            let shapes = layer.parts_mut(t.side).shapes;
            let mut slots: Vec<Option<Shape>> = std::mem::take(shapes).into_iter().map(Some).collect();
            for &i in order {
                let shape = slots.get_mut(i).and_then(Option::take).ok_or_else(|| p.bad(format!("shape {i} does not resolve")))?;
                shapes.push(shape);
            }
            // Hints follow their shapes to the new places: corner components by shape index,
            // PostScript hints and TrueType instructions by path index.
            if let Some(before) = before {
                let mut new_of: Vec<usize> = (0..order.len()).collect();
                for (new, &old) in order.iter().enumerate() {
                    if let Some(slot) = new_of.get_mut(old) {
                        *slot = new;
                    }
                }
                // A reference past the old shapes pointed at nothing before and is left as it is.
                let remap = |i: usize| Some(new_of.get(i).copied().unwrap_or(i));
                renumber_hints(&before, layer, t.side, &t.glyph, &remap, &mut done.report);
                done.insert(&t.glyph, &layer.id, t.side);
            }
            Ok(())
        })?;
        Ok(done)
    })?;
    s.hints_renumbered(done);
    Ok(())
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
    let mut order: Vec<usize> = (0..count).collect();
    order.swap(a, b);
    reorder_shapes(s, &p, &t, "Swap Shapes", &order)?;
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "a": a, "b": b}))
}
