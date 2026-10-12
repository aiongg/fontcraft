//! Clipboard and SVG outlines (M3.8): `edit.copy`, `edit.cut`, `edit.paste` and
//! `file.importOutlines`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, Component, Guide, Node, NodeKind, Path, Shape};
use fontcraft_testkit::{snapshot, temp_dir};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// `A` holds a closed square (shape 0), a component of `B` (1) and an open line path (2), the
/// anchor `top` and a local guide; its master has one global guide. `C` and `D` are empty.
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        if let Some(m) = Arc::make_mut(&mut font.masters).first_mut() {
            m.guides.push(Guide { y: 500.0, ..Guide::default() });
        }
        let mut g = font.glyph_mut("A")?;
        let l = g.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        l.shapes.push(Shape::Component(Component::new("B", 10.0, 0.0)));
        let line = |x, y| Node::new(x, y, NodeKind::Line);
        l.shapes.push(Shape::Path(Path::new(false, vec![line(200.0, 0.0), line(300.0, 0.0), line(300.0, 100.0)])));
        l.anchors.push(Anchor::new("top", 50.0, 700.0));
        l.info.guides.push(Guide { x: 20.0, angle: 90.0, ..Guide::default() });
        Ok(())
    })
    .unwrap();
    s
}

fn shapes(s: &Session, glyph: &str) -> Vec<Shape> {
    s.font().unwrap().glyph(glyph).unwrap().layer("m01").unwrap().shapes.clone()
}

fn nodes(shape: &Shape) -> Vec<(f64, f64, NodeKind)> {
    match shape {
        Shape::Path(p) => p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect(),
        other => panic!("not a path: {other:?}"),
    }
}

/// The on-curve points of a path, sorted.
fn corners(shape: &Shape) -> Vec<(f64, f64)> {
    let Shape::Path(p) = shape else { panic!("not a path: {shape:?}") };
    let mut v: Vec<(f64, f64)> = p.nodes.iter().filter(|n| n.on_curve()).map(|n| (n.x, n.y)).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn closed(shape: &Shape) -> bool {
    matches!(shape, Shape::Path(p) if p.closed)
}

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000">{body}</svg>"#)
}

#[test]
fn copy_then_paste_gives_the_same_shapes() {
    let mut s = session();
    let copied = run(&mut s, "edit.copy", json!({"glyph": "A"}));
    let (text, drawing) = (copied["json"].as_str().unwrap().to_string(), copied["svg"].as_str().unwrap().to_string());
    assert!(copied.get("skipped").is_none());
    // Copy is a query: no undo step.
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);

    let v = run(&mut s, "edit.paste", json!({"glyph": "C", "json": text}));
    assert_eq!(v["pasted"], 5, "three shapes, an anchor and a guide: {v}");
    assert_eq!(v["flavour"], "json");
    assert_eq!(v["outOfBounds"], false);
    assert_eq!(v["selection"]["shapes"], json!([0, 1, 2]));
    assert_eq!(v["selection"]["anchors"], json!(["top"]));
    assert_eq!(v["selection"]["guides"], json!([{"scope": "local", "index": 0}]));
    let font = s.font().unwrap();
    let (a, c) = (font.glyph("A").unwrap().layer("m01").unwrap(), font.glyph("C").unwrap().layer("m01").unwrap());
    assert_eq!(c.shapes, a.shapes);
    assert_eq!(c.anchors, a.anchors);
    assert_eq!(c.info.guides, a.info.guides);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label.as_ref(), "Paste");
    run(&mut s, "edit.undo", json!({}));
    assert!(shapes(&s, "C").is_empty(), "the paste is one undo step");

    // The SVG flavour pastes back at the copied coordinates; the empty component adds nothing.
    let v = run(&mut s, "edit.paste", json!({"glyph": "D", "svg": drawing}));
    assert_eq!(v["flavour"], "svg");
    let d = shapes(&s, "D");
    assert_eq!(d.len(), 2, "{d:?}");
    assert_eq!(nodes(&d[0]), nodes(&shapes(&s, "A")[0]));
    assert!(closed(&d[0]));
    assert_eq!(nodes(&d[1]), nodes(&shapes(&s, "A")[2]));
    assert!(!closed(&d[1]));

    // The same text through `text`: told apart by its content.
    let v = run(&mut s, "edit.paste", json!({"glyph": "E", "text": copied["json"]}));
    assert_eq!(v["flavour"], "json");
    let v = run(&mut s, "edit.paste", json!({"glyph": "F", "text": copied["svg"]}));
    assert_eq!(v["flavour"], "svg");
    // `json` wins over `svg`; `svg` is the fallback for a fragment that cannot be read.
    let v = run(&mut s, "edit.paste", json!({"glyph": "G", "json": copied["json"], "svg": copied["svg"]}));
    assert_eq!(v["flavour"], "json");
    let v = run(&mut s, "edit.paste", json!({"glyph": "H", "json": "{not json", "svg": copied["svg"]}));
    assert_eq!(v["flavour"], "svg");

    // Global guides go back to the master.
    let copied = run(&mut s, "edit.copy", json!({"glyph": "A", "guides": [{"scope": "global", "index": 0}]}));
    let v = run(&mut s, "edit.paste", json!({"glyph": "I", "json": copied["json"]}));
    assert_eq!(v["selection"]["guides"], json!([{"scope": "global", "index": 1}]));
    assert_eq!(s.font().unwrap().default_master().unwrap().guides.len(), 2);
}

