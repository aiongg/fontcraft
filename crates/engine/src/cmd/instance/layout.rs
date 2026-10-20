//! `font.instantiate` with `layoutOnly` or `interpolateBinaryLayout`: `GPOS` interpolated from
//! the compiled masters of a source, as fontTools' `varLib.interpolate_layout` makes it (M12.22c,
//! M12.22d; [`fontcraft_tables::interpolate_layout`]).
//!
//! The compiled masters are found from the file names of the source's master UFOs. A designspace
//! names them; for a `.glyphs` or `.glyphspackage` source they are the names glyphsLib gives them
//! when fontmake builds its designspace, in `master_ufo/` beside the source ([`glyphs_ufo_names`]).
//! A source that names a font or TTX file is that master; otherwise the `masterFinder` pattern
//! turns the file name into the master's (fontTools' `MasterFinder`: `{fullname}`, `{dirname}`,
//! `{basename}`, `{stem}`, `{ext}`). A relative result is relative to the source's folder.
//!
//! - `layoutOnly` reads only the designspace (not its UFOs), or the `.glyphs` source. Each
//!   instance is the base master (the source at the default location) with the interpolated
//!   `GPOS`. Default pattern `master_ttf_interpolatable/{stem}.ttf`.
//! - `interpolateBinaryLayout` is fontmake's `--interpolate-binary-layout [DIR]` (fontmake
//!   `font_project.py`, `save_otfs` and `_varLib_finder`, Apache License 2.0): each instance is
//!   compiled as usual, then its `GPOS` is replaced by the interpolated one, and its `GDEF` and
//!   `GSUB` by the base master's when the base master has them ([`BinaryLayout`]). Default
//!   pattern `{DIR}/{stem}.{ext}`, `DIR` defaulting to `master_ttf` or `master_otf` as the
//!   instances' format. The instances are compiled into a scratch folder inside the destination
//!   and staged at their real paths only once every layout is replaced, so a run that fails
//!   leaves the destination as it was.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use fontcraft_geom::variation::VariationModel;
use fontcraft_interp::Designspace;
use fontcraft_model::plist::Value as PValue;
use fontcraft_model::{Family, Font, SourceFormat};
use fontcraft_tables::interpolate_layout::MAX_LAYOUT_MASTERS;
use fontcraft_tables::sfnt::{SfntFont, Tag};
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Value, json};

use super::super::file_io::{self, InputKind, OutputRequest};
use super::source::{Job, jobs};
use super::{Staged, commit, discard, stage};
use crate::params::Params;
use crate::{EngineError, Result};

/// fontTools' default pattern.
const DEFAULT_FINDER: &str = "master_ttf_interpolatable/{stem}.ttf";

/// The folder fontmake writes the master UFOs of a `.glyphs` source into.
const GLYPHS_MASTER_DIR: &str = "master_ufo";

const GDEF: Tag = *b"GDEF";
const GSUB: Tag = *b"GSUB";
const GPOS: Tag = *b"GPOS";

/// A UFO with nothing in it, standing in for each source so the designspace can be read without
/// its UFOs.
fn stub_ufo() -> BTreeMap<String, Vec<u8>> {
    let plist = |body: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
        )
        .into_bytes()
    };
    BTreeMap::from([
        ("metainfo.plist".to_string(), plist("<dict>\n<key>formatVersion</key>\n<integer>3</integer>\n</dict>")),
        ("glyphs/contents.plist".to_string(), plist("<dict/>")),
    ])
}

pub(super) fn instantiate(p: &Params<'_>, input: &Path, req: &OutputRequest) -> Result<Value> {
    for key in ["instanceDir", "ufoVersion", "interpolateBinaryLayout"] {
        if p.get(key).is_some() {
            return Err(p.bad(format!("`{key}` applies to instances built from the source, and `layoutOnly` writes the base compiled master")));
        }
    }
    if p.bool_or("removeOverlaps", false)? || p.bool_or("autohint", false)? {
        return Err(p.bad("`layoutOnly` changes only the GPOS table of the base master: overlaps and hints stay as they are"));
    }
    if req.in_place {
        return Err(p.bad("an instance never replaces its source: give `output` or `outputDir`"));
    }
    let mut font = read_source(input)?;
    super::source::refuse_without_axes(p, &font)?;
    let ds = Designspace::new(&font).map_err(|e| p.bad(e.to_string()))?;
    let jobs = jobs(p, &mut font, &ds, p.bool_or("clamp", true)?)?;
    if jobs.len() > 1 && req.output.is_some() {
        return Err(p.bad("`output` names one file, and this run builds several instances: give `outputDir`"));
    }

    let finder = p.opt_str("masterFinder")?.unwrap_or(DEFAULT_FINDER);
    let set = CompiledMasters::load(p, input, &font, &ds, finder)?;
    let base_font = set.base_font(p)?;
    let ext = if base_font.is_cff() || base_font.flavour == fontcraft_tables::sfnt::Flavour::Cff { "otf" } else { "ttf" };
    match p.opt_str("to")? {
        None | Some("same") => {}
        Some(to) if to == ext => {}
        Some(to) => return Err(p.bad(format!("`layoutOnly` writes the base master's format ({ext}), not {to}"))),
    }
    let stamp = super::timestamp(p)?;

    let plan = Plan { font: &font, ds: &ds, set: &set, ext, stamp, dir: source_dir(input) };
    let created = file_io::create_output_dir(req)?;
    let mut staged: Vec<Staged> = Vec::new();
    let built = write_all(p, input, req, &plan, &jobs, &mut staged);
    let results = match built.and_then(|r| commit(&staged).map(|()| r)) {
        Ok(r) => r,
        Err(e) => {
            discard(&staged);
            file_io::remove_created(&created);
            return Err(e);
        }
    };
    Ok(json!({ "results": results, "warnings": [] }))
}

