//! Colour, math, SVG, bitmap and metric tables in a subset, on fonts built in code. The expected
//! values are pyftsubset's (fontTools 4.66.1) on the same fonts built with fontTools'
//! `FontBuilder` and `colorLib` (same glyph order, paints, palettes, documents and records),
//! except the CBLC case, whose expectation follows fontTools' `EBLC` and `EBDT` methods.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_model::{Component, Font, Path, Shape};
use write_fonts::dump_table;
use write_fonts::tables::layout::CoverageTable;
use write_fonts::tables::math as wm;
use write_fonts::types::{FWord, GlyphId16, UfWord};

use super::super::{SubsetOptions, SubsetResult, subset};
use crate::reorder::be::Data;
use crate::reorder::glyph_names;
use crate::sfnt::{SfntFont, read};

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// A TrueType font of `.notdef` and `glyphs` (name, code point or 0), in that order; `B` is
/// `A` as a component, the others rectangles.
fn outlines(glyphs: &[(&str, u32)]) -> SfntFont {
    let mut f = Font::new("Other Subset");
    f.glyphs.retain(|g| g.name == ".notdef");
    for (i, (name, cp)) in glyphs.iter().enumerate() {
        let cps = if *cp == 0 { vec![] } else { vec![*cp] };
        f.add_glyph(name, cps, 500.0).unwrap();
        if *name != "B" {
            let x = 10.0 * (i + 1) as f64;
            f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x, 0.0, x + 200.0, 700.0)));
        }
    }
    if let Ok(mut b) = f.glyph_mut("B") {
        b.layers[0].shapes.push(Shape::Component(Component::new("A", 50.0, 0.0)));
    }
    let out = compile(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    let mut font = read(&out.bytes, 0).unwrap();
    for tag in [*b"GDEF", *b"GSUB", *b"GPOS", *b"kern"] {
        font.remove(tag);
    }
    font
}

fn gids(font: &SfntFont) -> BTreeMap<String, u16> {
    glyph_names(font).unwrap().into_iter().enumerate().map(|(i, n)| (n, i as u16)).collect()
}

fn run(font: &SfntFont, text: &str) -> SubsetResult {
    subset(font, &SubsetOptions { text: text.into(), ..SubsetOptions::default() }).unwrap()
}

// ── COLR and CPAL ──

/// A paint pool: each paint's bytes, placed one after another.
struct Pool(Vec<Vec<u8>>);

impl Pool {
    fn add(&mut self, bytes: Vec<u8>) -> usize {
        self.0.push(bytes);
        self.0.len() - 1
    }
}

/// PaintGlyph of `g` filled with PaintSolid of palette entry `p`, the solid right after it.
fn paint_glyph(g: u16, p: u16) -> Vec<u8> {
    let mut v = vec![10, 0, 0, 6];
    be16(&mut v, g);
    v.extend_from_slice(&[2]);
    be16(&mut v, p);
    be16(&mut v, 0x4000);
    v
}

fn colr_layers(num: u8, first: u32) -> Vec<u8> {
    let mut v = vec![1, num];
    be32(&mut v, first);
    v
}

/// COLR 1 with base glyph paints `bases` and layer list `layers` (indices into `pool`).
fn colr(pool: &Pool, bases: &[(u16, usize)], layers: &[usize]) -> Vec<u8> {
    let base_at = 34;
    let layer_at = base_at + 4 + 6 * bases.len();
    let paints_at = layer_at + 4 + 4 * layers.len();
    let mut at = Vec::new();
    let mut p = paints_at;
    for paint in &pool.0 {
        at.push(p);
        p += paint.len();
    }
    let mut out = Vec::new();
    be16(&mut out, 1);
    be16(&mut out, 0);
    be32(&mut out, 0);
    be32(&mut out, 0);
    be16(&mut out, 0);
    for v in [base_at, layer_at, 0, 0, 0] {
        be32(&mut out, v as u32);
    }
    be32(&mut out, bases.len() as u32);
    for (g, paint) in bases {
        be16(&mut out, *g);
        be32(&mut out, (at[*paint] - base_at) as u32);
    }
    be32(&mut out, layers.len() as u32);
    for paint in layers {
        be32(&mut out, (at[*paint] - layer_at) as u32);
    }
    for paint in &pool.0 {
        out.extend_from_slice(paint);
    }
    out
}

/// A paint as text: `G<glyph>/<palette>`, `L<first>+<count>` or `C<glyph>`.
fn describe(d: Data<'_>, at: usize) -> String {
    match d.u8(at).unwrap() {
        10 => {
            let solid = at + (usize::from(d.u8(at + 1).unwrap()) << 16 | usize::from(d.u16(at + 2).unwrap()));
            assert_eq!(d.u8(solid).unwrap(), 2);
            format!("G{}/{}", d.u16(at + 4).unwrap(), d.u16(solid + 1).unwrap())
        }
        1 => format!("L{}+{}", d.u32(at + 2).unwrap(), d.u8(at + 1).unwrap()),
        11 => format!("C{}", d.u16(at + 1).unwrap()),
        f => format!("?{f}"),
    }
}

