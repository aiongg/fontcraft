//! The Master Compatibility view's model (`view.showMasterCompatibility`): every source layer of the edited glyph,
//! placed beside it, with its shapes, its segments coloured by how they match the rest of their group, and the lines
//! that join corresponding start nodes, component centres and anchors.
//!
//! Layers are compared as `fontcraft_interp::compat` compares them: the shapes are the paths and components (images
//! left out), matched by index within a group of layers. The main sources (master layers in master order, then
//! intermediate layers) form group 0; each alternate set is a group of its own, drawn after a wider gap. Each layer
//! starts where the one before it ends plus a gap and sits a little higher, so the joining lines run on a slant. The
//! edited layer stays at (0, 0). Everything is in the edited layer's font units.

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::interp::{ANGLE_WARNING_DEGREES, Designspace, GlyphSources};
use fontcraft_engine::model::{Component, Font, Glyph, Layer, Path, Shape};
use fontcraft_geom::{Affine, BezPath, PathSeg, Piece, Point, segments};

use super::ProjectCx;

/// Most layers the view draws.
pub const MAX_PLACED: usize = 64;
/// Most shapes per layer and anchors per group the view joins with lines.
pub const MAX_LINKED: usize = 1_024;
/// The gap after each layer, the rise from one layer to the next and the extra gap before an alternate set, as
/// shares of the em.
const GAP_EM: f64 = 0.25;
const RISE_EM: f64 = 0.08;
const GROUP_GAP_EM: f64 = 0.75;

/// What the view draws; empty while it is off or nothing is edited.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CompatibilityModel {
    pub layers: Vec<PlacedLayer>,
    pub links: Vec<Link>,
}

/// One layer drawn beside the edited one.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PlacedLayer {
    /// The edited layer: its shapes show their index.
    pub current: bool,
    pub shapes: Vec<PlacedShape>,
    /// Each path segment with its state against the rest of the group; none when the group has one layer.
    pub segments: Vec<PlacedSegment>,
}

/// One path segment of a placed layer and how it matches.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PlacedSegment {
    pub state: SegmentState,
    #[plain_data(assume = "a kurbo path: owned points")]
    pub path: BezPath,
}

/// One outline shape of a placed layer, the layer's offset applied.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PlacedShape {
    /// The shape's index among the layer's outline shapes.
    pub index: usize,
    pub component: bool,
    #[plain_data(assume = "a kurbo path: owned points")]
    pub outline: BezPath,
    /// Where the index is written: the start node or the component's centre.
    pub label: Option<[f64; 2]>,
}

/// How a segment matches its counterparts.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SegmentState {
    #[default]
    Compatible,
    /// Its chord turns by more than the warning angle against a layer of its group.
    Turns,
    /// It has no counterpart, or its nodes differ from that point on.
    Incompatible,
}

/// What a line joins.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkKind {
    /// A path's start node.
    #[default]
    Start,
    /// A component's centre.
    Centre,
    Anchor,
}

/// A line across the layers of one group.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub kind: LinkKind,
    /// One point per layer of the group; a layer without the element gives its (0, 0).
    pub points: Vec<[f64; 2]>,
    /// The component base names or anchor names that differ within the group, with where they are written.
    pub mismatches: Vec<([f64; 2], String)>,
}

pub fn project(model: &mut Tracked<CompatibilityModel>, cx: &mut ProjectCx<'_>) {
    let value = if cx.view.display.master_compatibility && !cx.view.edit_background { compute(cx).unwrap_or_default() } else { Default::default() };
    super::set(model, value);
}

fn compute(cx: &ProjectCx<'_>) -> Option<CompatibilityModel> {
    let font = cx.session.font().ok()?;
    let glyph = font.glyph(crate::canvas::tools::edited_glyph(font, cx.view)?)?;
    let current = crate::shell::masters::shown_layer(font, &cx.view.layers, glyph)?;
    Some(model_of(font, glyph, current))
}

