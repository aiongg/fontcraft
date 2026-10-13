//! The paint keys of a shape's `attr` dictionary as typed values.
//!
//! | Key | Model | Version 3 | Version 4 |
//! |---|---|---|---|
//! | `fillColor`, `strokeColor`, a stop's or shadow's colour | [`Color`] | RGBA or gray with alpha in bytes; `("p", index, alpha)` with an 8-bit alpha | the same in fractions from 0 to 1; `("p", index, alpha)` |
//! | `gradient` | [`Gradient`] | `colors`, `start`, `end`; `type = circle` for radial | adds `type = radial` with `startRadius`, `endRadius`, `angle`; `type = conic` with `center`, `startAngle`, `angle`, `controlRadius`; `extend` |
//! | `shadow`, `shadowIn` | [`Shadow`] | `blur`, `offsetX`, `offsetY` as number text, `color` | the same |
//! | `strokePos` | `stroke_pos` (version 4 convention) | 0 outside, 0.5 centred, 1 inside | −1 inside, 0 centred, 1 outside |
//!
//! A version 3 radial gradient has no radii in the file; the model reads it with both radii 0,
//! and a version 4 radial gradient with an end radius of 0 is written without `endRadius`.
//! Gray colours become RGBA colours with equal components; [`GRAY_KEY`] lists where the shape
//! wrote the gray form, so those colours are written that way again. CMYK colours, and any value
//! that would not be written back the same, stay stored values.
//!
//! Written from the GlyphsSDK format documents (Apache-2.0).

use fontcraft_geom::Point;
use fontcraft_model::{Color, ColorLabel, ColorStop, Dimension, Extend, Gradient, GradientKind, MAX_GRADIENT_STOPS, Shadow};
use fontcraft_plist::{Array, Dict, Layout, Num, Str, Value};

use crate::layer::number;
use crate::{int, list, num, string};

/// A native-family key on a shape's attributes: the colours (`fillColor`, `strokeColor`,
/// `shadow`, `shadowIn`, `gradient.<stop>`) that the file wrote in the gray form
/// `(gray, alpha)`, as a list of those names.
pub(crate) const GRAY_KEY: &str = "glyphsGrayColors";

/// How colours are written: the format version, and the colours written in the gray form when
/// their red, green and blue are equal.
#[derive(Clone, Copy)]
pub(crate) struct Form<'a> {
    pub version: u32,
    pub gray: &'a [String],
}

impl Form<'_> {
    fn gray(&self, at: &str) -> bool {
        self.gray.iter().any(|g| g == at)
    }
}

fn compact(items: Vec<Value>) -> Value {
    Value::Array(Array { items, layout: Layout::Compact, ..Default::default() })
}

fn byte(v: &Value) -> Option<u8> {
    match v.as_num()? {
        Num::Int(i) => u8::try_from(i).ok(),
        Num::Float(_) => None,
    }
}

/// A colour component: a byte in version 3, a fraction in version 4.
fn component(v: &Value, version: u32) -> Option<f64> {
    if version >= 4 { v.as_num().map(Num::as_f64).filter(|f| (0.0..=1.0).contains(f)) } else { byte(v).map(|b| f64::from(b) / 255.0) }
}

fn write_component(c: f64, version: u32) -> Option<Value> {
    if version >= 4 { num(c, "color").ok() } else { Some(int(crate::glyph::to_byte(c))) }
}

fn is_gray_form(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.items.len() == 2 && a.items.iter().all(|i| i.as_num().is_some()))
}

/// The colours in the dictionary written in the gray form, by the names [`GRAY_KEY`] uses.
pub(crate) fn gray_colors(d: &Dict) -> Vec<String> {
    let mut out: Vec<String> = ["fillColor", "strokeColor"].into_iter().filter(|k| d.get(k).is_some_and(is_gray_form)).map(String::from).collect();
    for k in ["shadow", "shadowIn"] {
        if d.get(k).and_then(Value::as_dict).and_then(|s| s.get("color")).is_some_and(is_gray_form) {
            out.push(k.into());
        }
    }
    let stops = d.get("gradient").and_then(Value::as_dict).and_then(|g| g.get("colors")).and_then(Value::as_array);
    for (i, s) in stops.into_iter().flat_map(|a| a.items.iter().take(MAX_GRADIENT_STOPS)).enumerate() {
        if s.as_array().and_then(|s| s.items.first()).is_some_and(is_gray_form) {
            out.push(format!("gradient.{i}"));
        }
    }
    out
}

