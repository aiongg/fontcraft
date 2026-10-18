//! The web preview address: where another device on the local network can open the hinted preview, while the dialog is open (desktop only).
//!
//! Rows: `dialog.ttPreviewAddress`, `hint.tt.showPreviewAddress`.
//!
//! `hint.tt.showPreviewAddress` builds the font as File › Export writes it, as TrueType, and a
//! page that sets the showing Edit View tab's text at every pixel size from 8 to 80, as the
//! Hinting Preview does. A [`PreviewHost`] serves both; the desktop app supplies one
//! (`apps/fontcraft/src/preview_server.rs`), the browser build has none. The dialog shows the
//! page's address with a Copy button. Closing it stops the server (M10 § 7, question 5).
//!
//! The page is plain HTML and CSS: a browser that applies TrueType hinting draws it with the
//! font's instructions, which is what the page is for. It holds no script.

use std::fmt::Write as _;

use base64::Engine as _;
use fontcraft_textlayout::Item;
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};

pub const ID: &str = "hint.tt.showPreviewAddress";
/// The smallest and the largest pixel size on the page, as in the Hinting Preview.
pub const MIN_SIZE: u32 = 8;
pub const MAX_SIZE: u32 = 80;
/// Most characters of sample text on the page (each is set once per size).
pub const MAX_TEXT_CHARS: usize = 500;
/// The family name the page gives the served font in its `@font-face` rule.
const PAGE_FAMILY: &str = "FontCraft Hinting Preview";
/// The font's path relative to the page.
pub const FONT_FILE: &str = "font.ttf";

pub const COMMANDS: &[UiCommand] = &[(
    ID,
    "Show Preview Address…",
    &[],
    "",
    "{close?: bool (true closes the dialog and stops the server), text?: string (the sample text, at most 500 characters; default the showing Edit View tab's glyphs, by their characters, leaving out glyphs without one), instance?: string (a static instance by name, `#n` for an unnamed one; default the first exported static instance, else the first master), thisComputerOnly?: bool (true serves this computer only; default the local network)} builds the instance as TrueType as File › Export would, starts a web server for a page that sets the text at every pixel size from 8 to 80 in that font, and opens the Preview Address dialog with the page's address. Run again while the dialog is open, it serves the font as it is now at the same address. Desktop app only → {open, address?, scope?: 'localNetwork'|'thisComputer', instance?, text?, leftOut?, warnings?}",
)];

/// Who may open the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Every device on the local network (all of this computer's network interfaces).
    #[default]
    LocalNetwork,
    /// This computer only (the loopback interface).
    ThisComputer,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::LocalNetwork => "localNetwork",
            Scope::ThisComputer => "thisComputer",
        }
    }
}

/// What the server hands out: the page, and the font it loads from [`FONT_FILE`] beside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub page: String,
    pub font: Vec<u8>,
}

/// A web server for the preview page, supplied by the app that can open network ports.
pub trait PreviewHost: Send {
    /// Serves `site` to `scope` and returns the page's address. While a server runs for the same
    /// scope, it keeps its address and serves `site` from the next request on. For another scope
    /// a new server starts first and the old one stops after, so a failure leaves the running
    /// server as it was.
    fn serve(&mut self, site: Site, scope: Scope) -> Result<String, String>;
    /// Stops the server. Its port is closed when this returns.
    fn stop(&mut self);
}

/// The dialog's state. Not saved with the window state: a server never outlives the app.
#[derive(Default)]
pub struct PreviewAddress {
    /// The server; `None` where the app cannot serve pages (the browser, MCP).
    pub host: Option<Box<dyn PreviewHost>>,
    pub open: bool,
    pub address: Option<String>,
    pub scope: Scope,
    /// The instance served and the characters shown, for the dialog.
    pub summary: String,
    /// The `text` and `instance` the dialog was opened with, so its buttons rebuild the same page.
    given: Map<String, Value>,
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != ID {
        return Err(format!("unknown command `{id}`"));
    }
    if args.flag("close")?.unwrap_or(false) {
        close(app);
        return Ok(json!({ "open": false }));
    }
    let scope = if args.flag("thisComputerOnly")?.unwrap_or(false) { Scope::ThisComputer } else { Scope::LocalNetwork };
    let given_text = args.string("text")?;
    let instance = args.string("instance")?;
    if app.preview_address.host.is_none() {
        return Err(args.err("the web preview needs the desktop app: no web server can be started here"));
    }
    let (text, left_out) = sample_text(app, given_text).map_err(|e| args.err(e))?;
    let built = build_font(app, instance).map_err(|e| args.err(e))?;
    let title = app.session.doc().map(|d| d.title.clone()).unwrap_or_default();
    let site = Site { page: page(&title, &built.instance, &text), font: built.font };
    let host = app.preview_address.host.as_mut().ok_or_else(|| args.err("no web server"))?;
    let address = host.serve(site, scope).map_err(|e| args.err(format!("the web server could not start: {e}")))?;

