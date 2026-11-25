//! Text in the edit view: a line of glyphs around the edited glyph, and text mode, in which the Text tool types it.
//!
//! The edit view's text is view state ([`EditText`] in `ViewState::text`); the shell commands in [`commands`] change
//! it and never make an undo step. The edited glyph is the text's current glyph (the glyph after the cursor): a text
//! command that moves the cursor also makes that glyph the shown one (`ViewState::glyph`), and the edited item's own
//! layer, when it has one, the active layer ([`sync_layer`]). While another glyph is
//! shown (picked in Font View, say), the text stays as it was and the canvas shows that glyph alone.
//!
//! | Module | Holds |
//! |---|---|
//! | [`commands`] | `editView.*` text commands, `kerning.setMode`, `kerning.showPairInEditView`, `view.showGroupMembers`, the measurement line |
//! | [`tool`] | the Text tool: the canvas's active tool in text mode, which takes the whole plain keyboard |
//! | [`input`] | the Text tool's clicks, drags and keys, run in `drain` |
//! | [`paint`] | the text's paint layers: kerned gaps, selection, the other glyphs, group members, the cursor, the measurement line |
//! | [`tokens`] | their colours and sizes |
//! | [`features`] | the preview features the text is shaped with (`editView.toggleFeature` and its kin) |
//! | [`edit`] | Edit › Copy, Cut, Paste and Delete in text mode, on the text |
//!
//! Text mode and the Text tool go together: choosing the Text tool enters text mode, any other tool ends it. The
//! layout (`app::models::text`) is in text coordinates, the first line's baseline at y = 0; the canvas draws in the
//! edited glyph's font units, so the layout is shifted by the edited glyph's place in the text, and the canvas view
//! moves by the same amount when the edited glyph changes ([`after_project`]), so the text stays still on screen.

pub mod commands;
pub mod edit;
pub mod features;
pub mod input;
pub mod paint;
pub mod tokens;
pub mod tool;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::Font;
use fontcraft_textlayout::{Direction, Item, KerningMode, Text};
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, ViewState};

/// The id of the Text tool (`fontcraft_tools::TOOLS`), which stands for text mode.
pub const TEXT_TOOL: &str = "text";

/// The direction the edit view lays its text out in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WritingDirection {
    #[default]
    Ltr,
    Rtl,
    Ttb,
}

impl WritingDirection {
    pub const ALL: [WritingDirection; 3] = [WritingDirection::Ltr, WritingDirection::Rtl, WritingDirection::Ttb];

    pub fn name(self) -> &'static str {
        match self {
            WritingDirection::Ltr => "ltr",
            WritingDirection::Rtl => "rtl",
            WritingDirection::Ttb => "ttb",
        }
    }

    /// `ltr`, `rtl`, `ttb` or `vertical`.
    pub fn from_name(name: &str) -> Option<WritingDirection> {
        match name {
            "vertical" => Some(WritingDirection::Ttb),
            _ => WritingDirection::ALL.into_iter().find(|d| d.name() == name),
        }
    }

    /// The layout's and the kerning lookup's direction.
    pub fn layout(self) -> Direction {
        match self {
            WritingDirection::Ltr => Direction::Ltr,
            WritingDirection::Rtl => Direction::Rtl,
            WritingDirection::Ttb => Direction::Vertical,
        }
    }
}

