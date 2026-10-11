//! Metrics export and import (`file.exportMetrics`, `file.importMetrics`): kerning-text restores
//! a master exactly, AFM its expanded pairs and widths; the four import modes; Selected Glyphs;
//! font sources and open documents; hostile files. The AFM measure of `docs/plans/M5.md` § 5 is
//! the ignored test at the end.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as Plist;
use fontcraft_engine::model::{Component, Font, Layer, Param, Path, Shape, SourceFormat};
use fontcraft_spacing::Field;
use fontcraft_spacing::afm;
use fontcraft_spacing::kerning::Direction;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changed the font");
    e
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-metrics-io-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Masters `m01` Regular and `m02` Bold. Outlines on both masters: H 60–540, n 50–550, o and
/// oacute 40–460 (width 500), T and Tcaron 20–580, A 10–590, V 15–585; Aacute is a component of
/// A. Groups: right T (T, Tcaron), left o (o, oacute), right A (A, Aacute), left V. Keys: o's
/// left `=n` (glyph-level), n's right `==H` (local to m01). Pairs of m01 in stored order:
/// @T–@o −60, T–o −20, A–V −80.5, @A–@V −50; m02: T–o −10.
fn font() -> Font {
    let mut f = Font::new("Metrics Test");
    for name in ["Tcaron", "oacute", "Aacute"] {
        f.add_glyph(name, vec![], 600.0).unwrap();
    }
    let mut bold = f.masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    Arc::make_mut(&mut f.masters).push(bold);
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        let width = g.layers[0].width;
        g.layers.push(Layer::new("m02", width));
    }
    for (name, x0, x1, width) in [
        ("H", 60.0, 540.0, 600.0),
        ("n", 50.0, 550.0, 600.0),
        ("o", 40.0, 460.0, 500.0),
        ("oacute", 40.0, 460.0, 500.0),
        ("T", 20.0, 580.0, 600.0),
        ("Tcaron", 20.0, 580.0, 600.0),
        ("A", 10.0, 590.0, 600.0),
        ("V", 15.0, 585.0, 600.0),
    ] {
        let mut g = f.glyph_mut(name).unwrap();
        for l in &mut g.layers {
            l.shapes = vec![Shape::Path(Path::rect(x0, 0.0, x1, 700.0))];
            l.width = width;
        }
    }
    {
        let mut g = f.glyph_mut("Aacute").unwrap();
        for l in &mut g.layers {
            l.shapes = vec![Shape::Component(Component::new("A", 0.0, 0.0))];
        }
    }
    for (glyph, right, left) in [
        ("T", Some("T"), None),
        ("Tcaron", Some("T"), None),
        ("o", None, Some("o")),
        ("oacute", None, Some("o")),
        ("A", Some("A"), None),
        ("Aacute", Some("A"), None),
        ("V", None, Some("V")),
    ] {
        let mut g = f.glyph_mut(glyph).unwrap();
        g.kern_groups.right = right.map(str::to_string);
        g.kern_groups.left = left.map(str::to_string);
    }
    f.glyph_mut("o").unwrap().info.metrics_keys.left = Some("=n".into());
    f.glyph_mut("n").unwrap().layers[0].info.metrics_keys.right = Some("=H".into());
    let k = &mut Arc::make_mut(&mut f.kerning).ltr;
    k.set("m01", "@MMK_L_T", "@MMK_R_o", -60.0);
    k.set("m01", "T", "o", -20.0);
    k.set("m01", "A", "V", -80.5);
    k.set("m01", "@MMK_L_A", "@MMK_R_V", -50.0);
    k.set("m02", "T", "o", -10.0);
    f
}

fn session() -> Session {
    let mut s = Session::new();
    s.add_document(font(), None).unwrap();
    s
}