    let st = &mut app.preview_address;
    st.open = true;
    st.address = Some(address.clone());
    st.scope = scope;
    st.summary = summary(&built.instance, &text, left_out);
    st.given = Map::new();
    if let Some(t) = given_text {
        st.given.insert("text".into(), json!(t));
    }
    if let Some(i) = instance {
        st.given.insert("instance".into(), json!(i));
    }
    Ok(json!({
        "open": true, "address": address, "scope": scope.name(), "instance": built.instance, "text": text,
        "leftOut": left_out, "warnings": built.warnings,
    }))
}

/// Closes the dialog and stops its server.
pub fn close(app: &mut FontcraftApp) {
    let st = &mut app.preview_address;
    if let Some(host) = st.host.as_mut() {
        host.stop();
    }
    st.open = false;
    st.address = None;
    st.summary.clear();
}

fn summary(instance: &str, text: &str, left_out: usize) -> String {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count();
    let mut s = format!("{instance}: {chars} characters at {MIN_SIZE} to {MAX_SIZE} pixels per em.");
    if left_out > 0 {
        let _ = write!(s, " {left_out} glyphs without a character are left out.");
    }
    s
}

/// The sample text: `given`, else the showing tab's glyphs by their characters, and how many
/// of the tab's glyphs have no character to show.
fn sample_text(app: &FontcraftApp, given: Option<&str>) -> Result<(String, usize), String> {
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
    let tab = app.state.tab().ok_or("no Edit View tab is showing: open the glyphs to preview, or give `text`")?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let encoded = |name: &str| font.glyph(name).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32).filter(|c| !c.is_control());
    let mut text = String::new();
    let mut left_out = 0usize;
    for item in tab.text.items().iter().take(MAX_TEXT_CHARS) {
        let c = match item {
            Item::Glyph { typed: Some(c), .. } if !c.is_control() => Some(*c),
            Item::Glyph { name, .. } => encoded(name),
            Item::Placeholder => tab.text.current_glyph().and_then(encoded),
            Item::Missing(_) => None,
            Item::LineBreak => Some(' '),
        };
        match c {
            Some(c) => text.push(c),
            None => left_out += 1,
        }
    }
    if text.trim().is_empty() {
        return Err("none of the tab's glyphs has a character to show: give `text`".into());
    }
    Ok((text, left_out))
}

struct BuiltFont {
    font: Vec<u8>,
    instance: String,
    warnings: Vec<Value>,
}

/// The first exported static instance, else the first static instance; `None` when the font has
/// none, so the export builds its first master. Labels as the export names them.
fn default_instance(font: &fontcraft_model::Font) -> Option<String> {
    let statics: Vec<(usize, &fontcraft_model::Instance)> = font.instances.iter().enumerate().filter(|(_, i)| i.kind.is_none()).collect();
    let (i, inst) = statics.iter().find(|(_, inst)| inst.exports).or_else(|| statics.first())?;
    Some(if inst.name.is_empty() { format!("#{i}") } else { inst.name.clone() })
}

/// The instance as File › Export writes it, with TrueType outlines, in memory.
fn build_font(app: &mut FontcraftApp, instance: Option<&str>) -> Result<BuiltFont, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let label = instance.map(str::to_string).or_else(|| default_instance(font));
    let mut params = Map::new();
    params.insert("outlines".into(), json!("truetype"));
    params.insert("containers".into(), json!(["plain"]));
    if let Some(label) = &label {
        params.insert("instances".into(), json!([label]));
    }
    let r = app.session.execute("file.exportStatic", &Value::Object(params)).map_err(|e| e.to_string())?;
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

/// The preview page: `text` set in the served font once per pixel size, each line labelled with
/// its size. CSS pixels equal device pixels only at 100% zoom and display scaling, which the
/// page says.
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

