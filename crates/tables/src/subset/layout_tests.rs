//! Layout subsetting on a font built in code. The expected glyph sets, tables and normalised
//! text are pyftsubset's (fontTools 4.66.1, run with --glyph-names) on a font built with
//! fontTools to the same glyph order, `cmap` and layout tables, its result printed by ttxn
//! (AFDKO 5.0.1).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_model::{Component, Font, Path, Shape};
use read_fonts::{FontData, FontRead};
use write_fonts::dump_table;
use write_fonts::tables::gdef as wdef;
use write_fonts::tables::gpos as wpos;
use write_fonts::tables::gsub as wsub;
use write_fonts::tables::layout as wl;
use write_fonts::types::{GlyphId16, Tag as WTag};

use super::super::{SubsetOptions, SubsetResult, subset};
use crate::reorder::glyph_names;
use crate::sfnt::{SfntFont, read};
use crate::text::{Out, ttxn_otl};

/// Glyph name, code point (0 for none).
const GLYPHS: [(&str, u32); 18] = [
    ("A", 0x41),
    ("V", 0x56),
    ("acute", 0xB4),
    ("Aacute", 0xC1),
    ("parenleft", 0x28),
    ("parenright", 0x29),
    ("f", 0x66),
    ("i", 0x69),
    ("f_i", 0),
    ("a", 0x61),
    ("a.sc", 0),
    ("a.alt", 0),
    ("a.calt", 0),
    ("b", 0x62),
    ("c", 0x63),
    ("T", 0x54),
    ("o", 0x6F),
    ("grave", 0x60),
];

fn outlines() -> SfntFont {
    let mut f = Font::new("Layout Subset");
    f.glyphs.retain(|g| g.name == ".notdef");
    for (i, (name, cp)) in GLYPHS.iter().enumerate() {
        let cps = if *cp == 0 { vec![] } else { vec![*cp] };
        f.add_glyph(name, cps, 500.0).unwrap();
        if *name != "Aacute" {
            let x = 10.0 * (i + 1) as f64;
            f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x, 0.0, x + 200.0, 700.0)));
        }
    }
    {
        let mut c = f.glyph_mut("Aacute").unwrap();
        c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        c.layers[0].shapes.push(Shape::Component(Component::new("acute", 200.0, 200.0)));
    }
    let out = compile(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    let mut font = read(&out.bytes, 0).unwrap();
    for tag in [*b"GDEF", *b"GSUB", *b"GPOS", *b"kern"] {
        font.remove(tag);
    }
    font
}

struct Ids(BTreeMap<String, u16>);

impl Ids {
    fn new(font: &SfntFont) -> Self {
        Ids(glyph_names(font).unwrap().into_iter().enumerate().map(|(i, n)| (n, i as u16)).collect())
    }

    fn g(&self, name: &str) -> GlyphId16 {
        GlyphId16::new(self.0[name])
    }

    /// A coverage table of `names` in glyph order.
    fn cov(&self, names: &[&str]) -> wl::CoverageTable {
        let mut ids: Vec<u16> = names.iter().map(|n| self.0[*n]).collect();
        ids.sort_unstable();
        wl::CoverageTable::format_1(ids.into_iter().map(GlyphId16::new).collect())
    }
}

fn feature(tag: &[u8; 4], lookups: Vec<u16>) -> wl::FeatureRecord {
    wl::FeatureRecord::new(WTag::new(tag), wl::Feature::new(None, lookups))
}

fn system(features: &[u16]) -> Option<wl::LangSys> {
    Some(wl::LangSys::new(features.to_vec()))
}

