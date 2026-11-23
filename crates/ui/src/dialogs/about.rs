//! About FontCraft (`app.about`, Help › About FontCraft): the version, the graphics backend in use, and the licences
//! of FontCraft and of everything it ships, in a scrolling list.
//!
//! The licences are `NOTICE` a paragraph to a row, each row wrapping at the dialog's width, then a row for each row of
//! the table in `ASSETS.md` (path, author, licence), both read in at compile time. The graphics line names wgpu, the only renderer the window is built with, and the
//! adapter and backend when the app was told them ([`FontCraftApp::graphics`]); the framework does not hand them to the
//! app yet, so without them it says they are not reported. The web build says it draws in the browser.

use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::runtime::env::Env;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Ui;

use crate::app::console::About;
use crate::app::{APP, FontCraftApp};
use crate::panels::widgets;

/// The notices of FontCraft and of the third-party code and data it ships.
const NOTICE: &str = include_str!("../../../../NOTICE");
/// The table of every asset the repository ships, with its author and licence.
const ASSETS: &str = include_str!("../../../../ASSETS.md");
/// Most rows the licences list holds.
pub const MAX_LICENCE_LINES: usize = 4_000;
/// The licences list's height, in lines of body text.
const LIST_ROWS: f32 = 16.0;

craft::ui::words! {
    /// The words of the About dialog.
    pub struct AboutWords {
        title: "fontcraftAbout.title" = "About FontCraft";
        blurb: "fontcraftAbout.blurb" = "A font editor and font tools, open source.";
        version: template "fontcraftAbout.version" = "Version {version}";
        graphics: template "fontcraftAbout.graphics" = "Graphics: {backend}";
        licences: "fontcraftAbout.licences" = "Licensed MIT OR Apache-2.0. The notices of the code and assets FontCraft ships:";
        close: "fontcraftAbout.close" = "Close";
    }
}

/// The graphics adapter the window draws with, as the renderer reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graphics {
    /// The graphics API in use (`Vulkan`, `Metal`, `Dx12`, `Gl`, `BrowserWebGpu`).
    pub backend: String,
    /// The adapter's name (the GPU or the software renderer).
    pub adapter: String,
}

/// What the graphics line says: wgpu, with the backend and adapter when they are known.
pub fn backend_text(graphics: Option<&Graphics>) -> String {
    let known = graphics.filter(|g| !g.backend.trim().is_empty() || !g.adapter.trim().is_empty());
    let web = cfg!(target_arch = "wasm32");
    match (known, web) {
        (Some(g), true) => format!("wgpu in the web browser, {} on {}", or_unknown(&g.backend), or_unknown(&g.adapter)),
        (None, true) => "wgpu in the web browser (WebGPU, or WebGL where the browser has no WebGPU)".into(),
        (Some(g), false) => format!("wgpu, {} on {}", or_unknown(&g.backend), or_unknown(&g.adapter)),
        (None, false) => "wgpu (the adapter in use is not reported)".into(),
    }
}

fn or_unknown(text: &str) -> &str {
    if text.trim().is_empty() { "an unnamed adapter" } else { text.trim() }
}

/// The licences list: `NOTICE` a paragraph to a row, a blank row, then a row for each asset of `ASSETS.md`.
pub fn licence_lines() -> Vec<String> {
    let mut lines = paragraphs(NOTICE);
    lines.push(String::new());
    lines.push("Assets (ASSETS.md):".into());
    lines.extend(ASSETS.lines().filter_map(asset_line));
    lines.truncate(MAX_LICENCE_LINES);
    lines
}

/// `text` with the hard-wrapped lines of each paragraph joined by spaces. Blank lines separate paragraphs and stay as
/// one empty row between them; a line that starts a list item (`- `) starts a paragraph of its own. A paragraph keeps
/// the indent of its first line.
fn paragraphs(text: &str) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let words = line.trim();
        if words.is_empty() {
            if rows.last().is_some_and(|row| !row.is_empty()) {
                rows.push(String::new());
            }
            open = false;
            continue;
        }
        if open
            && !words.starts_with("- ")
            && let Some(row) = rows.last_mut()
        {
            row.push(' ');
            row.push_str(words);
        } else {
            rows.push(line.trim_end().to_string());
            open = true;
        }
    }
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// One row of the assets table as a line: `path: author; licence`. `None` for any other line, the header and the
/// rule under it.
fn asset_line(row: &str) -> Option<String> {
    let cells: Vec<String> = row.trim().strip_prefix('|')?.strip_suffix('|')?.split('|').map(|c| c.trim().replace('`', "")).collect();
    let [path, _what, author, _source, licence] = cells.as_slice() else { return None };
    if path == "Path" || path.starts_with("---") || path.is_empty() {
        return None;
    }
    Some(format!("{path}: {author}; {licence}"))
}

/// What the dialog shows.
#[derive(Clone, Debug, PartialEq)]
pub struct AboutText {
    pub version: String,
    pub backend: String,
    pub licences: Vec<String>,
}

