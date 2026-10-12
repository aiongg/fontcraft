//! The Command Console window (`window.macroPanel`): Clear and Copy, the log pane, the input
//! line, and under it the completion list with the parameters doc of the command being typed.
//!
//! | Control | Does |
//! |---|---|
//! | Clear | `window.macroPanel {clear}`: empties the pane |
//! | Copy | `window.macroPanel {copy}`: the pane's text to the clipboard |
//! | input line, Return | `window.macroPanel {line}`: runs the line |
//! | Up, Down in the input line | walk the history (the input line's own text) |
//! | Tab in the input line, a click on a suggestion | puts the suggestion in place of the word being typed |
//!
//! The pane draws only the rows in view, each cut at [`MAX_DRAWN_CHARS`] characters.

use egui::{Key, Modifiers, RichText, vec2};
use serde_json::json;

use crate::app::FontcraftApp;
use crate::panels::{Pending, widgets};
use crate::shell::command_console::{self, Completion, MAX_LINE_BYTES, Recall, SuggestionKind};

/// Characters of one log entry drawn in the pane; Copy gives whole lines.
pub const MAX_DRAWN_CHARS: usize = 400;

pub fn window_id() -> egui::Id {
    widgets::id(("commandConsole", "window"))
}

/// The input line.
pub fn input_id() -> egui::Id {
    widgets::id(("commandConsole", "input"))
}

fn drawn(text: &str) -> String {
    let mut out: String = text.chars().take(MAX_DRAWN_CHARS).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, completion: &Completion, pending: &mut Pending) {
    let FontcraftApp { session, state, tokens, .. } = app;
    let log = &session.extensions().log;
    let st = &mut state.panels.command_console;
    let ctx = ui.ctx().clone();
    let mut open = true;
    egui::Window::new("Command Console")
        .id(window_id())
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.console_width, tokens.console_height))
        .show(&ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Clear").clicked() {
                    pending.push(("window.macroPanel", json!({"clear": true})));
                }
                if ui.button("Copy").clicked() {
                    pending.push(("window.macroPanel", json!({"copy": true})));
                }
            });
            ui.add_space(tokens.dialog_spacing);

            // The pane takes what the input line and the completion list leave.
            let below = tokens.console_row_height * 3.0 + if completion.items.is_empty() { 0.0 } else { tokens.console_completion_height };
            let height = (ui.available_height() - below).max(tokens.console_row_height * 3.0);
            let first = log.position_of(st.shown_from);
            let rows = log.len().saturating_sub(first);
            egui::ScrollArea::vertical()
                .id_salt(widgets::id(("commandConsole", "log")))
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .max_height(height)
                .show_rows(ui, tokens.console_row_height, rows, |ui, range| {
                    for i in range {
                        let Some(entry) = log.get(first + i) else { break };
                        let color = if entry.is_error() {
                            tokens.error_text
                        } else if matches!(entry.kind, fontcraft_engine::log::LogKind::Module { .. }) {
                            tokens.text_dim
                        } else {
                            tokens.text
                        };
                        ui.add(egui::Label::new(RichText::new(drawn(&entry.to_line())).monospace().color(color)).truncate());
                    }
                });
            ui.separator();

            // Up, Down and Tab belong to the input line while it has the keyboard.
            let id = input_id();
            let mut cursor_to_end = false;
            if ui.memory(|m| m.has_focus(id)) {
                let (up, down, tab) = ui.input_mut(|i| {
                    (
                        i.consume_key(Modifiers::NONE, Key::ArrowUp),
                        i.consume_key(Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(Modifiers::NONE, Key::Tab),
                    )
                });
                if up {
                    st.recall(Recall::Previous);
                    cursor_to_end = true;
                }
                if down {
                    st.recall(Recall::Next);
                    cursor_to_end = true;
                }
                if tab && let Some(line) = command_console::apply(&st.input, completion, 0).filter(|l| l.len() <= MAX_LINE_BYTES) {
                    st.input = line;
                    cursor_to_end = true;
                }
            }
            let edit = egui::TextEdit::singleline(&mut st.input)
                .id(id)
                .char_limit(MAX_LINE_BYTES)
                .lock_focus(true)
                .font(egui::TextStyle::Monospace)
                .hint_text("command {parameters}, or run --cmd ID --params JSON")
                .desired_width(f32::INFINITY);
            let response = ui.add(edit);
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let line = std::mem::take(&mut st.input);
                st.recall = None;
                if !line.trim().is_empty() {
                    pending.push(("window.macroPanel", json!({"line": line})));
                }
                response.request_focus();
            }

            let mut picked = None;
            if let Some((command, doc)) = completion.doc {
                ui.label(RichText::new(drawn(&format!("{command} {doc}"))).color(tokens.text_dim).small());
            }
            if !completion.items.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt(widgets::id(("commandConsole", "completion")))
                    .max_height(tokens.console_completion_height)
                    .show(ui, |ui| {
                        for (i, s) in completion.items.iter().enumerate() {
                            let color = match s.kind {
                                SuggestionKind::Command => tokens.text,
                                SuggestionKind::Glyph => tokens.accent,
                            };
                            let label = egui::Label::new(RichText::new(drawn(&s.text)).monospace().color(color)).sense(egui::Sense::click());
                            if ui.add(label).clicked() {
                                picked = Some(i);
                            }
                        }
                    });
            }
            if let Some(line) = picked.and_then(|i| command_console::apply(&st.input, completion, i)).filter(|l| l.len() <= MAX_LINE_BYTES) {
                st.input = line;
                cursor_to_end = true;
                ui.memory_mut(|m| m.request_focus(id));
            }
            if cursor_to_end && let Some(mut edit_state) = egui::widgets::text_edit::TextEditState::load(ui.ctx(), id) {
                let end = egui::text::CCursor::new(st.input.chars().count());
                edit_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                edit_state.store(ui.ctx(), id);
            }
        });
    if !open {
        pending.push(("window.macroPanel", json!({"open": false})));
    }
}
