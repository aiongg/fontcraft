//! Export profiles and glyph artwork. Rows: `file.exportWithProfiles`, `file.saveExportProfile`,
//! `file.removeExportProfile`, `file.listExportProfiles`, `glyph.exportArtwork`.
//!
//! A profile is a named, saved set of the static export's settings: an output subfolder, the
//! outline format, the containers, overlap removal and autohinting. Built-in profiles are fixed
//! and read-only; the user's own live in the application settings under
//! [`fontcraft_model::fc_keys::EXPORT_PROFILES`] as `[{name, params}]`. The settings file is
//! untrusted: every stored profile is checked when it is read, and one that fails is reported and
//! left out, never used.
//!
//! `file.exportWithProfiles` builds every chosen font through every chosen profile with the same
//! code as `file.exportAll`, staging all files first and moving them into place only when every
//! font was built, so a failure writes nothing. A profile's subfolder is a relative path that
//! never climbs out of the destination, whoever calls.
//!
//! `glyph.exportArtwork` writes one picture per glyph: SVG, PDF or PNG. Every picture spans the
//! glyph's advance width (or its outline, where that reaches further) and the master's descender
//! to ascender, so pictures of one font line up.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use fontcraft_compile::{Flavour, Options};
use fontcraft_geom::{Affine, BezPath, PathEl, Point, Rect, Shape as _};
use fontcraft_model::{Font, Layer};
use serde_json::{Map, Value, json};

use super::compile::{self, Container, Dest, Pick, Report, Request, Sink};
use super::*;
use crate::EngineError;
use crate::params::Params;
use crate::settings::Reserved;

/// Most profiles: built-in and the user's together.
pub const MAX_PROFILES: usize = 256;
/// Longest profile name, in characters.
pub const MAX_NAME_CHARS: usize = 128;
/// Longest output subfolder, in bytes.
pub const MAX_FOLDER_BYTES: usize = 1024;
/// Most folder levels in an output subfolder.
pub const MAX_FOLDER_SEGMENTS: usize = 16;
/// Most fonts named by `fonts`.
const MAX_FONT_REFS: usize = 1024;
/// Largest pixels-per-em of a PNG picture.
pub const MAX_PIXELS_PER_EM: f64 = 16_384.0;
/// Most bytes of pictures returned in a result (without a directory).
pub const MAX_RETURNED_ARTWORK: usize = 256 << 20;
/// Longest glyph-name part of a picture's file name, in characters.
const MAX_FILE_STEM: usize = 200;
/// Largest PDF page side, in points; larger pictures are scaled down to fit.
const PDF_MAX_PAGE: f64 = 14_400.0;

/// Most entries of a profile's `containers`.
const MAX_CONTAINERS: usize = 3;
/// The keys of a profile's `params`.
const SETTING_KEYS: [&str; 5] = ["folder", "outlines", "containers", "removeOverlap", "autohint"];

