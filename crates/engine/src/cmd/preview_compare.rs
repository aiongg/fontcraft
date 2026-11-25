//! `preview.compare`: the font as it is drawn over the font as it was at an earlier document revision, for agents that
//! check what their edits changed.
//!
//! Row: `preview.compare`.
//!
//! The earlier state is one of the document's undo or redo steps, found by the revision the document had then or by
//! counting steps back. Each glyph stands in a slot of its own, both states on one origin, so a changed width does not
//! push the glyphs after it apart. An unchanged drawing is grey; of a changed one the earlier is red and the current
//! blue over it.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::Font;
use fontcraft_render::color::composite_image;
use fontcraft_render::{DEFAULT_EM_PIXELS, Style, render_path, span};
use serde_json::{Value, json};

use super::preview::{MAX_RENDER_GLYPHS, PADDING, Scale, picture_size, scale_param};
use super::preview_line::{Nodes, Place, filled, place, view_at};
use super::stroke::OpenCorners;
use super::*;
use crate::params::Params;

/// Most glyphs drawn when the request names none.
pub const MAX_COMPARE_GLYPHS: usize = 100;
/// Most changed glyphs listed; `changedCount` counts them all.
pub const MAX_LISTED: usize = 1_000;
/// Most revisions an error lists.
const MAX_REVISIONS_NAMED: usize = 10;

const UNCHANGED: [u8; 4] = [0x80, 0x80, 0x80, 0xff];
const BEFORE: [u8; 4] = [0xd6, 0x45, 0x41, 0xff];
/// Lets the earlier drawing show through.
const AFTER: [u8; 4] = [0x21, 0x69, 0xd6, 0xb0];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "preview.compare",
        "Compare with Revision",
        [],
        None,
        "{revision?: number (a revision the document had, as document.inspect gave it) | steps?: number (that many undo steps back; default 1), glyphs?: [names], text?: string (default: the changed glyphs, the first 100; at most 1000), instance?: name|index, location?: {axis tag: internal value}, master?: id or name (default the first), perEm?: pixels (default 256) | scale?: pixels per font unit, showNodes?: bool} draws each glyph in a slot of its own, as it was at that revision in red under as it is in blue; a glyph whose drawing did not change in grey → {format, width, height, revision, changed: [names of the glyphs whose data differs, removed ones too; the first 1000], changedCount, kerningChanged, glyphs: [{name, changed (its drawing differs), before, after (it exists there)}], png? (base64; none when there is nothing to draw), warnings}",
        has_doc,
        compare
    )]
}

/// An earlier state of the document.
struct Earlier {
    font: Arc<Font>,
    path: Option<PathBuf>,
    revision: u64,
}

fn earlier(s: &Session, p: &Params) -> Result<Earlier> {
    let doc = s.doc()?;
    let whole = |key: &str| match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| p.bad(format!("`{key}` must be a whole number from 0"))),
    };
    let (revision, steps) = (whole("revision")?, whole("steps")?);
    let current = || Earlier { font: Arc::clone(&doc.font), path: doc.path.clone(), revision: doc.revision };
    let of = |e: &crate::HistoryEntry| Earlier { font: Arc::clone(&e.font), path: e.path.clone(), revision: e.revision };
    match (revision, steps) {
        (Some(_), Some(_)) => Err(p.bad("give `revision` or `steps`, not both")),
        (Some(r), None) if r == doc.revision => Ok(current()),
        (Some(r), None) => {
            let mut kept = doc.history.undo.iter().rev().chain(&doc.history.redo);
            kept.find(|e| e.revision == r).map(of).ok_or_else(|| {
                let known: Vec<String> = doc.history.undo.iter().rev().take(MAX_REVISIONS_NAMED).map(|e| e.revision.to_string()).collect();
                let known = if known.is_empty() { "none".to_string() } else { known.join(", ") };
                p.bad(format!("the document keeps no revision {r}; the last kept: {known} (or give `steps`)"))
            })
        }
        (None, Some(0)) => Ok(current()),
        (None, steps) => {
            let back = steps.unwrap_or(1);
            let index = usize::try_from(back).ok().and_then(|b| doc.history.undo.len().checked_sub(b));
            let count = doc.history.undo.len();
            index.and_then(|i| doc.history.undo.get(i)).map(of).ok_or_else(|| p.bad(format!("the document has {count} undo steps, not {back}")))
        }
    }
}

