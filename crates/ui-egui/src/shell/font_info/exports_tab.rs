//! The Exports tab: the export list at the left (static instances and variable font settings),
//! and the selected entries' preview line, General fields and Custom Parameters at the right.
//!
//! | Control | Does |
//! |---|---|
//! | list entry: click, Cmd-click, Shift-click | selects one entry, adds or drops one, adds a range (view state) |
//! | list entry: drag onto another | `font.instance.reorder` |
//! | plus menu › Add Instance | `font.instance.add` |
//! | plus menu › Add Instance for Each Master, Instance as Master, Add Variable Font Setting | their `font.instance.*` commands; disabled while the engine has no such command |
//! | minus | `font.instance.remove` on the selected entries |
//! | Active | `font.instance.setActive` |
//! | Style Name | `font.instance.set` with `styleName` |
//! | Weight Class, Width Class, and their menus of standard values | `font.instance.setWeightClass`, `font.instance.setWidthClass` |
//! | Style Linking: Bold, Italic, base style name | `font.instance.setStyleLinking` |
//! | Axes Coordinates: one number field per axis | `font.instance.setAxisLocation` with that axis's tag |
//! | General plus, added fields | the field picker; `font.info.*` through the shared rows |
//! | Custom Parameters | [`super::rows::parameters_section`] |
//!
//! Every edit goes to all selected entries at once; a field whose entries differ shows
//! "Multiple Values" or a dash. Inactive entries have a grey icon. A selected instance that lies
//! beyond the masters on some axis gets the extrapolation warning under the Axes Coordinates,
//! the one `font.instance.setAxisLocation` gives. Variable font settings show only Active, the
//! name and the parameters.
//!
//! The preview line draws the first selected instance with [`fontcraft_render::render_line`]:
//! the glyphs of `Aang126` the font has, or the glyph names of the instance's Instance Preview
//! parameter. An instance at a master's location draws that master; one between masters draws
//! its interpolated outlines, from the session's instance view (`Session::instance_view`).
//!
//! Rows: `fontInfo.exports`, `fontInfo.exports.selection`, `fontInfo.exports.inactiveGrayIcon`,
//! `fontInfo.exports.instancePreview`, the `Instance Preview` parameter (M7.18);
//! `instance.axesValues`, `font.instance.setAxisLocation`, `fontInfo.instance.extrapolation`
//! (M9.27).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Align2, Color32, ColorImage, Id, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::RowKind;
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_engine::interp_cache::view_master_id;
use fontcraft_model::axes::design_value;
use fontcraft_model::{Font, Instance, StyleClass};
use fontcraft_render::{LineOptions, Style, render_line};
use serde_json::{Value, json};

use super::param_value::{self, MULTIPLE_VALUES};
use super::rows::{Target, heading, plus_id};
use super::{FontInfoState, Picker};
use crate::app::FontcraftApp;
use crate::dialogs::field_picker::FieldPicker;
use crate::panels::{Pending, widgets};
use crate::theme::Tokens;

/// The characters the preview shows when the instance names no glyphs.
pub const DEFAULT_PREVIEW: &str = "Aang126";
/// The parameter that names the preview's glyphs.
pub const PREVIEW_PARAMETER: &str = "Instance Preview";
/// Most glyphs one preview line draws.
pub const MAX_PREVIEW_GLYPHS: usize = 64;
/// Most axes the Axes Coordinates rows list.
const MAX_AXES_SHOWN: usize = 64;
/// Most extrapolation warnings the Axes Coordinates show.
const MAX_WARNINGS_SHOWN: usize = 8;
/// The list's width, in list columns of the theme.
const LIST_COLUMNS: f32 = 4.0;
/// The preview's height, in list rows of the theme.
const PREVIEW_ROWS: f32 = 3.0;
/// Empty pixels around the preview's glyphs.
const PREVIEW_PADDING: u32 = 4;
/// Most times the preview is drawn again, smaller, to fit the largest texture.
const PREVIEW_FIT_TRIES: usize = 3;

