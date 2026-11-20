//! The Text Preview's view model: the text laid out in the shown style, shaped, and placed for the lines around the
//! view.
//!
//! The text is laid out with `textlayout`, as Edit View text is: characters map to glyphs by Unicode value, the
//! document's kerning applies, the Unicode Bidirectional Algorithm orders mixed directions within the writing
//! direction, and lines wrap at the area's line length at the break opportunities of the Unicode Line Breaking
//! Algorithm. Characters the font lacks are placed for the interface font.
//!
//! The lines around the view are then shaped through a full preview font of the shown master ([`shaping`]); lines
//! break on widths shaped through the same font. Until a font is ready, and when it cannot be built, the text shows
//! as laid out by code point, with the reason in [`TextPreviewModel::message`].
//!
//! A font with colour layers is also compiled with its colour tables ([`color_font`]): the shown master, or the
//! instance the style pop-up names. Each colour glyph placed carries its drawing from that font, in pixels, in
//! [`Page::colors`], which the panel draws in place of its outline. What a colour drawing leaves out, and why a colour
//! glyph draws as its outline, show in [`TextPreviewModel::message`] when nothing else does.
//!
//! The projection runs on every `FontCraftApp::project`, outside the stamp, because a font build lands between
//! frames; its own keys keep an unchanged preview from being placed again.

pub mod color_font;
pub mod shaping;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::prelude::*;
use fontcraft_geom::BezPath;
use fontcraft_model::Font;
use fontcraft_render::color::{ColorOptions, FrameBudget, GlyphRaster};
use fontcraft_spacing::kerning::KernIndex;
use fontcraft_textlayout::{
    Charmap, Direction, Embedding, Item, KerningMode, LENGTH_RANGE, Layout, Measure, Options, Placed, ShapedLine, Text, WidthCache,
};

use self::color_font::{BUILD_POLL, ColorFont, Drawing, MAX_WARNINGS, warning_line};
use self::shaping::{FontKey, FontSlot};
use super::{ProjectCx, set};
use crate::shell::text_preview::{self as tp, Area, BaseDirection, FindState, MAX_CHARS, Orientation, TextPreviewState, base_is_rtl, find_matches};

/// Lines placed before the view reports which lines it shows.
const DEFAULT_LINES: usize = 40;
/// The line length, in points, before the view reports its own.
const DEFAULT_SPAN: u32 = 600;
/// Most items placed around the view.
const MAX_PLACED: usize = 60_000;
/// Shaped lines kept; past it, the ones outside the placed lines go.
const MAX_SHAPED_LINES: usize = 4_096;
/// Distinct glyph outlines kept for one document revision and master.
const MAX_OUTLINES: usize = 65_536;
/// Most distinct glyphs drawn in colour for one colour font, pixel size and text colour.
const MAX_COLOR_GLYPHS: usize = 4_096;
/// The text colour (`CPAL` palette index 65,535) before the view reports its own.
const DEFAULT_INK: [u8; 4] = [0, 0, 0, 255];

/// One glyph to draw, at its origin in layout coordinates (font units, y up).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PageGlyph {
    pub name: Arc<str>,
    pub line: usize,
    pub x: f64,
    pub y: f64,
}

/// A character the font lacks, drawn in the interface font centred at `x`, `y` (layout coordinates).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PageChar {
    pub c: char,
    pub line: usize,
    pub x: f64,
    pub y: f64,
}

/// A match highlight: one character's box in layout coordinates (`[x0, y0, x1, y1]`).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PageBox {
    pub rect: [f64; 4],
    pub line: usize,
    /// Part of the find bar's current match.
    pub current: bool,
}

/// The outlines of the glyphs placed, by name, in font units. Two sets with the same `id` hold the same outlines,
/// so comparing them costs nothing.
#[derive(Clone, Debug, Default)]
pub struct Outlines {
    pub id: u64,
    pub paths: HashMap<Arc<str>, BezPath>,
}

// Paths are kurbo values, plain data that craft cannot derive for.
impl PlainData for Outlines {}

