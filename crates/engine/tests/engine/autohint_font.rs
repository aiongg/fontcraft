//! PostScript autohinting of a compiled font file (M10.37): `autohint.ps.runFont`.
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
use skrifa::raw::types::{Fixed, Tag};
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

fn tables_except_cff(bytes: &[u8]) -> Vec<(Tag, Vec<u8>)> {
    let font = FontRef::new(bytes).unwrap();
    let mut out: Vec<(Tag, Vec<u8>)> = font
        .table_directory
        .table_records()
        .iter()
        .map(|r| r.tag())
        .filter(|t| *t != Tag::new(b"CFF "))
        .map(|t| (t, font.table_data(t).unwrap().as_bytes().to_vec()))
        .collect();
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
    assert_eq!(tables_except_cff(&out), tables_except_cff(&before), "every other table keeps its bytes");

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
        json!({"font": plain, "output": refused, "overlap": "all"}),
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
