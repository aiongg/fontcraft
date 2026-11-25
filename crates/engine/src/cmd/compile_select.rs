//! What `font.compile` and `font.compilePlan` build, and where each file goes (M2.8): the formats
//! (static, variable, and the ones still refused), the instances picked by name or by pattern,
//! the variable fonts `variableFonts` picks, `familyName`, and the folder layout and naming of
//! the files (`outputLayout`, `nameBy`).
//!
//! The by-format folder names, the full-match patterns of `instances` and `variableFonts` and the
//! rule of `familyName` follow fontmake 3.12.1's `font_project.py` (`_output_dir`,
//! `_search_instances`, `build_variable_fonts`) and glyphsLib's `filter_instances_by_family`
//! (both Apache-2.0, see `NOTICE`).

use fontcraft_compile::var::{self, VarFlavour, VariableOptions};
use fontcraft_compile::{Flavour, Options};
use fontcraft_glyphquery::Pattern;
use fontcraft_model::{Font, Instance};
use serde_json::{Value, json};

use super::compile::{ByContainer, Container, Dest, Job, Pick, Report, Request, Sink, format_name, outlines_name};
use crate::params::Params;
use crate::{EngineError, Result};

/// Most variable fonts one command builds.
pub const MAX_VARIABLE_FONTS: usize = 256;

/// How the files of a destination folder are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum OutputLayout {
    /// Every file in the destination folder; with both TrueType and CFF outlines, each in a
    /// `ttf` or `otf` subfolder, so their WOFF files do not overwrite each other.
    #[default]
    Flat,
    /// One subfolder per kind of font and extension, named as fontmake names them
    /// ([`by_format_folder`]).
    ByFormat,
}

/// Where the files of a compile go inside the destination, and what names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Naming {
    pub layout: OutputLayout,
    /// `nameBy: "postscript-name"`: name every file by the font's PostScript name, ignoring an
    /// instance's fileName parameter.
    pub postscript_names: bool,
    /// The request writes both TrueType and CFF outlines (static or variable): the flat layout
    /// then splits them into `ttf` and `otf` subfolders.
    pub split_outlines: bool,
}

/// What a by-format folder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FolderKind {
    /// Masters, built when no instance was asked for.
    Master,
    /// Instances, and masters built as instances.
    Instance,
    Variable,
    /// Master fonts that keep their overlaps and share their point structure.
    #[allow(dead_code)] // The interpolatable formats are refused until M2.9 builds them.
    Interpolatable,
}

/// The folder fontmake writes a font of `kind` with the extension `ext` into: `master_ttf`,
/// `instance_otf`, `variable_ttf`, `master_ttf_interpolatable`.
pub(super) fn by_format_folder(kind: FolderKind, ext: &str) -> String {
    match kind {
        FolderKind::Master => format!("master_{ext}"),
        FolderKind::Instance => format!("instance_{ext}"),
        FolderKind::Variable => format!("variable_{ext}"),
        FolderKind::Interpolatable => format!("master_{ext}_interpolatable"),
    }
}

/// The by-format folder of a static font built for `job`: fontmake writes every static font of a
/// run into `instance_*` when the run builds instances or masters as instances, and into
/// `master_*` otherwise, which here is the first master a font without instances builds.
pub(super) fn static_folder(job: &Job) -> FolderKind {
    if job.source == "default" { FolderKind::Master } else { FolderKind::Instance }
}

