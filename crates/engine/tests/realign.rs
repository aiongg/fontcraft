//! Automatic alignment after commands (M6.2): a command that changes a base glyph re-aligns the
//! composites built on it inside its own undo step, previews are aligned as they run, undo gives
//! back the very font, and a re-alignment that fails rolls the command back.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, Component, Font, Glyph, Layer, Path, Shape};
use fontcraft_testkit::{rect, snapshot};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// `A` (outline 20–580, width 600, `top` at (300, 700)), `acutecomb` (`_top` at (0, 500)) and
/// `Aacute`, set up without alignment: `Aacute` still has a stale width of 1.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("fixture", |f| {
        {
            let mut a = f.glyph_mut("A").unwrap();
            let l = &mut a.layers[0];
            l.shapes.push(Shape::Path(Path::rect(20.0, 0.0, 580.0, 700.0)));
            l.anchors.push(Anchor::new("top", 300.0, 700.0));
        }
        let mut acute = Glyph::new("acutecomb");
        let mut l = Layer::new("m01", 0.0);
        l.shapes.push(Shape::Path(Path::rect(-50.0, 520.0, 50.0, 620.0)));
        l.anchors.push(Anchor::new("_top", 0.0, 500.0));
        acute.layers.push(l);
        f.glyphs.push(acute);
        let mut aacute = Glyph::new("Aacute");
        let mut l = Layer::new("m01", 1.0);
        l.shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        l.shapes.push(Shape::Component(Component::new("acutecomb", 0.0, 0.0)));
        aacute.layers.push(l);
        f.glyphs.push(aacute);
        Ok(())
    })
    .unwrap();
    s
}

fn aacute(f: &Font) -> (f64, (f64, f64)) {
    let l = &f.glyph("Aacute").unwrap().layers[0];
    let Shape::Component(c) = &l.shapes[1] else { panic!("not a component") };
    let k = c.affine().as_coeffs();
    (l.width, (k[4], k[5]))
}

#[test]
fn a_base_edit_realigns_its_composites_in_the_same_undo_step() {
    let mut s = session();
    let before = s.doc().unwrap().font.clone();
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["A"], "value": 70}));
    assert_eq!(aacute(s.font().unwrap()), (650.0, (350.0, 200.0)), "the acute follows A's top, the width follows A's");
    let after = s.doc().unwrap().font.clone();
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "one undo gives back the very font, alignment included");
    run(&mut s, "edit.redo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &after));
}

#[test]
fn a_command_that_touches_no_composite_is_left_alone() {
    let mut s = session();
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    assert_eq!(aacute(s.font().unwrap()), (1.0, (0.0, 0.0)), "Aacute does not depend on B and keeps what the file stored");
}

#[test]
fn a_failed_realignment_rolls_the_command_back() {
    let mut s = session();
    s.edit("wide", |f| {
        let mut wide = Glyph::new("wide");
        wide.layers.push(Layer::new("m01", 600_000.0));
        f.glyphs.push(wide);
        let mut twice = Glyph::new("twice");
        let mut l = Layer::new("m01", 1.0);
        l.shapes = vec![Shape::Component(Component::new("wide", 0.0, 0.0)), Shape::Component(Component::new("wide", 0.0, 0.0))];
        twice.layers.push(l);
        f.glyphs.push(twice);
        Ok(())
    })
    .unwrap();
    let before = snapshot(&s);
    let font = s.doc().unwrap().font.clone();
    let e = s.execute("path.addPath", &json!({"glyph": "wide", "nodes": rect(0.0, 0.0, 100.0, 100.0)})).unwrap_err();
    assert!(e.to_string().contains("cannot be aligned"), "{e}");
    assert!(Arc::ptr_eq(&font, &s.doc().unwrap().font), "the font is the very same");
    assert_eq!(snapshot(&s), before);
}

#[test]
fn previews_are_aligned_and_the_commit_is_one_step() {
    let mut s = session();
    let start = s.doc().unwrap().font.clone();
    s.begin("Drag").unwrap();
    s.preview("spacing.setLeftSidebearing", &json!({"glyphs": ["A"], "value": 40})).unwrap();
    assert_eq!(aacute(s.font().unwrap()), (620.0, (320.0, 200.0)));
    s.preview("spacing.setLeftSidebearing", &json!({"glyphs": ["A"], "value": 70})).unwrap();
    assert_eq!(aacute(s.font().unwrap()), (650.0, (350.0, 200.0)));
    assert!(s.commit().unwrap());
    assert_eq!(aacute(s.font().unwrap()), (650.0, (350.0, 200.0)));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &start));
}

#[test]
fn the_font_setting_stops_realignment() {
    let mut s = session();
    s.edit("setting", |f| {
        Arc::make_mut(&mut f.info).settings.disables_automatic_alignment = Some(true);
        Ok(())
    })
    .unwrap();
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["A"], "value": 70}));
    assert_eq!(aacute(s.font().unwrap()), (1.0, (0.0, 0.0)));
}
