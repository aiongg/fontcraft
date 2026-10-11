//! Behaviour tests of the syntax extensions: each lowering compiles to the tables its standard
//! spelling gives, and offsets map back to the written code.

use fontcraft_fea::{CompileRequest, Compiled, GlyphOrder, Tag, VirtualFiles};
use read_fonts::tables::gsub::Gsub;
use read_fonts::{FontData, FontRead};

use super::lower;

const GLYPHS: &[&str] = &[
    ".notdef",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "i",
    "idotaccent",
    "A",
    "Alpha",
    "tonos",
    "acutecomb",
    "acutecomb.case",
    "gravecomb",
    "gravecomb.case",
];

fn static_text(code: &str) -> String {
    lower(code, false).unwrap().0
}

fn compile(code: &str) -> Compiled {
    let files = VirtualFiles::new("features.fea", code).unwrap();
    let order = GlyphOrder::new(GLYPHS.iter().map(|g| g.to_string()).collect()).unwrap();
    fontcraft_fea::compile(&CompileRequest::new(&files, &order)).unwrap_or_else(|d| panic!("{code}\n{d:?}"))
}

fn gsub(code: &str) -> Vec<u8> {
    compile(code).gsub.unwrap()
}

/// Lowers `written` for a static target and compiles it: the same `GSUB` as `standard`.
fn same_gsub(written: &str, standard: &str) {
    let lowered = static_text(written);
    assert_eq!(gsub(&lowered), gsub(standard), "lowered:\n{lowered}");
}

/// The lookup indices of feature `tag`, and the number of lookups.
fn feature_lookups(gsub: &[u8], tag: &[u8; 4]) -> (Vec<u16>, u16) {
    let table = Gsub::read(FontData::new(gsub)).unwrap();
    let list = table.feature_list().unwrap();
    let mut indices: Vec<u16> = list
        .feature_records()
        .iter()
        .filter(|r| r.feature_tag() == read_fonts::types::Tag::new(tag))
        .flat_map(|r| r.feature(list.offset_data()).unwrap().lookup_list_indices().iter().map(|i| i.get()).collect::<Vec<_>>())
        .collect();
    indices.sort_unstable();
    indices.dedup();
    (indices, table.lookup_list().unwrap().lookup_count())
}

fn messages(code: &str, variable: bool) -> Vec<String> {
    lower(code, variable).unwrap_err().into_iter().map(|d| d.message).collect()
}

#[test]
fn code_without_extensions_is_unchanged_with_an_identity_map() {
    let code = "languagesystem DFLT dflt;\n@A = [a b];\nfeature liga { sub f i by c; # a comment with { and ;\n} liga;\ninclude(dir/x;y.fea);";
    let (text, map) = lower(code, false).unwrap();
    assert_eq!(text, code);
    assert!(map.is_identity());
}

#[test]
fn feature_code_delete_and_its_short_form_lower_to_sub_by_null() {
    // featureCode.delete, featureCode.deleteLowering
    let written = "feature ccmp {\n  delete tonos;\n  del acutecomb;\n} ccmp;";
    assert_eq!(static_text(written), "feature ccmp {\n  sub tonos by NULL;\n  sub acutecomb by NULL;\n} ccmp;");
    same_gsub(written, "feature ccmp { sub tonos by NULL; sub acutecomb by NULL; } ccmp;");
}

#[test]
fn feature_code_delete_contextual_keeps_the_context() {
    // featureCode.deleteContextual
    let written = "feature ccmp { delete tonos' Alpha; } ccmp;";
    assert_eq!(static_text(written), "feature ccmp { sub tonos' Alpha by NULL; } ccmp;");
    same_gsub(written, "feature ccmp { sub tonos' Alpha by NULL; } ccmp;");
}

#[test]
fn delete_without_a_glyph_is_an_error_at_the_keyword() {
    let code = "feature ccmp { delete; } ccmp;";
    let errors = lower(code, false).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].span, 15..21);
    assert!(errors[0].file.is_empty());
}

#[test]
fn a_glyph_named_like_a_keyword_is_not_rewritten() {
    let code = "@delete = [a b];\nfeature ccmp { sub a by del; } ccmp;\nlookup del { sub b by c; } del;";
    assert_eq!(static_text(code), code);
}