impl PartialEq for Outlines {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// The colour drawings of the glyphs placed, by name, from the compiled colour font: `None` for a glyph drawn as its
/// outline. Two sets with the same `id` hold the same drawings.
#[derive(Clone, Debug, Default)]
pub struct Colors {
    pub id: u64,
    /// Pixels per font unit the drawings are made at; the panel draws them only at that size.
    pub pixel_scale: f64,
    pub rasters: HashMap<Arc<str>, Option<Arc<GlyphRaster>>>,
    /// Drawings of `rasters` that are `Some`.
    pub drawn: usize,
}

// The set is compared by its id.
impl PlainData for Colors {}

impl PartialEq for Colors {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// The laid-out text, placed for the lines around the view.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Page {
    /// Moves whenever anything drawn changes.
    pub key: u64,
    pub direction: PageDirection,
    /// Points per font unit.
    pub scale: f64,
    pub units_per_em: f64,
    pub line_height: f64,
    pub ascender: f64,
    /// Lines in the whole text.
    pub lines: usize,
    /// The lines placed.
    pub placed: Range<usize>,
    /// Lines of `placed` drawn shaped through the preview font.
    pub shaped: usize,
    pub glyphs: Vec<PageGlyph>,
    pub fallback: Vec<PageChar>,
    pub highlights: Vec<PageBox>,
    pub outlines: Arc<Outlines>,
    /// The glyphs placed that draw in colour from the compiled colour font, in place of their outline.
    pub colors: Arc<Colors>,
}

/// The direction lines run in.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PageDirection {
    #[default]
    Ltr,
    Rtl,
    Vertical,
}

/// What the Text Preview shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TextPreviewModel {
    /// A font is open.
    pub font: bool,
    pub text: Arc<str>,
    pub length: usize,
    pub style: Option<String>,
    /// The style pop-up's entries.
    pub styles: Vec<String>,
    pub size: f64,
    pub pinned: bool,
    pub orientation: Orientation,
    pub direction: BaseDirection,
    pub find: FindState,
    /// Matches of the find bar's query.
    pub matches: usize,
    /// The area as the projection last read it; the view reports a new one when it differs.
    pub area: Area,
    pub page: Option<Arc<Page>>,
    /// Why the text is not laid out or not shaped.
    pub message: Option<String>,
    /// A preview font or a colour font is being built.
    pub building: bool,
}

/// The view model and what the projection keeps between frames.
#[derive(Default)]
pub struct TextPreviewModels {
    pub view: Tracked<TextPreviewModel>,
    cache: Cache,
}

#[derive(Default)]
struct Cache {
    /// The time, in seconds, the last frame began (`logic`).
    now: f64,
    /// Seconds until a frame should look at the preview font again.
    wake: Option<f64>,
    font: FontSlot,
    /// Widths shaped through the preview font, for one document revision, master and font.
    widths: Option<(u64, Arc<WidthCache>)>,
    layout_key: Option<u64>,
    laid_out: Option<(Arc<Text>, Arc<Layout>)>,
    error: Option<String>,
    shaped_key: Option<u64>,
    shaped: HashMap<usize, Arc<ShapedLine>>,
    matches_key: Option<u64>,
    matches: Arc<Vec<usize>>,
    outlines_key: Option<(u64, u64, String)>,
    outlines: Arc<Outlines>,
    color: ColorFont,
    /// The colour renderer, pixel size and text colour `colors` was drawn with.
    colors_key: Option<(u64, u64, [u8; 4])>,
    colors: Arc<Colors>,
    /// Glyphs of the page are still to be drawn in colour: the budget of the last projection was spent.
    colors_pending: bool,
    /// What the colour drawings of `colors` leave out, each glyph and reason once.
    warnings: Vec<(Arc<str>, String)>,
    page_key: Option<u64>,
    page: Option<Arc<Page>>,
    model_key: Option<u64>,
}

fn hash_of(value: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// Records the frame's time and asks for a frame when a build needs looking at.
pub fn logic(models: &mut TextPreviewModels, ctx: &egui::Context) {
    models.cache.now = ctx.input(|i| i.time);
    if let Some(wait) = models.cache.wake {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.0)));
    }
}

