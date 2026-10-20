//! The Text Preview window (`window.textPreview`): running text of any length drawn in the font,
//! for proofing passages that would slow Edit View down.
//!
//! The text is laid out with `textlayout`, as Edit View text is: characters map to glyphs by
//! Unicode value, the document's kerning applies, the Unicode Bidirectional Algorithm orders
//! mixed directions within the writing direction (`textlayout::bidi`), and lines wrap at the
//! width of the window (the height, in vertical text) at the break opportunities of the Unicode
//! Line Breaking Algorithm (`textlayout::linebreak`). Characters the font lacks are drawn in the
//! interface font. Only the lines in view are drawn.
//!
//! Horizontal text is then shaped (`textPreview.shaping`): the lines in view, and up to two
//! screens around them, go through a full preview font of the shown master (`GSUB` and `GPOS`
//! with the generated kerning and marks) with the script's default features, each run of one
//! direction on its own (`textlayout::shape_line`). Lines break on widths shaped through the same
//! font (`textlayout::measure`), kept by word in a cache that lasts while the font and the
//! document stay the same, so kerning and substitutions decide where lines end. The font is built
//! on a thread of its own:
//! at once when the window first needs it, then each time the document changes and editing has
//! paused for [`REBUILD_PAUSE`] seconds. Until a font is ready, and when it cannot be built, the
//! text shows as laid out by code point, with the reason under the text.
//!
//! Vertical text goes through the same font for its substitutions only ([`VERTICAL_FEATURES`],
//! `vert` among them), before its columns break; vertical widths, origins and kerning come from
//! the model, a glyph without a vertical width taking the ascender minus the descender.
//!
//! | Control | Does |
//! |---|---|
//! | style pop-up | `textPreview.setStyle`: a master, an instance at a master's location, or the master of the showing tab |
//! | size field | `textPreview.setFontSize` |
//! | Pin | `textPreview.togglePinned`: the window stays above the other windows |
//! | Find, Cmd+F over the window | `textPreview.find`: the find bar, with replace and its options |
//! | text field | `textPreview.setText` on every change |
//! | context menu of the text | Layout Orientation and Writing Direction (of the text, or of the field's selection) |
//!
//! Text positions in commands are character indexes; a line break is one character.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use egui::{Align2, Color32, ColorImage, FontId, Key, Modifiers, Order, Pos2, Rect, Sense, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::compile::preview::{self, PreviewKind};
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::Font;
use fontcraft_render::{Style, render_path};
use fontcraft_spacing::kerning::KernIndex;
use fontcraft_textlayout::{
    Charmap, Direction, Embedding, Item, KerningMode, LENGTH_RANGE, Layout, MAX_PREVIEW_ITEMS, Measure, Options, Placed, PreviewShaper, ShapedLine,
    Shaping, Text, WidthCache, bidi, shaping::Tag,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::composite::CompositeCache;
use crate::panels::widgets;
use crate::theme::Tokens;

type Pending = Vec<(&'static str, Value)>;

/// Most characters the preview holds.
pub const MAX_CHARS: usize = MAX_PREVIEW_ITEMS;
/// The text field edits texts up to this many characters; longer ones change through
/// `textPreview.setText` and Find and Replace.
pub const FIELD_LIMIT: usize = 100_000;
/// Allowed font sizes, in points.
pub const SIZE_RANGE: RangeInclusive<f64> = 4.0..=1_000.0;
/// The font size of a new preview, in points.
pub const DEFAULT_SIZE: f64 = 36.0;
/// Most runs with their own writing direction.
pub const MAX_RUNS: usize = fontcraft_textlayout::bidi::MAX_EMBEDDINGS;
/// Longest search or replacement, in characters.
pub const MAX_QUERY_CHARS: usize = 1_000;
/// Match ranges listed in a reply.
const MAX_LISTED: usize = 1_000;
/// Items one frame draws at most.
const MAX_DRAWN: usize = 20_000;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// How long editing must pause, in seconds, before the preview font is built again.
pub const REBUILD_PAUSE: f64 = 0.5;
/// Shaped lines kept; past it, the ones far from the view go.
const MAX_SHAPED_LINES: usize = 4_096;
/// How often a frame looks for a finished font build, in seconds.
const BUILD_POLL: f64 = 0.03;

pub const COMMANDS: &[UiCommand] = &[
    (
        "window.textPreview",
        "Text Preview",
        &["Window"],
        "",
        "{open?: bool (default true)} show or hide the Text Preview window → the preview's state",
    ),
    (
        "textPreview.setText",
        "Set Preview Text",
        &[],
        "",
        "{text: string} replace the preview text (at most 1,000,000 characters; \\r\\n and \\r become \\n). Directions set on parts of the text move with the characters around them → the preview's state",
    ),
    (
        "textPreview.setStyle",
        "Preview Font Style",
        &[],
        "",
        "{instance?: name | index into styles} the style the preview draws: a master, or an instance at a master's location. Without instance the preview follows the master of the showing tab → the preview's state, with styles",
    ),
    (
        "textPreview.setFontSize",
        "Preview Font Size",
        &[],
        "",
        "{size: number of points, 4 to 1000} the size the preview draws at → the preview's state",
    ),
    (
        "textPreview.togglePinned",
        "Pin Text Preview",
        &[],
        "",
        "{on?: bool} keep the Text Preview window above the other windows → the preview's state",
    ),
    (
        "textPreview.setLayoutOrientation",
        "Layout Orientation",
        &[],
        "",
        "{orientation: \"horizontal\" | \"vertical\"} lay the preview out in lines, or in columns from right to left → the preview's state",
    ),
    (
        "textPreview.setWritingDirection",
        "Writing Direction",
        &[],
        "",
        "{scope: \"text\" | \"selection\", direction: \"default\" | \"ltr\" | \"rtl\", range?: [start, end] (character indexes, required for selection)} set the base direction of the whole text (default: from its first letter) or of a range of it (default: as the text) → the preview's state",
    ),
    (
        "textPreview.find",
        "Find…",
        &[],
        "",
        "{open?: bool (default true), query?: string, replace?: string, options?: {ignoreCase?: bool (default true), match?: \"contains\" | \"startsWith\" | \"fullWord\", index?: number}} show the find bar and search the preview text; index picks the current match. With replace, replace every match, or only match index when it is given → the preview's state, with find.matches, find.ranges (the first 1,000) and find.replaced",
    ),
];

/// How the preview arranges its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

impl Orientation {
    pub fn name(self) -> &'static str {
        match self {
            Orientation::Horizontal => "horizontal",
            Orientation::Vertical => "vertical",
        }
    }

    fn from_name(name: &str) -> Option<Orientation> {
        [Orientation::Horizontal, Orientation::Vertical].into_iter().find(|o| o.name() == name)
    }
}

/// A base writing direction. `Default` takes it from the text (for the whole text) or from the
/// text around it (for a range).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BaseDirection {
    #[default]
    Default,
    Ltr,
    Rtl,
}

impl BaseDirection {
    pub fn name(self) -> &'static str {
        match self {
            BaseDirection::Default => "default",
            BaseDirection::Ltr => "ltr",
            BaseDirection::Rtl => "rtl",
        }
    }

    fn from_name(name: &str) -> Option<BaseDirection> {
        [BaseDirection::Default, BaseDirection::Ltr, BaseDirection::Rtl].into_iter().find(|d| d.name() == name)
    }
}

/// A range of the text with its own direction (`Ltr` or `Rtl`), in character indexes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub direction: BaseDirection,
}

