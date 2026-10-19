//! Palette › Pairs & Phrases (`palette.pairsAndPhrases`): a work list for kerning and spacing.
//! The commands and the lists live in `shell::kerning_review`.
//!
//! | Input | Does |
//! |---|---|
//! | Pairs, Phrases, Auto | `kerning.setPairListMode` |
//! | path field and Load; Reset | `kerning.loadPairList {path}`, `{reset}` |
//! | click a row | `kerning.stepPairList {index}`: shows it in the tab |
//! | Cmd-click, Shift-click | add a row to the selection, or the rows up to it (`{select}`) |
//! | Up, Down (list focused) | `kerning.stepPairList` previous, next |
//! | Ctrl+Up, Ctrl+Down; Alt+Up, Alt+Down (list focused) | `kerning.stepGroupMember` on the left side; on the right side |
//! | Auto Kern | `kerning.autoKernPairList` on the selected rows, for review; Apply, Discard |
//!
//! Pair rows show the value stored for the pair in the showing tab's master.

use egui::{Key, Modifiers, RichText, Sense};
use fontcraft_spacing::kerning::{Direction, Side, stored_pair};
use serde_json::{Value, json};

use super::{Pending, palette};
use crate::app::FontcraftApp;
use crate::shell::kerning_review::{ListMode, shows_pairs};

/// Rows the list shows before it scrolls.
const VISIBLE_ROWS: f32 = 8.0;
/// Proposal lines the section lists.
const MAX_PROPOSAL_LINES: usize = 200;

/// The list's id, which holds the keyboard for the arrow keys.
pub fn list_id() -> egui::Id {
    egui::Id::new("palette.pairsAndPhrases.list")
}

/// The master whose values the rows show: the showing tab's, else the first.
fn master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    app.state
        .tab()
        .and_then(|t| t.master.clone())
        .filter(|m| font.masters.iter().any(|x| x.id == *m))
        .or_else(|| font.masters.first().map(|m| m.id.clone()))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let pairs = shows_pairs(app);
    let master = master(app);
    let Ok(font) = app.session.doc().map(|d| d.font.clone()) else {
        palette::note(app, ui, "No font");
        return;
    };
    let FontcraftApp { state, tokens, .. } = app;
    let tokens: &crate::theme::Tokens = tokens;
    let st = &mut state.panels.kerning_review;
    let list = if pairs { "pairs" } else { "phrases" };
    ui.horizontal(|ui| {
        for (mode, label) in [(ListMode::Pairs, "Pairs"), (ListMode::Phrases, "Phrases"), (ListMode::Auto, "Auto")] {
            if ui.selectable_label(st.mode == mode, label).clicked() && st.mode != mode {
                pending.push(("kerning.setPairListMode", json!({"mode": mode.name()})));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.path_field).desired_width(tokens.field_width * 2.0).hint_text("list file"));
        if ui.button("Load").clicked() && !st.path_field.trim().is_empty() {
            pending.push(("kerning.loadPairList", json!({"path": st.path_field.trim(), "list": list})));
        }
        if ui.button("Reset").on_hover_text("Go back to the built-in list").clicked() {
            pending.push(("kerning.loadPairList", json!({"reset": true, "list": list})));
        }
    });
    let wl = st.list(pairs);
    let source = wl.source.clone().unwrap_or_else(|| "built-in list".to_string());
    let rows = wl.rows(pairs).to_vec();
    let (current, selected) = (wl.current, wl.selected.clone());
    ui.label(RichText::new(format!("{} rows from {source}", rows.len())).color(tokens.text_dim));
    let region = ui.interact(ui.max_rect(), list_id(), Sense::focusable_noninteractive());
    let filter = egui::EventFilter { vertical_arrows: true, ..Default::default() };
    ui.memory_mut(|m| m.set_focus_lock_filter(list_id(), filter));
    if region.has_focus() {
        keys(ui, list, pending);
    }
    let row_height = tokens.list_row_height;
    egui::ScrollArea::vertical().id_salt("rows").max_height(row_height * VISIBLE_ROWS).auto_shrink([false, true]).show_rows(
        ui,
        row_height,
        rows.len(),
        |ui, range| {
            for i in range {
                let Some(row) = rows.get(i) else { continue };
                let mut text = row.label();
                if row.is_pair()
                    && let (Some(m), Some([l, r])) = (master.as_deref(), row.lines.first().map(Vec::as_slice))
                    && let (Ok(l), Ok(r)) = (Side::parse(l), Side::parse(r))
                    && let Some(p) = stored_pair(&font, Direction::Ltr, m, &l, &r)
                {
                    text = format!("{text}    {}", p.value);
                }
                let mut label = RichText::new(text);
                if current == Some(i) {
                    label = label.strong();
                }
                let response = ui.selectable_label(selected.contains(&i), label);
                if response.clicked() {
                    region.request_focus();
                    let mods = ui.input(|input| input.modifiers);
                    pending.push(click(list, i, &selected, mods));
                }
            }
        },
    );
    ui.horizontal(|ui| {
        if ui.add_enabled(pairs, egui::Button::new("Auto Kern")).on_hover_text("Propose kerning for the selected pairs, for review").clicked() {
            pending.push(("kerning.autoKernPairList", json!({"list": list})));
        }
    });
    if let Some(proposal) = &st.auto_kern {
        ui.label(RichText::new("Proposed kerning").strong());
        let lines: Vec<String> = proposal
            .reply
            .get("pairs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_PROPOSAL_LINES)
            .map(|p| {
                let s = |k: &str| p.get(k).map_or(String::new(), |v| v.as_str().map_or_else(|| v.to_string(), str::to_string));
                format!("{} {}  {}  (was {})  {}", s("left"), s("right"), s("value"), s("previous"), s("result"))
            })
            .collect();
        for line in lines {
            ui.label(RichText::new(line).color(tokens.text));
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                pending.push(("kerning.autoKernPairList", json!({"apply": true})));
            }
            if ui.button("Discard").clicked() {
                pending.push(("kerning.autoKernPairList", json!({"discard": true})));
            }
        });
    }
}

/// The command a click on row `i` runs: a plain click shows the row; Cmd adds it to or takes it
/// out of the selection; Shift selects the rows from the last selected one to it.
fn click(list: &str, i: usize, selected: &[usize], mods: Modifiers) -> (&'static str, Value) {
    if mods.command {
        let mut next: Vec<usize> = selected.iter().copied().filter(|s| *s != i).collect();
        if next.len() == selected.len() {
            next.push(i);
        }
        return ("kerning.stepPairList", json!({"select": next, "list": list}));
    }
    if mods.shift
        && let Some(&from) = selected.last()
    {
        let range: Vec<usize> = if from <= i { (from..=i).collect() } else { (i..=from).rev().collect() };
        return ("kerning.stepPairList", json!({"select": range, "list": list}));
    }
    ("kerning.stepPairList", json!({"index": i, "list": list}))
}

/// Up and Down step the list; with Ctrl or Alt they step the representative glyph instead.
fn keys(ui: &egui::Ui, list: &str, pending: &mut Pending) {
    let (up, down, mods) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.modifiers));
    let direction = match (up, down) {
        (true, false) => "previous",
        (false, true) => "next",
        _ => return,
    };
    if mods.ctrl {
        pending.push(("kerning.stepGroupMember", json!({"side": "left", "direction": direction})));
    } else if mods.alt {
        pending.push(("kerning.stepGroupMember", json!({"side": "right", "direction": direction})));
    } else {
        pending.push(("kerning.stepPairList", json!({"direction": direction, "list": list})));
    }
}
