//! The compatibility check: whether a glyph's layers can interpolate (M9.3).
//!
//! Rows: `compatibility.rules`, `compatibility.checkScope`, `compatibility.exportBlocked` (the
//! check), `compile.interpolationCompatibility`, `intermediateLayer.mustBeCompatible`,
//! `alternateLayer.compatibilityAmongThemselves`, parameter `Enforce Compatibility Check`.
//!
//! Contract:
//! - Two layers are compatible when they have the same shapes in the same order (paths and
//!   components counted together; images left out), each path the same node count, the same
//!   node kinds in the same order and the same closedness, each component the same base glyph,
//!   and the same anchors by count and name. Shapes the model keeps unread match each other.
//! - Main sources are checked against each other; each alternate set against itself; each
//!   intermediate layer against every master layer (a guess on
//!   `intermediateLayer.mustBeCompatible`). Compatibility is an equivalence, so a group's layers
//!   are sorted into classes of mutually compatible layers; every layer outside the largest
//!   class (the first one on a tie) is reported against that class's first layer.
//! - The scope: masters with a non-zero weight at some static instance's location plus every
//!   master of a variable font setting not disabled; Enforce Compatibility Check on widens the
//!   scope to every master, off hides the indicators.
//! - Only masters in the [`MasterPairs`] scope count. Intermediate layers count whenever some
//!   master of the scope does. An instance counts when it needs two or more masters; the scope
//!   is the union of the masters such instances need, a guess on `compatibility.checkScope`.
//!   The handbook speaks of pairs of masters; as compatibility is an equivalence, the union
//!   differs from pairs only when two groups of masters share no instance.
//! - Compatible layers next to each other in source order whose corresponding segments turn by
//!   more than [`crate::ANGLE_WARNING_DEGREES`] (the direction from a segment's first on-curve
//!   node to its last, a guess) get an [`AngleWarning`]; warnings do not make a glyph
//!   incompatible.
//! - The check never fails: a glyph it cannot read is reported as incompatible.

use std::collections::{HashMap, HashSet};
use std::fmt;

use fontcraft_geom::variation::VariationModel;
use fontcraft_model::{Font, Layer, Shape};

use crate::designspace::{Designspace, variable_settings};
use crate::params::{self, enforce_compatibility};
use crate::sources::{GlyphSources, Source};
use crate::{ANGLE_WARNING_DEGREES, MAX_INSTANCES};

/// Most classes of mutually compatible layers kept per group; a layer that fits none of them is
/// reported against the largest.
const MAX_CLASSES: usize = 16;

/// A master's weight below this counts as zero.
const ZERO_WEIGHT: f64 = 1e-9;

/// What differs between two layers. Shape and node indices count from 0; shape indices count
/// paths and components (and shapes kept unread), images left out.
#[derive(Clone, Debug, PartialEq)]
pub enum MismatchKind {
    /// Different numbers of shapes (paths and components together).
    ShapeCount { a: usize, b: usize },
    /// A path in one layer where the other has a component, at shape index `shape`.
    ShapeKind { shape: usize },
    /// Paths at shape index `shape` with different node counts.
    NodeCount { shape: usize, a: usize, b: usize },
    /// Paths at shape index `shape` whose segments differ (line against curve, an on-curve node
    /// against a control point, or an open path against a closed one) at node `node`.
    SegmentType { shape: usize, node: usize },
    /// Components at shape index `shape` that refer to different glyphs.
    ComponentGlyph { shape: usize, a: String, b: String },
    /// Different numbers of anchors.
    AnchorCount { a: usize, b: usize },
    /// An anchor name one layer has and the other lacks.
    AnchorName { name: String },
}

