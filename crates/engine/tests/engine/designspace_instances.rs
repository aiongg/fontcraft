//! Static instances of a designspace source: the names, classes and font information fontmake's
//! instancer gives them. The expected values are those fontmake 3.12.1 writes for the same
//! designspace and UFOs (`fontmake -m Slice.designspace -i -o ttf`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_io_ufo::Files;
use fontcraft_io_ufo::designspace;
use fontcraft_model::Target;
use skrifa::FontRef;
use skrifa::raw::TableProvider;

fn plist(body: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
    )
    .into_bytes()
}

fn glif(name: &str, unicode: Option<&str>, right: u32) -> Vec<u8> {
    let unicode = unicode.map(|u| format!("<unicode hex=\"{u}\"/>")).unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<glyph name=\"{name}\" format=\"2\">\n<advance width=\"600\"/>{unicode}\n<outline><contour>\
<point x=\"50\" y=\"0\" type=\"line\"/><point x=\"{right}\" y=\"0\" type=\"line\"/><point x=\"{right}\" y=\"700\" type=\"line\"/><point x=\"50\" y=\"700\" type=\"line\"/>\
</contour></outline>\n</glyph>\n"
    )
    .into_bytes()
}

/// A UFO of family Slice with glyphs `A` (a box `stem` wide) and `A.alt` (twice as wide), and
/// `info` added to its font information.
fn ufo(style: &str, stem: u32, info: &str) -> Files {
    let fontinfo = format!(
        "<dict><key>familyName</key><string>Slice</string><key>styleName</key><string>{style}</string>\
<key>unitsPerEm</key><integer>1000</integer><key>ascender</key><integer>800</integer><key>descender</key><integer>-200</integer>\
<key>xHeight</key><integer>500</integer><key>capHeight</key><integer>700</integer>{info}</dict>"
    );
    BTreeMap::from([
        ("metainfo.plist".to_string(), plist("<dict><key>creator</key><string>test</string><key>formatVersion</key><integer>3</integer></dict>")),
        ("fontinfo.plist".to_string(), plist(&fontinfo)),
        ("layercontents.plist".to_string(), plist("<array><array><string>public.default</string><string>glyphs</string></array></array>")),
        ("glyphs/contents.plist".to_string(), plist("<dict><key>A</key><string>A_.glif</string><key>A.alt</key><string>A_.alt.glif</string></dict>")),
        ("glyphs/A_.glif".to_string(), glif("A", Some("0041"), 50 + stem)),
        ("glyphs/A_.alt.glif".to_string(), glif("A.alt", None, 50 + 2 * stem)),
    ])
}

const DOC: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<designspace format="5.0">
  <axes>
    <axis tag="wght" name="Weight" minimum="300" maximum="700" default="300">
      <map input="300" output="300"/>
      <map input="400" output="450"/>
      <map input="700" output="700"/>
    </axis>
  </axes>
  <rules>
    <rule name="heavy">
      <conditionset>
        <condition name="Weight" minimum="600" maximum="700"/>
      </conditionset>
      <sub name="A" with="A.alt"/>
    </rule>
  </rules>
  <sources>
    <source filename="Slice-Light.ufo" name="Slice Light">
      <location><dimension name="Weight" xvalue="300"/></location>
    </source>
    <source filename="Slice-Bold.ufo" name="Slice Bold">
      <location><dimension name="Weight" xvalue="700"/></location>
    </source>
  </sources>
  <instances>
    <instance familyname="Slice" stylename="Medium" filename="out/Slice-Medium.ufo">
      <location><dimension name="Weight" xvalue="500"/></location>
    </instance>
    <instance familyname="Slice Display" stylename="Bold" postscriptfontname="SliceDisplay-Bd" filename="out/SliceDisplay-Bold.ufo">
      <location><dimension name="Weight" xvalue="700"/></location>
    </instance>
    <instance familyname="Slice" stylename="Italic" filename="out/Slice-Italic.ufo">
      <location><dimension name="Weight" xvalue="400"/></location>
    </instance>
    <instance familyname="Slice" stylename="Book" stylemapfamilyname="Slice Book" stylemapstylename="regular" filename="out/Slice-Book.ufo">
      <location><dimension name="Weight" xvalue="350.6"/></location>
    </instance>
  </instances>
