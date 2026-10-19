//! `font.instantiate` (M12.20–M12.22; `fontcraft_tables::instance`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines. The command runs [`instantiate`]: a compiled font goes to the binary path
//! (M12.20, M12.21); any other input is a source, for [`instantiate_source`] (M12.22).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, a refusal on
//! the web. Each output is written to a temporary file beside its target as it is built; once
//! every instance is built they are moved into place, so a failure leaves existing files as
//! they were.

use std::path::{Path, PathBuf};

use fontcraft_tables::instance::{self, AxisLimit, InstanceOptions, NamedInstance};
use fontcraft_tables::outlines::{OutlineAction, OutlineOptions};
use fontcraft_tables::sfnt::{SfntFont, Tag, parse_tag, tag_str};
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Map, Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Seconds from 1904-01-01 (the `head` epoch) to 1970-01-01.
const HEAD_EPOCH_OFFSET: i64 = 2_082_844_800;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = binary_specs();
    v.extend(source_specs());
    v
}

/// `font.instantiate` on variable fonts: full instances (M12.20) and partial ones (M12.21).
fn binary_specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "font.instantiate",
        "Instantiate",
        [],
        None,
        "{input: path, face?: int, location?: {axis: number | {min?: number, default?: number, max?: number} | \"drop\"}, static?: bool, named?: regex, all?: bool, indices?: [int], to?: \"same\"|\"ttf\"|\"otf\"|\"ufo\", output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool, clamp?: bool, optimizeGvar?: bool, overlapFlag?: bool, removeOverlaps?: bool, ignoreOverlapErrors?: bool, updateNameTable?: bool, downgradeCff2?: bool, recalcBounds?: bool, round?: bool, timestamp?: \"keep\"|\"now\"|int, autohint?: bool, normalize?: bool, strict?: bool, ufoVersion?: 2|3, instanceDir?: path, layoutOnly?: bool, masterFinder?: string} instances a variable font: every axis pinned (a number, \"drop\" for the default, or static to pin the axes not named at their default) gives a static font; a range {min, default, max} (user values; a missing end keeps the axis end, default moves the default) narrows an axis and keeps it variable, and any axis left variable gives a partial instance, a smaller variable font with `static` false in its result (fonts with CFF2 or VARC outlines are refused); named, all and indices build fvar named instances instead, each named after its PostScript name; locations are clamped to the axis ranges; the output defaults to <name>.instance.<ext> beside the input; a designspace or other multi-master source gives static instances: at `location` (user values; axes not named at their default; clamped unless clamp is false) or the source's instances chosen by named, all or indices (default: every exporting one), written as UFOs (the default; to the file names the source records, or into instanceDir or outputDir; rounded unless round is false, PostScript-autohinted unless autohint is false, overlaps removed with removeOverlaps; ufoVersion default 3) or compiled with to ttf or otf as font.compile does; a glyph whose masters are not compatible is left out with a warning, or stops the run with strict → {results: [{output, location: {axis: number | {min, default, max}}, static: bool, name?, postscriptName?}], warnings?: [string]}",
        always,
        instantiate
    )
    .files(const { &[FileParam::read("input"), FileParam::write("output"), FileParam::write("outputDir"), FileParam::write("instanceDir")] })
    .writes_files()]
}

/// `font.instantiate` on designspace and multi-master sources (M12.22): the source part of the
/// one command above, so no command of its own.
fn source_specs() -> Vec<CommandSpec> {
    Vec::new()
}

/// `font.instantiate` with a source as input (M12.22): see [`source`].
fn instantiate_source(p: &Params<'_>, input: &Path, req: &OutputRequest) -> Result<Value> {
    source::instantiate(p, input, req)
}

/// One font to build: its limits and how its result is described and named.
struct Job {
    limits: Vec<(Tag, AxisLimit)>,
    named: Option<NamedInstance>,
}

