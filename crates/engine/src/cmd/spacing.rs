//! Spacing: the five metric setters (a number or a metrics key, as typed into the Info box) and
//! the twelve spacing shortcuts of the Text tool. Behaviour notes: `docs/spec/08-spacing-kerning.md`
//! § 1 and § 2.

use fontcraft_model::{Font, Glyph, Layer, MAX_COORDINATE, MetricsKeys, Shape, Transform};
use fontcraft_spacing::{self as spacing, Entry, Field, SIDEBEARING_RANGE, Scope, WIDTH_RANGE};
use serde_json::{Map, Value, json};

use super::common::in_range;
use super::*;
use crate::params::{Params, TargetKind};

/// Largest step a spacing shortcut takes, in units.
const MAX_AMOUNT: f64 = 32_767.0;

/// One spacing shortcut: an engine command for the Text tool, on the given glyphs. `true` when
/// it takes `amount`.
macro_rules! nudge_spec {
    ($id:literal, $label:literal, $sc:literal, $what:literal, true, $run:expr) => {
        nudge_spec!(@spec $id, $label, $sc, $run, concat!("{glyphs: [names], layer?: layer id (default: the first master), amount?: units (default 1)} ", $what, ". Metrics keys stay and may then be out of sync; glyphs aligned automatically on that layer are skipped, as alignment sets their metrics; a master that takes its metrics from another is refused → {glyphs: [names moved], layer, widths: {glyph: width}, skipped: [names]}"))
    };
    ($id:literal, $label:literal, $sc:literal, $what:literal, false, $run:expr) => {
        nudge_spec!(@spec $id, $label, $sc, $run, concat!("{glyphs: [names], layer?: layer id (default: the first master)} ", $what, ". Metrics keys stay and may then be out of sync; glyphs aligned automatically on that layer are skipped, as alignment sets their metrics; a master that takes its metrics from another is refused → {glyphs: [names moved], layer, widths: {glyph: width}, skipped: [names]}"))
    };
    (@spec $id:literal, $label:literal, $sc:literal, $run:expr, $params:expr) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: &[],
            shortcut: Some($sc),
            params: $params,
            enabled: has_doc,
            run: $run,
            journal: true,
            targets: &[TargetKind::Glyphs],
            when: Some("textMode"),
            context: &[],
            files: &[],
            writes_files: false,
        }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    let g: &'static [TargetKind] = &[TargetKind::Glyphs];
    vec![
        cmd!(
            "spacing.setLeftSidebearing",
            "Set Left Sidebearing",
            [],
            None,
            "{glyphs: [names], layer?: layer id (default: the first master), value: number or metrics key, beam?: height} set the left sidebearing of each glyph's layer. With `beam`, a number is the distance from the left edge to where the outline first meets the horizontal line at that height (the metrics beam), and layers the line misses are skipped. A number moves the layer's outlines, components, anchors, guides and background sideways and changes the width by the same amount, and removes the key the field holds. A key (`n`, `=n`, `=h+8`, `=|n`, `=|`, `=64`, `=H@200`, `==u` for this layer only) is stored and applied at once. Empty layers are skipped. On a layer aligned automatically the field takes only an offset, `=+N` or `=-N` (`==+N` for this layer only), which alignment adds to the derived value; `=+0` removes it; a number or another key is refused → {glyphs: [names changed], layer, values: {glyph: left sidebearing}, key?: text as shown, skipped: [names]}",
            has_doc,
            set_left
        )
        .targets(g),
        cmd!(
            "spacing.setRightSidebearing",
            "Set Right Sidebearing",
            [],
            None,
            "{glyphs: [names], layer?: layer id (default: the first master), value: number or metrics key, beam?: height} set the right sidebearing of each glyph's layer. With `beam`, a number is the distance from where the outline last meets the horizontal line at that height (the metrics beam) to the right edge, and layers the line misses are skipped. A number changes the width and removes the key the field holds; a key (`n`, `=n`, `=o-10`, `=|n`, `=|`, `=64`, `==u` for this layer only) is stored and applied at once. Empty layers are skipped. On a layer aligned automatically the field takes only an offset, `=+N` or `=-N` (`==+N` for this layer only), which alignment adds to the derived value; `=+0` removes it; a number or another key is refused → {glyphs: [names changed], layer, values: {glyph: right sidebearing}, key?: text as shown, skipped: [names]}",
            has_doc,
            set_right
        )
        .targets(g),
        cmd!(
            "spacing.setWidth",
            "Set Width",
            [],
            None,
            "{glyphs: [names], layer?: layer id (default: the first master), value: number or metrics key} set the advance width of each glyph's layer, from 0 to 65535; the right sidebearing takes the change. A number removes the key the field holds; a key (`n`, `=n`, `=n*2`, `=600`, `==u` for this layer only; no `|` or `@`) is stored and applied at once. A layer aligned automatically takes its width from its components and refuses the field → {glyphs: [names changed], layer, values: {glyph: width}, key?: text as shown, skipped: []}",
            has_doc,
            set_width
        )
        .targets(g),
        cmd!(
            "spacing.setTopSidebearing",
            "Set Top Sidebearing",
            [],
            None,
            "{glyphs: [names], layer?: layer id (default: the first master), value: number or metrics key} set the top sidebearing of each glyph's layer: the vertical origin and the vertical width change by the same amount, so the bottom sidebearing stays. A number removes the key the field holds; a key (`n`, `=n+10`, `=|n` for the bottom sidebearing of n, `=|`, `==u`; no `@`) is stored and applied at once. Empty layers are skipped → {glyphs: [names changed], layer, values: {glyph: top sidebearing}, key?: text as shown, skipped: [names]}",
            has_doc,
            set_top
        )
        .targets(g),
        cmd!(
            "spacing.setBottomSidebearing",
            "Set Bottom Sidebearing",
            [],
            None,
            "{glyphs: [names], layer?: layer id (default: the first master), value: number or metrics key} set the bottom sidebearing of each glyph's layer by changing its vertical width. A number removes the key the field holds; a key (`n`, `=n+10`, `=|n` for the top sidebearing of n, `=|`, `==u`; no `@`) is stored and applied at once. Empty layers are skipped → {glyphs: [names changed], layer, values: {glyph: bottom sidebearing}, key?: text as shown, skipped: [names]}",
            has_doc,
            set_bottom
        )
        .targets(g),
        nudge_spec!("spacing.increaseLeftSidebearing", "Increase Left Sidebearing", "Ctrl+Left", "add `amount` (default 1) units of space left of each glyph's outline; the width grows by as much", true, |s, v| nudge(s, v, "spacing.increaseLeftSidebearing", "Increase Left Sidebearing", Nudge::Left, 1.0, Step::Amount)),
        nudge_spec!("spacing.increaseLeftSidebearingBy10", "Increase Left Sidebearing by 10", "Ctrl+Shift+Left", "add 10 units of space left of each glyph's outline; the width grows by as much", false, |s, v| nudge(s, v, "spacing.increaseLeftSidebearingBy10", "Increase Left Sidebearing by 10", Nudge::Left, 1.0, Step::Ten)),
        nudge_spec!("spacing.decreaseLeftSidebearing", "Decrease Left Sidebearing", "Ctrl+Right", "remove `amount` (default 1) units of space left of each glyph's outline; the width shrinks by as much", true, |s, v| nudge(s, v, "spacing.decreaseLeftSidebearing", "Decrease Left Sidebearing", Nudge::Left, -1.0, Step::Amount)),
        nudge_spec!("spacing.decreaseLeftSidebearingBy10", "Decrease Left Sidebearing by 10", "Ctrl+Shift+Right", "remove 10 units of space left of each glyph's outline; the width shrinks by as much", false, |s, v| nudge(s, v, "spacing.decreaseLeftSidebearingBy10", "Decrease Left Sidebearing by 10", Nudge::Left, -1.0, Step::Ten)),
        nudge_spec!("spacing.increaseRightSidebearing", "Increase Right Sidebearing", "Cmd+Right", "add `amount` (default 1) units of space right of each glyph's outline by widening it", true, |s, v| nudge(s, v, "spacing.increaseRightSidebearing", "Increase Right Sidebearing", Nudge::Right, 1.0, Step::Amount)),
        nudge_spec!("spacing.increaseRightSidebearingBy10", "Increase Right Sidebearing by 10", "Cmd+Shift+Right", "add 10 units of space right of each glyph's outline by widening it", false, |s, v| nudge(s, v, "spacing.increaseRightSidebearingBy10", "Increase Right Sidebearing by 10", Nudge::Right, 1.0, Step::Ten)),
        nudge_spec!("spacing.decreaseRightSidebearing", "Decrease Right Sidebearing", "Cmd+Left", "remove `amount` (default 1) units of space right of each glyph's outline by narrowing it", true, |s, v| nudge(s, v, "spacing.decreaseRightSidebearing", "Decrease Right Sidebearing", Nudge::Right, -1.0, Step::Amount)),
        nudge_spec!("spacing.decreaseRightSidebearingBy10", "Decrease Right Sidebearing by 10", "Cmd+Shift+Left", "remove 10 units of space right of each glyph's outline by narrowing it", false, |s, v| nudge(s, v, "spacing.decreaseRightSidebearingBy10", "Decrease Right Sidebearing by 10", Nudge::Right, -1.0, Step::Ten)),
        nudge_spec!("spacing.shiftOutlineLeft", "Shift Outline Left", "Cmd+Ctrl+Left", "move each glyph's outline `amount` (default 1) units left inside its width, which stays", true, |s, v| nudge(s, v, "spacing.shiftOutlineLeft", "Shift Outline Left", Nudge::Shift, -1.0, Step::Amount)),
        nudge_spec!("spacing.shiftOutlineLeftBy10", "Shift Outline Left by 10", "Cmd+Ctrl+Shift+Left", "move each glyph's outline 10 units left inside its width, which stays", false, |s, v| nudge(s, v, "spacing.shiftOutlineLeftBy10", "Shift Outline Left by 10", Nudge::Shift, -1.0, Step::Ten)),
        nudge_spec!("spacing.shiftOutlineRight", "Shift Outline Right", "Cmd+Ctrl+Right", "move each glyph's outline `amount` (default 1) units right inside its width, which stays", true, |s, v| nudge(s, v, "spacing.shiftOutlineRight", "Shift Outline Right", Nudge::Shift, 1.0, Step::Amount)),
        cmd!(
            query "spacing.getDerivedMetrics",
            "Get Derived Metrics",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer)} whether automatic alignment sets the layer's width and left and right sidebearings, and the values it gives them: the advances of the base components (marks attached by anchors do not count) plus the offsets of the `=+N` and `=-N` keys, which are listed as a field shows them. Values are null when the layer is not aligned, and a sidebearing is null when the bases draw nothing → {glyph, layer, aligned: bool, width, left, right, offsets: {left: key | null, right: key | null}, error?: why the components cannot be aligned}",
            has_doc,
            get_derived_metrics
        )
        .targets(&[TargetKind::Layer]),
        nudge_spec!("spacing.shiftOutlineRightBy10", "Shift Outline Right by 10", "Cmd+Ctrl+Shift+Right", "move each glyph's outline 10 units right inside its width, which stays", false, |s, v| nudge(s, v, "spacing.shiftOutlineRightBy10", "Shift Outline Right by 10", Nudge::Shift, 1.0, Step::Ten)),
    ]
}

