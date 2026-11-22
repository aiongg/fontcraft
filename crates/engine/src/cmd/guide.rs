//! Guides. A local guide belongs to one side of a layer, a global guide to the layer's master;
//! both are addressed as `{scope, index}` (decision 0004), and every change is one undo step,
//! global guides included (D4). A locked guide refuses every change but `guide.unlock`.
//!
//! `guide.displayLines` gives the other lines Edit View draws for a layer: the master's metrics
//! with the per-script parameters applied, the alignment zones, the grid and the CJK grids.

use std::sync::Arc;

use fontcraft_geom::round_to_grid;
use fontcraft_model::guide_follow::{self, Reference};
use fontcraft_model::shown_metrics::{self, param_number, script_param};
use fontcraft_model::{Font, Guide, LayerParts, MAX_COORDINATE, MAX_LAYER_GUIDES, Master, metric};
use serde_json::{Value, json};

use super::common::grid;
use super::*;
use crate::EngineError;
use crate::params::{GuideRef, GuideScope, Params, TargetKind, Targets, kind};

/// Longest name, filter or colour text a guide may be given.
const MAX_GUIDE_TEXT: usize = 4096;
/// Most rows or columns a CJK grid parameter may ask for.
const MAX_CJK_DIVISIONS: f64 = 1000.0;
/// Most numbers read from a `Design Grid` value.
const MAX_DESIGN_GRID_VALUES: usize = 8;