/// The built-in profiles: name, outline format, containers. Each writes into a subfolder named
/// after it.
const BUILT_IN: [(&str, &str, &[&str]); 3] =
    [("OpenType PS", "cff", &["plain"]), ("OpenType TT", "truetype", &["plain"]), ("Web", "truetype", &["woff2", "woff"])];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.exportWithProfiles",
            "Export Fonts As…",
            ["File"],
            None,
            "{fonts?: [title or path] (default: the active document), profiles: [name], destination?: folder, bySource?: bool, installTemporarily?: bool, timestamp?: number|\"now\"} export every listed open font through every listed export profile in one run: each profile's fonts go into its output subfolder of the destination, or with bySource: true of the folder that holds each font's source file (give one or the other). Each profile builds the font's exportable instances as file.exportAll does, in the profile's outline format and containers, with its overlap removal; instances between masters are skipped and listed. Two profiles that would write the same file are refused. A failing export writes nothing. installTemporarily (installing the fonts until the app quits) is not supported yet → {profiles: [{profile, documents: [{title, destination, fonts, skipped}]}], written, warnings}",
            has_doc,
            export_with_profiles
        ).files(const { &[FileParam::write("destination")] }).writes_files(),
        cmd!(
            "file.saveExportProfile",
            "Save Export Profile",
            [],
            None,
            "{name: string, basedOn?: profile name, settings: {folder?: relative folder, outlines?: \"truetype\"|\"cff\", containers?: [\"plain\"|\"woff\"|\"woff2\"], removeOverlap?: bool, autohint?: bool}} create or update one of the user's export profiles, kept in the application settings. The settings start from basedOn's, else from the profile's own when it exists, else from the defaults (truetype, plain, removeOverlap true, autohint false), and `settings` replaces what it names. folder: the output subfolder, `/`-separated, relative and never climbing out with `..`; empty writes into the destination itself; a new profile's folder defaults to its name. Built-in profiles are read-only: their names are refused; at most 256 profiles → {profile: {name, builtIn, params}, created: bool}",
            always,
            save_profile
        ),
        cmd!(
            "file.removeExportProfile",
            "Remove Export Profile",
            [],
            None,
            "{name: string} remove one of the user's export profiles; built-in profiles cannot be removed → {removed: name, profiles: count left}",
            always,
            remove_profile
        ),
        cmd!(query
            "file.listExportProfiles",
            "List Export Profiles",
            [],
            None,
            "{} the export profiles: the built-in ones first, then the user's in the order they were made. problems lists stored profiles that cannot be read, which are left out → {profiles: [{name, builtIn, params: {folder, outlines, containers, removeOverlap, autohint}}], problems: [text]}",
            always,
            list_profiles
        ),
        cmd!(
            "glyph.exportArtwork",
            "Export Artwork…",
            ["File", "Export"],
            None,
            "{glyphs?: [name] (default: every glyph), layer?: id (default: the first master's layer), format: \"svg\"|\"pdf\"|\"png\", directory?: folder, pixelsPerEm?: number (PNG only, 1 to 16384, default 256)} write one picture per glyph of the chosen layer, named after the glyph (characters a file name cannot hold become `_`; names that differ only in case get a number). A picture spans the advance width and the master's descender to ascender, widened to the outline where it reaches further; components are drawn, open paths are not. SVG and PDF are in font units (one point per unit, a PDF scaled down to at most 14400 points a side); PNG is black on a transparent background. Glyphs without the layer are skipped and listed. Without a directory the pictures are returned as dataBase64. A failing export writes nothing → {format, files: [{glyph, name, bytes, path? | dataBase64?}], skipped: [{glyph, reason}]}",
            has_doc,
            export_artwork
        ).files(const { &[FileParam::write("directory")] }).writes_files()
        .targets(&[TargetKind::Glyphs]),
    ]
}

// ---------- profiles ----------

/// One export profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub built_in: bool,
    /// Every key of [`SETTING_KEYS`], checked.
    pub params: Map<String, Value>,
}

impl Profile {
    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "builtIn": self.built_in, "params": self.params })
    }
}

fn built_in() -> Vec<Profile> {
    BUILT_IN
        .iter()
        .map(|(name, outlines, containers)| {
            let params = json!({ "folder": name, "outlines": outlines, "containers": containers, "removeOverlap": true, "autohint": false });
            Profile { name: (*name).to_string(), built_in: true, params: params.as_object().cloned().unwrap_or_default() }
        })
        .collect()
}

fn is_built_in(name: &str) -> bool {
    BUILT_IN.iter().any(|(n, _, _)| *n == name)
}

/// A profile name, trimmed, or why it cannot be one.
fn check_name(name: &str) -> std::result::Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a profile name must not be empty".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("a profile name has at most {MAX_NAME_CHARS} characters"));
    }
    if name.chars().any(char::is_control) {
        return Err("a profile name cannot hold control characters".into());
    }
    Ok(name.to_string())
}

/// An output subfolder as `/`-separated folder names, or why it cannot be one: absolute paths,
/// `..` and `.` are refused, so a profile never writes outside the destination.
fn check_folder(text: &str) -> std::result::Result<String, String> {
    if text.len() > MAX_FOLDER_BYTES {
        return Err(format!("the folder is longer than {MAX_FOLDER_BYTES} bytes"));
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') || Path::new(trimmed).is_absolute() {
        return Err(format!("\"{trimmed}\" is an absolute path; give a folder relative to the export destination"));
    }
    let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
    let segments: Vec<&str> = trimmed.split('/').collect();
    if segments.len() > MAX_FOLDER_SEGMENTS {
        return Err(format!("\"{trimmed}\" has more than {MAX_FOLDER_SEGMENTS} folder levels"));
    }
    for segment in &segments {
        if segment.trim().is_empty() {
            return Err(format!("\"{trimmed}\" has an empty folder name"));
        }
        if *segment == "." || *segment == ".." {
            return Err(format!("\"{trimmed}\" holds `{segment}`: a profile's folder stays inside the export destination"));
        }
        if let Some(c) = segment.chars().find(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
            return Err(format!("the folder name \"{segment}\" holds {c:?}, which folder names cannot hold"));
        }
    }
    Ok(segments.join("/"))
}