/// What a search matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Matching {
    #[default]
    Contains,
    StartsWith,
    FullWord,
}

impl Matching {
    pub fn name(self) -> &'static str {
        match self {
            Matching::Contains => "contains",
            Matching::StartsWith => "startsWith",
            Matching::FullWord => "fullWord",
        }
    }

    fn from_name(name: &str) -> Option<Matching> {
        [Matching::Contains, Matching::StartsWith, Matching::FullWord].into_iter().find(|m| m.name() == name)
    }

    fn label(self) -> &'static str {
        match self {
            Matching::Contains => "Contains",
            Matching::StartsWith => "Starts With",
            Matching::FullWord => "Full Word",
        }
    }
}

/// The find bar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub ignore_case: bool,
    pub matching: Matching,
    /// The match the find bar points at.
    pub current: Option<usize>,
}

impl Default for FindState {
    fn default() -> Self {
        Self { open: false, query: String::new(), ignore_case: true, matching: Matching::Contains, current: None }
    }
}

/// The window's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextPreviewState {
    pub open: bool,
    pub text: Arc<str>,
    /// Characters in `text`.
    pub length: usize,
    /// Changes with every change of the text, unique in the process.
    pub revision: u64,
    /// The style drawn, by name; `None`: the master of the showing tab.
    pub style: Option<String>,
    /// Points per em.
    pub size: f64,
    pub pinned: bool,
    pub orientation: Orientation,
    pub direction: BaseDirection,
    /// Ranges with their own direction, sorted, not overlapping.
    pub runs: Vec<Run>,
    pub find: FindState,
    /// The window has the keyboard: it opened or was clicked last, so Cmd+F opens its find bar.
    #[serde(skip)]
    pub keyboard: bool,
    #[serde(skip)]
    pub cache: Cache,
}

impl Default for TextPreviewState {
    fn default() -> Self {
        Self {
            open: false,
            text: Arc::from(""),
            length: 0,
            revision: next_revision(),
            style: None,
            size: DEFAULT_SIZE,
            pinned: false,
            orientation: Orientation::Horizontal,
            direction: BaseDirection::Default,
            runs: Vec::new(),
            find: FindState::default(),
            keyboard: false,
            cache: Cache::default(),
        }
    }
}

/// `n` with a comma between groups of three digits.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl TextPreviewState {
    /// Replaces the text (already normalised and within the limit); runs move with the
    /// characters around them.
    fn set_text(&mut self, text: String) {
        let new_len = text.chars().count();
        self.runs = shift_runs(&self.text, self.length, &text, new_len, &self.runs);
        self.text = Arc::from(text);
        self.length = new_len;
        self.revision = next_revision();
        self.find.current = None;
    }
}

/// What the window keeps between frames: the layout, the matches, the drawn texture, and what
/// the last frame drew (tests read it).
#[derive(Default)]
pub struct Cache {
    layout_key: Option<u64>,
    text: Option<Arc<Text>>,
    layout: Option<Arc<Layout>>,
    error: Option<String>,
    matches_key: Option<u64>,
    matches: Arc<Vec<usize>>,
    texture_key: Option<u64>,
    texture: Option<TextureHandle>,
    outlines: CompositeCache,
    font: FontSlot,
    /// The widths shaped through the preview font that lines break on, for one document revision,
    /// master and font.
    widths: Option<(WidthsKey, Arc<WidthCache>)>,
    /// The shaped lines of the current layout and preview font, by line index.
    shaped_key: Option<u64>,
    shaped: HashMap<usize, Arc<ShapedLine>>,
    field: String,
    field_revision: Option<u64>,
    /// The text field's last selection, in character indexes.
    pub selection: Option<(usize, usize)>,
    pub drawn: Drawn,
    pub color: super::text_preview_color::ColorPreview,
}

// The cache is rebuilt from the state, so copies start empty and all caches compare equal.
impl Clone for Cache {
    fn clone(&self) -> Self {
        Cache::default()
    }
}

impl PartialEq for Cache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache").field("drawn", &self.drawn).finish_non_exhaustive()
    }
}

/// What the last frame drew.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawn {
    /// Glyphs drawn and their origins on the screen, in text order.
    pub glyphs: Vec<(Arc<str>, Pos2)>,
    /// Characters drawn in the interface font, with the centre of each.
    pub fallback: Vec<(char, Pos2)>,
    /// Match highlights drawn; the current match's highlight.
    pub matches: usize,
    pub current: Option<Rect>,
    /// The window's layer order (pinned: foreground).
    pub order: Option<Order>,
    /// The em, in points.
    pub em: f32,
    pub lines: usize,
    /// The master drawn.
    pub master: Option<String>,
    pub message: Option<String>,
    /// Lines drawn shaped through the preview font.
    pub shaped: usize,
    /// A preview font is being built.
    pub building: bool,
}

/// What a preview font is built for: the document's id and revision, and the master.
type FontKey = (u64, u64, String);
/// What shaped widths hold for: the document's id and revision, the master and the font.
type WidthsKey = (u64, u64, String, usize);
/// A built preview font's shaper, or why it could not be built.
type FontResult = Result<Arc<PreviewShaper>, String>;
/// Where a build on another thread leaves its result.
type Landing = Arc<Mutex<Option<FontResult>>>;

/// The preview font Text Preview shapes with.
#[derive(Default)]
struct FontSlot {
    /// The last build: its key and its result.
    built: Option<(FontKey, FontResult)>,
    /// The key asked for last, and the time (egui seconds) it was first asked for.
    wanted: Option<(FontKey, f64)>,
    /// The build under way.
    running: Option<(FontKey, Landing)>,
}

impl FontSlot {
    /// Takes the result of a finished build.
    fn collect(&mut self) {
        let Some((key, landing)) = &self.running else { return };
        let done = landing.lock().ok().and_then(|mut l| l.take());
        if let Some(result) = done {
            self.built = Some((key.clone(), result));
            self.running = None;
        }
    }

    /// Starts a build when `key` has no font yet: at once for a document or master without one,
    /// else once `key` has been asked for since [`REBUILD_PAUSE`] seconds (each edit makes a new
    /// key). Returns
    /// the shaper to draw with: the last font built for the same document and master, even when
    /// edits have made it older than the document; `None` while there is none.
    fn step(&mut self, key: &FontKey, now: f64, session: &Session, ctx: &egui::Context) -> Option<Arc<PreviewShaper>> {
        self.collect();
        let fresh = self.built.as_ref().is_some_and(|(k, _)| k == key);
        let under_way = self.running.as_ref().is_some_and(|(k, _)| k == key);
        if !fresh && !under_way {
            if self.wanted.as_ref().is_none_or(|(k, _)| k != key) {
                self.wanted = Some((key.clone(), now));
            }
            let since = self.wanted.as_ref().map_or(0.0, |(_, at)| now - at);
            let editing = self.built.as_ref().is_some_and(|(k, _)| k.0 == key.0 && k.2 == key.2);
            let wait = if editing { REBUILD_PAUSE - since } else { 0.0 };
            if self.running.is_none() && wait <= 0.0 {
                if let Some(doc) = session.active() {
                    self.running = Some((key.clone(), start_build(doc.font.clone(), key.2.clone(), build_options(session))));
                    self.collect();
                }
            } else if wait > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.clamp(BUILD_POLL, REBUILD_PAUSE)));
            }
        }
        if self.running.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(BUILD_POLL));
        }
        let (built, result) = self.built.as_ref()?;
        (built.0 == key.0 && built.2 == key.2).then(|| result.as_ref().ok().cloned()).flatten()
    }

    /// Why the font for `key`'s document and master could not be built.
    fn error(&self, key: &FontKey) -> Option<&str> {
        match &self.built {
            Some((k, Err(e))) if k.0 == key.0 && k.2 == key.2 => Some(e),
            _ => None,
        }
    }
}

