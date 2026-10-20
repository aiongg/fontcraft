//! `GDEF`, `GPOS` and `GSUB` of a full instance, as fontTools' `instantiateOTL` and
//! `instantiateFeatureVariations` make them (MIT, Copyright (c) 2019 Google LLC and the
//! fontTools authors):
//!
//! - every variation-index device of a `GPOS` value record or anchor, and of a `GDEF` ligature
//!   caret, adds its rounded delta to the value it varies and goes away; a value the record did
//!   not hold becomes present; the records of a subtable then share the union of their fields;
//! - an anchor or caret left without device tables becomes format 1;
//! - `GDEF` loses its variation store (and with it version 1.3), and goes away when nothing is
//!   left in it;
//! - the first feature-variation record whose conditions hold at the location replaces the
//!   features it names, and the feature variations go away.
//!
//! fontTools also joins subtables that a variable build had to split and drops lookups no
//! feature reaches any more; neither changes what the font does, and neither is done here.

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gdef::{CaretValue, Gdef};
use write_fonts::tables::gpos::{
    AnchorFormat1, AnchorTable, CursivePosFormat1, ExtensionSubtable, Gpos, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1, MarkRecord,
    PairPos, PositionLookup, SinglePos, ValueFormat, ValueRecord,
};
use write_fonts::tables::gsub::Gsub;
use write_fonts::tables::layout::{Condition, DeviceOrVariationIndex, FeatureList, FeatureVariations};
use write_fonts::{NullableOffsetMarker, OffsetMarker};

use super::ivs::StoreDeltas;
use super::ot_round;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

/// Applies `BASE`'s variation store to `BASE` and `GDEF`'s to `GDEF` and `GPOS`, then resolves
/// the feature variations of `GSUB` and `GPOS`. `coords` is the normalised location, one value per `fvar` axis.
pub(super) fn instantiate_layout(font: &mut SfntFont, coords: &[f64]) -> Result<()> {
    super::base::full(font, coords)?;
    let deltas = gdef_deltas(font, coords)?;
    if let Some(deltas) = &deltas
        && let Some(data) = font.get(*b"GDEF")
    {
        crate::layout_budget::check_gdef(data)?;
        let read = read_fonts::tables::gdef::Gdef::read(FontData::new(data)).map_err(|e| bad(format!("the GDEF table cannot be read: {e}")))?;
        let mut gdef: Gdef = read.to_owned_table();
        if let Some(list) = gdef.lig_caret_list.as_mut() {
            for lig in list.lig_glyphs.iter_mut() {
                for caret in lig.caret_values.iter_mut() {
                    if let CaretValue::Format3(t) = &**caret
                        && let DeviceOrVariationIndex::VariationIndex(v) = &*t.device
                    {
                        let d = ot_round(deltas.get(v.delta_set_outer_index, v.delta_set_inner_index));
                        let value = clamp_i16(f64::from(t.coordinate) + d);
                        **caret = CaretValue::format_1(value);
                    }
                }
            }
        }
        gdef.item_var_store = NullableOffsetMarker::new(None);
        let empty = gdef.glyph_class_def.is_none()
            && gdef.attach_list.is_none()
            && gdef.lig_caret_list.is_none()
            && gdef.mark_attach_class_def.is_none()
            && gdef.mark_glyph_sets_def.is_none();
        if empty {
            font.remove(*b"GDEF");
        } else {
            let bytes = write_fonts::dump_table(&gdef).map_err(|e| bad(format!("the GDEF table could not be written again: {e}")))?;
            font.insert(*b"GDEF", bytes);
        }
    }
    if let Some(data) = font.get(*b"GPOS") {
        crate::layout_budget::check_gpos(data)?;
        let read = read_fonts::tables::gpos::Gpos::read(FontData::new(data)).map_err(|e| bad(format!("the GPOS table cannot be read: {e}")))?;
        let has_variations = read.feature_variations_offset().is_some_and(|o| !o.is_null());
        if deltas.is_some() || has_variations {
            let mut gpos: Gpos = read.to_owned_table();
            crate::layout_budget::restore_empty_pair_records(&mut gpos)?;
            if let Some(deltas) = &deltas {
                apply_gpos(&mut gpos, Fold { deltas, keep_devices: false });
            }
            resolve_feature_variations(&mut gpos.feature_list, &mut gpos.feature_variations, coords);
            let bytes = write_fonts::dump_table(&gpos).map_err(|e| bad(format!("the GPOS table could not be written again: {e}")))?;
            font.insert(*b"GPOS", bytes);
        }
    }
    if let Some(data) = font.get(*b"GSUB") {
        let read = read_fonts::tables::gsub::Gsub::read(FontData::new(data)).map_err(|e| bad(format!("the GSUB table cannot be read: {e}")))?;
        if read.feature_variations_offset().is_some_and(|o| !o.is_null()) {
            crate::layout_budget::check_gsub(data)?;
            let mut gsub: Gsub = read.to_owned_table();
            resolve_feature_variations(&mut gsub.feature_list, &mut gsub.feature_variations, coords);
            let bytes = write_fonts::dump_table(&gsub).map_err(|e| bad(format!("the GSUB table could not be written again: {e}")))?;
            font.insert(*b"GSUB", bytes);
        }
    }
    Ok(())
}