/// The standard weight classes the Weight Class menu lists (OpenType `OS/2` names).
pub const WEIGHT_CLASSES: [(u16, &str); 9] = [
    (100, "Thin"),
    (200, "Extra-light"),
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "Semi-bold"),
    (700, "Bold"),
    (800, "Extra-bold"),
    (900, "Black"),
];
/// The width classes the Width Class menu lists (OpenType `OS/2` names).
pub const WIDTH_CLASSES: [(u16, &str); 9] = [
    (1, "Ultra-condensed"),
    (2, "Extra-condensed"),
    (3, "Condensed"),
    (4, "Semi-condensed"),
    (5, "Medium (normal)"),
    (6, "Semi-expanded"),
    (7, "Expanded"),
    (8, "Extra-expanded"),
    (9, "Ultra-expanded"),
];
/// Class names older sources store instead of numbers, letters and digits only, lower case.
pub const WEIGHT_NAMES: &[(&str, u16)] = &[
    ("thin", 100),
    ("hairline", 100),
    ("extralight", 200),
    ("ultralight", 200),
    ("light", 300),
    ("normal", 400),
    ("regular", 400),
    ("medium", 500),
    ("semibold", 600),
    ("demibold", 600),
    ("bold", 700),
    ("extrabold", 800),
    ("ultrabold", 800),
    ("black", 900),
    ("heavy", 900),
];
pub const WIDTH_NAMES: &[(&str, u16)] = &[
    ("ultracondensed", 1),
    ("extracondensed", 2),
    ("condensed", 3),
    ("semicondensed", 4),
    ("medium", 5),
    ("mediumnormal", 5),
    ("normal", 5),
    ("semiexpanded", 6),
    ("expanded", 7),
    ("extraexpanded", 8),
    ("ultraexpanded", 9),
];
/// The class an instance has when its source states none (as `font.instance.list` reports it).
const DEFAULT_WEIGHT_CLASS: f64 = 400.0;
const DEFAULT_WIDTH_CLASS: f64 = 5.0;

/// The plus menu's entries: command id and label. Add Instance is always there; the others are
/// enabled once the engine has their command.
pub const PLUS_MENU: [(&str, &str); 4] = [
    ("font.instance.add", "Add Instance"),
    ("font.instance.addForEachMaster", "Add Instance for Each Master"),
    ("font.instance.toMaster", "Instance as Master"),
    ("font.instance.addVariableFontSetting", "Add Variable Font Setting"),
];

/// The id of the list entry of the instance at `index`.
pub fn entry_id(index: usize) -> Id {
    widgets::id(("fontInfo.exports.entry", index))
}

/// The id of the plus button below the list.
pub fn plus_menu_id() -> Id {
    widgets::id(("fontInfo.exports", "plus"))
}

/// The id of a plus menu entry, by its command id.
pub fn plus_item_id(command: &str) -> Id {
    widgets::id(("fontInfo.exports.plusItem", command))
}

/// The id of the minus button below the list.
pub fn minus_id() -> Id {
    widgets::id(("fontInfo.exports", "minus"))
}

/// The id of a built-in control: `active`, `styleName`, `weightClass`, `widthClass`, `isBold`,
/// `isItalic`, `linkStyle`.
pub fn field_id(key: &str) -> Id {
    widgets::id(("fontInfo.exports.field", key))
}

/// The id of the disclosure button beside `weightClass` or `widthClass`.
pub fn class_menu_id(key: &str) -> Id {
    field_id(key).with("menu")
}

/// The id of the entry `value` in the class menu of `key`.
pub fn class_item_id(key: &str, value: u16) -> Id {
    field_id(key).with(("item", value))
}

/// The id of the coordinate field of axis `at`.
pub fn axis_field_id(at: usize) -> Id {
    field_id("axis").with(at)
}

/// The id of the `n`th extrapolation warning under the Axes Coordinates.
pub fn extrapolation_id(n: usize) -> Id {
    field_id("extrapolation").with(n)
}

/// The id of the preview line.
pub fn preview_id() -> Id {
    widgets::id(("fontInfo.exports", "preview"))
}

/// The entry a Shift-click selects from, kept in the context's memory.
fn anchor_id() -> Id {
    widgets::id(("fontInfo.exports", "anchor"))
}

