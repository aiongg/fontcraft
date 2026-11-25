//! File › Print… (`file.print`): the active font's proof pages, sent to a printer or saved as a PDF file.
//!
//! The pages are `font.proof` with `document`, made in memory as a PDF; the dialog shows them one at a time
//! (`fontcraft_io_pdf::render_pdf_page`) and prints the very bytes it shows. The printer, the copies, two-sided and
//! grayscale printing are the suite's print panel (`craft::print::ui`, decision 0008), and the job goes through the
//! app's [`PrintHost`](crate::app::print_host::PrintHost). The browser build has no printers and saves PDF only.
//!
//! | Control | Posts `file.print` with |
//! |---|---|
//! | Layout, Style, Paper | `{layout}`, `{master}` or `{instance}`, `{paper}` |
//! | Landscape, Page Titles | `{landscape}`, `{header}` |
//! | Selected Glyphs Only | `{glyphs: "selected"}` or `{glyphs: "all"}` |
//! | Text, Point Size | `{text}`, `{pointSize}` |
//! | Save as PDF… | `{saveAs: true}`: the window asks for the file |
//! | Done | `{close: true}` |
//!
//! Print sends the job and keeps the dialog open for the spooler's answer.

use std::sync::Arc;

use base64::Engine as _;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::Select;
use craft::ui::primitives::TextureCache;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::{NumberField, TextField};
use egui::{Color32, ColorImage, Pos2, Rect, Sense, TextureOptions, Ui, Vec2};
use fontcraft_model::Font;
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::panels::widgets::{self, labelled};
use crate::shell::commands::{Module, UiCommand};

craft::ui::words! {
    /// The words of the Print dialog.
    pub struct PrintWords {
        title: "fontcraftPrint.title" = "Print";
        layout: "fontcraftPrint.layout" = "Layout";
        font_chart: "fontcraftPrint.fontChart" = "Font Chart";
        glyph_per_page: "fontcraftPrint.glyphPerPage" = "One Glyph per Page";
        glyph_waterfall: "fontcraftPrint.glyphWaterfall" = "Waterfall";
        sample_text: "fontcraftPrint.sampleText" = "Sample Text";
        style: "fontcraftPrint.style" = "Style";
        paper: "fontcraftPrint.paper" = "Paper";
        letter: "fontcraftPrint.letter" = "US Letter";
        a4: "fontcraftPrint.a4" = "A4";
        a3: "fontcraftPrint.a3" = "A3";
        landscape: "fontcraftPrint.landscape" = "Landscape";
        header: "fontcraftPrint.header" = "Page Titles";
        selected_only: "fontcraftPrint.selectedOnly" = "Selected Glyphs Only";
        text: "fontcraftPrint.text" = "Text";
        point_size: "fontcraftPrint.pointSize" = "Point Size";
        preview: "fontcraftPrint.preview" = "Page preview";
        preview_note: "fontcraftPrint.previewNote" = "The preview leaves out the labels set in the printer's own font.";
        previous: "fontcraftPrint.previous" = "Previous Page";
        next: "fontcraftPrint.next" = "Next Page";
        page: "fontcraftPrint.page" = "Page {page} of {pages}";
        print: "fontcraftPrint.print" = "Print";
        save: "fontcraftPrint.save" = "Save as PDF…";
        done: "fontcraftPrint.done" = "Done";
        no_printing: "fontcraftPrint.noPrinting" = "Printing needs the desktop app. Save the pages as a PDF file here.";
        finding: "fontcraftPrint.finding" = "Looking for printers…";
        no_printers: "fontcraftPrint.noPrinters" = "No printer was found. Save the pages as a PDF file, or add a printer and press Refresh.";
        discovery_failed: "fontcraftPrint.discoveryFailed" = "The list of printers could not be read.";
        sending: "fontcraftPrint.sending" = "Sending the pages to the printer…";
        sent: "fontcraftPrint.sent" = "Sent to the printer: {receipt}";
        failed: "fontcraftPrint.failed" = "The pages were not printed: {error}";
        stopped: "fontcraftPrint.stopped" = "The print job stopped without an answer.";
    }
}

