//! Path attributes of full-colour layers (M11.4): stroke, fill, gradient, the image-only effects,
//! the gray fill of a new path, refusals and undo.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_engine::model::{Color, Dimension, Extend, GradientKind, Layer, Shadow, Shape, ShapeAttrs};
use fontcraft_geom::Point;
use serde_json::{Value, json};

const GRAY: Color = Color::Rgba([0.5, 0.5, 0.5, 1.0]);
const RED: Color = Color::Rgba([1.0, 0.0, 0.0, 1.0]);

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) {
    let before = serde_json::to_value(s.font().unwrap()).unwrap();
    assert!(s.execute(id, &p).is_err(), "{id} {p} should be refused");
    assert_eq!(serde_json::to_value(s.font().unwrap()).unwrap(), before, "{id} {p} changed the font");
}

/// A 100 by 200 rectangle with its corner at the origin.
fn rectangle() -> Value {
    json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 200}, {"x": 0, "y": 200}])
}

/// `A` with a full-colour layer `color` holding two rectangles, and a rectangle on its master
/// layer.
fn colour_session() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rectangle()}));
    s.edit("Add Color Layer", |font| {
        let mut g = font.glyph_mut("A")?;
        let mut l = Layer::new("m01", 600.0);
        l.id = "color".into();
        l.info.attrs.full_color = true;
        g.layers.push(l);
        Ok(())
    })
    .unwrap();
    for _ in 0..2 {
        run(&mut s, "path.addPath", json!({"glyph": "A", "layer": "color", "nodes": rectangle()}));
    }
    s
}

fn attrs_on(s: &Session, layer: &str, shape: usize) -> ShapeAttrs {
    let glyph = s.font().unwrap().glyph("A").unwrap();
    match &glyph.layers.iter().find(|l| l.id == layer).unwrap().shapes[shape] {
        Shape::Path(p) => p.info.attrs.clone(),
        other => panic!("not a path: {other:?}"),
    }
}

fn attrs(s: &Session, shape: usize) -> ShapeAttrs {
    attrs_on(s, "color", shape)
}

fn on(shapes: &[usize], extra: Value) -> Value {
    let mut p = json!({"glyph": "A", "layer": "color", "shapes": shapes});
    if let (Value::Object(m), Value::Object(e)) = (&mut p, extra) {
        m.extend(e);
    }
    p
}

fn stops(s: &Session, shape: usize) -> Vec<(f64, Color)> {
    attrs(s, shape).gradient.unwrap().stops.iter().map(|st| (st.offset, st.color)).collect()
}

#[test]
fn path_attribute_commands_are_refused_on_a_master_layer() {
    let mut s = colour_session();
    refused(&mut s, "attributes.setFillColor", json!({"glyph": "A", "shapes": [0], "color": "#FF0000"}));
    refused(&mut s, "attributes.setStrokeWidth", json!({"glyph": "A", "layer": "m01", "width": 10}));
    refused(&mut s, "attributes.addGradient", json!({"glyph": "A", "layer": "m01"}));
    refused(&mut s, "attributes.setFillColor", on(&[7], json!({"color": "#FF0000"})));
    assert_eq!(attrs_on(&s, "m01", 0).fill_color, None, "a path on a master layer gets no fill");
}

#[test]
fn a_new_path_on_a_full_colour_layer_is_filled_gray() {
    let s = colour_session();
    assert_eq!(attrs(&s, 0).fill_color, Some(GRAY));
    assert_eq!(attrs(&s, 1).fill_color, Some(GRAY));
}

#[test]
fn set_stroke_width_gives_a_black_stroke_and_null_is_zero() {
    let mut s = colour_session();
    let r = run(&mut s, "attributes.setStrokeWidth", on(&[0], json!({"width": 12})));
    assert_eq!(r["attributes"][0]["stroke"]["color"], "#000000FF");
    assert_eq!(r["attributes"][0]["stroke"]["width"], 12.0);
    let a = attrs(&s, 0);
    assert_eq!((a.stroke_width, a.stroke_color), (Some(Dimension::Number(12.0)), Some(Color::Rgba([0.0, 0.0, 0.0, 1.0]))));
    assert_eq!(attrs(&s, 1).stroke_width, None, "only the listed path changes");

    run(&mut s, "attributes.setStrokeWidth", on(&[0], json!({"width": null})));
    assert_eq!(attrs(&s, 0).stroke_width, Some(Dimension::Number(0.0)));
    refused(&mut s, "attributes.setStrokeWidth", on(&[0], json!({"width": -1})));
    refused(&mut s, "attributes.setStrokeWidth", on(&[0], json!({})));
}

