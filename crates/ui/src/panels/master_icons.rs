//! Master icons as icon glyphs: the 25 built-in icons, a lowercase n at five weights and five widths drawn in code,
//! and a glyph of the font drawn as an icon (`app::models::layers::MasterIcon`). The drawings are original to
//! FontCraft (`ASSETS.md`): a stem and an arch whose stroke grows with the weight and whose stems move apart with
//! the width, laid out on a unit square with y pointing down.

use craft::ui::primitives::Glyph;
use craft::ui::style::UiTokens;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

use crate::app::models::layers::MasterIcon;

/// Stroke thickness per weight, as a share of the icon's side.
const STROKE: [f32; 5] = [0.06, 0.1, 0.14, 0.18, 0.23];
/// Distance between the two stems' centres per width, as a share of the icon's side.
const SPAN: [f32; 5] = [0.26, 0.34, 0.42, 0.5, 0.6];
/// The x-height and the baseline of the drawing, as shares of the side from the top.
const X_HEIGHT: f32 = 0.3;
const BASELINE: f32 = 0.84;
/// Most vertical room the arch takes below the x-height, as a share of the side.
const ARCH_DEPTH: f32 = 0.2;
/// Points along the arch.
const ARCH_STEPS: usize = 16;
/// A glyph icon's line width, as a share of the side.
const GLYPH_LINE: f32 = 1.0 / 24.0;

/// A master's icon, painted in the colour of the control that shows it.
pub struct MasterGlyph<'a>(pub &'a MasterIcon);

impl Glyph for MasterGlyph<'_> {
    fn paint(&self, painter: &Painter, _: &UiTokens, rect: Rect, color: Color32, _: bool) {
        let side = rect.width().min(rect.height());
        let origin = rect.center() - egui::Vec2::splat(side) / 2.0;
        let at = |x: f32, y: f32| Pos2::new(origin.x + x * side, origin.y + y * side);
        match self.0 {
            MasterIcon::Builtin { weight, width } => {
                let (Some(stroke), Some(span)) = (STROKE.get(usize::from(*weight)), SPAN.get(usize::from(*width))) else { return };
                let pen = Stroke::new(stroke * side, color);
                let (left, right) = (0.5 - span / 2.0, 0.5 + span / 2.0);
                let depth = (span / 2.0).min(ARCH_DEPTH);
                let top = X_HEIGHT + stroke / 2.0;
                // The left stem, from a little above the arch's top down to the baseline.
                painter.add(Shape::line_segment([at(left, X_HEIGHT - stroke * 0.25), at(left, BASELINE)], pen));
                // The arch, from the left stem over to the right one, then the right stem down.
                let mut points: Vec<Pos2> = (0..=ARCH_STEPS)
                    .map(|i| {
                        let t = std::f32::consts::PI * (1.0 - i as f32 / ARCH_STEPS as f32);
                        at(0.5 + span / 2.0 * t.cos(), top + depth - depth * t.sin())
                    })
                    .collect();
                points.push(at(right, BASELINE));
                painter.add(Shape::line(points, pen));
            }
            MasterIcon::Outline(contours) => {
                let pen = Stroke::new(side * GLYPH_LINE, color);
                for contour in contours {
                    painter.add(Shape::line(contour.iter().map(|[x, y]| at(*x, *y)).collect(), pen));
                }
            }
        }
    }
}