/// Draws the tab's contents inside the Font Info window.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let doc = app.session.active().map(|d| (d.id, d.revision));
    let FontcraftApp { session, state, tokens, .. } = app;
    // An instance between masters is drawn from its interpolated view, built before the font is
    // borrowed for the rest of the tab.
    let between = {
        let Ok(font) = session.font() else { return };
        let fi = &state.panels.font_info;
        let index = fi.target(font).map(|t| t.first());
        index.and_then(|i| preview_glyphs(font, i).ok().filter(|(m, _)| *m == view_master_id(i)).map(|(_, names)| (i, names)))
    };
    let view = between.map(|(i, names)| session.instance_view(i, &names));
    let Ok(font) = session.font() else { return };
    let fi = &mut state.panels.font_info;
    let count = font.instances.len();
    fi.selected_instances.retain(|i| *i < count);
    let tokens: &Tokens = tokens;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(tokens.list_column_unit * LIST_COLUMNS);
            export_list(ui, tokens, fi, font, pending);
        });
        ui.separator();
        ui.vertical(|ui| {
            let Some(target) = fi.target(font) else {
                ui.label(egui::RichText::new("This font has no exports. Add one with the plus button below the list.").color(tokens.text_dim));
                return;
            };
            preview(ui, tokens, font, target.first(), doc, view.as_ref());
            general(ui, tokens, fi, font, &target, pending);
            super::rows::parameters_section(ui, tokens, fi, font, &target, pending);
        });
    });
}

// ---------- the list ----------

/// What a drag of a list entry carries: the entry's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DraggedExport(usize);

/// The entries the tab acts on: the selected ones, else the first.
pub fn shown_selection(fi: &FontInfoState, count: usize) -> Vec<usize> {
    let mut out: Vec<usize> = fi.selected_instances.iter().copied().filter(|i| *i < count).collect();
    out.sort_unstable();
    out.dedup();
    if out.is_empty() && count > 0 {
        out.push(0);
    }
    out
}

/// A click on the entry `index`: alone it selects only that entry, with `extend` (Cmd) it adds
/// or drops it, with `range` (Shift) it adds the entries from the last one clicked.
pub fn select_entry(fi: &mut FontInfoState, ctx: &egui::Context, index: usize, count: usize, extend: bool, range: bool) {
    let anchor: Option<usize> = ctx.data(|d| d.get_temp(anchor_id()));
    let mut selected = shown_selection(fi, count);
    match (range, anchor) {
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
            ctx.data_mut(|d| d.insert_temp(anchor_id(), index));
        }
        _ => {
            selected = vec![index];
            ctx.data_mut(|d| d.insert_temp(anchor_id(), index));
        }
    }
    fi.selected_instances = selected;
    // The rows on the right belong to other entries now.
    fi.clear_selection();
}

/// The colour of an entry's icon: the accent colour, grey when the entry is not exported.
pub fn icon_color(tokens: &Tokens, instance: &Instance) -> Color32 {
    if instance.exports { tokens.accent } else { tokens.text_dim }
}

/// Whether the entry is a variable font setting rather than a static instance.
pub fn is_variable(instance: &Instance) -> bool {
    instance.kind.as_deref() == Some("variable")
}

/// The name an entry shows.
fn entry_name(instance: &Instance) -> &str {
    let name = instance.name.trim();
    if name.is_empty() { "Untitled" } else { name }
}

/// The export list with its plus and minus buttons.
fn export_list(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, pending: &mut Pending) {
    let count = font.instances.len();
    let selected = shown_selection(fi, count);
    let mods = ui.input(|i| i.modifiers);
    for (index, instance) in font.instances.iter().enumerate() {
        let response = entry(ui, tokens, index, instance, selected.contains(&index));
        if response.drag_started() {
            response.dnd_set_drag_payload(DraggedExport(index));
        }
        if let Some(from) = response.dnd_release_payload::<DraggedExport>() {
            let DraggedExport(from) = *from;
            if from != index {
                pending.push(("font.instance.reorder", json!({"instance": from, "toIndex": index})));
                fi.selected_instances = vec![index];
                fi.clear_selection();
            }
        } else if response.clicked() {
            select_entry(fi, ui.ctx(), index, count, mods.command, mods.shift);
        }
    }
    ui.add_space(tokens.dialog_spacing);
    ui.horizontal(|ui| {
        let plus = trigger(ui, plus_menu_id(), Some("+"), "Add an export");
        egui::Popup::menu(&plus).show(|ui| {
            for (command, label) in PLUS_MENU {
                let enabled = fontcraft_engine::find_command(command).is_some() && (command != "font.instance.toMaster" || count > 0);
                let hover = if enabled { label } else { "Not available yet" };
                if widgets::button(ui, plus_item_id(command), label, hover, enabled, false) {
                    pending.push((command, plus_params(command, &selected)));
                    if command == "font.instance.add" {
                        fi.selected_instances = vec![count];
                        fi.clear_selection();
                    }
                }
            }
        });
        if widgets::button(ui, minus_id(), "−", "Remove the selected exports", count > 0, false) {
            pending.push(("font.instance.remove", json!({"instances": selected})));
            fi.selected_instances.clear();
            fi.clear_selection();
        }
    });
}

