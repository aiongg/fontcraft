//! Cap components (`capComponent.*`): attach a cap glyph to the two nodes that end a stroke,
//! decompose it, swap its glyph, scale it, choose its alignment and turn Fit on or off.
//!
//! A cap component is a `Cap` hint on the layer (`fontcraft_model::attach`); outline resolution
//! fits it into the path. Commands address caps by their position in the layer's hint list
//! (`caps: [indices]`) and nodes in the model's order (`points: [[shape, node]]`); the hint
//! counts a closed path's nodes in the file's order, start node last, as a corner's does. Caps
//! live on the foreground only. An edit keeps every other key of the stored hint and its
//! spelling.

use fontcraft_model::attach::{self, CAP_FIT_BIT, CAP_PREFIX, CapHint, MAX_CAP_SCALE};
use fontcraft_model::corner::CornerAlignment;
use fontcraft_model::plist::{Dict, Number, Str, Value as Plist};
use fontcraft_model::{Font, Hint, LayerSide, MAX_LAYER_HINTS, MAX_NAME_LEN, Shape};
use serde_json::{Value, json};

use super::corner::{file_index, pair, parse_mode, set_entry};
use super::*;
use crate::params::{Params, TargetKind, Targets};

const NO_LISTS: &str = "addresses caps with `caps` and takes no target lists";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "capComponent.add",
            "Add Cap Component",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), points: [[shape, node], [shape, node]], cap: name} attaches the cap glyph `cap` (its name starts with _cap.) to two neighbouring on-curve nodes of one path, given in either order: the stroke end between them is replaced by the cap's open path. It is stored on the first of the two in path order, unscaled, aligned left, Fit off. Two nodes on different paths, nodes that are not neighbours and an off-curve node are refused. A pair that already holds a cap gets the new one in its place. Foreground only; at most 4,096 hints per layer. A locked glyph refuses it → {glyph, layer, caps: [hint index], replaced: bool, warning?: why the layer's caps cannot be fitted, selection: {glyph, layer, caps}}",
            has_doc,
            add
        )
        .targets(&[TargetKind::Points]),
        cmd!(
            "capComponent.decompose",
            "Decompose Cap",
            [],
            None,
            "{glyph: name, layer?: id, caps?: [hint indices] (default: every cap of the layer)} fits the chosen caps into their paths and removes them; the other caps and corners keep their nodes. A cap that cannot be fitted (its glyph is missing, its stem end has zero length, the scale is too large) is an error and nothing changes → {glyph, layer, decomposed: number, selection: {glyph, layer}}",
            has_doc,
            decompose
        ),
        cmd!(
            "capComponent.setFit",
            "Fit Cap to Stroke",
            [],
            None,
            "{glyph: name, layer?: id, caps: [hint indices], fit: bool} turns Fit on or off: with Fit on, the cap's horizontal scale is the width of the stroke end over the cap's own width (node1 to node2, or without them from one end of its path to the other; its drawn width, then its advance width, when that is 0) and the stored horizontal scale is kept but not used → {glyph, layer, caps, warning?, selection: {glyph, layer, caps}}",
            has_doc,
            set_fit
        ),
        cmd!(
            "capComponent.setGlyph",
            "Replace Cap Component",
            [],
            None,
            "{glyph: name, layer?: id, caps: [hint indices], cap: name} gives the listed caps the cap glyph `cap`, keeping their nodes, scale, alignment and Fit → {glyph, layer, caps, warning?, selection: {glyph, layer, caps}}",
            has_doc,
            set_glyph
        ),
        cmd!(
            "capComponent.setScale",
            "Set Cap Scale",
            [],
            None,
            "{glyph: name, layer?: id, caps: [hint indices], scaleX?: percent, scaleY?: percent} sets the horizontal (along the stroke end) and vertical scale of the listed caps, about the cap's origin or node1; a negative value mirrors the cap. Give at least one; a scale is not 0 and within ±10,000 percent → {glyph, layer, caps, warning?, selection: {glyph, layer, caps}}",
            has_doc,
            set_scale
        ),
        cmd!(
            "capComponent.setAlignment",
            "Set Cap Alignment",
            [],
            None,
            "{glyph: name, layer?: id, caps: [hint indices], mode: \"left\" | \"right\" | \"both\" | \"none\"} sets how the listed caps turn: left follows the side before the first node, right the side after the second, both the line between the two nodes, none keeps the cap as drawn; Fit is kept → {glyph, layer, caps, warning?, selection: {glyph, layer, caps}}",
            has_doc,
            set_alignment
        ),
    ]
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// Refuses `name` unless it is a cap glyph of `font` other than `host`.
fn check_cap_glyph(p: &Params, font: &Font, name: &str, host: &str) -> Result<()> {
    if !attach::is_cap_name(name) || name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`{name}` is not a cap glyph: a cap glyph's name starts with {CAP_PREFIX}")));
    }
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("there is no glyph `{name}`")));
    }
    if name == host {
        return Err(p.bad(format!("a cap of `{name}` cannot be attached to `{name}` itself")));
    }
    Ok(())
}

