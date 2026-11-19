//! The PDF writer: every page of a proof in one PDF 1.7 document, each page at its own size.
//!
//! - Glyph outlines are vector paths in font units. A filled glyph is a form XObject written once
//!   for the document and placed with a matrix wherever it appears; an outlined glyph (outline
//!   pages) is drawn in place with a light fill and a dark contour. TrueType curves become cubic
//!   curves, which draw the same shape.
//! - Each glyph a cell or page is about sits in a marked-content sequence
//!   `/Glyph << /Name (A) /GID 36 >> BDC … EMC`, so the glyph order can be read back from the
//!   file, as `data-glyph` gives it in SVG.
//! - Labels and titles are set in the standard Helvetica font, which every PDF viewer has, in
//!   WinAnsi encoding; a character outside that encoding prints as `?`.
//! - Waterfall text is drawn as outlines, not set in an embedded font: each character's glyph
//!   comes from `cmap` (`.notdef` when unmapped), its advance from the font at the proof's
//!   location, and the space between two glyphs gets their pair kerning ([`crate::kern`]). Each
//!   line carries its text as `/ActualText`, so it can be searched and copied.
//! - A link item becomes a `/Link` annotation that opens its target page.
//! - Content streams are written uncompressed.
//!
//! Page items are in points from the top left with y growing downwards; PDF's origin is the
//! bottom left with y growing upwards, so every y becomes `page height - y`.

use std::collections::{BTreeMap, HashMap};

use pdf_writer::types::{ActionType, AnnotationType};
use pdf_writer::{Content, Name, Pdf, Rect, Ref, Str, TextStr};

use crate::ProofError;
use crate::font::{El, Face, Outline};
use crate::kern::Kerning;
use crate::page::{Anchor, Document, GlyphStyle, Item, LINE_WIDTH, OUTLINED_FILL, POINT_RADIUS, Page};

/// The resource name of the label font.
const LABEL_FONT: Name<'static> = Name(b"F1");
/// The marked-content tag around a shown glyph.
pub(crate) const GLYPH_TAG: &[u8] = b"Glyph";
/// The marked-content tag around a line of waterfall text.
const SPAN_TAG: Name<'static> = Name(b"Span");
/// The document information's producer.
const PRODUCER: &str = "FontCraft";
/// Contour width of an outlined glyph, in points.
const OUTLINE_WIDTH: f64 = 0.6;
/// Handle length of a quarter circle drawn with one cubic curve, as a fraction of the radius.
const KAPPA: f64 = 0.552_284_75;
/// Largest absolute coordinate written; the layout never comes near it.
const MAX_COORDINATE: f64 = 1e7;
/// Characters 0x80 to 0x9F of WinAnsi encoding (PDF 32000-1, annex D.2) that are not Latin-1.
const WIN_ANSI_HIGH: [(char, u8); 27] = [
    ('\u{20AC}', 0x80),
    ('\u{201A}', 0x82),
    ('\u{0192}', 0x83),
    ('\u{201E}', 0x84),
    ('\u{2026}', 0x85),
    ('\u{2020}', 0x86),
    ('\u{2021}', 0x87),
    ('\u{02C6}', 0x88),
    ('\u{2030}', 0x89),
    ('\u{0160}', 0x8A),
    ('\u{2039}', 0x8B),
    ('\u{0152}', 0x8C),
    ('\u{017D}', 0x8E),
    ('\u{2018}', 0x91),
    ('\u{2019}', 0x92),
    ('\u{201C}', 0x93),
    ('\u{201D}', 0x94),
    ('\u{2022}', 0x95),
    ('\u{2013}', 0x96),
    ('\u{2014}', 0x97),
    ('\u{02DC}', 0x98),
    ('\u{2122}', 0x99),
    ('\u{0161}', 0x9A),
    ('\u{203A}', 0x9B),
    ('\u{0153}', 0x9C),
    ('\u{017E}', 0x9E),
    ('\u{0178}', 0x9F),
];

