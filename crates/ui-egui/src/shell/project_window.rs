//! The project window: one window per open project document (`fontcraft_engine::project`), with
//! the font link at the top, the project's instance list at the left, the selected instances'
//! fields and custom parameters at the right (as Font Info › Exports shows an export), and the
//! export controls at the bottom. Every control acts through an engine command; the window keeps
//! only view state ([`ProjectView`]).
//!
//! | Control | Does |
//! |---|---|
//! | Choose, the font path | the system chooser, then `project.linkFont`; without a chooser the path is a text field |
//! | list entry: click, Cmd-click, Shift-click | selects one entry, adds or drops one, adds a range (view state) |
//! | list entry: drag onto another | `project.moveInstances` (the selection moves when the dragged entry is in it) |
//! | list entry: Alt-drag onto another | `project.duplicateInstances` with `to` |
//! | list entry: context menu › Duplicate | `project.duplicateInstances` on the selection |
//! | plus menu › Add Instance | `project.addInstance`; Add Variable Font Setting is disabled until the engine takes it |
//! | minus | `project.removeInstances` on the selection |
//! | arrow | `project.revertInstances` |
//! | Active, Style Name, Weight Class, Width Class, Style Linking | `project.editInstance` with those keys, once per selected entry |
//! | Custom Parameters: plus, checkbox, value, minus | the parameter picker; `project.editInstance` with `customParameters` |
//! | Export Destination | the system folder chooser, then `project.setExportDestination`; without a chooser a text field |
//! | Save | `project.save`; a project never saved asks the save chooser for its file, offering `<title>.fontcraftproject` |
//! | Export | `project.export`; disabled until the project is saved, linked to a font and has a destination |
//! | close box | `project.close`, which keeps a project with unsaved changes open and says why in the status bar |
//!
//! General fields an instance carries (localized names…) are shown read-only: the project
//! commands edit the built-in fields and the custom parameters. Axes Coordinates come from the
//! linked font as last read.
//!
//! Row: `window.project` (M7.19).

use std::collections::HashMap;

use egui::{Align, Align2, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::fields;
use fontcraft_engine::fontinfo::rows::{Row, RowKind, property_value};
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_engine::project::ProjectDoc;
use fontcraft_model::Instance;
use fontcraft_model::axes::design_value;
use fontcraft_model::plist::Value as PlistValue;
use serde_json::{Value, json};

use super::font_info::exports_tab::{WEIGHT_CLASSES, WEIGHT_NAMES, WIDTH_CLASSES, WIDTH_NAMES, class_value, icon_color, is_variable};
use super::font_info::param_value::{self, MULTIPLE_VALUES};
use super::font_info::rows::{self, RowAction, RowOptions, Target};
use crate::app::FontcraftApp;
use crate::dialogs::custom_parameter_picker::{self, Outcome, ParameterPicker};
use crate::icons::Icon;
use crate::panels::{Pending, widgets};
use crate::theme::Tokens;

/// The file kinds the font chooser offers: the sources a project can link.
pub const FONT_EXTENSIONS: [&str; 5] = ["glyphs", "glyphspackage", "ufo", "designspace", "fontcraft"];
/// The suffix a project chosen by folder is saved with.
pub const PROJECT_EXTENSION: &str = "fontcraftproject";
/// The list's width, in list columns of the theme.
const LIST_COLUMNS: f32 = 4.0;
/// The window's default size, in list columns and list rows of the theme.
const WINDOW_COLUMNS: f32 = 15.0;
const WINDOW_ROWS: f32 = 24.0;
/// The height kept below the scrolling part for the export row, in list rows.
const BOTTOM_ROWS: f32 = 2.5;
/// Most axes the Axes Coordinates rows list.
const MAX_AXES_SHOWN: usize = 64;
/// Windows of later projects open this many steps lower and to the right, then start over.
const CASCADE: usize = 8;
/// The class an instance has when its source states none (as `font.instance.list` reports it).
const DEFAULT_WEIGHT_CLASS: f64 = 400.0;
const DEFAULT_WIDTH_CLASS: f64 = 5.0;

/// The view state of one project's window, kept in the context's memory under the project's id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectView {
    /// Selected entries, as indices into the project's instances.
    pub selected: Vec<usize>,
    /// The entry a Shift-click selects from: the last one clicked.
    pub anchor: Option<usize>,
    /// The Add Custom Parameter picker, while open.
    pub picker: Option<ParameterPicker>,
}

/// What a drag of a list entry carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dragged {
    project: u64,
    index: usize,
}

/// A system chooser the window asks for after drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    Font,
    Destination,
    SaveFile,
}

/// What the person did to a path control.
enum PathEdit {
    /// Clicked the path: choose another.
    Clicked,
    /// Typed a path and pressed Return (no chooser).
    Typed(String),
}

// ---------- ids ----------

/// The id of the window of the project `project` (its id in the session).
pub fn window_id(project: u64) -> Id {
    widgets::id(("project.window", project))
}

/// The id of a control of the window: `font`, `choose`, `plus`, `minus`, `revert`,
/// `parameters` (the plus of Custom Parameters), `destination`, `save`, `export`.
pub fn control_id(project: u64, name: &str) -> Id {
    widgets::id(("project.control", project, name))
}

/// The id of the plus menu's entry `kind` (`instance`, `variableFontSetting`).
pub fn plus_item_id(project: u64, kind: &str) -> Id {
    widgets::id(("project.plusItem", project, kind))
}

/// The id of the list entry at `index`.
pub fn entry_id(project: u64, index: usize) -> Id {
    widgets::id(("project.entry", project, index))
}

/// The id of the context menu's Duplicate item of the entry at `index`.
pub fn duplicate_id(project: u64, index: usize) -> Id {
    entry_id(project, index).with("duplicate")
}

/// The id of a built-in field: `active`, `styleName`, `weightClass`, `widthClass`, `isBold`,
/// `isItalic`, `linkStyle`.
pub fn field_id(project: u64, key: &str) -> Id {
    widgets::id(("project.field", project, key))
}

