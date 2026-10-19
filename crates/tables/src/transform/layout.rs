//! Scaling the layout and variation tables to new units per em, as fontTools' `scaleUpem`
//! scales them: `GPOS` value records and anchors, `GDEF` caret coordinates, `BASE`
//! coordinates, and the deltas of every item variation store (`GDEF`, `BASE`, `HVAR`, `VVAR`,
//! `MVAR`). Device tables, which hold pixel adjustments, stay as they are. Each table goes
//! through write-fonts' owned types and is written again.

use read_fonts::{FontData, FontRead};
use write_fonts::NullableOffsetMarker;
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::base::{Base, BaseCoord, MinMax};
use write_fonts::tables::gdef::{CaretValue, Gdef};
use write_fonts::tables::gpos::{
    AnchorTable, CursivePosFormat1, ExtensionSubtable, Gpos, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1, MarkRecord, PairPos,
    PositionLookup, SinglePos, ValueRecord,
};
use write_fonts::tables::hvar::Hvar;
use write_fonts::tables::mvar::Mvar;
use write_fonts::tables::variations::{ItemVariationData, ItemVariationStore};
use write_fonts::tables::vvar::Vvar;

use super::glyf::ot_round;
use crate::{Result, TablesError};

/// Scales one 16-bit value, refusing a result outside its range.
#[derive(Clone, Copy)]
pub(super) struct Scaler {
    pub factor: f64,
}

impl Scaler {
    pub fn i32(self, v: i32) -> i32 {
        ot_round(f64::from(v) * self.factor) as i32
    }

    pub fn i16(self, v: i16, what: &str) -> Result<i16> {
        let r = self.i32(i32::from(v));
        i16::try_from(r).map_err(|_| TablesError::Invalid(format!("{what} would be {r}, outside ±32,767")))
    }

    fn opt(self, v: &mut Option<i16>, what: &str) -> Result<()> {
        if let Some(x) = v {
            *x = self.i16(*x, what)?;
        }
        Ok(())
    }
}

fn written<T: write_fonts::FontWrite + write_fonts::validate::Validate>(table: &T, tag: &str) -> Result<Vec<u8>> {
    write_fonts::dump_table(table).map_err(|e| TablesError::Invalid(format!("the {tag} table could not be written again: {e}")))
}

fn damaged(tag: &str, e: impl std::fmt::Display) -> TablesError {
    TablesError::Invalid(format!("the {tag} table: {e}"))
}

// GPOS

pub(super) fn scale_gpos(data: &[u8], s: Scaler) -> Result<Vec<u8>> {
    crate::layout_budget::check_gpos(data)?;
    let read = read_fonts::tables::gpos::Gpos::read(FontData::new(data)).map_err(|e| damaged("GPOS", e))?;
    let mut gpos: Gpos = read.to_owned_table();
    crate::layout_budget::restore_empty_pair_records(&mut gpos)?;
    for lookup in gpos.lookup_list.lookups.iter_mut() {
        lookup_values(lookup, s)?;
    }
    written(&gpos, "GPOS")
}

fn lookup_values(lookup: &mut PositionLookup, s: Scaler) -> Result<()> {
    match lookup {
        PositionLookup::Single(l) => l.subtables.iter_mut().try_for_each(|t| single(t, s)),
        PositionLookup::Pair(l) => l.subtables.iter_mut().try_for_each(|t| pair(t, s)),
        PositionLookup::Cursive(l) => l.subtables.iter_mut().try_for_each(|t| cursive(t, s)),
        PositionLookup::MarkToBase(l) => l.subtables.iter_mut().try_for_each(|t| mark_base(t, s)),
        PositionLookup::MarkToLig(l) => l.subtables.iter_mut().try_for_each(|t| mark_lig(t, s)),
        PositionLookup::MarkToMark(l) => l.subtables.iter_mut().try_for_each(|t| mark_mark(t, s)),
        PositionLookup::Contextual(_) | PositionLookup::ChainContextual(_) => Ok(()),
        PositionLookup::Extension(l) => l.subtables.iter_mut().try_for_each(|ext| match &mut **ext {
            ExtensionSubtable::Single(e) => single(&mut e.extension, s),
            ExtensionSubtable::Pair(e) => pair(&mut e.extension, s),
            ExtensionSubtable::Cursive(e) => cursive(&mut e.extension, s),
            ExtensionSubtable::MarkToBase(e) => mark_base(&mut e.extension, s),
            ExtensionSubtable::MarkToLig(e) => mark_lig(&mut e.extension, s),
            ExtensionSubtable::MarkToMark(e) => mark_mark(&mut e.extension, s),
            ExtensionSubtable::Contextual(_) | ExtensionSubtable::ChainContextual(_) => Ok(()),
        }),
    }
}

