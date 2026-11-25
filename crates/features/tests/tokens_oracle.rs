//! Measure (M8 plan § 5, tokens against glyphsLib): on a two-master font with number values,
//! glyph properties and 46 predicates, every token expands to the text of glyphsLib's
//! `TokenExpander` on both masters, apart from the differences listed in [`DIFFERENT`].
//! `oracle/tokens.py` writes the expected texts from `data/tokens/{font.glyphs,cases.txt}`.
//!
//! glyphsLib lists the glyphs of an OR in the order its parts found them, FontCraft in the
//! font's order (`token.predicate.resultOrder`): the names inside brackets are compared sorted.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use std::path::Path;

use fontcraft_features::source_map::EntryKind;
use fontcraft_features::{MasterValues, tokens};
use fontcraft_model::glyphdata::GlyphData;

/// Code, FontCraft's texts on the two masters, and why they differ from glyphsLib's.
const DIFFERENT: &[(&str, [&str; 2], &str)] = &[
    (
        r#"sub [$[category == "Letter" AND case == lower OR "draft" in tags]] by x;"#,
        ["sub [A a b a.sc alpha star] by x;"; 2],
        "precedence: glyphsLib reads AND and OR right to left, FontCraft binds AND tighter",
    ),
    (
        r#"sub [$[name matches "a.*"]] by x;"#,
        ["sub [a a.sc alpha acutecomb] by x;"; 2],
        "glyphsLib finds the expression anywhere in the value, FontCraft matches the whole value",
    ),
    ("sub [$[countOfLayers => 4]] by x;", ["sub [Alpha] by x;"; 2], "glyphsLib refuses the spelling `=>`"),
    ("sub [$[export == false]] by x;", ["sub [] by x;"; 2], "glyphsLib does not know `export`; only exported glyphs are selected"),
    (
        "pos A <${A:LSB} 0 ${A:RSB*2} 0>;",
        ["pos A <20 0 80 0>;", "pos A <24 0 84 0>;"],
        "glyphsLib does not know the sidebearings the handbook documents",
    ),
    ("pos A ${tracking/4} ${shift/8};", ["pos A 3 -1;", "pos A 8 1;"], "glyphsLib truncates, FontCraft rounds half away from zero"),
    ("pos A ${2 + 4 * 5 / 7 - 3};", ["pos A 2;"; 2], "glyphsLib truncates"),
];

/// The text with the words inside brackets sorted.
fn sorted(text: &str) -> String {
    match (text.find('['), text.rfind(']')) {
        (Some(a), Some(b)) if a < b => {
            let mut words: Vec<&str> = text[a + 1..b].split_whitespace().collect();
            words.sort_unstable();
            format!("{}[{}]{}", &text[..a], words.join(" "), &text[b + 1..])
        }
        _ => text.to_string(),
    }
}

#[test]
fn tokens_expand_as_the_reference_library_expands_them() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tokens");
    let font = fontcraft_io_glyphs::read(&std::fs::read_to_string(dir.join("font.glyphs")).unwrap()).unwrap();
    let data = GlyphData::new().unwrap();
    let expected = std::fs::read_to_string(dir.join("expected.tsv")).unwrap();
    let (mut equal, mut listed, mut refused, mut wrong) = (0, 0, 0, Vec::new());
    for row in expected.lines() {
        let f: Vec<&str> = row.split('\t').collect();
        let [code, master, theirs] = f[..] else { panic!("3 columns: {row}") };
        let values = MasterValues::new(&font, &data, master).unwrap();
        let ours = tokens::expand(code, &values, EntryKind::Feature).map(|(text, _)| text);
        if let Some((_, texts, _)) = DIFFERENT.iter().find(|(c, _, _)| *c == code) {
            let want = texts[usize::from(master != font.masters[0].id)];
            match &ours {
                Ok(text) if sorted(text) == sorted(want) && sorted(text) != sorted(theirs) => listed += 1,
                _ => wrong.push(format!("{code} on {master}: {ours:?}, listed as {want:?}")),
            }
            continue;
        }
        match (&ours, theirs.starts_with('!')) {
            (Ok(text), false) if sorted(text) == sorted(theirs) => equal += 1,
            (Err(_), true) => refused += 1,
            _ => wrong.push(format!("{code} on {master}: ours {ours:?}, glyphsLib {theirs:?}")),
        }
    }
    eprintln!("{equal} equal, {refused} refused by both, {listed} different as listed");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert_eq!((listed, equal + refused + listed), (2 * DIFFERENT.len(), expected.lines().count()));
}