#[test]
fn set_stroke_alignment_stores_inside_and_outside_as_the_stroke_position() {
    let mut s = colour_session();
    // A document that is not from a `.glyphs` file follows version 3: 1 is inside, 0 outside.
    let r = run(&mut s, "attributes.setStrokeAlignment", on(&[0], json!({"alignment": "inside"})));
    assert_eq!(r["attributes"][0]["stroke"]["alignment"], "inside");
    assert_eq!(attrs(&s, 0).stroke_pos, Some(Dimension::Number(1.0)));
    run(&mut s, "attributes.setStrokeAlignment", on(&[0], json!({"alignment": "outside"})));
    assert_eq!(attrs(&s, 0).stroke_pos, Some(Dimension::Number(0.0)));
    run(&mut s, "attributes.setStrokeAlignment", on(&[0], json!({"alignment": "center"})));
    assert_eq!(attrs(&s, 0).stroke_pos, None);
    refused(&mut s, "attributes.setStrokeAlignment", on(&[0], json!({"alignment": "left"})));
}

#[test]
fn set_stroke_color_and_remove_stroke() {
    let mut s = colour_session();
    run(&mut s, "attributes.setStrokeWidth", on(&[0], json!({"width": 20})));
    run(&mut s, "attributes.setStrokeColor", on(&[0], json!({"color": "#ff0000"})));
    assert_eq!(attrs(&s, 0).stroke_color, Some(RED));
    let r = run(&mut s, "attributes.removeStroke", on(&[0], json!({})));
    assert_eq!(r["attributes"][0]["stroke"]["transparent"], true);
    let a = attrs(&s, 0);
    assert_eq!((a.stroke_width, a.stroke_color), (Some(Dimension::Number(0.0)), Some(Color::Rgba([0.0; 4]))));
}

#[test]
fn set_fill_color_reads_hex_colours_and_refuses_other_forms() {
    let mut s = colour_session();
    let r = run(&mut s, "attributes.setFillColor", json!({"glyph": "A", "layer": "color", "color": "#FF000080"}));
    assert_eq!(r["shapes"], json!([0, 1]), "without shapes every path of the layer changes");
    assert_eq!(r["attributes"][1]["fill"], "#FF000080");
    let half_red = Color::Rgba([1.0, 0.0, 0.0, 128.0 / 255.0]);
    assert_eq!((attrs(&s, 0).fill_color, attrs(&s, 1).fill_color), (Some(half_red), Some(half_red)));
    for bad in [json!("#F00"), json!("red"), json!("#GG0000"), json!("#FF0000FF00"), json!(12), json!([1, 0, 0, 1])] {
        refused(&mut s, "attributes.setFillColor", on(&[0], json!({"color": bad})));
    }
    refused(&mut s, "attributes.setFillColor", on(&[0], json!({})));
}

#[test]
fn remove_fill_makes_the_fill_transparent() {
    let mut s = colour_session();
    let r = run(&mut s, "attributes.removeFill", on(&[1], json!({})));
    assert_eq!(r["attributes"][0]["fillTransparent"], true);
    assert_eq!(attrs(&s, 1).fill_color, Some(Color::Rgba([0.0; 4])));
    assert_eq!(attrs(&s, 0).fill_color, Some(GRAY));
}

#[test]
fn add_gradient_replaces_the_fill_until_remove_gradient_restores_it() {
    let mut s = colour_session();
    run(&mut s, "attributes.setFillColor", on(&[0], json!({"color": "#FF0000"})));
    let r = run(&mut s, "attributes.addGradient", on(&[0], json!({})));
    assert_eq!(r["attributes"][0]["gradient"]["type"], "linear");
    let g = attrs(&s, 0).gradient.unwrap();
    assert_eq!(g.kind, GradientKind::Linear { start: Point::new(0.0, 0.5), end: Point::new(1.0, 0.5) });
    assert_eq!(g.extend, Extend::Pad);
    assert_eq!(stops(&s, 0), vec![(0.0, RED), (1.0, Color::Rgba([1.0, 0.0, 0.0, 0.0]))]);
    assert_eq!(attrs(&s, 0).fill_color, Some(RED), "the fill is kept under the gradient");

    refused(&mut s, "attributes.setFillColor", on(&[0], json!({"color": "#00FF00"})));
    refused(&mut s, "attributes.removeFill", on(&[0], json!({})));

    run(&mut s, "attributes.removeGradient", on(&[0], json!({})));
    assert_eq!(attrs(&s, 0).gradient, None);
    assert_eq!(attrs(&s, 0).fill_color, Some(RED));
    refused(&mut s, "attributes.removeGradient", on(&[0, 1], json!({})));
}

