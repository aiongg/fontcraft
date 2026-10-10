//! `selection.move`: moving and duplicating what a target object names (decision 0004 and
//! `docs/spec/03a` § 7, § 11).
//!
//! | Mode | Nodes | Handles of moved nodes | Selected handles alone |
//! |---|---|---|---|
//! | `normal` | move | move with them | move; the opposite handle of a smooth node turns to stay in line (along a straight side, the handle slides on its line) |
//! | `pointsOnly` | move | stay | keep their angle: only the length changes |
//! | `nudge` | move | the handles of each segment with one moved end are scaled per axis between the fixed end and the moved one | move |

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use fontcraft_geom::{Point, Vec2, round_point, round_to_grid};
use fontcraft_model::{Component, Font, LayerParts, LayerPartsMut, Node, Path, Shape, Transform};

use super::ops::{EPS, handle_owner, next, on, prev};
use crate::params::{GuideRef, GuideScope, Targets};
use crate::{EngineError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Normal,
    PointsOnly,
    Nudge,
}

/// Most copies of an anchor name tried when a duplicate needs a free name.
const MAX_NAME_TRIES: usize = 10_000;

/// The target's points grouped by shape, with each segment's two end nodes added, leaving out
/// shapes that are moved whole.
pub(super) fn points_by_shape(t: &Targets, parts: LayerParts) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut out = super::group(&t.points);
    for (s, ends) in super::group(&t.segments) {
        let Some(path) = parts.path(s) else { continue };
        for seg in path.segments().iter().filter(|g| ends.contains(&g.end)) {
            out.entry(s).or_default().extend([seg.start, seg.end]);
        }
    }
    out.retain(|s, _| !t.shapes.contains(s));
    out
}

fn at(v: &[Point], i: usize) -> Point {
    v.get(i).copied().unwrap_or_default()
}

fn mark(new: &mut [Point], moved: &mut [bool], i: usize, by: Vec2) {
    if let (Some(p), Some(m)) = (new.get_mut(i), moved.get_mut(i))
        && !*m
    {
        *p += by;
        *m = true;
    }
}

/// Moves the nodes `selected` of `path` by `d` as `mode` says.
pub(super) fn move_path(path: &mut Path, selected: &BTreeSet<usize>, d: Vec2, mode: Mode, step: f64) {
    let old: Vec<Point> = path.nodes.iter().map(Node::point).collect();
    let mut new = old.clone();
    let mut moved = vec![false; old.len()];
    let path_ref: &Path = path;
    match mode {
        Mode::Normal => {
            for &i in selected {
                mark(&mut new, &mut moved, i, d);
            }
            for &i in selected.iter().filter(|&&i| on(path_ref, i)) {
                for j in [prev(path_ref, i), next(path_ref, i)].into_iter().flatten() {
                    if !on(path_ref, j) {
                        mark(&mut new, &mut moved, j, d);
                    }
                }
            }
            let handles: Vec<usize> = (0..old.len()).filter(|&j| moved.get(j).copied().unwrap_or(false) && !on(path_ref, j)).collect();
            for j in handles {
                keep_smooth(path_ref, j, &old, &mut new, &mut moved);
            }
        }
        Mode::PointsOnly => {
            for &i in selected {
                let mut by = d;
                if let Some((k, _)) = handle_owner(path_ref, i)
                    && !selected.contains(&k)
                {
                    let v = at(&old, i) - at(&old, k);
                    let len = v.hypot();
                    if len > EPS {
                        let u = v / len;
                        by = u * d.dot(u);
                    }
                }
                mark(&mut new, &mut moved, i, by);
            }
        }
        Mode::Nudge => {
            for &i in selected {
                mark(&mut new, &mut moved, i, d);
            }
            for seg in path_ref.segments() {
                let (ms, me) = (selected.contains(&seg.start), selected.contains(&seg.end));
                if ms && me {
                    for &h in &seg.handles {
                        mark(&mut new, &mut moved, h, d);
                    }
                    continue;
                }
                if !(ms || me) {
                    continue;
                }
                let (m, f) = if ms { (seg.start, seg.end) } else { (seg.end, seg.start) };
                let (pm, pf) = (at(&old, m), at(&old, f));
                for &h in seg.handles.iter().filter(|h| !selected.contains(*h)) {
                    let attached = prev(path_ref, h) == Some(m) || next(path_ref, h) == Some(m);
                    let ph = at(&old, h);
                    let axis = |h: f64, m: f64, f: f64, d: f64| {
                        let span = m - f;
                        if span.abs() > EPS {
                            f + (h - f) * (span + d) / span
                        } else if attached {
                            h + d
                        } else {
                            h
                        }
                    };
                    let to = Point::new(axis(ph.x, pm.x, pf.x, d.x), axis(ph.y, pm.y, pf.y, d.y));
                    if let (Some(p), Some(mv)) = (new.get_mut(h), moved.get_mut(h)) {
                        *p = to;
                        *mv = true;
                    }
                }
            }
        }
    }
    for ((node, p), m) in path.nodes.iter_mut().zip(&new).zip(&moved) {
        if *m {
            let r = round_point(*p, step);
            (node.x, node.y) = (r.x, r.y);
        }
    }
}

