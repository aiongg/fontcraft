//! CID-keyed fonts (M13.50). Rows: `cid.convertToCid`, `cid.flatten`, `cid.editSubfonts`.
//!
//! The CID-keyed state lives in the source as `fontcraft_model::cid` describes: the character
//! collection (ROS), each glyph's CID and the sub-fonts with their glyphs and hinting values.
//!
//! | Command | What it does |
//! |---|---|
//! | `cid.convertToCid` | makes an ordinary font CID-keyed with one sub-font; CIDs follow the glyph order (the Identity ordering) or a Unicode CMap file |
//! | `cid.flatten` | makes a CID-keyed font ordinary again, glyphs in CID order, optionally encoded by a CMap file |
//! | `cid.editSubfonts` | adds a blank sub-font or another font's glyphs as one, removes one with its glyphs, lists one's glyphs, changes the supplement or a sub-font's hinting values |
//!
//! # CMap files
//!
//! CMap files are read by the compile crate's reader ([`fontcraft_compile::cid::parse_cmap`]),
//! the one the CID-keyed build uses. Only Unicode CMaps (named `Uni…`, or unnamed) can be used,
//! because glyphs are encoded by code point. The file is untrusted: at most
//! [`super::encodings::MAX_FILE_BYTES`] bytes are read, and the reader caps the codes at
//! [`fontcraft_compile::cid::MAX_CMAP_CODES`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_compile::cid::{CMap, CodeKind, parse_cmap};
use fontcraft_model::cid::{self, CidFont, MAX_CID, MAX_SUBFONTS, MAX_SUPPLEMENT, Ros, SubFont};
use fontcraft_model::plist::{Array, Num, Number, Str, Value as PValue};
use fontcraft_model::{Font, GLYPH_ORDER_PARAM, Glyph, GlyphTable, MAX_GLYPHS, Shape, rename_glyphs};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::{Params, kind};

/// Most numbers in one hinting array of a sub-font (the CFF limit for BlueValues).
const MAX_PRIVATE_ARRAY: usize = 14;
/// Most names one message lists.
const LISTED: usize = 5;

/// The hinting values a sub-font may set: (key, kind), with kind `a` an array of numbers, `n` a
/// number and `b` a yes-or-no value.
const PRIVATE_KEYS: [(&str, char); 11] = [
    ("postscriptBlueValues", 'a'),
    ("postscriptOtherBlues", 'a'),
    ("postscriptFamilyBlues", 'a'),
    ("postscriptFamilyOtherBlues", 'a'),
    ("postscriptStemSnapH", 'a'),
    ("postscriptStemSnapV", 'a'),
    ("postscriptBlueFuzz", 'n'),
    ("postscriptBlueShift", 'n'),
    ("postscriptBlueScale", 'n'),
    ("postscriptForceBold", 'b'),
    ("ExpansionFactor", 'n'),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "cid.convertToCid",
            "Convert to CID-Keyed…",
            ["Font", "CID"],
            None,
            "{registry: string, ordering: string, supplement: whole number 0..65535, cmap?: path} makes the font CID-keyed with one sub-font holding every exporting glyph. Registry and ordering are letters, digits and _. Without `cmap` the ordering must be Identity and CIDs follow the glyph order (.notdef is CID 0, the next exporting glyph 1…). With `cmap`, a Unicode CMap file of the same registry and ordering, each exporting glyph takes the CID of its first code point the CMap maps; glyphs it does not reach, and a second glyph on a CID already taken, follow the highest CID in glyph order. The glyph order does not change → {ros, cids: number, highest: CID, mapped: number, appended: [names], notExported: number, warnings: [text]}",
            plain_font,
            convert_to_cid
        )
        .files(const { &[FileParam::read("cmap")] }),
        cmd!(
            "cid.flatten",
            "Flatten CID Font",
            ["Font", "CID"],
            None,
            "{cmap?: path} makes a CID-keyed font ordinary: the sub-fonts merge into one, the glyphs go into CID order (glyphs without a CID follow in their order) and the CID data is removed; each sub-font's own hinting values are dropped. With `cmap`, a Unicode CMap file, each glyph whose CID the CMap reaches gets the code points it maps to that CID instead of its own, and other glyphs lose those code points → {glyphs: number in CID order, subfonts: number merged, encoded: number, warnings: [text]}",
            cid_font,
            flatten
        )
        .files(const { &[FileParam::read("cmap")] }),
        cmd!(
            "cid.editSubfonts",
            "Edit CID Sub-fonts",
            ["Font", "CID"],
            None,
            "{insert?: \"blank\" | open document title | path, name?: string, remove?: subfont, show?: subfont, supplement?: whole number 0..65535, private?: {subfont: name, values: {key: number | [numbers] | bool | null}}} does one of these to a CID-keyed font. insert adds a sub-font: blank adds an empty one; a document or file adds its glyphs (all but .notdef; names the font already has get .001, .002…; code points the font already uses are dropped; exporting glyphs take CIDs after the highest) and needs as many masters, matched in order. name names the new sub-font (default the inserted font's family name, or FD<n>): printable ASCII without spaces or ()<>[]{}/%. remove deletes a sub-font and its glyphs; it refuses the last sub-font and glyphs that stay but use a removed glyph as a component. show lists a sub-font's glyphs in CID order and changes nothing. supplement changes the character collection's supplement number. private sets a sub-font's hinting values (postscriptBlueValues, postscriptOtherBlues, postscriptFamilyBlues, postscriptFamilyOtherBlues, postscriptStemSnapH, postscriptStemSnapV: lists of at most 14 numbers; postscriptBlueFuzz, postscriptBlueShift, postscriptBlueScale, ExpansionFactor: numbers; postscriptForceBold: bool; null removes one) → {subfont, index, glyphs: [names], ros, renamed?: [{from, to}], droppedCodepoints?: number, removed?: [names], private?: {key: value}}",
            cid_font,
            edit_subfonts
        )
        .files(const { &[FileParam::read("insert").document().keywords(&["blank"])] }),
    ]
}

