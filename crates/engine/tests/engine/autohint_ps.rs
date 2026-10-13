//! PostScript autohinting (M10.15): `autohint.ps.run`, `hint.ps.autohintGlyph` and the
//! `autohint` option of `file.exportStatic` for CFF outlines.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_engine::Session;
use fontcraft_engine::model::plist::openstep::parse_value;
use fontcraft_engine::model::{AlignmentZone, Node, NodeKind, Param, Path, Shape};
use serde_json::{Value, json};
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::cs::CommandSink;
use skrifa::raw::types::{Fixed, Tag};
use skrifa::{FontRef, MetadataProvider as _};

/// `l`: a stem from x 100 to 180, 0 to 700 high, on master `m01`, which has zones at the
/// baseline and at 700 and standard stems of 80.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let stem = Shape::Path(Path::new(true, vec![n(100.0, 0.0), n(180.0, 0.0), n(180.0, 700.0), n(100.0, 700.0)]));
        font.glyph_mut("l")?.layer_mut("m01").unwrap().shapes = vec![stem];
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

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn hint_count(s: &Session, glyph: &str) -> usize {
    s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap().info.hints.len()
}

#[derive(Default)]
struct Stems(Vec<(i32, i32)>);

impl CommandSink for Stems {
    fn move_to(&mut self, _: Fixed, _: Fixed) {}
    fn line_to(&mut self, _: Fixed, _: Fixed) {}
    fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed) {}
    fn close(&mut self) {}
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.0.push((x.to_i32(), dx.to_i32()));
    }
}

/// The vertical stems of `l` in a CFF export of the session's font.
fn exported_vstems(s: &mut Session, autohint: bool) -> Vec<(i32, i32)> {
    let r = run(s, "file.exportStatic", json!({"outlines": "cff", "autohint": autohint, "timestamp": 0}));
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["fonts"][0]["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    let gid = font.charmap().map('l').unwrap();
    let table = font.table_data(Tag::new(b"CFF ")).unwrap();
    let cff = CffFontRef::new_cff(table.as_bytes(), 0, None).unwrap();
    let subfont = cff.subfont(0, &[]).unwrap();
    let mut sink = Stems::default();
    cff.evaluate_charstring(&subfont, gid, &[], &mut sink).unwrap();
    sink.0
}

#[test]
fn export_static_autohints_cff_glyphs_without_manual_hints() {
    let mut s = session();
    assert_eq!(exported_vstems(&mut s, true), vec![(100, 180)]);
    assert!(exported_vstems(&mut s, false).is_empty());
}

#[test]
fn hint_ps_autohint_glyph_stores_the_stems_in_one_undo_step() {
    let mut s = session();
    let before = s.font().unwrap().clone();
    for bad in [json!({}), json!({"glyphs": ["nope"]}), json!({"glyphs": ["l"], "layer": "nope"})] {
        assert!(s.execute("hint.ps.autohintGlyph", &bad).is_err(), "{bad}");
    }
    assert_eq!(*s.font().unwrap(), before, "a refusal changes nothing");

    let r = run(&mut s, "hint.ps.autohintGlyph", json!({"glyphs": ["l"]}));
    assert_eq!((&r["hinted"][0]["glyph"], &r["hinted"][0]["vertical"]), (&json!("l"), &json!(1)), "{r}");
    let stored = hint_count(&s, "l");
    assert!(stored >= 1);
    // Stored hints are manual hints: the export writes them with autohinting off.
    assert_eq!(exported_vstems(&mut s, false), vec![(100, 180)]);
    // Linked to the nodes: moving the stem moves the hint.
    s.edit("move", |font| {
        let mut g = font.glyph_mut("l")?;
        let Shape::Path(p) = &mut g.layer_mut("m01").unwrap().shapes[0] else { panic!() };
        p.nodes.iter_mut().for_each(|n| n.x += 10.0);
        Ok(())
    })
    .unwrap();
    assert_eq!(exported_vstems(&mut s, false), vec![(110, 190)]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(hint_count(&s, "l"), 0, "one undo step");
}

#[test]
fn autohint_ps_run_leaves_hinted_layers_unless_hint_all() {
    let mut s = session();
    let r = run(&mut s, "autohint.ps.run", json!({"glyphs": ["l"]}));
    assert_eq!(r["hinted"].as_array().unwrap().len(), 1, "{r}");
    let after = s.font().unwrap().clone();
    let r = run(&mut s, "autohint.ps.run", json!({"glyphs": ["l"]}));
    assert_eq!((r["hinted"].as_array().unwrap().len(), &r["skipped"][0]["glyph"]), (0, &json!("l")));
    assert_eq!(*s.font().unwrap(), after);
    let r = run(&mut s, "autohint.ps.run", json!({"glyphs": ["l"], "hintAll": true, "flex": false}));
    assert_eq!(r["hinted"].as_array().unwrap().len(), 1);
    for bad in [json!({"masters": ["nope"]}), json!({"maxSegments": 0}), json!({"flex": 2})] {
        assert!(s.execute("autohint.ps.run", &bad).is_err(), "{bad}");
    }
}