/// `font` with master m01's spacing and kerning disturbed: other pairs, no groups, o moved and
/// wider, n wider, the keys gone and one added to H.
fn disturbed(font: &Font) -> Font {
    let mut f = font.clone();
    let k = &mut Arc::make_mut(&mut f.kerning).ltr;
    for m in &mut k.0 {
        if m.master == "m01" {
            m.firsts.clear();
        }
    }
    k.set("m01", "H", "n", 5.0);
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        g.kern_groups = Default::default();
    }
    {
        let mut o = f.glyph_mut("o").unwrap();
        o.info.metrics_keys.left = None;
        let l = &mut o.layers[0];
        let Shape::Path(p) = &mut l.shapes[0] else { panic!() };
        p.nodes.iter_mut().for_each(|n| n.x += 30.0);
        l.width += 17.0;
    }
    {
        let mut n = f.glyph_mut("n").unwrap();
        n.layers[0].info.metrics_keys.right = None;
        n.layers[0].width = 640.0;
    }
    f.glyph_mut("H").unwrap().info.metrics_keys.left = Some("=n".into());
    f
}

fn metric(font: &Font, glyph: &str, master: &str, field: Field) -> f64 {
    let l = font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == master).unwrap();
    fontcraft_spacing::metric(font, l, field).unwrap().unwrap()
}

/// Expanded pairs of a master, rounded half up as AFM writes them.
fn expanded(font: &Font, master: &str) -> Vec<(String, String, f64)> {
    afm::covered_pairs(font, Direction::Ltr, master)
        .unwrap()
        .into_iter()
        .map(|p| (p.left, p.right, fontcraft_spacing::round_half_up(p.value)))
        .collect()
}

#[test]
fn kerning_text_export_then_import_restores_the_master_exactly_as_one_undo_step() {
    let original = font();
    let mut s = session();
    let dir = temp("restore");
    let out = run(&mut s, "file.exportMetrics", json!({"format": "metrics", "destination": dir, "fileName": ""}));
    let path = PathBuf::from(out["path"].as_str().unwrap());
    assert_eq!(path, dir.join("Metrics Test-Regular.tsv"), "the default name is Family-Master");
    assert_eq!((out["master"].clone(), out["direction"].clone(), out["pairs"].clone()), (json!("m01"), json!("ltr"), json!(4)));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("kerning-text\t1\nmaster\tRegular\ndirection\tltr\n"), "{text}");
    assert!(text.contains("\nmetrics\to\t40\t40\t500\t=n\t\t\n"), "{text}");
    assert!(text.contains("\nmetrics\tn\t50\t50\t600\t\t==H\t\n"), "{text}");
    assert!(text.contains("\nA\tV\t-80.5\n"), "file values keep their fractions: {text}");
    let returned = run(&mut s, "file.exportMetrics", json!({"format": "metrics"}));
    assert_eq!(returned["text"], json!(text), "without a path the text is returned");

    s.add_document(disturbed(&original), None).unwrap();
    let before = s.doc().unwrap().font.clone();
    let r = run(&mut s, "file.importMetrics", json!({"file": path, "importMetrics": "lsbAndWidth"}));
    assert_eq!(r["source"], "kerning-text");
    assert_eq!(r["kerning"]["pairs"], 4);
    assert_eq!(r["metrics"]["badKeys"], json!([]));
    let after = s.font().unwrap();
    assert_eq!(after.kerning, original.kerning, "the pairs, their order and spelling, in place");
    for g in original.glyphs.iter() {
        assert_eq!(after.glyph(&g.name), Some(g), "glyph {}", g.name);
    }

    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "the import is one undo step");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn afm_export_expands_groups_and_imports_back_as_glyph_pairs() {
    let original = font();
    let mut s = session();
    let text = run(&mut s, "file.exportMetrics", json!({"format": "afm"}))["text"].as_str().unwrap().to_string();
    assert!(text.starts_with("StartFontMetrics 4.1\nFontName MetricsTest-Regular\n"), "{text}");
    assert!(text.contains("\nC 65 ; WX 600 ; N A ; B 10 0 590 700 ;\n"), "{text}");
    assert!(text.contains("\nC -1 ; WX 600 ; N Aacute ; B 10 0 590 700 ;\n"), "{text}");
    assert!(text.contains("\nC 32 ; WX 250 ; N space ; B 0 0 0 0 ;\n"), "{text}");
    for kpx in ["KPX T o -20", "KPX T oacute -60", "KPX Tcaron o -60", "KPX Tcaron oacute -60", "KPX A V -80", "KPX Aacute V -50"] {
        assert!(text.contains(&format!("\n{kpx}\n")), "{kpx}: {text}");
    }
    assert!(!text.contains('@'), "AFM has no groups");

    s.add_document(disturbed(&original), None).unwrap();
    let r = run(&mut s, "file.importMetrics", json!({"text": text, "importMetrics": true}));
    assert_eq!(r["source"], "afm");
    assert_eq!(r["groups"]["glyphs"], 0, "AFM holds no groups");
    let after = s.font().unwrap();
    assert_eq!(expanded(after, "m01"), expanded(&original, "m01"));
    for g in original.glyphs.iter() {
        let a = after.glyph(&g.name).unwrap();
        assert_eq!(a.layers[0].width, g.layers[0].width, "{}", g.name);
        assert_eq!(a.layers[0].shapes, g.layers[0].shapes, "{}", g.name);
    }
    assert_eq!(after.glyph("H").unwrap().info.metrics_keys.left.as_deref(), Some("=n"), "AFM leaves the keys alone");
}

