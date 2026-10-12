//! The `HVAR` and `VVAR` tables (M9.6).
//!
//! Rows: the `HVAR` and `VVAR` table rows, `compile.hvarStoreChoice`.
//!
//! Contract: the advance of each glyph (in glyph order) as a [`VarValue`]; the deltas go into an
//! item variation store, written with an advance-width mapping or as direct rows, whichever is
//! smaller, as fontc chooses. Side-bearing variations are left to `gvar`'s phantom points.
//!
//! The choice follows fontc (`fontbe` `hvar.rs`, `vvar.rs`, `metric_variations.rs`, Apache-2.0):
//! the store indexed by glyph id is built only when every glyph that varies has the same regions
//! (one variation model), and is kept when it is no larger than the deduplicated store plus its
//! index map. Regions enter the store in the order the advances first name them, so the caller
//! gives glyph 0 (`.notdef`) a zero delta for every region of the font's model when it has no
//! sources of its own, as fontc does to match fontTools' region order.

use std::collections::BTreeSet;

use write_fonts::dump_table;
use write_fonts::tables::hvar::Hvar;
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::tables::variations::{DeltaSetIndexMap, ItemVariationStore, RegionAxisCoordinates, VariationRegion};
use write_fonts::tables::vvar::Vvar;
use write_fonts::types::F2Dot14;

use super::{Region, VarValue};
use crate::CompileError;

/// The most glyphs a metrics variation table indexes.
pub const MAX_GLYPHS: usize = u16::MAX as usize + 1;
/// The most deltas of one value (one per region of its variation model).
pub const MAX_DELTAS: usize = fontcraft_geom::variation::MAX_MASTERS;

/// `region` as the store writes it, refused when it has the wrong number of axes or a coordinate
/// outside −1…1.
pub fn variation_region(region: &Region, axis_count: u16, table: &'static str) -> Result<VariationRegion, CompileError> {
    if region.0.len() != usize::from(axis_count) {
        return Err(CompileError::Table { table, message: format!("a region has {} axes; the font has {axis_count}", region.0.len()) });
    }
    let coord = |v: f64| {
        if v.is_finite() && (-1.0..=1.0).contains(&v) {
            Ok(F2Dot14::from_f32(v as f32))
        } else {
            Err(CompileError::Table { table, message: format!("a region coordinate {v} lies outside -1 to 1") })
        }
    };
    let axes = region
        .0
        .iter()
        .map(|a| Ok(RegionAxisCoordinates::new(coord(a.start)?, coord(a.peak)?, coord(a.end)?)))
        .collect::<Result<Vec<_>, CompileError>>()?;
    Ok(VariationRegion::new(axes))
}

/// The deltas of `values` as the store builder takes them.
pub(crate) fn store_deltas(values: &[&VarValue], axis_count: u16, table: &'static str) -> Result<Vec<Vec<(VariationRegion, i32)>>, CompileError> {
    values
        .iter()
        .map(|v| {
            if v.deltas.len() > MAX_DELTAS {
                return Err(CompileError::Table {
                    table,
                    message: format!("a value has {} deltas; at most {MAX_DELTAS} are written", v.deltas.len()),
                });
            }
            v.deltas.iter().map(|(r, d)| Ok((variation_region(r, axis_count, table)?, *d))).collect()
        })
        .collect()
}

fn size<T: write_fonts::FontWrite + write_fonts::validate::Validate>(table: &T, name: &'static str) -> Result<usize, CompileError> {
    dump_table(table).map(|b| b.len()).map_err(|e| CompileError::Table { table: name, message: e.to_string() })
}

