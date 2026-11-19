//! The SVG writer: one standalone SVG 1.1 document per page.
//!
//! Glyphs are vector paths in font units, defined once per page and placed with a transform, so
//! the page looks the same in every viewer. Each shown glyph sits in a group carrying
//! `data-glyph` (its name) and `data-gid`. Waterfall text is set in the font itself, embedded as
//! a `data:` URL in an `@font-face` rule, so the viewer's own rasteriser is what is seen.

use std::collections::BTreeSet;

use crate::font::El;
use crate::page::{Anchor, Document, GlyphStyle, Item, LINE_WIDTH, OUTLINED_FILL, POINT_RADIUS, Page};
use crate::text::{base64, escape, num, push};

/// The font family name the embedded font gets inside the page.
const FACE_NAME: &str = "Proof Face";
const LABEL_FONTS: &str = "Helvetica, Arial, sans-serif";

/// Writes every page of `doc`.
pub(crate) fn write(doc: &Document) -> Vec<Vec<u8>> {
    let font_face = doc.embedded.as_ref().map(|e| {
        format!(
            "@font-face{{font-family:\"{FACE_NAME}\";src:url(data:{};base64,{}) format(\"{}\");}}",
            e.container.mime(),
            base64(&e.bytes),
            e.container.css_format()
        )
    });
    let variation = doc
        .embedded
        .as_ref()
        .filter(|e| !e.variation.is_empty())
        .map(|e| {
            let settings: Vec<String> = e.variation.iter().map(|(tag, v)| format!("\"{}\" {}", escape(tag), num(*v))).collect();
            format!("font-variation-settings:{};", settings.join(","))
        })
        .unwrap_or_default();
    let total = doc.pages.len();
    doc.pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let has_sample = page.items.iter().any(|it| matches!(it, Item::Sample { .. }));
            let face = if has_sample { font_face.as_deref() } else { None };
            page_svg(doc, page, i + 1, total, face, &variation).into_bytes()
        })
        .collect()
}

fn page_svg(doc: &Document, page: &Page, number: usize, total: usize, font_face: Option<&str>, variation: &str) -> String {
    let (w, h) = (num(page.width), num(page.height));
    let mut out = String::new();
    push(&mut out, format_args!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
    push(
        &mut out,
        format_args!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\" width=\"{w}pt\" height=\"{h}pt\" viewBox=\"0 0 {w} {h}\" data-page=\"{number}\" data-pages=\"{total}\">\n"
        ),
    );
    push(&mut out, format_args!("<title>{} \u{2014} page {number} of {total}</title>\n", escape(&doc.title)));
    push(&mut out, format_args!("<style>.l{{font-family:{LABEL_FONTS};}}"));
    if let Some(face) = font_face {
        push(&mut out, format_args!("{face}.s{{font-family:\"{FACE_NAME}\";{variation}}}"));
    }
    out.push_str("</style>\n");
    // The outlines this page uses, each once.
    let used: BTreeSet<usize> = page.items.iter().filter_map(|it| if let Item::Glyph { glyph, .. } = it { Some(*glyph) } else { None }).collect();
    if !used.is_empty() {
        out.push_str("<defs>\n");
        for index in used {
            if let Some(g) = doc.glyphs.get(index) {
                push(&mut out, format_args!("<path id=\"g{index}\" d=\"{}\"/>\n", path_data(&g.outline.els)));
            }
        }
        out.push_str("</defs>\n");
    }
    push(&mut out, format_args!("<rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>\n"));
    for item in &page.items {
        item_svg(doc, item, &mut out);
    }
    out.push_str("</svg>\n");
    out
}