pub const ID: &str = "file.print";
/// Longest text the dialog and the command take, in bytes.
const MAX_TEXT: usize = 16 << 10;
/// Most glyphs one job names.
const MAX_GLYPHS: usize = 65_535;
/// The point sizes the dialog and the command take.
const POINT_SIZES: std::ops::RangeInclusive<f64> = 4.0..=720.0;
/// The sizes of the waterfall, in points, largest first.
const LADDER: [f64; 15] = [72.0, 60.0, 48.0, 36.0, 30.0, 24.0, 18.0, 14.0, 12.0, 10.0, 9.0, 8.0, 7.0, 6.0, 4.0];
/// Rows of the text field.
const TEXT_LINES: usize = 4;
/// The preview's room, in points: a page fits inside it.
const PREVIEW: Vec2 = Vec2::new(300.0, 300.0);
/// Pixels per point of the preview's picture, against the size it is shown at.
const PREVIEW_DENSITY: f64 = 2.0;
/// Most pixels of the preview's picture.
const PREVIEW_PIXELS: u64 = 1 << 21;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: ID,
    label: "Print…",
    menu: &["File"],
    shortcut: None,
    params: "{layout?: \"fontChart\"|\"glyphPerPage\"|\"glyphWaterfall\"|\"sampleText\" (default fontChart), glyphs?: [name]|\"selected\"|\"all\" (the Font View selection or every glyph; default every glyph for the chart and the sample text, else the selection), text?: string (the waterfall's and the sample's text; without it the glyphs' characters, else the proof's own sample), pointSize?: number (4 to 720, the sample text's size, default 24), master?: id|name, instance?: name (default the default master), paper?: \"letter\"|\"a4\"|\"a3\", landscape?: bool, header?: bool (a title on each page, default true), output?: path, saveAs?: bool, close?: bool} opens the Print dialog on the active font's proof pages (font.proof with document), or changes the open dialog; what is left out stays as it was. output writes the pages as a PDF file instead; saveAs asks for that file in the window; close closes the dialog. ui.inspect reports the dialog under `print`: {open, layout, glyphs, pages, font, error}",
}];

/// What the pages show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageLayout {
    /// A chart of the glyphs.
    #[default]
    FontChart,
    /// One glyph per page, at page size.
    GlyphPerPage,
    /// A line at each size of a ladder.
    GlyphWaterfall,
    /// A text.
    SampleText,
}

impl PageLayout {
    const ALL: [PageLayout; 4] = [PageLayout::FontChart, PageLayout::GlyphPerPage, PageLayout::GlyphWaterfall, PageLayout::SampleText];

    /// Its name in the parameters.
    pub fn name(self) -> &'static str {
        match self {
            PageLayout::FontChart => "fontChart",
            PageLayout::GlyphPerPage => "glyphPerPage",
            PageLayout::GlyphWaterfall => "glyphWaterfall",
            PageLayout::SampleText => "sampleText",
        }
    }

    /// The layout of `font.proof` that draws it.
    fn proof(self) -> &'static str {
        match self {
            PageLayout::FontChart => "palette",
            PageLayout::GlyphPerPage => "outline",
            PageLayout::GlyphWaterfall => "waterfall",
            PageLayout::SampleText => "text",
        }
    }

    fn label(self, words: &PrintWords) -> &Label {
        match self {
            PageLayout::FontChart => &words.font_chart,
            PageLayout::GlyphPerPage => &words.glyph_per_page,
            PageLayout::GlyphWaterfall => &words.glyph_waterfall,
            PageLayout::SampleText => &words.sample_text,
        }
    }

    /// Whether the layout reads a text.
    fn has_text(self) -> bool {
        matches!(self, PageLayout::GlyphWaterfall | PageLayout::SampleText)
    }

    /// Whether the layout shows the Font View selection when nothing says which glyphs.
    fn prefers_selection(self) -> bool {
        matches!(self, PageLayout::GlyphPerPage | PageLayout::GlyphWaterfall)
    }
}

/// A paper size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paper {
    #[default]
    Letter,
    A4,
    A3,
}

impl Paper {
    const ALL: [Paper; 3] = [Paper::Letter, Paper::A4, Paper::A3];

    pub fn name(self) -> &'static str {
        match self {
            Paper::Letter => "letter",
            Paper::A4 => "a4",
            Paper::A3 => "a3",
        }
    }

    /// Its width and height in points, upright.
    fn points(self) -> [f64; 2] {
        match self {
            Paper::Letter => [612.0, 792.0],
            Paper::A4 => [595.276, 841.89],
            Paper::A3 => [841.89, 1190.551],
        }
    }

    fn label(self, words: &PrintWords) -> &Label {
        match self {
            Paper::Letter => &words.letter,
            Paper::A4 => &words.a4,
            Paper::A3 => &words.a3,
        }
    }
}

/// The master or instance the pages show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// The default master.
    #[default]
    Default,
    /// The master with this id or name.
    Master(String),
    /// The static instance with this name (`#n` for an unnamed one).
    Instance(String),
}

/// What the dialog prints.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub layout: PageLayout,
    /// The glyphs shown; empty is every glyph.
    pub glyphs: Vec<String>,
    pub text: String,
    pub point_size: f64,
    pub style: Style,
    pub paper: Paper,
    pub landscape: bool,
    pub header: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            layout: PageLayout::default(),
            glyphs: Vec::new(),
            text: String::new(),
            point_size: 24.0,
            style: Style::default(),
            paper: Paper::default(),
            landscape: false,
            header: true,
        }
    }
}

