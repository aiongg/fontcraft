//! The glyph and outline fixes of the audit (M12.62): the fix commands of their own and the layer
//! steps of `audit.fix` that call them. Fixtures are built in code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_engine::model::{Anchor, Component, Shape};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn line(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y})
}

fn curve(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y, "type": "curve"})
}

fn off(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y, "type": "offcurve"})
}

fn add(s: &mut Session, glyph: &str, nodes: Value) {
    run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes, "closed": true}));
}

fn count(doc: &Value, check: &str) -> usize {
    doc["results"].as_array().unwrap().iter().filter(|r| r["check"] == check).count()
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The on-curve and off-curve coordinates of the first path of `glyph`'s first layer.
fn coords(s: &Session, glyph: &str) -> Vec<(f64, f64)> {
    let font = s.font().unwrap();
    match font.glyph(glyph).unwrap().layers[0].shapes.first() {
        Some(Shape::Path(p)) => p.nodes.iter().map(|n| (n.x, n.y)).collect(),
        _ => panic!("{glyph} has no path"),
    }
}

/// Runs `check` alone on `glyph` (all glyphs when `None`), fixes it with `audit.fix` and returns
/// the fix's result after checking that the check is silent, that `command` made the first
/// change and that the whole fix is one undo step.
fn fix_one(s: &mut Session, check: &str, glyph: Option<&str>, command: &str) -> Value {
    let mut select = json!({"profile": "strict", "checks": [check]});
    if let Some(g) = glyph {
        select["glyphs"] = json!([g]);
    }
    let doc = run(s, "audit.run", select.clone());
    assert!(count(&doc, check) >= 1, "{check}: the fixture is reported: {doc}");
    let steps = undo_steps(s);
    let mut p = select.clone();
    p["safeOnly"] = json!(false);
    let r = run(s, "audit.fix", p);
    assert_eq!(r["applied"][0]["command"], json!(command), "{check}: {r}");
    assert_eq!(count(&r["remaining"], check), 0, "{check}: reported after the fix: {r}");
    assert_eq!(undo_steps(s), steps + 1, "{check}: one undo step");
    let again = run(s, "audit.run", select);
    assert_eq!(count(&again, check), 0, "{check}: silent on a new run: {again}");
    r
}

/// A square whose bottom edge runs 10 units past the right edge and turns back to it, leaving a
/// small loop outside the corner at (100, 0).
fn open_corner(s: &mut Session, glyph: &str) {
    add(s, glyph, json!([line(0.0, 0.0), line(110.0, 0.0), line(100.0, -10.0), line(100.0, 100.0), line(0.0, 100.0)]));
}

#[test]
fn path_erase_open_corners_trims_the_loop_to_one_corner_node() {
    let mut s = Session::with_document();
    open_corner(&mut s, "A");
    let r = run(&mut s, "path.eraseOpenCorners", json!({"glyph": "A", "points": [[0, 1]]}));
    assert_eq!(r["erased"], json!(1), "{r}");
    assert_eq!(coords(&s, "A"), vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);

    // Without points every open corner goes; a node with none is skipped and nothing changes.
    open_corner(&mut s, "B");
    let r = run(&mut s, "path.eraseOpenCorners", json!({"glyph": "B"}));
    assert_eq!(r["erased"], json!(1), "{r}");
    let steps = undo_steps(&s);
    let r = run(&mut s, "path.eraseOpenCorners", json!({"glyph": "B", "points": [[0, 0]]}));
    assert_eq!((r["erased"].clone(), r["skipped"].clone()), (json!(0), json!([[0, 0]])), "{r}");
    assert_eq!(undo_steps(&s), steps, "a call that erases nothing makes no undo step");
    assert!(s.execute("path.eraseOpenCorners", &json!({"glyph": "B", "points": [[3, 0]]})).is_err(), "a point that does not resolve");

    open_corner(&mut s, "C");
    fix_one(&mut s, "check.outlines.openCorners", Some("C"), "path.eraseOpenCorners");
    assert_eq!(coords(&s, "C"), vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
}

/// Glyph c uses b, which is made of a component of a, and a itself.
fn nested_document() -> Session {
    let mut s = Session::with_document();
    add(&mut s, "a", rect(0.0, 0.0, 100.0, 100.0));
    s.edit("components", |font| {
        font.glyph_mut("b")?.layers[0].shapes.push(Shape::Component(Component::new("a", 10.0, 0.0)));
        let mut c = font.glyph_mut("c")?;
        c.layers[0].shapes.push(Shape::Component(Component::new("b", 5.0, 20.0)));
        c.layers[0].shapes.push(Shape::Component(Component::new("a", 300.0, 0.0)));
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn component_flatten_replaces_a_nested_component_by_innermost_components() {
    let check = |s: &Session| {
        let font = s.font().unwrap();
        let shapes = &font.glyph("c").unwrap().layers[0].shapes;
        assert_eq!(shapes.len(), 2);
        let Shape::Component(first) = &shapes[0] else { panic!("a component") };
        assert_eq!(first.glyph, "a", "the nested component now names the innermost glyph");
        assert_eq!(first.affine().as_coeffs(), [1.0, 0.0, 0.0, 1.0, 15.0, 20.0], "the transforms are combined");
    };

    // The command: a component that is not nested is skipped when named.
    let mut s = nested_document();
    let r = run(&mut s, "component.flatten", json!({"glyph": "c", "components": [0, 1]}));
    assert_eq!((r["replaced"].clone(), r["skipped"].clone()), (json!(1), json!([1])), "{r}");
    check(&s);
    assert!(s.execute("component.flatten", &json!({"glyph": "a", "components": [0]})).is_err(), "a path is not a component");

    // The audit fix, with safeOnly: the component steps run on layers with components.
    let mut s = nested_document();
    let doc = run(&mut s, "audit.run", json!({"profile": "strict", "checks": ["check.components.nested"], "glyphs": ["c"]}));
    assert_eq!(count(&doc, "check.components.nested"), 1, "{doc}");
    let r = run(&mut s, "audit.fix", json!({"profile": "strict", "checks": ["check.components.nested"], "glyphs": ["c"]}));
    assert_eq!(r["applied"][0]["command"], json!("component.flatten"), "{r}");
    assert_eq!(count(&r["remaining"], "check.components.nested"), 0, "{r}");
    check(&s);
}

#[test]
fn audit_fix_layer_steps_snap_align_and_clean_up() {
    let mut s = Session::with_document();
    // The bottom edge one unit above the baseline.
    add(&mut s, "H", json!([line(0.0, 1.0), line(100.0, 1.0), line(100.0, 100.0), line(0.0, 100.0)]));
    fix_one(&mut s, "check.outlines.alignmentMiss", Some("H"), "selection.move");
    assert!(coords(&s, "H").iter().take(2).all(|&(_, y)| y == 0.0), "{:?}", coords(&s, "H"));

    // A handle 1.4° off horizontal.
    add(&mut s, "O", json!([line(0.0, 0.0), off(40.0, 1.0), off(100.0, 60.0), curve(100.0, 100.0), line(0.0, 100.0)]));
    fix_one(&mut s, "check.outlines.almostOrthogonalHandle", Some("O"), "selection.setPosition");
    assert_eq!(coords(&s, "O")[1].1, 0.0, "the handle is horizontal");

    // A smooth node whose handles turn by 2.9°.
    run(
        &mut s,
        "path.addPath",
        json!({"glyph": "S", "closed": true, "nodes": [
            line(0.0, 0.0), off(0.0, 50.0), off(60.0, 98.0), {"x": 100.0, "y": 100.0, "type": "curve", "smooth": true},
            off(140.0, 100.0), off(200.0, 50.0), curve(200.0, 0.0)
        ]}),
    );
    fix_one(&mut s, "check.outlines.kinkedSmoothNode", Some("S"), "selection.setPosition");

    // Two anchors named top: the first stays where it was.
    s.edit("anchors", |font| {
        let mut g = font.glyph_mut("A")?;
        g.layers[0].anchors.push(Anchor::new("top", 100.0, 700.0));
        g.layers[0].anchors.push(Anchor::new("top", 120.0, 710.0));
        Ok(())
    })
    .unwrap();
    fix_one(&mut s, "check.anchors.duplicateName", Some("A"), "anchor.remove");
    let font = s.font().unwrap();
    let anchors: Vec<(String, f64, f64)> = font.glyph("A").unwrap().layers[0].anchors.iter().map(|a| (a.name.clone(), a.x, a.y)).collect();
    assert_eq!(anchors, vec![("top".to_string(), 100.0, 700.0)]);
}

#[test]
fn audit_fix_zeroes_a_mark_width_reported_by_a_compiled_instance() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "acutecomb", "unicodes": [0x301], "width": 300}));
    // The export zeroes the advance of a glyph stored as a non-spacing mark, so only a combining
    // character stored with another subcategory keeps its width in the compiled font.
    run(&mut s, "glyph.setCategory", json!({"glyphs": ["acutecomb"], "category": "Mark", "subCategory": "Spacing Combining"}));
    let select = json!({"profile": "strict", "checks": ["check.metrics.markWidths"]});
    let doc = run(&mut s, "audit.run", select.clone());
    let result =
        doc["results"].as_array().unwrap().iter().find(|r| r["check"] == "check.metrics.markWidths" && r["location"]["glyph"] == "acutecomb");
    let id = result.unwrap_or_else(|| panic!("the mark's width is reported: {doc}"))["id"].clone();
    assert_eq!(result.unwrap()["fix"]["available"], json!(true), "{doc}");

    let r = run(&mut s, "audit.fix", json!({"results": [id]}));
    assert_eq!(r["applied"][0]["command"], json!("spacing.setWidth"), "{r}");
    let font = s.font().unwrap();
    let g = font.glyph("acutecomb").unwrap();
    assert!(g.layers.iter().filter(|l| l.is_master_layer()).all(|l| l.width == 0.0), "every master layer of the source glyph is fixed");
    let again = run(&mut s, "audit.run", select);
    assert_eq!(count(&again, "check.metrics.markWidths"), 0, "{again}");
}
