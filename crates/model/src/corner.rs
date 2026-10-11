//! Corner components fitted into outlines.
//!
//! Ported from fontc `glyphs-reader/src/corner_components.rs` (Apache-2.0, commit 4504578),
//! which is itself based on glyphsLib `filters/cornerComponents.py` (Apache-2.0); see `NOTICE`.
//!
//! A corner component is a `Corner` hint on a layer: it names a corner glyph (`_corner.serif`)
//! and the node of a path it sits on, with a scale and an alignment. The corner glyph's first
//! path, an open path drawn around the layer origin (or its `origin` anchor), is scaled, rotated
//! to the host path's strokes by the alignment, and spliced into the host path in place of the
//! node: the incoming stroke is cut where the corner starts and the outgoing one where it ends.
//! The corner glyph's other paths are added to the layer as they are fitted. Inserted nodes are
//! rounded to whole units, as both reference compilers round them.
//!
//! Beyond the port, where the reference compilers do nothing:
//! - The entry anchor (`left`, or `right` for a corner whose path faces right) moves the fitted
//!   corner along its own baseline until the anchor lies on the incoming stroke.
//! - The exit anchor (`right`, or `left` for a right-facing corner) stands in for the path's end
//!   node when the corner is rotated and when the outgoing stroke is cut; the end node itself
//!   stays where it was drawn.
//! - Intermediate layers of the corner glyph named with a width and height in percent
//!   (`Width 150 Height 100`) are interpolated for the hint's scale on each axis they vary; an
//!   axis without such a layer is stretched.
//! - A corner on an extra node (an intersection of two paths) is left out: fitting it needs the
//!   overlaps removed first.

use fontcraft_geom::{Affine, CubicBez, Line, ParamCurve, ParamCurveArclen, ParamCurveNearest, PathSeg, Point, QuadBez, Vec2};
use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value};

use crate::{Family, Font, Glyph, Hint, Layer, MAX_OUTLINE_ELEMENTS, Node, NodeKind, Path, Shape, in_range};

/// The hint type of a corner component.
pub const CORNER_HINT: &str = "Corner";
/// Largest corner scale, in either direction, as a factor (100 is 10,000 %).
pub const MAX_CORNER_SCALE: f64 = 100.0;
/// Most node moves fitting the corners of one layer may cost: splicing a corner into a long path
/// moves every node after it.
pub const MAX_CORNER_WORK: usize = 50_000_000;

/// How a corner follows a diagonal stem (the hint's `options`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CornerAlignment {
    /// The start follows the diagonal; rotated to the outgoing stroke (`options` 0).
    #[default]
    Left,
    /// The end follows the diagonal; rotated to the incoming stroke (1).
    Right,
    /// Rotated to the mean of both strokes' angles (2).
    Both,
    /// A value the reference stores and the fitting treats as unaligned (3).
    Unused,
    /// Not rotated (4).
    Unaligned,
}

impl CornerAlignment {
    /// The alignment a stored `options` value stands for; an unknown value reads as the default,
    /// as fontc reads it.
    pub fn from_option(value: f64) -> Self {
        match value {
            v if v == 1.0 => Self::Right,
            v if v == 2.0 => Self::Both,
            v if v == 3.0 => Self::Unused,
            v if v == 4.0 => Self::Unaligned,
            _ => Self::Left,
        }
    }

    /// The stored `options` value.
    pub fn option(self) -> i64 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Both => 2,
            Self::Unused => 3,
            Self::Unaligned => 4,
        }
    }
}

/// Where a corner sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerTarget {
    /// A node of a path: the shape index and the node index.
    Node { shape: usize, node: usize },
    /// An extra node: the intersection of the segment ending at `node` of path `shape` with the
    /// segment ending at `other_node` of path `other_shape`.
    Intersection { shape: usize, node: usize, other_shape: usize, other_node: usize },
}

/// A corner component as a `Corner` hint stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerHint {
    /// The corner glyph.
    pub name: String,
    pub target: CornerTarget,
    /// Horizontal and vertical scale as factors; a negative factor mirrors the corner.
    pub scale: Vec2,
    pub alignment: CornerAlignment,
}

impl CornerHint {
    /// A corner of `name` on node `node` of path `shape`, unscaled and aligned to the left.
    pub fn new(name: impl Into<String>, shape: usize, node: usize) -> Self {
        Self { name: name.into(), target: CornerTarget::Node { shape, node }, scale: Vec2::new(1.0, 1.0), alignment: CornerAlignment::Left }
    }

    /// Whether the scale mirrors the corner (one factor negative).
    pub fn is_flipped(&self) -> bool {
        self.scale.x * self.scale.y < 0.0
    }