/// The view of `glyph` with `current` as the edited layer.
pub fn model_of(font: &Font, glyph: &Glyph, current: &Layer) -> CompatibilityModel {
    let sources = Designspace::new(font).ok().and_then(|ds| GlyphSources::of(font, &ds, &glyph.name).ok());
    let placed = placement(font, glyph, current, sources.as_ref());
    let layers: Vec<&Layer> =
        placed.iter().filter_map(|p| glyph.layers.iter().find(|l| l.id == p.layer).or((p.layer == current.id).then_some(current))).collect();
    if layers.len() != placed.len() {
        return CompatibilityModel::default();
    }
    let mut out = CompatibilityModel::default();
    for (p, layer) in placed.iter().zip(&layers) {
        let others: Vec<&Layer> = placed.iter().zip(&layers).filter(|(q, l)| q.group == p.group && l.id != layer.id).map(|(_, l)| *l).collect();
        out.layers.push(place_layer(font, layer, p.offset, &others, layer.id == current.id));
    }
    out.links = links(font, &placed, &layers);
    out
}

/// Where a layer is drawn.
struct Placed {
    layer: String,
    /// Relative to the edited layer, in font units.
    offset: (f64, f64),
    /// 0 for the main sources, then one group per alternate set.
    group: usize,
}

/// The layers drawn for `glyph`, in order, `current` at (0, 0). Without sources (no usable designspace) the master
/// layers are drawn, in master order. A current layer that is no source (a backup) is drawn first in the main group.
fn placement(font: &Font, glyph: &Glyph, current: &Layer, sources: Option<&GlyphSources>) -> Vec<Placed> {
    let mut order: Vec<(String, usize)> = Vec::new();
    let push = |id: &str, group: usize, order: &mut Vec<(String, usize)>| {
        if order.len() < MAX_PLACED && !order.iter().any(|(l, _)| l == id) && glyph.layers.iter().any(|l| l.id == id) {
            order.push((id.to_string(), group));
        }
    };
    match sources {
        Some(s) => {
            for src in &s.main {
                push(&src.layer_id, 0, &mut order);
            }
            for (k, set) in s.alternates.iter().enumerate() {
                for src in &set.sources {
                    push(&src.layer_id, k + 1, &mut order);
                }
            }
        }
        None => {
            for m in font.masters.iter() {
                if let Some(l) = glyph.layer(&m.id) {
                    push(&l.id, 0, &mut order);
                }
            }
        }
    }
    if !order.iter().any(|(l, _)| *l == current.id) {
        order.insert(0, (current.id.clone(), 0));
        order.truncate(MAX_PLACED);
    }
    let em = f64::from(font.units_per_em.max(1));
    let mut x = 0.0;
    let mut placed: Vec<Placed> = Vec::with_capacity(order.len());
    for (k, (id, group)) in order.into_iter().enumerate() {
        if placed.last().is_some_and(|p| p.group != group) {
            x += GROUP_GAP_EM * em;
        }
        let width = glyph.layers.iter().find(|l| l.id == id).map_or(current.width, |l| l.width);
        placed.push(Placed { layer: id, offset: (x, k as f64 * RISE_EM * em), group });
        x += if width.is_finite() { width.max(0.0) } else { 0.0 } + GAP_EM * em;
    }
    if let Some((cx, cy)) = placed.iter().find(|p| p.layer == current.id).map(|p| p.offset) {
        for p in &mut placed {
            p.offset = (p.offset.0 - cx, p.offset.1 - cy);
        }
    }
    placed
}

/// The layer's outline shapes (paths and components; images left out), in order.
fn outline_shapes(layer: &Layer) -> Vec<&Shape> {
    layer.shapes.iter().filter(|s| matches!(s, Shape::Path(_) | Shape::Component(_))).take(MAX_LINKED).collect()
}

