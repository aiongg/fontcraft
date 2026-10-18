//! The Master Compatibility view (`view.showMasterCompatibility`, panel
//! `editView.masterCompatibility`), Edit View's red bar over incompatible glyphs
//! (`compatibility.editViewRedBar`) and Keep Layer Selection in Sync
//! (`edit.keepLayerSelectionInSync`).
//!
//! Rows: `view.showMasterCompatibility`, `editView.masterCompatibility`, `masterCompatibility.*`,
//! `compatibility.editViewRedBar`, `edit.keepLayerSelectionInSync`.
//!
//! The view draws every source layer of the edited glyph beside the edited layer, which stays
//! where it is: the main sources (master layers in master order, then intermediate layers), then
//! after a wider gap each alternate set ([`placement`]). Each layer starts where the one before
//! it ends plus a gap and sits a little higher, so the lines joining corresponding points run on
//! a slant. Layers are compared as `fontcraft_interp::compat` compares them: shapes are the paths
//! and components (images left out), matched by index within a group of layers.
//!
//! On every layer each shape is tinted by its index (components with a dashed tint) and each
//! path segment is stroked green, yellow (its chord turns by more than the threshold against a
//! layer of its group) or red (no counterpart, or nodes that differ from that node on)
//! ([`segment_states`]). Lines join start nodes, component centres, anchors (dashed) and the
//! selected points across a group; a missing element's line ends at that layer's (0, 0)
//! ([`links`]). The edited layer shows each shape's index. Component and anchor names that differ
//! within a group are written in red. Dragging a line end on another layer onto the end of
//! another shape's line on that layer runs `glyph.swapShapes` there ([`drop_swap`]).

use std::collections::{HashMap, HashSet};

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use fontcraft_engine::interp::{ANGLE_WARNING_DEGREES, Designspace, GlyphSources};
use fontcraft_engine::model::{Font, Glyph, Layer, Shape as ModelShape};
use fontcraft_geom::Shape as _;
use fontcraft_geom::{Affine, Point, segments};
use fontcraft_textlayout::Layout;
use fontcraft_tools::Selection;
use serde_json::{Value, json};

use super::{Painting, Scene, flatten, view};
use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};

/// Most layers the view draws.
pub const MAX_PLACED: usize = 64;
/// Most shapes per layer, anchors per group and selected points the view joins with lines.
pub const MAX_LINKED: usize = 1_024;
/// Most glyphs of a tab checked for the red bar each frame.
pub const MAX_BAR_GLYPHS: usize = 2_000;
/// The gap after each layer, the rise from one layer to the next and the extra gap before an
/// alternate set, as shares of the em.
const GAP_EM: f64 = 0.25;
const RISE_EM: f64 = 0.08;
const GROUP_GAP_EM: f64 = 0.75;
/// Points sampled along each curve piece of a coloured segment.
const SEGMENT_SAMPLES: usize = 12;

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.showMasterCompatibility",
        "Show Master Compatibility",
        &["View"],
        "Cmd+Ctrl+Alt+N",
        "{on?: bool} draw every master, intermediate and alternate layer of the edited glyph beside it: shapes tinted by their index, lines joining start nodes, component centres, anchors and selected points across the layers, segments green (compatible), yellow (turning more than 20°) or red (incompatible). Dragging a line end on another layer onto another shape's line end there swaps the two shapes (glyph.swapShapes) (omitted: toggle) → {on}",
    ),
    (
        "edit.keepLayerSelectionInSync",
        "Keep Layer Selection in Sync",
        &["Edit"],
        "",
        "{on?: bool} while on, editing another master of a compatible glyph keeps the selected nodes, shapes and anchors, matched by index (omitted: toggle) → {on}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let display = &mut app.state.display;
    let flag = match id {
        "view.showMasterCompatibility" => &mut display.master_compatibility,
        "edit.keepLayerSelectionInSync" => &mut display.keep_layer_selection_in_sync,
        other => return Err(format!("unknown command `{other}`")),
    };
    let on = args.toggle(*flag)?;
    *flag = on;
    Ok(json!({"on": on}))
}

// ── Keep Layer Selection in Sync ────────────────────────────────────────────────────────────────