#[test]
fn a_partly_selected_path_is_copied_as_open_paths() {
    let mut s = session();
    let paste_into = |s: &mut Session, glyph: &str, targets: Value| {
        let mut params = targets;
        params["glyph"] = json!("A");
        let copied = run(s, "edit.copy", params);
        run(s, "edit.paste", json!({"glyph": glyph, "json": copied["json"]}));
        shapes(s, glyph)
    };
    let line = |x, y| (x, y, NodeKind::Line);

    // Nodes 0, 1 and 2 of the square select two segments: one open path along them.
    let c = paste_into(&mut s, "C", json!({"points": [[0, 0], [0, 1], [0, 2]]}));
    assert_eq!(c.len(), 1);
    assert!(!closed(&c[0]));
    assert_eq!(nodes(&c[0]), vec![line(0.0, 0.0), line(100.0, 0.0), line(100.0, 100.0)]);

    // A run across the closing segment stays one path.
    let d = paste_into(&mut s, "D", json!({"points": [[0, 3], [0, 0], [0, 1]]}));
    assert_eq!(d.len(), 1);
    assert_eq!(nodes(&d[0]), vec![line(0.0, 100.0), line(0.0, 0.0), line(100.0, 0.0)]);

    // Every on-curve node selected: the whole closed path.
    let e = paste_into(&mut s, "E", json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]}));
    assert_eq!(e, vec![shapes(&s, "A")[0].clone()]);

    // A segment named directly, and an anchor with it.
    let f = paste_into(&mut s, "F", json!({"segments": [[2, 2]], "anchors": ["top"]}));
    assert_eq!(nodes(&f[0]), vec![line(300.0, 0.0), line(300.0, 100.0)]);
    assert_eq!(s.font().unwrap().glyph("F").unwrap().layer("m01").unwrap().anchors.len(), 1);

    // A lone node makes no segment: nothing to copy.
    assert!(s.execute("edit.copy", &json!({"glyph": "A", "points": [[0, 0]]})).is_err());
}

#[test]
fn cut_copies_then_deletes_as_one_undo_step() {
    let mut s = session();
    let v = run(&mut s, "edit.cut", json!({"glyph": "A", "shapes": [0], "anchors": ["top"]}));
    assert!(v["json"].as_str().unwrap().contains("fontcraft-clipboard"));
    assert!(v["svg"].as_str().unwrap().contains("<path"));
    let a = s.font().unwrap().glyph("A").unwrap().layer("m01").unwrap().clone();
    assert_eq!(a.shapes.len(), 2);
    assert!(a.anchors.is_empty());
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label.as_ref(), "Cut");
    run(&mut s, "edit.paste", json!({"glyph": "C", "json": v["json"]}));
    assert_eq!(shapes(&s, "C"), vec![Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0))]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shapes(&s, "A").len(), 3, "one undo step brings the cut shape back");
}

