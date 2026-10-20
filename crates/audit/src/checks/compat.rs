//! Master compatibility checks (M12.53): the structural report from
//! [`crate::AuditServices::compatibility`] and the heuristic tests ported from fontTools
//! `varLib.interpolatable`. Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`]), the thresholds and a [`crate::Sink`]; their registry entries are in
//! `registry/variations_source.rs`.
//!
//! The heuristic tests (contour order, start point, kink, under- and overweight), the contour
//! statistics they compare and the order in which masters are compared are ported from fontTools
//! 4.66.1 (`varLib/interpolatable.py`, `interpolatableHelpers.py`,
//! `interpolatableTestContourOrder.py`, `interpolatableTestStartingPoint.py`,
//! `pens/statisticsPen.py`), Copyright (c) 2017 Just van Rossum and the fontTools authors, MIT
//! License. fontTools integrates the area moments with generated closed forms; here they are
//! integrated with six-point Gauss–Legendre quadrature, which is exact for the polynomials a
//! cubic segment gives. The minimum-cost matching is the Hungarian method, where fontTools calls
//! SciPy.
//!
//! Each master is compared with its neighbour toward the default master
//! (`audit.interpolationComparesNeighbours`): the masters' normalised locations form a minimum
//! spanning tree, walked outward from the master at the default location. A glyph's intermediate
//! layers are compared with the nearest master layer of the glyph.

use std::collections::VecDeque;

use fontcraft_geom::{BezPath, PathEl, Point, Vec2};
use fontcraft_model::{Font, Glyph, Layer, NodeKind, Path, Shape, axes};

use crate::services::{CompatIssue, CompatKind, CompatibilityReport, MAX_COMPAT_ISSUES};
use crate::{GlyphCtx, Hit, Location, MAX_LAYER_NODES, MAX_LAYER_SHAPES, Nth, Params, Sink, SourceCtx, Threshold, num};

/// Most masters the compatibility checks compare; a source with more is skipped with a reason.
pub(crate) const MAX_COMPAT_MASTERS: usize = 256;
/// Most intermediate layers of one glyph taken into the comparison.
pub(crate) const MAX_INTERMEDIATE_LAYERS: usize = 256;
/// Most contours and components the contour-order test matches (the matching takes cubic time;
/// [`MAX_COMPAT_GLYPH_WORK`] bounds the whole glyph).
pub(crate) const MAX_MATCHED_CONTOURS: usize = 256;
/// Most points a contour may have for the start-point and weight tests (they take quadratic
/// time); larger contours are left out of those two tests.
pub(crate) const MAX_START_POINT_NODES: usize = 2_000;

pub(crate) const TOLERANCE: Threshold = Threshold {
    name: "tolerance",
    default: 0.95,
    min: 0.0,
    max: 1.0,
    doc: "A score from 0 to 1 (best cost over stored cost, or halfway area over expected area); results below it are reported.",
};
pub(crate) const KINKINESS: Threshold =
    Threshold { name: "kinkiness", default: 0.5, min: 0.01, max: 10.0, doc: "Sensitivity of the kink test: a higher value reports smaller kinks." };
pub(crate) const IGNORE_MISSING: Threshold =
    Threshold { name: "ignoreMissing", default: 0.0, min: 0.0, max: 1.0, doc: "1 leaves out glyphs that a sparse master does not have." };

/// The fontTools defaults the kink test scales by: the smallest visible deviation is this share
/// of the em at the default kinkiness.
const KINK_LENGTH: f64 = 0.002;
const DEFAULT_KINKINESS: f64 = 0.5;
/// About the sine of 6°, the kink test's angle tolerance.
const KINK_T: f64 = 0.1;

// ---------------------------------------------------------------------------
// Where the masters sit and which pairs are compared

/// Design locations normalised around the origin master: per axis the origin's value is 0, the
/// smallest master value -1 and the largest 1.
struct Space {
    /// Per axis: minimum, default, maximum (design coordinates) and whether the axis is discrete.
    axes: Vec<(f64, f64, f64, bool)>,
}

impl Space {
    fn new(font: &Font) -> Self {
        let origin = axes::origin(font);
        let axes = (0..font.axes.len())
            .map(|i| {
                let (lo, hi) = font
                    .masters
                    .iter()
                    .map(|m| axes::design_value(&m.location, i))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
                let def = origin.map_or(lo, |m| axes::design_value(&m.location, i));
                let discrete = font.axes.get(i).is_some_and(|a| !a.values.is_empty());
                (lo.min(def), def, hi.max(def), discrete)
            })
            .collect();
        Self { axes }
    }

    fn normalise(&self, value: impl Fn(usize) -> f64) -> Vec<f64> {
        self.axes
            .iter()
            .enumerate()
            .map(|(i, &(lo, def, hi, _))| {
                let v = value(i);
                let n = if v < def && def > lo {
                    (v - def) / (def - lo)
                } else if v > def && hi > def {
                    (v - def) / (hi - def)
                } else {
                    0.0
                };
                if n.is_finite() { n } else { 0.0 }
            })
            .collect()
    }

    fn same_discrete(&self, a: &[f64], b: &[f64]) -> bool {
        self.axes.iter().zip(a.iter().zip(b)).all(|(ax, (x, y))| !ax.3 || x == y)
    }
}

fn dist2(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (y - x) * (y - x)).sum()
}

/// The order masters are visited in and each one's neighbour toward the default master.
struct Tree {
    order: Vec<usize>,
    parent: Vec<Option<usize>>,
    norm: Vec<Vec<f64>>,
}

fn find_root(root: &[usize], mut i: usize) -> usize {
    // Each step moves one link up; a chain is never longer than the number of masters.
    for _ in 0..root.len() {
        let Some(&up) = root.get(i) else { return i };
        if up == i {
            return i;
        }
        i = up;
    }
    i
}

fn master_tree(font: &Font, space: &Space) -> Tree {
    let n = font.masters.len();
    let norm: Vec<Vec<f64>> = font.masters.iter().map(|m| space.normalise(|i| axes::design_value(&m.location, i))).collect();
    // Kruskal: the cheapest edges first, edges only between masters on the same discrete values.
    let mut edges = Vec::new();
    for (i, a) in norm.iter().enumerate() {
        for (j, b) in norm.iter().enumerate().skip(i + 1) {
            if space.same_discrete(a, b) {
                edges.push((dist2(a, b), i, j));
            }
        }
    }
    edges.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut root: Vec<usize> = (0..n).collect();
    let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (_, i, j) in edges {
        let (a, b) = (find_root(&root, i), find_root(&root, j));
        if a == b {
            continue;
        }
        if let Some(slot) = root.get_mut(a) {
            *slot = b;
        }
        if let Some(list) = adjacent.get_mut(i) {
            list.push(j);
        }
        if let Some(list) = adjacent.get_mut(j) {
            list.push(i);
        }
    }
    for list in &mut adjacent {
        list.sort_unstable();
    }
    // Breadth first from the masters at the default location (on the continuous axes); masters
    // no base reaches start trees of their own.
    let bases = norm.iter().enumerate().filter(|(_, l)| l.iter().zip(&space.axes).all(|(v, ax)| ax.3 || *v == 0.0)).map(|(i, _)| i);
    let mut parent = vec![None; n];
    let mut seen = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut queue = VecDeque::new();
    for start in bases.collect::<Vec<_>>().into_iter().chain(0..n) {
        if seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        queue.push_back(start);
        while let Some(i) = queue.pop_front() {
            order.push(i);
            for &j in adjacent.get(i).map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(s) = seen.get_mut(j)
                    && !*s
                {
                    *s = true;
                    if let Some(p) = parent.get_mut(j) {
                        *p = Some(i);
                    }
                    queue.push_back(j);
                }
            }
        }
    }
    Tree { order, parent, norm }
}

/// A layer that takes part in the comparison: a master layer or an intermediate layer.
struct Part<'a> {
    layer: &'a Layer,
    /// The part this one is compared with.
    parent: Option<usize>,
    norm: Vec<f64>,
}

