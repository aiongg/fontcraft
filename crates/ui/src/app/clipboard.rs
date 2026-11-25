//! The system clipboard, app side.
//!
//! - Copy and Cut on the canvas run `edit.copy` and `edit.cut` on the selection and put the `json` text of the result
//!   on the clipboard; our own fragment pastes back without loss, and `edit.paste` also reads the SVG other apps put
//!   there.
//! - In Font View, Copy runs `edit.copy` with the selected glyphs and Font View's master and puts the copied glyphs
//!   (the `fontcraft-glyphs` text) on the clipboard; Paste runs `edit.paste` with that text, which adds the glyphs (a
//!   taken name gets the first free `.001` suffix), in this document or another. Text that holds no copied glyphs is
//!   not pasted in Font View.
//! - Copy Glyph Names (the glyph menus) puts the `text` of `glyph.copyGlyphNames` on the clipboard.
//! - Edit › Paste and Paste Special fired without text wait for the clipboard's text ([`Clipboard::request`]). On the
//!   desktop the shell reads it from the system clipboard at once ([`read_waiting`], through
//!   `FontCraftApp::clipboard_reader`) and calls [`paste`]; without a reader (tests, the browser) it asks the window for
//!   a paste event and takes the text when it arrives. Paste Special opens `dialog.pasteSpecial` on
//!   copied glyphs, with the selected glyphs as its `targets` and Font View's master, or on the canvas the edited glyph
//!   and the master of the layer it shows.
//! - Drawing apps put a PDF on the clipboard, which the text reader does not carry. On the canvas the desktop app's
//!   [`ClipboardFlavours`] (`FontCraftApp::clipboard_flavours`) reads it off the window's thread: Paste whose text is
//!   neither our fragment nor SVG, or that finds no text, starts the read with the edited glyph's targets and document
//!   as they are then ([`read_waiting`]). When the read ends, a later frame runs `edit.paste` with the PDF as base64 on
//!   those targets, or with the text when the clipboard holds no PDF ([`finish_pdf`]). Paste Special and Font View
//!   never read the PDF; without a reader (tests, the browser) only text is pasted.
//! - Font View's master is the current master (`view.selectMaster`). A drawing pastes into the layer the canvas shows
//!   (the Layers panel's pick), as the tools edit it.
//! - Only a person's copy reaches the system clipboard: a copy the control channel names (`engine.execute`) returns
//!   its text in the reply and leaves the clipboard alone.
//!
//! - In Font Info's Features sidebar, Copy runs `font.features.copyEntries` on the selected entries and puts its `text`
//!   on the clipboard; Paste (`font.info.features.paste`) asks for the clipboard's text and runs
//!   `font.features.pasteEntries` with it. Copy and Paste fired while one of the sidebar's lists holds the keyboard
//!   focus (Cmd+C, Cmd+V) go to that list, which posts the same commands ([`to_focused_list`]).
//! - In the Project panel, Copy and Paste (`window.project.copy`, `window.project.paste`) copy and duplicate instances;
//!   Paste asks for the clipboard's text and hands it back to `window.project.paste`. Copy and Paste fired while the
//!   instance list holds the keyboard focus go to that list, which posts the same commands.
//!
//! The text goes out through [`Clipboard::out`], which the shell hands to the window's clipboard.
//!
//! Why paste reads the clipboard itself: craft hands the platform's paste event only to a focused component that lists
//! Paste (a text field, a list taking edit commands, a canvas whose key use lists it). Anywhere else the event stands for
//! the Cmd+V chord, which fires `edit.paste`, and its text is dropped, so a paste event the app asks for never reaches
//! it. craft's own read path, `craft::platform::clipboard::Clipboard`, reads the text instead. The full version in craft
//! would hand the dropped text to the app with the fired command (a `paste` on the fired command or on `FrameOutput`),
//! which also covers the browser build, where the clipboard can only be read from a paste event.

