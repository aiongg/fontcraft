//! Hinting on the canvas: PostScript hints while View › Show Hints is on, and the TrueType Instructor's badges,
//! nodes, point numbers, pixel backdrop and preview outlines.
//!
//! | Module | Holds |
//! |---|---|
//! | [`commands`] | the shell commands of hint display, hint and instruction selection, and the Instructor's view state |
//! | [`ps`] | the PostScript hint layers: bars behind the outline, badges and markers in front |
//! | [`badges`] | the Instructor's layers (nodes, instruction badges, point numbers) and its badge clicks, drags and Delete |
//! | [`pixels`] | the Instructor's pixel backdrop and its blue (instance) and red (grid-fitted) outlines |
//! | [`tokens`] | their colours and sizes |
//!
//! The state is [`HintingView`], at `ViewState::hinting`; nothing in it is saved with the font. What the layers draw is
//! projected into `models.hint_canvas` (`app::models::hint_canvas`) once per engine or view-state change. The
//! PostScript hints the Select tool hits are read fresh into the tools' scene ([`ps_marks`]), so what is drawn is what
//! is hit. Menu commands get their hint and instruction targets from [`targets`].

pub mod badges;
pub mod commands;
pub mod pixels;
pub mod ps;
pub mod tokens;

use std::collections::BTreeSet;

use craft::ui::contract::Tracked;
use egui::Pos2;
use fontcraft_engine::Session;
use fontcraft_engine::hinting::{Direction, MAX_INSTRUCTOR_PPM, MIN_INSTRUCTOR_PPM, tt_instructions};
use fontcraft_render::hinted::Intent;
use fontcraft_tools::hint_drag::{self, HintLayout, HintMark, HintMarks};
use fontcraft_tools::tt_instructor::{self, TtDirection};
use fontcraft_tools::{Scene, Selection};
use serde_json::{Map, Value, json};

use self::tokens::HintTokens;
use crate::app::ViewState;
use crate::app::models::hint_canvas::HintCanvas;

/// The TrueType Instructor's tool id.
pub const INSTRUCTOR: &str = tt_instructor::ID;
/// Smallest preview pixel size, in pixels per em.
pub const MIN_PPM: u16 = MIN_INSTRUCTOR_PPM;
/// Largest preview pixel size.
pub const MAX_PPM: u16 = MAX_INSTRUCTOR_PPM;
/// The pixel size the Instructor starts with.
pub const DEFAULT_PPM: u16 = 12;

/// `ppm` brought into [`MIN_PPM`]..=[`MAX_PPM`].
pub fn clamp_ppm(ppm: i64) -> u16 {
    u16::try_from(ppm.clamp(i64::from(MIN_PPM), i64::from(MAX_PPM))).unwrap_or(DEFAULT_PPM)
}

/// The rendering the Instructor's grid-fitted outline and pixel backdrop imitate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RenderingIntent {
    /// Grayscale anti-aliasing.
    #[default]
    Grayscale,
    /// GDI ClearType: horizontal subpixels, no vertical oversampling.
    ClearType,
    /// DirectWrite ClearType: horizontal subpixels, smoothed vertically.
    DirectWrite,
}

impl RenderingIntent {
    pub const ALL: [RenderingIntent; 3] = [RenderingIntent::Grayscale, RenderingIntent::ClearType, RenderingIntent::DirectWrite];

    /// The name commands use: `grayscale`, `cleartype` or `directwrite`.
    pub fn as_str(self) -> &'static str {
        match self {
            RenderingIntent::Grayscale => "grayscale",
            RenderingIntent::ClearType => "cleartype",
            RenderingIntent::DirectWrite => "directwrite",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.as_str() == name)
    }

    /// The hinted renderer's intent.
    pub fn hinted(self) -> Intent {
        match self {
            RenderingIntent::Grayscale => Intent::Grayscale,
            RenderingIntent::ClearType => Intent::GdiClearType,
            RenderingIntent::DirectWrite => Intent::DirectWrite,
        }
    }
}

/// Instructions selected in one layer of one glyph, by their position in its instruction list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedInstructions {
    pub glyph: String,
    pub layer: String,
    pub instructions: BTreeSet<usize>,
}

/// The TrueType Instructor's view state.
#[derive(Clone, Debug, PartialEq)]
pub struct InstructorState {
    /// The direction new instructions take (`hint.tt.toggleDirection`).
    pub direction: TtDirection,
    /// The preview pixel size, [`MIN_PPM`] to [`MAX_PPM`].
    pub ppm: u16,
    /// Point numbers beside the on-curve nodes (`hint.tt.showPointIndexes`).
    pub point_indexes: bool,
    /// The selected instructions (`hint.tt.selectInstructions`).
    pub selected: SelectedInstructions,
    /// The blue and red outlines (`hint.tt.showPixelPreview`).
    pub pixel_preview: bool,
    /// The previewed instance as `hint.tt.showPixelPreview` names it; `None` previews the edited master.
    pub instance: Option<String>,
    /// The rendering the grid-fitted outline and the backdrop imitate (`hint.tt.setRenderingIntent`).
    pub intent: RenderingIntent,
}

impl Default for InstructorState {
    fn default() -> Self {
        InstructorState {
            direction: TtDirection::Vertical,
            ppm: DEFAULT_PPM,
            point_indexes: false,
            selected: SelectedInstructions::default(),
            pixel_preview: true,
            instance: None,
            intent: RenderingIntent::default(),
        }
    }
}

