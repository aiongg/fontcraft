//! The Actions window (`dialog.actions`, state and behaviour in `crate::shell::actions`): the
//! searchable action list grouped by menu, the chosen action's form or JSON editor, the scope and
//! layer pop-ups, the before-and-after preview, and the action set in the sidebar.
//!
//! | Control | Does |
//! |---|---|
//! | search field | `dialog.actions {search}` |
//! | a row of the list | `{command}`: chooses the action |
//! | a field of the form | `{set: {key: value}}` |
//! | JSON editor | edits the parameters in place; an error shows below it |
//! | Scope, Category, Layers | `{scope}`, `{category}`, `{layers}` |
//! | Preview, Wireframe | `{preview}`, `{wireframe}` |
//! | Run | `{run: true}` |
//! | Add to Set | `{add: true}` |
//! | a step, Remove, Clear | selects it; `{remove: index}`; `{clear: true}` |
//! | Title, Name or Path, Save, Open, Run Set | `{title}`; `{save}` or `{savePath}` (a name with a folder separator is a path); `{openSet}`; `{runSet: true}` |

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, ColorImage, Pos2, Rect, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_geom::{Affine, BezPath, Shape as _};
use fontcraft_model::Font;
use fontcraft_render::{Style, render_path};
use serde_json::{Value, json};

use super::form::{Field, Kind};
use crate::app::FontcraftApp;
use crate::panels::{Pending, widgets};
use crate::shell::actions::{self, GlyphScope, LayerScope, Shown};
use crate::theme::Tokens;

/// The command every control of the window runs.
const COMMAND: &str = "dialog.actions";
/// Most pixels a side of a filled preview half is drawn at.
const MAX_PREVIEW_PIXELS: f32 = 4096.0;

pub fn window_id() -> egui::Id {
    widgets::id(("actions", "window"))
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let ctx = ui.ctx().clone();
    let FontcraftApp { session, state, tokens, .. } = app;
    let st = &mut state.panels.actions;
    let mut open = true;
    let spec = st.command.as_deref().and_then(|id| actions::action_spec(id).ok());
    let json_error = match (spec, &st.form) {
        (Some(spec), None) => actions::check_json(spec, &st.json).err(),
        _ => None,
    };
    let can_save = session.extensions().macros_dir.is_some();
    egui::Window::new("Actions")
        .id(window_id())
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.console_width * 1.5, tokens.console_height * 1.5))
        .show(&ctx, |ui| {
            if let Some(text) = widgets::text(ui, widgets::id(("actions", "search")), &st.search, tokens.dialog_field_width * 3.0) {
                pending.push((COMMAND, json!({"search": text})));
            }
            ui.add_space(tokens.dialog_spacing);
            ui.columns(3, |cols| {
                let [list, middle, side] = cols else { return };
                action_list(list, st, tokens, pending);
                action_form(middle, st, spec, json_error.as_deref(), tokens, pending);
                set_sidebar(side, st, can_save, tokens, pending);
            });
        });
    if !open {
        pending.push((COMMAND, json!({"open": false})));
    }
}

/// The actions matching the search, under a heading per menu.
fn action_list(ui: &mut egui::Ui, st: &actions::ActionsState, tokens: &Tokens, pending: &mut Pending) {
    egui::ScrollArea::vertical().id_salt(widgets::id(("actions", "list"))).auto_shrink([false, false]).show(ui, |ui| {
        let mut group: Option<&str> = None;
        for entry in actions::matching(&st.search) {
            if group != Some(entry.group.as_str()) {
                group = Some(entry.group.as_str());
                ui.add_space(tokens.dialog_spacing);
                ui.label(egui::RichText::new(&entry.group).strong());
            }
            let on = st.command.as_deref() == Some(entry.id);
            if ui.add(egui::Button::selectable(on, entry.label)).on_hover_text(entry.id).clicked() && !on {
                pending.push((COMMAND, json!({"command": entry.id})));
            }
        }
    });
}

