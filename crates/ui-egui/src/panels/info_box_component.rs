//! The component Info box: while Show Info is on and a component is selected in Edit View, a
//! second box beside the selection box describes the glyph the component points to.
//!
//! | Control | Runs |
//! |---|---|
//! | base glyph name (a button) | opens the glyph picker for `component.setBaseGlyph` (`infoBox.componentGlyphPicker`) |
//! | anchor menu, only when the component can attach to more than one anchor | `component.setAnchor` (`components.anchorMenuOnlyWhenChoiceExists`) |
//! | Smart badge, only on a smart component | `smartComponent.showSettings` (`smart.badgeInInfoBox`) |
//! | arrow button, top right | `component.editBaseGlyph`: the base glyph joins the text left of the composite |
//! | X, Y | `component.setTransform` `x`, `y` |
//! | horizontal and vertical scale, in percent | `component.setTransform` `scaleX`, `scaleY` |
//! | slant to the right, rotation counterclockwise, in degrees | `component.setTransform` `slant`, `rotation` |
//!
//! With several components selected the box describes the first in shape order. X and Y are
//! greyed out while the component is aligned automatically (only X when it is aligned
//! horizontally), and every field while it is locked: the command would refuse the edit.

use fontcraft_engine::components::align::{AlignMode, component_alignment};
use fontcraft_engine::components::propagate::{base_layer, effective_anchors_with};
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::{Font, Layer, Shape, Transform};
use serde_json::{Map, Value, json};

use super::{Pending, widgets, with};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::dialogs::glyph_picker_component as picker;
use crate::theme::Tokens;

/// Most components before the selected one whose anchors the anchor menu reads.
const MAX_ANCHOR_SOURCES: usize = 64;
/// Below this a matrix entry counts as zero.
const TINY: f64 = 1e-12;

/// The values the box shows: position, scale in percent, slant and rotation in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Values {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub slant: f64,
    pub rotation: f64,
}

/// What the box needs, read before drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentBox {
    pub glyph: String,
    pub layer: String,
    /// The component's shape index on the layer.
    pub component: usize,
    pub base: String,
    /// `None` for a matrix the box cannot split into its fields; X and Y then still show.
    pub values: Option<Values>,
    pub position: (f64, f64),
    /// Whether X and Y can be edited.
    pub free: (bool, bool),
    pub locked: bool,
    pub smart: bool,
    /// The anchors the component can attach to; the menu shows when there are two or more.
    pub anchors: Vec<String>,
    /// The anchor the component attaches to as the menu shows it: the stored choice, else the
    /// first plain name offered.
    pub anchor: Option<String>,
    /// How many components are selected.
    pub selected: usize,
}

/// The Info box fields of a transform; `None` for a matrix that is not a rotation and scale
/// (a slanted matrix), or that is singular.
pub fn values(t: &Transform) -> Option<Values> {
    match *t {
        Transform::Decomposed { pos, scale, angle, slant } => {
            Some(Values { x: pos.0, y: pos.1, scale_x: scale.0 * 100.0, scale_y: scale.1 * 100.0, slant: slant.0, rotation: angle })
        }
        Transform::Matrix([a, b, c, d, dx, dy]) => {
            if ![a, b, c, d, dx, dy].iter().all(|v| v.is_finite()) {
                return None;
            }
            // Orthogonal columns: a rotation times a scale, with no slant.
            if (a * c + b * d).abs() > TINY * (1.0 + a.abs() + b.abs() + c.abs() + d.abs()) {
                return None;
            }
            let sx = a.hypot(b);
            if sx < TINY {
                return None;
            }
            let mut angle = b.atan2(a).to_degrees();
            let mut sy = (a * d - b * c) / sx;
            let mut sx = sx;
            // A mirror reads as a negative horizontal scale rather than a turn by 180°.
            if (angle.abs() - 180.0).abs() < 1e-9 && sy < 0.0 {
                (angle, sx, sy) = (0.0, -sx, -sy);
            }
            if sy.abs() < TINY {
                return None;
            }
            Some(Values { x: dx, y: dy, scale_x: sx * 100.0, scale_y: sy * 100.0, slant: 0.0, rotation: angle })
        }
    }
}

