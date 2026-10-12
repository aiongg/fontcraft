//! The `MVAR` table (M9.6).
//!
//! Rows: the `MVAR` table row.
//!
//! Contract: one record per value tag (`hasc`, `xhgt`, `undo`…) whose value varies, sorted by
//! tag; no table when nothing varies.
//!
//! As fontc writes it (`fontbe` `mvar.rs`, Apache-2.0): values whose deltas are all zero are left
//! out, the others go into one deduplicated item variation store, added in tag order.

use write_fonts::tables::mvar::{Mvar, ValueRecord};
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::types::{MajorMinor, Tag};

use super::VarValue;
use super::hvar::store_deltas;
use crate::CompileError;

/// The most value records (the table's count is 16 bits).
pub const MAX_VALUE_RECORDS: usize = u16::MAX as usize;

/// The `MVAR` table of `values` (value tag, value); `None` when no value varies.
pub fn build_mvar(axis_count: u16, values: &[(Tag, VarValue)]) -> Result<Option<Mvar>, CompileError> {
    if values.len() > MAX_VALUE_RECORDS {
        return Err(CompileError::Table { table: "MVAR", message: format!("{} values are more than the table holds", values.len()) });
    }
    let mut varying: Vec<&(Tag, VarValue)> = values.iter().filter(|(_, v)| v.deltas.iter().any(|(_, d)| *d != 0)).collect();
    if varying.is_empty() {
        return Ok(None);
    }
    if axis_count == 0 {
        return Err(CompileError::Table { table: "MVAR", message: "a variable font needs at least one axis".into() });
    }
    varying.sort_by_key(|(tag, _)| *tag);
    if let Some(pair) = varying.windows(2).find(|w| matches!(w, [a, b] if a.0 == b.0)) {
        let tag = pair.first().map(|p| p.0.to_string()).unwrap_or_default();
        return Err(CompileError::Table { table: "MVAR", message: format!("the value `{tag}` is given twice") });
    }
    let refs: Vec<&VarValue> = varying.iter().map(|(_, v)| v).collect();
    let deltas = store_deltas(&refs, axis_count, "MVAR")?;

    let mut builder = VariationStoreBuilder::new(axis_count);
    let ids: Vec<(Tag, u32)> = varying.iter().zip(deltas).map(|((tag, _), d)| (*tag, builder.add_deltas(d))).collect();
    let (store, remap) = builder.build();
    let records = ids
        .into_iter()
        .map(|(tag, id)| {
            let index = remap
                .get(id)
                .ok_or_else(|| CompileError::Table { table: "MVAR", message: "a delta set was lost while building the store".into() })?;
            Ok(ValueRecord::new(tag, index.delta_set_outer_index, index.delta_set_inner_index))
        })
        .collect::<Result<Vec<_>, CompileError>>()?;
    Ok(Some(Mvar::new(MajorMinor::VERSION_1_0, Some(store), records)))
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::variation::RegionAxis;
    use write_fonts::dump_table;

    use super::super::Region;
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn value(d: i32) -> VarValue {
        VarValue { default: 0, deltas: vec![(Region(vec![RegionAxis { start: 0.0, peak: 1.0, end: 1.0 }]), d)] }
    }

    // Expected bytes: fontc 1.0.0 on its test source `WghtVar.glyphs` (masters 400 and 700).
    #[test]
    fn mvar_matches_the_oracle() {
        let values = [
            (Tag::new(b"xhgt"), value(-1)),
            (Tag::new(b"hasc"), value(63)),
            (Tag::new(b"hcla"), value(-158)),
            (Tag::new(b"cpht"), value(-2)),
            (Tag::new(b"hcld"), value(158)),
            (Tag::new(b"hdsc"), value(-158)),
            (Tag::new(b"hlgp"), value(-221)),
            (Tag::new(b"stro"), value(-1)),
            (Tag::new(b"unds"), value(0)),
        ];
        let mvar = build_mvar(1, &values).unwrap().unwrap();
        assert_eq!(
            hex(&dump_table(&mvar).unwrap()),
            "00010000000000080008004c6370687400000002686173630000000468636c610000000168636c64000000056864736300000001686c6770000000007374726f00000003786867740000000300010000000c000100000016000100010000400040000006000100010000ff23ff62fffeffff003f009e"
        );
    }

    #[test]
    fn mvar_is_left_out_when_nothing_varies() {
        assert_eq!(build_mvar(1, &[(Tag::new(b"xhgt"), value(0)), (Tag::new(b"cpht"), VarValue::default())]).unwrap(), None);
        assert!(build_mvar(1, &[(Tag::new(b"xhgt"), value(3)), (Tag::new(b"xhgt"), value(4))]).is_err());
    }
}
