//! The Bitmap Glyph Editor's pixel tools (`[[tool]] bitmapPencil`, `bitmapLine`, `bitmapShift` in
//! `spec/inventory/90-secondary.toml`): which pixel a point falls in, the pixels a stroke passes over, and the
//! `bitmap.setPixels` parameters a stroke ends as.
//!
//! Positions are in strike pixels from the glyph origin, x rightwards and y upwards, as `bitmap.setPixels` counts its
//! `pixels`. The pixel `(x, y)` covers `x..x + 1` across and `y..y + 1` up.
//!
//! | Tool | A press | A drag | Ends as |
//! |---|---|---|---|
//! | Pencil | the pixel under it takes its new state: cleared when it holds the drawing level, else set to it | every pixel passed over takes that state | the pixels passed over, each with that state's level |
//! | Line | as the Pencil | the run from the press to the pointer, set or cleared by the state the first pixel took | the pixels of that run |
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

/// One of the editor's drawing tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BitmapTool {
    #[default]
    Pencil,
    Line,
    Shift,
}

impl BitmapTool {
    pub const ALL: [BitmapTool; 3] = [BitmapTool::Pencil, BitmapTool::Line, BitmapTool::Shift];

    /// The tool's inventory id.
    pub fn id(self) -> &'static str {
        match self {
            BitmapTool::Pencil => "bitmapPencil",
            BitmapTool::Line => "bitmapLine",
            BitmapTool::Shift => "bitmapShift",
        }
    }

    /// The word `view.setBitmapEditor` takes for the tool.
    pub fn word(self) -> &'static str {
        match self {
            BitmapTool::Pencil => "pencil",
            BitmapTool::Line => "line",
            BitmapTool::Shift => "shift",
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
        match self.tool {
            BitmapTool::Pencil | BitmapTool::Line => {
                if self.pixels.is_empty() {
                    return None;
                }
                let pixels: Vec<Value> = self.pixels.iter().map(|&(x, y)| json!({"x": x, "y": y, "value": self.value})).collect();
                Some(json!({"glyph": glyph, "strike": strike, "pixels": pixels}))
            }
            BitmapTool::Shift => {
                let (dx, dy) = self.offset();
                if (dx, dy) == (0, 0) {
                    return None;
                }
                let moved = |o: i32, d: i32| o.saturating_add(d).clamp(-MAX_STRIKE_COORDINATE, MAX_STRIKE_COORDINATE);
                Some(json!({"glyph": glyph, "strike": strike, "origin": [moved(self.origin.0, dx), moved(self.origin.1, dy)]}))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn start(level_under: u8, draw_level: u8) -> StrokeStart {
        StrokeStart { level_under, draw_level, origin: (1, -2) }
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
