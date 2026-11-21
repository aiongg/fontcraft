//! Reveal Image File (`image.revealFile`): the file of a layer's placed image in the system's file manager, through
//! `FontCraftApp::opener`. The image Info box's arrow runs it (`panels::info_box_image`).

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::LayerSide;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use super::params;
use crate::app::FontCraftApp;
use crate::canvas::images::side_image;
use crate::canvas::tools;

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "image.revealFile",
    label: "Reveal Image File",
    menu: &[],
    shortcut: None,
    params: "{glyph?: name, layer?: id, side?: \"foreground\" | \"background\"} show the file of the layer's image in the system's file manager; without `glyph`, the edited glyph's active layer on the side being edited; without `layer`, the edited layer of the edited glyph, else the first master's",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run };

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "image.revealFile" => reveal(app, &command.params),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn reveal(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let given = params::string(p, "glyph")?;
    let target = tools::target(&app.session, &app.view);
    let glyph = match given {
        Some(g) => g.to_string(),
        None => target.as_ref().map(|t| t.glyph.clone()).ok_or("give `glyph`: no glyph is being edited")?,
    };
    let side = match params::string(p, "side")? {
        Some("background") => LayerSide::Background,
        Some("foreground") => LayerSide::Foreground,
        Some(other) => return Err(format!("`side` must be \"foreground\" or \"background\", not {}", params::quoted(other))),
        None => target.as_ref().filter(|_| given.is_none()).map_or(LayerSide::Foreground, |t| t.side),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let g = font.glyph(&glyph).ok_or_else(|| format!("there is no glyph {}", params::quoted(&glyph)))?;
    let edited = target.as_ref().filter(|t| t.glyph == glyph).and_then(|t| g.layers.iter().find(|l| l.id == t.layer));
    let layer = match params::string(p, "layer")? {
        Some(id) => {
            g.layers.iter().find(|l| l.id == id).ok_or_else(|| format!("glyph {} has no layer {}", params::quoted(&glyph), params::quoted(id)))?
        }
        None => edited
            .or_else(|| font.masters.first().and_then(|m| g.layer(&m.id)))
            .ok_or_else(|| format!("glyph {} has no layer in the first master", params::quoted(&glyph)))?,
    };
    let image =
        side_image(layer, side).ok_or_else(|| format!("layer {} of glyph {} has no image", params::quoted(&layer.id), params::quoted(&glyph)))?;
    let stored = image.path.as_deref().ok_or("the image names no file")?;
    let file = app.session.asset_file(stored).ok_or_else(|| format!("the image file {} cannot be found", params::quoted(stored)))?;
    // The opener refuses a file that does not exist, and says so when this build has no file manager.
    app.opener.reveal(&file)
}
