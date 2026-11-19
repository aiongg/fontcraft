//! Vector pictures of outlines the caller places (row `file.exportWindowContents`): one page
//! drawn with the proof's SVG and PDF writers, from outlines given in font units instead of read
//! from a compiled font.
//!
//! The caller lays the page out: glyph origins, lines and point markers are in points from the
//! top left with y growing downwards, as every proof page is; outlines are in font units with y
//! up and are drawn `scale` points per unit. Each placed glyph is marked with its name
//! (`data-glyph` in SVG, a `/Glyph` marked-content sequence in PDF), and its index in
//! [`VectorPage::glyphs`] takes the place of the glyph id.

use crate::ProofError;
use crate::font::{El, Outline};
use crate::page::{Document, GlyphStyle, Ink, Item, Page, ShownGlyph};

/// Most outlines one picture defines.
pub const MAX_VECTOR_GLYPHS: usize = 65_535;
/// Most marks (placed glyphs, lines and points) one picture draws.
pub const MAX_VECTOR_MARKS: usize = 1_000_000;
/// Most outline segments of all the picture's outlines together.
pub const MAX_VECTOR_SEGMENTS: usize = crate::font::MAX_TOTAL_ELEMENTS;
/// Allowed page sides, in points (the largest is PDF's page limit).
pub const VECTOR_PAGE_SIDE: std::ops::RangeInclusive<f64> = 1.0..=14_400.0;
/// Largest absolute coordinate of an outline, in font units.
const MAX_UNITS: f64 = 1e7;

/// One element of an outline, in font units with y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Move([f64; 2]),
    Line([f64; 2]),
    /// Control point, end point.
    Quad([f64; 2], [f64; 2]),
    /// Two control points, end point.
    Cubic([f64; 2], [f64; 2], [f64; 2]),
    Close,
}

/// An outline the page places, under its glyph name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VectorGlyph {
    pub name: String,
    pub segments: Vec<Segment>,
}

/// The colour of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorInk {
    /// A guide line.
    Guide,
    /// A handle from an on-curve point to its control point.
    Handle,
    /// A metric line, such as the baseline.
    Metric,
}

/// One thing the page draws, in page points from the top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    /// Outline `glyph` of [`VectorPage::glyphs`] with its origin at (`x`, `y`).
    Glyph {
        glyph: usize,
        x: f64,
        y: f64,
    },
    Line {
        from: [f64; 2],
        to: [f64; 2],
        ink: VectorInk,
    },
    /// A point marker: a filled square on the curve, an open circle off it.
    Node {
        at: [f64; 2],
        on_curve: bool,
    },
}

/// A one-page picture.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorPage {
    pub title: String,
    /// `[width, height]` in points, each within [`VECTOR_PAGE_SIDE`].
    pub size: [f64; 2],
    /// Points per font unit.
    pub scale: f64,
    /// Glyphs drawn with a light fill and a thin dark contour; otherwise filled black.
    pub outlined: bool,
    pub glyphs: Vec<VectorGlyph>,
    /// Drawn in order, later marks over earlier ones.
    pub marks: Vec<Mark>,
}

/// The file a picture is written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorFormat {
    /// One standalone SVG document.
    Svg,
    /// One PDF document with one page.
    Pdf,
}

/// Writes `page` as one SVG or PDF file.
pub fn write_vector(page: &VectorPage, format: VectorFormat) -> Result<Vec<u8>, ProofError> {
    let doc = document(page)?;
    match format {
        VectorFormat::Svg => crate::svg::write(&doc).into_iter().next().ok_or_else(|| ProofError::BadOptions("the picture has no page".into())),
        VectorFormat::Pdf => crate::pdf::write(&doc, None),
    }
}

fn bad(message: impl Into<String>) -> ProofError {
    ProofError::BadOptions(message.into())
}

/// The picture as a one-page proof document, after checking it against the limits.
fn document(page: &VectorPage) -> Result<Document, ProofError> {
    let [width, height] = page.size;
    if !(VECTOR_PAGE_SIDE.contains(&width) && VECTOR_PAGE_SIDE.contains(&height)) {
        return Err(bad(format!("page sides must be from {} to {} points", VECTOR_PAGE_SIDE.start(), VECTOR_PAGE_SIDE.end())));
    }
    if !(page.scale.is_finite() && page.scale > 0.0) {
        return Err(bad("the scale must be a positive finite number"));
    }
    if page.glyphs.len() > MAX_VECTOR_GLYPHS {
        return Err(bad(format!("a picture defines at most {MAX_VECTOR_GLYPHS} outlines")));
    }
    if page.marks.len() > MAX_VECTOR_MARKS {
        return Err(bad(format!("a picture draws at most {MAX_VECTOR_MARKS} glyphs, lines and points")));
    }
    let total = page.glyphs.iter().fold(0usize, |n, g| n.saturating_add(g.segments.len()));
    if total > MAX_VECTOR_SEGMENTS {
        return Err(bad(format!("the outlines have more than {MAX_VECTOR_SEGMENTS} segments")));
    }
    let mut glyphs = Vec::with_capacity(page.glyphs.len());
    for (index, g) in page.glyphs.iter().enumerate() {
        let gid = u32::try_from(index).map_err(|_| bad("too many outlines"))?;
        glyphs.push(ShownGlyph { font: 0, gid, name: g.name.clone(), unicode: None, outline: outline(&g.segments) });
    }
    let style = if page.outlined { GlyphStyle::Outlined } else { GlyphStyle::Filled };
    let mut items = Vec::with_capacity(page.marks.len());
    for mark in &page.marks {
        items.push(match *mark {
            Mark::Glyph { glyph, x, y } => {
                if glyph >= glyphs.len() {
                    return Err(bad(format!("a placed glyph names outline {glyph}, which the picture does not define")));
                }
                Item::Glyph { glyph, x: finite(x), y: finite(y), scale: page.scale, style, primary: true }
            }
            Mark::Line { from, to, ink } => {
                let ink = match ink {
                    VectorInk::Guide => Ink::Rgb(GUIDE),
                    VectorInk::Handle => Ink::Handle,
                    VectorInk::Metric => Ink::Guide,
                };
                Item::Line { from: point(from), to: point(to), ink }
            }
            Mark::Node { at, on_curve } => {
                let [x, y] = point(at);
                Item::Point { x, y, on_curve, ink: if on_curve { Ink::Point } else { Ink::Handle } }
            }
        });
    }
    Ok(Document { title: page.title.clone(), pages: vec![Page { width, height, items }], glyphs, embedded: None })
}

