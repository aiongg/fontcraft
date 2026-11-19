//! The Hinting Preview's view model: a hinted waterfall of the shown glyph from 8 to 80 PPM.
//!
//! Each row is the glyph at one pixel size in the first master, built as TrueType in memory and grid-fitted by
//! `hint.tt.renderPreview`, then filled one image pixel per device pixel, black ink on white paper whatever the theme,
//! as the device would show it. The scale (`hint.tt.setPreviewScale`) only changes how large those pixels are drawn.
//! The rows are built only while the panel shows.
//!
//! Rows: `preview.ttHintingWaterfall`, `hint.tt.setPreviewScale`.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::cmd::preview::preview_range;
use serde_json::{Value, json};

use super::ProjectCx;
use crate::shell::dock::PanelId;

/// The shell command that sets the scale.
pub const SCALE_COMMAND: &str = "hint.tt.setPreviewScale";
/// The smallest and largest pixel size of the waterfall.
pub const FIRST_PPEM: u16 = 8;
pub const LAST_PPEM: u16 = 80;
/// The scales the pixels can be drawn at.
pub const SCALES: [u8; 3] = [1, 2, 4];
/// Widest and tallest row image, in device pixels; a longer row is cut.
pub const MAX_ROW_PIXELS: f64 = 4096.0;
/// Most points read back for one row.
const MAX_ROW_POINTS: usize = 1_000_000;
/// Samples per pixel row: each pixel's coverage is the mean of this many horizontal scan lines.
const SAMPLES: usize = 4;
/// Line segments a curve is flattened into.
const CURVE_STEPS: usize = 8;

/// One row: the glyph at one size, `width` × `height` pixels of premultiplied RGBA.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct WaterfallRow {
    pub ppem: u16,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
    /// Why the glyph was drawn unhinted at this size, when it was.
    pub warning: Option<String>,
}

/// What the Hinting Preview shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Waterfall {
    pub glyph: Option<String>,
    /// Screen pixels per device pixel; see [`Waterfall::scale`].
    pub scale: u8,
    pub rows: Vec<WaterfallRow>,
    /// Why the rows cannot be drawn.
    pub message: Option<String>,
    /// Changes whenever the rows do: the key of their textures.
    pub key: u64,
}

impl Waterfall {
    /// The scale, or 1 when the stored one is not one of [`SCALES`].
    pub fn scale(&self) -> u8 {
        if SCALES.contains(&self.scale) { self.scale } else { 1 }
    }
}

/// Runs `hint.tt.setPreviewScale {scale}`.
pub fn set_scale(model: &mut Tracked<Waterfall>, params: &Value) -> Result<(), String> {
    let n = params.get("scale").and_then(Value::as_f64).ok_or("give `scale`: 1, 2 or 4")?;
    let scale = SCALES.iter().copied().find(|s| f64::from(*s) == n).ok_or_else(|| format!("`scale` must be 1, 2 or 4, got {n}"))?;
    if model.scale != scale {
        model.edit(|m| m.scale = scale);
    }
    Ok(())
}

