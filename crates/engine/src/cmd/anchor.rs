//! Anchors (M6.7): add, rename, place and remove an anchor; Set Anchors and Reset Anchors on one
//! layer or on every master; and the anchor and component cases of Path › Align Selection, which
//! `transform.rs` hands to [`align_special`].
//!
//! Rows: `anchor.add`, `anchor.rename`, `anchor.setPosition`, `anchor.remove`,
//! `glyph.setAnchors`, `glyph.setAnchorsForAllMasters`, `glyph.resetAnchors`,
//! `glyph.resetAnchorsForAllMasters`, `anchors.uniqueNamesPerLayer`, `align.anchorToPoints`,
//! `align.componentOriginToNode`, `align.componentOriginPoint`, `anchors.originAnchor`,
//! `components.alignOriginToNode`.
//!
//! Anchor names are unique on each side of a layer: adding an anchor under a taken name appends
//! underscores until it is free, and renaming to a taken name is refused. A locked anchor refuses
//! being placed, aligned or removed. The default anchors come from
//! `fontcraft_components::default_anchors`.

use std::collections::{BTreeSet, HashSet};

use fontcraft_components::{MAX_ANCHOR_NAME_LEN, MAX_LAYER_ANCHORS};
use fontcraft_geom::{Affine, Point, Vec2, round_to_grid};
use fontcraft_model::{Anchor, Font, LayerSide, MAX_COORDINATE, Shape};
use serde_json::{Value, json};

use super::common::{Outcome, grid, in_range, master_metrics};
use super::transform::{Selection, apply};
use super::*;
use crate::params::{Params, TargetKind, Targets};
use crate::{EngineError, check_unlocked};

/// The name Add Anchor gives an anchor when none is asked for.
const NEW_ANCHOR: &str = "new anchor";
/// The name of the anchor that defines a component's origin point for Align Selection.
const ORIGIN_ANCHOR: &str = "origin";
/// Steepest italic angle, in degrees, used to slant an aligned anchor; a steeper one is ignored.
const MAX_ITALIC_ANGLE: f64 = 89.0;

const ONE_ANCHOR: &[TargetKind] = &[TargetKind::Layer];
const ANCHOR_LIST: &[TargetKind] = &[TargetKind::Layer, TargetKind::Anchors];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "anchor.add",
            "Add Anchor",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", name?: string (default \"new anchor\"), x: number, y: number} add an anchor at (x, y), rounded to the font's grid. Anchor names are unique on a layer: a name already there gets underscores appended until it is free. A locked glyph refuses it → {anchor: name, selection: {glyph, layer, anchors: [name]}}",
            has_doc,
            add
        )
        .targets(ONE_ANCHOR)
        .context(&["canvas"]),
        cmd!(
            "anchor.rename",
            "Rename",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", anchor: name, name: string} rename an anchor. A name another anchor of the layer has is refused. A locked glyph refuses it → {anchor: name, selection: {glyph, layer, anchors: [name]}}",
            has_doc,
            rename
        )
        .targets(ONE_ANCHOR),
        cmd!(
            "anchor.setPosition",
            "Set Position",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", anchor: name, x?: number, y?: number} place an anchor: the coordinates given, rounded to the font's grid; at least one of them. A locked anchor or glyph refuses it → {anchor: name, x, y, selection: {glyph, layer, anchors: [name]}}",
            has_doc,
            set_position
        )
        .targets(ONE_ANCHOR),
        cmd!(
            "anchor.remove",
            "Remove",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", anchors: [names]} remove the named anchors from the layer. A locked anchor or glyph refuses it → {removed: count, selection: {glyph, layer}}",
            has_doc,
            remove
        )
        .targets(ANCHOR_LIST),
        cmd!(
            "glyph.setAnchors",
            "Set Anchors",
            ["Glyph"],
            Some("Cmd+U"),
            "{glyph?: name, glyphs?: [names] (one of the two), layer?: id (default: the first master's layer), layers?: [ids]} add the default anchors each glyph's layer lacks: those of the custom glyph data, else of the built-in table (letters get top and bottom, marks their underscore anchors, ligatures named after their parts caret_1 … caret_(n-1)). Anchors already on the layer keep their place. Locked glyphs and glyphs without the layer are skipped and listed → {glyphs: [names changed], added: count, skipped: [{glyph, layer?, reason}]}",
            has_doc,
            set_anchors
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyph.setAnchorsForAllMasters",
            "Set Anchors for All Masters",
            ["Glyph"],
            Some("Cmd+Alt+U"),
            "{glyph?: name, glyphs?: [names] (one of the two)} glyph.setAnchors on every master layer of each glyph → {glyphs: [names changed], added: count, skipped: [{glyph, layer?, reason}]}",
            has_doc,
            set_anchors_all_masters
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyph.resetAnchors",
            "Reset Anchors",
            ["Glyph"],
            Some("Cmd+Shift+U"),
            "{glyph?: name, glyphs?: [names] (one of the two), layer?: id (default: the first master's layer), layers?: [ids]} remove every anchor of each glyph's layer except the locked ones and add the default anchors the layer then lacks in their default places, as glyph.setAnchors places them on an empty layer. Locked glyphs and glyphs without the layer are skipped and listed → {glyphs: [names changed], added: count, removed: count, skipped: [{glyph, layer?, reason}]}",
            has_doc,
            reset_anchors
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyph.resetAnchorsForAllMasters",
            "Reset Anchors for All Masters",
            ["Glyph"],
            Some("Cmd+Alt+Shift+U"),
            "{glyph?: name, glyphs?: [names] (one of the two)} glyph.resetAnchors on every master layer of each glyph → {glyphs: [names changed], added: count, removed: count, skipped: [{glyph, layer?, reason}]}",
            has_doc,
            reset_anchors_all_masters
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

// ── names and positions ─────────────────────────────────────────────────────────────────────────

/// Checks an anchor name from a parameter: not empty, no control characters, at most
/// [`MAX_ANCHOR_NAME_LEN`] bytes.
fn check_name<'a>(p: &Params, key: &str, name: &'a str) -> Result<&'a str> {
    if name.trim().is_empty() {
        return Err(p.bad(format!("`{key}` must not be empty")));
    }
    if name.len() > MAX_ANCHOR_NAME_LEN {
        return Err(p.bad(format!("`{key}` is {} bytes long; at most {MAX_ANCHOR_NAME_LEN} are allowed", name.len())));
    }
    if name.chars().any(char::is_control) {
        return Err(p.bad(format!("`{key}` must not hold control characters")));
    }
    Ok(name)
}

