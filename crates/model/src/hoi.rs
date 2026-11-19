//! A node's higher-order interpolation (HOI) data: per axis, a curve the node follows between its
//! positions on two layers in place of the straight line, or a timing along that line.
//!
//! The `.glyphs` format version 4 keeps it in a node's attribute dictionary under `hoi`, keyed by
//! axis tag, in the node of the lower layer of the span along that axis. Versions 2 and 3 have no
//! equivalent. A UFO keeps it in the node's glyphsLib lib key with the node's other attribute
//! keys. The model reads, keeps and writes it; `fontcraft_interp::hoi` evaluates it for static
//! instances (`hoi.defaultLinear`, `hoi.curveTypes`). Variable fonts do not carry it yet.
//!
//! | Curve | `.glyphs` version 4 |
//! |---|---|
//! | [`HoiCurve::Point`] | `{ip = (x,y);}` |
//! | [`HoiCurve::Rotation`] | `rc = (x,y);` or `rc = auto;`, and `rd = cw;` or `rd = ccw;`, each optional |
//! | [`HoiCurve::Timing`] | `al`, `au`, `tl`, `tu`, each optional |
//!
//! Written from GlyphsSDK's version 4 format description (Apache-2.0).

use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value};
use serde::{Deserialize, Serialize};

use crate::{MAX_AXIS_VALUES, MAX_NAME_LEN, in_range};

/// The time of a timing handle the file leaves out.
pub const DEFAULT_HOI_TIME: f64 = 1.0 / 3.0;

/// A node's HOI data, one entry per axis tag in the order the file gives them. At most
/// [`MAX_AXIS_VALUES`] entries, each tag used once.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hoi(pub Vec<HoiAxis>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoiAxis {
    /// The axis tag as written; a tag may be longer than four characters (see [`crate::Axis`]).
    pub axis: String,
    pub curve: HoiCurve,
}

