//! `font.instantiate` with a designspace or another multi-master source as input (M12.22).
//!
//! Rows: `cli.instance` (the source half). Each instance is interpolated by the routine static
//! export and Generate Instances use ([`super::super::instance_masters::generated_font`]), so a
//! UFO instance holds what the instance exports. The steps after interpolation are the editor's
//! own commands, run on the instance in a session of its own (decision D2): Remove Overlap, Round
//! Coordinates, Autohint PostScript. Compiled instances (`to: "ttf"|"otf"`) are `font.compile`
//! run on the source with those instances.
//!
//! UFO instances follow fontTools' instantiator (ufo2ft `instantiator.py`, MIT License) in what
//! they carry from the default source: the font info that is not specific to one style, the
//! instance's PostScript and style-map names from the designspace, and the OS/2 weight and width
//! classes from the `wght` and `wdth` locations when the masters state none.
//!
//! UFOs are staged in temporary folders beside their targets and swapped in once every instance
//! is built ([`super::stage_folder`], [`super::commit`]), so a failure leaves existing files as
//! they were.
// Nothing reads a source by path in the browser, so most of this is unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};

use fontcraft_interp::compat::{self, MasterPairs};
use fontcraft_interp::{Designspace, GlyphSources, UserLocation};
use fontcraft_model::plist::{Number, Value as PValue};
use fontcraft_model::{Font, Instance, SourceFormat};
use serde_json::{Map, Value, json};

use super::super::file_io::{self, OutputRequest};
use super::super::instance_masters::generated_font;
use super::Staged;
#[cfg(not(target_arch = "wasm32"))]
use super::{commit, discard, stage_folder};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Options of compiled variable fonts, with the value that leaves each without effect.
const BINARY_ONLY: [(&str, bool); 5] =
    [("optimizeGvar", true), ("overlapFlag", true), ("updateNameTable", false), ("downgradeCff2", false), ("recalcBounds", true)];

/// Font-info keys that name one style, so an instance does not take them from the default
/// source (the instantiator's list): parameters under their UFO and reference names, and the
/// properties that hold the same values.
const STYLE_PARAMS: [&str; 18] = [
    "postscriptFontName",
    "postscriptFullName",
    "postscriptUniqueID",
    "styleMapFamilyName",
    "styleMapStyleName",
    "openTypeNameCompatibleFullName",
    "compatibleFullName",
    "openTypeNamePreferredFamilyName",
    "preferredFamilyName",
    "openTypeNamePreferredSubfamilyName",
    "preferredSubfamilyName",
    "openTypeNameUniqueID",
    "openTypeNameWWSFamilyName",
    "WWSFamilyName",
    "openTypeNameWWSSubfamilyName",
    "WWSSubfamilyName",
    "woffMetadataUniqueID",
    "postscriptWeightName",
];
const STYLE_PROPERTIES: [&str; 8] = [
    "postscriptFontName",
    "postscriptFullName",
    "postscriptFullNames",
    "preferredFamilyNames",
    "preferredSubfamilyNames",
    "WWSFamilyName",
    "WWSSubfamilyName",
    "uniqueID",
];

/// usWidthClass 1 to 9 at these `wdth` user values (the OpenType specification's table).
const WIDTH_CLASSES: [(f64, f64); 9] =
    [(50.0, 1.0), (62.5, 2.0), (75.0, 3.0), (87.5, 4.0), (100.0, 5.0), (112.5, 6.0), (125.0, 7.0), (150.0, 8.0), (200.0, 9.0)];

/// What the instances are written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ufo,
    /// A compiled font: `ttf` or `otf`.
    Font(&'static str),
}

/// One instance to build.
struct Job {
    /// Its index in the prepared font's instances.
    index: usize,
    /// Made from `location` rather than one of the source's instances.
    from_location: bool,
    /// Its location in user coordinates, by axis tag.
    location: Value,
}

/// What the steps after interpolation do.
struct Steps {
    round: bool,
    autohint: bool,
    remove_overlaps: bool,
    ufo_version: u32,
}

