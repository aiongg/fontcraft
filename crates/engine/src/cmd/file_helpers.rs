//! Revert Glyph, Copy Layer to Layer, Paste After, Compare Layers. Rows: `file.revertGlyph`,
//! `edit.copyLayerToLayer`, `edit.pasteAfter`, `font.compareLayers`.
//!
//! Revert Glyph reads the document's saved font ([`crate::DocState::saved`]): the font as it was
//! last read from or written to disk, or as it was created for a document never saved.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_model::{Font, Glyph, Layer, MAX_COORDINATE, MAX_GLYPHS, ModelError, Shape};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind, Targets};

/// Most layer ids one `file.revertGlyph` call takes.
const MAX_LAYER_IDS: usize = 4096;
/// Largest copied-glyphs text `edit.pasteAfter` reads, in bytes (as `edit.paste` allows).
const MAX_CLIP_BYTES: usize = 64 << 20;
/// Most masters the copied-glyphs text may describe.
const MAX_CLIP_MASTERS: usize = 4096;
/// The newest copied-glyphs version `edit.pasteAfter` reads.
const CLIP_VERSION: u32 = 1;
/// Most differences `font.compareLayers` describes in `details`; `different` lists them all.
const MAX_DETAILS: usize = 10_000;
/// Two transform coefficients (scale, rotation, slant) closer than this are equal.
const LINEAR_EPSILON: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.revertGlyph",
            "Revert Glyph",
            ["File"],
            None,
            "{glyphs?: [names] (default: every glyph that differs from the saved file), layers?: [layer ids]} puts the glyphs back as they are in the saved file (for a document never saved: as it was created), as one undo step; every other unsaved change stays. Without `layers` the whole glyph is restored, and a glyph deleted since the save comes back at its saved position. With `layers` only those layers of each glyph are restored: a layer added since the save is removed, one deleted is put back, and glyphs that have none of the layers are left alone. A named glyph the saved file lacks is refused. Glyphs added since the save are never touched → {reverted: [names], unchanged: [names]}",
            has_doc,
            revert_glyph
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "edit.copyLayerToLayer",
            "Copy Layer to Layer…",
            ["Edit"],
            None,
            "{glyphs?: [names] (default: every glyph), from: layer id | fromName: layer name, to: layer id | toName: layer name, mode?: \"replace\" | \"append\" (default replace)} copies the paths and components of layer `from` into layer `to` of each glyph, as one undo step. A name stands for each glyph's first layer of that name that is not a master layer, or for the master layer of the master called so. replace removes the paths and components `to` had (its images and other shapes stay); append adds the copies after them. Widths, anchors and the background are not copied. A glyph without both layers is skipped and listed; a locked glyph refuses the command; when no glyph has both layers the command fails → {copied: [names], skipped: [{glyph, reason}]}",
            has_doc,
            copy_layer_to_layer
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "edit.pasteAfter",
            "Paste After",
            ["Edit"],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), json?: string, text?: string} pastes copied glyphs (the text edit.copy returns with `glyphs`) into the layer after its content: each copied glyph's paths and components are moved right by the layer's advance width, then the advance grows by that glyph's width, so several copied glyphs line up as a word. Each copied glyph gives its layer of the same master (by id, else by name), else of the master shown when copying, else its first master layer. Images and shapes the model keeps unread are left out. A component of the glyph itself, a component loop, or a locked glyph refuses it; a drawing fragment is refused: copy whole glyphs → {pasted: [names], width: the new advance, skipped?: shapes left out, selection}",
            has_doc,
            paste_after
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "font.compareLayers",
            "Compare Layers…",
            ["Edit"],
            None,
            "{glyphs?: [names] (default: every glyph), layerA: layer id | nameA: layer name, layerB: layer id | nameB: layer name, tolerance?: units (default 0)} compares two layers of each glyph (a name stands for each glyph's first layer of that name that is not a master layer, or for the master layer of the master called so): the advance widths, and the paths and components in order (each node's kind and position, each component's glyph and transform). Positions, widths and component offsets may differ by up to `tolerance` on each axis. A glyph without both layers is listed in `missing`; when no glyph has both the command fails. The shell selects `different` and scrolls to `first` → {different: [names], first?: name, same: count, missing: [names], details: [{glyph, reason}]}",
            has_doc,
            compare_layers
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

