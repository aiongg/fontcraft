//! The glyph set: Add Glyphs, Duplicate Glyph, Add to Font from the glyph database, and whole
//! glyphs on the clipboard (Copy, Paste and Paste Special in Font View).
//!
//! New glyphs take their code points from the glyph data and their place from the default sort
//! order, among the glyphs already there; the others keep their order (`glyphOrder.default`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_components::compose;
use fontcraft_components::recipe::{self, Recipe};
use fontcraft_geom::{Rect, Shape as _};
use fontcraft_model::glyphdata::{GlyphData, items, validate};
use fontcraft_model::guide_follow::{BothGuides, follow_width};
use fontcraft_model::{
    Component, Font, Glyph, GlyphTable, Layer, MAX_COMPONENT_DEPTH, MAX_GLYPHS, MAX_NAME_LEN, ModelError, Shape, glyph_order, insert_by_sort_key,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::widths::{new_glyph_layers, new_layer_width};
use super::*;
use crate::params::{Params, TargetKind, kind};

/// The `format` value of copied glyphs on the clipboard.
pub(crate) const GLYPHS_FORMAT: &str = "fontcraft-glyphs";
/// The version of the copied-glyphs text this build writes, and the newest it reads.
const GLYPHS_VERSION: u32 = 1;
/// Largest copied-glyphs text, in bytes.
const MAX_CLIP_BYTES: usize = 64 << 20;
/// Most masters copied-glyphs text may describe.
const MAX_CLIP_MASTERS: usize = 4096;
/// Most entries in one `parts` list.
const MAX_PARTS: usize = 16;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.addGlyphs",
            "Add Glyphs…",
            ["Glyph"],
            Some("Cmd+Shift+G"),
            "{text: string, onExisting?: \"ask\" | \"replace\" | \"keep\" | \"cancel\" (default keep)} adds a glyph for every item of the text, items separated by spaces, tabs or line breaks: a glyph name (a legacy or production name becomes the nice name, unless the font uses custom naming), a single character, uniXXXX or uXXXXX, or a range first:last of two such items (code points Unicode does not assign are left out; at most 65,536 code points per range), or a recipe: component=glyph, base+mark=glyph or base+base=glyph builds the glyph after `=` from the components named before it, marks aligned to their base by anchors (at most 4,096 bytes and 64 components; the components may be glyphs the same text adds). New glyphs get the code points the glyph data gives their names and their place in the default sort order; a new glyph the glyph data describes as a composite is built from its components when the font has all of them, and stays empty otherwise. Names already in the font: keep leaves them; replace makes each a fresh empty glyph in its place (a recipe's glyph is rebuilt from the recipe); ask and cancel change nothing when there are any. Invalid names and recipes that cannot be built are listed in `skipped`. Text at most 512 KB, 65,535 items → {added: [names], replaced: [names], existing: [names], skipped: [{item, reason}], composed: [names built from components], notComposed: [{glyph, reason}]}",
            has_doc,
            add_glyphs
        ),
        cmd!(
            "glyph.duplicateGlyph",
            "Duplicate Glyph",
            ["Glyph"],
            Some("Cmd+D"),
            "{glyphs: [names], suffix?: string} copies each glyph, with everything but its code points, right after the original in the glyph order. A copy is named with the first free counter suffix (A.001, A.002…); with `suffix` it is named A.<suffix>, or A.<suffix>.001 and on when that is taken → {glyphs: [copies], copies: [{glyph, copy}]}",
            has_doc,
            duplicate_glyph
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyphInfo.addToFont",
            "Add to Font",
            [],
            None,
            "{names: [names]} adds a glyph for each glyph-data name that the font does not have yet, with the code point the glyph data gives it, at its place in the default sort order; a composite is built from its components when the font has all of them, as in Add Glyphs. Names already in the font are listed in `existing` and left as they are → {added: [names], existing: [names], composed: [names], notComposed: [{glyph, reason}]}",
            has_doc,
            add_to_font
        ),
        cmd!(
            "edit.pasteSpecial",
            "Paste Special",
            ["Edit"],
            Some("Cmd+Alt+V"),
            "{json | text: string (the copied glyphs edit.copy returns for `glyphs`), mode: \"sameName\" | \"selected\" | \"asComponents\", parts?: [\"all\" | \"activeLayer\" | \"kerningGroups\" | \"metricsKeys\" | \"lsb\" | \"rsb\" | \"width\"] (default all), targets?: [names], master?: id (default the first master), allMasters?: bool} pastes copied glyphs over existing ones. sameName overwrites the glyph of the same name and adds copied glyphs the font lacks; selected overwrites `targets`, taken in glyph order and paired with the copied glyphs in copy order, keeping the targets' names (extra copied glyphs are dropped); asComponents adds each copied glyph as a component to every target on `master`, or on every master with allMasters. Parts: all is the whole glyph but its name and code points; activeLayer the layer shown when copying, into the target's `master` layer; kerningGroups and metricsKeys those values; lsb, rsb and width the numbers per master (rsb wins over width). Masters are matched by id, then name, then position → {mode, glyphs: [changed targets], added: [names], skipped: [{glyph, reason}]}",
            has_doc,
            paste_special
        ),
    ]
}