const LIST_DOC: &str = "guides: [{scope: \"local\" | \"global\", index}…]";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "guide.add",
            "Add Guide",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", x?: number = 0, y?: number = 0, angle?: degrees counter-clockwise from horizontal = 0, throughPoints?: [[shape, node], [shape, node]]} add a local guide through (x, y) at the angle, or through the two points given (then x, y and angle are not allowed). The position given is rounded to the font's grid. A locked glyph refuses it → {guide: {scope: \"local\", index}, selection: {glyph, layer, guides: [{scope, index}]}}",
            has_doc,
            add
        )
        .targets(&[TargetKind::Layer])
        .context(&["canvas", "canvas.node"]),
        cmd!(
            "guide.addGlobal",
            "Add Global Guide",
            [],
            None,
            "{master?: id, glyph?: name, layer?: id, x?: number = 0, y?: number = 0, angle?: degrees counter-clockwise from horizontal = 0} add a global guide to a master: the one given, else the master of the glyph's layer when `glyph` is given, else the first master. The position is rounded to the font's grid → {master, guide: {scope: \"global\", index}, selection?: {glyph, layer, guides} (when `glyph` is given)}",
            has_doc,
            add_global
        )
        .targets(&[TargetKind::Layer])
        .context(&["canvas", "canvas.node"]),
        cmd!(
            "guide.makeGlobal",
            "Make Global Guide",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guides: [{scope: \"local\", index}…]} move local guides to the layer's master, keeping their position and properties. A locked guide or a locked glyph refuses it → {selection: {glyph, layer, guides: [{scope: \"global\", index}…]}}",
            has_doc,
            make_global
        )
        .targets(&[TargetKind::Layer, TargetKind::Guides])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.makeLocal",
            "Make Local Guide",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guides: [{scope: \"global\", index}…]} move global guides from the master to this layer, keeping their position and properties. A locked guide or a locked glyph refuses it → {selection: {glyph, layer, guides: [{scope: \"local\", index}…]}}",
            has_doc,
            make_local
        )
        .targets(&[TargetKind::Layer, TargetKind::Guides])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.lock",
            "Lock Guides",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guides: [{scope: \"local\" | \"global\", index}…]} lock guides: a locked guide cannot be selected, and every command that changes a guide refuses it until it is unlocked. A locked glyph refuses its local guides → {guides: count}",
            has_doc,
            lock
        )
        .targets(&[TargetKind::Layer, TargetKind::Guides])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.unlock",
            "Unlock Guide",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guides: [{scope: \"local\" | \"global\", index}…]} unlock guides. A locked glyph refuses its local guides → {guides: count}",
            has_doc,
            unlock
        )
        .targets(&[TargetKind::Layer, TargetKind::Guides])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.setProperties",
            "Set Properties",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guide: {scope: \"local\" | \"global\", index}, x?: number, y?: number, angle?: degrees, angleLocked?: bool, relativeTo?: \"left\" | \"right\" | \"both\", measurement?: bool, name?: string, filter?: string (global guides only: a glyph predicate), color?: \"r,g,b,a\" with numbers from 0 to 1} change a guide's properties; an empty name, filter or colour clears it. x and y are rounded to the font's grid; the angle is kept from 0 to 360. An angle-locked guide refuses a new angle unless the same call sets angleLocked: false. A locked guide refuses every change → {guide: {scope, index}}",
            has_doc,
            set_properties
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "guide.remove",
            "Remove",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guides: [{scope: \"local\" | \"global\", index}…]} remove guides. A locked guide refuses it; a locked glyph refuses its local guides → {removed: count, selection: {glyph, layer}}",
            has_doc,
            remove
        )
        .targets(&[TargetKind::Layer, TargetKind::Guides])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.toggleMeasurement",
            "Measurement Guide",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guide: {scope: \"local\" | \"global\", index}, on?: bool (default: the opposite of the guide's present state)} make a guide a measurement guide, which shows the distances between its crossings with the outlines (`measure.ruler` with `guide`), or a plain one. A locked guide refuses it → {measurement: bool}",
            has_doc,
            toggle_measurement
        )
        .targets(&[TargetKind::Layer])
        .context(&["canvas.guide"]),
        cmd!(
            query "guide.list",
            "List Guides",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} every guide the layer shows, its own (local) then its master's (global), with all their properties → {glyph, layer, master, guides: [{scope, index, x, y, angle, locked, angleLocked, measurement, relativeTo: \"left\" | \"right\" | \"both\", name?, filter?, color?}]}",
            has_doc,
            list
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "guide.displayLines",
            "Display Lines",
            [],
            None,
            "{glyph: name, layer?: id} the lines Edit View draws for a layer besides guides: the grid, the master's metrics with the per-script ascender, cap height and descender parameters and the small-cap and shoulder heights applied, the alignment zones, and the CJK grid, CJK guide and design grid parameters → {glyph, layer, master, width, grid: {spacing, subdivision, step}, metrics: [{kind, name?, filter?, y, overshoot, source: \"master\" | \"parameter\"}], zones: [{pos, size}], cjk?: {box: [x0, y0, x1, y1], rows?, columns?, guide?: [x0, y0, x1, y1]}, designGrid?: [numbers]}",
            has_doc,
            display_lines
        )
        .targets(&[TargetKind::Layer]),
    ]
}

// ── shared helpers ──────────────────────────────────────────────────────────────────────────────

fn master_mut<'f>(font: &'f mut Font, id: &str, p: &Params) -> Result<&'f mut Master> {
    Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == id).ok_or_else(|| p.bad(format!("there is no master `{id}`")))
}

/// A coordinate within the model's limits.
fn coord(p: &Params, key: &str) -> Result<Option<f64>> {
    match p.opt_f64(key)? {
        Some(v) if v.abs() > MAX_COORDINATE => Err(p.bad(format!("`{key}` must be within ±{MAX_COORDINATE}, got {v}"))),
        v => Ok(v),
    }
}

/// An angle in degrees, kept from 0 to 360.
fn norm_angle(a: f64) -> f64 {
    let r = a.rem_euclid(360.0);
    if r.is_finite() && r < 360.0 { r } else { 0.0 }
}

fn opt_bool(p: &Params, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => Err(p.bad(format!("`{key}` must be true or false, got {}", kind(other)))),
    }
}

/// A text property: `Some(None)` clears it (an empty string).
fn opt_text(p: &Params, key: &str) -> Result<Option<Option<String>>> {
    let Some(text) = p.opt_str(key)? else { return Ok(None) };
    if text.chars().count() > MAX_GUIDE_TEXT {
        return Err(p.bad(format!("`{key}` is longer than {MAX_GUIDE_TEXT} characters")));
    }
    Ok(Some((!text.is_empty()).then(|| text.to_string())))
}

