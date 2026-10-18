//! `diff` on fonts built in code: presence, bytes and first offset, the hex and text diffs, the
//! limits, and the two normalisations.

use read_fonts::types::{GlyphId16, NameId, Tag as RTag};
use write_fonts::tables::gpos::{
    Class1Record, Class2Record, Gpos, PairPos, PairSet, PairValueRecord, PositionLookup, PositionLookupList, ValueRecord,
};
use write_fonts::tables::layout::{
    ClassDef, ClassRangeRecord, CoverageTable, Feature, FeatureList, FeatureRecord, LangSys, Lookup, LookupFlag, Script, ScriptList, ScriptRecord,
};
use write_fonts::tables::name::{Name, NameRecord};

use super::compiler::testing::{normalize_tree, strip, unique_id};
use super::compiler::write_tree;
use super::*;
use crate::sfnt::Flavour;
use crate::ttx::Element;

fn dump<T: write_fonts::FontWrite + write_fonts::validate::Validate>(table: &T) -> Vec<u8> {
    write_fonts::dump_table(table).unwrap()
}

/// A `head` table of 54 bytes with the magic number, long `loca` and `checkSumAdjustment` set.
fn head(checksum: u32) -> Vec<u8> {
    let mut h = vec![0u8; 54];
    h[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    h[8..12].copy_from_slice(&checksum.to_be_bytes());
    h[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    h[18..20].copy_from_slice(&1000u16.to_be_bytes());
    h[50..52].copy_from_slice(&1i16.to_be_bytes());
    h
}

/// A `name` table of Windows English records.
fn name(records: &[(u16, &str)]) -> Vec<u8> {
    dump(&Name::new(records.iter().map(|(id, s)| NameRecord::new(3, 1, 0x409, NameId::new(*id), s.to_string().into())).collect()))
}

fn font(tables: Vec<(&[u8; 4], Vec<u8>)>) -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    for (tag, data) in tables {
        font.insert(*tag, data);
    }
    font
}

fn with(mode: DiffMode) -> DiffOptions {
    DiffOptions { mode, ..DiffOptions::default() }
}

fn entry<'a>(diffs: &'a [TableDiff], name: &str) -> &'a TableDiff {
    diffs.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no entry {name}: {diffs:?}"))
}

#[test]
fn presence_bytes_and_first_offset() {
    let mut changed = head(0);
    changed[20] = 7;
    let a = font(vec![(b"head", head(0)), (b"zzzz", vec![1, 2, 3]), (b"cvt ", vec![0, 1])]);
    let b = font(vec![(b"head", changed), (b"yyyy", vec![4]), (b"cvt ", vec![0, 1])]);

    let diffs = diff(&a, &b, &with(DiffMode::Bytes)).unwrap();
    let names: Vec<&str> = diffs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["cvt", "head", "yyyy", "zzzz"], "one entry per table of either, in tag order");
    assert_eq!(entry(&diffs, "cvt").status, DiffStatus::Same);
    let h = entry(&diffs, "head");
    assert_eq!((h.status, h.first_offset, h.bytes_equal), (DiffStatus::Different, Some(20), false));
    assert!(h.lines.is_empty());
    assert_eq!((entry(&diffs, "zzzz").status, entry(&diffs, "zzzz").size_a), (DiffStatus::OnlyFirst, Some(3)));
    assert_eq!((entry(&diffs, "yyyy").status, entry(&diffs, "yyyy").size_b), (DiffStatus::OnlySecond, Some(1)));
    assert!(!identical(&diffs));

    // A prefix differs at the shorter length.
    let p = diff(&font(vec![(b"zzzz", vec![1, 2])]), &font(vec![(b"zzzz", vec![1, 2, 3])]), &with(DiffMode::Tables)).unwrap();
    assert_eq!(p[0].first_offset, Some(2));

    let only = diff(&a, &b, &DiffOptions { tables: vec![*b"head"], ..with(DiffMode::Tables) }).unwrap();
    assert_eq!(only.len(), 1);
    let skipped = diff(&a, &b, &DiffOptions { skip: vec![*b"head", *b"zzzz", *b"yyyy"], ..with(DiffMode::Tables) }).unwrap();
    assert!(identical(&skipped));
    let both = DiffOptions { tables: vec![*b"head"], skip: vec![*b"cvt "], ..DiffOptions::default() };
    assert!(matches!(diff(&a, &b, &both), Err(TablesError::BadOptions(_))));
    assert!(identical(&diff(&a, &a, &DiffOptions::default()).unwrap()), "a font is identical to itself");
}