/// The compile options a preview build reads: the session's glyph data and the folder the
/// source's feature code includes files from, as `font.features.compile` reads them.
pub(crate) fn build_options(session: &Session) -> fontcraft_engine::compile::Options {
    let path = session.active().and_then(|d| d.path.clone());
    fontcraft_engine::compile::Options {
        glyph_data: fontcraft_engine::compile::DataRef(session.glyph_data().ok()),
        feature_dir: feature_dir(path.as_deref()),
        include_check: fontcraft_engine::file_access::include_check(),
        ..fontcraft_engine::compile::Options::default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn feature_dir(path: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    path.and_then(fontcraft_engine::io::feature_dir).filter(|d| fontcraft_engine::file_access::check_read(d).is_ok())
}

/// The web build has no file system to read includes from.
#[cfg(target_arch = "wasm32")]
fn feature_dir(_path: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    None
}

/// The full preview font of `master` and its shaper.
fn build_font(font: &Font, master: &str, options: &fontcraft_engine::compile::Options) -> FontResult {
    let built = preview::layout_font_with(font, master, PreviewKind::Full, options).map_err(|e| format!("The text is not shaped: {e}"))?;
    let tags: Vec<fontcraft_textlayout::shaping::Tag> = built.features.iter().map(|f| f.tag).collect();
    PreviewShaper::new(Arc::<[u8]>::from(built.bytes), &built.glyph_order, &built.private_use, &tags)
        .map(Arc::new)
        .map_err(|e| format!("The text is not shaped: {e}"))
}

/// Builds the preview font on a thread of its own (here, on the web or when no thread can be
/// started); the result lands in the returned slot.
fn start_build(font: Arc<Font>, master: String, options: fontcraft_engine::compile::Options) -> Landing {
    let landing: Landing = Arc::default();
    let out = landing.clone();
    let work = move || {
        let result = fontcraft_engine::guard::catch_panic(|| build_font(&font, &master, &options))
            .unwrap_or_else(|e| Err(format!("The text is not shaped: the preview font could not be built ({e})")));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let work = Arc::new(Mutex::new(Some(work)));
        let in_thread = work.clone();
        let spawned = std::thread::Builder::new().name("text preview font".into()).spawn(move || {
            if let Some(w) = in_thread.lock().ok().and_then(|mut w| w.take()) {
                w();
            }
        });
        if spawned.is_err()
            && let Some(w) = work.lock().ok().and_then(|mut w| w.take())
        {
            w();
        }
    }
    #[cfg(target_arch = "wasm32")]
    work();
    landing
}

/// One entry of the style pop-up.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleEntry {
    pub name: String,
    /// The master whose layers it draws.
    pub master: String,
}

/// The styles the preview can draw: every master, then every instance at a master's location.
pub fn styles(font: &Font) -> Vec<StyleEntry> {
    let mut out: Vec<StyleEntry> = font.masters.iter().map(|m| StyleEntry { name: m.name.clone(), master: m.id.clone() }).collect();
    for instance in font.instances.iter() {
        if instance.kind.as_deref() == Some("variable") {
            continue;
        }
        if let Some(m) = font.masters.iter().find(|m| m.location == instance.location) {
            out.push(StyleEntry { name: instance.name.clone(), master: m.id.clone() });
        }
    }
    out
}

/// The master the preview draws: the chosen style's, else the showing tab's, else the first.
pub fn shown_master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    let tp = &app.state.panels.text_preview;
    if let Some(name) = &tp.style
        && let Some(entry) = styles(font).into_iter().find(|s| s.name == *name)
    {
        return Some(entry.master);
    }
    match app.state.tab() {
        Some(tab) => crate::canvas::view::tab_master(font, tab).map(|m| m.id.clone()),
        None => font.masters.first().map(|m| m.id.clone()),
    }
}

/// The id of the window.
pub fn window_id() -> egui::Id {
    widgets::id(("textPreview", "window"))
}

/// The id of the area that draws the text (its context menu holds orientation and direction).
pub fn area_id() -> egui::Id {
    widgets::id(("textPreview", "area"))
}

/// The id of the text field.
pub fn field_id() -> egui::Id {
    widgets::id(("textPreview", "field"))
}

fn quoted(s: &str) -> String {
    let mut q: String = s.chars().take(MAX_QUOTED).collect();
    if s.chars().nth(MAX_QUOTED).is_some() {
        q.push('…');
    }
    q
}

/// `s` with `\r\n` and `\r` as `\n`, when it holds at most [`MAX_CHARS`] characters.
fn normalised(s: &str) -> Option<String> {
    // A character takes at most four bytes, so a longer string cannot fit.
    if s.len() > MAX_CHARS.saturating_mul(4) {
        return None;
    }
    let out = s.replace("\r\n", "\n").replace('\r', "\n");
    (out.chars().count() <= MAX_CHARS).then_some(out)
}

/// Runs moved through an edit from `old` to `new`: the characters before the first difference
/// and after the last one keep their runs; a run reaching into the changed part ends (or starts)
/// at its edge.
fn shift_runs(old: &str, old_len: usize, new: &str, new_len: usize, runs: &[Run]) -> Vec<Run> {
    if runs.is_empty() {
        return Vec::new();
    }
    let prefix = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
    let room = old_len.min(new_len).saturating_sub(prefix);
    let suffix = old.chars().rev().zip(new.chars().rev()).take(room).take_while(|(a, b)| a == b).count();
    let old_tail = old_len.saturating_sub(suffix);
    let new_tail = new_len.saturating_sub(suffix);
    // At an insertion point (`prefix == old_tail`) a run that starts there moves with the text
    // after it, and a run that ends there keeps its end.
    let map = |pos: usize, end: bool| {
        if (end && pos <= prefix) || (!end && pos < old_tail && pos <= prefix) {
            pos
        } else if pos >= old_tail {
            (pos - old_tail) + new_tail
        } else if end {
            new_tail
        } else {
            prefix
        }
    };
    runs.iter()
        .map(|r| Run { start: map(r.start, false), end: map(r.end, true), direction: r.direction })
        .filter(|r| r.start < r.end && r.end <= new_len)
        .collect()
}