/// The deltas of `GDEF`'s variation store at `coords`; `None` when it has none.
fn gdef_deltas(font: &SfntFont, coords: &[f64]) -> Result<Option<StoreDeltas>> {
    let Some(data) = font.get(*b"GDEF") else { return Ok(None) };
    let gdef = read_fonts::tables::gdef::Gdef::read(FontData::new(data)).map_err(|e| bad(format!("the GDEF table cannot be read: {e}")))?;
    match gdef.item_var_store() {
        None => Ok(None),
        Some(store) => {
            let store = store.map_err(|e| bad(format!("the GDEF variation store cannot be read: {e}")))?;
            StoreDeltas::from_store(&store, coords).map(Some)
        }
    }
}

fn clamp_i16(v: f64) -> i16 {
    v.clamp(-32768.0, 32767.0) as i16
}

/// The rounded delta of a variation-index device; `None` for no device or a hinting device.
fn device_delta(device: &NullableOffsetMarker<DeviceOrVariationIndex>, deltas: &StoreDeltas) -> Option<f64> {
    match device.as_ref() {
        Some(DeviceOrVariationIndex::VariationIndex(v)) => Some(ot_round(deltas.get(v.delta_set_outer_index, v.delta_set_inner_index))),
        _ => None,
    }
}

fn apply_record(r: &mut ValueRecord, fold: Fold<'_>) -> bool {
    let mut changed = false;
    let pairs: [(&mut Option<i16>, &mut NullableOffsetMarker<DeviceOrVariationIndex>); 4] = [
        (&mut r.x_placement, &mut r.x_placement_device),
        (&mut r.y_placement, &mut r.y_placement_device),
        (&mut r.x_advance, &mut r.x_advance_device),
        (&mut r.y_advance, &mut r.y_advance_device),
    ];
    for (value, device) in pairs {
        if let Some(d) = device_delta(device, fold.deltas) {
            *value = Some(clamp_i16(f64::from(value.unwrap_or(0)) + d));
            if !fold.keep_devices {
                *device = NullableOffsetMarker::new(None);
            }
            changed = true;
        }
    }
    changed
}

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

/// Applies the deltas to every record of one subtable and gives them the union of their fields.
fn apply_records<'r>(records: impl Iterator<Item = &'r mut ValueRecord>, deltas: Fold<'_>) {
    let mut records: Vec<&mut ValueRecord> = records.collect();
    let mut changed = false;
    for r in records.iter_mut() {
        changed |= apply_record(r, deltas);
    }
    if changed {
        let format = records.iter().fold(ValueFormat::empty(), |f, r| f | fields(r));
        for r in records.iter_mut() {
            r.set_explicit_value_format(format);
        }
    }
}

