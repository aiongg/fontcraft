//! The glyph area's view models.

use craft::ui::contract::{PlainData, Tracked};
use serde_json::{Value, json};

use super::ProjectCx;
use crate::shell::masters;

/// The shown glyph's metrics in the layer it shows (the Layers panel's pick, else its layer of the current master), as
/// `spacing.getMetrics` reports them.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GlyphMetrics {
    /// The glyph's name; `None` without an open document or glyph.
    pub glyph: Option<String>,
    /// The id of the layer the values are read from and edits go to; `None` leaves the choice to the commands.
    pub layer: Option<String>,
    pub width: Option<f64>,
    /// `None` for an empty layer.
    pub left: Option<f64>,
    /// `None` for an empty layer.
    pub right: Option<f64>,
}

/// The glyph the glyph panels show: the selected one, else the font's first.
pub fn shown_glyph(cx: &ProjectCx<'_>) -> Option<String> {
    let font = cx.session.font().ok()?;
    match &cx.view.glyph {
        Some(name) if font.glyphs.get(name).is_some() => Some(name.clone()),
        _ => font.glyphs.first().map(|g| g.name.clone()),
    }
}

pub fn project(model: &mut Tracked<GlyphMetrics>, cx: &mut ProjectCx<'_>) {
    let Some(glyph) = shown_glyph(cx) else {
        super::set(model, GlyphMetrics::default());
        return;
    };
    let layer = cx.session.font().ok().and_then(|font| {
        let shown = font.glyph(&glyph)?;
        masters::shown_layer(font, &cx.view.layers, shown).map(|l| l.id.clone())
    });
    let mut params = json!({ "glyph": glyph });
    if let Some(id) = &layer
        && let Some(map) = params.as_object_mut()
    {
        map.insert("layer".into(), json!(id));
    }
    let reply = cx.session.query("spacing.getMetrics", &params).unwrap_or(Value::Null);
    let number = |key: &str| reply.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
    super::set(model, GlyphMetrics { width: number("width"), left: number("left"), right: number("right"), glyph: Some(glyph), layer });
}