/// Carries the selection over to the newly active layer when Keep Layer Selection in Sync is on,
/// the glyph stayed the same, both layers are master layers and the glyph is compatible. Items
/// the new layer does not have are dropped; guides and annotations are not carried. Runs before
/// the tab drops a selection that belongs to another layer.
pub fn carry_selection(app: &mut FontcraftApp) {
    if !app.state.display.keep_layer_selection_in_sync {
        return;
    }
    let Some(tab) = app.state.tab() else { return };
    let (Some(glyph), Some(old)) = (tab.selection_glyph.clone(), tab.selection_layer.clone()) else { return };
    if tab.edited_glyph() != Some(glyph.as_str()) || tab.selection.is_empty() {
        return;
    }
    let Ok(font) = app.session.font() else { return };
    let Some(new) = view::active_layer(font, tab) else { return };
    if new.id == old {
        return;
    }
    let Some(g) = font.glyph(&glyph) else { return };
    if !g.layers.iter().any(|l| l.id == old && l.is_master_layer()) || !new.is_master_layer() {
        return;
    }
    let carried = carried_selection(&tab.selection, new);
    let new_id = new.id.clone();
    if !app.session.glyph_compat(&glyph).is_some_and(|c| c.ok) {
        return;
    }
    if let Some(tab) = app.state.tab_mut() {
        tab.selection = carried;
        tab.selection_layer = Some(new_id);
    }
}

/// The part of `selection` that resolves on `layer`, by index (anchors by name).
pub fn carried_selection(selection: &Selection, layer: &Layer) -> Selection {
    Selection {
        points: selection
            .points
            .iter()
            .copied()
            .filter(|&(s, n)| matches!(layer.shapes.get(s), Some(ModelShape::Path(p)) if n < p.nodes.len()))
            .collect(),
        shapes: selection.shapes.iter().copied().filter(|&s| s < layer.shapes.len()).collect(),
        anchors: selection.anchors.iter().filter(|a| layer.anchors.iter().any(|x| x.name == **a)).cloned().collect(),
        ..Selection::default()
    }
}

// ── The red bar ─────────────────────────────────────────────────────────────────────────────────

/// The glyphs of the showing tab that show the incompatibility indicators.
pub fn incompatible_in_tab(app: &mut FontcraftApp) -> HashSet<String> {
    let Some(tab) = app.state.tab() else { return HashSet::new() };
    let names: HashSet<String> = tab.shown_glyphs().take(MAX_BAR_GLYPHS).map(str::to_string).collect();
    if names.is_empty() || !app.session.compat_indicators() {
        return HashSet::new();
    }
    names.into_iter().filter(|n| app.session.shows_incompatible(n)).collect()
}

/// The red bar above the ascender of each glyph of the layout in `incompatible`. `origin` is
/// where the edited glyph sits in the layout.
pub fn draw_bars(c: &Painting, font: &Font, layout: Option<&Layout>, origin: [f64; 2], incompatible: &HashSet<String>) {
    let Some(layout) = layout else { return };
    if incompatible.is_empty() {
        return;
    }
    let t = c.tokens;
    let lift = f64::from(t.compat_bar_height) / c.view.scale().max(f64::MIN_POSITIVE);
    let ascender = layout.ascender;
    for placed in &layout.placed {
        let Some(name) = placed.glyph.as_deref() else { continue };
        if !incompatible.contains(name) || font.glyph(name).is_none() {
            continue;
        }
        let (x, y) = (placed.x - origin[0], placed.y - origin[1] + ascender);
        let a = c.view.to_screen(Point::new(x, y + lift));
        let b = c.view.to_screen(Point::new(x + placed.advance.max(0.0), y + 2.0 * lift));
        let rect = Rect::from_two_pos(pos(a), pos(b));
        if rect.is_finite() && rect.intersects(c.rect) {
            c.painter.rect_filled(rect, 0.0, t.compat_bad);
        }
    }
}

// ── The view's layout ───────────────────────────────────────────────────────────────────────────

/// One layer the view draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub layer: String,
    /// Where the layer is drawn, relative to the edited layer, in font units.
    pub offset: (f64, f64),
    /// 0 for the main sources, then one group per alternate set.
    pub group: usize,
}

