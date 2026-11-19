//! Decomposing components and corners, and Component from Selection (M6.6). Rows:
//! `glyph.decomposeGlyph`, `glyph.decomposeGlyphForAllMasters`, `component.decompose`,
//! `cornerComponent.decompose`, `glyph.componentFromSelection`
//! (`spec/inventory/09-reusing-shapes.toml`, `05-font-view.toml`).
//!
//! A component is replaced by what the editor draws for it: a smart component by its outline
//! interpolated at its values ([`smart::component_shapes`]), any other by its base glyph's layer
//! (chosen as anchor propagation chooses it, [`base_layer`]) with that layer's corner components
//! fitted in ([`corner::insert_corners`]), moved by the component's transform. A contour under a
//! mirroring transform is reversed, as export decomposition reverses it. Decomposed shapes take
//! the component's place in the layer's shape order, and the layer's hints are renumbered so they
//! keep pointing at the same nodes.
//!
//! Limits: one layer holds at most [`MAX_LAYER_SHAPES`] shapes and [`MAX_OUTLINE_ELEMENTS`] nodes,
//! and decomposing it visits at most [`MAX_LAYER_SHAPES`] components, nested ones included, so a
//! fan-out of components that draw nothing is refused too. One command holds at most
//! [`MAX_CALL_NODES`] nodes and [`MAX_CALL_SHAPES`] shapes over all its layers, and visits at most
//! [`MAX_CALL_VISITS`] components; past these the command is refused and nothing changes.

use std::collections::BTreeSet;

use fontcraft_components::propagate::{AnchorSource, base_layer, effective_anchors_with};
use fontcraft_geom::{Affine, Point};
use fontcraft_hinting::Renumbered;
use fontcraft_model::corner::{self, CornerTarget};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{
    Anchor, Component, Font, Glyph, Layer, LayerSide, MAX_COMPONENT_DEPTH, MAX_COORDINATE, MAX_GLYPHS, MAX_OUTLINE_ELEMENTS, Shape, Transform, smart,
};
use serde_json::{Value, json};

use super::common::{Outcome, Scope, in_range, scope};
use super::shared::{MAX_LAYER_SHAPES, hint_shapes, remap_hint, renumber_hints};
use super::*;
use crate::EngineError;
use crate::params::{MAX_TARGETS, Params, TargetKind, Targets, kind};

/// Most nodes one command may hold in the layers it decomposes, over all of them: the new layers
/// are kept until the edit lands, so this bounds their memory. Each layer has its own
/// [`MAX_OUTLINE_ELEMENTS`] too.
const MAX_CALL_NODES: usize = 10_000_000;
/// Most shapes one command may hold in the layers it decomposes, over all of them.
const MAX_CALL_SHAPES: usize = 1_000_000;
/// Most components one command may visit over all the layers it decomposes.
const MAX_CALL_VISITS: usize = 20_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.decomposeGlyph",
            "Decompose Glyph",
            ["Glyph"],
            Some("Cmd+Shift+D"),
            "{glyph: name} or {glyphs: [names]}, with {layer?: id} or {layers?: [ids]} (default: the first master's layer) replace every component of the layer by the paths of its base glyph, descending through nested components until none remain. A smart component becomes its outline at its values; corner components of the base glyphs are fitted in first. The anchors the components passed on that the layer lacks are added, so marks attach as before. Hints on the layer's paths keep pointing at the same nodes. A layer without components, a locked glyph or a missing layer is skipped and listed; with one `glyph` each is an error → {glyphs: [names], skipped: [{glyph, layer?, reason}], anchorsAdded: number, warnings: [string]}",
            has_doc,
            decompose_glyph
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyph.decomposeGlyphForAllMasters",
            "Decompose Components for All Masters",
            ["Glyph"],
            Some("Cmd+Alt+Shift+D"),
            "{glyph: name} or {glyphs: [names]} glyph.decomposeGlyph on the master layer of every master. Other layers keep their components. A master layer without components is skipped and listed; with one `glyph` it is an error when no master layer has components → {glyphs: [names], skipped: [{glyph, layer?, reason}], anchorsAdded: number, warnings: [string]}",
            has_doc,
            decompose_glyph_all_masters
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "component.decompose",
            "Decompose",
            [],
            None,
            "{glyph: name, layer?: id, components?: [indices]} (`shapes` is accepted in place of `components`) replace the chosen components (default: every component of the layer) by their contents, one level deep: components inside their base glyphs stay components, with the transforms combined. A smart component becomes its outline at its values; corner components of the base glyph are fitted in first. No anchors are added. An index that is not a component is an error → {glyph, layer, decomposed: number, warnings: [string], selection: {glyph, layer, shapes}}",
            has_doc,
            decompose_components
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes])
        .context(&["canvas.component"]),
        cmd!(
            "cornerComponent.decompose",
            "Decompose Corner",
            [],
            None,
            "{glyph: name, layer?: id, corners?: [indices]} fit the chosen corner components (positions in the layer's hint list; default: every corner component of the layer) into their paths and remove them; the other corners keep their nodes. A corner on an extra node, or one whose corner glyph does not exist, is an error → {glyph, layer, decomposed: number, selection: {glyph, layer}}",
            has_doc,
            decompose_corners
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "glyph.componentFromSelection",
            "Component from Selection",
            ["Glyph"],
            None,
            "{glyph: name, layer?: id (a master layer), shapes?: [indices] (default: every path and component), name?: string (default: `_part.` and the glyph's name, with .001, .002… while taken)} move the shapes into a new glyph with that name (one that starts with an underscore does not export), at their coordinates, and put a component of it at offset zero where the first of them was. Every master layer whose shapes at those indices are of the same kinds (path or component) does the same; the new glyph's layer for any other master stays empty and the master is listed in `skipped`. Hints move with the paths they point at. Anchors stay. A name that is already a glyph is an error → {name, glyph, layer, component: index, skipped: [{layer, reason}], warnings: [string], selection: {glyph, layer, shapes}}",
            has_doc,
            component_from_selection
        )
        .targets(&[TargetKind::Layer, TargetKind::Shapes])
        .context(&["canvas"]),
    ]
}