/// The stored [`GRAY_KEY`] list.
pub(crate) fn stored_gray(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.items.iter().filter_map(|i| i.as_str().map(String::from)).collect()).unwrap_or_default()
}

pub(crate) fn gray_value(names: &[String]) -> Value {
    list(names.iter().map(|n| string(n)).collect())
}

pub(crate) fn read_color(v: &Value, version: u32) -> Option<Color> {
    let items = &v.as_array()?.items;
    match items.as_slice() {
        [p, index, alpha] if p.as_str() == Some("p") => {
            let index = match index.as_num()? {
                Num::Int(i) => u16::try_from(i).ok()?,
                Num::Float(_) => return None,
            };
            Some(Color::Palette { index, alpha: component(alpha, version)? })
        }
        [g, a] => {
            let g = component(g, version)?;
            Some(Color::Rgba([g, g, g, component(a, version)?]))
        }
        [_, _, _, _] => match crate::glyph::read_color(v, version)? {
            ColorLabel::Rgba(c) => Some(Color::Rgba(c)),
            ColorLabel::Index(_) => None,
        },
        _ => None,
    }
}

/// A colour as the file writes it; `at` names it as [`GRAY_KEY`] does.
pub(crate) fn write_color(c: &Color, f: Form, at: &str) -> Option<Value> {
    match c {
        Color::Palette { index, alpha } => Some(compact(vec![string("p"), int(*index), write_component(*alpha, f.version)?])),
        Color::Rgba([r, g, b, a]) if r == g && g == b && f.gray(at) => {
            Some(compact(vec![write_component(*r, f.version)?, write_component(*a, f.version)?]))
        }
        Color::Rgba(rgba) => Some(crate::glyph::write_color(&ColorLabel::Rgba(*rgba), f.version)),
    }
}

/// Whether version 3 loses part of the colour: a component or alpha that is not a whole byte.
pub(crate) fn color_needs_v4(c: &Color) -> bool {
    match c {
        Color::Rgba(rgba) => crate::glyph::color_needs_floats(&ColorLabel::Rgba(*rgba)),
        Color::Palette { alpha, .. } => crate::glyph::color_needs_floats(&ColorLabel::Rgba([0.0, 0.0, 0.0, *alpha])),
    }
}

fn point(v: &Value) -> Option<Point> {
    match v.as_array()?.items.as_slice() {
        [x, y] => Some(Point::new(number(x)?, number(y)?)),
        _ => None,
    }
}

fn write_point(p: Point) -> Option<Value> {
    Some(compact(vec![num(p.x, "gradient").ok()?, num(p.y, "gradient").ok()?]))
}

fn read_stop(v: &Value, version: u32) -> Option<ColorStop> {
    match v.as_array()?.items.as_slice() {
        [c, o] => Some(ColorStop { color: read_color(c, version)?, offset: number(o)? }),
        _ => None,
    }
}

