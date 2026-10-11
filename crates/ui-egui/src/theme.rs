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
    /// The text cursor in text mode.
    pub text_cursor: Color32,
    pub text_cursor_width: f32,
    /// Behind the selected items of the tab text.
    pub text_selection: Color32,
    /// The gap of a negatively and of a positively kerned pair in text mode
    /// (`kerning.highlightColors`; the defaults of the `appearance.color.kerning…` settings for
    /// the light canvas).
    pub kerning_negative: Color32,
    pub kerning_positive: Color32,
    /// The sidebearings and widths under each glyph in text mode with Show Metrics on
    /// (`spacing.metricsOverlayInTextMode`).
    pub sidebearing_label: Color32,
    pub sidebearing_label_size: f32,
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
    /// Radius of the three dots of a dialog's round actions button, and the distance between
    /// their centres.
    pub dialog_dot_radius: f32,
    pub dialog_dots_gap: f32,
    /// Side of a glyph cell in Font View, in points.
    pub cell_size: f32,
    /// Height of the name line under a glyph cell.
    pub cell_label_height: f32,
    /// Gap between glyph cells.
    pub cell_gap: f32,
    /// Size of the glyph name under a cell.
    pub cell_label_size: f32,
    /// The glyph name under a cell that is not selected (cells are light in both themes).
    pub cell_label: Color32,
    /// Margin around Font View's grid and list.
    pub font_view_margin: f32,
    /// Space between a cell's edge and its glyph.
    pub cell_inset: f32,
    /// Width of the ring around a selected cell.
    pub cell_selection_width: f32,
    /// The cut-out at the top left of a cell that only its layer colours, as a share of the side.
    pub cell_cutout: f32,
    /// The twelve colour labels (`glyph.colorLabelIndices`), and the opacity a cell is tinted with.
    pub color_labels: [Color32; 12],
    pub color_label_alpha: u8,
    /// Cell badges: annotation note, custom glyph info, not exported.
    pub badge_size: f32,
    pub badge_note: Color32,
    pub badge_info: Color32,
    pub badge_info_text: Color32,
    pub badge_non_export: Color32,
    /// The Unicode marker beside a cell's name, and the size of a code point printed there.
    pub unicode_marker: Color32,
    pub unicode_marker_size: f32,
    pub cell_value_size: f32,
    /// A `glyphOrder` section header across the grid: its height and text size.
    pub section_header_height: f32,
    pub section_header_size: f32,
    /// List View: row and header height, the width unit columns are measured in, the Char
    /// column's text size.
    pub list_row_height: f32,
    pub list_header_height: f32,
    pub list_column_unit: f32,
    pub list_char_size: f32,
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
    /// The glyph Info box: a metrics key out of sync with its metric, a key naming a missing
    /// glyph, and the space between the glyph's descender and the top of the box.
    pub info_box_key_out_of_sync: Color32,
    pub info_box_key_unlinked: Color32,
    pub info_box_glyph_gap: f32,
    /// Show Group Members: the outline of each member drawn behind the edited pair.
    pub group_members: Color32,
    pub group_members_width: f32,
    /// The Kerning window's entries: a group (`@name`) and a single glyph.
    pub kerning_group_text: Color32,
    pub kerning_glyph_text: Color32,
    /// Font View's warning triangle for out-of-sync metrics keys, and its exclamation mark.
    pub font_view_badge_out_of_sync: Color32,
    pub font_view_badge_out_of_sync_mark: Color32,
    /// The previews (Preview Area, Preview Panel): text and ground before Invert, the area's
    /// default and smallest height, the separator's grab height, the room a flush line keeps
    /// from the edge, the controls row's height and the panel's first size.
    pub preview_ink: Color32,
    pub preview_paper: Color32,
    pub preview_area_height: f32,
    pub preview_area_min_height: f32,
    pub preview_separator: f32,
    pub preview_margin: f32,
    pub preview_controls_height: f32,
    pub preview_panel_size: [f32; 2],
    /// The text-mode measurement line (`measurementLine.display`): the line, the sidebearings
    /// printed on it, and the thin lines at the edges of each advance.
    pub measurement_line: Color32,
    pub measurement_line_width: f32,
    pub measurement_line_label: Color32,
    pub measurement_line_label_size: f32,
    pub measurement_line_advance: Color32,
    pub measurement_line_advance_width: f32,
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
            text_cursor: Color32::from_rgb(0x2d, 0x8c, 0xeb),
            text_cursor_width: 1.5,
            text_selection: Color32::from_rgba_unmultiplied(0x2d, 0x8c, 0xeb, 0x40),
            kerning_negative: Color32::from_rgba_unmultiplied(0x8c, 0xc8, 0xf5, 0x90),
            kerning_positive: Color32::from_rgba_unmultiplied(0xf5, 0xdc, 0x46, 0xa0),
            sidebearing_label: Color32::from_rgb(0x60, 0x68, 0x74),
            sidebearing_label_size: 10.0,
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
            dialog_dot_radius: 1.5,
            dialog_dots_gap: 4.5,
            cell_size: 64.0,
            cell_label_height: 16.0,
            cell_gap: 4.0,
            cell_label_size: 10.0,
            cell_label: Color32::from_rgb(0x4a, 0x4a, 0x4a),
            font_view_margin: 8.0,
            cell_inset: 4.0,
            cell_selection_width: 2.0,
            cell_cutout: 0.3,
            color_labels: [
                Color32::from_rgb(0xe0, 0x4b, 0x4b),
                Color32::from_rgb(0xf0, 0x92, 0x3c),
                Color32::from_rgb(0xa5, 0x6f, 0x3c),
                Color32::from_rgb(0xf2, 0xd3, 0x3c),
                Color32::from_rgb(0x9a, 0xd4, 0x6a),
                Color32::from_rgb(0x3f, 0x9a, 0x4f),
                Color32::from_rgb(0x4c, 0xc4, 0xd8),
                Color32::from_rgb(0x4a, 0x7f, 0xe0),
                Color32::from_rgb(0x9b, 0x62, 0xd4),
                Color32::from_rgb(0xe8, 0x7b, 0xb8),
                Color32::from_rgb(0xc4, 0xc4, 0xc4),
                Color32::from_rgb(0x7a, 0x7a, 0x7a),
            ],
            color_label_alpha: 150,
            badge_size: 10.0,
            badge_note: Color32::from_rgb(0xf5, 0xd0, 0x40),
            badge_info: Color32::from_rgb(0x10, 0x10, 0x10),
            badge_info_text: Color32::WHITE,
            badge_non_export: Color32::from_rgb(0xd8, 0x30, 0x30),
            unicode_marker: Color32::from_rgb(0x8a, 0x8a, 0x8a),
            unicode_marker_size: 5.0,
            cell_value_size: 8.0,
            section_header_height: 24.0,
            section_header_size: 12.0,
            list_row_height: 20.0,
            list_header_height: 22.0,
            list_column_unit: 40.0,
            list_char_size: 15.0,
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
            audit_badge_offset: 20.0,
            toolbar_button: 28.0,
            toolbar_icon: 18.0,
            toolbar_gap: 2.0,
            toolbar_menu_width: 200.0,
            info_box_key_out_of_sync: Color32::from_rgb(0xe8, 0x3a, 0x3a),
            info_box_key_unlinked: Color32::from_rgb(0x1f, 0xbf, 0xb8),
            info_box_glyph_gap: 8.0,
            group_members: Color32::from_rgba_unmultiplied(0x3b, 0x82, 0xf6, 0xa0),
            group_members_width: 1.0,
            kerning_group_text: Color32::from_rgb(0x5c, 0xa8, 0xf5),
            kerning_glyph_text: Color32::from_rgb(0xd9, 0xb2, 0x48),
            font_view_badge_out_of_sync: Color32::from_rgb(0xf2, 0xa1, 0x1d),
            font_view_badge_out_of_sync_mark: Color32::from_rgb(0x10, 0x10, 0x10),
            preview_ink: Color32::BLACK,
            preview_paper: Color32::WHITE,
            preview_area_height: 140.0,
            preview_area_min_height: 48.0,
            preview_separator: 6.0,
            preview_margin: 16.0,
            preview_controls_height: 30.0,
            preview_panel_size: [640.0, 200.0],
            measurement_line: Color32::from_rgb(0x1e, 0x9e, 0x8a),
            measurement_line_width: 1.0,
            measurement_line_label: Color32::from_rgb(0x14, 0x6e, 0x62),
            measurement_line_label_size: 10.0,
            measurement_line_advance: Color32::from_rgb(0xb8, 0xb8, 0xbc),
            measurement_line_advance_width: 0.5,
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
            kerning_group_text: Color32::from_rgb(0x1f, 0x62, 0xc4),
            kerning_glyph_text: Color32::from_rgb(0x9a, 0x6e, 0x00),
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
