//! The web preview address (`hint.tt.showPreviewAddress`, Show Preview Address… of the TrueType Instructor): where a
//! browser, on this computer or another device of the local network, opens the hinted preview while the dialog is open.
//!
//! Rows: `dialog.ttPreviewAddress`, `hint.tt.showPreviewAddress`.
//!
//! The command builds the font as File › Export writes it, as TrueType, and a page that sets the edit view's text at
//! every pixel size from [`MIN_SIZE`] to [`MAX_SIZE`], as the Hinting Preview does. The app's
//! [`PreviewHost`](crate::app::preview_host::PreviewHost) serves both; the desktop app supplies one, the browser build
//! has none and the command is refused there. The dialog shows the address with Copy, who can reach it, Only this
//! computer and Update. Closing it stops the server.
//!
//! The page is plain HTML and CSS: a browser that applies TrueType hinting draws it with the font's instructions,
//! which is what the page is for. It holds no script.

use std::fmt::Write as _;

use base64::Engine as _;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Ui;
use fontcraft_textlayout::Item;
use serde_json::{Map, Value, json};

use crate::app::preview_host::{FONT_FILE, Scope, Site};
use crate::app::{FontCraftApp, Msg};

pub const ID: &str = "hint.tt.showPreviewAddress";
/// The smallest and the largest pixel size on the page, as in the Hinting Preview.
pub const MIN_SIZE: u32 = 8;
pub const MAX_SIZE: u32 = 80;
/// Most characters of sample text on the page (each is set once per size).
pub const MAX_TEXT_CHARS: usize = 500;
/// The family name the page gives the served font in its `@font-face` rule.
const PAGE_FAMILY: &str = "FontCraft Hinting Preview";

pub const PARAMS: &str = "{close?: bool (true closes the dialog and stops the server), text?: string (the sample text, at most 500 characters; default the edit view's glyphs, by their characters, leaving out glyphs without one), instance?: string (a static instance by name, `#n` for an unnamed one; default the first exported static instance, else the first master), thisComputerOnly?: bool (true serves this computer only; default the local network)} builds the instance as TrueType as File › Export would, starts a web server for a page that sets the text at every pixel size from 8 to 80 in that font, and opens the Preview Address dialog with the page's address. Run again while the dialog is open, it serves the font as it is now at the same address. Desktop app only";

craft::ui::words! {
    /// The words of the Preview Address dialog.
    pub struct PreviewAddressWords {
        title: "fontcraftPreviewAddress.title" = "Preview Address";
        intro: "fontcraftPreviewAddress.intro" = "Open this address in a web browser to see the hinted preview:";
        copy: "fontcraftPreviewAddress.copy" = "Copy";
        network: "fontcraftPreviewAddress.network" = "While this window is open, any device on the local network that has the address can load the font.";
        this_computer: "fontcraftPreviewAddress.thisComputer" = "Only browsers on this computer can open the address.";
        only_here: "fontcraftPreviewAddress.onlyHere" = "Only this computer";
        update: "fontcraftPreviewAddress.update" = "Update";
        update_tip: "fontcraftPreviewAddress.updateTip" = "Serve the font as it is now; reload the page to see it";
        done: "fontcraftPreviewAddress.done" = "Done";
    }
}

/// The dialog's app state, in `TtState::preview_address`. Never saved: a server never outlives the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewAddressState {
    pub open: bool,
    pub address: Option<String>,
    pub scope: Scope,
    /// The instance served and the characters shown.
    pub summary: String,
    /// The `text` and `instance` the dialog was opened with, so its controls rebuild the same page.
    pub given: Map<String, Value>,
    /// Moves with every serve, so an open dialog shows the new address and summary.
    pub generation: u64,
}

