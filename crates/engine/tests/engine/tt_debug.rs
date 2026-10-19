//! `hint.tt.debugStep` and `hint.tt.suggestDeltas` (M13.52): stepping a glyph program in
//! FontCraft's interpreter, and delta suggestions where edges pass through pixel centres.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeSet;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Path, Shape};
use fontcraft_geom::PathEl;
use fontcraft_render::hinted::{HintedRenderer, Intent};
use serde_json::{Value, json};

/// Glyph `I`: a bar 100 to 200 by 150 to 650 units, so at 10 PPM (1000 units per em) its
/// horizontal edges lie at 1.5 and 6.5 pixels, through the pixel centres.
fn session(snap: bool) -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut g = font.glyph_mut("I")?;
        g.layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(100.0, 150.0, 200.0, 650.0)));
        Ok(())
    })
    .unwrap();
    assert_eq!(s.font().unwrap().units_per_em, 1000);
    if snap {
        // Rounds the bottom-left point to the grid; the final IUP moves the rest of the contour
        // with it.
        s.execute("hint.tt.snapPoint", &json!({"glyph": "I", "nodes": [[0, 0]], "zone": "none"})).unwrap();
    }
    s
}

fn step(s: &mut Session, p: Value) -> Value {
    let mut p = p;
    p["glyph"] = json!("I");
    p["ppem"] = json!(10);
    s.execute("hint.tt.debugStep", &p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn points(r: &Value) -> Vec<(f64, f64)> {
    r["points"].as_array().unwrap().iter().map(|p| (p["x"].as_f64().unwrap(), p["y"].as_f64().unwrap())).collect()
}

/// The points sorted, as the compiler may start or turn a contour differently from the source.
fn sorted(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

#[test]
fn hint_tt_debug_step_runs_the_glyph_program_to_the_points_the_preview_draws() {
    let mut s = session(true);
    let start = step(&mut s, json!({"action": "start"}));
    assert_eq!((&start["step"], &start["finished"], &start["hinting"]), (&json!(0), &json!(false), &json!(true)));
    assert_eq!(start["next"]["program"], "glyph");
    let listing = start["program"].as_array().unwrap().clone();
    assert!(!listing.is_empty(), "the snap compiles to a glyph program");
    assert_eq!(start["next"]["pc"], listing[0]["pc"]);
    assert_eq!(sorted(points(&start)), vec![(1.0, 1.5), (1.0, 6.5), (2.0, 1.5), (2.0, 6.5)], "scaled, not yet moved");
    assert_eq!(start["phantom"].as_array().unwrap().len(), 4);

    // Run to the end: the bar moved up half a pixel onto the grid.
    let end = step(&mut s, json!({"action": "continue", "step": 0}));
    assert_eq!(end["finished"], true);
    assert!(end.get("next").is_none());
    assert!(end["step"].as_u64().unwrap() > 0);
    let got = points(&end);
    assert_eq!(sorted(got.clone()), vec![(1.0, 2.0), (1.0, 7.0), (2.0, 2.0), (2.0, 7.0)]);
    assert!(end["points"].as_array().unwrap().iter().any(|p| p["touchedY"] == true), "the snapped point is touched");

    // The same points as the hinted preview's outline, drawn by the reference interpreter.
    let compiled = fontcraft_engine::compile::compile_ttf(s.font().unwrap(), &Default::default()).unwrap();
    let gid = compiled.glyph_order.iter().position(|g| g == "I").unwrap() as u32;
    let renderer = HintedRenderer::new(Arc::from(compiled.bytes), Intent::Grayscale).unwrap();
    let outline = renderer.outline(gid, 10).unwrap();
    assert!(outline.hinted);
    let mut drawn = BTreeSet::new();
    for el in outline.path.elements() {
        if let PathEl::MoveTo(q) | PathEl::LineTo(q) = el {
            drawn.insert(((q.x * 64.0) as i64, (q.y * 64.0) as i64));
        }
    }
    let ours: BTreeSet<(i64, i64)> = got.iter().map(|p| ((p.0 * 64.0) as i64, (p.1 * 64.0) as i64)).collect();
    assert_eq!(ours, drawn);

    // Stepping one instruction at a time reaches the same end in as many steps.
    let total = end["step"].as_u64().unwrap();
    let mut at = 0u64;
    let mut last = start.clone();
    while last["finished"] != true {
        last = step(&mut s, json!({"action": "into", "step": at}));
        at = last["step"].as_u64().unwrap();
        assert!(at <= total, "stepping past the end");
    }
    assert_eq!(at, total);
    assert_eq!(points(&last), got);

    // 'over' from depth 0 stops only at depth 0; 'out' at depth 0 runs to the end.
    let mut at = 0u64;
    loop {
        let r = step(&mut s, json!({"action": "over", "step": at}));
        if r["finished"] == true {
            break;
        }
        assert_eq!(r["depth"], 0);
        assert!(r["step"].as_u64().unwrap() > at);
        at = r["step"].as_u64().unwrap();
    }
    assert_eq!(step(&mut s, json!({"action": "out", "step": 0}))["finished"], true);
    assert_eq!(step(&mut s, json!({"action": "stop"})), json!({"stopped": true}));
}

#[test]
fn hint_tt_debug_step_stops_at_breakpoints_in_the_glyph_program() {
    let mut s = session(true);
    let start = step(&mut s, json!({}));
    let listing = start["program"].as_array().unwrap().clone();
    let last_pc = listing.last().unwrap()["pc"].as_u64().unwrap();
    let r = step(&mut s, json!({"action": "continue", "step": 0, "breakpoints": [last_pc]}));
    assert_eq!((&r["finished"], &r["breakpoint"]), (&json!(false), &json!(last_pc)));
    assert_eq!((&r["next"]["program"], &r["next"]["pc"]), (&json!("glyph"), &json!(last_pc)));
    let at = r["step"].as_u64().unwrap();
    assert!(at > 0);
    // A breakpoint at the instruction the debugger stands on does not stop it again.
    let r = step(&mut s, json!({"action": "continue", "step": at, "breakpoints": [last_pc]}));
    assert_eq!(r["finished"], true);
    assert!(r["step"].as_u64().unwrap() > at, "the last instruction ran");
}

#[test]
fn hint_tt_debug_step_refuses_bad_requests() {
    let mut s = session(false);
    let bad = [
        json!({"glyph": "I"}),
        json!({"glyph": "nope", "ppem": 10}),
        json!({"glyph": "I", "ppem": 0}),
        json!({"glyph": "I", "ppem": 12.5}),
        json!({"glyph": "I", "ppem": 10, "action": "jump"}),
        json!({"glyph": "I", "ppem": 10, "breakpoints": [-1]}),
        json!({"glyph": "I", "ppem": 10, "intent": "mono"}),
    ];
    for p in bad {
        assert!(s.execute("hint.tt.debugStep", &p).is_err(), "{p}");
    }
    // A glyph without instructions runs nothing and ends at once.
    let r = step(&mut s, json!({"action": "continue"}));
    assert_eq!((&r["finished"], &r["step"], &r["program"]), (&json!(true), &json!(0), &json!([])));
    assert_eq!(sorted(points(&r)), vec![(1.0, 1.5), (1.0, 6.5), (2.0, 1.5), (2.0, 6.5)]);
}

fn suggestions(s: &mut Session, ppem: [u16; 2]) -> Vec<Value> {
    let r = s.execute("hint.tt.suggestDeltas", &json!({"glyphs": ["I"], "ppemRange": ppem})).unwrap();
    assert!(r.get("truncated").is_none());
    r["suggestions"].as_array().unwrap().clone()
}

#[test]
fn hint_tt_suggest_deltas_finds_edges_through_pixel_centres() {
    // Unhinted, the bar's horizontal edges cross pixel centres at 10 PPM (1.5 and 6.5 px), not at
    // 12 PPM (1.8 and 7.8 px).
    let mut s = session(false);
    let found = suggestions(&mut s, [10, 10]);
    assert_eq!(found.len(), 2, "{found:?}");
    for f in &found {
        assert_eq!((&f["glyph"], &f["ppem"], &f["direction"]), (&json!("I"), &json!(10), &json!("y")));
        assert_eq!((&f["offset"], &f["delta"]), (&json!(0.0), &json!(0.125)));
    }
    let pts: BTreeSet<u64> = found.iter().map(|f| f["point"].as_u64().unwrap()).collect();
    assert_eq!(pts.len(), 2, "one point on each edge");
    assert!(suggestions(&mut s, [12, 12]).is_empty());
    assert_eq!(suggestions(&mut s, [9, 12]).len(), 2, "only 10 PPM has a problem");

    // The snap moves both edges onto whole pixels: nothing left to suggest.
    let mut s = session(true);
    assert!(suggestions(&mut s, [10, 10]).is_empty());

    for bad in [
        json!({"ppemRange": [10]}),
        json!({"ppemRange": [12, 10]}),
        json!({"ppemRange": [1, 200]}),
        json!({"ppemRange": [10, 10], "within": 0}),
        json!({"ppemRange": [10, 10], "within": 0.6}),
        json!({"ppemRange": [10, 10], "glyphs": ["nope"]}),
    ] {
        assert!(s.execute("hint.tt.suggestDeltas", &bad).is_err(), "{bad}");
    }
}
