//! Behaviour of the database as a whole: the examples of 05-font-view.md § 4 and of its rows.

use crate::{Case, Category, Direction, GlyphData, Source, SubCategory, Via};

fn data() -> GlyphData {
    GlyphData::new().unwrap()
}

#[test]
fn production_names_follow_the_rules() {
    let d = data();
    for (name, production) in [
        ("A", "A"),
        ("A-cy", "uni0410"),
        ("A-cy.sc", "uni0410.sc"),
        ("lam_alef-ar", "uni06440627"),
        ("lam_alef-ar.fina", "uni06440627.fina"),
        ("f_f_l", "f_f_l"),
        ("nbspace", "uni00A0"),
        ("idotaccent", "i.loclTRK"),
        ("idotaccent.sc", "i.loclTRK.sc"),
        ("twosevenths", "two_fraction_seven"),
        ("pileOfPoo", "u1F4A9"),
        ("pileOfPoo_pileOfPoo", "u1F4A9_u1F4A9"),
        ("brevecomb_acutecomb.case", "uni03060301.case"),
        ("Ech_Vew-arm.liga", "uni0535054E.liga"),
        ("one_parenleftsmall-han.vert", "uni3021FE59.vert"),
        ("myGlyph", "myGlyph"),
        ("uni4E00", "uni4E00"),
    ] {
        assert_eq!(d.production_name(name, &[]), production, "{name}");
    }
}

#[test]
fn suffixes_inherit_and_plain_names_do_not() {
    let d = data();
    let a = d.lookup("A", &[]);
    let alt = d.lookup("A.001", &[]);
    assert_eq!(alt.category.as_ref().map(|c| c.value), Some(Category::Letter));
    assert_eq!(alt.category.as_ref().map(|c| c.source), Some(Source::Synthesised));
    assert_eq!(alt.script.as_ref().map(|s| s.value.as_str()), a.script.as_ref().map(|s| s.value.as_str()));
    assert_eq!(alt.case.as_ref().map(|c| c.value), Some(Case::Upper));
    assert_eq!(alt.unicode, None);
    let plain = d.lookup("A001", &[]);
    assert_eq!((plain.category, plain.script, plain.case), (None, None, None));
    assert_eq!(d.lookup("a.sc", &[]).case.map(|c| c.value), Some(Case::SmallCaps));
    // The longest known prefix is the base: `po-khmer.below` is a record of its own.
    assert_eq!(d.split_suffix("po-khmer.below.ro"), ("po-khmer.below", Some("ro")));
    assert_eq!(d.lookup("po-khmer.below.ro", &[]).sub_category.map(|c| c.value), Some(SubCategory::Nonspacing));
}

#[test]
fn categories_are_synthesised_like_the_oracles() {
    let d = data();
    let cat = |n: &str| {
        let i = d.lookup(n, &[]);
        i.category.map(|c| (c.value, i.sub_category.map(|s| s.value)))
    };
    use Category::*;
    use SubCategory as S;
    assert_eq!(cat("uni000D"), Some((Separator, None)));
    assert_eq!(cat("fi.alt"), Some((Letter, Some(S::Ligature))));
    assert_eq!(cat("one_two.foo"), Some((Number, Some(S::Ligature))));
    assert_eq!(cat("o_f_f_i"), Some((Letter, Some(S::Ligature))));
    assert_eq!(cat("ain_alefMaksura-ar.fina"), Some((Letter, Some(S::Ligature))));
    assert_eq!(cat("brevecomb_acutecomb.case"), Some((Mark, Some(S::Nonspacing))));
    assert_eq!(cat("macronlowmod_O"), Some((Mark, Some(S::Modifier))));
    assert_eq!(cat("O_dotaccentcomb"), Some((Letter, None)));
    assert_eq!(cat("O_period"), Some((Letter, Some(S::Ligature))));
    assert_eq!(cat("_a"), None);
    assert_eq!(cat("Whata-WEIRDNameLOL_brevecomb"), None);
    assert_eq!(cat("uni07F0"), Some((Mark, Some(S::Nonspacing))));
    assert_eq!(cat("uniE000"), Some((Letter, Some(S::Compatibility))));
    assert_eq!(cat("uniFDFA"), Some((Letter, Some(S::Ligature))));
}