/// Writes every page of `doc` as one PDF. `face` is the font the document was laid out from; it
/// gives the outlines, advances and kerning of waterfall text.
pub(crate) fn write(doc: &Document, face: &Face<'_>) -> Result<Vec<u8>, ProofError> {
    let mut next = Ref::new(1);
    let catalog = next.bump();
    let tree = next.bump();
    let label_font = next.bump();
    let info = next.bump();
    let page_refs: Vec<Ref> = doc.pages.iter().map(|_| next.bump()).collect();
    let count = i32::try_from(page_refs.len()).map_err(|_| ProofError::BadOptions("the proof has too many pages for one PDF".into()))?;

    let mut pdf = Pdf::new();
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids(page_refs.iter().copied()).count(count);
    pdf.type1_font(label_font).base_font(Name(b"Helvetica")).encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.document_info(info).title(TextStr(&doc.title)).producer(TextStr(PRODUCER));

    let mut writer =
        Writer { doc, face, glyphs: Glyphs::new(doc), kerning: Kerning::new(&face.font), shaped: HashMap::new(), forms: BTreeMap::new(), next };
    for (page, &page_ref) in doc.pages.iter().zip(&page_refs) {
        let (content, used, links) = writer.page(page)?;
        let content_ref = writer.next.bump();
        pdf.stream(content_ref, &content);
        let annotations: Vec<(Ref, Rect, Ref)> =
            links.into_iter().filter_map(|(rect, target)| page_refs.get(target).map(|t| (writer.next.bump(), rect, *t))).collect();
        let mut p = pdf.page(page_ref);
        p.parent(tree).media_box(Rect::new(0.0, 0.0, coord(page.width), coord(page.height))).contents(content_ref);
        if !annotations.is_empty() {
            p.annotations(annotations.iter().map(|a| a.0));
        }
        let mut resources = p.resources();
        resources.fonts().pair(LABEL_FONT, label_font);
        if !used.is_empty() {
            let mut objects = resources.x_objects();
            for (key, form) in &used {
                objects.pair(Name(form_name(*key).as_bytes()), *form);
            }
        }
        drop(resources);
        drop(p);
        for (id, rect, target) in annotations {
            let mut annotation = pdf.annotation(id);
            annotation.subtype(AnnotationType::Link).rect(rect).border(0.0, 0.0, 0.0, None);
            annotation.action().action_type(ActionType::GoTo).destination().page(target).fit();
        }
    }
    for (key, form) in &writer.forms {
        let Some(outline) = writer.glyphs.get(*key) else { continue };
        let mut c = Content::new();
        path(&mut c, &outline.els);
        c.fill_nonzero();
        let bytes = c.finish();
        let [x0, y0, x1, y1] = outline.bounds.unwrap_or_default();
        let mut object = pdf.form_xobject(*form, &bytes);
        object.bbox(Rect::new(coord(x0), coord(y0), coord(x1), coord(y1)));
        object.resources();
    }
    Ok(pdf.finish())
}

/// The outlines the pages draw, by glyph id: the shown glyphs' from the document, the others
/// (waterfall text) read from the font when first needed.
struct Glyphs<'d> {
    /// By font of the set and glyph id.
    shown: HashMap<GlyphKey, &'d Outline>,
    loaded: HashMap<u32, Outline>,
    /// Outline segments the waterfall glyphs may still use.
    budget: usize,
}

impl<'d> Glyphs<'d> {
    fn new(doc: &'d Document) -> Self {
        let shown = doc.glyphs.iter().map(|g| ((g.font, g.gid), &g.outline)).collect();
        Glyphs { shown, loaded: HashMap::new(), budget: crate::font::MAX_TOTAL_ELEMENTS }
    }

    fn get(&self, key: GlyphKey) -> Option<&Outline> {
        self.shown.get(&key).copied().or_else(|| if key.0 == 0 { self.loaded.get(&key.1) } else { None })
    }

    fn load(&mut self, face: &Face<'_>, gid: u32) -> Result<(), ProofError> {
        if self.shown.contains_key(&(0, gid)) || self.loaded.contains_key(&gid) {
            return Ok(());
        }
        let outline = face.outline(gid, &mut self.budget)?;
        self.loaded.insert(gid, outline);
        Ok(())
    }
}

struct Writer<'d, 'f> {
    doc: &'d Document,
    face: &'d Face<'f>,
    glyphs: Glyphs<'d>,
    kerning: Kerning<'f>,
    /// Waterfall lines already set: each glyph with its x offset in font units.
    shaped: HashMap<String, Vec<(u32, f64)>>,
    /// The form XObject of each filled glyph, written after the pages.
    forms: BTreeMap<GlyphKey, Ref>,
    next: Ref,
}

