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
//!   report them. The glyphs that changed are found once per call. A font where no glyph has
//!   anchor expressions or an automatic layer goes straight to alignment: whether a font has
//!   any is remembered for the last few fonts [`follow_changes`] returned, and worked out for a
//!   new one from that and the glyphs that changed.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_geom::expr::Expr;
use fontcraft_geom::{grid_step, round_to_grid};
use fontcraft_model::fc_keys::ANCHOR_EXPRESSIONS;
use fontcraft_model::guide_follow::placed;
use fontcraft_model::plist::{Dict, Str, Value};
use fontcraft_model::{Anchor, Font, Glyph, GlyphTable, Layer, MAX_COORDINATE, Master, Metrics, metric};

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
        let found = layer.info.guides.iter().chain(master.into_iter().flat_map(|m| m.guides.iter())).find(|g| g.name.as_deref() == Some(name));
        found.map(|g| placed(g, layer.width))
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

/// [`evaluate_glyphs`], and the glyphs whose anchors moved.
fn evaluate_names(mut font: Font, glyphs: &[String]) -> (Font, Vec<ExprFailure>, Vec<String>) {
    let mut failures = Vec::new();
    let mut moved_names = Vec::new();
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
            let mut moved = false;
            for (i, anchors) in updates {
                if let Some(layer) = g.layers.get_mut(i) {
                    layer.anchors = anchors;
                    moved = true;
                }
            }
            if moved {
                moved_names.push(name.clone());
            }
        }
    }
    (font, failures, moved_names)
}

/// Whether something every expression may read changed between `a` and `b`.
fn inputs_changed(a: &Font, b: &Font) -> bool {
    (!Arc::ptr_eq(&a.masters, &b.masters) && a.masters != b.masters)
        || (!Arc::ptr_eq(&a.metrics, &b.metrics) && a.metrics != b.metrics)
        || (!Arc::ptr_eq(&a.numbers, &b.numbers) && a.numbers != b.numbers)
        || (!Arc::ptr_eq(&a.info, &b.info) && a.info != b.info)
        || a.units_per_em != b.units_per_em
}

/// `after` with the expressions of `changed` (the glyphs that changed since `before`) worked
/// out; the glyphs whose anchors moved.
fn evaluate_changed(before: &Font, after: Font, changed: &[String]) -> (Font, Vec<String>) {
    #[cfg(test)]
    EXPRESSION_PASSES.with(|n| n.set(n.get() + 1));
    let names: Vec<String> = if inputs_changed(before, &after) {
        after.glyphs.iter().filter(|g| has_expressions(g)).map(|g| g.name.clone()).collect()
    } else {
        changed.iter().filter(|n| after.glyph(n).is_some_and(|g| has_expressions(g))).cloned().collect()
    };
    if names.is_empty() {
        return (after, Vec::new());
    }
    let (font, _, moved) = evaluate_names(after, &names);
    (font, moved)
}

#[cfg(test)]
thread_local! {
    /// How many times the expression pass ran on this thread.
    pub(crate) static EXPRESSION_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Fonts [`follow_changes`] remembers whether they have followers.
const REMEMBERED_FONTS: usize = 2;

thread_local! {
    /// The glyph tables of the fonts [`follow_changes`] returned last, newest first, each with
    /// whether a glyph in it has anchor expressions or an automatic layer. Holding a table keeps
    /// its storage alive and unchanged (every change to a shared table copies it first), so a
    /// table at the same address with the same length holds the same glyphs.
    static FOLLOWERS: RefCell<Vec<(GlyphTable, bool)>> = const { RefCell::new(Vec::new()) };
}

/// Whether `glyph` has something [`follow_changes`] keeps up to date before alignment.
fn has_followers(glyph: &Glyph) -> bool {
    has_expressions(glyph) || crate::auto_layer::has_auto_layer(glyph)
}

fn same_table(a: &GlyphTable, b: &GlyphTable) -> bool {
    std::ptr::eq(a.as_arcs(), b.as_arcs())
}

/// Whether a glyph of `font` has followers: remembered, else looked up glyph by glyph.
fn font_has_followers(font: &Font) -> bool {
    let known = FOLLOWERS.with(|c| c.try_borrow().ok().and_then(|c| c.iter().find(|(t, _)| same_table(t, &font.glyphs)).map(|(_, f)| *f)));
    known.unwrap_or_else(|| font.glyphs.iter().any(|g| has_followers(g)))
}

fn remember_followers(font: &Font, followers: bool) {
    FOLLOWERS.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut() {
            c.retain(|(t, _)| !same_table(t, &font.glyphs));
            c.insert(0, (font.glyphs.clone(), followers));
            c.truncate(REMEMBERED_FONTS);
        }
    });
}