fn plain_font(s: &Session) -> std::result::Result<(), String> {
    let font = s.font().map_err(|e| e.to_string())?;
    if cid::is_cid_keyed(font) { Err("the font is CID-keyed already".into()) } else { Ok(()) }
}

fn cid_font(s: &Session) -> std::result::Result<(), String> {
    let font = s.font().map_err(|e| e.to_string())?;
    if cid::is_cid_keyed(font) { Ok(()) } else { Err("the font is not CID-keyed".into()) }
}

/// The CID-keyed state of the open font, refused when there is none or it is malformed.
fn read_cid(p: &Params, font: &Font) -> Result<CidFont> {
    CidFont::read(font).map_err(|e| p.bad(format!("the CID-keyed data cannot be read: {e}")))?.ok_or_else(|| p.bad("the font is not CID-keyed"))
}

/// Up to [`LISTED`] names, then how many more.
fn some_names<S: AsRef<str>>(names: &[S]) -> String {
    let shown: Vec<String> = names.iter().take(LISTED).map(|n| format!("`{}`", n.as_ref())).collect();
    match names.len().saturating_sub(LISTED) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

// ── CMap files ──────────────────────────────────────────────────────────────────────────────────

/// The notes a CMap gives the user: the CMap it names with `usecmap`, which is not read.
fn cmap_warnings(cmap: &CMap) -> Vec<String> {
    cmap.uses.iter().map(|u| format!("the CMap uses the CMap `{u}`, which is not read; codes only it maps are left out")).collect()
}

/// Reads the CMap file at `path`; refuses one that names another character collection.
fn read_cmap(p: &Params, path: &str, ros: Option<&Ros>) -> Result<CMap> {
    let bytes = super::encodings::read_text_file(path)?;
    let cmap = parse_cmap(&bytes, CodeKind::Unicode).map_err(|e| p.bad(format!("{path}: {e}")))?;
    if let Some(ros) = ros {
        let same = |given: &Option<String>, ours: &str| given.as_deref().is_none_or(|g| g == ours);
        if !same(&cmap.registry, &ros.registry) || !same(&cmap.ordering, &ros.ordering) {
            return Err(p.bad(format!(
                "{path} is a CMap for {}-{}, not for the font's {}-{}",
                cmap.registry.as_deref().unwrap_or("?"),
                cmap.ordering.as_deref().unwrap_or("?"),
                ros.registry,
                ros.ordering
            )));
        }
    }
    Ok(cmap)
}

// ── Convert to CID-keyed ────────────────────────────────────────────────────────────────────────

fn ros_part<'a>(p: &Params<'a>, key: &str) -> Result<&'a str> {
    let v = p.str(key)?.trim();
    if !cid::part_ok(v) {
        return Err(p.bad(format!("`{key}` must be 1 to {} letters, digits or _", cid::MAX_ROS_PART)));
    }
    Ok(v)
}

