//! Anchor position expressions (M13.16), and the engine hook that keeps automatic layers,
//! anchor expressions and automatic alignment up to date after each font-changing command.
//!
//! Rows: `anchor.positionExpressions`, `anchor.removeExpressions`. Plan: `docs/plans/M13.md`
//! § 4 › M13.16. Grammar: [`fontcraft_geom::expr`].
//!
//! Contract:
//! - An anchor's expressions are `{"x"?: text, "y"?: text}` in its user data under
//!   [`fontcraft_model::fc_keys::ANCHOR_EXPRESSIONS`] ([`anchor_expressions`],
//!   [`set_anchor_expressions`]).
//! - Names an expression may use, for the anchor's layer ([`lookup`]):
//!
//! | Name | Value |
//! |---|---|
//! | `width` | the layer's advance width |
//! | `ascender`, `descender`, `baseline` | the master's metric |
//! | `xHeight` (also `x_height`, `xheight`) | the master's x-height |
//! | `capHeight` (also `caps_height`, `cap_height`, `capsheight`) | the master's cap height |
//! | `anchor:<name>.x`, `anchor:<name>.y` | another anchor of the layer, where it is at that moment |
//! | `guide:<name>` | a named guide of the layer, else of the master: y of a horizontal guide, x of a vertical one |
//! | `guide:<name>.x`, `guide:<name>.y` | the guide's position |
//! | `zone:<name>` | the master's metric with that name, else of that kind (`x-height`) |
//! | `param:<name>` | the master's number value with that name, else the font's numeric custom parameter |
//!
//! - A result is rounded to the font's grid. A result that cannot be worked out (a missing name,
//!   division by zero, a value that is not finite or beyond the model's coordinate range) leaves
//!   that coordinate as it is and is reported as an [`ExprFailure`].
//! - Anchors are worked out in layer order, so `anchor:` sees earlier anchors' new positions.
//!   Locked anchors and locked glyphs are left alone.
//! - [`follow_changes`] is the engine hook: automatic layers ([`crate::auto_layer`]), then the
//!   expressions of the glyphs that changed (every glyph when the masters, metric definitions,
//!   numbers or font settings changed), then automatic alignment ([`crate::metrics::realign`]),
//!   then once more the expressions of the glyphs alignment changed and the alignment that
//!   follows from them. Failures there are not reported; the commands that set expressions
//!   report them.

use std::sync::Arc;

use fontcraft_geom::expr::Expr;
use fontcraft_geom::{grid_step, round_to_grid};
use fontcraft_model::fc_keys::ANCHOR_EXPRESSIONS;
use fontcraft_model::plist::{Dict, Str, Value};
use fontcraft_model::{Anchor, Font, Glyph, Layer, MAX_COORDINATE, Master, Metrics, metric};

use crate::ComponentsError;

/// An anchor's position expressions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnchorExpressions {
    pub x: Option<String>,
    pub y: Option<String>,
}

impl AnchorExpressions {
    pub fn is_empty(&self) -> bool {
        self.x.is_none() && self.y.is_none()
    }
}

/// An expression that could not be worked out.
#[derive(Clone, Debug, PartialEq)]
pub struct ExprFailure {
    pub glyph: String,
    pub layer: String,
    pub anchor: String,
    /// `"x"` or `"y"`.
    pub axis: &'static str,
    pub expression: String,
    pub message: String,
}

/// The expressions stored on `anchor`, if any.
pub fn anchor_expressions(anchor: &Anchor) -> Option<AnchorExpressions> {
    let Value::Dict(d) = anchor.info.user_data.get(ANCHOR_EXPRESSIONS)? else { return None };
    let text = |k: &str| d.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string);
    let e = AnchorExpressions { x: text("x"), y: text("y") };
    (!e.is_empty()).then_some(e)
}

/// Stores `e` on `anchor`; an empty `e` removes the key. Whether anything changed.
pub fn set_anchor_expressions(anchor: &mut Anchor, e: &AnchorExpressions) -> bool {
    let stored = anchor.info.user_data.get(ANCHOR_EXPRESSIONS).is_some();
    if anchor_expressions(anchor).unwrap_or_default() == *e && stored != e.is_empty() {
        return false;
    }
    let data = &mut anchor.info.user_data;
    if e.is_empty() {
        data.0.entries.retain(|(k, _)| k.as_str() != ANCHOR_EXPRESSIONS);
        return true;
    }
    let mut d = Dict::new();
    if let Some(x) = &e.x {
        d.insert("x", Value::String(Str::new(x.as_str())));
    }
    if let Some(y) = &e.y {
        d.insert("y", Value::String(Str::new(y.as_str())));
    }
    data.0.insert(ANCHOR_EXPRESSIONS, Value::Dict(d));
    true
}