/// The anchors the component at `index` of `layer` can attach to: the anchors of the components
/// before it named `X` or `X_suffix`, where `_X` is one of its own attaching anchors, in the
/// order the components offer them.
pub fn anchor_choices(font: &Font, data: Option<&GlyphData>, layer: &Layer, index: usize) -> Vec<String> {
    let Some(Shape::Component(c)) = layer.shapes.get(index) else { return Vec::new() };
    let Some(base) = font.glyph(&c.glyph) else { return Vec::new() };
    let Some(from) = base_layer(base, layer, c) else { return Vec::new() };
    let Ok(own) = effective_anchors_with(font, data, &c.glyph, &from.id) else { return Vec::new() };
    let plain: Vec<&str> = own.iter().filter_map(|a| a.name.strip_prefix('_')).filter(|n| !n.is_empty()).collect();
    if plain.is_empty() {
        return Vec::new();
    }
    let fits = |name: &str| plain.iter().any(|p| name == *p || name.strip_prefix(p).is_some_and(|rest| rest.len() > 1 && rest.starts_with('_')));
    let mut offered: Vec<String> = Vec::new();
    let before = layer.shapes.iter().take(index).filter_map(|s| if let Shape::Component(p) = s { Some(p) } else { None });
    for prev in before.take(MAX_ANCHOR_SOURCES) {
        let Some(g) = font.glyph(&prev.glyph) else { continue };
        let Some(l) = base_layer(g, layer, prev) else { continue };
        let Ok(anchors) = effective_anchors_with(font, data, &prev.glyph, &l.id) else { continue };
        for a in anchors {
            if !a.name.starts_with('_') && fits(&a.name) && !offered.contains(&a.name) {
                offered.push(a.name);
            }
        }
    }
    offered
}

/// The box for the first selected component of the showing tab's active layer; `None` when no
/// component is selected.
pub fn read(app: &FontcraftApp) -> Option<ComponentBox> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    let glyph = tab.edited_glyph()?.to_string();
    let layer = view::active_layer(font, tab)?;
    let components: Vec<usize> = tab.selection.shapes.iter().copied().filter(|&i| matches!(layer.shapes.get(i), Some(Shape::Component(_)))).collect();
    let index = *components.first()?;
    let Some(Shape::Component(c)) = layer.shapes.get(index) else { return None };
    let locked = c.info.locked;
    let free = match (locked, component_alignment(font, layer, index)) {
        (true, _) | (false, Some(AlignMode::Full)) => (false, false),
        (false, Some(AlignMode::Horizontal)) => (false, true),
        (false, None) => (true, true),
    };
    let data = app.session.glyph_data().ok();
    let anchors = anchor_choices(font, data.as_deref(), layer, index);
    let anchor = c.info.anchor.clone().or_else(|| {
        let plain: Vec<String> = font
            .glyph(&c.glyph)
            .and_then(|g| base_layer(g, layer, c))
            .map(|l| l.anchors.iter().filter_map(|a| a.name.strip_prefix('_').map(str::to_string)).collect())
            .unwrap_or_default();
        anchors.iter().find(|a| plain.contains(a)).cloned()
    });
    let [_, _, _, _, dx, dy] = c.transform.affine().as_coeffs();
    Some(ComponentBox {
        glyph,
        layer: layer.id.clone(),
        component: index,
        base: c.glyph.clone(),
        values: values(&c.transform),
        position: (dx, dy),
        free,
        locked,
        smart: font.glyph(&c.glyph).is_some_and(|g| !g.info.parts.is_empty()),
        anchors,
        anchor,
        selected: components.len(),
    })
}

/// Draws the box beside the selection box while a component is selected.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(b) = read(app) else { return };
    let tokens = app.tokens.clone();
    let canvas = ui.available_rect_before_wrap();
    let pos = beside_selection_box(ui, &tokens, canvas);
    egui::Area::new(widgets::id("infoBox.component"))
        .order(egui::Order::Middle)
        .fixed_pos(pos)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .constrain_to(canvas)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| contents(ui, &tokens, &b, pending));
        });
}

