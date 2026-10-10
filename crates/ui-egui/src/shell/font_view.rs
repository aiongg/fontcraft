//! Font View: the glyph grid, its selection, and opening glyphs for editing.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use fontcraft_geom::{Affine, Point};
use fontcraft_model::{Font, metric};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::flatten;

pub const COMMANDS: &[UiCommand] = &[
    ("fontView.show", "Show Font View", &[], "Cmd+Alt+1", "{} leave Edit View and show the glyph grid"),
    ("fontView.setSelection", "Set Glyph Selection", &[], "", "{glyphs: [names]} select these glyphs in Font View (unknown names are an error)"),
    (
        "view.editSelectedGlyphs",
        "Edit Selected Glyphs",
        &[],
        "Cmd+Down",
        "{glyphs?: [names]} open the glyphs (default: the Font View selection) in Edit View: the tab that holds exactly them, else a new tab",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "fontView.show" => {
            app.state.active_tab = None;
            Ok(json!({}))
        }
        "fontView.setSelection" => {
            let font = app.session.font().map_err(|e| e.to_string())?;
            let names = args.strings("glyphs", fontcraft_model::MAX_GLYPHS)?.ok_or_else(|| args.err("`glyphs` must be a list of glyph names"))?;
            // Sets, so selecting every glyph of a large font is one pass, not a pass per name.
            let known: std::collections::HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
            let mut seen = std::collections::HashSet::new();
            let mut selected = Vec::new();
            for name in names {
                if !known.contains(name.as_str()) {
                    return Err(args.err(format!("no glyph named `{name}`")));
                }
                if seen.insert(name.clone()) {
                    selected.push(name);
                }
            }
            app.state.selected_glyphs = selected;
            Ok(json!({}))
        }
        "view.editSelectedGlyphs" => {
            let glyphs = match args.strings("glyphs", crate::state::MAX_TAB_GLYPHS)? {
                Some(list) => list,
                None => app.state.selected_glyphs.iter().take(crate::state::MAX_TAB_GLYPHS).cloned().collect(),
            };
            if glyphs.is_empty() {
                return Err(args.err("no glyph is selected"));
            }
            if let Some(index) = app.state.tabs.iter().position(|t| t.glyphs == glyphs) {
                app.state.active_tab = Some(index);
                return Ok(super::tabs::describe(app, index));
            }
            super::tabs::open(app, args, glyphs)
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Draws the glyph grid. Only the rows in view are laid out and painted, so a font of any size
/// scrolls.
pub fn show(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
    let tokens = &app.tokens;
    let frame = egui::Frame::NONE.fill(tokens.panel_darker).inner_margin(8.0);
    let selected = &app.state.selected_glyphs;
    let Ok(font) = app.session.font() else {
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.centered_and_justified(|ui| ui.label("File › New to start a font"));
        });
        return;
    };
    let master = font.masters.first().map(|m| m.id.clone()).unwrap_or_default();
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        let step = tokens.cell_size + tokens.cell_gap;
        let columns = (((ui.available_width() + tokens.cell_gap) / step).floor() as usize).max(1);
        let rows = font.glyphs.len().div_ceil(columns);
        let row_height = tokens.cell_size + tokens.cell_label_height;
        ui.spacing_mut().item_spacing = Vec2::splat(tokens.cell_gap);
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_height, rows, |ui, visible| {
            for row in visible {
                ui.horizontal(|ui| {
                    for glyph in font.glyphs.iter().skip(row.saturating_mul(columns)).take(columns) {
                        let size = Vec2::new(tokens.cell_size, row_height);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        let is_selected = selected.iter().any(|s| *s == glyph.name);
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, f32::from(tokens.radius), if is_selected { tokens.row_selected } else { tokens.canvas });
                        let art = Rect::from_min_size(rect.min, Vec2::splat(tokens.cell_size)).shrink(6.0);
                        paint_cell(&painter, font, glyph, &master, art, tokens.glyph_fill);
                        let label_color = if is_selected { tokens.selected_text } else { tokens.outline };
                        painter.text(
                            Pos2::new(rect.center().x, rect.max.y - 2.0),
                            Align2::CENTER_BOTTOM,
                            &glyph.name,
                            FontId::proportional(tokens.cell_label_size),
                            label_color,
                        );
                        if response.double_clicked() {
                            pending.push(("view.editSelectedGlyphs", json!({"glyphs": [glyph.name]})));
                        } else if response.clicked() {
                            pending.push(("fontView.setSelection", json!({"glyphs": [glyph.name]})));
                        }
                    }
                });
            }
        });
    });
}

/// Draws a glyph's outline fitted into `rect` (a Font View cell). Outlines that cannot be resolved
/// draw nothing.
fn paint_cell(painter: &egui::Painter, font: &Font, glyph: &fontcraft_model::Glyph, master: &str, rect: Rect, color: Color32) {
    let (Ok(outline), Ok(m)) = (font.outline_of(glyph, master), font.master(master)) else { return };
    if outline.elements().is_empty() {
        return;
    }
    let upm = f64::from(font.units_per_em);
    let (ascender, descender) = (font.metric_pos(m, metric::ASCENDER, upm * 0.8), font.metric_pos(m, metric::DESCENDER, -upm * 0.2));
    let height = (ascender - descender).max(1.0);
    let scale = f64::from(rect.height()) / height;
    let width = glyph.layer(master).map_or(0.0, |l| l.width);
    let origin = Point::new(f64::from(rect.center().x) - width * scale / 2.0, f64::from(rect.max.y) + descender * scale);
    let to_screen = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(scale, -scale);
    for contour in flatten(&(to_screen * outline)) {
        painter.add(egui::Shape::line(contour, Stroke::new(1.0, color)));
    }
}