/// The id of the disclosure button beside `weightClass` or `widthClass`.
pub fn class_menu_id(project: u64, key: &str) -> Id {
    field_id(project, key).with("menu")
}

/// The id of the entry `value` in the class menu of `key`.
pub fn class_item_id(project: u64, key: &str, value: u16) -> Id {
    field_id(project, key).with(("item", value))
}

/// The id of the parameter row `name` (the `occurrence`th of that name, from 0). Its minus
/// button is `.with("minus")`.
pub fn parameter_id(project: u64, name: &str, occurrence: usize) -> Id {
    widgets::id(("project.parameter", project, name, occurrence))
}

fn view_id(project: u64) -> Id {
    widgets::id(("project.view", project))
}

/// The view state of the window of `project`.
pub fn view(ctx: &egui::Context, project: u64) -> ProjectView {
    ctx.data(|d| d.get_temp(view_id(project))).unwrap_or_default()
}

fn store(ctx: &egui::Context, project: u64, view: ProjectView) {
    ctx.data_mut(|d| d.insert_temp(view_id(project), view));
}

// ---------- the windows ----------

/// Draws a window for every open project. Their controls queue commands in `pending`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let ctx = ui.ctx().clone();
    let ids: Vec<u64> = app.session.projects().iter().map(|d| d.id).collect();
    for (n, project) in ids.into_iter().enumerate() {
        window(app, &ctx, project, n, pending);
    }
}

/// The window of one project, the `n`th open.
fn window(app: &mut FontcraftApp, ctx: &egui::Context, project: u64, n: usize, pending: &mut Pending) {
    let mut view = view(ctx, project);
    list_picker(app, &mut view);
    let tokens = app.tokens.clone();
    let chooser = app.picker.is_some();
    let Some(doc) = app.session.projects().iter().find(|d| d.id == project) else { return };
    let count = doc.data.instances.len();
    view.selected.retain(|i| *i < count);
    let title = doc.title.clone();
    let shown_title = if doc.modified() { format!("{title} (edited)") } else { title.clone() };
    let step = u16::try_from(n % CASCADE).map(f32::from).unwrap_or(0.0) * tokens.list_row_height;
    let mut ask = None;
    let mut open = true;
    egui::Window::new(shown_title)
        .id(window_id(project))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.list_column_unit * WINDOW_COLUMNS, tokens.list_row_height * WINDOW_ROWS))
        .pivot(Align2::CENTER_TOP)
        .default_pos(pos2(ctx.content_rect().center().x + step, tokens.dialog_top + step))
        .show(ctx, |ui| body(ui, &tokens, doc, &mut view, chooser, &mut ask, pending));
    let outcome = view.picker.as_mut().and_then(|p| custom_parameter_picker::show(ctx, &tokens, p));
    match outcome {
        Some(Outcome::Add) => {
            let targets = shown_selection(&view, count);
            if let Some(picker) = view.picker.take() {
                let adds: Vec<Value> =
                    picker.choices.iter().filter(|c| picker.selected.contains(&c.name)).map(|c| json!({"action": "add", "name": c.name})).collect();
                if !adds.is_empty() {
                    pending.extend(edits(project, &targets, &json!({"customParameters": adds})));
                }
            }
        }
        Some(Outcome::Cancel) => view.picker = None,
        None => {}
    }
    store(ctx, project, view);
    if !open {
        pending.push(("project.close", json!({"project": project})));
    }
    if let Some(ask) = ask {
        choose(app, project, &title, ask, pending);
    }
}

/// Fills the open parameter picker's list when its search text changed.
fn list_picker(app: &mut FontcraftApp, view: &mut ProjectView) {
    let Some(params) = view.picker.as_ref().filter(|p| p.needs_listing()).map(ParameterPicker::query_params) else { return };
    let result = app.run("font.customParameters.describe", &params);
    if let Some(p) = view.picker.as_mut() {
        match result {
            Ok(r) => p.fill(&r),
            Err(e) => {
                p.error = Some(e);
                p.listed = Some(p.query.clone());
            }
        }
    }
}

/// Asks the system chooser and queues the command its answer runs. A cancelled chooser runs
/// nothing.
fn choose(app: &mut FontcraftApp, project: u64, title: &str, ask: Ask, pending: &mut Pending) {
    let Some(chooser) = app.picker.as_mut() else { return };
    match ask {
        Ask::Font => {
            if let Some(path) = chooser.files("Choose Font", "Font sources", &FONT_EXTENSIONS).into_iter().next() {
                pending.push(("project.linkFont", json!({"project": project, "font": path.to_string_lossy()})));
            }
        }
        Ask::Destination => {
            if let Some(folder) = chooser.folder("Export Destination") {
                pending.push(("project.setExportDestination", json!({"project": project, "folder": folder.to_string_lossy()})));
            }
        }
        Ask::SaveFile => {
            let suggested = format!("{title}.{PROJECT_EXTENSION}");
            if let Some(path) = chooser.save("Save Project", "Project", &[PROJECT_EXTENSION], &suggested) {
                pending.push(("project.save", json!({"project": project, "path": path.to_string_lossy()})));
            }
        }
    }
}

/// Why Export is disabled, or `None` when the project can be exported.
pub fn export_blocked(doc: &ProjectDoc) -> Option<&'static str> {
    if doc.data.font.is_none() {
        Some("Choose the font to export first")
    } else if doc.data.destination.is_none() {
        Some("Choose an export destination first")
    } else if doc.path.is_none() {
        Some("Save the project first")
    } else if doc.modified() {
        Some("Save the project's changes first")
    } else {
        None
    }
}

/// The window's contents.
fn body(ui: &mut egui::Ui, tokens: &Tokens, doc: &ProjectDoc, view: &mut ProjectView, chooser: bool, ask: &mut Option<Ask>, pending: &mut Pending) {
    let project = doc.id;
    font_row(ui, tokens, doc, chooser, ask, pending);
    ui.separator();
    let height = (ui.available_height() - tokens.list_row_height * BOTTOM_ROWS).max(tokens.list_row_height);
    egui::ScrollArea::vertical().id_salt(("project.scroll", project)).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(tokens.list_column_unit * LIST_COLUMNS);
                instance_list(ui, tokens, doc, view, pending);
            });
            ui.separator();
            ui.vertical(|ui| editor(ui, tokens, doc, view, pending));
        });
    });
    ui.separator();
    export_row(ui, tokens, doc, chooser, ask, pending);
}

