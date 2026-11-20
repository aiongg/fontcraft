//! The Preview Area under the edit view and the Preview Panel: a strip that draws the edit view's text filled on one
//! line, over a row of controls (the instance pop-up, Invert, Flip, Always Center Active Glyph and the blur slider).
//!
//! The strip is a canvas whose view the line's placement sets (`app::models::preview::place`): dragging it shifts the
//! line (`preview.setOffset`), a double-click on a glyph makes it the edited glyph (`editView.setCursor`). The line is
//! rasterised as `preview.render` draws it, so Invert, Flip and Blur act on pixels; the picture is kept as a texture
//! until what it shows changes. Both previews post the same commands with their own `target`.

pub mod tokens;

use craft::behavior::CanvasView;
use craft::behavior::PressPhase;
use craft::behavior::kurbo::Point;
use craft::ui::buttons::Button;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::canvas::{Canvas, canvas_space};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked, Trigger};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::TextureCache;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::value_drag::Slider;
use egui::{Color32, ColorImage, Pos2, Rect, TextureOptions, Ui};
use fontcraft_engine::cmd::preview::{MAX_BLUR, blur_pixels, blur_radius, turn_pixels};
use fontcraft_geom::Affine;
use fontcraft_render::{Style, render_path};
use serde_json::json;

pub use self::tokens::PreviewTokens;
use crate::app::Msg;
use crate::app::models::preview::{Placement, PreviewModel, hit, place};
use crate::shell::preview::Target;

craft::ui::words! {
    /// The words of the previews.
    pub struct PreviewWords {
        /// The Preview Area's strip.
        area: "fontcraftPreview.area" = "Preview Area";
        /// The Preview Panel's strip.
        panel: "fontcraftPreview.panel" = "Preview Panel";
        /// The pop-up entry that follows the current master.
        dash: "fontcraftPreview.dash" = "-";
        /// The pop-up's tooltip.
        instance_tip: "fontcraftPreview.instanceTip" = "The instance the preview shows";
        /// The pop-up entry that shows every visible instance.
        show_all: "fontcraftPreview.showAll" = "Show All Instances";
        /// The heading over the instances' eyes in the pop-up.
        eyes: "fontcraftPreview.eyes" = "In Show All Instances";
        /// An instance's eye row.
        eye: template "fontcraftPreview.eye" = "Include {name}";
        invert: "fontcraftPreview.invert" = "Invert";
        invert_tip: "fontcraftPreview.invertTip" = "Light text on a dark ground";
        flip: "fontcraftPreview.flip" = "Flip";
        flip_tip: "fontcraftPreview.flipTip" = "Turn the preview upside down";
        center: "fontcraftPreview.center" = "Always Center Active Glyph";
        center_tip: "fontcraftPreview.centerTip" = "Keep the edited glyph in the middle of the preview";
        blur: "fontcraftPreview.blur" = "Blur";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<PreviewModel>,
    pub target: Target,
    pub sent: &'a mut Vec<Msg>,
}

/// The Preview Panel: the strip fills the panel above the controls.
pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, None, ui);
}

/// The Preview Area: the strip and the controls, `height` points tall in all.
pub fn show_area(ui: &mut Ui, mut props: Props<'_>, height: f32) {
    view(&mut props, Some(height), ui);
}

fn view(props: &mut Props<'_>, height: Option<f32>, ui: &mut Ui) {
    let (model, target) = (props.model, props.target);
    let mut controls_sent = Vec::new();
    let mut strip_sent = Vec::new();
    let mut stack = match height {
        Some(h) => Stack::new().fill_width().room_height(h),
        None => Stack::new().fill(),
    };
    stack = stack.footer(|ui, _| controls(ui, model, target, &mut controls_sent));
    stack.show(ui, |ui, _| strip(ui, model, target, height, &mut strip_sent));
    props.sent.extend(strip_sent);
    props.sent.extend(controls_sent);
}