/// Lays the preview text out in `master`, lines wrapping at `line_width` font units at UAX #14 break opportunities:
/// one item per character, kerning on, measured through the preview font with `measure`, runs of the other
/// direction and the ranges with their own direction ordered by `textlayout::bidi` within each line.
pub fn lay_out(
    font: &Font,
    master: &str,
    state: &TextPreviewState,
    line_width: f64,
    index: Option<&KernIndex>,
    measure: Option<Measure<'_>>,
) -> Result<(Text, Layout), String> {
    let map = Charmap::new(font);
    let items: Vec<Item> = state.text.chars().map(|c| map.item(c)).collect();
    let mut text = Text::with_limit(MAX_CHARS).map_err(|e| e.to_string())?;
    text.insert(0, items).map_err(|e| e.to_string())?;
    let rtl = base_is_rtl(&state.text, state.direction);
    let direction = match (state.orientation, rtl) {
        (Orientation::Vertical, _) => Direction::Vertical,
        (Orientation::Horizontal, true) => Direction::Rtl,
        (Orientation::Horizontal, false) => Direction::Ltr,
    };
    let line_width = line_width.clamp(*LENGTH_RANGE.start(), *LENGTH_RANGE.end());
    let embeddings: Vec<Embedding> = state.runs.iter().map(|r| Embedding { items: r.start..r.end, rtl: r.direction == BaseDirection::Rtl }).collect();
    let options = Options {
        direction,
        kerning: KerningMode::Kerning,
        line_width,
        index,
        bidi: true,
        embeddings: &embeddings,
        break_opportunities: true,
        measure,
        ..Options::default()
    };
    let layout = fontcraft_textlayout::layout(font, master, &text, &options).map_err(|e| e.to_string())?;
    Ok((text, layout))
}

