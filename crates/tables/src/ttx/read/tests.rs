//! The TTX parser and the import's limits and refusals.

use super::{import, int_lit, parse, read_hex, timestamp_from_string, xml_to_tag};
use crate::TablesError;
use crate::sfnt::{Flavour, SfntFont};
use crate::ttx::ImportOptions;

#[test]
fn the_parser_reads_what_expat_reads() {
    let text = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- a comment -->\n<ttFont sfntVersion=\"OTTO\">\r\n  <a v='x&amp;y' w=\"line one\nline\ttwo\"/>\n  <b>t&lt;1&#62;&#x41;<![CDATA[<raw>]]><!-- skipped --></b>\n</ttFont>\n";
    let root = parse(text).unwrap();
    assert_eq!(root.name, "ttFont");
    assert_eq!(root.attribute("sfntVersion"), Some("OTTO"));
    let a = root.child("a").unwrap();
    assert_eq!(a.attribute("v"), Some("x&y"));
    assert_eq!(a.attribute("w"), Some("line one line two"), "literal line ends and tabs in values become spaces");
    assert_eq!(root.child("b").unwrap().text, "t<1>A<raw>");
    assert!(root.text.contains('\n') && !root.text.contains('\r'), "line ends are normalised");
}

#[test]
fn malformed_and_oversized_xml_is_refused() {
    for bad in ["<a><b></a>", "<a>", "</a>", "<a/><b/>", "text<a/>", "<a x=1/>", "<a x='1' x='2'/>", "<a>&unknown;</a>", "<a>&#xD800;</a>"] {
        assert!(matches!(parse(bad), Err(TablesError::Invalid(_))), "{bad}");
    }
    let deep = format!("{}{}", "<a>".repeat(100), "</a>".repeat(100));
    assert!(matches!(parse(&deep), Err(TablesError::TooLarge { .. })), "nesting is capped");
}

#[test]
fn numbers_tags_hex_and_timestamps_read_as_fonttools_reads_them() {
    assert_eq!(int_lit("0x5f0f3cf5").unwrap(), 0x5f0f_3cf5);
    assert_eq!(int_lit("-12").unwrap(), -12);
    assert_eq!(int_lit("True").unwrap(), 1);
    assert!(int_lit("1.5").is_err());
    assert_eq!(&xml_to_tag("OS_2").unwrap(), b"OS/2");
    assert_eq!(&xml_to_tag("cvt").unwrap(), b"cvt ");
    assert_eq!(&xml_to_tag("_g_l_y_f").unwrap(), b"glyf");
    assert_eq!(&xml_to_tag("CFF").unwrap(), b"CFF ");
    assert_eq!(read_hex("0a1 b").unwrap(), vec![0x0a, 0x1b]);
    assert_eq!(read_hex("abc").unwrap(), vec![0xab, 0xc0], "a lone last digit is completed with 0");
    assert_eq!(timestamp_from_string("Thu Jan  1 00:00:00 1970").unwrap(), 0x7C25_B080);
    assert_eq!(timestamp_from_string("Thu Aug 10 11:12:00 2017").unwrap(), 0xD5B1_F000);
    assert!(timestamp_from_string("Fri Aug 10 11:12:00 2017").is_err(), "the weekday must match the date");
}

/// The `post` table a dump lacks is compiled again for the dump's glyph order: a glyph it renames gets its new name, a
/// name list entry that is a standard name goes, and names that stand for others stay.
#[test]
fn a_truetype_font_s_post_table_is_compiled_again_for_the_dump_s_glyph_order() {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"maxp", vec![0, 0, 0x50, 0, 0, 4]);
    let mut post = vec![0u8; 32];
    post[..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    // .notdef; "A" as a name of its own; "foo" twice, the second made unique as foo.1 or foo#1.
    for v in [4u16, 0, 258, 259, 259] {
        post.extend(v.to_be_bytes());
    }
    post.extend(b"\x01A\x03foo");
    font.insert(*b"post", post.clone());
    let names = crate::ttx::glyph_order(&font).unwrap();
    let dump = |second: &str| {
        let ids: String = [".notdef", second, &names[2], &names[3]].iter().map(|n| format!("<GlyphID name=\"{n}\"/>")).collect();
        format!("<ttFont><GlyphOrder>{ids}</GlyphOrder><zzzz raw=\"True\"><hexdata>00</hexdata></zzzz></ttFont>")
    };
    let opts = ImportOptions::default();
    let compiled = |text: &str| import(font.clone(), text, &opts).unwrap().get(*b"post").unwrap()[32..].to_vec();
    let expect = |indices: [u16; 5], list: &[u8]| [indices.map(u16::to_be_bytes).concat(), list.to_vec()].concat();
    assert_eq!(compiled(&dump("A")), expect([4, 0, 36, 258, 258], b"\x03foo"));
    assert_eq!(compiled(&dump("A.alt")), expect([4, 0, 259, 258, 258], b"\x03foo\x05A.alt"));
    // A dump that names other tables only, for a call that picks them, leaves it alone.
    let picked = ImportOptions { tables: vec![*b"zzzz"], ..ImportOptions::default() };
    assert_eq!(import(font.clone(), &dump("A.alt"), &picked).unwrap().get(*b"post"), Some(post.as_slice()));
}

#[test]
fn import_refuses_other_documents_and_unknown_tables_without_hex() {
    let font = SfntFont::new(Flavour::TrueType);
    let opts = ImportOptions::default();
    assert!(import(font.clone(), "<html/>", &opts).is_err());
    let unknown = "<ttFont><GlyphOrder><GlyphID name=\".notdef\"/></GlyphOrder><zzzz><thing/></zzzz></ttFont>";
    assert!(matches!(import(font.clone(), unknown, &opts), Err(TablesError::Unsupported(_))));
    let raw = "<ttFont><zzzz raw=\"True\"><hexdata>0102 03</hexdata></zzzz></ttFont>";
    let out = import(font, raw, &opts).unwrap();
    assert_eq!(out.get(*b"zzzz"), Some(&[1u8, 2, 3][..]));
}
