//! Generate Dark Palette (M13.65): a palette for dark backgrounds derived from an existing one
//! ([`fontcraft_color::dark`]: OKLab lightness inverted, hue and alpha kept), appended to the
//! palettes, named and marked for dark backgrounds so that `CPAL` version 1 flags it.
//!
//! Rows: `color.generateDarkPalette`, `color.palette.multiplePalettes`.
//!
//! The `Color Palettes` value takes the version 4 form, the only one that records background
//! styles (`palette::store_dark`); the palette commands keep that form afterwards.

use std::sync::Arc;

use fontcraft_color::dark::dark_palette;
use fontcraft_color::palette::{self, COLOR_PALETTES_PARAM};
use fontcraft_model::ModelError;
use serde_json::{Value, json};

use super::palette::{current, index, owner};
use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "color.generateDarkPalette",
        "Generate Dark Palette",
        ["Font"],
        None,
        "{palette?: number (default: 0), master?: id} append a palette for dark backgrounds derived from palette `palette`: each colour's lightness inverted in OKLab, its hue and alpha kept (a colour that would fall outside sRGB loses chroma, not hue). The new palette is named after its source with \" Dark\" added (\"Dark\" when the source has no name) and marked for dark backgrounds, which CPAL version 1 writes as its palette type; the Color Palettes value takes the dictionary form that records this. With `master`, that master's own palettes (made from the font's when it has none). Refused when there are no palettes → {palette: the new palette's number, palettes: count, rows, name, colors: [\"#RRGGBBAA\"]}",
        has_doc,
        generate_dark_palette
    )]
}

fn generate_dark_palette(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("color.generateDarkPalette", raw)?;
    let font = s.font()?;
    let master = owner(&p, font)?;
    let (stored, palettes) = current(&p, font, master.as_deref())?;
    if palettes.is_empty() {
        return Err(p.bad("the font has no colour palettes to derive a dark palette from"));
    }
    let source = index(&p, "palette")?.unwrap_or(0);
    let colours = palettes.palette(source).ok_or_else(|| {
        p.bad(format!("there is no palette {source}; the palettes count from 0 to {}", palettes.palettes().len().saturating_sub(1)))
    })?;
    let dark = dark_palette(colours);
    let next = palettes.add_palette_with(dark.clone()).map_err(|e| p.bad(e.to_string()))?;
    let source_name = stored.map(palette::palette_info).and_then(|info| info.get(source).and_then(|i| i.name.clone()));
    let name = source_name.map_or_else(|| "Dark".to_string(), |n| format!("{n} Dark"));
    let value = palette::store_dark(stored, &next, &name);
    s.edit("Generate Dark Palette", |font| {
        match master.as_deref() {
            None => Arc::make_mut(&mut font.info).params.set(COLOR_PALETTES_PARAM, value),
            Some(id) => {
                let m = Arc::make_mut(&mut font.masters)
                    .iter_mut()
                    .find(|m| m.id == id)
                    .ok_or_else(|| EngineError::Model(ModelError::NoSuchMaster(id.to_string())))?;
                m.params.set(COLOR_PALETTES_PARAM, value);
            }
        }
        Ok(())
    })?;
    Ok(json!({
        "palette": next.palettes().len().saturating_sub(1),
        "palettes": next.palettes().len(),
        "rows": next.rows(),
        "name": name,
        "colors": dark.iter().map(fontcraft_color::Rgba::hex).collect::<Vec<_>>(),
    }))
}
