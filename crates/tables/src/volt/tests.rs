//! VOLT to feature code: expected texts are fontTools `voltLib.voltToFea` output for the same
//! projects (its test suite, MIT), so a difference is a difference from the oracle.

use super::*;

fn fea(volt: &str) -> String {
    volt_to_fea(volt).unwrap_or_else(|e| panic!("{e}")).0
}

#[test]
fn volt_glyph_definitions_become_gdef_classes() {
    assert_eq!(
        fea("DEF_GLYPH \"f_f\" ID 320 TYPE LIGATURE COMPONENTS 2 END_GLYPH"),
        "@GDEF_ligature = [f_f];\ntable GDEF {\n    GlyphClassDef , @GDEF_ligature, , ;\n} GDEF;\n"
    );
    assert_eq!(fea("DEF_GLYPH \"glyph20\" ID 20 END_GLYPH"), "");
    // A glyph named like a keyword is escaped.
    assert_eq!(
        fea("DEF_GLYPH \"by\" ID 1 TYPE MARK END_GLYPH"),
        "@GDEF_mark = [\\by];\ntable GDEF {\n    GlyphClassDef , , @GDEF_mark, ;\n} GDEF;\n"
    );
}

#[test]
fn volt_groups_are_sorted_before_the_groups_that_name_them() {
    let text = fea("DEF_GROUP \"TestGroup\"\nENUM GROUP \"Group1\" GROUP \"Group2\" END_ENUM\nEND_GROUP\n\
         DEF_GROUP \"Group1\"\nENUM GLYPH \"a\" GLYPH \"b\" GLYPH \"c\" GLYPH \"d\" END_ENUM\nEND_GROUP\n\
         DEF_GROUP \"Group2\"\nENUM GLYPH \"e\" GLYPH \"f\" GLYPH \"g\" GLYPH \"h\" END_ENUM\nEND_GROUP\n");
    assert_eq!(text, "# Glyph classes\n@Group1 = [a b c d];\n@Group2 = [e f g h];\n@TestGroup = [@Group1 @Group2];");
    assert_eq!(
        fea("DEF_GROUP \"foo\" ENUM GLYPH \"foo\" ENUM GLYPH \"foo.1\" GLYPH \"foo.2\" END_ENUM END_ENUM END_GROUP"),
        "# Glyph classes\n@foo = [foo foo.1 foo.2];"
    );
}

#[test]
fn volt_feature_with_sub_lookups_and_flags() {
    let text = fea(concat!(
        "DEF_SCRIPT NAME \"Latin\" TAG \"latn\"\nDEF_LANGSYS NAME \"Romanian\" TAG \"ROM \"\n",
        "DEF_FEATURE NAME \"Fractions\" TAG \"frac\"\nLOOKUP \"fraclookup\\1\"\nLOOKUP \"fraclookup\\1\"\n",
        "END_FEATURE\nEND_LANGSYS\nEND_SCRIPT\n",
        "DEF_LOOKUP \"fraclookup\\1\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION RTL\nIN_CONTEXT\nEND_CONTEXT\n",
        "AS_SUBSTITUTION\nSUB GLYPH \"one\" GLYPH \"slash\" GLYPH \"two\"\nWITH GLYPH \"one_slash_two.frac\"\nEND_SUB\nEND_SUBSTITUTION\n",
        "DEF_LOOKUP \"fraclookup\\2\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION RTL\nIN_CONTEXT\nEND_CONTEXT\n",
        "AS_SUBSTITUTION\nSUB GLYPH \"one\" GLYPH \"slash\" GLYPH \"three\"\nWITH GLYPH \"one_slash_three.frac\"\nEND_SUB\nEND_SUBSTITUTION",
    ));
    assert_eq!(
        text,
        concat!(
            "\n# Lookups\nlookup fraclookup {\n    lookupflag RightToLeft;\n    # fraclookup\\1\n",
            "    sub one slash two by one_slash_two.frac;\n    subtable;\n    # fraclookup\\2\n",
            "    sub one slash three by one_slash_three.frac;\n} fraclookup;\n\n",
            "# Features\nfeature frac {\n    script latn;\n    language ROM exclude_dflt;\n    lookup fraclookup;\n} frac;\n",
        )
    );
}

