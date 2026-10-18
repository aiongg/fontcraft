//! Colour layers in the Layers palette: indented and bold under their master, with the palette
//! swatch or the iColor size; the palette colour chooser; double-clicking an iColor entry edits
//! its size. Package M11.15 (`docs/plans/M11.md` § 4); rows `palette.layers.colorLayerEntry`,
//! `palette.layers.paletteColorChooser`.
//!
//! The Layers section draws a colour layer's row as any other non-master row (indented, eye,
//! name) and hands the part after the name to [`ColorEntries::show`]:
//!
//! | Kind | After the name |
//! |---|---|
//! | Color Palette | the label in bold (`Color 2`, `Color *`) and a swatch of its colour; a click on the swatch opens the chooser, whose entries run `layer.setColorPaletteIndex` |
//! | full colour | the label `Color` in bold and a colour-spectrum disc |
//! | iColor | the label (`iColor 64`) in bold; a double-click on it edits the size, which Return commits through `layer.setIColorSize` |
//! | svg | the label `svg` in bold |
//!
//! The chooser lists the colours of palette 0 of the palettes that apply to the layer's master
//! (its own `Color Palettes`, else the font's), then the asterisk entry for the text colour of
//! the font's user. The kinds themselves are chosen in the row's context menu
//! (`layer.convertToColorPalette`, `layer.convertToIColor`, `layer.convertToSvg`,
//! `layer.convertToColor`).

use std::collections::HashMap;

use egui::{Id, Popup, Sense, Stroke};
use fontcraft_engine::color::layers::{self as color_layers, ColorLayerKind};
use fontcraft_engine::color::palette as color_palette;
use fontcraft_model::PaletteEntry;
use serde_json::json;

use super::attributes::{self, Rgba8};
use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::theme::Tokens;

/// The iColor sizes the size field takes, in pixels per em.
const SIZES: std::ops::RangeInclusive<u16> = 1..=4096;
/// Hue segments of the full-colour disc.
const DISC_SEGMENTS: usize = 12;
/// Most palette colours the chooser lists; the rest are counted.
const MAX_CHOOSER_ROWS: usize = 256;

/// One colour layer as its row shows it.
struct Entry {
    kind: ColorLayerKind,
    label: String,
    /// Palette 0 of the palettes that apply to the layer's master.
    palette: Vec<Rgba8>,
}

/// The colour layers of one glyph, by layer id.
pub struct ColorEntries {
    glyph: String,
    entries: HashMap<String, Entry>,
}

/// The id of a palette layer's swatch.
pub fn swatch_id(layer: &str) -> Id {
    widgets::id(("layers.color.swatch", layer))
}

/// The id of a chooser entry: a palette row, or `None` for the text colour.
pub fn chooser_id(layer: &str, index: Option<usize>) -> Id {
    widgets::id(("layers.color.choose", layer, index))
}

/// The id of a colour layer's bold label.
pub fn label_id(layer: &str) -> Id {
    widgets::id(("layers.color.label", layer))
}

/// The id of an iColor layer's size field.
pub fn size_field(layer: &str) -> Id {
    widgets::id(("layers.color.size", layer))
}

/// The id under which the iColor layer whose size is being edited is remembered.
fn size_editing() -> Id {
    widgets::id("layers.color.sizeEditing")
}

impl ColorEntries {
    /// The colour layers of `glyph` in the active document.
    pub fn read(app: &FontcraftApp, glyph: &str) -> Self {
        let mut entries = HashMap::new();
        let Ok(font) = app.session.font() else { return Self { glyph: glyph.to_string(), entries } };
        let Some(g) = font.glyph(glyph) else { return Self { glyph: glyph.to_string(), entries } };
        let mut palettes: HashMap<&str, Vec<Rgba8>> = HashMap::new();
        for layer in &g.layers {
            let Some(kind) = color_layers::kind(layer) else { continue };
            let palette = palettes
                .entry(layer.master.as_str())
                .or_insert_with(|| {
                    color_palette::read(font, Some(layer.master.as_str()))
                        .ok()
                        .and_then(|p| p.palette(0).map(|row| row.iter().map(|c| [c.r, c.g, c.b, c.a]).collect()))
                        .unwrap_or_default()
                })
                .clone();
            entries.insert(layer.id.clone(), Entry { label: color_layers::label(&kind), kind, palette });
        }
        Self { glyph: glyph.to_string(), entries }
    }