/// The layers the view draws for `glyph`, in order, the edited layer `current` at (0, 0). Without
/// sources (no usable designspace) the master layers are drawn, in master order. A current layer
/// that is no source (a backup layer) is drawn first in the main group.
pub fn placement(font: &Font, glyph: &Glyph, current: &Layer, sources: Option<&GlyphSources>) -> Vec<Placed> {
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
    let mut placed = Vec::with_capacity(order.len());
    for (k, (id, group)) in order.into_iter().enumerate() {
        if k > 0 && placed.last().is_some_and(|p: &Placed| p.group != group) {
            x += GROUP_GAP_EM * em;
        }
        placed.push(Placed { layer: id.clone(), offset: (x, k as f64 * RISE_EM * em), group });
        let width = glyph.layers.iter().find(|l| l.id == id).map_or(0.0, |l| if l.width.is_finite() { l.width.max(0.0) } else { 0.0 });
        x += width + GAP_EM * em;
    }
    if let Some((cx, cy)) = placed.iter().find(|p| p.layer == current.id).map(|p| p.offset) {
        for p in &mut placed {
            p.offset = (p.offset.0 - cx, p.offset.1 - cy);
        }
    }
    placed
}

/// The layer.shapes indices of a layer's outline shapes (paths, components and shapes kept
/// unread; images left out), in order: outline index → layer index.
pub fn outline_indices(layer: &Layer) -> Vec<usize> {
    layer.shapes.iter().enumerate().filter(|(_, s)| !matches!(s, ModelShape::Image(_))).map(|(i, _)| i).collect()
}

fn outline_shape(layer: &Layer, index: usize) -> Option<&ModelShape> {
    outline_indices(layer).get(index).and_then(|&i| layer.shapes.get(i))
}

// ── Connecting lines ────────────────────────────────────────────────────────────────────────────

/// What a connecting line joins. Shape indices are outline indices; a selected point is
/// `(outline index, node)`.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkKind {
    /// A path's start node (its first on-curve node).
    Start(usize),
    /// A component's centre.
    Centre(usize),
    Anchor(String),
    Point(usize, usize),
}

/// One end of a connecting line, on one placed layer.
#[derive(Clone, Debug, PartialEq)]
pub struct End {
    /// Index into the placement.
    pub placed: usize,
    /// In the edited layer's font units, the layer's offset included.
    pub at: Point,
    /// The layer lacks the element: the end is that layer's (0, 0).
    pub missing: bool,
    /// The component's base glyph or the anchor's name, written in red when the group's layers
    /// disagree on it.
    pub mismatch: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub kind: LinkKind,
    pub ends: Vec<End>,
}

/// The connecting lines of every group. `layers` are the placed layers, parallel to `placed`;
/// `current` indexes the edited layer, whose `selection` adds lines for its selected points.
pub fn links(font: &Font, placed: &[Placed], layers: &[&Layer], current: usize, selection: &Selection) -> Vec<Link> {
    let mut out = Vec::new();
    let groups: Vec<usize> = {
        let mut g: Vec<usize> = placed.iter().map(|p| p.group).collect();
        g.dedup();
        g
    };
    for group in groups {
        let members: Vec<usize> = (0..placed.len().min(layers.len())).filter(|&i| placed.get(i).is_some_and(|p| p.group == group)).collect();
        let shift = |i: usize, p: Point| placed.get(i).map_or(p, |pl| Point::new(p.x + pl.offset.0, p.y + pl.offset.1));
        let layer = |i: usize| layers.get(i).copied();
        let count = members.iter().filter_map(|&i| layer(i)).map(|l| outline_indices(l).len()).max().unwrap_or(0).min(MAX_LINKED);
        for s in 0..count {
            let shapes: Vec<Option<&ModelShape>> = members.iter().map(|&i| layer(i).and_then(|l| outline_shape(l, s))).collect();
            let is_component = shapes.iter().flatten().next().is_some_and(|x| matches!(x, ModelShape::Component(_)));
            let bases: Vec<Option<&str>> = shapes
                .iter()
                .map(|x| match x {
                    Some(ModelShape::Component(c)) => Some(c.glyph.as_str()),
                    _ => None,
                })
                .collect();
            let disagree = is_component && bases.windows(2).any(|w| matches!(w, [a, b] if a != b));
            let mut ends = Vec::with_capacity(members.len());
            for (k, &i) in members.iter().enumerate() {
                let master = layer(i).map_or("", |l| l.master.as_str());
                let at = match shapes.get(k).copied().flatten() {
                    Some(ModelShape::Path(p)) => p.nodes.iter().find(|n| n.on_curve()).map(|n| n.point()),
                    Some(ModelShape::Component(c)) => Some(component_centre(font, c, master)),
                    _ => None,
                };
                let mismatch = if disagree { bases.get(k).copied().flatten().map(str::to_string) } else { None };
                ends.push(End { placed: i, at: shift(i, at.unwrap_or(Point::ZERO)), missing: at.is_none(), mismatch });
            }
            out.push(Link { kind: if is_component { LinkKind::Centre(s) } else { LinkKind::Start(s) }, ends });
        }
        let mut names: Vec<&str> = Vec::new();
        for &i in &members {
            for a in layer(i).map(|l| l.anchors.as_slice()).unwrap_or_default() {
                if names.len() < MAX_LINKED && !names.contains(&a.name.as_str()) {
                    names.push(&a.name);
                }
            }
        }
        for name in names {
            let found: Vec<Option<Point>> =
                members.iter().map(|&i| layer(i).and_then(|l| l.anchors.iter().find(|a| a.name == name)).map(|a| Point::new(a.x, a.y))).collect();
            let disagree = found.iter().any(Option::is_none);
            let ends = members
                .iter()
                .zip(&found)
                .map(|(&i, at)| End {
                    placed: i,
                    at: shift(i, at.unwrap_or(Point::ZERO)),
                    missing: at.is_none(),
                    mismatch: (disagree && at.is_some()).then(|| name.to_string()),
                })
                .collect();
            out.push(Link { kind: LinkKind::Anchor(name.to_string()), ends });
        }
        if members.contains(&current)
            && let Some(cur) = layer(current)
        {
            let to_outline = outline_indices(cur);
            for &(s, n) in selection.points.iter().take(MAX_LINKED) {
                let Some(o) = to_outline.iter().position(|&i| i == s) else { continue };
                let ends = members
                    .iter()
                    .map(|&i| {
                        let at = layer(i).and_then(|l| match outline_shape(l, o) {
                            Some(ModelShape::Path(p)) => p.nodes.get(n).map(|x| x.point()),
                            _ => None,
                        });
                        End { placed: i, at: shift(i, at.unwrap_or(Point::ZERO)), missing: at.is_none(), mismatch: None }
                    })
                    .collect();
                out.push(Link { kind: LinkKind::Point(o, n), ends });
            }
        }
    }
    out
}