/// The folder relative paths of a source are resolved against.
fn source_dir(input: &Path) -> &Path {
    input.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// The compiled masters of a source, in its masters' order, with the model that weighs them.
pub(super) struct CompiledMasters {
    masters: Vec<SfntFont>,
    /// The master at the default location.
    base: usize,
    model: VariationModel,
}

impl CompiledMasters {
    /// Finds and reads the compiled master of every master of `font` (see the module notes).
    fn load(p: &Params<'_>, input: &Path, font: &Font, ds: &Designspace, finder: &str) -> Result<Self> {
        if font.masters.len() > MAX_LAYOUT_MASTERS {
            return Err(EngineError::Unsupported(format!("interpolating layout from more than {MAX_LAYOUT_MASTERS} masters")));
        }
        let mut infos: Vec<_> = ds.masters().iter().collect();
        infos.sort_by_key(|m| m.index);
        let origin = ds.origin().ok_or_else(|| p.bad("no source is at the default location of every axis"))?;
        let base = infos.iter().position(|m| m.id == origin.id).ok_or_else(|| p.bad("the default source is not a master"))?;
        let dir = source_dir(input);
        let files = source_files(p, input, font)?;
        let mut masters = Vec::with_capacity(infos.len());
        let mut locations = Vec::with_capacity(infos.len());
        for info in &infos {
            let file = files.get(info.index).ok_or_else(|| p.bad(format!("source `{}` is missing", info.name)))?;
            masters.push(open_master(p, &dir.join(file), finder, dir)?);
            locations.push(ds.to_normalized(&info.location).map_err(|e| p.bad(e.to_string()))?.0);
        }
        let model = VariationModel::new(&locations, false).map_err(|e| p.bad(format!("the masters' locations: {e}")))?;
        Ok(Self { masters, base, model })
    }

    fn base_font(&self, p: &Params<'_>) -> Result<&SfntFont> {
        self.masters.get(self.base).ok_or_else(|| p.bad("the default source is not a master"))
    }

    /// The base master with the `GPOS` interpolated at the location of instance `index` of `font`.
    fn interpolate(&self, p: &Params<'_>, font: &Font, ds: &Designspace, index: usize) -> Result<SfntFont> {
        let scalars = self.scalars(p, font, ds, index)?;
        Ok(fontcraft_tables::interpolate_layout::interpolate_layout(&self.masters, self.base, &scalars)?)
    }

    /// Each master's weight at the location of instance `index` of `font`.
    fn scalars(&self, p: &Params<'_>, font: &Font, ds: &Designspace, index: usize) -> Result<Vec<f64>> {
        let location = ds.instance_location(font, index).map_err(|e| p.bad(e.to_string()))?;
        let normalized = ds.to_normalized(&location).map_err(|e| p.bad(e.to_string()))?;
        Ok(self.model.master_scalars(&normalized.0))
    }
}

/// The file name of each master's UFO, by master index, relative to the source's folder.
fn source_files(p: &Params<'_>, input: &Path, font: &Font) -> Result<Vec<PathBuf>> {
    match font.source.format {
        SourceFormat::Designspace => font
            .masters
            .iter()
            .map(|m| {
                if m.extra.get(Family::Designspace, "layer").is_some() {
                    return Err(p.bad(format!("source `{}` is a layer of another source's UFO, which has no compiled master of its own", m.name)));
                }
                Ok(PathBuf::from(m.extra.get(Family::Designspace, "filename").and_then(|v| v.as_str()).unwrap_or_default()))
            })
            .collect(),
        SourceFormat::Glyphs | SourceFormat::GlyphsPackage => {
            Ok(glyphs_ufo_names(font).into_iter().map(|name| Path::new(GLYPHS_MASTER_DIR).join(name)).collect())
        }
        _ => Err(EngineError::Unsupported(format!(
            "finding the compiled masters of {}: they are found from a designspace or a .glyphs source",
            input.display()
        ))),
    }
}

/// The file names glyphsLib gives the master UFOs of a Glyphs source (glyphsLib
/// `builder/sources.py`, Apache License 2.0): the master's `UFO Filename` parameter, else its
/// `ufoFilename` user data, else `Family-Master.ufo` with the spaces taken out; a name an earlier
/// master has gets `#1`, then `#2`…, before its extension. A source without a family name is
/// glyphsLib's "Unnamed font".
fn glyphs_ufo_names(font: &Font) -> Vec<String> {
    let text = |v: Option<&PValue>| v.and_then(PValue::as_str).map(str::to_string);
    let family = if font.family_name.is_empty() { "Unnamed font" } else { font.family_name.as_str() };
    let mut names: Vec<String> = Vec::with_capacity(font.masters.len());
    for m in font.masters.iter() {
        let mut name = text(m.params.get("UFO Filename"))
            .or_else(|| text(m.user_data.get("com.schriftgestaltung.Glyphs.ufoFilename")))
            .unwrap_or_else(|| format!("{}-{}.ufo", family.replace(' ', ""), m.name.replace(' ', "")));
        let mut n = 1;
        while names.contains(&name) {
            let (stem, ext) = split_ext(&name);
            name = format!("{stem}#{n}{ext}");
            n += 1;
        }
        names.push(name);
    }
    names
}

/// `os.path.splitext`: the extension is the last `.` and what follows it in the last path
/// part, unless that part starts with it.
fn split_ext(name: &str) -> (&str, &str) {
    let part = name.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let last = name.get(part..).unwrap_or_default();
    match last.rfind('.') {
        Some(i) if last.get(..i).is_some_and(|s| !s.chars().all(|c| c == '.')) => {
            let at = part + i;
            (name.get(..at).unwrap_or(name), name.get(at..).unwrap_or_default())
        }
        _ => (name, ""),
    }
}

/// `interpolateBinaryLayout`: what each compiled instance's layout is replaced with. `font.compile`
/// uses it too.
pub(in crate::cmd) struct BinaryLayout {
    /// The base master with the interpolated `GPOS`, by instance index.
    layouts: Vec<(usize, SfntFont)>,
}

impl BinaryLayout {
    /// Reads the compiled masters and interpolates the layout of every job, before anything is
    /// compiled. `dir` is fontmake's `DIR` (default `master_<ext>`); `masterFinder`, when given,
    /// is used instead.
    pub(super) fn load(p: &Params<'_>, input: &Path, font: &Font, ds: &Designspace, jobs: &[Job], ext: &str, dir: Option<&str>) -> Result<Self> {
        let indices: Vec<usize> = jobs.iter().map(|j| j.index).collect();
        Self::load_for(p, input, font, ds, &indices, ext, dir)
    }

    /// [`Self::load`] for the instances of `font` at `indices`.
    pub(in crate::cmd) fn load_for(
        p: &Params<'_>,
        input: &Path,
        font: &Font,
        ds: &Designspace,
        indices: &[usize],
        ext: &str,
        dir: Option<&str>,
    ) -> Result<Self> {
        let finder = match (p.opt_str("masterFinder")?, dir) {
            (Some(_), Some(_)) => {
                return Err(p.bad("`interpolateBinaryLayout` names the masters' folder and `masterFinder` their files: give one"));
            }
            (Some(f), None) => f.to_string(),
            (None, dir) => {
                let dir = dir.map_or_else(|| format!("master_{ext}"), str::to_string);
                format!("{}/{{stem}}.{ext}", dir.replace('{', "{{").replace('}', "}}"))
            }
        };
        let set = CompiledMasters::load(p, input, font, ds, &finder)?;
        let mut layouts = Vec::with_capacity(indices.len());
        for &index in indices {
            if !layouts.iter().any(|(i, _)| *i == index) {
                layouts.push((index, set.interpolate(p, font, ds, index)?));
            }
        }
        Ok(Self { layouts })
    }

    /// The compiled font `bytes` of instance `index`, with its layout replaced.
    pub(in crate::cmd) fn replace(&self, p: &Params<'_>, index: usize, bytes: &[u8]) -> Result<Vec<u8>> {
        let from = self
            .layouts
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, f)| f)
            .ok_or_else(|| p.bad(format!("no layout was interpolated for instance {index}")))?;
        let made = transfer_layout(from, fontcraft_tables::read(bytes, 0)?)?;
        Ok(fontcraft_tables::write(&made, &WriteOptions { order: TableOrder::Recommended })?)
    }

    /// Replaces the layout of each compiled instance, given by instance index, the file it was
    /// compiled to and the file to write; every file is written only once all of them are built.
    pub(super) fn apply(&self, p: &Params<'_>, outputs: &[(usize, PathBuf, PathBuf)]) -> Result<()> {
        let mut staged: Vec<Staged> = Vec::with_capacity(outputs.len());
        match self.stage_all(p, outputs, &mut staged).and_then(|()| commit(&staged)) {
            Ok(()) => Ok(()),
            Err(e) => {
                discard(&staged);
                Err(e)
            }
        }
    }

    fn stage_all(&self, p: &Params<'_>, outputs: &[(usize, PathBuf, PathBuf)], staged: &mut Vec<Staged>) -> Result<()> {
        for (index, compiled, path) in outputs {
            let read = file_io::read_input(compiled)?;
            let bytes = self.replace(p, *index, &read.bytes)?;
            staged.push(stage(path, &bytes, staged.len())?);
        }
        Ok(())
    }
}