</designspace>
"#;

/// The two masters state names of their own style and weight classes (100 and 900) that no
/// instance takes.
fn sources() -> BTreeMap<String, Files> {
    let light = "<key>styleMapFamilyName</key><string>Slice Light</string><key>styleMapStyleName</key><string>regular</string>\
<key>postscriptFontName</key><string>Slice-Light</string><key>postscriptFullName</key><string>Slice Light</string>\
<key>openTypeNamePreferredSubfamilyName</key><string>Light</string><key>openTypeNameUniqueID</key><string>uid-light</string>\
<key>openTypeNameDesigner</key><string>Ann Example</string><key>openTypeOS2VendorID</key><string>TEST</string>\
<key>openTypeOS2WeightClass</key><integer>100</integer>";
    let bold = "<key>styleMapFamilyName</key><string>Slice</string><key>styleMapStyleName</key><string>bold</string>\
<key>postscriptFontName</key><string>Slice-Bold</string><key>openTypeOS2WeightClass</key><integer>900</integer>";
    BTreeMap::from([("Slice-Light.ufo".to_string(), ufo("Light", 100, light)), ("Slice-Bold.ufo".to_string(), ufo("Bold", 300, bold))])
}

struct Expected {
    style: &'static str,
    names: &'static [(u16, &'static str)],
    weight: u16,
    /// `fsSelection` bits 0, 5 and 6, and `head.macStyle`.
    selection: u16,
    mac_style: u16,
}

const EXPECTED: &[Expected] = &[
    Expected {
        style: "Medium",
        names: &[(1, "Slice Medium"), (2, "Regular"), (4, "Slice Medium"), (6, "Slice-Medium"), (9, "Ann Example"), (16, "Slice"), (17, "Medium")],
        weight: 460,
        selection: 0x40,
        mac_style: 0,
    },
    Expected {
        style: "Bold",
        names: &[(1, "Slice Display"), (2, "Bold"), (4, "Slice Display Bold"), (6, "SliceDisplay-Bd"), (9, "Ann Example")],
        weight: 700,
        selection: 0x20,
        mac_style: 1,
    },
    Expected {
        style: "Italic",
        names: &[(1, "Slice"), (2, "Italic"), (4, "Slice Italic"), (6, "Slice-Italic"), (9, "Ann Example")],
        weight: 366,
        selection: 0x01,
        mac_style: 2,
    },
    Expected {
        style: "Book",
        names: &[(1, "Slice Book"), (2, "Regular"), (4, "Slice Book"), (6, "Slice-Book"), (9, "Ann Example"), (16, "Slice"), (17, "Book")],
        weight: 333,
        selection: 0x40,
        mac_style: 0,
    },
];

#[test]
fn designspace_instances_take_names_and_classes_from_their_element_as_fontmake_does() {
    let font = designspace::read(DOC, &sources()).unwrap();
    let options = Options { skip_features: true, remove_overlap: false, ..Options::default() };
    for (index, case) in EXPECTED.iter().enumerate() {
        assert_eq!(font.instances[index].name, case.style);
        let out = compile(&font, &Target::Instance(index), Flavour::TrueType, &options).unwrap();
        let built = FontRef::new(&out.bytes).unwrap();
        let table = built.name().unwrap();
        let data = table.string_data();
        let records: Vec<(u16, String)> = table
            .name_record()
            .iter()
            .filter(|r| r.platform_id() == 3 && r.language_id() == 0x409 && matches!(r.name_id().to_u16(), 1 | 2 | 4 | 6 | 9 | 16 | 17))
            .map(|r| (r.name_id().to_u16(), r.string(data).unwrap().chars().collect()))
            .collect();
        let expected: Vec<(u16, String)> = case.names.iter().map(|(id, t)| (*id, (*t).to_string())).collect();
        assert_eq!(records, expected, "{}", case.style);
        let os2 = built.os2().unwrap();
        assert_eq!(os2.us_weight_class(), case.weight, "{}: the location gives the class, not the masters' classes", case.style);
        assert_eq!(os2.fs_selection().bits() & 0x61, case.selection, "{}", case.style);
        assert_eq!(built.head().unwrap().mac_style().bits(), case.mac_style, "{}", case.style);
        assert_eq!(os2.ach_vend_id().to_string(), "TEST", "{}: the default source's vendor is copied", case.style);
    }
}