/// An intermediate layer that is nothing else (not an alternate, colour or smart-pole layer).
fn is_intermediate(layer: &Layer) -> bool {
    let a = &layer.info.attrs;
    !layer.is_master_layer()
        && !a.coordinates.is_empty()
        && a.axis_rules.is_empty()
        && a.color_palette.is_none()
        && !a.full_color
        && !a.svg
        && a.sbix_size.is_none()
        && a.smart_poles.is_empty()
}

/// The glyph's master layers in tree order, each with the nearest ancestor master the glyph has,
/// then its intermediate layers, each with the nearest master layer.
fn participants<'a>(font: &'a Font, glyph: &'a Glyph, tree: &Tree, space: &Space) -> Vec<Part<'a>> {
    let masters = font.masters.as_slice();
    let mut parts: Vec<Part<'a>> = Vec::new();
    let mut part_of: Vec<Option<usize>> = vec![None; masters.len()];
    for &m in &tree.order {
        let Some(master) = masters.get(m) else { continue };
        let Some(layer) = glyph.layer(&master.id) else { continue };
        let mut up = tree.parent.get(m).copied().flatten();
        let mut parent = None;
        for _ in 0..masters.len() {
            let Some(i) = up else { break };
            if let Some(p) = part_of.get(i).copied().flatten() {
                parent = Some(p);
                break;
            }
            up = tree.parent.get(i).copied().flatten();
        }
        if let Some(slot) = part_of.get_mut(m) {
            *slot = Some(parts.len());
        }
        parts.push(Part { layer, parent, norm: tree.norm.get(m).cloned().unwrap_or_default() });
    }
    let master_parts = parts.len();
    for layer in glyph.layers.iter().filter(|l| is_intermediate(l)).take(MAX_INTERMEDIATE_LAYERS) {
        let own = masters.iter().find(|m| m.id == layer.master);
        let coords = &layer.info.attrs.coordinates;
        let norm = space.normalise(|i| {
            coords.get(i).copied().flatten().filter(|v| v.is_finite()).unwrap_or_else(|| own.map_or(0.0, |m| axes::design_value(&m.location, i)))
        });
        let parent = (0..master_parts).min_by(|&a, &b| {
            let d = |p: usize| parts.get(p).map_or(f64::INFINITY, |p| dist2(&p.norm, &norm));
            d(a).total_cmp(&d(b))
        });
        parts.push(Part { layer, parent, norm });
    }
    parts
}

/// How a layer is named in messages: the master's name for a master layer, else the layer's.
fn label(font: &Font, layer: &Layer) -> String {
    if layer.is_master_layer()
        && let Ok(m) = font.master(&layer.master)
    {
        return m.name.clone();
    }
    layer.info.name.clone().unwrap_or_else(|| layer.id.clone())
}

// ---------------------------------------------------------------------------
// The structural report

/// The plain structural comparison of every glyph's masters and intermediate layers: missing
/// glyphs, path and node counts, node kinds, components and anchor names. The engine answers
/// [`crate::AuditServices::compatibility`] with it until the interpolation routine of M9 takes
/// over.
pub fn structural_report(font: &Font) -> CompatibilityReport {
    let mut report =
        CompatibilityReport { default_master: axes::origin(font).map(|m| m.id.clone()).unwrap_or_default(), ..CompatibilityReport::default() };
    if font.masters.len() < 2 || font.masters.len() > MAX_COMPAT_MASTERS {
        return report;
    }
    let space = Space::new(font);
    let tree = master_tree(font, &space);
    for glyph in font.glyphs.as_arcs() {
        let parts = participants(font, glyph, &tree, &space);
        let mut issues = Vec::new();
        let present = font.masters.iter().filter(|m| glyph.layer(&m.id).is_some()).count();
        if present >= 2 {
            for m in font.masters.iter().filter(|m| glyph.layer(&m.id).is_none()) {
                issues.push(issue(glyph, [&report.default_master, &m.id], CompatKind::MissingGlyph, None, None, format!("no layer for {}", m.name)));
            }
        }
        for part in &parts {
            let Some(base) = part.parent.and_then(|p| parts.get(p)) else { continue };
            compare_structure(glyph, base.layer, part.layer, &mut issues);
        }
        if !issues.is_empty() {
            report.compatible.insert(glyph.name.clone(), false);
        }
        for i in issues {
            if report.issues.len() < MAX_COMPAT_ISSUES {
                report.issues.push(i);
            } else {
                report.truncated = report.truncated.saturating_add(1);
            }
        }
    }
    report
}

fn issue(glyph: &Glyph, masters: [&str; 2], kind: CompatKind, path: Option<usize>, node: Option<usize>, detail: String) -> CompatIssue {
    CompatIssue { glyph: glyph.name.clone(), masters: masters.map(str::to_string), kind, path, node, detail }
}

fn kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Line => "line",
        NodeKind::Curve => "curve",
        NodeKind::QCurve => "quadratic curve",
        NodeKind::OffCurve => "off-curve",
    }
}

fn paths_of(layer: &Layer) -> Vec<&Path> {
    layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn compare_structure(glyph: &Glyph, a: &Layer, b: &Layer, out: &mut Vec<CompatIssue>) {
    let ids = [a.id.as_str(), b.id.as_str()];
    let (pa, pb) = (paths_of(a), paths_of(b));
    if pa.len() != pb.len() {
        out.push(issue(glyph, ids, CompatKind::PathCount, None, None, format!("{} paths against {}", pa.len(), pb.len())));
    } else {
        for (k, (x, y)) in pa.iter().zip(&pb).enumerate() {
            if x.nodes.len() != y.nodes.len() {
                out.push(issue(glyph, ids, CompatKind::NodeCount, Some(k), None, format!("{} nodes against {}", x.nodes.len(), y.nodes.len())));
                continue;
            }
            for (i, (m, n)) in x.nodes.iter().zip(&y.nodes).enumerate() {
                if m.kind != n.kind {
                    out.push(issue(
                        glyph,
                        ids,
                        CompatKind::NodeType,
                        Some(k),
                        Some(i),
                        format!("{} against {}", kind_name(m.kind), kind_name(n.kind)),
                    ));
                }
            }
        }
    }
    let components = |l: &'_ Layer| -> Vec<String> {
        l.shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Component(c) => Some(c.glyph.clone()),
                _ => None,
            })
            .collect()
    };
    let (ca, cb) = (components(a), components(b));
    if ca != cb {
        let detail = if ca.len() == cb.len() {
            format!("components {} against {}", ca.join(", "), cb.join(", "))
        } else {
            format!("{} components against {}", ca.len(), cb.len())
        };
        out.push(issue(glyph, ids, CompatKind::ComponentMismatch, None, None, detail));
    }
    let anchors = |l: &'_ Layer| -> Vec<String> {
        let mut names: Vec<String> = l.anchors.iter().map(|x| x.name.clone()).collect();
        names.sort();
        names
    };
    let (na, nb) = (anchors(a), anchors(b));
    if na != nb {
        let list = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join(", ") };
        out.push(issue(glyph, ids, CompatKind::AnchorMismatch, None, None, format!("anchors {} against {}", list(&na), list(&nb))));
    }
}