#[test]
fn volt_contexts_call_a_chained_lookup() {
    let text = fea(concat!(
        "DEF_GROUP \"Hebrew\" ENUM GLYPH \"uni05D0\" GLYPH \"uni05D1\" END_ENUM END_GROUP\n",
        "DEF_LOOKUP \"HebrewCurrency\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION LTR\n",
        "EXCEPT_CONTEXT\nRIGHT GROUP \"Hebrew\"\nRIGHT GLYPH \"one.Hebr\"\nEND_CONTEXT\n",
        "IN_CONTEXT\nLEFT GROUP \"Hebrew\"\nLEFT GLYPH \"one.Hebr\"\nEND_CONTEXT\n",
        "AS_SUBSTITUTION\nSUB GLYPH \"dollar\"\nWITH GLYPH \"dollar.Hebr\"\nEND_SUB\nEND_SUBSTITUTION",
    ));
    assert_eq!(
        text,
        concat!(
            "# Glyph classes\n@Hebrew = [uni05D0 uni05D1];\n\n# Lookups\n",
            "lookup HebrewCurrency_chained {\n    sub dollar by dollar.Hebr;\n} HebrewCurrency_chained;\n\n",
            "lookup HebrewCurrency {\n    ignore sub dollar' @Hebrew one.Hebr;\n",
            "    sub @Hebrew one.Hebr dollar' lookup HebrewCurrency_chained;\n} HebrewCurrency;\n",
        )
    );
}

#[test]
fn volt_alternates_and_empty_substitutions() {
    let (text, warnings) = volt_to_fea(concat!(
        "DEF_GROUP \"b\" ENUM GLYPH \"b\" END_ENUM END_GROUP\n",
        "DEF_LOOKUP \"test\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION LTR\nIN_CONTEXT\nEND_CONTEXT\nAS_SUBSTITUTION\n",
        "SUB GLYPH \"a\"\nWITH GLYPH \"a.alt1\"\nEND_SUB\nSUB GLYPH \"a\"\nWITH GLYPH \"a.alt2\"\nEND_SUB\n",
        "SUB GROUP \"b\"\nWITH GROUP \"b\"\nEND_SUB\nSUB GLYPH \"b\"\nWITH GLYPH \"b.alt\"\nEND_SUB\nSUB WITH\nEND_SUB\nEND_SUBSTITUTION",
    ))
    .unwrap();
    assert_eq!(text, "# Glyph classes\n@b = [b];\n\n# Lookups\nlookup test {\n    sub a from [a.alt1 a.alt2];\n    sub b from [b b.alt];\n} test;\n");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings.iter().all(|w| w.contains("Ignoring empty substitution")));
}