/// The chosen action's parameters, scope, preview and buttons.
fn action_form(
    ui: &mut egui::Ui,
    st: &mut actions::ActionsState,
    spec: Option<&fontcraft_engine::CommandSpec>,
    json_error: Option<&str>,
    tokens: &Tokens,
    pending: &mut Pending,
) {
    let Some(spec) = spec else {
        ui.label(egui::RichText::new("Choose an action from the list.").color(tokens.text_dim));
        return;
    };
    ui.label(egui::RichText::new(spec.label).strong()).on_hover_text(spec.params);
    ui.add_space(tokens.dialog_spacing);
    match &st.form {
        Some(form) => {
            egui::Grid::new(widgets::id(("actions", "fields"))).num_columns(2).spacing([tokens.dialog_spacing, tokens.dialog_spacing]).show(
                ui,
                |ui| {
                    let mut group: Option<&str> = None;
                    for field in &form.fields {
                        if field.group.as_deref() != group {
                            group = field.group.as_deref();
                            if let Some(g) = group {
                                ui.label(egui::RichText::new(g).strong());
                                ui.end_row();
                            }
                        }
                        ui.label(&field.label).on_hover_text(&field.doc);
                        let value = form.values.get(&field.key).unwrap_or(&Value::Null);
                        if let Some(v) = field_widget(ui, field, value, tokens) {
                            pending.push((COMMAND, json!({"set": {field.key.clone(): v}})));
                        }
                        ui.end_row();
                    }
                },
            );
        }
        None => {
            ui.label(egui::RichText::new("Parameters (JSON)").color(tokens.text_dim)).on_hover_text(spec.params);
            ui.add(egui::TextEdit::multiline(&mut st.json).code_editor().desired_rows(6).desired_width(f32::INFINITY));
            if let Some(e) = json_error {
                ui.label(egui::RichText::new(e).color(tokens.error_text));
            }
        }
    }
    ui.add_space(tokens.dialog_spacing);
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Scope");
        egui::ComboBox::from_id_salt(widgets::id(("actions", "scope"))).selected_text(st.scope.label()).show_ui(ui, |ui| {
            for scope in GlyphScope::ALL {
                if ui.add(egui::Button::selectable(st.scope == scope, scope.label())).clicked() && st.scope != scope {
                    pending.push((COMMAND, json!({"scope": scope.word()})));
                }
            }
        });
        if st.scope == GlyphScope::Category
            && let Some(text) = widgets::text(ui, widgets::id(("actions", "category")), &st.category, tokens.dialog_field_width * 2.0)
        {
            pending.push((COMMAND, json!({"category": text})));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Layers");
        egui::ComboBox::from_id_salt(widgets::id(("actions", "layers"))).selected_text(st.layers.label()).show_ui(ui, |ui| {
            for layers in LayerScope::ALL {
                if ui.add(egui::Button::selectable(st.layers == layers, layers.label())).clicked() && st.layers != layers {
                    pending.push((COMMAND, json!({"layers": layers.word()})));
                }
            }
        });
    });
    ui.horizontal(|ui| {
        let mut preview = st.preview;
        if ui.checkbox(&mut preview, "Preview").changed() {
            pending.push((COMMAND, json!({"preview": preview})));
        }
        let mut wireframe = st.wireframe;
        if ui.checkbox(&mut wireframe, "Wireframe").changed() {
            pending.push((COMMAND, json!({"wireframe": wireframe})));
        }
    });
    if st.preview {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), tokens.preview_area_height), egui::Sense::hover());
        paint_preview(ui, rect, &st.shown, tokens, st.wireframe);
        if let Some(e) = &st.shown.error {
            ui.label(egui::RichText::new(e).color(tokens.error_text));
        }
    }
    ui.add_space(tokens.dialog_spacing);
    ui.horizontal(|ui| {
        if ui.button("Run").clicked() {
            pending.push((COMMAND, json!({"run": true})));
        }
        if ui.button("Add to Set").clicked() {
            pending.push((COMMAND, json!({"add": true})));
        }
    });
    if !st.shown.summary.is_empty() {
        ui.label(egui::RichText::new(&st.shown.summary).color(tokens.text_dim));
    }
    if !st.status.is_empty() {
        ui.label(egui::RichText::new(&st.status).color(tokens.text_dim));
    }
}