/// The edit view's text and how it is laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct EditText {
    pub text: Text,
    pub direction: WritingDirection,
    pub kerning: KerningMode,
    /// View › Show Group Members.
    pub group_members: bool,
    /// View › Show Measurement Line, and the line's height above the baseline (`None`: half the x-height).
    pub measurement: bool,
    pub measurement_height: Option<f64>,
    /// Moves with every change to the text, its cursor or its layout options.
    pub revision: u64,
    /// The cursor index where a drag that selects text began.
    pub drag: Option<usize>,
    /// The preview features the text is shaped with.
    pub features: features::Features,
    /// Edit › Select Sample Text: the window and the string chosen last.
    pub samples: crate::dialogs::sample_strings::SampleStrings,
    /// Find Glyphs: the pop-up, its search and its selected results.
    pub find: crate::dialogs::find_glyphs::FindGlyphs,
    /// Text for the system clipboard and a paste waiting for the clipboard's text.
    pub clipboard: edit::TextClipboard,
    /// The Text tool is in the canvas's active-tool slot.
    slot: bool,
    /// Where the edited glyph sat in the text when the canvas last drew it.
    origin: Option<(String, [f64; 2])>,
    /// The revision the canvas last scrolled the cursor into view for.
    followed: u64,
    /// The whole text was replaced: the canvas fits it once it has drawn the new text.
    pub(super) fit_pending: bool,
}

impl Default for EditText {
    fn default() -> Self {
        EditText {
            text: Text::new(),
            direction: WritingDirection::Ltr,
            kerning: KerningMode::Kerning,
            group_members: false,
            measurement: false,
            measurement_height: None,
            revision: 0,
            drag: None,
            features: Default::default(),
            samples: Default::default(),
            find: Default::default(),
            clipboard: Default::default(),
            slot: false,
            origin: None,
            followed: 0,
            fit_pending: false,
        }
    }
}

impl EditText {
    /// The glyph being edited: the text's current glyph.
    pub fn current(&self) -> Option<&str> {
        self.text.current_glyph()
    }

    /// The glyph item `index` shows: its own, or the current glyph for a placeholder.
    pub fn item_glyph(&self, index: usize) -> Option<&str> {
        match self.text.items().get(index)? {
            Item::Glyph { name, .. } => Some(name),
            Item::Placeholder => self.text.current_glyph(),
            Item::Missing(_) | Item::LineBreak => None,
        }
    }

    /// The glyphs the text selection shows, each once, in text order and at most `limit`; empty with only a cursor.
    pub fn selected_glyphs(&self, limit: usize) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for i in self.text.selection() {
            if out.len() >= limit {
                break;
            }
            if let Some(name) = self.item_glyph(i)
                && seen.insert(name)
            {
                out.push(name.to_string());
            }
        }
        out
    }

    /// The index of the item that shows the current glyph: the item after the cursor, else the one before it, else
    /// the first item that shows it.
    pub fn edited_index(&self) -> Option<usize> {
        let current = self.text.current_glyph()?;
        let shows = |i: usize| self.item_glyph(i) == Some(current);
        let cursor = self.text.cursor();
        [Some(cursor), cursor.checked_sub(1)].into_iter().flatten().find(|&i| shows(i)).or_else(|| (0..self.text.len()).find(|&i| shows(i)))
    }

    /// Records a change.
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

/// The part of [`EditText`] each Edit View tab keeps for itself (`shell::tabs`): the text, its layout options and
/// preview features, and where its edited glyph sat when the canvas last drew it.
#[derive(Clone, Debug, PartialEq)]
pub struct TabText {
    pub text: Text,
    pub direction: WritingDirection,
    pub kerning: KerningMode,
    pub features: features::Features,
    origin: Option<(String, [f64; 2])>,
}

impl Default for TabText {
    fn default() -> Self {
        TabText { text: Text::new(), direction: WritingDirection::Ltr, kerning: KerningMode::Kerning, features: Default::default(), origin: None }
    }
}

impl TabText {
    /// A text of `items`, the cursor before the first.
    pub fn of(items: Vec<Item>) -> Result<TabText, String> {
        let mut text = Text::new();
        text.replace(0..0, items).map_err(|e| e.to_string())?;
        text.set_cursor(0);
        Ok(TabText { text, ..TabText::default() })
    }

    /// The glyphs the tab shows: the text's when its current glyph is `shown`, else `shown` alone.
    pub fn shown_glyphs(&self, shown: Option<&str>) -> Vec<String> {
        shown_glyphs(&self.text, shown)
    }
}

