//! Power guides and guide extras (M13.36). Rows: `guide.makePowerGuide`,
//! `guide.addPowerGuides`, `guide.unlinkPowerGuides`, `guide.movePowerGuide`,
//! `guide.powerGuideScope`, `guide.setThickness`, `guide.addMarkers`, `guide.setAppearance`.
//!
//! A power guide is a guide with links to nodes ([`fontcraft_model::guide_data`]). Moving it
//! with `guide.movePowerGuide` moves its linked nodes by the same distance, at right angles to
//! the guide. Unlinked nodes between two parallel power guides of the same layer move in
//! proportion to where they sit between them, as stretching the space between the guides would
//! move them. Nodes beyond the outermost power guides stay where they are. An off-curve node
//! whose on-curve neighbour (the one it belongs to) is linked moves with that neighbour. Links to
//! guides that are not parallel to the moved one play no part in the move.
//!
//! Local guides link nodes of their own layer side. Global guides link nodes of the master
//! layers (foreground) of any glyph, so one move reshapes every linked glyph, as one undo step;
//! a locked glyph keeps its outline and the result names it. Metric lines and alignment zones
//! take no links.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_geom::{BezPath, Line, Point, Vec2, round_to_grid};
use fontcraft_model::guide_data::{self, Link, MAX_GUIDE_MARKERS, MAX_MAGNETISM};
use fontcraft_model::{Font, Guide, LayerSide, MAX_COORDINATE, MAX_LAYER_GUIDES, Path, Shape};
use fontcraft_pathops::stems::{StemDirection, StemOptions, detect_stems};
use serde_json::{Value, json};

use super::common::{each_layer, grid, in_range, scope};
use super::guide::guide_ref;
use super::*;
use crate::EngineError;
use crate::params::{GuideRef, GuideScope, MAX_TARGETS, Params, TargetKind, Targets, kind};

/// A node lies on a guide when it is at most this far from it, in font units.
pub const LINK_TOLERANCE: f64 = 0.5;
/// Two guides are parallel when their angles differ by at most this many degrees (or 180 less).
const PARALLEL_DEGREES: f64 = 0.01;
/// Markers closer than this, in font units, are one marker.
const MARKER_MERGE: f64 = 0.01;

const GUIDE_DOC: &str = "{scope: \"local\" | \"global\", index}";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "guide.makePowerGuide",
            "Make Power Guide",
            ["Path", "Guides"],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", guides: [{scope: \"local\" | \"global\", index}…], nodes?: [[shape, node]…]} link nodes to each guide, making it a power guide: the nodes given, else every on-curve node of an unlocked path lying on the guide (within 0.5 units). Moving a power guide (guide.movePowerGuide) moves its linked nodes with it. A global guide links nodes of a master layer's foreground only. A locked guide or a locked glyph refuses it, and so does a guide on which no node lies → {guides: [{scope, index, links, added}]}",
            has_doc,
            make_power_guide
        )
        .targets(&[TargetKind::Layer, TargetKind::Guides])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.addPowerGuides",
            "Add Power Guides",
            ["Path", "Guides"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer)} add a local power guide along each edge of every horizontal and vertical stem of the layers' closed, unlocked paths, linked to the on-curve nodes lying on it. An existing parallel guide at the same place takes the links instead of a new one. Locked glyphs are skipped → {glyphs, skipped: [{glyph, layer?, reason}], guides: added, links: added}",
            has_doc,
            add_power_guides
        ),
        cmd!(
            "guide.unlinkPowerGuides",
            "Unlink Power Guides",
            ["Path", "Guides"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids], side?: \"foreground\" | \"background\", guides?: [{scope, index}…], nodes?: [[shape, node]…]} remove power-guide links without moving anything; the guides stay as ordinary guides. With `guides`, every link of those guides goes (a global guide's links in other glyphs too); with `nodes`, only the links to those nodes; with neither, every link of each glyph's layers: its local guides' links and its entries in its masters' global guides. `guides` and `nodes` need one `glyph`. A locked guide named in `guides` refuses it; other locked guides keep their links. Locked glyphs refuse it (one `glyph`) or are skipped → {unlinked: count, glyphs, skipped}",
            has_doc,
            unlink_power_guides
        )
        .targets(&[TargetKind::Layer, TargetKind::Guides])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.movePowerGuide",
            "Move Power Guide",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guide: {scope, index}, x?: number, y?: number} move a guide's knob to (x, y), rounded to the font's grid, and reshape the outlines its links hold: linked nodes move with the guide, unlinked nodes between two parallel power guides of the same layer move in proportion, nodes beyond the outermost power guides stay. A global guide reshapes every glyph it links, as one undo step; locked glyphs keep their outline and are listed in `skipped`. A locked guide refuses it, and a locked glyph refuses a move of its local guide → {guide: {scope, index}, distance, glyphs, skipped: [{glyph, reason}], nodes: moved}",
            has_doc,
            move_power_guide
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "guide.setThickness",
            "Set Guide Thickness",
            ["Path", "Guides"],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guide: {scope, index}, thickness: number} give a guide a second edge `thickness` units away, at right angles: on the left of the guide's direction when positive, on the right when negative, from -10000 to 10000. The band between the edges shows on the canvas; 0 removes it. Kept in the guide's user data. A locked guide refuses it → {guide: {scope, index}, thickness}",
            has_doc,
            set_thickness
        )
        .targets(&[TargetKind::Layer])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.addMarkers",
            "Add Guide Markers",
            ["Path", "Guides"],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guide: {scope, index}, positions?: [number], auto?: bool, clear?: bool} add markers along a guide: at the distances given (font units from the guide's knob, in the guide's direction), and with `auto: true` wherever the guide crosses the layer side's paths. `clear: true` removes every marker and takes neither. A guide holds at most 256 markers, kept in its user data. A locked guide refuses it → {guide: {scope, index}, markers: [distances]}",
            has_doc,
            add_markers
        )
        .targets(&[TargetKind::Layer])
        .context(&["canvas.guide"]),
        cmd!(
            "guide.setAppearance",
            "Guide Colour and Magnetism",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", guide: {scope, index}, color?: \"r,g,b,a\" | [r, g, b, a] (numbers from 0 to 1; \"\" or null clears it), magnetism?: number (screen points from 0 to 100 within which dragged points snap to the guide; 0 turns it off), showMeasurements?: bool} set a guide's own colour, its magnetism (kept in its user data) and whether it shows the distances where it crosses the outlines. A locked guide refuses it → {guide: {scope, index}, color: text | null, magnetism: number | null, showMeasurements}",
            has_doc,
            set_appearance
        )
        .targets(&[TargetKind::Layer])
        .context(&["canvas.guide"]),
    ]
}