/// `runs` with `start..end` set to `direction` (`Default` clears it).
fn set_run(runs: &[Run], start: usize, end: usize, direction: BaseDirection) -> Vec<Run> {
    let mut out = Vec::with_capacity(runs.len() + 2);
    for r in runs {
        if r.end <= start || r.start >= end {
            out.push(*r);
            continue;
        }
        if r.start < start {
            out.push(Run { start: r.start, end: start, direction: r.direction });
        }
        if r.end > end {
            out.push(Run { start: end, end: r.end, direction: r.direction });
        }
    }
    if direction != BaseDirection::Default && start < end {
        out.push(Run { start, end, direction });
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Whether the text runs right to left: as set, or by default from its first letter.
pub fn base_is_rtl(text: &str, direction: BaseDirection) -> bool {
    match direction {
        BaseDirection::Ltr => false,
        BaseDirection::Rtl => true,
        BaseDirection::Default => text.chars().find(|c| c.is_alphabetic()).is_some_and(bidi::is_strong_rtl),
    }
}

/// The features vertical text is shaped with: those a shaper applies to vertical text by default
/// that substitute glyphs.
pub const VERTICAL_FEATURES: [Tag; 4] = [Tag::new(b"ccmp"), Tag::new(b"locl"), Tag::new(b"rlig"), Tag::new(b"vert")];

/// Lays the preview text out in `master`, lines wrapping at `line_width` font units at UAX #14
/// break opportunities: one item per character, kerning on, measured through the preview font
/// with `measure` (in vertical text, substituted with [`VERTICAL_FEATURES`]), runs of the other
/// direction and the ranges with their own direction ordered by `textlayout::bidi` within each
/// line.
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
    let vertical = direction == Direction::Vertical;
    let shaping = measure.filter(|_| vertical).map(|m| Shaping { shaper: m.shaper, active: &VERTICAL_FEATURES, localized: None });
    let options = Options {
        direction,
        kerning: KerningMode::Kerning,
        line_width,
        index,
        bidi: true,
        embeddings: &embeddings,
        break_opportunities: true,
        measure: measure.filter(|_| !vertical),
        shaping,
        ..Options::default()
    };
    let layout = fontcraft_textlayout::layout(font, master, &text, &options).map_err(|e| e.to_string())?;
    Ok((text, layout))
}

/// The start of every match of `query` in `text`, without overlaps, as character indexes.
pub fn find_matches(text: &str, query: &str, ignore_case: bool, matching: Matching) -> Vec<usize> {
    let fold = |c: char| if ignore_case { c.to_lowercase().next().unwrap_or(c) } else { c };
    let needle: Vec<char> = query.chars().map(fold).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = text.chars().map(fold).collect();
    // Knuth–Morris–Pratt, so a hostile query costs no more than the text.
    let mut fail = vec![0usize; needle.len()];
    let mut k = 0;
    for i in 1..needle.len() {
        while k > 0 && needle.get(i) != needle.get(k) {
            k = fail.get(k - 1).copied().unwrap_or(0);
        }
        if needle.get(i) == needle.get(k) {
            k += 1;
        }
        if let Some(f) = fail.get_mut(i) {
            *f = k;
        }
    }
    let word = |i: Option<usize>| i.and_then(|i| hay.get(i)).is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut out = Vec::new();
    let mut k = 0;
    for (i, c) in hay.iter().enumerate() {
        while k > 0 && needle.get(k) != Some(c) {
            k = fail.get(k - 1).copied().unwrap_or(0);
        }
        if needle.get(k) == Some(c) {
            k += 1;
        }
        if k == needle.len() {
            let start = i + 1 - k;
            let starts = !word(start.checked_sub(1));
            let ends = !word(Some(i + 1));
            let ok = match matching {
                Matching::Contains => true,
                Matching::StartsWith => starts,
                Matching::FullWord => starts && ends,
            };
            if ok {
                out.push(start);
                k = 0;
            } else {
                k = fail.get(k - 1).copied().unwrap_or(0);
            }
        }
    }
    out
}

/// `text` with the matches at `starts` (each `len` characters, not overlapping) replaced by
/// `with`, or `None` when the result would hold more than `limit` characters. The length is
/// worked out before anything is built: a short text and replacement can multiply past memory.
fn replaced(text: &str, starts: &[usize], len: usize, with: &str, limit: usize) -> Option<String> {
    let removed = starts.len().saturating_mul(len);
    let added = starts.len().saturating_mul(with.chars().count());
    if text.chars().count().saturating_sub(removed).saturating_add(added) > limit {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut next = starts.iter().peekable();
    let mut skip = 0;
    for (i, c) in text.chars().enumerate() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if next.next_if(|s| **s == i).is_some() {
            out.push_str(with);
            skip = len.saturating_sub(1);
            continue;
        }
        out.push(c);
    }
    Some(out)
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "window.textPreview" => {
            let open = args.flag("open")?.unwrap_or(true);
            let tp = &mut app.state.panels.text_preview;
            tp.keyboard = open;
            tp.open = open;
        }
        "textPreview.setText" => {
            let text = args.string("text")?.ok_or_else(|| args.err("`text` is required"))?;
            let text = normalised(text).ok_or_else(|| args.err(format!("the text holds at most {MAX_CHARS} characters")))?;
            app.state.panels.text_preview.set_text(text);
        }
        "textPreview.setStyle" => {
            let list = app.session.font().map(styles).unwrap_or_default();
            let style = match args.get("instance") {
                None => None,
                Some(Value::String(name)) => Some(
                    list.iter()
                        .find(|s| s.name == *name)
                        .map(|s| s.name.clone())
                        .ok_or_else(|| args.err(format!("no style is named `{}`", quoted(name))))?,
                ),
                Some(v) => {
                    let i = v.as_u64().ok_or_else(|| args.err("`instance` must be a style name or an index"))?;
                    let entry = usize::try_from(i).ok().and_then(|i| list.get(i));
                    Some(entry.map(|s| s.name.clone()).ok_or_else(|| args.err(format!("there are {} styles, no style {i}", list.len())))?)
                }
            };
            app.state.panels.text_preview.style = style;
        }
        "textPreview.setFontSize" => {
            let size = args.number("size")?.ok_or_else(|| args.err("`size` is required"))?;
            if !SIZE_RANGE.contains(&size) {
                return Err(args.err(format!("`size` lies between {} and {} points", SIZE_RANGE.start(), SIZE_RANGE.end())));
            }
            app.state.panels.text_preview.size = size;
        }
        "textPreview.togglePinned" => {
            let tp = &mut app.state.panels.text_preview;
            tp.pinned = args.toggle(tp.pinned)?;
        }
        "textPreview.setLayoutOrientation" => {
            let name = args.string("orientation")?.ok_or_else(|| args.err("`orientation` is required"))?;
            let orientation = Orientation::from_name(name).ok_or_else(|| args.err("`orientation` must be horizontal or vertical"))?;
            app.state.panels.text_preview.orientation = orientation;
        }
        "textPreview.setWritingDirection" => writing_direction(app, args)?,
        "textPreview.find" => return find(app, args),
        _ => return Err(args.err("unknown command")),
    }
    Ok(summary(app))
}

fn writing_direction(app: &mut FontcraftApp, args: &Args) -> Result<(), String> {
    let scope = args.string("scope")?.ok_or_else(|| args.err("`scope` is required"))?;
    let name = args.string("direction")?.ok_or_else(|| args.err("`direction` is required"))?;
    let direction = BaseDirection::from_name(name).ok_or_else(|| args.err("`direction` must be default, ltr or rtl"))?;
    let tp = &mut app.state.panels.text_preview;
    match scope {
        "text" => tp.direction = direction,
        "selection" => {
            let [a, b] = args.numbers::<2>("range")?.ok_or_else(|| args.err("`range` is required for the selection"))?;
            let whole = |v: f64| (v >= 0.0 && v.fract() == 0.0 && v <= tp.length as f64).then_some(v as usize);
            let (Some(start), Some(end)) = (whole(a), whole(b)) else {
                return Err(args.err(format!("`range` must be two character indexes from 0 to {}", tp.length)));
            };
            if start > end {
                return Err(args.err("`range` must not end before it starts"));
            }
            let runs = set_run(&tp.runs, start, end, direction);
            if runs.len() > MAX_RUNS {
                return Err(args.err(format!("the text holds at most {MAX_RUNS} ranges with their own direction")));
            }
            tp.runs = runs;
        }
        _ => return Err(args.err("`scope` must be text or selection")),
    }
    Ok(())
}