/// `name`, or `name` with underscores appended until no anchor in `anchors` has it; `None` when
/// the result would be longer than [`MAX_ANCHOR_NAME_LEN`].
pub(crate) fn unique_anchor_name(anchors: &[Anchor], name: &str) -> Option<String> {
    let taken: HashSet<&str> = anchors.iter().map(|a| a.name.as_str()).collect();
    let mut candidate = name.to_string();
    // Each try is a distinct name, so at most `taken.len() + 1` tries are needed.
    for _ in 0..=taken.len() {
        if candidate.len() > MAX_ANCHOR_NAME_LEN {
            return None;
        }
        if !taken.contains(candidate.as_str()) {
            return Some(candidate);
        }
        candidate.push('_');
    }
    None
}

/// An optional coordinate parameter, within the model's range.
fn coordinate(p: &Params, key: &str) -> Result<Option<f64>> {
    match p.opt_f64(key)? {
        Some(v) if !in_range(v) => Err(p.bad(format!("`{key}` is {v}, beyond ±{MAX_COORDINATE}"))),
        other => Ok(other),
    }
}

/// The target object of a command that names one layer and no lists.
fn layer_target(p: &Params, what: &str) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.has_lists() {
        return Err(p.bad(format!("takes no target lists: {what}")));
    }
    Ok(t)
}

fn locked_anchor(p: &Params, name: &str) -> EngineError {
    p.bad(format!("anchor `{name}` is locked: unlock it to change it"))
}

// ── one anchor ──────────────────────────────────────────────────────────────────────────────────

