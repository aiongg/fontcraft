//! SVG outlines and the clipboard fragment (L3).
//!
//! - [`read_svg`]: the geometry of an SVG drawing as glyph paths (cubic, y up). Only geometry is
//!   read (`import.outlinesDropStyling`): styling, text, images, gradients, clip paths, masks,
//!   markers and filters are left out, and [`Dropped`] says what was. Paths and basic shapes are
//!   converted after every transform is applied.
//! - [`shapes_to_svg`], [`bez_to_svg`]: outlines as a compact SVG document, for other apps.
//! - [`clipboard`]: our own JSON fragment of shapes, anchors and guides, with a version field.
//! - [`svg_font`]: SVG 1.1 fonts read into a document and written from one master.
//! - [`read_eps`]: the outlines of an EPS file, from a small fixed subset of PostScript (see
//!   [`eps`]).
//!
//! SVG input is untrusted. It is checked against the limits below before usvg sees it, rewritten
//! as a geometry-only document (so usvg never reads files, decodes images or runs filters), and
//! the result is checked again. The functions work on file contents; reading files is the
//! caller's job, so they also run in the browser.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use fontcraft_model::MAX_COORDINATE;

pub mod clipboard;
pub mod eps;
mod read;
mod sanitize;
pub mod svg_font;
mod write;

pub use eps::{EpsError, EpsOutlines, looks_like_eps, read_eps};
pub use read::{Placement, SvgOutlines, read_svg};
pub use write::{bez_to_svg, shapes_to_svg};

/// Largest SVG input, in bytes.
pub const MAX_SVG_BYTES: usize = 8 << 20;
/// Most elements an SVG document may have.
pub const MAX_SVG_ELEMENTS: usize = 100_000;
/// Deepest element nesting, counting each `use` reference as one more level.
pub const MAX_SVG_DEPTH: usize = 64;
/// Most elements a document may have once every `use` reference is expanded.
pub const MAX_SVG_EXPANDED_ELEMENTS: usize = 200_000;
/// Most numbers in the geometry attributes (`d`, `points`, `transform`…), with every `use`
/// reference expanded.
pub const MAX_SVG_NUMBERS: usize = 2_000_000;
/// Largest absolute value of a number in a geometry attribute.
pub const MAX_SVG_NUMBER: f64 = 1e7;
/// Most paths (contours) one drawing may give.
pub const MAX_SVG_PATHS: usize = 10_000;
/// Most nodes, on-curve and off-curve, one drawing may give.
pub const MAX_SVG_NODES: usize = 200_000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SvgError {
    #[error("the SVG is {size} bytes; the limit is {MAX_SVG_BYTES}")]
    TooLarge { size: usize },
    #[error("the SVG is compressed (SVGZ); save it uncompressed and try again")]
    Compressed,
    #[error("the SVG is not UTF-8 text")]
    NotUtf8,
    #[error("the SVG could not be read: {0}")]
    Parse(String),
    #[error("the document's root element is not <svg>")]
    NoSvgRoot,
    #[error("the SVG has more than {MAX_SVG_ELEMENTS} elements")]
    TooManyElements,
    #[error("the SVG's elements nest deeper than {MAX_SVG_DEPTH} levels (counting `use` references), or a `use` refers to itself")]
    TooDeep,
    #[error("the SVG expands to more than {MAX_SVG_EXPANDED_ELEMENTS} elements through `use` references")]
    TooManyExpanded,
    #[error("the SVG has more than {MAX_SVG_NUMBERS} numbers of path data and coordinates")]
    TooMuchData,
    #[error("the SVG has a number beyond ±{MAX_SVG_NUMBER}, or one that is not a number")]
    NumberOutOfRange,
    #[error("the drawing has more than {MAX_SVG_PATHS} paths")]
    TooManyPaths,
    #[error("the drawing has more than {MAX_SVG_NODES} nodes")]
    TooManyNodes,
    #[error("a point of the drawing lands beyond ±{MAX_COORDINATE} font units")]
    CoordinateOutOfRange,
    #[error("the placement is not usable: {0}")]
    BadPlacement(String),
    #[error("a path cannot be written: {0}")]
    BadPath(String),
}

/// What reading an SVG left out because it is not geometry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    /// Elements left out with everything inside them (`text`, `image`, `style`, `clipPath`,
    /// `linearGradient`…), by name, with how many. Titles, descriptions, metadata and elements of
    /// other XML namespaces (editor data) are left out without being counted.
    pub elements: BTreeMap<String, usize>,
    /// Attributes left out of the elements that were read (`fill`, `stroke`, `stroke-width`,
    /// `style`, `class`, `opacity`, `clip-path`…), by name, with how many.
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

/// Does `text` look like an SVG document (rather than our clipboard fragment or plain text)?
pub fn looks_like_svg(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with('<') && t.contains("<svg")
}

/// The glyph name an imported file maps to: its base name without the `.svg`, `.svgz`, `.eps` or
/// `.pdf` extension (`file.importOutlines` matches files to glyphs this way).
pub fn glyph_name_for_file(path: &str) -> &str {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ["svg", "svgz", "eps", "pdf"].iter().any(|e| ext.eq_ignore_ascii_case(e)) => stem,
        _ => base,
    }
}

#[cfg(test)]
mod tests;
