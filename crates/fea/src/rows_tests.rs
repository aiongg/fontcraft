//! One test per `fea.*` row of `spec/inventory/12b-feature-language.toml`, with code from the
//! row's section of the feature file specification, plus the guards of `check.rs`,
//! `resolve.rs` and `provider.rs` and the hostile inputs of `docs/plans/M8.md` § 5. A test
//! whose name ends in `_gap` pins a construct fea-rs does not support: it must fail with a
//! message (or, where fea-rs drops it, compile without it), never panic.

use std::collections::BTreeMap;

use crate::*;

const NAMES: &str = ".notdef space a b c d e f g h i j k l m n o p q r s t u v w x y z A B C D E F G H I J K L M N O P Q R S T U V W X Y Z \
    zero one two three four five six seven eight nine a.sc b.sc c.sc A.sc B.sc a.alt b.alt c.alt a.alt2 f_i f_f_i f_l f_f c_t \
    acute grave cedilla dieresis dotbelow acutecomb gravecomb lam_alef e.fina e.init hyphen period comma x-y sub by";

fn order() -> GlyphOrder {
    GlyphOrder::new(NAMES.split_whitespace().map(String::from).collect()).unwrap()
}

fn files(code: &str) -> VirtualFiles {
    VirtualFiles::new("features.fea", code).unwrap()
}

fn run_files(files: &VirtualFiles) -> Result<Compiled, Vec<Diagnostic>> {
    compile(&CompileRequest::new(files, &order()))
}

fn run(code: &str) -> Result<Compiled, Vec<Diagnostic>> {
    run_files(&files(code))
}

fn ok(code: &str) -> Compiled {
    run(code).unwrap_or_else(|d| panic!("expected to compile, got {d:#?}"))
}

/// Fails with at least one error, and with no internal failure of the compiler.
fn fails(code: &str) -> Vec<Diagnostic> {
    let d = run(code).expect_err("expected an error");
    assert!(d.iter().any(Diagnostic::is_error), "{d:#?}");
    assert!(!d.iter().any(|d| d.message.contains("please report")), "{d:#?}");
    d
}

fn fails_with(code: &str, needle: &str) {
    let d = fails(code);
    assert!(d.iter().any(|d| d.message.contains(needle)), "no `{needle}` in {d:#?}");
}

fn warns(c: &Compiled, needle: &str) -> bool {
    c.warnings.iter().any(|d| d.message.contains(needle))
}

fn be16(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]))
}

/// The lookup types of a `GSUB` or `GPOS` table; an extension lookup reports 100 plus the type
/// it wraps.
fn lookup_types(bytes: &[u8], extension: usize) -> Vec<usize> {
    let list = be16(bytes, 8);
    (0..be16(bytes, list))
        .map(|i| {
            let lookup = list + be16(bytes, list + 2 + 2 * i);
            let kind = be16(bytes, lookup);
            if kind == extension {
                let sub = lookup + be16(bytes, lookup + 6);
                100 + be16(bytes, sub + 2)
            } else {
                kind
            }
        })
        .collect()
}

fn gsub(c: &Compiled) -> Vec<usize> {
    c.gsub.as_deref().map(|b| lookup_types(b, 7)).unwrap_or_default()
}

fn gpos(c: &Compiled) -> Vec<usize> {
    c.gpos.as_deref().map(|b| lookup_types(b, 9)).unwrap_or_default()
}

fn feature<'a>(c: &'a Compiled, tag: &[u8; 4]) -> Option<&'a FeatureSummary> {
    c.features.iter().find(|f| f.tag == Tag::new(tag))
}

fn tags(c: &Compiled) -> Vec<String> {
    c.features.iter().map(|f| f.tag.to_string()).collect()
}

fn langs(c: &Compiled, tag: &[u8; 4]) -> Vec<String> {
    feature(c, tag).map(|f| f.language_systems.iter().map(|(s, l)| format!("{s}/{l}")).collect()).unwrap_or_default()
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack.windows(needle.len()).filter(|w| *w == needle).count()
}

// ------------------------------------------------------------------------------------------
// Lexical structure

#[test]
fn fea_comment() {
    let c = ok("# a comment; with a semicolon\nfeature liga { sub f i by f_i; # behind a rule\n} liga;");
    assert_eq!(gsub(&c), [4]);
}

#[test]
fn fea_whitespace() {
    let c = ok("feature liga {\r\n\tsub   f\r\ni\rby f_i;\n} liga;");
    assert_eq!(gsub(&c), [4]);
}

#[test]
fn fea_keywords() {
    fails("feature liga { sub feature by a; } liga;");
    // `locationDef` is not a keyword fea-rs knows: written as a statement it is an error.
    fails("locationDef wght=200 @light;");
}

#[test]
fn fea_statement_terminator() {
    fails("feature liga { sub a by b } liga;");
    assert_eq!(gsub(&ok("feature liga { sub a by b;; } liga;")), [1]);
}

#[test]
fn fea_number() {
    assert_eq!(gpos(&ok("feature kern { pos a -20; } kern;")), [1]);
}

#[test]
fn fea_number_radix() {
    assert!(ok("table name { nameid 300 3 01 0x0409 \"x\"; } name;").name.is_some());
    assert!(ok("table OS/2 { FamilyClass 0x0105; } OS/2;").os2.family_class == Some(0x0105));
    // fea-rs reads FSType as a decimal only.
    fails("table OS/2 { FSType 010; } OS/2;");
}

#[test]
fn fea_float() {
    assert_eq!(ok("table head { FontRevision 1.500; } head;").head.font_revision, Some(1.5));
}

#[test]
fn fea_metric() {
    assert_eq!(gpos(&ok("feature kern { pos a 10; } kern;")), [1]);
}

#[test]
fn fea_glyph_name() {
    assert_eq!(gsub(&ok("feature smcp { sub a by a.sc; sub c_t by c; } smcp;")), [1]);
    fails("feature smcp { sub a by nosuchglyph; } smcp;");
}

#[test]
fn fea_glyph_name_development() {
    assert_eq!(gsub(&ok("feature ss01 { sub x-y by a; } ss01;")), [1]);
}

#[test]
fn fea_glyph_name_escape() {
    assert_eq!(gsub(&ok("feature ss01 { sub \\sub by \\by; sub \\a by b; } ss01;")), [1]);
}

#[test]
fn fea_glyph_name_vs_range() {
    // `x-y` is a glyph; `a-c` is a range of three.
    let c = ok("@one = [x-y]; @three = [a-c]; feature ss01 { sub @one by a; sub @three by [d e f]; } ss01;");
    assert_eq!(gsub(&c), [1]);
}

#[test]
fn fea_cid_gap() {
    // FontCraft's glyph orders are by name, so a CID never names a glyph.
    fails("feature ss01 { sub \\1 by a; } ss01;");
}

#[test]
fn fea_tag() {
    let c = ok("languagesystem DFLT dflt; languagesystem latn TRK; feature ss01 { sub a by b; } ss01;");
    assert_eq!(langs(&c, b"ss01"), ["DFLT/dflt", "latn/TRK "]);
}

#[test]
fn fea_tag_quoted_gap() {
    // fea-rs does not read quoted tags.
    fails("feature \"liga\" { sub f i by f_i; } \"liga\";");
}

#[test]
fn fea_tag_defaults() {
    let c = ok("languagesystem DFLT dflt; languagesystem latn dflt; feature ss01 { sub a by b; } ss01;");
    assert_eq!(langs(&c, b"ss01"), ["DFLT/dflt", "latn/dflt"]);
}

#[test]
fn fea_lookup_label() {
    assert_eq!(gsub(&ok("lookup Some_label.1 { sub a by b; } Some_label.1; feature ss01 { lookup Some_label.1; } ss01;")), [1]);
    fails("lookup 1abc { sub a by b; } 1abc;");
}

// ------------------------------------------------------------------------------------------
// Glyph classes

#[test]
fn fea_glyph_class() {
    assert_eq!(gsub(&ok("feature smcp { sub [a b c] by [a.sc b.sc c.sc]; } smcp;")), [1]);
}