// ── The setters ────────────────────────────────────────────────────────────────────────────────

fn set_left(s: &mut Session, v: &Value) -> Result<Value> {
    set_metric(s, v, "spacing.setLeftSidebearing", "Set Left Sidebearing", Field::Left)
}

fn set_right(s: &mut Session, v: &Value) -> Result<Value> {
    set_metric(s, v, "spacing.setRightSidebearing", "Set Right Sidebearing", Field::Right)
}

fn set_width(s: &mut Session, v: &Value) -> Result<Value> {
    set_metric(s, v, "spacing.setWidth", "Set Width", Field::Width)
}

fn set_top(s: &mut Session, v: &Value) -> Result<Value> {
    set_metric(s, v, "spacing.setTopSidebearing", "Set Top Sidebearing", Field::Top)
}

fn set_bottom(s: &mut Session, v: &Value) -> Result<Value> {
    set_metric(s, v, "spacing.setBottomSidebearing", "Set Bottom Sidebearing", Field::Bottom)
}

fn set_metric(s: &mut Session, v: &Value, id: &'static str, label: &str, field: Field) -> Result<Value> {
    let p = Params::new(id, v)?;
    let font = s.font()?;
    let names = glyph_names(&p, font)?;
    let layer = layer_id(&p, font)?;
    let entry = match p.get("value") {
        Some(Value::String(text)) => spacing::parse_field(text, field, |n| font.glyph(n).is_some()).map_err(|e| p.bad(format!("`value`: {e}")))?,
        Some(_) => Entry::Number(p.f64("value")?),
        None => return Err(p.bad("`value` is required")),
    };
    if let Entry::Number(n) = entry {
        check_value(&p, field, n)?;
    }
    let beam = beam_height(&p, field, &entry)?;
    let key = match &entry {
        Entry::Key(k) => Some(k),
        Entry::Number(_) => None,
    };
    let mut values = Map::new();
    let mut changed = Vec::new();
    let mut skipped = Vec::new();
    s.edit(label, |font| {
        for name in &names {
            // The width field of a glyph without this master's layer gives it one.
            if field == Field::Width && is_master(font, &layer) && font.glyph(name).is_some_and(|g| !g.layers.iter().any(|l| l.id == layer)) {
                font.glyph_mut(name)?.layers.push(Layer::new(&layer, 0.0));
            }
            let (glyph, l) = find_layer(&p, font, name, &layer)?;
            if spacing::derived_field(font, l, field) {
                let value = set_offset(&p, font, name, &layer, field, &entry)?;
                values.insert(name.clone(), json!(value));
                changed.push(name.clone());
                continue;
            }
            let current = match beam {
                Some(y) => {
                    spacing::sidebearings_at_height(font, l, y).map(|at| at.map(|(left, right)| if field == Field::Left { left } else { right }))
                }
                None => spacing::metric(font, l, field),
            };
            let current = current.map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
            let Some(current) = current else {
                skipped.push(name.clone());
                continue;
            };
            let target = match &entry {
                Entry::Number(n) => *n,
                Entry::Key(k) => spacing::evaluate_key(font, name, &layer, field, k).map_err(|e| p.bad(format!("`value` in glyph `{name}`: {e}")))?,
            };
            let vertical = match field {
                Field::Top | Field::Bottom => vertical_width(font, l).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?,
                Field::Left | Field::Right | Field::Width => None,
            };
            let shown = spacing::effective_key(glyph, l, field).map(|(scope, _)| scope);
            let mut gm = font.glyph_mut(name)?;
            let g: &mut Glyph = &mut gm;
            let l = g.layers.iter_mut().find(|l| l.id == layer).ok_or_else(|| p.bad(format!("glyph `{name}` has no layer `{layer}`")))?;
            apply(l, field, target - current, vertical).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
            match key {
                // A number replaces the key the field shows, wherever it is stored.
                None => match shown {
                    Some(Scope::Local) => *slot(&mut l.info.metrics_keys, field) = None,
                    Some(Scope::Glyph) => *slot(&mut g.info.metrics_keys, field) = None,
                    None => {}
                },
                Some(k) => match k.scope {
                    Scope::Local => *slot(&mut l.info.metrics_keys, field) = Some(k.stored()),
                    Scope::Glyph => {
                        // The layer's own key would hide the new one on this layer.
                        if shown == Some(Scope::Local) {
                            *slot(&mut l.info.metrics_keys, field) = None;
                        }
                        *slot(&mut g.info.metrics_keys, field) = Some(k.stored());
                    }
                },
            }
            values.insert(name.clone(), json!(target));
            changed.push(name.clone());
        }
        Ok(())
    })?;
    let mut out = json!({ "glyphs": changed, "layer": layer, "values": values, "skipped": skipped });
    if let (Some(k), Some(o)) = (key, out.as_object_mut()) {
        o.insert("key".into(), json!(k.display()));
    }
    Ok(out)
}