use base64::Engine as _;
/// Wakes the window when a clipboard read that runs off its thread ends.
pub use craft::host::Wake;
use craft::ui::runtime::commands::CommandRef;
use craft_ext::ui::list::{EditCommand, deliver};
use serde_json::{Map, Value, json};

use super::{FontCraftApp, Msg};

const COPY: &str = "edit.copy";
const CUT: &str = "edit.cut";
const PASTE: &str = "edit.paste";
const PASTE_SPECIAL: &str = "edit.pasteSpecial";
const SELECT_ALL: &str = "edit.selectAll";
/// The commands whose shortcut is Cmd+Delete.
const REMOVE_GLYPH: &str = "glyph.removeGlyph";
const REMOVE_FIELD: &str = "font.info.removeField";
/// The `format` the engine writes into copied glyphs (`edit.copy` with `glyphs`).
const GLYPHS_FORMAT: &str = "fontcraft-glyphs";
/// The `format` of copied corner components (`edit.copy` with `corners`).
const CORNERS_FORMAT: &str = "fontcraft-corners";
/// The `format` of copied caps, segment components and brushes (`edit.copy` with `caps`, `segmentComponents` or
/// `brushes`).
const ATTACHMENTS_FORMAT: &str = "fontcraft-attachments";
/// Longest clipboard text a paste takes.
pub const MAX_PASTE_BYTES: usize = 16 * 1024 * 1024;

/// Largest PDF a paste takes from the clipboard: `edit.paste`'s limit.
pub const MAX_PASTE_PDF_BYTES: usize = 64 << 20;
/// How often a frame looks for the end of a clipboard read, should the reader not wake the window.
pub const READ_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Reads the system clipboard's text: `Ok(None)` when it holds none.
pub type ReadClipboardFn = fn() -> Result<Option<String>, String>;

/// What reading the clipboard's PDF gave: the PDF, `None` when the clipboard holds none, or why it could not be read
/// (data larger than the limit included).
pub type PdfRead = Result<Option<Vec<u8>>, String>;

/// Clipboard data the text reader does not carry. The desktop app gives `FontCraftApp::clipboard_flavours` one;
/// without it (the browser, tests) only text is pasted.
pub trait ClipboardFlavours: Send {
    /// Starts reading the PDF on the system clipboard, refusing more than `max_bytes`, unless a read is running
    /// already. The read must not hold up the caller: the desktop app runs it on a thread of its own and calls `wake`
    /// when it ends.
    fn request_pdf(&mut self, max_bytes: usize, wake: Wake);
    /// The result of the read started last, once it has ended; `None` while it runs, and after its result was taken.
    fn take_pdf(&mut self) -> Option<PdfRead>;
}

/// A clipboard read under way: `edit.paste` with the targets of the moment it started, the document they are in, and
/// the text to paste when the clipboard holds no PDF.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingPdf {
    pub command: CommandRef,
    pub document: Option<u64>,
    pub text: Option<String>,
}

/// The system clipboard's text, through craft's clipboard (size limits included). Not in the browser build, whose
/// clipboard is read from paste events only.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_text() -> Result<Option<String>, String> {
    use craft::platform::clipboard::{Clipboard as SystemReader, SystemClipboard};
    SystemReader::new(SystemClipboard::new()).read_text().map_err(|e| e.to_string())
}

/// Which paste waits for the clipboard's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteKind {
    Paste,
    Special,
    /// Feature entries into the Features sidebar.
    FeatureEntries,
    /// Instances copied in the Project panel, into the project with this id (`window.project.paste`).
    ProjectInstances(u64),
}

/// The clipboard's state on the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clipboard {
    /// Text for the system clipboard, put there by the shell in the next pass.
    pub out: Option<String>,
    /// A paste waits for the clipboard's text.
    pub request: Option<PasteKind>,
    /// The pass the window was asked for the clipboard's text in.
    pub asked: Option<u64>,
    /// The clipboard's PDF is being read for this paste.
    pub pending: Option<PendingPdf>,
}