/// The parameters of a plus menu command.
fn plus_params(command: &str, selected: &[usize]) -> Value {
    match command {
        "font.instance.toMaster" => json!({"instance": selected.first().copied().unwrap_or(0)}),
        _ => json!({}),
    }
}

/// One list entry: the icon and the name, highlighted when selected. Takes clicks and drags.
fn entry(ui: &mut egui::Ui, tokens: &Tokens, index: usize, instance: &Instance, selected: bool) -> egui::Response {
    let size = vec2(ui.available_width().max(tokens.list_column_unit), tokens.list_row_height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, entry_id(index), Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, f32::from(tokens.radius), tokens.row_selected);
        }
        if response.dnd_hover_payload::<DraggedExport>().is_some() {
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
pub(super) fn trigger(ui: &mut egui::Ui, id: Id, label: Option<&str>, hover: &str) -> egui::Response {
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

// ---------- the fields ----------

/// Whether the entries in `all` differ in what `f` reads.
fn differs<T: PartialEq>(all: &[&Instance], f: impl Fn(&Instance) -> T) -> bool {
    let mut values = all.iter().map(|i| f(i));
    match values.next() {
        Some(first) => values.any(|v| v != first),
        None => false,
    }
}

/// The General heading, the built-in fields of the selected entries, and their added fields.
fn general(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    if heading(ui, tokens, plus_id(Scope::Instance, "general"), "General", "Add a field") {
        fi.picker = Some(Picker::Field(FieldPicker::new(target.clone())));
    }
    let all: Vec<&Instance> = target.indices.iter().filter_map(|i| font.instances.get(*i)).collect();
    let Some(first) = all.first().copied() else { return };
    let instances = json!(target.indices);
    let command = |id: &'static str, key: &str, value: Value| (id, json!({"instances": instances, key: value}));

    field_row(ui, tokens, "Active", |ui| {
        if check(ui, field_id("active"), first.exports, differs(&all, |i| i.exports), "", "Export this entry with File › Export") {
            let on = differs(&all, |i| i.exports) || !first.exports;
            pending.push(command("font.instance.setActive", "active", json!(on)));
        }
    });
    field_row(ui, tokens, if is_variable(first) { "Name" } else { "Style Name" }, |ui| {
        let mixed = differs(&all, |i| i.name.clone());
        let width = param_value::field_width(ui, tokens);
        if let Some(text) = param_value::text_field(ui, field_id("styleName"), if mixed { "" } else { &first.name }, width, mixed) {
            pending.push(command("font.instance.set", "styleName", json!(text)));
        }
    });
    if !is_variable(first) {
        let weight = |i: &Instance| class_value(i.weight_class.as_ref(), WEIGHT_NAMES, DEFAULT_WEIGHT_CLASS);
        if let Some(v) = class_row(ui, tokens, "Weight Class", "weightClass", &all, weight, &WEIGHT_CLASSES) {
            pending.push(command("font.instance.setWeightClass", "weightClass", class_json(v)));
        }
        let width = |i: &Instance| class_value(i.width_class.as_ref(), WIDTH_NAMES, DEFAULT_WIDTH_CLASS);
        if let Some(v) = class_row(ui, tokens, "Width Class", "widthClass", &all, width, &WIDTH_CLASSES) {
            pending.push(command("font.instance.setWidthClass", "widthClass", class_json(v)));
        }
        axes_rows(ui, tokens, font, &all, first, &target.indices, pending);
        field_row(ui, tokens, "Style Linking", |ui| {
            if check(ui, field_id("isBold"), first.is_bold, differs(&all, |i| i.is_bold), "Bold", "The bold style of the base instance") {
                let on = differs(&all, |i| i.is_bold) || !first.is_bold;
                pending.push(command("font.instance.setStyleLinking", "isBold", json!(on)));
            }
            if check(ui, field_id("isItalic"), first.is_italic, differs(&all, |i| i.is_italic), "Italic", "The italic style of the base instance") {
                let on = differs(&all, |i| i.is_italic) || !first.is_italic;
                pending.push(command("font.instance.setStyleLinking", "isItalic", json!(on)));
            }
            let mixed = differs(&all, |i| i.link_style.clone());
            let shown = if mixed { "" } else { first.link_style.as_deref().unwrap_or("") };
            let width = param_value::field_width(ui, tokens);
            if let Some(text) = param_value::text_field(ui, field_id("linkStyle"), shown, width, mixed) {
                pending.push(command("font.instance.setStyleLinking", "linkStyle", json!(text.trim())));
            }
        });
    }
    added_fields(ui, tokens, fi, font, target, pending);
}

/// One labelled row of a built-in field.
fn field_row(ui: &mut egui::Ui, tokens: &Tokens, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        field_label(ui, tokens, label);
        content(ui);
    });
}

