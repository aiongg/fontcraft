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
//! Drawing apps put a PDF on the clipboard, which egui does not carry. In Edit View the desktop
//! app's [`ClipboardFlavours`] reads it off the window's thread. A paste whose text is neither our
//! fragment nor SVG, Edit › Paste with no text on the clipboard (the text request answers nothing
//! in the next frame), and the paste key with no text (the platform then sends no paste event,
//! only the key's release) start the read, and take the edited glyph and document as they are
//! then. When the read ends, a later frame runs `edit.paste {pdf}` on that glyph with the PDF as
//! base64, or with the text when the clipboard holds no PDF. One stroke of the paste key pastes
//! once, however long the key is held or the read takes, and the command key may be let go before
//! the paste key. Paste Special, and Option held with the paste key, never read the PDF. Without
//! a reader (the browser, tests) only text is pasted.
//!
//! Paste Special (Option held with the paste key, or Edit › Paste Special, which asks for the
//! text the same way) opens `dialog.pasteSpecial` on copied glyphs, with the selected glyphs as
//! its `targets` and the master shown.
//!
//! In text mode Copy and Cut put the selected text on the clipboard, a glyph without a
//! character as `/name`, and Paste types the clipboard's text at the cursor (`text.clipboard`).
//! Paste Special is refused there.
//!
//! While the Kerning window's pair list has the keyboard, or had it when the menu was opened,
//! Copy and Paste, from the keys or the Edit menu, copy and paste the selected pairs there
//! ([`kerning_window_copy`]).
//!
//! While a project window has the keyboard, it copies and pastes its instances itself
//! (`project_window`).
//!
//! Nothing here runs while a text field has the keyboard (it copies and pastes text itself) or a
//! dialog is open.

use base64::Engine as _;
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
/// Largest PDF a paste takes from the clipboard: `edit.paste`'s limit.
pub const MAX_PASTE_PDF_BYTES: usize = 64 << 20;
/// How often a frame looks for the end of a clipboard read, should the reader not ask for one.
const READ_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// What reading the clipboard's PDF gave: the PDF, `None` when the clipboard holds none, or why
/// it could not be read (data larger than the limit included).
pub type PdfRead = Result<Option<Vec<u8>>, String>;

/// Clipboard data that egui does not carry. The desktop app gives
/// [`FontcraftApp::clipboard_flavours`] one; without it (the browser, tests) only text is pasted.
pub trait ClipboardFlavours: Send {
    /// Starts reading the PDF on the system clipboard, refusing more than `max_bytes`, unless a
    /// read is running already. The read must not hold up the caller: the desktop app runs it on
    /// a thread of its own and asks `ctx` for a repaint when it ends.
    fn request_pdf(&mut self, max_bytes: usize, ctx: &egui::Context);
    /// The result of the read started last, once it has ended; `None` while it runs, and after
    /// its result was taken.
    fn take_pdf(&mut self) -> Option<PdfRead>;
}

/// A clipboard read under way: `edit.paste`'s parameters with the targets of the moment it
/// started, the document they are in, and the text to paste when the clipboard holds no PDF.
#[derive(Clone)]
struct PendingPdf {
    params: Value,
    document: Option<u64>,
    text: Option<String>,
}

/// The paste key's stroke from its press to its release.
#[derive(Clone, Copy)]
struct Stroke {
    /// The key went down without the command key: a plain V, not the paste key.
    plain: bool,
    /// The stroke has pasted, or asked for the clipboard, already.
    pasted: bool,
}

/// The egui memory key under which Paste Special's request for the clipboard keeps the frame it
/// was made in: the text arrives in the next frame or not at all.
fn special_key() -> egui::Id {
    egui::Id::new("fontcraft-paste-special")
}

/// The egui memory key under which Edit › Paste's request for the clipboard keeps the frame it was
/// made in: with no text arriving in the next frame, the clipboard's PDF is pasted.
fn paste_key() -> egui::Id {
    egui::Id::new("fontcraft-paste-request")
}

/// The egui memory key holding the paste key's [`Stroke`] while the key is down.
fn stroke_key() -> egui::Id {
    egui::Id::new("fontcraft-paste-stroke")
}

/// The egui memory key holding the [`PendingPdf`] of a running clipboard read.
fn pending_key() -> egui::Id {
    egui::Id::new("fontcraft-paste-pending-pdf")
}

