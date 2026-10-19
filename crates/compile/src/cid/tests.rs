//! Behaviour tests of the CID-keyed CFF build (M12.41) on sources built here; the expected
//! values follow makeotf 5.0.1 on the same synthetic CID-keyed UFO.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use fontcraft_model::cid::{CidFont as CidState, Ros, SubFont};
use fontcraft_model::plist::{Array, Dict, Number as PNumber, Str, Value};
use fontcraft_model::{Font, Path, Shape};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::tables::cmap::CmapSubtable;
use read_fonts::types::{Fixed, GlyphId};
use read_fonts::{FontRef, TableProvider};

use super::*;
use crate::{Flavour, Options, compile};

fn options() -> Options {
    Options { remove_overlap: false, skip_features: true, ..Options::default() }
}

fn numbers(v: &[i64]) -> Value {
    Value::Array(Array { items: v.iter().map(|n| Value::Number(PNumber::int(*n))).collect(), ..Default::default() })
}

fn private(entries: &[(&str, Value)]) -> Dict {
    let mut d = Dict::new();
    for (k, v) in entries {
        d.insert(*k, v.clone());
    }
    d
}

fn subfont(name: &str, glyphs: &[&str], private_dict: Dict) -> SubFont {
    let mut s = SubFont::new(name);
    s.dict.insert("PrivateDict", Value::Dict(private_dict));
    s.glyphs = glyphs.iter().map(|g| g.to_string()).collect();
    s
}

/// `.notdef`, space and A to Z, a to z; A, B, a and b drawn with bars. The CID map names
/// `.notdef`, space, A, B, a and b only; two sub-fonts hold them.
fn cid_source() -> Font {
    let mut f = Font::new("CidTest");
    for (name, bars) in [("A", 2), ("B", 3), ("a", 2), ("b", 1)] {
        let layer = &mut f.glyph_mut(name).unwrap().layers[0];
        for i in 0..bars {
            let x = 40.0 + 120.0 * f64::from(i);
            layer.shapes.push(Shape::Path(Path::rect(x, 0.0, x + 80.0, 700.0)));
        }
    }
    let state = CidState {
        ros: Ros { registry: "Test".into(), ordering: "Latin1".into(), supplement: 2 },
        font_name: Some("CidTest-Regular".into()),
        cids: [(".notdef", 0), ("space", 1), ("A", 2), ("B", 3), ("a", 20), ("b", 21)].iter().map(|(n, c)| (n.to_string(), *c)).collect(),
        subfonts: vec![
            subfont(
                "CidTest-Regular-Alphabetic",
                &[".notdef", "space", "A", "B"],
                private(&[("postscriptBlueValues", numbers(&[-10, 0, 700, 710])), ("postscriptStemSnapH", numbers(&[50]))]),
            ),
            subfont(
                "CidTest-Regular-Lower",
                &["a", "b"],
                private(&[
                    ("postscriptBlueValues", numbers(&[-12, 0, 500, 512])),
                    ("postscriptStemSnapH", numbers(&[52, 48])),
                    ("postscriptStemSnapV", numbers(&[78, 82])),
                    ("LanguageGroup", Value::Number(PNumber::int(1))),
                ]),
            ),
        ],
    };
    state.store(&mut f);
    f
}

const UNI_CMAP: &str = "%!PS-Adobe-3.0 Resource-CMap\n/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
/CIDSystemInfo 3 dict dup begin\n  /Registry (Test) def\n  /Ordering (Latin1) def\n  /Supplement 2 def\nend def\n\
/CMapName /UniTest-UTF32-H def\n1 begincodespacerange\n  <00000000> <0010FFFF>\nendcodespacerange\n\
2 begincidrange\n<00000041> <00000042> 2\n<00000061> <00000062> 20\nendcidrange\n\
2 begincidchar\n<00000020> 1\n<00003042> 30\nendcidchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";

const MAC_CMAP: &str = "/CIDSystemInfo 3 dict dup begin /Registry (Test) def /Ordering (Latin1) def /Supplement 2 def end def\n\
/CMapName /TestMac-H def\n2 begincidrange\n<41> <42> 2\n<61> <62> 20\nendcidrange\n1 begincidchar\n<20> 1\nendcidchar\n";

#[derive(Default)]
struct Points(Vec<(i32, i32)>);

impl CommandSink for Points {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push((x.to_i32(), y.to_i32()));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push((x.to_i32(), y.to_i32()));
    }
    fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, x: Fixed, y: Fixed) {
        self.0.push((x.to_i32(), y.to_i32()));
    }
    fn close(&mut self) {}
}

