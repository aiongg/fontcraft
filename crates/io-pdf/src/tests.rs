use fontcraft_model::{Dimension, NodeKind, Path};
use lopdf::{Dictionary, Document, Object, SaveOptions, Stream, dictionary};

use super::*;

const TOP_200: Placement = Placement::Unscaled { top: 200.0 };

fn name(n: &str) -> Object {
    Object::Name(n.as_bytes().to_vec())
}

fn ints(v: &[i64]) -> Object {
    Object::Array(v.iter().map(|x| Object::Integer(*x)).collect())
}

fn stream(dict: Dictionary, content: &[u8], compress: bool) -> Stream {
    let mut s = Stream::new(dict, content.to_vec());
    if compress {
        s.compress().unwrap();
    }
    s
}

/// A document with one page per entry of `contents`, a 200-point square media box and
/// `resources` on the page tree's root, and `page_keys` on the first page.
fn document(contents: &[&[u8]], resources: Dictionary, page_keys: Dictionary, compress: bool) -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for (i, c) in contents.iter().enumerate() {
        let content_id = doc.add_object(stream(Dictionary::new(), c, compress));
        let mut page = dictionary! { "Type" => name("Page"), "Parent" => pages_id, "Contents" => content_id };
        if i == 0 {
            for (k, v) in page_keys.iter() {
                page.set(k.clone(), v.clone());
            }
        }
        kids.push(Object::Reference(doc.add_object(page)));
    }
    let count = kids.len() as i64;
    doc.set_object(
        pages_id,
        dictionary! { "Type" => name("Pages"), "Kids" => kids, "Count" => count, "MediaBox" => ints(&[0, 0, 200, 200]), "Resources" => resources },
    );
    let catalog = doc.add_object(dictionary! { "Type" => name("Catalog"), "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc
}

/// Sets the resources of the page tree's root, which its pages inherit.
fn set_resources(doc: &mut Document, resources: Dictionary) {
    let pages = doc.catalog().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
    doc.get_dictionary_mut(pages).unwrap().set("Resources", resources);
}

fn save(mut doc: Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn one_page(content: &str) -> Vec<u8> {
    save(document(&[content.as_bytes()], Dictionary::new(), Dictionary::new(), false))
}

/// A PDF of `objects` (object 1 the catalog) with a classic cross-reference table.
fn raw_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn nodes(p: &Path) -> Vec<(f64, f64, NodeKind)> {
    p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect()
}

fn err(data: &[u8]) -> PdfError {
    read_pdf(data, TOP_200).unwrap_err()
}

use NodeKind::{Curve, Line, OffCurve};

#[test]
fn pdf_first_page_paths_are_read_in_page_space() {
    let first: &[u8] = b"q 1 0 0 1 10 20 cm 0 0 50 50 re f Q\n100 100 m 150 100 l 150 150 150 150 100 150 c h f";
    let second: &[u8] = b"0 0 10 10 re f";
    // Compressed content, object streams and a cross-reference stream, as current apps write.
    let mut doc = document(&[first, second], Dictionary::new(), Dictionary::new(), true);
    let mut modern = Vec::new();
    doc.save_with_options(&mut modern, SaveOptions::builder().use_object_streams(true).use_xref_streams(true).build()).unwrap();
    let classic = save(document(&[first, second], Dictionary::new(), Dictionary::new(), false));
    for bytes in [modern, classic] {
        assert!(looks_like_pdf(&bytes));
        let out = read_pdf(&bytes, TOP_200).unwrap();
        assert_eq!(out.page_box, Some([0.0, 0.0, 200.0, 200.0]));
        // Only the first page is read.
        assert_eq!(out.paths.len(), 2);
        assert!(out.paths.iter().all(|p| p.closed && p.info.attrs.stroke_width.is_none()));
        assert_eq!(nodes(&out.paths[0]), vec![(10.0, 20.0, Line), (60.0, 20.0, Line), (60.0, 70.0, Line), (10.0, 70.0, Line)]);
        assert_eq!(
            nodes(&out.paths[1]),
            vec![(100.0, 100.0, Line), (150.0, 100.0, Line), (150.0, 150.0, OffCurve), (150.0, 150.0, OffCurve), (100.0, 150.0, Curve)]
        );
        assert_eq!(out.dropped.summary(), None);
    }
}

#[test]
fn pdf_stroke_width_becomes_the_paths_stroke_width() {
    let out = read_pdf(&one_page("2 0 0 2 0 0 cm 3 w 10 10 m 40 10 l S 0 w 10 50 m 40 50 l S 1 0 0 RG 1 w 50 50 20 20 re B"), TOP_200).unwrap();
    assert_eq!(out.paths.len(), 3);
    let [thick, hairline, boxed] = &out.paths[..] else { panic!() };
    assert!(!thick.closed);
    assert_eq!(nodes(thick), vec![(20.0, 20.0, Line), (80.0, 20.0, Line)]);
    assert_eq!(thick.info.attrs.stroke_width, Some(Dimension::Number(6.0)));
    assert!(!thick.info.attrs.fill);
    // A hairline keeps a stroke, one unit wide.
    assert_eq!(hairline.info.attrs.stroke_width, Some(Dimension::Number(1.0)));
    // Filled and stroked: closed, with the stroke width and marked as filled.
    assert!(boxed.closed);
    assert_eq!(nodes(boxed), vec![(100.0, 100.0, Line), (140.0, 100.0, Line), (140.0, 140.0, Line), (100.0, 140.0, Line)]);
    assert_eq!(boxed.info.attrs.stroke_width, Some(Dimension::Number(2.0)));
    assert!(boxed.info.attrs.fill);
    assert_eq!(out.dropped.attributes.get("colour"), Some(&1));
}

#[test]
fn pdf_compressed_stream_bomb_is_refused() {
    // A content stream of 10 MiB of spaces compresses about a thousand times.
    let mut bomb = vec![b' '; 10 << 20];
    bomb.extend_from_slice(b"0 0 10 10 re f");
    let content = save(document(&[&bomb], Dictionary::new(), Dictionary::new(), true));
    assert_eq!(err(&content), PdfError::StreamTooLarge);
    assert_eq!(render_pdf(&content, 1.0, 1 << 20).unwrap_err(), PdfError::StreamTooLarge);

    // An object stream is decoded while the file loads; it is held to the same allowance. The
    // writer drops object streams it did not make, so this file is put together by hand.
    let packed = stream(Dictionary::new(), &vec![b'0'; 10 << 20], true).content;
    let mut objstm = format!("<< /Type /ObjStm /N 1 /First 0 /Filter /FlateDecode /Length {} >>\nstream\n", packed.len()).into_bytes();
    objstm.extend_from_slice(&packed);
    objstm.extend_from_slice(b"\nendstream");
    let object_stream = raw_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        b"<< /Length 14 >>\nstream\n0 0 10 10 re f\nendstream".to_vec(),
        objstm,
    ]);
    assert_eq!(err(&object_stream), PdfError::StreamTooLarge);
    assert!(PdfError::StreamTooLarge.to_string().contains("100 times"));
}