/// The base glyph paints and the layer list of a COLR 1 table.
fn colr_view(table: &[u8]) -> (Vec<(u16, String)>, Vec<String>) {
    let d = Data::new(*b"COLR", table);
    assert_eq!(d.u16(0).unwrap(), 1);
    let base = d.u32_usize(14).unwrap();
    let bases = (0..d.u32_usize(base).unwrap())
        .map(|i| (d.u16(base + 4 + 6 * i).unwrap(), describe(d, base + d.u32_usize(base + 6 + 6 * i).unwrap())))
        .collect();
    let list = d.u32_usize(18).unwrap();
    let layers = (0..d.u32_usize(list).unwrap()).map(|i| describe(d, list + d.u32_usize(list + 4 + 4 * i).unwrap())).collect();
    (bases, layers)
}

/// CPAL 0 with one palette of `n` colours; entry `i` is (blue i, green 0, red 0, alpha 255).
fn cpal(n: u16) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [0, n, 1, n] {
        be16(&mut out, v);
    }
    be32(&mut out, 14);
    be16(&mut out, 0);
    for i in 0..n {
        out.extend_from_slice(&[i as u8, 0, 0, 255]);
    }
    out
}

#[test]
fn subset_colr_closes_over_paints_rebuilds_layer_reuse_and_prunes_the_palette() {
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43), ("a1", 0), ("a2", 0), ("b1", 0), ("x", 0x78)]);
    let g = gids(&font);
    let mut pool = Pool(Vec::new());
    let l0 = pool.add(paint_glyph(g["a1"], 1));
    let l1 = pool.add(paint_glyph(g["a2"], 2));
    let l2 = pool.add(colr_layers(2, 0));
    let l3 = pool.add(paint_glyph(g["b1"], 3));
    let pa = pool.add(colr_layers(2, 0));
    let pb = pool.add(colr_layers(2, 2));
    let pc = pool.add({
        let mut v = vec![11];
        be16(&mut v, g["A"]);
        v
    });
    font.insert(*b"COLR", colr(&pool, &[(g["A"], pa), (g["B"], pb), (g["C"], pc)], &[l0, l1, l2, l3]));
    font.insert(*b"CPAL", cpal(5));

    let result = run(&font, "BC");
    assert_eq!(result.retained, [".notdef", "A", "B", "C", "a1", "a2", "b1"], "C draws A, which draws a1 and a2");
    let (bases, layers) = colr_view(result.font.get(*b"COLR").unwrap());
    assert_eq!(layers, ["G4/0", "G5/1", "L0+2", "G6/2"]);
    assert_eq!(bases, [(1, "L0+2".to_string()), (2, "L2+2".to_string()), (3, "C1".to_string())]);
    let cpal = result.font.get(*b"CPAL").unwrap();
    let d = Data::new(*b"CPAL", cpal);
    assert_eq!(d.u16(2).unwrap(), 3, "entries 1, 2 and 3 are left");
    let colors: Vec<u8> = (0..3).map(|i| d.u8(d.u32_usize(8).unwrap() + 4 * i).unwrap()).collect();
    assert_eq!(colors, [1, 2, 3]);

    let none = run(&font, "x");
    assert!(none.font.get(*b"COLR").is_none() && none.font.get(*b"CPAL").is_none(), "no color glyph is left");
}

#[test]
fn subset_colr_skips_a_clip_record_whose_range_runs_backwards() {
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43)]);
    let g = gids(&font);
    let mut pool = Pool(Vec::new());
    let pa = pool.add(paint_glyph(g["B"], 0));
    let mut table = colr(&pool, &[(g["A"], pa)], &[]);
    let clips = table.len();
    table.splice(22..26, (clips as u32).to_be_bytes());
    table.push(1);
    be32(&mut table, 2);
    for (start, end) in [(g["A"], g["A"]), (g["C"], g["A"])] {
        be16(&mut table, start);
        be16(&mut table, end);
        table.extend_from_slice(&[0, 0, 19]);
    }
    table.push(1);
    for v in [0u16, 0, 500, 700] {
        be16(&mut table, v);
    }
    font.insert(*b"COLR", table);
    font.insert(*b"CPAL", cpal(1));

    // Before the guard, the backwards record (C to A) made the subsetter panic.
    let result = run(&font, "A");
    let out = result.font.get(*b"COLR").unwrap();
    let d = Data::new(*b"COLR", out);
    let list = d.u32_usize(22).unwrap();
    assert_ne!(list, 0, "the clip list stays");
    assert_eq!(d.u32(list + 1).unwrap(), 1, "only A's clip is left");
}

// ── MATH ──

