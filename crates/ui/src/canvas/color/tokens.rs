//! The look of the gradient knobs and the limits of the colour texture. The handbook does not say how a knob looks;
//! these are our choices.

use egui::Color32;

/// Colour drawing's colours and sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTokens {
    /// A knob's dot and its ring.
    pub knob: Color32,
    pub knob_outline: Color32,
    /// A knob's radius in screen points.
    pub knob_radius: f32,
    pub knob_outline_width: f32,
    /// The line from a linear gradient's start knob to its end knob.
    pub knob_line: Color32,
    pub knob_line_width: f32,
}

impl ColorTokens {
    pub fn standard() -> ColorTokens {
        ColorTokens {
            knob: Color32::WHITE,
            knob_outline: Color32::from_rgb(0x20, 0x20, 0x20),
            knob_radius: 5.0,
            knob_outline_width: 1.5,
            knob_line: Color32::from_rgba_unmultiplied(0x80, 0x80, 0x80, 0xc0),
            knob_line_width: 1.0,
        }
    }
}