impl fmt::Display for MismatchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MismatchKind::ShapeCount { a, b } => write!(f, "{a} shapes against {b}"),
            MismatchKind::ShapeKind { shape } => write!(f, "shape {} is a path in one and a component in the other", shape + 1),
            MismatchKind::NodeCount { shape, a, b } => write!(f, "shape {} has {a} nodes against {b}", shape + 1),
            MismatchKind::SegmentType { shape, node } => write!(f, "shape {} has different segments at node {}", shape + 1, node + 1),
            MismatchKind::ComponentGlyph { shape, a, b } => write!(f, "shape {} is a component of `{a}` against one of `{b}`", shape + 1),
            MismatchKind::AnchorCount { a, b } => write!(f, "{a} anchors against {b}"),
            MismatchKind::AnchorName { name } => write!(f, "only one has the anchor `{name}`"),
        }
    }
}

/// The first difference between two layers of a glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    pub glyph: String,
    /// Layer ids.
    pub layer_a: String,
    pub layer_b: String,
    pub kind: MismatchKind,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layers `{}` and `{}` differ: {}", self.layer_a, self.layer_b, self.kind)
    }
}

impl Mismatch {
    /// The difference as a person reads it, with the layers' names: `Light and Bold differ: shape 2 has 12 nodes
    /// against 13`. A layer the font does not have keeps its id.
    pub fn describe(&self, font: &Font) -> String {
        let glyph = font.glyph(&self.glyph);
        let name = |id: &str| match glyph.and_then(|g| g.layers.iter().find(|l| l.id == id)) {
            Some(layer) => crate::labels::layer_title(font, layer),
            None => id.to_string(),
        };
        format!("{} and {} differ: {}", name(&self.layer_a), name(&self.layer_b), self.kind)
    }
}

/// Corresponding segments of two compatible layers that turn by more than
/// [`crate::ANGLE_WARNING_DEGREES`].
#[derive(Clone, Debug, PartialEq)]
pub struct AngleWarning {
    pub layer_a: String,
    pub layer_b: String,
    pub shape: usize,
    /// The on-curve node that ends the segment.
    pub node: usize,
    /// The turn, in degrees.
    pub degrees: f64,
}

/// The result of checking one glyph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compat {
    /// Every pair of layers in scope is compatible.
    pub ok: bool,
    /// The first difference of each incompatible pair of layers.
    pub first_mismatch: Vec<Mismatch>,
    pub warnings: Vec<AngleWarning>,
}

/// The masters whose layers must be compatible with each other.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterPairs {
    /// Master ids, in the font's order.
    pub masters: Vec<String>,
    /// Whether the editor shows incompatibility indicators (Enforce Compatibility Check off hides
    /// them).
    pub indicators: bool,
}

/// Checks `sources` (one glyph's) within `scope`.
pub fn check_glyph(font: &Font, sources: &GlyphSources, scope: &MasterPairs) -> Compat {
    let mut out = Compat { ok: true, ..Compat::default() };
    let Some(glyph) = font.glyph(&sources.glyph) else {
        out.ok = false;
        return out;
    };
    let in_scope = |s: &Source| scope.masters.contains(&s.master);
    let any_master = sources.main.iter().any(|s| s.is_master && in_scope(s));
    let groups = std::iter::once(sources.main.as_slice()).chain(sources.alternates.iter().map(|a| a.sources.as_slice()));
    for group in groups {
        let mut layers = Vec::with_capacity(group.len());
        for s in group {
            let counts = if s.is_master { in_scope(s) } else { any_master };
            if !counts {
                continue;
            }
            match glyph.layers.iter().find(|l| l.id == s.layer_id) {
                Some(l) => layers.push(l),
                None => out.ok = false,
            }
        }
        check_group(&glyph.name, &layers, &mut out);
    }
    out
}

/// The paths, components and unread shapes of a layer, in order.
fn outline_shapes(layer: &Layer) -> Vec<&Shape> {
    layer.shapes.iter().filter(|s| !matches!(s, Shape::Image(_))).collect()
}

