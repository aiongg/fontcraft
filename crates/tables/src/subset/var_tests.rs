//! Variation tables in a subset, on fonts built in code. The expected values are pyftsubset's
//! (fontTools 4.66.1) on the same fonts built with fontTools' `FontBuilder`: the same glyph
//! order, deltas, delta sets and devices.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_model::{Component, Font, Path, Shape};
use read_fonts::{FontData, FontRead};
use write_fonts::dump_table;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::gpos as wpos;
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar};
use write_fonts::tables::layout as wl;
use write_fonts::tables::variations::Tent;
use write_fonts::types::{F2Dot14, Fixed, GlyphId, GlyphId16, NameId, Tag as WTag};

use super::super::{SubsetOptions, SubsetResult, subset};
use crate::reorder::be::Data;
use crate::reorder::glyph_names;
use crate::sfnt::{SfntFont, read};

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// `.notdef`, `A` (a rectangle), `B` (A as a component, moved 50) and `C` (a rectangle).
fn outlines() -> SfntFont {
    let mut f = Font::new("Var Subset");
    f.glyphs.retain(|g| g.name == ".notdef");
    for (i, (name, cp)) in [("A", 0x41), ("B", 0x42), ("C", 0x43)].into_iter().enumerate() {
        f.add_glyph(name, vec![cp], 500.0 + 10.0 * (i + 1) as f64).unwrap();
        if name != "B" {
            let x = 10.0 * (i + 1) as f64;
            f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x, 0.0, x + 200.0, 700.0)));
        }
    }
    f.glyph_mut("B").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 50.0, 0.0)));
    let out = compile(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    let mut font = read(&out.bytes, 0).unwrap();
    for tag in [*b"GDEF", *b"GSUB", *b"GPOS", *b"kern"] {
        font.remove(tag);
    }
    font
}