// ── geometry ────────────────────────────────────────────────────────────────────────────────────

fn knob(g: &Guide) -> Point {
    Point::new(g.x, g.y)
}

/// The guide's unit direction.
fn direction(g: &Guide) -> Vec2 {
    let a = if g.angle.is_finite() { g.angle.to_radians() } else { 0.0 };
    Vec2::new(a.cos(), a.sin())
}

/// The guide's unit normal: its direction turned a quarter turn counter-clockwise.
fn normal(g: &Guide) -> Vec2 {
    let d = direction(g);
    Vec2::new(-d.y, d.x)
}

/// Whether `p` lies on the guide.
fn on_guide(g: &Guide, p: Point) -> bool {
    (p - knob(g)).dot(normal(g)).abs() <= LINK_TOLERANCE
}

fn parallel(a: &Guide, b: &Guide) -> bool {
    let d = (a.angle - b.angle).rem_euclid(180.0);
    d.is_finite() && (d <= PARALLEL_DEGREES || d >= 180.0 - PARALLEL_DEGREES)
}

/// The on-curve nodes of the unlocked paths in `shapes` that lie on the guide.
fn nodes_on(shapes: &[Shape], g: &Guide) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (si, shape) in shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        if path.info.locked {
            continue;
        }
        for (ni, n) in path.nodes.iter().enumerate() {
            if n.on_curve() && on_guide(g, n.point()) {
                out.push((si, ni));
            }
        }
    }
    out
}

/// One power guide as a move sees it: where it was along the moved guide's normal, how far it
/// moves, and the nodes it holds in the layer being reshaped.
struct Anchor {
    at: f64,
    delta: f64,
    nodes: Vec<(usize, usize)>,
}

/// The on-curve node an off-curve node belongs to: the one before it, else the one after it.
fn owner(path: &Path, i: usize) -> Option<usize> {
    let count = path.nodes.len();
    let prev = if i > 0 {
        Some(i - 1)
    } else if path.closed && count > 0 {
        Some(count - 1)
    } else {
        None
    };
    let next = if i + 1 < count {
        Some(i + 1)
    } else if path.closed {
        Some(0)
    } else {
        None
    };
    let on = |j: Option<usize>| j.filter(|j| *j != i && path.nodes.get(*j).is_some_and(|n| n.on_curve()));
    on(prev).or_else(|| on(next))
}

/// How far a node at `u` along the normal moves: in proportion between the nearest anchors on
/// either side, not at all beyond the outermost ones.
fn spaced(bounds: &[(f64, f64)], u: f64) -> f64 {
    let lo = bounds.iter().filter(|(at, _)| *at <= u).max_by(|a, b| a.0.total_cmp(&b.0));
    let hi = bounds.iter().filter(|(at, _)| *at >= u).min_by(|a, b| a.0.total_cmp(&b.0));
    match (lo, hi) {
        (Some(&(a, da)), Some(&(b, db))) => {
            let span = b - a;
            if span.abs() < 1e-9 { da } else { da + (u - a) / span * (db - da) }
        }
        _ => 0.0,
    }
}

/// Moves the nodes of the unlocked paths in `shapes` for a guide move along `n`, measured from
/// `origin`. The first anchor is the moved guide: its links win over the others'. Returns how
/// many nodes moved.
fn reshape(shapes: &mut [Shape], origin: Point, n: Vec2, anchors: &[Anchor]) -> std::result::Result<usize, String> {
    let mut linked: HashMap<(usize, usize), f64> = HashMap::new();
    for a in anchors {
        for key in &a.nodes {
            linked.entry(*key).or_insert(a.delta);
        }
    }
    let bounds: Vec<(f64, f64)> = anchors.iter().map(|a| (a.at, a.delta)).collect();
    let mut moved = 0usize;
    for (si, shape) in shapes.iter_mut().enumerate() {
        let Shape::Path(path) = shape else { continue };
        if path.info.locked {
            continue;
        }
        let deltas: Vec<f64> = (0..path.nodes.len())
            .map(|i| {
                let Some(node) = path.nodes.get(i) else { return 0.0 };
                if node.locked {
                    return 0.0;
                }
                if let Some(d) = linked.get(&(si, i)) {
                    return *d;
                }
                if !node.on_curve()
                    && let Some(d) = owner(path, i).and_then(|o| linked.get(&(si, o)))
                {
                    return *d;
                }
                spaced(&bounds, (node.point() - origin).dot(n))
            })
            .collect();
        for (node, d) in path.nodes.iter_mut().zip(deltas) {
            if d == 0.0 {
                continue;
            }
            let (x, y) = (node.x + n.x * d, node.y + n.y * d);
            if !(in_range(x) && in_range(y)) {
                return Err(format!("a node would move to ({x}, {y}), beyond the coordinate limit of {MAX_COORDINATE}"));
            }
            (node.x, node.y) = (x, y);
            moved += 1;
        }
    }
    Ok(moved)
}

