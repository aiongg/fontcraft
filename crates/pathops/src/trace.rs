//! Outline tracing of a bitmap (row `glyph.autotraceImage`), written from published descriptions
//! of outline tracing: boundary following on the pixel grid, polygon simplification, corner
//! detection and curve fitting. No tracer's source was read.
//!
//! | Step | What happens |
//! |---|---|
//! | Threshold | A pixel is ink when its darkness is above the threshold. |
//! | Boundaries | Every border between ink and paper is walked along pixel edges with ink on the left, so outer contours run counter-clockwise and counters clockwise (y up). Ink pixels that touch only at a corner belong to separate contours. Contours with less area than [`MIN_AREA`] (specks) are dropped. |
//! | Staircases | Each straight run of pixel edges becomes the midpoints of its first and last unit edge, which turns a pixel staircase into a straight line. A short diagonal between two long runs that turn the same way is put back as the sharp corner it cut. |
//! | Corners | The smoothed polygon is simplified (Douglas–Peucker); a vertex of the simplified polygon that turns by more than the corner angle is a corner. |
//! | Curves | Between two corners, cubic curves are fitted to the smoothed polygon ([`fontcraft_geom::fit`]); a side with no simplified vertex between its corners is a straight line. |
//!
//! Coordinates are pixels: y up, the origin at the bitmap's bottom-left corner. The caller maps
//! them into the glyph.

use std::fmt;

use fontcraft_geom::fit::{MAX_FIT_POINTS, fit_polyline};
use fontcraft_geom::{BezPath, PathEl, Point, Vec2};

/// Most pixels one bitmap may have (64 megapixels).
pub const MAX_PIXELS: u64 = 1 << 26;
/// Most contours one trace keeps.
pub const MAX_CONTOURS: usize = 100_000;
/// Most boundary corners one trace keeps, over all its contours.
pub const MAX_CORNERS: usize = 8_000_000;
/// Contours with less area than this, in square pixels, are specks and are dropped.
pub const MIN_AREA: f64 = 2.5;
/// The darkness, from 0 (paper) to 1 (ink), above which a pixel is ink, unless the caller says.
pub const DEFAULT_THRESHOLD: f64 = 0.5;
/// How smooth the result is, from 0 to 1, unless the caller says.
pub const DEFAULT_SMOOTHNESS: f64 = 0.25;
/// How far, in pixels, the simplified polygon that finds corners may stray from the smoothed one.
/// Above the half pixel a staircase zigzags by, so staircases do not make corners.
const SIMPLIFY_TOLERANCE: f64 = 0.6;
/// Longest edge, in pixels, that counts as a cut corner (the diagonal of half a pixel is 0.71).
const MAX_CHAMFER: f64 = 0.75;
/// Shortest edge, in pixels, on either side of a cut corner for it to be put back.
const MIN_CHAMFER_SIDE: f64 = 1.5;
/// Simplification gives up, keeping every point, after this many distance tests per point.
const MAX_SIMPLIFY_WORK_PER_POINT: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum TraceError {
    /// The bitmap has no pixels.
    Empty,
    /// The bitmap has more than [`MAX_PIXELS`] pixels.
    TooLarge { width: u32, height: u32 },
    /// The pixel data does not match the size.
    BadPixels,
    /// An option out of its range.
    BadOption(String),
    /// More than [`MAX_CONTOURS`] contours.
    TooManyContours(usize),
    /// More than [`MAX_CORNERS`] boundary corners.
    TooDetailed,
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TraceError::Empty => write!(f, "the image has no pixels to trace"),
            TraceError::TooLarge { width, height } => {
                write!(f, "the image is {width} × {height} pixels; at most {MAX_PIXELS} pixels are traced")
            }
            TraceError::BadPixels => write!(f, "the pixel data does not match the image size"),
            TraceError::BadOption(m) => write!(f, "{m}"),
            TraceError::TooManyContours(n) => {
                write!(f, "the trace makes more than {MAX_CONTOURS} contours ({n}); raise the threshold or clean up the image")
            }
            TraceError::TooDetailed => {
                write!(f, "the trace has more than {MAX_CORNERS} boundary corners; raise the threshold or clean up the image")
            }
        }
    }
}

