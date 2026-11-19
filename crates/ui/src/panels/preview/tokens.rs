//! The previews' colours and sizes.

use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewTokens {
    /// The text and the ground before Invert swaps them.
    pub ink: Color32,
    pub paper: Color32,
    /// The room Always Center Active Glyph off leaves at the edge the text keeps flush with, in points.
    pub margin: f32,
    /// The Preview Area's height until one is set, and the least it may have, in points (strip and controls).
    pub area_height: f64,
    pub area_min_height: f64,
}

impl PreviewTokens {
    pub fn standard() -> PreviewTokens {
        PreviewTokens { ink: Color32::from_rgb(0x10, 0x10, 0x10), paper: Color32::WHITE, margin: 12.0, area_height: 140.0, area_min_height: 64.0 }
    }
}
