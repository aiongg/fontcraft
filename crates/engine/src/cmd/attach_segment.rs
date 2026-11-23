//! Segment components (`segmentComponent.*`) and brushes (`brush.*`): attach a segment glyph or
//! a brush glyph to path segments, decompose them and swap their glyph.
//!
//! Each is a `Segment` or `Brush` hint on the layer (`fontcraft_model::attach`) that names the
//! on-curve node ending its host segment; outline resolution bends the glyph along that segment.
//! Commands address them by their position in the layer's hint list (`segmentComponents` or
//! `brushes: [indices]`), segments by the node that ends them and nodes in the model's order
//! (`segments`, `points: [[shape, node]]`); the hint counts a closed path's nodes in the file's
//! order, start node last, as a corner's does. They live on the foreground only. An edit keeps
//! every other key of the stored hint and its spelling.

use fontcraft_model::attach::{self, Attachment, BRUSH_PREFIX, SEGMENT_PREFIX, SegmentHint};
use fontcraft_model::plist::{Str, Value as Plist};
use fontcraft_model::{Font, Hint, Layer, LayerSide, MAX_LAYER_HINTS, MAX_NAME_LEN, Shape};
use serde_json::{Value, json};

use super::corner::{file_index, set_entry};
use super::*;
use crate::params::{Params, TargetKind, Targets};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "segmentComponent.add",
            "Add Segment Component",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), segments: [[shape, node]] (each segment by the on-curve node that ends it), segmentGlyph: name} attaches the segment glyph `segmentGlyph` (its name starts with _segment.) to each listed segment: its open path takes the segment's place, scaled along a straight segment and bent along a curved one without adding nodes. A segment that already holds a segment component gets the new one in its place. Foreground only; at most 4,096 hints per layer. A locked glyph refuses it → {glyph, layer, segmentComponents: [hint indices], replaced: number, warning?: why the layer's attachments cannot be fitted, selection: {glyph, layer, segmentComponents}}",
            has_doc,
            segment_add
        )
        .targets(&[TargetKind::Segments]),
        cmd!(
            "segmentComponent.decompose",
            "Decompose Segment Component",
            [],
            None,
            "{glyph: name, layer?: id, segmentComponents?: [hint indices] (default: every segment component of the layer)} replaces each chosen host segment with the bent path and removes the component; the other attachments and corners keep their nodes. One that cannot be fitted (its glyph is missing, it has zero length, its node starts an open path) is an error and nothing changes → {glyph, layer, decomposed: number, selection: {glyph, layer}}",
            has_doc,
            segment_decompose
        ),
        cmd!(
            "segmentComponent.setGlyph",
            "Replace Segment Component",
            [],
            None,
            "{glyph: name, layer?: id, segmentComponents: [hint indices], segmentGlyph: name} gives the listed segment components the segment glyph `segmentGlyph`, keeping their segments → {glyph, layer, segmentComponents, warning?, selection: {glyph, layer, segmentComponents}}",
            has_doc,
            segment_set_glyph
        ),
        cmd!(
            "brush.add",
            "Add Brush",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), segments?: [[shape, node]], points?: [[shape, node]], brush: name} attaches the brush glyph `brush` (its name starts with _brush.) to each listed segment, and to the segment ending at each listed node; give at least one. The brush's closed path is drawn along the segment: the part between its start and end anchors stretches and bends, the rest keeps its shape. A node that ends no segment (the first node of an open path, an off-curve node) is skipped and listed. A segment that already holds a brush gets the new one in its place. Foreground only; at most 4,096 hints per layer. A locked glyph refuses it → {glyph, layer, brushes: [hint indices], replaced: number, skipped: [[shape, node]], warning?, selection: {glyph, layer, brushes}}",
            has_doc,
            brush_add
        )
        .targets(&[TargetKind::Segments, TargetKind::Points]),
        cmd!(
            "brush.decompose",
            "Decompose Brush",
            [],
            None,
            "{glyph: name, layer?: id, brushes?: [hint indices] (default: every brush of the layer)} adds the outline each chosen brush draws to the layer as a path and removes the brush; its host path stays. A brush that cannot be fitted (its glyph is missing or has more than one closed path, it has zero length) is an error and nothing changes → {glyph, layer, decomposed: number, selection: {glyph, layer}}",
            has_doc,
            brush_decompose
        ),
        cmd!(
            "brush.setGlyph",
            "Replace Brush",
            [],
            None,
            "{glyph: name, layer?: id, brushes: [hint indices], brush: name} gives the listed brushes the brush glyph `brush`, keeping their segments → {glyph, layer, brushes, warning?, selection: {glyph, layer, brushes}}",
            has_doc,
            brush_set_glyph
        ),
    ]
}

