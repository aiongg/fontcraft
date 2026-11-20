//! PostScript autohinting of a compiled font file (M10.37, CFF2 in M10.37b): `autohint.ps.runFont`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::plist::openstep::parse_value;
use fontcraft_engine::model::{AlignmentZone, Node, NodeKind, Param, Path as ModelPath, Shape};
use serde_json::{Value, json};
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::cs::CommandSink;
use skrifa::raw::types::{F2Dot14, Fixed, Tag};
use skrifa::{FontRef, MetadataProvider as _};

/// `l`: a stem from x 100 to 180, 0 to 700 high; `o`: a box with a counter. The master has zones
/// at the baseline and at 700 and standard stems of 80.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| Shape::Path(ModelPath::new(true, vec![n(x0, y0), n(x1, y0), n(x1, y1), n(x0, y1)]));
        font.glyph_mut("l")?.layer_mut("m01").unwrap().shapes = vec![rect(100.0, 0.0, 180.0, 700.0)];
        font.glyph_mut("o")?.layer_mut("m01").unwrap().shapes = vec![rect(50.0, 0.0, 450.0, 500.0), rect(130.0, 80.0, 370.0, 420.0)];
        let master = &mut Arc::make_mut(&mut font.masters)[0];
        master.zones = vec![AlignmentZone { pos: 0.0, size: -10.0 }, AlignmentZone { pos: 700.0, size: 10.0 }];
        for key in ["postscriptStemSnapH", "postscriptStemSnapV"] {
            master.params.0.push(Param::new(key, parse_value("(80)").unwrap()));
        }
        Ok(())
    })
    .unwrap();
    s
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-autohint-font-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A CFF export of the session's font, autohinted or not.
fn export(s: &mut Session, autohint: bool) -> Vec<u8> {
    let r = s.execute("file.exportStatic", &json!({"outlines": "cff", "autohint": autohint, "timestamp": 0})).unwrap();
    base64::engine::general_purpose::STANDARD.decode(r["fonts"][0]["files"][0]["dataBase64"].as_str().unwrap()).unwrap()
}

/// Every stem and drawing command of a glyph's charstring, in order.
#[derive(Default)]
struct Record(Vec<String>);

impl CommandSink for Record {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("M {} {}", x.to_f64(), y.to_f64()));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(format!("L {} {}", x.to_f64(), y.to_f64()));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, x: Fixed, y: Fixed) {
        self.0.push(format!("C {} {} {} {} {} {}", a.to_f64(), b.to_f64(), c.to_f64(), d.to_f64(), x.to_f64(), y.to_f64()));
    }
    fn close(&mut self) {}
    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.0.push(format!("hstem {} {}", y.to_f64(), dy.to_f64()));
    }
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.0.push(format!("vstem {} {}", x.to_f64(), dx.to_f64()));
    }
    fn hint_mask(&mut self, mask: &[u8]) {
        self.0.push(format!("hintmask {mask:?}"));
    }
}

fn glyph_commands(bytes: &[u8], ch: char) -> Vec<String> {
    let font = FontRef::new(bytes).unwrap();
    let gid = font.charmap().map(ch).unwrap();
    let table = font.table_data(Tag::new(b"CFF ")).unwrap();
    let cff = CffFontRef::new_cff(table.as_bytes(), 0, None).unwrap();
    let subfont = cff.subfont(0, &[]).unwrap();
    let mut rec = Record::default();
    let width = cff.evaluate_charstring(&subfont, gid, &[], &mut rec).unwrap();
    rec.0.insert(0, format!("width {width:?}"));
    rec.0
}

fn outline(cmds: &[String]) -> Vec<String> {
    cmds.iter().filter(|c| !c.contains("stem") && !c.starts_with("hintmask")).cloned().collect()
}