/// Sets `field` of a layer whose metrics automatic alignment derives: only an offset key
/// (`=+N`, `=-N`) on a sidebearing is taken, stored where it was typed for; alignment applies it
/// when the command ends (`Session::realign_after`). A glyph-wide `=+0` or `=-0` removes the
/// field's offset. Anything else is refused, naming the glyph. Returns the derived value with the
/// offset, or `None` when the components cannot be aligned.
fn set_offset(p: &Params, font: &mut Font, name: &str, layer: &str, field: Field, entry: &Entry) -> Result<Option<f64>> {
    let what = match field {
        Field::Left => "left sidebearing",
        Field::Right => "right sidebearing",
        Field::Width | Field::Top | Field::Bottom => "width",
    };
    let key = match entry {
        Entry::Key(k) if spacing::is_offset(k, field) => k,
        _ if field == Field::Width => {
            return Err(p.bad(format!(
                "glyph `{name}` is aligned automatically: its width comes from its components; add space with =+N or =-N in a sidebearing field, or disable automatic alignment of its components"
            )));
        }
        _ => {
            return Err(p.bad(format!(
                "glyph `{name}` is aligned automatically: its {what} comes from its components; type =+N or =-N to add to it or take from it, or disable automatic alignment of its components"
            )));
        }
    };
    let zero = spacing::aligned::offset_amount(key, field) == Some(0.0);
    {
        let mut gm = font.glyph_mut(name)?;
        let g: &mut Glyph = &mut gm;
        let l = g.layers.iter_mut().find(|l| l.id == layer).ok_or_else(|| p.bad(format!("glyph `{name}` has no layer `{layer}`")))?;
        match key.scope {
            Scope::Local => *slot(&mut l.info.metrics_keys, field) = Some(key.stored()),
            Scope::Glyph => {
                // The layer's own key would hide the glyph's on this layer.
                if slot(&mut l.info.metrics_keys, field).is_some() {
                    *slot(&mut l.info.metrics_keys, field) = None;
                }
                *slot(&mut g.info.metrics_keys, field) = if zero { None } else { Some(key.stored()) };
            }
        }
    }
    let derived = fontcraft_components::metrics::derived_metrics(font, name, layer).ok().flatten();
    Ok(derived.and_then(|d| match field {
        Field::Left => d.lsb,
        Field::Right => d.rsb,
        Field::Width | Field::Top | Field::Bottom => Some(d.width),
    }))
}