fn place_layer(font: &Font, layer: &Layer, offset: (f64, f64), others: &[&Layer], current: bool) -> PlacedLayer {
    let shift = Affine::translate(offset);
    let mut shapes = Vec::new();
    for (index, shape) in outline_shapes(layer).into_iter().enumerate() {
        let (outline, label) = match shape {
            Shape::Path(p) => (p.to_bez().ok(), p.nodes.iter().find(|n| n.on_curve()).map(|n| n.point())),
            Shape::Component(c) => (font.component_outline(c, &layer.master).ok(), Some(centre(font, c, &layer.master))),
            _ => (None, None),
        };
        let Some(outline) = outline else { continue };
        let label = label.filter(|_| current).map(|p| shift * p).map(|p| [p.x, p.y]);
        shapes.push(PlacedShape { index, component: matches!(shape, Shape::Component(_)), outline: shift * outline, label });
    }
    let segments = segment_states(layer, others).into_iter().map(|(state, path)| PlacedSegment { state, path: shift * path }).collect();
    PlacedLayer { current, shapes, segments }
}

/// The centre of a component's outline, or its offset when it has none.
fn centre(font: &Font, c: &Component, master: &str) -> Point {
    use fontcraft_geom::Shape as _;
    let fallback = c.affine() * Point::ZERO;
    match font.component_outline(c, master) {
        Ok(path) if !path.elements().is_empty() => {
            let centre = path.bounding_box().center();
            if centre.x.is_finite() && centre.y.is_finite() { centre } else { fallback }
        }
        _ => fallback,
    }
}

/// Each segment of each path of `layer` with its state against `others` (the rest of its group). Nothing without
/// other layers.
fn segment_states(layer: &Layer, others: &[&Layer]) -> Vec<(SegmentState, BezPath)> {
    let mut out = Vec::new();
    if others.is_empty() {
        return out;
    }
    let other_shapes: Vec<Vec<&Shape>> = others.iter().map(|l| outline_shapes(l)).collect();
    for (o, shape) in outline_shapes(layer).into_iter().enumerate() {
        let Shape::Path(p) = shape else { continue };
        for seg in segments(&p.node_points(), p.closed) {
            let mut state = SegmentState::Compatible;
            for shapes in &other_shapes {
                let s = match shapes.get(o) {
                    Some(Shape::Path(q)) => compare(p, q, &seg),
                    _ => SegmentState::Incompatible,
                };
                state = state.max(s);
            }
            out.push((state, piece_path(&seg.pieces)));
        }
    }
    out
}

/// One segment of `p` against path `q`.
fn compare(p: &Path, q: &Path, seg: &fontcraft_geom::Segment) -> SegmentState {
    let first_diff = p
        .nodes
        .iter()
        .zip(&q.nodes)
        .position(|(m, n)| m.kind != n.kind)
        .or_else(|| if p.closed != q.closed { Some(0) } else { (p.nodes.len() != q.nodes.len()).then(|| p.nodes.len().min(q.nodes.len())) });
    let late = |i: usize| first_diff.is_some_and(|d| i >= d);
    if late(seg.end) || seg.handles.iter().any(|&h| late(h)) {
        return SegmentState::Incompatible;
    }
    let chord = |path: &Path| {
        let (a, b) = (path.nodes.get(seg.start)?, path.nodes.get(seg.end)?);
        Some((b.x - a.x, b.y - a.y))
    };
    match (chord(p), chord(q)) {
        (Some(u), Some(v)) if turn(u, v).is_some_and(|d| d > ANGLE_WARNING_DEGREES) => SegmentState::Turns,
        _ => SegmentState::Compatible,
    }
}

fn piece_path(pieces: &[Piece]) -> BezPath {
    let mut path = BezPath::new();
    for (k, piece) in pieces.iter().enumerate() {
        let seg = match piece {
            Piece::Line(l) => PathSeg::Line(*l),
            Piece::Quad(q) => PathSeg::Quad(*q),
            Piece::Cubic(c) => PathSeg::Cubic(*c),
        };
        if k == 0 {
            path.move_to(piece.eval(0.0));
        }
        match seg {
            PathSeg::Line(l) => path.line_to(l.p1),
            PathSeg::Quad(q) => path.quad_to(q.p1, q.p2),
            PathSeg::Cubic(c) => path.curve_to(c.p1, c.p2, c.p3),
        }
    }
    path
}