/// One `{scope, index}` guide address.
pub(super) fn guide_ref(p: &Params, key: &str) -> Result<GuideRef> {
    let bad = || p.bad(format!("`{key}` must be {{scope: \"local\" | \"global\", index}}"));
    let Some(Value::Object(o)) = p.get(key) else { return Err(bad()) };
    let scope = match o.get("scope").and_then(Value::as_str) {
        Some("local") => GuideScope::Local,
        Some("global") => GuideScope::Global,
        _ => return Err(bad()),
    };
    let index = o.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(bad)?;
    Ok(GuideRef { scope, index })
}

/// The target object of a command that acts on guides only: a `guides` list, no other list.
fn guide_targets(p: &Params) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.guides.is_empty() {
        return Err(p.bad(format!("`guides` is required: {LIST_DOC}")));
    }
    if !(t.points.is_empty() && t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes only `guides` as a target list"));
    }
    Ok(t)
}

/// `t` without its lists: after a first change the addresses no longer hold, so later steps of
/// the same edit address the layer only (which still checks the glyph lock).
fn bare(t: &Targets) -> Targets {
    let mut b = t.clone();
    b.guides.clear();
    b
}

fn indices(t: &Targets, scope: GuideScope) -> Vec<usize> {
    t.guides.iter().filter(|g| g.scope == scope).map(|g| g.index).collect()
}

fn locked_error(p: &Params, scope: GuideScope, index: usize) -> EngineError {
    p.bad(format!("{} guide {index} is locked: unlock it first (guide.unlock)", scope.as_str()))
}

fn gone(p: &Params, scope: GuideScope, index: usize) -> EngineError {
    p.bad(format!("{} guide {index} does not resolve", scope.as_str()))
}

/// Runs `f` on every addressed guide. Call it inside [`Session::edit`] after resolving `t`.
fn apply(font: &mut Font, t: &Targets, master: &str, p: &Params, mut f: impl FnMut(&mut Guide) -> Result<()>) -> Result<()> {
    let local = indices(t, GuideScope::Local);
    if !local.is_empty() {
        bare(t).edit_layer(font, |l| {
            let guides = l.parts_mut(t.side).guides;
            for &i in &local {
                f(guides.get_mut(i).ok_or_else(|| gone(p, GuideScope::Local, i))?)?;
            }
            Ok(())
        })?;
    }
    let global = indices(t, GuideScope::Global);
    if !global.is_empty() {
        let guides = &mut master_mut(font, master, p)?.guides;
        for &i in &global {
            f(guides.get_mut(i).ok_or_else(|| gone(p, GuideScope::Global, i))?)?;
        }
    }
    Ok(())
}

/// Removes the addressed guides of one scope and returns them in list order. A locked guide
/// refuses it.
fn take(font: &mut Font, t: &Targets, master: &str, p: &Params, scope: GuideScope) -> Result<Vec<Guide>> {
    let mut idx = indices(t, scope);
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    idx.sort_unstable();
    let take_from = |guides: &mut Vec<Guide>| -> Result<Vec<Guide>> {
        if let Some(&i) = idx.iter().find(|i| guides.get(**i).is_some_and(|g| g.locked)) {
            return Err(locked_error(p, scope, i));
        }
        let mut out = Vec::with_capacity(idx.len());
        for &i in idx.iter().rev() {
            if i >= guides.len() {
                return Err(gone(p, scope, i));
            }
            out.push(guides.remove(i));
        }
        out.reverse();
        Ok(out)
    };
    match scope {
        GuideScope::Local => bare(t).edit_layer(font, |l| take_from(l.parts_mut(t.side).guides)),
        GuideScope::Global => take_from(&mut master_mut(font, master, p)?.guides),
    }
}

/// Appends guides to a list, within [`MAX_LAYER_GUIDES`]; returns the first new index.
fn append(p: &Params, guides: &mut Vec<Guide>, new: Vec<Guide>) -> Result<usize> {
    if guides.len().saturating_add(new.len()) > MAX_LAYER_GUIDES {
        return Err(p.bad(format!("at most {MAX_LAYER_GUIDES} guides are allowed in one layer or master")));
    }
    let first = guides.len();
    guides.extend(new);
    Ok(first)
}

fn relative_to(g: &Guide) -> &'static str {
    guide_follow::reference(g).name()
}