/// The widget of one field; the new value when the person changed it.
fn field_widget(ui: &mut egui::Ui, field: &Field, value: &Value, tokens: &Tokens) -> Option<Value> {
    let id = widgets::id(("actions", "field", field.key.as_str()));
    match &field.kind {
        Kind::Number { .. } | Kind::Integer { .. } => match widgets::number(ui, id, value.as_f64(), tokens.dialog_field_width, false)? {
            widgets::NumberEdit::Set(n) => Some(json!(n)),
            widgets::NumberEdit::Clear => None,
        },
        Kind::Bool | Kind::Lock => {
            let mut on = value.as_bool().unwrap_or(false);
            ui.checkbox(&mut on, "").changed().then(|| json!(on))
        }
        Kind::Text { .. } => widgets::text(ui, id, value.as_str().unwrap_or(""), tokens.dialog_field_width * 3.0).map(|t| json!(t)),
        Kind::Choice { choices, labels } => {
            let label_of = |i: usize| labels.get(i).or_else(|| choices.get(i)).map_or("", String::as_str);
            let current = value.as_str().unwrap_or("");
            let shown = choices.iter().position(|c| c == current).map_or(current, label_of);
            let mut picked = None;
            egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
                for (i, choice) in choices.iter().enumerate() {
                    if ui.add(egui::Button::selectable(choice == current, label_of(i))).clicked() && choice != current {
                        picked = Some(json!(choice));
                    }
                }
            });
            picked
        }
        Kind::Multi { choices, labels } => {
            let ticked: Vec<&str> = value.as_array().map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let mut changed = None;
            ui.vertical(|ui| {
                for (i, choice) in choices.iter().enumerate() {
                    let mut on = ticked.contains(&choice.as_str());
                    let label = labels.get(i).unwrap_or(choice);
                    if ui.checkbox(&mut on, label.as_str()).changed() {
                        let next: Vec<&str> =
                            choices.iter().map(String::as_str).filter(|c| if *c == choice.as_str() { on } else { ticked.contains(c) }).collect();
                        changed = Some(json!(next));
                    }
                }
            });
            changed
        }
        Kind::Folder | Kind::Order { .. } | Kind::List { .. } | Kind::Rules => None,
    }
}

/// The action set: its title, its steps, and the buttons that edit, save, open and run it.
fn set_sidebar(ui: &mut egui::Ui, st: &mut actions::ActionsState, can_save: bool, tokens: &Tokens, pending: &mut Pending) {
    ui.label(egui::RichText::new("Action Set").strong());
    if let Some(title) = widgets::text(ui, widgets::id(("actions", "setTitle")), &st.set_title, tokens.dialog_field_width * 3.0) {
        pending.push((COMMAND, json!({"title": title})));
    }
    ui.add_space(tokens.dialog_spacing);
    let height = (ui.available_height() - tokens.list_row_height * 6.0).max(tokens.list_row_height * 3.0);
    egui::ScrollArea::vertical().id_salt(widgets::id(("actions", "set"))).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        if st.set.is_empty() {
            ui.label(egui::RichText::new("Add actions to build a set.").color(tokens.text_dim));
        }
        for (i, step) in st.set.iter().enumerate() {
            let label = fontcraft_engine::find_command(&step.command).map_or(step.command.as_str(), |c| c.label);
            let on = st.set_selected == Some(i);
            let text = format!("{}. {label}", i + 1);
            if ui.add(egui::Button::selectable(on, text)).on_hover_text(step.params.to_string()).clicked() {
                st.set_selected = if on { None } else { Some(i) };
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(st.set_selected.is_some(), egui::Button::new("Remove")).clicked()
            && let Some(i) = st.set_selected
        {
            pending.push((COMMAND, json!({"remove": i})));
        }
        if ui.add_enabled(!st.set.is_empty(), egui::Button::new("Clear")).clicked() {
            pending.push((COMMAND, json!({"clear": true})));
        }
    });
    ui.add_space(tokens.dialog_spacing);
    ui.label(egui::RichText::new("Name or path").color(tokens.text_dim));
    ui.add(egui::TextEdit::singleline(&mut st.set_name).desired_width(f32::INFINITY));
    let name = st.set_name.trim();
    let is_path = name.contains(['/', '\\']);
    ui.horizontal(|ui| {
        let save_ok = !name.is_empty() && !st.set.is_empty() && (is_path || can_save);
        if ui
            .add_enabled(save_ok, egui::Button::new("Save"))
            .on_hover_text("A name saves into the Macros folder; a path saves to that file")
            .clicked()
        {
            let key = if is_path { "savePath" } else { "save" };
            pending.push((COMMAND, json!({key: name})));
        }
        if ui.add_enabled(!name.is_empty(), egui::Button::new("Open")).on_hover_text("A macro of the Macros folder, or a macro file").clicked() {
            pending.push((COMMAND, json!({"openSet": name})));
        }
        if ui.add_enabled(!st.set.is_empty(), egui::Button::new("Run Set")).clicked() {
            pending.push((COMMAND, json!({"runSet": true})));
        }
    });
}