#[cfg(target_arch = "wasm32")]
pub(super) fn instantiate(_: &Params<'_>, input: &Path, _: &OutputRequest) -> Result<Value> {
    Err(EngineError::Unsupported(format!("reading {} by path in the browser", input.display())))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn instantiate(p: &Params<'_>, input: &Path, req: &OutputRequest) -> Result<Value> {
    for (key, neutral) in BINARY_ONLY {
        if p.bool_or(key, neutral)? != neutral {
            return Err(p.bad(format!("`{key}` applies to compiled variable fonts, and {} is a source", input.display())));
        }
    }
    if p.get("face").is_some() {
        return Err(p.bad("`face` chooses a font of a collection, and the input is a source"));
    }
    if p.bool_or("layoutOnly", false)? || p.get("masterFinder").is_some() {
        return Err(EngineError::Unsupported("interpolating only the layout tables of compiled masters (`layoutOnly`)".into()));
    }
    if req.in_place {
        return Err(p.bad("an instance never replaces its source: give `output` or `outputDir`"));
    }
    let kind = kind(p, req)?;
    let instance_dir = p.opt_str("instanceDir")?.map(PathBuf::from);
    if instance_dir.is_some() && (kind != Kind::Ufo || req.output.is_some() || req.output_dir.is_some()) {
        return Err(p.bad("`instanceDir` is the folder for UFO instances: give it alone, without `output`, `outputDir` or a compiled `to`"));
    }
    let ufo_version = match p.opt_int("ufoVersion", 2, 3)? {
        Some(2) => 2,
        _ => 3,
    };
    let _ = p.bool_or("normalize", true)?;
    let strict = p.bool_or("strict", false)?;
    let clamp = p.bool_or("clamp", true)?;
    let autohint = p.get("autohint").map(|_| p.bool_or("autohint", true)).transpose()?;
    let remove_overlaps = p.bool_or("removeOverlaps", false)?;
    let _ = p.bool_or("ignoreOverlapErrors", false)?;

    let (mut font, mut warnings) = crate::io::open(input, None)?;
    if font.source.format == SourceFormat::Binary {
        return Err(EngineError::Unsupported(format!("instancing {}: it opens as a compiled font, not as a source", input.display())));
    }
    let designspace = Designspace::new(&font).map_err(|e| p.bad(e.to_string()))?;
    let jobs = jobs(p, &mut font, &designspace, clamp)?;
    if jobs.len() > 1 && req.output.is_some() {
        return Err(p.bad("`output` names one file, and this run builds several instances: give `outputDir`"));
    }
    let skipped = if strict { Vec::new() } else { set_aside_incompatible(p, &mut font, &designspace)? };
    for name in &skipped {
        warnings.push(format!("glyph `{name}` cannot be interpolated: its layers are not compatible, so the instances leave it out"));
    }
    let results = match kind {
        Kind::Ufo => {
            let steps = Steps { round: p.bool_or("round", true)?, autohint: autohint.unwrap_or(true), remove_overlaps, ufo_version };
            write_ufos(p, &font, input, req, instance_dir.as_deref(), &jobs, &steps, &skipped, &mut warnings)?
        }
        Kind::Font(ext) => {
            if !p.bool_or("round", true)? {
                return Err(p.bad("a compiled instance always has rounded coordinates"));
            }
            compile_fonts(p, font, input, req, &jobs, ext, autohint, remove_overlaps, &mut warnings)?
        }
    };
    Ok(json!({ "results": results, "warnings": warnings }))
}

/// The output kind: `to`, else the extension of `output`, else UFO.
fn kind(p: &Params<'_>, req: &OutputRequest) -> Result<Kind> {
    let from_output = req.output.as_ref().and_then(|o| o.extension()).map(|e| e.to_string_lossy().to_ascii_lowercase());
    let to = match p.opt_str("to")? {
        Some(t) if t != "same" => Some(t.to_string()),
        _ => from_output,
    };
    match to.as_deref() {
        None | Some("ufo") => Ok(Kind::Ufo),
        Some("ttf") => Ok(Kind::Font("ttf")),
        Some("otf") => Ok(Kind::Font("otf")),
        Some(other) => Err(p.bad(format!("an instance of a source is written as \"ufo\", \"ttf\" or \"otf\", not \"{other}\""))),
    }
}

/// The instances the parameters ask for. `location` (or `static` alone, for the default
/// location) adds an instance to `font`; otherwise `named`, `all` or `indices` choose among the
/// source's static instances, and nothing chooses every exporting one.
fn jobs(p: &Params<'_>, font: &mut Font, ds: &Designspace, clamp: bool) -> Result<Vec<Job>> {
    let pattern = p.opt_str("named")?;
    let all = p.bool_or("all", false)?;
    let indices = p.opt_array("indices")?;
    let chosen = usize::from(pattern.is_some()) + usize::from(all) + usize::from(indices.is_some());
    if chosen > 1 {
        return Err(p.bad("give one of `named`, `all` and `indices`"));
    }
    if p.get("location").is_some() || p.bool_or("static", false)? {
        if chosen > 0 {
            return Err(p.bad("`location` cannot be combined with `named`, `all` or `indices`: an instance of the source has its own location"));
        }
        return location_job(p, font, ds, clamp).map(|job| vec![job]);
    }
    let is_static = |inst: &Instance| inst.kind.is_none();
    let picked: Vec<usize> = if let Some(source) = pattern {
        let re = fontcraft_glyphquery::Pattern::new(&format!(".*(?:{source}).*"), false).map_err(|e| p.bad(e.to_string()))?;
        (0..font.instances.len())
            .filter(|&i| {
                font.instances.get(i).is_some_and(|inst| {
                    let names = fontcraft_io_ufo::designspace::instance_names(font, inst);
                    is_static(inst)
                        && (re.is_match(&inst.name) || names.postscript.as_deref().is_some_and(|ps| re.is_match(ps)) || re.is_match(&names.filename))
                })
            })
            .collect()
    } else if let Some(indices) = indices {
        let mut out = Vec::with_capacity(indices.len());
        for v in indices {
            let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad("`indices` holds zero-based whole numbers"))?;
            let inst =
                font.instances.get(i).ok_or_else(|| p.bad(format!("the source has {} instances; there is no index {i}", font.instances.len())))?;
            if !is_static(inst) {
                return Err(p.bad(format!("instance {i} (`{}`) is a variable font setting, not a static instance", inst.name)));
            }
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    } else {
        (0..font.instances.len()).filter(|&i| font.instances.get(i).is_some_and(|inst| is_static(inst) && (all || inst.exports))).collect()
    };
    if picked.is_empty() {
        return Err(match chosen {
            0 => p.bad("the source has no exporting static instance: give a location, or `all` for the inactive ones"),
            _ => p.bad("no instance of the source matches"),
        });
    }
    if picked.len() > fontcraft_tables::limits::MAX_NAMED_INSTANCES {
        return Err(p.bad(format!("one run builds at most {} instances", fontcraft_tables::limits::MAX_NAMED_INSTANCES)));
    }
    picked
        .into_iter()
        .map(|index| {
            let location = ds.instance_location(font, index).and_then(|l| ds.to_external(&l)).map_err(|e| p.bad(format!("instance {index}: {e}")))?;
            Ok(Job { index, from_location: false, location: user_json(ds, &location) })
        })
        .collect()
}

/// An instance at `location` (user coordinates by axis tag; an axis not named, or "drop", at its
/// default), added to `font`.
fn location_job(p: &Params<'_>, font: &mut Font, ds: &Designspace, clamp: bool) -> Result<Job> {
    let given = p.opt_object("location")?.cloned().unwrap_or_default();
    if given.len() > fontcraft_tables::limits::MAX_AXES {
        return Err(p.bad(format!("`location` names more than {} axes", fontcraft_tables::limits::MAX_AXES)));
    }
    if let Some(unknown) = given.keys().find(|k| ds.axis(k).is_none()) {
        let tags: Vec<&str> = ds.axes().iter().map(|a| a.tag.as_str()).collect();
        return Err(p.bad(format!("the source has no axis `{unknown}`; its axes are {}", tags.join(", "))));
    }
    let mut user = Vec::with_capacity(ds.axes().len());
    for a in ds.axes() {
        let range = &a.external;
        let v = match given.get(&a.tag) {
            None => range.default,
            Some(Value::String(s)) if s == "drop" => range.default,
            Some(Value::Number(n)) => {
                n.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad(format!("the value of `{}` must be a finite number", a.tag)))?
            }
            Some(Value::Object(_)) => {
                return Err(p.bad(format!("an instance of a source is static: give `{}` a value, not a range", a.tag)));
            }
            Some(_) => return Err(p.bad(format!("the value of `{}` must be a number or \"drop\"", a.tag))),
        };
        let v = if clamp && range.minimum <= range.maximum { v.clamp(range.minimum, range.maximum) } else { v };
        user.push(v);
    }
    let user = UserLocation(user);
    let internal = ds.to_internal(&user).map_err(|e| p.bad(e.to_string()))?;
    let default_master = font.default_master()?;
    let mut location: Vec<f64> = (0..font.axes.len()).map(|i| default_master.location.get(i).copied().unwrap_or(0.0)).collect();
    for (a, v) in ds.axes().iter().zip(&internal.0) {
        if let Some(slot) = location.get_mut(a.font_axis) {
            *slot = *v;
        }
    }
    let base: String = ds.axes().iter().zip(&user.0).map(|(a, v)| format!("{}{}", a.tag, number(*v))).collect::<Vec<_>>().join(" ");
    let base = if base.is_empty() { "Instance".to_string() } else { base };
    let taken = |name: &str| font.instances.iter().any(|i| i.name == name);
    let name = (1..=font.instances.len().saturating_add(1))
        .map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") })
        .find(|n| !taken(n))
        .unwrap_or(base);
    let instances = std::sync::Arc::make_mut(&mut font.instances);
    instances.push(Instance { name, location, ..Instance::default() });
    let index = instances.len().saturating_sub(1);
    Ok(Job { index, from_location: true, location: user_json(ds, &user) })
}