fn record(r: &mut ValueRecord, s: Scaler) -> Result<()> {
    s.opt(&mut r.x_placement, "a GPOS x placement")?;
    s.opt(&mut r.y_placement, "a GPOS y placement")?;
    s.opt(&mut r.x_advance, "a GPOS x advance")?;
    s.opt(&mut r.y_advance, "a GPOS y advance")
}

fn single(t: &mut SinglePos, s: Scaler) -> Result<()> {
    match t {
        SinglePos::Format1(t) => record(&mut t.value_record, s),
        SinglePos::Format2(t) => t.value_records.iter_mut().try_for_each(|r| record(r, s)),
    }
}

fn pair(t: &mut PairPos, s: Scaler) -> Result<()> {
    match t {
        PairPos::Format1(t) => {
            for set in t.pair_sets.iter_mut() {
                for r in set.pair_value_records.iter_mut() {
                    record(&mut r.value_record1, s)?;
                    record(&mut r.value_record2, s)?;
                }
            }
            Ok(())
        }
        PairPos::Format2(t) => {
            for c1 in t.class1_records.iter_mut() {
                for r in c1.class2_records.iter_mut() {
                    record(&mut r.value_record1, s)?;
                    record(&mut r.value_record2, s)?;
                }
            }
            Ok(())
        }
    }
}

fn anchor(a: &mut AnchorTable, s: Scaler) -> Result<()> {
    let (x, y) = match a {
        AnchorTable::Format1(t) => (&mut t.x_coordinate, &mut t.y_coordinate),
        AnchorTable::Format2(t) => (&mut t.x_coordinate, &mut t.y_coordinate),
        AnchorTable::Format3(t) => (&mut t.x_coordinate, &mut t.y_coordinate),
    };
    *x = s.i16(*x, "an anchor's x coordinate")?;
    *y = s.i16(*y, "an anchor's y coordinate")?;
    Ok(())
}

fn nullable_anchor(a: &mut NullableOffsetMarker<AnchorTable>, s: Scaler) -> Result<()> {
    a.as_mut().map_or(Ok(()), |a| anchor(a, s))
}

fn cursive(t: &mut CursivePosFormat1, s: Scaler) -> Result<()> {
    for r in t.entry_exit_record.iter_mut() {
        nullable_anchor(&mut r.entry_anchor, s)?;
        nullable_anchor(&mut r.exit_anchor, s)?;
    }
    Ok(())
}

fn marks(records: &mut [MarkRecord], s: Scaler) -> Result<()> {
    records.iter_mut().try_for_each(|m| anchor(&mut m.mark_anchor, s))
}

fn mark_base(t: &mut MarkBasePosFormat1, s: Scaler) -> Result<()> {
    marks(&mut t.mark_array.mark_records, s)?;
    for b in t.base_array.base_records.iter_mut() {
        b.base_anchors.iter_mut().try_for_each(|a| nullable_anchor(a, s))?;
    }
    Ok(())
}

fn mark_lig(t: &mut MarkLigPosFormat1, s: Scaler) -> Result<()> {
    marks(&mut t.mark_array.mark_records, s)?;
    for lig in t.ligature_array.ligature_attaches.iter_mut() {
        for c in lig.component_records.iter_mut() {
            c.ligature_anchors.iter_mut().try_for_each(|a| nullable_anchor(a, s))?;
        }
    }
    Ok(())
}

fn mark_mark(t: &mut MarkMarkPosFormat1, s: Scaler) -> Result<()> {
    marks(&mut t.mark1_array.mark_records, s)?;
    for r in t.mark2_array.mark2_records.iter_mut() {
        r.mark2_anchors.iter_mut().try_for_each(|a| nullable_anchor(a, s))?;
    }
    Ok(())
}