/// `beam`: the height a left or right sidebearing given as a number is measured at (the metrics
/// beam) instead of the outline's bounds. A layer whose outline does not reach it is skipped.
fn beam_height(p: &Params, field: Field, entry: &Entry) -> Result<Option<f64>> {
    let Some(y) = p.opt_f64("beam")? else { return Ok(None) };
    if !matches!(field, Field::Left | Field::Right) {
        return Err(p.bad("`beam` applies to the left and right sidebearings"));
    }
    if matches!(entry, Entry::Key(_)) {
        return Err(p.bad("`beam` takes a number `value`; a metrics key measures at the bounds"));
    }
    if !in_range(y) {
        return Err(p.bad(format!("`beam` must be a height within ±{MAX_COORDINATE}, got {y}")));
    }
    Ok(Some(y))
}

/// A typed number must lie in the field's range.
fn check_value(p: &Params, field: Field, n: f64) -> Result<()> {
    let range = if field == Field::Width { WIDTH_RANGE } else { SIDEBEARING_RANGE };
    if range.contains(&n) { Ok(()) } else { Err(p.bad(format!("`value` must be between {} and {}, got {n}", range.start(), range.end()))) }
}

/// The layer's vertical width as its sidebearings measure it: top, bottom and the outline's
/// height. `None` for an empty layer.
fn vertical_width(font: &Font, layer: &Layer) -> std::result::Result<Option<f64>, spacing::EvalError> {
    let Some(b) = spacing::measure::bounds(font, layer)? else { return Ok(None) };
    let top = spacing::metric(font, layer, Field::Top)?;
    let bottom = spacing::metric(font, layer, Field::Bottom)?;
    Ok(top.zip(bottom).map(|(t, bt)| t + bt + (b.y1 - b.y0)))
}

