//! Export profiles and glyph artwork. Rows: `file.exportWithProfiles`, `file.saveExportProfile`,
//! `file.removeExportProfile`, `file.listExportProfiles`, `glyph.exportArtwork`,
//! `filter.glyphAsImage`, `color.fullColor.imageOnlyEffects`.
//!
//! A profile is a named, saved set of the static export's settings: an output subfolder, the
//! outline format, the containers, overlap removal, autohinting and the `CBDT` bitmap tables, and a `compile`
//! group of `font.compile` settings ([`COMPILE_KEYS`]). Built-in profiles are fixed
//! and read-only; the user's own live in the application settings under
//! [`fontcraft_model::fc_keys::EXPORT_PROFILES`] as `[{name, params}]`. The settings file is
//! untrusted: every stored profile is checked when it is read, and one that fails is reported and
//! left out, never used.
//!
//! `file.exportWithProfiles` builds every chosen font through every chosen profile with the same
//! code as `file.exportAll`, staging all files first and moving them into place only when every
//! font was built. All profiles' files move into place as one step that is undone whole when a
//! move fails, so a failure leaves the folders as they were. A profile's subfolder is a relative path that
//! never climbs out of the destination, whoever calls. A profile with a `compile` group is built by
//! `font.compile`'s code with the parameters the profile maps to ([`compile_params`]), into the same staged folders,
//! so it writes what that `font.compile` call writes.
//!
//! `glyph.exportArtwork` (and its Filter-menu name `filter.glyphAsImage`) writes one picture per
//! glyph: SVG, PDF or PNG. Every picture spans the glyph's advance width (or what it draws, where
//! that reaches further) and the master's descender to ascender, so pictures of one font line up.
//! A palette or full-colour layer is drawn in colour in PNG and PDF through
//! `fontcraft_render::color::artwork`, with the shadows, inner shadows and pattern images of
//! full-colour paths, which no font table holds; SVG pictures are the outline. A PDF keeps
//! outlines and one-colour fills as vector paths and draws gradients (clipped by their outlines)
//! and the image-only effects as images at `pixelsPerEm`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_color::{AssetStore, ColorLayerKind};
use fontcraft_compile::{Flavour, Options};
use fontcraft_geom::{Affine, BezPath, PathEl, Point, Rect, Shape as _};
use fontcraft_model::{Family, Font, Glyph, Layer, Shape, ShapeAttrs};
use fontcraft_render::color::artwork::{self, Drawable, Item, Leaf, Pattern, Sources};
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
/// Most pixels of a decoded pattern image.
const MAX_PATTERN_PIXELS: u64 = 1 << 22;
/// Most bytes of images in one PDF picture.
const MAX_PDF_IMAGE_BYTES: usize = 256 << 20;

