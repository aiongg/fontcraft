//! The Attributes inspector of full-colour layers: stroke, fill, gradient and its gradient bar,
//! each control running an `attributes.*` command. Package M11.15 (`docs/plans/M11.md` § 4);
//! rows `inspector.attributes`, `inspector.attributes.gradientBar`.
//!
//! It shows at the foot of the Palette while Show Info is on, the edited layer is a full-colour
//! layer and a path is selected (a path selected whole or with a selected node or handle). Every
//! control acts on those paths; with several, a control shows a value only when every path has
//! it.
//!
//! | Control | Command |
//! |---|---|
//! | plus menu: Gradient, Shadow, Inner Shadow, Pattern Image | `attributes.addAttribute` |
//! | Stroke width | `attributes.setStrokeWidth` |
//! | Stroke alignment: centre, left (inside), right (outside) | `attributes.setStrokeAlignment` |
//! | Stroke swatch | `attributes.setStrokeColor` |
//! | Stroke minus | `attributes.removeStroke` |
//! | Fill swatch (hidden while a gradient is present) | `attributes.setFillColor` |
//! | Fill minus | `attributes.removeFill` |
//! | Gradient type: linear, radial | `attributes.setGradientType` |
//! | Gradient minus | `attributes.removeGradient` |
//! | a click on the gradient bar | `attributes.addGradientStop` at that offset |
//! | a click on a stop's knob, then a colour | `attributes.setGradientStopColor` with `color` |
//! | a knob dragged along the bar | `attributes.setGradientStopColor` with `offset` |
//! | a knob dragged away from the bar | `attributes.removeGradientStop` (never below two stops) |
//!
//! A swatch with a red diagonal line is a fully transparent colour. A colour picked in a swatch's
//! picker runs its command once, when the picker closes, so a drag through the picker is one
//! undo step. The swatch and picker helpers ([`color_button`], [`picker`]) also serve the colour
//! palettes editor and the Master Color picker.

use egui::color_picker::{self, Alpha};
use egui::{Color32, Id, Popup, PopupCloseBehavior, Rect, Response, Sense, Stroke, StrokeKind};
use fontcraft_engine::color::layers::{self as color_layers, ColorLayerKind};
use fontcraft_engine::color::palette::{self as color_palette, Palettes};
use fontcraft_model::{Color, Dimension, Gradient, GradientKind, Shape, ShapeAttrs};
use serde_json::{Map, Value, json};

use super::{Pending, edit_targets, stroke, widgets, with};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::theme::Tokens;
use crate::tools_glue;

/// A colour as four unmultiplied bytes: red, green, blue, alpha.
pub type Rgba8 = [u8; 4];

/// A stop dragged this many bar heights below or above the bar is removed.
const REMOVE_DISTANCE: f32 = 2.0;
/// A swatch is this many control heights wide.
const SWATCH_WIDTH: f32 = 1.5;
/// The palette row of the text colour.
const FOREGROUND: u16 = 0xFFFF;

/// The stroke alignments the buttons offer: (word, label, hover, stored position).
const ALIGNMENTS: [(&str, &str, &str, f64); 3] = [
    ("center", "Centre", "Stroke centred on the path", 0.0),
    ("inside", "Left", "Stroke inside the path (left of its direction)", -1.0),
    ("outside", "Right", "Stroke outside the path (right of its direction)", 1.0),
];

/// The styles the plus menu adds: (attribute, label).
const ADDABLE: [(&str, &str); 4] =
    [("gradient", "Gradient"), ("shadow", "Shadow"), ("innerShadow", "Inner Shadow"), ("patternImage", "Pattern Image")];

// ── Colours ─────────────────────────────────────────────────────────────────────────────────────

/// `#RRGGBBAA`.
pub fn hex(c: Rgba8) -> String {
    let [r, g, b, a] = c;
    format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}

/// `#RRGGBB` or `#RRGGBBAA`, either case.
pub fn parse_hex(text: &str) -> Option<Rgba8> {
    color_palette::Rgba::parse_hex(text).map(|c| [c.r, c.g, c.b, c.a])
}

fn color32(c: Rgba8) -> Color32 {
    let [r, g, b, a] = c;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// A component from 0 to 1 as a byte; anything not finite is 0.
fn byte(v: f64) -> u8 {
    if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 }
}