#[test]
fn fea_glyph_range_letter() {
    assert_eq!(gsub(&ok("feature case { sub [a-c] by [A-C]; } case;")), [1]);
}

#[test]
fn fea_glyph_range_numeric() {
    let order = GlyphOrder::new([".notdef", "a", "g01", "g02", "g03", "h01", "h02", "h03"].map(String::from).to_vec()).unwrap();
    let files = files("feature ss01 { sub [g01-g03] by [h01-h03]; } ss01;");
    let c = compile(&CompileRequest::new(&files, &order)).unwrap();
    assert_eq!(gsub(&c), [1]);
}

#[test]
fn fea_glyph_range_cid_gap() {
    fails("feature ss01 { sub [\\1-\\3] by a; } ss01;");
}

#[test]
fn fea_glyph_range_missing_members_gap() {
    // The specification leaves a missing member out; fea-rs refuses the range.
    let order = GlyphOrder::new([".notdef", "a", "b", "c", "a.sc", "c.sc"].map(String::from).to_vec()).unwrap();
    let files = files("feature smcp { sub [a c] by [a.sc-c.sc]; } smcp;");
    let d = compile(&CompileRequest::new(&files, &order)).unwrap_err();
    assert!(d.iter().any(|d| d.is_error() && d.message.contains("b.sc")), "{d:#?}");
}

#[test]
fn fea_named_glyph_class() {
    let c = ok("@lower = [a b c]; @lower = [a b]; feature smcp { sub @lower by [a.sc b.sc]; } smcp;");
    assert_eq!(gsub(&c), [1]);
}

#[test]
fn fea_named_glyph_class_nesting() {
    assert_eq!(gsub(&ok("@ab = [a b]; @abc = [@ab c]; feature smcp { sub @abc by [a.sc b.sc c.sc]; } smcp;")), [1]);
}

#[test]
fn fea_named_glyph_class_alias() {
    assert_eq!(gsub(&ok("@ab = [a b]; @same = @ab; feature smcp { sub @same by [a.sc b.sc]; } smcp;")), [1]);
}

#[test]
fn fea_class_sharing() {
    // Two lookups with the same coverage: the packer stores it once (format 1, glyphs a b c).
    let c = ok("feature ss01 { lookup A { sub [a b c] by d; } A; lookup B { sub [a b c] by e; } B; } ss01;");
    let coverage = [0, 1, 0, 3, 0, 2, 0, 3, 0, 4];
    assert_eq!(count(c.gsub.as_deref().unwrap(), &coverage), 1);
}

// ------------------------------------------------------------------------------------------
// Value records, devices, anchors

#[test]
fn fea_value_record_single() {
    assert_eq!(gpos(&ok("feature kern { pos a 30; } kern;")), [1]);
}

#[test]
fn fea_value_record_vertical_single() {
    let horizontal = ok("feature kern { pos a 30; } kern;").gpos.unwrap();
    let vertical = ok("feature vkrn { pos a 30; } vkrn;").gpos.unwrap();
    // The value format differs: x advance (4) against y advance (8).
    assert_ne!(horizontal, vertical);
}

#[test]
fn fea_value_record_four() {
    assert_eq!(gpos(&ok("feature kern { pos a <10 20 30 40>; } kern;")), [1]);
}

#[test]
fn fea_value_record_device() {
    assert_eq!(gpos(&ok("feature kern { pos a <0 0 30 0 <device 0 0> <device 0 0> <device 11 -1, 12 -1> <device 0 0>>; } kern;")), [1]);
}

#[test]
fn fea_value_record_null() {
    assert_eq!(gpos(&ok("feature kern { pos a b <NULL>; pos a c 10; } kern;")), [2]);
}

#[test]
fn fea_value_record_named() {
    assert_eq!(gpos(&ok("valueRecordDef <10 0 20 0> WIDE; feature kern { pos a <WIDE>; } kern;")), [1]);
}

#[test]
fn fea_value_record_def() {
    assert_eq!(gpos(&ok("feature kern { valueRecordDef 15 TIGHT; pos a <TIGHT>; } kern;")), [1]);
}

#[test]
fn fea_device() {
    assert_eq!(gpos(&ok("feature kern { pos a <0 0 20 0 <device NULL> <device NULL> <device 11 1, 12 2> <device NULL>>; } kern;")), [1]);
}

#[test]
fn fea_device_null() {
    assert_eq!(gpos(&ok("feature kern { pos a <0 0 20 0 <device NULL> <device NULL> <device NULL> <device NULL>>; } kern;")), [1]);
}

#[test]
fn fea_contour_point() {
    assert_eq!(gpos(&ok("markClass acute <anchor 0 0> @TOP; feature mark { pos base a <anchor 250 450 contourpoint 5> mark @TOP; } mark;")), [4]);
}

#[test]
fn fea_anchor_coordinates() {
    assert_eq!(gpos(&ok("markClass acute <anchor 0 500> @TOP; feature mark { pos base a <anchor 250 450> mark @TOP; } mark;")), [4]);
}

#[test]
fn fea_anchor_contour_point() {
    assert_eq!(gpos(&ok("markClass acute <anchor 0 500 contourpoint 2> @TOP; feature mark { pos base a <anchor 250 450> mark @TOP; } mark;")), [4]);
}

#[test]
fn fea_anchor_device() {
    let code = "markClass acute <anchor 0 500> @TOP; feature mark { pos base a <anchor 250 450 <device 11 1> <device NULL>> mark @TOP; } mark;";
    assert_eq!(gpos(&ok(code)), [4]);
}

#[test]
fn fea_anchor_null() {
    let code = "markClass acute <anchor 0 500> @TOP; markClass dotbelow <anchor 0 0> @BOT; \
        feature mark { pos ligature f_i <anchor 100 500> mark @TOP <anchor NULL> ligComponent <anchor 400 500> mark @TOP; } mark;";
    assert_eq!(gpos(&ok(code)), [5]);
}

#[test]
fn fea_anchor_named() {
    let code = "anchorDef 250 450 TOP_A; markClass acute <anchor 0 500> @TOP; feature mark { pos base a <anchor TOP_A> mark @TOP; } mark;";
    assert_eq!(gpos(&ok(code)), [4]);
}

#[test]
fn fea_anchor_def() {
    let code = "anchorDef 0 500 ACC; anchorDef 0 500 contourpoint 3 ACC2; markClass acute <anchor ACC> @TOP; markClass grave <anchor ACC2> @TOP; \
        feature mark { pos base a <anchor 250 450> mark @TOP; } mark;";
    assert_eq!(gpos(&ok(code)), [4]);
}

// ------------------------------------------------------------------------------------------
// Including files

fn with_files(root: &str, others: &[(&str, &str)]) -> VirtualFiles {
    let mut f = files(root);
    for (path, text) in others {
        f.insert(path, text).unwrap();
    }
    f
}

#[test]
fn fea_include() {
    let f = with_files(
        "include(liga.fea)\ninclude(classes/lower.fea);",
        &[("liga.fea", "feature liga { sub f i by f_i; } liga;"), ("classes/lower.fea", "@lower = [a b];")],
    );
    assert_eq!(gsub(&run_files(&f).unwrap()), [4]);
}

#[test]
fn fea_include_contexts() {
    let f = with_files("feature liga { include(rule.fea); } liga;", &[("rule.fea", "sub f i by f_i;")]);
    assert_eq!(gsub(&run_files(&f).unwrap()), [4]);
    // fea-rs reads an include inside a lookup or table block as top-level code: an error.
    let f = with_files("lookup L { include(rule.fea); } L;", &[("rule.fea", "sub a by b;")]);
    assert!(run_files(&f).is_err());
    let f = with_files("table OS/2 { include(os2.fea); } OS/2;", &[("os2.fea", "FSType 4;")]);
    assert!(run_files(&f).is_err());
}

