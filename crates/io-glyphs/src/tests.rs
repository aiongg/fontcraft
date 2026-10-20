use super::*;
use fontcraft_model::Transform;

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
    assert!(a.extra.get(Family::Glyphs, "lastChange").is_none(), "typed, not carried");
    assert!(a.info.last_change.is_some());
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
fn property_texts_with_quotes_they_do_not_need_are_read() {
    let v4 = "{\n.appVersion = \"4004\";\n.formatVersion = 4;\nfontMaster = (\n{\nid = m01;\n},\n);\nglyphs = (\n);\nproperties = (\n{\nkey = familyNames;\nvalues = (\n{\nlanguage = dflt;\nvalue = Markow;\n},\n{\nlanguage = DEU;\nvalue = \"Markow\";\n},\n);\n},\n{\nkey = vendorID;\nvalue = \"ABCD\";\n},\n);\nunitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}\n";
    let font = read(v4).unwrap();
    let PropertyValue::Localized(names) = &font.info.properties.get("familyNames").unwrap().value else { panic!() };
    assert_eq!(names[1].value, "Markow");
    assert_eq!(font.info.properties.value("vendorID"), Some("ABCD"));
    assert_eq!(write(&font).unwrap(), v4, "the quotes stay");
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

/// A version 4 font whose nodes carry higher-order interpolation data of each kind.
const V4_HOI: &str = "{
.appVersion = \"4004\";
.formatVersion = 4;
axes = (
{
name = Weight;
tag = wght;
},
);
fontMaster = (
{
axesValues = (
400,
);
id = m01;
},
{
axesValues = (
700,
);
id = m02;
},
);
glyphs = (
{
glyphname = A;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(500,0,l,{
hoi = {
wght = {
ip = (520,40.5);
};
};
}),
(300,700,l,{
hoi = {
wdth = {
rc = auto;
rd = ccw;
};
wght = {
rc = (300,350);
};
};
name = apex;
}),
(100,0,l,{
hoi = {
wght = {
al = 10;
tu = 0.25;
};
};
}),
);
},
);
width = 600;
},
{
layerId = m02;
shapes = (
{
closed = 1;
nodes = (
(540,0,l),
(300,720,l),
(60,0,l),
);
},
);
width = 640;
},
);
},
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
";

#[test]
fn version_4_interpolation_curves_are_typed_and_written_back_byte_for_byte() {
    use fontcraft_model::{HoiCenter, HoiCurve, HoiDirection};
    let mut font = read(V4_HOI).unwrap();
    let Shape::Path(p) = &font.glyph("A").unwrap().layers[0].shapes[0] else { panic!() };
    assert_eq!(p.nodes[0].info.hoi.get("wght"), Some(&HoiCurve::Point { x: 520.0, y: 40.5 }));
    let apex = &p.nodes[1].info;
    assert_eq!(apex.name.as_deref(), Some("apex"));
    assert_eq!(apex.hoi.get("wdth"), Some(&HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: Some(HoiDirection::CounterClockwise) }));
    assert_eq!(apex.hoi.get("wght"), Some(&HoiCurve::Rotation { center: Some(HoiCenter::At { x: 300.0, y: 350.0 }), direction: None }));
    assert!(matches!(p.nodes[2].info.hoi.get("wght"), Some(HoiCurve::Timing(_))));
    assert!(p.nodes.iter().all(|n| n.info.extra.get(Family::Glyphs, "attr").is_none()), "typed, not carried");
    assert_eq!(write(&font).unwrap(), V4_HOI);
    assert!(info_losses(&font).is_empty());
    // Version 3 has no place for the data: it is left out and reported.
    font.source.glyphs_version = Some(3);
    assert!(info_losses(&font).iter().any(|l| l == "the nodes' higher-order interpolation curves (3)"), "{:?}", info_losses(&font));
    let v3 = write(&font).unwrap();
    assert!(!v3.contains("hoi") && v3.contains("name = apex;"), "{v3}");
    // Data not of the documented shape stays as written.
    let odd = V4_HOI.replace("ip = (520,40.5);", "ip = (520,40.5);\nxx = 1;");
    let font = read(&odd).unwrap();
    let Shape::Path(p) = &font.glyph("A").unwrap().layers[0].shapes[0] else { panic!() };
    assert!(p.nodes[0].info.hoi.is_empty());
    assert_eq!(write(&font).unwrap(), odd);
}

#[test]
fn matrices_without_shear_decompose() {
    assert_eq!(decompose([2.0, 0.0, 0.0, 3.0, 5.0, 6.0]), Some(((5.0, 6.0), (2.0, 3.0), 0.0, (0.0, 0.0))));
    let ((_, _), (sx, sy), angle, _) = decompose([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]).unwrap();
    assert!((sx - 1.0).abs() < 1e-12 && (sy - 1.0).abs() < 1e-12 && (angle - 90.0).abs() < 1e-9);
}