/// After handle `j` moved: when its node is smooth and stays, the handle on the other side turns
/// to stay in line with it, keeping its length; when that side is straight, `j` slides along the
/// line instead.
fn keep_smooth(path: &Path, j: usize, old: &[Point], new: &mut [Point], moved: &mut [bool]) {
    let Some((k, Some(o))) = handle_owner(path, j) else { return };
    let is_moved = |i: usize| moved.get(i).copied().unwrap_or(false);
    if is_moved(k) || is_moved(o) || !path.nodes.get(k).is_some_and(|n| n.smooth) {
        return;
    }
    let pk = at(old, k);
    if on(path, o) {
        let u = pk - at(old, o);
        let len = u.hypot();
        if len <= EPS {
            return;
        }
        let u = u / len;
        if let Some(p) = new.get_mut(j) {
            *p = pk + u * (*p - pk).dot(u).max(0.0);
        }
    } else {
        let len_o = (at(old, o) - pk).hypot();
        let dir = pk - at(new, j);
        let l = dir.hypot();
        if l <= EPS {
            return;
        }
        if let (Some(p), Some(m)) = (new.get_mut(o), moved.get_mut(o)) {
            *p = pk + dir * (len_o / l);
            *m = true;
        }
    }
}

fn translate_path(path: &mut Path, d: Vec2, step: f64) {
    for n in &mut path.nodes {
        let r = round_point(n.point() + d, step);
        (n.x, n.y) = (r.x, r.y);
    }
}

fn translate_component(c: &mut Component, d: Vec2, step: f64) {
    match &mut c.transform {
        Transform::Decomposed { pos, .. } => {
            pos.0 = round_to_grid(pos.0 + d.x, step);
            pos.1 = round_to_grid(pos.1 + d.y, step);
        }
        Transform::Matrix(m) => {
            if let Some(x) = m.get_mut(4) {
                *x = round_to_grid(*x + d.x, step);
            }
            if let Some(y) = m.get_mut(5) {
                *y = round_to_grid(*y + d.y, step);
            }
        }
    }
}

fn translate_shape(shape: &mut Shape, d: Vec2, step: f64) {
    match shape {
        Shape::Path(p) => translate_path(p, d, step),
        Shape::Component(c) => translate_component(c, d, step),
        Shape::Image(_) | Shape::Raw(_) => {}
    }
}

fn shift(x: &mut f64, y: &mut f64, d: Vec2, step: f64) {
    *x = round_to_grid(*x + d.x, step);
    *y = round_to_grid(*y + d.y, step);
}

/// A free anchor name based on `name` (`top.1`, `top.2`…).
fn free_name(taken: &[String], name: &str) -> Option<String> {
    (1..=MAX_NAME_TRIES).map(|k| format!("{name}.{k}")).find(|c| !taken.iter().any(|t| t == c))
}

/// Moves (or, with `duplicate`, copies and moves the copies of) what `t` names in `font` by
/// `d`. Returns the selection afterwards: the same targets, or the copies.
pub(super) fn apply(font: &mut Font, t: &Targets, d: Vec2, mode: Mode, duplicate: bool, step: f64) -> Result<Targets> {
    let (master, points) = {
        let r = t.resolve(font)?;
        (r.master.id.clone(), points_by_shape(t, r.parts))
    };
    let mut out = t.clone();
    let layer_part = t.edit_layer(font, |layer| {
        let mut parts = layer.parts_mut(t.side);
        if duplicate {
            Ok(Some(duplicate_in(&mut parts, t, &points, d, step)))
        } else {
            move_in(&mut parts, t, &points, d, mode, step).map(|()| None)
        }
    })?;
    let global: Vec<usize> = t.guides.iter().filter(|g| g.scope == GuideScope::Global).map(|g| g.index).collect();
    let mut new_global = Vec::new();
    if !global.is_empty() {
        let masters = Arc::make_mut(&mut font.masters);
        let m = masters.iter_mut().find(|m| m.id == master).ok_or_else(|| EngineError::Internal(format!("master `{master}` went missing")))?;
        for i in global {
            let Some(mut g) = m.guides.get(i).cloned() else { continue };
            shift(&mut g.x, &mut g.y, d, step);
            if duplicate {
                new_global.push(GuideRef { scope: GuideScope::Global, index: m.guides.len() });
                m.guides.push(g);
            } else if let Some(slot) = m.guides.get_mut(i) {
                *slot = g;
            }
        }
    }
    if let Some(copies) = layer_part {
        out.segments.clear();
        out.points = copies.points;
        out.shapes = copies.shapes;
        out.anchors = copies.anchors;
        out.guides = copies.guides;
        out.guides.extend(new_global);
        out.annotations = copies.annotations;
    }
    Ok(out)
}

