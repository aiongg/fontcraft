//! Palette › Layers: the layers of the current glyph in palette order (`layers.list`), master
//! layers in bold, the others indented under their master with their kind, the active layer in
//! the accent colour. The eye shows or hides a layer (`layers.toggleVisibility`). In Edit View a
//! click makes a row the active layer (`layers.select`), Shift+click selects a range of rows and
//! Cmd+click adds or removes one; in Font View a click picks the row the buttons act on.
//! Double-clicking a non-master name renames it (`layers.rename`). Plus adds a backup of the
//! active layer (`layers.addBackupLayer`), minus deletes the selected rows (`layers.delete`), Use
//! as Master restores a selected backup (`layers.useAsMaster`). The filter menu runs
//! `layers.onlyShowLayersFromCurrentMaster` and `layers.hideBackupLayers`. A right-click on a
//! row opens its context menu: the commands whose row names `layers.row`, acting on that row.
//! With glyphs selected in the text (text mode), a click on a master row switches those glyphs and
//! the edited glyph to that master (`layers.select` with `glyphs`).
//!
//! A non-master row can be dragged by its name. Dropped on another row of the same master it moves
//! there (`layers.reorder`); a line shows where it will land. A backup dropped on its master's row
//! takes the master's place (`layers.useAsMaster`), and the master row is outlined while it
//! hovers; any other row dropped on its master row moves to the top of that master. A drop under
//! another master is refused: the cursor shows it and nothing runs.
//!
//! Special layers (`mastersAsLayers.paletteListing`) show their label in bold in place of the
//! name: `[…]` for an alternate layer, `{…}` for an intermediate one, both for a layer that is
//! both (`fontcraft_interp::labels`). A master layer with a range shows its `[…]` after the
//! master's name. Double-clicking a part of the label opens its editor
//! (`layers.editSpecialLayerSettings`). In the context menu, Intermediate and Alternate are
//! ticked while the layer has that status: choosing one opens its editor, unticking it removes
//! the status (`layers.clearIntermediate`, `layers.clearAlternate`), so those two commands have
//! no item of their own.

use std::collections::{HashMap, HashSet};

use fontcraft_engine::interp::labels::{brace_label, bracket_label};
use serde_json::{Value, json};

use super::{Pending, current_glyph, palette, widgets};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::shell::context_menu::{self, Section};

/// Most rows the section lists.
const MAX_ROWS: usize = 500;

/// One row of `layers.list`.
pub struct Row {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub master: String,
    pub visible: bool,
}

fn rows(result: &Value) -> Vec<Row> {
    let Some(list) = result.get("layers").and_then(Value::as_array) else { return Vec::new() };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    list.iter()
        .take(MAX_ROWS)
        .map(|v| Row {
            id: s(v, "id"),
            name: s(v, "name"),
            kind: s(v, "kind"),
            master: s(v, "master"),
            visible: v.get("visible").and_then(Value::as_bool).unwrap_or(true),
        })
        .collect()
}

/// The rows of `glyph` the section lists, after its filters. `tab` (an index into the tabs) is the
/// tab whose active master "Only Show Layers from Current Master" means; without one, the first
/// master.
pub fn shown_rows(app: &mut FontcraftApp, tab: Option<usize>, glyph: &str) -> Vec<Row> {
    let Ok(result) = app.session.execute("layers.list", &json!({ "glyph": glyph })) else { return Vec::new() };
    let current = current_master(app, tab);
    let panels = &app.state.panels;
    rows(&result)
        .into_iter()
        .filter(|r| !panels.layers_current_master_only || Some(&r.master) == current.as_ref())
        .filter(|r| !panels.hide_backup_layers || r.kind != "backup")
        .collect()
}

