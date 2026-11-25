//! The Higher-Order Interpolation tool's colours and sizes. The diamonds' and the straight paths' colours are named in
//! the reference (blue, light yellow, dark purple; grey); the rest are our choices.

use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoiTokens {
    /// The nodes of the edited layer, the next layer down the axis and the next layer up.
    pub here: Color32,
    pub below: Color32,
    pub above: Color32,
    /// The edge of every diamond.
    pub diamond_edge: Color32,
    /// Half a diamond's diagonal.
    pub diamond_radius: f32,
    /// The dotted line and the knob of a straight path.
    pub line: Color32,
    pub linear_knob: Color32,
    pub dot_spacing: f32,
    pub dot_radius: f32,
    /// A curve, its knobs, and the lines from its handles and rotation centre.
    pub curve: Color32,
    pub knob: Color32,
    pub arm: Color32,
    pub curve_width: f32,
    pub line_width: f32,
    pub knob_radius: f32,
    /// The timing graph's dots: the one at constant speed, the one that follows the graph, and their radius.
    pub clock_dot: Color32,
    pub timing_dot: Color32,
    pub clock_radius: f32,
}

impl HoiTokens {
    pub fn standard() -> HoiTokens {
        HoiTokens {
            here: Color32::from_rgb(0x2d, 0x6c, 0xe0),
            below: Color32::from_rgb(0xf7, 0xe7, 0x7d),
            above: Color32::from_rgb(0x55, 0x25, 0x86),
            diamond_edge: Color32::from_rgba_unmultiplied(0x20, 0x20, 0x20, 0x90),
            diamond_radius: 4.5,
            line: Color32::from_rgb(0x9a, 0x9a, 0x9a),
            linear_knob: Color32::from_rgb(0x8c, 0x8c, 0x8c),
            dot_spacing: 5.0,
            dot_radius: 1.0,
            curve: Color32::from_rgb(0x1f, 0x9d, 0x55),
            knob: Color32::from_rgb(0x1b, 0x86, 0x4a),
            arm: Color32::from_rgba_unmultiplied(0x1f, 0x9d, 0x55, 0x90),
            curve_width: 1.5,
            line_width: 1.0,
            knob_radius: 4.0,
            clock_dot: Color32::WHITE,
            timing_dot: Color32::from_rgb(0x1f, 0x9d, 0x55),
            clock_radius: 3.5,
        }
    }
}
