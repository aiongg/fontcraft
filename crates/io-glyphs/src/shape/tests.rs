use super::*;
use fontcraft_model::{Layer, MAX_PART_PROPERTIES};
use fontcraft_plist::openstep;

fn value(text: &str) -> Value {
    openstep::parse(text).unwrap().root
}

fn read(text: &str, version: u32) -> Layer {
    if version == 2 { crate::v2::read_layer(value(text), "test").unwrap() } else { crate::read_layer(value(text), "test", version).unwrap() }
}

fn write(l: &Layer, version: u32) -> String {
    let v = if version == 2 { crate::v2::write_layer(l, "test").unwrap() } else { crate::write_layer(l, "test", version).unwrap() };
    let mut out = String::new();
    openstep::write_value(&mut out, &v, crate::layer::dialect(version));
    out
}

fn round_trip(text: &str, version: u32) -> Layer {
    let l = read(text, version);
    assert_eq!(write(&l, version), text, "version {version} writes back the same");
    l
}

fn path(l: &Layer, i: usize) -> &Path {
    match l.shapes.get(i) {
        Some(Shape::Path(p)) => p,
        other => panic!("shape {i} is not a path: {other:?}"),
    }
}

fn component(l: &Layer, i: usize) -> &Component {
    match l.shapes.get(i) {
        Some(Shape::Component(c)) => c,
        other => panic!("shape {i} is not a component: {other:?}"),
    }
}

fn with_shapes(shapes: &str) -> String {
    format!("{{\nlayerId = m01;\nshapes = (\n{shapes}\n);\nwidth = 600;\n}}")
}

/// The same in version 4, which ends multi-line lists with a comma.
fn with_shapes4(shapes: &str) -> String {
    format!("{{\nlayerId = m01;\nshapes = (\n{shapes},\n);\nwidth = 600;\n}}")
}

const V3_PATH: &str = "{
attr = {
fillColor = (255,0,0,255);
gradient = {
colors = (
((255,0,0,255),0),
((0,0,255,255),1)
);
end = (0.9,0.9);
start = (0.1,0.1);
};
lineCapEnd = \"3\";
lineCapStart = 3;
strokePos = \"1\";
strokeWidth = 46;
};
closed = 1;
locked = 1;
nodes = (
(0,0,lsR),
(100,0,lCX),
(120,50,l,{
name = hr00;
userData = {
k = v;
};
}),
(10,100,l,{
name = \"Hallo Welt\";
test = 1;
})
);
}";

#[test]
fn version_3_paths_and_nodes_are_fields() {
    let l = round_trip(&with_shapes(V3_PATH), 3);
    let p = path(&l, 0);
    assert!(p.info.locked);
    let a = &p.info.attrs;
    assert_eq!(a.stroke_width, Some(Dimension::Number(46.0)));
    assert_eq!(a.stroke_pos, Some(Dimension::Expression("1".into())), "text stays text");
    assert_eq!(a.line_cap_start, Some(3));
    assert_eq!(a.line_cap_end, None, "a number written as text stays as written");
    assert!(a.extra.get(Family::Glyphs, "lineCapEnd").is_some() && a.extra.get(Family::Glyphs, "gradient").is_some());
    assert_eq!(a.fill_color, Some(fontcraft_model::Color::Rgba([1.0, 0.0, 0.0, 1.0])));
    // The format writes the start node last; the model keeps it first.
    let [last, first, second, third] = p.nodes.as_slice() else { panic!() };
    assert_eq!((first.x, first.smooth, first.orientation), (0.0, true, Orientation::Right));
    assert_eq!((second.orientation, second.locked, second.smooth), (Orientation::Center, true, false));
    assert_eq!(third.info.name.as_deref(), Some("hr00"));
    assert_eq!(third.info.user_data.get("k").and_then(Value::as_str), Some("v"));
    assert_eq!(last.info.name.as_deref(), Some("Hallo Welt"));
    assert!(last.info.extra.get(Family::Glyphs, "attr").is_some(), "an unknown node attribute stays");
}

#[test]
fn node_edits_write_the_new_values() {
    let mut l = read(&with_shapes(V3_PATH), 3);
    let Some(Shape::Path(p)) = l.shapes.get_mut(0) else { panic!() };
    // The model's order: the start node (written last) first.
    let n = &mut p.nodes[2];
    (n.locked, n.orientation, n.smooth) = (false, Orientation::Left, true);
    p.nodes[3].info.name = Some("renamed".into());
    p.nodes[0].info.name = None;
    p.info.locked = false;
    let out = write(&l, 3);
    assert!(out.contains("(100,0,ls),\n(120,50,l,{\nname = renamed;\nuserData"), "{out}");
    assert!(out.contains("(10,100,l,{\ntest = 1;\n})"), "the other attribute stays without the name: {out}");
    assert!(!out.contains("locked = 1;"));
}