// ── Drawing components as shapes ────────────────────────────────────────────────────────────────

/// What one command may still spend over every layer it decomposes ([`MAX_CALL_NODES`],
/// [`MAX_CALL_SHAPES`], [`MAX_CALL_VISITS`]).
struct CallBudget {
    nodes_left: usize,
    shapes_left: usize,
    visits_left: usize,
    /// A limit of the whole command ran out: decomposing more layers would fail too.
    spent: bool,
}

impl CallBudget {
    fn new() -> Self {
        Self { nodes_left: MAX_CALL_NODES, shapes_left: MAX_CALL_SHAPES, visits_left: MAX_CALL_VISITS, spent: false }
    }
}

/// Takes `n` from `left`, one of the command's budgets; marks the command `spent` when it runs
/// out.
fn take(left: &mut usize, spent: &mut bool, n: usize, what: &str, limit: usize) -> std::result::Result<(), String> {
    match left.checked_sub(n) {
        Some(rest) => {
            *left = rest;
            Ok(())
        }
        None => {
            *spent = true;
            Err(format!("decomposing would take more than {limit} {what} over all the layers; decompose fewer glyphs at a time"))
        }
    }
}

/// Turns components into the shapes they draw, within the work limits of one layer and of the
/// command.
struct Expander<'f, 'b> {
    font: &'f Font,
    /// Descend through nested components (Decompose Glyph) instead of keeping them (Decompose).
    nested: bool,
    nodes_left: usize,
    shapes_left: usize,
    visits_left: usize,
    smart_draws: usize,
    call: &'b mut CallBudget,
    warnings: Vec<String>,
}

impl<'f, 'b> Expander<'f, 'b> {
    fn new(font: &'f Font, nested: bool, call: &'b mut CallBudget) -> Self {
        Self {
            font,
            nested,
            nodes_left: MAX_OUTLINE_ELEMENTS,
            shapes_left: MAX_LAYER_SHAPES,
            visits_left: MAX_LAYER_SHAPES,
            smart_draws: smart::MAX_SMART_DRAWS_PER_LAYER,
            call,
            warnings: Vec::new(),
        }
    }

