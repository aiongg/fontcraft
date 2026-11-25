//! The Bitmap Glyph Editor's pixel tools (`[[tool]] bitmapPencil`, `bitmapLine`, `bitmapShift`, `bitmapRectangle`,
//! `bitmapEllipse` in `spec/inventory/90-secondary.toml`): which pixel a point falls in, the pixels a stroke passes over, and the
//! `bitmap.setPixels` parameters a stroke ends as.
//!
//! Positions are in strike pixels from the glyph origin, x rightwards and y upwards, as `bitmap.setPixels` counts its
//! `pixels`. The pixel `(x, y)` covers `x..x + 1` across and `y..y + 1` up.
//!
//! | Tool | A press | A drag | Ends as |
//! |---|---|---|---|
//! | Pencil | the pixel under it takes its new state: cleared when it holds the drawing level, else set to it | every pixel passed over takes that state | the pixels passed over, each with that state's level |
//! | Line | as the Pencil | the run from the press to the pointer, set or cleared by the state the first pixel took | the pixels of that run |
//! | Rectangle, Ellipse, outlined or filled | as the Pencil | the shape in the box from the press to the pointer, set or cleared by the state the first pixel took | the pixels of that shape |
//! | Shift | grabs the bitmap | moves it by whole pixels | the bitmap's new origin |
//!
//! A stroke is untrusted input: positions are clamped to [`MAX_PIXEL_COORDINATE`], a run between two positions takes
//! at most [`MAX_LINE_STEPS`] pixels and a stroke keeps at most [`MAX_STROKE_PIXELS`].

use std::collections::HashSet;

use fontcraft_model::strike::MAX_STRIKE_COORDINATE;
use serde_json::{Value, json};

/// Largest pixel position either way a stroke takes; further positions are clamped to it. A bitmap is at most 1,024
/// pixels a side, so this leaves room for a bitmap whose origin lies far from the glyph origin.
pub const MAX_PIXEL_COORDINATE: i32 = 4_096;
/// Most pixels one run between two positions holds: the longest run between clamped positions.
pub const MAX_LINE_STEPS: usize = 2 * MAX_PIXEL_COORDINATE as usize + 1;
/// Most distinct pixels one stroke keeps; pixels passed over after that are left as they are.
pub const MAX_STROKE_PIXELS: usize = 65_536;

/// Most pixels a drawn shape is wide or high, so a filled one fits in a stroke; the corner under the pointer stops there.
pub const MAX_SHAPE_SIDE: i32 = 256;

/// One of the editor's drawing tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BitmapTool {
    #[default]
    Pencil,
    Line,
    Shift,
    Rectangle,
    FilledRectangle,
    Ellipse,
    FilledEllipse,
}

impl BitmapTool {
    pub const ALL: [BitmapTool; 7] = [
        BitmapTool::Pencil,
        BitmapTool::Line,
        BitmapTool::Shift,
        BitmapTool::Rectangle,
        BitmapTool::FilledRectangle,
        BitmapTool::Ellipse,
        BitmapTool::FilledEllipse,
    ];

    /// The tool's inventory id.
    pub fn id(self) -> &'static str {
        match self {
            BitmapTool::Pencil => "bitmapPencil",
            BitmapTool::Line => "bitmapLine",
            BitmapTool::Shift => "bitmapShift",
            BitmapTool::Rectangle | BitmapTool::FilledRectangle => "bitmapRectangle",
            BitmapTool::Ellipse | BitmapTool::FilledEllipse => "bitmapEllipse",
        }
    }

    /// The word `view.setBitmapEditor` takes for the tool.
    pub fn word(self) -> &'static str {
        match self {
            BitmapTool::Pencil => "pencil",
            BitmapTool::Line => "line",
            BitmapTool::Shift => "shift",
            BitmapTool::Rectangle => "rectangle",
            BitmapTool::FilledRectangle => "filledRectangle",
            BitmapTool::Ellipse => "ellipse",
            BitmapTool::FilledEllipse => "filledEllipse",
        }
    }

    pub fn from_word(word: &str) -> Option<BitmapTool> {
        BitmapTool::ALL.into_iter().find(|t| t.word() == word)
    }

    /// The undo step a stroke of the tool makes.
    pub fn undo_label(self) -> &'static str {
        match self {
            BitmapTool::Pencil => "Draw Pixels",
            BitmapTool::Line => "Draw Pixel Line",
            BitmapTool::Shift => "Shift Bitmap",
            BitmapTool::Rectangle | BitmapTool::FilledRectangle => "Draw Pixel Rectangle",
            BitmapTool::Ellipse | BitmapTool::FilledEllipse => "Draw Pixel Ellipse",
        }
    }
}

