//! layout.convertSyntax: VOLT and FontDame layout source to feature code, and to TTX.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-convert-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn layout_convert_syntax_writes_volt_and_fontdame_as_feature_code() {
    let dir = temp("fea");
    let vtp = dir.join("project.vtp");
    std::fs::write(&vtp, "DEF_GLYPH \"f_f\" ID 1 TYPE LIGATURE COMPONENTS 2 END_GLYPH\nEND\n").unwrap();
    let mut s = Session::new();
    let r = run(&mut s, "layout.convertSyntax", json!({ "input": vtp }));
    assert_eq!(r["syntax"], "volt");
    assert_eq!(r["text"], "@GDEF_ligature = [f_f];\ntable GDEF {\n    GlyphClassDef , @GDEF_ligature, , ;\n} GDEF;\n");
    assert_eq!(r["warnings"], json!([]));

    let gsub = dir.join("gsub.txt");
    std::fs::write(&gsub, "FontDame GSUB table\nlookup\tl\tsingle\na\tb\nlookup end\n").unwrap();
    let out = dir.join("out.fea");
    let r = run(&mut s, "layout.convertSyntax", json!({ "input": [gsub], "output": out }));
    assert_eq!(r["syntax"], "fontdame");
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.contains("lookup GSUB_l {\n    sub a by b;\n} GSUB_l;"), "{written}");

    // Failures change nothing and write nothing.
    let never = dir.join("never.fea");
    assert!(s.execute("layout.convertSyntax", &json!({ "input": gsub, "output": out, "noClobber": true })).is_err());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), written);
    assert!(s.execute("layout.convertSyntax", &json!({ "input": gsub, "to": "pdf", "output": never })).is_err());
    assert!(s.execute("layout.convertSyntax", &json!({ "input": gsub, "to": "ttx", "output": never })).is_err(), "ttx needs a font");
    assert!(s.execute("layout.convertSyntax", &json!({ "input": [vtp, gsub], "output": never })).is_err(), "a project converts alone");
    assert!(s.execute("layout.convertSyntax", &json!({ "input": gsub, "tables": ["kern"], "output": never })).is_err());
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "hello").unwrap();
    assert!(s.execute("layout.convertSyntax", &json!({ "input": notes, "output": never })).is_err(), "neither syntax");
    assert!(s.execute("layout.convertSyntax", &json!({ "input": gsub, "output": gsub })).is_err(), "never over the input");
    assert!(!never.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn layout_convert_syntax_to_ttx_compiles_against_the_font() {
    let dir = temp("ttx");
    let mut s = Session::with_document();
    for name in ["a", "b"] {
        run(&mut s, "glyph.newGlyph", json!({ "name": name }));
    }
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    let txt = dir.join("smcp.txt");
    std::fs::write(
        &txt,
        "FontDame GSUB table\nscript table begin\nlatn\tdefault\t\t0\nscript table end\n\
         feature table begin\n0\tsmcp\tl\nfeature table end\nlookup\tl\tsingle\na\tb\nlookup end\n",
    )
    .unwrap();
    let r = run(&mut s, "layout.convertSyntax", json!({ "input": txt, "to": "ttx", "font": ttf }));
    let text = r["text"].as_str().unwrap();
    assert!(text.contains("<GSUB>"), "{text}");
    assert!(text.contains("<Substitution in=\"a\" out=\"b\"/>"), "{text}");
    assert!(text.contains("<FeatureTag value=\"smcp\"/>"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