/// The deltas one pass adds to `GPOS` values, and whether the variation-index devices stay (a
/// partial instance, whose store still varies them) or go (a full instance).
#[derive(Clone, Copy)]
pub(crate) struct Fold<'a> {
    pub deltas: &'a StoreDeltas,
    pub keep_devices: bool,
}

/// Adds the rounded delta of every variation-index device in `GPOS` to the value it varies.
pub(crate) fn apply_gpos(gpos: &mut Gpos, fold: Fold<'_>) {
    for lookup in gpos.lookup_list.lookups.iter_mut() {
        lookup_deltas(lookup, fold);
    }
}

fn lookup_deltas(lookup: &mut PositionLookup, d: Fold<'_>) {
    match lookup {
        PositionLookup::Single(l) => l.subtables.iter_mut().for_each(|s| single(s, d)),
        PositionLookup::Pair(l) => l.subtables.iter_mut().for_each(|s| pair(s, d)),
        PositionLookup::Cursive(l) => l.subtables.iter_mut().for_each(|s| cursive(s, d)),
        PositionLookup::MarkToBase(l) => l.subtables.iter_mut().for_each(|s| mark_base(s, d)),
        PositionLookup::MarkToLig(l) => l.subtables.iter_mut().for_each(|s| mark_lig(s, d)),
        PositionLookup::MarkToMark(l) => l.subtables.iter_mut().for_each(|s| mark_mark(s, d)),
        PositionLookup::Contextual(_) | PositionLookup::ChainContextual(_) => {}
        PositionLookup::Extension(l) => {
            for ext in l.subtables.iter_mut() {
                match &mut **ext {
                    ExtensionSubtable::Single(e) => single(&mut e.extension, d),
                    ExtensionSubtable::Pair(e) => pair(&mut e.extension, d),
                    ExtensionSubtable::Cursive(e) => cursive(&mut e.extension, d),
                    ExtensionSubtable::MarkToBase(e) => mark_base(&mut e.extension, d),
                    ExtensionSubtable::MarkToLig(e) => mark_lig(&mut e.extension, d),
                    ExtensionSubtable::MarkToMark(e) => mark_mark(&mut e.extension, d),
                    ExtensionSubtable::Contextual(_) | ExtensionSubtable::ChainContextual(_) => {}
                }
            }
        }
    }
}

fn single(s: &mut SinglePos, d: Fold<'_>) {
    match s {
        SinglePos::Format1(t) => apply_records(std::iter::once(&mut t.value_record), d),
        SinglePos::Format2(t) => apply_records(t.value_records.iter_mut(), d),
    }
}

fn pair(p: &mut PairPos, d: Fold<'_>) {
    match p {
        PairPos::Format1(t) => {
            apply_records(t.pair_sets.iter_mut().flat_map(|set| set.pair_value_records.iter_mut().map(|r| &mut r.value_record1)), d);
            apply_records(t.pair_sets.iter_mut().flat_map(|set| set.pair_value_records.iter_mut().map(|r| &mut r.value_record2)), d);
        }
        PairPos::Format2(t) => {
            apply_records(t.class1_records.iter_mut().flat_map(|c1| c1.class2_records.iter_mut().map(|r| &mut r.value_record1)), d);
            apply_records(t.class1_records.iter_mut().flat_map(|c1| c1.class2_records.iter_mut().map(|r| &mut r.value_record2)), d);
        }
    }
}