/// The pixel a point lies in; `None` for a point that is not finite. Points far out are clamped to
/// [`MAX_PIXEL_COORDINATE`].
pub fn pixel_at(x: f64, y: f64) -> Option<(i32, i32)> {
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let limit = f64::from(MAX_PIXEL_COORDINATE);
    let cell = |v: f64| v.floor().clamp(-limit, limit) as i32;
    Some((cell(x), cell(y)))
}

/// The pixels of a straight run from `a` to `b`, both included, one per step along the longer axis. Both ends are
/// clamped to [`MAX_PIXEL_COORDINATE`] first, so a run holds at most [`MAX_LINE_STEPS`] pixels.
pub fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let clamp = |(x, y): (i32, i32)| (x.clamp(-MAX_PIXEL_COORDINATE, MAX_PIXEL_COORDINATE), y.clamp(-MAX_PIXEL_COORDINATE, MAX_PIXEL_COORDINATE));
    let ((x0, y0), (x1, y1)) = (clamp(a), clamp(b));
    let (dx, dy) = (i64::from(x1) - i64::from(x0), i64::from(y1) - i64::from(y0));
    let steps = dx.abs().max(dy.abs());
    let count = usize::try_from(steps).unwrap_or(0).saturating_add(1).min(MAX_LINE_STEPS);
    let mut out = Vec::with_capacity(count);
    if steps == 0 {
        out.push((x0, y0));
        return out;
    }
    for i in 0..count as i64 {
        // Rounded to the nearest pixel centre, half away from zero, so a run reads the same from either end.
        let step = |from: i32, d: i64| {
            let num = d * i;
            let q = (2 * num + steps * num.signum()) / (2 * steps);
            i64::from(from) + q
        };
        let (x, y) = (step(x0, dx), step(y0, dy));
        out.push((i32::try_from(x).unwrap_or(x0), i32::try_from(y).unwrap_or(y0)));
    }
    out
}