fn check_group(glyph: &str, layers: &[&Layer], out: &mut Compat) {
    if layers.len() < 2 {
        return;
    }
    let shapes: Vec<Vec<&Shape>> = layers.iter().map(|l| outline_shapes(l)).collect();
    let item = |i: usize| layers.get(i).copied().zip(shapes.get(i));
    let differ = |i: usize, j: usize| match (item(i), item(j)) {
        (Some((a, sa)), Some((b, sb))) => first_difference(a, sa, b, sb),
        _ => None,
    };
    let mut classes: Vec<Vec<usize>> = Vec::new();
    let mut strays = Vec::new();
    for i in 0..layers.len() {
        let fits = classes.iter().position(|c| c.first().is_some_and(|&r| differ(r, i).is_none()));
        if let Some(c) = fits.and_then(|n| classes.get_mut(n)) {
            c.push(i);
        } else if classes.len() < MAX_CLASSES {
            classes.push(vec![i]);
        } else {
            strays.push(i);
        }
    }
    let mut main = 0;
    for (n, c) in classes.iter().enumerate() {
        if c.len() > classes.get(main).map_or(0, Vec::len) {
            main = n;
        }
    }
    let reference = classes.get(main).and_then(|c| c.first()).copied().unwrap_or(0);
    let others = classes.iter().enumerate().filter(|(n, _)| *n != main).flat_map(|(_, c)| c.iter()).chain(&strays);
    for &i in others {
        let (Some(kind), Some((a, _)), Some((b, _))) = (differ(reference, i), item(reference), item(i)) else { continue };
        out.ok = false;
        out.first_mismatch.push(Mismatch { glyph: glyph.to_string(), layer_a: a.id.clone(), layer_b: b.id.clone(), kind });
    }
    for c in &classes {
        for pair in c.windows(2) {
            if let [i, j] = *pair
                && let (Some((a, sa)), Some((b, sb))) = (item(i), item(j))
            {
                angle_warnings(a, sa, b, sb, &mut out.warnings);
            }
        }
    }
}

/// The first difference between layers `a` and `b`, given their outline shapes.
fn first_difference(a: &Layer, sa: &[&Shape], b: &Layer, sb: &[&Shape]) -> Option<MismatchKind> {
    if sa.len() != sb.len() {
        return Some(MismatchKind::ShapeCount { a: sa.len(), b: sb.len() });
    }
    for (shape, (x, y)) in sa.iter().zip(sb).enumerate() {
        match (x, y) {
            (Shape::Path(p), Shape::Path(q)) => {
                if p.nodes.len() != q.nodes.len() {
                    return Some(MismatchKind::NodeCount { shape, a: p.nodes.len(), b: q.nodes.len() });
                }
                if let Some(node) = p.nodes.iter().zip(&q.nodes).position(|(m, n)| m.kind != n.kind) {
                    return Some(MismatchKind::SegmentType { shape, node });
                }
                if p.closed != q.closed {
                    return Some(MismatchKind::SegmentType { shape, node: 0 });
                }
            }
            (Shape::Component(c), Shape::Component(d)) => {
                if c.glyph != d.glyph {
                    return Some(MismatchKind::ComponentGlyph { shape, a: c.glyph.clone(), b: d.glyph.clone() });
                }
            }
            (Shape::Raw(_), Shape::Raw(_)) => {}
            _ => return Some(MismatchKind::ShapeKind { shape }),
        }
    }
    if a.anchors.len() != b.anchors.len() {
        return Some(MismatchKind::AnchorCount { a: a.anchors.len(), b: b.anchors.len() });
    }
    let names_a: HashSet<&str> = a.anchors.iter().map(|x| x.name.as_str()).collect();
    let names_b: HashSet<&str> = b.anchors.iter().map(|x| x.name.as_str()).collect();
    let lone =
        a.anchors.iter().find(|x| !names_b.contains(x.name.as_str())).or_else(|| b.anchors.iter().find(|x| !names_a.contains(x.name.as_str())));
    lone.map(|x| MismatchKind::AnchorName { name: x.name.clone() })
}