fn gid(font: &SfntFont, name: &str) -> u16 {
    glyph_names(font).unwrap().iter().position(|n| n == name).unwrap() as u16
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

/// An item variation store of one-axis regions (start, peak, end) and one subtable of `rows`,
/// every delta written as a word.
fn store(regions: &[(f32, f32, f32)], rows: &[Vec<i16>]) -> Vec<u8> {
    let mut regions_bytes = Vec::new();
    be16(&mut regions_bytes, 1);
    be16(&mut regions_bytes, regions.len() as u16);
    for (s, p, e) in regions {
        for v in [s, p, e] {
            be16(&mut regions_bytes, F2Dot14::from_f32(*v).to_bits() as u16);
        }
    }
    let mut data = Vec::new();
    be16(&mut data, rows.len() as u16);
    be16(&mut data, regions.len() as u16);
    be16(&mut data, regions.len() as u16);
    for r in 0..regions.len() {
        be16(&mut data, r as u16);
    }
    for row in rows {
        for d in row {
            be16(&mut data, *d as u16);
        }
    }
    let mut out = Vec::new();
    be16(&mut out, 1);
    out.extend_from_slice(&12u32.to_be_bytes());
    be16(&mut out, 1);
    out.extend_from_slice(&((12 + regions_bytes.len()) as u32).to_be_bytes());
    out.extend(regions_bytes);
    out.extend(data);
    out
}

/// HVAR with `store` and an optional advance map (format 0, two-byte entries, 16 inner bits).
fn hvar(store: &[u8], advances: Option<&[u32]>) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, 1);
    be16(&mut out, 0);
    out.extend_from_slice(&20u32.to_be_bytes());
    let map_at = 20 + store.len();
    out.extend_from_slice(&(if advances.is_some() { map_at as u32 } else { 0 }).to_be_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(store);
    if let Some(map) = advances {
        // Entry format: 4-byte entries, 16 inner bits.
        out.extend_from_slice(&[0, 0x3F]);
        be16(&mut out, map.len() as u16);
        for v in map {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    out
}

/// The variable font: wght 100–400–900 (axis name 256, instance 650 named 257), gvar deltas
/// for A, B and C, and HVAR with `advances` as its advance map (`None`: rows by glyph id).
fn variable_font(advances: Option<&[u32]>) -> SfntFont {
    let mut font = outlines();
    let r = GlyphDelta::required;
    let tent = |peak: f32| Tent::new(F2Dot14::from_f32(peak), None);
    let (a, b, c) = (gid(&font, "A"), gid(&font, "B"), gid(&font, "C"));
    let gvar = Gvar::new(
        vec![
            GlyphVariations::new(GlyphId::new(0), vec![]),
            GlyphVariations::new(
                GlyphId::new(a.into()),
                vec![GlyphDeltas::new(vec![tent(1.0)], vec![r(-20, 0), r(-20, 50), r(20, 50), r(20, 0), r(0, 0), r(40, 0), r(0, 0), r(0, 0)])],
            ),
            GlyphVariations::new(
                GlyphId::new(b.into()),
                vec![GlyphDeltas::new(vec![tent(1.0)], vec![r(30, 10), r(0, 0), r(60, 0), r(0, 0), r(0, 0)])],
            ),
            GlyphVariations::new(
                GlyphId::new(c.into()),
                vec![GlyphDeltas::new(vec![tent(-1.0)], vec![r(5, 0), r(5, 5), r(-5, 5), r(-5, 0), r(0, 0), r(300, 0), r(0, 0), r(0, 0)])],
            ),
        ],
        1,
    )
    .unwrap();
    let axis =
        VariationAxisRecord::new(WTag::new(b"wght"), Fixed::from_f64(100.0), Fixed::from_f64(400.0), Fixed::from_f64(900.0), 0, NameId::new(256));
    let instance =
        InstanceRecord { subfamily_name_id: NameId::new(257), flags: 0, coordinates: vec![Fixed::from_f64(650.0)], post_script_name_id: None };
    font.insert(*b"fvar", dump_table(&Fvar::new(AxisInstanceArrays::new(vec![axis], vec![instance]))).unwrap());
    font.insert(*b"gvar", dump_table(&gvar).unwrap());
    font.insert(*b"name", name(&[(1, "Sub"), (2, "Regular"), (256, "Weight"), (257, "Semibold"), (300, "Unused")]));
    let regions = [(0.0, 1.0, 1.0), (-1.0, -1.0, 0.0)];
    let table = match advances {
        None => hvar(&store(&regions, &[vec![0, 0], vec![40, -20], vec![60, 0], vec![300, 0]]), None),
        Some(map) => hvar(&store(&regions, &[vec![0, 0], vec![40, 0], vec![300, 0]]), Some(map)),
    };
    font.insert(*b"HVAR", table);
    font
}

fn run(font: &SfntFont, text: &str, edit: impl FnOnce(&mut SubsetOptions)) -> SubsetResult {
    let mut options = SubsetOptions { text: text.into(), ..SubsetOptions::default() };
    edit(&mut options);
    subset(font, &options).unwrap()
}

/// Glyph `g`'s variation data in a gvar table.
fn gvar_data(table: &[u8], g: usize) -> Vec<u8> {
    let d = Data::new(*b"gvar", table);
    let long = d.u16(14).unwrap() & 1 != 0;
    let data_at = d.u32_usize(16).unwrap();
    let off = |i: usize| if long { d.u32_usize(20 + 4 * i).unwrap() } else { 2 * usize::from(d.u16(20 + 2 * i).unwrap()) };
    table[data_at + off(g)..data_at + off(g + 1)].to_vec()
}

/// The same glyph data, up to one byte of padding.
fn same_data(ours: &[u8], theirs: &[u8]) -> bool {
    ours.starts_with(theirs) && ours.len() - theirs.len() <= 1 && ours[theirs.len()..].iter().all(|b| *b == 0)
}

/// The region peaks, and each subtable's region indices and rows, of the store at `at`.
fn store_view(table: &[u8], at: usize) -> (Vec<f32>, Vec<(Vec<u16>, Vec<Vec<i32>>)>) {
    let d = Data::new(*b"HVAR", table);
    let regions_at = at + d.u32_usize(at + 2).unwrap();
    let axes = usize::from(d.u16(regions_at).unwrap());
    let peaks = (0..usize::from(d.u16(regions_at + 2).unwrap()))
        .map(|r| F2Dot14::from_bits(d.i16(regions_at + 4 + 6 * axes * r + 2).unwrap()).to_f32())
        .collect();
    let mut data = Vec::new();
    for i in 0..usize::from(d.u16(at + 6).unwrap()) {
        let v = at + d.u32_usize(at + 8 + 4 * i).unwrap();
        let (items, words, count) = (usize::from(d.u16(v).unwrap()), d.u16(v + 2).unwrap(), usize::from(d.u16(v + 4).unwrap()));
        let (long, wide) = (words & 0x8000 != 0, usize::from(words & 0x7FFF));
        let indices: Vec<u16> = (0..count).map(|r| d.u16(v + 6 + 2 * r).unwrap()).collect();
        let mut p = v + 6 + 2 * count;
        let mut rows = Vec::new();
        for _ in 0..items {
            let mut row = Vec::new();
            for c in 0..count {
                let (value, size) = match (long, c < wide) {
                    (true, true) => (d.u32(p).unwrap() as i32, 4),
                    (true, false) | (false, true) => (i32::from(d.i16(p).unwrap()), 2),
                    (false, false) => (i32::from(d.u8(p).unwrap() as i8), 1),
                };
                row.push(value);
                p += size;
            }
            rows.push(row);
        }
        data.push((indices, rows));
    }
    (peaks, data)
}

/// The regions (peaks), each subtable's region indices and rows, and the advance map of HVAR.
type StoreView = (Vec<f32>, Vec<(Vec<u16>, Vec<Vec<i32>>)>, Option<Vec<u32>>);

fn hvar_view(table: &[u8], glyphs: usize) -> StoreView {
    let d = Data::new(*b"HVAR", table);
    let (peaks, data) = store_view(table, d.u32_usize(4).unwrap());
    let map_at = d.u32_usize(8).unwrap();
    let map = (map_at != 0).then(|| {
        let format = d.u8(map_at + 1).unwrap();
        let (inner_bits, size) = (u32::from(format & 0x0F) + 1, usize::from((format & 0x30) >> 4) + 1);
        let count = usize::from(d.u16(map_at + 2).unwrap());
        (0..glyphs)
            .map(|g| {
                let at = map_at + 4 + size * g.min(count - 1);
                let entry = (0..size).fold(0u32, |acc, k| acc << 8 | u32::from(d.u8(at + k).unwrap()));
                (entry >> inner_bits) << 16 | (entry & ((1 << inner_bits) - 1))
            })
            .collect()
    });
    (peaks, data, map)
}

#[test]
fn subset_variable_font_keeps_glyph_variations_metrics_deltas_and_axis_names() {
    let font = variable_font(None);
    let c = gid(&font, "C") as usize;
    let result = run(&font, "C", |_| {});
    assert_eq!(result.retained, [".notdef", "C"]);
    let out = &result.font;
    for tag in [*b"fvar", *b"name", *b"gvar", *b"HVAR"] {
        assert!(out.get(tag).is_some(), "{} is kept", crate::sfnt::tag_str(tag));
    }
    assert_eq!(out.get(*b"fvar"), font.get(*b"fvar"), "fvar is copied");
    // gvar: one record per output glyph, C's data unchanged, .notdef without variations.
    let gvar = out.get(*b"gvar").unwrap();
    assert_eq!(Data::new(*b"gvar", gvar).u16(12).unwrap(), 2);
    assert!(same_data(&gvar_data(gvar, 1), &gvar_data(font.get(*b"gvar").unwrap(), c)));
    assert!(gvar_data(gvar, 0).is_empty());
    // HVAR without a map: rows for .notdef and C in output order; the column that is zero in
    // both goes, and its region with it.
    let (peaks, data, map) = hvar_view(out.get(*b"HVAR").unwrap(), 2);
    assert_eq!(peaks, [1.0]);
    assert_eq!(data, [(vec![0u16], vec![vec![0], vec![300]])]);
    assert_eq!(map, None);
    // The name table keeps the names fvar uses.
    let n = out.get(*b"name").unwrap();
    let d = Data::new(*b"name", n);
    let ids: Vec<u16> = (0..usize::from(d.u16(2).unwrap())).map(|i| d.u16(6 + 12 * i + 6).unwrap()).collect();
    assert_eq!(ids, [1, 2, 256, 257]);
}

#[test]
fn subset_hvar_advance_map_keeps_used_rows_and_trims_its_tail() {
    // A and B share row 1; C uses row 2.
    let font = variable_font(Some(&[0, 1, 1, 2]));
    let result = run(&font, "B", |_| {});
    assert_eq!(result.retained, [".notdef", "A", "B"]);
    let (peaks, data, map) = hvar_view(result.font.get(*b"HVAR").unwrap(), 3);
    assert_eq!(peaks, [1.0]);
    assert_eq!(data, [(vec![0u16], vec![vec![0], vec![40]])]);
    assert_eq!(map, Some(vec![0, 1, 1]));
    let raw = result.font.get(*b"HVAR").unwrap();
    let map_at = Data::new(*b"HVAR", raw).u32_usize(8).unwrap();
    assert_eq!(Data::new(*b"HVAR", raw).u16(map_at + 2).unwrap(), 2, "the repeated last entry is written once");

    let retained = run(&font, "C", |o| o.retain_gids = true);
    assert_eq!(retained.retained, [".notdef", "A", "B", "C"]);
    let (_, data, map) = hvar_view(retained.font.get(*b"HVAR").unwrap(), 4);
    assert_eq!(data, [(vec![0u16], vec![vec![0], vec![300]])]);
    assert_eq!(map, Some(vec![0, 0, 0, 1]), "emptied glyphs point at row 0");
    let gvar = retained.font.get(*b"gvar").unwrap();
    assert!(gvar_data(gvar, 1).is_empty() && gvar_data(gvar, 2).is_empty(), "emptied glyphs have no variations");
}

#[test]
fn subset_prunes_the_gdef_variation_store_to_the_devices_left() {
    let mut font = outlines();
    let (a, c) = (gid(&font, "A"), gid(&font, "C"));
    // GDEF 1.3 with only a store: rows 5, 10, 15.
    let mut gdef = Vec::new();
    for v in [1u16, 3, 0, 0, 0, 0, 0] {
        be16(&mut gdef, v);
    }
    gdef.extend_from_slice(&18u32.to_be_bytes());
    gdef.extend(store(&[(0.0, 1.0, 1.0)], &[vec![5], vec![10], vec![15]]));
    font.insert(*b"GDEF", gdef);
    let value = |adv: i16, inner: u16| {
        wpos::ValueRecord::new().with_x_advance(adv).with_x_advance_device(wl::DeviceOrVariationIndex::variation_index(0, inner))
    };
    let single = wpos::PositionLookup::Single(wl::Lookup::new(
        wl::LookupFlag::empty(),
        vec![wpos::SinglePos::format_2(wl::CoverageTable::format_1(vec![GlyphId16::new(a), GlyphId16::new(c)]), vec![value(100, 1), value(300, 2)])],
    ));
    let scripts = wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0])), vec![]))]);
    let features = wl::FeatureList::new(vec![wl::FeatureRecord::new(WTag::new(b"kern"), wl::Feature::new(None, vec![0]))]);
    font.insert(*b"GPOS", dump_table(&wpos::Gpos::new(scripts, features, wl::LookupList::new(vec![single]))).unwrap());

    let result = run(&font, "C", |_| {});
    let gdef = result.font.get(*b"GDEF").unwrap();
    let (_, data) = store_view(gdef, Data::new(*b"GDEF", gdef).u32_usize(14).unwrap());
    assert_eq!(data, [(vec![0u16], vec![vec![15]])], "only C's delta set is left");
    let gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(result.font.get(*b"GPOS").unwrap())).unwrap();
    let lookup = gpos.lookup_list().unwrap().lookups().get(0).unwrap();
    let read_fonts::tables::gpos::PositionLookup::Single(l) = lookup else { panic!() };
    let read_fonts::tables::gpos::SinglePos::Format1(s) = l.subtables().get(0).unwrap() else { panic!("C alone is format 1") };
    let device = s.value_record().x_advance_device().unwrap().unwrap();
    let read_fonts::tables::layout::DeviceOrVariationIndex::VariationIndex(v) = device else { panic!() };
    assert_eq!((v.delta_set_outer_index(), v.delta_set_inner_index()), (0, 0), "C's device points at the renumbered row");
}