/// After a run of `id`: Copy and Cut put their `json` text on the clipboard, Copy Glyph Names its `text`.
pub fn after_run(app: &mut FontCraftApp, id: &str, result: &Value) {
    let key = match id {
        COPY | CUT => "json",
        "glyph.copyGlyphNames" | "font.features.copyEntries" => "text",
        _ => return,
    };
    if let Some(text) = result.get(key).and_then(Value::as_str) {
        app.clipboard.out = Some(text.to_string());
    }
}

fn is_empty(params: &Value) -> bool {
    params.is_null() || params.as_object().is_some_and(Map::is_empty)
}

/// A clipboard command the person fired (menu item, shortcut), after its targets were filled. Returns the command to
/// run, or `None` when it waits for the clipboard's text: Paste and Paste Special with nothing to paste ask the window
/// for it. Copy and Cut in Font View copy the selected glyphs.
pub fn fired(app: &mut FontCraftApp, command: CommandRef) -> Option<CommandRef> {
    match command.id.as_str() {
        PASTE | PASTE_SPECIAL if ["json", "svg", "pdf", "text"].iter().all(|k| command.params.get(k).is_none_or(Value::is_null)) => {
            app.clipboard.request = Some(if command.id == PASTE { PasteKind::Paste } else { PasteKind::Special });
            None
        }
        COPY if is_empty(&command.params) && in_font_view(app) => {
            let mut m = Map::new();
            m.insert("glyphs".into(), json!(app.view.font_view.selected));
            if let Some(master) = font_view_master(app) {
                m.insert("master".into(), json!(master));
            }
            Some(CommandRef::with_params(COPY, Value::Object(m)))
        }
        _ => Some(command),
    }
}

/// The edit command a fired command stands for while a list holds the keyboard focus: the Edit menu's, and with
/// `by_key` the two commands Cmd+Delete fires.
fn edit_command(id: &str, by_key: bool) -> Option<EditCommand> {
    match id {
        REMOVE_GLYPH | REMOVE_FIELD if by_key => Some(EditCommand::Delete),
        COPY => Some(EditCommand::Copy),
        CUT => Some(EditCommand::Cut),
        PASTE => Some(EditCommand::Paste),
        SELECT_ALL => Some(EditCommand::SelectAll),
        _ => None,
    }
}

/// Hands an Edit command the person fired with no parameters to the list that holds the keyboard focus and takes it
/// (the Features sidebar and the Project panel's instance list take Copy and Paste, the Masters and Exports lists of
/// Font Info take Cmd+Delete); `true` when the list took it, and the document is left alone. `by_key`: a key fired it,
/// so a click on Glyph › Remove Glyph never reaches a list.
pub fn to_focused_list(app: &FontCraftApp, command: &CommandRef, by_key: bool) -> bool {
    let Some(edit) = edit_command(&command.id, by_key) else { return false };
    is_empty(&command.params) && app.window.as_ref().is_some_and(|ctx| deliver(ctx, edit))
}

/// Font View shows with glyphs selected: copying and pasting act on whole glyphs.
fn in_font_view(app: &FontCraftApp) -> bool {
    !app.view.font_view.selected.is_empty() && app.font_view_shows()
}

/// Font View's master: the current master (`view.selectMaster`).
fn font_view_master(app: &FontCraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    crate::shell::masters::current_master(font, &app.view.layers).map(|m| m.id.clone())
}

/// The master of the layer the canvas shows (the Layers panel's pick), else the current master.
fn canvas_master(app: &FontCraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    crate::canvas::tools::active_master(font, &app.view).map(|m| m.id.clone())
}

/// Whether clipboard text holds copied glyphs rather than a drawing.
fn holds_glyphs(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains(GLYPHS_FORMAT)
}

/// Whether clipboard text holds copied corner components, or copied caps, segment components or brushes, which
/// `edit.paste` puts onto the nodes in `points`.
fn holds_corners(text: &str) -> bool {
    text.trim_start().starts_with('{') && (text.contains(CORNERS_FORMAT) || text.contains(ATTACHMENTS_FORMAT))
}

