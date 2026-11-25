//! Component commands (M6.5): Create Composite, Add Component, the component Info box values
//! (transform, base glyph, attachment anchor), moves, automatic alignment on and off, locks,
//! the list of glyphs that use a glyph, and the List View Components cell.
//!
//! Rows: `spec/inventory/09-reusing-shapes.toml` (`glyph.createComposite*`, `glyph.addComponent*`,
//! `component.*`, `glyph.listComposites`, `components.swapBaseKeepsPlacement`,
//! `components.lockedCannotMove`, `components.autoAlignedCannotMove`,
//! `components.arrowKeyIncrements`), `03b-edit-view-display.toml` (`component.setTransform`,
//! `component.setBaseGlyph`) and `05-font-view.toml` (`glyph.setComponents`).
//!
//! Components are addressed by shape index on one layer (`component: index`,
//! `components: [indices]`), as path commands address paths. The engine re-aligns the
//! composites an edit touches after every command ([`fontcraft_components::metrics::realign`]),
//! so these commands only write what the user chose; aligned positions follow. An aligned
//! component refuses a move or a position edit with a message, except the vertical part when it
//! is aligned horizontally only; a locked component refuses every move and transform edit.

use std::collections::HashMap;

use fontcraft_components::align::{self, AlignMode};
use fontcraft_components::compose;
use fontcraft_components::propagate::{base_layer, effective_anchors_with};
use fontcraft_components::{ComponentIndex, MAX_ANCHOR_NAME_LEN, MAX_LAYER_COMPONENTS};
use fontcraft_geom::round_to_grid;
use fontcraft_hinting::Renumbered;
use fontcraft_model::{Alignment, Component, Font, Layer, LayerSide, MAX_COORDINATE, Shape, Transform};
use serde_json::{Value, json};

use super::common::{grid, in_range};
use super::shared::renumber_hints;
use super::*;
use crate::params::{MAX_TARGETS, Params, TargetKind as K, Targets, codepoint, kind};

/// Largest scale the Info box accepts, in percent.
const MAX_SCALE_PERCENT: f64 = 10_000.0;
/// Largest slant, in degrees either way: at 90° the slant is infinite.
const MAX_SLANT: f64 = 89.0;
/// Largest rotation, in degrees either way.
const MAX_ROTATION: f64 = 360.0;
/// Below this a matrix entry or scale counts as zero.
const TINY: f64 = 1e-12;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.createComposite",
            "Create Composite",
            ["Glyph"],
            Some("Cmd+Ctrl+C"),
            "{glyphs: [names] | glyph: name, layer?: id (default the first master's layer), allMasters?: bool = false} replaces every path and component on the layer with the components the glyph database names for the glyph, in its order (allMasters: on every master layer). Anchors and metrics keys stay. A glyph nothing decomposes, or whose parts the font lacks, refuses the whole command with a message naming them → {glyphs: [{glyph, components: [names]}]}",
            has_doc,
            create_composite
        )
        .targets(&[K::Glyphs]),
        cmd!(
            "glyph.createCompositeAllMasters",
            "Create Composite (all masters)",
            ["Glyph"],
            Some("Cmd+Ctrl+Alt+C"),
            "{glyphs: [names] | glyph: name} glyph.createComposite on every master layer → {glyphs: [{glyph, components: [names]}]}",
            has_doc,
            create_composite_all_masters
        )
        .targets(&[K::Glyphs]),
        cmd!(
            "glyph.addComponent",
            "Add Component",
            ["Glyph"],
            Some("Cmd+Shift+C"),
            "{glyph: name, layer?: id (default the first master's layer), component: name | unicode (a code point as a number or \"U+00C1\"), allMasters?: bool = false} appends a component of that glyph at offset zero (aligned when the layer allows it); allMasters adds it on every master layer. A component that would make a glyph contain itself is refused → {glyph, base, layers: [ids], shape: index on the addressed layer}",
            has_doc,
            add_component
        )
        .targets(&[K::Layer])
        .context(&["canvas"]),
        cmd!(
            "glyph.addComponentAllMasters",
            "Add Component (all masters)",
            ["Glyph"],
            Some("Cmd+Alt+Shift+C"),
            "{glyph: name, component: name | unicode} glyph.addComponent on every master layer → {glyph, base, layers: [ids], shape: index}",
            has_doc,
            add_component_all_masters
        )
        .targets(&[K::Layer]),
        cmd!(
            "component.setTransform",
            "Set Component Transform",
            [],
            None,
            "{glyph: name, layer?: id, component?: index (default the layer's only component), x?: number, y?: number, scaleX?: percent, scaleY?: percent, slant?: degrees (rightward, within ±89), rotation?: degrees (counterclockwise, within ±360)} sets the values the component Info box shows; scales are not 0 and within ±10,000 %. Position edits are refused on an aligned component (only y when it is aligned horizontally) and every edit on a locked one → {glyph, layer, component, x, y, scaleX, scaleY, slant, rotation}",
            has_doc,
            set_transform
        )
        .targets(&[K::Layer]),
        cmd!(
            "component.setBaseGlyph",
            "Change Component Glyph",
            [],
            None,
            "{glyph: name, layer?: id, component?: index (default the layer's only component), base: name} points the component at another glyph; its slot in the shape order, transform, alignment setting and attachment anchor stay. A base that would make a glyph contain itself is refused → {glyph, layer, component, base}",
            has_doc,
            set_base_glyph
        )
        .targets(&[K::Layer])
        .context(&["canvas.component"]),
        cmd!(
            "component.move",
            "Move Component",
            [],
            None,
            "{glyph: name, layer?: id, components?: [index] (default every component of the layer), dx?: number = 0, dy?: number = 0} moves the components, rounded to the grid; the arrow keys send 1, 10 with Shift and 100 with Cmd. A locked or aligned component refuses the move; one aligned horizontally only takes dy → {glyph, layer, moved: [index]}",
            has_doc,
            move_components
        )
        .targets(&[K::Layer]),
        cmd!(
            "component.setAnchor",
            "Choose Attachment Anchor",
            [],
            None,
            "{glyph: name, layer?: id, component: index, anchor: name (\"\" clears it)} chooses which anchor of the glyph before it the component attaches to: the name of one of its own underscore anchors without the underscore (\"top\" for \"_top\"), or that name with a suffix (\"top_viet\", \"top_2\") → {glyph, layer, component, anchor: name | null}",
            has_doc,
            set_anchor
        )
        .targets(&[K::Layer])
        .context(&["canvas.component"]),
        cmd!(
            "component.enableAutomaticAlignment",
            "Enable Automatic Alignment",
            [],
            None,
            "{glyph: name, layer?: id, components?: [index] (default every component of the layer)} aligns the components again and places them: decided by the layer on a layer of components only, forced on a layer that has paths → {glyph, layer, components: [index]}",
            has_doc,
            enable_alignment
        )
        .targets(&[K::Layer])
        .context(&["canvas.component"]),
        cmd!(
            "component.disableAutomaticAlignment",
            "Disable Automatic Alignment",
            [],
            None,
            "{glyph: name, layer?: id, components?: [index] (default every component of the layer)} stops aligning the components; they keep their position and can be moved by hand → {glyph, layer, components: [index]}",
            has_doc,
            disable_alignment
        )
        .targets(&[K::Layer])
        .context(&["canvas.component"]),
        cmd!(
            "component.lock",
            "Lock Component",
            [],
            None,
            "{glyph: name, layer?: id, components?: [index] (default every component of the layer)} locks the components so nothing moves them; an aligned component is refused (disable its alignment first). The flag is the one path.lockPath sets → {glyph, layer, components: [index]}",
            has_doc,
            lock
        )
        .targets(&[K::Layer])
        .context(&["canvas.component"]),
        cmd!(
            "component.unlock",
            "Unlock Component",
            [],
            None,
            "{glyph: name, layer?: id, components?: [index] (default every component of the layer)} unlocks the components → {glyph, layer, components: [index]}",
            has_doc,
            unlock
        )
        .targets(&[K::Layer])
        .context(&["canvas.component"]),
        cmd!(
            query "glyph.listComposites",
            "List Glyphs Using Glyph as Component",
            [],
            None,
            "{glyph: name, recursive?: bool = false} the glyphs with a component of the glyph, in font order; recursive adds the glyphs that use those, in dependency order → {glyph, composites: [names]}",
            has_doc,
            list_composites
        ),
        cmd!(
            "glyph.setComponents",
            "Edit Components List",
            [],
            None,
            "{glyph: name, layer?: id (default the first master's layer), components: [names] | \"a, b\"} makes the layer's components match the list: a name the list has more often than the layer appends a component at offset zero; a name the layer has more often loses its last components. Paths stay → {glyph, layer, components: [names]}",
            has_doc,
            set_components
        )
        .targets(&[K::Layer]),
    ]
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// The addressed layer: `glyph` and `layer?` (the first master's layer by default).
fn layer_target(p: &Params) -> Result<Targets> {
    Ok(Targets::new(p.command(), p.str("glyph")?, p.opt_str("layer")?.map(str::to_string)))
}