fn find(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let open = args.flag("open")?.unwrap_or(true);
    let short = |key: &str| -> Result<Option<String>, String> {
        match args.string(key)? {
            Some(s) if s.chars().count() > MAX_QUERY_CHARS => Err(args.err(format!("`{key}` holds at most {MAX_QUERY_CHARS} characters"))),
            other => Ok(other.map(str::to_string)),
        }
    };
    let query = short("query")?;
    let replace = short("replace")?;
    let empty = Map::new();
    let options = match args.get("options") {
        None => &empty,
        Some(Value::Object(m)) => m,
        Some(_) => return Err(args.err("`options` must be an object")),
    };
    let tp = &app.state.panels.text_preview;
    let ignore_case = match options.get("ignoreCase").filter(|v| !v.is_null()) {
        None => tp.find.ignore_case,
        Some(v) => v.as_bool().ok_or_else(|| args.err("`options.ignoreCase` must be true or false"))?,
    };
    let matching = match options.get("match").filter(|v| !v.is_null()) {
        None => tp.find.matching,
        Some(v) => v.as_str().and_then(Matching::from_name).ok_or_else(|| args.err("`options.match` must be contains, startsWith or fullWord"))?,
    };
    let index = match options.get("index").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| args.err("`options.index` must be a whole number"))?),
    };
    let query = query.unwrap_or_else(|| tp.find.query.clone());
    let qlen = query.chars().count();
    let starts = find_matches(&tp.text, &query, ignore_case, matching);
    if let Some(i) = index
        && i >= starts.len()
    {
        return Err(args.err(format!("there are {} matches, no match {i}", starts.len())));
    }
    let mut replaced_count = 0;
    let mut new_text = None;
    if let Some(with) = &replace {
        let chosen: Vec<usize> = match index {
            Some(i) => starts.get(i).copied().into_iter().collect(),
            None => starts.clone(),
        };
        if !chosen.is_empty() {
            let text = replaced(&tp.text, &chosen, qlen, with, MAX_CHARS)
                .ok_or_else(|| args.err(format!("replacing would make the text longer than {MAX_CHARS} characters")))?;
            replaced_count = chosen.len();
            new_text = Some(text);
        }
    }
    let tp = &mut app.state.panels.text_preview;
    if let Some(text) = new_text {
        tp.set_text(text);
    }
    tp.find.open = open;
    tp.find.query = query;
    tp.find.ignore_case = ignore_case;
    tp.find.matching = matching;
    let starts = if replace.is_some() { find_matches(&tp.text, &tp.find.query, ignore_case, matching) } else { starts };
    tp.find.current = match (&replace, index) {
        (Some(_), _) => None,
        (None, Some(i)) => Some(i),
        (None, None) => (!starts.is_empty()).then_some(0),
    };
    let mut out = summary(app);
    if let Some(find) = out.get_mut("find").and_then(Value::as_object_mut) {
        find.insert("matches".into(), json!(starts.len()));
        find.insert("ranges".into(), json!(starts.iter().take(MAX_LISTED).map(|s| [*s, s + qlen]).collect::<Vec<_>>()));
        find.insert("replaced".into(), json!(replaced_count));
    }
    Ok(out)
}

/// The preview's state as commands reply with it.
fn summary(app: &FontcraftApp) -> Value {
    let tp = &app.state.panels.text_preview;
    let styles: Vec<String> = app.session.font().map(styles).unwrap_or_default().into_iter().map(|s| s.name).collect();
    json!({
        "open": tp.open,
        "length": tp.length,
        "style": tp.style,
        "styles": styles,
        "master": shown_master(app),
        "size": tp.size,
        "pinned": tp.pinned,
        "orientation": tp.orientation.name(),
        "direction": tp.direction.name(),
        "rtl": base_is_rtl(&tp.text, tp.direction),
        "runs": tp.runs.iter().map(|r| json!({"start": r.start, "end": r.end, "direction": r.direction.name()})).collect::<Vec<_>>(),
        "find": {
            "open": tp.find.open,
            "query": tp.find.query,
            "ignoreCase": tp.find.ignore_case,
            "match": tp.find.matching.name(),
            "current": tp.find.current,
        },
    })
}

/// Where the text sits on the screen: a font-unit point (x, y) of the layout is at
/// (`ax` + x·`scale`, `ay` − y·`scale`).
#[derive(Clone, Copy)]
struct Placement {
    ax: f32,
    ay: f32,
    scale: f32,
}

impl Placement {
    fn screen(self, x: f64, y: f64) -> Pos2 {
        pos2(self.ax + x as f32 * self.scale, self.ay - y as f32 * self.scale)
    }

    fn affine(self) -> Affine {
        let s = f64::from(self.scale);
        Affine::new([s, 0.0, 0.0, -s, f64::from(self.ax), f64::from(self.ay)])
    }
}

/// Cmd+F while the window has the keyboard opens its find bar; it runs before the menu
/// shortcuts, where Cmd+F is the Edit View's and Font View's Find. A modal dialog keeps it.
pub fn find_shortcut(app: &FontcraftApp, ctx: &egui::Context, pending: &mut Pending) {
    let tp = &app.state.panels.text_preview;
    if tp.open && tp.keyboard && app.dialog.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
        pending.push(("textPreview.find", json!({})));
    }
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.text_preview.open {
        return;
    }
    let master = shown_master(app);
    let index = app.session.kern_index().ok();
    let doc = app.session.active().map(|d| (d.id, d.revision));
    let style_list = app.session.font().map(styles).unwrap_or_default();
    let FontcraftApp { session, state, tokens, .. } = app;
    let font = session.font().ok();
    let tp = &mut state.panels.text_preview;
    let order = if tp.pinned { Order::Foreground } else { Order::Middle };
    let mut open = true;
    let ctx = ui.ctx().clone();
    let shown = egui::Window::new("Text Preview")
        .id(window_id())
        .open(&mut open)
        .order(order)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.text_preview_width, tokens.text_preview_height))
        .pivot(Align2::CENTER_TOP)
        .default_pos(pos2(ctx.content_rect().center().x, tokens.dialog_top))
        .show(&ctx, |ui| {
            top_row(ui, tp, tokens, &style_list, pending);
            if tp.find.open {
                find_bar(ui, tp, tokens, pending);
            }
            text_field(ui, tp, tokens, pending);
            ui.add_space(tokens.dialog_spacing);
            match (font, master.as_deref(), doc) {
                (Some(font), Some(master), Some(doc)) => area(ui, tp, tokens, session, font, master, doc, index.as_deref(), pending),
                _ => {
                    ui.label(egui::RichText::new("No font is open").color(tokens.text_dim));
                    tp.cache.drawn = Drawn { message: Some("No font is open".into()), ..Drawn::default() };
                }
            }
            tp.cache.drawn.order = Some(order);
        });
    // A press inside the window gives it the keyboard; a press anywhere else takes it away.
    if let (Some(shown), Some(at)) = (shown, ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.press_origin()).flatten())) {
        tp.keyboard = shown.response.rect.contains(at);
    }
    if !open {
        pending.push(("window.textPreview", json!({ "open": false })));
    }
}

