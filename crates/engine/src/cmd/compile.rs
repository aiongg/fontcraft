//! Building static fonts: `font.compile` and `font.compilePlan`, and the machinery the export
//! commands in `file.rs` share with them.
//!
//! A request names what to build (instances, masters), in which outline formats and containers,
//! and where the files go. Instances are built only where they sit exactly at a master; the others
//! are reported as skipped until interpolation exists (ROADMAP.md, M9). Files are named by the
//! instance's `fileName` parameter, else by the PostScript name the font gets.
//!
//! Nothing reaches the destination until every font is built: each file is first written under a
//! temporary name beside its final path, and all of them are renamed into place at the end. A
//! command that fails removes what it staged, so it leaves the destination as it was.

use base64::Engine as _;
use serde_json::{Map, Value, json};

use std::sync::Arc;

use fontcraft_compile::{AssetsRef, DataRef, Flavour, Options};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, ModelError, Resolved, Target};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most fonts (instances times outline formats) one command builds.
pub const MAX_FONTS: usize = 1_000;
/// Most bytes of font files one command returns in its result when it writes no files.
pub const MAX_RETURNED_BYTES: usize = 256 << 20;
/// Longest file name stem, in characters.
const MAX_STEM: usize = 200;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.compile",
            "Compile Fonts",
            [],
            None,
            "{formats?: [\"ttf\"|\"otf\"], containers?: [\"plain\"|\"woff\"|\"woff2\"], instances?: bool|[string], mastersAsInstances?: bool, destination?: string, outlines?: {keepDirection?: bool, conversionError?: number, removeOverlaps?: bool, flattenComponents?: bool, decomposeComponents?: bool, decomposeTransformedComponents?: bool}, timestamp?: number|\"now\"} build static fonts from the active document: every exportable instance (instances: true, the default; a font without instances builds its first master), the named instances, or none (false); mastersAsInstances adds one font per master. Instances between masters are skipped. Each font is built once per format (ttf: TrueType outlines, otf: CFF; default ttf) and written once per container (default plain). destination: a folder (files named by the instance's fileName parameter, else the PostScript name, with the document's file name standing in for a missing family name; with several formats each goes into a ttf or otf subfolder; an Export Folder parameter puts a font's files in that sub-folder, `..` climbing out of the destination; outside the desktop app a font whose files would land outside the destination is skipped), or a file path when exactly one file results; without it the fonts are built in memory and only described. conversionError: in em, default 0.001. removeOverlaps: merge overlapping contours in every glyph (default true; a glyph whose overlaps cannot be removed keeps them, with a warning). decomposeComponents: write every composite as contours (default false: only the composites the source's parameters and filter list name). decomposeTransformedComponents: write as contours every composite with a scaled, mirrored, slanted or rotated component (default: as the source's filter list says). flattenComponents: replace a component that points at a composite by the glyphs it reaches, so no composite is nested (default: as the source's filter list says). timestamp: seconds since the Unix epoch for the head table (default SOURCE_DATE_EPOCH, else now). Glyphs are written under their production names unless the Don't use Production Names parameter or Use Custom Naming is on; a name the font cannot hold is made valid, with a warning → {fonts: [{instance, source, format, glyphs, warnings, files: [{name, container, bytes, path?}]}], skipped: [{instance, source, reason}], warnings}",
            has_doc,
            compile
        ).files(const { &[FileParam::write("destination")] }).writes_files(),
        cmd!(
            query "font.compilePlan",
            "Describe Compile Plan",
            [],
            None,
            "{formats?, containers?, instances?, mastersAsInstances?, destination?, outlines?, sections?: [\"outputs\"|\"glyphOrder\"|\"productionNames\"|\"tables\"|\"instances\"]} what font.compile would build with the same parameters, without building or writing: per output the instance, format, PostScript name, glyph count, whether overlaps are removed, the files (paths with a destination, names without) and the tables; the final glyph order of the first output, by source name; the name each glyph of it is written under where that differs from its source name (its production name, made valid); each instance's master and location. sections: the parts to return (default all) → {outputs?: [{instance, source, format, postscriptName, style, glyphs, removeOverlap, files, tables?}], glyphOrder?, productionNames?: {source name: written name}, instances?: [{instance, source, master, location}], skipped, warnings}",
            has_doc,
            compile_plan
        ),
    ]
}

// ---------- the request ----------

/// What a container wraps the compiled font in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Container {
    Plain,
    Woff,
    Woff2,
}

impl Container {
    const ALL: [Container; 3] = [Container::Plain, Container::Woff, Container::Woff2];

    pub(super) fn name(self) -> &'static str {
        match self {
            Container::Plain => "plain",
            Container::Woff => "woff",
            Container::Woff2 => "woff2",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    fn extension(self, flavour: Flavour) -> &'static str {
        match self {
            Container::Plain => format_name(flavour),
            Container::Woff => "woff",
            Container::Woff2 => "woff2",
        }
    }

    fn wrap(self, sfnt: &[u8]) -> Result<Vec<u8>> {
        use fontcraft_container::{Error, Extras, woff, woff2};
        let wrapped = match self {
            Container::Plain => return Ok(sfnt.to_vec()),
            Container::Woff => woff::encode(sfnt, &Extras::default()),
            Container::Woff2 => woff2::encode(sfnt, &woff2::Options::default()),
        };
        wrapped.map_err(|e| match e {
            Error::Unsupported { format, what } => EngineError::Unsupported(format!("{format}: {what}")),
            other => EngineError::Internal(format!("wrapping the compiled font: {other}")),
        })
    }
}

