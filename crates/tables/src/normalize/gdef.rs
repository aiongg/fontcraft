//! GDEF: the ligature caret list, one ligature per line block, carets in order.
//!
//! Ported from fontc's `otl-normalizer` (MIT OR Apache-2.0).

use std::collections::BTreeSet;

use read_fonts::tables::gdef::{CaretValue, Gdef};
use read_fonts::tables::layout::DeviceOrVariationIndex;
use read_fonts::types::F2Dot14;

use super::deltas::{DeltaComputer, Resolver, resolve};
use super::names::NameMap;
use super::{Budget, Out, read_err};
use crate::Result;

/// The variation subtables the caret values refer to; only their regions give master locations.
fn caret_subtables(table: &Gdef<'_>, budget: &mut Budget) -> Result<BTreeSet<u16>> {
    let err = read_err("GDEF's ligature caret list");
    let mut outers = BTreeSet::new();
    let Some(carets) = table.lig_caret_list() else { return Ok(outers) };
    let carets = carets.map_err(&err)?;
    for lig_glyph in carets.lig_glyphs().iter() {
        let lig_glyph = lig_glyph.map_err(&err)?;
        for caret in lig_glyph.caret_values().iter() {
            budget.charge(1)?;
            if let CaretValue::Format3(caret) = caret.map_err(&err)?
                && let DeviceOrVariationIndex::VariationIndex(index) = caret.device().map_err(&err)?
            {
                outers.insert(index.delta_set_outer_index());
            }
        }
    }
    Ok(outers)
}

/// Prints the ligature carets. `coords`: resolve variable carets at this location.
pub(super) fn print(out: &mut Out, table: &Gdef<'_>, names: &NameMap, coords: Option<&[F2Dot14]>, budget: &mut Budget) -> Result<()> {
    let err = read_err("GDEF's ligature caret list");
    let resolver = match table.item_var_store() {
        None => Resolver::None,
        Some(ivs) => {
            let ivs = ivs.map_err(read_err("GDEF's item variation store"))?;
            match coords {
                Some(coords) => Resolver::At(ivs, coords.to_vec()),
                None => {
                    let used = caret_subtables(table, budget)?;
                    Resolver::Masters(DeltaComputer::new(ivs, Some(&used), budget)?)
                }
            }
        }
    };
    let Some(carets) = table.lig_caret_list() else { return Ok(()) };
    let carets = carets.map_err(&err)?;
    let coverage = carets.coverage().map_err(&err)?;
    for (gid, lig_glyph) in coverage.iter().zip(carets.lig_glyphs().iter()) {
        let lig_glyph = lig_glyph.map_err(&err)?;
        out.put(format_args!("{}\n", names.get(gid)))?;
        for (i, caret) in lig_glyph.caret_values().iter().enumerate() {
            budget.charge(1)?;
            match caret.map_err(&err)? {
                CaretValue::Format1(caret) => out.put(format_args!("  {i}: coord {}\n", caret.coordinate()))?,
                CaretValue::Format2(caret) => out.put(format_args!("  {i}: idx {}\n", caret.caret_value_point_index()))?,
                CaretValue::Format3(caret) => {
                    let value = resolve(caret.coordinate(), caret.device().map_err(&err)?, &resolver, budget)?;
                    out.put(format_args!("  {i}: coord {value}\n"))?;
                }
            }
        }
    }
    Ok(())
}