#[test]
fn feature_code_standalone_lookup_keeps_its_place_and_stays_out_of_the_feature() {
    // featureCode.standaloneLookup, featureCode.standaloneLowering
    let written = "feature ccmp {\n  sub a by b;\n  standalone lookup demo {\n    sub acutecomb by acutecomb.case;\n  } demo;\n  sub A acutecomb' lookup demo;\n} ccmp;";
    let lowered = static_text(written);
    assert_eq!(
        lowered,
        "feature ccmp {\n  sub a by b;\n  } ccmp;\nlookup demo {\n    sub acutecomb by acutecomb.case;\n  } demo;\nfeature ccmp {\n  sub A acutecomb' lookup demo;\n} ccmp;"
    );
    let table = gsub(&lowered);
    // Lookups in file order: a → b, demo, the contextual rule. The feature skips demo.
    assert_eq!(feature_lookups(&table, b"ccmp"), (vec![0, 2], 3));
    assert_eq!(
        table,
        gsub(
            "feature ccmp { sub a by b; } ccmp;\nlookup demo { sub acutecomb by acutecomb.case; } demo;\nfeature ccmp { sub A acutecomb' lookup demo; } ccmp;"
        )
    );
}

#[test]
fn feature_code_standalone_lowering_repeats_script_language_and_lookupflag() {
    // featureCode.standaloneLowering
    let written = "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\nfeature locl {\nscript latn;\nlanguage TRK;\nlookupflag IgnoreMarks;\nsub i by idotaccent;\nstandalone lookup x useExtension {\nsub a by b;\n};\nsub i' lookup x a;\n} locl;";
    let lowered = static_text(written);
    assert_eq!(
        lowered,
        "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\nfeature locl {\nscript latn;\nlanguage TRK;\nlookupflag IgnoreMarks;\nsub i by idotaccent;\n} locl;\nlookup x useExtension { lookupflag IgnoreMarks;\nsub a by b;\n} x;\nfeature locl { script latn; language TRK; lookupflag IgnoreMarks;\nsub i' lookup x a;\n} locl;"
    );
    let compiled = compile(&lowered);
    let locl = compiled.features.iter().find(|f| f.tag == Tag::new(b"locl")).unwrap();
    assert!(locl.language_systems.contains(&(Tag::new(b"latn"), Tag::new(b"TRK "))), "{:?}", locl.language_systems);
    assert_eq!(feature_lookups(compiled.gsub.as_deref().unwrap(), b"locl"), (vec![0, 2], 3));
}

#[test]
fn a_script_statement_resets_the_flags_the_split_repeats() {
    let written = "feature ccmp { lookupflag IgnoreMarks; script latn; standalone lookup x { sub a by b; } x; } ccmp;";
    let lowered = static_text(written);
    assert!(lowered.contains("lookup x { sub a by b; } x;"), "{lowered}");
    assert!(lowered.ends_with("feature ccmp { script latn; } ccmp;"), "{lowered}");
}

#[test]
fn feature_code_standalone_redundant_outside_a_feature() {
    // featureCode.standaloneRedundant
    let written = "standalone lookup demo { sub a by b; } demo;\nfeature ccmp { sub c' lookup demo; } ccmp;";
    let lowered = static_text(written);
    assert_eq!(lowered, "lookup demo { sub a by b; } demo;\nfeature ccmp { sub c' lookup demo; } ccmp;");
    same_gsub(written, &lowered);
}

#[test]
fn standalone_must_introduce_a_lookup_block() {
    assert_eq!(messages("feature ccmp { standalone sub a by b; } ccmp;", false).len(), 1);
    assert_eq!(messages("feature ccmp { standalone lookup demo; } ccmp;", false).len(), 1);
    assert_eq!(messages("table GDEF { standalone lookup x { sub a by b; } x; } GDEF;", false).len(), 1);
}

#[test]
fn feature_code_optional_lookup_end_name() {
    // featureCode.optionalLookupEndName
    let written = "lookup one { sub a by b; };\nfeature liga { lookup two { sub c by d; } ; } liga;";
    let lowered = static_text(written);
    assert_eq!(lowered, "lookup one { sub a by b; } one;\nfeature liga { lookup two { sub c by d; } two ; } liga;");
    same_gsub(written, "lookup one { sub a by b; } one;\nfeature liga { lookup two { sub c by d; } two; } liga;");
}