#[test]
fn fea_include_resolution() {
    // Root folder first, then the including file's folder.
    let f = with_files(
        "include(sub/a.fea);",
        &[("sub/a.fea", "include(b.fea);"), ("sub/b.fea", "feature liga { sub f i by f_i; } liga;"), ("b.fea", "feature ss01 { sub a by b; } ss01;")],
    );
    assert_eq!(tags(&run_files(&f).unwrap()), ["ss01"]);
    let f = with_files("include(sub/a.fea);", &[("sub/a.fea", "include(b.fea);"), ("sub/b.fea", "feature liga { sub f i by f_i; } liga;")]);
    assert_eq!(tags(&run_files(&f).unwrap()), ["liga"]);
    // A path that leaves the file set is refused, not read.
    let d = run_files(&files("include(../../etc/passwd);")).unwrap_err();
    assert!(d.iter().any(Diagnostic::is_error));
}

#[test]
fn fea_include_depth() {
    let mut f = files("include(f1.fea);");
    for i in 1..60 {
        f.insert(&format!("f{i}.fea"), &format!("include(f{}.fea);", i + 1)).unwrap();
    }
    f.insert("f60.fea", "@x = [a];").unwrap();
    let d = run_files(&f).unwrap_err();
    assert!(d.iter().any(|d| d.message.contains("nested at most 50")), "{d:#?}");
    // The same file twice is allowed.
    let f = with_files("include(c.fea); include(c.fea); feature ss01 { sub @x by b; } ss01;", &[("c.fea", "@x = [a];")]);
    assert!(run_files(&f).is_ok());
}

// ------------------------------------------------------------------------------------------
// Features, lookups, language systems

#[test]
fn fea_feature_block() {
    assert_eq!(tags(&ok("feature liga { sub f i by f_i; } liga;")), ["liga"]);
    fails("feature liga { sub f i by f_i; } calt;");
}

#[test]
fn fea_rule_auto_type() {
    let c = ok("feature ss01 { sub a by b; } ss01; feature ss02 { sub f_i by f i; } ss02; feature ss03 { sub a from [a.alt a.alt2]; } ss03; \
        feature liga { sub f i by f_i; } liga; feature calt { sub a' b by c; } calt;");
    assert_eq!(gsub(&c), [1, 2, 3, 4, 6, 1]);
}

#[test]
fn fea_lookup_implicit() {
    let c = ok("feature ss01 { sub a by b; sub c by d; sub e from [a.alt a.alt2]; sub e by f; } ss01;");
    assert_eq!(gsub(&c), [1, 3, 1]);
    // fea-rs promotes single substitutions that share a run with a ligature.
    assert_eq!(gsub(&ok("feature ss01 { sub a by b; sub f i by f_i; } ss01;")), [4]);
}

#[test]
fn fea_languagesystem() {
    let c = ok("languagesystem DFLT dflt; languagesystem latn dflt; languagesystem latn DEU; feature ss01 { sub a by b; } ss01;");
    assert_eq!(langs(&c, b"ss01"), ["DFLT/dflt", "latn/dflt", "latn/DEU "]);
}

#[test]
fn fea_languagesystem_default() {
    assert_eq!(langs(&ok("feature ss01 { sub a by b; } ss01;"), b"ss01"), ["DFLT/dflt"]);
}

#[test]
fn fea_languagesystem_order() {
    // fea-rs accepts a languagesystem statement after the first feature.
    assert!(run("feature ss01 { sub a by b; } ss01; languagesystem latn dflt;").is_ok());
}

#[test]
fn fea_script() {
    let c = ok("languagesystem DFLT dflt; languagesystem latn dflt; languagesystem cyrl dflt; feature locl { script latn; sub a by b; } locl;");
    assert_eq!(langs(&c, b"locl"), ["latn/dflt"]);
    fails("lookup L { script latn; sub a by b; } L;");
}

#[test]
fn fea_language() {
    let c = ok("languagesystem latn dflt; languagesystem latn DEU; feature locl { script latn; language DEU; sub a by b; } locl;");
    assert_eq!(langs(&c, b"locl"), ["latn/DEU "]);
}

#[test]
fn fea_language_default_inheritance() {
    let c = ok("languagesystem latn dflt; languagesystem latn DEU; languagesystem latn TRK; \
        feature locl { sub a by b; script latn; language DEU; sub c by d; } locl;");
    // DEU gets the default rules and its own; TRK is not named, so keeps the top-level ones.
    let f = feature(&c, b"locl").unwrap();
    assert!(f.language_systems.contains(&(Tag::new(b"latn"), Tag::new(b"DEU "))));
}

#[test]
fn fea_language_exclude_dflt() {
    let c = ok(
        "languagesystem latn dflt; languagesystem latn DEU; feature locl { sub a by b; script latn; language DEU exclude_dflt; sub c by d; } locl;",
    );
    assert!(feature(&c, b"locl").is_some());
}

#[test]
fn fea_language_include_dflt() {
    let c = ok(
        "languagesystem latn dflt; languagesystem latn DEU; feature locl { sub a by b; script latn; language DEU include_dflt; sub c by d; } locl;",
    );
    assert!(feature(&c, b"locl").is_some());
}

#[test]
fn fea_language_deprecated_keywords() {
    let c = ok(
        "languagesystem latn dflt; languagesystem latn DEU; feature locl { sub a by b; script latn; language DEU excludeDFLT; sub c by d; } locl;",
    );
    assert!(feature(&c, b"locl").is_some());
}

#[test]
fn fea_language_required() {
    let c = ok("languagesystem latn dflt; languagesystem latn DEU; feature ss01 { script latn; language DEU required; sub a by b; } ss01;");
    assert!(feature(&c, b"ss01").is_some());
}

#[test]
fn fea_parameters() {
    let c = ok("feature size { parameters 10.0 0; } size;");
    assert_eq!(tags(&c), ["size"]);
}

#[test]
fn fea_lookupflag_named() {
    let c = ok("feature liga { lookupflag RightToLeft IgnoreMarks; sub f i by f_i; } liga;");
    let gsub = c.gsub.unwrap();
    let list = be16(&gsub, 8);
    let lookup = list + be16(&gsub, list + 2);
    assert_eq!(be16(&gsub, lookup + 2), 0x0009);
}

#[test]
fn fea_lookupflag_mark_attachment_type() {
    let c = ok("@top = [acute grave]; feature liga { lookupflag MarkAttachmentType @top; sub f i by f_i; } liga;");
    assert!(c.gdef.is_some());
}

#[test]
fn fea_lookupflag_use_mark_filtering_set() {
    let c = ok("@top = [acute grave]; feature liga { lookupflag UseMarkFilteringSet @top; sub f i by f_i; } liga;");
    assert!(c.gdef.is_some());
}

#[test]
fn fea_lookupflag_numeric() {
    assert_eq!(gsub(&ok("feature liga { lookupflag 8; sub f i by f_i; } liga;")), [4]);
}

#[test]
fn fea_lookupflag_scope() {
    let c = ok("feature liga { lookupflag IgnoreMarks; sub f i by f_i; lookupflag 0; sub f l by f_l; } liga; feature ss01 { sub a by b; } ss01;");
    assert_eq!(gsub(&c).len(), 3);
}

#[test]
fn fea_lookup_block() {
    assert_eq!(gsub(&ok("feature liga { lookup LIG { sub f i by f_i; } LIG; } liga;")), [4]);
    fails("feature liga { lookup L { sub f i by f_i; pos a 10; } L; } liga;");
}

#[test]
fn fea_lookup_block_standalone() {
    let c = ok("lookup UNUSED { sub a by b; } UNUSED; feature ss01 { sub c by d; } ss01;");
    assert_eq!(gsub(&c).len(), 2);
}

#[test]
fn fea_lookup_block_use_extension() {
    assert_eq!(gsub(&ok("feature liga { lookup L useExtension { sub f i by f_i; } L; } liga;")), [104]);
}

#[test]
fn fea_lookup_reference() {
    assert_eq!(gsub(&ok("lookup L { sub a by b; } L; feature ss01 { lookup L; } ss01; feature ss02 { lookup L; } ss02;")), [1]);
}