fn given<'v>(params: &'v Value, key: &str) -> Option<&'v Value> {
    params.get(key).filter(|v| !v.is_null())
}

fn flag(params: &Value, key: &str) -> Result<Option<bool>, String> {
    given(params, key).map(|v| v.as_bool().ok_or_else(|| format!("`{key}` must be true or false"))).transpose()
}

fn text_of<'v>(params: &'v Value, key: &str) -> Result<Option<&'v str>, String> {
    given(params, key).map(|v| v.as_str().ok_or_else(|| format!("`{key}` must be text"))).transpose()
}

impl Setup {
    /// Takes what `params` give; `selected` is the Font View selection.
    pub fn read(&mut self, params: &Value, selected: &[String]) -> Result<(), String> {
        let layout = text_of(params, "layout")?;
        if let Some(name) = layout {
            let found = PageLayout::ALL.into_iter().find(|l| l.name() == name);
            self.layout = found.ok_or("`layout` must be \"fontChart\", \"glyphPerPage\", \"glyphWaterfall\" or \"sampleText\"")?;
        }
        let preferred = |layout: PageLayout| if layout.prefers_selection() { selected.to_vec() } else { Vec::new() };
        match given(params, "glyphs") {
            None if layout.is_some() => self.glyphs = preferred(self.layout),
            None => {}
            Some(Value::String(word)) => {
                self.glyphs = match word.as_str() {
                    "all" => Vec::new(),
                    "selected" if selected.is_empty() => return Err("no glyph is selected".into()),
                    "selected" => selected.to_vec(),
                    _ => return Err("`glyphs` must be a list of names, \"selected\" or \"all\"".into()),
                };
            }
            Some(Value::Array(names)) => {
                let names: Option<Vec<String>> = names.iter().map(|n| n.as_str().map(str::to_string)).collect();
                self.glyphs = names.ok_or("`glyphs` holds glyph names")?;
            }
            Some(_) => return Err("`glyphs` must be a list of names, \"selected\" or \"all\"".into()),
        }
        if self.glyphs.len() > MAX_GLYPHS {
            return Err(format!("`glyphs` names at most {MAX_GLYPHS} glyphs"));
        }
        if let Some(text) = text_of(params, "text")? {
            if text.len() > MAX_TEXT {
                return Err(format!("`text` holds at most {MAX_TEXT} bytes"));
            }
            self.text = text.to_string();
        }
        if let Some(size) = given(params, "pointSize") {
            self.point_size = size.as_f64().filter(|s| POINT_SIZES.contains(s)).ok_or("`pointSize` must be from 4 to 720")?;
        }
        match (text_of(params, "master")?, text_of(params, "instance")?) {
            (Some(_), Some(_)) => return Err("give `master` or `instance`, not both".into()),
            (Some(m), None) => self.style = Style::Master(m.to_string()),
            (None, Some(i)) => self.style = Style::Instance(i.to_string()),
            (None, None) => {}
        }
        if let Some(name) = text_of(params, "paper")? {
            let found = Paper::ALL.into_iter().find(|p| p.name() == name);
            self.paper = found.ok_or("`paper` must be \"letter\", \"a4\" or \"a3\"")?;
        }
        if let Some(on) = flag(params, "landscape")? {
            self.landscape = on;
        }
        if let Some(on) = flag(params, "header")? {
            self.header = on;
        }
        Ok(())
    }

    /// The `font.proof` request for `font`'s pages, without an output.
    pub fn proof_params(&self, font: &Font) -> Map<String, Value> {
        let [short, long] = self.paper.points();
        let page = if self.landscape { [long, short] } else { [short, long] };
        let mut p = Map::new();
        p.insert("document".into(), json!(true));
        p.insert("layout".into(), json!(self.layout.proof()));
        p.insert("format".into(), json!("pdf"));
        p.insert("pageSize".into(), json!(page));
        p.insert("header".into(), json!(self.header));
        match &self.style {
            Style::Default => {}
            Style::Master(m) => _ = p.insert("master".into(), json!(m)),
            Style::Instance(i) => _ = p.insert("instance".into(), json!(i)),
        }
        if !self.glyphs.is_empty() && self.layout != PageLayout::GlyphWaterfall {
            p.insert("glyphs".into(), json!(self.glyphs));
        }
        if self.layout.has_text() {
            let text =
                if self.text.trim().is_empty() && self.layout == PageLayout::GlyphWaterfall { self.characters(font) } else { self.text.clone() };
            if !text.trim().is_empty() {
                p.insert("text".into(), json!(text));
            }
        }
        match self.layout {
            PageLayout::GlyphWaterfall => _ = p.insert("sizes".into(), json!(LADDER)),
            PageLayout::SampleText => _ = p.insert("size".into(), json!(self.point_size)),
            PageLayout::FontChart | PageLayout::GlyphPerPage => {}
        }
        p
    }