#[test]
fn feature_code_multiple_languages_pass_through_and_apply_to_each() {
    // featureCode.multipleLanguages, featureCode.multipleLanguagesCompiler
    let written = "languagesystem latn dflt;\nfeature locl {\nscript latn;\nlanguage AZE CRT TRK;\nsub i by idotaccent;\n} locl;";
    // The compiler reads one tag per statement: each language gets the statement and the rules.
    let (text, map) = lower(written, false).unwrap();
    assert_eq!(
        text,
        "languagesystem latn dflt;\nfeature locl {\nscript latn;\nlanguage AZE;\nsub i by idotaccent;\n\nlanguage CRT;\nsub i by idotaccent;\n\nlanguage TRK;\nsub i by idotaccent;\n} locl;"
    );
    let rule = text.find("sub i").unwrap();
    assert_eq!(map.source_range(rule..rule + 5), written.find("sub i").unwrap()..written.find("sub i").unwrap() + 5);
    // A named lookup is written once and referred to by the other languages.
    let named = "feature locl {\nscript latn;\nlanguage AZE TRK exclude_dflt;\nlookup dotted {\n  sub i by idotaccent;\n} dotted;\nsub a by b;\nscript cyrl;\nsub c by d;\n} locl;";
    assert_eq!(
        static_text(named),
        "feature locl {\nscript latn;\nlanguage AZE exclude_dflt;\nlookup dotted {\n  sub i by idotaccent;\n} dotted;\nsub a by b;\n\nlanguage TRK exclude_dflt;\nlookup dotted;\nsub a by b;\nscript cyrl;\nsub c by d;\n} locl;"
    );
    let compiled = compile(&text);
    // Each language has its own copy of the rules, so its own locl record.
    let systems: Vec<(Tag, Tag)> = compiled.features.iter().filter(|f| f.tag == Tag::new(b"locl")).flat_map(|f| f.language_systems.clone()).collect();
    for lang in [b"AZE ", b"CRT ", b"TRK "] {
        assert!(systems.contains(&(Tag::new(b"latn"), Tag::new(lang))), "{lang:?}: {systems:?}");
    }
}

#[test]
fn feature_code_multiple_substitution_classes_pair_members_by_position() {
    // featureCode.multipleSubstitutionClasses
    let written = "@Sym = [a b];\n@SymL = [c d];\n@SymR = [e f];\nfeature ccmp { sub @Sym by @SymL @SymR; } ccmp;";
    assert_eq!(static_text(written), written);
    same_gsub(written, "feature ccmp { sub a by c e; sub b by d f; } ccmp;");
    let contextual = "@Sym = [a b];\n@SymL = [c d];\n@SymR = [e f];\nfeature ccmp { sub A @Sym' A by @SymL @SymR; } ccmp;";
    assert_eq!(static_text(contextual), contextual);
    assert!(compile(contextual).gsub.is_some());
}

#[test]
fn feature_code_multiple_substitution_singletons_repeat() {
    // featureCode.multipleSubstitutionSingletons
    let named = "@Sym = [a b];\n@SymL = [c d];\n@One = [e];\nfeature ccmp { sub @Sym by @SymL @One; } ccmp;";
    assert_eq!(static_text(named), "@Sym = [a b];\n@SymL = [c d];\n@One = [e];\nfeature ccmp { sub @Sym by @SymL e; } ccmp;");
    same_gsub(named, "feature ccmp { sub a by c e; sub b by d e; } ccmp;");
    let inline = "@Sym = [a b];\n@SymL = [c d];\nfeature ccmp { sub @Sym by [ e ] @SymL; } ccmp;";
    assert_eq!(static_text(inline), "@Sym = [a b];\n@SymL = [c d];\nfeature ccmp { sub @Sym by e @SymL; } ccmp;");
    same_gsub(inline, "feature ccmp { sub a by e c; sub b by e d; } ccmp;");
    // A single substitution keeps its class.
    let single = "feature ccmp { sub [a] by [b]; } ccmp;";
    assert_eq!(static_text(single), single);
}

#[test]
fn feature_code_lookupflag_replaces_the_previous_flags() {
    // featureCode.lookupflagReplaces
    let written = "feature liga { lookupflag IgnoreMarks; sub a by b; lookupflag RightToLeft; sub c by d; } liga;";
    assert_eq!(static_text(written), written);
    same_gsub(written, "feature liga { lookupflag IgnoreMarks; sub a by b; lookupflag 1; sub c by d; } liga;");
}

