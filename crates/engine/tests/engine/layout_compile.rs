//! layout.compileFeatures and layout.lintFeatures on a font exported from a document.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};
use skrifa::raw::TableProvider;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-layout-compile-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A font with three glyphs whose GPOS kerns `left.k` against `right.k` by -50.
fn font(dir: &Path) -> (Session, PathBuf) {
    let mut s = Session::with_document();
    for name in ["left.k", "right.k", "both.k"] {
        run(&mut s, "glyph.newGlyph", json!({ "name": name }));
    }
    run(&mut s, "kerning.setPair", json!({ "left": "left.k", "right": "right.k", "value": -50 }));
    let ttf = dir.join("Base.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    (s, ttf)
}

fn features(s: &mut Session, font: &Path) -> Vec<(String, String)> {
    let listed = run(s, "layout.listFeatures", json!({ "font": font, "lookups": true }));
    listed["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["table"].as_str().unwrap().to_string(), f["feature"].as_str().unwrap().to_string()))
        .collect()
}

const CODE: &str = "languagesystem DFLT dflt;
feature liga {
    sub left.k right.k by both.k;
} liga;
table head { FontRevision 2.5; } head;
table OS/2 { TypoAscender 812; } OS/2;
table name { nameid 256 \"Example\"; } name;
";

#[test]
fn layout_compile_features_builds_the_code_into_a_font_as_the_reference_compiler_does() {
    let dir = temp("build");
    let (mut s, ttf) = font(&dir);
    let fea = dir.join("liga.fea");
    std::fs::write(&fea, CODE).unwrap();

    // Every table by default: the code has no positioning, so GPOS goes, as feaLib removes it.
    let out = dir.join("Out.ttf");
    let r = run(&mut s, "layout.compileFeatures", json!({ "source": fea, "font": ttf, "output": out }));
    assert_eq!(r["syntax"], "fea");
    let tables: Vec<&str> = r["tables"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    for t in ["GSUB", "head", "OS/2", "name"] {
        assert!(tables.contains(&t), "{t} written: {r}");
    }
    assert!(r["removed"].as_array().unwrap().iter().any(|t| t == "GPOS"), "{r}");
    assert_eq!(r["features"][0]["tag"], "liga");
    assert_eq!(r["features"][0]["table"], "GSUB");
    assert_eq!(features(&mut s, &out), vec![("GSUB".to_string(), "liga".to_string())]);
    let listed = run(&mut s, "layout.listFeatures", json!({ "font": out, "lookups": true }));
    assert_eq!(listed["features"][0]["systems"][0]["lookups"][0]["type"], 4, "a ligature lookup");

    let bytes = std::fs::read(&out).unwrap();
    let f = skrifa::FontRef::new(&bytes).unwrap();
    assert_eq!(f.head().unwrap().font_revision().to_f64(), 2.5);
    let os2 = f.os2().unwrap();
    assert_eq!(os2.s_typo_ascender(), 812);
    assert_eq!(os2.us_max_context(), Some(2), "a two-glyph ligature");
    let name = f.name().unwrap();
    let example =
        name.name_record().iter().find(|r| r.string(name.string_data()).is_ok_and(|t| t.chars().eq("Example".chars()))).expect("the code's name");
    assert!(example.name_id().to_u16() >= 256);

    // Only GSUB: the font's GPOS and OS/2 stay as they were.
    let only = dir.join("OnlyGsub.ttf");
    run(&mut s, "layout.compileFeatures", json!({ "source": fea, "font": ttf, "output": only, "tables": ["GSUB"] }));
    let mut found = features(&mut s, &only);
    found.sort();
    assert_eq!(found, vec![("GPOS".to_string(), "kern".to_string()), ("GSUB".to_string(), "liga".to_string())]);
    let before = std::fs::read(&ttf).unwrap();
    let typo = |b: &[u8]| skrifa::FontRef::new(b).unwrap().os2().unwrap().s_typo_ascender();
    assert_eq!(typo(&std::fs::read(&only).unwrap()), typo(&before));

    // Without output the file goes beside the font, never over it.
    let derived = run(&mut s, "layout.compileFeatures", json!({ "source": fea, "font": ttf }));
    assert_eq!(PathBuf::from(derived["output"].as_str().unwrap()), dir.join("Base.features.ttf"));
    assert_eq!(std::fs::read(&ttf).unwrap(), before, "the input font is unchanged");

    // The built tables as TTX text.
    let ttx = dir.join("Out.ttx");
    run(&mut s, "layout.compileFeatures", json!({ "source": fea, "font": ttf, "output": ttx, "to": "ttx" }));
    let text = std::fs::read_to_string(&ttx).unwrap();
    assert!(text.contains("<GSUB") && !text.contains("<glyf"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn layout_compile_features_with_errors_writes_nothing_and_names_the_places() {
    let dir = temp("errors");
    let (mut s, ttf) = font(&dir);
    let fea = dir.join("bad.fea");
    std::fs::write(&fea, "feature liga {\n    sub left.k right.k by missing.k;\n} liga;\n").unwrap();
    let out = dir.join("Out.ttf");
    let e = s.execute("layout.compileFeatures", &json!({ "source": fea, "font": ttf, "output": out })).unwrap_err().to_string();
    assert!(e.contains("bad.fea:2:27: error:"), "{e}");
    assert!(!out.exists(), "a failed compile writes nothing");

    let good = dir.join("good.fea");
    std::fs::write(&good, CODE).unwrap();
    assert!(s.execute("layout.compileFeatures", &json!({ "source": good, "font": ttf, "tables": ["kern"] })).is_err());
    assert!(
        s.execute("layout.compileFeatures", &json!({ "source": good, "font": ttf, "emitFea": dir.join("x.fea") })).is_err(),
        "feature code needs no emitted copy"
    );
    assert!(s.execute("layout.compileFeatures", &json!({ "source": good, "font": good })).is_err(), "feature code is not a font");
    assert!(s.execute("layout.compileFeatures", &json!({ "source": [good, good], "font": ttf })).is_err(), "several sources are FontDame files only");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn layout_lint_features_reports_errors_with_line_and_column() {
    let dir = temp("lint");
    let (mut s, ttf) = font(&dir);
    std::fs::write(dir.join("extra.fea"), "sub right.k by gone.k;\n").unwrap();
    let fea = dir.join("lint.fea");
    std::fs::write(&fea, "feature liga {\n    sub left.k right.k by missing.k;\n} liga;\nfeature calt {\ninclude(extra.fea);\n} calt;\n").unwrap();

    let r = run(&mut s, "layout.lintFeatures", json!({ "source": fea, "font": ttf }));
    assert_eq!(r["glyphsChecked"], true);
    assert_eq!(r["errors"], 2, "{r}");
    let d = &r["diagnostics"][0];
    assert_eq!(d["severity"], "error");
    assert!(d["file"].as_str().unwrap().ends_with("lint.fea"), "{d}");
    assert_eq!((d["line"].as_u64(), d["column"].as_u64()), (Some(2), Some(27)), "{d}");
    let included = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["file"].as_str().is_some_and(|f| f.ends_with("extra.fea")))
        .expect("a message in the included file");
    assert_eq!((included["line"].as_u64(), included["column"].as_u64()), (Some(1), Some(16)), "{included}");

    // A list of names, and no glyph set at all.
    let names = run(&mut s, "layout.lintFeatures", json!({ "source": fea, "glyphNames": [".notdef", "left.k", "right.k", "missing.k", "gone.k"] }));
    assert_eq!(names["errors"], 0, "{names}");
    let unchecked = run(&mut s, "layout.lintFeatures", json!({ "source": fea }));
    assert_eq!(unchecked["glyphsChecked"], false);
    assert_eq!(unchecked["errors"], 0, "glyph names are not checked without a glyph set: {unchecked}");

    // A syntax error is found either way.
    let broken = dir.join("broken.fea");
    std::fs::write(&broken, "feature liga {\n    sub left.k by right.k\n} liga;\n").unwrap();
    let r = run(&mut s, "layout.lintFeatures", json!({ "source": broken }));
    assert!(r["errors"].as_u64().unwrap() >= 1, "{r}");
    assert!(r["diagnostics"][0]["line"].as_u64().is_some(), "{r}");
    assert!(s.execute("layout.lintFeatures", &json!({ "source": broken, "font": ttf, "glyphNames": ["a"] })).is_err(), "one glyph set only");
    assert!(s.execute("layout.lintFeatures", &json!({ "source": broken, "warnings": "some" })).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