/// One shape index from `key`.
fn opt_index(p: &Params, key: &str) -> Result<Option<usize>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| p.bad(format!("`{key}` must be a whole number from 0, found {v}"))),
    }
}

/// A list of shape indices from `key`, repeats dropped, at most [`MAX_TARGETS`].
fn opt_indices(p: &Params, key: &str) -> Result<Option<Vec<usize>>> {
    let Some(items) = p.opt_array(key)? else { return Ok(None) };
    if items.len() > MAX_TARGETS {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {MAX_TARGETS} are allowed", items.len())));
    }
    let mut out: Vec<usize> = Vec::with_capacity(items.len());
    for v in items {
        let i =
            v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad(format!("`{key}` must hold whole numbers from 0, found {v}")))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(Some(out))
}

/// The components named by `components?` on `layer`, or every component of it. An index that is
/// not a component is an error.
fn component_indices(p: &Params, layer: &Layer) -> Result<Vec<usize>> {
    match opt_indices(p, "components")? {
        Some(list) => {
            if list.is_empty() {
                return Err(p.bad("`components` is empty"));
            }
            for &i in &list {
                check_component(p, "components", layer, i)?;
            }
            Ok(list)
        }
        None => {
            let all: Vec<usize> = layer.shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Component(_))).map(|(i, _)| i).collect();
            if all.is_empty() {
                return Err(p.bad(format!("layer `{}` has no components", layer.id)));
            }
            Ok(all)
        }
    }
}

/// The component named by `component?`, or the layer's only component.
fn one_component(p: &Params, layer: &Layer) -> Result<usize> {
    if let Some(i) = opt_index(p, "component")? {
        check_component(p, "component", layer, i)?;
        return Ok(i);
    }
    let mut it = layer.shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Component(_)));
    match (it.next(), it.next()) {
        (Some((i, _)), None) => Ok(i),
        (None, _) => Err(p.bad(format!("layer `{}` has no components", layer.id))),
        _ => Err(p.bad("the layer has several components: give `component`")),
    }
}

fn check_component(p: &Params, key: &str, layer: &Layer, i: usize) -> Result<()> {
    match layer.shapes.get(i) {
        Some(Shape::Component(_)) => Ok(()),
        Some(_) => Err(p.bad(format!("`{key}`: shape {i} is not a component"))),
        None => Err(p.bad(format!("`{key}`: {i} does not resolve: the layer has {} shapes", layer.shapes.len()))),
    }
}

fn component_at(layer: &Layer, i: usize) -> Option<&Component> {
    match layer.shapes.get(i) {
        Some(Shape::Component(c)) => Some(c),
        _ => None,
    }
}

fn component_at_mut(layer: &mut Layer, i: usize) -> Option<&mut Component> {
    match layer.shapes.get_mut(i) {
        Some(Shape::Component(c)) => Some(c),
        _ => None,
    }
}

/// The glyph `key` names: a glyph name first, else a code point some glyph carries.
fn base_glyph(p: &Params, font: &Font, key: &str) -> Result<String> {
    let v = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    if let Value::String(s) = v
        && font.glyph(s).is_some()
    {
        return Ok(s.clone());
    }
    if let Some(cp) = codepoint(v)
        && let Some(g) = font.glyphs.iter().find(|g| g.unicodes.contains(&cp))
    {
        return Ok(g.name.clone());
    }
    match v {
        Value::String(s) => Err(p.bad(format!("`{key}`: there is no glyph `{s}`"))),
        Value::Number(n) => Err(p.bad(format!("`{key}`: no glyph has the code point {n}"))),
        other => Err(p.bad(format!("`{key}` must be a glyph name or a code point, got {}", kind(other)))),
    }
}

/// Refuses a component of `base` in `glyph` when `base` is `glyph` or contains it.
fn check_no_cycle(p: &Params, index: &ComponentIndex, glyph: &str, base: &str) -> Result<()> {
    if base == glyph || index.users(glyph).iter().any(|u| u == base) {
        return Err(p.bad(format!("a component of `{base}` in `{glyph}` would make the glyph contain itself")));
    }
    Ok(())
}

fn components_error(p: &Params, e: fontcraft_components::ComponentsError) -> crate::EngineError {
    p.bad(e.to_string())
}

// ── Create Composite ────────────────────────────────────────────────────────────────────────────

fn create_composite(s: &mut Session, v: &Value) -> Result<Value> {
    run_create_composite(s, v, "glyph.createComposite", None)
}

fn create_composite_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    run_create_composite(s, v, "glyph.createCompositeAllMasters", Some(true))
}

fn run_create_composite(s: &mut Session, v: &Value, id: &'static str, all: Option<bool>) -> Result<Value> {
    let p = Params::new(id, v)?;
    let font = s.doc()?.font.clone();
    let names = match p.opt_glyphs("glyphs", &font)? {
        Some(names) => names,
        None => {
            let name = p.str("glyph")?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("there is no glyph `{name}`")));
            }
            vec![name.to_string()]
        }
    };
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let all_masters = match all {
        Some(a) => a,
        None => p.bool_or("allMasters", false)?,
    };
    let layer_id = match p.opt_str("layer")? {
        Some(l) if !all_masters => l.to_string(),
        _ => font.default_master()?.id.clone(),
    };
    let data = s.glyph_data()?;
    // Every glyph is decomposed before anything changes, so one refusal leaves the font alone.
    let mut built = Vec::with_capacity(names.len());
    for name in &names {
        let g = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        crate::check_unlocked(g)?;
        if !all_masters && !g.layers.iter().any(|l| l.id == layer_id) {
            return Err(p.bad(format!("glyph `{name}` has no layer `{layer_id}`")));
        }
        let parts = compose::components_for(&font, &data, name)
            .map_err(|e| components_error(&p, e))?
            .ok_or_else(|| p.bad(format!("the glyph database names no components for `{name}`")))?;
        let composite = compose::composite_glyph(&font, &data, name, &parts).map_err(|e| components_error(&p, e))?;
        built.push((name.clone(), parts, composite));
    }
    let done = s.edit(if all_masters { "Create Composite (all masters)" } else { "Create Composite" }, |f| {
        let mut done = Renumbered::default();
        for (name, _, composite) in &built {
            let mut g = f.glyph_mut(name)?;
            if all_masters {
                for new in composite.layers.iter().filter(|l| l.is_master_layer()) {
                    match g.layers.iter_mut().find(|l| l.id == new.id) {
                        Some(layer) => {
                            replace_all_shapes(layer, new.shapes.clone(), name, &mut done);
                            layer.width = new.width;
                        }
                        None => g.layers.push(new.clone()),
                    }
                }
            } else {
                let layer =
                    g.layers.iter_mut().find(|l| l.id == layer_id).ok_or_else(|| p.bad(format!("glyph `{name}` has no layer `{layer_id}`")))?;
                let new = composite.layer(&layer.master).ok_or_else(|| p.bad(format!("`{name}` has no composite for master `{}`", layer.master)))?;
                replace_all_shapes(layer, new.shapes.clone(), name, &mut done);
                layer.width = new.width;
            }
        }
        Ok(done)
    })?;
    s.hints_renumbered(done);
    let out: Vec<Value> = built.iter().map(|(name, parts, _)| json!({"glyph": name, "components": parts})).collect();
    Ok(json!({ "glyphs": out }))
}

