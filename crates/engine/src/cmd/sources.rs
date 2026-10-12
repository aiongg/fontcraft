//! Comparing and merging sources: `source.diff`, `source.merge`, `font.compareGlyphs` (M12.37).
//! A refused or stale merge writes nothing.
//!
//! `source.diff` compares two source files part by part ([`compare`]); every change has an id.
//! `source.merge` compares the same two files again, applies the changes whose ids it is given
//! ([`apply`]), checks the result and only then writes it, in BASE's format. A change whose
//! place in BASE holds something else than the comparison saw has another id, so it is refused.
// In the browser only `font.compareGlyphs` on open documents runs; the file commands refuse.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod apply;
mod compare;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;

use std::collections::BTreeSet;
use std::sync::Arc;

use fontcraft_model::Font;
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;
use compare::{Change, Options, Part, What};

/// Most changes one source diff lists.
pub const MAX_SOURCE_CHANGES: usize = 1_000_000;

/// The tolerance `font.compareGlyphs` uses for near matching when none is given, in units.
const DEFAULT_NEAR_TOLERANCE: f64 = 1.0;

/// The largest tolerance accepted, in units.
const MAX_TOLERANCE: f64 = 100_000.0;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "source.diff",
            "Compare Sources",
            [],
            None,
            "{a: path, b: path, parts?: [\"glyphs\"|\"info\"|\"masters\"|\"instances\"|\"axes\"|\"kerning\"|\"features\"], glyphs?: [name], tolerance?: number} compare two source files (.glyphs, .glyphspackage, .fontcraft, UFO, designspace; the two may differ in format) and list every change from A to B. Glyphs match by name, and a glyph only one side has is matched as renamed when its outlines and code points are unchanged; masters and instances match by axis location, then by name; axes by tag. Per glyph: name, code points, export, glyph info, other layers, and per master the width, outlines, components, anchors and layer info; kerning pairs per master and direction, kerning groups (per glyph) and contexts; Font Info fields, custom parameters and properties; each field of masters, instances and axes; feature, class and prefix code and glyph groups; the order of each list. glyphs: only these glyphs (named in A or B; kerning pairs are not filtered); tolerance: widths, outlines, components and anchors closer than this many units count as equal (default 0). Each change has an id that source.merge takes; it changes when either value changes. Nothing is written → {same: bool, changes: [{id, part, kind: \"added\"|\"removed\"|\"renamed\"|\"changed\", glyph?, master?, field?, a?, b?}], summary: {part: count}}",
            always,
            diff
        ),
        cmd!(
            "source.merge",
            "Merge Sources",
            [],
            None,
            "{base: path, other: path, take: [id] | \"all\", parts?: [string], glyphs?: [name], tolerance?: number, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} compare BASE with OTHER as source.diff does (parts, glyphs and tolerance as there) and apply the chosen changes from OTHER to a copy of BASE: the ids source.diff returned, or \"all\". The result is checked and written in BASE's format to output (or BASE.merged in outputDir or beside BASE); BASE itself is replaced only with inPlace. All or nothing: an id that is not among the changes as the sources are now (one of them changed since the comparison) refuses the whole merge, and so does a result that is not a consistent font; then nothing is written. A designspace is not written beside BASE's own UFOs unless inPlace → {applied: [id], output: path, format, losses}",
            always,
            merge
        ),
        cmd!(
            "font.compareGlyphs",
            "Compare Glyphs of Two Fonts…",
            [],
            None,
            "{a: document title | path, b: document title | path, what?: [\"glyphSet\"|\"outlines\"|\"names\"], outlineMatch?: \"exact\"|\"near\", tolerance?: number, acceptInlinedComponents?: bool, differencesToBackground?: bool} compare the glyphs of two fonts (open documents, sources or compiled fonts) by glyph name and matched master: glyphs only one has, glyphs whose outlines or components differ, and glyphs whose code points differ (default: all three). near: coordinates within tolerance (default 1 unit; giving a tolerance means near) count as equal. acceptInlinedComponents: compare the drawn outlines, so components and the same contours drawn in place are equal. differencesToBackground: a must be the active document; each differing outline of b is put into the background of a's layer, as one undo step. Bitmaps, substitutions and positioning are not compared yet → {same: bool, onlyInA: [name], onlyInB: [name], outlines: [{glyph, master}], names: [{glyph, a: [codepoint], b: [codepoint]}], copiedToBackground?: number}",
            always,
            compare_glyphs
        ),
    ]
}

