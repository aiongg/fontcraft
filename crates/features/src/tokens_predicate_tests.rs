//! Predicate tokens against a master, compared with glyphsLib's `TokenExpander` on the same
//! glyphs, and where tokens may stand.

use std::sync::Arc;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{ColorLabel, Font, Layer, Snippet};

use super::*;
use crate::target::MasterValues;

/// name, code point, category, subcategory, script, case, tags, note, colour label, exported,
/// left kerning group, layers, locked: the glyphs the glyphsLib run was given.
type Row = (
    &'static str,
    Option<u32>,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    Option<u8>,
    bool,
    Option<&'static str>,
    usize,
    bool,
);

const GLYPHS: &[Row] = &[
    ("A", Some(0x41), "Letter", "Uppercase", "latin", "upper", &["draft"], "fix apex", Some(0), true, Some("A"), 2, false),
    ("B", Some(0x42), "Letter", "Uppercase", "latin", "upper", &[], "", None, true, None, 1, false),
    ("a", Some(0x61), "Letter", "Lowercase", "latin", "lower", &["ipa"], "done", None, true, Some("a"), 1, false),
    ("b", Some(0x62), "Letter", "Lowercase", "latin", "lower", &["ipa", "math"], "", None, true, None, 1, true),
    ("a.sc", None, "Letter", "Smallcaps", "latin", "smallCaps", &["draft"], "", Some(5), true, Some("a"), 1, false),
    ("alpha", Some(0x3B1), "Letter", "Lowercase", "greek", "lower", &[], "", None, true, None, 1, false),
    ("Alpha", Some(0x391), "Letter", "Uppercase", "greek", "upper", &["math"], "", None, true, Some("A"), 3, false),
    ("zero", Some(0x30), "Number", "Decimal Digit", "latin", "noCase", &[], "", None, true, None, 1, false),
    ("zero.tf", None, "Number", "Decimal Digit", "latin", "noCase", &[], "tabular done", Some(5), true, None, 1, false),
    ("period", Some(0x2E), "Punctuation", "Other", "latin", "noCase", &[], "", None, true, None, 1, false),
    ("acutecomb", Some(0x301), "Mark", "Nonspacing", "latin", "noCase", &[], "", None, true, None, 1, false),
    ("a.alt", None, "Letter", "Lowercase", "latin", "lower", &["draft"], "", None, false, None, 1, false),
    ("star", Some(0x2605), "Symbol", "Other", "latin", "noCase", &["draft"], "", Some(5), true, None, 1, false),
];

fn oracle_font() -> Font {
    let mut font = Font::new("Oracle");
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
    for n in names {
        font.remove_glyph(&n).unwrap();
    }
    let master = font.masters[0].id.clone();
    for &(name, unicode, category, sub, script, case, tags, note, colour, export, left, layers, locked) in GLYPHS {
        font.add_glyph(name, unicode.into_iter().collect(), 600.0).unwrap();
        let mut g = font.glyph_mut(name).unwrap();
        g.info.category = Some(category.into());
        g.info.sub_category = Some(sub.into());
        g.info.script = Some(script.into());
        g.info.case = Some(case.into());
        g.info.tags = tags.iter().map(|t| t.to_string()).collect();
        g.info.note = note.into();
        g.info.color = colour.map(ColorLabel::Index);
        g.info.locked = locked;
        g.export = export;
        g.kern_groups.left = left.map(str::to_string);
        for i in 1..layers {
            let mut l = Layer::new(&master, 600.0);
            l.id = format!("backup{i}");
            g.layers.push(l);
        }
    }
    font
}

fn select(font: &Font, data: &GlyphData, predicate: &str) -> Result<Vec<String>, String> {
    let values = MasterValues::new(font, data, &font.masters[0].id).unwrap();
    glyphs(predicate, &values)
}

