//! Build Glyphs from Constructions (M13.17) through the command: rules run in order and later
//! rules use earlier results, one undo step, preview stores nothing, overwrite and `?` keep
//! existing glyphs, `*` decomposes, kerning with `\`, rules read from a file, and hostile rules
//! refused or listed without changing the font.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, ColorLabel, Font, Glyph, Layer, Path, Shape};
use serde_json::{Value, json};

const ID: &str = "font.buildFromConstructions";

fn run(s: &mut Session, p: Value) -> Value {
    s.execute(ID, &p).unwrap_or_else(|e| panic!("{ID}: {e}"))
}

fn refused(s: &mut Session, p: Value, needle: &str) {
    let before = s.doc().unwrap().font.clone();
    let err = s.execute(ID, &p).expect_err(ID).to_string();
    assert!(err.contains(needle), "expected an error naming {needle:?}, got {err:?}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "a refused command changes nothing");
}

fn put(font: &mut Font, name: &str, width: f64, path: Option<Path>, anchors: &[(&str, f64, f64)]) {
    let mut g = Glyph::new(name);
    let mut l = Layer::new("m01", width);
    if let Some(p) = path {
        l.shapes.push(Shape::Path(p));
    }
    l.anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
    g.layers.push(l);
    font.glyphs.remove(name);
    font.glyphs.push(g);
}

fn session() -> Session {
    let mut f = Font::new("Rules");
    put(&mut f, "A", 600.0, Some(Path::rect(50.0, 0.0, 550.0, 700.0)), &[("top", 300.0, 700.0)]);
    put(&mut f, "acute", 300.0, Some(Path::rect(200.0, 500.0, 300.0, 600.0)), &[("_top", 250.0, 500.0)]);
    put(&mut f, "V", 600.0, Some(Path::rect(0.0, 0.0, 600.0, 700.0)), &[]);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

/// Each component's glyph and offset on the first master.
fn placed(s: &Session, glyph: &str) -> Vec<(String, f64, f64)> {
    s.font().unwrap().glyph(glyph).unwrap().layers[0]
        .shapes
        .iter()
        .filter_map(|sh| match sh {
            Shape::Component(c) => {
                let m = c.transform.affine().as_coeffs();
                Some((c.glyph.clone(), m[4], m[5]))
            }
            _ => None,
        })
        .collect()
}

fn parts(list: &[(&str, f64, f64)]) -> Vec<(String, f64, f64)> {
    list.iter().map(|(n, x, y)| (n.to_string(), *x, *y)).collect()
}

#[test]
fn font_build_from_constructions_builds_in_order_in_one_undo_step() {
    let mut s = session();
    let rules = "Aacute = A + acute@center,top # built\nAacuteAacute = Aacute & Aacute\n";
    let r = run(&mut s, json!({"rules": rules}));
    assert_eq!(r["errors"], json!([]));
    assert_eq!(r["built"].as_array().unwrap().len(), 2);
    assert_eq!(r["built"][0]["created"], json!(true));
    assert_eq!(r["built"][0]["layers"][0]["components"][1]["transform"], json!([1.0, 0.0, 0.0, 1.0, 50.0, 200.0]));
    // The automatic alignment that runs after every edit leaves rule positions alone.
    assert_eq!(placed(&s, "Aacute"), parts(&[("A", 0.0, 0.0), ("acute", 50.0, 200.0)]));
    let f = s.font().unwrap();
    let aacute = f.glyph("Aacute").unwrap();
    assert_eq!(aacute.unicodes, vec![0xC1], "the glyph database's code point (autoUnicodes)");
    assert_eq!(aacute.info.note, "built");
    // The second rule sees the first one's glyph and its width.
    assert_eq!(placed(&s, "AacuteAacute"), parts(&[("Aacute", 0.0, 0.0), ("Aacute", 600.0, 0.0)]));
    assert_eq!(f.glyph("AacuteAacute").unwrap().layers[0].width, 1200.0);

    s.execute("edit.undo", &json!({})).unwrap();
    let f = s.font().unwrap();
    assert!(f.glyph("Aacute").is_none() && f.glyph("AacuteAacute").is_none(), "one undo step takes back the whole build");
}

#[test]
fn font_build_from_constructions_preview_returns_without_storing() {
    let mut s = session();
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, json!({"rules": "Aacute = A + acute@center,top", "preview": true}));
    assert_eq!(r["preview"], json!(true));
    assert_eq!(r["built"][0]["glyph"], json!("Aacute"));
    assert_eq!(r["built"][0]["layers"][0]["width"], json!(600.0));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "preview stores nothing");
}

