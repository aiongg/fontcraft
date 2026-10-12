//! The engine's export stage (`docs/plans/M7.md` § 3 › Export-time parameters): the export-time
//! parameters that need the engine, run on a copy of the font before the compiler resolves it. In
//! order:
//!
//! 1. linked sources: Import Font and Import Master refreshed from their files ([`crate::linked`]),
//!    once per export ([`linked`]);
//! 2. PreFilter strings on the target master's layers, components still in place;
//! 3. Scale to UPM, through `fontcraft_fontinfo::scale`;
//! 4. Filter strings, after every composite of the target master's layers is decomposed.
//!
//! Every job built here is a static font, so PreFilter always runs. Filter and PreFilter strings
//! come from the font, the master and the instance, in that order, and all of them run; at most
//! [`fontcraft_fontinfo::MAX_FILTER_STRINGS`] of both together. Scale to UPM is one value: the
//! instance's, else the master's, else the font's. Each string runs through `filter.apply` in a
//! session of its own that holds the copy, so export and the Filter menu share one implementation
//! per filter. Locked glyphs and paths are unlocked in that copy first: a lock guards editing,
//! not export.
//!
//! Before Filter decomposes, each composite of the target master is given the anchors it would
//! have inherited from its components, so mark attachment survives the decomposition (not for
//! UFO-family sources, whose propagation follows their own filter list).
//!
//! A parameter that cannot be used is reported as a warning and skipped; a Scale to UPM that
//! cannot be applied refuses the export, since the font would come out at the wrong size. Export
//! Folder ([`export_folder`]) names the sub-folder of the destination the job's files go to.
//! The compiler's stage (glyph sets, names, the `kern` table) runs after resolving, in
//! `fontcraft_compile::export_params`.
//!
//! File › Export, `font.compile`, the CLI, the control channel and the oracle harness all build
//! through `cmd::compile::build`, which calls [`linked`] once and [`prepare`] for every job.
//!
//! Rows: `Filter`, `PreFilter`, the `filter.customParameter.*` rows, `export.instanceExportFolder`
//! (M7.10); `Import Font`, `Import Master` (M7.12); `Scale to UPM` (M7.4 scales, M7.10 runs it).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_compile::Options;
use fontcraft_geom::{Affine, Point};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::plist::{Num, Value as PValue};
use fontcraft_model::{Anchor, Font, MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, Params, Resolved, Shape, Target, Transform};
use serde_json::Value;

use crate::cmd::compile::Job;
use crate::{EngineError, Result};

const FILTER: &str = "Filter";
const PRE_FILTER: &str = "PreFilter";
const SCALE_TO_UPM: &str = "Scale to UPM";
const EXPORT_FOLDER: &str = "Export Folder";

/// Most nodes and components decomposition for Filter may write for one job, over all glyphs.
/// Each layer has [`MAX_OUTLINE_ELEMENTS`] of its own; glyphs past this keep their components.
const MAX_DECOMPOSED_ELEMENTS: usize = 20_000_000;

/// Where two design locations count as the same (as the model's resolution).
const SAME_LOCATION: f64 = 1e-9;

/// What the steps may read besides the font.
pub(crate) struct StepContext<'a> {
    /// The document's glyph database (filters that look up glyph data, decomposition).
    pub data: &'a Arc<GlyphData>,
}

/// `font` with its linked sources read from their files, once for a whole export: every job
/// shares the result. `path` is the document's file, which relative paths start from; `None` for
/// a document never saved.
pub(crate) fn linked<'f>(font: &'f Font, path: Option<&Path>, warnings: &mut Vec<String>) -> Result<Cow<'f, Font>> {
    Ok(match crate::linked::resolve(font, path, warnings)? {
        Some(linked) => Cow::Owned(linked),
        None => Cow::Borrowed(font),
    })
}

