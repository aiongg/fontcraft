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
    /// What the next click of a drawing tool would add, and a freehand stroke while it is drawn.
    pub draw_preview: Color32,
    /// Messages that say what went wrong (a dialog's failed preview).
    pub error_text: Color32,
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
    /// Distance from the window's top edge to a dialog.
    pub dialog_top: f32,
    /// Text size of a dialog's title.
    pub dialog_title_size: f32,
    /// Laid over the window behind an open dialog; light, so the dialog's preview stays readable.
    pub dialog_backdrop: Color32,
    /// Width of a number field in a dialog.
    pub dialog_field_width: f32,
    /// Gap between a dialog's rows and columns.
    pub dialog_spacing: f32,
    /// Side of a glyph cell in Font View, in points.
    pub cell_size: f32,
    /// Height of the name line under a glyph cell.
    pub cell_label_height: f32,
    /// Gap between glyph cells.
    pub cell_gap: f32,
    /// Size of the glyph name under a cell.
    pub cell_label_size: f32,
    /// The selection's bounding box and its knobs.
    pub bounding_box: Color32,
    /// Side of a bounding-box knob.
    pub knob_size: f32,
    /// The transformation origin: ring and crosshair under the Rotate and Scale tools.
    pub transform_origin: Color32,
    pub transform_origin_radius: f32,
    /// Palette and Info box (`panels`): inner margin of the Palette, width of a number field,
    /// one cell of the nine-point grid, one Fit Curve step button and the gap in its row, the indent of non-master
    /// layer rows, the Info box's distance from the canvas's lower edge, the bottom bar's margin.
    pub palette_margin: f32,
    pub field_width: f32,
    pub point_grid_cell: f32,
    pub fit_curve_step: f32,
    pub fit_curve_gap: f32,
    pub layers_indent: f32,
    pub info_box_margin: f32,
    pub bottom_bar_margin: f32,
    /// Local guides (`guides.localVersusGlobal`: blue).
    pub guide_local: Color32,
    /// Global guides (red).
    pub guide_global: Color32,
    pub guide_width: f32,
    /// The knob at a guide's position: filled when the guide is selected.
    pub guide_knob_radius: f32,
    /// A guide's name, at its left end.
    pub guide_name_size: f32,
    /// The font's grid, drawn while its step is at least `grid_min_spacing` points on screen.
    pub grid_line: Color32,
    pub grid_min_spacing: f32,
    /// The CJK grid, the CJK guide box and the design grid.
    pub cjk_grid: Color32,
    /// Measurement tool labels (`measurement.labelColors`): on-curve coordinates.
    pub measure_on_curve: Color32,
    /// Off-curve coordinates.
    pub measure_off_curve: Color32,
    /// Deltas between neighbouring on-curve nodes.
    pub measure_delta: Color32,
    /// The ruler, its crossings and their distances; measurement guides use their own colour.
    pub measure_ruler: Color32,
    pub measure_ruler_width: f32,
    pub measure_label_size: f32,
    /// Dots where a ruler or a measurement guide crosses the outline.
    pub measure_crossing_radius: f32,
    /// The inactive side of the layer under Show Background (`background.outlineDisplay`).
    pub background_outline: Color32,
    pub background_outline_width: f32,
    /// Other layers of the glyph whose eye is on (`layers.visibleLayerOutline`).
    pub visible_layer_outline: Color32,
    pub visible_layer_outline_width: f32,
    /// Laid over the canvas while the background is the side being edited (`background.darkenedCanvas`).
    pub background_tint: Color32,
    /// Arrows, circles, plus and minus signs, and the edges of notes.
    pub annotation: Color32,
    pub annotation_width: f32,
    /// Inside a text note.
    pub annotation_note_fill: Color32,
    pub annotation_text: Color32,
    pub annotation_text_size: f32,
    /// The handle of a selected annotation.
    pub annotation_handle_radius: f32,
    /// The edited layer's paths as drawn, under the black composite outline, when strokes or
    /// masks make the two differ (`stroke.display`, `mask.display`).
    pub source_outline: Color32,
    pub source_outline_width: f32,
    /// Audit markers by severity (`audit.canvasMarkers`): badges, rings and leader lines.
    pub audit_fail: Color32,
    pub audit_warn: Color32,
    pub audit_info: Color32,
    /// Reported segments and paths: their width, and how much of the severity colour they keep.
    pub audit_highlight_width: f32,
    pub audit_highlight_alpha: f32,
    /// The ring around a reported node or handle, and the leader line to its badge.
    pub audit_ring_radius: f32,
    pub audit_ring_width: f32,
    /// A result's badge, and how far up and right of the reported spot it sits.
    pub audit_badge_radius: f32,
    pub audit_badge_offset: f32,
    /// Edit View's toolbar: a button's side, its icon's side, the space between buttons and the
    /// width of a shared button's menu.
    pub toolbar_button: f32,
    pub toolbar_icon: f32,
    pub toolbar_gap: f32,
    pub toolbar_menu_width: f32,
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
            draw_preview: Color32::from_rgba_unmultiplied(0x20, 0x20, 0x20, 0x90),
            error_text: Color32::from_rgb(0xe0, 0x50, 0x50),
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
            dialog_top: 48.0,
            dialog_title_size: 16.0,
            dialog_backdrop: Color32::from_black_alpha(40),
            dialog_field_width: 72.0,
            dialog_spacing: 8.0,
            cell_size: 64.0,
            cell_label_height: 16.0,
            cell_gap: 4.0,
            cell_label_size: 10.0,
            bounding_box: Color32::from_rgb(0x5a, 0x6e, 0x8c),
            knob_size: 6.0,
            transform_origin: Color32::from_rgb(0xe8, 0x40, 0x40),
            transform_origin_radius: 6.0,
            palette_margin: 8.0,
            field_width: 52.0,
            point_grid_cell: 9.0,
            fit_curve_step: 14.0,
            fit_curve_gap: 2.0,
            layers_indent: 12.0,
            info_box_margin: 12.0,
            bottom_bar_margin: 2.0,
            guide_local: Color32::from_rgb(0x2d, 0x7d, 0xd8),
            guide_global: Color32::from_rgb(0xd8, 0x3a, 0x3a),
            guide_width: 1.0,
            guide_knob_radius: 4.0,
            guide_name_size: 10.0,
            grid_line: Color32::from_rgba_unmultiplied(0x90, 0x98, 0xa4, 0x30),
            grid_min_spacing: 8.0,
            cjk_grid: Color32::from_rgba_unmultiplied(0x6a, 0x9a, 0x6a, 0x90),
            measure_on_curve: Color32::from_rgb(0x1e, 0x5f, 0xd0),
            measure_off_curve: Color32::from_rgb(0x2a, 0x9a, 0x3a),
            measure_delta: Color32::from_rgb(0xd0, 0x2c, 0x2c),
            measure_ruler: Color32::from_rgb(0x00, 0x8c, 0x9c),
            measure_ruler_width: 1.0,
            measure_label_size: 10.0,
            measure_crossing_radius: 3.0,
            background_outline: Color32::from_rgba_unmultiplied(0xe0, 0x40, 0x40, 0x90),
            background_outline_width: 1.0,
            visible_layer_outline: Color32::from_rgba_unmultiplied(0x2d, 0x8c, 0xeb, 0xb0),
            visible_layer_outline_width: 0.75,
            background_tint: Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0x18),
            annotation: Color32::from_rgb(0xe0, 0x30, 0x30),
            annotation_width: 2.0,
            annotation_note_fill: Color32::from_rgba_unmultiplied(0xff, 0xf4, 0xb0, 0xe0),
            annotation_text: Color32::from_rgb(0x30, 0x30, 0x30),
            annotation_text_size: 12.0,
            annotation_handle_radius: 4.0,
            source_outline: Color32::from_rgb(0xb4, 0xb4, 0xb4),
            source_outline_width: 1.0,
            audit_fail: Color32::from_rgb(0xe5, 0x48, 0x4d),
            audit_warn: Color32::from_rgb(0xf2, 0xa1, 0x1d),
            audit_info: Color32::from_rgb(0x3b, 0x82, 0xf6),
            audit_highlight_width: 4.0,
            audit_highlight_alpha: 0.45,
            audit_ring_radius: 9.0,
            audit_ring_width: 1.5,
            audit_badge_radius: 5.0,
            audit_badge_offset: 14.0,
            toolbar_button: 28.0,
            toolbar_icon: 18.0,
            toolbar_gap: 2.0,
            toolbar_menu_width: 200.0,
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
        // Checkboxes and other boxed widgets stand out from the panel they sit on.
        visuals.widgets.inactive.bg_fill = self.panel_darker;
        visuals.widgets.noninteractive.bg_stroke.color = self.border;
        ctx.set_visuals(visuals);
    }
}