fn top_row(ui: &mut egui::Ui, tp: &TextPreviewState, tokens: &Tokens, styles: &[StyleEntry], pending: &mut Pending) {
    ui.horizontal(|ui| {
        let following = "Edit View master";
        let shown = tp.style.as_deref().unwrap_or(following);
        egui::ComboBox::from_id_salt(widgets::id(("textPreview", "style"))).selected_text(shown).width(tokens.dialog_field_width * 2.0).show_ui(
            ui,
            |ui| {
                if ui.selectable_label(tp.style.is_none(), following).clicked() {
                    pending.push(("textPreview.setStyle", json!({})));
                }
                for (i, s) in styles.iter().enumerate() {
                    if ui.selectable_label(tp.style.as_deref() == Some(s.name.as_str()), &s.name).clicked() {
                        pending.push(("textPreview.setStyle", json!({ "instance": i })));
                    }
                }
            },
        );
        let mut size = tp.size;
        let field = egui::DragValue::new(&mut size).range(SIZE_RANGE).speed(0.5).suffix(" pt");
        if ui.add(field).changed() && size != tp.size {
            pending.push(("textPreview.setFontSize", json!({ "size": size })));
        }
        if ui.selectable_label(tp.pinned, "Pin").on_hover_text("Keep this window above the others").clicked() {
            pending.push(("textPreview.togglePinned", json!({})));
        }
        if ui.selectable_label(tp.find.open, "Find").clicked() {
            pending.push(("textPreview.find", json!({ "open": !tp.find.open })));
        }
    });
}

fn find_bar(ui: &mut egui::Ui, tp: &TextPreviewState, tokens: &Tokens, pending: &mut Pending) {
    let count = tp.cache.matches.len();
    ui.horizontal(|ui| {
        let mut query = tp.find.query.clone();
        let field =
            egui::TextEdit::singleline(&mut query).hint_text("Find").char_limit(MAX_QUERY_CHARS).desired_width(tokens.dialog_field_width * 2.0);
        if ui.add(field).changed() {
            pending.push(("textPreview.find", json!({ "query": query })));
        }
        ui.menu_button("Options", |ui| {
            let mut ignore = tp.find.ignore_case;
            if ui.checkbox(&mut ignore, "Ignore Case").changed() {
                pending.push(("textPreview.find", json!({ "options": {"ignoreCase": ignore} })));
            }
            ui.separator();
            for m in [Matching::Contains, Matching::StartsWith, Matching::FullWord] {
                if ui.radio(tp.find.matching == m, m.label()).clicked() {
                    pending.push(("textPreview.find", json!({ "options": {"match": m.name()} })));
                }
            }
        });
        let step = |forward: bool| {
            let current = tp.find.current.unwrap_or(0);
            let next = if forward { (current + 1) % count.max(1) } else { current.checked_sub(1).unwrap_or(count.saturating_sub(1)) };
            ("textPreview.find", json!({ "options": {"index": next} }))
        };
        if ui.add_enabled(count > 0, egui::Button::new("<")).clicked() {
            pending.push(step(false));
        }
        if ui.add_enabled(count > 0, egui::Button::new(">")).clicked() {
            pending.push(step(true));
        }
        let label = match (count, tp.find.query.is_empty()) {
            (_, true) => String::new(),
            (1, false) => "1 match".to_string(),
            (n, false) => format!("{n} matches"),
        };
        ui.label(egui::RichText::new(label).color(tokens.text_dim));
    });
    ui.horizontal(|ui| {
        let id = widgets::id(("textPreview", "replace"));
        let mut with = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
        let field =
            egui::TextEdit::singleline(&mut with).hint_text("Replace").char_limit(MAX_QUERY_CHARS).desired_width(tokens.dialog_field_width * 2.0);
        if ui.add(field).changed() {
            ui.data_mut(|d| d.insert_temp(id, with.clone()));
        }
        if ui.add_enabled(tp.find.current.is_some(), egui::Button::new("Replace")).clicked() {
            pending.push(("textPreview.find", json!({ "replace": with, "options": {"index": tp.find.current} })));
        }
        if ui.add_enabled(count > 0, egui::Button::new("All")).clicked() {
            pending.push(("textPreview.find", json!({ "replace": with })));
        }
        if ui.button("Done").clicked() {
            pending.push(("textPreview.find", json!({ "open": false })));
        }
    });
}

