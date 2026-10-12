//! Transform Metrics (M5.3): width mode, on both sides, sidebearing mode, relative values, the
//! Font View batch, and the Default Layer Width parameter for new layers.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::{Number, Value as PValue};
use fontcraft_engine::model::{Anchor, Background, Guide, Layer, Param, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn layer(s: &Session, glyph: &str) -> Layer {
    s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap().clone()
}

fn xs(layer: &Layer) -> Vec<f64> {
    layer
        .shapes
        .iter()
        .flat_map(|shape| match shape {
            Shape::Path(p) => p.nodes.iter().map(|n| n.x).collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

/// `A` and `B`: a square from x 100 to 300 in a 600-unit advance (LSB 100, RSB 300), an anchor
/// at x 200, a local guide at x 150 and a background square from 100 to 200. `C` stays empty.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        for name in ["A", "B"] {
            let mut g = font.glyph_mut(name)?;
            let l = g.layer_mut("m01").unwrap();
            l.shapes.push(Shape::Path(Path::rect(100.0, 0.0, 300.0, 700.0)));
            l.anchors.push(Anchor::new("top", 200.0, 700.0));
            l.info.guides.push(Guide { x: 150.0, ..Default::default() });
            l.info.background = Some(Background { shapes: vec![Shape::Path(Path::rect(100.0, 0.0, 200.0, 100.0))], ..Default::default() });
        }
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn glyph_transform_metrics_width_mode_keeps_the_outline_where_it_is() {
    let mut s = session();
    let r = run(&mut s, "glyph.transformMetrics", json!({"glyph": "A", "width": 500}));
    assert_eq!(r["mode"], "width");
    let l = layer(&s, "A");
    assert_eq!(l.width, 500.0);
    assert_eq!(xs(&l), xs(&layer(&s, "B")));
    assert_eq!(l.anchors[0].x, 200.0);
}

#[test]
fn glyph_transform_metrics_on_both_sides_moves_everything_by_half_the_change() {
    let mut s = session();
    run(&mut s, "glyph.transformMetrics", json!({"glyph": "A", "width": 500, "onBothSides": true}));
    let l = layer(&s, "A");
    assert_eq!(l.width, 500.0);
    assert_eq!(xs(&l), xs(&layer(&s, "B")).iter().map(|x| x - 50.0).collect::<Vec<_>>());
    assert_eq!(l.anchors[0].x, 150.0);
    assert_eq!(l.info.guides[0].x, 100.0);
    let Some(Shape::Path(bg)) = l.info.background.as_ref().unwrap().shapes.first() else { panic!("no background path") };
    assert_eq!(bg.nodes.iter().map(|n| n.x).fold(f64::INFINITY, f64::min), 50.0);
}

#[test]
fn glyph_transform_metrics_sidebearing_mode_writes_only_the_sides_given() {
    let mut s = session();
    let r = run(&mut s, "glyph.transformMetrics", json!({"glyph": "A", "lsb": 40}));
    assert_eq!(r["mode"], "sidebearings");
    let l = layer(&s, "A");
    // LSB 100 → 40: the outline moves 60 left and the RSB stays 300.
    assert_eq!(xs(&l).iter().copied().fold(f64::INFINITY, f64::min), 40.0);
    assert_eq!(l.width, 540.0);

    run(&mut s, "glyph.transformMetrics", json!({"glyph": "B", "rsb": 50}));
    let l = layer(&s, "B");
    assert_eq!(xs(&l).iter().copied().fold(f64::INFINITY, f64::min), 100.0);
    assert_eq!(l.width, 350.0);
}

#[test]
fn glyph_transform_metrics_relative_adds_to_the_current_sidebearings() {
    let mut s = session();
    run(&mut s, "glyph.transformMetrics", json!({"glyph": "A", "lsb": -10, "rsb": 20, "relative": true}));
    let l = layer(&s, "A");
    assert_eq!(xs(&l).iter().copied().fold(f64::INFINITY, f64::min), 90.0);
    assert_eq!(l.width, 610.0);
}

#[test]
fn font_view_batch_transform_metrics_changes_every_selected_glyph_in_one_undo_step() {
    let mut s = session();
    let before = s.font().unwrap().clone();
    s.edit("Lock", |font| {
        font.glyph_mut("D")?.info.locked = true;
        Ok(())
    })
    .unwrap();
    let locked = s.font().unwrap().clone();
    let r = run(&mut s, "glyph.transformMetrics", json!({"glyphs": ["A", "B", "C", "D"], "lsb": 50, "rsb": 50}));
    assert_eq!(r["glyphs"], json!(["A", "B"]));
    let skipped: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|v| v["glyph"].as_str().unwrap()).collect();
    assert_eq!(skipped, ["C", "D"]);
    for name in ["A", "B"] {
        assert_eq!(layer(&s, name).width, 300.0);
    }
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap(), &locked);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap(), &before);
}

#[test]
fn glyph_transform_metrics_refusals_change_nothing() {
    let mut s = session();
    let before = s.font().unwrap().clone();
    for p in [
        json!({"glyph": "A"}),
        json!({"glyph": "A", "width": 500, "lsb": 10}),
        json!({"glyph": "A", "width": -1}),
        json!({"glyph": "A", "lsb": 40000}),
        json!({"glyph": "A", "rsb": -1000}),
        json!({"glyph": "C", "lsb": 10}),
    ] {
        let one = p.get("glyph").is_some() && p["glyph"] != "C";
        match s.execute("glyph.transformMetrics", &p) {
            Err(_) => {}
            Ok(r) => assert!(!one && r["glyphs"] == json!([]), "{p} was accepted: {r}"),
        }
        assert_eq!(s.font().unwrap(), &before, "{p}");
    }
}

#[test]
fn default_layer_width_sets_the_width_of_a_new_master_layer() {
    let mut s = session();
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        let params = &mut Arc::make_mut(&mut font.info).params;
        params.0.push(Param::new("Default Layer Width", PValue::Number(Number::int(520))));
        params.0.push(Param::new("Default Layer Width", PValue::String("cyrillic: 400".into())));
        font.glyph_mut("B")?.info.script = Some("cyrillic".into());
        Ok(())
    })
    .unwrap();
    let square = json!([{"x": 0, "y": 0}, {"x": 10, "y": 0}, {"x": 10, "y": 10}]);
    for (glyph, width) in [("A", 520.0), ("B", 400.0)] {
        run(&mut s, "path.addPath", json!({"glyph": glyph, "layer": "m02", "nodes": square}));
        let g = s.font().unwrap().glyph(glyph).unwrap().clone();
        assert_eq!(g.layer("m02").unwrap().width, width, "{glyph}");
        assert_eq!(g.layer("m01").unwrap().width, 600.0, "{glyph}");
    }
}
