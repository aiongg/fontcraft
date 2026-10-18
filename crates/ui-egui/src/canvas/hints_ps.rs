//! PostScript hints in Edit View, drawn while View › Show Hints is on (`view.showHints`,
//! `hinting.ps.manual.display`, `hinting.ps.manual.select`, `hinting.ps.manual.dragMarkers`).
//!
//! What is drawn comes from `hint.ps.listHints` on the edited layer, read once per frame into
//! [`HintMarks`], the same list the Select tool hits ([`fontcraft_tools::hint_drag`]):
//!
//! | Part | Look |
//! |---|---|
//! | bar | across the canvas between the hint's edges: yellow for a horizontal stem, green for a vertical one; a ghost hint is a blue band on the inner side of its edge |
//! | badge | a gray box beside the hint with its origin and size (a ghost: its position and an arrow, up for a top ghost, down for a bottom one); orange while selected, ringed red when `hint.ps.listHints` warns about the hint |
//! | markers | blue, at the edges: a circle at the origin, a triangle at the other edge pointing from the origin; filled when that end is linked to a node |
//!
//! Hints are shown for the foreground of the edited layer only, outside text mode. The module
//! also holds the shell commands of hint display and selection ([`COMMANDS`]) and fills the
//! targets of the `hint.ps.*` commands from the Edit View selection ([`targets`]).

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, StrokeKind};
use fontcraft_engine::Session;
use fontcraft_geom::Point;
use fontcraft_tools::hint_drag::{self, End, HintKind, HintLayout, HintMark, HintMarks};
use fontcraft_tools::{Item, Selection};
use serde_json::{Map, Value, json};

use super::Painting;
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};
use crate::state::UiState;
use crate::theme::Tokens;
use crate::tools_glue::{self, Space};

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.showHints",
        "Show Hints",
        &["View"],
        "",
        "{on?: bool} draw the edited layer's PostScript hints (bars, number badges, and the markers the Select tool drags); TrueType hints are not affected (omitted: toggle) → {on}",
    ),
    (
        "hint.ps.selectNextHint",
        "Select Next Hint",
        &[],
        "",
        "{} with PostScript hints selected and Show Hints on: select the hint that follows the selected one latest in list order, the first after the last (Tab while hints are selected) → {selection}",
    ),
    (
        "hint.ps.selectPreviousHint",
        "Select Previous Hint",
        &[],
        "",
        "{} with PostScript hints selected and Show Hints on: select the hint before the selected one earliest in list order, the last before the first (Shift+Tab while hints are selected) → {selection}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "view.showHints" => {
            let on = args.toggle(app.state.display.hints)?;
            app.state.display.hints = on;
            if !on {
                // Hidden hints cannot be acted on.
                for tab in &mut app.state.tabs {
                    tab.selection.hints.clear();
                }
            }
            Ok(json!({"on": on}))
        }
        "hint.ps.selectNextHint" => step(app, args, true),
        "hint.ps.selectPreviousHint" => step(app, args, false),
        other => Err(format!("unknown command `{other}`")),
    }
}

/// The sizes of the marks, from the tokens.
pub fn layout(tokens: &Tokens) -> HintLayout {
    let t = &tokens.ps_hints;
    let [width, height] = t.badge_size;
    HintLayout { lane: f64::from(t.lane), badge: (f64::from(width), f64::from(height)), marker: f64::from(t.marker_radius), gap: f64::from(t.gap) }
}

/// The edited layer's hints as `hint.ps.listHints` reports them, placed for drawing and hitting.
/// Not shown (and empty) while Show Hints is off, in text mode, while the background is edited,
/// or when the list cannot be read.
pub fn marks(session: &mut Session, state: &UiState, tokens: &Tokens) -> HintMarks {
    let mut out = HintMarks { layout: layout(tokens), ..HintMarks::default() };
    if !state.display.hints {
        return out;
    }
    let Some(tab) = state.tab() else { return out };
    if tab.text_mode || tab.edit_background {
        return out;
    }
    let (glyph, layer, bottom) = {
        let Ok(font) = session.font() else { return out };
        let Some(glyph) = tab.edited_glyph() else { return out };
        let Some(layer) = super::view::active_layer(font, tab) else { return out };
        let bottom = super::view::active_master(font, tab).map_or(0.0, |m| super::view::vertical_span(font, m).0);
        (glyph.to_string(), layer.id.clone(), bottom)
    };
    let Ok(list) = session.query("hint.ps.listHints", &json!({"glyph": glyph, "layer": layer})) else { return out };
    out.shown = true;
    out.left = 0.0;
    out.bottom = if bottom.is_finite() { bottom } else { 0.0 };
    out.hints = list.get("hints").and_then(Value::as_array).map(|a| a.iter().filter_map(HintMark::from_value).collect()).unwrap_or_default();
    out
}