// ── Shared ──────────────────────────────────────────────────────────────────────────────────────

/// `glyphs` as names that exist in `font`, or every glyph of the font.
fn glyph_names(p: &Params, font: &Font) -> Result<Vec<String>> {
    match p.opt_glyphs("glyphs", font)? {
        Some(names) => Ok(names),
        None => Ok(font.glyphs.iter().map(|g| g.name.clone()).collect()),
    }
}

fn find_layer<'g>(glyph: &'g Glyph, id: &str) -> Option<&'g Layer> {
    glyph.layers.iter().find(|l| l.id == id)
}

/// A layer of each glyph, by id or by name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum LayerRef {
    Id(String),
    Name(String),
}

impl LayerRef {
    /// From the parameter `id`, else `name`; one of the two is needed.
    fn read(p: &Params, id: &str, name: &str) -> Result<LayerRef> {
        match (p.opt_str(id)?, p.opt_str(name)?) {
            (Some(v), None) => Ok(LayerRef::Id(v.to_string())),
            (None, Some(v)) => Ok(LayerRef::Name(v.to_string())),
            (Some(_), Some(_)) => Err(p.bad(format!("give only one of `{id}` and `{name}`"))),
            (None, None) => Err(p.bad(format!("`{id}` or `{name}` is required"))),
        }
    }

    fn find<'g>(&self, font: &Font, glyph: &'g Glyph) -> Option<&'g Layer> {
        match self {
            LayerRef::Id(id) => find_layer(glyph, id),
            LayerRef::Name(name) => {
                let named = glyph.layers.iter().find(|l| !l.is_master_layer() && l.info.name.as_deref() == Some(name.as_str()));
                let master = || font.masters.iter().find(|m| m.name == *name).and_then(|m| glyph.layer(&m.id));
                named.or_else(master)
            }
        }
    }

    fn text(&self) -> &str {
        match self {
            LayerRef::Id(text) | LayerRef::Name(text) => text,
        }
    }
}

fn same_glyph(a: &Arc<Glyph>, b: &Arc<Glyph>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}

fn is_outline(shape: &Shape) -> bool {
    matches!(shape, Shape::Path(_) | Shape::Component(_))
}

// ── Revert Glyph ────────────────────────────────────────────────────────────────────────────────

/// What one glyph's revert does.
enum Revert {
    /// Replace the glyph; with a position, put a deleted one back there.
    Whole { glyph: Glyph, restore_at: Option<usize> },
    /// Restore some layers.
    Layers { name: String, layers: Vec<Layer> },
}