/// The linked font's path and the Choose button.
fn font_row(ui: &mut egui::Ui, tokens: &Tokens, doc: &ProjectDoc, chooser: bool, ask: &mut Option<Ask>, pending: &mut Pending) {
    let project = doc.id;
    ui.horizontal(|ui| {
        ui.label("Font");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let hover = if chooser { "Link the project to a font file" } else { "This window has no file chooser: type the font's path" };
            if widgets::button(ui, control_id(project, "choose"), "Choose", hover, chooser, false) {
                *ask = Some(Ask::Font);
            }
            let shown = doc.data.font.as_ref().map(|p| p.to_string_lossy().into_owned());
            match path_field(ui, tokens, control_id(project, "font"), shown.as_deref(), "No font linked", "Choose another font file", chooser) {
                Some(PathEdit::Clicked) => *ask = Some(Ask::Font),
                Some(PathEdit::Typed(path)) => pending.push(("project.linkFont", json!({"project": project, "font": path}))),
                None => {}
            }
        });
    });
    if doc.data.font.is_some() && doc.font.is_none() {
        ui.label(RichText::new("The linked font could not be read: choose it again before exporting.").color(tokens.text_dim));
    }
}

/// The Export Destination path, Save and Export.
fn export_row(ui: &mut egui::Ui, tokens: &Tokens, doc: &ProjectDoc, chooser: bool, ask: &mut Option<Ask>, pending: &mut Pending) {
    let project = doc.id;
    ui.horizontal(|ui| {
        ui.label("Export Destination");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let blocked = export_blocked(doc);
            let hover = blocked.unwrap_or("Export every active instance into the export destination");
            if widgets::button(ui, control_id(project, "export"), "Export", hover, blocked.is_none(), false) {
                pending.push(("project.export", json!({"project": project})));
            }
            let (enabled, hover) = match (&doc.path, chooser) {
                (Some(_), _) => (true, "Save the project to its file"),
                (None, true) => (true, "Choose the file to save the project to"),
                (None, false) => (false, "This window has no file chooser: run project.save with a path"),
            };
            if widgets::button(ui, control_id(project, "save"), "Save", hover, enabled, false) {
                match doc.path {
                    Some(_) => pending.push(("project.save", json!({"project": project}))),
                    None => *ask = Some(Ask::SaveFile),
                }
            }
            let shown = doc.data.destination.as_ref().map(|p| p.to_string_lossy().into_owned());
            let hover = "Choose the folder the project exports into";
            match path_field(ui, tokens, control_id(project, "destination"), shown.as_deref(), "Choose a folder", hover, chooser) {
                Some(PathEdit::Clicked) => *ask = Some(Ask::Destination),
                Some(PathEdit::Typed(folder)) => pending.push(("project.setExportDestination", json!({"project": project, "folder": folder}))),
                None => {}
            }
        });
    });
}

/// A path shown as a button that asks the chooser, or, without a chooser, as a text field that
/// commits on Return.
fn path_field(ui: &mut egui::Ui, tokens: &Tokens, id: Id, shown: Option<&str>, empty: &str, hover: &str, chooser: bool) -> Option<PathEdit> {
    if chooser {
        return widgets::flat_button(ui, id, shown.unwrap_or(empty), hover).then_some(PathEdit::Clicked);
    }
    let width = ui.available_width().max(tokens.field_width);
    param_value::text_field(ui, id, shown.unwrap_or_default(), width, false)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(PathEdit::Typed)
}

// ---------- the list ----------

/// The entries the editor acts on: the selected ones, else the first.
pub fn shown_selection(view: &ProjectView, count: usize) -> Vec<usize> {
    let mut out: Vec<usize> = view.selected.iter().copied().filter(|i| *i < count).collect();
    out.sort_unstable();
    out.dedup();
    if out.is_empty() && count > 0 {
        out.push(0);
    }
    out
}

/// A click on the entry `index`: alone it selects only that entry, with `extend` (Cmd) it adds
/// or drops it, with `range` (Shift) it adds the entries from the last one clicked.
pub fn select(view: &mut ProjectView, index: usize, count: usize, extend: bool, range: bool) {
    let mut selected = shown_selection(view, count);
    match (range, view.anchor) {
        (true, Some(anchor)) => {
            let last = count.saturating_sub(1);
            let (lo, hi) = (anchor.min(index).min(last), anchor.max(index).min(last));
            selected.extend(lo..=hi);
            selected.sort_unstable();
            selected.dedup();
        }
        _ if extend => {
            match selected.iter().position(|i| *i == index) {
                Some(at) => {
                    selected.remove(at);
                }
                None => selected.push(index),
            }
            selected.sort_unstable();
            view.anchor = Some(index);
        }
        _ => {
            selected = vec![index];
            view.anchor = Some(index);
        }
    }
    view.selected = selected;
}

