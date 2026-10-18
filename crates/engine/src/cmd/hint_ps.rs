//! PostScript hint commands (`hint.ps.*`): add horizontal and vertical hints, ghosts, edge values, node links, delete, clear, list, the font hints and their check.
//!
//! Rows: `hint.ps.addHorizontalHint`, `hint.ps.addVerticalHint`, `hint.ps.makeGhostHint`,
//! `hint.ps.setGhostDirection`, `hint.ps.setHintValues`, `hint.ps.attachHintToNode`,
//! `hint.ps.deleteHints`, `hint.ps.clearGlyphHints`, `hint.ps.listHints`, `hint.ps.getFontHints`,
//! `hint.ps.validateFontHints`, `hinting.ps.manual.ghostFromSingleNode`,
//! `hinting.ps.manual.overlapAllowed`, `hinting.ps.manual.horizontalHintsNeedExactY`,
//! `hinting.ps.manual.linkToExtremes`.
//!
//! Targets are explicit: `glyph`, `layer` (the main master's layer when absent, since only its
//! hints count for the masters), nodes as `[path, node]` in the model's node order with `path`
//! counting paths only (components skipped), hints by their position in the layer's PostScript
//! hint list, as `hint.ps.listHints` returns it. New hints go to the end of the list. Hints may
//! overlap: the export separates them into hint-replacement groups. Each command that changes
//! the font is one undo step; a refusal changes nothing.
//!
//! Warnings, never refusals: a numeric edge of a horizontal hint where no node lies at exactly
//! that y aligns nothing, and a hint end linked to a node that is not an extreme in the hint's
//! direction hints poorly. `hint.ps.listHints` and the add commands report both.

use std::collections::{BTreeSet, HashMap};

use fontcraft_compile::cff::private_dict_entries;
use fontcraft_hinting::ps::{has_ps_hints, ps_hint_layer};
use fontcraft_hinting::settings::main_master;
use fontcraft_hinting::{
    Edge, HintSettings, MAX_LAYER_HINTS, NodeRef, PsHint, PsHintKind, PsZone, Severity, ps_hints, set_ps_hints, validate_font_hints,
};
use fontcraft_model::{Font, Layer, MAX_COORDINATE, Node, Shape, Target};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, Targets, kind};

