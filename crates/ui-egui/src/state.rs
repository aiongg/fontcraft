//! UI state: plain serializable data, so the control channel can read it (`ui.inspect`) and tests
//! can set it. Document state lives in the engine, never here.

use serde::{Deserialize, Serialize};

pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.05..=64.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// Glyphs selected in Font View, in selection order.
    pub selected_glyphs: Vec<String>,
    /// The glyph open in Edit View (`None`: Font View is showing).
    pub editing: Option<String>,
    /// Edit View scale: screen points per font unit, times 1000 (1.0 fits a 1000-unit em in 1000 points).
    pub zoom: f32,
    pub show_nodes: bool,
    pub show_metrics: bool,
    pub show_palette: bool,
    /// The last message for the status bar (results and errors; agents read it too).
    pub status: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected_glyphs: Vec::new(),
            editing: None,
            zoom: 0.5,
            show_nodes: true,
            show_metrics: true,
            show_palette: true,
            status: String::new(),
        }
    }
}

/// Most glyphs one tab (or one `text` parameter) may hold.
pub const MAX_TAB_GLYPHS: usize = 10_000;

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