fn instantiate(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("font.instantiate", params)?;
    let path = p.str("input")?;
    let req = OutputRequest::from_params(&p)?;
    let face = p.opt_int("face", 0, i64::from(u32::MAX))?.and_then(|f| u32::try_from(f).ok()).unwrap_or(0);
    // A UFO or a package is a folder, which no compiled font is.
    if Path::new(path).is_dir() {
        return instantiate_source(&p, Path::new(path), &req);
    }
    let input = file_io::read_input(Path::new(path))?;
    if !input.kind.is_font() {
        return instantiate_source(&p, &input.path, &req);
    }
    for key in ["autohint", "normalize", "strict", "ufoVersion", "instanceDir", "layoutOnly", "masterFinder"] {
        if p.get(key).is_some() {
            return Err(p.bad(format!("`{key}` applies to sources only, and {path} is a compiled font")));
        }
    }
    if !p.bool_or("clamp", true)? {
        return Err(p.bad("a compiled variable font cannot be extrapolated: locations are clamped to the axis ranges"));
    }
    if !p.bool_or("round", true)? {
        return Err(p.bad("the instance of a compiled font always has rounded coordinates"));
    }
    let extension = match (p.opt_str("to")?, input.kind) {
        (None | Some("same"), _) => None,
        (Some("ttf"), InputKind::TrueType) | (Some("otf"), InputKind::Cff) => None,
        (Some(other @ ("ttf" | "otf")), _) => {
            return Err(p.bad(format!("an instance keeps the outline format of its input: convert it to {other} afterwards")));
        }
        (Some("ufo"), _) => return Err(p.bad("`to: \"ufo\"` applies to sources only")),
        (Some(other), _) => return Err(p.bad(format!("`to` must be \"same\", \"ttf\" or \"otf\", got \"{other}\""))),
    };
    let font = fontcraft_tables::read(&input.bytes, face)?;
    let remove_overlaps = p.bool_or("removeOverlaps", false)?;
    let ignore_overlap_errors = p.bool_or("ignoreOverlapErrors", false)?;
    let options = InstanceOptions {
        limits: Vec::new(),
        update_names: p.bool_or("updateNameTable", false)?,
        overlap_flags: p.bool_or("overlapFlag", true)? && !remove_overlaps,
        downgrade_cff2: p.bool_or("downgradeCff2", false)?,
        recalc_bounds: p.bool_or("recalcBounds", true)?,
    };
    let _ = p.bool_or("optimizeGvar", true)?;
    let stamp = timestamp(&p)?;
    let jobs = jobs(&p, &font)?;
    if jobs.len() > 1 && req.output.is_some() {
        return Err(p.bad("`output` names one file, and this run builds several instances: give `outputDir`"));
    }
    if jobs.len() > 1 && req.in_place {
        return Err(p.bad("`inPlace` replaces one file, and this run builds several instances"));
    }

    let created = file_io::create_output_dir(&req)?;
    let mut staged: Vec<Staged> = Vec::with_capacity(jobs.len());
    let finish = Finish { remove_overlaps, ignore_overlap_errors, stamp, extension };
    let results = build_all(&p, &font, &input.path, &req, &options, &finish, jobs, &mut staged).and_then(|results| commit(&staged).map(|()| results));
    match results {
        Ok(results) => Ok(json!({ "results": results })),
        Err(e) => {
            discard(&staged);
            file_io::remove_created(&created);
            Err(e)
        }
    }
}

/// What is done to each instance after it is built.
struct Finish<'a> {
    remove_overlaps: bool,
    ignore_overlap_errors: bool,
    /// `head.modified`, in Unix seconds; `None` keeps it.
    stamp: Option<i64>,
    extension: Option<&'a str>,
}