fn revert_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.revertGlyph", p)?;
    let saved = Arc::clone(&s.doc()?.saved);
    let font = s.font()?;

    let names: Vec<String> = match p.opt_strings("glyphs")? {
        Some(list) => {
            if list.len() > MAX_GLYPHS {
                return Err(p.bad(format!("`glyphs` holds {} names; at most {MAX_GLYPHS} are allowed", list.len())));
            }
            let mut seen = HashSet::new();
            let mut out = Vec::with_capacity(list.len());
            for name in list {
                if saved.glyph(name).is_none() {
                    let why = if font.glyph(name).is_some() { "it was added since the save" } else { "there is no such glyph" };
                    return Err(p.bad(format!("`glyphs`: the saved file has no glyph `{name}`: {why}")));
                }
                if seen.insert(name) {
                    out.push(name.to_string());
                }
            }
            out
        }
        // Every glyph both fonts have that is not the same.
        None => font.glyphs.iter().filter(|g| saved.glyph(&g.name).is_some_and(|old| !same_glyph(old, g))).map(|g| g.name.clone()).collect(),
    };

    let layer_ids: Option<Vec<&str>> = p.opt_strings("layers")?;
    if let Some(ids) = &layer_ids {
        if ids.len() > MAX_LAYER_IDS {
            return Err(p.bad(format!("`layers` holds {} ids; at most {MAX_LAYER_IDS} are allowed", ids.len())));
        }
        if ids.is_empty() {
            return Err(p.bad("`layers` is empty"));
        }
    }

    let mut plan = Vec::new();
    let mut reverted = Vec::new();
    let mut unchanged = Vec::new();
    let mut any_layer = false;
    for name in &names {
        let Some(old) = saved.glyph(name) else { continue };
        let current = font.glyph(name);
        match &layer_ids {
            None => {
                if current.is_some_and(|g| same_glyph(g, old)) {
                    unchanged.push(name.clone());
                    continue;
                }
                let restore_at = if current.is_some() { None } else { Some(saved.glyphs.position(name).unwrap_or(usize::MAX)) };
                plan.push(Revert::Whole { glyph: Glyph::clone(old), restore_at });
                reverted.push(name.clone());
            }
            Some(ids) => {
                let Some(current) = current else {
                    return Err(p.bad(format!("glyph `{name}` was deleted since the save: revert it without `layers`")));
                };
                let mut layers = current.layers.clone();
                for id in ids {
                    let before = find_layer(old, id);
                    let now = layers.iter().position(|l| l.id == *id);
                    match (before, now) {
                        (Some(b), Some(i)) => {
                            any_layer = true;
                            if let Some(slot) = layers.get_mut(i) {
                                *slot = b.clone();
                            }
                        }
                        (Some(b), None) => {
                            any_layer = true;
                            let at = old.layers.iter().position(|l| l.id == *id).unwrap_or(layers.len()).min(layers.len());
                            layers.insert(at, b.clone());
                        }
                        (None, Some(i)) => {
                            any_layer = true;
                            layers.remove(i);
                        }
                        (None, None) => {}
                    }
                }
                if layers == current.layers {
                    unchanged.push(name.clone());
                } else {
                    plan.push(Revert::Layers { name: name.clone(), layers });
                    reverted.push(name.clone());
                }
            }
        }
    }
    if let Some(ids) = &layer_ids
        && !any_layer
        && !names.is_empty()
    {
        return Err(p.bad(format!("none of the glyphs has the layers {ids:?}, in the document or in the saved file")));
    }

    if !plan.is_empty() {
        s.edit("Revert Glyph", |font| {
            for r in plan {
                match r {
                    Revert::Whole { glyph, restore_at: None } => {
                        let name = glyph.name.clone();
                        *font.glyph_mut(&name)? = glyph;
                    }
                    Revert::Whole { glyph, restore_at: Some(at) } => font.glyphs.insert(at, glyph),
                    Revert::Layers { name, layers } => font.glyph_mut(&name)?.layers = layers,
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({"reverted": reverted, "unchanged": unchanged}))
}

// ── Copy Layer to Layer ─────────────────────────────────────────────────────────────────────────

fn copy_layer_to_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("edit.copyLayerToLayer", p)?;
    let from = LayerRef::read(&p, "from", "fromName")?;
    let to = LayerRef::read(&p, "to", "toName")?;
    if from == to {
        return Err(p.bad("`from` and `to` are the same layer"));
    }
    let append = match p.opt_str("mode")? {
        None | Some("replace") => false,
        Some("append") => true,
        Some(other) => return Err(p.bad(format!("`mode` must be \"replace\" or \"append\", got \"{other}\""))),
    };
    let font = s.font()?;
    let names = glyph_names(&p, font)?;

    // Each glyph with the id of the layer that takes the shapes.
    let mut plan: Vec<(String, String, Vec<Shape>)> = Vec::new();
    let mut skipped = Vec::new();
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        let (Some(src), Some(dst)) = (from.find(font, g), to.find(font, g)) else {
            let missing = if from.find(font, g).is_none() { &from } else { &to };
            skipped.push(json!({"glyph": name, "reason": format!("it has no layer `{}`", missing.text())}));
            continue;
        };
        if src.id == dst.id {
            skipped.push(json!({"glyph": name, "reason": "the two layers are the same"}));
            continue;
        }
        let mut shapes: Vec<Shape> = if append { dst.shapes.clone() } else { dst.shapes.iter().filter(|s| !is_outline(s)).cloned().collect() };
        shapes.extend(src.shapes.iter().filter(|s| is_outline(s)).cloned());
        if shapes != dst.shapes {
            crate::check_unlocked(g)?;
        }
        plan.push((name.clone(), dst.id.clone(), shapes));
    }
    if plan.is_empty() {
        return Err(p.bad(format!("no glyph has both layer `{}` and layer `{}`", from.text(), to.text())));
    }
    let copied: Vec<String> = plan.iter().map(|(n, ..)| n.clone()).collect();
    s.edit("Copy Layer to Layer", |font| {
        for (name, to, shapes) in plan {
            let mut g = font.glyph_mut(&name)?;
            let layer =
                g.layers.iter_mut().find(|l| l.id == to).ok_or_else(|| EngineError::Internal(format!("layer `{to}` of `{name}` vanished")))?;
            layer.shapes = shapes;
        }
        Ok(())
    })?;
    Ok(json!({"copied": copied, "skipped": skipped}))
}

// ── Paste After ─────────────────────────────────────────────────────────────────────────────────

/// The part of the copied-glyphs text (`edit.copy` with `glyphs`) that Paste After reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Clip {
    format: String,
    version: u32,
    #[serde(default)]
    masters: Vec<ClipMaster>,
    #[serde(default)]
    active_master: Option<String>,
    glyphs: Vec<ClipGlyph>,
}