#[test]
fn pdf_text_images_shadings_and_clipping_are_left_out() {
    let content: &[u8] = b"BT /F1 12 Tf (Hi \\) there) Tj ET q 0 0 10 10 re W n 1 0 0 rg 20 20 30 30 re f Q /Im1 Do \
        BI /W 1 /H 1 /BPC 8 /CS /G ID x EI /Sh1 sh 5 5 m 6 6 l n";
    let mut doc = document(&[content], Dictionary::new(), Dictionary::new(), false);
    let image = doc.add_object(stream(
        dictionary! { "Type" => name("XObject"), "Subtype" => name("Image"), "Width" => 1, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => name("DeviceGray") },
        &[255],
        false,
    ));
    set_resources(&mut doc, dictionary! { "XObject" => dictionary! { "Im1" => image } });
    let out = read_pdf(&save(doc), TOP_200).unwrap();
    assert_eq!(out.paths.len(), 1);
    assert_eq!(nodes(&out.paths[0])[0], (20.0, 20.0, Line));
    let e = &out.dropped.elements;
    assert_eq!(
        (e.get("text"), e.get("clipping path"), e.get("image"), e.get("shading"), e.get("unpainted path")),
        (Some(&1), Some(&1), Some(&2), Some(&1), Some(&1))
    );
    assert_eq!(out.dropped.attributes.get("colour"), Some(&1));
    assert!(out.dropped.summary().unwrap().contains("text"));
}