/// Projects the Text Preview while its panel shows.
pub fn project(models: &mut TextPreviewModels, cx: &mut ProjectCx<'_>, host: &Host) {
    let view = cx.view;
    if !tp::shows(view) {
        models.cache.wake = None;
        return;
    }
    let state = &view.text_preview;
    let cache = &mut models.cache;
    let font_info = cx.session.font().ok().map(|font| (tp::styles(font), tp::shown_master(font, view), font.units_per_em));
    let doc = cx.session.active().map(|d| (d.id, d.revision));
    let (styles, master, upm) = match (font_info, doc) {
        (Some((styles, Some(master), upm)), Some(_)) => (styles, master, f64::from(upm.max(1))),
        _ => {
            cache.wake = None;
            let model = TextPreviewModel { font: false, ..base(state, Vec::new(), 0) };
            set(&mut models.view, model);
            cache.model_key = None;
            return;
        }
    };
    let doc = doc.unwrap_or_default();

    // The preview font, and the widths shaped through it that lines break on.
    let font_key: FontKey = (doc.0, doc.1, master.clone());
    let (shaper, wake) = cache.font.step(&font_key, cache.now, cx.session, host);
    let color_wake = match cx.session.font() {
        Ok(font) => cache.color.step(font, state.style.as_deref(), &master, doc, cache.now, cx.session, host),
        Err(_) => None,
    };
    cache.wake = earliest(wake, color_wake);
    let shaper_id = shaper.as_ref().map(|s| Arc::as_ptr(s) as usize);
    match shaper_id {
        None => cache.widths = None,
        Some(id) => {
            let widths_key = hash_of((doc, &master, id));
            if cache.widths.as_ref().is_none_or(|(k, _)| *k != widths_key) {
                cache.widths = Some((widths_key, Arc::new(WidthCache::new())));
            }
        }
    }

    let span = if state.area.span == 0 { DEFAULT_SPAN } else { state.area.span };
    let scale = state.size / upm;
    // The colour drawings: made again for a new colour font, pixel size or text colour.
    let ppp = f64::from(f32::from_bits(state.area.ppp));
    let pixel_scale = scale * if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let ink = if state.area.ink == [0; 4] { DEFAULT_INK } else { state.area.ink };
    let colors_key = cache.color.ready().map(|made| (made, pixel_scale.to_bits(), ink));
    if cache.colors_key != colors_key {
        cache.colors_key = colors_key;
        cache.colors = Arc::new(Colors { id: cache.colors.id.wrapping_add(1), pixel_scale, ..Colors::default() });
        cache.warnings.clear();
        cache.colors_pending = false;
    }
    let line_width = f64::from(span) / scale;
    let layout_key = hash_of((doc, state.revision, &master, state.orientation, state.direction, &state.runs, line_width.to_bits(), shaper_id));
    if cache.layout_key != Some(layout_key) {
        cache.layout_key = Some(layout_key);
        let index = cx.session.kern_index().ok();
        let widths = cache.widths.as_ref().map(|(_, c)| c.clone());
        let measure = shaper.as_deref().zip(widths.as_deref()).map(|(shaper, cache)| Measure { shaper, cache });
        let result =
            cx.session.font().map_err(|e| e.to_string()).and_then(|font| lay_out(font, &master, state, line_width, index.as_deref(), measure));
        match result {
            Ok((text, layout)) => {
                cache.laid_out = Some((Arc::new(text), Arc::new(layout)));
                cache.error = None;
            }
            Err(e) => {
                cache.laid_out = None;
                cache.error = Some(e);
            }
        }
    }
    let matches_key = hash_of((state.revision, &state.find.query, state.find.ignore_case, state.find.matching));
    if cache.matches_key != Some(matches_key) {
        cache.matches_key = Some(matches_key);
        cache.matches = Arc::new(find_matches(&state.text, &state.find.query, state.find.ignore_case, state.find.matching));
    }
    let shaped_key = hash_of((layout_key, shaper_id));
    if cache.shaped_key != Some(shaped_key) {
        cache.shaped_key = Some(shaped_key);
        cache.shaped.clear();
    }

    let mut message = cache.error.clone().or_else(|| cache.font.error(&font_key).map(str::to_string));
    if let Some((text, layout)) = cache.laid_out.clone()
        && let Ok(font) = cx.session.font()
    {
        let placed = placed_lines(&state.area, &layout);
        // Shaping: every line placed, once per layout and font.
        if let Some(shaper) = shaper.as_deref() {
            if cache.shaped.len() > MAX_SHAPED_LINES {
                cache.shaped.retain(|l, _| placed.contains(l));
            }
            for l in placed.clone() {
                if !cache.shaped.contains_key(&l)
                    && let Some(line) = fontcraft_textlayout::shape_line(font, &master, &text, &layout, l, shaper)
                {
                    cache.shaped.insert(l, Arc::new(line));
                }
            }
            if message.is_none() {
                message = placed.clone().find_map(|l| cache.shaped.get(&l).and_then(|s| s.error.clone()));
            }
        }
        if message.is_none() {
            message = layout.shaping_error.clone();
        }
        let outlines_key = (doc.0, doc.1, master.clone());
        if cache.outlines_key.as_ref() != Some(&outlines_key) {
            cache.outlines_key = Some(outlines_key);
            cache.outlines = Arc::new(Outlines { id: cache.outlines.id.wrapping_add(1), paths: HashMap::new() });
        }
        let shaped_lines = placed.clone().filter(|l| cache.shaped.contains_key(l)).count();
        let page_key =
            hash_of((layout_key, shaper_id, shaped_lines, &placed, matches_key, state.find.open, state.find.current, scale.to_bits(), doc))
                ^ hash_of(colors_key);
        if cache.page_key != Some(page_key) || cache.colors_pending {
            cache.page_key = Some(page_key);
            let input = PageInput {
                font,
                master: &master,
                text: &text,
                layout: &layout,
                shaped: &cache.shaped,
                placed,
                matches: if state.find.open { cache.matches.as_slice() } else { &[] },
                query_len: state.find.query.chars().count(),
                current: state.find.current,
                length: state.length,
            };
            let mut placed_page = page(&input, &mut cache.outlines, page_key, scale, upm, shaped_lines);
            let options = ColorOptions { scale: pixel_scale, palette: 0, text_color: ink };
            let drawing = ColorDrawing { font: &mut cache.color, options, warnings: &mut cache.warnings };
            cache.colors_pending = add_colors(&placed_page.glyphs, &mut cache.colors, drawing);
            placed_page.colors = cache.colors.clone();
            cache.page = Some(Arc::new(placed_page));
        }
        if cache.colors_pending {
            cache.wake = earliest(cache.wake, Some(BUILD_POLL));
        }
    } else {
        cache.page = None;
        cache.page_key = None;
    }

    if message.is_none() {
        message = cache.color.error().map(str::to_string).or_else(|| warning_line(&cache.warnings));
    }
    let building = cache.font.building() || cache.color.building();
    let style_names: Vec<&str> = styles.iter().map(|s| s.name.as_str()).collect();
    let model_key = hash_of((cache.page_key, &message, building, &style_names, state.revision, &state.style, state.size.to_bits(), state.pinned))
        ^ hash_of((state.orientation, state.direction, &state.find.query, state.find.open, state.find.current, state.find.ignore_case))
        ^ hash_of((state.find.matching, state.find.replaced, state.area, matches_key, cache.colors.id));
    if cache.model_key == Some(model_key) {
        return;
    }
    cache.model_key = Some(model_key);
    let names = styles.into_iter().map(|s| s.name).collect();
    let model = TextPreviewModel { font: true, page: cache.page.clone(), message, building, ..base(state, names, cache.matches.len()) };
    set(&mut models.view, model);
}