#[test]
fn fea_mark_class() {
    let c = ok("markClass [acute grave] <anchor 0 500> @TOP; feature mark { pos base [a e] <anchor 250 450> mark @TOP; } mark;");
    assert_eq!(gpos(&c), [4]);
}

#[test]
fn fea_mark_class_rules() {
    // A mark class must be defined before a rule uses it.
    fails_with("feature mark { pos base a <anchor 250 450> mark @TOP; } mark; markClass acute <anchor 0 500> @TOP;", "undefined mark class");
    // Mark classes used in one lookup must not share glyphs.
    fails(
        "markClass acute <anchor 0 500> @TOP; markClass acute <anchor 0 0> @BOT; \
        feature mark { pos base a <anchor 250 450> mark @TOP <anchor 250 0> mark @BOT; } mark;",
    );
}

#[test]
fn fea_mark_class_gdef_inference() {
    let c = ok(
        "markClass acute <anchor 0 500> @TOP; feature mark { pos base a <anchor 250 450> mark @TOP; } mark; feature liga { sub f i by f_i; } liga;",
    );
    assert!(c.gdef.is_some());
    assert!(!c.gdef_from_code);
}

#[test]
fn fea_subtable() {
    assert_eq!(gsub(&ok("feature ss01 { sub a by b; subtable; sub c by d; } ss01;")), [1]);
}

// ------------------------------------------------------------------------------------------
// GSUB

#[test]
fn fea_gsub1_glyph_by_glyph() {
    assert_eq!(gsub(&ok("feature smcp { sub a by a.sc; } smcp;")), [1]);
}

#[test]
fn fea_gsub1_class_by_glyph() {
    assert_eq!(gsub(&ok("feature ss01 { sub [a b c] by d; } ss01;")), [1]);
}

#[test]
fn fea_gsub1_class_by_class() {
    assert_eq!(gsub(&ok("feature smcp { sub [a b c] by [a.sc b.sc c.sc]; } smcp;")), [1]);
    fails("feature smcp { sub [a b c] by [a.sc b.sc]; } smcp;");
}

#[test]
fn fea_gsub1_null() {
    assert_eq!(gsub(&ok("feature ss01 { sub a by NULL; } ss01;")), [2]);
}

#[test]
fn fea_gsub2_multiple() {
    assert_eq!(gsub(&ok("feature ccmp { sub f_f_i by f f i; } ccmp;")), [2]);
}

#[test]
fn fea_gsub3_alternate() {
    assert_eq!(gsub(&ok("feature salt { sub a from [a.alt a.alt2]; } salt;")), [3]);
}

#[test]
fn fea_gsub4_ligature() {
    assert_eq!(gsub(&ok("feature liga { sub f f i by f_f_i; } liga;")), [4]);
}

#[test]
fn fea_gsub4_class_enumeration() {
    assert_eq!(gsub(&ok("feature liga { sub [f F] [i I] by f_i; } liga;")), [4]);
}

#[test]
fn fea_gsub4_sorting() {
    // The shorter ligature first in the source; the longer one must still match.
    assert_eq!(gsub(&ok("feature liga { sub f f by f_f; sub f f i by f_f_i; } liga;")), [4]);
}

#[test]
fn fea_gsub5_contextual() {
    assert_eq!(gsub(&ok("lookup L { sub a by b; } L; feature calt { sub [c d] a' lookup L e; } calt;")), [1, 6]);
}

#[test]
fn fea_gsub6_marking() {
    assert_eq!(gsub(&ok("feature calt { sub c a' b' by f_i; } calt;")), [6, 4]);
}

#[test]
fn fea_gsub6_lookup_references() {
    let c = ok("lookup A { sub a by b; } A; lookup B { sub c by d; } B; feature calt { sub x a' lookup A c' lookup B; } calt;");
    assert_eq!(gsub(&c), [1, 1, 6]);
    fails("lookup P { pos a 10; } P; feature calt { sub x a' lookup P; } calt;");
}

#[test]
fn fea_gsub6_inline_single() {
    assert_eq!(gsub(&ok("feature calt { sub x a' by b; } calt;")), [6, 1]);
}

#[test]
fn fea_gsub6_inline_ligature() {
    assert_eq!(gsub(&ok("feature calt { sub x f' i' by f_i; } calt;")), [6, 4]);
}

#[test]
fn fea_gsub6_ignore() {
    assert_eq!(gsub(&ok("feature calt { ignore sub x a'; sub a' b by c; } calt;")), [6, 1]);
    // fea-rs accepts an ignore rule without a marked glyph.
    assert!(run("feature calt { ignore sub x a; sub a' b by c; } calt;").is_ok());
}

#[test]
fn fea_gsub6_ignore_list() {
    assert_eq!(gsub(&ok("feature calt { ignore sub x a', y a'; sub a' b by c; } calt;")), [6, 1]);
}

#[test]
fn fea_gsub7_extension() {
    assert_eq!(gsub(&ok("lookup L useExtension { sub a by b; } L; feature ss01 { lookup L; } ss01;")), [101]);
}

#[test]
fn fea_gsub8_reverse() {
    assert_eq!(gsub(&ok("feature rclt { rsub [a b] c' d by c.alt; } rclt;")), [8]);
}

#[test]
fn fea_gsub8_null_gap() {
    fails("feature rclt { rsub a c' by NULL; } rclt;");
}

// ------------------------------------------------------------------------------------------
// GPOS

#[test]
fn fea_gpos1_single() {
    assert_eq!(gpos(&ok("feature kern { pos [a b] <0 0 20 0>; } kern;")), [1]);
}

#[test]
fn fea_gpos2_format_a() {
    assert_eq!(gpos(&ok("feature kern { pos a b -20; pos c <0 0 -30 0> d <0 0 10 0>; } kern;")), [2]);
}

#[test]
fn fea_gpos2_format_b() {
    assert_eq!(gpos(&ok("@L = [a b]; @R = [c d]; feature kern { pos @L @R -40; } kern;")), [2]);
}

#[test]
fn fea_gpos2_class_vs_specific() {
    assert_eq!(gpos(&ok("@L = [a b]; @R = [c d]; feature kern { pos a c -10; pos @L @R -40; } kern;")), [2]);
}

#[test]
fn fea_gpos2_enum() {
    assert_eq!(gpos(&ok("@R = [c d]; feature kern { enum pos a @R -10; } kern;")), [2]);
}

#[test]
fn fea_gpos2_conflicts() {
    let c = ok("@A = [a b]; feature kern { pos a c 100; enum pos @A c -50; } kern;");
    assert_eq!(gpos(&c), [2]);
}

#[test]
fn fea_gpos2_subtable_breaks() {
    let c = ok("@L1 = [a b]; @L2 = [b c]; @R = [d]; feature kern { pos @L1 @R -10; subtable; pos @L2 @R -20; } kern;");
    assert_eq!(gpos(&c), [2]);
    // Overlapping first classes without a break: fea-rs splits the subtable itself.
    assert_eq!(gpos(&ok("@L1 = [a b]; @L2 = [b c]; @R = [d]; feature kern { pos @L1 @R -10; pos @L2 @R -20; } kern;")), [2]);
}

#[test]
fn fea_gpos3_cursive() {
    assert_eq!(gpos(&ok("feature curs { pos cursive a <anchor NULL> <anchor 500 20>; pos cursive b <anchor 0 20> <anchor NULL>; } curs;")), [3]);
}

#[test]
fn fea_gpos4_mark_to_base() {
    assert_eq!(gpos(&ok("markClass acute <anchor 0 500> @TOP; feature mark { pos base [a b] <anchor 250 450> mark @TOP; } mark;")), [4]);
}

#[test]
fn fea_gpos5_mark_to_ligature() {
    let code = "markClass acute <anchor 0 500> @TOP; feature mark { pos ligature f_i <anchor 100 500> mark @TOP ligComponent <anchor 400 500> mark @TOP; } mark;";
    assert_eq!(gpos(&ok(code)), [5]);
}

