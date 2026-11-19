//! Find and Replace Outline, on `fontcraft_pathops::matcher` (M13.15). Rows:
//! `path.findOutlinePattern`, `path.replaceOutlinePattern` (`spec/inventory/90-secondary.toml`).
//!
//! The pattern is a list of paths, given as nodes or as the paths of a glyph's layer with the same
//! id as the layer searched, so each master can have its own pattern. It is matched point for
//! point against the unlocked paths of every layer searched, up to translation and, with
//! `allowFlipped`, mirroring and half turns. Closed pattern paths match whole contours; an open
//! pattern path matches a run of consecutive nodes inside any contour, its end nodes by position
//! and on-curve status only. With `endpointsLoose`, the end nodes of a one-path open pattern give
//! only the direction of the segment that leads to them and its least length.
//!
//! Replace puts the replacement, under each match's transform, where the match was. Closed
//! patterns: the matched contours go and the replacement paths take the place of the first of
//! them. One open pattern path: the matched run is replaced by the replacement's nodes inside the
//! same contour; the replacement's first node takes the place of the run's first node and keeps
//! the kind of the segment that arrives there, and an end node of the replacement that lands
//! within the tolerance of the run's end node (always, with `endpointsLoose`) takes its position,
//! so the contour stays joined.

use std::collections::{BTreeMap, HashMap, HashSet};

use fontcraft_geom::{Affine, round_point};
use fontcraft_hinting::Renumbered;
use fontcraft_model::{Font, Glyph, Layer, LayerSide, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, Node, NodeKind, Path, Shape};
use fontcraft_pathops::matcher::{MAX_MATCHES, MAX_PATTERN_NODES, MatchOptions, MatchedRun, match_outline_within};
use fontcraft_pathops::{Contour, Node as PNode, NodeKind as PKind};
use serde_json::{Value, json};

use super::common::{Outcome, grid, in_range};
use super::shared::renumber_hints;
use super::*;
use crate::params::{Params, TargetKind, kind};

const FIND: &str = "path.findOutlinePattern";
const REPLACE: &str = "path.replaceOutlinePattern";

/// The largest `tolerance` accepted, in font units.
pub const MAX_FIND_TOLERANCE: f64 = 10.0;
/// Node comparisons one call may make over every layer it searches; past it the call stops and
/// says so.
pub const MAX_FIND_WORK: usize = 200_000_000;
/// Most paths in a pattern or a replacement.
pub const MAX_PATTERN_PATHS: usize = 256;
/// Most nodes in a replacement (the pattern's own limit).
pub const MAX_REPLACEMENT_NODES: usize = MAX_PATTERN_NODES;
/// Most nodes one Replace call writes into the paths it changes.
pub const MAX_WRITTEN_NODES: usize = 5_000_000;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// Distance slack for floating-point noise.
const EPSILON: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "path.findOutlinePattern",
            "Find Outline…",
            ["Edit"],
            None,
            "{pattern: [{closed?: bool (default true), nodes: [{x, y, type?: \"line\" | \"curve\" | \"qcurve\" | \"offcurve\" (default line), smooth?: bool}]}] (at most 256 paths and 1,024 nodes) | patternGlyph: name (its paths on each layer searched, from its layer with the same id; empty is none), glyph?: name | glyphs?: [names] (default: every glyph but the pattern's and the replacement's), layer?: id | layers?: [ids] (default: every master layer), allowFlipped?: bool (default false: translation only; true also finds mirrored and half-turned copies), endpointsLoose?: bool (default false; for a pattern of one open path of at least three nodes: its end nodes give only the direction and least length of the segments that lead to them), tolerance?: number (font units, 0 to 10, default 0.5), allowRotated?: false, allowScaled?: false} every place in the unlocked paths of those layers where the pattern occurs point for point: closed pattern paths match whole contours, an open pattern path a run of consecutive nodes inside a contour, its end nodes by position and on-curve status only. Matches do not overlap; at most 100,000 are listed. Replacement parameters are ignored, so a Replace request can be tried here first. A layer the pattern glyph lacks is listed in `skipped`. Past the work limit the search stops and says so in `stopped` → {matches: [{glyph, layer, paths: [{shape, start, nodes}], transform: [xx, yx, xy, yy, dx, dy], flipped: bool, error}], count, glyphs: [names], skipped: [{glyph?, layer?, reason}], stopped: string | null, text: one line per match}",
            has_doc,
            find_pattern
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "path.replaceOutlinePattern",
            "Replace Outline…",
            ["Edit"],
            None,
            "{the parameters of path.findOutlinePattern, replacement: [path as in `pattern`] (at most 1,024 nodes) | replacementGlyph: name (its paths on each layer, as `patternGlyph`)} replace every match of path.findOutlinePattern with the replacement moved by the match's transform (and reversed when the transform mirrors). Closed patterns: the matched contours go and the replacement paths take the place of the first of them, with its attributes; an empty replacement removes the matches. A pattern of one open path: the replacement must be one open path whose end nodes are on-curve where the pattern's are; it replaces the matched run inside its contour, its first node where the run's first was, keeping the kind of the segment that arrives there; an end node that lands within the tolerance and a grid step of the run's end node (always, with endpointsLoose) takes its position. Other patterns are refused. New nodes are rounded to the grid; hints on changed paths go. Locked glyphs are skipped and listed. Nothing changes when a result would be an invalid path or too large → {glyphs: [names], replaced: count, skipped: [{glyph?, layer?, reason}], stopped: string | null}",
            has_doc,
            replace_pattern
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn node_kind(p: &Params, key: &str, v: Option<&Value>) -> Result<NodeKind> {
    match v {
        None => Ok(NodeKind::Line),
        Some(Value::String(s)) => match s.as_str() {
            "line" => Ok(NodeKind::Line),
            "curve" => Ok(NodeKind::Curve),
            "qcurve" => Ok(NodeKind::QCurve),
            "offcurve" => Ok(NodeKind::OffCurve),
            other => Err(p.bad(format!("`{key}`: a node's `type` must be line, curve, qcurve or offcurve, got \"{}\"", quoted(other)))),
        },
        Some(other) => Err(p.bad(format!("`{key}`: a node's `type` must be a string, got {}", kind(other)))),
    }
}

