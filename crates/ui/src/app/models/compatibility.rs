//! The Master Compatibility view's model (`view.showMasterCompatibility`): every source layer of the edited glyph,
//! placed beside it, with its shapes, its segments coloured by how they match the rest of their group, and the lines
//! that join corresponding start nodes, component centres and anchors.
//!
//! Layers are compared as `fontcraft_interp::compat` compares them: the shapes are the paths and components (images
//! left out), matched by index within a group of layers. The main sources (master layers in master order, then
//! intermediate layers) form group 0; each alternate set is a group of its own, drawn after a wider gap. Each layer
//! starts where the one before it ends plus a gap and sits a little higher, so the joining lines run on a slant. The
//! edited layer stays at (0, 0). Everything is in the edited layer's font units.
//!
//! The selection and the pointer move without the models being projected again: [`project_probe`] keeps the lines of the
//! selected nodes, [`project_pointer`] the dragged line end and the difference under the pointer.

use std::collections::BTreeSet;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::interp::labels::layer_title;
use fontcraft_engine::interp::{ANGLE_WARNING_DEGREES, Designspace, GlyphSources};
use fontcraft_engine::model::{Component, Font, Glyph, Layer, Path, Shape};
use fontcraft_geom::{Affine, BezPath, ParamCurveNearest, PathSeg, Piece, Point, segments};
use fontcraft_tools::Selection;

use super::incompatibility::Bar;
use super::{ProjectCx, Stamp};

/// Most layers the view draws.
pub const MAX_PLACED: usize = 64;
/// Most shapes per layer and anchors per group the view joins with lines.
pub const MAX_LINKED: usize = 1_024;
/// Most lines from one layer to the next the view draws, and as many for the selected nodes.
pub const MAX_LINES: usize = 4_000;
/// Most segments whose difference is spelled out.
const MAX_NOTES: usize = 4_000;
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
    /// The lines of the selected nodes.
    pub probe: Vec<Link>,
    /// The line end being dragged.
    pub drag: Option<EndDrag>,
    /// The difference under the pointer; empty when there is none.
    pub under: String,
    /// The bars over the text's glyphs that cannot interpolate, whether or not the view is on.
    pub bars: Vec<Bar>,
    /// Why the edited glyph, or the one selected in Font View, cannot interpolate: the status line's text while it has
    /// no other. Empty when it can.
    pub reason: String,
}

impl CompatibilityModel {
    /// The status line's text while it has no error or notice: the difference under the pointer, else the reason.
    pub fn line(&self) -> &str {
        if self.under.is_empty() { &self.reason } else { &self.under }
    }

    /// Whether `end` can be dragged: the end of a shape's line on a layer beside the edited one.
    pub fn draggable(&self, link: &Link, end: &LinkEnd) -> bool {
        matches!(link.kind, LinkKind::Start | LinkKind::Centre)
            && end.shape.is_some()
            && self.layers.get(end.placed).is_some_and(|layer| !layer.current && !layer.layer.is_empty())
    }

    /// The difference nearest to `at` within `reach` font units: a segment's, else that of a line end.
    fn difference_at(&self, at: Point, reach: f64) -> &str {
        let mut best: Option<(f64, &str)> = None;
        let near = reach * reach;
        for segment in self.layers.iter().flat_map(|l| &l.segments).filter(|s| !s.note.is_empty()) {
            if !segment.path.control_box().inflate(reach, reach).contains(at) {
                continue;
            }
            for piece in segment.path.segments() {
                let distance = piece.nearest(at, reach.max(1e-3) * 0.1).distance_sq;
                if distance <= near && best.is_none_or(|(d, _)| distance < d) {
                    best = Some((distance, segment.note.as_str()));
                }
            }
        }
        if let Some((_, note)) = best {
            return note;
        }
        for link in self.links.iter().filter(|l| !l.note.is_empty()) {
            for end in &link.ends {
                let distance = (Point::new(end.at[0], end.at[1]) - at).hypot2();
                if distance <= near && best.is_none_or(|(d, _)| distance < d) {
                    best = Some((distance, link.note.as_str()));
                }
            }
        }
        best.map_or("", |(_, note)| note)
    }
}

