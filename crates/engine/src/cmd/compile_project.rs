//! The project compile of `font.compile` (M12.40; `fontcraft_compile::makeotf`): the project
//! file, the option words, the `fontinfo` file, the FontMenuNameDB, the GlyphOrderAndAliasDB and
//! the feature file named in the parameters, or found beside the source with `projectDefaults`,
//! read and parsed here and applied by the compile crate. Every path a project file names is
//! relative to the project file's folder and must lie inside the automation read root (the
//! output path: the write root) when one is in force.

use std::path::{Path, PathBuf};

use fontcraft_compile::cid::{CidOptions, MAX_CMAP_BYTES};
use fontcraft_compile::makeotf::{self, GlyphAlias, Prepared, Project};
use fontcraft_model::Font;

use crate::Result;
use crate::params::Params;

/// The parameters that make `font.compile` a project compile.
const KEYS: &[&str] = &[
    "projectFile",
    "projectOptions",
    "projectDefaults",
    "menuNames",
    "goadb",
    "fontinfo",
    "featureFile",
    "licenseCode",
    "os2BitsOn",
    "os2BitsOff",
    "os2Version",
];

/// Most bit numbers `os2BitsOn` or `os2BitsOff` may hold.
const MAX_BITS: usize = 64;

/// The feature files the reference looks for beside the source, in this order.
const DEFAULT_FEATURE_FILES: [&str; 2] = ["features", "features.fea"];

/// How many folders above the source's folder a FontMenuNameDB or GlyphOrderAndAliasDB is looked
/// for, as the reference looks for them.
const LEVELS_UP: usize = 4;

/// Whether the parameters ask for a project compile.
pub(super) fn requested(p: &Params) -> bool {
    KEYS.iter().any(|k| p.get(k).is_some_and(|v| !v.is_null()))
}

