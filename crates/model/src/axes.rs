//! User coordinates by the reference's rules: where a master or an instance sits on an axis in
//! user space, an axis's user-to-design map and range when the source states neither, and the
//! origin master.
//!
//! The map is the font's `Axis Mappings` parameter (`{tag: {user: design}}`), as in the
//! reference's format. Without it, the map comes from the masters' `Axis Location` parameters, or
//! from the instances' weight and width classes.
//!
//! Ported from glyphsLib 6.15.0 (Apache-2.0): `builder/axes.py` (`AxisDefinition.get_user_loc`,
//! `to_designspace_axes`, `get_regular_master`, `find_base_style`) and the weight and width
//! tables of `classes.py` and `builder/constants.py`. [`piecewise_linear_map`] ports fontTools'
//! `varLib.models.piecewiseLinearMap` (MIT). Where these rules differ from glyphsLib, the
//! function says so.

use fontcraft_plist::{Num, Value};

use crate::{Font, Instance, Master, Params, StyleClass};

/// The font parameter that holds the user-to-design map of every axis.
pub const AXIS_MAPPINGS: &str = "Axis Mappings";
/// A location the font is interpolated at as if a master were there.
pub const VIRTUAL_MASTER: &str = "Virtual Master";
/// The master or instance parameter that gives its user coordinates by axis name.
pub const AXIS_LOCATION: &str = "Axis Location";
/// The font parameter naming the origin master by id.
pub const ORIGIN: &str = "Variable Font Origin";
/// The older name of [`ORIGIN`], which names the master by name.
const ORIGIN_BY_NAME: &str = "Variation Font Origin";

/// The reference's weight names and the user value of each.
const WEIGHT_CODES: &[(&str, f64)] = &[
    ("Thin", 100.0),
    ("ExtraLight", 200.0),
    ("UltraLight", 200.0),
    ("Light", 300.0),
    ("Normal", 400.0),
    ("Regular", 400.0),
    ("Medium", 500.0),
    ("DemiBold", 600.0),
    ("SemiBold", 600.0),
    ("Bold", 700.0),
    ("UltraBold", 800.0),
    ("ExtraBold", 800.0),
    ("Black", 900.0),
    ("Heavy", 900.0),
];

/// The reference's width names and the OS/2 width class of each.
const WIDTH_CODES: &[(&str, u8)] = &[
    ("Ultra Condensed", 1),
    ("Extra Condensed", 2),
    ("Condensed", 3),
    ("SemiCondensed", 4),
    ("Medium (normal)", 5),
    ("Semi Expanded", 6),
    ("Expanded", 7),
    ("Extra Expanded", 8),
    ("Ultra Expanded", 9),
];

/// The user value (percent of the normal width) of OS/2 width classes 1 to 9.
const WIDTH_CLASS_VALUES: [f64; 9] = [50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 150.0, 200.0];

/// A code from a table by name, also with the name's spaces removed (some sources write
/// `Semi Condensed`).
fn code<T: Copy>(table: &[(&str, T)], name: &str) -> Option<T> {
    let find = |n: &str| table.iter().find(|(k, _)| *k == n).map(|(_, v)| *v);
    find(name).or_else(|| find(&name.split_whitespace().collect::<String>()))
}

/// The user value of OS/2 width class `class` (1 to 9).
pub fn width_class_value(class: f64) -> Option<f64> {
    if class.fract() != 0.0 || !(1.0..=9.0).contains(&class) {
        return None;
    }
    // 1 to 9, checked above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = class as usize - 1;
    WIDTH_CLASS_VALUES.get(i).copied()
}

