//! `GPOS` without hinting leftovers (M12.27), as pyftsubset `--no-hinting` prunes it: device
//! tables (delta formats 1 to 3) leave value records and anchors, contour-point anchors
//! (format 2) become plain anchors, and a format 3 anchor left without device tables becomes a
//! plain anchor. Variation indexes stay. Each value record of a subtable then takes the format
//! the remaining fields of the subtable need.

use read_fonts::{FontData, FontRead};
use write_fonts::NullableOffsetMarker;
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gpos::{
    AnchorFormat1, AnchorTable, CursivePosFormat1, ExtensionSubtable, Gpos, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1, PairPos,
    PositionLookup, SinglePos, ValueFormat, ValueRecord,
};
use write_fonts::tables::layout::DeviceOrVariationIndex;

use crate::{Result, TablesError};

/// The table without hinting leftovers; `None` when it has none.
pub(super) fn prune(data: &[u8]) -> Result<Option<Vec<u8>>> {
    crate::layout_budget::check_gpos(data)?;
    let read = read_fonts::tables::gpos::Gpos::read(FontData::new(data)).map_err(|e| TablesError::Invalid(format!("the GPOS table: {e}")))?;
    let mut gpos: Gpos = read.to_owned_table();
    let mut changed = false;
    for lookup in gpos.lookup_list.lookups.iter_mut() {
        changed |= prune_lookup(lookup);
    }
    if !changed {
        return Ok(None);
    }
    write_fonts::dump_table(&gpos).map(Some).map_err(|e| TablesError::Invalid(format!("the GPOS table could not be written again: {e}")))
}

fn prune_lookup(lookup: &mut PositionLookup) -> bool {
    let mut changed = false;
    match lookup {
        PositionLookup::Single(l) => l.subtables.iter_mut().for_each(|s| changed |= single(s)),
        PositionLookup::Pair(l) => l.subtables.iter_mut().for_each(|s| changed |= pair(s)),
        PositionLookup::Cursive(l) => l.subtables.iter_mut().for_each(|s| changed |= cursive(s)),
        PositionLookup::MarkToBase(l) => l.subtables.iter_mut().for_each(|s| changed |= mark_base(s)),
        PositionLookup::MarkToLig(l) => l.subtables.iter_mut().for_each(|s| changed |= mark_lig(s)),
        PositionLookup::MarkToMark(l) => l.subtables.iter_mut().for_each(|s| changed |= mark_mark(s)),
        PositionLookup::Contextual(_) | PositionLookup::ChainContextual(_) => {}
        PositionLookup::Extension(l) => {
            for ext in l.subtables.iter_mut() {
                changed |= match &mut **ext {
                    ExtensionSubtable::Single(e) => single(&mut e.extension),
                    ExtensionSubtable::Pair(e) => pair(&mut e.extension),
                    ExtensionSubtable::Cursive(e) => cursive(&mut e.extension),
                    ExtensionSubtable::MarkToBase(e) => mark_base(&mut e.extension),
                    ExtensionSubtable::MarkToLig(e) => mark_lig(&mut e.extension),
                    ExtensionSubtable::MarkToMark(e) => mark_mark(&mut e.extension),
                    ExtensionSubtable::Contextual(_) | ExtensionSubtable::ChainContextual(_) => false,
                };
            }
        }
    }
    changed
}

/// A device table that is a hint (not a variation index).
fn is_hinting(device: &NullableOffsetMarker<DeviceOrVariationIndex>) -> bool {
    matches!(device.as_ref(), Some(DeviceOrVariationIndex::Device(_)))
}

/// Drops the hinting device tables of a record; true when it had any.
fn prune_record(record: &mut ValueRecord) -> bool {
    let mut changed = false;
    for device in [&mut record.x_placement_device, &mut record.y_placement_device, &mut record.x_advance_device, &mut record.y_advance_device] {
        if is_hinting(device) {
            *device = NullableOffsetMarker::new(None);
            changed = true;
        }
    }
    changed
}

/// The fields a record holds.
fn fields(record: &ValueRecord) -> ValueFormat {
    let mut f = ValueFormat::empty();
    let pairs = [
        (record.x_placement.is_some(), ValueFormat::X_PLACEMENT),
        (record.y_placement.is_some(), ValueFormat::Y_PLACEMENT),
        (record.x_advance.is_some(), ValueFormat::X_ADVANCE),
        (record.y_advance.is_some(), ValueFormat::Y_ADVANCE),
        (record.x_placement_device.is_some(), ValueFormat::X_PLACEMENT_DEVICE),
        (record.y_placement_device.is_some(), ValueFormat::Y_PLACEMENT_DEVICE),
        (record.x_advance_device.is_some(), ValueFormat::X_ADVANCE_DEVICE),
        (record.y_advance_device.is_some(), ValueFormat::Y_ADVANCE_DEVICE),
    ];
    for (present, flag) in pairs {
        if present {
            f |= flag;
        }
    }
    f
}

