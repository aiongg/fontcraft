//! Palette › Dimensions' view model: the notepad of the master the canvas shows, one field per key of the shown
//! glyph's script, as `dimensions.getValues` reports them.

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use serde_json::{Map, Value, json};

use super::ProjectCx;
use super::glyph::shown_glyph;
use crate::shell::dock::PanelId;

/// Most fields the panel lists; a font can note any number of keys.
pub const MAX_FIELDS: usize = 64;

/// One field of the notepad.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct DimensionField {
    /// The field key (`nV`, `HH`).
    pub key: String,
    /// The number noted; `None` for an empty field.
    pub value: Option<f64>,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct DimensionsModel {
    /// The master whose notepad shows; `None` without a font master.
    pub master: Option<String>,
    /// The fields of the shown glyph's script, or every noted key when the script has no known fields.
    pub fields: Vec<DimensionField>,
}

pub fn project(model: &mut Tracked<DimensionsModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout().panel_state(&PanelId::Dimensions) == PanelState::Hidden {
        return;
    }
    let master = cx.session.font().ok().and_then(|font| crate::shell::masters::panel_master(font, cx.view).map(|m| m.id.clone()));
    let Some(master) = master else {
        super::set(model, DimensionsModel::default());
        return;
    };
    let mut query = Map::new();
    query.insert("master".into(), json!(master));
    // Several glyphs selected in Font View share the notepad of the first one's script.
    let font_view = cx.view.tabs.font_view_shows();
    if let Some(glyph) = shown_glyph(cx).or_else(|| cx.view.font_view.selected.first().filter(|_| font_view).cloned()) {
        query.insert("glyph".into(), json!(glyph));
    }
    let Ok(reply) = cx.session.query("dimensions.getValues", &Value::Object(query)) else {
        super::set(model, DimensionsModel::default());
        return;
    };
    let empty = Map::new();
    let values = reply.get("values").and_then(Value::as_object).unwrap_or(&empty);
    let keys: Vec<String> = match reply.get("fields").and_then(Value::as_array) {
        Some(fields) => fields.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => values.keys().cloned().collect(),
    };
    let fields = keys
        .into_iter()
        .take(MAX_FIELDS)
        .map(|key| DimensionField { value: values.get(&key).and_then(Value::as_f64).filter(|n| n.is_finite()), key })
        .collect();
    super::set(model, DimensionsModel { master: Some(master), fields });
}