/// `settings` over `base`, every key checked; `folder` falls back to `default_folder`.
fn merge_settings(base: &Map<String, Value>, settings: &Map<String, Value>, default_folder: &str) -> std::result::Result<Map<String, Value>, String> {
    if let Some(key) = settings.keys().find(|k| !SETTING_KEYS.contains(&k.as_str())) {
        return Err(format!("`{key}` is not a profile setting; the settings are {}", SETTING_KEYS.join(", ")));
    }
    let pick = |key: &str| settings.get(key).or_else(|| base.get(key));
    let folder = match pick("folder") {
        None => check_folder(default_folder)?,
        Some(Value::String(s)) => check_folder(s)?,
        Some(_) => return Err("`folder` must be text".into()),
    };
    let outlines = match pick("outlines") {
        None => "truetype",
        Some(Value::String(s)) if s == "truetype" || s == "cff" => s.as_str(),
        Some(_) => return Err("`outlines` must be \"truetype\" or \"cff\"".into()),
    };
    let containers: Vec<&str> = match pick("containers") {
        None => vec!["plain"],
        Some(Value::Array(items)) => {
            let mut out: Vec<&str> = Vec::new();
            if items.len() > MAX_CONTAINERS {
                return Err(format!("`containers` names at most {MAX_CONTAINERS} containers"));
            }
            for item in items {
                match item.as_str() {
                    Some(c @ ("plain" | "woff" | "woff2")) if !out.contains(&c) => out.push(c),
                    Some("plain" | "woff" | "woff2") => {}
                    _ => return Err("`containers` holds \"plain\", \"woff\" or \"woff2\"".into()),
                }
            }
            if out.is_empty() {
                return Err("`containers` names no container".into());
            }
            out
        }
        Some(_) => return Err("`containers` must be a list".into()),
    };
    let flag = |key: &str, default: bool| match pick(key) {
        None => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("`{key}` must be true or false")),
    };
    let params = json!({
        "folder": folder, "outlines": outlines, "containers": containers,
        "removeOverlap": flag("removeOverlap", true)?, "autohint": flag("autohint", false)?,
    });
    Ok(params.as_object().cloned().unwrap_or_default())
}

/// The user's stored profiles that can be read, and a line for each that cannot.
fn stored(s: &Session) -> (Vec<Profile>, Vec<String>) {
    let mut problems = Vec::new();
    let value = match s.settings().reserved::<Value>(Reserved::ExportProfiles) {
        Ok(v) => v.unwrap_or(Value::Null),
        Err(e) => return (Vec::new(), vec![e.to_string()]),
    };
    let items = match value {
        Value::Null => return (Vec::new(), problems),
        Value::Array(items) => items,
        _ => return (Vec::new(), vec!["the stored export profiles are not a list; none is used".into()]),
    };
    let room = MAX_PROFILES.saturating_sub(BUILT_IN.len());
    if items.len() > room {
        problems.push(format!("{} export profiles are stored; only the first {room} are used", items.len()));
    }
    let mut out: Vec<Profile> = Vec::new();
    for (i, item) in items.iter().take(room).enumerate() {
        let name = item.get("name").and_then(Value::as_str).ok_or_else(|| "it has no name".to_string()).and_then(check_name);
        let checked = name.and_then(|name| {
            if is_built_in(&name) {
                return Err(format!("`{name}` is the name of a built-in profile"));
            }
            if out.iter().any(|p| p.name == name) {
                return Err(format!("`{name}` is stored twice"));
            }
            let params = item.get("params").and_then(Value::as_object).ok_or_else(|| format!("`{name}` has no params"))?;
            let params = merge_settings(&Map::new(), params, &name).map_err(|e| format!("`{name}`: {e}"))?;
            Ok(Profile { name, built_in: false, params })
        });
        match checked {
            Ok(p) => out.push(p),
            Err(e) => problems.push(format!("stored export profile {}: {e}; it is left out", i + 1)),
        }
    }
    (out, problems)
}

/// Every profile, built-in first, and the problems of the stored ones.
pub fn profiles(s: &Session) -> (Vec<Profile>, Vec<String>) {
    let (user, problems) = stored(s);
    let mut all = built_in();
    all.extend(user);
    (all, problems)
}

fn save_store(s: &mut Session, user: &[Profile]) -> Result<()> {
    let items: Vec<Value> = user.iter().map(|p| json!({ "name": p.name, "params": p.params })).collect();
    s.settings_mut().set_reserved(Reserved::ExportProfiles, &items)?;
    Ok(())
}

fn list_profiles(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("file.listExportProfiles", v)?;
    let (all, problems) = profiles(s);
    Ok(json!({ "profiles": all.iter().map(Profile::to_json).collect::<Vec<_>>(), "problems": problems }))
}

