//! Cap components, segment components and brushes fitted into outlines. Behaviour:
//! `docs/spec/09-reusing-shapes.md` § 8 to § 10 and the `cap.*`, `capComponent.*`,
//! `segmentComponent.*` and `brush.*` rows; the fitting follows the rows' guesses until the
//! reference app's exports are in (`docs/plans/M13.md` § 3 and § 8).
//!
//! Each is a hint on the layer, as a corner component is ([`crate::corner`]):
//! - a **cap** (`_cap.<name>`) sits on two neighbouring nodes of one path, the end of a stroke,
//!   with a scale, an alignment and a Fit flag that sizes it to the stroke's width. Its glyph
//!   carries an `origin` anchor, or `node1` and `node2` anchors for both stem nodes;
//! - a **segment component** (`_segment.<name>`, one open path from anchor `start` to `end`)
//!   replaces one segment of a host path, bent along it without adding nodes;
//! - a **brush** (`_brush.<name>`, one closed path, anchors `start` and `end`) is drawn along one
//!   segment: the outline between the anchors stretches and bends, the rest keeps its shape.
//!
//! Outline resolution calls [`insert_attachments`]: the caps are fitted first, the other
//! attachment and corner hints of the layer are moved to the nodes they sat on, and the corners
//! are fitted into the result. Inserted nodes are rounded to whole units, as corner components
//! are.
//!
//! ## How a cap is fitted
//! The cap glyph's first path is open. It is placed in the cap glyph's own frame, with its
//! reference point (`node1`, else `origin`, else the layer origin) at (0, 0) and the `node1` to
//! `node2` line along x; it is scaled there, turned by the alignment and moved so the reference
//! point lands on the first host node. The path then takes the place of the straight segment
//! between the two host nodes: the side before the first node is cut at its point nearest the
//! cap's start node and ends there, and the side after the second node starts at the cap's end
//! node, cut likewise. The cap glyph's other paths are added to the layer, placed the same way.
//!
//! The cap glyph is drawn as for the bottom end of a stem whose path runs counter-clockwise, so
//! on the top end the centre alignment turns it 180 degrees. Alignments: centre (`Both`) lays the
//! cap's x axis on the line from the first to the second host node; left turns the cap's y axis
//! onto the side before the first node, right onto the side after the second; none does not turn
//! it. With Fit on, the horizontal scale is the host nodes' distance over the cap's reference
//! width (the `node1` to `node2` distance, or the advance width for an origin-only cap).
//!
//! ## How segment components and brushes are bent
//! Both read their glyph in a frame whose x axis runs from the `start` anchor to the `end`
//! anchor (a segment glyph without them uses its path's end nodes; a brush glyph uses (0, 0) and
//! (advance width, 0), where Set Anchors puts them). A point's x is its fraction of the
//! start-to-end length, its y its distance from that line, positive on the left. The host segment
//! is measured by its arc length, sampled at most [`MAX_ARC_SAMPLES`] times; each point goes to
//! the host point at the same fraction of that length, moved by its distance along the host's
//! left normal there. Only existing points move: a straight host scales the shape along its
//! length and not across it, a curved host bends it without new nodes, and a line between two
//! on-curve nodes stays a line.
//!
//! A segment component's open path takes the place of the host segment, its end nodes taking the
//! host's end nodes' places. A brush's closed path is added to the layer and the host path stays;
//! the brush outline before `start` and after `end` does not stretch: it moves rigidly with the
//! host's direction at that end. Fitting order: caps, then segment components, then brushes.

use fontcraft_geom::{Affine, CubicBez, ParamCurve, ParamCurveNearest, Point, QuadBez, Vec2};
use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value};

use crate::corner::{CORNER_HINT, CornerAlignment};
use crate::{Family, Font, Glyph, Hint, Layer, MAX_OUTLINE_ELEMENTS, Node, NodeKind, Path, Shape, in_range};

/// The hint type of a cap component.
pub const CAP_HINT: &str = "Cap";
/// The hint type of a segment component.
pub const SEGMENT_HINT: &str = "Segment";
/// The hint type of a brush.
pub const BRUSH_HINT: &str = "Brush";
/// What a cap glyph's name starts with.
pub const CAP_PREFIX: &str = "_cap.";
/// What a segment glyph's name starts with.
pub const SEGMENT_PREFIX: &str = "_segment.";
/// What a brush glyph's name starts with.
pub const BRUSH_PREFIX: &str = "_brush.";
/// Most points sampled along one host segment to measure its arc length.
pub const MAX_ARC_SAMPLES: usize = 4096;
/// Samples per curve piece of a host segment, while [`MAX_ARC_SAMPLES`] allows.
const SAMPLES_PER_PIECE: usize = 64;
/// Largest cap scale, in either direction, as a factor (100 is 10,000 %), stored or computed by
/// Fit.
pub const MAX_CAP_SCALE: f64 = 100.0;
/// Most node moves fitting the attachments of one layer may cost: each fit rebuilds its host path.
pub const MAX_ATTACH_WORK: usize = 50_000_000;
/// The `options` bit that turns Fit on; the low bits hold the alignment.
pub const CAP_FIT_BIT: i64 = 8;
/// Shortest stem end, in font units, a cap can sit on: below it the stem's direction is lost.
const MIN_STEM: f64 = 1e-6;

/// A cap component as its hint stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct CapHint {
    /// The cap glyph (`_cap.<name>`).
    pub name: String,
    /// The path (shape index) and its first stem node; the second is the next node. `node`
    /// counts in file order, as [`crate::corner::CornerHint::new`] explains: on a closed path the
    /// start node is last.
    pub shape: usize,
    pub node: usize,
    /// Horizontal and vertical scale as factors; with Fit on the horizontal one is not used.
    pub scale: Vec2,
    pub alignment: CornerAlignment,
    /// Sized to the stroke's width (`capComponent.setFit`).
    pub fit: bool,
}

impl CapHint {
    /// A cap of `name` on file-order node `node` of path `shape` and the node after it, unscaled,
    /// aligned left, Fit off.
    pub fn new(name: impl Into<String>, shape: usize, node: usize) -> Self {
        Self { name: name.into(), shape, node, scale: Vec2::new(1.0, 1.0), alignment: CornerAlignment::Left, fit: false }
    }

    /// The stored `options`: the alignment, plus [`CAP_FIT_BIT`] with Fit on.
    pub fn options(&self) -> i64 {
        self.alignment.option() + if self.fit { CAP_FIT_BIT } else { 0 }
    }