/// Most entries of a profile's `containers`.
const MAX_CONTAINERS: usize = 3;
/// The keys of a profile's `params`.
const SETTING_KEYS: [&str; 7] = ["folder", "outlines", "containers", "removeOverlap", "autohint", "cbdt", "compile"];
/// The keys of a profile's `compile` group: the `font.compile` settings a profile carries besides the dialog's.
pub const COMPILE_KEYS: [&str; 11] = [
    "variable",
    "skipFeatures",
    "productionNames",
    "postFormat",
    "conversionError",
    "subroutinize",
    "keepGlyphs",
    "removeGlyphs",
    "colr",
    "svg",
    "sbix",
];
/// Most names in `keepGlyphs` or `removeGlyphs`.
pub const MAX_GLYPH_REFS: usize = 4096;
/// Longest glyph name in `keepGlyphs` or `removeGlyphs`, in bytes.
const MAX_GLYPH_NAME_BYTES: usize = 512;

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
            "{fonts?: [title or path] (default: the active document), profiles: [name], destination?: folder, bySource?: bool, installTemporarily?: bool, timestamp?: number|\"now\"} export every listed open font through every listed export profile in one run: each profile's fonts go into its output subfolder of the destination, or with bySource: true of the folder that holds each font's source file (give one or the other). Each profile builds the font's exportable instances as file.exportAll does, in the profile's outline format and containers, with its overlap removal, when its cbdt setting is on the CBDT and CBLC bitmap tables from the iColor layers, and when its autohint is on autohinting as file.exportStatic's autohint does; instances between masters are skipped and listed. A profile with compile settings is built by font.compile's own code with the parameters they map to, so it writes what that font.compile call writes (its file names and folders included). A designspace without axes is refused when it states a location, or when it has two or more sources and instances to build. Two profiles that would write the same file are refused. A failing export writes nothing. installTemporarily (installing the fonts until the app quits) is not supported yet → {profiles: [{profile, documents: [{title, destination, fonts, skipped}]}], written, warnings}",
            has_doc,
            export_with_profiles
        ).files(const { &[FileParam::write("destination")] }).writes_files(),
        cmd!(
            "file.saveExportProfile",
            "Save Export Profile",
            [],
            None,
            "{name: string, basedOn?: profile name, settings: {folder?: relative folder, outlines?: \"truetype\"|\"cff\", containers?: [\"plain\"|\"woff\"|\"woff2\"], removeOverlap?: bool, autohint?: bool, cbdt?: bool, compile?: {variable?: bool, skipFeatures?: bool, productionNames?: bool, postFormat?: 2|3, conversionError?: number, subroutinize?: bool, keepGlyphs?: [name], removeGlyphs?: [name], colr?: bool, svg?: bool, sbix?: bool}}} create or update one of the user's export profiles, kept in the application settings. compile: the font.compile settings the profile carries, merged key by key over the base's (null removes one): variable also builds the variable fonts (font.compile's variable format); skipFeatures, productionNames, postFormat (2 or 3), subroutinize as in font.compile; conversionError is outlines.conversionError (above 0, at most 1 em); keepGlyphs and removeGlyphs (at most 4096 names each) become font.compile's glyphs: the exported glyphs kept, less the removed ones; colr, svg or sbix false leave out the COLR (with CPAL), SVG or sbix table. The settings start from basedOn's, else from the profile's own when it exists, else from the defaults (truetype, plain, removeOverlap true, autohint false, cbdt false), and `settings` replaces what it names. cbdt: also write CBDT and CBLC bitmap tables from the iColor layers. folder: the output subfolder, `/`-separated, relative and never climbing out with `..`; folder names ending in a space or a dot, and reserved device names (CON, PRN, AUX, NUL, COM1 to COM9, LPT1 to LPT9, with or without an extension), are refused on every system; empty writes into the destination itself; a new profile's folder defaults to its name. Built-in profiles are read-only: their names are refused; at most 256 profiles → {profile: {name, builtIn, params}, created: bool}",
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
            "{} the export profiles: the built-in ones first, then the user's in the order they were made. problems lists stored profiles that cannot be read, which are left out → {profiles: [{name, builtIn, params: {folder, outlines, containers, removeOverlap, autohint, cbdt, compile?}}], problems: [text]}",
            always,
            list_profiles
        ),
        cmd!(
            "glyph.exportArtwork",
            "Export Artwork…",
            ["File", "Export"],
            None,
            "{glyphs?: [name] (default: every glyph), layer?: id (default: the first master's layer), format: \"svg\"|\"pdf\"|\"png\", directory?: folder, pixelsPerEm?: number (1 to 16384, default 256; PNG pictures and the image parts of PDF pictures)} write one picture per glyph of the chosen layer, named after the glyph (characters a file name cannot hold become `_`; a dot-separated part that is a reserved device name such as CON or NUL gets a leading `_`; names that differ only in case get a number). A picture replaces an existing file of the same name. A picture spans the advance width and the master's descender to ascender, widened to what it draws; components are drawn, open paths are not. When the layer is a palette or full-colour layer, PNG and PDF pictures draw the glyph's colour layers of that master in colour (text colour black), with the shadows, inner shadows and pattern images of full-colour paths; SVG pictures, and pictures of any other layer, are the outline in black. SVG and PDF are in font units (one point per unit, a PDF scaled down to at most 14400 points a side); a PDF holds outlines and one-colour fills as vector paths, gradients and the image-only effects as images. PNG has a transparent background. Glyphs without the layer are skipped and listed; a pattern image that cannot be read is left out and named in warnings. Without a directory the pictures are returned as dataBase64. A failing export writes nothing and leaves replaced files as they were → {format, files: [{glyph, name, bytes, path? | dataBase64?}], skipped: [{glyph, reason}], warnings: [text]}",
            has_doc,
            export_artwork
        ).files(const { &[FileParam::write("directory")] }).writes_files()
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "filter.glyphAsImage",
            "Glyph as Image…",
            ["Filter"],
            None,
            "{glyphs?: [name] (default: every glyph), layer?: id, format: \"svg\"|\"pdf\"|\"png\", directory?: folder, pixelsPerEm?: number} the pictures of glyph.exportArtwork under the Filter menu, with the same parameters, files and result: one picture per glyph, a palette or full-colour layer in colour with the shadows, inner shadows and pattern images of its paths in PNG and PDF; SVG pictures have none of them → {format, files, skipped, warnings}",
            has_doc,
            glyph_as_image
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
            let params =
                json!({ "folder": name, "outlines": outlines, "containers": containers, "removeOverlap": true, "autohint": false, "cbdt": false });
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
/// `..` and `.` are refused, so a profile never writes outside the destination. Names some file
/// systems cannot hold are refused on every system, so a profile works wherever it is copied:
/// names ending in a space or a dot, and device names Windows reserves (`CON`, `nul.txt`…).
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
        if segment.ends_with(' ') || segment.ends_with('.') {
            return Err(format!("the folder name \"{segment}\" ends in a space or a dot, which some systems drop"));
        }
        if fontcraft_io_ufo::names::is_reserved(segment.split('.').next().unwrap_or_default()) {
            return Err(format!("\"{segment}\" is a device name some systems reserve; choose another folder name"));
        }
    }
    Ok(segments.join("/"))
}