#[test]
fn volt_mark_attachment_on_a_ligature_and_anchor_devices() {
    let text = fea(concat!(
        "DEF_LOOKUP \"TestLookup\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION LTR\nIN_CONTEXT\nEND_CONTEXT\n",
        "AS_POSITION\nATTACH GLYPH \"f_f\"\nTO GLYPH \"acutecomb\" AT ANCHOR \"top\"\nEND_ATTACH\nEND_POSITION\n",
        "DEF_GLYPH \"f_f\" ID 120 TYPE LIGATURE COMPONENTS 2 END_GLYPH\n",
        "DEF_ANCHOR \"top\" ON 120 GLYPH f_f COMPONENT 1 AT POS DX 250 DY 450 END_POS END_ANCHOR\n",
        "DEF_ANCHOR \"MARK_top\" ON 120 GLYPH acutecomb COMPONENT 1 AT POS  END_POS END_ANCHOR",
    ));
    assert_eq!(
        text,
        concat!(
            "\n# Mark classes\nmarkClass acutecomb <anchor 0 0> @top.TestLookup;\n\n# Lookups\n",
            "lookup TestLookup {\n    pos ligature f_f\n            <anchor 250 450> mark @top.TestLookup\n",
            "        ligComponent\n            <anchor NULL>;\n} TestLookup;\n\n",
            "@GDEF_ligature = [f_f];\ntable GDEF {\n    GlyphClassDef , @GDEF_ligature, , ;\n} GDEF;\n",
        )
    );
    let text = fea(concat!(
        "DEF_ANCHOR \"MARK_top\" ON 123 GLYPH diacglyph COMPONENT 1 AT POS DX 0 DY 456 ADJUST_BY 12 AT 34 ",
        "ADJUST_BY 56 AT 78 END_POS END_ANCHOR\n",
        "DEF_ANCHOR \"top\" ON 121 GLYPH baseglyph COMPONENT 1 AT POS DX 0 DY 0END_POS END_ANCHOR\n",
        "DEF_LOOKUP \"test\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION LTR\nAS_POSITION\n",
        "ATTACH GLYPH \"baseglyph\"\nTO GLYPH \"diacglyph\" AT ANCHOR \"top\"\nEND_ATTACH\nEND_POSITION",
    ));
    assert_eq!(
        text,
        concat!(
            "\n# Mark classes\nmarkClass diacglyph <anchor 0 456 <device NULL> <device 34 12, 78 56>> @top.test;\n",
            "\n# Lookups\nlookup test {\n    pos base baseglyph\n        <anchor 0 0> mark @top.test;\n} test;\n",
        )
    );
}

#[test]
fn volt_pair_and_single_adjustments() {
    let text = fea(concat!(
        "DEF_LOOKUP \"kern1\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION LTR\nIN_CONTEXT\nEND_CONTEXT\n",
        "AS_POSITION\nADJUST_PAIR\n FIRST GLYPH \"A\"\n SECOND GLYPH \"V\"\n",
        " 1 1 BY POS ADV -30 END_POS POS END_POS\n",
        " 1 1 BY POS ADV -40 END_POS POS END_POS\n",
        "END_ADJUST\nEND_POSITION\n",
        "DEF_LOOKUP \"sup\" PROCESS_BASE PROCESS_MARKS ALL DIRECTION LTR\nIN_CONTEXT\nEND_CONTEXT\n",
        "AS_POSITION\nADJUST_SINGLE GLYPH \"one\" BY POS DX 10 DY 200 END_POS\nEND_ADJUST\nEND_POSITION",
    ));
    assert_eq!(
        text,
        concat!("\n# Lookups\nlookup kern1 {\n    enum pos A V -40;\n} kern1;\n\n", "lookup sup {\n    pos one <10 200 0 0>;\n} sup;\n",)
    );
}

#[test]
fn volt_settings_and_tables_choice() {
    let (text, warnings) = volt_to_fea("CMAP_FORMAT 0 3 4").unwrap();
    assert_eq!(text, "");
    assert_eq!(warnings, vec!["Unsupported setting ignored: CMAP_FORMAT".to_string()]);
    let only_gsub = VoltOptions { tables: vec![*b"GSUB"], glyph_order: None };
    let (text, _) = convert("DEF_GLYPH \"a\" ID 1 TYPE BASE END_GLYPH", &only_gsub).unwrap();
    assert_eq!(text, "", "GDEF left out");
    let bad = VoltOptions { tables: vec![*b"kern"], glyph_order: None };
    assert!(convert("", &bad).is_err());
    // A font's glyph order renames the project's glyphs by id.
    let order = VoltOptions { tables: Vec::new(), glyph_order: Some(vec![".notdef".into(), "A".into()]) };
    let (text, _) = convert("DEF_GLYPH \"glyph1\" ID 1 TYPE BASE END_GLYPH", &order).unwrap();
    assert!(text.starts_with("@GDEF_base = [A];"), "{text}");
    let short = VoltOptions { tables: Vec::new(), glyph_order: Some(vec![".notdef".into()]) };
    assert!(convert("DEF_GLYPH \"glyph1\" ID 1 END_GLYPH", &short).is_err(), "an id beyond the font");
}