#[test]
fn pdf_form_xobjects_draw_with_their_matrix_and_cannot_draw_themselves() {
    let mut doc = document(&[b"1 0 0 1 0 100 cm /Fm1 Do /Fm1 Do"], Dictionary::new(), Dictionary::new(), false);
    let form = doc.add_object(stream(
        dictionary! { "Type" => name("XObject"), "Subtype" => name("Form"), "BBox" => ints(&[0, 0, 100, 100]), "Matrix" => ints(&[1, 0, 0, 1, 50, 0]) },
        b"0 0 10 10 re f",
        true,
    ));
    set_resources(&mut doc, dictionary! { "XObject" => dictionary! { "Fm1" => form } });
    let out = read_pdf(&save(doc), TOP_200).unwrap();
    assert_eq!(out.paths.len(), 2);
    for p in &out.paths {
        assert_eq!(nodes(p), vec![(50.0, 100.0, Line), (60.0, 100.0, Line), (60.0, 110.0, Line), (50.0, 110.0, Line)]);
    }

    let mut doc = document(&[b"/Me Do"], Dictionary::new(), Dictionary::new(), false);
    let me = doc.new_object_id();
    doc.set_object(
        me,
        stream(
            dictionary! { "Subtype" => name("Form"), "BBox" => ints(&[0, 0, 1, 1]), "Resources" => dictionary! { "XObject" => dictionary! { "Me" => me } } },
            b"/Me Do",
            false,
        ),
    );
    set_resources(&mut doc, dictionary! { "XObject" => dictionary! { "Me" => me } });
    assert_eq!(err(&save(doc)), PdfError::TooDeep);
}

#[test]
fn pdf_crop_box_rotation_and_fit_em_place_the_page() {
    let rotated =
        save(document(&[b"0 0 10 10 re f"], Dictionary::new(), dictionary! { "MediaBox" => ints(&[0, 0, 400, 200]), "Rotate" => 90 }, false));
    let out = read_pdf(&rotated, Placement::FitEm { units_per_em: 1000.0, ascender: 800.0 }).unwrap();
    assert_eq!(out.page_box, Some([0.0, 0.0, 400.0, 200.0]));
    // Turned a quarter clockwise, the page is 200 wide and 400 high; its height becomes the em.
    assert_eq!(nodes(&out.paths[0]), vec![(0.0, 800.0, Line), (0.0, 775.0, Line), (25.0, 775.0, Line), (25.0, 800.0, Line)]);

    let cropped = save(document(&[b"0 0 10 10 re f"], Dictionary::new(), dictionary! { "CropBox" => ints(&[0, 0, 100, 100]) }, false));
    let out = read_pdf(&cropped, TOP_200).unwrap();
    assert_eq!(out.page_box, Some([0.0, 0.0, 100.0, 100.0]));
    assert_eq!(nodes(&out.paths[0]), vec![(0.0, 100.0, Line), (10.0, 100.0, Line), (10.0, 110.0, Line), (0.0, 110.0, Line)]);

    // Origin puts the frame's bottom-left corner at the glyph's origin.
    let offset = save(document(&[b"60 30 10 10 re f"], Dictionary::new(), dictionary! { "CropBox" => ints(&[50, 20, 150, 120]) }, false));
    let out = read_pdf(&offset, Placement::Origin).unwrap();
    assert_eq!(nodes(&out.paths[0]), vec![(10.0, 10.0, Line), (20.0, 10.0, Line), (20.0, 20.0, Line), (10.0, 20.0, Line)]);
}