fn flag(p: &Params, key: &str, what: &str, v: Option<&Value>) -> Result<bool> {
    match v {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(p.bad(format!("`{key}`: {what} must be true or false, got {}", kind(other)))),
    }
}

/// The paths under `key`, checked: at most [`MAX_PATTERN_PATHS`] paths and `max_nodes` nodes,
/// coordinates within the model's range, every path drawable. `None` when absent.
fn parse_paths(p: &Params, key: &str, max_nodes: usize) -> Result<Option<Vec<Path>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_PATTERN_PATHS {
        return Err(p.bad(format!("`{key}` holds {} paths; at most {MAX_PATTERN_PATHS} are allowed", list.len())));
    }
    let mut total = 0usize;
    let mut out = Vec::with_capacity(list.len());
    for (i, v) in list.iter().enumerate() {
        let Value::Object(o) = v else {
            return Err(p.bad(format!("`{key}` must hold paths {{closed?, nodes}}, found {}", kind(v))));
        };
        let closed = match o.get("closed") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(other) => return Err(p.bad(format!("`{key}` path {i}: `closed` must be true or false, got {}", kind(other)))),
        };
        let Some(Value::Array(nodes)) = o.get("nodes") else {
            return Err(p.bad(format!("`{key}` path {i}: `nodes` must be a list")));
        };
        if nodes.is_empty() {
            return Err(p.bad(format!("`{key}` path {i} has no nodes")));
        }
        total = total.saturating_add(nodes.len());
        if total > max_nodes {
            return Err(p.bad(format!("`{key}` holds more than {max_nodes} nodes")));
        }
        let mut list = Vec::with_capacity(nodes.len());
        for n in nodes {
            let Value::Object(no) = n else {
                return Err(p.bad(format!("`{key}` path {i}: each node must be an object {{x, y, type?}}, found {}", kind(n))));
            };
            let coord = |c: &str| {
                no.get(c)
                    .and_then(Value::as_f64)
                    .filter(|v| in_range(*v))
                    .ok_or_else(|| p.bad(format!("`{key}` path {i}: a node's `{c}` must be a number within ±{MAX_COORDINATE}")))
            };
            let mut node = Node::new(coord("x")?, coord("y")?, node_kind(p, key, no.get("type"))?);
            node.smooth = flag(p, key, "a node's `smooth`", no.get("smooth"))?;
            list.push(node);
        }
        let path = Path::new(closed, list);
        path.to_bez().map_err(|why| p.bad(format!("`{key}` path {i}: {why}")))?;
        out.push(path);
    }
    Ok(Some(out))
}

/// Where a pattern or a replacement comes from.
#[derive(Clone, Debug)]
enum Source {
    Paths(Vec<Path>),
    /// A glyph's paths on the layer with the id of the layer searched.
    Glyph(String),
}

