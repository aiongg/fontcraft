//! Master icons: the 25 built-in icons, a lowercase n at five weights and five widths, drawn in
//! code, and a glyph of the font drawn as an icon. The drawings are original to FontCraft
//! (`ASSETS.md`): a stem and an arch whose stroke grows with the weight and whose stems move
//! apart with the width, laid out on a unit square with y pointing down.
//!
//! The icon names are the engine's (`font.master.setIcon`): the weight alone for the normal
//! width, else `Weight_Width`. A master without an icon, or whose glyph icon the font lacks, is
//! drawn with the Regular icon.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};
use fontcraft_engine::cmd::masters::MASTER_ICONS;
use fontcraft_geom::{Affine, Shape as _};
use fontcraft_model::{Font, Master};

/// The weights, lightest first: the rows of the picker.
pub const WEIGHTS: usize = 5;
/// The widths, narrowest first: the columns of the picker.
pub const WIDTHS: usize = 5;
/// The icon a master without one shows.
pub const DEFAULT_ICON: &str = "Regular";

/// Stroke thickness per weight, as a share of the icon's side.
const STROKE: [f32; WEIGHTS] = [0.06, 0.1, 0.14, 0.18, 0.23];
/// Distance between the two stems' centres per width, as a share of the icon's side.
const SPAN: [f32; WIDTHS] = [0.26, 0.34, 0.42, 0.5, 0.6];
/// The x-height and the baseline of the drawing, as shares of the side from the top.
const X_HEIGHT: f32 = 0.3;
const BASELINE: f32 = 0.84;
/// Most vertical room the arch takes below the x-height, as a share of the side.
const ARCH_DEPTH: f32 = 0.2;
/// Points along the arch.
const ARCH_STEPS: usize = 16;
/// Room left around a glyph icon, as a share of the side.
const GLYPH_MARGIN: f64 = 0.1;

/// The weight row and width column of a built-in icon name.
pub fn grid_cell(name: &str) -> Option<(usize, usize)> {
    let at = MASTER_ICONS.iter().position(|n| *n == name)?;
    Some((at / WIDTHS, at % WIDTHS))
}

/// The built-in icon at a weight row and width column.
pub fn name_at(weight: usize, width: usize) -> Option<&'static str> {
    if width >= WIDTHS {
        return None;
    }
    MASTER_ICONS.get(weight.checked_mul(WIDTHS)?.checked_add(width)?).copied()
}

/// A readable label of a built-in icon: `Bold Condensed`.
pub fn label(name: &str) -> String {
    name.replace('_', " ")
}

/// Draws the built-in icon `name` in `rect`. False when `name` is not a built-in icon.
pub fn paint_builtin(painter: &Painter, rect: Rect, name: &str, color: Color32) -> bool {
    let Some((weight, width)) = grid_cell(name) else { return false };
    let (Some(stroke), Some(span)) = (STROKE.get(weight), SPAN.get(width)) else { return false };
    let side = rect.width().min(rect.height());
    let origin = rect.center() - egui::vec2(side, side) / 2.0;
    let at = |x: f32, y: f32| Pos2::new(origin.x + x * side, origin.y + y * side);
    let pen = Stroke::new(stroke * side, color);
    let left = 0.5 - span / 2.0;
    let right = 0.5 + span / 2.0;
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
    true
}

/// Draws glyph `glyph` of `font` as it is in master `master`, fitted into `rect`. False when the
/// font lacks the glyph or its outline is empty or cannot be drawn.
pub fn paint_glyph(painter: &Painter, rect: Rect, font: &Font, glyph: &str, master: &str, color: Color32) -> bool {
    let Ok(path) = font.outline(glyph, master) else { return false };
    let bounds = path.bounding_box();
    let (w, h) = (bounds.width(), bounds.height());
    if !(w.is_finite() && h.is_finite()) || (w <= 0.0 && h <= 0.0) {
        return false;
    }
    let side = f64::from(rect.width().min(rect.height())) * (1.0 - 2.0 * GLYPH_MARGIN);
    let scale = side / w.max(h);
    let center = rect.center();
    let to_rect = Affine::translate((f64::from(center.x), f64::from(center.y)))
        * Affine::scale_non_uniform(scale, -scale)
        * Affine::translate((-bounds.center().x, -bounds.center().y));
    let pen = Stroke::new((rect.height() / 24.0).max(1.0), color);
    for contour in crate::canvas::flatten(&(to_rect * path)) {
        painter.add(Shape::line(contour, pen));
    }
    true
}

/// Draws the icon of `master`: its built-in icon, else the glyph it names, else the Regular
/// icon.
pub fn paint_master(painter: &Painter, rect: Rect, font: &Font, master: &Master, color: Color32) {
    let icon = master.icon.as_deref().unwrap_or(DEFAULT_ICON);
    if paint_builtin(painter, rect, icon, color) {
        return;
    }
    if font.glyph(icon).is_some() && paint_glyph(painter, rect, font, icon, &master.id, color) {
        return;
    }
    paint_builtin(painter, rect, DEFAULT_ICON, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_25_icons_fill_a_five_by_five_grid_and_each_draws() {
        let ctx = egui::Context::default();
        let mut seen = std::collections::HashSet::new();
        for weight in 0..WEIGHTS {
            for width in 0..WIDTHS {
                let name = name_at(weight, width).unwrap();
                assert_eq!(grid_cell(name), Some((weight, width)));
                assert!(seen.insert(name));
            }
        }
        assert_eq!(seen.len(), MASTER_ICONS.len());
        assert_eq!(name_at(0, WIDTHS), None);
        assert_eq!(name_at(WEIGHTS, 0), None);
        assert_eq!(label("SemiBold_Extended"), "SemiBold Extended");
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(32.0, 32.0));
            assert!(paint_builtin(painter, rect, "Bold_Condensed", Color32::WHITE));
            assert!(!paint_builtin(painter, rect, "Heavy", Color32::WHITE), "an unknown name draws nothing");
        });
    }
}