    /// The hint that stores this corner, with the keys a version 3 `.glyphs` file writes.
    pub fn to_hint(&self) -> Hint {
        let int = |v: usize| Value::Number(Number::int(i64::try_from(v).unwrap_or(i64::MAX)));
        let float = |v: f64| Number::float(v).map_or_else(|| Value::Number(Number::int(1)), Value::Number);
        let pair = |items: Vec<Value>| Value::Array(Array { items, layout: Layout::Compact, ..Default::default() });
        let mut d = Dict::new();
        d.insert("name", Value::String(Str::new(self.name.clone())));
        if self.alignment != CornerAlignment::Left {
            d.insert("options", Value::Number(Number::int(self.alignment.option())));
        }
        let origin = match self.target {
            CornerTarget::Node { shape, node } => vec![int(shape), int(node)],
            CornerTarget::Intersection { shape, node, other_shape, other_node } => vec![int(shape), int(node), int(other_shape), int(other_node)],
        };
        d.insert("origin", pair(origin));
        if self.scale != Vec2::new(1.0, 1.0) {
            d.insert("scale", pair(vec![float(self.scale.x), float(self.scale.y)]));
        }
        d.insert("type", Value::String(Str::new(CORNER_HINT)));
        Hint { family: Family::Glyphs, value: Value::Dict(d) }
    }
}

/// Why a corner could not be fitted.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("corner component `{corner}`: {message}")]
pub struct CornerError {
    pub corner: String,
    pub message: String,
}

/// Whether `hint` is a corner component.
pub fn is_corner(hint: &Hint) -> bool {
    hint.kind() == Some(CORNER_HINT)
}

/// The corner component `hint` stores; `Ok(None)` when it is another kind of hint, `Err` when
/// it is a corner whose keys cannot be read.
pub fn corner_hint(hint: &Hint) -> Result<Option<CornerHint>, String> {
    if !is_corner(hint) {
        return Ok(None);
    }
    let Some(d) = hint.value.as_dict() else { return Ok(None) };
    let name = d.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("the hint names no corner glyph")?;
    let origin = numbers(d.get("origin").ok_or("the hint has no origin")?).ok_or("the hint's origin is not a list of numbers")?;
    let index = |v: f64| (v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= f64::from(u32::MAX)).then(|| v as usize);
    let indices: Option<Vec<usize>> = origin.iter().map(|&v| index(v)).collect();
    let target = match indices.as_deref() {
        Some(&[shape, node]) => CornerTarget::Node { shape, node },
        Some(&[shape, node, other_shape, other_node]) => CornerTarget::Intersection { shape, node, other_shape, other_node },
        _ => return Err("the hint's origin is not a path index and a node index".into()),
    };
    let scale = match d.get("scale") {
        None => Vec2::new(1.0, 1.0),
        Some(v) => match numbers(v).as_deref() {
            Some(&[x, y]) => Vec2::new(x, y),
            _ => return Err("the hint's scale is not two numbers".into()),
        },
    };
    if !(scale.x.is_finite() && scale.y.is_finite() && scale.x.abs() <= MAX_CORNER_SCALE && scale.y.abs() <= MAX_CORNER_SCALE) {
        return Err(format!("the scale ({}, {}) is not a number or is beyond ±{MAX_CORNER_SCALE}", scale.x, scale.y));
    }
    let alignment = d.get("options").and_then(number).map_or(CornerAlignment::Left, CornerAlignment::from_option);
    Ok(Some(CornerHint { name: name.to_string(), target, scale, alignment }))
}

/// A number, or a string that spells one.
fn number(v: &Value) -> Option<f64> {
    v.as_num().map(Num::as_f64).or_else(|| v.as_str()?.trim().parse().ok())
}

/// The numbers of a list (version 3) or of a `{a, b}` string (version 2); at most four.
fn numbers(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Array(a) if a.items.len() <= 4 => a.items.iter().map(number).collect(),
        Value::String(s) => {
            let inner = s.as_str().trim().trim_start_matches('{').trim_end_matches('}');
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() > 4 {
                return None;
            }
            parts.iter().map(|p| p.trim().parse::<f64>().ok()).collect()
        }
        _ => None,
    }
}

/// The shapes of `layer` with every corner component fitted in; `Ok(None)` when the layer has
/// no corner hint. A corner whose glyph does not exist, or that sits on an extra node, is left
/// out.
pub fn insert_corners(font: &Font, layer: &Layer) -> Result<Option<Vec<Shape>>, CornerError> {
    if !layer.info.hints.iter().any(is_corner) {
        return Ok(None);
    }
    let mut hints = Vec::new();
    for h in layer.info.hints.iter() {
        match corner_hint(h) {
            Ok(Some(c)) => hints.push(c),
            Ok(None) => {}
            Err(message) => {
                let corner = h.value.as_dict().and_then(|d| d.get("name")).and_then(Value::as_str).unwrap_or("").to_string();
                return Err(CornerError { corner, message });
            }
        }
    }
    let mut shapes = layer.shapes.clone();
    apply(font, &layer.master, &mut shapes, &hints)?;
    Ok(Some(shapes))
}