fn math(g: &BTreeMap<String, u16>) -> Vec<u8> {
    let gid = |n: &str| GlyphId16::new(g[n]);
    let value = |v: i16| wm::MathValueRecord::new(FWord::new(v), None);
    let italics = wm::MathItalicsCorrectionInfo::new(
        CoverageTable::format_1(vec![gid("paren"), gid("paren.v1"), gid("x")]),
        vec![value(10), value(20), value(30)],
    );
    let part = |n: &str, flags: u16| {
        wm::GlyphPartRecord::new(gid(n), UfWord::new(0), UfWord::new(10), UfWord::new(300), wm::PartFlags::from_bits_truncate(flags))
    };
    let paren = wm::MathGlyphConstruction::new(
        Some(wm::GlyphAssembly::new(value(0), vec![part("paren.top", 0), part("paren.ext", 1)])),
        vec![wm::MathGlyphVariantRecord::new(gid("paren"), UfWord::new(700)), wm::MathGlyphVariantRecord::new(gid("paren.v1"), UfWord::new(900))],
    );
    let x = wm::MathGlyphConstruction::new(None, vec![wm::MathGlyphVariantRecord::new(gid("x"), UfWord::new(500))]);
    let variants = wm::MathVariants::new(UfWord::new(5), Some(CoverageTable::format_1(vec![gid("paren"), gid("x")])), None, vec![paren, x], vec![]);
    let table = wm::Math::new(wm::MathConstants::default(), wm::MathGlyphInfo::new(Some(italics), None, None, None), variants);
    dump_table(&table).unwrap()
}

fn coverage(d: Data<'_>, at: usize) -> Vec<u16> {
    assert_eq!(d.u16(at).unwrap(), 1, "a format 1 coverage");
    (0..usize::from(d.u16(at + 2).unwrap())).map(|i| d.u16(at + 4 + 2 * i).unwrap()).collect()
}

#[test]
fn subset_math_adds_variants_and_parts_and_keeps_the_constructions_of_kept_glyphs() {
    let mut font = outlines(&[("paren", 0x28), ("paren.v1", 0), ("paren.top", 0), ("paren.ext", 0), ("x", 0x78)]);
    font.insert(*b"MATH", math(&gids(&font)));
    let result = run(&font, "(");
    assert_eq!(result.retained, [".notdef", "paren", "paren.v1", "paren.top", "paren.ext"]);
    let table = result.font.get(*b"MATH").unwrap();
    let d = Data::new(*b"MATH", table);
    let info = usize::from(d.u16(6).unwrap());
    let italics = info + usize::from(d.u16(info).unwrap());
    assert_eq!(coverage(d, italics + usize::from(d.u16(italics).unwrap())), [1, 2]);
    let values: Vec<i16> = (0..usize::from(d.u16(italics + 2).unwrap())).map(|i| d.i16(italics + 4 + 4 * i).unwrap()).collect();
    assert_eq!(values, [10, 20]);
    let variants = usize::from(d.u16(8).unwrap());
    assert_eq!(coverage(d, variants + usize::from(d.u16(variants + 2).unwrap())), [1], "the construction of paren only");
    assert_eq!(d.u16(variants + 6).unwrap(), 1);
    let construction = variants + usize::from(d.u16(variants + 10).unwrap());
    let glyphs: Vec<u16> = (0..usize::from(d.u16(construction + 2).unwrap())).map(|i| d.u16(construction + 4 + 4 * i).unwrap()).collect();
    assert_eq!(glyphs, [1, 2]);
    let assembly = construction + usize::from(d.u16(construction).unwrap());
    let parts: Vec<u16> = (0..usize::from(d.u16(assembly + 4).unwrap())).map(|i| d.u16(assembly + 6 + 10 * i).unwrap()).collect();
    assert_eq!(parts, [3, 4]);
}

// ── SVG, kern, VORG, hdmx and sbix ──

fn svg(doc: impl AsRef<[u8]>, start: u16, end: u16) -> Vec<u8> {
    let doc = doc.as_ref();
    let mut out = Vec::new();
    be16(&mut out, 0);
    be32(&mut out, 10);
    be32(&mut out, 0);
    be16(&mut out, 1);
    be16(&mut out, start);
    be16(&mut out, end);
    be32(&mut out, 14);
    be32(&mut out, doc.len() as u32);
    out.extend_from_slice(doc);
    out
}

/// The documents of an SVG table as stored: (first glyph, last glyph, bytes).
fn svg_stored(table: &[u8]) -> Vec<(u16, u16, Vec<u8>)> {
    let d = Data::new(*b"SVG ", table);
    let list = d.u32_usize(2).unwrap();
    (0..usize::from(d.u16(list).unwrap()))
        .map(|i| {
            let r = list + 2 + 12 * i;
            let doc = d.slice(list + d.u32_usize(r + 4).unwrap(), d.u32_usize(r + 8).unwrap()).unwrap();
            (d.u16(r).unwrap(), d.u16(r + 2).unwrap(), doc.to_vec())
        })
        .collect()
}

/// The documents of an SVG table: (first glyph, last glyph, text).
fn svg_docs(table: &[u8]) -> Vec<(u16, u16, String)> {
    svg_stored(table).into_iter().map(|(s, e, doc)| (s, e, String::from_utf8(doc).unwrap())).collect()
}

const SVG_HEAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">"#;