#[test]
fn add_gradient_refuses_fewer_than_two_stops_offsets_outside_0_1_and_bad_colours() {
    let mut s = colour_session();
    refused(&mut s, "attributes.addGradient", on(&[0], json!({"stops": [{"offset": 0, "color": "#000000"}]})));
    refused(&mut s, "attributes.addGradient", on(&[0], json!({"stops": [{"offset": 0, "color": "#000000"}, {"offset": 1.5, "color": "#FFFFFF"}]})));
    refused(&mut s, "attributes.addGradient", on(&[0], json!({"stops": [{"offset": 0, "color": "blue"}, {"offset": 1, "color": "#FFFFFF"}]})));
    refused(&mut s, "attributes.addGradient", on(&[0], json!({"type": "diamond"})));
    refused(&mut s, "attributes.addGradient", on(&[0], json!({"extend": "mirror"})));

    let stops_given = json!([{"offset": 1, "color": "#FFFFFF"}, {"offset": 0, "color": "#000000"}, {"offset": 0.5, "color": "#FF0000"}]);
    run(&mut s, "attributes.addGradient", on(&[0], json!({"type": "conic", "extend": "reflect", "stops": stops_given})));
    let g = attrs(&s, 0).gradient.unwrap();
    assert!(matches!(g.kind, GradientKind::Conic { .. }));
    assert_eq!(g.extend, Extend::Reflect);
    let offsets: Vec<f64> = g.stops.iter().map(|st| st.offset).collect();
    assert_eq!(offsets, vec![0.0, 0.5, 1.0], "stops are kept in offset order");
}

#[test]
fn set_gradient_type_places_the_new_type_and_keeps_the_stops() {
    let mut s = colour_session();
    run(&mut s, "attributes.addGradient", on(&[0], json!({})));
    let before = stops(&s, 0);
    run(&mut s, "attributes.setGradientType", on(&[0], json!({"type": "radial"})));
    let g = attrs(&s, 0).gradient.unwrap();
    let centre = Point::new(0.5, 0.5);
    assert_eq!(g.kind, GradientKind::Radial { start: centre, end: centre, start_radius: 0.0, end_radius: 0.5, angle: 0.0 });
    assert_eq!(stops(&s, 0), before);
    run(&mut s, "attributes.setGradientType", on(&[0], json!({"extend": "repeat"})));
    let g = attrs(&s, 0).gradient.unwrap();
    assert!(matches!(g.kind, GradientKind::Radial { .. }));
    assert_eq!(g.extend, Extend::Repeat);
    refused(&mut s, "attributes.setGradientType", on(&[1], json!({"type": "linear"})));
    refused(&mut s, "attributes.setGradientType", on(&[0], json!({})));
}

#[test]
fn add_gradient_stop_takes_the_gradient_colour_at_its_offset() {
    let mut s = colour_session();
    run(&mut s, "attributes.addGradient", on(&[0], json!({"stops": [{"offset": 0, "color": "#000000"}, {"offset": 1, "color": "#FFFFFF"}]})));
    let r = run(&mut s, "attributes.addGradientStop", on(&[0], json!({"offset": 0.5})));
    assert_eq!(r["stops"], json!([1]));
    assert_eq!(stops(&s, 0)[1], (0.5, Color::Rgba([0.5, 0.5, 0.5, 1.0])));
    let r = run(&mut s, "attributes.addGradientStop", on(&[0], json!({"offset": 0.25, "color": "#FF0000"})));
    assert_eq!(r["stops"], json!([1]));
    assert_eq!(stops(&s, 0).len(), 4);
    refused(&mut s, "attributes.addGradientStop", on(&[0], json!({"offset": 2})));
    refused(&mut s, "attributes.addGradientStop", on(&[0], json!({})));
    refused(&mut s, "attributes.addGradientStop", on(&[1], json!({"offset": 0.5})));
}

#[test]
fn remove_gradient_stop_keeps_at_least_two_stops() {
    let mut s = colour_session();
    run(&mut s, "attributes.addGradient", on(&[0], json!({})));
    run(&mut s, "attributes.addGradientStop", on(&[0], json!({"offset": 0.5, "color": "#FF0000"})));
    run(&mut s, "attributes.removeGradientStop", on(&[0], json!({"stop": 1})));
    assert_eq!(stops(&s, 0).iter().map(|st| st.0).collect::<Vec<_>>(), vec![0.0, 1.0]);
    refused(&mut s, "attributes.removeGradientStop", on(&[0], json!({"stop": 0})));
    refused(&mut s, "attributes.removeGradientStop", on(&[0], json!({"stop": 5})));
}

