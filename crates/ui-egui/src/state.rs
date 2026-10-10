//! UI state: plain serializable data, so the control channel can read it (`ui.inspect`) and tests
//! can set it. Document state lives in the engine, never here.

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
/// Most glyphs one tab (or one `text` parameter) may hold.
pub const MAX_TAB_GLYPHS: usize = 10_000;
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
}

/// One Edit View tab: a line of glyphs, the one being edited, the camera and the selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tab {
    /// Stable for the session, unlike the tab's position in the strip.
    pub id: u64,
    pub glyphs: Vec<String>,
    /// Which entry of `glyphs` is edited.
    pub edited: usize,
    pub view: TabView,
    /// The selection in the edited glyph's layer.
    pub selection: Selection,
    /// Path focus (`path.focusOnSelectedPath`): only these shapes of the edited layer show their
    /// nodes and can be selected.
    pub focus: Option<Vec<usize>>,
    /// The glyph `selection` and `focus` belong to. Both are cleared when another glyph is edited.
    pub selection_glyph: Option<String>,
}

impl Tab {
    pub fn edited_glyph(&self) -> Option<&str> {
        self.glyphs.get(self.edited).map(String::as_str)
    }

    /// Clears the selection and the path focus when they belong to another glyph than the edited
    /// one.
    pub fn sync_selection(&mut self) {
        let edited = self.edited_glyph().map(str::to_string);
        if self.selection_glyph != edited {
            self.selection.clear();
            self.focus = None;
            self.selection_glyph = edited;
        }
    }

    /// The title in the tab strip.
    pub fn title(&self) -> String {
        const SHOWN: usize = 4;
        let mut title = self.glyphs.iter().take(SHOWN).map(String::as_str).collect::<Vec<_>>().join(" ");
        if self.glyphs.len() > SHOWN {
            title.push_str(" …");
        }
        title
    }
}

/// An Edit View camera.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabView {
    /// Screen points per font unit (the zoom value is this × 1000).
    pub scale: f64,
    /// The point drawn at the centre of the canvas, in the line's font units: x from the start of
    /// the tab's first glyph, y from the baseline.
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
