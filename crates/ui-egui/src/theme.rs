//! Design tokens. Colours, radii and sizes come from here and nowhere else, so themes and the
//! sibling apps' look stay one table (`docs/ui-design.md`).

use egui::Color32;

/// The values every widget draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub app_bar: Color32,
    pub panel: Color32,
    pub panel_darker: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub hover: Color32,
    pub accent: Color32,
    pub row_selected: Color32,
    /// Text drawn on `row_selected`.
    pub selected_text: Color32,
    /// The glyph canvas behind the outlines.
    pub canvas: Color32,
    /// Filled glyph shapes in cells and previews.
    pub glyph_fill: Color32,
    /// Outline strokes in Edit View.
    pub outline: Color32,
    /// Baseline, x-height, cap height, ascender and descender lines.
    pub metric_line: Color32,
    /// On-curve corner nodes.
    pub node_corner: Color32,
    /// On-curve smooth nodes.
    pub node_smooth: Color32,
    /// Off-curve handles.
    pub node_handle: Color32,
    /// The line from a handle to its node.
    pub handle_line: Color32,
    /// The strokes across the ends of an open path.
    pub open_end: Color32,
    /// The triangle at the first node of an open path.
    pub open_end_start: Color32,
    /// The circle around two neighbouring nodes at the same place.
    pub duplicate_node: Color32,
    /// Behind a node on a metric line (diamond) or in an alignment zone (circle).
    pub metric_highlight: Color32,
    /// Alignment zones, as bands along their metric lines.
    pub zone_fill: Color32,
    /// Names on the metric lines.
    pub metric_name: Color32,
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
    /// The Knife tool's cutting line while it is dragged.
    pub knife_line: Color32,
    pub radius: u8,
    pub font_size: f32,
    /// Outline stroke width in Edit View, in points.
    pub outline_width: f32,
    pub metric_line_width: f32,
    pub handle_line_width: f32,
    /// Side of a corner node's square.
    pub node_size: f32,
    pub smooth_node_radius: f32,
    pub handle_radius: f32,
    /// Length of the start node's arrowhead.
    pub start_arrow_size: f32,
    /// Length of the stroke across an open path's end.
    pub open_end_length: f32,
    pub duplicate_node_radius: f32,
    /// Size of the highlight behind a node on a metric line or in a zone.
    pub metric_highlight_size: f32,
    pub metric_name_size: f32,
    /// Ring around a selected node or handle.
    pub selection_ring_radius: f32,
    /// Stroke of selection rings and selected segments.
    pub selection_width: f32,
    pub snap_line_width: f32,
    /// Ring around the snap target and the hovered point.
    pub marker_radius: f32,
    /// How near the pointer must be to hit a node, a handle or a segment, in points.
    pub hit_tolerance: f32,
    /// How near a dragged point must come to a target to snap to it, in points (`03a` question 28).
    pub snap_distance: f32,
    /// How far the pointer must move before a press becomes a drag, in points.
    pub drag_threshold: f32,
    /// Share of the canvas left free on each side when a glyph is fitted into it.
    pub fit_margin: f32,
    pub tab_min_width: f32,
    pub palette_width: f32,
    /// Side of a glyph cell in Font View, in points.
    pub cell_size: f32,
    /// Height of the name line under a glyph cell.
    pub cell_label_height: f32,
    /// Gap between glyph cells.
    pub cell_gap: f32,
    /// Size of the glyph name under a cell.
    pub cell_label_size: f32,
}

