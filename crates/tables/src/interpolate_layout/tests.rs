//! Against fontTools' `interpolate_layout` on the TTX masters of its varLib tests: each expected
//! digest is the SHA-256 of fontTools' `GPOS` dump of the instance (masters compiled from TTX,
//! the instance saved and read again), without the `<?xml` and `<ttFont` lines. The weights are
//! fontTools' master scalars at the location named in the test.

use super::*;
use crate::sfnt::Flavour;
use crate::ttx::{DumpOptions, ImportOptions};

fn data() -> Option<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests/varLib/data");
    root.is_dir().then_some(root)
}

fn masters(root: &std::path::Path, dir: &str, stems: &[&str]) -> Vec<SfntFont> {
    stems
        .iter()
        .map(|s| {
            let text = std::fs::read_to_string(root.join(dir).join(format!("{s}.ttx"))).unwrap();
            crate::ttx::import(SfntFont::new(Flavour::TrueType), &text, &ImportOptions::default()).unwrap()
        })
        .collect()
}

fn gpos_digest(font: &SfntFont) -> (String, String) {
    let text = crate::ttx::dump(font, &DumpOptions { tables: vec![GPOS], ..Default::default() }).unwrap();
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.starts_with("<ttFont") && !l.starts_with("<?xml")).map(str::trim_end).collect();
    while lines.last() == Some(&"") {
        lines.pop();
    }
    let joined = lines.join("\n");
    (fontcraft_plugins::sha256::hex(joined.as_bytes()), joined)
}

#[test]
fn interpolated_gpos_matches_fonttools_on_corpus_masters() {
    let Some(root) = data() else {
        eprintln!("skipped: corpus not fetched");
        return;
    };
    let family3 = ["TestFamily3-Light", "TestFamily3-Regular", "TestFamily3-SemiBold", "TestFamily3-Bold"];
    let cases: [(&str, &str, &[&str], usize, &[f64], &str); 6] = [
        // Single adjustment (vpal), weight 500 and 250.
        (
            "vpal 500",
            "master_vpal_test",
            &["master_vpal_test_0", "master_vpal_test_1"],
            0,
            &[0.5, 0.5],
            "1a4711928cd4320cef3997dd74f8d33cc27d0ed83b8f20de425d87d02f515e3c",
        ),
        (
            "vpal 250",
            "master_vpal_test",
            &["master_vpal_test_0", "master_vpal_test_1"],
            0,
            &[0.75, 0.25],
            "3e5610657b7acff0e76c86441b005d5f89debf416dd20beb402387dd9c18f932",
        ),
        // Class kerning whose classes differ between masters, Weight 600 and 200.
        (
            "kerning 600",
            "master_kerning_merging",
            &["0", "1", "2"],
            1,
            &[0.0, 0.6, 0.4],
            "67776ccaf3f09e54dfbed2a8e62f9a5a4a3068169a26a4203fdc0961521c2400",
        ),
        (
            "kerning 200",
            "master_kerning_merging",
            &["0", "1", "2"],
            1,
            &[0.6666666666666666, 0.33333333333333337, 0.0],
            "17f52ae66c4edebc24ba52b97a887cbc87c60422577912ee8a2b3dcfaea49e3f",
        ),
        // Pair adjustment and mark-to-base, weight 500 through an axis map.
        (
            "family 3",
            "master_ttx_interpolatable_ttf",
            &family3,
            1,
            &[0.0, 0.7049180327868853, 0.29508196721311475, 0.0],
            "dc0fd59636b74a81b9a9c63c65382d75b2c167bc777ff86d12e7008c5780fcbb",
        ),
        // CFF2 masters, weight 650.
        (
            "cff2",
            "master_cff2",
            &["TestCFF2_ExtraLight", "TestCFF2_Regular", "TestCFF2_Black"],
            1,
            &[0.0, 0.45569620253164556, 0.5443037974683544],
            "bc847d537f91facd3f609f7f2ab122824a74ebd719f088dbc4dfb72cac2e7543",
        ),
    ];
    for (label, dir, stems, base, scalars, expected) in cases {
        let fonts = masters(&root, dir, stems);
        let out = interpolate_layout(&fonts, base, scalars).unwrap_or_else(|e| panic!("{label}: {e}"));
        let (digest, text) = gpos_digest(&out);
        assert_eq!(digest, expected, "{label}: our GPOS dump:\n{text}");
        // Everything but GPOS is the base master's.
        for tag in fonts[base].tags().into_iter().filter(|t| *t != GPOS) {
            assert_eq!(out.get(tag), fonts[base].get(tag), "{label}: {}", String::from_utf8_lossy(&tag));
        }
    }
}

