//! The context of contextual anchors (the `GPOS_Context` user data of `*` anchors) (M8.21).
//!
//! Commands: `anchor.setContext`.
//!
//! Rows: `anchor.setContext`, `palette.anchorContext` (the Palette field runs this command).
//!
//! The context is stored as the reference's files store it: a string in the anchor's user data
//! under `GPOS_Context`. Only anchors whose name starts with `*` take one, since only those are
//! contextual at export (`fontcraft_features::marks::contextual`). An empty context removes the
//! key. Setting the context the anchor already has changes nothing and adds no undo step.

use fontcraft_features::marks::contextual::{CONTEXT_KEY, MAX_CONTEXT_BYTES, anchor_context};
use fontcraft_model::plist::{Str, Value as PlistValue};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, Targets};

const ONE_ANCHOR: &[TargetKind] = &[TargetKind::Layer];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "anchor.setContext",
            "Set Context",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", anchor: name, context: string} store the context of a contextual anchor (a name starting with `*`): feature code with `*` for the glyph and an optional `&` for the mark, one context per line, an optional `lookupflag …;` before the pattern; tokens are allowed. An empty context removes it. At most 16,384 bytes; control characters other than line breaks and tabs are refused. A locked glyph refuses it → {anchor: name, context: string, changed: bool, selection: {glyph, layer, anchors: [name]}}",
            has_doc,
            set_context
        )
        .targets(ONE_ANCHOR),
    ]
}

fn set_context(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("anchor.setContext", v)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it sets the context of the anchor named by `anchor`"));
    }
    let name = p.str("anchor")?.to_string();
    let context = p.str("context")?;
    if context.len() > MAX_CONTEXT_BYTES {
        return Err(p.bad(format!("`context` is {} bytes long; at most {MAX_CONTEXT_BYTES} are allowed", context.len())));
    }
    if context.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(p.bad("`context` must not hold control characters other than line breaks and tabs"));
    }
    if !name.starts_with('*') {
        return Err(p.bad(format!("anchor `{name}` is not contextual: only an anchor whose name starts with `*` takes a context")));
    }
    // A context of blanks is no context.
    let context = if context.trim().is_empty() { String::new() } else { context.to_string() };
    let font = s.font()?;
    let r = t.resolve(font)?;
    let (_, anchor) = r.parts.anchor(&name).ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
    let changed = anchor_context(anchor).unwrap_or_default() != context || (context.is_empty() && anchor.info.user_data.get(CONTEXT_KEY).is_some());
    t.layer = Some(r.layer.id.clone());
    if changed {
        s.edit("Set Anchor Context", |font| {
            t.edit_layer(font, |l| {
                let a = l
                    .parts_mut(t.side)
                    .anchors
                    .iter_mut()
                    .find(|a| a.name == name)
                    .ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
                let data = &mut a.info.user_data.0;
                if context.is_empty() {
                    if let Some(i) = data.entries.iter().position(|(k, _)| k.as_str() == CONTEXT_KEY) {
                        data.remove_at(i);
                    }
                } else {
                    data.insert(CONTEXT_KEY, PlistValue::String(Str::new(context.clone())));
                }
                a.info.tidy();
                Ok(())
            })
        })?;
    }
    t.anchors = vec![name.clone()];
    Ok(json!({"anchor": name, "context": context, "changed": changed, "selection": t.to_json()}))
}