/// The id of the current master: the active master of tab `tab`, else the first master.
fn current_master(app: &FontcraftApp, tab: Option<usize>) -> Option<String> {
    let font = app.session.font().ok()?;
    match tab.and_then(|i| app.state.tabs.get(i)) {
        Some(tab) => view::active_master(font, tab),
        None => font.masters.first(),
    }
    .map(|m| m.id.clone())
}

/// The id of the row picked by a click in Font View (egui memory: widget state, not document
/// state).
fn picked_id() -> egui::Id {
    widgets::id("layers.picked")
}

/// The id of the layer being renamed.
fn renaming_id() -> egui::Id {
    widgets::id("layers.renaming")
}

/// The rename field of layer `layer`.
pub fn rename_field(layer: &str) -> egui::Id {
    widgets::id(("layers.rename", layer))
}

/// The `context` value of the Layers rows' context menu.
pub const ROW_CONTEXT: &str = "layers.row";

fn start_renaming(ctx: &egui::Context, layer: &str) {
    ctx.data_mut(|d| d.insert_temp(renaming_id(), layer.to_string()));
    ctx.memory_mut(|m| m.request_focus(rename_field(layer)));
}

/// Why command `id` of the row menu cannot act on `row`, if it cannot.
fn row_refuses(id: &str, row: &Row) -> Option<String> {
    match id {
        "layers.delete" | "layers.rename" if row.kind == "master" => Some("a master layer goes with its master".into()),
        "layers.useAsMaster" if row.kind != "backup" => Some("only a backup layer can become the master layer".into()),
        "layers.setIntermediate" if row.kind == "master" => Some("a master layer cannot be an intermediate layer".into()),
        _ => None,
    }
}

/// The UI command that opens a special layer's editor.
const EDIT_SPECIAL: &str = "layers.editSpecialLayerSettings";

/// The row menu's ticked items: the command that sets a status, the one that removes it, and
/// the part of the label it edits.
const STATUSES: [(&str, &str, &str); 2] =
    [("layers.setIntermediate", "layers.clearIntermediate", "intermediate"), ("layers.setAlternate", "layers.clearAlternate", "alternate")];

/// The label parts of a special layer.
struct Special {
    /// `[…]`: the alternate layer's ranges.
    bracket: Option<String>,
    /// `{…}`: the intermediate layer's coordinates.
    brace: Option<String>,
}

impl Special {
    /// Whether the layer has the status `part` names.
    fn has(&self, part: &str) -> bool {
        match part {
            "intermediate" => self.brace.is_some(),
            "alternate" => self.bracket.is_some(),
            _ => false,
        }
    }
}

/// A clickable part of a row's label: the text with a button's padding, under the stable `id`.
/// `drag`: the part can also be dragged (the row's drag handle).
fn label_part(ui: &mut egui::Ui, id: egui::Id, text: egui::RichText, drag: bool) -> egui::Response {
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let padding = ui.spacing().button_padding;
    let (rect, _) = ui.allocate_exact_size(galley.size() + 2.0 * padding, egui::Sense::hover());
    let sense = if drag { egui::Sense::click_and_drag() } else { egui::Sense::click() };
    let response = ui.interact(rect, id, sense);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            let visuals = ui.style().interact(&response);
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        ui.painter().galley(rect.min + padding, galley, ui.visuals().text_color());
    }
    response
}

/// The name part of row `layer`: the text a click selects and a drag picks up.
pub fn name_part(layer: &str) -> egui::Id {
    widgets::id(("layers.name", layer))
}

/// The drop zone of row `layer`: the row across the section's width, down to half the gap to its
/// neighbours.
pub fn drop_zone(layer: &str) -> egui::Id {
    widgets::id(("layers.drop", layer))
}

/// What a dragged row carries.
#[derive(Clone, Debug)]
struct Dragged {
    glyph: String,
    layer: String,
    master: String,
    backup: bool,
}