/// `layer` with the corner hints at `indices` (positions in the layer's hints) fitted into its
/// paths and removed. The other corner hints on those paths keep pointing at the nodes they sat
/// on.
pub fn decompose_corners(font: &Font, layer: &Layer, indices: &[usize]) -> Result<Layer, CornerError> {
    let bad = |corner: &str, message: String| CornerError { corner: corner.to_string(), message };
    let mut chosen = Vec::new();
    for &i in indices {
        let hint = layer.info.hints.get(i).ok_or_else(|| bad("", format!("the layer has no hint {i}")))?;
        let corner = corner_hint(hint).map_err(|m| bad("", m))?.ok_or_else(|| bad("", format!("hint {i} is not a corner component")))?;
        if !chosen.iter().any(|(j, _)| *j == i) {
            chosen.push((i, corner));
        }
    }
    let hints: Vec<CornerHint> = chosen.iter().map(|(_, c)| c.clone()).collect();
    let mut out = layer.clone();
    let lengths: Vec<usize> = layer.shapes.iter().map(|s| if let Shape::Path(p) = s { p.nodes.len() } else { 0 }).collect();
    let inserted = apply(font, &layer.master, &mut out.shapes, &hints)?;
    let removed: Vec<usize> = chosen.iter().map(|(i, _)| *i).collect();
    let kept: Vec<Hint> = layer
        .info
        .hints
        .iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, h)| match corner_hint(h) {
            Ok(Some(CornerHint { target: CornerTarget::Node { shape, node }, .. })) => {
                let len = lengths.get(shape).copied().unwrap_or(0);
                let Some(n) = node.checked_rem(len) else { return h.clone() };
                // A corner inserted after node `p` moves every later node; one inserted after the
                // last node of a closed contour lands at its front and moves them all.
                let shift: usize =
                    inserted.iter().filter(|ins| ins.shape == shape && (ins.vertex < n || ins.vertex + 1 == len)).map(|ins| ins.added).sum();
                if shift == 0 { h.clone() } else { with_node(h, n.saturating_add(shift)) }
            }
            _ => h.clone(),
        })
        .collect();
    out.info.hints = kept;
    Ok(out)
}

/// `hint` pointing at node `node` of the same path.
fn with_node(hint: &Hint, node: usize) -> Hint {
    let mut out = hint.clone();
    let Value::Dict(d) = &mut out.value else { return out };
    let Some(origin) = d.get_mut("origin") else { return out };
    match origin {
        Value::Array(a) => {
            if let Some(slot) = a.items.get_mut(1) {
                *slot = Value::Number(Number::int(i64::try_from(node).unwrap_or(i64::MAX)));
            }
        }
        Value::String(s) => {
            let shape = numbers(&Value::String(s.clone())).and_then(|v| v.first().copied()).unwrap_or(0.0);
            *s = Str::new(format!("{{{shape}, {node}}}"));
        }
        _ => {}
    }
    out
}

/// What [`insert_all_corner_components`] did.
#[derive(Debug, Default)]
pub struct CornerOutlines {
    /// Layers whose corners were fitted.
    pub layers: usize,
    /// Layers left as they were, each with the reason.
    pub failed: Vec<String>,
}

/// `font` with the corner components of every layer (not the backgrounds) fitted into its paths
/// and the corner hints removed, as both reference compilers do when they read a source. A layer
/// whose corners cannot be fitted is left as it was and listed in [`CornerOutlines::failed`].
pub fn insert_all_corner_components(font: &Font) -> (Font, CornerOutlines) {
    let mut report = CornerOutlines::default();
    let mut changes: Vec<(usize, usize, Vec<Shape>)> = Vec::new();
    for (gi, glyph) in font.glyphs.iter().enumerate() {
        for (li, layer) in glyph.layers.iter().enumerate() {
            match insert_corners(font, layer) {
                Ok(Some(shapes)) => changes.push((gi, li, shapes)),
                Ok(None) => {}
                Err(e) => report.failed.push(format!("glyph `{}` layer `{}`: {e}", glyph.name, layer.info.name.as_deref().unwrap_or(&layer.id))),
            }
        }
    }
    if changes.is_empty() {
        return (font.clone(), report);
    }
    let mut out = font.clone();
    for (gi, li, shapes) in changes {
        if let Some(mut g) = out.glyphs.get_mut_at(gi)
            && let Some(l) = g.layers.get_mut(li)
        {
            l.shapes = shapes;
            l.info.hints.retain(|h| !is_corner(h));
            report.layers += 1;
        }
    }
    (out, report)
}