#[derive(Deserialize)]
struct ClipMaster {
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct ClipGlyph {
    glyph: Glyph,
}

fn read_clip(p: &Params) -> Result<Clip> {
    let text = match super::glyph_set::glyphs_text(p)? {
        Some(t) => t,
        None if p.get("json").is_some() || p.get("text").is_some() => {
            return Err(p.bad("Paste After takes whole copied glyphs (edit.copy with `glyphs`), not a drawing"));
        }
        None => return Err(p.bad("nothing to paste: give the copied glyphs in `json` or `text`")),
    };
    if text.len() > MAX_CLIP_BYTES {
        return Err(p.bad(format!("the copied glyphs are {} bytes; at most {MAX_CLIP_BYTES} are allowed", text.len())));
    }
    let clip: Clip = serde_json::from_str(text).map_err(|e| p.bad(format!("the copied glyphs cannot be read: {e}")))?;
    if clip.format != super::glyph_set::GLYPHS_FORMAT {
        return Err(p.bad("the clipboard text does not hold copied glyphs"));
    }
    if clip.version > CLIP_VERSION {
        return Err(p.bad(format!("the copied glyphs are version {}; this build reads up to {CLIP_VERSION}", clip.version)));
    }
    if clip.glyphs.is_empty() {
        return Err(p.bad("the clipboard text holds no glyphs"));
    }
    if clip.glyphs.len() > MAX_GLYPHS {
        return Err(p.bad(format!("the clipboard text holds {} glyphs; at most {MAX_GLYPHS} are allowed", clip.glyphs.len())));
    }
    if clip.masters.len() > MAX_CLIP_MASTERS {
        return Err(p.bad(format!("the clipboard text describes {} masters; at most {MAX_CLIP_MASTERS} are allowed", clip.masters.len())));
    }
    Ok(clip)
}

/// The layer of a copied glyph that pastes into a layer of `master_id` (named `master_name`).
fn source_layer<'g>(clip: &Clip, glyph: &'g Glyph, master_id: &str, master_name: &str) -> Option<&'g Layer> {
    let master_layer = |id: &str| glyph.layers.iter().find(|l| l.is_master_layer() && l.id == id);
    let same_id = clip.masters.iter().any(|m| m.id == master_id).then_some(master_id);
    let same_name = clip.masters.iter().find(|m| !m.name.is_empty() && m.name == master_name).map(|m| m.id.as_str());
    [same_id, same_name, clip.active_master.as_deref()]
        .into_iter()
        .flatten()
        .find_map(master_layer)
        .or_else(|| glyph.layers.iter().find(|l| l.is_master_layer()))
        .or_else(|| glyph.layers.first())
}

