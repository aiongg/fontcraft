//! The preview of the selected instance on the Exports tab, one line high, filled in the text colour. It shrinks to the
//! width there is.

use craft::theme::Srgba;
use craft::ui::runtime::env::{Env, Label};
use egui::{Color32, Sense, Shape, Ui, vec2};
use fontcraft_geom::Affine;

use crate::app::models::instance_preview::InstancePreview;
use crate::canvas::paint::{fill::nonzero, flatten};

/// The preview's height.
const HEIGHT: f32 = 48.0;

pub fn view(ui: &mut Ui, label: &Label, preview: &InstancePreview) {
    let room = ui.available_width().max(1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(room, HEIGHT), Sense::hover());
    let name = label.as_str().to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, name.clone()));
    let tall = preview.top - preview.bottom;
    let scale = (f64::from(rect.height()) / tall).min(f64::from(rect.width()) / preview.width);
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let baseline = f64::from(rect.top()) + preview.top * scale;
    let affine = Affine::new([scale, 0.0, 0.0, -scale, f64::from(rect.left()), baseline]);
    let ink = Color32::from(Srgba::from(Env::of(ui).tokens.color.text.normal));
    if let Some(mesh) = nonzero(&flatten(&preview.outline, affine), rect.intersect(ui.clip_rect()), ink) {
        ui.painter().add(Shape::mesh(mesh));
    }
}
