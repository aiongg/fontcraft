//! SVG documents and the `SVG ` table (M11.9; bitmap documents M11.10).
//!
//! Rows: `SVG `, `color.export.fullColorToSvg`, `color.export.colrToSvg`,
//! `color.export.colorPaletteForSvg`, `color.export.colorLayersToSvg`, `color.layered.toSvg`,
//! `color.export.sbixToSvg`.
//!
//! Contract (`docs/plans/M11.md` § 3 › SVG documents):
//! - One uncompressed document per glyph. Its group has the id `glyph<ID>` and a transform that
//!   flips y at the baseline (nanoemoji's `untouchedsvg` layout).
//! - Palette colours are baked in from the palette `Color Palette for SVG` names; the text colour
//!   is written as `currentColor`.
//! - Centred strokes are written as SVG strokes; strokes on one side are expanded by the caller
//!   and arrive as outlines. [`is_native_stroke`] is the rule both sides follow.
//! - A reference to another colour glyph ([`Paint::ColrGlyph`]) is a copy of that glyph's
//!   drawing in a group with the reference's transform ([`glyph_document`]); documents are
//!   separate per glyph, so one cannot `<use>` another's elements.
//! - A document is at most [`crate::MAX_SVG_DOCUMENT_BYTES`]; the table at most
//!   [`crate::MAX_SVG_TABLE_BYTES`]. Writing one document visits at most
//!   [`crate::MAX_SVG_PAINT_VISITS`] paint nodes; references met past that are drawn as their
//!   component's outline, as the COLR fallback draws them.
//!
//! The document draws in font units: the glyph group's transform turns the font's y-up space
//! into SVG's y-down space, so outlines, gradient points and stroke widths are written as the
//! paint graph has them. Gradients and clip paths use user-space units, which inside the group
//! are font units.

use std::collections::HashMap;
use std::fmt::Write as _;

use fontcraft_geom::{Affine, BezPath, PathEl, Point};
use fontcraft_model::{Extend, MAX_COORDINATE};

use crate::paint::{ColorLine, ColorRef, Paint, ShapeRef, StrokeSpec, nesting, outline_fill};
use crate::{ColorError, MAX_PAINT_DEPTH, MAX_STOPS, MAX_SVG_DOCUMENT_BYTES, MAX_SVG_PAINT_VISITS, MAX_SVG_TABLE_BYTES, Rgba};

/// Where a bitmap is drawn in a glyph's document, in font units (y up, as the layer has it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BitmapPlacement {
    /// Maps the image's pixel grid (origin at its bottom left, one unit per pixel) into the
    /// glyph.
    pub transform: Affine,
    /// The image's size in pixels.
    pub width: u32,
    pub height: u32,
}

/// Whether a stroke is written as an SVG stroke. SVG strokes have one round pen, one cap for
/// both ends and centre on the path; anything else (a stroke on one side, an oval pen, a round
/// inset or axis-aligned cap, a fully round join) is expanded to an outline by the caller.
pub fn is_native_stroke(spec: &StrokeSpec) -> bool {
    spec.position == 0.0
        && spec.width.is_finite()
        && spec.width > 0.0
        && spec.height == spec.width
        && spec.cap_start == spec.cap_end
        && spec.cap_start <= 2
        && spec.join <= 2
}

/// Whether `paint` holds a conic gradient, which SVG cannot draw: [`document`] fills it with
/// the colour of its first stop.
pub fn has_sweep(paint: &Paint) -> bool {
    fn walk(paint: &Paint, depth: usize) -> bool {
        if depth > MAX_PAINT_DEPTH {
            return false;
        }
        match paint {
            Paint::Sweep { .. } => true,
            Paint::Layers(list) => list.iter().any(|p| walk(p, depth + 1)),
            Paint::Glyph { paint, .. } => walk(paint, depth + 1),
            Paint::ColrGlyph { .. } | Paint::Solid(_) | Paint::Linear { .. } | Paint::Radial { .. } => false,
        }
    }
    walk(paint, 0)
}

/// The document of glyph `gid` drawing `paint`. `outlines` draws a [`ShapeRef`] in font units;
/// `palette` is the palette baked in. A [`Paint::ColrGlyph`] is drawn as its component's outline
/// in [`outline_fill`]; [`glyph_document`] follows it instead.
pub fn document(paint: &Paint, outlines: &dyn Fn(&ShapeRef) -> BezPath, gid: u16, palette: &[Rgba]) -> Result<String, ColorError> {
    let outlines = |_: &str, shape: &ShapeRef| outlines(shape);
    glyph_document("", paint, &outlines, &|_: &str| None, gid, palette).map(|d| d.text)
}

/// A glyph's SVG document and what the writer left out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphDocument {
    pub text: String,
    /// References drawn as their component's outline because the document had visited
    /// [`MAX_SVG_PAINT_VISITS`] paint nodes when they were met.
    pub outlined_over_budget: usize,
}