impl InstructorState {
    /// The selected instructions of `glyph`'s layer `layer`: empty when the selection belongs to another glyph or
    /// layer.
    pub fn selected_in(&self, glyph: &str, layer: &str) -> BTreeSet<usize> {
        if self.selected.glyph == glyph && self.selected.layer == layer { self.selected.instructions.clone() } else { BTreeSet::new() }
    }
}

/// An instruction badge being dragged, in screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BadgeDrag {
    /// The instruction's position in the layer's instruction list.
    pub index: usize,
    pub from: Pos2,
    pub at: Pos2,
    /// Shift was held at the press: a click toggles the instruction instead of selecting it alone.
    pub shift: bool,
    /// The pointer went past the drag threshold: the release is a drop, not a click.
    pub moved: bool,
}

/// The Review Hints dialog's state (`hint.ps.reviewHints`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewState {
    pub open: bool,
    pub glyph: String,
    /// The layer's id.
    pub layer: String,
    /// The direction reviewed.
    pub horizontal: bool,
    /// The reviewed hint, by its position in the layer's hint list.
    pub hint: Option<usize>,
}

/// The hinting view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintingView {
    pub instructor: InstructorState,
    /// The instruction badge under a drag.
    pub drag: Option<BadgeDrag>,
    pub review: ReviewState,
}

/// What the hint layers read beside the paint context.
#[derive(Clone, Copy)]
pub struct HintPaint<'a> {
    pub model: &'a Tracked<HintCanvas>,
    pub view: &'a HintingView,
    pub tokens: &'a HintTokens,
}

/// The hinting crate's direction for the Instructor's.
pub fn hinting_direction(d: TtDirection) -> Direction {
    match d {
        TtDirection::Vertical => Direction::Vertical,
        TtDirection::Horizontal => Direction::Horizontal,
    }
}

/// The sizes of the PostScript hint marks.
pub fn layout(t: &HintTokens) -> HintLayout {
    let [width, height] = t.badge_size;
    HintLayout { lane: f64::from(t.lane), badge: (f64::from(width), f64::from(height)), marker: f64::from(t.marker_radius), gap: f64::from(t.gap) }
}

/// The edited layer's PostScript hints as `hint.ps.listHints` reports them, placed for drawing and hitting. Not shown
/// (and empty) while Show Hints is off, while the background is edited, or when the list cannot be read.
pub fn ps_marks(session: &mut Session, view: &ViewState) -> HintMarks {
    let mut out = HintMarks { layout: layout(&HintTokens::standard()), ..HintMarks::default() };
    if !view.display.hints || view.edit_background {
        return out;
    }
    let Some(target) = super::tools::target(session, view) else { return out };
    let bottom = session
        .font()
        .ok()
        .and_then(|font| font.masters.first().map(|m| crate::app::models::canvas::vertical_span(font, m).0))
        .filter(|b| b.is_finite())
        .unwrap_or(0.0);
    let Ok(list) = session.query("hint.ps.listHints", &json!({"glyph": target.glyph, "layer": target.layer})) else { return out };
    out.shown = true;
    out.bottom = bottom;
    out.hints = list.get("hints").and_then(Value::as_array).map(|a| a.iter().filter_map(HintMark::from_value).collect()).unwrap_or_default();
    out
}

/// The parameters a `hint.ps.*` or `hint.tt.*` command run from a menu or a key takes from the canvas: the PostScript
/// commands through [`hint_drag::menu_params`] (selected nodes or hints, the edited glyph and layer); the TrueType
/// instruction commands the selected instructions, or the selected nodes and the Instructor's direction. `None` for
/// other commands.
pub fn targets(scene: &Scene, selection: &Selection, view: &HintingView, id: &str) -> Option<Value> {
    if id.starts_with("hint.ps.") {
        return hint_drag::menu_params(scene, selection, id);
    }
    let base = || {
        let mut m = Map::new();
        m.insert("glyph".into(), json!(scene.glyph));
        m.insert("layer".into(), json!(scene.layer_id));
        m
    };
    match id {
        "hint.tt.snapPoint" | "hint.tt.addStemHint" | "hint.tt.shiftPoints" | "hint.tt.interpolate" | "hint.tt.addDelta" => {
            let direction = view.instructor.direction;
            let mut points = tt_instructor::selected_points(scene, selection);
            if id == "hint.tt.addStemHint" {
                points = tt_instructor::stem_pairs(points, direction);
            }
            let mut m = base();
            if !points.is_empty() {
                let nodes: Vec<Value> = points.iter().map(|p| json!([p.path, p.node])).collect();
                m.insert("nodes".into(), Value::Array(nodes));
            }
            m.insert("direction".into(), json!(direction.as_str()));
            Some(Value::Object(m))
        }
        "hint.tt.deleteInstructions"
        | "hint.tt.setRounding"
        | "hint.tt.setSnapZone"
        | "hint.tt.setStemReference"
        | "hint.tt.setDeltas"
        | "hint.tt.makeTripleHint" => {
            let count = tt_instructions(&scene.layer).map(|l| l.len()).unwrap_or(0);
            let selected: Vec<usize> = view.instructor.selected_in(&scene.glyph, &scene.layer_id).into_iter().filter(|&i| i < count).collect();
            let mut m = base();
            if !selected.is_empty() {
                m.insert("instructions".into(), json!(selected));
            }
            Some(Value::Object(m))
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