#[test]
fn paste_places_rounds_and_reports_far_outside() {
    let mut s = session();
    let far = svg(r#"<rect x="5000.4" y="0" width="100.2" height="100"/>"#);
    let v = run(&mut s, "edit.paste", json!({"glyph": "C", "svg": far}));
    assert_eq!(v["outOfBounds"], true, "{v}");
    let b: Vec<f64> = v["bounds"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect();
    assert_eq!(b, vec![5000.0, -100.0, 5101.0, 0.0], "rounded to the grid of 1");
    let c: Vec<f64> = v["correction"].as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect();
    assert_eq!(c, vec![-5000.0, 100.0]);

    // `at` puts the box's bottom left there, then the grid rounds.
    let v = run(&mut s, "edit.paste", json!({"glyph": "D", "svg": far, "at": [50.6, 0]}));
    assert_eq!(v["outOfBounds"], false);
    assert!(v.get("correction").is_none());
    assert_eq!(corners(&shapes(&s, "D")[0]), vec![(51.0, 0.0), (51.0, 100.0), (151.0, 0.0), (151.0, 100.0)]);

    // A grid of 0 keeps fractions; a grid of 10 rounds to tens.
    s.edit("grid", |font| {
        Arc::make_mut(&mut font.info).settings.grid_length = Some(0.0);
        Ok(())
    })
    .unwrap();
    run(&mut s, "edit.paste", json!({"glyph": "E", "svg": far}));
    let xs: Vec<f64> = corners(&shapes(&s, "E")[0]).iter().map(|p| p.0).collect();
    assert!((xs[0] - 5000.4).abs() < 1e-9 && (xs[3] - 5100.6).abs() < 1e-9, "{xs:?}");
    s.edit("grid", |font| {
        Arc::make_mut(&mut font.info).settings.grid_length = Some(10.0);
        Ok(())
    })
    .unwrap();
    run(&mut s, "edit.paste", json!({"glyph": "F", "svg": far}));
    assert_eq!(corners(&shapes(&s, "F")[0]), vec![(5000.0, -100.0), (5000.0, 0.0), (5100.0, -100.0), (5100.0, 0.0)]);
}

#[test]
fn an_svg_drawing_pastes_as_outlines_with_styling_dropped() {
    let mut s = session();
    let drawing = svg(r#"<rect x="100" y="100" width="200" height="300" fill="red"/>
        <circle cx="500" cy="500" r="100"/>
        <path d="M 600 800 A 100 100 0 0 1 800 800 Z"/>
        <g transform="translate(100 0)"><g transform="scale(2)"><rect x="0" y="0" width="10" height="5"/></g></g>"#);
    let v = run(&mut s, "edit.paste", json!({"glyph": "C", "text": drawing}));
    assert_eq!(v["pasted"], 4);
    assert_eq!(v["flavour"], "svg");
    assert_eq!(v["outOfBounds"], false);
    assert!(v["notes"].as_str().unwrap().contains("fill"), "{v}");
    let c = shapes(&s, "C");
    assert_eq!(corners(&c[0]), vec![(100.0, -400.0), (100.0, -100.0), (300.0, -400.0), (300.0, -100.0)]);
    for curved in [&c[1], &c[2]] {
        assert!(closed(curved));
        assert!(nodes(curved).iter().any(|n| n.2 == NodeKind::Curve), "{curved:?}");
    }
    assert_eq!(corners(&c[3]), vec![(100.0, -10.0), (100.0, 0.0), (120.0, -10.0), (120.0, 0.0)]);
}

#[test]
fn hostile_clipboard_text_fails_with_a_message_and_changes_nothing() {
    let mut s = session();
    s.edit("lock", |font| {
        let mut z = font.glyph_mut("Z")?;
        z.layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        z.info.locked = true;
        Ok(())
    })
    .unwrap();
    let deep = svg(&format!("{}{}", "<g>".repeat(10_000), "</g>".repeat(10_000)));
    let huge = svg(r#"<rect width="1e300" height="10"/>"#);
    let long = svg(&format!(r#"<path d="M0 0{}Z"/>"#, " L1 1".repeat(2_100_000)));
    let self_component = r#"{"format": "fontcraft-clipboard", "version": 1, "shapes": [{"component": {"glyph": "C"}}]}"#;
    let cases = [
        json!({"glyph": "C", "svg": deep}),
        json!({"glyph": "C", "svg": huge}),
        json!({"glyph": "C", "svg": long}),
        json!({"glyph": "C", "svg": "<svg"}),
        json!({"glyph": "C", "svg": svg("")}),
        json!({"glyph": "C", "json": r#"{"format": "fontcraft-clipboard", "version": 99}"#}),
        json!({"glyph": "C", "json": r#"{"format": "fontcraft-clipboard", "version": 1, "shapes": [{"path": {"closed": true, "nodes": [{"x": 1e300, "y": 0, "type": "line"}]}}]}"#}),
        json!({"glyph": "C", "json": self_component}),
        json!({"glyph": "C", "text": "hello"}),
        json!({"glyph": "C"}),
        json!({"glyph": "C", "svg": svg(r#"<rect width="10" height="10"/>"#), "at": [1e300, 0]}),
        json!({"glyph": "C", "svg": svg(r#"<rect width="10" height="10"/>"#), "shapes": [0]}),
        json!({"glyph": "Z", "svg": svg(r#"<rect width="10" height="10"/>"#)}),
        json!({"glyph": "nope", "svg": svg(r#"<rect width="10" height="10"/>"#)}),
    ];
    for params in cases {
        let before = snapshot(&s);
        let err = match s.execute("edit.paste", &params) {
            Ok(v) => panic!("a hostile paste succeeded: {v}"),
            Err(e) => e,
        };
        assert!(!err.to_string().is_empty());
        assert_eq!(snapshot(&s), before, "a failed paste changed the session: {err}");
    }
    // A locked glyph can be copied from, not cut from.
    assert!(s.execute("edit.copy", &json!({"glyph": "Z"})).is_ok());
    let before = snapshot(&s);
    assert!(s.execute("edit.cut", &json!({"glyph": "Z"})).is_err());
    assert_eq!(snapshot(&s), before);
}

#[test]
fn import_outlines_matches_files_to_glyphs_by_name() {
    let dir = temp_dir("import-outlines");
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    };
    let tall = svg(r#"<rect x="0" y="0" width="100" height="800" fill="black"/>"#);
    let a = write("A.svg", &tall);
    let new = write("newglyph.svg", &tall);
    // `B` exists, so the broken file is read and fails rather than going unmatched.
    let broken = write("B.svg", "<svg");
    let small =
        write("small.svg", r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="0" y="0" width="10" height="80"/></svg>"#);
    let mut s = Session::with_document();

    let v = run(&mut s, "file.importOutlines", json!({"files": [a, new, broken]}));
    assert_eq!(v["imported"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["imported"][0]["glyph"], "A");
    assert_eq!(v["imported"][0]["paths"], 1);
    assert!(v["imported"][0]["notes"].as_str().unwrap().contains("fill"));
    assert_eq!(v["unmatched"], json!([new]));
    assert_eq!(v["failed"][0]["file"], json!(broken));
    assert_eq!(v["added"], json!([]));
    // One font unit per SVG unit, SVG y = 0 at the ascender (800).
    assert_eq!(corners(&shapes(&s, "A")[0]), vec![(0.0, 0.0), (0.0, 800.0), (100.0, 0.0), (100.0, 800.0)]);
    run(&mut s, "edit.undo", json!({}));
    assert!(shapes(&s, "A").is_empty(), "the import is one undo step");

    let v = run(&mut s, "file.importOutlines", json!({"files": [new], "onMissingGlyph": "add"}));
    assert_eq!(v["added"], json!(["newglyph"]));
    assert_eq!(shapes(&s, "newglyph").len(), 1);

    // An explicit glyph bypasses the names; `em` fits the drawing's box to the em.
    run(&mut s, "file.importOutlines", json!({"files": [small.clone()], "glyphs": ["B"], "scale": "em"}));
    assert_eq!(corners(&shapes(&s, "B")[0]), vec![(0.0, 0.0), (0.0, 800.0), (100.0, 0.0), (100.0, 800.0)]);
    run(&mut s, "file.importOutlines", json!({"files": [small], "glyphs": ["C"]}));
    assert_eq!(corners(&shapes(&s, "C")[0]), vec![(0.0, 720.0), (0.0, 800.0), (10.0, 720.0), (10.0, 800.0)]);

    for bad in [
        json!({"files": [broken]}),
        json!({"files": [a.clone(), a.clone()], "glyphs": ["A"]}),
        json!({"files": []}),
        json!({"files": [a.clone()], "onMissingGlyph": "ask"}),
        json!({"files": [a.clone()], "scale": "huge"}),
    ] {
        let before = snapshot(&s);
        assert!(s.execute("file.importOutlines", &bad).is_err(), "{bad}");
        assert_eq!(snapshot(&s), before);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