#[test]
fn feature_code_empty_lookupflag_clears_all_flags() {
    // featureCode.emptyLookupflag
    let written = "feature liga { lookupflag IgnoreMarks; sub a by b; lookupflag; sub c by d; } liga;";
    assert_eq!(static_text(written), "feature liga { lookupflag IgnoreMarks; sub a by b; lookupflag 0; sub c by d; } liga;");
    same_gsub(written, "feature liga { lookupflag IgnoreMarks; sub a by b; lookupflag 0; sub c by d; } liga;");
}

#[test]
fn ifdef_variable_blocks_are_kept_for_the_target_they_name() {
    // featureCode.ifdefVariable, featureCode.ifndefVariable (static and variable halves)
    let code = "feature ss01 {\n#ifdef VARIABLE\nsub a by b;\n#endif\n#ifndef VARIABLE\nsub a by c;\n#endif\n} ss01;";
    assert_eq!(lower(code, false).unwrap().0, "feature ss01 {\n\n#ifndef VARIABLE\nsub a by c;\n#endif\n} ss01;");
    assert_eq!(lower(code, true).unwrap().0, "feature ss01 {\n#ifdef VARIABLE\nsub a by b;\n#endif\n\n} ss01;");
    same_gsub(code, "feature ss01 { sub a by c; } ss01;");
}

#[test]
fn an_unterminated_block_ends_with_its_enclosing_block() {
    let code = "feature ss01 {\n#ifndef VARIABLE\nsub a by c;\n} ss01;\nfeature ss02 { sub b by d; } ss02;";
    assert_eq!(lower(code, true).unwrap().0, "feature ss01 {\n} ss01;\nfeature ss02 { sub b by d; } ss02;");
    assert_eq!(lower(code, false).unwrap().0, code);
}

#[test]
fn other_directives_are_errors_at_the_directive() {
    let code = "#ifdef BOLD\nsub a by b;\n#endif";
    let errors = lower(code, false).unwrap_err();
    assert_eq!(errors.first().map(|e| e.span.clone()), Some(0..11));
    assert_eq!(messages("#ifdef VARIABLE\n#else\n#endif", true).len(), 1);
    assert_eq!(messages("#ifndef VARIABLE\n#ifdef VARIABLE\n#endif\n#endif", false).len(), 2);
    assert_eq!(messages("sub a by b;\n#endif", false).len(), 1);
    // A comment that only starts with the same letters is a comment.
    assert!(lower("# ifdef is not a directive\n#ifdefault\n#endiff", false).is_ok());
}

#[test]
fn offsets_of_the_lowered_text_map_back_to_the_written_code() {
    let code = "#ifdef VARIABLE\nsub x by y;\n#endif\nfeature ccmp {\n  delete tonos;\n  standalone lookup demo { sub a by b; };\n  lookupflag;\n  sub A tonos' lookup demo;\n} ccmp;";
    let (text, map) = lower(code, false).unwrap();
    for needle in ["feature ccmp", "tonos", "sub a by b", "lookupflag", "sub A tonos'", "\n} ccmp;"] {
        let out = text.find(needle).unwrap();
        let input = code.find(needle).unwrap();
        assert_eq!(map.source_offset(out), input, "{needle}");
    }
    // Inserted text points at the code it stands for.
    let null = text.find("NULL").unwrap();
    assert_eq!(map.source_offset(null), code.find("tonos;").unwrap() + "tonos".len());
}

#[test]
fn nesting_is_capped_without_recursion() {
    let deep = format!("feature ccmp {{ sub a by {}b; }} ccmp;", "[".repeat(200));
    let errors = lower(&deep, false).unwrap_err();
    assert_eq!(errors.len(), 1);
    let flat = format!("@A = [{}];", "[]".repeat(10_000));
    assert!(lower(&flat, false).is_ok());
}

#[test]
fn many_standalone_lookups_in_a_long_named_feature_stop_at_the_size_cap() {
    let tag = "x".repeat(1 << 20);
    let mut code = format!("feature {tag} {{ lookupflag {tag}; ");
    for i in 0..20 {
        code.push_str(&format!("standalone lookup l{i} {{ sub a by b; }} l{i}; "));
    }
    code.push_str(&format!("}} {tag};"));
    let errors = lower(&code, false).unwrap_err();
    assert!(errors.iter().any(|e| e.message.contains("grows past")), "{errors:?}");
}
