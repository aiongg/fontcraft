//! Expected values come from fontTools 4.66 (`varLib.avar.map.map`, `varLib.avar.build.build`)
//! run on the same `fvar` and `avar` data built with fontTools.

use write_fonts::NullableOffsetMarker;
use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, VariationAxisRecord};
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::tables::variations::{DeltaSetIndexMap, RegionAxisCoordinates, VariationRegion};
use write_fonts::types::{F2Dot14, Fixed, NameId, Tag as WTag};

use super::*;
use crate::sfnt::Flavour;

const WGHT: Tag = *b"wght";
const WDTH: Tag = *b"wdth";

fn fvar() -> Vec<u8> {
    let axis = |tag: &[u8; 4], lo: f64, de: f64, hi: f64| {
        VariationAxisRecord::new(WTag::new(tag), Fixed::from_f64(lo), Fixed::from_f64(de), Fixed::from_f64(hi), 0, NameId::new(256))
    };
    let axes = vec![axis(b"wght", 100.0, 400.0, 900.0), axis(b"wdth", 75.0, 100.0, 125.0)];
    write_fonts::dump_table(&Fvar::new(AxisInstanceArrays::new(axes, Vec::new()))).unwrap()
}

fn segments(pairs: &[(i16, i16)]) -> SegmentMaps {
    SegmentMaps::new(pairs.iter().map(|&(a, b)| AxisValueMap::new(F2Dot14::from_bits(a), F2Dot14::from_bits(b))).collect())
}

/// wght: −0.5 → −0.70001…, 0.4 → 0.6; wdth: the identity.
fn v1_maps() -> Vec<SegmentMaps> {
    vec![segments(&[(-16384, -16384), (-8192, -11469), (0, 0), (6554, 9830), (16384, 16384)]), segments(&[(-16384, -16384), (0, 0), (16384, 16384)])]
}

fn font_with(avar: Option<Vec<u8>>) -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(FVAR, fvar());
    if let Some(avar) = avar {
        font.insert(AVAR, avar);
    }
    font
}

fn v1_font() -> SfntFont {
    font_with(Some(write_fonts::dump_table(&Avar::new(v1_maps())).unwrap()))
}

/// The v1 maps plus a variation store: at wght 1 the wdth axis moves by −0.25; at wdth −1 the
/// wght axis moves by 0.125.
fn v2_font() -> SfntFont {
    let f = |v: f64| F2Dot14::from_f32(v as f32);
    let region = |wght: (f64, f64, f64), wdth: (f64, f64, f64)| {
        VariationRegion::new(vec![
            RegionAxisCoordinates::new(f(wght.0), f(wght.1), f(wght.2)),
            RegionAxisCoordinates::new(f(wdth.0), f(wdth.1), f(wdth.2)),
        ])
    };
    let mut builder = VariationStoreBuilder::new(2);
    let for_wght = builder.add_deltas(vec![(region((0.0, 0.0, 0.0), (-1.0, -1.0, 0.0)), 2048)]);
    let for_wdth = builder.add_deltas(vec![(region((0.0, 1.0, 1.0), (0.0, 0.0, 0.0)), -4096)]);
    let (store, remap) = builder.build();
    let index = |id| remap.get(id).map(|v| (u32::from(v.delta_set_outer_index) << 16) | u32::from(v.delta_set_inner_index)).unwrap();
    let map: DeltaSetIndexMap = [index(for_wght), index(for_wdth)].into_iter().collect();
    let mut avar = Avar::new(v1_maps());
    avar.axis_index_map = NullableOffsetMarker::new(Some(map));
    avar.var_store = NullableOffsetMarker::new(Some(store));
    font_with(Some(write_fonts::dump_table(&avar).unwrap()))
}

fn value(list: &[(Tag, f64)], tag: Tag) -> f64 {
    list.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v).unwrap()
}