// ── The two kinds ───────────────────────────────────────────────────────────────────────────────

/// What differs between segment components and brushes.
#[derive(Clone, Copy)]
struct Kind {
    brush: bool,
}

const SEGMENT: Kind = Kind { brush: false };
const BRUSH: Kind = Kind { brush: true };

impl Kind {
    /// The parameter and result key listing hint indices.
    fn list_key(self) -> &'static str {
        if self.brush { "brushes" } else { "segmentComponents" }
    }

    /// The parameter naming the glyph.
    fn glyph_key(self) -> &'static str {
        if self.brush { "brush" } else { "segmentGlyph" }
    }

    fn noun(self) -> &'static str {
        if self.brush { "brush" } else { "segment component" }
    }

    fn glyph_noun(self) -> &'static str {
        if self.brush { "brush glyph" } else { "segment glyph" }
    }

    fn prefix(self) -> &'static str {
        if self.brush { BRUSH_PREFIX } else { SEGMENT_PREFIX }
    }

    fn is(self, hint: &Hint) -> bool {
        if self.brush { attach::is_brush(hint) } else { attach::is_segment(hint) }
    }

    fn is_name(self, name: &str) -> bool {
        if self.brush { attach::is_brush_name(name) } else { attach::is_segment_name(name) }
    }

    fn hint(self, s: SegmentHint) -> Hint {
        if self.brush { Attachment::Brush(s).to_hint() } else { Attachment::Segment(s).to_hint() }
    }

    /// The segment (path, file-order end node) of each hint of this kind; `None` for others.
    fn targets(self, hints: &[Hint]) -> Vec<Option<(usize, usize)>> {
        hints.iter().map(|h| if self.is(h) { attach::segment_hint(h).ok().flatten().map(|s| (s.shape, s.node)) } else { None }).collect()
    }
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// Refuses `name` unless it is a glyph of `kind` in `font` other than `host`.
fn check_glyph(p: &Params, font: &Font, kind: Kind, name: &str, host: &str) -> Result<()> {
    if !kind.is_name(name) || name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`{name}` is not a {}: its name starts with {}", kind.glyph_noun(), kind.prefix())));
    }
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("there is no glyph `{name}`")));
    }
    if name == host {
        return Err(p.bad(format!("`{name}` cannot be attached to `{name}` itself")));
    }
    Ok(())
}

/// The target object, refused on the background and with target lists other than `segments`
/// and, when `points_allowed`, `points`.
fn foreground_targets(p: &Params, kind: Kind, segments_allowed: bool, points_allowed: bool) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.side != LayerSide::Foreground {
        return Err(p.bad(format!("{}s sit on the foreground only", kind.noun())));
    }
    let others = !(t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty());
    if others || (!segments_allowed && !t.segments.is_empty()) || (!points_allowed && !t.points.is_empty()) {
        let why = match (segments_allowed, points_allowed) {
            (true, true) => "takes `segments` and `points` and no other target list".to_string(),
            (true, false) => "takes `segments` and no other target list".to_string(),
            _ => format!("addresses {}s with `{}` and takes no target lists", kind.noun(), kind.list_key()),
        };
        return Err(p.bad(why));
    }
    Ok(t)
}