/// An empty glyph with a layer for every master of `font`. A name that starts with an
/// underscore makes a helper glyph that does not export (`components.underscoreNameNotExported`).
fn fresh_glyph(font: &Font, data: &GlyphData, name: &str, code_points: &[u32]) -> Glyph {
    let mut g = Glyph::new(name);
    g.export = !name.starts_with('_');
    g.unicodes = code_points.to_vec();
    g.layers = new_glyph_layers(font, Some(data), name, code_points);
    g
}

/// `base.001`, `base.002`…: the first that is neither in `font` nor in `taken`.
pub(crate) fn free_name(font: &Font, taken: &HashSet<String>, base: &str) -> String {
    // Each candidate either is free or is one of the font's or taken's names, so this many
    // candidates always include a free one.
    let tries = font.glyphs.len().saturating_add(taken.len()).saturating_add(1);
    (1..=tries)
        .map(|n| format!("{base}.{n:03}"))
        .find(|name| font.glyph(name).is_none() && !taken.contains(name))
        .unwrap_or_else(|| format!("{base}.{tries:03}"))
}

/// The font's Use Custom Naming setting: names are kept as typed.
fn custom_naming(font: &Font) -> bool {
    font.info.settings.disables_nice_names == Some(true)
}

// ── Add Glyphs ──────────────────────────────────────────────────────────────────────────────────

fn add_glyphs(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.addGlyphs", v)?;
    let text = p.str("text")?;
    let on_existing = p.opt_str("onExisting")?.unwrap_or("keep");
    if !matches!(on_existing, "ask" | "replace" | "keep" | "cancel") {
        return Err(p.bad(format!("`onExisting` must be \"ask\", \"replace\", \"keep\" or \"cancel\", got \"{on_existing}\"")));
    }
    let list = items::parse(text).map_err(|e| p.bad(e.to_string()))?;
    if list.is_empty() {
        return Err(p.bad("`text` holds no items: give glyph names, characters, uniXXXX names, ranges or recipes, separated by spaces"));
    }
    let (recipe_items, plain): (Vec<items::Item>, Vec<items::Item>) = list.into_iter().partition(|i| matches!(i, items::Item::Recipe(_)));
    let data = s.glyph_data()?;
    let font = s.font()?;
    let custom = custom_naming(font);
    let resolved = items::resolve(&data, &plain, custom).map_err(|e| p.bad(e.to_string()))?;
    let mut skipped: Vec<Value> = resolved.skipped.iter().map(|k| json!({"item": k.item, "reason": k.reason})).collect();
    // Each recipe target once; a name a recipe builds is not also added empty.
    let mut recipes: Vec<(String, Recipe)> = Vec::new();
    let mut targets: HashSet<String> = HashSet::new();
    for item in &recipe_items {
        let text = item.text();
        match recipe::parse_recipe(&text) {
            Ok(r) if targets.contains(&r.target) => {
                skipped.push(json!({"item": text, "reason": format!("an earlier recipe already builds `{}`", r.target)}));
            }
            Ok(r) => {
                targets.insert(r.target.clone());
                recipes.push((text, r));
            }
            Err(e) => skipped.push(json!({"item": text, "reason": e.to_string()})),
        }
    }
    let wanted: Vec<items::Wanted> = resolved.glyphs.into_iter().filter(|w| !targets.contains(&w.name)).collect();
    let (existing, new): (Vec<items::Wanted>, Vec<items::Wanted>) = wanted.into_iter().partition(|w| font.glyph(&w.name).is_some());
    let mut existing_names: Vec<String> = existing.iter().map(|w| w.name.clone()).collect();
    existing_names.extend(recipes.iter().filter(|(_, r)| font.glyph(&r.target).is_some()).map(|(_, r)| r.target.clone()));
    if !existing_names.is_empty() && matches!(on_existing, "ask" | "cancel") {
        return Ok(json!({"added": [], "replaced": [], "existing": existing_names, "skipped": skipped, "composed": [], "notComposed": []}));
    }
    let replace = on_existing == "replace";
    let replace_plain = if replace { existing } else { Vec::new() };
    // Recipes whose target the font has run only with replace; the others are listed as existing.
    recipes.retain(|(_, r)| replace || font.glyph(&r.target).is_none());
    for name in replace_plain.iter().map(|w| w.name.as_str()).chain(recipes.iter().map(|(_, r)| r.target.as_str())) {
        if let Some(g) = font.glyph(name) {
            crate::check_unlocked(g)?;
        }
    }

    let mut next = Font::clone(font);
    for w in &replace_plain {
        let mut slot = next.glyph_mut(&w.name)?;
        *slot = fresh_glyph(font, &data, &w.name, &w.code_points);
    }
    // New recipe targets go in with the new glyphs, empty, so the table is rebuilt once and a
    // later recipe can use an earlier one's glyph; those that cannot be built are taken out.
    let used: HashSet<u32> = if custom { HashSet::new() } else { font.glyphs.iter().flat_map(|g| g.unicodes.iter().copied()).collect() };
    let mut new_glyphs: Vec<Glyph> = new.iter().map(|w| fresh_glyph(font, &data, &w.name, &w.code_points)).collect();
    for (_, r) in recipes.iter().filter(|(_, r)| font.glyph(&r.target).is_none()) {
        let code_points: Vec<u32> = if custom {
            Vec::new()
        } else {
            data.lookup(&r.target, &[]).unicode.map(|u| u.value).filter(|cp| !used.contains(cp)).into_iter().collect()
        };
        new_glyphs.push(fresh_glyph(font, &data, &r.target, &code_points));
    }
    insert_by_sort_key(&mut next, &data, new_glyphs)?;

    let mut composed = Vec::new();
    let (mut added_by_recipe, mut replaced_by_recipe, mut failed) = (Vec::new(), Vec::new(), HashSet::new());
    // Glyphs a recipe part may name: the font's own, the new ones, and earlier recipes' targets.
    let mut ready: HashSet<String> = new.iter().map(|w| w.name.clone()).collect();
    for (text, r) in &recipes {
        let original = font.glyph(&r.target);
        match build_recipe(&mut next, font, &data, r, &ready) {
            Ok(()) => {
                ready.insert(r.target.clone());
                composed.push(r.target.clone());
                if original.is_some() { replaced_by_recipe.push(r.target.clone()) } else { added_by_recipe.push(r.target.clone()) }
            }
            Err(reason) => {
                skipped.push(json!({"item": text, "reason": reason}));
                match original {
                    Some(g) => {
                        let mut slot = next.glyph_mut(&r.target)?;
                        *slot = Glyph::clone(g);
                    }
                    None => {
                        failed.insert(r.target.clone());
                    }
                }
            }
        }
    }
    if !failed.is_empty() {
        let kept: Vec<Arc<Glyph>> = next.glyphs.as_arcs().iter().filter(|g| !failed.contains(&g.name)).cloned().collect();
        next.glyphs = GlyphTable::from_glyphs(kept);
    }
    let mut added: Vec<String> = new.iter().map(|w| w.name.clone()).collect();
    let (auto, not_composed) = auto_compose(&mut next, &data, &added);
    composed.extend(auto);
    added.extend(added_by_recipe);
    let mut replaced: Vec<String> = replace_plain.iter().map(|w| w.name.clone()).collect();
    replaced.extend(replaced_by_recipe);
    s.edit("Add Glyphs", |f| {
        *f = next;
        Ok(())
    })?;
    Ok(json!({
        "added": added,
        "replaced": replaced,
        "existing": existing_names,
        "skipped": skipped,
        "composed": composed,
        "notComposed": not_composed,
    }))
}