/// What dropping a dragged row on a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropAction {
    /// `layers.reorder` to `to`; the line goes above the row (`above`) or below it.
    Reorder { to: usize, above: bool },
    /// `layers.useAsMaster`: a backup dropped on its master's row.
    UseAsMaster,
    /// A drop under another master (or from another glyph's rows).
    Refused,
    /// Dropped where it already is.
    Nothing,
}

/// What dropping `dragged` on `target` does, with the pointer in the row's upper half (`upper`)
/// or its lower half. `all` is every row of `glyph`, unfiltered: `layers.reorder` counts its index
/// among all the non-master layers of the master, hidden ones included.
fn drop_action(all: &[Row], glyph: &str, dragged: &Dragged, target: &Row, upper: bool) -> DropAction {
    if dragged.glyph != glyph || target.master != dragged.master {
        return DropAction::Refused;
    }
    if target.id == dragged.layer {
        return DropAction::Nothing;
    }
    let siblings: Vec<&str> = all.iter().filter(|r| r.master == dragged.master && r.kind != "master").map(|r| r.id.as_str()).collect();
    let Some(from) = siblings.iter().position(|id| *id == dragged.layer) else { return DropAction::Refused };
    let (slot, above) = if target.kind == "master" {
        if dragged.backup {
            return DropAction::UseAsMaster;
        }
        (0, false)
    } else {
        let Some(at) = siblings.iter().position(|id| *id == target.id) else { return DropAction::Refused };
        (if upper { at } else { at + 1 }, upper)
    };
    // `slot` counts the dragged row; the command's index counts the rows without it.
    let to = if from < slot { slot - 1 } else { slot };
    if to == from { DropAction::Nothing } else { DropAction::Reorder { to, above } }
}

/// The drop zone of `row` (drawn in `row_rect`): shows what a drop there would do while a row is
/// held over it, and queues the command when it is dropped.
fn drop_target(ui: &mut egui::Ui, row: &Row, row_rect: egui::Rect, all: &[Row], glyph: &str, tokens: &crate::theme::Tokens, pending: &mut Pending) {
    let zone = row_rect.expand2(egui::vec2(0.0, ui.spacing().item_spacing.y / 2.0));
    let response = ui.interact(zone, drop_zone(&row.id), egui::Sense::hover());
    let ctx = ui.ctx().clone();
    let upper = ctx.pointer_interact_pos().is_some_and(|p| p.y < zone.center().y);
    if let Some(dragged) = response.dnd_hover_payload::<Dragged>() {
        let stroke = egui::Stroke::new(tokens.selection_width, tokens.accent);
        match drop_action(all, glyph, &dragged, row, upper) {
            DropAction::Reorder { above, .. } => {
                ui.painter().hline(zone.x_range(), if above { zone.top() } else { zone.bottom() }, stroke);
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            DropAction::UseAsMaster => {
                ui.painter().rect_stroke(row_rect, f32::from(tokens.radius), stroke, egui::StrokeKind::Inside);
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            DropAction::Refused => ctx.set_cursor_icon(egui::CursorIcon::NoDrop),
            DropAction::Nothing => ctx.set_cursor_icon(egui::CursorIcon::Grabbing),
        }
    }
    if let Some(dragged) = response.dnd_release_payload::<Dragged>() {
        match drop_action(all, glyph, &dragged, row, upper) {
            DropAction::Reorder { to, .. } => pending.push(("layers.reorder", json!({"glyph": glyph, "layer": dragged.layer, "toIndex": to}))),
            DropAction::UseAsMaster => pending.push(("layers.useAsMaster", json!({"glyphs": [glyph], "layer": dragged.layer}))),
            DropAction::Refused | DropAction::Nothing => {}
        }
    }
}

/// The context menu of `row`: the items of `sections`, acting on that row.
fn row_menu(ui: &mut egui::Ui, sections: &[Section], row: &Row, special: Option<&Special>, glyph: &str, ctx: &egui::Context, pending: &mut Pending) {
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            ui.separator();
        }
        for item in section.items.iter().filter(|i| !STATUSES.iter().any(|(_, clear, _)| *clear == i.command)) {
            let status = STATUSES.iter().find(|(set, _, _)| *set == item.command);
            let ticked = status.map(|(_, _, part)| special.is_some_and(|s| s.has(part)));
            let why = item.disabled_reason.clone().or_else(|| row_refuses(item.command, row));
            let button = match ticked {
                Some(on) => egui::Button::selectable(on, item.label),
                None => egui::Button::new(item.label),
            };
            let response = ui.add_enabled(why.is_none(), button);
            let response = match &why {
                Some(why) => response.on_disabled_hover_text(why),
                None => response,
            };
            if response.clicked() {
                match (item.command, status, ticked) {
                    ("layers.rename", _, _) => start_renaming(ctx, &row.id),
                    (_, Some(&(_, clear, _)), Some(true)) => pending.push((clear, json!({"glyph": glyph, "layer": row.id}))),
                    (_, Some(&(_, _, part)), _) => pending.push((EDIT_SPECIAL, json!({"glyph": glyph, "layer": row.id, "part": part}))),
                    (id, _, _) => pending.push((id, row_params(id, glyph, &row.id))),
                }
                ui.close();
            }
        }
    }
}

