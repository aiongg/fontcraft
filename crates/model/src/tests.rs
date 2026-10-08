use std::sync::Arc;

use fontcraft_geom::{PathEl, Point, Shape as _};

use crate::*;

fn path(closed: bool, nodes: Vec<Node>) -> Path {
    Path::new(closed, nodes)
}

#[test]
fn a_new_font_is_valid_and_has_the_starting_glyphs() {
    let f = Font::new("Test");
    assert_eq!(f.check(), Ok(()));
    assert_eq!(f.glyphs.first().map(|g| g.name.as_str()), Some(".notdef"));
    assert_eq!(f.glyph("A").map(|g| g.unicodes.clone()), Some(vec![0x41]));
    assert_eq!(f.glyphs.len(), 2 + 52);
    let m = f.default_master().unwrap();
    assert_eq!(f.metric_pos(m, metric::ASCENDER, 0.0), 800.0);
    assert_eq!(f.metric_pos(m, metric::DESCENDER, 0.0), -200.0);
    assert_eq!(f.metric(m, metric::ITALIC_ANGLE), None);
}

#[test]
fn metrics_are_set_through_their_definitions() {
    let mut f = Font::new("Test");
    f.set_metric_pos("m01", metric::X_HEIGHT, 520.0).unwrap();
    f.set_metric_pos("m01", metric::ITALIC_ANGLE, 12.0).unwrap();
    let m = f.default_master().unwrap();
    assert_eq!(f.metric_pos(m, metric::X_HEIGHT, 0.0), 520.0);
    assert_eq!(f.metric_pos(m, metric::ITALIC_ANGLE, 0.0), 12.0);
    assert_eq!(f.metrics.len(), 6);
    assert!(f.set_metric_pos("nope", metric::X_HEIGHT, 1.0).is_err());
}

#[test]
fn rect_is_a_closed_four_segment_path() {
    let bez = Path::rect(0.0, 0.0, 10.0, 20.0).to_bez().unwrap();
    assert_eq!(bez.elements().len(), 5); // move, three lines, close
    assert_eq!(bez.bounding_box(), fontcraft_geom::Rect::new(0.0, 0.0, 10.0, 20.0));
}

#[test]
fn a_closed_path_may_start_with_the_off_curves_of_its_last_segment() {
    let n = Node::new;
    // off, off, curve(0,0) closes back; then a line: the same loop written two ways.
    let a = path(
        true,
        vec![
            n(0.0, 0.0, NodeKind::Line),
            n(100.0, 0.0, NodeKind::Line),
            n(100.0, 50.0, NodeKind::OffCurve),
            n(50.0, 100.0, NodeKind::OffCurve),
            n(0.0, 100.0, NodeKind::Curve),
        ],
    );
    let b = path(
        true,
        vec![
            n(100.0, 50.0, NodeKind::OffCurve),
            n(50.0, 100.0, NodeKind::OffCurve),
            n(0.0, 100.0, NodeKind::Curve),
            n(0.0, 0.0, NodeKind::Line),
            n(100.0, 0.0, NodeKind::Line),
        ],
    );
    let (a, b) = (a.to_bez().unwrap(), b.to_bez().unwrap());
    assert!(a.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))));
    assert!((a.area() - b.area()).abs() < 1e-9);
}

