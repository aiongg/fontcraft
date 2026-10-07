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
    pub radius: u8,
    pub font_size: f32,
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
            radius: 3,
            font_size: 12.0,
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