/// The message that pastes `text` as `kind` asks: `edit.paste` into the edited glyph (on the canvas) or the copied
/// glyphs into the font (in Font View), or Paste Special's dialog. Errors when there is nothing to paste, or no glyph
/// to paste a drawing into ([`FontCraftApp::targets_chosen`]).
pub fn paste_msg(app: &mut FontCraftApp, text: &str, kind: PasteKind) -> Result<Msg, String> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(format!("Paste: the clipboard's text is larger than {} MiB", MAX_PASTE_BYTES >> 20));
    }
    if text.trim().is_empty() {
        return Err("Paste: the clipboard holds no text".into());
    }
    if kind == PasteKind::FeatureEntries {
        return Ok(Msg::run("font.features.pasteEntries", json!({ "text": text })));
    }
    if let PasteKind::ProjectInstances(project) = kind {
        let params = json!({ "project": project, "text": text });
        return Ok(Msg::Shell(CommandRef::with_params(crate::shell::project::PASTE, params)));
    }
    let glyphs = holds_glyphs(text);
    if kind == PasteKind::Special {
        if !glyphs {
            return Err("Paste Special: the clipboard holds no copied glyphs".into());
        }
        // With no glyph chosen the copied glyphs go over the glyphs of their names, not over the font's first glyph.
        let targets: Vec<String> = if in_font_view(app) {
            app.view.font_view.selected.clone()
        } else if app.targets_chosen() {
            crate::canvas::tools::target(&app.session, &app.view).map(|t| t.glyph).into_iter().collect()
        } else {
            Vec::new()
        };
        let mut params = Map::new();
        params.insert("text".into(), json!(text));
        if !targets.is_empty() {
            params.insert("targets".into(), json!(targets));
        }
        let master = if !in_font_view(app) && app.targets_chosen() { canvas_master(app) } else { font_view_master(app) };
        if let Some(master) = master {
            params.insert("master".into(), json!(master));
        }
        return Ok(Msg::Shell(CommandRef::with_params("dialog.open", json!({"dialog": "dialog.pasteSpecial", "params": params}))));
    }
    if app.font_view_shows() && glyphs {
        return Ok(Msg::run(PASTE, json!({ "text": text })));
    }
    if app.font_view_shows() && in_font_view(app) {
        return Err("Paste: the clipboard holds no copied glyphs to add to the font".into());
    }
    if !app.targets_chosen() {
        return Err(super::select_glyph_first(PASTE));
    }
    let mut command = crate::canvas::tools::targets::with_targets(app, CommandRef::new(PASTE));
    let mut params = match command.params {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let points = &app.tools.selection.points;
    if holds_corners(text) && !points.is_empty() {
        params.insert("points".into(), json!(points.iter().map(|&(s, n)| [s, n]).collect::<Vec<_>>()));
    }
    params.insert("text".into(), json!(text));
    command.params = Value::Object(params);
    Ok(Msg::Run(command))
}

/// Takes the clipboard's text for the paste that waits for it from `FontCraftApp::clipboard_reader` and pastes it.
/// On the canvas, text that is neither our fragment nor SVG, or no text, starts reading the clipboard's PDF instead
/// ([`start_pdf`]). `false` when no paste waits or the app has no reader: the shell then asks the window for a paste
/// event.
pub fn read_waiting(app: &mut FontCraftApp) -> bool {
    let (Some(kind), Some(read)) = (app.clipboard.request, app.clipboard_reader) else { return false };
    app.clipboard.request = None;
    app.clipboard.asked = None;
    let read = read();
    if kind == PasteKind::Paste && reads_pdf(app) {
        match &read {
            Ok(Some(text)) if !pasteable(text) && text.len() <= MAX_PASTE_BYTES => {
                start_pdf(app, Some(text.clone()));
                return true;
            }
            Ok(None) => {
                start_pdf(app, None);
                return true;
            }
            _ => {}
        }
    }
    match read {
        Ok(Some(text)) => paste(app, &text, kind),
        Ok(None) => app.status = "Paste: the clipboard holds no text".into(),
        Err(e) => app.status = format!("Paste: {e}"),
    }
    true
}