    /// Takes one shape with `nodes` nodes from the layer's budget and the command's.
    fn charge(&mut self, nodes: usize) -> std::result::Result<(), String> {
        self.shapes_left = self.shapes_left.checked_sub(1).ok_or_else(|| format!("the layer would hold more than {MAX_LAYER_SHAPES} shapes"))?;
        self.nodes_left = self.nodes_left.checked_sub(nodes).ok_or_else(|| format!("the layer would hold more than {MAX_OUTLINE_ELEMENTS} nodes"))?;
        let call = &mut *self.call;
        take(&mut call.shapes_left, &mut call.spent, 1, "shapes", MAX_CALL_SHAPES)?;
        take(&mut call.nodes_left, &mut call.spent, nodes, "nodes", MAX_CALL_NODES)
    }

    /// Counts one component, or one shape that draws nothing, looked at: a component whose base
    /// draws nothing still costs the time to reach it, and nesting multiplies that.
    fn visit(&mut self) -> std::result::Result<(), String> {
        self.visits_left =
            self.visits_left.checked_sub(1).ok_or_else(|| format!("decomposing the layer visits more than {MAX_LAYER_SHAPES} components"))?;
        let call = &mut *self.call;
        take(&mut call.visits_left, &mut call.spent, 1, "component visits", MAX_CALL_VISITS)
    }

    /// Appends to `out` the shapes the component `c` of `layer` draws, in `layer`'s coordinates.
    /// `depth` is 1 for a component of the layer being decomposed.
    fn component(&mut self, c: &Component, layer: &Layer, depth: usize, out: &mut Vec<Shape>) -> std::result::Result<(), String> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(format!("components nest more than {MAX_COMPONENT_DEPTH} deep below `{}`, or a glyph uses itself", c.glyph));
        }
        self.visit()?;
        if c.transform.numbers().iter().any(|v| !v.is_finite()) {
            return Err(format!("the transform of the component `{}` is not a number", c.glyph));
        }
        let font = self.font;
        let base = font.glyph(&c.glyph).ok_or_else(|| format!("the component's glyph `{}` does not exist", c.glyph))?;
        match smart::component_shapes(font, c, &layer.master, &mut self.smart_draws) {
            // The drawn shapes carry the component's transform already.
            Ok(Some(drawn)) => {
                for shape in &drawn {
                    self.place(shape, Affine::IDENTITY, layer, depth, out)?;
                }
                return Ok(());
            }
            Ok(None) => {}
            // The editor draws such a component from its base layer; decomposing follows it.
            Err(reason) => self.warnings.push(format!("the smart component `{}` was decomposed from its base layer: {reason}", c.glyph)),
        }
        let from = base_layer(base, layer, c).ok_or_else(|| format!("glyph `{}` has no layer for master `{}`", c.glyph, layer.master))?;
        let cornered = match corner::insert_corners(font, from) {
            Ok(shapes) => shapes,
            Err(e) => {
                self.warnings.push(format!("glyph `{}` was decomposed without its corner components: {e}", c.glyph));
                None
            }
        };
        let affine = c.affine();
        for shape in cornered.as_deref().unwrap_or(&from.shapes) {
            self.place(shape, affine, from, depth, out)?;
        }
        Ok(())
    }

    /// Appends `shape` of `context` to `out`, moved by `affine`.
    fn place(&mut self, shape: &Shape, affine: Affine, context: &Layer, depth: usize, out: &mut Vec<Shape>) -> std::result::Result<(), String> {
        match shape {
            Shape::Path(path) => {
                self.charge(path.nodes.len())?;
                let mut path = path.clone();
                for n in &mut path.nodes {
                    let at = affine * Point::new(n.x, n.y);
                    (n.x, n.y) = (at.x, at.y);
                }
                if !path.nodes.iter().all(|n| in_range(n.x) && in_range(n.y)) {
                    return Err(format!("decomposing puts points beyond ±{MAX_COORDINATE}"));
                }
                if affine.determinant() < 0.0 {
                    path.reverse();
                }
                out.push(Shape::Path(path));
            }
            Shape::Component(inner) => {
                let mut inner = inner.clone();
                if affine != Affine::IDENTITY {
                    inner.transform = Transform::Matrix((affine * inner.affine()).as_coeffs());
                }
                if self.nested {
                    self.component(&inner, context, depth + 1, out)?;
                } else {
                    self.charge(0)?;
                    out.push(Shape::Component(inner));
                }
            }
            // Outlines leave images and unreadable shapes out; so does decomposition.
            Shape::Image(_) | Shape::Raw(_) => self.visit()?,
        }
        Ok(())
    }
}

