//! Path operations (M3.6): Remove Overlap and the boolean operations, Correct Path Direction,
//! Add and Force Extremes, Tidy up Paths, Open Corner and Reconnect Nodes, Cut with Line (the
//! Knife tool's command), conversion between cubic and quadratic curves, and Round Coordinates
//! (`docs/spec/03a-edit-view-drawing.md` §§ 8 and 12–15, `docs/spec/11-filters.md`,
//! `docs/spec/05-font-view.md` › Batch-Processing).
//!
//! Whole-layer commands take one `glyph` (with `layer`, `side` and target lists) or a `glyphs`
//! list with `layer` or `layers` (Font View). With `glyphs`, and in the all-masters forms, a
//! locked glyph and a layer the operation cannot finish are skipped and listed in `skipped`; with
//! one `glyph` they are errors.
//!
//! The geometry comes from `fontcraft-pathops`. Each layer's operation runs through
//! [`po::run_with_time_limit`]: the boolean engine can loop forever on rare inputs
//! (docs/plans/M3.md › M3.6, rule 8), so on native targets the operation runs on a worker thread
//! and a layer that takes longer than [`po::TIME_LIMIT`] is left as it was. Nodes carry a tag through
//! each operation, so a node that survives keeps its name, identifier, lock and user data, and a
//! result path keeps the attributes of the path it comes from.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use fontcraft_geom::{BezPath, CubicBez, Point, Rect, Shape as _, Vec2, cubics_to_quadratic_splines, round_point, round_to_grid};
use fontcraft_model::{Font, Layer, LayerPartsMut, LayerSide, MAX_COORDINATE, Master, Node, NodeKind, Path, Shape, Transform};
use fontcraft_pathops::{self as po, BoolOp, OpenPaths, OverlapOptions, PathOpError, TIME_LIMIT, run_with_time_limit};
use serde_json::{Value, json};

use super::common::{Outcome, each_layer, grid, in_range, scope};
use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind as K, Targets, kind};

/// Open Corner's overlap, as a share of the em, for a master without stems of either direction.
const DEFAULT_OVERLAP_SHARE: f64 = 0.04;
/// Largest Open Corner overlap a caller may give, in units.
const MAX_OVERLAP: f64 = 10_000.0;
/// Range of the cubic-to-quadratic tolerance, in units.
const MIN_TOLERANCE: f64 = 0.001;
const MAX_TOLERANCE: f64 = 1_000.0;
/// The keys that address things inside one layer; they go with one `glyph`.
const LIST_KEYS: [&str; 7] = ["points", "segments", "segment", "shapes", "anchors", "guides", "annotations"];