// GDEF and BASE

/// `None` when the table holds nothing in font units.
pub(super) fn scale_gdef(data: &[u8], s: Scaler) -> Result<Option<Vec<u8>>> {
    crate::layout_budget::check_gdef(data)?;
    let read = read_fonts::tables::gdef::Gdef::read(FontData::new(data)).map_err(|e| damaged("GDEF", e))?;
    let mut gdef: Gdef = read.to_owned_table();
    if gdef.lig_caret_list.is_none() && gdef.item_var_store.is_none() {
        return Ok(None);
    }
    if let Some(list) = gdef.lig_caret_list.as_mut() {
        for lig in list.lig_glyphs.iter_mut() {
            for caret in lig.caret_values.iter_mut() {
                match &mut **caret {
                    CaretValue::Format1(c) => c.coordinate = s.i16(c.coordinate, "a ligature caret")?,
                    CaretValue::Format2(_) => {}
                    CaretValue::Format3(c) => c.coordinate = s.i16(c.coordinate, "a ligature caret")?,
                }
            }
        }
    }
    if let Some(store) = gdef.item_var_store.as_mut() {
        scale_store(store, s)?;
    }
    written(&gdef, "GDEF").map(Some)
}

fn base_coord(c: &mut BaseCoord, s: Scaler) -> Result<()> {
    let v = match c {
        BaseCoord::Format1(t) => &mut t.coordinate,
        BaseCoord::Format2(t) => &mut t.coordinate,
        BaseCoord::Format3(t) => &mut t.coordinate,
    };
    *v = s.i16(*v, "a BASE coordinate")?;
    Ok(())
}

fn nullable_coord(c: &mut NullableOffsetMarker<BaseCoord>, s: Scaler) -> Result<()> {
    c.as_mut().map_or(Ok(()), |c| base_coord(c, s))
}

fn min_max(m: &mut MinMax, s: Scaler) -> Result<()> {
    nullable_coord(&mut m.min_coord, s)?;
    nullable_coord(&mut m.max_coord, s)?;
    for f in m.feat_min_max_records.iter_mut() {
        nullable_coord(&mut f.min_coord, s)?;
        nullable_coord(&mut f.max_coord, s)?;
    }
    Ok(())
}

pub(super) fn scale_base(data: &[u8], s: Scaler) -> Result<Vec<u8>> {
    crate::layout_budget::check_base(data)?;
    let read = read_fonts::tables::base::Base::read(FontData::new(data)).map_err(|e| damaged("BASE", e))?;
    let mut base: Base = read.to_owned_table();
    for axis in [base.horiz_axis.as_mut(), base.vert_axis.as_mut()].into_iter().flatten() {
        for rec in axis.base_script_list.base_script_records.iter_mut() {
            let script = &mut *rec.base_script;
            if let Some(values) = script.base_values.as_mut() {
                values.base_coords.iter_mut().try_for_each(|c| base_coord(c, s))?;
            }
            if let Some(m) = script.default_min_max.as_mut() {
                min_max(m, s)?;
            }
            for lang in script.base_lang_sys_records.iter_mut() {
                min_max(&mut lang.min_max, s)?;
            }
        }
    }
    if let Some(store) = base.item_var_store.as_mut() {
        scale_store(store, s)?;
    }
    written(&base, "BASE")
}

// Item variation stores

pub(super) fn scale_hvar(data: &[u8], s: Scaler) -> Result<Vec<u8>> {
    let read = read_fonts::tables::hvar::Hvar::read(FontData::new(data)).map_err(|e| damaged("HVAR", e))?;
    let mut t: Hvar = read.to_owned_table();
    scale_store(&mut t.item_variation_store, s)?;
    written(&t, "HVAR")
}

pub(super) fn scale_vvar(data: &[u8], s: Scaler) -> Result<Vec<u8>> {
    let read = read_fonts::tables::vvar::Vvar::read(FontData::new(data)).map_err(|e| damaged("VVAR", e))?;
    let mut t: Vvar = read.to_owned_table();
    scale_store(&mut t.item_variation_store, s)?;
    written(&t, "VVAR")
}

