//! FontCraft's lib key for what a path holds beyond its geometry: its lock and its drawing
//! attributes (stroke, mask, fill, colours). A GLIF contour has no attribute for them, and
//! glyphsLib keeps none of them in a UFO.
//!
//! | Model | GLIF lib key |
//! |---|---|
//! | each path's `info.locked` and `info.attrs` | `com.fontcraft.pathAttributes`: an array with one dictionary per `<contour>`, in file order; a contour without any is an empty dictionary |
//!
//! The dictionary uses the `.glyphs` attribute names: `strokeWidth`, `strokeHeight` and
//! `strokePos` (a number, or text for an expression; the position in the model's −1 to 1
//! convention), `lineCapStart`, `lineCapEnd`, `lineJoin` and `mask` (the stored codes),
//! `lineCapAxis` (the forced axis of an axis-aligned cap), `fill`, `reversePaths`, `hidden` and
//! `locked` (written when true), `fillColor` and `strokeColor` (red, green, blue and alpha from 0
//! to 1, or `{palette, alpha}`), `color` (a label number, or red, green, blue and alpha), and
//! `opacity`. Gradients, shadows, other stored attributes and numbers that are not finite have no
//! entry; [`lost`] says which paths have them, for the shape loss report.
//!
//! The key is written only when a path has something to keep, and read only when it has one
//! dictionary per contour, each with known keys of the right type; otherwise it stays user data.

use fontcraft_model::{Color, ColorLabel, Dimension, Family, Path, Shape, ShapeAttrs};
use fontcraft_plist::{Dict, Num, Str, Value};

use crate::glyphkeys::{array, dict, int, number, string};
use crate::shapekeys::is_contour;

pub(crate) const PATH_ATTRIBUTES: &str = "com.fontcraft.pathAttributes";
/// The native extra key of a forced cap axis (the engine's `CAP_AXIS_KEY`).
const CAP_AXIS: &str = "lineCapAxis";

