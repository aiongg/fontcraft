//! UI state: plain serializable data, so the control channel can read it (`ui.inspect`) and tests
//! can set it. Document state lives in the engine, never here.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use fontcraft_textlayout::{Direction, Item, KerningMode, Text, TextError};
use fontcraft_tools::Selection;
use serde::{Deserialize, Serialize};

/// Edit View scale limits, in screen points per font unit (a zoom value of 50 to 64,000).
pub const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 0.05..=64.0;
/// Zoom In and Zoom Out multiply or divide the scale by this.
pub const ZOOM_STEP: f64 = std::f64::consts::SQRT_2;
/// Most Edit View tabs open at once.
pub const MAX_TABS: usize = 1_000;
/// Most closed tabs kept for Reopen Last Closed Tab.
pub const MAX_CLOSED_TABS: usize = 32;
/// Most glyphs one tab (or one `text` parameter) may hold: a tab text's item limit.
pub const MAX_TAB_GLYPHS: usize = fontcraft_textlayout::MAX_ITEMS;
/// Largest coordinate, in font units, a view may be scrolled to.
pub const MAX_SCROLL: f64 = 10_000_000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// Glyphs selected in Font View, in selection order.
    pub selected_glyphs: Vec<String>,
    /// The Edit View tabs, in strip order. Font View is the fixed first tab and is not listed.
    pub tabs: Vec<Tab>,
    /// The showing Edit View tab, as an index into `tabs` (`None`: Font View is showing).
    pub active_tab: Option<usize>,
    /// Closed tabs, the most recent last, with the strip index each had.
    pub closed_tabs: Vec<(usize, Tab)>,
    /// The View menu's display toggles (for every tab).
    pub display: DisplayOptions,
    pub show_palette: bool,
    /// Size of the Edit View canvas at the last frame, in points. Commands that fit the view to
    /// the window use it, so they give the same result with and without a window.
    pub canvas_size: [f64; 2],
    /// The last message for the status bar (results and errors; agents read it too).
    pub status: String,
    /// The id the next new tab gets.
    pub next_tab_id: u64,
    /// The Transformations section's origin, fields and lock.
    pub transform: crate::shell::transform::TransformSettings,
    /// The Palette's and the Info box's own settings.
    pub panels: crate::panels::PanelState,
    /// Live audit, the audit markers, and the results they show.
    pub audit: crate::shell::audit::AuditState,
    /// Font View: view mode, cell size, master, List View columns and sort, the query.
    pub font_view: crate::shell::font_view::FontViewState,
    /// The Glyph Info window: open, query, sort and selected rows.
    pub glyph_info: crate::shell::glyph_info::GlyphInfoState,
    /// The Start Window: open, the selected recent document and the script chooser.
    pub start: crate::shell::start_window::StartState,
    /// The TrueType Instructor: direction, pixel size, point numbers, selected instructions.
    pub tt_instructor: crate::canvas::tt_badges::InstructorState,
    /// The document `tabs`, `active_tab` and `closed_tabs` belong to (`None`: none yet; the
    /// active document takes them).
    pub tabs_document: Option<u64>,
    /// The tabs of the other open documents, by document id, kept while those documents are not
    /// the active one.
    pub other_documents: BTreeMap<u64, DocumentTabs>,
}

/// One document's Edit View tabs while another document is the active one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocumentTabs {
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    pub closed_tabs: Vec<(usize, Tab)>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected_glyphs: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            closed_tabs: Vec::new(),
            display: DisplayOptions::default(),
            show_palette: true,
            canvas_size: [960.0, 720.0],
            status: String::new(),
            next_tab_id: 1,
            transform: Default::default(),
            panels: crate::panels::PanelState::default(),
            audit: Default::default(),
            font_view: Default::default(),
            glyph_info: Default::default(),
            start: Default::default(),
            tt_instructor: Default::default(),
            tabs_document: None,
            other_documents: BTreeMap::new(),
        }
    }
}

impl UiState {
    /// The showing Edit View tab.
    pub fn tab(&self) -> Option<&Tab> {
        self.active_tab.and_then(|i| self.tabs.get(i))
    }

    pub fn tab_mut(&mut self) -> Option<&mut Tab> {
        self.active_tab.and_then(|i| self.tabs.get_mut(i))
    }

