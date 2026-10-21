//! Referenced SVG files as glyph documents (M11.10): the file of an svg layer, parsed with usvg
//! and written back with the layer's image transform and the `glyph<ID>` id.
//!
//! Rows: `color.svg.threeSources`, `color.svg.imageTransform`, `color.svg.layerHoldsFile`.
//!
//! Contract (`docs/plans/M11.md` § 3 › Limits):
//! - Files over [`crate::MAX_SVG_FILE_BYTES`], nested deeper than [`crate::MAX_SVG_DEPTH`], or
//!   with external references are refused with [`crate::ColorError::BadSvg`].
//! - usvg runs without text and without system fonts; embedded bitmaps are kept.
//!
//! Placement: the file's user space (y down, its top left at the origin) is placed like a
//! bitmap's pixel grid, with its bottom left at the image's origin on the layer and one user
//! unit per font unit before the layer's image transform. The result goes into a group inside
//! the glyph group, whose own transform flips y at the baseline. Content outside the file's
//! width and height is kept, not clipped.
//!
//! The file is checked before usvg sees it: the XML is parsed once with a node limit, its
//! element depth measured without recursion, and every link checked to be local (`#id`) or
//! embedded (`data:`). usvg then reads the parsed tree with a resolver that never opens a file
//! and drops compressed embedded SVG images, whose unpacked size has no bound.

use std::sync::Arc;

use fontcraft_geom::Affine;
use usvg::roxmltree;

use crate::svg::matrix;
use crate::{ColorError, MAX_SVG_DEPTH, MAX_SVG_DOCUMENT_BYTES, MAX_SVG_FILE_BYTES};

/// Most XML nodes read from one SVG file (usvg's own limit for its tree).
const MAX_SVG_NODES: u32 = 1_000_000;

/// The document of glyph `gid` drawing the SVG file `svg`, placed by `transform` (the layer's
/// image transform, font units, y up).
pub fn import(svg: &[u8], transform: Affine, gid: u16) -> Result<String, ColorError> {
    import_placed(svg, |_, _| transform, gid)
}