/// Reports the issues of one kind from the services' report.
fn report_issues(ctx: &SourceCtx<'_>, sink: &mut Sink, kind: CompatKind, code: &'static str) {
    let font = ctx.audit.font;
    if font.masters.len() < 2 {
        return;
    }
    let report = match ctx.services.compatibility(font) {
        Ok(r) => r,
        Err(e) => {
            sink.skip(e.reason);
            return;
        }
    };
    for i in report.issues.iter().filter(|i| i.kind == kind) {
        let glyph = font.glyph(&i.glyph);
        let layer_of = |id: &str| glyph.and_then(|g| g.layers.iter().find(|l| l.id == id));
        let name = |id: &str| match layer_of(id) {
            Some(l) => label(font, l),
            None => font.master(id).map(|m| m.name.clone()).unwrap_or_else(|_| id.to_string()),
        };
        let [first, second] = &i.masters;
        let target = layer_of(second);
        // The path index of the report counts paths only; results address shapes.
        let shape = i.path.and_then(|k| {
            let l = target?;
            l.shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Path(_))).nth(k).map(|(s, _)| s)
        });
        let what = match kind {
            CompatKind::MissingGlyph => "The glyph has no layer",
            CompatKind::PathCount => "The path count differs",
            CompatKind::NodeCount => "The node count of a path differs",
            CompatKind::NodeType => "A node's type differs",
            CompatKind::ComponentMismatch => "The components differ",
            CompatKind::AnchorMismatch => "The anchors differ",
        };
        let mut message = match kind {
            CompatKind::MissingGlyph => format!("{what} for master {}.", name(second)),
            _ => format!("{what} between {} and {}: {}.", name(first), name(second), i.detail),
        };
        if let (Some(k), true) = (i.path, matches!(kind, CompatKind::NodeCount | CompatKind::NodeType)) {
            message = match i.node {
                Some(n) => format!("{message} Path {}, node {}.", Nth(k), Nth(n)),
                None => format!("{message} Path {}.", Nth(k)),
            };
        }
        let mut place = Location {
            glyph: i.glyph.clone(),
            layer: target.map(|l| l.id.clone()).unwrap_or_default(),
            master: Some(target.map_or_else(|| second.clone(), |l| l.master.clone())),
            masters: vec![first.clone(), second.clone()],
            ..Location::default()
        };
        if let Some(s) = shape {
            place.shapes = vec![s];
            if let Some(n) = i.node {
                place.points = vec![[s, n]];
            }
        }
        sink.push(Hit::new(code, message).place(place));
    }
}

pub(crate) fn glyph_missing(ctx: &SourceCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    if params.get(&IGNORE_MISSING) >= 0.5 {
        return;
    }
    report_issues(ctx, sink, CompatKind::MissingGlyph, "missing");
}

pub(crate) fn path_count(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    report_issues(ctx, sink, CompatKind::PathCount, "path-count");
}

pub(crate) fn node_count(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    report_issues(ctx, sink, CompatKind::NodeCount, "node-count");
}

pub(crate) fn node_type(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    report_issues(ctx, sink, CompatKind::NodeType, "node-type");
}

pub(crate) fn component_mismatch(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    report_issues(ctx, sink, CompatKind::ComponentMismatch, "component-mismatch");
}

pub(crate) fn anchor_mismatch(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    report_issues(ctx, sink, CompatKind::AnchorMismatch, "anchor-mismatch");
}

// ---------------------------------------------------------------------------
// Designspace checks

const SAME: f64 = 1e-9;

pub(crate) fn masters_share_location(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let masters = font.masters.as_slice();
    if masters.len() < 2 {
        return;
    }
    if masters.len() > MAX_COMPAT_MASTERS {
        sink.skip(format!("the source has more than {MAX_COMPAT_MASTERS} masters"));
        return;
    }
    let n = font.axes.len().max(1);
    let at = |loc: &dyn Fn(usize) -> f64| (0..n).map(loc).collect::<Vec<f64>>();
    let same = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= SAME);
    let locations: Vec<Vec<f64>> = masters.iter().map(|m| at(&|i| axes::design_value(&m.location, i))).collect();
    for (i, (a, la)) in masters.iter().zip(&locations).enumerate() {
        for (b, lb) in masters.iter().zip(&locations).skip(i + 1) {
            if same(la, lb) {
                let place = Location { master: Some(b.id.clone()), masters: vec![a.id.clone(), b.id.clone()], ..Location::default() };
                sink.push(Hit::new("masters-share-location", format!("Masters {} and {} sit at the same location.", a.name, b.name)).place(place));
            }
        }
    }
    for glyph in font.glyphs.as_arcs() {
        let layers: Vec<(&Layer, Vec<f64>)> = glyph
            .layers
            .iter()
            .filter(|l| is_intermediate(l))
            .take(MAX_INTERMEDIATE_LAYERS)
            .map(|l| {
                let own = masters.iter().find(|m| m.id == l.master);
                let coords = &l.info.attrs.coordinates;
                let loc = at(&|i| {
                    coords
                        .get(i)
                        .copied()
                        .flatten()
                        .filter(|v| v.is_finite())
                        .unwrap_or_else(|| own.map_or(0.0, |m| axes::design_value(&m.location, i)))
                });
                (l, loc)
            })
            .collect();
        for (k, (layer, loc)) in layers.iter().enumerate() {
            let twin = masters
                .iter()
                .zip(&locations)
                .find(|(m, l)| glyph.layer(&m.id).is_some() && same(l, loc))
                .map(|(m, _)| (m.id.clone(), m.name.clone()))
                .or_else(|| layers.iter().take(k).find(|(_, l)| same(l, loc)).map(|(o, _)| (o.id.clone(), label(font, o))));
            if let Some((id, name)) = twin {
                let place = Location {
                    glyph: glyph.name.clone(),
                    layer: layer.id.clone(),
                    master: Some(layer.master.clone()),
                    masters: vec![id, layer.id.clone()],
                    ..Location::default()
                };
                let message = format!("Layer {} of {} sits at the same location as {name}.", label(font, layer), glyph.name);
                sink.push(Hit::new("layer-shares-location", message).place(place));
            }
        }
    }
}

pub(crate) fn instance_outside_axis_range(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    if font.masters.len() < 2 {
        return;
    }
    for (i, axis) in font.axes.iter().enumerate() {
        let (lo, hi) = font
            .masters
            .iter()
            .map(|m| axes::design_value(&m.location, i))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
        for inst in font.instances.iter().filter(|x| x.exports && x.kind.as_deref() != Some("variable")) {
            let v = axes::design_value(&inst.location, i);
            if v >= lo - SAME && v <= hi + SAME {
                continue;
            }
            let place = Location { instance: Some(inst.name.clone()), axis: Some(axis.tag.clone()), ..Location::default() };
            let message = format!(
                "Instance {} sits at {} on {}, outside the masters' range {} to {}: it needs extrapolation.",
                inst.name,
                num(v),
                axis.name,
                num(lo),
                num(hi)
            );
            sink.push(Hit::new("outside-axis-range", message).data("value", v).data("minimum", lo).data("maximum", hi).place(place));
        }
    }
}

// ---------------------------------------------------------------------------
// Contours as the heuristic tests read them

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
}

impl Op {
    fn end(self) -> Point {
        match self {
            Op::Line(p) | Op::Quad(_, p) | Op::Cubic(_, _, p) => p,
        }
    }

    fn lerp(self, other: Op) -> Option<Op> {
        let m = |a: Point, b: Point| a.midpoint(b);
        match (self, other) {
            (Op::Line(a), Op::Line(b)) => Some(Op::Line(m(a, b))),
            (Op::Quad(a, b), Op::Quad(c, d)) => Some(Op::Quad(m(a, c), m(b, d))),
            (Op::Cubic(a, b, c), Op::Cubic(d, e, f)) => Some(Op::Cubic(m(a, d), m(b, e), m(c, f))),
            _ => None,
        }
    }
}

/// The rotations and reflections of a contour that keep its sequence of on- and off-curve points
/// (`add_isomorphisms`): the complex vectors of the points forward and reversed, and per
/// isomorphism whether it is reversed, the rotation, and the point it would start at.
struct Iso {
    fwd: Vec<Vec2>,
    rev: Vec<Vec2>,
    rots: Vec<(bool, usize, usize)>,
}

/// A contour or a component of one layer.
struct Item {
    shape: usize,
    /// A contour's start point and segments, the last one ending on the start; empty for a
    /// component.
    start: Point,
    ops: Vec<Op>,
    /// A contour's points from its start in path order, whether each is on-curve, and its node
    /// index; empty for a component.
    points: Vec<(Point, bool)>,
    nodes: Vec<usize>,
    green: [f64; 6],
    control: [f64; 6],
    iso: Option<Iso>,
}

fn quads(offs: &[Point], end: Point, out: &mut Vec<Op>) {
    let mut it = offs.iter().peekable();
    while let Some(&c) = it.next() {
        match it.peek() {
            Some(&&next) => out.push(Op::Quad(c, c.midpoint(next))),
            None => out.push(Op::Quad(c, end)),
        }
    }
}