#[test]
fn set_gradient_stop_color_recolours_and_moves_a_stop() {
    let mut s = colour_session();
    let three = json!([{"offset": 0, "color": "#000000"}, {"offset": 0.5, "color": "#00FF00"}, {"offset": 1, "color": "#FFFFFF"}]);
    run(&mut s, "attributes.addGradient", on(&[0], json!({"stops": three})));
    let r = run(&mut s, "attributes.setGradientStopColor", on(&[0], json!({"stop": 0, "color": "#FF0000"})));
    assert_eq!(r["stops"], json!([0]));
    assert_eq!(stops(&s, 0)[0], (0.0, RED));
    let r = run(&mut s, "attributes.setGradientStopColor", on(&[0], json!({"stop": 0, "offset": 0.75})));
    assert_eq!(r["stops"], json!([1]), "the moved stop sits between the green and the white stops");
    assert_eq!(stops(&s, 0).iter().map(|st| st.0).collect::<Vec<_>>(), vec![0.5, 0.75, 1.0]);
    assert_eq!(stops(&s, 0)[1].1, RED);
    refused(&mut s, "attributes.setGradientStopColor", on(&[0], json!({"stop": 0})));
    refused(&mut s, "attributes.setGradientStopColor", on(&[0], json!({"stop": 3, "color": "#000000"})));
    refused(&mut s, "attributes.setGradientStopColor", on(&[0], json!({"stop": 0, "offset": -0.1})));
}

#[test]
fn set_gradient_position_converts_font_units_to_fractions_of_the_bounds() {
    let mut s = colour_session();
    run(&mut s, "attributes.addGradient", on(&[0], json!({})));
    run(&mut s, "attributes.setGradientPosition", on(&[0], json!({"start": [25, 100], "end": [75, 200]})));
    let kind = |s: &Session| attrs(s, 0).gradient.unwrap().kind;
    assert_eq!(kind(&s), GradientKind::Linear { start: Point::new(0.25, 0.5), end: Point::new(0.75, 1.0) });
    run(&mut s, "attributes.setGradientPosition", on(&[0], json!({"start": [50, 0]})));
    assert_eq!(
        kind(&s),
        GradientKind::Linear { start: Point::new(0.5, 0.0), end: Point::new(1.0, 0.5) },
        "without `end` the end moves with the start"
    );
    run(&mut s, "attributes.setGradientPosition", on(&[0], json!({"start": [0.1, 0.2], "end": [0.9, 0.2], "relative": true})));
    assert_eq!(kind(&s), GradientKind::Linear { start: Point::new(0.1, 0.2), end: Point::new(0.9, 0.2) });

    run(&mut s, "attributes.setGradientType", on(&[0], json!({"type": "conic"})));
    refused(&mut s, "attributes.setGradientPosition", on(&[0], json!({"start": [10, 10], "end": [20, 20]})));
    refused(&mut s, "attributes.setGradientPosition", on(&[0], json!({"start": [10]})));
    refused(&mut s, "attributes.setGradientPosition", on(&[1], json!({"start": [10, 10]})));
}

#[test]
fn add_attribute_stores_shadows_a_pattern_image_and_gradients() {
    let mut s = colour_session();
    run(&mut s, "attributes.addAttribute", on(&[0], json!({"attribute": "shadow"})));
    assert_eq!(attrs(&s, 0).shadow, Some(Shadow { blur: 10.0, color: Color::Rgba([0.0, 0.0, 0.0, 0.5]), offset: [10.0, -10.0] }));
    run(&mut s, "attributes.addAttribute", on(&[0], json!({"attribute": "innerShadow", "blur": 3, "offset": [1, 2], "color": "#FF0000FF"})));
    assert_eq!(attrs(&s, 0).shadow_in, Some(Shadow { blur: 3.0, color: RED, offset: [1.0, 2.0] }));
    let r = run(&mut s, "attributes.addAttribute", on(&[0], json!({"attribute": "patternImage", "image": "pattern.png"})));
    assert_eq!(r["attributes"][0]["patternImage"], "pattern.png");
    run(&mut s, "attributes.addAttribute", on(&[1], json!({"attribute": "gradient", "type": "radial"})));
    assert!(matches!(attrs(&s, 1).gradient.unwrap().kind, GradientKind::Radial { .. }));
    refused(&mut s, "attributes.addAttribute", on(&[0], json!({"attribute": "glow"})));
    refused(&mut s, "attributes.addAttribute", on(&[0], json!({"attribute": "shadow", "blur": -1})));
}

#[test]
fn attribute_edits_undo_in_one_step() {
    let mut s = colour_session();
    run(&mut s, "attributes.setFillColor", json!({"glyph": "A", "layer": "color", "color": "#FF0000"}));
    assert_eq!((attrs(&s, 0).fill_color, attrs(&s, 1).fill_color), (Some(RED), Some(RED)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((attrs(&s, 0).fill_color, attrs(&s, 1).fill_color), (Some(GRAY), Some(GRAY)));
}