/// Where the guide crosses the paths in `shapes`: distances from its knob along its direction,
/// sorted, near repeats merged.
fn crossings(shapes: &[Shape], g: &Guide) -> Vec<f64> {
    let (k, d) = (knob(g), direction(g));
    let reach = 4.0 * MAX_COORDINATE;
    let line = Line::new(k - d * reach, k + d * reach);
    let mut out = Vec::new();
    for shape in shapes {
        let Shape::Path(path) = shape else { continue };
        let Ok(bez) = path.to_bez() else { continue };
        for seg in bez.segments() {
            for hit in seg.intersect_line(line) {
                let along = ((hit.line_t * 2.0 - 1.0) * reach * 100.0).round() / 100.0;
                if along.is_finite() {
                    out.push(along);
                }
            }
        }
    }
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < MARKER_MERGE);
    out
}

// ── parameters and access ───────────────────────────────────────────────────────────────────────

fn guide_json(r: GuideRef) -> Value {
    json!({"scope": r.scope.as_str(), "index": r.index})
}

/// `glyph`, `layer?`, `side?` and one `guide`, without target lists.
fn single_guide(p: &Params) -> Result<(Targets, GuideRef)> {
    let mut t = Targets::parse(p)?;
    if t.has_lists() {
        return Err(p.bad(format!("takes one `guide`: {GUIDE_DOC}, not target lists")));
    }
    let r = guide_ref(p, "guide")?;
    t.guides = vec![r];
    Ok((t, r))
}

/// A `nodes` list of `[shape, node]` pairs, repeats dropped.
fn node_list(p: &Params, key: &str) -> Result<Option<Vec<(usize, usize)>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_TARGETS {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {MAX_TARGETS} are allowed", list.len())));
    }
    let index = |v: &Value| v.as_u64().and_then(|n| usize::try_from(n).ok());
    let mut out = Vec::with_capacity(list.len());
    for v in list {
        let pair = match v.as_array().map(Vec::as_slice) {
            Some([s, n]) => index(s).zip(index(n)),
            _ => None,
        };
        let pair = pair.ok_or_else(|| p.bad(format!("`{key}` must hold [shape, node] pairs, found {}", kind(v))))?;
        if !out.contains(&pair) {
            out.push(pair);
        }
    }
    Ok(Some(out))
}

fn locked_guide(p: &Params, r: GuideRef) -> EngineError {
    p.bad(format!("{} guide {} is locked: unlock it first (guide.unlock)", r.scope.as_str(), r.index))
}

/// The guide `r` of the target layer, and the layer's master id.
fn read_guide(font: &Font, t: &Targets, r: GuideRef, p: &Params) -> Result<(Guide, String)> {
    let rt = t.resolve(font)?;
    let list = match r.scope {
        GuideScope::Local => rt.parts.guides,
        GuideScope::Global => rt.master.guides.as_slice(),
    };
    let g = list.get(r.index).ok_or_else(|| p.bad(format!("{} guide {} does not resolve", r.scope.as_str(), r.index)))?;
    Ok((g.clone(), rt.master.id.clone()))
}

/// Runs `f` on guide `r` of the target layer, for editing. A local guide checks the glyph lock;
/// a global one belongs to the master. Call inside [`Session::edit`].
fn edit_guide<T>(font: &mut Font, t: &Targets, r: GuideRef, p: &Params, f: impl FnOnce(&mut Guide) -> Result<T>) -> Result<T> {
    let master = t.resolve(font)?.master.id.clone();
    let gone = || p.bad(format!("{} guide {} does not resolve", r.scope.as_str(), r.index));
    match r.scope {
        GuideScope::Local => t.edit_layer(font, |l| f(l.parts_mut(t.side).guides.get_mut(r.index).ok_or_else(gone)?)),
        GuideScope::Global => {
            let masters = Arc::make_mut(&mut font.masters);
            let m = masters.iter_mut().find(|m| m.id == master).ok_or_else(|| p.bad(format!("there is no master `{master}`")))?;
            f(m.guides.get_mut(r.index).ok_or_else(gone)?)
        }
    }
}

/// [`edit_guide`] for a change to one guide that a locked guide refuses.
fn change_guide<T>(font: &mut Font, t: &Targets, r: GuideRef, p: &Params, f: impl FnOnce(&mut Guide) -> Result<T>) -> Result<T> {
    edit_guide(font, t, r, p, |g| {
        if g.locked {
            return Err(locked_guide(p, r));
        }
        f(g)
    })
}

fn data_error<'p>(p: &'p Params<'_>) -> impl Fn(String) -> EngineError + 'p {
    move |e| p.bad(e)
}

// ── commands ────────────────────────────────────────────────────────────────────────────────────