fn tables_except(bytes: &[u8], skip: &[u8; 4]) -> Vec<(Tag, Vec<u8>)> {
    let font = FontRef::new(bytes).unwrap();
    let mut out: Vec<(Tag, Vec<u8>)> = font
        .table_directory
        .table_records()
        .iter()
        .map(|r| r.tag())
        .filter(|t| *t != Tag::new(skip))
        .map(|t| (t, font.table_data(t).unwrap().as_bytes().to_vec()))
        .collect();
    // head.checkSumAdjustment covers the whole file, so it follows the changed outline table.
    for (tag, data) in &mut out {
        if *tag == Tag::new(b"head") {
            data[8..12].fill(0);
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

fn run(s: &mut Session, p: Value) -> Value {
    s.execute("autohint.ps.runFont", &p).unwrap_or_else(|e| panic!("autohint.ps.runFont: {e}"))
}

#[test]
fn autohint_ps_run_font_hints_an_otf_as_the_export_does_and_keeps_the_rest() {
    let dir = scratch("run");
    let mut s = session();
    let plain = dir.join("Plain.otf");
    std::fs::write(&plain, export(&mut s, false)).unwrap();
    let before = std::fs::read(&plain).unwrap();

    let r = run(&mut s, json!({"font": plain}));
    let output = dir.join("Plain.autohinted.otf");
    assert_eq!((&r["output"], &r["written"]), (&json!(output), &json!(true)), "{r}");
    let hinted: Vec<&str> = r["hinted"].as_array().unwrap().iter().map(|h| h["glyph"].as_str().unwrap()).collect();
    assert!(hinted.contains(&"l") && hinted.contains(&"o"), "{r}");
    assert_eq!(std::fs::read(&plain).unwrap(), before, "the input is left as it was");

    // The same stems and masks as an export with autohinting on, on the same outline.
    let out = std::fs::read(&output).unwrap();
    let exported = export(&mut s, true);
    for ch in ['l', 'o'] {
        let ours = glyph_commands(&out, ch);
        assert_eq!(ours, glyph_commands(&exported, ch), "{ch}");
        assert_eq!(outline(&ours), outline(&glyph_commands(&before, ch)), "{ch} draws as it did");
    }
    assert!(glyph_commands(&out, 'l').contains(&"vstem 100 180".to_string()));
    assert_eq!(tables_except(&out, b"CFF "), tables_except(&before, b"CFF "), "every other table keeps its bytes");

    // Glyph selection, hintAll and dryRun.
    let again = dir.join("Again.otf");
    let r = run(&mut s, json!({"font": output, "output": again, "hintAll": false, "glyphs": ["l"]}));
    assert_eq!((r["hinted"].as_array().unwrap().len(), &r["skipped"][0]["glyph"]), (0, &json!("l")), "{r}");
    let r = run(&mut s, json!({"font": plain, "dryRun": true, "excludeGlyphs": ["o"]}));
    assert_eq!(r["written"], json!(false));
    assert!(r["hinted"].as_array().unwrap().iter().all(|h| h["glyph"] != "o"), "{r}");

    // Refusals write nothing.
    let refused = dir.join("Refused.otf");
    for bad in [
        json!({"font": plain, "output": refused, "glyphs": ["nope"]}),
        json!({"font": plain, "output": refused, "overlap": "some"}),
        json!({"font": plain, "output": refused, "maxSegments": 0}),
        json!({"font": dir.join("missing.otf"), "output": refused}),
    ] {
        assert!(s.execute("autohint.ps.runFont", &bad).is_err(), "{bad}");
    }
    assert!(!refused.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn autohint_ps_run_font_refuses_a_truetype_font() {
    let dir = scratch("ttf");
    let mut s = session();
    let r = s.execute("file.exportStatic", &json!({"outlines": "truetype", "autohint": false, "timestamp": 0})).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["fonts"][0]["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    let ttf: &Path = &dir.join("Plain.ttf");
    std::fs::write(ttf, bytes).unwrap();
    let e = s.execute("autohint.ps.runFont", &json!({"font": ttf})).unwrap_err().to_string();
    assert!(e.contains("CFF"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every stem command of a glyph's charstring, sorted.
fn stems(cmds: &[String]) -> Vec<String> {
    let mut out: Vec<String> = cmds.iter().filter(|c| c.contains("stem")).cloned().collect();
    out.sort();
    out
}

#[test]
fn autohint_ps_run_font_hints_an_overlapping_glyph_on_its_overlap_free_outline() {
    let dir = scratch("overlap");
    let mut s = session();
    // `l` as two overlapping stems, 100 to 180 and 140 to 220: one stem 100 to 220 once the
    // overlap is removed.
    s.edit("overlap", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let rect = |x0: f64, x1: f64| Shape::Path(ModelPath::new(true, vec![n(x0, 0.0), n(x1, 0.0), n(x1, 700.0), n(x0, 700.0)]));
        font.glyph_mut("l")?.layer_mut("m01").unwrap().shapes = vec![rect(100.0, 180.0), rect(140.0, 220.0)];
        Ok(())
    })
    .unwrap();
    let export_kept = |s: &mut Session| {
        let r = s.execute("file.exportStatic", &json!({"outlines": "cff", "autohint": false, "removeOverlap": false, "timestamp": 0})).unwrap();
        base64::engine::general_purpose::STANDARD.decode(r["fonts"][0]["files"][0]["dataBase64"].as_str().unwrap()).unwrap()
    };
    let plain = dir.join("Overlap.otf");
    std::fs::write(&plain, export_kept(&mut s)).unwrap();
    let before = std::fs::read(&plain).unwrap();
    assert_eq!(glyph_commands(&before, 'l').iter().filter(|c| c.starts_with("M ")).count(), 2, "the export keeps both contours");

    let forced = dir.join("Forced.otf");
    let r = run(&mut s, json!({"font": plain, "output": forced, "overlap": "all"}));
    assert!(r["hinted"].as_array().unwrap().iter().any(|h| h["glyph"] == "l"), "{r}");
    let ours = glyph_commands(&std::fs::read(&forced).unwrap(), 'l');
    assert_eq!(outline(&ours), outline(&glyph_commands(&before, 'l')), "only the hints change");

    // The stems the source autohinter stores with overlap handling on, written by an export.
    s.execute("autohint.ps.run", &json!({"glyphs": ["l"], "overlap": "all", "hintAll": true})).unwrap();
    let source = glyph_commands(&export_kept(&mut s), 'l');
    assert!(!stems(&source).is_empty(), "{source:?}");
    assert_eq!(stems(&ours), stems(&source));

    // Naming the glyph in overlapGlyphs hints it the same way.
    let listed = dir.join("Listed.otf");
    run(&mut s, json!({"font": plain, "output": listed, "overlapGlyphs": ["l"]}));
    assert_eq!(glyph_commands(&std::fs::read(&listed).unwrap(), 'l'), ours);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The two-master test font with `A` as a box with a counter: stems of 80 in Regular, and in
/// Bold vertical stems of 120 and horizontal ones of 100, with zones and standard stems to match.
fn variable_session(dir: &Path) -> Session {
    let path = dir.join("TestSans.glyphs");
    std::fs::write(&path, crate::export::TWO_MASTERS).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": path })).unwrap();
    s.edit("setup", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let outer = |x0: f64, y0: f64, x1: f64, y1: f64| Shape::Path(ModelPath::new(true, vec![n(x0, y0), n(x1, y0), n(x1, y1), n(x0, y1)]));
        let inner = |x0: f64, y0: f64, x1: f64, y1: f64| Shape::Path(ModelPath::new(true, vec![n(x0, y0), n(x0, y1), n(x1, y1), n(x1, y0)]));
        {
            let mut a = font.glyph_mut("A")?;
            a.layer_mut("m01").unwrap().shapes = vec![outer(50.0, 0.0, 450.0, 500.0), inner(130.0, 80.0, 370.0, 420.0)];
            a.layer_mut("m02").unwrap().shapes = vec![outer(50.0, 0.0, 550.0, 500.0), inner(170.0, 100.0, 430.0, 400.0)];
        }
        for (master, (h, v)) in Arc::make_mut(&mut font.masters).iter_mut().zip([("(80)", "(80)"), ("(100)", "(120)")]) {
            master.zones = vec![AlignmentZone { pos: 0.0, size: -10.0 }, AlignmentZone { pos: 500.0, size: 10.0 }];
            master.params.0.push(Param::new("postscriptStemSnapH", parse_value(h).unwrap()));
            master.params.0.push(Param::new("postscriptStemSnapV", parse_value(v).unwrap()));
        }
        Ok(())
    })
    .unwrap();
    s
}

/// A CFF2 variable export of the session's font, autohinted or not.
fn variable_export(s: &Session, autohint: bool) -> Vec<u8> {
    use fontcraft_engine::compile::var::{VarFlavour, VariableOptions, compile_variable};
    use fontcraft_engine::compile::{HintingOptions, Options};
    let options = Options { hinting: HintingOptions { autohint, strip: false }, ..Options::default() };
    let var = VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() };
    compile_variable(s.font().unwrap(), None, &options, &var).unwrap().compiled.bytes
}

/// Every stem, mask and drawing command of a glyph's CFF2 charstring at normalized `wght`.
fn cff2_commands(bytes: &[u8], ch: char, wght: f64) -> Vec<String> {
    let font = FontRef::new(bytes).unwrap();
    let gid = font.charmap().map(ch).unwrap();
    let table = font.table_data(Tag::new(b"CFF2")).unwrap();
    let cff = CffFontRef::new_cff2(table.as_bytes(), None).unwrap();
    let coords = [F2Dot14::from_f64(wght)];
    let subfont = cff.subfont(0, &coords).unwrap();
    let mut rec = Record::default();
    cff.evaluate_charstring(&subfont, gid, &coords, &mut rec).unwrap();
    rec.0
}

#[test]
fn autohint_ps_run_font_hints_a_cff2_font_as_the_variable_export_does() {
    let dir = scratch("cff2");
    let mut s = variable_session(&dir);
    let plain = dir.join("PlainVF.otf");
    std::fs::write(&plain, variable_export(&s, false)).unwrap();
    let before = std::fs::read(&plain).unwrap();
    assert!(!cff2_commands(&before, 'A', 0.0).iter().any(|c| c.contains("stem")), "the input is unhinted");

    let r = run(&mut s, json!({"font": plain}));
    let hinted: Vec<&str> = r["hinted"].as_array().unwrap().iter().map(|h| h["glyph"].as_str().unwrap()).collect();
    assert!(hinted.contains(&"A"), "{r}");
    let out = std::fs::read(dir.join("PlainVF.autohinted.otf")).unwrap();

    // At each master, the stems and masks of the variable export with autohinting on.
    let exported = variable_export(&s, true);
    for wght in [0.0, 1.0] {
        let ours = cff2_commands(&out, 'A', wght);
        assert!(ours.iter().any(|c| c.starts_with("vstem")) && ours.iter().any(|c| c.starts_with("hstem")), "at {wght}: {ours:?}");
        assert_eq!(ours, cff2_commands(&exported, 'A', wght), "at {wght}");
    }
    assert!(cff2_commands(&out, 'A', 1.0).contains(&"vstem 50 170".to_string()), "{:?}", cff2_commands(&out, 'A', 1.0));
    // The outline draws as it did, at the masters and between them; every other table is kept.
    for wght in [0.0, 0.5, 1.0] {
        assert_eq!(outline(&cff2_commands(&out, 'A', wght)), outline(&cff2_commands(&before, 'A', wght)), "at {wght}");
    }
    assert_eq!(tables_except(&out, b"CFF2"), tables_except(&before, b"CFF2"), "every other table keeps its bytes");
    let _ = std::fs::remove_dir_all(&dir);
}