fn set_relative_to(g: &mut Guide, to: &str) {
    let to = match to {
        "right" => Reference::Right,
        "both" => Reference::Both,
        _ => Reference::Left,
    };
    guide_follow::set_reference(g, to);
}

/// A colour as `r,g,b,a`, each from 0 to 1 (the form a UFO guideline uses).
fn check_color(p: &Params, text: &str) -> Result<()> {
    let parts: Vec<&str> = text.split(',').collect();
    let ok = parts.len() == 4 && parts.iter().all(|c| c.trim().parse::<f64>().is_ok_and(|v| (0.0..=1.0).contains(&v)));
    if ok { Ok(()) } else { Err(p.bad(format!("`color` must be \"r,g,b,a\" with numbers from 0 to 1, got \"{text}\""))) }
}

fn guide_json(scope: GuideScope, index: usize, g: &Guide) -> Value {
    let mut v = json!({
        "scope": scope.as_str(),
        "index": index,
        "x": g.x,
        "y": g.y,
        "angle": g.angle,
        "locked": g.locked,
        "angleLocked": g.angle_locked,
        "measurement": g.show_measurement,
        "relativeTo": relative_to(g),
    });
    if let Some(o) = v.as_object_mut() {
        for (key, text) in [("name", &g.name), ("filter", &g.filter), ("color", &g.color)] {
            if let Some(text) = text {
                o.insert(key.into(), json!(text));
            }
        }
    }
    v
}

// ── adding ──────────────────────────────────────────────────────────────────────────────────────

/// `throughPoints`: two `[shape, node]` addresses.
fn through_points(p: &Params) -> Result<Option<[(usize, usize); 2]>> {
    let Some(list) = p.opt_array("throughPoints")? else { return Ok(None) };
    let pair = |v: &Value| -> Option<(usize, usize)> {
        match v.as_array()?.as_slice() {
            [s, n] => Some((usize::try_from(s.as_u64()?).ok()?, usize::try_from(n.as_u64()?).ok()?)),
            _ => None,
        }
    };
    match list.as_slice() {
        [a, b] => match (pair(a), pair(b)) {
            (Some(a), Some(b)) => Ok(Some([a, b])),
            _ => Err(p.bad("`throughPoints` must hold two [shape, node] pairs of whole numbers")),
        },
        _ => Err(p.bad(format!("`throughPoints` must hold exactly two [shape, node] pairs, got {}", list.len()))),
    }
}

fn point_at(p: &Params, parts: LayerParts, (s, n): (usize, usize)) -> Result<(f64, f64)> {
    let node = parts.path(s).and_then(|path| path.nodes.get(n)).ok_or_else(|| p.bad(format!("`throughPoints`: [{s}, {n}] does not resolve")))?;
    Ok((node.x, node.y))
}

/// A guide from the parameters: `x`, `y`, `angle`, or `throughPoints` in `parts`.
fn new_guide(p: &Params, parts: Option<LayerParts>, step: f64) -> Result<Guide> {
    let (x, y, angle) = (coord(p, "x")?, coord(p, "y")?, p.opt_f64("angle")?);
    let Some(through) = through_points(p)? else {
        return Ok(Guide {
            x: round_to_grid(x.unwrap_or(0.0), step),
            y: round_to_grid(y.unwrap_or(0.0), step),
            angle: norm_angle(angle.unwrap_or(0.0)),
            ..Guide::default()
        });
    };
    if x.is_some() || y.is_some() || angle.is_some() {
        return Err(p.bad("give `throughPoints`, or `x`, `y` and `angle`, not both"));
    }
    let parts = parts.ok_or_else(|| p.bad("`throughPoints` needs `glyph`"))?;
    let [a, b] = through;
    let ((x0, y0), (x1, y1)) = (point_at(p, parts, a)?, point_at(p, parts, b)?);
    if (x0, y0) == (x1, y1) {
        return Err(p.bad("the two `throughPoints` are at the same place: a guide needs two different points"));
    }
    Ok(Guide { x: x0, y: y0, angle: norm_angle((y1 - y0).atan2(x1 - x0).to_degrees()), ..Guide::default() })
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("guide.add", p)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it adds a guide to the layer"));
    }
    let font = s.font()?;
    let guide = new_guide(&p, Some(t.resolve(font)?.parts), grid(font))?;
    t.layer = Some(t.layer_id(font)?);
    let index = s.edit("Add Guide", |font| t.edit_layer(font, |l| append(&p, l.parts_mut(t.side).guides, vec![guide])))?;
    t.guides = vec![GuideRef { scope: GuideScope::Local, index }];
    Ok(json!({"guide": {"scope": "local", "index": index}, "selection": t.to_json()}))
}

