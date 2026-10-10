//! The Selection Info window (`window.selectionInfo`, Edit › Info for Selection): the
//! properties of the selected glyphs, including the ones only it edits (sort name, case,
//! writing direction). In Font View it describes the selected glyphs, in Edit View the edited
//! glyph.
//!
//! The fields follow the inspector's rules ([`inspector`]): *Multiple Values* where the glyphs
//! differ, one command for every glyph per edit. The sort name field is enabled by its check
//! box; turning the box off returns the glyphs to the default sort order
//! (`glyph.setSortName` with `null`). The window's close button runs
//! `edit.infoForSelection {open: false}`.

use serde_json::json;

use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::shell::font_view::columns::Column;
use crate::shell::font_view::inspector::{self, Edit, Summary};
use crate::theme::Tokens;

/// The salt of the window's controls (`widgets::id((SALT, …))`).
pub const SALT: &str = "selectionInfo";

/// The rows besides the sort name, in order.
const ROWS: [Column; 8] =
    [Column::Name, Column::Unicode, Column::ProductionName, Column::Script, Column::Category, Column::SubCategory, Column::Case, Column::Direction];

/// The glyphs the window describes: the edited glyph in Edit View, else the Font View selection.
pub fn targets(app: &FontcraftApp) -> Vec<String> {
    match app.state.tab() {
        Some(_) => app.state.editing().map(|g| vec![g.to_string()]).unwrap_or_default(),
        None => app.state.selected_glyphs.clone(),
    }
}

/// The id of the window.
pub fn window_id() -> egui::Id {
    widgets::id((SALT, "window"))
}

/// The id of the check box that enables the sort name field.
pub fn sort_name_box() -> egui::Id {
    widgets::id((SALT, "sortNameOn"))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.selection_info {
        return;
    }
    crate::shell::font_view::refresh(app);
    let glyphs = targets(app);
    let summary = inspector::cached_summary(app, ui.ctx(), SALT, &glyphs);
    let tokens = app.tokens.clone();
    let mut open = true;
    let mut edits = Vec::new();
    egui::Window::new("Selection Info").id(window_id()).open(&mut open).resizable(false).collapsible(false).show(ui.ctx(), |ui| {
        edits = contents(ui, &tokens, &summary);
    });
    if !open {
        pending.push(("edit.infoForSelection", json!({ "open": false })));
    }
    for edit in &edits {
        pending.extend(inspector::commands(app, &summary.glyphs, edit));
    }
}

fn contents(ui: &mut egui::Ui, tokens: &Tokens, s: &Summary) -> Vec<Edit> {
    let mut edits = Vec::new();
    let any = !s.glyphs.is_empty();
    let one = s.glyphs.len() == 1;
    if !any {
        ui.label(egui::RichText::new("No glyph is selected").color(tokens.text_dim));
    }
    let width = tokens.dialog_field_width * 2.5;
    egui::Grid::new(widgets::id((SALT, "grid"))).num_columns(2).show(ui, |ui| {
        for column in ROWS {
            let enabled = if column == Column::Name { one } else { any };
            edits.extend(inspector::text_row(ui, tokens, SALT, column.label(), column, s, width, enabled));
        }
        ui.label(egui::RichText::new("Sort Name").color(tokens.text_dim));
        ui.horizontal(|ui| {
            // Turned on with nothing stored yet, the box only enables the field; the field's
            // commit stores the sort name.
            let local = ui.ctx().data(|d| d.get_temp::<bool>(sort_name_box())).unwrap_or(false);
            let state = match s.sort_name_stored {
                Some(false) if local => Some(true),
                other => other,
            };
            if inspector::check_box(ui, tokens, sort_name_box(), state, any, "Use a custom sort name") {
                match state {
                    Some(false) => ui.ctx().data_mut(|d| d.insert_temp(sort_name_box(), true)),
                    _ => {
                        ui.ctx().data_mut(|d| d.remove::<bool>(sort_name_box()));
                        if s.sort_name_stored != Some(false) {
                            edits.push(Edit::ClearSortName);
                        }
                    }
                }
            }
            let side = ui.spacing().icon_width + ui.spacing().item_spacing.x;
            let id = inspector::field_id(SALT, Column::SortName);
            if let Some(text) = inspector::text_field(ui, tokens, id, &s.field(Column::SortName), width - side, any && state != Some(false)) {
                edits.push(Edit::Text(Column::SortName, text));
            }
        });
        ui.end_row();
    });
    edits
}
