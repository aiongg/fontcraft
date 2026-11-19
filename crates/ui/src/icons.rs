//! The icons of the tools on the tool rail, from craft's icon catalog.
//!
//! The rail shows a tool's icon from its command (`CommandDef::with_icon`); the catalog names it here.

/// The catalog icon of the tool `id` (`fontcraft_tools::TOOLS`).
pub fn for_tool(id: &str) -> &'static str {
    match id {
        "select" => "pointer",
        "lasso" => "lasso",
        "draw" => "pen_tool",
        "pencil" => "pencil",
        "pixel" => "grid_small",
        "rectangle" => "rectangle",
        "ellipse" => "circle",
        "text" => "text",
        "hand" => "hand",
        "zoom" => "zoom_in",
        "knife" => "slice_knife",
        "rotate" => "rotate_cw",
        "scale" => "scaling",
        "measurement" => "ruler",
        "thickness" => "stroke_align_center",
        "magnet" => "snap",
        "rapid" => "spline",
        "spiro" => "pen_curvature",
        "trueTypeInstructor" => "pin",
        "annotationSelect" => "object_select",
        "annotationText" => "note",
        "annotationArrow" => "navigation",
        "annotationCircle" => "target",
        "annotationPlus" => "plus",
        "annotationMinus" => "minus",
        _ => "more",
    }
}