type Op<T> = std::result::Result<T, String>;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.removeOverlap",
            "Remove Overlap",
            ["Path"],
            Some("Cmd+Shift+O"),
            "{glyph: name, layer?: id (default the first master's), side?: \"foreground\" | \"background\", shapes?: [index] (default every path)} or {glyphs: [names], layer?: id | layers?: [ids]}, keepOpenPaths?: bool = false. Merges the overlapping closed paths into outlines without overlaps (nonzero rule; new points on the grid) and deletes the open paths and one-node paths among them unless keepOpenPaths. Components stay. With `glyphs`, a locked glyph or a layer that cannot be done (one taking longer than 5 s included) is skipped and left as it was → {glyphs: [names], skipped: [{glyph, layer?, reason}], selection?: {glyph, layer, shapes} (one glyph: the result paths)}",
            has_doc,
            remove_overlap
        )
        .targets(&[K::Glyphs, K::Layer, K::Shapes]),
        cmd!(
            "path.removeOverlapForAllMasters",
            "Remove Overlap for All Masters",
            ["Path"],
            None,
            "{glyphs: [names] | glyph: name, keepOpenPaths?: bool = false} path.removeOverlap on every master layer of the glyphs. A locked glyph is skipped (an error with one `glyph`); a layer that cannot be done is skipped and left as it was → {glyphs: [names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            remove_overlap_for_all_masters
        )
        .targets(&[K::Glyphs]),
        cmd!(
            "transform.booleanUnion",
            "Union",
            [],
            None,
            "{glyph: name, layer?: id, side?, shapes?: [index] (default every path)} or {glyphs: [names], layer?: id | layers?: [ids]}. Merges the closed paths into their combined outline (nonzero rule); open paths stay as they are. With `glyphs`, a locked glyph or a layer that cannot be done is skipped → {glyphs, skipped, selection?: {glyph, layer, shapes}}",
            has_doc,
            boolean_union
        )
        .targets(&[K::Glyphs, K::Layer, K::Shapes]),
        cmd!(
            "transform.booleanSubtract",
            "Subtract",
            [],
            None,
            "{glyph: name, layer?: id, side?, shapes?: [index]} or {glyphs: [names], layer?: id | layers?: [ids]}. Cuts the closed paths in `shapes` out of the other closed paths of the layer; without `shapes`, the frontmost closed path (the last one) is cut out of the rest. Each group is merged first. Needs closed paths on both sides; with `glyphs` a layer without them is skipped → {glyphs, skipped, selection?: {glyph, layer, shapes}}",
            has_doc,
            boolean_subtract
        )
        .targets(&[K::Glyphs, K::Layer, K::Shapes]),
        cmd!(
            "transform.booleanIntersect",
            "Intersect",
            [],
            None,
            "{glyph: name, layer?: id, side?, shapes?: [index]} or {glyphs: [names], layer?: id | layers?: [ids]}. Keeps the area where the closed paths in `shapes` and the other closed paths overlap; without `shapes`, the frontmost closed path (the last one) plays the selected part. Needs closed paths on both sides; with `glyphs` a layer without them is skipped → {glyphs, skipped, selection?: {glyph, layer, shapes}}",
            has_doc,
            boolean_intersect
        )
        .targets(&[K::Glyphs, K::Layer, K::Shapes]),
        cmd!(
            "path.correctPathDirection",
            "Correct Path Direction",
            ["Path"],
            Some("Cmd+Shift+R"),
            "{glyph: name, layer?: id, side?} or {glyphs: [names], layer?: id | layers?: [ids]}. Outer closed paths run counter-clockwise and counters clockwise, by nesting; each closed path starts at its lowest, then leftmost, on-curve node; paths and components are sorted by the bottom-left corner of their bounds (left edge first). Open paths keep their direction → {glyphs, skipped, selection?: {glyph, layer}}",
            has_doc,
            correct_path_direction
        )
        .targets(&[K::Glyphs, K::Layer]),
        cmd!(
            "path.correctPathDirectionForAllMasters",
            "Correct Path Direction for All Masters",
            ["Path"],
            Some("Cmd+Alt+Shift+R"),
            "{glyphs: [names] | glyph: name} path.correctPathDirection on every master layer and every intermediate and alternate layer of the glyphs; other layers are left alone. A locked glyph is skipped (an error with one `glyph`) → {glyphs, skipped: [{glyph, layer?, reason}]}",
            has_doc,
            correct_path_direction_for_all_masters
        )
        .targets(&[K::Glyphs]),
        cmd!(
            "path.addExtremes",
            "Add Extremes",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], shapes?: [index]} or {glyphs: [names], layer?: id | layers?: [ids]}. Adds a node at each horizontal and vertical extreme of the segments whose two end nodes are in `points`, of the paths in `shapes`, or of every path; skips an extreme closer than 3 units to a segment end or to the extreme before it; a node within 2 units and 2° of an extreme gets its handles made exactly horizontal or vertical. New nodes are smooth and on the grid → {glyphs, skipped, selection?: {glyph, layer, shapes}}",
            has_doc,
            add_extremes
        )
        .targets(&[K::Glyphs, K::Layer, K::Points, K::Shapes]),
        cmd!(
            "path.forceExtremes",
            "Force Extremes",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], shapes?: [index]} or {glyphs: [names], layer?: id | layers?: [ids]}. path.addExtremes without the short-segment exception: every extreme that rounds to a new point gets a node → {glyphs, skipped, selection?: {glyph, layer, shapes}}",
            has_doc,
            force_extremes
        )
        .targets(&[K::Glyphs, K::Layer, K::Points, K::Shapes]),
        cmd!(
            "path.addInflections",
            "Add Inflections",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], shapes?: [index]} or {glyphs: [names], layer?: id | layers?: [ids]}. Adds a smooth node at each inflection of the cubic segments whose two end nodes are in `points`, of the paths in `shapes`, or of every path, splitting each segment there without changing the outline (new points on the grid). An inflection within 1 unit of a segment end gets no node → {glyphs, skipped, selection?: {glyph, layer, shapes}}",
            has_doc,
            add_inflections
        )
        .targets(&[K::Glyphs, K::Layer, K::Points, K::Shapes]),
        cmd!(
            "path.tidyUpPaths",
            "Tidy up Paths",
            ["Path"],
            Some("Cmd+Shift+T"),
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], shapes?: [index], straighten?: units (0 to 100, default 0)} or {glyphs: [names], layer?: id | layers?: [ids], straighten?}. On the nodes in `points`, the paths in `shapes`, or every path: straight curves become lines (with `straighten`, also curves that stray less than that many units from the line between their ends), a node on the one before it is merged into it, a node on the line between its neighbours is removed, and each node becomes smooth when a curve meets it and its directions are within 1°, else a corner. Removing points can break compatibility with other masters → {glyphs, skipped, selection?: {glyph, layer, shapes}}",
            has_doc,
            tidy_up_paths
        )
        .targets(&[K::Glyphs, K::Layer, K::Points, K::Shapes]),
        cmd!(
            "path.tidyUpPathsForAllMasters",
            "Tidy up Paths for All Masters",
            ["Path"],
            Some("Cmd+Alt+Shift+T"),
            "{glyphs: [names] | glyph: name} path.tidyUpPaths on every path of every master layer of the glyphs. A locked glyph is skipped (an error with one `glyph`) → {glyphs, skipped: [{glyph, layer?, reason}]}",
            has_doc,
            tidy_up_paths_for_all_masters
        )
        .targets(&[K::Glyphs]),
        cmd!(
            "path.openCorner",
            "Open Corner",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]] (corner nodes), segments?: [[shape, node]] (both end nodes), overlap?: number | [x, y] (default half the master's first vertical and first horizontal stem; 4% of the em without stems)} turns each corner node into two nodes whose segments go on past the corner and cross in a small triangle. Smooth nodes, open path ends and nearly straight corners are left alone → {selection: {glyph, layer, points}} (the new nodes)",
            has_doc,
            open_corner
        )
        .targets(&[K::Layer, K::Points, K::Segments])
        .context(&["canvas.node", "canvas.segment"]),
        cmd!(
            "path.reconnectNodes",
            "Reconnect Nodes",
            [],
            None,
            "{glyph: name, layer?: id, side?, points: [[shape, node]] (an even number of on-curve nodes)} pairs the nodes, nearest first. The two nodes of an open corner become one corner node again; any other pair is rewired: the path into each node continues along the path out of the other, which joins two paths or splits one → {selection: {glyph, layer, points}}",
            has_doc,
            reconnect_nodes
        )
        .targets(&[K::Layer, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.cutWithLine",
            "Cut with Line",
            [],
            None,
            "{glyph: name, layer?: id, side?, from: [x, y], to: [x, y], shapes?: [index] (default every path)} cuts the paths the line crosses: closed paths are closed again along the line (pieces of overlapping paths join one another; a stretch where the line ends inside a shape is not cut); open paths are split. New points on the grid → {selection: {glyph, layer, points}} (the nodes made on the line)",
            has_doc,
            cut_with_line
        )
        .targets(&[K::Layer, K::Shapes]),
        cmd!(
            "path.convertToCubic",
            "Convert to Cubic",
            ["Path", "Other"],
            None,
            "{glyph: name, layer?: id, side?} or {glyphs: [names], layer?: id | layers?: [ids]} or {glyphs | glyph, allMasters: true}. Rewrites every quadratic segment as cubic ones, exactly: a spline with several off-curve points gets a smooth node at each implied point; coordinates are not rounded → {glyphs, skipped, segments: count}",
            has_doc,
            convert_to_cubic
        )
        .targets(&[K::Glyphs, K::Layer]),
        cmd!(
            "path.convertToQuadratic",
            "Convert to Quadratic",
            ["Path", "Other"],
            None,
            "{glyph: name, layer?: id, side?} or {glyphs: [names], layer?: id | layers?: [ids]} or {glyphs | glyph, allMasters: true}, tolerance?: units (0.001 to 1000; default 0.001 em, the compiler's). Rewrites every cubic segment as a quadratic spline within the tolerance; new off-curve points on the grid. With allMasters, master layers with the same node structure are converted together and stay compatible → {glyphs, skipped, segments: count}",
            has_doc,
            convert_to_quadratic
        )
        .targets(&[K::Glyphs, K::Layer]),
        cmd!(
            "path.roundCoordinates",
            "Round Coordinates",
            ["Path"],
            None,
            "{glyph: name, layer?: id, side?} or {glyphs: [names], layer?: id | layers?: [ids]} or {glyphs | glyph, allMasters: true}. Moves every node, handle, anchor and component offset to the nearest point of the font's grid (whole units when the font has no grid) → {glyphs, skipped}",
            has_doc,
            round_coordinates
        )
        .targets(&[K::Glyphs, K::Layer]),
        cmd!(
            "path.roundCoordinatesForAllMasters",
            "Round Coordinates for All Masters",
            ["Path"],
            None,
            "{glyphs: [names] | glyph: name} path.roundCoordinates on every master layer of the glyphs. A locked glyph is skipped (an error with one `glyph`) → {glyphs, skipped}",
            has_doc,
            round_coordinates_for_all_masters
        )
        .targets(&[K::Glyphs]),
    ]
}

// ── Contours in, paths out ──────────────────────────────────────────────────────────────────────

/// What a command selects after it ran on one layer: nodes, or whole paths.
#[derive(Default)]
struct Picked {
    points: Vec<(usize, usize)>,
    shapes: Vec<usize>,
}

/// The paths of one layer side that take part in an operation, as contours whose nodes are tagged
/// with their place.
struct Input {
    /// The shape index of each contour.
    slots: Vec<usize>,
    /// The paths as they were.
    paths: Vec<Path>,
    contours: Vec<po::Contour>,
    /// For each tag, the contour and node it was given to.
    origin: Vec<(usize, usize)>,
}

impl Input {
    fn new(shapes: &[Shape], take: impl Fn(usize) -> bool) -> Self {
        let mut input = Self { slots: Vec::new(), paths: Vec::new(), contours: Vec::new(), origin: Vec::new() };
        for (i, shape) in shapes.iter().enumerate() {
            let Shape::Path(path) = shape else { continue };
            if !take(i) {
                continue;
            }
            let c = input.contours.len();
            let mut nodes = Vec::with_capacity(path.nodes.len());
            for (k, n) in path.nodes.iter().enumerate() {
                nodes.push(po::Node { point: n.point(), kind: po_kind(n.kind), smooth: n.smooth, tag: Some(input.origin.len()) });
                input.origin.push((c, k));
            }
            input.slots.push(i);
            input.paths.push(path.clone());
            input.contours.push(po::Contour::new(path.closed, nodes));
        }
        input
    }
}

/// What an operation gives back.
struct Done {
    /// Each result contour with the input contour it comes from; `None`: the one its first tagged
    /// node comes from.
    contours: Vec<(Option<usize>, po::Contour)>,
    /// Nodes to select, as (result contour, node).
    selection: Vec<(usize, usize)>,
}

impl Done {
    fn merged(contours: Vec<po::Contour>) -> Self {
        Self { contours: contours.into_iter().map(|c| (None, c)).collect(), selection: Vec::new() }
    }

    fn in_place(contours: Vec<po::Contour>) -> Self {
        Self { contours: contours.into_iter().enumerate().map(|(i, c)| (Some(i), c)).collect(), selection: Vec::new() }
    }

    fn rewired(r: po::Rewired) -> Self {
        Self { contours: r.sources.into_iter().map(Some).zip(r.contours).collect(), selection: r.selection }
    }
}

fn po_kind(k: NodeKind) -> po::NodeKind {
    match k {
        NodeKind::Line => po::NodeKind::Line,
        NodeKind::Curve => po::NodeKind::Curve,
        NodeKind::QCurve => po::NodeKind::QCurve,
        NodeKind::OffCurve => po::NodeKind::OffCurve,
    }
}