#[test]
fn matrices_with_shear_decompose_into_a_slant() {
    use fontcraft_model::Transform;
    let back = |m: [f64; 6]| {
        let (pos, scale, angle, slant) = decompose(m).unwrap();
        let c = Transform::Decomposed { pos, scale, angle, slant }.affine().as_coeffs();
        assert!(c.iter().zip(&m).all(|(x, y)| (x - y).abs() < 1e-9), "{m:?} → {c:?}");
        (scale, angle, slant)
    };
    // A horizontal shear is a horizontal slant, without a turn.
    let (scale, angle, slant) = back([1.0, 0.0, 0.5, 1.0, 10.0, 0.0]);
    assert!(angle == 0.0 && slant.1 == 0.0 && (slant.0 - 0.5f64.atan().to_degrees()).abs() < 1e-9 && (scale.0 - 1.0).abs() < 1e-12);
    // A slanted, turned and scaled component, and a mirror with a shear.
    back([0.8, 0.3, 0.9, 1.4, 5.0, -7.0]);
    let (scale, angle, _) = back([-1.0, 0.0, 0.2, 1.0, 0.0, 0.0]);
    assert!(angle == 0.0 && scale.0 < 0.0, "a mirror is a negative scale: {scale:?} {angle}");
    // A steep vertical shear has no form without a vertical slant.
    let (_, _, slant) = back([1.0, 3.0, 0.0, 1.0, 0.0, 0.0]);
    assert!(slant.1 != 0.0);
    assert_eq!(decompose([1.0, 2.0, 2.0, 4.0, 0.0, 0.0]), None, "a singular matrix");
    assert_eq!(decompose([f64::NAN, 0.0, 0.5, 1.0, 0.0, 0.0]), None);
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
fn version_2_metrics_no_master_states_stay_absent() {
    use fontcraft_model::metric;
    // No master states the cap height or the x-height; a zone at 0 is the baseline's even though
    // the absent metrics would read as 0.
    let text = V2.replace("capHeight = 810;\n", "").replace("xHeight = 578;\n", "").replace("\"{578, 11}\",\n", "");
    let font = read(&text).unwrap();
    let m = font.default_master().unwrap();
    assert!(font.metric(m, metric::CAP_HEIGHT).is_none() && font.metric(m, metric::X_HEIGHT).is_none());
    assert_eq!(font.metric(m, metric::BASELINE).map(|v| v.over), Some(-13.0));
    let resolved = font.resolve(&font.default_target().unwrap()).unwrap();
    assert_eq!((resolved.metrics.cap_height, resolved.metrics.x_height), (700.0, 500.0), "the format's defaults");
    assert_eq!(write(&font).unwrap(), text, "the keys stay out");
}

#[test]
fn code_points_spelled_unusually_are_read_and_keep_their_spelling() {
    // Version 2: hexadecimal, here written as a bare number.
    let text = V2.replace("unicode = 0041;", "unicode = 41;");
    let mut font = read(&text).unwrap();
    assert_eq!(font.glyph("A").unwrap().unicodes, [0x41]);
    assert_eq!(write(&font).unwrap(), text);
    font.glyph_mut("A").unwrap().unicodes = vec![0x42];
    assert!(write(&font).unwrap().contains("unicode = 0042;"), "a changed value is written canonically");
    // Version 3: decimal, here with a leading zero, which reads as decimal too.
    let v3 = V3.replace("unicode = 65;", "unicode = 0065;");
    let font = read(&v3).unwrap();
    assert_eq!(font.glyph("A").unwrap().unicodes, [65]);
    assert_eq!(write(&font).unwrap(), v3);
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
fn a_version_3_file_with_the_version_2_kerning_key_reads_its_kerning() {
    let text = V3.replace("metrics = (", "kerning = {\nm01 = {\nA = {\nV = 4.2001;\n};\n};\n};\nmetrics = (");
    let mut font = read(&text).unwrap();
    assert_eq!(font.kerning.ltr.get("m01", "A", "V"), Some(4.2001));
    assert_eq!(write(&font).unwrap(), text);
    // A changed table is written under the version 3 key, which is read first.
    std::sync::Arc::make_mut(&mut font.kerning).ltr.set("m01", "A", "V", -40.0);
    let written = write(&font).unwrap();
    assert!(!written.contains("\nkerning = {"), "the stale version 2 table is dropped: {written}");
    let back = read(&written).unwrap();
    assert_eq!(back.kerning.ltr.get("m01", "A", "V"), Some(-40.0));
    // Deleting all the kerning and saving leaves no kerning to reopen.
    let mut font = read(&text).unwrap();
    std::sync::Arc::make_mut(&mut font.kerning).ltr = Default::default();
    let back = read(&write(&font).unwrap()).unwrap();
    assert!(back.kerning.ltr.is_empty(), "the deleted kerning came back");
}

/// A version 2 key the reader did not take the kerning from is written back as read: one whose
/// table does not parse, and one beside `kerningLTR`, also when the kerning changes.
#[test]
fn a_version_2_kerning_key_the_reader_ignored_is_kept() {
    let odd = V3.replace("metrics = (", "kerning = {\nm01 = {\nA = {\nV = \"odd\";\n};\n};\n};\nmetrics = (");
    let font = read(&odd).unwrap();
    assert!(font.kerning.ltr.is_empty());
    assert_eq!(write(&font).unwrap(), odd);
    let both =
        V3.replace("metrics = (", "kerning = {\nm01 = {\nA = {\nV = 7;\n};\n};\n};\nkerningLTR = {\nm01 = {\nA = {\nV = 4;\n};\n};\n};\nmetrics = (");
    let mut font = read(&both).unwrap();
    assert_eq!(font.kerning.ltr.get("m01", "A", "V"), Some(4.0));
    assert_eq!(write(&font).unwrap(), both);
    std::sync::Arc::make_mut(&mut font.kerning).ltr.set("m01", "A", "V", -40.0);
    assert!(write(&font).unwrap().contains("\nkerning = {"), "the key beside kerningLTR is kept");
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

/// [`V3`] with stems, numbers, master guides and two instances.
fn v3_design() -> String {
    V3.replace(
        "id = m01;\nmetricValues",
        "guides = (\n{\nangle = 90;\nlocked = 1;\nname = center;\npos = (300,0);\n},\n{\npos = (0,500);\n}\n);\nid = m01;\nmetricValues",
    )
    .replace("name = Regular;\n}\n);\nglyphs", "name = Regular;\nnumberValues = (\n40\n);\nstemValues = (\n80,\n60\n);\n}\n);\nglyphs")
    .replace(
        "metrics = (",
        "instances = (\n{\naxesValues = (\n700\n);\ninstanceInterpolations = {\nm01 = 1;\n};\nisBold = 1;\nlinkStyle = Regular;\nmanualInterpolation = 1;\nname = Bold;\nvisible = 0;\nweightClass = 700;\n},\n{\nname = Variable;\ntype = variable;\n}\n);\nmetrics = (",
    )
    .replace(
        "unitsPerEm = 1000;",
        "numbers = (\n{\nname = gap;\n}\n);\nstems = (\n{\nname = stem;\n},\n{\nhorizontal = 1;\nname = bar;\n}\n);\nunitsPerEm = 1000;",
    )
}

#[test]
fn version_3_stems_numbers_guides_and_instances_are_fields() {
    let text = v3_design();
    let font = read(&text).unwrap();
    let names: Vec<(&str, bool)> = font.stems.iter().map(|s| (s.name.as_str(), s.horizontal)).collect();
    assert_eq!(names, [("stem", false), ("bar", true)]);
    assert_eq!(font.numbers[0].name, "gap");
    let m = font.default_master().unwrap();
    assert_eq!((m.stem_values.clone(), m.number_values.clone()), (vec![80.0, 60.0], vec![40.0]));
    assert_eq!(m.guides.len(), 2);
    assert_eq!((m.guides[0].x, m.guides[0].angle, m.guides[0].locked, m.guides[0].name.as_deref()), (300.0, 90.0, true, Some("center")));
    assert_eq!((m.guides[1].x, m.guides[1].y), (0.0, 500.0));
    let bold = &font.instances[0];
    assert_eq!(bold.weight_class, Some(fontcraft_model::StyleClass::Number(700.0)));
    assert!(bold.is_bold && !bold.is_italic && bold.manual_interpolation && !bold.visible);
    assert_eq!((bold.link_style.as_deref(), bold.interpolation.clone()), (Some("Regular"), vec![("m01".to_string(), 1.0)]));
    assert_eq!(font.instances[1].kind.as_deref(), Some("variable"));
    assert!(font.instances[1].visible);
    for x in [&font.extra, &m.extra, &bold.extra] {
        assert!(x.is_empty(), "everything typed: {x:?}");
    }
    assert_eq!(write(&font).unwrap(), text);
}

#[test]
fn a_guide_colour_and_identifier_go_into_its_name() {
    let mut font = read(&v3_design()).unwrap();
    let m = &mut std::sync::Arc::make_mut(&mut font.masters)[0];
    m.guides[0].color = Some("1,0,0,1".into());
    m.guides[0].identifier = Some("g1".into());
    m.guides[1].identifier = Some("g2".into());
    let text = write(&font).unwrap();
    assert!(text.contains("name = \"center [1,0,0,1] [#g1]\";"), "{text}");
    assert!(text.contains("name = \" [#g2]\";"), "{text}");
    // Read back, the suffixes are part of the name, as glyphsLib reads them.
    let back = read(&text).unwrap();
    assert_eq!(back.masters[0].guides[0].name.as_deref(), Some("center [1,0,0,1] [#g1]"));
}

#[test]
fn editing_a_typed_design_field_writes_the_new_value() {
    let text = v3_design();
    let mut font = read(&text).unwrap();
    std::sync::Arc::make_mut(&mut font.masters)[0].stem_values[0] = 85.0;
    let i = &mut std::sync::Arc::make_mut(&mut font.instances)[0];
    i.weight_class = Some(fontcraft_model::StyleClass::Number(600.0));
    i.visible = true;
    let out = write(&font).unwrap();
    let changed: Vec<(&str, &str)> = text.lines().zip(out.lines()).filter(|(a, b)| a != b).collect();
    assert_eq!(changed.first(), Some(&("80,", "85,")));
    assert!(out.contains("weightClass = 600;") && !out.contains("visible = 0;"), "{out}");
    assert_eq!(read(&out).unwrap().instances[0].weight_class, Some(fontcraft_model::StyleClass::Number(600.0)));
}

#[test]
fn a_key_the_writer_leaves_out_is_kept_as_written() {
    let text = v3_design().replace("isBold = 1;", "isBold = 0;");
    let mut font = read(&text).unwrap();
    assert!(!font.instances[0].is_bold);
    assert_eq!(write(&font).unwrap(), text);
    std::sync::Arc::make_mut(&mut font.instances)[0].is_bold = true;
    assert!(write(&font).unwrap().contains("isBold = 1;"));
}

/// [`V2`] with name parts, three axis values (the width at its default, written anyway), stems,
/// a guide, a zone at no metric and an instance that is not exported.
fn v2_design() -> String {
    V2.replace("capHeight = 810;\n", "capHeight = 810;\ncustom = Italic;\ncustomValue = 12;\n")
        .replace("\"{578, 11}\",\n", "\"{578, 11}\",\n\"{400, 8}\",\n")
        .replace(
            "descender = -190;\n",
            "descender = -190;\nguideLines = (\n{\nangle = 90;\nposition = \"{100, 0}\";\n}\n);\nhorizontalStems = (\n20\n);\n",
        )
        .replace("weightValue = 180;\n", "verticalStems = (\n80,\n90\n);\nweight = Bold;\nweightValue = 180;\nwidthValue = 100;\n")
        .replace(
            "unitsPerEm = 1000;",
            "instances = (\n{\nactive = 0;\ninterpolationCustom = 12;\ninterpolationWeight = 180;\nisBold = 1;\nname = Bold;\nweightClass = Bold;\n}\n);\nunitsPerEm = 1000;",
        )
}

#[test]
fn version_2_axis_values_name_parts_stems_and_instances_are_fields() {
    let text = v2_design();
    let font = read(&text).unwrap();
    let axes: Vec<(&str, &str)> = font.axes.iter().map(|a| (a.name.as_str(), a.tag.as_str())).collect();
    assert_eq!(axes, [("Weight", "wght"), ("Width", "wdth"), ("Custom", "XXXX")], "as many as the highest key used");
    let m = font.default_master().unwrap();
    assert_eq!(m.location, [180.0, 100.0, 12.0]);
    assert!(m.extra.get(Family::Glyphs, "widthValue").is_some(), "a default written anyway stays as written");
    assert_eq!((m.name.as_str(), m.name_parts.weight.as_deref(), m.name_parts.custom.as_deref()), ("Bold Italic", Some("Bold"), Some("Italic")));
    let stems: Vec<bool> = font.stems.iter().map(|s| s.horizontal).collect();
    assert_eq!((stems, m.stem_values.clone()), (vec![true, false, false], vec![20.0, 80.0, 90.0]));
    assert_eq!((m.guides[0].x, m.guides[0].y, m.guides[0].angle), (100.0, 0.0, 90.0));
    assert_eq!(m.zones, [fontcraft_model::AlignmentZone { pos: 400.0, size: 8.0 }]);
    let i = &font.instances[0];
    assert!(!i.exports && i.is_bold);
    assert_eq!((i.location.clone(), i.weight_class.clone()), (vec![180.0, 100.0, 12.0], Some(fontcraft_model::StyleClass::Name("Bold".into()))));
    assert!(m.extra.get(Family::Glyphs, "alignmentZones").is_none() && font.extra.get(Family::Glyphs, "instances").is_none());
    assert_eq!(write(&font).unwrap(), text);
}

#[test]
fn version_2_writes_axis_values_except_defaults() {
    let mut font = read(&v2_design()).unwrap();
    let m = &mut std::sync::Arc::make_mut(&mut font.masters)[0];
    m.location = vec![100.0, 120.0, 12.0];
    std::sync::Arc::make_mut(&mut font.instances)[0].exports = true;
    let out = write(&font).unwrap();
    assert!(!out.contains("weightValue"), "the default is left out: {out}");
    assert!(out.contains("widthValue = 120;") && !out.contains("widthValue = 100;"));
    assert!(out.contains("active = 1;"), "the kept key is overwritten: {out}");
    let back = read(&out).unwrap();
    assert_eq!(back.masters[0].location, [100.0, 120.0, 12.0]);
    assert!(back.instances[0].exports);
}

#[test]
fn a_version_2_axes_parameter_names_the_axes_and_stays_a_parameter() {
    let param = "customParameters = (\n{\nname = Axes;\nvalue = (\n{\nName = Optical;\nTag = opsz;\n}\n);\n}\n);\n";
    let text = V2.replace("familyName = Old;\n", &format!("{param}familyName = Old;\n"));
    let mut font = read(&text).unwrap();
    assert_eq!((font.axes.len(), font.axes[0].tag.as_str(), font.masters[0].location.clone()), (1, "opsz", vec![180.0]));
    assert!(font.info.params.get("Axes").is_some());
    assert_eq!(write(&font).unwrap(), text);
    std::sync::Arc::make_mut(&mut font.axes)[0].tag = "OPSZ".into();
    let out = write(&font).unwrap();
    assert!(out.contains("Tag = OPSZ;"), "{out}");
    assert_eq!(read(&out).unwrap().axes[0].tag, "OPSZ");
    // Without values on any axis key, a file has no axes.
    assert!(read(&V2.replace("weightValue = 180;\n", "")).unwrap().axes.is_empty());
}

#[test]
fn an_axis_range_from_a_designspace_does_not_add_an_axes_parameter() {
    let mut font = read(V2).unwrap();
    std::sync::Arc::make_mut(&mut font.axes)[0].range = Some(fontcraft_model::AxisRange { minimum: 100.0, default: 180.0, maximum: 900.0 });
    assert!(!write(&font).unwrap().contains("name = Axes;"));
}

#[test]
fn version_2_holds_properties_and_palettes_in_its_own_forms() {
    use fontcraft_model::{Localized, Param, Property, PropertyValue};
    let mut font = read(V2).unwrap();
    let info = std::sync::Arc::make_mut(&mut font.info);
    info.properties.set("vendorID", "ABCD", false);
    let lang = |l: &str, v: &str| Localized { language: l.into(), value: v.into(), extra: Default::default() };
    info.properties.0.push(Property {
        key: "licenses".into(),
        value: PropertyValue::Localized(vec![lang("dflt", "Free"), lang("DEU", "Frei")]),
        extra: Default::default(),
    });
    info.properties.set("styleNames", "Old", true);
    let palette = "(\n(\n(\n255,\n0,\n0,\n255\n),\n(\n128,\n255\n)\n)\n)";
    info.params.0.push(Param::new("Color Palettes", fontcraft_plist::openstep::parse(palette).unwrap().root));
    let mut map = fontcraft_plist::Dict::new();
    map.insert("100", Value::Number(fontcraft_plist::Number::int(100)));
    let mut maps = fontcraft_plist::Dict::new();
    maps.insert("wght", Value::Dict(map));
    info.params.0.push(Param::new("Axis Mappings", Value::Dict(maps)));
    let mut instance = fontcraft_model::Instance { name: "Bold".into(), ..Default::default() };
    instance.properties.set("familyNames", "Old Family", true);
    instance.properties.set("designers", "Someone", true);
    std::sync::Arc::make_mut(&mut font.instances).push(instance);
    let out = write(&font).unwrap();
    for expected in [
        "name = vendorID;\nvalue = ABCD;",
        "name = license;\nvalue = Free;",
        "\"255,0,0,255\",\n\"128,255\"",
        "wght = {\n100 = 100;\n};",
        "name = familyName;\nvalue = \"Old Family\";",
    ] {
        assert!(out.contains(expected), "{expected} in {out}");
    }
    let lost = info_losses(&font);
    for expected in [
        "property `licenses`: languages other than the default (1)",
        "property `styleNames` (no key in format version 2)",
        "instance `Bold`: property `designers` (no parameter in format version 2)",
    ] {
        assert!(lost.iter().any(|l| l == expected), "{expected} in {lost:?}");
    }
    let back = read(&out).unwrap();
    assert_eq!(back.info.params.get("vendorID").and_then(Value::as_str), Some("ABCD"));
    assert_eq!(back.instances[0].params.get("familyName").and_then(Value::as_str), Some("Old Family"));
}

#[test]
fn zones_beyond_the_overshoots_are_lost_only_after_version_2() {
    let mut font = read(&v2_design()).unwrap();
    assert!(crate::info_losses(&font).is_empty(), "version 2 writes them into `alignmentZones`");
    assert!(write(&font).unwrap().contains("\"{400, 8}\""));
    for version in [3, 4] {
        font.source.glyphs_version = Some(version);
        // Name parts are version 2's too.
        assert_eq!(
            crate::info_losses(&font),
            ["master `Bold Italic`: alignment zones other than overshoots (1)", "master `Bold Italic`: name parts"]
        );
        let out = write(&font).unwrap();
        assert!(!out.contains("alignmentZones") && !out.contains("{400, 8}"), "version {version} has no place for them");
    }
    let m = &mut std::sync::Arc::make_mut(&mut font.masters)[0];
    m.zones.clear();
    m.name_parts = Default::default();
    assert!(crate::info_losses(&font).is_empty());
}

#[test]
fn designspace_axis_data_is_reported_lost() {
    use fontcraft_model::{AxisLabel, AxisRange, LangName};
    let mut font = read(&v3_design()).unwrap();
    assert!(crate::info_losses(&font).is_empty());
    let range = {
        let m = fontcraft_model::axes::mapping(&font, 0);
        AxisRange { minimum: m.minimum(), default: m.default, maximum: m.maximum() }
    };
    std::sync::Arc::make_mut(&mut font.axes)[0].range = Some(range);
    assert!(crate::info_losses(&font).is_empty(), "a range the axis parameters give back travels");
    let a = &mut std::sync::Arc::make_mut(&mut font.axes)[0];
    a.range = Some(AxisRange { maximum: range.maximum + 100.0, ..range });
    a.label_names.push(LangName { language: "de".into(), name: "Gewicht".into() });
    a.labels.push(AxisLabel { name: "Regular".into(), user_value: 400.0, elidable: true, ..Default::default() });
    let name = a.name.clone();
    assert_eq!(crate::info_losses(&font), [format!("axis `{name}`: its range, its name in other languages, its STAT labels")]);
}

/// A version 4 file with contextual kerning.
const V4_CONTEXT: &str = "{\n.appVersion = \"4004\";\n.formatVersion = 4;\nfontMaster = (\n{\nid = m01;\n},\n{\nid = m02;\n},\n);\nglyphs = (\n);\nkerningContext = {\n\"A V | T\" = {\nm01 = -20;\nm02 = 12.5;\n};\nT = {\nm02 = -8;\n};\n};\nunitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}\n";

#[test]
fn version_4_contextual_kerning_is_stored_as_written() {
    let mut font = read(V4_CONTEXT).unwrap();
    let ctx = &font.kerning.context;
    assert_eq!(ctx.len(), 2);
    assert_eq!(ctx[0].context, "A V | T");
    assert_eq!(ctx[0].values, [("m01".to_string(), -20.0), ("m02".to_string(), 12.5)]);
    assert!(font.extra.get(Family::Glyphs, "kerningContext").is_none(), "typed, not carried");
    assert_eq!(write(&font).unwrap(), V4_CONTEXT);
    std::sync::Arc::make_mut(&mut font.kerning).context[1].values[0].1 = -9.0;
    let out = write(&font).unwrap();
    assert!(out.contains("T = {\nm02 = -9;\n};"), "{out}");
    assert_eq!(read(&out).unwrap().kerning.context[1].values[0].1, -9.0);
}

#[test]
fn contextual_kerning_that_would_not_write_back_stays_as_written() {
    for text in [
        V4_CONTEXT.replace("m02 = -8;", "m02 = odd;"),
        V4_CONTEXT.replace("m01 = -20;", "m01 = -20.0;"),
        V4_CONTEXT.replace(".formatVersion = 4;", ".formatVersion = 3;").replace("},\n);\nglyphs", "}\n);\nglyphs"),
    ] {
        let font = read(&text).unwrap();
        assert!(font.kerning.context.is_empty(), "{text}");
        assert!(font.extra.get(Family::Glyphs, "kerningContext").is_some());
        assert_eq!(write(&font).unwrap(), text);
    }
}

#[test]
fn a_version_4_package_keeps_contextual_kerning_in_its_kerning_file() {
    let font = read(V4_CONTEXT).unwrap();
    let files = write_package(&font).unwrap();
    let kerning = std::str::from_utf8(&files["kerning.plist"]).unwrap();
    assert!(kerning.contains("kerningContext = {"), "{kerning}");
    assert!(!std::str::from_utf8(&files["fontinfo.plist"]).unwrap().contains("kerningContext"));
    let back = read_package(&files).unwrap();
    assert_eq!(back.kerning.context, font.kerning.context);
    assert!(back.files.is_empty(), "the kerning file is read, not kept");
}

#[test]
fn contextual_kerning_is_reported_lost_before_version_4() {
    let mut font = read(V4_CONTEXT).unwrap();
    assert!(info_losses(&font).is_empty());
    for version in [3, 2] {
        font.source.glyphs_version = Some(version);
        assert!(info_losses(&font).iter().any(|l| l.starts_with("contextual kerning (2 contexts)")), "{:?}", info_losses(&font));
        assert!(!write(&font).unwrap().contains("kerningContext"), "version {version} has no place for it");
    }
}

/// [`V3`] with a stylistic set whose names are labels.
fn v3_labels() -> String {
    V3.replace(
        "fontMaster = (",
        "features = (\n{\nautomatic = 1;\ncode = \"sub A by B;\";\nlabels = (\n{\nlanguage = dflt;\nvalue = \"Alternate A\";\n},\n{\nlanguage = DEU;\nvalue = \"Anderes A\";\n}\n);\ntag = ss01;\n}\n);\nfontMaster = (",
    )
}

#[test]
fn feature_labels_are_fields() {
    let text = v3_labels();
    let mut font = read(&text).unwrap();
    let ss01 = &font.features.features[0];
    assert_eq!(
        ss01.labels.iter().map(|l| (l.language.as_str(), l.value.as_str())).collect::<Vec<_>>(),
        [("dflt", "Alternate A"), ("DEU", "Anderes A")]
    );
    assert!(ss01.extra.is_empty(), "typed, not carried");
    assert_eq!(write(&font).unwrap(), text);
    std::sync::Arc::make_mut(&mut font.features).features[0].labels[1].value = "Zweites A".into();
    let out = write(&font).unwrap();
    assert!(out.contains("value = \"Zweites A\";"), "{out}");
    assert_eq!(read(&out).unwrap().features.features[0].labels[1].value, "Zweites A");
}

#[test]
fn labels_that_would_not_write_back_stay_as_written() {
    // A text the writer would spell otherwise is read, and keeps its spelling.
    let text = v3_labels().replace("value = \"Alternate A\";", "value = \"Alternate\\012A\";");
    let font = read(&text).unwrap();
    assert_eq!(font.features.features[0].labels[0].value, "Alternate\nA");
    assert_eq!(write(&font).unwrap(), text);
    for text in [
        v3_labels().replace("language = DEU;\n", "language = \"DEU\";\n"),
        v3_labels().replace(
            "labels = (\n{\nlanguage = dflt;\nvalue = \"Alternate A\";\n},\n{\nlanguage = DEU;\nvalue = \"Anderes A\";\n}\n);",
            "labels = (\n);",
        ),
    ] {
        let font = read(&text).unwrap();
        assert!(font.features.features[0].labels.is_empty(), "{text}");
        assert!(font.features.features[0].extra.get(Family::Glyphs, "labels").is_some());
        assert_eq!(write(&font).unwrap(), text);
    }
}

/// [`V3`] with a feature class and two classes that are UFO groups, named in the user data as
/// glyphsLib names them.
fn v3_groups() -> String {
    V3.replace(
        "familyName = ",
        "classes = (\n{\ncode = \"A B\";\nname = Upper;\n},\n{\ncode = A;\nname = round;\n},\n{\ncode = \"\";\nname = empty;\n}\n);\nfamilyName = ",
    )
    .replace(
        "unitsPerEm = 1000;\n",
        "unitsPerEm = 1000;\nuserData = {\ncom.schriftgestaltung.Glyphs.groupsNotInFeature = (\nround,\nempty\n);\nzzz = 1;\n};\n",
    )
}

#[test]
fn classes_named_as_groups_are_groups() {
    let text = v3_groups();
    let mut font = read(&text).unwrap();
    assert_eq!(font.features.classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Upper"]);
    let groups: Vec<(&str, Vec<&str>)> = font.groups.iter().map(|g| (g.name.as_str(), g.members.iter().map(String::as_str).collect())).collect();
    assert_eq!(groups, [("round", vec!["A"]), ("empty", vec![])]);
    assert!(font.info.user_data.get(groups::GROUPS_KEY).is_none(), "the key is the groups' list, not user data");
    assert_eq!(write(&font).unwrap(), text);
    std::sync::Arc::make_mut(&mut font.groups).push(fontcraft_model::Group { name: "new".into(), members: vec!["A".into(), "B".into()] });
    let out = write(&font).unwrap();
    assert!(out.contains("{\ncode = \"A B\";\nname = new;\n}\n);"), "{out}");
    assert!(out.contains("round,\nempty,\nnew\n);"), "{out}");
    assert_eq!(read(&out).unwrap().groups.len(), 3);
}

#[test]
fn classes_stay_classes_unless_the_groups_come_last_and_plain() {
    for text in [
        // Not the last classes.
        v3_groups().replace("round,\nempty\n", "Upper\n"),
        // Out of order.
        v3_groups().replace("round,\nempty\n", "empty,\nround\n"),
        // A class with a switch.
        v3_groups().replace("code = A;\nname = round;", "automatic = 1;\ncode = A;\nname = round;"),
        // The key not at its sorted place.
        v3_groups().replace(
            "userData = {\ncom.schriftgestaltung.Glyphs.groupsNotInFeature = (\nround,\nempty\n);\nzzz = 1;\n};",
            "userData = {\nzzz = 1;\ncom.schriftgestaltung.Glyphs.groupsNotInFeature = (\nround,\nempty\n);\n};",
        ),
    ] {
        let font = read(&text).unwrap();
        assert!(font.groups.is_empty(), "{text}");
        assert_eq!(write(&font).unwrap(), text);
    }
}

#[test]
fn version_2_reports_feature_labels_lost() {
    let mut font = read(&v3_labels()).unwrap();
    font.source.glyphs_version = Some(2);
    assert!(info_losses(&font).contains(&"feature labels (1 features)".to_string()), "{:?}", info_losses(&font));
}

#[test]
fn hand_written_files_read_as_the_reference_compilers_read_them() {
    // A layer without a width, and version 2's top-level names in a version 3 file.
    let v3 = "{\n.formatVersion = 3;\ncopyright = \"Copy!\";\ndesigner = Someone;\nfontMaster = (\n{\nid = m01;\n}\n);\nglyphs = (\n{\nglyphname = a;\nlayers = (\n{\nlayerId = m01;\n}\n);\n}\n);\nunitsPerEm = 1000;\n}\n";
    let font = read(v3).unwrap();
    assert_eq!(font.glyph("a").unwrap().layers[0].width, 600.0);
    assert_eq!(font.info.properties.value("copyrights"), Some("Copy!"));
    assert_eq!(font.info.properties.value("designers"), Some("Someone"));
    let out = write(&font).unwrap();
    assert!(out.contains("copyright = \"Copy!\";") && !out.contains("properties"), "the names are written where they were: {out}");
    // An edited name is written as a property, which wins over the key when read again.
    let mut edited = font.clone();
    std::sync::Arc::make_mut(&mut edited.info).properties.set("copyrights", "Edited", true);
    let again = read(&write(&edited).unwrap()).unwrap();
    assert_eq!(again.info.properties.value("copyrights"), Some("Edited"));
    // A deleted name stays deleted, in every version.
    let mut deleted = font.clone();
    assert!(std::sync::Arc::make_mut(&mut deleted.info).properties.remove("designers"));
    for version in [2, 3, 4] {
        deleted.source.glyphs_version = Some(version);
        let again = read(&write(&deleted).unwrap()).unwrap();
        assert_eq!(again.info.properties.value("designers"), None, "version {version}");
        assert_eq!(again.info.properties.value("copyrights"), Some("Copy!"), "version {version}");
    }

    let v2 = "{\nfamilyName = V2;\nfontMaster = (\n{\nid = m01;\n}\n);\nglyphs = (\n{\nglyphname = a;\nlayers = (\n{\nlayerId = m01;\n}\n);\n}\n);\nunitsPerEm = 1000;\n}\n";
    assert_eq!(read(v2).unwrap().glyph("a").unwrap().layers[0].width, 600.0);
}

#[test]
fn one_line_dictionaries_are_read_into_the_model_and_keep_their_layout() {
    let text = V3
        .replace("unitsPerEm = 1000;", "numbers = ({name = gap;});\nunitsPerEm = 1000;")
        .replace("associatedMasterId = m01;\n", "associatedMasterId = m01;\nattr = {axisRules = ({min = 600;});};\n");
    let font = read(&text).unwrap();
    assert_eq!(font.numbers.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["gap"]);
    assert!(font.extra.get(Family::Glyphs, "numbers").is_none(), "typed, not carried");
    let alternate = &font.glyph("A").unwrap().layers[1];
    assert_eq!(alternate.info.attrs.axis_rules, [fontcraft_model::AxisRule { min: Some(600.0), max: None }]);
    assert_eq!(write(&font).unwrap(), text);

    // An edited value is written in the writer's layout.
    let mut edited = font.clone();
    Arc::make_mut(&mut edited.numbers)[0].name = "space".into();
    assert!(write(&edited).unwrap().contains("numbers = (\n{\nname = space;\n}\n);"));
}

#[test]
fn a_large_one_line_dictionary_is_read_and_written_in_linear_time() {
    // Matching entries by key, one search per entry, was quadratic in the dictionary's size.
    let weights = (0..200_000).map(|i| format!("m{i} = 1;")).collect::<Vec<_>>().join(" ");
    let text = V3.replace("metrics = (", &format!("instances = (\n{{\ninstanceInterpolations = {{{weights}}};\nname = Big;\n}}\n);\nmetrics = ("));
    let started = std::time::Instant::now();
    let font = read(&text).unwrap();
    assert_eq!(font.instances[0].interpolation.len(), 200_000);
    let written = write(&font).unwrap();
    assert!(written == text, "the large dictionary was not written back as read");
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
}

#[test]
fn a_value_kept_for_its_layout_is_left_out_once_the_field_changes() {
    let text = V3.replace("unitsPerEm = 1000;", "numbers = ({name = gap;});\nunitsPerEm = 1000;");
    let mut font = read(&text).unwrap();
    Arc::make_mut(&mut font.numbers)[0].name = "space".into();
    let written = write(&font).unwrap();
    assert!(written.contains("numbers = (\n{\nname = space;\n}\n);") && !written.contains("gap") && !written.contains(LAYOUT_KEY), "{written}");
    // Written in another version, the kept value still matches while the field gives it.
    let mut font = read(&text).unwrap();
    font.source.glyphs_version = Some(4);
    let v4 = write(&font).unwrap();
    assert!(v4.contains("numbers = ({name = gap;});"), "{v4}");
}
