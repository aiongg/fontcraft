//! `measure.sidebearingsAtHeight` (M5.11): the sidebearings on one horizontal line, which the
//! text-mode measurement line prints.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::Session;
use fontcraft_engine::model::{Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

/// `A`: a right triangle (100, 0), (500, 0), (100, 600) in a 600-unit advance, so the right
/// sidebearing grows with the height. `B` stays empty.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("A")?;
        let l = g.layer_mut("m01").unwrap();
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        l.shapes.push(Shape::Path(Path::new(true, vec![n(100.0, 0.0), n(500.0, 0.0), n(100.0, 600.0)])));
        Ok(())
    })
    .unwrap();
    s
}

fn measure(s: &mut Session, p: Value) -> Value {
    s.execute("measure.sidebearingsAtHeight", &p).unwrap_or_else(|e| panic!("{e}"))
}

/// An entry's left and right sidebearings, to a thousandth of a unit.
fn sides(entry: &Value) -> (f64, f64) {
    let round = |v: &Value| (v.as_f64().unwrap() * 1000.0).round() / 1000.0;
    (round(&entry["left"]), round(&entry["right"]))
}

#[test]
fn measure_sidebearings_at_height_follows_the_outline_at_that_height_only() {
    let mut s = session();
    let r = measure(&mut s, json!({"glyphs": ["A", "B"], "y": 300}));
    assert_eq!(r["master"], "m01");
    let a = &r["glyphs"][0];
    assert_eq!((&a["glyph"], &a["layer"], &a["width"]), (&json!("A"), &json!("m01"), &json!(600.0)));
    assert_eq!(sides(a), (100.0, 300.0));
    assert_eq!((&r["glyphs"][1]["left"], &r["glyphs"][1]["right"]), (&Value::Null, &Value::Null), "an empty layer has no crossings");
    let r = measure(&mut s, json!({"glyphs": ["A"], "y": 150, "master": "m01"}));
    assert_eq!(sides(&r["glyphs"][0]), (100.0, 200.0));
    let r = measure(&mut s, json!({"glyphs": ["A"], "y": 700}));
    assert_eq!(r["glyphs"][0]["left"], Value::Null, "the line passes over the outline");
    for bad in [
        json!({"glyphs": ["A"]}),
        json!({"glyphs": ["nope"], "y": 0}),
        json!({"glyphs": ["A"], "y": 1e9}),
        json!({"glyphs": ["A"], "y": 0, "master": "x"}),
    ] {
        assert!(s.execute("measure.sidebearingsAtHeight", &bad).is_err(), "{bad}");
    }
}
