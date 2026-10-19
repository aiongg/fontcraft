//! Table patches on fonts built in code.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use read_fonts::types::{Fixed, GlyphId, GlyphId16, NameId, Tag as RTag};
use read_fonts::{FontData, FontRead};
use write_fonts::tables::cmap::Cmap as WCmap;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::gpos::{Gpos, PositionLookup, PositionLookupList, PositionSequenceContext};
use write_fonts::tables::layout::{
    CoverageTable, FeatureList, Lookup, LookupFlag, ScriptList, SequenceContext, SequenceContextFormat3, SequenceLookupRecord,
};
use write_fonts::tables::name::{Name, NameRecord};

use super::*;
use crate::sfnt::Flavour;

fn dump<T: write_fonts::FontWrite + write_fonts::validate::Validate>(table: &T) -> Vec<u8> {
    write_fonts::dump_table(table).unwrap()
}

fn with(tables: Vec<(&[u8; 4], Vec<u8>)>) -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    for (tag, data) in tables {
        font.insert(*tag, data);
    }
    font
}

fn mappings(font: &SfntFont) -> Vec<(u16, u16, Vec<(u32, u32)>)> {
    let cmap = read_fonts::tables::cmap::Cmap::read(FontData::new(font.get(*b"cmap").unwrap())).unwrap();
    cmap.encoding_records()
        .iter()
        .map(|r| {
            let sub = r.subtable(cmap.offset_data()).unwrap();
            let m = if matches!(sub, read_fonts::tables::cmap::CmapSubtable::Format14(_)) {
                Vec::new()
            } else {
                sub.iter().map(|(c, g)| (c, g.to_u32())).collect()
            };
            (r.platform_id() as u16, r.encoding_id(), m)
        })
        .collect()
}

#[test]
fn rebuild_cmap_writes_format_4_and_12_from_the_union() {
    // A Windows BMP subtable only; a code point beyond the BMP lives in a (0, 4) subtable alone.
    let full =
        WCmap::from_mappings([('A', GlyphId::new(1)), ('B', GlyphId::new(5)), ('C', GlyphId::new(2)), ('\u{1F600}', GlyphId::new(3))]).unwrap();
    let font = with(vec![(b"cmap", dump(&full))]);
    let r = rebuild_cmap(&font).unwrap();
    assert_eq!(r.mappings, 4);
    let got = mappings(&r.font);
    let keys: Vec<(u16, u16)> = got.iter().map(|(p, e, _)| (*p, *e)).collect();
    assert_eq!(keys, [(0, 3), (0, 4), (3, 1), (3, 10)]);
    assert_eq!(got[0].2, [(0x41, 1), (0x42, 5), (0x43, 2)], "format 4 maps the BMP, glyph array included");
    assert_eq!(got[1].2.len(), 4, "format 12 maps everything");

    let bmp = WCmap::from_mappings([('A', GlyphId::new(1)), ('B', GlyphId::new(2))]).unwrap();
    let r = rebuild_cmap(&with(vec![(b"cmap", dump(&bmp))])).unwrap();
    let keys: Vec<(u16, u16)> = mappings(&r.font).iter().map(|(p, e, _)| (*p, *e)).collect();
    assert_eq!(keys, [(0, 3), (3, 1)], "no format 12 without code points beyond the BMP");
}

fn name_font(records: Vec<(u16, u16, u16, u16, &str)>) -> SfntFont {
    let mut recs: Vec<NameRecord> =
        records.into_iter().map(|(p, e, l, id, s)| NameRecord::new(p, e, l, NameId::new(id), s.to_string().into())).collect();
    recs.sort();
    with(vec![(b"name", dump(&Name::new(recs)))])
}

fn names(font: &SfntFont) -> Vec<(u16, u16, String)> {
    let name = read_fonts::tables::name::Name::read(FontData::new(font.get(*b"name").unwrap())).unwrap();
    name.name_record().iter().map(|r| (r.platform_id(), r.name_id().to_u16(), r.string(name.string_data()).unwrap().chars().collect())).collect()
}

#[test]
fn clean_name_records_trims_drops_and_agrees() {
    let font = name_font(vec![
        (1, 0, 0, 1, "Mac Family"),
        (3, 1, 0x409, 1, "  My   Family "),
        (3, 1, 0x409, 2, "Regular\n"),
        (3, 1, 0x409, 3, "   "),
        (3, 1, 0x409, 6, "MyFamily-Regular"),
        (3, 1, 0x407, 6, "Other-Name"),
    ]);
    let r = clean_name_records(&font, &NameOptions::default()).unwrap();
    assert_eq!(r.removed, 2, "the Mac record and the empty one");
    assert_eq!(r.changed, 3);
    assert_eq!(
        names(&r.font),
        [(3, 1, "My Family".into()), (3, 2, "Regular".into()), (3, 6, "MyFamily-Regular".into()), (3, 6, "MyFamily-Regular".into())]
    );
    let keep_mac = clean_name_records(&font, &NameOptions { drop_mac: false }).unwrap();
    assert_eq!(names(&keep_mac.font).first(), Some(&(1, 1, "Mac Family".into())));
}

