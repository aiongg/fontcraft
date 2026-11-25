//! Glyph → Transform Metrics: one new advance width, or new sidebearings, on many layers at once
//! (`docs/spec/11-filters.md` § Transform Metrics).
//!
//! The command takes one `glyph` or a `glyphs` list (Font View). With `glyphs`, a locked glyph,
//! a glyph without the named layer and, in sidebearing mode, a layer without an outline are
//! skipped and listed in `skipped`; with one `glyph`, a locked glyph or a missing layer is an
//! error.

use std::collections::HashMap;

use fontcraft_geom::round_to_grid;
use fontcraft_model::{Font, Layer};
use fontcraft_spacing::{Field, SIDEBEARING_RANGE, WIDTH_RANGE, metric};
use serde_json::{Value, json};

use super::common::{each_layer, grid, scope};
use super::shared::shift_layer;
use super::*;
use crate::params::{Params, TargetKind, Targets};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.transformMetrics",
            "Transform Metrics",
            ["Glyph"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids] (default: the first master's layer), width?: number (0 to 65535), onBothSides?: bool = false, lsb?: number, rsb?: number, relative?: bool = false} with `width`, give every layer that advance width: the outline keeps its coordinates, or with `onBothSides` it moves by half the change, so both sidebearings share it. Without `width`, set the sidebearings given (`lsb`, `rsb` or both, from -32768 to 32767): `relative` adds them to the current ones, otherwise they replace them. `width` does not combine with `lsb` or `rsb`. The outline, components, anchors, guides, annotations, image and background move together; shifts and new widths are rounded to the grid. A layer without an outline has no sidebearings and is skipped in sidebearing mode → {glyphs: [names], skipped: [{glyph, layer?, reason}], mode: \"width\" | \"sidebearings\"}",
            has_doc,
            transform_metrics
        )
        .targets(&[TargetKind::Glyphs, TargetKind::Layer]),
    ]
}

/// A new value of a metric, or an amount added to the one there is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shift {
    pub value: f64,
    pub relative: bool,
}

impl Shift {
    /// The metric after it, from `current`.
    fn after(self, current: f64) -> f64 {
        if self.relative { current + self.value } else { self.value }
    }
}

/// What the command sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Mode {
    Width { width: Shift, both: bool },
    Sides { lsb: Option<Shift>, rsb: Option<Shift> },
}

/// What happens to one layer: a horizontal shift of everything in it, then a new width.
#[derive(Clone, Copy)]
struct Change {
    dx: f64,
    width: f64,
}

fn read_mode(p: &Params) -> Result<Mode> {
    let (lsb, rsb) = (p.opt_f64("lsb")?, p.opt_f64("rsb")?);
    let both = p.bool_or("onBothSides", false)?;
    let relative = p.bool_or("relative", false)?;
    if let Some(width) = p.opt_f64("width")? {
        if lsb.is_some() || rsb.is_some() {
            return Err(p.bad("`width` does not combine with `lsb` or `rsb`: give `width` to set the width, or leave it out to set sidebearings"));
        }
        if !WIDTH_RANGE.contains(&width) {
            return Err(p.bad(format!("`width` must be between 0 and 65535, got {width}")));
        }
        return Ok(Mode::Width { width: Shift { value: width, relative: false }, both });
    }
    if lsb.is_none() && rsb.is_none() {
        return Err(p.bad("give `width`, or `lsb` and/or `rsb`"));
    }
    for (key, value) in [("lsb", lsb), ("rsb", rsb)] {
        if let Some(v) = value
            && !SIDEBEARING_RANGE.contains(&v)
        {
            return Err(p.bad(format!("`{key}` must be between -32768 and 32767, got {v}")));
        }
    }
    let shift = |value: f64| Shift { value, relative };
    Ok(Mode::Sides { lsb: lsb.map(shift), rsb: rsb.map(shift) })
}

/// The change for one layer, or why it is left alone.
fn plan(p: &Params, font: &Font, glyph: &str, layer: &Layer, mode: Mode, step: f64) -> Result<std::result::Result<Change, String>> {
    let change = match mode {
        Mode::Width { width, both } => {
            let width = width.after(layer.width);
            let dx = if both { round_to_grid((width - layer.width) / 2.0, step) } else { 0.0 };
            Change { dx, width }
        }
        Mode::Sides { lsb, rsb } => {
            let measure = |field| metric(font, layer, field);
            let (left, right) = match (measure(Field::Left), measure(Field::Right)) {
                (Ok(Some(l)), Ok(Some(r))) => (l, r),
                (Err(e), _) | (_, Err(e)) => return Ok(Err(format!("the sidebearings cannot be measured: {e}"))),
                _ => return Ok(Err("the layer has no outline, so it has no sidebearings".into())),
            };
            let delta = |shift: Option<Shift>, current: f64| shift.map_or(0.0, |s| s.after(current) - current);
            let dx = round_to_grid(delta(lsb, left), step);
            let width = round_to_grid(layer.width + dx + delta(rsb, right), step);
            Change { dx, width }
        }
    };
    if !(change.dx.is_finite() && WIDTH_RANGE.contains(&change.width)) {
        return Err(p.bad(format!("glyph `{glyph}`, layer `{}`: the new width would be {}, outside 0 to 65535", layer.id, change.width)));
    }
    Ok(Ok(change))
}

fn change_layer(layer: &mut Layer, change: Change) -> std::result::Result<(), String> {
    // The shift moves the guides with the contents; they then follow the rest of the change.
    let before = layer.width + change.dx;
    shift_layer(layer, change.dx)?;
    layer.width = change.width;
    fontcraft_model::guide_follow::follow_width(layer, before);
    Ok(())
}

/// Changes the metrics of the layer `t` names as `mode` says. A layer that has no sidebearings is
/// left alone when they are to change.
pub(super) fn change_target(p: &Params, font: &mut Font, t: &Targets, mode: Mode, step: f64) -> Result<()> {
    let change = {
        let r = t.resolve(font)?;
        plan(p, font, &t.glyph, r.layer, mode, step)?
    };
    match change {
        Ok(change) => t.edit_layer(font, |l| change_layer(l, change).map_err(|e| p.bad(format!("glyph `{}`: {e}", t.glyph)))),
        Err(_) => Ok(()),
    }
}

fn transform_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.transformMetrics", v)?;
    let mode = read_mode(&p)?;
    let font = s.font()?;
    let scope = scope(&p, font)?;
    let step = grid(font);
    let mut plans: HashMap<(String, String), std::result::Result<Change, String>> = HashMap::new();
    for name in &scope.glyphs {
        let Some(glyph) = font.glyph(name) else { continue };
        for layer in glyph.layers.iter().filter(|l| scope.layers.contains(&l.id)) {
            plans.insert((name.clone(), layer.id.clone()), plan(&p, font, name, layer, mode, step)?);
        }
    }
    let out = s.edit("Transform Metrics", |font| {
        each_layer(font, &scope, "glyph.transformMetrics", |glyph, layer| match plans.get(&(glyph.to_string(), layer.id.clone())) {
            Some(Ok(change)) => Ok(change_layer(layer, *change).err()),
            Some(Err(reason)) => Ok(Some(reason.clone())),
            None => Ok(Some("the layer was not measured".into())),
        })
    })?;
    let mut v = out.to_json();
    if let Some(o) = v.as_object_mut() {
        o.insert("mode".into(), json!(if matches!(mode, Mode::Width { .. }) { "width" } else { "sidebearings" }));
    }
    Ok(v)
}