fn model_kind(k: po::NodeKind) -> NodeKind {
    match k {
        po::NodeKind::Line => NodeKind::Line,
        po::NodeKind::Curve => NodeKind::Curve,
        po::NodeKind::QCurve => NodeKind::QCurve,
        po::NodeKind::OffCurve => NodeKind::OffCurve,
    }
}

/// The result contours as model paths, each with the input contour it belongs to. A node that
/// survived keeps the data of the node it was; a path keeps the data of the path it comes from.
/// An identifier goes to the first copy only.
fn to_paths(input: &Input, done: &Done) -> Op<Vec<(usize, Path)>> {
    let mut used_tags = HashSet::new();
    let mut used_paths = HashSet::new();
    let mut out = Vec::with_capacity(done.contours.len());
    for (source, contour) in &done.contours {
        let tagged = contour.nodes.iter().find_map(|n| n.tag).and_then(|t| input.origin.get(t)).map(|&(c, _)| c);
        let source = source.filter(|s| *s < input.paths.len()).or(tagged).unwrap_or(0);
        let mut nodes = Vec::with_capacity(contour.nodes.len());
        for n in &contour.nodes {
            if !(in_range(n.point.x) && in_range(n.point.y)) {
                return Err(format!("the result has a coordinate beyond ±{MAX_COORDINATE}"));
            }
            // `+ 0.0` turns a negative zero into zero.
            let mut m = Node::new(n.point.x + 0.0, n.point.y + 0.0, model_kind(n.kind));
            m.smooth = n.smooth && n.kind != po::NodeKind::OffCurve;
            let old = n.tag.and_then(|t| {
                let &(c, k) = input.origin.get(t)?;
                Some((t, input.paths.get(c)?.nodes.get(k)?))
            });
            if let Some((t, old)) = old {
                m.tangent = old.tangent && m.smooth;
                m.locked = old.locked;
                m.orientation = old.orientation;
                m.info = old.info.clone();
                if !used_tags.insert(t) && m.info.identifier.is_some() {
                    m.info.identifier = None;
                }
            }
            nodes.push(m);
        }
        let mut path = Path::new(contour.closed, nodes);
        if let Some(old) = input.paths.get(source) {
            path.info = old.info.clone();
            path.extra = old.extra.clone();
            if !used_paths.insert(source) && path.info.identifier.is_some() {
                path.info.identifier = None;
            }
        }
        path.to_bez()?;
        out.push((source, path));
    }
    Ok(out)
}

/// Puts `paths` in place of the input's paths: each where the path it comes from was, in order;
/// shapes that took no part keep their order around them. Returns each path's new index.
fn place(shapes: &mut Vec<Shape>, input: &Input, paths: Vec<(usize, Path)>) -> Vec<usize> {
    let count = paths.len();
    let mut buckets: Vec<Vec<(usize, Path)>> = (0..input.slots.len()).map(|_| Vec::new()).collect();
    let mut orphans = Vec::new();
    for (r, (source, path)) in paths.into_iter().enumerate() {
        match buckets.get_mut(source) {
            Some(b) => b.push((r, path)),
            None => orphans.push((r, path)),
        }
    }
    let slot_of: HashMap<usize, usize> = input.slots.iter().enumerate().map(|(c, &s)| (s, c)).collect();
    let mut index = vec![0; count];
    let mut out = Vec::with_capacity(shapes.len().saturating_add(count));
    let mut put = |out: &mut Vec<Shape>, r: usize, path: Path| {
        if let Some(i) = index.get_mut(r) {
            *i = out.len();
        }
        out.push(Shape::Path(path));
    };
    for (j, shape) in std::mem::take(shapes).into_iter().enumerate() {
        let bucket = match slot_of.get(&j) {
            Some(&c) => buckets.get_mut(c),
            None => None,
        };
        match bucket {
            Some(bucket) => {
                for (r, path) in bucket.drain(..) {
                    put(&mut out, r, path);
                }
            }
            None => out.push(shape),
        }
    }
    for (r, path) in orphans {
        put(&mut out, r, path);
    }
    *shapes = out;
    index
}

/// Runs `job` on the input's contours within `limit` and puts the result in place of the input's
/// paths in `shapes`. `Ok(None)` when no path takes part; nothing changes when the job fails.
fn apply(
    shapes: &mut Vec<Shape>,
    mut input: Input,
    limit: Duration,
    job: impl FnOnce(Vec<po::Contour>) -> std::result::Result<Done, PathOpError> + Send + 'static,
) -> Op<Option<Picked>> {
    if input.contours.is_empty() {
        return Ok(None);
    }
    let contours = std::mem::take(&mut input.contours);
    let done = run_with_time_limit(limit, move || job(contours)).map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let paths = to_paths(&input, &done)?;
    let index = place(shapes, &input, paths);
    let points = done.selection.iter().filter_map(|&(r, k)| index.get(r).map(|&s| (s, k))).collect();
    let mut made = index;
    made.sort_unstable();
    Ok(Some(Picked { points, shapes: made }))
}

// ── Targets ─────────────────────────────────────────────────────────────────────────────────────

/// The paths among `t.shapes`; `None` without a `shapes` list (every path takes part).
fn chosen_paths(t: Option<&Targets>, shapes: &[Shape]) -> Op<Option<BTreeSet<usize>>> {
    let Some(t) = t.filter(|t| !t.shapes.is_empty()) else { return Ok(None) };
    let set: BTreeSet<usize> = t.shapes.iter().copied().filter(|&i| matches!(shapes.get(i), Some(Shape::Path(_)))).collect();
    if set.is_empty() {
        return Err("none of `shapes` is a path".into());
    }
    Ok(Some(set))
}

/// The nodes a command acts on, path by path (`None`: the whole path); `None` without `points`
/// or `shapes` (every path, whole).
fn chosen_nodes(t: Option<&Targets>, shapes: &[Shape]) -> Op<Option<BTreeMap<usize, Option<Vec<usize>>>>> {
    let Some(t) = t.filter(|t| !(t.points.is_empty() && t.shapes.is_empty())) else { return Ok(None) };
    let is_path = |s: usize| matches!(shapes.get(s), Some(Shape::Path(_)));
    let mut map: BTreeMap<usize, Option<Vec<usize>>> = BTreeMap::new();
    for &(s, k) in t.points.iter().filter(|(s, _)| is_path(*s)) {
        if let Some(list) = map.entry(s).or_insert_with(|| Some(Vec::new())) {
            list.push(k);
        }
    }
    for &s in t.shapes.iter().filter(|s| is_path(**s)) {
        map.insert(s, None);
    }
    if map.is_empty() {
        return Err("none of the targets is on a path".into());
    }
    Ok(Some(map))
}

/// Refuses the target lists a command does not take.
fn takes_only(p: &Params, t: &Targets, keys: &[&str]) -> Result<()> {
    let given = [
        ("points", !t.points.is_empty()),
        ("segments", !t.segments.is_empty()),
        ("shapes", !t.shapes.is_empty()),
        ("anchors", !t.anchors.is_empty()),
        ("guides", !t.guides.is_empty()),
        ("annotations", !t.annotations.is_empty()),
    ];
    match given.iter().find(|(key, on)| *on && !keys.contains(key)) {
        Some((key, _)) => Err(p.bad(format!("does not take `{key}`"))),
        None => Ok(()),
    }
}

/// A command for one glyph only.
fn one_glyph(p: &Params) -> Result<Targets> {
    if p.get("glyphs").is_some() {
        return Err(p.bad("acts on one glyph: give `glyph`, not `glyphs`"));
    }
    Targets::parse(p)
}

/// `{glyph, layer, side, points | shapes}` for a command's `selection` result.
fn selection_json(p: &Params, t: &Targets, layer: &str, picked: Option<Picked>) -> Value {
    let mut sel = Targets::new(p.command(), t.glyph.clone(), Some(layer.to_string()));
    sel.side = t.side;
    if let Some(picked) = picked {
        if picked.points.is_empty() {
            sel.shapes = picked.shapes;
        } else {
            sel.points = picked.points;
        }
    }
    sel.to_json()
}

