//! The Info box's placed-image part (`panels::info_box_image`), read from `image.getInfo`.

use craft::ui::contract::PlainData;
use serde_json::{Map, Value, json};

/// The selected image as `image.getInfo` reports it, with the targets its commands take.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ImageBox {
    pub glyph: String,
    pub layer: String,
    /// `Some("background")` while the background's image is selected.
    pub side: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub rotation: Option<f64>,
    /// Why the file cannot be drawn.
    pub problem: Option<String>,
}

impl ImageBox {
    pub fn targets(&self, extra: Value) -> Value {
        let mut m = Map::new();
        m.insert("glyph".into(), json!(self.glyph));
        m.insert("layer".into(), json!(self.layer));
        if let Some(side) = &self.side {
            m.insert("side".into(), json!(side));
        }
        if let Value::Object(e) = extra {
            m.extend(e);
        }
        Value::Object(m)
    }
}

/// The box of the image `targets` names (`glyph`, `layer`, `side`), from `image.getInfo`; `None` when that layer side
/// has no image.
pub fn image_box(session: &mut fontcraft_engine::Session, targets: &Map<String, Value>) -> Option<ImageBox> {
    let info = session.query("image.getInfo", &Value::Object(targets.clone())).ok()?;
    let image = info.get("image").filter(|i| i.is_object())?;
    let num = |key: &str| image.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
    let text = |key: &str| targets.get(key).and_then(Value::as_str).map(str::to_string);
    Some(ImageBox {
        glyph: text("glyph")?,
        layer: text("layer")?,
        side: text("side"),
        x: num("x"),
        y: num("y"),
        width: num("width"),
        height: num("height"),
        rotation: num("rotation"),
        problem: image.get("problem").and_then(Value::as_str).map(str::to_string),
    })
}