/// `into` with the `GPOS` of `from`, or none when `from` has none, and the `GDEF` and `GSUB` of
/// `from` when it has them (fontmake takes those from the full source nearest the instance;
/// layer sources are refused, so that is the default source). The tables are copied as they are
/// when both fonts have one glyph order, and through TTX, by glyph name, otherwise.
fn transfer_layout(from: &SfntFont, mut into: SfntFont) -> Result<SfntFont> {
    if from.get(GPOS).is_none() {
        into.remove(GPOS);
    }
    let tags: Vec<Tag> = [GDEF, GSUB, GPOS].into_iter().filter(|t| from.get(*t).is_some()).collect();
    if tags.is_empty() {
        return Ok(into);
    }
    if fontcraft_tables::ttx::glyph_order(from)? == fontcraft_tables::ttx::glyph_order(&into)? {
        for tag in tags {
            if let Some(bytes) = from.get(tag) {
                into.insert(tag, bytes.to_vec());
            }
        }
        return Ok(into);
    }
    let text = fontcraft_tables::ttx::dump(from, &fontcraft_tables::ttx::DumpOptions { tables: tags.clone(), ..Default::default() })?;
    Ok(fontcraft_tables::ttx::import(into, &text, &fontcraft_tables::ttx::ImportOptions { tables: tags, ..Default::default() })?)
}

