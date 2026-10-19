use fontcraft_compile::{Options, compile_ttf};
use fontcraft_model::{Font, Localized, Snippet};

use super::*;

fn entry(name: &str, code: &str) -> Snippet {
    Snippet { name: name.into(), code: code.into(), ..Default::default() }
}

/// A font with the extra glyphs the feature code below names.
fn glyphs() -> Font {
    let mut f = Font::new("Layout Back");
    for name in ["f_i", "a.sc", "b.sc", "c.sc", "a.alt", "acutecomb"] {
        f.add_glyph(name, vec![], 600.0).unwrap();
    }
    f
}

/// The glyph names a compiled font opens with, by glyph id.
fn names(bytes: &[u8]) -> Vec<String> {
    crate::decompile(bytes, None).unwrap().font.glyphs.iter().map(|g| g.name.clone()).collect()
}

fn summary(bytes: &[u8]) -> LayoutSummary {
    layout_summary(bytes, &names(bytes), &[]).unwrap()
}

/// Opens `bytes`, then compiles the opened document again.
fn round_trip(bytes: &[u8]) -> (Decompiled, Vec<u8>) {
    let d = crate::decompile(bytes, None).unwrap();
    let again = compile_ttf(&d.font, &Options::default()).unwrap().bytes;
    (d, again)
}

fn code<'a>(list: &'a [Snippet], name: &str) -> &'a str {
    list.iter().find(|s| s.name == name).map(|s| s.code.as_str()).unwrap_or_else(|| panic!("no entry {name} in {list:#?}"))
}

#[test]
fn reconstructed_feature_code_compiles_back_to_the_same_layout() {
    let mut f = glyphs();
    let features = Arc::make_mut(&mut f.features);
    features.prefixes.push(entry(
        "shared",
        "markClass acutecomb <anchor 0 500> @TOP;\n\
         table GDEF { GlyphClassDef [A a b c f i a.sc b.sc c.sc a.alt], [f_i], [acutecomb], ; } GDEF;\n\
         lookup ALTS { sub a by a.sc; sub b by b.sc; } ALTS;\n\
         lookup SWASH { sub a by a.alt; } SWASH;",
    ));
    features.features.push(entry("liga", "sub f i by f_i;"));
    let mut ss01 = entry("ss01", "lookup ALTS;");
    ss01.labels.push(Localized { language: "dflt".into(), value: "Small a and b".into(), extra: Default::default() });
    features.features.push(ss01);
    features.features.push(entry("salt", "lookup ALTS;"));
    features.features.push(entry("calt", "sub a' lookup SWASH b;"));
    features.features.push(entry("mark", "pos base [a b] <anchor 250 500> mark @TOP;"));
    let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;

    let (d, again) = round_trip(&bytes);
    let fea = &d.font.features;
    let prefixes: Vec<&str> = fea.prefixes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(prefixes, [LANGUAGESYSTEMS, PREFIX]);
    assert!(code(&fea.prefixes, LANGUAGESYSTEMS).starts_with("languagesystem DFLT dflt;"), "{fea:#?}");
    let prefix = code(&fea.prefixes, PREFIX);
    // The lookup two features use, and the one a contextual rule calls, are in the prefix.
    assert!(prefix.contains("sub a by a.sc;") && prefix.contains("sub a by a.alt;"), "{prefix}");
    assert!(prefix.contains("markClass [acutecomb] <anchor 0 500>") && prefix.contains("table GDEF {"), "{prefix}");
    // A lookup only one feature uses is written in the feature, which needs no script statements.
    let liga = code(&fea.features, "liga");
    assert!(liga.starts_with("lookup ") && liga.contains("sub f i by f_i;") && !liga.contains("script"), "{liga}");
    for tag in ["ss01", "salt"] {
        let c = code(&fea.features, tag);
        assert!(c.starts_with("lookup ") && !c.contains('{'), "{tag}: {c}");
    }
    assert!(code(&fea.features, "calt").contains("sub a' lookup "), "{fea:#?}");
    assert!(code(&fea.features, "mark").contains("pos base [a b] <anchor 250 500> mark @MC_"), "{fea:#?}");
    let ss01 = fea.features.iter().find(|s| s.name == "ss01").unwrap();
    assert_eq!(ss01.labels.iter().map(|l| (l.language.as_str(), l.value.as_str())).collect::<Vec<_>>(), [("dflt", "Small a and b")]);
    assert!(!d.warnings.iter().any(|w| w.contains("GSUB") || w.contains("GDEF") || w.contains("feature code")), "{:?}", d.warnings);

    // Compiled again: the same lookups, at the same indices, in the same features.
    assert_eq!(summary(&again), summary(&bytes));
}