#[test]
fn fea_gpos5_null_component() {
    let code = "markClass acute <anchor 0 500> @TOP; feature mark { pos ligature f_i <anchor NULL> ligComponent <anchor 400 500> mark @TOP; } mark;";
    assert_eq!(gpos(&ok(code)), [5]);
}

#[test]
fn fea_gpos6_mark_to_mark() {
    assert_eq!(gpos(&ok("markClass acute <anchor 0 500> @TOP; feature mkmk { pos mark grave <anchor 0 700> mark @TOP; } mkmk;")), [6]);
}

#[test]
fn fea_gpos7_contextual() {
    assert_eq!(gpos(&ok("lookup K { pos a 20; } K; feature kern { pos x a' lookup K b; } kern;")), [1, 8]);
}

#[test]
fn fea_gpos8_marking() {
    assert_eq!(gpos(&ok("feature kern { pos x a' 20 b; } kern;")), [8, 1]);
}

#[test]
fn fea_gpos8_lookup_references() {
    assert_eq!(gpos(&ok("lookup K { pos a 20; } K; feature kern { pos x a' lookup K; } kern;")), [1, 8]);
    fails("lookup S { sub a by b; } S; feature kern { pos x a' lookup S; } kern;");
}

#[test]
fn fea_gpos8_inline_single() {
    assert_eq!(gpos(&ok("feature kern { pos x a' <0 0 20 0> b' <0 0 -5 0>; } kern;")), [8, 1]);
}

#[test]
fn fea_gpos8_pair_shorthand() {
    // Each value becomes its own single positioning lookup.
    assert_eq!(gpos(&ok("feature kern { pos x a' 20 b; pos [y z] a' -20 c; } kern;")), [8, 1, 1]);
}

#[test]
fn fea_gpos8_inline_cursive_gap() {
    fails("feature curs { pos x cursive a' <anchor 0 0> <anchor 500 0>; } curs;");
}

#[test]
fn fea_gpos8_inline_mark_gap() {
    fails("markClass acute <anchor 0 500> @TOP; feature mark { pos x base a' <anchor 250 450> mark @TOP; } mark;");
}

#[test]
fn fea_gpos8_ignore() {
    assert_eq!(gpos(&ok("feature kern { ignore pos x a'; pos a' 20 b; } kern;")), [8, 1]);
}

#[test]
fn fea_gpos9_extension() {
    assert_eq!(gpos(&ok("lookup K useExtension { pos a 20; } K; feature kern { lookup K; } kern;")), [101]);
}

// ------------------------------------------------------------------------------------------
// Ordering

#[test]
fn fea_order_lookups() {
    let c = ok("feature ss02 { sub c by d; } ss02; feature ss01 { sub a by b; } ss01;");
    assert_eq!(gsub(&c), [1, 1]);
    let first = feature(&c, b"ss02").unwrap();
    assert_eq!(first.lookups, 1);
}

#[test]
fn fea_order_aalt_first() {
    let c = ok("feature smcp { sub a by a.sc; } smcp; feature aalt { feature smcp; } aalt;");
    assert_eq!(gsub(&c), [1, 1]);
    assert!(feature(&c, b"aalt").is_some());
}

#[test]
fn fea_order_rules() {
    // A second rule for the same glyph in one lookup is accepted without a message.
    assert_eq!(gsub(&ok("feature ss01 { sub a by b; sub a by c; } ss01;")), [1]);
}

#[test]
fn fea_order_subtables() {
    assert_eq!(gsub(&ok("feature liga { sub f i by f_i; subtable; sub f l by f_l; } liga;")), [4]);
}

// ------------------------------------------------------------------------------------------
// aalt, size, names, cvParameters

#[test]
fn fea_aalt_feature_reference() {
    let c = ok(
        "feature aalt { feature salt; feature smcp; } aalt; feature salt { sub a from [a.alt a.alt2]; } salt; feature smcp { sub b by b.sc; } smcp;",
    );
    assert!(feature(&c, b"aalt").is_some());
}

#[test]
fn fea_aalt_extra_rules() {
    let c = ok("feature aalt { feature smcp; sub c by c.alt; } aalt; feature smcp { sub a by a.sc; } smcp;");
    assert!(feature(&c, b"aalt").is_some());
}

#[test]
fn fea_aalt_grouping() {
    let c = ok(
        "feature aalt { feature smcp; feature salt; } aalt; feature smcp { sub a by a.sc; } smcp; feature salt { sub a by a.alt; sub b by b.alt; } salt;",
    );
    // a has two alternates (an alternate lookup), b one (a single lookup).
    assert!(gsub(&c).contains(&3));
}

#[test]
fn fea_aalt_restrictions() {
    fails("languagesystem latn dflt; feature aalt { script latn; feature smcp; } aalt; feature smcp { sub a by a.sc; } smcp;");
}

#[test]
fn fea_size_parameters() {
    let c = ok("feature size { parameters 10.0 0; } size;");
    assert!(feature(&c, b"size").is_some());
    fails_with("feature size { parameters -10.0 0; } size;", "negative");
    fails_with("feature size { parameters 10.0 3; sizemenuname \"x\"; } size;", "range start");
}

#[test]
fn fea_size_sizemenuname() {
    let c = ok("feature size { parameters 10.0 3 80 139; sizemenuname \"Small\"; sizemenuname 1 \"Small\"; } size;");
    assert!(c.name.is_some());
}

#[test]
fn fea_size_restrictions() {
    fails("languagesystem latn dflt; feature size { script latn; parameters 10.0 0; } size;");
}

#[test]
fn fea_feature_names() {
    let c = ok("feature ss01 { featureNames { name \"Alternate a\"; name 1 \"Alternate a\"; }; sub a by a.alt; } ss01;");
    assert!(c.name.is_some());
    fails("feature ss21 { featureNames { name \"x\"; }; sub a by b; } ss21;");
}

#[test]
fn fea_cv_parameters() {
    let c = ok("feature cv01 { cvParameters { FeatUILabelNameID { name \"Alt a\"; }; Character 0x61; }; sub a by a.alt; } cv01;");
    assert!(c.name.is_some());
}

#[test]
fn fea_cv_parameters_character() {
    assert!(ok("feature cv01 { cvParameters { Character 0x10FFFF; Character 97; }; sub a by a.alt; } cv01;").gsub.is_some());
}

// ------------------------------------------------------------------------------------------
// Table blocks

#[test]
fn fea_table_block() {
    let c = ok("table hhea { Ascender 800; } hhea; table OS/2 { WeightClass 700; } OS/2; table head { FontRevision 2.0; } head;");
    assert_eq!(c.hhea.ascender, Some(800));
    assert_eq!(c.os2.weight_class, Some(700));
    assert_eq!(c.head.font_revision, Some(2.0));
}

#[test]
fn fea_table_base_tag_list() {
    let c = ok("table BASE { HorizAxis.BaseTagList ideo romn; HorizAxis.BaseScriptList latn romn -120 0; } BASE;");
    assert!(c.base.is_some());
    assert!(ok("feature ss01 { sub a by b; } ss01;").base.is_none());
}

#[test]
fn fea_table_base_script_list() {
    fails("table BASE { HorizAxis.BaseTagList ideo romn; HorizAxis.BaseScriptList latn romn -120; } BASE;");
}

#[test]
fn fea_table_base_min_max_gap() {
    // Parsed and accepted; fea-rs does not write the MinMax record.
    assert!(
        run("table BASE { HorizAxis.BaseTagList ideo romn; HorizAxis.BaseScriptList latn romn -120 0; HorizAxis.MinMax latn dflt -200, 800; } BASE;")
            .is_ok()
    );
}

#[test]
fn fea_table_gdef_glyph_class_def() {
    let c = ok("table GDEF { GlyphClassDef [a b], [f_i], [acute grave], ; } GDEF;");
    assert!(c.gdef.is_some());
    assert!(c.gdef_from_code);
}