#[test]
fn subset_svg_skips_a_backwards_record_and_reads_a_shared_document_once() {
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43)]);
    let doc = format!(r#"{SVG_HEAD}<g id="glyph1"><path d="M0 0"/></g><g id="glyph2"><path d="M1 1"/></g></svg>"#);
    // Two records over one document, and one whose first glyph (3) comes after its last (1).
    let mut table = Vec::new();
    be16(&mut table, 0);
    be32(&mut table, 10);
    be32(&mut table, 0);
    be16(&mut table, 3);
    let doc_at = 2 + 12 * 3;
    for (start, end) in [(1u16, 1u16), (2, 2), (3, 1)] {
        be16(&mut table, start);
        be16(&mut table, end);
        be32(&mut table, doc_at);
        be32(&mut table, doc.len() as u32);
    }
    table.extend_from_slice(doc.as_bytes());
    font.insert(*b"SVG ", table);

    // Before the guard, the backwards record made the subsetter panic.
    let result = run(&font, "AB");
    let docs = svg_docs(result.font.get(*b"SVG ").unwrap());
    assert_eq!(docs.len(), 2, "{docs:?}");
    assert!(docs[0].2.contains("glyph1") && !docs[0].2.contains("glyph2"), "{docs:?}");
    assert!(docs[1].2.contains("glyph2") && !docs[1].2.contains("glyph1"), "{docs:?}");
}

#[test]
fn subset_svg_refuses_many_lengths_over_one_large_document_quickly() {
    let mut font = outlines(&[("A", 0x41)]);
    // A 1 MB document with 65,535 bytes of trailing spaces; every record reads it with a
    // different length, so no two records share a document.
    let mut doc = b"<svg><!--".to_vec();
    doc.resize(doc.len() + (1 << 20), b'x');
    doc.extend_from_slice(b"--></svg>");
    let base = doc.len();
    let count = usize::from(u16::MAX);
    doc.resize(base + count, b' ');
    let mut table = Vec::new();
    be16(&mut table, 0);
    be32(&mut table, 10);
    be32(&mut table, 0);
    be16(&mut table, count as u16);
    let doc_at = (2 + 12 * count) as u32;
    for i in 0..count {
        be16(&mut table, 1);
        be16(&mut table, 1);
        be32(&mut table, doc_at);
        be32(&mut table, (base + i) as u32);
    }
    table.extend_from_slice(&doc);
    font.insert(*b"SVG ", table);

    // Before the cap on bytes read, each record parsed the whole document again.
    let started = std::time::Instant::now();
    let result = subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() });
    assert!(started.elapsed().as_secs() < 10, "took {:?}", started.elapsed());
    let err = result.expect_err("the overlapping documents are refused");
    assert!(err.to_string().contains("SVG documents read"), "{err}");
}

#[test]
fn subset_svg_keeps_referenced_elements_and_renumbers_glyph_ids() {
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43)]);
    let doc = format!(
        r##"<?xml version="1.0"?>{SVG_HEAD}<defs><linearGradient id="g1"/><linearGradient id="unused"/></defs><g id="glyph1"><path d="M0 0"/></g><g id="glyph2" fill="url(#g1)"><use xlink:href="#glyph1"/></g><!-- c --></svg>"##
    );
    font.insert(*b"SVG ", svg(&doc, 1, 2));
    let both = run(&font, "B");
    assert_eq!(
        svg_docs(both.font.get(*b"SVG ").unwrap()),
        [(
            1,
            2,
            format!(
                r##"{SVG_HEAD}<defs><linearGradient id="g1"/></defs><g id="glyph1"><path d="M0 0"/></g><g id="glyph2" fill="url(#g1)"><use xlink:href="#glyph1"/></g></svg>"##
            )
        )]
    );
    let doc = format!(
        "{SVG_HEAD}<g id=\"glyph1\"><path d=\"M0 0\"/></g>\n<g id=\"glyph3\" style=\"fill:url(#g1)\"><use xlink:href=\"#glyph1\"/></g><defs><linearGradient id=\"g1\"/></defs></svg>"
    );
    font.insert(*b"SVG ", svg(&doc, 1, 3));
    let c = run(&font, "C");
    assert_eq!(
        svg_docs(c.font.get(*b"SVG ").unwrap()),
        [(
            1,
            1,
            format!(
                r##"{SVG_HEAD}<g id=".glyph1"><path d="M0 0"/></g><g id="glyph1" style="fill:url(#g1)"><use xlink:href="#.glyph1"/></g><defs><linearGradient id="g1"/></defs></svg>"##
            )
        )],
        "glyph 3 becomes glyph 1; the glyph it uses, not in the subset, gets a leading dot"
    );
    font.insert(*b"SVG ", b"\0\0\0\0\0\x0a\0\0\0\0\0\x01\0\x01\0\x01\0\0\0\x0e\0\0\0\x05<svg ".to_vec());
    assert!(subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() }).is_err(), "a broken document is an error");
}

/// `data` as a gzip member with every optional header field: extra field, name, comment and
/// header CRC.
fn gzip_with_fields(data: &[u8]) -> Vec<u8> {
    let mut m = vec![0x1F, 0x8B, 8, 0x04 | 0x08 | 0x10 | 0x02, 0, 0, 0, 0, 0, 3];
    m.extend_from_slice(&[3, 0, b'a', b'b', b'c']);
    m.extend_from_slice(b"doc.svg\0a comment\0");
    m.extend_from_slice(&[0x12, 0x34]);
    m.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    m.extend_from_slice(&super::svg::crc32(&super::svg::crc_table(), data).to_le_bytes());
    m.extend_from_slice(&(data.len() as u32).to_le_bytes());
    m
}

