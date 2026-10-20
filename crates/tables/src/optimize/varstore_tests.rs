#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

//! Expected values come from fontTools 4.66: `VarStore.optimize()` on a store built by
//! `varLib.builder.buildVarStore` with the same regions and rows.

use super::*;
use crate::Flavour;

/// Two regions: weight 0 → 1 → 1, and width 0 → 1 → 1.
fn regions() -> Vec<Vec<u8>> {
    let tent = |on: bool| if on { [0u8, 0, 0x40, 0, 0x40, 0] } else { [0u8; 6] };
    vec![[tent(true), tent(false)].concat(), [tent(false), tent(true)].concat()]
}

fn store() -> Vec<u8> {
    let r = regions();
    let refs: Vec<&Vec<u8>> = r.iter().collect();
    write_store(2, &refs, &[(vec![0], vec![vec![5], vec![0], vec![300]]), (vec![0, 1], vec![vec![5, 0], vec![1, 2]])]).unwrap()
}

/// GDEF 1.3 holding only the store.
fn gdef() -> Vec<u8> {
    let mut g = vec![0, 1, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 18];
    g.extend(store());
    g
}

/// GPOS with one single adjustment of glyph 0 whose advance varies by `outer`/`inner`.
fn gpos(outer: u16, inner: u16) -> Vec<u8> {
    let mut g = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 10];
    g.extend([0, 1, 0, 4]); // lookup list at 10: one lookup at +4
    g.extend([0, 1, 0, 0, 0, 1, 0, 8]); // lookup at 14: type 1, one subtable at +8
    // Subtable at 22: format 1, coverage at +12, xAdvance and xAdvDevice, device at +18.
    g.extend([0, 1, 0, 12, 0, 0x44, 0, 50, 0, 18]);
    g.extend([0, 0]);
    g.extend([0, 1, 0, 1, 0, 0]); // coverage at 34
    g.extend(outer.to_be_bytes()); // device at 40
    g.extend(inner.to_be_bytes());
    g.extend([0x80, 0]);
    g
}

#[test]
fn the_store_is_merged_as_the_reference_merges_it() {
    let s = read_store(&store()).unwrap();
    let (bytes, map) = optimize_store(&s, 1).unwrap();
    let mut pairs: Vec<(u32, u32)> = map.into_iter().filter(|(k, _)| *k != NO_VARIATION).collect();
    pairs.sort_unstable();
    assert_eq!(pairs, [(0, 1), (1, NO_VARIATION), (2, 2), (1 << 16, 1), ((1 << 16) | 1, 0)]);
    let out = read_store(&bytes).unwrap();
    assert_eq!(out.regions.len(), 2);
    assert_eq!(out.data, [(vec![0, 1], vec![vec![1, 2], vec![5, 0], vec![300, 0]])]);
    let sub = u32_at(&bytes, 8).unwrap() as usize;
    assert_eq!(u16_at(&bytes, sub + 2), Some(1), "one word column, as NumShorts says");
}

#[test]
fn quantisation_rounds_half_to_even_before_merging() {
    let r = regions();
    let refs: Vec<&Vec<u8>> = r.iter().collect();
    let bytes = write_store(2, &refs, &[(vec![0, 1], vec![vec![15, 5], vec![25, 4], vec![35, 0]])]).unwrap();
    let (out, _) = optimize_store(&read_store(&bytes).unwrap(), 10).unwrap();
    // 1.5 → 2, 0.5 → 0, 2.5 → 2, 0.4 → 0, 3.5 → 4: the first two rows become one.
    assert_eq!(read_store(&out).unwrap().data, [(vec![0], vec![vec![20], vec![40]])]);
}

#[test]
fn gdef_and_gpos_follow_the_new_indices() {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"GDEF", gdef());
    font.insert(*b"GPOS", gpos(1, 1));
    let tables = run(&font, 1).unwrap().unwrap();
    let get = |tag: &[u8; 4]| tables.iter().find(|(t, _)| t == tag).map(|(_, b)| b.clone()).unwrap();
    let gpos_out = get(b"GPOS");
    assert_eq!(&gpos_out[40..44], &[0, 0, 0, 0], "row (1, 2) is now 0/0");
    assert_eq!(gpos_out.len(), gpos(1, 1).len(), "GPOS keeps its layout");
    let gdef_out = get(b"GDEF");
    let at = u32_at(&gdef_out, 14).unwrap() as usize;
    assert_eq!(read_store(&gdef_out[at..]).unwrap().data.len(), 1);

    font.insert(*b"GPOS", gpos(3, 0));
    assert!(run(&font, 1).is_err(), "an index the store lacks fails the step");
    assert!(matches!(run(&font, 0), Err(TablesError::BadOptions(_))));
}