    /// The hint that stores this cap, with the keys a version 3 `.glyphs` file writes.
    pub fn to_hint(&self) -> Hint {
        let int = |v: usize| Value::Number(Number::int(i64::try_from(v).unwrap_or(i64::MAX)));
        let float = |v: f64| Number::float(v).map_or_else(|| Value::Number(Number::int(1)), Value::Number);
        let pair = |items: Vec<Value>| Value::Array(Array { items, layout: Layout::Compact, ..Default::default() });
        let mut d = Dict::new();
        d.insert("name", Value::String(Str::new(self.name.clone())));
        if self.options() != 0 {
            d.insert("options", Value::Number(Number::int(self.options())));
        }
        d.insert("origin", pair(vec![int(self.shape), int(self.node)]));
        if self.scale != Vec2::new(1.0, 1.0) {
            d.insert("scale", pair(vec![float(self.scale.x), float(self.scale.y)]));
        }
        d.insert("type", Value::String(Str::new(CAP_HINT)));
        Hint { family: Family::Glyphs, value: Value::Dict(d) }
    }
}

/// A segment component or a brush as its hint stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentHint {
    /// The segment or brush glyph.
    pub name: String,
    /// The path (shape index) and the on-curve node the host segment ends at, in file order (on
    /// a closed path the start node is last).
    pub shape: usize,
    pub node: usize,
}

impl SegmentHint {
    /// A segment component or brush of `name` on the segment ending at file-order node `node` of
    /// path `shape`.
    pub fn new(name: impl Into<String>, shape: usize, node: usize) -> Self {
        Self { name: name.into(), shape, node }
    }

    /// The hint of type `kind` that stores it: `type`, `name` and `origin` only.
    fn to_hint(&self, kind: &str) -> Hint {
        let int = |v: usize| Value::Number(Number::int(i64::try_from(v).unwrap_or(i64::MAX)));
        let mut d = Dict::new();
        d.insert("name", Value::String(Str::new(self.name.clone())));
        d.insert("origin", Value::Array(Array { items: vec![int(self.shape), int(self.node)], layout: Layout::Compact, ..Default::default() }));
        d.insert("type", Value::String(Str::new(kind)));
        Hint { family: Family::Glyphs, value: Value::Dict(d) }
    }
}

/// One attachment on a layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Attachment {
    Cap(CapHint),
    Segment(SegmentHint),
    Brush(SegmentHint),
}

impl Attachment {
    /// The hint that stores this attachment.
    pub fn to_hint(&self) -> Hint {
        match self {
            Attachment::Cap(c) => c.to_hint(),
            Attachment::Segment(s) => s.to_hint(SEGMENT_HINT),
            Attachment::Brush(b) => b.to_hint(BRUSH_HINT),
        }
    }

    /// The cap, segment or brush glyph.
    pub fn name(&self) -> &str {
        match self {
            Attachment::Cap(c) => &c.name,
            Attachment::Segment(s) | Attachment::Brush(s) => &s.name,
        }
    }

    /// The host path (shape index) and the stored file-order node.
    pub fn origin(&self) -> (usize, usize) {
        match self {
            Attachment::Cap(c) => (c.shape, c.node),
            Attachment::Segment(s) | Attachment::Brush(s) => (s.shape, s.node),
        }
    }

    /// The kind's name in messages: cap, segment or brush.
    pub fn noun(&self) -> &'static str {
        match self {
            Attachment::Cap(_) => "cap",
            Attachment::Segment(_) => "segment",
            Attachment::Brush(_) => "brush",
        }
    }

    /// The order in which a layer's attachments are fitted.
    fn rank(&self) -> u8 {
        match self {
            Attachment::Cap(_) => 0,
            Attachment::Segment(_) => 1,
            Attachment::Brush(_) => 2,
        }
    }
}

/// Why an attachment could not be fitted.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("`{glyph}`: {message}")]
pub struct AttachError {
    /// The cap, segment or brush glyph.
    pub glyph: String,
    pub message: String,
}

/// Whether `hint` is a cap component.
pub fn is_cap(hint: &Hint) -> bool {
    hint.kind() == Some(CAP_HINT)
}

/// Whether `hint` is a segment component.
pub fn is_segment(hint: &Hint) -> bool {
    hint.kind() == Some(SEGMENT_HINT)
}

/// Whether `hint` is a brush.
pub fn is_brush(hint: &Hint) -> bool {
    hint.kind() == Some(BRUSH_HINT)
}

/// Whether `hint` is a cap component, a segment component or a brush.
pub fn is_attachment(hint: &Hint) -> bool {
    matches!(hint.kind(), Some(CAP_HINT | SEGMENT_HINT | BRUSH_HINT))
}

/// Whether `name` is a cap glyph's name: `_cap.` and at least one more character.
pub fn is_cap_name(name: &str) -> bool {
    name.len() > CAP_PREFIX.len() && name.starts_with(CAP_PREFIX)
}

/// Whether `name` is a segment glyph's name: `_segment.` and at least one more character.
pub fn is_segment_name(name: &str) -> bool {
    name.len() > SEGMENT_PREFIX.len() && name.starts_with(SEGMENT_PREFIX)
}

/// Whether `name` is a brush glyph's name: `_brush.` and at least one more character.
pub fn is_brush_name(name: &str) -> bool {
    name.len() > BRUSH_PREFIX.len() && name.starts_with(BRUSH_PREFIX)
}

/// The attachment `hint` stores; `Ok(None)` for a hint of another kind, `Err` for an attachment
/// whose keys cannot be read.
pub fn attachment(hint: &Hint) -> Result<Option<Attachment>, String> {
    if is_segment(hint) {
        return Ok(segment_hint(hint)?.map(Attachment::Segment));
    }
    if is_brush(hint) {
        return Ok(segment_hint(hint)?.map(Attachment::Brush));
    }
    Ok(cap_hint(hint)?.map(Attachment::Cap))
}

/// The segment component or brush `hint` stores; `Ok(None)` when it is another kind of hint.
pub fn segment_hint(hint: &Hint) -> Result<Option<SegmentHint>, String> {
    let noun = match hint.kind() {
        Some(SEGMENT_HINT) => "segment",
        Some(BRUSH_HINT) => "brush",
        _ => return Ok(None),
    };
    let Some(d) = hint.value.as_dict() else { return Ok(None) };
    let name = d.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("the hint names no {noun} glyph"))?;
    let (shape, node) = origin(d)?;
    Ok(Some(SegmentHint { name: name.to_string(), shape, node }))
}

/// The path index and node index of a hint's `origin`.
fn origin(d: &Dict) -> Result<(usize, usize), String> {
    let origin = numbers(d.get("origin").ok_or("the hint has no origin")?).ok_or("the hint's origin is not a list of numbers")?;
    let index = |v: f64| (v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= f64::from(u32::MAX)).then_some(v as usize);
    let (shape, node) = match origin.as_slice() {
        &[s, n] => (index(s), index(n)),
        _ => (None, None),
    };
    let (Some(shape), Some(node)) = (shape, node) else { return Err("the hint's origin is not a path index and a node index".into()) };
    Ok((shape, node))
}

