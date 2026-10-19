//! The bitmap tables, `sbix` and `EBSC` against `ttx`'s compiler (M12.7c). Each input
//! (`testdata/<name>.ttx`, written by `testdata/oracle.py`) is the dump `ttx` writes of a font
//! fontTools compiled from the writer tests' tables; `testdata/<name>.digest` holds the length
//! and SHA-256 of each table fontTools compiles from it. Every case also writes the tables
//! back: the dump of what was read equals the input, so dump and import lose nothing.
#![allow(clippy::string_slice)]

use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::ttx::oracle_digest::{assert_table, expected};
use crate::ttx::{DumpOptions, ImportOptions, dump, import, import_tree, parse, tag_to_xml};

const BITMAP: (&str, &str) = (include_str!("testdata/bitmap.ttx"), include_str!("testdata/bitmap.digest"));
const CBDT: (&str, &str) = (include_str!("testdata/cbdt.ttx"), include_str!("testdata/cbdt.digest"));
const SBIX: (&str, &str) = (include_str!("testdata/sbix.ttx"), include_str!("testdata/sbix.digest"));
/// `text` without its comments, the spaces before them and the lines only a comment was on.
/// `text` without its comments, and without the lines only a comment was on.
fn strip_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        rest = rest[i..].find("-->").map_or("", |j| &rest[i + j + 3..]);
    }
    out.push_str(rest);
    out.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// The element of table `tag` in a dump, as text.
fn block(text: &str, tag: Tag) -> String {
    let name = tag_to_xml(tag);
    let start =
        text.find(&format!("\n  <{name}>")).or_else(|| text.find(&format!("\n  <{name} "))).unwrap_or_else(|| panic!("no '{name}' in the dump"));
    let close = format!("\n  </{name}>");
    let end = text[start..].find(&close).map(|i| start + i + close.len()).unwrap_or_else(|| panic!("'{name}' is not closed"));
    text[start..end].to_string()
}

/// Reads `text`, checks every table of the digest against fontTools' bytes, then dumps those
/// tables and compares each with the input (comments aside).
pub(crate) fn assert_reads_as_ttx((text, digest): (&str, &str)) -> SfntFont {
    let font = import(SfntFont::new(Flavour::TrueType), text, &ImportOptions::default()).unwrap();
    let want = expected(digest);
    for (tag, len, sha) in &want {
        let got = font.get(*tag).unwrap_or_else(|| panic!("no '{}' table", String::from_utf8_lossy(tag)));
        assert_table(tag, got, *len, sha);
    }
    let tags: Vec<Tag> = want.iter().map(|(t, _, _)| *t).collect();
    let options = DumpOptions { tables: tags.clone(), strict: true, ..DumpOptions::default() };
    let ours = dump(&font, &options).unwrap();
    for tag in tags {
        assert_eq!(
            strip_comments(&block(&ours, tag)),
            strip_comments(&block(text, tag)),
            "the dump of '{}' differs from its input",
            String::from_utf8_lossy(&tag)
        );
    }
    font
}

/// Every table element of `text` with each child, and each grandchild, left out in turn: the
/// import fails or succeeds, and never panics.
pub(crate) fn survives_missing_parts(text: &str) {
    let root = parse(text).unwrap();
    for (t, table) in root.children.iter().enumerate() {
        if table.name == "GlyphOrder" {
            continue;
        }
        for c in 0..table.children.len() {
            let mut cut = root.clone();
            cut.children[t].children.remove(c);
            let _ = import_tree(SfntFont::new(Flavour::TrueType), &cut, &ImportOptions::default());
            for g in 0..table.children[c].children.len() {
                let mut cut = root.clone();
                cut.children[t].children[c].children.remove(g);
                let _ = import_tree(SfntFont::new(Flavour::TrueType), &cut, &ImportOptions::default());
            }
        }
    }
}

#[test]
fn bitmap_tables_compile_as_ttx_compiles_and_dump_back() {
    assert_reads_as_ttx(BITMAP);
    assert_reads_as_ttx(CBDT);
}

#[test]
fn sbix_and_ebsc_compile_as_ttx_compiles_and_dump_back() {
    assert_reads_as_ttx(SBIX);
}

#[test]
fn bitmap_readers_survive_missing_parts() {
    for (text, _) in [BITMAP, CBDT, SBIX] {
        survives_missing_parts(text);
    }
}

#[test]
fn a_bitmap_data_table_needs_its_location_table() {
    let (text, _) = CBDT;
    let start = text.find("  <CBLC>").unwrap();
    let end = text.find("  </CBLC>\n").unwrap() + "  </CBLC>\n".len();
    let alone = format!("{}{}", &text[..start], &text[end..]);
    let err = import(SfntFont::new(Flavour::TrueType), &alone, &ImportOptions::default()).unwrap_err();
    assert!(err.to_string().contains("'CBLC'"), "{err}");
}

#[test]
fn bitmap_images_in_other_forms_are_refused() {
    let (text, _) = BITMAP;
    let row = text.replacen("<rawimagedata>", "<rowimagedata bitDepth=\"1\" width=\"1\" height=\"1\">", 1).replacen(
        "</rawimagedata>",
        "</rowimagedata>",
        1,
    );
    assert!(import(SfntFont::new(Flavour::TrueType), &row, &ImportOptions::default()).is_err());
}

#[test]
fn a_huge_fixed_image_size_is_refused_before_it_is_allocated() {
    let (text, _) = BITMAP;
    // 2^40 bytes of padding: allocating it first would abort the process.
    for size in ["1099511627776", "4294967295"] {
        let huge = text.replacen("<imageSize value=\"2\"/>", &format!("<imageSize value=\"{size}\"/>"), 1);
        assert_ne!(huge, text);
        let err = import(SfntFont::new(Flavour::TrueType), &huge, &ImportOptions::default()).unwrap_err();
        assert!(err.to_string().contains("image size") || err.to_string().contains("bitmap data table"), "{err}");
    }
}
