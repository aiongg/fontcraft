//! The tool interface and the list of tools.

use fontcraft_geom::Point;
use fontcraft_model::Font;
use serde::Serialize;

use crate::event::{KeyEvent, Modifiers, PointerEvent};
use crate::output::{Output, Overlay};
use crate::scene::Scene;
use crate::selection::Selection;
use crate::view::View;
use crate::{
    annotate::AnnotateTool, draw::DrawTool, hand::HandTool, lasso::LassoTool, measure::MeasureTool, pencil::PencilTool, primitives::PrimitiveTool,
    rotate::RotateTool, scale::ScaleTool, select::SelectTool, zoom::ZoomTool,
};

/// Distances and steps the tools use. The shell fills the screen distances from `theme::Tokens`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ToolSettings {
    /// How near the pointer must be to hit an object, in screen points.
    pub hit_tolerance: f64,
    /// How near a dragged point must be to a target to snap, in screen points.
    pub snap_distance: f64,
    /// How far the pointer must move before a press becomes a drag, in screen points.
    pub drag_threshold: f64,
    /// Arrow-key steps in font units: plain, with `Shift`, with `Cmd` (`move.arrowKeyIncrements`).
    pub increments: [f64; 3],
    /// The selection's bounding box and its knobs are showing (`view.showBoundingBox`).
    pub bounding_box: bool,
    /// The reference point placed with the Rotate or Scale tool (`transform.setOrigin`).
    pub reference: Option<Point>,
    /// The TrueType Instructor's hinting direction (`hint.tt.toggleDirection`).
    pub tt_direction: crate::tt_instructor::TtDirection,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            hit_tolerance: 6.0,
            snap_distance: 4.0,
            drag_threshold: 3.0,
            increments: [1.0, 10.0, 100.0],
            bounding_box: false,
            reference: None,
            tt_direction: Default::default(),
        }
    }
}

impl ToolSettings {
    /// Largest arrow-key step a font may set.
    pub const MAX_INCREMENT: f64 = 10_000.0;

    /// These settings with the arrow-key steps the font sets, where it sets usable ones
    /// (`fontInfo.keyboardIncrements`).
    pub fn with_font(mut self, font: &Font) -> Self {
        let s = &font.info.settings;
        for (slot, value) in self.increments.iter_mut().zip([s.keyboard_increment, s.keyboard_increment_big, s.keyboard_increment_huge]) {
            if let Some(v) = value
                && v.is_finite()
                && v > 0.0
                && v <= Self::MAX_INCREMENT
            {
                *slot = v;
            }
        }
        self
    }
}

/// What a tool reads for one event. The tool never changes any of it: it returns actions.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub scene: &'a Scene<'a>,
    pub view: View,
    pub selection: &'a Selection,
    pub settings: ToolSettings,
}

/// A pointer tool: a state machine from events to actions.
pub trait Tool: Send {
    /// The tool's id, as `tool.select` names it.
    fn id(&self) -> &'static str;

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output;

    /// A key the tool may use (only `Escape` for the tools here). Unused keys return an output
    /// that is not `handled`.
    fn key(&mut self, _cx: &Context<'_>, _ev: &KeyEvent) -> Output {
        Output::default()
    }

    /// The modifier keys changed without the pointer moving.
    fn modifiers(&mut self, _cx: &Context<'_>, _mods: Modifiers) -> Output {
        Output::default()
    }

    /// Abandons the gesture in progress (Escape, a tool switch, the edited glyph changing).
    fn cancel(&mut self) -> Output;

    /// A gesture is in progress.
    fn busy(&self) -> bool;

    /// What the canvas draws for the tool now.
    fn overlays(&self) -> Vec<Overlay> {
        Vec::new()
    }
}

/// A tool as `tool.list` describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ToolInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub shortcut: &'static str,
    /// Tools sharing a toolbar button share a group and a shortcut letter.
    pub group: &'static str,
}

/// The tools of this crate, in toolbar order.
pub const TOOLS: &[ToolInfo] = &[
    ToolInfo { id: "select", label: "Select", shortcut: "V", group: "select" },
    ToolInfo { id: "lasso", label: "Lasso Select", shortcut: "V", group: "select" },
    ToolInfo { id: "draw", label: "Draw", shortcut: "P", group: "" },
    ToolInfo { id: "pencil", label: "Pencil", shortcut: "B", group: "pencil" },
    ToolInfo { id: "rectangle", label: "Primitives (Rectangle)", shortcut: "F", group: "primitives" },
    ToolInfo { id: "ellipse", label: "Primitives (Ellipse)", shortcut: "F", group: "primitives" },
    ToolInfo { id: "text", label: "Text", shortcut: "T", group: "" },
    ToolInfo { id: "hand", label: "Hand", shortcut: "H", group: "" },
    ToolInfo { id: "zoom", label: "Zoom", shortcut: "Z", group: "" },
    ToolInfo { id: "knife", label: "Knife", shortcut: "E", group: "erase" },
    ToolInfo { id: "rotate", label: "Rotate", shortcut: "R", group: "" },
    ToolInfo { id: "scale", label: "Scale", shortcut: "S", group: "" },
    ToolInfo { id: "measurement", label: "Measurement", shortcut: "L", group: "" },
    ToolInfo { id: "thickness", label: "Thickness", shortcut: "", group: "" },
    ToolInfo { id: "trueTypeInstructor", label: "TrueType Instructor", shortcut: "I", group: "" },
    ToolInfo { id: "annotationSelect", label: "Annotation Selection", shortcut: "A", group: "annotation" },
    ToolInfo { id: "annotationText", label: "Text Annotation", shortcut: "A", group: "annotation" },
    ToolInfo { id: "annotationArrow", label: "Arrow Annotation", shortcut: "A", group: "annotation" },
    ToolInfo { id: "annotationCircle", label: "Circle Annotation", shortcut: "A", group: "annotation" },
    ToolInfo { id: "annotationPlus", label: "Plus Annotation", shortcut: "A", group: "annotation" },
    ToolInfo { id: "annotationMinus", label: "Minus Annotation", shortcut: "A", group: "annotation" },
];

/// A new tool by id, or `None` for an unknown id.
pub fn make_tool(id: &str) -> Option<Box<dyn Tool>> {
    let tool: Box<dyn Tool> = match id {
        "select" => Box::new(SelectTool::default()),
        "lasso" => Box::new(LassoTool::default()),
        "text" => Box::new(crate::text::TextTool),
        "hand" => Box::new(HandTool::default()),
        "zoom" => Box::new(ZoomTool::default()),
        "draw" => Box::new(DrawTool::default()),
        "pencil" => Box::new(PencilTool::default()),
        "rectangle" => Box::new(PrimitiveTool::rectangle()),
        "ellipse" => Box::new(PrimitiveTool::ellipse()),
        "knife" => Box::new(crate::knife::KnifeTool::default()),
        "rotate" => Box::new(RotateTool::default()),
        "scale" => Box::new(ScaleTool::default()),
        "measurement" => Box::new(MeasureTool::default()),
        "thickness" => Box::new(crate::thickness::ThicknessTool::default()),
        "trueTypeInstructor" => Box::new(crate::tt_instructor::TtInstructorTool::default()),
        _ if id.starts_with("annotation") => Box::new(AnnotateTool::new(id)?),
        _ => return None,
    };
    Some(tool)
}
