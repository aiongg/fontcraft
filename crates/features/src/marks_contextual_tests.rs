#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_fea::{CompileRequest, GlyphOrder, VirtualFiles};
use read_fonts::tables::gpos::{Gpos, PositionLookup};
use read_fonts::{FontData, FontRead};

use super::super::{MarkAnchor, MarkGlyph, MarkInput, MarkLimits, MasterAnchors};
use super::*;

fn glyph(name: &str, cps: &[u32], category: &str, sub: Option<&str>, anchors: &[(&str, f64, f64)]) -> MarkGlyph {
    MarkGlyph {
        name: name.into(),
        codepoints: cps.to_vec(),
        category: Some(category.into()),
        sub_category: sub.map(str::to_string),
        opentype_category: None,
        anchors: vec![MasterAnchors {
            master: "m01".into(),
            anchors: anchors.iter().map(|(n, x, y)| MarkAnchor { name: (*n).into(), x: *x, y: *y }).collect(),
        }],
    }
}

/// The font the fontmake comparison was made with (`docs/plans/M8.md` § 5, contextual
/// anchors): `n` with `*top` after `o` and `*bottom.alt` before `o` with a filtering set,
/// `acutecomb` with `*top` after `n` (mark to mark), `f_i` with `*top_2` before `o`.
fn probe() -> (MarkInput, AnchorContexts) {
    let mark = ("Mark", Some("Nonspacing"));
    let input = MarkInput {
        glyphs: vec![
            glyph(".notdef", &[], "Separator", None, &[]),
            glyph(
                "n",
                &[0x6E],
                "Letter",
                Some("Lowercase"),
                &[("top", 250.0, 500.0), ("*top", 300.0, 520.0), ("bottom", 250.0, 0.0), ("*bottom.alt", 200.0, -10.0)],
            ),
            glyph("o", &[0x6F], "Letter", Some("Lowercase"), &[("top", 260.0, 500.0)]),
            glyph("acutecomb", &[0x301], mark.0, mark.1, &[("_top", 100.0, 500.0), ("top", 100.0, 700.0), ("*top", 120.0, 720.0)]),
            glyph("gravecomb", &[0x300], mark.0, mark.1, &[("_top", 110.0, 500.0), ("_bottom", 110.0, 0.0)]),
            glyph("f_i", &[], "Letter", Some("Ligature"), &[("top_1", 150.0, 700.0), ("top_2", 450.0, 700.0), ("*top_2", 470.0, 750.0)]),
        ],
        ..Default::default()
    };
    let mut contexts = BTreeMap::new();
    for (g, a, c) in [
        ("n", "*top", "o *"),
        ("n", "*bottom.alt", "lookupflag UseMarkFilteringSet [gravecomb]; * o"),
        ("acutecomb", "*top", "n *"),
        ("f_i", "*top_2", "* & o"),
    ] {
        contexts.insert((g.to_string(), a.to_string()), c.to_string());
    }
    (input, AnchorContexts(contexts))
}

fn request(input: &MarkInput) -> MarkRequest<'_> {
    MarkRequest { input, master: "m01", tags: MarkTags::default(), limits: MarkLimits::default(), gsub: None }
}

fn written(input: &MarkInput, contexts: &AnchorContexts) -> Option<ContextualMarks> {
    let marks = request(input);
    contextual_marks(&ContextualRequest { marks: &marks, contexts, values: None }).unwrap()
}

/// The line of `code` holding `needle`, and its index.
fn find(code: &str, needle: &str) -> usize {
    code.lines().position(|l| l.trim() == needle).unwrap_or_else(|| panic!("no line `{needle}` in:\n{code}"))
}