    /// The characters of the chosen glyphs, as many as the text takes.
    fn characters(&self, font: &Font) -> String {
        let mut text = String::new();
        for name in &self.glyphs {
            let encoded = font.glyph(name).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32).filter(|c| !c.is_control());
            if let Some(c) = encoded
                && text.len().saturating_add(c.len_utf8()) <= MAX_TEXT
            {
                text.push(c);
            }
        }
        text
    }
}

/// The pages the dialog shows and prints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pages {
    pub pdf: Arc<Vec<u8>>,
    pub count: usize,
    /// The family and the master or instance.
    pub font: String,
}

/// The dialog's app state, in `FontCraftApp::printing`.
#[derive(Clone, Debug, Default)]
pub struct PrintState {
    pub open: bool,
    pub setup: Setup,
    pub pages: Option<Pages>,
    /// Why there are no pages.
    pub error: Option<String>,
    /// Moves whenever the pages are made again.
    generation: u64,
}

/// Makes the pages of `setup`.
fn make_pages(app: &mut FontCraftApp, setup: &Setup) -> Result<Pages, String> {
    let params = setup.proof_params(app.session.font().map_err(|e| e.to_string())?);
    let r = app.run_command("font.proof", Value::Object(params))?;
    let proof = r.pointer("/proofs/0").ok_or("the proof made no pages")?;
    let data = proof.get("dataBase64").and_then(Value::as_str).ok_or("the proof kept no pages")?;
    let pdf = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| format!("the pages cannot be read: {e}"))?;
    let count = proof.get("pages").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let font = proof.get("font").and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(Pages { pdf: Arc::new(pdf), count, font })
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if command.id != ID {
        return Err(format!("no print command `{}`", command.id));
    }
    let params = &command.params;
    if flag(params, "close")? == Some(true) {
        app.printing.open = false;
        app.printing.pages = None;
        app.printing.error = None;
        return Ok(());
    }
    let selected: Vec<String> = app.view.font_view.selected.iter().take(MAX_GLYPHS).cloned().collect();
    let mut setup = app.printing.setup.clone();
    if !app.printing.open {
        // A new dialog shows the glyphs its layout prefers, of the document now active, in its default style.
        setup.style = Style::default();
        setup.glyphs = if setup.layout.prefers_selection() { selected.clone() } else { Vec::new() };
    }
    setup.read(params, &selected)?;
    if let Some(path) = text_of(params, "output")? {
        let mut proof = setup.proof_params(app.session.font().map_err(|e| e.to_string())?);
        proof.insert("output".into(), json!(path));
        app.run_command("font.proof", Value::Object(proof))?;
        app.status = format!("Saved {path}");
        return Ok(());
    }
    if flag(params, "saveAs")? == Some(true) {
        return Err("`saveAs` asks for a file in the window: give `output`".into());
    }
    let made = make_pages(app, &setup);
    if !app.printing.open {
        app.printing = PrintState { open: true, setup, generation: app.printing.generation.wrapping_add(1), pages: Some(made?), error: None };
        return Ok(());
    }
    let st = &mut app.printing;
    st.setup = setup;
    st.generation = st.generation.wrapping_add(1);
    (st.pages, st.error) = match made {
        Ok(pages) => (Some(pages), None),
        Err(e) => (None, Some(e)),
    };
    Ok(())
}

/// The `print` key of `ui.inspect`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let st = &app.printing;
    let pages = st.pages.as_ref();
    let value = json!({
        "open": st.open,
        "layout": st.setup.layout.name(),
        "glyphs": st.setup.glyphs.len(),
        "pages": pages.map_or(0, |p| p.count),
        "font": pages.map(|p| p.font.as_str()),
        "error": st.error.as_deref(),
    });
    state.insert("print".into(), value);
}

/// What the dialog shows.
#[derive(Clone, Debug, Default)]
pub struct PrintModel {
    pub setup: Setup,
    /// Every master and static instance, by its place in the pop-up.
    pub styles: Vec<(Style, String)>,
    /// How many glyphs the Font View has selected.
    pub selected: usize,
    pub pages: Option<Pages>,
    pub error: Option<String>,
    generation: u64,
}

impl PrintModel {
    fn of(app: &FontCraftApp) -> Option<PrintModel> {
        let st = &app.printing;
        let font = app.session.font().ok().filter(|_| st.open)?;
        let masters = font.masters.iter().map(|m| (Style::Master(m.id.clone()), m.name.clone()));
        let instances = font.instances.iter().enumerate().filter(|(_, i)| i.kind.is_none()).map(|(n, i)| {
            let label = if i.name.is_empty() { format!("#{n}") } else { i.name.clone() };
            (Style::Instance(label.clone()), label)
        });
        Some(PrintModel {
            setup: st.setup.clone(),
            styles: masters.chain(instances).collect(),
            selected: app.view.font_view.selected.len(),
            pages: st.pages.clone(),
            error: st.error.clone(),
            generation: st.generation,
        })
    }