/// The document of glyph `name` (glyph id `gid`) drawing `paint`, with each
/// [`Paint::ColrGlyph`] drawn as the paint graph `paints` gives for its glyph, inside a group
/// that moves it by the reference's transform (`color.export.colrGlyphComponent`). References
/// inside that graph are followed the same way. `outlines` draws a [`ShapeRef`] of the named
/// glyph in font units; `palette` is the palette baked in.
///
/// A reference is drawn as its component's outline in [`outline_fill`], as the COLR fallback
/// draws it, when `paints` has no graph for its glyph, when the glyph is already being drawn
/// (a cycle), when references nest more than [`MAX_PAINT_DEPTH`] deep, when the inlined paint
/// would nest past [`MAX_PAINT_DEPTH`], when its transform cannot be written, or when the
/// document has visited [`MAX_SVG_PAINT_VISITS`] paint nodes (counted in
/// [`GlyphDocument::outlined_over_budget`]). Each inlined copy also counts towards
/// [`MAX_SVG_DOCUMENT_BYTES`].
pub fn glyph_document<'a>(
    name: &str,
    paint: &Paint,
    outlines: &'a dyn Fn(&str, &ShapeRef) -> BezPath,
    paints: &'a dyn Fn(&str) -> Option<&'a Paint>,
    gid: u16,
    palette: &'a [Rgba],
) -> Result<GlyphDocument, ColorError> {
    let mut w = Writer::new(name, outlines, paints, gid, palette);
    w.paint(paint, 0)?;
    let mut out = String::with_capacity(w.defs.len() + w.body.len() + 160);
    out.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" version="1.1">"#);
    if !w.defs.is_empty() {
        out.push_str("<defs>");
        out.push_str(&w.defs);
        out.push_str("</defs>");
    }
    let _ = write!(out, r#"<g id="glyph{gid}" transform="matrix(1 0 0 -1 0 0)">"#);
    out.push_str(&w.body);
    out.push_str("</g></svg>");
    if out.len() > MAX_SVG_DOCUMENT_BYTES {
        return Err(ColorError::LimitExceeded { what: "bytes in one SVG document", limit: MAX_SVG_DOCUMENT_BYTES });
    }
    Ok(GlyphDocument { text: out, outlined_over_budget: w.over_budget })
}

/// The document of glyph `gid` showing a PNG image, embedded as a data URL in an `<image>`
/// element. JPEG data is embedded as it is too, with its own media type; anything else is
/// refused.
///
/// The `<image>` element draws the pixel grid y down from its top left, so its transform first
/// flips the grid at the image's height and then applies `placement`; the glyph group's own flip
/// takes the result into SVG's space.
pub fn bitmap_document(png: &[u8], placement: &BitmapPlacement, gid: u16) -> Result<String, ColorError> {
    let media = bitmap_media_type(png).ok_or_else(|| ColorError::BadSvg("the bitmap is neither PNG nor JPEG data".to_string()))?;
    if placement.width == 0 || placement.height == 0 {
        return Err(ColorError::BadSvg(format!("the bitmap is {} by {} pixels", placement.width, placement.height)));
    }
    // Base64 takes four bytes for every three; the rest of the document is short.
    let encoded = png.len().div_ceil(3).saturating_mul(4);
    if encoded.saturating_add(512) > MAX_SVG_DOCUMENT_BYTES {
        return Err(ColorError::LimitExceeded { what: "bytes in one SVG document", limit: MAX_SVG_DOCUMENT_BYTES });
    }
    let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, f64::from(placement.height)]);
    let transform = matrix(placement.transform * flip)?;
    let mut out = String::with_capacity(encoded + 512);
    out.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" version="1.1">"#);
    let _ = write!(out, r#"<g id="glyph{gid}" transform="matrix(1 0 0 -1 0 0)">"#);
    let _ =
        write!(out, r#"<image width="{}" height="{}" transform="{transform}" xlink:href="data:{media};base64,"#, placement.width, placement.height);
    base64(png, &mut out);
    out.push_str(r#""/></g></svg>"#);
    Ok(out)
}

/// The width and height in pixels of PNG or JPEG data, read from its header; `None` for any
/// other data or a header that cannot be read.
pub fn bitmap_size(data: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| Some(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]));
    let be32 = |at: usize| Some(u32::from_be_bytes([*data.get(at)?, *data.get(at + 1)?, *data.get(at + 2)?, *data.get(at + 3)?]));
    match bitmap_media_type(data)? {
        "image/png" => {
            // The signature, then the IHDR chunk's length and type, then width and height.
            if data.get(12..16)? != b"IHDR" {
                return None;
            }
            Some((be32(16)?, be32(20)?))
        }
        _ => {
            // JPEG: walk the marker segments to the first start-of-frame. Each step moves past
            // at least one byte, so the walk ends.
            let mut at = 2;
            loop {
                if *data.get(at)? != 0xFF {
                    return None;
                }
                let marker = *data.get(at + 1)?;
                match marker {
                    0xFF => at += 1,
                    0x01 | 0xD0..=0xD7 => at += 2,
                    0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                        let (height, width) = (be16(at + 5)?, be16(at + 7)?);
                        return Some((u32::from(width), u32::from(height)));
                    }
                    0xD9 | 0xDA => return None,
                    _ => at = at.checked_add(2 + usize::from(be16(at + 2)?))?,
                }
            }
        }
    }
}

