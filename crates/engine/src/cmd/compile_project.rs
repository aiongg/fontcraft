//! The project compile of `font.compile` (M12.40; `fontcraft_compile::makeotf`): the project
//! file, the option words, the `fontinfo` file, the FontMenuNameDB and the GlyphOrderAndAliasDB
//! named in the parameters, or found beside the source with `projectDefaults`, read and parsed
//! here and applied by the compile crate.

use std::path::{Path, PathBuf};

use fontcraft_compile::makeotf::{self, GlyphAlias, Prepared, Project};
use fontcraft_model::Font;

use crate::Result;
use crate::params::Params;

/// The parameters that make `font.compile` a project compile.
const KEYS: &[&str] = &["projectFile", "projectOptions", "projectDefaults", "menuNames", "goadb", "fontinfo"];

/// How many folders above the source's folder a FontMenuNameDB or GlyphOrderAndAliasDB is looked
/// for, as the reference looks for them.
const LEVELS_UP: usize = 4;

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
        let text = text(p, "projectFile", &path)?;
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
    let fontinfo = match p.opt_str("fontinfo")? {
        Some(path) => Some(PathBuf::from(path)),
        None if defaults => folder.map(|f| f.join("fontinfo")).filter(|f| f.is_file()),
        None => None,
    };
    if let Some(path) = fontinfo {
        settings.apply_fontinfo(&text(p, "fontinfo", &path)?).map_err(|e| p.bad(format!("`fontinfo`: {e}")))?;
    }
    let from_project = |relative: &Option<String>| -> Option<PathBuf> {
        relative.as_ref().map(|r| project_dir.as_deref().map_or_else(|| PathBuf::from(r), |d| d.join(r)))
    };
    let menu = match p.opt_str("menuNames")? {
        Some(path) => Some(PathBuf::from(path)),
        None => from_project(&settings.paths.menu_names).or_else(|| if defaults { near(folder, "FontMenuNameDB") } else { None }),
    };
    let goadb = match p.opt_str("goadb")? {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            from_project(&settings.paths.goadb).or_else(|| if defaults && settings.apply_goadb { near(folder, "GlyphOrderAndAliasDB") } else { None })
        }
    };
    let menu_db = match menu {
        Some(path) => Some(makeotf::parse_menu_db(&text(p, "menuNames", &path)?).map_err(|e| p.bad(format!("`menuNames`: {e}")))?),
        None => None,
    };
    let aliases = match goadb {
        Some(path) => {
            let table = fontcraft_tables::goadb::parse(&text(p, "goadb", &path)?).map_err(|e| p.bad(format!("`goadb`: {e}")))?;
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
    for (key, set) in [
        ("InputFontPath", settings.paths.input.is_some()),
        ("OutputFontPath", settings.paths.output.is_some()),
        ("FeaturePath", settings.paths.features.is_some()),
    ] {
        if set {
            notes
                .push(format!("the project file's {key} is not used: the open source is built, with its own feature code, to the destination given"));
        }
    }
    Ok(Some((Project { settings, menu_db, aliases }, notes)))
}

/// Applies `project` to `font` and sets the compile options it decides: the finishing steps,
/// subroutinising unless `subroutinize` is given, and overlaps kept, as the reference never
/// removes them, unless `outlines.removeOverlaps` is given.
pub(super) fn prepare(p: &Params, project: &Project, font: &Font, options: &mut fontcraft_compile::Options) -> Result<Prepared> {
    let prepared = project.prepare(font).map_err(|e| p.bad(e.to_string()))?;
    options.project = Some(prepared.finish.clone());
    if !p.get("subroutinize").is_some_and(|v| !v.is_null()) {
        options.subroutinize = Some(prepared.subroutinize);
    }
    if !p.get("outlines").and_then(|o| o.get("removeOverlaps")).is_some_and(|v| !v.is_null()) {
        options.remove_overlap = false;
    }
    Ok(prepared)
}

/// The text of the file `key` names.
fn text(p: &Params, key: &str, path: &Path) -> Result<String> {
    let input = super::file_io::read_input(path)?;
    if input.bytes.len() > makeotf::MAX_PROJECT_BYTES {
        return Err(p.bad(format!("`{key}`: {} is larger than {} bytes", path.display(), makeotf::MAX_PROJECT_BYTES)));
    }
    String::from_utf8(input.bytes).map_err(|_| p.bad(format!("`{key}`: {} is not UTF-8 text", path.display())))
}

/// A file called `name` in `folder` or up to [`LEVELS_UP`] folders above it.
fn near(folder: Option<&Path>, name: &str) -> Option<PathBuf> {
    let mut dir = folder;
    for _ in 0..=LEVELS_UP {
        let d = dir?;
        let candidate = d.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}