    /// The place of the shown style in the pop-up. A master may be named by its name as well as its id; the default
    /// master is the first.
    fn style(&self) -> Option<usize> {
        match &self.setup.style {
            Style::Default => Some(0),
            Style::Master(m) => self.styles.iter().position(|(s, label)| matches!(s, Style::Master(id) if id == m || label == m)),
            Style::Instance(_) => self.styles.iter().position(|(s, _)| *s == self.setup.style),
        }
    }
}

/// A page drawn for the preview, or why it cannot be.
#[derive(Clone)]
struct Drawn {
    of: (u64, usize),
    picture: Result<Arc<ColorImage>, String>,
}

/// The preview: the page shown and its picture.
#[derive(Default)]
pub struct Preview {
    page: usize,
    drawn: Option<Drawn>,
}

impl Preview {
    /// The picture of the shown page, drawn when the pages or the page changed.
    fn picture(&mut self, pages: &Pages, generation: u64) -> Result<Arc<ColorImage>, String> {
        self.page = self.page.min(pages.count.saturating_sub(1));
        let of = (generation, self.page);
        match &self.drawn {
            Some(drawn) if drawn.of == of => drawn.picture.clone(),
            _ => {
                let picture = draw(&pages.pdf, self.page);
                self.drawn = Some(Drawn { of, picture: picture.clone() });
                picture
            }
        }
    }
}

fn draw(pdf: &[u8], page: usize) -> Result<Arc<ColorImage>, String> {
    // The density counts against the longest side a page has here, so every paper is drawn at about the same size.
    let density = PREVIEW_DENSITY * f64::from(PREVIEW.y) / 1190.551;
    let drawn = fontcraft_io_pdf::render_pdf_page(pdf, page, density, PREVIEW_PIXELS).map_err(|e| format!("The page cannot be shown: {e}"))?;
    let size = [drawn.image.width as usize, drawn.image.height as usize];
    if size.iter().product::<usize>().checked_mul(4) != Some(drawn.image.pixels.len()) {
        return Err("The page cannot be shown: its picture is incomplete".into());
    }
    Ok(Arc::new(ColorImage::from_rgba_unmultiplied(size, &drawn.image.pixels)))
}

fn post(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(ID, params))
}

/// The preview half: the page, its place among the pages, and the buttons to the pages beside it.
pub fn preview_view(ui: &mut Ui, m: &PrintModel, preview: &mut Preview) {
    let words = Env::of(ui).words::<PrintWords>();
    let Some(pages) = &m.pages else {
        Text::new(m.error.clone().unwrap_or_default()).show(ui);
        return;
    };
    match preview.picture(pages, m.generation) {
        Ok(picture) => {
            let (rect, response) = ui.allocate_exact_size(PREVIEW, Sense::hover());
            let name = format!("{}: {} {}", words.preview.as_str(), pages.font, preview.page.saturating_add(1));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, name.clone()));
            if ui.is_rect_visible(rect) {
                let [w, h] = picture.size.map(|s| s as f32);
                let scale = (rect.width() / w.max(1.0)).min(rect.height() / h.max(1.0));
                let page = Rect::from_center_size(rect.center(), Vec2::new(w, h) * scale);
                // Paper is white in every theme.
                ui.painter().rect_filled(page, 0.0, Color32::WHITE);
                let key = ("fontcraftPrintPreview", m.generation, preview.page);
                let texture = TextureCache::texture_with(ui.ctx(), key, TextureOptions::LINEAR, || picture.as_ref().clone());
                ui.painter().image(texture, page, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
        }
        Err(why) => _ = Text::new(why).show(ui),
    }
    let last = pages.count.saturating_sub(1);
    let _ = Row::new().show(ui, |ui, _| {
        if Button::new(words.previous.clone()).enabled(preview.page > 0).show(ui).clicked() {
            preview.page = preview.page.saturating_sub(1);
        }
        let (page, count) = (preview.page.saturating_add(1).to_string(), pages.count.to_string());
        Text::new(words.page.fill(&[("page", page.as_str()), ("pages", count.as_str())])).show(ui);
        if Button::new(words.next.clone()).enabled(preview.page < last).show(ui).clicked() {
            preview.page = preview.page.saturating_add(1).min(last);
        }
    });
    Text::new(words.preview_note.clone()).shade(Shade::Muted).show(ui);
}