/// [`import`] with the transform chosen by `place` from the width and height usvg gives the
/// file's viewport (its `width` and `height`, else its `viewBox`'s, else 100).
pub(crate) fn import_placed(svg: &[u8], place: impl FnOnce(f64, f64) -> Affine, gid: u16) -> Result<String, ColorError> {
    if svg.len() > MAX_SVG_FILE_BYTES {
        return Err(ColorError::BadSvg(format!("it is {} bytes; at most {MAX_SVG_FILE_BYTES} are read", svg.len())));
    }
    if svg.starts_with(&[0x1f, 0x8b]) {
        return Err(ColorError::BadSvg("compressed SVG files are not read; save it uncompressed".to_string()));
    }
    let text = std::str::from_utf8(svg).map_err(|_| ColorError::BadSvg("it is not UTF-8 text".to_string()))?;
    check_text_depth(text)?;
    let xml_options = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: MAX_SVG_NODES, ..roxmltree::ParsingOptions::default() };
    let xml = roxmltree::Document::parse_with_options(text, xml_options).map_err(|e| ColorError::BadSvg(e.to_string()))?;
    check_depth(&xml)?;
    check_references(&xml)?;
    crate::svg_filter::check_orders(&xml)?;

    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver { resolve_data: Box::new(embedded_data), resolve_string: Box::new(no_file) },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_xmltree(&xml, &options).map_err(|e| ColorError::BadSvg(e.to_string()))?;
    let height = f64::from(tree.size().height());
    let transform = place(f64::from(tree.size().width()), height);

    // Ids from the file get a prefix, so none of them can be the glyph group's `glyph<ID>`.
    let write = usvg::WriteOptions {
        id_prefix: Some("svg-".to_string()),
        indent: usvg::Indent::None,
        attributes_indent: usvg::Indent::None,
        ..usvg::WriteOptions::default()
    };
    let written = tree.to_string(&write);
    let (xlink, content) = root_content(&written).ok_or_else(|| ColorError::BadSvg("usvg wrote a document without a root element".to_string()))?;

    let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, height]);
    let placement = matrix(transform * flip)?;
    let mut out = String::with_capacity(content.len() + 256);
    out.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg""#);
    if xlink {
        out.push_str(r#" xmlns:xlink="http://www.w3.org/1999/xlink""#);
    }
    out.push_str(r#" version="1.1"><g id="glyph"#);
    out.push_str(&gid.to_string());
    out.push_str(r#"" transform="matrix(1 0 0 -1 0 0)"><g transform=""#);
    out.push_str(&placement);
    out.push_str(r#"">"#);
    out.push_str(content);
    out.push_str("</g></g></svg>");
    if out.len() > MAX_SVG_DOCUMENT_BYTES {
        return Err(ColorError::LimitExceeded { what: "bytes in one SVG document", limit: MAX_SVG_DOCUMENT_BYTES });
    }
    Ok(out)
}

/// Embedded images as usvg reads them, except compressed SVG images.
fn embedded_data(mime: &str, data: Arc<Vec<u8>>, options: &usvg::Options) -> Option<usvg::ImageKind> {
    if data.starts_with(&[0x1f, 0x8b]) {
        return None;
    }
    (usvg::ImageHrefResolver::default_data_resolver())(mime, data, options)
}

/// A link to a file is never followed.
fn no_file(_: &str, _: &usvg::Options) -> Option<usvg::ImageKind> {
    None
}

/// Whether usvg's root element declares the xlink namespace, and what it holds.
fn root_content(written: &str) -> Option<(bool, &str)> {
    let start = written.find("<svg")?;
    let rest = written.get(start..)?;
    let open_end = rest.find('>')?;
    let head = rest.get(..open_end)?;
    let xlink = head.contains("xmlns:xlink");
    if head.ends_with('/') {
        return Some((xlink, ""));
    }
    let content = rest.get(open_end + 1..)?.trim_end().strip_suffix("</svg>")?;
    Some((xlink, content))
}

/// Refuses text whose elements nest deeper than [`MAX_SVG_DEPTH`], and entity declarations that
/// hold markup, before the XML parser sees it: the parser recurses on nesting and would run out
/// of stack on a deep file before [`check_depth`] could refuse it. The scan counts start and end
/// tags without recursion, skipping comments, CDATA, processing instructions and quoted
/// attribute values.
fn check_text_depth(text: &str) -> Result<(), ColorError> {
    let bytes = text.as_bytes();
    // The index just past the first `end` at or after `from`, or the end of the text.
    let past = |from: usize, end: &[u8]| -> usize {
        bytes.get(from..).and_then(|rest| rest.windows(end.len()).position(|w| w == end)).map_or(bytes.len(), |at| from + at + end.len())
    };
    // The index just past the `>` that closes a declaration or tag opened before `from`, with
    // quoted text skipped, and whether the quoted text held a `<`.
    let close = |from: usize, stop_at_bracket: bool| -> (usize, bool) {
        let mut quote = None;
        let mut markup = false;
        for (i, &b) in bytes.iter().enumerate().skip(from) {
            match (quote, b) {
                (Some(q), _) if b == q => quote = None,
                (Some(_), b'<') => markup = true,
                (Some(_), _) => {}
                (None, b'"' | b'\'') => quote = Some(b),
                (None, b'>') => return (i + 1, markup),
                (None, b'[') if stop_at_bracket => return (i + 1, markup),
                (None, _) => {}
            }
        }
        (bytes.len(), markup)
    };
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(at) = bytes.get(i..).and_then(|rest| rest.iter().position(|&b| b == b'<')) {
        let start = i + at;
        let rest = bytes.get(start..).unwrap_or_default();
        i = if rest.starts_with(b"<!--") {
            past(start + 4, b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            past(start + 9, b"]]>")
        } else if rest.starts_with(b"<?") {
            past(start + 2, b"?>")
        } else if rest.starts_with(b"<!DOCTYPE") {
            close(start + 9, true).0
        } else if rest.starts_with(b"<!") {
            let (end, markup) = close(start + 2, false);
            if markup && rest.starts_with(b"<!ENTITY") {
                return Err(ColorError::BadSvg("it declares an entity that holds markup".to_string()));
            }
            end
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            close(start + 2, false).0
        } else {
            let (end, _) = close(start + 1, false);
            if end < 2 || bytes.get(end - 2) != Some(&b'/') {
                depth += 1;
                if depth > MAX_SVG_DEPTH {
                    return Err(ColorError::BadSvg(format!("its elements are nested more than {MAX_SVG_DEPTH} deep")));
                }
            }
            end
        };
    }
    Ok(())
}

/// Refuses elements nested deeper than [`MAX_SVG_DEPTH`]. Nodes come in document order, so a
/// node's parent always has its depth already.
fn check_depth(xml: &roxmltree::Document) -> Result<(), ColorError> {
    let mut depths: Vec<u16> = vec![0; xml.descendants().count()];
    for node in xml.descendants() {
        let Some(parent) = node.parent() else { continue };
        let depth = depths.get(parent.id().get_usize()).copied().unwrap_or(0).saturating_add(1);
        if usize::from(depth) > MAX_SVG_DEPTH {
            return Err(ColorError::BadSvg(format!("its elements are nested more than {MAX_SVG_DEPTH} deep")));
        }
        if let Some(slot) = depths.get_mut(node.id().get_usize()) {
            *slot = depth;
        }
    }
    Ok(())
}

/// Refuses links to anything outside the file: `href` values other than `#id` and `data:`
/// URLs, `url(…)` values other than `url(#id)`, style sheet imports and linked style sheets.
fn check_references(xml: &roxmltree::Document) -> Result<(), ColorError> {
    let external = |what: &str| {
        let shown: String = what.chars().take(80).collect();
        Err(ColorError::BadSvg(format!("it refers to `{shown}` outside the file; only references within the file and embedded data are read")))
    };
    for node in xml.descendants() {
        if let Some(pi) = node.pi()
            && pi.target == "xml-stylesheet"
        {
            return external(pi.value.unwrap_or("a style sheet"));
        }
        if node.is_text() && node.parent().is_some_and(|p| p.tag_name().name() == "style") {
            let css = node.text().unwrap_or("");
            if css.contains("@import") {
                return external("@import");
            }
            if let Some(url) = external_url(css) {
                return external(url);
            }
        }
        for attr in node.attributes() {
            let value = attr.value();
            if attr.name() == "href" {
                let v = value.trim();
                let local = v.starts_with('#') || v.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"));
                if !local {
                    return external(v);
                }
            }
            if let Some(url) = external_url(value) {
                return external(url);
            }
        }
    }
    Ok(())
}

/// The first `url(…)` in `text` that does not point to an element of the file.
fn external_url(text: &str) -> Option<&str> {
    let mut rest = text;
    while let Some(at) = rest.find("url(") {
        let after = rest.get(at + 4..)?;
        let target = after.trim_start().trim_start_matches(['"', '\'']);
        if !target.starts_with('#') {
            let end = target.find(')').unwrap_or(target.len());
            return target.get(..end);
        }
        rest = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::Affine;

    use super::import;
    use crate::strike::pick_size;
    use crate::{ColorError, MAX_SVG_FILE_BYTES};

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect id="glyph1" x="10" y="10" width="20" height="20" fill="#ff0000"/></svg>"##;

    fn bad(svg: &[u8]) -> bool {
        matches!(import(svg, Affine::IDENTITY, 1), Err(ColorError::BadSvg(_)) | Err(ColorError::LimitExceeded { .. }))
    }

    #[test]
    fn svg_layer_moved_and_scaled_is_written_with_its_placement() {
        // Moved by (200, -100) and scaled by 2: the file's bottom left (0, 50) lands on (200, -100).
        let placement = Affine::translate((200.0, -100.0)) * Affine::scale(2.0);
        let doc = import(SQUARE.as_bytes(), placement, 5).unwrap();
        assert!(doc.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#), "{doc}");
        assert!(doc.contains(r#"<g id="glyph5" transform="matrix(1 0 0 -1 0 0)"><g transform="matrix(2 0 0 -2 200 0)">"#), "{doc}");
        // The file's own id is prefixed, so the document has one element with the glyph's id.
        assert!(doc.contains(r#"id="svg-glyph1""#) && doc.matches("id=\"glyph").count() == 1, "{doc}");
        assert!(doc.ends_with("</g></g></svg>"), "{doc}");
        let tree = usvg::Tree::from_str(&doc, &usvg::Options::default()).unwrap();
        // The square spans x 10 to 30 and y 10 to 30 in the file; placed, it is x 220 to 260 and
        // y (font units, up) -100 + 2 × (50 − 30) = -60 to -20, so SVG y 20 to 60.
        let b = tree.root().abs_bounding_box();
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(near(b.left(), 220.0) && near(b.right(), 260.0) && near(b.top(), 20.0) && near(b.bottom(), 60.0), "{b:?}");
    }

    #[test]
    fn svg_with_an_embedded_bitmap_keeps_it() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="10" height="10" xlink:href="data:image/png;base64,{png}"/></svg>"#
        );
        let doc = import(svg.as_bytes(), Affine::IDENTITY, 2).unwrap();
        assert!(doc.contains("<image") && doc.contains("data:image/png;base64,"), "{doc}");
        assert!(usvg::Tree::from_str(&doc, &usvg::Options::default()).is_ok(), "{doc}");
    }

    #[test]
    fn hostile_svg_files_are_refused() {
        // Nested 10,000 deep.
        let deep = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{}{}</svg>"#, "<g>".repeat(10_000), "</g>".repeat(10_000));
        assert!(bad(deep.as_bytes()));
        // Nesting built from entities is refused before it is expanded.
        let entities = r#"<!DOCTYPE svg [<!ENTITY a "<g><g></g></g>">]><svg xmlns="http://www.w3.org/2000/svg">&a;</svg>"#;
        assert!(bad(entities.as_bytes()));
        // Comments, CDATA and quoted `>` do not count as tags.
        let quoted =
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><!-- <g> --><g id="a>b"><rect width="5" height="5"/></g></svg>"#;
        assert!(import(quoted.as_bytes(), Affine::IDENTITY, 1).is_ok());
        // A file over the limit is refused before it is parsed.
        let mut big = SQUARE.as_bytes().to_vec();
        big.resize(MAX_SVG_FILE_BYTES + 1, b' ');
        assert!(bad(&big));
        // External references: an image file, a use of another file, a paint from another file,
        // a style sheet import, a linked style sheet.
        let wrap = |body: &str| {
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">{body}</svg>"#)
        };
        assert!(bad(wrap(r#"<image width="10" height="10" xlink:href="/etc/passwd.png"/>"#).as_bytes()));
        assert!(bad(wrap(r#"<image width="10" height="10" href="https://example.com/a.png"/>"#).as_bytes()));
        assert!(bad(wrap(r#"<use href="other.svg#a"/>"#).as_bytes()));
        assert!(bad(wrap(r#"<rect width="5" height="5" fill="url(other.svg#g)"/>"#).as_bytes()));
        assert!(bad(wrap(r#"<style>@import "a.css";</style>"#).as_bytes()));
        let linked = format!(r#"<?xml-stylesheet href="a.css"?>{}"#, wrap(""));
        assert!(bad(linked.as_bytes()));
        // Local references are fine.
        let local = wrap(
            r##"<defs><linearGradient id="g"><stop offset="0"/></linearGradient></defs><rect id="r" width="5" height="5" fill="url('#g')"/><use href="#r"/>"##,
        );
        assert!(import(local.as_bytes(), Affine::IDENTITY, 1).is_ok());
        // Compressed, not UTF-8, not XML.
        assert!(bad(&[0x1f, 0x8b, 8, 0]));
        assert!(bad(&[0xff, 0xfe, 0xfd]));
        assert!(bad(b"<svg"));
        // A placement that is not a number.
        assert!(bad_placement());
    }

    #[test]
    fn sbix_to_svg_takes_the_exact_then_the_next_larger_then_the_largest_smaller_size() {
        let sizes = [32, 160, 64, 512];
        assert_eq!(pick_size(&sizes, 64), Some(64));
        assert_eq!(pick_size(&sizes, 128), Some(160));
        assert_eq!(pick_size(&sizes, 600), Some(512));
        assert_eq!(pick_size(&sizes, 1), Some(32));
        assert_eq!(pick_size(&[], 128), None);
    }

    fn bad_placement() -> bool {
        matches!(import(SQUARE.as_bytes(), Affine::scale(f64::NAN), 1), Err(ColorError::BadSvg(_)))
    }
}