fn save_profile(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.saveExportProfile", v)?;
    let name = check_name(p.str("name")?).map_err(|e| p.bad(e))?;
    if is_built_in(&name) {
        return Err(p.bad(format!("`{name}` is a built-in profile and cannot be changed; save it under another name")));
    }
    let settings = p.opt_object("settings")?.ok_or_else(|| p.bad("`settings` is required (an object; {} keeps the base's settings)"))?;
    let (all, _) = profiles(s);
    let (mut user, _) = stored(s);
    let existing = user.iter().position(|q| q.name == name);
    let base: Map<String, Value> = match p.opt_str("basedOn")? {
        Some(based) => {
            let from = all.iter().find(|q| q.name == based).ok_or_else(|| p.bad(format!("there is no export profile `{based}`")))?;
            let mut base = from.params.clone();
            // A copy writes into its own folder unless the settings name one.
            if existing.is_none() || from.name != name {
                base.remove("folder");
            }
            base
        }
        None => existing.and_then(|i| user.get(i)).map(|q| q.params.clone()).unwrap_or_default(),
    };
    let params = merge_settings(&base, settings, &name).map_err(|e| p.bad(e))?;
    let profile = Profile { name: name.clone(), built_in: false, params };
    let created = match existing.and_then(|i| user.get_mut(i)) {
        Some(slot) => {
            *slot = profile.clone();
            false
        }
        None => {
            if user.len() + BUILT_IN.len() >= MAX_PROFILES {
                return Err(p.bad(format!("there are {MAX_PROFILES} export profiles already; remove one first")));
            }
            user.push(profile.clone());
            true
        }
    };
    save_store(s, &user)?;
    Ok(json!({ "profile": profile.to_json(), "created": created }))
}

fn remove_profile(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.removeExportProfile", v)?;
    let name = p.str("name")?.trim();
    if is_built_in(name) {
        return Err(p.bad(format!("`{name}` is a built-in profile and cannot be removed")));
    }
    let (mut user, _) = stored(s);
    let i = user.iter().position(|q| q.name == name).ok_or_else(|| p.bad(format!("there is no export profile `{name}` of the user's")))?;
    user.remove(i);
    save_store(s, &user)?;
    Ok(json!({ "removed": name, "profiles": user.len() + BUILT_IN.len() }))
}

// ---------- exporting through profiles ----------

/// What a profile asks the compiler for.
struct Plan {
    profile: Profile,
    flavour: Flavour,
    containers: Vec<Container>,
    remove_overlap: bool,
}

fn plan(p: &Params, profile: &Profile) -> Result<Plan> {
    let params = Value::Object(profile.params.clone());
    let pp = Params::new("file.exportWithProfiles", &params)?;
    let flavour = match pp.str("outlines")? {
        "cff" => Flavour::Cff,
        _ => Flavour::TrueType,
    };
    let containers = compile::containers(&pp, "containers", &Dest::Memory).map_err(|e| p.bad(format!("profile `{}`: {e}", profile.name)))?;
    if pp.bool_or("autohint", false)? {
        return Err(EngineError::Unsupported(format!(
            "profile `{}` asks for autohinting, which export does not support yet (ROADMAP.md, M10)",
            profile.name
        )));
    }
    Ok(Plan { profile: profile.clone(), flavour, containers, remove_overlap: pp.bool_or("removeOverlap", true)? })
}