/// Whether `edit.paste` can read clipboard text: JSON (our fragment or copied glyphs) or an SVG document. Other text
/// gives way to the clipboard's PDF.
fn pasteable(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with('{') || (t.starts_with('<') && t.contains("<svg"))
}

/// Whether a paste may read the clipboard's PDF: the app has a reader, and the paste goes into a glyph on the canvas.
fn reads_pdf(app: &FontCraftApp) -> bool {
    app.clipboard_flavours.is_some() && !app.font_view_shows() && app.targets_chosen()
}

/// Starts reading the clipboard's PDF for a paste into the edited glyph, with `text` to paste should the clipboard
/// hold no PDF. While another read runs, this paste is dropped with a message.
fn start_pdf(app: &mut FontCraftApp, text: Option<String>) {
    if app.clipboard.pending.is_some() {
        app.status = "Paste: the clipboard is still being read for the last paste".into();
        return;
    }
    let command = crate::canvas::tools::targets::with_targets(app, CommandRef::new(PASTE));
    let document = app.session.active().map(|d| d.id);
    let wake = match &app.window {
        Some(ctx) => {
            let ctx = ctx.clone();
            Wake::new(move || ctx.request_repaint())
        }
        None => Wake::none(),
    };
    let Some(reader) = app.clipboard_flavours.as_mut() else { return };
    reader.request_pdf(MAX_PASTE_PDF_BYTES, wake);
    app.clipboard.pending = Some(PendingPdf { command, document, text });
}

/// Once the running clipboard read has ended, posts `edit.paste` with the PDF on the targets taken when the read
/// started, or with the text when the clipboard holds no PDF. It waits while a dialog is open. A PDF that cannot be
/// read, or a document closed or switched meanwhile, leaves a message in the status line. Returns whether a read still
/// runs, so the window looks again after [`READ_POLL`].
pub fn finish_pdf(app: &mut FontCraftApp) -> bool {
    if app.clipboard.pending.is_none() {
        return false;
    }
    if app.dialogs.open.is_some() {
        return true;
    }
    let read = match app.clipboard_flavours.as_mut() {
        Some(reader) => match reader.take_pdf() {
            Some(read) => read,
            None => return true,
        },
        None => Ok(None),
    };
    let Some(PendingPdf { mut command, document, text }) = app.clipboard.pending.take() else { return false };
    let (key, value) = match read {
        Ok(Some(bytes)) if bytes.len() > MAX_PASTE_PDF_BYTES => {
            app.status = format!("Paste: the clipboard's PDF is larger than {} MiB", MAX_PASTE_PDF_BYTES >> 20);
            return false;
        }
        Ok(Some(bytes)) => ("pdf", Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))),
        Ok(None) => match text {
            Some(text) => ("text", Value::String(text)),
            None => {
                app.status = "Paste: the clipboard holds no text".into();
                return false;
            }
        },
        // The text, if any, is neither our fragment nor SVG, so the reason the PDF failed is the one worth showing.
        Err(e) => {
            app.status = format!("Paste: the clipboard's PDF cannot be read: {e}");
            return false;
        }
    };
    if app.session.active().map(|d| d.id) != document {
        app.status = "Paste: the document changed while the clipboard was read; nothing was pasted".into();
        return false;
    }
    let mut params = match command.params {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    params.insert(key.into(), value);
    command.params = Value::Object(params);
    app.sent.push(Msg::Run(command));
    false
}