/// GSUB: smcp a → a.sc (0), liga f i → f_i (1), salt a → [a.alt] (2), ccmp f_i → f i (3),
/// calt b a' c calling lookup 5 (4, format 3), a → a.calt in no feature (5). DFLT and latn use
/// every feature; latn TRK liga only.
fn gsub(ids: &Ids) -> Vec<u8> {
    let flag = wl::LookupFlag::empty();
    let single = |from: &str, to: &str| {
        wsub::SubstitutionLookup::Single(wl::Lookup::new(flag, vec![wsub::SingleSubst::format_2(ids.cov(&[from]), vec![ids.g(to)])]))
    };
    let liga = wsub::SubstitutionLookup::Ligature(wl::Lookup::new(
        flag,
        vec![wsub::LigatureSubstFormat1::new(
            ids.cov(&["f"]),
            vec![wsub::LigatureSet::new(vec![wsub::Ligature::new(ids.g("f_i"), vec![ids.g("i")])])],
        )],
    ));
    let alt = wsub::SubstitutionLookup::Alternate(wl::Lookup::new(
        flag,
        vec![wsub::AlternateSubstFormat1::new(ids.cov(&["a"]), vec![wsub::AlternateSet::new(vec![ids.g("a.alt")])])],
    ));
    let multi = wsub::SubstitutionLookup::Multiple(wl::Lookup::new(
        flag,
        vec![wsub::MultipleSubstFormat1::new(ids.cov(&["f_i"]), vec![wsub::Sequence::new(vec![ids.g("f"), ids.g("i")])])],
    ));
    let chain = wsub::SubstitutionLookup::ChainContextual(wl::Lookup::new(
        flag,
        vec![
            wl::ChainedSequenceContext::format_3(
                vec![ids.cov(&["b"])],
                vec![ids.cov(&["a"])],
                vec![ids.cov(&["c"])],
                vec![wl::SequenceLookupRecord::new(0, 5)],
            )
            .into(),
        ],
    ));
    let features = wl::FeatureList::new(vec![
        feature(b"calt", vec![4]),
        feature(b"ccmp", vec![3]),
        feature(b"liga", vec![1]),
        feature(b"salt", vec![2]),
        feature(b"smcp", vec![0]),
    ]);
    let all = [0, 1, 2, 3, 4];
    let scripts = wl::ScriptList::new(vec![
        wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(system(&all), vec![])),
        wl::ScriptRecord::new(
            WTag::new(b"latn"),
            wl::Script::new(system(&all), vec![wl::LangSysRecord::new(WTag::new(b"TRK "), wl::LangSys::new(vec![2]))]),
        ),
    ]);
    let table = wsub::Gsub::new(scripts, features, wl::LookupList::new(vec![single("a", "a.sc"), liga, alt, multi, chain, single("a", "a.calt")]));
    dump_table(&table).unwrap()
}

