#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use write_fonts::tables::gdef::CaretValue;
use write_fonts::tables::gpos::{AnchorTable, MarkArray};
use write_fonts::tables::layout::CoverageTable;

use super::*;

fn glyph(name: &str, cps: &[u32], category: Option<&str>, sub: Option<&str>, anchors: &[(&str, f64, f64)]) -> MarkGlyph {
    MarkGlyph {
        name: name.into(),
        codepoints: cps.to_vec(),
        category: category.map(str::to_string),
        sub_category: sub.map(str::to_string),
        opentype_category: None,
        anchors: vec![MasterAnchors {
            master: "m01".into(),
            anchors: anchors.iter().map(|(n, x, y)| MarkAnchor { name: (*n).into(), x: *x, y: *y }).collect(),
        }],
    }
}

fn write(input: &MarkInput, tags: MarkTags) -> MarkFeatures {
    mark_features(&MarkRequest { input, master: "m01", tags, limits: MarkLimits::default(), gsub: None }).unwrap()
}

/// The probe both reference compilers build to the same tables: a base with two anchors, a
/// ligature with two components and a caret, a stacking mark, a mark below, a composite whose
/// anchors propagation gave it, and an Arabic glyph with cursive anchors.
fn probe() -> MarkInput {
    let letter = Some("Letter");
    let mark = (Some("Mark"), Some("Nonspacing"));
    MarkInput {
        glyphs: vec![
            glyph(".notdef", &[], None, None, &[]),
            glyph("A", &[0x41], letter, Some("Uppercase"), &[("top", 300.0, 700.0), ("bottom", 300.0, 0.0)]),
            glyph("V", &[0x56], letter, Some("Uppercase"), &[]),
            glyph("f_i", &[], letter, Some("Ligature"), &[("top_1", 150.0, 700.0), ("top_2", 450.0, 700.0), ("caret_1", 300.0, 0.0)]),
            glyph("acutecomb", &[0x301], mark.0, mark.1, &[("_top", 100.0, 500.0), ("top", 100.0, 800.0)]),
            glyph("dotbelowcomb", &[0x323], mark.0, mark.1, &[("_bottom", 100.0, 0.0)]),
            glyph("Aacute", &[0xC1], letter, Some("Uppercase"), &[("top", 300.0, 1000.0), ("bottom", 300.0, 0.0)]),
            glyph("beh-ar", &[0x628], letter, None, &[("entry", 600.0, 100.0), ("exit", 0.0, 100.0)]),
        ],
        ..Default::default()
    }
}

fn names(input: &MarkInput) -> Vec<String> {
    input.glyphs.iter().map(|g| g.name.clone()).collect()
}

fn at(a: Option<&AnchorTable>) -> String {
    match a {
        Some(AnchorTable::Format1(f)) => format!("{},{}", f.x_coordinate, f.y_coordinate),
        Some(_) => "?".into(),
        None => "-".into(),
    }
}

