//! Add pixel glyph (row `dialog.addPixelGlyph`): the offer the Pixel tool makes when the font has no glyph named
//! `pixel`. Asked after `pixel.setPixels` returns `pixelGlyphMissing` (a click or drag of the tool); Add runs
//! `pixel.addPixelGlyph` with its defaults, a square as large as the grid spacing, and Cancel leaves the font as it
//! was. The next click paints.

use serde_json::{Map, Value};

use super::spec::{After, DialogSpec, Fields};

pub const SPECS: &[DialogSpec] = &[DialogSpec::new("dialog.addPixelGlyph", "Add pixel glyph", "pixel.addPixelGlyph", Fields::Fixed(|_| Vec::new()))
    .run_on_ok()
    .buttons("Add", "")
    .message("The Pixel tool paints with the glyph named pixel, which this font does not have. Add it?")
    .after(After { command: "pixel.setPixels", params: follow_up })];

/// The parameters to open the offer with after `pixel.setPixels` returned `result`: none, so the glyph gets its
/// default size. `None` when the font has the glyph.
fn follow_up(_request: &Value, result: &Value) -> Option<Map<String, Value>> {
    (result.get("pixelGlyphMissing").and_then(Value::as_bool) == Some(true)).then(Map::new)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::behavior::CanvasView;
    use craft::behavior::kurbo::Point as KPoint;
    use craft::host::Host;
    use egui::{Pos2, Rect, vec2};
    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::control;

    fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
        control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    fn font(app: &FontCraftApp) -> Arc<fontcraft_model::Font> {
        app.session.doc().unwrap().font.clone()
    }

    fn undo_steps(app: &FontCraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// A font with a 50-unit grid; with `edit`, the empty glyph `o` is shown on the canvas with the Pixel tool.
    fn app(edit: bool) -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.session.execute("glyph.newGlyph", &json!({"name": "o"})).ok();
        app.session.execute("font.settings.set", &json!({"gridSpacing": 50})).unwrap();
        if edit {
            app.sent.push(Msg::View(ViewMsg::SelectGlyph("o".into())));
            app.drain();
            app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(300.0, 300.0), 1.0);
            call(&mut app, "ui.tool.select", json!({"tool": "pixel"}));
        }
        app
    }

    #[test]
    fn a_pixel_click_without_the_pixel_glyph_offers_it_and_add_equals_the_command() {
        let mut app = app(true);
        let click = json!({"events": [{"kind": "down", "x": 75, "y": 25}, {"kind": "up", "x": 75, "y": 25}]});

        // A click without the pixel glyph changes nothing and offers to add it.
        let before = font(&app);
        call(&mut app, "ui.pointer", click.clone());
        assert!(font(&app).same_as(&before), "the click painted nothing");
        let offer = call(&mut app, "ui.dialog.inspect", json!({}));
        assert_eq!((offer["dialog"].clone(), offer["command"].clone()), (json!("dialog.addPixelGlyph"), json!("pixel.addPixelGlyph")), "{offer}");
        assert_eq!(offer["captions"]["ok"], "Add");

        // Add gives the font the command gives.
        call(&mut app, "ui.dialog.confirm", json!({}));
        let mut direct = self::app(false);
        direct.session.execute("pixel.addPixelGlyph", &json!({})).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "Add gives the font the command gives");

        // The next click paints, as one undo step, and asks nothing.
        let steps = undo_steps(&app);
        call(&mut app, "ui.pointer", click);
        assert_eq!(undo_steps(&app), steps + 1, "the click is one undo step");
        assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], false);
    }
}
