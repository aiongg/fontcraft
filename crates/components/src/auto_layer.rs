//! Automatic layers (M13.16): a layer built from a recipe stored on it, rebuilt whenever the
//! glyphs the recipe names appear or go away.
//!
//! Rows: `glyph.setAutoLayerRecipe`, `recipe.directives`. Plan: `docs/plans/M13.md` § 4 › M13.16.
//!
//! Contract:
//! - The recipe is a string in the layer's user data under
//!   [`fontcraft_model::fc_keys::AUTO_LAYER_RECIPE`]. A layer that holds one is automatic.
//! - Recipe text ([`parse_auto_recipe`]): one line of component names joined by `+`
//!   (`a+acutecomb`, spaces around `+` allowed), optionally followed by `=` and the glyph's own
//!   name. Directives may follow `#` on any line: `#!` copies the rest of the line into the glyph
//!   note, `#?noanchors` stops the glyph from inheriting anchors from its components, and any
//!   other `#` starts a comment.
//! - [`rebuild_auto_layers`] runs after every font-changing command, before re-alignment. An
//!   automatic layer whose components all exist holds exactly one component per recipe part, in
//!   order; until every part exists it holds nothing (pending), and it fills in once the missing
//!   glyph is added. Positions and width come from automatic alignment, which runs next.
//! - A layer is only rewritten when its shapes differ from what the recipe gives, so a layer
//!   that is already built keeps its components (and the positions alignment gave them).
//! - Locked glyphs, recipes that cannot be read and recipes whose parts lead back to the glyph
//!   (a cycle) are left as they are.
//! - [`stops_anchor_inheritance`] is read by anchor propagation ([`crate::propagate`]).

use std::collections::HashSet;

use fontcraft_model::fc_keys::AUTO_LAYER_RECIPE;
use fontcraft_model::plist::{Str, Value};
use fontcraft_model::{Component, Font, Glyph, Layer, Shape};

use crate::recipe::parse_recipe;
use crate::{ComponentsError, MAX_COMPONENT_DEPTH, MAX_RECIPE_BYTES};

/// The keyword after `#?` that stops anchor inheritance.
pub const NO_ANCHORS: &str = "noanchors";

/// A parsed automatic-layer recipe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoRecipe {
    /// The components, in order.
    pub parts: Vec<String>,
    /// The text of the `#!` directives, one line each, joined by line breaks.
    pub note: Option<String>,
    /// `#?noanchors` is present.
    pub no_anchors: bool,
}

/// What the directives of a recipe say, and its one line of components.
struct Directives<'a> {
    content: Option<&'a str>,
    note: Vec<&'a str>,
    no_anchors: bool,
}