/// The target object, refused on the background and with target lists (other than `points`
/// when `points_allowed`).
fn foreground_targets(p: &Params, points_allowed: bool, why: &str) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.side != LayerSide::Foreground {
        return Err(p.bad("cap components sit on the foreground only"));
    }
    let others = !(t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty());
    if others || (!points_allowed && !t.points.is_empty()) {
        return Err(p.bad(why));
    }
    Ok(t)
}

/// `caps`: hint indices, at least one, each once.
fn cap_list(p: &Params) -> Result<Vec<usize>> {
    let list = p.array("caps")?;
    if list.is_empty() {
        return Err(p.bad("`caps` must not be empty"));
    }
    if list.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`caps` names {} caps; a layer holds at most {MAX_LAYER_HINTS} hints", list.len())));
    }
    let mut out = Vec::new();
    for v in list {
        let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`caps` must hold hint indices, found {v}")))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// Refuses a scale factor of 0, or one that is not a number or beyond the model's limit.
fn check_factor(p: &Params, key: &str, f: f64) -> Result<()> {
    if !f.is_finite() || f == 0.0 || f.abs() > MAX_CAP_SCALE {
        return Err(p.bad(format!("`{key}` must not be 0 and lie within ±{} percent, got {}", MAX_CAP_SCALE * 100.0, f * 100.0)));
    }
    Ok(())
}

/// The cap on the two nodes `a` and `b` (model order) of path `shape`: its path and file-order
/// first node, the earlier of the two in path order.
fn cap_target(p: &Params, layer: &fontcraft_model::Layer, (sa, a): (usize, usize), (sb, b): (usize, usize)) -> Result<(usize, usize)> {
    if sa != sb {
        return Err(p.bad(format!("`points`: [{sa}, {a}] and [{sb}, {b}] lie on two paths; a cap sits on two nodes of one path")));
    }
    let path = match layer.shapes.get(sa) {
        Some(Shape::Path(path)) => path,
        Some(_) => return Err(p.bad(format!("`points`: shape {sa} is not a path"))),
        None => return Err(p.bad(format!("`points`: shape {sa} does not resolve: the layer has {} shapes", layer.shapes.len()))),
    };
    let len = path.nodes.len();
    for n in [a, b] {
        match path.nodes.get(n) {
            None => return Err(p.bad(format!("`points`: [{sa}, {n}] does not resolve: path {sa} has {len} nodes"))),
            Some(node) if !node.on_curve() => return Err(p.bad(format!("`points`: [{sa}, {n}] is an off-curve node; a cap sits on on-curve nodes"))),
            Some(_) => {}
        }
    }
    let next = |n: usize| if path.closed { Some((n + 1) % len) } else { Some(n + 1).filter(|&m| m < len) };
    let first = if next(a) == Some(b) {
        a
    } else if next(b) == Some(a) {
        b
    } else {
        return Err(
            p.bad(format!("`points`: nodes {a} and {b} of path {sa} are not neighbours; a cap sits on the two nodes of one straight segment"))
        );
    };
    Ok((sa, file_index(path.closed, len, first)))
}

/// The cap of each hint, `None` for other hints and for caps that cannot be read.
fn cap_targets(hints: &[Hint]) -> Vec<Option<(usize, usize)>> {
    hints.iter().map(|h| attach::cap_hint(h).ok().flatten().map(|c| (c.shape, c.node))).collect()
}

/// Why the caps of the layer cannot be fitted, if they cannot.
fn fit_warning(font: &Font, glyph: &str, layer: &str) -> Option<String> {
    let l = font.glyph(glyph)?.layers.iter().find(|l| l.id == layer)?;
    attach::insert_attachments(font, l).err().map(|e| e.to_string())
}

/// The common part of a result: glyph, layer, caps, warning and selection.
fn result(s: &Session, p: &Params, t: &Targets, layer: &str, caps: &[usize]) -> Result<Value> {
    let mut selection = Targets::new(p.command(), t.glyph.clone(), Some(layer.to_string())).to_json();
    if let Some(o) = selection.as_object_mut() {
        o.insert("caps".into(), json!(caps));
    }
    let mut out = json!({ "glyph": t.glyph, "layer": layer, "caps": caps, "selection": selection });
    if let Some(w) = fit_warning(s.font()?, &t.glyph, layer)
        && let Some(o) = out.as_object_mut()
    {
        o.insert("warning".into(), json!(w));
    }
    Ok(out)
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn add(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("capComponent.add", v)?;
    let name = p.str("cap")?.to_string();
    let mut t = foreground_targets(&p, true, "takes its two nodes as `points` and no other target list")?;
    let mut points = Vec::new();
    for &pt in &t.points {
        if !points.contains(&pt) {
            points.push(pt);
        }
    }
    let &[one, two] = points.as_slice() else {
        return Err(p.bad(format!("give the cap's two nodes as `points`, found {}", points.len())));
    };
    check_cap_glyph(&p, s.font()?, &name, &t.glyph)?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    let (index, replaced) = s.edit("Add Cap Component", |font| {
        t.edit_layer(font, |l| {
            let (shape, node) = cap_target(&p, l, one, two)?;
            let hint = CapHint::new(name.clone(), shape, node).to_hint();
            if let Some(i) = cap_targets(&l.info.hints).iter().position(|c| *c == Some((shape, node)))
                && let Some(slot) = l.info.hints.get_mut(i)
            {
                *slot = hint;
                return Ok((i, true));
            }
            if l.info.hints.len() >= MAX_LAYER_HINTS {
                return Err(p.bad(format!("the layer already holds {MAX_LAYER_HINTS} hints, the most allowed")));
            }
            l.info.hints.push(hint);
            Ok((l.info.hints.len() - 1, false))
        })
    })?;
    let mut out = result(s, &p, &t, &layer, &[index])?;
    if let Some(o) = out.as_object_mut() {
        o.insert("replaced".into(), json!(replaced));
    }
    Ok(out)
}

fn decompose(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("capComponent.decompose", v)?;
    let mut t = foreground_targets(&p, false, NO_LISTS)?;
    let chosen = match p.opt_array("caps")? {
        Some(_) => Some(cap_list(&p)?),
        None => None,
    };
    let count = s.edit("Decompose Cap", |font| {
        let (layer, count) = {
            let r = t.resolve(font)?;
            crate::check_unlocked(r.glyph)?;
            let hints = &r.layer.info.hints;
            let which: Vec<usize> = match &chosen {
                Some(list) => list.clone(),
                None => hints.iter().enumerate().filter(|(_, h)| attach::is_cap(h)).map(|(i, _)| i).collect(),
            };
            if which.is_empty() {
                return Err(p.bad("the layer has no cap components"));
            }
            let layer = attach::decompose_attachments(font, r.layer, &which).map_err(|e| p.bad(e.to_string()))?;
            (layer, which.len())
        };
        t.edit_layer(font, |l| {
            *l = layer;
            Ok(())
        })?;
        Ok(count)
    })?;
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "decomposed": count, "selection": t.to_json()}))
}