/// A model colour as the swatches show it: a palette entry is looked up in palette 0 of
/// `palettes`, its alpha multiplied in; the text colour is `foreground`. `None` for a palette row
/// the palettes do not have.
pub fn resolve(color: &Color, palettes: &Palettes, foreground: Color32) -> Option<Rgba8> {
    match color {
        Color::Rgba([r, g, b, a]) => Some([byte(*r), byte(*g), byte(*b), byte(*a)]),
        Color::Palette { index, alpha } => {
            let [r, g, b, a] = if *index == FOREGROUND {
                foreground.to_srgba_unmultiplied()
            } else {
                let entry = palettes.get(0, usize::from(*index))?;
                [entry.r, entry.g, entry.b, entry.a]
            };
            let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 };
            Some([r, g, b, (f64::from(a) * alpha).round() as u8])
        }
    }
}

/// Paints a swatch: the colour over the panel, crossed by a red diagonal when fully transparent,
/// a dash when the paths disagree (`None`).
pub fn paint_swatch(painter: &egui::Painter, rect: Rect, color: Option<Rgba8>, tokens: &Tokens) {
    let radius = f32::from(tokens.radius);
    match color {
        Some(c) => {
            painter.rect_filled(rect, radius, color32(c));
            if let [_, _, _, 0] = c {
                painter.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(tokens.outline_width, tokens.error_text));
            }
        }
        None => {
            let font = egui::TextStyle::Button.resolve(&egui::Style::default());
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "–", font, tokens.text_dim);
        }
    }
    painter.rect_stroke(rect, radius, Stroke::new(tokens.outline_width, tokens.border), StrokeKind::Inside);
}

/// A swatch that opens a colour picker with an opacity slider. Returns the colour picked, once,
/// when the picker closes on a colour other than `color`.
pub fn color_button(ui: &mut egui::Ui, id: Id, color: Option<Rgba8>, hover: &str, tokens: &Tokens) -> Option<Rgba8> {
    let side = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side * SWATCH_WIDTH, side), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let shown = working(ui.ctx(), id).or(color);
        paint_swatch(ui.painter(), rect.shrink(ui.spacing().button_padding.y / 2.0), shown, tokens);
    }
    let response = response.on_hover_text(hover);
    picker(ui, &response, id, color)
}

/// A button with a stable id whose response opens a menu or picker ([`Popup::menu`]).
pub fn menu_button(ui: &mut egui::Ui, id: Id, label: &str, hover: &str) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, ui.visuals().text_color());
    let side = ui.spacing().interact_size.y;
    let size = (galley.size() + 2.0 * ui.spacing().button_padding).max(egui::vec2(side, side));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, visuals.text_color());
    }
    response.on_hover_text(hover)
}

/// The colour being picked in the picker of control `id`.
fn working(ctx: &egui::Context, id: Id) -> Option<Rgba8> {
    ctx.data(|d| d.get_temp::<Rgba8>(id.with("working")))
}

/// The colour picker of the control `response`, opened by a click on it. Returns the colour
/// picked, once, when the picker closes on a colour other than `color`.
pub fn picker(ui: &egui::Ui, response: &Response, id: Id, color: Option<Rgba8>) -> Option<Rgba8> {
    let memory = id.with("working");
    let start = color.unwrap_or([0, 0, 0, 255]);
    let open = Popup::menu(response).id(id.with("picker")).close_behavior(PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let mut c = color32(ui.ctx().data(|d| d.get_temp::<Rgba8>(memory)).unwrap_or(start));
        if color_picker::color_picker_color32(ui, &mut c, Alpha::OnlyBlend) {
            let picked = c.to_srgba_unmultiplied();
            ui.ctx().data_mut(|d| d.insert_temp(memory, picked));
        }
    });
    if open.is_some() {
        return None;
    }
    let picked = ui.ctx().data(|d| d.get_temp::<Rgba8>(memory))?;
    ui.ctx().data_mut(|d| d.remove::<Rgba8>(memory));
    (Some(picked) != color).then_some(picked)
}

// ── What the selected paths have ────────────────────────────────────────────────────────────────