/// One layer of one glyph with target lists, or the same layers of several glyphs.
enum Form {
    One(Targets),
    Many(super::common::Scope),
}

fn form(p: &Params, font: &Font) -> Result<Form> {
    if p.get("glyph").is_some() && p.get("glyphs").is_some() {
        return Err(p.bad("give `glyph` or `glyphs`, not both"));
    }
    if p.get("glyph").is_some() && p.get("layers").is_none() {
        return Ok(Form::One(Targets::parse(p)?));
    }
    if let Some(key) = LIST_KEYS.iter().chain(&["side"]).find(|k| p.get(k).is_some()) {
        return Err(p.bad(format!("`{key}` goes with one `glyph` and one `layer`")));
    }
    Ok(Form::Many(scope(p, font)?))
}

/// Runs `op` on the addressed layers and returns `{glyphs, skipped, selection?}`. `op` gets the
/// layer, the side, and the target object when one glyph is addressed with `glyph`; it returns
/// what to select (`None`: nothing to do), or why it leaves the layer alone. `lists` are the
/// target lists the command takes.
fn run_layers(
    s: &mut Session,
    p: &Params,
    label: &str,
    lists: &[&str],
    op: impl Fn(&mut Layer, LayerSide, Option<&Targets>) -> Op<Option<Picked>>,
) -> Result<Value> {
    match form(p, s.font()?)? {
        Form::One(t) => {
            takes_only(p, &t, lists)?;
            let layer = t.layer_id(s.font()?)?;
            let picked = s.edit(label, |font| {
                t.edit_layer(font, |l| {
                    // Editing a background the layer does not have would add an empty one.
                    if t.side == LayerSide::Background && l.info.background.is_none() {
                        return Ok(None);
                    }
                    op(l, t.side, Some(&t)).map_err(|m| p.bad(m))
                })
            })?;
            let selection = selection_json(p, &t, &layer, picked);
            Ok(json!({"glyphs": [t.glyph], "skipped": [], "selection": selection}))
        }
        Form::Many(scope) => {
            let single = scope.single;
            let out = s.edit(label, |font| {
                each_layer(font, &scope, p.command(), |_, layer| match op(layer, LayerSide::Foreground, None) {
                    Ok(_) => Ok(None),
                    Err(why) if single => Err(p.bad(why)),
                    Err(why) => Ok(Some(why)),
                })
            })?;
            Ok(out.to_json())
        }
    }
}

/// Master layers.
fn master_layer(l: &Layer) -> bool {
    l.is_master_layer()
}

/// Master layers, and the intermediate and alternate layers that interpolate with them.
fn master_or_special_layer(l: &Layer) -> bool {
    l.is_master_layer() || !l.info.attrs.coordinates.is_empty() || !l.info.attrs.axis_rules.is_empty()
}

/// The glyphs of an all-masters form, and whether one `glyph` was given.
fn all_masters_glyphs(p: &Params, font: &Font) -> Result<(Vec<String>, bool)> {
    if let Some(key) = ["layer", "layers", "side"].iter().chain(LIST_KEYS.iter()).find(|k| p.get(k).is_some()) {
        return Err(p.bad(format!("takes no `{key}`: it acts on every master layer of the glyphs")));
    }
    let scope = scope(p, font)?;
    Ok((scope.glyphs, scope.single))
}

/// Runs `f` once per glyph on the glyph's layers that `which` accepts. A locked glyph is an error
/// when `single`, else skipped. `f` returns whether it changed the glyph and may add skips. Call
/// inside [`Session::edit`].
fn each_glyph(
    font: &mut Font,
    glyphs: &[String],
    single: bool,
    which: fn(&Layer) -> bool,
    mut f: impl FnMut(&str, Vec<&mut Layer>, &mut Outcome) -> Result<bool>,
) -> Result<Outcome> {
    let mut out = Outcome::default();
    for name in glyphs {
        if font.glyph(name).is_some_and(|g| g.info.locked) {
            if single {
                return Err(EngineError::Locked(name.clone()));
            }
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        let mut g = font.glyph_mut(name)?;
        let layers: Vec<&mut Layer> = g.layers.iter_mut().filter(|l| which(l)).collect();
        if f(name, layers, &mut out)? {
            out.glyphs.push(name.clone());
        }
    }
    Ok(out)
}

/// Runs `op` on the layers `which` accepts of every glyph; a layer it cannot do is skipped.
fn run_all_masters(
    s: &mut Session,
    p: &Params,
    label: &str,
    which: fn(&Layer) -> bool,
    op: impl Fn(&mut Layer, LayerSide, Option<&Targets>) -> Op<Option<Picked>>,
) -> Result<Value> {
    let (glyphs, single) = all_masters_glyphs(p, s.font()?)?;
    let out = s.edit(label, |font| {
        each_glyph(font, &glyphs, single, which, |name, layers, out| {
            let mut changed = false;
            for layer in layers {
                match op(layer, LayerSide::Foreground, None) {
                    Ok(_) => changed = true,
                    Err(why) => out.skip(name, Some(layer.id.as_str()), why),
                }
            }
            Ok(changed)
        })
    })?;
    Ok(out.to_json())
}

// ── Remove Overlap and the booleans ─────────────────────────────────────────────────────────────

fn overlap_options(p: &Params, font: &Font) -> Result<OverlapOptions> {
    let keep = p.bool_or("keepOpenPaths", false)?;
    Ok(OverlapOptions { grid: grid(font), open_paths: if keep { OpenPaths::Keep } else { OpenPaths::Remove } })
}

fn remove_overlap_in(layer: &mut Layer, side: LayerSide, t: Option<&Targets>, options: OverlapOptions) -> Op<Option<Picked>> {
    let shapes = layer.parts_mut(side).shapes;
    let chosen = chosen_paths(t, shapes)?;
    let input = Input::new(shapes, |i| chosen.as_ref().is_none_or(|c| c.contains(&i)));
    apply(shapes, input, TIME_LIMIT, move |c| Ok(Done::merged(po::remove_overlap(&c, options)?)))
}

fn remove_overlap(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.removeOverlap", v)?;
    let options = overlap_options(&p, s.font()?)?;
    run_layers(s, &p, "Remove Overlap", &["shapes"], |layer, side, t| remove_overlap_in(layer, side, t, options))
}

fn remove_overlap_for_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.removeOverlapForAllMasters", v)?;
    let options = overlap_options(&p, s.font()?)?;
    run_all_masters(s, &p, "Remove Overlap for All Masters", master_layer, |layer, side, t| remove_overlap_in(layer, side, t, options))
}

/// Takes part in a boolean operation: a closed path of more than one node.
fn fills(c: &po::Contour) -> bool {
    c.closed && c.nodes.len() > 1
}

fn boolean_in(layer: &mut Layer, side: LayerSide, t: Option<&Targets>, op: BoolOp, grid: f64) -> Op<Option<Picked>> {
    let shapes = layer.parts_mut(side).shapes;
    let chosen = chosen_paths(t, shapes)?;
    if op == BoolOp::Union {
        let input = Input::new(shapes, |i| chosen.as_ref().is_none_or(|c| c.contains(&i)));
        return apply(shapes, input, TIME_LIMIT, move |c| Ok(Done::merged(po::boolean(&c, &[], BoolOp::Union, grid)?)));
    }
    let input = Input::new(shapes, |_| true);
    // The selected paths; without a selection, the frontmost closed path.
    let selected: Vec<bool> = match &chosen {
        Some(set) => input.slots.iter().map(|s| set.contains(s)).collect(),
        None => {
            let front = input.contours.iter().rposition(fills);
            (0..input.contours.len()).map(|i| Some(i) == front).collect()
        }
    };
    let on = |want: bool| input.contours.iter().zip(&selected).any(|(c, &s)| s == want && fills(c));
    if !(on(true) && on(false)) {
        return Err("needs closed paths on both sides: among the selected paths and among the others (without a selection, at least two)".into());
    }
    apply(shapes, input, TIME_LIMIT, move |contours| {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for (c, s) in contours.into_iter().zip(selected) {
            if s { a.push(c) } else { b.push(c) }
        }
        let out = match op {
            BoolOp::Difference => po::boolean(&b, &a, BoolOp::Difference, grid)?,
            _ => po::boolean(&a, &b, op, grid)?,
        };
        Ok(Done::merged(out))
    })
}