/// GPOS: kern pairs A V -50 and T o -40 (0), kern classes [T] [o] -80 (1), mark acute and grave
/// on A and o with mark filtering set 1 (2), A +20 in no feature (3). DFLT and latn use kern and
/// mark.
fn gpos(ids: &Ids) -> Vec<u8> {
    let flag = wl::LookupFlag::empty();
    let x = |v: i16| wpos::ValueRecord::new().with_x_advance(v);
    let mut firsts = vec![(ids.g("A"), ids.g("V"), -50), (ids.g("T"), ids.g("o"), -40)];
    firsts.sort_by_key(|p| p.0);
    let pairs = wpos::PositionLookup::Pair(wl::Lookup::new(
        flag,
        vec![wpos::PairPos::format_1(
            wl::CoverageTable::format_1(firsts.iter().map(|p| p.0).collect()),
            firsts.iter().map(|p| wpos::PairSet::new(vec![wpos::PairValueRecord::new(p.1, x(p.2), wpos::ValueRecord::new())])).collect(),
        )],
    ));
    let zero = || wpos::Class2Record::new(x(0), wpos::ValueRecord::new());
    let classes = wpos::PositionLookup::Pair(wl::Lookup::new(
        flag,
        vec![wpos::PairPos::format_2(
            ids.cov(&["T"]),
            wl::ClassDef::format_2(vec![wl::ClassRangeRecord::new(ids.g("T"), ids.g("T"), 1)]),
            wl::ClassDef::format_2(vec![wl::ClassRangeRecord::new(ids.g("o"), ids.g("o"), 1)]),
            vec![
                wpos::Class1Record::new(vec![zero(), zero()]),
                wpos::Class1Record::new(vec![zero(), wpos::Class2Record::new(x(-80), wpos::ValueRecord::new())]),
            ],
        )],
    ));
    let mut marks = vec![(ids.g("acute"), wpos::AnchorTable::format_1(250, 500)), (ids.g("grave"), wpos::AnchorTable::format_1(240, 510))];
    marks.sort_by_key(|m| m.0);
    let mut bases = vec![(ids.g("A"), wpos::AnchorTable::format_1(300, 700)), (ids.g("o"), wpos::AnchorTable::format_1(300, 450))];
    bases.sort_by_key(|b| b.0);
    let mut mark = wl::Lookup::new(
        wl::LookupFlag::USE_MARK_FILTERING_SET,
        vec![wpos::MarkBasePosFormat1::new(
            wl::CoverageTable::format_1(marks.iter().map(|m| m.0).collect()),
            wl::CoverageTable::format_1(bases.iter().map(|b| b.0).collect()),
            wpos::MarkArray::new(marks.iter().map(|m| wpos::MarkRecord::new(0, m.1.clone())).collect()),
            wpos::BaseArray::new(bases.iter().map(|b| wpos::BaseRecord::new(vec![Some(b.1.clone())])).collect()),
        )],
    );
    mark.mark_filtering_set = Some(1);
    let single = wpos::PositionLookup::Single(wl::Lookup::new(flag, vec![wpos::SinglePos::format_1(ids.cov(&["A"]), x(20))]));
    let features = wl::FeatureList::new(vec![feature(b"kern", vec![0, 1]), feature(b"mark", vec![2])]);
    let scripts = wl::ScriptList::new(vec![
        wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(system(&[0, 1]), vec![])),
        wl::ScriptRecord::new(WTag::new(b"latn"), wl::Script::new(system(&[0, 1]), vec![])),
    ]);
    let table = wpos::Gpos::new(scripts, features, wl::LookupList::new(vec![pairs, classes, wpos::PositionLookup::MarkToBase(mark), single]));
    dump_table(&table).unwrap()
}

/// GDEF: A and o bases, f_i a ligature with a caret, acute and grave marks; mark glyph sets
/// {acute} and {grave}.
fn gdef(ids: &Ids) -> Vec<u8> {
    let classes: wl::ClassDef = [("A", 1), ("o", 1), ("f_i", 2), ("acute", 3), ("grave", 3)].iter().map(|(n, c)| (ids.g(n), *c)).collect();
    let carets = wdef::LigCaretList::new(ids.cov(&["f_i"]), vec![wdef::LigGlyph::new(vec![wdef::CaretValue::format_1(250)])]);
    let mut table = wdef::Gdef::new(Some(classes), None, Some(carets), None);
    table.mark_glyph_sets_def = Some(wdef::MarkGlyphSets::new(vec![ids.cov(&["acute"]), ids.cov(&["grave"])])).into();
    dump_table(&table).unwrap()
}

fn layout_font() -> SfntFont {
    let mut font = outlines();
    let ids = Ids::new(&font);
    font.insert(*b"GSUB", gsub(&ids));
    font.insert(*b"GPOS", gpos(&ids));
    font.insert(*b"GDEF", gdef(&ids));
    font
}

fn run(text: &str, edit: impl FnOnce(&mut SubsetOptions)) -> SubsetResult {
    let mut options = SubsetOptions { text: text.into(), glyph_names: true, ..SubsetOptions::default() };
    edit(&mut options);
    subset(&layout_font(), &options).unwrap()
}

fn otl(result: &SubsetResult, tag: [u8; 4]) -> String {
    let mut out = Out::new();
    ttxn_otl::write(&result.font, tag, false, &mut out).unwrap();
    out.text
}

/// Glyph name → GDEF glyph class of the subset.
fn gdef_classes(result: &SubsetResult) -> Vec<(String, u16)> {
    let names = glyph_names(&result.font).unwrap();
    let t = read_fonts::tables::gdef::Gdef::read(FontData::new(result.font.get(*b"GDEF").unwrap())).unwrap();
    let mut out: Vec<(String, u16)> = t.glyph_class_def().unwrap().unwrap().iter().filter(|(_, c)| *c != 0).map(|(g, c)| (names[g.to_u16() as usize].clone(), c)).collect();
    out.sort();
    out
}