/// How a node travels along one axis span.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoiCurve {
    /// A curve through an intermediate point (`ip`).
    Point { x: f64, y: f64 },
    /// A rotation about a centre (`rc`) in a direction (`rd`). With no direction the smaller angle
    /// is used.
    Rotation {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        center: Option<HoiCenter>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<HoiDirection>,
    },
    /// A Bézier timing along the straight line.
    Timing(HoiTiming),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoiCenter {
    /// Found from the shared centre of the lower and the upper shape (`auto`).
    Auto,
    At {
        x: f64,
        y: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoiDirection {
    Clockwise,
    CounterClockwise,
}

/// The two handles of a timing. A time is a fraction of the way from the node to its position on
/// the other layer; an amplitude is the offset across the line, in percent of its length. `None`
/// is a value the file leaves out (times [`DEFAULT_HOI_TIME`], amplitudes 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HoiTiming {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower_time: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_time: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower_amplitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_amplitude: Option<f64>,
}

impl HoiTiming {
    /// The four values, with the defaults filled in: lower time, upper time, lower amplitude,
    /// upper amplitude.
    pub fn values(&self) -> [f64; 4] {
        [
            self.lower_time.unwrap_or(DEFAULT_HOI_TIME),
            self.upper_time.unwrap_or(DEFAULT_HOI_TIME),
            self.lower_amplitude.unwrap_or(0.0),
            self.upper_amplitude.unwrap_or(0.0),
        ]
    }

    fn stored(&self) -> [(&'static str, Option<f64>); 4] {
        [("al", self.lower_amplitude), ("au", self.upper_amplitude), ("tl", self.lower_time), ("tu", self.upper_time)]
    }
}

impl HoiCurve {
    fn numbers(&self) -> Vec<f64> {
        match self {
            HoiCurve::Point { x, y } | HoiCurve::Rotation { center: Some(HoiCenter::At { x, y }), .. } => vec![*x, *y],
            HoiCurve::Rotation { .. } => Vec::new(),
            HoiCurve::Timing(t) => t.stored().iter().filter_map(|(_, v)| *v).collect(),
        }
    }
}

impl Hoi {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The curve for an axis tag, as written.
    pub fn get(&self, axis: &str) -> Option<&HoiCurve> {
        self.0.iter().find(|e| e.axis == axis).map(|e| &e.curve)
    }

    /// What is wrong with the data, if anything: too many entries, an empty, overlong or repeated
    /// tag, or a number that is not finite or lies beyond the model's coordinate range.
    pub fn check(&self) -> Result<(), String> {
        if self.0.len() > MAX_AXIS_VALUES {
            return Err(format!("a node has interpolation curves for more than {MAX_AXIS_VALUES} axes"));
        }
        for (i, e) in self.0.iter().enumerate() {
            if e.axis.is_empty() || e.axis.len() > MAX_NAME_LEN {
                return Err("a node's interpolation curve names no axis, or a tag that is too long".into());
            }
            if self.0.iter().skip(i + 1).any(|o| o.axis == e.axis) {
                return Err(format!("a node has two interpolation curves for axis `{}`", e.axis));
            }
            if !e.curve.numbers().into_iter().all(in_range) {
                return Err(format!("a node's interpolation curve for axis `{}` holds a value that is not a number or is out of range", e.axis));
            }
        }
        Ok(())
    }

    /// The data from a `hoi` dictionary, or `None` when it is not of the documented shape or
    /// fails [`Hoi::check`].
    pub fn from_value(v: &Value) -> Option<Hoi> {
        let d = v.as_dict()?;
        if d.entries.len() > MAX_AXIS_VALUES {
            return None;
        }
        let entries = d.entries.iter().map(|(k, v)| Some(HoiAxis { axis: k.as_str().to_string(), curve: curve(v.as_dict()?)? }));
        let hoi = Hoi(entries.collect::<Option<Vec<_>>>()?);
        hoi.check().ok().map(|()| hoi)
    }

    /// The `hoi` dictionary, laid out over several lines as other node attributes are; `None` when
    /// a number is not finite.
    pub fn to_value(&self) -> Option<Value> {
        let entries = self.0.iter().map(|e| Some((Str::new(&e.axis), curve_value(&e.curve)?)));
        Some(dict(entries.collect::<Option<Vec<_>>>()?))
    }
}

fn dict(entries: Vec<(Str, Value)>) -> Value {
    let layout = if entries.is_empty() { Layout::Compact } else { Layout::Multiline };
    Value::Dict(Dict { entries, layout, ..Default::default() })
}

fn number(v: &Value) -> Option<f64> {
    v.as_num().map(Num::as_f64).filter(|f| in_range(*f))
}

fn pair(v: &Value) -> Option<(f64, f64)> {
    match v.as_array()?.items.as_slice() {
        [x, y] => Some((number(x)?, number(y)?)),
        _ => None,
    }
}

fn pair_value(x: f64, y: f64) -> Option<Value> {
    let items = vec![Value::Number(Number::float(x)?), Value::Number(Number::float(y)?)];
    Some(Value::Array(Array { items, layout: Layout::Compact, ..Default::default() }))
}

/// One axis's curve. The keys of a dictionary say which kind it is; a dictionary that mixes the
/// kinds' keys, repeats a key or has others is not read.
fn curve(d: &Dict) -> Option<HoiCurve> {
    let keys: Vec<&str> = d.entries.iter().map(|(k, _)| k.as_str()).collect();
    if keys.iter().enumerate().any(|(i, k)| keys.iter().skip(i + 1).any(|o| o == k)) {
        return None;
    }
    let only = |allowed: &[&str]| keys.iter().all(|k| allowed.contains(k));
    if keys.contains(&"ip") {
        let (x, y) = pair(d.get("ip")?)?;
        return only(&["ip"]).then_some(HoiCurve::Point { x, y });
    }
    if keys.contains(&"rc") || keys.contains(&"rd") {
        if !only(&["rc", "rd"]) {
            return None;
        }
        let center = match d.get("rc") {
            None => None,
            Some(v) if v.as_str() == Some("auto") => Some(HoiCenter::Auto),
            Some(v) => {
                let (x, y) = pair(v)?;
                Some(HoiCenter::At { x, y })
            }
        };
        let direction = match d.get("rd").map(Value::as_str) {
            None => None,
            Some(Some("cw")) => Some(HoiDirection::Clockwise),
            Some(Some("ccw")) => Some(HoiDirection::CounterClockwise),
            Some(_) => return None,
        };
        return Some(HoiCurve::Rotation { center, direction });
    }
    if !only(&["al", "au", "tl", "tu"]) {
        return None;
    }
    let value = |k: &str| match d.get(k) {
        None => Some(None),
        Some(v) => number(v).map(Some),
    };
    Some(HoiCurve::Timing(HoiTiming {
        lower_time: value("tl")?,
        upper_time: value("tu")?,
        lower_amplitude: value("al")?,
        upper_amplitude: value("au")?,
    }))
}

fn curve_value(c: &HoiCurve) -> Option<Value> {
    let text = |s: &str| Value::String(Str::new(s));
    let mut e: Vec<(Str, Value)> = Vec::new();
    match c {
        HoiCurve::Point { x, y } => e.push(("ip".into(), pair_value(*x, *y)?)),
        HoiCurve::Rotation { center, direction } => {
            match center {
                None => {}
                Some(HoiCenter::Auto) => e.push(("rc".into(), text("auto"))),
                Some(HoiCenter::At { x, y }) => e.push(("rc".into(), pair_value(*x, *y)?)),
            }
            match direction {
                None => {}
                Some(HoiDirection::Clockwise) => e.push(("rd".into(), text("cw"))),
                Some(HoiDirection::CounterClockwise) => e.push(("rd".into(), text("ccw"))),
            }
        }
        HoiCurve::Timing(t) => {
            for (k, v) in t.stored() {
                if let Some(v) = v {
                    e.push((k.into(), Value::Number(Number::float(v)?)));
                }
            }
        }
    }
    Some(dict(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, NodeKind, Path, Rare};

    fn parse(text: &str) -> Value {
        fontcraft_plist::openstep::parse_value(text).unwrap()
    }

    #[test]
    fn the_three_kinds_read_and_others_do_not() {
        let v = parse("{\nwght = {\nip = (150,210.5);\n};\nwdth = {\nrc = auto;\nrd = ccw;\n};\nopsz = {\nal = 10;\ntl = 0.25;\n};\n}");
        let hoi = Hoi::from_value(&v).unwrap();
        assert_eq!(hoi.get("wght"), Some(&HoiCurve::Point { x: 150.0, y: 210.5 }));
        assert_eq!(hoi.get("wdth"), Some(&HoiCurve::Rotation { center: Some(HoiCenter::Auto), direction: Some(HoiDirection::CounterClockwise) }));
        let Some(HoiCurve::Timing(t)) = hoi.get("opsz") else { panic!() };
        assert_eq!(t.values(), [0.25, DEFAULT_HOI_TIME, 10.0, 0.0]);
        assert_eq!(Hoi::from_value(&hoi.to_value().unwrap()), Some(hoi));
        for bad in [
            "{wght = {ip = (1,2); tl = 3;};}",
            "{wght = {rd = left;};}",
            "{wght = {ip = (1,2,3);};}",
            "{wght = {ip = (1e300,2);};}",
            "{wght = {xx = 1;};}",
            "{wght = {}; wght = {};}",
        ] {
            assert_eq!(Hoi::from_value(&parse(bad)), None, "{bad}");
        }
        let many = (0..=MAX_AXIS_VALUES).map(|i| format!("a{i} = {{}};")).collect::<String>();
        assert_eq!(Hoi::from_value(&parse(&format!("{{{many}}}"))), None, "more entries than the cap");
    }

    #[test]
    fn reversing_a_path_keeps_the_data_on_its_nodes() {
        let mut nodes = vec![
            Node::new(0.0, 0.0, NodeKind::Line),
            Node::new(100.0, 0.0, NodeKind::Line),
            Node::new(100.0, 50.0, NodeKind::OffCurve),
            Node::new(50.0, 100.0, NodeKind::OffCurve),
            Node::new(0.0, 100.0, NodeKind::Curve),
        ];
        let tag = |n: &mut Node, x: f64| {
            let mut info = (*n.info).clone();
            info.hoi = Hoi(vec![HoiAxis { axis: "wght".into(), curve: HoiCurve::Point { x, y: 7.0 } }]);
            n.info = Rare::new(info);
        };
        for (n, x) in nodes.iter_mut().zip([1.0, 2.0, 3.0, 4.0, 5.0]) {
            tag(n, x);
        }
        let mut path = Path::new(true, nodes);
        let before: Vec<_> = path.nodes.iter().map(|n| (n.x, n.y, n.info.hoi.clone())).collect();
        path.reverse();
        for (x, y, hoi) in &before {
            let n = path.nodes.iter().find(|n| n.x == *x && n.y == *y).unwrap();
            assert_eq!(&n.info.hoi, hoi, "the node at ({x}, {y})");
        }
        path.reverse();
        let again: Vec<_> = path.nodes.iter().map(|n| (n.x, n.y, n.info.hoi.clone())).collect();
        assert_eq!(again, before);
    }

    #[test]
    fn the_native_form_round_trips() {
        let hoi = Hoi(vec![
            HoiAxis { axis: "wght".into(), curve: HoiCurve::Rotation { center: Some(HoiCenter::At { x: 1.5, y: -2.0 }), direction: None } },
            HoiAxis { axis: "wdth".into(), curve: HoiCurve::Timing(HoiTiming { upper_amplitude: Some(-12.5), ..Default::default() }) },
        ]);
        let json = serde_json::to_string(&hoi).unwrap();
        assert_eq!(serde_json::from_str::<Hoi>(&json).unwrap(), hoi);
    }
}