#[test]
fn fea_table_gdef_attach() {
    assert!(ok("table GDEF { Attach a 1; Attach b 2 5; } GDEF;").gdef.is_some());
}

#[test]
fn fea_table_gdef_ligature_caret_by_pos() {
    assert!(ok("table GDEF { LigatureCaretByPos f_i 300; LigatureCaretByPos f_f_i 300 600; } GDEF;").gdef.is_some());
}

#[test]
fn fea_table_gdef_ligature_caret_by_index() {
    assert!(ok("table GDEF { LigatureCaretByIndex f_i 3; } GDEF;").gdef.is_some());
    fails("table GDEF { LigatureCaretByIndex f_i -3; } GDEF;");
}

#[test]
fn fea_table_gdef_ligature_caret_by_dev_gap() {
    // Parsed and dropped: no caret list is written.
    assert!(run("table GDEF { LigatureCaretByDev f_i <device 11 -1> ; } GDEF;").is_ok());
}

#[test]
fn fea_table_gdef_derived_classes() {
    fails("table GDEF { MarkAttachClassDef [acute]; } GDEF;");
}

#[test]
fn fea_table_head_font_revision() {
    assert_eq!(ok("table head { FontRevision 1.1; FontRevision 1.25; } head;").head.font_revision, Some(1.25));
}

#[test]
fn fea_table_hhea() {
    let c = ok("table hhea { CaretOffset -10; Ascender 900; Descender -200; LineGap 50; } hhea;");
    assert_eq!(c.hhea, MetricsValues { caret_offset: Some(-10), ascender: Some(900), descender: Some(-200), line_gap: Some(50) });
    fails("table hhea { CaretSlopeRise 1; } hhea;");
}

#[test]
fn fea_table_name_nameid() {
    assert!(ok("table name { nameid 300 \"Wide\"; nameid 300 1 \"Wide\"; } name;").name.is_some());
}

#[test]
fn fea_table_name_reserved() {
    // fea-rs keeps a record with a reserved id; dropping it is left to the font compiler.
    let c = ok("table name { nameid 2 \"Bold\"; } name;");
    assert!(c.name.is_some() && c.warnings.is_empty());
}

#[test]
fn fea_table_name_strings() {
    assert!(ok("table name { nameid 300 3 1 0x0409 \"caf\\00e9\"; nameid 300 1 0 0 \"caf\\8e\"; } name;").name.is_some());
    fails("table name { nameid 300 3 1 0x0409 \"caf\\e9\"; } name;");
}

#[test]
fn fea_table_os2_fs_type() {
    assert_eq!(ok("table OS/2 { FSType 4; } OS/2;").os2.fs_type, Some(4));
    fails("table OS/2 { fsType 4; } OS/2;");
}

#[test]
fn fea_table_os2_panose() {
    assert_eq!(ok("table OS/2 { Panose 2 15 0 0 2 2 4 3 2 4; } OS/2;").os2.panose, Some([2, 15, 0, 0, 2, 2, 4, 3, 2, 4]));
}

#[test]
fn fea_table_os2_unicode_range() {
    assert_eq!(ok("table OS/2 { UnicodeRange 0 1 32; } OS/2;").os2.unicode_range, Some([0b11, 1, 0, 0]));
}

#[test]
fn fea_table_os2_code_page_range() {
    assert!(ok("table OS/2 { CodePageRange 1252 1251; } OS/2;").os2.code_page_range.is_some());
    fails("table OS/2 { CodePageRange 1234; } OS/2;");
}

#[test]
fn fea_table_os2_typo_metrics() {
    let c = ok("table OS/2 { TypoAscender 800; TypoDescender -200; TypoLineGap 100; } OS/2;");
    assert_eq!((c.os2.typo_ascender, c.os2.typo_descender, c.os2.typo_line_gap), (Some(800), Some(-200), Some(100)));
}

#[test]
fn fea_table_os2_win_metrics() {
    let c = ok("table OS/2 { winAscent 900; winDescent 250; } OS/2;");
    assert_eq!((c.os2.win_ascent, c.os2.win_descent), (Some(900), Some(250)));
    fails("table OS/2 { winAscent -900; } OS/2;");
}

#[test]
fn fea_table_os2_heights() {
    let c = ok("table OS/2 { XHeight 500; CapHeight 700; } OS/2;");
    assert_eq!((c.os2.x_height, c.os2.cap_height), (Some(500), Some(700)));
}

#[test]
fn fea_table_os2_classes() {
    let c = ok("table OS/2 { WeightClass 700; WidthClass 5; FamilyClass 0x0105; } OS/2;");
    assert_eq!((c.os2.weight_class, c.os2.width_class, c.os2.family_class), (Some(700), Some(5), Some(0x0105)));
    assert!(warns(&ok("table OS/2 { FamilyClass 0x0F00; } OS/2;"), "sFamilyClass"));
}

#[test]
fn fea_table_os2_vendor() {
    assert_eq!(ok("table OS/2 { Vendor \"ABCD\"; } OS/2;").os2.vendor_id, Some(Tag::new(b"ABCD")));
}

#[test]
fn fea_table_os2_optical_size() {
    let c = ok("table OS/2 { LowerOpSize 80; UpperOpSize 140; } OS/2;");
    assert_eq!((c.os2.lower_optical_point_size, c.os2.upper_optical_point_size), (Some(80), Some(140)));
}

#[test]
fn fea_table_os2_script_and_strikeout_gap() {
    fails("table OS/2 { SubscriptXSize 650; } OS/2;");
}

#[test]
fn fea_table_vhea() {
    let c = ok("table vhea { VertTypoAscender 500; VertTypoDescender -500; VertTypoLineGap 0; } vhea;");
    assert_eq!(c.vhea, MetricsValues { caret_offset: None, ascender: Some(500), descender: Some(-500), line_gap: Some(0) });
    fails("table vhea { CaretOffset 0; } vhea;");
}

#[test]
fn fea_table_vmtx() {
    // Parsed; fea-rs's result has no vmtx or VORG values to hand on.
    assert!(run("table vmtx { VertOriginY a 880; VertAdvanceY a 1000; } vmtx;").is_ok());
}

#[test]
fn fea_table_stat_elided_fallback_name() {
    let c = ok("table STAT { ElidedFallbackName { name \"Regular\"; }; DesignAxis wght 0 { name \"Weight\"; }; } STAT;");
    assert!(c.stat.is_some());
}

#[test]
fn fea_table_stat_design_axis() {
    let c = ok("table name { nameid 256 \"Regular\"; } name; \
        table STAT { ElidedFallbackNameID 256; DesignAxis wght 0 { name \"Weight\"; }; DesignAxis wdth 1 { name \"Width\"; }; } STAT;");
    assert!(c.stat.is_some());
}

#[test]
fn fea_table_stat_axis_value() {
    let c = ok("table name { nameid 256 \"Regular\"; } name; \
        table STAT { ElidedFallbackNameID 256; DesignAxis wght 0 { name \"Weight\"; }; \
        AxisValue { location wght 400; name \"Regular\"; flag ElidableAxisValueName; }; } STAT;");
    assert!(c.stat.is_some());
}

#[test]
fn fea_table_stat_location_formats() {
    let c = ok("table name { nameid 256 \"Regular\"; } name; \
        table STAT { ElidedFallbackNameID 256; DesignAxis wght 0 { name \"Weight\"; }; DesignAxis wdth 1 { name \"Width\"; }; \
        AxisValue { location wght 400 300 500; name \"Regular\"; }; AxisValue { location wght 700; location wdth 100; name \"Bold\"; }; } STAT;");
    assert!(c.stat.is_some());
}

// ------------------------------------------------------------------------------------------
// Anonymous blocks and legacy forms

#[test]
fn fea_anonymous() {
    let c = ok("anon sbit {\n  this is not feature syntax { ;\n} sbit;\nfeature ss01 { sub a by b; } ss01;");
    assert_eq!(tags(&c), ["ss01"]);
}

#[test]
fn fea_legacy_except_gap() {
    fails("feature calt { except x sub a' by b; } calt;");
}