fn path_item(shape: usize, path: &Path) -> Item {
    let n = path.nodes.len();
    let mut ops = Vec::new();
    let mut points = Vec::new();
    let mut nodes = Vec::new();
    let start;
    match path.nodes.iter().position(|x| x.on_curve()) {
        Some(first) => {
            start = path.nodes.get(first).map_or(Point::ZERO, |x| x.point());
            let mut pending: Vec<Point> = Vec::new();
            for step in 1..=n {
                let Some(node) = path.nodes.get((first + step) % n) else { continue };
                let p = node.point();
                match node.kind {
                    NodeKind::OffCurve => pending.push(p),
                    NodeKind::Line => ops.push(Op::Line(p)),
                    NodeKind::Curve => match pending.as_slice() {
                        [] => ops.push(Op::Line(p)),
                        [a] => ops.push(Op::Quad(*a, p)),
                        [a, b] => ops.push(Op::Cubic(*a, *b, p)),
                        more => quads(more, p, &mut ops),
                    },
                    NodeKind::QCurve => match pending.as_slice() {
                        [] => ops.push(Op::Line(p)),
                        more => quads(more, p, &mut ops),
                    },
                }
                if node.on_curve() {
                    pending.clear();
                }
            }
            for step in 0..n {
                let k = (first + step) % n;
                if let Some(node) = path.nodes.get(k) {
                    points.push((node.point(), node.on_curve()));
                    nodes.push(k);
                }
            }
        }
        None => {
            // A closed quadratic contour without on-curve points starts between its last and
            // first off-curve points. Its points take no part in the start-point and kink tests.
            let offs: Vec<Point> = path.nodes.iter().map(|x| x.point()).collect();
            start = match (offs.first(), offs.last()) {
                (Some(a), Some(b)) => b.midpoint(*a),
                _ => Point::ZERO,
            };
            quads(&offs, start, &mut ops);
        }
    }
    let mut stats = Stats::new(start);
    for op in &ops {
        stats.op(*op);
    }
    stats.close();
    let (green, control) = stats.vectors();
    let iso = isomorphisms(&points);
    Item { shape, start, ops, points, nodes, green, control, iso }
}

fn component_item(shape: usize, outline: Option<BezPath>) -> Item {
    let mut stats = Stats::empty();
    let mut started = false;
    for el in outline.iter().flat_map(|b| b.elements()) {
        match *el {
            PathEl::MoveTo(p) => {
                if started {
                    stats.close();
                }
                stats.move_to(p);
                started = true;
            }
            PathEl::LineTo(p) => stats.op(Op::Line(p)),
            PathEl::QuadTo(a, p) => stats.op(Op::Quad(a, p)),
            PathEl::CurveTo(a, b, p) => stats.op(Op::Cubic(a, b, p)),
            PathEl::ClosePath => {
                stats.close();
                started = false;
            }
        }
    }
    if started {
        stats.close();
    }
    let (green, control) = stats.vectors();
    Item { shape, start: Point::ZERO, ops: Vec::new(), points: Vec::new(), nodes: Vec::new(), green, control, iso: None }
}

// ---------------------------------------------------------------------------
// Contour statistics (statisticsPen.py)

/// Six-point Gauss–Legendre nodes and weights on 0…1: exact for polynomials up to degree 11,
/// and the moment integrands of a cubic segment are of degree 11 at most.
const GAUSS: [(f64, f64); 6] = [
    (0.033_765_242_898_423_975, 0.085_662_246_189_585_17),
    (0.169_395_306_766_867_74, 0.180_380_786_524_069_3),
    (0.380_690_406_958_401_5, 0.233_956_967_286_345_53),
    (0.619_309_593_041_598_5, 0.233_956_967_286_345_53),
    (0.830_604_693_233_132_3, 0.180_380_786_524_069_3),
    (0.966_234_757_101_576, 0.085_662_246_189_585_17),
];

/// The area moments of the closed contours drawn so far (Green's theorem: each is the integral of
/// a function of x and y times dx along the outline, with the sign that makes counter-clockwise
/// areas positive), and the control points for the control-polygon statistics.
struct Stats {
    area: f64,
    mx: f64,
    my: f64,
    mxx: f64,
    mxy: f64,
    myy: f64,
    start: Point,
    current: Point,
    control: Vec<Point>,
}

impl Stats {
    fn new(start: Point) -> Self {
        Self { area: 0.0, mx: 0.0, my: 0.0, mxx: 0.0, mxy: 0.0, myy: 0.0, start, current: start, control: vec![start] }
    }

    /// Statistics of an outline whose contours each begin with [`Stats::move_to`].
    fn empty() -> Self {
        Self { control: Vec::new(), ..Self::new(Point::ZERO) }
    }

    fn move_to(&mut self, p: Point) {
        self.start = p;
        self.current = p;
        self.control.push(p);
    }

    fn op(&mut self, op: Op) {
        let p0 = self.current;
        match op {
            Op::Line(p) => {
                self.cubic(p0, p0.lerp(p, 1.0 / 3.0), p0.lerp(p, 2.0 / 3.0), p);
                self.control.push(p);
            }
            Op::Quad(c, p) => {
                self.cubic(p0, p0.lerp(c, 2.0 / 3.0), p.lerp(c, 2.0 / 3.0), p);
                self.control.extend([c, p]);
            }
            Op::Cubic(a, b, p) => {
                self.cubic(p0, a, b, p);
                self.control.extend([a, b, p]);
            }
        }
        self.current = op.end();
    }

    fn close(&mut self) {
        if self.current != self.start {
            self.op(Op::Line(self.start));
        }
    }

    fn cubic(&mut self, p0: Point, p1: Point, p2: Point, p3: Point) {
        for (t, w) in GAUSS {
            let mt = 1.0 - t;
            let (b0, b1, b2, b3) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
            let x = b0 * p0.x + b1 * p1.x + b2 * p2.x + b3 * p3.x;
            let y = b0 * p0.y + b1 * p1.y + b2 * p2.y + b3 * p3.y;
            let dx = 3.0 * (mt * mt * (p1.x - p0.x) + 2.0 * mt * t * (p2.x - p1.x) + t * t * (p3.x - p2.x));
            let f = -w * dx;
            self.area += f * y;
            self.mx += f * x * y;
            self.my += f * y * y / 2.0;
            self.mxx += f * x * x * y;
            self.mxy += f * x * y * y / 2.0;
            self.myy += f * y * y * y / 3.0;
        }
    }

    /// The statistics vectors of the outline and of its control polygon
    /// (`contour_vector_from_stats`).
    fn vectors(&self) -> ([f64; 6], [f64; 6]) {
        let green = if self.area.abs() < 1e-12 {
            [0.0; 6]
        } else {
            let a = self.area;
            let (mean_x, mean_y) = (self.mx / a, self.my / a);
            let var_x = self.mxx / a - mean_x * mean_x;
            let var_y = self.myy / a - mean_y * mean_y;
            let cov = self.mxy / a - mean_x * mean_y;
            vector(a, mean_x, mean_y, var_x, var_y, cov)
        };
        let nodes = &self.control;
        let n = nodes.len();
        let control = if n == 0 {
            [0.0; 6]
        } else {
            let next = nodes.iter().cycle().skip(1);
            let area = nodes.iter().zip(next).map(|(p, q)| p.x * q.y - q.x * p.y).sum::<f64>() / 2.0;
            let count = n as f64;
            let (sx, sy) = nodes.iter().fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
            let (var_x, var_y, cov) = if n > 1 {
                let sxx: f64 = nodes.iter().map(|p| p.x * p.x).sum();
                let syy: f64 = nodes.iter().map(|p| p.y * p.y).sum();
                let sxy: f64 = nodes.iter().map(|p| p.x * p.y).sum();
                ((sxx - sx * sx / count) / (count - 1.0), (syy - sy * sy / count) / (count - 1.0), (sxy - sx * sy / count) / (count - 1.0))
            } else {
                (0.0, 0.0, 0.0)
            };
            vector(area, sx / count, sy / count, var_x, var_y, cov)
        };
        (finite_or_zero(green), finite_or_zero(control))
    }
}

fn finite_or_zero(v: [f64; 6]) -> [f64; 6] {
    if v.iter().all(|x| x.is_finite()) { v } else { [0.0; 6] }
}