impl EditText {
    /// Takes the tab's part out, leaving a fresh one: the tab goes behind another.
    pub fn park(&mut self) -> TabText {
        let parked = TabText {
            text: std::mem::take(&mut self.text),
            direction: self.direction,
            kerning: self.kerning,
            features: std::mem::take(&mut self.features),
            origin: self.origin.take(),
        };
        self.direction = WritingDirection::Ltr;
        self.kerning = KerningMode::Kerning;
        self.drag = None;
        self.touch();
        parked
    }

    /// Puts a tab's part back: the tab shows again.
    pub fn restore(&mut self, tab: TabText) {
        self.text = tab.text;
        self.direction = tab.direction;
        self.kerning = tab.kerning;
        self.features = tab.features;
        self.origin = tab.origin;
        self.drag = None;
        self.touch();
    }
}

/// The glyphs an edit view shows: the text's (placeholders as the current glyph) when the text's current glyph is
/// `shown`, else `shown` alone.
pub fn shown_glyphs(text: &Text, shown: Option<&str>) -> Vec<String> {
    let current = text.current_glyph();
    let glyph = |item: &Item| match item {
        Item::Glyph { name, .. } => Some(name.to_string()),
        Item::Placeholder => current.map(str::to_string),
        Item::Missing(_) | Item::LineBreak => None,
    };
    let glyphs: Vec<String> = text.items().iter().filter_map(glyph).collect();
    if shown.is_some() && current == shown && glyphs.iter().any(|g| Some(g.as_str()) == shown) {
        return glyphs;
    }
    shown.map(str::to_string).into_iter().collect()
}

/// Whether the edit view is in text mode: the Text tool is the current tool.
pub fn text_mode(view: &ViewState) -> bool {
    view.tool == TEXT_TOOL
}

/// The glyphs a filter or a hint command on glyphs fired in text mode acts on: those the text selection shows
/// (`filter.target.editViewTextSelection`). `None` outside text mode, without a selection, for any other command and
/// one given parameters.
pub fn selection_glyphs(view: &ViewState, command: &CommandRef) -> Option<Vec<String>> {
    let empty = command.params.is_null() || command.params.as_object().is_some_and(Map::is_empty);
    let takes = command.id.starts_with("filter.") || crate::canvas::hinting::GLYPH_COMMANDS.contains(&command.id.as_str());
    if !text_mode(view) || !empty || !takes {
        return None;
    }
    let spec = fontcraft_engine::find_command(&command.id)?;
    if !spec.targets.contains(&fontcraft_engine::params::TargetKind::Glyphs) {
        return None;
    }
    let glyphs = view.text.selected_glyphs(crate::app::font_view::MAX_BATCH_GLYPHS);
    (!glyphs.is_empty()).then_some(glyphs)
}

/// Whether the canvas shows the text around the edited glyph: the text's current glyph is the shown glyph.
pub fn shows_text(font: &Font, view: &ViewState) -> bool {
    let shown = crate::canvas::tools::edited_glyph(font, view);
    shown.is_some() && view.text.current() == shown && view.text.edited_index().is_some()
}

/// The display toggles the canvas paints with: text mode hides the edited glyph's nodes and anchors.
pub fn display(view: &ViewState) -> crate::canvas::Display {
    let mut display = view.display;
    if text_mode(view) {
        display.nodes = false;
        display.nodes_in_background = false;
        display.extra_nodes = false;
        display.anchors = false;
    }
    display
}

/// Makes the text show the shown glyph: an empty text, or one whose current glyph is another, becomes that glyph
/// alone, the cursor before it. Entering text mode does this.
pub fn hold_shown_glyph(app: &mut FontCraftApp) {
    let Ok(font) = app.session.font() else { return };
    let Some(shown) = crate::canvas::tools::edited_glyph(font, &app.view).map(str::to_string) else { return };
    if shows_text(font, &app.view) {
        return;
    }
    let typed = font.glyph(&shown).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32);
    let text = &mut app.view.text;
    let mut next = Text::new();
    if next.replace(0..0, vec![Item::Glyph { name: shown.as_str().into(), typed }]).is_ok() {
        next.set_cursor(0);
        text.text = next;
        text.touch();
    }
}