/// `outputLayout` and `nameBy`.
pub(super) fn naming(p: &Params, formats: &Formats) -> Result<Naming> {
    let layout = match p.opt_str("outputLayout")? {
        None | Some("flat") => OutputLayout::Flat,
        Some("by-format") => OutputLayout::ByFormat,
        Some(other) => return Err(p.bad(format!("`outputLayout` is \"flat\" or \"by-format\", got \"{other}\""))),
    };
    let postscript_names = match p.opt_str("nameBy")? {
        None | Some("filename") => false,
        Some("postscript-name") => true,
        Some(other) => return Err(p.bad(format!("`nameBy` is \"filename\" or \"postscript-name\", got \"{other}\""))),
    };
    let truetype = formats.statics.contains(&Flavour::TrueType) || formats.variable.contains(&VarFlavour::TrueType);
    let cff = formats.statics.contains(&Flavour::Cff) || formats.variable.contains(&VarFlavour::Cff2);
    Ok(Naming { layout, postscript_names, split_outlines: truetype && cff })
}

// ---------- formats ----------

/// The formats a request builds.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Formats {
    /// Static fonts, one per instance or master picked.
    pub statics: Vec<Flavour>,
    /// Variable fonts, one per variable font picked.
    pub variable: Vec<VarFlavour>,
}

impl Formats {
    /// The format name of a variable flavour in `formats`.
    pub(super) fn variable_name(flavour: VarFlavour) -> &'static str {
        match flavour {
            VarFlavour::TrueType => "variable",
            VarFlavour::Cff2 => "variable-cff2",
        }
    }

    /// The outline flavour of a variable font's file (its extension).
    pub(super) fn file_flavour(flavour: VarFlavour) -> Flavour {
        match flavour {
            VarFlavour::TrueType => Flavour::TrueType,
            VarFlavour::Cff2 => Flavour::Cff,
        }
    }

    /// The outline flavour of each format's files, static formats first: a static and a variable
    /// TrueType format give TrueType twice.
    pub(super) fn file_flavours(&self) -> Vec<Flavour> {
        self.statics.iter().copied().chain(self.variable.iter().map(|&v| Self::file_flavour(v))).collect()
    }
}

/// The parameters that ask for static fonts: given, they keep a font with several masters from
/// defaulting to a variable font, and with only variable formats they are refused.
const STATIC_ONLY: &[&str] =
    &["instances", "mastersAsInstances", "collection", "interpolateBinaryLayout", "cbdt", "passThroughTables", "stubDsig", "subset"];

/// The parameter of `STATIC_ONLY` the request gives, other than as `false`, if any.
fn static_only(p: &Params) -> Option<&'static str> {
    STATIC_ONLY.iter().copied().find(|k| p.get(k).is_some_and(|v| v != &Value::Bool(false)))
}

/// Whether `dest` names a collection file (`.ttc`, `.otc`), which holds static fonts.
fn collection_destination(dest: &Dest) -> bool {
    match dest {
        Dest::Folder(path) | Dest::File(path) => {
            path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ttc") || e.eq_ignore_ascii_case("otc"))
        }
        Dest::Memory => false,
    }
}