/// Fills the recipe's target in `next` with the recipe's parts as components, aligned
/// (`addGlyphs.recipe`). The target is already in `next`: a new, empty glyph, or the glyph of
/// `font` it replaces, which becomes a fresh glyph with the same code points. Every part must be
/// a glyph of `font` or one of `ready`. On failure the reason is returned, and a replaced glyph
/// may be left fresh in `next` (the caller puts the original back).
fn build_recipe(next: &mut Font, font: &Font, data: &GlyphData, r: &Recipe, ready: &HashSet<String>) -> std::result::Result<(), String> {
    let mut missing: Vec<&str> = Vec::new();
    for part in &r.parts {
        if font.glyph(part).is_none() && !ready.contains(part) && !missing.contains(&part.as_str()) {
            missing.push(part);
        }
    }
    if !missing.is_empty() {
        return Err(format!("`{}` needs glyphs the font does not have: {}", r.target, missing.join(", ")));
    }
    if let Some(original) = font.glyph(&r.target) {
        let mut slot = next.glyph_mut(&r.target).map_err(|e| e.to_string())?;
        *slot = fresh_glyph(font, data, &r.target, &original.unicodes);
    }
    let glyph = compose::composite_glyph(next, data, &r.target, &r.parts).map_err(|e| e.to_string())?;
    let mut slot = next.glyph_mut(&r.target).map_err(|e| e.to_string())?;
    *slot = glyph;
    Ok(())
}