fn mapped(font: &SfntFont, location: &[(Tag, f64)], from: Space) -> MappedLocation {
    map_coordinates(font, &[location.to_vec()], from).unwrap().pop().unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

#[test]
fn map_through_avar_version_1_matches_fonttools() {
    let font = v1_font();
    let m = mapped(&font, &[(WGHT, 250.0)], Space::User);
    assert_eq!(value(&m.normalized, WGHT), -0.5);
    assert_eq!(value(&m.mapped, WGHT), -0.70001220703125);
    let axes = fvar_axes(&font).unwrap();
    assert!(close(denormalize_value(value(&m.mapped, WGHT), &axes[0]), 189.996337890625));
    assert_eq!(value(&m.mapped, WDTH), 0.0, "an axis not named stays at the default");
    assert_eq!(value(&m.user, WDTH), 100.0);

    let m = mapped(&font, &[(WGHT, 650.0), (WDTH, 90.0)], Space::User);
    assert!(close(value(&m.mapped, WGHT), 0.6666327568667345));
    assert!(close(denormalize_value(value(&m.mapped, WGHT), &axes[0]), 733.3163784333673));
    assert!(close(value(&m.mapped, WDTH), -0.4));

    let m = mapped(&font, &[(WGHT, 1000.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 1.0, "user values are clamped to the axis range");

    let m = mapped(&font, &[(WGHT, -0.25)], Space::Normalized);
    assert!(close(value(&m.mapped, WGHT), -0.350006103515625));
    assert_eq!(value(&m.user, WGHT), 325.0);
}

#[test]
fn map_through_avar_version_2_matches_fonttools() {
    let font = v2_font();
    let m = mapped(&font, &[(WGHT, 650.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 0.6666259765625, "version 2 quantises");
    assert_eq!(value(&m.mapped, WDTH), -0.16668701171875, "an axis not named moves");

    let m = mapped(&font, &[(WGHT, 650.0), (WDTH, 90.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 0.71661376953125);
    assert_eq!(value(&m.mapped, WDTH), -0.56671142578125);

    let m = mapped(&font, &[(WDTH, 75.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 0.125);
    assert_eq!(value(&m.mapped, WDTH), -1.0);

    let m = mapped(&font, &[(WGHT, 400.0), (WDTH, 110.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 0.0);
    assert_eq!(value(&m.mapped, WDTH), 0.4000244140625);

    let m = mapped(&font, &[(WGHT, 0.5), (WDTH, -0.5)], Space::Normalized);
    assert_eq!(value(&m.mapped, WGHT), 0.7291259765625);
    assert_eq!(value(&m.mapped, WDTH), -0.66668701171875);
    assert_eq!(avar_version(&font).unwrap(), Some(2));
}

#[test]
fn mapped_coordinates_map_back_through_avar_version_1() {
    let font = v1_font();
    let forward = mapped(&font, &[(WGHT, 650.0)], Space::User);
    let back = mapped(&font, &[(WGHT, value(&forward.mapped, WGHT))], Space::Mapped);
    assert!(close(value(&back.normalized, WGHT), 0.5));
    assert!(close(value(&back.user, WGHT), 650.0));
    assert!(matches!(map_coordinates(&v2_font(), &[vec![(WGHT, 0.5)]], Space::Mapped), Err(TablesError::Unsupported(_))));
}

#[test]
fn a_font_without_avar_maps_by_normalisation_only() {
    let m = mapped(&font_with(None), &[(WDTH, 112.5)], Space::User);
    assert_eq!(value(&m.mapped, WDTH), 0.5);
}

#[test]
fn bad_locations_and_damaged_tables_are_refused() {
    let font = v1_font();
    assert!(matches!(map_coordinates(&font, &[vec![(*b"opsz", 12.0)]], Space::User), Err(TablesError::BadOptions(_))));
    assert!(matches!(map_coordinates(&font, &[vec![(WGHT, f64::NAN)]], Space::User), Err(TablesError::BadOptions(_))));
    assert!(matches!(map_coordinates(&font, &[vec![(WGHT, 1.0), (WGHT, 2.0)]], Space::User), Err(TablesError::BadOptions(_))));
    assert!(matches!(map_coordinates(&SfntFont::new(Flavour::TrueType), &[], Space::User), Err(TablesError::MissingTable(_))));

    let mut bytes = write_fonts::dump_table(&Avar::new(v1_maps())).unwrap();
    bytes[1] = 3;
    assert!(matches!(map_coordinates(&font_with(Some(bytes)), &[vec![(WGHT, 500.0)]], Space::User), Err(TablesError::Unsupported(_))));
    let mut cut = write_fonts::dump_table(&Avar::new(v1_maps())).unwrap();
    cut.truncate(12);
    assert!(map_coordinates(&font_with(Some(cut)), &[vec![(WGHT, 500.0)]], Space::User).is_err());
    // A version 2 header whose offsets point past the end.
    let mut v2 = write_fonts::dump_table(&Avar::new(v1_maps())).unwrap();
    v2[1] = 2;
    v2.extend_from_slice(&[0xFF, 0, 0, 0, 0xFF, 0, 0, 0]);
    assert!(map_coordinates(&font_with(Some(v2)), &[vec![(WGHT, 500.0)]], Space::User).is_err());
}

const DS1: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<!-- a comment with <axis> inside -->
<designspace format="5.0">
  <axes>
    <axis tag="wght" name="Weight &amp; more" minimum="100" default="400" maximum="900">
      <map input="100" output="20"/>
      <map input="400" output="80"/>
      <map input='700' output='150'/>
      <map input="900" output="200"/>
    </axis>
    <axis tag="wdth" name="Width" minimum="75" default="100" maximum="125"/>
    <axis tag="ital" name="Italic" values="0 1" default="0"/>
  </axes>
  <sources>
    <source filename="missing.ufo"><location><dimension name="Width" xvalue="1"/></location></source>
  </sources>
</designspace>
"#;

const DS2: &str = r#"<designspace format="5.0">
  <axes>
    <axis tag="wght" name="Weight" minimum="100" default="400" maximum="900">
      <map input="100" output="20"/>
      <map input="400" output="80"/>
      <map input="900" output="200"/>
    </axis>
    <axis tag="wdth" name="Width" minimum="75" default="100" maximum="125"/>
    <mappings>
      <mapping>
        <input><dimension name="Weight" xvalue="200"/><dimension name="Width" xvalue="125"/></input>
        <output><dimension name="Width" xvalue="115"/></output>
      </mapping>
    </mappings>
  </axes>
</designspace>"#;

fn raw_segments(font: &SfntFont, tag: Tag) -> Vec<(i16, i16)> {
    let maps = segment_maps(font).unwrap();
    let pairs = &maps.iter().find(|(t, _)| *t == tag).unwrap().1;
    pairs.iter().map(|&(a, b)| (f2dot14(a).to_bits(), f2dot14(b).to_bits())).collect()
}

#[test]
fn designspace_axes_become_avar_options() {
    let options = avar_options_from_designspace(DS1).unwrap();
    assert_eq!(options.maps, vec![(WGHT, vec![(100.0, 20.0), (400.0, 80.0), (700.0, 150.0), (900.0, 200.0)])], "the discrete axis is left out");
    assert!(options.mappings.is_empty());
    let cross = avar_options_from_designspace(DS2).unwrap().mappings;
    assert_eq!(cross, vec![CrossAxisMapping { input: vec![(WGHT, 200.0), (WDTH, 125.0)], output: vec![(WDTH, 115.0)] }]);

    for broken in [
        "<designspace><axes><axis tag=\"wght\" name=\"W\"",
        "<designspace><axes></axis></designspace>",
        "<fontinfo/>",
        "<designspace><axes><axis tag=wght/></axes></designspace>",
        "<designspace><axes><axis tag=\"wght\"><map input=\"x\" output=\"1\"/></axis></axes></designspace>",
        "<designspace><axes><mappings><mapping><input><dimension name=\"Nope\" xvalue=\"1\"/></input></mapping></mappings></axes></designspace>",
        "<designspace>&bogus;<axes/>",
    ] {
        assert!(avar_options_from_designspace(broken).is_err(), "{broken}");
    }
}

#[test]
fn build_avar_from_a_designspace_matches_fonttools() {
    let options = avar_options_from_designspace(DS1).unwrap();
    let built = build_avar(&font_with(None), &options).unwrap();
    assert_eq!(raw_segments(&built, WGHT), vec![(-16384, -16384), (0, 0), (9830, 9557), (16384, 16384)]);
    assert_eq!(raw_segments(&built, WDTH), vec![(-16384, -16384), (0, 0), (16384, 16384)]);
    assert_eq!(avar_version(&built).unwrap(), Some(1));
    let m = mapped(&built, &[(WGHT, 700.0)], Space::User);
    assert!(close(value(&m.mapped, WGHT), 0.5833384192859322));
    let m = mapped(&built, &[(WGHT, 550.0), (WDTH, 110.0)], Space::User);
    assert!(close(value(&m.mapped, WGHT), 0.29166836215666325));
    assert!(close(value(&m.mapped, WDTH), 0.4));

    // Rebuilt from its own table, the font keeps the same maps.
    let again = build_avar(&built, &avar_options_from_font(&built).unwrap()).unwrap();
    assert_eq!(again.get(AVAR), built.get(AVAR));

    let v2 = build_avar(&font_with(None), &AvarOptions { version2: true, ..options }).unwrap();
    assert_eq!(avar_version(&v2).unwrap(), Some(2));
    assert_eq!(raw_segments(&v2, WGHT), raw_segments(&built, WGHT));
}

#[test]
fn identity_maps_leave_no_avar_table() {
    let options = AvarOptions { maps: vec![(WGHT, vec![(100.0, 20.0), (400.0, 80.0), (900.0, 200.0)])], ..AvarOptions::default() };
    let built = build_avar(&v1_font(), &options).unwrap();
    assert_eq!(built.get(AVAR), None, "the old table is removed and none is written");
}

#[test]
fn build_avar_refuses_maps_fonttools_refuses() {
    let font = font_with(None);
    let with = |pairs: Vec<(f64, f64)>| build_avar(&font, &AvarOptions { maps: vec![(WGHT, pairs)], ..AvarOptions::default() });
    for (pairs, why) in [
        (vec![(400.0, 80.0), (900.0, 200.0)], "no point at the minimum"),
        (vec![(100.0, 20.0), (400.0, 80.0)], "no point at the maximum"),
        (vec![(100.0, 20.0), (500.0, 80.0), (900.0, 200.0)], "no point at the default"),
        (vec![(100.0, 20.0), (400.0, 80.0), (700.0, 60.0), (900.0, 200.0)], "outputs that go down"),
        (vec![(100.0, 20.0), (400.0, 80.0), (400.0, 90.0), (900.0, 200.0)], "an input with two outputs"),
        (vec![(100.0, 20.0), (400.0, f64::INFINITY), (900.0, 200.0)], "a value that is not finite"),
    ] {
        assert!(matches!(with(pairs), Err(TablesError::BadOptions(_))), "{why}");
    }
    let unknown = AvarOptions { maps: vec![(*b"opsz", vec![(6.0, 6.0)])], ..AvarOptions::default() };
    assert!(matches!(build_avar(&font, &unknown), Err(TablesError::BadOptions(_))));
}

#[test]
#[ignore = "needs VariationModel::regions and ::deltas (M9.5)"]
fn build_avar_version_2_from_cross_axis_mappings_matches_fonttools() {
    let built = build_avar(&font_with(None), &avar_options_from_designspace(DS2).unwrap()).unwrap();
    assert_eq!(avar_version(&built).unwrap(), Some(2));
    assert_eq!(raw_segments(&built, WGHT), vec![(-16384, -16384), (0, 0), (16384, 16384)]);
    let m = mapped(&built, &[(WGHT, 900.0), (WDTH, 125.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 1.0);
    assert_eq!(value(&m.mapped, WDTH), 0.5999755859375);
    let m = mapped(&built, &[(WGHT, 550.0), (WDTH, 110.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 0.29998779296875);
    assert_eq!(value(&m.mapped, WDTH), 0.35205078125);
    let m = mapped(&built, &[(WGHT, 700.0)], Space::User);
    assert_eq!(value(&m.mapped, WGHT), 0.5999755859375);
    assert_eq!(value(&m.mapped, WDTH), 0.0);
}

/// fontTools 4.66 `varLib.avar.unbuild` printed these on the same `fvar` and `avar` data.
#[test]
fn avar_designspace_matches_fonttools_unbuild() {
    let v1 = avar_designspace(&v1_font()).unwrap();
    assert_eq!(
        v1,
        "<?xml version='1.0' encoding='UTF-8'?>\n\
         <designspace format=\"5.1\">\n  <axes>\n\
         \x20   <axis tag=\"wght\" name=\"wght\" minimum=\"100\" maximum=\"900\" default=\"400\">\n\
         \x20     <map input=\"100\" output=\"100\"/>\n\
         \x20     <map input=\"250\" output=\"189.996337890625\"/>\n\
         \x20     <map input=\"400\" output=\"400\"/>\n\
         \x20     <map input=\"600.01220703125\" output=\"699.98779296875\"/>\n\
         \x20     <map input=\"900\" output=\"900\"/>\n\
         \x20   </axis>\n\
         \x20   <axis tag=\"wdth\" name=\"wdth\" minimum=\"75\" maximum=\"125\" default=\"100\"/>\n\
         \x20 </axes>\n</designspace>\n"
    );
    // The mappings come in the order of the store's regions, which the builder chooses.
    let v2 = avar_designspace(&v2_font()).unwrap();
    let block = |input: &str, output: &str| {
        format!(
            "      <mapping>\n        <input>\n          {input}\n        </input>\n        <output>\n          {output}\n        </output>\n      </mapping>\n"
        )
    };
    assert!(v2.contains(&block("<dimension name=\"wdth\" xvalue=\"75\"/>", "<dimension name=\"wght\" xvalue=\"462.5\"/>")), "{v2}");
    assert!(v2.contains(&block("<dimension name=\"wght\" xvalue=\"900\"/>", "<dimension name=\"wdth\" xvalue=\"93.75\"/>")), "{v2}");
    assert_eq!(v2.matches("<mapping>").count(), 2, "the default location moves nothing and is left out: {v2}");
    assert!(v2.contains("    </mappings>\n  </axes>\n</designspace>\n"), "{v2}");
    assert!(matches!(avar_designspace(&SfntFont::new(Flavour::TrueType)), Err(TablesError::MissingTable(_))));
}
