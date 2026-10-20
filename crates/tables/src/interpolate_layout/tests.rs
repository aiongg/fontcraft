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
    // (label, master folder, master stems, base master, scalars, expected digest)
    type Case<'a> = (&'a str, &'a str, &'a [&'a str], usize, &'a [f64], &'a str);
    let cases: [Case; 6] = [
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
    // The bold master of IncompatibleLookupTypes holds a PairPos subtable in a lookup of type 4:
    // fontTools refuses to compile that master, and the TTX reader refuses to read it.
    let lookup_types = root.join("master_incompatible_lookup_types/IncompatibleLookupTypes-Bold.ttx");
    let text = std::fs::read_to_string(lookup_types).unwrap();
    assert!(crate::ttx::import(SfntFont::new(Flavour::TrueType), &text, &ImportOptions::default()).is_err());
    let cases: [(&str, &[&str]); 3] = [
        ("master_incompatible_arrays", &["IncompatibleArrays-Regular", "IncompatibleArrays-Bold"]),
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