#[test]
fn each_import_mode_takes_the_metrics_it_names() {
    let line = "metrics\tn\t30\t70\t700\t\t\t\n";
    for (mode, lsb, rsb, width) in [
        (json!("lsbAndWidth"), 30.0, 170.0, 700.0),
        (json!(true), 30.0, 170.0, 700.0),
        (json!("rsbAndWidth"), 130.0, 70.0, 700.0),
        (json!("bothSidebearings"), 30.0, 70.0, 600.0),
        (json!("widthCentered"), 100.0, 100.0, 700.0),
    ] {
        let mut s = session();
        let r = run(&mut s, "file.importMetrics", json!({"text": line, "importKerning": false, "importKerningGroups": false, "importMetrics": mode}));
        assert_eq!(r["metrics"]["glyphs"], 1, "{mode}");
        assert!(r.get("kerning").is_none() && r.get("groups").is_none(), "{r}");
        let f = s.font().unwrap();
        let got = (metric(f, "n", "m01", Field::Left), metric(f, "n", "m01", Field::Right), f.glyph("n").unwrap().layers[0].width);
        assert_eq!(got, (lsb, rsb, width), "{mode}");
        assert_eq!(metric(f, "n", "m02", Field::Left), 50.0, "{mode}: the other master stays");
        assert_eq!(f.glyph("n").unwrap().layers[0].info.metrics_keys.right, None, "{mode}: an empty key field removes the key");
        assert!(f.kerning == font().kerning, "{mode}");
    }
    // AFM gives the left sidebearing from the bounding box and keeps the keys.
    let mut s = session();
    let text = "StartFontMetrics 2.0\nC 110 ; WX 700 ; N n ; B 30 0 530 700 ;\nC -1 ; WX 9 ; N missing ; B 0 0 0 0 ;\n";
    let r = run(&mut s, "file.importMetrics", json!({"text": text, "master": "m02", "importKerning": false, "importMetrics": "bothSidebearings"}));
    assert_eq!(r["metrics"]["unknownGlyphs"], json!(["missing"]));
    let f = s.font().unwrap();
    assert_eq!((metric(f, "n", "m02", Field::Left), metric(f, "n", "m02", Field::Right)), (30.0, 170.0));
    assert_eq!(f.glyph("n").unwrap().layers[0].info.metrics_keys.right.as_deref(), Some("=H"));

    // A file of pairs alone replaces the pairs and leaves the groups.
    let mut s = session();
    let r = run(&mut s, "file.importMetrics", json!({"text": "T\tV\t-5\n"}));
    assert_eq!((r["groups"]["glyphs"].clone(), r["kerning"]["pairs"].clone()), (json!(0), json!(1)));
    assert_eq!(s.font().unwrap().glyph("T").unwrap().kern_groups.right.as_deref(), Some("T"));
}

#[test]
fn selected_glyphs_limit_metrics_and_groups_and_an_empty_selection_means_all() {
    let original = font();
    let text = run(&mut session(), "file.exportMetrics", json!({"format": "metrics"}))["text"].as_str().unwrap().to_string();
    let mut s = Session::new();
    s.add_document(disturbed(&original), None).unwrap();
    let r = run(&mut s, "file.importMetrics", json!({"text": text, "importKerning": false, "importMetrics": "lsbAndWidth", "glyphs": ["o"]}));
    assert_eq!((r["metrics"]["glyphs"].clone(), r["groups"]["glyphs"].clone()), (json!(1), json!(1)));
    let f = s.font().unwrap();
    assert_eq!(f.glyph("o"), original.glyph("o"));
    assert_eq!(f.glyph("n").unwrap().layers[0].width, 640.0, "n is not selected");
    assert_eq!(f.glyph("T").unwrap().kern_groups.right, None, "T is not selected");

    run(&mut s, "file.importMetrics", json!({"text": text, "importKerning": false, "importMetrics": "lsbAndWidth", "glyphs": []}));
    let f = s.font().unwrap();
    for g in original.glyphs.iter() {
        assert_eq!(f.glyph(&g.name), Some(g), "{}", g.name);
    }
}

