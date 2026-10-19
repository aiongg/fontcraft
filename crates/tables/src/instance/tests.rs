//! Full instances of a small variable font built in code. The expected values come from
//! fontTools 4.66.1 `varLib.instancer.instantiateVariableFont` run on the same font built with
//! `fontBuilder` (same glyphs, deltas, avar map, STAT and names).

use write_fonts::NullableOffsetMarker;
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::tables::fvar::InstanceRecord;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, VariationAxisRecord};
use write_fonts::tables::gdef as wgdef;
use write_fonts::tables::gpos as wpos;
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar};
use write_fonts::tables::layout as wl;
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::tables::variations::{RegionAxisCoordinates, Tent, VariationRegion};
use write_fonts::types::{F2Dot14, Fixed, GlyphId, GlyphId16, NameId, Tag as WTag};

use super::glyf::{self, Component, Composite, Glyph, Simple};
use super::*;
use crate::sfnt::Flavour;

const WGHT: Tag = *b"wght";

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn head() -> Vec<u8> {
    let mut h = vec![0u8; 54];
    h[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    h[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    h[18..20].copy_from_slice(&1000u16.to_be_bytes());
    for (at, v) in [(36, 100i16), (38, 0), (40, 550), (42, 700)] {
        h[at..at + 2].copy_from_slice(&v.to_be_bytes());
    }
    h
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut m = vec![0u8; 32];
    m[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[4..6].copy_from_slice(&glyphs.to_be_bytes());
    m
}

fn hhea(long: u16) -> Vec<u8> {
    let mut h = vec![0u8; 36];
    h[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    h[4..6].copy_from_slice(&800i16.to_be_bytes());
    h[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    h[34..36].copy_from_slice(&long.to_be_bytes());
    h
}

fn os2() -> Vec<u8> {
    let mut o = vec![0u8; 96];
    o[0..2].copy_from_slice(&4u16.to_be_bytes());
    o[4..6].copy_from_slice(&400u16.to_be_bytes());
    o[6..8].copy_from_slice(&5u16.to_be_bytes());
    o
}

/// A format 0 `name` table of Windows English records.
fn name(records: &[(u16, &str)]) -> Vec<u8> {
    let mut strings: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    be16(&mut out, 0);
    be16(&mut out, records.len() as u16);
    be16(&mut out, (6 + 12 * records.len()) as u16);
    for (id, text) in records {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        for v in [3, 1, 0x409, *id, bytes.len() as u16, strings.len() as u16] {
            be16(&mut out, v);
        }
        strings.extend(bytes);
    }
    out.extend(strings);
    out
}

/// `STAT` 1.1: one design axis (wght, name 256) and format 1 values 400 (name 2) and 650
/// (name 257); elided fallback name 2.
fn stat() -> Vec<u8> {
    let mut s = Vec::new();
    for v in [1u16, 1, 8, 1] {
        be16(&mut s, v);
    }
    s.extend_from_slice(&20u32.to_be_bytes());
    be16(&mut s, 2);
    s.extend_from_slice(&28u32.to_be_bytes());
    be16(&mut s, 2);
    s.extend_from_slice(b"wght");
    be16(&mut s, 256);
    be16(&mut s, 0);
    // Offsets from the offset array at 28: two offsets (4 bytes), then the tables.
    be16(&mut s, 4);
    be16(&mut s, 16);
    for (value, id) in [(400i32, 2u16), (650, 257)] {
        for v in [1u16, 0, 0, id] {
            be16(&mut s, v);
        }
        s.extend_from_slice(&(value << 16).to_be_bytes());
    }
    s
}

fn tent(peak: f32) -> Tent {
    Tent::new(F2Dot14::from_f32(peak), None)
}

/// `.notdef` (empty), `A` (a square 100,0–500,700, advance 600) and `B` (A offset by 50,
/// advance 700) on a wght axis 100–400–900 whose avar maps 0.5 to 0.25. At wght +1, A's left
/// side moves −20, its top +50 and its advance +40 (two points left to inference); at −1 it
/// narrows by 10 on each side and loses 30 in height and 20 in advance. B's component moves
/// (30, 10) and its advance +60 at +1.
fn variable_font() -> SfntFont {
    let square = Glyph::Simple(Simple {
        bbox: [100, 0, 500, 700],
        end_pts: vec![3],
        instructions: Vec::new(),
        flags: vec![1; 4],
        points: vec![(100.0, 0.0), (100.0, 700.0), (500.0, 700.0), (500.0, 0.0)],
    });
    let composite = Glyph::Composite(Composite {
        bbox: [150, 0, 550, 700],
        components: vec![Component { flags: 0x0006, gid: 1, dx: 50.0, dy: 0.0, points: (0, 0), transform_raw: Vec::new(), transform: None }],
        instructions: None,
    });
    let (glyf_bytes, loca, long) = glyf::write_glyf(&[Glyph::Empty, square, composite]).unwrap();
    let mut head = head();
    head[50..52].copy_from_slice(&u16::from(long).to_be_bytes());
    let mut hmtx = Vec::new();
    for (adv, lsb) in [(500u16, 0i16), (600, 100), (700, 150)] {
        be16(&mut hmtx, adv);
        be16(&mut hmtx, lsb as u16);
    }
    let r = GlyphDelta::required;
    let o = GlyphDelta::optional;
    let gvar = Gvar::new(
        vec![
            GlyphVariations::new(GlyphId::new(0), vec![]),
            GlyphVariations::new(
                GlyphId::new(1),
                vec![
                    GlyphDeltas::new(vec![tent(1.0)], vec![r(-20, 0), o(-20, 50), r(20, 50), o(20, 0), o(0, 0), r(40, 0), o(0, 0), o(0, 0)]),
                    GlyphDeltas::new(vec![tent(-1.0)], vec![r(10, 0), r(10, -30), r(-10, -30), r(-10, 0), r(0, 0), r(-20, 0), r(0, 0), r(0, 0)]),
                ],
            ),
            GlyphVariations::new(GlyphId::new(2), vec![GlyphDeltas::new(vec![tent(1.0)], vec![r(30, 10), o(0, 0), r(60, 0), o(0, 0), o(0, 0)])]),
        ],
        1,
    )
    .unwrap();
    let axis =
        VariationAxisRecord::new(WTag::new(b"wght"), Fixed::from_f64(100.0), Fixed::from_f64(400.0), Fixed::from_f64(900.0), 0, NameId::new(256));
    let instance =
        InstanceRecord { subfamily_name_id: NameId::new(257), flags: 0, coordinates: vec![Fixed::from_f64(650.0)], post_script_name_id: None };
    let fvar = Fvar::new(AxisInstanceArrays::new(vec![axis], vec![instance]));
    let map = |a: i16, b: i16| AxisValueMap::new(F2Dot14::from_bits(a), F2Dot14::from_bits(b));
    let avar = Avar::new(vec![SegmentMaps::new(vec![map(-16384, -16384), map(0, 0), map(8192, 4096), map(16384, 16384)])]);
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());

    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head);
    font.insert(*b"maxp", maxp(3));
    font.insert(*b"hhea", hhea(3));
    font.insert(*b"hmtx", hmtx);
    font.insert(*b"OS/2", os2());
    font.insert(*b"post", post);
    font.insert(*b"loca", loca);
    font.insert(*b"glyf", glyf_bytes);
    font.insert(*b"gvar", write_fonts::dump_table(&gvar).unwrap());
    font.insert(*b"fvar", write_fonts::dump_table(&fvar).unwrap());
    font.insert(*b"avar", write_fonts::dump_table(&avar).unwrap());
    font.insert(*b"name", name(&[(1, "Test"), (2, "Regular"), (256, "Weight"), (257, "Medium")]));
    font.insert(*b"STAT", stat());
    font
}

fn glyphs_of(font: &SfntFont) -> Vec<Glyph> {
    let long = static_tables::u16_at(font.get(*b"head").unwrap(), 50).unwrap() != 0;
    glyf::read_glyf(font.get(*b"glyf").unwrap(), font.get(*b"loca").unwrap(), long, 3).unwrap()
}

fn i16s(font: &SfntFont, tag: &[u8; 4], at: &[usize]) -> Vec<i16> {
    at.iter().map(|a| static_tables::u16_at(font.get(*tag).unwrap(), *a).unwrap() as i16).collect()
}

fn name_ids(font: &SfntFont) -> Vec<u16> {
    let n = font.get(*b"name").unwrap();
    let count = usize::from(static_tables::u16_at(n, 2).unwrap());
    (0..count).map(|i| static_tables::u16_at(n, 6 + 12 * i + 6).unwrap()).collect()
}

#[test]
fn full_instance_between_masters_matches_the_instancer() {
    let font = variable_font();
    let out = instantiate(&font, &InstanceOptions { limits: vec![(WGHT, AxisLimit::Pin(650.0))], ..InstanceOptions::default() }).unwrap();
    for tag in [b"fvar", b"avar", b"gvar"] {
        assert!(out.get(*tag).is_none(), "{} is dropped", String::from_utf8_lossy(tag));
    }
    let glyphs = glyphs_of(&out);
    let Glyph::Simple(a) = &glyphs[1] else { panic!("A stays simple") };
    // 650 normalises to 0.5, which avar maps to 0.25: a quarter of the +1 deltas, the inferred
    // ones included; 712.5 rounds up.
    assert_eq!(a.points, vec![(95.0, 0.0), (95.0, 713.0), (505.0, 713.0), (505.0, 0.0)]);
    assert_eq!(a.bbox, [95, 0, 505, 713]);
    assert_eq!(a.flags[0] & 0x40, 0x40, "the overlap flag is set");
    let Glyph::Composite(b) = &glyphs[2] else { panic!("B stays composite") };
    assert_eq!((b.components[0].dx, b.components[0].dy), (58.0, 3.0), "57.5 and 2.5 round up when written");
    assert_eq!(b.components[0].flags & 0x400, 0x400);
    assert_eq!(b.bbox, [153, 3, 563, 716], "bounds from the fractional offset, rounded once");
    let hmtx = glyf::read_metrics(out.get(*b"hmtx").unwrap(), out.get(*b"hhea").unwrap(), 3).unwrap();
    assert_eq!(hmtx.advances, vec![500, 610, 715]);
    assert_eq!(hmtx.bearings, vec![0, 95, 153]);
    assert_eq!(i16s(&out, b"head", &[36, 38, 40, 42]), vec![95, 0, 563, 716]);
    assert_eq!(i16s(&out, b"hhea", &[10, 12, 14, 16, 34]), vec![715, 95, 105, 563, 3]);
    assert_eq!(i16s(&out, b"OS/2", &[2, 4]), vec![608, 650], "xAvgCharWidth and usWeightClass");
    // STAT keeps the value at 650, so its name stays; the fvar instance's name was the same.
    assert_eq!(static_tables::u16_at(out.get(*b"STAT").unwrap(), 12), Some(1));
    assert_eq!(name_ids(&out), vec![1, 2, 256, 257]);
}

#[test]
fn full_instance_at_a_master_and_at_the_default() {
    let font = variable_font();
    let out = full::full_instance(&font, &[(WGHT, 100.0)]).unwrap();
    let glyphs = glyphs_of(&out);
    let Glyph::Simple(a) = &glyphs[1] else { panic!("A stays simple") };
    assert_eq!(a.points, vec![(110.0, 0.0), (110.0, 670.0), (490.0, 670.0), (490.0, 0.0)]);
    let Glyph::Composite(b) = &glyphs[2] else { panic!("B stays composite") };
    assert_eq!(b.bbox, [160, 0, 540, 670], "B has no tuple at −1 but follows A");
    let hmtx = glyf::read_metrics(out.get(*b"hmtx").unwrap(), out.get(*b"hhea").unwrap(), 3).unwrap();
    assert_eq!(hmtx.advances, vec![500, 580, 700]);
    assert_eq!(hmtx.bearings, vec![0, 110, 160]);
    assert_eq!(i16s(&out, b"hhea", &[10, 12, 14, 16]), vec![700, 110, 90, 540]);
    assert_eq!(i16s(&out, b"OS/2", &[2, 4]), vec![593, 100]);
    assert_eq!(static_tables::u16_at(out.get(*b"STAT").unwrap(), 12), Some(0), "no STAT value holds at 100");
    assert_eq!(name_ids(&out), vec![1, 2, 256], "the name only fvar and the dropped STAT value used goes");

    let default = full::full_instance(&font, &[]).unwrap();
    let glyphs = glyphs_of(&default);
    let Glyph::Simple(a) = &glyphs[1] else { panic!("A stays simple") };
    assert_eq!(a.points, vec![(100.0, 0.0), (100.0, 700.0), (500.0, 700.0), (500.0, 0.0)]);
    assert_eq!(i16s(&default, b"OS/2", &[4]), vec![400]);
}

#[test]
fn instancing_refuses_bad_locations_and_clamps_to_the_range() {
    let font = variable_font();
    let err = full::full_instance(&font, &[(*b"wdth", 100.0)]).unwrap_err();
    assert!(matches!(err, TablesError::BadOptions(_)), "{err}");
    assert!(full::full_instance(&font, &[(WGHT, f64::NAN)]).is_err());
    assert!(full::full_instance(&font, &[(WGHT, 400.0), (WGHT, 500.0)]).is_err());
    let clamped = pin_location(&font, &[(WGHT, 2000.0)]).unwrap();
    assert_eq!(clamped.user, vec![(WGHT, 900.0)]);
    assert_eq!(clamped.normalized, vec![1.0]);
    let static_font = full::full_instance(&font, &[]).unwrap();
    assert!(matches!(full::full_instance(&static_font, &[]), Err(TablesError::MissingTable(_))), "a static font has nothing to instance");
}

#[test]
fn named_instances_list_their_names_and_coordinates() {
    let list = named_instances(&variable_font()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].coordinates, vec![(WGHT, 650.0)]);
    assert_eq!(list[0].subfamily_name.as_deref(), Some("Medium"));
    assert_eq!(list[0].postscript_name, None);
}

#[test]
fn gpos_variation_deltas_are_applied_and_gdef_goes_when_left_empty() {
    let mut font = variable_font();
    let mut builder = VariationStoreBuilder::new(1);
    let region = VariationRegion::new(vec![RegionAxisCoordinates::new(F2Dot14::from_f32(0.0), F2Dot14::from_f32(1.0), F2Dot14::from_f32(1.0))]);
    let temp = builder.add_deltas(vec![(region, 100)]);
    let (store, remap) = builder.build();
    let index = remap.get(temp).unwrap();
    let record = wpos::ValueRecord::new().with_x_advance(10).with_x_advance_device(wl::DeviceOrVariationIndex::VariationIndex(
        wl::VariationIndex::new(index.delta_set_outer_index, index.delta_set_inner_index),
    ));
    let single = wpos::PositionLookup::Single(wl::Lookup::new(
        wl::LookupFlag::empty(),
        vec![wpos::SinglePos::format_1(wl::CoverageTable::format_1(vec![GlyphId16::new(1)]), record)],
    ));
    let features = wl::FeatureList::new(vec![wl::FeatureRecord::new(WTag::new(b"kern"), wl::Feature::new(None, vec![0]))]);
    let scripts = wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0])), vec![]))]);
    let gpos = wpos::Gpos::new(scripts, features, wl::LookupList::new(vec![single]));
    let mut gdef = wgdef::Gdef::new(None, None, None, None);
    gdef.item_var_store = NullableOffsetMarker::new(Some(store));
    font.insert(*b"GPOS", write_fonts::dump_table(&gpos).unwrap());
    font.insert(*b"GDEF", write_fonts::dump_table(&gdef).unwrap());

    let out = full::full_instance(&font, &[(WGHT, 650.0)]).unwrap();
    assert!(out.get(*b"GDEF").is_none(), "GDEF held only the variation store");
    let read = read_fonts::tables::gpos::Gpos::read(FontData::new(out.get(*b"GPOS").unwrap())).unwrap();
    let gpos: wpos::Gpos = read.to_owned_table();
    let wpos::PositionLookup::Single(lookup) = &*gpos.lookup_list.lookups[0] else { panic!("a single adjustment") };
    let wpos::SinglePos::Format1(t) = &*lookup.subtables[0] else { panic!("format 1") };
    assert_eq!(t.value_record.x_advance, Some(35), "10 plus a quarter of 100");
    assert!(t.value_record.x_advance_device.is_none());
}

