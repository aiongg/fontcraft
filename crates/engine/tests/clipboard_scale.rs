//! The `Point To Unit Ratio` parameter on the SVG flavour of `edit.copy` (M7.11).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::{Number, Value};
use fontcraft_engine::model::{Param, Path, Shape};
use serde_json::json;

/// `A` holds a 200 × 100 rectangle at the origin.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("A")?;
        g.layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(0.0, 0.0, 200.0, 100.0)));
        Ok(())
    })
    .unwrap();
    s
}

fn set_ratio(s: &mut Session, master: bool, ratio: i64) {
    s.edit("ratio", |font| {
        let param = Param::new("Point To Unit Ratio", Value::Number(Number::int(ratio)));
        if master {
            Arc::make_mut(&mut font.masters)[0].params.0.push(param);
        } else {
            Arc::make_mut(&mut font.info).params.0.push(param);
        }
        Ok(())
    })
    .unwrap();
}

fn copied_svg(s: &mut Session) -> String {
    let v = s.execute("edit.copy", &json!({"glyph": "A"})).unwrap();
    v["svg"].as_str().unwrap().to_string()
}

#[test]
fn point_to_unit_ratio_scales_the_copied_svg_to_points() {
    let mut s = session();
    let plain = copied_svg(&mut s);
    assert!(plain.contains(r#"width="200" height="100" viewBox="0 -100 200 100""#), "{plain}");

    // Ten units to the point: the drawing is a tenth of the size, in points.
    set_ratio(&mut s, false, 10);
    let scaled = copied_svg(&mut s);
    assert!(scaled.contains(r#"width="20pt" height="10pt" viewBox="0 -10 20 10""#), "{scaled}");
    assert!(scaled.contains("<path"), "{scaled}");

    // The master's parameter wins over the font's.
    set_ratio(&mut s, true, 2);
    let master = copied_svg(&mut s);
    assert!(master.contains(r#"width="100pt" height="50pt""#), "{master}");

    // A ratio of 0 cannot be used: one unit per user unit, as without the parameter.
    let mut s = session();
    set_ratio(&mut s, false, 0);
    assert_eq!(copied_svg(&mut s), plain);

    // The JSON flavour keeps font units, so pasting it back is unchanged by the ratio.
    let mut s = session();
    set_ratio(&mut s, false, 10);
    let v = s.execute("edit.copy", &json!({"glyph": "A"})).unwrap();
    s.execute("edit.paste", &json!({"glyph": "B", "json": v["json"]})).unwrap();
    let pasted = s.font().unwrap().glyph("B").unwrap().layer("m01").unwrap().shapes.clone();
    let Some(Shape::Path(p)) = pasted.last() else { panic!("no path pasted: {pasted:?}") };
    assert!(p.nodes.iter().any(|n| n.x == 200.0 && n.y == 100.0), "{p:?}");
}