/// A number as a style name writes it: whole numbers without a fraction.
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{v:.0}") } else { format!("{v}") }
}

fn user_json(ds: &Designspace, location: &UserLocation) -> Value {
    Value::Object(ds.axes().iter().zip(&location.0).map(|(a, v)| (a.tag.clone(), json!(v))).collect::<Map<String, Value>>())
}

/// Marks every exporting glyph whose master layers are not compatible as not exporting, so the
/// interpolation keeps going without it; answers their names.
fn set_aside_incompatible(p: &Params<'_>, font: &mut Font, ds: &Designspace) -> Result<Vec<String>> {
    let scope = MasterPairs { masters: ds.masters().iter().map(|m| m.id.clone()).collect(), indicators: true };
    let mut names = Vec::new();
    for g in font.glyphs.as_arcs().iter().filter(|g| g.export) {
        let sources = GlyphSources::of(font, ds, &g.name).map_err(|e| p.bad(e.to_string()))?;
        if sources.main.len() > 1 && !compat::check_glyph(font, &sources, &scope).ok {
            names.push(g.name.clone());
        }
    }
    for name in &names {
        if let Some(mut g) = font.glyphs.get_mut(name) {
            g.export = false;
        }
    }
    Ok(names)
}

// ---------- UFO instances ----------

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn write_ufos(
    p: &Params<'_>,
    font: &Font,
    input: &Path,
    req: &OutputRequest,
    instance_dir: Option<&Path>,
    jobs: &[Job],
    steps: &Steps,
    skipped: &[String],
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let mut staged: Vec<Staged> = Vec::with_capacity(jobs.len());
    let mut created: Vec<Vec<PathBuf>> = Vec::new();
    let built = build_ufos(p, font, input, req, instance_dir, jobs, steps, skipped, warnings, &mut staged, &mut created)
        .and_then(|results| commit(&staged).map(|()| results));
    if built.is_err() {
        discard(&staged);
        for dirs in created.iter().rev() {
            file_io::remove_created(dirs);
        }
    }
    built
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn build_ufos(
    p: &Params<'_>,
    font: &Font,
    input: &Path,
    req: &OutputRequest,
    instance_dir: Option<&Path>,
    jobs: &[Job],
    steps: &Steps,
    skipped: &[String],
    warnings: &mut Vec<String>,
    staged: &mut Vec<Staged>,
    created: &mut Vec<Vec<PathBuf>>,
) -> Result<Vec<Value>> {
    let mut outs: Vec<PathBuf> = Vec::with_capacity(jobs.len());
    let mut results = Vec::with_capacity(jobs.len());
    for job in jobs {
        let inst = font.instances.get(job.index).ok_or_else(|| p.bad(format!("there is no instance #{}", job.index)))?;
        let names = fontcraft_io_ufo::designspace::instance_names(font, inst);
        let label = inst.name.clone();
        let (made, notes) = generated_font(p, font, job.index).map_err(|e| match e {
            EngineError::BadParams { message, .. } => p.bad(format!("instance `{label}`: {message}")),
            other => other,
        })?;
        warnings.extend(notes.into_iter().map(|n| format!("instance `{label}`: {n}")));
        let made = finish(p, made, inst, &names, &job.location, steps, skipped, &label, warnings)?;
        let out = ufo_path(input, req, instance_dir, job, &names.filename)?;
        if out == input || outs.contains(&out) {
            return Err(p.bad(format!("two instances, or an instance and the source, would both be written to {}", out.display())));
        }
        file_io::check_write(&out)?;
        if req.no_clobber && out.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", out.display())));
        }
        let master = made.masters.first().map(|m| m.id.clone()).ok_or_else(|| p.bad(format!("instance `{label}` has no master")))?;
        let files = fontcraft_io_ufo::ufo::write_one(&made, &master)?;
        created.push(make_parent(&out)?);
        let is_ufo = |dir: &Path| dir.join("metainfo.plist").is_file() || std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none());
        staged.push(stage_folder(&out, &files, staged.len(), is_ufo)?);
        let mut entry = json!({ "output": out.display().to_string(), "location": job.location.clone(), "static": true });
        if let Some(obj) = entry.as_object_mut() {
            if !job.from_location {
                obj.insert("name".into(), json!(label));
            }
            if let Some(ps) = &names.postscript {
                obj.insert("postscriptName".into(), json!(ps));
            }
        }
        outs.push(out);
        results.push(entry);
    }
    Ok(results)
}