impl Source {
    fn glyph(&self) -> Option<&str> {
        match self {
            Source::Glyph(name) => Some(name),
            Source::Paths(_) => None,
        }
    }

    /// The source's paths for the layer `id`; `Err` with the reason when it has none to give.
    fn paths(&self, font: &Font, id: &str, max_nodes: usize) -> std::result::Result<Vec<Path>, String> {
        match self {
            Source::Paths(list) => Ok(list.clone()),
            Source::Glyph(name) => {
                let layer = font
                    .glyph(name)
                    .and_then(|g| g.layers.iter().find(|l| l.id == id))
                    .ok_or_else(|| format!("glyph `{name}` has no layer `{id}`"))?;
                let list: Vec<Path> = layer
                    .shapes
                    .iter()
                    .filter_map(|s| match s {
                        Shape::Path(p) if !p.nodes.is_empty() => Some(p.clone()),
                        _ => None,
                    })
                    .collect();
                let nodes = list.iter().fold(0usize, |n, p| n.saturating_add(p.nodes.len()));
                if list.len() > MAX_PATTERN_PATHS || nodes > max_nodes {
                    return Err(format!("glyph `{name}` holds more than {MAX_PATTERN_PATHS} paths or {max_nodes} nodes on layer `{id}`"));
                }
                Ok(list)
            }
        }
    }
}

/// `paths_key` or `glyph_key` (an empty glyph name is none).
fn source(p: &Params, font: &Font, paths_key: &str, glyph_key: &str, max_nodes: usize) -> Result<Option<Source>> {
    let paths = parse_paths(p, paths_key, max_nodes)?;
    let glyph = p.opt_str(glyph_key)?.filter(|g| !g.is_empty());
    match (paths, glyph) {
        (Some(_), Some(_)) => Err(p.bad(format!("give `{paths_key}` or `{glyph_key}`, not both"))),
        (Some(list), None) => Ok(Some(Source::Paths(list))),
        (None, Some(name)) if font.glyph(name).is_none() => Err(p.bad(format!("`{glyph_key}`: there is no glyph `{}`", quoted(name)))),
        (None, Some(name)) => Ok(Some(Source::Glyph(name.to_string()))),
        (None, None) => Ok(None),
    }
}

/// A checked request of either command.
struct Request {
    pattern: Source,
    replacement: Option<Source>,
    opts: MatchOptions,
    loose: bool,
    glyphs: Vec<String>,
    layers: Vec<String>,
}

fn request(p: &Params, font: &Font, replacing: bool) -> Result<Request> {
    let pattern = source(p, font, "pattern", "patternGlyph", MAX_PATTERN_NODES)?.ok_or_else(|| p.bad("`pattern` or `patternGlyph` is required"))?;
    let replacement = if replacing {
        let r = source(p, font, "replacement", "replacementGlyph", MAX_REPLACEMENT_NODES)?;
        Some(r.ok_or_else(|| p.bad("`replacement` or `replacementGlyph` is required"))?)
    } else {
        None
    };
    for key in ["allowRotated", "allowScaled"] {
        if p.bool_or(key, false)? {
            return Err(p.bad(format!(
                "`{key}` is not supported: matches are found up to translation, and with `allowFlipped` up to mirroring and half turns"
            )));
        }
    }
    let tolerance = p.opt_f64("tolerance")?.unwrap_or(0.5);
    if !(0.0..=MAX_FIND_TOLERANCE).contains(&tolerance) {
        return Err(p.bad(format!("`tolerance` must be from 0 to {MAX_FIND_TOLERANCE}, got {tolerance}")));
    }
    let opts = MatchOptions { tolerance, flips: p.bool_or("allowFlipped", false)?, ..MatchOptions::default() };
    let loose = p.bool_or("endpointsLoose", false)?;
    let named = match (p.opt_glyphs("glyph", font)?, p.opt_glyphs("glyphs", font)?) {
        (Some(_), Some(_)) => return Err(p.bad("give `glyph` or `glyphs`, not both")),
        (a, b) => a.or(b),
    };
    let own: Vec<&str> = [Some(&pattern), replacement.as_ref()].into_iter().flatten().filter_map(Source::glyph).collect();
    let glyphs: Vec<String> =
        named.unwrap_or_else(|| font.glyphs.iter().map(|g| g.name.clone()).collect()).into_iter().filter(|n| !own.contains(&n.as_str())).collect();
    let layers = layer_ids(p, font)?;
    Ok(Request { pattern, replacement, opts, loose, glyphs, layers })
}