/// The documents `fonts` names (titles or paths), else the active one, as indices.
fn chosen_documents(s: &Session, p: &Params) -> Result<Vec<usize>> {
    let docs = s.documents();
    let Some(names) = p.opt_strings("fonts")? else {
        let active = s.active().ok_or(EngineError::NoDocument)?;
        let i = docs.iter().position(|d| std::ptr::eq(d, active)).ok_or(EngineError::NoDocument)?;
        return Ok(vec![i]);
    };
    if names.len() > MAX_FONT_REFS {
        return Err(p.bad(format!("`fonts` names more than {MAX_FONT_REFS} fonts")));
    }
    let mut out = Vec::new();
    for name in names {
        let i = docs
            .iter()
            .position(|d| d.title == name || d.path.as_deref().is_some_and(|path| path == Path::new(name)))
            .ok_or_else(|| p.bad(format!("no open document is called or saved as `{name}`")))?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    if out.is_empty() {
        return Err(p.bad("`fonts` names no font"));
    }
    Ok(out)
}

/// `base` with the profile's `/`-separated folder below it.
fn below(base: &Path, folder: &str) -> PathBuf {
    folder.split('/').filter(|s| !s.is_empty()).fold(base.to_path_buf(), |path, part| path.join(part))
}

/// The folder that holds a document's source file.
fn source_folder(path: &Path) -> PathBuf {
    path.parent().filter(|d| !d.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn export_with_profiles(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.exportWithProfiles", v)?;
    if p.bool_or("installTemporarily", false)? {
        return Err(EngineError::Unsupported("`installTemporarily`: installing exported fonts until the app quits".into()));
    }
    let names = p.strings("profiles")?;
    if names.is_empty() {
        return Err(p.bad("`profiles` names no profile"));
    }
    if names.len() > MAX_PROFILES {
        return Err(p.bad(format!("`profiles` names more than {MAX_PROFILES} profiles")));
    }
    let (all, _) = profiles(s);
    let mut plans: Vec<Plan> = Vec::new();
    for name in names {
        let profile = all.iter().find(|q| q.name == name).ok_or_else(|| p.bad(format!("there is no export profile `{name}`")))?;
        if !plans.iter().any(|plan| plan.profile.name == profile.name) {
            plans.push(plan(&p, profile)?);
        }
    }
    let chosen = chosen_documents(s, &p)?;
    let by_source = p.bool_or("bySource", false)?;
    let base = match (p.opt_str("destination")?, by_source) {
        (Some(_), true) => return Err(p.bad("give a `destination` folder or bySource: true, not both")),
        (None, false) => return Err(p.bad("give a `destination` folder, or bySource: true to export next to each source file")),
        (Some(d), false) => {
            if d.trim().is_empty() {
                return Err(p.bad("`destination` is empty"));
            }
            let path = PathBuf::from(d);
            if path.exists() && !path.is_dir() {
                return Err(p.bad(format!("{} is a file: give a folder", path.display())));
            }
            Some(path)
        }
        (None, true) => None,
    };
    let timestamp = compile::timestamp(&p)?;
    let docs = s.documents();
    let mut jobs = Vec::with_capacity(chosen.len());
    for &i in &chosen {
        let doc = docs.get(i).ok_or(EngineError::NoDocument)?;
        if base.is_none() && doc.path.is_none() {
            return Err(p.bad(format!("`{}` has no file yet: bySource exports next to the source file, so save it first", doc.title)));
        }
        compile::check_glyph_names(&doc.font)?;
        jobs.push(compile::jobs(&doc.font, &Pick::Exportable, false, &p)?);
    }
    let fonts = plans.len().saturating_mul(jobs.iter().map(Vec::len).sum());
    if fonts > compile::MAX_FONTS {
        return Err(EngineError::Unsupported(format!(
            "building {fonts} fonts in one command (at most {}); choose fewer profiles or fonts",
            compile::MAX_FONTS
        )));
    }

    // Build everything into staged sinks; nothing moves into place until every font is built.
    let mut sinks: Vec<Sink> = Vec::new();
    let mut written: Vec<(PathBuf, String)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut results: Vec<Value> = Vec::with_capacity(plans.len());
    let mut skipped_all: Vec<Value> = Vec::new();
    let mut file_count = 0usize;
    for plan in &plans {
        let folder = plan.profile.params.get("folder").and_then(Value::as_str).unwrap_or_default();
        let options = Options { timestamp, remove_overlap: plan.remove_overlap, ..Default::default() };
        // One sink per destination folder: the profile's, or the profile's beside each source.
        let groups: Vec<(PathBuf, Vec<(usize, usize)>)> = match &base {
            Some(base) => vec![(below(base, folder), chosen.iter().copied().enumerate().collect())],
            None => chosen
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(k, i)| docs.get(i).and_then(|d| d.path.as_deref()).map(|path| (below(&source_folder(path), folder), vec![(k, i)])))
                .collect(),
        };
        let mut documents = Vec::new();
        for (dest_path, members) in groups {
            let req = Request {
                flavours: vec![plan.flavour],
                containers: plan.containers.clone(),
                options: options.clone(),
                dest: Dest::Folder(dest_path.clone()),
                first_only: false,
            };
            let mut sink = Sink::new(&req.dest, false, s.export_outside())?;
            for (k, i) in members {
                let doc = docs.get(i).ok_or(EngineError::NoDocument)?;
                let doc_jobs = jobs.get(k).ok_or(EngineError::NoDocument)?;
                let mut report = Report::default();
                compile::build(&doc.font, &doc.title, doc.path.as_deref(), s.glyph_data_of(doc)?, doc_jobs, &req, &mut sink, &mut report)?;
                for file in report.fonts.iter().flat_map(|f| f.files.iter()) {
                    let Some(path) = &file.path else { continue };
                    let key = path.to_string_lossy().to_lowercase();
                    if let Some((_, other)) = written.iter().find(|(p, _)| p.to_string_lossy().to_lowercase() == key) {
                        return Err(p.bad(format!(
                            "profiles `{other}` and `{}` both write {}: give them different folders",
                            plan.profile.name,
                            path.display()
                        )));
                    }
                    written.push((path.clone(), plan.profile.name.clone()));
                }
                file_count += report.file_count();
                warnings.append(&mut report.warnings);
                skipped_all.extend(report.skipped.iter().cloned());
                documents.push(json!({
                    "title": doc.title, "destination": dest_path.to_string_lossy(),
                    "fonts": report.fonts_json(false), "skipped": report.skipped,
                }));
            }
            sinks.push(sink);
        }
        results.push(json!({ "profile": plan.profile.name, "documents": documents }));
    }
    if file_count == 0 {
        return Err(compile::nothing_built(&Report { skipped: skipped_all, ..Default::default() }));
    }
    for sink in sinks {
        sink.commit()?;
    }
    Ok(json!({ "profiles": results, "written": file_count, "warnings": warnings }))
}

// ---------- artwork ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Artwork {
    Svg,
    Pdf,
    Png,
}

impl Artwork {
    fn extension(self) -> &'static str {
        match self {
            Artwork::Svg => "svg",
            Artwork::Pdf => "pdf",
            Artwork::Png => "png",
        }
    }
}