/// The file extension of a plain font, and the format's name in `font.compile`.
pub(super) fn format_name(flavour: Flavour) -> &'static str {
    match flavour {
        Flavour::TrueType => "ttf",
        Flavour::Cff => "otf",
    }
}

/// The outline format's name in `file.exportStatic` and `file.exportAll`.
pub(super) fn outlines_name(flavour: Flavour) -> &'static str {
    match flavour {
        Flavour::TrueType => "truetype",
        Flavour::Cff => "cff",
    }
}

/// Where the files go.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Dest {
    /// Nowhere: the fonts stay in memory.
    Memory,
    /// Into this folder, created when missing.
    Folder(std::path::PathBuf),
    /// To this one file.
    File(std::path::PathBuf),
}

/// The font extension of `path` (`ttf`, `otf`, `woff`, `woff2`), lower case.
fn font_extension(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    ["ttf", "otf", "woff", "woff2"].into_iter().find(|e| *e == ext)
}

impl Dest {
    /// `destination`: an existing folder, a path ending in a font extension (one file), or a
    /// folder to create.
    pub(super) fn read(p: &Params) -> Result<Self> {
        let Some(d) = p.opt_str("destination")? else { return Ok(Dest::Memory) };
        if d.trim().is_empty() {
            return Err(p.bad("`destination` is empty"));
        }
        let path = std::path::PathBuf::from(d);
        Ok(if path.is_dir() || font_extension(&path).is_none() { Dest::Folder(path) } else { Dest::File(path) })
    }

    /// The extension of a one-file destination.
    pub(super) fn extension(&self) -> Option<&'static str> {
        match self {
            Dest::File(path) => font_extension(path),
            _ => None,
        }
    }
}

/// The containers asked for under `key` (default: the one a file destination's extension names,
/// else plain), without repeats.
pub(super) fn containers(p: &Params, key: &str, dest: &Dest) -> Result<Vec<Container>> {
    let implied = match dest.extension() {
        Some("woff") => Container::Woff,
        Some("woff2") => Container::Woff2,
        _ => Container::Plain,
    };
    let Some(names) = p.opt_strings(key)? else { return Ok(vec![implied]) };
    let mut out = Vec::new();
    for name in names {
        let c = Container::parse(name).ok_or_else(|| p.bad(format!("`{key}` holds \"plain\", \"woff\" or \"woff2\", got \"{name}\"")))?;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    if out.is_empty() {
        return Err(p.bad(format!("`{key}` names no container")));
    }
    Ok(out)
}

/// A one-file destination must agree with what is written to it.
pub(super) fn check_file_destination(p: &Params, dest: &Dest, flavours: &[Flavour], containers: &[Container]) -> Result<()> {
    let Some(ext) = dest.extension() else { return Ok(()) };
    let (&[flavour], &[container]) = (flavours, containers) else {
        return Err(p.bad("`destination` is one file, but this export writes several: give a folder"));
    };
    let wanted = container.extension(flavour);
    if ext != wanted {
        return Err(p.bad(format!(
            "`destination` ends in .{ext}, but this export writes a .{wanted} file ({} outlines, {} container)",
            outlines_name(flavour),
            container.name()
        )));
    }
    Ok(())
}

/// The head-table time: `timestamp` (seconds since the Unix epoch, or "now"), else
/// `SOURCE_DATE_EPOCH`, else the clock.
pub(super) fn timestamp(p: &Params) -> Result<i64> {
    if p.get("timestamp").and_then(Value::as_str) == Some("now") {
        return Ok(super::file::clock());
    }
    // Years 1904 to 9999, which is what the head table's date can sensibly hold.
    Ok(p.opt_int("timestamp", -2_082_844_800, 253_402_300_799)?.unwrap_or_else(super::file::default_timestamp))
}

/// Which instances to build.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Pick {
    /// Every exportable static instance; the first master when the font has no static instance.
    Exportable,
    /// These instances, by name (`#n` for an unnamed one), exported or not.
    Named(Vec<String>),
    /// No instance.
    None,
}

impl Pick {
    pub(super) fn read(p: &Params, key: &str) -> Result<Self> {
        match p.get(key) {
            None | Some(Value::Bool(true)) => Ok(Pick::Exportable),
            Some(Value::Bool(false)) => Ok(Pick::None),
            Some(_) => {
                let names = p.opt_strings(key)?.unwrap_or_default();
                if names.len() > MAX_FONTS {
                    return Err(p.bad(format!("`{key}` names more than {MAX_FONTS} instances")));
                }
                Ok(Pick::Named(names.into_iter().map(str::to_string).collect()))
            }
        }
    }
}

/// One font to build, before its formats.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Job {
    /// The instance's name (`#n` when it has none), or the master's.
    pub label: String,
    /// "instance", "master", or "default" (the first master, for a font without instances).
    pub source: &'static str,
    pub target: Target,
}

fn instance_label(i: usize, name: &str) -> String {
    if name.is_empty() { format!("#{i}") } else { name.to_string() }
}