/// The instance list with its plus, minus and revert buttons.
fn instance_list(ui: &mut egui::Ui, tokens: &Tokens, doc: &ProjectDoc, view: &mut ProjectView, pending: &mut Pending) {
    let project = doc.id;
    let count = doc.data.instances.len();
    let selected = shown_selection(view, count);
    let mods = ui.input(|i| i.modifiers);
    for (index, instance) in doc.data.instances.iter().enumerate() {
        let response = entry(ui, tokens, entry_id(project, index), instance, selected.contains(&index));
        if response.drag_started() {
            response.dnd_set_drag_payload(Dragged { project, index });
        }
        if let Some(dragged) = response.dnd_release_payload::<Dragged>() {
            if dragged.project == project {
                dropped(view, project, &selected, count, dragged.index, index, mods.alt, pending);
            }
        } else if response.clicked() {
            select(view, index, count, mods.command, mods.shift);
        }
        response.context_menu(|ui| {
            if widgets::button(ui, duplicate_id(project, index), "Duplicate", "Duplicate the selected instances", true, false) {
                let originals = if selected.contains(&index) { selected.clone() } else { vec![index] };
                pending.push(("project.duplicateInstances", json!({"project": project, "instances": originals})));
                ui.close();
            }
        });
    }
    if count == 0 {
        ui.label(RichText::new("No instances").color(tokens.text_dim));
    }
    ui.add_space(tokens.dialog_spacing);
    ui.horizontal(|ui| {
        let plus = trigger(ui, control_id(project, "plus"), Some("+"), "Add an instance");
        egui::Popup::menu(&plus).show(|ui| {
            if widgets::button(ui, plus_item_id(project, "instance"), "Add Instance", "Add Instance", true, false) {
                pending.push(("project.addInstance", json!({"project": project})));
                view.selected = vec![count];
                view.anchor = Some(count);
            }
            widgets::button(ui, plus_item_id(project, "variableFontSetting"), "Add Variable Font Setting", "Not available yet", false, false);
        });
        if widgets::button(ui, control_id(project, "minus"), "−", "Remove the selected instances", count > 0, false) {
            pending.push(("project.removeInstances", json!({"project": project, "instances": selected})));
            view.selected.clear();
            view.anchor = None;
        }
        let linked = doc.data.font.is_some();
        let hover = if linked { "Revert to the instances of the linked font" } else { "Choose a font first" };
        if widgets::icon_button(ui, control_id(project, "revert"), Icon::RotateCounterclockwise, hover, linked, false) {
            pending.push(("project.revertInstances", json!({"project": project})));
            view.selected.clear();
            view.anchor = None;
        }
    });
}

/// A drop of the entry `from` onto the entry `onto`: a move of the dragged entry (with the rest
/// of the selection when it is selected), or with `alt` a copy put before `onto`.
#[allow(clippy::too_many_arguments)]
fn dropped(view: &mut ProjectView, project: u64, selected: &[usize], count: usize, from: usize, onto: usize, alt: bool, pending: &mut Pending) {
    let moved: Vec<usize> = if selected.contains(&from) { selected.to_vec() } else { vec![from] };
    if alt {
        pending.push(("project.duplicateInstances", json!({"project": project, "instances": moved, "to": onto})));
        view.selected = (onto..onto.saturating_add(moved.len())).collect();
    } else {
        if moved == [onto] {
            return;
        }
        let to = onto.min(count.saturating_sub(moved.len()));
        pending.push(("project.moveInstances", json!({"project": project, "instances": moved, "to": to})));
        view.selected = (to..to.saturating_add(moved.len())).collect();
    }
    view.anchor = view.selected.first().copied();
}

/// The name an entry shows.
fn entry_name(instance: &Instance) -> &str {
    let name = instance.name.trim();
    if name.is_empty() { "Untitled" } else { name }
}

/// One list entry: the icon and the name, highlighted when selected. Takes clicks and drags.
fn entry(ui: &mut egui::Ui, tokens: &Tokens, id: Id, instance: &Instance, selected: bool) -> egui::Response {
    let size = vec2(ui.available_width().max(tokens.list_column_unit), tokens.list_row_height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
        }
        if response.dnd_hover_payload::<Dragged>().is_some() {
            painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(tokens.selection_width, tokens.accent));
        }
        let side = (tokens.list_row_height * 0.6).max(1.0);
        let icon = Rect::from_center_size(pos2(rect.left() + tokens.list_row_height / 2.0, rect.center().y), vec2(side, side));
        let color = icon_color(tokens, instance);
        if is_variable(instance) {
            painter.rect_stroke(icon, f32::from(tokens.radius), Stroke::new(tokens.selection_width, color), StrokeKind::Inside);
            painter.circle_filled(icon.center(), side / 5.0, color);
        } else {
            painter.rect_filled(icon, f32::from(tokens.radius), color);
        }
        let text_color = match (selected, instance.exports) {
            (true, _) => tokens.selected_text,
            (false, true) => tokens.text,
            (false, false) => tokens.text_dim,
        };
        let font = egui::TextStyle::Body.resolve(ui.style());
        let at = pos2(rect.left() + tokens.list_row_height, rect.center().y);
        painter.with_clip_rect(rect).text(at, Align2::LEFT_CENTER, entry_name(instance), font, text_color);
    }
    response.on_hover_text(entry_name(instance))
}

/// A small button that opens a menu: `label`, or a downward triangle without one.
fn trigger(ui: &mut egui::Ui, id: Id, label: Option<&str>, hover: &str) -> egui::Response {
    let side = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        if response.hovered() {
            painter.rect(rect.expand(visuals.expansion), visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        }
        match label {
            Some(text) => {
                let font = egui::TextStyle::Button.resolve(ui.style());
                painter.text(rect.center(), Align2::CENTER_CENTER, text, font, visuals.text_color());
            }
            None => {
                let r = side / 5.0;
                let c = rect.center();
                let points = vec![pos2(c.x - r, c.y - r / 2.0), pos2(c.x + r, c.y - r / 2.0), pos2(c.x, c.y + r / 2.0)];
                painter.add(egui::Shape::convex_polygon(points, visuals.text_color(), Stroke::NONE));
            }
        }
    }
    response.on_hover_text(hover)
}

// ---------- the editor ----------

/// One `project.editInstance` per target with the same `changes`.
fn edits(project: u64, targets: &[usize], changes: &Value) -> Vec<(&'static str, Value)> {
    targets.iter().map(|k| ("project.editInstance", json!({"project": project, "instance": k, "changes": changes}))).collect()
}

/// Whether the entries in `all` differ in what `f` reads.
fn differs<T: PartialEq>(all: &[&Instance], f: impl Fn(&Instance) -> T) -> bool {
    let mut values = all.iter().map(|i| f(i));
    match values.next() {
        Some(first) => values.any(|v| v != first),
        None => false,
    }
}

