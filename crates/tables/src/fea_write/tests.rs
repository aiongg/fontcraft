//! Feature list and decompile on fonts built in code. The expected lines were checked by
//! compiling the expected text with fontTools feaLib 4.66.1 against the same glyph order.

use super::*;
use crate::features::{list_features, lookup_kinds};
use crate::sfnt::Flavour;
use write_fonts::dump_table;
use write_fonts::tables::gdef as wgdef;
use write_fonts::tables::gpos as wpos;
use write_fonts::tables::gsub as wsub;
use write_fonts::tables::layout as wl;
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::post::Post;
use write_fonts::types::{GlyphId16, Tag as WTag};

fn g(i: u16) -> GlyphId16 {
    GlyphId16::new(i)
}

fn cov(ids: &[u16]) -> wl::CoverageTable {
    wl::CoverageTable::format_1(ids.iter().map(|i| g(*i)).collect())
}

fn font(names: &[&str], tables: Vec<(Tag, Vec<u8>)>) -> SfntFont {
    let mut f = SfntFont::new(Flavour::TrueType);
    f.insert(*b"maxp", dump_table(&Maxp::new(names.len() as u16)).unwrap());
    f.insert(*b"post", dump_table(&Post::new_v2(names.iter().copied())).unwrap());
    for (t, d) in tables {
        f.insert(t, d);
    }
    f
}

fn feature(tag: &[u8; 4], lookups: Vec<u16>) -> wl::FeatureRecord {
    wl::FeatureRecord::new(WTag::new(tag), wl::Feature::new(None, lookups))
}

const GSUB_NAMES: [&str; 10] = [".notdef", "a", "b", "c", "f", "i", "f_i", "a.alt", "a.sc", "sub"];

/// smcp: a → a.sc (0); liga: f i → f_i (1); salt: a from a.alt a.sc (2); ccmp: f_i → f i (3);
/// calt: b a' c calls lookup 0 (4); and `sub` (a keyword) → a in lookup 0 too.
fn gsub_font() -> SfntFont {
    let flag = wl::LookupFlag::empty();
    let single = wsub::SubstitutionLookup::Single(wl::Lookup::new(flag, vec![wsub::SingleSubst::format_2(cov(&[1, 9]), vec![g(8), g(1)])]));
    let liga = wsub::SubstitutionLookup::Ligature(wl::Lookup::new(
        flag,
        vec![wsub::LigatureSubstFormat1::new(cov(&[4]), vec![wsub::LigatureSet::new(vec![wsub::Ligature::new(g(6), vec![g(5)])])])],
    ));
    let alt = wsub::SubstitutionLookup::Alternate(wl::Lookup::new(
        flag,
        vec![wsub::AlternateSubstFormat1::new(cov(&[1]), vec![wsub::AlternateSet::new(vec![g(7), g(8)])])],
    ));
    let multi = wsub::SubstitutionLookup::Multiple(wl::Lookup::new(
        flag,
        vec![wsub::MultipleSubstFormat1::new(cov(&[6]), vec![wsub::Sequence::new(vec![g(4), g(5)])])],
    ));
    let chain = wsub::SubstitutionLookup::ChainContextual(wl::Lookup::new(
        flag,
        vec![
            wl::ChainedSequenceContext::format_3(vec![cov(&[2])], vec![cov(&[1])], vec![cov(&[3])], vec![wl::SequenceLookupRecord::new(0, 0)]).into(),
        ],
    ));
    let features = wl::FeatureList::new(vec![
        feature(b"calt", vec![4]),
        feature(b"ccmp", vec![3]),
        feature(b"liga", vec![1]),
        feature(b"salt", vec![2]),
        feature(b"smcp", vec![0]),
    ]);
    let scripts =
        wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0, 1, 2, 3, 4])), vec![]))]);
    let gsub = wsub::Gsub::new(scripts, features, wl::LookupList::new(vec![single, liga, alt, multi, chain]));
    font(&GSUB_NAMES, vec![(*b"GSUB", dump_table(&gsub).unwrap())])
}

const GPOS_NAMES: [&str; 7] = [".notdef", "A", "V", "T", "o", "acute", "grave"];