/// Whether `a` and `b` hold the same glyph named `name` (or neither has one).
fn same_glyph(a: &Font, b: &Font, name: &str) -> bool {
    match (a.glyph(name), b.glyph(name)) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        (None, None) => true,
        _ => false,
    }
}

/// The engine hook; see the module doc. Returns `new` (pointer-equal glyphs) when nothing
/// follows; fails only as [`crate::metrics::realign`] fails.
pub fn follow_changes(old: &Font, new: Font) -> Result<Font, ComponentsError> {
    let changed = crate::metrics::changed_glyphs(old, &new);
    // A glyph has followers in `new` only if it had them in `old` or changed: the flag can
    // stay set after the last one goes, never unset while one is there.
    let followers = font_has_followers(old) || changed.iter().any(|n| new.glyph(n).is_some_and(|g| has_followers(g)));
    let out = if followers { follow(old, new, changed)? } else { crate::metrics::realign_known(old, new, &changed)? };
    remember_followers(&out, followers);
    Ok(out)
}

/// [`follow_changes`] for a font with followers: automatic layers, expressions, alignment, then
/// the expressions and alignment that follow from alignment.
fn follow(old: &Font, new: Font, mut changed: Vec<String>) -> Result<Font, ComponentsError> {
    let (built, rebuilt) = crate::auto_layer::rebuild_auto_layers_for(old, new, &changed);
    let mut seen: HashSet<String> = changed.iter().cloned().collect();
    let extra: Vec<String> = rebuilt.into_iter().filter(|n| seen.insert(n.clone())).collect();
    changed.extend(extra);
    let (evaluated, moved) = evaluate_changed(old, built, &changed);
    let extra: Vec<String> = moved.into_iter().filter(|n| seen.insert(n.clone())).collect();
    changed.extend(extra);
    // The glyphs that differ from `old`, in font order (removed ones last), as alignment
    // expects them.
    changed.retain(|n| !same_glyph(old, &evaluated, n));
    changed.sort_by_key(|n| evaluated.glyphs.position(n).unwrap_or(usize::MAX));
    let aligned = crate::metrics::realign_known(old, evaluated.clone(), &changed)?;
    let realigned = crate::metrics::changed_glyphs(&evaluated, &aligned);
    let (again, moved) = evaluate_changed(&evaluated, aligned.clone(), &realigned);
    if moved.is_empty() {
        return Ok(again);
    }
    crate::metrics::realign(&aligned, again)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn font_of(n: usize) -> Font {
        let mut f = Font::new("Test");
        for i in 0..n {
            let mut g = Glyph::new(format!("g{i}"));
            let mut l = Layer::new("m01", 500.0);
            l.anchors.push(Anchor::new("top", 0.0, 0.0));
            g.layers.push(l);
            f.glyphs.push(g);
        }
        f
    }

    #[test]
    fn a_large_font_without_expressions_takes_no_expression_pass() {
        let old = font_of(50_000);
        let mut new = old.clone();
        new.glyph_mut("g7").unwrap().layers[0].width = 600.0;
        let passes = EXPRESSION_PASSES.with(std::cell::Cell::get);
        let out = follow_changes(&old, new).unwrap();
        assert_eq!(EXPRESSION_PASSES.with(std::cell::Cell::get), passes);
        // The next edit starts from the remembered font.
        let mut next = out.clone();
        next.glyph_mut("g8").unwrap().layers[0].width = 600.0;
        let out = follow_changes(&out, next).unwrap();
        assert_eq!(EXPRESSION_PASSES.with(std::cell::Cell::get), passes);
        // An expression added to a changed glyph is worked out at once.
        let mut with = out.clone();
        {
            let mut g = with.glyph_mut("g9").unwrap();
            let a = &mut g.layers[0].anchors[0];
            set_anchor_expressions(a, &AnchorExpressions { x: Some("width / 2".into()), y: None });
        }
        let out = follow_changes(&out, with).unwrap();
        assert!(EXPRESSION_PASSES.with(std::cell::Cell::get) > passes);
        assert_eq!(out.glyph("g9").unwrap().layers[0].anchors[0].x, 250.0);
    }
}
