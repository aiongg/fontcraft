//! Decoded outlines and their comparison.
//!
//! An outline is drawn through skrifa in font units, without hinting, components resolved. That
//! makes the comparison independent of how a table encodes it: implied on-curve points, flag
//! packing, point order inside a contour (the start point) and composites against flattened
//! copies all draw the same segments.

use fontcraft_geom::{BezPath, Point};
use fontcraft_pathops::coverage_difference;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::types::GlyphId;
use skrifa::{FontRef, MetadataProvider};

/// Two coordinates closer than this are the same (half a font unit).
pub const TOLERANCE: f32 = 0.5;
/// Most segments drawn for one glyph; a glyph with more is reported, not compared.
pub const MAX_SEGMENTS: usize = 200_000;
/// Most start points tried when matching two contours that may start at different points.
const MAX_ROTATIONS: usize = 64;

/// A segment's end point and control points, in drawing order (end point last).
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Line([f32; 2]),
    Quad([f32; 2], [f32; 2]),
    Cubic([f32; 2], [f32; 2], [f32; 2]),
}

impl Segment {
    fn end(&self) -> [f32; 2] {
        match *self {
            Segment::Line(p) | Segment::Quad(_, p) | Segment::Cubic(_, _, p) => p,
        }
    }

    fn close_to(&self, other: &Segment) -> bool {
        let near = |a: &[f32; 2], b: &[f32; 2]| (a[0] - b[0]).abs() <= TOLERANCE && (a[1] - b[1]).abs() <= TOLERANCE;
        match (self, other) {
            (Segment::Line(a), Segment::Line(b)) => near(a, b),
            (Segment::Quad(a1, a2), Segment::Quad(b1, b2)) => near(a1, b1) && near(a2, b2),
            (Segment::Cubic(a1, a2, a3), Segment::Cubic(b1, b2, b3)) => near(a1, b1) && near(a2, b2) && near(a3, b3),
            _ => false,
        }
    }
}

/// A closed contour: segments starting from the end point of the last one, the closing line
/// included.
pub type Contour = Vec<Segment>;

/// A glyph's outline as drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub contours: Vec<Contour>,
}

#[derive(Default)]
struct Pen {
    contours: Vec<Contour>,
    current: Contour,
    start: [f32; 2],
    count: usize,
}

impl Pen {
    fn push(&mut self, s: Segment) {
        self.count += 1;
        if self.count <= MAX_SEGMENTS {
            self.current.push(s);
        }
    }

    fn finish_contour(&mut self) {
        let mut c = std::mem::take(&mut self.current);
        if c.is_empty() {
            return;
        }
        // Some encodings spell out the closing line and some leave it implied: always spell it
        // out, so every point is a segment's end and a contour can be read from any of them.
        if c.last().map(Segment::end) != Some(self.start) {
            c.push(Segment::Line(self.start));
        }
        self.contours.push(c);
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        self.start = [x, y];
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(Segment::Line([x, y]));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.push(Segment::Quad([cx0, cy0], [x, y]));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.push(Segment::Cubic([cx0, cy0], [cx1, cy1], [x, y]));
    }
    fn close(&mut self) {
        self.finish_contour();
    }
}

/// Draws glyph `gid` of `font`: `Ok(None)` when it has no outline, an error when drawing fails or
/// the glyph is too large to compare.
pub fn draw(font: &FontRef, gid: GlyphId) -> Result<Option<Outline>, String> {
    draw_at(font, gid, LocationRef::default()).map(|(o, _)| o)
}

/// Draws glyph `gid` of a variable font at `location`, as [`draw`] does, with the advance width
/// the scaler gives (from the phantom points, where the font has them).
pub fn draw_at(font: &FontRef, gid: GlyphId, location: LocationRef) -> Result<(Option<Outline>, Option<f32>), String> {
    let glyphs = font.outline_glyphs();
    let Some(glyph) = glyphs.get(gid) else { return Ok((None, None)) };
    let mut pen = Pen::default();
    let metrics = glyph.draw(DrawSettings::unhinted(Size::unscaled(), location), &mut pen).map_err(|e| e.to_string())?;
    pen.finish_contour();
    if pen.count > MAX_SEGMENTS {
        return Err(format!("more than {MAX_SEGMENTS} segments"));
    }
    // A contour of one point draws nothing.
    pen.contours.retain(|c| !c.is_empty());
    Ok((Some(Outline { contours: pen.contours }), metrics.advance_width))
}

/// How two outlines relate.
#[derive(Clone, Debug, PartialEq)]
pub enum Match {
    /// Same segments, same start points, within [`TOLERANCE`].
    Same,
    /// Same segments within [`TOLERANCE`], but some contour starts at a different point.
    Rotated,
    /// Different: where first, as drawn in each font.
    Different { ours: String, theirs: String },
}