fn add(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("anchor.add", v)?;
    let mut t = layer_target(&p, "it adds an anchor to the layer")?;
    let x = coordinate(&p, "x")?.ok_or_else(|| p.bad("`x` is required"))?;
    let y = coordinate(&p, "y")?.ok_or_else(|| p.bad("`y` is required"))?;
    let wanted = match p.opt_str("name")? {
        Some(n) => check_name(&p, "name", n)?,
        None => NEW_ANCHOR,
    };
    let font = s.font()?;
    t.resolve(font)?;
    let step = grid(font);
    let (x, y) = (round_to_grid(x, step), round_to_grid(y, step));
    t.layer = Some(t.layer_id(font)?);
    let name = s.edit("Add Anchor", |font| {
        t.edit_layer(font, |l| {
            let parts = l.parts_mut(t.side);
            if parts.anchors.len() >= MAX_LAYER_ANCHORS {
                return Err(p.bad(format!("the layer already has {MAX_LAYER_ANCHORS} anchors, the most it may hold")));
            }
            let name = unique_anchor_name(parts.anchors, wanted)
                .ok_or_else(|| p.bad(format!("`{wanted}` is taken and no free name based on it is at most {MAX_ANCHOR_NAME_LEN} bytes long")))?;
            parts.anchors.push(Anchor::new(name.clone(), x, y));
            Ok(name)
        })
    })?;
    t.anchors = vec![name.clone()];
    Ok(json!({"anchor": name, "selection": t.to_json()}))
}

fn rename(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("anchor.rename", v)?;
    let mut t = layer_target(&p, "it renames the anchor named by `anchor`")?;
    let old = p.str("anchor")?.to_string();
    let new = check_name(&p, "name", p.str("name")?)?.to_string();
    let font = s.font()?;
    let r = t.resolve(font)?;
    if r.parts.anchor(&old).is_none() {
        return Err(p.bad(format!("the layer has no anchor `{old}`")));
    }
    if new != old && r.parts.anchor(&new).is_some() {
        return Err(p.bad(format!("the layer already has an anchor `{new}`: anchor names are unique on a layer")));
    }
    t.layer = Some(r.layer.id.clone());
    if new != old {
        s.edit("Rename Anchor", |font| {
            t.edit_layer(font, |l| {
                let a = l
                    .parts_mut(t.side)
                    .anchors
                    .iter_mut()
                    .find(|a| a.name == old)
                    .ok_or_else(|| p.bad(format!("the layer has no anchor `{old}`")))?;
                a.name.clone_from(&new);
                Ok(())
            })
        })?;
    }
    t.anchors = vec![new.clone()];
    Ok(json!({"anchor": new, "selection": t.to_json()}))
}

fn set_position(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("anchor.setPosition", v)?;
    let mut t = layer_target(&p, "it places the anchor named by `anchor`")?;
    let name = p.str("anchor")?.to_string();
    let (x, y) = (coordinate(&p, "x")?, coordinate(&p, "y")?);
    if x.is_none() && y.is_none() {
        return Err(p.bad("give `x`, `y` or both"));
    }
    let font = s.font()?;
    let step = grid(font);
    let r = t.resolve(font)?;
    let (_, a) = r.parts.anchor(&name).ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
    if a.info.locked {
        return Err(locked_anchor(&p, &name));
    }
    let to = (x.map_or(a.x, |x| round_to_grid(x, step)), y.map_or(a.y, |y| round_to_grid(y, step)));
    t.layer = Some(r.layer.id.clone());
    s.edit("Move Anchor", |font| {
        t.edit_layer(font, |l| {
            let a =
                l.parts_mut(t.side).anchors.iter_mut().find(|a| a.name == name).ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
            (a.x, a.y) = to;
            Ok(())
        })
    })?;
    t.anchors = vec![name.clone()];
    Ok(json!({"anchor": name, "x": to.0, "y": to.1, "selection": t.to_json()}))
}

fn remove(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("anchor.remove", v)?;
    let mut t = Targets::parse(&p)?;
    if t.anchors.is_empty() {
        return Err(p.bad("`anchors` is required and must name at least one anchor"));
    }
    if !(t.points.is_empty() && t.segments.is_empty() && t.shapes.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("removes anchors only: give `anchors` and no other target list"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    if let Some(a) = r.parts.anchors.iter().find(|a| a.info.locked && t.anchors.contains(&a.name)) {
        return Err(locked_anchor(&p, &a.name));
    }
    t.layer = Some(r.layer.id.clone());
    let removed = s.edit("Remove Anchors", |font| {
        t.edit_layer(font, |l| {
            let anchors = l.parts_mut(t.side).anchors;
            let before = anchors.len();
            anchors.retain(|a| !t.anchors.contains(&a.name));
            Ok(before.saturating_sub(anchors.len()))
        })
    })?;
    t.anchors.clear();
    Ok(json!({"removed": removed, "selection": t.to_json()}))
}

// ── Set Anchors and Reset Anchors ───────────────────────────────────────────────────────────────

fn set_anchors(s: &mut Session, v: &Value) -> Result<Value> {
    default_anchors(s, &Params::new("glyph.setAnchors", v)?, "Set Anchors", false, false)
}

fn set_anchors_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    default_anchors(s, &Params::new("glyph.setAnchorsForAllMasters", v)?, "Set Anchors for All Masters", false, true)
}

fn reset_anchors(s: &mut Session, v: &Value) -> Result<Value> {
    default_anchors(s, &Params::new("glyph.resetAnchors", v)?, "Reset Anchors", true, false)
}

fn reset_anchors_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    default_anchors(s, &Params::new("glyph.resetAnchorsForAllMasters", v)?, "Reset Anchors for All Masters", true, true)
}