/// A profile's `compile` group: `settings` over `base` key by key (null removes a key), every value checked.
fn merge_compile(base: Option<&Value>, settings: Option<&Value>) -> std::result::Result<Map<String, Value>, String> {
    let mut out = match base {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    match settings {
        None => {}
        Some(Value::Object(m)) => {
            for (key, value) in m {
                if value.is_null() {
                    out.remove(key);
                } else {
                    out.insert(key.clone(), value.clone());
                }
            }
        }
        Some(_) => return Err("`compile` must be an object".into()),
    }
    if let Some(key) = out.keys().find(|k| !COMPILE_KEYS.contains(&k.as_str())) {
        return Err(format!(
            "`compile.{}` is not a profile setting; the compile settings are {}",
            key.chars().take(64).collect::<String>(),
            COMPILE_KEYS.join(", ")
        ));
    }
    for (key, value) in &out {
        let ok = match key.as_str() {
            "postFormat" => value.as_i64().is_some_and(|n| n == 2 || n == 3),
            "conversionError" => value.as_f64().is_some_and(|e| e.is_finite() && e > 0.0 && e <= 1.0),
            "keepGlyphs" | "removeGlyphs" => value.as_array().is_some_and(|names| {
                names.len() <= MAX_GLYPH_REFS && names.iter().all(|n| n.as_str().is_some_and(|n| !n.is_empty() && n.len() <= MAX_GLYPH_NAME_BYTES))
            }),
            _ => value.is_boolean(),
        };
        if !ok {
            return Err(match key.as_str() {
                "postFormat" => "`compile.postFormat` is 2 or 3".into(),
                "conversionError" => "`compile.conversionError` is a number above 0 and at most 1 (em)".into(),
                "keepGlyphs" | "removeGlyphs" => {
                    format!("`compile.{key}` is a list of at most {MAX_GLYPH_REFS} glyph names of at most {MAX_GLYPH_NAME_BYTES} bytes")
                }
                _ => format!("`compile.{key}` must be true or false"),
            });
        }
    }
    Ok(out)
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
    let mut params = json!({
        "folder": folder, "outlines": outlines, "containers": containers,
        "removeOverlap": flag("removeOverlap", true)?, "autohint": flag("autohint", false)?,
        "cbdt": flag("cbdt", false)?,
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    // A profile without compile settings has no `compile` group, as the built-in ones.
    let compile = merge_compile(base.get("compile"), settings.get("compile"))?;
    if !compile.is_empty() {
        params.insert("compile".into(), Value::Object(compile));
    }
    Ok(params)
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
    cbdt: bool,
    autohint: bool,
    /// The `compile` group; a profile with one is built by `font.compile`'s own code ([`compile_params`]).
    compile: Option<Map<String, Value>>,
}

fn plan(p: &Params, profile: &Profile) -> Result<Plan> {
    let params = Value::Object(profile.params.clone());
    let pp = Params::new("file.exportWithProfiles", &params)?;
    let flavour = match pp.str("outlines")? {
        "cff" => Flavour::Cff,
        _ => Flavour::TrueType,
    };
    let containers = compile::containers(&pp, "containers", &Dest::Memory).map_err(|e| p.bad(format!("profile `{}`: {e}", profile.name)))?;
    Ok(Plan {
        profile: profile.clone(),
        flavour,
        containers,
        remove_overlap: pp.bool_or("removeOverlap", true)?,
        cbdt: pp.bool_or("cbdt", false)?,
        autohint: pp.bool_or("autohint", false)?,
        compile: profile.params.get("compile").and_then(Value::as_object).filter(|m| !m.is_empty()).cloned(),
    })
}

/// The `font.compile` parameters that build `font` as the profile `plan` asks, into `dest`: the profile's outline
/// format, containers, overlap removal, autohinting and `CBDT`, and its `compile` group. Keep and Remove Glyphs
/// become `glyphs` (the exported glyphs kept, less the removed ones); `colr`, `svg` and `sbix` false leave those
/// tables out (`COLR` with its `CPAL`).
fn compile_params(plan: &Plan, group: &Map<String, Value>, font: &Font, timestamp: i64, dest: &Path) -> Value {
    let flag = |key: &str| group.get(key).and_then(Value::as_bool);
    let mut formats = vec![json!(if plan.flavour == Flavour::Cff { "otf" } else { "ttf" })];
    if flag("variable") == Some(true) {
        formats.push(json!("variable"));
    }
    let mut outlines = Map::new();
    outlines.insert("removeOverlaps".into(), json!(plan.remove_overlap));
    if let Some(e) = group.get("conversionError") {
        outlines.insert("conversionError".into(), e.clone());
    }
    let containers: Vec<&str> = plan.containers.iter().map(|c| c.name()).collect();
    let mut out = json!({
        "formats": formats, "containers": containers, "outlines": outlines, "autohint": plan.autohint, "cbdt": plan.cbdt,
        "timestamp": timestamp, "destination": dest.to_string_lossy(),
    });
    let Value::Object(m) = &mut out else { return out };
    for key in ["skipFeatures", "productionNames", "postFormat", "subroutinize"] {
        if let Some(v) = group.get(key) {
            m.insert(key.into(), v.clone());
        }
    }
    let names =
        |key: &str| -> Option<HashSet<&str>> { group.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()) };
    let (keep, remove) = (names("keepGlyphs"), names("removeGlyphs"));
    if keep.is_some() || remove.is_some() {
        let glyphs: Vec<&str> = font
            .glyphs
            .iter()
            .filter(|g| g.export)
            .map(|g| g.name.as_str())
            .filter(|n| keep.as_ref().is_none_or(|k| k.contains(n)) && !remove.as_ref().is_some_and(|r| r.contains(n)))
            .collect();
        m.insert("glyphs".into(), json!(glyphs));
    }
    let mut exclude: Vec<&str> = Vec::new();
    if flag("colr") == Some(false) {
        exclude.extend(["COLR", "CPAL"]);
    }
    if flag("svg") == Some(false) {
        exclude.push("SVG ");
    }
    if flag("sbix") == Some(false) {
        exclude.push("sbix");
    }
    if !exclude.is_empty() {
        m.insert("excludeTables".into(), json!(exclude));
    }
    out
}

/// The items of a list, none for anything else.
fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

/// Runs `f` with document `i` active, then makes the document active before it active again.
fn with_active<T>(s: &mut Session, i: usize, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    let before = s.active().and_then(|a| s.documents().iter().position(|d| std::ptr::eq(d, a)));
    s.set_active(i)?;
    let out = f(s);
    if let Some(before) = before {
        s.set_active(before)?;
    }
    out
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
    let mut per_plan: Vec<Vec<Value>> = vec![Vec::new(); plans.len()];
    let mut skipped_all: Vec<Value> = Vec::new();
    let mut file_count = 0usize;
    // Profiles with a `compile` group, built after the others by `font.compile`'s code: (plan, folder, documents).
    let mut through_compile: Vec<(usize, PathBuf, Vec<usize>)> = Vec::new();
    for (pi, plan) in plans.iter().enumerate() {
        let folder = plan.profile.params.get("folder").and_then(Value::as_str).unwrap_or_default();
        let hinting = fontcraft_compile::HintingOptions { autohint: plan.autohint, strip: false };
        let options = Options { timestamp, remove_overlap: plan.remove_overlap, cbdt: plan.cbdt, hinting, ..Default::default() };
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
        if plan.compile.is_some() {
            through_compile.extend(groups.into_iter().map(|(dest, members)| (pi, dest, members.into_iter().map(|(_, i)| i).collect())));
            continue;
        }
        let mut documents = Vec::new();
        for (dest_path, members) in groups {
            let req = Request {
                flavours: vec![plan.flavour],
                containers: plan.containers.clone(),
                options: options.clone(),
                dest: Dest::Folder(dest_path.clone()),
                first_only: false,
                autohinter: crate::external_autohint::trusted(s),
                naming: Default::default(),
                instance_outputs: true,
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
        if let Some(list) = per_plan.get_mut(pi) {
            *list = documents;
        }
    }
    for (pi, dest_path, members) in through_compile {
        let (Some(plan), Some(group)) = (plans.get(pi), plans.get(pi).and_then(|plan| plan.compile.as_ref())) else { continue };
        let mut sink = Sink::new(&Dest::Folder(dest_path.clone()), false, s.export_outside())?;
        for i in members {
            let doc = s.documents().get(i).ok_or(EngineError::NoDocument)?;
            let (title, font) = (doc.title.clone(), doc.font.clone());
            let params = compile_params(plan, group, &font, timestamp, &dest_path);
            let result = with_active(s, i, |s| compile::compile_into(s, &params, Some(&mut sink)))
                .map_err(|e| p.bad(format!("profile `{}`, `{title}`: {e}", plan.profile.name)))?;
            let fonts = result.get("fonts").cloned().unwrap_or(Value::Null);
            let skipped = result.get("skipped").cloned().unwrap_or(Value::Null);
            for path in
                items(&fonts).flat_map(|f| items(f.get("files").unwrap_or(&Value::Null))).filter_map(|f| f.get("path").and_then(Value::as_str))
            {
                let key = path.to_lowercase();
                if let Some((_, other)) = written.iter().find(|(w, _)| w.to_string_lossy().to_lowercase() == key) {
                    return Err(p.bad(format!("profiles `{other}` and `{}` both write {path}: give them different folders", plan.profile.name)));
                }
                written.push((PathBuf::from(path), plan.profile.name.clone()));
                file_count += 1;
            }
            warnings.extend(items(result.get("warnings").unwrap_or(&Value::Null)).filter_map(Value::as_str).map(str::to_string));
            skipped_all.extend(items(&skipped).cloned());
            if let Some(list) = per_plan.get_mut(pi) {
                list.push(json!({
                    "title": title, "destination": dest_path.to_string_lossy(),
                    "fonts": fonts, "skipped": skipped,
                }));
            }
        }
        sinks.push(sink);
    }
    let results: Vec<Value> =
        plans.iter().zip(per_plan).map(|(plan, documents)| json!({ "profile": plan.profile.name, "documents": documents })).collect();
    if file_count == 0 {
        return Err(compile::nothing_built(&Report { skipped: skipped_all, ..Default::default() }));
    }
    compile::commit_all(sinks)?;
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
    artwork_command(s, v, "glyph.exportArtwork")
}

/// `filter.glyphAsImage`: [`export_artwork`] under its menu name.
fn glyph_as_image(s: &mut Session, v: &Value) -> Result<Value> {
    artwork_command(s, v, "filter.glyphAsImage")
}

fn artwork_command(s: &mut Session, v: &Value, id: &'static str) -> Result<Value> {
    let p = Params::new(id, v)?;
    let font = s.font()?;
    let mut art = ArtSources { store: s.assets(), patterns: HashMap::new(), warnings: Vec::new() };
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
        let bytes = artwork(font, glyph, layer, format, ppem, &mut art).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
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
    Ok(json!({ "format": format.extension(), "files": files, "skipped": skipped, "warnings": art.warnings }))
}

/// File names for pictures: the glyph name with characters a file name cannot hold replaced, a
/// `_` before a part that is a reserved device name (as the UFO file-name rule does), and a number
/// added where names would differ only in case.
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
        let stem = if stem.is_empty() { "_".to_string() } else { fontcraft_io_ufo::names::escape_reserved(&stem) };
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

/// Where a picture's pattern images come from, with the files read so far and what could not be
/// read.
struct ArtSources {
    store: Arc<dyn AssetStore>,
    patterns: HashMap<String, Option<Pattern>>,
    warnings: Vec<String>,
}

impl ArtSources {
    /// The pattern image `attrs` name, decoded once per file; a file that cannot be read is
    /// reported once and left out.
    fn pattern(&mut self, attrs: &ShapeAttrs) -> Option<Pattern> {
        let file = attrs.extra.get(Family::Native, super::attributes::PATTERN_IMAGE_KEY)?.as_str()?.trim();
        if file.is_empty() {
            return None;
        }
        if let Some(known) = self.patterns.get(file) {
            return known.clone();
        }
        let read =
            self.store.read(file).map_err(|e| e.to_string()).and_then(|bytes| super::image::decode_pixels(&bytes, MAX_PATTERN_PIXELS)).and_then(
                |d| {
                    (d.image.width > 0 && d.image.height > 0 && d.units_per_pixel.is_finite() && d.units_per_pixel > 0.0)
                        .then(|| Pattern { image: Arc::new(d.image), unit: d.units_per_pixel })
                        .ok_or_else(|| "the image is empty".to_string())
                },
            );
        let pattern = match read {
            Ok(p) => Some(p),
            Err(e) => {
                self.warnings.push(format!("the pattern image `{file}` cannot be read ({e}); the paths that use it are drawn without it"));
                None
            }
        };
        self.patterns.insert(file.to_string(), pattern.clone());
        pattern
    }
}

/// The artwork of `glyph` when `layer` is a palette or full-colour layer: the glyph's vector
/// colour layers of the layer's master with their image-only effects, in black text colour.
/// `None` for any other layer.
fn colour_items(font: &Font, glyph: &Glyph, layer: &Layer, art: &mut ArtSources) -> Option<Vec<Item>> {
    let kind = fontcraft_color::layers::export_kind(layer)?;
    if !matches!(kind, ColorLayerKind::Palette(_) | ColorLayerKind::FullColor) {
        return None;
    }
    let outline = |f: &Font, shapes: &[Shape], master: &str| {
        super::stroke::shapes_outline(f, shapes, master).ok().map(|parts| {
            parts.into_iter().fold(BezPath::new(), |mut all, p| {
                all.extend(p.elements().iter().copied());
                all
            })
        })
    };
    let mut pattern = |attrs: &ShapeAttrs| art.pattern(attrs);
    let mut src = Sources { outline: &outline, pattern: &mut pattern, text: [0.0, 0.0, 0.0, 1.0], palette: 0 };
    artwork::collect(font, glyph, &layer.master, &mut src)
}

/// One picture of `layer` of `glyph`. A palette or full-colour layer is drawn in colour in PNG
/// and PDF ([`colour_items`]); everything else, and every SVG, is the layer's outline in black.
fn artwork(font: &Font, glyph: &Glyph, layer: &Layer, format: Artwork, ppem: f64, art: &mut ArtSources) -> std::result::Result<Vec<u8>, String> {
    let outline = fontcraft_render::layer_outline(font, layer).map_err(|e| e.to_string())?;
    let outline = fontcraft_render::closed_contours(&outline);
    let items = match format {
        Artwork::Svg => None,
        Artwork::Pdf | Artwork::Png => colour_items(font, glyph, layer, art),
    };
    let content = match &items {
        Some(items) => artwork::bounds(items),
        None => (!outline.elements().is_empty()).then(|| outline.bounding_box()),
    };
    let frame = frame(font, layer, content)?;
    let scale = ppem / f64::from(font.units_per_em.max(1));
    let marks: Vec<Mark> = match &items {
        Some(items) => items.iter().map(Mark::Item).collect(),
        None => vec![Mark::Outline(&outline)],
    };
    match format {
        Artwork::Svg => Ok(fontcraft_io_svg::bez_to_svg(&outline, Some(frame)).into_bytes()),
        Artwork::Pdf => pdf(frame, &marks, scale),
        Artwork::Png => png(&marks, frame, scale),
    }
}

/// The area a picture shows: the advance width and the master's descender to ascender, widened
/// to what it draws (`content`), at least one unit each way.
fn frame(font: &Font, layer: &Layer, content: Option<Rect>) -> std::result::Result<Rect, String> {
    let master = font.master(&layer.master).or_else(|_| font.default_master()).map_err(|e| e.to_string())?;
    let metrics = super::common::master_metrics(font, master);
    let (mut x0, mut x1) = (0.0f64.min(layer.width), 0.0f64.max(layer.width));
    let (mut y0, mut y1) = (metrics.descender.min(metrics.ascender), metrics.descender.max(metrics.ascender));
    if let Some(b) = content {
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

/// What a picture draws, in font units, bottom first.
enum Mark<'a> {
    /// An outline filled in black.
    Outline(&'a BezPath),
    /// A piece of colour artwork.
    Item(&'a Item),
}

/// Pixels per unit `scale` over `rect` (font units): the image size and the map from font units
/// to its pixels.
fn raster_grid(rect: Rect, scale: f64) -> std::result::Result<(u32, u32, Affine), String> {
    let (w, h) = ((rect.width() * scale).ceil(), (rect.height() * scale).ceil());
    let side = f64::from(fontcraft_render::MAX_SIDE);
    if !(w.is_finite() && h.is_finite()) || w > side || h > side {
        return Err(format!("the picture would be larger than {side} pixels on a side; choose fewer pixels per em"));
    }
    let transform = Affine::translate((-rect.x0 * scale, rect.y1 * scale)) * Affine::scale_non_uniform(scale, -scale);
    Ok(((w as u32).max(1), (h as u32).max(1), transform))
}

/// `marks` on a transparent background, `scale` pixels per unit; outlines in black.
fn png(marks: &[Mark], frame: Rect, scale: f64) -> std::result::Result<Vec<u8>, String> {
    let (width, height, transform) = raster_grid(frame, scale)?;
    let image = match marks {
        [Mark::Outline(outline)] => {
            let style = fontcraft_render::Style { ink: [0, 0, 0, 255], background: [0, 0, 0, 0], anti_alias: true };
            fontcraft_render::render_path(outline, transform, width, height, &style)
        }
        _ => {
            let items: Vec<Item> = marks
                .iter()
                .filter_map(|m| match m {
                    Mark::Item(i) => Some((*i).clone()),
                    Mark::Outline(_) => None,
                })
                .collect();
            artwork::rasterize(&items, transform, width, height)
        }
    }
    .map_err(|e| e.to_string())?;
    image.png().map_err(|e| e.to_string())
}

/// `v` rounded to three decimals, in its shortest form, with no negative zero.
fn num(v: f64) -> String {
    let r = if v.is_finite() { (v * 1000.0).round() / 1000.0 } else { 0.0 };
    if r == 0.0 { "0".to_string() } else { format!("{r}") }
}

/// The path operators of `outline`'s closed contours.
fn path_ops(content: &mut String, outline: &BezPath) {
    use std::fmt::Write as _;
    let (mut current, mut start) = (Point::ZERO, Point::ZERO);
    let pt = |q: Point| format!("{} {}", num(q.x), num(q.y));
    for el in fontcraft_render::closed_contours(outline).elements() {
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
}

/// The objects of a one-page PDF after the catalog, the page tree, the page and its content.
#[derive(Default)]
struct PdfParts {
    content: String,
    objects: Vec<Vec<u8>>,
    /// Image objects, named `/Im<index>`.
    images: Vec<usize>,
    /// Fill opacities in thousandths and their graphics state objects, named `/A<thousandths>`.
    alphas: Vec<(u32, usize)>,
    image_bytes: usize,
}

/// Objects 1 to 4 are the catalog, the page tree, the page and its content.
const PDF_FIXED_OBJECTS: usize = 4;

fn pdf_stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

impl PdfParts {
    /// Adds an object; answers its number.
    fn add(&mut self, body: Vec<u8>) -> usize {
        self.objects.push(body);
        PDF_FIXED_OBJECTS + self.objects.len()
    }

    /// The operator that sets fill opacity `a` (below 1).
    fn alpha(&mut self, a: f32) -> String {
        let key = (a.clamp(0.0, 1.0) * 1000.0).round() as u32;
        if !self.alphas.iter().any(|(k, _)| *k == key) {
            let n = self.add(format!("<< /Type /ExtGState /ca {} >>", num(f64::from(key) / 1000.0)).into_bytes());
            self.alphas.push((key, n));
        }
        format!("/A{key} gs\n")
    }

    /// Draws `image` (premultiplied) over `place` (font units), with its alpha as a soft mask.
    fn image(&mut self, image: &fontcraft_render::Image, place: Rect) -> std::result::Result<(), String> {
        use std::fmt::Write as _;
        let pixels = image.pixels.as_chunks::<4>().0;
        if pixels.iter().all(|p| p[3] == 0) {
            return Ok(());
        }
        self.image_bytes = self.image_bytes.saturating_add(pixels.len().saturating_mul(4));
        if self.image_bytes > MAX_PDF_IMAGE_BYTES {
            return Err(format!("the PDF would hold more than {} MB of images; choose fewer pixels per em", MAX_PDF_IMAGE_BYTES >> 20));
        }
        let mut rgb = Vec::with_capacity(pixels.len() * 3);
        let mut alpha = Vec::with_capacity(pixels.len());
        for p in pixels {
            let a = u32::from(p[3]);
            let straight = |c: u8| (u32::from(c) * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
            rgb.extend([straight(p[0]), straight(p[1]), straight(p[2])]);
            alpha.push(p[3]);
        }
        let (w, h) = (image.width, image.height);
        let mask = self
            .add(pdf_stream(&format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8"), &alpha));
        let n = self.add(pdf_stream(
            &format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask {mask} 0 R"),
            &rgb,
        ));
        let _ = writeln!(
            self.content,
            "q {} 0 0 {} {} {} cm /Im{} Do Q",
            num(place.width()),
            num(place.height()),
            num(place.x0),
            num(place.y0),
            self.images.len()
        );
        self.images.push(n);
        Ok(())
    }

    /// `item` rasterised at `scale` pixels per unit over its own area within `frame`.
    fn raster(&mut self, item: &Item, frame: Rect, scale: f64, leaf_only: Option<&Leaf>) -> std::result::Result<(), String> {
        let Some(rect) = item.bounds().map(|b| b.intersect(frame)).filter(|r| r.width() > 0.0 && r.height() > 0.0) else { return Ok(()) };
        let (w, h, transform) = raster_grid(rect, scale)?;
        let image = match leaf_only {
            Some(leaf) => artwork::leaf_image(leaf, transform, w, h),
            None => artwork::rasterize(std::slice::from_ref(item), transform, w, h),
        }
        .map_err(|e| e.to_string())?;
        let (pw, ph) = (f64::from(w) / scale, f64::from(h) / scale);
        self.image(&image, Rect::new(rect.x0, rect.y1 - ph, rect.x0 + pw, rect.y1))
    }

    /// Draws `mark`: outlines and one-colour fills as vector paths, gradient fills as an image
    /// clipped by their outlines, and the image-only effects as images.
    fn mark(&mut self, mark: &Mark, frame: Rect, scale: f64) -> std::result::Result<(), String> {
        match mark {
            Mark::Outline(outline) => {
                if !outline.elements().is_empty() {
                    path_ops(&mut self.content, outline);
                    self.content.push_str("f\n");
                }
                Ok(())
            }
            Mark::Item(item) => match item {
                Item::Fill(d) => self.fill(item, d, frame, scale),
                _ => self.raster(item, frame, scale, None),
            },
        }
    }

    fn fill(&mut self, item: &Item, d: &Drawable, frame: Rect, scale: f64) -> std::result::Result<(), String> {
        use std::fmt::Write as _;
        let Some((last, outer)) = d.clips.split_last() else { return Ok(()) };
        match d.leaf.solid() {
            Some([_, _, _, a]) if a <= 0.0 => {}
            Some([r, g, b, a]) => {
                self.content.push_str("q\n");
                for clip in outer {
                    path_ops(&mut self.content, clip);
                    self.content.push_str("W n\n");
                }
                let _ = writeln!(self.content, "{} {} {} rg", num(f64::from(r)), num(f64::from(g)), num(f64::from(b)));
                if a < 1.0 {
                    let state = self.alpha(a);
                    self.content.push_str(&state);
                }
                path_ops(&mut self.content, last);
                self.content.push_str("f\nQ\n");
            }
            None => {
                self.content.push_str("q\n");
                for clip in &d.clips {
                    path_ops(&mut self.content, clip);
                    self.content.push_str("W n\n");
                }
                self.raster(item, frame, scale, Some(&d.leaf))?;
                self.content.push_str("Q\n");
            }
        }
        Ok(())
    }
}

/// A one-page PDF of `marks`, one point per unit (scaled down to fit the largest page); the
/// image parts drawn at `scale` pixels per unit.
fn pdf(frame: Rect, marks: &[Mark], scale: f64) -> std::result::Result<Vec<u8>, String> {
    use std::fmt::Write as _;
    let longest = frame.width().max(frame.height());
    let s = if longest > PDF_MAX_PAGE { PDF_MAX_PAGE / longest } else { 1.0 };
    let mut parts = PdfParts::default();
    if !marks.is_empty() {
        let _ = writeln!(parts.content, "{} 0 0 {} {} {} cm", num(s), num(s), num(-frame.x0 * s), num(-frame.y0 * s));
        for mark in marks {
            parts.mark(mark, frame, scale)?;
        }
    }
    let mut resources = String::new();
    if !parts.images.is_empty() {
        resources.push_str("/XObject <<");
        for (i, n) in parts.images.iter().enumerate() {
            let _ = write!(resources, " /Im{i} {n} 0 R");
        }
        resources.push_str(" >> ");
    }
    if !parts.alphas.is_empty() {
        resources.push_str("/ExtGState <<");
        for (key, n) in &parts.alphas {
            let _ = write!(resources, " /A{key} {n} 0 R");
        }
        resources.push_str(" >> ");
    }
    let content = parts.content;
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << {resources}>> /Contents 4 0 R >>",
            num(frame.width() * s),
            num(frame.height() * s)
        )
        .into_bytes(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
    ];
    objects.extend(parts.objects);
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = writeln!(tail, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF", objects.len() + 1);
    out.extend_from_slice(tail.as_bytes());
    Ok(out)
}

// ---------- staging pictures on disk ----------

/// Pictures written under temporary names in the directory and moved into place by
/// [`Staged::commit`] as the font export does: all or nothing, replaced files put back on a
/// failure. Dropping it uncommitted removes the temporary files and the folders it made.
#[cfg(not(target_arch = "wasm32"))]
struct Staged {
    dir: PathBuf,
    stage: compile::Stage,
}

#[cfg(not(target_arch = "wasm32"))]
impl Staged {
    fn new(dir: PathBuf) -> Result<Self> {
        Ok(Self { stage: compile::Stage::new(dir.clone(), false, false), dir })
    }

    /// Stages the picture `name`, a single file name; returns its final path.
    fn put(&mut self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.stage.put(name, bytes, &mut Vec::new())?;
        Ok(self.dir.join(name))
    }

    fn commit(self) -> Result<()> {
        self.stage.commit()
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
        // Names some file systems cannot hold are refused everywhere.
        for bad in ["con", "Web/NUL", "aux.d", "fonts/Com1.old/x", "LPT9", "prn", "Web./x", "a /b", "dir."] {
            assert!(check_folder(bad).is_err(), "{bad}");
        }
        for good in ["console", "com10", "a.b/c", "nul_d", "Web Fonts"] {
            assert!(check_folder(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn artwork_file_names_are_safe_and_differ_in_more_than_case() {
        let mut names = FileNames::default();
        assert_eq!(names.next("A", "svg"), "A.svg");
        assert_eq!(names.next("a", "svg"), "a_2.svg");
        assert_eq!(names.next("../x", "svg"), ".._x.svg");
        assert_eq!(names.next(".notdef", "svg"), ".notdef.svg");
        assert_eq!(names.next("CON", "png"), "_CON.png");
        assert_eq!(names.next("nul.alt", "png"), "_nul.alt.png");
        assert_eq!(names.next("console", "png"), "console.png");
    }

    #[test]
    fn pdf_offsets_point_at_their_objects() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.quad_to((50.0, 100.0), (100.0, 0.0));
        path.close_path();
        let bytes = pdf(Rect::new(0.0, -200.0, 500.0, 800.0), &[Mark::Outline(&path)], 1.0).unwrap();
        // One character per byte, so string offsets are file offsets (the header's second line
        // holds bytes above 127).
        let text: String = bytes.iter().map(|&b| if b.is_ascii() { b as char } else { '?' }).collect();
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