#[test]
fn pair_kerning_comes_back_as_kerning_and_contextual_kerning_as_code() {
    let mut f = glyphs();
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -80.0);
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "T", "o", -40.0);
    let features = Arc::make_mut(&mut f.features);
    features.prefixes.push(entry("context", "lookup TIGHT { pos T -20; } TIGHT;"));
    features.features.push(entry("kern", "pos T' lookup TIGHT o;"));
    let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;

    let (d, again) = round_trip(&bytes);
    assert_eq!(d.font.kerning.ltr.get("m01", "A", "V"), Some(-80.0));
    let kern = code(&d.font.features.features, "kern");
    // The contextual rule stays code; the generated kerning goes after it, where the pairs were.
    let (rule, marker) = (kern.find("pos T' lookup "), kern.find(AUTOMATIC_CODE));
    assert!(rule.is_some() && marker > rule, "{kern}");
    assert!(!kern.contains("pos A V"), "{kern}");
    assert!(!d.warnings.iter().any(|w| w.contains("contextual kerning")), "{:?}", d.warnings);
    assert_eq!(summary(&again), summary(&bytes));
}

#[test]
fn kerning_the_document_cannot_hold_stays_feature_code() {
    let mut f = glyphs();
    Arc::make_mut(&mut f.features).features.push(entry("kern", "pos A V <0 10 -80 0>;"));
    let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;

    let (d, again) = round_trip(&bytes);
    // Kerning and code both would kern the pair twice, so only the code is kept.
    assert!(d.font.kerning.is_empty(), "{:?}", d.font.kerning);
    assert!(code(&d.font.features.features, "kern").contains("pos A V <0 10 -80 0>;"), "{:#?}", d.font.features);
    assert!(d.warnings.iter().any(|w| w.contains("stays feature code")), "{:?}", d.warnings);
    assert_eq!(summary(&again), summary(&bytes));
}

#[test]
fn a_feature_after_a_shared_lookup_keeps_the_lookup_order() {
    // liga's lookup comes first, then one ss01 and salt share, then calt's: liga's lookup has
    // to go into the prefix before the shared one, and calt's stays in calt.
    let mut f = glyphs();
    let features = Arc::make_mut(&mut f.features);
    features.features.push(entry("liga", "sub f i by f_i;"));
    features.features.push(entry("ss01", "lookup SHARED { sub a by a.sc; } SHARED;"));
    features.features.push(entry("salt", "lookup SHARED;"));
    features.features.push(entry("calt", "sub b by b.sc;"));
    let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;

    let (d, again) = round_trip(&bytes);
    let fea = &d.font.features;
    let prefix = code(&fea.prefixes, PREFIX);
    assert!(prefix.contains("sub f i by f_i;") && prefix.contains("sub a by a.sc;"), "{prefix}");
    assert!(code(&fea.features, "calt").contains("sub b by b.sc;"), "{fea:#?}");
    assert!(code(&fea.features, "liga").starts_with("lookup ") && !code(&fea.features, "liga").contains('{'), "{fea:#?}");
    assert_eq!(summary(&again), summary(&bytes));
}