/// Runs `hint.tt.showPreviewAddress`.
pub fn command(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let flag = |key: &str| -> Result<bool, String> {
        match params.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(v) => v.as_bool().ok_or_else(|| format!("`{key}` must be true or false")),
        }
    };
    let text_of = |key: &str| -> Result<Option<String>, String> {
        match params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_str().map(|s| Some(s.to_string())).ok_or_else(|| format!("`{key}` must be text")),
        }
    };
    if flag("close")? {
        close(app);
        return Ok(());
    }
    let scope = if flag("thisComputerOnly")? { Scope::ThisComputer } else { Scope::LocalNetwork };
    let given_text = text_of("text")?;
    let instance = text_of("instance")?;
    if app.preview_host.is_none() {
        return Err("the web preview needs the desktop app: no web server can be started here".into());
    }
    let (text, left_out) = sample_text(app, given_text.as_deref())?;
    let built = build_font(app, instance.as_deref())?;
    let title = app.session.active().map(|d| d.title.clone()).unwrap_or_default();
    let site = Site { page: page(&title, &built.instance, &text), font: built.font };
    let host = app.preview_host.as_mut().ok_or("no web server")?;
    let address = host.serve(site, scope).map_err(|e| format!("the web server could not start: {e}"))?;
    let st = &mut app.tt.preview_address;
    st.open = true;
    st.address = Some(address);
    st.scope = scope;
    st.summary = summary(&built.instance, &text, left_out);
    st.given = Map::new();
    if let Some(t) = given_text {
        st.given.insert("text".into(), json!(t));
    }
    if let Some(i) = instance {
        st.given.insert("instance".into(), json!(i));
    }
    st.generation = st.generation.wrapping_add(1);
    if !built.warnings.is_empty() {
        app.status = format!("{} export warning(s) for the preview font", built.warnings.len());
    }
    Ok(())
}

/// Closes the dialog and stops its server.
pub fn close(app: &mut FontCraftApp) {
    if let Some(host) = app.preview_host.as_mut() {
        host.stop();
    }
    let st = &mut app.tt.preview_address;
    st.open = false;
    st.address = None;
    st.summary.clear();
    st.generation = st.generation.wrapping_add(1);
}

fn summary(instance: &str, text: &str, left_out: usize) -> String {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count();
    let mut s = format!("{instance}: {chars} characters at {MIN_SIZE} to {MAX_SIZE} pixels per em.");
    if left_out > 0 {
        let _ = write!(s, " {left_out} glyphs without a character are left out.");
    }
    s
}

/// The sample text: `given`, else the edit view's glyphs by their characters, and how many of its glyphs have no
/// character to show.
fn sample_text(app: &FontCraftApp, given: Option<&str>) -> Result<(String, usize), String> {
    if let Some(t) = given {
        if t.chars().count() > MAX_TEXT_CHARS {
            return Err(format!("`text` may hold at most {MAX_TEXT_CHARS} characters"));
        }
        let text: String = t.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        if text.trim().is_empty() {
            return Err("`text` holds no character to show".into());
        }
        return Ok((text, 0));
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let edit = &app.view.text;
    let encoded = |name: &str| font.glyph(name).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32).filter(|c| !c.is_control());
    let mut text = String::new();
    let mut left_out = 0usize;
    for item in edit.text.items().iter().take(MAX_TEXT_CHARS) {
        let c = match item {
            Item::Glyph { typed: Some(c), .. } if !c.is_control() => Some(*c),
            Item::Glyph { name, .. } => encoded(name),
            Item::Placeholder => edit.current().and_then(encoded),
            Item::Missing(_) => None,
            Item::LineBreak => Some(' '),
        };
        match c {
            Some(c) => text.push(c),
            None => left_out += 1,
        }
    }
    if text.trim().is_empty() {
        return Err("none of the edit view's glyphs has a character to show: give `text`".into());
    }
    Ok((text, left_out))
}

struct BuiltFont {
    font: Vec<u8>,
    instance: String,
    warnings: Vec<Value>,
}

/// The first exported static instance, else the first static instance; `None` when the font has none, so the export
/// builds its first master. Labels as the export names them.
fn default_instance(font: &fontcraft_model::Font) -> Option<String> {
    let statics: Vec<(usize, &fontcraft_model::Instance)> = font.instances.iter().enumerate().filter(|(_, i)| i.kind.is_none()).collect();
    let (i, inst) = statics.iter().find(|(_, inst)| inst.exports).or_else(|| statics.first())?;
    Some(if inst.name.is_empty() { format!("#{i}") } else { inst.name.clone() })
}