fn gid(out: &Compiled, name: &str) -> u32 {
    out.glyph_order.iter().position(|n| n == name).unwrap() as u32
}

fn cff_table(out: &Compiled) -> Vec<u8> {
    FontRef::new(&out.bytes).unwrap().table_data(Tag::new(b"CFF ")).unwrap().as_bytes().to_vec()
}

#[test]
fn a_cid_keyed_source_compiles_to_a_cid_keyed_cff_in_cid_order() {
    let f = cid_source();
    let target = f.default_target().unwrap();
    let out = compile(&f, &target, Flavour::Cff, &options()).unwrap();
    assert_eq!(&out.glyph_order[..7], [".notdef", "space", "A", "B", "a", "b", "C"]);
    let table = cff_table(&out);
    let cff = CffFontRef::new_cff(&table, 0, None).unwrap();
    assert!(cff.is_cid());
    assert_eq!(cff.num_subfonts(), 2);
    let charset = cff.charset().unwrap();
    let cid = |name: &str| charset.string_id(GlyphId::new(gid(&out, name))).unwrap().to_u16();
    assert_eq!([cid("space"), cid("A"), cid("B"), cid("a"), cid("b")], [1, 2, 3, 20, 21]);
    assert_eq!(cid("C"), 22, "glyphs the map leaves out follow the highest CID in glyph order");
    assert_eq!(cid("z"), 22 + 24 + 24 - 1);
    let fd = |name: &str| cff.subfont_index(GlyphId::new(gid(&out, name))).unwrap();
    assert_eq!([fd(".notdef"), fd("A"), fd("a"), fd("b"), fd("C")], [0, 0, 1, 1, 0]);
    // The table is subroutinised and still draws each glyph with its sub-font.
    let id = GlyphId::new(gid(&out, "B"));
    let sub = cff.subfont(fd("B"), &[]).unwrap();
    let mut points = Points::default();
    let width = cff.evaluate_charstring(&sub, id, &[], &mut points).unwrap();
    assert_eq!(points.0.len(), 12);
    assert_eq!(width.map(|w| w.to_i32()), Some(600));
    assert!(out.warnings.iter().any(|w| w.contains("the CID map leaves out `C`")), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w.contains("belong to no sub-font")), "{:?}", out.warnings);
    // Without a CMap file the source's own code points are encoded.
    let font = FontRef::new(&out.bytes).unwrap();
    let cmap = font.cmap().unwrap();
    assert_eq!(cmap.map_codepoint('C').map(|g| g.to_u32()), Some(gid(&out, "C")));
    // makeotf writes a stub Macintosh subtable without a Macintosh CMap: code 0 to glyph 0.
    let mac = cmap.encoding_records().iter().find(|r| r.platform_id() as u16 == 1).expect("a Macintosh subtable");
    assert_eq!(mac.encoding_id(), 0);
    let CmapSubtable::Format6(sub) = mac.subtable(cmap.offset_data()).unwrap() else { panic!("format 6") };
    assert_eq!((sub.first_code(), sub.entry_count(), sub.glyph_id_array()[0].get()), (0, 1, 0));
    // The CFF names the CID font; post holds no glyph names.
    assert_eq!(cff.metadata().unwrap().name(), Some("CidTest-Regular"));
    assert_eq!(font.post().unwrap().version(), read_fonts::types::Version16Dot16::VERSION_3_0);
}