/// Puts `shapes` in place of every shape of `layer` (of glyph `glyph`), with its hints kept where
/// they can be ([`renumber_hints`]): a PostScript end on an old path becomes a number at its node's
/// place, a TrueType instruction or corner component on one is removed and reported in `done`,
/// other hints on old shapes are dropped, and hints without node references stay. The layer is
/// added to `done`, so the session's hint upkeep leaves it alone.
fn replace_all_shapes(layer: &mut Layer, shapes: Vec<Shape>, glyph: &str, done: &mut Renumbered) {
    if layer.info.hints.is_empty() {
        layer.shapes = shapes;
        return;
    }
    let before = layer.clone();
    layer.shapes = shapes;
    renumber_hints(&before, layer, LayerSide::Foreground, glyph, &|_| None, &mut done.report);
    done.insert(glyph, &layer.id, LayerSide::Foreground);
}

// ── Add Component ───────────────────────────────────────────────────────────────────────────────

fn add_component(s: &mut Session, v: &Value) -> Result<Value> {
    run_add_component(s, v, "glyph.addComponent", None)
}

fn add_component_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    run_add_component(s, v, "glyph.addComponentAllMasters", Some(true))
}

fn run_add_component(s: &mut Session, v: &Value, id: &'static str, all: Option<bool>) -> Result<Value> {
    let p = Params::new(id, v)?;
    let t = layer_target(&p)?;
    let all_masters = match all {
        Some(a) => a,
        None => p.bool_or("allMasters", false)?,
    };
    let font = s.doc()?.font.clone();
    let base = base_glyph(&p, &font, "component")?;
    check_no_cycle(&p, &ComponentIndex::new(&font), &t.glyph, &base)?;
    let addressed = t.resolve(&font)?.layer.id.clone();
    let label = if all_masters { "Add Component (all masters)" } else { "Add Component" };
    let (layers, shape) = s.edit(label, |f| {
        let mut changed = Vec::new();
        let mut shape = None;
        if all_masters {
            // The checks of the addressed layer (glyph lock, layer) run first.
            t.edit_layer(f, |_| Ok(()))?;
            let mut g = f.glyph_mut(&t.glyph)?;
            for layer in g.layers.iter_mut().filter(|l| l.is_master_layer()) {
                let i = append(&p, layer, &base)?;
                if layer.id == addressed {
                    shape = Some(i);
                }
                changed.push(layer.id.clone());
            }
        } else {
            t.edit_layer(f, |layer| {
                shape = Some(append(&p, layer, &base)?);
                changed.push(layer.id.clone());
                Ok(())
            })?;
        }
        Ok((changed, shape))
    })?;
    Ok(json!({ "glyph": t.glyph, "base": base, "layers": layers, "shape": shape }))
}

/// Appends a component of `base` at offset zero; its shape index.
fn append(p: &Params, layer: &mut Layer, base: &str) -> Result<usize> {
    let count = layer.shapes.iter().filter(|s| matches!(s, Shape::Component(_))).count();
    if count >= MAX_LAYER_COMPONENTS {
        return Err(p.bad(format!("layer `{}` already holds {MAX_LAYER_COMPONENTS} components", layer.id)));
    }
    layer.shapes.push(Shape::Component(Component::new(base, 0.0, 0.0)));
    Ok(layer.shapes.len().saturating_sub(1))
}

// ── Transform ───────────────────────────────────────────────────────────────────────────────────

/// A transform as the Info box shows it: position, scale, rotation in degrees and slant in
/// degrees, in the model's order (translate · slant · rotate · scale).
type Fields = ((f64, f64), (f64, f64), f64, (f64, f64));

/// The Info box values of `t`; `None` for a matrix that is singular or cannot be split.
fn fields_of(t: &Transform) -> Option<Fields> {
    match *t {
        Transform::Decomposed { pos, scale, angle, slant } => Some((pos, scale, angle, slant)),
        Transform::Matrix(m) => split_matrix(m),
    }
}

/// Splits a matrix into translate · slant · rotate · scale with a horizontal slant only. The
/// linear part `M = K · R · S` with `K = [[1, k], [0, 1]]` holds exactly when the columns of
/// `K⁻¹ · M` are orthogonal, a quadratic in `k`; of its roots the gentlest slant that gives the
/// matrix back is taken. A mirror is a negative horizontal scale rather than a turn by 180°.
fn split_matrix([a, b, c, d, dx, dy]: [f64; 6]) -> Option<Fields> {
    if ![a, b, c, d, dx, dy].iter().all(|v| v.is_finite()) {
        return None;
    }
    if b == 0.0 && c == 0.0 {
        return (a.abs() > TINY && d.abs() > TINY).then_some(((dx, dy), (a, d), 0.0, (0.0, 0.0)));
    }
    // (a − k·b)(c − k·d) + b·d = 0, that is b·d·k² − (a·d + b·c)·k + (a·c + b·d) = 0.
    let (qa, qb, qc) = (b * d, -(a * d + b * c), a * c + b * d);
    let mut candidates = vec![0.0];
    if qa.abs() > TINY {
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            let r = disc.sqrt();
            candidates.push((-qb + r) / (2.0 * qa));
            candidates.push((-qb - r) / (2.0 * qa));
        }
    } else if qb.abs() > TINY {
        candidates.push(-qc / qb);
    }
    let magnitude = [a, b, c, d].iter().fold(1.0f64, |m, v| m.max(v.abs()));
    let mut best: Option<(f64, Fields)> = None;
    for k in candidates.into_iter().filter(|k| k.is_finite()) {
        let (n00, n01, n10, n11) = (a - k * b, c - k * d, b, d);
        let sx = n00.hypot(n10);
        if sx < TINY {
            continue;
        }
        let angle = n10.atan2(n00);
        let sy = -angle.sin() * n01 + angle.cos() * n11;
        let mut f: Fields = ((dx, dy), (sx, sy), angle.to_degrees(), (k.atan().to_degrees(), 0.0));
        if (f.2.abs() - 180.0).abs() < 1e-9 && sy < 0.0 {
            f = (f.0, (-sx, -sy), 0.0, f.3);
        }
        let back = Transform::Decomposed { pos: f.0, scale: f.1, angle: f.2, slant: f.3 }.affine().as_coeffs();
        let same = back.iter().zip([a, b, c, d]).all(|(p, q)| (p - q).abs() <= 1e-9 * magnitude);
        if same && best.as_ref().is_none_or(|(bk, _)| k.abs() < bk.abs()) {
            best = Some((k, f));
        }
    }
    best.map(|(_, f)| f)
}

/// Whether moving or repositioning the component at `i` is allowed, and in which directions:
/// `(x, y)`. Locked: neither; aligned: neither, or only y when it is aligned horizontally.
/// `selection.move` follows the same rule.
pub(crate) fn free_axes(font: &Font, layer: &Layer, i: usize) -> (bool, bool) {
    let Some(c) = component_at(layer, i) else { return (false, false) };
    if c.info.locked {
        return (false, false);
    }
    match align::component_alignment(font, layer, i) {
        Some(AlignMode::Full) => (false, false),
        Some(AlignMode::Horizontal) => (false, true),
        None => (true, true),
    }
}