/// Adds the default anchors to each addressed layer; with `reset`, removes the layer's anchors
/// first. With `all_masters`, the addressed layers are every master layer of each glyph.
fn default_anchors(s: &mut Session, p: &Params, label: &str, reset: bool, all_masters: bool) -> Result<Value> {
    let data = s.glyph_data()?;
    let font = s.font()?;
    if all_masters && (p.get("layer").is_some() || p.get("layers").is_some()) {
        return Err(p.bad("acts on every master layer: `layer` and `layers` are not taken"));
    }
    let scope = super::common::scope(p, font)?;
    let masters: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    let (mut added, mut removed) = (0usize, 0usize);
    let out = s.edit(label, |font| {
        let mut out = Outcome::default();
        for name in &scope.glyphs {
            let g = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
            if g.info.locked {
                if scope.single {
                    check_unlocked(g)?;
                }
                out.skip(name, None, "the glyph is locked");
                continue;
            }
            let layers: Vec<String> = if all_masters {
                g.layers.iter().filter(|l| l.is_master_layer() && masters.contains(&l.id)).map(|l| l.id.clone()).collect()
            } else {
                let mut present = Vec::new();
                for id in &scope.layers {
                    if g.layers.iter().any(|l| l.id == *id) {
                        present.push(id.clone());
                    } else if scope.single {
                        return Err(p.bad(format!("glyph `{name}` has no layer `{id}`")));
                    } else {
                        out.skip(name, Some(id.as_str()), "the glyph has no such layer");
                    }
                }
                present
            };
            let mut changed = false;
            for id in &layers {
                if reset {
                    let mut g = font.glyph_mut(name)?;
                    // A locked anchor stays where it is; the defaults skip its name.
                    if let Some(l) = g.layers.iter_mut().find(|l| l.id == *id)
                        && l.anchors.iter().any(|a| !a.info.locked)
                    {
                        let before = l.anchors.len();
                        l.anchors.retain(|a| a.info.locked);
                        removed = removed.saturating_add(before.saturating_sub(l.anchors.len()));
                        changed = true;
                    }
                }
                let new = fontcraft_components::default_anchors::default_anchors(font, &data, name, id).map_err(|e| p.bad(e.to_string()))?;
                if new.is_empty() {
                    continue;
                }
                added = added.saturating_add(new.len());
                let mut g = font.glyph_mut(name)?;
                if let Some(l) = g.layers.iter_mut().find(|l| l.id == *id) {
                    l.anchors.extend(new);
                    changed = true;
                }
            }
            if changed {
                out.glyphs.push(name.clone());
            }
        }
        Ok(out)
    })?;
    let mut result = json!({"glyphs": out.glyphs, "added": added, "skipped": out.skipped});
    if reset && let Some(o) = result.as_object_mut() {
        o.insert("removed".into(), json!(removed));
    }
    Ok(result)
}

// ── Align Selection: the anchor and component cases ─────────────────────────────────────────────

/// Path › Align Selection when the selection is one anchor with one or two points (the anchor
/// goes above the point, or between the two, along the italic angle) or one component with one
/// node (the component's origin point goes onto the node). `None` for any other selection.
pub(super) fn align_special(s: &mut Session, p: &Params, t: &Targets) -> Result<Option<Value>> {
    if !(t.segments.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Ok(None);
    }
    match (t.anchors.as_slice(), t.shapes.as_slice(), t.points.len()) {
        ([anchor], [], 1 | 2) => align_anchor(s, p, t, anchor).map(Some),
        ([], [shape], 1) => {
            let is_component = matches!(t.resolve(s.font()?)?.parts.shapes.get(*shape), Some(Shape::Component(_)));
            if is_component { align_component(s, p, t, *shape).map(Some) } else { Ok(None) }
        }
        _ => Ok(None),
    }
}