#[test]
fn fonts_and_open_documents_give_kerning_and_groups_but_a_glyphs_file_no_kerning_values() {
    let mut source = font();
    for i in 0..source.glyphs.len() {
        source.glyphs.get_mut_at(i).unwrap().kern_groups = Default::default();
    }
    source.glyph_mut("H").unwrap().kern_groups.right = Some("H".into());
    let k = &mut Arc::make_mut(&mut source.kerning).ltr;
    k.0.retain(|m| m.master != "m01");
    k.set("m01", "@MMK_L_H", "n", -33.0);
    let mut s = session();
    s.add_document(source.clone(), None).unwrap();
    run(&mut s, "document.activate", json!({"index": 0}));

    let r = run(&mut s, "file.importMetrics", json!({"sourceDocument": 1}));
    assert_eq!(r["source"], "font");
    let f = s.font().unwrap();
    assert_eq!(f.kerning.ltr.0.iter().find(|m| m.master == "m01").unwrap().firsts.len(), 1);
    assert_eq!(f.glyph("H").unwrap().kern_groups.right.as_deref(), Some("H"));
    assert_eq!(f.glyph("T").unwrap().kern_groups.right, None, "the groups are copied, removals included");
    let r = run(&mut s, "file.importMetrics", json!({"sourceDocument": 1, "sourceMaster": "Bold", "importKerningGroups": false}));
    assert_eq!(r["kerning"]["pairs"], 1);
    let pair = run(&mut s, "kerning.getPair", json!({"left": "T", "right": "o"}));
    assert_eq!(pair["value"], -10.0, "Bold's pair is now Regular's: {pair}");
    let e = refused(&mut s, "file.importMetrics", json!({"sourceDocument": 1, "importMetrics": "lsbAndWidth"}));
    assert!(e.contains("kerning-text and AFM"), "{e}");

    // A .glyphs file: groups only, and kerning values are refused.
    let dir = temp("glyphs");
    let file = dir.join("Source.glyphs");
    run(&mut s, "document.activate", json!({"index": 1}));
    run(&mut s, "file.saveAs", json!({"path": file}));
    run(&mut s, "document.activate", json!({"index": 0}));
    let mut fresh = Session::new();
    fresh.add_document(font(), None).unwrap();
    let r = run(&mut fresh, "file.importMetrics", json!({"file": file}));
    assert!(r.get("kerning").is_none(), "{r}");
    assert_eq!(fresh.font().unwrap().glyph("H").unwrap().kern_groups.right.as_deref(), Some("H"));
    assert_eq!(fresh.font().unwrap().kerning, font().kerning);
    let e = refused(&mut fresh, "file.importMetrics", json!({"file": file, "importKerning": true}));
    assert!(e.contains(".glyphs file"), "{e}");
    let mut glyphs = source;
    glyphs.source.format = SourceFormat::Glyphs;
    fresh.add_document(glyphs, None).unwrap();
    run(&mut fresh, "document.activate", json!({"index": 0}));
    refused(&mut fresh, "file.importMetrics", json!({"sourceDocument": 1, "importKerning": true}));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_files_and_bad_requests_fail_with_a_message_and_change_nothing() {
    let mut s = session();
    let huge = "a\tb\t1\n".repeat(fontcraft_spacing::kerning_text::MAX_TEXT_BYTES / 6 + 1);
    assert!(refused(&mut s, "file.importMetrics", json!({"text": huge})).contains("bytes long"));
    assert!(refused(&mut s, "file.importMetrics", json!({"text": "x".repeat(5000)})).contains("line 1"));
    let many = format!("StartFontMetrics 2.0\n{}", "KPX a b 1\n".repeat(fontcraft_spacing::afm::MAX_PAIRS + 1));
    assert!(refused(&mut s, "file.importMetrics", json!({"text": many})).contains("pairs"));
    let long_line = format!("StartFontMetrics 2.0\nC 1 ; WX 1 ; N {} ;\n", "a".repeat(5000));
    assert!(refused(&mut s, "file.importMetrics", json!({"text": long_line})).contains("line 2"));
    for (params, says) in [
        (json!({"text": "StartFontMetrics\nC 1 ; WX 1e999 ; N a ;"}), "too large"),
        (json!({"text": "A\tV\tabc"}), "not a number"),
        (json!({"text": "A\tV\t-50", "importKerning": false, "importKerningGroups": false}), "nothing to import"),
        (json!({"text": "A\tV\t-50", "file": "x"}), "only one"),
        (json!({}), "give `file`"),
        (json!({"sourceDocument": 7}), "no open document"),
        (json!({"sourceDocument": 0}), "go into"),
        (json!({"text": "A\tV\t-50", "importMetrics": "sideways"}), "importMetrics"),
        (json!({"text": "A\tV\t-50", "sourceMaster": "m01"}), "font source"),
        (json!({"text": "A\tV\t-50", "master": "m09"}), "m09"),
        (json!({"text": "A\tV\t-50", "glyphs": ["nothing"]}), "nothing"),
        (json!({"file": "/nonexistent/fontcraft/metrics.tsv"}), "metrics.tsv"),
    ] {
        let e = refused(&mut s, "file.importMetrics", params.clone());
        assert!(e.contains(says), "{params}: {e}");
    }
    for (params, says) in [
        (json!({"format": "pdf"}), "metrics or afm"),
        (json!({"format": "afm", "direction": "rtl"}), "left-to-right"),
        (json!({"format": "metrics", "destination": "x", "fileName": "a/b"}), "fileName"),
        (json!({"format": "metrics", "path": "x", "destination": "y"}), "not both"),
        (json!({"format": "metrics", "fileName": "x"}), "destination"),
        (json!({"format": "metrics", "master": "m09"}), "m09"),
    ] {
        let e = s.execute("file.exportMetrics", &params).expect_err("refused").to_string();
        assert!(e.contains(says), "{params}: {e}");
    }

    // A master that takes its metrics from another is refused.
    let mut f = font();
    Arc::make_mut(&mut f.masters)[1].params.0.push(Param::new("Link Metrics With First Master", Plist::Bool(true)));
    s.add_document(f, None).unwrap();
    let e = refused(&mut s, "file.importMetrics", json!({"text": "A\tV\t-50", "master": "m02"}));
    assert!(e.contains("m01"), "{e}");
    let linked = run(&mut s, "file.exportMetrics", json!({"format": "metrics", "master": "m02"}))["text"].as_str().unwrap().to_string();
    assert!(linked.contains("master\tBold\n") && linked.contains("\nA\tV\t-80.5\n"), "a linked master exports what it shows: {linked}");
}

// ---------- the § 5 AFM measure ----------

fn python() -> PathBuf {
    if let Some(p) = std::env::var_os("FONTMAKE_PYTHON") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let uv = home.join(".local/share/uv/tools/fontmake/bin/python");
    if uv.exists() { uv } else { PathBuf::from("python3") }
}

/// Reads `afm` with fontTools' afmLib, prints its widths and pairs as JSON and writes the file
/// again with afmLib to `out`. afmLib's name pattern lacks `-`, which PostScript glyph names may
/// hold (Inter has `Che-cy`), so the script widens it to take `-` too.
const AFMLIB_SCRIPT: &str = r#"
import json, re, sys
from fontTools import afmLib
for name in ("charRE", "kernRE", "compositeRE", "componentRE"):
    rx = getattr(afmLib, name)
    setattr(afmLib, name, re.compile(rx.pattern.replace("[.A-Za-z0-9_]", "[-.A-Za-z0-9_]")))
f = afmLib.AFM(sys.argv[1])
widths = {name: f[name][1] for name in f.chars()}
pairs = [[l, r, f[(l, r)]] for (l, r) in f.kernpairs()]
f.write(sys.argv[2])
print(json.dumps({"widths": widths, "pairs": pairs}))
"#;

type Pairs = std::collections::BTreeMap<(String, String), f64>;

fn afmlib(afm_path: &FsPath, out: &FsPath, dir: &FsPath) -> (std::collections::BTreeMap<String, f64>, Pairs) {
    let script = dir.join("afmlib_check.py");
    std::fs::write(&script, AFMLIB_SCRIPT).unwrap();
    let output =
        std::process::Command::new(python()).arg("-I").arg(&script).arg(afm_path).arg(out).output().expect("the fontmake tool's Python runs");
    assert!(output.status.success(), "afmLib failed: {}", String::from_utf8_lossy(&output.stderr));
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    let widths = v["widths"].as_object().unwrap().iter().map(|(k, w)| (k.clone(), w.as_f64().unwrap())).collect();
    let pairs = v["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| ((p[0].as_str().unwrap().to_string(), p[1].as_str().unwrap().to_string()), p[2].as_f64().unwrap()))
        .collect();
    (widths, pairs)
}

/// `docs/plans/M5.md` § 5, AFM: an AFM exported from Inter Roman's Regular master reads back
/// with afmLib with FontCraft's widths and every expanded pair; the AFM afmLib writes from it
/// imports into the master with the same values.
/// `cargo test --release -p fontcraft-engine --test metrics_io -- --ignored --nocapture`
#[test]
#[ignore = "needs corpus/ (cargo xtask corpus --fetch) and fontTools (the fontmake tool's Python)"]
fn afm_from_inter_regular_reads_back_with_afmlib_both_ways() {
    let path = FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    let (font, _) = fontcraft_engine::io::open(&path, None).unwrap();
    let master = font.masters.iter().find(|m| m.name == "Regular").expect("a Regular master").id.clone();
    let dir = temp("inter");
    let ours = dir.join("Inter-Regular.afm");
    let back = dir.join("Inter-Regular-afmlib.afm");
    let mut s = Session::new();
    s.add_document(font.clone(), None).unwrap();
    let out = run(&mut s, "file.exportMetrics", json!({"master": master, "format": "afm", "path": ours}));
    eprintln!("exported {} glyphs and {} pairs, {} bytes", out["glyphs"], out["pairs"], out["bytes"]);

    let widths: std::collections::BTreeMap<String, f64> =
        font.glyphs.iter().filter_map(|g| g.layer(&master).map(|l| (g.name.clone(), fontcraft_spacing::round_half_up(l.width)))).collect();
    let pairs: Pairs = expanded(&font, &master).into_iter().map(|(l, r, v)| ((l, r), v)).collect();
    let (lib_widths, lib_pairs) = afmlib(&ours, &back, &dir);
    assert_eq!(lib_widths.len(), widths.len(), "afmLib reads every glyph");
    let width_diffs: Vec<_> = widths.iter().filter(|(n, w)| lib_widths.get(*n) != Some(w)).take(20).collect();
    assert!(width_diffs.is_empty(), "widths afmLib reads differently: {width_diffs:?}");
    assert_eq!(lib_pairs.len(), pairs.len(), "afmLib reads every expanded pair");
    let pair_diffs: Vec<_> = pairs.iter().filter(|(k, v)| lib_pairs.get(*k) != Some(v)).take(20).collect();
    assert!(pair_diffs.is_empty(), "pairs afmLib reads differently: {pair_diffs:?}");

    // The other way: afmLib's file imports into the master, its kerning emptied first.
    let mut emptied = font.clone();
    Arc::make_mut(&mut emptied.kerning).ltr.0.retain(|m| m.master != master);
    let mut s = Session::new();
    s.add_document(emptied, None).unwrap();
    let r = run(&mut s, "file.importMetrics", json!({"file": back, "master": master, "importKerningGroups": false, "importMetrics": "lsbAndWidth"}));
    eprintln!("imported {} pairs and the metrics of {} glyphs", r["kerning"]["pairs"], r["metrics"]["glyphs"]);
    let after = s.font().unwrap();
    let imported: Pairs = expanded(after, &master).into_iter().map(|(l, r, v)| ((l, r), v)).collect();
    assert_eq!(imported, lib_pairs, "the pairs afmLib wrote");
    for (name, w) in &lib_widths {
        assert_eq!(after.glyph(name).and_then(|g| g.layer(&master)).map(|l| l.width), Some(*w), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