fn add_global(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("guide.addGlobal", p)?;
    let font = s.font()?;
    let mut selection = None;
    let (layer_master, guide) = match p.get("glyph") {
        Some(_) => {
            let mut t = Targets::parse(&p)?;
            if t.has_lists() {
                return Err(p.bad("takes no target lists: it adds a guide to a master"));
            }
            let r = t.resolve(font)?;
            let guide = new_guide(&p, Some(r.parts), grid(font))?;
            let layer_master = r.master.id.clone();
            t.layer = Some(r.layer.id.clone());
            selection = Some(t);
            (Some(layer_master), guide)
        }
        None if p.get("layer").is_some() || p.get("side").is_some() => return Err(p.bad("`layer` and `side` need `glyph`")),
        None => (None, new_guide(&p, None, grid(font))?),
    };
    let master = match (p.opt_str("master")?, layer_master) {
        (Some(m), Some(lm)) if m != lm => {
            return Err(p.bad(format!("`master` is `{m}`, but the glyph's layer belongs to master `{lm}`")));
        }
        (Some(m), _) => font.master(m)?.id.clone(),
        (None, Some(lm)) => lm,
        (None, None) => font.default_master()?.id.clone(),
    };
    let index = s.edit("Add Global Guide", |font| append(&p, &mut master_mut(font, &master, &p)?.guides, vec![guide]))?;
    let mut out = json!({"master": master, "guide": {"scope": "global", "index": index}});
    if let (Some(mut t), Some(o)) = (selection, out.as_object_mut()) {
        t.guides = vec![GuideRef { scope: GuideScope::Global, index }];
        o.insert("selection".into(), t.to_json());
    }
    Ok(out)
}

// ── moving between scopes ───────────────────────────────────────────────────────────────────────

fn make_global(s: &mut Session, p: &Value) -> Result<Value> {
    move_scope(s, p, "guide.makeGlobal", "Make Global Guide", GuideScope::Local)
}

fn make_local(s: &mut Session, p: &Value) -> Result<Value> {
    move_scope(s, p, "guide.makeLocal", "Make Local Guide", GuideScope::Global)
}

/// Moves the addressed guides, all of scope `from`, to the other scope.
fn move_scope(s: &mut Session, p: &Value, id: &'static str, label: &str, from: GuideScope) -> Result<Value> {
    let p = Params::new(id, p)?;
    let mut t = guide_targets(&p)?;
    if t.guides.iter().any(|g| g.scope != from) {
        return Err(p.bad(format!("`guides` must all be {} guides", from.as_str())));
    }
    t.layer = Some(t.layer_id(s.font()?)?);
    let (to, first, count) = s.edit(label, |font| {
        let master = t.resolve(font)?.master.id.clone();
        let moved = take(font, &t, &master, &p, from)?;
        let count = moved.len();
        match from {
            GuideScope::Local => Ok((GuideScope::Global, append(&p, &mut master_mut(font, &master, &p)?.guides, moved)?, count)),
            GuideScope::Global => Ok((GuideScope::Local, bare(&t).edit_layer(font, |l| append(&p, l.parts_mut(t.side).guides, moved))?, count)),
        }
    })?;
    t.guides = (first..first.saturating_add(count)).map(|index| GuideRef { scope: to, index }).collect();
    Ok(json!({"selection": t.to_json()}))
}

// ── changing guides ─────────────────────────────────────────────────────────────────────────────

fn lock(s: &mut Session, p: &Value) -> Result<Value> {
    set_locked(s, p, "guide.lock", "Lock Guides", true)
}

fn unlock(s: &mut Session, p: &Value) -> Result<Value> {
    set_locked(s, p, "guide.unlock", "Unlock Guide", false)
}