/// The hint indices under `kind`'s list key: at least one, each once.
fn index_list(p: &Params, kind: Kind) -> Result<Vec<usize>> {
    let key = kind.list_key();
    let list = p.array(key)?;
    if list.is_empty() {
        return Err(p.bad(format!("`{key}` must not be empty")));
    }
    if list.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`{key}` names {} hints; a layer holds at most {MAX_LAYER_HINTS}", list.len())));
    }
    let mut out = Vec::new();
    for v in list {
        let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`{key}` must hold hint indices, found {v}")))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// Refuses each index of `list` that is not a hint of `kind` on `layer`.
fn check_kind(p: &Params, kind: Kind, layer: &Layer, list: &[usize]) -> Result<()> {
    let key = kind.list_key();
    let count = layer.info.hints.len();
    for &i in list {
        let hint = layer.info.hints.get(i).ok_or_else(|| p.bad(format!("`{key}`: {i} does not resolve: the layer has {count} hints")))?;
        if !kind.is(hint) {
            return Err(p.bad(format!("`{key}`: hint {i} is not a {}", kind.noun())));
        }
        attach::segment_hint(hint).map_err(|m| p.bad(format!("`{key}`: hint {i}: {m}")))?;
    }
    Ok(())
}

/// Why the layer's attachments cannot be fitted, if they cannot.
fn fit_warning(font: &Font, glyph: &str, layer: &str) -> Option<String> {
    let l = font.glyph(glyph)?.layers.iter().find(|l| l.id == layer)?;
    attach::insert_attachments(font, l).err().map(|e| e.to_string())
}

/// The common part of a result: glyph, layer, the hint list, warning and selection.
fn result(s: &Session, p: &Params, kind: Kind, t: &Targets, layer: &str, list: &[usize]) -> Result<Value> {
    let mut selection = Targets::new(p.command(), t.glyph.clone(), Some(layer.to_string())).to_json();
    if let Some(o) = selection.as_object_mut() {
        o.insert(kind.list_key().into(), json!(list));
    }
    let mut out = json!({ "glyph": t.glyph, "layer": layer, "selection": selection });
    if let Some(o) = out.as_object_mut() {
        o.insert(kind.list_key().into(), json!(list));
        if let Some(w) = fit_warning(s.font()?, &t.glyph, layer) {
            o.insert("warning".into(), json!(w));
        }
    }
    Ok(out)
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────

fn segment_add(s: &mut Session, v: &Value) -> Result<Value> {
    add(s, &Params::new("segmentComponent.add", v)?, SEGMENT, "Add Segment Component")
}

fn brush_add(s: &mut Session, v: &Value) -> Result<Value> {
    add(s, &Params::new("brush.add", v)?, BRUSH, "Add Brush")
}

fn segment_decompose(s: &mut Session, v: &Value) -> Result<Value> {
    decompose(s, &Params::new("segmentComponent.decompose", v)?, SEGMENT, "Decompose Segment Component")
}

fn brush_decompose(s: &mut Session, v: &Value) -> Result<Value> {
    decompose(s, &Params::new("brush.decompose", v)?, BRUSH, "Decompose Brush")
}

fn segment_set_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    set_glyph(s, &Params::new("segmentComponent.setGlyph", v)?, SEGMENT, "Replace Segment Component")
}

fn brush_set_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    set_glyph(s, &Params::new("brush.setGlyph", v)?, BRUSH, "Replace Brush")
}