#[test]
fn a_doubling_chain_of_composites_over_an_empty_glyph_is_refused() {
    // Each composite uses the previous one twice, scaled so that its bounds come from the
    // flattened points: 2^64 component visits without a shared budget.
    let mut glyphs = vec![Glyph::Empty, Glyph::Empty];
    for g in 1..=64u16 {
        let comp = Component {
            flags: 0x000A,
            gid: g,
            dx: 0.0,
            dy: 0.0,
            points: (0, 0),
            transform_raw: vec![0x40, 0],
            transform: Some([1.0, 0.0, 0.0, 1.0]),
        };
        glyphs.push(Glyph::Composite(Composite { bbox: [0; 4], components: vec![comp.clone(), comp], instructions: None }));
    }
    let (glyf_bytes, loca, long) = glyf::write_glyf(&glyphs).unwrap();
    let mut glyphs = glyf::read_glyf(&glyf_bytes, &loca, long, glyphs.len()).unwrap();
    let gvar = Gvar::new((0..glyphs.len() as u32).map(|g| GlyphVariations::new(GlyphId::new(g), vec![])).collect(), 1).unwrap();
    let gvar = write_fonts::dump_table(&gvar).unwrap();
    let mut hmtx = glyf::Metrics { advances: vec![0; glyphs.len()], bearings: vec![0; glyphs.len()] };
    let err = glyf::apply_gvar(&mut glyphs, &gvar, 1, &[0.0], &mut hmtx, None).unwrap_err();
    assert!(matches!(err, TablesError::TooLarge { .. }), "{err}");
}

#[test]
fn full_instance_recalculates_maxp_and_the_head_flags_as_the_instancer_saves() {
    // The input's maxp maxima and head flags are all zero.
    let font = variable_font();
    let pin = |recalc_bounds| InstanceOptions { limits: vec![(WGHT, AxisLimit::Pin(650.0))], recalc_bounds, ..InstanceOptions::default() };
    let out = instantiate(&font, &pin(true)).unwrap();
    assert_eq!(i16s(&out, b"maxp", &[6, 8, 10, 12, 28, 30]), vec![4, 1, 4, 1, 1, 1]);
    assert_eq!(i16s(&out, b"head", &[16])[0] & 2, 2, "every left side bearing equals xMin");
    let kept = instantiate(&font, &pin(false)).unwrap();
    assert_eq!(kept.get(*b"maxp"), font.get(*b"maxp"), "without bounds recalculation fontTools leaves maxp alone");
    assert_eq!(i16s(&kept, b"head", &[16])[0] & 2, 0);
}