/// After `drain`: puts the Text tool in the active-tool slot while text mode is on and the tool bridge back when it
/// ends; entering text mode makes the text hold the shown glyph.
pub fn sync(app: &mut FontCraftApp) {
    let on = text_mode(&app.view);
    if on == app.view.text.slot {
        return;
    }
    app.view.text.slot = on;
    if on {
        hold_shown_glyph(app);
        app.active_tool = Box::new(tool::TextTool::default());
    } else {
        app.active_tool = Box::new(crate::canvas::tools::bridge::ToolBridge::default());
    }
    app.view_changed();
}

/// After the projection: when the edited glyph moved to another place in the text, the canvas view moves with it,
/// so the text stays where it was on screen; then a replaced text is fitted and the cursor kept in view.
pub fn after_project(app: &mut FontCraftApp) {
    let model = &app.models.text;
    let now = model.edited_glyph.clone().filter(|_| model.shown).map(|g| (g, model.origin));
    let before = std::mem::replace(&mut app.view.text.origin, now.clone());
    if let (Some((_, [x0, y0])), Some((_, [x1, y1]))) = (before, now) {
        let (dx, dy) = (x0 - x1, y1 - y0);
        if (dx != 0.0 || dy != 0.0) && dx.is_finite() && dy.is_finite() {
            app.view.canvas.center.x += dx;
            app.view.canvas.center.y += dy;
        }
    }
    fit_text(app);
    keep_cursor_in_view(app);
}

/// After `editView.setText`: fits the new text in the canvas, when it holds several glyphs and the canvas has drawn.
fn fit_text(app: &mut FontCraftApp) {
    if !app.view.text.fit_pending || !app.view.canvas.rect.is_positive() {
        return;
    }
    app.view.text.fit_pending = false;
    let Some(glyph) = app.models.canvas.glyph.as_ref() else { return };
    if let Some(rect) = text_rect(&app.models.text, glyph.ascender, glyph.descender) {
        crate::shell::view::fit(&mut app.view.canvas, rect, glyph.ascender, app.models.canvas_tokens.fit_margin);
    }
}

/// In text mode, after the text or the cursor changed: scrolls the canvas sideways, when the cursor left it, so the
/// cursor is back inside, the fit margin from the edge. The zoom stays.
fn keep_cursor_in_view(app: &mut FontCraftApp) {
    let revision = app.view.text.revision;
    if app.view.text.followed == revision {
        return;
    }
    let model = &app.models.text;
    let view = &mut app.view.canvas;
    let (left, right) = (f64::from(view.rect.left()), f64::from(view.rect.right()));
    // A canvas not drawn yet has no rect: it fits its document the first time it draws.
    if !(left.is_finite() && right.is_finite() && right > left && view.zoom > 0.0) {
        return;
    }
    app.view.text.followed = revision;
    let Some([[x, _], _]) = model.cursor.filter(|_| model.text_mode && model.shown) else { return };
    let at = f64::from(view.to_screen(craft::behavior::kurbo::Point::new(x, 0.0)).x);
    let margin = (right - left) * app.models.canvas_tokens.fit_margin.clamp(0.0, 0.45);
    let shift = if at < left + margin {
        at - (left + margin)
    } else if at > right - margin {
        at - (right - margin)
    } else {
        0.0
    };
    if shift != 0.0 && shift.is_finite() {
        view.center.x += shift / view.zoom;
    }
}