impl std::error::Error for TraceError {}

/// The pixel count of a `width × height` bitmap, refused when empty or beyond [`MAX_PIXELS`].
fn pixel_count(width: u32, height: u32) -> Result<usize, TraceError> {
    if width == 0 || height == 0 {
        return Err(TraceError::Empty);
    }
    let n = u64::from(width) * u64::from(height);
    if n > MAX_PIXELS {
        return Err(TraceError::TooLarge { width, height });
    }
    usize::try_from(n).map_err(|_| TraceError::TooLarge { width, height })
}

/// A grey bitmap to trace: one darkness byte per pixel (0 paper, 255 ink), rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    width: u32,
    height: u32,
    darkness: Vec<u8>,
}

impl Bitmap {
    /// Wraps `width × height` darkness bytes. The size is checked before the data.
    pub fn from_darkness(width: u32, height: u32, darkness: Vec<u8>) -> Result<Self, TraceError> {
        let n = pixel_count(width, height)?;
        if darkness.len() != n {
            return Err(TraceError::BadPixels);
        }
        Ok(Self { width, height, darkness })
    }

    /// Straight (not premultiplied) RGBA pixels, rows top to bottom, laid on white paper: a
    /// pixel's darkness is its alpha times one minus its luminance.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Self, TraceError> {
        let n = pixel_count(width, height)?;
        let (pixels, rest) = rgba.as_chunks::<4>();
        if !rest.is_empty() || pixels.len() != n {
            return Err(TraceError::BadPixels);
        }
        let darkness = pixels
            .iter()
            .map(|&[r, g, b, a]| {
                // Rec. 709 luminance weights, in ten-thousandths.
                let lum = (2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) + 5000) / 10_000;
                let d = (255u32.saturating_sub(lum) * u32::from(a) + 127) / 255;
                u8::try_from(d).unwrap_or(u8::MAX)
            })
            .collect();
        Ok(Self { width, height, darkness })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The part of the bitmap `width × height` pixels large whose bottom-left corner is pixel
    /// `(x, y)`, counted from the bottom-left; clipped to the bitmap.
    pub fn cropped(&self, x: u32, y: u32, width: u32, height: u32) -> Result<Bitmap, TraceError> {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        let n = pixel_count(width, height)?;
        let mut darkness = Vec::with_capacity(n);
        let stride = self.width as usize;
        for row in 0..height {
            // Output rows run top to bottom: the first is the crop's top row.
            let from_bottom = y + height - 1 - row;
            let source_row = (self.height - 1 - from_bottom) as usize;
            let start = source_row * stride + x as usize;
            let line = self.darkness.get(start..start + width as usize).ok_or(TraceError::BadPixels)?;
            darkness.extend_from_slice(line);
        }
        Bitmap::from_darkness(width, height, darkness)
    }

    /// Whether the pixel whose bottom-left corner is `(x, y)` (y up) is darker than `cut`;
    /// pixels outside the bitmap are paper.
    fn ink(&self, (x, y): (i64, i64), cut: u8) -> bool {
        let (w, h) = (i64::from(self.width), i64::from(self.height));
        if x < 0 || y < 0 || x >= w || y >= h {
            return false;
        }
        let at = (h - 1 - y) * w + x;
        usize::try_from(at).ok().and_then(|i| self.darkness.get(i)).is_some_and(|d| *d > cut)
    }
}

/// How a bitmap is traced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceOptions {
    /// The darkness above which a pixel is ink, greater than 0 and less than 1.
    pub threshold: f64,
    /// From 0 to 1: higher values keep fewer corners and fit curves more loosely.
    pub smoothness: f64,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self { threshold: DEFAULT_THRESHOLD, smoothness: DEFAULT_SMOOTHNESS }
    }
}

/// What the options mean for the trace.
struct Settings {
    /// Ink is darkness above this byte.
    cut: u8,
    /// A simplified vertex turning more than this, in degrees, is a corner.
    corner_degrees: f64,
    /// How far, in pixels, fitted curves may stray from the smoothed polygon.
    accuracy: f64,
}