/// The dictionary of the contour `s` writes, if it writes one.
fn contour_entry(s: &Shape) -> Option<Value> {
    match s {
        Shape::Path(p) => Some(path_dict(p)),
        _ => is_contour(s).then(|| Value::Dict(Dict::new())),
    }
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn write_dimension(d: &Dimension) -> Option<Value> {
    match d {
        Dimension::Number(f) if f.is_finite() => Some(number(*f)),
        Dimension::Number(_) => None,
        Dimension::Expression(s) => Some(string(s)),
    }
}

fn write_color(c: &Color) -> Option<Value> {
    match c {
        Color::Rgba(v) if finite(v) => Some(array(v.iter().map(|x| number(*x)).collect())),
        Color::Palette { index, alpha } if alpha.is_finite() => Some(dict(vec![("alpha", number(*alpha)), ("palette", int(i64::from(*index)))])),
        _ => None,
    }
}

fn write_label(c: &ColorLabel) -> Option<Value> {
    match c {
        ColorLabel::Index(i) => Some(int(i64::from(*i))),
        ColorLabel::Rgba(v) if finite(v) => Some(array(v.iter().map(|x| number(*x)).collect())),
        ColorLabel::Rgba(_) => None,
    }
}

/// The dictionary of one path; empty when it has nothing to keep.
fn path_dict(p: &Path) -> Value {
    let a = &p.info.attrs;
    let mut e: Vec<(&str, Value)> = Vec::new();
    for (k, d) in [("strokeWidth", &a.stroke_width), ("strokeHeight", &a.stroke_height), ("strokePos", &a.stroke_pos)] {
        if let Some(v) = d.as_ref().and_then(write_dimension) {
            e.push((k, v));
        }
    }
    for (k, b) in [("lineCapStart", a.line_cap_start), ("lineCapEnd", a.line_cap_end), ("lineJoin", a.line_join), ("mask", a.mask)] {
        if let Some(b) = b {
            e.push((k, int(i64::from(b))));
        }
    }
    if let Some(axis) = a.extra.get(Family::Native, CAP_AXIS).and_then(Value::as_str) {
        e.push((CAP_AXIS, string(axis)));
    }
    for (k, on) in [("fill", a.fill), ("reversePaths", a.reverse_paths), ("hidden", a.hidden), ("locked", p.info.locked)] {
        if on {
            e.push((k, Value::Bool(true)));
        }
    }
    for (k, c) in [("fillColor", &a.fill_color), ("strokeColor", &a.stroke_color)] {
        if let Some(v) = c.as_ref().and_then(write_color) {
            e.push((k, v));
        }
    }
    if let Some(v) = a.color.as_ref().and_then(write_label) {
        e.push(("color", v));
    }
    if let Some(o) = a.opacity.filter(|o| o.is_finite()) {
        e.push(("opacity", number(o)));
    }
    dict(e)
}

/// Whether a path has attributes the key cannot keep.
pub(crate) fn lost(p: &Path) -> bool {
    let a = &p.info.attrs;
    let dimension = |d: &Option<Dimension>| d.as_ref().is_some_and(|d| write_dimension(d).is_none());
    let color = |c: &Option<Color>| c.as_ref().is_some_and(|c| write_color(c).is_none());
    a.gradient.is_some()
        || a.shadow.is_some()
        || a.shadow_in.is_some()
        || dimension(&a.stroke_width)
        || dimension(&a.stroke_height)
        || dimension(&a.stroke_pos)
        || color(&a.fill_color)
        || color(&a.stroke_color)
        || a.color.as_ref().is_some_and(|c| write_label(c).is_none())
        || a.opacity.is_some_and(|o| !o.is_finite())
        || a.extra.iter().any(|x| !(x.family == Family::Native && x.key.as_str() == CAP_AXIS && x.value.as_str().is_some()))
}

/// The key's value for `shapes`; `None` when no path has anything to keep.
pub(crate) fn entry(shapes: &[Shape]) -> Option<Value> {
    let items: Vec<Value> = shapes.iter().filter_map(contour_entry).collect();
    let any = items.iter().any(|v| v.as_dict().is_some_and(|d| !d.is_empty()));
    any.then(|| array(items))
}

// ── reading ────────────────────────────────────────────────────────────────────────────────────

fn read_number(v: &Value) -> Option<f64> {
    v.as_num().map(Num::as_f64).filter(|f| f.is_finite())
}

fn read_dimension(v: &Value) -> Option<Dimension> {
    match v {
        Value::String(s) => Some(Dimension::Expression(s.as_str().to_string())),
        _ => read_number(v).map(Dimension::Number),
    }
}

fn read_byte(v: &Value) -> Option<u8> {
    match v.as_num()? {
        Num::Int(i) => u8::try_from(i).ok(),
        Num::Float(_) => None,
    }
}

fn read_four(v: &Value) -> Option<[f64; 4]> {
    let items = &v.as_array()?.items;
    match items.as_slice() {
        [r, g, b, a] => Some([read_number(r)?, read_number(g)?, read_number(b)?, read_number(a)?]),
        _ => None,
    }
}

fn read_color(v: &Value) -> Option<Color> {
    match v {
        Value::Array(_) => read_four(v).map(Color::Rgba),
        Value::Dict(d) => {
            if d.len() != 2 {
                return None;
            }
            let index = match d.get("palette")?.as_num()? {
                Num::Int(i) => u16::try_from(i).ok()?,
                Num::Float(_) => return None,
            };
            Some(Color::Palette { index, alpha: read_number(d.get("alpha")?)? })
        }
        _ => None,
    }
}

fn read_label(v: &Value) -> Option<ColorLabel> {
    match v {
        Value::Array(_) => read_four(v).map(ColorLabel::Rgba),
        _ => read_byte(v).map(ColorLabel::Index),
    }
}

/// One contour's dictionary: the path's lock and attributes. `None` when a key is unknown or
/// has a value of the wrong type.
fn read_dict(d: &Dict) -> Option<(bool, ShapeAttrs)> {
    let mut a = ShapeAttrs::default();
    let mut locked = false;
    for (k, v) in &d.entries {
        match k.as_str() {
            "strokeWidth" => a.stroke_width = Some(read_dimension(v)?),
            "strokeHeight" => a.stroke_height = Some(read_dimension(v)?),
            "strokePos" => a.stroke_pos = Some(read_dimension(v)?),
            "lineCapStart" => a.line_cap_start = Some(read_byte(v)?),
            "lineCapEnd" => a.line_cap_end = Some(read_byte(v)?),
            "lineJoin" => a.line_join = Some(read_byte(v)?),
            "mask" => a.mask = Some(read_byte(v)?),
            CAP_AXIS => a.extra.insert(Family::Native, CAP_AXIS, Value::String(Str::new(v.as_str()?))),
            "fill" => a.fill = v.as_bool()?,
            "reversePaths" => a.reverse_paths = v.as_bool()?,
            "hidden" => a.hidden = v.as_bool()?,
            "locked" => locked = v.as_bool()?,
            "fillColor" => a.fill_color = Some(read_color(v)?),
            "strokeColor" => a.stroke_color = Some(read_color(v)?),
            "color" => a.color = Some(read_label(v)?),
            "opacity" => a.opacity = Some(read_number(v)?),
            _ => return None,
        }
    }
    Some((locked, a))
}

/// Gives the paths of `shapes` their lock and attributes from the key's value `v`, all or none;
/// `false` when the value does not fit the shapes (see the module doc) or a path has some already.
pub(crate) fn apply(shapes: &mut [Shape], v: &Value) -> bool {
    let Some(items) = v.as_array() else { return false };
    if items.items.len() != shapes.iter().filter(|s| is_contour(s)).count() {
        return false;
    }
    let mut read: Vec<Option<(bool, ShapeAttrs)>> = Vec::with_capacity(items.items.len());
    for (s, e) in shapes.iter().filter(|s| is_contour(s)).zip(&items.items) {
        let Some(d) = e.as_dict() else { return false };
        match s {
            Shape::Path(p) => {
                if p.info.locked || !p.info.attrs.is_empty() {
                    return false;
                }
                let Some(x) = read_dict(d) else { return false };
                read.push(Some(x));
            }
            _ if d.is_empty() => read.push(None),
            _ => return false,
        }
    }
    for (s, r) in shapes.iter_mut().filter(|s| is_contour(s)).zip(read) {
        if let (Shape::Path(p), Some((locked, attrs))) = (s, r) {
            p.info.locked = locked;
            p.info.attrs = attrs;
            p.info.tidy();
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ufo;
    use fontcraft_model::{Background, Font, Rare};

    fn stroked_font() -> Font {
        let mut font = Font::new("Strokes");
        {
            let mut a = font.glyph_mut("A").unwrap();
            let l = a.layers.get_mut(0).unwrap();
            let mut line = Path::new(false, Path::rect(0.0, 0.0, 100.0, 10.0).nodes.into_iter().take(2).collect());
            line.info.attrs = ShapeAttrs {
                stroke_width: Some(Dimension::Number(30.0)),
                stroke_height: Some(Dimension::Expression("{stem}".into())),
                stroke_pos: Some(Dimension::Number(-0.5)),
                line_cap_start: Some(1),
                line_cap_end: Some(4),
                line_join: Some(2),
                stroke_color: Some(Color::Rgba([0.25, 0.5, 1.0, 1.0])),
                ..Default::default()
            };
            line.info.attrs.extra.insert(Family::Native, CAP_AXIS, Value::String(Str::new("x")));
            let plain = Path::rect(200.0, 0.0, 300.0, 100.0);
            let mut mask = Path::rect(0.0, 0.0, 50.0, 50.0);
            mask.info.attrs = ShapeAttrs { mask: Some(1), fill: true, color: Some(ColorLabel::Index(3)), opacity: Some(0.5), ..Default::default() };
            mask.info.attrs.fill_color = Some(Color::Palette { index: 2, alpha: 0.75 });
            mask.info.locked = true;
            l.shapes = vec![Shape::Path(line), Shape::Path(plain), Shape::Path(mask)];
            let mut bg = Path::rect(0.0, 0.0, 10.0, 10.0);
            bg.info = Rare::new(fontcraft_model::PathInfo {
                attrs: ShapeAttrs { stroke_width: Some(Dimension::Number(5.0)), ..Default::default() },
                ..Default::default()
            });
            l.info.background = Some(Background { shapes: vec![Shape::Path(bg)], ..Default::default() });
        }
        font
    }

    #[test]
    fn strokes_and_masks_survive_a_ufo() {
        let font = stroked_font();
        let files = ufo::write(&font).unwrap();
        let glif = String::from_utf8(files["glyphs/A_.glif"].clone()).unwrap();
        assert!(glif.contains("<key>com.fontcraft.pathAttributes</key>"), "{glif}");
        assert!(glif.contains("<key>strokeWidth</key>") && glif.contains("<dict/>"), "the plain path has an empty entry: {glif}");
        let bg = String::from_utf8(files["glyphs.public.background/A_.glif"].clone()).unwrap();
        assert!(bg.contains("<key>com.fontcraft.pathAttributes</key>"), "{bg}");

        let back = ufo::read(&files).unwrap();
        let (l, was) = (&back.glyph("A").unwrap().layers[0], &font.glyph("A").unwrap().layers[0]);
        assert_eq!(l.shapes, was.shapes, "strokes, the mask, colours and the lock come back");
        assert_eq!(l.info.background.as_ref().unwrap().shapes, was.info.background.as_ref().unwrap().shapes);
        assert!(l.info.user_data.is_empty(), "{:?}", l.info.user_data);
        assert_eq!(ufo::write(&back).unwrap(), files, "it comes back unchanged");
        assert!(!ufo::shape_losses(&font, false).iter().any(|x| x.starts_with(ufo::SHAPE_LOSSES[0])), "nothing is reported lost");
    }

    #[test]
    fn a_value_that_does_not_fit_stays_user_data_and_unkept_attributes_are_reported() {
        let mut font = stroked_font();
        {
            let mut a = font.glyph_mut("A").unwrap();
            let l = a.layers.get_mut(0).unwrap();
            l.info.background = None;
            let mut d = Dict::new();
            d.insert(PATH_ATTRIBUTES, array(vec![dict(vec![("strokeWidth", int(1))])]));
            l.info.user_data = fontcraft_model::UserData(d);
            for s in &mut l.shapes {
                if let Shape::Path(p) = s {
                    p.info = Rare::default();
                }
            }
        }
        let files = ufo::write(&font).unwrap();
        let back = ufo::read(&files).unwrap();
        let l = &back.glyph("A").unwrap().layers[0];
        assert!(l.info.user_data.get(PATH_ATTRIBUTES).is_some(), "one entry for three contours does not read");
        assert!(l.shapes.iter().all(|s| matches!(s, Shape::Path(p) if p.info.attrs.is_empty())));

        let mut p = Path::rect(0.0, 0.0, 1.0, 1.0);
        p.info.attrs.extra.push(Family::Glyphs, "compositing", int(1));
        assert!(lost(&p), "a stored attribute has no entry");
        p.info.attrs.extra = fontcraft_model::Extra::new();
        p.info.attrs.stroke_width = Some(Dimension::Number(f64::NAN));
        assert!(lost(&p) && entry(&[Shape::Path(p)]).is_none(), "a number that is not finite is not written");
    }
}
