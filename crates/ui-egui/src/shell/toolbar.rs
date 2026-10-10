//! Edit View's toolbar: one button per toolbar group of [`TOOLS`], the list `tool.list` returns
//! (`editView.toolbar`, `tools.sharedButtonCycling`).
//!
//! A button shared by several tools shows the active tool when it is one of them, else the one of
//! them used last, else the first. Clicking a button runs `tool.select` for the tool it shows.
//! Holding a shared button (or clicking it with the secondary button) opens a menu of its tools.
//! The tool used last per button comes from the toolbox (`Toolbox::last_in_group`), so a tool
//! chosen by its key counts too. The buttons never take the keyboard focus, so the tool keys keep
//! working after a click.

use std::time::Duration;

use egui::{Align2, Color32, Id, Rect, Sense, vec2};
use fontcraft_tools::{TOOLS, ToolInfo};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::icons;
use crate::theme::Tokens;

/// Holding a shared button this long opens its menu.
pub const HOLD_SECONDS: f64 = 0.4;

/// One toolbar button: the tools of one group in toolbar order (a single tool when its group is
/// empty).
#[derive(Debug)]
pub struct Button {
    /// The group, or the tool id of an ungrouped tool.
    pub key: &'static str,
    pub tools: Vec<&'static ToolInfo>,
}

impl Button {
    pub fn shared(&self) -> bool {
        self.tools.len() > 1
    }

    /// The tool the button shows while `active` is the chosen tool and `last` was the one of its
    /// tools used last.
    pub fn shown(&self, active: &str, last: Option<&str>) -> Option<&'static ToolInfo> {
        let find = |id: &str| self.tools.iter().copied().find(|t| t.id == id);
        find(active).or_else(|| last.and_then(find)).or_else(|| self.tools.first().copied())
    }
}

/// The toolbar's buttons, in the order of [`TOOLS`].
pub fn buttons() -> Vec<Button> {
    let mut out: Vec<Button> = Vec::new();
    for tool in TOOLS {
        let key = if tool.group.is_empty() { tool.id } else { tool.group };
        match out.iter_mut().find(|b| b.key == key) {
            Some(b) => b.tools.push(tool),
            None => out.push(Button { key, tools: vec![tool] }),
        }
    }
    out
}

/// The id of the button `key` ([`Button::key`]).
pub fn button_id(key: &str) -> Id {
    Id::new(("fontcraft.toolbar", key))
}

/// The id of tool `id`'s row in a button's menu.
pub fn menu_item_id(id: &str) -> Id {
    Id::new(("fontcraft.toolbar.menu", id))
}

/// Hover text: the tool's name and its key.
fn hover(tool: &ToolInfo) -> String {
    if tool.shortcut.is_empty() { tool.label.to_string() } else { format!("{} ({})", tool.label, tool.shortcut) }
}

/// What the toolbar remembers between frames.
#[derive(Clone, Default)]
struct Memory {
    /// The button whose menu is open.
    open: Option<&'static str>,
    /// The pointer press that opened a menu by holding is not released yet; the click its
    /// release makes selects nothing.
    held: bool,
}

fn memory_id() -> Id {
    Id::new("fontcraft.toolbar.memory")
}