/// `layers`, or `layer`, or every master's id.
fn layer_ids(p: &Params, font: &Font) -> Result<Vec<String>> {
    match (p.opt_str("layer")?, p.opt_strings("layers")?) {
        (Some(_), Some(_)) => Err(p.bad("give `layer` or `layers`, not both")),
        (Some(id), None) => Ok(vec![id.to_string()]),
        (None, Some(ids)) => {
            if ids.len() > crate::params::MAX_TARGETS {
                return Err(p.bad(format!("`layers` holds {} ids; at most {} are allowed", ids.len(), crate::params::MAX_TARGETS)));
            }
            let mut seen = HashSet::new();
            let out: Vec<String> = ids.into_iter().filter(|id| seen.insert(*id)).map(str::to_string).collect();
            if out.is_empty() {
                return Err(p.bad("`layers` is empty"));
            }
            Ok(out)
        }
        (None, None) => Ok(font.masters.iter().map(|m| m.id.clone()).collect()),
    }
}

// ── Patterns ────────────────────────────────────────────────────────────────────────────────────

fn pkind(k: NodeKind) -> PKind {
    match k {
        NodeKind::Line => PKind::Line,
        NodeKind::Curve => PKind::Curve,
        NodeKind::QCurve => PKind::QCurve,
        NodeKind::OffCurve => PKind::OffCurve,
    }
}

fn contour(path: &Path) -> Contour {
    Contour::new(path.closed, path.nodes.iter().map(|n| PNode { point: n.point(), kind: pkind(n.kind), smooth: n.smooth, tag: None }).collect())
}

/// The pattern and replacement of one layer id.
struct Prepared {
    id: String,
    contours: Vec<Contour>,
    /// Closed pattern paths only; otherwise the pattern is one open path.
    closed: bool,
    /// `endpointsLoose` applies: the pattern is one open path of at least three nodes.
    loose: bool,
    replacement: Option<Vec<Path>>,
}

fn prepare(font: &Font, req: &Request, id: &str) -> std::result::Result<Prepared, String> {
    let pattern = req.pattern.paths(font, id, MAX_PATTERN_NODES)?;
    if pattern.is_empty() {
        return Err(format!("the pattern has no paths on layer `{id}`"));
    }
    let closed = pattern.iter().all(|p| p.closed);
    let one_open = matches!(pattern.as_slice(), [p] if !p.closed);
    let loose = req.loose && !closed;
    if loose && !matches!(pattern.as_slice(), [p] if !p.closed && p.nodes.len() >= 3) {
        return Err("`endpointsLoose` needs a pattern of one open path of at least three nodes".into());
    }
    let replacement = match &req.replacement {
        None => None,
        Some(src) => {
            let list = src.paths(font, id, MAX_REPLACEMENT_NODES)?;
            if !closed {
                if !one_open {
                    return Err("Replace takes a pattern of closed paths, or of one open path".into());
                }
                let ends = |p: &Path| p.nodes.first().zip(p.nodes.last()).map(|(a, b)| (a.kind != NodeKind::OffCurve, b.kind != NodeKind::OffCurve));
                let fits = match (pattern.first(), list.as_slice()) {
                    (Some(pat), [rep]) => !rep.closed && ends(pat) == ends(rep),
                    _ => false,
                };
                if !fits {
                    return Err(
                        "the replacement of an open pattern must be one open path whose end nodes are on-curve where the pattern's are".into()
                    );
                }
            }
            Some(list)
        }
    };
    Ok(Prepared { id: id.to_string(), contours: pattern.iter().map(contour).collect(), closed, loose, replacement })
}

/// The pattern of every layer id that has one; the others are listed in `skipped`. None at all
/// is an error naming the first reason.
fn prepare_all(p: &Params, font: &Font, req: &Request, out: &mut Outcome) -> Result<Vec<Prepared>> {
    let mut list = Vec::new();
    let mut first_reason = None;
    for id in &req.layers {
        match prepare(font, req, id) {
            Ok(prep) => list.push(prep),
            Err(why) => {
                out.skipped.push(json!({"layer": id, "reason": why}));
                first_reason.get_or_insert(why);
            }
        }
    }
    match (list.is_empty(), first_reason) {
        (true, Some(why)) => Err(p.bad(why)),
        _ => Ok(list),
    }
}

// ── Matching ────────────────────────────────────────────────────────────────────────────────────