impl Tokens {
    /// Medium dark, the default the sibling apps start in.
    pub fn dark() -> Self {
        Self {
            dark: true,
            app_bar: Color32::from_rgb(0x32, 0x32, 0x32),
            panel: Color32::from_rgb(0x3c, 0x3c, 0x3c),
            panel_darker: Color32::from_rgb(0x2a, 0x2a, 0x2a),
            border: Color32::from_rgb(0x22, 0x22, 0x22),
            text: Color32::from_rgb(0xdd, 0xdd, 0xdd),
            text_dim: Color32::from_rgb(0x9a, 0x9a, 0x9a),
            hover: Color32::from_rgb(0x4a, 0x4a, 0x4a),
            accent: Color32::from_rgb(0x2d, 0x8c, 0xeb),
            row_selected: Color32::from_rgb(0x1f, 0x5f, 0xa8),
            selected_text: Color32::WHITE,
            canvas: Color32::from_rgb(0xf4, 0xf4, 0xf4),
            glyph_fill: Color32::from_rgb(0x1a, 0x1a, 0x1a),
            outline: Color32::from_rgb(0x20, 0x20, 0x20),
            metric_line: Color32::from_rgb(0xb0, 0xb8, 0xc4),
            node_corner: Color32::from_rgb(0x2d, 0x6c, 0xdf),
            node_smooth: Color32::from_rgb(0x2f, 0xa8, 0x4f),
            node_handle: Color32::from_rgb(0x88, 0x88, 0x88),
            handle_line: Color32::from_rgb(0xa8, 0xa8, 0xa8),
            open_end: Color32::from_rgb(0x1e, 0x7b, 0xe6),
            open_end_start: Color32::from_rgb(0x8c, 0xc4, 0xf5),
            duplicate_node: Color32::from_rgb(0xe0, 0x30, 0x30),
            metric_highlight: Color32::from_rgb(0xdc, 0xc4, 0x8c),
            zone_fill: Color32::from_rgba_unmultiplied(0x9a, 0xb8, 0xdc, 0x38),
            metric_name: Color32::from_rgb(0x80, 0x88, 0x94),
            selection: Color32::from_rgb(0xf0, 0x8c, 0x1e),
            marquee_fill: Color32::from_rgba_unmultiplied(0x2d, 0x8c, 0xeb, 0x24),
            marquee_stroke: Color32::from_rgb(0x2d, 0x8c, 0xeb),
            snap_line: Color32::from_rgb(0xd0, 0x2c, 0x9c),
            hover_point: Color32::from_rgb(0x2d, 0x8c, 0xeb),
            locked_point: Color32::from_rgb(0xe0, 0x28, 0x28),
            knife_line: Color32::from_rgb(0xe0, 0x28, 0x28),
            radius: 3,
            font_size: 12.0,
            outline_width: 1.5,
            metric_line_width: 1.0,
            handle_line_width: 1.0,
            node_size: 7.0,
            smooth_node_radius: 4.0,
            handle_radius: 2.5,
            start_arrow_size: 11.0,
            open_end_length: 16.0,
            duplicate_node_radius: 7.5,
            metric_highlight_size: 18.0,
            metric_name_size: 10.0,
            selection_ring_radius: 6.5,
            selection_width: 2.0,
            snap_line_width: 1.0,
            marker_radius: 7.0,
            hit_tolerance: 6.0,
            snap_distance: 4.0,
            drag_threshold: 3.0,
            fit_margin: 0.08,
            tab_min_width: 72.0,
            palette_width: 220.0,
            cell_size: 64.0,
            cell_label_height: 16.0,
            cell_gap: 4.0,
            cell_label_size: 10.0,
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            app_bar: Color32::from_rgb(0xe6, 0xe6, 0xe6),
            panel: Color32::from_rgb(0xf0, 0xf0, 0xf0),
            panel_darker: Color32::from_rgb(0xdc, 0xdc, 0xdc),
            border: Color32::from_rgb(0xc4, 0xc4, 0xc4),
            text: Color32::from_rgb(0x22, 0x22, 0x22),
            text_dim: Color32::from_rgb(0x6a, 0x6a, 0x6a),
            hover: Color32::from_rgb(0xd0, 0xd0, 0xd0),
            row_selected: Color32::from_rgb(0xb4, 0xd4, 0xf8),
            selected_text: Color32::from_rgb(0x10, 0x10, 0x10),
            canvas: Color32::WHITE,
            ..Self::dark()
        }
    }

    /// Applies the tokens to egui's own widgets.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.extreme_bg_color = self.panel_darker;
        visuals.override_text_color = Some(self.text);
        visuals.selection.bg_fill = self.row_selected;
        visuals.widgets.hovered.bg_fill = self.hover;
        visuals.widgets.noninteractive.bg_stroke.color = self.border;
        ctx.set_visuals(visuals);
    }
}