/// Changes one metric of `layer` by `delta`. A left sidebearing moves the contents and the width
/// with them; a right one moves the width; a top one moves the vertical origin up and lengthens
/// the vertical width; a bottom one lengthens the vertical width. `vertical` is the current
/// vertical width (needed for top and bottom).
fn apply(layer: &mut Layer, field: Field, delta: f64, vertical: Option<f64>) -> std::result::Result<(), String> {
    if delta == 0.0 {
        return Ok(());
    }
    match field {
        Field::Left => {
            shift_layer(layer, delta)?;
            set_layer_width(layer, layer.width + delta)
        }
        Field::Right | Field::Width => set_layer_width(layer, layer.width + delta),
        Field::Top | Field::Bottom => {
            let vw = vertical.ok_or("the layer is empty, so it has no vertical sidebearings")? + delta;
            if !WIDTH_RANGE.contains(&vw) {
                return Err(format!("the vertical width would be {vw}, outside {} to {}", WIDTH_RANGE.start(), WIDTH_RANGE.end()));
            }
            if field == Field::Top {
                let origin = layer.info.vert_origin.unwrap_or(0.0) - delta;
                if !in_range(origin) {
                    return Err(format!("the vertical origin would be {origin}, beyond ±{MAX_COORDINATE}"));
                }
                layer.info.vert_origin = Some(origin);
            }
            layer.info.vert_width = Some(vw);
            Ok(())
        }
    }
}