/// The colour of guide lines.
const GUIDE: [u8; 3] = [0x2f, 0xa8, 0x6b];

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn point([x, y]: [f64; 2]) -> [f64; 2] {
    [finite(x), finite(y)]
}

/// The writers' outline of `segments`: coordinates as `f32`, a value that is not finite or lies
/// beyond the limit dropped to 0, and the bounds of every point, control points included.
fn outline(segments: &[Segment]) -> Outline {
    let mut bounds: Option<[f64; 4]> = None;
    let mut at = |[x, y]: [f64; 2]| -> (f32, f32) {
        let (x, y) = (unit(x), unit(y));
        let [x0, y0, x1, y1] = bounds.unwrap_or([x, y, x, y]);
        bounds = Some([x0.min(x), y0.min(y), x1.max(x), y1.max(y)]);
        (x as f32, y as f32)
    };
    let mut els = Vec::with_capacity(segments.len());
    for s in segments {
        els.push(match *s {
            Segment::Move(p) => {
                let (x, y) = at(p);
                El::Move(x, y)
            }
            Segment::Line(p) => {
                let (x, y) = at(p);
                El::Line(x, y)
            }
            Segment::Quad(c, p) => {
                let (cx, cy) = at(c);
                let (x, y) = at(p);
                El::Quad(cx, cy, x, y)
            }
            Segment::Cubic(c1, c2, p) => {
                let (ax, ay) = at(c1);
                let (bx, by) = at(c2);
                let (x, y) = at(p);
                El::Cubic(ax, ay, bx, by, x, y)
            }
            Segment::Close => El::Close,
        });
    }
    Outline { els, advance: 0.0, bounds }
}

fn unit(v: f64) -> f64 {
    if v.is_finite() && v.abs() <= MAX_UNITS { v } else { 0.0 }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn square() -> VectorGlyph {
        let segments = vec![Segment::Move([0.0, 0.0]), Segment::Line([500.0, 0.0]), Segment::Quad([500.0, 700.0], [0.0, 700.0]), Segment::Close];
        VectorGlyph { name: "A&B".into(), segments }
    }

    fn page(outlined: bool) -> VectorPage {
        VectorPage {
            title: "Window".into(),
            size: [200.0, 100.0],
            scale: 0.1,
            outlined,
            glyphs: vec![square()],
            marks: vec![
                Mark::Glyph { glyph: 0, x: 10.0, y: 80.0 },
                Mark::Line { from: [0.0, 80.0], to: [200.0, 80.0], ink: VectorInk::Guide },
                Mark::Node { at: [10.0, 80.0], on_curve: true },
                Mark::Node { at: [60.0, 10.0], on_curve: false },
            ],
        }
    }

    #[test]
    fn svg_and_pdf_carry_the_placed_glyphs_and_marks() {
        let svg = String::from_utf8(write_vector(&page(true), VectorFormat::Svg).unwrap()).unwrap();
        assert!(svg.contains("width=\"200pt\" height=\"100pt\""), "{svg}");
        assert!(svg.contains("data-glyph=\"A&amp;B\""), "{svg}");
        assert!(svg.contains("<line x1=\"0\" y1=\"80\" x2=\"200\" y2=\"80\" stroke=\"#2fa86b\""), "{svg}");
        assert!(svg.contains("stroke=\"#000000\""), "outlined glyphs have a contour: {svg}");
        let pdf = write_vector(&page(false), VectorFormat::Pdf).unwrap();
        let doc = lopdf::Document::load_mem(&pdf).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Glyph"), "the placed glyph is marked");
    }

    #[test]
    fn bad_pictures_are_refused() {
        let mut p = page(false);
        p.marks.push(Mark::Glyph { glyph: 1, x: 0.0, y: 0.0 });
        assert!(write_vector(&p, VectorFormat::Svg).is_err());
        let mut p = page(false);
        p.size = [0.5, 100.0];
        assert!(write_vector(&p, VectorFormat::Pdf).is_err());
        let mut p = page(false);
        p.scale = f64::NAN;
        assert!(write_vector(&p, VectorFormat::Pdf).is_err());
        // Coordinates that are not finite are drawn at 0 instead of reaching the writers.
        let mut p = page(false);
        p.glyphs[0].segments.push(Segment::Line([f64::INFINITY, 1e300]));
        p.marks.push(Mark::Node { at: [f64::NAN, 0.0], on_curve: true });
        assert!(write_vector(&p, VectorFormat::Svg).is_ok());
    }
}