fn marks(names: &[String], coverage: &CoverageTable, array: &MarkArray) -> String {
    coverage
        .iter()
        .zip(&array.mark_records)
        .map(|(g, r)| format!("{}@{}", names[usize::from(g.to_u16())], at(Some(&*r.mark_anchor))))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One line per lookup subtable: index, type, flags, filter set, then the attachments.
fn describe(gpos: &Gpos, names: &[String]) -> Vec<String> {
    let name = |g: GlyphId16| names[usize::from(g.to_u16())].clone();
    let mut out = Vec::new();
    for (i, lookup) in gpos.lookup_list.lookups.iter().enumerate() {
        let head = |kind: &str, flags: LookupFlag, set: Option<u16>| {
            format!("{i} {kind} {} {}", flags.to_bits(), set.map_or("-".to_string(), |s| s.to_string()))
        };
        match &**lookup {
            PositionLookup::Cursive(l) => {
                for st in &l.subtables {
                    for (g, r) in st.coverage.iter().zip(&st.entry_exit_record) {
                        let h = head("curs", l.lookup_flag, l.mark_filtering_set);
                        out.push(format!("{h} {} {} {}", name(g), at(r.entry_anchor.as_deref()), at(r.exit_anchor.as_deref())));
                    }
                }
            }
            PositionLookup::MarkToBase(l) => {
                for st in &l.subtables {
                    let bases: Vec<String> = st
                        .base_coverage
                        .iter()
                        .zip(&st.base_array.base_records)
                        .map(|(g, r)| format!("{}@{}", name(g), at(r.base_anchors[0].as_deref())))
                        .collect();
                    let h = head("base", l.lookup_flag, l.mark_filtering_set);
                    out.push(format!("{h} {} <- {}", marks(names, &st.mark_coverage, &st.mark_array), bases.join(" ")));
                }
            }
            PositionLookup::MarkToLig(l) => {
                for st in &l.subtables {
                    let ligatures: Vec<String> = st
                        .ligature_coverage
                        .iter()
                        .zip(&st.ligature_array.ligature_attaches)
                        .map(|(g, attach)| {
                            let parts: Vec<String> = attach.component_records.iter().map(|c| at(c.ligature_anchors[0].as_deref())).collect();
                            format!("{}@{}", name(g), parts.join("|"))
                        })
                        .collect();
                    let h = head("lig", l.lookup_flag, l.mark_filtering_set);
                    out.push(format!("{h} {} <- {}", marks(names, &st.mark_coverage, &st.mark_array), ligatures.join(" ")));
                }
            }
            PositionLookup::MarkToMark(l) => {
                for st in &l.subtables {
                    let bases: Vec<String> = st
                        .mark2_coverage
                        .iter()
                        .zip(&st.mark2_array.mark2_records)
                        .map(|(g, r)| format!("{}@{}", name(g), at(r.mark2_anchors[0].as_deref())))
                        .collect();
                    let h = head("mkmk", l.lookup_flag, l.mark_filtering_set);
                    out.push(format!("{h} {} <- {}", marks(names, &st.mark1_coverage, &st.mark1_array), bases.join(" ")));
                }
            }
            _ => out.push(format!("{i} other")),
        }
    }
    out
}

/// One line per language system: its features and their lookups.
fn features(gpos: &Gpos) -> Vec<String> {
    let mut out = Vec::new();
    for record in &gpos.script_list.script_records {
        let mut systems: Vec<(Tag, &LangSys)> = Vec::new();
        if let Some(lang_sys) = record.script.default_lang_sys.as_deref() {
            systems.push((DFLT_LANG, lang_sys));
        }
        for lang in &record.script.lang_sys_records {
            systems.push((lang.lang_sys_tag, &*lang.lang_sys));
        }
        for (lang, sys) in systems {
            let feats: Vec<String> = sys
                .feature_indices
                .iter()
                .map(|i| {
                    let f = &gpos.feature_list.feature_records[usize::from(*i)];
                    format!("{}{:?}", f.feature_tag, f.feature.lookup_list_indices)
                })
                .collect();
            out.push(format!("{}/{} {}", record.script_tag, lang, feats.join(" ")));
        }
    }
    out
}

/// The glyph classes by name. A format 1 class table lists the glyphs between classed glyphs
/// with class 0, which means no class, so those are left out.
fn classes(gdef: &Gdef, names: &[String]) -> Vec<(String, u16)> {
    let mut out: Vec<(String, u16)> = gdef
        .glyph_class_def
        .as_deref()
        .map(|c| c.iter().filter(|&(_, class)| class != 0).map(|(g, class)| (names[usize::from(g.to_u16())].clone(), class)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn marks_ligatures_stacked_marks_and_cursive_glyphs_attach_as_the_reference_compilers_write_them() {
    let input = probe();
    let names = names(&input);
    let out = write(&input, MarkTags::default());
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let gpos = out.gpos.as_ref().unwrap();
    // U+0301's script extensions include scripts that position marks with abvm, so the reference
    // compilers also write its stacking to abvm. `V` has no anchors; `f_i` is a ligature, never
    // a base.
    assert_eq!(
        describe(gpos, &names),
        [
            "0 curs 9 - beh-ar 600,100 0,100",
            "1 mkmk 16 0 acutecomb@100,500 <- acutecomb@100,800",
            "2 base 0 - dotbelowcomb@100,0 <- A@300,0 Aacute@300,0",
            "3 base 0 - acutecomb@100,500 <- A@300,700 Aacute@300,1000",
            "4 lig 0 - acutecomb@100,500 <- f_i@150,700|450,700",
            "5 mkmk 16 0 acutecomb@100,500 <- acutecomb@100,800",
        ]
    );
    assert_eq!(features(gpos), ["DFLT/dflt abvm[1] curs[0] mark[2, 3, 4] mkmk[5]"]);
    assert_eq!((out.lookups, out.curs_lookups), (6, 1));
    // The plain-data form holds the same lookups, in the same order.
    let generated: Vec<(String, String)> = out.generated.lookups.iter().map(|l| (l.feature.to_string(), l.class.clone())).collect();
    let pair = |f: &str, c: &str| (f.to_string(), c.to_string());
    assert_eq!(
        generated,
        [pair("curs", ""), pair("abvm", "top"), pair("mark", "bottom"), pair("mark", "top"), pair("mark", "top"), pair("mkmk", "top")]
    );
    assert_eq!(
        out.generated.lookups[4].rules,
        MarkRules::MarkToLigature {
            marks: vec![(GlyphId16::new(4), (100, 500))],
            ligatures: vec![(GlyphId16::new(3), vec![Some((150, 700)), Some((450, 700))])]
        }
    );

    let gdef = out.gdef.as_ref().unwrap();
    let class = |n: &str, c: u16| (n.to_string(), c);
    assert_eq!(
        classes(gdef, &names),
        [class("A", 1), class("Aacute", 1), class("acutecomb", 3), class("beh-ar", 1), class("dotbelowcomb", 3), class("f_i", 2)]
    );
    let carets = gdef.lig_caret_list.as_deref().unwrap();
    let positions: Vec<Vec<i16>> = carets
        .lig_glyphs
        .iter()
        .map(|g| {
            g.caret_values
                .iter()
                .map(|c| match &**c {
                    CaretValue::Format1(f) => f.coordinate,
                    _ => i16::MIN,
                })
                .collect()
        })
        .collect();
    assert_eq!(
        (carets.coverage.iter().map(|g| names[usize::from(g.to_u16())].clone()).collect::<Vec<_>>(), positions),
        (vec!["f_i".to_string()], vec![vec![300]])
    );
    let sets = gdef.mark_glyph_sets_def.as_deref().unwrap();
    assert_eq!(sets.coverages.len(), 1);
    assert_eq!(sets.coverages[0].iter().map(|g| names[usize::from(g.to_u16())].clone()).collect::<Vec<_>>(), ["acutecomb"]);
}

#[test]
fn feature_code_language_systems_get_the_features_and_limit_the_abvm_scripts() {
    let mut input = probe();
    input.language_systems = vec![("DFLT".into(), "dflt".into()), ("latn".into(), "dflt".into()), ("latn".into(), "TRK".into())];
    let names = names(&input);
    let out = write(&input, MarkTags::default());
    let gpos = out.gpos.as_ref().unwrap();
    // Latin is no abvm script, so with Latin declared nothing goes to abvm.
    assert_eq!(
        describe(gpos, &names),
        [
            "0 curs 9 - beh-ar 600,100 0,100",
            "1 base 0 - dotbelowcomb@100,0 <- A@300,0 Aacute@300,0",
            "2 base 0 - acutecomb@100,500 <- A@300,700 Aacute@300,1000",
            "3 lig 0 - acutecomb@100,500 <- f_i@150,700|450,700",
            "4 mkmk 16 0 acutecomb@100,500 <- acutecomb@100,800",
        ]
    );
    assert_eq!(
        features(gpos),
        ["DFLT/dflt curs[0] mark[1, 2, 3] mkmk[4]", "latn/dflt curs[0] mark[1, 2, 3] mkmk[4]", "latn/TRK  curs[0] mark[1, 2, 3] mkmk[4]"]
    );
}

#[test]
fn the_feature_writer_setting_and_the_tags_turn_generated_features_off() {
    let tags = |text: Option<&str>| {
        let value = text.map(|t| fontcraft_model::plist::openstep::parse_value(t).unwrap());
        let mut warnings = Vec::new();
        (mark_tags(value.as_ref(), &mut warnings), warnings.len())
    };
    assert_eq!(tags(None), (MarkTags::default(), 0), "without the setting every writer runs");
    assert_eq!(tags(Some("()")), (MarkTags::NONE, 0), "an empty list runs none");
    assert_eq!(tags(Some("({class = MarkFeatureWriter; options = {features = (mkmk);};})")), (MarkTags { mkmk: true, ..MarkTags::NONE }, 0));
    assert_eq!(
        tags(Some("({class = CursFeatureWriter;}, {class = GdefFeatureWriter;})")),
        (MarkTags { curs: true, gdef: true, ..MarkTags::NONE }, 0)
    );
    assert_eq!(
        tags(Some("({class = ContextualMarkFeatureWriter; module = glyphsLib.featureWriters.markFeatureWriter;})")),
        (MarkTags { mark: true, mkmk: true, abvm: true, blwm: true, ..MarkTags::NONE }, 0)
    );
    assert_eq!(tags(Some("({class = MarkFeatureWriter; options = {mode = bogus;};})")), (MarkTags::NONE, 0), "a malformed entry is dropped");
    assert_eq!(tags(Some("({class = MarkFeatureWriter; options = {features = (kern);};})")), (MarkTags::NONE, 0), "a tag of another writer");
    assert_eq!(
        tags(Some("({class = MarkFeatureWriter; options = {groupMarkClasses = 1;};})")),
        (MarkTags { mark: true, mkmk: true, abvm: true, blwm: true, ..MarkTags::NONE }, 1),
        "an option fontc refuses is written with the standard settings and a warning"
    );

    // With mark off, only the stacking and the cursive lookups are left; without gdef, no
    // classes or carets, but the filter set the stacking lookup needs.
    let input = probe();
    let names = names(&input);
    let out = write(&input, MarkTags { mark: false, abvm: false, gdef: false, ..MarkTags::default() });
    assert_eq!(
        describe(out.gpos.as_ref().unwrap(), &names),
        ["0 curs 9 - beh-ar 600,100 0,100", "1 mkmk 16 0 acutecomb@100,500 <- acutecomb@100,800"]
    );
    let gdef = out.gdef.unwrap();
    assert!(gdef.glyph_class_def.is_none() && gdef.lig_caret_list.is_none() && gdef.mark_glyph_sets_def.is_some());

    let out = write(&input, MarkTags::NONE);
    assert!(out.gpos.is_none() && out.gdef.is_none() && out.lookups == 0);
}

#[test]
fn glyph_classes_follow_the_categories_and_anchors_or_the_stated_classes() {
    let mut input = MarkInput {
        glyphs: vec![
            glyph(".notdef", &[], None, None, &[]),
            glyph("acutecomb", &[0x301], Some("Mark"), Some("Nonspacing"), &[]),
            glyph("f_f", &[], Some("Letter"), Some("Ligature"), &[]),
            glyph("f_i", &[], Some("Letter"), Some("Ligature"), &[("caret_1", 250.0, 0.0)]),
            glyph("B", &[0x42], Some("Letter"), Some("Uppercase"), &[("top", 250.0, 700.0)]),
            glyph("C", &[0x43], Some("Letter"), Some("Uppercase"), &[]),
            glyph("grave", &[0x60], Some("Mark"), Some("Spacing"), &[("_top", 100.0, 500.0)]),
        ],
        ..Default::default()
    };
    let names = names(&input);
    let class = |n: &str, c: u16| (n.to_string(), c);
    // A mark without anchors keeps its class; a ligature needs an attaching anchor (a caret is
    // one); a glyph with a base anchor is a base; a mark anchor alone attaches nothing.
    let out = write(&input, MarkTags::default());
    assert_eq!(classes(out.gdef.as_ref().unwrap(), &names), [class("B", 1), class("acutecomb", 3), class("f_i", 2)]);

    // A UFO's stated classes are taken as they are.
    input.explicit_classes = true;
    input.glyphs[5].opentype_category = Some("base".into());
    input.glyphs[4].opentype_category = Some("component".into());
    input.glyphs[6].opentype_category = Some("bogus".into());
    let out = write(&input, MarkTags::default());
    assert_eq!(classes(out.gdef.as_ref().unwrap(), &names), [class("B", 4), class("C", 1)]);
    assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
}

#[test]
fn hostile_anchors_are_left_out_with_a_warning_and_the_work_is_capped() {
    let input = MarkInput {
        glyphs: vec![
            glyph(".notdef", &[], None, None, &[]),
            glyph(
                "A",
                &[0x41],
                Some("Letter"),
                Some("Uppercase"),
                &[
                    ("top", 300.0, 700.0),
                    ("_", 0.0, 0.0),
                    ("_top_1", 0.0, 0.0),
                    ("caret_0", 0.0, 0.0),
                    ("top_0", 0.0, 0.0),
                    ("top_99999999999999", 0.0, 0.0),
                    ("bottom", f64::NAN, 0.0),
                    ("ogonek", 1e9, 0.0),
                ],
            ),
            glyph("acutecomb", &[0x301], Some("Mark"), Some("Nonspacing"), &[("_top", 100.0, 500.0), ("_top", 120.0, 520.0)]),
        ],
        ..Default::default()
    };
    let names = names(&input);
    let out = write(&input, MarkTags { abvm: false, ..MarkTags::default() });
    assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
    assert!(out.warnings[0].starts_with("5 anchors were left out") && out.warnings[1].starts_with("2 anchors were left out"), "{:?}", out.warnings);
    // A repeated name keeps its last position.
    assert_eq!(describe(out.gpos.as_ref().unwrap(), &names), ["0 base 0 - acutecomb@120,520 <- A@300,700"]);

    let limited = mark_features(&MarkRequest { input: &input, master: "m01", tags: MarkTags::default(), limits: MarkLimits { work: 3 }, gsub: None });
    assert!(matches!(limited, Err(FeatureError::TooMany { what: "steps of work", limit: 3 })), "{limited:?}");
}

#[test]
fn anchor_names_follow_the_reference_compilers() {
    assert_eq!(anchor_kind("top"), Ok(AnchorKind::Base("top".into())));
    assert_eq!(anchor_kind("top_"), Ok(AnchorKind::Base("top_".into())));
    assert_eq!(anchor_kind("_top"), Ok(AnchorKind::Mark("top".into())));
    assert_eq!(anchor_kind("top_right_2"), Ok(AnchorKind::Ligature { group: "top_right".into(), index: 2 }));
    assert_eq!(anchor_kind("_3"), Ok(AnchorKind::ComponentMarker(3)));
    assert_eq!(anchor_kind("caret_2"), Ok(AnchorKind::Caret(2)));
    assert_eq!(anchor_kind("caret_left"), Ok(AnchorKind::Caret(1)));
    assert_eq!(anchor_kind("vcaret_1"), Ok(AnchorKind::VCaret(1)));
    assert_eq!(anchor_kind("entry"), Ok(AnchorKind::CursiveEntry));
    assert_eq!(anchor_kind("exit"), Ok(AnchorKind::CursiveExit));
    // Numbered cursive anchors are not cursive for the reference compilers.
    assert_eq!(anchor_kind("entry.1"), Ok(AnchorKind::Base("entry.1".into())));
    for refused in ["_", "_top_1", "caret_0", "top_0", "_0"] {
        assert!(anchor_kind(refused).is_err(), "{refused}");
    }
}

#[test]
fn ligature_component_slots_are_charged_to_the_work_budget() {
    // Each ligature keeps a slot per component up to its highest number in every group it has:
    // 40 ligatures × 100 groups × 1,024 slots, from only 4,140 anchors.
    let groups = 100;
    let mut glyphs = vec![glyph(".notdef", &[], None, None, &[])];
    let lig_anchors: Vec<(String, f64, f64)> =
        (0..groups).map(|g| (format!("g{g}_1"), 0.0, 0.0)).chain([("g0_1024".to_string(), 0.0, 0.0)]).collect();
    let lig_anchors: Vec<(&str, f64, f64)> = lig_anchors.iter().map(|(n, x, y)| (n.as_str(), *x, *y)).collect();
    for i in 0..40 {
        glyphs.push(glyph(&format!("lig{i}"), &[], Some("Letter"), Some("Ligature"), &lig_anchors));
    }
    let mark_anchors: Vec<(String, f64, f64)> = (0..groups).map(|g| (format!("_g{g}"), 0.0, 0.0)).collect();
    let mark_anchors: Vec<(&str, f64, f64)> = mark_anchors.iter().map(|(n, x, y)| (n.as_str(), *x, *y)).collect();
    glyphs.push(glyph("mark", &[0x301], Some("Mark"), Some("Nonspacing"), &mark_anchors));
    let input = MarkInput { glyphs, ..Default::default() };
    let work = 1_000_000;
    let limited = mark_features(&MarkRequest { input: &input, master: "m01", tags: MarkTags::default(), limits: MarkLimits { work }, gsub: None });
    assert!(matches!(limited, Err(FeatureError::TooMany { what: "steps of work", .. })), "{limited:?}");
}