/// `formats`. Without it a font with one master, or with no axis, builds a static TTF, and one
/// with several masters a variable TTF, unless the request asks for static fonts
/// ([`STATIC_ONLY`], a project compile, a `.otf` or collection destination file).
pub(super) fn formats(p: &Params, font: &Font, project: bool, dest: &Dest) -> Result<Formats> {
    let Some(names) = p.opt_strings("formats")? else {
        let variable = font.masters.len() > 1
            && !font.axes.is_empty()
            && !project
            && static_only(p).is_none()
            && !matches!(dest.extension(), Some("otf"))
            && !collection_destination(dest);
        return Ok(if variable {
            Formats { statics: Vec::new(), variable: vec![VarFlavour::TrueType] }
        } else {
            Formats { statics: vec![Flavour::TrueType], variable: Vec::new() }
        });
    };
    let mut out = Formats::default();
    for name in names {
        match name {
            "ttf" | "otf" => {
                let flavour = if name == "ttf" { Flavour::TrueType } else { Flavour::Cff };
                if !out.statics.contains(&flavour) {
                    out.statics.push(flavour);
                }
            }
            "variable" | "variable-cff2" => {
                let flavour = if name == "variable" { VarFlavour::TrueType } else { VarFlavour::Cff2 };
                if !out.variable.contains(&flavour) {
                    out.variable.push(flavour);
                }
            }
            "otf-cff2" | "ttf-interpolatable" | "otf-interpolatable" => {
                return Err(EngineError::Unsupported(format!("building {name} fonts (ROADMAP.md, M2.9)")));
            }
            other => {
                return Err(p.bad(format!(
                    "`formats` holds \"ttf\", \"otf\", \"variable\", \"variable-cff2\", \"otf-cff2\", \"ttf-interpolatable\" or \"otf-interpolatable\", got \"{other}\""
                )));
            }
        }
    }
    if out.statics.is_empty() && out.variable.is_empty() {
        return Err(p.bad("`formats` names no format"));
    }
    if !out.variable.is_empty() {
        if project {
            return Err(p.bad("a project compile builds static fonts: leave the variable formats out of `formats`"));
        }
        if out.statics.is_empty()
            && let Some(key) = static_only(p)
        {
            return Err(p.bad(format!("`{key}` applies to static fonts, and `formats` names only variable ones")));
        }
        for key in ["collection", "interpolateBinaryLayout"] {
            if p.get(key).is_some_and(|v| v != &Value::Bool(false)) {
                return Err(
                    p.bad(format!("`{key}` cannot go with a variable format in `formats`: build the variable fonts in a request of their own"))
                );
            }
        }
    }
    Ok(out)
}

// ---------- instances ----------

/// A pattern parameter (`instances`, `variableFonts`): a regular expression that must match a
/// whole name, as fontmake's `-i` and `--variable-fonts` match.
fn pattern(p: &Params, key: &str, source: &str) -> Result<Pattern> {
    Pattern::new(source, false).map_err(|e| p.bad(format!("`{key}`: {e}")))
}

/// `instances`: true (every exportable instance, the default), false, a list of names, or one
/// string, a regular expression matched against each exportable instance's name and its family
/// and style name (`Noto Sans Bold`).
pub(super) fn pick(p: &Params) -> Result<Pick> {
    match p.get("instances") {
        Some(Value::String(source)) => Ok(Pick::Pattern(pattern(p, "instances", source)?)),
        _ => Pick::read(p, "instances"),
    }
}

/// The family an instance belongs to: its own family name (the familyNames property or the
/// familyName parameter), else the font's.
pub(super) fn family_of<'f>(font: &'f Font, inst: &'f Instance) -> &'f str {
    inst.properties
        .value("familyNames")
        .or_else(|| inst.params.get("familyName").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(font.family_name.as_str())
}

/// Whether the static instance `inst` (number `i`) matches `pattern`, by its name or by its family
/// and style name.
pub(super) fn instance_matches(font: &Font, i: usize, inst: &Instance, pattern: &Pattern) -> bool {
    let label = if inst.name.is_empty() { format!("#{i}") } else { inst.name.clone() };
    pattern.is_match(&label) || pattern.is_match(&format!("{} {}", family_of(font, inst), inst.name))
}

/// `familyName`: the family name the masters and variable fonts are built with; only instances of
/// that family are built. Returns the name, after dropping from `jobs` the instances of other
/// families, which are added to `skipped`.
pub(super) fn family(p: &Params, font: &Font, jobs: &mut Vec<Job>, skipped: &mut Vec<Value>) -> Result<Option<String>> {
    let Some(family) = p.opt_str("familyName")? else { return Ok(None) };
    let family = family.trim();
    if family.is_empty() {
        return Err(p.bad("`familyName` is empty"));
    }
    jobs.retain(|job| {
        let fontcraft_model::Target::Instance(i) = &job.target else { return true };
        let Some(inst) = font.instances.get(*i) else { return true };
        let own = family_of(font, inst);
        if own == family {
            return true;
        }
        skipped.push(json!({ "instance": job.label, "source": job.source, "reason": format!("its family is {own}, not the `familyName` {family}") }));
        false
    });
    Ok(Some(family.to_string()))
}

/// `font` with the family name `family`, as fontmake gives it to the masters.
pub(super) fn renamed(font: &Font, family: Option<&str>) -> Option<Font> {
    let family = family?;
    (font.family_name != family).then(|| Font { family_name: family.to_string(), ..font.clone() })
}

// ---------- variable fonts ----------

/// One variable font to build: a variable font setting (an index into `Font::instances`), or
/// `None` for the whole designspace.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct VariablePick {
    pub setting: Option<usize>,
    /// The setting's name, `default` for the whole designspace.
    pub label: String,
    /// The file name without extension.
    pub stem: String,
}

