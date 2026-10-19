//! `BASE` variation data of an instance (M12.20b), as fontTools' `varLib.instancer`
//! (`_instantiateBASE`, with `MutatorMerger` on `BaseCoord`; MIT, Copyright (c) 2019 Google LLC
//! and the fontTools authors) handles it: every format 3 coordinate whose device is a variation
//! index adds its rounded delta; when the store keeps no region, the device goes, the
//! coordinate becomes format 1 and the table version 1.0.
//!
//! A format 3 coordinate with a hinting device (not a variation index) is left as it is.

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::base::{Axis, Base, BaseCoord, MinMax};
use write_fonts::tables::layout::DeviceOrVariationIndex;
use write_fonts::tables::variations::ItemVariationStore;
use write_fonts::{NullableOffsetMarker, OffsetMarker};

use super::ivs::StoreDeltas;
use super::ot_round;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

/// The offset of the `BASE` table's item variation store; `None` without one.
pub(super) fn store_offset(font: &SfntFont) -> Result<Option<usize>> {
    let Some(data) = font.get(*b"BASE") else { return Ok(None) };
    let base = read_fonts::tables::base::Base::read(FontData::new(data)).map_err(|e| bad(format!("the BASE table cannot be read: {e}")))?;
    Ok(base.item_var_store_offset().filter(|o| !o.is_null()).map(|o| o.offset().to_u32() as usize))
}

/// A full instance: the deltas of the `BASE` store at `coords` (normalised, one per `fvar` axis)
/// go into the coordinates, and the store goes.
pub(super) fn full(font: &mut SfntFont, coords: &[f64]) -> Result<()> {
    let Some(offset) = store_offset(font)? else { return Ok(()) };
    let data = font.get(*b"BASE").unwrap_or_default();
    let deltas = StoreDeltas::read(data, offset, coords)?;
    apply(font, &deltas, None)
}

/// Adds the rounded deltas to every coordinate with a variation-index device. With `store`
/// (a partial instance whose store keeps regions) the devices stay and the store is replaced;
/// without, the devices and the store go.
pub(super) fn apply(font: &mut SfntFont, deltas: &StoreDeltas, store: Option<ItemVariationStore>) -> Result<()> {
    let Some(data) = font.get(*b"BASE") else { return Ok(()) };
    crate::layout_budget::check_base(data)?;
    let read = read_fonts::tables::base::Base::read(FontData::new(data)).map_err(|e| bad(format!("the BASE table cannot be read: {e}")))?;
    let mut base: Base = read.to_owned_table();
    let keep = store.is_some();
    for axis in [base.horiz_axis.as_mut(), base.vert_axis.as_mut()].into_iter().flatten() {
        axis_coords(axis, deltas, keep);
    }
    base.item_var_store = NullableOffsetMarker::new(store);
    let bytes = write_fonts::dump_table(&base).map_err(|e| bad(format!("the BASE table could not be written again: {e}")))?;
    font.insert(*b"BASE", bytes);
    Ok(())
}

fn axis_coords(axis: &mut Axis, deltas: &StoreDeltas, keep: bool) {
    for record in axis.base_script_list.base_script_records.iter_mut() {
        let script = &mut *record.base_script;
        if let Some(values) = script.base_values.as_mut() {
            for coord in values.base_coords.iter_mut() {
                coord_delta(coord, deltas, keep);
            }
        }
        if let Some(mm) = script.default_min_max.as_mut() {
            min_max(mm, deltas, keep);
        }
        for lang in script.base_lang_sys_records.iter_mut() {
            min_max(&mut lang.min_max, deltas, keep);
        }
    }
}

fn min_max(mm: &mut MinMax, deltas: &StoreDeltas, keep: bool) {
    for c in [mm.min_coord.as_mut(), mm.max_coord.as_mut()].into_iter().flatten() {
        coord_delta_in(c, deltas, keep);
    }
    for rec in mm.feat_min_max_records.iter_mut() {
        for c in [rec.min_coord.as_mut(), rec.max_coord.as_mut()].into_iter().flatten() {
            coord_delta_in(c, deltas, keep);
        }
    }
}

fn coord_delta(coord: &mut OffsetMarker<BaseCoord>, deltas: &StoreDeltas, keep: bool) {
    coord_delta_in(coord, deltas, keep);
}

fn coord_delta_in(coord: &mut BaseCoord, deltas: &StoreDeltas, keep: bool) {
    let BaseCoord::Format3(t) = coord else { return };
    let Some(DeviceOrVariationIndex::VariationIndex(v)) = t.device.as_ref() else { return };
    let d = ot_round(deltas.get(v.delta_set_outer_index, v.delta_set_inner_index));
    let value = (f64::from(t.coordinate) + d).clamp(-32768.0, 32767.0) as i16;
    if keep {
        t.coordinate = value;
    } else {
        *coord = BaseCoord::format_1(value);
    }
}