/// Builds every job and stages its output; answers the result entries.
#[allow(clippy::too_many_arguments)]
fn build_all(
    p: &Params<'_>,
    font: &SfntFont,
    input: &Path,
    req: &OutputRequest,
    options: &InstanceOptions,
    finish: &Finish<'_>,
    jobs: Vec<Job>,
    staged: &mut Vec<Staged>,
) -> Result<Vec<Value>> {
    let mut outs: Vec<PathBuf> = Vec::with_capacity(jobs.len());
    let mut entries = Vec::with_capacity(jobs.len());
    for job in jobs {
        let mut result = instance::instantiate(font, &InstanceOptions { limits: job.limits.clone(), ..options.clone() })?;
        let is_static = result.get(*b"fvar").is_none();
        if finish.remove_overlaps && is_static {
            let outline = OutlineOptions {
                keep_hinting: true,
                ignore_errors: finish.ignore_overlap_errors,
                ..OutlineOptions::new(OutlineAction::RemoveOverlaps)
            };
            result = fontcraft_tables::outlines::repair(&result, &outline)?.font;
        }
        if let Some(t) = finish.stamp {
            set_modified(&mut result, t);
        }
        let bytes = fontcraft_tables::write(&result, &WriteOptions { order: TableOrder::Recommended })?;
        let out = match &job.named {
            Some(named) if req.output.is_none() && !req.in_place => named_path(input, req, named, finish.extension)?,
            _ => file_io::output_path(input, req, ".instance", finish.extension)?,
        };
        if outs.contains(&out) {
            return Err(p.bad(format!("two instances would both be written to {}", out.display())));
        }
        file_io::check_write(&out)?;
        if req.no_clobber && !req.in_place && out.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", out.display())));
        }
        staged.push(stage(&out, &bytes, staged.len())?);
        let mut entry = json!({ "output": out.display().to_string(), "location": location_json(&job.limits), "static": is_static });
        if let (Some(named), Some(obj)) = (&job.named, entry.as_object_mut()) {
            obj.insert("location".into(), tagged(&named.coordinates));
            if let Some(name) = &named.subfamily_name {
                obj.insert("name".into(), json!(name));
            }
            if let Some(ps) = &named.postscript_name {
                obj.insert("postscriptName".into(), json!(ps));
            }
        }
        outs.push(out);
        entries.push(entry);
    }
    Ok(entries)
}

/// The instances the parameters ask for: the fvar named instances chosen by `named`, `all` or
/// `indices`, else one instance at `location`.
fn jobs(p: &Params<'_>, font: &SfntFont) -> Result<Vec<Job>> {
    let pattern = p.opt_str("named")?;
    let all = p.bool_or("all", false)?;
    let indices = p.opt_array("indices")?;
    let chosen = usize::from(pattern.is_some()) + usize::from(all) + usize::from(indices.is_some());
    if chosen > 1 {
        return Err(p.bad("give one of `named`, `all` and `indices`"));
    }
    if chosen == 0 {
        let mut limits = location(p)?;
        if p.bool_or("static", false)? {
            for axis in fontcraft_tables::variations::fvar_axes(font)? {
                if !limits.iter().any(|(t, _)| *t == axis.tag) {
                    limits.push((axis.tag, AxisLimit::Drop));
                }
            }
        }
        return Ok(vec![Job { limits, named: None }]);
    }
    if p.get("location").is_some() {
        return Err(p.bad("`location` cannot be combined with `named`, `all` or `indices`: a named instance has its own location"));
    }
    let list = instance::named_instances(font)?;
    let picked: Vec<NamedInstance> = if let Some(source) = pattern {
        // The pattern may match anywhere in the subfamily or PostScript name.
        let re = fontcraft_glyphquery::Pattern::new(&format!(".*(?:{source}).*"), false).map_err(|e| p.bad(e.to_string()))?;
        list.into_iter()
            .filter(|n| n.subfamily_name.as_deref().is_some_and(|s| re.is_match(s)) || n.postscript_name.as_deref().is_some_and(|s| re.is_match(s)))
            .collect()
    } else if let Some(indices) = indices {
        let mut out = Vec::with_capacity(indices.len());
        for v in indices {
            let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad("`indices` holds zero-based whole numbers"))?;
            let n = list.get(i).ok_or_else(|| p.bad(format!("the font has {} named instances; there is no index {i}", list.len())))?;
            out.push(n.clone());
        }
        out
    } else {
        list
    };
    if picked.is_empty() {
        return Err(p.bad("no named instance matches"));
    }
    if picked.len() > fontcraft_tables::limits::MAX_NAMED_INSTANCES {
        return Err(p.bad(format!("one run builds at most {} instances", fontcraft_tables::limits::MAX_NAMED_INSTANCES)));
    }
    Ok(picked.into_iter().map(|n| Job { limits: n.coordinates.iter().map(|(t, v)| (*t, AxisLimit::Pin(*v))).collect(), named: Some(n) }).collect())
}