/// One layer drawn beside the edited one.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PlacedLayer {
    /// The layer's id.
    pub layer: String,
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
    /// How it differs from its counterpart; empty for a compatible segment.
    pub note: String,
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
    /// A selected node.
    Node,
}

/// Where a line meets a layer.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkEnd {
    pub at: [f64; 2],
    /// The layer, as an index into [`CompatibilityModel::layers`].
    pub placed: usize,
    /// The shape's index in the layer's shapes (`glyph.swapShapes` counts images too); `None` for an anchor or a node,
    /// and where the layer lacks the shape.
    pub shape: Option<usize>,
}

/// A line end being dragged.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct EndDrag {
    /// The document, its revision and the glyph the drag began in: in any other the shapes are not those dragged.
    pub document: Option<(u64, u64)>,
    pub glyph: String,
    pub layer: String,
    pub placed: usize,
    pub shape: usize,
    /// Where the press was and where the pointer is.
    pub from: [f64; 2],
    pub at: [f64; 2],
    /// The pointer left the press: a drag, not a click.
    pub moved: bool,
    /// The shape of the same layer whose line end is under the pointer: a release swaps the two.
    pub onto: Option<usize>,
}

/// A line across the layers of one group.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub kind: LinkKind,
    /// One end per layer of the group; a layer without the element gives its (0, 0).
    pub ends: Vec<LinkEnd>,
    /// How the layers of the group differ in it; empty when they agree.
    pub note: String,
    /// The component base names or anchor names that differ within the group, with where they are written.
    pub mismatches: Vec<([f64; 2], String)>,
}

pub fn project(model: &mut Tracked<CompatibilityModel>, cx: &mut ProjectCx<'_>) {
    let mut value = if cx.view.display.master_compatibility && !cx.view.edit_background {
        compute(cx).unwrap_or_default()
    } else {
        CompatibilityModel::default()
    };
    // `incompatibility::project` keeps these.
    value.bars.clone_from(&model.bars);
    value.reason.clone_from(&model.reason);
    // `project_probe` and `project_pointer` keep these.
    value.probe.clone_from(&model.probe);
    value.drag.clone_from(&model.drag);
    value.under.clone_from(&model.under);
    super::set(model, value);
}

/// What the selected nodes' lines were last made from.
pub type ProbeSeen = Option<(Stamp, BTreeSet<(usize, usize)>)>;

/// The lines of the selected nodes to the same nodes of the other layers of the group
/// (`masterCompatibility.selectionProbe`), while the view is on.
pub fn project_probe(model: &mut Tracked<CompatibilityModel>, seen: &mut ProbeSeen, selection: &Selection, stamp: Stamp, cx: &ProjectCx<'_>) {
    if seen.as_ref().is_some_and(|(s, points)| *s == stamp && *points == selection.points) {
        return;
    }
    *seen = Some((stamp, selection.points.clone()));
    let probe = if model.layers.is_empty() { Vec::new() } else { probe_of(cx, &selection.points).unwrap_or_default() };
    if model.probe != probe {
        model.edit(|m| m.probe = probe);
    }
}

fn probe_of(cx: &ProjectCx<'_>, points: &BTreeSet<(usize, usize)>) -> Option<Vec<Link>> {
    if points.is_empty() {
        return None;
    }
    let name = super::glyph::shown_glyph(cx)?;
    let font = cx.session.font().ok()?;
    let glyph = font.glyph(&name)?;
    let current = crate::shell::masters::shown_layer(font, &cx.view.layers, glyph)?;
    let (placed, layers) = placed_layers(font, glyph, current)?;
    Some(probe_links(&placed, &layers, current, points))
}

