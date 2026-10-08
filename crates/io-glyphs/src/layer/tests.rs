use super::*;
use fontcraft_model::{ColorLabel, Layer};
use fontcraft_plist::openstep;

fn value(text: &str) -> Value {
    openstep::parse(text).unwrap().root
}

/// Reads a layer dictionary in `version` and writes it back.
fn round_trip(text: &str, version: u32) -> (Layer, String) {
    let l = read(text, version);
    (l.clone(), write(&l, version))
}

fn read(text: &str, version: u32) -> Layer {
    if version == 2 { crate::v2::read_layer(value(text), "test").unwrap() } else { crate::read_layer(value(text), "test", version).unwrap() }
}

fn write(l: &Layer, version: u32) -> String {
    let v = if version == 2 { crate::v2::write_layer(l, "test").unwrap() } else { crate::write_layer(l, "test", version).unwrap() };
    let mut out = String::new();
    openstep::write_value(&mut out, &v, dialect(version));
    out
}

const V3_LAYER: &str = "{
annotations = (
{
angle = 45.5;
pos = (248,407);
type = Arrow;
},
{
pos = (292,575);
text = \"a note\";
type = Text;
width = 120;
}
);
associatedMasterId = m01;
attr = {
axisRules = (
{
max = 600;
min = 155;
}
);
colorPalette = 2;
coordinates = (
550,
100
);
};
background = {
anchors = (
{
name = top;
pos = (10,20);
}
);
guides = (
{
pos = (1,2);
}
);
shapes = (
{
closed = 1;
nodes = (
(0,0,l),
(10,0,l),
(10,10,l)
);
}
);
};
backgroundImage = {
alpha = 70;
angle = 3;
crop = (0,0,140,160);
imagePath = \"files/Smily.png\";
locked = 1;
pos = (106,89);
scale = (0.9,0.8);
};
color = 4;
guides = (
{
angle = 90;
pos = (422,356);
}
);
hints = (
{
horizontal = 1;
origin = (0,0);
target = (0,7);
type = Stem;
}
);
layerId = L1;
metricLeft = \"=H\";
metricRight = n;
metricVertOrigin = a;
name = \"Apr 1 24\";
partSelection = {
Width = 2;
height = 1;
};
userData = {
k = v;
};
vertOrigin = 128;
vertWidth = 780;
visible = 1;
width = 600;
}";

#[test]
fn each_field_reads_in_version_3() {
    let (l, written) = round_trip(V3_LAYER, 3);
    assert!(l.extra.is_empty(), "left over: {:?}", l.extra);
    let i = &*l.info;
    assert_eq!(i.name.as_deref(), Some("Apr 1 24"));
    assert_eq!(i.attrs.coordinates, [Some(550.0), Some(100.0)]);
    assert_eq!(i.attrs.axis_rules, [AxisRule { min: Some(155.0), max: Some(600.0) }]);
    assert_eq!(i.attrs.color_palette, Some(PaletteEntry::Index(2)));
    assert_eq!(i.attrs.smart_poles, [SmartPole { property: "Width".into(), top: true }, SmartPole { property: "height".into(), top: false }]);
    assert_eq!((i.vert_width, i.vert_origin), (Some(780.0), Some(128.0)));
    assert_eq!(i.metrics_keys.left.as_deref(), Some("=H"));
    assert_eq!(i.metrics_keys.vert_origin.as_deref(), Some("a"));
    assert_eq!(i.color, Some(ColorLabel::Index(4)));
    assert!(i.visible);
    assert_eq!((i.guides.len(), i.guides[0].angle), (1, 90.0));
    assert_eq!(i.hints.len(), 1);
    assert_eq!(i.hints[0].kind(), Some("Stem"));
    assert_eq!(i.annotations.len(), 2);
    assert_eq!((i.annotations[1].kind.as_str(), i.annotations[1].text.as_deref(), i.annotations[1].width), ("Text", Some("a note"), 120.0));
    let image = i.image.as_ref().unwrap();
    assert_eq!(image.path.as_deref(), Some("files/Smily.png"));
    assert_eq!(image.transform, Transform::Decomposed { pos: (106.0, 89.0), scale: (0.9, 0.8), angle: 3.0, slant: (0.0, 0.0) });
    assert_eq!((image.crop, image.alpha, image.locked), (Some([0.0, 0.0, 140.0, 160.0]), Some(70.0), true));
    let b = i.background.as_ref().unwrap();
    assert_eq!((b.shapes.len(), b.anchors.len(), b.guides.len()), (1, 1, 1));
    assert!(matches!(b.shapes[0], Shape::Path(_)));
    assert!(i.user_data.get("k").is_some());
    assert_eq!(written, V3_LAYER, "written back as read");
}