/// Follows the paste key through this frame's events: whether it was let go without having
/// pasted. `asked`: this frame answers the stroke's request for the clipboard, which pastes.
///
/// The platform takes the paste key's press for its paste command: it sends the clipboard's text
/// as a paste event, or nothing when there is no text, never the key press. So a release with no
/// press seen is the paste key's, whether or not the command key is still down. A press seen
/// with the command key comes from a platform that sends it (the browser, tests) and is the paste
/// key too; a press seen without it is a plain V.
fn paste_key_released(ctx: &egui::Context, asked: bool) -> bool {
    let mut stroke = ctx.data_mut(|d| d.get_temp::<Stroke>(stroke_key()));
    if asked && let Some(s) = stroke.as_mut() {
        s.pasted = true;
    }
    let mut released = false;
    ctx.input(|i| {
        for event in &i.raw.events {
            match event {
                // A press while the stroke is down repeats the key.
                egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if stroke.is_none() => {
                    stroke = Some(Stroke { plain: !modifiers.command, pasted: false });
                }
                egui::Event::Paste(_) if !asked => stroke.get_or_insert(Stroke { plain: false, pasted: false }).pasted = true,
                egui::Event::Key { key: egui::Key::V, pressed: false, modifiers, .. } => {
                    let ended = stroke.take();
                    let paste_key = ended.is_none_or(|s| !s.plain) || modifiers.command;
                    released = paste_key && !ended.is_some_and(|s| s.pasted);
                }
                // The paste command's other keys end their stroke; a window that loses the
                // keyboard never sees the release.
                egui::Event::Key { key: egui::Key::Paste | egui::Key::Insert, pressed: false, .. } | egui::Event::WindowFocused(false) => {
                    stroke = None;
                }
                _ => {}
            }
        }
    });
    ctx.data_mut(|d| {
        if let Some(s) = stroke {
            d.insert_temp(stroke_key(), s);
        } else {
            d.remove::<Stroke>(stroke_key());
        }
    });
    released
}