/// Whether any anchor of `glyph` has expressions.
pub fn has_expressions(glyph: &Glyph) -> bool {
    glyph.layers.iter().flat_map(|l| l.anchors.iter()).any(|a| a.info.user_data.get(ANCHOR_EXPRESSIONS).is_some())
}

/// The value of `name` for an anchor expression on `layer`, whose anchors are `anchors` (the
/// layer's, possibly already moved by earlier expressions). `None` when the name means nothing
/// there.
pub fn lookup(font: &Font, layer: &Layer, anchors: &[Anchor], name: &str) -> Option<f64> {
    let master = font.master(&layer.master).ok();
    let defaults = Metrics::defaults(font.source.format, font.units_per_em);
    let m = |kind: &str, default: f64| master.map(|master| font.metric_pos(master, kind, default));
    match name {
        "width" => return Some(layer.width),
        "ascender" => return m(metric::ASCENDER, defaults.ascender),
        "descender" => return m(metric::DESCENDER, defaults.descender),
        "baseline" => return m(metric::BASELINE, 0.0),
        "xHeight" | "x_height" | "xheight" => return m(metric::X_HEIGHT, defaults.x_height),
        "capHeight" | "caps_height" | "cap_height" | "capsheight" => return m(metric::CAP_HEIGHT, defaults.cap_height),
        _ => {}
    }
    if let Some(rest) = name.strip_prefix("anchor:") {
        let (anchor, axis) = split_axis(rest)?;
        let a = anchors.iter().find(|a| a.name == anchor)?;
        return Some(if axis == 'x' { a.x } else { a.y });
    }
    if let Some(rest) = name.strip_prefix("guide:") {
        return guide_value(layer, master, rest);
    }
    if let Some(rest) = name.strip_prefix("zone:") {
        let master = master?;
        let i = font
            .metrics
            .iter()
            .position(|d| d.name.as_deref() == Some(rest))
            .or_else(|| font.metrics.iter().position(|d| d.kind == rest && d.name.is_none()))?;
        return Some(master.metric_values.get(i).map_or(0.0, |v| v.pos));
    }
    if let Some(rest) = name.strip_prefix("param:") {
        if let (Some(master), Some(i)) = (master, font.numbers.iter().position(|n| n.name == rest)) {
            return Some(master.number_values.get(i).copied().unwrap_or(0.0));
        }
        return match font.info.params.get(rest)? {
            Value::Number(n) => Some(n.value().as_f64()),
            Value::String(s) => s.as_str().trim().parse().ok(),
            _ => None,
        };
    }
    None
}

/// `top.x` → (`top`, 'x').
fn split_axis(text: &str) -> Option<(&str, char)> {
    if let Some(name) = text.strip_suffix(".x") { Some((name, 'x')) } else { text.strip_suffix(".y").map(|name| (name, 'y')) }
}

fn guide_value(layer: &Layer, master: Option<&Master>, text: &str) -> Option<f64> {
    let find = |name: &str| {
        layer.info.guides.iter().chain(master.into_iter().flat_map(|m| m.guides.iter())).find(|g| g.name.as_deref() == Some(name)).cloned()
    };
    if let Some(g) = find(text) {
        let angle = g.angle.rem_euclid(180.0);
        return if angle.abs() < 1e-9 {
            Some(g.y)
        } else if (angle - 90.0).abs() < 1e-9 {
            Some(g.x)
        } else {
            None
        };
    }
    let (name, axis) = split_axis(text)?;
    let g = find(name)?;
    Some(if axis == 'x' { g.x } else { g.y })
}