/// The centre of a component's outline, or its offset when it has none.
fn component_centre(font: &Font, c: &fontcraft_engine::model::Component, master: &str) -> Point {
    let fallback = c.affine() * Point::ZERO;
    match font.component_outline(c, master) {
        Ok(path) if !path.elements().is_empty() => {
            let b = path.bounding_box();
            let centre = b.center();
            if centre.x.is_finite() && centre.y.is_finite() { centre } else { fallback }
        }
        _ => fallback,
    }
}

/// The `glyph.swapShapes` parameters for dropping the end `from` of a shape line on the end of
/// another shape line on the same layer, nearest to `at` (font units, offset included) within
/// `reach`; `None` when no such end is near or the layer is the edited one.
pub fn drop_swap(glyph: &str, layers: &[&Layer], links: &[Link], from: (usize, usize), at: Point, reach: f64, current: usize) -> Option<Value> {
    let (link, end) = from;
    let source = links.get(link)?;
    let a = shape_of(&source.kind)?;
    let on = source.ends.get(end)?.placed;
    if on == current {
        return None;
    }
    let mut best: Option<(f64, usize)> = None;
    for other in links {
        let Some(b) = shape_of(&other.kind) else { continue };
        if b == a {
            continue;
        }
        let Some(e) = other.ends.iter().find(|e| e.placed == on && !e.missing) else { continue };
        let d = (e.at - at).hypot();
        if d <= reach && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, b));
        }
    }
    let (_, b) = best?;
    let layer = layers.get(on)?;
    let map = outline_indices(layer);
    let (la, lb) = (*map.get(a)?, *map.get(b)?);
    Some(json!({"glyph": glyph, "layer": layer.id, "a": la, "b": lb}))
}

fn shape_of(kind: &LinkKind) -> Option<usize> {
    match kind {
        LinkKind::Start(s) | LinkKind::Centre(s) => Some(*s),
        _ => None,
    }
}

// ── Segment colours ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SegmentState {
    /// Green.
    Compatible,
    /// Yellow: compatible, but the segment's chord turns by more than the threshold against
    /// another layer of the group.
    Turns,
    /// Red: another layer has no such segment, or a node of it or before it differs.
    Incompatible,
}