/// Where a UFO instance goes: `output`; else a location's `<stem>.instance.ufo` beside the
/// source or in `instanceDir` / `outputDir`; else the instance's file name from the source,
/// relative to the source's folder, or its last part in `instanceDir` / `outputDir`.
fn ufo_path(input: &Path, req: &OutputRequest, instance_dir: Option<&Path>, job: &Job, filename: &str) -> Result<PathBuf> {
    if let Some(out) = &req.output {
        return Ok(out.clone());
    }
    let dir = instance_dir.map(Path::to_path_buf).or_else(|| req.output_dir.clone());
    if job.from_location {
        let req = OutputRequest { output_dir: dir, ..req.clone() };
        return file_io::output_path(input, &req, ".instance", Some("ufo"));
    }
    let bad = || EngineError::Io(format!("the instance file name `{filename}` is not a relative path"));
    if filename.is_empty() || filename.contains('\\') || Path::new(filename).is_absolute() {
        return Err(bad());
    }
    match dir {
        Some(dir) => {
            let last = filename.rsplit('/').find(|s| !s.is_empty()).ok_or_else(bad)?;
            Ok(dir.join(last))
        }
        None => {
            let base = input.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
            Ok(base.join(filename))
        }
    }
}

/// Creates the folders above `out` that are missing; answers them, deepest first.
#[cfg(not(target_arch = "wasm32"))]
fn make_parent(out: &Path) -> Result<Vec<PathBuf>> {
    let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) else { return Ok(Vec::new()) };
    let missing: Vec<PathBuf> = dir.ancestors().take_while(|a| !a.as_os_str().is_empty() && !a.exists()).map(Path::to_path_buf).collect();
    if missing.is_empty() {
        return Ok(missing);
    }
    file_io::check_write(dir)?;
    if let Err(e) = std::fs::create_dir_all(dir) {
        file_io::remove_created(&missing);
        return Err(EngineError::Io(format!("cannot create {}: {e}", dir.display())));
    }
    Ok(missing)
}