/// One corner spliced into a path, in the path's node numbering before any corner was added.
struct Inserted {
    shape: usize,
    vertex: usize,
    added: usize,
}

/// Fits `hints` into `shapes`, earlier paths and nodes first, and says where each went.
fn apply(font: &Font, master: &str, shapes: &mut Vec<Shape>, hints: &[CornerHint]) -> Result<Vec<Inserted>, CornerError> {
    let mut targets: Vec<(usize, usize, &CornerHint)> = hints
        .iter()
        .filter_map(|h| match h.target {
            CornerTarget::Node { shape, node } => Some((shape, node, h)),
            CornerTarget::Intersection { .. } => None,
        })
        .collect();
    targets.sort_by_key(|&(shape, node, _)| (shape, node));
    // Each corner moves the nodes after it, so later corners on the same path add what the
    // earlier ones inserted to their node index (fontc's bookkeeping).
    let lengths: Vec<usize> = shapes.iter().map(|s| if let Shape::Path(p) = s { p.nodes.len() } else { 0 }).collect();
    let mut inserted = Vec::new();
    let mut delta = 0usize;
    let mut current = None;
    let mut work = 0usize;
    for (shape, node, hint) in targets {
        if current != Some(shape) {
            current = Some(shape);
            delta = 0;
        }
        let Some(glyph) = font.glyph(&hint.name) else { continue };
        let err = |message: String| CornerError { corner: hint.name.clone(), message };
        let component = CornerComponent::new(glyph, master, hint.scale).map_err(err)?;
        let added = fit(shapes, component, hint, shape, node, delta, &mut work).map_err(err)?;
        let vertex = lengths.get(shape).and_then(|&len| node.checked_rem(len)).unwrap_or(node);
        inserted.push(Inserted { shape, vertex, added });
        delta = delta.saturating_add(added);
    }
    Ok(inserted)
}

/// Splices one corner into path `shape` at node `node + delta`; returns how many nodes it added.
fn fit(
    shapes: &mut Vec<Shape>,
    mut component: CornerComponent,
    hint: &CornerHint,
    shape: usize,
    node: usize,
    delta: usize,
    work: &mut usize,
) -> Result<usize, String> {
    let flipped = hint.is_flipped();
    let Some(Shape::Path(path)) = shapes.get_mut(shape) else {
        return Err(format!("there is no path at shape index {shape}"));
    };
    let len = path.nodes.len();
    if len < 2 {
        return Err(format!("the path at shape index {shape} has fewer than two nodes"));
    }
    let point_idx = node.checked_add(delta).ok_or("the node index is too large")? % len;
    component.apply_transform(Affine::scale_non_uniform(component.scale.x, component.scale.y));
    let state = component.align_to_main_path(&path.nodes, hint, point_idx)?;
    let mut instroke_pt = state.instroke_pt;
    let original_outstroke = next_segment(&path.nodes, point_idx).ok_or("the node has no outgoing segment")?;
    if hint.alignment != CornerAlignment::Right && state.correction {
        instroke_pt = component.recompute_instroke_intersection_point(&path.nodes, point_idx).unwrap_or(instroke_pt);
        if !matches!(next_segment(&component.corner_path.nodes, 0), Some(PathSeg::Line(_))) {
            component.stretch_first_seg_to_fit(instroke_pt);
        }
    }
    split_instroke(&mut path.nodes, point_idx, instroke_pt).ok_or("the node has no incoming segment")?;

    let new_nodes: Vec<Node> = component.corner_path.nodes.iter().skip(1).cloned().map(round_node).collect();
    let added = new_nodes.len();
    if len.saturating_add(added) > MAX_OUTLINE_ELEMENTS {
        return Err(format!("the path with its corners has more than {MAX_OUTLINE_ELEMENTS} nodes"));
    }
    *work = work.saturating_add(len).saturating_add(added);
    if *work > MAX_CORNER_WORK {
        return Err("the layer's corners would take too much work to fit".into());
    }
    let insert_pt = next_idx(len, point_idx);
    if insert_pt > path.nodes.len() {
        return Err("the node index is out of range".into());
    }
    path.nodes.splice(insert_pt..insert_pt, new_nodes);
    // The last inserted node starts the outgoing stroke.
    let new_outstroke_idx = prev_idx(path.nodes.len(), insert_pt + added);
    if let Some(p) = component.recompute_outstroke_intersection_point(original_outstroke, flipped) {
        fixup_outstroke(&mut path.nodes, original_outstroke, p, new_outstroke_idx);
        // With an exit anchor the outgoing stroke is cut at the anchor and the end node stays.
        if component.exit.is_some()
            && let (Some(end), Some(n)) = (component.corner_path.nodes.last(), path.nodes.get_mut(new_outstroke_idx))
        {
            (n.x, n.y) = (ot_round(end.x), ot_round(end.y));
        }
    }
    if !path.nodes.iter().all(|n| in_range(n.x) && in_range(n.y)) {
        return Err("the fitted corner is not a number or lies beyond the coordinate limit".into());
    }
    for mut other in component.other_paths {
        other.nodes = other.nodes.into_iter().map(round_node).collect();
        if !other.nodes.iter().all(|n| in_range(n.x) && in_range(n.y)) {
            return Err("a closed path of the corner lies beyond the coordinate limit".into());
        }
        shapes.push(Shape::Path(other));
    }
    Ok(added)
}