fn directives(text: &str) -> Result<Directives<'_>, ComponentsError> {
    let mut out = Directives { content: None, note: Vec::new(), no_anchors: false };
    for line in text.lines() {
        let (content, directive) = match line.split_once('#') {
            Some((c, d)) => (c.trim(), Some(d)),
            None => (line.trim(), None),
        };
        if !content.is_empty() {
            if out.content.is_some() {
                return Err(ComponentsError::BadRecipe("a recipe has one line of components".into()));
            }
            out.content = Some(content);
        }
        match directive {
            Some(d) if d.starts_with('!') => out.note.push(d.get(1..).unwrap_or_default().trim()),
            Some(d) if d.starts_with('?') => {
                let keyword = d.get(1..).unwrap_or_default().trim();
                if keyword.eq_ignore_ascii_case(NO_ANCHORS) {
                    out.no_anchors = true;
                } else {
                    return Err(ComponentsError::BadRecipe(format!("unknown directive `#?{keyword}`; the one known is `#?{NO_ANCHORS}`")));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The recipe in `text` for the glyph `glyph`; see the module doc. Refused with
/// [`ComponentsError::BadRecipe`] or [`ComponentsError::LimitExceeded`] as
/// [`crate::recipe::parse_recipe`] refuses, and when it names no component, names another glyph
/// after `=`, or has an unknown `#?` keyword.
pub fn parse_auto_recipe(text: &str, glyph: &str) -> Result<AutoRecipe, ComponentsError> {
    if text.len() > MAX_RECIPE_BYTES {
        return Err(ComponentsError::LimitExceeded { what: "bytes in a recipe", limit: MAX_RECIPE_BYTES });
    }
    let d = directives(text)?;
    let content = d.content.ok_or_else(|| ComponentsError::BadRecipe("the recipe names no components".into()))?;
    let parts_text = match content.split_once('=') {
        Some((parts, target)) => {
            let target = target.trim();
            if target != glyph {
                return Err(ComponentsError::BadRecipe(format!("the recipe builds `{target}`, not `{glyph}`")));
            }
            parts
        }
        None => content,
    };
    let joined: Vec<&str> = parts_text.split('+').map(str::trim).collect();
    let recipe = parse_recipe(&format!("{}={glyph}", joined.join("+")))?;
    let note = (!d.note.is_empty()).then(|| d.note.join("\n"));
    Ok(AutoRecipe { parts: recipe.parts, note, no_anchors: d.no_anchors })
}

/// The recipe text stored on `layer`, if it is automatic.
pub fn layer_recipe(layer: &Layer) -> Option<&str> {
    layer.info.user_data.get(AUTO_LAYER_RECIPE).and_then(Value::as_str)
}

/// Stores `recipe` on `layer` (`None`: the layer is no longer automatic and keeps its contents).
pub fn set_layer_recipe(layer: &mut Layer, recipe: Option<&str>) {
    let data = &mut layer.info.user_data;
    match recipe {
        Some(text) => data.0.insert(AUTO_LAYER_RECIPE, Value::String(Str::new(text))),
        None => data.0.entries.retain(|(k, _)| k.as_str() != AUTO_LAYER_RECIPE),
    }
}

/// Whether any layer of `glyph` is automatic.
pub fn has_auto_layer(glyph: &Glyph) -> bool {
    glyph.layers.iter().any(|l| layer_recipe(l).is_some())
}

/// Whether `layer`'s recipe has `#?noanchors`: the layer then has only its own anchors.
pub fn stops_anchor_inheritance(layer: &Layer) -> bool {
    layer_recipe(layer).and_then(|text| directives(text).ok()).is_some_and(|d| d.no_anchors)
}

/// Whether `from` reaches `target` through components (or is it), following at most
/// [`MAX_COMPONENT_DEPTH`] levels and each glyph once.
pub fn reaches(font: &Font, from: &str, target: &str) -> bool {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<(&str, usize)> = vec![(from, 0)];
    while let Some((at, depth)) = stack.pop() {
        if at == target {
            return true;
        }
        if depth >= MAX_COMPONENT_DEPTH || !seen.insert(at) {
            continue;
        }
        let Some(g) = font.glyph(at) else { continue };
        for shape in g.layers.iter().flat_map(|l| l.shapes.iter()) {
            if let Shape::Component(c) = shape {
                stack.push((c.glyph.as_str(), depth + 1));
            }
        }
    }
    false
}

/// The components the automatic layer should hold now: the recipe's parts when every one exists
/// and none leads back to `glyph`, else none (pending). `None` when the recipe cannot be read.
pub fn wanted_parts(font: &Font, glyph: &str, recipe: &str) -> Option<Vec<String>> {
    let r = parse_auto_recipe(recipe, glyph).ok()?;
    let ready = r.parts.iter().all(|p| font.glyph(p).is_some()) && !r.parts.iter().any(|p| reaches(font, p, glyph));
    Some(if ready { r.parts } else { Vec::new() })
}

/// Whether `layer` holds exactly the components `parts`, in order, and nothing else.
fn holds(layer: &Layer, parts: &[String]) -> bool {
    layer.shapes.len() == parts.len() && layer.shapes.iter().zip(parts).all(|(s, p)| matches!(s, Shape::Component(c) if c.glyph == *p))
}

/// `glyph`'s automatic layers rebuilt in `font` where they differ from their recipes; whether
/// anything changed.
pub fn rebuild_glyph(font: &mut Font, glyph: &str) -> bool {
    let Some(g) = font.glyph(glyph) else { return false };
    if g.info.locked {
        return false;
    }
    let mut plans: Vec<(usize, Vec<String>)> = Vec::new();
    for (i, layer) in g.layers.iter().enumerate() {
        let Some(text) = layer_recipe(layer) else { continue };
        let Some(want) = wanted_parts(font, glyph, text) else { continue };
        if !holds(layer, &want) {
            plans.push((i, want));
        }
    }
    if plans.is_empty() {
        return false;
    }
    let Ok(mut g) = font.glyph_mut(glyph) else { return false };
    for (i, parts) in plans {
        if let Some(layer) = g.layers.get_mut(i) {
            layer.shapes = parts.iter().map(|p| Shape::Component(Component::new(p.as_str(), 0.0, 0.0))).collect();
        }
    }
    true
}

/// `new` with the automatic layers that follow its changes since `old` rebuilt: those of the
/// glyphs that changed, and, when glyphs were added, removed or renamed, those whose recipes
/// name one of them. Pointer-equal to `new` when nothing is rebuilt. Never fails: a layer that
/// cannot be rebuilt keeps what it has.
pub fn rebuild_auto_layers(old: &Font, new: Font) -> Font {
    let changed = crate::metrics::changed_glyphs(old, &new);
    rebuild_auto_layers_for(old, new, &changed).0
}

/// [`rebuild_auto_layers`] with the glyphs that changed since `old` already known; also returns
/// the glyphs whose automatic layers it tried to rebuild.
pub(crate) fn rebuild_auto_layers_for(old: &Font, mut new: Font, changed: &[String]) -> (Font, Vec<String>) {
    if changed.is_empty() {
        return (new, Vec::new());
    }
    let came_or_went: HashSet<&str> = changed.iter().filter(|n| old.glyph(n).is_none() || new.glyph(n).is_none()).map(String::as_str).collect();
    let mut targets: Vec<String> = changed.iter().filter(|n| new.glyph(n).is_some_and(|g| has_auto_layer(g))).cloned().collect();
    if !came_or_went.is_empty() {
        let already: HashSet<String> = targets.iter().cloned().collect();
        for g in new.glyphs.iter() {
            if already.contains(&g.name) {
                continue;
            }
            let names_one = g
                .layers
                .iter()
                .filter_map(layer_recipe)
                .filter_map(|t| parse_auto_recipe(t, &g.name).ok())
                .any(|r| r.parts.iter().any(|p| came_or_went.contains(p.as_str())));
            if names_one {
                targets.push(g.name.clone());
            }
        }
    }
    for name in &targets {
        rebuild_glyph(&mut new, name);
    }
    (new, targets)
}