/// The page half: what the pages show. Edits are posted as `file.print` with what changed.
pub fn setup_view(ui: &mut Ui, m: &PrintModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<PrintWords>();
    let setup = &m.setup;
    let layouts: Vec<Entry<PageLayout>> = PageLayout::ALL.into_iter().map(|l| Entry::new(l, l.label(&words).clone())).collect();
    let picked =
        labelled(ui, &words.layout, |ui| Select::new(words.layout.clone(), Bind::given(setup.layout), &layouts).show(ui).events.committed().copied());
    if let Some(layout) = picked.flatten().filter(|l| *l != setup.layout) {
        sent.push(post(json!({"layout": layout.name()})));
    }
    let styles: Vec<Entry<usize>> = m.styles.iter().enumerate().map(|(i, (_, label))| Entry::new(i, label.clone())).collect();
    if let Some(now) = m.style() {
        let picked =
            labelled(ui, &words.style, |ui| Select::new(words.style.clone(), Bind::given(now), &styles).show(ui).events.committed().copied());
        match picked.flatten().filter(|at| *at != now).and_then(|at| m.styles.get(at)) {
            Some((Style::Master(id), _)) => sent.push(post(json!({"master": id}))),
            Some((Style::Instance(name), _)) => sent.push(post(json!({"instance": name}))),
            Some((Style::Default, _)) | None => {}
        }
    }
    let papers: Vec<Entry<Paper>> = Paper::ALL.into_iter().map(|p| Entry::new(p, p.label(&words).clone())).collect();
    let picked =
        labelled(ui, &words.paper, |ui| Select::new(words.paper.clone(), Bind::given(setup.paper), &papers).show(ui).events.committed().copied());
    if let Some(paper) = picked.flatten().filter(|p| *p != setup.paper) {
        sent.push(post(json!({"paper": paper.name()})));
    }
    for (label, on, key) in [(&words.landscape, setup.landscape, "landscape"), (&words.header, setup.header, "header")] {
        let out = Checkbox::new(Bind::given(on)).label(label.clone()).id_salt(("fontcraftPrint", key)).show(ui);
        if let Some(next) = out.events.committed().copied().filter(|next| *next != on) {
            let mut p = Map::new();
            p.insert(key.into(), json!(next));
            sent.push(post(Value::Object(p)));
        }
    }
    let only = !setup.glyphs.is_empty();
    let out = Checkbox::new(Bind::given(only)).label(words.selected_only.clone()).enabled(only || m.selected > 0).show(ui);
    if let Some(next) = out.events.committed().copied().filter(|next| *next != only) {
        sent.push(post(json!({"glyphs": if next { "selected" } else { "all" }})));
    }
    if setup.layout.has_text() {
        let typed = labelled(ui, &words.text, |ui| {
            let out = TextField::new(words.text.clone(), Bind::given(setup.text.clone())).lines(TEXT_LINES).limit(MAX_TEXT).show(ui);
            out.events.value().cloned()
        });
        if let Some(text) = typed.flatten().filter(|t| *t != setup.text) {
            sent.push(post(json!({"text": text})));
        }
    }
    if setup.layout == PageLayout::SampleText {
        let typed = labelled(ui, &words.point_size, |ui| {
            let field = NumberField::new(words.point_size.clone(), Bind::given(Some(setup.point_size)));
            widgets::clearable(ui, field.range(POINT_SIZES), Some(setup.point_size))
        });
        if let Some(Some(widgets::NumberEdit::Set(size))) = typed
            && POINT_SIZES.contains(&size)
        {
            sent.push(post(json!({"pointSize": size})));
        }
    }
}

/// The printer half of the dialog: the suite's print panel over the app's print service.
#[cfg(not(target_arch = "wasm32"))]
mod printer {
    use std::sync::Arc;

    use craft::print::ui::{Discover, Feedback, State};
    use craft::ui::contract::Show as _;
    use craft::ui::display::Text;
    use craft::ui::runtime::env::{Env, Label};
    use egui::Ui;

    use super::PrintWords;
    use crate::app::FontCraftApp;
    use crate::app::print_host::PrintHost;

    pub struct PrinterHalf {
        state: State,
        service: Option<Arc<dyn PrintHost>>,
    }

    impl PrinterHalf {
        pub fn new(service: Option<Arc<dyn PrintHost>>) -> Self {
            PrinterHalf { state: State::default(), service }
        }

        pub fn of(app: &FontCraftApp) -> Self {
            PrinterHalf::new(app.print_host.clone())
        }

        /// Shows the panel and the latest feedback; whether a job can start.
        pub fn show(&mut self, ui: &mut Ui) -> bool {
            let words = Env::of(ui).words::<PrintWords>();
            let Some(service) = self.service.clone() else {
                Text::new(words.no_printing.clone()).show(ui);
                return false;
            };
            let discover: Discover = Box::new(move || service.printers());
            let _ = self.state.run(ui, &discover);
            if let Some(message) = &self.state.message {
                Text::new(feedback(&words, message)).show(ui);
            }
            self.state.ready()
        }

        /// Sends `pdf`; nothing happens while a job runs.
        pub fn print(&mut self, pdf: Vec<u8>) {
            let Some(service) = self.service.clone().filter(|_| self.state.ready()) else { return };
            let job = service.print(pdf, self.state.settings.clone());
            // `ready` is false while a job runs, so the receiver is always taken.
            let _ = self.state.begin(job);
        }
    }