fn set_locked(s: &mut Session, p: &Value, id: &'static str, label: &str, on: bool) -> Result<Value> {
    let p = Params::new(id, p)?;
    let t = guide_targets(&p)?;
    s.edit(label, |font| {
        let master = t.resolve(font)?.master.id.clone();
        apply(font, &t, &master, &p, |g| {
            g.locked = on;
            Ok(())
        })
    })?;
    Ok(json!({"guides": t.guides.len()}))
}

fn set_properties(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("guide.setProperties", p)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes one `guide`, not target lists"));
    }
    let r = guide_ref(&p, "guide")?;
    t.guides = vec![r];
    let (x, y) = (coord(&p, "x")?, coord(&p, "y")?);
    let angle = p.opt_f64("angle")?.map(norm_angle);
    let angle_locked = opt_bool(&p, "angleLocked")?;
    let relative = match p.opt_str("relativeTo")? {
        None => None,
        Some(v @ ("left" | "right" | "both")) => Some(v),
        Some(other) => return Err(p.bad(format!("`relativeTo` must be \"left\", \"right\" or \"both\", got \"{other}\""))),
    };
    let measurement = opt_bool(&p, "measurement")?;
    let name = opt_text(&p, "name")?;
    let filter = opt_text(&p, "filter")?;
    if filter.is_some() && r.scope == GuideScope::Local {
        return Err(p.bad("only a global guide has a `filter`"));
    }
    let color = opt_text(&p, "color")?;
    if let Some(Some(c)) = &color {
        check_color(&p, c)?;
    }
    let step = grid(s.font()?);
    s.edit("Set Guide Properties", |font| {
        let master = t.resolve(font)?.master.id.clone();
        apply(font, &t, &master, &p, |g| {
            if g.locked {
                return Err(locked_error(&p, r.scope, r.index));
            }
            if let Some(a) = angle
                && g.angle_locked
                && angle_locked != Some(false)
                && a != g.angle
            {
                return Err(p.bad("the guide's angle is locked: set `angleLocked: false` in the same call to change it"));
            }
            if let Some(v) = x {
                g.x = round_to_grid(v, step);
            }
            if let Some(v) = y {
                g.y = round_to_grid(v, step);
            }
            if let Some(a) = angle {
                g.angle = a;
            }
            if let Some(v) = angle_locked {
                g.angle_locked = v;
            }
            if let Some(to) = relative {
                set_relative_to(g, to);
            }
            if let Some(v) = measurement {
                g.show_measurement = v;
            }
            if let Some(v) = &name {
                g.name.clone_from(v);
            }
            if let Some(v) = &filter {
                g.filter.clone_from(v);
            }
            if let Some(v) = &color {
                g.color.clone_from(v);
            }
            Ok(())
        })
    })?;
    Ok(json!({"guide": {"scope": r.scope.as_str(), "index": r.index}}))
}

fn toggle_measurement(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("guide.toggleMeasurement", p)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes one `guide`, not target lists"));
    }
    let r = guide_ref(&p, "guide")?;
    t.guides = vec![r];
    let on = match opt_bool(&p, "on")? {
        Some(on) => on,
        None => {
            let resolved = t.resolve(s.font()?)?;
            let list = match r.scope {
                GuideScope::Local => resolved.parts.guides,
                GuideScope::Global => resolved.master.guides.as_slice(),
            };
            !list.get(r.index).is_some_and(|g| g.show_measurement)
        }
    };
    s.edit("Measurement Guide", |font| {
        let master = t.resolve(font)?.master.id.clone();
        apply(font, &t, &master, &p, |g| {
            if g.locked {
                return Err(locked_error(&p, r.scope, r.index));
            }
            g.show_measurement = on;
            Ok(())
        })
    })?;
    Ok(json!({"measurement": on}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("guide.remove", p)?;
    let mut t = guide_targets(&p)?;
    t.layer = Some(t.layer_id(s.font()?)?);
    let removed = s.edit("Remove Guides", |font| {
        let master = t.resolve(font)?.master.id.clone();
        let local = take(font, &t, &master, &p, GuideScope::Local)?;
        let global = take(font, &t, &master, &p, GuideScope::Global)?;
        Ok(local.len().saturating_add(global.len()))
    })?;
    t.guides.clear();
    Ok(json!({"removed": removed, "selection": t.to_json()}))
}

// ── queries ─────────────────────────────────────────────────────────────────────────────────────

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("guide.list", p)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it returns every guide of the layer"));
    }
    let r = t.resolve(s.font()?)?;
    let guides: Vec<Value> = r
        .parts
        .guides
        .iter()
        .enumerate()
        .map(|(i, g)| guide_json(GuideScope::Local, i, g))
        .chain(r.master.guides.iter().enumerate().map(|(i, g)| guide_json(GuideScope::Global, i, g)))
        .collect();
    Ok(json!({"glyph": r.glyph.name, "layer": r.layer.id, "master": r.master.id, "guides": guides}))
}

