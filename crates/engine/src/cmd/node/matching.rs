//! The same edit, by the same addresses, on the other layers of the glyph that interpolate with the
//! edited one, in the edit's undo step: `matchLayers` of Insert Node, Delete, Make Node First and
//! Reverse Contours (M9.35, row `edit.toggleMatchWhenEditing`), and the "in All Masters" commands
//! (M3.52, [`super::masters`]).
//!
//! - A layer takes part when it is a master, intermediate or alternate layer and the edited side of
//!   it has the edited layer's shapes and node types ([`super::signature`]). With `matchLayers`, a
//!   backup layer and the background match nothing; an "in All Masters" command edits the side its
//!   targets name, and takes its addresses from any layer.
//! - A master or intermediate layer that does not match is left alone and named in the result. An
//!   alternate layer may be drawn differently, so it is left alone unnamed, as is a layer with no
//!   shapes yet.
//! - What depends on the drawing (the parameter a segment is split at, the new type of a toggled
//!   node, how nodes pair up) is settled on the edited layer and repeated on the others, so the
//!   layers stay compatible.
//! - After the edit every matched layer must have the edited layer's shapes and node types again;
//!   otherwise, as when one layer refuses the edit, the whole edit is refused.

use fontcraft_interp::labels::special_label;
use fontcraft_model::{Font, Layer, LayerPartsMut, LayerSide, Shape};
use serde_json::{Value, json};

use super::{Sig, signature};
use crate::params::{Params, Targets};
use crate::{EngineError, Result};

/// How the id of a command that edits every matching layer ends.
const IN_ALL_MASTERS: &str = "InAllMasters";

/// Whether `p` belongs to an "in All Masters" command.
fn all_masters(p: &Params) -> bool {
    p.command().ends_with(IN_ALL_MASTERS)
}

/// The undo label of the command of `p`, whose one-layer form is labelled `base`.
pub(in crate::cmd) fn undo_label(p: &Params, base: &str) -> String {
    if all_masters(p) { format!("{base} in All Masters") } else { base.to_string() }
}

/// What is edited beside the layer the targets name.
#[derive(Default)]
pub(in crate::cmd) struct Plan {
    glyph: String,
    side: LayerSide,
    /// The edited layer's id.
    source: String,
    /// The ids of the layers that take the same edit.
    layers: Vec<String>,
    /// The layers left alone, for the reply.
    skipped: Vec<Value>,
}

/// Whether `layer` takes part in interpolation.
fn is_source(layer: &Layer) -> bool {
    let a = &layer.info.attrs;
    let special = !a.coordinates.is_empty() || !a.axis_rules.is_empty();
    let outline = a.color_palette.is_none() && !a.full_color && !a.svg && a.sbix_size.is_none() && a.smart_poles.is_empty();
    layer.is_master_layer() || (special && outline)
}

impl Plan {
    /// The layers that match the one `t` names, before the edit; `None` without `matchLayers` in a command for one
    /// layer.
    pub(in crate::cmd) fn of(font: &Font, p: &Params, t: &Targets) -> Result<Option<Plan>> {
        let all = all_masters(p);
        if !all && !p.bool_or("matchLayers", false)? {
            return Ok(None);
        }
        let r = t.resolve(font)?;
        let mut plan = Plan { glyph: r.glyph.name.clone(), side: t.side, source: r.layer.id.clone(), ..Plan::default() };
        if !all && (t.side != LayerSide::Foreground || !is_source(r.layer)) {
            return Ok(Some(plan));
        }
        let sig = signature(r.parts);
        for layer in r.glyph.layers.iter().filter(|l| l.id != plan.source && is_source(l)) {
            let parts = layer.parts(t.side);
            let reason = if signature(parts) != sig {
                "its shapes or node types differ from the edited layer's".to_string()
            } else if let Some(name) = t.anchors.iter().find(|a| parts.anchor(a).is_none()) {
                format!("it has no anchor `{name}`")
            } else {
                plan.layers.push(layer.id.clone());
                continue;
            };
            if layer.info.attrs.axis_rules.is_empty() && !parts.shapes.is_empty() {
                let name = layer_name(font, layer);
                plan.skipped.push(json!({"glyph": plan.glyph, "layer": layer.id, "reason": format!("{name}: {reason}")}));
            }
        }
        Ok(Some(plan))
    }

