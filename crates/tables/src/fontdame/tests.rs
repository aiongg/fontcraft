//! FontDame to feature code. The texts are ours (mtiLib builds binary tables, not feature
//! code); the corpus test reads fontTools' mtiLib test files, and the integration measure
//! compiles the result and compares it with mtiLib's tables.
#![allow(clippy::string_slice)]

use super::*;

fn tabs(s: &str) -> String {
    s.replace("\\t", "\t")
}

#[test]
fn fontdame_gsub_lookups_subtables_and_features() {
    let text = tabs(concat!(
        "FontDame GSUB table\n\nscript table begin\nlatn\\tdefault\\t\\t0, 1\nlatn\\tDEU \\t\\t1\nscript table end\n\n",
        "feature table begin\n0\\tliga\\tligs\n1\\tsmcp\\tsc\nfeature table end\n\n",
        "lookup\\tligs\\tligature\nRightToLeft\\tno\nIgnoreMarks\\tyes\n\nf_i\\tf\\ti\nlookup end\n\n",
        "lookup\\tsc\\tsingle\na\\ta.sc\n% subtable\nb\\tb.sc\nlookup end\n",
    ));
    let (fea, warnings) = fontdame_to_fea(&text, &[]).unwrap();
    assert_eq!(
        fea,
        concat!(
            "languagesystem latn dflt;\nlanguagesystem latn DEU;\n\n",
            "# GSUB lookups\nlookup GSUB_ligs {\n    lookupflag IgnoreMarks;\n    sub f i by f_i;\n} GSUB_ligs;\n\n",
            "lookup GSUB_sc {\n    sub a by a.sc;\n    subtable;\n    sub b by b.sc;\n} GSUB_sc;\n\n",
            "feature liga {\n    script latn;\n    language dflt;\n    lookup GSUB_ligs;\n} liga;\n\n",
            "feature smcp {\n    script latn;\n    language dflt;\n    lookup GSUB_sc;\n",
            "    script latn;\n    language DEU exclude_dflt;\n    lookup GSUB_sc;\n} smcp;\n\n",
        )
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn fontdame_contextual_rules_call_lookups_defined_first() {
    // Carriage returns end lines, as in files saved by the reference tool.
    let text = tabs(concat!(
        "lookup\\traucontext-sinh\\tchained\rmarkattachmenttype\\t2\r",
        "glyph\\trakarsinh\\tuvowelsignsinh\\t\\t1,u2aelow-sinh\rlookup end\r",
        "lookup\\tu2aelow-sinh\\tsingle\ruvowelsignsinh\\tuuvowelsignsinh\rlookup end\r",
    ));
    let (fea, warnings) = fontdame_to_fea(&text, &[]).unwrap();
    assert_eq!(
        fea,
        concat!(
            "# GSUB lookups\nlookup GSUB_u2aelow_sinh {\n    sub uvowelsignsinh by uuvowelsignsinh;\n} GSUB_u2aelow_sinh;\n\n",
            "lookup GSUB_raucontext_sinh {\n",
            "    # FontDame lookup flag left out: mark attachment class 2 (convert the GDEF file with this one)\n",
            "    sub rakarsinh uvowelsignsinh' lookup GSUB_u2aelow_sinh;\n} GSUB_raucontext_sinh;\n\n",
        )
    );
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("change places")));
    assert!(warnings.iter().any(|w| w.contains("mark attachment class 2")));

    // Class rules: named classes, the backtrack reversed, class 0 from the glyph order.
    let text = tabs(concat!(
        "FontDame GSUB table\nlookup\\tctx\\tchained\nbacktrackclass definition begin\nk\\t1\nclass definition end\n",
        "class definition begin\nu\\t1\nclass definition end\n",
        "class-chain\\t0,1\\t1\\t\\t1,sw\nlookup end\nlookup\\tsw\\tsingle\nu\\tu.alt\nlookup end\n",
    ));
    let order: Vec<String> = [".notdef", "k", "u", "u.alt"].iter().map(|s| s.to_string()).collect();
    let (fea, _) = fontdame_to_fea(&text, &order).unwrap();
    assert!(fea.contains("@GSUB_ctx_1_backtrack1 = [k];\n"), "{fea}");
    assert!(fea.contains("@GSUB_ctx_1_backtrack0all = [.notdef u u.alt];\n"), "{fea}");
    assert!(fea.contains("sub @GSUB_ctx_1_backtrack1 @GSUB_ctx_1_backtrack0all @GSUB_ctx_1_input1' lookup GSUB_sw;"), "{fea}");
    let (_, warnings) = fontdame_to_fea(&text, &[]).unwrap();
    assert!(warnings.iter().any(|w| w.contains("need the glyph order")), "{warnings:?}");
}

