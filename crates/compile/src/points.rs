//! The point map: from each source node to its exported TrueType point.
//!
//! Rows: `hinting.tt.cubicSourceQuadraticOutput`, `hinting.tt.export.overlapRemovalMovesHints`,
//! `hint.tt.getPointIndexes`. The map is built from the outline as the TrueType stage prepared it
//! (`outline.rs`: decomposition, overlap removal, cubic-to-quadratic conversion, direction), by
//! position, so it holds through every step that renumbers points:
//!
//! | Source node | Exported point |
//! |---|---|
//! | on-curve | the on-curve point at its place (within [`SAME_PLACE`]); with several, the one in the contour where most of its path's nodes landed. It is never left out as an implied point |
//! | on-curve, removed by overlap removal | the nearest on-curve point the removal created (the handbook's rule, as the editor's Remove Overlap applies it) |
//! | off-curve | the nearest off-curve point between the points of the on-curve nodes around it, on the shorter way round the contour |
//! | intersection of two segments | the created on-curve point nearest to where the two segments' chords cross |
//! | anything else | none: an instruction on it is left out with a warning |
//!
//! A glyph with instructions is written from these points ([`exported`]): the same points
//! `write-fonts` writes from the prepared path, except that an on-curve point a source node maps
//! to is kept even where it could be implied. Composites have no map of their own: their
//! components' glyphs carry the programs (guess on `hinting.tt.export.instanceValues`).

use std::collections::HashMap;

use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_hinting::{NodeRef, OutlinePoint, PointMap, TtInstruction, TtPoint};
use fontcraft_model::{Font, Layer, MAX_COMPONENT_DEPTH, ResolvedGlyph, Shape, Target};
use read_fonts::tables::glyf::CurvePoint;
use write_fonts::tables::glyf::{Contour, SimpleGlyph};

use crate::outline::{self, Flavour, Prepared};
use crate::{CompileError, DEFAULT_CONVERSION_ERROR, MAX_EXPANDED_ELEMENTS, MAX_TT_POINTS, Options};

/// How far, in font units, an exported point may lie from a source node and still be that
/// node: overlap removal computes the outline again in floating point.
pub const SAME_PLACE: f64 = 0.5;
/// Most (node, created point) distances one glyph's removed nodes and intersections may compare.
pub const MAX_NEAREST_WORK: usize = 4_000_000;

/// One glyph's exported point numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphPointMap {
    /// `paths[path][node]`: path index as [`NodeRef::path`] counts paths, nodes in the model's
    /// order.
    pub paths: Vec<Vec<Option<u16>>>,
    /// Points created by overlap removal at intersections: (segment ends, point).
    pub intersections: Vec<(NodeRef, NodeRef, u16)>,
    pub count: u16,
}

impl PointMap for GlyphPointMap {
    fn point(&self, node: NodeRef) -> Option<u16> {
        self.paths.get(node.path)?.get(node.node).copied().flatten()
    }

    fn point_count(&self) -> u16 {
        self.count
    }

    fn intersection(&self, a: NodeRef, b: NodeRef) -> Option<u16> {
        self.intersections.iter().find(|(x, y, _)| (*x, *y) == (a, b) || (*x, *y) == (b, a)).map(|(_, _, p)| *p)
    }
}

/// A simple glyph as exported, with its point map.
#[derive(Clone, Debug)]
pub struct Exported {
    /// The `glyf` entry, without instructions.
    pub glyph: SimpleGlyph,
    /// Its points, rounded, in `glyf` order.
    pub outline: Vec<OutlinePoint>,
    pub contour_ends: Vec<u16>,
    pub map: GlyphPointMap,
}

#[derive(Clone, Copy, Debug)]
struct Pt {
    p: Point,
    on: bool,
    contour: usize,
}

/// The nodes of `layer`'s paths (paths only, in shape order), as (position, on-curve).
fn source_nodes(layer: &Layer) -> Vec<Vec<(Point, bool)>> {
    layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Path(p) => Some(p.nodes.iter().map(|n| (n.point(), n.on_curve())).collect()),
            _ => None,
        })
        .collect()
}