/// The cap `hint` stores; `Ok(None)` when it is another kind of hint.
pub fn cap_hint(hint: &Hint) -> Result<Option<CapHint>, String> {
    if !is_cap(hint) {
        return Ok(None);
    }
    let Some(d) = hint.value.as_dict() else { return Ok(None) };
    let name = d.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("the hint names no cap glyph")?;
    let (shape, node) = origin(d)?;
    let scale = match d.get("scale") {
        None => Vec2::new(1.0, 1.0),
        Some(v) => match numbers(v).as_deref() {
            Some(&[x, y]) => Vec2::new(x, y),
            _ => return Err("the hint's scale is not two numbers".into()),
        },
    };
    check_scale(scale)?;
    let options = d.get("options").and_then(number).filter(|v| v.is_finite() && v.fract() == 0.0 && v.abs() <= 1024.0).unwrap_or(0.0) as i64;
    let alignment = CornerAlignment::from_option((options & (CAP_FIT_BIT - 1)) as f64);
    Ok(Some(CapHint { name: name.to_string(), shape, node, scale, alignment, fit: options & CAP_FIT_BIT != 0 }))
}

/// Refuses a scale that is not a number or lies beyond ±[`MAX_CAP_SCALE`].
fn check_scale(scale: Vec2) -> Result<(), String> {
    if scale.x.is_finite() && scale.y.is_finite() && scale.x.abs() <= MAX_CAP_SCALE && scale.y.abs() <= MAX_CAP_SCALE {
        return Ok(());
    }
    Err(format!("the scale ({}, {}) is not a number or is beyond ±{MAX_CAP_SCALE}", scale.x, scale.y))
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

// ── Layers ──────────────────────────────────────────────────────────────────────────────────────

/// The shapes of `layer` with every attachment fitted in, then its corner components;
/// `Ok(None)` when the layer has no attachment whose glyph exists. A cap whose glyph does not
/// exist is left out, as a corner's is. Outline resolution calls this and, on an error, draws
/// the layer with its corners only.
pub fn insert_attachments(font: &Font, layer: &Layer) -> Result<Option<Vec<Shape>>, AttachError> {
    if !layer.info.hints.iter().any(is_attachment) {
        return Ok(None);
    }
    let mut indices = Vec::new();
    for (i, h) in layer.info.hints.iter().enumerate() {
        match attachment(h) {
            Ok(Some(a)) if font.glyph(a.name()).is_some() => indices.push(i),
            Ok(_) => {}
            Err(message) => return Err(AttachError { glyph: hint_name(h), message }),
        }
    }
    if indices.is_empty() {
        return Ok(None);
    }
    let fitted = decompose_attachments(font, layer, &indices)?;
    match crate::corner::insert_corners(font, &fitted) {
        Ok(Some(shapes)) => Ok(Some(shapes)),
        Ok(None) => Ok(Some(fitted.shapes)),
        Err(e) => Err(AttachError { glyph: e.corner, message: e.message }),
    }
}

/// `layer` with the attachments at `indices` (positions in the layer's hints) fitted into its
/// paths and their hints removed (Decompose Cap, Decompose Segment Component, Decompose Brush).
/// The other corner and attachment hints keep pointing at the nodes they sat on; a hint on a
/// cap's first node moves to the cap's start node, one on its second node to the cap's end node.
pub fn decompose_attachments(font: &Font, layer: &Layer, indices: &[usize]) -> Result<Layer, AttachError> {
    let bad = |glyph: &str, message: String| AttachError { glyph: glyph.to_string(), message };
    let mut items: Vec<(usize, Attachment)> = Vec::new();
    for &i in indices {
        let hint = layer.info.hints.get(i).ok_or_else(|| bad("", format!("the layer has no hint {i}")))?;
        let item = attachment(hint)
            .map_err(|m| bad(&hint_name(hint), m))?
            .ok_or_else(|| bad("", format!("hint {i} is not a cap component, a segment component or a brush")))?;
        if !items.iter().any(|(j, _)| *j == i) {
            items.push((i, item));
        }
    }
    items.sort_by_key(|(_, a)| a.rank());
    let mut shapes = layer.shapes.clone();
    let mut ids: Vec<Vec<Option<usize>>> =
        shapes.iter().map(|s| if let Shape::Path(p) = s { (0..p.nodes.len()).map(Some).collect() } else { Vec::new() }).collect();
    let old: Vec<(bool, usize)> = shapes.iter().map(|s| if let Shape::Path(p) = s { (p.closed, p.nodes.len()) } else { (false, 0) }).collect();
    let mut extra = Vec::new();
    let mut work = 0usize;
    for (_, item) in &items {
        let err = |message: String| bad(item.name(), message);
        let glyph = font.glyph(item.name()).ok_or_else(|| err(format!("there is no such {} glyph", item.noun())))?;
        let (shape, node) = item.origin();
        let Some(&(closed, len)) = old.get(shape) else { return Err(err(format!("there is no path at shape index {shape}"))) };
        let (Some(Shape::Path(path)), Some(path_ids)) = (shapes.get_mut(shape), ids.get_mut(shape)) else {
            return Err(err(format!("shape {shape} is not a path")));
        };
        if node >= len {
            return Err(err(format!("path {shape} has {len} nodes; the {} sits on node {node}", item.noun())));
        }
        let target = to_model(closed, len, node);
        match item {
            Attachment::Cap(cap) => {
                let geometry = CapGlyph::new(glyph, &layer.master).map_err(err)?;
                let second = if closed { (target + 1) % len } else { target + 1 };
                let placed = fit_cap_into(path, path_ids, &geometry, cap, (target, second), &mut work).map_err(err)?;
                extra.extend(placed.into_iter().map(Shape::Path));
            }
            Attachment::Segment(_) => {
                let geometry = StrokeGlyph::new(glyph, &layer.master, false).map_err(err)?;
                fit_segment_into(path, path_ids, &geometry, target, &mut work).map_err(err)?;
            }
            Attachment::Brush(_) => {
                let geometry = StrokeGlyph::new(glyph, &layer.master, true).map_err(err)?;
                extra.push(Shape::Path(brush_path(path, path_ids, &geometry, target, &mut work).map_err(err)?));
            }
        }
    }
    shapes.extend(extra);
    let mut out = layer.clone();
    let maps: Vec<Option<PathMap>> = old
        .iter()
        .zip(&ids)
        .zip(&shapes)
        .map(|(((closed, len), ids), shape)| {
            let Shape::Path(p) = shape else { return None };
            if ids.len() == *len && ids.iter().enumerate().all(|(i, id)| *id == Some(i)) {
                return None;
            }
            let mut new_of_old = vec![None; *len];
            for (new, id) in ids.iter().enumerate() {
                if let Some(slot) = id.and_then(|o| new_of_old.get_mut(o)) {
                    *slot = Some(new);
                }
            }
            Some(PathMap { closed: *closed, old_len: *len, new_len: p.nodes.len(), new_of_old })
        })
        .collect();
    let removed: Vec<usize> = items.iter().map(|(i, _)| *i).collect();
    out.info.hints = layer.info.hints.iter().enumerate().filter(|(i, _)| !removed.contains(i)).map(|(_, h)| remap_hint(h, &maps)).collect();
    out.shapes = shapes;
    Ok(out)
}

/// `shapes` (a layer's shapes in master `master`) with `cap` fitted onto its two nodes.
pub fn fit_cap(font: &Font, master: &str, shapes: &[Shape], cap: &CapHint) -> Result<Vec<Shape>, AttachError> {
    fit_one(font, master, shapes, cap.to_hint())
}

/// `shapes` (a layer's shapes in master `master`) with the segment that `segment` names
/// replaced by the segment glyph's path, bent along it.
pub fn fit_segment(font: &Font, master: &str, shapes: &[Shape], segment: &SegmentHint) -> Result<Vec<Shape>, AttachError> {
    fit_one(font, master, shapes, segment.to_hint(SEGMENT_HINT))
}

/// `shapes` (a layer's shapes in master `master`) with the brush's outline drawn along the
/// segment that `brush` names, added after them.
pub fn fit_brush(font: &Font, master: &str, shapes: &[Shape], brush: &SegmentHint) -> Result<Vec<Shape>, AttachError> {
    fit_one(font, master, shapes, brush.to_hint(BRUSH_HINT))
}

/// `shapes` with the one attachment `hint` fitted in.
fn fit_one(font: &Font, master: &str, shapes: &[Shape], hint: Hint) -> Result<Vec<Shape>, AttachError> {
    let mut layer = Layer::new(master, 0.0);
    layer.shapes = shapes.to_vec();
    layer.info.hints.push(hint);
    Ok(decompose_attachments(font, &layer, &[0])?.shapes)
}

/// What [`insert_all_attachments`] did.
#[derive(Debug, Default)]
pub struct AttachOutlines {
    /// Layers whose attachments were fitted.
    pub layers: usize,
    /// Layers left as they were, each with the reason.
    pub failed: Vec<String>,
}

/// `font` with the attachments and corner components of every layer that has attachments
/// fitted into its paths and those hints removed, as the compile reads a source. A layer that
/// cannot be fitted is left as it was and listed in [`AttachOutlines::failed`]; its corners are
/// then fitted by [`crate::corner::insert_all_corner_components`].
pub fn insert_all_attachments(font: &Font) -> (Font, AttachOutlines) {
    let mut report = AttachOutlines::default();
    let mut changes: Vec<(usize, usize, Vec<Shape>)> = Vec::new();
    for (gi, glyph) in font.glyphs.iter().enumerate() {
        for (li, layer) in glyph.layers.iter().enumerate() {
            match insert_attachments(font, layer) {
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
            l.info.hints.retain(|h| !is_attachment(h) && h.kind() != Some(CORNER_HINT));
            report.layers += 1;
        }
    }
    (out, report)
}

/// The name a hint gives, for messages.
fn hint_name(hint: &Hint) -> String {
    hint.value.as_dict().and_then(|d| d.get("name")).and_then(Value::as_str).unwrap_or("").to_string()
}

// ── Node numbering ──────────────────────────────────────────────────────────────────────────────

/// The model's index of file-order node `node`: on a closed path the file's start node is last.
fn to_model(closed: bool, len: usize, node: usize) -> usize {
    if closed && len > 0 { (node + 1) % len } else { node }
}

/// The file-order index of the model's node `node`.
fn to_file(closed: bool, len: usize, node: usize) -> usize {
    if closed && len > 0 { (node + len - 1) % len } else { node }
}

/// Where the nodes of one path went when attachments were fitted into it, in the model's order.
struct PathMap {
    closed: bool,
    old_len: usize,
    new_len: usize,
    new_of_old: Vec<Option<usize>>,
}

impl PathMap {
    /// The new file-order index of old file-order node `node`; `None` when it is gone.
    fn file(&self, node: usize) -> Option<usize> {
        if node >= self.old_len {
            return None;
        }
        let new = (*self.new_of_old.get(to_model(self.closed, self.old_len, node))?)?;
        Some(to_file(self.closed, self.new_len, new))
    }
}

/// `hint` with its `origin` node indices moved by `maps`, kept as spelled; hints of other kinds,
/// and nodes that are gone, stay as they are.
fn remap_hint(hint: &Hint, maps: &[Option<PathMap>]) -> Hint {
    if !matches!(hint.kind(), Some(CORNER_HINT | CAP_HINT | SEGMENT_HINT | BRUSH_HINT)) {
        return hint.clone();
    }
    let mut out = hint.clone();
    let Value::Dict(d) = &mut out.value else { return out };
    let Some(origin) = d.get_mut("origin") else { return out };
    let Some(mut nums) = numbers(origin) else { return out };
    let mut changed = false;
    for pair in [0usize, 2] {
        let (Some(&shape), Some(&node)) = (nums.get(pair), nums.get(pair + 1)) else { continue };
        let index = |v: f64| (v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= f64::from(u32::MAX)).then_some(v as usize);
        let (Some(shape), Some(node)) = (index(shape), index(node)) else { continue };
        let Some(Some(map)) = maps.get(shape) else { continue };
        if let (Some(new), Some(slot)) = (map.file(node), nums.get_mut(pair + 1)) {
            *slot = new as f64;
            changed = true;
        }
    }
    if !changed {
        return out;
    }
    match origin {
        Value::String(s) => {
            let parts: Vec<String> = nums.iter().map(|v| format!("{v}")).collect();
            *s = Str::new(format!("{{{}}}", parts.join(", ")));
        }
        Value::Array(a) => a.items = nums.iter().map(|&v| Value::Number(Number::int(v as i64))).collect(),
        _ => {}
    }
    out
}

// ── Cap fitting ─────────────────────────────────────────────────────────────────────────────────

/// A cap glyph's layer for one master, as the fitting reads it.
struct CapGlyph {
    /// The first path, open.
    path: Path,
    /// The other paths, added to the layer.
    others: Vec<Path>,
    /// The point that lands on the first host node: `node1`, else `origin`, else (0, 0).
    reference: Point,
    /// The angle of the `node1` to `node2` line, 0 for an origin-only cap.
    angle: f64,
    /// What Fit sizes to the stem: the `node1` to `node2` distance, or the advance width.
    width: f64,
}

impl CapGlyph {
    fn new(glyph: &Glyph, master: &str) -> Result<Self, String> {
        let layer = glyph.layer(master).ok_or_else(|| format!("the cap glyph has no layer for master `{master}`"))?;
        let anchor = |name: &str| layer.anchors.iter().find(|a| a.name == name).map(|a| Point::new(a.x, a.y));
        let mut paths = layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p.clone()) } else { None });
        let path = paths.next().ok_or("the cap glyph has no paths")?;
        if path.closed {
            return Err("the cap glyph's first path is closed; a cap is drawn with an open path".into());
        }
        if path.nodes.len() < 2 || !path.nodes.first().is_some_and(Node::on_curve) || !path.nodes.last().is_some_and(Node::on_curve) {
            return Err("the cap glyph's first path needs two on-curve end nodes".into());
        }
        let (reference, angle, width) = match (anchor("node1"), anchor("node2")) {
            (Some(n1), Some(n2)) => {
                let v = n2 - n1;
                (n1, v.y.atan2(v.x), v.hypot())
            }
            _ => (anchor("origin").unwrap_or(Point::ZERO), 0.0, layer.width),
        };
        if !(in_range(reference.x) && in_range(reference.y) && width.is_finite() && angle.is_finite()) {
            return Err("the cap glyph's anchors or width are not numbers".into());
        }
        Ok(Self { path, others: paths.collect(), reference, angle, width })
    }
}