/// The steps after interpolation, and the names and classes of the instance.
#[allow(clippy::too_many_arguments)]
fn finish(
    p: &Params<'_>,
    made: Font,
    inst: &Instance,
    names: &fontcraft_io_ufo::designspace::InstanceNames,
    location: &Value,
    steps: &Steps,
    skipped: &[String],
    label: &str,
    warnings: &mut Vec<String>,
) -> Result<Font> {
    let mut made = made;
    for name in skipped {
        made.glyphs.remove(name);
    }
    let mut made = run_steps(made, steps, label, warnings)?;
    made.source.ufo_version = Some(steps.ufo_version);
    let info = std::sync::Arc::make_mut(&mut made.info);
    let has = |params: &fontcraft_model::Params, keys: &[&str]| keys.iter().any(|k| params.get(k).is_some());
    let font_weight = has(&info.params, &["weightClass", "openTypeOS2WeightClass"]);
    let font_width = has(&info.params, &["widthClass", "openTypeOS2WidthClass"]);
    for key in STYLE_PARAMS {
        info.params.remove(key);
    }
    info.properties.0.retain(|prop| !STYLE_PROPERTIES.contains(&prop.key.as_str()));
    let masters = std::sync::Arc::make_mut(&mut made.masters);
    let master = masters.first_mut().ok_or_else(|| p.bad(format!("instance `{label}` has no master")))?;
    for key in STYLE_PARAMS {
        master.params.remove(key);
    }
    master.properties.0.retain(|prop| !STYLE_PROPERTIES.contains(&prop.key.as_str()));
    let text = |s: &str| PValue::String(s.into());
    if let Some(ps) = &names.postscript {
        master.params.set("postscriptFontName", text(ps));
    }
    master.params.set("styleMapFamilyName", text(&names.style_map_family));
    master.params.set("styleMapStyleName", text(names.style_map_style));
    let axis = |tag: &str| location.get(tag).and_then(Value::as_f64);
    let weight = inst.params.get("weightClass").cloned().or_else(|| {
        let known = font_weight || has(&master.params, &["weightClass", "openTypeOS2WeightClass"]);
        let value = axis("wght").map(|w| fontcraft_compile::ot_round(w.clamp(1.0, 1000.0)));
        let value = value.or_else(|| inst.weight_class.is_some().then(|| fontcraft_model::axes::instance_class_value(inst, "wght")));
        (!known).then_some(value).flatten().map(class_value)
    });
    let width = inst.params.get("widthClass").cloned().or_else(|| {
        let known = font_width || has(&master.params, &["widthClass", "openTypeOS2WidthClass"]);
        let value = axis("wdth")
            .map(width_class)
            .or_else(|| inst.width_class.is_some().then(|| width_class(fontcraft_model::axes::instance_class_value(inst, "wdth"))));
        (!known).then_some(value).flatten().map(class_value)
    });
    if let Some(w) = weight {
        master.params.set("weightClass", w);
    }
    if let Some(w) = width {
        master.params.set("widthClass", w);
    }
    Ok(made)
}

/// A class as an integer parameter value.
fn class_value(v: f64) -> PValue {
    // Classes are 1 to 1000 here.
    #[allow(clippy::cast_possible_truncation)]
    PValue::Number(Number::int(v as i64))
}

/// usWidthClass at a `wdth` user value: linear between the specification's values, rounded,
/// within 1 to 9.
fn width_class(wdth: f64) -> f64 {
    let v = wdth.clamp(50.0, 200.0);
    let mut class = 9.0;
    for pair in WIDTH_CLASSES.windows(2) {
        if let [(x0, c0), (x1, c1)] = pair
            && v <= *x1
        {
            class = c0 + (v - x0) / (x1 - x0) * (c1 - c0);
            break;
        }
    }
    fontcraft_compile::ot_round(class).clamp(1.0, 9.0)
}