/// `get` of every item when all agree.
fn common<T: PartialEq, I>(items: &[I], get: impl Fn(&I) -> T) -> Option<T> {
    let mut values = items.iter().map(get);
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

/// The attributes of the selected paths, with the palettes their palette colours come from.
struct Selected {
    attrs: Vec<ShapeAttrs>,
    palettes: Palettes,
}

/// Whether `layer`, the edited layer, is a full-colour layer.
fn full_color(app: &FontcraftApp) -> bool {
    let (Ok(font), Some(tab)) = (app.session.font(), app.state.tab()) else { return false };
    view::active_layer(font, tab).is_some_and(|l| matches!(color_layers::kind(l), Some(ColorLayerKind::FullColor)))
}

/// Whether the section shows: the edited layer is a full-colour layer and a path is selected.
pub fn shown(app: &FontcraftApp) -> bool {
    full_color(app) && !stroke::selected_paths(app).is_empty()
}

fn read(app: &FontcraftApp, paths: &[usize]) -> Option<Selected> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    let master = view::active_layer(font, tab)?.master.clone();
    let scene = tools_glue::scene(font, tab)?;
    let attrs = paths
        .iter()
        .filter_map(|&i| match scene.layer.shapes.get(i) {
            Some(Shape::Path(p)) => Some(p.info.attrs.clone()),
            _ => None,
        })
        .collect();
    let palettes = color_palette::read(font, Some(&master)).unwrap_or_default();
    Some(Selected { attrs, palettes })
}

fn stroke_width(a: &ShapeAttrs) -> Option<f64> {
    match a.stroke_width {
        Some(Dimension::Number(n)) => Some(n),
        _ => None,
    }
}

/// The alignment word of a stored stroke position.
fn alignment(a: &ShapeAttrs) -> &'static str {
    let position = match a.stroke_pos {
        Some(Dimension::Number(n)) => n,
        _ => 0.0,
    };
    ALIGNMENTS.iter().min_by(|x, y| (x.3 - position).abs().total_cmp(&(y.3 - position).abs())).map_or("center", |a| a.0)
}