/// `StatisticsBase._update` followed by `contour_vector_from_stats`.
fn vector(area: f64, mean_x: f64, mean_y: f64, var_x: f64, var_y: f64, cov: f64) -> [f64; 6] {
    let (sd_x, sd_y) = (var_x.abs().sqrt(), var_y.abs().sqrt());
    let correlation = if sd_x * sd_y == 0.0 { 0.0 } else { (cov / (sd_x * sd_y)).clamp(-1.0, 1.0) };
    let correlation = if correlation.abs() > 1e-3 { correlation } else { 0.0 };
    let size = area.abs().sqrt();
    [size.copysign(area), mean_x, mean_y, sd_x * 2.0, sd_y * 2.0, correlation * size]
}

// ---------------------------------------------------------------------------
// Isomorphisms and the start-point test (interpolatableHelpers.py,
// interpolatableTestStartingPoint.py)

/// Per point: the point, three times the step to the next one, the turn, and four times the
/// signed square root of the turn's cross product (`points_complex_vector`; the weights are
/// fontTools').
fn complex_vector(points: &[Point]) -> Vec<Vec2> {
    let n = points.len();
    let at = |i: usize| points.get(i % n.max(1)).copied().unwrap_or(Point::ZERO).to_vec2();
    let mut v = Vec::with_capacity(n * 4);
    for i in 0..n {
        let (p0, p1, p2) = (at(i), at(i + 1), at(i + 2));
        let (d0, d1) = (p1 - p0, p2 - p1);
        let cross = d0.cross(d1);
        v.extend([p0, d0 * 3.0, d1 - d0, Vec2::new(cross.abs().sqrt().copysign(cross) * 4.0, 0.0)]);
    }
    v
}

fn isomorphisms(points: &[(Point, bool)]) -> Option<Iso> {
    let n = points.len();
    if n == 0 || n > MAX_START_POINT_NODES {
        return None;
    }
    let reference: Vec<bool> = points.iter().map(|p| p.1).collect();
    let fwd_points: Vec<Point> = points.iter().map(|p| p.0).collect();
    let rev_points: Vec<Point> = fwd_points.iter().rev().copied().collect();
    let rev_flags: Vec<bool> = reference.iter().rev().copied().collect();
    let mut rots = Vec::new();
    for (reverse, flags) in [(false, &reference), (true, &rev_flags)] {
        for i in 0..n {
            if (0..n).all(|k| flags.get((k + i) % n) == reference.get(k)) {
                rots.push((reverse, i, if reverse { n - 1 - i } else { i }));
            }
        }
    }
    Some(Iso { fwd: complex_vector(&fwd_points), rev: complex_vector(&rev_points), rots })
}

/// The squared distance between `c0` and `c1` rotated left by `rot` points, over the shorter of
/// the two (`vdiff_hypot2_complex` on `rot_list`).
fn rotated_cost(c0: &[Vec2], c1: &[Vec2], rot: usize) -> f64 {
    let len = c1.len();
    if len == 0 {
        return 0.0;
    }
    let shift = rot.saturating_mul(4) % len;
    c0.iter().enumerate().map(|(k, a)| c1.get((k + shift) % len).map_or(0.0, |b| (*b - *a).hypot2())).sum()
}

/// The index and value of the first smallest cost.
fn argmin(costs: &[f64]) -> Option<(usize, f64)> {
    costs.iter().copied().enumerate().fold(None, |best, (i, c)| match best {
        Some((_, b)) if b <= c => best,
        _ => Some((i, c)),
    })
}

/// The stretch and rotation recovered from a contour's statistics vector, applied to a point
/// (the start-point test's second try).
fn covariance_map(v: &[f64; 6]) -> impl Fn(Vec2) -> Vec2 {
    let (sx, sy) = (v[3] * 0.5, v[4] * 0.5);
    let correlation = if v[5] != 0.0 && v[0] != 0.0 { v[5] / v[0].abs() } else { 0.0 };
    let (a, c) = (sx * sx, sy * sy);
    let b = correlation * sx * sy;
    let delta = (((a - c) * 0.5).powi(2) + b * b).sqrt();
    let l1 = (a + c) * 0.5 + delta;
    let l2 = ((a + c) * 0.5 - delta).max(0.0);
    let theta = if b != 0.0 {
        (l1 - a).atan2(b)
    } else if a < c {
        std::f64::consts::FRAC_PI_2
    } else {
        0.0
    };
    let (sin, cos) = theta.sin_cos();
    let (kx, ky) = (l1.sqrt(), l2.sqrt());
    move |p: Vec2| Vec2::new(kx * cos * p.x - ky * sin * p.y, kx * sin * p.x + ky * cos * p.y)
}

/// The start-point test of contour `ix` of `g0` against its partner in `g1`: the score, the
/// proposed start point (an index in the partner's points) and whether to reverse.
fn start_point(g0: &[Item], g1: &[Item], ix: usize, tolerance: f64, matching: &[usize]) -> Option<(f64, usize, bool)> {
    let a = g0.get(ix)?;
    let b = g1.get(*matching.get(ix)?)?;
    let (ia, ib) = (a.iso.as_ref()?, b.iso.as_ref()?);
    let costs = |c0: &[Vec2], fwd: &[Vec2], rev: &[Vec2]| -> Vec<f64> {
        ib.rots.iter().map(|&(r, i, _)| rotated_cost(c0, if r { rev } else { fwd }, i)).collect()
    };
    let first = costs(&ia.fwd, &ib.fwd, &ib.rev);
    let (best, mut min_cost) = argmin(&first)?;
    let mut first_cost = *first.first()?;
    let &(reverse, _, proposed) = ib.rots.get(best)?;
    if min_cost < first_cost * tolerance {
        // Two round shapes slightly rotated against each other look like a wrong start point
        // one or two points off; compare them again in their principal axes. fontTools reads
        // the point count of contour `ix` of the second master here, not of its partner.
        let num_points = g1.get(ix).map_or(0, |x| x.points.len());
        let leeway = 3;
        if !reverse && (proposed <= leeway || proposed + leeway >= num_points) {
            let (t0, t1) = (covariance_map(&a.green), covariance_map(&b.green));
            let c0: Vec<Vec2> = ia.fwd.iter().map(|p| t0(*p)).collect();
            let fwd: Vec<Vec2> = ib.fwd.iter().map(|p| t1(*p)).collect();
            let rev: Vec<Vec2> = ib.rev.iter().map(|p| t1(*p)).collect();
            let again = costs(&c0, &fwd, &rev);
            if let (Some((_, m)), Some(f)) = (argmin(&again), again.first()) {
                min_cost = m;
                first_cost = *f;
            }
        }
    }
    let score = if first_cost != 0.0 { min_cost / first_cost } else { 1.0 };
    Some((score, proposed, reverse))
}

// ---------------------------------------------------------------------------
// The contour-order test (interpolatableTestContourOrder.py)

fn cost_matrix(m0: &[[f64; 6]], m1: &[[f64; 6]]) -> Vec<Vec<f64>> {
    m0.iter().map(|a| m1.iter().map(|b| a.iter().zip(b).map(|(x, y)| (y - x) * (y - x)).sum()).collect()).collect()
}

fn cell(g: &[Vec<f64>], i: usize, j: usize) -> f64 {
    g.get(i).and_then(|r| r.get(j)).copied().unwrap_or(f64::INFINITY)
}