fn export_artwork(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.exportArtwork", v)?;
    let font = s.font()?;
    let format = match p.str("format")? {
        "svg" => Artwork::Svg,
        "pdf" => Artwork::Pdf,
        "png" => Artwork::Png,
        other => return Err(p.bad(format!("`format` must be \"svg\", \"pdf\" or \"png\", got \"{}\"", other.chars().take(64).collect::<String>()))),
    };
    let glyphs: Vec<String> = match p.opt_glyphs("glyphs", font)? {
        Some(g) if g.is_empty() => return Err(p.bad("`glyphs` is empty")),
        Some(g) => g,
        None => font.glyphs.iter().map(|g| g.name.clone()).collect(),
    };
    let layer_id = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let ppem = p.opt_f64("pixelsPerEm")?.unwrap_or(fontcraft_render::DEFAULT_EM_PIXELS);
    if !(1.0..=MAX_PIXELS_PER_EM).contains(&ppem) {
        return Err(p.bad(format!("`pixelsPerEm` must be from 1 to {MAX_PIXELS_PER_EM}")));
    }
    let directory = match p.opt_str("directory")? {
        None => None,
        Some(d) if d.trim().is_empty() => return Err(p.bad("`directory` is empty")),
        Some(d) => {
            let dir = PathBuf::from(d);
            if dir.exists() && !dir.is_dir() {
                return Err(p.bad(format!("{} is a file: give a folder", dir.display())));
            }
            Some(dir)
        }
    };
    let mut staged = match &directory {
        Some(dir) => Some(Staged::new(dir.clone())?),
        None => None,
    };
    let mut names = FileNames::default();
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut returned = 0usize;
    for name in &glyphs {
        let Some(glyph) = font.glyph(name) else { continue };
        let Some(layer) = glyph.layers.iter().find(|l| l.id == layer_id) else {
            skipped.push(json!({ "glyph": name, "reason": format!("the glyph has no layer `{layer_id}`") }));
            continue;
        };
        let bytes = artwork(font, layer, format, ppem).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
        let file = names.next(name, format.extension());
        let mut entry = json!({ "glyph": name, "name": file, "bytes": bytes.len() });
        match staged.as_mut() {
            Some(stage) => {
                let path = stage.put(&file, &bytes)?;
                if let Value::Object(m) = &mut entry {
                    m.insert("path".into(), json!(path.to_string_lossy()));
                }
            }
            None => {
                returned = returned.saturating_add(bytes.len());
                if returned > MAX_RETURNED_ARTWORK {
                    return Err(EngineError::Unsupported(format!(
                        "returning more than {} MB of pictures in one result; give a `directory` to write them to files",
                        MAX_RETURNED_ARTWORK >> 20
                    )));
                }
                if let Value::Object(m) = &mut entry {
                    m.insert("dataBase64".into(), json!(base64::engine::general_purpose::STANDARD.encode(&bytes)));
                }
            }
        }
        files.push(entry);
    }
    if files.is_empty() {
        return Err(p.bad(format!("none of the glyphs has the layer `{layer_id}`")));
    }
    if let Some(stage) = staged {
        stage.commit()?;
    }
    Ok(json!({ "format": format.extension(), "files": files, "skipped": skipped }))
}

/// File names for pictures: the glyph name with characters a file name cannot hold replaced, and
/// a number added where names would differ only in case.
#[derive(Default)]
struct FileNames {
    taken: HashSet<String>,
    next_number: HashMap<String, usize>,
}

impl FileNames {
    fn next(&mut self, glyph: &str, ext: &str) -> String {
        let stem: String = glyph
            .chars()
            .take(MAX_FILE_STEM)
            .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
            .collect();
        let stem = if stem.is_empty() { "_".to_string() } else { stem };
        let first = format!("{stem}.{ext}");
        if self.taken.insert(first.to_lowercase()) {
            return first;
        }
        let key = stem.to_lowercase();
        let mut n = self.next_number.get(&key).copied().unwrap_or(2);
        loop {
            let name = format!("{stem}_{n}.{ext}");
            n += 1;
            if self.taken.insert(name.to_lowercase()) {
                self.next_number.insert(key, n);
                return name;
            }
        }
    }
}