/// The selected entries' General fields and Custom Parameters.
fn editor(ui: &mut egui::Ui, tokens: &Tokens, doc: &ProjectDoc, view: &mut ProjectView, pending: &mut Pending) {
    let project = doc.id;
    let targets = shown_selection(view, doc.data.instances.len());
    let all: Vec<&Instance> = targets.iter().filter_map(|i| doc.data.instances.get(*i)).collect();
    let Some(first) = all.first().copied() else {
        let text = "This project has no instances. Add one with the plus button, or choose a font to start from its instances.";
        ui.label(RichText::new(text).color(tokens.text_dim));
        return;
    };
    let push = |pending: &mut Pending, key: &str, value: Value| {
        let mut changes = serde_json::Map::new();
        changes.insert(key.to_string(), value);
        pending.extend(edits(project, &targets, &Value::Object(changes)));
    };
    ui.label(RichText::new("General").strong().size(tokens.section_header_size));
    field_row(ui, tokens, "Active", |ui| {
        let mixed = differs(&all, |i| i.exports);
        if check(ui, field_id(project, "active"), first.exports, mixed, "", "Export this instance with the project") {
            push(pending, "active", json!(mixed || !first.exports));
        }
    });
    field_row(ui, tokens, if is_variable(first) { "Name" } else { "Style Name" }, |ui| {
        let mixed = differs(&all, |i| i.name.clone());
        let width = param_value::field_width(ui, tokens);
        if let Some(text) = param_value::text_field(ui, field_id(project, "styleName"), if mixed { "" } else { &first.name }, width, mixed) {
            push(pending, "styleName", json!(text));
        }
    });
    if !is_variable(first) {
        let weight = |i: &Instance| class_value(i.weight_class.as_ref(), WEIGHT_NAMES, DEFAULT_WEIGHT_CLASS);
        if let Some(v) = class_row(ui, tokens, project, "Weight Class", "weightClass", &all, weight, &WEIGHT_CLASSES) {
            push(pending, "weightClass", whole(v));
        }
        let width = |i: &Instance| class_value(i.width_class.as_ref(), WIDTH_NAMES, DEFAULT_WIDTH_CLASS);
        if let Some(v) = class_row(ui, tokens, project, "Width Class", "widthClass", &all, width, &WIDTH_CLASSES) {
            push(pending, "widthClass", whole(v));
        }
        axes_rows(ui, tokens, doc, &all, first);
        field_row(ui, tokens, "Style Linking", |ui| {
            let mixed = differs(&all, |i| i.is_bold);
            if check(ui, field_id(project, "isBold"), first.is_bold, mixed, "Bold", "The bold style of the base instance") {
                push(pending, "isBold", json!(mixed || !first.is_bold));
            }
            let mixed = differs(&all, |i| i.is_italic);
            if check(ui, field_id(project, "isItalic"), first.is_italic, mixed, "Italic", "The italic style of the base instance") {
                push(pending, "isItalic", json!(mixed || !first.is_italic));
            }
            let mixed = differs(&all, |i| i.link_style.clone());
            let shown = if mixed { "" } else { first.link_style.as_deref().unwrap_or("") };
            let width = param_value::field_width(ui, tokens);
            if let Some(text) = param_value::text_field(ui, field_id(project, "linkStyle"), shown, width, mixed) {
                push(pending, "linkStyle", json!(text.trim()));
            }
        });
    }
    added_fields(ui, tokens, first);
    if rows::heading(ui, tokens, control_id(project, "parameters"), "Custom Parameters", "Add a custom parameter") {
        view.picker = Some(ParameterPicker::new(Target { scope: Scope::Instance, indices: targets.clone() }));
    }
    parameter_rows(ui, tokens, project, &all, &targets, pending);
}

/// One labelled row of a built-in field.
fn field_row(ui: &mut egui::Ui, tokens: &Tokens, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        rows::label_cell(ui, tokens, None, label, false, true);
        content(ui);
    });
}

/// A class number as the commands take it: an integer when it is whole (a fraction is passed on
/// for the command to refuse).
fn whole(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1.0e15 { json!(n as i64) } else { json!(n) }
}

/// A class field with its disclosure menu of standard values. The value the user chose.
#[allow(clippy::too_many_arguments)]
fn class_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    project: u64,
    label: &str,
    key: &str,
    all: &[&Instance],
    read: impl Fn(&Instance) -> Option<f64>,
    standard: &[(u16, &str)],
) -> Option<f64> {
    let first = all.first().copied()?;
    let mixed = differs(all, |i| read(i).map(f64::to_bits));
    let value = if mixed { None } else { read(first) };
    let mut chosen = None;
    field_row(ui, tokens, label, |ui| {
        if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, field_id(project, key), value, tokens.field_width, false) {
            chosen = Some(n);
        }
        if mixed {
            ui.label(RichText::new(MULTIPLE_VALUES).color(tokens.text_dim));
        }
        let button = trigger(ui, class_menu_id(project, key), None, "Standard values");
        egui::Popup::menu(&button).show(|ui| {
            for (n, name) in standard {
                let text = format!("{n} {name}");
                if widgets::button(ui, class_item_id(project, key, *n), &text, name, true, value == Some(f64::from(*n))) {
                    chosen = Some(f64::from(*n));
                }
            }
        });
    });
    chosen
}

/// The Axes Coordinates rows: one read-only value per axis of the linked font.
fn axes_rows(ui: &mut egui::Ui, tokens: &Tokens, doc: &ProjectDoc, all: &[&Instance], first: &Instance) {
    let Some(font) = doc.font.as_deref() else { return };
    if font.axes.is_empty() {
        return;
    }
    field_row(ui, tokens, "Axes Coordinates", |ui| {
        ui.label(RichText::new("Read-only").color(tokens.text_dim));
    });
    for (i, axis) in font.axes.iter().enumerate().take(MAX_AXES_SHOWN) {
        let mixed = differs(all, |inst| design_value(&inst.location, i).to_bits());
        let text = if mixed { MULTIPLE_VALUES.to_string() } else { widgets::format_number(design_value(&first.location, i)) };
        field_row(ui, tokens, &axis.name, |ui| {
            ui.label(RichText::new(text).color(tokens.text_dim));
        });
    }
}