/// Rounds half up, as fontc's `OtRound` does.
fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

fn round_node(mut n: Node) -> Node {
    (n.x, n.y) = (ot_round(n.x), ot_round(n.y));
    n
}

fn prev_idx(len: usize, idx: usize) -> usize {
    if idx == 0 { len.saturating_sub(1) } else { idx - 1 }
}

fn next_idx(len: usize, idx: usize) -> usize {
    (idx + 1).checked_rem(len).unwrap_or(0)
}

fn point_at(nodes: &[Node], idx: usize) -> Option<Point> {
    nodes.get(idx).map(Node::point)
}

/// The segment ending at node `idx`, wrapping round the node list.
fn previous_segment(nodes: &[Node], idx: usize) -> Option<PathSeg> {
    let len = nodes.len();
    if len < 2 {
        return None;
    }
    let end = point_at(nodes, idx)?;
    let mut i = prev_idx(len, idx);
    let prev = nodes.get(i)?;
    if prev.kind == NodeKind::OffCurve {
        i = prev_idx(len, i);
        let c1 = nodes.get(i)?;
        if c1.kind != NodeKind::OffCurve {
            return Some(PathSeg::Quad(QuadBez::new(c1.point(), prev.point(), end)));
        }
        let start = point_at(nodes, prev_idx(len, i))?;
        Some(PathSeg::Cubic(CubicBez::new(start, c1.point(), prev.point(), end)))
    } else {
        Some(PathSeg::Line(Line::new(prev.point(), end)))
    }
}

/// The segment starting at node `idx`, wrapping round the node list.
fn next_segment(nodes: &[Node], idx: usize) -> Option<PathSeg> {
    let len = nodes.len();
    if len < 2 {
        return None;
    }
    let start = point_at(nodes, idx)?;
    let mut i = next_idx(len, idx);
    let next = nodes.get(i)?;
    if next.kind == NodeKind::OffCurve {
        i = next_idx(len, i);
        let c2 = nodes.get(i)?;
        if c2.kind != NodeKind::OffCurve {
            return Some(PathSeg::Quad(QuadBez::new(start, next.point(), c2.point())));
        }
        let end = point_at(nodes, next_idx(len, i))?;
        Some(PathSeg::Cubic(CubicBez::new(start, next.point(), c2.point(), end)))
    } else {
        Some(PathSeg::Line(Line::new(start, next.point())))
    }
}

/// Whether a segment has no length: every point the same.
fn degenerate(seg: PathSeg) -> bool {
    match seg {
        PathSeg::Line(l) => l.p0 == l.p1,
        PathSeg::Quad(q) => q.p0 == q.p1 && q.p1 == q.p2,
        PathSeg::Cubic(c) => c.p0 == c.p1 && c.p1 == c.p2 && c.p2 == c.p3,
    }
}

fn set_point(nodes: &mut [Node], idx: usize, p: Point) {
    if let Some(n) = nodes.get_mut(idx) {
        (n.x, n.y) = (ot_round(p.x), ot_round(p.y));
    }
}

/// Cuts the segment ending at `point_idx` where it is nearest to `intersection`.
fn split_instroke(nodes: &mut [Node], point_idx: usize, intersection: Point) -> Option<()> {
    let len = nodes.len();
    let instroke = previous_segment(nodes, point_idx)?;
    let t = instroke.nearest(intersection, 1e-6).t;
    match instroke.subsegment(0.0..t) {
        PathSeg::Line(l) => set_point(nodes, point_idx, l.p1),
        PathSeg::Quad(q) => {
            set_point(nodes, point_idx, q.p2);
            set_point(nodes, prev_idx(len, point_idx), q.p1);
        }
        PathSeg::Cubic(c) => {
            set_point(nodes, point_idx, c.p3);
            let i = prev_idx(len, point_idx);
            set_point(nodes, i, c.p2);
            set_point(nodes, prev_idx(len, i), c.p1);
        }
    }
    Some(())
}