    fn feedback(words: &PrintWords, message: &Feedback) -> Label {
        match message {
            Feedback::FindingPrinters => words.finding.clone(),
            Feedback::NoPrinters => words.no_printers.clone(),
            Feedback::DiscoveryFailed => words.discovery_failed.clone(),
            Feedback::Submitting => words.sending.clone(),
            Feedback::Submitted(receipt) => words.sent.fill(&[("receipt", receipt.as_str())]).into(),
            Feedback::Failed(error) => words.failed.fill(&[("error", error.as_str())]).into(),
            Feedback::WorkerStopped => words.stopped.clone(),
            _ => Label::from(String::new()),
        }
    }
}

/// The browser build prints nothing.
#[cfg(target_arch = "wasm32")]
mod printer {
    use craft::ui::contract::Show as _;
    use craft::ui::display::Text;
    use craft::ui::runtime::env::Env;
    use egui::Ui;

    pub struct PrinterHalf;

    impl PrinterHalf {
        pub fn of(_: &crate::app::FontCraftApp) -> Self {
            PrinterHalf
        }

        pub fn show(&mut self, ui: &mut Ui) -> bool {
            let words = Env::of(ui).words::<super::PrintWords>();
            Text::new(words.no_printing.clone()).show(ui);
            false
        }

        pub fn print(&mut self, _: Vec<u8>) {}
    }
}

pub use printer::PrinterHalf;

/// The dialog's body: the preview, what the pages show, the printer, Print and Save as PDF….
pub fn view(ui: &mut Ui, m: &PrintModel, preview: &mut Preview, printer: &mut PrinterHalf, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<PrintWords>();
    let _ = Row::new().show(ui, |ui, _| {
        let _ = Stack::new().show(ui, |ui, _| preview_view(ui, m, preview));
        let _ = Stack::new().show(ui, |ui, _| setup_view(ui, m, sent));
        let _ = Stack::new().show(ui, |ui, _| {
            let ready = printer.show(ui);
            let _ = Row::new().show(ui, |ui, _| {
                let can_print = ready && m.pages.is_some();
                if Button::new(words.print.clone()).enabled(can_print).show(ui).clicked()
                    && can_print
                    && let Some(pages) = &m.pages
                {
                    printer.print(pages.pdf.as_ref().clone());
                }
                if Button::new(words.save.clone()).enabled(m.pages.is_some()).show(ui).clicked() {
                    sent.push(post(json!({"saveAs": true})));
                }
            });
        });
    });
}

/// The Print dialog on the window's host.
pub struct PrintDialog {
    pub model: PrintModel,
    preview: Preview,
    printer: PrinterHalf,
    /// What the person did, until the dialog module takes it.
    outbox: Vec<Msg>,
    closing: bool,
    open: bool,
}

impl StackDialog for PrintDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftPrint")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<PrintWords>();
        let PrintDialog { model, preview, printer, outbox, open, .. } = self;
        let shown = Dialog::new(words.title.clone())
            .opened(Bind::bound(open))
            .buttons([FooterButton::new(words.done.clone(), DialogRole::Accept)])
            .show(ui, |ui, _| view(ui, model, preview, printer, outbox));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// Opens, refreshes or closes the dialog to match [`PrintState`], and posts what the person did.