/// What every instance of a run is built from.
struct Plan<'a> {
    font: &'a Font,
    ds: &'a Designspace,
    set: &'a CompiledMasters,
    ext: &'static str,
    stamp: Option<i64>,
    dir: &'a Path,
}

/// Builds each job's font and stages it beside its output; answers the result entries.
fn write_all(p: &Params<'_>, input: &Path, req: &OutputRequest, plan: &Plan<'_>, jobs: &[Job], staged: &mut Vec<Staged>) -> Result<Vec<Value>> {
    let Plan { font, ds, set, ext, stamp, dir } = *plan;
    let mut outs: Vec<PathBuf> = Vec::new();
    let mut results = Vec::with_capacity(jobs.len());
    for job in jobs {
        let inst = font.instances.get(job.index).ok_or_else(|| p.bad("an instance is missing"))?;
        let mut made = set.interpolate(p, font, ds, job.index)?;
        if let Some(t) = stamp {
            super::set_modified(&mut made, t);
        }
        let bytes = fontcraft_tables::write(&made, &WriteOptions { order: TableOrder::Recommended })?;
        let names = fontcraft_io_ufo::designspace::instance_names(font, inst);
        let out = if req.output.is_some() || job.from_location {
            file_io::output_path(input, req, ".instance", Some(ext))?
        } else {
            let stem = names
                .postscript
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| Path::new(&names.filename).file_stem().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| inst.name.clone());
            if stem.is_empty() || stem.contains(['/', '\\']) || stem == "." || stem == ".." {
                return Err(p.bad(format!("instance `{}` has no name a file can take", inst.name)));
            }
            req.output_dir.clone().unwrap_or_else(|| dir.to_path_buf()).join(format!("{stem}.{ext}"))
        };
        if outs.contains(&out) {
            return Err(p.bad(format!("two instances would both be written to {}", out.display())));
        }
        file_io::check_write(&out)?;
        if req.no_clobber && out.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", out.display())));
        }
        staged.push(stage(&out, &bytes, staged.len())?);
        let mut entry = json!({ "output": out.display().to_string(), "location": job.location.clone(), "static": true });
        if let (false, Some(obj)) = (job.from_location, entry.as_object_mut()) {
            obj.insert("name".into(), json!(inst.name));
            if let Some(ps) = &names.postscript {
                obj.insert("postscriptName".into(), json!(ps));
            }
        }
        outs.push(out);
        results.push(entry);
    }
    Ok(results)
}

