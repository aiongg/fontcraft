//! Image files dropped on the window (`image.dragAndDrop`, `fontView.image.dropOnCell`), placed as Glyph › Add Image…
//! places them (`glyph.addImage`).
//!
//! | Where | One file | Several files |
//! |---|---|---|
//! | the canvas, while an Edit View tab shows ([`take`]) | the edited glyph's active layer, on the side being edited | the glyphs their names match, on the edited layer's master |
//! | a Font View cell (`panels::font_view::grid`, [`cell_params`]) | that glyph's layer of Font View's master | the glyphs their names match, on Font View's master |
//!
//! On the web a dropped file has no path, only a name, and is not placed (its bytes would need the memory store).

use std::path::PathBuf;

use fontcraft_engine::assets::MAX_ADD_IMAGE_FILES;
use fontcraft_engine::imagery::ImageFormat;
use fontcraft_engine::model::LayerSide;
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::canvas::tools;

/// Whether the file name says it is an image Add Image places.
pub fn is_image(name: &str) -> bool {
    ImageFormat::from_extension(name) != ImageFormat::Other
}

/// The absolute paths among `paths` that name images, at most [`MAX_ADD_IMAGE_FILES`].
fn image_paths<'a>(paths: impl Iterator<Item = &'a std::path::Path>) -> Vec<String> {
    paths
        .filter(|p| p.is_absolute() && p.file_name().and_then(|n| n.to_str()).is_some_and(is_image))
        .take(MAX_ADD_IMAGE_FILES)
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Takes the image files dropped on the canvas out of this frame's input and places them. Runs in `logic`, after the
/// font and extension files are taken; while Font View shows, the files are left for its cells.
pub fn take(app: &mut FontCraftApp, ctx: &egui::Context) {
    if app.font_view_shows() {
        return;
    }
    let canvas = app.view.canvas.rect;
    let paths: Vec<PathBuf> = ctx.input_mut(|i| {
        let over = i.pointer.hover_pos().is_none_or(|p| canvas.contains(p));
        if !over || !i.raw.dropped_files.iter().any(|f| f.path().file_name().and_then(|n| n.to_str()).is_some_and(is_image)) {
            return Vec::new();
        }
        let (ours, rest): (Vec<_>, Vec<_>) =
            std::mem::take(&mut i.raw.dropped_files).into_iter().partition(|f| f.path().file_name().and_then(|n| n.to_str()).is_some_and(is_image));
        i.raw.dropped_files = rest;
        ours.iter().map(|f| f.path().to_path_buf()).collect()
    });
    if let Some(params) = canvas_params(app, &paths) {
        app.sent.push(Msg::run("glyph.addImage", params));
    }
}

/// The `glyph.addImage` parameters for `paths` dropped on the canvas; `None` without an image or an edited glyph.
pub fn canvas_params(app: &FontCraftApp, paths: &[PathBuf]) -> Option<Value> {
    let files = image_paths(paths.iter().map(PathBuf::as_path));
    let target = tools::target(&app.session, &app.view)?;
    let font = app.session.font().ok()?;
    let layer = tools::active_layer(font, &app.view)?;
    let mut m = Map::new();
    match files.as_slice() {
        [] => return None,
        [one] => {
            m.insert("files".into(), json!([one]));
            m.insert("glyphs".into(), json!([target.glyph]));
            m.insert("layer".into(), json!(target.layer));
            if target.side == LayerSide::Background {
                m.insert("side".into(), json!("background"));
            }
        }
        many => {
            m.insert("files".into(), json!(many));
            m.insert("layer".into(), json!(layer.master));
        }
    }
    Some(Value::Object(m))
}

/// The `glyph.addImage` parameters for `paths` dropped on the cell of `glyph`, Font View showing `master`; `None`
/// without an image.
pub fn cell_params(glyph: &str, master: &str, paths: &[PathBuf]) -> Option<Value> {
    let files = image_paths(paths.iter().map(PathBuf::as_path));
    match files.as_slice() {
        [] => None,
        [one] => Some(json!({"files": [one], "glyphs": [glyph], "layer": master})),
        many => Some(json!({"files": many, "layer": master})),
    }
}