/// A layer's shapes after decomposing some of its components.
struct Decomposed {
    shapes: Vec<Shape>,
    /// The new index of every old shape; `None` for a decomposed component.
    map: Vec<Option<usize>>,
    /// The indices of the shapes decomposition added.
    added: Vec<usize>,
    components: usize,
    warnings: Vec<String>,
}

/// Decomposes the components of `layer` at `which` (every component when `None`).
fn decompose_shapes(
    font: &Font,
    layer: &Layer,
    which: Option<&BTreeSet<usize>>,
    nested: bool,
    call: &mut CallBudget,
) -> std::result::Result<Decomposed, String> {
    let mut ex = Expander::new(font, nested, call);
    let mut shapes = Vec::with_capacity(layer.shapes.len());
    let mut map = Vec::with_capacity(layer.shapes.len());
    let mut added = Vec::new();
    let mut components = 0usize;
    for (i, shape) in layer.shapes.iter().enumerate() {
        match shape {
            Shape::Component(c) if which.is_none_or(|w| w.contains(&i)) => {
                let start = shapes.len();
                ex.component(c, layer, 1, &mut shapes)?;
                added.extend(start..shapes.len());
                map.push(None);
                components += 1;
            }
            other => {
                ex.charge(if let Shape::Path(p) = other { p.nodes.len() } else { 0 })?;
                map.push(Some(shapes.len()));
                shapes.push(other.clone());
            }
        }
    }
    Ok(Decomposed { shapes, map, added, components, warnings: ex.warnings })
}

/// `shapes` (one side of `layer`) with every component of a glyph `doomed` accepts decomposed,
/// including those that decomposing brings in from nested glyphs; other components are kept,
/// with the transforms combined (glyph.removeGlyph with `components: "decompose"`).
pub(crate) fn decompose_where(
    font: &Font,
    layer: &Layer,
    shapes: &[Shape],
    doomed: impl Fn(&str) -> bool,
) -> std::result::Result<Vec<Shape>, String> {
    let mut side = layer.clone();
    side.shapes = shapes.to_vec();
    let mut call = CallBudget::new();
    // Each pass descends one level of nesting.
    for _ in 0..=MAX_COMPONENT_DEPTH {
        let which: BTreeSet<usize> =
            side.shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Component(c) if doomed(&c.glyph))).map(|(i, _)| i).collect();
        if which.is_empty() {
            return Ok(side.shapes);
        }
        side.shapes = decompose_shapes(font, &side, Some(&which), false, &mut call)?.shapes;
    }
    Err(format!("components nest more than {MAX_COMPONENT_DEPTH} deep, or a glyph uses itself"))
}

/// `layer` (of glyph `glyph`) with the decomposed shapes and its hints renumbered; the layer is
/// added to `done`, so the session's hint upkeep leaves it alone.
fn with_shapes(layer: &Layer, d: Decomposed, glyph: &str, done: &mut Renumbered) -> Layer {
    let mut out = layer.clone();
    out.shapes = d.shapes;
    if !layer.info.hints.is_empty() {
        // A reference past the old shapes pointed at nothing before and is left as it is.
        let map = |i: usize| d.map.get(i).map_or(Some(i), |m| *m);
        renumber_hints(layer, &mut out, LayerSide::Foreground, glyph, &map, &mut done.report);
        done.insert(glyph, &layer.id, LayerSide::Foreground);
    }
    out
}

/// What [`full_decomposition`] made of one layer.
struct FullDecomposition {
    layer: Layer,
    anchors_added: usize,
    warnings: Vec<String>,
}

/// `layer` of `glyph` with every component decomposed down to paths, plus the anchors its
/// components passed on that it lacks.
fn full_decomposition(
    font: &Font,
    data: &GlyphData,
    glyph: &str,
    layer: &Layer,
    call: &mut CallBudget,
    done: &mut Renumbered,
) -> std::result::Result<FullDecomposition, String> {
    let mut warnings = Vec::new();
    let passed: Vec<Anchor> = match effective_anchors_with(font, Some(data), glyph, &layer.id) {
        Ok(list) => list
            .into_iter()
            .filter(|a| matches!(a.source, AnchorSource::Component { .. }) && !layer.anchors.iter().any(|own| own.name == a.name))
            .map(|a| Anchor::new(a.name, a.x, a.y))
            .collect(),
        Err(e) => {
            warnings.push(format!("glyph `{glyph}`: no anchors were added: {e}"));
            Vec::new()
        }
    };
    let d = decompose_shapes(font, layer, None, true, call)?;
    warnings.extend(d.warnings.iter().map(|w| format!("glyph `{glyph}`: {w}")));
    let mut out = with_shapes(layer, d, glyph, done);
    let anchors_added = passed.len();
    out.anchors.extend(passed);
    Ok(FullDecomposition { layer: out, anchors_added, warnings })
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// A list of indices under `key`, repeats dropped; `None` when absent.
fn indices(p: &Params, key: &str) -> Result<Option<BTreeSet<usize>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_TARGETS {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {MAX_TARGETS} are allowed", list.len())));
    }
    let mut out = BTreeSet::new();
    for v in list {
        let i = v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| p.bad(format!("`{key}` must hold indices (whole numbers from 0), found {} ({v})", kind(v))))?;
        out.insert(i);
    }
    Ok(Some(out))
}