/// The transform that places the cap on the host nodes `p1`, `p2`; `before` and `after` are the
/// nearest points of the sides before `p1` and after `p2`.
fn cap_transform(cap: &CapGlyph, hint: &CapHint, p1: Point, p2: Point, before: Option<Point>, after: Option<Point>) -> Result<Affine, String> {
    let stem = p2 - p1;
    let distance = stem.hypot();
    if !distance.is_finite() || distance <= MIN_STEM {
        return Err("the stem end between the cap's two nodes has zero length".into());
    }
    let sx = if hint.fit {
        if !cap.width.is_finite() || cap.width.abs() <= MIN_STEM {
            return Err("the cap glyph has no width to fit: its node1 and node2 anchors coincide, or its advance width is 0".into());
        }
        distance / cap.width.abs()
    } else {
        hint.scale.x
    };
    let scale = Vec2::new(sx, hint.scale.y);
    check_scale(scale).map_err(|m| format!("{m} (the cap glyph is too narrow for the stem)"))?;
    let along = stem.y.atan2(stem.x);
    let quarter = std::f64::consts::FRAC_PI_2;
    let turn = match hint.alignment {
        CornerAlignment::Both => along,
        CornerAlignment::Left => before.map_or(along, |b| (b - p1).atan2() - quarter),
        CornerAlignment::Right => after.map_or(along, |a| (a - p2).atan2() - quarter),
        CornerAlignment::Unaligned | CornerAlignment::Unused => 0.0,
    };
    Ok(Affine::translate(p1.to_vec2())
        * Affine::rotate(turn)
        * Affine::scale_non_uniform(scale.x, scale.y)
        * Affine::rotate(-cap.angle)
        * Affine::translate(-cap.reference.to_vec2()))
}

