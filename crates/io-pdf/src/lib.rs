//! PDF import: the outlines of a PDF page's content stream as paths, and a PDF page as a glyph's
//! image (docs/plans/M13.md › M13.58; rows `pdf-outline` and `image-pdf`).
//!
//! - [`read_pdf`]: the path operators of the first page as glyph paths (cubic, y up). Filled
//!   paths give closed contours; stroked paths keep their line width as the path's stroke width
//!   (and a path both filled and stroked is marked as filled too). Colour, line style, text,
//!   images, shadings and clipping are left out, and [`Dropped`] says what was.
//! - [`render_pdf`]: the first page drawn to RGBA pixels (filled and stroked paths in their
//!   colours, on a transparent background), for showing a placed PDF as a template image.
//!
//! What runs: path construction (`m l c v y h re`), painting (`f F f* S s B B* b b* n`), clipping
//! (`W W*`, read and left out), the graphics state (`q Q cm w gs`, colours), form XObjects (`Do`,
//! with their `/Matrix`), text objects and inline images (skipped). Other operators are skipped.
//! The page's `/Rotate` turns the page as a viewer shows it.
//!
//! Placement: a PDF's y axis points up, like a glyph's. The frame is the page's crop box, else its
//! media box, else the bounds of the drawing. [`Placement::Unscaled`] keeps one point per font
//! unit and moves the frame's top edge to `top`; [`Placement::FitEm`] scales the frame's height
//! to the em, with its top edge at the ascender and its left edge at x = 0;
//! [`Placement::Origin`] keeps one point per font unit with the frame's bottom-left corner at the
//! glyph's origin, as a paste does.
//!
//! PDF input is untrusted. Before the parser sees the file, its size and its number of streams
//! are checked; every stream is decoded with a bound of [`MAX_PDF_EXPANSION`] times its stored
//! size and the [`MAX_PDF_DECOMPRESSED`] bytes the whole file may decode to; nesting, operators,
//! operands, segments, paths and pixels each have a limit below. The functions work on file
//! contents; reading files is the caller's job, so they also run in the browser.
//!
//! [`render_pdf`] and [`page_size`] parse a file once for the whole process: the parsed first
//! page is kept in a small cache keyed by a hash of the file's contents (see
//! [`MAX_PDF_CACHE_ENTRIES`]), so the engine and the window drawing the same placed PDF share
//! one parse. [`read_pdf`] reads pasted and imported files, which come once, without the cache.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;

use fontcraft_imagery::Rgba8;
use fontcraft_model::{MAX_COORDINATE, Path};

mod cache;
mod content;
mod load;
mod outline;
mod raster;

pub use cache::{MAX_PDF_CACHE_BYTES, MAX_PDF_CACHE_ENTRIES};

/// Largest PDF input, in bytes.
pub const MAX_PDF_BYTES: usize = 256 << 20;
/// Most bytes every stream of one file may decode to, together.
pub const MAX_PDF_DECOMPRESSED: usize = 256 << 20;
/// Most times its stored size a stream may grow when decoded.
pub const MAX_PDF_EXPANSION: usize = 100;
/// What a short stream may decode to whatever its stored size, so that small streams with a
/// high but harmless ratio still open.
pub const MIN_PDF_STREAM_ALLOWANCE: usize = 64 << 10;
/// Largest stream decoded while the file's structure is read (cross-reference and object
/// streams).
pub const MAX_PDF_STREAM_BYTES: usize = 16 << 20;
/// The bound on a structure stream when the file holds very many streams; see
/// [`MAX_PDF_STREAMS`].
pub const MIN_PDF_STRUCTURE_STREAM_BYTES: usize = 64 << 10;
/// Most streams (counted by their `stream` keywords) a file may hold.
pub const MAX_PDF_STREAMS: usize = 10_000;
/// Most filters applied to one stream.
pub const MAX_PDF_FILTERS: usize = 8;
/// Deepest nesting: page tree levels, saved graphics states (`q`), and form XObjects inside one
/// another (each counts as one saved state).
pub const MAX_PDF_DEPTH: usize = 64;
/// Most page tree nodes visited while looking for a page.
pub const MAX_PDF_PAGE_NODES: usize = 100_000;
/// Most operators run, over the page and every form it draws.
pub const MAX_PDF_OPERATORS: usize = 10_000_000;
/// Most bytes of content streams read, counting a form once per use.
pub const MAX_PDF_SCANNED_BYTES: usize = 1 << 30;
/// Most operands waiting for an operator.
pub const MAX_PDF_OPERANDS: usize = 10_000;
/// Most path segments built and painted, over the whole page.
pub const MAX_PDF_SEGMENTS: usize = 1_000_000;
/// Largest absolute value of a number, read or computed, and of a transformed coordinate.
pub const MAX_PDF_NUMBER: f64 = 1e9;
/// Most paths (contours) one page may give.
pub const MAX_PDF_PATHS: usize = 10_000;
/// Most nodes, on-curve and off-curve, one page may give.
pub const MAX_PDF_NODES: usize = 200_000;
/// Most pixel visits while drawing a page: each painted path costs the pixels of its bounding
/// box.
pub const MAX_PDF_RENDER_WORK: u64 = 1 << 30;