/// The glyphs whose data differs between the two fonts: those of `now` in its order, then the removed ones.
fn changed_glyphs(then: &Font, now: &Font) -> Vec<String> {
    let mut out: Vec<String> = now
        .glyphs
        .iter()
        .filter(|g| then.glyphs.get(&g.name).is_none_or(|old| !Arc::ptr_eq(old, g) && **old != ***g))
        .map(|g| g.name.clone())
        .collect();
    out.extend(then.glyphs.iter().filter(|g| now.glyphs.get(&g.name).is_none()).map(|g| g.name.clone()));
    out
}

/// The glyphs `glyphs` and `text` name in either font, else the changed ones.
fn names(p: &Params, then: &Font, now: &Font, changed: &[String], warnings: &mut Vec<String>) -> Result<Vec<String>> {
    let listed = p.opt_strings("glyphs")?;
    let text = p.opt_str("text")?;
    if listed.is_none() && text.is_none() {
        if changed.len() > MAX_COMPARE_GLYPHS {
            warnings.push(format!("{} glyphs changed; the first {MAX_COMPARE_GLYPHS} are drawn", changed.len()));
        }
        return Ok(changed.iter().take(MAX_COMPARE_GLYPHS).cloned().collect());
    }
    let listed = listed.unwrap_or_default();
    let text = text.unwrap_or_default();
    if listed.len() > MAX_RENDER_GLYPHS || listed.len().saturating_add(text.chars().count()) > MAX_RENDER_GLYPHS {
        return Err(p.bad(format!("at most {MAX_RENDER_GLYPHS} glyphs can be drawn")));
    }
    let mut out = Vec::new();
    for name in listed {
        if now.glyph(name).is_none() && then.glyph(name).is_none() {
            return Err(p.bad(format!("there is no glyph `{name}`")));
        }
        out.push(name.to_string());
    }
    if !text.is_empty() {
        let mut map: HashMap<u32, &str> = HashMap::new();
        for g in now.glyphs.iter().chain(then.glyphs.iter()) {
            for u in &g.unicodes {
                map.entry(*u).or_insert(g.name.as_str());
            }
        }
        for ch in text.chars().filter(|c| !c.is_control()) {
            match map.get(&u32::from(ch)) {
                Some(name) => out.push((*name).to_string()),
                None => warnings.push(format!("no glyph has the character U+{:04X}", u32::from(ch))),
            }
        }
    }
    Ok(out)
}

/// A glyph as it is drawn: its outline and advance.
type Drawing = (BezPath, f64);

/// The drawings of `names` at the place the parameters name, and the view and master they come from.
fn drawings(s: &mut Session, p: &Params, names: &[String]) -> Result<(HashMap<String, Drawing>, Arc<Font>, String)> {
    let font = Arc::clone(&s.doc()?.font);
    let at: Place = place(p, &font)?;
    let mut seen = HashSet::new();
    let known: Vec<String> = names.iter().filter(|n| font.glyph(n).is_some() && seen.insert(n.as_str())).cloned().collect();
    let (view, master) = view_at(s, p, &at, &known, false)?;
    let corners = OpenCorners::of(&view);
    let mut out = HashMap::new();
    for name in known {
        let Some(layer) = view.glyph(&name).and_then(|g| g.layer(&master)) else { continue };
        let width = if layer.width.is_finite() { layer.width.max(0.0) } else { 0.0 };
        let outline = filled(&view, &corners, &master, &name, layer);
        out.insert(name, (outline, width));
    }
    Ok((out, view, master))
}