pub fn show(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    let tokens = &app.tokens;
    let ctx = ui.ctx().clone();
    let active = app.tools.toolbox.base_id();
    let mut mem: Memory = ctx.data_mut(|d| d.get_temp(memory_id())).unwrap_or_default();
    let buttons = buttons();
    let mut drawn: Vec<(&Button, Rect)> = Vec::new();
    let frame = egui::Frame::NONE.fill(tokens.panel_darker).inner_margin(egui::Margin::symmetric(4, 2));
    egui::Panel::top("edit_view_toolbar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens.toolbar_gap;
            for b in &buttons {
                let Some(tool) = b.shown(active, app.tools.toolbox.last_in_group(b.key)) else { continue };
                let on = b.tools.iter().any(|t| t.id == active);
                let (rect, _) = ui.allocate_exact_size(vec2(tokens.toolbar_button, tokens.toolbar_button), Sense::hover());
                let response = ui.interact(rect, button_id(b.key), Sense::CLICK);
                paint_button(ui, tokens, b, tool, rect, on, response.hovered());
                if b.shared() && !mem.held && response.is_pointer_button_down_on() {
                    let held_for = ui.input(|i| i.pointer.press_start_time().map(|t| i.time - t));
                    match held_for {
                        Some(t) if t >= HOLD_SECONDS => {
                            mem.open = Some(b.key);
                            mem.held = true;
                        }
                        Some(t) => ctx.request_repaint_after(Duration::try_from_secs_f64(HOLD_SECONDS - t).unwrap_or(Duration::ZERO)),
                        None => {}
                    }
                }
                if b.shared() && response.secondary_clicked() {
                    mem.open = Some(b.key);
                }
                if response.on_hover_text(hover(tool)).clicked() {
                    if mem.held {
                        mem.held = false;
                    } else {
                        pending.push(("tool.select", json!({ "tool": tool.id })));
                        mem.open = None;
                    }
                }
                drawn.push((b, rect));
            }
        });
    });
    if !ctx.input(|i| i.pointer.any_down()) {
        mem.held = false;
    }
    if let Some((b, rect)) = mem.open.and_then(|key| drawn.iter().find(|(b, _)| b.key == key)) {
        let mut chosen = false;
        let area = egui::Area::new(Id::new(("fontcraft.toolbar.popup", b.key)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.left_bottom() + vec2(0.0, tokens.toolbar_gap))
            .show(&ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    for tool in &b.tools {
                        if menu_row(ui, tokens, tool, tool.id == active) {
                            pending.push(("tool.select", json!({ "tool": tool.id })));
                            chosen = true;
                        }
                    }
                });
            });
        let menu = area.response.rect;
        let pressed_outside =
            ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !menu.contains(p) && !rect.contains(p)));
        if chosen || pressed_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            mem.open = None;
        }
    } else {
        mem.open = None;
    }
    ctx.data_mut(|d| d.insert_temp(memory_id(), mem));
}

/// Draws toolbar button `b` showing `tool` in `rect`; `on` while the active tool is one of its.
fn paint_button(ui: &egui::Ui, tokens: &Tokens, b: &Button, tool: &ToolInfo, rect: Rect, on: bool, hovered: bool) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let size = rect.width();
    let painter = ui.painter();
    let fill = if on {
        tokens.row_selected
    } else if hovered {
        tokens.hover
    } else {
        Color32::TRANSPARENT
    };
    let color = if on { tokens.selected_text } else { tokens.text };
    painter.rect_filled(rect, tokens.radius, fill);
    paint_tool(ui, tokens, tool, rect.shrink((size - tokens.toolbar_icon).max(0.0) / 2.0), color);
    if b.shared() {
        painter.add(icons::menu_mark(rect, size * 0.18, color));
    }
}

/// The tool's icon in `rect`, or its key for a tool without one.
fn paint_tool(ui: &egui::Ui, tokens: &Tokens, tool: &ToolInfo, rect: Rect, color: Color32) {
    match icons::for_tool(tool.id) {
        Some(icon) => icons::paint(ui.painter(), rect, icon, color),
        None => {
            let text: String = if tool.shortcut.is_empty() { tool.label.chars().take(1).collect() } else { tool.shortcut.to_string() };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, egui::FontId::proportional(tokens.font_size), color);
        }
    }
}

/// One row of a button's menu: icon, name and key. True when clicked.
fn menu_row(ui: &mut egui::Ui, tokens: &Tokens, tool: &ToolInfo, on: bool) -> bool {
    let height = tokens.toolbar_icon + 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(tokens.toolbar_menu_width, height), Sense::hover());
    let response = ui.interact(rect, menu_item_id(tool.id), Sense::CLICK);
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() {
            tokens.row_selected
        } else if on {
            tokens.hover
        } else {
            Color32::TRANSPARENT
        };
        let color = if response.hovered() { tokens.selected_text } else { tokens.text };
        ui.painter().rect_filled(rect, tokens.radius, fill);
        let icon = Rect::from_min_size(rect.min + vec2(3.0, 3.0), vec2(tokens.toolbar_icon, tokens.toolbar_icon));
        paint_tool(ui, tokens, tool, icon, color);
        let font = egui::FontId::proportional(tokens.font_size);
        let painter = ui.painter();
        painter.text(egui::pos2(icon.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, tool.label, font.clone(), color);
        if !tool.shortcut.is_empty() {
            painter.text(egui::pos2(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, tool.shortcut, font, tokens.text_dim);
        }
    }
    response.on_hover_text(hover(tool)).clicked()
}