/// The anchors of `layer` with their expressions worked out, or `None` when nothing moves; the
/// failures. `glyph` names the glyph in the failures.
pub fn evaluate_layer(font: &Font, glyph: &str, layer: &Layer) -> (Option<Vec<Anchor>>, Vec<ExprFailure>) {
    let mut anchors = layer.anchors.clone();
    let mut failures = Vec::new();
    let mut moved = false;
    let step = grid_step(font.info.settings.grid_length.unwrap_or(1.0), font.info.settings.grid_sub_division.unwrap_or(1.0));
    for i in 0..anchors.len() {
        let Some(a) = anchors.get(i) else { break };
        if a.info.locked {
            continue;
        }
        let Some(e) = anchor_expressions(a) else { continue };
        let name = a.name.clone();
        let mut position = (a.x, a.y);
        for (axis, text) in [("x", &e.x), ("y", &e.y)] {
            let Some(text) = text else { continue };
            let value =
                Expr::parse(text).and_then(|expr| expr.eval(|n| lookup(font, layer, &anchors, n))).map_err(|err| err.to_string()).and_then(|v| {
                    let v = round_to_grid(v, step);
                    if v.is_finite() && v.abs() <= MAX_COORDINATE { Ok(v) } else { Err(format!("the result {v} is beyond ±{MAX_COORDINATE}")) }
                });
            match value {
                Ok(v) if axis == "x" => position.0 = v,
                Ok(v) => position.1 = v,
                Err(message) => failures.push(ExprFailure {
                    glyph: glyph.to_string(),
                    layer: layer.id.clone(),
                    anchor: name.clone(),
                    axis,
                    expression: text.clone(),
                    message,
                }),
            }
        }
        if let Some(a) = anchors.get_mut(i)
            && ((a.x - position.0).abs() > 1e-9 || (a.y - position.1).abs() > 1e-9)
        {
            (a.x, a.y) = position;
            moved = true;
        }
    }
    (moved.then_some(anchors), failures)
}

/// `font` with the anchor expressions of `glyphs` worked out on every layer; the failures.
/// Pointer-equal glyphs where nothing moves.
pub fn evaluate_glyphs(font: Font, glyphs: &[String]) -> (Font, Vec<ExprFailure>) {
    let (font, failures, _) = evaluate_names(font, glyphs);
    (font, failures)
}

/// [`evaluate_glyphs`], and whether any anchor moved.
fn evaluate_names(mut font: Font, glyphs: &[String]) -> (Font, Vec<ExprFailure>, bool) {
    let mut failures = Vec::new();
    let mut moved_any = false;
    for name in glyphs {
        let Some(g) = font.glyph(name) else { continue };
        if g.info.locked || !has_expressions(g) {
            continue;
        }
        let mut updates: Vec<(usize, Vec<Anchor>)> = Vec::new();
        for (i, layer) in g.layers.iter().enumerate() {
            let (moved, failed) = evaluate_layer(&font, name, layer);
            failures.extend(failed);
            if let Some(anchors) = moved {
                updates.push((i, anchors));
            }
        }
        if updates.is_empty() {
            continue;
        }
        if let Ok(mut g) = font.glyph_mut(name) {
            for (i, anchors) in updates {
                if let Some(layer) = g.layers.get_mut(i) {
                    layer.anchors = anchors;
                    moved_any = true;
                }
            }
        }
    }
    (font, failures, moved_any)
}

/// Whether something every expression may read changed between `a` and `b`.
fn inputs_changed(a: &Font, b: &Font) -> bool {
    (!Arc::ptr_eq(&a.masters, &b.masters) && a.masters != b.masters)
        || (!Arc::ptr_eq(&a.metrics, &b.metrics) && a.metrics != b.metrics)
        || (!Arc::ptr_eq(&a.numbers, &b.numbers) && a.numbers != b.numbers)
        || (!Arc::ptr_eq(&a.info, &b.info) && a.info != b.info)
        || a.units_per_em != b.units_per_em
}

/// `after` with the expressions of the glyphs that changed since `before` worked out; whether
/// any anchor moved.
fn evaluate_changed(before: &Font, after: Font) -> (Font, bool) {
    let names: Vec<String> = if inputs_changed(before, &after) {
        after.glyphs.iter().filter(|g| has_expressions(g)).map(|g| g.name.clone()).collect()
    } else {
        crate::metrics::changed_glyphs(before, &after).into_iter().filter(|n| after.glyph(n).is_some_and(|g| has_expressions(g))).collect()
    };
    if names.is_empty() {
        return (after, false);
    }
    let (font, _, moved) = evaluate_names(after, &names);
    (font, moved)
}

/// The engine hook; see the module doc. Returns `new` (pointer-equal glyphs) when nothing
/// follows; fails only as [`crate::metrics::realign`] fails.
pub fn follow_changes(old: &Font, new: Font) -> Result<Font, ComponentsError> {
    let built = crate::auto_layer::rebuild_auto_layers(old, new);
    let (evaluated, _) = evaluate_changed(old, built);
    let aligned = crate::metrics::realign(old, evaluated.clone())?;
    let (again, moved) = evaluate_changed(&evaluated, aligned.clone());
    if !moved {
        return Ok(again);
    }
    crate::metrics::realign(&aligned, again)
}