#[test]
fn incompatible_masters_are_refused_as_fonttools_refuses_them() {
    let Some(root) = data() else {
        eprintln!("skipped: corpus not fetched");
        return;
    };
    let cases: [(&str, &[&str]); 4] = [
        ("master_incompatible_arrays", &["IncompatibleArrays-Regular", "IncompatibleArrays-Bold"]),
        ("master_incompatible_lookup_types", &["IncompatibleLookupTypes-Regular", "IncompatibleLookupTypes-Bold"]),
        ("master_incompatible_features", &["IncompatibleFeatures-Regular", "IncompatibleFeatures-Bold"]),
        // The medium master has no GPOS table.
        ("master_ttx_interpolatable_ttf", &["SparseMasters-Regular", "SparseMasters-Medium", "SparseMasters-Bold"]),
    ];
    for (dir, stems) in cases {
        let fonts = masters(&root, dir, stems);
        let mut scalars = vec![0.0; fonts.len()];
        scalars[0] = 0.5;
        scalars[fonts.len() - 1] = 0.5;
        assert!(interpolate_gpos(&fonts, 0, &scalars).is_err(), "{dir}");
    }
}

#[test]
fn the_classifier_refines_and_orders_classes_as_fonttools_does() {
    let names: Vec<String> = ["a", "b", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
    let ctx = Ctx { scalars: vec![1.0], remap: vec![None], names, base: 0 };
    // Master classes {a,b,c} {d} and {a,b} {c,d,e}: the refinement is {a,b} {c} {d} {e},
    // largest first, then by sorted names.
    let classes = pair::classify_for_test(&ctx, &[&[0, 1, 2], &[3], &[0, 1], &[2, 3, 4]]);
    assert_eq!(classes, vec![vec![0, 1], vec![2], vec![3], vec![4]]);
}

#[test]
fn values_round_half_up_after_weighting() {
    let ctx = Ctx { scalars: vec![0.5, 0.5], remap: vec![None, None], names: Vec::new(), base: 0 };
    assert_eq!(ctx.value([-3, 0]).unwrap(), -1);
    assert_eq!(ctx.value([3, 0]).unwrap(), 2);
    assert!(ctx.value([32767, 32767 * 3]).is_err());
}

// ---------- hostile and large inputs ----------

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// A font of `glyphs` glyphs (version 0.5 `maxp`) holding `gpos`.
fn font_with_gpos(glyphs: u16, gpos: Vec<u8>) -> SfntFont {
    let mut maxp = vec![0, 0, 0x50, 0];
    be16(&mut maxp, glyphs);
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"maxp", maxp);
    font.insert(GPOS, gpos);
    font
}

/// One extension lookup holding two class pair adjustment subtables over glyphs 1 to `n`:
/// X gives each glyph a first class of its own and has one column; Y has an empty first class
/// definition, so one row, of `columns` columns. The class definitions and coverages overlap
/// the value records, which keeps every 16-bit offset in range.
fn two_class_kerning_subtables(n: u16, columns: u16) -> Vec<u8> {
    let mut x = Vec::new();
    // Format 2, coverage at 22, x advance only, class definition 1 at 20 and 2 at 16,
    // n + 1 first classes, one second class.
    for v in [2, 22, 0x0004, 0, 20, 16, n + 1, 1] {
        be16(&mut x, v);
    }
    // At 16: an empty format 2 class definition. At 20: format 1 from glyph 1, n classes
    // 1..=n. At 22, sharing its tail: a format 1 coverage of glyphs 1..=n. The first n + 1
    // values are also the n + 1 value records.
    for v in [2, 0, 1, 1, n] {
        be16(&mut x, v);
    }
    for g in 1..=n {
        be16(&mut x, g);
    }
    let mut y = Vec::new();
    // Format 2, coverage at 20, x advance only, both class definitions at 16, one first
    // class, `columns` second classes.
    for v in [2, 20, 0x0004, 0, 16, 16, 1, columns] {
        be16(&mut y, v);
    }
    // At 16: an empty format 2 class definition. At 20: a format 2 coverage of glyphs 1..=n.
    // The rest of the row's values are zero.
    for v in [2, 0, 2, 1, 1, n, 0] {
        be16(&mut y, v);
    }
    for _ in 7..columns {
        be16(&mut y, 0);
    }
    let mut t = Vec::new();
    // Header, empty script and feature lists, a lookup list with one lookup at 18.
    for v in [1, 0, 10, 12, 14, 0, 0, 1, 4] {
        be16(&mut t, v);
    }
    // Lookup at 18: extension, two subtables at 28 and 36.
    for v in [9, 0, 2, 10, 18] {
        be16(&mut t, v);
    }
    let x_at = 44u32;
    let y_at = x_at + u32::try_from(x.len()).unwrap();
    for (at, target) in [(28u32, x_at), (36, y_at)] {
        be16(&mut t, 1);
        be16(&mut t, 2);
        t.extend_from_slice(&(target - at).to_be_bytes());
    }
    t.extend_from_slice(&x);
    t.extend_from_slice(&y);
    t
}