impl TraceOptions {
    /// Refuses a threshold or smoothness out of its range, as [`trace`] would.
    pub fn validate(&self) -> Result<(), TraceError> {
        self.settings().map(|_| ())
    }

    fn settings(&self) -> Result<Settings, TraceError> {
        if !(self.threshold.is_finite() && self.threshold > 0.0 && self.threshold < 1.0) {
            return Err(TraceError::BadOption(format!("the threshold must be above 0 and below 1, not {}", self.threshold)));
        }
        if !(self.smoothness.is_finite() && (0.0..=1.0).contains(&self.smoothness)) {
            return Err(TraceError::BadOption(format!("the smoothness must be from 0 to 1, not {}", self.smoothness)));
        }
        let cut = (self.threshold * 255.0).round().clamp(0.0, 254.0) as u8;
        Ok(Settings { cut, corner_degrees: 30.0 + 120.0 * self.smoothness, accuracy: 0.3 + 1.2 * self.smoothness })
    }
}

/// The outlines of the ink in `bitmap`, one closed path per contour, in pixel coordinates (y up,
/// origin at the bottom-left corner). Outer contours run counter-clockwise, counters clockwise.
pub fn trace(bitmap: &Bitmap, options: &TraceOptions) -> Result<Vec<BezPath>, TraceError> {
    let settings = options.settings()?;
    let contours = boundaries(bitmap, settings.cut)?;
    Ok(contours.iter().filter_map(|c| fit_contour(c, &settings)).collect())
}

// ── boundaries ──────────────────────────────────────────────────────────────────────────────────

/// One bit per pixel.
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Self {
        Bits(vec![0; n.div_ceil(64)])
    }

    fn get(&self, i: usize) -> bool {
        self.0.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    fn set(&mut self, i: usize) {
        if let Some(w) = self.0.get_mut(i / 64) {
            *w |= 1 << (i % 64);
        }
    }
}

/// Directions along pixel edges: +x, +y, −x, −y.
const STEPS: [(i64, i64); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
const DOWN: usize = 3;

/// The pixels ahead-left and ahead-right of a walker at vertex `(x, y)` heading `d`.
fn ahead((x, y): (i64, i64), d: usize) -> ((i64, i64), (i64, i64)) {
    match d {
        0 => ((x, y), (x, y - 1)),
        1 => ((x - 1, y), (x, y)),
        2 => ((x - 1, y - 1), (x - 1, y)),
        _ => ((x, y - 1), (x - 1, y - 1)),
    }
}

/// The boundary polygons of the ink, as the corners of their pixel-edge walks; specks dropped.
fn boundaries(b: &Bitmap, cut: u8) -> Result<Vec<Vec<Point>>, TraceError> {
    let (w, h) = (i64::from(b.width), i64::from(b.height));
    let index = |(x, y): (i64, i64)| usize::try_from(y * w + x).unwrap_or(usize::MAX);
    // A pixel whose left edge has been walked (downward, with the pixel on the left).
    let mut walked = Bits::new(b.darkness.len());
    let max_steps = 4 * (w + 1) * (h + 1);
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut used = 0usize;
    for y in 0..h {
        for x in 0..w {
            if !b.ink((x, y), cut) || b.ink((x - 1, y), cut) || walked.get(index((x, y))) {
                continue;
            }
            let corners = walk(b, cut, (x, y), &mut walked, &index, max_steps, MAX_CORNERS.saturating_sub(used))?;
            if area(&corners).abs() < MIN_AREA {
                continue;
            }
            used += corners.len();
            out.push(corners);
            if out.len() > MAX_CONTOURS {
                return Err(TraceError::TooManyContours(out.len()));
            }
        }
    }
    Ok(out)
}

/// Walks the boundary that starts down the left edge of ink pixel `start`, ink on the left, and
/// returns the vertices where it turns. At most `budget` corners.
fn walk(
    b: &Bitmap,
    cut: u8,
    (px, py): (i64, i64),
    walked: &mut Bits,
    index: &impl Fn((i64, i64)) -> usize,
    max_steps: i64,
    budget: usize,
) -> Result<Vec<Point>, TraceError> {
    let start = (px, py + 1, DOWN);
    let (mut x, mut y, mut d) = start;
    let mut corners = Vec::new();
    let mut steps = 0i64;
    loop {
        let (dx, dy) = STEPS.get(d).copied().unwrap_or((0, -1));
        x += dx;
        y += dy;
        steps += 1;
        if d == DOWN {
            walked.set(index((x, y)));
        }
        let (left, right) = ahead((x, y), d);
        // Paper ahead-left: turn left, which also keeps pixels that touch at a corner apart.
        let next = if !b.ink(left, cut) {
            (d + 1) % 4
        } else if b.ink(right, cut) {
            (d + 3) % 4
        } else {
            d
        };
        if next != d {
            corners.push(Point::new(x as f64, y as f64));
            if corners.len() > budget {
                return Err(TraceError::TooDetailed);
            }
        }
        d = next;
        if (x, y, d) == start {
            return Ok(corners);
        }
        if steps > max_steps {
            return Err(TraceError::TooDetailed);
        }
    }
}

/// Signed area of a polygon: positive counter-clockwise (y up).
fn area(pts: &[Point]) -> f64 {
    let n = pts.len();
    let mut twice = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if let Some(q) = pts.get((i + 1) % n) {
            twice += p.x * q.y - q.x * p.y;
        }
    }
    twice / 2.0
}