    /// The glyph being edited in the showing tab (`None`: Font View is showing).
    pub fn editing(&self) -> Option<&str> {
        self.tab().and_then(Tab::edited_glyph)
    }

    /// The index of the tab with id `id`.
    pub fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Makes `tabs`, `active_tab` and `closed_tabs` those of document `active` (`None`: no
    /// document is open). The ones shown until now are kept for their document while `is_open`
    /// says it is still open, and dropped with it otherwise. Returns whether the tabs changed
    /// hands.
    pub fn follow_document(&mut self, active: Option<u64>, is_open: impl Fn(u64) -> bool) -> bool {
        self.other_documents.retain(|id, _| is_open(*id));
        let switched = match (self.tabs_document, active) {
            (Some(shown), Some(now)) if shown == now => false,
            (None, Some(_)) => false,
            (shown, _) => {
                let kept = DocumentTabs {
                    tabs: std::mem::take(&mut self.tabs),
                    active_tab: self.active_tab.take(),
                    closed_tabs: std::mem::take(&mut self.closed_tabs),
                };
                if let Some(shown) = shown
                    && is_open(shown)
                    && (!kept.tabs.is_empty() || !kept.closed_tabs.is_empty())
                {
                    self.other_documents.insert(shown, kept);
                }
                if let Some(back) = active.and_then(|now| self.other_documents.remove(&now)) {
                    self.tabs = back.tabs;
                    self.active_tab = back.active_tab.filter(|i| *i < self.tabs.len());
                    self.closed_tabs = back.closed_tabs;
                }
                true
            }
        };
        self.tabs_document = active;
        if let Some(id) = active {
            self.tabs.iter_mut().chain(self.closed_tabs.iter_mut().map(|(_, t)| t)).for_each(|t| t.document = id);
        }
        switched
    }
}

/// One Edit View tab: its text, the camera and the selection. The edited glyph is the text's
/// current glyph, the glyph after the cursor ([`Tab::edited_glyph`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tab {
    /// Stable for the session, unlike the tab's position in the strip.
    pub id: u64,
    /// The id of the document the tab shows (its `DocState::id` in the session).
    pub document: u64,
    /// The items, the cursor, the selection and the current glyph.
    pub text: TabText,
    /// Typing edits the text (`editView.toggleTextMode`); otherwise the current glyph is edited.
    pub text_mode: bool,
    pub direction: WritingDirection,
    #[serde(with = "kerning_mode")]
    pub kerning: KerningMode,
    pub view: TabView,
    /// The selection in the edited glyph's layer.
    pub selection: Selection,
    /// Path focus (`path.focusOnSelectedPath`): only these shapes of the edited layer show their
    /// nodes and can be selected.
    pub focus: Option<Vec<usize>>,
    /// The glyph `selection` and `focus` belong to. Both are cleared when another glyph is edited.
    pub selection_glyph: Option<String>,
    /// The master the tab draws its glyphs in (`view.selectMaster`). `None`, or a master the font
    /// no longer has: the first master.
    pub master: Option<String>,
    /// Glyphs drawn in another layer than their layer of `master` (`layers.select`): glyph name →
    /// layer id. A layer the glyph no longer has falls back to `master`.
    pub layers: BTreeMap<String, String>,
    /// The rows selected in the Layers section for the edited glyph (layer ids, in the order they
    /// were picked). A plain click selects the active layer alone.
    pub layer_selection: Vec<String>,
    /// The layer `selection` and `focus` belong to. Both are cleared when another layer is edited.
    pub selection_layer: Option<String>,
    /// The tab edits the background of the active layer (`path.editBackground`). It stays set
    /// when another glyph, layer or master is edited.
    pub edit_background: bool,
    /// The OpenType features the tab previews (the Features menu).
    pub features: crate::shell::features_menu::TabFeatures,
}

impl Tab {
    /// The glyph being edited: the text's current glyph.
    pub fn edited_glyph(&self) -> Option<&str> {
        self.text.current_glyph()
    }