/// The unlocked, non-empty paths of a layer as contours, with their shape indices.
struct Outline {
    slots: Vec<usize>,
    contours: Vec<Contour>,
}

fn outline(layer: &Layer) -> Outline {
    let mut slots = Vec::new();
    let mut contours = Vec::new();
    for (i, shape) in layer.shapes.iter().enumerate() {
        if let Shape::Path(p) = shape
            && !p.nodes.is_empty()
            && !p.info.locked
        {
            slots.push(i);
            contours.push(contour(p));
        }
    }
    Outline { slots, contours }
}

/// One occurrence: one run per pattern contour (an open pattern's run with `endpointsLoose`
/// includes its end nodes), and the transform from the pattern onto the outline.
#[derive(Clone, Debug)]
struct Found {
    runs: Vec<MatchedRun>,
    transform: Affine,
    error: f64,
}

/// The index of the `i`th node of a run starting at `start` in a contour of `len` nodes.
fn node_at(closed: bool, len: usize, start: usize, i: usize) -> Option<usize> {
    let at = start.checked_add(i)?;
    if closed { at.checked_rem(len) } else { (at < len).then_some(at) }
}

/// Every match of the prepared pattern in `outline`, at most `max`.
fn search(prep: &Prepared, outline: &Outline, opts: &MatchOptions, max: usize, work: &mut usize) -> Vec<Found> {
    let opts = MatchOptions { max_matches: max, ..opts.clone() };
    if !prep.loose {
        return match_outline_within(&prep.contours, &outline.contours, &opts, work)
            .into_iter()
            .map(|m| Found { runs: m.runs, transform: m.transform, error: m.error })
            .collect();
    }
    let Some(full) = prep.contours.first() else { return Vec::new() };
    let inner_len = full.nodes.len().saturating_sub(2);
    let inner: Vec<PNode> = full.nodes.iter().skip(1).take(inner_len).copied().collect();
    let inner = [Contour::new(false, inner)];
    let mut taken: HashSet<(usize, usize)> = HashSet::new();
    match_outline_within(&inner, &outline.contours, &opts, work)
        .into_iter()
        .filter_map(|m| {
            let run = *m.runs.first()?;
            loosen(full, run, m.transform, opts.tolerance, outline, &mut taken).map(|ext| Found {
                runs: vec![ext],
                transform: m.transform,
                error: m.error,
            })
        })
        .collect()
}

/// The run of a match of an open pattern's inner nodes (`run`), grown by the contour's nodes on
/// either side when they agree with the pattern's end nodes: the same on-curve status at the
/// start and the same kind at the end, and the segment from the neighbouring node in the
/// pattern's direction (within the tolerance at the pattern's length) and at least as long. A run
/// that would share nodes with an earlier one is refused.
fn loosen(full: &Contour, run: MatchedRun, t: Affine, tol: f64, outline: &Outline, taken: &mut HashSet<(usize, usize)>) -> Option<MatchedRun> {
    let tc = outline.contours.get(run.contour)?;
    let n = tc.nodes.len();
    // The matcher placed the inner nodes reversed when the transform mirrors.
    let expected = if t.determinant() < 0.0 { full.reversed().ok()? } else { full.clone() };
    let len = expected.nodes.len();
    let start = if tc.closed {
        if n < len {
            return None;
        }
        run.start.checked_add(n)?.checked_sub(1)?.checked_rem(n)?
    } else {
        if run.start.checked_add(run.len)? >= n {
            return None;
        }
        run.start.checked_sub(1)?
    };
    let at = |i: usize| node_at(tc.closed, n, start, i);
    let last = len.checked_sub(1)?;
    for (i, e) in expected.nodes.iter().enumerate() {
        let node = tc.nodes.get(at(i)?)?;
        let same = if i == 0 { node.on_curve() == e.on_curve() } else { node.kind == e.kind };
        if !same {
            return None;
        }
    }
    for (end, next) in [(0, 1), (last, last.checked_sub(1)?)] {
        let (e, b) = (expected.nodes.get(end)?.point, expected.nodes.get(next)?.point);
        let (te, tb) = (tc.nodes.get(at(end)?)?.point, tc.nodes.get(at(next)?)?.point);
        let want = t * e - t * b;
        let have = te - tb;
        let (lw, lh) = (want.hypot(), have.hypot());
        if lh + tol + EPSILON < lw {
            return None;
        }
        if lw > EPSILON && lh > EPSILON && (have * (lw / lh) - want).hypot() > tol + EPSILON {
            return None;
        }
    }
    let nodes: Vec<(usize, usize)> = (0..len).filter_map(|i| at(i).map(|a| (run.contour, a))).collect();
    if nodes.iter().any(|k| taken.contains(k)) {
        return None;
    }
    taken.extend(nodes);
    Some(MatchedRun { contour: run.contour, start, len })
}