#[derive(Clone, Debug, PartialEq)]
pub enum PdfError {
    TooLarge {
        size: usize,
    },
    NotPdf,
    TooManyStreams,
    /// A stream decodes to more than its allowance, or past the file's total.
    StreamTooLarge,
    Encrypted,
    Read(String),
    NoPage,
    TooDeep,
    TooMuchWork,
    TooMuchData,
    NumberOutOfRange,
    TooManyPaths,
    TooManyNodes,
    CoordinateOutOfRange,
    BadPlacement(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PdfError::TooLarge { size } => write!(f, "the PDF file is {size} bytes; the limit is {MAX_PDF_BYTES}"),
            PdfError::NotPdf => write!(f, "the file is not a PDF: it has no `%PDF-` header"),
            PdfError::TooManyStreams => write!(f, "the PDF has more than {MAX_PDF_STREAMS} streams"),
            PdfError::StreamTooLarge => write!(
                f,
                "a compressed stream of the PDF expands more than {MAX_PDF_EXPANSION} times its size, or the file expands beyond {MAX_PDF_DECOMPRESSED} bytes in all"
            ),
            PdfError::Encrypted => write!(f, "the PDF is encrypted with a password; save an unprotected copy and try again"),
            PdfError::Read(why) => write!(f, "the PDF cannot be read: {why}"),
            PdfError::NoPage => write!(f, "the PDF has no page"),
            PdfError::TooDeep => {
                write!(f, "the PDF nests pages, saved graphics states or forms more than {MAX_PDF_DEPTH} deep, or a form draws itself")
            }
            PdfError::TooMuchWork => write!(
                f,
                "the page runs more than {MAX_PDF_OPERATORS} operators, reads more than {MAX_PDF_SCANNED_BYTES} bytes of content, or takes more than {MAX_PDF_RENDER_WORK} pixel steps to draw"
            ),
            PdfError::TooMuchData => {
                write!(f, "the page has more than {MAX_PDF_SEGMENTS} path segments or {MAX_PDF_OPERANDS} operands in a row")
            }
            PdfError::NumberOutOfRange => write!(f, "the page has a number beyond ±{MAX_PDF_NUMBER}, or one that is not a number"),
            PdfError::TooManyPaths => write!(f, "the drawing has more than {MAX_PDF_PATHS} paths"),
            PdfError::TooManyNodes => write!(f, "the drawing has more than {MAX_PDF_NODES} nodes"),
            PdfError::CoordinateOutOfRange => write!(f, "a point of the drawing lands beyond ±{MAX_COORDINATE} font units"),
            PdfError::BadPlacement(why) => write!(f, "the placement is not usable: {why}"),
        }
    }
}

impl std::error::Error for PdfError {}

/// Where the page lands in the glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// One point per font unit: a point at page `(x, y)` lands at
    /// `(x, top - (frame top - y))`.
    Unscaled { top: f64 },
    /// The frame's height becomes `units_per_em`, its top edge lands at `ascender` and its left
    /// edge at x = 0.
    FitEm { units_per_em: f64, ascender: f64 },
    /// One point per font unit, the frame's bottom-left corner at `(0, 0)`: a point at page
    /// `(x, y)` lands at `(x - frame left, y - frame bottom)`.
    Origin,
}

/// What reading a page left out because it is not outline geometry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    /// Parts left out (`text`, `image`, `shading`, `clipping path`, `unpainted path`…), by name,
    /// with how many.
    pub elements: BTreeMap<String, usize>,
    /// Styling left out (`colour`, `line style`, `graphics state`), by name, with how many.
    pub attributes: BTreeMap<String, usize>,
}

impl Dropped {
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty() && self.attributes.is_empty()
    }

    /// One line for the user naming what was left out, or `None` when nothing was.
    pub fn summary(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if !self.elements.is_empty() {
            parts.push(format!("elements {}", list(&self.elements)));
        }
        if !self.attributes.is_empty() {
            parts.push(format!("styling {}", list(&self.attributes)));
        }
        Some(format!("Only outlines were read; left out: {}.", parts.join("; ")))
    }

    fn element(&mut self, what: &str) {
        let n = self.elements.entry(what.into()).or_default();
        *n = n.saturating_add(1);
    }

    fn attribute(&mut self, what: &str) {
        let n = self.attributes.entry(what.into()).or_default();
        *n = n.saturating_add(1);
    }
}