/// The fonts `pick` and `masters` ask for, in the font's order: instances, then masters.
pub(super) fn jobs(font: &Font, pick: &Pick, masters: bool, p: &Params) -> Result<Vec<Job>> {
    let mut jobs = Vec::new();
    match pick {
        Pick::Exportable => {
            let statics: Vec<(usize, &fontcraft_model::Instance)> = font.instances.iter().enumerate().filter(|(_, i)| i.kind.is_none()).collect();
            if statics.is_empty() && !masters {
                let m = font.default_master()?;
                jobs.push(Job { label: m.name.clone(), source: "default", target: Target::Master(m.id.clone()) });
            }
            for (i, inst) in statics.into_iter().filter(|(_, inst)| inst.exports) {
                jobs.push(Job { label: instance_label(i, &inst.name), source: "instance", target: Target::Instance(i) });
            }
        }
        Pick::Named(names) => {
            for name in names {
                let found = font.instances.iter().enumerate().find(|(i, inst)| instance_label(*i, &inst.name) == *name);
                let Some((i, inst)) = found else { return Err(p.bad(format!("the font has no instance named `{name}`"))) };
                if inst.kind.is_some() {
                    return Err(p.bad(format!("`{name}` is a variable font setting, not a static instance")));
                }
                if !jobs.iter().any(|j: &Job| j.target == Target::Instance(i)) {
                    jobs.push(Job { label: name.clone(), source: "instance", target: Target::Instance(i) });
                }
            }
        }
        Pick::None => {}
    }
    if masters {
        jobs.extend(font.masters.iter().map(|m| Job { label: m.name.clone(), source: "master", target: Target::Master(m.id.clone()) }));
    }
    Ok(jobs)
}

/// What every export and compile command reads besides its selection.
pub(super) struct Request {
    pub flavours: Vec<Flavour>,
    pub containers: Vec<Container>,
    pub options: Options,
    pub dest: Dest,
    /// The destination is one file and no instance was named: build the first font that can be
    /// built and skip the rest.
    pub first_only: bool,
}

impl Request {
    fn fonts(&self, jobs: usize) -> usize {
        jobs.saturating_mul(self.flavours.len())
    }

    /// The file a font of `stem` and `flavour` is written to, relative to the destination folder:
    /// with several formats, each has a subfolder named by its extension.
    fn file_name(&self, stem: &str, flavour: Flavour, container: Container) -> String {
        let ext = container.extension(flavour);
        if self.flavours.len() > 1 { format!("{}/{stem}.{ext}", format_name(flavour)) } else { format!("{stem}.{ext}") }
    }

    /// `name` inside the job's Export Folder; a one-file destination is written where it says.
    fn placed(&self, folder: Option<&std::path::Path>, name: String) -> String {
        match folder {
            Some(folder) if !matches!(self.dest, Dest::File(_)) => crate::export_steps::in_folder(folder, &name),
            _ => name,
        }
    }
}

/// The file name stem of a font: its `fileName` parameter, else its PostScript name, else family
/// and style; characters that cannot be in a file name are dropped. `source` (the document's file
/// name without extension) stands in for a missing family name.
fn stem(resolved: &Resolved, options: &Options, source: &str) -> String {
    let wanted = resolved
        .params
        .get("fileName")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| fontcraft_compile::plan::file_stem(resolved, options, source))
        .unwrap_or_else(|| {
            let family = if resolved.family_name.trim().is_empty() { source } else { resolved.family_name.as_str() };
            format!("{family}-{}", resolved.style_name)
        });
    let clean: String =
        wanted.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).take(MAX_STEM).collect();
    let clean = clean.trim().trim_start_matches('.');
    if clean.is_empty() { "font".to_string() } else { clean.to_string() }
}

// ---------- building ----------

/// One file built.
pub(super) struct FileOut {
    pub name: String,
    pub container: Container,
    pub bytes: usize,
    pub path: Option<std::path::PathBuf>,
    /// The file itself, when it was not written.
    pub data: Option<Vec<u8>>,
}

/// One font built, in every container asked for.
pub(super) struct FontOut {
    pub job: Job,
    pub flavour: Flavour,
    pub glyph_order: Vec<String>,
    pub warnings: Vec<String>,
    pub files: Vec<FileOut>,
}

/// What a build did.
#[derive(Default)]
pub(super) struct Report {
    pub fonts: Vec<FontOut>,
    pub skipped: Vec<Value>,
    pub warnings: Vec<String>,
}

impl Report {
    pub(super) fn file_count(&self) -> usize {
        self.fonts.iter().map(|f| f.files.len()).sum()
    }

    pub(super) fn fonts_json(&self, with_data: bool) -> Vec<Value> {
        self.fonts.iter().map(|f| font_json(f, with_data)).collect()
    }
}

fn skip(job: &Job, reason: &str) -> Value {
    json!({ "instance": job.label, "source": job.source, "reason": reason })
}

const BETWEEN_MASTERS: &str = "it lies between masters, and building interpolated instances is not supported yet (ROADMAP.md, M9)";

fn font_json(f: &FontOut, with_data: bool) -> Value {
    let files: Vec<Value> = f
        .files
        .iter()
        .map(|file| {
            let mut v = json!({ "name": file.name, "container": file.container.name(), "bytes": file.bytes });
            if let (Some(path), Value::Object(m)) = (&file.path, &mut v) {
                m.insert("path".into(), json!(path.to_string_lossy()));
            }
            if let (true, Some(data), Value::Object(m)) = (with_data, &file.data, &mut v) {
                m.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(data)));
            }
            v
        })
        .collect();
    json!({
        "instance": f.job.label, "source": f.job.source, "format": format_name(f.flavour), "outlines": outlines_name(f.flavour),
        "glyphs": f.glyph_order.len(), "warnings": f.warnings, "files": files,
    })
}

