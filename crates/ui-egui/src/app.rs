//! The application shell: one [`FontcraftApp`] owns the engine session and the UI state, and draws
//! itself into any egui context (the desktop window, the browser canvas, a headless test frame).
//!
//! The shell is thin. Panels read the session and act only through [`FontcraftApp::run`], so
//! anything a click does, the control channel and MCP can do with the same command id.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use fontcraft_engine::{EngineError, Session, guard};
use fontcraft_geom::{Affine, PathEl, Point};
use fontcraft_model::{Font, NodeKind, Shape};
use serde_json::{Value, json};

use crate::menus::{self, UI_COMMANDS, is_ui_command};
use crate::state::{UiState, ZOOM_RANGE};
use crate::theme::Tokens;

pub struct FontcraftApp {
    pub session: Session,
    pub state: UiState,
    pub tokens: Tokens,
    themed: bool,
}

impl Default for FontcraftApp {
    fn default() -> Self {
        Self::new(Session::with_document())
    }
}

impl FontcraftApp {
    pub fn new(session: Session) -> Self {
        Self { session, state: UiState::default(), tokens: Tokens::dark(), themed: false }
    }

    /// Runs a command by id: a shell command from [`UI_COMMANDS`] or an engine command. The result
    /// or the error also goes to the status bar. Never panics: a bug in a command is reported as
    /// an internal error and the document is left as it was.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let result = if is_ui_command(id) {
            match guard::catch_panic(|| self.run_ui_command(id, params)) {
                Ok(r) => r,
                Err(panic) => Err(EngineError::Internal(format!("`{id}` failed unexpectedly: {panic}")).to_string()),
            }
        } else {
            self.session.execute(id, params).map_err(|e| e.to_string())
        };
        self.after_command();
        self.state.status = match &result {
            Ok(_) => format!("{id}: done"),
            Err(e) => e.clone(),
        };
        result
    }

    /// Drops UI references to glyphs a command removed (or to a closed document).
    fn after_command(&mut self) {
        if self.state.selected_glyphs.is_empty() && self.state.editing.is_none() {
            return;
        }
        let known: std::collections::HashSet<&str> =
            self.session.font().map(|f| f.glyphs.iter().map(|g| g.name.as_str()).collect()).unwrap_or_default();
        self.state.selected_glyphs.retain(|name| known.contains(name.as_str()));
        if self.state.editing.as_ref().is_some_and(|g| !known.contains(g.as_str())) {
            self.state.editing = None;
        }
    }

    fn run_ui_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let obj = match params {
            Value::Null => None,
            Value::Object(m) => Some(m),
            _ => return Err(format!("{id}: params must be a JSON object")),
        };
        let get = |key: &str| obj.and_then(|m| m.get(key)).filter(|v| !v.is_null());
        let toggle = |key: &str, current: bool| -> Result<bool, String> {
            match get(key) {
                None => Ok(!current),
                Some(Value::Bool(b)) => Ok(*b),
                Some(_) => Err(format!("{id}: `{key}` must be true or false")),
            }
        };
        let zoom = |z: f32| z.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        match id {
            "fontView.show" => self.state.editing = None,
            "fontView.setSelection" => {
                let font = self.session.font().map_err(|e| e.to_string())?;
                let names = get("glyphs").and_then(Value::as_array).ok_or_else(|| format!("{id}: `glyphs` must be a list of glyph names"))?;
                // Sets, so selecting every glyph of a large font is one pass, not a pass per name.
                let known: std::collections::HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
                let mut seen = std::collections::HashSet::new();
                let mut selected = Vec::new();
                for n in names.iter().take(fontcraft_model::MAX_GLYPHS) {
                    let name = n.as_str().ok_or_else(|| format!("{id}: `glyphs` must be a list of glyph names"))?;
                    if !known.contains(name) {
                        return Err(format!("{id}: no glyph named `{name}`"));
                    }
                    if seen.insert(name) {
                        selected.push(name.to_string());
                    }
                }
                self.state.selected_glyphs = selected;
            }
            "view.editSelectedGlyphs" => {
                let font = self.session.font().map_err(|e| e.to_string())?;
                let name = match get("glyph") {
                    Some(Value::String(n)) => n.clone(),
                    Some(_) => return Err(format!("{id}: `glyph` must be a glyph name")),
                    None => self.state.selected_glyphs.first().cloned().ok_or_else(|| format!("{id}: no glyph is selected"))?,
                };
                if font.glyph(&name).is_none() {
                    return Err(format!("{id}: no glyph named `{name}`"));
                }
                self.state.editing = Some(name);
            }
            "view.zoomIn" => self.state.zoom = zoom(self.state.zoom * 1.25),
            "view.zoomOut" => self.state.zoom = zoom(self.state.zoom / 1.25),
            "view.zoomToActualSize" => self.state.zoom = 1.0,
            "view.showNodes" => self.state.show_nodes = toggle("show", self.state.show_nodes)?,
            "view.showMetrics" => self.state.show_metrics = toggle("show", self.state.show_metrics)?,
            "window.palette" => self.state.show_palette = toggle("show", self.state.show_palette)?,
            other => return Err(format!("unknown command `{other}`")),
        }
        Ok(json!({}))
    }

    /// Every command the app can run, engine and shell, as the control channel lists them.
    pub fn commands(&self) -> Value {
        let mut list: Vec<Value> =
            fontcraft_engine::command_specs().iter().filter_map(|c| serde_json::to_value(c.info(&self.session)).ok()).collect();
        list.extend(UI_COMMANDS.iter().map(|(id, label, menu, shortcut, params)| {
            json!({"id": id, "label": label, "menu": menu, "shortcut": if shortcut.is_empty() { Value::Null } else { json!(shortcut) }, "params": params, "ui": true, "enabled": true})
        }));
        json!({ "commands": list })
    }

    /// Draws one frame into `ui` (the root of a window or of a headless test context).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.themed {
            self.tokens.apply(ui.ctx());
            self.themed = true;
        }
        let mut pending: Vec<(&'static str, Value)> = Vec::new();
        self.menu_bar(ui, &mut pending);
        self.status_bar(ui);
        if self.state.show_palette {
            self.palette(ui);
        }
        match self.state.editing.clone() {
            Some(glyph) => self.edit_view(ui, &glyph, &mut pending),
            None => self.font_view(ui, &mut pending),
        }
        // Commands run after drawing, so a frame never sees a half-changed document.
        for (id, params) in pending {
            let _ = self.run(id, &params);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
        let tree = menus::menu_tree(&self.session);
        let frame = egui::Frame::NONE.fill(self.tokens.app_bar).inner_margin(4.0);
        egui::Panel::top("menu_bar").frame(frame).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for menu in &tree {
                    ui.menu_button(menu.title, |ui| {
                        for item in &menu.items {
                            let mut button = egui::Button::new(item.label);
                            if !item.shortcut.is_empty() {
                                button = button.shortcut_text(item.shortcut);
                            }
                            let response = ui.add_enabled(item.enabled, button);
                            let response = match &item.disabled_reason {
                                Some(why) => response.on_disabled_hover_text(why),
                                None => response,
                            };
                            if response.clicked() {
                                pending.push((item.command, json!({})));
                                ui.close();
                            }
                        }
                    });
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::NONE.fill(self.tokens.panel_darker).inner_margin(4.0);
        let title =
            self.session.active().map(|d| format!("{}{}", d.title, if d.modified() { " •" } else { "" })).unwrap_or_else(|| "No document".into());
        egui::Panel::bottom("status_bar").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).color(self.tokens.text));
                ui.separator();
                ui.label(egui::RichText::new(&self.state.status).color(self.tokens.text_dim));
            });
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::NONE.fill(self.tokens.panel).inner_margin(8.0);
        let font = self.session.font().ok();
        let target = self.state.editing.clone().or_else(|| self.state.selected_glyphs.first().cloned());
        egui::Panel::right("palette").frame(frame).default_size(220.0).show(ui, |ui| {
            ui.label(egui::RichText::new("Palette").strong());
            ui.separator();
            let (Some(font), Some(name)) = (font, target) else {
                ui.label(egui::RichText::new("No glyph selected").color(self.tokens.text_dim));
                return;
            };
            let Some(glyph) = font.glyph(&name) else { return };
            let master = font.masters.first();
            let layer = master.and_then(|m| glyph.layer(&m.id));
            ui.label(format!("Glyph: {}", glyph.name));
            let unicodes: Vec<String> = glyph.unicodes.iter().map(|u| format!("{u:04X}")).collect();
            ui.label(format!("Unicode: {}", if unicodes.is_empty() { "—".into() } else { unicodes.join(", ") }));
            ui.label(format!("Width: {}", layer.map_or("—".into(), |l| l.width.to_string())));
            ui.label(format!("Shapes: {}", layer.map_or(0, |l| l.shapes.len())));
        });
    }

    fn font_view(&mut self, ui: &mut egui::Ui, pending: &mut Vec<(&'static str, Value)>) {
        let frame = egui::Frame::NONE.fill(self.tokens.panel_darker).inner_margin(8.0);
        let tokens = self.tokens.clone();
        let selected = self.state.selected_glyphs.clone();
        let Ok(font) = self.session.font() else {
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                ui.centered_and_justified(|ui| ui.label("File › New to start a font"));
            });
            return;
        };
        let master = font.masters.first().map(|m| m.id.clone()).unwrap_or_default();
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            // Only the rows in view are laid out and painted, so a font of any size scrolls.
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
                            paint_glyph(&painter, font, glyph, &master, art, tokens.glyph_fill);
                            let label_color = if is_selected { tokens.selected_text } else { tokens.outline };
                            painter.text(
                                Pos2::new(rect.center().x, rect.max.y - 2.0),
                                Align2::CENTER_BOTTOM,
                                &glyph.name,
                                FontId::proportional(tokens.cell_label_size),
                                label_color,
                            );
                            if response.double_clicked() {
                                pending.push(("view.editSelectedGlyphs", json!({"glyph": glyph.name})));
                            } else if response.clicked() {
                                pending.push(("fontView.setSelection", json!({"glyphs": [glyph.name]})));
                            }
                        }
                    });
                }
            });
        });
    }

    fn edit_view(&mut self, ui: &mut egui::Ui, glyph: &str, pending: &mut Vec<(&'static str, Value)>) {
        let tokens = self.tokens.clone();
        let state = self.state.clone();
        let frame = egui::Frame::NONE.fill(tokens.canvas);
        let Ok(font) = self.session.font() else { return };
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
            if response.double_clicked() {
                pending.push(("fontView.show", json!({})));
            }
            let painter = ui.painter_at(rect);
            let Some(master) = font.masters.first() else { return };
            let Some(layer) = font.glyph(glyph).and_then(|g| g.layer(&master.id)) else { return };
            // Font units → screen: y flips, the glyph's advance is centred, the baseline sits low.
            let scale = f64::from(state.zoom);
            let origin =
                Point::new(f64::from(rect.center().x) - layer.width * scale / 2.0, f64::from(rect.center().y) + (master.cap_height / 2.0) * scale);
            let to_screen = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(scale, -scale);
            let screen = |p: Point| {
                let q = to_screen * p;
                Pos2::new(q.x as f32, q.y as f32)
            };
            if state.show_metrics {
                let line = Stroke::new(1.0, tokens.metric_line);
                for y in [0.0, master.x_height, master.cap_height, master.ascender, master.descender] {
                    painter.line_segment([screen(Point::new(0.0, y)), screen(Point::new(layer.width, y))], line);
                }
                for x in [0.0, layer.width] {
                    painter.line_segment([screen(Point::new(x, master.descender)), screen(Point::new(x, master.ascender))], line);
                }
            }
            if let Ok(outline) = font.outline(glyph, &master.id) {
                for contour in flatten(&(to_screen * outline)) {
                    painter.add(egui::Shape::closed_line(contour, Stroke::new(1.5, tokens.outline)));
                }
            }
            if state.show_nodes {
                for shape in &layer.shapes {
                    let Shape::Path(path) = shape else { continue };
                    for node in &path.nodes {
                        let at = screen(node.point());
                        match node.kind {
                            NodeKind::OffCurve => {
                                painter.circle_filled(at, 2.5, tokens.node_handle);
                            }
                            _ if node.smooth => {
                                painter.circle_filled(at, 4.0, tokens.node_smooth);
                            }
                            _ => {
                                painter.rect_filled(Rect::from_center_size(at, Vec2::splat(7.0)), 0.0, tokens.node_corner);
                            }
                        }
                    }
                }
            }
        });
    }
}

