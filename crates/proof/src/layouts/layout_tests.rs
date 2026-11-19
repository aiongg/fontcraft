//! The ported layouts on fonts built in code: pages, glyphs per page, glyph order, the elements
//! each layout draws, links from the palette to the outline pages, and the font-set rows.

use fontcraft_compile::{Flavour, Options, compile, compile_ttf};
use fontcraft_model::{Font, Path, Shape};

use crate::font::{self, Face};
use crate::page::{self, Document, Ink, Item};
use crate::{ProofFormat, ProofOptions, Sheet, proof};

/// A font with `.notdef` and the given glyphs, each a rectangle `x0..x1` by `0..700`.
fn model(family: &str, glyphs: &[(&str, u32, f64, f64)]) -> Font {
    let mut f = Font::new(family);
    f.glyphs.retain(|g| g.name == ".notdef");
    for &(name, cp, x0, x1) in glyphs {
        f.add_glyph(name, vec![cp], 600.0).unwrap();
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x0, 0.0, x1, 700.0)));
    }
    f
}

fn ttf(family: &str, glyphs: &[(&str, u32, f64, f64)]) -> Vec<u8> {
    compile_ttf(&model(family, glyphs), &Options::default()).unwrap().bytes
}

/// The same glyphs as CFF, hinted by the export's autohinter.
fn otf(family: &str, glyphs: &[(&str, u32, f64, f64)]) -> Vec<u8> {
    let f = model(family, glyphs);
    compile(&f, &f.default_target().unwrap(), Flavour::Cff, &Options::default()).unwrap().bytes
}

const ABC: &[(&str, u32, f64, f64)] = &[("A", 0x41, 60.0, 540.0), ("B", 0x42, 70.0, 520.0), ("I", 0x49, 100.0, 180.0)];

fn layout(fonts: &[Vec<u8>], options: &ProofOptions) -> Document {
    let loaded: Vec<_> = fonts.iter().map(|b| font::load(b, 0).unwrap()).collect();
    let locations: Vec<_> = loaded.iter().map(|l| font::location(&font::font_ref(&l.sfnt).unwrap(), &[]).unwrap()).collect();
    let faces: Vec<Face<'_>> = loaded.iter().zip(&locations).map(|(l, loc)| Face::new(font::font_ref(&l.sfnt).unwrap(), loc).unwrap()).collect();
    let gids = faces[0].resolve(&options.glyphs).unwrap();
    page::layout(&faces, &gids, options, None).unwrap()
}

fn sheet(sheet: Sheet) -> ProofOptions {
    ProofOptions { sheets: vec![sheet], sizes: Vec::new(), ..ProofOptions::default() }
}

/// The names of the glyphs a page is about, in drawing order.
fn primary(doc: &Document, page: usize) -> Vec<String> {
    doc.pages[page]
        .items
        .iter()
        .filter_map(|it| match it {
            Item::Glyph { glyph, primary: true, .. } => Some(doc.glyphs[*glyph].name.clone()),
            _ => None,
        })
        .collect()
}

fn labels(doc: &Document, page: usize) -> Vec<String> {
    doc.pages[page].items.iter().filter_map(|it| if let Item::Label { text, .. } = it { Some(text.clone()) } else { None }).collect()
}

/// How many items of each kind a page holds: a small snapshot of its frame.
fn summary(doc: &Document, page: usize) -> String {
    let mut n = [0usize; 7];
    for it in &doc.pages[page].items {
        let k = match it {
            Item::Glyph { .. } => 0,
            Item::Line { .. } => 1,
            Item::Rect { .. } => 2,
            Item::Point { .. } => 3,
            Item::Label { .. } => 4,
            Item::Link { .. } => 5,
            Item::Sample { .. } => 6,
        };
        n[k] += 1;
    }
    format!("glyph {}, line {}, rect {}, point {}, label {}, link {}, sample {}", n[0], n[1], n[2], n[3], n[4], n[5], n[6])
}

#[test]
fn boxes_hints_and_digi_follow_glyph_order_with_their_glyphs_per_page() {
    let font = vec![ttf("Tiles", ABC)];
    let order = [".notdef", "A", "B", "I"];

    let boxes = layout(&font, &sheet(Sheet::Boxes));
    assert_eq!(boxes.pages.len(), 1, "156 glyphs per page");
    assert_eq!(primary(&boxes, 0), order);
    // Per glyph: the glyph, four centre marks, the advance box, name and side bearings; then the title.
    assert_eq!(summary(&boxes, 0), "glyph 4, line 16, rect 4, point 0, label 9, link 0, sample 0");
    assert!(labels(&boxes, 0).iter().any(|l| l == "L = 60, R = 60 Width = 600"), "{:?}", labels(&boxes, 0));

    let hints = layout(&font, &sheet(Sheet::Hints));
    assert_eq!(hints.pages.len(), 1, "four glyphs per page");
    assert_eq!(primary(&hints, 0), order);
    let two = layout(&font, &ProofOptions { glyphs_per_page: Some(2), ..sheet(Sheet::Hints) });
    assert_eq!(two.pages.len(), 2);
    assert_eq!(primary(&two, 1), ["B", "I"]);

    let digi = layout(&font, &sheet(Sheet::Digi));
    assert_eq!(digi.pages.len(), 4, "one glyph per page");
    for (page, name) in order.iter().enumerate() {
        assert_eq!(primary(&digi, page), [*name], "the left copy is the glyph the page is about");
    }
    let page = &digi.pages[1].items;
    let copies = page.iter().filter(|it| matches!(it, Item::Glyph { glyph: 1, .. })).count();
    assert!(copies >= 2, "both copies of A are drawn");
    assert!(page.iter().any(|it| matches!(it, Item::Point { on_curve: true, .. })), "the right copy marks points");
    assert!(labels(&digi, 1).iter().any(|l| l == "P 0"), "the right copy labels contours");
}