#[test]
fn hex_diff_shows_the_changed_row_with_context() {
    let a: Vec<u8> = (0..64).collect();
    let mut b = a.clone();
    b[20] = 0xFF;
    let diffs = diff(&font(vec![(b"zzzz", a)]), &font(vec![(b"zzzz", b)]), &with(DiffMode::Hex)).unwrap();
    let d = &diffs[0];
    assert_eq!(d.lines[0], "@@ -1,4 +1,4 @@");
    assert_eq!(d.lines.len(), 6, "{:#?}", d.lines);
    assert_eq!(d.lines[1], " 00000000  00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f  |................|");
    assert!(d.lines[2].starts_with("-00000010  10 11 12 13 14 "), "{}", d.lines[2]);
    assert!(d.lines[3].starts_with("+00000010  10 11 12 13 ff "), "{}", d.lines[3]);
    assert_eq!(d.truncated, 0);
}

#[test]
fn hex_diff_stops_at_its_limit_unless_all() {
    let rows = MAX_DIFF_HEX / 16 + 100;
    let a = vec![0u8; rows * 16];
    let b = vec![1u8; rows * 16];
    let fa = font(vec![(b"zzzz", a)]);
    let fb = font(vec![(b"zzzz", b)]);
    let capped = diff(&fa, &fb, &with(DiffMode::Hex)).unwrap();
    let changed = capped[0].lines.iter().filter(|l| l.starts_with('-') || l.starts_with('+')).count();
    assert_eq!(changed, MAX_DIFF_HEX / 16);
    assert_eq!(capped[0].truncated, 2 * rows - MAX_DIFF_HEX / 16);
    let all = diff(&fa, &fb, &DiffOptions { all: true, ..with(DiffMode::Hex) }).unwrap();
    assert_eq!(all[0].truncated, 0);
    assert_eq!(all[0].lines.iter().filter(|l| l.starts_with('-')).count(), rows);
}

#[test]
fn text_diff_of_decoded_tables() {
    let a = font(vec![(b"name", name(&[(1, "Family"), (2, "Regular")]))]);
    let b = font(vec![(b"name", name(&[(1, "Family"), (2, "Bold")]))]);
    let diffs = diff(&a, &b, &DiffOptions::default()).unwrap();
    let d = entry(&diffs, "name");
    assert_eq!(d.status, DiffStatus::Different);
    assert!(d.lines.iter().any(|l| l.starts_with('-') && l.contains("Regular")), "{:#?}", d.lines);
    assert!(d.lines.iter().any(|l| l.starts_with('+') && l.contains("Bold")), "{:#?}", d.lines);
    assert!(d.lines.iter().any(|l| l.starts_with(' ') && l.contains("Family")), "context lines: {:#?}", d.lines);
    let ratio = d.similarity.unwrap();
    assert!(ratio > 0.5 && ratio < 1.0, "{ratio}");

    // Verbose: every line of the table.
    let verbose = diff(&a, &b, &DiffOptions { context: usize::MAX, keep_texts: true, ..DiffOptions::default() }).unwrap();
    let (ta, _) = verbose[0].texts.clone().unwrap();
    let shown = verbose[0].lines.iter().filter(|l| !l.starts_with('+') && !l.starts_with("@@")).count();
    assert_eq!(shown, ta.lines().count());

    // Equal bytes are not decoded unless asked, and then show no difference.
    let same = diff(&a, &a, &DiffOptions { compare_identical: true, keep_texts: true, ..DiffOptions::default() }).unwrap();
    assert_eq!((same[0].status, same[0].similarity), (DiffStatus::Same, Some(1.0)));
    assert!(same[0].texts.is_some());
}