fn display_lines(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("guide.displayLines", p)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it describes the whole layer"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    let master = r.master;
    let script = shown_metrics::glyph_script(r.glyph);
    let script = script.as_deref();
    let param = |name: &str, plain: bool| script_param(font, master, name, script, plain).as_deref().and_then(param_number);

    let shown = shown_metrics::shown_metrics(font, master, r.glyph);
    let ascender = shown_metrics::shown_pos(&shown, metric::ASCENDER, font.metric_pos(master, metric::ASCENDER, 800.0));
    let descender = shown_metrics::shown_pos(&shown, metric::DESCENDER, font.metric_pos(master, metric::DESCENDER, -200.0));
    let metrics: Vec<Value> = shown
        .iter()
        .map(|m| {
            let source = if m.from_param { "parameter" } else { "master" };
            let mut line = json!({"kind": m.kind, "y": m.y, "overshoot": m.over, "source": source});
            if let Some(o) = line.as_object_mut() {
                if let Some(name) = &m.name {
                    o.insert("name".into(), json!(name));
                }
                if let Some(filter) = &m.filter {
                    o.insert("filter".into(), json!(filter));
                }
            }
            line
        })
        .collect();
    let settings = &font.info.settings;
    let grid = json!({"spacing": settings.grid_length.unwrap_or(1.0), "subdivision": settings.grid_sub_division.unwrap_or(1.0), "step": grid(font)});
    let zones: Vec<Value> = master.zones.iter().map(|z| json!({"pos": z.pos, "size": z.size})).collect();
    let mut out = json!({
        "glyph": r.glyph.name,
        "layer": r.layer.id,
        "master": master.id,
        "width": r.layer.width,
        "grid": grid,
        "metrics": metrics,
        "zones": zones,
    });

    let divisions = |name: &str| param(name, true).filter(|n| (1.0..=MAX_CJK_DIVISIONS).contains(n)).map(f64::round);
    let both = divisions("CJK Grid");
    let (columns, rows) = (divisions("CJK Grid Horizontal").or(both), divisions("CJK Grid Vertical").or(both));
    let inset = param("CJK Guide", true).filter(|n| (0.0..50.0).contains(n));
    if columns.is_some() || rows.is_some() || inset.is_some() {
        let (x0, x1, y0, y1) = (0.0, r.layer.width, descender, ascender);
        let mut cjk = json!({"box": [x0, y0, x1, y1]});
        if let Some(o) = cjk.as_object_mut() {
            if let Some(c) = columns {
                o.insert("columns".into(), json!(c));
            }
            if let Some(n) = rows {
                o.insert("rows".into(), json!(n));
            }
            if let Some(pct) = inset {
                let (dx, dy) = ((x1 - x0) * pct / 100.0, (y1 - y0) * pct / 100.0);
                o.insert("guide".into(), json!([x0 + dx, y0 + dy, x1 - dx, y1 - dy]));
            }
        }
        if let Some(o) = out.as_object_mut() {
            o.insert("cjk".into(), cjk);
        }
    }
    let design = script_param(font, master, "Design Grid", script, true)
        .map(|v| v.split(',').take(MAX_DESIGN_GRID_VALUES.saturating_add(1)).map(param_number).collect::<Option<Vec<f64>>>());
    if let (Some(Some(values)), Some(o)) = (design, out.as_object_mut())
        && !values.is_empty()
        && values.len() <= MAX_DESIGN_GRID_VALUES
    {
        o.insert("designGrid".into(), json!(values));
    }
    Ok(out)
}
