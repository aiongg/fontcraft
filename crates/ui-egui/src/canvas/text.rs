//! Text mode on the canvas: the kerned gaps (`kerning.highlightColors`), the selection, the text
//! cursor and, with Show Metrics on, each glyph's sidebearings and width under it
//! (`spacing.metricsOverlayInTextMode`); and the pointer in text mode: a click places the cursor,
//! a drag selects, a double-click on a glyph edits it.
//!
//! Everything here works in text coordinates: font units of the laid-out text, the first line's
//! baseline at y = 0 (`fontcraft_textlayout::Layout`).

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, FontId, Pos2, Rect, Shape, Stroke};
use fontcraft_geom::Point;
use fontcraft_model::Font;
use fontcraft_spacing::kerning::Direction;
use fontcraft_spacing::{Field, metric};
use fontcraft_textlayout::Layout;
use fontcraft_tools::{PointerKind, View};
use serde_json::json;

use super::Painting;
use crate::app::FontcraftApp;
use crate::panels::widgets::format_number;
use crate::state::{Tab, TabView};
use crate::tools_glue::{RawPointer, Space};

/// Most sidebearing entries kept; the cache starts over past it.
const KEPT_SIDEBEARINGS: usize = 4096;
/// Below this many screen points of advance a glyph gets no metrics labels: they would overlap.
const MIN_LABELLED_ADVANCE: f32 = 24.0;

/// The transform of text coordinates on a canvas whose centre is `center`.
pub fn text_view(view: &TabView, center: Point) -> View {
    super::view::tools_view(view, [0.0, 0.0], center)
}

/// The rectangle `a0..a1` along line `line` covers, from `low` to `high` above the baseline (a
/// column one line height wide in vertical text), as `[x0, y0, x1, y1]` in text coordinates.
fn band(layout: &Layout, line: usize, (a0, a1): (f64, f64), (low, high): (f64, f64)) -> [f64; 4] {
    let (a0, a1) = (a0.min(a1), a0.max(a1));
    let base = layout.lines.get(line).map_or(0.0, |l| l.baseline);
    let lh = layout.line_height;
    match layout.direction {
        Direction::Ltr => [a0, base + low, a1, base + high],
        Direction::Rtl => [-a1, base + low, -a0, base + high],
        Direction::Vertical => [base - lh / 2.0, -a1, base + lh / 2.0, -a0],
    }
}

/// [`band`] across the line's full height: from the ascender down by one line height.
fn span(layout: &Layout, line: usize, a0: f64, a1: f64) -> [f64; 4] {
    band(layout, line, (a0, a1), (layout.ascender - layout.line_height, layout.ascender))
}

/// The advance box of item `i`.
fn item_box(layout: &Layout, i: usize) -> Option<[f64; 4]> {
    let p = layout.placed.get(i)?;
    Some(span(layout, p.line, p.offset, p.offset + p.advance))
}

/// The glyph whose advance box holds the text point (x, y).
pub fn item_at(layout: &Layout, x: f64, y: f64) -> Option<usize> {
    let line = layout.placed.get(layout.hit(x, y)).or(layout.placed.last())?.line;
    let items = layout.lines.get(line)?.items.clone();
    items.into_iter().find(|&i| {
        layout.placed.get(i).is_some_and(|p| p.glyph.is_some())
            && item_box(layout, i).is_some_and(|[x0, y0, x1, y1]| (x0..=x1).contains(&x) && (y0..=y1).contains(&y))
    })
}

fn screen_rect(v: View, [x0, y0, x1, y1]: [f64; 4]) -> Rect {
    let a = v.to_screen(Point::new(x0, y0));
    let b = v.to_screen(Point::new(x1, y1));
    Rect::from_two_pos(Pos2::new(a.x as f32, a.y as f32), Pos2::new(b.x as f32, b.y as f32))
}

/// A glyph's sidebearings in a layer: left and right (top and bottom in vertical text); `None`
/// for an empty layer or one that cannot be measured.
type Sides = Option<(f64, f64)>;

/// Sidebearings measured for the metrics labels, kept while the document revision stays the
/// same.
#[derive(Default)]
pub struct Sidebearings {
    doc: (u64, u64),
    entries: HashMap<(Arc<str>, String, bool), Sides>,
}

impl Sidebearings {
    fn get(&mut self, doc: (u64, u64), font: &Font, tab: &Tab, glyph: &Arc<str>, vertical: bool) -> Sides {
        if self.doc != doc || self.entries.len() >= KEPT_SIDEBEARINGS {
            self.doc = doc;
            self.entries.clear();
        }
        let layer = super::view::shown_layer(font, tab, glyph)?;
        let key = (glyph.clone(), layer.id.clone(), vertical);
        if let Some(sides) = self.entries.get(&key) {
            return *sides;
        }
        let (a, b) = if vertical { (Field::Top, Field::Bottom) } else { (Field::Left, Field::Right) };
        let sides = match (metric(font, layer, a), metric(font, layer, b)) {
            (Ok(Some(a)), Ok(Some(b))) if a.is_finite() && b.is_finite() => Some((a, b)),
            _ => None,
        };
        self.entries.insert(key, sides);
        sides
    }
}