/// Where a box beside the selection box hangs from: the lower right corner of that box as it
/// was last drawn, else the middle of the canvas's foot.
pub fn beside_selection_box(ui: &egui::Ui, tokens: &Tokens, canvas: egui::Rect) -> egui::Pos2 {
    let main = ui.ctx().memory(|m| m.area_rect(widgets::id("infoBox")));
    match main {
        Some(r) => egui::pos2(r.right() + tokens.info_box_margin, r.bottom()),
        None => egui::pos2(canvas.center().x, canvas.bottom() - tokens.info_box_margin),
    }
}

fn contents(ui: &mut egui::Ui, tokens: &Tokens, b: &ComponentBox, pending: &mut Pending) {
    let mut target = Map::new();
    target.insert("glyph".into(), json!(b.glyph));
    target.insert("layer".into(), json!(b.layer));
    target.insert("component".into(), json!(b.component));
    let set = |pending: &mut Pending, key: &str, v: f64| {
        let mut extra = Map::new();
        extra.insert(key.into(), json!(v));
        pending.push(("component.setTransform", with(target.clone(), Value::Object(extra))));
    };
    ui.horizontal(|ui| {
        let name = if b.selected > 1 { format!("{} (+{})", b.base, b.selected - 1) } else { b.base.clone() };
        if widgets::button(ui, widgets::id("infoBox.component.base"), &name, "Choose another glyph for this component", true, false) {
            pending.push((picker::COMMAND, with(target.clone(), json!({ "purpose": "baseGlyph" }))));
        }
        if b.anchors.len() > 1 {
            let shown = b.anchor.clone().unwrap_or_default();
            egui::ComboBox::from_id_salt(widgets::id("infoBox.component.anchor")).selected_text(shown.as_str()).show_ui(ui, |ui| {
                for a in &b.anchors {
                    if ui.selectable_label(Some(a) == b.anchor.as_ref(), a.as_str()).clicked() && Some(a) != b.anchor.as_ref() {
                        pending.push(("component.setAnchor", with(target.clone(), json!({ "anchor": a }))));
                    }
                }
            });
        }
        if b.smart && widgets::button(ui, widgets::id("infoBox.component.smart"), "Smart", "Smart Component Settings", true, true) {
            let mut t = target.clone();
            t.remove("component");
            pending.push(("smartComponent.showSettings", with(t, json!({ "components": [b.component] }))));
        }
        if widgets::button(ui, widgets::id("infoBox.component.edit"), "↗", "Edit the base glyph", true, false) {
            pending.push(("component.editBaseGlyph", json!({ "component": b.component })));
        }
    });
    let width = tokens.field_width;
    let field = |ui: &mut egui::Ui, label: &str, hover: &str, salt: &str, value: Option<f64>, enabled: bool| -> Option<f64> {
        ui.label(egui::RichText::new(label).color(tokens.text_dim)).on_hover_text(hover);
        let edit = ui.add_enabled_ui(enabled, |ui| widgets::number(ui, widgets::id(("infoBox.component", salt)), value, width, false)).inner;
        match edit {
            Some(widgets::NumberEdit::Set(v)) if enabled => Some(v),
            _ => None,
        }
    };
    let v = b.values;
    let editable = !b.locked;
    ui.horizontal(|ui| {
        if let Some(x) = field(ui, "X", "Horizontal offset", "x", Some(b.position.0), b.free.0) {
            set(pending, "x", x);
        }
        if let Some(y) = field(ui, "Y", "Vertical offset", "y", Some(b.position.1), b.free.1) {
            set(pending, "y", y);
        }
        if let Some(s) = field(ui, "↔%", "Horizontal scale in percent", "scaleX", v.map(|v| v.scale_x), editable && v.is_some()) {
            set(pending, "scaleX", s);
        }
        if let Some(s) = field(ui, "↕%", "Vertical scale in percent", "scaleY", v.map(|v| v.scale_y), editable && v.is_some()) {
            set(pending, "scaleY", s);
        }
        if let Some(s) = field(ui, "⧄", "Slant to the right, in degrees", "slant", v.map(|v| v.slant), editable && v.is_some()) {
            set(pending, "slant", s);
        }
        if let Some(r) = field(ui, "⟲", "Rotation counterclockwise, in degrees", "rotation", v.map(|v| v.rotation), editable && v.is_some()) {
            set(pending, "rotation", r);
        }
    });
}