    /// The matched layers.
    pub(in crate::cmd) fn layers<'f>(&self, font: &'f Font) -> Vec<&'f Layer> {
        font.glyph(&self.glyph).map(|g| g.layers.iter().filter(|l| self.layers.contains(&l.id)).collect()).unwrap_or_default()
    }

    /// Makes the edit `f`, which changes no structure, on every matched layer: `f` gets `t` naming that layer.
    pub(in crate::cmd) fn apply_targets(
        &self,
        font: &mut Font,
        p: &Params,
        t: &Targets,
        mut f: impl FnMut(&mut Font, &Targets) -> Result<()>,
    ) -> Result<()> {
        for id in &self.layers {
            let mut at = t.clone();
            at.layer = Some(id.clone());
            if let Err(e) = f(font, &at) {
                let name = font.glyph(&self.glyph).and_then(|g| g.layers.iter().find(|l| l.id == *id)).map(|l| layer_name(font, l));
                return Err(p.bad(format!("matching layer {}: {}", name.unwrap_or_else(|| id.clone()), message(e))));
            }
        }
        Ok(())
    }

    /// Makes the edit `f` on every matched layer, after it was made on the edited one. `f` gets the layer's id and
    /// its edited side.
    pub(in crate::cmd) fn apply(
        &self,
        font: &mut Font,
        p: &Params,
        mut f: impl FnMut(&str, &mut LayerPartsMut) -> std::result::Result<(), String>,
    ) -> Result<()> {
        if self.layers.is_empty() {
            return Ok(());
        }
        let names: Vec<(String, String)> = {
            let g = font.glyph(&self.glyph).ok_or_else(|| p.bad(format!("there is no glyph `{}`", self.glyph)))?;
            g.layers.iter().filter(|l| self.layers.contains(&l.id)).map(|l| (l.id.clone(), layer_name(font, l))).collect()
        };
        let mut g = font.glyph_mut(&self.glyph)?;
        let edited: Option<Vec<Sig>> = g.layers.iter().find(|l| l.id == self.source).map(|l| signature(l.parts(self.side)));
        for (id, name) in names {
            let Some(layer) = g.layers.iter_mut().find(|l| l.id == id) else { continue };
            f(&id, &mut layer.parts_mut(self.side)).map_err(|m| p.bad(format!("matching layer {name}: {m}")))?;
            if Some(signature(layer.parts(self.side))) != edited {
                return Err(p.bad(format!("the edit would leave layer {name} incompatible with the edited one; edit the layers one at a time")));
            }
        }
        Ok(())
    }

    /// The matched and skipped layers, added to the reply.
    pub(in crate::cmd) fn report(plans: Vec<Plan>, reply: &mut Value) {
        let Some(out) = reply.as_object_mut() else { return };
        let matched: Vec<Value> = plans.iter().flat_map(|plan| plan.layers.iter().map(move |id| json!({"glyph": plan.glyph, "layer": id}))).collect();
        out.insert("matched".into(), Value::Array(matched));
        out.insert("skipped".into(), Value::Array(plans.into_iter().flat_map(|plan| plan.skipped).collect()));
    }
}

/// What `e` says, without the name of the command.
fn message(e: EngineError) -> String {
    match e {
        EngineError::BadParams { message, .. } => message,
        other => other.to_string(),
    }
}

fn layer_name(font: &Font, layer: &Layer) -> String {
    let master = font.masters.iter().find(|m| m.id == layer.master).map_or(layer.master.as_str(), |m| m.name.as_str());
    match special_label(font, layer) {
        Some(label) => format!("{master} {label}"),
        None => master.to_string(),
    }
}

/// The path at `shape` of a matched layer.
pub(super) fn path_at<'a>(parts: &'a mut LayerPartsMut, shape: usize) -> std::result::Result<&'a mut fontcraft_model::Path, String> {
    match parts.shapes.get_mut(shape) {
        Some(Shape::Path(path)) => Ok(path),
        _ => Err(format!("shape {shape} is not a path")),
    }
}
