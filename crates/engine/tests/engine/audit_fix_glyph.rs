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

/// Flattening one component into two moves the shapes after it but no path: a stem hint on the
/// path after it stays on its nodes and a corner component on that path follows it to shape 2.
#[test]
fn component_flatten_keeps_hints_on_the_path_after_the_component() {
    use fontcraft_engine::model::Path;
    use fontcraft_engine::model::corner::{self, CornerHint, CornerTarget};
    use fontcraft_hinting::{Edge, NodeRef, PsHint, ps_hints, set_ps_hints};
    let mut s = Session::with_document();
    add(&mut s, "a", rect(0.0, 0.0, 100.0, 100.0));
    run(&mut s, "glyph.newGlyph", json!({"name": "_corner.one"}));
    s.edit("components", |font| {
        font.glyph_mut("b")?.layers[0].shapes =
            vec![Shape::Component(Component::new("a", 0.0, 0.0)), Shape::Component(Component::new("a", 200.0, 0.0))];
        let mut c = font.glyph_mut("c")?;
        let l = &mut c.layers[0];
        l.shapes = vec![Shape::Component(Component::new("b", 0.0, 0.0)), Shape::Path(Path::rect(400.0, 0.0, 500.0, 700.0))];
        let node = |n| Edge::Node(NodeRef::new(0, n));
        set_ps_hints(l, &[PsHint::stem(true, node(0), node(3))]).unwrap();
        l.info.hints.push(CornerHint::new("_corner.one", 1, 1).to_hint());
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "component.flatten", json!({"glyph": "c"}));
    assert_eq!(r["replaced"], json!(1), "{r}");
    let font = s.font().unwrap();
    let l = &font.glyph("c").unwrap().layers[0];
    assert_eq!(l.shapes.len(), 3, "{:?}", l.shapes);
    let ps = ps_hints(l).unwrap();
    assert_eq!(ps.len(), 1, "the stem is kept: {:?}", l.info.hints);
    assert_eq!(ps[0].1.edges(l).unwrap(), (0.0, 700.0));
    let corners: Vec<CornerTarget> = l.info.hints.iter().filter_map(|h| corner::corner_hint(h).unwrap()).map(|c| c.target).collect();
    assert_eq!(corners, [CornerTarget::Node { shape: 2, node: 1 }]);
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

#[test]
fn audit_fix_puts_the_shapes_of_a_master_in_the_proposed_contour_order() {
    use fontcraft_engine::model::{Axis, Glyph, Layer, Master, Path};
    use std::sync::Arc;
    let mut s = Session::with_document();
    let small = || Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0));
    let big = |grow: f64| Shape::Path(Path::rect(200.0, 0.0, 500.0 + grow, 300.0 + grow));
    s.edit("setup", |font| {
        font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
        let masters = Arc::make_mut(&mut font.masters);
        masters.truncate(1);
        masters[0].location = vec![400.0];
        masters.push(Master { id: "m02".into(), name: "Bold".into(), location: vec![700.0], ..Default::default() });
        let first = masters[0].id.clone();
        let mut g = Glyph::new("oo");
        let mut a = Layer::new(&first, 600.0);
        a.shapes = vec![small(), big(0.0)];
        let mut b = Layer::new("m02", 600.0);
        b.shapes = vec![big(20.0), small()];
        g.layers = vec![a, b];
        font.glyphs.push(g);
        Ok(())
    })
    .unwrap();
    let before = s.font().unwrap().glyph("oo").unwrap().layers[0].clone();
    let r = fix_one(&mut s, "check.variations.contourOrder", Some("oo"), "glyph.setShapeOrder");
    assert_eq!(r["applied"][0]["params"]["order"], json!([1, 0]), "{r}");
    let font = s.font().unwrap();
    let g = font.glyph("oo").unwrap();
    let bold = g.layers.iter().find(|l| l.id == "m02").unwrap();
    assert_eq!(bold.shapes, vec![small(), big(20.0)], "the bold master follows the regular one");
    assert_eq!(g.layers[0], before, "the reference master is left as it was");
}

#[test]
fn audit_fix_first_stage_resets_a_code_point_from_the_glyph_name() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.setUnicodes", json!({"glyphs": ["A"], "unicodes": ["E000"]}));
    fix_one(&mut s, "check.glyphset.nameCodepointMismatch", Some("A"), "glyph.updateGlyphInfo");
    assert_eq!(s.font().unwrap().glyph("A").unwrap().unicodes, vec![0x41]);
}

