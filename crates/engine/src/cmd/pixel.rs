//! The Pixel tool's commands (M6.13): Add Pixel Glyph and Set Pixels.
//!
//! Rows: `spec/inventory/09-reusing-shapes.toml` (`pixel`, `pixel.*`, `dialog.addPixelGlyph`).
//!
//! A pixel is a component of the glyph named `pixel` placed on the font's grid: cell
//! `[column, row]` puts the component's offset at `(column × spacing, row × spacing)`, where
//! spacing is the font's Grid Spacing. A pixel is found again by that offset alone. Pixels are
//! written with automatic alignment off, so the re-alignment after each command leaves them
//! where they were painted.
//!
//! `pixel.setPixels` is also what the Pixel tool runs: the tool sends the pointer positions of a
//! click or drag as `stroke` with mode `paint`, and the command finds the cells they cross.
//! When the font has no glyph named `pixel`, the command changes nothing and says so in its
//! result (`pixelGlyphMissing`), which is what makes the shell offer Add Pixel Glyph.

use std::collections::{HashMap, HashSet};

use fontcraft_components::{ComponentIndex, MAX_LAYER_COMPONENTS};
use fontcraft_model::{Alignment, Component, Layer, MAX_COORDINATE, Path, Shape};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind as K, Targets, kind};

/// The glyph every pixel is a component of.
pub const PIXEL_GLYPH: &str = "pixel";
/// Smallest grid spacing the Pixel tool paints with.
pub const MIN_PIXEL_GRID: f64 = 2.0;
/// Most cells one `pixel.setPixels` may name, or a stroke may cross.
pub const MAX_PIXEL_CELLS: usize = 65_536;
/// Most pointer positions one stroke may hold.
pub const MAX_STROKE_POINTS: usize = 65_536;
/// Largest side of the pixel glyph's square, in font units (the largest grid spacing).
const MAX_PIXEL_SIZE: f64 = 10_000.0;
/// How far a component's offset may lie from a cell's corner and still be that cell's pixel.
const OFFSET_EPSILON: f64 = 1e-6;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "pixel.addPixelGlyph",
            "Add Pixel Glyph",
            [],
            None,
            "{size?: number (default the font's grid spacing), above 0 up to 10000} adds the glyph named pixel, which the Pixel tool paints with: on every master a square of that side with its lower left corner at the origin, as wide as the square. Refused when the font already has a glyph named pixel → {name, size, index}",
            has_doc,
            add_pixel_glyph
        ),
        cmd!(
            "pixel.setPixels",
            "Set Pixels",
            [],
            None,
            "{glyph: name, layer?: id (default the first master's layer), cells: [[column, row]] | stroke: [[x, y]] (pointer positions in font units; the cells are those the straight pieces between them cross), mode?: add | remove | toggle | paint = paint} paints with the Pixel tool: a pixel is a component of the glyph named pixel at (column × grid spacing, row × grid spacing), found again by that offset. add fills empty cells, remove empties filled ones, toggle flips each cell, paint removes from every cell when the first cell holds a pixel and adds to every cell otherwise (a click or drag of the tool). Repeated cells count once; at most 65,536 cells. Refused when the grid spacing is below 2. When the font has no glyph named pixel nothing changes and the result has pixelGlyphMissing: true → {glyph, layer, mode, added: [[column, row]], removed: [[column, row]], pixelGlyphMissing?, selection}",
            has_doc,
            set_pixels
        )
        .targets(&[K::Layer]),
    ]
}

// ── Add Pixel Glyph ─────────────────────────────────────────────────────────────────────────────

fn add_pixel_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("pixel.addPixelGlyph", v)?;
    let font = s.doc()?.font.clone();
    if font.glyph(PIXEL_GLYPH).is_some() {
        return Err(p.bad("the font already has a glyph named pixel"));
    }
    let size = match p.opt_f64("size")? {
        Some(size) => size,
        None => {
            let spacing = font.info.settings.grid_length.unwrap_or(1.0);
            if !(spacing.is_finite() && spacing > 0.0) {
                return Err(p.bad(format!("the font's grid spacing is {spacing}: give `size`")));
            }
            spacing
        }
    };
    if !(size > 0.0 && size <= MAX_PIXEL_SIZE) {
        return Err(p.bad(format!("`size` must be above 0 and at most {MAX_PIXEL_SIZE}, got {size}")));
    }
    let data = s.glyph_data()?;
    let index = s.edit("Add Pixel Glyph", |f| {
        let index = fontcraft_model::insertion_index(f, &data, &fontcraft_model::Glyph::new(PIXEL_GLYPH));
        f.add_glyph_at(index, PIXEL_GLYPH, Vec::new(), size)?;
        let mut g = f.glyph_mut(PIXEL_GLYPH)?;
        for layer in g.layers.iter_mut() {
            layer.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, size, size))];
        }
        Ok(index)
    })?;
    Ok(json!({ "name": PIXEL_GLYPH, "size": size, "index": index }))
}