/// Fills each of the new, empty glyphs `names` in `font` with the components the glyph data
/// names for it (`components.autoComposeOnAddGlyph`). A glyph whose parts the font lacks stays
/// empty. Returns the glyphs filled, and those the glyph data decomposes that could not be
/// built, with the reason.
fn auto_compose(font: &mut Font, data: &GlyphData, names: &[String]) -> (Vec<String>, Vec<Value>) {
    let (mut composed, mut not_composed) = (Vec::new(), Vec::new());
    for name in names {
        let built = match compose::components_for(font, data, name) {
            Ok(None) => continue,
            Ok(Some(parts)) => compose::composite_glyph(font, data, name, &parts),
            Err(e) => Err(e),
        };
        let result = built.map_err(|e| e.to_string()).and_then(|g| {
            let mut slot = font.glyph_mut(name).map_err(|e| e.to_string())?;
            *slot = g;
            Ok(())
        });
        match result {
            Ok(()) => composed.push(name.clone()),
            Err(reason) => not_composed.push(json!({"glyph": name, "reason": reason})),
        }
    }
    (composed, not_composed)
}

// ── Duplicate Glyph ─────────────────────────────────────────────────────────────────────────────

fn duplicate_glyph(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.duplicateGlyph", v)?;
    let font = s.font()?;
    let names = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let suffix = match p.opt_str("suffix")? {
        None => None,
        Some(raw) => {
            let sfx = raw.strip_prefix('.').unwrap_or(raw);
            if sfx.is_empty() || sfx.len() > MAX_NAME_LEN {
                return Err(p.bad(format!("`suffix` must be 1 to {MAX_NAME_LEN} bytes long")));
            }
            validate(&format!("a.{sfx}"), true).map_err(|e| p.bad(format!("`suffix` cannot be part of a glyph name: {e}")))?;
            Some(sfx)
        }
    };
    let mut taken = HashSet::new();
    let mut copies: HashMap<String, Glyph> = HashMap::new();
    let mut pairs = Vec::with_capacity(names.len());
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        let copy_name = match suffix {
            Some(sfx) => {
                let base = format!("{name}.{sfx}");
                if font.glyph(&base).is_none() && !taken.contains(&base) { base } else { free_name(font, &taken, &base) }
            }
            None => free_name(font, &taken, name),
        };
        let mut copy = Glyph::clone(g);
        copy.name = copy_name.clone();
        copy.unicodes.clear();
        copy.file_name = None;
        taken.insert(copy_name.clone());
        pairs.push(json!({"glyph": name, "copy": copy_name}));
        copies.insert(name.clone(), copy);
    }
    let order: Vec<String> = names.iter().filter_map(|n| copies.get(n).map(|c| c.name.clone())).collect();
    s.edit("Duplicate Glyph", |font| {
        if font.glyphs.len().saturating_add(copies.len()) > MAX_GLYPHS {
            return Err(ModelError::TooManyGlyphs.into());
        }
        let mut out = Vec::with_capacity(font.glyphs.len() + copies.len());
        for g in font.glyphs.as_arcs() {
            out.push(Arc::clone(g));
            if let Some(copy) = copies.remove(&g.name) {
                out.push(Arc::new(copy));
            }
        }
        font.glyphs = GlyphTable::from_glyphs(out);
        Ok(())
    })?;
    Ok(json!({"glyphs": order, "copies": pairs}))
}

// ── Add to Font (Glyph Info window) ─────────────────────────────────────────────────────────────

fn add_to_font(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyphInfo.addToFont", v)?;
    if let Some(a) = p.opt_array("names")?
        && a.len() > MAX_GLYPHS
    {
        return Err(p.bad(format!("`names` holds {} names; at most {MAX_GLYPHS} are allowed", a.len())));
    }
    let names = p.strings("names")?;
    if names.is_empty() {
        return Err(p.bad("`names` is empty"));
    }
    for name in &names {
        if name.len() > MAX_NAME_LEN {
            return Err(p.bad(format!("a name in `names` is longer than {MAX_NAME_LEN} bytes")));
        }
        validate(name, true).map_err(|e| p.bad(format!("`{name}` is not a valid glyph name: {e}")))?;
    }
    let data = s.glyph_data()?;
    let font = s.font()?;
    let mut seen = HashSet::new();
    let (mut added, mut existing, mut glyphs) = (Vec::new(), Vec::new(), Vec::new());
    for name in names {
        if !seen.insert(name) {
            continue;
        }
        if font.glyph(name).is_some() {
            existing.push(name.to_string());
            continue;
        }
        let code_points: Vec<u32> = data.lookup(name, &[]).unicode.map(|u| u.value).into_iter().collect();
        glyphs.push(fresh_glyph(font, &data, name, &code_points));
        added.push(name.to_string());
    }
    let mut next = Font::clone(font);
    insert_by_sort_key(&mut next, &data, glyphs)?;
    let (composed, not_composed) = auto_compose(&mut next, &data, &added);
    s.edit("Add to Font", |f| {
        *f = next;
        Ok(())
    })?;
    Ok(json!({"added": added, "existing": existing, "composed": composed, "notComposed": not_composed}))
}

// ── Whole glyphs on the clipboard ───────────────────────────────────────────────────────────────