/// One picture of `layer`.
fn artwork(font: &Font, layer: &Layer, format: Artwork, ppem: f64) -> std::result::Result<Vec<u8>, String> {
    let outline = fontcraft_render::layer_outline(font, layer).map_err(|e| e.to_string())?;
    let outline = fontcraft_render::closed_contours(&outline);
    let frame = frame(font, layer, &outline)?;
    match format {
        Artwork::Svg => Ok(fontcraft_io_svg::bez_to_svg(&outline, Some(frame)).into_bytes()),
        Artwork::Pdf => Ok(pdf(&outline, frame)),
        Artwork::Png => png(&outline, frame, ppem / f64::from(font.units_per_em.max(1))),
    }
}

/// The area a picture shows: the advance width and the master's descender to ascender, widened
/// to the outline, at least one unit each way.
fn frame(font: &Font, layer: &Layer, outline: &BezPath) -> std::result::Result<Rect, String> {
    let master = font.master(&layer.master).or_else(|_| font.default_master()).map_err(|e| e.to_string())?;
    let metrics = super::common::master_metrics(font, master);
    let (mut x0, mut x1) = (0.0f64.min(layer.width), 0.0f64.max(layer.width));
    let (mut y0, mut y1) = (metrics.descender.min(metrics.ascender), metrics.descender.max(metrics.ascender));
    if !outline.elements().is_empty() {
        let b = outline.bounding_box();
        (x0, x1, y0, y1) = (x0.min(b.x0), x1.max(b.x1), y0.min(b.y0), y1.max(b.y1));
    }
    if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
        return Err("the layer has a coordinate or metric that is not a finite number".into());
    }
    if x1 - x0 < 1.0 {
        x1 = x0 + 1.0;
    }
    if y1 - y0 < 1.0 {
        y1 = y0 + 1.0;
    }
    Ok(Rect::new(x0, y0, x1, y1))
}

/// Black on transparent, `scale` pixels per unit.
fn png(outline: &BezPath, frame: Rect, scale: f64) -> std::result::Result<Vec<u8>, String> {
    let (w, h) = ((frame.width() * scale).ceil(), (frame.height() * scale).ceil());
    let side = f64::from(fontcraft_render::MAX_SIDE);
    if !(w.is_finite() && h.is_finite()) || w > side || h > side {
        return Err(format!("the picture would be larger than {side} pixels on a side; choose fewer pixels per em"));
    }
    let (width, height) = ((w as u32).max(1), (h as u32).max(1));
    let transform = Affine::translate((-frame.x0 * scale, frame.y1 * scale)) * Affine::scale_non_uniform(scale, -scale);
    let style = fontcraft_render::Style { ink: [0, 0, 0, 255], background: [0, 0, 0, 0], anti_alias: true };
    let image = fontcraft_render::render_path(outline, transform, width, height, &style).map_err(|e| e.to_string())?;
    image.png().map_err(|e| e.to_string())
}

/// `v` rounded to three decimals, in its shortest form, with no negative zero.
fn num(v: f64) -> String {
    let r = if v.is_finite() { (v * 1000.0).round() / 1000.0 } else { 0.0 };
    if r == 0.0 { "0".to_string() } else { format!("{r}") }
}

