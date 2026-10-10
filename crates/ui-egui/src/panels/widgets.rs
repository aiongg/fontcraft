//! The controls every panel uses: number and text fields with the field rules of
//! `docs/spec/03b` § Info box, buttons with stable ids, the nine-point grid and section headers.
//!
//! Field rules: clicking a field selects its text; Return or leaving the field commits; Tab
//! commits and moves to the next field (Shift+Tab to the previous one); Escape cancels and leaves
//! no field focused; Up and Down change a number by 1, by 10 with Shift, and commit at once. A
//! field left with its text unchanged commits nothing, so tabbing through fields adds no undo
//! steps.
//!
//! Every field and button has an id made by [`id`] from a salt such as `"infoBox.x"`, so tests
//! (and the control channel) find it with `Context::read_response` or focus it directly.

use std::hash::Hash;

use egui::text::CCursorRange;
use egui::{Id, Key, Modifiers, Sense, StrokeKind};

use crate::icons::{self, Icon};
use crate::theme::Tokens;

/// Up and Down change a number by this much, and by [`BIG_STEP`] with Shift.
pub const STEP: f64 = 1.0;
pub const BIG_STEP: f64 = 10.0;
/// Longest text a field takes.
const MAX_FIELD_CHARS: usize = 256;
/// Decimals a number field shows.
const DECIMALS: i32 = 2;

/// The keys a focused field keeps for itself: arrows move the cursor or step the value, Escape
/// cancels. Tab is left to egui, which moves the focus.
const FILTER: egui::EventFilter = egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true };

/// The id of the panel control made from `salt`.
pub fn id(salt: impl Hash + std::fmt::Debug) -> Id {
    Id::new(("fontcraft.panel", salt))
}

/// A field being edited: its text, and the pass that last drew it focused.
#[derive(Clone)]
struct Editing {
    text: String,
    pass: u64,
}

/// What the user did to a number field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumberEdit {
    Set(f64),
    /// The field was emptied (only fields made with `clearable`).
    Clear,
}

/// `n` as a field shows it: at most two decimals, no trailing zeros, no negative zero.
pub fn format_number(n: f64) -> String {
    let scale = 10f64.powi(DECIMALS);
    let rounded = (n * scale).round() / scale;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = format!("{rounded:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn parse_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A number field showing `value` (`None`: `--` when clearable, else empty).
pub fn number(ui: &mut egui::Ui, id: Id, value: Option<f64>, width: f32, clearable: bool) -> Option<NumberEdit> {
    let shown = match value {
        Some(v) => format_number(v),
        None if clearable => "--".to_string(),
        None => String::new(),
    };
    let step = |text: &str, delta: f64| parse_number(text).or(value).map(|v| format_number(v + delta));
    let text = line(ui, id, &shown, width, &step)?;
    let trimmed = text.trim();
    if clearable && (trimmed.is_empty() || trimmed == "--") {
        return value.is_some().then_some(NumberEdit::Clear);
    }
    parse_number(trimmed).map(NumberEdit::Set)
}

/// A text field showing `value`; the committed text.
pub fn text(ui: &mut egui::Ui, id: Id, value: &str, width: f32) -> Option<String> {
    line(ui, id, value, width, &|_, _| None)
}

/// The arrow keys of this frame: +1/-1, ±10 with Shift; consumed so the field keeps them.
fn arrow_delta(input: &mut egui::InputState) -> f64 {
    let mut delta = 0.0;
    for (key, sign) in [(Key::ArrowUp, 1.0), (Key::ArrowDown, -1.0)] {
        // Shift first: a plain pattern also matches presses with Shift held.
        delta += sign * BIG_STEP * input.count_and_consume_key(Modifiers::SHIFT, key) as f64;
        delta += sign * STEP * input.count_and_consume_key(Modifiers::NONE, key) as f64;
    }
    delta
}

/// One line of text under the field rules. Returns the text to commit. `step` turns the text
/// and an arrow-key delta into the new text, or `None` where arrows do nothing.
fn line(ui: &mut egui::Ui, id: Id, shown: &str, width: f32, step: &dyn Fn(&str, f64) -> Option<String>) -> Option<String> {
    let ctx = ui.ctx().clone();
    let pass = ctx.cumulative_pass_nr();
    // An edit left behind by a field that stopped being drawn (the selection changed under it)
    // is dropped, not committed later.
    let editing = ctx.data(|d| d.get_temp::<Editing>(id)).filter(|e| pass.saturating_sub(e.pass) <= 1);
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut text = editing.as_ref().map_or_else(|| shown.to_string(), |e| e.text.clone());
    let mut commit = None;
    let mut cancelled = false;
    if focused && editing.is_some() {
        let (escape, delta) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Escape), arrow_delta(i)));
        if escape {
            cancelled = true;
            text = shown.to_string();
            ctx.memory_mut(|m| m.surrender_focus(id));
        } else if delta != 0.0
            && let Some(next) = step(&text, delta)
        {
            text.clone_from(&next);
            commit = Some(next);
        }
    }
    let mut output = egui::TextEdit::singleline(&mut text).id(id).desired_width(width).char_limit(MAX_FIELD_CHARS).event_filter(FILTER).show(ui);
    if output.response.has_focus() {
        if editing.is_none() {
            output.state.cursor.set_char_range(Some(CCursorRange::select_all(&output.galley)));
            output.state.store(&ctx, id);
        }
        ctx.data_mut(|d| d.insert_temp(id, Editing { text, pass }));
    } else if editing.is_some() {
        ctx.data_mut(|d| d.remove::<Editing>(id));
        if !cancelled && text != shown {
            commit = Some(text);
        }
    }
    commit
}