/// kern: A V -50 (0), [T] [o] -80 (1); mark: acute and grave on o (2); A +20 with IgnoreMarks,
/// used by no feature (3). DFLT and latn use kern and mark; latn TRK uses only lookup 1.
fn gpos_font() -> SfntFont {
    let flag = wl::LookupFlag::empty();
    let pairs = wpos::PositionLookup::Pair(wl::Lookup::new(
        flag,
        vec![wpos::PairPos::format_1(
            cov(&[1]),
            vec![wpos::PairSet::new(vec![wpos::PairValueRecord::new(g(2), wpos::ValueRecord::new().with_x_advance(-50), wpos::ValueRecord::new())])],
        )],
    ));
    let classes = wpos::PositionLookup::Pair(wl::Lookup::new(
        flag,
        vec![wpos::PairPos::format_2(
            cov(&[3]),
            wl::ClassDef::format_2(vec![wl::ClassRangeRecord::new(g(3), g(3), 1)]),
            wl::ClassDef::format_2(vec![wl::ClassRangeRecord::new(g(4), g(4), 1)]),
            vec![
                wpos::Class1Record::new(vec![
                    wpos::Class2Record::new(wpos::ValueRecord::new(), wpos::ValueRecord::new()),
                    wpos::Class2Record::new(wpos::ValueRecord::new(), wpos::ValueRecord::new()),
                ]),
                wpos::Class1Record::new(vec![
                    wpos::Class2Record::new(wpos::ValueRecord::new().with_x_advance(0), wpos::ValueRecord::new()),
                    wpos::Class2Record::new(wpos::ValueRecord::new().with_x_advance(-80), wpos::ValueRecord::new()),
                ]),
            ],
        )],
    ));
    let mark = wpos::PositionLookup::MarkToBase(wl::Lookup::new(
        flag,
        vec![wpos::MarkBasePosFormat1::new(
            cov(&[5, 6]),
            cov(&[4]),
            wpos::MarkArray::new(vec![
                wpos::MarkRecord::new(0, wpos::AnchorTable::format_1(250, 500)),
                wpos::MarkRecord::new(0, wpos::AnchorTable::format_1(250, 500)),
            ]),
            wpos::BaseArray::new(vec![wpos::BaseRecord::new(vec![Some(wpos::AnchorTable::format_1(300, 450))])]),
        )],
    ));
    let single = wpos::PositionLookup::Single(wl::Lookup::new(
        wl::LookupFlag::IGNORE_MARKS,
        vec![wpos::SinglePos::format_1(cov(&[1]), wpos::ValueRecord::new().with_x_advance(20))],
    ));
    let features = wl::FeatureList::new(vec![feature(b"kern", vec![0, 1]), feature(b"kern", vec![1]), feature(b"mark", vec![2])]);
    let scripts = wl::ScriptList::new(vec![
        wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0, 2])), vec![])),
        wl::ScriptRecord::new(
            WTag::new(b"latn"),
            wl::Script::new(Some(wl::LangSys::new(vec![0, 2])), vec![wl::LangSysRecord::new(WTag::new(b"TRK "), wl::LangSys::new(vec![1]))]),
        ),
    ]);
    let gpos = wpos::Gpos::new(scripts, features, wl::LookupList::new(vec![pairs, classes, mark, single]));
    let gdef = wgdef::Gdef::new(
        Some(wl::ClassDef::format_2(vec![wl::ClassRangeRecord::new(g(1), g(4), 1), wl::ClassRangeRecord::new(g(5), g(6), 3)])),
        None,
        None,
        None,
    );
    font(&GPOS_NAMES, vec![(*b"GPOS", dump_table(&gpos).unwrap()), (*b"GDEF", dump_table(&gdef).unwrap())])
}

fn has(text: &str, line: &str) {
    assert!(text.lines().any(|l| l.trim_end() == line), "missing line {line:?} in:\n{text}");
}

#[test]
fn layout_decompile_features_writes_each_substitution_type() {
    let d = decompile_report(&gsub_font(), &DecompileOptions::default()).unwrap();
    let t = &d.text;
    has(t, "languagesystem DFLT dflt;");
    has(t, "lookup smcp_DFLT_dflt_0 {");
    has(t, "    sub a by a.sc;");
    has(t, "    sub \\sub by a;");
    has(t, "    sub f i by f_i;");
    has(t, "    sub a from [a.alt a.sc];");
    has(t, "    sub f_i by f i;");
    has(t, "    sub b a' lookup smcp_DFLT_dflt_0 c;");
    has(t, "} calt_DFLT_dflt_4;");
    has(t, "feature calt {");
    has(t, "    script DFLT;");
    has(t, "            lookup calt_DFLT_dflt_4;");
    // Lookups keep their list order, so the compiled indices match the font's.
    let pos = |s: &str| t.find(s).unwrap();
    assert!(pos("lookup smcp_DFLT_dflt_0 {") < pos("lookup liga_DFLT_dflt_1 {"));
    assert!(pos("lookup ccmp_DFLT_dflt_3 {") < pos("lookup calt_DFLT_dflt_4 {"));
    assert!(d.unsupported.is_empty(), "{:?}", d.unsupported);
    // Only one feature, and the lookups it reaches.
    let only = DecompileOptions { features: vec![*b"calt"], ..DecompileOptions::default() };
    let t = decompile(&gsub_font(), &only).unwrap();
    // Lookup 0 is reached only through calt, so it takes its table name.
    has(&t, "lookup GSUB_0 {");
    has(&t, "    sub b a' lookup GSUB_0 c;");
    assert!(!t.contains("liga"), "{t}");
}