#[test]
fn font_build_from_constructions_overwrite_off_and_question_mark_keep_existing_glyphs() {
    let mut s = session();
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, json!({"rules": "A = acute", "overwrite": false}));
    assert!(r["skipped"][0]["reason"].as_str().unwrap().contains("overwrite"));
    let r = run(&mut s, json!({"rules": "# OverwriteExistingGlyphs: False\nA = acute"}));
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "the file's option line");
    let r = run(&mut s, json!({"rules": "?A = acute"}));
    assert!(r["skipped"][0]["reason"].as_str().unwrap().contains('?'));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "the existing glyph is kept");

    // The parameter wins over the file; the glyph keeps its code point and other data.
    run(&mut s, json!({"rules": "# OverwriteExistingGlyphs: False\nA = acute", "overwrite": true}));
    assert_eq!(placed(&s, "A"), parts(&[("acute", 0.0, 0.0)]));
    assert_eq!(s.font().unwrap().glyph("A").unwrap().unicodes, vec![0x41]);
}

#[test]
fn font_build_from_constructions_skips_locked_glyphs() {
    let mut f = session().font().unwrap().clone();
    f.glyphs.get_mut("A").unwrap().info.locked = true;
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    let r = run(&mut s, json!({"rules": "A = acute\nAacute = A + acute"}));
    assert!(r["skipped"][0]["reason"].as_str().unwrap().contains("locked"));
    assert_eq!(r["built"][0]["glyph"], json!("Aacute"), "the other rules still run");
}

#[test]
fn font_build_from_constructions_decomposes_kerns_and_marks() {
    let mut s = session();
    let r = run(&mut s, json!({"rules": "*Aacute = A + acute@center,top", "markColor": [1, 0, 0, 1]}));
    assert_eq!(r["built"][0]["layers"][0]["paths"], json!(2));
    let g = s.font().unwrap().glyph("Aacute").unwrap().clone();
    assert!(g.layers[0].shapes.iter().all(|sh| matches!(sh, Shape::Path(_))), "decomposed to paths");
    assert_eq!(g.info.color, Some(ColorLabel::Rgba([1.0, 0.0, 0.0, 1.0])));

    let mut f = s.font().unwrap().clone();
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -50.0);
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    run(&mut s, json!({"rules": "A_V = A &\\ V\nAV = A & V"}));
    assert_eq!(placed(&s, "A_V"), parts(&[("A", 0.0, 0.0), ("V", 550.0, 0.0)]));
    assert_eq!(s.font().unwrap().glyph("A_V").unwrap().layers[0].width, 1150.0);
    assert_eq!(placed(&s, "AV"), parts(&[("A", 0.0, 0.0), ("V", 600.0, 0.0)]));
}

#[test]
fn font_build_from_constructions_reads_a_rules_file() {
    let dir = fontcraft_testkit::temp_dir("construction");
    let path = dir.join("accents.txt");
    std::fs::write(&path, "$mark = acute\nAacute = A + {mark}@center,top\n").unwrap();
    let mut s = session();
    run(&mut s, json!({"path": path.to_string_lossy()}));
    assert_eq!(placed(&s, "Aacute"), parts(&[("A", 0.0, 0.0), ("acute", 50.0, 200.0)]));
    refused(&mut s, json!({"path": dir.join("missing.txt").to_string_lossy()}), "cannot read");
    refused(&mut s, json!({"rules": "x = a", "path": path.to_string_lossy()}), "not both");
}

#[test]
fn font_build_from_constructions_refuses_or_lists_hostile_rules() {
    let mut s = session();
    refused(&mut s, json!({}), "required");
    refused(&mut s, json!({"rules": "a = b\n".repeat(1 << 20)}), "bytes");
    refused(&mut s, json!({"rules": "x = {nope}"}), "nope");
    refused(&mut s, json!({"rules": "$a = {b}\n$b = {a}\nx = {a}"}), "deep");
    refused(&mut s, json!({"rules": "x = A", "masters": ["nope"]}), "nope");
    refused(&mut s, json!({"rules": "x = A", "markColor": [2, 0, 0, 1]}), "markColor");

    // Lines that are not rules, and rules that cannot be built, are listed; the rest are built.
    let r = run(&mut s, json!({"rules": "good = A\nnot a rule\nA = A + acute\nbad = A + acute@`top\nalso = acute"}));
    let built: Vec<&str> = r["built"].as_array().unwrap().iter().map(|b| b["glyph"].as_str().unwrap()).collect();
    assert_eq!(built, vec!["good", "also"]);
    let lines: Vec<u64> = r["errors"].as_array().unwrap().iter().map(|e| e["line"].as_u64().unwrap()).collect();
    assert_eq!(lines, vec![2, 4, 3], "line errors first, then build errors in rule order");
    assert!(r["errors"][2]["message"].as_str().unwrap().contains("itself"));
}