/// The state of each segment of each path of `layer` against `others` (the rest of its group),
/// as `(layer.shapes index, the segment's end node, state)`. Nothing without other layers.
pub fn segment_states(layer: &Layer, others: &[&Layer]) -> Vec<(usize, usize, SegmentState)> {
    let mut out = Vec::new();
    if others.is_empty() {
        return out;
    }
    for (o, &index) in outline_indices(layer).iter().enumerate().take(MAX_LINKED) {
        let Some(ModelShape::Path(p)) = layer.shapes.get(index) else { continue };
        for seg in segments(&p.node_points(), p.closed) {
            let mut state = SegmentState::Compatible;
            for other in others {
                let s = match outline_shape(other, o) {
                    Some(ModelShape::Path(q)) => {
                        let first_diff = p.nodes.iter().zip(&q.nodes).position(|(m, n)| m.kind != n.kind).or_else(|| {
                            if p.closed != q.closed { Some(0) } else { (p.nodes.len() != q.nodes.len()).then(|| p.nodes.len().min(q.nodes.len())) }
                        });
                        let late = |i: usize| first_diff.is_some_and(|d| i >= d);
                        if late(seg.end) || seg.handles.iter().any(|&h| late(h)) {
                            SegmentState::Incompatible
                        } else {
                            let chord = |path: &fontcraft_engine::model::Path| {
                                let (a, b) = (path.nodes.get(seg.start)?, path.nodes.get(seg.end)?);
                                Some((b.x - a.x, b.y - a.y))
                            };
                            match (chord(p), chord(q)) {
                                (Some(u), Some(v)) if turn(u, v).is_some_and(|d| d > ANGLE_WARNING_DEGREES) => SegmentState::Turns,
                                _ => SegmentState::Compatible,
                            }
                        }
                    }
                    _ => SegmentState::Incompatible,
                };
                state = state.max(s);
            }
            out.push((index, seg.end, state));
        }
    }
    out
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

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The tint of shape `index`: the colour labels in turn.
fn tint(t: &crate::theme::Tokens, index: usize, alpha: u8) -> Color32 {
    let base = t.color_labels.get(index % t.color_labels.len().max(1)).copied().unwrap_or(t.accent);
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
}

/// Draws the view for the edited layer while Show Master Compatibility is on, and queues
/// `glyph.swapShapes` when a line end is dropped on another.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    if !scene.state.display.master_compatibility || scene.tab.edit_background {
        return;
    }
    let Some(glyph) = scene.tab.edited_glyph().and_then(|g| scene.font.glyph(g)) else { return };
    let font = scene.font;
    let sources = Designspace::new(font).ok().and_then(|ds| GlyphSources::of(font, &ds, &glyph.name).ok());
    let placed = placement(font, glyph, scene.layer, sources.as_ref());
    let layers: Vec<&Layer> = placed
        .iter()
        .filter_map(|p| if p.layer == scene.layer.id { Some(scene.layer) } else { glyph.layers.iter().find(|l| l.id == p.layer) })
        .collect();
    if layers.len() != placed.len() {
        return;
    }
    let Some(current) = placed.iter().position(|p| p.layer == scene.layer.id) else { return };
    let t = c.tokens;
    let to_screen = |p: Point| pos(c.view.to_screen(p));
    for (i, (p, layer)) in placed.iter().zip(&layers).enumerate() {
        let others: Vec<&Layer> =
            placed.iter().zip(&layers).filter(|(q, _)| q.group == p.group).map(|(_, l)| *l).filter(|l| l.id != layer.id).collect();
        draw_layer(c, font, layer, p.offset, &others, i == current);
    }
    let all = links(font, &placed, &layers, current, &scene.tab.selection);
    let line = Stroke::new(t.compat_line_width, t.compat_line);
    for link in &all {
        let points: Vec<Pos2> = link.ends.iter().map(|e| to_screen(e.at)).filter(|p| usable(*p)).collect();
        if points.len() < 2 {
            continue;
        }
        match link.kind {
            LinkKind::Anchor(_) => c.painter.extend(Shape::dashed_line(&points, line, 4.0, 3.0)),
            _ => {
                c.painter.add(Shape::line(points, line));
            }
        }
        for e in &link.ends {
            if let Some(name) = &e.mismatch {
                c.painter.text(
                    to_screen(e.at) + Vec2::new(0.0, -t.compat_end_radius * 2.0),
                    Align2::CENTER_BOTTOM,
                    name,
                    FontId::proportional(t.compat_label_size),
                    t.compat_bad,
                );
            }
        }
    }
    let reach = f64::from(t.compat_end_radius) * 3.0 / c.view.scale().max(f64::MIN_POSITIVE);
    for (k, link) in all.iter().enumerate() {
        let Some(shape) = shape_of(&link.kind) else { continue };
        for (j, e) in link.ends.iter().enumerate() {
            if e.placed == current || e.missing {
                continue;
            }
            let at = to_screen(e.at);
            if !usable(at) || !c.rect.contains(at) {
                continue;
            }
            c.painter.circle(at, t.compat_end_radius, tint(t, shape, 0xff), Stroke::new(1.0, t.canvas));
            let id = ui.id().with(("compat-end", placed.get(e.placed).map_or("", |p| p.layer.as_str()), shape));
            let area = Rect::from_center_size(at, Vec2::splat(t.compat_end_radius * 2.5));
            let response = ui.interact(area, id, Sense::drag()).on_hover_text("Drag onto another line's end on this layer to swap the two shapes");
            let pointer = response.interact_pointer_pos();
            if response.dragged()
                && let Some(p) = pointer
            {
                c.painter.line_segment([at, p], Stroke::new(t.compat_line_width * 2.0, tint(t, shape, 0xff)));
            }
            if response.drag_stopped()
                && let Some(p) = pointer
            {
                let dropped = c.view.to_font(Point::new(f64::from(p.x), f64::from(p.y)));
                if let Some(params) = drop_swap(&glyph.name, &layers, &all, (k, j), dropped, reach, current) {
                    pending.push(("glyph.swapShapes", params));
                }
            }
        }
    }
}