/// The variable fonts `variableFonts` picks: every one (the default), the settings a list
/// names, or those whose setting name or file name a regular expression matches in full. A
/// pattern that matches none is a warning, as in fontmake.
pub(super) fn variable_picks(p: &Params, font: &Font, warnings: &mut Vec<String>) -> Result<Vec<VariablePick>> {
    let targets = var::variable_targets(font);
    if targets.len() > MAX_VARIABLE_FONTS {
        return Err(EngineError::Unsupported(format!("a font with more than {MAX_VARIABLE_FONTS} variable font settings")));
    }
    let mut all = Vec::with_capacity(targets.len());
    for setting in targets {
        let stem = var::names::variable_names(font, setting)?.file_stem;
        let label = setting.and_then(|i| font.instances.get(i)).map_or_else(|| "default".to_string(), |i| i.name.clone());
        all.push(VariablePick { setting, label, stem });
    }
    match p.get("variableFonts") {
        None => Ok(all),
        Some(Value::String(source)) => {
            let pattern = pattern(p, "variableFonts", source)?;
            all.retain(|v| pattern.is_match(&v.label) || pattern.is_match(&v.stem));
            if all.is_empty() {
                warnings.push(format!("no variable font matches `variableFonts` \"{source}\""));
            }
            Ok(all)
        }
        Some(_) => {
            let names = p.opt_strings("variableFonts")?.unwrap_or_default();
            if names.len() > MAX_VARIABLE_FONTS {
                return Err(p.bad(format!("`variableFonts` names more than {MAX_VARIABLE_FONTS} variable fonts")));
            }
            let mut out: Vec<VariablePick> = Vec::with_capacity(names.len());
            for name in names {
                let found = all.iter().find(|v| v.setting.is_some() && v.label == name);
                let found =
                    found.ok_or_else(|| p.bad(format!("`variableFonts` names \"{name}\", which is not a variable font setting of the font")))?;
                if !out.contains(found) {
                    out.push(found.clone());
                }
            }
            Ok(out)
        }
    }
}

/// The file a variable font of `stem` is written to, relative to the destination folder.
pub(super) fn variable_file_name(req: &Request, stem: &str, flavour: VarFlavour, container: Container) -> String {
    let file_flavour = Formats::file_flavour(flavour);
    let ext = container.extension(file_flavour);
    match req.naming.layout {
        OutputLayout::ByFormat => format!("{}/{stem}.{ext}", by_format_folder(FolderKind::Variable, format_name(file_flavour))),
        OutputLayout::Flat if req.naming.split_outlines => format!("{}/{stem}.{ext}", format_name(file_flavour)),
        OutputLayout::Flat => format!("{stem}.{ext}"),
    }
}

/// The options of a variable build: the static request's, without what only static fonts take.
fn variable_options(req: &Request) -> Options {
    let o = &req.options;
    Options {
        timestamp: o.timestamp,
        conversion_error: o.conversion_error,
        keep_direction: o.keep_direction,
        glyph_data: o.glyph_data.clone(),
        components: o.components,
        assets: o.assets.clone(),
        skip_features: o.skip_features,
        feature_dir: o.feature_dir.clone(),
        include_check: o.include_check.clone(),
        version_stamp: o.version_stamp,
        curves: o.curves,
        stub_gsub: o.stub_gsub,
        post_format: o.post_format,
        ..Options::default()
    }
}