pub(super) fn scale_mvar(data: &[u8], s: Scaler) -> Result<Vec<u8>> {
    let read = read_fonts::tables::mvar::Mvar::read(FontData::new(data)).map_err(|e| damaged("MVAR", e))?;
    let mut t: Mvar = read.to_owned_table();
    if let Some(store) = t.item_variation_store.as_mut() {
        scale_store(store, s)?;
    }
    written(&t, "MVAR")
}

fn scale_store(store: &mut ItemVariationStore, s: Scaler) -> Result<()> {
    for data in store.item_variation_data.iter_mut() {
        if let Some(d) = data.as_mut() {
            *d = scaled_var_data(d, s)?;
        }
    }
    Ok(())
}

const LONG_WORDS: u16 = 0x8000;
const WORD_DELTA_COUNT_MASK: u16 = 0x7FFF;

/// The rows of `d` decoded, one value per region.
fn rows(d: &ItemVariationData) -> Result<Vec<Vec<i32>>> {
    let regions = d.region_indexes.len();
    let long = d.word_delta_count & LONG_WORDS != 0;
    let words = usize::from(d.word_delta_count & WORD_DELTA_COUNT_MASK);
    if words > regions {
        return Err(TablesError::invalid("an item variation data table has more word columns than regions"));
    }
    let (wide, narrow) = if long { (4, 2) } else { (2, 1) };
    let row_len = words * wide + (regions - words) * narrow;
    let items = usize::from(d.item_count);
    if items.checked_mul(row_len).is_none_or(|n| n > d.delta_sets.len()) {
        return Err(TablesError::invalid("an item variation data table is shorter than its item count says"));
    }
    let mut out = Vec::with_capacity(items);
    for item in 0..items {
        let row = d.delta_sets.get(item * row_len..(item + 1) * row_len).unwrap_or_default();
        let mut values = Vec::with_capacity(regions);
        let mut at = 0;
        for col in 0..regions {
            let width = if col < words { wide } else { narrow };
            let bytes = row.get(at..at + width).unwrap_or_default();
            let v = match bytes {
                [a] => i32::from(*a as i8),
                [a, b] => i32::from(i16::from_be_bytes([*a, *b])),
                [a, b, c, e] => i32::from_be_bytes([*a, *b, *c, *e]),
                _ => 0,
            };
            values.push(v);
            at += width;
        }
        out.push(values);
    }
    Ok(out)
}

/// `d` with every delta scaled, its columns reordered so the wide ones come first, as fontTools'
/// `calculateNumShorts` lays them out.
fn scaled_var_data(d: &ItemVariationData, s: Scaler) -> Result<ItemVariationData> {
    let mut rows = rows(d)?;
    let regions = d.region_indexes.len();
    let mut need = vec![1usize; regions];
    for row in rows.iter_mut() {
        for (col, v) in row.iter_mut().enumerate() {
            *v = s.i32(*v);
            let n = if (-128..=127).contains(v) {
                1
            } else if (-32768..=32767).contains(v) {
                2
            } else {
                4
            };
            if let Some(slot) = need.get_mut(col) {
                *slot = (*slot).max(n);
            }
        }
    }
    let long = need.contains(&4);
    let is_wide = |n: usize| if long { n == 4 } else { n == 2 };
    let mut order: Vec<usize> = (0..regions).filter(|&c| need.get(c).is_some_and(|&n| is_wide(n))).collect();
    let words = order.len();
    order.extend((0..regions).filter(|&c| need.get(c).is_some_and(|&n| !is_wide(n))));
    let region_indexes = order.iter().filter_map(|&c| d.region_indexes.get(c).copied()).collect();
    let mut delta_sets = Vec::new();
    for row in &rows {
        for (i, &c) in order.iter().enumerate() {
            let v = row.get(c).copied().unwrap_or(0);
            match (long, i < words) {
                (true, true) => delta_sets.extend_from_slice(&v.to_be_bytes()),
                (true, false) | (false, true) => delta_sets.extend_from_slice(&(v as i16).to_be_bytes()),
                (false, false) => delta_sets.push(v as i8 as u8),
            }
        }
    }
    let word_count = u16::try_from(words).map_err(|_| TablesError::invalid("an item variation data table has too many regions"))?;
    Ok(ItemVariationData::new(d.item_count, word_count | if long { LONG_WORDS } else { 0 }, region_indexes, delta_sets))
}
