use super::*;

const V3: &str = "{
.appVersion = \"3343\";
.formatVersion = 3;
axes = (
{
name = Weight;
tag = wght;
}
);
familyName = \"Test Sans\";
fontMaster = (
{
axesValues = (
400
);
id = m01;
metricValues = (
{
over = 16;
pos = 800;
},
{
}
);
name = Regular;
}
);
glyphs = (
{
glyphname = A;
lastChange = \"2024-01-01 00:00:00 +0000\";
layers = (
{
anchors = (
{
name = top;
pos = (300,700);
}
);
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(500,0,l),
(300,700,l),
(100,0,l)
);
},
{
pos = (10,0);
ref = B;
}
);
width = 600;
},
{
associatedMasterId = m01;
layerId = \"F1A2-backup\";
name = \"Mar 1\";
width = 600;
}
);
unicode = 65;
},
{
export = 0;
glyphname = B;
layers = (
{
layerId = m01;
width = 500.5;
}
);
unicode = (66,67);
}
);
metrics = (
{
type = ascender;
},
{
type = baseline;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
";

#[test]
fn a_version_3_file_round_trips_through_the_model() {
    let font = read(V3).unwrap();
    assert_eq!(font.family_name, "Test Sans");
    assert_eq!(font.source.glyphs_version, Some(3));
    let m = font.default_master().unwrap();
    assert_eq!(font.metric_pos(m, fontcraft_model::metric::ASCENDER, 0.0), 800.0);
    let a = font.glyph("A").unwrap();
    assert_eq!(a.unicodes, [65]);
    assert_eq!(a.layers.len(), 2);
    assert!(!a.layers[1].is_master_layer());
    let Shape::Path(p) = &a.layers[0].shapes[0] else { panic!() };
    // The file's last node is the start node.
    assert_eq!((p.nodes[0].x, p.nodes[0].y), (100.0, 0.0));
    assert!(a.extra.get(Family::Glyphs, "lastChange").is_some());
    let b = font.glyph("B").unwrap();
    assert_eq!((b.unicodes.clone(), b.export), (vec![66, 67], false));
    assert_eq!(write(&font).unwrap(), V3);
}

#[test]
fn edits_change_only_their_own_lines() {
    let mut font = read(V3).unwrap();
    font.glyph_mut("B").unwrap().layers[0].width = 510.0;
    let out = write(&font).unwrap();
    let changed: Vec<(&str, &str)> = V3.lines().zip(out.lines()).filter(|(a, b)| a != b).collect();
    assert_eq!(changed, [("width = 500.5;", "width = 510;")]);
}

#[test]
fn version_4_keeps_the_family_name_in_its_properties() {
    let v4 = "{\n.appVersion = \"4004\";\n.formatVersion = 4;\nfontMaster = (\n{\nid = m01;\n},\n);\nglyphs = (\n);\nproperties = (\n{\nkey = familyNames;\nvalues = (\n{\nlanguage = dflt;\nvalue = Markow;\n},\n);\n},\n);\nunitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}\n";
    let mut font = read(v4).unwrap();
    assert_eq!(font.family_name, "Markow");
    assert_eq!(write(&font).unwrap(), v4);
    font.family_name = "Markow Two".into();
    let out = write(&font).unwrap();
    assert!(out.contains("value = \"Markow Two\";"), "{out}");
    assert_eq!(read(&out).unwrap().family_name, "Markow Two");
}

#[test]
fn shapes_the_model_cannot_hold_are_kept_as_written() {
    let text = V3.replace("(500,0,l),", "(500,0,u),");
    let font = read(&text).unwrap();
    assert!(matches!(font.glyph("A").unwrap().layers[0].shapes[0], Shape::Raw(_)));
    assert_eq!(write(&font).unwrap(), text);
    // A node list that makes no outline is kept too, not refused.
    let text = V3.replace("(500,0,l),\n(300,700,l),", "(500,0,o),\n(300,700,l),");
    let font = read(&text).unwrap();
    assert!(matches!(font.glyph("A").unwrap().layers[0].shapes[0], Shape::Raw(_)));
    assert_eq!(write(&font).unwrap(), text);
}

#[test]
fn node_flags_and_attributes_survive() {
    let text = V3.replace("(300,700,l),", "(300,700,lsX,{name = apex;}),");
    let font = read(&text).unwrap();
    let Shape::Path(p) = &font.glyph("A").unwrap().layers[0].shapes[0] else { panic!() };
    assert!(p.nodes.iter().any(|n| n.smooth));
    assert_eq!(write(&font).unwrap(), text);
}

#[test]
fn matrices_without_shear_decompose() {
    assert_eq!(decompose([2.0, 0.0, 0.0, 3.0, 5.0, 6.0]), Some(((5.0, 6.0), (2.0, 3.0), 0.0, (0.0, 0.0))));
    let ((_, _), (sx, sy), angle, _) = decompose([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]).unwrap();
    assert!((sx - 1.0).abs() < 1e-12 && (sy - 1.0).abs() < 1e-12 && (angle - 90.0).abs() < 1e-9);
    assert_eq!(decompose([1.0, 0.0, 0.5, 1.0, 0.0, 0.0]).map(|_| ()), None, "a shear is not a rotation and a scale");
}

#[test]
fn malformed_files_are_errors_not_panics() {
    for bad in ["(", "()", "{\n.formatVersion = 3;\n}", "{\n.formatVersion = 9;\nunitsPerEm = 1000;\n}", "{\nunitsPerEm = 1000;\n}"] {
        assert!(read(bad).is_err(), "{bad:?}");
    }
    for i in 0..V3.len() {
        if let Some(prefix) = V3.get(..i) {
            let _ = read(prefix);
        }
    }
}