pub(crate) fn read_gradient(v: &Value, version: u32) -> Option<Gradient> {
    let d = v.as_dict()?;
    let mut known = vec!["colors", "type"];
    let stops = &d.get("colors")?.as_array()?.items;
    if stops.len() > MAX_GRADIENT_STOPS {
        return None;
    }
    let stops = stops.iter().map(|s| read_stop(s, version)).collect::<Option<Vec<_>>>()?;
    let get = |k: &str, default: f64| d.get(k).map_or(Some(default), number);
    let kind = match (d.get("type").map(Value::as_str), version >= 4) {
        (None, _) => {
            known.extend(["start", "end"]);
            GradientKind::Linear { start: point(d.get("start")?)?, end: point(d.get("end")?)? }
        }
        (Some(Some("circle")), false) => {
            known.extend(["start", "end"]);
            GradientKind::Radial { start: point(d.get("start")?)?, end: point(d.get("end")?)?, start_radius: 0.0, end_radius: 0.0, angle: 0.0 }
        }
        (Some(Some("radial")), true) => {
            known.extend(["start", "end", "startRadius", "endRadius", "angle"]);
            GradientKind::Radial {
                start: point(d.get("start")?)?,
                end: point(d.get("end")?)?,
                start_radius: get("startRadius", 0.0)?,
                end_radius: get("endRadius", 0.0)?,
                angle: get("angle", 0.0)?,
            }
        }
        (Some(Some("conic")), true) => {
            known.extend(["center", "startAngle", "angle", "controlRadius"]);
            GradientKind::Conic {
                center: point(d.get("center")?)?,
                start_angle: get("startAngle", 0.0)?,
                angle: get("angle", 360.0)?,
                control_radius: get("controlRadius", 0.0)?,
            }
        }
        _ => return None,
    };
    let extend = match (d.get("extend").map(Value::as_str), version >= 4) {
        (None, _) => Extend::Pad,
        (Some(Some("repeat")), true) => Extend::Repeat,
        (Some(Some("reflect")), true) => Extend::Reflect,
        _ => return None,
    };
    known.push("extend");
    if d.entries.iter().any(|(k, _)| !known.contains(&k.as_str())) {
        return None;
    }
    Some(Gradient { kind, stops, extend })
}

/// The `gradient` dictionary; `None` when the version cannot write the kind (conic before 4).
pub(crate) fn write_gradient(g: &Gradient, f: Form) -> Option<Value> {
    let mut e: Vec<(Str, Value)> = Vec::new();
    let opt = |e: &mut Vec<(Str, Value)>, k: &str, v: f64, default: f64| -> Option<()> {
        if v != default {
            e.push((k.into(), num(v, "gradient").ok()?));
        }
        Some(())
    };
    match g.kind {
        GradientKind::Linear { start, end } => {
            e.push(("start".into(), write_point(start)?));
            e.push(("end".into(), write_point(end)?));
        }
        GradientKind::Radial { start, end, start_radius, end_radius, angle } => {
            e.push(("start".into(), write_point(start)?));
            e.push(("end".into(), write_point(end)?));
            if f.version >= 4 {
                opt(&mut e, "startRadius", start_radius, 0.0)?;
                opt(&mut e, "endRadius", end_radius, 0.0)?;
                opt(&mut e, "angle", angle, 0.0)?;
                e.push(("type".into(), string("radial")));
            } else {
                e.push(("type".into(), string("circle")));
            }
        }
        GradientKind::Conic { center, start_angle, angle, control_radius } => {
            if f.version < 4 {
                return None;
            }
            e.push(("center".into(), write_point(center)?));
            opt(&mut e, "startAngle", start_angle, 0.0)?;
            opt(&mut e, "angle", angle, 360.0)?;
            opt(&mut e, "controlRadius", control_radius, 0.0)?;
            e.push(("type".into(), string("conic")));
        }
    }
    if f.version >= 4 {
        match g.extend {
            Extend::Pad => {}
            Extend::Repeat => e.push(("extend".into(), string("repeat"))),
            Extend::Reflect => e.push(("extend".into(), string("reflect"))),
        }
    }
    let stops = g
        .stops
        .iter()
        .take(MAX_GRADIENT_STOPS)
        .enumerate()
        .map(|(i, s)| Some(list(vec![write_color(&s.color, f, &format!("gradient.{i}"))?, num(s.offset, "gradient").ok()?])))
        .collect::<Option<Vec<_>>>()?;
    e.push(("colors".into(), list(stops)));
    e.sort_by(|a, b| a.0.as_str().as_bytes().cmp(b.0.as_str().as_bytes()));
    Some(Value::Dict(Dict { entries: e, layout: Layout::Multiline, ..Default::default() }))
}