impl Writer<'_, '_> {
    /// The content stream of `page`, the glyph forms it uses and its links (area, target page).
    #[allow(clippy::type_complexity)]
    fn page(&mut self, page: &Page) -> Result<(Vec<u8>, BTreeMap<GlyphKey, Ref>, Vec<(Rect, usize)>), ProofError> {
        let h = page.height;
        let mut c = Content::new();
        let mut used = BTreeMap::new();
        let mut links = Vec::new();
        for item in &page.items {
            match item {
                Item::Glyph { glyph, x, y, scale, style, primary } => {
                    let doc = self.doc;
                    let Some(g) = doc.glyphs.get(*glyph) else { continue };
                    if *primary {
                        let gid = i32::try_from(g.gid).unwrap_or(i32::MAX);
                        c.begin_marked_content_with_properties(Name(GLYPH_TAG))
                            .properties()
                            .pair(Name(b"Name"), Str(g.name.as_bytes()))
                            .pair(Name(b"GID"), gid);
                    }
                    let matrix = [coord(*scale), 0.0, 0.0, coord(*scale), coord(*x), coord(h - y)];
                    match style {
                        GlyphStyle::Filled => {
                            let key = (g.font, g.gid);
                            if let Some(form) = self.form(key) {
                                used.insert(key, form);
                                c.save_state();
                                c.set_fill_rgb(0.0, 0.0, 0.0);
                                c.transform(matrix);
                                c.x_object(Name(form_name(key).as_bytes()));
                                c.restore_state();
                            }
                        }
                        GlyphStyle::Outlined if !g.outline.els.is_empty() => {
                            let (r, gr, b) = rgb(OUTLINED_FILL);
                            c.save_state();
                            c.set_fill_rgb(r, gr, b);
                            c.set_stroke_rgb(0.0, 0.0, 0.0);
                            c.transform(matrix);
                            // The contour width is in font units, inside the scaling matrix.
                            c.set_line_width(width(OUTLINE_WIDTH / scale.max(1e-6)));
                            path(&mut c, &g.outline.els);
                            c.fill_nonzero_and_stroke();
                            c.restore_state();
                        }
                        GlyphStyle::Outlined => {}
                    }
                    if *primary {
                        c.end_marked_content();
                    }
                }
                Item::Line { from, to, ink } => {
                    let (r, g, b) = rgb(ink.rgb());
                    c.set_stroke_rgb(r, g, b);
                    c.set_line_width(width(LINE_WIDTH));
                    c.move_to(coord(from[0]), coord(h - from[1]));
                    c.line_to(coord(to[0]), coord(h - to[1]));
                    c.stroke();
                }
                Item::Rect { x, y, w, h: rh, ink } => {
                    let (r, g, b) = rgb(ink.rgb());
                    c.set_stroke_rgb(r, g, b);
                    c.set_line_width(width(LINE_WIDTH));
                    c.rect(coord(*x), coord(h - y - rh), coord(*w), coord(*rh));
                    c.stroke();
                }
                Item::Point { x, y, on_curve: true, ink } => {
                    let (r, g, b) = rgb(ink.rgb());
                    c.set_fill_rgb(r, g, b);
                    c.rect(coord(x - POINT_RADIUS), coord(h - y - POINT_RADIUS), coord(2.0 * POINT_RADIUS), coord(2.0 * POINT_RADIUS));
                    c.fill_nonzero();
                }
                Item::Point { x, y, on_curve: false, ink } => {
                    let (r, g, b) = rgb(ink.rgb());
                    c.set_fill_rgb(1.0, 1.0, 1.0);
                    c.set_stroke_rgb(r, g, b);
                    c.set_line_width(width(LINE_WIDTH));
                    circle(&mut c, *x, h - y, POINT_RADIUS);
                    c.fill_nonzero_and_stroke();
                }
                Item::Label { x, y, size, text, anchor, ink } => {
                    let dx = match anchor {
                        Anchor::Start => 0.0,
                        // Labels are set in Helvetica, so its widths place the end of the text.
                        Anchor::End => -crate::helvetica::width(text, *size),
                    };
                    let (r, g, b) = rgb(ink.rgb());
                    c.begin_text();
                    c.set_fill_rgb(r, g, b);
                    c.set_font(LABEL_FONT, coord(*size));
                    c.set_text_matrix([1.0, 0.0, 0.0, 1.0, coord(x + dx), coord(h - y)]);
                    c.show(Str(&win_ansi(text)));
                    c.end_text();
                }
                Item::Link { x, y, w, h: lh, page } => {
                    links.push((Rect::new(coord(*x), coord(h - y - lh), coord(x + w), coord(h - y)), *page));
                }
                Item::Sample { x, y, size, text } => {
                    let placed = self.shape(text)?;
                    let k = size / self.face.upem;
                    c.begin_marked_content_with_properties(SPAN_TAG).properties().actual_text(TextStr(text));
                    c.save_state();
                    c.set_fill_rgb(0.0, 0.0, 0.0);
                    for (gid, pen) in placed {
                        let Some(form) = self.form((0, gid)) else { continue };
                        used.insert((0, gid), form);
                        c.save_state();
                        c.transform([coord(k), 0.0, 0.0, coord(k), coord(x + pen * k), coord(h - y)]);
                        c.x_object(Name(form_name((0, gid)).as_bytes()));
                        c.restore_state();
                    }
                    c.restore_state();
                    c.end_marked_content();
                }
            }
        }
        Ok((c.finish().into_vec(), used, links))
    }

