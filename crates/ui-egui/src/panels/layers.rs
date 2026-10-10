//! Palette › Layers: the layers of the current glyph in palette order (`layers.list`), master
//! layers in bold, the others indented under their master with their kind. The eye shows or
//! hides a layer (`layers.toggleVisibility`); a click picks the row the buttons act on;
//! double-clicking a non-master name renames it (`layers.rename`). Plus adds a backup of the
//! active layer (`layers.addBackupLayer`), minus deletes the picked row (`layers.delete`), Use as
//! Master restores a picked backup (`layers.useAsMaster`). The filter menu runs
//! `layers.onlyShowLayersFromCurrentMaster` and `layers.hideBackupLayers`. A right-click on a
//! row opens its context menu: the commands whose row names `layers.row`, acting on that row.
//!
//! The active layer is the edited glyph's layer of the first master, as in the canvas.

use serde_json::{Value, json};

use super::{Pending, current_glyph, palette, widgets};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::shell::context_menu;

/// Most rows the section lists.
const MAX_ROWS: usize = 500;

/// One row of `layers.list`.
struct Row {
    id: String,
    name: String,
    kind: String,
    master: String,
    visible: bool,
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

/// The id of the row picked by a click (egui memory: widget state, not document state).
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
    let Ok(result) = app.session.execute("layers.list", &json!({ "glyph": glyph })) else {
        palette::note(app, ui, "No glyph selected");
        return;
    };
    let Some((active_master, active_layer)) = app.session.font().ok().and_then(|font| {
        let master = view::active_master(font)?;
        let layer = font.glyph(&glyph)?.layer(&master.id)?;
        Some((master.id.clone(), layer.id.clone()))
    }) else {
        return;
    };
    let panels = &app.state.panels;
    let all = rows(&result);
    let shown: Vec<&Row> = all
        .iter()
        .filter(|r| !panels.layers_current_master_only || r.master == active_master)
        .filter(|r| !panels.hide_backup_layers || r.kind != "backup")
        .collect();
    let ctx = ui.ctx().clone();
    let picked: Option<String> = ctx.data(|d| d.get_temp(picked_id()));
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
            let label = ui.add(egui::Button::selectable(picked.as_deref() == Some(row.id.as_str()), text).frame(false));
            if !master_row && row.kind != "backup" {
                ui.label(egui::RichText::new(&row.kind).strong().color(tokens.text_dim));
            }
            if label.clicked() {
                ctx.data_mut(|d| d.insert_temp(picked_id(), row.id.clone()));
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
                            ctx.data_mut(|d| d.insert_temp(picked_id(), row.id.clone()));
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
    let picked_row = picked.as_ref().and_then(|p| all.iter().find(|r| &r.id == p));
    let deletable = picked_row.is_some_and(|r| r.kind != "master");
    let backup = picked_row.filter(|r| r.kind == "backup");
    ui.horizontal(|ui| {
        if widgets::button(ui, widgets::id("layers.add"), "+", "New Backup Layer", true, false) {
            pending.push(("layers.addBackupLayer", json!({"glyphs": [glyph], "master": active_master})));
        }
        if widgets::button(ui, widgets::id("layers.delete"), "−", "Delete Layer", deletable, false)
            && let Some(r) = picked_row
        {
            pending.push(("layers.delete", json!({"glyphs": [glyph], "layers": [r.id]})));
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