fn add(s: &mut Session, p: &Params, kind: Kind, label: &str) -> Result<Value> {
    let name = p.str(kind.glyph_key())?.to_string();
    let mut t = foreground_targets(p, kind, true, kind.brush)?;
    if t.segments.is_empty() && t.points.is_empty() {
        return Err(p.bad(if kind.brush {
            "give the segments as `segments` or their end nodes as `points`"
        } else {
            "give the segments as `segments`"
        }));
    }
    check_glyph(p, s.font()?, kind, &name, &t.glyph)?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    let (indices, replaced, skipped) = s.edit(label, |font| {
        t.edit_layer(font, |l| {
            let mut ends: Vec<(usize, usize)> = t.segments.clone();
            let mut skipped: Vec<(usize, usize)> = Vec::new();
            for &(shape, node) in &t.points {
                let Some(Shape::Path(path)) = l.shapes.get(shape) else { return Err(p.bad(format!("`points`: shape {shape} is not a path"))) };
                if !path.ends_segment(node) {
                    skipped.push((shape, node));
                } else if !ends.contains(&(shape, node)) {
                    ends.push((shape, node));
                }
            }
            if ends.is_empty() {
                return Err(p.bad("no listed node ends a segment: the first node of an open path and off-curve nodes end none"));
            }
            let mut stored = Vec::with_capacity(ends.len());
            for (shape, node) in ends {
                let Some(Shape::Path(path)) = l.shapes.get(shape) else { return Err(p.bad(format!("shape {shape} is not a path"))) };
                let file = file_index(path.closed, path.nodes.len(), node);
                if !stored.contains(&(shape, file)) {
                    stored.push((shape, file));
                }
            }
            let existing = kind.targets(&l.info.hints);
            let (mut indices, mut replaced) = (Vec::with_capacity(stored.len()), 0usize);
            for (shape, file) in stored {
                let hint = kind.hint(SegmentHint::new(name.clone(), shape, file));
                if let Some(i) = existing.iter().position(|c| *c == Some((shape, file)))
                    && let Some(slot) = l.info.hints.get_mut(i)
                {
                    *slot = hint;
                    indices.push(i);
                    replaced += 1;
                    continue;
                }
                if l.info.hints.len() >= MAX_LAYER_HINTS {
                    return Err(p.bad(format!("the layer already holds {MAX_LAYER_HINTS} hints, the most allowed")));
                }
                l.info.hints.push(hint);
                indices.push(l.info.hints.len() - 1);
            }
            Ok((indices, replaced, skipped))
        })
    })?;
    let mut out = result(s, p, kind, &t, &layer, &indices)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("replaced".into(), json!(replaced));
        if kind.brush {
            o.insert("skipped".into(), Value::Array(skipped.iter().map(|(s, n)| json!([s, n])).collect()));
        }
    }
    Ok(out)
}

fn decompose(s: &mut Session, p: &Params, kind: Kind, label: &str) -> Result<Value> {
    let mut t = foreground_targets(p, kind, false, false)?;
    let chosen = match p.opt_array(kind.list_key())? {
        Some(_) => Some(index_list(p, kind)?),
        None => None,
    };
    let count = s.edit(label, |font| {
        let (layer, count) = {
            let r = t.resolve(font)?;
            crate::check_unlocked(r.glyph)?;
            let which: Vec<usize> = match &chosen {
                Some(list) => {
                    check_kind(p, kind, r.layer, list)?;
                    list.clone()
                }
                None => r.layer.info.hints.iter().enumerate().filter(|(_, h)| kind.is(h)).map(|(i, _)| i).collect(),
            };
            if which.is_empty() {
                return Err(p.bad(format!("the layer has no {}s", kind.noun())));
            }
            let layer = attach::decompose_attachments(font, r.layer, &which).map_err(|e| p.bad(e.to_string()))?;
            (layer, which.len())
        };
        t.edit_layer(font, |l| {
            *l = layer;
            Ok(())
        })?;
        Ok(count)
    })?;
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "decomposed": count, "selection": t.to_json()}))
}

fn set_glyph(s: &mut Session, p: &Params, kind: Kind, label: &str) -> Result<Value> {
    let name = p.str(kind.glyph_key())?.to_string();
    let mut t = foreground_targets(p, kind, false, false)?;
    check_glyph(p, s.font()?, kind, &name, &t.glyph)?;
    let list = index_list(p, kind)?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    s.edit(label, |font| {
        t.edit_layer(font, |l| {
            check_kind(p, kind, l, &list)?;
            for &i in &list {
                if let Some(Plist::Dict(d)) = l.info.hints.get_mut(i).map(|h| &mut h.value) {
                    set_entry(d, "name", Some(Plist::String(Str::new(name.clone()))));
                }
            }
            Ok(())
        })
    })?;
    result(s, p, kind, &t, &layer, &list)
}