/// The minimum-cost perfect matching of a square cost matrix (the Hungarian method with
/// potentials): `matching[row]` is the column. `None` when a cost is not finite.
fn min_cost_matching(g: &[Vec<f64>]) -> Option<Vec<usize>> {
    let n = g.len();
    if g.iter().any(|r| r.len() != n || r.iter().any(|v| !v.is_finite())) {
        return None;
    }
    // 1-based, column 0 is the virtual start.
    let mut u = vec![0.0f64; n + 1];
    let mut v = vec![0.0f64; n + 1];
    let mut p = vec![0usize; n + 1];
    let mut way = vec![0usize; n + 1];
    for i in 1..=n {
        *p.get_mut(0)? = i;
        let mut j0 = 0usize;
        let mut minv = vec![f64::INFINITY; n + 1];
        let mut used = vec![false; n + 1];
        // Each round adds one column to the tree, so at most n + 1 rounds.
        let mut rounds = 0;
        loop {
            rounds += 1;
            if rounds > n + 1 {
                return None;
            }
            *used.get_mut(j0)? = true;
            let i0 = *p.get(j0)?;
            let row = i0.checked_sub(1)?;
            let mut delta = f64::INFINITY;
            let mut j1 = 0usize;
            for j in 1..=n {
                if *used.get(j)? {
                    continue;
                }
                let cur = cell(g, row, j - 1) - u.get(i0)? - v.get(j)?;
                if cur < *minv.get(j)? {
                    *minv.get_mut(j)? = cur;
                    *way.get_mut(j)? = j0;
                }
                if *minv.get(j)? < delta {
                    delta = *minv.get(j)?;
                    j1 = j;
                }
            }
            if j1 == 0 {
                return None;
            }
            for j in 0..=n {
                if *used.get(j)? {
                    let row = *p.get(j)?;
                    *u.get_mut(row)? += delta;
                    *v.get_mut(j)? -= delta;
                } else {
                    *minv.get_mut(j)? -= delta;
                }
            }
            j0 = j1;
            if *p.get(j0)? == 0 {
                break;
            }
        }
        for _ in 0..=n {
            let j1 = *way.get(j0)?;
            *p.get_mut(j0)? = *p.get(j1)?;
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut matching = vec![0usize; n];
    for j in 1..=n {
        let row = *p.get(j)?;
        *matching.get_mut(row.checked_sub(1)?)? = j - 1;
    }
    Some(matching)
}

/// The best matching, its cost and the cost of keeping the stored order (`matching_for_vectors`).
fn matching_for(m0: &[[f64; 6]], m1: &[[f64; 6]]) -> Option<(Vec<usize>, f64, f64)> {
    let g = cost_matrix(m0, m1);
    let matching = min_cost_matching(&g)?;
    let cost = matching.iter().enumerate().map(|(i, &j)| cell(&g, i, j)).sum();
    let identity = (0..g.len()).map(|i| cell(&g, i, i)).sum();
    Some((matching, cost, identity))
}

/// The contour-order score and, when another order is cheaper, the matching: `matching[i]` is the
/// item of `g1` that pairs with item `i` of `g0`.
fn contour_order(g0: &[Item], g1: &[Item]) -> (f64, Option<Vec<usize>>) {
    let n = g0.len();
    if n <= 1 || n > MAX_MATCHED_CONTOURS || g1.len() != n {
        return (1.0, None);
    }
    let control0: Vec<[f64; 6]> = g0.iter().map(|x| x.control).collect();
    let control1: Vec<[f64; 6]> = g1.iter().map(|x| x.control).collect();
    let green0: Vec<[f64; 6]> = g0.iter().map(|x| x.green).collect();
    let green1: Vec<[f64; 6]> = g1.iter().map(|x| x.green).collect();
    let Some((m_control, cost_control, id_control)) = matching_for(&control0, &control1) else { return (1.0, None) };
    if cost_control == id_control {
        return (1.0, None);
    }
    let Some((m_green, cost_green, id_green)) = matching_for(&green0, &green1) else { return (1.0, None) };
    if cost_green == id_green {
        return (1.0, None);
    }
    // Contours that only run the other way are the start-point test's business.
    let flipped = |v: &[[f64; 6]]| -> Vec<[f64; 6]> {
        v.iter()
            .map(|x| {
                let mut y = *x;
                y[0] = -y[0];
                y
            })
            .collect()
    };
    if matching_for(&control0, &flipped(&control1)).is_some_and(|(_, c, i)| c == i) {
        return (1.0, None);
    }
    if matching_for(&green0, &flipped(&green1)).is_some_and(|(_, c, i)| c == i) {
        return (1.0, None);
    }
    let (matching, cost, identity) =
        if cost_control * id_green < cost_green * id_control { (m_control, cost_control, id_control) } else { (m_green, cost_green, id_green) };
    let score = if identity != 0.0 { cost / identity } else { 1.0 };
    (score, Some(matching))
}

// ---------------------------------------------------------------------------
// One glyph's comparison (interpolatable.py, test_gen)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Test {
    ContourOrder,
    StartPoint,
    Kink,
    Underweight,
    Overweight,
}

/// One heuristic result between part `a` (toward the default) and part `b`.
struct Found {
    test: Test,
    a: usize,
    b: usize,
    /// The item of part `b` concerned.
    item: Option<usize>,
    /// A node index in that item's path.
    node: Option<usize>,
    score: f64,
    /// The proposed order of part `b`'s items.
    order: Vec<usize>,
    reverse: bool,
}

/// The masters' normalised locations and comparison tree, built once per font
/// ([`crate::AuditContext`] keeps it); `None` for a source with fewer than two or more than
/// [`MAX_COMPAT_MASTERS`] masters.
pub(crate) struct MasterOrder(Option<(Space, Tree)>);

impl MasterOrder {
    pub(crate) fn new(font: &Font) -> Self {
        let n = font.masters.len();
        if !(2..=MAX_COMPAT_MASTERS).contains(&n) {
            return Self(None);
        }
        let space = Space::new(font);
        let tree = master_tree(font, &space);
        Self(Some((space, tree)))
    }
}

/// The glyph's parts and their items, ready for the heuristic tests.
struct Prepared<'a> {
    parts: Vec<Part<'a>>,
    items: Vec<Vec<Item>>,
    open: bool,
    /// Per part: the contour-order score against its parent part and, when another order is
    /// cheaper, the matching (see [`contour_order`]); `None` for a part that is not compared.
    orders: Vec<Option<(f64, Option<Vec<usize>>)>>,
    /// Set when the comparison would take more than [`MAX_COMPAT_GLYPH_WORK`]: the heuristic
    /// tests skip the glyph with this reason.
    too_much: Option<String>,
}

/// One glyph as the compatibility tests see it, built once and shared by them
/// ([`crate::GlyphCtx`] keeps it).
pub(crate) struct Compared<'a> {
    /// `Err` holds the reason the glyph is skipped; `Ok(None)` means there is nothing to compare.
    prep: Result<Option<Prepared<'a>>, String>,
}

impl<'a> Compared<'a> {
    pub(crate) fn new(audit: &'a crate::AuditContext<'a>, glyph: &'a Glyph) -> Self {
        Self { prep: prepare(audit, glyph) }
    }

    /// The prepared glyph, or `None` after recording the skip reason, if any.
    fn get(&self, sink: &mut Sink) -> Option<&Prepared<'a>> {
        match &self.prep {
            Ok(prep) => prep.as_ref(),
            Err(reason) => {
                sink.skip(reason.clone());
                None
            }
        }
    }
}

/// Most work units the heuristic tests may spend on one glyph (see [`comparison_work`]); a glyph
/// over it is skipped with a reason.
pub(crate) const MAX_COMPAT_GLYPH_WORK: u64 = 250_000_000;