// ── smoothing and fitting ───────────────────────────────────────────────────────────────────────

/// The midpoints of the first and last unit edge of each straight run between `corners`.
fn midpoints(corners: &[Point]) -> Vec<Point> {
    let n = corners.len();
    let mut out = Vec::with_capacity(2 * n);
    for (i, &a) in corners.iter().enumerate() {
        let Some(&b) = corners.get((i + 1) % n) else { continue };
        let len = (b - a).hypot();
        if len <= 0.0 {
            continue;
        }
        if len <= 1.0 + 1e-9 {
            out.push(a.midpoint(b));
        } else {
            let u = (b - a) / len;
            out.push(a + u * 0.5);
            out.push(b - u * 0.5);
        }
    }
    out
}

/// Where the line through `p0` and `a` meets the line through `b` and `p3`.
fn meet(p0: Point, a: Point, b: Point, p3: Point) -> Option<Point> {
    let (d1, d2) = (a - p0, p3 - b);
    let denom = d1.cross(d2);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = (b - a).cross(d2) / denom;
    let c = a + d1 * t;
    (c.x.is_finite() && c.y.is_finite()).then_some(c)
}

/// `m` with each cut corner put back: a short edge between two long ones, both turns the same
/// way, becomes the point where the long ones' lines meet.
fn sharpen(m: &[Point]) -> Vec<Point> {
    let n = m.len();
    if n < 4 {
        return m.to_vec();
    }
    let at = |i: usize| m.get(i % n).copied().unwrap_or_default();
    let mut replaced: Vec<Option<Point>> = vec![None; n];
    let mut dropped = vec![false; n];
    let touched = |replaced: &[Option<Point>], dropped: &[bool], i: usize| {
        replaced.get(i).is_some_and(Option::is_some) || dropped.get(i).copied().unwrap_or(true)
    };
    for i in 0..n {
        let j = (i + 1) % n;
        if touched(&replaced, &dropped, i) || touched(&replaced, &dropped, j) {
            continue;
        }
        let (p0, a, b, p3) = (at(i + n - 1), at(i), at(i + 1), at(i + 2));
        if (b - a).hypot() > MAX_CHAMFER || (a - p0).hypot() < MIN_CHAMFER_SIDE || (p3 - b).hypot() < MIN_CHAMFER_SIDE {
            continue;
        }
        let (t1, t2) = ((a - p0).cross(b - a), (b - a).cross(p3 - b));
        if t1 * t2 <= 0.0 {
            continue;
        }
        let Some(c) = meet(p0, a, b, p3) else { continue };
        if (c - a.midpoint(b)).hypot() > 1.0 {
            continue;
        }
        if let Some(r) = replaced.get_mut(i) {
            *r = Some(c);
        }
        if let Some(d) = dropped.get_mut(j) {
            *d = true;
        }
    }
    m.iter()
        .enumerate()
        .filter(|(i, _)| !dropped.get(*i).copied().unwrap_or(false))
        .map(|(i, p)| replaced.get(i).copied().flatten().unwrap_or(*p))
        .collect()
}