fn compare(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("preview.compare", v)?;
    for key in ["color", "palette", "layer", "layers", "features", "script", "language"] {
        if p.get(key).is_some_and(|v| !matches!(v, Value::Bool(false)) && !v.as_array().is_some_and(Vec::is_empty)) {
            return Err(p.bad(format!("a comparison draws the glyphs' outlines in their slots: it takes no `{key}`")));
        }
    }
    let scale = scale_param(&p)?.unwrap_or(Scale::PerEm(DEFAULT_EM_PIXELS));
    let then = earlier(s, &p)?;
    let now = Arc::clone(&s.doc()?.font);
    let mut warnings = Vec::new();
    let changed = changed_glyphs(&then.font, &now);
    let names = names(&p, &then.font, &now, &changed, &mut warnings)?;
    let mut out = json!({
        "format": "png",
        "revision": then.revision,
        "changedCount": changed.len(),
        "kerningChanged": !Arc::ptr_eq(&then.font.kerning, &now.kerning) && then.font.kerning != now.kerning,
    });
    let put = |out: &mut Value, key: &str, value: Value| {
        if let Some(m) = out.as_object_mut() {
            m.insert(key.into(), value);
        }
    };
    put(&mut out, "changed", json!(changed.iter().take(MAX_LISTED).collect::<Vec<_>>()));
    if names.is_empty() {
        put(&mut out, "glyphs", json!([]));
        put(&mut out, "warnings", json!(warnings));
        return Ok(out);
    }

    let (after, view, master) = drawings(s, &p, &names)?;
    let mut past = s.without_documents();
    past.add_document(Font::clone(&then.font), then.path)?;
    // A place the earlier font does not have draws nothing of it.
    let before = match drawings(&mut past, &p, &names) {
        Ok((found, ..)) => found,
        Err(e) => {
            warnings.push(format!("nothing of the earlier state is drawn: {e}"));
            HashMap::new()
        }
    };
    let scale = scale.of(&p, f64::from(view.units_per_em).max(1.0))?;
    let (low, high) = span(&view, view.master(&master).map_err(|e| p.bad(e.to_string()))?);

    let mut plain = BezPath::new();
    let (mut old, mut new) = (BezPath::new(), BezPath::new());
    let mut nodes = Nodes::default();
    let show_nodes = p.bool_or("showNodes", false)?;
    let mut x = 0.0;
    let mut slots = Vec::with_capacity(names.len());
    let add = |to: &mut BezPath, outline: &BezPath, x: f64| {
        let mut moved = outline.clone();
        moved.apply_affine(Affine::translate((x, 0.0)));
        to.extend(moved.elements().iter().copied());
    };
    for name in &names {
        let (b, a) = (before.get(name), after.get(name));
        let differs = b != a;
        match (b, a) {
            (Some(_), Some((outline, _))) if !differs => add(&mut plain, outline, x),
            (b, a) => {
                if let Some((outline, _)) = b {
                    add(&mut old, outline, x);
                }
                if let Some((outline, _)) = a {
                    add(&mut new, outline, x);
                }
            }
        }
        if show_nodes && let Some(layer) = view.glyph(name).and_then(|g| g.layer(&master)) {
            nodes.add(layer, (x, 0.0));
        }
        slots.push(json!({"name": name, "changed": differs, "before": b.is_some(), "after": a.is_some()}));
        x += b.map_or(0.0, |d| d.1).max(a.map_or(0.0, |d| d.1));
    }
    let (width, height) = picture_size(&p, (x * scale + 2.0 * PADDING).ceil(), ((high - low) * scale + 2.0 * PADDING).ceil())?;
    let to_pixels = Affine::translate((PADDING, PADDING + high * scale)) * Affine::scale_non_uniform(scale, -scale);
    let failed = |e: fontcraft_render::RenderError| p.bad(e.to_string());
    let style = |ink: [u8; 4], background: [u8; 4]| Style { ink, background, anti_alias: true };
    let mut image = render_path(&plain, to_pixels, width, height, &style(UNCHANGED, [255; 4])).map_err(failed)?;
    for (path, ink) in [(&old, BEFORE), (&new, AFTER)] {
        let over = render_path(path, to_pixels, width, height, &style(ink, [0; 4])).map_err(failed)?;
        composite_image(&mut image, &over, 0, 0);
    }
    nodes.draw_over(&mut image, to_pixels).map_err(failed)?;
    put(&mut out, "width", json!(width));
    put(&mut out, "height", json!(height));
    put(&mut out, "glyphs", json!(slots));
    put(&mut out, "png", json!(base64::engine::general_purpose::STANDARD.encode(image.png().map_err(failed)?)));
    put(&mut out, "warnings", json!(warnings));
    Ok(out)
}
