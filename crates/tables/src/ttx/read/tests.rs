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