/// This frame's Copy, Cut and Paste events, as the commands they run.
pub fn events(app: &mut FontcraftApp, ctx: &egui::Context) -> Vec<(&'static str, Value)> {
    let frame = ctx.cumulative_frame_nr();
    let asked = ctx.data_mut(|d| d.remove_temp::<u64>(special_key()));
    let special_asked = asked.is_some_and(|f| frame <= f.saturating_add(1));
    let asked = ctx.data_mut(|d| d.remove_temp::<u64>(paste_key()));
    let paste_asked = asked.is_some_and(|f| frame <= f.saturating_add(1));
    let key_released = paste_key_released(ctx, paste_asked || special_asked);
    let (text_pasted, alt) = ctx.input(|i| (i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))), i.modifiers.alt));
    let special = special_asked || alt;
    let mut out = Vec::new();
    // A read started in an earlier frame pastes into the glyph it was started for; it waits
    // while a dialog is open or the Kerning window would take the paste.
    if app.dialog.is_none()
        && !super::kerning_window::takes_clipboard(app)
        && let Some(params) = finish_pdf(app, ctx)
    {
        out.push((PASTE, params));
    }
    if app.dialog.is_some()
        || ctx.text_edit_focused()
        || super::kerning_window::takes_clipboard(app)
        || super::font_info::has_keyboard(app)
        || super::project_window::has_keyboard(app, ctx).is_some()
    {
        return out;
    }
    let events = ctx.input(|i| {
        i.events.iter().filter(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))).cloned().collect::<Vec<_>>()
    });
    if super::text::in_text_mode(app) {
        text_mode_events(app, events, special);
        return out;
    }
    let font_view = app.state.tab().is_none();
    // No text came for Edit › Paste, or the paste key gave none: the clipboard may hold a PDF.
    let key_without_text = key_released && !special;
    if !font_view && !text_pasted && (paste_asked || key_without_text) {
        start_pdf(app, ctx, None);
    }
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
                } else if !special && !pasteable(&text) && app.clipboard_flavours.is_some() {
                    start_pdf(app, ctx, Some(text));
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

/// Whether `edit.paste` can read clipboard text: JSON (our fragment or copied glyphs) or an SVG
/// document. Other text gives way to the clipboard's PDF.
fn pasteable(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with('{') || (t.starts_with('<') && t.contains("<svg"))
}

/// Starts reading the clipboard's PDF for a paste into the edited glyph, with `text` to paste
/// should the clipboard hold no PDF; nothing without a reader. While another read runs, this
/// paste is dropped with a message.
fn start_pdf(app: &mut FontcraftApp, ctx: &egui::Context, text: Option<String>) {
    let params = tools_glue::with_targets(app, PASTE, Value::Null);
    let document = app.session.active().map(|d| d.id);
    let pending = PendingPdf { params, document, text };
    let Some(reader) = app.clipboard_flavours.as_mut() else { return };
    if ctx.data_mut(|d| d.get_temp::<PendingPdf>(pending_key())).is_some() {
        app.state.status = "Paste: the clipboard is still being read for the last paste".into();
        return;
    }
    reader.request_pdf(MAX_PASTE_PDF_BYTES, ctx);
    ctx.data_mut(|d| d.insert_temp(pending_key(), pending));
    ctx.request_repaint_after(READ_POLL);
}

/// `edit.paste`'s parameters once the running clipboard read has ended: the PDF with the targets
/// taken when the read started, or the text when the clipboard holds no PDF. `None` while the
/// read runs or when there is nothing to paste; a PDF that cannot be read, or a document closed
/// or switched meanwhile, leaves a message in the status bar.
fn finish_pdf(app: &mut FontcraftApp, ctx: &egui::Context) -> Option<Value> {
    let pending = ctx.data_mut(|d| d.get_temp::<PendingPdf>(pending_key()))?;
    let read = match app.clipboard_flavours.as_mut() {
        Some(reader) => match reader.take_pdf() {
            Some(read) => read,
            None => {
                ctx.request_repaint_after(READ_POLL);
                return None;
            }
        },
        None => Ok(None),
    };
    ctx.data_mut(|d| d.remove::<PendingPdf>(pending_key()));
    let PendingPdf { mut params, document, text } = pending;
    let flavour = match read {
        Ok(Some(bytes)) if bytes.len() > MAX_PASTE_PDF_BYTES => {
            app.state.status = format!("Paste: the clipboard's PDF is larger than {} MiB", MAX_PASTE_PDF_BYTES >> 20);
            return None;
        }
        Ok(Some(bytes)) => ("pdf", Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))),
        Ok(None) => ("text", Value::String(text?)),
        // The text, if any, is neither our fragment nor SVG, so the reason the PDF failed is
        // the one worth showing.
        Err(e) => {
            app.state.status = format!("Paste: the clipboard's PDF cannot be read: {e}");
            return None;
        }
    };
    if app.session.active().map(|d| d.id) != document {
        app.state.status = "Paste: the document changed while the clipboard was read; nothing was pasted".into();
        return None;
    }
    params.as_object_mut()?.insert(flavour.0.into(), flavour.1);
    Some(params)
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

/// Edit › Copy while the Kerning window takes the clipboard: the selected pairs go on it as
/// kerning-text. `None`: the run is not the window's. (Edit › Paste asks for the clipboard's text
/// as everywhere, and the window takes the text when it arrives.)
/// The Font Info window's Copy and Cmd+Delete on its rows come first (`font_info::menu_command`).
pub fn kerning_window_copy(app: &mut FontcraftApp, id: &str) -> Option<Result<Value, String>> {
    if let Some(result) = super::font_info::menu_command(app, id) {
        return Some(result);
    }
    if id != COPY || !super::kerning_window::takes_clipboard(app) {
        return None;
    }
    super::kerning_window::copy(app);
    Some(Ok(json!({"kerningPairs": true, "status": app.state.status})))
}

/// A run of `id` with `params` that needs the clipboard's text first: Paste or Paste Special
/// with nothing to paste.
pub fn wants_text(id: &str, params: &Value) -> bool {
    (id == PASTE || id == PASTE_SPECIAL) && ["json", "svg", "pdf", "text"].iter().all(|k| params.get(k).is_none_or(Value::is_null))
}