fn paste_after(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("edit.pasteAfter", p)?;
    let glyph = p.str("glyph")?;
    let clip = read_clip(&p)?;
    let mut t = Targets::new("edit.pasteAfter", glyph, p.opt_str("layer")?.map(String::from));

    let font = s.font()?;
    let r = t.resolve(font)?;
    crate::check_unlocked(r.glyph)?;
    let (layer_id, master_id) = (r.layer.id.clone(), r.layer.master.clone());
    let mut width = r.layer.width;
    let first_shape = r.layer.shapes.len();

    let mut added: Vec<Shape> = Vec::new();
    let mut pasted = Vec::with_capacity(clip.glyphs.len());
    let mut skipped = 0usize;
    for c in &clip.glyphs {
        let Some(src) = source_layer(&clip, &c.glyph, &master_id, &r.master.name) else {
            skipped = skipped.saturating_add(1);
            continue;
        };
        if src.shapes.iter().any(|sh| matches!(sh, Shape::Component(comp) if comp.glyph == glyph)) {
            return Err(p.bad(format!("`{}` holds a component of `{glyph}`, which cannot be pasted into `{glyph}` itself", c.glyph.name)));
        }
        let mut part = Layer::new(master_id.as_str(), 0.0);
        part.shapes = src.shapes.iter().filter(|sh| is_outline(sh)).cloned().collect();
        skipped = skipped.saturating_add(src.shapes.len().saturating_sub(part.shapes.len()));
        super::shared::shift_layer(&mut part, width).map_err(|e| p.bad(format!("`{}` cannot be placed: {e}", c.glyph.name)))?;
        added.extend(part.shapes);
        width += src.width;
        if !width.is_finite() || width.abs() > MAX_COORDINATE {
            return Err(p.bad(format!("the advance width would leave the range ±{MAX_COORDINATE}")));
        }
        pasted.push(c.glyph.name.clone());
    }
    if pasted.is_empty() {
        return Err(p.bad("none of the copied glyphs has a layer to paste"));
    }

    let shape_count = added.len();
    t.layer = Some(layer_id.clone());
    s.edit("Paste After", |font| {
        t.edit_layer(font, |layer| {
            layer.shapes.extend(added);
            layer.width = width;
            Ok(())
        })?;
        // A pasted component may lead back to this glyph through other glyphs.
        let g = font.glyph(glyph).ok_or_else(|| EngineError::Internal(format!("glyph `{glyph}` vanished")))?;
        match font.outline_of(g, &master_id) {
            Err(e @ ModelError::ComponentCycle(_)) => Err(EngineError::BadParams { command: "edit.pasteAfter".into(), message: e.to_string() }),
            _ => Ok(()),
        }
    })?;

    t.shapes = (first_shape..first_shape + shape_count).collect();
    let mut out = json!({"pasted": pasted, "width": width, "selection": t.to_json()});
    if skipped > 0
        && let Some(o) = out.as_object_mut()
    {
        o.insert("skipped".into(), json!(skipped));
    }
    Ok(out)
}

// ── Compare Layers ──────────────────────────────────────────────────────────────────────────────

