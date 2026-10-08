//! Designspace axes as the model's axes, and their maps as the font's `Axis Mappings` parameter.
//!
//! When a designspace's maps become the parameter, and the identity map an axis without one
//! gets so that its range survives, are ported from glyphsLib 6.15.0 (Apache-2.0):
//! `builder/axes.py` (`to_glyphs_axes`, `_has_meaningful_map`), `builder/builders.py`
//! (`_valid_designspace`) and `builder/stat.py` (`is_stat_only_ital`). The element layout follows
//! fontTools' `designspaceLib` writer (MIT).

use fontcraft_model::axes::{self as rules, AXIS_MAPPINGS};
use fontcraft_model::{Axis, AxisLabel, AxisRange, Font, LangName};
use fontcraft_plist::xml::Element;
use fontcraft_plist::{Dict, Str, Value};

use super::{DOC, MAX_ITEMS, int_or_float, number};
use crate::{Result, bad};

const LANG: &str = "xml:lang";

fn lang_names(el: &Element) -> Vec<LangName> {
    el.elements()
        .filter(|e| e.name() == "labelname")
        .take(MAX_ITEMS)
        .filter_map(|e| Some(LangName { language: e.attr(LANG)?.to_string(), name: e.text() }))
        .collect()
}

fn flag(el: &Element, attr: &str) -> bool {
    el.attr(attr) == Some("true")
}

fn read_label(el: &Element) -> Result<Option<AxisLabel>> {
    let (Some(name), Some(user_value)) = (el.attr("name"), number(el, "uservalue")?) else { return Ok(None) };
    Ok(Some(AxisLabel {
        name: name.to_string(),
        user_value,
        user_minimum: number(el, "userminimum")?,
        user_maximum: number(el, "usermaximum")?,
        linked_user_value: number(el, "linkeduservalue")?,
        elidable: flag(el, "elidable"),
        older_sibling: flag(el, "oldersibling"),
        names: lang_names(el),
    }))
}

/// A discrete axis's values, as written.
fn values(el: &Element) -> Result<Vec<f64>> {
    let Some(text) = el.attr("values") else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for v in text.split_whitespace() {
        if out.len() >= MAX_ITEMS {
            return Err(bad(DOC, format!("an axis has more than {MAX_ITEMS} values")));
        }
        match v.parse::<f64>() {
            Ok(f) if f.is_finite() => out.push(f),
            _ => return Err(bad(DOC, format!("`<axis values=\"{text}\">` is not a list of numbers"))),
        }
    }
    Ok(out)
}

/// An `<axis>` element as the model's axis.
pub(super) fn read_axis(a: &Element) -> Result<Axis> {
    let name = a.attr("name").ok_or_else(|| bad(DOC, "an axis has no name"))?.to_string();
    let values = values(a)?;
    let default = number(a, "default")?;
    let range = if let Some(first) = values.first() {
        let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some(AxisRange { minimum, default: default.unwrap_or(*first), maximum })
    } else {
        match (number(a, "minimum")?, number(a, "maximum")?, default) {
            (None, None, None) => None,
            (min, max, d) => {
                let d = d.or(min).or(max).unwrap_or(0.0);
                Some(AxisRange { minimum: min.unwrap_or(d), default: d, maximum: max.unwrap_or(d) })
            }
        }
    };
    let (mut labels, mut ordering) = (Vec::new(), None);
    if let Some(l) = a.child("labels") {
        ordering = l.attr("ordering").and_then(|o| o.trim().parse::<u32>().ok());
        for e in l.elements().filter(|e| e.name() == "label").take(MAX_ITEMS) {
            labels.extend(read_label(e)?);
        }
    }
    Ok(Axis {
        tag: a.attr("tag").unwrap_or("").to_string(),
        hidden: a.attr("hidden").is_some_and(|h| h == "1" || h == "true"),
        name,
        range,
        values,
        label_names: lang_names(a),
        labels,
        ordering,
        ..Default::default()
    })
}

/// An axis's `<map>` elements: `(input, output)`, as written.
pub(super) fn read_map(a: &Element) -> Result<Vec<(f64, f64)>> {
    let mut map = Vec::new();
    for m in a.elements().filter(|e| e.name() == "map").take(MAX_ITEMS) {
        if let (Some(i), Some(o)) = (number(m, "input")?, number(m, "output")?) {
            map.push((i, o));
        }
    }
    Ok(map)
}