/// The text around the edited glyph as a rect `[x, y, width, height]` in the edited glyph's font units: every
/// placed glyph's advance box from the descender to the ascender of its line. `None` while the text shows fewer
/// than two glyphs.
pub fn text_rect(model: &crate::app::models::text::TextModel, ascender: f64, descender: f64) -> Option<[f64; 4]> {
    let layout = model.layout.as_ref().filter(|_| model.shown)?;
    let placed: Vec<_> = layout.placed.iter().filter(|p| p.glyph.is_some()).collect();
    if placed.len() < 2 {
        return None;
    }
    let [ox, oy] = model.origin;
    let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in placed {
        x0 = x0.min(p.x.min(p.x + p.advance));
        x1 = x1.max(p.x.max(p.x + p.advance));
        y0 = y0.min(p.y + descender);
        y1 = y1.max(p.y + ascender);
    }
    let rect = [x0 - ox, y0 - oy, x1 - x0, y1 - y0];
    (rect.iter().all(|v| v.is_finite()) && rect[2] > 0.0 && rect[3] > 0.0).then_some(rect)
}

/// The glyph and the own layer of the edited text item, while the canvas shows the text and the glyph has that layer.
fn edited_item_layer(font: &Font, view: &ViewState) -> Option<(String, String)> {
    if !shows_text(font, view) {
        return None;
    }
    let at = view.text.edited_index()?;
    let glyph = font.glyph(view.text.item_glyph(at)?)?;
    let id = view.text.text.layer(at)?;
    glyph.layers.iter().any(|l| l.id == id).then(|| (glyph.name.clone(), id.to_string()))
}

/// Makes the edited text item's own layer the active layer, or the glyph's shown layer again once no edited item has
/// its own: after the cursor, the text or the shown glyph changed, before the tools read their target.
pub fn sync_layer(app: &mut FontCraftApp) {
    let item = app.session.font().ok().and_then(|font| edited_item_layer(font, &app.view));
    if app.view.layers.item != item {
        app.view.layers.item = item;
        app.view.layers.selection.clear();
        app.view_changed();
    }
}

/// Gives the edited text item its own layer (`layers.select`, `view.selectMaster` on an item that has one), or takes
/// it away.
pub fn set_item_layer(app: &mut FontCraftApp, layer: Option<&str>) {
    if let Some(at) = app.view.text.edited_index()
        && app.view.text.text.set_layer(at, layer)
    {
        app.view.text.touch();
    }
    sync_layer(app);
}

/// Makes the shown glyph follow the text's current glyph, after a text command.
pub fn follow(app: &mut FontCraftApp) {
    if let Some(current) = app.view.text.current() {
        app.view.glyph = Some(current.to_string());
    }
    app.view.text.touch();
    sync_layer(app);
    app.view_changed();
}

/// The edit view's text as agents read it (`ui.inspect` → `editText`).
pub fn reply(app: &FontCraftApp) -> Value {
    let text = &app.view.text;
    let font = app.session.font().ok();
    let items: Vec<Value> = text
        .text
        .items()
        .iter()
        .map(|item| match item {
            Item::Glyph { name, .. } => json!(name.as_ref()),
            Item::Placeholder => json!({"placeholder": true}),
            Item::LineBreak => json!({"lineBreak": true}),
            Item::Missing(c) => json!({"missing": c.to_string()}),
        })
        .collect();
    let selection = text.text.selection();
    let layers: Vec<Option<&str>> = (0..text.text.len()).map(|i| text.text.layer(i)).collect();
    json!({
        "items": items,
        "layers": layers.iter().any(Option::is_some).then_some(layers),
        "text": font.map(|f| text.text.to_display_string(f)),
        "cursor": text.text.cursor(),
        "selection": [selection.start, selection.end],
        "current": text.current(),
        "textMode": text_mode(&app.view),
        "shown": font.is_some_and(|f| shows_text(f, &app.view)),
        "direction": text.direction.name(),
        "kerning": text.kerning.name(),
        "groupMembers": text.group_members,
        "measurementLine": text.measurement,
        "measurementLineHeight": text.measurement_height,
        "features": features::reply(&app.models.features),
        "metricsLabels": app.models.text.labels.len(),
        "sampleStrings": crate::dialogs::sample_strings::reply(app),
        "findGlyphs": crate::dialogs::find_glyphs::reply(app),
    })
}

/// `ui.inspect`'s `editText` key.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    state.insert("editText".into(), reply(app));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod feature_tests;