fn supplement(p: &Params, required: bool) -> Result<Option<u32>> {
    let v = p.opt_int("supplement", 0, i64::from(MAX_SUPPLEMENT))?;
    if required && v.is_none() {
        return Err(p.bad("`supplement` is required"));
    }
    Ok(v.and_then(|v| u32::try_from(v).ok()))
}

fn convert_to_cid(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("cid.convertToCid", v)?;
    let font = s.font()?;
    if cid::is_cid_keyed(font) {
        return Err(p.bad("the font is CID-keyed already"));
    }
    let registry = ros_part(&p, "registry")?.to_string();
    let ordering = ros_part(&p, "ordering")?.to_string();
    let supplement = supplement(&p, true)?.unwrap_or_default();
    let ros = Ros { registry, ordering, supplement };
    let cmap = match p.opt_str("cmap")? {
        Some(path) => Some(read_cmap(&p, path, Some(&ros))?),
        None if !ros.is_identity() => {
            return Err(p.bad(format!(
                "the CIDs of the {}-{} collection come from its CMap file: give one in `cmap`, or use the Identity ordering",
                ros.registry, ros.ordering
            )));
        }
        None => None,
    };
    let mut used: HashSet<u32> = HashSet::new();
    let mut cids: Vec<(String, u32)> = Vec::new();
    let mut appended: Vec<String> = Vec::new();
    let mut not_exported = 0usize;
    let mut next_identity = 1u32;
    if font.glyph(".notdef").is_some_and(|g| g.export) {
        used.insert(0);
        cids.push((".notdef".into(), 0));
    }
    for g in font.glyphs.iter().filter(|g| g.name != ".notdef") {
        if !g.export {
            not_exported += 1;
            continue;
        }
        let mapped = cmap.as_ref().and_then(|m| g.unicodes.iter().find_map(|cp| m.cids.get(cp).copied()));
        match mapped {
            Some(c) if used.insert(c) => cids.push((g.name.clone(), c)),
            _ if cmap.is_some() => appended.push(g.name.clone()),
            _ => {
                used.insert(next_identity);
                cids.push((g.name.clone(), next_identity));
                next_identity = next_identity.saturating_add(1);
            }
        }
    }
    let mapped = if cmap.is_some() { cids.iter().filter(|(n, _)| n != ".notdef").count() } else { 0 };
    let first = used.iter().max().map_or(1, |m| m.saturating_add(1)).max(1);
    if appended.len() > cids_left(first) {
        return Err(p.bad(format!("the font needs more than {} CIDs", u64::from(MAX_CID) + 1)));
    }
    cids.extend((first..=MAX_CID).zip(&appended).map(|(cid, name)| (name.clone(), cid)));
    if let Some((_, c)) = cids.iter().find(|(_, c)| *c > MAX_CID) {
        return Err(p.bad(format!("the CID {c} is above {MAX_CID}")));
    }
    let font_name = cid::font_name_from(&font.family_name);
    let mut sub = SubFont::new(font_name.clone().unwrap_or_else(|| "FD0".into()));
    let mut ordered = cids.clone();
    ordered.sort_by_key(|(_, c)| *c);
    sub.glyphs = ordered.iter().map(|(n, _)| n.clone()).collect();
    let state = CidFont { ros: ros.clone(), font_name, cids, subfonts: vec![sub] };
    let highest = state.max_cid().unwrap_or_default();
    let count = state.cids.len();
    s.edit("Convert to CID-Keyed", |f| {
        state.store(f);
        Ok(())
    })?;
    let warnings = cmap.as_ref().map(cmap_warnings).unwrap_or_default();
    Ok(json!({
        "ros": ros.text(),
        "cids": count,
        "highest": highest,
        "mapped": mapped,
        "appended": appended,
        "notExported": not_exported,
        "warnings": warnings,
    }))
}

// ── Flatten ─────────────────────────────────────────────────────────────────────────────────────