/// The parameters command `id` of the row menu runs with on `layer`.
fn row_params(id: &str, glyph: &str, layer: &str) -> Value {
    match id {
        "layers.delete" => json!({"glyphs": [glyph], "layers": [layer]}),
        "layers.useAsMaster" => json!({"glyphs": [glyph], "layer": layer}),
        _ => json!({"glyph": glyph, "layer": layer}),
    }
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(glyph) = current_glyph(app) else {
        palette::note(app, ui, "No glyph selected");
        return;
    };
    let ctx = ui.ctx().clone();
    // In Edit View the rows belong to the showing tab; in Font View to the first master.
    let tab = app.state.active_tab;
    let Some((active_master, active_layer)) = app.session.font().ok().and_then(|font| {
        let layer = match app.state.tab() {
            Some(tab) => view::active_layer(font, tab)?,
            None => font.glyph(&glyph)?.layer(&font.masters.first()?.id)?,
        };
        Some((layer.master.clone(), layer.id.clone()))
    }) else {
        palette::note(app, ui, "No glyph selected");
        return;
    };
    let all = match app.session.execute("layers.list", &json!({ "glyph": glyph })) {
        Ok(result) => rows(&result),
        Err(_) => Vec::new(),
    };
    let shown_ids: Vec<String> = shown_rows(app, tab, &glyph).into_iter().map(|r| r.id).collect();
    let shown: Vec<&Row> = all.iter().filter(|r| shown_ids.contains(&r.id)).collect();
    let specials: HashMap<String, Special> = app
        .session
        .font()
        .ok()
        .and_then(|font| {
            let wanted: HashSet<&str> = shown.iter().map(|r| r.id.as_str()).collect();
            let g = font.glyph(&glyph)?;
            Some(
                g.layers
                    .iter()
                    .filter(|l| wanted.contains(l.id.as_str()))
                    .map(|l| (l.id.clone(), Special { bracket: bracket_label(font, l), brace: brace_label(font, l) }))
                    .filter(|(_, s)| s.bracket.is_some() || s.brace.is_some())
                    .collect(),
            )
        })
        .unwrap_or_default();
    // Rows a command removed drop out; an Edit View tab with none selected shows the active one.
    let mut selected: Vec<String> = match app.state.tab() {
        Some(t) => t.layer_selection.clone(),
        None => ctx.data(|d| d.get_temp::<String>(picked_id())).into_iter().collect(),
    };
    selected.retain(|id| all.iter().any(|r| r.id == *id));
    if selected.is_empty() && tab.is_some() {
        selected.push(active_layer.clone());
    }
    let editing = tab.is_some();
    let renaming: Option<String> = ctx.data(|d| d.get_temp(renaming_id()));
    let tokens = app.tokens.clone();
    let row_menu_sections = context_menu::sections(&app.session, &app.state, &[ROW_CONTEXT]);
    let colors = super::layers_color::ColorEntries::read(app, &glyph);
    // With glyphs selected in the text, a click on a master row switches those of them that have
    // the master's layer, and the edited glyph, whose rows these are.
    let mut text_glyphs: Vec<String> = app
        .state
        .tab()
        .filter(|t| t.text_mode)
        .map(|t| {
            let mut seen = HashSet::new();
            t.text.selection().filter_map(|i| t.item_glyph(i)).filter(|g| seen.insert(*g)).map(str::to_string).collect()
        })
        .unwrap_or_default();
    if !text_glyphs.is_empty() && !text_glyphs.contains(&glyph) {
        text_glyphs.push(glyph.clone());
    }
    let master_targets: HashMap<String, Vec<String>> = match app.session.font() {
        Ok(font) if !text_glyphs.is_empty() => shown
            .iter()
            .filter(|r| r.kind == "master")
            .map(|r| {
                let has = |g: &&String| font.glyph(g).is_some_and(|g| g.layer(&r.id).is_some());
                (r.id.clone(), text_glyphs.iter().filter(has).cloned().collect::<Vec<_>>())
            })
            .filter(|(_, glyphs)| !glyphs.is_empty())
            .collect(),
        _ => HashMap::new(),
    };
    for row in &shown {
        // A selected row is filled across the section's full width, behind the eye and the kind.
        let fill = ui.painter().add(egui::Shape::Noop);
        let full_width = ui.available_rect_before_wrap().x_range();
        let is_selected = selected.contains(&row.id);
        let drawn = ui.horizontal(|ui| {
            let eye = if row.visible { "👁" } else { " " };
            let hover = if row.visible { "Hide Layer" } else { "Show Layer" };
            if widgets::flat_button(ui, widgets::id(("layers.eye", &row.id)), eye, hover) {
                pending.push(("layers.toggleVisibility", json!({"glyphs": [glyph], "layer": row.id})));
            }
            let master_row = row.kind == "master";
            if !master_row {
                ui.add_space(tokens.layers_indent);
            }
            if renaming.as_deref() == Some(row.id.as_str()) {
                let field = rename_field(&row.id);
                let width = ui.available_width();
                if let Some(name) = widgets::text(ui, field, &row.name, width) {
                    pending.push(("layers.rename", json!({"glyph": glyph, "layer": row.id, "name": name.trim()})));
                }
                if !ctx.memory(|m| m.has_focus(field)) {
                    ctx.data_mut(|d| d.remove::<String>(renaming_id()));
                }
                return;
            }
            let special = specials.get(&row.id);
            // On a selected row the active layer's text is underlined; elsewhere it is in the
            // accent colour.
            let style = |text: egui::RichText| match (is_selected, row.id == active_layer) {
                (true, true) if selected.len() > 1 => text.color(tokens.selected_text).underline(),
                (true, _) => text.color(tokens.selected_text),
                (false, true) => text.color(tokens.accent),
                (false, false) => text,
            };
            // What the row shows, each with the part of a special layer's label it is (`None`:
            // the name).
            let mut parts: Vec<(egui::Response, Option<&'static str>)> = Vec::new();
            if master_row || special.is_none() {
                let mut text = egui::RichText::new(if row.name.is_empty() { "Untitled" } else { row.name.as_str() });
                if master_row {
                    text = text.strong();
                }
                parts.push((label_part(ui, name_part(&row.id), style(text), !master_row), None));
            }
            if let Some(s) = special {
                for (label, key, part) in [(&s.bracket, "layers.bracket", "alternate"), (&s.brace, "layers.brace", "intermediate")] {
                    if let Some(label) = label {
                        let id = widgets::id((key, &row.id));
                        parts.push((label_part(ui, id, style(egui::RichText::new(label).strong()), !master_row), Some(part)));
                    }
                }
            } else if !master_row && row.kind != "backup" && !colors.show(ui, &tokens, &row.id, is_selected, pending) {
                ui.label(egui::RichText::new(&row.kind).strong().color(if is_selected { tokens.selected_text } else { tokens.text_dim }));
            }
            for (response, part) in &parts {
                if !master_row && response.drag_started_by(egui::PointerButton::Primary) {
                    let dragged = Dragged { glyph: glyph.clone(), layer: row.id.clone(), master: row.master.clone(), backup: row.kind == "backup" };
                    response.dnd_set_drag_payload(dragged);
                }
                if response.clicked() {
                    if editing {
                        let m = ui.input(|i| i.modifiers);
                        let params = match (m.shift, m.command, master_targets.get(&row.id)) {
                            (true, _, _) => json!({"layer": row.id, "extend": "range"}),
                            (false, true, _) => json!({"layer": row.id, "extend": "toggle"}),
                            (false, false, Some(glyphs)) => json!({"layer": row.id, "glyphs": glyphs}),
                            _ => json!({"layer": row.id}),
                        };
                        pending.push(("layers.select", params));
                    } else {
                        ctx.data_mut(|d| d.insert_temp(picked_id(), row.id.clone()));
                    }
                }
                if response.double_clicked() {
                    match part {
                        Some(part) => pending.push((EDIT_SPECIAL, json!({"glyph": glyph, "layer": row.id, "part": part}))),
                        None if !master_row => start_renaming(&ctx, &row.id),
                        None => {}
                    }
                }
                response.context_menu(|ui| row_menu(ui, &row_menu_sections, row, special, &glyph, &ctx, pending));
            }
        });
        let row_rect = egui::Rect::from_x_y_ranges(full_width, drawn.response.rect.y_range());
        if is_selected {
            ui.painter().set(fill, egui::Shape::rect_filled(row_rect, f32::from(tokens.radius), tokens.row_selected));
        }
        drop_target(ui, row, row_rect, &all, &glyph, &tokens, pending);
    }
    let selected_rows: Vec<&Row> = all.iter().filter(|r| selected.contains(&r.id)).collect();
    let deletable: Vec<&str> = selected_rows.iter().filter(|r| r.kind != "master").map(|r| r.id.as_str()).collect();
    let backup = match selected_rows.as_slice() {
        [one] if one.kind == "backup" => Some(*one),
        _ => None,
    };
    ui.horizontal(|ui| {
        if widgets::button(ui, widgets::id("layers.add"), "+", "New Backup Layer", true, false) {
            pending.push(("layers.addBackupLayer", json!({"glyphs": [glyph], "master": active_master, "from": active_layer})));
        }
        if widgets::button(ui, widgets::id("layers.delete"), "−", "Delete Layer", !deletable.is_empty(), false) {
            pending.push(("layers.delete", json!({"glyphs": [glyph], "layers": deletable})));
            ctx.data_mut(|d| d.remove::<String>(picked_id()));
        }
        if widgets::button(ui, widgets::id("layers.useAsMaster"), "Use as Master", "Use as Master", backup.is_some(), false)
            && let Some(r) = backup
        {
            pending.push(("layers.useAsMaster", json!({"glyphs": [glyph], "layer": r.id})));
        }
        ui.menu_button("Filter", |ui| {
            let (current, backups) = (app.state.panels.layers_current_master_only, app.state.panels.hide_backup_layers);
            if ui.add(egui::Button::selectable(current, "Only Show Layers from Current Master")).clicked() {
                pending.push(("layers.onlyShowLayersFromCurrentMaster", json!({ "on": !current })));
            }
            if ui.add(egui::Button::selectable(backups, "Hide Backup Layers")).clicked() {
                pending.push(("layers.hideBackupLayers", json!({ "on": !backups })));
            }
        });
    });
}
