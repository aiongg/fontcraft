//! `COLR` (M11.6 version 0, M11.7 version 1, M11.19 variable).
//!
//! Rows: `COLR`, `color.export.colrFromPaletteLayers`, `color.export.fullColorToColrV1`,
//! `color.export.colrVersionChoice`, `color.export.strokeInColr`, `compile.colrClipBoxes`,
//! `compile.colrGradientGeometry`, `color.export.variableColr`.
//!
//! Contract (`docs/plans/M11.md` § 3): base glyph and layer records for version 0; for version 1
//! `BaseGlyphList`, `LayerList` and `ClipList`. Layer glyphs are those
//! [`super::layer_glyphs`] appended, found by name in the [`GlyphOrder`]. `*` (the text colour)
//! is palette index 0xFFFF. Colours that are not palette references become new rows, added once
//! to every palette, with their alpha in CPAL only.

use std::collections::HashMap;

use fontcraft_color::export::{ColrPlan, ColrVersion};
use fontcraft_color::paint::{ColorRef, Paint, shape_refs};
use fontcraft_color::palette::FOREGROUND_INDEX;
use fontcraft_color::{MAX_PALETTE_ROWS, Rgba};
use write_fonts::tables::colr::{BaseGlyph, Colr, Layer};
use write_fonts::types::GlyphId16;

use super::GlyphOrder;
use crate::CompileError;
use crate::order::MadeFrom;

/// A built `COLR` table and the colours it added to the palettes (rows after `palette_rows`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColrOutput {
    pub table: Vec<u8>,
    pub added: Vec<Rgba>,
}

/// The table for `plan`, numbering new colours from `palette_rows`.
///
/// Version 0: base glyph records sorted by glyph id, each glyph's layer records in paint order,
/// the bottom layer first. A base glyph that is not in the font is left out.
pub fn build(plan: &ColrPlan, order: &GlyphOrder, palette_rows: usize) -> Result<ColrOutput, CompileError> {
    if plan.version == ColrVersion::V1 {
        // M11.7: version 1.
        return Err(CompileError::NotYetImplemented("COLR version 1"));
    }
    // The layer glyphs of each base glyph, in the order `super::layer_glyphs` appended them,
    // which is the order of the paint's shape references.
    let mut layer_gids: HashMap<&str, Vec<u16>> = HashMap::new();
    for m in order.made {
        if let MadeFrom::ColorLayer { glyph, .. } = &m.source
            && let Some(gid) = order.gid(&m.name)
        {
            layer_gids.entry(glyph.as_str()).or_default().push(gid);
        }
    }
    let mut colors = AddedColors { rows: palette_rows, added: Vec::new() };
    let mut records: Vec<(u16, Vec<Layer>)> = Vec::with_capacity(plan.glyphs.len());
    for g in &plan.glyphs {
        let Some(base) = order.gid(&g.glyph) else { continue };
        let solids = solid_layers(&g.paint).ok_or_else(|| table_error(format!("glyph `{}` needs COLR version 1", g.glyph)))?;
        let gids = layer_gids.get(g.glyph.as_str()).map_or(&[][..], Vec::as_slice);
        if gids.len() != solids.len() || shape_refs(&g.paint).len() != solids.len() {
            return Err(table_error(format!("glyph `{}` has {} colour layers but {} layer glyphs", g.glyph, solids.len(), gids.len())));
        }
        let layers = gids
            .iter()
            .zip(solids)
            .map(|(&gid, color)| Ok(Layer::new(GlyphId16::new(gid), colors.index(color)?)))
            .collect::<Result<Vec<_>, CompileError>>()?;
        records.push((base, layers));
    }
    records.sort_by_key(|(gid, _)| *gid);
    records.dedup_by_key(|(gid, _)| *gid);
    let too_many = || table_error("more than 65,535 layer records".to_string());
    let mut base_records = Vec::with_capacity(records.len());
    let mut layer_records = Vec::new();
    for (gid, layers) in records {
        let first = u16::try_from(layer_records.len()).map_err(|_| too_many())?;
        let count = u16::try_from(layers.len()).map_err(|_| too_many())?;
        base_records.push(BaseGlyph::new(GlyphId16::new(gid), first, count));
        layer_records.extend(layers);
    }
    let num_layers = u16::try_from(layer_records.len()).map_err(|_| too_many())?;
    // At most one record per glyph id.
    let num_base = u16::try_from(base_records.len()).map_err(|_| too_many())?;
    let colr = Colr::new(num_base, Some(base_records), Some(layer_records), num_layers);
    let table = write_fonts::dump_table(&colr).map_err(|e| table_error(e.to_string()))?;
    Ok(ColrOutput { table, added: colors.added })
}

fn table_error(message: String) -> CompileError {
    CompileError::Table { table: "COLR", message }
}

/// The colour of each layer of a stack of solid layers, bottom first; `None` for any other
/// paint.
fn solid_layers(paint: &Paint) -> Option<Vec<ColorRef>> {
    let solid = |p: &Paint| match p {
        Paint::Glyph { paint, .. } => match paint.as_ref() {
            Paint::Solid(c) => Some(*c),
            _ => None,
        },
        _ => None,
    };
    match paint {
        Paint::Layers(list) => list.iter().map(solid).collect(),
        other => solid(other).map(|c| vec![c]),
    }
}

/// Colours that are not palette references, numbered after the palette rows in the order COLR
/// first uses them, once each.
struct AddedColors {
    rows: usize,
    added: Vec<Rgba>,
}

impl AddedColors {
    /// The palette index of `color`. Version 0 has no alpha of its own: a palette reference's
    /// alpha is dropped (palette layers always have 1), and a colour's alpha goes into CPAL.
    fn index(&mut self, color: ColorRef) -> Result<u16, CompileError> {
        let rgba = match color {
            ColorRef::Palette { index, .. } => return Ok(index),
            ColorRef::Foreground { .. } => return Ok(FOREGROUND_INDEX),
            ColorRef::Rgba(c) => to_rgba(c),
        };
        let at = match self.added.iter().position(|&a| a == rgba) {
            Some(at) => at,
            None => {
                self.added.push(rgba);
                self.added.len().saturating_sub(1)
            }
        };
        self.rows
            .checked_add(at)
            .filter(|&i| i < MAX_PALETTE_ROWS)
            .and_then(|i| u16::try_from(i).ok())
            .ok_or_else(|| table_error(format!("the palettes and the colours the layers add are more than {MAX_PALETTE_ROWS} colours")))
    }
}

/// Components from 0 to 1 as CPAL's bytes; a component that is not a number is 0.
fn to_rgba(c: [f64; 4]) -> Rgba {
    // Clamped to 0–255 and rounded, so the conversion is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = |v: f64| if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
    let [r, g, b, a] = c;
    Rgba::new(byte(r), byte(g), byte(b), byte(a))
}