/// Makes the outgoing segment start at `point_idx` with the part of `original` beyond the point
/// nearest to `intersection`.
fn fixup_outstroke(nodes: &mut [Node], original: PathSeg, intersection: Point, point_idx: usize) {
    let len = nodes.len();
    let t = original.nearest(intersection, 1e-6).t;
    match original.subsegment(t..1.0) {
        PathSeg::Line(l) => set_point(nodes, point_idx, l.p0),
        PathSeg::Quad(q) => {
            set_point(nodes, point_idx, q.p0);
            set_point(nodes, next_idx(len, point_idx), q.p1);
        }
        PathSeg::Cubic(c) => {
            set_point(nodes, point_idx, c.p0);
            let i = next_idx(len, point_idx);
            set_point(nodes, i, c.p1);
            set_point(nodes, next_idx(len, i), c.p2);
        }
    }
}

/// The corner glyph's paths for one fitting.
struct CornerComponent {
    /// The open path spliced into the host path.
    corner_path: Path,
    /// The other paths, added to the layer.
    other_paths: Vec<Path>,
    /// The entry and exit anchors, moved with the paths.
    entry: Option<Point>,
    exit: Option<Point>,
    /// The scale left to apply geometrically after intermediate layers took their share.
    scale: Vec2,
}

impl CornerComponent {
    /// The corner glyph's layer for `master`, moved so its origin (the `origin` anchor, or the
    /// layer origin) is at (0, 0), with intermediate layers interpolated for `scale`.
    fn new(glyph: &Glyph, master: &str, scale: Vec2) -> Result<Self, String> {
        let layer = glyph.layer(master).ok_or_else(|| format!("the corner glyph has no layer for master `{master}`"))?;
        let anchor = |name: &str| layer.anchors.iter().find(|a| a.name == name).map(|a| Point::new(a.x, a.y));
        let origin = anchor("origin").unwrap_or_default().to_vec2();
        let (paths, scale) = intermediate_paths(glyph, layer, scale);
        let mut paths = paths.into_iter().map(|mut p| {
            for n in &mut p.nodes {
                (n.x, n.y) = (n.x - origin.x, n.y - origin.y);
            }
            p
        });
        let corner_path = paths.next().ok_or("the corner glyph has no paths")?;
        let (Some(first), Some(second)) = (corner_path.nodes.first(), corner_path.nodes.get(1)) else {
            return Err("the corner glyph's path has fewer than two nodes".into());
        };
        // The path faces right when it leaves its start node to the right; the anchor names swap.
        let faces_right = second.x > first.x;
        let (entry_name, exit_name) = if faces_right { ("right", "left") } else { ("left", "right") };
        let entry = anchor(entry_name).map(|p| p - origin);
        let exit = anchor(exit_name).map(|p| p - origin);
        Ok(Self { corner_path, other_paths: paths.collect(), entry, exit, scale })
    }

    fn apply_transform(&mut self, transform: Affine) {
        for node in self.corner_path.nodes.iter_mut().chain(self.other_paths.iter_mut().flat_map(|p| p.nodes.iter_mut())) {
            let p = transform * node.point();
            (node.x, node.y) = (p.x, p.y);
        }
        self.entry = self.entry.map(|p| transform * p);
        self.exit = self.exit.map(|p| transform * p);
    }

    fn first_point(&self) -> Point {
        self.corner_path.nodes.first().map(Node::point).unwrap_or_default()
    }

    fn last_point(&self) -> Point {
        self.corner_path.nodes.last().map(Node::point).unwrap_or_default()
    }

    /// Where the corner ends: its exit anchor, or the end node.
    fn end_point(&self) -> Point {
        self.exit.unwrap_or_else(|| self.last_point())
    }

    /// Reverses the corner path. Each on-curve node takes the kind of the segment that now ends
    /// at it.
    fn reverse_corner_path(&mut self) {
        let nodes = &mut self.corner_path.nodes;
        nodes.reverse();
        let mut prev_off = false;
        for (i, n) in nodes.iter_mut().enumerate() {
            if n.kind != NodeKind::OffCurve && i > 0 {
                n.kind = match (prev_off, n.kind) {
                    (true, NodeKind::Line) => NodeKind::Curve,
                    (false, NodeKind::Curve | NodeKind::QCurve) => NodeKind::Line,
                    (_, kind) => kind,
                };
            }
            prev_off = n.kind == NodeKind::OffCurve;
        }
    }