#[test]
fn version_4_coordinates_may_use_the_masters_value() {
    let text = "{\nassociatedMasterId = m01;\nattr = {\ncoordinates = (\n\"-\",\n120,\n);\n};\nlayerId = L1;\nvisible = 1;\nwidth = 600;\n}";
    let (l, written) = round_trip(text, 4);
    assert_eq!(l.info.attrs.coordinates, [None, Some(120.0)]);
    assert!(l.info.visible);
    assert_eq!(written, text);
    // A dash is not a number in version 3: the attributes stay as written.
    let l = read(text, 3);
    assert!(l.info.attrs.is_empty());
    assert!(l.extra.get(fontcraft_model::Family::Glyphs, "attr").is_some());
}

#[test]
fn colour_layer_attributes() {
    let text = "{\nassociatedMasterId = m01;\nattr = {\ncolor = 1;\nsbixSize = 64;\nsvg = 1;\n};\nlayerId = L1;\nwidth = 600;\n}";
    let (l, written) = round_trip(text, 3);
    assert!(l.info.attrs.full_color && l.info.attrs.svg);
    assert_eq!(l.info.attrs.sbix_size, Some(64));
    assert_eq!(written, text);
    let text = "{\nassociatedMasterId = m01;\nattr = {\ncolorPalette = \"*\";\nunknownKey = 3;\n};\nlayerId = L1;\nwidth = 600;\n}";
    let (l, written) = round_trip(text, 3);
    assert_eq!(l.info.attrs.color_palette, Some(PaletteEntry::Foreground));
    assert!(l.info.attrs.extra.get(fontcraft_model::Family::Glyphs, "unknownKey").is_some(), "an attribute we do not know is kept with them");
    assert_eq!(written, text);
}

const V2_LAYER: &str = "{
annotations = (
{
position = \"{12, 30}\";
text = hi;
type = Text;
}
);
associatedMasterId = m01;
background = {
components = (
{
name = B;
}
);
guideLines = (
{
position = \"{5, 6}\";
}
);
paths = (
{
closed = 1;
nodes = (
\"0 0 LINE\",
\"10 0 LINE\",
\"10 10 LINE\"
);
}
);
};
backgroundImage = {
crop = \"{{0, 0}, {140, 160}}\";
imagePath = Screenshot.png;
locked = \"1\";
transform = \"{0.89877, 0.04712, -0.04188, 0.7989, 106, 89}\";
};
guideLines = (
{
angle = 11.2478;
position = \"{172, 310}\";
}
);
hints = (
{
horizontal = 1;
origin = \"{0, 0}\";
target = \"{0, 3}\";
type = Stem;
}
);
layerId = L1;
leftMetricsKey = H;
rightMetricsKey = n;
widthMetricsKey = o;
name = \"Bold [300]\";
userData = {
PartSelection = {
Width = 1;
};
zz = 1;
};
vertOrigin = -155;
vertWidth = 0;
width = 600;
}";