#[test]
fn pdf_hostile_input_fails_with_a_message() {
    assert_eq!(err(b"%!PS-Adobe-3.0\n"), PdfError::NotPdf);
    assert_eq!(err(&one_page(&format!("{}0 0 10 10 re f", "q ".repeat(MAX_PDF_DEPTH + 1)))), PdfError::TooDeep);
    assert_eq!(err(&one_page("99999999999 0 m 0 0 l S")), PdfError::NumberOutOfRange);
    assert_eq!(err(&one_page("1000 0 0 1000 0 0 cm 1500 1500 m 0 0 l S")), PdfError::CoordinateOutOfRange);
    assert_eq!(err(&save(document(&[], Dictionary::new(), Dictionary::new(), false))), PdfError::NoPage);
    assert_eq!(
        read_pdf(&one_page("0 0 10 10 re f"), Placement::FitEm { units_per_em: 0.0, ascender: 800.0 }).unwrap_err().to_string(),
        "the placement is not usable: units per em 0 and ascender 800 must be finite, with a positive em"
    );
    // Cut short, unbalanced or garbled files fail or read what they can, without a panic.
    let whole = one_page("0 0 10 10 re f ( unbalanced <ab [ << /K");
    for end in [10, whole.len() / 2, whole.len() - 20] {
        let _ = read_pdf(&whole[..end], TOP_200);
    }
    assert_eq!(read_pdf(&whole, TOP_200).unwrap().paths.len(), 1);
}

#[test]
fn pdf_page_renders_as_an_image() {
    let content = "0 0 0 rg 0 0 100 100 re f 1 1 1 rg 25 25 50 50 re f \
        0 0 1 rg 120 0 60 60 re 135 15 30 30 re f* \
        1 0 0 RG 4 w 100 90 m 190 90 l S";
    let bytes = save(document(&[content.as_bytes()], Dictionary::new(), dictionary! { "MediaBox" => ints(&[0, 0, 200, 100]) }, false));
    let out = render_pdf(&bytes, 1.0, 1 << 20).unwrap();
    assert_eq!((out.image.width, out.image.height, out.pixels_per_point), (200, 100, 1.0));
    let px = |x: usize, row: usize| -> [u8; 4] {
        let at = (row * 200 + x) * 4;
        out.image.pixels[at..at + 4].try_into().unwrap()
    };
    assert_eq!(px(10, 95), [0, 0, 0, 255]);
    assert_eq!(px(50, 50), [255, 255, 255, 255]);
    assert_eq!(px(125, 80), [0, 0, 255, 255]);
    // The even-odd hole and the unpainted corner stay transparent.
    assert_eq!(px(150, 80)[3], 0);
    assert_eq!(px(190, 30)[3], 0);
    assert_eq!(px(150, 10), [255, 0, 0, 255]);

    // A page that would have more pixels than allowed is drawn at a lower resolution.
    let small = render_pdf(&bytes, 4.0, 200 * 100).unwrap();
    assert_eq!((small.image.width, small.image.height, small.pixels_per_point), (200, 100, 1.0));
}

/// A page of a file is drawn by its number; pages past the last are refused.
#[test]
fn any_page_of_a_pdf_is_drawn() {
    let contents: [&[u8]; 3] = [b"1 0 0 rg 0 0 200 200 re f", b"0 1 0 rg 0 0 200 200 re f", b"0 0 1 rg 0 0 200 200 re f"];
    let bytes = save(document(&contents, Dictionary::new(), Dictionary::new(), true));
    let inks = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
    for (page, ink) in inks.into_iter().enumerate() {
        let drawn = render_pdf_page(&bytes, page, 0.1, 1 << 20).unwrap();
        let middle = drawn.image.pixels.len() / 8 * 4 + 40;
        assert_eq!(drawn.image.pixels[middle..middle + 4], ink, "page {page}");
    }
    assert_eq!(render_pdf_page(&bytes, 3, 0.1, 1 << 20).unwrap_err(), PdfError::NoPage);
    assert_eq!(render_pdf_page(&bytes, usize::MAX, 0.1, 1 << 20).unwrap_err(), PdfError::NoPage);
}