#[test]
fn a_unicode_cmap_gives_the_code_points_and_a_mac_cmap_a_format_6_subtable() {
    let f = cid_source();
    let target = f.default_target().unwrap();
    let mut o = options();
    o.cid = CidOptions { unicode_cmap: Some(UNI_CMAP.into()), mac_cmap: Some(MAC_CMAP.into()), mac_script: None, mac_language: None };
    let out = compile(&f, &target, Flavour::Cff, &o).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    let cmap = font.cmap().unwrap();
    assert_eq!(cmap.map_codepoint('A').map(|g| g.to_u32()), Some(gid(&out, "A")));
    assert_eq!(cmap.map_codepoint('b').map(|g| g.to_u32()), Some(gid(&out, "b")));
    assert_eq!(cmap.map_codepoint(' ').map(|g| g.to_u32()), Some(1));
    // The Macintosh subtable covers 0x20 to 0x62, so `C` reads as glyph 0 there.
    assert_eq!(cmap.map_codepoint('C').filter(|g| g.to_u32() != 0), None, "only what the CMap maps is encoded");
    assert!(out.warnings.iter().any(|w| w.contains("Roman (0)")), "{:?}", out.warnings);
    let mac = cmap.encoding_records().iter().find(|r| r.platform_id() as u16 == 1).expect("a Macintosh subtable");
    assert_eq!(mac.encoding_id(), 0);
    let CmapSubtable::Format6(sub) = mac.subtable(cmap.offset_data()).unwrap() else { panic!("format 6") };
    assert_eq!(sub.first_code(), 0x20);
    assert_eq!(sub.entry_count(), 0x62 - 0x20 + 1);
    let ids = sub.glyph_id_array();
    assert_eq!(u32::from(ids[0x41 - 0x20].get()), gid(&out, "A"));
    assert_eq!(u32::from(ids[0x62 - 0x20].get()), gid(&out, "b"));
    assert_eq!(ids[0x43 - 0x20].get(), 0);

    o.cid.mac_script = Some(1);
    o.cid.mac_language = Some(12);
    let out = compile(&f, &target, Flavour::Cff, &o).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    let cmap = font.cmap().unwrap();
    let ids: Vec<(u16, u16)> = cmap.encoding_records().iter().map(|r| (r.platform_id() as u16, r.encoding_id())).collect();
    assert_eq!(ids, [(0, 3), (1, 1), (3, 1)], "records stay sorted");
    let mac = cmap.encoding_records().iter().find(|r| r.platform_id() as u16 == 1).unwrap();
    let CmapSubtable::Format6(sub) = mac.subtable(cmap.offset_data()).unwrap() else { panic!("format 6") };
    assert_eq!(sub.language(), 12);
}

#[test]
fn cmap_files_that_cannot_be_used_stop_the_compile() {
    let f = cid_source();
    let target = f.default_target().unwrap();
    let with = |uni: &str| {
        let mut o = options();
        o.cid.unicode_cmap = Some(uni.into());
        compile(&f, &target, Flavour::Cff, &o).map(|_| ()).unwrap_err().to_string()
    };
    assert!(with("1 begincidrange <41> 2 endcidrange").contains("Unicode CMap cannot be read"));
    assert!(with(&UNI_CMAP.replace("(Test)", "(Adobe)")).contains("is for Adobe-Latin1"));
    assert!(with(&UNI_CMAP.replace("/UniTest-UTF32-H", "/TestMac-H")).contains("not a Unicode CMap"));
    // A range runs past the highest CID long before it reaches the cap on codes.
    assert!(with("/CMapName /UniX-UTF32-H def 1 begincidrange <00000000> <FFFFFFFF> 0 endcidrange").contains("is not from 0 to"));
    assert!(with("/CMapName /UniX-UTF32-H def 1 begincidchar <41> 70000 endcidchar").contains("70000"));
    assert!(with("/CMapName /UniX-UTF32-H def 1 begincidrange <41> <43> 2").contains("not closed"));
    assert!(with("(unclosed").contains("not closed"));
    assert!(with("/CMapName /UniX-UTF32-H def").contains("no codes"));

    // A two-byte Macintosh CMap leaves the Macintosh subtable out, with a warning.
    let mut o = options();
    o.cid.mac_cmap = Some(MAC_CMAP.replace("<41> <42> 2", "<8140> <8141> 2").into());
    let out = compile(&f, &target, Flavour::Cff, &o).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("longer than one byte")), "{:?}", out.warnings);

    // An ordinary font ignores the files, with a warning.
    let plain = Font::new("Plain");
    let mut o = options();
    o.cid.unicode_cmap = Some(UNI_CMAP.into());
    let out = compile(&plain, &plain.default_target().unwrap(), Flavour::Cff, &o).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("not CID-keyed")), "{:?}", out.warnings);
    assert!(!CffFontRef::new_cff(&cff_table(&out), 0, None).unwrap().is_cid());

    // Malformed CID data stops the compile.
    let mut bad = cid_source();
    for m in std::sync::Arc::make_mut(&mut bad.masters) {
        m.user_data.0.insert(fontcraft_model::cid::ROS_KEY, Value::String(Str::new("Test-Latin1")));
    }
    let e = compile(&bad, &bad.default_target().unwrap(), Flavour::Cff, &options()).map(|_| ()).unwrap_err().to_string();
    assert!(e.contains("cannot be read"), "{e}");
}

