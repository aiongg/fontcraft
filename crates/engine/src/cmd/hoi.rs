//! Higher-order interpolation commands (M9.30, `hoi.*`, `docs/plans/M9.md` › M9.30): set,
//! convert and remove the curve a node follows between two layers along one axis
//! (`fontcraft_model::hoi`, evaluated by `fontcraft_interp::hoi`).
//!
//! Every command names a glyph, a layer, the nodes (`points`, `[shape, node]` pairs on that
//! layer) and an axis tag. A curve belongs to a span: the layer and its neighbour along the axis,
//! the next layer with the same coordinates on every other axis. `toward` picks the span:
//!
//! | `toward` | Span | Node that keeps the curve |
//! |---|---|---|
//! | `up` (default) | the layer → the next layer up | the layer's node |
//! | `down` | the next layer down → the layer | the node of the layer below |
//!
//! The node is the same `[shape, node]` on both layers; a path whose node counts differ between
//! them is refused. A font with no axis or fewer than two masters is refused, as is a locked
//! glyph. Each command that changes something is one undo step; one that fails changes nothing.

use fontcraft_interp::hoi::{self, Span};
use fontcraft_interp::{Designspace, GlyphSources};
use fontcraft_model::{Font, Hoi, HoiAxis, HoiCenter, HoiCurve, HoiDirection, HoiTiming, Layer, LayerSide, Path, Shape};
use serde_json::{Value, json};

use super::common::in_range;
use super::*;
use crate::params::{Params, TargetKind as K, Targets};

const TARGETS: &[K] = &[K::Layer, K::Points];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "hoi.setInterpolationType",
            "Interpolation Type",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), points: [[shape, node]], axis: tag, toward?: \"up\" | \"down\" (default up), type: \"linear\" | \"intermediatePoint\" | \"handles\" | \"rotation\"} sets how the nodes travel along the span: linear removes the curve; intermediatePoint puts the point where the node is halfway along its present path; handles takes the cubic equal to an intermediate point (or to the present path through its halfway position), the straight line for a linear node; rotation turns about the automatic centre (the middle of the path's bounding boxes) in the automatic direction. A node already of the type is left as it is. Needs a font with an axis and two masters → {glyph, layer (the layer that keeps the curves), axis, changed: [[shape, node]]}",
            hoi_enabled,
            set_interpolation_type
        )
        .targets(TARGETS),
        cmd!(
            "hoi.setControlPoints",
            "Set Interpolation Control Points",
            [],
            None,
            "{glyph: name, layer?: id, points: [[shape, node]] (one node), axis: tag, toward?: \"up\" | \"down\" (default up), positions: [[x, y]] (one or two, glyph coordinates)} the headless form of dragging a knob: two positions make the curve's two handles (lower, then upper); one position moves a rotation's centre, or makes any other curve an intermediate point there → {glyph, layer, axis, changed: [[shape, node]]}",
            hoi_enabled,
            set_control_points
        )
        .targets(TARGETS),
        cmd!(
            "hoi.convertToHandles",
            "Convert Intermediate Point to Handles",
            [],
            Some("Return"),
            "{glyph: name, layer?: id, points: [[shape, node]], axis: tag, toward?: \"up\" | \"down\" (default up)} replaces each intermediate point among the nodes with the two handles of the same curve; other nodes are left alone, and nodes without an intermediate point are refused → {glyph, layer, axis, changed: [[shape, node]]}",
            hoi_enabled,
            convert_to_handles
        )
        .targets(TARGETS)
        .when("tool:higherOrderInterpolation"),
        cmd!(
            "hoi.centerRotation",
            "Center Rotation in Bounds",
            [],
            None,
            "{glyph: name, layer?: id, points: [[shape, node]], axis: tag, toward?: \"up\" | \"down\" (default up)} puts the centre of each rotation among the nodes back at the automatic centre, the middle of the path's bounding boxes on the two layers; nodes without a rotation are left alone, and nodes with none are refused → {glyph, layer, axis, changed: [[shape, node]]}",
            hoi_enabled,
            center_rotation
        )
        .targets(TARGETS),
        cmd!(
            "hoi.setRotationDirection",
            "Rotation Direction",
            [],
            None,
            "{glyph: name, layer?: id, points: [[shape, node]], axis: tag, toward?: \"up\" | \"down\" (default up), direction: \"automatic\" | \"clockwise\" | \"counterclockwise\"} sets the sense of each rotation among the nodes; automatic takes the smaller angle. Nodes without a rotation are left alone, and nodes with none are refused → {glyph, layer, axis, changed: [[shape, node]]}",
            hoi_enabled,
            set_rotation_direction
        )
        .targets(TARGETS),
        cmd!(
            "hoi.removeCurve",
            "Remove Interpolation Curve",
            [],
            Some("Delete"),
            "{glyph: name, layer?: id, points: [[shape, node]], axis: tag, toward?: \"up\" | \"down\" (default up)} removes the nodes' curves on the axis, so they move in a straight line again (the same as type linear); works without a layer on the other side of the span. Nodes without a curve are refused → {glyph, layer, axis, changed: [[shape, node]]}",
            hoi_enabled,
            remove_curve
        )
        .targets(TARGETS)
        .when("tool:higherOrderInterpolation"),
    ]
}

