//! The colours and sizes of the hint layers, in screen points (`hinting.ps.manual.display`,
//! `hinting.tt.instructions.badgeColours`, `hinting.tt.tool.masterOutline`, `hint.tt.showPointIndexes`).

use egui::Color32;

/// The hint layers' colours and sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HintTokens {
    /// A horizontal stem's bar.
    pub ps_horizontal: Color32,
    /// A vertical stem's bar.
    pub ps_vertical: Color32,
    /// A ghost hint's band.
    pub ps_ghost: Color32,
    pub ps_ghost_band: f32,
    pub ps_badge: Color32,
    pub ps_badge_selected: Color32,
    pub ps_badge_text: Color32,
    pub ps_badge_text_size: f32,
    pub ps_badge_rounding: f32,
    /// The ring of a badge whose hint has warnings, and the warnings in the Info box.
    pub ps_warning: Color32,
    pub ps_warning_width: f32,
    pub ps_marker: Color32,
    pub ps_marker_width: f32,
    /// Distance from the glyph's origin (or the descender) to the markers' column (row).
    pub lane: f32,
    pub badge_size: [f32; 2],
    pub marker_radius: f32,
    pub gap: f32,
    /// The TrueType badges by instruction type.
    pub tt_snap: Color32,
    pub tt_stem: Color32,
    pub tt_triple: Color32,
    pub tt_shift: Color32,
    pub tt_interpolate: Color32,
    pub tt_delta: Color32,
    /// Instructions of the other direction.
    pub tt_dimmed: Color32,
    /// A badge's rim.
    pub tt_rim: Color32,
    /// A selected badge's ring, a selected node.
    pub tt_selected: Color32,
    /// The on-curve nodes while the Instructor is active.
    pub tt_node: Color32,
    pub tt_node_radius: f32,
    pub tt_badge_radius: f32,
    /// How far a badge sits from the point it moves.
    pub tt_badge_offset: f32,
    pub tt_line_width: f32,
    pub tt_rim_width: f32,
    pub tt_arrow_size: f32,
    pub tt_selected_gap: f32,
    pub tt_index_text: Color32,
    pub tt_index_size: f32,
    pub tt_index_gap: f32,
    /// The previewed instance before hinting.
    pub tt_instance_outline: Color32,
    /// The grid-fitted outline.
    pub tt_fitted_outline: Color32,
    pub tt_preview_outline_width: f32,
    /// The pixel backdrop's ink.
    pub tt_pixel: Color32,
}

impl HintTokens {
    pub fn standard() -> HintTokens {
        HintTokens {
            ps_horizontal: Color32::from_rgba_unmultiplied(0xf2, 0xc9, 0x1f, 0x40),
            ps_vertical: Color32::from_rgba_unmultiplied(0x3c, 0xb4, 0x4b, 0x40),
            ps_ghost: Color32::from_rgba_unmultiplied(0x2d, 0x7d, 0xd2, 0x60),
            ps_ghost_band: 6.0,
            ps_badge: Color32::from_rgb(0x8a, 0x8a, 0x90),
            ps_badge_selected: Color32::from_rgb(0xf2, 0x96, 0x2a),
            ps_badge_text: Color32::WHITE,
            ps_badge_text_size: 10.0,
            ps_badge_rounding: 2.0,
            ps_warning: Color32::from_rgb(0xe0, 0x40, 0x30),
            ps_warning_width: 1.5,
            ps_marker: Color32::from_rgb(0x2d, 0x7d, 0xd2),
            ps_marker_width: 1.5,
            lane: 18.0,
            badge_size: [64.0, 16.0],
            marker_radius: 5.0,
            gap: 6.0,
            tt_snap: Color32::from_rgb(0x9b, 0x4d, 0xd6),
            tt_stem: Color32::from_rgb(0x4a, 0xb4, 0xf0),
            tt_triple: Color32::from_rgb(0x9b, 0x4d, 0xd6),
            tt_shift: Color32::from_rgb(0x1d, 0x3c, 0x9e),
            tt_interpolate: Color32::from_rgb(0x2f, 0xa0, 0x4b),
            tt_delta: Color32::from_rgb(0xf2, 0x8a, 0x1f),
            tt_dimmed: Color32::from_rgba_unmultiplied(0x9a, 0x9a, 0x9a, 0x90),
            tt_rim: Color32::WHITE,
            tt_selected: Color32::from_rgb(0xf0, 0x8c, 0x1e),
            tt_node: Color32::from_rgb(0x80, 0x80, 0x80),
            tt_node_radius: 5.5,
            tt_badge_radius: 6.0,
            tt_badge_offset: 16.0,
            tt_line_width: 1.5,
            tt_rim_width: 1.0,
            tt_arrow_size: 8.0,
            tt_selected_gap: 2.5,
            tt_index_text: Color32::from_rgb(0xc0, 0x39, 0x2b),
            tt_index_size: 10.0,
            tt_index_gap: 4.0,
            tt_instance_outline: Color32::from_rgb(0x3a, 0x7b, 0xe8),
            tt_fitted_outline: Color32::from_rgb(0xe0, 0x3c, 0x31),
            tt_preview_outline_width: 1.25,
            tt_pixel: Color32::from_rgba_unmultiplied(0x9a, 0x9a, 0x9a, 0x70),
        }
    }
}

impl HintTokens {
    /// The tokens with the PostScript hint bands faded to `opacity`, from 0 to 1 (`appearance.guideOpacity`).
    pub fn with_opacity(opacity: f32) -> HintTokens {
        let mut t = HintTokens::standard();
        let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
        for c in [&mut t.ps_horizontal, &mut t.ps_vertical, &mut t.ps_ghost] {
            *c = c.gamma_multiply(opacity);
        }
        t
    }
}

impl Default for HintTokens {
    fn default() -> Self {
        HintTokens::standard()
    }
}