    /// The index of the item that shows the edited glyph: the item after the cursor, else the
    /// one before it, else the first item that shows it.
    pub fn edited_index(&self) -> Option<usize> {
        let current = self.text.current_glyph()?;
        let shows = |i: usize| self.item_glyph(i) == Some(current);
        let cursor = self.text.cursor();
        [Some(cursor), cursor.checked_sub(1)].into_iter().flatten().find(|&i| shows(i)).or_else(|| (0..self.text.len()).find(|&i| shows(i))).or_else(
            // A glyph that a preview feature shows in place of the item after the cursor
            // (`editView.setCursor` with `glyph`) is edited there.
            || (!self.features.active.is_empty() && matches!(self.text.items().get(cursor), Some(Item::Glyph { .. }))).then_some(cursor),
        )
    }

    /// The glyph item `index` shows: its own, or the current glyph for a placeholder.
    pub fn item_glyph(&self, index: usize) -> Option<&str> {
        match self.text.items().get(index)? {
            Item::Glyph { name, .. } => Some(name),
            Item::Placeholder => self.text.current_glyph(),
            Item::Missing(_) | Item::LineBreak => None,
        }
    }

    /// The glyphs the text shows, in order, placeholders as the current glyph.
    pub fn shown_glyphs(&self) -> impl Iterator<Item = &str> {
        (0..self.text.len()).filter_map(|i| self.item_glyph(i))
    }

    /// Whether the text is exactly these glyphs, with nothing else.
    pub fn holds(&self, glyphs: &[String]) -> bool {
        self.text.len() == glyphs.len()
            && self.text.items().iter().zip(glyphs).all(|(item, g)| matches!(item, Item::Glyph { name, .. } if **name == **g))
    }

    /// The side of the active layer the tab edits.
    pub fn side(&self) -> fontcraft_model::LayerSide {
        if self.edit_background { fontcraft_model::LayerSide::Background } else { fontcraft_model::LayerSide::Foreground }
    }

    /// Clears the selection and the path focus when they belong to another glyph or layer than
    /// the edited one (`layer`: the active layer's id), and the layer rows when the glyph changed.
    pub fn sync_selection(&mut self, layer: Option<&str>) {
        let edited = self.edited_glyph().map(str::to_string);
        if self.selection_glyph != edited {
            self.layer_selection.clear();
        }
        if self.selection_glyph != edited || self.selection_layer.as_deref() != layer {
            self.selection.clear();
            self.focus = None;
            self.selection_glyph = edited;
            self.selection_layer = layer.map(str::to_string);
        }
    }

    /// The title in the tab strip.
    pub fn title(&self) -> String {
        const SHOWN: usize = 4;
        let mut title = self.shown_glyphs().take(SHOWN).collect::<Vec<_>>().join(" ");
        if self.shown_glyphs().nth(SHOWN).is_some() {
            title.push_str(" …");
        }
        if title.is_empty() {
            title.push('–');
        }
        title
    }
}

/// A tab text's writing direction (`editView.setWritingDirection`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

    /// A parameter value; `vertical`, as the kerning commands name the direction, means `ttb`.
    pub fn from_name(name: &str) -> Option<WritingDirection> {
        match name {
            "vertical" => Some(WritingDirection::Ttb),
            _ => WritingDirection::ALL.into_iter().find(|d| d.name() == name),
        }
    }

    /// The layout and kerning direction.
    pub fn layout(self) -> Direction {
        match self {
            WritingDirection::Ltr => Direction::Ltr,
            WritingDirection::Rtl => Direction::Rtl,
            WritingDirection::Ttb => Direction::Vertical,
        }
    }
}

mod kerning_mode {
    use fontcraft_textlayout::KerningMode;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(mode: &KerningMode, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(mode.name())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<KerningMode, D::Error> {
        let name = String::deserialize(d)?;
        KerningMode::from_name(&name).ok_or_else(|| D::Error::custom(format!("unknown kerning mode `{name}`")))
    }
}

static NEXT_TEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// A tab's [`Text`] and a revision that changes with every edit, so its layout can be cached.
/// Revisions are unique in the process, so two texts never share one.
#[derive(Clone, Debug)]
pub struct TabText {
    text: Text,
    revision: u64,
}

impl Default for TabText {
    fn default() -> Self {
        TabText::new(Text::new())
    }
}

impl PartialEq for TabText {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl std::ops::Deref for TabText {
    type Target = Text;
    fn deref(&self) -> &Text {
        &self.text
    }
}

impl TabText {
    pub fn new(text: Text) -> TabText {
        TabText { text, revision: NEXT_TEXT_REVISION.fetch_add(1, Ordering::Relaxed) }
    }