fn item_svg(doc: &Document, item: &Item, out: &mut String) {
    match item {
        Item::Glyph { glyph, x, y, scale, style, primary } => {
            let Some(g) = doc.glyphs.get(*glyph) else { return };
            let paint = match style {
                GlyphStyle::Filled => "fill=\"#000000\"".to_string(),
                GlyphStyle::Outlined => {
                    let [r, gr, b] = OUTLINED_FILL;
                    // The stroke is in font units, inside the scaling transform.
                    format!("fill=\"#{r:02x}{gr:02x}{b:02x}\" stroke=\"#000000\" stroke-width=\"{}\"", num(0.6 / scale.max(1e-6)))
                }
            };
            let use_el = format!(
                "<use xlink:href=\"#g{glyph}\" href=\"#g{glyph}\" transform=\"matrix({s} 0 0 {ns} {x} {y})\" {paint}/>",
                s = num(*scale),
                ns = num(-scale),
                x = num(*x),
                y = num(*y),
            );
            if *primary {
                push(out, format_args!("<g class=\"glyph\" data-glyph=\"{}\" data-gid=\"{}\">{use_el}</g>\n", escape(&g.name), g.gid));
            } else {
                push(out, format_args!("{use_el}\n"));
            }
        }
        Item::Line { from, to, ink } => push(
            out,
            format_args!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                num(from[0]),
                num(from[1]),
                num(to[0]),
                num(to[1]),
                ink.hex(),
                num(LINE_WIDTH)
            ),
        ),
        Item::Rect { x, y, w, h, ink } => push(
            out,
            format_args!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                num(*x),
                num(*y),
                num(*w),
                num(*h),
                ink.hex(),
                num(LINE_WIDTH)
            ),
        ),
        Item::Point { x, y, on_curve: true, ink } => push(
            out,
            format_args!(
                "<rect x=\"{}\" y=\"{}\" width=\"{d}\" height=\"{d}\" fill=\"{}\"/>\n",
                num(x - POINT_RADIUS),
                num(y - POINT_RADIUS),
                ink.hex(),
                d = num(2.0 * POINT_RADIUS)
            ),
        ),
        Item::Point { x, y, on_curve: false, ink } => push(
            out,
            format_args!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#ffffff\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
                num(*x),
                num(*y),
                num(POINT_RADIUS),
                ink.hex(),
                num(LINE_WIDTH)
            ),
        ),
        Item::Label { x, y, size, text, anchor, ink } => {
            let anchor = match anchor {
                Anchor::Start => "start",
                Anchor::End => "end",
            };
            push(
                out,
                format_args!(
                    "<text class=\"l\" x=\"{}\" y=\"{}\" font-size=\"{}\" text-anchor=\"{anchor}\" fill=\"{}\">{}</text>\n",
                    num(*x),
                    num(*y),
                    num(*size),
                    ink.hex(),
                    escape(text)
                ),
            )
        }
        // Pages are separate files, so a link names the page it opens rather than linking to it.
        Item::Link { x, y, w, h, page } => push(
            out,
            format_args!(
                "<rect class=\"link\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"none\" data-link-page=\"{}\"/>\n",
                num(*x),
                num(*y),
                num(*w),
                num(*h),
                page + 1
            ),
        ),
        Item::Sample { x, y, size, text } => push(
            out,
            format_args!(
                "<text class=\"s\" xml:space=\"preserve\" x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"#000000\">{}</text>\n",
                num(*x),
                num(*y),
                num(*size),
                escape(text)
            ),
        ),
    }
}

/// SVG path data of an outline, in font units.
pub(crate) fn path_data(els: &[El]) -> String {
    let n = |v: f32| num(f64::from(v));
    let mut d = String::new();
    for el in els {
        match *el {
            El::Move(x, y) => push(&mut d, format_args!("M{} {}", n(x), n(y))),
            El::Line(x, y) => push(&mut d, format_args!("L{} {}", n(x), n(y))),
            El::Quad(cx, cy, x, y) => push(&mut d, format_args!("Q{} {} {} {}", n(cx), n(cy), n(x), n(y))),
            El::Cubic(a, b, c, e, x, y) => push(&mut d, format_args!("C{} {} {} {} {} {}", n(a), n(b), n(c), n(e), n(x), n(y))),
            El::Close => d.push('Z'),
        }
    }
    d
}