fn make_power_guide(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.makePowerGuide", v)?;
    let t = Targets::parse(&p)?;
    if t.guides.is_empty() {
        return Err(p.bad(format!("`guides` is required: [{GUIDE_DOC}…]")));
    }
    if !(t.points.is_empty() && t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `guides` and `nodes` only"));
    }
    let nodes = node_list(&p, "nodes")?;
    let font = s.font()?;
    let rt = t.resolve(font)?;
    crate::check_unlocked(rt.glyph)?;
    if let Some(list) = &nodes {
        for &(si, ni) in list {
            if rt.parts.path(si).and_then(|path| path.nodes.get(ni)).is_none() {
                return Err(p.bad(format!("`nodes`: [{si}, {ni}] does not resolve to a node of a path")));
            }
        }
    }
    let master_layer = rt.layer.is_master_layer() && t.side == LayerSide::Foreground;
    let mut plan = Vec::with_capacity(t.guides.len());
    for &r in &t.guides {
        let (g, _) = read_guide(font, &t, r, &p)?;
        if g.locked {
            return Err(locked_guide(&p, r));
        }
        if r.scope == GuideScope::Global && !master_layer {
            return Err(p.bad("a global guide links nodes of a master layer's foreground only"));
        }
        let picked = match &nodes {
            Some(list) => list.clone(),
            None => nodes_on(rt.parts.shapes, &g),
        };
        if picked.is_empty() {
            return Err(p.bad(format!("no node lies on {} guide {}: give `nodes`", r.scope.as_str(), r.index)));
        }
        let mut links = guide_data::links(&g);
        let before = links.len();
        for (si, ni) in picked {
            let link = match r.scope {
                GuideScope::Local => Link::local(si, ni),
                GuideScope::Global => Link::global(t.glyph.as_str(), si, ni),
            };
            if !links.contains(&link) {
                links.push(link);
            }
        }
        let added = links.len() - before;
        plan.push((r, links, added));
    }
    s.edit("Make Power Guide", |font| {
        for (r, links, _) in &plan {
            change_guide(font, &t, *r, &p, |g| guide_data::set_links(g, links).map_err(data_error(&p)))?;
        }
        Ok(())
    })?;
    let out: Vec<Value> =
        plan.iter().map(|(r, links, added)| json!({"scope": r.scope.as_str(), "index": r.index, "links": links.len(), "added": added})).collect();
    Ok(json!({ "guides": out }))
}

fn add_power_guides(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.addPowerGuides", v)?;
    let sc = scope(&p, s.font()?)?;
    let (mut guides, mut links) = (0usize, 0usize);
    let out = s.edit("Add Power Guides", |font| {
        each_layer(font, &sc, p.command(), |_, layer| {
            let mut outline = BezPath::new();
            for shape in &layer.shapes {
                let Shape::Path(path) = shape else { continue };
                if !path.closed || path.info.locked {
                    continue;
                }
                if let Ok(bez) = path.to_bez() {
                    for el in bez.elements() {
                        outline.push(*el);
                    }
                }
            }
            if outline.elements().is_empty() {
                return Ok(Some("the layer has no closed, unlocked paths".into()));
            }
            let mut lines: Vec<(bool, f64)> = Vec::new();
            for stem in detect_stems(&outline, &StemOptions::default()) {
                let vertical = match stem.direction {
                    StemDirection::Vertical => true,
                    StemDirection::Horizontal => false,
                    StemDirection::Diagonal => continue,
                };
                for edge in [stem.first, stem.second] {
                    let mid = if vertical { (edge.p0.x + edge.p1.x) / 2.0 } else { (edge.p0.y + edge.p1.y) / 2.0 };
                    let at = (mid * 100.0).round() / 100.0;
                    if at.is_finite() && !lines.iter().any(|(v, x)| *v == vertical && (x - at).abs() <= LINK_TOLERANCE) {
                        lines.push((vertical, at));
                    }
                }
            }
            let (before_guides, before_links) = (guides, links);
            for (vertical, at) in lines {
                let new = Guide {
                    x: if vertical { at } else { 0.0 },
                    y: if vertical { 0.0 } else { at },
                    angle: if vertical { 90.0 } else { 0.0 },
                    ..Guide::default()
                };
                let on = nodes_on(&layer.shapes, &new);
                if on.is_empty() {
                    continue;
                }
                let list = &mut layer.info.guides;
                let index = match list.iter().position(|g| !g.locked && parallel(g, &new) && on_guide(g, knob(&new))) {
                    Some(i) => i,
                    None => {
                        if list.len() >= MAX_LAYER_GUIDES {
                            return Err(p.bad(format!("a layer holds at most {MAX_LAYER_GUIDES} guides")));
                        }
                        list.push(new);
                        guides += 1;
                        list.len() - 1
                    }
                };
                let Some(g) = list.get_mut(index) else { continue };
                let mut held = guide_data::links(g);
                for (si, ni) in on {
                    let link = Link::local(si, ni);
                    if !held.contains(&link) {
                        held.push(link);
                        links += 1;
                    }
                }
                guide_data::set_links(g, &held).map_err(data_error(&p))?;
            }
            if (guides, links) == (before_guides, before_links) {
                return Ok(Some("no horizontal or vertical stem with nodes on its edges".into()));
            }
            Ok(None)
        })
    })?;
    let mut v = out.to_json();
    if let Some(o) = v.as_object_mut() {
        o.insert("guides".into(), json!(guides));
        o.insert("links".into(), json!(links));
    }
    Ok(v)
}

/// Removes from `g` the links `drop` picks; how many went.
fn drop_links(g: &mut Guide, p: &Params, drop: impl Fn(&Link) -> bool) -> Result<usize> {
    let all = guide_data::links(g);
    let keep: Vec<Link> = all.iter().filter(|l| !drop(l)).cloned().collect();
    let gone = all.len() - keep.len();
    if gone > 0 {
        guide_data::set_links(g, &keep).map_err(data_error(p))?;
    }
    Ok(gone)
}