/// The comparison options of `source.diff` and `source.merge`.
fn options(p: &Params<'_>) -> Result<Options> {
    let parts = match p.opt_strings("parts")? {
        None => Part::ALL.into_iter().collect(),
        Some(names) => names
            .into_iter()
            .map(|n| {
                Part::parse(n)
                    .ok_or_else(|| p.bad(format!("unknown part `{n}`: the parts are glyphs, info, masters, instances, axes, kerning and features")))
            })
            .collect::<Result<BTreeSet<_>>>()?,
    };
    let glyphs = match p.opt_strings("glyphs")? {
        None => None,
        Some(names) if names.len() > fontcraft_model::MAX_GLYPHS => {
            return Err(p.bad(format!("`glyphs` names more than {} glyphs", fontcraft_model::MAX_GLYPHS)));
        }
        Some(names) => Some(names.into_iter().map(str::to_string).collect()),
    };
    let tolerance = tolerance(p)?.unwrap_or(0.0);
    Ok(Options { parts, glyphs, tolerance })
}

fn tolerance(p: &Params<'_>) -> Result<Option<f64>> {
    match p.opt_f64("tolerance")? {
        Some(t) if !(0.0..=MAX_TOLERANCE).contains(&t) => Err(p.bad(format!("`tolerance` must be from 0 to {MAX_TOLERANCE} units"))),
        t => Ok(t),
    }
}

fn summary(changes: &[Change], parts: &BTreeSet<Part>) -> Value {
    let mut m = Map::new();
    for part in parts {
        m.insert(part.name().into(), json!(changes.iter().filter(|c| c.part == *part).count()));
    }
    Value::Object(m)
}

/// A source read from `path`; compiled fonts are refused.
#[cfg(not(target_arch = "wasm32"))]
fn load_source(path: &std::path::Path) -> Result<Font> {
    let (font, _) = crate::io::open(path, None)?;
    if font.source.format == fontcraft_model::SourceFormat::Binary {
        return Err(EngineError::Io(format!(
            "{} is a compiled font: comparing and merging sources reads .glyphs, .glyphspackage, .fontcraft, UFO and designspace files (compare compiled fonts with font.diff)",
            path.display()
        )));
    }
    Ok(font)
}

#[cfg(not(target_arch = "wasm32"))]
fn diff(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("source.diff", p)?;
    let a = load_source(std::path::Path::new(p.str("a")?))?;
    let b = load_source(std::path::Path::new(p.str("b")?))?;
    let opts = options(&p)?;
    let d = compare::diff(&a, &b, &opts)?;
    let changes: Vec<Value> = d.changes.iter().map(Change::to_json).collect();
    Ok(json!({ "same": d.changes.is_empty(), "changes": changes, "summary": summary(&d.changes, &opts.parts) }))
}

#[cfg(not(target_arch = "wasm32"))]
fn merge(_: &mut Session, p: &Value) -> Result<Value> {
    use super::file_io::{OutputRequest, output_path};
    use crate::io::SaveFormat;
    let p = Params::new("source.merge", p)?;
    let base_path = std::path::PathBuf::from(p.str("base")?);
    let other_path = std::path::PathBuf::from(p.str("other")?);
    let take: Option<Vec<&str>> = match p.get("take") {
        Some(Value::String(s)) if s == "all" => None,
        Some(_) => Some(p.strings("take")?),
        None => return Err(p.bad("`take` is required: a list of change ids, or \"all\"")),
    };
    let opts = options(&p)?;
    let req = OutputRequest::from_params(&p)?;
    if req.output.is_none() && req.output_dir.is_none() && !req.in_place {
        return Err(p.bad("give `output`, `outputDir`, or `inPlace` to replace BASE"));
    }
    let out_path = output_path(&base_path, &req, ".merged", None)?;
    let base = load_source(&base_path)?;
    let other = load_source(&other_path)?;
    let format = SaveFormat::of_source(base.source.format)
        .ok_or_else(|| p.bad(format!("{} is in a format that cannot be written; convert it first", base_path.display())))?;
    if let Some(named) = SaveFormat::of_path(&out_path)
        && named != format
    {
        return Err(p.bad(format!(
            "the output {} names the {} format, but BASE is {}: the merge is written in BASE's format",
            out_path.display(),
            named.name(),
            format.name()
        )));
    }
    // A designspace writes its UFOs beside itself, where BASE's own UFOs are.
    if format == SaveFormat::Designspace && !req.in_place && out_path.parent() == base_path.parent() {
        return Err(p.bad("a merged designspace beside BASE would overwrite BASE's UFOs: write it to another folder, or pass inPlace"));
    }
    let d = compare::diff(&base, &other, &opts)?;
    let chosen: Vec<&Change> = match &take {
        None => d.changes.iter().collect(),
        Some(ids) => {
            let mut seen = BTreeSet::new();
            let mut out = Vec::new();
            for id in ids {
                if !seen.insert(*id) {
                    continue;
                }
                let c = d.changes.iter().find(|c| c.id == *id).ok_or_else(|| {
                    EngineError::Interaction(format!(
                        "change {id} is not among the changes from BASE to OTHER as they are now (one of them changed since it was listed, or the parts or glyphs differ): nothing was written"
                    ))
                })?;
                out.push(c);
            }
            out
        }
    };
    let merged = apply::apply(&base, &other, &d, &chosen)?;
    merged.check().map_err(|e| EngineError::Interaction(format!("the merged source would not be a consistent font ({e}): nothing was written")))?;
    if req.no_clobber && !req.in_place && out_path.exists() {
        return Err(EngineError::Io(format!("{} exists and noClobber is set", out_path.display())));
    }
    let saved = crate::io::save(&merged, &out_path, format)?;
    let applied: Vec<&str> = chosen.iter().map(|c| c.id.as_str()).collect();
    Ok(json!({ "applied": applied, "output": out_path.to_string_lossy(), "format": format.name(), "losses": saved.losses }))
}

