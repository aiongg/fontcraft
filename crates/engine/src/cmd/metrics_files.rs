//! Import kerning from a legacy metrics file beside a Type 1 font: AFM (through the M5 reader in
//! `fontcraft_spacing::afm`) or PFM (`fontcraft_spacing::pfm`). Rows: `file.importMetricsFile`,
//! `pfm`.
//!
//! The pairs merge into the left-to-right kerning of each named master as glyph–glyph pairs, as
//! one undo step. AFM names glyphs; PFM gives character codes, which are found in the font by the
//! Unicode values its character set gives them, and its amounts are scaled from the file's units
//! per em to the font's. Pairs naming a glyph or code the font lacks are left out and listed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::Font;
use fontcraft_spacing::kerning::pairs::{self, NewPair};
use fontcraft_spacing::kerning::{Direction, Side};
use fontcraft_spacing::kerning_text::VALUE_RANGE;
use fontcraft_spacing::{afm, metrics_source, pfm, round_half_up};
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

/// Most unknown glyph names or codes listed in a result.
const MAX_REPORTED: usize = 1000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "file.importMetricsFile",
        "Import Kerning from Metrics File…",
        ["File", "Import"],
        None,
        "{path: an AFM or PFM file (told apart by content), masters?: [master ids] (default: the first master), merge?: \"replace\"|\"addMissing\" (default replace)} merges the file's kerning pairs into the left-to-right kerning of each master as glyph–glyph pairs, as one undo step. replace: a pair the master has takes the file's value; addMissing: only pairs the master lacks are added. AFM pairs name glyphs; PFM pairs name character codes, read as Windows-1252 (a symbol font's code c as U+F000+c, then c) and found by Unicode value, with amounts scaled from the file's units per em to the font's and rounded half up. Pairs whose glyphs or codes the font lacks are left out and listed; ligature and track kerning data are not read. A master that follows another's metrics is refused → {format: \"afm\"|\"pfm\", face?, merge, pairs: pairs in the file, imported: pairs merged, masters: [{master, added, replaced, unchanged, kept}], unknownGlyphs: [names], unknownCodes: [codes]}",
        has_doc,
        import_metrics_file
    )]
}

#[derive(Clone, Copy, PartialEq)]
enum Merge {
    Replace,
    AddMissing,
}

impl Merge {
    fn name(self) -> &'static str {
        match self {
            Merge::Replace => "replace",
            Merge::AddMissing => "addMissing",
        }
    }
}