fn unlink_power_guides(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.unlinkPowerGuides", v)?;
    let nodes = node_list(&p, "nodes")?;
    let named = p.get("guides").is_some() || nodes.is_some();
    let mut unlinked = 0usize;
    let mut glyphs: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    if named {
        let t = Targets::parse(&p)?;
        if !(t.points.is_empty() && t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.annotations.is_empty()) {
            return Err(p.bad("takes `guides` and `nodes` only"));
        }
        let font = s.font()?;
        let rt = t.resolve(font)?;
        crate::check_unlocked(rt.glyph)?;
        let master_layer = rt.layer.is_master_layer() && t.side == LayerSide::Foreground;
        let explicit = !t.guides.is_empty();
        let refs: Vec<GuideRef> = if explicit {
            t.guides.clone()
        } else {
            let local = (0..rt.parts.guides.len()).map(|index| GuideRef { scope: GuideScope::Local, index });
            let global = (0..if master_layer { rt.master.guides.len() } else { 0 }).map(|index| GuideRef { scope: GuideScope::Global, index });
            local.chain(global).collect()
        };
        let mut bare = t.clone();
        bare.guides.clear();
        let glyph = t.glyph.clone();
        s.edit("Unlink Power Guides", |font| {
            for r in refs {
                unlinked += edit_guide(font, &bare, r, &p, |g| {
                    if g.locked {
                        return if explicit { Err(locked_guide(&p, r)) } else { Ok(0) };
                    }
                    let mine = |l: &Link| match r.scope {
                        GuideScope::Local => l.glyph.is_none(),
                        GuideScope::Global => master_layer && l.glyph.as_deref() == Some(glyph.as_str()),
                    };
                    drop_links(g, &p, |l| match &nodes {
                        Some(list) => mine(l) && list.contains(&(l.shape, l.node)),
                        None => explicit || mine(l),
                    })
                })?;
            }
            if unlinked == 0 {
                return Err(p.bad("there are no power-guide links to remove"));
            }
            Ok(())
        })?;
        glyphs.push(t.glyph);
    } else {
        let sc = scope(&p, s.font()?)?;
        let side = match p.opt_str("side")? {
            None => LayerSide::Foreground,
            Some(text) => LayerSide::parse(text).ok_or_else(|| p.bad(format!("`side` must be \"foreground\" or \"background\", got \"{text}\"")))?,
        };
        s.edit("Unlink Power Guides", |font| {
            for name in &sc.glyphs {
                let Some(glyph) = font.glyph(name) else { return Err(p.bad(format!("there is no glyph `{name}`"))) };
                if glyph.info.locked {
                    if sc.single {
                        return Err(EngineError::Locked(name.clone()));
                    }
                    skipped.push(json!({"glyph": name, "reason": "the glyph is locked"}));
                    continue;
                }
                let masters: Vec<String> = glyph
                    .layers
                    .iter()
                    .filter(|l| sc.layers.contains(&l.id) && l.is_master_layer() && side == LayerSide::Foreground)
                    .map(|l| l.master.clone())
                    .collect();
                let mut count = 0usize;
                {
                    let mut g = font.glyph_mut(name)?;
                    for layer in g.layers.iter_mut().filter(|l| sc.layers.contains(&l.id)) {
                        for guide in layer.parts_mut(side).guides.iter_mut().filter(|g| !g.locked) {
                            count += drop_links(guide, &p, |_| true)?;
                        }
                    }
                }
                if !masters.is_empty() {
                    for m in Arc::make_mut(&mut font.masters).iter_mut().filter(|m| masters.contains(&m.id)) {
                        for guide in m.guides.iter_mut().filter(|g| !g.locked) {
                            count += drop_links(guide, &p, |l| l.glyph.as_deref() == Some(name.as_str()))?;
                        }
                    }
                }
                if count > 0 {
                    glyphs.push(name.clone());
                    unlinked += count;
                }
            }
            if unlinked == 0 {
                return Err(p.bad("there are no power-guide links to remove"));
            }
            Ok(())
        })?;
    }
    Ok(json!({"unlinked": unlinked, "glyphs": glyphs, "skipped": skipped}))
}

/// The power guides a layer holds that are parallel to `moved` (before the move), as anchors of
/// a move of `delta` along `moved`'s normal. `moved_ref` is the moved guide's place among them;
/// it comes first.
fn anchors_for(
    moved: &Guide,
    delta: f64,
    moved_links: Vec<(usize, usize)>,
    local: &[Guide],
    global: &[Guide],
    glyph: &str,
    skip: Option<GuideRef>,
) -> Vec<Anchor> {
    let (origin, n) = (knob(moved), normal(moved));
    let mut out = vec![Anchor { at: 0.0, delta, nodes: moved_links }];
    let lists = [(GuideScope::Local, local), (GuideScope::Global, global)];
    for (scope, list) in lists {
        for (index, g) in list.iter().enumerate() {
            if skip == Some(GuideRef { scope, index }) || !parallel(g, moved) {
                continue;
            }
            let nodes: Vec<(usize, usize)> = guide_data::links(g)
                .into_iter()
                .filter(|l| match scope {
                    GuideScope::Local => l.glyph.is_none(),
                    GuideScope::Global => l.glyph.as_deref() == Some(glyph),
                })
                .map(|l| (l.shape, l.node))
                .collect();
            if nodes.is_empty() {
                continue;
            }
            let at = (knob(g) - origin).dot(n);
            if at.is_finite() {
                out.push(Anchor { at, delta: 0.0, nodes });
            }
        }
    }
    out
}