/// The source as `layoutOnly` reads it: a designspace without its UFOs, or a `.glyphs` or
/// `.glyphspackage` source.
fn read_source(input: &Path) -> Result<Font> {
    let ext = input.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "designspace" => {
            let read = file_io::read_input(input)?;
            let text = String::from_utf8(read.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", input.display())))?;
            let mut stubs = BTreeMap::new();
            for rel in fontcraft_io_ufo::designspace::ufo_paths(&text)? {
                stubs.insert(rel, stub_ufo());
            }
            Ok(fontcraft_io_ufo::designspace::read(&text, &stubs)?)
        }
        "glyphs" | "glyphspackage" => {
            let (font, _) = crate::io::open(input, None)?;
            if !matches!(font.source.format, SourceFormat::Glyphs | SourceFormat::GlyphsPackage) {
                return Err(EngineError::Unsupported(format!("`layoutOnly`: {} does not open as a .glyphs source", input.display())));
            }
            Ok(font)
        }
        _ => Err(EngineError::Unsupported(format!(
            "`layoutOnly` reads the sources of a designspace or a .glyphs source, and {} is neither",
            input.display()
        ))),
    }
}

/// The compiled master of the source at `source`: the file itself when it is a font or TTX
/// file, else the file `finder` names for it (fontTools' `_open_font`).
fn open_master(p: &Params<'_>, source: &Path, finder: &str, dir: &Path) -> Result<SfntFont> {
    let as_font = |path: &Path| -> Result<Option<SfntFont>> {
        // Checked before the file is looked at, so that the answer does not tell whether a file
        // outside the read root exists.
        crate::file_access::check_read(path).map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))?;
        if !path.is_file() {
            return Ok(None);
        }
        let read = file_io::read_input(path)?;
        match read.kind {
            InputKind::Ttx => {
                let text = String::from_utf8(read.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", path.display())))?;
                let empty = SfntFont::new(fontcraft_tables::sfnt::Flavour::TrueType);
                Ok(Some(fontcraft_tables::ttx::import(empty, &text, &fontcraft_tables::ttx::ImportOptions::default())?))
            }
            k if k.is_font() => Ok(Some(fontcraft_tables::read(&read.bytes, 0)?)),
            _ => Ok(None),
        }
    };
    // A source outside the read root is not looked at; the pattern may still find its master.
    if crate::file_access::check_read(source).is_ok()
        && let Some(font) = as_font(source)?
    {
        return Ok(font);
    }
    let found = find(finder, source).map_err(|e| p.bad(e))?;
    let found = if found.is_absolute() { found } else { dir.join(found) };
    as_font(&found)?
        .ok_or_else(|| EngineError::Io(format!("the compiled master of {} is not at {} (a font or TTX file)", source.display(), found.display())))
}

/// fontTools' `MasterFinder`: `pattern` with `{fullname}` (the source's absolute path),
/// `{dirname}`, `{basename}`, `{stem}` and `{ext}` (with its dot) filled in; `{{` and `}}` are
/// braces.
fn find(pattern: &str, source: &Path) -> std::result::Result<PathBuf, String> {
    let full = lexical(&std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf()));
    let text = |p: Option<&std::ffi::OsStr>| p.map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let basename = text(full.file_name());
    let (stem, ext) = match basename.rfind('.') {
        Some(i) if i > 0 => (basename.get(..i).unwrap_or_default().to_string(), basename.get(i..).unwrap_or_default().to_string()),
        _ => (basename.clone(), String::new()),
    };
    let fields = [
        ("fullname", full.to_string_lossy().into_owned()),
        ("dirname", text(full.parent().map(Path::as_os_str))),
        ("basename", basename.clone()),
        ("stem", stem),
        ("ext", ext),
    ];
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("{{") || rest.starts_with("}}") {
            out.push(c);
            rest = rest.get(2..).unwrap_or_default();
        } else if c == '{' {
            let end = rest.find('}').ok_or_else(|| format!("`masterFinder` has an unclosed brace: {pattern}"))?;
            let name = rest.get(1..end).unwrap_or_default();
            let value = fields
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.as_str())
                .ok_or_else(|| format!("`masterFinder` knows fullname, dirname, basename, stem and ext, not `{name}`"))?;
            out.push_str(value);
            rest = rest.get(end + 1..).unwrap_or_default();
        } else if c == '}' {
            return Err(format!("`masterFinder` has an unmatched brace: {pattern}"));
        } else {
            out.push(c);
            rest = rest.get(c.len_utf8()..).unwrap_or_default();
        }
    }
    if out.is_empty() {
        return Err("`masterFinder` gives an empty path".into());
    }
    Ok(lexical(Path::new(&out)))
}

/// `path` with `.` dropped and each `..` folded into the part before it, by spelling only (as
/// `os.path.normpath` does).
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
pub(in crate::cmd) mod tests {
    use super::*;

    #[test]
    fn the_finder_fills_the_source_name_fields() {
        let src = Path::new("/w/master_ufo/Test-Bold.ufo");
        assert_eq!(find(DEFAULT_FINDER, src).unwrap(), PathBuf::from("master_ttf_interpolatable/Test-Bold.ttf"));
        assert_eq!(find("{dirname}/../ttx/{stem}.ttx", src).unwrap(), PathBuf::from("/w/ttx/Test-Bold.ttx"));
        assert_eq!(find("{basename}{ext}{{x}}", src).unwrap(), PathBuf::from("Test-Bold.ufo.ufo{x}"));
        assert!(find("{name}.ttf", src).is_err());
        assert!(find("{stem.ttf", src).is_err());
    }

