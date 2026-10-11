//! The numbers of the text-mode measurement line: `measure.sidebearingsAtHeight` measures each
//! glyph's sidebearings on one horizontal line (`fontcraft_spacing::sidebearings_at_height`), so
//! the shell's line and this query print the same values.

use fontcraft_model::MAX_COORDINATE;
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "measure.sidebearingsAtHeight",
            "Sidebearings at Height",
            [],
            None,
            "{glyphs: [names], master?: id (default: the first master), y: number (from -32768 to 32767)} for each glyph's layer of the master: `left`, the distance from the left edge of the advance to the first point where the horizontal line at height `y` meets the outline (components included), and `right`, from the last such point to the right edge. In a master with an italic angle the points are moved along the angle to the height sidebearings are measured at. `left` and `right` are null where the outline does not reach `y`; `layer` is null for a glyph without a layer in the master → {master, y, glyphs: [{glyph, layer, width, left, right}]}",
            has_doc,
            sidebearings_at_height
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

fn sidebearings_at_height(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("measure.sidebearingsAtHeight", v)?;
    let y = p.f64("y")?;
    if y.abs() > MAX_COORDINATE {
        return Err(p.bad(format!("`y` must lie within ±{MAX_COORDINATE}")));
    }
    let font = s.font()?;
    let glyphs = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    let master = match p.opt_str("master")? {
        Some(id) => font.masters.iter().find(|m| m.id == id).ok_or_else(|| p.bad(format!("there is no master `{id}`")))?,
        None => font.masters.first().ok_or_else(|| p.bad("the font has no master"))?,
    };
    let mut out = Vec::with_capacity(glyphs.len());
    for name in &glyphs {
        let Some(layer) = font.glyph(name).and_then(|g| g.layer(&master.id)) else {
            out.push(json!({"glyph": name, "layer": null, "width": null, "left": null, "right": null}));
            continue;
        };
        let sides = fontcraft_spacing::sidebearings_at_height(font, layer, y).map_err(|e| p.bad(format!("`{name}`: {e}")))?;
        let (left, right) = match sides {
            Some((l, r)) if l.is_finite() && r.is_finite() => (json!(l), json!(r)),
            _ => (Value::Null, Value::Null),
        };
        out.push(json!({"glyph": name, "layer": layer.id, "width": layer.width, "left": left, "right": right}));
    }
    Ok(json!({"master": master.id, "y": y, "glyphs": out}))
}
