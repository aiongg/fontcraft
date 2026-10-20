#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

//! Expected values come from fontTools 4.66.1: `otlLib.optimize.gpos.compact` on a font with
//! the same glyph names and the same class pairs, its subtable built by
//! `buildPairPosClassesSubtable`.

use super::*;
use crate::Flavour;
use crate::optimize::{OptimizeOptions, optimize};
use std::collections::BTreeMap;
use write_fonts::tables::gpos::PositionLookupList;
use write_fonts::tables::layout::{FeatureList, Lookup, LookupFlag, ScriptList};
use write_fonts::tables::post::Post;

const GLYPHS: u16 = 320;
const FIRST: usize = 64;
const SECOND: usize = 56;

/// Names whose order differs from the glyph order, so the classes sort by name.
fn name(g: u16) -> String {
    if g == 0 { ".notdef".into() } else { format!("g{:03}", (u32::from(g) * 37) % 512) }
}

fn group1(k: usize) -> Vec<u16> {
    (0..1 + k % 3).map(|t| (1 + k + 64 * t) as u16).collect()
}

fn group2(k: usize) -> Vec<u16> {
    (0..1 + k % 2).map(|t| (200 + k + 56 * t) as u16).collect()
}

/// Blocks along the diagonal and a few pairs off it.
fn value(i: usize, j: usize) -> i16 {
    if i / 8 == j / 7 || (i * 7 + j * 13) % 97 == 0 { -((10 + (i * 3 + j) % 40) as i16) } else { 0 }
}

fn original() -> PairPos {
    let coverage: CoverageTable = (0..FIRST).flat_map(group1).map(GlyphId16::new).collect::<Vec<_>>().into();
    let cd1: ClassDef = (0..FIRST).flat_map(|k| group1(k).into_iter().map(move |g| (GlyphId16::new(g), k as u16 + 1))).collect();
    let cd2: ClassDef = (0..SECOND).flat_map(|k| group2(k).into_iter().map(move |g| (GlyphId16::new(g), k as u16 + 1))).collect();
    let record = |v: i16| {
        let mut r = ValueRecord::new().with_explicit_value_format(ValueFormat::X_ADVANCE);
        r.x_advance = Some(v);
        Class2Record::new(r, ValueRecord::new().with_explicit_value_format(ValueFormat::empty()))
    };
    let rows = (0..=FIRST)
        .map(|i| Class1Record::new((0..=SECOND).map(|j| record(if i == 0 || j == 0 { 0 } else { value(i - 1, j - 1) })).collect()))
        .collect();
    PairPos::Format2(PairPosFormat2::new(coverage, cd1, cd2, rows))
}

fn font_with(lookup: PositionLookup) -> SfntFont {
    let gpos = Gpos::new(ScriptList::default(), FeatureList::default(), PositionLookupList::new(vec![lookup]));
    let names: Vec<String> = (0..GLYPHS).map(name).collect();
    let post = Post::new_v2(names.iter().map(String::as_str));
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&GLYPHS.to_be_bytes());
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"maxp", maxp);
    font.insert(*b"post", write_fonts::dump_table(&post).unwrap());
    font.insert(*b"GPOS", write_fonts::dump_table(&gpos).unwrap());
    font
}

fn pair_font() -> SfntFont {
    font_with(PositionLookup::Pair(Lookup::new(LookupFlag::empty(), vec![original()])))
}

fn extension_font() -> SfntFont {
    let ext = ExtensionSubtable::Pair(ExtensionPosFormat1::new(2, original()));
    font_with(PositionLookup::Extension(Lookup::new(LookupFlag::empty(), vec![ext])))
}

/// The pair subtables of the first lookup.
fn subtables(table: &[u8]) -> Vec<PairPosFormat2> {
    let gpos: Gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(table)).unwrap().to_owned_table();
    let pairs: Vec<PairPos> = match &*gpos.lookup_list.lookups[0] {
        PositionLookup::Pair(l) => l.subtables.iter().map(|s| (**s).clone()).collect(),
        PositionLookup::Extension(l) => l
            .subtables
            .iter()
            .map(|s| match &**s {
                ExtensionSubtable::Pair(e) => (*e.extension).clone(),
                other => panic!("{other:?}"),
            })
            .collect(),
        other => panic!("{other:?}"),
    };
    pairs
        .into_iter()
        .map(|p| match p {
            PairPos::Format2(f) => f,
            PairPos::Format1(_) => panic!("format 1"),
        })
        .collect()
}