#[test]
fn subset_svg_pretty_prints_documents_as_the_reference() {
    // Expected: pyftsubset --pretty-svg --text=AC on the same document and glyph order.
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43)]);
    let doc = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg version="1.1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
  <defs>
    <linearGradient id="g1"><stop offset="0" stop-color='#"f00'/></linearGradient>
    <linearGradient id="unused"/>
  </defs>
  <g id="glyph1"><path d="M0 0"></path><!-- kept --></g>
  <g id="glyph2"><use xlink:href="#glyph1"/></g>
  <g id="glyph3" fill="url(#g1)">
    <text x="1">é &amp; <tspan>t</tspan></text>
    <g><g><use   xlink:href = "#glyph1" /></g></g>
    <style>  </style>
  </g>
</svg>
"##;
    font.insert(*b"SVG ", svg(doc, 1, 3));
    let result = subset(&font, &SubsetOptions { text: "AC".into(), pretty_svg: true, ..SubsetOptions::default() }).unwrap();
    let expected = "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\">\n  <defs>\n    <linearGradient id=\"g1\">\n      <stop offset=\"0\" stop-color=\"#&quot;f00\"/>\n    </linearGradient>\n  </defs>\n  <g id=\"glyph1\">\n    <path d=\"M0 0\"/>\n    <!-- kept -->\n  </g>\n  <g id=\"glyph2\" fill=\"url(#g1)\">\n    <text x=\"1\">&#233; &amp; <tspan>t</tspan></text>\n    <g>\n      <g>\n        <use xlink:href=\"#glyph1\"/>\n      </g>\n    </g>\n    <style>  </style>\n  </g>\n</svg>\n";
    assert_eq!(svg_docs(result.font.get(*b"SVG ").unwrap()), [(1, 2, expected.to_string())]);
}

#[test]
fn subset_svg_inflates_gzip_documents_and_compresses_the_subset_again() {
    let mut font = outlines(&[("A", 0x41), ("X", 0x58)]);
    let path = "M0 0 L10 10 ".repeat(40);
    let doc = format!(r#"{SVG_HEAD}<g id="glyph1"><path d="{path}"/></g><g id="glyph2"><path d="{path}"/></g></svg>"#);
    // Two members, the first with every optional header field, then zero padding.
    let (head, tail) = doc.as_bytes().split_at(doc.len() / 2);
    let mut packed = gzip_with_fields(head);
    packed.extend(super::svg::gzip(tail));
    packed.extend_from_slice(&[0, 0, 0]);
    font.insert(*b"SVG ", svg(&packed, 1, 2));

    let docs = svg_stored(run(&font, "A").font.get(*b"SVG ").unwrap());
    assert_eq!(docs.len(), 1);
    let (start, end, stored) = &docs[0];
    assert_eq!((*start, *end), (1, 1));
    assert_eq!(stored[..10], [0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 2, 255], "one member, no name, time 0");
    let text = miniz_oxide::inflate::decompress_to_vec(&stored[10..stored.len() - 8]).unwrap();
    assert_eq!(String::from_utf8(text).unwrap(), format!(r#"{SVG_HEAD}<g id="glyph1"><path d="{path}"/></g></svg>"#));

    // A subset document that gzip does not make smaller is stored as text.
    let small = r#"<svg><g id="glyph1"/><g id="glyph2"/></svg>"#;
    font.insert(*b"SVG ", svg(super::svg::gzip(small.as_bytes()), 1, 2));
    assert_eq!(svg_docs(run(&font, "A").font.get(*b"SVG ").unwrap()), [(1, 1, r#"<svg><g id="glyph1"/></svg>"#.to_string())]);

    // A damaged checksum is an error.
    let at = packed.len() - 3 - 8;
    packed[at] ^= 0xFF;
    font.insert(*b"SVG ", svg(&packed, 1, 2));
    let err = subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() }).expect_err("refused");
    assert!(err.to_string().contains("checksum"), "{err}");
}

#[test]
fn subset_svg_refuses_a_gzip_bomb_quickly() {
    let mut font = outlines(&[("A", 0x41)]);
    // 32 MiB of spaces packs into a few tens of kilobytes, far past the inflate budget.
    let mut bomb = super::svg::gzip(&vec![b' '; 32 << 20]);
    let started = std::time::Instant::now();
    font.insert(*b"SVG ", svg(&bomb, 1, 1));
    let err = subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() }).expect_err("the declared length is refused");
    assert!(err.to_string().contains("inflated SVG documents"), "{err}");

    // A stream that understates its length is stopped when the inflated bytes reach the budget.
    let n = bomb.len();
    bomb[n - 4..].copy_from_slice(&100u32.to_le_bytes());
    font.insert(*b"SVG ", svg(&bomb, 1, 1));
    let err = subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() }).expect_err("the inflated bytes are refused");
    assert!(err.to_string().contains("inflated SVG documents"), "{err}");
    assert!(started.elapsed().as_secs() < 10, "took {:?}", started.elapsed());
}