/// The media type of PNG or JPEG data, from its signature.
fn bitmap_media_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// Standard base64 with padding, appended to `out`.
fn base64(data: &[u8], out: &mut String) {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let symbol = |v: u32| char::from(ALPHABET.get((v & 63) as usize).copied().unwrap_or(b'A'));
    for chunk in data.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let v = (b(0) << 16) | (b(1) << 8) | b(2);
        out.push(symbol(v >> 18));
        out.push(symbol(v >> 12));
        out.push(if chunk.len() > 1 { symbol(v >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { symbol(v) } else { '=' });
    }
}

/// An SVG `matrix(…)` transform; refused when a coefficient is not finite or beyond
/// ±[`MAX_COORDINATE`]. Six decimals keep small image scales exact enough.
pub(crate) fn matrix(affine: Affine) -> Result<String, ColorError> {
    let mut out = String::from("matrix(");
    for (i, v) in affine.as_coeffs().into_iter().enumerate() {
        if !(v.is_finite() && v.abs() <= MAX_COORDINATE) {
            return Err(ColorError::BadSvg(format!("the placement has the number {v}, which is not within ±{MAX_COORDINATE}")));
        }
        if i > 0 {
            out.push(' ');
        }
        let rounded = (v * 1e6).round() / 1e6;
        let rounded = if rounded == 0.0 { 0.0 } else { rounded };
        let mut s = format!("{rounded:.6}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        out.push_str(&s);
    }
    out.push(')');
    Ok(out)
}

/// The `SVG ` table from documents given as (first glyph id, last glyph id, document bytes);
/// records are sorted by glyph id and must not overlap. Identical documents are stored once and
/// shared by their records, which the format allows.
pub fn table(docs: &[(u16, u16, Vec<u8>)]) -> Result<Vec<u8>, ColorError> {
    const HEADER: usize = 10;
    const RECORD: usize = 12;
    let count = u16::try_from(docs.len()).map_err(|_| ColorError::LimitExceeded { what: "SVG documents", limit: usize::from(u16::MAX) })?;
    let mut sorted: Vec<&(u16, u16, Vec<u8>)> = docs.iter().collect();
    sorted.sort_by_key(|d| (d.0, d.1));
    let mut previous_end: Option<u16> = None;
    for d in &sorted {
        let (first, last, bytes) = (d.0, d.1, &d.2);
        if first > last {
            return Err(ColorError::BadSvg(format!("the document for glyphs {first} to {last} has its range reversed")));
        }
        if previous_end.is_some_and(|end| first <= end) {
            return Err(ColorError::BadSvg(format!("two documents are given for glyph {first}")));
        }
        if bytes.is_empty() || bytes.len() > MAX_SVG_DOCUMENT_BYTES {
            return Err(ColorError::BadSvg(format!("the document for glyph {first} is empty or larger than {MAX_SVG_DOCUMENT_BYTES} bytes")));
        }
        previous_end = Some(last);
    }

    let too_big = || ColorError::LimitExceeded { what: "bytes in the SVG table", limit: MAX_SVG_TABLE_BYTES };
    // Document offsets count from the start of the document list, which follows the header.
    let records_end = 2 + RECORD * sorted.len();
    let mut data: Vec<u8> = Vec::new();
    let mut stored: HashMap<&[u8], u32> = HashMap::new();
    let mut records: Vec<u8> = Vec::with_capacity(RECORD * sorted.len());
    for d in &sorted {
        let (first, last, bytes) = (d.0, d.1, &d.2);
        let offset = match stored.get(bytes.as_slice()) {
            Some(&offset) => offset,
            None => {
                let at = records_end.checked_add(data.len()).ok_or_else(too_big)?;
                if HEADER + at + bytes.len() > MAX_SVG_TABLE_BYTES {
                    return Err(too_big());
                }
                let offset = u32::try_from(at).map_err(|_| too_big())?;
                data.extend_from_slice(bytes);
                stored.insert(bytes.as_slice(), offset);
                offset
            }
        };
        let length = u32::try_from(bytes.len()).map_err(|_| too_big())?;
        records.extend_from_slice(&first.to_be_bytes());
        records.extend_from_slice(&last.to_be_bytes());
        records.extend_from_slice(&offset.to_be_bytes());
        records.extend_from_slice(&length.to_be_bytes());
    }

    let mut out = Vec::with_capacity(HEADER + records_end + data.len());
    out.extend_from_slice(&0u16.to_be_bytes()); // version
    out.extend_from_slice(&(HEADER as u32).to_be_bytes()); // offset of the document list
    out.extend_from_slice(&0u32.to_be_bytes()); // reserved
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&records);
    out.extend_from_slice(&data);
    Ok(out)
}

/// Builds one document: `defs` collects gradients, clip paths and masks, `body` the drawing.
struct Writer<'a> {
    gid: u16,
    palette: &'a [Rgba],
    outlines: &'a dyn Fn(&str, &ShapeRef) -> BezPath,
    paints: &'a dyn Fn(&str) -> Option<&'a Paint>,
    /// The glyphs being drawn: the document's glyph, then each glyph a reference inlined.
    glyphs: Vec<String>,
    /// [`nesting`] of each referenced glyph's paint, so that many references to one large graph
    /// walk it once.
    nesting: HashMap<String, usize>,
    defs: String,
    body: String,
    next_id: usize,
    /// Paint nodes visited, bounded by [`MAX_SVG_PAINT_VISITS`] through the references.
    visits: usize,
    /// References drawn as their outline because `visits` had reached its limit.
    over_budget: usize,
    /// How often [`nesting`] walked a graph.
    #[cfg(test)]
    nesting_walks: usize,
}

/// How a shape is painted: as a fill, or as an SVG stroke.
#[derive(Clone, Copy)]
enum Mode<'s> {
    Fill,
    Stroke(&'s StrokeSpec),
}

impl<'a> Writer<'a> {
    fn new(
        name: &str,
        outlines: &'a dyn Fn(&str, &ShapeRef) -> BezPath,
        paints: &'a dyn Fn(&str) -> Option<&'a Paint>,
        gid: u16,
        palette: &'a [Rgba],
    ) -> Self {
        Writer {
            gid,
            palette,
            outlines,
            paints,
            glyphs: vec![name.to_string()],
            nesting: HashMap::new(),
            defs: String::new(),
            body: String::new(),
            next_id: 0,
            visits: 0,
            over_budget: 0,
            #[cfg(test)]
            nesting_walks: 0,
        }
    }

    /// [`nesting`] of `inner`, the paint of `glyph`.
    fn nesting(&mut self, glyph: &str, inner: &Paint) -> usize {
        if let Some(&n) = self.nesting.get(glyph) {
            return n;
        }
        #[cfg(test)]
        {
            self.nesting_walks += 1;
        }
        let n = nesting(inner);
        self.nesting.insert(glyph.to_string(), n);
        n
    }

