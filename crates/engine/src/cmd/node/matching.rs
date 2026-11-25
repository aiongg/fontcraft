//! `matchLayers` of Insert Node, Delete, Make Node First and Reverse Contours (M9.35, row
//! `edit.toggleMatchWhenEditing`): the same edit, by the same addresses, on the other layers of the
//! glyph that interpolate with the edited one, in the edit's undo step.
//!
//! - A layer takes part when it is a master, intermediate or alternate layer and its foreground has
//!   the edited layer's shapes and node types ([`super::signature`]). A backup layer and the
//!   background match nothing.
//! - A master or intermediate layer that does not match is left alone and named in the result. An
//!   alternate layer may be drawn differently, so it is left alone unnamed.
//! - After the edit every matched layer must have the edited layer's shapes and node types again;
//!   otherwise, as when one layer refuses the edit, the whole edit is refused.

use fontcraft_interp::labels::layer_title;
use fontcraft_model::{Font, Layer, LayerPartsMut, LayerSide, Shape};
use serde_json::{Value, json};

use super::{Sig, signature};
use crate::Result;
use crate::params::{Params, Targets};

/// What `matchLayers` edits beside the layer the targets name.
#[derive(Default)]
pub(super) struct Plan {
    glyph: String,
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
    /// The layers that match the one `t` names, before the edit; `None` without `matchLayers`.
    pub(super) fn of(font: &Font, p: &Params, t: &Targets) -> Result<Option<Plan>> {
        if !p.bool_or("matchLayers", false)? {
            return Ok(None);
        }
        let r = t.resolve(font)?;
        let mut plan = Plan { glyph: r.glyph.name.clone(), source: r.layer.id.clone(), ..Plan::default() };
        if t.side != LayerSide::Foreground || !is_source(r.layer) {
            return Ok(Some(plan));
        }
        let sig = signature(r.parts);
        for layer in r.glyph.layers.iter().filter(|l| l.id != plan.source && is_source(l)) {
            if signature(layer.parts(LayerSide::Foreground)) == sig {
                plan.layers.push(layer.id.clone());
            } else if layer.info.attrs.axis_rules.is_empty() {
                let name = layer_title(font, layer);
                plan.skipped.push(json!({
                    "glyph": plan.glyph,
                    "layer": layer.id,
                    "reason": format!("{name}: its shapes or node types differ from the edited layer's"),
                }));
            }
        }
        Ok(Some(plan))
    }

    /// Makes the edit `f` on every matched layer, after it was made on the edited one.
    pub(super) fn apply(&self, font: &mut Font, p: &Params, mut f: impl FnMut(&mut LayerPartsMut) -> std::result::Result<(), String>) -> Result<()> {
        if self.layers.is_empty() {
            return Ok(());
        }
        let names: Vec<(String, String)> = {
            let g = font.glyph(&self.glyph).ok_or_else(|| p.bad(format!("there is no glyph `{}`", self.glyph)))?;
            g.layers.iter().filter(|l| self.layers.contains(&l.id)).map(|l| (l.id.clone(), layer_title(font, l))).collect()
        };
        let mut g = font.glyph_mut(&self.glyph)?;
        let edited: Option<Vec<Sig>> = g.layers.iter().find(|l| l.id == self.source).map(|l| signature(l.parts(LayerSide::Foreground)));
        for (id, name) in names {
            let Some(layer) = g.layers.iter_mut().find(|l| l.id == id) else { continue };
            f(&mut layer.parts_mut(LayerSide::Foreground)).map_err(|m| p.bad(format!("matching layer {name}: {m}")))?;
            if Some(signature(layer.parts(LayerSide::Foreground))) != edited {
                return Err(p.bad(format!("the edit would leave layer {name} incompatible with the edited one; edit without `matchLayers`")));
            }
        }
        Ok(())
    }

    /// The matched and skipped layers, added to the reply.
    pub(super) fn report(plans: Vec<Plan>, reply: &mut Value) {
        let Some(out) = reply.as_object_mut() else { return };
        let matched: Vec<Value> = plans.iter().flat_map(|plan| plan.layers.iter().map(move |id| json!({"glyph": plan.glyph, "layer": id}))).collect();
        out.insert("matched".into(), Value::Array(matched));
        out.insert("skipped".into(), Value::Array(plans.into_iter().flat_map(|plan| plan.skipped).collect()));
    }
}

/// The path at `shape` of a matched layer.
pub(super) fn path_at<'a>(parts: &'a mut LayerPartsMut, shape: usize) -> std::result::Result<&'a mut fontcraft_model::Path, String> {
    match parts.shapes.get_mut(shape) {
        Some(Shape::Path(path)) => Ok(path),
        _ => Err(format!("shape {shape} is not a path")),
    }
}