#[test]
fn layout_decompile_features_writes_positioning_language_systems_and_gdef() {
    let options = DecompileOptions { named_classes: true, ..DecompileOptions::default() };
    let d = decompile_report(&gpos_font(), &options).unwrap();
    let t = &d.text;
    has(t, "languagesystem DFLT dflt;");
    has(t, "languagesystem latn dflt;");
    has(t, "languagesystem latn TRK;");
    has(t, "@acute_grave = [acute grave];");
    has(t, "    GlyphClassDef [A V T o], , @acute_grave, ;");
    has(t, "    pos A V -50;");
    has(t, "    pos [T] [o] -80;");
    has(t, "markClass @acute_grave <anchor 250 500> @MC_mark_DFLT_dflt_2_0_0;");
    has(t, "    pos base o <anchor 300 450> mark @MC_mark_DFLT_dflt_2_0_0;");
    has(t, "lookup GPOS_3 {");
    has(t, "    lookupflag IgnoreMarks;");
    has(t, "    pos A 20;");
    has(t, "    script latn;");
    has(t, "        language TRK exclude_dflt;");
    has(t, "            lookup kern_DFLT_dflt_1;");
    // A glyph map writes development names.
    let mut renamed = DecompileOptions::default();
    renamed.rename.insert("A".into(), "Adev".into());
    let t = decompile(&gpos_font(), &renamed).unwrap();
    has(&t, "    pos Adev V -50;");
    has(&t, "    GlyphClassDef [Adev V T o], , [acute grave], ;");
    // Only GDEF.
    let gdef_only = DecompileOptions { tables: vec![*b"GDEF"], ..DecompileOptions::default() };
    let t = decompile(&gpos_font(), &gdef_only).unwrap();
    assert!(t.contains("table GDEF {") && !t.contains("lookup "), "{t}");
}

#[test]
fn layout_list_features_lists_every_language_system() {
    let list = list_features(&gpos_font()).unwrap();
    let rows: Vec<(String, String, String, Vec<u16>)> =
        list.iter().map(|e| (tag_str(e.script), tag_str(e.language), tag_str(e.feature), e.lookups.clone())).collect();
    assert_eq!(
        rows,
        vec![
            ("DFLT".into(), "dflt".into(), "kern".into(), vec![0, 1]),
            ("DFLT".into(), "dflt".into(), "mark".into(), vec![2]),
            ("latn".into(), "dflt".into(), "kern".into(), vec![0, 1]),
            ("latn".into(), "dflt".into(), "mark".into(), vec![2]),
            ("latn".into(), "TRK ".into(), "kern".into(), vec![1]),
        ]
    );
    assert!(list.iter().all(|e| &e.table == b"GPOS"));
    let kinds = lookup_kinds(&gpos_font(), *b"GPOS").unwrap();
    assert_eq!(kinds.iter().map(|k| k.lookup_type).collect::<Vec<_>>(), vec![2, 2, 4, 1]);
    assert_eq!(kinds.get(3).map(|k| k.flag), Some(8));
    assert!(list_features(&font(&[".notdef"], vec![])).unwrap().is_empty(), "no layout, no features");
}

#[test]
fn damaged_layout_tables_fail_with_a_message() {
    let mut f = gpos_font();
    let gpos = f.get(*b"GPOS").unwrap().to_vec();
    f.insert(*b"GPOS", gpos.get(..12).unwrap().to_vec());
    assert!(matches!(decompile(&f, &DecompileOptions::default()), Err(TablesError::Invalid(_))));
    assert!(list_features(&f).is_err());
    let bad = DecompileOptions { tables: vec![*b"kern"], ..DecompileOptions::default() };
    assert!(matches!(decompile(&gpos_font(), &bad), Err(TablesError::BadOptions(_))));
}