/// `name`, or `name ×n` when there are several, for the first few names.
fn list(names: &BTreeMap<String, usize>) -> String {
    const SHOWN: usize = 12;
    let mut out: Vec<String> = names.iter().take(SHOWN).map(|(name, n)| if *n > 1 { format!("{name} ×{n}") } else { name.clone() }).collect();
    if names.len() > SHOWN {
        out.push(format!("and {} more", names.len() - SHOWN));
    }
    out.join(", ")
}

/// What [`read_pdf`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfOutlines {
    /// The contours, in painting order: closed or open, lines and cubic curves, on-curve nodes
    /// marked smooth where the curve runs through them without a corner. Coordinates and stroke
    /// widths are rounded to 0.001 units.
    pub paths: Vec<Path>,
    /// The first page's crop box, else its media box, as `[llx, lly, urx, ury]` in points, before
    /// the page's rotation.
    pub page_box: Option<[f64; 4]>,
    /// What was left out because it is not outline geometry.
    pub dropped: Dropped,
}

/// What [`render_pdf`] drew.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfRaster {
    /// The page, rows top to bottom, transparent where nothing is painted. The page's
    /// bottom-left corner is the image's; when the page's size in pixels is not whole, the part
    /// pixels lie along the top and right edges.
    pub image: Rgba8,
    /// Pixels per point of the image: the one asked for, or less when the page would have had
    /// more pixels than allowed. An image placed at `72 × pixels_per_point` pixels per inch lands
    /// at one point per font unit.
    pub pixels_per_point: f64,
    /// The first page's crop box, else its media box (see [`PdfOutlines::page_box`]).
    pub page_box: Option<[f64; 4]>,
    /// What was not drawn: text, images and shadings.
    pub dropped: Dropped,
}

/// Does `data` look like a PDF (a `%PDF-` header in its first kilobyte)?
pub fn looks_like_pdf(data: &[u8]) -> bool {
    data.get(..data.len().min(1024)).is_some_and(|head| head.windows(5).any(|w| w == b"%PDF-"))
}

/// The outlines of the first page of a PDF, placed by `placement`. See the crate documentation
/// for what is read.
pub fn read_pdf(data: &[u8], placement: Placement) -> Result<PdfOutlines, PdfError> {
    let page = load::first_page_drawing(data)?;
    let paths = outline::paths(&page, placement)?;
    Ok(PdfOutlines { paths, page_box: page.page_box, dropped: page.dropped })
}

/// The first page of a PDF drawn at `pixels_per_point` (at most 16), with no more than
/// `max_pixels` pixels (itself capped at the image limit of `fontcraft_imagery`); a page that
/// would be larger is drawn at a lower resolution.
pub fn render_pdf(data: &[u8], pixels_per_point: f64, max_pixels: u64) -> Result<PdfRaster, PdfError> {
    raster_of(&cache::first_page(data)?, pixels_per_point, max_pixels)
}

/// As [`render_pdf`], for page `index` (from 0); a page the file lacks is [`PdfError::NoPage`]. The
/// file is read for the call, not kept.
pub fn render_pdf_page(data: &[u8], index: usize, pixels_per_point: f64, max_pixels: u64) -> Result<PdfRaster, PdfError> {
    raster_of(&load::page_drawing(data, index)?, pixels_per_point, max_pixels)
}

fn raster_of(page: &content::PageDrawing, pixels_per_point: f64, max_pixels: u64) -> Result<PdfRaster, PdfError> {
    let (image, pixels_per_point) = raster::render(page, pixels_per_point, max_pixels)?;
    let mut dropped = Dropped::default();
    for (what, n) in &page.dropped.elements {
        if matches!(what.as_str(), "text" | "image" | "shading") {
            dropped.elements.insert(what.clone(), *n);
        }
    }
    Ok(PdfRaster { image, pixels_per_point, page_box: page.page_box, dropped })
}

/// The size of a PDF's first page in points, `(width, height)`, as [`render_pdf`] frames it: the
/// crop box, else the media box, turned by the page's rotation; for a page with neither, the
/// bounds of its drawing. A PDF placed as an image maps one point to one unit of its own space,
/// so this is the size such an image has where other formats have a pixel size.
pub fn page_size(data: &[u8]) -> Result<(f64, f64), PdfError> {
    let page = cache::first_page(data)?;
    let [x0, y0, x1, y1] = page.frame.or_else(|| outline::bounds(&page.painted)).ok_or(PdfError::NoPage)?;
    let (w, h) = (x1 - x0, y1 - y0);
    if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 { Ok((w, h)) } else { Err(PdfError::NoPage) }
}

#[cfg(test)]
mod tests;
