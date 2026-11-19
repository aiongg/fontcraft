//! The ported layouts on fonts built in code: pages, glyphs per page, glyph order, the elements
//! each layout draws, links from the palette to the outline pages, and the font-set rows; the
//! dumper's sheets (synopsis, cmap, vertical origins, text, feature rules), `show`, `hide` and
//! the page header.

use std::sync::Arc;

use fontcraft_compile::{Flavour, Options, compile, compile_ttf};
use fontcraft_model::{Font, Path, Shape, Snippet};

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

/// `ABC` with a ligature, an alternate and two kerning pairs.
fn features_font() -> Vec<u8> {
    let mut f = model("Feat", ABC);
    let features = Arc::make_mut(&mut f.features);
    for (tag, code) in [("liga", "sub A B by I;"), ("salt", "sub A by B;"), ("kern", "pos A B -50; pos B A 20;")] {
        features.features.push(Snippet { name: tag.into(), code: code.into(), ..Default::default() });
    }
    compile_ttf(&f, &Options::default()).unwrap().bytes
}

/// The origins of the glyphs a page is about, in drawing order.
fn origins(doc: &Document, page: usize) -> Vec<[f64; 2]> {
    doc.pages[page].items.iter().filter_map(|it| if let Item::Glyph { x, y, primary: true, .. } = it { Some([*x, *y]) } else { None }).collect()
}

fn rects(doc: &Document, page: usize) -> usize {
    doc.pages[page].items.iter().filter(|it| matches!(it, Item::Rect { .. })).count()
}

#[test]
fn synopsis_cmap_vorg_and_text_show_their_glyphs_in_order() {
    let font = vec![ttf("Spot", ABC)];
    let order = [".notdef", "A", "B", "I"];

    let synopsis = layout(&font, &sheet(Sheet::Synopsis));
    assert_eq!(synopsis.pages.len(), 1, "320 cells per page");
    assert_eq!(primary(&synopsis, 0), order);
    let text = labels(&synopsis, 0);
    assert!(text.iter().any(|l| l == "A") && text.iter().any(|l| l == "600") && text.iter().any(|l| l == "3"), "{text:?}");
    assert!(text.iter().any(|l| l.ends_with("synopsis \u{2014} page 1 of 1")), "the title names the sheet: {text:?}");
    let one = layout(&font, &ProofOptions { glyphs_per_page: Some(1), ..sheet(Sheet::Synopsis) });
    assert_eq!(one.pages.len(), 4, "one cell per page");

    // Code-point order whatever the selection's order; .notdef has no code point.
    let cmap = layout(&font, &ProofOptions { glyphs: vec!["I".into(), "A".into()], ..sheet(Sheet::Cmap) });
    assert_eq!(primary(&cmap, 0), ["A", "I"]);
    assert!(labels(&cmap, 0).iter().any(|l| l == "A@1 0041"), "{:?}", labels(&cmap, 0));
    let unmapped = ProofOptions { glyphs: vec![".notdef".into()], ..sheet(Sheet::Cmap) };
    assert!(proof(&font[0], 0, &unmapped).is_err(), "nothing to show");

    // Without VORG every glyph is shown, each in its vertical em box.
    let vorg = layout(&font, &sheet(Sheet::Vorg));
    assert_eq!(primary(&vorg, 0), order);
    assert_eq!(rects(&vorg, 0), 4, "one em box per glyph");
    assert!(labels(&vorg, 0).iter().any(|l| l.starts_with("adv ")));

    // Text: the glyphs of the characters in order, a line break back at the left margin.
    let text = layout(&font, &sheet(Sheet::Text { text: "BIA\nA".into() }));
    assert_eq!(primary(&text, 0), ["B", "I", "A", "A"]);
    let at = origins(&text, 0);
    assert!(at[1][0] > at[0][0] && at[1][1] == at[0][1], "I follows B on the line");
    assert!(at[3][0] == at[0][0] && at[3][1] > at[0][1], "the second line starts under the first");
    assert_eq!(labels(&text, 0).len(), 1, "the text sheet labels nothing but the title");
    let glyphs = layout(&font, &sheet(Sheet::Text { text: String::new() }));
    assert_eq!(primary(&glyphs, 0), order, "without text the glyphs to show are set");

    for sheet_kind in [Sheet::Synopsis, Sheet::Cmap, Sheet::Vorg, Sheet::Text { text: "AB".into() }] {
        let pdf = proof(&font[0], 0, &ProofOptions { format: ProofFormat::Pdf, ..sheet(sheet_kind.clone()) }).unwrap();
        assert!(pdf.files[0].starts_with(b"%PDF-"), "{sheet_kind:?}");
    }
}