/// The glyph names a subset keeps, sorted: the glyph order is the compiler's.
fn kept(result: &SubsetResult) -> Vec<String> {
    let mut v = result.retained.clone();
    v.sort();
    v
}

fn names(list: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|n| n.to_string()).collect();
    v.sort();
    v
}

fn named(list: &[(&str, u16)]) -> Vec<(String, u16)> {
    list.iter().map(|(n, c)| (n.to_string(), *c)).collect()
}

#[test]
fn subset_closes_over_ligatures_and_decompositions_of_the_default_features() {
    let result = run("fia", |_| {});
    assert_eq!(kept(&result), names(&[".notdef", "f", "i", "f_i", "a"]), "liga adds f_i; smcp and salt are not default features");
    let expected = "    languagesystem latn dflt;
    languagesystem DFLT dflt;
    languagesystem latn TRK ;



    feature ccmp {
      lookup ccmp_DFLT_dflt_0 {
        sub f_i by [f i];
      } ccmp_DFLT_dflt_0;

    } ccmp;

    feature liga {
      lookup liga_DFLT_dflt_0 {
        sub f i by f_i;
      } liga_DFLT_dflt_0;

    } liga;

";
    assert_eq!(otl(&result, *b"GSUB"), expected);
    assert_eq!(otl(&result, *b"GPOS"), "    languagesystem DFLT dflt;\n\n\n\n", "GPOS keeps DFLT with an empty default language system");
    assert_eq!(gdef_classes(&result), named(&[("f_i", 2)]));
    let t = read_fonts::tables::gdef::Gdef::read(FontData::new(result.font.get(*b"GDEF").unwrap())).unwrap();
    assert!(t.mark_glyph_sets_def().is_none(), "both mark glyph sets are empty, so they go");
    assert!(t.lig_caret_list().is_some());
}

#[test]
fn subset_runs_the_lookups_a_matched_context_calls() {
    let result = run("bac", |_| {});
    assert_eq!(kept(&result), names(&[".notdef", "a", "a.calt", "b", "c"]));
    let none = run("ba", |_| {});
    assert_eq!(kept(&none), names(&[".notdef", "a", "b"]), "without c the context never matches");
}

#[test]
fn subset_layout_features_star_keeps_every_feature() {
    let result = run("a", |o| o.layout_features = Some(vec!["*".into()]));
    assert_eq!(kept(&result), names(&[".notdef", "a", "a.sc", "a.alt"]));
    let only_salt = run("a", |o| o.layout_features = Some(vec!["salt".into()]));
    assert_eq!(kept(&only_salt), names(&[".notdef", "a", "a.alt"]));
}

#[test]
fn subset_without_layout_closure_only_trims_the_rules() {
    let result = run("fi", |o| o.layout_closure = false);
    assert_eq!(kept(&result), names(&[".notdef", "f", "i"]));
    assert_eq!(otl(&result, *b"GSUB"), "    languagesystem DFLT dflt;\n\n\n\n");
    let t = read_fonts::tables::gsub::Gsub::read(FontData::new(result.font.get(*b"GSUB").unwrap())).unwrap();
    let scripts = t.script_list().unwrap();
    let tags: Vec<WTag> = scripts.script_records().iter().map(|r| r.script_tag()).collect();
    assert_eq!(tags, [WTag::new(b"DFLT"), WTag::new(b"latn")], "GSUB keeps scripts left without features");
    assert_eq!(t.lookup_list().unwrap().lookup_count(), 0);
    assert!(result.font.get(*b"GDEF").is_none());
    assert!(result.dropped.iter().any(|d| &d.tag == b"GDEF" && d.reason == "empty after subsetting"), "GDEF lost every glyph");
}