#[test]
fn volt_bad_projects_fail_with_a_place_and_never_panic() {
    for (project, needle) in [
        ("DEF_GLYPH \"a\" ID -1 END_GLYPH", "Invalid glyph ID"),
        ("DEF_LOOKUP \"1x\" AS_SUBSTITUTION", "must start with a letter"),
        ("DEF_GROUP \"a\" END_GROUP DEF_GROUP \"A\" END_GROUP", "already defined"),
        ("DEF_GROUP \"a\" ENUM GROUP \"b\" END_ENUM END_GROUP DEF_GROUP \"b\" ENUM GROUP \"a\" END_ENUM END_GROUP", "cycle"),
        ("DEF_GROUP \"a\" ENUM GROUP \"missing\" END_ENUM END_GROUP", "used but undefined"),
        ("DEF_SCRIPT TAG \"latn\" DEF_LANGSYS TAG \"dflt\"", "END_LANGSYS"),
        ("DEF_GLYPH \"a", "terminate string"),
        ("$", "Unexpected character"),
        ("DEF_GLYPH \"a\" ID 99999999999999999999 END_GLYPH", "out of range"),
        (
            "DEF_LOOKUP \"k\" AS_POSITION ADJUST_PAIR FIRST GLYPH \"a\" SECOND GLYPH \"b\" 0 1 BY POS END_POS POS END_POS END_ADJUST END_POSITION",
            "ADJUST_PAIR",
        ),
        ("DEF_LOOKUP \"m\" AS_POSITION ATTACH GLYPH \"a\" TO GLYPH \"m\" AT ANCHOR \"top\" END_ATTACH END_POSITION", "no anchor"),
    ] {
        let e = volt_to_fea(project).expect_err(project).to_string();
        assert!(e.contains(needle), "{project}: {e}");
    }
    let deep = format!("DEF_GROUP \"g\" {} GLYPH \"a\" {} END_GROUP", "ENUM ".repeat(200), "END_ENUM ".repeat(200));
    assert!(volt_to_fea(&deep).is_err(), "nesting is bounded");
    // Groups that double at each level would expand beyond any budget.
    let mut project = String::from("DEF_GROUP \"g0\" ENUM GLYPH \"a\" GLYPH \"b\" END_ENUM END_GROUP\n");
    for i in 1..40 {
        project.push_str(&format!("DEF_GROUP \"g{i}\" ENUM GROUP \"g{}\" GROUP \"g{}\" END_ENUM END_GROUP\n", i - 1, i - 1));
    }
    project.push_str("DEF_LOOKUP \"m\" AS_POSITION ATTACH GROUP \"g39\" TO GLYPH \"m\" AT ANCHOR \"top\" END_ATTACH END_POSITION");
    assert!(matches!(volt_to_fea(&project), Err(TablesError::TooLarge { .. })));
    assert!(looks_like_volt("\u{feff}  DEF_GLYPH \"a\" ID 0 END_GLYPH"));
    assert!(!looks_like_volt("feature liga { } liga;"));
}

/// fontTools' own conversions of two real projects, when the corpus is fetched.
#[test]
fn volt_corpus_projects_equal_the_reference_conversion() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests/voltLib/data");
    for name in ["Nutso", "NamdhinggoSIL1006"] {
        let (Ok(vtp), Ok(expected)) =
            (std::fs::read_to_string(dir.join(format!("{name}.vtp"))), std::fs::read_to_string(dir.join(format!("{name}.fea"))))
        else {
            eprintln!("skipped: corpus not fetched");
            return;
        };
        let (text, _) = volt_to_fea(&vtp).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(text, expected, "{name}");
    }
}