/// One placed layer: shape tints, coloured segments and, on the edited layer, shape indices.
fn draw_layer(c: &Painting, font: &Font, layer: &Layer, offset: (f64, f64), others: &[&Layer], current: bool) {
    let t = c.tokens;
    let to_screen =
        Affine::translate(c.view.origin.to_vec2()) * Affine::scale_non_uniform(c.view.scale(), -c.view.scale()) * Affine::translate(offset);
    let screen = |p: Point| pos(to_screen * p);
    for (o, &index) in outline_indices(layer).iter().enumerate().take(MAX_LINKED) {
        let Some(shape) = layer.shapes.get(index) else { continue };
        let outline = super::background::outline(font, std::slice::from_ref(shape), &layer.master);
        let stroke = Stroke::new(t.compat_tint_width, tint(t, o, t.compat_tint_alpha));
        for contour in flatten(&(to_screen * outline)) {
            match shape {
                ModelShape::Component(_) => c.painter.extend(Shape::dashed_line(&contour, stroke, 3.0, 3.0)),
                _ => {
                    c.painter.add(Shape::line(contour, stroke));
                }
            }
        }
        if current {
            let label = match shape {
                ModelShape::Path(p) => p.nodes.iter().find(|n| n.on_curve()).map(|n| n.point()),
                ModelShape::Component(comp) => Some(component_centre(font, comp, &layer.master)),
                _ => None,
            };
            if let Some(at) = label.map(screen).filter(|p| usable(*p)) {
                c.painter.text(
                    at + Vec2::new(t.compat_end_radius, -t.compat_end_radius),
                    Align2::LEFT_BOTTOM,
                    o.to_string(),
                    FontId::proportional(t.compat_label_size),
                    tint(t, o, 0xff),
                );
            }
        }
    }
    let states: HashMap<(usize, usize), SegmentState> = segment_states(layer, others).into_iter().map(|(i, e, s)| ((i, e), s)).collect();
    if states.is_empty() {
        return;
    }
    for &index in outline_indices(layer).iter().take(MAX_LINKED) {
        let Some(ModelShape::Path(p)) = layer.shapes.get(index) else { continue };
        for seg in segments(&p.node_points(), p.closed) {
            let Some(state) = states.get(&(index, seg.end)) else { continue };
            let colour = match state {
                SegmentState::Compatible => t.compat_ok,
                SegmentState::Turns => t.compat_warn,
                SegmentState::Incompatible => t.compat_bad,
            };
            let mut line: Vec<Pos2> = Vec::with_capacity(seg.pieces.len() * SEGMENT_SAMPLES + 1);
            for piece in &seg.pieces {
                line.extend((0..=SEGMENT_SAMPLES).map(|k| screen(piece.eval(k as f64 / SEGMENT_SAMPLES as f64))));
            }
            if line.len() > 1 && line.iter().all(|p| usable(*p)) {
                c.painter.add(Shape::line(line, Stroke::new(t.compat_segment_width, colour)));
            }
        }
    }
}