/// After an edit, undo or redo: drops the selected hints the layer no longer has, and every
/// selected hint while hints are not shown.
pub fn retain(app: &mut FontcraftApp) {
    if app.state.tab().is_none_or(|t| t.selection.hints.is_empty()) {
        return;
    }
    let marks = marks(&mut app.session, &app.state, &app.tokens);
    if let Some(tab) = app.state.tab_mut() {
        if marks.shown {
            tab.selection.hints.retain(|&i| marks.get(i).is_some());
        } else {
            tab.selection.hints.clear();
        }
    }
}

/// `hint.ps.selectNextHint` and `hint.ps.selectPreviousHint`.
fn step(app: &mut FontcraftApp, args: &Args, forward: bool) -> Result<Value, String> {
    tools_glue::sync(app);
    let marks = marks(&mut app.session, &app.state, &app.tokens);
    if !marks.shown {
        return Err(args.err("turn on View › Show Hints and edit a glyph first"));
    }
    let tab = app.state.tab().ok_or_else(|| args.err("no Edit View tab is showing (open a glyph first)"))?;
    let selected = marks.selected(&tab.selection);
    let from = if forward { selected.last() } else { selected.first() };
    let from = *from.ok_or_else(|| args.err("select a hint first"))?;
    let count = marks.hints.len();
    let next = if forward { from.saturating_add(1).checked_rem(count).unwrap_or(0) } else { from.checked_sub(1).unwrap_or(count.saturating_sub(1)) };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tab_mut().ok_or_else(|| args.err("no Edit View tab is showing"))?;
    tab.selection = Selection::from_items([Item::Hint(next)]);
    let scene = tools_glue::scene(font, tab).ok_or_else(|| args.err("the edited glyph has no layer in the active master"))?;
    Ok(json!({"selection": Value::Object(tab.selection.targets(&scene))}))
}

/// The parameters a `hint.ps.*` command run from a menu takes from the Edit View: the edited
/// glyph and layer, the selected on-curve nodes as `nodes` (Add Horizontal and Add Vertical
/// Hint), the selected hints as `hints` (Make Ghost Hint, Delete Hints), the edited glyph and its
/// master (Autohint) or layer (Remove PostScript Hints). `None` for other commands, or when no
/// glyph is edited.
pub fn targets(app: &FontcraftApp, id: &str) -> Option<Value> {
    if !id.starts_with("hint.ps.") {
        return None;
    }
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    if tab.text_mode {
        return None;
    }
    let scene = tools_glue::scene(font, tab)?;
    let mut m = Map::new();
    m.insert("glyph".into(), json!(scene.glyph));
    m.insert("layer".into(), json!(scene.layer_id));
    match id {
        "hint.ps.addHorizontalHint" | "hint.ps.addVerticalHint" => {
            let nodes: Vec<Value> = tab
                .selection
                .on_curve_points(&scene)
                .into_iter()
                .filter_map(|(shape, node)| hint_drag::path_index(&scene, shape).map(|path| json!([path, node])))
                .collect();
            if !nodes.is_empty() {
                m.insert("nodes".into(), Value::Array(nodes));
            }
        }
        "hint.ps.makeGhostHint" | "hint.ps.deleteHints" => {
            if !tab.selection.hints.is_empty() {
                m.insert("hints".into(), json!(tab.selection.hints.iter().collect::<Vec<_>>()));
            }
        }
        "hint.ps.autohintGlyph" => return Some(json!({"glyphs": [scene.glyph], "layer": scene.layer.master})),
        "hint.ps.clearGlyphHints" => return Some(json!({"glyphs": [scene.glyph], "layers": [scene.layer_id]})),
        _ => return None,
    }
    Some(Value::Object(m))
}

/// The hint whose badge or marker is at `at`, while hints are shown.
pub fn hint_at(app: &mut FontcraftApp, at: Point, space: Space) -> Option<usize> {
    let marks = marks(&mut app.session, &app.state, &app.tokens);
    if !marks.shown {
        return None;
    }
    let view = tools_glue::current_view(app)?;
    let pos = match space {
        Space::Font => at,
        Space::Screen => view.to_font(at),
    };
    marks.hit(&view, pos, f64::from(app.tokens.hit_tolerance)).map(|h| h.index)
}

/// After `command` ran from a menu: the hints that Add Horizontal Hint, Add Vertical Hint or
/// Make Ghost Hint returned become the selection, so the Info box shows them.
pub fn adopt_result(app: &mut FontcraftApp, command: &str, result: &Value) {
    if !matches!(command, "hint.ps.addHorizontalHint" | "hint.ps.addVerticalHint" | "hint.ps.makeGhostHint") || !app.state.display.hints {
        return;
    }
    let Some(tab) = app.state.tab_mut() else { return };
    if result.get("selection").and_then(|s| s.get("glyph")).and_then(Value::as_str) != tab.edited_glyph() {
        return;
    }
    let hints: Vec<Item> = result
        .get("hints")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).map(Item::Hint).collect())
        .unwrap_or_default();
    if !hints.is_empty() {
        tab.selection = Selection::from_items(hints);
    }
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

/// A number as a badge shows it: whole units without decimals.
fn number(v: f64) -> String {
    crate::panels::widgets::format_number(v)
}

