//! The size of an intermediate layer of a corner glyph (`popover.cornerLayerSize`): its width and height in percent of
//! the corner, which scaled corner components are interpolated from (`corner.scaleInterpolatesWithIntermediateLayers`).
//!
//! It opens with `{glyph, layer}`, from the Layers panel's Intermediate on a corner glyph, and gives the layer the name
//! the corner fitting reads (`layers.rename`): `Width 150 Height 100`. The fields start from the layer's present name,
//! 100 % for what it leaves out.

use fontcraft_engine::Session;
use fontcraft_engine::model::corner::{MAX_CORNER_SCALE, layer_size};
use serde_json::{Map, Value, json};

use super::super::fields::Field;
use super::super::spec::{DialogSpec, Fields, Shaped};
use super::no_link;

/// Opened for a corner glyph's layer, never as "the dialog of `layers.rename`".
pub const CORNER_LAYER_SIZE: DialogSpec = DialogSpec {
    follow_up: true,
    ..DialogSpec::new(
        "popover.cornerLayerSize",
        "Corner Layer Size",
        "layers.rename",
        Fields::Shaped(Shaped { fields, given, params, link: no_link }),
    )
};

/// The prefix of corner glyphs' names.
pub const CORNER_PREFIX: &str = "_corner.";

/// Smallest size, in percent.
const MIN_PERCENT: f64 = 1.0;

fn fields(_: &Session) -> Vec<Field> {
    let max = MAX_CORNER_SCALE * 100.0;
    vec![
        Field::number("width", "Width %", MIN_PERCENT, max, 100.0, "The corner width this layer is drawn at, in percent"),
        Field::number("height", "Height %", MIN_PERCENT, max, 100.0, "The corner height this layer is drawn at, in percent"),
    ]
}

fn given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    const TITLE: &str = "Corner Layer Size";
    let font = session.font().map_err(|e| e.to_string())?;
    let text = |key: &str| given.get(key).and_then(Value::as_str).ok_or_else(|| format!("{TITLE}: give `{key}`"));
    let (name, id) = (text("glyph")?, text("layer")?);
    if !name.starts_with(CORNER_PREFIX) {
        return Err(format!("{TITLE}: `{name}` is not a corner glyph"));
    }
    let glyph = font.glyph(name).ok_or_else(|| format!("{TITLE}: there is no glyph `{name}`"))?;
    let layer = glyph.layers.iter().find(|l| l.id == id).ok_or_else(|| format!("{TITLE}: glyph `{name}` has no layer `{id}`"))?;
    if layer.is_master_layer() {
        return Err(format!("{TITLE}: a master layer is the corner at 100 %: add a layer under the master for another size"));
    }
    let (width, height) = layer.info.name.as_deref().and_then(layer_size).unwrap_or((100.0, 100.0));
    let mut out = given.clone();
    out.entry("width").or_insert_with(|| json!(width));
    out.entry("height").or_insert_with(|| json!(height));
    Ok(out)
}

fn params(values: &Map<String, Value>) -> Map<String, Value> {
    let percent = |key: &str| values.get(key).and_then(Value::as_f64).unwrap_or(100.0);
    let mut m = Map::new();
    m.insert("name".into(), json!(format!("Width {} Height {}", percent("width"), percent("height"))));
    m
}