pub fn project(model: &mut Tracked<Waterfall>, cx: &mut ProjectCx<'_>) {
    let scale = model.scale;
    let shows = cx.view.workspace.layout.panel_state(&PanelId::HintingWaterfall) != PanelState::Hidden;
    let glyph = super::glyph::shown_glyph(cx);
    let (Some(glyph), true) = (glyph, shows) else {
        super::set(model, Waterfall { scale, ..Waterfall::default() });
        return;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (cx.session.active().map(|d| (d.id, d.revision)), &glyph).hash(&mut hasher);
    let key = hasher.finish();
    let (rows, message) = match render(cx, &glyph) {
        Ok(rows) => (rows, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    super::set(model, Waterfall { glyph: Some(glyph), scale, rows, message, key });
}

/// Renders the rows of `glyph` in the first master.
fn render(cx: &mut ProjectCx<'_>, glyph: &str) -> Result<Vec<WaterfallRow>, String> {
    let font = cx.session.font().map_err(|e| e.to_string())?;
    let master = font.masters.first().ok_or("the font has no master")?;
    let (range, upm, instance) = (preview_range(font, master), f64::from(font.units_per_em), master.id.clone());
    if !(upm.is_finite() && upm > 0.0) {
        return Err("the font's units per em are not usable".into());
    }
    let sizes: Vec<u16> = (FIRST_PPEM..=LAST_PPEM).collect();
    let params = json!({"instance": instance, "glyphs": [glyph], "ppem": sizes, "intent": "grayscale", "output": "outline"});
    let reply = cx.session.query("hint.tt.renderPreview", &params).map_err(|e| e.to_string())?;
    let found = reply.get("sizes").and_then(Value::as_array).ok_or("the hinted preview returned no sizes")?;
    found.iter().map(|size| row(size, range, upm)).collect()
}

/// One size of a `hint.tt.renderPreview` reply as a row.
fn row(size: &Value, (descender, ascender): (f64, f64), upm: f64) -> Result<WaterfallRow, String> {
    let ppem = size.get("ppem").and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()).ok_or("a size without `ppem`")?;
    let glyphs = size.get("glyphs").and_then(Value::as_array).ok_or("a size without glyphs")?;
    let mut line = Line::default();
    let mut warning = None;
    for g in glyphs {
        if warning.is_none() && g.get("hinted").and_then(Value::as_bool) == Some(false) {
            let why = g.get("message").and_then(Value::as_str).unwrap_or("drawn unhinted");
            warning = Some(why.to_string());
        }
        for contour in g.get("contours").and_then(Value::as_array).into_iter().flatten() {
            if let Some(points) = contour.as_array() {
                line.contour(points)?;
            }
        }
        line.x += g.get("advance").and_then(Value::as_f64).filter(|a| a.is_finite()).unwrap_or(0.0).max(0.0);
    }
    let scale = f64::from(ppem) / upm;
    let mut top = (ascender * scale).ceil().max(1.0);
    let mut bottom = (descender * scale).floor().min(0.0);
    let (mut left, mut right) = (0.0_f64, line.x.ceil().max(1.0));
    if let Some([x0, y0, x1, y1]) = line.bounds {
        (left, right) = (left.min(x0.floor()), right.max(x1.ceil()));
        (bottom, top) = (bottom.min(y0.floor()), top.max(y1.ceil()));
    }
    let right = right.min(left + MAX_ROW_PIXELS);
    let bottom = bottom.max(top - MAX_ROW_PIXELS);
    if ![left, right, bottom, top].iter().all(|v| v.is_finite()) {
        return Err(format!("the row at {ppem} PPM has no finite size"));
    }
    let (width, height) = ((right - left).max(1.0) as u32, (top - bottom).max(1.0) as u32);
    let coverage = fill(&line.edges, left, top, width, height);
    let mut pixels = Vec::with_capacity(coverage.len().saturating_mul(4));
    for c in coverage {
        let paper = (255.0 * (1.0 - c.clamp(0.0, 1.0))).round() as u8;
        pixels.extend_from_slice(&[paper, paper, paper, 255]);
    }
    Ok(WaterfallRow { ppem, width, height, pixels: Arc::new(pixels), warning })
}

/// A row's outlines in pixels, y up, from the row's origin, as line segments.
#[derive(Default)]
struct Line {
    edges: Vec<[(f64, f64); 2]>,
    /// The pen position: the advances so far.
    x: f64,
    points: usize,
    bounds: Option<[f64; 4]>,
}

impl Line {
    /// Adds one contour of `[x, y, onCurve]` points at the pen position. One off-curve point between on-curve points
    /// is a quadratic segment, two are a cubic one.
    fn contour(&mut self, points: &[Value]) -> Result<(), String> {
        self.points = self.points.saturating_add(points.len());
        if self.points > MAX_ROW_POINTS {
            return Err(format!("a row has more than {MAX_ROW_POINTS} points"));
        }
        let mut read = points.iter().filter_map(|p| {
            let p = p.as_array()?;
            let (x, y) = (p.first()?.as_f64()?, p.get(1)?.as_f64()?);
            let on = p.get(2).and_then(Value::as_bool).unwrap_or(true);
            (x.is_finite() && y.is_finite()).then_some(((x + self.x, y), on))
        });
        let Some((start, true)) = read.next() else { return Ok(()) };
        let mut polyline = vec![start];
        let mut off: Vec<(f64, f64)> = Vec::with_capacity(2);
        let push = |polyline: &mut Vec<(f64, f64)>, off: &[(f64, f64)], to: (f64, f64)| {
            let from = polyline.last().copied().unwrap_or(to);
            match off {
                [a] => polyline.extend((1..=CURVE_STEPS).map(|i| quad(from, *a, to, i as f64 / CURVE_STEPS as f64))),
                [a, b] => polyline.extend((1..=CURVE_STEPS).map(|i| cubic(from, *a, *b, to, i as f64 / CURVE_STEPS as f64))),
                _ => polyline.push(to),
            }
        };
        for (p, on) in read {
            if !on {
                if off.len() == 2 {
                    return Ok(());
                }
                off.push(p);
                continue;
            }
            push(&mut polyline, &off, p);
            off.clear();
        }
        push(&mut polyline, &off, start);
        for pair in polyline.windows(2) {
            if let [a, b] = pair {
                self.edges.push([*a, *b]);
            }
        }
        for (x, y) in polyline {
            let [x0, y0, x1, y1] = self.bounds.unwrap_or([x, y, x, y]);
            self.bounds = Some([x0.min(x), y0.min(y), x1.max(x), y1.max(y)]);
        }
        Ok(())
    }
}

fn quad(a: (f64, f64), b: (f64, f64), c: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    (u * u * a.0 + 2.0 * u * t * b.0 + t * t * c.0, u * u * a.1 + 2.0 * u * t * b.1 + t * t * c.1)
}

fn cubic(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (w0 * a.0 + w1 * b.0 + w2 * c.0 + w3 * d.0, w0 * a.1 + w1 * b.1 + w2 * c.1 + w3 * d.1)
}

/// The ink coverage of each pixel of a `width` × `height` image whose top-left corner is at (`left`, `top`) in pixel
/// space (y up), under the non-zero winding rule.
fn fill(edges: &[[(f64, f64); 2]], left: f64, top: f64, width: u32, height: u32) -> Vec<f32> {
    let (w, h) = (width as usize, height as usize);
    let mut coverage = vec![0.0_f32; w.saturating_mul(h)];
    let share = 1.0 / SAMPLES as f32;
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    for row in 0..h {
        for sample in 0..SAMPLES {
            let y = top - (row as f64 + (sample as f64 + 0.5) / SAMPLES as f64);
            crossings.clear();
            for [(x0, y0), (x1, y1)] in edges {
                if (*y0 <= y) == (*y1 <= y) {
                    continue;
                }
                let x = x0 + (y - y0) * (x1 - x0) / (y1 - y0);
                crossings.push((x - left, if y1 > y0 { 1 } else { -1 }));
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for pair in crossings.windows(2) {
                let [(xa, dir), (xb, _)] = pair else { continue };
                winding += dir;
                if winding != 0 {
                    span(&mut coverage, row * w, w, *xa, *xb, share);
                }
            }
        }
    }
    coverage
}

/// Adds `share` times the covered part of each pixel of the row starting at `at` between `xa` and `xb`.
fn span(coverage: &mut [f32], at: usize, width: usize, xa: f64, xb: f64, share: f32) {
    let (xa, xb) = (xa.clamp(0.0, width as f64), xb.clamp(0.0, width as f64));
    if xb <= xa {
        return;
    }
    let (first, last) = (xa.floor() as usize, (xb.ceil() as usize).min(width));
    for px in first..last {
        let covered = (xb.min(px as f64 + 1.0) - xa.max(px as f64)).max(0.0) as f32;
        if let Some(c) = coverage.get_mut(at.saturating_add(px)) {
            *c += covered * share;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 × 3 pixel rectangle from (1, 0) fills its pixels fully and leaves the rest as paper.
    #[test]
    fn a_rectangle_fills_its_pixels() {
        let size = json!({"ppem": 10, "glyphs": [{"glyph": "I", "hinted": true, "advance": 4.0,
            "contours": [[[1, 0, true], [3, 0, true], [3, 3, true], [1, 3, true]]]}]});
        let row = row(&size, (0.0, 300.0), 1000.0).expect("a row");
        assert_eq!((row.width, row.height), (4, 3));
        let ink: Vec<bool> = row.pixels.chunks(4).map(|p| p.first() == Some(&0)).collect();
        assert_eq!(ink, [false, true, true, false].repeat(3));
        assert!(row.pixels.chunks(4).all(|p| matches!(p, [0, 0, 0, 255] | [255, 255, 255, 255])));
    }
}