/// Runs `f` on each listed cap's stored dictionary, as one undo step named `label`.
fn edit_caps(s: &mut Session, p: &Params, label: &str, f: impl Fn(&CapHint, &mut Dict) -> Result<()>) -> Result<Value> {
    let mut t = foreground_targets(p, false, NO_LISTS)?;
    let caps = cap_list(p)?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    s.edit(label, |font| {
        t.edit_layer(font, |l| {
            let count = l.info.hints.len();
            for &i in &caps {
                let hint = l.info.hints.get_mut(i).ok_or_else(|| p.bad(format!("`caps`: {i} does not resolve: the layer has {count} hints")))?;
                let cap = attach::cap_hint(hint)
                    .map_err(|m| p.bad(format!("`caps`: hint {i}: {m}")))?
                    .ok_or_else(|| p.bad(format!("`caps`: hint {i} is not a cap component")))?;
                let Plist::Dict(d) = &mut hint.value else { return Err(p.bad(format!("`caps`: hint {i} is not a cap component"))) };
                f(&cap, d)?;
            }
            Ok(())
        })
    })?;
    result(s, p, &t, &layer, &caps)
}

/// Writes `options` for `alignment` and `fit`; 0 (left, Fit off) is written without the key.
fn set_options(d: &mut Dict, alignment: CornerAlignment, fit: bool) {
    let options = alignment.option() + if fit { CAP_FIT_BIT } else { 0 };
    set_entry(d, "options", (options != 0).then(|| Plist::Number(Number::int(options))));
}