/// Distance from `p` to the segment from `a` to `b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 <= 0.0 {
        return (p - a).hypot();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).hypot()
}

/// The indices of `pts` (a closed polygon) that Douglas–Peucker keeps at `tolerance`, in order.
/// Every index when the work would exceed its bound.
fn simplify(pts: &[Point], tolerance: f64) -> Vec<usize> {
    let n = pts.len();
    let all = || (0..n).collect::<Vec<usize>>();
    let Some(&first) = pts.first() else { return Vec::new() };
    if n <= 3 {
        return all();
    }
    let far = pts.iter().enumerate().fold((0, 0.0), |best, (i, p)| {
        let d = (*p - first).hypot2();
        if d > best.1 { (i, d) } else { best }
    });
    if far.0 == 0 {
        return all();
    }
    let at = |i: usize| pts.get(i % n).copied().unwrap_or(first);
    let mut keep = vec![false; n];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.get_mut(far.0) {
        *k = true;
    }
    let mut stack = vec![(0usize, far.0), (far.0, n)];
    let mut budget = n.saturating_mul(MAX_SIMPLIFY_WORK_PER_POINT);
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        budget = budget.saturating_sub(b - a);
        if budget == 0 {
            return all();
        }
        let (pa, pb) = (at(a), at(b));
        let (mut worst, mut at_worst) = (0.0, a);
        for i in a + 1..b {
            let d = segment_distance(at(i), pa, pb);
            if d > worst {
                (worst, at_worst) = (d, i);
            }
        }
        if worst > tolerance {
            if let Some(k) = keep.get_mut(at_worst) {
                *k = true;
            }
            stack.push((a, at_worst));
            stack.push((at_worst, b));
        }
    }
    keep.iter().enumerate().filter(|(_, k)| **k).map(|(i, _)| i).collect()
}

/// How far the path turns at `p`, coming from `prev` and going to `next`, in degrees.
fn turn(prev: Point, p: Point, next: Point) -> f64 {
    let (a, b): (Vec2, Vec2) = (p - prev, next - p);
    a.cross(b).atan2(a.dot(b)).abs().to_degrees()
}

/// Appends `fitted`'s segments to `out`, its last one ending exactly at `end`.
fn append(out: &mut BezPath, fitted: &BezPath, end: Point) {
    let mut els: Vec<PathEl> = fitted.elements().iter().copied().filter(|e| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)).collect();
    match els.last_mut() {
        Some(PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p)) => *p = end,
        _ => {
            out.line_to(end);
            return;
        }
    }
    for el in els {
        out.push(el);
    }
}