/// The bars, behind the outline.
pub fn draw_bars(c: &Painting, marks: &HintMarks) {
    if !marks.shown {
        return;
    }
    let t = &c.tokens.ps_hints;
    for h in &marks.hints {
        let Some(origin) = h.origin else { continue };
        let at = |v: f64| if h.horizontal { c.view.to_screen(Point::new(0.0, v)).y } else { c.view.to_screen(Point::new(v, 0.0)).x };
        let a = at(origin) as f32;
        if !a.is_finite() {
            continue;
        }
        let (lo, hi, color) = match h.kind {
            HintKind::Stem => {
                let b = h.target().map_or(a, |t| at(t) as f32);
                if !b.is_finite() {
                    continue;
                }
                (a.min(b), a.max(b), if h.horizontal { t.horizontal } else { t.vertical })
            }
            // The band lies inside the glyph: below a top edge, above a bottom one (screen y
            // grows downwards). A vertical ghost does not exist; drawn to the right if read.
            HintKind::GhostTop if h.horizontal => (a, a + t.ghost_band, t.ghost),
            HintKind::GhostBottom if h.horizontal => (a - t.ghost_band, a, t.ghost),
            HintKind::GhostTop | HintKind::GhostBottom => (a, a + t.ghost_band, t.ghost),
        };
        let rect = if h.horizontal {
            Rect::from_min_max(Pos2::new(c.rect.left(), lo), Pos2::new(c.rect.right(), hi))
        } else {
            Rect::from_min_max(Pos2::new(lo, c.rect.top()), Pos2::new(hi, c.rect.bottom()))
        };
        c.painter.rect_filled(rect, 0.0, color);
    }
}

/// The badges and markers, in front of the outline.
pub fn draw_marks(c: &Painting, marks: &HintMarks, selection: &Selection) {
    if !marks.shown {
        return;
    }
    let t = &c.tokens.ps_hints;
    let r = t.marker_radius;
    let stroke = Stroke::new(t.marker_width, t.marker);
    for h in &marks.hints {
        let Some(origin) = marks.marker(&c.view, h, End::Origin) else { continue };
        let target = marks.marker(&c.view, h, End::Target);
        // The badge.
        if let Some(b) = marks.badge(&c.view, h) {
            let rect = Rect::from_min_max(pos(b.min), pos(b.max));
            let fill = if selection.hints.contains(&h.index) { t.badge_selected } else { t.badge };
            c.painter.rect_filled(rect, 2.0, fill);
            if !h.warnings.is_empty() {
                c.painter.rect_stroke(rect, 2.0, Stroke::new(t.warning_width, t.warning), StrokeKind::Outside);
            }
            let font = FontId::proportional(t.badge_text_size);
            let centre = rect.center();
            match (h.kind, h.origin) {
                (HintKind::Stem, Some(o)) => {
                    let text = match h.size {
                        Some(s) => format!("{} / {}", number(o), number(s)),
                        None => number(o),
                    };
                    c.painter.text(centre, Align2::CENTER_CENTER, text, font, t.badge_text);
                }
                (kind, Some(o)) => {
                    // The position, then an arrow: up for a top ghost, down for a bottom one.
                    let arrow = Pos2::new(rect.right() - r - 2.0, centre.y);
                    c.painter.text(Pos2::new(centre.x - r, centre.y), Align2::CENTER_CENTER, number(o), font, t.badge_text);
                    let up = kind == HintKind::GhostTop;
                    let (tip, base) = if up { (arrow.y - r * 0.8, arrow.y + r * 0.8) } else { (arrow.y + r * 0.8, arrow.y - r * 0.8) };
                    let points = vec![Pos2::new(arrow.x, tip), Pos2::new(arrow.x + r * 0.7, base), Pos2::new(arrow.x - r * 0.7, base)];
                    c.painter.add(Shape::convex_polygon(points, t.badge_text, Stroke::NONE));
                }
                _ => {}
            }
        }
        // The circle at the origin.
        if h.origin_node.is_some() {
            c.painter.circle_filled(pos(origin), r, t.marker);
        } else {
            c.painter.circle_stroke(pos(origin), r, stroke);
        }
        // The triangle at the other edge, pointing away from the origin.
        if let Some(to) = target.filter(|_| h.kind == HintKind::Stem) {
            let (dx, dy) = (to.x - origin.x, to.y - origin.y);
            let len = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = if len > 1e-9 && len.is_finite() {
                ((dx / len) as f32, (dy / len) as f32)
            } else if h.horizontal {
                (0.0, -1.0)
            } else {
                (1.0, 0.0)
            };
            let at = pos(to);
            let tip = Pos2::new(at.x + ux * r, at.y + uy * r);
            let left = Pos2::new(at.x - ux * r - uy * r, at.y - uy * r + ux * r);
            let right = Pos2::new(at.x - ux * r + uy * r, at.y - uy * r - ux * r);
            let fill = if h.target_node.is_some() { t.marker } else { Color32::TRANSPARENT };
            c.painter.add(Shape::convex_polygon(vec![tip, left, right], fill, stroke));
        }
    }
}