/// Warnings for the segments of compatible layers `a` and `b` whose direction (first on-curve
/// node to last) turns by more than the threshold.
fn angle_warnings(a: &Layer, sa: &[&Shape], b: &Layer, sb: &[&Shape], out: &mut Vec<AngleWarning>) {
    for (shape, (x, y)) in sa.iter().zip(sb).enumerate() {
        let (Shape::Path(p), Shape::Path(q)) = (x, y) else { continue };
        let on: Vec<usize> = p.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).map(|(i, _)| i).collect();
        for (k, &end) in on.iter().enumerate() {
            let start = match k.checked_sub(1) {
                Some(prev) => on.get(prev),
                None if p.closed && on.len() > 1 => on.last(),
                None => None,
            };
            let Some(&start) = start else { continue };
            let (Some(p0), Some(p1), Some(q0), Some(q1)) = (p.nodes.get(start), p.nodes.get(end), q.nodes.get(start), q.nodes.get(end)) else {
                continue;
            };
            let Some(degrees) = turn((p1.x - p0.x, p1.y - p0.y), (q1.x - q0.x, q1.y - q0.y)) else { continue };
            if degrees > ANGLE_WARNING_DEGREES {
                out.push(AngleWarning { layer_a: a.id.clone(), layer_b: b.id.clone(), shape, node: end, degrees });
            }
        }
    }
}

/// The angle in degrees (0 to 180) between two directions; `None` when either has no length or
/// is not finite.
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

/// The compatibility scope of `font` in `designspace`.
pub fn scope(font: &Font, designspace: &Designspace) -> MasterPairs {
    let ids: Vec<&str> = designspace.masters().iter().map(|m| m.id.as_str()).collect();
    let enforce = enforce_compatibility(font);
    let indicators = enforce != Some(false);
    if enforce == Some(true) {
        return MasterPairs { masters: ids.iter().map(|s| s.to_string()).collect(), indicators };
    }
    let mut needed = vec![false; ids.len()];
    let index: HashMap<&str, usize> = ids.iter().enumerate().rev().map(|(k, id)| (*id, k)).collect();
    let mut mark = |id: &str| {
        if let Some(n) = index.get(id).and_then(|i| needed.get_mut(*i)) {
            *n = true;
        }
    };
    let settings: HashSet<usize> = variable_settings(font).into_iter().collect();
    let model = master_model(designspace);
    // Settings that disable the same masters and carry the same Axis Mappings have the same
    // masters: each such designspace is built once.
    let mut setting_masters: HashMap<(Vec<String>, Option<String>), Vec<String>> = HashMap::new();
    for i in 0..font.instances.len().min(MAX_INSTANCES) {
        if settings.contains(&i) {
            let key = font.instances.get(i).map(|inst| {
                let mut disabled = params::disabled_masters(inst);
                disabled.sort();
                disabled.dedup();
                (disabled, inst.params.get(params::AXIS_MAPPINGS).map(|v| format!("{v:?}")))
            });
            let cached = key.as_ref().and_then(|k| setting_masters.get(k)).cloned();
            let masters: Vec<String> = match cached {
                Some(m) => m,
                None => {
                    let m: Vec<String> = match Designspace::for_setting(font, i) {
                        Ok(ds) => ds.masters().iter().map(|m| m.id.clone()).collect(),
                        Err(_) => ids.iter().map(|s| s.to_string()).collect(),
                    };
                    if let Some(k) = key {
                        setting_masters.insert(k, m.clone());
                    }
                    m
                }
            };
            if masters.len() >= 2 {
                for m in &masters {
                    mark(m);
                }
            }
            continue;
        }
        let weights = model.as_ref().and_then(|m| {
            let location = designspace.instance_location(font, i).ok()?;
            let normalized = designspace.to_normalized(&location).ok()?;
            Some(m.master_scalars(&normalized.0))
        });
        let used: Vec<&str> = match &weights {
            Some(w) => ids.iter().zip(w).filter(|(_, x)| x.abs() > ZERO_WEIGHT).map(|(id, _)| *id).collect(),
            None => ids.clone(),
        };
        if used.len() >= 2 {
            for m in used {
                mark(m);
            }
        }
    }
    let masters = ids.iter().zip(&needed).filter(|(_, n)| **n).map(|(id, _)| id.to_string()).collect();
    MasterPairs { masters, indicators }
}

