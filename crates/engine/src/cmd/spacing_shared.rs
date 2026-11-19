//! What the metric and kerning group fields show for several glyphs at once (the Info box with
//! several glyphs selected, the Font View inspector): the value every glyph shares, or a mark that
//! the values differ. The setters (`spacing.set*`, `kerning.set*Group`) take several glyphs and
//! change them all in one undo step. Behaviour notes: `docs/spec/08-spacing-kerning.md` § 1.2.

use fontcraft_model::{Glyph, KernGroups, Layer};
use fontcraft_spacing::{Field, derived_field, metric};
use serde_json::{Map, Value, json};

use super::metrics_keys::shown_key;
use super::*;
use crate::params::{Params, TargetKind};

/// Measured metrics are compared after rounding to this many parts of a unit, the precision a
/// field shows.
const PARTS: f64 = 100.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "spacing.getSharedMetrics",
            "Get Shared Metrics",
            [],
            None,
            "{glyphs: [names], layer?: id (default: the first master's layer)} what the metric and kerning group fields show for these glyphs together: a field shows a value only when every glyph shares it. A metric field reads each glyph's layer: while the glyphs share the metrics key that applies (`==…` for a layer's own key) it shows that key; with no key on any of them it shows the measured value they share to two decimals (null for empty layers); otherwise it is mixed. Glyphs without the layer are listed and left out of the metric fields; refused when none has it. Group fields compare every glyph → {glyphs: [names whose layer was read], layer, missing: [names without the layer], fields: {left|right|width|top|bottom|vertWidth|vertOrigin: {key: text | null (the shared key), value: number | null (the shared measured value), mixed: bool}}, groups: {left|right|top|bottom: {value: name | null, mixed: bool}}, derived: [fields that automatic alignment sets on at least one of the layers]}",
            has_doc,
            get_shared
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

/// The item every entry of `items` equals, or `None` when two differ or `items` is empty.
fn common<T: PartialEq>(items: &[T]) -> Option<&T> {
    let (first, rest) = items.split_first()?;
    rest.iter().all(|x| x == first).then_some(first)
}

/// The kerning group of one side.
fn group<'a>(k: &'a KernGroups, side: &str) -> Option<&'a str> {
    match side {
        "left" => k.left.as_deref(),
        "right" => k.right.as_deref(),
        "top" => k.top.as_deref(),
        _ => k.bottom.as_deref(),
    }
}

fn get_shared(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.getSharedMetrics", v)?;
    let font = s.font()?;
    let names = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let mut glyphs: Vec<&Glyph> = Vec::with_capacity(names.len());
    let mut layers: Vec<(&Glyph, &Layer)> = Vec::with_capacity(names.len());
    let mut read = Vec::new();
    let mut missing = Vec::new();
    for name in &names {
        let g: &Glyph = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        glyphs.push(g);
        match g.layers.iter().find(|l| l.id == layer) {
            Some(l) => {
                layers.push((g, l));
                read.push(name.as_str());
            }
            None => missing.push(name.as_str()),
        }
    }
    if layers.is_empty() {
        return Err(p.bad(format!("none of the glyphs has the layer `{layer}`")));
    }

    let mut fields = Map::new();
    for f in Field::ALL {
        let mut keys = Vec::with_capacity(layers.len());
        let mut values = Vec::with_capacity(layers.len());
        for &(g, l) in &layers {
            keys.push(shown_key(font, g, l, f).map(|(_, _, shown)| shown));
            let measured = metric(font, l, f).map_err(|e| p.bad(format!("glyph `{}`: {e}", g.name)))?;
            values.push(measured.map(|x| (x * PARTS).round() / PARTS));
        }
        let key = common(&keys);
        let value = common(&values);
        let mixed = match key {
            None => true,
            Some(Some(_)) => false,
            Some(None) => value.is_none(),
        };
        let entry = json!({"key": key.cloned().flatten(), "value": value.copied().flatten(), "mixed": mixed});
        fields.insert(f.name().into(), entry);
    }

    let mut groups = Map::new();
    for side in ["left", "right", "top", "bottom"] {
        let named: Vec<Option<&str>> = glyphs.iter().map(|g| group(&g.kern_groups, side)).collect();
        let shared = common(&named);
        groups.insert(side.into(), json!({"value": shared.copied().flatten(), "mixed": shared.is_none()}));
    }

    let derived: Vec<&str> = Field::ALL.iter().filter(|&&f| layers.iter().any(|&(_, l)| derived_field(font, l, f))).map(|f| f.name()).collect();
    Ok(json!({
        "glyphs": read,
        "layer": layer,
        "missing": missing,
        "fields": fields,
        "groups": groups,
        "derived": derived,
    }))
}