/// Where built files go while a command runs.
pub(super) enum Sink {
    /// Kept in memory; `bytes` counts what is kept.
    Memory { bytes: usize, keep: bool },
    #[cfg(not(target_arch = "wasm32"))]
    Disk(Stage),
}

impl Sink {
    /// A sink for `dest`. `keep`: hold the files of a memory build (for the result), or only
    /// count them.
    /// `outside`: files may land outside a destination folder where an Export Folder parameter
    /// climbs out of it ([`crate::Session::export_outside`]).
    pub(super) fn new(dest: &Dest, keep: bool, outside: bool) -> Result<Self> {
        match dest {
            Dest::Memory => Ok(Sink::Memory { bytes: 0, keep }),
            #[cfg(not(target_arch = "wasm32"))]
            Dest::Folder(path) => Ok(Sink::Disk(Stage::new(path.clone(), false, outside))),
            #[cfg(not(target_arch = "wasm32"))]
            Dest::File(path) => Ok(Sink::Disk(Stage::new(path.clone(), true, outside))),
            #[cfg(target_arch = "wasm32")]
            _ => Err(EngineError::Unsupported("writing to a path in the browser; omit `destination` to get the fonts in the result".into())),
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    fn put(&mut self, name: String, container: Container, data: Vec<u8>, warnings: &mut Vec<String>) -> Result<FileOut> {
        let bytes = data.len();
        match self {
            Sink::Memory { bytes: total, keep } => {
                if !*keep {
                    return Ok(FileOut { name, container, bytes, path: None, data: None });
                }
                *total = total.saturating_add(bytes);
                if *total > MAX_RETURNED_BYTES {
                    return Err(EngineError::Unsupported(format!(
                        "returning more than {} MB of fonts in one result; give a `destination` to write them to files",
                        MAX_RETURNED_BYTES >> 20
                    )));
                }
                Ok(FileOut { name, container, bytes, path: None, data: Some(data) })
            }
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Disk(stage) => {
                let path = stage.put(&name, &data, warnings)?;
                Ok(FileOut { name, container, bytes, path: Some(path), data: None })
            }
        }
    }

    /// Whether the file `name` (relative to the destination) lands outside the destination folder,
    /// and whether that is refused.
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    fn outside(&self, name: &str) -> (bool, bool) {
        match self {
            Sink::Memory { .. } => (false, false),
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Disk(stage) => {
                let out = stage.outside(name);
                (out, out && !stage.outside_allowed)
            }
        }
    }

    /// Moves the staged files into place.
    pub(super) fn commit(self) -> Result<()> {
        match self {
            Sink::Memory { .. } => Ok(()),
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Disk(stage) => stage.commit(),
        }
    }
}

/// Builds `jobs` of `font` into `sink`, adding to `report`. `source` is the document's title (its
/// file name without extension), which names the files of a font without a family name. `data` is
/// the document's glyph data, which gives production names and categories.
#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    font: &Font,
    source: &str,
    path: Option<&std::path::Path>,
    data: Arc<GlyphData>,
    jobs: &[Job],
    req: &Request,
    sink: &mut Sink,
    report: &mut Report,
) -> Result<()> {
    font.check()?;
    let linked = crate::export_steps::linked(font, path, &mut report.warnings)?;
    let font: &Font = &linked;
    let ctx = crate::export_steps::StepContext { data: &data };
    let options = Options { glyph_data: DataRef(Some(data.clone())), ..req.options.clone() };
    let prepared = fontcraft_compile::prepare_source(font, &mut report.warnings);
    for job in jobs {
        if req.first_only && !report.fonts.is_empty() {
            report.skipped.push(skip(job, "the destination is one file: give a folder to export every instance"));
            continue;
        }
        // Propagation reads the job's source as stored; the drawing reads it prepared.
        let source_job = crate::export_steps::prepare(font, job, &ctx, &mut report.warnings)?;
        let font: &Font = &source_job;
        let prepared_job = match &source_job {
            std::borrow::Cow::Borrowed(_) => std::borrow::Cow::Borrowed(&*prepared),
            std::borrow::Cow::Owned(f) => fontcraft_compile::prepare_source(f, &mut report.warnings),
        };
        let resolved = match fontcraft_compile::drawing::resolve(&prepared_job, &job.target) {
            Err(ModelError::BetweenMasters(_)) => {
                report.skipped.push(skip(job, BETWEEN_MASTERS));
                continue;
            }
            other => other?,
        };
        let stem = stem(&resolved, &options, source);
        let folder = match crate::export_steps::export_folder(&resolved, &options) {
            Ok(folder) => folder,
            Err(e) => {
                report.skipped.push(skip(job, &e.to_string()));
                continue;
            }
        };
        let names = |flavour: Flavour, container: Container| req.placed(folder.as_deref(), req.file_name(&stem, flavour, container));
        let refused = req.flavours.iter().flat_map(|&f| req.containers.iter().map(move |&c| (f, c))).find(|&(f, c)| sink.outside(&names(f, c)).1);
        if let Some((f, c)) = refused {
            let reason = format!(
                "the Export Folder parameter puts {} outside the export destination; only an export run in the app may write there",
                names(f, c)
            );
            report.skipped.push(skip(job, &reason));
            continue;
        }
        for &flavour in &req.flavours {
            let compiled = fontcraft_compile::compile_resolved_from(font, resolved.clone(), flavour, &options)?;
            let mut files = Vec::with_capacity(req.containers.len());
            for &container in &req.containers {
                let data = container.wrap(&compiled.bytes)?;
                let name = names(flavour, container);
                let outside = sink.outside(&name).0;
                let file = sink.put(name, container, data, &mut report.warnings)?;
                if outside {
                    let at = file.path.as_ref().map_or_else(|| file.name.clone(), |p| p.to_string_lossy().into_owned());
                    report.warnings.push(format!("{at} is written outside the export destination, as the Export Folder parameter asks"));
                }
                files.push(file);
            }
            report.fonts.push(FontOut { job: job.clone(), flavour, glyph_order: compiled.glyph_order, warnings: compiled.warnings, files });
        }
    }
    Ok(())
}

/// Refuses a font with an exported glyph whose name breaks the naming rules, naming the glyph and
/// the position of the bad character (`glyphName.invalidBlocksExport`). File › Export refuses
/// such a font; `font.compile` writes a valid name with a warning instead.
pub(super) fn check_glyph_names(font: &Font) -> Result<()> {
    for g in font.glyphs.iter().filter(|g| g.export) {
        if let Err(e) = fontcraft_model::glyphdata::validate(&g.name, true) {
            return Err(fontcraft_compile::CompileError::Glyph { glyph: g.name.clone(), message: format!("illegal glyph name: {e}") }.into());
        }
    }
    Ok(())
}

/// Refuses a request that would build more than [`MAX_FONTS`] fonts.
pub(super) fn check_size(req: &Request, jobs: usize) -> Result<()> {
    let n = req.fonts(jobs);
    if n > MAX_FONTS {
        return Err(EngineError::Unsupported(format!("building {n} fonts in one command (at most {MAX_FONTS}); choose fewer instances or formats")));
    }
    Ok(())
}

// ---------- staging files on disk ----------

/// Files written under temporary names, renamed into place by [`Stage::commit`]. Dropping a stage
/// that was not committed removes its temporary files and the folders it created.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct Stage {
    /// Absolute, without `.` or `..`, so that an Export Folder's `..` climbs out of the folder the
    /// user named whatever text named it.
    root: std::path::PathBuf,
    /// `root` is the one file to write, not a folder.
    single: bool,
    /// Files may land outside `root` ([`Sink::new`]).
    outside_allowed: bool,
    /// (temporary, final) paths.
    staged: Vec<(std::path::PathBuf, std::path::PathBuf)>,
    /// Folders this stage created, outermost first.
    made: Vec<std::path::PathBuf>,
}