#[test]
fn version_2_spells_its_own_keys() {
    let (l, written) = round_trip(V2_LAYER, 2);
    assert!(l.extra.is_empty(), "left over: {:?}", l.extra);
    let i = &*l.info;
    assert_eq!(i.name.as_deref(), Some("Bold [300]"));
    assert_eq!(i.attrs.axis_rules, [AxisRule { min: Some(300.0), max: None }], "the bracket in the name");
    assert_eq!(i.attrs.smart_poles, [SmartPole { property: "Width".into(), top: false }]);
    assert_eq!(i.user_data.0.len(), 1, "the poles leave the user data");
    assert_eq!((i.metrics_keys.left.as_deref(), i.metrics_keys.width.as_deref()), (Some("H"), Some("o")));
    assert_eq!((i.vert_width, i.vert_origin), (Some(0.0), Some(-155.0)));
    assert_eq!((i.guides[0].x, i.guides[0].y), (172.0, 310.0));
    assert_eq!((i.annotations[0].x, i.annotations[0].y), (12.0, 30.0));
    let image = i.image.as_ref().unwrap();
    assert_eq!(image.transform, Transform::Matrix([0.89877, 0.04712, -0.04188, 0.7989, 106.0, 89.0]));
    assert_eq!((image.crop, image.locked), (Some([0.0, 0.0, 140.0, 160.0]), true));
    let b = i.background.as_ref().unwrap();
    assert_eq!(b.shapes.len(), 2, "paths and components");
    assert!(matches!(b.shapes[0], Shape::Path(_)) && matches!(b.shapes[1], Shape::Component(_)));
    assert_eq!(b.guides.len(), 1);
    assert_eq!(written, V2_LAYER, "written back as read");
}

#[test]
fn version_2_names_give_the_attributes() {
    assert_eq!(v2_attrs("{120, 80}").coordinates, [Some(120.0), Some(80.0)]);
    assert_eq!(v2_attrs("Medium {120}").coordinates, [Some(120.0)]);
    assert!(v2_attrs("x {120}.background").coordinates.is_empty());
    assert!(v2_attrs("{a, b}").coordinates.is_empty());
    assert_eq!(v2_attrs("]600]").axis_rules, [AxisRule { min: None, max: Some(600.0) }]);
    assert_eq!(v2_attrs("Alt [ 300 ] then [400]").axis_rules, [AxisRule { min: Some(400.0), max: None }], "the last bracket");
    assert!(v2_attrs("[x]").axis_rules.is_empty());
    assert_eq!(v2_attrs("Color 3").color_palette, Some(PaletteEntry::Index(3)));
    assert_eq!(v2_attrs(" Color * ").color_palette, Some(PaletteEntry::Foreground));
    assert!(v2_attrs("Colors 3").color_palette.is_none());
    assert!(v2_attrs("Apr 1 24").is_empty());
}

#[test]
fn explicit_defaults_and_odd_spellings_stay_as_written() {
    let text = "{\nannotations = (\n);\nassociatedMasterId = m01;\nattr = {\n};\nbackgroundImage = {\nlocked = 0;\n};\nguides = (\n);\nhints = (\n);\nlayerId = L1;\nname = \"\";\npartSelection = {\n};\nuserData = {\n};\nvisible = 0;\nwidth = 600;\n}";
    let (l, written) = round_trip(text, 3);
    // The image is typed; its `locked = 0` stays with it.
    let image = l.info.image.clone().unwrap();
    assert!(!image.locked && image.extra.len() == 1);
    assert!(LayerInfo { image: None, ..(*l.info).clone() } == LayerInfo::default(), "nothing else typed: {:?}", *l.info);
    assert_eq!(l.extra.len(), 8, "every other key stays in Extra");
    assert_eq!(written, text);
    // An empty background is a background.
    let text = "{\nbackground = {\n};\nlayerId = m01;\nwidth = 600;\n}";
    let (l, written) = round_trip(text, 3);
    assert_eq!(l.info.background, Some(Background::default()));
    assert_eq!(written, text);
}