/// The plan's description of the variable fonts a request builds, with the files they would be
/// written to.
pub(super) fn plan_variable(
    font: &Font,
    picks: &[VariablePick],
    flavours: &[VarFlavour],
    req: &Request,
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for pick in picks {
        let names = var::names::variable_names(font, pick.setting)?;
        let containers = crate::export_steps::setting_containers(font, pick.setting, &req.containers, warnings);
        for &flavour in flavours {
            let files: Vec<Value> = containers
                .iter()
                .map(|&c| {
                    let name = variable_file_name(req, &pick.stem, flavour, c);
                    match plan_path(&req.dest, &name) {
                        Some(path) => json!({ "name": name, "container": c.name(), "path": path.to_string_lossy() }),
                        None => json!({ "name": name, "container": c.name() }),
                    }
                })
                .collect();
            out.push(json!({
                "instance": pick.label, "source": "variable", "format": Formats::variable_name(flavour),
                "postscriptName": names.postscript_name, "style": names.style, "removeOverlap": false, "files": files,
            }));
        }
    }
    Ok(out)
}

/// Where a file named `name` (relative to the destination folder) lands; `None` in memory.
pub(super) fn plan_path(dest: &Dest, name: &str) -> Option<std::path::PathBuf> {
    match dest {
        Dest::Memory => None,
        Dest::File(path) => Some(path.clone()),
        Dest::Folder(root) => Some(name.split('/').fold(root.clone(), |p, part| p.join(part))),
    }
}

/// Builds the variable fonts `picks` of `font` (its linked sources already read) in each of
/// `flavours` into `sink`, adding their results to `fonts` and the sink's warnings to `report`.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_variable(
    font: &Font,
    picks: &[VariablePick],
    flavours: &[VarFlavour],
    req: &Request,
    optimize_deltas: Option<bool>,
    sink: &mut Sink,
    report: &mut Report,
    fonts: &mut Vec<Value>,
) -> Result<()> {
    let options = variable_options(req);
    for pick in picks {
        let containers = crate::export_steps::setting_containers(font, pick.setting, &req.containers, &mut report.warnings);
        for &flavour in flavours {
            let mut stem = String::new();
            let mut built = ByContainer::new(fontcraft_compile::export_params::may_differ_by_container(font));
            let mut files = Vec::with_capacity(containers.len());
            for &container in &containers {
                let compiled = built.get(container, |web_container| {
                    let v = var::compile_variable(
                        font,
                        pick.setting,
                        &Options { web_container, ..options.clone() },
                        &VariableOptions { flavour, optimize_deltas },
                    )?;
                    stem = v.file_stem;
                    Ok(v.compiled)
                })?;
                let data = container.wrap(&compiled.bytes)?;
                let name = variable_file_name(req, &stem, flavour, container);
                let file = sink.put(name, container, data, &mut report.warnings)?;
                let mut v = json!({ "name": file.name, "container": container.name(), "bytes": file.bytes });
                if let (Some(path), Value::Object(m)) = (&file.path, &mut v) {
                    m.insert("path".into(), json!(path.to_string_lossy()));
                }
                files.push(v);
            }
            let Some((compiled, warnings)) = built.finish() else { continue };
            if report.feature_code.is_none() {
                report.feature_code = compiled.feature_code;
            }
            fonts.push(json!({
                "instance": pick.label, "source": "variable", "format": Formats::variable_name(flavour),
                "outlines": outlines_name(Formats::file_flavour(flavour)), "fileName": stem,
                "glyphs": compiled.glyph_order.len(), "warnings": warnings, "files": files,
            }));
        }
    }
    Ok(())
}