fn gradient_type(g: &Gradient) -> &'static str {
    match g.kind {
        GradientKind::Linear { .. } => "linear",
        GradientKind::Radial { .. } => "radial",
        GradientKind::Conic { .. } => "conic",
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────────────────────────

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let paths = stroke::selected_paths(app);
    let Some(mut targets) = edit_targets(app, "attributes.setFillColor") else { return };
    let Some(selected) = read(app, &paths) else { return };
    if selected.attrs.is_empty() {
        return;
    }
    targets.insert("shapes".into(), json!(paths));
    let tokens = app.tokens.clone();
    let foreground = tokens.text;
    let run = |pending: &mut Pending, id: &'static str, extra: Value| pending.push((id, with(targets.clone(), extra)));
    let attrs = &selected.attrs;
    let colour_of = |c: &Option<Color>| c.as_ref().and_then(|c| resolve(c, &selected.palettes, foreground));

    ui.horizontal(|ui| {
        let plus = menu_button(ui, widgets::id("attributes.add"), "+", "Add a gradient, shadow or pattern image");
        Popup::menu(&plus).id(widgets::id("attributes.addMenu")).show(|ui| {
            for (attribute, label) in ADDABLE {
                if widgets::button(ui, widgets::id(("attributes.add", attribute)), label, &format!("Add {label}"), true, false) {
                    run(pending, "attributes.addAttribute", json!({ "attribute": attribute }));
                    ui.close();
                }
            }
        });
    });

    // Stroke: width, alignment, colour, minus.
    ui.horizontal_wrapped(|ui| {
        ui.label("Stroke");
        match widgets::number(ui, widgets::id("attributes.strokeWidth"), common(attrs, stroke_width).flatten(), tokens.field_width, true) {
            Some(widgets::NumberEdit::Set(v)) => run(pending, "attributes.setStrokeWidth", json!({ "width": v.max(0.0) })),
            Some(widgets::NumberEdit::Clear) => run(pending, "attributes.setStrokeWidth", json!({ "width": null })),
            None => {}
        }
        let current = common(attrs, alignment);
        for (word, label, hover, _) in ALIGNMENTS {
            if widgets::button(ui, widgets::id(("attributes.strokeAlignment", word)), label, hover, true, current == Some(word)) {
                run(pending, "attributes.setStrokeAlignment", json!({ "alignment": word }));
            }
        }
        let colour = common(attrs, |a| colour_of(&a.stroke_color)).flatten();
        if let Some(c) = color_button(ui, widgets::id("attributes.strokeColor"), colour, "Stroke colour", &tokens) {
            run(pending, "attributes.setStrokeColor", json!({ "color": hex(c) }));
        }
        if widgets::button(ui, widgets::id("attributes.removeStroke"), "−", "Remove the stroke", true, false) {
            run(pending, "attributes.removeStroke", json!({}));
        }
    });

    let any_gradient = attrs.iter().any(|a| a.gradient.is_some());
    // Fill: hidden while a gradient replaces it.
    if !any_gradient {
        ui.horizontal(|ui| {
            ui.label("Fill");
            let colour = common(attrs, |a| colour_of(&a.fill_color)).flatten();
            if let Some(c) = color_button(ui, widgets::id("attributes.fillColor"), colour, "Fill colour", &tokens) {
                run(pending, "attributes.setFillColor", json!({ "color": hex(c) }));
            }
            if widgets::button(ui, widgets::id("attributes.removeFill"), "−", "Remove the fill", true, false) {
                run(pending, "attributes.removeFill", json!({}));
            }
        });
    }

    // Gradient: type, minus, the bar of the first path's gradient.
    if let Some(gradient) = attrs.iter().find_map(|a| a.gradient.as_ref()) {
        ui.horizontal(|ui| {
            ui.label("Gradient");
            let current = common(attrs, |a| a.gradient.as_ref().map(gradient_type)).flatten();
            for (word, label) in [("linear", "Linear"), ("radial", "Radial")] {
                if widgets::button(
                    ui,
                    widgets::id(("attributes.gradientType", word)),
                    label,
                    &format!("{label} gradient"),
                    true,
                    current == Some(word),
                ) {
                    run(pending, "attributes.setGradientType", json!({ "type": word }));
                }
            }
            if widgets::button(ui, widgets::id("attributes.removeGradient"), "−", "Remove the gradient", true, false) {
                run(pending, "attributes.removeGradient", json!({}));
            }
        });
        let stops: Vec<(f64, Rgba8)> = gradient
            .stops
            .iter()
            .map(|s| {
                (
                    if s.offset.is_finite() { s.offset.clamp(0.0, 1.0) } else { 0.0 },
                    resolve(&s.color, &selected.palettes, foreground).unwrap_or([0, 0, 0, 0]),
                )
            })
            .collect();
        for (id, extra) in gradient_bar(ui, &stops, &tokens) {
            run(pending, id, extra);
        }
    }

    // Shadows: stored and drawn by image export only; shown so the person knows they are there.
    let first = attrs.first();
    for (label, shadow) in [("Shadow", first.and_then(|a| a.shadow.as_ref())), ("Inner Shadow", first.and_then(|a| a.shadow_in.as_ref()))] {
        if let Some(s) = shadow {
            let [x, y] = s.offset;
            ui.label(
                egui::RichText::new(format!(
                    "{label}: blur {}, offset {}, {}",
                    widgets::format_number(s.blur),
                    widgets::format_number(x),
                    widgets::format_number(y)
                ))
                .color(tokens.text_dim),
            );
        }
    }
}