/// What version 3 cannot hold of a gradient: a conic kind, an extend mode, radii or an angle.
pub(crate) fn gradient_needs_v4(g: &Gradient) -> bool {
    let kind = match g.kind {
        GradientKind::Linear { .. } => false,
        GradientKind::Radial { start_radius, end_radius, angle, .. } => start_radius != 0.0 || end_radius != 0.0 || angle != 0.0,
        GradientKind::Conic { .. } => true,
    };
    kind || g.extend != Extend::Pad || g.stops.iter().any(|s| color_needs_v4(&s.color))
}

/// A number written as text (`blur = "10"`).
fn text_number(v: &Value) -> Option<f64> {
    v.as_str()?.trim().parse::<f64>().ok().filter(|f| f.is_finite() && f.abs() <= fontcraft_model::MAX_COORDINATE)
}

fn write_text_number(f: f64) -> Option<Value> {
    f.is_finite().then(|| string(&format!("{f}")))
}

pub(crate) fn read_shadow(v: &Value, version: u32) -> Option<Shadow> {
    let d = v.as_dict()?;
    if d.entries.iter().any(|(k, _)| !matches!(k.as_str(), "blur" | "color" | "offsetX" | "offsetY")) {
        return None;
    }
    Some(Shadow {
        blur: text_number(d.get("blur")?)?,
        color: read_color(d.get("color")?, version)?,
        offset: [text_number(d.get("offsetX")?)?, text_number(d.get("offsetY")?)?],
    })
}

/// A shadow; `at` is `shadow` or `shadowIn`.
pub(crate) fn write_shadow(s: &Shadow, f: Form, at: &str) -> Option<Value> {
    let entries = vec![
        ("blur".into(), write_text_number(s.blur)?),
        ("color".into(), write_color(&s.color, f, at)?),
        ("offsetX".into(), write_text_number(s.offset[0])?),
        ("offsetY".into(), write_text_number(s.offset[1])?),
    ];
    Some(Value::Dict(Dict { entries, layout: Layout::Multiline, ..Default::default() }))
}

/// The version 4 stroke position for a version 3 one (0 outside, 0.5 centred, 1 inside).
fn from_v3(p: f64) -> f64 {
    tidy(1.0 - 2.0 * p)
}

/// The version 3 stroke position for a version 4 one (−1 inside, 0 centred, 1 outside).
fn to_v3(p: f64) -> f64 {
    tidy((1.0 - p) / 2.0)
}

/// `p` to nine decimals, so a converted position written back reads as the text it came from
/// (0.3 stays 0.3, not 0.29999999999999993); `p` itself when that is not finite.
fn tidy(p: f64) -> f64 {
    let r = (p * 1e9).round() / 1e9;
    if r.is_finite() { r } else { p }
}

/// A stroke position given as an expression that is not a plain number: version 3 cannot convert
/// it, so it is written as it is.
pub(crate) fn stroke_pos_unconvertible(d: &Dimension) -> bool {
    matches!(d, Dimension::Expression(e) if !e.trim().parse::<f64>().is_ok_and(f64::is_finite))
}

/// `strokePos` in the model's convention. A version 3 expression is converted when it is a plain
/// number and stays a stored value otherwise.
pub(crate) fn read_stroke_pos(v: &Value, version: u32) -> Option<Dimension> {
    let d = match v {
        Value::String(s) => Dimension::Expression(s.as_str().to_string()),
        _ => Dimension::Number(number(v)?),
    };
    if version >= 4 {
        return Some(d);
    }
    Some(match d {
        Dimension::Number(p) => Dimension::Number(from_v3(p)),
        Dimension::Expression(e) => Dimension::Expression(format!("{}", from_v3(e.trim().parse::<f64>().ok().filter(|f| f.is_finite())?))),
    })
}

pub(crate) fn write_stroke_pos(d: &Dimension, version: u32) -> Option<Value> {
    let v3 = version < 4;
    match d {
        Dimension::Number(p) => num(if v3 { to_v3(*p) } else { *p }, "strokePos").ok(),
        Dimension::Expression(e) if v3 => match e.trim().parse::<f64>().ok().filter(|f| f.is_finite()) {
            Some(p) => Some(string(&format!("{}", to_v3(p)))),
            None => Some(string(e)),
        },
        Dimension::Expression(e) => Some(string(e)),
    }
}