/// The preview glyph before (left) and after (right) the action, at one scale.
fn paint_preview(ui: &egui::Ui, rect: Rect, shown: &Shown, tokens: &Tokens, wireframe: bool) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, tokens.preview_paper);
    let outline = |font: &Option<Arc<Font>>| font.as_ref().and_then(|f| f.outline(&shown.glyph, &shown.master).ok());
    let paths = [outline(&shown.before), outline(&shown.after)];
    let mut bounds: Option<fontcraft_geom::Rect> = None;
    for path in paths.iter().flatten() {
        let b = path.bounding_box();
        if [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && (b.width() > 0.0 || b.height() > 0.0) {
            bounds = Some(bounds.map_or(b, |u| u.union(b)));
        }
    }
    let mid = rect.center().x;
    let halves = [Rect::from_min_max(rect.min, pos2(mid, rect.max.y)), Rect::from_min_max(pos2(mid, rect.min.y), rect.max)];
    painter.line_segment([pos2(mid, rect.min.y), pos2(mid, rect.max.y)], Stroke::new(1.0, tokens.border));
    let Some(bounds) = bounds else {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Nothing to show", egui::FontId::default(), tokens.text_dim);
        return;
    };
    let room = halves.iter().map(|h| (h.width().min(h.height()) - 2.0 * tokens.preview_margin).max(1.0)).fold(f32::INFINITY, f32::min);
    let scale = f64::from(room) / bounds.width().max(bounds.height()).max(1.0);
    let fonts = [&shown.before, &shown.after];
    for (side, (path, half)) in paths.iter().zip(halves).enumerate() {
        let Some(path) = path else { continue };
        // The drawn font stays alive in `shown`, so its address names its contents.
        let source = fonts.get(side).and_then(|f| f.as_ref()).map_or(0, |f| Arc::as_ptr(f) as usize);
        let center = half.center();
        let to_rect = Affine::translate((f64::from(center.x), f64::from(center.y)))
            * Affine::scale_non_uniform(scale, -scale)
            * Affine::translate((-bounds.center().x, -bounds.center().y));
        if wireframe {
            for contour in crate::canvas::flatten(&(to_rect * path.clone())) {
                painter.add(egui::Shape::line(contour, Stroke::new(1.0, tokens.preview_ink)));
            }
        } else if let Some(texture) = filled(ui, side, (source, &shown.glyph, &shown.master), path, to_rect, half, tokens.preview_ink) {
            painter.image(texture.id(), half, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
    }
    let label = |half: Rect, text: &str| {
        painter.text(
            half.left_top() + vec2(tokens.dialog_spacing, tokens.dialog_spacing),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::default(),
            tokens.text_dim,
        );
    };
    let [before, after] = halves;
    label(before, "Before");
    label(after, "After");
}

/// The filled outline of one preview half as a texture, kept in the context's memory and drawn
/// again only when the outline or the half's size changes.
/// `source`: the font's address, the glyph and the master the outline is from.
fn filled(
    ui: &egui::Ui,
    side: usize,
    source: (usize, &str, &str),
    path: &BezPath,
    to_rect: Affine,
    half: Rect,
    ink: Color32,
) -> Option<TextureHandle> {
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = ((half.width() * ppp).round(), (half.height() * ppp).round());
    if !(w >= 1.0 && h >= 1.0 && w <= MAX_PREVIEW_PIXELS && h <= MAX_PREVIEW_PIXELS) {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for v in to_rect.as_coeffs().into_iter().chain([f64::from(w), f64::from(h), f64::from(ppp)]) {
        v.to_bits().hash(&mut hasher);
    }
    source.hash(&mut hasher);
    ink.to_array().hash(&mut hasher);
    let key = hasher.finish();
    let id = widgets::id(("actions", "fill", side));
    if let Some((k, texture)) = ui.ctx().data(|d| d.get_temp::<(u64, TextureHandle)>(id))
        && k == key
    {
        return Some(texture);
    }
    let to_pixels = Affine::scale(f64::from(ppp)) * Affine::translate((-f64::from(half.min.x), -f64::from(half.min.y))) * to_rect;
    let [r, g, b, a] = ink.to_srgba_unmultiplied();
    let style = Style { ink: [r, g, b, a], background: [0, 0, 0, 0], anti_alias: true };
    let (width, height) = (w as u32, h as u32);
    let image = render_path(path, to_pixels, width, height, &style).ok()?;
    let color = ColorImage::from_rgba_premultiplied([width as usize, height as usize], &image.pixels);
    let texture = ui.ctx().load_texture(format!("actions-preview-{side}"), color, TextureOptions::LINEAR);
    ui.ctx().data_mut(|d| d.insert_temp(id, (key, texture.clone())));
    Some(texture)
}