fn move_power_guide(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.movePowerGuide", v)?;
    let (t, r) = single_guide(&p)?;
    let (x, y) = (p.opt_f64("x")?, p.opt_f64("y")?);
    if x.is_none() && y.is_none() {
        return Err(p.bad("give `x`, `y` or both: the guide's new knob position"));
    }
    let font = s.font()?;
    let (old, master) = read_guide(font, &t, r, &p)?;
    if old.locked {
        return Err(locked_guide(&p, r));
    }
    let rt = t.resolve(font)?;
    if r.scope == GuideScope::Local {
        crate::check_unlocked(rt.glyph)?;
    }
    let step = grid(font);
    let (nx, ny) = (round_to_grid(x.unwrap_or(old.x), step), round_to_grid(y.unwrap_or(old.y), step));
    if !(in_range(nx) && in_range(ny)) {
        return Err(p.bad(format!("`x` and `y` must be within the coordinate limit of {MAX_COORDINATE}")));
    }
    let delta = (Point::new(nx, ny) - knob(&old)).dot(normal(&old));
    let links = guide_data::links(&old);
    // The layers the move reshapes: (glyph, layer id, side, the moved guide's nodes in it).
    let mut layers: Vec<(String, String, LayerSide, Vec<(usize, usize)>)> = Vec::new();
    match r.scope {
        GuideScope::Local => {
            let nodes = links.iter().filter(|l| l.glyph.is_none()).map(|l| (l.shape, l.node)).collect();
            layers.push((t.glyph.clone(), rt.layer.id.clone(), t.side, nodes));
        }
        GuideScope::Global => {
            for l in links.iter() {
                let Some(name) = &l.glyph else { continue };
                match layers.iter_mut().find(|(g, ..)| g == name) {
                    Some(entry) => entry.3.push((l.shape, l.node)),
                    None => layers.push((name.clone(), master.clone(), LayerSide::Foreground, vec![(l.shape, l.node)])),
                }
            }
        }
    }
    let mut done: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut moved_nodes = 0usize;
    s.edit("Move Power Guide", |font| {
        let global: Vec<Guide> = font.master(&master).map(|m| m.guides.clone()).unwrap_or_default();
        for (name, layer_id, side, nodes) in layers {
            let Some(glyph) = font.glyph(&name) else {
                skipped.push(json!({"glyph": name, "reason": "there is no such glyph"}));
                continue;
            };
            if glyph.info.locked {
                skipped.push(json!({"glyph": name, "reason": "the glyph is locked: its outline is kept"}));
                continue;
            }
            let Some(layer) = glyph.layers.iter().find(|l| l.id == layer_id) else {
                skipped.push(json!({"glyph": name, "reason": format!("the glyph has no layer `{layer_id}`")}));
                continue;
            };
            let local = layer.parts(side).guides.to_vec();
            let global_here: &[Guide] = if layer.is_master_layer() && side == LayerSide::Foreground { &global } else { &[] };
            let anchors = anchors_for(&old, delta, nodes, &local, global_here, &name, Some(r));
            let mut g = font.glyph_mut(&name)?;
            let Some(layer) = g.layers.iter_mut().find(|l| l.id == layer_id) else { continue };
            let count =
                reshape(layer.parts_mut(side).shapes, knob(&old), normal(&old), &anchors).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
            moved_nodes += count;
            done.push(name);
        }
        change_guide(font, &t, r, &p, |g| {
            (g.x, g.y) = (nx, ny);
            Ok(())
        })
    })?;
    Ok(json!({"guide": guide_json(r), "distance": delta, "glyphs": done, "skipped": skipped, "nodes": moved_nodes}))
}

fn set_thickness(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.setThickness", v)?;
    let (t, r) = single_guide(&p)?;
    let thickness = p.f64("thickness")?;
    s.edit("Set Guide Thickness", |font| change_guide(font, &t, r, &p, |g| guide_data::set_thickness(g, Some(thickness)).map_err(data_error(&p))))?;
    Ok(json!({"guide": guide_json(r), "thickness": thickness}))
}

fn add_markers(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.addMarkers", v)?;
    let (t, r) = single_guide(&p)?;
    let clear = p.bool_or("clear", false)?;
    let auto = p.bool_or("auto", false)?;
    let positions: Option<Vec<f64>> = match p.opt_array("positions")? {
        None => None,
        Some(list) => {
            if list.len() > MAX_GUIDE_MARKERS {
                return Err(p.bad(format!("`positions` holds {} values; a guide holds at most {MAX_GUIDE_MARKERS} markers", list.len())));
            }
            Some(
                list.iter()
                    .map(|v| {
                        v.as_f64().filter(|d| d.is_finite()).ok_or_else(|| p.bad(format!("`positions` must hold finite numbers, found {}", kind(v))))
                    })
                    .collect::<Result<_>>()?,
            )
        }
    };
    if clear && (auto || positions.is_some()) {
        return Err(p.bad("`clear` takes neither `positions` nor `auto`"));
    }
    if !clear && !auto && positions.is_none() {
        return Err(p.bad("give `positions`, `auto: true` or `clear: true`"));
    }
    let font = s.font()?;
    let (g, _) = read_guide(font, &t, r, &p)?;
    let mut markers = if clear { Vec::new() } else { guide_data::markers(&g) };
    for d in positions.unwrap_or_default() {
        markers.push(d);
    }
    if auto {
        let rt = t.resolve(font)?;
        markers.extend(crossings(rt.parts.shapes, &g));
    }
    markers.sort_by(f64::total_cmp);
    markers.dedup_by(|a, b| (*a - *b).abs() < MARKER_MERGE);
    if markers.len() > MAX_GUIDE_MARKERS {
        return Err(p.bad(format!("the guide would hold {} markers; at most {MAX_GUIDE_MARKERS} are allowed", markers.len())));
    }
    let label = if clear { "Clear Guide Markers" } else { "Add Guide Markers" };
    s.edit(label, |font| change_guide(font, &t, r, &p, |g| guide_data::set_markers(g, &markers).map_err(data_error(&p))))?;
    Ok(json!({"guide": guide_json(r), "markers": markers}))
}

