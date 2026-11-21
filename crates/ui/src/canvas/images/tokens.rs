//! The look of placed images: the missing-image marker and the selected image's outline and knobs. The handbook does
//! not say how they look; these are our choices.

use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageTokens {
    /// The missing-image marker's lines and the name of a format kept as a reference.
    pub marker: Color32,
    /// Side of the missing-image marker, in screen points.
    pub marker_size: f32,
    pub marker_width: f32,
    /// The name of a format kept as a reference, as a share of the marker's side.
    pub marker_text: f32,
    /// The selected image's outline (in the selection colour).
    pub outline_width: f32,
    /// Side of a knob, in screen points.
    pub knob_size: f32,
}

impl ImageTokens {
    pub fn standard() -> ImageTokens {
        ImageTokens {
            marker: Color32::from_rgb(0xd0, 0x40, 0x40),
            marker_size: 24.0,
            marker_width: 1.5,
            marker_text: 0.35,
            outline_width: 1.5,
            knob_size: 7.0,
        }
    }

    /// The marker drawn `side` screen points large, for Font View cells.
    pub fn with_marker(self, side: f32) -> ImageTokens {
        ImageTokens { marker_size: side, ..self }
    }
}