#[test]
fn contextual_anchor_names_read_as_the_reference_compiler_reads_them() {
    let read = |name: &str| contextual_name(name).map(|n| n.map(|n| (n.key, n.number)));
    assert_eq!(read("*top"), Ok(Some(("top".into(), None))));
    assert_eq!(read("*bottom.reh"), Ok(Some(("bottom".into(), None))), "the suffix after the dot only tells anchors apart");
    assert_eq!(read("*top_2"), Ok(Some(("top".into(), Some(2)))), "a ligature component");
    assert_eq!(read("*top_2.noon"), Ok(Some(("top".into(), Some(2)))));
    assert_eq!(read("*top2"), Ok(Some(("top2".into(), None))), "digits without an underscore are part of the key");
    assert_eq!(read("top"), Ok(None), "no asterisk");
    assert_eq!(read("*1st"), Ok(None), "a key that does not start with a letter is ignored");
    assert!(read("*_top").is_err(), "a mark anchor cannot be contextual");
    assert!(read("*top_0").is_err(), "components count from 1");
    assert!(read("*top_99999999999999999999999").is_err(), "a number that does not fit is refused, not a crash");
}

#[test]
fn contextual_lookups_dispatch_as_fontmake_builds_them() {
    let (input, contexts) = probe();
    let found = written(&input, &contexts).expect("contextual rules");
    let code = &found.code;
    assert_eq!(found.rules, 4, "{code}");
    // The combining marks' script extensions include Tai Le, a script that uses abvm, so abvm
    // gets their mark-to-mark lookup, as fontmake writes it.
    assert_eq!(found.features, [ABVM, MARK, MKMK]);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);

    // The lookups the contexts call, in fontmake's order: base contexts longest first, then the
    // ligature ones.
    let bottom = find(code, "lookup gen_ContextualMark_0 {");
    assert_eq!(code.lines().nth(bottom + 1).unwrap().trim(), "pos base \\n <anchor 200 -10> mark @gen_mark_bottom;");
    let top = find(code, "lookup gen_ContextualMark_1 {");
    assert_eq!(code.lines().nth(top + 1).unwrap().trim(), "pos base \\n <anchor 300 520> mark @gen_mark_top;");
    let lig = find(code, "lookup gen_ContextualMark_2 {");
    assert_eq!(code.lines().nth(lig + 1).unwrap().trim(), "pos ligature \\f_i <anchor NULL> ligComponent <anchor 470 750> mark @gen_mark_top;");

    // One dispatch lookup per lookupflag prefix; `*` is the glyph, `&` the marks.
    let d0 = find(code, "lookup gen_ContextualMarkDispatch_0 {");
    assert_eq!(code.lines().nth(d0 + 1).unwrap().trim(), "lookupflag UseMarkFilteringSet [gravecomb];");
    find(code, "pos [\\n] [\\gravecomb]' lookup gen_ContextualMark_0 o;");
    let d1 = find(code, "lookup gen_ContextualMarkDispatch_1 {");
    assert!(d0 < d1 && lig < d0, "called lookups come before the dispatch lookups");
    find(code, "pos o [\\n] [\\acutecomb \\gravecomb]' lookup gen_ContextualMark_1;");
    find(code, "pos [\\f_i] [\\acutecomb \\gravecomb]' lookup gen_ContextualMark_2 o;");
    find(code, "pos n [\\acutecomb] [\\acutecomb \\gravecomb]' lookup gen_ContextualMarkToMark_0;");

    // The plain lookups first, the dispatch lookups after them, in each feature.
    let mark = find(code, "feature mark {");
    let refs: Vec<&str> = code.lines().skip(mark + 1).take_while(|l| !l.starts_with('}')).map(str::trim).collect();
    assert_eq!(
        refs,
        [
            "lookup gen_mark_mark2base_bottom;",
            "lookup gen_mark_mark2base_top;",
            "lookup gen_mark_mark2liga_top;",
            "lookup gen_ContextualMarkDispatch_0;",
            "lookup gen_ContextualMarkDispatch_1;",
        ]
    );
    let mkmk = find(code, "feature mkmk {");
    let refs: Vec<&str> = code.lines().skip(mkmk + 1).take_while(|l| !l.starts_with('}')).map(str::trim).collect();
    assert_eq!(refs, ["lookup gen_mkmk_mark2mark_top;", "lookup gen_ContextualMarkToMarkDispatch_0;"]);
    let left = found.tags_left(MarkTags::default());
    assert!(!left.mark && !left.mkmk && !left.abvm && !left.blwm && left.curs && left.gdef, "the code replaces the generated mark features");
}

