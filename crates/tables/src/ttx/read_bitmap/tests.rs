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
const FORMS_ROW: (&str, &str) = (include_str!("testdata/forms_row.ttx"), include_str!("testdata/forms_row.digest"));
const FORMS_BITWISE: (&str, &str) = (include_str!("testdata/forms_bitwise.ttx"), include_str!("testdata/forms_bitwise.digest"));
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

/// Imports `text`, which must succeed, and checks each table of `digest` against fontTools.
fn assert_compiles_as_ttx(text: &str, digest: &str) {
    let font = import(SfntFont::new(Flavour::TrueType), text, &ImportOptions::default()).unwrap();
    for (tag, len, sha) in &expected(digest) {
        assert_table(tag, font.get(*tag).unwrap(), *len, sha);
    }
}

/// `text` with each `(from, to)` replaced; each `from` must occur once.
fn edited(text: &str, edits: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (from, to) in edits {
        assert_eq!(out.matches(from).count(), 1, "{from}");
        out = out.replacen(from, to, 1);
    }
    out
}

fn import_err(text: &str) -> String {
    import(SfntFont::new(Flavour::TrueType), text, &ImportOptions::default()).unwrap_err().to_string()
}

#[test]
fn bitmap_images_in_the_row_and_bitwise_forms_compile_as_ttx_compiles() {
    // Strikes of bit depths 2, 4 and 1, every image format with pixel data, rows that do not
    // end on a byte; the bitwise form loses the deeper pixels, as it does in fontTools.
    assert_compiles_as_ttx(FORMS_ROW.0, FORMS_ROW.1);
    assert_compiles_as_ttx(FORMS_BITWISE.0, FORMS_BITWISE.1);
}

#[test]
fn bitmap_rows_of_the_wrong_length_compile_as_ttx_compiles() {
    // The digests are of what fontTools 4.66.1 compiles from the same edited text.
    let row = edited(
        FORMS_ROW.0,
        &[
            // A bit-aligned row with set bits past its 10: they spill into the next row.
            (r#"<row value="9c00"/>"#, r#"<row value="9cff"/>"#),
            // A bit-aligned row too short, a byte-aligned row too long, one empty, one too many.
            (r#"<row value="0180"/>"#, r#"<row value="01"/>"#),
            (r#"<row value="e4"/>"#, r#"<row value="e4ff"/>"#),
            (r#"<row value="18"/>"#, r#"<row value=""/>"#),
            (r#"<row value="5b70"/>"#, "<row value=\"5b70\"/>\n          <row value=\"77\"/>"),
        ],
    );
    assert_compiles_as_ttx(
        &row,
        "EBDT 67 38e6c8199a49df2cf06a7c73ce77cbcb746b232c2480098c3e0a1c77ebf42da7\n\
         EBLC 316 2fae50310fca92c5941fedbe56ee190e4d32cf4edc048e4f7c4fb2f1886f2f3c\n",
    );
    let bitwise = edited(
        FORMS_BITWISE.0,
        &[
            // Any character but a space, a full stop or 0 is a set bit.
            (r#"<row value="@..@@"/>"#, r##"<row value="#1x 0.@@@@@@"/>"##),
            (r#"<row value="@@@@@@@@@"/>"#, r#"<row value="@@"/>"#),
            ("<row value=\"@@@\"/>\n          <row value=\"...\"/>", "<row value=\"@@@@@@@@@@\"/>\n          <row value=\"\"/>"),
        ],
    );
    assert_compiles_as_ttx(
        &bitwise,
        "EBDT 64 d606bc3ee2a4152f1f5edf3033ca3d662df1342d414196ee7e57f7b1ee0812fd\n\
         EBLC 316 a6c467413cd7c653b420808092f92fb8059842ee56863e9033920baaf0feaa5e\n",
    );
}

#[test]
fn hostile_bitmap_rows_are_refused() {
    let (text, _) = FORMS_ROW;
    let header = r#"<rowimagedata bitDepth="2" height="2" width="5">"#;
    for (from, to, says) in [
        // Sizes are checked before anything is allocated.
        (header, r#"<rowimagedata bitDepth="2" height="2" width="4294967296">"#, "pixels wide"),
        (header, r#"<rowimagedata bitDepth="2" height="2" width="-1">"#, "pixels wide"),
        (header, r#"<rowimagedata bitDepth="99999999" height="2" width="5">"#, "bit depth"),
        (header, r#"<rowimagedata bitDepth="0" height="2" width="5">"#, "bit depth"),
        (header, r#"<rowimagedata height="2" width="5">"#, "bitDepth"),
        (header, r#"<rowimagedata bitDepth="2" width="5">"#, "height"),
        (r#"<row value="9c00"/>"#, r#"<row value="9zz0"/>"#, "hex"),
        (r#"<row value="9c00"/>"#, r#"<row/>"#, "row without a value"),
    ] {
        let err = import_err(&edited(text, &[(from, to)]));
        assert!(err.contains(says), "{to}: {err}");
    }
    let many = format!("{header}{}", r#"<row value="ff"/>"#.repeat(300));
    assert!(import_err(&edited(text, &[(header, &many)])).contains("rows of a bitmap image"));
}

#[test]
fn bitmap_images_in_external_files_and_colour_rows_are_refused() {
    let (text, _) = BITMAP;
    let ext = text.replacen("<rawimagedata>", r#"<extfileimagedata value="/etc/passwd"/><x>"#, 1).replacen("</rawimagedata>", "</x>", 1);
    assert!(import_err(&ext).contains("external file"));
    let (text, _) = CBDT;
    for form in ["rowimagedata", "bitwiseimagedata"] {
        let colour = text.replacen("<rawimagedata>", &format!(r#"<{form} bitDepth="32" width="1" height="1"><row value="00"/>"#), 1).replacen(
            "</rawimagedata>",
            &format!("</{form}>"),
            1,
        );
        assert!(import_err(&colour).contains("colour bitmap"), "{form}");
    }
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