/// The model's copy of the view state.
fn base(state: &TextPreviewState, styles: Vec<String>, matches: usize) -> TextPreviewModel {
    TextPreviewModel {
        font: true,
        text: state.text.clone(),
        length: state.length,
        style: state.style.clone(),
        styles,
        size: state.size,
        pinned: state.pinned,
        orientation: state.orientation,
        direction: state.direction,
        find: state.find.clone(),
        matches,
        area: state.area,
        page: None,
        message: None,
        building: false,
    }
}

/// The lines to place: those the view shows and two screens on each side, at most [`MAX_PLACED`] items.
fn placed_lines(area: &Area, layout: &Layout) -> Range<usize> {
    let count = layout.lines.len();
    let (first, last) = if area.last == 0 { (0, DEFAULT_LINES) } else { (area.first, area.last) };
    let screen = last.saturating_sub(first).max(1);
    let lo = first.saturating_sub(2 * screen).min(count);
    let mut hi = last.saturating_add(2 * screen).min(count);
    let start = layout.lines.get(lo).map_or(0, |l| l.items.start);
    while hi > lo + 1 && layout.lines.get(hi - 1).is_some_and(|l| l.items.end.saturating_sub(start) > MAX_PLACED) {
        hi -= 1;
    }
    lo..hi
}

struct PageInput<'a> {
    font: &'a Font,
    master: &'a str,
    text: &'a Text,
    layout: &'a Layout,
    shaped: &'a HashMap<usize, Arc<ShapedLine>>,
    placed: Range<usize>,
    matches: &'a [usize],
    query_len: usize,
    current: Option<usize>,
    length: usize,
}

impl PageInput<'_> {
    /// An item as drawn: from its shaped line when there is one.
    fn placed_of(&self, i: usize) -> Option<&Placed> {
        let p = self.layout.placed.get(i)?;
        match (self.shaped.get(&p.line), self.layout.lines.get(p.line)) {
            (Some(s), Some(line)) => s.placed.get(i.checked_sub(line.items.start)?),
            _ => Some(p),
        }
    }

    /// The box of item `i` across its line, in layout coordinates.
    fn line_box(&self, i: usize) -> Option<(usize, [f64; 4])> {
        let p = self.placed_of(i)?;
        let layout = self.layout;
        let line = layout.lines.get(p.line)?;
        let half = layout.line_height / 2.0;
        Some((
            p.line,
            match layout.direction {
                Direction::Vertical => [line.baseline - half, -p.offset, line.baseline + half, -p.offset - p.advance],
                Direction::Ltr | Direction::Rtl => {
                    let left = if layout.direction == Direction::Rtl { -p.offset - p.advance } else { p.offset };
                    [left, line.baseline + layout.ascender, left + p.advance, line.baseline + layout.ascender - layout.line_height]
                }
            },
        ))
    }
}