/// A field's label, right-aligned in the label column.
fn field_label(ui: &mut egui::Ui, tokens: &Tokens, label: &str) {
    super::rows::label_cell(ui, tokens, None, label, false, true);
}

/// A class field with its disclosure menu of standard values. The value the user chose.
fn class_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
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
        if let Some(widgets::NumberEdit::Set(n)) = widgets::number(ui, field_id(key), value, tokens.field_width, false) {
            chosen = Some(n);
        }
        if mixed {
            ui.label(egui::RichText::new(MULTIPLE_VALUES).color(tokens.text_dim));
        }
        let button = trigger(ui, class_menu_id(key), None, "Standard values");
        egui::Popup::menu(&button).show(|ui| {
            for (n, name) in standard {
                let text = format!("{n} {name}");
                if widgets::button(ui, class_item_id(key, *n), &text, name, true, value == Some(f64::from(*n))) {
                    chosen = Some(f64::from(*n));
                }
            }
        });
    });
    chosen
}

/// A class as the field shows it: its number, a known name's number, or the default when the
/// source states none. `None` for a name the tables do not know.
pub fn class_value(class: Option<&StyleClass>, names: &[(&str, u16)], default: f64) -> Option<f64> {
    match class {
        None => Some(default),
        Some(StyleClass::Number(n)) => n.is_finite().then_some(*n),
        Some(StyleClass::Name(name)) => {
            let key: String = name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
            names.iter().find(|(k, _)| *k == key).map(|(_, n)| f64::from(*n))
        }
    }
}

/// A class number as the commands take it: an integer when it is whole (a fraction is passed on
/// for the command to refuse).
fn class_json(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1.0e15 { json!(n as i64) } else { json!(n) }
}

/// The Axes Coordinates rows: one number field per font axis, Multiple Values where the
/// entries differ, then the extrapolation warnings of the entries beyond the masters.
fn axes_rows(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, all: &[&Instance], first: &Instance, indices: &[usize], pending: &mut Pending) {
    if font.axes.is_empty() {
        return;
    }
    field_row(ui, tokens, "Axes Coordinates", |_| {});
    for (i, axis) in font.axes.iter().enumerate().take(MAX_AXES_SHOWN) {
        let mixed = differs(all, |inst| design_value(&inst.location, i).to_bits());
        let value = (!mixed).then(|| design_value(&first.location, i));
        field_row(ui, tokens, &axis.name, |ui| {
            if let Some(v) = super::masters_tab::number_field(ui, axis_field_id(i), value, tokens.field_width) {
                let mut location = serde_json::Map::new();
                location.insert(axis.tag.clone(), json!(v));
                pending.push(("font.instance.setAxisLocation", json!({"instances": indices, "location": location})));
            }
        });
    }
    let warnings: Vec<String> = all.iter().filter_map(|inst| extrapolation_warning(font, inst)).collect();
    for (n, warning) in warnings.iter().enumerate().take(MAX_WARNINGS_SHOWN) {
        ui.horizontal_top(|ui| {
            field_label(ui, tokens, "");
            ui.vertical(|ui| {
                let text = egui::RichText::new(sentence(warning)).color(tokens.audit_warn);
                let label = ui.add(egui::Label::new(text).wrap());
                ui.interact(label.rect, extrapolation_id(n), Sense::hover());
            });
        });
    }
    if warnings.len() > MAX_WARNINGS_SHOWN {
        field_row(ui, tokens, "", |ui| {
            ui.label(egui::RichText::new(format!("and {} more extrapolated", warnings.len() - MAX_WARNINGS_SHOWN)).color(tokens.audit_warn));
        });
    }
}