/// The font needs an axis and two masters (`higherOrderInterpolation`).
fn font_refusal(font: &Font) -> Option<&'static str> {
    if font.axes.is_empty() {
        Some("higher-order interpolation needs a font with at least one axis")
    } else if font.masters.len() < 2 {
        Some("higher-order interpolation needs a font with at least two masters")
    } else {
        None
    }
}

fn hoi_enabled(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    let font = s.font().map_err(|e| e.to_string())?;
    font_refusal(font).map_or(Ok(()), |why| Err(why.to_string()))
}

/// One addressed node: its address and its span (`None` only when there is no layer on the far
/// side and the command does not need one).
struct Target {
    at: (usize, usize),
    span: Option<Span>,
    curve: Option<HoiCurve>,
}

/// The nodes a command acts on, read from the font before it changes anything.
struct Scope {
    glyph: String,
    /// The layer whose nodes keep the curves.
    lower: String,
    tag: String,
    targets: Vec<Target>,
}

impl Scope {
    /// Reads the targets. `need_span` refuses a span without a layer on the far side.
    fn read(p: &Params, font: &Font, need_span: bool) -> Result<Scope> {
        if let Some(why) = font_refusal(font) {
            return Err(p.bad(why));
        }
        let t = Targets::parse(p)?;
        if t.side == LayerSide::Background {
            return Err(p.bad("interpolation curves belong to a layer's foreground"));
        }
        if t.points.is_empty() {
            return Err(p.bad("`points` names no node"));
        }
        let r = t.resolve(font)?;
        crate::check_unlocked(r.glyph)?;
        let tag = p.str("axis")?;
        let ds = Designspace::new(font).map_err(|e| p.bad(e.to_string()))?;
        if !ds.axes().iter().any(|a| a.tag == tag) {
            return Err(p.bad(format!("the font has no axis `{tag}`")));
        }
        let up = match p.opt_str("toward")? {
            None | Some("up") => true,
            Some("down") => false,
            Some(other) => return Err(p.bad(format!("`toward` must be \"up\" or \"down\", got \"{other}\""))),
        };
        let sources = GlyphSources::of(font, &ds, &t.glyph).map_err(|e| p.bad(e.to_string()))?;
        let here = r.layer.id.as_str();
        let neighbour = hoi::neighbour_layer(&ds, &sources, here, tag, up).map_err(|e| p.bad(e.to_string()))?;
        let (lower_id, upper_id) = if up {
            (here.to_string(), neighbour)
        } else {
            let below = neighbour.ok_or_else(|| {
                p.bad(format!("layer `{here}` has no layer below it along axis `{tag}` with the same coordinates on the other axes"))
            })?;
            (below, Some(here.to_string()))
        };
        if need_span && upper_id.is_none() {
            return Err(p.bad(format!("layer `{here}` has no layer above it along axis `{tag}` with the same coordinates on the other axes")));
        }
        let layer = |id: &str| r.glyph.layers.iter().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{}` has no layer `{id}`", t.glyph)));
        let lower = layer(lower_id.as_str())?;
        let upper = upper_id.as_deref().map(layer).transpose()?;
        let mut targets = Vec::with_capacity(t.points.len());
        for &(s, n) in &t.points {
            let lp = path(lower, s)
                .filter(|lp| n < lp.nodes.len())
                .ok_or_else(|| p.bad(format!("node [{s}, {n}] has no counterpart on layer `{lower_id}`")))?;
            let span = match upper {
                Some(u) => Some(
                    path(u, s)
                        .and_then(|upath| hoi::node_span(lp, upath, n))
                        .ok_or_else(|| p.bad(format!("path {s} does not correspond node for node on layers `{lower_id}` and `{}`", u.id)))?,
                ),
                None => None,
            };
            let curve = lp.nodes.get(n).and_then(|node| node.info.hoi.get(tag)).cloned();
            targets.push(Target { at: (s, n), span, curve });
        }
        Ok(Scope { glyph: t.glyph.clone(), lower: lower_id, tag: tag.to_string(), targets })
    }

    /// Writes the new curves (`None` removes one) as one undo step labelled `label`, and returns
    /// the command's result.
    fn write(&self, s: &mut Session, p: &Params, label: &str, changes: Vec<((usize, usize), Option<HoiCurve>)>) -> Result<Value> {
        let changed: Vec<Value> = changes.iter().map(|((sh, n), _)| json!([sh, n])).collect();
        if !changes.is_empty() {
            s.edit(label, |font| {
                let mut g = font.glyph_mut(&self.glyph)?;
                let layer = g.layers.iter_mut().find(|l| l.id == self.lower).ok_or_else(|| p.bad(format!("there is no layer `{}`", self.lower)))?;
                for ((sh, n), curve) in changes {
                    let Some(Shape::Path(path)) = layer.shapes.get_mut(sh) else { return Err(p.bad(format!("shape {sh} is not a path"))) };
                    let node = path.nodes.get_mut(n).ok_or_else(|| p.bad(format!("path {sh} has no node {n}")))?;
                    let mut hoi = node.info.hoi.clone();
                    set_entry(&mut hoi, &self.tag, curve);
                    hoi.check().map_err(|e| p.bad(e))?;
                    node.info.hoi = hoi;
                    node.info.tidy();
                }
                Ok(())
            })?;
        }
        Ok(json!({"glyph": self.glyph, "layer": self.lower, "axis": self.tag, "changed": changed}))
    }
}

fn path(layer: &Layer, shape: usize) -> Option<&Path> {
    match layer.shapes.get(shape) {
        Some(Shape::Path(p)) => Some(p),
        _ => None,
    }
}

/// Replaces, adds or (with `None`) removes the curve for `tag`.
fn set_entry(hoi: &mut Hoi, tag: &str, curve: Option<HoiCurve>) {
    match curve {
        None => hoi.0.retain(|e| e.axis != tag),
        Some(curve) => match hoi.0.iter_mut().find(|e| e.axis == tag) {
            Some(e) => e.curve = curve,
            None => hoi.0.push(HoiAxis { axis: tag.to_string(), curve }),
        },
    }
}

/// The span of a target that needs one ([`Scope::read`] with `need_span` guarantees it).
fn span_of(p: &Params, t: &Target) -> Result<Span> {
    t.span.ok_or_else(|| p.bad(format!("node [{}, {}] has no layer on the other side of the span", t.at.0, t.at.1)))
}

/// The handles of the cubic equal to the curve through `ip`, refused for a node at the same place
/// on both layers.
fn to_handles(p: &Params, t: &Target, span: &Span, ip: hoi::Point) -> Result<HoiCurve> {
    hoi::point_to_handles(span, ip).map(HoiCurve::Timing).ok_or_else(|| {
        p.bad(format!("node [{}, {}] is at the same place on both layers, or its curve is too far out, so it cannot have handles", t.at.0, t.at.1))
    })
}

fn set_interpolation_type(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hoi.setInterpolationType", v)?;
    let kind = p.str("type")?;
    if !["linear", "intermediatePoint", "handles", "rotation"].contains(&kind) {
        return Err(p.bad(format!("`type` must be \"linear\", \"intermediatePoint\", \"handles\" or \"rotation\", got \"{kind}\"")));
    }
    let scope = Scope::read(&p, s.font()?, kind != "linear")?;
    let mut changes = Vec::new();
    for t in &scope.targets {
        let new = match (kind, &t.curve) {
            ("linear", None)
            | ("intermediatePoint", Some(HoiCurve::Point { .. }))
            | ("handles", Some(HoiCurve::Timing(_)))
            | ("rotation", Some(HoiCurve::Rotation { .. })) => {
                continue;
            }
            ("linear", Some(_)) => None,
            ("intermediatePoint", curve) => {
                let span = span_of(&p, t)?;
                let (x, y) = curve.as_ref().map_or_else(|| span.linear(0.5), |c| hoi::position(c, &span, 0.5));
                Some(HoiCurve::Point { x, y })
            }
            ("handles", None) => Some(HoiCurve::Timing(HoiTiming::default())),
            ("handles", Some(HoiCurve::Point { x, y })) => Some(to_handles(&p, t, &span_of(&p, t)?, (*x, *y))?),
            ("handles", Some(curve)) => {
                let span = span_of(&p, t)?;
                Some(to_handles(&p, t, &span, hoi::position(curve, &span, 0.5))?)
            }
            _ => Some(HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: None }),
        };
        changes.push((t.at, new));
    }
    scope.write(s, &p, "Interpolation Type", changes)
}

fn set_control_points(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hoi.setControlPoints", v)?;
    let scope = Scope::read(&p, s.font()?, true)?;
    let [t] = scope.targets.as_slice() else { return Err(p.bad("`points` must name exactly one node")) };
    let positions = p
        .array("positions")?
        .iter()
        .map(|v| match v.as_array().map(Vec::as_slice) {
            Some([x, y]) => match (x.as_f64(), y.as_f64()) {
                (Some(x), Some(y)) if in_range(x) && in_range(y) => Ok((x, y)),
                _ => Err(p.bad(format!("`positions`: {v} is not a pair of coordinates within the model's range"))),
            },
            _ => Err(p.bad(format!("`positions` must hold [x, y] pairs, found {v}"))),
        })
        .take(3)
        .collect::<Result<Vec<_>>>()?;
    let span = span_of(&p, t)?;
    let new = match (positions.as_slice(), &t.curve) {
        ([(x, y)], Some(HoiCurve::Rotation { direction, .. })) => {
            HoiCurve::Rotation { center: Some(HoiCenter::At { x: *x, y: *y }), direction: *direction }
        }
        ([(x, y)], _) => HoiCurve::Point { x: *x, y: *y },
        ([h1, h2], _) => HoiCurve::Timing(
            hoi::handles_to_timing(&span, *h1, *h2)
                .ok_or_else(|| p.bad("the node is at the same place on both layers, or a handle is too far out, so the handles cannot be set"))?,
        ),
        _ => return Err(p.bad("`positions` must hold one point (an intermediate point or a rotation centre) or two (the handles)")),
    };
    let changes = if t.curve.as_ref() == Some(&new) { Vec::new() } else { vec![(t.at, Some(new))] };
    scope.write(s, &p, "Set Interpolation Control Points", changes)
}

fn convert_to_handles(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hoi.convertToHandles", v)?;
    let scope = Scope::read(&p, s.font()?, true)?;
    let mut changes = Vec::new();
    for t in &scope.targets {
        if let Some(HoiCurve::Point { x, y }) = t.curve {
            changes.push((t.at, Some(to_handles(&p, t, &span_of(&p, t)?, (x, y))?)));
        }
    }
    if changes.is_empty() {
        return Err(p.bad(format!("none of the nodes has an intermediate point on axis `{}`", scope.tag)));
    }
    scope.write(s, &p, "Convert Intermediate Point to Handles", changes)
}

/// Changes each rotation among the targets with `f` (`None` leaves it), refusing targets with no
/// rotation at all.
fn each_rotation(
    s: &mut Session,
    p: &Params,
    label: &str,
    f: impl Fn(Option<HoiCenter>, Option<HoiDirection>) -> Option<(Option<HoiCenter>, Option<HoiDirection>)>,
) -> Result<Value> {
    let scope = Scope::read(p, s.font()?, true)?;
    let mut changes = Vec::new();
    let mut any = false;
    for t in &scope.targets {
        let Some(HoiCurve::Rotation { center, direction }) = t.curve else { continue };
        any = true;
        if let Some((center, direction)) = f(center, direction) {
            changes.push((t.at, Some(HoiCurve::Rotation { center, direction })));
        }
    }
    if !any {
        return Err(p.bad(format!("none of the nodes has a rotation on axis `{}`", scope.tag)));
    }
    scope.write(s, p, label, changes)
}

fn center_rotation(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hoi.centerRotation", v)?;
    each_rotation(s, &p, "Center Rotation in Bounds", |center, direction| {
        (center != Some(HoiCenter::Auto)).then_some((Some(HoiCenter::Auto), direction))
    })
}

fn set_rotation_direction(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hoi.setRotationDirection", v)?;
    let want = match p.str("direction")? {
        "automatic" => None,
        "clockwise" => Some(HoiDirection::Clockwise),
        "counterclockwise" => Some(HoiDirection::CounterClockwise),
        other => return Err(p.bad(format!("`direction` must be \"automatic\", \"clockwise\" or \"counterclockwise\", got \"{other}\""))),
    };
    each_rotation(s, &p, "Rotation Direction", |center, direction| {
        // A rotation with neither a centre nor a direction would be written as an empty
        // dictionary, which reads back as a timing; the automatic centre is the same rotation.
        (direction != want).then_some((center.or(Some(HoiCenter::Auto)), want))
    })
}

fn remove_curve(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hoi.removeCurve", v)?;
    let scope = Scope::read(&p, s.font()?, false)?;
    let changes: Vec<_> = scope.targets.iter().filter(|t| t.curve.is_some()).map(|t| (t.at, None)).collect();
    if changes.is_empty() {
        return Err(p.bad(format!("none of the nodes has a curve on axis `{}`", scope.tag)));
    }
    scope.write(s, &p, "Remove Interpolation Curve", changes)
}