/// The font `job` is built from: `font` (already [`linked`]) itself when no step applies, else a
/// changed copy. Warnings name the parameters that could not be used.
pub(crate) fn prepare<'f>(font: &'f Font, job: &Job, ctx: &StepContext, warnings: &mut Vec<String>) -> Result<Cow<'f, Font>> {
    let mut font = Cow::Borrowed(font);
    // A target that is not a static instance at a master is left to the compiler to report.
    let Some(master) = target_master(&font, &job.target) else { return Ok(font) };
    let (pre, post, upm) = {
        let levels = levels(&font, &master, &job.target);
        let mut pre = filter_strings(&levels, PRE_FILTER, warnings);
        let mut post = filter_strings(&levels, FILTER, warnings);
        let total = pre.len().saturating_add(post.len());
        if total > fontcraft_fontinfo::MAX_FILTER_STRINGS {
            pre.truncate(fontcraft_fontinfo::MAX_FILTER_STRINGS);
            post.truncate(fontcraft_fontinfo::MAX_FILTER_STRINGS.saturating_sub(pre.len()));
            warnings.push(format!(
                "{} has {total} Filter and PreFilter parameters; only the first {} were run",
                job.label,
                fontcraft_fontinfo::MAX_FILTER_STRINGS
            ));
        }
        (usable(pre, warnings), usable(post, warnings), scale_target(&levels, warnings))
    };

    if !pre.is_empty() {
        font = Cow::Owned(run_filters(font.into_owned(), &master, &pre, warnings)?);
    }
    if let Some(upm) = upm.filter(|u| *u != font.units_per_em) {
        let scaled = fontcraft_fontinfo::scale::scale_font(&font, upm).map_err(|e| EngineError::BadParams {
            command: SCALE_TO_UPM.into(),
            message: format!("{} cannot be scaled to {upm} units per em: {e}", job.label),
        })?;
        font = Cow::Owned(scaled);
    }
    if !post.is_empty() {
        let decomposed = decompose_master(&font, &master, ctx.data, warnings);
        font = Cow::Owned(run_filters(decomposed, &master, &post, warnings)?);
    }
    Ok(font)
}

/// The id of the master whose layers `target` draws: the master itself, or the one a static
/// instance sits at. `None` for anything else.
fn target_master(font: &Font, target: &Target) -> Option<String> {
    match target {
        Target::Master(id) => font.master(id).ok().map(|m| m.id.clone()),
        Target::Instance(i) => {
            let inst = font.instances.get(*i).filter(|inst| inst.kind.is_none())?;
            let at = |m: &&fontcraft_model::Master| {
                let n = font.axes.len().max(m.location.len()).max(inst.location.len());
                (0..n).all(|a| {
                    let (x, y) = (fontcraft_model::axes::design_value(&m.location, a), fontcraft_model::axes::design_value(&inst.location, a));
                    (x - y).abs() <= SAME_LOCATION
                })
            };
            font.masters.iter().find(at).map(|m| m.id.clone())
        }
    }
}

/// The parameter lists of the font, the target master and the instance, in that order.
fn levels<'a>(font: &'a Font, master: &str, target: &Target) -> [Option<&'a Params>; 3] {
    let instance = match target {
        Target::Instance(i) => font.instances.get(*i).map(|inst| &inst.params),
        Target::Master(_) => None,
    };
    [Some(&font.info.params), font.masters.iter().find(|m| m.id == master).map(|m| &m.params), instance]
}

/// Every enabled `name` string of `levels`, in order. A value that is not text is skipped with a
/// warning; an empty one is skipped silently.
fn filter_strings(levels: &[Option<&Params>; 3], name: &'static str, warnings: &mut Vec<String>) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for params in levels.iter().flatten() {
        for value in params.all(name) {
            match value.as_str() {
                Some(text) if text.trim().is_empty() => {}
                Some(text) => out.push((name, text.to_string())),
                None => warnings.push(format!("a {name} parameter that is not text was not used")),
            }
        }
    }
    out
}