fn flatten(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("cid.flatten", v)?;
    let font = s.font()?;
    let state = read_cid(&p, font)?;
    let cmap = match p.opt_str("cmap")? {
        Some(path) => Some(read_cmap(&p, path, Some(&state.ros))?),
        None => None,
    };
    let map = state.cid_map();
    let mut with_cid: Vec<(u32, usize)> = Vec::new();
    let mut without: Vec<usize> = Vec::new();
    for (i, g) in font.glyphs.iter().enumerate() {
        match map.get(g.name.as_str()) {
            Some(c) => with_cid.push((*c, i)),
            None => without.push(i),
        }
    }
    with_cid.sort();
    let order: Vec<usize> = with_cid.iter().map(|(_, i)| *i).chain(without).collect();
    let in_cid_order = with_cid.len();
    // Glyph name → the code points the CMap gives it.
    let mut encode: HashMap<String, Vec<u32>> = HashMap::new();
    if let Some(cmap) = &cmap {
        let by_cid = cmap.codes_by_cid();
        for (c, i) in &with_cid {
            if let (Some(cps), Some(g)) = (by_cid.get(c), font.glyphs.as_arcs().get(*i)) {
                encode.entry(g.name.clone()).or_insert_with(|| cps.clone());
            }
        }
    }
    let taken: HashSet<u32> = encode.values().flatten().copied().collect();
    let encoded = encode.len();
    let subfonts = state.subfonts.len();
    s.edit("Flatten CID Font", |f| {
        let glyphs: Vec<Arc<Glyph>> = order.iter().filter_map(|i| f.glyphs.as_arcs().get(*i).cloned()).collect();
        f.glyphs = GlyphTable::from_glyphs(glyphs);
        if !taken.is_empty() {
            for i in 0..f.glyphs.len() {
                let Some(mut g) = f.glyphs.get_mut_at(i) else { continue };
                match encode.get(g.name.as_str()) {
                    Some(cps) => g.unicodes = cps.clone(),
                    None if g.unicodes.iter().any(|cp| taken.contains(cp)) => g.unicodes.retain(|cp| !taken.contains(cp)),
                    None => {}
                }
            }
        }
        if f.info.params.get(GLYPH_ORDER_PARAM).is_some() {
            let names = f.glyphs.iter().map(|g| PValue::String(Str::new(g.name.as_str()))).collect();
            Arc::make_mut(&mut f.info).params.set(GLYPH_ORDER_PARAM, PValue::Array(Array { items: names, ..Default::default() }));
        }
        CidFont::remove(f);
        Ok(())
    })?;
    let warnings = cmap.as_ref().map(cmap_warnings).unwrap_or_default();
    Ok(json!({"glyphs": in_cid_order, "subfonts": subfonts, "encoded": encoded, "warnings": warnings}))
}

// ── Sub-fonts ───────────────────────────────────────────────────────────────────────────────────

fn edit_subfonts(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("cid.editSubfonts", v)?;
    let actions: Vec<&str> =
        ["insert", "remove", "show", "supplement", "private"].into_iter().filter(|k| p.get(k).is_some_and(|v| !v.is_null())).collect();
    match actions.as_slice() {
        [_] => {}
        [] => return Err(p.bad("give one of `insert`, `remove`, `show`, `supplement` or `private`")),
        more => return Err(p.bad(format!("give one action at a time, not {}", more.join(", ")))),
    }
    if p.get("name").is_some_and(|v| !v.is_null()) && actions.first() != Some(&"insert") {
        return Err(p.bad("`name` goes with `insert`"));
    }
    let font = s.font()?;
    let mut state = read_cid(&p, font)?;
    let index_of = |state: &CidFont, key: &str| -> Result<usize> {
        let name = p.str(key)?;
        state.subfont_index(name).ok_or_else(|| {
            let known: Vec<&str> = state.subfonts.iter().map(|s| s.name.as_str()).collect();
            p.bad(format!("there is no sub-font `{name}`; the font has {}", some_names(&known)))
        })
    };
    match actions.first().copied() {
        Some("show") => {
            let i = index_of(&state, "show")?;
            Ok(subfont_json(&state, i, Map::new()))
        }
        Some("supplement") => {
            state.ros.supplement = supplement(&p, true)?.unwrap_or_default();
            s.edit("Change Supplement", |f| {
                state.store(f);
                Ok(())
            })?;
            Ok(json!({"ros": state.ros.text()}))
        }
        Some("remove") => {
            let i = index_of(&state, "remove")?;
            remove_subfont(s, &p, state, i)
        }
        Some("private") => set_private(s, &p, state),
        _ => insert_subfont(s, &p, state),
    }
}