/// Whether two output paths can name the same file: some file systems ignore case.
#[cfg(not(target_arch = "wasm32"))]
fn same_output(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// `path` from the root of the file system, with `.` dropped and `..` taking off the folder before
/// it (by the text, as the Export Folder's `..` is read). Left as given when the working folder
/// is unknown.
#[cfg(not(target_arch = "wasm32"))]
fn absolute(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let Ok(full) = std::path::absolute(path) else { return path.to_path_buf() };
    let mut out = std::path::PathBuf::new();
    for c in full.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The folder that holds `path`.
#[cfg(not(target_arch = "wasm32"))]
fn folder_of(path: &std::path::Path) -> std::path::PathBuf {
    path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new(".")).to_path_buf()
}

/// How many names [`scratch_path`] tries before giving up.
#[cfg(not(target_arch = "wasm32"))]
const SCRATCH_TRIES: usize = 16;

/// A path in `dir` for a temporary (`kind` "tmp") or set-aside ("bak") file of an export. The
/// name never depends on the final file name, so two long names cannot shorten to the same one.
#[cfg(not(target_arch = "wasm32"))]
fn scratch_path(dir: &std::path::Path, kind: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".fontcraft-export-{}-{n}.{kind}", std::process::id()))
}

/// Creates a new temporary file in `dir`, never opening one that exists.
#[cfg(not(target_arch = "wasm32"))]
fn create_scratch(dir: &std::path::Path) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    for _ in 0..SCRATCH_TRIES {
        let path = scratch_path(dir, "tmp");
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

/// Moves `target` to a free set-aside name next to it; returns that name.
#[cfg(not(target_arch = "wasm32"))]
fn set_aside(target: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let dir = folder_of(target);
    for _ in 0..SCRATCH_TRIES {
        let backup = scratch_path(&dir, "bak");
        if std::fs::symlink_metadata(&backup).is_err() {
            std::fs::rename(target, &backup)?;
            return Ok(backup);
        }
    }
    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

#[cfg(not(target_arch = "wasm32"))]
impl Stage {
    fn new(root: std::path::PathBuf, single: bool, outside_allowed: bool) -> Self {
        Self { root: absolute(&root), single, outside_allowed, staged: Vec::new(), made: Vec::new() }
    }

    /// Where the file `name` (relative to the destination folder) goes. `..` (an Export Folder
    /// above the destination) is resolved here, so no folder is created on the way up.
    fn target(&self, name: &str) -> std::path::PathBuf {
        if self.single {
            return self.root.clone();
        }
        name.split('/').fold(self.root.clone(), |mut p, part| {
            match part {
                ".." => {
                    p.pop();
                }
                "" | "." => {}
                part => p.push(part),
            }
            p
        })
    }

    /// Whether the file `name` lands outside the destination folder.
    fn outside(&self, name: &str) -> bool {
        !self.single && !self.target(name).starts_with(&self.root)
    }

    fn put(&mut self, name: &str, bytes: &[u8], warnings: &mut Vec<String>) -> Result<std::path::PathBuf> {
        use std::io::Write as _;
        let target = self.target(name);
        if !self.outside_allowed && self.outside(name) {
            return Err(EngineError::Io(format!("{} is outside the export destination {}", target.display(), self.root.display())));
        }
        if let Some(i) = self.staged.iter().position(|(_, f)| same_output(f, &target)) {
            let (tmp, _) = self.staged.remove(i);
            let _ = std::fs::remove_file(tmp);
            warnings.push(format!(
                "two fonts are written to {}: the later one is kept (give instances a fileName parameter to keep both)",
                target.display()
            ));
        }
        if target.file_name().is_none() {
            return Err(EngineError::Io(format!("{} is not a file path", target.display())));
        }
        let dir = folder_of(&target);
        self.make_dir(&dir)?;
        let cannot = |e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", target.display()));
        let (tmp, mut f) = create_scratch(&dir).map_err(cannot)?;
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(cannot(e));
        }
        self.staged.push((tmp, target.clone()));
        Ok(target)
    }

    /// Creates `dir` and the folders above it that are missing, remembering each.
    fn make_dir(&mut self, dir: &std::path::Path) -> Result<()> {
        if dir.is_dir() {
            return Ok(());
        }
        let missing: Vec<&std::path::Path> = dir.ancestors().take_while(|d| !d.as_os_str().is_empty() && !d.exists()).collect();
        for d in missing.into_iter().rev() {
            std::fs::create_dir(d).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", d.display())))?;
            self.made.push(d.to_path_buf());
        }
        if !dir.is_dir() {
            return Err(EngineError::Io(format!("{} is not a folder", dir.display())));
        }
        Ok(())
    }

    fn commit(mut self) -> Result<()> {
        self.check()?;
        self.swap()?;
        self.made.clear();
        Ok(())
    }

    /// Refuses, before anything moves, a final path that cannot take a file. Each temporary file
    /// was written next to its final path, so that folder exists and takes new files.
    fn check(&self) -> Result<()> {
        for (tmp, target) in &self.staged {
            if target.is_dir() {
                return Err(EngineError::Io(format!("cannot write {}: a folder has that name", target.display())));
            }
            if !tmp.is_file() {
                return Err(EngineError::Io(format!("cannot write {}: its temporary file is gone", target.display())));
            }
        }
        Ok(())
    }

    /// Sets every existing final file aside, then moves every staged file into place. On a
    /// failure it removes what it moved in and puts back what it set aside, so the folder is as
    /// it was; on success it deletes what it set aside.
    fn swap(&mut self) -> Result<()> {
        let mut aside: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
        let mut failure = None;
        for (_, target) in &self.staged {
            if std::fs::symlink_metadata(target).is_err() {
                continue;
            }
            match set_aside(target) {
                Ok(backup) => aside.push((backup, target.clone())),
                Err(e) => {
                    failure = Some(format!("cannot replace {}: {e}", target.display()));
                    break;
                }
            }
        }
        let mut moved = 0usize;
        if failure.is_none() {
            for (tmp, target) in &self.staged {
                if let Err(e) = std::fs::rename(tmp, target) {
                    failure = Some(format!("cannot write {}: {e}", target.display()));
                    break;
                }
                moved += 1;
            }
        }
        let Some(failure) = failure else {
            for (backup, _) in &aside {
                let _ = std::fs::remove_file(backup);
            }
            self.staged.clear();
            return Ok(());
        };
        for (_, target) in self.staged.drain(..moved) {
            let _ = std::fs::remove_file(target);
        }
        let mut lost = Vec::new();
        for (backup, target) in aside.iter().rev() {
            if let Err(e) = std::fs::rename(backup, target) {
                lost.push(format!("the previous {} could not be put back ({e}) and is kept as {}", target.display(), backup.display()));
            }
        }
        if lost.is_empty() { Err(EngineError::Io(failure)) } else { Err(EngineError::Io(format!("{failure}; {}", lost.join("; ")))) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Stage {
    fn drop(&mut self) {
        for (tmp, _) in &self.staged {
            let _ = std::fs::remove_file(tmp);
        }
        for d in self.made.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod stage_tests {
    #![allow(clippy::unwrap_used)]
    use super::Stage;

    #[test]
    fn a_failed_move_puts_back_the_files_it_replaced() {
        let dir = std::env::temp_dir().join(format!("fontcraft-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("A.ttf"), b"old").unwrap();
        let mut stage = Stage::new(dir.clone(), false, false);
        let mut warnings = Vec::new();
        stage.put("A.ttf", b"new", &mut warnings).unwrap();
        stage.put("sub/B.ttf", b"new", &mut warnings).unwrap();
        // The second file's folder disappears after staging, so its move fails after `A.ttf`
        // was already replaced.
        std::fs::remove_dir_all(dir.join("sub")).unwrap();
        assert!(stage.swap().is_err());
        drop(stage);
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["A.ttf"]);
        assert_eq!(std::fs::read(dir.join("A.ttf")).unwrap(), b"old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A destination typed as `.` or with `..` in it is the folder it names: an Export Folder's
    /// `..` climbs out of that folder, not out of the typed text.
    #[test]
    fn export_folder_climbs_out_of_the_folder_a_relative_destination_names() {
        let cwd = std::env::current_dir().unwrap();
        let up = cwd.parent().unwrap().to_path_buf();
        let stage = Stage::new(".".into(), false, false);
        assert_eq!(stage.target("../Trial/F.ttf"), up.join("Trial/F.ttf"));
        assert!(stage.outside("../Trial/F.ttf"));
        assert!(!stage.outside("web/F.ttf"));
        let name = cwd.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!stage.outside(&format!("../{name}/F.ttf")), "back into the destination is inside it");
        let stage = Stage::new(std::path::Path::new("a/../b").into(), false, false);
        assert_eq!(stage.target("../F.ttf"), cwd.join("F.ttf"));
    }
}

// ---------- font.compile and font.compilePlan ----------

/// The formats of `font.compile`: `ttf` and `otf` are built; the others are named so a caller
/// learns when they arrive.
fn formats(p: &Params) -> Result<Vec<Flavour>> {
    let Some(names) = p.opt_strings("formats")? else { return Ok(vec![Flavour::TrueType]) };
    let mut out = Vec::new();
    for name in names {
        let flavour = match name {
            "ttf" => Flavour::TrueType,
            "otf" => Flavour::Cff,
            "otf-cff2" | "variable" | "variable-cff2" | "ttf-interpolatable" | "otf-interpolatable" => {
                return Err(EngineError::Unsupported(format!("building {name} fonts (ROADMAP.md, M9)")));
            }
            other => return Err(p.bad(format!("`formats` holds \"ttf\" or \"otf\", got \"{other}\""))),
        };
        if !out.contains(&flavour) {
            out.push(flavour);
        }
    }
    if out.is_empty() {
        return Err(p.bad("`formats` names no format"));
    }
    Ok(out)
}

/// The `outlines` group of `font.compile`, into `options`.
fn outline_options(p: &Params, options: &mut Options) -> Result<()> {
    let Some(group) = p.opt_object("outlines")? else { return Ok(()) };
    let flag = |key: &str| -> Result<Option<bool>> {
        match group.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(p.bad(format!("`outlines.{key}` must be true or false"))),
        }
    };
    options.keep_direction = flag("keepDirection")?.unwrap_or(false);
    if let Some(v) = group.get("conversionError").filter(|v| !v.is_null()) {
        let e = v.as_f64().filter(|e| e.is_finite() && *e > 0.0 && *e <= 1.0);
        options.conversion_error = e.ok_or_else(|| p.bad("`outlines.conversionError` must be a number above 0 and at most 1 (em)"))?;
    }
    if let Some(key) = group.keys().find(|k| {
        !matches!(
            k.as_str(),
            "keepDirection" | "conversionError" | "removeOverlaps" | "flattenComponents" | "decomposeComponents" | "decomposeTransformedComponents"
        )
    }) {
        return Err(EngineError::Unsupported(format!("`outlines.{key}`")));
    }
    options.remove_overlap = flag("removeOverlaps")?.unwrap_or(true);
    options.components.decompose_all = flag("decomposeComponents")?.unwrap_or(false);
    options.components.decompose_transformed = flag("decomposeTransformedComponents")?;
    options.components.flatten = flag("flattenComponents")?;
    Ok(())
}

/// The parts of a `font.compile` request shared with `font.compilePlan`.
fn compile_request(s: &Session, p: &Params) -> Result<(Vec<Job>, Request)> {
    let font = s.font()?;
    let flavours = formats(p)?;
    let dest = Dest::read(p)?;
    let containers = containers(p, "containers", &dest)?;
    let mut options = Options { glyph_data: DataRef(Some(s.glyph_data()?)), assets: AssetsRef(Some(s.assets())), ..Options::default() };
    outline_options(p, &mut options)?;
    let pick = Pick::read(p, "instances")?;
    let jobs = jobs(font, &pick, p.bool_or("mastersAsInstances", false)?, p)?;
    if jobs.is_empty() {
        return Err(p.bad("nothing to build: `instances` is false and `mastersAsInstances` is not true"));
    }
    if dest.extension().is_some() {
        check_file_destination(p, &dest, &flavours, &containers)?;
    }
    let req = Request { flavours, containers, options, dest, first_only: false };
    check_size(&req, jobs.len())?;
    Ok((jobs, req))
}

fn compile(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.compile", p)?;
    let (jobs, mut req) = compile_request(s, &p)?;
    req.options.timestamp = timestamp(&p)?;
    let doc = s.doc()?;
    let mut report = Report::default();
    let mut sink = Sink::new(&req.dest, false, s.export_outside())?;
    build(&doc.font, &doc.title, doc.path.as_deref(), s.glyph_data()?, &jobs, &req, &mut sink, &mut report)?;
    if matches!(req.dest, Dest::File(_)) && report.file_count() > 1 {
        return Err(p.bad(format!("`destination` is one file, but {} fonts were built: give a folder", report.fonts.len())));
    }
    if report.fonts.is_empty() {
        return Err(nothing_built(&report));
    }
    sink.commit()?;
    Ok(json!({ "fonts": report.fonts_json(false), "skipped": report.skipped, "warnings": report.warnings }))
}

/// The error of a command that built nothing: why each font was skipped.
pub(super) fn nothing_built(report: &Report) -> EngineError {
    let reasons: Vec<String> = report
        .skipped
        .iter()
        .map(|v| format!("{}: {}", v.get("instance").and_then(Value::as_str).unwrap_or("?"), v.get("reason").and_then(Value::as_str).unwrap_or("?")))
        .collect();
    EngineError::Unsupported(format!("nothing could be built ({})", reasons.join("; ")))
}

fn compile_plan(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.compilePlan", p)?;
    let (jobs, req) = compile_request(s, &p)?;
    const SECTIONS: [&str; 5] = ["outputs", "glyphOrder", "productionNames", "tables", "instances"];
    let sections: Vec<&str> = match p.opt_strings("sections")? {
        None => SECTIONS.to_vec(),
        Some(list) => {
            for name in &list {
                match *name {
                    n if SECTIONS.contains(&n) => {}
                    "features" => return Err(EngineError::Unsupported(format!("the `{name}` section of a compile plan"))),
                    other => return Err(p.bad(format!("`sections` holds {}, got \"{other}\"", SECTIONS.join(", ")))),
                }
            }
            list
        }
    };
    let want = |name: &str| sections.contains(&name);
    let data = s.glyph_data()?;
    let doc = s.doc()?;
    let (font, source) = (&doc.font, doc.title.as_str());
    font.check()?;
    let mut warnings: Vec<String> = Vec::new();
    let linked = crate::export_steps::linked(font, doc.path.as_deref(), &mut warnings)?;
    let font: &Font = &linked;
    let ctx = crate::export_steps::StepContext { data: &data };
    let (mut outputs, mut instances, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    let prepared = fontcraft_compile::prepare_source(font, &mut warnings);
    let mut glyph_order: Option<Vec<String>> = None;
    let mut production_names: Option<Map<String, Value>> = None;
    for job in &jobs {
        // Propagation reads the job's source as stored; the drawing reads it prepared.
        let source_job = crate::export_steps::prepare(font, job, &ctx, &mut warnings)?;
        let font: &Font = &source_job;
        let prepared_job = match &source_job {
            std::borrow::Cow::Borrowed(_) => std::borrow::Cow::Borrowed(&*prepared),
            std::borrow::Cow::Owned(f) => fontcraft_compile::prepare_source(f, &mut warnings),
        };
        let resolved = match fontcraft_compile::drawing::resolve(&prepared_job, &job.target) {
            Err(ModelError::BetweenMasters(_)) => {
                skipped.push(skip(job, BETWEEN_MASTERS));
                continue;
            }
            other => other?,
        };
        let stem = stem(&resolved, &req.options, source);
        let folder = match crate::export_steps::export_folder(&resolved, &req.options) {
            Ok(folder) => folder,
            Err(e) => {
                skipped.push(skip(job, &e.to_string()));
                continue;
            }
        };
        instances.push(json!({ "instance": job.label, "source": job.source, "master": resolved.master, "location": resolved.location }));
        for &flavour in &req.flavours {
            let plan = fontcraft_compile::plan::plan_resolved_from(font, resolved.clone(), flavour, &req.options)?;
            let files: Vec<Value> = req
                .containers
                .iter()
                .map(|&c| {
                    let name = req.placed(folder.as_deref(), req.file_name(&stem, flavour, c));
                    let path = match &req.dest {
                        Dest::Memory => None,
                        Dest::File(path) => Some(path.clone()),
                        Dest::Folder(root) => Some(name.split('/').fold(root.clone(), |p, part| p.join(part))),
                    };
                    match path {
                        Some(path) => json!({ "name": name, "container": c.name(), "path": path.to_string_lossy() }),
                        None => json!({ "name": name, "container": c.name() }),
                    }
                })
                .collect();
            let mut out = json!({
                "instance": job.label, "source": job.source, "format": format_name(flavour), "postscriptName": plan.postscript_name,
                "style": plan.style_name, "glyphs": plan.glyph_order.len(), "removeOverlap": req.options.remove_overlap, "files": files,
                "warnings": plan.warnings,
            });
            if let (true, Value::Object(m)) = (want("tables"), &mut out) {
                m.insert("tables".into(), json!(plan.tables));
            }
            outputs.push(out);
            if glyph_order.is_none() {
                production_names = Some(
                    plan.glyph_order
                        .iter()
                        .zip(&plan.final_names)
                        .filter(|(source, written)| source != written)
                        .map(|(source, written)| (source.clone(), json!(written)))
                        .collect(),
                );
                glyph_order = Some(plan.glyph_order);
            }
        }
    }
    if outputs.is_empty() {
        warnings.push("nothing would be built".into());
    } else if matches!(req.dest, Dest::File(_)) && outputs.len() > 1 {
        warnings.push(format!("`destination` is one file, but {} fonts would be built: font.compile refuses this", outputs.len()));
    }
    let mut result = Map::new();
    if want("outputs") || want("tables") {
        result.insert("outputs".into(), Value::Array(outputs));
    }
    if want("glyphOrder") {
        result.insert("glyphOrder".into(), json!(glyph_order.unwrap_or_default()));
    }
    if want("productionNames") {
        result.insert("productionNames".into(), Value::Object(production_names.unwrap_or_default()));
    }
    if want("instances") {
        result.insert("instances".into(), Value::Array(instances));
    }
    result.insert("skipped".into(), Value::Array(skipped));
    result.insert("warnings".into(), json!(warnings));
    Ok(Value::Object(result))
}