/// Copied glyphs as clipboard text: the glyphs whole, the masters they were drawn for (so a
/// paste can match them to the target's), the master shown when copying, and each master
/// layer's side bearings with components resolved in the source font.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlyphClip {
    format: String,
    version: u32,
    #[serde(default)]
    masters: Vec<ClipMaster>,
    #[serde(default)]
    active_master: Option<String>,
    glyphs: Vec<ClipGlyph>,
}

#[derive(Serialize, Deserialize)]
struct ClipMaster {
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Serialize, Deserialize)]
struct ClipGlyph {
    glyph: Glyph,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    metrics: Vec<ClipMetrics>,
}

#[derive(Serialize, Deserialize)]
struct ClipMetrics {
    layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lsb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rsb: Option<f64>,
}

/// Whether clipboard text holds copied glyphs (and not a drawing fragment).
pub(crate) fn looks_like_glyphs(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains(GLYPHS_FORMAT)
}

/// The copied-glyphs text in `json` or `text`, if either holds some.
pub(crate) fn glyphs_text<'a>(p: &Params<'a>) -> Result<Option<&'a str>> {
    for key in ["json", "text"] {
        if let Some(t) = p.opt_str(key)?
            && looks_like_glyphs(t)
        {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

/// The bounding box of a glyph's outline on a master, components resolved; `None` when empty.
fn outline_box(font: &Font, glyph: &Glyph, master: &str) -> Option<Rect> {
    font.outline_of(glyph, master).ok().filter(|b| !b.elements().is_empty()).map(|b| b.bounding_box())
}

/// `edit.copy` with `glyphs`: the glyphs whole, as clipboard text.
pub(crate) fn copy_glyphs(s: &Session, p: &Params) -> Result<Value> {
    let font = s.font()?;
    let names = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let active = match p.opt_str("master")? {
        Some(id) => font.master(id)?.id.clone(),
        None => font.default_master()?.id.clone(),
    };
    let mut glyphs = Vec::with_capacity(names.len());
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        let metrics = g
            .layers
            .iter()
            .filter(|l| l.is_master_layer())
            .map(|l| {
                let b = outline_box(font, g, &l.master);
                ClipMetrics { layer: l.id.clone(), lsb: b.map(|b| b.min_x()), rsb: b.map(|b| l.width - b.max_x()) }
            })
            .collect();
        glyphs.push(ClipGlyph { glyph: Glyph::clone(g), metrics });
    }
    let clip = GlyphClip {
        format: GLYPHS_FORMAT.into(),
        version: GLYPHS_VERSION,
        masters: font.masters.iter().map(|m| ClipMaster { id: m.id.clone(), name: m.name.clone() }).collect(),
        active_master: Some(active),
        glyphs,
    };
    let text = serde_json::to_string(&clip).map_err(|e| crate::EngineError::Internal(format!("copied glyphs could not be written: {e}")))?;
    Ok(json!({"json": text, "glyphs": names}))
}

fn decode(p: &Params, text: &str) -> Result<GlyphClip> {
    if text.len() > MAX_CLIP_BYTES {
        return Err(p.bad(format!("the copied glyphs are {} bytes; at most {MAX_CLIP_BYTES} are allowed", text.len())));
    }
    let clip: GlyphClip = serde_json::from_str(text).map_err(|e| p.bad(format!("the copied glyphs cannot be read: {e}")))?;
    if clip.format != GLYPHS_FORMAT {
        return Err(p.bad("the clipboard text does not hold copied glyphs"));
    }
    if clip.version > GLYPHS_VERSION {
        return Err(p.bad(format!("the copied glyphs are version {}; this build reads up to {GLYPHS_VERSION}", clip.version)));
    }
    if clip.glyphs.is_empty() {
        return Err(p.bad("the clipboard text holds no glyphs"));
    }
    if clip.glyphs.len() > MAX_GLYPHS {
        return Err(p.bad(format!("the clipboard text holds {} glyphs; at most {MAX_GLYPHS} are allowed", clip.glyphs.len())));
    }
    if clip.masters.len() > MAX_CLIP_MASTERS {
        return Err(p.bad(format!("the clipboard text describes {} masters; at most {MAX_CLIP_MASTERS} are allowed", clip.masters.len())));
    }
    Ok(clip)
}

/// Copied master id → target master id: the same id, else the same name, else the same
/// position; each target master is used once.
fn master_map(clip: &GlyphClip, font: &Font) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for m in &clip.masters {
        if font.masters.iter().any(|t| t.id == m.id) && used.insert(m.id.clone()) {
            map.insert(m.id.clone(), m.id.clone());
        }
    }
    for m in &clip.masters {
        if map.contains_key(&m.id) {
            continue;
        }
        if let Some(t) = font.masters.iter().find(|t| t.name == m.name && !used.contains(&t.id)) {
            used.insert(t.id.clone());
            map.insert(m.id.clone(), t.id.clone());
        }
    }
    for (i, m) in clip.masters.iter().enumerate() {
        if map.contains_key(&m.id) {
            continue;
        }
        if let Some(t) = font.masters.get(i).filter(|t| !used.contains(&t.id)) {
            used.insert(t.id.clone());
            map.insert(m.id.clone(), t.id.clone());
        }
    }
    map
}

