//! Palette › Anchor Context (M8.21), in headless frames: it shows for one selected `*` anchor
//! while Show Info is on, and its field sets the context as `anchor.setContext` does.

use egui::{Event, Pos2};
use serde_json::json;

use crate::FontcraftApp;
use crate::panels::{SECTIONS, palette_anchor_context};

/// The default document's `n` with a plain `top` and a contextual `*top` anchor, edited, Show Info on, the built-in
/// Palette sections folded so the contextual section sits in view.
fn app_with_anchors() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("anchor.add", &json!({"glyph": "n", "name": "top", "x": 250, "y": 500})).unwrap();
    app.run("anchor.add", &json!({"glyph": "n", "name": "*top", "x": 300, "y": 520})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["n"]})).unwrap();
    app.state.display.info = true;
    for (section, _) in SECTIONS {
        app.run("palette.toggleSection", &json!({"section": section, "expanded": false})).unwrap();
    }
    app
}

fn frame(ctx: &egui::Context, app: &mut FontcraftApp, events: Vec<Event>) {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    output.textures_delta.clear();
}

fn stored(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().unwrap();
    let layer = font.glyph("n").unwrap().layer(&font.masters[0].id).unwrap().clone();
    let anchor = layer.anchors.iter().find(|a| a.name == "*top").unwrap().clone();
    fontcraft_engine::features::marks::contextual::anchor_context(&anchor).map(str::to_string)
}

#[test]
fn anchor_context_field_shows_for_one_contextual_anchor_and_sets_its_context() {
    let mut app = app_with_anchors();
    app.run("selection.set", &json!({"anchors": ["top"]})).unwrap();
    assert!(!palette_anchor_context::shown(&app), "a plain anchor has no context field");
    app.run("selection.set", &json!({"anchors": ["top", "*top"]})).unwrap();
    assert!(!palette_anchor_context::shown(&app), "two anchors selected");
    app.run("selection.set", &json!({"anchors": ["*top"]})).unwrap();
    let sel = palette_anchor_context::selected(&app).expect("the field shows");
    assert_eq!((sel.anchor.as_str(), sel.context.as_str()), ("*top", ""));

    // Typing into the field and moving the focus away commits it.
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut app, Vec::new());
    }
    let id = palette_anchor_context::field_id("*top");
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut app, Vec::new());
    assert!(ctx.memory(|m| m.has_focus(id)), "the field takes the focus");
    frame(&ctx, &mut app, vec![Event::Text("o *".into())]);
    ctx.memory_mut(|m| m.surrender_focus(id));
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(stored(&app).as_deref(), Some("o *"), "{}", app.state.status);
    assert_eq!(palette_anchor_context::selected(&app).unwrap().context, "o *");

    let mut headless = app_with_anchors();
    headless.run("anchor.setContext", &json!({"glyph": "n", "anchor": "*top", "context": "o *"})).unwrap();
    assert_eq!(stored(&headless), stored(&app), "the field gives what the command gives");
}