/// The one layer a single-layer command names.
fn layer_target(p: &Params, command: &'static str) -> Result<Targets> {
    if p.get("side").is_some_and(|v| v.as_str() != Some(LayerSide::Foreground.as_str())) {
        return Err(p.bad("works on the foreground of a layer"));
    }
    Ok(Targets::new(command, p.str("glyph")?, p.opt_str("layer")?.map(str::to_string)))
}

// ── Decompose Glyph ─────────────────────────────────────────────────────────────────────────────

fn decompose_glyph(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("glyph.decomposeGlyph", value)?;
    let sc = scope(&p, s.font()?)?;
    decompose_in_scope(s, &p, &sc, "Decompose Glyph", true)
}

fn decompose_glyph_all_masters(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("glyph.decomposeGlyphForAllMasters", value)?;
    if let Some(k) = ["layer", "layers", "side"].iter().find(|k| p.get(k).is_some()) {
        return Err(p.bad(format!("`{k}` is not taken: every master layer is decomposed")));
    }
    let font = s.font()?;
    let mut sc = scope(&p, font)?;
    sc.layers = font.masters.iter().map(|m| m.id.clone()).collect();
    decompose_in_scope(s, &p, &sc, "Decompose Components for All Masters", false)
}

/// Fully decomposes every layer of `sc`. With `strict`, a missing layer of a single glyph is an
/// error; without it (all masters) it is skipped.
fn decompose_in_scope(s: &mut Session, p: &Params, sc: &Scope, label: &str, strict: bool) -> Result<Value> {
    let data = s.glyph_data()?;
    let (out, anchors_added, warnings, done) = s.edit(label, |font| {
        let mut out = Outcome::default();
        let mut done = Renumbered::default();
        let mut anchors_added = 0usize;
        let mut warnings = Vec::new();
        let mut changes: Vec<(String, Layer)> = Vec::new();
        // One budget for every layer: the new layers are all held until the edit lands.
        let mut call = CallBudget::new();
        for name in &sc.glyphs {
            let glyph = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
            if glyph.info.locked {
                if sc.single {
                    return Err(EngineError::Locked(name.clone()));
                }
                out.skip(name, None, "the glyph is locked");
                continue;
            }
            let first_change = changes.len();
            for id in &sc.layers {
                let Some(layer) = glyph.layers.iter().find(|l| l.id == *id) else {
                    if sc.single && strict {
                        return Err(p.bad(format!("glyph `{name}` has no layer `{id}`")));
                    }
                    out.skip(name, Some(id.as_str()), "the glyph has no such layer");
                    continue;
                };
                if !layer.shapes.iter().any(|sh| matches!(sh, Shape::Component(_))) {
                    out.skip(name, Some(id.as_str()), "the layer has no components");
                    continue;
                }
                match full_decomposition(font, &data, name, layer, &mut call, &mut done) {
                    Ok(done) => {
                        anchors_added += done.anchors_added;
                        warnings.extend(done.warnings);
                        changes.push((name.clone(), done.layer));
                    }
                    Err(reason) if sc.single || call.spent => return Err(p.bad(format!("glyph `{name}` layer `{id}`: {reason}"))),
                    Err(reason) => out.skip(name, Some(id.as_str()), reason),
                }
            }
            if changes.len() > first_change {
                out.glyphs.push(name.clone());
            }
        }
        if sc.single && out.glyphs.is_empty() {
            let name = sc.glyphs.first().map_or("", String::as_str);
            return Err(p.bad(format!("glyph `{name}` has no components on the layers given")));
        }
        for (name, layer) in changes {
            let mut g = font.glyph_mut(&name)?;
            if let Some(slot) = g.layers.iter_mut().find(|l| l.id == layer.id) {
                *slot = layer;
            }
        }
        Ok((out, anchors_added, warnings, done))
    })?;
    s.hints_renumbered(done);
    Ok(json!({"glyphs": out.glyphs, "skipped": out.skipped, "anchorsAdded": anchors_added, "warnings": warnings}))
}