#[test]
fn broken_node_lists_are_errors_not_panics() {
    let n = Node::new;
    for nodes in [
        vec![n(0.0, 0.0, NodeKind::OffCurve)],
        vec![n(0.0, 0.0, NodeKind::Line), n(1.0, 1.0, NodeKind::OffCurve), n(2.0, 2.0, NodeKind::Curve)],
        vec![n(0.0, 0.0, NodeKind::Line), n(1.0, 1.0, NodeKind::OffCurve), n(2.0, 2.0, NodeKind::Line)],
        vec![n(f64::NAN, 0.0, NodeKind::Line)],
        vec![n(0.0, f64::INFINITY, NodeKind::Line)],
        vec![n(MAX_COORDINATE + 1.0, 0.0, NodeKind::Line)],
        vec![n(0.0, 0.0, NodeKind::Line), n(1.0, 1.0, NodeKind::QCurve)],
    ] {
        assert!(path(true, nodes).to_bez().is_err());
    }
    assert!(path(false, vec![n(1.0, 1.0, NodeKind::OffCurve), n(0.0, 0.0, NodeKind::Line)]).to_bez().is_err());
    assert!(path(false, vec![n(1.0, 1.0, NodeKind::OffCurve), n(2.0, 0.0, NodeKind::OffCurve)]).to_bez().is_err());
    assert_eq!(path(true, vec![]).to_bez().map(|b| b.elements().len()), Ok(0));
}

#[test]
fn a_closed_path_of_only_off_curve_nodes_is_a_quadratic_spline() {
    let n = Node::new;
    // Four control points of a rounded square: on-curve points are implied at the midpoints.
    let bez = path(
        true,
        vec![
            n(0.0, 0.0, NodeKind::OffCurve),
            n(100.0, 0.0, NodeKind::OffCurve),
            n(100.0, 100.0, NodeKind::OffCurve),
            n(0.0, 100.0, NodeKind::OffCurve),
        ],
    )
    .to_bez()
    .unwrap();
    assert_eq!(bez.elements().first(), Some(&PathEl::MoveTo(Point::new(0.0, 50.0))));
    assert_eq!(bez.elements().iter().filter(|e| matches!(e, PathEl::QuadTo(..))).count(), 4);
    assert!(bez.area().abs() > 5000.0 && bez.area().abs() < 10000.0);
}

#[test]
fn components_resolve_and_cycles_are_errors() {
    let mut f = Font::new("Test");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    f.glyph_mut("B").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 50.0, 0.0)));
    assert_eq!(f.outline("B", "m01").unwrap().bounding_box(), fontcraft_geom::Rect::new(50.0, 0.0, 150.0, 100.0));
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
    assert!(matches!(f.outline("B", "m01"), Err(ModelError::ComponentCycle(_))));
}

#[test]
fn decomposed_and_matrix_transforms_agree() {
    let d = Transform::Decomposed { pos: (10.0, 20.0), scale: (2.0, 3.0), angle: 90.0, slant: (0.0, 0.0) };
    let a = d.affine().as_coeffs();
    // Scale, then rotate a quarter turn counter-clockwise, then move.
    let expected = [0.0, 2.0, -3.0, 0.0, 10.0, 20.0];
    for (x, y) in a.iter().zip(expected) {
        assert!((x - y).abs() < 1e-12, "{a:?}");
    }
    assert!(Transform::default().is_identity());
    assert!(Transform::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).is_identity());
    let slant = Transform::Decomposed { pos: (0.0, 0.0), scale: (1.0, 1.0), angle: 0.0, slant: (45.0, 0.0) };
    let p = slant.affine() * Point::new(0.0, 100.0);
    assert!((p.x - 100.0).abs() < 1e-9 && (p.y - 100.0).abs() < 1e-9, "{p:?}");
}

#[test]
fn check_rejects_duplicates_and_orphan_layers() {
    let mut f = Font::new("Test");
    assert_eq!(f.add_glyph("A", vec![], 500.0), Err(ModelError::DuplicateGlyph("A".into())));
    assert_eq!(f.add_glyph("", vec![], 500.0), Err(ModelError::EmptyGlyphName));
    f.glyph_mut("A").unwrap().layers[0].master = "nope".into();
    assert_eq!(f.check(), Err(ModelError::NoSuchMaster("nope".into())));
}