/// The anchor moves horizontally onto the line through the point (or the two points' midpoint)
/// at the master's italic angle; the points stay.
fn align_anchor(s: &mut Session, p: &Params, t: &Targets, name: &str) -> Result<Value> {
    let font = s.font()?;
    let step = grid(font);
    let r = t.resolve(font)?;
    let (_, a) = r.parts.anchor(name).ok_or_else(|| p.bad(format!("`anchors`: the layer has no anchor `{name}`")))?;
    if a.info.locked {
        return Err(locked_anchor(p, name));
    }
    let points: Vec<Point> = t.points.iter().filter_map(|(s, n)| r.parts.node(*s, *n).map(|n| n.point())).collect();
    let count = points.len().max(1) as f64;
    let mid = points.iter().fold(Point::ZERO, |sum, q| sum + q.to_vec2() / count);
    let angle = master_metrics(font, r.master).italic_angle;
    let lean = if angle.is_finite() && angle.abs() < MAX_ITALIC_ANGLE { angle.to_radians().tan() } else { 0.0 };
    let x = round_to_grid(mid.x + (a.y - mid.y) * lean, step);
    if !in_range(x) {
        return Err(p.bad(format!("the anchor would go to x = {x}, beyond ±{MAX_COORDINATE}")));
    }
    let moved = usize::from(x != a.x);
    if moved > 0 {
        s.edit("Align Selection", |font| {
            t.edit_layer(font, |l| {
                let a = l
                    .parts_mut(t.side)
                    .anchors
                    .iter_mut()
                    .find(|a| a.name == name)
                    .ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
                a.x = x;
                Ok(())
            })
        })?;
    }
    Ok(json!({"glyph": t.glyph, "moved": moved, "anchor": name, "x": x}))
}

/// The component at `shape` moves so its origin point lands on the selected node; the node stays.
fn align_component(s: &mut Session, p: &Params, t: &Targets, shape: usize) -> Result<Value> {
    let font = s.font()?;
    let step = grid(font);
    let r = t.resolve(font)?;
    let Some(Shape::Component(c)) = r.parts.shapes.get(shape) else {
        return Err(p.bad(format!("shape {shape} is not a component")));
    };
    let Some(&(s0, n0)) = t.points.first() else { return Err(p.bad("select one node with the component")) };
    let node = r.parts.node(s0, n0).ok_or_else(|| p.bad(format!("`points`: [{s0}, {n0}] does not resolve")))?;
    if !node.on_curve() {
        return Err(p.bad("Align Selection puts a component on a node, not on a handle"));
    }
    if c.info.locked {
        return Err(p.bad(format!("component {shape} is locked: unlock it to move it")));
    }
    if t.side == LayerSide::Foreground && fontcraft_components::align::component_alignment(font, r.layer, shape).is_some() {
        return Err(p.bad(format!("component {shape} is placed by automatic alignment: turn alignment off for it to move it")));
    }
    let master = c.info.master_id.as_deref().unwrap_or(r.layer.master.as_str());
    let origin = c.affine() * component_origin(font, &c.glyph, master);
    let to = node.point();
    let d: Vec2 = to - origin;
    if !(d.x.is_finite() && d.y.is_finite()) {
        return Err(p.bad(format!("component {shape} has a placement that is not a usable number")));
    }
    if d == Vec2::ZERO {
        return Ok(json!({"glyph": t.glyph, "moved": 0, "component": shape, "origin": [origin.x, origin.y]}));
    }
    let sel = Selection { shapes: BTreeSet::from([shape]), ..Selection::default() };
    s.edit("Align Selection", |font| t.edit_layer(font, |l| apply(p, l, t.side, &sel, Affine::translate(d), step)))?;
    Ok(json!({"glyph": t.glyph, "moved": 1, "component": shape, "origin": [to.x, to.y]}))
}

/// A component's origin point in its base glyph's coordinates: the base layer's `origin` anchor
/// when it has one. Otherwise the point where the baseline meets the left sidebearing line,
/// x = 0. On a slanted master the vertical through the slanted left sidebearing line at half the
/// x-height is used instead; that line is slanted about half the x-height, so the vertical is
/// x = 0 as well.
fn component_origin(font: &Font, base: &str, master: &str) -> Point {
    font.glyph(base)
        .and_then(|g| g.layer(master))
        .and_then(|l| l.anchors.iter().find(|a| a.name == ORIGIN_ANCHOR))
        .filter(|a| in_range(a.x) && in_range(a.y))
        .map_or(Point::ZERO, |a| Point::new(a.x, a.y))
}