/// `location`: per axis a number (pin), "drop" (pin at the default) or {min, default, max}.
fn location(p: &Params<'_>) -> Result<Vec<(Tag, AxisLimit)>> {
    let Some(given) = p.opt_object("location")? else { return Ok(Vec::new()) };
    if given.len() > fontcraft_tables::limits::MAX_AXES {
        return Err(p.bad(format!("`location` names more than {} axes", fontcraft_tables::limits::MAX_AXES)));
    }
    let finite = |v: &Value, what: &str| v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad(format!("{what} must be a finite number")));
    let mut out = Vec::with_capacity(given.len());
    for (axis, v) in given {
        let tag = parse_tag(axis).map_err(|_| p.bad(format!("`{axis}` is not an axis tag")))?;
        let limit = match v {
            Value::String(s) if s == "drop" => AxisLimit::Drop,
            Value::Number(_) => AxisLimit::Pin(finite(v, &format!("the value of `{axis}`"))?),
            Value::Object(range) => {
                let part = |k: &str| range.get(k).map(|v| finite(v, &format!("`{axis}.{k}`"))).transpose();
                let (min, default, max) = (part("min")?, part("default")?, part("max")?);
                AxisLimit::Range { min: min.unwrap_or(f64::NEG_INFINITY), default, max: max.unwrap_or(f64::INFINITY) }
            }
            _ => return Err(p.bad(format!("the value of `{axis}` must be a number, \"drop\" or {{min, default, max}}"))),
        };
        out.push((tag, limit));
    }
    Ok(out)
}

fn tagged(values: &[(Tag, f64)]) -> Value {
    Value::Object(values.iter().map(|(t, v)| (tag_str(*t).trim_end().to_string(), json!(v))).collect())
}

fn location_json(limits: &[(Tag, AxisLimit)]) -> Value {
    let mut m = Map::new();
    for (tag, limit) in limits {
        let v = match limit {
            AxisLimit::Pin(v) => json!(v),
            AxisLimit::Drop => json!("drop"),
            AxisLimit::Range { min, default, max } => {
                let mut r = Map::new();
                if min.is_finite() {
                    r.insert("min".into(), json!(min));
                }
                if let Some(d) = default {
                    r.insert("default".into(), json!(d));
                }
                if max.is_finite() {
                    r.insert("max".into(), json!(max));
                }
                Value::Object(r)
            }
        };
        m.insert(tag_str(*tag).trim_end().to_string(), v);
    }
    Value::Object(m)
}

/// `<PostScript name>.<ext>` (else `<stem>-<subfamily>.<ext>`, spaces removed) in `outputDir`
/// or beside the input.
fn named_path(input: &Path, req: &OutputRequest, named: &NamedInstance, extension: Option<&str>) -> Result<PathBuf> {
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "instance".into());
    let base = match (&named.postscript_name, &named.subfamily_name) {
        (Some(ps), _) if !ps.trim().is_empty() => ps.clone(),
        (_, Some(sub)) => format!("{stem}-{}", sub.replace(' ', "")),
        _ => format!("{stem}-instance{}", named.index),
    };
    let base: String = base.chars().map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '-' } else { c }).collect();
    let ext = extension.map(str::to_string).or_else(|| input.extension().map(|e| e.to_string_lossy().into_owned())).unwrap_or_else(|| "ttf".into());
    let dir = req.output_dir.clone().or_else(|| input.parent().map(Path::to_path_buf)).unwrap_or_default();
    let path = dir.join(format!("{base}.{ext}"));
    if path == input {
        return Err(EngineError::Io(format!("the output {} would replace the input: give outputDir", path.display())));
    }
    Ok(path)
}

/// The `head.modified` time to set, in Unix seconds; `None` for "keep".
fn timestamp(p: &Params<'_>) -> Result<Option<i64>> {
    if p.get("timestamp").and_then(Value::as_str) == Some("keep") {
        return Ok(None);
    }
    super::compile::timestamp(p).map(Some)
}

fn set_modified(font: &mut SfntFont, unix: i64) {
    let stamp = unix.saturating_add(HEAD_EPOCH_OFFSET).max(0);
    if let Some(slot) = font.tables.get_mut(b"head").and_then(|h| h.get_mut(28..36)) {
        slot.copy_from_slice(&stamp.to_be_bytes());
    }
}