/// A number from a parameter value that may be written as text (`"600"`).
fn number_of(v: &Value) -> Option<f64> {
    let n = match v {
        Value::Number(n) => n.value().as_f64(),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// The user value an OS/2 class stands for on a weight (`wght`: the class) or width (`wdth`: the
/// percentage) axis.
fn class_value(tag: &str, class: f64) -> Option<f64> {
    match tag {
        "wght" => Some(class.trunc()),
        "wdth" => width_class_value(class.trunc()),
        _ => None,
    }
}

/// The user value a weight or width name or number of a master or instance stands for.
fn style_value(tag: &str, class: &StyleClass) -> Option<f64> {
    match (tag, class) {
        ("wght", StyleClass::Name(n)) => code(WEIGHT_CODES, n),
        ("wdth", StyleClass::Name(n)) => code(WIDTH_CODES, n).and_then(|c| width_class_value(f64::from(c))),
        // glyphsLib takes a whole number as the value itself.
        (_, StyleClass::Number(n)) if n.fract() == 0.0 && n.is_finite() => class_value(tag, *n),
        _ => None,
    }
}

/// The design coordinate of a location on axis `i`; a location shorter than the axes has the
/// reference's defaults (100 on the first two axes, 0 on the others).
pub fn design_value(location: &[f64], i: usize) -> f64 {
    location.get(i).copied().filter(|v| v.is_finite()).unwrap_or(if i < 2 { 100.0 } else { 0.0 })
}

/// The `Location` an `Axis Location` parameter gives for the axis called `name`.
fn axis_location(params: &Params, name: &str) -> Option<f64> {
    let list = params.get(AXIS_LOCATION)?.as_array()?;
    let entry = list.items.iter().filter_map(Value::as_dict).find(|d| d.get("Axis").and_then(Value::as_str) == Some(name))?;
    entry.get("Location").and_then(number_of)
}

/// Where a master or an instance sits on axis `i` in user coordinates (glyphsLib's
/// `get_user_loc`): on a weight axis 400, on others the design coordinate; then the weight or
/// width name (`class`, absent meaning `Regular` or `Medium (normal)`), then the `weightClass`
/// or `widthClass` parameter, then the `Axis Location` parameter, each overriding the last.
fn user_value(font: &Font, i: usize, location: &[f64], class: Option<&StyleClass>, default_class: &str, params: &Params) -> f64 {
    let Some(axis) = font.axes.get(i) else { return design_value(location, i) };
    let tag = axis.tag.as_str();
    let mut user = if tag == "wght" { 400.0 } else { design_value(location, i) };
    if tag == "wght" || tag == "wdth" {
        let named = StyleClass::Name(default_class.to_string());
        if let Some(v) = style_value(tag, class.unwrap_or(&named)) {
            user = v;
        }
        let param = if tag == "wght" { "weightClass" } else { "widthClass" };
        if let Some(v) = params.get(param).and_then(number_of).and_then(|c| class_value(tag, c)) {
            user = v;
        }
    }
    axis_location(params, &axis.name).unwrap_or(user)
}

/// Where a master sits on axis `i` in user coordinates. Its weight and width are its name parts.
pub fn master_user_value(font: &Font, i: usize, m: &Master) -> f64 {
    let tag = font.axes.get(i).map_or("", |a| a.tag.as_str());
    let part = if tag == "wght" { m.name_parts.weight.as_ref() } else { m.name_parts.width.as_ref() };
    let class = part.map(|p| StyleClass::Name(p.clone()));
    user_value(font, i, &m.location, class.as_ref(), "Regular", &m.params)
}

/// Where an instance sits on axis `i` in user coordinates.
pub fn instance_user_value(font: &Font, i: usize, inst: &Instance) -> f64 {
    let tag = font.axes.get(i).map_or("", |a| a.tag.as_str());
    let (class, default) = if tag == "wght" { (inst.weight_class.as_ref(), "Regular") } else { (inst.width_class.as_ref(), "Medium (normal)") };
    user_value(font, i, &inst.location, class, default, &inst.params)
}

/// The words every master name shares, in the first master's order (glyphsLib's
/// `find_base_style`).
fn base_style(masters: &[Master]) -> String {
    let Some(first) = masters.first() else { return String::new() };
    let mut base: Vec<&str> = first.name.split_whitespace().collect();
    for m in masters {
        base = m.name.split_whitespace().filter(|w| base.contains(w)).collect();
    }
    base.join(" ")
}

/// The origin master by glyphsLib's rules, which `heuristic` alone ignores the origin
/// parameters for: the master the `Variable Font Origin` parameter names by id (or the older
/// `Variation Font Origin` by name), else the master named by the words all master names share
/// (`Regular` when none), else one with that name once `Regular` is left out of it, else the
/// first.
fn origin_by(font: &Font, heuristic: bool) -> Option<&Master> {
    let masters = font.masters.as_slice();
    if !heuristic {
        if let Some(id) = font.info.params.get(ORIGIN) {
            if let Some(m) = id.as_str().and_then(|id| masters.iter().find(|m| m.id == id)) {
                return Some(m);
            }
        } else if let Some(m) = font.info.params.get(ORIGIN_BY_NAME).and_then(Value::as_str).and_then(|n| masters.iter().find(|m| m.name == n)) {
            return Some(m);
        }
    }
    let base = base_style(masters);
    let base = if base.is_empty() { "Regular".to_string() } else { base };
    masters
        .iter()
        .find(|m| m.name == base)
        .or_else(|| masters.iter().find(|m| m.name.split(' ').filter(|w| *w != "Regular").collect::<Vec<_>>().join(" ") == base))
        .or_else(|| masters.first())
}

/// The origin master: the one at every axis's default.
pub fn origin(font: &Font) -> Option<&Master> {
    origin_by(font, false)
}

/// The master glyphsLib takes as the origin when no parameter names one.
pub fn origin_without_parameter(font: &Font) -> Option<&Master> {
    origin_by(font, true)
}

/// fontTools' `piecewiseLinearMap`: `v` through the `(input, output)` points, interpolated
/// between them and shifted by the nearest end's offset outside them.
pub fn piecewise_linear_map(v: f64, pairs: &[(f64, f64)]) -> f64 {
    if let Some((_, out)) = pairs.iter().find(|(k, _)| *k == v) {
        return *out;
    }
    let (Some(lo), Some(hi)) = (pairs.iter().min_by(|a, b| a.0.total_cmp(&b.0)).copied(), pairs.iter().max_by(|a, b| a.0.total_cmp(&b.0)).copied())
    else {
        return v;
    };
    if v < lo.0 {
        return v + lo.1 - lo.0;
    }
    if v > hi.0 {
        return v + hi.1 - hi.0;
    }
    let below = pairs.iter().filter(|(k, _)| *k < v).max_by(|a, b| a.0.total_cmp(&b.0));
    let above = pairs.iter().filter(|(k, _)| *k > v).min_by(|a, b| a.0.total_cmp(&b.0));
    match (below, above) {
        (Some((a, va)), Some((b, vb))) if b != a => va + (vb - va) * (v - a) / (b - a),
        _ => v,
    }
}

/// `pairs` as a dictionary keyed by the first value, later pairs replacing earlier ones with the
/// same key but keeping its place, then sorted by key (Python's `sorted(dict.items())`).
fn as_sorted_map(pairs: impl IntoIterator<Item = (f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (k, v) in pairs {
        if !(k.is_finite() && v.is_finite()) {
            continue;
        }
        match out.iter_mut().find(|(x, _)| *x == k) {
            Some(slot) => slot.1 = v,
            None => out.push((k, v)),
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    out
}

/// The `(user, design)` pairs the `Axis Mappings` parameter gives for the axis tagged `tag`,
/// sorted by user value; `None` when the parameter has no entry for it.
pub fn mapping_parameter(font: &Font, tag: &str) -> Option<Vec<(f64, f64)>> {
    let d = font.info.params.get(AXIS_MAPPINGS)?.as_dict()?.get(tag)?.as_dict()?;
    let pairs = d.entries.iter().filter_map(|(k, v)| Some((k.as_str().trim().parse::<f64>().ok()?, number_of(v)?)));
    Some(as_sorted_map(pairs))
}

/// The user-to-design map of an axis and its default in user coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapping {
    /// `(user, design)`, sorted by user value; never empty.
    pub pairs: Vec<(f64, f64)>,
    pub default: f64,
}

impl Mapping {
    pub fn is_identity(&self) -> bool {
        self.pairs.iter().all(|(u, d)| u == d)
    }

    pub fn minimum(&self) -> f64 {
        self.pairs.first().map_or(self.default, |p| p.0)
    }

    pub fn maximum(&self) -> f64 {
        self.pairs.last().map_or(self.default, |p| p.0)
    }

    /// The design value of a user value.
    pub fn to_design(&self, user: f64) -> f64 {
        piecewise_linear_map(user, &self.pairs)
    }

    /// The user value of a design value (the map read backwards, a later pair winning where two
    /// share a design value).
    pub fn to_user(&self, design: f64) -> f64 {
        piecewise_linear_map(design, &as_sorted_map(self.pairs.iter().map(|(u, d)| (*d, *u))))
    }
}

/// Whether every master has an `Axis Location` parameter, so that the masters' user coordinates
/// make the map.
fn uses_axis_locations(font: &Font) -> bool {
    !font.axes.is_empty()
        && font.masters.iter().all(|m| m.params.get(AXIS_LOCATION).is_some_and(|v| v.as_array().is_some_and(|a| !a.items.is_empty())))
}

/// The static instances that are exported.
fn active_instances(font: &Font) -> impl Iterator<Item = &Instance> {
    font.instances.iter().filter(|i| i.exports && i.kind.as_deref() != Some("variable"))
}

/// The map and user default of axis `i` by glyphsLib's `to_designspace_axes`: from the `Axis
/// Mappings` parameter; else, when every master has an `Axis Location`, from the masters' user
/// coordinates (and the instances' `Axis Location`s); else from the exported instances' user
/// coordinates when they make a map that is not the identity, else the masters' design
/// coordinates as their user coordinates. The default is the origin master's user coordinate.
///
/// Unlike glyphsLib, an axis the `Axis Mappings` parameter leaves out gets the next rule's map
/// (glyphsLib leaves the axis out of the designspace); instances that put one user value at
/// different design values make no map (glyphsLib keeps the last one's).
///
/// When the map is the identity, the font's `Virtual Master` parameters widen the range: each
/// one's location on the axis becomes a point of the map (glyphsLib's issue 859 rule).
pub fn mapping(font: &Font, i: usize) -> Mapping {
    let Some(axis) = font.axes.get(i) else { return Mapping { pairs: vec![(0.0, 0.0)], default: 0.0 } };
    let regular = origin(font);
    let regular_design = regular.map_or(0.0, |m| design_value(&m.location, i));
    let reverse_default = |pairs: &[(f64, f64)]| piecewise_linear_map(regular_design, &as_sorted_map(pairs.iter().map(|(u, d)| (*d, *u))));
    let (pairs, default) = if let Some(pairs) = mapping_parameter(font, &axis.tag).filter(|p| !p.is_empty()) {
        let default = reverse_default(&pairs);
        (pairs, default)
    } else if uses_axis_locations(font) {
        let masters = font.masters.iter().map(|m| (master_user_value(font, i, m), design_value(&m.location, i)));
        let instances = active_instances(font).filter_map(|inst| Some((axis_location(&inst.params, &axis.name)?, design_value(&inst.location, i))));
        let pairs = as_sorted_map(masters.chain(instances));
        let default = regular.map_or(regular_design, |m| master_user_value(font, i, m));
        (pairs, default)
    } else {
        let points: Vec<(f64, f64)> =
            active_instances(font).map(|inst| (instance_user_value(font, i, inst), design_value(&inst.location, i))).collect();
        // Instances that give one user value different design values do not make a map.
        let conflict = points.iter().any(|(u, d)| points.iter().any(|(v, e)| u == v && d != e));
        let from_instances = if conflict { Vec::new() } else { as_sorted_map(points) };
        let pairs = if !from_instances.is_empty() && from_instances.iter().any(|(u, d)| u != d) {
            from_instances
        } else {
            as_sorted_map(font.masters.iter().map(|m| (design_value(&m.location, i), design_value(&m.location, i))))
        };
        let default = reverse_default(&pairs);
        (pairs, default)
    };
    let pairs = if pairs.is_empty() { vec![(default, default)] } else { pairs };
    let pairs = if pairs.iter().all(|(u, d)| u == d) {
        let virtual_points = virtual_locations(font, &axis.name).map(|v| (v, v));
        as_sorted_map(pairs.into_iter().chain(virtual_points))
    } else {
        pairs
    };
    let mut mapping = Mapping { pairs, default };
    // The default, clamped to the range.
    mapping.default = default.max(mapping.minimum()).min(mapping.maximum());
    if !mapping.default.is_finite() {
        mapping.default = mapping.minimum();
    }
    mapping
}

/// The locations on the axis named `axis` of the font's switched-on `Virtual Master` parameters
/// (each a list of `{Axis, Location}`).
fn virtual_locations<'a>(font: &'a Font, axis: &'a str) -> impl Iterator<Item = f64> + 'a {
    font.info
        .params
        .0
        .iter()
        .filter(|p| p.name == VIRTUAL_MASTER && !p.disabled)
        .filter_map(|p| p.value.as_array())
        .flat_map(|a| a.items.iter())
        .filter_map(|v| v.as_dict())
        .filter(move |d| d.get("Axis").and_then(Value::as_str) == Some(axis))
        .filter_map(|d| d.get("Location").and_then(Value::as_num).map(Num::as_f64))
}

/// The user range of axis `i`: the one the source states, else the one [`mapping`] derives.
pub fn range(font: &Font, i: usize) -> crate::AxisRange {
    if let Some(r) = font.axes.get(i).and_then(|a| a.range) {
        return r;
    }
    let m = mapping(font, i);
    crate::AxisRange { minimum: m.minimum(), default: m.default, maximum: m.maximum() }
}

/// The integer form of a whole number, for map keys and the like.
pub fn whole(v: f64) -> Option<i64> {
    // In range and whole, so the conversion is exact.
    #[allow(clippy::cast_possible_truncation)]
    (v.fract() == 0.0 && v.abs() < 1e15).then_some(v as i64)
}

/// A number as a property-list value: an integer when whole.
pub fn number_value(v: f64) -> Option<Value> {
    match whole(v) {
        Some(i) => Some(Value::Number(fontcraft_plist::Number::int(i))),
        None => fontcraft_plist::Number::float(v).map(Value::Number),
    }
}

/// Whether a parameter value is a whole number.
pub fn is_whole(v: &Value) -> bool {
    matches!(v.as_num(), Some(Num::Int(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Axis, Param};
    use fontcraft_plist::{Dict, Number, Str};
    use std::sync::Arc;

    fn font(axes: &[(&str, &str)], masters: &[(&str, &[f64])]) -> Font {
        let mut f = Font::new("Axes");
        f.axes = Arc::new(axes.iter().map(|(n, t)| Axis { name: (*n).into(), tag: (*t).into(), ..Default::default() }).collect());
        let template = f.masters[0].clone();
        f.masters = Arc::new(
            masters
                .iter()
                .enumerate()
                .map(|(k, (name, loc))| Master { id: format!("m{k}"), name: (*name).into(), location: loc.to_vec(), ..template.clone() })
                .collect(),
        );
        f
    }

    fn mappings(tag: &str, pairs: &[(&str, f64)]) -> Param {
        let inner = Dict { entries: pairs.iter().map(|(k, v)| (Str::new(*k), number_value(*v).unwrap())).collect(), ..Default::default() };
        Param::new(AXIS_MAPPINGS, Value::Dict(Dict { entries: vec![(Str::new(tag), Value::Dict(inner))], ..Default::default() }))
    }

    #[test]
    fn the_axis_mappings_parameter_gives_the_map_and_the_origin_the_default() {
        let mut f = font(&[("Weight", "wght")], &[("ExtraLight", &[200.0]), ("Regular", &[400.0]), ("Bold", &[710.0])]);
        Arc::make_mut(&mut f.info).params.0.push(mappings("wght", &[("200", 200.0), ("400", 400.0), ("700", 710.0), ("300", 275.0)]));
        let m = mapping(&f, 0);
        assert_eq!(m.pairs, [(200.0, 200.0), (300.0, 275.0), (400.0, 400.0), (700.0, 710.0)]);
        assert_eq!((m.minimum(), m.default, m.maximum()), (200.0, 400.0, 700.0), "Regular is the origin");
        assert_eq!(m.to_user(275.0), 300.0);
        assert_eq!(m.to_design(350.0), 337.5);
    }

    #[test]
    fn virtual_masters_widen_an_identity_map() {
        let mut f = font(&[("Weight", "wght"), ("Cap Height", "CHGT")], &[("Regular", &[400.0, 700.0])]);
        let vm = |axis: &str, loc: i64| {
            let mut d = Dict::new();
            d.insert("Axis", Value::String(Str::new(axis)));
            d.insert("Location", Value::Number(Number::int(loc)));
            Value::Dict(d)
        };
        let params = &mut Arc::make_mut(&mut f.info).params.0;
        params.push(Param::new(
            VIRTUAL_MASTER,
            Value::Array(fontcraft_plist::Array { items: vec![vm("Cap Height", 600), vm("Weight", 400)], ..Default::default() }),
        ));
        params.push(Param::new(VIRTUAL_MASTER, Value::Array(fontcraft_plist::Array { items: vec![vm("Cap Height", 800)], ..Default::default() })));
        let m = mapping(&f, 1);
        assert_eq!((m.minimum(), m.default, m.maximum()), (600.0, 700.0, 800.0));
        assert!(m.is_identity());
        assert_eq!(range(&f, 0), crate::AxisRange { minimum: 400.0, default: 400.0, maximum: 400.0 });
        // A switched-off one does not count, and a map that is not the identity stays as it is.
        Arc::make_mut(&mut f.info).params.0[1].disabled = true;
        assert_eq!(mapping(&f, 1).maximum(), 700.0);
        Arc::make_mut(&mut f.info).params.0.push(mappings("CHGT", &[("650", 700.0), ("700", 720.0)]));
        assert_eq!(mapping(&f, 1).pairs, [(650.0, 700.0), (700.0, 720.0)]);
    }

    #[test]
    fn without_parameters_masters_or_instances_make_the_map() {
        let mut f = font(&[("Weight", "wght"), ("Width", "wdth")], &[("Light", &[40.0, 100.0]), ("Bold", &[90.0, 100.0])]);
        // Masters only: the identity over their design coordinates; the first is the origin.
        let m = mapping(&f, 0);
        assert!(m.is_identity());
        assert_eq!((m.minimum(), m.default, m.maximum()), (40.0, 40.0, 90.0));
        // Instances with weight classes make a map.
        let inst = |name: &str, w: f64, class: &str| Instance {
            name: name.into(),
            location: vec![w, 100.0],
            weight_class: Some(StyleClass::Name(class.into())),
            ..Default::default()
        };
        f.instances =
            Arc::new(vec![inst("Light", 40.0, "Light"), inst("Bold", 90.0, "Bold"), Instance { exports: false, ..inst("X", 60.0, "Black") }]);
        let m = mapping(&f, 0);
        assert_eq!(m.pairs, [(300.0, 40.0), (700.0, 90.0)]);
        assert_eq!(m.default, 300.0);
        // Two instances at one user value and different design values: the masters' map.
        let mut g = f.clone();
        Arc::make_mut(&mut g.instances).push(inst("Light 2", 50.0, "Light"));
        assert!(mapping(&g, 0).is_identity());
        // The width axis: every instance is at the normal width.
        assert_eq!(mapping(&f, 1).pairs, [(100.0, 100.0)]);
        // A `weightClass` parameter and an `Axis Location` override the name.
        let mut i = inst("Semi", 70.0, "Bold");
        i.params.0.push(Param::new("weightClass", Value::Number(Number::int(600))));
        assert_eq!(instance_user_value(&f, 0, &i), 600.0);
        let loc = |name: &str, v: i64| {
            let d = Dict {
                entries: vec![(Str::new("Axis"), Value::String(Str::new(name))), (Str::new("Location"), Value::Number(Number::int(v)))],
                ..Default::default()
            };
            Param::new(AXIS_LOCATION, Value::Array(fontcraft_plist::Array { items: vec![Value::Dict(d)], ..Default::default() }))
        };
        i.params.0.push(loc("Weight", 650));
        assert_eq!(instance_user_value(&f, 0, &i), 650.0);
        // Every master with an `Axis Location`: their user coordinates.
        for (m, v) in Arc::make_mut(&mut f.masters).iter_mut().zip([300, 800]) {
            m.params.0.push(loc("Weight", v));
        }
        let m = mapping(&f, 0);
        assert_eq!(m.pairs, [(300.0, 40.0), (800.0, 90.0)]);
        assert_eq!(m.default, 300.0);
    }

    #[test]
    fn the_origin_is_named_or_found_by_the_shared_style() {
        let mut f = font(&[("Weight", "wght")], &[("Light Italic", &[1.0]), ("Italic", &[2.0]), ("Bold Italic", &[3.0])]);
        assert_eq!(origin(&f).unwrap().id, "m1", "the words every name shares");
        let mut g = font(&[("Weight", "wght")], &[("Thin", &[1.0]), ("Regular", &[2.0])]);
        assert_eq!(origin(&g).unwrap().id, "m1");
        Arc::make_mut(&mut g.info).params.0.push(Param::new(ORIGIN, Value::String(Str::new("m0"))));
        assert_eq!(origin(&g).unwrap().id, "m0");
        assert_eq!(origin_without_parameter(&g).unwrap().id, "m1");
        Arc::make_mut(&mut f.masters)[1].name = "Medium".into();
        assert_eq!(origin(&f).unwrap().id, "m0", "no shared style and no Regular: the first");
    }

    #[test]
    fn piecewise_maps_extrapolate_by_the_end_offset() {
        let m = [(100.0, 10.0), (200.0, 30.0)];
        assert_eq!(piecewise_linear_map(150.0, &m), 20.0);
        assert_eq!(piecewise_linear_map(50.0, &m), -40.0);
        assert_eq!(piecewise_linear_map(250.0, &m), 80.0);
        assert_eq!(piecewise_linear_map(7.0, &[]), 7.0);
    }

    #[test]
    fn weight_and_width_names_have_user_values() {
        assert_eq!(style_value("wght", &StyleClass::Name("Semi Bold".into())), Some(600.0));
        assert_eq!(style_value("wdth", &StyleClass::Name("SemiCondensed".into())), Some(87.5));
        assert_eq!(style_value("wdth", &StyleClass::Number(7.0)), Some(125.0));
        assert_eq!(style_value("wdth", &StyleClass::Number(12.0)), None);
        assert_eq!(style_value("wght", &StyleClass::Name("Clearly Not".into())), None);
        assert_eq!(width_class_value(f64::NAN), None);
    }
}