#[test]
fn tangent_nodes_are_a_version_4_join() {
    let nodes = "{\nclosed = 0;\nnodes = (\n(0,0,l),\n(50,0,lt),\n(80,0,o),\n(100,20,o),\n(100,50,c)NL);\n}";
    let text = with_shapes(&nodes.replace("NL", "\n"));
    let l = round_trip(&with_shapes4(&nodes.replace("NL", ",\n")), 4);
    let n = &path(&l, 0).nodes[1];
    assert!(n.smooth && n.tangent);
    // Version 3 has no tangent join: the flag stays as written there.
    let l3 = round_trip(&text, 3);
    let n = &path(&l3, 0).nodes[1];
    assert!(!n.tangent && !n.smooth && n.info.extra.get(Family::Glyphs, "flags").is_some());
    // A tangent node of a version 4 document saved as version 3 is smooth.
    assert!(write(&l, 3).contains("(50,0,ls)"));
}

#[test]
fn attributes_that_would_change_stay_whole() {
    // Keys out of order (the writer sorts them) and an empty attribute dictionary.
    let text = with_shapes("{\nattr = {\nstrokeWidth = 20;\nfill = 1;\n};\nclosed = 1;\nnodes = (\n(0,0,l),\n(10,0,l),\n(5,9,l,{\n})\n);\n}");
    let l = round_trip(&text, 3);
    let p = path(&l, 0);
    assert!(p.info.attrs.is_empty() && p.extra.get(Family::Glyphs, "attr").is_some());
    assert!(p.nodes[0].info.name.is_none() && p.nodes[0].info.extra.get(Family::Glyphs, "attr").is_some());
}

const V3_COMPONENTS: &str = "{
alignment = -1;
anchor = top_2;
attr = {
reversePaths = 1;
};
locked = 1;
masterId = m02;
orientation = 1;
piece = {
Width = 29.74825;
height = 0;
};
pos = (10,0);
ref = A;
userData = {
note = x;
};
},
{
alignment = 0;
ref = B;
}";

#[test]
fn version_3_components_are_fields() {
    let l = round_trip(&with_shapes(V3_COMPONENTS), 3);
    let c = component(&l, 0);
    let i = &*c.info;
    assert_eq!((i.alignment, i.anchor.as_deref(), i.master_id.as_deref()), (Alignment::Disabled, Some("top_2"), Some("m02")));
    assert!(i.locked && i.attrs.reverse_paths);
    assert_eq!(i.orientation, Orientation::Right);
    assert_eq!(i.smart_values.iter().map(|v| (v.property.as_str(), v.value)).collect::<Vec<_>>(), [("Width", 29.74825), ("height", 0.0)]);
    assert_eq!(i.user_data.get("note").and_then(Value::as_str), Some("x"));
    // `alignment = 0` is the default: it stays in `Extra`, and is written while the field says
    // the same.
    let b = component(&l, 1);
    assert_eq!(b.info.alignment, Alignment::Default);
    assert!(b.extra.get(Family::Glyphs, "alignment").is_some());
    let mut edited = l.clone();
    let Some(Shape::Component(b)) = edited.shapes.get_mut(1) else { panic!() };
    b.info.alignment = Alignment::Forced;
    b.info.smart_values = vec![SmartValue { property: "Wide".into(), value: 100.0 }];
    let out = write(&edited, 3);
    assert!(out.contains("alignment = 1;\npiece = {\nWide = 100;\n};\nref = B;"), "{out}");
}

#[test]
fn version_4_keeps_user_data_in_attr() {
    let text = with_shapes4("{\nattr = {\nuserData = {\nk = 1;\n};\n};\nref = A;\n}");
    let l = round_trip(&text, 4);
    assert_eq!(component(&l, 0).info.user_data.get("k").and_then(Value::as_num).map(Num::as_f64), Some(1.0));
    let anchors = "{\nanchors = (\n{\nattr = {\nuserData = {\nk = 1;\n};\n};\nlocked = 1;\nname = top;\norientation = center;\npos = (1,2);\n},\n);\nlayerId = m01;\nwidth = 600;\n}";
    let l = round_trip(anchors, 4);
    let a = &l.anchors[0];
    assert!(a.info.locked);
    assert_eq!(a.info.orientation, Orientation::Center);
    assert_eq!(a.info.user_data.0.len(), 1);
    // Version 3 has `userData` of its own; `attr` stays as written.
    let v3 = "{\nanchors = (\n{\nlocked = 1;\nname = top;\norientation = right;\nuserData = {\nk = 1;\n};\n}\n);\nlayerId = m01;\nwidth = 600;\n}";
    let l = round_trip(v3, 3);
    assert_eq!((l.anchors[0].info.orientation, l.anchors[0].info.user_data.0.len()), (Orientation::Right, 1));
}