#[test]
fn lookup_order_and_provenance() {
    let d = data();
    let i = d.lookup("uni0410", &[]);
    let f = i.found.unwrap();
    assert_eq!((f.record.name.as_str(), f.via), ("A-cy", Via::ProductionName));
    assert_eq!(i.unicode.map(|u| u.value), Some(0x410));
    // An alternative name gives everything but the code point.
    let i = d.lookup("Acyrillic", &[]);
    assert_eq!(i.found.as_ref().map(|f| f.via), Some(Via::AltName));
    assert_eq!(i.unicode, None);
    assert_eq!(i.production_name.value, "uni0410");
    // By code point when the name is unknown.
    let i = d.lookup("SignU.bn", &[0x09C1]);
    assert_eq!(i.found.as_ref().map(|f| f.via), Some(Via::CodePoint));
    // The production name comes before the code points.
    assert_eq!(d.production_name("triagrt", &[0x25BA, 0x25B6]), "triagrt");
    // Legacy code points are never found by code point, nor assigned.
    assert!(d.lookup("allahlong", &[0xFDFA]).found.is_none());
    assert_eq!(d.lookup("uniFDFA", &[]).unicode, None);
    assert_eq!(d.by_legacy_code_point(0xFB04).map(|r| r.name), Some("f_f_l".into()));
    // Ideographs.
    let i = d.lookup("uni4E00", &[]);
    assert_eq!(i.script.map(|s| s.value).as_deref(), Some("han"));
    assert_eq!(d.nice_name(0x4E00), "uni4E00");
}

#[test]
fn directions() {
    let d = data();
    assert_eq!(d.lookup("alef-ar", &[]).direction.value, Direction::Rtl);
    assert_eq!(d.lookup("lam_alef-ar", &[]).direction.value, Direction::Rtl);
    assert_eq!(d.lookup("alef-hb.ss01", &[]).direction.value, Direction::Rtl);
    assert_eq!(d.lookup("A", &[]).direction.value, Direction::Ltr);
    assert_eq!(d.lookup("A", &[]).direction.source, Source::Default);
    assert!(d.is_rtl_script("hebrew") && !d.is_rtl_script("latin"));
}

#[test]
fn names_convert_both_ways() {
    let d = data();
    assert_eq!(d.nice_name(0xE4), "adieresis");
    assert_eq!(d.nice_name(0x10FFFF), "u10FFFF");
    assert_eq!(d.canonical_name("uni00E4").as_deref(), Some("adieresis"));
    assert_eq!(d.canonical_name("uni0410.sc").as_deref(), Some("A-cy.sc"));
    assert_eq!(d.canonical_name("afii10017").as_deref(), Some("A-cy"));
    assert_eq!(d.canonical_name("uni0644_uni0627").as_deref(), Some("lam_alef-ar"));
    assert_eq!(d.canonical_name("adieresis").as_deref(), Some("adieresis"));
    assert_eq!(d.canonical_name("myGlyph"), None);
    assert_eq!(d.name_to_text("f_f_i.alt"), "ffi");
    assert!(d.is_aglfn_name("Aacute") && !d.is_aglfn_name("A-cy"));
}

#[test]
fn custom_records_come_first() {
    let xml = br#"<glyphData>
        <glyph name="A" category="Mark" subCategory="Nonspacing" production="MagicA" />
        <glyph name="myGlyph" unicode="E000" category="Symbol" altNames="myAlias" sortName="zz" />
    </glyphData>"#;
    let later = br#"<glyphData><glyph name="myGlyph" unicode="E001" category="Number" /></glyphData>"#;
    let d = GlyphData::with_custom_files(&[xml, later]).unwrap();
    let a = d.lookup("A", &[]);
    assert_eq!(a.category.map(|c| (c.value, c.source)), Some((Category::Mark, Source::Custom)));
    assert_eq!(a.production_name.value, "MagicA");
    assert_eq!(d.lookup("A.ss01", &[]).production_name.value, "MagicA.ss01");
    assert_eq!(d.lookup("myGlyph", &[]).category.map(|c| c.value), Some(Category::Number));
    assert_eq!(d.nice_name(0xE001), "myGlyph");
    assert_eq!(d.lookup("whatever", &[0xE001]).found.map(|f| f.record.name), Some("myGlyph".into()));
    assert!(d.search("myGlyph", None, 10).iter().any(|r| r.unicode == Some(0xE001)));
    assert!(GlyphData::with_custom_files(&[b"<glyphData><glyph/></glyphData>"]).is_err());
}