#[test]
fn style_map_names_the_element_leaves_out_are_ufo2ft_s_and_read_back_unchanged() {
    let font = designspace::read(DOC, &sources()).unwrap();
    let flags: Vec<(bool, bool, Option<&str>)> = font.instances.iter().map(|i| (i.is_bold, i.is_italic, i.link_style.as_deref())).collect();
    assert_eq!(flags, [(false, false, None), (true, false, None), (false, true, None), (false, false, None)]);
    let written = designspace::write(&font).unwrap();
    assert_eq!(written.document, DOC, "the kept document comes back as read");
}

/// The right edge of glyph `name`'s outline in a compiled font.
fn x_max(bytes: &[u8], order: &[String], name: &str) -> i16 {
    let font = FontRef::new(bytes).unwrap();
    let gid = order.iter().position(|n| n == name).unwrap();
    let glyf = font.glyf().unwrap();
    let loca = font.loca(None).unwrap();
    loca.get_glyf(skrifa::GlyphId::new(gid as u32), &glyf).unwrap().unwrap().x_max()
}

#[test]
fn designspace_rules_swap_glyphs_in_static_instances_and_vary_the_variable_font() {
    let font = designspace::read(DOC, &sources()).unwrap();
    let options = Options { skip_features: true, remove_overlap: false, ..Options::default() };
    // fontmake: Medium (design 500) keeps A; Bold (700, at a master) has A.alt's outline in A.
    let medium = compile(&font, &Target::Instance(0), Flavour::TrueType, &options).unwrap();
    assert_eq!((x_max(&medium.bytes, &medium.glyph_order, "A"), x_max(&medium.bytes, &medium.glyph_order, "A.alt")), (250, 450));
    let bold = compile(&font, &Target::Instance(1), Flavour::TrueType, &options).unwrap();
    assert_eq!((x_max(&bold.bytes, &bold.glyph_order, "A"), x_max(&bold.bytes, &bold.glyph_order, "A.alt")), (650, 350));

    // fontmake's variable font: `rvrn` substitutes A.alt for A from normalized 0.75 (design 600).
    let vf = fontcraft_compile::var::compile_variable(
        &font,
        None,
        &Options { remove_overlap: false, ..Options::default() },
        &fontcraft_compile::var::VariableOptions::default(),
    )
    .unwrap();
    let built = FontRef::new(&vf.compiled.bytes).unwrap();
    let gsub = built.gsub().unwrap();
    let tags: Vec<String> = gsub.feature_list().unwrap().feature_records().iter().map(|r| r.feature_tag().to_string()).collect();
    assert_eq!(tags, ["rvrn"]);
    let fv = gsub.feature_variations().unwrap().unwrap();
    let records = fv.feature_variation_records();
    assert_eq!(records.len(), 1);
    let set = records[0].condition_set(fv.offset_data()).transpose().unwrap().unwrap();
    let ranges: Vec<(u16, f32, f32)> = set
        .conditions()
        .iter()
        .map(|c| match c.unwrap() {
            skrifa::raw::tables::layout::Condition::Format1AxisRange(r) => {
                (r.axis_index(), r.filter_range_min_value().to_f32(), r.filter_range_max_value().to_f32())
            }
            _ => panic!("an axis range"),
        })
        .collect();
    assert_eq!(ranges, [(0, 0.75, 1.0)]);
}