/// The points `write-fonts` makes of a prepared path, before implied on-curve points are left
/// out; `None` for a path with a cubic segment or a segment before any move.
fn path_points(path: &BezPath) -> Option<Vec<Vec<(Point, bool)>>> {
    let mut contours = Vec::new();
    let mut current: Option<Vec<(Point, bool)>> = None;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                contours.extend(current.take());
                current = Some(vec![(p, true)]);
            }
            PathEl::LineTo(p) => current.as_mut()?.push((p, true)),
            PathEl::QuadTo(a, p) => {
                let c = current.as_mut()?;
                c.push((a, false));
                c.push((p, true));
            }
            PathEl::CurveTo(..) => return None,
            PathEl::ClosePath => {
                let c = current.as_mut()?;
                if c.len() > 1 && c.last() == c.first() {
                    c.pop();
                }
            }
        }
    }
    contours.extend(current);
    Some(contours)
}

fn round(v: f64) -> f64 {
    (v + 0.5).floor()
}

fn isclose(a: f64, b: f64) -> bool {
    a == b || (a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-9 * a.abs().max(b.abs()))
}

/// Can the on-curve point `i` of `c` be implied by its off-curve neighbours? (`write-fonts`'
/// test: halfway between them, before or after rounding.)
fn implied(c: &[(Point, bool)], i: usize) -> bool {
    let n = c.len();
    let (Some(&(p1, on)), Some(&(p0, on0)), Some(&(p2, on2))) = (c.get(i), c.get((i + n - 1) % n.max(1)), c.get((i + 1) % n.max(1))) else {
        return false;
    };
    if !on || on0 || on2 {
        return false;
    }
    let mid = p0.midpoint(p2);
    (isclose(mid.x, p1.x) && isclose(mid.y, p1.y))
        || (round(p0.x) + round(p2.x) == round(p1.x) * 2.0 && round(p0.y) + round(p2.y) == round(p1.y) * 2.0)
}

fn cell(p: Point) -> (i64, i64) {
    // Finite and within the TrueType coordinate range here (checked by the outline stage).
    #[allow(clippy::cast_possible_truncation)]
    ((p.x / SAME_PLACE).floor() as i64, (p.y / SAME_PLACE).floor() as i64)
}