/// A sub-font's name, index and glyphs in CID order, with the ROS and `more`.
fn subfont_json(state: &CidFont, i: usize, more: Map<String, Value>) -> Value {
    let map = state.cid_map();
    let mut glyphs: Vec<(u32, &str)> = state
        .subfonts
        .get(i)
        .map(|s| s.glyphs.iter().map(|g| (map.get(g.as_str()).copied().unwrap_or(u32::MAX), g.as_str())).collect())
        .unwrap_or_default();
    glyphs.sort_by_key(|(c, _)| *c);
    let mut out = Map::new();
    out.insert("subfont".into(), json!(state.subfonts.get(i).map(|s| s.name.as_str())));
    out.insert("index".into(), json!(i));
    out.insert("glyphs".into(), json!(glyphs.iter().map(|(_, g)| *g).collect::<Vec<_>>()));
    out.insert("ros".into(), json!(state.ros.text()));
    out.extend(more);
    Value::Object(out)
}

fn remove_subfont(s: &mut Session, p: &Params, mut state: CidFont, i: usize) -> Result<Value> {
    if state.subfonts.len() <= 1 {
        return Err(p.bad("a CID-keyed font keeps at least one sub-font; flatten the font instead"));
    }
    let font = s.font()?;
    let doomed: HashSet<String> = state.subfonts.get(i).map(|s| s.glyphs.iter().cloned().collect()).unwrap_or_default();
    let mut users: Vec<String> = Vec::new();
    for g in font.glyphs.iter().filter(|g| !doomed.contains(&g.name)) {
        let uses = g.layers.iter().flat_map(|l| l.shapes.iter()).any(|sh| matches!(sh, Shape::Component(c) if doomed.contains(&c.glyph)));
        if uses {
            users.push(g.name.clone());
        }
    }
    if !users.is_empty() {
        return Err(p.bad(format!("{} use glyphs of the sub-font as components; decompose or remove those components first", some_names(&users))));
    }
    let name = state.subfonts.get(i).map(|s| s.name.clone()).unwrap_or_default();
    state.subfonts.remove(i);
    state.cids.retain(|(n, _)| !doomed.contains(n));
    for sub in &mut state.subfonts {
        sub.glyphs.retain(|g| !doomed.contains(g));
    }
    let removed: Vec<String> = font.glyphs.iter().filter(|g| doomed.contains(&g.name)).map(|g| g.name.clone()).collect();
    s.edit("Remove Sub-font", |f| {
        f.glyphs.retain(|g| !doomed.contains(&g.name));
        state.store(f);
        Ok(())
    })?;
    Ok(json!({"subfont": name, "index": i, "glyphs": [], "removed": removed, "ros": state.ros.text()}))
}

/// The value a hinting key takes from JSON; `None` removes it.
fn private_value(p: &Params, key: &str, kind_of: char, v: &Value) -> Result<Option<PValue>> {
    let number = |x: &Value| -> Option<PValue> {
        let f = x.as_f64().filter(|f| f.is_finite() && f.abs() <= 1e9)?;
        if f.fract() == 0.0 { Some(PValue::Number(Number::int(f as i64))) } else { Number::float(f).map(PValue::Number) }
    };
    match (kind_of, v) {
        (_, Value::Null) => Ok(None),
        ('a', Value::Array(items)) if items.len() <= MAX_PRIVATE_ARRAY => {
            let items = items.iter().map(number).collect::<Option<Vec<_>>>().ok_or_else(|| p.bad(format!("`{key}` must hold numbers")))?;
            Ok(Some(PValue::Array(Array { items, ..Default::default() })))
        }
        ('a', _) => Err(p.bad(format!("`{key}` must be a list of at most {MAX_PRIVATE_ARRAY} numbers"))),
        ('n', x) => number(x).map(Some).ok_or_else(|| p.bad(format!("`{key}` must be a finite number, got {}", kind(x)))),
        (_, Value::Bool(b)) => Ok(Some(PValue::Bool(*b))),
        (_, x) => Err(p.bad(format!("`{key}` must be true or false, got {}", kind(x)))),
    }
}