/// An upper estimate of the heuristic tests' work on `parts`: the isomorphisms of every contour
/// (quadratic in its points), and per compared pair the four contour-order matchings (cubic in
/// the contours) and the start-point costs (quadratic in the points).
fn comparison_work(parts: &[Part<'_>]) -> u64 {
    let items = |layer: &Layer| layer.shapes.iter().filter(|s| matches!(s, Shape::Path(_) | Shape::Component(_))).count() as u64;
    let squares = |layer: &Layer| {
        layer
            .shapes
            .iter()
            .map(|s| match s {
                Shape::Path(p) if p.nodes.len() <= MAX_START_POINT_NODES => (p.nodes.len() as u64).saturating_pow(2),
                Shape::Path(p) => p.nodes.len() as u64,
                _ => 1,
            })
            .fold(0u64, u64::saturating_add)
    };
    let mut work = 0u64;
    for part in parts {
        let own = squares(part.layer);
        work = work.saturating_add(own);
        let Some(parent) = part.parent.and_then(|a| parts.get(a)) else { continue };
        let n = items(part.layer);
        if n > 1 && n == items(parent.layer) && n <= MAX_MATCHED_CONTOURS as u64 {
            work = work.saturating_add(n.saturating_pow(3).saturating_mul(4)).saturating_add(n.saturating_pow(2).saturating_mul(4));
        }
        work = work.saturating_add(own.saturating_mul(8));
    }
    work
}

fn prepare<'a>(audit: &'a crate::AuditContext<'a>, glyph: &'a Glyph) -> Result<Option<Prepared<'a>>, String> {
    let font = audit.font;
    let masters = font.masters.len();
    if masters < 2 {
        return Ok(None);
    }
    if masters > MAX_COMPAT_MASTERS {
        return Err(format!("the source has more than {MAX_COMPAT_MASTERS} masters"));
    }
    let Some((space, tree)) = &audit.compat_masters().0 else { return Ok(None) };
    let parts = participants(font, glyph, tree, space);
    let mut open = false;
    for part in &parts {
        let layer = part.layer;
        let nodes = layer
            .shapes
            .iter()
            .map(|s| match s {
                Shape::Path(p) => p.nodes.len(),
                _ => 1,
            })
            .fold(0usize, usize::saturating_add);
        if layer.shapes.len() > MAX_LAYER_SHAPES || nodes > MAX_LAYER_NODES {
            return Err(format!("a layer has more than {MAX_LAYER_SHAPES} shapes or {MAX_LAYER_NODES} nodes"));
        }
        for shape in &layer.shapes {
            if let Shape::Path(p) = shape {
                if p.nodes.iter().any(|n| !(crate::contour::in_range(n.x) && crate::contour::in_range(n.y))) {
                    return Err("a node's coordinate is not a usable number".to_string());
                }
                open |= !p.closed;
            }
        }
    }
    let mut prep = Prepared { parts, items: Vec::new(), open, orders: Vec::new(), too_much: None };
    if open {
        return Ok(Some(prep));
    }
    if comparison_work(&prep.parts) > MAX_COMPAT_GLYPH_WORK {
        prep.too_much = Some(format!("comparing the glyph's layers would take more than {MAX_COMPAT_GLYPH_WORK} steps"));
        return Ok(Some(prep));
    }
    prep.items = prep
        .parts
        .iter()
        .map(|part| {
            let shapes = part.layer.shapes.iter().enumerate();
            let paths = shapes.clone().filter_map(|(i, s)| match s {
                Shape::Path(p) => Some(path_item(i, p)),
                _ => None,
            });
            let components = shapes.filter_map(|(i, s)| match s {
                Shape::Component(c) => Some(component_item(i, font.component_outline(c, &part.layer.master).ok())),
                _ => None,
            });
            paths.chain(components).collect()
        })
        .collect();
    prep.orders = prep
        .parts
        .iter()
        .enumerate()
        .map(|(b, part)| {
            let a = part.parent?;
            let (g0, g1) = (prep.items.get(a)?, prep.items.get(b)?);
            if g0.is_empty() || g1.is_empty() || g0.len() != g1.len() {
                return None;
            }
            Some(contour_order(g0, g1))
        })
        .collect();
    Ok(Some(prep))
}