#[test]
fn audit_fix_first_stage_unmaps_a_code_point_the_compiled_font_reports() {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "softhyphen", "unicodes": [0xAD, 0xE001], "width": 300}));
    let r = fix_one(&mut s, "check.glyphset.softHyphen", None, "glyph.setUnicodes");
    assert_eq!(r["applied"][0]["params"], json!({"glyphs": ["softhyphen"], "unicodes": [0xE001]}), "{r}");
    let font = s.font().unwrap();
    assert_eq!(font.glyph("softhyphen").unwrap().unicodes, vec![0xE001], "the other code point stays");
}

#[test]
fn audit_fix_last_stage_deletes_obsolete_and_redundant_kerning() {
    use std::sync::Arc;
    let mut s = Session::with_document();
    let master = s.font().unwrap().masters[0].id.clone();
    s.edit("kerning", |font| {
        let k = Arc::make_mut(&mut font.kerning);
        k.ltr.set(&master, "A", "V", 0.0);
        k.ltr.set(&master, "A", "missing", -20.0);
        k.ltr.set(&master, "T", "O", -30.0);
        Ok(())
    })
    .unwrap();
    let checks = ["check.kerning.redundantPairs", "check.kerning.missingGlyphReference"];
    let select = json!({"profile": "strict", "checks": checks});
    let doc = run(&mut s, "audit.run", select.clone());
    assert!(checks.iter().all(|c| count(&doc, c) == 1), "{doc}");
    let steps = undo_steps(&s);
    let mut p = select.clone();
    p["safeOnly"] = json!(false);
    let r = run(&mut s, "audit.fix", p);
    let commands: Vec<&str> = r["applied"].as_array().unwrap().iter().map(|a| a["command"].as_str().unwrap()).collect();
    assert_eq!(commands, ["kerning.cleanUp", "kerning.removePair"], "{r}");
    assert!(checks.iter().all(|c| count(&r["remaining"], c) == 0), "{r}");
    assert_eq!(undo_steps(&s), steps + 1, "one undo step");
    let font = s.font().unwrap();
    assert_eq!(font.kerning.ltr.get(&master, "A", "V"), None);
    assert_eq!(font.kerning.ltr.get(&master, "A", "missing"), None);
    assert_eq!(font.kerning.ltr.get(&master, "T", "O"), Some(-30.0), "a pair no check reports stays");
}

#[test]
fn audit_fix_reverses_a_path_that_runs_the_other_way_in_a_master() {
    use fontcraft_engine::model::{Axis, Glyph, Layer, Master, Node, NodeKind, Path};
    use std::sync::Arc;
    let mut s = Session::with_document();
    let poly = |points: &[(f64, f64)]| Shape::Path(Path::new(true, points.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()));
    let regular = [(0.0, 0.0), (300.0, 0.0), (400.0, 200.0), (150.0, 350.0), (-50.0, 150.0)];
    // The same outline 10% larger, drawn the other way round from another node.
    let bold = [(440.0, 220.0), (330.0, 0.0), (0.0, 0.0), (-55.0, 165.0), (165.0, 385.0)];
    s.edit("setup", |font| {
        font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
        let masters = Arc::make_mut(&mut font.masters);
        masters.truncate(1);
        masters[0].location = vec![400.0];
        masters.push(Master { id: "m02".into(), name: "Bold".into(), location: vec![700.0], ..Default::default() });
        let first = masters[0].id.clone();
        let mut g = Glyph::new("pentagon");
        let mut a = Layer::new(&first, 600.0);
        a.shapes = vec![poly(&regular)];
        let mut b = Layer::new("m02", 600.0);
        b.shapes = vec![poly(&bold)];
        g.layers = vec![a, b];
        font.glyphs.push(g);
        Ok(())
    })
    .unwrap();
    let before = s.font().unwrap().glyph("pentagon").unwrap().layers[0].clone();
    let r = fix_one(&mut s, "check.variations.wrongStartPoint", Some("pentagon"), "path.reverseContours");
    assert_eq!(r["applied"][1]["command"], "path.makeNodeFirst", "{r}");
    let font = s.font().unwrap();
    let g = font.glyph("pentagon").unwrap();
    let b = g.layers.iter().find(|l| l.id == "m02").unwrap();
    let Shape::Path(path) = &b.shapes[0] else { panic!("a path") };
    let at: Vec<(f64, f64)> = path.nodes.iter().map(|n| (n.x, n.y)).collect();
    assert_eq!(at, [(0.0, 0.0), (330.0, 0.0), (440.0, 220.0), (165.0, 385.0), (-55.0, 165.0)], "the bold path follows the regular one");
    assert_eq!(g.layers[0], before, "the reference master is left as it was");
}