/// A push button with a stable id. `selected` draws it pressed (a mode that is on).
pub fn button(ui: &mut egui::Ui, id: Id, label: &str, hover: &str, enabled: bool, selected: bool) -> bool {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, ui.visuals().text_color());
    let padding = ui.spacing().button_padding;
    let side = ui.spacing().interact_size.y;
    let size = (galley.size() + 2.0 * padding).max(egui::vec2(side, side));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        let color = if enabled { visuals.text_color() } else { ui.visuals().weak_text_color() };
        ui.painter().rect(rect.expand(visuals.expansion), visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    }
    enabled && response.on_hover_text(hover).clicked()
}

/// A square push button showing a drawn icon (`crate::icons`) instead of a label.
pub fn icon_button(ui: &mut egui::Ui, id: Id, icon: Icon, hover: &str, enabled: bool, selected: bool) -> bool {
    let response = icon_button_response(ui, id, icon, hover, enabled, selected);
    enabled && response.clicked()
}

/// An icon button's side, in widget heights: a little taller than a text row, so the icon's
/// details show.
const ICON_BUTTON_SCALE: f32 = 1.25;
/// The margin around a button's icon, as a share of its side.
const ICON_INSET: f32 = 0.12;

/// [`icon_button`]'s response.
pub fn icon_button_response(ui: &mut egui::Ui, id: Id, icon: Icon, hover: &str, enabled: bool, selected: bool) -> egui::Response {
    let side = ui.spacing().interact_size.y * ICON_BUTTON_SCALE;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        let color = if enabled { visuals.text_color() } else { ui.visuals().weak_text_color() };
        ui.painter().rect(rect.expand(visuals.expansion), visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        icons::paint(ui.painter(), rect.shrink(side * ICON_INSET), icon, color);
    }
    if enabled { response.on_hover_text(hover) } else { response }
}

/// The nine points of a bounding box, row by row from the top, as the commands name them.
pub const BOX_POINTS: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];

/// A 3×3 grid of points; `active` is drawn in the accent colour. Returns the point clicked.
pub fn point_grid(ui: &mut egui::Ui, tokens: &Tokens, salt: &str, active: Option<&str>) -> Option<&'static str> {
    let cell = tokens.point_grid_cell;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(cell * 3.0, cell * 3.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect.shrink(cell / 2.0), 0.0, egui::Stroke::new(1.0, tokens.text_dim), StrokeKind::Middle);
    let mut clicked = None;
    for (i, name) in BOX_POINTS.iter().enumerate() {
        let (col, row) = ((i % 3) as f32, (i / 3) as f32);
        let r = egui::Rect::from_min_size(rect.min + egui::vec2(col * cell, row * cell), egui::vec2(cell, cell));
        let response = ui.interact(r, id((salt, *name)), Sense::click()).on_hover_text(*name);
        let on = active == Some(*name);
        let color = if on {
            tokens.accent
        } else if response.hovered() {
            tokens.text
        } else {
            tokens.text_dim
        };
        painter.circle_filled(r.center(), if on { cell * 0.3 } else { cell * 0.2 }, color);
        if response.clicked() {
            clicked = Some(*name);
        }
    }
    clicked
}

/// A section's title row with its chevron; true when clicked.
pub fn section_header(ui: &mut egui::Ui, tokens: &Tokens, section: &str, title: &str, expanded: bool) -> bool {
    let height = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let response = ui.interact(rect, id(("section", section)), Sense::click());
    let painter = ui.painter_at(rect);
    let c = egui::pos2(rect.left() + height / 2.0, rect.center().y);
    let s = height * 0.2;
    let points = if expanded {
        vec![c + egui::vec2(-s, -s * 0.6), c + egui::vec2(s, -s * 0.6), c + egui::vec2(0.0, s * 0.8)]
    } else {
        vec![c + egui::vec2(-s * 0.6, -s), c + egui::vec2(-s * 0.6, s), c + egui::vec2(s * 0.8, 0.0)]
    };
    let color = if response.hovered() { tokens.text } else { tokens.text_dim };
    painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
    let font = egui::TextStyle::Button.resolve(ui.style());
    painter.text(egui::pos2(rect.left() + height, rect.center().y), egui::Align2::LEFT_CENTER, title, font, tokens.text);
    response.clicked()
}