/// The angle in degrees (0 to 180) between two directions; `None` when either has no length.
fn turn(u: (f64, f64), v: (f64, f64)) -> Option<f64> {
    let usable = |(x, y): (f64, f64)| x.is_finite() && y.is_finite() && x.hypot(y) > 1e-9;
    if !usable(u) || !usable(v) {
        return None;
    }
    let mut d = (v.1.atan2(v.0) - u.1.atan2(u.0)).to_degrees();
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    Some(d.abs())
}

/// The lines of every group: start nodes, component centres and anchors.
fn links(font: &Font, placed: &[Placed], layers: &[&Layer]) -> Vec<Link> {
    let mut out = Vec::new();
    let mut groups: Vec<usize> = placed.iter().map(|p| p.group).collect();
    groups.dedup();
    for group in groups {
        let members: Vec<(usize, &Layer)> =
            placed.iter().zip(layers).enumerate().filter(|(_, (p, _))| p.group == group).map(|(i, (_, l))| (i, *l)).collect();
        let shift = |i: usize, p: Point| placed.get(i).map_or([p.x, p.y], |pl| [p.x + pl.offset.0, p.y + pl.offset.1]);
        let shapes: Vec<Vec<&Shape>> = members.iter().map(|(_, l)| outline_shapes(l)).collect();
        let count = shapes.iter().map(Vec::len).max().unwrap_or(0);
        for s in 0..count {
            let at_s: Vec<Option<&&Shape>> = shapes.iter().map(|list| list.get(s)).collect();
            let component = at_s.iter().flatten().next().is_some_and(|x| matches!(x, Shape::Component(_)));
            let bases: Vec<Option<&str>> = at_s
                .iter()
                .map(|x| match x {
                    Some(Shape::Component(c)) => Some(c.glyph.as_str()),
                    _ => None,
                })
                .collect();
            let disagree = component && bases.windows(2).any(|w| matches!(w, [a, b] if a != b));
            let mut link = Link { kind: if component { LinkKind::Centre } else { LinkKind::Start }, ..Link::default() };
            for (k, (i, layer)) in members.iter().enumerate() {
                let at = match at_s.get(k).copied().flatten() {
                    Some(Shape::Path(p)) => p.nodes.iter().find(|n| n.on_curve()).map(|n| n.point()),
                    Some(Shape::Component(c)) => Some(centre(font, c, &layer.master)),
                    _ => None,
                };
                let point = shift(*i, at.unwrap_or(Point::ZERO));
                link.points.push(point);
                if disagree && let Some(base) = bases.get(k).copied().flatten() {
                    link.mismatches.push((point, base.to_string()));
                }
            }
            out.push(link);
        }
        let mut names: Vec<&str> = Vec::new();
        for (_, layer) in &members {
            for a in &layer.anchors {
                if names.len() < MAX_LINKED && !names.contains(&a.name.as_str()) {
                    names.push(&a.name);
                }
            }
        }
        for name in names {
            let found: Vec<Option<Point>> =
                members.iter().map(|(_, l)| l.anchors.iter().find(|a| a.name == name).map(|a| Point::new(a.x, a.y))).collect();
            let disagree = found.iter().any(Option::is_none);
            let mut link = Link { kind: LinkKind::Anchor, ..Link::default() };
            for ((i, _), at) in members.iter().zip(&found) {
                let point = shift(*i, at.unwrap_or(Point::ZERO));
                link.points.push(point);
                if disagree && at.is_some() {
                    link.mismatches.push((point, name.to_string()));
                }
            }
            out.push(link);
        }
    }
    out
}