#[test]
fn non_master_layers_are_kept_beside_the_master_layer() {
    let mut f = Font::new("Test");
    let mut backup = Layer::new("m01", 300.0);
    backup.id = "B5D6A6C4-backup".into();
    f.glyph_mut("A").unwrap().layers.push(backup);
    assert_eq!(f.check(), Ok(()));
    let a = f.glyph("A").unwrap();
    assert_eq!(a.layer("m01").map(|l| l.width), Some(600.0), "the master layer, not the backup");
    assert_eq!(a.layers.iter().filter(|l| !l.is_master_layer()).count(), 1);
}

#[test]
fn check_holds_every_limit() {
    let ok = Font::new("Test");
    let broken = |edit: &dyn Fn(&mut Font)| {
        let mut f = ok.clone();
        edit(&mut f);
        f.check().is_err()
    };
    assert!(broken(&|f| f.family_name = "x".repeat(MAX_NAME_LEN + 1)));
    assert!(broken(&|f| f.family_name = " ".into()));
    assert!(broken(&|f| {
        let m = f.masters[0].clone();
        Arc::make_mut(&mut f.masters).push(m);
    }));
    assert!(broken(&|f| Arc::make_mut(&mut f.masters)[0].metric_values[0].pos = f64::NAN));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().name = "a\u{0}b".into()));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().unicodes = vec![0xD800]));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().unicodes = vec![u32::MAX]));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().unicodes = (0..=MAX_UNICODES as u32).collect()));
    assert!(broken(&|f| {
        let mut g = f.glyph_mut("A").unwrap();
        let l = g.layers[0].clone();
        g.layers.push(l);
    }));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].width = -1e308));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(path(true, vec![Node::new(0.0, 0.0, NodeKind::Curve)])))));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Component(Component {
        glyph: "B".into(),
        transform: Transform::Matrix([f64::NAN; 6]),
        extra: Extra::new()
    }))));
    assert!(broken(&|f| f.glyph_mut("A").unwrap().layers[0].anchors.push(Anchor::new("top", f64::INFINITY, 0.0))));
    assert_eq!(ok.check(), Ok(()));
}

#[test]
fn check_changed_looks_only_at_edited_glyphs() {
    let before = Font::new("Test");
    let mut after = before.clone();
    after.glyph_mut("A").unwrap().layers[0].width = f64::NAN;
    assert!(after.check_changed(&before).is_err());
    // A glyph still shared with `before` is trusted, whatever `before` held.
    let mut bad_before = before.clone();
    bad_before.glyph_mut("B").unwrap().layers[0].width = f64::NAN;
    let mut after = bad_before.clone();
    after.glyph_mut("A").unwrap().layers[0].width = 10.0;
    assert_eq!(after.check_changed(&bad_before), Ok(()));
    assert!(after.check().is_err());
}

#[test]
fn the_name_index_follows_adds_removals_and_renames() {
    let mut f = Font::new("Test");
    f.add_glyph("A.alt", vec![], 500.0).unwrap();
    assert_eq!(f.glyphs.position("A.alt"), Some(f.glyphs.len() - 1));
    f.glyph_mut("A").unwrap().name = "Aa".into();
    assert!(f.glyph("A").is_none());
    assert_eq!(f.glyph("Aa").map(|g| g.unicodes.clone()), Some(vec![0x41]));
    f.remove_glyph("space").unwrap();
    assert_eq!(f.glyphs.position("Aa"), Some(1), "positions after the removed glyph move up");
    assert_eq!(f.glyph("A.alt").map(|g| g.name.as_str()), Some("A.alt"));
    assert_eq!(f.remove_glyph("space"), Err(ModelError::NoSuchGlyph("space".into())));
}

#[test]
fn an_edit_that_changes_nothing_compares_equal_cheaply() {
    let before = Font::new("Test");
    let mut after = before.clone();
    assert!(after.same_as(&before));
    // Touching a glyph copies it; an identical copy still compares equal.
    let w = after.glyph("A").unwrap().layers[0].width;
    after.glyph_mut("A").unwrap().layers[0].width = w;
    assert!(after.same_as(&before));
    after.glyph_mut("A").unwrap().layers[0].width = w + 1.0;
    assert!(!after.same_as(&before));
}

