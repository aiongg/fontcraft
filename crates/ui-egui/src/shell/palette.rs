//! The Palette sidebar.

use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

pub const COMMANDS: &[UiCommand] =
    &[("window.palette", "Palette", &["Window"], "Cmd+Alt+P", "{on?: bool} show or hide the Palette sidebar (omitted: toggle)")];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "window.palette" => {
            app.state.show_palette = args.toggle(app.state.show_palette)?;
            Ok(json!({"on": app.state.show_palette}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Draws the Palette: the edited glyph, else the first glyph selected in Font View.
pub fn show(app: &FontcraftApp, ui: &mut egui::Ui) {
    let tokens = &app.tokens;
    let frame = egui::Frame::NONE.fill(tokens.panel).inner_margin(8.0);
    let font = app.session.font().ok();
    let target = app.state.editing().map(str::to_string).or_else(|| app.state.selected_glyphs.first().cloned());
    egui::Panel::right("palette").frame(frame).default_size(tokens.palette_width).show(ui, |ui| {
        ui.label(egui::RichText::new("Palette").strong());
        ui.separator();
        let (Some(font), Some(name)) = (font, target) else {
            ui.label(egui::RichText::new("No glyph selected").color(tokens.text_dim));
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