#[test]
fn sub_font_private_values_are_read_as_given() {
    let mut w = Vec::new();
    let d = private(&[
        ("postscriptBlueValues", numbers(&[-12, 0, 500, 512])),
        ("postscriptOtherBlues", numbers(&[-250, -240])),
        ("postscriptStemSnapH", numbers(&[52, 48])),
        ("postscriptStemSnapV", numbers(&[78])),
        ("postscriptBlueScale", Value::Number(PNumber::float(0.05).unwrap())),
        ("LanguageGroup", Value::Number(PNumber::int(1))),
        ("postscriptDefaultWidthX", Value::Number(PNumber::int(1000))),
    ]);
    let (p, widths) = private_from(&d, &mut w, "Kana");
    assert_eq!(p.blue_values, [-12, 0, 500, 512].map(Number::Int));
    assert_eq!(p.other_blues, [-250, -240].map(Number::Int));
    assert_eq!(p.stem_snap_h, [48, 52].map(Number::Int), "StemSnapH is sorted");
    assert_eq!(p.stem_snap_v, [78].map(Number::Int));
    assert_eq!((p.std_hw, p.std_vw), (None, None), "a sub-font gets no StdHW or StdVW, as makeotf writes it");
    assert!(matches!(p.blue_scale, Number::Real(s) if s == 0.05));
    assert_eq!(p.blue_fuzz, Number::Int(1), "the CFF default");
    assert_eq!(p.language_group, Number::Int(1));
    assert_eq!(widths, Widths::Fixed { default_width_x: 1000, nominal_width_x: 0 });
    let (_, widths) = private_from(&private(&[("LanguageGroup", Value::Number(PNumber::int(7)))]), &mut w, "Kana");
    assert_eq!(widths, Widths::Optimise);
    assert!(w.iter().any(|m| m.contains("LanguageGroup 7")));
}

#[test]
fn cmap_files_read_ranges_chars_and_the_collection() {
    let text = b"%!PS-Adobe-3.0 Resource-CMap\n/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
/CIDSystemInfo 3 dict dup begin\n  /Registry (Test) def\n  /Ordering (Han1) def\n  /Supplement 2 def\nend def\n\
/CMapName /UniHan-UTF16-H def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
2 begincidrange\n<4E00> <4E02> 100\n<d840dc00> <d840dc01> 200\nendcidrange\n\
1 begincidchar\n<0020> 1 % a space\nendcidchar\nendcmap\n";
    let cmap = parse_cmap(text, CodeKind::Unicode).unwrap();
    assert_eq!(cmap.name.as_deref(), Some("UniHan-UTF16-H"));
    assert_eq!((cmap.registry.as_deref(), cmap.ordering.as_deref(), cmap.supplement), (Some("Test"), Some("Han1"), Some(2)));
    let got: Vec<(u32, u32)> = cmap.cids.iter().map(|(a, b)| (*a, *b)).collect();
    assert_eq!(got, [(0x20, 1), (0x4E00, 100), (0x4E01, 101), (0x4E02, 102), (0x20000, 200), (0x20001, 201)]);
    assert_eq!(cmap.codes_by_cid().get(&100), Some(&vec![0x4E00]));

    let utf8 = b"/CMapName /UniHan-UTF8-H def\n1 begincidchar\n<E4B880> 7\nendcidchar\n";
    assert_eq!(parse_cmap(utf8, CodeKind::Unicode).unwrap().cids.get(&0x4E00), Some(&7));
    assert!(
        parse_cmap(b"/CMapName /90ms-RKSJ-H def\n1 begincidchar\n<20> 1\nendcidchar\n", CodeKind::Unicode)
            .unwrap_err()
            .contains("not a Unicode CMap")
    );
    assert!(parse_cmap(b"1 begincidrange\n<0000> <00> 1\nendcidrange\n", CodeKind::Unicode).is_err());
    assert!(parse_cmap(b"1 begincidchar\n<0041> 70000\nendcidchar\n", CodeKind::Unicode).is_err());
    assert!(parse_cmap(b"1 begincidchar\n<0041\n", CodeKind::Unicode).is_err());
    assert!(parse_cmap(b"1 begincidrange\n<00000000> <0000FFFF> 0\nendcidrange\n", CodeKind::Unicode).unwrap_err().contains("65534"));
    // 17 ranges of 65,535 codes each: every CID is in range, the count is not.
    let ranges: String = (0..17).map(|i| format!("<{i:02X}0000> <{i:02X}FFFE> 0\n")).collect();
    let huge = format!("17 begincidrange\n{ranges}endcidrange\n");
    assert!(parse_cmap(huge.as_bytes(), CodeKind::Unicode).unwrap_err().contains("more than"));
}