#[test]
fn a_background_that_would_not_write_back_stays_as_written() {
    // An anchor without a name is written with one, so the background stays whole in Extra.
    let text = "{\nbackground = {\nanchors = (\n{\npos = (1,2);\n}\n);\n};\nlayerId = m01;\nwidth = 600;\n}";
    let (l, written) = round_trip(text, 3);
    assert!(l.info.background.is_none());
    assert!(l.extra.get(fontcraft_model::Family::Glyphs, "background").is_some());
    assert_eq!(written, text);
}

#[test]
fn an_edit_writes_the_new_value() {
    let mut l = read(V3_LAYER, 3);
    let i = &mut *l.info;
    i.name = Some("Renamed".into());
    i.attrs.coordinates = vec![Some(300.0)];
    i.attrs.smart_poles.clear();
    i.visible = false;
    i.vert_width = None;
    i.guides.clear();
    if let Some(img) = &mut i.image {
        img.locked = false;
    }
    let written = write(&l, 3);
    assert!(written.contains("name = Renamed;"));
    assert!(written.contains("coordinates = (\n300\n);"));
    for gone in ["partSelection", "visible", "vertWidth", "angle = 90;", "locked"] {
        assert!(!written.contains(gone), "{gone} is still written");
    }
    // A layer of a new document gets its fields written.
    let mut l = Layer::new("m01", 500.0);
    l.info.vert_origin = Some(10.0);
    l.info.annotations.push(Annotation { kind: "Plus".into(), x: 1.0, y: 2.0, angle: 0.0, text: None, width: 0.0, extra: Extra::new() });
    assert_eq!(write(&l, 3), "{\nannotations = (\n{\npos = (1,2);\ntype = Plus;\n}\n);\nlayerId = m01;\nvertOrigin = 10;\nwidth = 500;\n}");
}

#[test]
fn counts_from_input_are_capped() {
    let many: Vec<String> = (0..=MAX_LAYER_GUIDES).map(|i| format!("{{\npos = ({i},0);\n}}")).collect();
    let text = format!("{{\nguides = (\n{}\n);\nlayerId = m01;\nwidth = 600;\n}}", many.join(",\n"));
    let l = read(&text, 3);
    assert!(l.info.guides.is_empty());
    assert_eq!(l.extra.len(), 1);
    let coords: Vec<String> = (0..=MAX_AXIS_VALUES).map(|i| i.to_string()).collect();
    let text = format!("{{\nattr = {{\ncoordinates = (\n{}\n);\n}};\nlayerId = m01;\nwidth = 600;\n}}", coords.join(",\n"));
    assert!(read(&text, 3).info.attrs.is_empty());
}

#[test]
fn version_2_reports_attributes_its_names_do_not_say() {
    let mut l = read(V2_LAYER, 2);
    let glyph = |l: &Layer| {
        let mut g = fontcraft_model::Glyph::new("a");
        g.layers.push(l.clone());
        vec![std::sync::Arc::new(g)]
    };
    assert!(losses(&glyph(&l), 2).is_empty());
    l.info.attrs.svg = true;
    assert_eq!(losses(&glyph(&l), 2), ["layer attributes that the layers' names do not say (1)"]);
    l.info.attrs.svg = false;
    l.info.color = Some(ColorLabel::Rgba([0.5, 0.0, 0.0, 1.0]));
    assert_eq!(losses(&glyph(&l), 3), ["the layers' colours, rounded to 8 bits (1)"]);
}

#[test]
fn values_beyond_the_models_limits_stay_as_written() {
    // The model's check would refuse these; kept as written, the file still opens.
    for key in
        ["vertWidth = 1e300;", "guides = (\n{\npos = (2000000,0);\n}\n);", "background = {\nanchors = (\n{\nname = a;\npos = (0,9e9);\n}\n);\n};"]
    {
        let text = format!("{{\n{key}\nlayerId = m01;\nwidth = 600;\n}}");
        let l = read(&text, 3);
        assert!(l.info.is_default(), "{key}");
        assert_eq!(l.extra.len(), 1, "{key}");
    }
}