/// Prunes every record and gives all of them the union of their fields, so the subtable's
/// records stay the same size.
fn prune_records<'r>(records: impl Iterator<Item = &'r mut ValueRecord>) -> bool {
    let mut records: Vec<&mut ValueRecord> = records.collect();
    let mut changed = false;
    for r in records.iter_mut() {
        changed |= prune_record(r);
    }
    if changed {
        let format = records.iter().fold(ValueFormat::empty(), |f, r| f | fields(r));
        for r in records.iter_mut() {
            r.set_explicit_value_format(format);
        }
    }
    changed
}

fn single(s: &mut SinglePos) -> bool {
    match s {
        SinglePos::Format1(t) => prune_records(std::iter::once(&mut t.value_record)),
        SinglePos::Format2(t) => prune_records(t.value_records.iter_mut()),
    }
}

fn pair(p: &mut PairPos) -> bool {
    match p {
        PairPos::Format1(t) => {
            let firsts = t.pair_sets.iter_mut().flat_map(|set| set.pair_value_records.iter_mut().map(|r| &mut r.value_record1));
            let changed1 = prune_records(firsts);
            let seconds = t.pair_sets.iter_mut().flat_map(|set| set.pair_value_records.iter_mut().map(|r| &mut r.value_record2));
            changed1 | prune_records(seconds)
        }
        PairPos::Format2(t) => {
            let firsts = t.class1_records.iter_mut().flat_map(|c1| c1.class2_records.iter_mut().map(|r| &mut r.value_record1));
            let changed1 = prune_records(firsts);
            let seconds = t.class1_records.iter_mut().flat_map(|c1| c1.class2_records.iter_mut().map(|r| &mut r.value_record2));
            changed1 | prune_records(seconds)
        }
    }
}

/// A plain anchor in place of a contour-point anchor, or of a device anchor left without
/// device tables; true when the anchor changed.
fn anchor(a: &mut AnchorTable) -> bool {
    let replacement = match a {
        AnchorTable::Format1(_) => return false,
        AnchorTable::Format2(t) => AnchorFormat1::new(t.x_coordinate, t.y_coordinate),
        AnchorTable::Format3(t) => {
            let mut changed = false;
            for device in [&mut t.x_device, &mut t.y_device] {
                if is_hinting(device) {
                    *device = NullableOffsetMarker::new(None);
                    changed = true;
                }
            }
            if t.x_device.is_some() || t.y_device.is_some() {
                return changed;
            }
            AnchorFormat1::new(t.x_coordinate, t.y_coordinate)
        }
    };
    *a = AnchorTable::Format1(replacement);
    true
}

fn nullable_anchor(a: &mut NullableOffsetMarker<AnchorTable>) -> bool {
    a.as_mut().is_some_and(anchor)
}

fn cursive(c: &mut CursivePosFormat1) -> bool {
    let mut changed = false;
    for r in c.entry_exit_record.iter_mut() {
        changed |= nullable_anchor(&mut r.entry_anchor);
        changed |= nullable_anchor(&mut r.exit_anchor);
    }
    changed
}

fn marks(records: &mut [write_fonts::tables::gpos::MarkRecord]) -> bool {
    let mut changed = false;
    for m in records {
        changed |= anchor(&mut m.mark_anchor);
    }
    changed
}

fn mark_base(m: &mut MarkBasePosFormat1) -> bool {
    let mut changed = marks(&mut m.mark_array.mark_records);
    for b in m.base_array.base_records.iter_mut() {
        for a in b.base_anchors.iter_mut() {
            changed |= nullable_anchor(a);
        }
    }
    changed
}

fn mark_lig(m: &mut MarkLigPosFormat1) -> bool {
    let mut changed = marks(&mut m.mark_array.mark_records);
    for lig in m.ligature_array.ligature_attaches.iter_mut() {
        for c in lig.component_records.iter_mut() {
            for a in c.ligature_anchors.iter_mut() {
                changed |= nullable_anchor(a);
            }
        }
    }
    changed
}

fn mark_mark(m: &mut MarkMarkPosFormat1) -> bool {
    let mut changed = marks(&mut m.mark1_array.mark_records);
    for r in m.mark2_array.mark2_records.iter_mut() {
        for a in r.mark2_anchors.iter_mut() {
            changed |= nullable_anchor(a);
        }
    }
    changed
}