/// The General fields the first selected entry carries beside the built-in ones, read-only.
fn added_fields(ui: &mut egui::Ui, tokens: &Tokens, first: &Instance) {
    for property in &first.properties.0 {
        let label = fields::field(Scope::Instance, &property.key).map_or_else(|| property.key.clone(), |f| f.label.to_string());
        let value = property_value(&property.value);
        let languages = rows::languages_of(&value);
        let text = match languages.as_slice() {
            [(_, only)] => only.clone(),
            many => many.iter().map(|(language, text)| format!("{language}: {text}")).collect::<Vec<_>>().join("\n"),
        };
        field_row(ui, tokens, &label, |ui| {
            ui.label(RichText::new(text).color(tokens.text_dim))
                .on_hover_text("Shown read-only: a project edits the built-in fields and the custom parameters");
        });
    }
}

/// The custom parameters of one instance as the shared rows show them.
pub fn parameter_rows_of(instance: &Instance) -> Vec<Row> {
    instance
        .params
        .0
        .iter()
        .map(|p| Row { scope: Scope::Instance, kind: RowKind::Param, key: p.name.clone(), value: p.value.clone(), enabled: !p.disabled })
        .collect()
}

/// Parameter values by (name, occurrence among the parameters of that name).
fn keyed(rows: Vec<Row>) -> HashMap<(String, usize), PlistValue> {
    let mut out = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let count = seen.entry(row.key.clone()).or_insert(0);
        out.insert((row.key, *count), row.value);
        *count = count.saturating_add(1);
    }
    out
}

/// The custom parameter edit of `project.editInstance` a row action makes, or `None` for an
/// action that changes nothing (a click that selects).
pub fn parameter_edit(row: &Row, occurrence: usize, action: &RowAction) -> Option<Value> {
    match action {
        RowAction::Edit(v) => Some(json!({"action": "set", "name": row.key, "index": occurrence, "value": fv::to_json(v)})),
        RowAction::SetEnabled(on) => Some(json!({"action": "set", "name": row.key, "index": occurrence, "enabled": on})),
        RowAction::Remove => Some(json!({"action": "remove", "name": row.key, "index": occurrence})),
        _ => None,
    }
}

/// The first selected entry's parameter rows; a row whose value differs among the selected
/// entries shows the placeholder, and an edit goes to every selected entry.
fn parameter_rows(ui: &mut egui::Ui, tokens: &Tokens, project: u64, all: &[&Instance], targets: &[usize], pending: &mut Pending) {
    let Some(first) = all.first() else { return };
    let others: Vec<HashMap<(String, usize), PlistValue>> = all.iter().skip(1).map(|i| keyed(parameter_rows_of(i))).collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for row in parameter_rows_of(first) {
        let count = seen.entry(row.key.clone()).or_insert(0);
        let occurrence = *count;
        *count = count.saturating_add(1);
        let lookup = (row.key.clone(), occurrence);
        let mut opts = RowOptions::for_row(parameter_id(project, &row.key, occurrence), &row, false);
        opts.mixed = others.iter().any(|o| o.get(&lookup).is_none_or(|v| *v != row.value));
        if let Some(edit) = rows::row_with(ui, tokens, &row, &opts).and_then(|a| parameter_edit(&row, occurrence, &a)) {
            pending.extend(edits(project, targets, &json!({"customParameters": [edit]})));
        }
    }
}