/// Pastes `text` as `kind` asks, after what views posted: the shell calls it when the clipboard's text arrives.
pub fn paste(app: &mut FontCraftApp, text: &str, kind: PasteKind) {
    match paste_msg(app, text, kind) {
        Ok(msg) => app.sent.push(msg),
        Err(e) => app.status = e,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::{ClipboardFlavours, PasteKind, PdfRead, ReadClipboardFn, Wake, finish_pdf, read_waiting};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

    fn copied() -> Result<Option<String>, String> {
        Ok(Some("feature liga { sub f i by f_i; } liga;".to_string()))
    }

    fn empty() -> Result<Option<String>, String> {
        Ok(None)
    }

    /// With a reader, a waiting paste takes the system clipboard's text at once, without a paste event; without one
    /// it keeps waiting for the window's event; an empty clipboard says so in the status line.
    #[test]
    fn a_waiting_paste_reads_the_system_clipboard() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.clipboard.request = Some(PasteKind::FeatureEntries);
        assert!(!read_waiting(&mut app), "no reader: the paste waits for the window's event");
        assert_eq!(app.clipboard.request, Some(PasteKind::FeatureEntries));

        app.clipboard_reader = Some(copied);
        assert!(read_waiting(&mut app));
        assert_eq!(app.clipboard.request, None);
        assert_eq!(app.sent, vec![Msg::run("font.features.pasteEntries", json!({ "text": "feature liga { sub f i by f_i; } liga;" }))]);

        app.sent.clear();
        app.clipboard_reader = Some(empty);
        app.clipboard.request = Some(PasteKind::Paste);
        assert!(read_waiting(&mut app));
        assert!(app.sent.is_empty());
        assert_eq!(app.status, "Paste: the clipboard holds no text");
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

    /// A clipboard holding `pdf` beside the text of the app's reader. A read ends once `delay` calls have looked for
    /// it, as a read on another thread would; `requests` counts the reads started.
    struct PdfClipboard {
        pdf: Option<Vec<u8>>,
        delay: usize,
        waiting: Option<usize>,
        requests: Arc<AtomicUsize>,
    }

    impl ClipboardFlavours for PdfClipboard {
        fn request_pdf(&mut self, _: usize, _: Wake) {
            if self.waiting.is_none() {
                self.requests.fetch_add(1, Ordering::Relaxed);
                self.waiting = Some(self.delay);
            }
        }

        fn take_pdf(&mut self) -> Option<PdfRead> {
            let left = self.waiting?;
            if left > 0 {
                self.waiting = Some(left - 1);
                return None;
            }
            self.waiting = None;
            Some(Ok(self.pdf.clone()))
        }
    }

    fn caption() -> Result<Option<String>, String> {
        Ok(Some("a caption".to_string()))
    }

    fn svg() -> Result<Option<String>, String> {
        Ok(Some(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="0" y="0" width="10" height="10"/></svg>"#.to_string()))
    }

    /// An Edit View tab on glyph A over a clipboard whose text `text` gives and whose PDF (`pdf`) is read in `delay`
    /// calls; and the count of reads started.
    fn pdf_app(text: ReadClipboardFn, pdf: Option<Vec<u8>>, delay: usize) -> (FontCraftApp, Arc<AtomicUsize>) {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        let requests = Arc::new(AtomicUsize::new(0));
        app.clipboard_reader = Some(text);
        app.clipboard_flavours = Some(Box::new(PdfClipboard { pdf, delay, waiting: None, requests: Arc::clone(&requests) }));
        (app, requests)
    }

    /// Edit › Paste or the paste key: the paste waits for the clipboard and the shell reads it.
    fn paste(app: &mut FontCraftApp, kind: PasteKind) {
        app.clipboard.request = Some(kind);
        assert!(read_waiting(app));
    }

    /// The `edit.paste` posted last and its `key` parameter.
    fn posted(app: &FontCraftApp, key: &str) -> Option<serde_json::Value> {
        app.sent.iter().rev().find_map(|m| match m {
            Msg::Run(c) if c.id == "edit.paste" => c.params.get(key).cloned(),
            _ => None,
        })
    }

    /// On the canvas, text that is neither our fragment nor SVG, or no text, gives way to the clipboard's PDF, which a
    /// later frame pastes as `pdf`; SVG pastes as text without a read, Paste Special and Font View never read it,
    /// and with no PDF on the clipboard the text is pasted.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn the_canvas_pastes_the_clipboards_pdf_when_no_readable_text_comes() {
        use base64::Engine as _;

        let (mut app, requests) = pdf_app(caption, Some(square_pdf()), 0);
        paste(&mut app, PasteKind::Paste);
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        assert!(app.sent.is_empty(), "nothing is pasted before the read ends");
        assert!(!finish_pdf(&mut app), "the read ended");
        assert_eq!(posted(&app, "pdf"), Some(json!(base64::engine::general_purpose::STANDARD.encode(square_pdf()))));
        let glyph = posted(&app, "glyph").and_then(|g| g.as_str().map(str::to_string)).unwrap();
        let shapes = |app: &FontCraftApp| app.session.font().unwrap().glyph(&glyph).unwrap().layers.first().unwrap().shapes.len();
        let before = shapes(&app);
        app.drain();
        assert_eq!(shapes(&app), before + 1, "the PDF's square was pasted");

        // No text on the clipboard reads the PDF too.
        app.clipboard_reader = Some(empty);
        paste(&mut app, PasteKind::Paste);
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        assert!(!finish_pdf(&mut app));

        // SVG pastes as it is; Paste Special never reads the PDF.
        app.sent.clear();
        app.clipboard_reader = Some(svg);
        paste(&mut app, PasteKind::Paste);
        assert!(posted(&app, "text").is_some());
        app.clipboard_reader = Some(caption);
        paste(&mut app, PasteKind::Special);
        assert_eq!(requests.load(Ordering::Relaxed), 2, "no PDF read for SVG or Paste Special");

        // With no PDF on the clipboard the text is pasted.
        let (mut app, _) = pdf_app(caption, None, 0);
        paste(&mut app, PasteKind::Paste);
        assert!(!finish_pdf(&mut app));
        assert_eq!(posted(&app, "text"), Some(json!("a caption")));

        // Font View takes no drawing, from the PDF or else.
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let requests = Arc::new(AtomicUsize::new(0));
        app.clipboard_reader = Some(caption);
        app.clipboard_flavours = Some(Box::new(PdfClipboard { pdf: Some(square_pdf()), delay: 0, waiting: None, requests: Arc::clone(&requests) }));
        paste(&mut app, PasteKind::Paste);
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        assert!(app.clipboard.pending.is_none());
    }

    /// A slow read pastes later, into the targets taken when it started; a paste made meanwhile is dropped with a
    /// message, and a document switched meanwhile pastes nothing.
    #[test]
    #[allow(clippy::unwrap_used, clippy::panic)]
    fn a_slow_clipboard_read_pastes_later_into_the_targets_it_started_with() {
        let (mut app, requests) = pdf_app(caption, Some(square_pdf()), 2);
        paste(&mut app, PasteKind::Paste);
        let started = app.clipboard.pending.clone().unwrap().command;
        assert!(finish_pdf(&mut app), "the read runs");
        paste(&mut app, PasteKind::Paste);
        assert_eq!(app.status, "Paste: the clipboard is still being read for the last paste");
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        assert!(finish_pdf(&mut app));
        assert!(!finish_pdf(&mut app));
        match app.sent.last() {
            Some(Msg::Run(c)) => {
                assert_eq!(c.id, "edit.paste");
                assert_eq!(c.params.get("glyph"), started.params.get("glyph"));
                assert!(c.params.get("pdf").is_some());
            }
            other => panic!("expected edit.paste, got {other:?}"),
        }

        let (mut app, _) = pdf_app(caption, Some(square_pdf()), 1);
        paste(&mut app, PasteKind::Paste);
        app.run_now(Msg::run("file.new", json!({})), false).unwrap();
        assert!(finish_pdf(&mut app));
        assert!(!finish_pdf(&mut app));
        assert!(posted(&app, "pdf").is_none());
        assert_eq!(app.status, "Paste: the document changed while the clipboard was read; nothing was pasted");
    }
}