/// The glyph's layers moved to the target's masters. Layers of masters without a counterpart
/// are dropped; with `fill` (the glyph data), a target master the glyph has no layer for gets an
/// empty one as wide as the glyph's first layer, or as a new layer when it has none.
fn map_layers(glyph: &Glyph, map: &HashMap<String, String>, font: &Font, fill: Option<&GlyphData>) -> Vec<Layer> {
    let mut layers = Vec::with_capacity(glyph.layers.len());
    for l in &glyph.layers {
        let Some(t) = map.get(&l.master) else { continue };
        let mut l = l.clone();
        if l.is_master_layer() {
            l.id = t.clone();
        }
        l.master = t.clone();
        layers.push(l);
    }
    if let Some(data) = fill {
        let width = glyph.layers.first().map(|l| l.width);
        for m in font.masters.iter() {
            if !layers.iter().any(|l| l.id == m.id) {
                let width = width.unwrap_or_else(|| new_layer_width(font, Some(data), &glyph.name, &m.id));
                layers.push(Layer::new(m.id.as_str(), width));
            }
        }
    }
    layers
}

/// `edit.paste` with copied glyphs: each becomes a new glyph. A name the font has already is
/// given the first free counter suffix, and the renamed glyph drops its code points.
pub(crate) fn paste_glyphs(s: &mut Session, p: &Params, text: &str) -> Result<Value> {
    let clip = decode(p, text)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let map = master_map(&clip, font);
    let mut taken: HashSet<String> = HashSet::new();
    let mut glyphs = Vec::with_capacity(clip.glyphs.len());
    let mut renamed = Vec::new();
    for c in clip.glyphs {
        let mut g = c.glyph;
        g.layers = map_layers(&g, &map, font, Some(&data));
        g.file_name = None;
        if font.glyph(&g.name).is_some() || taken.contains(&g.name) {
            let name = free_name(font, &taken, &g.name);
            renamed.push(json!({"from": g.name, "to": name}));
            g.name = name;
            g.unicodes.clear();
        }
        taken.insert(g.name.clone());
        glyphs.push(g);
    }
    let names: Vec<String> = glyphs.iter().map(|g| g.name.clone()).collect();
    s.edit("Paste", |font| Ok(insert_by_sort_key(font, &data, glyphs)?))?;
    Ok(json!({"pasted": names.len(), "flavour": "glyphs", "glyphs": names, "renamed": renamed}))
}

// ── Paste Special ───────────────────────────────────────────────────────────────────────────────

/// What Paste Special transfers.
#[derive(Default)]
struct Parts {
    all: bool,
    active_layer: bool,
    kerning_groups: bool,
    metrics_keys: bool,
    lsb: bool,
    rsb: bool,
    width: bool,
}

fn parts(p: &Params) -> Result<Parts> {
    let Some(list) = p.opt_array("parts")? else { return Ok(Parts { all: true, ..Parts::default() }) };
    if list.is_empty() {
        return Err(p.bad("`parts` is empty"));
    }
    if list.len() > MAX_PARTS {
        return Err(p.bad(format!("`parts` holds {} entries; at most {MAX_PARTS} are allowed", list.len())));
    }
    let mut out = Parts::default();
    for v in list {
        match v.as_str() {
            Some("all") => out.all = true,
            Some("activeLayer") => out.active_layer = true,
            Some("kerningGroups") => out.kerning_groups = true,
            Some("metricsKeys") => out.metrics_keys = true,
            Some("lsb") => out.lsb = true,
            Some("rsb") => out.rsb = true,
            Some("width") => out.width = true,
            Some(other) => {
                return Err(p.bad(format!(
                    "`parts` may hold \"all\", \"activeLayer\", \"kerningGroups\", \"metricsKeys\", \"lsb\", \"rsb\" and \"width\", got \"{other}\""
                )));
            }
            None => return Err(p.bad(format!("`parts` must be a list of strings, found {}", kind(v)))),
        }
    }
    Ok(out)
}

/// One target overwritten by one copied glyph.
struct Overwrite {
    target: String,
    /// The copied glyph with its layers moved to the target's masters.
    source: Glyph,
    /// The layer shown when copying, as copied.
    active: Option<Layer>,
    /// Target master id → the copied glyph's side bearings on the matching master.
    metrics: HashMap<String, (Option<f64>, Option<f64>)>,
    /// Target master id → the target's outline box there, before the paste.
    boxes: HashMap<String, Option<Rect>>,
}