#[test]
fn fea_legacy_bare_value_record_gap() {
    fails("feature kern { pos a 80 0 -160 0; } kern;");
}

// ------------------------------------------------------------------------------------------
// Guards against fea-rs panics. Each input made fea-rs panic before its guard.

#[test]
fn a_cid_over_16_bits_is_refused() {
    fails_with("@a = [\\70000];", "CID 70000");
    fails_with("@a = [\\1-\\70000];", "CID 70000");
}

#[test]
fn numbers_outside_16_bits_are_refused() {
    for code in [
        "feature kern { pos a 40000; } kern;",
        "feature kern { pos a <40000 0 0 0>; } kern;",
        "table hhea { Ascender 40000; } hhea;",
        "table vhea { VertTypoAscender 40000; } vhea;",
        "table GDEF { LigatureCaretByPos f_i 40000; } GDEF;",
        "table BASE { HorizAxis.BaseTagList ideo romn; HorizAxis.BaseScriptList latn romn 40000 0; } BASE;",
        "table STAT { ElidedFallbackNameID 70000; } STAT;",
        "table STAT { ElidedFallbackName { name \"x\"; }; DesignAxis wght 70000 { name \"W\"; }; } STAT;",
        "table STAT { ElidedFallbackName { name \"x\"; }; DesignAxis wght 0 { name \"W\"; }; AxisValue { location wght 99999999999; name \"x\"; }; } STAT;",
        "table OS/2 { CodePageRange 99999; } OS/2;",
        "table OS/2 { winAscent 70000; } OS/2;",
        "feature kern { pos cursive a <anchor 0 0> <anchor 40000 0>; } kern;",
        "valueRecordDef 40000 FOO; feature kern { pos a <FOO>; } kern;",
    ] {
        fails_with(code, "outside -32768 to 32767");
    }
}

#[test]
fn a_stat_fallback_name_the_code_does_not_define_is_refused() {
    fails_with("table STAT { ElidedFallbackNameID 2; DesignAxis wght 0 { name \"Weight\"; }; } STAT;", "ElidedFallbackNameID 2");
}

#[test]
fn an_empty_head_table_is_refused() {
    fails_with("table head { } head;", "FontRevision");
}

#[test]
fn a_size_feature_needs_menu_names_exactly_with_a_subfamily() {
    fails_with("feature size { parameters 10.0 3 80 139; } size;", "sizemenuname");
    fails_with("feature size { parameters 10.0 0; sizemenuname \"x\"; } size;", "sizemenuname");
}

#[test]
fn a_character_beyond_unicode_is_refused() {
    fails_with("feature cv01 { cvParameters { Character 0x1100000; }; sub a by b; } cv01;", "beyond Unicode");
}

#[test]
fn a_lone_surrogate_in_a_name_is_refused() {
    fails_with("table name { nameid 300 3 1 0x409 \"\\d800\"; } name;", "UTF-16");
    assert!(ok("table name { nameid 300 3 1 0x409 \"\\d83d\\de00\"; } name;").name.is_some());
}

#[test]
fn a_panic_on_the_compiler_thread_is_reported() {
    let result: Result<(), Vec<Diagnostic>> = crate::run::on_big_stack(|| panic!("boom"));
    let d = result.unwrap_err();
    assert!(d[0].message.contains("boom"), "{d:#?}");
}

// ------------------------------------------------------------------------------------------
// Hostile inputs (docs/plans/M8.md § 5)

#[test]
fn sixteen_megabytes_of_code_are_refused() {
    let code = "# filler line\n".repeat((16 << 20) / 14 + 1);
    fails_with(&code, "larger than");
}

#[test]
fn brackets_ten_thousand_deep_are_refused() {
    fails_with(&"[".repeat(10_000), "nested at most 64");
}

#[test]
fn a_class_doubled_forty_times_is_refused() {
    let mut code = String::from("@c0 = [a b];\n");
    for i in 1..=40 {
        code.push_str(&format!("@c{i} = [@c{} @c{}];\n", i - 1, i - 1));
    }
    fails_with(&code, "glyph classes expand");
}

#[test]
fn a_ligature_of_thirty_classes_of_a_hundred_is_refused() {
    let mut names = vec![".notdef".to_string(), "x".to_string()];
    names.extend((0..100).map(|i| format!("g{i}")));
    let order = GlyphOrder::new(names).unwrap();
    let class: Vec<String> = (0..100).map(|i| format!("g{i}")).collect();
    let code = format!("@L = [{}]; feature liga {{ sub {} by x; }} liga;", class.join(" "), vec!["@L"; 30].join(" "));
    let files = files(&code);
    let d = compile(&CompileRequest::new(&files, &order)).unwrap_err();
    assert!(d.iter().any(|d| d.message.contains("sequences")), "{d:#?}");
}

#[test]
fn an_include_cycle_is_refused() {
    let f = with_files("include(a.fea);", &[("a.fea", "include(b.fea);"), ("b.fea", "include(./x/../a.fea);")]);
    let d = run_files(&f).unwrap_err();
    assert!(d.iter().any(|d| d.message.contains("includes itself")), "{d:#?}");
    let d = run_files(&files("include(././features.fea);")).unwrap_err();
    assert!(d.iter().any(|d| d.message.contains("includes itself")), "{d:#?}");
}

#[test]
fn includes_that_double_at_each_level_are_refused() {
    let mut f = files("include(l0.fea); include(l0.fea);");
    for i in 0..40 {
        f.insert(&format!("l{i}.fea"), &format!("include(l{}.fea); include(l{}.fea); # padding to make each copy count\n", i + 1, i + 1)).unwrap();
    }
    f.insert("l40.fea", "@x = [a];").unwrap();
    let d = run_files(&f).unwrap_err();
    assert!(d.iter().any(|d| d.message.contains("more than")), "{d:#?}");
}