pub fn run_dialog(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<PrintDialog>().is_some() && app.printing.open {
        app.sent.push(post(json!({"close": true})));
        app.printing.open = false;
    }
    let model = PrintModel::of(app);
    match (host.get_mut::<PrintDialog>(), model) {
        (Some(dialog), model) => {
            app.sent.append(&mut dialog.outbox);
            match model {
                Some(model) => dialog.model = model,
                // Closed by a command, or the document is gone.
                None => {
                    app.printing.open = false;
                    dialog.closing = true;
                }
            }
        }
        (None, Some(model)) => {
            let printer = PrinterHalf::of(app);
            host.open(PrintDialog { model, preview: Preview::default(), printer, outbox: Vec::new(), closing: false, open: true });
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, channel};

    use craft::host::Host;
    use craft::print::ui::{Printer, Settings, Submission};
    use craft::ui::testing::Fixture;
    use fontcraft_engine::Session;

    use super::*;
    use crate::app::APP;
    use crate::app::print_host::PrintHost;
    use crate::control;

    /// A print service with one printer that keeps the jobs it is sent.
    #[derive(Default)]
    struct Fake {
        jobs: Mutex<Vec<(Vec<u8>, Settings)>>,
    }

    impl PrintHost for Fake {
        fn printers(&self) -> Receiver<Vec<Printer>> {
            let (sent, got) = channel();
            sent.send(vec![Printer::new("Office").is_default(true)]).unwrap();
            got
        }

        fn print(&self, pdf: Vec<u8>, settings: Settings) -> Submission {
            self.jobs.lock().unwrap().push((pdf, settings));
            let (sent, got) = channel();
            sent.send(Ok("request id is Office-1".into())).unwrap();
            got
        }
    }

    fn app() -> FontCraftApp {
        let mut session = Session::with_document();
        let rect = json!([{"x": 60.0, "y": 0.0}, {"x": 540.0, "y": 0.0}, {"x": 540.0, "y": 700.0}, {"x": 60.0, "y": 700.0}]);
        session.execute("path.addPath", &json!({"glyph": "A", "nodes": rect})).unwrap();
        FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()))
    }

    fn execute(app: &mut FontCraftApp, params: Value) -> Result<Value, String> {
        control::dispatch(app, "engine.execute", &json!({"command": ID, "params": params})).map_err(|e| format!("{e:?}"))
    }

    /// The command opens the dialog on the font's pages, changes them, and writes them as a PDF file.
    #[test]
    fn file_print_makes_the_pages_the_dialog_shows() {
        let mut app = app();
        execute(&mut app, json!({})).unwrap();
        let shown = control::dispatch(&mut app, "ui.inspect", &json!({})).unwrap()["print"].clone();
        assert_eq!((&shown["open"], &shown["layout"], &shown["error"]), (&json!(true), &json!("fontChart"), &Value::Null));
        assert!(shown["pages"].as_u64().unwrap() >= 1, "{shown}");

        execute(&mut app, json!({"layout": "glyphPerPage", "glyphs": ["A", "B"], "paper": "a4", "landscape": true})).unwrap();
        let pages = app.printing.pages.clone().unwrap();
        assert_eq!(pages.count, 2);
        let drawn = draw(&pages.pdf, 1).unwrap();
        assert!(drawn.size[0] > drawn.size[1], "the second page is drawn, and lies: {:?}", drawn.size);
        assert!(draw(&pages.pdf, 2).is_err(), "there is no third page");

        // What cannot be made shows why and keeps the dialog.
        execute(&mut app, json!({"master": "No such master"})).unwrap();
        assert!(app.printing.error.as_deref().unwrap().contains("No such master"));
        for bad in [json!({"layout": "poster"}), json!({"pointSize": 1e9}), json!({"glyphs": "odd"}), json!({"paper": "b0"})] {
            let before = app.printing.setup.clone();
            assert!(execute(&mut app, bad).is_err());
            assert_eq!(app.printing.setup, before, "a refused change leaves the dialog as it was");
        }

        let out = std::env::temp_dir().join(format!("fontcraft-print-{}.pdf", std::process::id()));
        let _ = std::fs::remove_file(&out);
        execute(&mut app, json!({"layout": "sampleText", "text": "AB", "master": "Regular", "output": out})).unwrap();
        assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF-"));
        let _ = std::fs::remove_file(&out);
        execute(&mut app, json!({"close": true})).unwrap();
        assert!(!app.printing.open && app.printing.pages.is_none());
        assert!(execute(&mut app, json!({"instance": "Nope"})).is_err(), "no dialog opens on pages that cannot be made");
        assert!(!app.printing.open);
    }

    /// Print hands the shown pages and the panel's settings to the print service, and its receipt is shown.
    #[test]
    fn print_sends_the_shown_pages_to_the_print_service() {
        let mut app = app();
        execute(&mut app, json!({"layout": "glyphPerPage", "glyphs": ["A", "B"]})).unwrap();
        let model = PrintModel::of(&app).unwrap();
        let pdf = model.pages.clone().unwrap().pdf;
        let fake = Arc::new(Fake::default());
        let mut printer = PrinterHalf::new(Some(fake.clone()));
        let mut preview = Preview::default();
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &model, &mut preview, &mut printer, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        f.room(Vec2::new(1600.0, 1200.0)).settle();
        let shows = |f: &Fixture, text: &str| f.texts().iter().any(|t| t.text.contains(text));
        assert!(shows(&f, "Page 1 of 2"), "the preview's place");
        let next = f.by_label("Next Page").expect("the next page");
        f.click(&next).settle();
        assert!(shows(&f, "Page 2 of 2"));

        let print = f.by_label("Print").expect("the Print button");
        f.click(&print).settle();
        let jobs = fake.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].0, *pdf, "the pages shown are the pages printed");
        assert_eq!((jobs[0].1.printer.as_str(), jobs[0].1.copies), ("Office", 1));
        drop(jobs);
        f.settle();
        assert!(shows(&f, "Sent to the printer"), "the receipt shows");

        let save = f.by_label("Save as PDF…").expect("the Save as PDF button");
        f.click(&save).settle();
        assert_eq!(posted.borrow().last(), Some(&post(json!({"saveAs": true}))));
    }
}