/// Most points one glyph is flattened to for drawing. The model bounds coordinates and outline
/// size, so real glyphs stay far below this; it keeps a pathological one from filling memory.
const MAX_FLATTEN_POINTS: usize = 200_000;

/// The contours of `path` as polylines (curves flattened to a quarter of a screen point).
fn flatten(path: &fontcraft_geom::BezPath) -> Vec<Vec<Pos2>> {
    let mut contours: Vec<Vec<Pos2>> = Vec::new();
    let mut count = 0usize;
    fontcraft_geom::flatten(path, 0.25, |el| {
        count += 1;
        if count > MAX_FLATTEN_POINTS {
            return;
        }
        match el {
            PathEl::MoveTo(p) => contours.push(vec![Pos2::new(p.x as f32, p.y as f32)]),
            PathEl::LineTo(p) => {
                if let Some(c) = contours.last_mut() {
                    c.push(Pos2::new(p.x as f32, p.y as f32));
                }
            }
            _ => {}
        }
    });
    contours.retain(|c| c.len() > 1);
    contours
}

/// Draws a glyph's outline fitted into `rect` (a Font View cell). Outlines that cannot be resolved
/// draw nothing.
fn paint_glyph(painter: &egui::Painter, font: &Font, glyph: &fontcraft_model::Glyph, master: &str, rect: Rect, color: Color32) {
    let (Ok(outline), Ok(m)) = (font.outline_of(glyph, master), font.master(master)) else { return };
    if outline.elements().is_empty() {
        return;
    }
    let height = (m.ascender - m.descender).max(1.0);
    let scale = f64::from(rect.height()) / height;
    let width = glyph.layer(master).map_or(0.0, |l| l.width);
    let origin = Point::new(f64::from(rect.center().x) - width * scale / 2.0, f64::from(rect.max.y) + m.descender * scale);
    let to_screen = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(scale, -scale);
    // Cells show the outline; filled glyph rendering (with holes) comes with the renderer crate.
    for contour in flatten(&(to_screen * outline)) {
        painter.add(egui::Shape::closed_line(contour, Stroke::new(1.0, color)));
    }
}