/// The dragged line end and the difference under the pointer. `pointer` is in the view's font units, `reach` how near
/// it must come.
pub fn project_pointer(model: &mut Tracked<CompatibilityModel>, pointer: Option<Point>, reach: f64, drag: Option<&EndDrag>) {
    let drag = drag.filter(|d| d.moved && !model.layers.is_empty());
    let under = match pointer.filter(|p| drag.is_none() && p.x.is_finite() && p.y.is_finite()) {
        Some(at) if reach.is_finite() && reach > 0.0 => model.difference_at(at, reach).to_string(),
        _ => String::new(),
    };
    if model.drag.as_ref() != drag || model.under != under {
        let drag = drag.cloned();
        model.edit(|m| {
            m.drag = drag;
            m.under = under;
        });
    }
}

fn compute(cx: &ProjectCx<'_>) -> Option<CompatibilityModel> {
    let font = cx.session.font().ok()?;
    let glyph = font.glyph(crate::canvas::tools::edited_glyph(font, cx.view)?)?;
    let current = crate::shell::masters::shown_layer(font, &cx.view.layers, glyph)?;
    Some(model_of(font, glyph, current))
}

/// The view of `glyph` with `current` as the edited layer.
pub fn model_of(font: &Font, glyph: &Glyph, current: &Layer) -> CompatibilityModel {
    let Some((placed, layers)) = placed_layers(font, glyph, current) else { return CompatibilityModel::default() };
    let mut out = CompatibilityModel::default();
    let mut notes = 0;
    for (p, layer) in placed.iter().zip(&layers) {
        let others: Vec<&Layer> = placed.iter().zip(&layers).filter(|(q, l)| q.group == p.group && l.id != layer.id).map(|(_, l)| *l).collect();
        out.layers.push(place_layer(font, layer, p.offset, &others, layer.id == current.id, &mut notes));
    }
    out.links = links(font, &placed, &layers);
    out
}