#[test]
fn fontdame_gpos_pairs_marks_and_cursive() {
    let text = tabs(concat!(
        "FontDame GPOS table\nlookup\\tkern\\tkernset\nleft x advance\\tA\\tV\\t-10\nsubtable end\n",
        "firstclass definition begin\nA\\t1\nO\\t2\nclass definition end\n",
        "secondclass definition begin\nV\\t1\nclass definition end\n",
        "left x advance\\t1\\t1\\t-50\nleft x advance\\t2\\t1\\t-20\nlookup end\n",
        "lookup\\tmarks\\tmark to base\nmark\\tacute\\t0\\t100, 500\nbase\\tA\\t0\\t300, 700\\t4\nlookup end\n",
        "lookup\\tcurs\\tcursive\nentry\\tB\\t0, 0\nexit\\tB\\t500, 0\nlookup end\n",
    ));
    let (fea, warnings) = fontdame_to_fea(&text, &[]).unwrap();
    assert_eq!(
        fea,
        concat!(
            "# GPOS lookups\n@GPOS_kern_2_first1 = [A];\n@GPOS_kern_2_second1 = [V];\n@GPOS_kern_2_first2 = [O];\n",
            "lookup GPOS_kern {\n    pos A V <0 0 -10 0>;\n    subtable;\n",
            "    pos @GPOS_kern_2_first1 @GPOS_kern_2_second1 <0 0 -50 0>;\n",
            "    pos @GPOS_kern_2_first2 @GPOS_kern_2_second1 <0 0 -20 0>;\n} GPOS_kern;\n\n",
            "markClass acute <anchor 100 500> @GPOS_marks_1_mark0;\n",
            "lookup GPOS_marks {\n    pos base A <anchor 300 700 contourpoint 4> mark @GPOS_marks_1_mark0;\n} GPOS_marks;\n\n",
            "lookup GPOS_curs {\n    pos cursive B <anchor 0 0> <anchor 500 0>;\n} GPOS_curs;\n\n",
        )
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn fontdame_gdef_classes_feed_the_lookup_flags_of_files_given_with_it() {
    let gdef = tabs(concat!(
        "FontDame GDEF table\nclass definition begin\nf_i\\t2\nacute\\t3\nclass definition end\n",
        "mark attachment class definition begin\nacute\\t1\nclass definition end\n",
        "carets begin\nf_i\\t1\\t300\ncarets end\n",
    ));
    let gsub = tabs("FontDame GSUB table\nlookup\\tl\\tsingle\nMarkAttachmentType\\t1\na\\tb\nlookup end\n");
    let (fea, warnings) = convert(&[&gsub, &gdef], &FontDameOptions::default()).unwrap();
    assert_eq!(
        fea,
        concat!(
            "# GDEF classes for lookup flags\n@GDEF_MarkAttachClass1 = [acute];\n\n",
            "# GSUB lookups\nlookup GSUB_l {\n    lookupflag MarkAttachmentType @GDEF_MarkAttachClass1;\n    sub a by b;\n} GSUB_l;\n\n",
            "table GDEF {\n    GlyphClassDef , [f_i], [acute], ;\n    LigatureCaretByPos f_i 300;\n} GDEF;\n",
        )
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    // Asked for GDEF only, the lookups are left out.
    let only = FontDameOptions { tables: vec![*b"GDEF"], ..FontDameOptions::default() };
    let (fea, _) = convert(&[&gsub, &gdef], &only).unwrap();
    assert!(!fea.contains("lookup GSUB_"), "{fea}");
    assert!(looks_like_fontdame(&gsub) && looks_like_fontdame(&gdef));
    assert!(!looks_like_fontdame("feature liga { sub f i by f_i; } liga;"));
}

#[test]
fn fontdame_bad_files_fail_with_the_line_and_never_panic() {
    for (text, needle) in [
        ("FontDame GSUB table\nlookup\\tl\\tsingle\na\\tb\nlookup end\nlookup\\tl\\tsingle\na\\tc\nlookup end\n", "Duplicate lookup name"),
        ("FontDame GSUB table\nlookup\\tl\\tpair\nlookup end\n", "unknown GSUB lookup type"),
        ("FontDame GSUB table\nlookup\\tl\\tchained\nglyph\\ta\\tb\\t\\t0,l\nlookup end\n", "not 1 or more"),
        ("FontDame GSUB table\nlookup\\tl\\tchained\nglyph\\ta\\tb\\t\\t1,missing\nlookup end\n", "does not define"),
        ("FontDame GSUB table\nlookup\\tl\\tchained\nglyph\\ta\\tb\\t\\t1,l\nlookup end\n", "rule of its own"),
        ("FontDame GSUB table\nlookup\\tl\\tchained\nglyph\\ta\\tb\\t\\t3,l2\nlookup end\nlookup\\tl2\\tsingle\na\\tb\nlookup end\n", "beyond"),
        ("FontDame cmap table\ncmap subtable 0\n", "not layout"),
        ("FontDame GPOS table\nlookup\\tl\\tsingle\nx placement\\ta\\t1\nx placement\\ta\\t2\nlookup end\n", "twice"),
        ("FontDame GSUB table\nlookup\\tl\\tsingle\na b\\tc\nlookup end\n", "Space found"),
        ("FontDame GSUB table\nlookup\\tl\\tsingle\na\\tb\n", "file ends"),
        ("FontDame GDEF table\ncarets begin\nfi\\t2\\t100\ncarets end\n", "announces 2 carets"),
    ] {
        let e = fontdame_to_fea(&tabs(text), &[]).expect_err(text).to_string();
        assert!(e.contains(needle), "{text}: {e}");
    }
    let many: Vec<&str> = vec![""; MAX_FILES + 1];
    assert!(matches!(convert(&many, &FontDameOptions::default()), Err(TablesError::TooLarge { .. })));
    let bad = FontDameOptions { table: Some(*b"cmap"), ..FontDameOptions::default() };
    assert!(convert(&["lookup\tl\tsingle\na\tb\nlookup end\n"], &bad).is_err());
    // `# N` names a glyph by id.
    let (fea, _) = fontdame_to_fea("FontDame GSUB table\nlookup\tl\tsingle\n# 1\t# 7\nlookup end\n", &[".notdef".into(), "a".into()]).unwrap();
    assert!(fea.contains("sub a by glyph00007;"), "{fea}");
}

/// A `U XXXX` glyph takes the name mtiLib gives it: AGLFN's, else `uniXXXX` or `uXXXXX`.
#[test]
fn fontdame_code_points_take_their_aglfn_names() {
    let text = tabs("FontDame GSUB table\nlookup\tnum\talternate\nU 30\tU F730\tU 2039\tu 1D400\nlookup end\n");
    let (fea, _) = fontdame_to_fea(&text, &[]).unwrap();
    assert!(fea.contains("sub zero from [uniF730 guilsinglleft u1D400];"), "{fea}");
}

/// feaLib pairs the two classes of a reverse chaining rule after sorting each by name, fea-rs
/// in written order: the classes are written sorted, and a mapping whose replacements sort in
/// another order is written one glyph at a time. No `subtable;` goes between the subtables.
#[test]
fn fontdame_reverse_chaining_keeps_its_pairs_in_both_compilers() {
    let text = tabs(concat!(
        "FontDame GSUB table\nlookup\trev\treversechained\nRightToLeft\tyes\n",
        "backtrackcoverage definition begin\nk\ncoverage definition end\n",
        "zay\tzay.fina\nray\tray.fina\n% subtable\n",
        "lookaheadcoverage definition begin\nk\ncoverage definition end\n",
        "b\ta.alt\na\tb.alt\nlookup end\n",
    ));
    let (fea, warnings) = fontdame_to_fea(&text, &[]).unwrap();
    assert_eq!(
        fea,
        concat!(
            "# GSUB lookups\nlookup GSUB_rev {\n    lookupflag RightToLeft;\n",
            "    rsub k [ray zay]' by [ray.fina zay.fina];\n",
            "    rsub a' k by b.alt;\n    rsub b' k by a.alt;\n} GSUB_rev;\n\n",
        )
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// fontTools' mtiLib test files, each read as the table its test reads it as.
#[test]
fn fontdame_corpus_test_files_convert() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests/mtiLib");
    let Ok(harness) = std::fs::read_to_string(root.join("mti_test.py")) else {
        eprintln!("skipped: corpus not fetched");
        return;
    };
    let start = harness.find("GLYPH_ORDER = [").unwrap();
    let end = start + harness[start..].find(']').unwrap();
    let order: Vec<String> = harness[start..end].split('"').skip(1).step_by(2).map(str::to_string).collect();
    assert!(order.len() > 50, "{}", order.len());
    let files: [(&str, &[u8; 4]); 28] = [
        ("featurename-backward", b"GSUB"),
        ("featurename-forward", b"GSUB"),
        ("lookupnames-backward", b"GSUB"),
        ("lookupnames-forward", b"GSUB"),
        ("mixed-toplevels", b"GSUB"),
        ("mti/scripttable", b"GSUB"),
        ("mti/chainedclass", b"GSUB"),
        ("mti/chainedcoverage", b"GSUB"),
        ("mti/chained-glyph", b"GSUB"),
        ("mti/gsubalternate", b"GSUB"),
        ("mti/gsubligature", b"GSUB"),
        ("mti/gsubmultiple", b"GSUB"),
        ("mti/gsubreversechanined", b"GSUB"),
        ("mti/gsubsingle", b"GSUB"),
        ("mti/scripttable", b"GPOS"),
        ("mti/chained-glyph", b"GPOS"),
        ("mti/gposcursive", b"GPOS"),
        ("mti/gposkernset", b"GPOS"),
        ("mti/gposmarktobase", b"GPOS"),
        ("mti/gpospairclass", b"GPOS"),
        ("mti/gpospairglyph", b"GPOS"),
        ("mti/gpossingle", b"GPOS"),
        ("mti/mark-to-ligature", b"GPOS"),
        ("mti/gdefattach", b"GDEF"),
        ("mti/gdefclasses", b"GDEF"),
        ("mti/gdefligcaret", b"GDEF"),
        ("mti/gdefmarkattach", b"GDEF"),
        ("mti/gdefmarkfilter", b"GDEF"),
    ];
    for (name, tag) in files {
        let text = std::fs::read_to_string(root.join("data").join(format!("{name}.txt"))).unwrap();
        let options = FontDameOptions { table: Some(*tag), glyph_order: order.clone(), ..FontDameOptions::default() };
        let (fea, _) = convert(&[&text], &options).unwrap_or_else(|e| panic!("{name}: {e}"));
        if !name.contains("scripttable") {
            assert!(!fea.is_empty(), "{name}");
        }
    }
}