const ONE_LAYER: &[TargetKind] = &[TargetKind::Layer];
/// Size of a hint added with nothing selected when the master has no standard stem of its
/// direction.
const DEFAULT_HINT_SIZE: f64 = 50.0;
/// How far a node may lie from a numeric edge and still count as on it: the rule is exact
/// equality, this only absorbs the noise of reading decimal text.
const EDGE_EPSILON: f64 = 1e-6;
/// How far a neighbouring point may lie beyond a linked node and the node still count as an
/// extreme (a handle off by rounding).
const EXTREME_TOLERANCE: f64 = 0.5;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "hint.ps.addHorizontalHint",
            "Add Horizontal Hint",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), nodes?: [[path, node]] | [[[path, node], [path, node]]], origin?: number, size?: number} add a horizontal hint (its edges are y values) at the end of the layer's PostScript hint list. Nodes are [path, node] (model order, paths only). One node: a ghost hint linked to it, a top ghost when the node lies in or nearer to a top alignment zone than to a bottom one, else a bottom ghost. Two nodes: one stem hint linked to both, the first as origin. More nodes: one stem hint per path, each path holding exactly two of them. `nodes` may instead list explicit pairs, one stem hint each. Without nodes: an unlinked stem hint at `origin` with `size`; a missing origin centres it on the layer's bounding box, a missing size is the master's first horizontal stem (50 without one) → {hints: [index], warnings: [text], selection: {glyph, layer}}",
            has_doc,
            add_horizontal
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.ps.addVerticalHint",
            "Add Vertical Hint",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), nodes?: [[path, node]] | [[[path, node], [path, node]]], origin?: number, size?: number} add a vertical stem hint (its edges are x values) at the end of the layer's PostScript hint list. Nodes as for Add Horizontal Hint, but a single node is refused: vertical hints have no ghost form. Without nodes: an unlinked hint at `origin` with `size`; a missing origin centres it on the layer's bounding box, a missing size is the master's first vertical stem (50 without one) → {hints: [index], warnings: [text], selection: {glyph, layer}}",
            has_doc,
            add_vertical
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.ps.makeGhostHint",
            "Make Ghost Hint",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), hints: [index], direction?: \"top\" | \"bottom\"} turn horizontal hints into ghost hints at their origin edge, which keeps its node link. Without `direction`, a stem becomes a top ghost when its origin lies in or nearer to a top alignment zone than to a bottom one, else a bottom ghost, and a ghost keeps its direction. A vertical hint is refused → {hints: [index], selection: {glyph, layer}}",
            has_doc,
            make_ghost
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.ps.setGhostDirection",
            "Set Ghost Hint Direction",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), hints: [index], direction: \"top\" | \"bottom\"} make ghost hints mark a top or a bottom edge. A stem hint is refused → {hints: [index], selection: {glyph, layer}}",
            has_doc,
            set_ghost_direction
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.ps.setHintValues",
            "Set Hint Values",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), hint: index, origin?: number, size?: number} type a hint's values. A typed origin unlinks the origin end; a stem's other end keeps its node link, or, when it is a number, the hint keeps its size. A typed size puts the other end at origin + size, unlinked; a ghost hint given a size becomes a stem hint → {hints: [index], selection: {glyph, layer}}",
            has_doc,
            set_values
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.ps.attachHintToNode",
            "Attach Hint to Node",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), hint: index, end: \"origin\" | \"target\", node: [path, node] | null} link one end of a hint to a node, so the edge follows the node's y (horizontal hints) or x (vertical hints); null unlinks it and keeps its current position. A ghost hint has only an origin → {hints: [index], warnings: [text], selection: {glyph, layer}}",
            has_doc,
            attach
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.ps.deleteHints",
            "Delete Hints",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), hints: [index]} remove the listed PostScript hints from the layer; the ones after them move up in the list → {removed: count, selection: {glyph, layer}}",
            has_doc,
            delete
        )
        .targets(ONE_LAYER)
        .when("editView"),
        cmd!(
            "hint.ps.clearGlyphHints",
            "Remove PostScript Hints",
            [],
            None,
            "{glyphs?: [names] (default: every glyph; locked glyphs are then skipped), layers?: [layer id] | \"all\" (default all)} remove every PostScript hint from those layers of the glyphs, so the export autohints them again once no master layer of a glyph has any; a named glyph that is locked is refused → {cleared: [{glyph, layer}], skipped: [glyph]}",
            has_doc,
            clear
        ),
        cmd!(
            query "hint.ps.listHints",
            "List PostScript Hints",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer)} the layer's PostScript hints in list order. `origin` is the origin edge's position; `size` the signed distance to the other edge (null for a ghost); `nodes` the linked node of each end; `hintSource` says whether the export takes this glyph's hints from this layer, `sourceLayer` names the layer it takes them from (null: none, the glyph is autohinted) → {glyph, layer, hintSource, sourceLayer, hints: [{index, direction: \"horizontal\" | \"vertical\", kind: \"stem\" | \"ghostTop\" | \"ghostBottom\", origin, size, nodes: {origin: [path, node] | null, target: [path, node] | null}, warnings: [text]}], warnings: [text]}",
            has_doc,
            list
        )
        .targets(ONE_LAYER),
        cmd!(
            query "hint.ps.getFontHints",
            "Get PostScript Font Hints",
            [],
            None,
            "{master?: id, instance?: index | name (a static instance)} the values a CFF export of the master or instance writes to the Private dict (default: the first master); lists are empty and stems null where the dict leaves them out, and blueScale, blueShift and blueFuzz are the values that apply → {master, instance, blueValues, otherBlues, familyBlues, familyOtherBlues, stdHW, stdVW, stemSnapH, stemSnapV, blueScale, blueShift, blueFuzz, warnings}",
            has_doc,
            font_hints
        ),
        cmd!(
            query "hint.ps.validateFontHints",
            "Check PostScript Font Hints",
            [],
            None,
            "{masters?: [id] (default: every master)} check each master's alignment zones and standard stems against the PostScript rules (zone counts, overlap, the gap of 2 × blueFuzz + 1, a zone at the baseline, the 25-unit and blueScale size limits, at most 12 stems per direction) → {findings: [{master, severity: \"error\" | \"warning\", rule, message}], errors: count, warnings: count}",
            has_doc,
            validate
        ),
    ]
}

// ── shared ──────────────────────────────────────────────────────────────────────────────────────

/// The addressed layer's PostScript hints, read from the font.
struct Read {
    target: Targets,
    list: Vec<PsHint>,
}

fn read(p: &Params, font: &Font) -> Result<Read> {
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => main_master(font),
    };
    let target = Targets::new(p.command(), p.str("glyph")?, Some(layer));
    let r = target.resolve(font)?;
    let list = ps_hints(r.layer).map_err(|e| p.bad(e.to_string()))?.into_iter().map(|(_, h)| h).collect();
    Ok(Read { target, list })
}