/// One boundary polygon as a closed outline; `None` when too little is left of it.
fn fit_contour(corners: &[Point], s: &Settings) -> Option<BezPath> {
    let pts = sharpen(&midpoints(corners));
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let at = |i: usize| pts.get(i % n).copied().unwrap_or_default();
    let keys = simplify(&pts, SIMPLIFY_TOLERANCE);
    let k = keys.len();
    let mut is_key = vec![false; n];
    for &i in &keys {
        if let Some(f) = is_key.get_mut(i) {
            *f = true;
        }
    }
    let mut cuts: Vec<usize> = (0..k)
        .filter_map(|j| {
            let (prev, here, next) = (keys.get((j + k - 1) % k)?, keys.get(j)?, keys.get((j + 1) % k)?);
            (turn(at(*prev), at(*here), at(*next)) > s.corner_degrees).then_some(*here)
        })
        .collect();
    if cuts.is_empty() {
        if n <= MAX_FIT_POINTS {
            let fitted = fit_polyline(&pts, s.accuracy, true);
            return (!fitted.is_empty()).then_some(fitted);
        }
        // Too long for one fit: start and end at the first point, as if it were a corner.
        cuts.push(0);
    }
    let first = *cuts.first()?;
    let mut out = BezPath::new();
    out.move_to(at(first));
    let chunk = MAX_FIT_POINTS.saturating_sub(1).max(2);
    for (j, &a) in cuts.iter().enumerate() {
        let b = cuts.get(j + 1).copied().unwrap_or(first + n);
        let straight = (a + 1..b).all(|i| !is_key.get(i % n).copied().unwrap_or(false));
        if straight {
            out.line_to(at(b));
            continue;
        }
        for from in (a..b).step_by(chunk) {
            let to = (from + chunk).min(b);
            let run: Vec<Point> = (from..=to).map(at).collect();
            append(&mut out, &fit_polyline(&run, s.accuracy, false), at(to));
        }
    }
    out.close_path();
    Some(out)
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::Shape;

    use super::*;
    use crate::coverage_difference;

    /// A bitmap where the pixels whose centres `inside` accepts are ink.
    fn raster(width: u32, height: u32, inside: impl Fn(f64, f64) -> bool) -> Bitmap {
        let mut d = Vec::new();
        for row in 0..height {
            let y = f64::from(height - 1 - row) + 0.5;
            for x in 0..width {
                d.push(if inside(f64::from(x) + 0.5, y) { 255 } else { 0 });
            }
        }
        Bitmap::from_darkness(width, height, d).unwrap()
    }

    /// A circle of four cubic arcs, counter-clockwise unless `clockwise`.
    fn circle(path: &mut BezPath, c: Point, r: f64, clockwise: bool) {
        let k = 0.552_284_75 * r;
        let s = if clockwise { -1.0 } else { 1.0 };
        let p = |x: f64, y: f64| Point::new(c.x + x, c.y + s * y);
        path.move_to(p(r, 0.0));
        path.curve_to(p(r, k), p(k, r), p(0.0, r));
        path.curve_to(p(-k, r), p(-r, k), p(-r, 0.0));
        path.curve_to(p(-r, -k), p(-k, -r), p(0.0, -r));
        path.curve_to(p(k, -r), p(r, -k), p(r, 0.0));
        path.close_path();
    }

    fn segments(path: &BezPath) -> usize {
        path.elements().iter().filter(|e| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)).count()
    }

    #[test]
    fn a_traced_o_covers_the_original_within_one_percent() {
        let c = Point::new(500.0, 500.0);
        let bitmap = raster(1000, 1000, |x, y| {
            let d = (Point::new(x, y) - c).hypot();
            d > 200.0 && d <= 400.0
        });
        let traced = trace(&bitmap, &TraceOptions::default()).unwrap();
        assert_eq!(traced.len(), 2, "an outer contour and a counter");
        let areas: Vec<f64> = traced.iter().map(|p| p.area()).collect();
        assert!(areas.iter().any(|a| *a > 0.0) && areas.iter().any(|a| *a < 0.0), "outer counter-clockwise, counter clockwise: {areas:?}");
        assert!(traced.iter().all(|p| segments(p) < 40), "a few curves each, not pixel steps");
        let mut original = BezPath::new();
        circle(&mut original, c, 400.0, false);
        circle(&mut original, c, 200.0, true);
        let diff = coverage_difference(&[original], &traced).unwrap();
        assert!(diff.total <= 0.01 * diff.ink, "{} of {} square pixels differ", diff.total, diff.ink);
    }

    #[test]
    fn a_rectangle_traces_to_four_sharp_corners() {
        let bitmap = raster(60, 40, |x, y| (10.0..50.0).contains(&x) && (5.0..25.0).contains(&y));
        let traced = trace(&bitmap, &TraceOptions::default()).unwrap();
        assert_eq!(traced.len(), 1);
        let path = &traced[0];
        assert!(path.elements().iter().all(|e| !matches!(e, PathEl::CurveTo(..) | PathEl::QuadTo(..))), "{path:?}");
        assert_eq!(segments(path), 4, "{path:?}");
        let mut ends: Vec<(i64, i64)> = path
            .elements()
            .iter()
            .filter_map(|e| match e {
                PathEl::LineTo(p) => Some((p.x.round() as i64, p.y.round() as i64)),
                _ => None,
            })
            .collect();
        ends.sort();
        assert_eq!(ends, [(10, 5), (10, 25), (50, 5), (50, 25)]);
        assert!((path.area() - 800.0).abs() < 1e-6, "{}", path.area());
    }

    #[test]
    fn specks_go_and_pixels_touching_at_a_corner_stay_apart() {
        // A lone pixel at (1, 1); two 3 × 3 squares meeting at (10, 10).
        let bitmap = raster(20, 20, |x, y| {
            let lone = x < 2.0 && x > 1.0 && y < 2.0 && y > 1.0;
            let low = (7.0..10.0).contains(&x) && (7.0..10.0).contains(&y);
            let high = (10.0..13.0).contains(&x) && (10.0..13.0).contains(&y);
            lone || low || high
        });
        let traced = trace(&bitmap, &TraceOptions::default()).unwrap();
        assert_eq!(traced.len(), 2, "{traced:?}");
        assert!(traced.iter().all(|p| (p.area() - 9.0).abs() < 1.0), "{:?}", traced.iter().map(|p| p.area()).collect::<Vec<_>>());
    }

    #[test]
    fn the_threshold_decides_what_is_ink() {
        // Grey 100 of 255 everywhere inside a square.
        let mut d = vec![0u8; 400];
        for row in 5..15 {
            for x in 5..15 {
                d[row * 20 + x] = 100;
            }
        }
        let bitmap = Bitmap::from_darkness(20, 20, d).unwrap();
        assert!(trace(&bitmap, &TraceOptions::default()).unwrap().is_empty());
        let light = TraceOptions { threshold: 0.3, ..TraceOptions::default() };
        assert_eq!(trace(&bitmap, &light).unwrap().len(), 1);
        // White and transparent pixels are paper; opaque black is ink.
        let rgba = [255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 255];
        let b = Bitmap::from_rgba(4, 1, &rgba).unwrap();
        assert_eq!(b.darkness, [0, 0, 255, 201]);
    }

    #[test]
    fn sizes_beyond_64_megapixels_and_bad_options_are_refused() {
        // Refused from the size alone, before the data is looked at.
        assert_eq!(Bitmap::from_darkness(9_000, 8_000, Vec::new()), Err(TraceError::TooLarge { width: 9_000, height: 8_000 }));
        assert_eq!(Bitmap::from_rgba(1 << 20, 1 << 20, &[]), Err(TraceError::TooLarge { width: 1 << 20, height: 1 << 20 }));
        assert_eq!(Bitmap::from_darkness(8_192, 8_192, Vec::new()), Err(TraceError::BadPixels), "64 megapixels exactly are allowed");
        assert_eq!(Bitmap::from_darkness(0, 5, Vec::new()), Err(TraceError::Empty));
        assert_eq!(Bitmap::from_rgba(2, 2, &[0; 15]), Err(TraceError::BadPixels));
        let b = raster(4, 4, |_, _| true);
        for (threshold, smoothness) in [(0.0, 0.5), (1.0, 0.5), (f64::NAN, 0.5), (0.5, -0.1), (0.5, 1.5), (0.5, f64::INFINITY)] {
            assert!(matches!(trace(&b, &TraceOptions { threshold, smoothness }), Err(TraceError::BadOption(_))), "{threshold} {smoothness}");
        }
    }

    #[test]
    fn a_crop_keeps_the_region_counted_from_the_bottom_left() {
        // Ink in the bottom-left quarter only.
        let b = raster(8, 6, |x, y| x < 4.0 && y < 3.0);
        let low = b.cropped(0, 0, 4, 3).unwrap();
        assert!(low.darkness.iter().all(|d| *d == 255));
        let high = b.cropped(4, 3, 100, 100).unwrap();
        assert_eq!((high.width(), high.height()), (4, 3), "clipped to the bitmap");
        assert!(high.darkness.iter().all(|d| *d == 0));
        assert_eq!(b.cropped(8, 0, 4, 4), Err(TraceError::Empty));
    }

    #[test]
    fn a_checkerboard_stays_within_its_limits() {
        // Every other pixel ink: each is a speck, so nothing is kept and nothing is refused.
        let b = raster(300, 300, |x, y| ((x as i64) + (y as i64)) % 2 == 0);
        assert!(trace(&b, &TraceOptions::default()).unwrap().is_empty());
    }
}