fn set_fit(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("capComponent.setFit", v)?;
    let fit = p.get("fit").and_then(Value::as_bool).ok_or_else(|| p.bad("`fit` must be true or false"))?;
    edit_caps(s, &p, "Fit Cap to Stroke", |c, d| {
        set_options(d, c.alignment, fit);
        Ok(())
    })
}

fn set_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("capComponent.setGlyph", v)?;
    let name = p.str("cap")?;
    check_cap_glyph(&p, s.font()?, name, p.str("glyph")?)?;
    edit_caps(s, &p, "Replace Cap Component", |_, d| {
        set_entry(d, "name", Some(Plist::String(Str::new(name))));
        Ok(())
    })
}

fn set_scale(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("capComponent.setScale", v)?;
    let x = p.opt_f64("scaleX")?.map(|v| v / 100.0);
    let y = p.opt_f64("scaleY")?.map(|v| v / 100.0);
    if x.is_none() && y.is_none() {
        return Err(p.bad("give `scaleX` or `scaleY`"));
    }
    for (key, f) in [("scaleX", x), ("scaleY", y)] {
        if let Some(f) = f {
            check_factor(&p, key, f)?;
        }
    }
    edit_caps(s, &p, "Set Cap Scale", |c, d| {
        let (sx, sy) = (x.unwrap_or(c.scale.x), y.unwrap_or(c.scale.y));
        let value = (sx != 1.0 || sy != 1.0).then(|| pair(d, sx, sy));
        set_entry(d, "scale", value);
        Ok(())
    })
}

fn set_alignment(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("capComponent.setAlignment", v)?;
    let mode = parse_mode(&p, p.str("mode")?, false)?;
    edit_caps(s, &p, "Set Cap Alignment", |c, d| {
        set_options(d, mode, c.fit);
        Ok(())
    })
}