/// The pixels of a shape of `tool` in the box from `a` to `b`, both corners included, rows from the bottom; none for a
/// tool that draws no shape. The corner `b` is drawn at most [`MAX_SHAPE_SIDE`] pixels from `a`, so a shape holds at most
/// [`MAX_STROKE_PIXELS`].
pub fn shape(tool: BitmapTool, a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (ellipse, filled) = match tool {
        BitmapTool::Rectangle => (false, false),
        BitmapTool::FilledRectangle => (false, true),
        BitmapTool::Ellipse => (true, false),
        BitmapTool::FilledEllipse => (true, true),
        BitmapTool::Pencil | BitmapTool::Line | BitmapTool::Shift => return Vec::new(),
    };
    let reach = MAX_SHAPE_SIDE - 1;
    let side = |a: i32, b: i32| {
        let a = a.clamp(-MAX_PIXEL_COORDINATE, MAX_PIXEL_COORDINATE);
        let b = b.clamp(a - reach, a + reach);
        (a.min(b), a.max(b))
    };
    let ((x0, x1), (y0, y1)) = (side(a.0, b.0), side(a.1, b.1));
    // The run of pixels the filled shape has in the row `y`.
    let row = |y: i32| -> Option<(i32, i32)> {
        if y < y0 || y > y1 {
            return None;
        }
        if !ellipse {
            return Some((x0, x1));
        }
        let (cx, rx) = (f64::from(x0 + x1 + 1) / 2.0, f64::from(x1 - x0 + 1) / 2.0);
        let (cy, ry) = (f64::from(y0 + y1 + 1) / 2.0, f64::from(y1 - y0 + 1) / 2.0);
        let up = (f64::from(y) + 0.5 - cy) / ry;
        let half = rx * (1.0 - up * up).max(0.0).sqrt();
        // The pixels whose centres lie in the ellipse, and always the middle ones, so it reaches its box.
        let first = ((cx - half - 0.5).ceil()).min((cx - 0.5).floor()) as i32;
        let last = ((cx + half - 0.5).floor()).max((cx - 0.5).ceil()) as i32;
        Some((first.max(x0), last.min(x1)))
    };
    let mut out = Vec::new();
    for y in y0..=y1 {
        let Some((first, last)) = row(y) else { continue };
        let inside = |x: i32| {
            let holds = |y: i32| row(y).is_some_and(|(f, l)| (f..=l).contains(&x));
            x > first && x < last && holds(y - 1) && holds(y + 1)
        };
        out.extend((first..=last).filter(|x| filled || !inside(*x)).map(|x| (x, y)));
    }
    out
}

/// What the bitmap holds where a stroke starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StrokeStart {
    /// The level of the pixel under the press; 0 outside the bitmap.
    pub level_under: u8,
    /// The level the tools draw with: the strike's top level, or the shade chosen in a grey strike.
    pub draw_level: u8,
    /// The bitmap's bottom-left corner in pixels from the glyph origin.
    pub origin: (i32, i32),
}

/// A stroke in progress: from the press to the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapStroke {
    tool: BitmapTool,
    start: (i32, i32),
    last: (i32, i32),
    /// The level every pixel of the stroke takes.
    value: u8,
    origin: (i32, i32),
    pixels: Vec<(i32, i32)>,
    seen: HashSet<(i32, i32)>,
}

impl BitmapStroke {
    /// A stroke of `tool` pressed on pixel `at`. The first pixel decides the state the stroke draws: a pixel that
    /// holds the drawing level is cleared, any other takes the drawing level.
    pub fn begin(tool: BitmapTool, at: (i32, i32), start: StrokeStart) -> BitmapStroke {
        let value = if start.level_under == start.draw_level { 0 } else { start.draw_level };
        let mut stroke = BitmapStroke { tool, start: at, last: at, value, origin: start.origin, pixels: Vec::new(), seen: HashSet::new() };
        if tool != BitmapTool::Shift {
            stroke.add(&line(at, at));
        }
        stroke
    }

    pub fn tool(&self) -> BitmapTool {
        self.tool
    }

    /// The level the stroke draws with: 0 when it clears.
    pub fn value(&self) -> u8 {
        self.value
    }

    /// The pixels the stroke sets or clears, in the order it reached them.
    pub fn pixels(&self) -> &[(i32, i32)] {
        &self.pixels
    }

    /// The bitmap's origin when the stroke began.
    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    /// Whole pixels the Shift tool has moved the bitmap by.
    pub fn offset(&self) -> (i32, i32) {
        (self.last.0.saturating_sub(self.start.0), self.last.1.saturating_sub(self.start.1))
    }

    /// The pointer reached pixel `at`.
    pub fn extend(&mut self, at: (i32, i32)) {
        match self.tool {
            BitmapTool::Pencil => {
                // Every pixel between the last position and this one, so a fast drag leaves no gaps.
                let run = line(self.last, at);
                self.add(&run);
            }
            BitmapTool::Line => {
                self.pixels.clear();
                self.seen.clear();
                let run = line(self.start, at);
                self.add(&run);
            }
            BitmapTool::Rectangle | BitmapTool::FilledRectangle | BitmapTool::Ellipse | BitmapTool::FilledEllipse => {
                self.pixels.clear();
                self.seen.clear();
                let drawn = shape(self.tool, self.start, at);
                self.add(&drawn);
            }
            BitmapTool::Shift => {}
        }
        self.last = at;
    }

