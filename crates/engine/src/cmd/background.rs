//! Path menu, background part: copying the selection to the other side, swapping, clearing,
//! assigning from another font and interpolating with the background
//! (`docs/spec/03b-edit-view-display.md` § Background, `docs/spec/11-filters.md`).
//!
//! The commands that act on whole layers take either one `glyph` or a `glyphs` list (Font View).
//! With `glyphs`, a locked glyph or one without the named layer is skipped and listed in
//! `skipped`; with one `glyph` it is an error.

use std::collections::HashSet;

use fontcraft_geom::{grid_step, round_to_grid};
use fontcraft_model::{Anchor, Background, Font, Layer, LayerSide, Shape, Transform};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind, Targets};

/// Most layer ids one `layers` list may hold.
const MAX_LAYER_IDS: usize = 1024;
/// Largest interpolation amount, in percent either way. Beyond it every outline leaves the
/// coordinate range anyway.
const MAX_AMOUNT: f64 = 100_000.0;

/// The keys that address things inside one layer; they go with `glyph`, not with `glyphs`.
const LIST_KEYS: [&str; 7] = ["points", "segments", "segment", "shapes", "anchors", "guides", "annotations"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.selectionToBackground",
            "Selection to Background",
            ["Path"],
            Some("Cmd+J"),
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\" (the side being edited), points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name]} or {glyphs: [names], layer?: id | layers?: [ids]} replace the shapes and anchors of the other side with a copy of the targeted ones: whole paths for the paths a point or segment is in, the shapes listed, the anchors listed; without target lists, every shape and anchor of the side. While the background is edited it writes the foreground. With `glyphs` each layer's foreground goes to its background. A locked glyph refuses it → {glyphs: [names], skipped: [{glyph, layer?, reason}], to: \"foreground\" | \"background\", shapes: count, anchors: count}",
            has_doc,
            selection_to_background
        )
        .targets(&[TargetKind::Layer, TargetKind::Points, TargetKind::Segments, TargetKind::Shapes, TargetKind::Anchors]),
        cmd!(
            "path.addSelectionToBackground",
            "Add Selection to Background",
            ["Path"],
            Some("Cmd+Alt+J"),
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name]} or {glyphs: [names], layer?: id | layers?: [ids]} as Selection to Background, but keeps what the other side holds: the copied shapes are added after it, and a copied anchor replaces one of the same name → {glyphs: [names], skipped: [{glyph, layer?, reason}], to: \"foreground\" | \"background\", shapes: count, anchors: count}",
            has_doc,
            add_selection_to_background
        )
        .targets(&[TargetKind::Layer, TargetKind::Points, TargetKind::Segments, TargetKind::Shapes, TargetKind::Anchors]),
        cmd!(
            "path.swapWithBackground",
            "Swap with Background",
            ["Path"],
            Some("Cmd+Ctrl+J"),
            "{glyph: name | glyphs: [names], layer?: id (default: the first master's layer) | layers?: [ids]} exchange the foreground and the background of each layer: shapes, anchors, guides, hints and annotations (the placed image stays). A background left empty is removed, so swapping twice restores the layer. With one glyph, a locked glyph refuses it → {glyphs: [names], skipped: [{glyph, layer?, reason}], selection?: {glyph, layer} (one glyph: the edited side was replaced)}",
            has_doc,
            swap_with_background
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "path.clearBackground",
            "Clear Background",
            ["Path"],
            None,
            "{glyph: name | glyphs: [names], layer?: id (default: the first master's layer) | layers?: [ids]} empty the background of each layer → {glyphs: [names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            clear_background
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "path.assignBackground",
            "Assign Background",
            ["Path"],
            None,
            "{file?: path (a font source; default: this font, which keeps a copy of the current outlines), glyphs?: [names] (default: every glyph), master?: id (default: every master)} replace the shapes and anchors of the master layers' backgrounds with the foreground of the glyph of the same name in the source. Masters are matched by name, then by position; a source with fewer masters gives its first. Glyphs the source lacks, and locked glyphs, are skipped and listed → {glyphs: [names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            assign_background
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "path.interpolateWithBackground",
            "Interpolate with Background",
            ["Path"],
            None,
            "{glyph: name | glyphs: [names], layer?: id (default: the first master's layer) | layers?: [ids], amount: percent (0 keeps the foreground, 100 gives the background, beyond either end extrapolates; within ±100000)} move the foreground's nodes, component placements and same-named anchors toward the background's, rounded to the grid. Foreground and background must match: the same shapes in the same order, paths with the same node count, node types and closedness, components of the same glyph. A layer that does not match is left as it is and listed → {glyphs: [names], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            interpolate_with_background
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

// ── Scope: one glyph or many ────────────────────────────────────────────────────────────────────

/// The glyphs and layers a whole-layer command acts on.
struct Scope {
    glyphs: Vec<String>,
    layers: Vec<String>,
    /// One `glyph` was given: a locked glyph or a missing layer is an error, not a skip.
    single: bool,
}

/// What a whole-layer command did.
#[derive(Default)]
struct Outcome {
    glyphs: Vec<String>,
    skipped: Vec<Value>,
}

impl Outcome {
    fn skip(&mut self, glyph: &str, layer: Option<&str>, reason: impl Into<String>) {
        let mut v = json!({"glyph": glyph, "reason": reason.into()});
        if let (Some(layer), Some(o)) = (layer, v.as_object_mut()) {
            o.insert("layer".into(), json!(layer));
        }
        self.skipped.push(v);
    }

    fn to_json(&self) -> Value {
        json!({"glyphs": self.glyphs, "skipped": self.skipped})
    }
}

/// Reads `glyph` or `glyphs`, and `layer` or `layers`.
fn scope(p: &Params, font: &Font) -> Result<Scope> {
    let (glyphs, single) = match (p.get("glyph"), p.get("glyphs")) {
        (Some(_), Some(_)) => return Err(p.bad("give `glyph` or `glyphs`, not both")),
        (Some(_), None) => {
            let name = p.str("glyph")?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("there is no glyph `{name}`")));
            }
            (vec![name.to_string()], true)
        }
        (None, _) => {
            let glyphs = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyph` or `glyphs` is required"))?;
            if glyphs.is_empty() {
                return Err(p.bad("`glyphs` is empty"));
            }
            (glyphs, false)
        }
    };
    Ok(Scope { glyphs, layers: layer_ids(p, font)?, single })
}

/// `layers` or `layer`; the first master's layer when neither is given.
fn layer_ids(p: &Params, font: &Font) -> Result<Vec<String>> {
    match (p.get("layer"), p.get("layers")) {
        (Some(_), Some(_)) => Err(p.bad("give `layer` or `layers`, not both")),
        (Some(_), None) => Ok(vec![p.str("layer")?.to_string()]),
        (None, Some(Value::Array(a))) if a.len() > MAX_LAYER_IDS => {
            Err(p.bad(format!("`layers` holds {} ids; at most {MAX_LAYER_IDS} are allowed", a.len())))
        }
        (None, Some(_)) => {
            let mut seen = HashSet::new();
            let ids: Vec<String> = p.strings("layers")?.into_iter().filter(|id| seen.insert(*id)).map(str::to_string).collect();
            if ids.is_empty() {
                return Err(p.bad("`layers` is empty"));
            }
            Ok(ids)
        }
        (None, None) => Ok(vec![font.default_master()?.id.clone()]),
    }
}

/// Runs `f` on every addressed layer of every glyph in `scope`. `f` returns `Some(reason)` when
/// it leaves a layer as it is. Call inside [`Session::edit`].
fn each_layer(
    font: &mut Font,
    scope: &Scope,
    command: &'static str,
    mut f: impl FnMut(&str, &mut Layer) -> Result<Option<String>>,
) -> Result<Outcome> {
    let bad = |message: String| EngineError::BadParams { command: command.into(), message };
    let mut out = Outcome::default();
    for name in &scope.glyphs {
        let Some(glyph) = font.glyph(name) else {
            return Err(bad(format!("there is no glyph `{name}`")));
        };
        if glyph.info.locked {
            if scope.single {
                return Err(EngineError::Locked(name.clone()));
            }
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        let missing: Vec<&String> = scope.layers.iter().filter(|id| !glyph.layers.iter().any(|l| l.id == **id)).collect();
        if let Some(id) = missing.first()
            && scope.single
        {
            return Err(bad(format!("glyph `{name}` has no layer `{id}`")));
        }
        for id in missing {
            out.skip(name, Some(id.as_str()), "the glyph has no such layer");
        }
        let mut g = font.glyph_mut(name)?;
        let mut changed = false;
        for layer in g.layers.iter_mut().filter(|l| scope.layers.contains(&l.id)) {
            match f(name, layer)? {
                Some(reason) => out.skip(name, Some(layer.id.as_str()), reason),
                None => changed = true,
            }
        }
        if changed {
            out.glyphs.push(name.clone());
        }
    }
    Ok(out)
}

/// A layer that had no background keeps none when it is left empty.
fn tidy_background(layer: &mut Layer, had: bool) {
    if !had && layer.info.background.as_ref().is_some_and(|b| b.is_empty()) {
        layer.info.background = None;
    }
}

fn other(side: LayerSide) -> LayerSide {
    match side {
        LayerSide::Foreground => LayerSide::Background,
        LayerSide::Background => LayerSide::Foreground,
    }
}

// ── Selection to Background ─────────────────────────────────────────────────────────────────────

/// What a selection copies: shape indices and anchor names. `None` copies everything.
struct Pick {
    shapes: HashSet<usize>,
    anchors: HashSet<String>,
}

/// Copies the picked shapes and anchors of `from` to the other side, replacing what is there
/// unless `add`. Returns how many shapes and anchors were copied.
fn copy_side(layer: &mut Layer, from: LayerSide, pick: Option<&Pick>, add: bool) -> (usize, usize) {
    let had = layer.info.background.is_some();
    let src = layer.parts(from);
    let shapes: Vec<Shape> =
        src.shapes.iter().enumerate().filter(|(i, _)| pick.is_none_or(|p| p.shapes.contains(i))).map(|(_, s)| s.clone()).collect();
    let anchors: Vec<Anchor> = src.anchors.iter().filter(|a| pick.is_none_or(|p| p.anchors.contains(&a.name))).cloned().collect();
    let counts = (shapes.len(), anchors.len());
    let dst = layer.parts_mut(other(from));
    if !add {
        dst.shapes.clear();
        dst.anchors.clear();
    }
    dst.shapes.extend(shapes);
    for a in anchors {
        match dst.anchors.iter_mut().find(|e| e.name == a.name) {
            Some(existing) => *existing = a,
            None => dst.anchors.push(a),
        }
    }
    tidy_background(layer, had);
    counts
}

fn selection_to_background(s: &mut Session, p: &Value) -> Result<Value> {
    to_background(s, p, "path.selectionToBackground", "Selection to Background", false)
}

fn add_selection_to_background(s: &mut Session, p: &Value) -> Result<Value> {
    to_background(s, p, "path.addSelectionToBackground", "Add Selection to Background", true)
}

fn to_background(s: &mut Session, value: &Value, id: &'static str, label: &str, add: bool) -> Result<Value> {
    let p = Params::new(id, value)?;
    if p.get("glyphs").is_some() {
        if let Some(key) = LIST_KEYS.iter().chain(&["side"]).find(|k| p.get(k).is_some()) {
            return Err(p.bad(format!("`{key}` goes with one `glyph`; with `glyphs` the whole foreground is copied")));
        }
        let scope = scope(&p, s.font()?)?;
        let (mut shapes, mut anchors) = (0usize, 0usize);
        let out = s.edit(label, |font| {
            each_layer(font, &scope, id, |_, layer| {
                let (n, m) = copy_side(layer, LayerSide::Foreground, None, add);
                shapes = shapes.saturating_add(n);
                anchors = anchors.saturating_add(m);
                Ok(None)
            })
        })?;
        let mut v = out.to_json();
        if let Some(o) = v.as_object_mut() {
            o.insert("to".into(), json!("background"));
            o.insert("shapes".into(), json!(shapes));
            o.insert("anchors".into(), json!(anchors));
        }
        return Ok(v);
    }
    let t = Targets::parse(&p)?;
    if !(t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("copies shapes and anchors: `guides` and `annotations` are not taken"));
    }
    let pick = t.has_lists().then(|| Pick {
        shapes: t.points.iter().chain(&t.segments).map(|(shape, _)| *shape).chain(t.shapes.iter().copied()).collect(),
        anchors: t.anchors.iter().cloned().collect(),
    });
    let (shapes, anchors) = s.edit(label, |font| t.edit_layer(font, |layer| Ok(copy_side(layer, t.side, pick.as_ref(), add))))?;
    Ok(json!({
        "glyphs": [t.glyph],
        "skipped": [],
        "to": other(t.side).as_str(),
        "shapes": shapes,
        "anchors": anchors,
    }))
}

// ── Swap and Clear ──────────────────────────────────────────────────────────────────────────────

/// Exchanges the two sides. A background the swap leaves empty is dropped, so swapping a layer
/// that had none twice gives it back unchanged.
fn swap(layer: &mut Layer) {
    let i = &*layer.info;
    let front_empty = layer.shapes.is_empty() && layer.anchors.is_empty() && i.guides.is_empty() && i.hints.is_empty() && i.annotations.is_empty();
    if front_empty && i.background.as_ref().is_none_or(Background::is_empty) {
        return;
    }
    let info = &mut *layer.info;
    let b = info.background.get_or_insert_with(Default::default);
    std::mem::swap(&mut layer.shapes, &mut b.shapes);
    std::mem::swap(&mut layer.anchors, &mut b.anchors);
    std::mem::swap(&mut info.guides, &mut b.guides);
    std::mem::swap(&mut info.hints, &mut b.hints);
    std::mem::swap(&mut info.annotations, &mut b.annotations);
    if b.is_empty() {
        info.background = None;
    }
}

fn swap_with_background(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("path.swapWithBackground", p)?;
    no_lists(&p)?;
    let scope = scope(&p, s.font()?)?;
    let out = s.edit("Swap with Background", |font| {
        each_layer(font, &scope, "path.swapWithBackground", |_, layer| {
            swap(layer);
            Ok(None)
        })
    })?;
    let mut v = out.to_json();
    if let (true, [glyph], [layer], Some(o)) = (scope.single, scope.glyphs.as_slice(), scope.layers.as_slice(), v.as_object_mut()) {
        o.insert("selection".into(), Targets::new("path.swapWithBackground", glyph.clone(), Some(layer.clone())).to_json());
    }
    Ok(v)
}

fn clear_background(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("path.clearBackground", p)?;
    no_lists(&p)?;
    let scope = scope(&p, s.font()?)?;
    let out = s.edit("Clear Background", |font| {
        each_layer(font, &scope, "path.clearBackground", |_, layer| {
            layer.info.background = None;
            Ok(None)
        })
    })?;
    Ok(out.to_json())
}

/// Whole-layer commands take no target lists.
fn no_lists(p: &Params) -> Result<()> {
    match LIST_KEYS.iter().chain(&["side"]).find(|k| p.get(k).is_some()) {
        Some(key) => Err(p.bad(format!("acts on whole layers: `{key}` is not taken"))),
        None => Ok(()),
    }
}

// ── Assign Background ───────────────────────────────────────────────────────────────────────────

fn assign_background(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("path.assignBackground", p)?;
    let font = s.font()?;
    let glyphs = match p.opt_glyphs("glyphs", font)? {
        Some(g) if g.is_empty() => return Err(p.bad("`glyphs` is empty")),
        Some(g) => g,
        None => font.glyphs.iter().map(|g| g.name.clone()).collect(),
    };
    let masters: Vec<(usize, &fontcraft_model::Master)> = match p.opt_str("master")? {
        Some(id) => {
            let i = font.masters.iter().position(|m| m.id == id).ok_or_else(|| p.bad(format!("there is no master `{id}`")))?;
            font.masters.get(i).map(|m| (i, m)).into_iter().collect()
        }
        None => font.masters.iter().enumerate().collect(),
    };
    let source = match p.opt_str("file")? {
        Some(path) => open_file(path)?,
        None => Font::clone(font),
    };
    if source.masters.is_empty() {
        return Err(p.bad("the source has no masters"));
    }
    // This font's master id → the source master id it takes its outlines from.
    let pairs: Vec<(String, String)> = masters
        .iter()
        .filter_map(|(i, m)| {
            let from = source.masters.iter().find(|sm| sm.name == m.name).or_else(|| source.masters.get(*i)).or_else(|| source.masters.first())?;
            Some((m.id.clone(), from.id.clone()))
        })
        .collect();
    let scope = Scope { glyphs, layers: pairs.iter().map(|(id, _)| id.clone()).collect(), single: false };
    let out = s.edit("Assign Background", |font| {
        let mut out = Outcome::default();
        for name in &scope.glyphs {
            let Some(glyph) = font.glyph(name) else { continue };
            if glyph.info.locked {
                out.skip(name, None, "the glyph is locked");
                continue;
            }
            let Some(src) = source.glyph(name) else {
                out.skip(name, None, "the source has no glyph of this name");
                continue;
            };
            let mut g = font.glyph_mut(name)?;
            let mut changed = false;
            for layer in g.layers.iter_mut().filter(|l| l.is_master_layer()) {
                let Some((_, from)) = pairs.iter().find(|(id, _)| *id == layer.id) else { continue };
                let Some(src_layer) = src.layer(from) else {
                    out.skip(name, Some(layer.id.as_str()), format!("the source glyph has no layer for master `{from}`"));
                    continue;
                };
                let b = layer.parts_mut(LayerSide::Background);
                *b.shapes = src_layer.shapes.clone();
                *b.anchors = src_layer.anchors.clone();
                changed = true;
            }
            if changed {
                out.glyphs.push(name.clone());
            }
        }
        Ok(out)
    })?;
    Ok(out.to_json())
}

#[cfg(not(target_arch = "wasm32"))]
fn open_file(path: &str) -> Result<Font> {
    crate::io::open(std::path::Path::new(path), None).map(|(font, _)| font)
}

#[cfg(target_arch = "wasm32")]
fn open_file(_: &str) -> Result<Font> {
    Err(EngineError::Unsupported("reading a file by path in the browser".into()))
}

// ── Interpolate with Background ─────────────────────────────────────────────────────────────────

fn interpolate_with_background(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("path.interpolateWithBackground", p)?;
    no_lists(&p)?;
    let amount = p.f64("amount")?;
    if amount.abs() > MAX_AMOUNT {
        return Err(p.bad(format!("`amount` must be within ±{MAX_AMOUNT} percent, got {amount}")));
    }
    let t = amount / 100.0;
    let font = s.font()?;
    let step = grid_step(font.info.settings.grid_length.unwrap_or(1.0), font.info.settings.grid_sub_division.unwrap_or(1.0));
    let scope = scope(&p, font)?;
    let out = s.edit("Interpolate with Background", |font| {
        each_layer(font, &scope, "path.interpolateWithBackground", |_, layer| {
            let bg = layer.parts(LayerSide::Background);
            let shapes = match blend_shapes(&layer.shapes, bg.shapes, t, step) {
                Ok(shapes) => shapes,
                Err(reason) => return Ok(Some(reason)),
            };
            let anchors: Vec<Anchor> = layer
                .anchors
                .iter()
                .map(|a| match bg.anchor(&a.name) {
                    Some((_, b)) => Anchor { x: lerp(a.x, b.x, t, step), y: lerp(a.y, b.y, t, step), ..a.clone() },
                    None => a.clone(),
                })
                .collect();
            layer.shapes = shapes;
            layer.anchors = anchors;
            Ok(None)
        })
    })?;
    Ok(out.to_json())
}

fn lerp(a: f64, b: f64, t: f64, step: f64) -> f64 {
    round_to_grid(a + (b - a) * t, step)
}

fn shape_kind(s: &Shape) -> &'static str {
    match s {
        Shape::Path(_) => "path",
        Shape::Component(_) => "component",
        Shape::Image(_) => "image",
        Shape::Raw(_) => "other shape",
    }
}

/// The foreground shapes moved `t` of the way to the background's, or why they do not match.
fn blend_shapes(fg: &[Shape], bg: &[Shape], t: f64, step: f64) -> std::result::Result<Vec<Shape>, String> {
    if fg.len() != bg.len() {
        return Err(format!("the foreground has {} shapes and the background {}", fg.len(), bg.len()));
    }
    let mut out = Vec::with_capacity(fg.len());
    for (i, (a, b)) in fg.iter().zip(bg).enumerate() {
        let shape = match (a, b) {
            (Shape::Path(pa), Shape::Path(pb)) => {
                if pa.closed != pb.closed {
                    return Err(format!("path {i} is closed on one side and open on the other"));
                }
                if pa.nodes.len() != pb.nodes.len() {
                    return Err(format!("path {i} has {} nodes in the foreground and {} in the background", pa.nodes.len(), pb.nodes.len()));
                }
                let mut path = pa.clone();
                for (j, (n, m)) in path.nodes.iter_mut().zip(&pb.nodes).enumerate() {
                    if n.kind != m.kind {
                        return Err(format!("node {j} of path {i} has a different type in the background"));
                    }
                    n.x = lerp(n.x, m.x, t, step);
                    n.y = lerp(n.y, m.y, t, step);
                }
                Shape::Path(path)
            }
            (Shape::Component(ca), Shape::Component(cb)) => {
                if ca.glyph != cb.glyph {
                    return Err(format!("component {i} is `{}` in the foreground and `{}` in the background", ca.glyph, cb.glyph));
                }
                let mut c = ca.clone();
                c.transform = blend_transform(&ca.transform, &cb.transform, t, step);
                Shape::Component(c)
            }
            (Shape::Image(_), Shape::Image(_)) | (Shape::Raw(_), Shape::Raw(_)) => a.clone(),
            _ => return Err(format!("shape {i} is a {} in the foreground and a {} in the background", shape_kind(a), shape_kind(b))),
        };
        out.push(shape);
    }
    Ok(out)
}

/// Two placements blended: part by part when both are decomposed, else as matrices. Only the
/// offset is rounded to the grid.
fn blend_transform(a: &Transform, b: &Transform, t: f64, step: f64) -> Transform {
    let mix = |x: f64, y: f64| x + (y - x) * t;
    match (*a, *b) {
        (Transform::Decomposed { pos: p1, scale: s1, angle: a1, slant: k1 }, Transform::Decomposed { pos: p2, scale: s2, angle: a2, slant: k2 }) => {
            Transform::Decomposed {
                pos: (lerp(p1.0, p2.0, t, step), lerp(p1.1, p2.1, t, step)),
                scale: (mix(s1.0, s2.0), mix(s1.1, s2.1)),
                angle: mix(a1, a2),
                slant: (mix(k1.0, k2.0), mix(k1.1, k2.1)),
            }
        }
        _ => {
            let (m1, m2) = (a.affine().as_coeffs(), b.affine().as_coeffs());
            let mut m = [0.0; 6];
            for (k, (out, (x, y))) in m.iter_mut().zip(m1.iter().zip(m2.iter())).enumerate() {
                *out = if k >= 4 { lerp(*x, *y, t, step) } else { mix(*x, *y) };
            }
            Transform::Matrix(m)
        }
    }
}