#[test]
fn compiler_normalisation_hides_checksums_and_version_stamps() {
    let a = font(vec![(b"head", head(0x1234_5678)), (b"name", name(&[(1, "Family"), (5, "Version 1.000;FontCraft 0.9.1")]))]);
    let b = font(vec![(b"head", head(0x0BAD_F00D)), (b"name", name(&[(1, "Family"), (5, "Version 1.000")]))]);
    let plain = diff(&a, &b, &DiffOptions::default()).unwrap();
    assert_eq!(entry(&plain, "head").status, DiffStatus::Different);
    assert_eq!(entry(&plain, "name").status, DiffStatus::Different);
    let normalized = diff(&a, &b, &with(DiffMode::NormalizeCompiler)).unwrap();
    assert!(identical(&normalized), "{normalized:#?}");
    assert!(!entry(&normalized, "head").bytes_equal, "the bytes still differ");
}

/// A simple glyph: one contour of three on-curve points given as coordinate deltas, with a fixed
/// bounding box.
fn triangle(dx: [i16; 3], dy: [i16; 3]) -> Vec<u8> {
    let mut g = Vec::new();
    for v in [1i16, 0, 0, 100, 100, 2, 0] {
        g.extend_from_slice(&v.to_be_bytes());
    }
    g.extend_from_slice(&[1, 1, 1]);
    for v in dx.iter().chain(&dy) {
        g.extend_from_slice(&v.to_be_bytes());
    }
    g.push(0);
    g
}

fn glyf_font(glyph: Vec<u8>) -> SfntFont {
    let mut maxp = 0x5000u32.to_be_bytes().to_vec();
    maxp.extend_from_slice(&1u16.to_be_bytes());
    let mut loca = 0u32.to_be_bytes().to_vec();
    loca.extend_from_slice(&(glyph.len() as u32).to_be_bytes());
    font(vec![(b"head", head(0)), (b"maxp", maxp), (b"loca", loca), (b"glyf", glyph)])
}

#[test]
fn off_by_one_coordinates_count_only_past_the_budget() {
    let a = glyf_font(triangle([0, 100, 0], [0, 0, 100]));
    let b = glyf_font(triangle([0, 101, -1], [0, 0, 100]));
    let tight = diff(&a, &b, &DiffOptions { tables: vec![*b"glyf"], ..with(DiffMode::NormalizeCompiler) }).unwrap();
    assert_eq!(tight[0].status, DiffStatus::Different, "0.1 % of six coordinates allows none");
    let loose = DiffOptions { tables: vec![*b"glyf"], off_by_one_budget: 100.0, ..with(DiffMode::NormalizeCompiler) };
    let loose = diff(&a, &b, &loose).unwrap();
    assert_eq!(loose[0].status, DiffStatus::Same, "{:#?}", loose[0].lines);
    assert!(matches!(diff(&a, &b, &DiffOptions { off_by_one_budget: f64::NAN, ..DiffOptions::default() }), Err(TablesError::BadOptions(_))));
}

fn g(id: u16) -> GlyphId16 {
    GlyphId16::new(id)
}

fn kern_font(pair: PairPos) -> SfntFont {
    let lookups = PositionLookupList::new(vec![PositionLookup::Pair(Lookup::new(LookupFlag::empty(), vec![pair]))]);
    let features = FeatureList::new(vec![FeatureRecord::new(RTag::new(b"kern"), Feature::new(None, vec![0]))]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(RTag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0])), vec![]))]);
    font(vec![
        (b"maxp", dump(&write_fonts::tables::maxp::Maxp::new(3))),
        (b"post", dump(&write_fonts::tables::post::Post::new_v2([".notdef", "A", "V"]))),
        (b"GPOS", dump(&Gpos::new(scripts, features, lookups))),
    ])
}