fn glyph_tables(font: &mut SfntFont) {
    let g = gids(font);
    let (a, b, c) = (g["A"], g["B"], g["C"]);
    let mut kern = Vec::new();
    for v in [0u16, 1, 0, 14 + 18, 0x0001, 3, 12, 1, 6] {
        be16(&mut kern, v);
    }
    let mut pairs = vec![(a, c, -10i16), (c, c, -5), (b, a, -20)];
    pairs.sort();
    for (l, r, v) in pairs {
        be16(&mut kern, l);
        be16(&mut kern, r);
        be16(&mut kern, v as u16);
    }
    font.insert(*b"kern", kern);
    let mut vorg = Vec::new();
    for v in [1u16, 0, 880, 2, a, 900, c, 870] {
        be16(&mut vorg, v);
    }
    font.insert(*b"VORG", vorg);
    let mut hdmx = Vec::new();
    be16(&mut hdmx, 0);
    be16(&mut hdmx, 1);
    be32(&mut hdmx, 8);
    hdmx.extend_from_slice(&[12, 8, 5, 6, 7, 8, 0, 0]);
    font.insert(*b"hdmx", hdmx);
    // sbix: one strike at 20 ppem; A a PNG, C a duplicate of A.
    let mut glyph_a = vec![0, 0, 0, 0];
    glyph_a.extend_from_slice(b"png AAAA");
    let mut glyph_c = vec![0, 0, 0, 0];
    glyph_c.extend_from_slice(b"dupe");
    be16(&mut glyph_c, a);
    let mut data: Vec<Vec<u8>> = vec![Vec::new(); 4];
    data[usize::from(a)] = glyph_a;
    data[usize::from(c)] = glyph_c;
    let mut strike = Vec::new();
    be16(&mut strike, 20);
    be16(&mut strike, 72);
    let mut at = 4 + 4 * 5;
    for d in &data {
        be32(&mut strike, at as u32);
        at += d.len();
    }
    be32(&mut strike, at as u32);
    for d in &data {
        strike.extend_from_slice(d);
    }
    let mut sbix = Vec::new();
    be16(&mut sbix, 1);
    be16(&mut sbix, 1);
    be32(&mut sbix, 1);
    be32(&mut sbix, 12);
    sbix.extend(strike);
    font.insert(*b"sbix", sbix);
}

/// kern pairs, VORG records, hdmx widths and the sbix bitmaps (type and payload) by glyph.
type GlyphTables = (Option<Vec<(u16, u16, i16)>>, Vec<(u16, i16)>, Vec<u8>, Vec<Option<(String, Vec<u8>)>>);

fn glyph_tables_view(font: &SfntFont) -> GlyphTables {
    let kern = font.get(*b"kern").map(|t| {
        let d = Data::new(*b"kern", t);
        assert_eq!(d.u16(2).unwrap(), 1);
        (0..usize::from(d.u16(10).unwrap())).map(|i| (d.u16(18 + 6 * i).unwrap(), d.u16(20 + 6 * i).unwrap(), d.i16(22 + 6 * i).unwrap())).collect()
    });
    let v = Data::new(*b"VORG", font.get(*b"VORG").unwrap());
    let vorg = (0..usize::from(v.u16(6).unwrap())).map(|i| (v.u16(8 + 4 * i).unwrap(), v.i16(10 + 4 * i).unwrap())).collect();
    let h = font.get(*b"hdmx").unwrap();
    let n = glyph_names(font).unwrap().len();
    let hdmx = h[8..10 + n].to_vec();
    let s = Data::new(*b"sbix", font.get(*b"sbix").unwrap());
    let strike = s.u32_usize(8).unwrap();
    let sbix = (0..n)
        .map(|g| {
            let (start, end) = (s.u32_usize(strike + 4 + 4 * g).unwrap(), s.u32_usize(strike + 8 + 4 * g).unwrap());
            (end > start).then(|| {
                let bytes = s.slice(strike + start, end - start).unwrap();
                (String::from_utf8(bytes[4..8].to_vec()).unwrap(), bytes[8..].to_vec())
            })
        })
        .collect();
    (kern, vorg, hdmx, sbix)
}

/// An sbix for 4 glyphs whose strike list holds `list` (indices into `ppems`, one strike per
/// ppem, every bitmap empty).
fn sbix_strikes(ppems: &[u16], list: &[usize]) -> Vec<u8> {
    let header = 8 + 4 * list.len();
    let strike_len = 4 + 4 * 5;
    let mut out = Vec::new();
    be16(&mut out, 1);
    be16(&mut out, 1);
    be32(&mut out, list.len() as u32);
    for i in list {
        be32(&mut out, (header + i * strike_len) as u32);
    }
    for p in ppems {
        be16(&mut out, *p);
        be16(&mut out, 72);
        for _ in 0..5 {
            be32(&mut out, strike_len as u32);
        }
    }
    out
}