#[test]
fn subset_layout_uses_the_glyphs_before_components_join() {
    // acute joins only as a component of Aacute, so the mark lookup has no mark left.
    let result = run("AVÁo", |_| {});
    assert_eq!(kept(&result), names(&[".notdef", "A", "V", "acute", "Aacute", "o"]));
    let expected = "    languagesystem latn dflt;
    languagesystem DFLT dflt;



    feature kern {
      lookup kern_DFLT_dflt_0 {
        pos A V -50;
      } kern_DFLT_dflt_0;

    } kern;

";
    assert_eq!(otl(&result, *b"GPOS"), expected);
    assert_eq!(gdef_classes(&result), named(&[("A", 1), ("o", 1)]));
}

#[test]
fn subset_mark_filtering_set_that_is_gone_becomes_ignore_marks() {
    let result = run("Ao\u{B4}", |_| {});
    assert_eq!(kept(&result), names(&[".notdef", "A", "acute", "o"]));
    let expected = "    languagesystem latn dflt;
    languagesystem DFLT dflt;



    # Mark Class definitions *********************************
    mark [acute] <anchor 250 500> acute_mark;

    feature mark {
      lookup mark_DFLT_dflt_0 {
        lookupflag IgnoreMarks;
        pos base [A] <anchor 300 700> mark acute_mark;
        pos base [o] <anchor 300 450> mark acute_mark;
      } mark_DFLT_dflt_0;

    } mark;

";
    assert_eq!(otl(&result, *b"GPOS"), expected);
    let gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(result.font.get(*b"GPOS").unwrap())).unwrap();
    let lookup = gpos.lookup_list().unwrap().lookups().get(0).unwrap();
    let flag = match &lookup {
        read_fonts::tables::gpos::PositionLookup::MarkToBase(l) => (l.lookup_flag().to_bits(), l.mark_filtering_set()),
        _ => panic!("the lookup is not a mark-to-base lookup"),
    };
    assert_eq!(flag, (0x0008, None));
    assert_eq!(gdef_classes(&result), named(&[("A", 1), ("acute", 3), ("o", 1)]));
    let gdef = read_fonts::tables::gdef::Gdef::read(FontData::new(result.font.get(*b"GDEF").unwrap())).unwrap();
    let sets = gdef.mark_glyph_sets_def().unwrap().unwrap();
    assert_eq!(sets.coverages().len(), 1, "the set of grave goes, the set of acute stays");
}

#[test]
fn subset_layout_scripts_keep_the_chosen_scripts_and_languages() {
    let result = run("fia", |o| {
        o.layout_features = Some(vec!["*".into()]);
        o.layout_scripts = Some(vec!["latn".into()]);
    });
    assert_eq!(kept(&result), names(&[".notdef", "f", "i", "f_i", "a", "a.sc", "a.alt"]));
    let expected = "    languagesystem DFLT dflt;
    languagesystem latn dflt;
    languagesystem latn TRK ;



    feature ccmp {
      script latn;
      language dflt;
      lookup ccmp_latn_dflt_0 {
        sub f_i by [f i];
      } ccmp_latn_dflt_0;

    } ccmp;

    feature liga {
      script latn;
      language TRK ;
      lookup liga_latn_TRK_0 {
        sub f i by f_i;
      } liga_latn_TRK_0;

      language dflt;
      lookup liga_latn_TRK_0;
    } liga;

    feature salt {
      script latn;
      language dflt;
      lookup salt_latn_dflt_0 {
        sub a from [a.alt];
      } salt_latn_dflt_0;

    } salt;

    feature smcp {
      script latn;
      language dflt;
      lookup smcp_latn_dflt_0 {
        sub a by a.sc;
      } smcp_latn_dflt_0;

    } smcp;

";
    assert_eq!(otl(&result, *b"GSUB"), expected);

    let trk = run("fia", |o| o.layout_scripts = Some(vec!["latn.TRK".into()]));
    assert_eq!(kept(&trk), names(&[".notdef", "f", "i", "f_i", "a"]));
    let expected = "    languagesystem DFLT dflt;
    languagesystem latn TRK ;



    feature liga {
      script latn;
      language TRK ;
      lookup liga_latn_TRK_0 {
        sub f i by f_i;
      } liga_latn_TRK_0;

    } liga;

";
    assert_eq!(otl(&trk, *b"GSUB"), expected);
    assert_eq!(otl(&trk, *b"GPOS"), "\n\n\n", "GPOS has no latn TRK, so no script is left");
}

