//! Add pixel glyph: the offer the Pixel tool makes when the font has no glyph named `pixel`.
//! Asked after `pixel.setPixels` returns `pixelGlyphMissing` (a click or drag of the tool);
//! Add runs `pixel.addPixelGlyph` with its defaults, a square as large as the grid spacing, and
//! Cancel leaves the font as it was. The next click paints.

use serde_json::{Map, Value};

use super::{After, Dialog, Fields};

pub const DIALOG: Dialog = Dialog {
    follow_up: true,
    after: Some(After { command: "pixel.setPixels", params: follow_up }),
    buttons: ["Add", "Cancel"],
    message: "The Pixel tool paints with the glyph named pixel, which this font does not have. Add it?",
    ..Dialog::new("dialog.addPixelGlyph", "Add pixel glyph", "pixel.addPixelGlyph", Fields::Fixed(|_| Vec::new()))
};

/// The parameters to open the offer with after `pixel.setPixels` returned `result`: none, so
/// the glyph gets its default size. `None` when the font has the glyph.
pub fn follow_up(_request: &Value, result: &Value) -> Option<Map<String, Value>> {
    (result.get("pixelGlyphMissing").and_then(Value::as_bool) == Some(true)).then(Map::new)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::DIALOG;
    use crate::{FontcraftApp, control};

    fn font(app: &FontcraftApp) -> Arc<fontcraft_model::Font> {
        app.session.doc().unwrap().font.clone()
    }

    fn undo_steps(app: &FontcraftApp) -> usize {
        app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
    }

    /// Pointer events in font units of the edited glyph; every command they run must succeed.
    fn pointer(app: &mut FontcraftApp, events: Value) {
        let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["errors"], json!([]), "{r}");
        assert_eq!(r["result"]["busy"], false, "{r}");
    }

    /// An app editing the empty glyph `o` with a 50-unit grid, and one that runs the commands.
    fn apps() -> (FontcraftApp, FontcraftApp) {
        let mut app = FontcraftApp::default();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["o"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [300, 300]})).unwrap();
        app.run("font.settings.set", &json!({"gridSpacing": 50})).unwrap();
        app.run("tool.select", &json!({"tool": "pixel"})).unwrap();
        let mut direct = FontcraftApp::default();
        direct.run("font.settings.set", &json!({"gridSpacing": 50})).unwrap();
        (app, direct)
    }

    #[test]
    fn the_offer_and_each_gesture_equal_their_command() {
        let (mut app, mut direct) = apps();

        // A click without the pixel glyph changes nothing and offers to add it; Add equals the
        // command.
        let before = font(&app);
        pointer(&mut app, json!([{"kind": "down", "x": 75, "y": 25}, {"kind": "up", "x": 75, "y": 25}]));
        assert_eq!(app.dialog.as_ref().map(|d| d.id.as_str()), Some(DIALOG.id), "{}", app.state.status);
        let r = control::handle(&mut app, &json!({"id": 1, "method": "ui.dialog.confirm", "params": {}}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(!font(&app).same_as(&before));
        direct.run("pixel.addPixelGlyph", &json!({})).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "Add gives the font the command gives");

        let click = json!([{"kind": "down", "x": 75, "y": 25}, {"kind": "up", "x": 75, "y": 25}]);
        let gestures = [
            // A click on an empty cell adds a pixel there.
            (click.clone(), json!({"cells": [[1, 0]], "mode": "add"})),
            // A click on a pixel removes it.
            (click, json!({"cells": [[1, 0]], "mode": "remove"})),
            // A drag from an empty cell adds a pixel in every cell it crosses.
            (
                json!([{"kind": "down", "x": 25, "y": 25}, {"kind": "drag", "x": 100, "y": 25}, {"kind": "drag", "x": 175, "y": 25}, {"kind": "up", "x": 175, "y": 25}]),
                json!({"cells": [[0, 0], [1, 0], [2, 0], [3, 0]], "mode": "add"}),
            ),
            // A drag from a pixel removes the pixels in every cell it crosses.
            (
                json!([{"kind": "down", "x": 25, "y": 25}, {"kind": "drag", "x": 125, "y": 25}, {"kind": "up", "x": 125, "y": 25}]),
                json!({"cells": [[0, 0], [1, 0], [2, 0]], "mode": "remove"}),
            ),
        ];
        for (events, mut params) in gestures {
            let steps = undo_steps(&app);
            pointer(&mut app, events.clone());
            assert_eq!(undo_steps(&app), steps + 1, "{events} is one undo step");
            params["glyph"] = json!("o");
            direct.run("pixel.setPixels", &params).unwrap();
            assert!(font(&app).same_as(&font(&direct)), "{events} gives the font {params} gives");
        }
        assert!(app.dialog.is_none());
    }
}
