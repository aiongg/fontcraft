//! `font.renameGlyphs` and `font.reorderGlyphs` on compiled fonts and sources (M12.15; `fontcraft_tables::rename`, `reorder`, `goadb`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! A compiled font (TrueType or CFF-flavoured sfnt) is changed table by table with
//! `fontcraft_tables`. A source is opened, changed with the model's own rename (components,
//! kerning, groups and the glyph order parameter follow; feature code is reported, not
//! rewritten) or by moving its glyphs, and saved in its own format. Names a rename list mentions
//! that the font lacks are reported as `missing`, as a GOADB usually lists more glyphs than one
//! font has.

use serde_json::{Value, json};

use super::file_io::{InputKind, OutputRequest, output_path, read_input, write_output};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};
use fontcraft_tables::goadb::{self, Goadb};
use fontcraft_tables::rename::{RenameOptions, rename_glyphs};
use fontcraft_tables::reorder::{self, ReorderOptions, SortKey};
use fontcraft_tables::sfnt::{SfntFont, WriteOptions};

/// Most lines in an order file (one name per line).
const MAX_ORDER_LINES: usize = fontcraft_tables::limits::MAX_GLYPHS;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.renameGlyphs",
            "Rename Glyphs in Font",
            [],
            None,
            "{font: path, face?: int, renames?: {old: new}, goadb?: path, map?: path, reverse?: bool, productionNames?: bool, updateUnicodes?: bool, order?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} renames glyphs of a compiled font (post names and the CFF charset) or a source (with its components, kerning and groups). A GOADB renames from its second column to its first and is applied before `map` (new name, then old name per line) and `renames`; `reverse` applies the GOADB and map the other way; `productionNames` derives names from code points; `order` also puts the glyphs in the GOADB's order. Fails, writing nothing, when two glyphs would get the same name → {output, renamed: [{from, to}], missing: [names], stillUsed: [{name, place}]}",
            always,
            rename
        ).files(const { &[FileParam::read("font"), FileParam::read("goadb"), FileParam::read("map"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            "font.reorderGlyphs",
            "Reorder Glyphs in Font",
            [],
            None,
            "{font: path, face?: int, order?: [name], orderFile?: path, goadb?: path, sort?: \"unicode\"|\"name\"|\"cid\"|\"size\", output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} gives a compiled font or a source a new glyph order and rewrites every table indexed by glyph. Glyphs the order leaves out follow in their old order; .notdef stays first → {output, order: [[old index, new index]]}",
            always,
            reorder_cmd
        ).files(const { &[FileParam::read("font"), FileParam::read("orderFile"), FileParam::read("goadb"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
    ]
}

/// The compiled font of `bytes`, refusing the containers this command cannot write back.
fn binary(p: &Params<'_>, kind: InputKind, bytes: &[u8]) -> Result<SfntFont> {
    if !matches!(kind, InputKind::TrueType | InputKind::Cff) {
        return Err(p.bad(format!("a {} file cannot be changed in place: convert it to a TrueType or CFF font first", kind.name())));
    }
    let face = p.opt_int("face", 0, 255)?.unwrap_or(0) as u32;
    Ok(fontcraft_tables::read(bytes, face)?)
}

fn text_file(p: &Params<'_>, key: &str) -> Result<Option<String>> {
    let Some(path) = p.opt_str(key)? else { return Ok(None) };
    let input = read_input(std::path::Path::new(path))?;
    String::from_utf8(input.bytes).map(Some).map_err(|_| p.bad(format!("`{key}`: {path} is not UTF-8 text")))
}

fn goadb_param(p: &Params<'_>, key: &str) -> Result<Option<Goadb>> {
    match text_file(p, key)? {
        Some(text) => goadb::parse(&text).map(Some).map_err(|e| p.bad(format!("`{key}`: {e}"))),
        None => Ok(None),
    }
}

/// `renames`, then the `map` file, as (old, new) pairs.
fn map_pairs(p: &Params<'_>, reverse: bool) -> Result<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    if let Some(map) = goadb_param(p, "map")? {
        // A map line is the new name, then the old one: the GOADB layout without code points.
        pairs.extend(map.pairs(reverse));
    }
    if let Some(obj) = p.opt_object("renames")? {
        for (old, new) in obj {
            let new = new.as_str().ok_or_else(|| p.bad(format!("`renames`: the new name of `{old}` is not a string")))?;
            pairs.push((old.clone(), new.to_string()));
        }
    }
    Ok(pairs)
}

fn write_binary(p: &Params<'_>, path: &std::path::Path, font: &SfntFont) -> Result<String> {
    let req = OutputRequest::from_params(p)?;
    let out = output_path(path, &req, ".renamed", None)?;
    let bytes = fontcraft_tables::write(font, &WriteOptions::default())?;
    write_output(&out, &bytes, &req)?;
    Ok(out.display().to_string())
}

fn rename(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.renameGlyphs", v)?;
    let path = std::path::PathBuf::from(p.str("font")?);
    let reverse = p.bool_or("reverse", false)?;
    let goadb = goadb_param(&p, "goadb")?;
    let pairs = map_pairs(&p, reverse)?;
    let production = p.bool_or("productionNames", false)?;
    let update_unicodes = p.bool_or("updateUnicodes", false)?;
    let reorder_too = p.bool_or("order", false)?;
    if (update_unicodes || reorder_too) && goadb.is_none() {
        return Err(p.bad("`updateUnicodes` and `order` need a `goadb`"));
    }
    let input = read_input(&path)?;
    if input.kind.is_font() {
        if update_unicodes {
            return Err(EngineError::Unsupported("setting code points from a GOADB in a compiled font".into()));
        }
        let font = binary(&p, input.kind, &input.bytes)?;
        let old = reorder::glyph_names(&font)?;
        let mut map = pairs;
        if production {
            let data = s.glyph_data()?;
            let codes = reorder::glyph_code_points(&font)?;
            for (i, name) in old.iter().enumerate().skip(1) {
                let cps = codes.get(i).cloned().unwrap_or_default();
                let new = data.production_name(name, &cps);
                if new != *name {
                    map.push((name.clone(), new));
                }
            }
        }
        let (goadb_opt, extra) = match (&goadb, reverse) {
            (Some(g), true) => (None, g.pairs(true)),
            (g, _) => (g.clone(), Vec::new()),
        };
        let options = RenameOptions { map: extra.into_iter().chain(map).collect(), goadb: goadb_opt, post_names: false };
        let result = rename_glyphs(font, &options)?;
        let new = reorder::glyph_names(&result.font)?;
        let mut font = result.font;
        if reorder_too && let Some(g) = &goadb {
            let wanted: Vec<String> =
                g.entries.iter().map(|e| if reverse { e.source_name.clone() } else { e.final_name.clone() }).filter(|n| new.contains(n)).collect();
            font = reorder::reorder_glyphs(font, &ReorderOptions { order: wanted })?;
        }
        let renamed: Vec<Value> = old.iter().zip(&new).filter(|(a, b)| a != b).map(|(a, b)| json!({"from": a, "to": b})).collect();
        let output = write_binary(&p, &path, &font)?;
        return Ok(json!({ "output": output, "renamed": renamed, "missing": result.missing, "stillUsed": [] }));
    }
    rename_source(s, &p, &path, goadb, pairs, reverse, production, update_unicodes, reorder_too)
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn rename_source(
    s: &mut Session,
    p: &Params<'_>,
    path: &std::path::Path,
    goadb: Option<Goadb>,
    pairs: Vec<(String, String)>,
    reverse: bool,
    production: bool,
    update_unicodes: bool,
    reorder_too: bool,
) -> Result<Value> {
    use std::collections::HashMap;
    let (mut font, _) = crate::io::open(path, None)?;
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut missing = Vec::new();
    if let Some(g) = &goadb {
        wanted.extend(g.pairs(reverse));
    }
    // The map and `renames` apply to the names after the GOADB.
    let mut after: HashMap<String, String> = names.iter().map(|n| (n.clone(), n.clone())).collect();
    for (from, to) in &wanted {
        if let Some(slot) = after.get_mut(from) {
            *slot = to.clone();
        } else {
            missing.push(from.clone());
        }
    }
    let by_current: HashMap<String, String> = after.iter().map(|(orig, cur)| (cur.clone(), orig.clone())).collect();
    for (from, to) in &pairs {
        match by_current.get(from) {
            Some(orig) => {
                after.insert(orig.clone(), to.clone());
            }
            None => missing.push(from.clone()),
        }
    }
    if production {
        let data = s.glyph_data()?;
        for g in font.glyphs.iter() {
            let new = data.production_name(&g.name, &g.unicodes);
            after.insert(g.name.clone(), new);
        }
    }
    let renames: Vec<(String, String)> = names.iter().filter_map(|n| after.get(n).filter(|t| *t != n).map(|t| (n.clone(), t.clone()))).collect();
    let report = fontcraft_model::rename_glyphs(&mut font, &renames)?;
    if update_unicodes && let Some(g) = &goadb {
        for e in g.entries.iter().filter(|e| !e.code_points.is_empty()) {
            let name = if reverse { &e.source_name } else { &e.final_name };
            if let Ok(mut glyph) = font.glyph_mut(name) {
                glyph.unicodes = e.code_points.clone();
            }
        }
    }
    if reorder_too && let Some(g) = &goadb {
        let order: Vec<String> = g.entries.iter().map(|e| if reverse { e.source_name.clone() } else { e.final_name.clone() }).collect();
        reorder_source(&mut font, &order)?;
    }
    let output = save_source(p, path, &font)?;
    let still: Vec<Value> = report.still_used.iter().map(|r| json!({"name": r.name, "place": r.place})).collect();
    let renamed: Vec<Value> = report.renamed.iter().map(|r| json!({"from": r.from, "to": r.to})).collect();
    Ok(json!({ "output": output, "renamed": renamed, "missing": missing, "stillUsed": still }))
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
fn rename_source(
    _: &mut Session,
    _: &Params<'_>,
    _: &std::path::Path,
    _: Option<Goadb>,
    _: Vec<(String, String)>,
    _: bool,
    _: bool,
    _: bool,
    _: bool,
) -> Result<Value> {
    Err(EngineError::Unsupported("reading a file by path in the browser".into()))
}

/// Moves the glyphs named in `order` to the front, in that order; the others follow in their
/// old order. Names the font lacks are skipped.
#[cfg(not(target_arch = "wasm32"))]
fn reorder_source(font: &mut fontcraft_model::Font, order: &[String]) -> Result<Vec<(usize, usize)>> {
    use std::collections::HashMap;
    use std::sync::Arc;
    let index: HashMap<&str, usize> = font.glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let mut taken = vec![false; index.len()];
    let mut new_to_old = Vec::with_capacity(taken.len());
    for name in order {
        if let Some(i) = index.get(name.as_str())
            && let Some(t) = taken.get_mut(*i)
            && !*t
        {
            *t = true;
            new_to_old.push(*i);
        }
    }
    for (i, t) in taken.iter().enumerate() {
        if !t {
            new_to_old.push(i);
        }
    }
    let glyphs: Vec<_> = font.glyphs.iter().map(Arc::clone).collect();
    let moved: Vec<_> = new_to_old.iter().filter_map(|i| glyphs.get(*i).cloned()).collect();
    font.glyphs = fontcraft_model::GlyphTable::from_glyphs(moved);
    Ok(new_to_old.iter().enumerate().map(|(new, old)| (*old, new)).collect())
}

#[cfg(not(target_arch = "wasm32"))]
fn save_source(p: &Params<'_>, path: &std::path::Path, font: &fontcraft_model::Font) -> Result<String> {
    use crate::io::SaveFormat;
    let req = OutputRequest::from_params(p)?;
    let out = output_path(path, &req, ".renamed", None)?;
    if req.no_clobber && !req.in_place && out.exists() {
        return Err(EngineError::Io(format!("{} exists and noClobber is set", out.display())));
    }
    let format = SaveFormat::of_path(&out)
        .or_else(|| SaveFormat::of_path(path))
        .ok_or_else(|| p.bad(format!("{} is not a source format that can be saved", out.display())))?;
    crate::io::save(font, &out, format)?;
    Ok(out.display().to_string())
}

/// The names of an order file: one per line, `#` comments, blank lines skipped.
fn order_lines(p: &Params<'_>, text: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let name = line.split('#').next().unwrap_or_default().trim();
        if name.is_empty() {
            continue;
        }
        if out.len() >= MAX_ORDER_LINES || name.len() > fontcraft_tables::limits::MAX_LIST_LINE {
            return Err(p.bad("`orderFile` has too many lines or a line that is too long"));
        }
        out.push(name.to_string());
    }
    Ok(out)
}

/// The order the parameters ask for, as glyph names; `None` for `sort`.
fn order_param(p: &Params<'_>) -> Result<Option<Vec<String>>> {
    let given = [p.get("order").is_some(), p.get("orderFile").is_some(), p.get("goadb").is_some(), p.get("sort").is_some()];
    if given.iter().filter(|g| **g).count() != 1 {
        return Err(p.bad("give exactly one of `order`, `orderFile`, `goadb` and `sort`"));
    }
    if let Some(list) = p.opt_array("order")? {
        if list.len() > MAX_ORDER_LINES {
            return Err(p.bad("`order` lists too many glyphs"));
        }
        return list
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| p.bad("`order` must list glyph names")))
            .collect::<Result<_>>()
            .map(Some);
    }
    if let Some(text) = text_file(p, "orderFile")? {
        return order_lines(p, &text).map(Some);
    }
    if let Some(g) = goadb_param(p, "goadb")? {
        return Ok(Some(g.entries.into_iter().map(|e| e.final_name).collect()));
    }
    Ok(None)
}

fn reorder_cmd(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.reorderGlyphs", v)?;
    let path = std::path::PathBuf::from(p.str("font")?);
    let sort = match p.opt_str("sort")? {
        Some(key) => Some(SortKey::parse(key).ok_or_else(|| p.bad(format!("`sort`: unknown key `{key}` (unicode, name, cid, size)")))?),
        None => None,
    };
    let order = order_param(&p)?;
    let input = read_input(&path)?;
    if input.kind.is_font() {
        let font = binary(&p, input.kind, &input.bytes)?;
        let names = reorder::glyph_names(&font)?;
        let wanted = match (order, sort) {
            (Some(list), _) => list,
            (None, Some(key)) => reorder::sort_order(&font, key)?,
            (None, None) => Vec::new(),
        };
        let new_to_old = reorder::new_order(&names, &wanted)?;
        let font = reorder::reorder_glyphs(font, &ReorderOptions { order: wanted })?;
        let output = write_binary(&p, &path, &font)?;
        let pairs: Vec<Value> = new_to_old.iter().enumerate().map(|(new, old)| json!([old, new])).collect();
        return Ok(json!({ "output": output, "order": pairs }));
    }
    reorder_source_file(&p, &path, order, sort)
}

#[cfg(not(target_arch = "wasm32"))]
fn reorder_source_file(p: &Params<'_>, path: &std::path::Path, order: Option<Vec<String>>, sort: Option<SortKey>) -> Result<Value> {
    let (mut font, _) = crate::io::open(path, None)?;
    let wanted = match (order, sort) {
        (Some(list), _) => list,
        (None, Some(SortKey::Name)) => {
            let mut names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            names.sort();
            names
        }
        (None, Some(SortKey::Unicode)) => {
            let mut glyphs: Vec<(u32, String)> =
                font.glyphs.iter().map(|g| (g.unicodes.iter().min().copied().unwrap_or(u32::MAX), g.name.clone())).collect();
            glyphs.sort_by_key(|g| g.0);
            glyphs.into_iter().map(|g| g.1).collect()
        }
        (None, Some(_)) => return Err(p.bad("a source can be sorted by `unicode` or `name`")),
        (None, None) => Vec::new(),
    };
    for name in &wanted {
        if font.glyph(name).is_none() {
            return Err(p.bad(format!("the font has no glyph `{name}`")));
        }
    }
    let notdef_first: Vec<String> = font.glyph(".notdef").map(|_| ".notdef".to_string()).into_iter().chain(wanted).collect();
    let pairs = reorder_source(&mut font, &notdef_first)?;
    let output = save_source(p, path, &font)?;
    Ok(json!({ "output": output, "order": pairs.iter().map(|(o, n)| json!([o, n])).collect::<Vec<_>>() }))
}

#[cfg(target_arch = "wasm32")]
fn reorder_source_file(_: &Params<'_>, _: &std::path::Path, _: Option<Vec<String>>, _: Option<SortKey>) -> Result<Value> {
    Err(EngineError::Unsupported("reading a file by path in the browser".into()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "glyph_order_file_tests.rs"]
mod tests;
