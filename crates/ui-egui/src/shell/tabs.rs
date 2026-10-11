//! Tabs: Font View is the fixed first tab, Edit View tabs follow (`tabs.windowHoldsFontViewPlusEditTabs`).
//!
//! Tab commands name tabs by **position** in the strip, counting from 1 for Font View, as the
//! reference's `Cmd+Alt+1` … `Cmd+Alt+9` do. View commands name a tab by its stable `id`.

use egui::{Color32, RichText};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::state::{MAX_CLOSED_TABS, MAX_TAB_GLYPHS, MAX_TABS, Tab, TabText};

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.newTab",
        "New Tab",
        &["View"],
        "Cmd+T",
        "{glyphs?: [names], text?: string} open a new Edit View tab with these glyphs (default: the selected glyphs); `text` names them by their characters",
    ),
    ("view.showTab", "Show Tab", &[], "Cmd+Alt+2", "{index: integer} show the tab at this position (1 is Font View)"),
    ("view.showNextTab", "Show Next Tab", &["View", "Navigation"], "Ctrl+Tab", "{} show the tab to the right (after the last: Font View)"),
    (
        "view.showPreviousTab",
        "Show Previous Tab",
        &["View", "Navigation"],
        "Ctrl+Shift+Tab",
        "{} show the tab to the left (before Font View: the last tab)",
    ),
    (
        "view.closeTab",
        "Close Tab",
        &["View"],
        "Cmd+Shift+W",
        "{index?: integer} close the tab at this position (default: the showing tab; Font View cannot be closed)",
    ),
    (
        "view.closeOtherTabs",
        "Close Other Tabs",
        &[],
        "",
        "{keep?: integer} close every Edit View tab but the one at this position (default: the showing tab)",
    ),
    ("view.reopenLastClosedTab", "Reopen Last Closed Tab", &["View"], "Cmd+Alt+Shift+W", "{} bring back the most recently closed tab where it was"),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let state = &mut app.state;
    let positions = state.tabs.len() + 1;
    let current = state.active_tab.map_or(1, |i| i + 2);
    match id {
        "view.newTab" => {
            let glyphs = match (args.strings("glyphs", MAX_TAB_GLYPHS)?, args.string("text")?) {
                (Some(_), Some(_)) => return Err(args.err("give `glyphs` or `text`, not both")),
                (Some(list), None) => list,
                (None, Some(text)) => {
                    let font = app.session.font().map_err(|e| e.to_string())?;
                    crate::state::glyphs_for_text(font, text).map_err(|e| args.err(e))?
                }
                (None, None) => match state.editing() {
                    Some(g) if state.active_tab.is_some() => vec![g.to_string()],
                    _ => state.selected_glyphs.iter().take(MAX_TAB_GLYPHS).cloned().collect(),
                },
            };
            if glyphs.is_empty() {
                return Err(args.err("no glyph is selected"));
            }
            open(app, args, glyphs)
        }
        "view.showTab" => {
            let index = args.index("index")?.ok_or_else(|| args.err("`index` (a tab position, 1 for Font View) is required"))?;
            let index = position(args, index, positions)?;
            state.active_tab = index.checked_sub(2);
            Ok(describe_position(app, index))
        }
        "view.showNextTab" | "view.showPreviousTab" => {
            let next = if id == "view.showNextTab" { current % positions + 1 } else { (current + positions - 2) % positions + 1 };
            state.active_tab = next.checked_sub(2);
            Ok(describe_position(app, next))
        }
        "view.closeTab" => {
            let index = match args.index("index")? {
                Some(i) => position(args, i, positions)?,
                None => current,
            };
            let Some(at) = index.checked_sub(2) else { return Err(args.err("Font View cannot be closed")) };
            close(app, at);
            Ok(json!({"closed": index, "tabs": app.state.tabs.len()}))
        }
        "view.closeOtherTabs" => {
            let keep = match args.index("keep")? {
                Some(i) => position(args, i, positions)?,
                None => current,
            };
            let keep_id = keep.checked_sub(2).and_then(|i| state.tabs.get(i)).map(|t| t.id);
            let mut at = state.tabs.len();
            while at > 0 {
                at -= 1;
                if app.state.tabs.get(at).map(|t| t.id) != keep_id {
                    close(app, at);
                }
            }
            app.state.active_tab = keep_id.and_then(|id| app.state.tab_index(id));
            Ok(json!({"tabs": app.state.tabs.len()}))
        }
        "view.reopenLastClosedTab" => {
            let (at, tab) = state.closed_tabs.pop().ok_or_else(|| args.err("no tab was closed"))?;
            if state.tabs.len() >= MAX_TABS {
                state.closed_tabs.push((at, tab));
                return Err(args.err(format!("at most {MAX_TABS} tabs can be open")));
            }
            let at = at.min(state.tabs.len());
            state.tabs.insert(at, tab);
            state.active_tab = Some(at);
            Ok(describe(app, at))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// A tab position from a parameter: 1 to `positions`.
fn position(args: &Args, index: u64, positions: usize) -> Result<usize, String> {
    usize::try_from(index)
        .ok()
        .filter(|i| (1..=positions).contains(i))
        .ok_or_else(|| args.err(format!("there is no tab at position {index} (1 is Font View; {positions} positions now)")))
}

/// Closes the tab at `at` (an index into the tabs) and keeps it for reopening. The tab to its
/// left shows when it was the showing one.
fn close(app: &mut FontcraftApp, at: usize) {
    let state = &mut app.state;
    if at >= state.tabs.len() {
        return;
    }
    let tab = state.tabs.remove(at);
    state.closed_tabs.push((at, tab));
    if state.closed_tabs.len() > MAX_CLOSED_TABS {
        state.closed_tabs.remove(0);
    }
    state.active_tab = match state.active_tab {
        Some(a) if a == at => at.checked_sub(1),
        Some(a) if a > at => Some(a - 1),
        other => other,
    };
}

/// Opens `glyphs` in a new tab, fitted to the canvas, and shows it.
pub(crate) fn open(app: &mut FontcraftApp, args: &Args, glyphs: Vec<String>) -> Result<Value, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    if let Some(missing) = glyphs.iter().find(|g| font.glyph(g).is_none()) {
        return Err(args.err(format!("no glyph named `{missing}`")));
    }
    if app.state.tabs.len() >= MAX_TABS {
        return Err(args.err(format!("at most {MAX_TABS} tabs can be open")));
    }
    let id = app.state.next_tab_id;
    app.state.next_tab_id += 1;
    // A new tab draws in the master of the tab it was opened from, or Font View's.
    let master = match app.state.tab() {
        Some(t) => t.master.clone(),
        None => app.state.font_view.master.clone(),
    };
    let text = TabText::from_glyphs(&glyphs).map_err(|e| args.err(e))?;
    Ok(push(app, Tab { id, text, master, ..Default::default() }))
}

/// Adds `tab` at the end of the strip, fitted to the canvas, shows it and describes it.
pub(crate) fn push(app: &mut FontcraftApp, mut tab: Tab) -> Value {
    if let Some(doc) = app.session.active() {
        tab.document = doc.id;
    }
    tab.view = view::fit(&app.session, &app.layouts, &tab, app.state.canvas_size, f64::from(app.tokens.fit_margin));
    app.state.tabs.push(tab);
    let index = app.state.tabs.len() - 1;
    app.state.active_tab = Some(index);
    describe(app, index)
}

/// What tab commands return about the tab at `index` (into the tabs).
pub(crate) fn describe(app: &FontcraftApp, index: usize) -> Value {
    match app.state.tabs.get(index) {
        Some(tab) => {
            let font = app.session.font().ok();
            let master = font.and_then(|f| view::active_master(f, tab)).map(|m| m.id.clone());
            let layer = font.and_then(|f| view::active_layer(f, tab)).map(|l| l.id.clone());
            let glyphs: Vec<&str> = tab.shown_glyphs().collect();
            json!({"tab": tab.id, "position": index + 2, "glyphs": glyphs, "editing": tab.edited_glyph(), "master": master, "layer": layer})
        }
        None => json!({}),
    }
}

fn describe_position(app: &FontcraftApp, position: usize) -> Value {
    match position.checked_sub(2) {
        Some(index) => describe(app, index),
        None => json!({"position": 1}),
    }
}

/// The tab strip under the menu bar: Font View, then one title per Edit View tab with a close
/// button. Alt-clicking a close button closes the other tabs.
pub fn strip(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    let tokens = &app.tokens;
    let frame = egui::Frame::NONE.fill(tokens.panel_darker).inner_margin(egui::Margin::symmetric(4, 2));
    egui::Panel::top("tab_strip").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            let tab_button = |ui: &mut egui::Ui, title: &str, active: bool| {
                let (fill, text) = if active { (tokens.row_selected, tokens.selected_text) } else { (tokens.panel, tokens.text) };
                ui.add(
                    egui::Button::new(RichText::new(title).color(text).size(tokens.font_size))
                        .fill(fill)
                        .min_size(egui::vec2(tokens.tab_min_width, 0.0)),
                )
            };
            if tab_button(ui, "Font View", app.state.active_tab.is_none()).clicked() {
                pending.push(("view.showTab", json!({"index": 1})));
            }
            for (i, tab) in app.state.tabs.iter().enumerate() {
                let position = i + 2;
                if tab_button(ui, &tab.title(), app.state.active_tab == Some(i)).clicked() {
                    pending.push(("view.showTab", json!({"index": position})));
                }
                let close = ui.add(egui::Button::new(RichText::new("×").color(tokens.text_dim)).fill(Color32::TRANSPARENT).frame(false));
                if close.clicked() {
                    if ui.input(|i| i.modifiers.alt) {
                        pending.push(("view.closeOtherTabs", json!({"keep": position})));
                    } else {
                        pending.push(("view.closeTab", json!({"index": position})));
                    }
                }
            }
        });
    });
}