fn anchor(a: &mut AnchorTable, d: Fold<'_>) {
    let AnchorTable::Format3(t) = a else { return };
    let mut changed = false;
    if let Some(dx) = device_delta(&t.x_device, d.deltas) {
        t.x_coordinate = clamp_i16(f64::from(t.x_coordinate) + dx);
        if !d.keep_devices {
            t.x_device = NullableOffsetMarker::new(None);
        }
        changed = true;
    }
    if let Some(dy) = device_delta(&t.y_device, d.deltas) {
        t.y_coordinate = clamp_i16(f64::from(t.y_coordinate) + dy);
        if !d.keep_devices {
            t.y_device = NullableOffsetMarker::new(None);
        }
        changed = true;
    }
    if changed && t.x_device.is_none() && t.y_device.is_none() {
        let plain = AnchorFormat1::new(t.x_coordinate, t.y_coordinate);
        *a = AnchorTable::Format1(plain);
    }
}

fn nullable_anchor(a: &mut NullableOffsetMarker<AnchorTable>, d: Fold<'_>) {
    if let Some(a) = a.as_mut() {
        anchor(a, d);
    }
}

fn cursive(c: &mut CursivePosFormat1, d: Fold<'_>) {
    for r in c.entry_exit_record.iter_mut() {
        nullable_anchor(&mut r.entry_anchor, d);
        nullable_anchor(&mut r.exit_anchor, d);
    }
}

fn marks(records: &mut [MarkRecord], d: Fold<'_>) {
    for m in records {
        anchor(&mut m.mark_anchor, d);
    }
}

fn mark_base(m: &mut MarkBasePosFormat1, d: Fold<'_>) {
    marks(&mut m.mark_array.mark_records, d);
    for b in m.base_array.base_records.iter_mut() {
        for a in b.base_anchors.iter_mut() {
            nullable_anchor(a, d);
        }
    }
}

fn mark_lig(m: &mut MarkLigPosFormat1, d: Fold<'_>) {
    marks(&mut m.mark_array.mark_records, d);
    for lig in m.ligature_array.ligature_attaches.iter_mut() {
        for c in lig.component_records.iter_mut() {
            for a in c.ligature_anchors.iter_mut() {
                nullable_anchor(a, d);
            }
        }
    }
}

fn mark_mark(m: &mut MarkMarkPosFormat1, d: Fold<'_>) {
    marks(&mut m.mark1_array.mark_records, d);
    for r in m.mark2_array.mark2_records.iter_mut() {
        for a in r.mark2_anchors.iter_mut() {
            nullable_anchor(a, d);
        }
    }
}

/// Applies the first feature-variation record whose conditions all hold at `coords` and drops
/// the feature variations. A record without a condition set always holds; a condition other
/// than an axis range never does (fontTools' rule).
pub(super) fn resolve_feature_variations(
    features: &mut OffsetMarker<FeatureList>,
    variations: &mut NullableOffsetMarker<FeatureVariations, 4>,
    coords: &[f64],
) {
    let Some(fv) = variations.as_ref() else { return };
    let quantized: Vec<i32> = coords.iter().map(|v| (v * 16384.0).round() as i32).collect();
    let applies = |set: Option<&write_fonts::tables::layout::ConditionSet>| -> bool {
        let Some(set) = set else { return true };
        set.conditions.iter().all(|c| match &**c {
            Condition::Format1AxisRange(r) => {
                let v = quantized.get(usize::from(r.axis_index)).copied().unwrap_or(0);
                i32::from(r.filter_range_min_value.to_bits()) <= v && v <= i32::from(r.filter_range_max_value.to_bits())
            }
            _ => false,
        })
    };
    let chosen = fv.feature_variation_records.iter().find(|rec| applies(rec.condition_set.as_ref()));
    if let Some(subst) = chosen.and_then(|rec| rec.feature_table_substitution.as_ref()) {
        for s in &subst.substitutions {
            if let Some(record) = features.feature_records.get_mut(usize::from(s.feature_index)) {
                record.feature = OffsetMarker::new((*s.alternate_feature).clone());
            }
        }
    }
    *variations = NullableOffsetMarker::new(None);
}