fn set_transform(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("component.setTransform", v)?;
    let t = layer_target(&p)?;
    let font = s.doc()?.font.clone();
    let r = t.resolve(&font)?;
    let i = one_component(&p, r.layer)?;
    let c = component_at(r.layer, i).ok_or_else(|| p.bad(format!("shape {i} is not a component")))?;
    let x = p.opt_f64("x")?;
    let y = p.opt_f64("y")?;
    let scale_x = p.opt_f64("scaleX")?;
    let scale_y = p.opt_f64("scaleY")?;
    let slant = p.opt_f64("slant")?;
    let rotation = p.opt_f64("rotation")?;
    if [x, y, scale_x, scale_y, slant, rotation].iter().all(Option::is_none) {
        return Err(p.bad("give at least one of `x`, `y`, `scaleX`, `scaleY`, `slant`, `rotation`"));
    }
    for (key, v) in [("x", x), ("y", y)] {
        if let Some(v) = v
            && !in_range(v)
        {
            return Err(p.bad(format!("`{key}` must be within ±{MAX_COORDINATE}")));
        }
    }
    for (key, v) in [("scaleX", scale_x), ("scaleY", scale_y)] {
        if let Some(v) = v
            && (v == 0.0 || v.abs() > MAX_SCALE_PERCENT)
        {
            return Err(p.bad(format!("`{key}` must not be 0 and must be within ±{MAX_SCALE_PERCENT} %")));
        }
    }
    if slant.is_some_and(|v| v.abs() > MAX_SLANT) {
        return Err(p.bad(format!("`slant` must be within ±{MAX_SLANT}°")));
    }
    if rotation.is_some_and(|v| v.abs() > MAX_ROTATION) {
        return Err(p.bad(format!("`rotation` must be within ±{MAX_ROTATION}°")));
    }
    if c.info.locked {
        return Err(p.bad(format!("component {i} is locked: unlock it to change its transform")));
    }
    let (free_x, free_y) = free_axes(&font, r.layer, i);
    if (x.is_some() && !free_x) || (y.is_some() && !free_y) {
        return Err(p.bad(match (free_x, free_y) {
            (false, true) => {
                format!("component {i} is aligned horizontally: only `y` can be set; disable its automatic alignment to place it by hand")
            }
            _ => format!("component {i} is aligned automatically: disable its automatic alignment to place it by hand"),
        }));
    }
    let step = grid(&font);
    let x = x.map(|v| round_to_grid(v, step));
    let y = y.map(|v| round_to_grid(v, step));
    let only_position = [scale_x, scale_y, slant, rotation].iter().all(Option::is_none);
    let transform = match (c.transform, only_position) {
        // A position edit keeps a matrix a matrix.
        (Transform::Matrix([a, b, c, d, mx, my]), true) => Transform::Matrix([a, b, c, d, x.unwrap_or(mx), y.unwrap_or(my)]),
        (other, _) => {
            let ((px, py), (sx, sy), angle, (kx, ky)) = fields_of(&other)
                .ok_or_else(|| p.bad(format!("the transform of component {i} cannot be shown as position, scale, slant and rotation")))?;
            Transform::Decomposed {
                pos: (x.unwrap_or(px), y.unwrap_or(py)),
                scale: (scale_x.map_or(sx, |v| v / 100.0), scale_y.map_or(sy, |v| v / 100.0)),
                angle: rotation.unwrap_or(angle),
                slant: (slant.unwrap_or(kx), ky),
            }
        }
    };
    if !transform.numbers().iter().all(|v| in_range(*v)) || !transform.affine().as_coeffs().iter().all(|v| in_range(*v)) {
        return Err(p.bad("the resulting transform is out of range"));
    }
    s.edit("Set Component Transform", |f| {
        t.edit_layer(f, |layer| {
            if let Some(c) = component_at_mut(layer, i) {
                c.transform = transform;
            }
            Ok(())
        })
    })?;
    let ((px, py), (sx, sy), angle, (kx, _)) = fields_of(&transform).unwrap_or(((0.0, 0.0), (1.0, 1.0), 0.0, (0.0, 0.0)));
    Ok(json!({
        "glyph": t.glyph,
        "layer": r.layer.id,
        "component": i,
        "x": px,
        "y": py,
        "scaleX": sx * 100.0,
        "scaleY": sy * 100.0,
        "slant": kx,
        "rotation": angle,
    }))
}

// ── Base glyph ──────────────────────────────────────────────────────────────────────────────────