/// Each subtable's first-glyph groups by class id (class 0 first) and second-glyph groups from
/// class 1, as group numbers.
fn layout(subs: &[PairPosFormat2]) -> Vec<(Vec<usize>, Vec<usize>)> {
    let group_of = |groups: &dyn Fn(usize) -> Vec<u16>, n: usize, g: GlyphId16| (0..n).find(|&k| groups(k).contains(&g.to_u16())).unwrap();
    subs.iter()
        .map(|f| {
            let mut ones: BTreeMap<u16, BTreeSet<usize>> = BTreeMap::new();
            for g in f.coverage.iter() {
                ones.entry(f.class_def1.get(g)).or_default().insert(group_of(&group1, FIRST, g));
            }
            let mut twos: BTreeMap<u16, BTreeSet<usize>> = BTreeMap::new();
            for (g, c) in f.class_def2.iter() {
                twos.entry(c).or_default().insert(group_of(&group2, SECOND, g));
            }
            let one = |m: BTreeMap<u16, BTreeSet<usize>>| -> Vec<usize> {
                m.into_values()
                    .map(|s| {
                        assert_eq!(s.len(), 1, "a class holds one group");
                        s.into_iter().next().unwrap()
                    })
                    .collect()
            };
            (one(ones), one(twos))
        })
        .collect()
}

/// The x advance a shaper applies to the pair: the first subtable covering `a` decides.
fn kern(subs: &[PairPosFormat2], a: u16, b: u16) -> i16 {
    let (a, b) = (GlyphId16::new(a), GlyphId16::new(b));
    let Some(f) = subs.iter().find(|f| f.coverage.iter().any(|g| g == a)) else { return 0 };
    let row = &f.class1_records[usize::from(f.class_def1.get(a))];
    row.class2_records[usize::from(f.class_def2.get(b))].value_record1.x_advance.unwrap_or(0)
}

fn expected(rows: &[(&[usize], &[usize])]) -> Vec<(Vec<usize>, Vec<usize>)> {
    rows.iter().map(|(a, b)| (a.to_vec(), b.to_vec())).collect()
}

const LEVEL_1: &[(&[usize], &[usize])] = &[(
    &[
        32, 41, 5, 38, 14, 47, 11, 56, 20, 53, 29, 62, 26, 2, 35, 44, 8, 17, 50, 59, 23, 13, 46, 55, 19, 28, 61, 1, 34, 43, 7, 16, 49, 58, 22, 31, 4,
        37, 10, 52, 25, 40, 27, 0, 42, 15, 57, 30, 3, 45, 18, 60, 33, 6, 48, 21, 63, 36, 9, 51, 24, 39, 12, 54,
    ],
    &[
        7, 21, 35, 49, 9, 23, 37, 51, 11, 25, 39, 53, 13, 27, 41, 55, 1, 15, 29, 43, 3, 17, 31, 45, 5, 19, 33, 47, 8, 22, 36, 50, 10, 24, 38, 52, 12,
        26, 40, 54, 0, 14, 28, 42, 2, 16, 30, 44, 4, 18, 32, 46, 6, 20, 34, 48,
    ],
)];
const LEVEL_5: &[(&[usize], &[usize])] = &[
    (
        &[32, 38, 14, 11, 35, 8, 13, 34, 37, 10, 15, 33, 36, 9, 39, 12],
        &[7, 35, 9, 11, 25, 13, 27, 1, 29, 3, 17, 31, 19, 33, 8, 10, 12, 28, 30, 32, 34],
    ),
    (&[41, 47, 44, 46, 43, 40, 42, 45], &[35, 37, 39, 41, 36, 38, 52, 12, 40, 28, 44, 20]),
    (&[29, 26, 28, 31, 25, 27, 30, 24], &[21, 23, 51, 25, 27, 43, 22, 24, 26, 2]),
    (&[53, 50, 55, 49, 52, 48, 51, 54], &[53, 43, 45, 47, 42, 44, 4, 46, 48]),
    (&[5, 56, 62, 2, 59, 61, 1, 7, 58, 4, 0, 57, 3, 60, 6, 63], &[21, 49, 37, 51, 53, 13, 41, 55, 1, 29, 3, 5, 50, 52, 54, 0, 2, 4, 6]),
    (&[20, 17, 23, 19, 16, 22, 18, 21], &[15, 17, 19, 50, 10, 26, 14, 42, 16, 18, 20, 34]),
];
const LEVEL_9: &[(&[usize], &[usize])] = &[
    (&[32, 38, 35, 34, 37, 33, 36, 39], &[35, 11, 27, 29, 3, 31, 19, 33, 28, 30, 32, 34]),
    (&[14, 11, 8, 13, 10, 15, 9, 12], &[7, 9, 11, 25, 13, 1, 17, 33, 8, 10, 12]),
    (&[41, 47, 44, 46, 43, 40, 42, 45], &[35, 37, 39, 41, 36, 38, 52, 12, 40, 28, 44, 20]),
    (&[29, 26, 28, 31, 25, 27, 30, 24], &[21, 23, 51, 25, 27, 43, 22, 24, 26, 2]),
    (&[53, 50, 55, 49, 52, 48, 51, 54], &[53, 43, 45, 47, 42, 44, 4, 46, 48]),
    (&[5, 2, 1, 7, 4, 0, 3, 6], &[49, 41, 1, 3, 5, 0, 2, 4, 6]),
    (&[20, 17, 23, 19, 16, 22, 18, 21], &[15, 17, 19, 50, 10, 26, 14, 42, 16, 18, 20, 34]),
    (&[56, 62, 59, 61, 58, 57, 60, 63], &[21, 49, 37, 51, 53, 13, 55, 29, 5, 50, 52, 54]),
];