    /// Rotates the corner by the hint's alignment and places its origin on the node; with an
    /// entry anchor, then slides it along its baseline until the anchor is on the incoming
    /// stroke.
    fn align_to_main_path(&mut self, nodes: &[Node], hint: &CornerHint, point_idx: usize) -> Result<AlignmentState, String> {
        let flipped = hint.is_flipped();
        let end = if flipped { self.last_point() } else { self.end_point() };
        let mut angle = (-end.y).atan2(end.x);
        if flipped {
            angle += std::f64::consts::FRAC_PI_2;
            self.reverse_corner_path();
            // The anchors are not used on a mirrored corner.
            (self.entry, self.exit) = (None, None);
        }
        let instroke = previous_segment(nodes, point_idx).ok_or("the node has no incoming segment")?;
        let outstroke = next_segment(nodes, point_idx).ok_or("the node has no outgoing segment")?;
        if degenerate(instroke) || degenerate(outstroke) {
            return Err("the corner sits next to a segment of zero length".into());
        }
        let target = point_at(nodes, point_idx).ok_or("the node does not exist")?;

        let distance = if flipped { self.last_point().y } else { self.end_point().x };
        let outstroke_t = outstroke.inv_arclen(distance.abs(), 1e-6);
        let outstroke_pt = outstroke.eval(outstroke_t);
        let outstroke_angle = (outstroke_pt - target).angle();

        let distance = if flipped { -self.first_point().x } else { self.first_point().y };
        let instroke_t = instroke.inv_arclen(distance.abs(), 1e-6);
        let instroke_pt = instroke.reverse().eval(instroke_t);
        let instroke_angle = (target - instroke_pt).angle() + std::f64::consts::FRAC_PI_2;

        let correction = !(py_is_close(instroke_t, 0.0) || py_is_close(instroke_t, 1.0));
        angle += match hint.alignment {
            CornerAlignment::Left => outstroke_angle,
            CornerAlignment::Right => instroke_angle,
            CornerAlignment::Both => (instroke_angle + outstroke_angle) / 2.0,
            CornerAlignment::Unused | CornerAlignment::Unaligned => 0.0,
        };
        self.apply_transform(Affine::translate(target.to_vec2()).pre_rotate(angle));

        if let Some(entry) = self.entry {
            let along = Vec2::from_angle(angle);
            if let Some(hit) = unbounded_seg_seg_intersection(PathSeg::Line(Line::new(entry, entry + along)), instroke) {
                let shift = along * (hit - entry).dot(along);
                if shift.x.is_finite() && shift.y.is_finite() {
                    self.apply_transform(Affine::translate(shift));
                }
            }
        }
        Ok(AlignmentState { instroke_pt, correction })
    }

    /// Where the corner's first segment, taken as a line, crosses the incoming stroke.
    fn recompute_instroke_intersection_point(&self, nodes: &[Node], point_idx: usize) -> Option<Point> {
        let (a, b) = (self.corner_path.nodes.first()?, self.corner_path.nodes.get(1)?);
        let instroke = previous_segment(nodes, point_idx)?;
        unbounded_seg_seg_intersection(PathSeg::Line(Line::new(a.point(), b.point())), instroke)
    }

    /// Where the outgoing stroke starts again after the corner.
    fn recompute_outstroke_intersection_point(&self, original_outstroke: PathSeg, flipped: bool) -> Option<Point> {
        if flipped {
            let last = self.corner_path.nodes.len().checked_sub(1)?;
            unbounded_seg_seg_intersection(previous_segment(&self.corner_path.nodes, last)?, original_outstroke)
        } else {
            let nearest = original_outstroke.nearest(self.end_point(), 1e-6);
            Some(original_outstroke.eval(nearest.t))
        }
    }

    /// Moves the first off-curve node by what the start node would move to reach the stroke.
    fn stretch_first_seg_to_fit(&mut self, intersection: Point) {
        let delta = intersection - self.first_point();
        if let Some(n) = self.corner_path.nodes.get_mut(1) {
            (n.x, n.y) = (n.x + delta.x, n.y + delta.y);
        }
    }
}

struct AlignmentState {
    instroke_pt: Point,
    correction: bool,
}

/// Where two segments meet when lines among them are taken as unbounded.
fn unbounded_seg_seg_intersection(seg1: PathSeg, seg2: PathSeg) -> Option<Point> {
    match (seg1, seg2) {
        (PathSeg::Line(one), PathSeg::Line(two)) => one.crossing_point(two),
        (seg, PathSeg::Line(line)) | (PathSeg::Line(line), seg) => {
            // Far enough for any outline within the coordinate limit.
            const LITERALLY_UNBOUNDED: f64 = 1e9;
            if line.p0 == line.p1 {
                return None;
            }
            let direction = (line.p1 - line.p0).normalize();
            let extended = Line::new(line.p0 - direction * LITERALLY_UNBOUNDED, line.p1 + direction * LITERALLY_UNBOUNDED);
            seg.intersect_line(extended).first().map(|hit| seg.eval(hit.segment_t))
        }
        _ => None,
    }
}