    fn check_size(&self) -> Result<(), ColorError> {
        if self.defs.len() + self.body.len() > MAX_SVG_DOCUMENT_BYTES {
            return Err(ColorError::LimitExceeded { what: "bytes in one SVG document", limit: MAX_SVG_DOCUMENT_BYTES });
        }
        Ok(())
    }

    fn bad(&self, message: impl Into<String>) -> ColorError {
        ColorError::BadPaint { glyph: format!("glyph id {}", self.gid), message: message.into() }
    }

    fn new_id(&mut self, kind: &str) -> String {
        let id = format!("glyph{}-{kind}{}", self.gid, self.next_id);
        self.next_id += 1;
        id
    }

    fn paint(&mut self, paint: &Paint, depth: usize) -> Result<(), ColorError> {
        if depth > MAX_PAINT_DEPTH {
            return Err(ColorError::LimitExceeded { what: "nested paints", limit: MAX_PAINT_DEPTH });
        }
        self.visits = self.visits.saturating_add(1);
        match paint {
            Paint::Layers(list) => {
                for p in list {
                    self.paint(p, depth + 1)?;
                }
            }
            Paint::Glyph { shape, paint: inner } => self.glyph(shape, inner, depth)?,
            Paint::ColrGlyph { glyph, transform, shape } => self.reference(glyph, *transform, shape, depth)?,
            // A paint outside any glyph fills everything; the renderer clips it to the glyph's
            // drawing area.
            simple => {
                let neg = fmt(-MAX_COORDINATE);
                let span = fmt(2.0 * MAX_COORDINATE);
                let fill = self.paint_attrs(simple, Mode::Fill)?;
                let _ = write!(self.body, r#"<rect x="{neg}" y="{neg}" width="{span}" height="{span}"{fill}/>"#);
            }
        }
        self.check_size()
    }

    /// A [`Paint::ColrGlyph`] of `glyph` moved by `transform`, `shape` being its component.
    fn reference(&mut self, glyph: &str, transform: Affine, shape: &ShapeRef, depth: usize) -> Result<(), ColorError> {
        let paints = self.paints;
        let mut inner = paints(glyph).filter(|_| self.glyphs.len() <= MAX_PAINT_DEPTH && !self.glyphs.iter().any(|g| g == glyph));
        if let Some(p) = inner
            && (depth + 1).saturating_add(self.nesting(glyph, p)) > MAX_PAINT_DEPTH
        {
            inner = None;
        }
        if inner.is_some() && self.visits >= MAX_SVG_PAINT_VISITS {
            self.over_budget += 1;
            inner = None;
        }
        let (Some(inner), Ok(m)) = (inner, matrix(transform)) else {
            return self.glyph(shape, &outline_fill(), depth);
        };
        let _ = write!(self.body, r#"<g transform="{m}">"#);
        self.glyphs.push(glyph.to_string());
        let drawn = self.paint(inner, depth + 1);
        self.glyphs.pop();
        drawn?;
        self.body.push_str("</g>");
        self.check_size()
    }

    fn glyph(&mut self, shape: &ShapeRef, inner: &Paint, depth: usize) -> Result<(), ColorError> {
        let name = self.glyphs.last().map_or("", String::as_str);
        let d = path_data(&(self.outlines)(name, shape)).map_err(|m| self.bad(m))?;
        if d.is_empty() {
            return Ok(());
        }
        let native = shape.stroke.as_ref().filter(|s| is_native_stroke(s));
        let simple = matches!(inner, Paint::Solid(_) | Paint::Linear { .. } | Paint::Radial { .. } | Paint::Sweep { .. });
        match (native, simple) {
            (None, true) => {
                let fill = self.paint_attrs(inner, Mode::Fill)?;
                let _ = write!(self.body, r#"<path d="{d}"{fill}/>"#);
            }
            (Some(spec), true) => {
                let stroke = self.paint_attrs(inner, Mode::Stroke(spec))?;
                let _ = write!(self.body, r#"<path d="{d}" fill="none"{stroke}/>"#);
            }
            (None, false) => {
                let id = self.new_id("clip");
                let _ = write!(self.defs, r#"<clipPath id="{id}"><path d="{d}"/></clipPath>"#);
                let _ = write!(self.body, r#"<g clip-path="url(#{id})">"#);
                self.paint(inner, depth + 1)?;
                self.body.push_str("</g>");
            }
            (Some(spec), false) => {
                // A stroke whose paint is itself layered: the stroked band masks the layers.
                let id = self.new_id("mask");
                let pen = stroke_style(spec);
                let (neg, span) = (fmt(-MAX_COORDINATE), fmt(2.0 * MAX_COORDINATE));
                let _ = write!(
                    self.defs,
                    r##"<mask id="{id}" maskUnits="userSpaceOnUse" x="{neg}" y="{neg}" width="{span}" height="{span}"><path d="{d}" fill="none" stroke="#FFFFFF"{pen}/></mask>"##
                );
                let _ = write!(self.body, r#"<g mask="url(#{id})">"#);
                self.paint(inner, depth + 1)?;
                self.body.push_str("</g>");
            }
        }
        self.check_size()
    }

    /// The attributes that paint a shape with `paint`, each with a leading space.
    fn paint_attrs(&mut self, paint: &Paint, mode: Mode<'_>) -> Result<String, ColorError> {
        let (attr, opacity_attr) = match mode {
            Mode::Fill => ("fill", "fill-opacity"),
            Mode::Stroke(_) => ("stroke", "stroke-opacity"),
        };
        let mut out = String::new();
        match paint {
            Paint::Solid(color) => {
                let (value, opacity) = self.color(color)?;
                let _ = write!(out, r#" {attr}="{value}""#);
                if opacity < 1.0 {
                    let _ = write!(out, r#" {opacity_attr}="{}""#, fmt(opacity));
                }
            }
            Paint::Linear { p0, p1, p2, line } => {
                let id = self.linear(*p0, *p1, *p2, line)?;
                let _ = write!(out, r#" {attr}="url(#{id})""#);
            }
            Paint::Radial { c0, r0, c1, r1, line } => {
                let id = self.radial(*c0, *r0, *c1, *r1, line)?;
                let _ = write!(out, r#" {attr}="url(#{id})""#);
            }
            Paint::Sweep { line, .. } => {
                // SVG has no conic gradient: the first stop's colour stands in for it.
                let first = line.stops.first().map_or(ColorRef::Foreground { alpha: 1.0 }, |s| s.color);
                let (value, opacity) = self.color(&first)?;
                let _ = write!(out, r#" {attr}="{value}""#);
                if opacity < 1.0 {
                    let _ = write!(out, r#" {opacity_attr}="{}""#, fmt(opacity));
                }
            }
            Paint::Layers(_) | Paint::Glyph { .. } | Paint::ColrGlyph { .. } => {
                return Err(self.bad("a layered paint cannot fill a single shape"));
            }
        }
        if let Mode::Stroke(spec) = mode {
            out.push_str(&stroke_style(spec));
        }
        Ok(out)
    }

    /// The colour as an SVG value and its opacity from 0 to 1.
    fn color(&self, color: &ColorRef) -> Result<(String, f64), ColorError> {
        let alpha_of = |a: f64| if a.is_finite() { Ok(a.clamp(0.0, 1.0)) } else { Err(self.bad("an alpha is not a number")) };
        match *color {
            ColorRef::Palette { index, alpha } => {
                let entry = self.palette.get(usize::from(index)).ok_or(ColorError::NoSuchRow(usize::from(index)))?;
                let [_, _, _, a] = entry.to_f64();
                Ok((format!("#{:02X}{:02X}{:02X}", entry.r, entry.g, entry.b), a * alpha_of(alpha)?))
            }
            ColorRef::Foreground { alpha } => Ok(("currentColor".to_string(), alpha_of(alpha)?)),
            ColorRef::Rgba(c) => {
                if !c.iter().all(|v| v.is_finite()) {
                    return Err(self.bad("a colour component is not a number"));
                }
                let [r, g, b, a] = c.map(|v| v.clamp(0.0, 1.0));
                let byte = |v: f64| (v * 255.0).round() as u8;
                Ok((format!("#{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b)), a))
            }
        }
    }

    fn stops(&self, line: &ColorLine) -> Result<String, ColorError> {
        if line.stops.len() > MAX_STOPS {
            return Err(ColorError::LimitExceeded { what: "gradient stops", limit: MAX_STOPS });
        }
        let mut out = String::new();
        for stop in &line.stops {
            if !stop.offset.is_finite() {
                return Err(self.bad("a gradient stop offset is not a number"));
            }
            let (value, opacity) = self.color(&stop.color)?;
            let _ = write!(out, r#"<stop offset="{}" stop-color="{value}""#, fmt(stop.offset.clamp(0.0, 1.0)));
            if opacity < 1.0 {
                let _ = write!(out, r#" stop-opacity="{}""#, fmt(opacity));
            }
            out.push_str("/>");
        }
        Ok(out)
    }

    fn point(&self, p: Point) -> Result<(String, String), ColorError> {
        Ok((coordinate(p.x).map_err(|m| self.bad(m))?, coordinate(p.y).map_err(|m| self.bad(m))?))
    }

    fn linear(&mut self, p0: Point, p1: Point, p2: Point, line: &ColorLine) -> Result<String, ColorError> {
        let end = linear_end(p0, p1, p2);
        let (x1, y1) = self.point(p0)?;
        let (x2, y2) = self.point(end)?;
        let stops = self.stops(line)?;
        let id = self.new_id("linear");
        let _ = write!(
            self.defs,
            r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}"{}>{stops}</linearGradient>"#,
            spread(line.extend)
        );
        Ok(id)
    }

    fn radial(&mut self, c0: Point, r0: f64, c1: Point, r1: f64, line: &ColorLine) -> Result<String, ColorError> {
        let (fx, fy) = self.point(c0)?;
        let (cx, cy) = self.point(c1)?;
        let r = coordinate(r1.max(0.0)).map_err(|m| self.bad(m))?;
        let fr = coordinate(r0.max(0.0)).map_err(|m| self.bad(m))?;
        let stops = self.stops(line)?;
        let id = self.new_id("radial");
        let focal_radius = if r0 > 0.0 { format!(r#" fr="{fr}""#) } else { String::new() };
        let _ = write!(
            self.defs,
            r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{cx}" cy="{cy}" r="{r}" fx="{fx}" fy="{fy}"{focal_radius}{}>{stops}</radialGradient>"#,
            spread(line.extend)
        );
        Ok(id)
    }
}

/// The SVG end point of a COLR linear gradient: `p1` projected onto the line through `p0` at
/// right angles to `p0`–`p2`, so the colour bands run parallel to `p0`–`p2`. A degenerate
/// `p2` leaves `p1` as it is.
fn linear_end(p0: Point, p1: Point, p2: Point) -> Point {
    let rotation = p2 - p0;
    let normal = fontcraft_geom::Vec2::new(rotation.y, -rotation.x);
    let len2 = normal.hypot2();
    if !(len2.is_finite() && len2 > 1e-12) {
        return p1;
    }
    p0 + normal * ((p1 - p0).dot(normal) / len2)
}

fn spread(extend: Extend) -> &'static str {
    match extend {
        Extend::Pad => "",
        Extend::Repeat => r#" spreadMethod="repeat""#,
        Extend::Reflect => r#" spreadMethod="reflect""#,
    }
}

/// The stroke attributes of a native stroke, each with a leading space.
fn stroke_style(spec: &StrokeSpec) -> String {
    let cap = match spec.cap_start {
        1 => " stroke-linecap=\"round\"",
        2 => " stroke-linecap=\"square\"",
        _ => "",
    };
    let join = match spec.join {
        1 => " stroke-linejoin=\"round\"",
        2 => " stroke-linejoin=\"bevel\"",
        _ => "",
    };
    format!(r#" stroke-width="{}"{cap}{join}"#, fmt(spec.width))
}

/// A path's `d` attribute; empty for a path without segments.
fn path_data(path: &BezPath) -> Result<String, String> {
    fn push(d: &mut String, letter: char, points: &[Point]) -> Result<(), String> {
        d.push(letter);
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                d.push(' ');
            }
            let _ = write!(d, "{} {}", coordinate(p.x)?, coordinate(p.y)?);
        }
        Ok(())
    }
    let mut d = String::new();
    let mut drawn = false;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => push(&mut d, 'M', &[p])?,
            PathEl::LineTo(p) => push(&mut d, 'L', &[p])?,
            PathEl::QuadTo(a, b) => push(&mut d, 'Q', &[a, b])?,
            PathEl::CurveTo(a, b, c) => push(&mut d, 'C', &[a, b, c])?,
            PathEl::ClosePath => d.push('Z'),
        }
        drawn |= !matches!(el, PathEl::MoveTo(_));
    }
    Ok(if drawn { d } else { String::new() })
}

/// A coordinate within ±[`MAX_COORDINATE`], written short.
fn coordinate(v: f64) -> Result<String, String> {
    if !(v.is_finite() && v.abs() <= MAX_COORDINATE) {
        return Err(format!("the coordinate {v} is not a number within ±{MAX_COORDINATE}"));
    }
    Ok(fmt(v))
}

/// A number rounded to thousandths, without trailing zeros (`12`, `0.5`, `-3.125`).
fn fmt(v: f64) -> String {
    let rounded = (v * 1000.0).round() / 1000.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let mut s = format!("{rounded:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::{Affine, BezPath, Point};
    use fontcraft_model::Extend;

    use super::{BitmapPlacement, Writer, bitmap_document, bitmap_size, document, glyph_document, table};
    use crate::paint::{ColorLine, ColorRef, ColorStop, Paint, ShapeRef, StrokeSpec};
    use crate::{ColorError, MAX_SVG_DOCUMENT_BYTES, MAX_SVG_PAINT_VISITS, Rgba};

    fn square(x: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x, 0.0));
        p.line_to((x + 100.0, 0.0));
        p.line_to((x + 100.0, 100.0));
        p.line_to((x, 100.0));
        p.close_path();
        p
    }

    /// Draws shape `i` as a square at x = 200 × i.
    fn outlines(shape: &ShapeRef) -> BezPath {
        square(200.0 * shape.shapes.first().copied().unwrap_or(0) as f64)
    }

    fn shape(i: usize, stroke: Option<StrokeSpec>) -> ShapeRef {
        ShapeRef { layer: "m01".into(), shapes: vec![i], stroke }
    }

    fn stroke(position: f64) -> StrokeSpec {
        StrokeSpec { width: 20.0, height: 20.0, position, cap_start: 1, cap_end: 1, join: 0 }
    }

    fn glyph(shape: ShapeRef, paint: Paint) -> Paint {
        Paint::Glyph { shape, paint: Box::new(paint) }
    }

    const PALETTE: [Rgba; 2] = [Rgba::new(255, 0, 0, 255), Rgba::new(0, 255, 0, 255)];

    #[test]
    fn svg_document_bakes_the_palette_and_writes_gradients_and_strokes() {
        let line = ColorLine {
            stops: vec![
                ColorStop { offset: 0.0, color: ColorRef::Palette { index: 0, alpha: 1.0 } },
                ColorStop { offset: 1.0, color: ColorRef::Rgba([0.0, 0.0, 1.0, 0.25]) },
            ],
            extend: Extend::Reflect,
        };
        let paint = Paint::Layers(vec![
            glyph(shape(0, None), Paint::Solid(ColorRef::Palette { index: 1, alpha: 0.5 })),
            // Rotated: p1 sits off the axis p2 sets, so the gradient runs along x to x = 100.
            glyph(
                shape(1, None),
                Paint::Linear { p0: Point::new(0.0, 0.0), p1: Point::new(100.0, 100.0), p2: Point::new(0.0, 100.0), line: line.clone() },
            ),
            glyph(shape(2, Some(stroke(0.0))), Paint::Solid(ColorRef::Foreground { alpha: 1.0 })),
            // A stroke on one side arrives expanded and is filled.
            glyph(shape(3, Some(stroke(1.0))), Paint::Solid(ColorRef::Rgba([1.0, 0.0, 0.0, 1.0]))),
            glyph(shape(4, None), Paint::Radial { c0: Point::new(50.0, 50.0), r0: 0.0, c1: Point::new(50.0, 50.0), r1: 70.5, line }),
        ]);
        let doc = document(&paint, &outlines, 7, &PALETTE).unwrap();
        assert!(doc.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg""#), "{doc}");
        assert!(doc.contains(r#"<g id="glyph7" transform="matrix(1 0 0 -1 0 0)">"#), "{doc}");
        assert!(doc.contains(r##"<path d="M0 0L100 0L100 100L0 100Z" fill="#00FF00" fill-opacity="0.5"/>"##), "{doc}");
        assert!(doc.contains(r#"x1="0" y1="0" x2="100" y2="0" spreadMethod="reflect">"#), "{doc}");
        assert!(doc.contains(r##"<stop offset="0" stop-color="#FF0000"/><stop offset="1" stop-color="#0000FF" stop-opacity="0.25"/>"##), "{doc}");
        assert!(doc.contains(r#"fill="none" stroke="currentColor" stroke-width="20" stroke-linecap="round"/>"#), "{doc}");
        assert!(doc.contains(r##"<path d="M600 0L700 0L700 100L600 100Z" fill="#FF0000"/>"##), "{doc}");
        assert!(doc.contains(r#"cx="50" cy="50" r="70.5" fx="50" fy="50" spreadMethod="reflect">"#), "{doc}");
        assert!(usvg::Tree::from_str(&doc, &usvg::Options::default()).is_ok(), "{doc}");
    }

    #[test]
    fn svg_document_clips_layered_paints_and_refuses_bad_input() {
        let inner = Paint::Layers(vec![glyph(shape(1, None), Paint::Solid(ColorRef::Palette { index: 0, alpha: 1.0 }))]);
        let doc = document(&glyph(shape(0, None), inner), &outlines, 3, &PALETTE).unwrap();
        assert!(doc.contains(r#"<clipPath id="glyph3-clip0"><path d="M0 0L100 0L100 100L0 100Z"/></clipPath>"#), "{doc}");
        assert!(doc.contains(r#"<g clip-path="url(#glyph3-clip0)">"#), "{doc}");
        assert!(usvg::Tree::from_str(&doc, &usvg::Options::default()).is_ok(), "{doc}");

        let missing_row = glyph(shape(0, None), Paint::Solid(ColorRef::Palette { index: 9, alpha: 1.0 }));
        assert_eq!(document(&missing_row, &outlines, 3, &PALETTE), Err(ColorError::NoSuchRow(9)));
        let nan = |_: &ShapeRef| {
            let mut p = BezPath::new();
            p.move_to((f64::NAN, 0.0));
            p.line_to((1.0, 1.0));
            p
        };
        let solid = glyph(shape(0, None), Paint::Solid(ColorRef::Foreground { alpha: 1.0 }));
        assert!(matches!(document(&solid, &nan, 3, &PALETTE), Err(ColorError::BadPaint { .. })));
        let mut deep = solid;
        for _ in 0..100 {
            deep = Paint::Layers(vec![deep]);
        }
        assert!(matches!(document(&deep, &outlines, 3, &PALETTE), Err(ColorError::LimitExceeded { .. })));
    }

    #[test]
    fn svg_document_draws_a_colour_glyph_reference_as_that_glyphs_paint_moved() {
        let red = glyph(shape(0, None), Paint::Solid(ColorRef::Rgba([1.0, 0.0, 0.0, 1.0])));
        let reference = |glyph: &str, transform: Affine| Paint::ColrGlyph { glyph: glyph.into(), transform, shape: shape(1, None) };
        // `A` is a red square; `L` refers to itself; `B` refers to `A` moved, to `L`, and to `S`,
        // which has no paint.
        let a = Paint::Layers(vec![red]);
        let l = Paint::Layers(vec![reference("L", Affine::IDENTITY)]);
        let b =
            Paint::Layers(vec![reference("A", Affine::translate((100.0, 50.0))), reference("L", Affine::IDENTITY), reference("S", Affine::IDENTITY)]);
        let paints = |name: &str| match name {
            "A" => Some(&a),
            "L" => Some(&l),
            _ => None,
        };
        // Each glyph's shape i is a square at x = 200 × i, plus 1000 for `A` so the test sees whose
        // outline is drawn.
        let draw = |name: &str, s: &ShapeRef| square(if name == "A" { 1000.0 } else { 0.0 } + 200.0 * s.shapes[0] as f64);
        let doc = glyph_document("B", &b, &draw, &paints, 5, &PALETTE).unwrap().text;
        assert!(doc.contains(r##"<g transform="matrix(1 0 0 1 100 50)"><path d="M1000 0L1100 0L1100 100L1000 100Z" fill="#FF0000"/></g>"##), "{doc}");
        // `L` inlined once; its reference back to itself is its component's outline in grey, as is
        // the reference to `S`.
        assert_eq!(doc.matches(r##"<path d="M200 0L300 0L300 100L200 100Z" fill="#808080"/>"##).count(), 2, "{doc}");
        assert_eq!(doc.matches("<g transform=").count(), 2, "{doc}");
        assert!(usvg::Tree::from_str(&doc, &usvg::Options::default()).is_ok(), "{doc}");

        // Without paints to follow, the reference is the component's outline.
        let plain = document(&b, &outlines, 5, &PALETTE).unwrap();
        assert!(!plain.contains("<g transform=") && plain.contains(r##"fill="#808080""##), "{plain}");
    }

    #[test]
    fn svg_document_of_references_fanning_out_stops_at_the_visit_limit() {
        // `g0` holds two references to `g1`, `g1` two to `g2`, and so on down to `g18`, a red
        // square: copied in full, each reference to `g0` would draw 2^18 squares, more than one
        // document may hold. The document refers to `g0` 64 times.
        const LEVELS: usize = 18;
        let reference = |k: usize| Paint::ColrGlyph { glyph: format!("g{k}"), transform: Affine::translate((1.0, 0.0)), shape: shape(1, None) };
        let mut chain: Vec<Paint> = (0..LEVELS).map(|k| Paint::Layers(vec![reference(k + 1), reference(k + 1)])).collect();
        chain.push(glyph(shape(0, None), Paint::Solid(ColorRef::Rgba([1.0, 0.0, 0.0, 1.0]))));
        let paints = |name: &str| name.strip_prefix('g').and_then(|k| k.parse::<usize>().ok()).and_then(|k| chain.get(k));
        let draw = |_: &str, s: &ShapeRef| outlines(s);
        let top = Paint::Layers((0..64).map(|_| reference(0)).collect());
        let doc = glyph_document("top", &top, &draw, &paints, 1, &PALETTE).unwrap();
        let squares = doc.text.matches(r##"fill="#FF0000""##).count();
        assert!(squares > 0 && squares < MAX_SVG_PAINT_VISITS, "{squares} squares");
        // Past the limit each reference met is its component's outline in grey: the second
        // reference of each level being drawn, and the rest of the 64.
        assert!(doc.outlined_over_budget > 0 && doc.outlined_over_budget <= LEVELS + 64, "{}", doc.outlined_over_budget);
        assert_eq!(doc.text.matches(r##"fill="#808080""##).count(), doc.outlined_over_budget);
        assert!(doc.text.len() < MAX_SVG_DOCUMENT_BYTES / 2, "{} bytes", doc.text.len());
    }

    #[test]
    fn many_references_to_one_deep_graph_walk_it_once() {
        // `D` nests 80 paints deep, too deep to copy, so each of the 5000 references to it is its
        // component's outline; finding that walks `D`'s graph once, not once per reference.
        let mut deep = glyph(shape(0, None), Paint::Solid(ColorRef::Rgba([1.0, 0.0, 0.0, 1.0])));
        for _ in 0..80 {
            deep = Paint::Layers(vec![deep]);
        }
        let paints = |name: &str| (name == "D").then_some(&deep);
        let draw = |_: &str, s: &ShapeRef| outlines(s);
        let reference = || Paint::ColrGlyph { glyph: "D".into(), transform: Affine::IDENTITY, shape: shape(1, None) };
        let top = Paint::Layers((0..5000).map(|_| reference()).collect());
        let mut w = Writer::new("top", &draw, &paints, 1, &PALETTE);
        w.paint(&top, 0).unwrap();
        assert_eq!(w.nesting_walks, 1);
        assert_eq!(w.visits, 5001);
        assert_eq!(w.body.matches(r##"fill="#808080""##).count(), 5000);
    }

    #[test]
    fn svg_table_sorts_records_and_shares_identical_documents() {
        let a = b"<svg>a</svg>".to_vec();
        let b = b"<svg>bb</svg>".to_vec();
        let bytes = table(&[(3, 3, b.clone()), (1, 1, a.clone()), (5, 6, a.clone())]).unwrap();
        let be16 = |at: usize| u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        let be32 = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        assert_eq!((be16(0), be32(2), be32(6)), (0, 10, 0));
        assert_eq!(be16(10), 3);
        // Records: (start, end, offset from the list, length); the list starts at 10 and its
        // documents after 2 + 3 × 12 = 38 bytes.
        let record = |i: usize| (be16(12 + 12 * i), be16(14 + 12 * i), be32(16 + 12 * i), be32(20 + 12 * i));
        assert_eq!(record(0), (1, 1, 38, 12));
        assert_eq!(record(1), (3, 3, 50, 13));
        assert_eq!(record(2), (5, 6, 38, 12));
        assert_eq!(&bytes[48..60], a.as_slice());
        assert_eq!(&bytes[60..73], b.as_slice());
        assert_eq!(bytes.len(), 73);

        assert!(table(&[(1, 2, a.clone()), (2, 3, b.clone())]).is_err());
        assert!(table(&[(4, 2, a.clone())]).is_err());
        assert!(table(&[(1, 1, Vec::new())]).is_err());
        assert_eq!(table(&[]).unwrap().len(), 12);
    }

    /// A 2 × 3 PNG header: the signature and an IHDR chunk, enough for the size.
    fn png_2x3() -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&2u32.to_be_bytes());
        png.extend_from_slice(&3u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png
    }

    #[test]
    fn bitmap_document_embeds_the_image_where_the_icolor_layer_places_it() {
        let png = png_2x3();
        assert_eq!(bitmap_size(&png), Some((2, 3)));
        // A JPEG with an APP0 segment before its start of frame: 640 wide, 480 high.
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0x01, 0xE0, 0x02, 0x80, 1, 0, 0, 0];
        assert_eq!(bitmap_size(&jpeg), Some((640, 480)));
        assert_eq!(bitmap_size(b"GIF89a"), None);
        assert_eq!(bitmap_size(&[0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xFF]), None);

        // 1000 units per em at 128 pixels per em, the bottom left at (10, -20).
        let k = 1000.0 / 128.0;
        let placement = BitmapPlacement { transform: Affine::translate((10.0, -20.0)) * Affine::scale(k), width: 2, height: 3 };
        let doc = bitmap_document(&png, &placement, 4).unwrap();
        assert!(doc.contains(r#"<g id="glyph4" transform="matrix(1 0 0 -1 0 0)">"#), "{doc}");
        // The image's top row lands at -20 + 3 × 7.8125 font units.
        assert!(
            doc.contains(
                r#"<image width="2" height="3" transform="matrix(7.8125 0 0 -7.8125 10 3.4375)" xlink:href="data:image/png;base64,iVBORw0KGgo"#
            ),
            "{doc}"
        );
        assert!(doc.ends_with(r#""/></g></svg>"#), "{doc}");
        assert!(usvg::Tree::from_str(&doc, &usvg::Options::default()).is_ok(), "{doc}");

        assert!(bitmap_document(b"GIF89a", &placement, 4).is_err());
        assert!(bitmap_document(&png, &BitmapPlacement { width: 0, ..placement }, 4).is_err());
        assert!(bitmap_document(&png, &BitmapPlacement { transform: Affine::scale(f64::INFINITY), ..placement }, 4).is_err());
    }
}