/// Places the lines of `input.placed`: glyphs, the characters the font lacks and the match highlights, with the
/// outlines of the glyphs placed added to `outlines`. The page's colour drawings are left empty.
fn page(input: &PageInput<'_>, outlines: &mut Arc<Outlines>, key: u64, scale: f64, upm: f64, shaped: usize) -> Page {
    let layout = input.layout;
    let items = match (layout.lines.get(input.placed.start), input.placed.end.checked_sub(1).and_then(|l| layout.lines.get(l))) {
        (Some(a), Some(b)) if input.placed.start < input.placed.end => a.items.start..b.items.end,
        _ => 0..0,
    };
    let mut glyphs = Vec::new();
    let mut fallback = Vec::new();
    for i in items.clone() {
        let Some(p) = input.placed_of(i) else { break };
        match (p.glyph.as_ref(), input.text.items().get(i)) {
            (Some(name), _) => {
                glyphs.push(PageGlyph { name: name.clone(), line: p.line, x: p.x, y: p.y });
                for (name, x, y) in &p.extra {
                    glyphs.push(PageGlyph { name: name.clone(), line: p.line, x: *x, y: *y });
                }
            }
            (None, Some(Item::Missing(c))) if !c.is_whitespace() && !c.is_control() => {
                let (x, y) = match layout.direction {
                    Direction::Vertical => (p.x + p.advance / 2.0, p.y - p.advance / 2.0),
                    Direction::Ltr | Direction::Rtl => (p.x + p.advance / 2.0, p.y + upm * 0.35),
                };
                fallback.push(PageChar { c: *c, line: p.line, x, y });
            }
            _ => {}
        }
    }
    let missing: Vec<&Arc<str>> = glyphs.iter().map(|g| &g.name).filter(|n| !outlines.paths.contains_key(*n)).collect();
    if !missing.is_empty() {
        let set = Arc::make_mut(outlines);
        if set.paths.len() > MAX_OUTLINES {
            set.paths.clear();
        }
        for name in missing {
            let path = input.font.glyph(name).and_then(|g| input.font.outline_of(g, input.master).ok()).unwrap_or_default();
            set.paths.insert(name.clone(), path);
        }
        set.id = set.id.wrapping_add(1);
    }
    let mut highlights = Vec::new();
    let qlen = input.query_len;
    let shown = input.matches.partition_point(|s| s + qlen <= items.start)..input.matches.partition_point(|s| *s < items.end);
    for (k, start) in input.matches.get(shown.clone()).unwrap_or_default().iter().enumerate() {
        let current = input.current == Some(shown.start + k);
        for i in *start..(start + qlen).min(input.length) {
            if let Some((line, rect)) = input.line_box(i) {
                highlights.push(PageBox { rect, line, current });
            }
        }
    }
    Page {
        key,
        direction: match layout.direction {
            Direction::Ltr => PageDirection::Ltr,
            Direction::Rtl => PageDirection::Rtl,
            Direction::Vertical => PageDirection::Vertical,
        },
        scale,
        units_per_em: upm,
        line_height: layout.line_height,
        ascender: layout.ascender,
        lines: layout.lines.len(),
        placed: input.placed.clone(),
        shaped,
        glyphs,
        fallback,
        highlights,
        outlines: outlines.clone(),
        colors: Arc::default(),
    }
}

/// What [`add_colors`] draws with: the colour font, how glyphs are drawn, and where warnings go.
struct ColorDrawing<'a> {
    font: &'a mut ColorFont,
    options: ColorOptions,
    warnings: &'a mut Vec<(Arc<str>, String)>,
}

/// Adds to `colors` the drawing from the compiled colour font of each glyph of `glyphs` it has not looked at, within
/// one projection's budget. Past [`MAX_COLOR_GLYPHS`] drawings, glyphs draw as their outlines. Returns whether glyphs
/// are left to draw on a later projection.
fn add_colors(glyphs: &[PageGlyph], colors: &mut Arc<Colors>, drawing: ColorDrawing<'_>) -> bool {
    let ColorDrawing { font, options, warnings } = drawing;
    if font.ready().is_none() {
        return false;
    }
    let unseen: Vec<&Arc<str>> = glyphs.iter().map(|g| &g.name).filter(|n| !colors.rasters.contains_key(*n)).collect();
    if unseen.is_empty() {
        return false;
    }
    let set = Arc::make_mut(colors);
    if set.rasters.len() > MAX_OUTLINES {
        set.rasters.clear();
        set.drawn = 0;
    }
    let mut budget = FrameBudget::frame();
    let mut note = |glyph: &Arc<str>, warning: String| {
        if warnings.len() < MAX_WARNINGS && !warnings.iter().any(|(g, w)| g == glyph && *w == warning) {
            warnings.push((glyph.clone(), warning));
        }
    };
    let mut pending = false;
    for name in unseen {
        if set.rasters.contains_key(name) {
            continue;
        }
        let raster = if set.drawn < MAX_COLOR_GLYPHS {
            match font.glyph(name, &options, &mut budget) {
                Drawing::Color(raster) => {
                    for w in &raster.warnings {
                        note(name, w.clone());
                    }
                    Some(raster)
                }
                Drawing::Outline(why) => {
                    if let Some(why) = why {
                        note(name, format!("drawn as its outline: {why}"));
                    }
                    None
                }
                Drawing::Deferred => {
                    pending = true;
                    continue;
                }
            }
        } else {
            None
        };
        set.drawn += usize::from(raster.is_some());
        set.rasters.insert(name.clone(), raster);
    }
    set.id = set.id.wrapping_add(1);
    pending
}

/// The sooner of two waits.
fn earliest(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
pub(crate) fn set_now(models: &mut TextPreviewModels, now: f64) {
    models.cache.now = now;
}

/// Makes the colour font of the shown document and style a failed build with `reason` (tests).
#[cfg(test)]
pub(crate) fn fail_color_font(models: &mut TextPreviewModels, reason: &str) {
    models.cache.color.fail(reason);
}