/// The gradient bar: the colour ramp of `stops` (offset, colour) with a knob per stop below it.
/// Returns the commands (id and the parameters beyond the targets) the person's actions ask for.
fn gradient_bar(ui: &mut egui::Ui, stops: &[(f64, Rgba8)], tokens: &Tokens) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    let height = tokens.list_row_height;
    let knob = height / 2.0;
    let width = ui.available_width().max(height * 4.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height + knob), Sense::hover());
    let bar = Rect::from_min_size(rect.min, egui::vec2(width, height)).shrink2(egui::vec2(knob / 2.0, 0.0));
    let x_of = |offset: f64| bar.left() + bar.width() * offset as f32;
    let offset_of = |x: f32| f64::from(((x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0));

    let bar_response = ui.interact(bar, widgets::id("attributes.gradientBar"), Sense::click()).on_hover_text("Click to add a colour stop");
    if bar_response.clicked()
        && let Some(at) = bar_response.interact_pointer_pos()
    {
        out.push(("attributes.addGradientStop", json!({ "offset": offset_of(at.x) })));
    }
    if ui.is_rect_visible(rect) {
        let mut mesh = egui::Mesh::default();
        let first = stops.first().map_or([0, 0, 0, 0], |s| s.1);
        let last = stops.last().map_or(first, |s| s.1);
        let mut ramp: Vec<(f32, Rgba8)> = vec![(bar.left(), first)];
        ramp.extend(stops.iter().map(|(o, c)| (x_of(*o), *c)));
        ramp.push((bar.right(), last));
        for pair in ramp.windows(2) {
            let (Some((x0, c0)), Some((x1, c1))) = (pair.first(), pair.get(1)) else { continue };
            let base = u32::try_from(mesh.vertices.len()).unwrap_or(u32::MAX);
            mesh.colored_vertex(egui::pos2(*x0, bar.top()), color32(*c0));
            mesh.colored_vertex(egui::pos2(*x1, bar.top()), color32(*c1));
            mesh.colored_vertex(egui::pos2(*x1, bar.bottom()), color32(*c1));
            mesh.colored_vertex(egui::pos2(*x0, bar.bottom()), color32(*c0));
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        ui.painter().rect_stroke(bar, 0.0, Stroke::new(tokens.outline_width, tokens.border), StrokeKind::Inside);
    }
    for (i, (offset, colour)) in stops.iter().enumerate() {
        let id = widgets::id(("attributes.gradientStop", i));
        let centre = egui::pos2(x_of(*offset), bar.bottom() + knob / 2.0);
        let knob_rect = Rect::from_center_size(centre, egui::vec2(knob, knob));
        let response =
            ui.interact(knob_rect, id, Sense::click_and_drag()).on_hover_text("Click to change the colour; drag to move, drag away to remove");
        let drawn_at = match (response.dragged(), response.interact_pointer_pos()) {
            (true, Some(p)) => Rect::from_center_size(egui::pos2(p.x, p.y.max(centre.y)), knob_rect.size()),
            _ => knob_rect,
        };
        if ui.is_rect_visible(drawn_at) {
            paint_swatch(ui.painter(), drawn_at, Some(*colour), tokens);
        }
        if response.drag_stopped()
            && let Some(p) = response.interact_pointer_pos()
        {
            if (p.y - centre.y).abs() > height * REMOVE_DISTANCE {
                if stops.len() > 2 {
                    out.push(("attributes.removeGradientStop", json!({ "stop": i })));
                }
            } else {
                out.push(("attributes.setGradientStopColor", json!({ "stop": i, "offset": offset_of(p.x) })));
            }
        }
        if let Some(c) = picker(ui, &response, id, Some(*colour)) {
            out.push(("attributes.setGradientStopColor", json!({ "stop": i, "color": hex(c) })));
        }
    }
    out
}

/// The targets of the attributes commands for the selected paths, as the section runs them: for
/// tests and agents that want the parameters a control would send.
pub fn targets(app: &FontcraftApp) -> Option<Map<String, Value>> {
    let mut t = edit_targets(app, "attributes.setFillColor")?;
    t.insert("shapes".into(), json!(stroke::selected_paths(app)));
    Some(t)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use egui::{Event, Modifiers, PointerButton, Pos2};
    use serde_json::{Value, json};

    use super::*;

    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
            let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
            output.textures_delta.clear();
        }

        fn settle(&self, app: &mut FontcraftApp) {
            for _ in 0..3 {
                self.frame(app, Vec::new());
            }
        }

        fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
            let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(app, vec![Event::PointerMoved(at)]);
            self.frame(app, vec![button(true)]);
            self.frame(app, vec![button(false)]);
        }
    }

    fn last(app: &FontcraftApp) -> (String, Value) {
        app.session.journal().last().cloned().expect("a command ran")
    }

    /// Glyph A with a full-colour layer holding a square (shape 0), edited in that layer with
    /// the square selected and Show Info on.
    fn full_color_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        let master = app.session.font().unwrap().masters[0].id.clone();
        app.run("layers.addBackupLayer", &json!({"glyphs": ["A"], "master": master})).unwrap();
        let layer = app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone();
        app.run("layer.convertToColor", &json!({"glyph": "A", "layer": layer})).unwrap();
        app.run("path.addRectangle", &json!({"glyph": "A", "layer": layer, "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("layers.select", &json!({"layer": layer})).unwrap();
        app.run("selection.set", &json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]})).unwrap();
        app.run("view.showInfo", &json!({"on": true})).unwrap();
        // Collapsed sections keep the contextual area on screen.
        for (section, _) in crate::panels::SECTIONS {
            app.run("palette.toggleSection", &json!({"section": section, "expanded": false})).unwrap();
        }
        app
    }

    fn square_attrs(app: &FontcraftApp) -> ShapeAttrs {
        let font = app.session.font().unwrap();
        let layer = font.glyph("A").unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap();
        let Some(Shape::Path(p)) = layer.shapes.first() else { panic!("shape 0 is not a path") };
        p.info.attrs.clone()
    }

    #[test]
    fn inspector_attributes_controls_run_the_attributes_commands() {
        let mut app = full_color_app();
        assert!(shown(&app), "a selected path of a full-colour layer shows the section");
        let w = Window { ctx: egui::Context::default() };
        w.settle(&mut app);

        w.click(&mut app, widgets::id(("attributes.strokeAlignment", "outside")));
        let (id, params) = last(&app);
        assert_eq!((id.as_str(), params["alignment"].as_str()), ("attributes.setStrokeAlignment", Some("outside")));
        assert_eq!(params["shapes"], json!([0]));

        w.click(&mut app, widgets::id("attributes.removeFill"));
        assert_eq!(last(&app).0, "attributes.removeFill");
        assert!(square_attrs(&app).fill_color.is_some_and(|c| matches!(c, Color::Rgba([_, _, _, a]) if a == 0.0)), "a removed fill is transparent");

        // The plus menu adds a gradient, which hides the Fill row and shows the bar.
        w.click(&mut app, widgets::id("attributes.add"));
        w.click(&mut app, widgets::id(("attributes.add", "gradient")));
        assert_eq!(last(&app).0, "attributes.addAttribute");
        assert!(square_attrs(&app).gradient.is_some());
        w.settle(&mut app);
        assert!(w.ctx.read_response(widgets::id("attributes.fillColor")).is_none(), "the gradient replaces the fill row");

        w.click(&mut app, widgets::id(("attributes.gradientType", "radial")));
        assert!(matches!(square_attrs(&app).gradient.unwrap().kind, GradientKind::Radial { .. }));

        w.click(&mut app, widgets::id("attributes.removeGradient"));
        assert_eq!(last(&app).0, "attributes.removeGradient");
        assert!(square_attrs(&app).gradient.is_none());
    }

    #[test]
    fn inspector_attributes_gradient_bar_adds_a_stop_where_it_is_clicked() {
        let mut app = full_color_app();
        app.run("attributes.addGradient", &Value::Object(targets(&app).unwrap())).unwrap();
        let w = Window { ctx: egui::Context::default() };
        w.settle(&mut app);
        let bar = w.ctx.read_response(widgets::id("attributes.gradientBar")).expect("the bar is drawn").rect;
        let at = egui::pos2(bar.left() + bar.width() * 0.25, bar.center().y);
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        w.frame(&mut app, vec![Event::PointerMoved(at)]);
        w.frame(&mut app, vec![button(true)]);
        w.frame(&mut app, vec![button(false)]);
        let (id, params) = last(&app);
        assert_eq!(id, "attributes.addGradientStop");
        assert!((params["offset"].as_f64().unwrap() - 0.25).abs() < 0.02, "{params}");
        assert_eq!(square_attrs(&app).gradient.unwrap().stops.len(), 3);
    }

    #[test]
    fn the_section_hides_on_a_master_layer() {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("selection.set", &json!({"points": [[0, 0]]})).unwrap();
        assert!(!shown(&app));
    }

    #[test]
    fn palette_colours_resolve_through_palette_zero() {
        let palettes = Palettes::new(vec![vec![color_palette::Rgba::new(255, 0, 0, 255)]]).unwrap();
        let red = resolve(&Color::Palette { index: 0, alpha: 0.5 }, &palettes, Color32::WHITE);
        assert_eq!(red, Some([255, 0, 0, 128]));
        assert_eq!(resolve(&Color::Palette { index: 3, alpha: 1.0 }, &palettes, Color32::WHITE), None);
        assert_eq!(resolve(&Color::Palette { index: FOREGROUND, alpha: 1.0 }, &palettes, Color32::WHITE), Some([255, 255, 255, 255]));
        assert_eq!(parse_hex(&hex([1, 2, 3, 4])), Some([1, 2, 3, 4]));
    }
}