#[test]
fn gpos_compaction_splits_class_kerning_as_fonttools_does_and_keeps_every_pair() {
    let font = pair_font();
    let before = subtables(font.get(*b"GPOS").unwrap());
    for (level, rows) in [(1u8, LEVEL_1), (5, LEVEL_5), (9, LEVEL_9)] {
        let c = run(&font, level).unwrap().unwrap();
        let subs = subtables(c.table.as_ref().unwrap());
        assert_eq!(layout(&subs), expected(rows), "level {level}");
        assert_eq!((c.split, c.produced), (1, rows.len()), "level {level}");
        for k1 in 0..FIRST {
            for k2 in 0..SECOND {
                let (a, b) = (group1(k1)[0], *group2(k2).last().unwrap());
                assert_eq!(kern(&subs, a, b), kern(&before, a, b), "level {level}, pair {k1} {k2}");
            }
            // A second glyph in no class.
            assert_eq!(kern(&subs, group1(k1)[0], 150), 0);
        }
    }
    // An extension lookup is split the same way and stays one.
    let c = run(&extension_font(), 5).unwrap().unwrap();
    assert_eq!(layout(&subtables(c.table.as_ref().unwrap())), expected(LEVEL_5));

    // The default run leaves class kerning alone; a level runs the step and makes GPOS smaller.
    let plain = optimize(&font, &OptimizeOptions::default()).unwrap();
    assert!(plain.steps.iter().all(|s| s.step != "gposCompaction"));
    assert_eq!(plain.font.get(*b"GPOS"), font.get(*b"GPOS"));
    let only = OptimizeOptions {
        cff_widths: false,
        subroutines: false,
        variation_store: false,
        gvar: None,
        drop_implied_oncurves: false,
        colr_layer_reuse: false,
        gpos_compaction: 5,
        ..OptimizeOptions::default()
    };
    let r = optimize(&font, &only).unwrap();
    let step = r.steps.iter().find(|s| s.step == "gposCompaction").unwrap();
    assert!(step.applied && step.after < step.before, "{step:?}");
    assert_eq!(subtables(r.font.get(*b"GPOS").unwrap()).len(), LEVEL_5.len());
    assert!(optimize(&font, &OptimizeOptions { gpos_compaction: 10, ..only }).is_err());
}

#[test]
fn gpos_compaction_keeps_a_subtable_with_values_outside_every_second_class() {
    let mut pair = original();
    let PairPos::Format2(f) = &mut pair else { panic!("format 2") };
    f.class1_records[1].class2_records[0].value_record1.x_advance = Some(-5);
    let font = font_with(PositionLookup::Pair(Lookup::new(LookupFlag::empty(), vec![pair])));
    let c = run(&font, 5).unwrap().unwrap();
    assert_eq!(c.table, None);
    assert_eq!(c.notes.len(), 1, "{:?}", c.notes);
    let r = optimize(&font, &OptimizeOptions { gpos_compaction: 5, ..OptimizeOptions::default() }).unwrap();
    assert_eq!(r.font.get(*b"GPOS"), font.get(*b"GPOS"));
}