    /// The form XObject of a glyph, made on first use; `None` for a glyph with no contours.
    fn form(&mut self, key: GlyphKey) -> Option<Ref> {
        if self.glyphs.get(key).is_none_or(|o| o.els.is_empty()) {
            return None;
        }
        if let Some(form) = self.forms.get(&key) {
            return Some(*form);
        }
        let form = self.next.bump();
        self.forms.insert(key, form);
        Some(form)
    }

    /// The glyphs of a waterfall line with their x offsets in font units: glyphs from `cmap`,
    /// advances from the font, pair kerning between neighbours.
    fn shape(&mut self, text: &str) -> Result<Vec<(u32, f64)>, ProofError> {
        if let Some(placed) = self.shaped.get(text) {
            return Ok(placed.clone());
        }
        let mut placed = Vec::new();
        let mut pen = 0.0;
        let mut previous: Option<u32> = None;
        for ch in text.chars() {
            let gid = self.face.glyph_for(ch).unwrap_or(0);
            self.glyphs.load(self.face, gid)?;
            if let Some(left) = previous {
                pen += self.kerning.pair(left, gid);
            }
            placed.push((gid, pen));
            pen += self.glyphs.get((0, gid)).map_or(0.0, |o| o.advance);
            previous = Some(gid);
        }
        self.shaped.insert(text.to_string(), placed.clone());
        Ok(placed)
    }
}

/// A glyph of the document: the font of the set it comes from, and its glyph id.
type GlyphKey = (usize, u32);

/// The resource name of a glyph's form.
fn form_name((font, gid): GlyphKey) -> String {
    if font == 0 { format!("G{gid}") } else { format!("G{gid}F{font}") }
}

/// An outline as path construction operators, in font units.
fn path(c: &mut Content, els: &[El]) {
    let mut current = (0.0_f32, 0.0_f32);
    let mut start = current;
    for el in els {
        match *el {
            El::Move(x, y) => {
                c.move_to(x, y);
                current = (x, y);
                start = current;
            }
            El::Line(x, y) => {
                c.line_to(x, y);
                current = (x, y);
            }
            El::Quad(qx, qy, x, y) => {
                // The cubic curve that draws the same quadratic curve.
                let (x0, y0) = current;
                let third = 2.0 / 3.0;
                c.cubic_to(x0 + third * (qx - x0), y0 + third * (qy - y0), x + third * (qx - x), y + third * (qy - y), x, y);
                current = (x, y);
            }
            El::Cubic(x1, y1, x2, y2, x, y) => {
                c.cubic_to(x1, y1, x2, y2, x, y);
                current = (x, y);
            }
            El::Close => {
                c.close_path();
                current = start;
            }
        }
    }
}

/// A circle of radius `r` around (`x`, `y`), as four cubic curves.
fn circle(c: &mut Content, x: f64, y: f64, r: f64) {
    let k = r * KAPPA;
    c.move_to(coord(x + r), coord(y));
    c.cubic_to(coord(x + r), coord(y + k), coord(x + k), coord(y + r), coord(x), coord(y + r));
    c.cubic_to(coord(x - k), coord(y + r), coord(x - r), coord(y + k), coord(x - r), coord(y));
    c.cubic_to(coord(x - r), coord(y - k), coord(x - k), coord(y - r), coord(x), coord(y - r));
    c.cubic_to(coord(x + k), coord(y - r), coord(x + r), coord(y - k), coord(x + r), coord(y));
    c.close_path();
}

/// A coordinate as the writer takes it; a value that is not finite becomes 0.
fn coord(v: f64) -> f32 {
    if v.is_finite() { v.clamp(-MAX_COORDINATE, MAX_COORDINATE) as f32 } else { 0.0 }
}

/// A line width: finite and not negative (the writer refuses a negative one).
fn width(v: f64) -> f32 {
    let w = coord(v);
    if w > 0.0 { w } else { 0.0 }
}

fn rgb([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    (f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// `text` in WinAnsi encoding, the label font's; other characters become `?`.
fn win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match u8::try_from(u32::from(c)) {
            Ok(b) if (0x20..=0x7E).contains(&b) || b >= 0xA0 => b,
            _ => WIN_ANSI_HIGH.iter().find(|(u, _)| *u == c).map_or(b'?', |(_, b)| *b),
        })
        .collect()
}

#[cfg(test)]
mod tests;