// ── Decompose (one level) ───────────────────────────────────────────────────────────────────────

fn decompose_components(s: &mut Session, value: &Value) -> Result<Value> {
    const ID: &str = "component.decompose";
    let p = Params::new(ID, value)?;
    let mut t = layer_target(&p, ID)?;
    let chosen = match (indices(&p, "components")?, indices(&p, "shapes")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `components` or `shapes`, not both")),
        (a, b) => a.or(b),
    };
    let (added, count, warnings, done) = s.edit("Decompose", |font| {
        let mut done = Renumbered::default();
        let new = {
            let r = t.resolve(font)?;
            crate::check_unlocked(r.glyph)?;
            let layer = r.layer;
            let which: BTreeSet<usize> = match &chosen {
                Some(list) => {
                    for &i in list {
                        match layer.shapes.get(i) {
                            Some(Shape::Component(_)) => {}
                            Some(_) => return Err(p.bad(format!("shape {i} is not a component"))),
                            None => return Err(p.bad(format!("shape {i} does not resolve: the layer has {} shapes", layer.shapes.len()))),
                        }
                    }
                    list.clone()
                }
                None => layer.shapes.iter().enumerate().filter(|(_, sh)| matches!(sh, Shape::Component(_))).map(|(i, _)| i).collect(),
            };
            if which.is_empty() {
                return Err(p.bad("the layer has no components"));
            }
            let d = decompose_shapes(font, layer, Some(&which), false, &mut CallBudget::new()).map_err(|m| p.bad(m))?;
            let summary = (d.added.clone(), d.components, d.warnings.clone());
            (with_shapes(layer, d, &t.glyph, &mut done), summary)
        };
        let (layer, (added, count, warnings)) = new;
        t.edit_layer(font, |l| {
            *l = layer;
            Ok(())
        })?;
        Ok((added, count, warnings, done))
    })?;
    s.hints_renumbered(done);
    t.layer = Some(t.layer_id(s.font()?)?);
    let layer = t.layer.clone();
    t.shapes = added;
    Ok(json!({"glyph": t.glyph, "layer": layer, "decomposed": count, "warnings": warnings, "selection": t.to_json()}))
}

// ── Decompose Corner ────────────────────────────────────────────────────────────────────────────

