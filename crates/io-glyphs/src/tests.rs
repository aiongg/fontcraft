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