#[test]
fn subset_sbix_reads_a_repeated_strike_once_and_caps_the_strikes() {
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43)]);
    let mut list = vec![0; 200_000];
    list.push(1);
    font.insert(*b"sbix", sbix_strikes(&[20, 40], &list));
    let out = run(&font, "A");
    let d = Data::new(*b"sbix", out.font.get(*b"sbix").unwrap());
    assert_eq!(d.u32(4).unwrap(), 2, "one strike per ppem");

    let ppems: Vec<u16> = (1..=257).collect();
    let list: Vec<usize> = (0..257).collect();
    font.insert(*b"sbix", sbix_strikes(&ppems, &list));
    let err = subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() }).expect_err("refused");
    assert!(err.to_string().contains("sbix strikes"), "{err}");
}

#[test]
fn subset_kern_vorg_hdmx_and_sbix_follow_the_output_glyphs() {
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43)]);
    glyph_tables(&mut font);
    let ac = run(&font, "AC");
    assert_eq!(ac.retained, [".notdef", "A", "C"]);
    let (kern, vorg, hdmx, sbix) = glyph_tables_view(&ac.font);
    assert_eq!(kern, Some(vec![(1, 2, -10), (2, 2, -5)]));
    assert_eq!(vorg, [(1, 900), (2, 870)]);
    assert_eq!(hdmx, [12, 8, 5, 6, 8], "the maximum is recomputed over the kept widths");
    assert_eq!(sbix, [None, Some(("png ".into(), b"AAAA".to_vec())), Some(("dupe".into(), vec![0, 1]))]);

    // B pulls A in as a component only: the kern pair B A goes, as A is not in the layout set.
    let b = run(&font, "B");
    assert_eq!(b.retained, [".notdef", "A", "B"]);
    let (kern, vorg, _, sbix) = glyph_tables_view(&b.font);
    assert_eq!(kern, None);
    assert_eq!(vorg, [(1, 900)]);
    assert_eq!(sbix, [None, Some(("png ".into(), b"AAAA".to_vec())), None]);
}

#[test]
fn subset_cblc_keeps_the_bitmaps_of_the_output_glyphs() {
    let mut font = outlines(&[("A", 0x41), ("B", 0x42), ("C", 0x43)]);
    // One strike, one index subtable of format 1 over glyphs 1 to 3, image format 17.
    let images: [&[u8]; 3] = [b"imgA", b"imageB", b"pngC!"];
    let mut cbdt = vec![0, 3, 0, 0];
    let mut offsets = vec![0u32];
    for img in images {
        cbdt.extend_from_slice(img);
        offsets.push(offsets.last().unwrap() + img.len() as u32);
    }
    let mut sub = Vec::new();
    be16(&mut sub, 1);
    be16(&mut sub, 17);
    be32(&mut sub, 4);
    for o in &offsets {
        be32(&mut sub, *o);
    }
    let mut cblc = vec![0, 3, 0, 0];
    be32(&mut cblc, 1);
    let mut record = Vec::new();
    be32(&mut record, 8 + 48);
    be32(&mut record, (8 + sub.len()) as u32);
    be32(&mut record, 1);
    be32(&mut record, 0);
    record.extend_from_slice(&[0; 24]);
    be16(&mut record, 1);
    be16(&mut record, 3);
    record.extend_from_slice(&[20, 20, 32, 1]);
    cblc.extend(record);
    be16(&mut cblc, 1);
    be16(&mut cblc, 3);
    be32(&mut cblc, 8);
    cblc.extend(sub);
    font.insert(*b"CBLC", cblc);
    font.insert(*b"CBDT", cbdt);

    let result = run(&font, "AC");
    let l = Data::new(*b"CBLC", result.font.get(*b"CBLC").unwrap());
    let d = Data::new(*b"CBDT", result.font.get(*b"CBDT").unwrap());
    assert_eq!(l.u32(4).unwrap(), 1);
    assert_eq!((l.u16(8 + 40).unwrap(), l.u16(8 + 42).unwrap()), (1, 2), "the strike covers the new ids of A and C");
    let array = l.u32_usize(8).unwrap();
    assert_eq!(l.u32(8 + 8).unwrap(), 1);
    assert_eq!((l.u16(array).unwrap(), l.u16(array + 2).unwrap()), (1, 2));
    let sub = array + l.u32_usize(array + 4).unwrap();
    assert_eq!((l.u16(sub).unwrap(), l.u16(sub + 2).unwrap()), (1, 17));
    let image_at = l.u32_usize(sub + 4).unwrap();
    let image = |i: usize| {
        let (s, e) = (l.u32_usize(sub + 8 + 4 * i).unwrap(), l.u32_usize(sub + 12 + 4 * i).unwrap());
        d.slice(image_at + s, e - s).unwrap().to_vec()
    };
    assert_eq!((image(0), image(1)), (b"imgA".to_vec(), b"pngC!".to_vec()));

    let none = run(&font, "B");
    let l = Data::new(*b"CBLC", none.font.get(*b"CBLC").unwrap());
    assert_eq!(l.u32(4).unwrap(), 1, "B and A keep their bitmaps");
}