/// The instance as File › Export writes it, with TrueType outlines, in memory.
fn build_font(app: &mut FontCraftApp, instance: Option<&str>) -> Result<BuiltFont, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let label = instance.map(str::to_string).or_else(|| default_instance(font));
    let mut params = Map::new();
    params.insert("outlines".into(), json!("truetype"));
    params.insert("containers".into(), json!(["plain"]));
    if let Some(label) = &label {
        params.insert("instances".into(), json!([label]));
    }
    let r = app.run_command("file.exportStatic", Value::Object(params))?;
    let first = r.pointer("/fonts/0").ok_or("the export built no font")?;
    let data = first.pointer("/files/0/dataBase64").and_then(Value::as_str).ok_or("the export kept no font data")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| format!("the exported font cannot be read: {e}"))?;
    let mut warnings: Vec<Value> = r.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default();
    warnings.extend(first.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default());
    let instance = first.get("instance").and_then(Value::as_str).map(str::to_string).or(label).unwrap_or_default();
    Ok(BuiltFont { font: bytes, instance, warnings })
}

/// `s` with the characters HTML gives a meaning written as references.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The preview page: `text` set in the served font once per pixel size, each line labelled with its size. CSS pixels
/// equal device pixels only at 100% zoom and display scaling, which the page says.
pub fn page(document: &str, instance: &str, text: &str) -> String {
    let title = escape(&format!("{document} {instance}").replace(char::is_control, " "));
    let sample = escape(text);
    let mut rows = String::new();
    for size in MIN_SIZE..=MAX_SIZE {
        let _ = writeln!(
            rows,
            "<div class=\"row\"><span class=\"size\">{size}</span><span class=\"sample\" style=\"font-size:{size}px\">{sample}</span></div>"
        );
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{title}: hinting preview</title>\n<style>\n@font-face {{ font-family: \"{PAGE_FAMILY}\"; src: url(\"{FONT_FILE}\") format(\"truetype\"); }}\nbody {{ margin: 16px; background: #fff; color: #000; font: 13px sans-serif; }}\n.row {{ display: flex; align-items: baseline; white-space: nowrap; overflow: hidden; }}\n.size {{ flex: none; width: 2.5em; margin-right: 12px; text-align: right; color: #666; }}\n.sample {{ font-family: \"{PAGE_FAMILY}\"; line-height: 1.25; }}\n.note {{ color: #666; margin-bottom: 16px; }}\n</style>\n</head>\n<body>\n<h1 style=\"font-size: 15px\">{title}</h1>\n<p class=\"note\">Each line is set at the pixel size on its left, in pixels per em. Sizes are exact at 100% browser zoom and 100% display scaling. Reload the page after updating the preview in FontCraft.</p>\n{rows}</body>\n</html>\n"
    )
}

/// What the dialog's controls asked for, taken by [`run`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asked {
    /// Copy the address to the clipboard.
    pub copy: bool,
    /// Serve again, to this computer only (`true`) or the local network.
    pub serve: Option<bool>,
}

/// The Preview Address dialog on the window's host.
pub struct PreviewAddressDialog {
    pub state: PreviewAddressState,
    pub asked: Asked,
    pub closing: bool,
    open: bool,
}

impl PreviewAddressDialog {
    pub fn new(state: PreviewAddressState) -> Self {
        PreviewAddressDialog { state, asked: Asked::default(), closing: false, open: true }
    }
}

/// The dialog's body: the address with Copy, the summary, who can reach it, Only this computer and Update.
pub fn view(ui: &mut Ui, state: &PreviewAddressState, asked: &mut Asked) {
    let words = Env::of(ui).words::<PreviewAddressWords>();
    let _ = Stack::new().show(ui, |ui, _| {
        Text::new(words.intro.clone()).show(ui);
        let _ = Row::new().show(ui, |ui, _| {
            Text::new(state.address.clone().unwrap_or_default()).show(ui);
            if Button::new(words.copy.clone()).show(ui).clicked() {
                asked.copy = true;
            }
        });
        Text::new(state.summary.clone()).shade(Shade::Muted).show(ui);
        let reach = match state.scope {
            Scope::LocalNetwork => words.network.clone(),
            Scope::ThisComputer => words.this_computer.clone(),
        };
        Text::new(reach).shade(Shade::Muted).show(ui);
        let _ = Row::new().show(ui, |ui, _| {
            let here = state.scope == Scope::ThisComputer;
            let only = Checkbox::new(Bind::given(here)).label(words.only_here.clone()).show(ui);
            if let Some(on) = only.events.committed()
                && *on != here
            {
                asked.serve = Some(*on);
            }
            if Button::new(words.update.clone()).tooltip(words.update_tip.clone()).show(ui).clicked() {
                asked.serve = Some(here);
            }
        });
    });
}

impl StackDialog for PreviewAddressDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftTtPreviewAddress")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<PreviewAddressWords>();
        let PreviewAddressDialog { state, asked, open, .. } = self;
        let shown = Dialog::new(words.title.clone())
            .opened(Bind::bound(open))
            .modeless()
            .buttons([FooterButton::new(words.done.clone(), DialogRole::Accept)])
            .show(ui, |ui, _| view(ui, state, asked));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// Opens, refreshes or closes the dialog to match [`PreviewAddressState`], and acts on what its controls asked for.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    // Closed by the person: the server stops with it.
    if host.take::<PreviewAddressDialog>().is_some() && app.tt.preview_address.open {
        app.sent.push(Msg::Shell(CommandRef::with_params(ID, json!({ "close": true }))));
        app.tt.preview_address.open = false;
    }
    let st = &app.tt.preview_address;
    match host.get_mut::<PreviewAddressDialog>() {
        Some(dialog) => {
            if !st.open {
                dialog.closing = true;
            } else if dialog.state.generation != st.generation {
                dialog.state = st.clone();
            }
            let asked = std::mem::take(&mut dialog.asked);
            if asked.copy
                && let Some(address) = &dialog.state.address
            {
                app.clipboard.out = Some(address.clone());
            }
            if let Some(here) = asked.serve {
                let mut params = app.tt.preview_address.given.clone();
                params.insert("thisComputerOnly".into(), json!(here));
                app.sent.push(Msg::Shell(CommandRef::with_params(ID, Value::Object(params))));
            }
        }
        None if st.open => _ = host.open(PreviewAddressDialog::new(st.clone())),
        None => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::sync::{Arc, Mutex};

    use craft::host::Host;
    use fontcraft_engine::Session;

    use super::*;
    use crate::app::APP;
    use crate::app::preview_host::PreviewHost;

    /// A server that records what it was asked to serve.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<Option<(Site, Scope)>>>>);

    impl PreviewHost for Recorder {
        fn serve(&mut self, site: Site, scope: Scope) -> Result<String, String> {
            self.0.lock().unwrap().push(Some((site, scope)));
            Ok("http://192.0.2.1:8123/".into())
        }

        fn stop(&mut self) {
            self.0.lock().unwrap().push(None);
        }
    }

    fn shell(app: &mut FontCraftApp, params: Value) -> Result<Value, String> {
        app.run_now(Msg::Shell(CommandRef::with_params(ID, params)), false)
    }

    /// The command serves the page and the TrueType font with the given text and opens the dialog; Only this computer
    /// serves again to this computer; close stops the server. Without a server it is refused.
    #[test]
    fn the_command_serves_the_page_and_closing_stops_it() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        assert!(shell(&mut app, json!({"text": "Hamburg"})).unwrap_err().contains("desktop app"));

        let served = Recorder::default();
        app.preview_host = Some(Box::new(served.clone()));
        app.run_now(Msg::run("path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 400, "height": 600})), false).unwrap();
        shell(&mut app, json!({"text": "A<b>"})).unwrap();
        let st = app.tt.preview_address.clone();
        assert!(st.open);
        assert_eq!(st.address.as_deref(), Some("http://192.0.2.1:8123/"));
        assert_eq!(st.scope, Scope::LocalNetwork);
        {
            let log = served.0.lock().unwrap();
            let Some(Some((site, scope))) = log.first() else { panic!("one serve") };
            assert_eq!(*scope, Scope::LocalNetwork);
            assert!(site.page.contains("A&lt;b&gt;"), "the text is escaped on the page");
            assert!(site.page.contains("font-size:80px") && site.page.contains("font-size:8px"));
            assert_eq!(site.font.get(..4), Some(&[0u8, 1, 0, 0][..]), "a TrueType font");
        }

        // What the dialog's Only this computer asks for, as `run` posts it.
        let mut params = st.given.clone();
        params.insert("thisComputerOnly".into(), json!(true));
        shell(&mut app, Value::Object(params)).unwrap();
        assert_eq!(app.tt.preview_address.scope, Scope::ThisComputer);
        assert!(app.tt.preview_address.generation > st.generation, "an open dialog refreshes");

        shell(&mut app, json!({"close": true})).unwrap();
        assert!(!app.tt.preview_address.open);
        assert_eq!(served.0.lock().unwrap().last(), Some(&None), "closing stops the server");
    }
}