/// A one-page PDF that fills the outline, one point per unit (scaled down to fit the largest page).
fn pdf(outline: &BezPath, frame: Rect) -> Vec<u8> {
    use std::fmt::Write as _;
    let longest = frame.width().max(frame.height());
    let s = if longest > PDF_MAX_PAGE { PDF_MAX_PAGE / longest } else { 1.0 };
    let mut content = String::new();
    if !outline.elements().is_empty() {
        let _ = writeln!(content, "{} 0 0 {} {} {} cm", num(s), num(s), num(-frame.x0 * s), num(-frame.y0 * s));
        let (mut current, mut start) = (Point::ZERO, Point::ZERO);
        let pt = |q: Point| format!("{} {}", num(q.x), num(q.y));
        for el in outline.elements() {
            match *el {
                PathEl::MoveTo(q) => {
                    let _ = writeln!(content, "{} m", pt(q));
                    (current, start) = (q, q);
                }
                PathEl::LineTo(q) => {
                    let _ = writeln!(content, "{} l", pt(q));
                    current = q;
                }
                PathEl::QuadTo(c, q) => {
                    let c1 = current + (c - current) * (2.0 / 3.0);
                    let c2 = q + (c - q) * (2.0 / 3.0);
                    let _ = writeln!(content, "{} {} {} c", pt(c1), pt(c2), pt(q));
                    current = q;
                }
                PathEl::CurveTo(c1, c2, q) => {
                    let _ = writeln!(content, "{} {} {} c", pt(c1), pt(c2), pt(q));
                    current = q;
                }
                PathEl::ClosePath => {
                    content.push_str("h\n");
                    current = start;
                }
            }
        }
        content.push_str("f\n");
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << >> /Contents 4 0 R >>",
            num(frame.width() * s),
            num(frame.height() * s)
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = writeln!(tail, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF", objects.len() + 1);
    out.extend_from_slice(tail.as_bytes());
    out
}

// ---------- staging pictures on disk ----------

/// Pictures written under temporary names in the directory and renamed into place by
/// [`Staged::commit`]. Dropping it uncommitted removes the temporary files and the folders it made.
#[cfg(not(target_arch = "wasm32"))]
struct Staged {
    dir: PathBuf,
    made: Vec<PathBuf>,
    /// (temporary, final) paths.
    staged: Vec<(PathBuf, PathBuf)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Staged {
    fn new(dir: PathBuf) -> Result<Self> {
        let mut made = Vec::new();
        if !dir.is_dir() {
            let missing: Vec<PathBuf> = dir.ancestors().take_while(|d| !d.as_os_str().is_empty() && !d.exists()).map(Path::to_path_buf).collect();
            for d in missing.into_iter().rev() {
                if let Err(e) = std::fs::create_dir(&d) {
                    let stage = Self { dir: dir.clone(), made, staged: Vec::new() };
                    drop(stage);
                    return Err(EngineError::Io(format!("cannot create {}: {e}", d.display())));
                }
                made.push(d);
            }
        }
        Ok(Self { dir, made, staged: Vec::new() })
    }

    fn put(&mut self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        use std::io::Write as _;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let target = self.dir.join(name);
        let cannot = |e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", target.display()));
        let tmp = self.dir.join(format!(".fontcraft-artwork-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(cannot)?;
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(cannot(e));
        }
        self.staged.push((tmp, target.clone()));
        Ok(target)
    }

    fn commit(mut self) -> Result<()> {
        if let Some((_, target)) = self.staged.iter().find(|(_, target)| target.is_dir()) {
            return Err(EngineError::Io(format!("cannot write {}: a folder has that name", target.display())));
        }
        let staged = std::mem::take(&mut self.staged);
        let mut rest = staged.into_iter();
        for (tmp, target) in rest.by_ref() {
            if let Err(e) = std::fs::rename(&tmp, &target) {
                let _ = std::fs::remove_file(&tmp);
                self.staged = rest.collect();
                return Err(EngineError::Io(format!("cannot write {}: {e}", target.display())));
            }
        }
        self.made.clear();
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Staged {
    fn drop(&mut self) {
        for (tmp, _) in &self.staged {
            let _ = std::fs::remove_file(tmp);
        }
        for d in self.made.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
    }
}

/// On the web there is no directory to write to: the pictures come back in the result.
#[cfg(target_arch = "wasm32")]
struct Staged;

#[cfg(target_arch = "wasm32")]
impl Staged {
    fn new(_: PathBuf) -> Result<Self> {
        Err(EngineError::Unsupported("writing files in the browser; omit `directory` to get the pictures in the result".into()))
    }

    fn put(&mut self, _: &str, _: &[u8]) -> Result<PathBuf> {
        Err(EngineError::Unsupported("writing files in the browser".into()))
    }

    fn commit(self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::string_slice)]

    use super::*;

    #[test]
    fn profile_folders_stay_inside_the_destination() {
        assert_eq!(check_folder("Web/Fonts/").unwrap(), "Web/Fonts");
        assert_eq!(check_folder("  ").unwrap(), "");
        for bad in ["../up", "a/../../b", "/abs", "\\abs", "a//b", "./a", "a:b"] {
            assert!(check_folder(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn artwork_file_names_are_safe_and_differ_in_more_than_case() {
        let mut names = FileNames::default();
        assert_eq!(names.next("A", "svg"), "A.svg");
        assert_eq!(names.next("a", "svg"), "a_2.svg");
        assert_eq!(names.next("../x", "svg"), ".._x.svg");
        assert_eq!(names.next(".notdef", "svg"), ".notdef.svg");
    }

    #[test]
    fn pdf_offsets_point_at_their_objects() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.quad_to((50.0, 100.0), (100.0, 0.0));
        path.close_path();
        let bytes = pdf(&path, Rect::new(0.0, -200.0, 500.0, 800.0));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/MediaBox [0 0 500 1000]"));
        assert!(text.contains(" c\nh\nf\n"));
        let start: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(text[start..].starts_with("xref\n0 5\n"));
        for (i, line) in text[start..].lines().skip(3).take(4).enumerate() {
            let offset: usize = line[..10].parse().unwrap();
            assert!(text[offset..].starts_with(&format!("{} 0 obj", i + 1)), "object {}", i + 1);
        }
    }
}