/// What [`draw_front`] needs for the metrics labels.
pub struct Overlay<'a> {
    pub font: &'a Font,
    pub doc: (u64, u64),
    /// The descender of the tab's master, where the labels start under each line.
    pub descender: f64,
    /// Show Metrics is on: label every glyph.
    pub labels: bool,
}

/// Draws what lies behind the glyphs of `tab`'s text in text mode: the kerned gaps, while the
/// mode shows kerning, and the selection. `c.view` is the text transform ([`text_view`]).
pub fn draw_behind(c: &Painting, layout: &Layout, tab: &Tab, descender: f64) {
    let (v, tokens) = (c.view, c.tokens);
    let heights = (descender.min(layout.ascender), layout.ascender);
    let visible = |r: Rect| r.intersects(c.rect);
    if tab.kerning.shows_kerning() {
        for p in &layout.placed {
            if p.kern == 0.0 || !p.kern.is_finite() {
                continue;
            }
            let r = screen_rect(v, band(layout, p.line, (p.offset - p.kern, p.offset), heights));
            if visible(r) {
                let color = if p.kern < 0.0 { tokens.kerning_negative } else { tokens.kerning_positive };
                c.painter.rect_filled(r, 0.0, color);
            }
        }
    }
    for i in tab.text.selection() {
        if let Some(p) = layout.placed.get(i) {
            let r = screen_rect(v, band(layout, p.line, (p.offset, p.offset + p.advance), heights));
            if visible(r) && r.width() > 0.0 && r.height() > 0.0 {
                c.painter.rect_filled(r, 0.0, tokens.text_selection);
            }
        }
    }
}

/// Draws what lies in front of the glyphs: the metrics labels while Show Metrics is on, and the
/// text cursor when nothing is selected.
pub fn draw_front(c: &Painting, layout: &Layout, tab: &Tab, overlay: &Overlay, sidebearings: &mut Sidebearings) {
    let (v, tokens) = (c.view, c.tokens);
    if overlay.labels {
        labels(c, layout, tab, overlay, sidebearings);
    }
    if tab.text.selection().is_empty() {
        let (x, y) = layout.caret(tab.text.cursor());
        let lh = layout.line_height;
        let (a, b) = match layout.direction {
            Direction::Vertical => (Point::new(x - lh / 2.0, y), Point::new(x + lh / 2.0, y)),
            _ => (Point::new(x, y + overlay.descender.min(layout.ascender)), Point::new(x, y + layout.ascender)),
        };
        let (a, b) = (v.to_screen(a), v.to_screen(b));
        let stroke = Stroke::new(tokens.text_cursor_width, tokens.text_cursor);
        c.painter.add(Shape::line_segment([Pos2::new(a.x as f32, a.y as f32), Pos2::new(b.x as f32, b.y as f32)], stroke));
    }
}

/// The sidebearings and width of every glyph on the canvas: under the descender, the left
/// sidebearing at the glyph's left edge, the right one at its right edge and the width centred
/// below them. In vertical text the top and bottom sidebearings sit at the top and bottom of the
/// advance, the vertical width between them, along the column's left edge.
fn labels(c: &Painting, layout: &Layout, tab: &Tab, overlay: &Overlay, sidebearings: &mut Sidebearings) {
    let (v, tokens) = (c.view, c.tokens);
    let font_id = FontId::proportional(tokens.sidebearing_label_size);
    let row = tokens.sidebearing_label_size + 2.0;
    let pad = 2.0;
    let vertical = layout.direction == Direction::Vertical;
    for (i, p) in layout.placed.iter().enumerate() {
        let Some(glyph) = &p.glyph else { continue };
        let Some(b) = item_box(layout, i) else { continue };
        let r = screen_rect(v, b);
        let along = if vertical { r.height() } else { r.width() };
        if along < MIN_LABELLED_ADVANCE || !r.expand(row * 2.0).intersects(c.rect) {
            continue;
        }
        let sides = sidebearings.get(overlay.doc, overlay.font, tab, glyph, vertical);
        let [first, second] = match sides {
            Some((a, b)) => [format_number(a), format_number(b)],
            None => [String::new(), String::new()],
        };
        let width = format_number(p.advance);
        let text = |at: Pos2, align: Align2, s: &str| {
            if !s.is_empty() {
                c.painter.text(at, align, s, font_id.clone(), tokens.sidebearing_label);
            }
        };
        // Negative kerning overlaps neighbouring advances: each label keeps to the part of the
        // advance its neighbour does not cover, so the two never sit on each other.
        let overlap = |kern: Option<f64>| (kern.unwrap_or(0.0).min(0.0).abs() * v.scale()) as f32;
        let next_kern = layout.placed.get(i + 1).filter(|n| n.line == p.line).map(|n| n.kern);
        let (before, after) = (overlap(Some(p.kern)), overlap(next_kern));
        match layout.direction {
            Direction::Vertical => {
                let x = r.left() + pad;
                text(Pos2::new(x, r.top() + before + pad), Align2::LEFT_TOP, &first);
                text(Pos2::new(x, r.bottom() - after - pad), Align2::LEFT_BOTTOM, &second);
                text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, &width);
            }
            direction => {
                let (left, right) = if direction == Direction::Rtl { (after, before) } else { (before, after) };
                let base = layout.lines.get(p.line).map_or(0.0, |l| l.baseline);
                let y = v.to_screen(Point::new(0.0, base + overlay.descender)).y as f32 + pad;
                let (start, end) = (r.left() + left + pad, r.right() - right - pad);
                let label = |s: &str| c.painter.layout_no_wrap(s.to_string(), font_id.clone(), tokens.sidebearing_label);
                let (a, b) = (label(&first), label(&second));
                // A narrow glyph shows only its width: the two sidebearings would run into each
                // other. Zooming in brings them back.
                if a.size().x + b.size().x + pad <= end - start {
                    c.painter.galley(Pos2::new(start, y), a, tokens.sidebearing_label);
                    c.painter.galley(Pos2::new(end - b.size().x, y), b, tokens.sidebearing_label);
                }
                text(Pos2::new(r.center().x, y + row), Align2::CENTER_TOP, &width);
            }
        }
    }
}