/// The glyph `layer` (the built master's layer of the glyph) exports as `prepared`, with its
/// point map. `instructions` name the intersections the map needs; `overlaps_removed`: overlap
/// removal ran, so removed nodes move to created points. `None` for a glyph that is not a simple
/// TrueType outline.
pub fn exported(layer: &Layer, instructions: &[TtInstruction], prepared: &Prepared, overlaps_removed: bool) -> Option<Exported> {
    let (contours, droppable) = match prepared {
        Prepared::Simple(path) => (path_points(path)?, true),
        Prepared::Points(g) => {
            (g.contours.iter().map(|c| c.iter().map(|p| (Point::new(f64::from(p.x), f64::from(p.y)), p.on_curve)).collect()).collect(), false)
        }
        Prepared::Empty | Prepared::Composite(_) => return None,
    };
    let pts: Vec<Pt> = contours.iter().enumerate().flat_map(|(ci, c)| c.iter().map(move |&(p, on)| Pt { p, on, contour: ci })).collect();
    if pts.len() > MAX_TT_POINTS {
        return None;
    }
    // Where each contour starts in `pts`.
    let mut starts = Vec::with_capacity(contours.len());
    let mut at = 0usize;
    for c in &contours {
        starts.push(at);
        at += c.len();
    }
    let nodes = source_nodes(layer);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in pts.iter().enumerate().filter(|(_, p)| p.on) {
        grid.entry(cell(p.p)).or_default().push(i);
    }
    let near = |at: Point| -> Vec<usize> {
        let (cx, cy) = cell(at);
        let mut out: Vec<usize> = (-1..=1i64)
            .flat_map(|dx| (-1..=1i64).map(move |dy| (cx.saturating_add(dx), cy.saturating_add(dy))))
            .filter_map(|k| grid.get(&k))
            .flatten()
            .copied()
            .filter(|&i| pts.get(i).is_some_and(|p| p.p.distance(at) <= SAME_PLACE))
            .collect();
        out.sort_by(|&a, &b| {
            let d = |i: usize| pts.get(i).map_or(f64::INFINITY, |p| p.p.distance(at));
            d(a).total_cmp(&d(b))
        });
        out
    };

    // On-curve nodes at their place, in the contour most of their path's nodes landed in.
    let mut map: Vec<Vec<Option<usize>>> = nodes.iter().map(|p| vec![None; p.len()]).collect();
    let mut known = vec![false; pts.len()];
    for (pi, path) in nodes.iter().enumerate() {
        let candidates: Vec<Vec<usize>> = path.iter().map(|&(p, on)| if on { near(p) } else { Vec::new() }).collect();
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for c in &candidates {
            if let Some(&first) = c.first() {
                *votes.entry(pts.get(first).map_or(0, |p| p.contour)).or_default() += 1;
            }
        }
        let main = votes.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(c, _)| c);
        for (ni, c) in candidates.iter().enumerate() {
            let pick = c.iter().copied().find(|&i| pts.get(i).map(|p| p.contour) == main).or_else(|| c.first().copied());
            if let (Some(i), Some(slot)) = (pick, map.get_mut(pi).and_then(|m| m.get_mut(ni))) {
                *slot = Some(i);
                if let Some(k) = known.get_mut(i) {
                    *k = true;
                }
            }
        }
    }

    // Points overlap removal created: on-curve points no source node is at.
    let created: Vec<usize> = if overlaps_removed {
        (0..pts.len()).filter(|&i| pts.get(i).is_some_and(|p| p.on) && known.get(i) == Some(&false)).collect()
    } else {
        Vec::new()
    };
    let mut work = 0usize;
    let mut nearest_created = |at: Point| -> Option<usize> {
        work = work.saturating_add(created.len());
        if work > MAX_NEAREST_WORK {
            return None;
        }
        created.iter().copied().min_by(|&a, &b| {
            let d = |i: usize| pts.get(i).map_or(f64::INFINITY, |p| p.p.distance(at));
            d(a).total_cmp(&d(b))
        })
    };
    if overlaps_removed {
        for (pi, path) in nodes.iter().enumerate() {
            for (ni, &(p, on)) in path.iter().enumerate() {
                if on && map.get(pi).and_then(|m| m.get(ni)).copied().flatten().is_none() {
                    let found = nearest_created(p);
                    if let Some(slot) = map.get_mut(pi).and_then(|m| m.get_mut(ni)) {
                        *slot = found;
                    }
                }
            }
        }
    }

    // Off-curve nodes: the nearest off-curve point of their segment.
    for (pi, path) in nodes.iter().enumerate() {
        let n = path.len();
        for (ni, &(p, on)) in path.iter().enumerate() {
            if on {
                continue;
            }
            let on_at = |k: usize| path.get(k).is_some_and(|q| q.1);
            let prev = (1..n).map(|k| (ni + n - k) % n).find(|&k| on_at(k));
            let next = (1..n).map(|k| (ni + k) % n).find(|&k| on_at(k));
            let mapped = |k: Option<usize>| k.and_then(|k| map.get(pi).and_then(|m| m.get(k)).copied().flatten());
            let found = match (mapped(prev), mapped(next)) {
                (Some(a), Some(b)) => off_curve_between(&pts, &starts, a, b, p),
                _ => None,
            };
            if let Some(slot) = map.get_mut(pi).and_then(|m| m.get_mut(ni)) {
                *slot = found;
            }
        }
    }

    // Intersections the instructions name.
    let mut crossings: Vec<(NodeRef, NodeRef, usize)> = Vec::new();
    if overlaps_removed {
        let named = instructions.iter().flat_map(|t| t.points()).filter_map(|p| match p {
            TtPoint::Intersection { a, b } => Some((a, b)),
            _ => None,
        });
        for (a, b) in named {
            if crossings.iter().any(|(x, y, _)| (*x, *y) == (a, b)) {
                continue;
            }
            if let Some(i) = chord_crossing(&nodes, a, b).and_then(&mut nearest_created) {
                crossings.push((a, b, i));
            }
        }
    }

    // Leave out implied on-curve points no node maps to, then number what is left.
    let mut keep = vec![true; pts.len()];
    if droppable {
        let mut needed = vec![false; pts.len()];
        for i in map.iter().flatten().flatten().copied().chain(crossings.iter().map(|c| c.2)) {
            if let Some(k) = needed.get_mut(i) {
                *k = true;
            }
        }
        for (ci, c) in contours.iter().enumerate() {
            let start = starts.get(ci).copied().unwrap_or(0);
            for i in 0..c.len() {
                if implied(c, i)
                    && needed.get(start + i) == Some(&false)
                    && let Some(k) = keep.get_mut(start + i)
                {
                    *k = false;
                }
            }
        }
    }
    let mut number = vec![None; pts.len()];
    let mut next = 0u16;
    for (i, k) in keep.iter().enumerate() {
        if *k {
            if let Some(slot) = number.get_mut(i) {
                *slot = Some(next);
            }
            next = next.checked_add(1)?;
        }
    }
    let num = |i: usize| number.get(i).copied().flatten();

    let mut glyph_contours = Vec::with_capacity(contours.len());
    let mut outline = Vec::with_capacity(usize::from(next));
    let mut contour_ends = Vec::with_capacity(contours.len());
    for (ci, c) in contours.iter().enumerate() {
        let start = starts.get(ci).copied().unwrap_or(0);
        let mut points = Vec::with_capacity(c.len());
        for (i, &(p, on)) in c.iter().enumerate() {
            if keep.get(start + i) != Some(&true) {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let (x, y) = (round(p.x) as i16, round(p.y) as i16);
            points.push(CurvePoint::new(x, y, on));
            outline.push(OutlinePoint { x: f64::from(x), y: f64::from(y), on_curve: on });
        }
        if !points.is_empty() {
            contour_ends.push(u16::try_from(outline.len().checked_sub(1)?).ok()?);
        }
        glyph_contours.push(Contour::from(points));
    }
    let bbox = match prepared {
        Prepared::Simple(path) => path.control_box().into(),
        Prepared::Points(g) => g.bbox,
        _ => Default::default(),
    };
    let glyph = SimpleGlyph { bbox, contours: glyph_contours, instructions: Vec::new(), overlaps: false };
    let map = GlyphPointMap {
        paths: map.iter().map(|m| m.iter().map(|i| i.and_then(num)).collect()).collect(),
        intersections: crossings.into_iter().filter_map(|(a, b, i)| Some((a, b, num(i)?))).collect(),
        count: next,
    };
    Some(Exported { glyph, outline, contour_ends, map })
}