/// The layers drawn for `glyph` and where; `None` when one cannot be found.
fn placed_layers<'a>(font: &Font, glyph: &'a Glyph, current: &'a Layer) -> Option<(Vec<Placed>, Vec<&'a Layer>)> {
    let sources = Designspace::new(font).ok().and_then(|ds| GlyphSources::of(font, &ds, &glyph.name).ok());
    let placed = placement(font, glyph, current, sources.as_ref());
    let layers: Vec<&Layer> =
        placed.iter().filter_map(|p| glyph.layers.iter().find(|l| l.id == p.layer).or((p.layer == current.id).then_some(current))).collect();
    (layers.len() == placed.len()).then_some((placed, layers))
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

/// The index in the layer's shapes of each of [`outline_shapes`].
fn outline_slots(layer: &Layer) -> Vec<usize> {
    let outline = |s: &Shape| matches!(s, Shape::Path(_) | Shape::Component(_));
    layer.shapes.iter().enumerate().filter(|(_, s)| outline(s)).map(|(i, _)| i).take(MAX_LINKED).collect()
}

fn place_layer(font: &Font, layer: &Layer, offset: (f64, f64), others: &[&Layer], current: bool, notes: &mut usize) -> PlacedLayer {
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
    let segments =
        segment_states(font, layer, others, notes).into_iter().map(|(state, note, path)| PlacedSegment { state, note, path: shift * path }).collect();
    PlacedLayer { layer: layer.id.clone(), current, shapes, segments }
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

/// How a segment differs from its counterpart on another layer.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Difference {
    /// The other layer has no path at this place of the shape order.
    NoPath,
    /// The paths' segments differ at this node.
    Segments { node: usize },
    /// One path is open, the other closed.
    Closed,
    /// The paths have different numbers of nodes.
    Nodes { here: usize, there: usize },
    /// Compatible, but its chord turns by this many degrees.
    Turns(f64),
}

impl Difference {
    fn state(self) -> SegmentState {
        if matches!(self, Difference::Turns(_)) { SegmentState::Turns } else { SegmentState::Incompatible }
    }

    /// As the status line says it of shape `shape` (from 0) of the layer called `here`, against the one called `there`.
    fn text(self, shape: usize, here: &str, there: &str) -> String {
        let shape = shape + 1;
        match self {
            Difference::NoPath => format!("{here}: shape {shape} is a path, and {there} has no path in its place"),
            Difference::Segments { node } => format!("{here}: shape {shape} has other segments than on {there} from node {} on", node + 1),
            Difference::Closed => format!("{here}: shape {shape} is open on one layer and closed on the other, {there}"),
            Difference::Nodes { here: a, there: b } => format!("{here}: shape {shape} has {a} nodes against {b} on {there}"),
            Difference::Turns(degrees) => format!("{here}: this segment of shape {shape} turns by {}° against {there}", degrees.round()),
        }
    }
}

/// Each segment of each path of `layer` with its state against `others` (the rest of its group) and how it differs.
/// Nothing without other layers.
fn segment_states(font: &Font, layer: &Layer, others: &[&Layer], notes: &mut usize) -> Vec<(SegmentState, String, BezPath)> {
    let mut out = Vec::new();
    if others.is_empty() {
        return out;
    }
    let other_shapes: Vec<Vec<&Shape>> = others.iter().map(|l| outline_shapes(l)).collect();
    for (o, shape) in outline_shapes(layer).into_iter().enumerate() {
        let Shape::Path(p) = shape else { continue };
        for seg in segments(&p.node_points(), p.closed) {
            // The first of the worst differences.
            let mut worst: Option<(Difference, &Layer)> = None;
            for (shapes, other) in other_shapes.iter().zip(others) {
                let found = match shapes.get(o) {
                    Some(Shape::Path(q)) => compare(p, q, &seg),
                    _ => Some(Difference::NoPath),
                };
                if let Some(found) = found
                    && worst.is_none_or(|(w, _)| found.state() > w.state())
                {
                    worst = Some((found, other));
                }
            }
            let mut note = String::new();
            if let Some((difference, other)) = worst
                && *notes < MAX_NOTES
            {
                *notes += 1;
                note = difference.text(o, &layer_title(font, layer), &layer_title(font, other));
            }
            out.push((worst.map_or(SegmentState::Compatible, |(d, _)| d.state()), note, piece_path(&seg.pieces)));
        }
    }
    out
}

/// One segment of `p` against path `q`.
fn compare(p: &Path, q: &Path, seg: &fontcraft_geom::Segment) -> Option<Difference> {
    let (here, there) = (p.nodes.len(), q.nodes.len());
    let first = match p.nodes.iter().zip(&q.nodes).position(|(m, n)| m.kind != n.kind) {
        // The node counts say more than where the kinds first part.
        Some(node) if here != there => Some((node, Difference::Nodes { here, there })),
        Some(node) => Some((node, Difference::Segments { node })),
        None if p.closed != q.closed => Some((0, Difference::Closed)),
        None => (here != there).then(|| (here.min(there), Difference::Nodes { here, there })),
    };
    if let Some((from, difference)) = first
        && (seg.end >= from || seg.handles.iter().any(|&h| h >= from))
    {
        return Some(difference);
    }
    let chord = |path: &Path| {
        let (a, b) = (path.nodes.get(seg.start)?, path.nodes.get(seg.end)?);
        Some((b.x - a.x, b.y - a.y))
    };
    match (chord(p), chord(q)) {
        (Some(u), Some(v)) => turn(u, v).filter(|d| *d > ANGLE_WARNING_DEGREES).map(Difference::Turns),
        _ => None,
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
    let mut lines = 0usize;
    let mut groups: Vec<usize> = placed.iter().map(|p| p.group).collect();
    groups.dedup();
    for group in groups {
        let members: Vec<(usize, &Layer)> =
            placed.iter().zip(layers).enumerate().filter(|(_, (p, _))| p.group == group).map(|(i, (_, l))| (i, *l)).collect();
        let shift = |i: usize, p: Point| placed.get(i).map_or([p.x, p.y], |pl| [p.x + pl.offset.0, p.y + pl.offset.1]);
        let shapes: Vec<Vec<&Shape>> = members.iter().map(|(_, l)| outline_shapes(l)).collect();
        let slots: Vec<Vec<usize>> = members.iter().map(|(_, l)| outline_slots(l)).collect();
        let titles: Vec<String> = members.iter().map(|(_, l)| layer_title(font, l)).collect();
        let per_link = members.len().saturating_sub(1);
        let count = shapes.iter().map(Vec::len).max().unwrap_or(0);
        for s in 0..count {
            lines = lines.saturating_add(per_link);
            if lines > MAX_LINES {
                return out;
            }
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
            let mut link =
                Link { kind: if component { LinkKind::Centre } else { LinkKind::Start }, note: shape_note(s, &at_s, &titles), ..Link::default() };
            for (k, (i, layer)) in members.iter().enumerate() {
                let at = match at_s.get(k).copied().flatten() {
                    Some(Shape::Path(p)) => p.nodes.iter().find(|n| n.on_curve()).map(|n| n.point()),
                    Some(Shape::Component(c)) => Some(centre(font, c, &layer.master)),
                    _ => None,
                };
                let point = shift(*i, at.unwrap_or(Point::ZERO));
                link.ends.push(LinkEnd { at: point, placed: *i, shape: slots.get(k).and_then(|l| l.get(s)).copied() });
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
            lines = lines.saturating_add(per_link);
            if lines > MAX_LINES {
                return out;
            }
            let found: Vec<Option<Point>> =
                members.iter().map(|(_, l)| l.anchors.iter().find(|a| a.name == name).map(|a| Point::new(a.x, a.y))).collect();
            let disagree = found.iter().any(Option::is_none);
            let lacking: Vec<&str> = titles.iter().zip(&found).filter(|(_, at)| at.is_none()).map(|(t, _)| t.as_str()).collect();
            let note = if lacking.is_empty() { String::new() } else { format!("no anchor `{name}` on {}", lacking.join(", ")) };
            let mut link = Link { kind: LinkKind::Anchor, note, ..Link::default() };
            for ((i, _), at) in members.iter().zip(&found) {
                let point = shift(*i, at.unwrap_or(Point::ZERO));
                link.ends.push(LinkEnd { at: point, placed: *i, shape: None });
                if disagree && at.is_some() {
                    link.mismatches.push((point, name.to_string()));
                }
            }
            out.push(link);
        }
    }
    out
}

/// How the layers of a group differ in the shape at `s` (each layer's, in order); empty when they agree in what it is.
fn shape_note(s: usize, shapes: &[Option<&&Shape>], titles: &[String]) -> String {
    let kind = |shape: &Option<&&Shape>| match shape {
        Some(Shape::Path(_)) => "a path".to_string(),
        Some(Shape::Component(c)) => format!("a component of `{}`", c.glyph),
        _ => "missing".to_string(),
    };
    let kinds: Vec<String> = shapes.iter().map(kind).collect();
    let Some(first) = kinds.first() else { return String::new() };
    match kinds.iter().zip(titles).find(|(k, _)| *k != first).zip(titles.first()) {
        Some(((other, there), here)) => format!("shape {} is {first} on {here} and {other} on {there}", s + 1),
        None => String::new(),
    }
}

/// The lines of the nodes `points` (shape and node index on `current`) across the layers of its group. A layer without
/// the node gives its (0, 0).
fn probe_links(placed: &[Placed], layers: &[&Layer], current: &Layer, points: &BTreeSet<(usize, usize)>) -> Vec<Link> {
    let Some(group) = placed.iter().find(|p| p.layer == current.id).map(|p| p.group) else { return Vec::new() };
    let members: Vec<(usize, &Placed, &Layer)> =
        placed.iter().zip(layers).enumerate().filter(|(_, (p, _))| p.group == group).map(|(i, (p, l))| (i, p, *l)).collect();
    let shapes: Vec<Vec<&Shape>> = members.iter().map(|(_, _, l)| outline_shapes(l)).collect();
    let own = outline_slots(current);
    let per_link = members.len().saturating_sub(1).max(1);
    let mut out = Vec::new();
    for &(shape, node) in points.iter().take(MAX_LINES / per_link) {
        let Some(o) = own.iter().position(|slot| *slot == shape) else { continue };
        let mut link = Link { kind: LinkKind::Node, ..Link::default() };
        for ((i, p, _), list) in members.iter().zip(&shapes) {
            let at = match list.get(o) {
                Some(Shape::Path(path)) => path.nodes.get(node).map(|n| n.point()),
                _ => None,
            };
            let at = at.unwrap_or(Point::ZERO);
            link.ends.push(LinkEnd { at: [at.x + p.offset.0, at.y + p.offset.1], placed: *i, shape: None });
        }
        out.push(link);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use fontcraft_engine::model::{Axis, Font, Glyph, GlyphTable, Layer, Master, Path, Shape};

    use super::{LinkKind, MAX_LINES, model_of, placed_layers, probe_links};

    /// Masters Light and Bold; glyph `n` of `light` and `bold` squares, 200 units apart.
    fn font(light: usize, bold: usize) -> Font {
        let mut f = Font::new("Compat");
        f.axes = std::sync::Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
        let master = |id: &str, at: f64| Master { id: id.into(), name: id.into(), location: vec![at], ..Master::default() };
        f.masters = std::sync::Arc::new(vec![master("Light", 30.0), master("Bold", 120.0)]);
        let mut g = Glyph::new("n");
        for (id, count) in [("Light", light), ("Bold", bold)] {
            let mut l = Layer::new(id, 600.0);
            l.shapes = (0..count).map(|i| Shape::Path(Path::rect(200.0 * i as f64, 0.0, 200.0 * i as f64 + 100.0, 100.0))).collect();
            g.layers.push(l);
        }
        f.glyphs = GlyphTable::from_glyphs(vec![std::sync::Arc::new(g)]);
        f
    }

    /// The probe's lines run to the same nodes of the other layer; where it lacks the shape, to its (0, 0).
    #[test]
    fn the_probe_joins_the_selected_nodes_and_a_missing_shape_points_to_the_origin() {
        let f = font(2, 1);
        let glyph = f.glyph("n").unwrap();
        let current = &glyph.layers[0];
        let (placed, layers) = placed_layers(&f, glyph, current).unwrap();
        let links = probe_links(&placed, &layers, current, &[(0, 2), (1, 2)].into());
        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|l| l.kind == LinkKind::Node && l.ends.len() == 2));
        let Shape::Path(path) = &glyph.layers[1].shapes[0] else { panic!("a path") };
        let (dx, dy) = placed[1].offset;
        assert_eq!(links[0].ends[1].at, [path.nodes[2].x + dx, path.nodes[2].y + dy]);
        assert_eq!(links[1].ends[1].at, [dx, dy], "Bold has no second shape");

        let model = model_of(&f, glyph, current);
        assert_eq!((model.links[1].ends[1].at, model.links[1].ends[1].shape), ([dx, dy], None));
        assert_eq!(model.links[1].note, "shape 2 is a path on Light and missing on Bold");
        assert_eq!(model.layers[0].segments[4].note, "Light: shape 2 is a path, and Bold has no path in its place");
    }

    #[test]
    fn the_lines_are_capped() {
        let f = font(1_500, 1_500);
        let glyph = f.glyph("n").unwrap();
        let model = model_of(&f, glyph, &glyph.layers[0]);
        let lines: usize = model.links.iter().map(|l| l.ends.len() - 1).sum();
        assert!(lines <= MAX_LINES, "{lines}");
        let (placed, layers) = placed_layers(&f, glyph, &glyph.layers[0]).unwrap();
        let points = (0..1_000).flat_map(|s| (0..4).map(move |n| (s, n))).chain((1_000..1_400).map(|s| (s, 0))).collect();
        assert_eq!(probe_links(&placed, &layers, &glyph.layers[0], &points).len(), MAX_LINES);
    }
}