/// Remove Overlap, Round Coordinates (and the widths and kerning, as the instantiator rounds
/// them) and Autohint PostScript, run on `font` as a document of a session of its own.
fn run_steps(font: Font, steps: &Steps, label: &str, warnings: &mut Vec<String>) -> Result<Font> {
    let mut font = font;
    if steps.round {
        for i in 0..font.glyphs.len() {
            if let Some(mut g) = font.glyphs.get_mut_at(i) {
                for l in g.layers.iter_mut() {
                    l.width = fontcraft_compile::ot_round(l.width);
                }
            }
        }
        let kerning = std::sync::Arc::make_mut(&mut font.kerning);
        for table in [&mut kerning.ltr, &mut kerning.rtl, &mut kerning.vertical] {
            for pairs in table.0.iter_mut().flat_map(|m| m.firsts.iter_mut()) {
                for (_, v) in pairs.seconds.iter_mut() {
                    *v = fontcraft_compile::ot_round(*v);
                }
            }
        }
    }
    if !(steps.round || steps.autohint || steps.remove_overlaps) {
        return Ok(font);
    }
    let glyphs: Vec<String> = font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
    let mut s = Session::new();
    s.push_document(font, None);
    let mut report = |what: &str, r: &Value| {
        for skip in r.get("skipped").and_then(Value::as_array).into_iter().flatten() {
            let glyph = skip.get("glyph").and_then(Value::as_str).unwrap_or("?");
            let reason = skip.get("reason").and_then(Value::as_str).unwrap_or("skipped");
            warnings.push(format!("instance `{label}`: {what} left glyph `{glyph}` as it was: {reason}"));
        }
    };
    if !glyphs.is_empty() {
        if steps.remove_overlaps {
            let r = s.execute("path.removeOverlap", &json!({ "glyphs": glyphs }))?;
            report("removing overlaps", &r);
        }
        if steps.round {
            let r = s.execute("path.roundCoordinates", &json!({ "glyphs": glyphs }))?;
            report("rounding", &r);
        }
    }
    if steps.autohint {
        let r = s.execute("autohint.ps.run", &json!({}))?;
        report("autohinting", &r);
    }
    Ok(Font::clone(&s.doc()?.font))
}

// ---------- compiled instances ----------

