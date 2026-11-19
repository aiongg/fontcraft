//! The PDF writer on a font built in code, read back with lopdf: page count, page sizes and
//! glyph order against the SVG pages of the same proof, and waterfall text placed by advances and
//! pair kerning.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_compile::{Options, compile_ttf};
use fontcraft_model::{Font, Path, Shape};
use lopdf::content::{Content as Ops, Operation};
use skrifa::raw::TableProvider;

use super::{GLYPH_TAG, win_ansi};
use crate::{ProofFormat, ProofOptions, Sheet, proof};

/// `space`, `A`, `V` and `O`, with the pair `A V` kerned by -80 units.
fn font() -> Vec<u8> {
    let mut f = Font::new("Proof Pdf");
    f.glyphs.retain(|g| g.name == ".notdef");
    for (name, cp, width) in [("space", 0x20, 250.0), ("A", 0x41, 600.0), ("V", 0x56, 600.0), ("O", 0x4F, 640.0)] {
        f.add_glyph(name, vec![cp], width).unwrap();
        if name != "space" {
            f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, width - 50.0, 700.0)));
        }
    }
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -80.0);
    compile_ttf(&f, &Options::default()).unwrap().bytes
}

/// Each page of a PDF: its media box and its content operations.
fn read(pdf: &[u8]) -> Vec<(Vec<f32>, Vec<Operation>)> {
    let doc = lopdf::Document::load_mem(pdf).unwrap();
    doc.get_pages()
        .values()
        .map(|&id| {
            let page = doc.get_object(id).unwrap().as_dict().unwrap();
            let media_box = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|v| v.as_float().unwrap()).collect();
            (media_box, Ops::decode(&doc.get_page_content(id)).unwrap().operations)
        })
        .collect()
}

/// The names of the shown glyphs of a page, in drawing order.
fn shown_pdf(ops: &[Operation]) -> Vec<String> {
    ops.iter()
        .filter(|op| op.operator == "BDC" && op.operands[0].as_name().unwrap() == GLYPH_TAG)
        .map(|op| String::from_utf8(op.operands[1].as_dict().unwrap().get(b"Name").unwrap().as_str().unwrap().to_vec()).unwrap())
        .collect()
}

fn shown_svg(svg: &[u8]) -> Vec<String> {
    let svg = std::str::from_utf8(svg).unwrap();
    svg.split("data-glyph=\"").skip(1).map(|rest| rest.split('"').next().unwrap().to_string()).collect()
}

#[test]
fn pdf_pages_match_the_svg_pages_in_count_size_and_glyph_order() {
    let bytes = font();
    let sheets = vec![Sheet::GlyphGrid, Sheet::GlyphPerPage, Sheet::Waterfall { text: "AVOA".into() }];
    let options =
        |format| ProofOptions { format, sheets: sheets.clone(), page_size: [400.0, 500.0], sizes: vec![24.0, 12.0], ..ProofOptions::default() };
    let svg = proof(&bytes, 0, &options(ProofFormat::Svg)).unwrap();
    let pdf = proof(&bytes, 0, &options(ProofFormat::Pdf)).unwrap();
    assert_eq!(pdf.files.len(), 1, "one PDF for every page");
    assert_eq!(pdf.pages, svg.pages);
    assert!(pdf.files[0].starts_with(b"%PDF-1.7"));

    let pages = read(&pdf.files[0]);
    assert_eq!(pages.len(), svg.files.len(), "a grid page, one outline page per glyph and a waterfall page");
    for (i, ((media_box, ops), svg_page)) in pages.iter().zip(&svg.files).enumerate() {
        assert_eq!(media_box, &vec![0.0, 0.0, 400.0, 500.0], "page {i} keeps the page size");
        assert_eq!(shown_pdf(ops), shown_svg(svg_page), "page {i} shows the glyphs of the SVG page in its order");
    }
    let first = shown_pdf(&pages[0].1);
    assert_eq!(first, [".notdef", "space", "A", "V", "O"], "the grid follows glyph order");
}

#[test]
fn waterfall_text_is_placed_by_advances_and_pair_kerning() {
    let bytes = font();
    let upem = f64::from(skrifa::FontRef::new(&bytes).unwrap().head().unwrap().units_per_em());
    let options = ProofOptions {
        format: ProofFormat::Pdf,
        sheets: vec![Sheet::Waterfall { text: "AVA".into() }],
        sizes: vec![10.0],
        ..ProofOptions::default()
    };
    let pdf = proof(&bytes, 0, &options).unwrap();
    let pages = read(&pdf.files[0]);
    assert_eq!(pages.len(), 1);
    let ops = &pages[0].1;
    let span = ops.iter().find(|op| op.operator == "BDC" && op.operands[0].as_name().unwrap() == b"Span").unwrap();
    assert_eq!(span.operands[1].as_dict().unwrap().get(b"ActualText").unwrap().as_str().unwrap(), b"AVA");
    // Glyphs are the only items placed with a matrix; labels use the text matrix.
    let xs: Vec<f64> = ops.iter().filter(|op| op.operator == "cm").map(|op| f64::from(op.operands[4].as_float().unwrap())).collect();
    assert_eq!(xs.len(), 3, "A, V and A drawn as outlines");
    let k = 10.0 / upem;
    assert!((xs[1] - xs[0] - (600.0 - 80.0) * k).abs() < 1e-3, "A then V: the advance of A plus the kerning: {xs:?}");
    assert!((xs[2] - xs[1] - 600.0 * k).abs() < 1e-3, "V then A: no kerning: {xs:?}");
}

#[test]
fn labels_are_written_in_win_ansi() {
    assert_eq!(win_ansi("A \u{2014}\u{e9}\u{4e00}\u{2026}"), vec![b'A', b' ', 0x97, 0xE9, b'?', 0x85]);
}