#[test]
fn snapshots_share_untouched_glyphs() {
    let before = Font::new("Test");
    let mut after = before.clone();
    after.glyph_mut("A").unwrap().layers[0].width = 1.0;
    let shared = after.glyphs.iter().zip(before.glyphs.iter()).filter(|(a, b)| Arc::ptr_eq(a, b)).count();
    assert_eq!(shared, before.glyphs.len() - 1);
}

#[test]
fn component_fan_out_and_far_transforms_are_bounded() {
    // g0 is a square; g(k) is two copies of g(k-1): 2^k squares.
    let mut f = Font::new("Test");
    f.add_glyph("g0", vec![], 500.0).unwrap();
    f.glyph_mut("g0").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
    for k in 1..=24 {
        f.add_glyph(&format!("g{k}"), vec![], 500.0).unwrap();
        let mut g = f.glyph_mut(&format!("g{k}")).unwrap();
        let shapes = &mut g.layers[0].shapes;
        shapes.push(Shape::Component(Component::new(format!("g{}", k - 1), 0.0, 0.0)));
        shapes.push(Shape::Component(Component::new(format!("g{}", k - 1), 1.0, 0.0)));
    }
    assert_eq!(f.check(), Ok(()));
    assert!(f.outline("g5", "m01").is_ok());
    let start = std::time::Instant::now();
    assert!(matches!(f.outline("g24", "m01"), Err(ModelError::OutlineTooLarge(_))));
    assert!(start.elapsed().as_secs() < 5, "the budget must stop the work early");

    let mut f = Font::new("Test");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    for huge in [[1e300, 0.0, 0.0, 1e300, 0.0, 0.0], [1e308, 1e308, -1e308, -1e308, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, 1e12, 0.0]] {
        let mut g = f.clone();
        g.glyph_mut("B").unwrap().layers[0].shapes =
            vec![Shape::Component(Component { glyph: "A".into(), transform: Transform::Matrix(huge), extra: Extra::new() })];
        assert!(matches!(g.outline("B", "m01"), Err(ModelError::BadOutline { .. })), "{huge:?}");
    }
}

#[test]
fn extra_entries_survive_the_native_form() {
    let mut f = Font::new("Test");
    let value = plist::openstep::parse_value("(\n1,\n\"two\"\n)").unwrap();
    Arc::make_mut(&mut f.extra).push(Family::Glyphs, "futureKey", value.clone());
    f.glyph_mut("A").unwrap().layers[0].extra.push(Family::Ufo, "public.markColor", plist::Value::String("1,0,0,1".into()));
    let json = serde_json::to_string(&f).unwrap();
    let back: Font = serde_json::from_str(&json).unwrap();
    assert_eq!(back, f);
    assert_eq!(back.extra.get(Family::Glyphs, "futureKey"), Some(&value));
    assert_eq!(back.glyph("A").unwrap().layers[0].extra.len(), 1);
}

#[test]
fn version_2_master_name_parts_join_as_the_reference_joins_them() {
    use crate::NameParts;
    let parts = |w: Option<&str>, d: Option<&str>, c: Option<&str>| NameParts {
        weight: w.map(Into::into),
        width: d.map(Into::into),
        custom: c.map(Into::into),
    };
    assert_eq!(parts(None, None, None).joined(false), "Regular");
    assert_eq!(parts(None, None, None).joined(true), "Italic");
    assert_eq!(parts(Some("Bold"), Some("Condensed"), None).joined(false), "Condensed Bold");
    assert_eq!(parts(Some("Bold"), None, None).joined(true), "Bold Italic");
    assert_eq!(parts(Some("Light"), None, Some("Oblique")).joined(true), "Light Oblique");
    assert_eq!(parts(None, None, Some("Display")).joined(false), "Display");
}