    fn add(&mut self, run: &[(i32, i32)]) {
        for &p in run {
            if self.pixels.len() >= MAX_STROKE_PIXELS {
                return;
            }
            if self.seen.insert(p) {
                self.pixels.push(p);
            }
        }
    }

    /// The `bitmap.setPixels` parameters of the stroke so far on `glyph` in `strike` (`{ppem, depth}`); `None` when
    /// it changes nothing (a shift back to where it started).
    pub fn params(&self, glyph: &str, strike: Value) -> Option<Value> {
        if self.tool == BitmapTool::Shift {
            let (dx, dy) = self.offset();
            if (dx, dy) == (0, 0) {
                return None;
            }
            let moved = |o: i32, d: i32| o.saturating_add(d).clamp(-MAX_STRIKE_COORDINATE, MAX_STRIKE_COORDINATE);
            return Some(json!({"glyph": glyph, "strike": strike, "origin": [moved(self.origin.0, dx), moved(self.origin.1, dy)]}));
        }
        if self.pixels.is_empty() {
            return None;
        }
        let pixels: Vec<Value> = self.pixels.iter().map(|&(x, y)| json!({"x": x, "y": y, "value": self.value})).collect();
        Some(json!({"glyph": glyph, "strike": strike, "pixels": pixels}))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn start(level_under: u8, draw_level: u8) -> StrokeStart {
        StrokeStart { level_under, draw_level, origin: (1, -2) }
    }

    /// A rectangle is its border or its box whichever corner it starts from; an ellipse reaches its box on every side,
    /// is symmetric, and its outline is the border of the filled one; a shape drawn far stops at the limit.
    #[test]
    fn shapes_fill_or_border_their_box() {
        let of = |tool, a, b| {
            let mut p = shape(tool, a, b);
            p.sort_unstable();
            p
        };
        let border = of(BitmapTool::Rectangle, (3, 2), (0, 0));
        assert_eq!(border.len(), 10);
        assert!(!border.contains(&(1, 1)) && !border.contains(&(2, 1)));
        assert_eq!(of(BitmapTool::FilledRectangle, (0, 0), (3, 2)).len(), 12);
        assert_eq!(of(BitmapTool::Ellipse, (4, 4), (4, 4)), vec![(4, 4)]);
        for (w, h) in [(5, 5), (8, 3), (2, 10), (1, 4), (12, 12)] {
            let filled = of(BitmapTool::FilledEllipse, (0, 0), (w - 1, h - 1));
            assert!(filled.iter().any(|p| p.0 == 0) && filled.iter().any(|p| p.0 == w - 1), "{w}×{h}: {filled:?}");
            assert!((0..h).all(|y| filled.iter().any(|p| p.1 == y)), "{w}×{h}: every row");
            assert!(filled.iter().all(|&(x, y)| filled.contains(&(w - 1 - x, y)) && filled.contains(&(x, h - 1 - y))), "{w}×{h}: {filled:?}");
            let outline = of(BitmapTool::Ellipse, (0, 0), (w - 1, h - 1));
            assert!(outline.iter().all(|p| filled.contains(p)));
            let hidden = |&(x, y): &(i32, i32)| [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().all(|p| filled.contains(p));
            assert!(filled.iter().all(|p| outline.contains(p) != hidden(p)), "{w}×{h}: {outline:?}");
        }
        assert_eq!(of(BitmapTool::FilledRectangle, (0, 0), (i32::MAX, i32::MIN)).len(), MAX_STROKE_PIXELS);
        let mut stroke = BitmapStroke::begin(BitmapTool::FilledEllipse, (0, 0), start(0, 1));
        stroke.extend((9, 9));
        stroke.extend((1, 1));
        assert_eq!(stroke.pixels().len(), 4, "the last box counts");
    }

    /// A point falls in the pixel whose corner is its floor; points far out are clamped and non-finite ones refused.
    #[test]
    fn a_point_falls_in_the_pixel_below_and_left_of_it() {
        assert_eq!(pixel_at(0.2, 0.9), Some((0, 0)));
        assert_eq!(pixel_at(-0.1, 3.0), Some((-1, 3)));
        assert_eq!(pixel_at(1e12, -1e12), Some((MAX_PIXEL_COORDINATE, -MAX_PIXEL_COORDINATE)));
        assert_eq!(pixel_at(f64::NAN, 0.0), None);
    }

    /// A run steps one pixel at a time along its longer axis and is the same pixels read from either end.
    #[test]
    fn a_run_has_one_pixel_per_step_and_no_direction() {
        assert_eq!(line((0, 0), (3, 1)), vec![(0, 0), (1, 0), (2, 1), (3, 1)]);
        let mut back = line((3, 1), (0, 0));
        back.reverse();
        assert_eq!(back, line((0, 0), (3, 1)));
        assert_eq!(line((2, 2), (2, 2)), vec![(2, 2)]);
        let far = line((-1_000_000, 0), (1_000_000, 0));
        assert_eq!(far.len(), MAX_LINE_STEPS);
    }

    /// The Pencil sets the pixels passed over, without gaps, when the first one was clear, and clears them when it held
    /// the drawing level; each pixel is listed once.
    #[test]
    fn the_pencil_draws_what_the_first_pixel_became() {
        let mut stroke = BitmapStroke::begin(BitmapTool::Pencil, (0, 0), start(0, 1));
        stroke.extend((3, 0));
        stroke.extend((1, 0));
        assert_eq!(stroke.pixels(), &[(0, 0), (1, 0), (2, 0), (3, 0)]);
        let p = stroke.params("A", json!({"ppem": 12})).unwrap();
        assert_eq!(p["pixels"][3], json!({"x": 3, "y": 0, "value": 1}));
        assert_eq!(p["strike"], json!({"ppem": 12}));

        let clearing = BitmapStroke::begin(BitmapTool::Pencil, (0, 0), start(1, 1));
        assert_eq!(clearing.value(), 0);
        // In a grey strike a pixel of another level takes the drawing level.
        assert_eq!(BitmapStroke::begin(BitmapTool::Pencil, (0, 0), start(5, 9)).value(), 9);
    }

    /// The Line keeps only the run from the press to the pointer.
    #[test]
    fn the_line_follows_the_pointer_from_the_press() {
        let mut stroke = BitmapStroke::begin(BitmapTool::Line, (0, 0), start(0, 1));
        stroke.extend((0, 4));
        stroke.extend((2, 0));
        assert_eq!(stroke.pixels(), &[(0, 0), (1, 0), (2, 0)]);
    }

    /// The Shift moves the origin by whole pixels and changes nothing when the pointer comes back.
    #[test]
    fn the_shift_moves_the_origin() {
        let mut stroke = BitmapStroke::begin(BitmapTool::Shift, (4, 4), start(0, 1));
        assert!(stroke.pixels().is_empty());
        stroke.extend((6, 3));
        assert_eq!(stroke.params("A", json!({"ppem": 12})).unwrap()["origin"], json!([3, -3]));
        stroke.extend((4, 4));
        assert_eq!(stroke.params("A", json!({"ppem": 12})), None);
    }

    /// A long scribble keeps at most the stroke's pixel limit.
    #[test]
    fn a_stroke_keeps_at_most_its_pixel_limit() {
        let mut stroke = BitmapStroke::begin(BitmapTool::Pencil, (0, 0), start(0, 1));
        for row in 0..40 {
            stroke.extend((MAX_PIXEL_COORDINATE, row));
            stroke.extend((-MAX_PIXEL_COORDINATE, row));
        }
        assert_eq!(stroke.pixels().len(), MAX_STROKE_PIXELS);
    }
}