/// The project the parameters describe, with notes about settings it does not use; `None` for
/// an ordinary compile. `source` is the document's path.
pub(super) fn read(p: &Params, source: Option<&Path>) -> Result<Option<(Project, Vec<String>)>> {
    if !KEYS.iter().any(|k| p.get(k).is_some_and(|v| !v.is_null())) {
        return Ok(None);
    }
    let defaults = p.bool_or("projectDefaults", false)?;
    let folder = source.and_then(Path::parent);
    let mut settings = makeotf::Settings::default();
    let mut notes = Vec::new();
    let mut project_dir: Option<PathBuf> = None;
    if let Some(path) = p.opt_str("projectFile")? {
        let path = PathBuf::from(path);
        let text = text(p, "`projectFile`", &path)?;
        notes.extend(settings.apply_project(&text).map_err(|e| p.bad(format!("`projectFile`: {e}")))?);
        project_dir = path.parent().map(Path::to_path_buf);
    }
    if let Some(words) = p.opt_strings("projectOptions")? {
        if words.len() > makeotf::MAX_OPTION_WORDS {
            return Err(p.bad(format!("`projectOptions` holds more than {} words", makeotf::MAX_OPTION_WORDS)));
        }
        for word in words {
            settings.apply_option(word).map_err(|e| p.bad(format!("`projectOptions`: {e}")))?;
        }
    }
    for (key, on) in [("os2BitsOn", true), ("os2BitsOff", false)] {
        let Some(bits) = p.opt_array(key)? else { continue };
        if bits.len() > MAX_BITS {
            return Err(p.bad(format!("`{key}` holds more than {MAX_BITS} bits")));
        }
        for bit in bits {
            let bit = bit.as_i64().ok_or_else(|| p.bad(format!("`{key}` holds bit numbers from 0 to 15")))?;
            settings.apply_os2_bit(on, bit).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
        }
    }
    if let Some(version) = p.opt_int("os2Version", 1, 65_535)? {
        settings.apply_os2_version(version).map_err(|e| p.bad(format!("`os2Version`: {e}")))?;
    }
    if let Some(code) = p.opt_str("licenseCode")? {
        settings.apply_licence(code);
    }
    let fontinfo = match p.opt_str("fontinfo")? {
        Some(path) => Some(PathBuf::from(path)),
        None if defaults => folder.map(|f| f.join("fontinfo")).filter(|f| readable(f) && f.is_file()),
        None => None,
    };
    if let Some(path) = fontinfo {
        settings.apply_fontinfo(&text(p, "`fontinfo`", &path)?).map_err(|e| p.bad(format!("`fontinfo`: {e}")))?;
    }
    // Each path comes with the name its messages use: the parameter, or the project file's key.
    let from_project = |relative: &Option<String>, key: &'static str| -> Option<(PathBuf, &'static str)> {
        relative.as_ref().map(|r| (project_dir.as_deref().map_or_else(|| PathBuf::from(r), |d| d.join(r)), key))
    };
    let menu = match p.opt_str("menuNames")? {
        Some(path) => Some((PathBuf::from(path), "`menuNames`")),
        None => from_project(&settings.paths.menu_names, "`projectFile` FontMenuDBPath")
            .or_else(|| if defaults { near(folder, "FontMenuNameDB").map(|f| (f, "`menuNames`")) } else { None }),
    };
    let goadb = match p.opt_str("goadb")? {
        Some(path) => Some((PathBuf::from(path), "`goadb`")),
        None => from_project(&settings.paths.goadb, "`projectFile` GlyphAliasDBPath")
            .or_else(|| if defaults && settings.apply_goadb { near(folder, "GlyphOrderAndAliasDB").map(|f| (f, "`goadb`")) } else { None }),
    };
    let menu_db = match menu {
        Some((path, key)) => Some(makeotf::parse_menu_db(&text(p, key, &path)?).map_err(|e| p.bad(format!("{key}: {e}")))?),
        None => None,
    };
    let aliases = match goadb {
        Some((path, key)) => {
            let table = fontcraft_tables::goadb::parse(&text(p, key, &path)?).map_err(|e| p.bad(format!("{key}: {e}")))?;
            Some(
                table
                    .entries
                    .into_iter()
                    .map(|e| GlyphAlias { final_name: e.final_name, source_name: e.source_name, code_points: e.code_points })
                    .collect(),
            )
        }
        None => None,
    };
    let feature_file = match p.opt_str("featureFile")? {
        Some(path) => Some((PathBuf::from(path), "`featureFile`")),
        None => from_project(&settings.paths.features, "`projectFile` FeaturePath").or_else(|| {
            if defaults { DEFAULT_FEATURE_FILES.iter().find_map(|name| beside(folder, name)).map(|f| (f, "`featureFile`")) } else { None }
        }),
    };
    let (features, feature_dir) = match feature_file {
        Some((path, key)) => {
            let code = text(p, key, &path)?;
            (Some(code), path.parent().filter(|d| readable(d)).map(Path::to_path_buf))
        }
        None => (None, None),
    };
    let mut cid = CidOptions { mac_script: settings.mac_script, mac_language: settings.mac_language, ..CidOptions::default() };
    for (relative, key, slot) in [
        (&settings.paths.unicode_cmap, "`projectFile` H_UniEncodingCMAPPath", &mut cid.unicode_cmap),
        (&settings.paths.mac_cmap, "`projectFile` MacEncodingCMAPPath", &mut cid.mac_cmap),
        (&settings.paths.uvs, "`projectFile` UVSPath", &mut cid.uvs),
    ] {
        if let Some((path, key)) = from_project(relative, key) {
            *slot = Some(bytes(p, key, &path)?);
        }
    }
    if let Some((input, _)) = from_project(&settings.paths.input, "`projectFile` InputFontPath") {
        let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
        if !source.is_some_and(|s| same(s, &input)) {
            notes.push(format!("the project file's InputFontPath names {}; the open source is built instead", input.display()));
        }
    }
    if settings.paths.output.is_some() && p.get("destination").is_some_and(|v| !v.is_null()) {
        notes.push("the project file's OutputFontPath is not used: `destination` is given".into());
    }
    Ok(Some((Project { settings, menu_db, aliases, features, feature_dir, cid }, notes)))
}