/// Runs `test` on every compared pair of the glyph's parts.
fn compare_pairs(font: &Font, prep: &Prepared<'_>, test: Test, tolerance: f64, kinkiness: f64) -> Vec<Found> {
    let mut out = Vec::new();
    let upem = f64::from(font.units_per_em);
    for (b, part) in prep.parts.iter().enumerate() {
        let Some(a) = part.parent else { continue };
        let Some(Some(order)) = prep.orders.get(b) else { continue };
        let (Some(g0), Some(g1)) = (prep.items.get(a), prep.items.get(b)) else { continue };
        compare_pair(g0, g1, (a, b), order, upem, test, tolerance, kinkiness, &mut out);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn compare_pair(
    g0: &[Item],
    g1: &[Item],
    (a, b): (usize, usize),
    (score, proposed): &(f64, Option<Vec<usize>>),
    upem: f64,
    test: Test,
    tolerance: f64,
    kinkiness: f64,
    out: &mut Vec<Found>,
) {
    let found = |test, item, node, score| Found { test, a, b, item, node, score, order: Vec::new(), reverse: false };
    let score = *score;
    let mut matching: Vec<usize> = (0..g0.len()).collect();
    if score < tolerance
        && let Some(m) = proposed
    {
        if test == Test::ContourOrder {
            out.push(Found { order: m.clone(), ..found(Test::ContourOrder, None, None, score) });
        }
        matching = m.clone();
    }
    if matches!(test, Test::StartPoint | Test::Underweight | Test::Overweight) {
        for ix in 0..g0.len() {
            let Some(j) = matching.get(ix).copied() else { continue };
            let (Some(c0), Some(c1)) = (g0.get(ix), g1.get(j)) else { continue };
            let (Some(i0), Some(i1)) = (c0.iso.as_ref(), c1.iso.as_ref()) else { continue };
            if i0.rots.is_empty() || i0.rots.len() != i1.rots.len() {
                continue;
            }
            if test == Test::StartPoint {
                if let Some((score, point, reverse)) = start_point(g0, g1, ix, tolerance, &matching)
                    && score < tolerance
                {
                    let node = c1.nodes.get(point).copied();
                    out.push(Found { reverse, ..found(Test::StartPoint, Some(j), node, score) });
                }
                continue;
            }
            // The weight tests: the halfway contour of two contours that run the same way.
            if (c0.green[0] < 0.0) != (c1.green[0] < 0.0) || c0.ops.len() != c1.ops.len() {
                continue;
            }
            let Some(ops) = c0.ops.iter().zip(&c1.ops).map(|(x, y)| x.lerp(*y)).collect::<Option<Vec<Op>>>() else { continue };
            let mut mid = Stats::new(c0.start.midpoint(c1.start));
            for op in ops {
                mid.op(op);
            }
            mid.close();
            let (mid_vector, _) = mid.vectors();
            let (size0, size1, mid_size) = (c0.green[0] * c0.green[0], c1.green[0] * c1.green[0], mid_vector[0] * mid_vector[0]);
            if test == Test::Underweight {
                let expected = (size0 * size1).sqrt();
                if expected * tolerance > mid_size + 1e-5 {
                    let score = if expected != 0.0 { mid_size / expected } else { 0.0 };
                    out.push(found(Test::Underweight, Some(j), None, score));
                }
            } else {
                let expected = size0.max(size1);
                if tolerance > 0.0 && 1e-5 + expected / tolerance < mid_size {
                    let score = if mid_size != 0.0 { expected / mid_size } else { 0.0 };
                    out.push(found(Test::Overweight, Some(j), None, score));
                }
            }
        }
    }
    if test == Test::Kink {
        let threshold = upem * KINK_LENGTH * DEFAULT_KINKINESS / kinkiness;
        for ix in 0..g0.len() {
            let Some(j) = matching.get(ix).copied() else { continue };
            let (Some(c0), Some(c1)) = (g0.get(ix), g1.get(j)) else { continue };
            for (i, score) in kinks(&c0.points, &c1.points, tolerance, kinkiness, threshold) {
                out.push(found(Test::Kink, Some(j), c1.nodes.get(i).copied(), score));
            }
        }
    }
}

/// The points of `p0` and `p1` (the same contour in two masters) where the halfway contour turns
/// a smooth node into a visible corner, with each one's score.
fn kinks(p0: &[(Point, bool)], p1: &[(Point, bool)], tolerance: f64, kinkiness: f64, threshold: f64) -> Vec<(usize, f64)> {
    let n = p0.len();
    let mut out = Vec::new();
    if n == 0 || n != p1.len() {
        return out;
    }
    let get = |p: &[(Point, bool)], i: usize| p.get(i % n).copied().unwrap_or((Point::ZERO, false));
    let sine = |d0: Vec2, d1: Vec2| {
        let l = d0.hypot() * d1.hypot();
        (l != 0.0).then(|| d0.cross(d1) / l)
    };
    for i in 0..n {
        let (a, b) = (get(p0, i), get(p1, i));
        if !a.1 || !b.1 {
            continue;
        }
        let (a_prev, b_prev) = (get(p0, i + n - 1), get(p1, i + n - 1));
        let (a_next, b_next) = (get(p0, i + 1), get(p1, i + 1));
        // At least one handle on each side.
        if (a_prev.1 && b_prev.1) || (a_next.1 && b_next.1) {
            continue;
        }
        let (d0p, d0n) = (a.0 - a_prev.0, a_next.0 - a.0);
        let (d1p, d1n) = (b.0 - b_prev.0, b_next.0 - b.0);
        let (Some(s0), Some(s1)) = (sine(d0p, d0n), sine(d1p, d1n)) else { continue };
        if s0.abs() > KINK_T || s1.abs() > KINK_T {
            continue;
        }
        if d0p.dot(d0n) < 0.0 || d1p.dot(d1n) < 0.0 {
            continue;
        }
        let r0 = d0p.hypot() / (d0p.hypot() + d0n.hypot());
        let r1 = d1p.hypot() / (d1p.hypot() + d1n.hypot());
        if (r0 - r1).abs() < KINK_T {
            continue;
        }
        let mid = a.0.midpoint(b.0);
        let (mid_prev, mid_next) = (a_prev.0.midpoint(b_prev.0), a_next.0.midpoint(b_next.0));
        let (m0, m1) = (mid - mid_prev, mid_next - mid);
        let Some(s_mid) = sine(m0, m1) else { continue };
        if s_mid.abs() * (tolerance * kinkiness) <= KINK_T {
            continue;
        }
        let cross = s_mid * m0.hypot() * m1.hypot();
        let arc = (m0 + m1).hypot();
        if arc == 0.0 {
            continue;
        }
        let deviation = (cross / arc).abs();
        if deviation < threshold || deviation / arc > KINK_T {
            continue;
        }
        out.push((i, KINK_T / (s_mid.abs() * kinkiness)));
    }
    out
}

// ---------------------------------------------------------------------------
// The glyph-scope runners

pub(crate) fn open_path(ctx: &GlyphCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let Some(prep) = ctx.compat().get(sink) else { return };
    if !prep.open {
        return;
    }
    for part in &prep.parts {
        let layer = part.layer;
        for (s, shape) in layer.shapes.iter().enumerate() {
            let Shape::Path(p) = shape else { continue };
            if p.closed {
                continue;
            }
            let place = Location { layer: layer.id.clone(), master: Some(layer.master.clone()), ..Location::default() };
            let message = format!("Path {} is open in {}, so the glyph's other compatibility tests are skipped.", Nth(s), label(font, layer));
            sink.push(Hit::new("open-path", message).shapes(vec![s]).place(place));
        }
    }
}

fn heuristic(ctx: &GlyphCtx<'_>, sink: &mut Sink, test: Test, tolerance: f64, kinkiness: f64) {
    let font = ctx.audit.font;
    let Some(prep) = ctx.compat().get(sink) else { return };
    if prep.open {
        return;
    }
    if let Some(reason) = &prep.too_much {
        sink.skip(reason.clone());
        return;
    }
    for f in compare_pairs(font, prep, test, tolerance, kinkiness) {
        let (Some(pa), Some(pb)) = (prep.parts.get(f.a), prep.parts.get(f.b)) else { continue };
        let items = prep.items.get(f.b).map(Vec::as_slice).unwrap_or(&[]);
        let shape = f.item.and_then(|i| items.get(i)).map(|x| x.shape);
        let (la, lb) = (label(font, pa.layer), label(font, pb.layer));
        let place = Location {
            layer: pb.layer.id.clone(),
            master: Some(pb.layer.master.clone()),
            masters: vec![pa.layer.id.clone(), pb.layer.id.clone()],
            ..Location::default()
        };
        let score = num((f.score * 1000.0).round() / 1000.0);
        let hit = match f.test {
            Test::ContourOrder => {
                let order: Vec<usize> = f.order.iter().filter_map(|&j| items.get(j)).map(|x| x.shape).collect();
                let moved: Vec<usize> = order.iter().zip(items).filter(|(s, x)| **s != x.shape).map(|(s, _)| *s).collect();
                let list = order.iter().map(|s| Nth(*s).to_string()).collect::<Vec<_>>().join(", ");
                let message = format!("The shapes of {lb} match those of {la} better in the order {list} (score {score}).");
                let full = layer_order(pb.layer.shapes.len(), items, &order);
                let hit = Hit::new("contour-order", message).shapes(moved);
                match full {
                    Some(full) => {
                        sink.push(hit.data("score", f.score).place(Location { order: full, ..place }));
                        continue;
                    }
                    None => hit.without_fix(),
                }
            }
            Test::StartPoint => {
                let mut message = match f.node {
                    Some(n) => format!("A path in {lb} starts at a different node than in {la}; node {} matches its start better", Nth(n)),
                    None => format!("A path in {lb} starts at a different node than in {la}"),
                };
                if f.reverse {
                    message.push_str(" once the path is reversed");
                }
                message.push_str(&format!(" (score {score})."));
                let mut hit = Hit::new("wrong-start-point", message).data("reverse", if f.reverse { 1.0 } else { 0.0 });
                if let Some(s) = shape {
                    hit = hit.shapes(vec![s]);
                    if let Some(n) = f.node {
                        hit = hit.points(vec![[s, n]]).data("node", n as f64);
                    }
                }
                // The node is counted before any reversal; the fix reverses the path first.
                if f.node.is_none() { hit.without_fix() } else { hit }
            }
            Test::Kink => {
                let message = match f.node {
                    Some(n) => format!("Node {} is smooth in {la} and {lb} but makes a corner halfway between them (score {score}).", Nth(n)),
                    None => format!("A smooth node makes a corner halfway between {la} and {lb} (score {score})."),
                };
                let mut hit = Hit::new("kink", message);
                if let (Some(s), Some(n)) = (shape, f.node) {
                    hit = hit.shapes(vec![s]).points(vec![[s, n]]).data("node", n as f64);
                }
                hit
            }
            Test::Underweight => {
                let message = format!(
                    "A path halfway between {la} and {lb} encloses less area than expected (score {score}): its points may be matched wrongly."
                );
                Hit::new("underweight", message).shapes(shape.into_iter().collect())
            }
            Test::Overweight => {
                let message = format!("A path halfway between {la} and {lb} encloses more area than either of them (score {score}).");
                Hit::new("overweight", message).shapes(shape.into_iter().collect())
            }
        };
        sink.push(hit.data("score", f.score).place(place));
    }
}

/// The shape order of a whole layer of `len` shapes that puts `order[k]` (the shape matched with
/// item `k` of the reference master) where item `k` stands now: every slot that holds an item
/// takes its partner, and the other shapes keep their place. `None` when the result is not a
/// permutation of the layer's shapes.
fn layer_order(len: usize, items: &[Item], order: &[usize]) -> Option<Vec<usize>> {
    if items.len() != order.len() {
        return None;
    }
    let mut full: Vec<usize> = (0..len).collect();
    for (item, &shape) in items.iter().zip(order) {
        *full.get_mut(item.shape)? = shape;
    }
    let mut seen = vec![false; len];
    for &s in &full {
        let slot = seen.get_mut(s)?;
        if *slot {
            return None;
        }
        *slot = true;
    }
    Some(full)
}

pub(crate) fn contour_order_check(ctx: &GlyphCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    heuristic(ctx, sink, Test::ContourOrder, params.get(&TOLERANCE), KINKINESS.default);
}

pub(crate) fn wrong_start_point(ctx: &GlyphCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    heuristic(ctx, sink, Test::StartPoint, params.get(&TOLERANCE), KINKINESS.default);
}

pub(crate) fn kink(ctx: &GlyphCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    heuristic(ctx, sink, Test::Kink, params.get(&TOLERANCE), params.get(&KINKINESS));
}

pub(crate) fn underweight(ctx: &GlyphCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    heuristic(ctx, sink, Test::Underweight, params.get(&TOLERANCE), KINKINESS.default);
}

pub(crate) fn overweight(ctx: &GlyphCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    heuristic(ctx, sink, Test::Overweight, params.get(&TOLERANCE), KINKINESS.default);
}

#[cfg(test)]
mod tests;