fn overwrite(font: &Font, clip: &GlyphClip, map: &HashMap<String, String>, c: &ClipGlyph, target: &Glyph) -> Overwrite {
    let mut source = c.glyph.clone();
    source.layers = map_layers(&c.glyph, map, font, None);
    let active = clip.active_master.as_deref().and_then(|m| c.glyph.layer(m)).cloned();
    let metrics = c
        .metrics
        .iter()
        .filter_map(|m| {
            let layer = c.glyph.layers.iter().find(|l| l.id == m.layer && l.is_master_layer())?;
            Some((map.get(&layer.master)?.clone(), (m.lsb, m.rsb)))
        })
        .collect();
    let boxes = target.layers.iter().filter(|l| l.is_master_layer()).map(|l| (l.id.clone(), outline_box(font, target, &l.master))).collect();
    Overwrite { target: target.name.clone(), source, active, metrics, boxes }
}

/// Applies the chosen parts of `o` to `target`; returns why a part was left out on a master.
fn apply(target: &mut Glyph, o: &Overwrite, parts: &Parts, shown: &str, both: BothGuides) -> Vec<String> {
    let mut notes = Vec::new();
    let src = &o.source;
    if parts.all {
        let mut layers = src.layers.clone();
        for l in &target.layers {
            if !layers.iter().any(|x| x.id == l.id) {
                layers.push(l.clone());
            }
        }
        target.layers = layers;
        target.kern_groups = src.kern_groups.clone();
        target.export = src.export;
        let locked = target.info.locked;
        target.info = src.info.clone();
        if target.info.locked != locked {
            target.info.locked = locked;
        }
        return notes;
    }
    if parts.active_layer {
        match (&o.active, target.layer_mut(shown)) {
            (Some(from), Some(to)) => {
                to.width = from.width;
                to.shapes = from.shapes.clone();
                to.anchors = from.anchors.clone();
                to.info = from.info.clone();
                to.extra = from.extra.clone();
            }
            _ => notes.push("the layer shown when copying has no counterpart".to_string()),
        }
    }
    if parts.kerning_groups {
        target.kern_groups = src.kern_groups.clone();
    }
    if parts.metrics_keys {
        if target.info.metrics_keys != src.info.metrics_keys {
            target.info.metrics_keys = src.info.metrics_keys.clone();
        }
        for from in &src.layers {
            if let Some(to) = target.layers.iter_mut().find(|l| l.id == from.id)
                && to.info.metrics_keys != from.info.metrics_keys
            {
                to.info.metrics_keys = from.info.metrics_keys.clone();
            }
        }
    }
    if parts.lsb || parts.rsb || parts.width {
        for from in src.layers.iter().filter(|l| l.is_master_layer()) {
            let Some(to) = target.layers.iter_mut().find(|l| l.id == from.id) else { continue };
            let (lsb, rsb) = o.metrics.get(&from.id).copied().unwrap_or((None, None));
            let bounds = o.boxes.get(&from.id).copied().flatten();
            let before = to.width;
            let mut dx = 0.0;
            if parts.lsb {
                match (lsb, bounds) {
                    (Some(lsb), Some(b)) => match super::shared::shift_with_right_edge(to, lsb - b.min_x(), both) {
                        Ok(()) => {
                            dx = lsb - b.min_x();
                            to.width += dx;
                        }
                        Err(reason) => notes.push(format!("no left side bearing on master `{}`: {reason}", from.id)),
                    },
                    _ => notes.push(format!("no left side bearing on master `{}`: one of the outlines is empty", from.id)),
                }
            }
            let mut rsb_done = false;
            if parts.rsb {
                match (rsb, bounds) {
                    (Some(rsb), Some(b)) => {
                        to.width = b.max_x() + dx + rsb;
                        rsb_done = true;
                    }
                    _ => notes.push(format!("no right side bearing on master `{}`: one of the outlines is empty", from.id)),
                }
            }
            if parts.width && !rsb_done {
                to.width = from.width;
            }
            follow_width(to, before + dx, both);
        }
    }
    notes
}

/// Whether `from`, through components on any of its layers or the components `planned` adds,
/// reaches `to`.
fn reaches(font: &Font, planned: &HashMap<String, Vec<String>>, from: &str, to: &str) -> bool {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut next = vec![(from, 0usize)];
    while let Some((name, depth)) = next.pop() {
        if name == to {
            return true;
        }
        if depth > MAX_COMPONENT_DEPTH || !seen.insert(name) {
            continue;
        }
        if let Some(g) = font.glyph(name) {
            for l in &g.layers {
                for shape in &l.shapes {
                    if let Shape::Component(c) = shape {
                        next.push((c.glyph.as_str(), depth + 1));
                    }
                }
            }
        }
        if let Some(bases) = planned.get(name) {
            next.extend(bases.iter().map(|b| (b.as_str(), depth + 1)));
        }
    }
    false
}