/// Writes `list` back to the addressed layer as one undo step and returns the command's result.
fn write(s: &mut Session, p: &Params, label: &str, r: &Read, list: &[PsHint], mut out: Value) -> Result<Value> {
    s.edit(label, |font| r.target.edit_layer(font, |l| set_ps_hints(l, list).map_err(|e| p.bad(e.to_string()))))?;
    if let Some(m) = out.as_object_mut() {
        m.insert("selection".into(), r.target.to_json());
    }
    Ok(out)
}

/// A `[path, node]` value as a node reference, unchecked.
fn node_ref(v: &Value) -> Option<NodeRef> {
    let [path, node] = v.as_array()?.as_slice() else { return None };
    Some(NodeRef::new(usize::try_from(path.as_u64()?).ok()?, usize::try_from(node.as_u64()?).ok()?))
}

/// `r` when it names a node of `layer`.
fn checked(p: &Params, key: &str, r: NodeRef, layer: &Layer) -> Result<NodeRef> {
    r.check(layer).map_err(|_| p.bad(format!("`{key}`: [{}, {}] does not resolve on the layer", r.path, r.node)))?;
    Ok(r)
}

/// One hint index under `key`.
fn hint_index(p: &Params, key: &str, list: &[PsHint]) -> Result<usize> {
    let v = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`{key}` must be a hint index, found {}", kind(v))))?;
    if i >= list.len() {
        return Err(p.bad(format!("`{key}`: {i} does not resolve: the layer has {} PostScript hints", list.len())));
    }
    Ok(i)
}