/// 46 predicates, each with the glyphs glyphsLib 6's `TokenExpander` selected on the same
/// glyphs, put in the font's order. glyphsLib lists the glyphs of an OR in the order its parts
/// found them; FontCraft lists them in the font's order (`token.predicate.resultOrder`). The
/// marked rows differ in content:
/// - precedence: glyphsLib reads AND and OR right to left, FontCraft binds AND tighter;
/// - `matches`: glyphsLib finds the expression anywhere in the value, FontCraft matches the
///   whole value, as Apple's predicate format does;
/// - `=>` and `export`: glyphsLib refuses them.
#[test]
fn predicate_tokens_agree_with_glyphslib() {
    let (font, data) = (oracle_font(), GlyphData::new().unwrap());
    let cases: &[(&str, &[&str])] = &[
        ("name == \"A\"", &["A"]),
        ("name = 'a'", &["a"]),
        ("name beginswith \"a\"", &["a", "a.sc", "alpha", "acutecomb"]),
        ("name endswith \".sc\"", &["a.sc"]),
        ("name contains \"ph\"", &["alpha", "Alpha"]),
        ("name BEGINSWITH \"A\"", &["A", "Alpha"]),
        ("name != \"A\"", &["B", "a", "b", "a.sc", "alpha", "Alpha", "zero", "zero.tf", "period", "acutecomb", "star"]),
        ("name <> \"B\"", &["A", "a", "b", "a.sc", "alpha", "Alpha", "zero", "zero.tf", "period", "acutecomb", "star"]),
        ("name like \"a*\"", &["a", "a.sc", "alpha", "acutecomb"]),
        ("name like \"?\"", &["A", "B", "a", "b"]),
        ("name in {\"A\", \"b\", \"star\"}", &["A", "b", "star"]),
        ("category == \"Letter\"", &["A", "B", "a", "b", "a.sc", "alpha", "Alpha"]),
        ("category == \"Number\"", &["zero", "zero.tf"]),
        ("subCategory == \"Uppercase\"", &["A", "B", "Alpha"]),
        ("script == \"greek\"", &["alpha", "Alpha"]),
        ("case == upper", &["A", "B", "Alpha"]),
        ("case == lower", &["a", "b", "alpha"]),
        ("case == smallCaps", &["a.sc"]),
        ("\"draft\" in tags", &["A", "a.sc", "star"]),
        ("\"ipa\" in tags", &["a", "b"]),
        ("NOT \"draft\" in tags", &["B", "a", "b", "alpha", "Alpha", "zero", "zero.tf", "period", "acutecomb"]),
        ("note contains \"done\"", &["a", "zero.tf"]),
        ("countOfUnicodes == 0", &["a.sc", "zero.tf"]),
        ("countOfUnicodes > 0", &["A", "B", "a", "b", "alpha", "Alpha", "zero", "period", "acutecomb", "star"]),
        ("countOfLayers > 1", &["A", "Alpha"]),
        ("countOfLayers >= 2", &["A", "Alpha"]),
        // glyphsLib: an error.
        ("countOfLayers => 3", &["Alpha"]),
        ("colorIndex == 5", &["a.sc", "zero.tf", "star"]),
        ("colorIndex == 0", &["A"]),
        ("unicode == \"0041\"", &["A"]),
        ("leftKerningGroup == \"A\"", &["A", "Alpha"]),
        // glyphsLib: an error.
        ("export == false", &[]),
        ("locked == true", &["b"]),
        ("category == \"Letter\" AND case == upper", &["A", "B", "Alpha"]),
        ("case == upper OR case == smallCaps", &["A", "B", "a.sc", "Alpha"]),
        ("script == \"latin\" AND case == lower AND \"ipa\" in tags", &["a", "b"]),
        ("\"draft\" in tags OR \"math\" in tags", &["A", "b", "a.sc", "Alpha", "star"]),
        // Precedence. glyphsLib: A a b a.sc alpha (Letter AND (lower OR draft)).
        ("category == \"Letter\" AND case == lower OR \"draft\" in tags", &["A", "a", "b", "a.sc", "alpha", "star"]),
        ("\"draft\" in tags OR category == \"Number\" AND name endswith \".tf\"", &["A", "a.sc", "zero.tf", "star"]),
        ("name contains \"a\" && case == lower", &["a", "alpha"]),
        ("name endswith \"a\" || name endswith \"B\"", &["B", "a", "alpha", "Alpha"]),
        ("NOT case == upper AND category == \"Letter\"", &["a", "b", "a.sc", "alpha"]),
        ("countOfUnicodes between {1, 1}", &["A", "B", "a", "b", "alpha", "Alpha", "zero", "period", "acutecomb", "star"]),
        ("colorIndex in {0, 5}", &["A", "a.sc", "zero.tf", "star"]),
        // Whole value. glyphsLib: a a.sc alpha Alpha acutecomb star.
        ("name matches \"a.*\"", &["a", "a.sc", "alpha", "acutecomb"]),
        ("case == upper AND NOT \"draft\" in tags", &["B", "Alpha"]),
    ];
    assert_eq!(cases.len(), 46);
    for (predicate, want) in cases {
        assert_eq!(select(&font, &data, predicate).unwrap_or_else(|e| panic!("{predicate}: {e}")), *want, "{predicate}");
    }
}

#[test]
fn class_function_reads_the_feature_classes_and_errors_are_messages() {
    let (mut font, data) = (oracle_font(), GlyphData::new().unwrap());
    let classes = &mut Arc::make_mut(&mut font.features).classes;
    classes.push(Snippet { name: "Greek".into(), code: "alpha Alpha".into(), ..Snippet::default() });
    classes.push(Snippet { name: "Caps".into(), code: "@Greek $[case == upper] a.alt".into(), ..Snippet::default() });
    assert_eq!(select(&font, &data, "name in class(Caps) AND name in class(Greek) AND name beginswith \"A\"").unwrap(), ["Alpha"]);
    // Non-exporting glyphs are left out even when a class lists them.
    assert_eq!(select(&font, &data, "name in class(Caps) AND case == lower").unwrap(), ["alpha"]);
    assert!(select(&font, &data, "name in class(Nope)").unwrap_err().contains("Nope"));
    assert!(select(&font, &data, "weight > 3").unwrap_err().contains("weight"));
    assert!(select(&font, &data, "(name == 'a'").unwrap_err().contains("cannot be read"));
}

#[test]
fn predicate_tokens_stand_inside_class_brackets_in_code() {
    let at = |code: &str| code.find("$[").unwrap();
    for (code, kind, ok) in [
        ("@Minuscule = [$[case == lower]];", EntryKind::Prefix, true),
        ("sub [a b $[name endswith \".sc\"]] by x;", EntryKind::Feature, true),
        ("sub $[case == lower] by x;", EntryKind::Feature, false),
        ("# [ in a comment\nsub $[case == lower] by x;", EntryKind::Feature, false),
        ("name \"[\"; sub $[case == lower] by x;", EntryKind::Feature, false),
        ("hyphen endash $[category == \"Number\"]", EntryKind::Class, true),
    ] {
        assert_eq!(check_placement(code, at(code), kind).is_ok(), ok, "{code}");
    }
    let code = "sub [$[name matches \"[a-z]\"] $[case == upper]] by x;";
    let second = code.rfind("$[").unwrap();
    assert!(inside_brackets(code, second));
    assert_eq!(token_len(code, 5), Some("$[name matches \"[a-z]\"]".len()));
    assert_eq!(token_len(code, 0), None);
    assert_eq!(token_len("$[name == 'a'", 0), None);
}