/// A stored value as JSON, for results.
fn plist_json(v: &PValue, depth: usize) -> Value {
    match v {
        PValue::Number(n) => match n.value() {
            Num::Int(i) => json!(i),
            Num::Float(f) => json!(f),
        },
        PValue::Bool(b) => json!(b),
        PValue::String(s) | PValue::Date(s) => json!(s.as_str()),
        PValue::Array(a) if depth > 0 => Value::Array(a.items.iter().map(|x| plist_json(x, depth - 1)).collect()),
        PValue::Dict(d) if depth > 0 => Value::Object(d.entries.iter().map(|(k, x)| (k.as_str().to_string(), plist_json(x, depth - 1))).collect()),
        _ => Value::Null,
    }
}

fn set_private(s: &mut Session, p: &Params, mut state: CidFont) -> Result<Value> {
    let Some(spec) = p.opt_object("private")? else { return Err(p.bad("`private` must be an object")) };
    let name = match spec.get("subfont") {
        Some(Value::String(n)) => n.as_str(),
        _ => return Err(p.bad("`private.subfont` must name a sub-font")),
    };
    let i = state.subfont_index(name).ok_or_else(|| p.bad(format!("there is no sub-font `{name}`")))?;
    let Some(Value::Object(values)) = spec.get("values") else { return Err(p.bad("`private.values` must be an object")) };
    let mut changes: Vec<(&str, Option<PValue>)> = Vec::new();
    for (key, value) in values {
        let Some((k, kind_of)) = PRIVATE_KEYS.iter().find(|(k, _)| *k == key.as_str()) else {
            let known: Vec<&str> = PRIVATE_KEYS.iter().map(|(k, _)| *k).collect();
            return Err(p.bad(format!("`{key}` is not a sub-font hinting value; use {}", known.join(", "))));
        };
        changes.push((*k, private_value(p, k, *kind_of, value)?));
    }
    let Some(sub) = state.subfonts.get_mut(i) else { return Err(p.bad(format!("there is no sub-font `{name}`"))) };
    for (k, value) in changes {
        sub.set_private(k, value);
    }
    let private = sub.private().map(|d| plist_json(&PValue::Dict(d.clone()), 3)).unwrap_or_else(|| json!({}));
    s.edit("Sub-font Hinting", |f| {
        state.store(f);
        Ok(())
    })?;
    let mut more = Map::new();
    more.insert("private".into(), private);
    Ok(subfont_json(&state, i, more))
}