/// The warning `font.instance.setAxisLocation` gives for `instance`: the tags of the axes on
/// which it lies beyond every master (`fontInfo.instance.extrapolation`). `None` for an
/// instance inside the masters, a font without masters, and a variable font setting.
pub fn extrapolation_warning(font: &Font, instance: &Instance) -> Option<String> {
    if is_variable(instance) {
        return None;
    }
    let outside: Vec<&str> = font
        .axes
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let mut coordinates = font.masters.iter().map(|m| design_value(&m.location, *i));
            let Some(start) = coordinates.next() else { return false };
            let (lo, hi) = coordinates.fold((start, start), |(lo, hi), x| (lo.min(x), hi.max(x)));
            let v = design_value(&instance.location, *i);
            v < lo || v > hi
        })
        .map(|(_, a)| a.tag.as_str())
        .collect();
    (!outside.is_empty()).then(|| format!("instance `{}` lies beyond the masters on {}; it is extrapolated", instance.name, outside.join(", ")))
}

/// `text` with its first letter capitalised, as a line on its own reads.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The fields added under General (familyNames, styleNames…), drawn with the shared row: they
/// are selectable, copyable and removable as on the other tabs.
fn added_fields(ui: &mut egui::Ui, tokens: &Tokens, fi: &mut FontInfoState, font: &Font, target: &Target, pending: &mut Pending) {
    super::rows::section_rows(ui, tokens, fi, font, target, RowKind::Field, pending);
}

/// A checkbox with a stable id: a check mark when `on`, a dash when the entries differ. True
/// when clicked.
pub(super) fn check(ui: &mut egui::Ui, id: Id, on: bool, mixed: bool, text: &str, hover: &str) -> bool {
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

// ---------- the preview ----------

/// The master and glyphs the preview of the instance at `index` draws, or why it draws none.
///
/// The glyphs are those of the instance's enabled Instance Preview parameter (glyph names
/// separated by spaces or line breaks), else the glyphs mapped to the characters of
/// [`DEFAULT_PREVIEW`]; names the font lacks are skipped, and at most [`MAX_PREVIEW_GLYPHS`]
/// are drawn. The master is the only one, the one at the instance's location, or, for an instance
/// between masters, the master of its interpolated view ([`view_master_id`]).
pub fn preview_glyphs(font: &Font, index: usize) -> Result<(String, Vec<String>), &'static str> {
    let instance = font.instances.get(index).ok_or("There is no such export.")?;
    if is_variable(instance) {
        return Err("A variable font setting has no single preview.");
    }
    let at_location =
        |m: &&fontcraft_model::Master| (0..font.axes.len()).all(|i| design_value(&m.location, i) == design_value(&instance.location, i));
    let master = match font.masters.as_slice() {
        [] => return Err("The font has no master."),
        [only] => only.id.clone(),
        all => all.iter().find(at_location).map_or_else(|| view_master_id(index), |m| m.id.clone()),
    };
    let parameter = instance.params.0.iter().find(|p| p.name == PREVIEW_PARAMETER && !p.disabled);
    let names: Vec<String> = match parameter {
        Some(p) => fv::lines(&p.value)
            .unwrap_or_default()
            .into_iter()
            .flat_map(str::split_whitespace)
            .filter(|n| font.glyph(n).is_some())
            .take(MAX_PREVIEW_GLYPHS)
            .map(str::to_string)
            .collect(),
        None => {
            DEFAULT_PREVIEW.chars().filter_map(|c| font.glyphs.iter().find(|g| g.unicodes.contains(&u32::from(c)))).map(|g| g.name.clone()).collect()
        }
    };
    if names.is_empty() {
        return Err("The font has none of the preview's glyphs.");
    }
    Ok((master, names))
}

/// The preview's texture and what it was drawn from.
#[derive(Clone)]
struct PreviewTexture {
    key: u64,
    texture: Option<TextureHandle>,
}