/// The Scale to UPM value that applies: the instance's, else the master's, else the font's.
fn scale_target(levels: &[Option<&Params>; 3], warnings: &mut Vec<String>) -> Option<u16> {
    let value = levels.iter().rev().flatten().find_map(|p| p.get(SCALE_TO_UPM))?;
    let upm = match value {
        PValue::Number(n) => match n.value() {
            Num::Int(i) => u16::try_from(i).ok(),
            Num::Float(f) if f.is_finite() && f.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(&f) => {
                // In range and whole, checked above.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(f as u16)
            }
            Num::Float(_) => None,
        },
        PValue::String(s) => s.as_str().trim().parse::<u16>().ok(),
        _ => None,
    };
    match upm.filter(|u| fontcraft_fontinfo::UPM_RANGE.contains(u)) {
        Some(u) => Some(u),
        None => {
            warnings.push(format!(
                "the {SCALE_TO_UPM} parameter was not used: its value must be a whole number from {} to {}",
                fontcraft_fontinfo::UPM_RANGE.start(),
                fontcraft_fontinfo::UPM_RANGE.end()
            ));
            None
        }
    }
}

// ── running filter strings ──────────────────────────────────────────────────────────────────────

/// The strings of `strings` that name a filter with an export string form; each other one is
/// skipped with a warning (Extrude, which has none, among them).
fn usable(strings: Vec<(&'static str, String)>, warnings: &mut Vec<String>) -> Vec<(&'static str, String)> {
    strings
        .into_iter()
        .filter(|(param, text)| match crate::cmd::filter::parse_filter_string(text) {
            Ok(_) => true,
            Err(e) => {
                warnings.push(format!("the {param} parameter \"{text}\" was not used: {e}"));
                false
            }
        })
        .collect()
}

/// `font` with each of `strings` run on the master layers of `master`, in order, through
/// `filter.apply`. A string that cannot be read or run is skipped with a warning.
fn run_filters(mut font: Font, master: &str, strings: &[(&'static str, String)], warnings: &mut Vec<String>) -> Result<Font> {
    unlock(&mut font, master);
    let mut s = crate::Session::new();
    s.push_document(font, None);
    let layers = [master.to_string()];
    for (param, text) in strings {
        match crate::cmd::filter::run_filter_string(&mut s, text, &layers) {
            Ok(result) => left_as_they_were(&result, param, text, warnings),
            Err(e) => warnings.push(format!("the {param} parameter \"{text}\" was not used: {e}")),
        }
    }
    Ok(s.font()?.clone())
}

/// Warns about the glyphs a filter run left as they were for a reason other than having no
/// layer for the master.
fn left_as_they_were(result: &Value, param: &str, text: &str, warnings: &mut Vec<String>) {
    let Some(skipped) = result.get("skipped").and_then(Value::as_array) else { return };
    let failed: Vec<(&str, &str)> = skipped
        .iter()
        .filter_map(|v| Some((v.get("glyph")?.as_str()?, v.get("reason").and_then(Value::as_str).unwrap_or(""))))
        .filter(|(_, reason)| !reason.contains("no such layer"))
        .collect();
    let Some((first, reason)) = failed.first() else { return };
    let more = failed.len().saturating_sub(1);
    let others = if more == 0 { String::new() } else { format!(" and {more} more glyphs") };
    warnings.push(format!("the {param} parameter \"{text}\" left `{first}`{others} as they were: {reason}"));
}

/// Unlocks every glyph, and the locked paths of the master layers of `master`.
fn unlock(font: &mut Font, master: &str) {
    let locked: Vec<usize> = font
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, g)| g.info.locked || g.layer(master).is_some_and(|l| l.shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.info.locked))))
        .map(|(i, _)| i)
        .collect();
    for i in locked {
        let Some(mut g) = font.glyphs.get_mut_at(i) else { continue };
        if g.info.locked {
            g.info.locked = false;
        }
        if let Some(layer) = g.layer_mut(master) {
            for shape in &mut layer.shapes {
                if let Shape::Path(p) = shape
                    && p.info.locked
                {
                    p.info.locked = false;
                }
            }
        }
    }
}

// ── decomposition before Filter ─────────────────────────────────────────────────────────────────