/// Rounds half up, as the corner fitting does.
fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// `node` moved by `affine` and rounded to whole units.
fn place(affine: Affine, node: &Node) -> Node {
    let p = affine * node.point();
    let mut out = node.clone();
    (out.x, out.y) = (ot_round(p.x), ot_round(p.y));
    out
}

/// A new node at `p`, rounded.
fn new_node(p: Point, kind: NodeKind) -> Node {
    Node::new(ot_round(p.x), ot_round(p.y), kind)
}

/// The first node position after (`forward`) or before node `from` that differs from `at`.
fn neighbour(nodes: &[Node], closed: bool, from: usize, forward: bool, at: Point) -> Option<Point> {
    let len = nodes.len();
    let mut i = from;
    for _ in 1..len {
        i = match (forward, closed) {
            (true, true) => (i + 1) % len,
            (true, false) if i + 1 < len => i + 1,
            (false, true) => (i + len - 1) % len,
            (false, false) if i > 0 => i - 1,
            _ => return None,
        };
        let p = nodes.get(i)?.point();
        if p != at {
            return Some(p);
        }
    }
    None
}

/// The off-curve points of the segment through `pts` (start, off-curves, end) cut at its point
/// nearest `to`, keeping the part from the start, with its end moved to `to`.
fn cut_keep_start(pts: &[Point], to: Point) -> Vec<Point> {
    let shifted = |mut offs: Vec<Point>, end: Point| {
        if let Some(last) = offs.last_mut() {
            *last += to - end;
        }
        offs
    };
    match *pts {
        [_, _] => Vec::new(),
        [p0, p1, p2] => {
            let q = QuadBez::new(p0, p1, p2);
            let s = q.subsegment(0.0..q.nearest(to, 1e-6).t);
            shifted(vec![s.p1], s.p2)
        }
        [p0, p1, p2, p3] => {
            let c = CubicBez::new(p0, p1, p2, p3);
            let s = c.subsegment(0.0..c.nearest(to, 1e-6).t);
            shifted(vec![s.p1, s.p2], s.p3)
        }
        // A quadratic spline with several off-curve nodes keeps them; only its end moves.
        _ => {
            let end = pts.last().copied().unwrap_or(to);
            shifted(pts.get(1..pts.len().saturating_sub(1)).unwrap_or_default().to_vec(), end)
        }
    }
}

/// As [`cut_keep_start`], keeping the part up to the end, with its start moved to `to`.
fn cut_keep_end(pts: &[Point], to: Point) -> Vec<Point> {
    let reversed: Vec<Point> = pts.iter().rev().copied().collect();
    let mut offs = cut_keep_start(&reversed, to);
    offs.reverse();
    offs
}

/// The positions of `nodes`.
fn points(nodes: &[(Node, Option<usize>)]) -> Vec<Point> {
    nodes.iter().map(|(n, _)| n.point()).collect()
}

/// New off-curve nodes at `points`, rounded.
fn offs(points: Vec<Point>) -> impl Iterator<Item = (Node, Option<usize>)> {
    points.into_iter().map(|p| (new_node(p, NodeKind::OffCurve), None))
}