/// The file the project file's `OutputFontPath` names (a font file or a folder), relative to
/// the project file, which must lie inside the automation write root; `None` without a project
/// file or without that key.
pub(super) fn output_path(p: &Params) -> Result<Option<PathBuf>> {
    let Some(file) = p.opt_str("projectFile")? else { return Ok(None) };
    let file = PathBuf::from(file);
    let mut settings = makeotf::Settings::default();
    settings.apply_project(&text(p, "`projectFile`", &file)?).map_err(|e| p.bad(format!("`projectFile`: {e}")))?;
    let Some(output) = settings.paths.output else { return Ok(None) };
    let path = file.parent().map_or_else(|| PathBuf::from(&output), |d| d.join(&output));
    crate::file_access::check_write(&path).map_err(|e| p.bad(format!("`projectFile` OutputFontPath: {e}")))?;
    Ok(Some(path))
}

/// Applies `project` to `font` and sets the compile options it decides: the finishing steps,
/// subroutinising unless `subroutinize` is given, and overlaps kept, as the reference never
/// removes them, unless `outlines.removeOverlaps` is given.
pub(super) fn prepare(p: &Params, project: &Project, font: &Font, options: &mut fontcraft_compile::Options) -> Result<Prepared> {
    let prepared = project.prepare(font).map_err(|e| p.bad(e.to_string()))?;
    options.project = Some(prepared.finish.clone());
    if project.features.is_some() {
        options.feature_dir.clone_from(&project.feature_dir);
    }
    // The parameters (the command line) come before the project file's files and ids.
    let (given, from_project) = (&mut options.cid, &project.cid);
    for (slot, value) in
        [(&mut given.unicode_cmap, &from_project.unicode_cmap), (&mut given.mac_cmap, &from_project.mac_cmap), (&mut given.uvs, &from_project.uvs)]
    {
        if slot.is_none() {
            slot.clone_from(value);
        }
    }
    given.mac_script = given.mac_script.or(from_project.mac_script);
    given.mac_language = given.mac_language.or(from_project.mac_language);
    if !p.get("subroutinize").is_some_and(|v| !v.is_null()) {
        options.subroutinize = Some(prepared.subroutinize);
    }
    if !p.get("outlines").and_then(|o| o.get("removeOverlaps")).is_some_and(|v| !v.is_null()) {
        options.remove_overlap = false;
    }
    Ok(prepared)
}

/// The text of the file at `path`, which must lie inside the automation read root when one is in
/// force, since a project file can name any path. `key` names the parameter or project file key
/// in messages.
fn text(p: &Params, key: &str, path: &Path) -> Result<String> {
    crate::file_access::check_read(path).map_err(|e| p.bad(format!("{key}: {e}")))?;
    let input = super::file_io::read_input_up_to(path, makeotf::MAX_PROJECT_BYTES)?;
    if input.bytes.len() > makeotf::MAX_PROJECT_BYTES {
        return Err(p.bad(format!("{key}: {} is larger than {} bytes", path.display(), makeotf::MAX_PROJECT_BYTES)));
    }
    String::from_utf8(input.bytes).map_err(|_| p.bad(format!("{key}: {} is not UTF-8 text", path.display())))
}

/// The bytes of the CMap or variation sequence file at `path`, which must lie inside the
/// automation read root when one is in force.
fn bytes(p: &Params, key: &str, path: &Path) -> Result<Vec<u8>> {
    crate::file_access::check_read(path).map_err(|e| p.bad(format!("{key}: {e}")))?;
    let input = super::file_io::read_input_up_to(path, MAX_CMAP_BYTES)?;
    if input.bytes.len() > MAX_CMAP_BYTES {
        return Err(p.bad(format!("{key}: {} is larger than {MAX_CMAP_BYTES} bytes", path.display())));
    }
    Ok(input.bytes)
}

/// A file called `name` in `folder`, when automation may read it.
fn beside(folder: Option<&Path>, name: &str) -> Option<PathBuf> {
    folder.map(|f| f.join(name)).filter(|f| readable(f) && f.is_file())
}

/// May automation read `path`? A search candidate it may not read counts as missing.
fn readable(path: &Path) -> bool {
    crate::file_access::check_read(path).is_ok()
}

/// A file called `name` in `folder` or up to [`LEVELS_UP`] folders above it, skipping folders
/// automation may not read.
fn near(folder: Option<&Path>, name: &str) -> Option<PathBuf> {
    let mut dir = folder;
    for _ in 0..=LEVELS_UP {
        let d = dir?;
        let candidate = d.join(name);
        if readable(&candidate) && candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}
