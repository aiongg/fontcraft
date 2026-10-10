//! The system clipboard.
//!
//! In Edit View, Copy and Cut run `edit.copy` and `edit.cut` on the selection and put the `json`
//! text of the result on the system clipboard. egui hands the platform one text per copy, and our
//! own fragment pastes back without loss; `edit.paste` also reads the SVG other apps put there.
//!
//! In Font View, Copy runs `edit.copy` with the selected glyphs (and the master shown) and puts
//! the copied glyphs, the `fontcraft-glyphs` text, on the clipboard; Paste runs `edit.paste` with
//! that text, which adds the glyphs (a taken name gets the first free `.001` suffix), in this
//! document or another. Text that holds no copied glyphs is not pasted in Font View.
//!
//! Paste takes the text the platform hands over with the key (`egui::Event::Paste`) and runs
//! `edit.paste {text}` as the person's run, so the question about an outline pasted far outside
//! the glyph can open. Edit › Paste from the menu carries no text: it asks the platform for the
//! clipboard first ([`request_text`]), and the text arrives as a paste event in the next frame.
//!
//! Paste Special (Option held with the paste key, or Edit › Paste Special, which asks for the
//! text the same way) opens `dialog.pasteSpecial` on copied glyphs, with the selected glyphs as
//! its `targets` and the master shown.
//!
//! Nothing here runs while a text field has the keyboard (it copies and pastes text itself) or a
//! dialog is open.

use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::tools_glue;

const COPY: &str = "edit.copy";
const CUT: &str = "edit.cut";
const PASTE: &str = "edit.paste";
const PASTE_SPECIAL: &str = "edit.pasteSpecial";
/// The `format` the engine writes into copied glyphs (`edit.copy` with `glyphs`).
const GLYPHS_FORMAT: &str = "fontcraft-glyphs";

/// Longest clipboard text a paste takes.
pub const MAX_PASTE_BYTES: usize = 16 * 1024 * 1024;

/// The egui memory key under which Paste Special's request for the clipboard keeps the frame it
/// was made in: the text arrives in the next frame or not at all.
fn special_key() -> egui::Id {
    egui::Id::new("fontcraft-paste-special")
}