fn boolean(s: &mut Session, v: &Value, id: &'static str, label: &str, op: BoolOp) -> Result<Value> {
    let p = Params::new(id, v)?;
    let step = grid(s.font()?);
    run_layers(s, &p, label, &["shapes"], |layer, side, t| boolean_in(layer, side, t, op, step))
}

fn boolean_union(s: &mut Session, v: &Value) -> Result<Value> {
    boolean(s, v, "transform.booleanUnion", "Union", BoolOp::Union)
}

fn boolean_subtract(s: &mut Session, v: &Value) -> Result<Value> {
    boolean(s, v, "transform.booleanSubtract", "Subtract", BoolOp::Difference)
}

fn boolean_intersect(s: &mut Session, v: &Value) -> Result<Value> {
    boolean(s, v, "transform.booleanIntersect", "Intersect", BoolOp::Intersection)
}

// ── Correct Path Direction ──────────────────────────────────────────────────────────────────────

/// Component outlines by (base glyph, master), for every component in the layers of `glyphs`.
type Outlines = HashMap<(String, String), BezPath>;

fn component_outlines(font: &Font, glyphs: &[String]) -> Outlines {
    let mut out = Outlines::new();
    for name in glyphs {
        let Some(g) = font.glyph(name) else { continue };
        for layer in &g.layers {
            let background = layer.info.background.as_ref().map_or(&[][..], |b| b.shapes.as_slice());
            for shape in layer.shapes.iter().chain(background) {
                let Shape::Component(c) = shape else { continue };
                let key = (c.glyph.clone(), layer.master.clone());
                if !out.contains_key(&key)
                    && let Ok(outline) = font.outline(&c.glyph, &layer.master)
                {
                    out.insert(key, outline);
                }
            }
        }
    }
    out
}

/// The glyphs a command's `glyph` or `glyphs` names, as far as they can be read (the command
/// itself reports bad parameters).
fn named_glyphs(p: &Params, font: &Font) -> Vec<String> {
    match p.opt_str("glyph") {
        Ok(Some(name)) => vec![name.to_string()],
        _ => p.opt_glyphs("glyphs", font).ok().flatten().unwrap_or_default(),
    }
}

fn bounds_of(path: &BezPath) -> Option<Rect> {
    (!path.elements().is_empty()).then(|| path.bounding_box())
}

fn correct_direction_in(layer: &mut Layer, side: LayerSide, outlines: &Outlines) -> Op<Option<Picked>> {
    let master = layer.master.clone();
    let shapes = layer.parts_mut(side).shapes;
    let mut input = Input::new(shapes, |_| true);
    if input.contours.is_empty() {
        return Ok(None);
    }
    let contours = std::mem::take(&mut input.contours);
    let directed =
        run_with_time_limit(TIME_LIMIT, move || po::correct_direction(&contours)).map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let done = Done { contours: directed.order.into_iter().map(Some).zip(directed.contours).collect(), selection: Vec::new() };
    let paths = to_paths(&input, &done)?;
    // Paths and components sorted together, as the paths among themselves are sorted: by the
    // left edge of their bounds, then the bottom edge; shapes without bounds go last.
    let corner = |r: Option<Rect>| r.map_or((f64::INFINITY, f64::INFINITY), |r| (r.x0, r.y0));
    let mut items: Vec<((f64, f64), Shape)> =
        paths.into_iter().map(|(_, path)| (corner(path.to_bez().ok().as_ref().and_then(bounds_of)), Shape::Path(path))).collect();
    for shape in shapes.iter() {
        if let Shape::Component(c) = shape {
            let b = outlines.get(&(c.glyph.clone(), master.clone())).and_then(|o| {
                let mut o = o.clone();
                o.apply_affine(c.affine());
                bounds_of(&o)
            });
            items.push((corner(b), shape.clone()));
        }
    }
    items.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.0.1.total_cmp(&b.0.1)));
    let mut items = items.into_iter().map(|(_, shape)| shape);
    let mut out = Vec::with_capacity(shapes.len());
    for shape in std::mem::take(shapes) {
        if matches!(shape, Shape::Path(_) | Shape::Component(_)) {
            out.extend(items.next());
        } else {
            out.push(shape);
        }
    }
    out.extend(items);
    *shapes = out;
    Ok(Some(Picked::default()))
}

fn correct_path_direction(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.correctPathDirection", v)?;
    let outlines = component_outlines(s.font()?, &named_glyphs(&p, s.font()?));
    run_layers(s, &p, "Correct Path Direction", &[], |layer, side, _| correct_direction_in(layer, side, &outlines))
}

fn correct_path_direction_for_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.correctPathDirectionForAllMasters", v)?;
    let outlines = component_outlines(s.font()?, &named_glyphs(&p, s.font()?));
    run_all_masters(s, &p, "Correct Path Direction for All Masters", master_or_special_layer, |layer, side, _| {
        correct_direction_in(layer, side, &outlines)
    })
}

// ── Extremes and Tidy up ────────────────────────────────────────────────────────────────────────

/// Runs `f` on each path that takes part (with its selected nodes, or `None` for all of them) and
/// puts each result in its place.
fn each_contour<F>(shapes: &mut Vec<Shape>, t: Option<&Targets>, f: F) -> Op<Option<Picked>>
where
    F: Fn(&po::Contour, Option<&[usize]>) -> std::result::Result<po::Contour, PathOpError> + Send + 'static,
{
    let choice = chosen_nodes(t, shapes)?;
    let input = Input::new(shapes, |i| choice.as_ref().is_none_or(|m| m.contains_key(&i)));
    let selected: Vec<Option<Vec<usize>>> = input.slots.iter().map(|s| choice.as_ref().and_then(|m| m.get(s).cloned().flatten())).collect();
    apply(shapes, input, TIME_LIMIT, move |contours| {
        let out = contours.iter().zip(&selected).map(|(c, sel)| f(c, sel.as_deref())).collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Done::in_place(out))
    })
}

fn extremes(s: &mut Session, v: &Value, id: &'static str, label: &str, force: bool) -> Result<Value> {
    let p = Params::new(id, v)?;
    let options = po::ExtremesOptions { force, grid: grid(s.font()?) };
    run_layers(s, &p, label, &["points", "shapes"], |layer, side, t| {
        each_contour(layer.parts_mut(side).shapes, t, move |c, sel| po::add_extremes(c, sel, options))
    })
}

fn add_extremes(s: &mut Session, v: &Value) -> Result<Value> {
    extremes(s, v, "path.addExtremes", "Add Extremes", false)
}

fn force_extremes(s: &mut Session, v: &Value) -> Result<Value> {
    extremes(s, v, "path.forceExtremes", "Force Extremes", true)
}

fn add_inflections(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.addInflections", v)?;
    let grid = grid(s.font()?);
    run_layers(s, &p, "Add Inflections", &["points", "shapes"], |layer, side, t| {
        each_contour(layer.parts_mut(side).shapes, t, move |c, sel| po::add_inflections(c, sel, grid).map(|(c, _)| c))
    })
}

fn tidy_up_paths(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.tidyUpPaths", v)?;
    let straighten = match p.opt_f64("straighten")? {
        Some(v) if !(0.0..=po::MAX_STRAIGHTEN).contains(&v) => {
            return Err(p.bad(format!("`straighten` must be between 0 and {}", po::MAX_STRAIGHTEN)));
        }
        v => v.unwrap_or(0.0),
    };
    run_layers(s, &p, "Tidy up Paths", &["points", "shapes"], |layer, side, t| {
        each_contour(layer.parts_mut(side).shapes, t, move |c, sel| po::tidy_up_straightening(c, sel, straighten))
    })
}

fn tidy_up_paths_for_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.tidyUpPathsForAllMasters", v)?;
    run_all_masters(s, &p, "Tidy up Paths for All Masters", master_layer, |layer, side, t| each_contour(layer.parts_mut(side).shapes, t, po::tidy_up))
}

// ── Open Corner, Reconnect Nodes, Cut with Line ─────────────────────────────────────────────────

