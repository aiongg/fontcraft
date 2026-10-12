//! The reference font's glyph drawn behind the outline (`view.setReferenceFont`): the selected
//! reference font's glyph for the edited character, as an outline at the edited glyph's origin,
//! scaled from the reference font's units per em to the edited font's.

use egui::Stroke;
use fontcraft_geom::Affine;

use super::{Painting, Scene};
use crate::panels::Pending;

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, pending);
    let state = &scene.state.panels.reference_font;
    if !state.show {
        return;
    }
    let Some(reference) = state.loaded.0.as_deref() else { return };
    let Some(edited) = scene.tab.edited_glyph().and_then(|g| scene.font.glyph(g)) else { return };
    let Some(glyph) = reference.glyph_for(edited, &state.character, &state.language) else { return };
    let Some(layer) = reference.layer(glyph) else { return };
    let outline = super::background::outline(&reference.font, &layer.shapes, &layer.master);
    let k = f64::from(scene.font.units_per_em.max(1)) / f64::from(reference.font.units_per_em.max(1));
    let v = c.view;
    let to_screen = Affine::translate(v.origin.to_vec2()) * Affine::scale_non_uniform(v.scale() * k, -v.scale() * k);
    let stroke = Stroke::new(c.tokens.reference_font_outline_width, c.tokens.reference_font_outline);
    for contour in super::flatten(&(to_screen * outline)) {
        c.painter.add(egui::Shape::line(contour, stroke));
    }
}