#[test]
fn contextual_code_compiles_to_chained_lookups_after_the_plain_ones() {
    let (input, contexts) = probe();
    let found = written(&input, &contexts).unwrap();
    let empty = VirtualFiles::new("features.fea", "languagesystem DFLT dflt;\n").unwrap();
    let files = with_code(&empty, &found.code).unwrap();
    assert!(files.root_text().ends_with(&format!("include({CONTEXTUAL_FILE});\n")));
    let order = GlyphOrder::new(input.glyphs.iter().map(|g| g.name.clone()).collect()).unwrap();
    let compiled = fontcraft_fea::compile(&CompileRequest::new(&files, &order)).unwrap_or_else(|d| panic!("{}\n{d:?}", found.code));
    let bytes = compiled.gpos.expect("GPOS");
    let gpos = Gpos::read(FontData::new(&bytes)).unwrap();
    let kinds: Vec<&str> = gpos
        .lookup_list()
        .unwrap()
        .lookups()
        .iter()
        .map(|l| match l.unwrap() {
            PositionLookup::MarkToBase(_) => "base",
            PositionLookup::MarkToLig(_) => "lig",
            PositionLookup::MarkToMark(_) => "mkmk",
            PositionLookup::ChainContextual(_) => "chain",
            _ => "other",
        })
        .collect();
    // fontmake's order: abvm's mark-to-mark lookup comes last.
    assert_eq!(kinds, ["base", "base", "lig", "base", "base", "lig", "chain", "chain", "mkmk", "mkmk", "chain", "mkmk"]);
}

#[test]
fn contexts_that_cannot_be_written_are_left_out_with_a_warning() {
    let (input, _) = probe();
    let mut contexts = BTreeMap::new();
    contexts.insert(("n".to_string(), "*top".to_string()), "$[name like \"o*\"] *".to_string());
    contexts.insert(("n".to_string(), "*bottom.alt".to_string()), "} lookup x { *".to_string());
    contexts.insert(("f_i".to_string(), "*top_2".to_string()), "o".to_string());
    let marks = request(&input);
    let contexts = AnchorContexts(contexts);
    let found = contextual_marks(&ContextualRequest { marks: &marks, contexts: &contexts, values: None }).unwrap();
    assert!(found.is_none(), "no contextual rule is left, so the generated lookups stay");

    // Through the compiler's entry the reasons reach the warnings; an empty context too.
    let mut only = BTreeMap::new();
    only.insert(("n".to_string(), "*top".to_string()), "  ;\n".to_string());
    only.insert(("acutecomb".to_string(), "*top".to_string()), "n *".to_string());
    let contexts = AnchorContexts(only);
    let found = contextual_marks(&ContextualRequest { marks: &marks, contexts: &contexts, values: None }).unwrap().unwrap();
    assert_eq!(found.rules, 1);
    assert_eq!(found.features, [ABVM, MARK, MKMK], "the plain mark lookups move to the code with the contextual ones");
    assert!(found.warnings.iter().any(|w| w.contains("empty")), "{:?}", found.warnings);
}

#[test]
fn no_contexts_or_no_mark_features_write_nothing() {
    let (input, contexts) = probe();
    assert!(written(&input, &AnchorContexts::default()).is_none());
    let mut marks = request(&input);
    marks.tags = MarkTags { mark: false, mkmk: false, abvm: false, blwm: false, ..MarkTags::default() };
    assert!(contextual_marks(&ContextualRequest { marks: &marks, contexts: &contexts, values: None }).unwrap().is_none());
    // An anchor without a context is a plain anchor of no group, as the reference reads it.
    let mut one = contexts.0.clone();
    one.retain(|(g, _), _| g == "acutecomb");
    let found = written(&input, &AnchorContexts(one)).unwrap();
    assert_eq!(found.rules, 1);
    assert!(!found.code.contains("gen_ContextualMark_"), "only the mark-to-mark context");
}