#[test]
fn lookups_that_call_each_other_in_a_loop_open_with_the_call_commented_out() {
    use write_fonts::tables::gsub as ws;
    use write_fonts::tables::layout as wl;
    use write_fonts::types::{GlyphId16 as WGid, Tag as WTag};

    let base = compile_ttf(&glyphs(), &Options::default()).unwrap().bytes;
    let n = names(&base);
    let a = WGid::new(n.iter().position(|x| x == "a").unwrap() as u16);
    let call = |target: u16| {
        let cov: wl::CoverageTable = [a].into_iter().collect();
        ws::SubstitutionLookup::ChainContextual(wl::Lookup::new(
            wl::LookupFlag::empty(),
            vec![wl::ChainedSequenceContext::format_3(vec![], vec![cov], vec![], vec![wl::SequenceLookupRecord::new(0, target)]).into()],
        ))
    };
    let gsub = ws::Gsub::new(
        wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0])), vec![]))]),
        wl::FeatureList::new(vec![wl::FeatureRecord::new(WTag::new(b"calt"), wl::Feature::new(None, vec![0]))]),
        ws::SubstitutionLookupList::new(vec![call(1), call(0)]),
    );
    let mut b = write_fonts::FontBuilder::new();
    b.add_table(&gsub).unwrap();
    b.copy_missing_tables(skrifa::raw::FontRef::new(&base).unwrap());
    let looped = b.build();

    let d = crate::decompile(&looped, None).unwrap();
    assert!(!d.font.features.features.is_empty());
    assert!(d.warnings.iter().any(|w| w.contains("cannot be defined before it")), "{:?}", d.warnings);
    let all: String = d.font.features.prefixes.iter().chain(&d.font.features.features).map(|s| s.code.as_str()).collect();
    assert!(all.contains("# sub a' lookup "), "{all}");
}

#[test]
fn a_font_without_features_opens_without_feature_entries() {
    let bytes = compile_ttf(&Font::new("Plain"), &Options::default()).unwrap().bytes;
    let d = crate::decompile(&bytes, None).unwrap();
    assert!(d.font.features.features.is_empty() && d.font.features.classes.is_empty(), "{:#?}", d.font.features);
    assert!(!d.warnings.iter().any(|w| w.contains("feature code")), "{:?}", d.warnings);
}

/// A font with `lookups` in GSUB, `calt` using lookup 0 and `liga` lookup 2.
fn with_gsub(base: &[u8], lookups: Vec<write_fonts::tables::gsub::SubstitutionLookup>) -> Vec<u8> {
    use write_fonts::tables::gsub as ws;
    use write_fonts::tables::layout as wl;
    use write_fonts::types::Tag as WTag;
    let gsub = ws::Gsub::new(
        wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0, 1])), vec![]))]),
        wl::FeatureList::new(vec![
            wl::FeatureRecord::new(WTag::new(b"calt"), wl::Feature::new(None, vec![0])),
            wl::FeatureRecord::new(WTag::new(b"liga"), wl::Feature::new(None, vec![2])),
        ]),
        ws::SubstitutionLookupList::new(lookups),
    );
    let mut b = write_fonts::FontBuilder::new();
    b.add_table(&gsub).unwrap();
    b.copy_missing_tables(skrifa::raw::FontRef::new(base).unwrap());
    b.build()
}