fn fvar_font(instances: Vec<(u16, f64, Option<u16>)>) -> SfntFont {
    let axis =
        VariationAxisRecord::new(RTag::new(b"wght"), Fixed::from_f64(100.0), Fixed::from_f64(400.0), Fixed::from_f64(900.0), 0, NameId::new(256));
    let records = instances
        .into_iter()
        .map(|(sub, w, ps)| InstanceRecord {
            subfamily_name_id: NameId::new(sub),
            coordinates: vec![Fixed::from_f64(w)],
            flags: 0,
            post_script_name_id: ps.map(NameId::new),
        })
        .collect();
    let mut font = name_font(vec![(3, 1, 0x409, 2, "Regular"), (3, 1, 0x409, 6, "F-Regular")]);
    font.insert(*b"fvar", dump(&Fvar::new(AxisInstanceArrays::new(vec![axis], records))));
    font
}

#[test]
fn clean_instances_sorts_dedupes_and_names_the_default() {
    let font = fvar_font(vec![(258, 700.0, Some(259)), (257, 400.0, Some(260)), (258, 700.0, Some(259)), (261, 100.0, Some(0xFFFF))]);
    let r = clean_instances(&font).unwrap();
    assert_eq!(r.removed, 1);
    assert!(r.reordered);
    assert!(r.dropped_postscript_names, "one record lacks a PostScript name, so none keep one");
    let fvar = read_fonts::tables::fvar::Fvar::read(FontData::new(r.font.get(*b"fvar").unwrap())).unwrap();
    let got: Vec<(u16, f64, bool)> = fvar
        .instances()
        .unwrap()
        .iter()
        .map(|i| {
            let i = i.unwrap();
            (i.subfamily_name_id.to_u16(), i.coordinates.first().unwrap().get().to_f64(), i.post_script_name_id.is_some())
        })
        .collect();
    assert_eq!(got, [(261, 100.0, false), (2, 400.0, false), (258, 700.0, false)]);
    let again = clean_instances(&r.font).unwrap();
    assert_eq!(again.font, r.font, "a clean table is left as it is");
}

#[test]
fn recompile_layout_turns_contextual_into_chained_positioning() {
    let rule = SequenceContextFormat3::new(vec![CoverageTable::format_1(vec![GlyphId16::new(1)])], vec![SequenceLookupRecord::new(0, 1)]);
    let ctx = PositionSequenceContext::from(SequenceContext::Format3(rule));
    let lookups = PositionLookupList::new(vec![PositionLookup::Contextual(Lookup::new(LookupFlag::empty(), vec![ctx]))]);
    let gpos = Gpos::new(ScriptList::new(vec![]), FeatureList::new(vec![]), lookups);
    let font = with(vec![(b"GPOS", dump(&gpos))]);
    let (out, converted) = recompile_layout(&font).unwrap();
    assert_eq!(converted, [0]);
    let read = read_fonts::tables::gpos::Gpos::read(FontData::new(out.get(*b"GPOS").unwrap())).unwrap();
    let lookup = read.lookup_list().unwrap().lookups().get(0).unwrap();
    assert_eq!(lookup.lookup_type(), 8);
    let (again, none) = recompile_layout(&out).unwrap();
    assert!(none.is_empty());
    assert_eq!(again, out);
}

#[test]
fn patch_table_and_os2_upgrade_write_fields_in_place() {
    let mut os2 = vec![0u8; 78];
    os2[2] = 0x01; // xAvgCharWidth 256
    let font = with(vec![(b"OS/2", os2.clone())]);
    let patched = patch_table(&font, &PatchOptions { table: *b"OS/2", fields: vec![("xAvgCharWidth".into(), PatchValue::Number(500.0))] }).unwrap();
    assert_eq!(read_field(&patched, *b"OS/2", "xAvgCharWidth").unwrap(), PatchValue::Number(500.0));
    assert!(
        patch_table(&font, &PatchOptions { table: *b"OS/2", fields: vec![("sxHeight".into(), PatchValue::Number(1.0))] }).is_err(),
        "version 0 has no sxHeight"
    );
    assert!(patch_table(&font, &PatchOptions { table: *b"OS/2", fields: vec![("xAvgCharWidth".into(), PatchValue::Number(0.5))] }).is_err());

    let fill = Os2Upgrade { code_page_range: [1, 0], x_height: 500, cap_height: 700, default_char: 0, break_char: 32, max_context: 2 };
    let up = raise_os2_version(&font, 4, &fill).unwrap();
    let data = up.get(*b"OS/2").unwrap();
    assert_eq!(data.len(), 96);
    assert_eq!(read_field(&up, *b"OS/2", "sCapHeight").unwrap(), PatchValue::Number(700.0));
    assert_eq!(read_field(&up, *b"OS/2", "ulCodePageRange1").unwrap(), PatchValue::Number(1.0));
    assert_eq!(&data[2..4], &os2[2..4], "older fields keep their bytes");
    assert_eq!(raise_os2_version(&up, 2, &fill).unwrap(), up, "never lowered");
}