/// A checkbox with a stable id: a check mark when `on`, a dash when the entries differ. True
/// when clicked.
fn check(ui: &mut egui::Ui, id: Id, on: bool, mixed: bool, text: &str, hover: &str) -> bool {
    let side = ui.spacing().interact_size.y;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = (!text.is_empty()).then(|| ui.painter().layout_no_wrap(text.to_string(), font, ui.visuals().text_color()));
    let gap = ui.spacing().item_spacing.x;
    let width = side + galley.as_ref().map_or(0.0, |g| gap + g.size().x);
    let (rect, _) = ui.allocate_exact_size(vec2(width, side), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        let icon = ui.spacing().icon_width;
        let square = Rect::from_center_size(pos2(rect.left() + side / 2.0, rect.center().y), vec2(icon, icon));
        painter.rect(square, visuals.corner_radius, visuals.bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        let inner = square.shrink(icon / 4.0);
        if mixed {
            painter.line_segment([inner.left_center(), inner.right_center()], visuals.fg_stroke);
        } else if on {
            let mid = Pos2::new(inner.left() + inner.width() * 0.4, inner.bottom());
            painter.line_segment([inner.left_center(), mid], visuals.fg_stroke);
            painter.line_segment([mid, inner.right_top()], visuals.fg_stroke);
        }
        if let Some(galley) = galley {
            let at = pos2(rect.left() + side + gap, rect.center().y - galley.size().y / 2.0);
            painter.galley_with_override_text_color(at, galley, visuals.text_color());
        }
    }
    response.on_hover_text(hover).clicked()
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, PointerButton};
    use serde_json::json;

    use fontcraft_model::StyleClass;

    use super::*;
    use crate::FontcraftApp;
    use crate::shell::font_info::exports_tab::PREVIEW_PARAMETER;

    fn input(events: Vec<Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() }
    }

    /// The whole app, one egui context kept across frames.
    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn open(app: &mut FontcraftApp) -> Self {
            let w = Self { ctx: egui::Context::default() };
            w.frame(app, Vec::new());
            w.frame(app, Vec::new());
            w
        }

        fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
            let mut output = self.ctx.run_ui(input(events), |ui| app.ui(ui));
            output.textures_delta.clear();
        }

        fn at(&self, id: Id) -> Pos2 {
            self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center()
        }

        fn click_with(&self, app: &mut FontcraftApp, id: Id, modifiers: Modifiers, button: PointerButton) {
            // A modal whose content changed size is centred again over the next two frames.
            self.frame(app, Vec::new());
            self.frame(app, Vec::new());
            let at = self.at(id);
            self.frame(app, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
            self.frame(app, vec![Event::PointerButton { pos: at, button, pressed: true, modifiers }]);
            self.frame(app, vec![Event::PointerButton { pos: at, button, pressed: false, modifiers }]);
            self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        }

        fn click(&self, app: &mut FontcraftApp, id: Id, modifiers: Modifiers) {
            self.click_with(app, id, modifiers, PointerButton::Primary);
        }

        /// Opens the menu of `menu` and clicks its entry `item`.
        fn pick(&self, app: &mut FontcraftApp, menu: Id, item: Id) {
            self.click(app, menu, Modifiers::NONE);
            self.frame(app, Vec::new());
            self.click(app, item, Modifiers::NONE);
        }

        /// Drags the entry `from` onto the entry `to` with `modifiers` held.
        fn drag(&self, app: &mut FontcraftApp, project: u64, from: usize, to: usize, modifiers: Modifiers) {
            let (start, end) = (self.at(entry_id(project, from)), self.at(entry_id(project, to)));
            let middle = Pos2::new(start.x, (start.y + end.y) / 2.0);
            let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
            self.frame(app, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(start)]);
            self.frame(app, vec![button(start, true)]);
            self.frame(app, vec![Event::PointerMoved(middle)]);
            self.frame(app, vec![Event::PointerMoved(end)]);
            self.frame(app, vec![button(end, false)]);
            self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
            self.frame(app, Vec::new());
        }

        /// Types `text` into the field `id` and presses Return.
        fn type_into(&self, app: &mut FontcraftApp, id: Id, text: &str) {
            self.click(app, id, Modifiers::NONE);
            self.frame(app, vec![Event::Text(text.into())]);
            self.frame(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
            self.frame(app, Vec::new());
        }
    }

    /// An app with a new project holding instances named `names`; the project's id.
    fn project_app(names: &[&str]) -> (FontcraftApp, u64) {
        let mut app = FontcraftApp::default();
        app.run("file.newProject", &json!({})).unwrap();
        for name in names {
            app.run("project.addInstance", &json!({"styleName": name})).unwrap();
        }
        let id = app.session.active_project().unwrap().id;
        (app, id)
    }

    fn instances(app: &FontcraftApp) -> String {
        format!("{:?}", app.session.active_project().unwrap().data.instances)
    }

    #[test]
    fn window_project_list_buttons_give_the_project_what_the_project_commands_give() {
        let (mut app, project) = project_app(&["Light", "Regular"]);
        let font = app.session.font().unwrap().clone();
        let w = Window::open(&mut app);
        w.pick(&mut app, control_id(project, "plus"), plus_item_id(project, "instance"));
        let (mut direct, _) = project_app(&["Light", "Regular"]);
        direct.run("project.addInstance", &json!({})).unwrap();
        assert_eq!(instances(&app), instances(&direct), "Add Instance gives the project what the command gives");
        assert_eq!(view(&w.ctx, project).selected, [2], "the new entry is selected");
        assert!(app.session.font().unwrap().same_as(&font), "the open font is not touched");

        w.click(&mut app, entry_id(project, 0), Modifiers::NONE);
        w.click(&mut app, control_id(project, "minus"), Modifiers::NONE);
        direct.run("project.removeInstances", &json!({"instances": [0]})).unwrap();
        assert_eq!(instances(&app), instances(&direct), "minus removes the selected entry");

        // The context menu's Duplicate copies the entry right after itself.
        w.click_with(&mut app, entry_id(project, 1), Modifiers::NONE, PointerButton::Secondary);
        w.click(&mut app, duplicate_id(project, 1), Modifiers::NONE);
        direct.run("project.duplicateInstances", &json!({"instances": [1]})).unwrap();
        assert_eq!(instances(&app), instances(&direct), "Duplicate copies the entry");
    }

    #[test]
    fn window_project_selection_and_fields_edit_every_selected_instance_through_edit_instance() {
        let names = ["Light", "Regular", "Bold"];
        let (mut app, project) = project_app(&names);
        let w = Window::open(&mut app);
        w.click(&mut app, entry_id(project, 0), Modifiers::NONE);
        w.click(&mut app, entry_id(project, 2), Modifiers::SHIFT);
        assert_eq!(view(&w.ctx, project).selected, [0, 1, 2], "Shift-click selects the range");
        w.click(&mut app, entry_id(project, 1), Modifiers::NONE);
        assert_eq!(view(&w.ctx, project).selected, [1], "a plain click selects one entry");
        w.click(&mut app, entry_id(project, 0), Modifiers::NONE);
        w.click(&mut app, entry_id(project, 2), Modifiers::COMMAND);
        assert_eq!(view(&w.ctx, project).selected, [0, 2], "Cmd-click adds an entry");

        w.pick(&mut app, class_menu_id(project, "weightClass"), class_item_id(project, "weightClass", 700));
        w.click(&mut app, field_id(project, "active"), Modifiers::NONE);
        w.click(&mut app, field_id(project, "isItalic"), Modifiers::NONE);
        w.type_into(&mut app, field_id(project, "linkStyle"), "Regular");

        let (mut direct, _) = project_app(&names);
        for k in [0, 2] {
            for changes in [json!({"weightClass": 700}), json!({"active": false}), json!({"isItalic": true}), json!({"linkStyle": "Regular"})] {
                direct.run("project.editInstance", &json!({"instance": k, "changes": changes})).unwrap();
            }
        }
        assert_eq!(instances(&app), instances(&direct), "the fields give the project what project.editInstance gives");
        let doc = app.session.active_project().unwrap();
        let inactive = doc.data.instances.first().unwrap();
        assert_eq!(icon_color(&app.tokens, inactive), app.tokens.text_dim, "an inactive entry has a grey icon");
    }

    #[test]
    fn window_project_drag_moves_and_alt_drag_duplicates() {
        let names = ["Light", "Regular", "Bold"];
        let (mut app, project) = project_app(&names);
        let w = Window::open(&mut app);
        w.drag(&mut app, project, 0, 2, Modifiers::NONE);
        let (mut direct, _) = project_app(&names);
        direct.run("project.moveInstances", &json!({"instances": [0], "to": 2})).unwrap();
        assert_eq!(instances(&app), instances(&direct), "a drag moves the entry");
        assert_eq!(view(&w.ctx, project).selected, [2], "the moved entry is selected");

        w.click(&mut app, entry_id(project, 0), Modifiers::NONE);
        w.drag(&mut app, project, 0, 1, Modifiers::ALT);
        direct.run("project.duplicateInstances", &json!({"instances": [0], "to": 1})).unwrap();
        assert_eq!(instances(&app), instances(&direct), "an Alt-drag puts a copy where it is dropped");
        assert_eq!(app.session.active_project().unwrap().data.instances.len(), 4);
    }

    #[test]
    fn window_project_parameter_picker_and_rows_act_through_edit_instance() {
        let (mut app, project) = project_app(&["Regular"]);
        let w = Window::open(&mut app);
        let before = instances(&app);
        let search = |w: &Window| {
            let mut v = view(&w.ctx, project);
            let picker = v.picker.as_mut().expect("the parameter picker is open");
            assert_eq!(picker.target, Target { scope: Scope::Instance, indices: vec![0] }, "the picker adds to the selected entry");
            picker.query = PREVIEW_PARAMETER.to_string();
            store(&w.ctx, project, v);
        };
        w.click(&mut app, control_id(project, "parameters"), Modifiers::NONE);
        search(&w);
        w.frame(&mut app, Vec::new());
        w.click(&mut app, custom_parameter_picker::item_id(PREVIEW_PARAMETER), Modifiers::NONE);
        w.click(&mut app, custom_parameter_picker::cancel_id(), Modifiers::NONE);
        assert!(view(&w.ctx, project).picker.is_none(), "Cancel closes the picker");
        assert_eq!(instances(&app), before, "Cancel changes nothing");

        w.click(&mut app, control_id(project, "parameters"), Modifiers::NONE);
        search(&w);
        w.frame(&mut app, Vec::new());
        w.click(&mut app, custom_parameter_picker::item_id(PREVIEW_PARAMETER), Modifiers::NONE);
        w.click(&mut app, custom_parameter_picker::add_id(), Modifiers::NONE);
        let (mut direct, _) = project_app(&["Regular"]);
        let add = json!({"customParameters": [{"action": "add", "name": PREVIEW_PARAMETER}]});
        direct.run("project.editInstance", &json!({"instance": 0, "changes": add})).unwrap();
        assert_eq!(instances(&app), instances(&direct), "the picker adds what the command adds");

        // The row's minus removes it again.
        w.click(&mut app, parameter_id(project, PREVIEW_PARAMETER, 0).with("minus"), Modifiers::NONE);
        assert_eq!(instances(&app), before, "minus removes the parameter");
        let row = Row { scope: Scope::Instance, kind: RowKind::Param, key: "Export".into(), value: PlistValue::Bool(true), enabled: true };
        assert_eq!(
            parameter_edit(&row, 1, &RowAction::SetEnabled(false)),
            Some(json!({"action": "set", "name": "Export", "index": 1, "enabled": false})),
            "the checkbox sets the parameter's use"
        );
        assert_eq!(parameter_edit(&row, 0, &RowAction::Select { extend: false, range: false }), None);
    }

    #[test]
    fn window_project_export_waits_for_a_font_a_destination_and_a_save_and_the_window_closes_with_the_project() {
        let (mut app, project) = project_app(&["Regular"]);
        let w = Window::open(&mut app);
        let doc = |app: &FontcraftApp| app.session.active_project().unwrap().clone();
        assert_eq!(export_blocked(&doc(&app)), Some("Choose the font to export first"));
        // Without a chooser the destination is a text field.
        let folder = std::env::temp_dir().join(format!("fontcraft-project-window-{}", std::process::id()));
        w.type_into(&mut app, control_id(project, "destination"), &folder.to_string_lossy());
        assert_eq!(doc(&app).data.destination.as_deref(), Some(folder.as_path()), "the typed folder is the destination");

        let mut ready = doc(&app);
        let mut data = fontcraft_engine::project::ProjectData::clone(&ready.data);
        data.font = Some(folder.join("Font.glyphs"));
        ready.data = std::sync::Arc::new(data);
        assert_eq!(export_blocked(&ready), Some("Save the project first"));
        ready.path = Some(folder.join("Web.fontcraftproject"));
        assert_eq!(export_blocked(&ready), Some("Save the project's changes first"), "unsaved changes block the export");
        ready.saved = ready.data.clone();
        assert_eq!(export_blocked(&ready), None);

        // A project with unsaved changes stays open; once closed, its window is gone.
        assert!(app.run("project.close", &json!({"project": project})).is_err());
        app.run("project.close", &json!({"project": project, "discard": true})).unwrap();
        w.frame(&mut app, Vec::new());
        w.frame(&mut app, Vec::new());
        assert!(w.ctx.read_response(entry_id(project, 0)).is_none(), "the closed project's window is not drawn");
    }

    #[test]
    fn class_names_of_older_sources_show_as_their_numbers() {
        assert_eq!(class_value(None, WEIGHT_NAMES, 400.0), Some(400.0), "no class is the default");
        assert_eq!(class_value(Some(&StyleClass::Name("SemiBold".into())), WEIGHT_NAMES, 400.0), Some(600.0));
        assert_eq!(class_value(Some(&StyleClass::Name("DemiBold".into())), WEIGHT_NAMES, 400.0), Some(600.0), "an alias");
        assert_eq!(class_value(Some(&StyleClass::Name("Medium (normal)".into())), WIDTH_NAMES, 5.0), Some(5.0));
        assert_eq!(class_value(Some(&StyleClass::Number(300.0)), WEIGHT_NAMES, 400.0), Some(300.0));
    }
}
