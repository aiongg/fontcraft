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
    assert_eq!((font.axes.len(), font.axes[0].tag.as_str(), m.location.clone()), (1, "wght", vec![400.0]));
    assert!(font.extra.get(Family::Glyphs, "axes").is_none(), "typed, not carried");
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

const V2: &str = "{
.appVersion = \"1192\";
familyName = Old;
fontMaster = (
{
alignmentZones = (
\"{810, 12}\",
\"{578, 11}\",
\"{0, -13}\",
\"{-190, -16}\"
);
ascender = 810;
capHeight = 810;
descender = -190;
id = \"3FB85DAF-C7EF\";
weightValue = 180;
xHeight = 578;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
anchors = (
{
name = top;
position = \"{400, 810}\";
}
);
layerId = \"3FB85DAF-C7EF\";
paths = (
{
closed = 1;
nodes = (
\"400 810 LINE\",
\"169 810 LINE SMOOTH\",
\"22 0 LINE\"
);
}
);
width = 568;
}
);
unicode = 0041;
},
{
glyphname = Aacute;
layers = (
{
components = (
{
name = A;
},
{
name = acutecomb;
transform = \"{1, 0, 0, 1, 285, 232}\";
}
);
layerId = \"3FB85DAF-C7EF\";
width = 568;
}
);
unicode = 00C1;
},
{
glyphname = uni2000;
layers = (
{
layerId = \"3FB85DAF-C7EF\";
width = 1000;
}
);
unicode = 2000;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
";

#[test]
fn a_version_2_file_round_trips_through_the_model() {
    let font = read(V2).unwrap();
    assert_eq!(font.source.glyphs_version, Some(2));
    let m = font.default_master().unwrap();
    let get = |k| font.metric(m, k).unwrap();
    assert_eq!((get(fontcraft_model::metric::ASCENDER).pos, get(fontcraft_model::metric::ASCENDER).over), (810.0, 12.0));
    assert_eq!(get(fontcraft_model::metric::CAP_HEIGHT).over, 0.0, "the one zone at 810 went to the ascender");
    assert_eq!(get(fontcraft_model::metric::BASELINE).over, -13.0);
    assert_eq!(font.glyph("Aacute").unwrap().unicodes, [0xC1]);
    assert_eq!(font.glyph("uni2000").unwrap().unicodes, [0x2000], "hexadecimal even when it reads as a number");
    let Shape::Component(c) = &font.glyph("Aacute").unwrap().layers[0].shapes[1] else { panic!() };
    assert_eq!(c.transform, Transform::Matrix([1.0, 0.0, 0.0, 1.0, 285.0, 232.0]));
    assert_eq!(write(&font).unwrap(), V2);
}

#[test]
fn version_2_spellings_that_are_not_canonical_are_kept_while_they_hold() {
    let text = V2.replace("{1, 0, 0, 1, 285, 232}", "{1.0, 0, 0, 1, 285, 232}").replace("{400, 810}", "{400.0, 810}");
    let mut font = read(&text).unwrap();
    assert_eq!(write(&font).unwrap(), text);
    // Once the value changes, the canonical spelling takes over.
    font.glyph_mut("A").unwrap().layers[0].anchors[0].x = 401.0;
    assert!(write(&font).unwrap().contains("position = \"{401, 810}\";"));
}

#[test]
fn kerning_and_kerning_groups_are_fields() {
    let text = V3
        .replace("glyphname = A;\n", "glyphname = A;\nkernLeft = A;\nkernRight = A;\n")
        .replace(
            "metrics = (",
            "kerningLTR = {\nm01 = {\n\"@MMK_L_A\" = {\n\"@MMK_R_V\" = -50;\nT = -30.5;\n};\n};\n};\nkerningRTL = {\nm01 = {\n\"@MMK_R_alef\" = {\n\"@MMK_L_beh\" = 12;\n};\n};\n};\nmetrics = (",
        );
    let mut font = read(&text).unwrap();
    assert_eq!(font.kerning.ltr.get("m01", "@MMK_L_A", "@MMK_R_V"), Some(-50.0));
    assert_eq!(font.kerning.ltr.get("m01", "@MMK_L_A", "T"), Some(-30.5));
    assert_eq!(font.kerning.rtl.pair_count(), 1);
    assert_eq!(fontcraft_model::side_group("@MMK_R_alef"), Some("alef"));
    let a = font.glyph("A").unwrap();
    assert_eq!((a.kern_groups.left.as_deref(), a.kern_groups.right.as_deref()), (Some("A"), Some("A")));
    assert!(font.extra.get(Family::Glyphs, "kerningLTR").is_none(), "typed, not carried");
    assert_eq!(write(&font).unwrap(), text);
    std::sync::Arc::make_mut(&mut font.kerning).ltr.set("m01", "@MMK_L_A", "T", -40.0);
    assert!(write(&font).unwrap().contains("T = -40;"));
}

#[test]
fn a_kerning_table_with_other_values_is_kept_as_written() {
    let text = V3.replace("metrics = (", "kerningLTR = {\nm01 = {\nA = {\nV = \"odd\";\n};\n};\n};\nmetrics = (");
    let font = read(&text).unwrap();
    assert!(font.kerning.ltr.is_empty());
    assert_eq!(write(&font).unwrap(), text);
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
