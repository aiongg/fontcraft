#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_model::{Component, Font, Group, Path, Shape, Snippet};
use serde_json::json;

use super::fea::Pruner;
use crate::Session;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fontcraft-subset-source-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn snippet(name: &str, code: &str) -> Snippet {
    Snippet { name: name.into(), code: code.into(), ..Snippet::default() }
}

/// Letters, small caps, an accented letter built from components, a ligature, kerning with
/// groups, named groups, classes and feature code.
fn source() -> Font {
    let mut f = Font::new("Subset Sans");
    f.glyphs.retain(|g| g.name == ".notdef");
    for (name, cps) in [
        ("A", vec![0x41]),
        ("B", vec![0x42]),
        ("C", vec![0x43]),
        ("A.sc", vec![]),
        ("B.sc", vec![]),
        ("C.sc", vec![]),
        ("acutecomb", vec![0x301]),
        ("Aacute", vec![0xC1]),
        ("f", vec![0x66]),
        ("i", vec![0x69]),
        ("f_i", vec![]),
    ] {
        f.add_glyph(name, cps, 600.0).unwrap();
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 500.0, 700.0)));
    }
    f.glyph_mut("Aacute").unwrap().layers[0].shapes =
        vec![Shape::Component(Component::new("A", 0.0, 0.0)), Shape::Component(Component::new("acutecomb", 250.0, 0.0))];
    f.glyph_mut("A").unwrap().kern_groups.right = Some("A".into());
    f.glyph_mut("C").unwrap().kern_groups.left = Some("C".into());
    let kerning = Arc::make_mut(&mut f.kerning);
    kerning.ltr.set("m01", "A", "B", -40.0);
    kerning.ltr.set("m01", "A", "C", -30.0);
    kerning.ltr.set("m01", "@MMK_L_A", "@MMK_R_C", -20.0);
    *Arc::make_mut(&mut f.groups) =
        vec![Group { name: "misc".into(), members: vec!["A".into(), "C".into()] }, Group { name: "onlyC".into(), members: vec!["C".into()] }];
    let features = Arc::make_mut(&mut f.features);
    features.classes = vec![snippet("Upper", "A B C"), snippet("Gone", "C")];
    features.features = vec![
        snippet("liga", "sub f i by f_i;\n"),
        snippet("smcp", "sub [A B C] by [A.sc B.sc C.sc];\n"),
        snippet("kern", "pos @Gone A -10;\npos A B -20;\n"),
    ];
    f
}