/// The anchor of a drag that selects text: the cursor index where the button went down.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextDrag {
    anchor: Option<usize>,
}

/// Whether pointer input on the canvas places the text cursor: the Text tool is active (not
/// only the base under a held Hand or Zoom tool) and the tab is in text mode. The two go together
/// (`shell::text::tool_follows_tab`); the Text tool's own `pointer` does nothing, so this is the
/// one path for clicks on text.
pub fn takes_pointer(app: &FontcraftApp) -> bool {
    app.state.tab().is_some_and(|t| t.text_mode) && app.tools.toolbox.active_id() == crate::shell::text::TEXT_TOOL
}

/// A pointer event in text mode. `None` when the event is not for the text (another mode, a
/// Hand or Zoom tool held, a pointer move); else the errors of the commands it ran. A position
/// in [`Space::Font`] is in text coordinates.
pub fn pointer(app: &mut FontcraftApp, ev: RawPointer) -> Option<Vec<String>> {
    if !takes_pointer(app) || ev.kind == PointerKind::Move {
        return None;
    }
    let tab = app.state.tab()?.clone();
    let layout = super::view::text_layout(&app.session, &app.layouts, &tab)?;
    let at = match ev.space {
        Space::Screen => text_view(&tab.view, crate::tools_glue::canvas_center(app)).to_font(ev.at),
        // Font units of the text, not of the edited glyph: the edited glyph moves with the cursor.
        Space::Font => ev.at,
    };
    if let Some(errors) = super::measurement_line::pointer(app, &tab, &layout, &ev, at) {
        app.tools.text_drag = TextDrag::default();
        return Some(errors);
    }
    let index = layout.hit(at.x, at.y);
    let mut errors = Vec::new();
    match ev.kind {
        PointerKind::DoubleClick => {
            app.tools.text_drag = TextDrag::default();
            if let Some(item) = item_at(&layout, at.x, at.y) {
                // A glyph a preview feature shows in place of the item is the one edited.
                let shown = layout.placed.get(item).and_then(|p| p.glyph.as_deref()).filter(|g| tab.item_glyph(item) != Some(*g));
                let mut params = json!({"tab": tab.id, "index": item});
                if let (Some(glyph), Some(obj)) = (shown, params.as_object_mut()) {
                    obj.insert("glyph".into(), json!(glyph));
                }
                run(app, &mut errors, "editView.setCursor", params);
                // Leaving text mode makes the Select tool active, as Escape does.
                run(app, &mut errors, "editView.toggleTextMode", json!({"tab": tab.id, "mode": "edit"}));
            }
        }
        PointerKind::Down | PointerKind::Drag | PointerKind::Up => {
            let anchor = match (ev.kind, app.tools.text_drag.anchor) {
                // Shift keeps the end of the selection away from the click.
                (PointerKind::Down, _) if ev.mods.shift => {
                    let s = tab.text.selection();
                    Some(if index < s.start { s.end } else { s.start })
                }
                (PointerKind::Down, _) => Some(index),
                // A drag or release without a press on the text gives `None`.
                (_, anchor) => anchor,
            };
            if let Some(anchor) = anchor {
                app.tools.text_drag.anchor = (ev.kind != PointerKind::Up).then_some(anchor);
                let (start, end) = (anchor.min(index), anchor.max(index));
                if tab.text.selection() != (start..end) || tab.text.cursor() != start {
                    run(app, &mut errors, "editView.setCursor", json!({"tab": tab.id, "index": start, "selectionLength": end - start}));
                }
            }
        }
        PointerKind::Move => {}
    }
    Some(errors)
}

fn run(app: &mut FontcraftApp, errors: &mut Vec<String>, id: &str, params: serde_json::Value) {
    if let Err(e) = app.run_from_shell(id, &params) {
        app.state.status = e.clone();
        errors.push(e);
    }
}