/// An output written to a temporary file beside its target.
// The browser writes no files, so nothing builds one there.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct Staged {
    temp: PathBuf,
    /// The file the temporary replaces: the output, or the file it links to.
    target: PathBuf,
    /// A folder (a UFO), which replaces an existing one by being swapped in for it.
    folder: bool,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl Staged {
    /// The temporary file or folder.
    pub(super) fn temp(&self) -> &Path {
        &self.temp
    }
}

/// Writes `bytes` to a new temporary file beside `out` (beside the file it links to, for a
/// link), with the permissions of the file it will replace. Both the file replaced and the
/// temporary must pass the write check before anything is created.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn stage(out: &Path, bytes: &[u8], n: usize) -> Result<Staged> {
    use std::io::Write as _;
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    let target = if out.is_symlink() { std::fs::canonicalize(out).map_err(|e| io(out, e))? } else { out.to_path_buf() };
    if target.is_dir() {
        return Err(EngineError::Io(format!("cannot write {}: it is a folder", target.display())));
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "instance".into());
    let temp = target.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    // `out` was checked by the caller; a link may lead elsewhere, so check where the write lands.
    file_io::check_write(&target)?;
    file_io::check_write(&temp)?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| io(&temp, e))?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(io(&temp, e));
    }
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    Ok(Staged { temp, target, folder: false })
}

/// Most files one staged folder holds.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FOLDER_FILES: usize = crate::io::MAX_PACKAGE_FILES;

/// Writes `files` (paths relative to the folder, `/`-separated) into a new temporary folder
/// beside `out`. An existing `out` must be a folder that `is_replaceable` accepts.
#[cfg(not(target_arch = "wasm32"))]
fn stage_folder(out: &Path, files: &std::collections::BTreeMap<String, Vec<u8>>, n: usize, is_replaceable: impl Fn(&Path) -> bool) -> Result<Staged> {
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    let target = if out.is_symlink() { std::fs::canonicalize(out).map_err(|e| io(out, e))? } else { out.to_path_buf() };
    if target.exists() && !(target.is_dir() && is_replaceable(&target)) {
        return Err(EngineError::Io(format!("cannot write {}: something that is not a UFO has that name", target.display())));
    }
    if files.len() > MAX_FOLDER_FILES {
        return Err(EngineError::Io(format!("cannot write {}: more than {MAX_FOLDER_FILES} files", target.display())));
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "instance".into());
    let temp = target.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    std::fs::create_dir(&temp).map_err(|e| io(&temp, e))?;
    let staged = Staged { temp, target, folder: true };
    let written = write_tree(&staged.temp, &staged.target, files);
    match written {
        Ok(()) => Ok(staged),
        Err(e) => {
            discard(std::slice::from_ref(&staged));
            Err(e)
        }
    }
}

/// Writes `files` under `root`, a new folder that stands in for `target` in messages.
#[cfg(not(target_arch = "wasm32"))]
fn write_tree(root: &Path, target: &Path, files: &std::collections::BTreeMap<String, Vec<u8>>) -> Result<()> {
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    for (rel, bytes) in files {
        let safe = !rel.is_empty() && !rel.starts_with('/') && !rel.contains('\\') && rel.split('/').all(|s| !s.is_empty() && s != "." && s != "..");
        if !safe {
            return Err(EngineError::Io(format!("cannot write {}: `{rel}` is not a path inside it", target.display())));
        }
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        std::fs::write(&path, bytes).map_err(|e| io(&path, e))?;
    }
    Ok(())
}

/// Moves every staged output into place. A staged folder is swapped in: the folder it replaces
/// is moved aside first, then removed. If one cannot be moved, the ones after it are discarded
/// and the error says which were written.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn commit(staged: &[Staged]) -> Result<()> {
    for (i, s) in staged.iter().enumerate() {
        let moved = if s.folder { swap_folder(s) } else { std::fs::rename(&s.temp, &s.target) };
        if let Err(e) = moved {
            discard(staged.get(i..).unwrap_or_default());
            let before = if i == 0 { String::new() } else { format!("; the {i} outputs before it were written") };
            return Err(EngineError::Io(format!("cannot write {}: {e}{before}", s.target.display())));
        }
    }
    Ok(())
}