#[cfg(target_arch = "wasm32")]
fn diff(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("comparing source files by path in the browser".into()))
}

#[cfg(target_arch = "wasm32")]
fn merge(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("merging source files by path in the browser".into()))
}

/// An open document named by title or path, else a file read from `r`; with the open
/// document's id.
fn font_ref(s: &Session, r: &str) -> Result<(Arc<Font>, Option<u64>)> {
    if let Some(d) = s.documents().iter().find(|d| d.title == r || d.path.as_deref().is_some_and(|p| p == std::path::Path::new(r))) {
        return Ok((d.font.clone(), Some(d.id)));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (font, _) = crate::io::open(std::path::Path::new(r), None)?;
        Ok((Arc::new(font), None))
    }
    #[cfg(target_arch = "wasm32")]
    Err(EngineError::Io(format!("no open document is called `{r}`")))
}

fn compare_glyphs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.compareGlyphs", p)?;
    let (a, a_doc) = font_ref(s, p.str("a")?)?;
    let (b, _) = font_ref(s, p.str("b")?)?;
    let mut what = What::default();
    match p.opt_strings("what")? {
        None => what = What { glyph_set: true, outlines: true, names: true },
        Some(items) => {
            for item in items {
                match item {
                    "glyphSet" => what.glyph_set = true,
                    "outlines" => what.outlines = true,
                    "names" => what.names = true,
                    "bitmaps" | "substitutions" | "positioning" => {
                        return Err(EngineError::Unsupported(format!("comparing {item} of two fonts")));
                    }
                    other => return Err(p.bad(format!("unknown comparison `{other}`: use glyphSet, outlines or names"))),
                }
            }
        }
    }
    let given = tolerance(&p)?;
    let near = match p.opt_str("outlineMatch")? {
        None => given.is_some(),
        Some("exact") => false,
        Some("near") => true,
        Some(other) => return Err(p.bad(format!("`outlineMatch` must be \"exact\" or \"near\", got \"{other}\""))),
    };
    let tol = if near { given.unwrap_or(DEFAULT_NEAR_TOLERANCE) } else { 0.0 };
    let inlined = p.bool_or("acceptInlinedComponents", false)?;
    let to_background = p.bool_or("differencesToBackground", false)?;
    if to_background && (a_doc.is_none() || a_doc != s.active().map(|d| d.id)) {
        return Err(p.bad("differencesToBackground needs `a` to be the active document"));
    }
    let report = compare::compare_glyphs(&a, &b, what, tol, inlined)?;
    let mut reply = json!({
        "same": report.only_a.is_empty() && report.only_b.is_empty() && report.outlines.is_empty() && report.names.is_empty(),
        "onlyInA": report.only_a,
        "onlyInB": report.only_b,
        "outlines": report.outlines.iter().map(|(g, m, ..)| json!({ "glyph": g, "master": m })).collect::<Vec<_>>(),
        "names": report.names.iter().map(|(g, x, y)| json!({ "glyph": g, "a": x, "b": y })).collect::<Vec<_>>(),
    });
    if to_background {
        let copied = s.edit("Compare Glyphs", |font| {
            let mut n = 0usize;
            for (glyph, _, master_a, j, master_b) in &report.outlines {
                let Some(shapes) = compare::glyph_at(&b, *j).and_then(|g| g.layer(master_b)).map(|l| l.shapes.clone()) else { continue };
                let Some(mut g) = font.glyphs.get_mut(glyph) else { continue };
                let Some(layer) = g.layer_mut(master_a) else { continue };
                layer.info.background.get_or_insert_with(Default::default).shapes = shapes;
                n += 1;
            }
            Ok(n)
        })?;
        if let Value::Object(m) = &mut reply {
            m.insert("copiedToBackground".into(), json!(copied));
        }
    }
    Ok(reply)
}
