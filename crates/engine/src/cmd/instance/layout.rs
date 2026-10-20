//! `font.instantiate` with `layoutOnly`: an instance's `GPOS` interpolated from the compiled
//! masters of a designspace, as fontTools' `varLib.interpolate_layout` makes it (M12.22c;
//! [`fontcraft_tables::interpolate_layout`]).
//!
//! Only the designspace is read, not its UFOs: each source names its compiled master directly
//! (a font or TTX file), or the `masterFinder` pattern turns its file name into the master's
//! (fontTools' `MasterFinder`: `{fullname}`, `{dirname}`, `{basename}`, `{stem}`, `{ext}`; default
//! `master_ttf_interpolatable/{stem}.ttf`, relative to the designspace's folder). The result is
//! the base master (the source at the default location) with that `GPOS`, one file per instance.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use fontcraft_geom::variation::VariationModel;
use fontcraft_interp::Designspace;
use fontcraft_model::Family;
use fontcraft_tables::sfnt::SfntFont;
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Value, json};

use super::super::file_io::{self, InputKind, OutputRequest};
use super::source::jobs;
use super::{Staged, commit, discard, stage};
use crate::params::Params;
use crate::{EngineError, Result};

/// fontTools' default pattern.
const DEFAULT_FINDER: &str = "master_ttf_interpolatable/{stem}.ttf";

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
    for key in ["instanceDir", "ufoVersion"] {
        if p.get(key).is_some() {
            return Err(p.bad(format!("`{key}` applies to UFO instances, and `layoutOnly` writes compiled fonts")));
        }
    }
    if p.bool_or("removeOverlaps", false)? || p.bool_or("autohint", false)? {
        return Err(p.bad("`layoutOnly` changes only the GPOS table of the base master: overlaps and hints stay as they are"));
    }
    let text = read_designspace(input)?;
    let mut stubs = BTreeMap::new();
    for rel in fontcraft_io_ufo::designspace::ufo_paths(&text)? {
        stubs.insert(rel, stub_ufo());
    }
    let mut font = fontcraft_io_ufo::designspace::read(&text, &stubs)?;
    let ds = Designspace::new(&font).map_err(|e| p.bad(e.to_string()))?;
    let jobs = jobs(p, &mut font, &ds, p.bool_or("clamp", true)?)?;
    if jobs.len() > 1 && req.output.is_some() {
        return Err(p.bad("`output` names one file, and this run builds several instances: give `outputDir`"));
    }

    // The masters in the designspace's order, each with its normalised location.
    let mut infos: Vec<_> = ds.masters().iter().collect();
    infos.sort_by_key(|m| m.index);
    let origin = ds.origin().ok_or_else(|| p.bad("no source is at the default location of every axis"))?;
    let base = infos.iter().position(|m| m.id == origin.id).ok_or_else(|| p.bad("the default source is not a master"))?;
    let finder = p.opt_str("masterFinder")?.unwrap_or(DEFAULT_FINDER);
    let dir = input.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut masters = Vec::with_capacity(infos.len());
    let mut locations = Vec::with_capacity(infos.len());
    for info in &infos {
        let master = font.masters.get(info.index).ok_or_else(|| p.bad(format!("source `{}` is missing", info.name)))?;
        if master.extra.get(Family::Designspace, "layer").is_some() {
            return Err(p.bad(format!("source `{}` is a layer of another source's UFO, which has no compiled master of its own", info.name)));
        }
        let filename = master.extra.get(Family::Designspace, "filename").and_then(|v| v.as_str()).unwrap_or_default();
        masters.push(open_master(p, &dir.join(filename), finder, dir)?);
        locations.push(ds.to_normalized(&info.location).map_err(|e| p.bad(e.to_string()))?.0);
    }
    let model = VariationModel::new(&locations, false).map_err(|e| p.bad(format!("the masters' locations: {e}")))?;
    let base_font = masters.get(base).ok_or_else(|| p.bad("the default source is not a master"))?;
    let ext = if base_font.is_cff() || base_font.flavour == fontcraft_tables::sfnt::Flavour::Cff { "otf" } else { "ttf" };
    match p.opt_str("to")? {
        None | Some("same") => {}
        Some(to) if to == ext => {}
        Some(to) => return Err(p.bad(format!("`layoutOnly` writes the base master's format ({ext}), not {to}"))),
    }
    let stamp = super::timestamp(p)?;

    let plan = Plan { font: &font, ds: &ds, masters: &masters, base, model: &model, ext, stamp, dir };
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

/// What every instance of a run is built from.
struct Plan<'a> {
    font: &'a fontcraft_model::Font,
    ds: &'a Designspace,
    masters: &'a [SfntFont],
    base: usize,
    model: &'a VariationModel,
    ext: &'static str,
    stamp: Option<i64>,
    dir: &'a Path,
}

/// Builds each job's font and stages it beside its output; answers the result entries.
fn write_all(
    p: &Params<'_>,
    input: &Path,
    req: &OutputRequest,
    plan: &Plan<'_>,
    jobs: &[super::source::Job],
    staged: &mut Vec<Staged>,
) -> Result<Vec<Value>> {
    let Plan { font, ds, masters, base, model, ext, stamp, dir } = *plan;
    let mut outs: Vec<PathBuf> = Vec::new();
    let mut results = Vec::with_capacity(jobs.len());
    for job in jobs {
        let inst = font.instances.get(job.index).ok_or_else(|| p.bad("an instance is missing"))?;
        let location = ds.instance_location(font, job.index).map_err(|e| p.bad(e.to_string()))?;
        let normalized = ds.to_normalized(&location).map_err(|e| p.bad(e.to_string()))?;
        let scalars = model.master_scalars(&normalized.0);
        let mut made = fontcraft_tables::interpolate_layout::interpolate_layout(masters, base, &scalars)?;
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

fn read_designspace(input: &Path) -> Result<String> {
    let is_designspace = input.extension().is_some_and(|e| e.eq_ignore_ascii_case("designspace"));
    if !is_designspace {
        return Err(EngineError::Unsupported(format!("`layoutOnly` reads the sources of a designspace, and {} is not one", input.display())));
    }
    let read = file_io::read_input(input)?;
    String::from_utf8(read.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", input.display())))
}

/// The compiled master of the source at `source`: the file itself when it is a font or TTX
/// file, else the file `finder` names for it (fontTools' `_open_font`).
fn open_master(p: &Params<'_>, source: &Path, finder: &str, dir: &Path) -> Result<SfntFont> {
    let as_font = |path: &Path| -> Result<Option<SfntFont>> {
        if !path.is_file() {
            return Ok(None);
        }
        crate::file_access::check_read(path).map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))?;
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
    if let Some(font) = as_font(source)? {
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
mod tests {
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
