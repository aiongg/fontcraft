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

use serde_json::{Value, json};

use super::{Pending, current_glyph, palette, widgets};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::shell::context_menu;

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
        _ => None,
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
    let row_menu = context_menu::sections(&app.session, &app.state, &[ROW_CONTEXT]);
    for row in &shown {
        ui.horizontal(|ui| {
            let eye = if row.visible { "👁" } else { " " };
            let hover = if row.visible { "Hide Layer" } else { "Show Layer" };
            if widgets::button(ui, widgets::id(("layers.eye", &row.id)), eye, hover, true, false) {
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
            let mut text = egui::RichText::new(if row.name.is_empty() { "Untitled" } else { row.name.as_str() });
            if master_row {
                text = text.strong();
            }
            if row.id == active_layer {
                text = text.color(tokens.accent);
            }
            let label = ui.add(egui::Button::selectable(selected.contains(&row.id), text).frame(false));
            if !master_row && row.kind != "backup" {
                ui.label(egui::RichText::new(&row.kind).strong().color(tokens.text_dim));
            }
            if label.clicked() {
                if editing {
                    let m = ui.input(|i| i.modifiers);
                    let params = match (m.shift, m.command) {
                        (true, _) => json!({"layer": row.id, "extend": "range"}),
                        (false, true) => json!({"layer": row.id, "extend": "toggle"}),
                        _ => json!({"layer": row.id}),
                    };
                    pending.push(("layers.select", params));
                } else {
                    ctx.data_mut(|d| d.insert_temp(picked_id(), row.id.clone()));
                }
            }
            if label.double_clicked() && !master_row {
                start_renaming(&ctx, &row.id);
            }
            label.context_menu(|ui| {
                for (index, section) in row_menu.iter().enumerate() {
                    if index > 0 {
                        ui.separator();
                    }
                    for item in &section.items {
                        let why = item.disabled_reason.clone().or_else(|| row_refuses(item.command, row));
                        let response = ui.add_enabled(why.is_none(), egui::Button::new(item.label));
                        let response = match &why {
                            Some(why) => response.on_disabled_hover_text(why),
                            None => response,
                        };
                        if response.clicked() {
                            match item.command {
                                "layers.rename" => start_renaming(&ctx, &row.id),
                                id => pending.push((id, row_params(id, &glyph, &row.id))),
                            }
                            ui.close();
                        }
                    }
                }
            });
        });
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