/// An open document named by title or path, else the source read from `r`.
fn font_ref(s: &Session, r: &str) -> Result<Arc<Font>> {
    if let Some(d) = s.documents().iter().find(|d| d.title == r || d.path.as_deref().is_some_and(|p| p == std::path::Path::new(r))) {
        return Ok(d.font.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (font, _) = crate::io::open(std::path::Path::new(r), None)?;
        Ok(Arc::new(font))
    }
    #[cfg(target_arch = "wasm32")]
    Err(crate::EngineError::Io(format!("no open document is called `{r}`")))
}

/// A name for a new sub-font: `wanted`, else `base`, else `FD<n>`, made unique with `-1`, `-2`….
fn new_subfont_name(p: &Params, state: &CidFont, wanted: Option<&str>, base: Option<String>) -> Result<String> {
    if let Some(w) = wanted {
        if !cid::font_name_ok(w) {
            return Err(p.bad(format!("`name` must be 1 to {} printable ASCII characters without spaces or ()<>[]{{}}/%", cid::MAX_FONT_NAME)));
        }
        if state.subfont_index(w).is_some() {
            return Err(p.bad(format!("the font has a sub-font `{w}` already")));
        }
        return Ok(w.to_string());
    }
    let base = base.unwrap_or_else(|| format!("FD{}", state.subfonts.len()));
    if state.subfont_index(&base).is_none() {
        return Ok(base);
    }
    (1..=MAX_SUBFONTS)
        .map(|n| format!("{base}-{n}"))
        .find(|n| state.subfont_index(n).is_none())
        .ok_or_else(|| p.bad("no free sub-font name; give one in `name`"))
}

fn insert_subfont(s: &mut Session, p: &Params, mut state: CidFont) -> Result<Value> {
    if state.subfonts.len() >= MAX_SUBFONTS {
        return Err(p.bad(format!("a CID-keyed font holds at most {MAX_SUBFONTS} sub-fonts")));
    }
    let wanted = p.opt_str("name")?;
    let source = p.str("insert")?;
    if source == "blank" {
        let name = new_subfont_name(p, &state, wanted, None)?;
        state.subfonts.push(SubFont::new(name));
        let i = state.subfonts.len() - 1;
        s.edit("Insert Sub-font", |f| {
            state.store(f);
            Ok(())
        })?;
        return Ok(subfont_json(&state, i, Map::new()));
    }
    let other = font_ref(s, source)?;
    let font = s.font()?;
    if other.masters.len() != font.masters.len() {
        return Err(p.bad(format!(
            "{source} has {} masters and the font {}; a sub-font's glyphs need one layer for each master",
            other.masters.len(),
            font.masters.len()
        )));
    }
    let name = new_subfont_name(p, &state, wanted, cid::font_name_from(&other.family_name))?;
    // Names the font has get a numbered suffix in the incoming copy, references included.
    let mut taken: HashSet<String> = font.glyphs.iter().chain(other.glyphs.iter()).map(|g| g.name.clone()).collect();
    let mut renames: Vec<(String, String)> = Vec::new();
    for g in other.glyphs.iter().filter(|g| g.name != ".notdef" && font.glyph(&g.name).is_some()) {
        let new = (1..=999)
            .map(|n| format!("{}.{n:03}", g.name))
            .find(|n| !taken.contains(n))
            .ok_or_else(|| p.bad(format!("no free name for the glyph `{}`", g.name)))?;
        taken.insert(new.clone());
        renames.push((g.name.clone(), new));
    }
    let mut incoming = Font::clone(&other);
    rename_glyphs(&mut incoming, &renames)?;
    let masters: HashMap<String, String> = incoming.masters.iter().zip(font.masters.iter()).map(|(a, b)| (a.id.clone(), b.id.clone())).collect();
    let mut used: HashSet<u32> = font.glyphs.iter().flat_map(|g| g.unicodes.iter().copied()).collect();
    let mut dropped = 0usize;
    let mut glyphs: Vec<Glyph> = Vec::new();
    for g in incoming.glyphs.iter().filter(|g| g.name != ".notdef") {
        let mut g = Glyph::clone(g);
        let before = g.unicodes.len();
        g.unicodes.retain(|cp| used.insert(*cp));
        dropped += before - g.unicodes.len();
        g.layers.retain(|l| masters.contains_key(&l.master));
        for l in &mut g.layers {
            if let Some(m) = masters.get(&l.master) {
                if l.id == l.master {
                    l.id = m.clone();
                }
                l.master = m.clone();
            }
        }
        glyphs.push(g);
    }
    if font.glyphs.len().saturating_add(glyphs.len()) > MAX_GLYPHS {
        return Err(p.bad(format!("the font would hold more than {MAX_GLYPHS} glyphs")));
    }
    let first = state.max_cid().map_or(1, |m| m.saturating_add(1)).max(1);
    let mut sub = SubFont::new(name);
    if glyphs.iter().filter(|g| g.export).count() > cids_left(first) {
        return Err(p.bad(format!("the font would need more than {} CIDs", u64::from(MAX_CID) + 1)));
    }
    for (cid, g) in (first..=MAX_CID).zip(glyphs.iter().filter(|g| g.export)) {
        state.cids.push((g.name.clone(), cid));
        sub.glyphs.push(g.name.clone());
    }
    state.subfonts.push(sub);
    let i = state.subfonts.len() - 1;
    s.edit("Insert Sub-font", |f| {
        for g in glyphs {
            f.glyphs.push(g);
        }
        state.store(f);
        Ok(())
    })?;
    let mut more = Map::new();
    more.insert("renamed".into(), json!(renames.iter().map(|(a, b)| json!({"from": a, "to": b})).collect::<Vec<_>>()));
    more.insert("droppedCodepoints".into(), json!(dropped));
    Ok(subfont_json(&state, i, more))
}

/// How many CIDs from `first` to [`MAX_CID`] are free to hand out.
fn cids_left(first: u32) -> usize {
    usize::try_from(u64::from(MAX_CID).saturating_add(1).saturating_sub(u64::from(first))).unwrap_or(usize::MAX)
}