/// `font` as Filter strings see it: corner and smart components drawn, then every component of
/// the master layers of `master` replaced by the outlines it stands for, each composite keeping
/// the anchors it inherited. A glyph whose components cannot be resolved keeps them, with a
/// warning.
fn decompose_master(font: &Font, master: &str, data: &GlyphData, warnings: &mut Vec<String>) -> Font {
    let inherited = inherited_anchors(font, master, data, warnings);
    let mut out = fontcraft_compile::prepare_source(font, warnings).into_owned();
    let mut budget = MAX_DECOMPOSED_ELEMENTS;
    let mut changes: Vec<(usize, Vec<Shape>)> = Vec::new();
    let mut kept: Vec<(String, String)> = Vec::new();
    for (i, g) in out.glyphs.iter().enumerate() {
        let Some(layer) = g.layer(master) else { continue };
        if !layer.shapes.iter().any(|s| matches!(s, Shape::Component(_))) {
            continue;
        }
        if budget == 0 {
            kept.push((g.name.clone(), format!("the decomposed composites hold more than {MAX_DECOMPOSED_ELEMENTS} nodes in all")));
            continue;
        }
        let start = MAX_OUTLINE_ELEMENTS.min(budget);
        let mut layer_budget = start;
        let mut shapes = Vec::with_capacity(layer.shapes.len());
        let result = decompose(&out, &layer.shapes, master, Affine::IDENTITY, 0, &mut layer_budget, &mut shapes);
        budget = budget.saturating_sub(start.saturating_sub(layer_budget));
        match result {
            Ok(()) => changes.push((i, shapes)),
            Err(why) => kept.push((g.name.clone(), why)),
        }
    }
    if let Some((first, why)) = kept.first() {
        let more = kept.len().saturating_sub(1);
        let others = if more == 0 { String::new() } else { format!(" and {more} more glyphs") };
        warnings.push(format!("`{first}`{others} kept their components for the Filter parameters: {why}"));
    }
    for (i, shapes) in changes {
        let Some(mut g) = out.glyphs.get_mut_at(i) else { continue };
        let anchors = inherited.as_ref().and_then(|map| map.get(&g.name)).cloned();
        if let Some(layer) = g.layer_mut(master) {
            layer.shapes = shapes;
            if let Some(anchors) = anchors {
                layer.anchors = anchors;
            }
        }
    }
    out
}

/// The anchors each glyph of `master` has once its components pass theirs on, by glyph name;
/// `None` for a UFO-family source or when they cannot be worked out (with a warning).
fn inherited_anchors(font: &Font, master: &str, data: &GlyphData, warnings: &mut Vec<String>) -> Option<HashMap<String, Vec<Anchor>>> {
    if fontcraft_model::is_ufo_family(font.source.format) {
        return None;
    }
    match fontcraft_components::propagate::effective_anchors_for_master_with(font, Some(data), master) {
        Ok(m) => Some(m.anchors.into_iter().map(|(name, list)| (name, list.into_iter().map(|a| Anchor::new(a.name, a.x, a.y)).collect())).collect()),
        Err(e) => {
            warnings.push(format!("composites decomposed for the Filter parameters keep only their own anchors: {e}"));
            None
        }
    }
}

/// Adds `shapes` to `out` with every component replaced by its base glyph's shapes in `master`,
/// moved by `affine` (a path under a mirroring transform is reversed, as export decomposition
/// does). A component whose base is missing stays a component. Each node and each component takes
/// one from `budget`.
fn decompose(
    font: &Font,
    shapes: &[Shape],
    master: &str,
    affine: Affine,
    depth: usize,
    budget: &mut usize,
    out: &mut Vec<Shape>,
) -> std::result::Result<(), String> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(format!("components are nested more than {MAX_COMPONENT_DEPTH} deep, or in a cycle"));
    }
    let too_big = || format!("the outline would have more than {MAX_OUTLINE_ELEMENTS} nodes");
    for shape in shapes {
        match shape {
            Shape::Path(path) => {
                *budget = budget.checked_sub(path.nodes.len()).ok_or_else(too_big)?;
                let mut path = path.clone();
                if depth > 0 {
                    for n in &mut path.nodes {
                        let at = affine * Point::new(n.x, n.y);
                        n.x = at.x;
                        n.y = at.y;
                    }
                    if affine.determinant() < 0.0 {
                        path.reverse();
                    }
                }
                out.push(Shape::Path(path));
            }
            Shape::Component(c) => {
                *budget = budget.checked_sub(1).ok_or_else(too_big)?;
                match font.glyph(&c.glyph).and_then(|g| g.layer(master)) {
                    Some(base) => decompose(font, &base.shapes, master, affine * c.affine(), depth + 1, budget, out)?,
                    None => {
                        let mut c = c.clone();
                        if depth > 0 {
                            c.transform = Transform::Matrix((affine * c.affine()).as_coeffs());
                        }
                        out.push(Shape::Component(c));
                    }
                }
            }
            // A base glyph's images are not part of the composite's outline.
            other if depth == 0 => out.push(other.clone()),
            _ => {}
        }
    }
    Ok(())
}

