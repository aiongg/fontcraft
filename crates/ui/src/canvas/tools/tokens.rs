//! The colours and sizes of the selection and the tools' overlays, and the distances the tools measure in screen
//! points. The handbook does not say how a selection looks; these are our choices.

use egui::Color32;

/// The tool layers' colours and sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolTokens {
    /// Selected nodes, handles and segments.
    pub selection: Color32,
    /// Inside of a marquee or a Zoom tool rectangle.
    pub marquee_fill: Color32,
    /// Edge of a marquee, a lasso or a Zoom tool rectangle.
    pub marquee_stroke: Color32,
    /// Snapping lines and the point a drag snaps to.
    pub snap_line: Color32,
    /// The point under the pointer.
    pub hover_point: Color32,
    /// A point under the pointer that cannot be moved (`paths.lockedPointsHoverRed`).
    pub locked_point: Color32,
    /// The Knife tool's cutting line.
    pub knife_line: Color32,
    /// The segment a drawing tool would add next, and freehand strokes.
    pub draw_preview: Color32,
    /// The Measurement tool's ruler.
    pub measure_ruler: Color32,
    pub selection_ring_radius: f32,
    /// Stroke of selection rings and selected segments.
    pub selection_width: f32,
    pub snap_line_width: f32,
    pub measure_ruler_width: f32,
    /// Ring around the snap target and the hovered point.
    pub marker_radius: f32,
    /// How near the pointer must be to an object to hit it, in points.
    pub hit_tolerance: f32,
    /// How near a dragged point must come to a target to snap to it, in points.
    pub snap_distance: f32,
    /// How far the pointer moves before a press becomes a drag, in points.
    pub drag_threshold: f32,
}

impl ToolTokens {
    pub fn standard() -> ToolTokens {
        ToolTokens {
            selection: Color32::from_rgb(0xf0, 0x8c, 0x1e),
            marquee_fill: Color32::from_rgba_unmultiplied(0x2d, 0x8c, 0xeb, 0x24),
            marquee_stroke: Color32::from_rgb(0x2d, 0x8c, 0xeb),
            snap_line: Color32::from_rgb(0xd0, 0x2c, 0x9c),
            hover_point: Color32::from_rgb(0x2d, 0x8c, 0xeb),
            locked_point: Color32::from_rgb(0xe0, 0x28, 0x28),
            knife_line: Color32::from_rgb(0xe0, 0x28, 0x28),
            draw_preview: Color32::from_rgba_unmultiplied(0x20, 0x20, 0x20, 0x90),
            measure_ruler: Color32::from_rgb(0x00, 0x8c, 0x9c),
            selection_ring_radius: 6.5,
            selection_width: 2.0,
            snap_line_width: 1.0,
            measure_ruler_width: 1.0,
            marker_radius: 7.0,
            hit_tolerance: 6.0,
            snap_distance: 4.0,
            drag_threshold: 3.0,
        }
    }
}