/// The on-curve node before `node` on `path` (around the end of a closed path).
fn previous_on_curve(path: &Path, node: usize) -> Option<usize> {
    let n = path.nodes.len();
    (1..n).map(|d| (node + n - d) % n).filter(|&i| path.closed || i < node).find(|&i| path.nodes.get(i).is_some_and(Node::on_curve))
}

/// Open Corner's overlap: the `overlap` parameter, else half the master's first vertical stem
/// (for horizontal segments) and half its first horizontal stem (for vertical ones). A master
/// with stems of one direction uses them for both; one without stems uses a share of the em.
fn corner_overlap(p: &Params, font: &Font, master: &Master) -> Result<Vec2> {
    let bad = || p.bad(format!("`overlap` must be a number or [x, y], each above 0 and at most {MAX_OVERLAP}"));
    let check = |v: &Value| v.as_f64().filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_OVERLAP).ok_or_else(bad);
    match p.get("overlap") {
        None | Some(Value::Null) => {}
        Some(n @ Value::Number(_)) => {
            let v = check(n)?;
            return Ok(Vec2::new(v, v));
        }
        Some(Value::Array(a)) => {
            return match a.as_slice() {
                [x, y] => Ok(Vec2::new(check(x)?, check(y)?)),
                _ => Err(bad()),
            };
        }
        Some(other) => return Err(p.bad(format!("`overlap` must be a number or [x, y], found {}", kind(other)))),
    }
    let first = |horizontal: bool| {
        font.stem_widths(master, horizontal).first().copied().filter(|v| v.is_finite() && *v > 0.0 && *v <= 2.0 * MAX_OVERLAP).map(|v| v / 2.0)
    };
    let (vertical, horizontal) = (first(false), first(true));
    let fallback = f64::from(font.units_per_em) * DEFAULT_OVERLAP_SHARE;
    Ok(Vec2::new(vertical.or(horizontal).unwrap_or(fallback), horizontal.or(vertical).unwrap_or(fallback)))
}

fn open_corner(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.openCorner", v)?;
    let t = one_glyph(&p)?;
    takes_only(&p, &t, &["points", "segments"])?;
    let font = s.font()?;
    let step = grid(font);
    let layer = t.layer_id(font)?;
    let (overlap, corners) = {
        let r = t.resolve(font)?;
        let overlap = corner_overlap(&p, font, r.master)?;
        let mut corners: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for &(shape, node) in &t.points {
            if r.parts.path(shape).is_some_and(|path| path.nodes.get(node).is_some_and(Node::on_curve)) {
                corners.entry(shape).or_default().insert(node);
            }
        }
        for &(shape, node) in &t.segments {
            let Some(path) = r.parts.path(shape) else { continue };
            let set = corners.entry(shape).or_default();
            set.insert(node);
            set.extend(previous_on_curve(path, node));
        }
        (overlap, corners)
    };
    if corners.values().all(BTreeSet::is_empty) {
        return Err(p.bad("give corner nodes in `points`, or segments in `segments`"));
    }
    let picked = s.edit("Open Corner", |font| {
        t.edit_layer(font, |l| {
            let shapes = l.parts_mut(t.side).shapes;
            let input = Input::new(shapes, |i| corners.contains_key(&i));
            let lists: Vec<Vec<usize>> =
                input.slots.iter().map(|s| corners.get(s).map(|n| n.iter().copied().collect()).unwrap_or_default()).collect();
            apply(shapes, input, TIME_LIMIT, move |contours| {
                let mut done = Done { contours: Vec::new(), selection: Vec::new() };
                for (i, (c, nodes)) in contours.iter().zip(&lists).enumerate() {
                    let opened = po::open_corners(c, nodes, overlap, step)?;
                    done.selection.extend(opened.nodes.iter().map(|&n| (i, n)));
                    done.contours.push((Some(i), opened.contour));
                }
                Ok(done)
            })
            .map_err(|m| p.bad(m))
        })
    })?;
    Ok(json!({ "selection": selection_json(&p, &t, &layer, picked) }))
}

fn reconnect_nodes(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.reconnectNodes", v)?;
    let t = one_glyph(&p)?;
    takes_only(&p, &t, &["points"])?;
    if t.points.is_empty() {
        return Err(p.bad("give the nodes to reconnect in `points`"));
    }
    let font = s.font()?;
    let step = grid(font);
    let layer = t.layer_id(font)?;
    let picked = s.edit("Reconnect Nodes", |font| {
        t.edit_layer(font, |l| {
            let shapes = l.parts_mut(t.side).shapes;
            let input = Input::new(shapes, |_| true);
            let at: HashMap<usize, usize> = input.slots.iter().enumerate().map(|(c, &s)| (s, c)).collect();
            let nodes = t
                .points
                .iter()
                .map(|&(shape, node)| at.get(&shape).map(|&c| (c, node)).ok_or_else(|| p.bad(format!("point [{shape}, {node}] is not on a path"))))
                .collect::<Result<Vec<_>>>()?;
            apply(shapes, input, TIME_LIMIT, move |c| Ok(Done::rewired(po::reconnect_nodes(&c, &nodes, step)?))).map_err(|m| p.bad(m))
        })
    })?;
    Ok(json!({ "selection": selection_json(&p, &t, &layer, picked) }))
}

/// A point parameter `[x, y]`, within the coordinate range.
fn point(p: &Params, key: &str) -> Result<Point> {
    let bad = || p.bad(format!("`{key}` must be [x, y], two finite numbers within ±{MAX_COORDINATE}"));
    match p.array(key)?.as_slice() {
        [x, y] => {
            let (x, y) = (x.as_f64().ok_or_else(bad)?, y.as_f64().ok_or_else(bad)?);
            if in_range(x) && in_range(y) { Ok(Point::new(x, y)) } else { Err(bad()) }
        }
        _ => Err(bad()),
    }
}

fn cut_with_line(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.cutWithLine", v)?;
    let t = one_glyph(&p)?;
    takes_only(&p, &t, &["shapes"])?;
    let (from, to) = (point(&p, "from")?, point(&p, "to")?);
    if from == to {
        return Err(p.bad("`from` and `to` are the same point: the line has no length"));
    }
    let font = s.font()?;
    let step = grid(font);
    let layer = t.layer_id(font)?;
    let picked = s.edit("Cut with Line", |font| {
        t.edit_layer(font, |l| {
            if t.side == LayerSide::Background && l.info.background.is_none() {
                return Ok(None);
            }
            let shapes = l.parts_mut(t.side).shapes;
            let chosen = chosen_paths(Some(&t), shapes).map_err(|m| p.bad(m))?;
            let input = Input::new(shapes, |i| chosen.as_ref().is_none_or(|c| c.contains(&i)));
            apply(shapes, input, TIME_LIMIT, move |c| Ok(Done::rewired(po::cut_with_line(&c, from, to, step)?))).map_err(|m| p.bad(m))
        })
    })?;
    // The new nodes, or nothing when the line crossed no path.
    let picked = picked.map(|mut picked| {
        picked.shapes.clear();
        picked
    });
    Ok(json!({ "selection": selection_json(&p, &t, &layer, picked) }))
}

// ── Cubic and quadratic ─────────────────────────────────────────────────────────────────────────

/// A path's nodes by segment: each on-curve node with the off-curve nodes before it. In a closed
/// path the first group also takes the off-curve nodes at the end of the list, which wrap around
/// to it. `None` for a path with no on-curve node, or an open path with off-curve nodes before
/// its first or after its last on-curve node.
fn groups(path: &Path) -> Option<Vec<(usize, Vec<usize>)>> {
    let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut pending = Vec::new();
    for (i, n) in path.nodes.iter().enumerate() {
        if n.on_curve() {
            out.push((i, std::mem::take(&mut pending)));
        } else {
            pending.push(i);
        }
    }
    let first = out.first_mut()?;
    if path.closed {
        pending.append(&mut first.1);
        first.1 = pending;
    } else if !(pending.is_empty() && first.1.is_empty()) {
        return None;
    }
    Some(out)
}