#[cfg(unix)]
#[test]
fn includes_that_make_new_paths_through_a_folder_link_stop() {
    let dir = std::env::temp_dir().join(format!("fontcraft-fea-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
    std::fs::write(dir.join("f.fea"), "include(loop/f.fea);").unwrap();
    let f = files("include(f.fea);").with_include_dir(dir.clone());
    let d = run_files(&f).unwrap_err();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(d.iter().any(Diagnostic::is_error), "{d:#?}");
}

#[test]
fn an_included_file_is_read_from_the_source_folder() {
    let dir = std::env::temp_dir().join(format!("fontcraft-fea-read-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).unwrap();
    std::fs::write(dir.join("features/shared.fea"), "feature liga { sub f i by f_i; } liga;").unwrap();
    let f = VirtualFiles::new("Font.ufo/features.fea", "include(../features/shared.fea);").unwrap().with_include_dir(dir.clone());
    let result = run_files(&f);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(tags(&result.unwrap()), ["liga"]);
}

#[test]
fn a_diagnostic_names_the_included_file() {
    let f = with_files("include(bad.fea);", &[("bad.fea", "feature liga { sub f i by nosuch; } liga;")]);
    let d = run_files(&f).unwrap_err();
    let bad = d.iter().find(|d| d.is_error()).unwrap();
    assert_eq!(bad.file, "bad.fea");
    assert_eq!("feature liga { sub f i by nosuch; } liga;".get(bad.span.clone()), Some("nosuch"));
}

// ------------------------------------------------------------------------------------------
// Generated lookups

fn gid(name: &str) -> GlyphId16 {
    order().id(name).unwrap()
}

fn kern_lookups() -> GeneratedLookups {
    let mut g = GeneratedLookups::default();
    g.lookups.push(GeneratedLookup {
        class: String::new(),
        flags: LookupFlags::default(),
        filter_set: None,
        rules: GeneratedRules::Pair {
            glyph_pairs: vec![GlyphPair { left: gid("a"), right: gid("b"), value: Value { x_advance: Some((-30).into()), ..Value::default() } }],
            class_pairs: vec![],
        },
    });
    g.features.insert(FeatureKey { feature: Tag::new(b"kern"), script: Tag::new(b"DFLT"), language: Tag::new(b"dflt") }, vec![0]);
    g
}

fn compile_with(code: &str, generated: &GeneratedLookups) -> Result<Compiled, Vec<Diagnostic>> {
    let files = files(code);
    let order = order();
    let mut req = CompileRequest::new(&files, &order);
    req.generated = Some(generated);
    compile(&req)
}

#[test]
fn generated_kerning_goes_at_the_automatic_code_marker() {
    let generated = kern_lookups();
    let c = compile_with("feature kern { pos c d 10; # Automatic Code\n pos e f 20; } kern;", &generated).unwrap();
    assert_eq!(gpos(&c), [2, 2, 2]);
    // Without code the generated feature is added on its own.
    let c = compile_with("", &generated).unwrap();
    assert_eq!(tags(&c), ["kern"]);
    assert_eq!(gpos(&c), [2]);
}

#[test]
fn generated_marks_and_cursive_compile() {
    let mut g = GeneratedLookups::default();
    let anchor = |x: i16, y: i16| Anchor { x: x.into(), y: y.into() };
    g.lookups.push(GeneratedLookup {
        class: "top".into(),
        flags: LookupFlags::default(),
        filter_set: None,
        rules: GeneratedRules::MarkToBase { marks: vec![(gid("acute"), anchor(0, 500))], bases: vec![(gid("a"), anchor(250, 450))] },
    });
    g.lookups.push(GeneratedLookup {
        class: "top".into(),
        flags: LookupFlags::default(),
        filter_set: Some(vec![gid("acute")]),
        rules: GeneratedRules::MarkToMark { marks: vec![(gid("acute"), anchor(0, 500))], bases: vec![(gid("grave"), anchor(0, 700))] },
    });
    g.lookups.push(GeneratedLookup {
        class: "top".into(),
        flags: LookupFlags::default(),
        filter_set: None,
        rules: GeneratedRules::MarkToLigature {
            marks: vec![(gid("acute"), anchor(0, 500))],
            ligatures: vec![(gid("f_i"), vec![Some(anchor(100, 500)), None])],
        },
    });
    g.lookups.push(GeneratedLookup {
        class: String::new(),
        flags: LookupFlags(LookupFlags::RIGHT_TO_LEFT),
        filter_set: None,
        rules: GeneratedRules::Cursive { glyphs: vec![(gid("e.fina"), Some(anchor(0, 0)), None), (gid("e.init"), None, Some(anchor(500, 0)))] },
    });
    // An empty lookup is left out rather than built.
    g.lookups.push(GeneratedLookup {
        class: "bottom".into(),
        flags: LookupFlags::default(),
        filter_set: None,
        rules: GeneratedRules::MarkToBase { marks: vec![], bases: vec![] },
    });
    let key = |t: &[u8; 4]| FeatureKey { feature: Tag::new(t), script: Tag::new(b"DFLT"), language: Tag::new(b"dflt") };
    g.features.insert(key(b"mark"), vec![0, 2, 4]);
    g.features.insert(key(b"mkmk"), vec![1]);
    g.features.insert(key(b"curs"), vec![3]);
    g.glyph_classes = BTreeMap::from([(gid("a"), GlyphClass::Base), (gid("acute"), GlyphClass::Mark), (gid("f_i"), GlyphClass::Ligature)]);
    g.carets.insert(gid("f_i"), vec![Metric::fixed(300)]);
    let c = compile_with("", &g).unwrap();
    assert_eq!(gpos(&c), [3, 4, 5, 6]);
    assert!(c.gdef.is_some());
    assert!(!c.gdef_from_code);
}

#[test]
fn generated_lookups_fea_rs_cannot_take_are_refused() {
    let mut shared = kern_lookups();
    shared.features.insert(FeatureKey { feature: Tag::new(b"mark"), script: Tag::new(b"DFLT"), language: Tag::new(b"dflt") }, vec![0]);
    let d = compile_with("", &shared).unwrap_err();
    assert!(d[0].message.contains("merged separately"), "{d:#?}");

    let mut other = kern_lookups();
    other.features.insert(FeatureKey { feature: Tag::new(b"liga"), script: Tag::new(b"DFLT"), language: Tag::new(b"dflt") }, vec![0]);
    assert!(compile_with("", &other).is_err());

    let mut outside = kern_lookups();
    if let GeneratedRules::Pair { glyph_pairs, .. } = &mut outside.lookups[0].rules {
        glyph_pairs[0].right = GlyphId16::new(60_000);
    }
    assert!(compile_with("", &outside).is_err());

    let mut variable = kern_lookups();
    if let GeneratedRules::Pair { glyph_pairs, .. } = &mut variable.lookups[0].rules {
        glyph_pairs[0].value.x_advance = Some(Metric { default: 1, deltas: vec![(Region::default(), 2)] });
    }
    assert!(compile_with("", &variable).is_err());
}

#[test]
fn validate_reports_errors_without_building() {
    let files = files("feature liga { sub f i by nosuch; } liga;");
    let d = validate(&CompileRequest::new(&files, &order()));
    assert!(d.iter().any(Diagnostic::is_error));
    let files = self::files("feature liga { sub f i by f_i; } liga;");
    assert!(!validate(&CompileRequest::new(&files, &order())).iter().any(Diagnostic::is_error));
}

#[test]
fn gpos_off_leaves_gpos_out() {
    let files = files("feature kern { pos a 10; } kern; feature liga { sub f i by f_i; } liga;");
    let order = order();
    let mut req = CompileRequest::new(&files, &order);
    req.gpos = false;
    let c = compile(&req).unwrap();
    assert!(c.gpos.is_none() && c.gsub.is_some());
}

#[test]
fn name_ids_move_above_the_source_names() {
    let files = files("feature ss01 { featureNames { name \"Alt\"; }; sub a by a.alt; } ss01;");
    let order = order();
    let mut req = CompileRequest::new(&files, &order);
    req.first_name_id = 300;
    let c = compile(&req).unwrap();
    let name = c.name.unwrap();
    // The single record's name id (at offset 6 + 6) is 300.
    assert_eq!(be16(&name, 12), 300);
}

// ------------------------------------------------------------------------------------------
// The fontTools feaLib test files: each compiles or fails with a message.

#[test]
fn fealib_test_files_compile_or_fail_with_a_message() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests/feaLib/data");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("corpus missing; skipped");
        return;
    };
    let (mut compiled, mut failed) = (0, 0);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "fea") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let mut names: Vec<String> = vec![".notdef".into()];
        let mut seen = std::collections::HashSet::new();
        for word in text.split(|c: char| c.is_whitespace() || "[](){}<>;,'\"=@\\#$".contains(c)).flat_map(|w| w.split('-')) {
            if word.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '.') && seen.insert(word.to_string()) {
                names.push(word.to_string());
            }
        }
        names.truncate(MAX_GLYPHS);
        let order = GlyphOrder::new(names).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let files = VirtualFiles::new(&name, &text).unwrap().with_include_dir(dir.clone());
        match compile(&CompileRequest::new(&files, &order)) {
            Ok(_) => compiled += 1,
            Err(d) => {
                assert!(!d.is_empty() && !d.iter().any(|d| d.message.contains("please report")), "{name}: {d:#?}");
                failed += 1;
            }
        }
    }
    eprintln!("feaLib test files: {compiled} compiled, {failed} failed with messages");
    assert!(compiled > 0);
}

#[test]
fn number_value_and_predicate_syntax_left_in_the_code_fails_with_messages() {
    let deep = format!("@a = [$[{}name like \"a\"{}]];", "(".repeat(1_000), ")".repeat(1_000));
    let long = format!("feature kern {{ pos a 1{}; }} kern;", "0".repeat(400));
    for code in [
        "feature kern { pos a ${x/0}; } kern;",
        "feature kern { pos a ${a:anchors.",
        long.as_str(),
        deep.as_str(),
        "feature kern { pos a $tracking; } kern;",
    ] {
        fails(code);
    }
}