#[test]
fn version_4_images_are_shapes() {
    let text = with_shapes4(
        "{\nalpha = 70;\nangle = 3;\ncrop = (0,0,140,160);\nimagePath = \"files/a.png\";\nlocked = 1;\npos = (10,20);\n},\n{\nattr = {\ngroup = g1;\n};\ngroupId = g2;\n}",
    );
    let l = round_trip(&text, 4);
    let Some(Shape::Image(i)) = l.shapes.first() else { panic!("{:?}", l.shapes) };
    assert_eq!((i.path.as_deref(), i.alpha, i.locked, i.crop), (Some("files/a.png"), Some(70.0), true, Some([0.0, 0.0, 140.0, 160.0])));
    assert!(matches!(l.shapes.get(1), Some(Shape::Raw(_))), "a shape group stays a stored value");
    let mut edited = l.clone();
    if let Some(Shape::Image(i)) = edited.shapes.get_mut(0) {
        i.alpha = Some(40.0);
    }
    assert!(write(&edited, 4).contains("alpha = 40;"));
    assert!(crate::v2::write_layer(&l, "test").is_err() && crate::write_layer(&l, "test", 3).is_err(), "earlier versions have no image shapes");
}

#[test]
fn smart_values_beyond_the_limit_stay_as_written() {
    let piece: String = (0..=MAX_PART_PROPERTIES).map(|i| format!("p{i} = {i};\n")).collect();
    let text = with_shapes(&format!("{{\npiece = {{\n{piece}}};\nref = A;\n}}"));
    let l = round_trip(&text, 3);
    let c = component(&l, 0);
    assert!(c.info.smart_values.is_empty() && c.extra.get(Family::Glyphs, "piece").is_some());
}

const V2_LAYER: &str = "{
anchors = (
{
locked = 1;
name = top;
position = \"{250, 700}\";
}
);
components = (
{
alignment = -1;
anchor = cedilla;
locked = 1;
name = A;
piece = {
Width = 413;
};
transform = \"{1, 0, 0, 1, 10, 0}\";
}
);
layerId = m01;
paths = (
{
closed = 1;
nodes = (
\"131 250 LINE {name = hr00;}\",
\"470 250 LINE SMOOTH\",
\"470 330 LINE SMOOTH {name = \\\"Hello World\\\";\\012userData = {\\012k = v;\\012};}\",
\"131 330 LINE\"
);
}
);
width = 600;
}";

#[test]
fn version_2_node_strings_and_components_are_fields() {
    let l = round_trip(V2_LAYER, 2);
    let p = path(&l, 0);
    assert_eq!(p.nodes[1].info.name.as_deref(), Some("hr00"));
    assert!(p.nodes[2].smooth && p.nodes[3].smooth);
    assert_eq!(p.nodes[3].info.name.as_deref(), Some("Hello World"));
    assert_eq!(p.nodes[3].info.user_data.get("k").and_then(Value::as_str), Some("v"));
    let c = component(&l, 1);
    assert_eq!((c.info.alignment, c.info.anchor.as_deref(), c.info.locked), (Alignment::Disabled, Some("cedilla"), true));
    assert_eq!(c.info.smart_values.first().map(|v| v.value), Some(413.0));
    assert!(l.anchors[0].info.locked);
    // An edited name is written into the node string.
    let mut edited = l.clone();
    let Some(Shape::Path(p)) = edited.shapes.get_mut(0) else { panic!() };
    p.nodes[1].info.name = Some("hr01".into());
    p.nodes[2].info.name = Some("x".into());
    let out = write(&edited, 2);
    assert!(out.contains("\"131 250 LINE {name = hr01;}\"") && out.contains("\"470 250 LINE SMOOTH {name = x;}\""), "{out}");
}

#[test]
fn version_2_node_strings_written_otherwise_keep_the_path() {
    let escaped = "\"131 250 LINE {name = \\\"a\\\";\\nk = v;}\"";
    for odd in [
        "\"131 250 LINE  {name = hr00;}\"",
        "\"131 250 LINE {name = hr00; }\"",
        "\"131 250 LINE {name = hr00;} x\"",
        "\"131 250 LINE name\"",
        escaped,
    ] {
        let text = V2_LAYER.replace("\"131 250 LINE {name = hr00;}\"", odd);
        let l = round_trip(&text, 2);
        assert!(matches!(l.shapes.first(), Some(Shape::Raw(_))), "{odd}");
    }
}

#[test]
fn losses_name_what_a_version_cannot_hold() {
    let mut l = read(&with_shapes(V3_PATH), 3);
    let Some(Shape::Path(p)) = l.shapes.get_mut(0) else { panic!() };
    p.info.identifier = Some("c1".into());
    p.nodes[0].tangent = true;
    let g = std::sync::Arc::new(fontcraft_model::Glyph { layers: vec![l], ..fontcraft_model::Glyph::new("a") });
    let v3 = losses(std::slice::from_ref(&g), 3);
    assert!(v3.iter().any(|l| l.starts_with("the identifiers of paths and nodes (1)")) && v3.iter().any(|l| l.starts_with("tangent nodes")));
    let v2 = losses(std::slice::from_ref(&g), 2);
    assert!(v2.iter().any(|l| l.starts_with("node locks and orientations (2)")), "{v2:?}");
    assert!(!losses(&[g], 4).iter().any(|l| l.starts_with("tangent")));
}
