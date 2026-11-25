//! The names special layers are shown with in the Layers palette and elsewhere (M9.16; the
//! palette that shows them is M9.21's).
//!
//! Rows: `intermediateLayer.braceLabel`, `alternateLayer.bracketLabel`,
//! `alternateLayer.axisAbbreviations`.
//!
//! Contract: pure text functions of the font and a layer. An intermediate layer reads
//! `{coordinate, …}`, an alternate layer `[range …]` with each axis abbreviated, a layer that is
//! both shows both ([`special_label`]). Non-special layers have no label.
//!
//! - Brace label: one value per font axis, in axis order, joined by `, `. An empty coordinate
//!   shows the value it inherits from the layer's master (a guess on
//!   `intermediateLayer.braceLabel`).
//! - Bracket label: one entry per axis that has a bound, joined by `, `: `90‹wg` for a minimum,
//!   `wg‹140` for a maximum (a guess: the handbook shows no maximum alone), `25‹wd‹50` for both.
//!   An alternate layer without any bound is `[]`.
//! - Numbers are written in their shortest form, rounded to three decimals: `90`, `90.5`.

use fontcraft_model::{Axis, Font, Layer, axes};

/// The intermediate-layer label of `layer` (`{400}`, `{400, 100}`); `None` when it is not an
/// intermediate layer.
pub fn brace_label(font: &Font, layer: &Layer) -> Option<String> {
    let coords = &layer.info.attrs.coordinates;
    if coords.is_empty() {
        return None;
    }
    let master = font.masters.iter().find(|m| m.id == layer.master);
    // Without axes there is nothing to inherit from: show what the layer stores.
    let count = if font.axes.is_empty() { coords.len() } else { font.axes.len() };
    let parts: Vec<String> = (0..count)
        .map(|i| match coords.get(i).copied().flatten() {
            Some(v) => number(v),
            None => master.map(|m| number(axes::design_value(&m.location, i))).unwrap_or_default(),
        })
        .collect();
    Some(format!("{{{}}}", parts.join(", ")))
}

/// The alternate-layer label of `layer` (`[600‹wg]`); `None` when it is not an alternate layer.
pub fn bracket_label(font: &Font, layer: &Layer) -> Option<String> {
    let rules = &layer.info.attrs.axis_rules;
    if rules.is_empty() {
        return None;
    }
    let parts: Vec<String> = rules
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let abbr = font.axes.get(i).map_or_else(|| format!("axis{}", i.saturating_add(1)), axis_abbreviation);
            match (r.min, r.max) {
                (Some(lo), Some(hi)) => Some(format!("{}‹{abbr}‹{}", number(lo), number(hi))),
                (Some(lo), None) => Some(format!("{}‹{abbr}", number(lo))),
                (None, Some(hi)) => Some(format!("{abbr}‹{}", number(hi))),
                (None, None) => None,
            }
        })
        .collect();
    Some(format!("[{}]", parts.join(", ")))
}

/// The whole label of a special layer: the bracket part, then the brace part, separated by a
/// space when the layer is both (`[90‹wg] {400}`); `None` for a layer that is neither.
pub fn special_label(font: &Font, layer: &Layer) -> Option<String> {
    match (bracket_label(font, layer), brace_label(font, layer)) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    }
}

/// What `layer` is called in a message: its master's name, with the label of a special layer (`Light`, `Light {400}`).
pub fn layer_title(font: &Font, layer: &Layer) -> String {
    let master = font.masters.iter().find(|m| m.id == layer.master).map_or(layer.master.as_str(), |m| m.name.as_str());
    match special_label(font, layer) {
        Some(label) => format!("{master} {label}"),
        None => master.to_string(),
    }
}

/// The short form of `axis` used in alternate-layer labels.
pub fn axis_abbreviation(axis: &Axis) -> String {
    match axis.tag.as_str() {
        "ital" => "it",
        "opsz" => "oz",
        "slnt" => "sl",
        "wdth" => "wd",
        "wght" => "wg",
        other => other,
    }
    .to_string()
}

/// `v` in its shortest form, rounded to three decimals; `?` for a value that is not finite.
fn number(v: f64) -> String {
    if !v.is_finite() {
        return "?".to_string();
    }
    let r = (v * 1000.0).round() / 1000.0;
    // −0 reads as 0.
    format!("{}", if r == 0.0 { 0.0 } else { r })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Axis, AxisRule, Font, Layer, Master};

    use super::*;

    fn font(tags: &[&str]) -> Font {
        let mut f = Font::new("Labels");
        f.axes = Arc::new(tags.iter().map(|t| Axis { name: (*t).into(), tag: (*t).into(), ..Axis::default() }).collect());
        f.masters = Arc::new(vec![Master { id: "m".into(), name: "Regular".into(), location: vec![400.0, 100.0, 12.0], ..Master::default() }]);
        f
    }

    fn layer() -> Layer {
        let mut l = Layer::new("m", 500.0);
        l.id = "x".into();
        l
    }

    #[test]
    fn brace_label_lists_coordinates_in_axis_order_with_inherited_values() {
        let f = font(&["wght", "wdth"]);
        let mut l = layer();
        assert_eq!(brace_label(&f, &l), None, "a backup layer has no label");
        l.info.attrs.coordinates = vec![Some(90.0), Some(120.5)];
        assert_eq!(brace_label(&f, &l).as_deref(), Some("{90, 120.5}"));
        l.info.attrs.coordinates = vec![Some(-0.0001), None];
        assert_eq!(brace_label(&f, &l).as_deref(), Some("{0, 100}"), "an empty coordinate shows the master's value");
    }

    #[test]
    fn bracket_label_abbreviates_the_five_registered_axes() {
        let f = font(&["wght", "wdth", "XHGT"]);
        let mut l = layer();
        assert_eq!(bracket_label(&f, &l), None);
        l.info.attrs.axis_rules = vec![AxisRule { min: Some(90.0), max: None }];
        assert_eq!(bracket_label(&f, &l).as_deref(), Some("[90‹wg]"));
        l.info.attrs.axis_rules =
            vec![AxisRule { min: Some(80.0), max: None }, AxisRule { min: Some(25.0), max: Some(50.0) }, AxisRule { min: None, max: Some(500.0) }];
        assert_eq!(bracket_label(&f, &l).as_deref(), Some("[80‹wg, 25‹wd‹50, XHGT‹500]"));
        l.info.attrs.axis_rules = vec![AxisRule::default(); 3];
        assert_eq!(bracket_label(&f, &l).as_deref(), Some("[]"), "the partner of a master layer with a range");
        let abbr: Vec<String> = ["ital", "opsz", "slnt", "wdth", "wght", "GRAD"]
            .iter()
            .map(|t| axis_abbreviation(&Axis { tag: (*t).into(), ..Axis::default() }))
            .collect();
        assert_eq!(abbr, ["it", "oz", "sl", "wd", "wg", "GRAD"]);
    }

    #[test]
    fn a_layer_that_is_both_shows_both_labels() {
        let f = font(&["wght"]);
        let mut l = layer();
        assert_eq!(special_label(&f, &l), None);
        l.info.attrs.coordinates = vec![Some(400.0)];
        assert_eq!(special_label(&f, &l).as_deref(), Some("{400}"));
        l.info.attrs.axis_rules = vec![AxisRule { min: Some(90.0), max: None }];
        assert_eq!(special_label(&f, &l).as_deref(), Some("[90‹wg] {400}"));
    }
}