fn set_base_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("component.setBaseGlyph", v)?;
    let t = layer_target(&p)?;
    let font = s.doc()?.font.clone();
    let r = t.resolve(&font)?;
    let i = one_component(&p, r.layer)?;
    let base = p.str("base")?;
    if font.glyph(base).is_none() {
        return Err(p.bad(format!("`base`: there is no glyph `{base}`")));
    }
    check_no_cycle(&p, &ComponentIndex::new(&font), &t.glyph, base)?;
    let layer_id = r.layer.id.clone();
    s.edit("Change Component Glyph", |f| {
        t.edit_layer(f, |layer| {
            if let Some(c) = component_at_mut(layer, i)
                && c.glyph != base
            {
                c.glyph = base.to_string();
            }
            Ok(())
        })
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": layer_id, "component": i, "base": base }))
}

// ── Move ────────────────────────────────────────────────────────────────────────────────────────

fn move_components(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("component.move", v)?;
    let t = layer_target(&p)?;
    let dx = p.opt_f64("dx")?.unwrap_or(0.0);
    let dy = p.opt_f64("dy")?.unwrap_or(0.0);
    if !in_range(dx) || !in_range(dy) {
        return Err(p.bad(format!("`dx` and `dy` must be within ±{MAX_COORDINATE}")));
    }
    let font = s.doc()?.font.clone();
    let r = t.resolve(&font)?;
    let list = component_indices(&p, r.layer)?;
    // (index, dx, dy) for each component, after the lock and alignment rules.
    let mut moves = Vec::with_capacity(list.len());
    for &i in &list {
        if component_at(r.layer, i).is_some_and(|c| c.info.locked) {
            return Err(p.bad(format!("component {i} is locked: unlock it to move it")));
        }
        match free_axes(&font, r.layer, i) {
            (true, true) => moves.push((i, dx, dy)),
            (false, true) => {
                if dy == 0.0 && dx != 0.0 {
                    return Err(p.bad(format!("component {i} is aligned horizontally: it moves only up and down")));
                }
                moves.push((i, 0.0, dy));
            }
            _ => return Err(p.bad(format!("component {i} is aligned automatically: disable its automatic alignment to move it"))),
        }
    }
    let step = grid(&font);
    let layer_id = r.layer.id.clone();
    s.edit("Move Component", |f| {
        t.edit_layer(f, |layer| {
            for &(i, dx, dy) in &moves {
                let Some(c) = component_at_mut(layer, i) else { continue };
                let (x, y) = match &mut c.transform {
                    Transform::Matrix([.., x, y]) => (x, y),
                    Transform::Decomposed { pos, .. } => (&mut pos.0, &mut pos.1),
                };
                let (nx, ny) = (round_to_grid(*x + dx, step), round_to_grid(*y + dy, step));
                if !in_range(nx) || !in_range(ny) {
                    return Err(p.bad(format!("component {i} would move beyond ±{MAX_COORDINATE}")));
                }
                (*x, *y) = (nx, ny);
            }
            Ok(())
        })
    })?;
    let moved: Vec<usize> = moves.iter().map(|m| m.0).collect();
    Ok(json!({ "glyph": t.glyph, "layer": layer_id, "moved": moved }))
}

// ── Attachment anchor ───────────────────────────────────────────────────────────────────────────

fn set_anchor(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("component.setAnchor", v)?;
    let t = layer_target(&p)?;
    let font = s.doc()?.font.clone();
    let r = t.resolve(&font)?;
    let i = opt_index(&p, "component")?.ok_or_else(|| p.bad("`component` is required"))?;
    check_component(&p, "component", r.layer, i)?;
    let c = component_at(r.layer, i).ok_or_else(|| p.bad(format!("shape {i} is not a component")))?;
    let name = p.str("anchor")?;
    let anchor = if name.is_empty() {
        None
    } else {
        if name.len() > MAX_ANCHOR_NAME_LEN || name.chars().any(char::is_control) {
            return Err(p.bad(format!("`anchor` must be at most {MAX_ANCHOR_NAME_LEN} bytes without control characters")));
        }
        let data = s.glyph_data()?;
        // The base layer the component draws and attaches by, as propagation picks it.
        let base = font.glyph(&c.glyph).ok_or_else(|| p.bad(format!("the component's glyph `{}` does not exist", c.glyph)))?;
        let from = base_layer(base, r.layer, c).ok_or_else(|| p.bad(format!("glyph `{}` has no layer for master `{}`", c.glyph, r.layer.master)))?;
        let own = effective_anchors_with(&font, Some(&*data), &c.glyph, &from.id).map_err(|e| components_error(&p, e))?;
        let plain: Vec<&str> = own.iter().filter_map(|a| a.name.strip_prefix('_')).filter(|n| !n.is_empty()).collect();
        let fits = plain.iter().any(|base| name == *base || name.strip_prefix(base).is_some_and(|rest| rest.len() > 1 && rest.starts_with('_')));
        if !fits {
            return Err(p.bad(if plain.is_empty() {
                format!("`{}` has no attachment anchors (names starting with an underscore)", c.glyph)
            } else {
                format!("`{name}` is not an anchor `{}` attaches to; it attaches to {}", c.glyph, plain.join(", "))
            }));
        }
        Some(name.to_string())
    };
    let layer_id = r.layer.id.clone();
    s.edit("Choose Attachment Anchor", |f| {
        t.edit_layer(f, |layer| {
            if let Some(c) = component_at_mut(layer, i)
                && c.info.anchor != anchor
            {
                c.info.anchor = anchor.clone();
            }
            Ok(())
        })
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": layer_id, "component": i, "anchor": anchor }))
}

// ── Alignment and locks ─────────────────────────────────────────────────────────────────────────

fn enable_alignment(s: &mut Session, v: &Value) -> Result<Value> {
    set_flags(s, v, "component.enableAutomaticAlignment", "Enable Automatic Alignment", Flag::Align(true))
}

fn disable_alignment(s: &mut Session, v: &Value) -> Result<Value> {
    set_flags(s, v, "component.disableAutomaticAlignment", "Disable Automatic Alignment", Flag::Align(false))
}

fn lock(s: &mut Session, v: &Value) -> Result<Value> {
    set_flags(s, v, "component.lock", "Lock Component", Flag::Lock(true))
}

fn unlock(s: &mut Session, v: &Value) -> Result<Value> {
    set_flags(s, v, "component.unlock", "Unlock Component", Flag::Lock(false))
}

#[derive(Clone, Copy)]
enum Flag {
    Align(bool),
    Lock(bool),
}

fn set_flags(s: &mut Session, v: &Value, id: &'static str, label: &str, flag: Flag) -> Result<Value> {
    let p = Params::new(id, v)?;
    let t = layer_target(&p)?;
    let font = s.doc()?.font.clone();
    let r = t.resolve(&font)?;
    let list = component_indices(&p, r.layer)?;
    if let Flag::Lock(true) = flag
        && let Some(i) = list.iter().copied().find(|&i| align::component_alignment(&font, r.layer, i).is_some())
    {
        return Err(p.bad(format!("component {i} is aligned automatically: only components placed by hand can be locked")));
    }
    // Enabling aligns the component whatever the layer holds: a path on the layer turns the
    // layer's own rule off, so the component is forced there.
    let has_paths = r.layer.shapes.iter().any(|s| matches!(s, Shape::Path(_)));
    let enabled = if has_paths { Alignment::Forced } else { Alignment::Default };
    let layer_id = r.layer.id.clone();
    s.edit(label, |f| {
        t.edit_layer(f, |layer| {
            for &i in &list {
                let Some(c) = component_at_mut(layer, i) else { continue };
                match flag {
                    Flag::Align(on) => {
                        let want = if on { enabled } else { Alignment::Disabled };
                        if c.info.alignment != want {
                            c.info.alignment = want;
                        }
                    }
                    Flag::Lock(on) => {
                        if c.info.locked != on {
                            c.info.locked = on;
                        }
                    }
                }
            }
            Ok(())
        })?;
        // The positions follow at once, also where a component was aligned already but stands
        // elsewhere.
        if let Flag::Align(true) = flag
            && let Some(aligned) = align::align_layer(f, &t.glyph, &layer_id).map_err(|e| components_error(&p, e))?
        {
            t.edit_layer(f, |layer| {
                *layer = aligned;
                Ok(())
            })?;
        }
        Ok(())
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": layer_id, "components": list }))
}

// ── Listing and the Components cell ─────────────────────────────────────────────────────────────

fn list_composites(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.listComposites", v)?;
    let font = s.font()?;
    let glyph = p.str("glyph")?;
    if font.glyph(glyph).is_none() {
        return Err(p.bad(format!("there is no glyph `{glyph}`")));
    }
    let recursive = p.bool_or("recursive", false)?;
    let index = ComponentIndex::new(font);
    let composites: Vec<String> = if recursive { index.users(glyph) } else { index.direct_users(glyph).to_vec() };
    Ok(json!({ "glyph": glyph, "composites": composites }))
}

fn set_components(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setComponents", v)?;
    let t = layer_target(&p)?;
    let raw: Vec<&str> = match p.get("components") {
        Some(Value::String(text)) => text.split(',').collect(),
        Some(_) => p.strings("components")?,
        None => return Err(p.bad("`components` is required")),
    };
    if raw.len() > MAX_LAYER_COMPONENTS {
        return Err(p.bad(format!("`components` holds {} names; at most {MAX_LAYER_COMPONENTS} are allowed", raw.len())));
    }
    let wanted: Vec<String> = raw.iter().map(|n| n.trim()).filter(|n| !n.is_empty()).map(str::to_string).collect();
    let font = s.doc()?.font.clone();
    let index = ComponentIndex::new(&font);
    for name in &wanted {
        if font.glyph(name).is_none() {
            return Err(p.bad(format!("`components`: there is no glyph `{name}`")));
        }
        check_no_cycle(&p, &index, &t.glyph, name)?;
    }
    let r = t.resolve(&font)?;
    let layer_id = r.layer.id.clone();
    let (result, done) = s.edit("Edit Components List", |f| {
        let mut done = Renumbered::default();
        let result = t.edit_layer(f, |layer| match_components(layer, &wanted, &t.glyph, &mut done).map_err(|m| p.bad(m)))?;
        Ok((result, done))
    })?;
    s.hints_renumbered(done);
    Ok(json!({ "glyph": t.glyph, "layer": layer_id, "components": result }))
}

/// Makes the components of `layer` match `wanted` by count per name: extra components of a name
/// go from the end, missing ones are appended in the list's order. The layer's hints follow their
/// shapes to their new places ([`renumber_hints`]: paths keep their indices, so PostScript hints
/// and TrueType instructions stay as they are); a hint on a removed component goes with it, and the
/// layer of glyph `glyph` is added to `done`. A locked component that would go refuses the change.
/// The component names after.
fn match_components(layer: &mut Layer, wanted: &[String], glyph: &str, done: &mut Renumbered) -> std::result::Result<Vec<String>, String> {
    let mut want: HashMap<&str, usize> = HashMap::new();
    for n in wanted {
        *want.entry(n.as_str()).or_default() += 1;
    }
    let mut have: HashMap<String, usize> = HashMap::new();
    for s in &layer.shapes {
        if let Shape::Component(c) = s {
            *have.entry(c.glyph.clone()).or_default() += 1;
        }
    }
    // Remove from the end: keep the first `want` components of each name.
    let mut kept: HashMap<String, usize> = HashMap::new();
    let mut keep = vec![true; layer.shapes.len()];
    for (i, s) in layer.shapes.iter().enumerate() {
        if let Shape::Component(c) = s {
            let k = kept.entry(c.glyph.clone()).or_default();
            *k += 1;
            if *k > want.get(c.glyph.as_str()).copied().unwrap_or(0)
                && let Some(slot) = keep.get_mut(i)
            {
                *slot = false;
            }
        }
    }
    let locked: Vec<String> = layer
        .shapes
        .iter()
        .zip(&keep)
        .filter_map(|(s, k)| match s {
            Shape::Component(c) if !k && c.info.locked => Some(format!("`{}`", c.glyph)),
            _ => None,
        })
        .collect();
    if !locked.is_empty() {
        return Err(format!("the locked components {} would be removed: unlock them first", locked.join(", ")));
    }
    // The new index of every old shape; `None` for a removed one.
    let map: Vec<Option<usize>> = keep
        .iter()
        .scan(0usize, |next, &k| {
            let at = k.then_some(*next);
            *next += usize::from(k);
            Some(at)
        })
        .collect();
    let before = (!layer.info.hints.is_empty() && map.iter().any(Option::is_none)).then(|| layer.clone());
    let mut flags = keep.into_iter();
    layer.shapes.retain(|_| flags.next().unwrap_or(true));
    if let Some(before) = before {
        // A reference past the old shapes pointed at nothing before and is left as it is.
        let to = |i: usize| map.get(i).map_or(Some(i), |m| *m);
        renumber_hints(&before, layer, LayerSide::Foreground, glyph, &to, &mut done.report);
        done.insert(glyph, &layer.id, LayerSide::Foreground);
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for n in wanted {
        let k = seen.entry(n.as_str()).or_default();
        *k += 1;
        if *k > have.get(n.as_str()).copied().unwrap_or(0) {
            layer.shapes.push(Shape::Component(Component::new(n.as_str(), 0.0, 0.0)));
        }
    }
    Ok(layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Component(c) => Some(c.glyph.clone()),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Anchor, Glyph, Path};

    use super::*;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn refused(s: &mut Session, id: &str, p: Value) -> String {
        let before = s.doc().unwrap().font.clone();
        let e = s.execute(id, &p).expect_err("the command should refuse");
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changes nothing");
        e.to_string()
    }

    /// The starter font plus `A` (outline 20–580, `top` at (300, 700), `top_viet` at (360, 700)),
    /// `acutecomb` and `brevecomb` (`_top` at (0, 500), `top` at (0, 800)), an empty `Aacute`
    /// (U+00C1) and `Acomp` holding A and acutecomb, stored where alignment puts them.
    fn session() -> Session {
        let mut s = Session::with_document();
        s.edit("fixture", |f| {
            {
                let mut a = f.glyph_mut("A")?;
                let l = &mut a.layers[0];
                l.shapes.push(Shape::Path(Path::rect(20.0, 0.0, 580.0, 700.0)));
                l.anchors.push(Anchor::new("top", 300.0, 700.0));
                l.anchors.push(Anchor::new("top_viet", 360.0, 700.0));
            }
            for (name, cp) in [("acutecomb", 0x301), ("brevecomb", 0x306)] {
                let mut g = Glyph::new(name);
                g.unicodes.push(cp);
                let mut l = Layer::new("m01", 0.0);
                l.shapes.push(Shape::Path(Path::rect(-50.0, 520.0, 50.0, 620.0)));
                l.anchors.push(Anchor::new("_top", 0.0, 500.0));
                l.anchors.push(Anchor::new("top", 0.0, 800.0));
                g.layers.push(l);
                f.glyphs.push(g);
            }
            let mut aacute = Glyph::new("Aacute");
            aacute.unicodes.push(0xC1);
            aacute.layers.push(Layer::new("m01", 600.0));
            f.glyphs.push(aacute);
            let mut comp = Glyph::new("Acomp");
            let mut l = Layer::new("m01", 600.0);
            l.shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
            l.shapes.push(Shape::Component(Component::new("acutecomb", 300.0, 200.0)));
            comp.layers.push(l);
            f.glyphs.push(comp);
            Ok(())
        })
        .unwrap();
        s
    }

    fn layer<'a>(s: &'a Session, glyph: &str) -> &'a Layer {
        &s.font().unwrap().glyph(glyph).unwrap().layers[0]
    }

    fn comp(s: &Session, glyph: &str, i: usize) -> Component {
        component_at(layer(s, glyph), i).cloned().expect("a component")
    }

    fn offset(c: &Component) -> (f64, f64) {
        let k = c.affine().as_coeffs();
        (k[4], k[5])
    }

    /// `B` with a path and a component of `A` placed by hand at (10, 20).
    fn manual(s: &mut Session) {
        s.edit("manual", |f| {
            let mut b = f.glyph_mut("B")?;
            let l = &mut b.layers[0];
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
            l.shapes.push(Shape::Component(Component::new("A", 10.0, 20.0)));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn create_composite_builds_the_glyph_from_its_database_parts_and_undoes_to_the_very_font() {
        let mut s = session();
        let before = s.doc().unwrap().font.clone();
        let out = run(&mut s, "glyph.createComposite", json!({"glyphs": ["Aacute"]}));
        assert_eq!(out["glyphs"][0]["components"], json!(["A", "acutecomb"]));
        let names: Vec<String> = layer(&s, "Aacute").shapes.iter().filter_map(component_at_ref).collect();
        assert_eq!(names, ["A", "acutecomb"]);
        assert_eq!(offset(&comp(&s, "Aacute", 1)), (300.0, 200.0), "the acute sits on A's top anchor");
        run(&mut s, "edit.undo", json!({}));
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
    }

    fn component_at_ref(s: &Shape) -> Option<String> {
        match s {
            Shape::Component(c) => Some(c.glyph.clone()),
            _ => None,
        }
    }

    #[test]
    fn create_composite_all_masters_replaces_paths_on_every_master_layer() {
        let mut s = session();
        run(&mut s, "path.addPath", json!({"glyph": "Aacute", "nodes": [{"x": 0, "y": 0}, {"x": 10, "y": 0}, {"x": 10, "y": 10}]}));
        run(&mut s, "glyph.createCompositeAllMasters", json!({"glyph": "Aacute"}));
        assert!(layer(&s, "Aacute").shapes.iter().all(|s| matches!(s, Shape::Component(_))), "the path is gone");
        assert_eq!(layer(&s, "Aacute").shapes.len(), 2);
    }

    #[test]
    fn create_composite_refuses_missing_parts_and_unknown_compositions() {
        let mut s = session();
        s.edit("no ring", |f| f.add_glyph("Aring", vec![0xC5], 600.0).map_err(Into::into)).unwrap();
        let e = refused(&mut s, "glyph.createComposite", json!({"glyphs": ["Aacute", "Aring"]}));
        assert!(e.contains("ringcomb"), "the message names the missing part: {e}");
        let e = refused(&mut s, "glyph.createComposite", json!({"glyph": "B"}));
        assert!(e.contains("names no components"), "{e}");
    }

    #[test]
    fn add_component_by_name_or_code_point_and_refuses_unknown_glyphs_and_cycles() {
        let mut s = session();
        let out = run(&mut s, "glyph.addComponent", json!({"glyph": "C", "component": "A"}));
        assert_eq!(out["shape"], json!(0));
        let out = run(&mut s, "glyph.addComponent", json!({"glyph": "C", "component": "U+0301"}));
        assert_eq!(out["base"], json!("acutecomb"));
        assert_eq!(offset(&comp(&s, "C", 1)), (300.0, 200.0), "a mark added after a base is aligned");
        refused(&mut s, "glyph.addComponent", json!({"glyph": "C", "component": "nosuchglyph"}));
        refused(&mut s, "glyph.addComponent", json!({"glyph": "A", "component": "Acomp"}));
        refused(&mut s, "glyph.addComponent", json!({"glyph": "A", "component": "A"}));
    }

    #[test]
    fn add_component_all_masters_adds_on_each_master_layer() {
        let mut s = session();
        let out = run(&mut s, "glyph.addComponentAllMasters", json!({"glyph": "D", "component": "A"}));
        assert_eq!(out["layers"], json!(["m01"]));
        assert_eq!(comp(&s, "D", 0).glyph, "A");
    }

    #[test]
    fn set_transform_writes_info_box_values_and_refuses_locked_and_aligned_positions() {
        let mut s = session();
        manual(&mut s);
        let out = run(&mut s, "component.setTransform", json!({"glyph": "B", "component": 1, "x": 40, "scaleX": 50, "rotation": 90}));
        assert_eq!(out["x"], json!(40.0));
        assert_eq!(out["scaleX"], json!(50.0));
        assert_eq!(out["rotation"], json!(90.0));
        let Transform::Decomposed { pos, scale, angle, .. } = comp(&s, "B", 1).transform else { panic!("decomposed") };
        assert_eq!((pos, scale, angle), ((40.0, 20.0), (0.5, 1.0), 90.0));
        run(&mut s, "component.lock", json!({"glyph": "B", "components": [1]}));
        refused(&mut s, "component.setTransform", json!({"glyph": "B", "component": 1, "y": 0}));
        // Acomp's acute is aligned: its position cannot be set, its scale can.
        refused(&mut s, "component.setTransform", json!({"glyph": "Acomp", "component": 1, "x": 0}));
        run(&mut s, "component.setTransform", json!({"glyph": "Acomp", "component": 1, "scaleY": 80}));
    }

    #[test]
    fn a_matrix_splits_into_info_box_values_that_give_it_back() {
        for m in [[0.0, 1.0, -1.0, 0.0, 5.0, 6.0], [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [2.0, 0.0, 0.5, 1.0, 0.0, 0.0], [0.6, 0.8, -1.6, 1.2, 1.0, 2.0]] {
            let f = split_matrix(m).unwrap_or_else(|| panic!("{m:?} splits"));
            let back = Transform::Decomposed { pos: f.0, scale: f.1, angle: f.2, slant: f.3 }.affine().as_coeffs();
            for (p, q) in back.iter().zip(m) {
                assert!((p - q).abs() < 1e-9, "{m:?} → {f:?} → {back:?}");
            }
        }
        assert_eq!(split_matrix([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap().2, 0.0, "a mirror is a negative scale, not a turn");
        assert!(split_matrix([1.0, 1.0, 1.0, 1.0, 0.0, 0.0]).is_none(), "a singular matrix does not split");
    }

    #[test]
    fn set_base_glyph_swaps_the_glyph_and_keeps_placement_alignment_and_anchor() {
        let mut s = session();
        run(&mut s, "component.setAnchor", json!({"glyph": "Acomp", "component": 1, "anchor": "top_viet"}));
        let before = comp(&s, "Acomp", 1);
        run(&mut s, "component.setBaseGlyph", json!({"glyph": "Acomp", "component": 1, "base": "brevecomb"}));
        let after = comp(&s, "Acomp", 1);
        assert_eq!(after.glyph, "brevecomb");
        assert_eq!(
            (after.transform, after.info.anchor.clone(), after.info.alignment),
            (before.transform, before.info.anchor.clone(), before.info.alignment)
        );
        refused(&mut s, "component.setBaseGlyph", json!({"glyph": "Acomp", "component": 1, "base": "nosuchglyph"}));
        refused(&mut s, "component.setBaseGlyph", json!({"glyph": "Acomp", "component": 0, "base": "Acomp"}));
    }

    #[test]
    fn component_move_follows_the_arrow_key_increments_and_refuses_locked_and_aligned_components() {
        let mut s = session();
        manual(&mut s);
        for (dx, dy) in [(1.0, 0.0), (10.0, 0.0), (0.0, 100.0)] {
            run(&mut s, "component.move", json!({"glyph": "B", "components": [1], "dx": dx, "dy": dy}));
        }
        assert_eq!(offset(&comp(&s, "B", 1)), (21.0, 120.0));
        let e = refused(&mut s, "component.move", json!({"glyph": "Acomp", "components": [1], "dx": 5}));
        assert!(e.contains("aligned"), "{e}");
        refused(&mut s, "component.move", json!({"glyph": "B", "components": [0], "dx": 5}));
        run(&mut s, "component.lock", json!({"glyph": "B", "components": [1]}));
        let e = refused(&mut s, "component.move", json!({"glyph": "B", "components": [1], "dx": 5}));
        assert!(e.contains("locked"), "{e}");
        run(&mut s, "component.unlock", json!({"glyph": "B", "components": [1]}));
        run(&mut s, "component.move", json!({"glyph": "B", "components": [1], "dx": -1}));
        assert_eq!(offset(&comp(&s, "B", 1)), (20.0, 120.0));
    }

    #[test]
    fn a_component_aligned_horizontally_moves_only_vertically() {
        let mut s = session();
        s.edit("horizontal", |f| {
            let mut g = f.glyph_mut("Acomp")?;
            if let Some(c) = component_at_mut(&mut g.layers[0], 1) {
                c.info.alignment = Alignment::Horizontal;
            }
            Ok(())
        })
        .unwrap();
        let x = offset(&comp(&s, "Acomp", 1)).0;
        run(&mut s, "component.move", json!({"glyph": "Acomp", "components": [1], "dx": 50, "dy": 30}));
        assert_eq!(offset(&comp(&s, "Acomp", 1)), (x, 230.0), "the horizontal part is ignored");
        refused(&mut s, "component.move", json!({"glyph": "Acomp", "components": [1], "dx": 50}));
    }

    #[test]
    fn set_anchor_stores_a_suffixed_target_and_refuses_names_the_mark_cannot_take() {
        let mut s = session();
        let out = run(&mut s, "component.setAnchor", json!({"glyph": "Acomp", "component": 1, "anchor": "top_viet"}));
        assert_eq!(out["anchor"], json!("top_viet"));
        assert_eq!(comp(&s, "Acomp", 1).info.anchor.as_deref(), Some("top_viet"));
        assert_eq!(offset(&comp(&s, "Acomp", 1)), (360.0, 200.0), "the acute moves to the alternative anchor");
        refused(&mut s, "component.setAnchor", json!({"glyph": "Acomp", "component": 1, "anchor": "bottom"}));
        refused(&mut s, "component.setAnchor", json!({"glyph": "Acomp", "component": 1, "anchor": "topx"}));
        run(&mut s, "component.setAnchor", json!({"glyph": "Acomp", "component": 1, "anchor": ""}));
        assert_eq!(comp(&s, "Acomp", 1).info.anchor, None);
    }

    /// The anchors offered are those of the base layer the component draws: on a special layer,
    /// the base glyph's layer with the same id, not its master layer.
    #[test]
    fn set_anchor_reads_the_base_layer_the_component_draws() {
        let mut s = session();
        s.edit("special layers", |f| {
            f.glyph_mut("A")?.layers[0].anchors.push(Anchor::new("bottom", 300.0, 0.0));
            {
                let mut acute = f.glyph_mut("acutecomb")?;
                let mut l = acute.layers[0].clone();
                l.id = "x1".into();
                l.info.name = Some("Alt".into());
                l.anchors.push(Anchor::new("_bottom", 0.0, 0.0));
                acute.layers.push(l);
            }
            let mut comp = f.glyph_mut("Acomp")?;
            let mut l = comp.layers[0].clone();
            l.id = "x1".into();
            l.info.name = Some("Alt".into());
            comp.layers.push(l);
            Ok(())
        })
        .unwrap();
        run(&mut s, "component.setAnchor", json!({"glyph": "Acomp", "layer": "x1", "component": 1, "anchor": "bottom"}));
        refused(&mut s, "component.setAnchor", json!({"glyph": "Acomp", "component": 1, "anchor": "bottom"}));
    }

    #[test]
    fn alignment_can_be_disabled_and_enabled_again() {
        let mut s = session();
        run(&mut s, "component.disableAutomaticAlignment", json!({"glyph": "Acomp", "components": [1]}));
        assert_eq!(comp(&s, "Acomp", 1).info.alignment, Alignment::Disabled);
        run(&mut s, "component.move", json!({"glyph": "Acomp", "components": [1], "dx": 5}));
        assert_eq!(offset(&comp(&s, "Acomp", 1)), (305.0, 200.0));
        run(&mut s, "component.enableAutomaticAlignment", json!({"glyph": "Acomp", "components": [1]}));
        assert_eq!(comp(&s, "Acomp", 1).info.alignment, Alignment::Default, "a layer of components only decides by itself");
        assert_eq!(offset(&comp(&s, "Acomp", 1)), (300.0, 200.0), "the acute is aligned again");
        manual(&mut s);
        run(&mut s, "component.enableAutomaticAlignment", json!({"glyph": "B", "components": [1]}));
        assert_eq!(comp(&s, "B", 1).info.alignment, Alignment::Forced, "a layer with paths needs the forced setting");
    }

    #[test]
    fn lock_refuses_aligned_components_and_unlock_releases() {
        let mut s = session();
        let e = refused(&mut s, "component.lock", json!({"glyph": "Acomp", "components": [1]}));
        assert!(e.contains("aligned"), "{e}");
        manual(&mut s);
        run(&mut s, "component.lock", json!({"glyph": "B"}));
        assert!(comp(&s, "B", 1).info.locked);
        run(&mut s, "component.unlock", json!({"glyph": "B", "components": [1]}));
        assert!(!comp(&s, "B", 1).info.locked);
        refused(&mut s, "component.unlock", json!({"glyph": "B", "components": [0]}));
    }

    #[test]
    fn list_composites_names_direct_and_recursive_users() {
        let mut s = session();
        run(&mut s, "glyph.addComponent", json!({"glyph": "E", "component": "Acomp"}));
        assert_eq!(run(&mut s, "glyph.listComposites", json!({"glyph": "A"}))["composites"], json!(["Acomp"]));
        assert_eq!(run(&mut s, "glyph.listComposites", json!({"glyph": "A", "recursive": true}))["composites"], json!(["Acomp", "E"]));
        assert!(s.execute("glyph.listComposites", &json!({"glyph": "nosuchglyph"})).is_err());
    }

    #[test]
    fn set_components_adds_and_removes_to_match_the_list() {
        let mut s = session();
        let out = run(&mut s, "glyph.setComponents", json!({"glyph": "Acomp", "components": "A, brevecomb, acutecomb"}));
        assert_eq!(out["components"], json!(["A", "acutecomb", "brevecomb"]));
        let out = run(&mut s, "glyph.setComponents", json!({"glyph": "Acomp", "components": ["A"]}));
        assert_eq!(out["components"], json!(["A"]));
        refused(&mut s, "glyph.setComponents", json!({"glyph": "Acomp", "components": ["nosuchglyph"]}));
        refused(&mut s, "glyph.setComponents", json!({"glyph": "A", "components": ["Acomp"]}));
    }

    /// Hints follow their shapes when components before them go; a locked component is never
    /// removed silently.
    #[test]
    fn set_components_renumbers_hints_and_refuses_to_remove_a_locked_component() {
        use fontcraft_model::corner::{self, CornerHint, CornerTarget};
        let mut s = session();
        s.edit("path and hint", |f| {
            let mut g = f.glyph_mut("Acomp")?;
            let l = &mut g.layers[0];
            l.shapes.insert(1, Shape::Path(Path::rect(0.0, -100.0, 10.0, -90.0)));
            l.info.hints = vec![CornerHint::new("_corner.one", 1, 0).to_hint()];
            Ok(())
        })
        .unwrap();
        run(&mut s, "glyph.setComponents", json!({"glyph": "Acomp", "components": ["acutecomb"]}));
        let l = layer(&s, "Acomp");
        assert!(matches!(l.shapes[0], Shape::Path(_)), "{:?}", l.shapes);
        let hint = corner::corner_hint(&l.info.hints[0]).unwrap().unwrap();
        assert!(matches!(hint.target, CornerTarget::Node { shape: 0, .. }), "the hint follows its path to index 0: {:?}", hint.target);

        s.edit("lock", |f| {
            let mut g = f.glyph_mut("Acomp")?;
            if let Some(Shape::Component(c)) = g.layers[0].shapes.get_mut(1) {
                c.info.locked = true;
            }
            Ok(())
        })
        .unwrap();
        let e = refused(&mut s, "glyph.setComponents", json!({"glyph": "Acomp", "components": ["A"]}));
        assert!(e.contains("locked") && e.contains("acutecomb"), "{e}");
    }

    /// PostScript hints count paths: removing the component before a path moves its shape index
    /// but not its path index, so a stem hint on the path stays on its nodes.
    #[test]
    fn set_components_keeps_a_stem_hint_on_the_path_after_a_removed_component() {
        use fontcraft_hinting::{Edge, NodeRef, PsHint, ps_hints, set_ps_hints};
        let mut s = session();
        let node = |n| Edge::Node(NodeRef::new(0, n));
        s.edit("path and stem", |f| {
            let mut g = f.glyph_mut("Acomp")?;
            let l = &mut g.layers[0];
            l.shapes.insert(1, Shape::Path(Path::rect(0.0, -100.0, 10.0, -90.0)));
            set_ps_hints(l, &[PsHint::stem(false, node(0), node(1))]).unwrap();
            Ok(())
        })
        .unwrap();
        let r = run(&mut s, "glyph.setComponents", json!({"glyph": "Acomp", "components": ["acutecomb"]}));
        let l = layer(&s, "Acomp");
        let ps = ps_hints(l).unwrap();
        assert_eq!(ps.len(), 1, "the stem is kept: {:?}", l.info.hints);
        assert_eq!((ps[0].1.origin, ps[0].1.other), (node(0), Some(node(1))));
        assert_eq!(ps[0].1.edges(l).unwrap(), (0.0, 10.0));
        assert!(r.get("hintUpkeep").is_none(), "nothing is unlinked or removed: {r}");
    }

    /// Create Composite replaces every path: a stem hint's ends become numbers at their places,
    /// and the result says so.
    #[test]
    fn create_composite_unlinks_the_stem_hints_on_the_replaced_paths() {
        use fontcraft_hinting::{Edge, NodeRef, PsHint, ps_hints, set_ps_hints};
        let mut s = session();
        s.edit("path and stem", |f| {
            let mut g = f.glyph_mut("Aacute")?;
            let l = &mut g.layers[0];
            l.shapes.push(Shape::Path(Path::rect(100.0, 0.0, 180.0, 700.0)));
            let node = |n| Edge::Node(NodeRef::new(0, n));
            set_ps_hints(l, &[PsHint::stem(false, node(0), node(1))]).unwrap();
            Ok(())
        })
        .unwrap();
        let r = run(&mut s, "glyph.createComposite", json!({"glyph": "Aacute"}));
        let l = layer(&s, "Aacute");
        assert!(l.shapes.iter().all(|sh| matches!(sh, Shape::Component(_))), "{:?}", l.shapes);
        let ps = ps_hints(l).unwrap();
        assert_eq!(ps.len(), 1, "the stem is kept: {:?}", l.info.hints);
        assert_eq!((ps[0].1.origin, ps[0].1.other), (Edge::Value(100.0), Some(Edge::Value(180.0))));
        assert_eq!(r["hintUpkeep"]["unlinked"], json!(2), "{r}");
    }
}
