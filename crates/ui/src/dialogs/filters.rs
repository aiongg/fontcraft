//! Filter dialogs. Every filter with parameters gets a dialog from its `filter.list` schema without a line here; a
//! spec here names a filter dialog's row in `spec/inventory` (and its id for `dialog.open`).
//!
//! Glyph as Image (`dialog.glyphAsImage`, `filter.glyphAsImage`) is a command rather than a `filter.list` filter: its
//! fields are written here. It writes files, so nothing runs until Export; an empty folder asks the chooser first.

use fontcraft_engine::Session;

use super::fields::Field;
use super::spec::{DialogSpec, Fields};

pub const SPECS: &[DialogSpec] = &[
    // Filter › Round Corners: the radius and the visual corrections of `filter.roundCorners`.
    DialogSpec::new("dialog.filterRoundCorners", "Round Corners", "filter.roundCorners", Fields::Filter),
    DialogSpec::new("dialog.glyphAsImage", "Glyph as Image", "filter.glyphAsImage", Fields::Fixed(glyph_as_image_fields))
        .run_on_ok()
        .buttons("Export", ""),
];

fn glyph_as_image_fields(session: &Session) -> Vec<Field> {
    vec![
        Field::choice("format", "Format", &[("svg", "SVG"), ("pdf", "PDF"), ("png", "PNG")], "png", "The picture format, one file per glyph"),
        super::export::master_field(session, "layer", "Master", "The master whose layers are drawn, colour layers included"),
        Field::number("pixelsPerEm", "Pixels per Em", 1.0, 16_384.0, 256.0, "The size of PNG pictures and of the image parts of PDF pictures"),
        Field::folder("directory", "Folder", "The folder the pictures are written to; files of the same name are replaced"),
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::control;

    /// Filter › Glyph as Image opens its dialog on the shown glyph; Export writes one picture per glyph.
    #[test]
    fn glyph_as_image_asks_then_writes_the_pictures() {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-glyph-as-image-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.session.execute("glyph.newGlyph", &json!({"name": "A"})).ok();
        app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();

        app.fire(CommandRef::new("filter.glyphAsImage"));
        app.drain();
        let opened = control::dispatch(&mut app, "ui.dialog.inspect", &json!({})).unwrap();
        assert_eq!(opened["dialog"], "dialog.glyphAsImage", "{opened}");
        assert_eq!(opened["params"]["format"], "png");
        assert!(!dir.exists(), "nothing is written before Export");

        control::dispatch(&mut app, "ui.dialog.set", &json!({"field": "directory", "value": dir})).unwrap();
        control::dispatch(&mut app, "ui.dialog.confirm", &json!({})).unwrap();
        assert!(dir.join("A.png").exists(), "{}", app.status);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