/// A CBLC and CBDT of one strike over glyphs 1 to 4: `images` (glyph, image format, bytes), one
/// index subtable of format 1 per run of glyphs with the same image format.
fn bitmap_strike(images: &[(u16, u16, &[u8])]) -> (Vec<u8>, Vec<u8>) {
    let mut cbdt = vec![0, 3, 0, 0];
    let mut runs: Vec<Vec<(u16, u16, &[u8])>> = Vec::new();
    for img in images {
        match runs.last_mut() {
            Some(run) if run.last().unwrap().1 == img.1 && run.last().unwrap().0 + 1 == img.0 => run.push(*img),
            _ => runs.push(vec![*img]),
        }
    }
    let mut array = Vec::new();
    let mut bodies = Vec::new();
    for run in &runs {
        be16(&mut array, run[0].0);
        be16(&mut array, run.last().unwrap().0);
        be32(&mut array, (8 * runs.len() + bodies.len()) as u32);
        be16(&mut bodies, 1);
        be16(&mut bodies, run[0].1);
        be32(&mut bodies, cbdt.len() as u32);
        let mut rel = 0u32;
        for (_, _, bytes) in run {
            be32(&mut bodies, rel);
            cbdt.extend_from_slice(bytes);
            rel += bytes.len() as u32;
        }
        be32(&mut bodies, rel);
    }
    array.extend(bodies);
    let mut cblc = vec![0, 3, 0, 0];
    be32(&mut cblc, 1);
    be32(&mut cblc, 8 + 48);
    be32(&mut cblc, array.len() as u32);
    be32(&mut cblc, runs.len() as u32);
    be32(&mut cblc, 0);
    cblc.extend_from_slice(&[0; 24]);
    be16(&mut cblc, 1);
    be16(&mut cblc, 4);
    cblc.extend_from_slice(&[20, 20, 32, 1]);
    cblc.extend(array);
    (cblc, cbdt)
}

/// The images of a CBLC and CBDT of one strike whose index subtables are all of format 1.
fn bitmap_images(font: &SfntFont) -> BTreeMap<u16, Vec<u8>> {
    let l = Data::new(*b"CBLC", font.get(*b"CBLC").unwrap());
    let d = Data::new(*b"CBDT", font.get(*b"CBDT").unwrap());
    let array = l.u32_usize(8).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..l.u32_usize(8 + 8).unwrap() {
        let a = array + 8 * i;
        let (first, last) = (l.u16(a).unwrap(), l.u16(a + 2).unwrap());
        let sub = array + l.u32_usize(a + 4).unwrap();
        assert_eq!(l.u16(sub).unwrap(), 1, "index format");
        let image_at = l.u32_usize(sub + 4).unwrap();
        for (k, g) in (first..=last).enumerate() {
            let (s, e) = (l.u32_usize(sub + 8 + 4 * k).unwrap(), l.u32_usize(sub + 12 + 4 * k).unwrap());
            if e > s {
                out.insert(g, d.slice(image_at + s, e - s).unwrap().to_vec());
            }
        }
    }
    out
}

#[test]
fn subset_cblc_renumbers_composite_bitmaps_without_adding_their_components() {
    let mut font = outlines(&[("A", 0x41), ("X", 0x58), ("Y", 0x59), ("Z", 0x5A)]);
    let g = gids(&font);
    assert_eq!((g["A"], g["X"], g["Y"], g["Z"]), (1, 2, 3, 4));
    // Y (image format 8, small metrics and a pad byte) is A and X; Z (format 9, big metrics) is
    // Y and X. Components: glyph id, x offset, y offset.
    let y: &[u8] = &[2, 8, 0, 2, 8, 0, 0, 2, 0, 1, 0, 0, 0, 2, 1, 0];
    let z: &[u8] = &[2, 8, 0, 2, 8, 0, 0, 8, 0, 2, 0, 3, 1, 1, 0, 2, 0, 0];
    let (cblc, cbdt) = bitmap_strike(&[(1, 17, b"imgA"), (2, 17, b"imgX"), (3, 8, y), (4, 9, z)]);
    font.insert(*b"CBLC", cblc);
    font.insert(*b"CBDT", cbdt);

    // pyftsubset does not close over bitmap components: A stays out. Y loses it, and X, Y and Z
    // become glyphs 1, 2 and 3.
    let out = run(&font, "XYZ");
    assert_eq!(out.retained, [".notdef", "X", "Y", "Z"]);
    let images = bitmap_images(&out.font);
    assert_eq!(images[&1], b"imgX");
    assert_eq!(images[&2], [2, 8, 0, 2, 8, 0, 0, 1, 0, 1, 1, 0]);
    assert_eq!(images[&3], [2, 8, 0, 2, 8, 0, 0, 8, 0, 2, 0, 2, 1, 1, 0, 1, 0, 0]);
    assert_eq!(images.len(), 3);

    // With glyph ids retained, A stays in place without a bitmap and the composites keep their
    // bytes, as pyftsubset writes them.
    let kept = subset(&font, &SubsetOptions { text: "XYZ".into(), retain_gids: true, ..SubsetOptions::default() }).unwrap();
    let images = bitmap_images(&kept.font);
    assert_eq!((images.get(&1), &images[&3], &images[&4]), (None, &y.to_vec(), &z.to_vec()));
}