// ── Set Pixels ──────────────────────────────────────────────────────────────────────────────────

/// What `pixel.setPixels` does to each cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Add,
    Remove,
    Toggle,
    /// Remove when the first cell holds a pixel, add otherwise.
    Paint,
}

impl Mode {
    fn parse(p: &Params) -> Result<Self> {
        match p.opt_str("mode")?.unwrap_or("paint") {
            "add" => Ok(Mode::Add),
            "remove" => Ok(Mode::Remove),
            "toggle" => Ok(Mode::Toggle),
            "paint" => Ok(Mode::Paint),
            other => Err(p.bad(format!("`mode` must be add, remove, toggle or paint, got \"{other}\""))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Add => "add",
            Mode::Remove => "remove",
            Mode::Toggle => "toggle",
            Mode::Paint => "paint",
        }
    }
}

type Cell = (i64, i64);

/// The cells changed on the layer, and the mode that changed them.
struct Painted {
    mode: Mode,
    added: Vec<Cell>,
    removed: Vec<Cell>,
}

fn set_pixels(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("pixel.setPixels", v)?;
    let t = Targets::new(p.command(), p.str("glyph")?, p.opt_str("layer")?.map(str::to_string));
    if let Some(side) = p.opt_str("side")?
        && side != "foreground"
    {
        return Err(p.bad("the Pixel tool paints on the foreground of a layer only"));
    }
    let mode = Mode::parse(&p)?;
    let font = s.doc()?.font.clone();
    let spacing = font.info.settings.grid_length.unwrap_or(1.0);
    if !(spacing.is_finite() && spacing >= MIN_PIXEL_GRID) {
        return Err(
            p.bad(format!("the Pixel tool needs a grid spacing of 2 or more (Font Info › Other Settings › Grid Spacing); this font's is {spacing}"))
        );
    }
    let cells = cells(&p, spacing)?;
    let layer = t.resolve(&font)?.layer.id.clone();
    if font.glyph(PIXEL_GLYPH).is_none() {
        return Ok(json!({
            "glyph": t.glyph, "layer": layer, "mode": mode.name(), "added": [], "removed": [], "pixelGlyphMissing": true,
        }));
    }
    if mode != Mode::Remove && (t.glyph == PIXEL_GLYPH || ComponentIndex::new(&font).users(&t.glyph).iter().any(|u| u == PIXEL_GLYPH)) {
        return Err(p.bad(format!("a pixel in `{}` would make the glyph pixel contain itself", t.glyph)));
    }
    let painted = s.edit("Set Pixels", |f| t.edit_layer(f, |l| paint(&p, l, &cells, mode, spacing)))?;
    let list = |cells: &[Cell]| Value::Array(cells.iter().map(|(c, r)| json!([c, r])).collect());
    // Removing shapes moves the indices after them: the selection starts empty.
    let selection = Targets::new(p.command(), t.glyph.clone(), Some(layer.clone())).to_json();
    Ok(json!({
        "glyph": t.glyph,
        "layer": layer,
        "mode": painted.mode.name(),
        "added": list(&painted.added),
        "removed": list(&painted.removed),
        "selection": selection,
    }))
}

/// The cells `cells` or `stroke` names, each once, in the order given or crossed.
fn cells(p: &Params, spacing: f64) -> Result<Vec<Cell>> {
    let mut out = match (p.opt_array("cells")?, p.opt_array("stroke")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `cells` or `stroke`, not both")),
        (None, None) => return Err(p.bad("`cells` or `stroke` is required")),
        (Some(list), None) => {
            if list.len() > MAX_PIXEL_CELLS {
                return Err(p.bad(format!("`cells` holds {} cells; at most {MAX_PIXEL_CELLS}", list.len())));
            }
            // A cell whose corner would lie outside the coordinate range is refused.
            let limit = (MAX_COORDINATE / spacing).floor();
            list.iter()
                .map(|v| {
                    let (c, r) = pair(p, "cells", v)?;
                    if c.fract() != 0.0 || r.fract() != 0.0 || c.abs() > limit || r.abs() > limit {
                        return Err(p.bad(format!("`cells`: [{c}, {r}] is not a pair of whole numbers within ±{limit}")));
                    }
                    Ok((c as i64, r as i64))
                })
                .collect::<Result<Vec<Cell>>>()?
        }
        (None, Some(list)) => {
            if list.len() > MAX_STROKE_POINTS {
                return Err(p.bad(format!("`stroke` holds {} positions; at most {MAX_STROKE_POINTS}", list.len())));
            }
            let points = list
                .iter()
                .map(|v| {
                    let (x, y) = pair(p, "stroke", v)?;
                    if x.abs() > MAX_COORDINATE || y.abs() > MAX_COORDINATE {
                        return Err(p.bad(format!("`stroke`: [{x}, {y}] lies outside ±{MAX_COORDINATE}")));
                    }
                    Ok((x, y))
                })
                .collect::<Result<Vec<(f64, f64)>>>()?;
            crossed(p, &points, spacing)?
        }
    };
    let mut seen = HashSet::new();
    out.retain(|c| seen.insert(*c));
    Ok(out)
}

/// A `[a, b]` pair of finite numbers.
fn pair(p: &Params, key: &str, v: &Value) -> Result<(f64, f64)> {
    let numbers = v.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
    match numbers {
        Some((a, b)) if a.is_finite() && b.is_finite() => Ok((a, b)),
        _ => Err(p.bad(format!("`{key}` must hold [number, number] pairs, found {}", kind(v)))),
    }
}

/// The cell holding the font-unit position `pos`.
fn cell_at(pos: (f64, f64), spacing: f64) -> Cell {
    ((pos.0 / spacing).floor() as i64, (pos.1 / spacing).floor() as i64)
}

/// Every cell the stroke crosses, in order: the first position's cell, then the cells each
/// straight piece passes into. A piece through a grid corner steps into one of the two cells
/// beside the corner, not both.
fn crossed(p: &Params, points: &[(f64, f64)], spacing: f64) -> Result<Vec<Cell>> {
    let Some(&first) = points.first() else { return Ok(Vec::new()) };
    // Each piece passes into as many cells as the grid lines it crosses; count before walking.
    let mut total: u64 = 1;
    for w in points.windows(2) {
        if let [a, b] = w {
            let (ca, cb) = (cell_at(*a, spacing), cell_at(*b, spacing));
            total = total.saturating_add(ca.0.abs_diff(cb.0)).saturating_add(ca.1.abs_diff(cb.1));
        }
    }
    if total > MAX_PIXEL_CELLS as u64 {
        return Err(p.bad(format!("the stroke crosses {total} cells; at most {MAX_PIXEL_CELLS}")));
    }
    let mut out = Vec::with_capacity(total as usize);
    out.push(cell_at(first, spacing));
    for w in points.windows(2) {
        if let [a, b] = w {
            walk(*a, *b, spacing, &mut out);
        }
    }
    Ok(out)
}

/// Pushes the cells the piece from `a` to `b` passes into after `a`'s own, ending with `b`'s,
/// stepping across one grid line at a time in the order the piece meets them.
fn walk(a: (f64, f64), b: (f64, f64), spacing: f64, out: &mut Vec<Cell>) {
    let (ax, ay, bx, by) = (a.0 / spacing, a.1 / spacing, b.0 / spacing, b.1 / spacing);
    let (mut cx, mut cy) = cell_at(a, spacing);
    let (ex, ey) = cell_at(b, spacing);
    let (dx, dy) = (bx - ax, by - ay);
    let step_x = if ex > cx { 1 } else { -1 };
    let step_y = if ey > cy { 1 } else { -1 };
    // How far along the piece (0 at `a`, 1 at `b`) it meets the next grid line of each direction.
    let next = |from: f64, cell: i64, d: f64| {
        if d > 0.0 {
            ((cell + 1) as f64 - from) / d
        } else if d < 0.0 {
            (from - cell as f64) / -d
        } else {
            f64::INFINITY
        }
    };
    let (mut t_x, mut t_y) = (next(ax, cx, dx), next(ay, cy, dy));
    let delta = |d: f64| if d != 0.0 { 1.0 / d.abs() } else { f64::INFINITY };
    let (dt_x, dt_y) = (delta(dx), delta(dy));
    // Each step moves one coordinate one cell nearer `b`'s cell, so the loop ends after exactly
    // the number of cells `crossed` counted, whatever rounding does to the crossing distances.
    while (cx, cy) != (ex, ey) {
        if cy == ey || (cx != ex && t_x < t_y) {
            cx += step_x;
            t_x += dt_x;
        } else {
            cy += step_y;
            t_y += dt_y;
        }
        out.push((cx, cy));
    }
}

/// The cell a component sits on: its offset is a whole number of cells from the origin.
fn cell_of(c: &Component, spacing: f64) -> Option<Cell> {
    let [_, _, _, _, x, y] = c.affine().as_coeffs();
    let (col, row) = ((x / spacing).round(), (y / spacing).round());
    let on_grid = (x - col * spacing).abs() <= OFFSET_EPSILON && (y - row * spacing).abs() <= OFFSET_EPSILON;
    on_grid.then_some((col as i64, row as i64))
}

/// Applies `mode` to `cells` on the layer.
fn paint(p: &Params, layer: &mut Layer, cells: &[Cell], mode: Mode, spacing: f64) -> Result<Painted> {
    let mut filled: HashMap<Cell, Vec<usize>> = HashMap::new();
    let mut components = 0usize;
    for (i, shape) in layer.shapes.iter().enumerate() {
        let Shape::Component(c) = shape else { continue };
        components += 1;
        if c.glyph == PIXEL_GLYPH
            && let Some(cell) = cell_of(c, spacing)
        {
            filled.entry(cell).or_default().push(i);
        }
    }
    let mode = match mode {
        Mode::Paint if cells.first().is_some_and(|c| filled.contains_key(c)) => Mode::Remove,
        Mode::Paint => Mode::Add,
        other => other,
    };
    let mut dropped: HashSet<usize> = HashSet::new();
    let (mut added, mut removed) = (Vec::new(), Vec::new());
    for cell in cells {
        match (filled.get(cell), mode) {
            (Some(shapes), Mode::Remove | Mode::Toggle) => {
                dropped.extend(shapes.iter().copied());
                removed.push(*cell);
            }
            (None, Mode::Add | Mode::Toggle) => added.push(*cell),
            _ => {}
        }
    }
    if components.saturating_sub(dropped.len()).saturating_add(added.len()) > MAX_LAYER_COMPONENTS {
        return Err(p.bad(format!("layer `{}` would hold more than {MAX_LAYER_COMPONENTS} components", layer.id)));
    }
    if !dropped.is_empty() {
        let mut i = 0usize;
        layer.shapes.retain(|_| {
            let keep = !dropped.contains(&i);
            i += 1;
            keep
        });
    }
    for &(col, row) in &added {
        let mut c = Component::new(PIXEL_GLYPH, col as f64 * spacing, row as f64 * spacing);
        c.info.alignment = Alignment::Disabled;
        layer.shapes.push(Shape::Component(c));
    }
    Ok(Painted { mode, added, removed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walked(points: &[(f64, f64)]) -> Vec<Cell> {
        let v = json!({});
        let p = Params::new("pixel.setPixels", &v).unwrap();
        crossed(&p, points, 10.0).unwrap()
    }

    #[test]
    fn a_stroke_crosses_every_cell_between_its_positions() {
        assert_eq!(walked(&[(5.0, 5.0)]), vec![(0, 0)]);
        assert_eq!(walked(&[(5.0, 5.0), (35.0, 5.0)]), vec![(0, 0), (1, 0), (2, 0), (3, 0)]);
        assert_eq!(walked(&[(5.0, 5.0), (-15.0, -5.0)]), vec![(0, 0), (-1, 0), (-1, -1), (-2, -1)]);
        // A diagonal steps through one neighbour at each corner, so every cell touches the last.
        let diagonal = walked(&[(1.0, 2.0), (29.0, 28.0)]);
        assert_eq!(diagonal.first(), Some(&(0, 0)));
        assert_eq!(diagonal.last(), Some(&(2, 2)));
        assert_eq!(diagonal.len(), 5);
        for pair in diagonal.windows(2) {
            let [a, b] = pair else { panic!() };
            assert_eq!(a.0.abs_diff(b.0) + a.1.abs_diff(b.1), 1, "{diagonal:?}");
        }
    }
}