/// What one axis contributes to the `Axis Mappings` parameter.
pub(super) struct MapSource<'a> {
    pub axis: &'a Axis,
    /// The `(user, design)` pairs as written.
    pub map: Vec<(f64, f64)>,
    /// The lowest and highest design coordinate of the sources on the axis.
    pub extent: Option<(f64, f64)>,
}

/// glyphsLib's axis that only says whether the family is italic, for STAT; it is not an axis of
/// the reference's font.
pub(super) fn stat_only_italic(a: &Axis) -> bool {
    a.tag == "ital" && (a.values == [0.0] || a.values == [1.0])
}

/// The map an axis is given: its own, or the identity over its range so that the range survives.
fn filled(s: &MapSource<'_>) -> Vec<(f64, f64)> {
    if !s.map.is_empty() {
        return s.map.clone();
    }
    match s.axis.range {
        Some(r) if r.minimum == r.maximum => vec![(r.minimum, r.minimum)],
        Some(r) => vec![(r.minimum, r.minimum), (r.maximum, r.maximum)],
        None => Vec::new(),
    }
}

/// Whether a map says more than the sources do: it is not the identity, or its ends are not
/// the sources' extent (a range wider than the masters).
fn meaningful(map: &[(f64, f64)], extent: Option<(f64, f64)>) -> bool {
    let (Some(first), Some(last)) = (map.first(), map.last()) else { return false };
    map.iter().any(|(u, d)| u != d) || extent.is_none_or(|(lo, hi)| lo != first.0 || hi != last.0)
}