#[test]
fn hints_layout_draws_the_stems_of_a_cff_font_and_filters_by_group() {
    let font = vec![otf("Hinted", ABC)];
    let options = ProofOptions { glyphs: vec!["I".into()], ..sheet(Sheet::Hints) };
    let doc = layout(&font, &options);
    let text = labels(&doc, 0);
    assert!(text.iter().any(|l| l.starts_with("v(100, 180)")), "the stem of I: {text:?}");
    assert!(text.iter().any(|l| l.starts_with("Hints:  ") && !l.starts_with("Hints:  0")), "{text:?}");
    // A group the glyph does not have hides every stem.
    let filtered = layout(&font, &ProofOptions { hint_groups: vec![7], ..options.clone() });
    assert!(!labels(&filtered, 0).iter().any(|l| l.starts_with("v(") || l.starts_with("h(")));
    // A TrueType font has no PostScript hints.
    let tt = layout(&[ttf("Plain", ABC)], &options);
    assert!(labels(&tt, 0).iter().any(|l| l == "Hints:  0 horiz: 0, vert: 0"));
}

#[test]
fn palette_with_outlines_links_each_cell_to_its_outline_page() {
    let font = vec![ttf("Linked", ABC)];
    let doc = layout(&font, &ProofOptions { sizes: vec![60.0], ..sheet(Sheet::GridWithOutlines) });
    assert_eq!(doc.pages.len(), 1 + 4, "one grid page, then one outline page per glyph");
    let links: Vec<usize> = doc.pages[0].items.iter().filter_map(|it| if let Item::Link { page, .. } = it { Some(*page) } else { None }).collect();
    assert_eq!(links, [1, 2, 3, 4]);
    for (i, target) in links.iter().enumerate() {
        assert_eq!(primary(&doc, *target), [doc.glyphs[i].name.clone()], "the link of cell {i} opens that glyph's page");
    }
    let pdf = proof(&font[0], 0, &ProofOptions { format: ProofFormat::Pdf, sizes: vec![60.0], ..sheet(Sheet::GridWithOutlines) }).unwrap();
    let annotations = pdf.files[0].windows(14).filter(|w| w == b"/Subtype /Link").count();
    assert_eq!(annotations, 4, "a link annotation per cell");
}

#[test]
fn compare_sets_one_row_per_font_sorted_by_glyph_count() {
    let big = ttf("Big", ABC);
    let small = ttf("Small", &[("A", 0x41, 60.0, 540.0)]);
    let doc = layout(&[big, small], &sheet(Sheet::Compare));
    assert_eq!(doc.pages.len(), 1);
    let text = labels(&doc, 0);
    assert_eq!(text[0], "Glyph group 1 of 1");
    assert!(text[1].ends_with(" 0") && text[1].contains("Small"), "the font with fewer glyphs comes first: {text:?}");
    assert!(text[2].ends_with(" 1") && text[2].contains("Big"), "{text:?}");
    let fonts: Vec<usize> =
        doc.pages[0].items.iter().filter_map(|it| if let Item::Glyph { glyph, .. } = it { Some(doc.glyphs[*glyph].font) } else { None }).collect();
    assert_eq!(fonts, [1, 1, 0, 0, 0, 0], "two glyphs of the small font, then four of the big one");
}

#[test]
fn black_turns_every_mark_black_or_grey() {
    let font = vec![ttf("Black", ABC)];
    let doc = layout(&font, &ProofOptions { black: true, ..sheet(Sheet::GlyphPerPage) });
    for it in doc.pages.iter().flat_map(|p| &p.items) {
        if let Item::Point { ink, .. } | Item::Line { ink, .. } | Item::Label { ink, .. } | Item::Rect { ink, .. } = it {
            assert!(matches!(ink, Ink::Text | Ink::Rgb([0x80, 0x80, 0x80])), "{ink:?}");
        }
    }
    let hints = layout(&font, &ProofOptions { black: true, ..sheet(Sheet::Hints) });
    for it in &hints.pages[0].items {
        if let Item::Rect { ink: Ink::Rgb(c), .. } = it {
            assert!(c[0] == c[1] && c[1] == c[2], "grey or black: {c:?}");
        }
    }
}