#[test]
fn pdf_page_drawn_smaller_keeps_its_bottom_edge_on_the_image_bottom() {
    // 100.1 points tall at about half a pixel per point: 49.5 pixel rows in a 50-row image. The
    // bottom 10 points are black and must reach the image's last row whole, as the traced or
    // shown image is placed from its bottom-left corner.
    let page = Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Integer(20), Object::Real(100.1)]);
    let bytes = save(document(&["0 0 0 rg 0 0 20 10 re f".as_bytes()], Dictionary::new(), dictionary! { "MediaBox" => page }, false));
    let out = render_pdf(&bytes, 4.0, 500).unwrap();
    assert!(out.pixels_per_point < 0.5, "{}", out.pixels_per_point);
    let (w, h) = (out.image.width as usize, out.image.height as usize);
    let alpha = |row: usize| out.image.pixels[(row * w + w / 2) * 4 + 3];
    assert!(alpha(h - 1) >= 250, "the last row is whole: {}", alpha(h - 1));
    let top = (10.0 * out.pixels_per_point).ceil() as usize;
    assert_eq!(alpha(h - 1 - top), 0, "nothing above 10 points");
}

#[test]
fn pdf_parsed_once_for_drawing_and_measuring_and_the_cache_is_bounded() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::cache::{self, PageCache};

    // Drawing and measuring go through the process's cache, which other tests fill at the same
    // time, so only their agreement is checked here.
    let bytes =
        save(document(&["0 0 0 rg 3 7 11 13 re f".as_bytes()], Dictionary::new(), dictionary! { "MediaBox" => ints(&[0, 0, 37, 41]) }, false));
    let drawn = render_pdf(&bytes, 1.0, 1 << 20).unwrap();
    assert_eq!(page_size(&bytes).unwrap(), (37.0, 41.0));
    assert_eq!((drawn.image.width, drawn.image.height), (37, 41), "drawn at the size measured");

    // A cache of its own counts the parses.
    let mine = Mutex::new(PageCache::new());
    let parses = AtomicUsize::new(0);
    let get = |data: &[u8]| {
        PageCache::get(&mine, data, |d| {
            parses.fetch_add(1, Ordering::Relaxed);
            crate::load::first_page_drawing(d)
        })
    };
    let pdfs: Vec<Vec<u8>> = (0..=MAX_PDF_CACHE_ENTRIES).map(|i| one_page(&format!("0 0 {} 10 re f", i + 1))).collect();
    let a = get(&pdfs[0]).unwrap();
    let b = get(&pdfs[0]).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(parses.load(Ordering::Relaxed), 1, "the same bytes are parsed once");
    // A failure is kept too.
    assert!(get(b"%PDF-1.5\n1 0 obj").is_err());
    assert!(get(b"%PDF-1.5\n1 0 obj").is_err());
    assert_eq!(parses.load(Ordering::Relaxed), 2);
    // Two more files than the cache holds drop the two least recently used.
    for pdf in &pdfs {
        get(pdf).unwrap();
    }
    let held = cache::lock(&mine);
    assert_eq!(held.len(), MAX_PDF_CACHE_ENTRIES);
    assert!(!held.holds(b"%PDF-1.5\n1 0 obj") && !held.holds(&pdfs[0]));
    assert!(pdfs[1..].iter().all(|pdf| held.holds(pdf)));
    assert_eq!(parses.load(Ordering::Relaxed), 2 + MAX_PDF_CACHE_ENTRIES);
}
