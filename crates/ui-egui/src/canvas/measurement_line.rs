//! The text-mode measurement line (`view.showMeasurementLine`): a horizontal line at the tab's
//! height on every line of the text, each glyph's sidebearings measured on it
//! (`measurementLine.semantics`, the numbers of `measure.sidebearingsAtHeight`), and thin lines at
//! the edges of every advance (`measurementLine.display`). It shows only while the Text tool is
//! the chosen tool (`measurementLine.textToolOnly`), in horizontal text. Cmd+Ctrl+Alt with a
//! click or a drag moves it to the pointer's height.
//!
//! Like [`super::text`], this works in text coordinates.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, FontId, Pos2, Shape, Stroke};
use fontcraft_geom::Point;
use fontcraft_model::{Font, MAX_COORDINATE};
use fontcraft_spacing::kerning::Direction;
use fontcraft_textlayout::Layout;
use fontcraft_tools::PointerKind;
use serde_json::json;

use super::Painting;
use crate::app::FontcraftApp;
use crate::panels::widgets::format_number;
use crate::shell::measurement::MeasurementLine;
use crate::state::Tab;
use crate::tools_glue::RawPointer;

/// Most measurements kept; the cache starts over past it.
const KEPT_MEASUREMENTS: usize = 4096;
/// Below this many screen points of advance a glyph gets no numbers: they would overlap.
const MIN_LABELLED_ADVANCE: f32 = 24.0;

/// Left and right sidebearings on the line; `None` where the outline does not reach it.
type Sides = Option<(f64, f64)>;

/// Measurements kept while the document revision and the height stay the same.
#[derive(Default)]
struct Cache {
    key: ((u64, u64), u64),
    entries: HashMap<(Arc<str>, String), Sides>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// The sidebearings of `glyph`, in the layer the tab shows it in, on the line at height `y`.
pub fn sides(font: &Font, doc: (u64, u64), tab: &Tab, glyph: &Arc<str>, y: f64) -> Sides {
    let layer = super::view::shown_layer(font, tab, glyph)?;
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (doc, y.to_bits());
        if cache.key != key || cache.entries.len() >= KEPT_MEASUREMENTS {
            cache.key = key;
            cache.entries.clear();
        }
        let entry = (glyph.clone(), layer.id.clone());
        if let Some(sides) = cache.entries.get(&entry) {
            return *sides;
        }
        let sides = match fontcraft_spacing::sidebearings_at_height(font, layer, y) {
            Ok(Some((l, r))) if l.is_finite() && r.is_finite() => Some((l, r)),
            _ => None,
        };
        cache.entries.insert(entry, sides);
        sides
    })
}

/// What [`draw`] needs besides the painting and the layout.
pub struct Line<'a> {
    pub font: &'a Font,
    pub doc: (u64, u64),
    pub tab: &'a Tab,
    pub state: &'a MeasurementLine,
    /// The chosen tool, ignoring a temporary Hand or Zoom tool.
    pub tool: &'a str,
}

/// Whether the line shows: it is on, the Text tool is chosen and the text runs horizontally.
fn shows(state: &MeasurementLine, tool: &str, layout: &Layout) -> bool {
    state.on && tool == crate::shell::text::TEXT_TOOL && layout.direction != Direction::Vertical
}

/// Draws the measurement line in front of the text. `c.view` is the text transform.
pub fn draw(c: &Painting, layout: &Layout, line: &Line) {
    if !shows(line.state, line.tool, layout) {
        return;
    }
    let (v, tokens) = (c.view, c.tokens);
    let height = line.state.height(line.font, line.tab);
    let screen = |x: f64, y: f64| {
        let p = v.to_screen(Point::new(x, y));
        Pos2::new(p.x as f32, p.y as f32)
    };
    let advance = Stroke::new(tokens.measurement_line_advance_width, tokens.measurement_line_advance);
    let rule = Stroke::new(tokens.measurement_line_width, tokens.measurement_line);
    let font_id = FontId::proportional(tokens.measurement_line_label_size);
    let (left, right) = (c.rect.left(), c.rect.right());
    for l in &layout.lines {
        let y = screen(0.0, l.baseline + height).y;
        if (c.rect.top()..=c.rect.bottom()).contains(&y) {
            c.painter.add(Shape::line_segment([Pos2::new(left, y), Pos2::new(right, y)], rule));
        }
    }
    for p in &layout.placed {
        let Some(glyph) = &p.glyph else { continue };
        let (top, bottom) = (screen(p.x, p.y + layout.ascender).y, screen(p.x, p.y + layout.ascender - layout.line_height).y);
        let (x0, x1) = (screen(p.x, p.y).x, screen(p.x + p.advance, p.y).x);
        if x1 < left || x0 > right || bottom < c.rect.top() || top > c.rect.bottom() {
            continue;
        }
        for x in [x0, x1] {
            c.painter.add(Shape::line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], advance));
        }
        if x1 - x0 < MIN_LABELLED_ADVANCE {
            continue;
        }
        let Some((lsb, rsb)) = sides(line.font, line.doc, line.tab, glyph, height) else { continue };
        let y = screen(p.x, p.y + height).y - 2.0;
        let label = |from: f64, to: f64, value: f64| {
            let at = Pos2::new((screen(p.x + from, p.y).x + screen(p.x + to, p.y).x) / 2.0, y);
            c.painter.text(at, Align2::CENTER_BOTTOM, format_number(value), font_id.clone(), tokens.measurement_line_label);
        };
        label(0.0, lsb, lsb);
        label(p.advance - rsb, p.advance, rsb);
    }
}

/// Cmd+Ctrl+Alt with a press or a drag in text mode, while the line shows: moves the tab's line
/// to the pointer's height above the baseline of the line under it. `at` is in text
/// coordinates. `None` when the event is not for the line.
pub fn pointer(app: &mut FontcraftApp, tab: &Tab, layout: &Layout, ev: &RawPointer, at: Point) -> Option<Vec<String>> {
    let m = ev.mods;
    if !(m.cmd && m.ctrl && m.alt) || !shows(&app.state.panels.measurement, app.tools.toolbox.base_id(), layout) {
        return None;
    }
    match ev.kind {
        PointerKind::Down | PointerKind::Drag => {}
        PointerKind::Up => return Some(Vec::new()),
        PointerKind::DoubleClick | PointerKind::Move => return None,
    }
    let line = layout.placed.get(layout.hit(at.x, at.y)).or(layout.placed.last()).map_or(0, |p| p.line);
    let base = layout.lines.get(line).map_or(0.0, |l| l.baseline);
    let y = (at.y - base).round().clamp(-MAX_COORDINATE, MAX_COORDINATE);
    if !y.is_finite() {
        return Some(Vec::new());
    }
    let mut errors = Vec::new();
    if let Err(e) = app.run_from_shell("view.setMeasurementLineHeight", &json!({"tab": tab.id, "y": y})) {
        app.state.status = e.clone();
        errors.push(e);
    }
    Some(errors)
}