/// Draws the Preview Address dialog while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.preview_address.open {
        return;
    }
    let tokens = &app.tokens;
    let st = &app.preview_address;
    let address = st.address.clone().unwrap_or_default();
    let mut copy = false;
    let mut local_only = st.scope == Scope::ThisComputer;
    let mut rebuild: Option<bool> = None;
    let mut open = true;
    egui::Window::new("Preview Address")
        .id(egui::Id::new("dialog.ttPreviewAddress"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.label("Open this address in a web browser to see the hinted preview:");
            ui.add_space(tokens.dialog_spacing);
            ui.horizontal(|ui| {
                let mut shown = address.as_str();
                ui.add(egui::TextEdit::singleline(&mut shown).desired_width(tokens.dialog_field_width * 2.0));
                copy = ui.button("Copy").clicked();
            });
            ui.add_space(tokens.dialog_spacing);
            ui.colored_label(tokens.text_dim, &st.summary);
            let reach = match st.scope {
                Scope::LocalNetwork => "While this window is open, any device on the local network that has the address can load the font.",
                Scope::ThisComputer => "Only browsers on this computer can open the address.",
            };
            ui.colored_label(tokens.text_dim, reach);
            ui.add_space(tokens.dialog_spacing);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut local_only, "Only this computer").changed() {
                    rebuild = Some(local_only);
                }
                if ui.button("Update").on_hover_text("Serve the font as it is now; reload the page to see it").clicked() {
                    rebuild = Some(local_only);
                }
            });
        });
    if copy && !address.is_empty() {
        app.clipboard_out = Some(address);
    }
    if !open {
        pending.push((ID, json!({ "close": true })));
    } else if let Some(local_only) = rebuild {
        let mut params = app.preview_address.given.clone();
        params.insert("thisComputerOnly".into(), json!(local_only));
        pending.push((ID, Value::Object(params)));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    /// What the fake server was asked to do.
    #[derive(Default)]
    struct Log {
        served: Vec<(Site, Scope)>,
        stops: usize,
    }

    struct FakeHost(Arc<Mutex<Log>>);

    impl PreviewHost for FakeHost {
        fn serve(&mut self, site: Site, scope: Scope) -> Result<String, String> {
            let mut log = self.0.lock().unwrap();
            log.served.push((site, scope));
            Ok(format!("http://192.0.2.7:4321/{}/", log.served.len()))
        }
        fn stop(&mut self) {
            self.0.lock().unwrap().stops += 1;
        }
    }

    fn app_with_host() -> (FontcraftApp, Arc<Mutex<Log>>) {
        let mut app = FontcraftApp::default();
        let log = Arc::new(Mutex::new(Log::default()));
        app.preview_address.host = Some(Box::new(FakeHost(log.clone())));
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 50, "y": 0, "width": 500, "height": 700})).unwrap();
        (app, log)
    }

    #[test]
    fn show_preview_address_serves_the_tabs_text_in_the_exported_font_until_the_dialog_closes() {
        let (mut app, log) = app_with_host();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        let r = app.run(ID, &json!({})).unwrap();
        assert_eq!(r["open"], true, "{r}");
        assert_eq!(r["scope"], "localNetwork", "the local network by default, as decided for M10");
        assert_eq!(r["text"], "A");
        assert_eq!(r["address"], "http://192.0.2.7:4321/1/");
        assert!(app.preview_address.open);
        {
            let log = log.lock().unwrap();
            let (site, scope) = &log.served[0];
            assert_eq!(*scope, Scope::LocalNetwork);
            assert_eq!(site.font.get(..4), Some(&[0u8, 1, 0, 0][..]), "a TrueType font is served");
            for size in [8, 80] {
                assert!(site.page.contains(&format!("font-size:{size}px\">A</span>")), "size {size} is on the page");
            }
            assert!(!site.page.contains("font-size:81px"));
            assert!(!site.page.contains("<script"), "the page holds no script");
        }

        // The dialog draws in a headless frame.
        let ctx = egui::Context::default();
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();

        // Only this computer: the same page served on loopback.
        let r = app.run(ID, &json!({"thisComputerOnly": true})).unwrap();
        assert_eq!(r["scope"], "thisComputer");
        assert_eq!(log.lock().unwrap().served[1].1, Scope::ThisComputer);

        let r = app.run(ID, &json!({"close": true})).unwrap();
        assert_eq!(r["open"], false);
        assert!(!app.preview_address.open);
        assert_eq!(app.preview_address.address, None);
        assert_eq!(log.lock().unwrap().stops, 1, "closing the dialog stops the server");
    }

    #[test]
    fn show_preview_address_escapes_the_text_and_refuses_without_a_server_or_a_sample() {
        let (mut app, log) = app_with_host();
        let r = app.run(ID, &json!({"text": "A<b>&\"x"})).unwrap();
        assert_eq!(r["open"], true);
        let page = log.lock().unwrap().served[0].0.page.clone();
        assert!(page.contains("A&lt;b&gt;&amp;&quot;x"), "{page}");
        assert!(!page.contains("<b>"));

        let too_long = "a".repeat(MAX_TEXT_CHARS + 1);
        assert!(app.run(ID, &json!({"text": too_long})).is_err());
        assert!(app.run(ID, &json!({"text": "\n\t"})).is_err());
        assert!(app.run(ID, &json!({"instance": "No Such Instance"})).is_err());
        assert_eq!(log.lock().unwrap().served.len(), 1, "a refused request serves nothing new");

        // Font View showing and no text: nothing to show.
        let mut bare = FontcraftApp::default();
        bare.preview_address.host = Some(Box::new(FakeHost(Arc::new(Mutex::new(Log::default())))));
        let err = bare.run(ID, &json!({})).unwrap_err();
        assert!(err.contains("Edit View tab"), "{err}");

        // No server where the app cannot open ports.
        let mut browser = FontcraftApp::default();
        let err = browser.run(ID, &json!({"text": "A"})).unwrap_err();
        assert!(err.contains("desktop app"), "{err}");
        assert!(!browser.preview_address.open);
    }
}