/// The off-curve point nearest `at` between points `a` and `b` of one contour, on the shorter
/// way round.
fn off_curve_between(pts: &[Pt], starts: &[usize], a: usize, b: usize, at: Point) -> Option<usize> {
    let (pa, pb) = (pts.get(a)?, pts.get(b)?);
    if pa.contour != pb.contour {
        return None;
    }
    let start = *starts.get(pa.contour)?;
    let len = pts.iter().skip(start).take_while(|p| p.contour == pa.contour).count();
    if len == 0 {
        return None;
    }
    let (la, lb) = (a - start, b - start);
    let forward = (lb + len - la) % len;
    let backward = (la + len - lb) % len;
    let steps: Vec<usize> = if forward <= backward {
        (1..forward).map(|k| start + (la + k) % len).collect()
    } else {
        (1..backward).map(|k| start + (la + len - k) % len).collect()
    };
    steps.into_iter().filter(|&i| pts.get(i).is_some_and(|p| !p.on)).min_by(|&x, &y| {
        let d = |i: usize| pts.get(i).map_or(f64::INFINITY, |p| p.p.distance(at));
        d(x).total_cmp(&d(y))
    })
}

/// Where the chords of the segments ending at nodes `a` and `b` cross; the middle of the two end
/// nodes when the chords are parallel (as the editor's Remove Overlap places it).
fn chord_crossing(nodes: &[Vec<(Point, bool)>], a: NodeRef, b: NodeRef) -> Option<Point> {
    let chord = |r: NodeRef| -> Option<(Point, Point)> {
        let path = nodes.get(r.path)?;
        let end = path.get(r.node)?.0;
        let len = path.len();
        let start = (1..len).map(|k| (r.node + len - k) % len).filter_map(|i| path.get(i)).find(|n| n.1)?.0;
        Some((start, end))
    };
    let (p1, p2) = chord(a)?;
    let (p3, p4) = chord(b)?;
    let d = (p1.x - p2.x) * (p3.y - p4.y) - (p1.y - p2.y) * (p3.x - p4.x);
    if d.abs() < 1e-9 {
        return Some(p2.midpoint(p4));
    }
    let t = ((p1.x - p3.x) * (p3.y - p4.y) - (p1.y - p3.y) * (p3.x - p4.x)) / d;
    let p = p1.lerp(p2, t);
    (p.x.is_finite() && p.y.is_finite()).then_some(p)
}