/// The `hints` list: positions in `list`, repeats dropped, in the order given.
fn hint_indexes(p: &Params, list: &[PsHint]) -> Result<Vec<usize>> {
    let values = p.array("hints")?;
    if values.is_empty() {
        return Err(p.bad("`hints` lists no hint"));
    }
    if values.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`hints` lists more than {MAX_LAYER_HINTS} entries")));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for v in values {
        let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`hints` must hold indexes, found {}", kind(v))))?;
        if i >= list.len() {
            return Err(p.bad(format!("`hints`: {i} does not resolve: the layer has {} PostScript hints", list.len())));
        }
        if seen.insert(i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// A typed coordinate or size: finite and within the model's coordinate range.
fn coordinate(p: &Params, key: &str) -> Result<Option<f64>> {
    match p.opt_f64(key)? {
        Some(v) if v.abs() > MAX_COORDINATE => Err(p.bad(format!("`{key}` is {v}; it must lie within ±{MAX_COORDINATE}"))),
        other => Ok(other),
    }
}

/// `direction`: `Some(true)` for "top", `Some(false)` for "bottom".
fn ghost_direction(p: &Params) -> Result<Option<bool>> {
    match p.opt_str("direction")? {
        None => Ok(None),
        Some("top") => Ok(Some(true)),
        Some("bottom") => Ok(Some(false)),
        Some(other) => Err(p.bad(format!("`direction` must be \"top\" or \"bottom\", got \"{other}\""))),
    }
}

fn kind_name(k: PsHintKind) -> &'static str {
    match k {
        PsHintKind::Stem => "stem",
        PsHintKind::GhostTop => "ghostTop",
        PsHintKind::GhostBottom => "ghostBottom",
    }
}

fn paths(layer: &Layer) -> impl Iterator<Item = &fontcraft_model::Path> {
    layer.shapes.iter().filter_map(|s| match s {
        Shape::Path(p) => Some(p),
        _ => None,
    })
}

/// The box (x0, y0, x1, y1) around the on-curve nodes of the layer's paths.
fn node_bounds(layer: &Layer) -> Option<(f64, f64, f64, f64)> {
    let mut out: Option<(f64, f64, f64, f64)> = None;
    for n in paths(layer).flat_map(|p| p.nodes.iter()).filter(|n| n.on_curve()) {
        out = Some(match out {
            Some((x0, y0, x1, y1)) => (x0.min(n.x), y0.min(n.y), x1.max(n.x), y1.max(n.y)),
            None => (n.x, n.y, n.x, n.y),
        });
    }
    out
}

/// The master's alignment zones for the layer, none when its settings cannot be read.
fn zones(font: &Font, layer: &Layer) -> Vec<PsZone> {
    HintSettings::for_master(font, &layer.master).map(|s| s.zones).unwrap_or_default()
}

/// Whether a ghost at `y` marks a top edge: `y` lies in or nearer to a top zone than to a bottom
/// zone. A zone at 0 (the baseline zone) or with a negative size is a bottom zone, as the Private
/// dict sorts them. Without zones, a ghost above the middle of the layer's outline is a top ghost.
fn ghost_is_top(zones: &[PsZone], layer: &Layer, y: f64) -> bool {
    if zones.is_empty() {
        return node_bounds(layer).is_some_and(|(_, y0, _, y1)| y > (y0 + y1) / 2.0);
    }
    let distance = |z: &PsZone| {
        let (a, b) = (z.position, z.position + z.size);
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        if y < lo {
            lo - y
        } else if y > hi {
            y - hi
        } else {
            0.0
        }
    };
    let is_top = |z: &PsZone| z.position != 0.0 && z.size >= 0.0;
    let nearest = |top: bool| zones.iter().filter(|z| is_top(z) == top).map(distance).fold(f64::INFINITY, f64::min);
    nearest(true) < nearest(false)
}

/// Whether the node `r` names is an extreme along the hint's axis (y for a horizontal hint, x for
/// a vertical one): an on-curve node with its neighbouring points all on one side of it.
fn is_extreme(layer: &Layer, r: NodeRef, horizontal: bool) -> bool {
    let Some(path) = r.path(layer) else { return false };
    let Some(node) = path.nodes.get(r.node) else { return false };
    if !node.on_curve() {
        return false;
    }
    let along = |n: &Node| if horizontal { n.y } else { n.x };
    let count = path.nodes.len();
    let prev = match r.node.checked_sub(1) {
        Some(i) => path.nodes.get(i),
        None if path.closed => count.checked_sub(1).and_then(|i| path.nodes.get(i)),
        None => None,
    };
    let next = match path.nodes.get(r.node.saturating_add(1)) {
        Some(n) => Some(n),
        None if path.closed => path.nodes.first(),
        None => None,
    };
    let offsets: Vec<f64> = [prev, next].into_iter().flatten().map(|n| along(n) - along(node)).collect();
    offsets.iter().all(|d| *d <= EXTREME_TOLERANCE) || offsets.iter().all(|d| *d >= -EXTREME_TOLERANCE)
}

/// What is doubtful about hint `i` on `layer`: a linked node that is gone or is not an extreme,
/// a numeric edge of a horizontal hint that no node lies on.
fn hint_warnings(layer: &Layer, i: usize, h: &PsHint) -> Vec<String> {
    let mut out = Vec::new();
    let axis = if h.horizontal { "y" } else { "x" };
    for (end, edge) in [("origin", Some(h.origin)), ("target", h.other)] {
        match edge {
            Some(Edge::Node(r)) => {
                if r.resolve(layer).is_none() {
                    out.push(format!("hint {i}: its {end} is linked to node [{}, {}], which the layer does not have", r.path, r.node));
                } else if !is_extreme(layer, r, h.horizontal) {
                    out.push(format!(
                        "hint {i}: its {end} is linked to node [{}, {}], which is not an extreme in {axis}; hints work best on extreme nodes",
                        r.path, r.node
                    ));
                }
            }
            Some(Edge::Value(v)) if h.horizontal => {
                let on_it = paths(layer).flat_map(|p| p.nodes.iter()).any(|n| (n.y - v).abs() <= EDGE_EPSILON);
                if !on_it {
                    out.push(format!(
                        "hint {i}: no node lies exactly at y = {v}, so its {end} aligns nothing; a horizontal hint acts only on nodes at its edges"
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

// ── adding ──────────────────────────────────────────────────────────────────────────────────────

/// The nodes a new hint is made from.
enum Selection {
    Nothing,
    One(NodeRef),
    Pairs(Vec<(NodeRef, NodeRef)>),
}

/// The `nodes` parameter: a flat list of nodes, or a list of explicit pairs.
fn selection(p: &Params, layer: &Layer) -> Result<Selection> {
    let Some(values) = p.opt_array("nodes")? else { return Ok(Selection::Nothing) };
    if values.is_empty() {
        return Ok(Selection::Nothing);
    }
    if values.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`nodes` lists more than {MAX_LAYER_HINTS} entries")));
    }
    let shape = |v: &Value| p.bad(format!("`nodes`: each entry is [path, node], or a pair [[path, node], [path, node]], found {v}"));
    let pairs_given = values.first().and_then(Value::as_array).and_then(|a| a.first()).is_some_and(Value::is_array);
    if pairs_given {
        let mut pairs = Vec::with_capacity(values.len());
        for v in values {
            let (a, b) = v
                .as_array()
                .and_then(|a| match a.as_slice() {
                    [x, y] => Some((node_ref(x)?, node_ref(y)?)),
                    _ => None,
                })
                .ok_or_else(|| shape(v))?;
            pairs.push((checked(p, "nodes", a, layer)?, checked(p, "nodes", b, layer)?));
        }
        return distinct(p, pairs);
    }
    let refs = values.iter().map(|v| node_ref(v).ok_or_else(|| shape(v)).and_then(|r| checked(p, "nodes", r, layer))).collect::<Result<Vec<_>>>()?;
    match refs.as_slice() {
        [one] => Ok(Selection::One(*one)),
        [a, b] => distinct(p, vec![(*a, *b)]),
        _ => {
            // Grouped by path, in the order the paths first appear.
            let mut groups: Vec<(usize, Vec<NodeRef>)> = Vec::new();
            let mut at: HashMap<usize, usize> = HashMap::new();
            for r in refs {
                match at.get(&r.path).and_then(|&g| groups.get_mut(g)) {
                    Some((_, g)) => g.push(r),
                    None => {
                        at.insert(r.path, groups.len());
                        groups.push((r.path, vec![r]));
                    }
                }
            }
            let mut pairs = Vec::with_capacity(groups.len());
            for (path, g) in groups {
                let [a, b] = g.as_slice() else {
                    return Err(p.bad(format!(
                        "`nodes`: path {path} holds {} of the nodes; with more than two nodes, each pair must lie on a path of its own",
                        g.len()
                    )));
                };
                pairs.push((*a, *b));
            }
            distinct(p, pairs)
        }
    }
}

fn distinct(p: &Params, pairs: Vec<(NodeRef, NodeRef)>) -> Result<Selection> {
    if let Some((a, _)) = pairs.iter().find(|(a, b)| a == b) {
        return Err(p.bad(format!("`nodes`: [{}, {}] is given twice; a stem hint needs two different nodes", a.path, a.node)));
    }
    Ok(Selection::Pairs(pairs))
}

fn add_horizontal(s: &mut Session, v: &Value) -> Result<Value> {
    add(s, v, "hint.ps.addHorizontalHint", "Add Horizontal Hint", true)
}

fn add_vertical(s: &mut Session, v: &Value) -> Result<Value> {
    add(s, v, "hint.ps.addVerticalHint", "Add Vertical Hint", false)
}

fn add(s: &mut Session, v: &Value, id: &'static str, label: &str, horizontal: bool) -> Result<Value> {
    let p = Params::new(id, v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let sel = selection(&p, layer)?;
    let origin = coordinate(&p, "origin")?;
    let size = coordinate(&p, "size")?;
    if !matches!(sel, Selection::Nothing) && (origin.is_some() || size.is_some()) {
        return Err(p.bad("`origin` and `size` are for a hint without nodes: a linked hint takes its edges from its nodes"));
    }
    let new: Vec<PsHint> = match sel {
        Selection::One(node) => {
            if !horizontal {
                return Err(p.bad("a vertical hint needs two nodes: only horizontal hints have a ghost form, made from one node"));
            }
            let y = Edge::Node(node).position(layer, true).map_err(|e| p.bad(e.to_string()))?;
            vec![PsHint::ghost(ghost_is_top(&zones(font, layer), layer, y), Edge::Node(node))]
        }
        Selection::Pairs(pairs) => pairs.into_iter().map(|(a, b)| PsHint::stem(horizontal, Edge::Node(a), Edge::Node(b))).collect(),
        Selection::Nothing => {
            let size = size.unwrap_or_else(|| {
                let settings = HintSettings::for_master(font, &layer.master).ok();
                let first = settings.and_then(|s| if horizontal { s.stems_h.first().copied() } else { s.stems_v.first().copied() });
                first.filter(|w| w.is_finite() && *w != 0.0).unwrap_or(DEFAULT_HINT_SIZE)
            });
            let origin = origin.unwrap_or_else(|| {
                let centre = match node_bounds(layer) {
                    Some((x0, y0, x1, y1)) => (if horizontal { y0 + y1 } else { x0 + x1 }) / 2.0,
                    // An empty layer: the hint starts at the baseline or is centred on the width.
                    None if horizontal => size / 2.0,
                    None => layer.width / 2.0,
                };
                centre - size / 2.0
            });
            vec![PsHint::stem(horizontal, Edge::Value(origin), Edge::Value(origin + size))]
        }
    };
    let mut list = std::mem::take(&mut r.list);
    let first = list.len();
    list.extend(new);
    let added: Vec<usize> = (first..list.len()).collect();
    let warnings: Vec<String> = added.iter().filter_map(|&i| list.get(i).map(|h| hint_warnings(layer, i, h))).flatten().collect();
    write(s, &p, label, &r, &list, json!({"hints": added, "warnings": warnings}))
}

// ── changing ────────────────────────────────────────────────────────────────────────────────────

fn make_ghost(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.makeGhostHint", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let ix = hint_indexes(&p, &r.list)?;
    let direction = ghost_direction(&p)?;
    let zones = zones(font, layer);
    let mut list = std::mem::take(&mut r.list);
    for &i in &ix {
        let Some(h) = list.get_mut(i) else { continue };
        if !h.horizontal {
            return Err(p.bad(format!("hint {i} is vertical: only horizontal hints have a ghost form")));
        }
        let top = match (direction, h.kind) {
            (Some(top), _) => top,
            (None, PsHintKind::GhostTop) => true,
            (None, PsHintKind::GhostBottom) => false,
            (None, PsHintKind::Stem) => {
                let y = h.origin.position(layer, true).map_err(|e| p.bad(format!("hint {i}: {e}")))?;
                ghost_is_top(&zones, layer, y)
            }
        };
        h.kind = if top { PsHintKind::GhostTop } else { PsHintKind::GhostBottom };
        h.other = None;
    }
    write(s, &p, "Make Ghost Hint", &r, &list, json!({"hints": ix}))
}

fn set_ghost_direction(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.setGhostDirection", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let ix = hint_indexes(&p, &r.list)?;
    let top = ghost_direction(&p)?.ok_or_else(|| p.bad("`direction` is required: \"top\" or \"bottom\""))?;
    let mut list = std::mem::take(&mut r.list);
    for &i in &ix {
        let Some(h) = list.get_mut(i) else { continue };
        if h.kind == PsHintKind::Stem {
            return Err(p.bad(format!("hint {i} is a stem hint: only ghost hints have a direction (Make Ghost Hint turns a hint into one)")));
        }
        h.kind = if top { PsHintKind::GhostTop } else { PsHintKind::GhostBottom };
    }
    write(s, &p, "Set Ghost Hint Direction", &r, &list, json!({"hints": ix}))
}

fn set_values(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.setHintValues", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let i = hint_index(&p, "hint", &r.list)?;
    let origin = coordinate(&p, "origin")?;
    let size = coordinate(&p, "size")?;
    if origin.is_none() && size.is_none() {
        return Err(p.bad("give `origin`, `size` or both"));
    }
    let mut list = std::mem::take(&mut r.list);
    let h = list.get_mut(i).ok_or_else(|| p.bad(format!("`hint`: {i} does not resolve")))?;
    let was = h.origin.position(layer, h.horizontal).map_err(|e| p.bad(format!("hint {i}: {e}")))?;
    let now = origin.unwrap_or(was);
    if origin.is_some() {
        h.origin = Edge::Value(now);
    }
    match size {
        Some(size) => {
            h.kind = PsHintKind::Stem;
            h.other = Some(Edge::Value(now + size));
        }
        None => {
            if let Some(Edge::Value(other)) = h.other {
                h.other = Some(Edge::Value(now + (other - was)));
            }
        }
    }
    write(s, &p, "Set Hint Values", &r, &list, json!({"hints": [i]}))
}

fn attach(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.attachHintToNode", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let i = hint_index(&p, "hint", &r.list)?;
    let target_end = match p.str("end")? {
        "origin" => false,
        "target" => true,
        other => return Err(p.bad(format!("`end` must be \"origin\" or \"target\", got \"{other}\""))),
    };
    // `node: null` unlinks, so the key itself must be there.
    if v.get("node").is_none() {
        return Err(p.bad("`node` is required: a node [path, node], or null to unlink the end"));
    }
    let node = match p.get("node") {
        Some(n) => Some(checked(&p, "node", node_ref(n).ok_or_else(|| p.bad(format!("`node` must be [path, node] or null, found {n}")))?, layer)?),
        None => None,
    };
    let mut list = std::mem::take(&mut r.list);
    let h = list.get_mut(i).ok_or_else(|| p.bad(format!("`hint`: {i} does not resolve")))?;
    if target_end && h.kind != PsHintKind::Stem {
        return Err(p.bad(format!("hint {i} is a ghost hint: it has only an origin")));
    }
    let horizontal = h.horizontal;
    let end = if target_end { h.other.as_mut() } else { Some(&mut h.origin) };
    let end = end.ok_or_else(|| p.bad(format!("hint {i} has no target")))?;
    *end = match node {
        Some(n) => Edge::Node(n),
        None => Edge::Value(end.position(layer, horizontal).map_err(|e| p.bad(format!("hint {i}: {e}")))?),
    };
    let warnings = list.get(i).map(|h| hint_warnings(layer, i, h)).unwrap_or_default();
    write(s, &p, "Attach Hint to Node", &r, &list, json!({"hints": [i], "warnings": warnings}))
}

fn delete(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.deleteHints", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let ix: BTreeSet<usize> = hint_indexes(&p, &r.list)?.into_iter().collect();
    let list: Vec<PsHint> = std::mem::take(&mut r.list).into_iter().enumerate().filter(|(i, _)| !ix.contains(i)).map(|(_, h)| h).collect();
    write(s, &p, "Delete Hints", &r, &list, json!({"removed": ix.len()}))
}

fn clear(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.clearGlyphHints", v)?;
    let font = s.font()?;
    let named = p.opt_glyphs("glyphs", font)?;
    let explicit = named.is_some();
    let glyphs = named.unwrap_or_else(|| font.glyphs.iter().map(|g| g.name.clone()).collect());
    let layers: Option<Vec<&str>> = match p.get("layers") {
        None => None,
        Some(Value::String(all)) if all == "all" => None,
        Some(Value::Array(_)) => Some(p.strings("layers")?),
        Some(other) => return Err(p.bad(format!("`layers` must be a list of layer ids or \"all\", got {}", kind(other)))),
    };
    if let Some(ids) = &layers
        && let Some(missing) = ids.iter().find(|id| !glyphs.iter().filter_map(|g| font.glyph(g)).any(|g| g.layers.iter().any(|l| l.id == **id)))
    {
        return Err(p.bad(format!("`layers`: none of the glyphs has a layer `{missing}`")));
    }
    let mut todo: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for name in &glyphs {
        let glyph = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        let chosen: Vec<&Layer> =
            glyph.layers.iter().filter(|l| layers.as_ref().is_none_or(|ids| ids.contains(&l.id.as_str())) && has_ps_hints(l)).collect();
        if chosen.is_empty() {
            continue;
        }
        if let Err(e) = crate::check_unlocked(glyph) {
            if explicit {
                return Err(e);
            }
            skipped.push(name.clone());
            continue;
        }
        todo.extend(chosen.into_iter().map(|l| (name.clone(), l.id.clone())));
    }
    if !todo.is_empty() {
        s.edit("Remove PostScript Hints", |font| {
            for (g, id) in &todo {
                let mut glyph = font.glyph_mut(g)?;
                let layer = glyph
                    .layers
                    .iter_mut()
                    .find(|l| l.id == *id)
                    .ok_or_else(|| crate::EngineError::Internal(format!("glyph `{g}` has no layer `{id}`")))?;
                set_ps_hints(layer, &[]).map_err(|e| crate::EngineError::Internal(format!("glyph `{g}`: {e}")))?;
            }
            Ok(())
        })?;
    }
    let cleared: Vec<Value> = todo.iter().map(|(g, l)| json!({"glyph": g, "layer": l})).collect();
    Ok(json!({"cleared": cleared, "skipped": skipped}))
}

// ── queries ─────────────────────────────────────────────────────────────────────────────────────

fn list(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.listHints", v)?;
    let font = s.font()?;
    let r = read(&p, font)?;
    let res = r.target.resolve(font)?;
    let layer = res.layer;
    let mut warnings = Vec::new();
    let source = match HintSettings::for_master(font, &layer.master) {
        Ok(settings) => ps_hint_layer(font, res.glyph, &settings, layer).map(|l| l.id.clone()),
        Err(e) => {
            warnings.push(format!("the hint settings cannot be read ({e}), so the layer the export takes hints from is unknown"));
            None
        }
    };
    let link = |e: Option<Edge>| match e {
        Some(Edge::Node(r)) => json!([r.path, r.node]),
        _ => Value::Null,
    };
    let mut hints = Vec::with_capacity(r.list.len());
    for (i, h) in r.list.iter().enumerate() {
        let origin = h.origin.position(layer, h.horizontal).ok();
        let other = h.other.and_then(|e| e.position(layer, h.horizontal).ok());
        let size = match (h.kind, origin, other) {
            (PsHintKind::Stem, Some(o), Some(t)) => json!(t - o),
            _ => Value::Null,
        };
        let own = hint_warnings(layer, i, h);
        warnings.extend(own.iter().cloned());
        hints.push(json!({
            "index": i,
            "direction": if h.horizontal { "horizontal" } else { "vertical" },
            "kind": kind_name(h.kind),
            "origin": origin,
            "size": size,
            "nodes": {"origin": link(Some(h.origin)), "target": link(h.other)},
            "warnings": own,
        }));
    }
    Ok(json!({
        "glyph": r.target.glyph,
        "layer": layer.id,
        "hintSource": source.as_deref() == Some(layer.id.as_str()),
        "sourceLayer": source,
        "hints": hints,
        "warnings": warnings,
    }))
}

/// A number as JSON: a whole number as an integer.
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9.0e15 { json!(v as i64) } else { json!(v) }
}

fn font_hints(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.getFontHints", v)?;
    let font = s.font()?;
    let target = match (p.opt_str("master")?, p.get("instance")) {
        (Some(_), Some(_)) => return Err(p.bad("give `master` or `instance`, not both")),
        (Some(id), None) => Target::Master(font.master(id).map_err(|_| p.bad(format!("`master`: there is no master `{id}`")))?.id.clone()),
        (None, Some(which)) => {
            let index = match which {
                Value::String(name) => {
                    font.instances.iter().position(|i| i.name == *name).ok_or_else(|| p.bad(format!("`instance`: there is no instance `{name}`")))?
                }
                other => other
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i < font.instances.len())
                    .ok_or_else(|| p.bad(format!("`instance` must be an instance index or name, found {other}")))?,
            };
            if font.instances.get(index).is_some_and(|i| i.kind.is_some()) {
                return Err(p.bad(format!("instance {index} is a variable font setting, not a static instance")));
            }
            Target::Instance(index)
        }
        (None, None) => font.default_target()?,
    };
    let mut warnings = Vec::new();
    let entries = private_dict_entries(font, &target, &mut warnings).map_err(|e| p.bad(e.to_string()))?;
    let mut out = serde_json::Map::new();
    match &target {
        Target::Master(id) => {
            out.insert("master".into(), json!(id));
            out.insert("instance".into(), Value::Null);
        }
        Target::Instance(i) => {
            out.insert("master".into(), Value::Null);
            out.insert("instance".into(), json!(i));
        }
    }
    for (name, values) in entries {
        let (key, scalar) = match name {
            "BlueValues" => ("blueValues", false),
            "OtherBlues" => ("otherBlues", false),
            "FamilyBlues" => ("familyBlues", false),
            "FamilyOtherBlues" => ("familyOtherBlues", false),
            "StdHW" => ("stdHW", true),
            "StdVW" => ("stdVW", true),
            "StemSnapH" => ("stemSnapH", false),
            "StemSnapV" => ("stemSnapV", false),
            "BlueScale" => ("blueScale", true),
            "BlueShift" => ("blueShift", true),
            "BlueFuzz" => ("blueFuzz", true),
            _ => continue,
        };
        let value = if scalar { values.first().copied().map_or(Value::Null, number) } else { Value::Array(values.into_iter().map(number).collect()) };
        out.insert(key.into(), value);
    }
    out.insert("warnings".into(), json!(warnings));
    Ok(Value::Object(out))
}

fn validate(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.ps.validateFontHints", v)?;
    let font = s.font()?;
    let masters: Vec<String> = match p.opt_strings("masters")? {
        Some(ids) => {
            let mut out: Vec<String> = Vec::with_capacity(ids.len());
            for id in ids {
                let m = font.master(id).map_err(|_| p.bad(format!("`masters`: there is no master `{id}`")))?;
                if !out.contains(&m.id) {
                    out.push(m.id.clone());
                }
            }
            out
        }
        None => font.masters.iter().map(|m| m.id.clone()).collect(),
    };
    let mut findings = Vec::new();
    let (mut errors, mut warnings) = (0usize, 0usize);
    for m in &masters {
        let list = validate_font_hints(font, m).map_err(|e| p.bad(format!("master `{m}`: {e}")))?;
        for f in list {
            let severity = match f.severity {
                Severity::Error => {
                    errors += 1;
                    "error"
                }
                Severity::Warning => {
                    warnings += 1;
                    "warning"
                }
            };
            findings.push(json!({"master": m, "severity": severity, "rule": f.rule, "message": f.message}));
        }
    }
    Ok(json!({"findings": findings, "errors": errors, "warnings": warnings}))
}