fn decompose_corners(s: &mut Session, value: &Value) -> Result<Value> {
    const ID: &str = "cornerComponent.decompose";
    let p = Params::new(ID, value)?;
    let mut t = layer_target(&p, ID)?;
    let chosen = indices(&p, "corners")?;
    let count = s.edit("Decompose Corner", |font| {
        let (layer, count) = {
            let r = t.resolve(font)?;
            crate::check_unlocked(r.glyph)?;
            let hints = &r.layer.info.hints;
            let which: Vec<usize> = match &chosen {
                Some(list) => list.iter().copied().collect(),
                None => hints.iter().enumerate().filter(|(_, h)| corner::is_corner(h)).map(|(i, _)| i).collect(),
            };
            if which.is_empty() {
                return Err(p.bad("the layer has no corner components"));
            }
            for &i in &which {
                let hint = hints.get(i).ok_or_else(|| p.bad(format!("`corners`: the layer has no hint {i}")))?;
                let c = corner::corner_hint(hint)
                    .map_err(|m| p.bad(format!("`corners`: hint {i}: {m}")))?
                    .ok_or_else(|| p.bad(format!("`corners`: hint {i} is not a corner component")))?;
                match c.target {
                    CornerTarget::Intersection { .. } => {
                        return Err(p.bad(format!("corner {i} sits on an extra node: remove the overlap first, then decompose it")));
                    }
                    CornerTarget::Node { shape, .. } => {
                        if !matches!(r.layer.shapes.get(shape), Some(Shape::Path(_))) {
                            return Err(p.bad(format!("corner {i} points at shape {shape}, which is not a path")));
                        }
                    }
                }
                if font.glyph(&c.name).is_none() {
                    return Err(p.bad(format!("corner {i}: there is no corner glyph `{}`", c.name)));
                }
            }
            let layer = corner::decompose_corners(font, r.layer, &which).map_err(|e| p.bad(e.to_string()))?;
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

// ── Component from Selection ────────────────────────────────────────────────────────────────────

/// `_part.` and the glyph's name, with `.001`, `.002`… appended while the name is taken.
pub(crate) fn suggested_part_name(font: &Font, glyph: &str) -> String {
    let base = format!("_part.{glyph}");
    if font.glyph(&base).is_none() {
        return base;
    }
    (1..=MAX_GLYPHS).map(|n| format!("{base}.{n:03}")).find(|n| font.glyph(n).is_none()).unwrap_or(base)
}

/// One master layer split by Component from Selection.
struct Split {
    /// The new glyph's layer: the moved shapes and the hints on them.
    part: Layer,
    /// The original layer with a component in their place.
    rest: Layer,
    /// Whether a hint linked moved and kept shapes and was removed.
    dropped_hints: usize,
}

/// Moves the shapes of `layer` at `picked` (sorted, all present) into a new layer and puts a
/// component of `name` where the first of them was.
fn split(layer: &Layer, picked: &BTreeSet<usize>, name: &str) -> Split {
    let first = picked.first().copied().unwrap_or(0);
    let mut part = Layer::new(layer.master.as_str(), layer.width);
    let mut rest = layer.clone();
    rest.shapes = Vec::with_capacity(layer.shapes.len().saturating_sub(picked.len()).saturating_add(1));
    let mut moved: Vec<Option<usize>> = vec![None; layer.shapes.len()];
    let mut kept: Vec<Option<usize>> = vec![None; layer.shapes.len()];
    for (i, shape) in layer.shapes.iter().enumerate() {
        if i == first {
            rest.shapes.push(Shape::Component(Component::new(name, 0.0, 0.0)));
        }
        if picked.contains(&i) {
            if let Some(slot) = moved.get_mut(i) {
                *slot = Some(part.shapes.len());
            }
            part.shapes.push(shape.clone());
        } else {
            if let Some(slot) = kept.get_mut(i) {
                *slot = Some(rest.shapes.len());
            }
            rest.shapes.push(shape.clone());
        }
    }
    let mut dropped_hints = 0;
    if !layer.info.hints.is_empty() {
        let len = layer.shapes.len();
        let (mut stay, mut go) = (Vec::new(), Vec::new());
        let in_map = |map: &[Option<usize>], i: usize| map.get(i).is_some_and(Option::is_some);
        for h in layer.info.hints.iter() {
            let refs = hint_shapes(h);
            if refs.iter().all(|&i| i >= len || in_map(kept.as_slice(), i)) {
                // A reference past the old shapes pointed at nothing before and is left as it is.
                stay.extend(remap_hint(h, &|i| kept.get(i).map_or(Some(i), |m| *m)));
            } else if refs.iter().all(|&i| in_map(moved.as_slice(), i)) {
                go.extend(remap_hint(h, &|i| moved.get(i).copied().flatten()));
            } else {
                dropped_hints += 1;
            }
        }
        rest.info.hints = stay;
        if !go.is_empty() {
            part.info.hints = go;
        }
    }
    Split { part, rest, dropped_hints }
}

fn component_from_selection(s: &mut Session, value: &Value) -> Result<Value> {
    const ID: &str = "glyph.componentFromSelection";
    let p = Params::new(ID, value)?;
    let mut t = Targets::parse(&p)?;
    if t.side != LayerSide::Foreground {
        return Err(p.bad("works on the foreground of a layer"));
    }
    if !(t.points.is_empty() && t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `shapes` only"));
    }
    let data = s.glyph_data()?;
    let name = match p.opt_str("name")? {
        Some(n) => n.to_string(),
        None => suggested_part_name(s.font()?, &t.glyph),
    };
    if s.font()?.glyph(&name).is_some() {
        return Err(p.bad(format!("there is already a glyph `{name}`: choose another name")));
    }
    let (layer_id, first, skipped, warnings) = s.edit("Component from Selection", |font| {
        let (layer_id, first, width, part_layers, rewrites, skipped, warnings) = {
            let r = t.resolve(font)?;
            crate::check_unlocked(r.glyph)?;
            let source = r.layer;
            if !source.is_master_layer() {
                return Err(p.bad(format!("layer `{}` is not a master layer: Component from Selection works on master layers", source.id)));
            }
            let picked: BTreeSet<usize> = if t.shapes.is_empty() {
                source.shapes.iter().enumerate().filter(|(_, sh)| matches!(sh, Shape::Path(_) | Shape::Component(_))).map(|(i, _)| i).collect()
            } else {
                t.shapes.iter().copied().collect()
            };
            let Some(&first) = picked.first() else { return Err(p.bad("the layer has no paths or components to move")) };
            let mut kinds = Vec::with_capacity(picked.len());
            for &i in &picked {
                kinds.push(match source.shapes.get(i) {
                    Some(Shape::Path(_)) => true,
                    Some(Shape::Component(_)) => false,
                    _ => return Err(p.bad(format!("shape {i} is not a path or a component"))),
                });
            }
            let glyph: &Glyph = r.glyph;
            let (mut part_layers, mut rewrites, mut skipped, mut warnings) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for m in font.masters.iter() {
                let Some(layer) = glyph.layer(&m.id) else {
                    part_layers.push(Layer::new(m.id.as_str(), source.width));
                    skipped.push(json!({"layer": m.id, "reason": "the glyph has no layer for this master"}));
                    continue;
                };
                let same = picked.iter().zip(&kinds).all(|(&i, &path)| match layer.shapes.get(i) {
                    Some(Shape::Path(_)) => path,
                    Some(Shape::Component(_)) => !path,
                    _ => false,
                });
                if !same {
                    part_layers.push(Layer::new(m.id.as_str(), layer.width));
                    skipped.push(json!({"layer": m.id, "reason": "the layer has no shapes of the same kinds at those indices"}));
                    continue;
                }
                let sp = split(layer, &picked, &name);
                if sp.dropped_hints > 0 {
                    warnings.push(format!("layer `{}`: {} hints linked moved and kept shapes and were removed", m.id, sp.dropped_hints));
                }
                part_layers.push(sp.part);
                rewrites.push(sp.rest);
            }
            (source.id.clone(), first, source.width, part_layers, rewrites, skipped, warnings)
        };
        let index = fontcraft_model::insertion_index(font, &data, &Glyph::new(name.as_str()));
        font.add_glyph_at(index, &name, Vec::new(), width)?;
        let mut part = font.glyph_mut(&name)?;
        part.layers = part_layers;
        // An underscore name makes a part that does not export (`components.underscoreNameNotExported`).
        part.export = !name.starts_with('_');
        drop(part);
        let mut g = font.glyph_mut(&t.glyph)?;
        for layer in rewrites {
            if let Some(slot) = g.layers.iter_mut().find(|l| l.id == layer.id) {
                *slot = layer;
            }
        }
        Ok((layer_id, first, skipped, warnings))
    })?;
    t.layer = Some(layer_id.clone());
    t.shapes = vec![first];
    Ok(json!({
        "name": name,
        "glyph": t.glyph,
        "layer": layer_id,
        "component": first,
        "skipped": skipped,
        "warnings": warnings,
        "selection": t.to_json(),
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use fontcraft_model::Path;

    use super::*;

    /// The command's budget runs across layers: layers that each fit their own limits are refused
    /// once together they pass the command's, and the budget says it is spent.
    #[test]
    fn one_budget_covers_every_layer_of_a_command() {
        let mut font = Font::new("Budget");
        let master = font.masters.first().unwrap().id.clone();
        let mut base = Glyph::new("box");
        let mut l = Layer::new(master.as_str(), 500.0);
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        base.layers.push(l);
        font.glyphs.push(base);
        let mut user = Layer::new(master.as_str(), 500.0);
        user.shapes.push(Shape::Component(Component::new("box", 0.0, 0.0)));

        let mut call = CallBudget { nodes_left: MAX_CALL_NODES, shapes_left: 3, visits_left: MAX_CALL_VISITS, spent: false };
        for _ in 0..3 {
            decompose_shapes(&font, &user, None, true, &mut call).unwrap();
        }
        let e = decompose_shapes(&font, &user, None, true, &mut call).err().unwrap();
        assert!(e.contains("over all the layers"), "{e}");
        assert!(call.spent);
    }
}