/// The store of `advances` and, when it is the smaller choice, the glyph-to-delta-set map.
fn advance_store(
    axis_count: u16,
    advances: &[VarValue],
    table: &'static str,
) -> Result<(ItemVariationStore, Option<DeltaSetIndexMap>), CompileError> {
    if axis_count == 0 {
        return Err(CompileError::Table { table, message: "a variable font needs at least one axis".into() });
    }
    if advances.len() > MAX_GLYPHS {
        return Err(CompileError::Table { table, message: format!("{} glyphs are more than the table indexes", advances.len()) });
    }
    let refs: Vec<&VarValue> = advances.iter().collect();
    let deltas = store_deltas(&refs, axis_count, table)?;

    // One model: every glyph with deltas names the same regions.
    let region_sets: BTreeSet<Vec<VariationRegion>> = deltas
        .iter()
        .filter(|d| !d.is_empty())
        .map(|d| {
            let mut regions: Vec<VariationRegion> = d.iter().map(|(r, _)| r.clone()).collect();
            regions.sort();
            regions.dedup();
            regions
        })
        .collect();
    let direct = if region_sets.len() <= 1 {
        let mut builder = VariationStoreBuilder::new_with_implicit_indices(axis_count);
        for d in &deltas {
            builder.add_deltas(d.clone());
        }
        Some(builder.build().0)
    } else {
        None
    };

    let mut builder = VariationStoreBuilder::new(axis_count);
    let ids: Vec<_> = deltas.into_iter().map(|d| builder.add_deltas(d)).collect();
    let (store, remap) = builder.build();
    let indices = ids
        .into_iter()
        .map(|id| remap.get(id).ok_or_else(|| CompileError::Table { table, message: "a delta set was lost while building the store".into() }))
        .collect::<Result<Vec<_>, _>>()?;
    let map: DeltaSetIndexMap = indices.into_iter().collect();

    if let Some(direct) = direct
        && size(&direct, table)? <= size(&store, table)? + size(&map, table)?
    {
        return Ok((direct, None));
    }
    Ok((store, Some(map)))
}

/// The `HVAR` table of `advances` (one per glyph, in glyph order).
pub fn build_hvar(axis_count: u16, advances: &[VarValue]) -> Result<Hvar, CompileError> {
    let (store, map) = advance_store(axis_count, advances, "HVAR")?;
    Ok(Hvar::new(store, map, None, None))
}

/// The `VVAR` table of vertical `advances` (one per glyph, in glyph order).
pub fn build_vvar(axis_count: u16, advances: &[VarValue]) -> Result<Vvar, CompileError> {
    let (store, map) = advance_store(axis_count, advances, "VVAR")?;
    Ok(Vvar::new(store, map, None, None, None))
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::variation::RegionAxis;

    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn region(start: f64, peak: f64, end: f64) -> Region {
        Region(vec![RegionAxis { start, peak, end }])
    }

    fn up(d: i32) -> VarValue {
        VarValue { default: 500, deltas: vec![(region(0.0, 1.0, 1.0), d)] }
    }

    // Expected bytes: fontc 1.0.0 on its test source `WghtVar.glyphs` (seven glyphs, only `space`
    // changes width): the deduplicated store and its map are smaller than one row per glyph.
    #[test]
    fn hvar_keeps_the_mapped_store_when_smaller() {
        let advances = [up(0), up(0), up(0), up(0), up(0), up(0), up(400)];
        let hvar = build_hvar(1, &advances).unwrap();
        assert_eq!(
            hex(&dump_table(&hvar).unwrap()),
            "000100000000001400000036000000000000000000010000000c0001000000160001000100004000400000020001000100000000019000000003000100"
        );
    }

    // Expected bytes: fontc 1.0.0 on `WghtVar_Avar.glyphs` (masters 300, 400, 700; one of five
    // glyphs changes width): one row per glyph is the smaller choice.
    #[test]
    fn hvar_keeps_the_direct_store_when_smaller() {
        let both = |a: i32, b: i32| VarValue { default: 500, deltas: vec![(region(-1.0, -1.0, 0.0), a), (region(0.0, 1.0, 1.0), b)] };
        let advances = [both(0, 0), both(-50, 50), both(0, 0), both(0, 0), both(0, 0)];
        let hvar = build_hvar(1, &advances).unwrap();
        assert_eq!(
            hex(&dump_table(&hvar).unwrap()),
            "000100000000001400000000000000000000000000010000000c00010000001c00010002c000c0000000000040004000000500000002000000010000ce32000000000000"
        );
        assert!(build_vvar(1, &advances).unwrap().advance_height_mapping.is_none());
    }

    #[test]
    fn glyphs_with_their_own_regions_always_get_a_map() {
        let sparse = VarValue { default: 500, deltas: vec![(region(0.0, 0.5, 1.0), 10), (region(0.0, 1.0, 1.0), 20)] };
        let advances = [up(0), sparse, VarValue::default()];
        let hvar = build_hvar(1, &advances).unwrap();
        assert!(hvar.advance_width_mapping.is_some());
    }

    #[test]
    fn hvar_refuses_bad_regions() {
        assert!(build_hvar(0, &[up(1)]).is_err());
        assert!(build_hvar(2, &[up(1)]).is_err());
        let wide = VarValue { default: 0, deltas: vec![(region(0.0, 1.5, 2.0), 1)] };
        assert!(build_hvar(1, &[wide]).is_err());
        let nan = VarValue { default: 0, deltas: vec![(region(0.0, f64::NAN, 1.0), 1)] };
        assert!(build_hvar(1, &[nan]).is_err());
    }
}