#[test]
fn subset_no_hinting_removes_device_tables() {
    let mut font = outlines();
    let ids = Ids::new(&font);
    let value = wpos::ValueRecord::new().with_x_advance(20).with_x_advance_device(wl::DeviceOrVariationIndex::device(12, 13, &[1, -1]));
    let single = wpos::PositionLookup::Single(wl::Lookup::new(wl::LookupFlag::empty(), vec![wpos::SinglePos::format_1(ids.cov(&["A"]), value)]));
    let scripts = wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(system(&[0]), vec![]))]);
    let table = wpos::Gpos::new(scripts, wl::FeatureList::new(vec![feature(b"kern", vec![0])]), wl::LookupList::new(vec![single]));
    font.insert(*b"GPOS", dump_table(&table).unwrap());
    let format = |hinting: bool| {
        let result = subset(&font, &SubsetOptions { text: "A".into(), hinting, ..SubsetOptions::default() }).unwrap();
        let gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(result.font.get(*b"GPOS").unwrap())).unwrap();
        let lookup = gpos.lookup_list().unwrap().lookups().get(0).unwrap();
        let read_fonts::tables::gpos::PositionLookup::Single(l) = lookup else { panic!() };
        let read_fonts::tables::gpos::SinglePos::Format1(s) = l.subtables().get(0).unwrap() else { panic!() };
        s.value_format().bits()
    };
    assert_eq!(format(true), 0x0004 | 0x0040, "the device table stays with hinting");
    assert_eq!(format(false), 0x0004, "only the advance is left");
}

#[test]
fn subset_closure_of_a_context_calling_itself_ends() {
    let mut font = outlines();
    let ids = Ids::new(&font);
    let flag = wl::LookupFlag::empty();
    // Lookup 0 calls itself at the first position, and lookup 1 at the second, which calls 0.
    let context = |calls: u16, at: u16| {
        wsub::SubstitutionLookup::ChainContextual(wl::Lookup::new(
            flag,
            vec![
                wl::ChainedSequenceContext::format_3(
                    vec![],
                    vec![ids.cov(&["a"]), ids.cov(&["b"])],
                    vec![],
                    vec![wl::SequenceLookupRecord::new(at, calls)],
                )
                .into(),
            ],
        ))
    };
    let single = wsub::SubstitutionLookup::Single(wl::Lookup::new(flag, vec![wsub::SingleSubst::format_2(ids.cov(&["b"]), vec![ids.g("c")])]));
    let scripts = wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(system(&[0]), vec![]))]);
    let lookups = vec![context(0, 0), context(2, 1), single];
    let features = wl::FeatureList::new(vec![feature(b"calt", vec![0, 1])]);
    font.insert(*b"GSUB", dump_table(&wsub::Gsub::new(scripts, features, wl::LookupList::new(lookups))).unwrap());
    let result = subset(&font, &SubsetOptions { text: "ab".into(), glyph_names: true, ..SubsetOptions::default() }).unwrap();
    assert_eq!(kept(&result), names(&[".notdef", "a", "b", "c"]), "the nested single substitution runs on b");
}

#[test]
fn damaged_layout_tables_are_errors_or_empty_tables_never_crashes() {
    for (tag, bytes) in [
        (*b"GSUB", vec![0u8, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
        (*b"GSUB", vec![0u8, 1, 0, 0, 0, 10, 0, 12, 0, 14, 0, 1, 0xFF, 0xFF]),
        (*b"GPOS", vec![0u8, 1, 0, 0, 0, 10, 0, 10, 0, 10, 0xFF, 0xFF]),
        (*b"GDEF", vec![0u8, 1, 0, 2, 0, 12, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0]),
        (*b"GSUB", vec![0u8, 1]),
    ] {
        let mut font = outlines();
        font.insert(tag, bytes);
        let _ = subset(&font, &SubsetOptions { text: "fia".into(), ..SubsetOptions::default() });
    }
}