/// The `Axis Mappings` parameter a designspace's axes make: `{tag: {user: design}}` for every axis,
/// when the map of any says more than its sources; `None` otherwise.
pub(super) fn mappings_param(sources: &[MapSource<'_>]) -> Option<Value> {
    let axes: Vec<(&MapSource<'_>, Vec<(f64, f64)>)> = sources.iter().filter(|s| !stat_only_italic(s.axis)).map(|s| (s, filled(s))).collect();
    if !axes.iter().any(|(s, map)| meaningful(map, s.extent)) {
        return None;
    }
    let mut out = Dict::new();
    for (s, map) in axes.iter().filter(|(_, m)| !m.is_empty()) {
        let mut inner = Dict::new();
        for (user, design) in map {
            if let Some(v) = rules::number_value(*design) {
                inner.insert(Str::new(int_or_float(*user)), v);
            }
        }
        out.insert(Str::new(s.axis.tag.as_str()), Value::Dict(inner));
    }
    Some(Value::Dict(out))
}

/// The lowest and highest of the values, per axis.
pub(super) fn extents<'a>(locations: impl Iterator<Item = &'a [f64]>, axes: usize) -> Vec<Option<(f64, f64)>> {
    let mut out: Vec<Option<(f64, f64)>> = vec![None; axes];
    for loc in locations {
        for (slot, v) in out.iter_mut().zip(loc) {
            *slot = Some(match *slot {
                Some((lo, hi)) => (lo.min(*v), hi.max(*v)),
                None => (*v, *v),
            });
        }
    }
    out
}

/// The `(user, design)` pairs the model gives axis `i`, and its range: the stated range with the
/// map of the `Axis Mappings` parameter, else both derived ([`rules::mapping`]). An identity map
/// is no map.
pub(super) fn map_and_range(font: &Font, i: usize) -> (Vec<(f64, f64)>, AxisRange) {
    let Some(axis) = font.axes.get(i) else { return (Vec::new(), AxisRange { minimum: 0.0, default: 0.0, maximum: 0.0 }) };
    let (map, range) = match axis.range {
        Some(r) => (rules::mapping_parameter(font, &axis.tag).unwrap_or_default(), r),
        None => {
            let m = rules::mapping(font, i);
            let range = AxisRange { minimum: m.minimum(), default: m.default, maximum: m.maximum() };
            (m.pairs, range)
        }
    };
    let map = if map.iter().all(|(u, d)| u == d) { Vec::new() } else { map };
    (map, range)
}

fn lang_elements(names: &[LangName]) -> Vec<Element> {
    let mut sorted: Vec<&LangName> = names.iter().collect();
    sorted.sort_by(|a, b| a.language.cmp(&b.language));
    sorted.into_iter().map(|n| Element::new("labelname").with_attr(LANG, &n.language).with_text(&n.name)).collect()
}

/// `<labelname>` elements in designspaceLib's order (by language).
pub(super) fn label_name_elements(a: &Axis) -> Vec<Element> {
    lang_elements(&a.label_names)
}

pub(super) fn map_elements(map: &[(f64, f64)]) -> Vec<Element> {
    map.iter().map(|(i, o)| Element::new("map").with_attr("input", int_or_float(*i)).with_attr("output", int_or_float(*o))).collect()
}

/// The `<labels>` element, when the axis has labels or an ordering.
pub(super) fn labels_element(a: &Axis) -> Option<Element> {
    if a.labels.is_empty() && a.ordering.is_none() {
        return None;
    }
    let mut el = Element::new("labels");
    if let Some(o) = a.ordering {
        el.set_attr("ordering", o.to_string());
    }
    for l in &a.labels {
        let mut e = Element::new("label").with_attr("uservalue", int_or_float(l.user_value));
        if let Some(v) = l.user_minimum {
            e.set_attr("userminimum", int_or_float(v));
        }
        if let Some(v) = l.user_maximum {
            e.set_attr("usermaximum", int_or_float(v));
        }
        e.set_attr("name", &l.name);
        if l.elidable {
            e.set_attr("elidable", "true");
        }
        if l.older_sibling {
            e.set_attr("oldersibling", "true");
        }
        if let Some(v) = l.linked_user_value {
            e.set_attr("linkeduservalue", int_or_float(v));
        }
        for n in lang_elements(&l.names) {
            e = e.with_child(n);
        }
        el = el.with_child(e);
    }
    Some(el)
}

/// Sets the range attributes: `values` and `default` for a discrete axis, else `minimum`,
/// `maximum` and `default`.
pub(super) fn set_range(el: &mut Element, a: &Axis, range: AxisRange) {
    if a.values.is_empty() {
        el.remove_attr("values");
        el.set_attr("minimum", int_or_float(range.minimum));
        el.set_attr("maximum", int_or_float(range.maximum));
    } else {
        el.set_attr("values", a.values.iter().map(|v| int_or_float(*v)).collect::<Vec<_>>().join(" "));
    }
    el.set_attr("default", int_or_float(range.default));
}

/// A new `<axis>` element in designspaceLib's layout.
pub(super) fn axis_element(font: &Font, i: usize) -> Element {
    let Some(a) = font.axes.get(i) else { return Element::new("axis") };
    let (map, range) = map_and_range(font, i);
    let mut el = Element::new("axis").with_attr("tag", &a.tag).with_attr("name", &a.name);
    for c in label_name_elements(a).into_iter().chain(map_elements(&map)).chain(labels_element(a)) {
        el = el.with_child(c);
    }
    set_range(&mut el, a, range);
    if a.hidden {
        el.set_attr("hidden", "1");
    }
    el
}

/// The parameter's value, as the model holds it (enabled).
pub(super) fn model_mappings(font: &Font) -> Option<&Value> {
    font.info.params.get(AXIS_MAPPINGS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(range: Option<(f64, f64, f64)>) -> Axis {
        Axis {
            name: "Weight".into(),
            tag: "wght".into(),
            range: range.map(|(minimum, default, maximum)| AxisRange { minimum, default, maximum }),
            ..Default::default()
        }
    }

    #[test]
    fn a_map_becomes_the_parameter_only_when_it_says_more_than_the_sources() {
        let a = axis(Some((100.0, 400.0, 900.0)));
        // No map, a range as wide as the sources: nothing to keep.
        assert_eq!(mappings_param(&[MapSource { axis: &a, map: Vec::new(), extent: Some((100.0, 900.0)) }]), None);
        // A range wider than the sources: the identity over the range.
        let p = mappings_param(&[MapSource { axis: &a, map: Vec::new(), extent: Some((300.0, 700.0)) }]).unwrap();
        let wght = p.as_dict().unwrap().get("wght").unwrap().as_dict().unwrap();
        assert_eq!(wght.entries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["100", "900"]);
        // A map that bends: kept, and every other axis gets its identity map.
        let w = Axis { name: "Width".into(), tag: "wdth".into(), ..axis(Some((75.0, 100.0, 100.0))) };
        let p = mappings_param(&[
            MapSource { axis: &a, map: vec![(100.0, 20.0), (900.0, 200.0)], extent: Some((20.0, 200.0)) },
            MapSource { axis: &w, map: Vec::new(), extent: Some((75.0, 100.0)) },
        ])
        .unwrap();
        let d = p.as_dict().unwrap();
        assert_eq!(d.get("wght").unwrap().as_dict().unwrap().get("900"), rules::number_value(200.0).as_ref());
        assert_eq!(d.get("wdth").unwrap().as_dict().unwrap().len(), 2);
    }
}