/// Puts a staged folder in place of its target, which is set aside and removed once the new
/// folder is there; when the move fails, the old folder is put back.
#[cfg(not(target_arch = "wasm32"))]
fn swap_folder(s: &Staged) -> std::io::Result<()> {
    if std::fs::symlink_metadata(&s.target).is_err() {
        return std::fs::rename(&s.temp, &s.target);
    }
    let name = s.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "instance".into());
    let aside = s.target.with_file_name(format!(".{name}.{}.old", std::process::id()));
    std::fs::rename(&s.target, &aside)?;
    if let Err(e) = std::fs::rename(&s.temp, &s.target) {
        let _ = std::fs::rename(&aside, &s.target);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&aside);
    Ok(())
}

/// Removes the temporaries that are still there.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn discard(staged: &[Staged]) {
    for s in staged {
        if s.folder {
            let _ = std::fs::remove_dir_all(&s.temp);
        } else {
            let _ = std::fs::remove_file(&s.temp);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn stage(out: &Path, _: &[u8], _: usize) -> Result<Staged> {
    Err(EngineError::Unsupported(format!("writing {} in the browser", out.display())))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn commit(_: &[Staged]) -> Result<()> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub(super) fn discard(_: &[Staged]) {}

mod source;

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use fontcraft_tables::sfnt::Flavour;

    /// A TrueType variable font with no outlines: `head`, `maxp`, `hhea`, `hmtx` (one empty
    /// glyph), `OS/2`, a `name` table and an `fvar` with wght 100–400–900 and two named
    /// instances, Light (300, PostScript name "T-Light") and Bold (700).
    fn variable_font() -> Vec<u8> {
        let mut head = vec![0u8; 54];
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let mut maxp = vec![0u8; 6];
        maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&1u16.to_be_bytes());
        let mut hhea = vec![0u8; 36];
        hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
        let hmtx = vec![1, 244, 0, 0];
        let mut os2 = vec![0u8; 96];
        os2[0..2].copy_from_slice(&4u16.to_be_bytes());
        os2[4..6].copy_from_slice(&400u16.to_be_bytes());
        let names = [(256u16, "Weight"), (257, "Light"), (258, "Bold"), (259, "T-Light")];
        let mut name = Vec::new();
        let mut strings = Vec::new();
        for v in [0u16, names.len() as u16, (6 + 12 * names.len()) as u16] {
            name.extend_from_slice(&v.to_be_bytes());
        }
        for (id, text) in names {
            let bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
            for v in [3u16, 1, 0x409, id, bytes.len() as u16, strings.len() as u16] {
                name.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend(bytes);
        }
        name.extend(strings);
        // fvar 1.0: one axis record of 20 bytes, two instance records of 10 (with a PostScript
        // name ID; 0xFFFF for none).
        let mut fvar = Vec::new();
        for v in [1u16, 0, 16, 2, 1, 20, 2, 10] {
            fvar.extend_from_slice(&v.to_be_bytes());
        }
        fvar.extend_from_slice(b"wght");
        for v in [100i32, 400, 900] {
            fvar.extend_from_slice(&(v << 16).to_be_bytes());
        }
        fvar.extend_from_slice(&[0, 0, 1, 0]);
        for (sub, value, ps) in [(257u16, 300i32, 259u16), (258, 700, 0xFFFF)] {
            fvar.extend_from_slice(&sub.to_be_bytes());
            fvar.extend_from_slice(&[0, 0]);
            fvar.extend_from_slice(&(value << 16).to_be_bytes());
            fvar.extend_from_slice(&ps.to_be_bytes());
        }
        let mut font = SfntFont::new(Flavour::TrueType);
        for (tag, data) in
            [(*b"head", head), (*b"maxp", maxp), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"OS/2", os2), (*b"name", name), (*b"fvar", fvar)]
        {
            font.insert(tag, data);
        }
        fontcraft_tables::write(&font, &WriteOptions::default()).unwrap()
    }

    #[test]
    fn instantiate_writes_static_and_named_instances() {
        let dir = std::env::temp_dir().join(format!("fontcraft-instance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("V.ttf");
        std::fs::write(&input, variable_font()).unwrap();
        let before = std::fs::read(&input).unwrap();
        let mut s = Session::new();

        let r = s.execute("font.instantiate", &json!({ "input": input, "location": { "wght": 650 }, "timestamp": "keep" })).unwrap();
        let out = dir.join("V.instance.ttf");
        assert_eq!(r["results"][0]["output"], json!(out.display().to_string()), "the default output name");
        assert_eq!(r["results"][0]["static"], json!(true));
        let font = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
        assert!(font.get(*b"fvar").is_none(), "a static instance has no fvar");
        assert_eq!(font.get(*b"OS/2").unwrap()[4..6], 650u16.to_be_bytes(), "usWeightClass follows wght");
        assert_eq!(std::fs::read(&input).unwrap(), before, "the input is not changed");

        let r = s.execute("font.instantiate", &json!({ "input": input, "all": true, "outputDir": dir.join("all") })).unwrap();
        let results = r["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["output"], json!(dir.join("all").join("T-Light.ttf").display().to_string()), "named after the PostScript name");
        assert_eq!(results[0]["location"]["wght"], json!(300.0));
        assert_eq!(results[1]["output"], json!(dir.join("all").join("V-Bold.ttf").display().to_string()), "no PostScript name: stem and subfamily");
        let r = s.execute("font.instantiate", &json!({ "input": input, "named": "Bol", "outputDir": dir.join("one") })).unwrap();
        assert_eq!(r["results"][0]["name"], json!("Bold"));

        assert!(s.execute("font.instantiate", &json!({ "input": input, "location": { "opsz": 12 }, "static": true })).is_err(), "no such axis");
        assert!(
            s.execute("font.instantiate", &json!({ "input": input, "all": true, "output": dir.join("x.ttf") })).is_err(),
            "one output for two fonts"
        );
        assert!(s.execute("font.instantiate", &json!({ "input": input, "indices": [5] })).is_err(), "no such named instance");
        assert!(s.execute("font.instantiate", &json!({ "input": input, "location": { "wght": 500 }, "clamp": false })).is_err());
        let refused = s.execute("font.instantiate", &json!({ "input": input, "location": { "wght": 650 }, "noClobber": true }));
        assert!(refused.is_err(), "the output exists and noClobber is set");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_range_gives_a_partial_instance() {
        let dir = std::env::temp_dir().join(format!("fontcraft-instance-partial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("V.ttf");
        std::fs::write(&input, variable_font()).unwrap();
        let mut s = Session::new();
        let out = dir.join("P.ttf");
        let r = s
            .execute(
                "font.instantiate",
                &json!({ "input": input, "location": { "wght": { "min": 300, "default": 500, "max": 700 } }, "output": out }),
            )
            .unwrap();
        assert_eq!(r["results"][0]["static"], json!(false), "an axis left variable");
        assert_eq!(r["results"][0]["location"]["wght"], json!({ "min": 300.0, "default": 500.0, "max": 700.0 }));
        let font = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
        let axes = fontcraft_tables::variations::fvar_axes(&font).unwrap();
        assert_eq!(axes.len(), 1);
        assert_eq!((axes[0].minimum, axes[0].default, axes[0].maximum), (300.0, 500.0, 700.0), "the new range and default");
        let named: Vec<f64> = instance::named_instances(&font).unwrap().iter().map(|n| n.coordinates[0].1).collect();
        assert_eq!(named, [300.0, 700.0], "named instances inside the range are kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_instance_leaves_existing_outputs_as_they_were() {
        let dir = std::env::temp_dir().join(format!("fontcraft-instance-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("V.ttf");
        std::fs::write(&input, variable_font()).unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(out.join("V-Bold.ttf")).unwrap();
        std::fs::write(out.join("T-Light.ttf"), b"old").unwrap();
        let mut s = Session::new();
        let err = s.execute("font.instantiate", &json!({ "input": input, "all": true, "outputDir": out })).unwrap_err();
        assert!(err.to_string().contains("folder"), "{err}");
        assert_eq!(std::fs::read(out.join("T-Light.ttf")).unwrap(), b"old", "the first instance did not replace the file");
        let mut left: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["T-Light.ttf", "V-Bold.ttf"], "no temporary file is left");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
