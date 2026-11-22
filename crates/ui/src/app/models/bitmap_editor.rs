//! The Bitmap Glyph Editor's view model: one glyph's bitmap in one strike, the strike's line band, the glyph's
//! outline scaled to the strike as a guide, and the tool, shade and overlay switches the window shows.
//!
//! The bitmap is the glyph's own pixels; bitmaps it places by reference are not drawn in, since the tools edit the
//! glyph's own pixels only.

use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::model::Font;
use fontcraft_engine::model::strike::{BitmapStrike, StrikeGlyph, max_level};

use super::ProjectCx;
use super::font_view::{CellArt, StrikeCell, StrikeShown, flatten};
use crate::app::ViewState;
use crate::shell::dock::PanelId;
use crate::shell::masters;

/// Most outline points the guide holds; a glyph with more shows no guide.
const MAX_GUIDE_POINTS: usize = 65_536;

/// What the window shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct BitmapEditorModel {
    /// The glyph shown; `None` without a font, a glyph or a strike.
    pub glyph: Option<String>,
    /// The strike shown: its size, depth and line band (`magnification` unused).
    pub strike: Option<StrikeShown>,
    /// `12`, or `12@4` for a grey strike.
    pub strike_label: String,
    /// Whether the font has any strike: without one the window says how to add one.
    pub has_strikes: bool,
    /// The glyph's own bitmap; `None` when the strike has none for it yet.
    pub bitmap: Option<Arc<StrikeCell>>,
    /// The advance in pixels: the bitmap's, or the glyph's width scaled to the strike when it has no bitmap.
    pub advance: i32,
    /// The glyph's outline in the default master, in strike pixels from the glyph origin.
    pub guide: Arc<CellArt>,
    /// The tool's word (`fontcraft_tools::bitmap::BitmapTool::word`).
    pub tool: String,
    /// The level the tools draw with, and the strike's top level.
    pub level: u8,
    pub max_level: u8,
    pub show_outline: bool,
    pub show_grid: bool,
}

/// The glyph and strike the window shows: the ones it was opened on while the font has them, else the glyph the
/// glyph panels show and the font's first strike.
pub fn shown<'f>(font: &'f Font, view: &ViewState) -> Option<(&'f fontcraft_engine::model::Glyph, &'f BitmapStrike)> {
    let state = &view.bitmap_editor;
    let glyph = state.glyph.as_deref().and_then(|n| font.glyph(n)).map(|g| &**g).or_else(|| masters::shown_glyph(font, view))?;
    let strike = state
        .strike
        .and_then(|(ppem, depth)| font.bitmap_strikes.iter().find(|s| s.ppem == ppem && s.depth == depth))
        .or_else(|| font.bitmap_strikes.first())?;
    Some((glyph, strike))
}

/// The level the tools draw with in `strike`: the chosen shade, at most the strike's top level.
pub fn draw_level(view: &ViewState, strike: &BitmapStrike) -> u8 {
    let top = max_level(strike.depth);
    view.bitmap_editor.level.map_or(top, |l| l.min(top))
}

pub fn project(model: &mut Tracked<BitmapEditorModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout.panel_state(&PanelId::BitmapEditor) == PanelState::Hidden {
        return;
    }
    let value = cx.session.font().ok().map(|font| editor(font, cx.view)).unwrap_or_default();
    super::set(model, value);
}

fn editor(font: &Font, view: &ViewState) -> BitmapEditorModel {
    let state = &view.bitmap_editor;
    let switches = BitmapEditorModel {
        tool: state.tool.word().to_string(),
        show_outline: state.outline,
        show_grid: state.grid,
        has_strikes: !font.bitmap_strikes.is_empty(),
        ..BitmapEditorModel::default()
    };
    let Some((glyph, strike)) = shown(font, view) else { return switches };
    let own = strike.glyph(&glyph.name);
    let scale = f64::from(strike.ppem) / f64::from(font.units_per_em.max(1));
    let advance = own.map_or_else(|| scaled_width(font, &glyph.name, scale), |b| b.advance);
    BitmapEditorModel {
        glyph: Some(glyph.name.clone()),
        strike: Some(StrikeShown { ppem: strike.ppem, depth: strike.depth, ascent: strike.ascent, descent: strike.descent, magnification: 1 }),
        strike_label: strike.label(),
        bitmap: own.filter(|b| b.width > 0 && b.height > 0).map(|b| Arc::new(cell(b, strike.depth))),
        advance,
        guide: Arc::new(guide(font, &glyph.name, scale as f32)),
        level: draw_level(view, strike),
        max_level: max_level(strike.depth),
        ..switches
    }
}

/// The glyph's default-master width in strike pixels, rounded.
fn scaled_width(font: &Font, glyph: &str, scale: f64) -> i32 {
    let width = font.default_master().ok().and_then(|m| font.glyph(glyph).and_then(|g| g.layer(&m.id))).map_or(0.0, |l| l.width);
    let v = (width * scale).round();
    if v.is_finite() { v.clamp(-f64::from(i32::MAX), f64::from(i32::MAX)) as i32 } else { 0 }
}

/// A bitmap as the window draws it; its id is a hash of its pixels, so an unchanged bitmap keeps its texture.
fn cell(b: &StrikeGlyph, depth: u8) -> StrikeCell {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (b.width, b.height, depth, &b.data).hash(&mut h);
    StrikeCell { id: h.finish(), width: b.width, height: b.height, left: b.left, bottom: b.bottom, advance: b.advance, depth, data: b.data.clone() }
}

/// The glyph's default-master outline, flattened and scaled to strike pixels; empty when it has none or too many
/// points.
fn guide(font: &Font, glyph: &str, scale: f32) -> CellArt {
    let Some(outline) = font.default_master().ok().and_then(|m| font.outline(glyph, &m.id).ok()) else { return CellArt::default() };
    let contours = flatten(&outline);
    if contours.iter().map(Vec::len).sum::<usize>() > MAX_GUIDE_POINTS || !scale.is_finite() {
        return CellArt::default();
    }
    let contours = contours.into_iter().map(|c| c.into_iter().map(|[x, y]| [x * scale, y * scale]).collect()).collect();
    CellArt { width: 0.0, contours, color: None }
}