#[test]
fn search_matches_name_description_hex_and_character() {
    let d = data();
    assert!(d.search("adieresis", None, 10).iter().any(|r| r.name == "adieresis"));
    assert!(d.search("ä", None, 10).iter().any(|r| r.name == "adieresis"));
    assert!(d.search("00E4", None, 10).iter().any(|r| r.name == "adieresis"));
    assert!(d.search("diaeresis", None, 100_000).len() > 20);
    let deva = d.search("-deva", Some(crate::SortBy::Unicode), 5);
    assert_eq!(deva.len(), 5);
    assert!(deva.windows(2).all(|w| w[0].unicode <= w[1].unicode));
    assert_eq!(d.search("4E00", None, 10).first().map(|r| r.name.as_str()), Some("uni4E00"));
}

#[test]
fn hostile_and_empty_inputs_are_harmless() {
    let d = data();
    let huge = "a_".repeat(1 << 19);
    let dots = "a.".repeat(1 << 19);
    for name in ["", "_", ".", "-", "__", "..", "a-", "_-", "a_-b", "uni", "u", "\u{FEFF}", huge.as_str(), dots.as_str()] {
        let _ = d.lookup(name, &[0, 0x10FFFF, u32::MAX]);
        let _ = d.production_name(name, &[]);
        let _ = d.canonical_name(name);
        let _ = d.split_ligature(name);
        let _ = d.split_suffix(name);
        let _ = d.name_to_text(name);
        let _ = d.search(name, None, usize::MAX);
        let _ = d.sort_key(name, &[u32::MAX], None, Some(name), true);
        let _ = crate::validate(name, true);
    }
    let many: Vec<u32> = (0..100_000).collect();
    let _ = d.lookup("x", &many);
    for cp in [0, 0xD800, 0x10FFFF, 0x110000, u32::MAX] {
        let _ = d.nice_name(cp);
        let _ = d.by_code_point(cp);
        let _ = d.general_category(cp);
        let _ = d.bidi_class(cp);
        let _ = d.unicode_script(cp);
        let _ = d.script_for_code_point(cp);
    }
    let hundred_mb = vec![b' '; 100 << 20];
    assert!(GlyphData::with_custom_files(&[&hundred_mb]).is_err());
    assert!(GlyphData::with_custom(vec![Vec::new(); crate::MAX_CUSTOM_FILES + 1]).is_err());
}

#[test]
fn the_built_in_tables_load_quickly_and_say_where_they_come_from() {
    let start = std::time::Instant::now();
    let t = crate::tables::Tables::open(include_bytes!("../data/glyphdata.bin")).unwrap();
    let elapsed = start.elapsed();
    assert!(t.record_count() > 30_000);
    assert!(elapsed.as_millis() < 2_000, "loading took {elapsed:?}");
    assert!(data().sources().contains("1f126bd1f07cc6a40561900edf87132ea6233967"));
}

#[test]
fn unicode_properties() {
    let d = data();
    use crate::{BidiClass, GeneralCategory};
    assert_eq!(d.general_category(0x41), GeneralCategory::Lu);
    assert_eq!(d.general_category(0x0301), GeneralCategory::Mn);
    assert_eq!(d.general_category(0x4E00), GeneralCategory::Lo);
    assert_eq!(d.general_category(0xE000), GeneralCategory::Co);
    assert_eq!(d.general_category(0x0378), GeneralCategory::Cn);
    assert_eq!(d.bidi_class(0x05D0), BidiClass::R);
    assert_eq!(d.bidi_class(0x0627), BidiClass::Al);
    assert_eq!(d.bidi_class(0x05FF), BidiClass::R);
    assert_eq!(d.unicode_script(0x0627), Some("Arabic"));
    assert_eq!(d.script_for_code_point(0x0627), Some("arabic"));
    assert_eq!(d.script_for_code_point(0x4E00), Some("han"));
    assert!(d.is_assigned(0x41) && !d.is_assigned(0x0378));
}