// ── Export Folder ───────────────────────────────────────────────────────────────────────────────

/// The sub-folder of the destination the files of `resolved` go to (its Export Folder
/// parameter), relative, or `None` for the destination itself. An absolute path, more than
/// [`fontcraft_fontinfo::MAX_EXPORT_FOLDER_SEGMENTS`] segments or
/// [`fontcraft_fontinfo::MAX_EXPORT_FOLDER_BYTES`] bytes is refused.
pub(crate) fn export_folder(resolved: &Resolved, options: &Options) -> Result<Option<PathBuf>> {
    let _ = options;
    let Some(value) = resolved.params.get(EXPORT_FOLDER) else { return Ok(None) };
    let refused = |message: String| EngineError::BadParams { command: EXPORT_FOLDER.into(), message };
    let text = value.as_str().ok_or_else(|| refused("the value must be text".into()))?;
    folder_path(text).map_err(refused)
}

/// Reads an Export Folder value: `/`-separated folder names, `..` climbing out of the
/// destination, `.` and one trailing `/` ignored.
fn folder_path(text: &str) -> std::result::Result<Option<PathBuf>, String> {
    if text.len() > fontcraft_fontinfo::MAX_EXPORT_FOLDER_BYTES {
        return Err(format!("the value is longer than {} bytes", fontcraft_fontinfo::MAX_EXPORT_FOLDER_BYTES));
    }
    if text.trim().is_empty() {
        return Ok(None);
    }
    if text.starts_with('/') || text.starts_with('\\') || Path::new(text).is_absolute() {
        return Err(format!("\"{text}\" is an absolute path; give a folder relative to the export destination"));
    }
    let text = text.strip_suffix('/').unwrap_or(text);
    let segments: Vec<&str> = text.split('/').collect();
    if segments.len() > fontcraft_fontinfo::MAX_EXPORT_FOLDER_SEGMENTS {
        return Err(format!("\"{text}\" has more than {} folder levels", fontcraft_fontinfo::MAX_EXPORT_FOLDER_SEGMENTS));
    }
    let mut out = PathBuf::new();
    for segment in segments {
        if segment.is_empty() {
            return Err(format!("\"{text}\" has an empty folder name between two slashes"));
        }
        if let Some(c) = segment.chars().find(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
            return Err(format!("the folder name \"{segment}\" holds {c:?}, which folder names cannot hold"));
        }
        if segment != "." {
            out.push(segment);
        }
    }
    Ok(if out.as_os_str().is_empty() { None } else { Some(out) })
}

/// `name` (a `/`-separated path relative to the destination) inside `folder`.
pub(crate) fn in_folder(folder: &Path, name: &str) -> String {
    let mut out: Vec<String> = folder.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    out.push(name.to_string());
    out.join("/")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// `export.instanceExportFolder`: nested folders, `..` kept, absolute paths refused.
    #[test]
    fn export_folder_values_are_read_as_relative_paths() {
        let path = |t: &str| folder_path(t).unwrap().map(|p| in_folder(&p, "F.ttf"));
        assert_eq!(path("Trial Versions/Webfonts"), Some("Trial Versions/Webfonts/F.ttf".into()));
        assert_eq!(path("../Trial Versions/"), Some("../Trial Versions/F.ttf".into()));
        assert_eq!(path("./web"), Some("web/F.ttf".into()));
        assert_eq!(path(""), None);
        assert_eq!(path("."), None);
        for bad in ["/tmp/x", "\\x", "a//b", "a:b", "C:/fonts"] {
            assert!(folder_path(bad).is_err(), "{bad} is refused");
        }
        assert!(folder_path(&"a/".repeat(40)).is_err(), "too many levels");
        assert!(folder_path(&"a".repeat(2_000)).is_err(), "too long");
    }
}