/// Python's `math.isclose` with its default tolerances.
fn py_is_close(a: f64, b: f64) -> bool {
    const REL_TOL: f64 = 1e-09;
    (a - b).abs() <= REL_TOL * a.abs().max(b.abs())
}

/// The width and height in percent an intermediate corner layer's name gives
/// (`Width 150 Height 100`); a missing one is 100.
fn size_percent(name: &str) -> Option<(f64, f64)> {
    let lower = name.to_ascii_lowercase();
    let words: Vec<&str> = lower.split(|c: char| c.is_whitespace() || matches!(c, ':' | ',' | '%' | '=')).filter(|w| !w.is_empty()).collect();
    let (mut w, mut h) = (None, None);
    for pair in words.windows(2) {
        let &[key, value] = pair else { continue };
        let n = value.parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0 && *n <= MAX_CORNER_SCALE * 100.0);
        match key {
            "width" => w = w.or(n),
            "height" => h = h.or(n),
            _ => {}
        }
    }
    if w.is_none() && h.is_none() {
        return None;
    }
    Some((w.unwrap_or(100.0), h.unwrap_or(100.0)))
}

fn layer_paths(layer: &Layer) -> Vec<&Path> {
    layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).collect()
}

/// The corner glyph's paths for `scale`: on each axis where intermediate layers of the master
/// vary the size (the other axis at 100 %), the outline is interpolated piecewise linearly
/// between the master layer (100 %) and those layers, extrapolated beyond them, and that axis is
/// left with only the scale's sign to apply. Returns the paths and the scale still to apply.
fn intermediate_paths(glyph: &Glyph, layer: &Layer, scale: Vec2) -> (Vec<Path>, Vec2) {
    let base: Vec<&Path> = layer_paths(layer);
    let mut out: Vec<Path> = base.iter().map(|p| (*p).clone()).collect();
    let compatible = |other: &[&Path]| other.len() == base.len() && other.iter().zip(&base).all(|(a, b)| a.nodes.len() == b.nodes.len());
    let sizes: Vec<(f64, f64, Vec<&Path>)> = glyph
        .layers
        .iter()
        .filter(|l| l.master == layer.master && !l.is_master_layer())
        .filter_map(|l| {
            let (w, h) = size_percent(l.info.name.as_deref()?)?;
            let paths = layer_paths(l);
            compatible(&paths).then_some((w, h, paths))
        })
        .collect();
    if sizes.is_empty() {
        return (out, scale);
    }
    let mut remaining = scale;
    for horizontal in [true, false] {
        let factor = if horizontal { scale.x } else { scale.y };
        let target = factor.abs() * 100.0;
        // (percent, paths) stops on this axis, the master layer at 100 %.
        let mut stops: Vec<(f64, &[&Path])> = sizes
            .iter()
            .filter(|(w, h, _)| if horizontal { *h == 100.0 && *w != 100.0 } else { *w == 100.0 && *h != 100.0 })
            .map(|(w, h, p)| (if horizontal { *w } else { *h }, p.as_slice()))
            .collect();
        if stops.is_empty() || target == 100.0 {
            continue;
        }
        stops.push((100.0, base.as_slice()));
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        stops.dedup_by(|a, b| a.0 == b.0);
        // The pair of stops around the target, or the nearest pair when it is beyond them.
        let i = stops.iter().rposition(|s| s.0 <= target).unwrap_or(0).min(stops.len().saturating_sub(2));
        let (Some(&(a, pa)), Some(&(b, pb))) = (stops.get(i), stops.get(i + 1)) else { continue };
        let f = (target - a) / (b - a);
        if !f.is_finite() {
            continue;
        }
        for (k, path) in out.iter_mut().enumerate() {
            let (Some(from), Some(to), Some(orig)) = (pa.get(k), pb.get(k), base.get(k)) else { continue };
            for (j, n) in path.nodes.iter_mut().enumerate() {
                let (Some(p0), Some(p1), Some(o)) = (from.nodes.get(j), to.nodes.get(j), orig.nodes.get(j)) else { continue };
                n.x += p0.x + (p1.x - p0.x) * f - o.x;
                n.y += p0.y + (p1.y - p0.y) * f - o.y;
            }
        }
        let sign = if factor < 0.0 { -1.0 } else { 1.0 };
        if horizontal {
            remaining.x = sign;
        } else {
            remaining.y = sign;
        }
    }
    (out, remaining)
}

#[cfg(test)]
#[path = "corner_tests.rs"]
mod tests;