#[test]
fn layout_the_feature_compiler_refuses_opens_as_code_that_exports() {
    use write_fonts::tables::gsub as ws;
    use write_fonts::tables::layout as wl;
    use write_fonts::types::GlyphId16 as WGid;

    let base = compile_ttf(&glyphs(), &Options::default()).unwrap().bytes;
    let n = names(&base);
    let g = |name: &str| WGid::new(n.iter().position(|x| x == name).unwrap() as u16);
    let flag = wl::LookupFlag::empty();
    // calt calls a lookup without subtables; liga has the same components twice in one set.
    let call: wl::CoverageTable = [g("a")].into_iter().collect();
    let calt = ws::SubstitutionLookup::ChainContextual(wl::Lookup::new(
        flag,
        vec![wl::ChainedSequenceContext::format_3(vec![], vec![call], vec![], vec![wl::SequenceLookupRecord::new(0, 1)]).into()],
    ));
    let empty = ws::SubstitutionLookup::Single(wl::Lookup::new(flag, vec![]));
    let lig = |to: &str| ws::Ligature::new(g(to), vec![g("i")]);
    let liga = ws::SubstitutionLookup::Ligature(wl::Lookup::new(
        flag,
        vec![ws::LigatureSubstFormat1::new([g("f")].into_iter().collect(), vec![ws::LigatureSet::new(vec![lig("f_i"), lig("a.alt")])])],
    ));
    let d = crate::decompile(&with_gsub(&base, vec![calt, empty, liga]), None).unwrap();
    let all: String = d.font.features.prefixes.iter().chain(&d.font.features.features).map(|s| format!("{}\n", s.code)).collect();
    assert!(all.contains("ignore sub a';") && all.contains("sub f i by f_i;") && !all.contains("by a.alt"), "{all}");
    compile_ttf(&d.font, &Options::default()).unwrap();

    // A substitution into a glyph id the font does not have: the entries open turned off.
    let past = WGid::new(n.len() as u16 + 5);
    let single =
        |to: WGid| ws::SubstitutionLookup::Single(wl::Lookup::new(flag, vec![ws::SingleSubst::format_2([g("a")].into_iter().collect(), vec![to])]));
    let mut kerned = glyphs();
    Arc::make_mut(&mut kerned.kerning).ltr.set("m01", "A", "V", -80.0);
    let base = compile_ttf(&kerned, &Options::default()).unwrap().bytes;
    let d = crate::decompile(&with_gsub(&base, vec![single(past), single(g("b")), single(g("c"))]), None).unwrap();
    assert!(d.font.features.features.iter().chain(&d.font.features.prefixes).all(|s| s.disabled), "{:#?}", d.font.features);
    assert!(d.warnings.iter().any(|w| w.contains("glyph ids the font does not have")), "{:?}", d.warnings);
    assert_eq!(d.font.kerning.ltr.get("m01", "A", "V"), Some(-80.0));
    compile_ttf(&d.font, &Options::default()).unwrap();
}

#[test]
fn a_rule_whose_every_call_names_a_missing_lookup_opens_as_an_ignore_rule() {
    use write_fonts::tables::gsub as ws;
    use write_fonts::tables::layout as wl;
    use write_fonts::types::GlyphId16 as WGid;

    let base = compile_ttf(&glyphs(), &Options::default()).unwrap().bytes;
    let n = names(&base);
    let g = |name: &str| WGid::new(n.iter().position(|x| x == name).unwrap() as u16);
    let flag = wl::LookupFlag::empty();
    // calt's one rule calls lookup 9, which the lookup list (of three) does not have.
    let call: wl::CoverageTable = [g("a")].into_iter().collect();
    let calt = ws::SubstitutionLookup::ChainContextual(wl::Lookup::new(
        flag,
        vec![wl::ChainedSequenceContext::format_3(vec![], vec![call], vec![], vec![wl::SequenceLookupRecord::new(0, 9)]).into()],
    ));
    let single = ws::SubstitutionLookup::Single(wl::Lookup::new(flag, vec![ws::SingleSubst::format_2([g("a")].into_iter().collect(), vec![g("b")])]));
    let lig = ws::Ligature::new(g("f_i"), vec![g("i")]);
    let liga = ws::SubstitutionLookup::Ligature(wl::Lookup::new(
        flag,
        vec![ws::LigatureSubstFormat1::new([g("f")].into_iter().collect(), vec![ws::LigatureSet::new(vec![lig])])],
    ));
    let d = crate::decompile(&with_gsub(&base, vec![calt, single, liga]), None).unwrap();
    let all: String = d.font.features.prefixes.iter().chain(&d.font.features.features).map(|s| format!("{}\n", s.code)).collect();
    assert!(all.contains("ignore sub a';") && all.contains("does not exist"), "{all}");
    compile_ttf(&d.font, &Options::default()).unwrap();
}