fn paste_special(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.pasteSpecial", v)?;
    let mode = p.str("mode")?;
    if !matches!(mode, "sameName" | "selected" | "asComponents") {
        return Err(p.bad(format!("`mode` must be \"sameName\", \"selected\" or \"asComponents\", got \"{mode}\"")));
    }
    let parts = parts(&p)?;
    let all_masters = p.bool_or("allMasters", false)?;
    let text = glyphs_text(&p)?.ok_or_else(|| p.bad("give the copied glyphs (what edit.copy returns for `glyphs`) as `json` or `text`"))?;
    let clip = decode(&p, text)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let shown = match p.opt_str("master")? {
        Some(id) => font.master(id)?.id.clone(),
        None => font.default_master()?.id.clone(),
    };
    let targets = p.opt_glyphs("targets", font)?;
    let map = master_map(&clip, font);
    let mut skipped: Vec<Value> = Vec::new();

    if mode == "asComponents" {
        let targets = targets.filter(|t| !t.is_empty()).ok_or_else(|| p.bad("`targets` is required for asComponents"))?;
        let masters: Vec<String> = if all_masters { font.masters.iter().map(|m| m.id.clone()).collect() } else { vec![shown] };
        let mut planned: HashMap<String, Vec<String>> = HashMap::new();
        for t in &targets {
            if let Some(g) = font.glyph(t) {
                crate::check_unlocked(g)?;
            }
            for c in &clip.glyphs {
                let base = &c.glyph.name;
                if base == t || reaches(font, &planned, base, t) {
                    skipped.push(json!({"glyph": t, "reason": format!("a component of `{base}` in `{t}` would refer back to `{t}`")}));
                } else {
                    planned.entry(t.clone()).or_default().push(base.clone());
                }
            }
        }
        let plan: Vec<(String, Vec<String>)> = targets.iter().filter_map(|t| planned.remove(t).map(|b| (t.clone(), b))).collect();
        for c in clip.glyphs.iter().filter(|c| font.glyph(&c.glyph.name).is_none()) {
            skipped.push(json!({"glyph": c.glyph.name, "reason": "the font has no glyph of this name, so its components show no outline"}));
        }
        let changed: Vec<String> = plan.iter().filter(|(_, b)| !b.is_empty()).map(|(t, _)| t.clone()).collect();
        s.edit("Paste Special", |font| {
            for (t, bases) in &plan {
                let mut g = font.glyph_mut(t)?;
                for m in &masters {
                    if let Some(layer) = g.layer_mut(m) {
                        layer.shapes.extend(bases.iter().map(|b| Shape::Component(Component::new(b.as_str(), 0.0, 0.0))));
                    }
                }
            }
            Ok(())
        })?;
        return Ok(json!({"mode": mode, "glyphs": changed, "added": [], "skipped": skipped}));
    }

    let mut writes: Vec<Overwrite> = Vec::new();
    let mut new_glyphs: Vec<Glyph> = Vec::new();
    if mode == "sameName" {
        let mut seen: HashSet<&str> = HashSet::new();
        for c in &clip.glyphs {
            let name = c.glyph.name.as_str();
            if !seen.insert(name) {
                skipped.push(json!({"glyph": name, "reason": "copied twice; the first copy is pasted"}));
                continue;
            }
            match font.glyph(name) {
                Some(target) => {
                    crate::check_unlocked(target)?;
                    writes.push(overwrite(font, &clip, &map, c, target));
                }
                None => {
                    let mut g = c.glyph.clone();
                    g.layers = map_layers(&c.glyph, &map, font, Some(&data));
                    g.file_name = None;
                    new_glyphs.push(g);
                }
            }
        }
    } else {
        let mut targets = targets.filter(|t| !t.is_empty()).ok_or_else(|| p.bad("`targets` is required for selected"))?;
        let rank: HashMap<&str, usize> =
            glyph_order(font).iter().enumerate().filter_map(|(r, &i)| font.glyphs.as_arcs().get(i).map(|g| (g.name.as_str(), r))).collect();
        targets.sort_by_key(|t| rank.get(t.as_str()).copied().unwrap_or(usize::MAX));
        for (c, t) in clip.glyphs.iter().zip(&targets) {
            let Some(target) = font.glyph(t) else { continue };
            crate::check_unlocked(target)?;
            writes.push(overwrite(font, &clip, &map, c, target));
        }
        for c in clip.glyphs.iter().skip(targets.len()) {
            skipped.push(json!({"glyph": c.glyph.name, "reason": "more glyphs were copied than are selected"}));
        }
    }
    let changed: Vec<String> = writes.iter().map(|o| o.target.clone()).collect();
    let added: Vec<String> = new_glyphs.iter().map(|g| g.name.clone()).collect();
    let notes = s.edit("Paste Special", |font| {
        let both = BothGuides::of(font);
        let mut notes = Vec::new();
        for o in &writes {
            let mut g = font.glyph_mut(&o.target)?;
            for reason in apply(&mut g, o, &parts, &shown, both) {
                notes.push(json!({"glyph": o.target, "reason": reason}));
            }
        }
        insert_by_sort_key(font, &data, new_glyphs)?;
        Ok(notes)
    })?;
    skipped.extend(notes);
    Ok(json!({"mode": mode, "glyphs": changed, "added": added, "skipped": skipped}))
}