/// Asks the platform for the clipboard's text for a run of `id` that [`wants_text`]; it arrives
/// as a paste event in the next frame, which Paste Special's run marks as its own. Paste marks
/// the frame too, so that the clipboard's PDF is pasted when no text arrives.
pub fn request_text(ctx: &egui::Context, id: &str) {
    let frame = ctx.cumulative_frame_nr();
    if id == PASTE_SPECIAL {
        ctx.data_mut(|d| d.insert_temp(special_key(), frame));
    } else if id == PASTE {
        ctx.data_mut(|d| d.insert_temp(paste_key(), frame));
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    ctx.request_repaint();
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

/// Copy, Cut and Paste in text mode, on the tab's text. Paste Special (`special`) is refused.
fn text_mode_events(app: &mut FontcraftApp, events: Vec<egui::Event>, special: bool) {
    for event in events {
        let result = match event {
            egui::Event::Paste(_) if special => Err(super::text::NO_PASTE_SPECIAL.to_string()),
            egui::Event::Copy => super::text::menu_command(app, COPY).unwrap_or(Ok(Value::Null)).map(|_| ()),
            egui::Event::Cut => super::text::menu_command(app, CUT).unwrap_or(Ok(Value::Null)).map(|_| ()),
            egui::Event::Paste(text) if text.len() > MAX_PASTE_BYTES => Err(format!("Paste: the clipboard holds more than {MAX_PASTE_BYTES} bytes")),
            egui::Event::Paste(text) if holds_glyphs(&text) => {
                Err("Paste: the clipboard holds copied glyphs, not text; leave text mode to paste them".into())
            }
            egui::Event::Paste(text) => super::text::paste(app, &text),
            _ => Ok(()),
        };
        if let Err(e) = result {
            app.state.status = e;
        }
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
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

    /// A one-page PDF with a 100-point page drawing a 50-point black square.
    fn square_pdf() -> Vec<u8> {
        let content = "0 0 0 rg 0 0 50 50 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.5\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        out
    }

    /// A clipboard holding `pdf` beside whatever text the test sends. A read ends once `delay`
    /// frames have looked for it, as a read on another thread would; `requests` counts the reads
    /// started.
    struct PdfClipboard {
        pdf: Option<Vec<u8>>,
        delay: usize,
        waiting: Option<usize>,
        requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl super::ClipboardFlavours for PdfClipboard {
        fn request_pdf(&mut self, _: usize, _: &egui::Context) {
            if self.waiting.is_none() {
                self.requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.waiting = Some(self.delay);
            }
        }

        fn take_pdf(&mut self) -> Option<super::PdfRead> {
            let left = self.waiting?;
            if left > 0 {
                self.waiting = Some(left - 1);
                return None;
            }
            self.waiting = None;
            Some(Ok(self.pdf.clone()))
        }
    }

    /// `square_app` with a clipboard holding the square PDF, read in `delay` frames; and the
    /// count of reads started.
    fn pdf_app(delay: usize) -> (FontcraftApp, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let mut app = square_app();
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        app.clipboard_flavours = Some(Box::new(PdfClipboard { pdf: Some(square_pdf()), delay, waiting: None, requests: requests.clone() }));
        (app, requests)
    }

    fn v(pressed: bool, modifiers: egui::Modifiers) -> Event {
        Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers }
    }

    /// A frame at input time `time` with `modifiers` held (from this frame on).
    fn frame_at(ctx: &egui::Context, app: &mut FontcraftApp, events: Vec<Event>, time: f64, modifiers: egui::Modifiers) {
        let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            events,
            time: Some(time),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.ui(ui));
    }

    fn glyph_shapes(app: &FontcraftApp, glyph: &str) -> usize {
        app.session.font().unwrap().glyph(glyph).unwrap().layers.first().unwrap().shapes.len()
    }

    #[test]
    fn edit_view_pastes_the_clipboards_pdf_when_no_readable_text_comes() {
        use std::sync::atomic::Ordering;

        let (mut app, requests) = pdf_app(0);
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, Vec::new());
        let key = |pressed| v(pressed, egui::Modifiers::COMMAND);

        // The paste key with no text on the clipboard: the platform sends only the key's release.
        // The read ends off the window's thread, and a later frame pastes.
        frame(&ctx, &mut app, vec![key(false)]);
        assert_eq!(shapes(&app), 1, "nothing is pasted before the read ends");
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(shapes(&app), 2, "the PDF's square was pasted");
        let pasted = app.session.font().unwrap().glyph("A").unwrap().layers.first().unwrap().shapes.last().cloned();
        let mut direct = square_app();
        direct.run("edit.paste", &json!({"glyph": "A", "pdf": base64::engine::general_purpose::STANDARD.encode(square_pdf())})).unwrap();
        assert_eq!(
            pasted,
            direct.session.font().unwrap().glyph("A").unwrap().layers.first().unwrap().shapes.last().cloned(),
            "as edit.paste with the PDF"
        );

        // Text that is neither our fragment nor SVG gives way to the PDF; the key's release
        // pastes nothing more.
        frame(&ctx, &mut app, vec![Event::Paste("a caption".into()), key(false)]);
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(shapes(&app), 3);

        // Edit › Paste (the key as a plain key press) asks for text; none comes, so the PDF is
        // read in the next frame and pasted in the one after, and the key's release pastes
        // nothing more.
        frame(&ctx, &mut app, vec![key(true)]);
        frame(&ctx, &mut app, Vec::new());
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(shapes(&app), 4);
        frame(&ctx, &mut app, vec![key(false)]);
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(shapes(&app), 4, "one paste per key stroke");
        assert_eq!(requests.load(Ordering::Relaxed), 3);

        // Our own fragment wins without reading the PDF, and Paste Special never reads it.
        let copied = app.run("edit.copy", &json!({"glyph": "A"})).unwrap()["json"].as_str().unwrap().to_string();
        frame(&ctx, &mut app, vec![Event::Paste(copied)]);
        frame_at(&ctx, &mut app, vec![Event::Paste("a caption".into()), v(false, egui::Modifiers::NONE)], 100.0, egui::Modifiers::ALT);
        frame_at(&ctx, &mut app, vec![v(false, egui::Modifiers::NONE)], 100.1, egui::Modifiers::ALT);
        assert_eq!(requests.load(Ordering::Relaxed), 3, "no PDF read for the fragment or with Option held");
    }

    #[test]
    fn a_slow_clipboard_read_pastes_later_into_the_glyph_edited_at_the_stroke() {
        use std::sync::atomic::Ordering;

        let (mut app, requests) = pdf_app(3);
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, Vec::new());
        frame(&ctx, &mut app, vec![v(false, egui::Modifiers::COMMAND)]);
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        // The person moves on to B while the clipboard is read.
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["B"]})).unwrap();
        let b = glyph_shapes(&app, "B");
        for _ in 0..3 {
            frame(&ctx, &mut app, Vec::new());
            assert_eq!(glyph_shapes(&app, "A"), 1, "the read has not ended");
        }
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(glyph_shapes(&app, "A"), 2, "pasted into A, edited when the key was pressed");
        assert_eq!(glyph_shapes(&app, "B"), b);
        assert_eq!(requests.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn one_stroke_pastes_once_however_long_the_key_is_held() {
        use std::sync::atomic::Ordering;

        let (mut app, requests) = pdf_app(2);
        let ctx = egui::Context::default();
        frame_at(&ctx, &mut app, Vec::new(), 0.0, egui::Modifiers::NONE);
        // Text that gives way to the PDF, the key let go five seconds later.
        frame_at(&ctx, &mut app, vec![Event::Paste("a caption".into())], 0.1, egui::Modifiers::COMMAND);
        for i in 0..5 {
            frame_at(&ctx, &mut app, Vec::new(), 1.0 + f64::from(i), egui::Modifiers::COMMAND);
        }
        frame_at(&ctx, &mut app, vec![v(false, egui::Modifiers::COMMAND)], 6.0, egui::Modifiers::COMMAND);
        for i in 0..4 {
            frame_at(&ctx, &mut app, Vec::new(), 7.0 + f64::from(i), egui::Modifiers::NONE);
        }
        assert_eq!(requests.load(Ordering::Relaxed), 1, "the release does not read again");
        assert_eq!(shapes(&app), 2, "one paste");
        // The next stroke reads again.
        frame_at(&ctx, &mut app, vec![v(false, egui::Modifiers::COMMAND)], 12.0, egui::Modifiers::COMMAND);
        assert_eq!(requests.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn the_paste_key_counts_when_the_command_key_is_let_go_first() {
        use std::sync::atomic::Ordering;

        let (mut app, requests) = pdf_app(0);
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, Vec::new());
        // The platform swallowed the paste key's press (no text), and the command key was let go
        // before V.
        frame(&ctx, &mut app, vec![v(false, egui::Modifiers::NONE)]);
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        assert_eq!(shapes(&app), 2);
        // A plain V (a tool's key) is no paste.
        frame(&ctx, &mut app, vec![v(true, egui::Modifiers::NONE)]);
        frame(&ctx, &mut app, vec![v(false, egui::Modifiers::NONE)]);
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        assert_eq!(shapes(&app), 2);
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