#[test]
fn audit_fix_last_stage_renames_a_mistyped_language_tag_in_the_feature_code() {
    let mut s = Session::with_document();
    let prefix = "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\nlanguagesystem latn TUR;\n";
    run(&mut s, "font.features.addPrefix", json!({"name": "languagesystems", "code": prefix}));
    run(&mut s, "font.features.addFeature", json!({"tag": "locl", "code": "script latn;\nlanguage TUR; # Turkish\nsub i by j;\n"}));
    fix_one(&mut s, "check.layout.validLanguageTags", None, "font.features.setCode");
    let f = &s.font().unwrap().features;
    assert_eq!(f.prefixes[0].code, "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\nlanguagesystem latn TRK;\n");
    assert_eq!(f.features[0].code, "script latn;\nlanguage TRK; # Turkish\nsub i by j;\n");
}

#[test]
fn audit_fix_last_stage_puts_the_default_languagesystem_first() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addPrefix", json!({"name": "languagesystems", "code": "languagesystem latn dflt;\nlanguagesystem DFLT dflt;\n"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by a;\n"}));
    fix_one(&mut s, "check.layout.defaultLanguagesystemStatement", None, "font.features.setCode");
    let f = &s.font().unwrap().features;
    assert_eq!(f.prefixes[0].code, "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\n");
    assert_eq!(f.features[0].code, "sub f i by a;\n", "the features are left as they were");
}

#[test]
fn audit_fix_last_stage_moves_small_caps_before_ligatures() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by a;\n"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "smcp", "code": "sub a by b;\n"}));
    fix_one(&mut s, "check.layout.smallCapsBeforeLigatures", None, "font.features.reorder");
    let f = &s.font().unwrap().features;
    let names: Vec<&str> = f.features.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["smcp", "liga"]);
    assert_eq!(f.features[1].code, "sub f i by a;\n", "the code is left as it was");
}

#[test]
fn audit_fix_last_stage_removes_the_stored_pairs_behind_tabular_kerning() {
    use fontcraft_engine::model::{Glyph, Layer};
    use std::sync::Arc;
    let mut s = Session::with_document();
    let master = s.font().unwrap().masters[0].id.clone();
    let figures = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
    s.edit("setup", |font| {
        for (i, name) in figures.iter().enumerate() {
            let mut g = Glyph::new(*name);
            g.unicodes.push(0x30 + i as u32);
            g.layers.push(Layer::new(&master, 500.0));
            match *name {
                "two" | "three" => g.kern_groups.right = Some("figs".into()),
                "four" => g.kern_groups.left = Some("fourX".into()),
                _ => {}
            }
            font.glyphs.push(g);
        }
        for (name, group) in [("O", "O"), ("X", "fourX")] {
            font.glyphs.get_mut(name).unwrap().kern_groups.left = Some(group.into());
        }
        let k = Arc::make_mut(&mut font.kerning);
        k.ltr.set(&master, "one", "A", -50.0);
        k.ltr.set(&master, "@MMK_L_figs", "@MMK_R_O", -20.0);
        k.ltr.set(&master, "V", "@MMK_R_fourX", -30.0);
        k.ltr.set(&master, "T", "O", -40.0);
        Ok(())
    })
    .unwrap();
    let check = "check.kerning.tabularKerning";
    let select = json!({"profile": "strict", "checks": [check]});
    let doc = run(&mut s, "audit.run", select.clone());
    assert_eq!(count(&doc, check), 4, "{doc}");
    let steps = undo_steps(&s);
    let mut p = select.clone();
    p["safeOnly"] = json!(false);
    let r = run(&mut s, "audit.fix", p);
    let removed: Vec<(&str, &str)> =
        r["applied"].as_array().unwrap().iter().map(|a| (a["params"]["left"].as_str().unwrap(), a["params"]["right"].as_str().unwrap())).collect();
    assert_eq!(removed, [("@figs", "@O"), ("one", "A")], "{r}");
    assert!(r["skipped"].as_array().unwrap().iter().any(|x| x["reason"].as_str().unwrap().starts_with("V @fourX")), "{r}");
    assert_eq!(count(&r["remaining"], check), 1, "only V four is left: {r}");
    assert_eq!(undo_steps(&s), steps + 1, "one undo step");
    let font = s.font().unwrap();
    assert_eq!(font.kerning.ltr.get(&master, "V", "@MMK_R_fourX"), Some(-30.0), "the pair also kerns X");
    assert_eq!(font.kerning.ltr.get(&master, "T", "O"), Some(-40.0), "a pair without figures stays");
}