fn compare_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.compareLayers", p)?;
    let a = LayerRef::read(&p, "layerA", "nameA")?;
    let b = LayerRef::read(&p, "layerB", "nameB")?;
    let tolerance = p.opt_f64("tolerance")?.unwrap_or(0.0);
    if !tolerance.is_finite() || !(0.0..=MAX_COORDINATE).contains(&tolerance) {
        return Err(p.bad(format!("`tolerance` must be between 0 and {MAX_COORDINATE}")));
    }
    let font = s.font()?;
    let names = glyph_names(&p, font)?;

    let mut different = Vec::new();
    let mut missing = Vec::new();
    let mut details = Vec::new();
    let mut same = 0usize;
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        let (Some(la), Some(lb)) = (a.find(font, g), b.find(font, g)) else {
            missing.push(name.clone());
            continue;
        };
        match layer_difference(la, lb, tolerance) {
            None => same += 1,
            Some(reason) => {
                if details.len() < MAX_DETAILS {
                    details.push(json!({"glyph": name, "reason": reason}));
                }
                different.push(name.clone());
            }
        }
    }
    if same == 0 && different.is_empty() && !names.is_empty() {
        return Err(p.bad(format!("no glyph has both layer `{}` and layer `{}`", a.text(), b.text())));
    }
    let mut out = json!({"different": different, "same": same, "missing": missing, "details": details});
    if let (Some(first), Some(o)) = (different.first(), out.as_object_mut()) {
        o.insert("first".into(), json!(first));
    }
    Ok(out)
}

/// The first way `b` differs from `a` beyond `tol`, in words; `None` when they match.
fn layer_difference(a: &Layer, b: &Layer, tol: f64) -> Option<String> {
    let far = |x: f64, y: f64| (x - y).abs() > tol;
    if far(a.width, b.width) {
        return Some(format!("advance width {} against {}", a.width, b.width));
    }
    let sa: Vec<&Shape> = a.shapes.iter().filter(|s| is_outline(s)).collect();
    let sb: Vec<&Shape> = b.shapes.iter().filter(|s| is_outline(s)).collect();
    if sa.len() != sb.len() {
        return Some(format!("{} shapes against {}", sa.len(), sb.len()));
    }
    for (i, (x, y)) in sa.iter().zip(&sb).enumerate() {
        match (x, y) {
            (Shape::Path(pa), Shape::Path(pb)) => {
                if pa.closed != pb.closed {
                    return Some(format!("path {i} is open in one layer and closed in the other"));
                }
                if pa.nodes.len() != pb.nodes.len() {
                    return Some(format!("path {i} has {} nodes against {}", pa.nodes.len(), pb.nodes.len()));
                }
                for (j, (na, nb)) in pa.nodes.iter().zip(&pb.nodes).enumerate() {
                    if na.kind != nb.kind {
                        return Some(format!("node {j} of path {i} is of another kind"));
                    }
                    if far(na.x, nb.x) || far(na.y, nb.y) {
                        return Some(format!("node {j} of path {i} is at ({}, {}) against ({}, {})", na.x, na.y, nb.x, nb.y));
                    }
                }
            }
            (Shape::Component(ca), Shape::Component(cb)) => {
                if ca.glyph != cb.glyph {
                    return Some(format!("component {i} is `{}` against `{}`", ca.glyph, cb.glyph));
                }
                let (ma, mb) = (ca.transform.affine().as_coeffs(), cb.transform.affine().as_coeffs());
                let linear_differs = ma.iter().zip(&mb).take(4).any(|(u, v)| (u - v).abs() > LINEAR_EPSILON);
                let offset_differs = ma.iter().zip(&mb).skip(4).any(|(u, v)| far(*u, *v));
                if linear_differs || offset_differs {
                    return Some(format!("component {i} (`{}`) is placed differently", ca.glyph));
                }
            }
            _ => return Some(format!("shape {i} is a path in one layer and a component in the other")),
        }
    }
    None
}