/// Compares two outlines. Contour order and direction are significant; start points are not.
pub fn compare(ours: &Outline, theirs: &Outline) -> Match {
    if ours.contours.len() != theirs.contours.len() {
        return Match::Different { ours: format!("{} contours", ours.contours.len()), theirs: format!("{} contours", theirs.contours.len()) };
    }
    let mut rotated = false;
    for (i, (a, b)) in ours.contours.iter().zip(&theirs.contours).enumerate() {
        if a.len() != b.len() {
            return Match::Different { ours: format!("contour {i}: {} segments", a.len()), theirs: format!("contour {i}: {} segments", b.len()) };
        }
        if same_from(a, b, 0) {
            continue;
        }
        if rotation(a, b).is_some() {
            rotated = true;
            continue;
        }
        if rotation(&reversed(a), b).is_some() || same_from(&reversed(a), b, 0) {
            return Match::Different { ours: format!("contour {i}: one direction"), theirs: format!("contour {i}: the opposite direction") };
        }
        let first = a.iter().zip(b).position(|(x, y)| !x.close_to(y)).unwrap_or(0);
        let show = |c: &Contour| format!("contour {i}, segment {first}: {}", c.get(first).map(describe).unwrap_or_default());
        return Match::Different { ours: show(a), theirs: show(b) };
    }
    if rotated { Match::Rotated } else { Match::Same }
}

/// The largest coverage difference of one pixel, and the share of the reference's ink the
/// differences may add up to, for two outlines to be coverage-equal (docs/plans/M3.md § 5).
pub const COVERAGE_PIXEL: f64 = 0.5;
pub const COVERAGE_SHARE: f64 = 0.005;

/// `outline` as Bézier paths in font units, one per contour.
pub fn to_paths(outline: &Outline) -> Vec<BezPath> {
    let pt = |q: &[f32; 2]| Point::new(f64::from(q[0]), f64::from(q[1]));
    outline
        .contours
        .iter()
        .filter_map(|c| {
            let start = c.last()?.end();
            let mut path = BezPath::new();
            path.move_to(pt(&start));
            for s in c {
                match s {
                    Segment::Line(p) => path.line_to(pt(p)),
                    Segment::Quad(a, p) => path.quad_to(pt(a), pt(p)),
                    Segment::Cubic(a, b, p) => path.curve_to(pt(a), pt(b), pt(p)),
                }
            }
            path.close_path();
            Some(path)
        })
        .collect()
}

/// Do `ours` and `theirs` cover the same pixels? Both are filled with the nonzero rule at one
/// pixel per unit; they are coverage-equal when no pixel differs by more than
/// [`COVERAGE_PIXEL`] and the differences add up to at most [`COVERAGE_SHARE`] of the
/// reference's ink. An error when the outlines cannot be rasterised (too large, out of range).
pub fn coverage_equal(ours: &Outline, theirs: &Outline) -> Result<Coverage, String> {
    let d = coverage_difference(&to_paths(theirs), &to_paths(ours)).map_err(|e| e.to_string())?;
    let share = if d.ink > 0.0 { d.total / d.ink } else { d.total };
    Ok(Coverage { equal: d.within(COVERAGE_PIXEL, COVERAGE_SHARE), max_pixel: d.max_pixel, share })
}

/// How two outlines' coverage compared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coverage {
    pub equal: bool,
    /// Largest difference of one pixel, 0 to 1.
    pub max_pixel: f64,
    /// The differences added up, as a share of the reference's ink.
    pub share: f64,
}

/// `c` drawn the other way round, from the same start point.
fn reversed(c: &Contour) -> Contour {
    let n = c.len();
    let end = |i: usize| c.get(i % n).map(Segment::end).unwrap_or_default();
    (0..n)
        .map(|j| {
            // Segment j of the reversed contour runs along segment n-1-j of `c`, backwards: it
            // ends where that segment started.
            let i = n - 1 - j;
            let to = end(i + n - 1);
            match c.get(i) {
                Some(Segment::Quad(p, _)) => Segment::Quad(*p, to),
                Some(Segment::Cubic(p1, p2, _)) => Segment::Cubic(*p2, *p1, to),
                _ => Segment::Line(to),
            }
        })
        .collect()
}

/// Is `b`, read from segment `k` on, the same as `a`?
fn same_from(a: &Contour, b: &Contour, k: usize) -> bool {
    let n = b.len();
    a.iter().enumerate().all(|(i, s)| b.get((i + k) % n).is_some_and(|t| s.close_to(t)))
}

/// The start offset at which `b` matches `a`, if any.
fn rotation(a: &Contour, b: &Contour) -> Option<usize> {
    let first = a.first()?;
    (1..b.len()).filter(|&k| b.get(k).is_some_and(|t| t.close_to(first))).take(MAX_ROTATIONS).find(|&k| same_from(a, b, k))
}

fn describe(s: &Segment) -> String {
    let p = |q: &[f32; 2]| format!("({} {})", q[0], q[1]);
    match s {
        Segment::Line(a) => format!("line to {}", p(a)),
        Segment::Quad(c, a) => format!("quad {} to {}", p(c), p(a)),
        Segment::Cubic(c1, c2, a) => format!("cubic {} {} to {}", p(c1), p(c2), p(a)),
    }
}