/// `font.compile` on the source with the chosen instances. Overlaps are removed only when asked
/// (`compile`'s own default is to remove them).
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn compile_fonts(
    p: &Params<'_>,
    font: Font,
    input: &Path,
    req: &OutputRequest,
    jobs: &[Job],
    ext: &'static str,
    autohint: Option<bool>,
    remove_overlaps: bool,
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let mut font = font;
    let labels: Vec<String> = jobs
        .iter()
        .map(|j| font.instances.get(j.index).map(|i| if i.name.is_empty() { format!("#{}", j.index) } else { i.name.clone() }).unwrap_or_default())
        .collect();
    if let Some(on) = autohint {
        let instances = std::sync::Arc::make_mut(&mut font.instances);
        for job in jobs {
            if let Some(inst) = instances.get_mut(job.index) {
                inst.params.set("Autohint", PValue::Number(Number::int(i64::from(on))));
            }
        }
    }
    let single = jobs.len() == 1 && (req.output.is_some() || jobs.iter().all(|j| j.from_location));
    let destination = if single {
        file_io::output_path(input, req, ".instance", Some(ext))?
    } else {
        req.output_dir
            .clone()
            .or_else(|| input.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    if destination == input {
        return Err(p.bad(format!("the output {} would replace the source", destination.display())));
    }
    let mut params = json!({ "formats": [ext], "instances": labels, "destination": destination.display().to_string() });
    if let Some(obj) = params.as_object_mut() {
        obj.insert("outlines".into(), json!({ "removeOverlaps": remove_overlaps }));
    }
    let mut s = Session::new();
    s.push_document(font, Some(input.to_path_buf()));
    if req.no_clobber {
        let plan = s.execute("font.compilePlan", &params)?;
        let paths = plan
            .get("outputs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|o| o.get("files").and_then(Value::as_array).into_iter().flatten());
        if let Some(existing) = paths.filter_map(|f| f.get("path").and_then(Value::as_str)).find(|f| Path::new(f).exists()) {
            return Err(EngineError::Io(format!("{existing} exists and noClobber is set")));
        }
    }
    match p.get("timestamp") {
        Some(Value::String(t)) if t == "keep" => {
            return Err(p.bad("an instance compiled from a source has no earlier time to keep: give a time or \"now\""));
        }
        Some(t) => {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("timestamp".into(), t.clone());
            }
        }
        None => {}
    }
    let r = s.execute("font.compile", &params)?;
    warnings.extend(r.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
    for skip in r.get("skipped").and_then(Value::as_array).into_iter().flatten() {
        let name = skip.get("instance").and_then(Value::as_str).unwrap_or("?");
        let reason = skip.get("reason").and_then(Value::as_str).unwrap_or("skipped");
        warnings.push(format!("instance `{name}` was not built: {reason}"));
    }
    let mut results = Vec::new();
    for f in r.get("fonts").and_then(Value::as_array).into_iter().flatten() {
        let label = f.get("instance").and_then(Value::as_str).unwrap_or_default();
        let Some((job, _)) = jobs.iter().zip(&labels).find(|(_, l)| l.as_str() == label) else { continue };
        let entry_warnings = f.get("warnings").cloned().unwrap_or_else(|| json!([]));
        for file in f.get("files").and_then(Value::as_array).into_iter().flatten() {
            let mut entry = json!({ "output": file.get("path").cloned().unwrap_or(Value::Null), "location": job.location.clone(), "static": true, "warnings": entry_warnings.clone() });
            if let (false, Some(obj)) = (job.from_location, entry.as_object_mut()) {
                obj.insert("name".into(), json!(label));
            }
            results.push(entry);
        }
    }
    Ok(results)
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    const PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n";

    fn rect(x0: i32, x1: i32) -> String {
        format!(
            "<contour><point x=\"{x0}\" y=\"0\" type=\"line\"/><point x=\"{x1}\" y=\"0\" type=\"line\"/><point x=\"{x1}\" y=\"700\" type=\"line\"/><point x=\"{x0}\" y=\"700\" type=\"line\"/></contour>"
        )
    }

    fn glif(name: &str, unicode: &str, width: i32, contours: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<glyph name=\"{name}\" format=\"2\"><advance width=\"{width}\"/><unicode hex=\"{unicode}\"/><outline>{contours}</outline></glyph>\n"
        )
    }

    /// A UFO of style `style`: `A` a rectangle from `a.0` to `a.1` with width `a.2`, and `B`
    /// with the contours given.
    fn ufo(dir: &Path, style: &str, a: (i32, i32, i32), b: &str) {
        let glyphs = dir.join("glyphs");
        std::fs::create_dir_all(&glyphs).unwrap();
        let write = |rel: &str, text: String| std::fs::write(dir.join(rel), text).unwrap();
        write(
            "metainfo.plist",
            format!("{PLIST}<dict><key>creator</key><string>test</string><key>formatVersion</key><integer>3</integer></dict></plist>\n"),
        );
        write(
            "fontinfo.plist",
            format!(
                "{PLIST}<dict><key>familyName</key><string>Test</string><key>styleName</key><string>{style}</string><key>unitsPerEm</key><integer>1000</integer><key>ascender</key><integer>800</integer><key>descender</key><integer>-200</integer><key>xHeight</key><integer>500</integer><key>capHeight</key><integer>700</integer><key>postscriptFontName</key><string>Test-{style}</string></dict></plist>\n"
            ),
        );
        write(
            "layercontents.plist",
            format!("{PLIST}<array><array><string>public.default</string><string>glyphs</string></array></array></plist>\n"),
        );
        write(
            "glyphs/contents.plist",
            format!("{PLIST}<dict><key>A</key><string>A_.glif</string><key>B</key><string>B_.glif</string></dict></plist>\n"),
        );
        write("glyphs/A_.glif", glif("A", "0041", a.2, &rect(a.0, a.1)));
        write("glyphs/B_.glif", glif("B", "0042", 500, b));
    }

    /// Light (wght 300) and Bold (700) masters; `A` is compatible, `B` has one contour in Light
    /// and two in Bold. One instance, Regular at 500, written to `instances/Test-Regular.ufo`.
    fn source(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("fontcraft-instance-source-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ufo(&dir.join("Light.ufo"), "Light", (100, 400, 500), &rect(100, 400));
        ufo(&dir.join("Bold.ufo"), "Bold", (50, 550, 600), &format!("{}{}", rect(50, 200), rect(300, 450)));
        let ds = dir.join("Test.designspace");
        std::fs::write(
            &ds,
            r#"<?xml version='1.0' encoding='UTF-8'?>
<designspace format="5.0">
  <axes><axis tag="wght" name="Weight" minimum="300" default="300" maximum="700"/></axes>
  <sources>
    <source filename="Light.ufo" name="Light"><location><dimension name="Weight" xvalue="300"/></location></source>
    <source filename="Bold.ufo" name="Bold"><location><dimension name="Weight" xvalue="700"/></location></source>
  </sources>
  <instances>
    <instance familyname="Test" stylename="Regular" filename="instances/Test-Regular.ufo" postscriptfontname="Test-Regular"><location><dimension name="Weight" xvalue="500"/></location></instance>
  </instances>
</designspace>
"#,
        )
        .unwrap();
        (dir, ds)
    }

    /// The width and the x of every node of glyph `A` in the UFO at `path`.
    fn glyph_a(path: &Path) -> (f64, Vec<f64>) {
        let (font, _) = crate::io::open(path, None).unwrap();
        let layer = font.glyph("A").unwrap().layers.first().unwrap().clone();
        let xs = layer
            .shapes
            .iter()
            .filter_map(|s| match s {
                fontcraft_model::Shape::Path(p) => Some(p.nodes.iter().map(|n| n.x).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect();
        (layer.width, xs)
    }

    #[test]
    fn instantiate_source_writes_interpolated_ufo_instances() {
        let (dir, ds) = source("ufo");
        let mut s = Session::new();
        let r = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let out = dir.join("instances").join("Test-Regular.ufo");
        assert_eq!(r["results"][0]["output"], json!(out.display().to_string()), "the file name the designspace records");
        assert_eq!(r["results"][0]["name"], json!("Regular"));
        assert_eq!(r["results"][0]["location"]["wght"], json!(500.0));
        let (width, xs) = glyph_a(&out);
        assert_eq!(width, 550.0, "halfway between 500 and 600");
        assert!(xs.contains(&75.0) && xs.contains(&475.0), "{xs:?}");
        let (font, _) = crate::io::open(&out, None).unwrap();
        assert!(font.glyph("B").is_none(), "the incompatible glyph is left out");
        assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("`B`")), "{r}");
        let info = std::fs::read_to_string(out.join("fontinfo.plist")).unwrap();
        assert!(info.contains("<string>Test-Regular</string>"), "the instance's PostScript name, not the default source's: {info}");
        assert!(!info.contains("Test-Light"), "{info}");
        assert!(info.contains("<key>openTypeOS2WeightClass</key>") && info.contains("<integer>500</integer>"), "the weight class from wght: {info}");

        // A location, between the masters and clamped beyond them.
        let r = s.execute("font.instantiate", &json!({ "input": ds, "location": { "wght": 380 }, "autohint": false })).unwrap();
        let at = dir.join("Test.instance.ufo");
        assert_eq!(r["results"][0]["output"], json!(at.display().to_string()));
        let (width, mut xs) = glyph_a(&at);
        xs.sort_by(f64::total_cmp);
        assert_eq!((width, xs), (520.0, vec![90.0, 90.0, 430.0, 430.0]));
        s.execute("font.instantiate", &json!({ "input": ds, "location": { "wght": 900 }, "autohint": false })).unwrap();
        assert_eq!(glyph_a(&at).0, 600.0, "clamped to the axis maximum, replacing the earlier instance");
        s.execute("font.instantiate", &json!({ "input": ds, "location": { "wght": 900 }, "clamp": false, "round": false, "autohint": false }))
            .unwrap();
        assert_eq!(glyph_a(&at).0, 650.0, "extrapolated");

        assert!(s.execute("font.instantiate", &json!({ "input": ds, "location": { "wdth": 75 } })).is_err(), "no such axis");
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "location": { "wght": { "min": 300 } } })).is_err(), "a range");
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "named": "Nothing" })).is_err());
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "updateNameTable": true })).is_err(), "an option of compiled fonts");
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "inPlace": true })).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instantiate_source_strict_and_failures_write_nothing() {
        let (dir, ds) = source("strict");
        let mut s = Session::new();
        let err = s.execute("font.instantiate", &json!({ "input": ds, "strict": true, "autohint": false })).unwrap_err();
        assert!(err.to_string().contains('B'), "{err}");
        assert!(!dir.join("instances").exists(), "a refused run creates no folder");

        // The target is a folder that is not a UFO: refused, and left as it was.
        let target = dir.join("instances").join("Test-Regular.ufo");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.txt"), b"mine").unwrap();
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).is_err());
        assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"mine");
        let left: Vec<String> =
            std::fs::read_dir(dir.join("instances")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["Test-Regular.ufo"], "no temporary folder is left");

        // noClobber refuses an existing UFO.
        std::fs::remove_dir_all(&target).unwrap();
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "noClobber": true })).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instantiate_source_compiles_and_autohints() {
        let (dir, ds) = source("compile");
        let mut s = Session::new();
        let r =
            s.execute("font.instantiate", &json!({ "input": ds, "all": true, "to": "ttf", "outputDir": dir.join("ttf"), "timestamp": 0 })).unwrap();
        let out = r["results"][0]["output"].as_str().unwrap().to_string();
        assert!(out.ends_with(".ttf"), "{r}");
        let font = fontcraft_tables::read(&std::fs::read(&out).unwrap(), 0).unwrap();
        assert!(font.get(*b"fvar").is_none() && font.get(*b"glyf").is_some());
        assert_eq!(font.get(*b"OS/2").unwrap()[4..6], 500u16.to_be_bytes(), "usWeightClass of the instance");

        // Autohinting (the default for UFO instances) stores PostScript hints on the glyphs.
        let r = s.execute("font.instantiate", &json!({ "input": ds, "outputDir": dir.join("hinted") })).unwrap();
        let glyphs = Path::new(r["results"][0]["output"].as_str().unwrap()).join("glyphs");
        let hinted = std::fs::read_dir(&glyphs)
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .any(|text| text.contains("<glyph name=\"A\"") && text.contains("com.schriftgestaltung.hints"));
        assert!(hinted, "glyph A carries PostScript hints");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