#[test]
fn class_kerning_joins_build_rows_from_the_aligned_columns_only() {
    // 2,000 first classes against a 65,535-column row: copying that row once per first class
    // would take gigabytes.
    let n = 2_000;
    let fonts = vec![font_with_gpos(n + 1, two_class_kerning_subtables(n, 65_535)); 2];
    let out = interpolate_gpos(&fonts, 0, &[0.5, 0.5]).unwrap().unwrap();
    let gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(&out)).unwrap();
    let lookup = gpos.lookup_list().unwrap().lookups().get(0).unwrap();
    let read_fonts::tables::gpos::PositionLookup::Pair(lookup) = lookup else { panic!("not a pair lookup") };
    let read_fonts::tables::gpos::PairPos::Format2(sub) = lookup.subtables().get(0).unwrap() else { panic!("not a class pair subtable") };
    assert_eq!((sub.class1_count(), sub.class2_count()), (n, 1));
    // The same shape at the largest glyph count finishes too (written, or refused by the
    // writer), without copying rows.
    let n = 65_534;
    let fonts = vec![font_with_gpos(n + 1, two_class_kerning_subtables(n, 65_535)); 2];
    let _ = interpolate_gpos(&fonts, 0, &[0.5, 0.5]);
}

#[test]
fn masters_share_one_size_budget() {
    use crate::layout_budget::tests::shared_gpos;
    // About 6.3 million records each: under the limit for one table, over it for four.
    let gpos = shared_gpos(700, 1_000, 3);
    crate::layout_budget::check_gpos(&gpos).unwrap();
    let fonts = vec![font_with_gpos(4, gpos); 4];
    let err = interpolate_gpos(&fonts, 0, &[0.25; 4]).unwrap_err().to_string();
    assert!(err.contains("too large"), "{err}");
}

fn ctx(masters: usize) -> Ctx {
    Ctx { scalars: vec![1.0 / masters as f64; masters], remap: vec![None; masters], names: Vec::new(), base: 0 }
}

#[test]
fn large_pair_sets_merge_without_scanning_the_coverage_per_pair() {
    use write_fonts::tables::gpos::{PairPos, PairSet, PairValueRecord, ValueRecord};
    // Each master kerns every glyph against a second glyph the other lacks, so every pair
    // asks the other master's lookup for its value.
    let n = 50_000u16;
    let master = |second: u16| {
        let cov = CoverageTable::format_1((0..n).map(GlyphId16::new).collect());
        let sets = (0..n)
            .map(|_| PairSet::new(vec![PairValueRecord::new(GlyphId16::new(second), ValueRecord::new().with_x_advance(-10), ValueRecord::new())]))
            .collect();
        PairPos::format_1(cov, sets)
    };
    let (a, b) = (master(n), master(n + 1));
    let out = pair::merge(&ctx(2), &[&a], &[vec![&a], vec![&b]]).unwrap();
    let [PairPos::Format1(t)] = out.as_slice() else { panic!("one format 1 subtable expected") };
    assert_eq!(t.pair_sets.len(), usize::from(n));
    let first = &t.pair_sets[0].pair_value_records;
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].value_record1.x_advance, Some(-5));
}

#[test]
fn large_single_adjustments_merge_without_scanning_the_coverage_per_glyph() {
    use write_fonts::tables::gpos::ValueRecord;
    // Disjoint coverages: every glyph takes the other master's effective value.
    let n = 32_000u16;
    let master = |from: u16| {
        let cov = CoverageTable::format_1((from..from + n).map(GlyphId16::new).collect());
        SinglePos::format_2(cov, (0..n).map(|_| ValueRecord::new().with_x_advance(-10)).collect())
    };
    let (a, b) = (master(0), master(n));
    let out = single::merge(&ctx(2), &[&a], &[vec![&a], vec![&b]]).unwrap();
    let [SinglePos::Format2(t)] = out.as_slice() else { panic!("one format 2 subtable expected") };
    assert_eq!(t.value_records.len(), 2 * usize::from(n));
    assert!(t.value_records.iter().all(|v| v.x_advance == Some(-5)));
}