/// The variation model of the designspace's masters, extrapolating.
fn master_model(designspace: &Designspace) -> Option<VariationModel> {
    let locations: Vec<Vec<f64>> =
        designspace.masters().iter().map(|m| designspace.to_normalized(&m.location).map(|n| n.0)).collect::<Result<_, _>>().ok()?;
    VariationModel::new(&locations, true).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::Value;
    use fontcraft_model::{Anchor, Component, Instance, Layer, Node, NodeKind, Path, Shape};

    use super::*;
    use crate::designspace::Location;
    use crate::params::ENFORCE_COMPATIBILITY_CHECK;
    use crate::sources::AlternateSet;
    use crate::sources::tests::{font_with, glyph, layer, master, rect, two_masters};

    fn source(layer_id: &str, master: &str, at: f64, is_master: bool) -> Source {
        Source { layer_id: layer_id.into(), master: master.into(), location: Location(vec![at]), is_master }
    }

    fn both() -> MasterPairs {
        MasterPairs { masters: vec!["thin".into(), "black".into()], indicators: true }
    }

    /// A rectangle, a component of `b` and the anchors `top` and `bottom`.
    fn base(master: &str, id: &str) -> Layer {
        let mut l = layer(master, id, vec![rect(100.0), Shape::Component(Component::new("b", 0.0, 0.0))]);
        l.anchors = vec![Anchor::new("top", 50.0, 700.0), Anchor::new("bottom", 50.0, 0.0)];
        l
    }

    fn check_pair(black: Layer) -> Compat {
        let font = two_masters(vec![glyph("a", vec![base("thin", "thin"), black])]);
        let sources = GlyphSources {
            glyph: "a".into(),
            main: vec![source("thin", "thin", 100.0, true), source("black", "black", 900.0, true)],
            ..GlyphSources::default()
        };
        check_glyph(&font, &sources, &both())
    }

    fn line(x: f64, y: f64) -> Node {
        Node::new(x, y, NodeKind::Line)
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn each_compatibility_rule_broken_once_is_reported() {
        assert_eq!(check_pair(base("black", "black")), Compat { ok: true, ..Compat::default() });
        let broken: Vec<(Box<dyn Fn(&mut Layer)>, MismatchKind)> = vec![
            (Box::new(|l| l.shapes.push(rect(10.0))), MismatchKind::ShapeCount { a: 2, b: 3 }),
            (Box::new(|l| l.shapes.reverse()), MismatchKind::ShapeKind { shape: 0 }),
            (
                Box::new(|l| {
                    if let Some(Shape::Path(p)) = l.shapes.first_mut() {
                        p.nodes.push(line(0.0, 350.0));
                    }
                }),
                MismatchKind::NodeCount { shape: 0, a: 4, b: 5 },
            ),
            (
                Box::new(|l| {
                    if let Some(Shape::Path(p)) = l.shapes.first_mut() {
                        p.nodes = vec![
                            line(0.0, 0.0),
                            Node::new(100.0, 0.0, NodeKind::OffCurve),
                            Node::new(100.0, 700.0, NodeKind::OffCurve),
                            Node::new(0.0, 700.0, NodeKind::Curve),
                        ];
                    }
                }),
                MismatchKind::SegmentType { shape: 0, node: 1 },
            ),
            (
                Box::new(|l| {
                    if let Some(Shape::Component(c)) = l.shapes.get_mut(1) {
                        c.glyph = "c".into();
                    }
                }),
                MismatchKind::ComponentGlyph { shape: 1, a: "b".into(), b: "c".into() },
            ),
            (Box::new(|l| l.anchors.push(Anchor::new("ogonek", 0.0, 0.0))), MismatchKind::AnchorCount { a: 2, b: 3 }),
            (
                Box::new(|l| {
                    if let Some(a) = l.anchors.first_mut() {
                        a.name = "center".into();
                    }
                }),
                MismatchKind::AnchorName { name: "top".into() },
            ),
        ];
        for (change, kind) in broken {
            let mut black = base("black", "black");
            change(&mut black);
            let c = check_pair(black);
            assert!(!c.ok, "{kind:?}");
            let m = c.first_mismatch.first().unwrap();
            assert_eq!((m.layer_a.as_str(), m.layer_b.as_str(), &m.kind), ("thin", "black", &kind));
            assert!(m.to_string().starts_with("layers `thin` and `black` differ: "), "{m}");
        }
        // Moving points, anchors and components keeps layers compatible; an image is no shape.
        let mut moved = base("black", "black");
        if let Some(Shape::Path(p)) = moved.shapes.first_mut() {
            *p = Path::rect(0.0, 0.0, 300.0, 700.0);
        }
        moved.anchors.reverse();
        moved.shapes.push(Shape::Image(Box::default()));
        assert!(check_pair(moved).ok);
    }

    #[test]
    fn the_layer_at_odds_with_the_rest_is_the_one_reported() {
        let odd = layer("bold", "bold", vec![rect(100.0), rect(10.0)]);
        let g = glyph("o", vec![layer("thin", "thin", vec![rect(100.0)]), odd, layer("black", "black", vec![rect(300.0)])]);
        let font = two_masters(vec![g]);
        let sources = GlyphSources {
            glyph: "o".into(),
            main: vec![source("thin", "thin", 100.0, true), source("bold", "bold", 600.0, true), source("black", "black", 900.0, true)],
            ..GlyphSources::default()
        };
        let scope = MasterPairs { masters: vec!["thin".into(), "bold".into(), "black".into()], indicators: true };
        let c = check_glyph(&font, &sources, &scope);
        assert!(!c.ok);
        assert_eq!(c.first_mismatch.len(), 1);
        let m = c.first_mismatch.first().unwrap();
        assert_eq!((m.layer_a.as_str(), m.layer_b.as_str()), ("thin", "bold"));
        // Out of scope, the odd master is not checked.
        let narrow = MasterPairs { masters: vec!["thin".into(), "black".into()], indicators: true };
        assert!(check_glyph(&font, &sources, &narrow).ok);
    }

    #[test]
    fn alternate_layers_need_only_match_each_other_and_intermediates_every_master() {
        let two = || vec![rect(100.0), Shape::Path(Path::rect(200.0, 0.0, 250.0, 700.0))];
        let g = glyph(
            "dollar",
            vec![
                layer("thin", "thin", vec![rect(100.0)]),
                layer("black", "black", vec![rect(300.0)]),
                layer("thin", "alt-thin", two()),
                layer("black", "alt-black", two()),
                layer("thin", "brace", vec![rect(100.0), rect(5.0)]),
            ],
        );
        let font = two_masters(vec![g]);
        let alternates = |second: &str| {
            vec![AlternateSet {
                ranges: Default::default(),
                sources: vec![source("alt-thin", "thin", 100.0, true), source(second, "black", 900.0, true)],
            }]
        };
        let mut sources = GlyphSources {
            glyph: "dollar".into(),
            main: vec![source("thin", "thin", 100.0, true), source("black", "black", 900.0, true)],
            alternates: alternates("alt-black"),
            ..GlyphSources::default()
        };
        assert!(check_glyph(&font, &sources, &both()).ok, "alternates differ from the masters, which is allowed");

        sources.alternates = alternates("black");
        let c = check_glyph(&font, &sources, &both());
        assert_eq!(c.first_mismatch.iter().map(|m| (m.layer_a.as_str(), m.layer_b.as_str())).collect::<Vec<_>>(), [("alt-thin", "black")]);

        sources.alternates = alternates("alt-black");
        sources.main.push(source("brace", "thin", 400.0, false));
        let c = check_glyph(&font, &sources, &both());
        assert_eq!(c.first_mismatch.iter().map(|m| m.layer_b.as_str()).collect::<Vec<_>>(), ["brace"]);
        assert!(check_glyph(&font, &sources, &MasterPairs::default()).ok, "no master in scope: nothing to check");
    }

    #[test]
    fn segments_turning_by_more_than_twenty_degrees_warn() {
        let stroke = |degrees: f64| {
            let (s, c) = degrees.to_radians().sin_cos();
            Shape::Path(Path::new(false, vec![line(0.0, 0.0), line(100.0 * c, 100.0 * s)]))
        };
        let warnings = |degrees: f64| {
            let g = glyph("l", vec![layer("thin", "thin", vec![stroke(0.0)]), layer("black", "black", vec![stroke(degrees)])]);
            let font = two_masters(vec![g]);
            let sources = GlyphSources {
                glyph: "l".into(),
                main: vec![source("thin", "thin", 100.0, true), source("black", "black", 900.0, true)],
                ..GlyphSources::default()
            };
            check_glyph(&font, &sources, &both())
        };
        let c = warnings(21.0);
        assert!(c.ok, "a warning does not make the glyph incompatible");
        assert_eq!(c.warnings.len(), 1);
        let w = c.warnings.first().unwrap();
        assert_eq!((w.shape, w.node), (0, 1));
        assert!((w.degrees - 21.0).abs() < 1e-6, "{}", w.degrees);
        assert!(warnings(19.0).warnings.is_empty());
        assert_eq!(warnings(-21.0).warnings.len(), 1, "either direction");
    }

    #[test]
    fn scope_with_three_masters_and_one_instance() {
        let masters = vec![master("thin", "Thin", &[100.0]), master("regular", "Regular", &[400.0]), master("black", "Black", &[900.0])];
        let between = Instance { name: "Light".into(), location: vec![250.0], ..Instance::default() };
        let on_black = Instance { name: "Black".into(), location: vec![900.0], ..Instance::default() };
        let mut font = font_with(&[("Weight", "wght")], masters, vec![between, on_black], Vec::new());
        let ds = Designspace::new(&font).unwrap();
        assert_eq!(scope(&font, &ds), MasterPairs { masters: vec!["thin".into(), "regular".into()], indicators: true });

        Arc::make_mut(&mut font.info).params.set(ENFORCE_COMPATIBILITY_CHECK, Value::Bool(true));
        assert_eq!(scope(&font, &ds).masters, ["thin", "regular", "black"]);
        Arc::make_mut(&mut font.info).params.set(ENFORCE_COMPATIBILITY_CHECK, Value::Bool(false));
        assert_eq!(scope(&font, &ds), MasterPairs { masters: vec!["thin".into(), "regular".into()], indicators: false });

        Arc::make_mut(&mut font.info).params.remove(ENFORCE_COMPATIBILITY_CHECK);
        Arc::make_mut(&mut font.instances).push(Instance { name: "VF".into(), kind: Some("variable".into()), ..Instance::default() });
        assert_eq!(scope(&font, &ds).masters, ["thin", "regular", "black"], "a variable font needs every master");
    }

    #[test]
    fn many_settings_and_instances_finish_quickly() {
        // Thousands of settings over thousands of instances: building every setting's
        // designspace, each mapping its axis from every instance pair by pair, would not finish.
        let masters = vec![master("thin", "Thin", &[100.0]), master("regular", "Regular", &[400.0]), master("black", "Black", &[900.0])];
        let mut instances: Vec<Instance> =
            (0..2_000).map(|k| Instance { name: format!("Light {k}"), location: vec![250.0], ..Instance::default() }).collect();
        for k in 0..2_000 {
            let mut setting = Instance { name: format!("VF {k}"), kind: Some("variable".into()), ..Instance::default() };
            if k % 2 == 0 {
                setting.params.set(crate::params::DISABLE_MASTERS, Value::String(fontcraft_model::plist::Str::new("Black")));
            }
            instances.push(setting);
        }
        let font = font_with(&[("Weight", "wght")], masters, instances, Vec::new());
        let ds = Designspace::new(&font).unwrap();
        assert_eq!(scope(&font, &ds).masters, ["thin", "regular", "black"]);
    }
}
