//! The colours and sizes of the edit view's text: the kerned gaps (`kerning.highlightColors`), the selection, the
//! cursor, the other glyphs of the text, the group members and the measurement line.

use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextTokens {
    /// A gap the kerning closes.
    pub kerning_negative: Color32,
    /// A gap the kerning opens.
    pub kerning_positive: Color32,
    pub selection: Color32,
    pub cursor: Color32,
    pub cursor_width: f32,
    /// The text's glyphs besides the edited one.
    pub glyph: Color32,
    pub glyph_width: f32,
    /// Show Group Members.
    pub group_members: Color32,
    pub group_members_width: f32,
    /// The measurement line, its numbers and the edges of the advances.
    pub measurement: Color32,
    pub measurement_width: f32,
    pub measurement_label: Color32,
    pub measurement_label_size: f32,
    /// Between the line and its numbers, in points.
    pub measurement_label_gap: f32,
    pub measurement_advance: Color32,
    pub measurement_advance_width: f32,
    /// Below this many points of advance a glyph gets no numbers: they would overlap.
    pub measurement_min_advance: f32,
}

impl TextTokens {
    pub fn standard() -> TextTokens {
        TextTokens {
            kerning_negative: Color32::from_rgba_unmultiplied(0x8c, 0xc8, 0xf5, 0x90),
            kerning_positive: Color32::from_rgba_unmultiplied(0xf5, 0xdc, 0x46, 0xa0),
            selection: Color32::from_rgba_unmultiplied(0x2d, 0x8c, 0xeb, 0x40),
            cursor: Color32::from_rgb(0x2d, 0x8c, 0xeb),
            cursor_width: 1.5,
            glyph: Color32::from_rgb(0x20, 0x20, 0x20),
            glyph_width: 1.0,
            group_members: Color32::from_rgba_unmultiplied(0x3b, 0x82, 0xf6, 0xa0),
            group_members_width: 1.0,
            measurement: Color32::from_rgb(0x1e, 0x9e, 0x8a),
            measurement_width: 1.0,
            measurement_label: Color32::from_rgb(0x14, 0x6e, 0x62),
            measurement_label_size: 10.0,
            measurement_label_gap: 2.0,
            measurement_advance: Color32::from_rgb(0xb8, 0xb8, 0xbc),
            measurement_advance_width: 0.5,
            measurement_min_advance: 24.0,
        }
    }
}
