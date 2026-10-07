//! Spacing: advance widths (sidebearings, metrics keys and kerning join in M5).

use fontcraft_model::Layer;
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "spacing.setWidth",
        "Set Width",
        [],
        None,
        "{glyphs: [names], layer?: master id (default: the first master), value: number} set the advance width of each glyph's layer, from 0 to 65535 (metrics keys as text: M5) → {glyphs: [names], layer, width}",
        has_doc,
        set_width
    )]
}

fn set_width(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("spacing.setWidth", p)?;
    let names: Vec<String> = p.strings("glyphs")?.into_iter().map(str::to_string).collect();
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    if p.get("value").is_some_and(Value::is_string) {
        return Err(EngineError::Unsupported("metrics keys (ROADMAP.md, M5); give `value` as a number".into()));
    }
    let width = p.f64("value")?;
    if !(0.0..=65_535.0).contains(&width) {
        return Err(p.bad(format!("`value` must be between 0 and 65535, got {width}")));
    }
    let layer = match p.opt_str("layer")? {
        Some(id) => s.font()?.master(id)?.id.clone(),
        None => s.font()?.default_master()?.id.clone(),
    };
    s.edit("Set Width", |font| {
        for name in &names {
            let g = font.glyph_mut(name)?;
            match g.layer_mut(&layer) {
                Some(l) => l.width = width,
                None => g.layers.push(Layer::new(&layer, width)),
            }
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": names, "layer": layer, "width": width }))
}