fn set_layer_width(layer: &mut Layer, width: f64) -> std::result::Result<(), String> {
    if !WIDTH_RANGE.contains(&width) {
        return Err(format!("the width would be {width}, outside {} to {}", WIDTH_RANGE.start(), WIDTH_RANGE.end()));
    }
    layer.width = width;
    Ok(())
}

/// The stored key of one field.
fn slot(keys: &mut MetricsKeys, field: Field) -> &mut Option<String> {
    match field {
        Field::Left => &mut keys.left,
        Field::Right => &mut keys.right,
        Field::Width => &mut keys.width,
        Field::Top => &mut keys.top,
        Field::Bottom => &mut keys.bottom,
    }
}

// ── Derived metrics ────────────────────────────────────────────────────────────────────────────

fn get_derived_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.getDerivedMetrics", v)?;
    let font = s.font()?;
    let name = p.str("glyph")?;
    let g = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
    let id = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let l = g.layers.iter().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{name}` has no layer `{id}`")))?;
    let mut out = json!({ "glyph": name, "layer": id, "aligned": false, "width": null, "left": null, "right": null, "offsets": { "left": null, "right": null } });
    if !spacing::derives_metrics(font, l) {
        return Ok(out);
    }
    let derived = match fontcraft_components::metrics::derived_metrics(font, name, &id) {
        Ok(Some(d)) => d,
        Ok(None) => return Ok(out),
        Err(e) => {
            if let Some(o) = out.as_object_mut() {
                o.insert("error".into(), json!(e.to_string()));
            }
            return Ok(out);
        }
    };
    let offset = |field: Field| -> Value {
        spacing::effective_key(g, l, field)
            .and_then(|(scope, text)| spacing::parse_stored(text, scope, field, |n| font.glyph(n).is_some()).ok())
            .filter(|k| spacing::is_offset(k, field))
            .map_or(Value::Null, |k| json!(k.display()))
    };
    out = json!({
        "glyph": name, "layer": id, "aligned": true, "width": derived.width, "left": derived.lsb, "right": derived.rsb,
        "offsets": { "left": offset(Field::Left), "right": offset(Field::Right) },
    });
    Ok(out)
}

// ── The shortcuts ──────────────────────────────────────────────────────────────────────────────

/// Which edge a shortcut moves.
#[derive(Clone, Copy)]
enum Nudge {
    /// The left edge of the outline: contents and width move together.
    Left,
    /// The right edge of the advance: the width.
    Right,
    /// The contents inside an unchanged width.
    Shift,
}

#[derive(Clone, Copy)]
enum Step {
    /// `amount`, 1 by default.
    Amount,
    /// Always 10.
    Ten,
}

fn nudge(s: &mut Session, v: &Value, id: &'static str, label: &str, edge: Nudge, sign: f64, step: Step) -> Result<Value> {
    let p = Params::new(id, v)?;
    let font = s.font()?;
    let names = glyph_names(&p, font)?;
    let layer = layer_id(&p, font)?;
    let amount = match step {
        Step::Ten => 10.0,
        Step::Amount => p.opt_f64("amount")?.unwrap_or(1.0),
    };
    if amount.abs() > MAX_AMOUNT {
        return Err(p.bad(format!("`amount` must be between -{MAX_AMOUNT} and {MAX_AMOUNT}, got {amount}")));
    }
    let d = sign * amount;
    let mut widths = Map::new();
    let mut moved_glyphs = Vec::new();
    let mut skipped = Vec::new();
    s.edit(label, |font| {
        for name in &names {
            // Alignment would put the components and the width back at once.
            if font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == layer)).is_some_and(|l| spacing::derives_metrics(font, l)) {
                skipped.push(name.clone());
                continue;
            }
            let mut g = font.glyph_mut(name)?;
            let l = g.layers.iter_mut().find(|l| l.id == layer).ok_or_else(|| p.bad(format!("glyph `{name}` has no layer `{layer}`")))?;
            let moved = match edge {
                Nudge::Left => shift_layer(l, d).and_then(|()| set_layer_width(l, l.width + d)),
                Nudge::Right => set_layer_width(l, l.width + d),
                Nudge::Shift => shift_layer(l, d),
            };
            moved.map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
            widths.insert(name.clone(), json!(l.width));
            moved_glyphs.push(name.clone());
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": moved_glyphs, "layer": layer, "widths": widths, "skipped": skipped }))
}

// ── Shared ─────────────────────────────────────────────────────────────────────────────────────

/// `glyphs`: required, not empty, every name in the font.
fn glyph_names(p: &Params, font: &Font) -> Result<Vec<String>> {
    let names = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    Ok(names)
}

fn is_master(font: &Font, id: &str) -> bool {
    font.masters.iter().any(|m| m.id == id)
}

/// `layer`: a layer id, the first master's by default. A master that shows another master's
/// metrics is refused, naming that master.
fn layer_id(p: &Params, font: &Font) -> Result<String> {
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    if let Some(source) = spacing::metrics_source(font, &layer)
        && source.id != layer
    {
        let name = font.masters.iter().find(|m| m.id == layer).map_or(layer.as_str(), |m| m.name.as_str());
        return Err(p.bad(format!("`layer`: master `{name}` takes its widths from master `{}`; edit that master instead", source.name)));
    }
    Ok(layer)
}

/// Glyph `name` and its layer `layer`.
fn find_layer<'f>(p: &Params, font: &'f Font, name: &str, layer: &str) -> Result<(&'f Glyph, &'f Layer)> {
    font.glyph(name)
        .and_then(|g| g.layers.iter().find(|l| l.id == layer).map(|l| (g.as_ref(), l)))
        .ok_or_else(|| p.bad(format!("glyph `{name}` has no layer `{layer}`")))
}

/// Moves everything drawn in a layer sideways by `dx`: outlines, components, images, anchors,
/// guides, annotations, the placed image and the background. Hints stay as they are. Fails when a
/// coordinate would leave the model's range or a shape cannot be moved, and then leaves the
/// layer as it was.
pub(crate) fn shift_layer(layer: &mut Layer, dx: f64) -> std::result::Result<(), String> {
    if dx == 0.0 {
        return Ok(());
    }
    let mut moved = layer.clone();
    shift_in_place(&mut moved, dx)?;
    *layer = moved;
    Ok(())
}

fn shift_in_place(layer: &mut Layer, dx: f64) -> std::result::Result<(), String> {
    shift_shapes(&mut layer.shapes, dx)?;
    for a in &mut layer.anchors {
        shift_x(&mut a.x, dx)?;
    }
    if layer.info.is_default() {
        return Ok(());
    }
    let info = &mut *layer.info;
    for g in &mut info.guides {
        shift_x(&mut g.x, dx)?;
    }
    for a in &mut info.annotations {
        shift_x(&mut a.x, dx)?;
    }
    if let Some(img) = &mut info.image {
        shift_transform(&mut img.transform, dx)?;
    }
    if let Some(b) = &mut info.background {
        shift_shapes(&mut b.shapes, dx)?;
        for a in &mut b.anchors {
            shift_x(&mut a.x, dx)?;
        }
        for g in &mut b.guides {
            shift_x(&mut g.x, dx)?;
        }
        for a in &mut b.annotations {
            shift_x(&mut a.x, dx)?;
        }
        if let Some(img) = &mut b.image {
            shift_transform(&mut img.transform, dx)?;
        }
    }
    Ok(())
}

fn shift_shapes(shapes: &mut [Shape], dx: f64) -> std::result::Result<(), String> {
    for shape in shapes {
        match shape {
            Shape::Path(path) => {
                for n in &mut path.nodes {
                    shift_x(&mut n.x, dx)?;
                }
            }
            Shape::Component(c) => shift_transform(&mut c.transform, dx)?,
            Shape::Image(img) => shift_transform(&mut img.transform, dx)?,
            // Its points are not in the model, so it cannot follow the rest.
            Shape::Raw(_) => return Err("the layer holds a shape of a kind that cannot be moved yet".into()),
        }
    }
    Ok(())
}

fn shift_transform(t: &mut Transform, dx: f64) -> std::result::Result<(), String> {
    match t {
        Transform::Decomposed { pos, .. } => shift_x(&mut pos.0, dx),
        Transform::Matrix([.., x, _]) => shift_x(x, dx),
    }
}

fn shift_x(x: &mut f64, dx: f64) -> std::result::Result<(), String> {
    let v = *x + dx;
    if !in_range(v) {
        return Err(format!("the change puts a point at x = {v}, beyond ±{MAX_COORDINATE}"));
    }
    *x = v;
    Ok(())
}