#[test]
fn compiler_normalisation_compares_kerning_through_the_normaliser() {
    let adv = |x: i16| ValueRecord::new().with_x_advance(x);
    let by_glyph =
        PairPos::format_1(CoverageTable::format_1(vec![g(1)]), vec![PairSet::new(vec![PairValueRecord::new(g(2), adv(-80), ValueRecord::new())])]);
    let by_class = PairPos::format_2(
        CoverageTable::format_1(vec![g(1)]),
        ClassDef::format_2(vec![ClassRangeRecord::new(g(1), g(1), 1)]),
        ClassDef::format_2(vec![ClassRangeRecord::new(g(2), g(2), 1)]),
        vec![
            Class1Record::new(vec![Class2Record::new(adv(0), ValueRecord::new()), Class2Record::new(adv(0), ValueRecord::new())]),
            Class1Record::new(vec![Class2Record::new(adv(0), ValueRecord::new()), Class2Record::new(adv(-80), ValueRecord::new())]),
        ],
    );
    let (a, b) = (kern_font(by_glyph), kern_font(by_class.clone()));
    let diffs = diff(&a, &b, &with(DiffMode::NormalizeCompiler)).unwrap();
    let names: Vec<&str> = diffs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["GPOS", "GPOS=normalized", "maxp", "post"]);
    assert_eq!(entry(&diffs, "GPOS=normalized").status, DiffStatus::Same, "the same pair, stored two ways");

    let mut other = by_class;
    if let PairPos::Format2(f) = &mut other {
        f.class1_records[1].class2_records[1] = Class2Record::new(adv(-90), ValueRecord::new());
    }
    let diffs = diff(&a, &kern_font(other), &with(DiffMode::NormalizeCompiler)).unwrap();
    let n = entry(&diffs, "GPOS=normalized");
    assert_eq!(n.status, DiffStatus::Different);
    assert!(n.lines.contains(&"-A -80 V".to_string()) && n.lines.contains(&"+A -90 V".to_string()), "{:#?}", n.lines);
    assert!(diff(&a, &a, &with(DiffMode::Text)).unwrap().iter().all(|d| d.name != "GPOS=normalized"), "only the compiler mode adds it");
}

#[test]
fn dump_normalisation_hides_checksums() {
    let a = font(vec![(b"head", head(1))]);
    let b = font(vec![(b"head", head(2))]);
    match diff(&a, &b, &with(DiffMode::NormalizeDump)) {
        // The normalised dump belongs to the text views; until they land this mode says so.
        Err(TablesError::NotBuilt(_)) => {}
        other => assert!(identical(&other.unwrap()), "checksums are left out of the normalised dump"),
    }
}

fn el(name: &str, attributes: &[(&str, &str)], children: Vec<Element>) -> Element {
    Element { name: name.into(), attributes: attributes.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), children, text: String::new() }
}

fn value(name: &str, v: &str) -> Element {
    el(name, &[("value", v)], vec![])
}