/// A colour as the guide's `r,g,b,a` text, or `None` to clear it.
fn color_text(p: &Params) -> Result<Option<Option<String>>> {
    let bad = |found: &str| {
        p.bad(format!("`color` must be \"r,g,b,a\" or [r, g, b, a] with numbers from 0 to 1, or \"\" or null to clear it; got {found}"))
    };
    let numbers: Vec<f64> = match p.get("color") {
        None => return Ok(None),
        Some(Value::Null) => return Ok(Some(None)),
        Some(Value::String(text)) if text.trim().is_empty() => return Ok(Some(None)),
        Some(Value::String(text)) => text.split(',').map(|c| c.trim().parse::<f64>().ok()).collect::<Option<_>>().ok_or_else(|| bad(text))?,
        Some(Value::Array(items)) => items.iter().map(Value::as_f64).collect::<Option<_>>().ok_or_else(|| bad("a list that is not all numbers"))?,
        Some(other) => return Err(bad(kind(other))),
    };
    if numbers.len() != 4 || !numbers.iter().all(|c| (0.0..=1.0).contains(c)) {
        return Err(bad(&format!("{numbers:?}")));
    }
    let text: Vec<String> = numbers.iter().map(|c| format!("{}", (c * 1000.0).round() / 1000.0)).collect();
    Ok(Some(Some(text.join(","))))
}

fn set_appearance(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("guide.setAppearance", v)?;
    let (t, r) = single_guide(&p)?;
    let color = color_text(&p)?;
    let magnetism = p.opt_f64("magnetism")?;
    if let Some(m) = magnetism
        && !(0.0..=MAX_MAGNETISM).contains(&m)
    {
        return Err(p.bad(format!("`magnetism` must be from 0 to {MAX_MAGNETISM} screen points, got {m}")));
    }
    let measurements = match p.get("showMeasurements") {
        None => None,
        Some(_) => Some(p.bool_or("showMeasurements", false)?),
    };
    if color.is_none() && magnetism.is_none() && measurements.is_none() {
        return Err(p.bad("give `color`, `magnetism`, `showMeasurements` or several"));
    }
    let after = s.edit("Guide Appearance", |font| {
        change_guide(font, &t, r, &p, |g| {
            if let Some(c) = &color {
                g.color.clone_from(c);
            }
            if let Some(m) = magnetism {
                guide_data::set_magnetism(g, Some(m)).map_err(data_error(&p))?;
            }
            if let Some(on) = measurements {
                g.show_measurement = on;
            }
            Ok(g.clone())
        })
    })?;
    Ok(json!({
        "guide": guide_json(r),
        "color": after.color,
        "magnetism": guide_data::magnetism(&after),
        "showMeasurements": after.show_measurement,
    }))
}

#[cfg(test)]
mod tests {
    use fontcraft_model::Node;