    /// Under a read root, a compiled master outside it is refused with the same message whether
    /// or not the file exists.
    #[test]
    fn a_master_outside_the_read_root_is_refused_before_it_is_looked_at() {
        let base = std::env::temp_dir().join(format!("fontcraft-layout-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("in")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(base.join("outside/there.ttf"), b"not a font").unwrap();
        let roots = crate::file_access::AutomationRoots::new(Some(&base.join("in")), None).unwrap().unwrap();
        let params = json!({});
        let p = Params::new("font.instantiate", &params).unwrap();
        let source = base.join("outside/A.ufo");
        let refusal = |name: &str| {
            let finder = format!("{}/{name}", base.join("outside").display());
            crate::file_access::confine(Some(&roots), || open_master(&p, &source, &finder, &base.join("in")))
                .err()
                .unwrap()
                .to_string()
                .replace(name, "X")
        };
        let present = refusal("there.ttf");
        assert!(present.contains("outside the read root"), "{present}");
        assert_eq!(present, refusal("missing.ttf"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn glyphs_master_ufos_are_named_as_glyphs_lib_names_them() {
        let mut font = Font::new("Test Sans");
        font.masters = std::sync::Arc::new(
            ["Light Two", "Bold", "Bold"].map(|name| fontcraft_model::Master { id: name.into(), name: name.into(), ..Default::default() }).to_vec(),
        );
        assert_eq!(glyphs_ufo_names(&font), ["TestSans-LightTwo.ufo", "TestSans-Bold.ufo", "TestSans-Bold#1.ufo"]);
        font.family_name = String::new();
        assert_eq!(glyphs_ufo_names(&font)[0], "Unnamedfont-LightTwo.ufo");
        assert_eq!(split_ext("a.b/c"), ("a.b/c", ""));
        assert_eq!(split_ext(".ufo"), (".ufo", ""));
        assert_eq!(split_ext("x/A.ufo"), ("x/A", ".ufo"));
    }

    /// A two-master Glyphs source whose only kerning pair, A V, is `light` in the light master
    /// and `bold` in the bold one.
    fn glyphs_source(light: i32, bold: i32) -> String {
        glyphs_source_with(light, bold, false)
    }

    /// [`glyphs_source`], with a glyph W kerned against A when `with_w`.
    pub(in crate::cmd) fn glyphs_source_with(light: i32, bold: i32, with_w: bool) -> String {
        let glyph = |name: &str, unicode: u32, light: i32, bold: i32| {
            let layer = |id: &str, w: i32| {
                format!(
                    "{{\nlayerId = {id};\nshapes = (\n{{\nclosed = 1;\nnodes = (\n(0,0,l),\n({w},0,l),\n({},700,l)\n);\n}}\n);\nwidth = {w};\n}}",
                    w / 2
                )
            };
            format!("{{\nglyphname = {name};\nlayers = (\n{},\n{}\n);\nunicode = {unicode};\n}}", layer("m01", light), layer("m02", bold))
        };
        format!(
            "{{\n.appVersion = \"3260\";\n.formatVersion = 3;\naxes = (\n{{\nname = Weight;\ntag = wght;\n}}\n);\nfamilyName = \"Test Sans\";\n\
             fontMaster = (\n{{\naxesValues = (\n300\n);\nid = m01;\nname = \"Light Two\";\n}},\n{{\naxesValues = (\n700\n);\nid = m02;\nname = Bold;\n}}\n);\n\
             glyphs = (\n{},\n{}\n);\ninstances = (\n{{\naxesValues = (\n400\n);\nname = Regular;\n}}\n);\n\
             kerningLTR = {{\nm01 = {{\nA = {{\nV = {light};{w_light}\n}};\n}};\nm02 = {{\nA = {{\nV = {bold};{w_bold}\n}};\n}};\n}};\n\
             unitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}}\n",
            glyph("A", 65, 500, 600),
            if with_w { format!("{},\n{}", glyph("V", 86, 520, 640), glyph("W", 87, 700, 800)) } else { glyph("V", 86, 520, 640) },
            w_light = if with_w { "\nW = -30;" } else { "" },
            w_bold = if with_w { "\nW = -70;" } else { "" },
        )
    }

    /// A Glyphs source whose compiled masters (kerned -60 and -140) sit where fontmake writes
    /// them, `master_ttf/TestSans-LightTwo.ttf` and `master_ttf/TestSans-Bold.ttf`, while the
    /// source itself kerns -50 and -110. At wght 400 the masters weigh 0.75 and 0.25: the
    /// interpolated pair is -80 where the source's own would be -65.
    #[test]
    fn a_glyphs_source_finds_its_compiled_masters_by_their_ufo_names() {
        let dir = std::env::temp_dir().join(format!("fontcraft-layout-glyphs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("masters")).unwrap();
        std::fs::create_dir_all(dir.join("master_ttf")).unwrap();
        let masters_source = dir.join("masters/Test.glyphs");
        std::fs::write(&masters_source, glyphs_source(-60, -140)).unwrap();
        let input = dir.join("Test.glyphs");
        std::fs::write(&input, glyphs_source(-50, -110)).unwrap();

        let (font, _) = crate::io::open(&masters_source, None).unwrap();
        let mut s = crate::Session::new();
        s.push_document(font, Some(masters_source.clone()));
        let built = s
            .execute(
                "font.compile",
                &json!({ "formats": ["ttf"], "instances": false, "mastersAsInstances": true, "destination": dir.join("masters/ttf").display().to_string() }),
            )
            .unwrap();
        let mut masters = Vec::new();
        for (label, stem) in [("Light Two", "TestSans-LightTwo"), ("Bold", "TestSans-Bold")] {
            let f = built["fonts"].as_array().unwrap().iter().find(|f| f["instance"] == json!(label)).unwrap();
            let to = dir.join("master_ttf").join(format!("{stem}.ttf"));
            std::fs::rename(f["files"][0]["path"].as_str().unwrap(), &to).unwrap();
            masters.push(fontcraft_tables::read(&std::fs::read(&to).unwrap(), 0).unwrap());
        }
        let expected = fontcraft_tables::interpolate_layout::interpolate_gpos(&masters, 0, &[0.75, 0.25]).unwrap().unwrap();

        let mut s = crate::Session::new();
        let r = s
            .execute(
                "font.instantiate",
                &json!({ "input": input, "location": { "wght": 400 }, "layoutOnly": true, "masterFinder": "master_ttf/{stem}.ttf" }),
            )
            .unwrap();
        let out = dir.join("Test.instance.ttf");
        assert_eq!(r["results"][0]["output"], json!(out.display().to_string()));
        let made = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
        assert_eq!(made.get(GPOS), Some(expected.as_slice()), "layoutOnly: the interpolated GPOS");
        assert_eq!(made.get(*b"glyf"), masters[0].get(*b"glyf"), "layoutOnly: the base master's outlines");

        let out = dir.join("bin.ttf");
        s.execute(
            "font.instantiate",
            &json!({ "input": input, "location": { "wght": 400 }, "to": "ttf", "interpolateBinaryLayout": true, "output": out }),
        )
        .unwrap();
        let made = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
        assert_eq!(made.get(GPOS), Some(expected.as_slice()), "the compiled instance's GPOS is replaced");
        assert_eq!(made.get(GDEF), masters[0].get(GDEF), "GDEF follows the default master");
        assert_ne!(made.get(*b"hmtx"), masters[0].get(*b"hmtx"), "the outlines and metrics are the instance's own");

        let without = dir.join("own.ttf");
        s.execute("font.instantiate", &json!({ "input": input, "location": { "wght": 400 }, "to": "ttf", "output": without })).unwrap();
        let own = fontcraft_tables::read(&std::fs::read(&without).unwrap(), 0).unwrap();
        assert_ne!(own.get(GPOS), Some(expected.as_slice()), "without the option the source's own kerning");

        let mut refused = |params: Value| s.execute("font.instantiate", &params).is_err();
        assert!(
            refused(json!({ "input": input, "location": { "wght": 400 }, "interpolateBinaryLayout": true, "outputDir": dir.join("u") })),
            "UFO instances"
        );
        assert!(
            refused(
                json!({ "input": input, "location": { "wght": 400 }, "to": "ttf", "interpolateBinaryLayout": "elsewhere", "output": dir.join("x.ttf") })
            ),
            "no masters in that folder"
        );
        assert!(!dir.join("x.ttf").exists(), "a refused run writes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The compiled masters name a glyph, W, that the instance's source lacks, so the layout
    /// cannot be moved into the compiled instance. The run fails after compiling, and must leave
    /// the destination as it was: an existing file untouched, no folder made, no scratch left.
    #[test]
    fn interpolate_binary_layout_failing_after_compile_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("fontcraft-layout-fails-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("masters")).unwrap();
        std::fs::create_dir_all(dir.join("master_ttf")).unwrap();
        let masters_source = dir.join("masters/Test.glyphs");
        std::fs::write(&masters_source, glyphs_source_with(-60, -140, true)).unwrap();
        let input = dir.join("Test.glyphs");
        std::fs::write(&input, glyphs_source(-50, -110)).unwrap();

        let (font, _) = crate::io::open(&masters_source, None).unwrap();
        let mut s = crate::Session::new();
        s.push_document(font, Some(masters_source.clone()));
        let built = s
            .execute(
                "font.compile",
                &json!({ "formats": ["ttf"], "instances": false, "mastersAsInstances": true, "destination": dir.join("masters/ttf").display().to_string() }),
            )
            .unwrap();
        for (label, stem) in [("Light Two", "TestSans-LightTwo"), ("Bold", "TestSans-Bold")] {
            let f = built["fonts"].as_array().unwrap().iter().find(|f| f["instance"] == json!(label)).unwrap();
            std::fs::rename(f["files"][0]["path"].as_str().unwrap(), dir.join("master_ttf").join(format!("{stem}.ttf"))).unwrap();
        }

        let out = dir.join("out.ttf");
        std::fs::write(&out, b"earlier").unwrap();
        let entries = || {
            let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        };
        let before = entries();
        let mut s = crate::Session::new();
        let r = s.execute(
            "font.instantiate",
            &json!({ "input": input, "location": { "wght": 400 }, "to": "ttf", "interpolateBinaryLayout": true, "output": out }),
        );
        assert!(r.is_err(), "W is not in the instance's glyph order: {r:?}");
        assert_eq!(std::fs::read(&out).unwrap(), b"earlier", "the existing file is untouched");
        assert_eq!(entries(), before, "nothing else is left beside it");

        let missing = dir.join("new/instances");
        let r = s
            .execute("font.instantiate", &json!({ "input": input, "all": true, "to": "ttf", "interpolateBinaryLayout": true, "outputDir": missing }));
        assert!(r.is_err(), "W is not in the instance's glyph order: {r:?}");
        assert!(!dir.join("new").exists(), "the folders made for the outputs are removed");
        assert_eq!(entries(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// fontTools' vpal test: a designspace whose sources exist only as TTX masters.
    #[test]
    fn layout_only_interpolates_gpos_from_the_compiled_masters() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests/varLib/data");
        let Ok(doc) = std::fs::read_to_string(data.join("test_vpal.designspace")) else {
            eprintln!("skipped: corpus not fetched");
            return;
        };
        let dir = std::env::temp_dir().join(format!("fontcraft-layout-only-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("master_vpal_test")).unwrap();
        let input = dir.join("test_vpal.designspace");
        std::fs::write(&input, doc).unwrap();
        let mut masters = Vec::new();
        for stem in ["master_vpal_test_0", "master_vpal_test_1"] {
            let rel = format!("master_vpal_test/{stem}.ttx");
            let text = std::fs::read_to_string(data.join(&rel)).unwrap();
            std::fs::write(dir.join(&rel), &text).unwrap();
            let empty = SfntFont::new(fontcraft_tables::sfnt::Flavour::TrueType);
            masters.push(fontcraft_tables::ttx::import(empty, &text, &fontcraft_tables::ttx::ImportOptions::default()).unwrap());
        }
        let mut s = crate::Session::new();
        let finder = "{dirname}/{stem}.ttx";

        let r = s
            .execute(
                "font.instantiate",
                &json!({ "input": input, "location": { "wght": 250 }, "layoutOnly": true, "masterFinder": finder, "timestamp": 0 }),
            )
            .unwrap();
        let out = dir.join("test_vpal.instance.otf");
        assert_eq!(r["results"][0]["output"], json!(out.display().to_string()));
        let made = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
        let expected = fontcraft_tables::interpolate_layout::interpolate_gpos(&masters, 0, &[0.75, 0.25]).unwrap().unwrap();
        assert_eq!(made.get(*b"GPOS"), Some(expected.as_slice()), "GPOS a quarter of the way to the bold master");
        assert_eq!(made.get(*b"GSUB"), masters[0].get(*b"GSUB"), "the base master's other tables");

        let r = s
            .execute(
                "font.instantiate",
                &json!({ "input": input, "all": true, "layoutOnly": true, "masterFinder": finder, "outputDir": dir.join("all") }),
            )
            .unwrap();
        assert_eq!(
            r["results"][1]["output"],
            json!(dir.join("all").join("SHSansJPVFTest-Heavy.otf").display().to_string()),
            "named after the PostScript name"
        );
        let heavy = fontcraft_tables::read(&std::fs::read(dir.join("all").join("SHSansJPVFTest-Heavy.otf")).unwrap(), 0).unwrap();
        let expected = fontcraft_tables::interpolate_layout::interpolate_gpos(&masters, 0, &[0.0, 1.0]).unwrap().unwrap();
        assert_eq!(heavy.get(*b"GPOS"), Some(expected.as_slice()));

        assert!(
            s.execute("font.instantiate", &json!({ "input": input, "location": { "wght": 250 }, "layoutOnly": true })).is_err(),
            "the default pattern finds no master"
        );
        assert!(
            s.execute("font.instantiate", &json!({ "input": input, "location": { "wght": 250 }, "masterFinder": finder })).is_err(),
            "masterFinder alone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