fn move_in(parts: &mut LayerPartsMut, t: &Targets, points: &BTreeMap<usize, BTreeSet<usize>>, d: Vec2, mode: Mode, step: f64) -> Result<()> {
    for (&s, set) in points {
        if let Some(path) = parts.path_mut(s) {
            move_path(path, set, d, mode, step);
        }
    }
    for &s in &t.shapes {
        if let Some(shape) = parts.shapes.get_mut(s) {
            translate_shape(shape, d, step);
        }
    }
    for a in parts.anchors.iter_mut().filter(|a| t.anchors.contains(&a.name)) {
        shift(&mut a.x, &mut a.y, d, step);
    }
    for g in t.guides.iter().filter(|g| g.scope == GuideScope::Local) {
        if let Some(guide) = parts.guides.get_mut(g.index) {
            shift(&mut guide.x, &mut guide.y, d, step);
        }
    }
    for &i in &t.annotations {
        if let Some(a) = parts.annotations.get_mut(i) {
            shift(&mut a.x, &mut a.y, d, step);
        }
    }
    Ok(())
}

/// The new lists of a duplicate: what the shell selects afterwards.
struct Copies {
    points: Vec<(usize, usize)>,
    shapes: Vec<usize>,
    anchors: Vec<String>,
    guides: Vec<GuideRef>,
    annotations: Vec<usize>,
}

fn duplicate_in(parts: &mut LayerPartsMut, t: &Targets, points: &BTreeMap<usize, BTreeSet<usize>>, d: Vec2, step: f64) -> Copies {
    let mut new_shapes: Vec<Shape> = Vec::new();
    for &s in &t.shapes {
        if let Some(shape) = parts.shapes.get(s) {
            let mut copy = shape.clone();
            if let Shape::Path(p) = &mut copy
                && p.info.identifier.is_some()
            {
                p.info.identifier = None;
            }
            new_shapes.push(copy);
        }
    }
    for (&s, set) in points {
        let Some(Shape::Path(path)) = parts.shapes.get(s) else { continue };
        if (0..path.nodes.len()).all(|i| set.contains(&i)) {
            let mut whole = path.clone();
            if whole.info.identifier.is_some() {
                whole.info.identifier = None;
            }
            new_shapes.push(Shape::Path(whole));
        } else {
            new_shapes.extend(super::ops::selected_segments(path, set).into_iter().map(Shape::Path));
        }
    }
    let first = parts.shapes.len();
    let mut copies = Copies { points: Vec::new(), shapes: Vec::new(), anchors: Vec::new(), guides: Vec::new(), annotations: Vec::new() };
    for (k, mut shape) in new_shapes.into_iter().enumerate() {
        translate_shape(&mut shape, d, step);
        let index = first + k;
        match &shape {
            Shape::Path(p) => copies.points.extend((0..p.nodes.len()).map(|n| (index, n))),
            _ => copies.shapes.push(index),
        }
        parts.shapes.push(shape);
    }
    let mut taken: Vec<String> = parts.anchors.iter().map(|a| a.name.clone()).collect();
    for name in &t.anchors {
        let Some(a) = parts.anchors.iter().find(|a| a.name == *name) else { continue };
        let Some(fresh) = free_name(&taken, name) else { continue };
        let mut copy = a.clone();
        copy.name = fresh.clone();
        shift(&mut copy.x, &mut copy.y, d, step);
        taken.push(fresh.clone());
        copies.anchors.push(fresh);
        parts.anchors.push(copy);
    }
    for g in t.guides.iter().filter(|g| g.scope == GuideScope::Local) {
        let Some(mut copy) = parts.guides.get(g.index).cloned() else { continue };
        shift(&mut copy.x, &mut copy.y, d, step);
        copies.guides.push(GuideRef { scope: GuideScope::Local, index: parts.guides.len() });
        parts.guides.push(copy);
    }
    for &i in &t.annotations {
        let Some(mut copy) = parts.annotations.get(i).cloned() else { continue };
        shift(&mut copy.x, &mut copy.y, d, step);
        copies.annotations.push(parts.annotations.len());
        parts.annotations.push(copy);
    }
    copies
}