/// This frame's Copy, Cut and Paste events, as the commands they run.
pub fn events(app: &mut FontcraftApp, ctx: &egui::Context) -> Vec<(&'static str, Value)> {
    let asked = ctx.data_mut(|d| d.remove_temp::<u64>(special_key()));
    let special_asked = asked.is_some_and(|frame| ctx.cumulative_frame_nr() <= frame.saturating_add(1));
    if app.dialog.is_some() || ctx.egui_wants_keyboard_input() {
        return Vec::new();
    }
    let (events, alt) = ctx.input(|i| {
        let events =
            i.events.iter().filter(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))).cloned().collect::<Vec<_>>();
        (events, i.modifiers.alt)
    });
    let font_view = app.state.tab().is_none();
    let mut out = Vec::new();
    for event in events {
        match event {
            egui::Event::Copy if font_view => {
                let params = font_view_params(app, COPY, json!({}));
                if params.get("glyphs").is_some() {
                    out.push((COPY, params));
                }
            }
            egui::Event::Cut if font_view => {}
            egui::Event::Copy => out.push((COPY, json!({}))),
            egui::Event::Cut => out.push((CUT, json!({}))),
            egui::Event::Paste(text) if text.len() > MAX_PASTE_BYTES => {
                app.state.status = format!("Paste: the clipboard holds more than {MAX_PASTE_BYTES} bytes");
            }
            egui::Event::Paste(text) => {
                if (special_asked || alt) && holds_glyphs(&text) {
                    out.push(("dialog.open", paste_special(app, text)));
                } else if font_view {
                    if holds_glyphs(&text) {
                        out.push((PASTE, json!({"text": text})));
                    } else {
                        app.state.status = "Paste: the clipboard holds no copied glyphs; open a glyph to paste an outline into it".into();
                    }
                } else {
                    let mut params = tools_glue::with_targets(app, PASTE, Value::Null);
                    if let Some(m) = params.as_object_mut() {
                        m.insert("text".into(), json!(text));
                        out.push((PASTE, params));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Copy's parameters in Font View, where it copies whole glyphs: the selected glyphs and the
/// master shown, when `params` names nothing. Any other run keeps its `params`.
pub fn font_view_params(app: &FontcraftApp, id: &str, params: Value) -> Value {
    let empty = params.is_null() || params.as_object().is_some_and(Map::is_empty);
    if id != COPY || !empty || app.state.tab().is_some() || app.state.selected_glyphs.is_empty() {
        return params;
    }
    let mut m = Map::new();
    m.insert("glyphs".into(), json!(app.state.selected_glyphs));
    if let Some(master) = app.render_master() {
        m.insert("master".into(), json!(master));
    }
    Value::Object(m)
}

/// Whether clipboard text holds copied glyphs rather than a drawing.
fn holds_glyphs(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains(GLYPHS_FORMAT)
}

/// `dialog.open`'s parameters for Paste Special on `text`: the selected glyphs (the edited glyph
/// in Edit View) as `targets`, and the master shown.
fn paste_special(app: &FontcraftApp, text: String) -> Value {
    let targets: Vec<String> = match app.state.tab() {
        Some(tab) => tab.edited_glyph().map(str::to_string).into_iter().collect(),
        None => app.state.selected_glyphs.clone(),
    };
    let mut params = Map::new();
    params.insert("text".into(), json!(text));
    if !targets.is_empty() {
        params.insert("targets".into(), json!(targets));
    }
    if let Some(master) = app.render_master() {
        params.insert("master".into(), json!(master));
    }
    json!({"dialog": "dialog.pasteSpecial", "params": params})
}

/// A run of `id` with `params` that needs the clipboard's text first: Paste or Paste Special
/// with nothing to paste.
pub fn wants_text(id: &str, params: &Value) -> bool {
    (id == PASTE || id == PASTE_SPECIAL) && ["json", "svg", "text"].iter().all(|k| params.get(k).is_none_or(Value::is_null))
}

/// Asks the platform for the clipboard's text for a run of `id` that [`wants_text`]; it arrives
/// as a paste event in the next frame, which Paste Special's run marks as its own.
pub fn request_text(ctx: &egui::Context, id: &str) {
    if id == PASTE_SPECIAL {
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(special_key(), frame));
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

/// After the person's run of `id`: Copy and Cut put their `json` text on the system clipboard
/// (sent to the platform at the end of the frame).
pub fn after_run(app: &mut FontcraftApp, id: &str, result: &Value) {
    if (id == COPY || id == CUT)
        && let Some(text) = result.get("json").and_then(Value::as_str)
    {
        app.clipboard_out = Some(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, OutputCommand, Pos2, ViewportCommand, ViewportId};
    use serde_json::json;

    use crate::FontcraftApp;

    /// Glyph A with a 200-unit square, edited in a tab.
    fn square_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app
    }

    fn frame(ctx: &egui::Context, app: &mut FontcraftApp, events: Vec<Event>) -> egui::FullOutput {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        output
    }

    fn copied(output: &egui::FullOutput) -> Vec<String> {
        output
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn shapes(app: &FontcraftApp) -> usize {
        app.session.font().unwrap().glyph("A").unwrap().layers.first().unwrap().shapes.len()
    }

    #[test]
    fn copy_cut_and_paste_in_edit_view_use_the_system_clipboard() {
        let mut app = square_app();
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, Vec::new());
        let expected = app.run("edit.copy", &json!({"glyph": "A"})).unwrap()["json"].as_str().unwrap().to_string();

        let output = frame(&ctx, &mut app, vec![Event::Copy]);
        assert_eq!(copied(&output), std::slice::from_ref(&expected), "Copy puts edit.copy's json on the clipboard");

        let output = frame(&ctx, &mut app, vec![Event::Paste(expected.clone())]);
        assert!(copied(&output).is_empty());
        assert_eq!(shapes(&app), 2, "Paste added the copied square");
        let mut direct = square_app();
        direct.run("edit.paste", &json!({"glyph": "A", "text": expected})).unwrap();
        assert!(app.session.font().unwrap().same_as(direct.session.font().unwrap()), "the same font as edit.paste with the text");

        let output = frame(&ctx, &mut app, vec![Event::Cut]);
        assert_eq!(copied(&output).len(), 1, "Cut puts the text on the clipboard");
        assert!(shapes(&app) < 2, "and removes what it copied");

        // A key without the platform's text (Edit › Paste, or Cmd+V as a plain key) asks the
        // platform for the clipboard instead of running edit.paste with nothing.
        let output = frame(
            &ctx,
            &mut app,
            vec![Event::Key { key: egui::Key::V, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }],
        );
        let commands = output.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
        assert!(commands.contains(&ViewportCommand::RequestPaste), "{commands:?}");
    }

    #[test]
    fn a_paste_far_outside_the_glyph_asks_the_question_and_font_view_takes_no_outline() {
        let mut app = square_app();
        let ctx = egui::Context::default();
        let far = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><rect x="5000" y="0" width="100" height="100"/></svg>"#;
        frame(&ctx, &mut app, vec![Event::Paste(far.into())]);
        assert_eq!(app.dialog.as_ref().map(|d| d.id.as_str()), Some("dialog.pastedOutlineBounds"), "a person's paste asks");
        crate::dialogs::cancel(&mut app).unwrap();

        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        let before = app.session.font().unwrap().clone();
        let output = frame(&ctx, &mut app, vec![Event::Copy, Event::Paste(far.into())]);
        assert!(copied(&output).is_empty());
        assert!(app.session.font().unwrap().same_as(&before), "Font View takes no outline paste");
    }
}