/// The exported point numbers of `glyph`'s nodes in the static TrueType font `options` make of
/// `font` at `target`: decomposition, overlap removal and curve conversion as the export runs
/// them on that glyph and the glyphs it uses. Export parameters that rename or decompose glyphs
/// are not applied. `None` for a glyph that exports as a composite or without an outline.
pub fn point_indexes(font: &Font, target: &Target, glyph: &str, options: &Options) -> Result<Option<GlyphPointMap>, CompileError> {
    let mut warnings = Vec::new();
    let (source, prepared) = crate::sources_at(font, target, &mut warnings)?;
    let resolved = crate::drawing::resolve(&prepared, target)?;
    // The glyph and every glyph it uses, so decomposition finds them.
    let mut wanted: Vec<&ResolvedGlyph> = Vec::new();
    let mut frontier = vec![(glyph.to_string(), 0usize)];
    while let Some((name, depth)) = frontier.pop() {
        if depth > MAX_COMPONENT_DEPTH || wanted.iter().any(|g| g.name == name) {
            continue;
        }
        let Some(g) = resolved.glyphs.iter().find(|g| g.name == name) else { continue };
        frontier.extend(g.components.iter().map(|c| (c.base.clone(), depth + 1)));
        wanted.push(g);
    }
    if wanted.first().is_none_or(|g| g.name != glyph) {
        return Err(CompileError::Glyph { glyph: glyph.to_string(), message: "the glyph is not in the font".to_string() });
    }
    let subset: Vec<ResolvedGlyph> = wanted.into_iter().cloned().collect();
    let error =
        if options.conversion_error.is_finite() && options.conversion_error > 0.0 { options.conversion_error } else { DEFAULT_CONVERSION_ERROR };
    let settings = outline::Settings {
        flavour: Flavour::TrueType,
        tolerance: f64::from(resolved.units_per_em) * error,
        keep_direction: options.keep_direction,
        max_expanded: MAX_EXPANDED_ELEMENTS,
        remove_overlap: options.remove_overlap,
        overlap_time_limit: options.overlap_time_limit,
    };
    let outlines = outline::prepare(&subset, &settings, &mut warnings)?;
    let Some(prepared) = outlines.first() else { return Ok(None) };
    let hint_settings = crate::tt_hints::settings_for(&source, &resolved, &mut warnings);
    let Some(g) = source.glyph(glyph) else { return Ok(None) };
    let built = hint_settings.as_ref().map_or(resolved.master.as_str(), |s| s.master.as_str());
    let Some(layer) = g.layer(built) else { return Ok(None) };
    let instructions = hint_settings
        .as_ref()
        .and_then(|s| g.layer(&s.main_master))
        .and_then(|l| fontcraft_hinting::tt_instructions(l).ok())
        .map(|v| v.into_iter().map(|(_, t)| t).collect::<Vec<_>>())
        .unwrap_or_default();
    Ok(exported(layer, &instructions, prepared, options.remove_overlap).map(|e| e.map))
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Node, NodeKind, Path};

    use super::*;

    fn layer_of(paths: Vec<Vec<(f64, f64, NodeKind)>>) -> Layer {
        let mut l = Layer::new("m01", 600.0);
        for p in paths {
            let nodes = p.into_iter().map(|(x, y, kind)| Node::new(x, y, kind)).collect();
            l.shapes.push(Shape::Path(Path::new(true, nodes)));
        }
        l
    }

    fn prepared(layer: &Layer, remove_overlap: bool) -> Prepared {
        let mut g = ResolvedGlyph::new("a", 600.0);
        g.contours = layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { p.to_contour().ok() } else { None }).collect();
        let s = outline::Settings {
            flavour: Flavour::TrueType,
            tolerance: 1.0,
            keep_direction: false,
            max_expanded: MAX_EXPANDED_ELEMENTS,
            remove_overlap,
            overlap_time_limit: std::time::Duration::from_secs(5),
        };
        outline::prepare(&[g], &s, &mut Vec::new()).unwrap().remove(0)
    }

    fn at(e: &Exported, i: u16) -> (f64, f64, bool) {
        let p = e.outline[usize::from(i)];
        (p.x, p.y, p.on_curve)
    }

    #[test]
    fn cubic_nodes_find_their_quadratic_points_and_uninstructed_points_match_write_fonts() {
        use NodeKind::{Curve, OffCurve as Off};
        // An o: four smooth cubic segments, whose on-curve points cu2qu would let be implied.
        let layer = layer_of(vec![vec![
            (300.0, 0.0, Curve),
            (466.0, 0.0, Off),
            (600.0, 134.0, Off),
            (600.0, 300.0, Curve),
            (600.0, 466.0, Off),
            (466.0, 600.0, Off),
            (300.0, 600.0, Curve),
            (134.0, 600.0, Off),
            (0.0, 466.0, Off),
            (0.0, 300.0, Curve),
            (0.0, 134.0, Off),
            (134.0, 0.0, Off),
        ]]);
        let prep = prepared(&layer, false);
        let e = exported(&layer, &[], &prep, false).unwrap();
        for (ni, node) in
            layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).flat_map(|p| p.nodes.iter()).enumerate()
        {
            let i = e.map.point(NodeRef::new(0, ni)).unwrap_or_else(|| panic!("node {ni} has no point"));
            let (x, y, on) = at(&e, i);
            assert_eq!(on, node.on_curve(), "node {ni}");
            if on {
                assert_eq!((x, y), (node.x, node.y), "on-curve node {ni} keeps its place");
            } else {
                assert!((x - node.x).abs() < 120.0 && (y - node.y).abs() < 120.0, "off-curve node {ni} maps near itself: {x},{y}");
            }
        }
        // Every on-curve node is kept, though write-fonts would imply some.
        let Prepared::Simple(path) = &prep else { panic!() };
        let plain = SimpleGlyph::from_bezpath(path).unwrap();
        assert!(e.map.count as usize >= plain.contours.iter().map(|c| c.len()).sum::<usize>());
        // A glyph whose nodes need nothing kept writes what write-fonts writes.
        let none = exported(&Layer::new("m01", 600.0), &[], &prep, false).unwrap();
        assert_eq!(none.glyph.contours, plain.contours);
    }

    #[test]
    fn overlap_removal_moves_removed_nodes_to_created_points() {
        use NodeKind::Line;
        // Two overlapping squares; node (1, 3) of the second lies inside the first.
        let layer = layer_of(vec![
            vec![(0.0, 0.0, Line), (200.0, 0.0, Line), (200.0, 200.0, Line), (0.0, 200.0, Line)],
            vec![(100.0, 100.0, Line), (300.0, 100.0, Line), (300.0, 300.0, Line), (100.0, 300.0, Line)],
        ]);
        let prep = prepared(&layer, true);
        let crossing = TtInstruction {
            kind: fontcraft_hinting::TtKind::Snap { zone: fontcraft_hinting::ZoneRef::NoZone },
            direction: fontcraft_hinting::Direction::Vertical,
            rounding: fontcraft_hinting::Rounding::Round,
            target: TtPoint::Intersection { a: NodeRef::new(0, 2), b: NodeRef::new(1, 1) },
            origin: None,
            other: None,
            source: None,
        };
        let e = exported(&layer, std::slice::from_ref(&crossing), &prep, true).unwrap();
        assert_eq!(e.map.count, 8, "the union has eight corners");
        let inside = e.map.point(NodeRef::new(1, 0)).unwrap();
        let (x, y, _) = at(&e, inside);
        assert!((x, y) == (200.0, 100.0) || (x, y) == (100.0, 200.0), "the removed corner moves to a crossing: {x},{y}");
        let kept = e.map.point(NodeRef::new(1, 2)).unwrap();
        assert_eq!(at(&e, kept), (300.0, 300.0, true));
        let cross = e.map.intersection(NodeRef::new(0, 2), NodeRef::new(1, 1)).unwrap();
        assert_eq!(at(&e, cross), (200.0, 100.0, true));
    }
}