/// The strip: the line filled on the ground, dragged to scroll, double-clicked to edit a glyph. `height`: the room of
/// the Preview Area, whose stack sizes from its content.
fn strip(ui: &mut Ui, model: &PreviewModel, target: Target, height: Option<f32>, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<PreviewWords>();
    let tokens = PreviewTokens::standard();
    let label = match target {
        Target::Area => words.area.clone(),
        Target::Panel => words.panel.clone(),
    };
    let (ink, paper) = if model.options.invert { (tokens.paper, tokens.ink) } else { (tokens.ink, tokens.paper) };
    let mut placed = None;
    let canvas = Canvas::new(label).id_salt(("fontcraftPreview", target.name())).view(Bind::given(CanvasView::new(Rect::NOTHING, Point::ZERO, 1.0)));
    let out = canvas.show(ui, |ui, _| {
        // The canvas takes its content height, held to the room the controls leave.
        if let Some(h) = height.filter(|h| h.is_finite() && *h > 0.0) {
            ui.allocate_space(egui::vec2(ui.available_width().max(0.0), h));
        }
        let Some(space) = canvas_space(&Env::of(ui).view()) else { return };
        let rect = space.view.rect;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, paper);
        if !model.shown {
            return;
        }
        let bounds = [f64::from(rect.left()), f64::from(rect.top()), f64::from(rect.right()), f64::from(rect.bottom())];
        let Some(placement) = place(model, bounds, f64::from(tokens.margin)) else { return };
        let ppp = space.pixels_per_point;
        let key = (
            "fontcraftPreview",
            target.name(),
            model.key,
            [placement.scale, placement.x0, placement.baseline].map(f64::to_bits),
            bounds.map(f64::to_bits),
            ppp.to_bits(),
            model.options.blur().to_bits(),
            (model.options.invert, model.options.flip),
            (ink.to_array(), paper.to_array()),
        );
        let image = || picture(model, &placement, ppp, ink, paper).unwrap_or_else(|| ColorImage::filled([1, 1], paper));
        let texture = TextureCache::texture_with(ui.ctx(), key, TextureOptions::LINEAR, image);
        // White is the identity tint: the texture's own colours show.
        painter.image(texture, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        placed = Some(placement);
    });
    let Some(placement) = placed else { return };
    let dx = match out.press.phase {
        PressPhase::DragStart { start, pos, .. } => pos.x - start.origin.x,
        PressPhase::Drag { delta, .. } => delta.x,
        _ => 0.0,
    };
    if dx != 0.0 && dx.is_finite() {
        let dx = f64::from(if placement.flip { -dx } else { dx });
        sent.push(Msg::Shell(CommandRef::with_params("preview.setOffset", json!({"target": target.name(), "dx": dx}))));
    }
    if out.press.double_pressed()
        && let Some(start) = out.press.phase.start()
        && let Some(index) = hit(model, &placement, [f64::from(start.origin.x), f64::from(start.origin.y)])
    {
        sent.push(Msg::Shell(CommandRef::with_params("editView.setCursor", json!({ "index": index }))));
    }
}

/// The line as pixels the size of the strip: filled, inverted, turned and blurred as the options say. `None` for an
/// empty or oversized strip.
fn picture(model: &PreviewModel, placement: &Placement, ppp: f32, ink: Color32, paper: Color32) -> Option<ColorImage> {
    let [left, top, right, bottom] = placement.rect;
    let ppp = if ppp.is_finite() && ppp > 0.0 { f64::from(ppp) } else { 1.0 };
    let (w, h) = (((right - left) * ppp).round(), ((bottom - top) * ppp).round());
    if !(w >= 1.0 && h >= 1.0 && w <= f64::from(u32::MAX) && h <= f64::from(u32::MAX)) {
        return None;
    }
    let (width, height) = (w as u32, h as u32);
    let s = placement.scale;
    let to_pixels = Affine::scale(ppp) * Affine::translate((placement.x0 - left, placement.baseline - top)) * Affine::scale_non_uniform(s, -s);
    let style = Style { ink: ink.to_srgba_unmultiplied(), background: paper.to_srgba_unmultiplied(), anti_alias: true };
    let image = render_path(&model.outline, to_pixels, width, height, &style).ok()?;
    let mut pixels = image.pixels;
    if placement.flip {
        pixels = turn_pixels(&pixels);
    }
    blur_pixels(&mut pixels, width as usize, height as usize, blur_radius(model.options.blur() * ppp));
    Some(ColorImage::from_rgba_premultiplied([width as usize, height as usize], &pixels))
}