/// `path` rebuilt group by group. `seg(group, start, end)` returns the nodes that go before the
/// group's end node and the end node itself; `start` is the point the segment starts from (none
/// for the first node of an open path). A closed path keeps its first on-curve node first.
fn rebuild(
    path: &Path,
    groups: &[(usize, Vec<usize>)],
    mut seg: impl FnMut(usize, Option<Point>, usize) -> Option<(Vec<Node>, Node)>,
) -> Option<Path> {
    let end_point = |g: usize| groups.get(g).and_then(|(e, _)| path.nodes.get(*e)).map(Node::point);
    let (mut head, mut body, mut tail) = (Vec::new(), Vec::new(), Vec::new());
    for (g, (end, _)) in groups.iter().enumerate() {
        let start = match g {
            0 if path.closed => end_point(groups.len().saturating_sub(1)),
            0 => None,
            _ => end_point(g - 1),
        };
        let (before, end_node) = seg(g, start, *end)?;
        if g == 0 {
            head.push(end_node);
            tail = before;
        } else {
            body.extend(before);
            body.push(end_node);
        }
    }
    let mut out = path.clone();
    head.extend(body);
    head.extend(tail);
    out.nodes = head;
    Some(out)
}

fn offs(path: &Path, list: &[usize]) -> Vec<Node> {
    list.iter().filter_map(|&i| path.nodes.get(i).cloned()).collect()
}

/// `path` with every quadratic segment as cubic ones, exactly, and how many segments changed;
/// `None` when it has none. A closed path of off-curve points only gets an on-curve node at the
/// implied point between its last and first one.
fn cubic_path(path: &Path) -> Option<(Path, usize)> {
    let mut path = path.clone();
    if path.closed && path.nodes.len() > 1 && path.nodes.iter().all(|n| !n.on_curve()) {
        let (a, b) = (path.nodes.last()?.point(), path.nodes.first()?.point());
        let m = a.midpoint(b);
        let mut node = Node::new(m.x, m.y, NodeKind::QCurve);
        node.smooth = true;
        path.nodes.push(node);
    }
    let groups = groups(&path)?;
    if !groups.iter().any(|(e, _)| path.nodes.get(*e).is_some_and(|n| n.kind == NodeKind::QCurve)) {
        return None;
    }
    let mut count = 0;
    let out = rebuild(&path, &groups, |g, start, end| {
        let e = path.nodes.get(end)?;
        let ctrl: Vec<Point> = groups.get(g)?.1.iter().filter_map(|&i| path.nodes.get(i).map(Node::point)).collect();
        let mut end_node = e.clone();
        let start = match start {
            Some(start) if e.kind == NodeKind::QCurve => start,
            _ => return Some((offs(&path, &groups.get(g)?.1), end_node)),
        };
        if ctrl.is_empty() {
            end_node.kind = NodeKind::Line;
            return Some((Vec::new(), end_node));
        }
        count += 1;
        let mut before = Vec::with_capacity(3 * ctrl.len());
        let mut from = start;
        for (j, c) in ctrl.iter().enumerate() {
            let next = ctrl.get(j + 1);
            let to = next.map_or(e.point(), |n| c.midpoint(*n));
            let (h1, h2) = (from.lerp(*c, 2.0 / 3.0), to.lerp(*c, 2.0 / 3.0));
            before.push(Node::new(h1.x, h1.y, NodeKind::OffCurve));
            before.push(Node::new(h2.x, h2.y, NodeKind::OffCurve));
            if next.is_some() {
                let mut m = Node::new(to.x, to.y, NodeKind::Curve);
                m.smooth = true;
                before.push(m);
            }
            from = to;
        }
        end_node.kind = NodeKind::Curve;
        Some((before, end_node))
    })?;
    (count > 0).then_some((out, count))
}

/// Do `paths` have the same node kinds in the same order (so they convert together)?
fn compatible(paths: &[&Path]) -> bool {
    paths.windows(2).all(|w| match w {
        [a, b] => a.closed == b.closed && a.nodes.len() == b.nodes.len() && a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.kind == y.kind),
        _ => true,
    })
}