    /// A text of these glyphs, entered by name, with the cursor before the first.
    pub fn from_glyphs(glyphs: &[String]) -> Result<TabText, TextError> {
        let mut text = Text::new();
        text.replace(0..0, glyphs.iter().map(|g| Item::glyph(g)).collect())?;
        text.set_cursor(0);
        Ok(TabText::new(text))
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The text, to change it; the revision changes.
    pub fn edit(&mut self) -> &mut Text {
        self.revision = NEXT_TEXT_REVISION.fetch_add(1, Ordering::Relaxed);
        &mut self.text
    }

    /// The end of the selection the cursor is not at (the cursor when nothing is selected).
    pub fn anchor(&self) -> usize {
        let s = self.text.selection();
        if self.text.cursor() == s.start { s.end } else { s.start }
    }

    /// Passes every item through `f`, dropping those it returns `None` for, and makes `current`
    /// the current glyph. The cursor and the selection keep their places among the items kept.
    pub fn rebuild(&mut self, mut f: impl FnMut(&Item) -> Option<Item>, current: Option<&str>) {
        let (cursor, anchor) = (self.text.cursor(), self.anchor());
        let (mut new_cursor, mut new_anchor) = (0, 0);
        let mut items = Vec::with_capacity(self.text.len());
        for (i, item) in self.text.items().iter().enumerate() {
            if let Some(kept) = f(item) {
                items.push(kept);
                new_cursor += usize::from(i < cursor);
                new_anchor += usize::from(i < anchor);
            }
        }
        let text = self.edit();
        let len = text.len();
        // `f` returns at most one item per item, so the text cannot outgrow its limit.
        if text.replace(0..len, items).is_ok() {
            text.set_current_glyph(current);
            text.select(new_anchor, new_cursor);
        }
    }

    /// Renames glyph `old` to `new` in the items and the current glyph.
    pub fn rename_glyph(&mut self, old: &str, new: &str) {
        let named = |item: &Item| matches!(item, Item::Glyph { name, .. } if &**name == old);
        if !self.text.items().iter().any(named) && self.text.current_glyph() != Some(old) {
            return;
        }
        let current = self.text.current_glyph().map(|c| if c == old { new.to_string() } else { c.to_string() });
        self.rebuild(
            |item| {
                Some(match item {
                    Item::Glyph { name, typed } if &**name == old => Item::Glyph { name: Arc::from(new), typed: *typed },
                    other => other.clone(),
                })
            },
            current.as_deref(),
        );
    }

    /// Drops the glyph items `gone` names; a current glyph it names is replaced by the glyph
    /// next to the cursor. Returns whether anything was dropped.
    pub fn remove_glyphs(&mut self, gone: impl Fn(&str) -> bool) -> bool {
        let named = |item: &Item| matches!(item, Item::Glyph { name, .. } if gone(name));
        let current_gone = self.text.current_glyph().is_some_and(&gone);
        if !self.text.items().iter().any(named) && !current_gone {
            return false;
        }
        let current = if current_gone { None } else { self.text.current_glyph().map(str::to_string) };
        self.rebuild(|item| (!named(item)).then(|| item.clone()), current.as_deref());
        true
    }
}

/// How a tab text is written in the UI state (`ui.inspect`).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ItemForm {
    Glyph {
        name: String,
        #[serde(default)]
        typed: Option<char>,
    },
    Missing(char),
    Placeholder,
    LineBreak,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct TextForm {
    items: Vec<ItemForm>,
    cursor: usize,
    anchor: usize,
    current: Option<String>,
}

impl Serialize for TabText {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let items = self
            .text
            .items()
            .iter()
            .map(|item| match item {
                Item::Glyph { name, typed } => ItemForm::Glyph { name: name.to_string(), typed: *typed },
                Item::Missing(c) => ItemForm::Missing(*c),
                Item::Placeholder => ItemForm::Placeholder,
                Item::LineBreak => ItemForm::LineBreak,
            })
            .collect();
        TextForm { items, cursor: self.text.cursor(), anchor: self.anchor(), current: self.text.current_glyph().map(str::to_string) }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for TabText {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<TabText, D::Error> {
        use serde::de::Error;
        let form = TextForm::deserialize(d)?;
        let items = form
            .items
            .into_iter()
            .map(|item| match item {
                ItemForm::Glyph { name, typed } => Item::Glyph { name: Arc::from(name), typed },
                ItemForm::Missing(c) => Item::Missing(c),
                ItemForm::Placeholder => Item::Placeholder,
                ItemForm::LineBreak => Item::LineBreak,
            })
            .collect();
        let mut text = Text::new();
        text.replace(0..0, items).map_err(D::Error::custom)?;
        text.set_current_glyph(form.current.as_deref());
        text.select(form.anchor, form.cursor);
        Ok(TabText::new(text))
    }
}

/// An Edit View camera.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabView {
    /// Screen points per font unit (the zoom value is this × 1000).
    pub scale: f64,
    /// The point drawn at the centre of the canvas, in the text's font units: x from the text's
    /// origin, y from its first baseline.
    pub center: [f64; 2],
}

impl Default for TabView {
    fn default() -> Self {
        Self { scale: 0.5, center: [0.0, 300.0] }
    }
}

impl TabView {
    /// The scale held to [`ZOOM_RANGE`]; a value that is not a number becomes the default.
    pub fn clamped_scale(scale: f64) -> f64 {
        if scale.is_finite() { scale.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end()) } else { TabView::default().scale }
    }
}

/// The View menu's display toggles. Toggles whose drawing belongs to a later package are state
/// only until then (`docs/plans/M3.md` › M3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DisplayOptions {
    /// Nodes and handles of the edited layer (`view.showNodes`, `view.showNodesInForeground`).
    pub nodes: bool,
    pub nodes_in_background: bool,
    pub extra_nodes: bool,
    pub metrics: bool,
    pub metrics_names: bool,
    pub info: bool,
    pub background: bool,
    pub guides: bool,
    pub bounding_box: bool,
    pub annotations: bool,
    pub fill_preview: bool,
    /// The edited layer's anchors (`view.showAnchors`).
    pub anchors: bool,
    /// The other members of the edited pair's kerning groups (`view.showGroupMembers`).
    pub group_members: bool,
    /// The edited layer's PostScript hints (`view.showHints`).
    pub hints: bool,
    /// Every master, intermediate and alternate layer of the edited glyph drawn together
    /// (`view.showMasterCompatibility`).
    pub master_compatibility: bool,
    /// Switching masters keeps the selection, by index (`edit.keepLayerSelectionInSync`).
    pub keep_layer_selection_in_sync: bool,
    /// Placed images on glyph layers (`view.showImage`); a layer with no paths and no
    /// components shows its image regardless.
    pub show_image: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            nodes: true,
            nodes_in_background: false,
            extra_nodes: false,
            metrics: true,
            metrics_names: false,
            info: true,
            background: true,
            guides: true,
            bounding_box: false,
            annotations: true,
            fill_preview: true,
            anchors: true,
            group_members: false,
            hints: false,
            master_compatibility: false,
            keep_layer_selection_in_sync: false,
            show_image: true,
        }
    }
}

/// The glyphs that encode the characters of `text`, in order. A character no glyph encodes is an
/// error naming it.
pub fn glyphs_for_text(font: &fontcraft_model::Font, text: &str) -> Result<Vec<String>, String> {
    if text.chars().count() > MAX_TAB_GLYPHS {
        return Err(format!("`text` holds at most {MAX_TAB_GLYPHS} characters"));
    }
    let mut by_code: std::collections::HashMap<u32, &str> = std::collections::HashMap::new();
    for glyph in font.glyphs.iter() {
        for u in &glyph.unicodes {
            by_code.entry(*u).or_insert(glyph.name.as_str());
        }
    }
    text.chars()
        .map(|c| by_code.get(&u32::from(c)).map(|n| n.to_string()).ok_or_else(|| format!("no glyph encodes U+{:04X} `{c}`", u32::from(c))))
        .collect()
}