#[test]
fn layout_trees_lose_order_wrappers_and_normalised_lookups() {
    let pair_lookup = |index: &str| {
        el(
            "Lookup",
            &[("index", index)],
            vec![
                value("LookupType", "9"),
                value("LookupFlag", "0"),
                el(
                    "ExtensionPos",
                    &[("index", "0"), ("Format", "1")],
                    vec![value("ExtensionLookupType", "2"), el("PairPos", &[("Format", "1")], vec![])],
                ),
            ],
        )
    };
    let single = el(
        "Lookup",
        &[("index", "1")],
        vec![
            value("LookupType", "9"),
            el(
                "ExtensionPos",
                &[("index", "0"), ("Format", "1")],
                vec![value("ExtensionLookupType", "1"), el("SinglePos", &[("Format", "1")], vec![])],
            ),
        ],
    );
    let mut gpos = el(
        "GPOS",
        &[],
        vec![
            el(
                "FeatureList",
                &[],
                vec![el(
                    "FeatureRecord",
                    &[("index", "0")],
                    vec![el(
                        "Feature",
                        &[],
                        vec![
                            el("FeatureParamsSize", &[], vec![]),
                            el("LookupListIndex", &[("index", "0"), ("value", "1")], vec![]),
                            el("LookupListIndex", &[("index", "1"), ("value", "0")], vec![]),
                            value("UINameID", "256"),
                        ],
                    )],
                )],
            ),
            el("LookupList", &[], vec![pair_lookup("0"), single]),
        ],
    );
    normalize_tree(&mut gpos, *b"GPOS", &[("256", "Fancy")], true);
    let text = write_tree(&gpos).unwrap();
    let expected = r#"<GPOS>
  <FeatureList>
    <FeatureRecord index="0">
      <Feature>
        <FeatureParamsSize/>
        <LookupListIndex index="0" value="0"/>
        <LookupListIndex index="1" value="1"/>
        <UINameID value="Fancy"/>
      </Feature>
    </FeatureRecord>
  </FeatureList>
  <LookupList>
    <Lookup index="0">
      <LookupType value="2"/>
    </Lookup>
    <Lookup index="1">
      <LookupType value="1"/>
      <SinglePos index="0" Format="1"/>
    </Lookup>
  </LookupList>
</GPOS>
"#;
    assert_eq!(text, expected);
}

#[test]
fn name_records_are_sorted_with_user_ids_and_vendor_blanked() {
    let rec = |p: &str, id: &str, text: &str| {
        let mut e = el(
            "namerecord",
            &[("nameID", id), ("platformID", p), ("platEncID", if p == "1" { "0" } else { "1" }), ("langID", if p == "1" { "0x0" } else { "0x409" })],
            vec![],
        );
        e.text = text.into();
        e
    };
    let mut t = el("name", &[], vec![rec("3", "300", "Wide"), rec("1", "1", "Mac"), rec("3", "3", "1.000;NONE;Fam-Regular"), rec("3", "1", "Fam")]);
    normalize_tree(&mut t, *b"name", &[], false);
    let ids: Vec<(&str, &str)> = t.children.iter().map(|r| (r.attribute("nameID").unwrap(), r.text.as_str())).collect();
    assert_eq!(ids, [("1", "Fam"), ("3", "1.000;    ;Fam-Regular"), ("USER_ID", "Wide")], "Mac Roman English dropped, sorted by ID text");

    assert_eq!(strip("Version 2.000;fontc 0.3.1;extra"), "Version 2.000;extra");
    assert_eq!(strip("Version 2.000;FontCraft 1.2 tail"), "Version 2.000 tail");
    assert_eq!(strip("Version 2.000;fontc is great"), "Version 2.000;fontc is great", "only a stamp followed by a version number");
    assert_eq!(unique_id("1.000;;Fam"), "1.000;    ;Fam");
    assert_eq!(unique_id("1.000;ABCD;Fam"), "1.000;ABCD;Fam");
}

#[test]
fn line_scripts_rebuild_the_second_side_even_past_the_work_limit() {
    let a: Vec<u32> = (0..300).map(|i| i % 17).collect();
    let b: Vec<u32> = (0..280).map(|i| (i * 7) % 19).collect();
    for work in [usize::MAX, 10] {
        let runs = lines::script(a.len(), b.len(), |i, j| a.get(i) == b.get(j), work);
        let (mut i, mut j, mut rebuilt) = (0usize, 0usize, Vec::new());
        for (op, len) in runs {
            match op {
                lines::Op::Equal => {
                    assert_eq!(a[i..i + len], b[j..j + len]);
                    rebuilt.extend_from_slice(&a[i..i + len]);
                    i += len;
                    j += len;
                }
                lines::Op::Delete => i += len,
                lines::Op::Insert => {
                    rebuilt.extend_from_slice(&b[j..j + len]);
                    j += len;
                }
            }
        }
        assert_eq!((i, j), (a.len(), b.len()));
        assert_eq!(rebuilt, b, "work limit {work}");
    }
    assert_eq!(lines::quick_ratio(&["a", "b", "c"], &["c", "b", "x", "y"]), 4.0 / 7.0);
}