/// The cubic segments of compatible `paths` as quadratic splines within `tolerance`, converted
/// together so the paths stay compatible; new off-curve points rounded to `step`. Returns the
/// new paths and how many segments of each changed; `None` when there is nothing to convert.
fn quadratic_paths(paths: &[&Path], tolerance: f64, step: f64) -> Option<(Vec<Path>, usize)> {
    let first = paths.first()?;
    let groups = groups(first)?;
    // For each group with a cubic segment, the nodes before its end node, path by path.
    let mut splines: Vec<Option<Vec<Vec<Node>>>> = vec![None; groups.len()];
    let mut count = 0;
    for (g, (end, ctrl)) in groups.iter().enumerate() {
        let [c1, c2] = ctrl.as_slice() else { continue };
        if first.nodes.get(*end).map(|n| n.kind) != Some(NodeKind::Curve) {
            continue;
        }
        let start = match g {
            0 if first.closed => groups.last().map(|(e, _)| *e),
            0 => None,
            _ => groups.get(g - 1).map(|(e, _)| *e),
        };
        let Some(start) = start else { continue };
        let cubics: Option<Vec<CubicBez>> = paths
            .iter()
            .map(|p| {
                let at = |i: usize| p.nodes.get(i).map(Node::point);
                Some(CubicBez::new(at(start)?, at(*c1)?, at(*c2)?, at(*end)?))
            })
            .collect();
        let Some(cubics) = cubics else { continue };
        let off = |q: Point| {
            let q = round_point(q, step);
            Node::new(q.x, q.y, NodeKind::OffCurve)
        };
        let per_path: Vec<Vec<Node>> = match cubics_to_quadratic_splines(&cubics, tolerance) {
            Some(list) => list
                .iter()
                .map(|spline| {
                    let points = spline.points();
                    points.get(1..points.len().saturating_sub(1)).unwrap_or(&[]).iter().map(|&q| off(q)).collect()
                })
                .collect(),
            // No common spline: each curve on its own, with an on-curve node between its pieces.
            None => cubics
                .iter()
                .map(|c| {
                    let quads: Vec<_> = c.to_quads(tolerance).map(|(_, _, q)| q).collect();
                    let mut nodes = Vec::new();
                    for (k, q) in quads.iter().enumerate() {
                        nodes.push(off(q.p1));
                        if k + 1 < quads.len() {
                            let p = round_point(q.p2, step);
                            nodes.push(Node::new(p.x, p.y, NodeKind::QCurve));
                        }
                    }
                    nodes
                })
                .collect(),
        };
        if per_path.iter().all(|n| !n.is_empty())
            && let Some(slot) = splines.get_mut(g)
        {
            *slot = Some(per_path);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let out = paths
        .iter()
        .enumerate()
        .map(|(l, p)| {
            rebuild(p, &groups, |g, _, end| {
                let mut e = p.nodes.get(end)?.clone();
                match splines.get(g).and_then(Option::as_ref).and_then(|per| per.get(l)) {
                    Some(before) => {
                        e.kind = NodeKind::QCurve;
                        Some((before.clone(), e))
                    }
                    None => Some((offs(p, &groups.get(g)?.1), e)),
                }
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((out, count))
}

/// Converts the paths among `shapes` one by one; returns how many segments changed.
fn convert_shapes(shapes: &mut [Shape], quadratic: bool, tolerance: f64, step: f64) -> usize {
    let mut count = 0;
    for shape in shapes {
        let Shape::Path(path) = shape else { continue };
        let done = if quadratic { quadratic_paths(&[&*path], tolerance, step).and_then(|(mut v, n)| Some((v.pop()?, n))) } else { cubic_path(path) };
        if let Some((new, n)) = done {
            *path = new;
            count += n;
        }
    }
    count
}

/// Converts the master layers of one glyph. For quadratic curves, layers whose paths have the
/// same structure are converted together so they stay compatible; otherwise each layer alone.
fn convert_layers(layers: &mut [&mut Layer], quadratic: bool, tolerance: f64, step: f64) -> usize {
    let joint = quadratic
        && layers.len() > 1
        && layers.windows(2).all(|w| match w {
            [a, b] => {
                a.shapes.len() == b.shapes.len()
                    && a.shapes.iter().zip(&b.shapes).all(|pair| match pair {
                        (Shape::Path(x), Shape::Path(y)) => compatible(&[x, y]),
                        (Shape::Path(_), _) | (_, Shape::Path(_)) => false,
                        _ => true,
                    })
            }
            _ => true,
        });
    if !joint {
        return layers.iter_mut().map(|l| convert_shapes(&mut l.shapes, quadratic, tolerance, step)).sum();
    }
    let shape_count = layers.first().map_or(0, |l| l.shapes.len());
    let mut count = 0;
    for i in 0..shape_count {
        let paths: Vec<&Path> = layers
            .iter()
            .filter_map(|l| match l.shapes.get(i) {
                Some(Shape::Path(p)) => Some(p),
                _ => None,
            })
            .collect();
        if paths.len() != layers.len() {
            continue;
        }
        let Some((new, n)) = quadratic_paths(&paths, tolerance, step) else { continue };
        for (layer, path) in layers.iter_mut().zip(new) {
            if let Some(Shape::Path(old)) = layer.shapes.get_mut(i) {
                *old = path;
            }
        }
        count += n;
    }
    count
}

fn convert(s: &mut Session, v: &Value, id: &'static str, label: &str, quadratic: bool) -> Result<Value> {
    let p = Params::new(id, v)?;
    let font = s.font()?;
    let tolerance = match p.opt_f64("tolerance")? {
        _ if !quadratic => 0.0,
        Some(t) if (MIN_TOLERANCE..=MAX_TOLERANCE).contains(&t) => t,
        Some(_) => return Err(p.bad(format!("`tolerance` must be between {MIN_TOLERANCE} and {MAX_TOLERANCE} units"))),
        None => fontcraft_compile::DEFAULT_CONVERSION_ERROR * f64::from(font.units_per_em),
    };
    let step = grid(font);
    let count = Cell::new(0usize);
    let mut out = if p.bool_or("allMasters", false)? {
        let (glyphs, single) = all_masters_glyphs(&p, font)?;
        s.edit(label, |font| {
            each_glyph(font, &glyphs, single, master_layer, |_, mut layers, _| {
                let n = convert_layers(&mut layers, quadratic, tolerance, step);
                count.set(count.get().saturating_add(n));
                Ok(n > 0)
            })
        })?
        .to_json()
    } else {
        run_layers(s, &p, label, &[], |layer, side, _| {
            let n = convert_shapes(layer.parts_mut(side).shapes, quadratic, tolerance, step);
            count.set(count.get().saturating_add(n));
            Ok(None)
        })?
    };
    if let Some(o) = out.as_object_mut() {
        o.insert("segments".into(), json!(count.get()));
    }
    Ok(out)
}

fn convert_to_cubic(s: &mut Session, v: &Value) -> Result<Value> {
    convert(s, v, "path.convertToCubic", "Convert to Cubic", false)
}

fn convert_to_quadratic(s: &mut Session, v: &Value) -> Result<Value> {
    convert(s, v, "path.convertToQuadratic", "Convert to Quadratic", true)
}

// ── Round Coordinates ───────────────────────────────────────────────────────────────────────────

fn round_in(layer: &mut Layer, side: LayerSide, step: f64) {
    let r = |v: f64| round_to_grid(v, step) + 0.0;
    let LayerPartsMut { shapes, anchors, .. } = layer.parts_mut(side);
    for shape in shapes.iter_mut() {
        match shape {
            Shape::Path(path) => {
                for n in &mut path.nodes {
                    (n.x, n.y) = (r(n.x), r(n.y));
                }
            }
            Shape::Component(c) => match &mut c.transform {
                Transform::Decomposed { pos, .. } => *pos = (r(pos.0), r(pos.1)),
                Transform::Matrix(m) => {
                    for v in m.iter_mut().skip(4) {
                        *v = r(*v);
                    }
                }
            },
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    for a in anchors.iter_mut() {
        (a.x, a.y) = (r(a.x), r(a.y));
    }
}

/// The font's grid step; whole units when the font has no grid.
fn round_step(font: &Font) -> f64 {
    let step = grid(font);
    if step > 0.0 { step } else { 1.0 }
}

fn round_coordinates(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.roundCoordinates", v)?;
    let step = round_step(s.font()?);
    let op = |layer: &mut Layer, side: LayerSide, _: Option<&Targets>| -> Op<Option<Picked>> {
        round_in(layer, side, step);
        Ok(None)
    };
    if p.bool_or("allMasters", false)? {
        run_all_masters(s, &p, "Round Coordinates", master_layer, op)
    } else {
        run_layers(s, &p, "Round Coordinates", &[], op)
    }
}

fn round_coordinates_for_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.roundCoordinatesForAllMasters", v)?;
    let step = round_step(s.font()?);
    run_all_masters(s, &p, "Round Coordinates for All Masters", master_layer, |layer, side, _| {
        round_in(layer, side, step);
        Ok(None)
    })
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::Shape as _;
    use fontcraft_model::NodeKind::{Curve, Line, OffCurve};

    use super::*;

    // A layer whose operation never finishes must come back unchanged, with the reason, and
    // without waiting for the worker.
    #[test]
    fn a_layer_that_runs_out_of_time_is_left_as_it_was() {
        let mut shapes = vec![Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)), Shape::Path(Path::rect(50.0, 50.0, 150.0, 150.0))];
        let before = shapes.clone();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let input = Input::new(&shapes, |_| true);
        let out = apply(&mut shapes, input, Duration::from_millis(20), move |c| {
            let _ = rx.recv();
            Ok(Done::merged(c))
        });
        drop(tx);
        match out {
            Err(why) => assert!(why.contains("longer than 0.02 s"), "{why}"),
            Ok(_) => panic!("the blocked job finished"),
        }
        assert_eq!(shapes, before);
    }

    #[test]
    fn surviving_nodes_keep_their_data_and_a_split_path_its_identifier_once() {
        let mut path = Path::rect(0.0, 0.0, 100.0, 100.0);
        path.info.identifier = Some("p1".into());
        if let Some(n) = path.nodes.first_mut() {
            n.info.name = Some("corner".into());
        }
        let mut shapes = vec![Shape::Path(path)];
        let input = Input::new(&shapes, |_| true);
        let picked = apply(&mut shapes, input, TIME_LIMIT, |c| {
            Ok(Done::rewired(po::cut_with_line(&c, Point::new(50.0, -10.0), Point::new(50.0, 110.0), 1.0)?))
        })
        .unwrap()
        .unwrap();
        assert_eq!(shapes.len(), 2);
        assert_eq!(picked.points.len(), 4, "two new nodes in each piece");
        let paths: Vec<&Path> = shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).collect();
        let ids: Vec<Option<&String>> = paths.iter().map(|p| p.info.identifier.as_ref()).collect();
        assert_eq!(ids.iter().filter(|i| i.is_some()).count(), 1, "{ids:?}");
        let named: Vec<&Node> = paths.iter().flat_map(|p| &p.nodes).filter(|n| n.info.name.is_some()).collect();
        assert_eq!(named.len(), 1);
        assert_eq!((named[0].x, named[0].y), (0.0, 0.0));
    }

    #[test]
    fn quadratic_splines_become_exact_cubics_and_back() {
        // One quadratic spline with two off-curve points: an implied on-curve point at (100, 100).
        let q = |x, y| Node::new(x, y, NodeKind::OffCurve);
        let path =
            Path::new(true, vec![Node::new(0.0, 0.0, NodeKind::Line), q(0.0, 100.0), q(200.0, 100.0), Node::new(200.0, 0.0, NodeKind::QCurve)]);
        let (cubic, n) = cubic_path(&path).unwrap();
        assert_eq!(n, 1);
        let kinds: Vec<NodeKind> = cubic.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [Line, OffCurve, OffCurve, Curve, OffCurve, OffCurve, Curve]);
        assert_eq!((cubic.nodes[3].x, cubic.nodes[3].y, cubic.nodes[3].smooth), (100.0, 100.0, true));
        let area = |p: &Path| p.to_bez().unwrap().area();
        assert!((area(&path) - area(&cubic)).abs() < 1e-9);
        let (back, _) = quadratic_paths(&[&cubic], 1.0, 1.0).unwrap();
        assert!(back[0].nodes.iter().all(|n| n.kind != Curve));
        assert!((area(&back[0]) - area(&path)).abs() < 200.0);
    }
}