    use super::*;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn add_path(s: &mut Session, glyph: &str, points: &[(f64, f64)]) -> usize {
        let nodes: Vec<Value> = points.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
        run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes, "closed": true}))["shape"].as_u64().unwrap() as usize
    }

    fn xs(s: &Session, glyph: &str) -> Vec<f64> {
        let font = s.font().unwrap();
        let id = font.default_master().unwrap().id.clone();
        let layer = font.glyph(glyph).unwrap().layer(&id).unwrap();
        match &layer.shapes[0] {
            Shape::Path(p) => p.nodes.iter().map(|n: &Node| n.x).collect(),
            other => panic!("not a path: {other:?}"),
        }
    }

    fn local_guide(s: &Session, glyph: &str, index: usize) -> Guide {
        let font = s.font().unwrap();
        let id = font.default_master().unwrap().id.clone();
        font.glyph(glyph).unwrap().layer(&id).unwrap().info.guides[index].clone()
    }

    fn global_guide(s: &Session, index: usize) -> Guide {
        s.font().unwrap().default_master().unwrap().guides[index].clone()
    }

    /// A bar from x 100 to 300 with nodes at x 100, 200 and 300.
    const BAR: [(f64, f64); 6] = [(100.0, 0.0), (100.0, 100.0), (200.0, 100.0), (300.0, 100.0), (300.0, 0.0), (200.0, 0.0)];

    fn vertical(s: &mut Session, glyph: &str, x: f64) -> Value {
        run(s, "guide.add", json!({"glyph": glyph, "x": x, "angle": 90}))["guide"].clone()
    }

    #[test]
    fn moving_a_power_guide_moves_its_nodes_and_spaces_the_rest_in_one_undo_step() {
        let mut s = Session::with_document();
        add_path(&mut s, "A", &BAR);
        let left = vertical(&mut s, "A", 100.0);
        let right = vertical(&mut s, "A", 300.0);
        let r = run(&mut s, "guide.makePowerGuide", json!({"glyph": "A", "guides": [left, right]}));
        assert_eq!(r["guides"][0]["links"], json!(2), "{r}");
        assert_eq!(r["guides"][1]["links"], json!(2), "{r}");
        let r = run(&mut s, "guide.movePowerGuide", json!({"glyph": "A", "guide": right, "x": 400}));
        assert_eq!(r["distance"].as_f64().map(f64::abs), Some(100.0), "{r}");
        // The linked nodes follow the guide; the middle ones keep their place between the guides.
        assert_eq!(xs(&s, "A"), vec![100.0, 100.0, 250.0, 400.0, 400.0, 250.0]);
        assert_eq!(local_guide(&s, "A", 1).x, 400.0);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(xs(&s, "A"), BAR.iter().map(|p| p.0).collect::<Vec<_>>());
        assert_eq!(local_guide(&s, "A", 1).x, 300.0);
    }

    #[test]
    fn a_global_power_guide_reshapes_every_linked_glyph_but_keeps_locked_ones() {
        let mut s = Session::with_document();
        for glyph in ["A", "B"] {
            add_path(&mut s, glyph, &BAR);
        }
        let g = run(&mut s, "guide.addGlobal", json!({"x": 300, "angle": 90}))["guide"].clone();
        for glyph in ["A", "B"] {
            run(&mut s, "guide.makePowerGuide", json!({"glyph": glyph, "guides": [g]}));
        }
        assert_eq!(guide_data::links(&global_guide(&s, 0)).len(), 4);
        run(&mut s, "glyph.toggleLocked", json!({"glyph": "B", "locked": true}));
        let r = run(&mut s, "guide.movePowerGuide", json!({"glyph": "A", "guide": g, "x": 350}));
        assert_eq!(r["glyphs"], json!(["A"]), "{r}");
        assert_eq!(r["skipped"][0]["glyph"], json!("B"), "{r}");
        // No other power guide: only the linked nodes move.
        assert_eq!(xs(&s, "A"), vec![100.0, 100.0, 200.0, 350.0, 350.0, 200.0]);
        assert_eq!(xs(&s, "B"), BAR.iter().map(|p| p.0).collect::<Vec<_>>());
        // A locked glyph refuses a move of its own local guide.
        let local = vertical(&mut s, "A", 100.0);
        run(&mut s, "glyph.toggleLocked", json!({"glyph": "A", "locked": true}));
        assert!(s.execute("guide.movePowerGuide", &json!({"glyph": "A", "guide": local, "x": 90})).is_err());
    }

    #[test]
    fn unlinking_leaves_ordinary_guides_and_outlines() {
        let mut s = Session::with_document();
        add_path(&mut s, "A", &BAR);
        let left = vertical(&mut s, "A", 100.0);
        let right = vertical(&mut s, "A", 300.0);
        run(&mut s, "guide.makePowerGuide", json!({"glyph": "A", "guides": [left, right]}));
        let r = run(&mut s, "guide.unlinkPowerGuides", json!({"glyph": "A", "nodes": [[0, 0]]}));
        assert_eq!(r["unlinked"], json!(1), "{r}");
        assert_eq!(guide_data::links(&local_guide(&s, "A", 0)), vec![Link::local(0, 1)]);
        let r = run(&mut s, "guide.unlinkPowerGuides", json!({"glyphs": ["A"]}));
        assert_eq!(r["unlinked"], json!(3), "{r}");
        assert!(!guide_data::is_power_guide(&local_guide(&s, "A", 1)));
        assert_eq!(xs(&s, "A"), BAR.iter().map(|p| p.0).collect::<Vec<_>>());
        assert!(s.execute("guide.unlinkPowerGuides", &json!({"glyph": "A"})).is_err(), "nothing left to unlink");
    }

    #[test]
    fn add_power_guides_puts_linked_guides_on_the_stem_edges() {
        let mut s = Session::with_document();
        // A vertical stem 80 wide and 600 tall.
        add_path(&mut s, "I", &[(100.0, 0.0), (100.0, 600.0), (180.0, 600.0), (180.0, 0.0)]);
        let r = run(&mut s, "guide.addPowerGuides", json!({"glyphs": ["I"]}));
        assert_eq!(r["guides"], json!(2), "{r}");
        let font = s.font().unwrap();
        let id = font.default_master().unwrap().id.clone();
        let guides = &font.glyph("I").unwrap().layer(&id).unwrap().info.guides;
        let mut at: Vec<(f64, usize)> = guides.iter().map(|g| (g.x, guide_data::links(g).len())).collect();
        at.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(at, vec![(100.0, 2), (180.0, 2)]);
        assert!(guides.iter().all(|g| g.angle == 90.0));
    }

    #[test]
    fn thickness_markers_and_magnetism_are_kept_in_glyphs_user_data() {
        let mut s = Session::with_document();
        add_path(&mut s, "A", &BAR);
        let g = vertical(&mut s, "A", 150.0);
        run(&mut s, "guide.setThickness", json!({"glyph": "A", "guide": g, "thickness": 20}));
        let r = run(&mut s, "guide.addMarkers", json!({"glyph": "A", "guide": g, "positions": [40], "auto": true}));
        // The vertical guide at x 150 crosses the bar's top and bottom edges.
        assert_eq!(r["markers"], json!([0.0, 40.0, 100.0]), "{r}");
        let r =
            run(&mut s, "guide.setAppearance", json!({"glyph": "A", "guide": g, "magnetism": 8, "color": [1, 0, 0, 1], "showMeasurements": true}));
        assert_eq!(r["color"], json!("1,0,0,1"), "{r}");
        assert!(s.execute("guide.setThickness", &json!({"glyph": "A", "guide": g, "thickness": f64::MAX})).is_err());
        let text = fontcraft_io_glyphs::write(s.font().unwrap()).unwrap();
        assert!(text.contains("com.fontcraft.guideThickness"), "{text}");
        let back = fontcraft_io_glyphs::read(&text).unwrap();
        let id = back.default_master().unwrap().id.clone();
        let guide = &back.glyph("A").unwrap().layer(&id).unwrap().info.guides[0];
        assert_eq!(guide_data::thickness(guide), Some(20.0));
        assert_eq!(guide_data::markers(guide), vec![0.0, 40.0, 100.0]);
        assert_eq!(guide_data::magnetism(guide), Some(8.0));
        assert!(guide.show_measurement);
    }

    #[test]
    fn a_locked_guide_refuses_every_change() {
        let mut s = Session::with_document();
        add_path(&mut s, "A", &BAR);
        let g = vertical(&mut s, "A", 100.0);
        run(&mut s, "guide.lock", json!({"glyph": "A", "guides": [g]}));
        for (id, extra) in [
            ("guide.makePowerGuide", json!({"guides": [g]})),
            ("guide.setThickness", json!({"guide": g, "thickness": 5})),
            ("guide.addMarkers", json!({"guide": g, "positions": [1]})),
            ("guide.setAppearance", json!({"guide": g, "magnetism": 3})),
            ("guide.movePowerGuide", json!({"guide": g, "x": 120})),
        ] {
            let mut params = extra;
            params["glyph"] = json!("A");
            let e = s.execute(id, &params).unwrap_err().to_string();
            assert!(e.contains("locked"), "{id}: {e}");
        }
    }
}