/// Fits one cap into `path` on its model-order nodes `pair` (original numbering, found through
/// `ids`, which follow the nodes). Returns the cap glyph's other paths, placed.
fn fit_cap_into(
    path: &mut Path,
    ids: &mut Vec<Option<usize>>,
    cap: &CapGlyph,
    hint: &CapHint,
    pair: (usize, usize),
    work: &mut usize,
) -> Result<Vec<Path>, String> {
    let len = path.nodes.len();
    let a = ids.iter().position(|i| *i == Some(pair.0)).ok_or("the cap's first node is no longer on the path")?;
    let b = if path.closed { (a + 1) % len } else { a + 1 };
    if b >= len {
        return Err("the cap's first node ends the open path; a cap needs the node after it".into());
    }
    if b == a || ids.get(b).copied().flatten() != Some(pair.1) {
        return Err("the cap's two nodes are not neighbours on the path (or another cap sits on them)".into());
    }
    let (Some(n1), Some(n2)) = (path.nodes.get(a), path.nodes.get(b)) else {
        return Err("the cap's nodes are not on the path".into());
    };
    if !n1.on_curve() || !n2.on_curve() {
        return Err("a cap sits on two on-curve nodes".into());
    }
    let (p1, p2) = (n1.point(), n2.point());
    let before = neighbour(&path.nodes, path.closed, a, false, p1);
    let after = neighbour(&path.nodes, path.closed, b, true, p2);
    let affine = cap_transform(cap, hint, p1, p2, before, after)?;
    let placed: Vec<Node> = cap.path.nodes.iter().map(|n| place(affine, n)).collect();
    let (Some(start), Some(end)) = (placed.first().cloned(), placed.last().cloned()) else { return Err("the cap glyph's path is empty".into()) };
    let middle = placed.get(1..placed.len() - 1).unwrap_or_default();
    let added = placed.len();
    *work = work.saturating_add(len).saturating_add(added);
    if *work > MAX_ATTACH_WORK {
        return Err("the layer's caps would take too much work to fit".into());
    }
    if len.saturating_add(added) > MAX_OUTLINE_ELEMENTS {
        return Err(format!("the path with its caps has more than {MAX_OUTLINE_ELEMENTS} nodes"));
    }
    let (s, e) = (start.point(), end.point());
    let first_kind = n1.kind;
    let mut nodes: Vec<(Node, Option<usize>)> = path.nodes.drain(..).zip(ids.drain(..)).collect();
    let mut out: Vec<(Node, Option<usize>)> = Vec::with_capacity(len + added);
    let cap_start = |kind: NodeKind| (Node { kind, ..start.clone() }, Some(pair.0));
    let cap_end = (end.clone(), Some(pair.1));
    let on = |k: &usize, nodes: &[(Node, Option<usize>)]| nodes.get(*k).is_some_and(|(n, _)| n.on_curve());
    if path.closed {
        let start_id = nodes.first().and_then(|(_, id)| *id);
        nodes.rotate_left(b);
        // The second node is first now and the first node last.
        let last = len - 1;
        let j = (1..len).find(|k| on(k, &nodes)).unwrap_or(last);
        let p = (0..last).rev().find(|k| on(k, &nodes)).unwrap_or(0);
        if j >= last || p < j {
            return Err("a cap on a closed path needs at least three on-curve nodes".into());
        }
        let outgoing = cut_keep_end(&points(nodes.get(0..=j).unwrap_or_default()), e);
        let incoming = cut_keep_start(&points(nodes.get(p..=last).unwrap_or_default()), s);
        out.push(cap_end);
        out.extend(offs(outgoing));
        out.extend(nodes.get(j..=p).unwrap_or_default().iter().cloned());
        out.extend(offs(incoming));
        out.push(cap_start(first_kind));
        out.extend(middle.iter().cloned().map(|n| (n, None)));
        if let Some(at) = start_id.and_then(|id| out.iter().position(|(_, i)| *i == Some(id))) {
            out.rotate_left(at);
        }
    } else {
        let p = (0..a).rev().find(|k| on(k, &nodes));
        let j = (b + 1..len).find(|k| on(k, &nodes));
        if (p.is_none() && a > 0) || (j.is_none() && b + 1 < len) {
            return Err("the open path starts or ends with an off-curve node".into());
        }
        match p {
            Some(p) => {
                out.extend(nodes.get(..=p).unwrap_or_default().iter().cloned());
                out.extend(offs(cut_keep_start(&points(nodes.get(p..=a).unwrap_or_default()), s)));
                out.push(cap_start(first_kind));
            }
            None => out.push(cap_start(NodeKind::Line)),
        }
        out.extend(middle.iter().cloned().map(|n| (n, None)));
        out.push(cap_end);
        if let Some(j) = j {
            out.extend(offs(cut_keep_end(&points(nodes.get(b..=j).unwrap_or_default()), e)));
            out.extend(nodes.get(j..).unwrap_or_default().iter().cloned());
        }
    }
    if !out.iter().all(|(n, _)| in_range(n.x) && in_range(n.y)) {
        return Err("the fitted cap is not a number or lies beyond the coordinate limit".into());
    }
    (path.nodes, *ids) = out.into_iter().unzip();
    let mut others = Vec::with_capacity(cap.others.len());
    for other in &cap.others {
        let mut placed = other.clone();
        placed.nodes = other.nodes.iter().map(|n| place(affine, n)).collect();
        if !placed.nodes.iter().all(|n| in_range(n.x) && in_range(n.y)) {
            return Err("a path of the cap glyph lies beyond the coordinate limit".into());
        }
        *work = work.saturating_add(placed.nodes.len());
        others.push(placed);
    }
    if *work > MAX_ATTACH_WORK {
        return Err("the layer's caps would take too much work to fit".into());
    }
    Ok(others)
}

// ── Segment components and brushes ──────────────────────────────────────────────────────────────

/// A segment or brush glyph's layer for one master, as the bending reads it.
struct StrokeGlyph {
    /// A segment glyph's open path, or a brush glyph's closed path.
    path: Path,
    /// The `start` anchor, the unit direction to the `end` anchor, and their distance.
    start: Point,
    dir: Vec2,
    length: f64,
}

impl StrokeGlyph {
    fn new(glyph: &Glyph, master: &str, brush: bool) -> Result<Self, String> {
        let noun = if brush { "brush" } else { "segment" };
        let layer = glyph.layer(master).ok_or_else(|| format!("the {noun} glyph has no layer for master `{master}`"))?;
        let anchor = |name: &str| layer.anchors.iter().find(|a| a.name == name).map(|a| Point::new(a.x, a.y));
        let paths: Vec<&Path> = layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None }).collect();
        let path = *paths.first().ok_or_else(|| format!("the {noun} glyph has no paths"))?;
        if path.nodes.len() > MAX_OUTLINE_ELEMENTS {
            return Err(format!("the {noun} glyph's path has {} nodes, more than {MAX_OUTLINE_ELEMENTS}", path.nodes.len()));
        }
        if !path.nodes.iter().all(|n| in_range(n.x) && in_range(n.y)) {
            return Err(format!("the {noun} glyph's path is not a number or lies beyond the coordinate limit"));
        }
        let (start, end) = if brush {
            if !path.closed {
                return Err("the brush glyph's first path is open; a brush is drawn with one closed path".into());
            }
            let closed = paths.iter().filter(|p| p.closed).count();
            if closed > 1 {
                return Err(format!("the brush glyph has {closed} closed paths; a brush is drawn with one, so merge them with Remove Overlap"));
            }
            (anchor("start").unwrap_or(Point::ZERO), anchor("end").unwrap_or(Point::new(layer.width, 0.0)))
        } else {
            if path.closed {
                return Err("the segment glyph's first path is closed; a segment component is drawn with one open path".into());
            }
            let (Some(first), Some(last)) = (path.nodes.first(), path.nodes.last()) else {
                return Err("the segment glyph's path needs two on-curve end nodes".into());
            };
            if path.nodes.len() < 2 || !first.on_curve() || !last.on_curve() {
                return Err("the segment glyph's path needs two on-curve end nodes".into());
            }
            (anchor("start").unwrap_or(first.point()), anchor("end").unwrap_or(last.point()))
        };
        if !(in_range(start.x) && in_range(start.y) && in_range(end.x) && in_range(end.y)) {
            return Err(format!("the {noun} glyph's start or end anchor is not a number or lies beyond the coordinate limit"));
        }
        let axis = end - start;
        let length = axis.hypot();
        if !length.is_finite() || length <= MIN_STEM {
            return Err(format!("the {noun} glyph's start and end lie on one point: it has zero length"));
        }
        Ok(Self { path: path.clone(), start, dir: axis * (1.0 / length), length })
    }

    /// `p` as (distance along the start-to-end line from start, distance to its left).
    fn frame(&self, p: Point) -> (f64, f64) {
        let v = p - self.start;
        (v.dot(self.dir), self.dir.cross(v))
    }
}