/// The controls row: the instance pop-up, Invert, Flip, Always Center Active Glyph and the blur slider.
fn controls(ui: &mut Ui, model: &PreviewModel, target: Target, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<PreviewWords>();
    let t = target.name();
    let post = |sent: &mut Vec<Msg>, id: &str, params: serde_json::Value| sent.push(Msg::Shell(CommandRef::with_params(id, params)));
    Row::new().show(ui, |ui, _| {
        instance_menu(ui, model, target, &words, sent);
        let options = &model.options;
        let toggles = [
            ("preview.invert", &words.invert, &words.invert_tip, options.invert),
            ("preview.flip", &words.flip, &words.flip_tip, options.flip),
            ("preview.alwaysCenterActiveGlyph", &words.center, &words.center_tip, options.always_center),
        ];
        for (id, label, tip, on) in toggles {
            let out = ToggleButton::new(label.clone(), Bind::given(on)).tooltip(tip.clone()).id_salt(("fontcraftPreview", t, id)).show(ui);
            if let Some(now) = out.events.committed() {
                post(sent, id, json!({"target": t, "on": *now}));
            }
        }
        let blur = Slider::new(Bind::given(options.blur()), 0.0..=MAX_BLUR)
            .label(words.blur.clone())
            .tooltip(words.blur.clone())
            .id_salt(("fontcraftPreview", t, "blur"))
            .show(ui);
        if let Some(amount) = blur.events.committed().filter(|a| a.is_finite()) {
            post(sent, "preview.setBlur", json!({"target": t, "amount": *amount}));
        }
    });
}

/// The instance pop-up: the dash, each static instance, Show All Instances, and the instances' eyes.
fn instance_menu(ui: &mut Ui, model: &PreviewModel, target: Target, words: &PreviewWords, sent: &mut Vec<Msg>) {
    let t = target.name();
    let shown = match (&model.instance_label, model.all_chosen) {
        (Some(name), _) => name.clone().into(),
        (None, true) => words.show_all.clone(),
        (None, false) => words.dash.clone(),
    };
    let opener = Button::new(shown).tooltip(words.instance_tip.clone()).id_salt(("fontcraftPreview", t, "instance")).show(ui);
    let set = |sent: &mut Vec<Msg>, instance: serde_json::Value| {
        sent.push(Msg::Shell(CommandRef::with_params("preview.setInstance", json!({"target": t, "instance": instance}))));
    };
    Menu::new(words.instance_tip.clone()).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
        if MenuItem::new(words.dash.clone()).radio_mark(model.master_chosen).show(ui).ran {
            set(sent, json!("master"));
        }
        for entry in &model.instances {
            if MenuItem::new(entry.name.clone()).radio_mark(entry.chosen).show(ui).ran {
                set(sent, json!(entry.index));
            }
        }
        if MenuItem::new(words.show_all.clone()).radio_mark(model.all_chosen).show(ui).ran {
            sent.push(Msg::Shell(CommandRef::with_params("preview.showAllInstances", json!({ "target": t }))));
        }
        if !model.instances.is_empty() {
            MenuItem::heading(words.eyes.clone()).show(ui);
        }
        for entry in &model.instances {
            let label = words.eye.fill(&[("name", entry.name.as_str())]);
            if MenuItem::new(label).check(entry.visible).keep_open(true).show(ui).ran {
                sent.push(Msg::run("preview.toggleInstanceVisibility", json!({ "instance": entry.index })));
            }
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