    /// Draws what follows the name in the row of `layer`. `false` for a layer that is not a
    /// colour layer, whose row shows its kind as before.
    pub fn show(&self, ui: &mut egui::Ui, tokens: &Tokens, layer: &str, selected: bool, pending: &mut Pending) -> bool {
        let Some(entry) = self.entries.get(layer) else { return false };
        let ctx = ui.ctx().clone();
        let colour = if selected { tokens.selected_text } else { tokens.text_dim };
        if let ColorLayerKind::IColor(size) = entry.kind
            && ctx.data(|d| d.get_temp::<String>(size_editing())).as_deref() == Some(layer)
        {
            let field = size_field(layer);
            if let Some(text) = widgets::text(ui, field, &size.to_string(), tokens.field_width)
                && let Ok(n) = text.trim().parse::<u16>()
                && SIZES.contains(&n)
                && n != size
            {
                pending.push(("layer.setIColorSize", json!({"glyph": self.glyph, "layer": layer, "size": n})));
            }
            if !ctx.memory(|m| m.has_focus(field)) {
                ctx.data_mut(|d| d.remove::<String>(size_editing()));
            }
            return true;
        }
        let shown = ui.label(egui::RichText::new(&entry.label).strong().color(colour));
        let label = ui.interact(shown.rect, label_id(layer), Sense::click());
        match entry.kind {
            ColorLayerKind::Palette(current) => self.swatch(ui, tokens, layer, entry, current, pending),
            ColorLayerKind::FullColor => disc(ui, tokens),
            ColorLayerKind::IColor(_) => {
                if label.on_hover_text("Double-click to change the size in pixels per em").double_clicked() {
                    ctx.data_mut(|d| d.insert_temp(size_editing(), layer.to_string()));
                    ctx.memory_mut(|m| m.request_focus(size_field(layer)));
                }
            }
            ColorLayerKind::Svg => {}
        }
        true
    }

    /// The swatch of a palette layer and its chooser.
    fn swatch(&self, ui: &mut egui::Ui, tokens: &Tokens, layer: &str, entry: &Entry, current: PaletteEntry, pending: &mut Pending) {
        let side = ui.spacing().interact_size.y;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
        let response = ui.interact(rect, swatch_id(layer), Sense::click()).on_hover_text("Choose the palette colour");
        let inner = rect.shrink(ui.spacing().button_padding.y / 2.0);
        if ui.is_rect_visible(rect) {
            match current {
                PaletteEntry::Index(i) => attributes::paint_swatch(ui.painter(), inner, entry.palette.get(usize::from(i)).copied(), tokens),
                PaletteEntry::Foreground => star(ui.painter(), inner, tokens),
            }
        }
        Popup::menu(&response).id(swatch_id(layer).with("chooser")).show(|ui| {
            let mut choose = |index: serde_json::Value| {
                pending.push(("layer.setColorPaletteIndex", json!({"glyph": self.glyph, "layer": layer, "index": index})));
            };
            for (i, c) in entry.palette.iter().enumerate().take(MAX_CHOOSER_ROWS) {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
                    attributes::paint_swatch(ui.painter(), rect.shrink(ui.spacing().button_padding.y / 2.0), Some(*c), tokens);
                    let chosen = current == PaletteEntry::Index(u16::try_from(i).unwrap_or(u16::MAX));
                    if widgets::button(ui, chooser_id(layer, Some(i)), &format!("Color {i}"), &attributes::hex(*c), true, chosen) {
                        choose(json!(i));
                        ui.close();
                    }
                });
            }
            if entry.palette.len() > MAX_CHOOSER_ROWS {
                ui.label(egui::RichText::new(format!("{} more", entry.palette.len() - MAX_CHOOSER_ROWS)).color(tokens.text_dim));
            }
            if entry.palette.is_empty() {
                ui.label(egui::RichText::new("The font has no Color Palettes yet").color(tokens.text_dim));
            }
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
                star(ui.painter(), rect.shrink(ui.spacing().button_padding.y / 2.0), tokens);
                let chosen = current == PaletteEntry::Foreground;
                if widgets::button(ui, chooser_id(layer, None), "*", "The text colour the font's user chooses", true, chosen) {
                    choose(json!("*"));
                    ui.close();
                }
            });
        });
    }
}

/// The text-colour swatch: an asterisk in the text colour.
fn star(painter: &egui::Painter, rect: egui::Rect, tokens: &Tokens) {
    painter.rect_stroke(rect, f32::from(tokens.radius), Stroke::new(tokens.outline_width, tokens.border), egui::StrokeKind::Inside);
    let font = egui::TextStyle::Button.resolve(&egui::Style::default());
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, "*", font, tokens.text);
}