impl AboutText {
    /// The text for this build, with the graphics adapter when it is known.
    pub fn new(graphics: Option<&Graphics>) -> AboutText {
        AboutText { version: APP.version.to_string(), backend: backend_text(graphics), licences: licence_lines() }
    }
}

/// The dialog's body: the name, the version, the graphics line and the licences in a list that scrolls.
pub fn view(ui: &mut Ui, about: &AboutText) {
    let w = Env::of(ui).words::<AboutWords>();
    let list_height = ui.text_style_height(&egui::TextStyle::Body) * LIST_ROWS;
    Stack::new().show(ui, |ui, _| {
        Text::new(APP.name).show(ui);
        Text::new(w.blurb.clone()).show(ui);
        Text::new(w.version.fill(&[("version", about.version.as_str())]).to_string()).show(ui);
        Text::new(w.graphics.fill(&[("backend", about.backend.as_str())]).to_string()).show(ui);
        Text::new(w.licences.clone()).show(ui);
        Stack::new().room_height(list_height).show(ui, |ui, _| {
            widgets::scrolling(ui, |ui| {
                for line in &about.licences {
                    Text::new(line.clone()).shade(Shade::Muted).show(ui);
                }
            });
        });
    });
}

/// The open dialog on the window's host.
pub struct AboutDialog {
    pub about: AboutText,
    open: bool,
}

impl StackDialog for AboutDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftAbout")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let words = Env::of(ui).words::<AboutWords>();
        let about = &self.about;
        let shown = Dialog::new(words.title.clone())
            .opened(Bind::bound(&mut self.open))
            .buttons([FooterButton::new(words.close.clone(), DialogRole::Accept)])
            .show(ui, |ui, _| view(ui, about));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    match app.view.commands.about {
        About::Open => {
            let _ = host.open(AboutDialog { about: AboutText::new(app.graphics.as_ref()), open: true });
            app.view.commands.about = About::Shown;
        }
        About::Shown => {
            if host.take::<AboutDialog>().is_some() || !host.is_open::<AboutDialog>() {
                app.view.commands.about = About::Closed;
            }
        }
        About::Closed => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use craft::ui::testing::Fixture;

    use super::*;

    /// The dialog shows the version, the graphics line and the licences, starting with NOTICE's copyright line, and
    /// lists the assets of ASSETS.md one to a row.
    #[test]
    fn about_shows_the_version_and_the_notices() {
        let about = AboutText::new(None);
        assert!(about.licences.iter().any(|l| l.starts_with("crates/glyphdata/data/glyphdata.bin: ")), "an asset row");
        let shown = about.clone();
        let mut f = Fixture::new(move |ui| view(ui, &shown));
        f.room(egui::vec2(640.0, 900.0)).settle();
        let painted: Vec<&str> = f.texts().iter().map(|t| t.text.as_str()).collect();
        let version = format!("Version {}", APP.version);
        assert!(painted.contains(&version.as_str()), "{painted:?}");
        assert!(painted.iter().any(|t| t.contains("Copyright (c) 2026 The FontCraft contributors")), "{painted:?}");
        assert!(painted.iter().any(|t| t.starts_with("Graphics: wgpu")), "{painted:?}");
    }

    /// NOTICE's hard-wrapped lines join into one row a paragraph; blank lines leave one empty row, and each list item
    /// is a row of its own.
    #[test]
    fn notice_rows_are_whole_paragraphs() {
        let text = "Title\nfirst line\nsecond line.\n\n\n  Intro:\n\n  - one\n    more\n  - two\n\n";
        assert_eq!(paragraphs(text), ["Title first line second line.", "", "  Intro:", "", "  - one more", "  - two"]);
        let rows = licence_lines();
        let sentence = "It follows the conventions of the ArtCraft Crafting Apps (https://github.com/storytold) and is not";
        assert!(rows.iter().any(|row| row.contains(sentence)), "a sentence NOTICE wraps is one row");
        assert!(rows.len() <= MAX_LICENCE_LINES);
    }

    /// Without adapter information (a headless run, or a framework that does not report it) the graphics line still
    /// reads as a sentence; an adapter that is reported is named.
    #[test]
    fn the_graphics_line_falls_back_when_no_adapter_is_reported() {
        assert_eq!(backend_text(None), "wgpu (the adapter in use is not reported)");
        assert_eq!(backend_text(Some(&Graphics::default())), "wgpu (the adapter in use is not reported)");
        let adapter = Graphics { backend: "Vulkan".into(), adapter: "llvmpipe".into() };
        assert_eq!(backend_text(Some(&adapter)), "wgpu, Vulkan on llvmpipe");
        let unnamed = Graphics { backend: "Gl".into(), adapter: " ".into() };
        assert_eq!(backend_text(Some(&unnamed)), "wgpu, Gl on an unnamed adapter");
    }
}