#[test]
fn a_source_subset_keeps_components_and_loses_what_names_removed_glyphs() {
    let d = dir("behaviour");
    let input = d.join("Subset.fontcraft");
    crate::io::save(&source(), &input, crate::io::SaveFormat::Native).unwrap();
    let before = std::fs::read(&input).unwrap();
    let mut s = Session::new();
    let r = s.execute("font.subset", &json!({"font": input, "keep": {"glyphs": ["Aacute", "B", "A.sc", "B.sc"]}})).unwrap();
    let output = d.join("Subset.subset.fontcraft");
    assert_eq!(r["output"], json!(output.display().to_string()), "{r}");
    assert_eq!(r["retained"], json!([".notdef", "A", "B", "A.sc", "B.sc", "acutecomb", "Aacute"]), "components come along: {r}");
    assert_eq!(r["featureRulesRemoved"], json!(2), "{r}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "the input is never changed");

    let (font, _) = crate::io::open(&output, None).unwrap();
    assert!(font.glyph("C").is_none() && font.glyph("f_i").is_none());
    let pairs: Vec<(String, String)> =
        font.kerning.ltr.0[0].firsts.iter().flat_map(|f| f.seconds.iter().map(|(s, _)| (f.first.clone(), s.clone()))).collect();
    assert_eq!(pairs, [("A".to_string(), "B".to_string())], "pairs with C or its emptied group go");
    assert_eq!(font.groups.len(), 1);
    assert_eq!(font.groups[0].members, ["A"]);
    let classes: Vec<(&str, &str)> = font.features.classes.iter().map(|c| (c.name.as_str(), c.code.as_str())).collect();
    assert_eq!(classes, [("Upper", "A B")]);
    let code: Vec<&str> = font.features.features.iter().map(|f| f.code.as_str()).collect();
    assert_eq!(code, ["", "sub [A B] by [A.sc B.sc];\n", "pos A B -20;\n"]);

    // Export flags narrow a selection; Font DICTs and containers are refused for a source.
    let mut hidden = source();
    hidden.glyph_mut("B").unwrap().export = false;
    let flagged = d.join("Flagged.fontcraft");
    crate::io::save(&hidden, &flagged, crate::io::SaveFormat::Native).unwrap();
    let r = s.execute("font.subset", &json!({"font": flagged, "keep": {"exportFlags": true, "excludeGlyphs": ["f", "i", "f_i"]}})).unwrap();
    let kept: Vec<&str> = r["retained"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(!kept.contains(&"B") && kept.contains(&"C") && !kept.contains(&"f"), "{kept:?}");
    for bad in [
        json!({"font": input, "keep": {"glyphs": ["A"], "fd": [0]}, "output": d.join("x.fontcraft")}),
        json!({"font": input, "keep": {"glyphs": ["A"]}, "container": "woff2", "output": d.join("x.fontcraft")}),
        json!({"font": input, "keep": {"glyphs": ["nosuch"]}, "output": d.join("x.fontcraft")}),
    ] {
        assert!(s.execute("font.subset", &bad).is_err(), "{bad}");
    }
    assert!(!d.join("x.fontcraft").exists(), "a refused subset writes nothing");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn feature_code_loses_removed_glyphs_rule_by_rule() {
    let names = ["a", "b", "c", "d", "e", "a.sc", "b.sc", "c.sc", "acute", "grave", "f", "i", "f_i"];
    let all: HashSet<String> = names.iter().map(|n| n.to_string()).collect();
    let removed: HashSet<String> = ["c", "c.sc", "grave", "f_i"].iter().map(|n| n.to_string()).collect();
    let mut p = Pruner::new(&all, &removed);
    let code = concat!(
        "@lc = [a-e];\n",
        "markClass [acute grave] <anchor 0 500> @TOP;\n",
        "markClass grave <anchor 0 500> @GRAVE;\n",
        "lookup L {\n  sub f i by f_i; # ligature\n} L;\n",
        "pos base [a c] <anchor 250 500> mark @TOP;\n",
        "pos base a <anchor 250 500> mark @GRAVE;\n",
        "sub [a c]' lookup L [b d];\n",
        "sub [a b c] by [a.sc b.sc c.sc];\n",
        "table GDEF {\n GlyphClassDef [a c], [f_i], [acute grave], ;\n} GDEF;\n",
    );
    let expected = concat!(
        "@lc = [a b d e];\n",
        "markClass [acute] <anchor 0 500> @TOP;\n",
        "lookup L {\n   # ligature\n} L;\n",
        "pos base [a] <anchor 250 500> mark @TOP;\n",
        "sub [a]' lookup L [b d];\n",
        "sub [a b] by [a.sc b.sc];\n",
        "table GDEF {\n GlyphClassDef [a], , [acute], ;\n} GDEF;\n",
    );
    assert_eq!(p.code(code), expected);
    assert_eq!(p.rules_removed, 3, "the second markClass, the ligature and the attachment to the emptied mark class");

    // A class entry emptied by the subset is removed, and so are the rules that use it.
    assert_eq!(p.class_entry("Gone", "c c.sc"), None);
    assert_eq!(p.class_entry("Lower", "a c b").as_deref(), Some("a b"));
    assert_eq!(p.code("pos @Gone a -10;\npos a b -20;\n"), "pos a b -20;\n");
    assert_eq!(p.classes_removed, ["Gone"]);
    // Code that names no removed glyph comes back unchanged.
    assert_eq!(p.code("sub a by b; # c\n"), "sub a by b; # c\n");
}