/// What one layer's search found.
struct Hit<'f> {
    glyph: &'f Glyph,
    layer: &'f Layer,
    prep: &'f Prepared,
    outline: Outline,
    found: Vec<Found>,
}

/// Searches every requested layer of every requested glyph, calling `each` for every layer with
/// matches. Locked glyphs are skipped and listed when `skip_locked`. Returns why the search
/// stopped early, if it did.
fn scan<'f>(
    font: &'f Font,
    req: &Request,
    preps: &'f [Prepared],
    skip_locked: bool,
    out: &mut Outcome,
    mut each: impl FnMut(Hit<'f>) -> Result<()>,
) -> Result<Option<String>> {
    let mut work = MAX_FIND_WORK;
    let mut count = 0usize;
    for name in &req.glyphs {
        let Some(glyph) = font.glyph(name) else { continue };
        if skip_locked && glyph.info.locked {
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        for prep in preps {
            if work == 0 {
                return Ok(Some(format!("the work limit was reached in glyph `{name}`; it and the glyphs after it may hold more matches")));
            }
            if count >= MAX_MATCHES {
                return Ok(Some(format!("{MAX_MATCHES} matches were found; glyph `{name}` and the glyphs after it were not searched")));
            }
            let Some(layer) = glyph.layers.iter().find(|l| l.id == prep.id) else { continue };
            let outline = outline(layer);
            if outline.contours.is_empty() {
                continue;
            }
            let found = search(prep, &outline, &req.opts, MAX_MATCHES.saturating_sub(count), &mut work);
            if found.is_empty() {
                continue;
            }
            count += found.len();
            each(Hit { glyph, layer, prep, outline, found })?;
        }
    }
    Ok((work == 0).then(|| "the work limit was reached; the last glyph searched may hold more matches".to_string()))
}

// ── Find ────────────────────────────────────────────────────────────────────────────────────────

fn find_pattern(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new(FIND, value)?;
    let font = s.font()?;
    let req = request(&p, font, false)?;
    let mut out = Outcome::default();
    let preps = prepare_all(&p, font, &req, &mut out)?;
    let mut matches = Vec::new();
    let mut lines = Vec::new();
    let mut glyphs: Vec<String> = Vec::new();
    let stopped = scan(font, &req, &preps, false, &mut out, |hit| {
        let name = &hit.glyph.name;
        if glyphs.last() != Some(name) {
            glyphs.push(name.clone());
        }
        for f in &hit.found {
            let paths: Vec<Value> = f
                .runs
                .iter()
                .filter_map(|r| hit.outline.slots.get(r.contour).map(|shape| json!({"shape": shape, "start": r.start, "nodes": r.len})))
                .collect();
            let flipped = f.transform.determinant() < 0.0;
            let where_: Vec<String> = f
                .runs
                .iter()
                .filter_map(|r| hit.outline.slots.get(r.contour).map(|shape| format!("path {shape} from node {} ({} nodes)", r.start, r.len)))
                .collect();
            let [.., dx, dy] = f.transform.as_coeffs();
            lines.push(format!(
                "{name} ({}): {}; offset {}, {}{}",
                hit.layer.id,
                where_.join(", "),
                round2(dx),
                round2(dy),
                if flipped { ", flipped" } else { "" }
            ));
            matches.push(json!({
                "glyph": name,
                "layer": hit.layer.id,
                "paths": paths,
                "transform": f.transform.as_coeffs(),
                "flipped": flipped,
                "error": f.error,
            }));
        }
        Ok(())
    })?;
    Ok(json!({
        "matches": matches,
        "count": matches.len(),
        "glyphs": glyphs,
        "skipped": out.skipped,
        "stopped": stopped,
        "text": lines.join("\n"),
    }))
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ── Replace ─────────────────────────────────────────────────────────────────────────────────────

/// `r` moved by `t`, reversed when `t` mirrors, its nodes rounded to `step`; `like` gives its
/// attributes.
fn placed(r: &Path, t: Affine, step: f64, like: Option<&Path>) -> std::result::Result<Path, String> {
    let mut path = r.clone();
    for n in &mut path.nodes {
        let q = round_point(t * n.point(), step);
        if !in_range(q.x) || !in_range(q.y) {
            return Err("the replacement would fall outside the coordinate range".into());
        }
        (n.x, n.y) = (q.x, q.y);
    }
    if t.determinant() < 0.0 {
        path.reverse();
    }
    if let Some(like) = like {
        path.info = like.info.clone();
    }
    Ok(path)
}

/// The nodes that take the place of `run` in `target`: the replacement placed by the match, its
/// first node with the kind and smoothness of the run's first, its end nodes on the run's end
/// nodes when they land close to them (always with `loose`).
fn spliced_nodes(rep: &Path, f: &Found, run: MatchedRun, target: &Path, loose: bool, tol: f64, step: f64) -> std::result::Result<Vec<Node>, String> {
    let mut path = placed(rep, f.transform, step, None)?;
    let n = target.nodes.len();
    let last_at = run.len.checked_sub(1).and_then(|i| node_at(target.closed, n, run.start, i));
    let (Some(first), Some(last)) = (target.nodes.get(run.start), last_at.and_then(|i| target.nodes.get(i))) else {
        return Err("a match lies outside its path".into());
    };
    let near = tol + step.max(0.0) + EPSILON;
    let count = path.nodes.len();
    if let Some(node) = path.nodes.last_mut()
        && count > 1
        && (loose || (node.point() - last.point()).hypot() <= near)
    {
        (node.x, node.y) = (last.x, last.y);
    }
    if let Some(node) = path.nodes.first_mut() {
        if loose || (node.point() - first.point()).hypot() <= near {
            (node.x, node.y) = (first.x, first.y);
        }
        node.kind = first.kind;
        node.smooth = first.smooth;
    }
    Ok(path.nodes)
}

/// `nodes` with each run replaced by its new nodes. Runs do not overlap. A closed contour whose
/// node 0 lies inside a run that wraps around starts at the first node that does not.
fn splice(nodes: &[Node], closed: bool, runs: &[(MatchedRun, Vec<Node>)]) -> Vec<Node> {
    let n = nodes.len();
    let mut covered = vec![false; n];
    let mut start_of: HashMap<usize, usize> = HashMap::new();
    for (k, (run, _)) in runs.iter().enumerate() {
        start_of.insert(run.start, k);
        for i in 0..run.len {
            if let Some(c) = node_at(closed, n, run.start, i).and_then(|a| covered.get_mut(a)) {
                *c = true;
            }
        }
    }
    let free = |i: usize| !covered.get(i).copied().unwrap_or(true) || start_of.contains_key(&i);
    let begin = if closed { (0..n).find(|&i| free(i)).unwrap_or(0) } else { 0 };
    let mut out = Vec::with_capacity(n);
    for step in 0..n {
        let Some(i) = begin.checked_add(step).and_then(|v| v.checked_rem(n)) else { break };
        if let Some(new) = start_of.get(&i).and_then(|&k| runs.get(k)) {
            out.extend(new.1.iter().cloned());
        } else if !covered.get(i).copied().unwrap_or(true)
            && let Some(node) = nodes.get(i)
        {
            out.push(node.clone());
        }
    }
    out
}

/// The layer with every match replaced, and the nodes written. A closed pattern replaces whole
/// paths, so the layer's hints follow their shapes ([`renumber_hints`]): a PostScript end on a
/// replaced path becomes a number at its node's place, a TrueType instruction or corner component
/// on one is removed, and both are reported in `done`, which also lists the layer so the session's
/// hint upkeep leaves it alone. An open pattern edits nodes inside paths that keep their indices;
/// the session's hint upkeep finds the referenced nodes again, as after any node edit.
fn apply(hit: &Hit, tol: f64, step: f64, done: &mut Renumbered) -> std::result::Result<(Layer, usize), String> {
    let layer = hit.layer;
    let replacement = hit.prep.replacement.as_deref().unwrap_or(&[]);
    let mut out = layer.clone();
    let mut written = 0usize;
    // The new index of each old shape, `None` for a replaced one; only closed patterns replace.
    let map: Option<Vec<Option<usize>>> = if hit.prep.closed {
        let mut first_of: BTreeMap<usize, usize> = BTreeMap::new();
        let mut gone: HashSet<usize> = HashSet::new();
        for (k, f) in hit.found.iter().enumerate() {
            let shapes: Vec<usize> = f.runs.iter().filter_map(|r| hit.outline.slots.get(r.contour).copied()).collect();
            if let Some(first) = shapes.iter().min() {
                first_of.insert(*first, k);
            }
            gone.extend(shapes);
        }
        let mut shapes = Vec::with_capacity(layer.shapes.len());
        let mut moved = Vec::with_capacity(layer.shapes.len());
        for (i, shape) in layer.shapes.iter().enumerate() {
            if let Some(f) = first_of.get(&i).and_then(|&k| hit.found.get(k)) {
                let like = match shape {
                    Shape::Path(p) => Some(p),
                    _ => None,
                };
                for r in replacement {
                    let path = placed(r, f.transform, step, like)?;
                    written = written.saturating_add(path.nodes.len());
                    shapes.push(Shape::Path(path));
                }
            }
            if gone.contains(&i) {
                moved.push(None);
            } else {
                moved.push(Some(shapes.len()));
                shapes.push(shape.clone());
            }
        }
        out.shapes = shapes;
        Some(moved)
    } else {
        let Some(rep) = replacement.first() else { return Err("the replacement has no path".into()) };
        let mut by_contour: BTreeMap<usize, Vec<(MatchedRun, Vec<Node>)>> = BTreeMap::new();
        for f in &hit.found {
            let Some(run) = f.runs.first().copied() else { continue };
            let Some(Shape::Path(target)) = hit.outline.slots.get(run.contour).and_then(|&s| layer.shapes.get(s)) else { continue };
            let nodes = spliced_nodes(rep, f, run, target, hit.prep.loose, tol, step)?;
            by_contour.entry(run.contour).or_default().push((run, nodes));
        }
        for (c, runs) in by_contour {
            let Some(&slot) = hit.outline.slots.get(c) else { continue };
            let Some(Shape::Path(path)) = out.shapes.get_mut(slot) else { continue };
            path.nodes = splice(&path.nodes, path.closed, &runs);
            path.to_bez().map_err(|why| format!("path {slot} would become invalid: {why}"))?;
            written = written.saturating_add(path.nodes.len());
        }
        None
    };
    let total = out.shapes.iter().fold(0usize, |n, s| match s {
        Shape::Path(p) => n.saturating_add(p.nodes.len()),
        _ => n,
    });
    if total > MAX_OUTLINE_ELEMENTS {
        return Err(format!("the layer would hold more than {MAX_OUTLINE_ELEMENTS} nodes"));
    }
    if let Some(map) = map
        && !layer.info.hints.is_empty()
    {
        let at = |i: usize| map.get(i).copied().flatten();
        renumber_hints(layer, &mut out, LayerSide::Foreground, &hit.glyph.name, &at, &mut done.report);
        done.insert(&hit.glyph.name, &layer.id, LayerSide::Foreground);
    }
    Ok((out, written))
}

fn replace_pattern(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new(REPLACE, value)?;
    let mut out = Outcome::default();
    let mut changes: Vec<(String, Layer)> = Vec::new();
    let mut glyphs: Vec<String> = Vec::new();
    let mut replaced = 0usize;
    let mut done = Renumbered::default();
    let stopped = {
        let font = s.font()?;
        let req = request(&p, font, true)?;
        let preps = prepare_all(&p, font, &req, &mut out)?;
        let step = grid(font);
        let tol = req.opts.tolerance;
        let mut written = 0usize;
        scan(font, &req, &preps, true, &mut out, |hit| {
            let name = &hit.glyph.name;
            let (layer, nodes) =
                apply(&hit, tol, step, &mut done).map_err(|why| p.bad(format!("glyph `{name}`, layer `{}`: {why}", hit.layer.id)))?;
            written = written.saturating_add(nodes);
            if written > MAX_WRITTEN_NODES {
                return Err(p.bad(format!("the replacements would write more than {MAX_WRITTEN_NODES} nodes; replace in fewer glyphs at a time")));
            }
            replaced += hit.found.len();
            if glyphs.last() != Some(name) {
                glyphs.push(name.clone());
            }
            changes.push((name.clone(), layer));
            Ok(())
        })?
    };
    if !changes.is_empty() {
        s.edit("Replace Outline", |font| {
            for (name, layer) in changes {
                let mut g = font.glyph_mut(&name)?;
                if let Some(slot) = g.layers.iter_mut().find(|l| l.id == layer.id) {
                    *slot = layer;
                }
            }
            Ok(())
        })?;
        s.hints_renumbered(done);
    }
    Ok(json!({"glyphs": glyphs, "replaced": replaced, "skipped": out.skipped, "stopped": stopped}))
}

#[cfg(test)]
#[path = "find_outline_tests.rs"]
mod tests;
