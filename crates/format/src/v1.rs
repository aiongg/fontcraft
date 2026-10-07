//! Format version 1 (the M0 skeleton's), read and upgraded to the current model.
//!
//! Version 1 stored master metrics as four fixed fields, gave layers no id of their own, wrote
//! component transforms as six numbers, and kept unknown keys as JSON values beside the known
//! ones. The upgrade maps each to its version 2 form: metric definitions with per-master values,
//! master layers whose id is the master's, [`Transform::Matrix`], and native-family [`Extra`]
//! entries.

use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Layout, Number, Value};
use fontcraft_model::{Extra, Family, GlyphTable, MetricDef, MetricValue, NodeKind, Transform, metric};
use serde::Deserialize;
use serde_json::Map;

type JsonExtra = Map<String, serde_json::Value>;

fn one() -> u16 {
    1
}
fn yes() -> bool {
    true
}
fn identity() -> [f64; 6] {
    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
}

#[derive(Deserialize)]
pub struct Font {
    family_name: String,
    units_per_em: u16,
    #[serde(default = "one")]
    version_major: u16,
    #[serde(default)]
    version_minor: u16,
    masters: Vec<Master>,
    glyphs: Vec<Glyph>,
    #[serde(flatten)]
    extra: JsonExtra,
}

#[derive(Deserialize)]
struct Master {
    id: String,
    name: String,
    ascender: f64,
    cap_height: f64,
    x_height: f64,
    descender: f64,
    #[serde(flatten)]
    extra: JsonExtra,
}

#[derive(Deserialize)]
struct Glyph {
    name: String,
    #[serde(default)]
    unicodes: Vec<u32>,
    #[serde(default = "yes")]
    export: bool,
    #[serde(default)]
    layers: Vec<Layer>,
    #[serde(flatten)]
    extra: JsonExtra,
}

#[derive(Deserialize)]
struct Layer {
    master: String,
    width: f64,
    #[serde(default)]
    shapes: Vec<Shape>,
    #[serde(default)]
    anchors: Vec<Anchor>,
    #[serde(flatten)]
    extra: JsonExtra,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Shape {
    Path(Path),
    Component(Component),
}

#[derive(Deserialize)]
struct Path {
    #[serde(default = "yes")]
    closed: bool,
    nodes: Vec<Node>,
    #[serde(flatten)]
    extra: JsonExtra,
}

#[derive(Deserialize)]
struct Node {
    x: f64,
    y: f64,
    #[serde(rename = "type")]
    kind: NodeKind,
    #[serde(default)]
    smooth: bool,
    #[serde(flatten)]
    extra: JsonExtra,
}

#[derive(Deserialize)]
struct Component {
    glyph: String,
    #[serde(default = "identity")]
    transform: [f64; 6],
    #[serde(flatten)]
    extra: JsonExtra,
}

#[derive(Deserialize)]
struct Anchor {
    name: String,
    x: f64,
    y: f64,
    #[serde(flatten)]
    extra: JsonExtra,
}

/// A JSON value as a property-list value. JSON has no data type and property lists have no null
/// or boolean: null becomes an empty string, booleans become 0 and 1.
fn plist_of(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::String("".into()),
        serde_json::Value::Bool(b) => Value::Number(Number::int(i64::from(b))),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Number(Number::int(i)),
            None => n.as_f64().and_then(Number::float).map_or_else(|| Value::String(n.to_string().into()), Value::Number),
        },
        serde_json::Value::String(s) => Value::String(s.into()),
        serde_json::Value::Array(a) => Value::Array(Array { items: a.into_iter().map(plist_of).collect(), layout: Layout::Multiline }),
        serde_json::Value::Object(o) => {
            let mut d = Dict::new();
            for (k, v) in o {
                d.entries.push((k.into(), plist_of(v)));
            }
            Value::Dict(d)
        }
    }
}

fn extra_of(json: JsonExtra) -> Extra {
    let mut x = Extra::new();
    for (k, v) in json {
        x.push(Family::Native, k, plist_of(v));
    }
    x
}

impl Font {
    pub fn upgrade(self) -> fontcraft_model::Font {
        let kinds = [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::DESCENDER];
        let masters = self
            .masters
            .into_iter()
            .map(|m| fontcraft_model::Master {
                id: m.id,
                name: m.name,
                metric_values: [m.ascender, m.cap_height, m.x_height, m.descender].into_iter().map(|pos| MetricValue { pos, over: 0.0 }).collect(),
                extra: extra_of(m.extra),
            })
            .collect();
        let glyphs = self.glyphs.into_iter().map(|g| Arc::new(g.upgrade())).collect();
        fontcraft_model::Font {
            source: fontcraft_model::Source::default(),
            family_name: self.family_name,
            units_per_em: self.units_per_em,
            version_major: self.version_major,
            version_minor: self.version_minor,
            metrics: Arc::new(kinds.iter().map(|k| MetricDef::new(k)).collect()),
            masters: Arc::new(masters),
            glyphs: GlyphTable::from_glyphs(glyphs),
            extra: Arc::new(extra_of(self.extra)),
        }
    }
}

impl Glyph {
    fn upgrade(self) -> fontcraft_model::Glyph {
        fontcraft_model::Glyph {
            name: self.name,
            unicodes: self.unicodes,
            export: self.export,
            layers: self.layers.into_iter().map(Layer::upgrade).collect(),
            extra: extra_of(self.extra),
        }
    }
}

impl Layer {
    fn upgrade(self) -> fontcraft_model::Layer {
        fontcraft_model::Layer {
            id: self.master.clone(),
            master: self.master,
            width: self.width,
            shapes: self
                .shapes
                .into_iter()
                .map(|s| match s {
                    Shape::Path(p) => fontcraft_model::Shape::Path(fontcraft_model::Path {
                        closed: p.closed,
                        nodes: p
                            .nodes
                            .into_iter()
                            .map(|n| fontcraft_model::Node { x: n.x, y: n.y, kind: n.kind, smooth: n.smooth, extra: extra_of(n.extra) })
                            .collect(),
                        extra: extra_of(p.extra),
                    }),
                    Shape::Component(c) => fontcraft_model::Shape::Component(fontcraft_model::Component {
                        glyph: c.glyph,
                        transform: Transform::Matrix(c.transform),
                        extra: extra_of(c.extra),
                    }),
                })
                .collect(),
            anchors: self.anchors.into_iter().map(|a| fontcraft_model::Anchor { name: a.name, x: a.x, y: a.y, extra: extra_of(a.extra) }).collect(),
            extra: extra_of(self.extra),
        }
    }
}