/// The masters to merge into: each must exist and keep its own kerning.
fn target_masters(p: &Params, font: &Font) -> Result<Vec<String>> {
    let wanted: Vec<String> = match p.opt_strings("masters")? {
        Some(list) if !list.is_empty() => list.into_iter().map(str::to_string).collect(),
        _ => vec![font.default_master()?.id.clone()],
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in wanted {
        if !seen.insert(id.clone()) {
            continue;
        }
        let master = font.master(&id).map_err(|_| p.bad(format!("the font has no master `{}`", clip(&id))))?;
        if let Some(source) = metrics_source(font, &master.id).filter(|m| m.id != master.id) {
            return Err(p.bad(format!(
                "master `{}` takes its metrics and kerning from master `{}` ({}); import into that master instead",
                master.id, source.id, source.name
            )));
        }
        out.push(master.id.clone());
    }
    Ok(out)
}

fn clip(text: &str) -> String {
    text.chars().take(200).collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn load(path: &str) -> Result<Vec<u8>> {
    crate::io::read_file(std::path::Path::new(path))
}

#[cfg(target_arch = "wasm32")]
fn load(_: &str) -> Result<Vec<u8>> {
    Err(crate::EngineError::Unsupported("reading a metrics file by path in the browser".into()))
}

/// The pairs a file gives, mapped to the font's glyphs.
struct Plan {
    format: &'static str,
    face: Option<String>,
    in_file: usize,
    pairs: Vec<NewPair>,
    unknown_glyphs: Vec<String>,
    unknown_codes: Vec<u8>,
}

fn plan(p: &Params, font: &Font, bytes: &[u8]) -> Result<Plan> {
    if pfm::sniff(bytes) {
        let doc = pfm::read(bytes).map_err(|e| p.bad(format!("`path` (PFM): {e}")))?;
        return plan_pfm(p, font, &doc);
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => std::borrow::Cow::Borrowed(t),
        // Older AFM files are Latin-1.
        Err(_) => std::borrow::Cow::Owned(bytes.iter().map(|&b| char::from(b)).collect::<String>()),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if !afm::sniff(text) {
        return Err(p.bad("`path` is neither an AFM file (it starts with StartFontMetrics) nor a PFM file"));
    }
    let doc = afm::read(text).map_err(|e| p.bad(format!("`path` (AFM): {e}")))?;
    let mut out = Plan {
        format: "afm",
        face: doc.font_name.clone(),
        in_file: doc.pairs.len(),
        pairs: Vec::new(),
        unknown_glyphs: Vec::new(),
        unknown_codes: Vec::new(),
    };
    let mut seen = HashSet::new();
    for k in &doc.pairs {
        let mut known = true;
        for name in [&k.left, &k.right] {
            // A name starting with `@` would be stored as a group.
            if name.starts_with('@') || font.glyph(name).is_none() {
                known = false;
                if out.unknown_glyphs.len() < MAX_REPORTED && seen.insert(name.clone()) {
                    out.unknown_glyphs.push(name.clone());
                }
            }
        }
        if known {
            out.pairs.push(NewPair { left: Side::Glyph(k.left.clone()), right: Side::Glyph(k.right.clone()), value: k.value });
        }
    }
    Ok(out)
}

fn plan_pfm(p: &Params, font: &Font, doc: &pfm::Pfm) -> Result<Plan> {
    let mut by_unicode: HashMap<u32, &str> = HashMap::new();
    for g in font.glyphs.iter() {
        for u in &g.unicodes {
            by_unicode.entry(*u).or_insert(g.name.as_str());
        }
    }
    let glyph = |code: u8| pfm::unicodes(doc.char_set, code).into_iter().find_map(|u| by_unicode.get(&u).copied());
    let scale = f64::from(font.units_per_em) / f64::from(doc.units());
    let mut out = Plan {
        format: "pfm",
        face: doc.face.clone(),
        in_file: doc.pairs.len(),
        pairs: Vec::new(),
        unknown_glyphs: Vec::new(),
        unknown_codes: Vec::new(),
    };
    let mut seen = HashSet::new();
    for k in &doc.pairs {
        let (left, right) = (glyph(k.first), glyph(k.second));
        for (code, found) in [(k.first, left), (k.second, right)] {
            if found.is_none() && seen.insert(code) {
                out.unknown_codes.push(code);
            }
        }
        let (Some(left), Some(right)) = (left, right) else { continue };
        let value = round_half_up(f64::from(k.value) * scale);
        if !VALUE_RANGE.contains(&value) {
            return Err(p.bad(format!(
                "the pair of codes {} and {} is {value} units at the font's units per em; kerning values run from {} to {}",
                k.first,
                k.second,
                VALUE_RANGE.start(),
                VALUE_RANGE.end()
            )));
        }
        out.pairs.push(NewPair { left: Side::Glyph(left.to_string()), right: Side::Glyph(right.to_string()), value });
    }
    out.unknown_codes.sort_unstable();
    Ok(out)
}

fn import_metrics_file(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.importMetricsFile", v)?;
    let path = p.str("path")?;
    let merge = match p.opt_str("merge")? {
        None | Some("replace") => Merge::Replace,
        Some("addMissing") => Merge::AddMissing,
        Some(other) => return Err(p.bad(format!("`merge` must be replace or addMissing, got `{}`", clip(other)))),
    };
    let font = s.font()?;
    let masters = target_masters(&p, font)?;
    let bytes = load(path)?;
    let plan = plan(&p, font, &bytes)?;
    let overwrite = merge == Merge::Replace;
    let direction = Direction::Ltr;
    let per_master = s.edit("Import Kerning from Metrics File", |font| {
        let mut out = Vec::new();
        for master in &masters {
            let paste = pairs::plan_paste(font, direction, master, &plan.pairs);
            let conflicts = paste.conflicts.len();
            if !paste.added.is_empty() || (overwrite && conflicts > 0) {
                pairs::apply_paste(direction.table_mut(Arc::make_mut(&mut font.kerning)), direction, master, &plan.pairs, &paste, overwrite);
            }
            let (replaced, kept) = if overwrite { (conflicts, 0) } else { (0, conflicts) };
            out.push(json!({
                "master": master,
                "added": paste.added.len(),
                "replaced": replaced,
                "unchanged": paste.unchanged.len(),
                "kept": kept,
            }));
        }
        Ok(out)
    })?;
    let mut result = json!({
        "format": plan.format,
        "merge": merge.name(),
        "pairs": plan.in_file,
        "imported": plan.pairs.len(),
        "masters": per_master,
        "unknownGlyphs": plan.unknown_glyphs,
        "unknownCodes": plan.unknown_codes,
    });
    if let (Some(face), Some(obj)) = (plan.face, result.as_object_mut()) {
        obj.insert("face".into(), json!(face));
    }
    Ok(result)
}