/// One curve piece of a host segment.
#[derive(Clone, Copy, Debug)]
enum HostPiece {
    Line(Point, Point),
    Quad(Point, Point, Point),
    Cubic(Point, Point, Point, Point),
}

impl HostPiece {
    fn eval(self, t: f64) -> Point {
        match self {
            HostPiece::Line(a, b) => a.lerp(b, t),
            HostPiece::Quad(a, b, c) => a.lerp(b, t).lerp(b.lerp(c, t), t),
            HostPiece::Cubic(a, b, c, d) => {
                let (ab, bc, cd) = (a.lerp(b, t), b.lerp(c, t), c.lerp(d, t));
                ab.lerp(bc, t).lerp(bc.lerp(cd, t), t)
            }
        }
    }

    fn deriv(self, t: f64) -> Vec2 {
        let mt = 1.0 - t;
        match self {
            HostPiece::Line(a, b) => b - a,
            HostPiece::Quad(a, b, c) => ((b - a) * mt + (c - b) * t) * 2.0,
            HostPiece::Cubic(a, b, c, d) => ((b - a) * (mt * mt) + (c - b) * (2.0 * mt * t) + (d - c) * (t * t)) * 3.0,
        }
    }
}

/// A host segment measured by arc length.
struct HostCurve {
    pieces: Vec<HostPiece>,
    /// (piece, parameter in it, arc length from the segment's start), from (0, 0, 0) on.
    samples: Vec<(usize, f64, f64)>,
    length: f64,
}

impl HostCurve {
    /// The segment through `pts` (start node, off-curve nodes, end node); `cubic` when its end
    /// node ends a cubic segment. Two off-curve nodes of a cubic segment make a cubic; other
    /// off-curve runs make a quadratic spline with implied on-curve points between them.
    fn new(pts: &[Point], cubic: bool) -> Result<Self, String> {
        let pieces = match *pts {
            [a, b] => vec![HostPiece::Line(a, b)],
            [a, b, c] => vec![HostPiece::Quad(a, b, c)],
            [a, b, c, d] if cubic => vec![HostPiece::Cubic(a, b, c, d)],
            _ => {
                let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return Err("the host segment has no nodes".into()) };
                let offs = pts.get(1..pts.len().saturating_sub(1)).unwrap_or_default();
                let mut out = Vec::with_capacity(offs.len());
                let mut from = first;
                for (k, &o) in offs.iter().enumerate() {
                    let to = offs.get(k + 1).map_or(last, |&next| o.midpoint(next));
                    out.push(HostPiece::Quad(from, o, to));
                    from = to;
                }
                out
            }
        };
        let per = (MAX_ARC_SAMPLES / pieces.len().max(1)).clamp(1, SAMPLES_PER_PIECE);
        let mut samples = Vec::with_capacity(pieces.len().saturating_mul(per).saturating_add(1));
        samples.push((0, 0.0, 0.0));
        let mut length = 0.0;
        let mut prev = pieces.first().map_or(Point::ZERO, |p| p.eval(0.0));
        for (i, piece) in pieces.iter().enumerate() {
            for k in 1..=per {
                let t = k as f64 / per as f64;
                let p = piece.eval(t);
                length += (p - prev).hypot();
                prev = p;
                samples.push((i, t, length));
            }
        }
        if !length.is_finite() || length <= MIN_STEM {
            return Err("the host segment has zero length".into());
        }
        Ok(Self { pieces, samples, length })
    }

    /// The point at fraction `f` (0 to 1) of the arc length and the unit tangent there.
    fn at(&self, f: f64) -> (Point, Vec2) {
        let s = f.clamp(0.0, 1.0) * self.length;
        let k = self.samples.partition_point(|(_, _, l)| *l < s).max(1);
        let (Some(&lo), Some(&hi)) = (self.samples.get(k - 1), self.samples.get(k).or(self.samples.last())) else {
            return (Point::ZERO, Vec2::new(1.0, 0.0));
        };
        // A sample that ends the previous piece is where the next one starts.
        let t0 = if lo.0 == hi.0 { lo.1 } else { 0.0 };
        let span = hi.2 - lo.2;
        let w = if span > 0.0 { ((s - lo.2) / span).clamp(0.0, 1.0) } else { 0.0 };
        let t = t0 + (hi.1 - t0) * w;
        let Some(&piece) = self.pieces.get(hi.0) else { return (Point::ZERO, Vec2::new(1.0, 0.0)) };
        (piece.eval(t), self.tangent(piece, t))
    }

    /// The unit tangent of `piece` at `t`; where the derivative vanishes (a handle on its node),
    /// the direction a little further along, else the chord.
    fn tangent(&self, piece: HostPiece, t: f64) -> Vec2 {
        let nudged = if t < 0.5 { t + 1e-3 } else { t - 1e-3 };
        let chord = match (self.pieces.first(), self.pieces.last()) {
            (Some(a), Some(b)) => b.eval(1.0) - a.eval(0.0),
            _ => Vec2::new(1.0, 0.0),
        };
        let nudge = if t < 0.5 { piece.eval(nudged) - piece.eval(t) } else { piece.eval(t) - piece.eval(nudged) };
        for v in [piece.deriv(t), nudge, chord] {
            let h = v.hypot();
            if h.is_finite() && h > 1e-9 {
                return v * (1.0 / h);
            }
        }
        Vec2::new(1.0, 0.0)
    }
}