#[test]
fn show_hide_and_no_header_change_what_is_drawn() {
    let font = vec![ttf("Shown", ABC)];
    let plain = layout(&font, &sheet(Sheet::Synopsis));
    let boxed = layout(
        &font,
        &ProofOptions {
            show: vec!["em-box".into(), "bbox".into()],
            hide: vec!["name".into(), "numeric-labels".into()],
            header: false,
            ..sheet(Sheet::Synopsis)
        },
    );
    assert_eq!(rects(&plain, 0), 4, "one cell border per glyph");
    assert!(rects(&boxed, 0) >= 4 + 4 + 3, "cells, em boxes and the bounding boxes of A, B and I");
    assert!(labels(&boxed, 0).is_empty(), "no names, no numbers and no title: {:?}", labels(&boxed, 0));
    // The same element names reach the tile layouts.
    let tiles = layout(&font, &ProofOptions { hide: vec!["sidebearings".into(), "name".into()], ..sheet(Sheet::Boxes) });
    assert_eq!(labels(&tiles, 0).len(), 1, "only the title is left: {:?}", labels(&tiles, 0));
    let bad = ProofOptions { show: vec!["glitter".into()], ..sheet(Sheet::Synopsis) };
    assert!(proof(&font[0], 0, &bad).is_err());
}

#[test]
fn features_sheet_draws_each_rule_under_its_feature_and_lookup() {
    let font = vec![features_font()];
    let all = layout(&font, &sheet(Sheet::Features { tags: Vec::new(), font_order: false }));
    assert_eq!(all.pages.len(), 3, "one page per feature");
    let page_of = |doc: &Document, heading: &str| (0..doc.pages.len()).find(|p| labels(doc, *p).iter().any(|l| l == heading)).unwrap();
    let liga = page_of(&all, "GSUB feature liga");
    assert_eq!(primary(&all, liga), ["A", "B", "I"], "the components, then the ligature");
    assert!(labels(&all, liga).iter().any(|l| l.starts_with("lookup ") && l.contains("ligature substitution") && l.contains("DFLT/dflt")));
    assert!(labels(&all, liga).iter().any(|l| l == "+"));
    let salt = page_of(&all, "GSUB feature salt");
    assert_eq!(primary(&all, salt), ["A", "B"]);
    let kern = page_of(&all, "GPOS feature kern");
    assert_eq!(primary(&all, kern), ["A", "B", "B", "A"], "pairs by the first glyph's id");
    assert!(labels(&all, kern).iter().any(|l| l == "-50") && labels(&all, kern).iter().any(|l| l == "20"));
    let at = origins(&all, kern);
    let upem = 1000.0;
    let scale = super::spot::DEFAULT_SIZE / upem;
    assert!((at[1][0] - at[0][0] - 550.0 * scale).abs() < 1e-6, "B is set 50 units closer: {at:?}");

    // One feature, the rules whose first glyph is selected.
    let only = layout(&font, &ProofOptions { glyphs: vec!["B".into()], ..sheet(Sheet::Features { tags: vec![*b"kern"], font_order: true }) });
    assert_eq!(only.pages.len(), 1);
    assert_eq!(primary(&only, 0), ["B", "A"]);
    let missing = ProofOptions { format: ProofFormat::Pdf, ..sheet(Sheet::Features { tags: vec![*b"smcp"], font_order: false }) };
    assert!(proof(&font[0], 0, &missing).is_err(), "a feature the font lacks is refused");
    let pdf =
        proof(&font[0], 0, &ProofOptions { format: ProofFormat::Pdf, ..sheet(Sheet::Features { tags: Vec::new(), font_order: false }) }).unwrap();
    assert_eq!(pdf.pages, 3);
}