/// The colour-spectrum disc of a full-colour layer.
fn disc(ui: &mut egui::Ui, tokens: &Tokens) {
    let side = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let centre = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0 - ui.spacing().button_padding.y / 2.0;
    let point = |k: usize| {
        let angle = std::f32::consts::TAU * k as f32 / DISC_SEGMENTS as f32;
        centre + radius * egui::vec2(angle.cos(), -angle.sin())
    };
    for k in 0..DISC_SEGMENTS {
        let hue = egui::ecolor::Hsva::new(k as f32 / DISC_SEGMENTS as f32, 1.0, 1.0, 1.0);
        ui.painter().add(egui::Shape::convex_polygon(vec![centre, point(k), point(k + 1)], egui::Color32::from(hue), Stroke::NONE));
    }
    ui.painter().circle_stroke(centre, radius, Stroke::new(tokens.outline_width, tokens.border));
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

        fn press(&self, app: &mut FontcraftApp, id: egui::Id, clicks: usize) {
            let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(app, vec![Event::PointerMoved(at)]);
            for _ in 0..clicks {
                self.frame(app, vec![button(true)]);
                self.frame(app, vec![button(false)]);
            }
        }
    }

    fn last(app: &FontcraftApp) -> (String, Value) {
        app.session.journal().last().cloned().expect("a command ran")
    }

    /// Glyph A in Edit View with one non-master layer converted by `convert`, and a palette of
    /// red and blue. Returns the layer's id.
    fn app_with(convert: &str) -> (FontcraftApp, String) {
        let mut app = FontcraftApp::default();
        app.run("colorPalette.addColor", &json!({"color": "#FF0000"})).unwrap();
        app.run("colorPalette.addColor", &json!({"color": "#0000FF"})).unwrap();
        let master = app.session.font().unwrap().masters[0].id.clone();
        app.run("layers.addBackupLayer", &json!({"glyphs": ["A"], "master": master})).unwrap();
        let layer = app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| !l.is_master_layer()).unwrap().id.clone();
        app.run(convert, &json!({"glyph": "A", "layer": layer})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        (app, layer)
    }

    #[test]
    fn palette_layers_palette_color_chooser_sets_the_layer_index() {
        let (mut app, layer) = app_with("layer.convertToColorPalette");
        let w = Window { ctx: egui::Context::default() };
        w.settle(&mut app);
        assert!(w.ctx.read_response(label_id(&layer)).is_some(), "the row shows its bold label");
        w.press(&mut app, swatch_id(&layer), 1);
        w.press(&mut app, chooser_id(&layer, Some(1)), 1);
        let (id, params) = last(&app);
        assert_eq!((id.as_str(), params["index"].as_u64()), ("layer.setColorPaletteIndex", Some(1)));
        let font = app.session.font().unwrap();
        let l = font.glyph("A").unwrap().layers.iter().find(|l| l.id == layer).unwrap();
        assert_eq!(color_layers::kind(l), Some(ColorLayerKind::Palette(PaletteEntry::Index(1))));

        w.settle(&mut app);
        w.press(&mut app, swatch_id(&layer), 1);
        w.press(&mut app, chooser_id(&layer, None), 1);
        assert_eq!(last(&app).1["index"], "*");
    }

    #[test]
    fn palette_layers_color_layer_entry_icolor_size_is_edited_by_double_click() {
        let (mut app, layer) = app_with("layer.convertToIColor");
        let w = Window { ctx: egui::Context::default() };
        w.settle(&mut app);
        w.press(&mut app, label_id(&layer), 2);
        assert!(w.ctx.memory(|m| m.has_focus(size_field(&layer))), "the size field takes the keyboard");
        w.frame(&mut app, vec![Event::Text("64".into())]);
        w.frame(&mut app, vec![Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        let (id, params) = last(&app);
        assert_eq!((id.as_str(), params["size"].as_u64()), ("layer.setIColorSize", Some(64)));
    }

    #[test]
    fn color_layers_are_listed_and_other_layers_are_not() {
        let (app, layer) = app_with("layer.convertToColor");
        let entries = ColorEntries::read(&app, "A");
        assert_eq!(entries.entries.get(&layer).map(|e| e.label.as_str()), Some("Color"));
        let master_layer = app.session.font().unwrap().glyph("A").unwrap().layers.iter().find(|l| l.is_master_layer()).unwrap().id.clone();
        assert!(!entries.entries.contains_key(&master_layer));
    }
}