fn text_field(ui: &mut egui::Ui, tp: &mut TextPreviewState, tokens: &Tokens, pending: &mut Pending) {
    if tp.length > FIELD_LIMIT {
        let note =
            format!("{} characters: the field edits texts of up to {}; Find and Replace change any text.", grouped(tp.length), grouped(FIELD_LIMIT));
        ui.label(egui::RichText::new(note).color(tokens.text_dim));
        return;
    }
    let cache = &mut tp.cache;
    if cache.field_revision != Some(tp.revision) {
        cache.field = tp.text.to_string();
        cache.field_revision = Some(tp.revision);
    }
    egui::ScrollArea::vertical().id_salt(widgets::id(("textPreview", "fieldScroll"))).max_height(tokens.text_preview_field_height).show(ui, |ui| {
        let edit = egui::TextEdit::multiline(&mut cache.field).id(field_id()).char_limit(FIELD_LIMIT).desired_rows(3).desired_width(f32::INFINITY);
        let output = edit.show(ui);
        if let Some(range) = output.cursor_range {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            cache.selection = Some((a.min(b), a.max(b)));
        }
        if output.response.response.changed() {
            pending.push(("textPreview.setText", json!({ "text": cache.field.clone() })));
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn area(
    ui: &mut egui::Ui,
    tp: &mut TextPreviewState,
    tokens: &Tokens,
    session: &Session,
    font: &Font,
    master: &str,
    doc: (u64, u64),
    index: Option<&KernIndex>,
    pending: &mut Pending,
) {
    let pad = tokens.text_preview_padding;
    let avail = ui.available_size();
    let size = vec2(avail.x.max(pad * 8.0), avail.y.max(pad * 8.0));
    let upm = f64::from(font.units_per_em.max(1));
    let scale = (tp.size / upm) as f32;
    let vertical = tp.orientation == Orientation::Vertical;
    // The line length, in whole steps of 8 points so that resizing re-lays out only now and then.
    let span = if vertical { size.y } else { size.x } - 2.0 * pad - tokens.text_preview_padding;
    let span = ((span / 8.0).floor() * 8.0).max(8.0);
    let line_width = f64::from(span) / f64::from(scale);

    // The preview font, and the widths shaped through it that lines break on.
    let now = ui.input(|i| i.time);
    let font_key: FontKey = (doc.0, doc.1, master.to_string());
    let shaper = tp.cache.font.step(&font_key, now, session, ui.ctx());
    let shaper_id = shaper.as_ref().map(|s| Arc::as_ptr(s) as usize);
    match shaper_id {
        None => tp.cache.widths = None,
        Some(id) => {
            let widths_key: WidthsKey = (doc.0, doc.1, master.to_string(), id);
            if tp.cache.widths.as_ref().is_none_or(|(k, _)| *k != widths_key) {
                tp.cache.widths = Some((widths_key, Arc::new(WidthCache::new())));
            }
        }
    }

    let mut h = std::collections::hash_map::DefaultHasher::new();
    (doc, tp.revision, master, tp.orientation, tp.direction, &tp.runs, line_width.to_bits(), shaper_id).hash(&mut h);
    let key = h.finish();
    if tp.cache.layout_key != Some(key) {
        tp.cache.layout_key = Some(key);
        let widths = tp.cache.widths.as_ref().map(|(_, c)| c.clone());
        let measure = shaper.as_deref().zip(widths.as_deref()).map(|(shaper, cache)| Measure { shaper, cache });
        match lay_out(font, master, tp, line_width, index, measure) {
            Ok((text, layout)) => {
                tp.cache.text = Some(Arc::new(text));
                tp.cache.layout = Some(Arc::new(layout));
                tp.cache.error = None;
            }
            Err(e) => {
                tp.cache.text = None;
                tp.cache.layout = None;
                tp.cache.error = Some(e);
            }
        }
    }
    let mut mh = std::collections::hash_map::DefaultHasher::new();
    (tp.revision, &tp.find.query, tp.find.ignore_case, tp.find.matching).hash(&mut mh);
    let mkey = mh.finish();
    if tp.cache.matches_key != Some(mkey) {
        tp.cache.matches_key = Some(mkey);
        tp.cache.matches = Arc::new(find_matches(&tp.text, &tp.find.query, tp.find.ignore_case, tp.find.matching));
    }

    // The shaped lines kept for this layout and font.
    let mut sh = std::collections::hash_map::DefaultHasher::new();
    (key, shaper_id).hash(&mut sh);
    let shaped_key = sh.finish();
    if tp.cache.shaped_key != Some(shaped_key) {
        tp.cache.shaped_key = Some(shaped_key);
        tp.cache.shaped.clear();
    }

    let message = tp.cache.error.clone().or_else(|| tp.cache.font.error(&font_key).map(str::to_string));
    let mut drawn =
        Drawn { em: tp.size as f32, master: Some(master.to_string()), message, building: tp.cache.font.running.is_some(), ..Drawn::default() };
    let (Some(layout), Some(text)) = (tp.cache.layout.clone(), tp.cache.text.clone()) else {
        if let Some(e) = &tp.cache.error {
            ui.label(egui::RichText::new(e).color(tokens.text_dim));
        }
        tp.cache.drawn = drawn;
        return;
    };
    drawn.lines = layout.lines.len();
    let lh = (layout.line_height as f32) * scale;
    let extent = pad * 2.0 + lh * layout.lines.len().max(1) as f32;
    let content = if vertical { vec2(extent.max(size.x), size.y) } else { vec2(size.x, extent.max(size.y)) };
    // Matches are highlighted only while the find bar shows.
    let matches = if tp.find.open { tp.cache.matches.clone() } else { Arc::default() };
    let qlen = tp.find.query.chars().count();
    let current = tp.find.current;
    let length = tp.length;
    let ppp = ui.ctx().pixels_per_point();
    tp.cache.color.begin(session, font, tp.style.as_deref(), master, doc, now, ui.ctx(), f64::from(scale) * f64::from(ppp), tokens.glyph_fill);

    let scroll = egui::ScrollArea::both().id_salt(widgets::id(("textPreview", "scroll"))).auto_shrink([false, false]).max_height(size.y);
    scroll.stick_to_right(vertical).show_viewport(ui, |ui, viewport| {
        ui.set_width(content.x);
        ui.set_height(content.y);
        let origin = ui.max_rect().min;
        let visible = viewport.translate(origin.to_vec2());
        let response = ui.interact(visible, area_id(), Sense::click());
        let painter = ui.painter_at(visible);
        painter.rect_filled(visible, 0.0, tokens.canvas);
        let asc = layout.ascender as f32;
        let place = match layout.direction {
            Direction::Ltr => Placement { ax: origin.x + pad, ay: origin.y + pad + asc * scale, scale },
            Direction::Rtl => Placement { ax: origin.x + content.x - pad, ay: origin.y + pad + asc * scale, scale },
            Direction::Vertical => Placement { ax: origin.x + content.x - pad - 0.5 * lh, ay: origin.y + pad, scale },
        };
        // The lines in view, with one more on each side for ascenders and descenders.
        let (lo, hi) = if vertical {
            (content.x - pad - viewport.max.x, content.x - pad - viewport.min.x)
        } else {
            (viewport.min.y - pad, viewport.max.y - pad)
        };
        let first = ((lo / lh).floor() - 1.0).max(0.0) as usize;
        let last = (((hi / lh).ceil() + 1.0).max(0.0) as usize).min(layout.lines.len());

        // Shaping: the lines in view first, then up to a screen's worth more per frame from the
        // two screens around them.
        let mut shaping_error = None;
        if let Some(shaper) = shaper.as_deref().filter(|_| !vertical) {
            let page = last.saturating_sub(first).max(1);
            let around = first.saturating_sub(2 * page)..last.saturating_add(2 * page).min(layout.lines.len());
            if tp.cache.shaped.len() > MAX_SHAPED_LINES {
                tp.cache.shaped.retain(|l, _| around.contains(l));
            }
            let mut budget = page;
            let mut more = false;
            let wanted = (first..last).chain(around.clone().filter(|l| !(first..last).contains(l)));
            for (n, l) in wanted.enumerate() {
                if tp.cache.shaped.contains_key(&l) {
                    continue;
                }
                if n >= last.saturating_sub(first) {
                    if budget == 0 {
                        more = true;
                        break;
                    }
                    budget -= 1;
                }
                if let Some(line) = fontcraft_textlayout::shape_line(font, master, &text, &layout, l, shaper) {
                    tp.cache.shaped.insert(l, Arc::new(line));
                }
            }
            if more {
                ui.ctx().request_repaint();
            }
            shaping_error = (first..last).find_map(|l| tp.cache.shaped.get(&l).and_then(|s| s.error.clone()));
        }
        let shaped = &tp.cache.shaped;
        // An item as drawn: from its shaped line when there is one.
        let placed_of = |i: usize| -> Option<&Placed> {
            let p = layout.placed.get(i)?;
            match (shaped.get(&p.line), layout.lines.get(p.line)) {
                (Some(s), Some(line)) => s.placed.get(i.checked_sub(line.items.start)?),
                _ => Some(p),
            }
        };
        let line_box = |i: usize| -> Option<Rect> {
            let p = placed_of(i)?;
            let line = layout.lines.get(p.line)?;
            Some(match layout.direction {
                Direction::Vertical => Rect::from_two_pos(
                    place.screen(line.baseline - layout.line_height / 2.0, -p.offset),
                    place.screen(line.baseline + layout.line_height / 2.0, -p.offset - p.advance),
                ),
                Direction::Ltr | Direction::Rtl => {
                    let left = if layout.direction == Direction::Rtl { -p.offset - p.advance } else { p.offset };
                    Rect::from_two_pos(
                        place.screen(left, line.baseline + layout.ascender),
                        place.screen(left + p.advance, line.baseline + layout.ascender - layout.line_height),
                    )
                }
            })
        };
        let items = match (layout.lines.get(first), last.checked_sub(1).and_then(|l| layout.lines.get(l))) {
            (Some(a), Some(b)) if first < last => a.items.start..b.items.end.min(a.items.start.saturating_add(MAX_DRAWN)),
            _ => 0..0,
        };
        // Match highlights, behind the glyphs.
        let shown = matches.partition_point(|s| s + qlen <= items.start)..matches.partition_point(|s| *s < items.end);
        for (k, start) in matches.get(shown.clone()).unwrap_or_default().iter().enumerate() {
            let is_current = current == Some(shown.start + k);
            let colour = if is_current { tokens.text_preview_current } else { tokens.text_preview_match };
            let mut whole: Option<Rect> = None;
            for i in *start..(start + qlen).min(length) {
                if let Some(r) = line_box(i) {
                    painter.rect_filled(r, 0.0, colour);
                    whole = Some(whole.map_or(r, |w| w.union(r)));
                }
            }
            drawn.matches += 1;
            if is_current {
                drawn.current = whole;
            }
        }
        // Vertical text is shaped with the layout.
        drawn.shaped =
            if vertical && shaper.is_some() { last.saturating_sub(first) } else { (first..last).filter(|l| shaped.contains_key(l)).count() };
        // Glyph outlines, filled into one texture of the area in view.
        let mut outline = BezPath::new();
        let mut add = |name: &str, x: f64, y: f64, drawn: &mut Drawn| {
            if tp.cache.color.take(name, place.screen(x, y)) {
                return drawn.glyphs.push((Arc::from(name), place.screen(x, y)));
            }
            let Some(layer) = font.glyph(name).and_then(|g| g.layer(master)) else { return };
            if let Some(composite) = tp.cache.outlines.get(doc, font, name, layer) {
                let mut glyph = composite.outline.clone();
                glyph.apply_affine(Affine::translate((x, y)));
                outline.extend(glyph.elements().iter().copied());
            }
            drawn.glyphs.push((Arc::from(name), place.screen(x, y)));
        };
        for i in items.clone() {
            let Some(p) = placed_of(i) else { break };
            match (p.glyph.as_deref(), text.items().get(i)) {
                (Some(name), _) => {
                    add(name, p.x, p.y, &mut drawn);
                    for (name, x, y) in &p.extra {
                        add(name, *x, *y, &mut drawn);
                    }
                }
                (None, Some(Item::Missing(c))) if !c.is_whitespace() && !c.is_control() => {
                    let centre = match layout.direction {
                        Direction::Vertical => place.screen(p.x + p.advance / 2.0, p.y - p.advance / 2.0),
                        Direction::Ltr | Direction::Rtl => place.screen(p.x + p.advance / 2.0, p.y + upm * 0.35),
                    };
                    painter.text(centre, Align2::CENTER_CENTER, c, FontId::proportional(tp.size as f32), tokens.text_preview_fallback);
                    drawn.fallback.push((*c, centre));
                }
                _ => {}
            }
        }
        if drawn.message.is_none() {
            drawn.message = shaping_error.or_else(|| layout.shaping_error.clone());
        }
        let mut th = std::collections::hash_map::DefaultHasher::new();
        (shaped_key, drawn.shaped, tp.cache.color.key()).hash(&mut th);
        fill(ui, &painter, &mut tp.cache, tokens, &outline, place, visible, ppp, th.finish());
        tp.cache.color.paint(ui.ctx(), &painter, visible, ppp);
        // Why the text is not shaped, over the bottom of the area.
        if let Some(note) = &drawn.message {
            let at = visible.left_bottom() + vec2(pad, -pad);
            painter.text(at, Align2::LEFT_BOTTOM, note, egui::TextStyle::Small.resolve(ui.style()), tokens.text_dim);
        }
        response.context_menu(|ui| context_menu(ui, tp, pending));
    });
    tp.cache.drawn = drawn;
}

#[allow(clippy::too_many_arguments)]
fn fill(
    ui: &egui::Ui,
    painter: &egui::Painter,
    cache: &mut Cache,
    tokens: &Tokens,
    outline: &BezPath,
    place: Placement,
    clip: Rect,
    ppp: f32,
    key: u64,
) {
    if outline.elements().is_empty() {
        return;
    }
    let (w, h) = ((clip.width() * ppp).round(), (clip.height() * ppp).round());
    if !(w >= 1.0 && h >= 1.0) {
        return;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    for v in [place.ax, place.ay, place.scale, clip.min.x, clip.min.y, w, h, ppp] {
        v.to_bits().hash(&mut hasher);
    }
    tokens.glyph_fill.to_array().hash(&mut hasher);
    let tkey = hasher.finish();
    if cache.texture_key != Some(tkey) {
        cache.texture_key = Some(tkey);
        let to_pixels = Affine::scale(f64::from(ppp)) * Affine::translate((-f64::from(clip.min.x), -f64::from(clip.min.y))) * place.affine();
        let [r, g, b, a] = tokens.glyph_fill.to_srgba_unmultiplied();
        let style = Style { ink: [r, g, b, a], background: [0, 0, 0, 0], anti_alias: true };
        cache.texture = render_path(outline, to_pixels, w as u32, h as u32, &style).ok().map(|image| {
            let colour = ColorImage::from_rgba_premultiplied([w as usize, h as usize], &image.pixels);
            ui.ctx().load_texture("text-preview-fill", colour, TextureOptions::LINEAR)
        });
    }
    if let Some(texture) = &cache.texture {
        painter.image(texture.id(), clip, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

fn context_menu(ui: &mut egui::Ui, tp: &TextPreviewState, pending: &mut Pending) {
    ui.menu_button("Layout Orientation", |ui| {
        for (o, label) in [(Orientation::Horizontal, "Horizontal"), (Orientation::Vertical, "Vertical")] {
            if ui.radio(tp.orientation == o, label).clicked() {
                pending.push(("textPreview.setLayoutOrientation", json!({ "orientation": o.name() })));
                ui.close();
            }
        }
    });
    ui.menu_button("Writing Direction", |ui| {
        let labels = [(BaseDirection::Default, "Default"), (BaseDirection::Ltr, "Left to Right"), (BaseDirection::Rtl, "Right to Left")];
        ui.label("Text");
        for (d, label) in labels {
            if ui.radio(tp.direction == d, label).clicked() {
                pending.push(("textPreview.setWritingDirection", json!({ "scope": "text", "direction": d.name() })));
                ui.close();
            }
        }
        ui.separator();
        ui.label("Selection");
        let range = tp.cache.selection.filter(|(a, b)| a < b && *b <= tp.length);
        for (d, label) in labels {
            if ui.add_enabled(range.is_some(), egui::Button::new(label)).clicked()
                && let Some((a, b)) = range
            {
                pending.push(("textPreview.setWritingDirection", json!({ "scope": "selection", "direction": d.name(), "range": [a, b] })));
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_follow_the_characters_around_an_edit() {
        let runs = [Run { start: 2, end: 4, direction: BaseDirection::Rtl }, Run { start: 6, end: 8, direction: BaseDirection::Ltr }];
        // "abCDefGH" → "abXCDefGH": both runs move one to the right.
        let moved = shift_runs("abCDefGH", 8, "abXCDefGH", 9, &runs);
        assert_eq!(moved.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(3, 5), (7, 9)]);
        // Setting a range inside a run splits it.
        let split = set_run(&runs, 3, 4, BaseDirection::Default);
        assert_eq!(split.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(), [(2, 3), (6, 8)]);
    }

    #[test]
    fn find_matches_whole_words_and_ignores_case() {
        assert_eq!(find_matches("Hat hat that", "hat", true, Matching::Contains), [0, 4, 9]);
        assert_eq!(find_matches("Hat hat that", "hat", false, Matching::Contains), [4, 9]);
        assert_eq!(find_matches("Hat hat that", "hat", true, Matching::FullWord), [0, 4]);
        assert_eq!(find_matches("aaaa", "aa", true, Matching::Contains), [0, 2]);
        assert_eq!(replaced("Hat hat that", &[0, 4], 3, "cap", 12).as_deref(), Some("cap cap that"));
        assert_eq!(replaced("Hat hat that", &[0, 4], 3, "caps", 13), None);
    }

    #[test]
    fn replace_all_refuses_a_result_past_the_limit_before_building_it() {
        // A full-length text with every character replaced by the longest replacement would
        // be a billion characters; the refusal must come from arithmetic alone.
        let text = "a".repeat(MAX_CHARS);
        let starts: Vec<usize> = (0..MAX_CHARS).collect();
        let with = "b".repeat(MAX_QUERY_CHARS);
        let started = std::time::Instant::now();
        assert_eq!(replaced(&text, &starts, 1, &with, MAX_CHARS), None);
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "refused without building: {:?}", started.elapsed());
    }
}