/// The preview line of the instance at `index`, redrawn only when the document, the instance or
/// the size changes. `view` is the instance's interpolated view when it lies between masters.
fn preview(ui: &mut egui::Ui, tokens: &Tokens, font: &Font, index: usize, doc: Option<(u64, u64)>, view: Option<&Result<Arc<Font>, String>>) {
    let height = tokens.list_row_height * PREVIEW_ROWS;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(1.0), height), Sense::hover());
    let _ = ui.interact(rect, preview_id(), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(rect, f32::from(tokens.radius), tokens.canvas);
    let message = |ui: &egui::Ui, text: &str| {
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.painter().with_clip_rect(rect).text(rect.center(), Align2::CENTER_CENTER, text, font, tokens.text_dim);
    };
    let (master, names) = match preview_glyphs(font, index) {
        Ok(found) => found,
        Err(why) => return message(ui, why),
    };
    let drawn: &Font = if master == view_master_id(index) {
        match view {
            Some(Ok(view)) => view,
            Some(Err(e)) => return message(ui, &format!("The instance could not be interpolated: {e}")),
            None => return message(ui, "The instance could not be interpolated."),
        }
    } else {
        font
    };
    let ppp = ui.ctx().pixels_per_point();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (doc, index, &master, &names, height.to_bits(), ppp.to_bits(), tokens.glyph_fill.to_array()).hash(&mut hasher);
    let key = hasher.finish();
    let cache = preview_id().with("texture");
    let kept: Option<PreviewTexture> = ui.ctx().data(|d| d.get_temp(cache));
    let texture = match kept.filter(|k| k.key == key) {
        Some(k) => k.texture,
        None => {
            let max_side = ui.ctx().input(|i| i.max_texture_side);
            let texture = render_preview(ui.ctx(), tokens, drawn, &master, &names, height * ppp, max_side);
            ui.ctx().data_mut(|d| d.insert_temp(cache, PreviewTexture { key, texture: texture.clone() }));
            texture
        }
    };
    let Some(texture) = texture else { return message(ui, "The preview could not be drawn.") };
    let size = texture.size_vec2() / ppp.max(f32::EPSILON);
    let fit = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0)).min(1.0);
    let shown = size * fit;
    let at = Rect::from_min_size(pos2(rect.left(), rect.center().y - shown.y / 2.0), shown);
    ui.painter().with_clip_rect(rect).image(texture.id(), at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// Draws `names` of `master` into a texture `pixels` high (about: the line spans the master's
/// descender to ascender), drawn smaller where a line that high would be wider than `max_side`
/// pixels, the largest texture the graphics backend takes.
fn render_preview(
    ctx: &egui::Context,
    tokens: &Tokens,
    font: &Font,
    master: &str,
    names: &[String],
    pixels: f32,
    max_side: usize,
) -> Option<TextureHandle> {
    let [r, g, b, a] = tokens.glyph_fill.to_srgba_unmultiplied();
    let padding = f64::from(PREVIEW_PADDING);
    let mut em = f64::from(pixels) - 2.0 * padding;
    let glyphs: Vec<&str> = names.iter().map(String::as_str).collect();
    // Each try scales the em by how far the last image overshot; rounding can leave it a pixel
    // over, which the next try settles.
    for _ in 0..PREVIEW_FIT_TRIES {
        if !(em.is_finite() && em >= 1.0) {
            return None;
        }
        let options = LineOptions {
            master: Some(master.to_string()),
            size: Some(em),
            scale: None,
            padding: PREVIEW_PADDING,
            style: Style { ink: [r, g, b, a], background: [0, 0, 0, 0], anti_alias: true },
        };
        let image = render_line(font, &glyphs, &options).ok()?;
        let (width, height) = (image.width as usize, image.height as usize);
        let longest = width.max(height);
        if longest <= max_side {
            let colour = ColorImage::from_rgba_premultiplied([width, height], &image.pixels);
            return Some(ctx.load_texture("font-info-instance-preview", colour, TextureOptions::LINEAR));
        }
        let room = (max_side as f64 - 2.0 * padding).max(0.0);
        let drawn = (longest as f64 - 2.0 * padding).max(1.0);
        em = (em * room / drawn).floor();
    }
    None
}

#[cfg(test)]
#[path = "exports_tab_tests.rs"]
mod tests;