/// Where the bending puts glyph point `p` on `host`. Outside the start-to-end span a brush's
/// point moves rigidly with the host's end direction (`rigid`); a segment glyph's keeps scaling.
fn bend(host: &HostCurve, glyph: &StrokeGlyph, p: Point, rigid: bool) -> Point {
    let (u, d) = glyph.frame(p);
    let left = |t: Vec2| Vec2::new(-t.y, t.x);
    let scale = host.length / glyph.length;
    if u < 0.0 {
        let (a, t) = host.at(0.0);
        let along = if rigid { u } else { u * scale };
        return a + t * along + left(t) * d;
    }
    if u > glyph.length {
        let (b, t) = host.at(1.0);
        let along = if rigid { u - glyph.length } else { (u - glyph.length) * scale };
        return b + t * along + left(t) * d;
    }
    let (c, t) = host.at(u / glyph.length);
    c + left(t) * d
}

/// The model index of the on-curve node before `node` on the path, if a segment ends at `node`.
fn segment_start(nodes: &[Node], closed: bool, node: usize) -> Option<usize> {
    let len = nodes.len();
    let mut i = node;
    for _ in 1..len {
        i = if closed { (i + len - 1) % len } else { i.checked_sub(1)? };
        if nodes.get(i).is_some_and(Node::on_curve) {
            return Some(i);
        }
    }
    None
}

/// The host segment that ends at the node whose original model index is `end`: its start and
/// end positions in `path` now, and the curve.
fn host_segment(path: &Path, ids: &[Option<usize>], end: usize, noun: &str) -> Result<(usize, usize, HostCurve), String> {
    let len = path.nodes.len();
    let b = ids.iter().position(|i| *i == Some(end)).ok_or_else(|| format!("the {noun}'s node is no longer on the path"))?;
    let end_node = path.nodes.get(b).ok_or_else(|| format!("the {noun}'s node is not on the path"))?;
    if !end_node.on_curve() {
        return Err(format!("a {noun} sits on the on-curve node that ends a segment; this node is off-curve"));
    }
    let a = segment_start(&path.nodes, path.closed, b).ok_or_else(|| format!("no segment ends at the {noun}'s node: it starts the open path"))?;
    if a == b {
        return Err("the path has one on-curve node, so its only segment has zero length".into());
    }
    let count = if b > a { b - a + 1 } else { len - a + b + 1 };
    let pts: Vec<Point> = (0..count).filter_map(|k| path.nodes.get((a + k) % len).map(Node::point)).collect();
    Ok((a, b, HostCurve::new(&pts, end_node.kind == NodeKind::Curve)?))
}

/// Adds the cost of moving `nodes` nodes to `work`; refuses past [`MAX_ATTACH_WORK`].
fn charge(work: &mut usize, nodes: usize) -> Result<(), String> {
    *work = work.saturating_add(nodes);
    if *work > MAX_ATTACH_WORK {
        return Err("the layer's caps, segment components and brushes would take too much work to fit".into());
    }
    Ok(())
}

/// `node` moved to `p` and rounded; refused beyond the coordinate limit.
fn moved(node: &Node, p: Point) -> Result<Node, String> {
    let mut out = node.clone();
    (out.x, out.y) = (ot_round(p.x), ot_round(p.y));
    if !(in_range(out.x) && in_range(out.y)) {
        return Err("the bent outline is not a number or lies beyond the coordinate limit".into());
    }
    Ok(out)
}

/// Replaces the segment of `path` that ends at original model node `end` (found through `ids`,
/// which follow the nodes) with the segment glyph's path, bent along it. The glyph's end nodes
/// take the places of the segment's end nodes, which keep their kind on the start side and
/// take the glyph's on the end side.
fn fit_segment_into(path: &mut Path, ids: &mut Vec<Option<usize>>, glyph: &StrokeGlyph, end: usize, work: &mut usize) -> Result<(), String> {
    let (a, b, host) = host_segment(path, ids, end, "segment component")?;
    let len = path.nodes.len();
    let added = glyph.path.nodes.len();
    charge(work, len.saturating_add(added))?;
    let removed = if b > a { b - a + 1 } else { len - a + b + 1 };
    if (len - removed).saturating_add(added) > MAX_OUTLINE_ELEMENTS {
        return Err(format!("the path with its segment components has more than {MAX_OUTLINE_ELEMENTS} nodes"));
    }
    let mut bent = Vec::with_capacity(added);
    for n in &glyph.path.nodes {
        bent.push(moved(n, bend(&host, glyph, n.point(), false))?);
    }
    let (Some(first), Some(last)) = (bent.first(), bent.last()) else { return Err("the segment glyph's path is empty".into()) };
    let (Some(start_node), Some(end_node)) = (path.nodes.get(a), path.nodes.get(b)) else {
        return Err("the segment's nodes are not on the path".into());
    };
    let start = Node { x: first.x, y: first.y, ..start_node.clone() };
    let finish = Node { x: last.x, y: last.y, kind: last.kind, ..end_node.clone() };
    let middle = bent.get(1..added.saturating_sub(1)).unwrap_or_default();
    let mut nodes: Vec<(Node, Option<usize>)> = path.nodes.drain(..).zip(ids.drain(..)).collect();
    // Rotate so the segment starts at index 0 and ends at `to`.
    nodes.rotate_left(a);
    let to = (b + len - a) % len;
    let (Some(&(_, a_id)), Some(&(_, b_id))) = (nodes.first(), nodes.get(to)) else { return Err("the segment's nodes are not on the path".into()) };
    let mut out: Vec<(Node, Option<usize>)> = Vec::with_capacity(len - removed + added);
    out.push((start, a_id));
    out.extend(middle.iter().cloned().map(|n| (n, None)));
    out.push((finish, b_id));
    out.extend(nodes.get(to + 1..).unwrap_or_default().iter().cloned());
    // Back to the path's own start. A segment that wraps past the start of a closed path held
    // it: its end node starts the path again when it was the start, else the first bent node.
    let count = out.len();
    if a > b {
        let at = if b == 0 { 1 + middle.len() } else { 1 };
        out.rotate_left(at.min(count));
    } else {
        out.rotate_right(a.min(count));
    }
    (path.nodes, *ids) = out.into_iter().unzip();
    Ok(())
}

/// The brush's closed outline drawn along the segment of `path` that ends at original model node
/// `end`; the path itself does not change.
fn brush_path(path: &Path, ids: &[Option<usize>], glyph: &StrokeGlyph, end: usize, work: &mut usize) -> Result<Path, String> {
    let (_, _, host) = host_segment(path, ids, end, "brush")?;
    charge(work, glyph.path.nodes.len())?;
    let mut out = glyph.path.clone();
    let mut nodes = Vec::with_capacity(glyph.path.nodes.len());
    for n in &glyph.path.nodes {
        nodes.push(moved(n, bend(&host, glyph, n.point(), true))?);
    }
    out.nodes = nodes;
    Ok(out)
}

#[cfg(test)]
#[path = "attach_tests.rs"]
mod tests;
