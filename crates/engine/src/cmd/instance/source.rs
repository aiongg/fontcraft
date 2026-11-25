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
//! classes from the `wght` and `wdth` locations by the rule static export of a UFO-family source
//! uses ([`fontcraft_model::strip_instance_info`], [`fontcraft_model::location_class`]), and
//! the instance's custom parameters and properties as glyphsLib applies them to a UFO instance
//! ([`custom_params`]). Between masters, the PostScript weight name follows the interpolated
//! weight class and PANOSE keeps only the digits every source shares ([`shared_panose`]); every
//! instance takes glyphsLib's embedding type and underline where no source states them
//! ([`glyphs_defaults`]). A designspace without axes whose sources or instances state a location,
//! or that has two or more sources, is refused, as fontTools refuses it ([`without_axes`]).
//! Of two instances with one output file, the later is kept, as fontmake writes each in turn.
//!
//! UFOs are staged in temporary folders beside their targets and swapped in once every instance
//! is built ([`super::stage_folder`], [`super::commit`]), so a failure leaves existing files as
//! they were.
// Nothing reads a source by path in the browser, so most of this is unused there.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::path::{Component, Path, PathBuf};

use fontcraft_interp::compat::{self, MasterPairs};
use fontcraft_interp::{Designspace, GlyphSources, UserLocation};
use fontcraft_model::plist::xml;
use fontcraft_model::plist::{Number, Value as PValue};
use fontcraft_model::{Family, Font, Instance, SourceFormat};
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

/// What the instances are written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ufo,
    /// A compiled font: `ttf` or `otf`.
    Font(&'static str),
}

/// One instance to build.
pub(super) struct Job {
    /// Its index in the prepared font's instances.
    pub(super) index: usize,
    /// Made from `location` rather than one of the source's instances.
    pub(super) from_location: bool,
    /// Its location in user coordinates, by axis tag.
    pub(super) location: Value,
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
    if p.bool_or("layoutOnly", false)? {
        return super::layout::instantiate(p, input, req);
    }
    let binary = binary_layout(p)?;
    if p.get("masterFinder").is_some() && binary.is_none() {
        return Err(p.bad("`masterFinder` finds the compiled masters `layoutOnly` and `interpolateBinaryLayout` read: give one of them too"));
    }
    if req.in_place {
        return Err(p.bad("an instance never replaces its source: give `output` or `outputDir`"));
    }
    let kind = kind(p, req)?;
    if binary.is_some() && kind == Kind::Ufo {
        return Err(p.bad("`interpolateBinaryLayout` replaces the layout of compiled instances: give `to` \"ttf\" or \"otf\""));
    }
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
    refuse_without_axes(p, &font)?;
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
            let layout = match &binary {
                Some(dir) => Some(super::layout::BinaryLayout::load(p, input, &font, &designspace, &jobs, ext, dir.as_deref())?),
                None => None,
            };
            compile_fonts(p, font, input, req, &jobs, ext, autohint, remove_overlaps, layout.as_ref(), &mut warnings)?
        }
    };
    Ok(json!({ "results": results, "warnings": warnings }))
}

/// Refuses a designspace without `<axes>` that fontTools cannot interpolate ([`without_axes`]).
pub(super) fn refuse_without_axes(p: &Params<'_>, font: &Font) -> Result<()> {
    match without_axes(font, true, true) {
        Some(why) => Err(p.bad(why)),
        None => Ok(()),
    }
}

/// Why a designspace without `<axes>` cannot be built as fontmake builds it; `None` when it can,
/// or when the font is not such a designspace.
///
/// fontTools' reader fails on the first `<location>` it reads ("No axes defined"): one in a
/// location label, a source, an instance, or an instance's glyph or glyph master. Without axes
/// those locations cannot be read. A document without axes and without any location loads, with
/// every source at one location: its sources can be built one by one, but interpolating
/// (`interpolates`) between two or more of them fails ("Locations must be unique"); a single
/// source is its own instance.
///
/// A font without a kept document states a location when `unkept_states_location`: a source
/// file just read always keeps its document, while a font saved as a designspace from another
/// format has none, and its sources state no location for fontTools to refuse.
pub(in crate::cmd) fn without_axes(font: &Font, interpolates: bool, unkept_states_location: bool) -> Option<String> {
    if font.source.format != SourceFormat::Designspace || !font.axes.is_empty() {
        return None;
    }
    let document = font.extra.get(Family::Designspace, "document").and_then(PValue::as_str);
    let states_location = match document.map(xml::parse) {
        Some(Ok(doc)) => states_a_location(&doc.root),
        Some(Err(_)) => true,
        None => unkept_states_location,
    };
    if states_location {
        return Some("the designspace defines no axes, so its sources have no locations to interpolate between".into());
    }
    let sources = font.masters.len();
    (interpolates && sources > 1).then(|| {
        format!("the designspace defines no axes, so its {sources} sources sit at one location and no instance can be interpolated between them")
    })
}

/// Does the designspace `root` have a `<location>` where fontTools' reader reads one?
fn states_a_location(root: &xml::Element) -> bool {
    fn children<'a>(e: &'a xml::Element, list: &str, item: &'a str) -> impl Iterator<Item = &'a xml::Element> {
        e.child(list).into_iter().flat_map(move |l| l.elements().filter(move |c| c.name() == item))
    }
    let instances = || children(root, "instances", "instance");
    let glyphs = || instances().flat_map(|i| children(i, "glyphs", "glyph"));
    let mut places = children(root, "labels", "label")
        .chain(children(root, "sources", "source"))
        .chain(instances())
        .chain(glyphs())
        .chain(glyphs().flat_map(|g| children(g, "masters", "master")));
    places.any(|e| e.child("location").is_some())
}

/// `interpolateBinaryLayout`: `None` when off, else the folder of the compiled masters it names
/// (`None` inside for the default).
fn binary_layout(p: &Params<'_>) -> Result<Option<Option<String>>> {
    match p.get("interpolateBinaryLayout") {
        None | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(None)),
        Some(Value::String(dir)) if !dir.is_empty() => Ok(Some(Some(dir.clone()))),
        Some(_) => Err(p.bad("`interpolateBinaryLayout` is true, false or the folder of the compiled masters")),
    }
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
pub(super) fn jobs(p: &Params<'_>, font: &mut Font, ds: &Designspace, clamp: bool) -> Result<Vec<Job>> {
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
    let sources = source_paths(input)?;
    // Every target is worked out and checked before anything is built.
    let mut plans: Vec<(&Job, &Instance, fontcraft_io_ufo::designspace::InstanceNames, PathBuf, String)> = Vec::with_capacity(jobs.len());
    for job in jobs {
        let inst = font.instances.get(job.index).ok_or_else(|| p.bad(format!("there is no instance #{}", job.index)))?;
        let names = fontcraft_io_ufo::designspace::instance_names(font, inst);
        let label = &inst.name;
        if names.filename_from_names && !job.from_location && (label.contains('/') || names.family.contains('/')) {
            return Err(EngineError::Io(format!(
                "instance `{label}`: a family or instance name with `/` gives no file name; give the instance a UFO file name"
            )));
        }
        let out = ufo_path(input, req, instance_dir, job, &names.filename)?;
        let at = file_io::resolved(&out);
        if let Some(source) = sources.iter().find(|s| at.starts_with(s) || s.starts_with(&at)) {
            return Err(p.bad(format!(
                "instance `{label}` would be written to {}, which would replace {}, a file the source reads",
                out.display(),
                source.display()
            )));
        }
        file_io::check_write(&out)?;
        plans.push((job, inst, names, out, same_file_key(&at)));
    }
    let mut results = Vec::with_capacity(jobs.len());
    for (k, (job, inst, names, out, key)) in plans.iter().enumerate() {
        let label = inst.name.clone();
        // As fontmake writes each instance in turn, of two instances with one file the later is
        // the one left on disk; the earlier is not built.
        if let Some((_, later, ..)) = plans.iter().skip(k.saturating_add(1)).find(|plan| plan.4 == *key) {
            warnings.push(format!(
                "instance `{label}` is not written: instance `{}`, later in the list, is written to the same file {}; give the instances different file names to keep both",
                later.name,
                out.display()
            ));
            continue;
        }
        let (made, notes) = generated_font(p, font, job.index).map_err(|e| match e {
            EngineError::BadParams { message, .. } => p.bad(format!("instance `{label}`: {message}")),
            other => other,
        })?;
        warnings.extend(notes.into_iter().map(|n| format!("instance `{label}`: {n}")));
        let made = finish(p, font, made, inst, names, steps, skipped, &label, warnings)?;
        if req.no_clobber && out.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", out.display())));
        }
        let master = made.masters.first().ok_or_else(|| p.bad(format!("instance `{label}` has no master")))?;
        // A postscriptFontName parameter of the instance wins over its stated name.
        let postscript = master.params.get("postscriptFontName").and_then(PValue::as_str).map(str::to_string).or_else(|| names.postscript.clone());
        let master = master.id.clone();
        let files = fontcraft_io_ufo::ufo::write_one(&made, &master)?;
        created.push(make_parent(out)?);
        let is_ufo = |dir: &Path| dir.join("metainfo.plist").is_file() || std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none());
        staged.push(stage_folder(out, &files, staged.len(), is_ufo)?);
        let mut entry = json!({ "output": out.display().to_string(), "location": job.location.clone(), "static": true });
        if let Some(obj) = entry.as_object_mut() {
            if !job.from_location {
                obj.insert("name".into(), json!(label));
            }
            if let Some(ps) = &postscript {
                obj.insert("postscriptName".into(), json!(ps));
            }
        }
        results.push(entry);
    }
    Ok(results)
}

/// The key under which two output paths that can name the same file meet: some file systems
/// ignore case (as font.compile compares its outputs).
fn same_file_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
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
    let bad = || EngineError::Io(format!("the instance file name `{filename}` does not name a UFO folder"));
    // As in fontmake, a recorded name is relative to the source's folder and may climb out of
    // it (`../instance_ufo/X.ufo`); build_ufos refuses a target on or around a source file.
    if filename.is_empty() || filename.contains('\\') || !relative_parts(Path::new(filename)) {
        return Err(bad());
    }
    let last = filename.rsplit('/').find(|s| !s.is_empty()).filter(|s| *s != "." && *s != "..").ok_or_else(bad)?;
    match dir {
        Some(dir) => Ok(dir.join(last)),
        None => {
            let base = input.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
            Ok(normalized(&base.join(filename)))
        }
    }
}

/// Whether `path` is made only of names, `.` and `..`: no root and no Windows drive prefix, whose
/// join would replace the source's folder.
fn relative_parts(path: &Path) -> bool {
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir | Component::ParentDir))
}

/// `path` with `.` dropped and each `..` folded into the part before it, by spelling only (as
/// fontmake resolves instance file names); a leading `..` stays.
fn normalized(path: &Path) -> PathBuf {
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
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// The source at `input` and every file opening it reads (a designspace's UFOs, the files of
/// Import Font and Import Master parameters), each [`file_io::resolved`]. An instance is never written
/// over one of them, inside one, or around one.
#[cfg(not(target_arch = "wasm32"))]
fn source_paths(input: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = vec![file_io::resolved(input)];
    crate::io::check_references(input, &mut |p| {
        paths.push(file_io::resolved(p));
        Ok(())
    })
    .map_err(EngineError::Io)?;
    Ok(paths)
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
    font: &Font,
    made: Font,
    inst: &Instance,
    names: &fontcraft_io_ufo::designspace::InstanceNames,
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
    let lone = font.masters.len() == 1 && font.master_at(&inst.location).is_some();
    let info = std::sync::Arc::make_mut(&mut made.info);
    let has = |params: &fontcraft_model::Params, keys: &[&str]| keys.iter().any(|k| params.get(k).is_some());
    let font_weight = has(&info.params, &["weightClass", "openTypeOS2WeightClass"]);
    let font_width = has(&info.params, &["widthClass", "openTypeOS2WidthClass"]);
    fontcraft_model::strip_instance_info(&mut info.properties, &mut info.params, lone);
    let masters = std::sync::Arc::make_mut(&mut made.masters);
    let master = masters.first_mut().ok_or_else(|| p.bad(format!("instance `{label}` has no master")))?;
    fontcraft_model::strip_instance_info(&mut master.properties, &mut master.params, lone);
    // The instance lists only the layers its glyphs use, not the ones the default source lists
    // without having them on disk.
    master.extra.remove(fontcraft_model::Family::Ufo, "missingLayers");
    if !lone {
        // Before the class is set from the location below.
        let class = ["weightClass", "openTypeOS2WeightClass"]
            .iter()
            .find_map(|k| master.params.get(k).or_else(|| info.params.get(k)))
            .and_then(PValue::as_num)
            .map(|n| n.as_f64());
        match class.and_then(postscript_weight_name) {
            Some(name) => master.params.set("postscriptWeightName", PValue::String(name.into())),
            None => {
                master.params.remove("postscriptWeightName");
            }
        }
        shared_panose(font, &mut info.params, &mut master.params);
    }
    glyphs_defaults(font, &mut info.params, master);
    let text = |s: &str| PValue::String(s.into());
    if let Some(ps) = &names.postscript {
        master.params.set("postscriptFontName", text(ps));
    }
    master.params.set("styleMapFamilyName", text(&names.style_map_family));
    // fontTools' instantiator writes the stated style-map style in lower case.
    master.params.set("styleMapStyleName", text(&names.style_map_style.to_lowercase()));
    // fontmake sets the classes from the location on a wght or wdth axis whatever the masters
    // state (glyphsLib's apply_instance_data after the instantiator); without the axis, from the
    // instance's own class when the masters state none.
    let class = |tag: &str, param: &str, known: bool| {
        inst.params.get(param).cloned().or_else(|| {
            let value = fontcraft_model::location_class(font, &inst.location, tag).or_else(|| {
                let own = if tag == "wght" { inst.weight_class.is_some() } else { inst.width_class.is_some() };
                (own && !known).then(|| fontcraft_model::user_class(tag, fontcraft_model::axes::instance_class_value(inst, tag))).flatten()
            });
            value.map(|v| PValue::Number(Number::int(v)))
        })
    };
    let weight = class("wght", "weightClass", font_weight || has(&master.params, &["weightClass", "openTypeOS2WeightClass"]));
    let width = class("wdth", "widthClass", font_width || has(&master.params, &["widthClass", "openTypeOS2WidthClass"]));
    if let Some(w) = weight {
        master.params.set("weightClass", w);
    }
    if let Some(w) = width {
        master.params.set("widthClass", w);
    }
    custom_params(&mut made, inst, label, warnings);
    Ok(made)
}

/// The PostScript weight name fontMath gives an interpolated OS/2 weight class: the class rounded
/// to hundreds (halves away from zero), kept within 100 to 900.
fn postscript_weight_name(class: f64) -> Option<&'static str> {
    const NAMES: [&str; 9] = ["Thin", "Extra-light", "Light", "Normal", "Medium", "Semi-bold", "Bold", "Extra-bold", "Black"];
    if !class.is_finite() {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = ((class / 100.0).round().clamp(1.0, 9.0) as usize).saturating_sub(1);
    NAMES.get(index).copied()
}

/// The PANOSE digits of an instance between masters, as ufo2ft's instantiator gives them: only
/// when the default source has some, each digit the sources that have PANOSE share, 0 where they
/// differ, and none when every digit is 0.
#[allow(clippy::cast_possible_truncation)]
fn shared_panose(font: &Font, info: &mut fontcraft_model::Params, own: &mut fontcraft_model::Params) {
    const PANOSE: &str = "panose";
    const DIGITS: usize = 10;
    let has_default = own.get(PANOSE).or_else(|| info.get(PANOSE)).is_some();
    info.remove(PANOSE);
    own.remove(PANOSE);
    if !has_default {
        return;
    }
    let digits = |v: &PValue| -> Option<Vec<f64>> {
        v.as_array().map(|a| a.items.iter().take(DIGITS).map(|d| d.as_num().map_or(f64::NAN, |n| n.as_f64())).collect())
    };
    let lists: Vec<Vec<f64>> =
        font.masters.iter().filter_map(|m| m.params.get(PANOSE).or_else(|| font.info.params.get(PANOSE))).filter_map(digits).collect();
    let len = lists.iter().map(Vec::len).min().unwrap_or(0);
    let shared: Vec<i64> = (0..DIGITS)
        .map(|i| {
            let mut values = lists.iter().map(|l| l.get(i).copied().unwrap_or(f64::NAN));
            let first = values.next().unwrap_or(f64::NAN);
            if i < len && first.is_finite() && values.all(|v| v == first) { first.round() as i64 } else { 0 }
        })
        .collect();
    if shared.iter().any(|d| *d != 0) {
        let items = shared.into_iter().map(|d| PValue::Number(Number::int(d))).collect();
        own.set(PANOSE, PValue::Array(fontcraft_model::plist::Array { items, ..Default::default() }));
    }
}

/// glyphsLib's defaults where the font information states none (`DEFAULT_PARAMETERS`, which
/// fontmake applies to every instance UFO): (parameter, UFO key, value).
const GLYPHS_DEFAULTS: [(&str, &str, i64); 3] = [
    ("fsType", "openTypeOS2Type", 3),
    ("underlineThickness", "postscriptUnderlineThickness", 50),
    ("underlinePosition", "postscriptUnderlinePosition", -100),
];

/// Gives the instance glyphsLib's defaults where its sources state none. A UFO-family source's
/// reader fills these keys with ufo2ft's defaults, so there the sources' files decide: the
/// default source's for the embedding type, which is copied from it, any source's for the
/// underline, which is interpolated. Each value is put in the instance's font information as
/// read, so that the UFO writer writes it even where it equals ufo2ft's default.
fn glyphs_defaults(font: &Font, info: &mut fontcraft_model::Params, master: &mut fontcraft_model::Master) {
    use fontcraft_model::Family;
    const FONTINFO: &str = "fontinfo.plist";
    let file_has =
        |m: &fontcraft_model::Master, key: &str| m.extra.get(Family::Ufo, FONTINFO).and_then(PValue::as_dict).is_some_and(|d| d.get(key).is_some());
    let ufo = fontcraft_model::is_ufo_family(font.source.format);
    for (param, key, value) in GLYPHS_DEFAULTS {
        let stated = if !ufo {
            master.params.get(param).or_else(|| info.get(param)).is_some()
        } else if param == "fsType" {
            file_has(master, key)
        } else {
            font.masters.iter().any(|m| file_has(m, key))
        };
        if stated {
            continue;
        }
        info.remove(param);
        master.params.remove(param);
        let value = if param == "fsType" {
            PValue::Array(fontcraft_model::plist::Array { items: vec![PValue::Number(Number::int(value))], ..Default::default() })
        } else {
            PValue::Number(Number::int(value))
        };
        let mut read = master.extra.get(Family::Ufo, FONTINFO).and_then(PValue::as_dict).cloned().unwrap_or_default();
        read.insert(key, value);
        master.extra.insert(Family::Ufo, FONTINFO, PValue::Dict(read));
    }
}

/// Instance parameters a UFO instance does not take: the ones interpolation and the names and
/// classes above have used, and the family name, which the designspace's `familyname` states.
const NOT_CARRIED: [&str; 10] = [
    "InterpolationWeightY",
    "Local Interpolation",
    "weightClass",
    "widthClass",
    "familyName",
    "UFO Filename",
    "Rename Glyphs",
    "Reencode Glyphs",
    "Replace Feature",
    "Replace Prefix",
];

/// The properties glyphsLib copies from an instance into its UFO's font information
/// (`PROPERTIES_FIELDS`).
const CARRIED_PROPERTIES: [&str; 10] = [
    "copyrights",
    "familyNames",
    "trademarks",
    "manufacturers",
    "manufacturerURL",
    "designers",
    "designerURL",
    "descriptions",
    "licenses",
    "sampleTexts",
];

/// The instance's custom parameters and properties, as glyphsLib's `apply_instance_data_to_ufo`
/// gives them to a UFO instance (`builder/instances.py`, `builder/custom_params.py`; Apache-2.0):
/// each parameter on the instance's master, where the UFO writer puts it in the font information
/// or the lib; Rename Glyphs and Reencode Glyphs applied to the glyphs, and Replace Feature and
/// Replace Prefix to the features. Values that cannot be used are skipped with a warning.
fn custom_params(made: &mut Font, inst: &Instance, label: &str, warnings: &mut Vec<String>) {
    let params: Vec<&fontcraft_model::Param> = inst.params.0.iter().filter(|p| !p.disabled).collect();
    if let Some(master) = std::sync::Arc::make_mut(&mut made.masters).first_mut() {
        for p in params.iter().filter(|p| !NOT_CARRIED.contains(&p.name.as_str())) {
            master.params.set(&p.name, p.value.clone());
        }
        for prop in inst.properties.0.iter().filter(|p| CARRIED_PROPERTIES.contains(&p.key.as_str())) {
            match master.properties.0.iter_mut().find(|q| q.key == prop.key) {
                Some(q) => *q = prop.clone(),
                None => master.properties.0.push(prop.clone()),
            }
        }
    }
    for p in params.iter().filter(|p| p.name == "Rename Glyphs") {
        for entry in entries(&p.value) {
            let Some((old, new)) = entry.split_once('=').filter(|(_, new)| !new.contains('=')) else {
                warnings.push(format!("instance `{label}`: Rename Glyphs entry `{entry}` is not `old=new`; it was skipped"));
                continue;
            };
            let (old, new) = (old.trim(), new.trim());
            let (Some(a), Some(b)) = (made.glyph(old).map(|g| g.layers.clone()), made.glyph(new).map(|g| g.layers.clone())) else {
                warnings.push(format!("instance `{label}`: Rename Glyphs cannot swap `{old}` and `{new}`: the instance lacks one of them"));
                continue;
            };
            // The drawings swap; the code points stay with the names.
            if let Some(mut g) = made.glyphs.get_mut(old) {
                g.layers = b;
            }
            if let Some(mut g) = made.glyphs.get_mut(new) {
                g.layers = a;
            }
        }
    }
    for p in params.iter().filter(|p| p.name == "Reencode Glyphs") {
        reencode(made, &entries(&p.value), label, warnings);
    }
    let feature_params = fontcraft_model::Params(
        params.iter().filter(|p| matches!(p.name.as_str(), "Replace Feature" | "Replace Prefix")).map(|p| (*p).clone()).collect(),
    );
    if !feature_params.0.is_empty() {
        match fontcraft_features::params::apply_instance_params(&made.features, &feature_params) {
            Ok(features) => made.features = std::sync::Arc::new(features),
            Err(e) => warnings.push(format!("instance `{label}`: the feature parameters were not applied: {e}")),
        }
    }
}

/// The text entries of a list parameter, or of one text value.
fn entries(v: &PValue) -> Vec<String> {
    match v {
        PValue::String(s) => vec![s.as_str().to_string()],
        PValue::Array(a) => a.items.iter().filter_map(PValue::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Reencode Glyphs (`name=hex`, or `name=` to clear), as glyphsLib applies it: a code point
/// another glyph has as its first is taken from that glyph with all of its code points, and
/// becomes the first of the named glyph's.
fn reencode(made: &mut Font, list: &[String], label: &str, warnings: &mut Vec<String>) {
    let mut cmap: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for g in made.glyphs.as_arcs().iter() {
        if let Some(&u) = g.unicodes.first() {
            cmap.insert(u, g.name.clone());
        }
    }
    for entry in list {
        let Some((name, hex)) = entry.split_once('=').filter(|(_, hex)| !hex.contains('=')) else {
            warnings.push(format!("instance `{label}`: Reencode Glyphs entry `{entry}` is not `name=code`; it was skipped"));
            continue;
        };
        if made.glyph(name).is_none() {
            continue;
        }
        let hex = hex.trim();
        if hex.is_empty() {
            if let Some(mut g) = made.glyphs.get_mut(name) {
                g.unicodes.clear();
            }
            continue;
        }
        let Some(code) = u32::from_str_radix(hex, 16).ok().filter(|c| char::from_u32(*c).is_some()) else {
            warnings.push(format!("instance `{label}`: Reencode Glyphs entry `{entry}` has no code point; it was skipped"));
            continue;
        };
        if let Some(previous) = cmap.get(&code)
            && let Some(mut g) = made.glyphs.get_mut(previous)
        {
            g.unicodes.clear();
        }
        if let Some(mut g) = made.glyphs.get_mut(name)
            && g.unicodes.first() != Some(&code)
        {
            g.unicodes.retain(|u| *u != code);
            g.unicodes.insert(0, code);
        }
    }
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
    // The steps run as editor commands, and a session aligns the composites a command touches.
    // An instance keeps its components where interpolation put them, as fontmake's do, so
    // alignment is off while the steps run and the source's setting is put back after.
    let alignment = font.info.settings.disables_automatic_alignment;
    std::sync::Arc::make_mut(&mut font.info).settings.disables_automatic_alignment = Some(true);
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
    let mut font = Font::clone(&s.doc()?.font);
    std::sync::Arc::make_mut(&mut font.info).settings.disables_automatic_alignment = alignment;
    Ok(font)
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
    layout: Option<&super::layout::BinaryLayout>,
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let mut font = font;
    let labels: Vec<String> = jobs
        .iter()
        .map(|j| font.instances.get(j.index).map(|i| if i.name.is_empty() { format!("#{}", j.index) } else { i.name.clone() }).unwrap_or_default())
        .collect();
    // font.compile chooses instances by name, and two instances can share one, so the jobs are
    // chosen by index instead: they alone export, and font.compile builds every exporting one.
    let instances = std::sync::Arc::make_mut(&mut font.instances);
    for (i, inst) in instances.iter_mut().enumerate() {
        if inst.kind.is_none() {
            inst.exports = jobs.iter().any(|j| j.index == i);
        }
    }
    if let Some(on) = autohint {
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
    let mut params = json!({ "formats": [ext], "instances": true, "destination": destination.display().to_string() });
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
    // When the layout is replaced afterwards, the fonts are compiled into a scratch folder first,
    // so that nothing reaches the destination before every font is final.
    let mut scratch = match layout {
        Some(_) => {
            let (scratch, to) = Scratch::new(&destination, single)?;
            if let Some(obj) = params.as_object_mut() {
                obj.insert("destination".into(), json!(to.display().to_string()));
            }
            Some(scratch)
        }
        None => None,
    };
    let r = s.execute("font.compile", &params)?;
    let shown = |text: &str| match &scratch {
        Some(sc) => text.replace(&sc.folder.display().to_string(), &sc.base.display().to_string()),
        None => text.to_string(),
    };
    warnings.extend(r.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(shown));
    for skip in r.get("skipped").and_then(Value::as_array).into_iter().flatten() {
        let name = skip.get("instance").and_then(Value::as_str).unwrap_or("?");
        let reason = skip.get("reason").and_then(Value::as_str).unwrap_or("skipped");
        warnings.push(format!("instance `{name}` was not built: {reason}"));
    }
    // font.compile reports its fonts in the font's order, so the n-th font labelled `label`
    // belongs to the n-th job, by index, with that label.
    let mut by_index: Vec<(&Job, &String)> = jobs.iter().zip(&labels).collect();
    by_index.sort_by_key(|(j, _)| j.index);
    let mut taken = vec![false; by_index.len()];
    // Each entry: the job's place, the result entry, the file's key, and where font.compile wrote it.
    let mut built: Vec<(usize, Value, String, PathBuf)> = Vec::new();
    for f in r.get("fonts").and_then(Value::as_array).into_iter().flatten() {
        let label = f.get("instance").and_then(Value::as_str).unwrap_or_default();
        let found = by_index.iter().zip(taken.iter_mut()).find(|((_, l), taken)| !**taken && l.as_str() == label);
        let Some(((job, _), taken)) = found else { continue };
        *taken = true;
        let order = jobs.iter().position(|j| j.index == job.index).unwrap_or(usize::MAX);
        let entry_warnings = f.get("warnings").cloned().unwrap_or_else(|| json!([]));
        for file in f.get("files").and_then(Value::as_array).into_iter().flatten() {
            let written = PathBuf::from(file.get("path").and_then(Value::as_str).unwrap_or_default());
            let out = scratch.as_ref().map_or_else(|| Some(written.clone()), |sc| sc.final_path(&written));
            let Some(out) = out.filter(|o| !o.as_os_str().is_empty()) else { continue };
            let path = out.display().to_string();
            // font.compile writes the fonts in order and keeps the later of two with one file,
            // as fontmake does; the earlier one's entry goes.
            let key = same_file_key(&out);
            if let Some(k) = built.iter().position(|(_, _, other, _)| *other == key) {
                let (_, earlier, ..) = built.remove(k);
                let name = earlier.get("name").and_then(Value::as_str).unwrap_or("?");
                warnings.push(format!("instance `{name}` is not kept: instance `{label}`, later in the source, is written to the same file {path}"));
            }
            let mut entry = json!({ "output": path, "location": job.location.clone(), "static": true, "warnings": entry_warnings.clone() });
            if let (false, Some(obj)) = (job.from_location, entry.as_object_mut()) {
                obj.insert("name".into(), json!(label));
            }
            built.push((order, entry, key, written));
        }
    }
    // In the order the jobs were asked for.
    built.sort_by_key(|(order, ..)| *order);
    if let (Some(layout), Some(sc)) = (layout, scratch.as_mut()) {
        let mut outputs: Vec<(usize, PathBuf, PathBuf)> = Vec::with_capacity(built.len());
        for (order, entry, _, written) in &built {
            let (Some(job), Some(out)) = (jobs.get(*order), entry.get("output").and_then(Value::as_str)) else { continue };
            let out = PathBuf::from(out);
            sc.make_parent(&out)?;
            outputs.push((job.index, written.clone(), out));
        }
        layout.apply(p, &outputs)?;
        sc.keep_made = true;
    }
    Ok(built.into_iter().map(|(_, entry, ..)| entry).collect())
}

/// A folder inside the destination that instances are compiled into when their layout is
/// replaced afterwards. Dropping it removes it, and, unless `keep_made` is set, the folders made
/// for it and for the outputs.
#[cfg(not(target_arch = "wasm32"))]
struct Scratch {
    folder: PathBuf,
    /// The folder `folder` stands in for.
    base: PathBuf,
    made: Vec<PathBuf>,
    keep_made: bool,
}

/// Names tried for a scratch folder before giving up.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SCRATCH_NAMES: usize = 16;

#[cfg(not(target_arch = "wasm32"))]
impl Scratch {
    /// A new scratch folder for `destination` (one file when `single`, else a folder); answers
    /// it with the destination to compile to inside it.
    fn new(destination: &Path, single: bool) -> Result<(Self, PathBuf)> {
        let destination = super::super::compile::absolute(destination);
        let (base, name) = if single {
            match (destination.parent(), destination.file_name()) {
                (Some(parent), Some(name)) => (parent.to_path_buf(), Some(name.to_os_string())),
                _ => return Err(EngineError::Io(format!("cannot write {}: it is not a file path", destination.display()))),
            }
        } else {
            (destination.clone(), None)
        };
        let made = make_parent(&base.join("scratch"))?;
        let mut scratch = Self { folder: PathBuf::new(), base, made, keep_made: false };
        for n in 0..MAX_SCRATCH_NAMES {
            let folder = scratch.base.join(format!(".fontcraft-instances.{}-{n}.tmp", std::process::id()));
            file_io::check_write(&folder)?;
            match std::fs::create_dir(&folder) {
                Ok(()) => {
                    let to = name.map_or_else(|| folder.clone(), |name| folder.join(name));
                    scratch.folder = folder;
                    return Ok((scratch, to));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(EngineError::Io(format!("cannot create {}: {e}", folder.display()))),
            }
        }
        Err(EngineError::Io(format!("cannot create a scratch folder in {}: every name tried is taken", scratch.base.display())))
    }

    /// Where a file font.compile wrote at `written` goes.
    fn final_path(&self, written: &Path) -> Option<PathBuf> {
        written.strip_prefix(&self.folder).ok().map(|rel| self.base.join(rel))
    }

    /// Creates the missing folders above `out`, to be removed again if the run fails.
    fn make_parent(&mut self, out: &Path) -> Result<()> {
        let made = make_parent(out)?;
        self.made.extend(made);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.folder.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
        if !self.keep_made {
            // Deepest first; a folder that is not empty stays.
            self.made.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
            for dir in &self.made {
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_name_with_a_root_or_drive_is_not_relative() {
        assert!(relative_parts(Path::new("../instance_ufo/X.ufo")));
        assert!(relative_parts(Path::new("./X.ufo")));
        assert!(!relative_parts(Path::new("/tmp/X.ufo")));
        // Only Windows parses a drive prefix; elsewhere `C:x` is a plain name.
        #[cfg(windows)]
        {
            assert!(!relative_parts(Path::new("C:x/Y.ufo")));
            assert!(!relative_parts(Path::new(r"\\server\share\Y.ufo")));
        }
    }

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

    /// A master of a designspace built as an instance is named after its UFO, as fontmake names it.
    #[test]
    fn masters_built_as_instances_are_named_after_their_ufos() {
        let (dir, ds) = source("master-names");
        let mut s = Session::new();
        s.execute("file.open", &json!({ "path": ds })).unwrap();
        let mut names = |extra: Value| {
            let mut params = json!({ "formats": ["ttf"], "instances": false, "mastersAsInstances": true });
            params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let built = s.execute("font.compile", &params).unwrap();
            let fonts = built["fonts"].as_array().unwrap();
            fonts.iter().map(|f| f["files"][0]["name"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        };
        assert_eq!(names(json!({})), ["Light.ttf", "Bold.ttf"]);
        assert_eq!(names(json!({ "nameBy": "postscript-name" })), ["Test-Light.ttf", "Test-Bold.ttf"]);
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn instantiate_source_writes_only_the_style_map_names_the_designspace_states() {
        let (dir, ds) = source("stylemap");
        let mut s = Session::new();
        let out = dir.join("instances").join("Test-Regular.ufo");
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let info = std::fs::read_to_string(out.join("fontinfo.plist")).unwrap();
        assert!(!info.contains("styleMapFamilyName") && !info.contains("styleMapStyleName"), "fontmake leaves them to ufo2ft: {info}");

        let text = std::fs::read_to_string(&ds).unwrap().replace("postscriptfontname=", "stylemapstylename=\"bold\" postscriptfontname=");
        std::fs::write(&ds, text).unwrap();
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let info = std::fs::read_to_string(out.join("fontinfo.plist")).unwrap();
        assert!(info.contains("<key>styleMapStyleName</key>") && info.contains("<string>bold</string>"), "the stated style: {info}");
        assert!(!info.contains("styleMapFamilyName"), "the family is still left out: {info}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instantiate_source_lists_only_the_layers_the_instance_has() {
        let (dir, ds) = source("layers");
        std::fs::write(
            dir.join("Light.ufo/layercontents.plist"),
            format!("{PLIST}<array><array><string>public.default</string><string>glyphs</string></array><array><string>{{600}}</string><string>glyphs.{{600}}</string></array></array></plist>\n"),
        )
        .unwrap();
        let mut s = Session::new();
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let out = dir.join("instances").join("Test-Regular.ufo");
        let lc = std::fs::read_to_string(out.join("layercontents.plist")).unwrap();
        assert!(lc.contains("<string>glyphs</string>") && !lc.contains("{600}"), "a layer the default source lists but does not have: {lc}");
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

    /// An instance between masters takes the font information fontmake writes: the PostScript
    /// weight name of the interpolated class, the PANOSE digits both sources share (0 where they
    /// differ), and glyphsLib's embedding type and underline where no source states them.
    /// Expected values from `fontmake -i` on the same two masters.
    #[test]
    fn instantiate_source_writes_the_font_info_fontmake_derives() {
        let (dir, ds) = source("derived-info");
        for (style, class, digit) in [("Light", 300, 5), ("Bold", 700, 8)] {
            let info = dir.join(format!("{style}.ufo/fontinfo.plist"));
            let text = std::fs::read_to_string(&info).unwrap();
            let panose: String = [2, 11, digit, 2, 4, 5, 4, 2, 2, 4].iter().map(|d| format!("<integer>{d}</integer>")).collect();
            let added = format!(
                "<key>openTypeOS2WeightClass</key><integer>{class}</integer><key>openTypeOS2Panose</key><array>{panose}</array><key>unitsPerEm</key>"
            );
            std::fs::write(&info, text.replace("<key>unitsPerEm</key>", &added)).unwrap();
        }
        let mut s = Session::new();
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let out: String = std::fs::read_to_string(dir.join("instances/Test-Regular.ufo/fontinfo.plist")).unwrap().split_whitespace().collect();
        let shared: String = [2, 11, 0, 2, 4, 5, 4, 2, 2, 4].iter().map(|d| format!("<integer>{d}</integer>")).collect();
        for want in [
            "<key>postscriptWeightName</key><string>Medium</string>".to_string(),
            format!("<key>openTypeOS2Panose</key><array>{shared}</array>"),
            "<key>openTypeOS2Type</key><array><integer>3</integer></array>".to_string(),
            "<key>postscriptUnderlineThickness</key><integer>50</integer>".to_string(),
            "<key>postscriptUnderlinePosition</key><integer>-100</integer>".to_string(),
        ] {
            assert!(out.contains(&want), "{want} in {out}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A designspace without `<axes>` (fontTools' InterpolateLayout3) puts every source at one
    /// location: both instance paths refuse it as fontTools does, naming the missing axes rather
    /// than two `.notdef` layers at one location.
    #[test]
    fn instantiate_source_refuses_a_designspace_without_axes() {
        let (dir, ds) = source("no-axes");
        let text = std::fs::read_to_string(&ds).unwrap();
        let start = text.find("<axes>").unwrap();
        let end = text.find("</axes>").unwrap() + "</axes>".len();
        std::fs::write(&ds, format!("{}{}", text.get(..start).unwrap(), text.get(end..).unwrap())).unwrap();
        let mut s = Session::new();
        let err = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap_err().to_string();
        assert!(err.contains("defines no axes") && !err.contains(".notdef"), "{err}");
        let out = dir.join("layout.ttf");
        let err = s.execute("font.instantiate", &json!({ "input": ds, "layoutOnly": true, "output": out })).unwrap_err().to_string();
        assert!(err.contains("defines no axes"), "{err}");
        assert!(!dir.join("instances").exists() && !out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// fontTools reads a designspace without axes when nothing in it states a location: such a
    /// document is not refused for its missing axes.
    #[test]
    fn instantiate_source_reads_a_designspace_without_axes_or_locations() {
        let (dir, ds) = source("no-axes-no-locations");
        std::fs::write(
            &ds,
            r#"<?xml version='1.0' encoding='UTF-8'?>
<designspace format="4.1">
  <sources>
    <source filename="Light.ufo" name="Light"/>
  </sources>
  <instances>
    <instance familyname="Test" stylename="Regular" filename="instances/Test-Regular.ufo" postscriptfontname="Test-Regular"/>
  </instances>
</designspace>
"#,
        )
        .unwrap();
        let mut s = Session::new();
        if let Err(e) = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })) {
            assert!(!e.to_string().contains("defines no axes"), "{e}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// fontmake builds each master of a designspace without axes, and its instances only from a
    /// single source: two sources sit at one location and cannot be interpolated ("Locations
    /// must be unique"). font.compile and font.compilePlan follow it.
    #[test]
    fn compile_of_a_designspace_without_axes_builds_as_fontmake_does() {
        let (dir, ds) = source("compile-no-axes");
        let write = |sources: &str| {
            std::fs::write(
                &ds,
                format!(
                    r#"<?xml version='1.0' encoding='UTF-8'?>
<designspace format="4.1">
  <sources>{sources}</sources>
  <instances>
    <instance familyname="Test" stylename="Regular" filename="instances/Test-Regular.ufo" postscriptfontname="Test-Regular"/>
  </instances>
</designspace>
"#
                ),
            )
            .unwrap()
        };
        let light = r#"<source filename="Light.ufo" name="Light"/>"#;
        write(&format!(r#"{light}<source filename="Bold.ufo" name="Bold"/>"#));
        let mut s = Session::new();
        s.execute("file.open", &json!({ "path": ds })).unwrap();
        let err = s.execute("font.compile", &json!({})).unwrap_err().to_string();
        assert!(err.contains("defines no axes") && err.contains("2 sources"), "{err}");
        let err = s.execute("font.compilePlan", &json!({})).unwrap_err().to_string();
        assert!(err.contains("defines no axes"), "{err}");
        let r = s.execute("font.compile", &json!({ "instances": false, "mastersAsInstances": true })).unwrap();
        assert_eq!(r["fonts"].as_array().map(Vec::len), Some(2), "each master builds: {r}");

        write(light);
        let mut s = Session::new();
        s.execute("file.open", &json!({ "path": ds })).unwrap();
        let r = s.execute("font.compile", &json!({})).unwrap();
        assert_eq!(r["fonts"][0]["instance"], json!("Regular"), "one source is its own instance: {r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// fontc's fontinfo_var.designspace states the style-map style `thin`, which is not one of
    /// the four. fontTools' instantiator keeps it in the UFO instance; ufo2ft writes it
    /// title-cased as name ID 2 with no REGULAR, BOLD or ITALIC bit; and the style-map family,
    /// stated as the family alone, stays name ID 1 (checked with fontmake 3.12.1). fontmake writes
    /// `thin` into the instance UFO's `fontinfo.plist`, which ufoLib's validating reader and fontc
    /// refuse; the instance UFO keeps it in its lib, which ufoLib2 opens with validation
    /// (checked with ufoLib2 0.18.1), and compiles to the same names.
    #[test]
    fn a_thin_style_map_style_is_kept_as_fontmake_keeps_it() {
        let (dir, ds) = source("thin");
        let text = std::fs::read_to_string(&ds).unwrap().replace(
            r#"stylename="Regular" filename="instances/Test-Regular.ufo" postscriptfontname="Test-Regular"><location><dimension name="Weight" xvalue="500"/>"#,
            r#"stylename="Thin" filename="instances/Test-Thin.ufo" postscriptfontname="Test-Thin" stylemapfamilyname="Test" stylemapstylename="thin"><location><dimension name="Weight" xvalue="300"/>"#,
        );
        assert!(text.contains("stylemapstylename"));
        std::fs::write(&ds, text).unwrap();
        let mut s = Session::new();
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let instance = dir.join("instances").join("Test-Thin.ufo");
        let info = std::fs::read_to_string(instance.join("fontinfo.plist")).unwrap();
        assert!(!info.contains("styleMapStyleName") && info.contains("<key>styleMapFamilyName</key>"), "{info}");
        let lib = std::fs::read_to_string(instance.join("lib.plist")).unwrap();
        let kept = lib.split("<key>com.fontcraft.styleMapStyleName</key>").nth(1).map(str::trim_start);
        assert!(kept.is_some_and(|k| k.starts_with("<string>thin</string>")), "{lib}");

        s.execute("file.open", &json!({ "path": ds })).unwrap();
        let out = dir.join("Thin.ttf");
        s.execute("font.compile", &json!({ "instances": ["Thin"], "destination": out })).unwrap();
        let direct = style_names(&out);
        assert_eq!(direct, (Some("Test".into()), Some("Thin".into()), None, None, 0), "IDs 16 and 17 equal 1 and 2; no ITALIC, BOLD or REGULAR bit");

        // The instance UFO compiles to the same names and bits as the instance itself.
        s.execute("file.open", &json!({ "path": dir.join("instances").join("Test-Thin.ufo") })).unwrap();
        let via_ufo = dir.join("Thin-ufo.ttf");
        s.execute("font.compile", &json!({ "destination": via_ufo })).unwrap();
        assert_eq!(style_names(&via_ufo), direct);

        // Bold set on the instance afterwards wins over the stated `thin`.
        s.execute("file.open", &json!({ "path": ds })).unwrap();
        s.execute("font.instance.setStyleLinking", &json!({ "instances": ["Thin"], "isBold": true })).unwrap();
        let bold = dir.join("Thin-bold.ttf");
        s.execute("font.compile", &json!({ "instances": ["Thin"], "destination": bold })).unwrap();
        let (_, id2, _, _, bits) = style_names(&bold);
        assert_eq!((id2.as_deref(), bits), (Some("Bold"), 0x20));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ufo2ft compares a UFO's style-map style with the four lower-case names exactly: `Bold`
    /// sets no style bit and is written title-cased as name ID 2 (checked with fontmake 3.12.1).
    #[test]
    fn a_ufo_style_map_style_is_compared_exactly() {
        let dir = std::env::temp_dir().join(format!("fontcraft-instance-source-boldcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("Bold.ufo");
        ufo(&path, "Bold", (100, 400, 500), &rect(100, 400));
        let info = path.join("fontinfo.plist");
        let text = std::fs::read_to_string(&info)
            .unwrap()
            .replace("<key>styleName</key>", "<key>styleMapStyleName</key><string>Bold</string><key>styleName</key>");
        std::fs::write(&info, text).unwrap();
        let mut s = Session::new();
        s.execute("file.open", &json!({ "path": path })).unwrap();
        let out = dir.join("Bold.ttf");
        s.execute("font.compile", &json!({ "destination": out })).unwrap();
        let (_, id2, _, _, bits) = style_names(&out);
        assert_eq!((id2.as_deref(), bits), (Some("Bold"), 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A font saved as a designspace from another format keeps no designspace document: it
    /// states no location, so it compiles as a designspace without axes and one source does.
    #[test]
    fn a_font_saved_as_a_designspace_compiles() {
        let dir = std::env::temp_dir().join(format!("fontcraft-instance-source-saved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("Light.ufo");
        ufo(&path, "Light", (100, 400, 500), &rect(100, 400));
        let mut s = Session::new();
        s.execute("file.open", &json!({ "path": path })).unwrap();
        s.execute("file.saveAs", &json!({ "path": dir.join("FromUfo.designspace") })).unwrap();
        s.execute("font.compile", &json!({ "destination": dir.join("FromUfo.ttf") })).unwrap();
        // fontTools refuses any `<location>` in a document without `<axes>`.
        let text = std::fs::read_to_string(dir.join("FromUfo.designspace")).unwrap();
        assert!(!text.contains("<location"), "{text}");
        s.execute("file.open", &json!({ "path": dir.join("FromUfo.designspace") })).unwrap();
        s.execute("font.compile", &json!({ "destination": dir.join("Reopened.ttf") })).unwrap();

        s.execute("file.new", &json!({ "familyName": "Fresh" })).unwrap();
        s.execute("file.saveAs", &json!({ "path": dir.join("Fresh.designspace") })).unwrap();
        s.execute("font.compile", &json!({ "destination": dir.join("Fresh.ttf") })).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Name IDs 1, 2, 16 and 17 of the compiled font at `path`, and its ITALIC, BOLD and
    /// REGULAR selection bits.
    fn style_names(path: &Path) -> (Option<String>, Option<String>, Option<String>, Option<String>, u16) {
        use skrifa::raw::TableProvider;
        let bytes = std::fs::read(path).unwrap();
        let font = skrifa::FontRef::new(&bytes).unwrap();
        let name = font.name().unwrap();
        let text = |id: u16| {
            name.name_record()
                .iter()
                .find(|r| r.platform_id() == 3 && r.name_id().to_u16() == id)
                .map(|r| r.string(name.string_data()).unwrap().chars().collect::<String>())
        };
        (text(1), text(2), text(16), text(17), font.os2().unwrap().fs_selection().bits() & 0x61)
    }

    /// Points the designspace's instance at `filename` (or at none) and gives it `style`.
    fn retarget(ds: &Path, filename: Option<&str>, style: &str) {
        let text = std::fs::read_to_string(ds).unwrap();
        let file = filename.map(|f| format!(" filename=\"{f}\"")).unwrap_or_default();
        let text = text.replace("stylename=\"Regular\" filename=\"instances/Test-Regular.ufo\"", &format!("stylename=\"{style}\"{file}"));
        std::fs::write(ds, text).unwrap();
    }

    /// The names in `dir`, sorted.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn instantiate_source_never_writes_over_a_source_ufo() {
        let (dir, ds) = source("over-master");
        let before = names_in(&dir);
        let light = std::fs::read_to_string(dir.join("Light.ufo/fontinfo.plist")).unwrap();
        let original = std::fs::read_to_string(&ds).unwrap();
        let mut s = Session::new();
        // The instance records a master's file name, spelled directly and with a detour.
        for filename in ["Light.ufo", "instances/../Light.ufo", "./Light.ufo"] {
            retarget(&ds, Some(filename), "Regular");
            let err = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap_err();
            assert!(err.to_string().contains("Light.ufo"), "{filename}: {err}");
            std::fs::write(&ds, &original).unwrap();
        }
        // outputDir is the source's folder and the recorded name ends in a master's name.
        retarget(&ds, Some("out/Bold.ufo"), "Regular");
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "outputDir": dir, "autohint": false })).is_err());
        // An instanceDir inside a master.
        assert!(s.execute("font.instantiate", &json!({ "input": ds, "instanceDir": dir.join("Bold.ufo"), "autohint": false })).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("Light.ufo/fontinfo.plist")).unwrap(), light, "the master is as it was");
        assert!(dir.join("Bold.ufo/glyphs/A_.glif").is_file());
        assert_eq!(names_in(&dir), before, "nothing written beside the source");
        assert_eq!(names_in(&dir.join("Bold.ufo")), ["fontinfo.plist", "glyphs", "layercontents.plist", "metainfo.plist"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instantiate_source_resolves_recorded_names_from_the_source_folder() {
        let (root, _) = source("sibling");
        let dir = root.join("master_ufo");
        std::fs::create_dir_all(&dir).unwrap();
        let ds = dir.join("Test.designspace");
        for ufo in ["Light.ufo", "Bold.ufo"] {
            std::fs::rename(root.join(ufo), dir.join(ufo)).unwrap();
        }
        std::fs::rename(root.join("Test.designspace"), &ds).unwrap();
        let light = std::fs::read_to_string(dir.join("Light.ufo/glyphs/A_.glif")).unwrap();
        let original = std::fs::read_to_string(&ds).unwrap();
        let mut s = Session::new();
        // The layout fontmake writes: instances in a folder beside the masters' folder.
        retarget(&ds, Some("../instance_ufo/Test-Regular.ufo"), "Regular");
        let r = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let out = root.join("instance_ufo").join("Test-Regular.ufo");
        assert_eq!(r["results"][0]["output"], json!(out.display().to_string()), "{r}");
        assert_eq!(glyph_a(&out).0, 550.0);
        // Run again: the earlier instance is replaced.
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        assert_eq!(glyph_a(&out).0, 550.0);
        std::fs::write(&ds, &original).unwrap();
        retarget(&ds, Some("./instances/Test-Regular.ufo"), "Regular");
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        assert!(dir.join("instances/Test-Regular.ufo/glyphs/A_.glif").is_file());
        std::fs::write(&ds, &original).unwrap();
        // A detour that lands on a master is refused.
        retarget(&ds, Some("../master_ufo/Light.ufo"), "Regular");
        let err = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap_err();
        assert!(err.to_string().contains("Light.ufo"), "{err}");
        std::fs::write(&ds, &original).unwrap();
        // Names that give no folder name, or an absolute path.
        for (filename, out_dir) in [("..", Some(dir.join("out"))), (".", Some(dir.join("out"))), ("a/..", None), ("/abs/X.ufo", None)] {
            retarget(&ds, Some(filename), "Regular");
            let mut params = json!({ "input": ds, "autohint": false });
            if let Some(o) = &out_dir {
                params["outputDir"] = json!(o);
            }
            let err = s.execute("font.instantiate", &params).unwrap_err();
            assert!(err.to_string().contains("does not name a UFO folder"), "{filename}: {err}");
            std::fs::write(&ds, &original).unwrap();
        }
        // No recorded file name, and a style name that would climb out through the default name.
        retarget(&ds, None, "/../../../victim");
        let err = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap_err();
        assert!(err.to_string().contains("name with `/`"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.join("Light.ufo/glyphs/A_.glif")).unwrap(), light, "the master is as it was");
        assert!(dir.join("Bold.ufo/glyphs/A_.glif").is_file());
        assert_eq!(names_in(&root), ["instance_ufo", "master_ufo"]);
        assert_eq!(names_in(&dir), ["Bold.ufo", "Light.ufo", "Test.designspace", "instances"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn instantiate_source_compiles_instances_that_share_a_name_by_index() {
        let (dir, ds) = source("same-name");
        let text = std::fs::read_to_string(&ds).unwrap();
        let one = |family: &str, ps: &str, at: u32| {
            format!(
                "<instance familyname=\"{family}\" stylename=\"Regular\" postscriptfontname=\"{ps}\"><location><dimension name=\"Weight\" xvalue=\"{at}\"/></location></instance>"
            )
        };
        let start = text.find("<instances>").unwrap();
        let end = text.find("</instances>").unwrap();
        let text = format!(
            "{}<instances>{}{}{}",
            text.get(..start).unwrap(),
            one("Test", "Test-Regular", 400),
            one("Test Wide", "TestWide-Regular", 600),
            text.get(end..).unwrap()
        );
        std::fs::write(&ds, text).unwrap();
        let mut s = Session::new();
        let r = s.execute("font.instantiate", &json!({ "input": ds, "to": "ttf", "outputDir": dir.join("both"), "timestamp": 0 })).unwrap();
        let at: Vec<Value> = r["results"].as_array().unwrap().iter().map(|e| e["location"]["wght"].clone()).collect();
        assert_eq!(at, [json!(400.0), json!(600.0)], "{r}");
        let r = s
            .execute("font.instantiate", &json!({ "input": ds, "indices": [1], "to": "ttf", "outputDir": dir.join("second"), "timestamp": 0 }))
            .unwrap();
        let results = r["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "{r}");
        assert_eq!(results[0]["location"]["wght"], json!(600.0));
        let font = fontcraft_tables::read(&std::fs::read(results[0]["output"].as_str().unwrap()).unwrap(), 0).unwrap();
        assert_eq!(font.get(*b"OS/2").unwrap()[4..6], 600u16.to_be_bytes(), "the second instance, not the first of that name");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Replaces the designspace's instances with `instances`, the XML of each.
    fn set_instances(ds: &Path, instances: &[String]) {
        let text = std::fs::read_to_string(ds).unwrap();
        let start = text.find("<instances>").unwrap();
        let end = text.find("</instances>").unwrap();
        std::fs::write(ds, format!("{}<instances>{}{}", text.get(..start).unwrap(), instances.concat(), text.get(end..).unwrap())).unwrap();
    }

    /// An instance element: family Test, style Regular, PostScript name Test-Regular, at `wght`.
    fn regular_at(wght: u32, filename: &str) -> String {
        format!(
            "<instance familyname=\"Test\" stylename=\"Regular\" postscriptfontname=\"Test-Regular\"{filename}><location><dimension name=\"Weight\" xvalue=\"{wght}\"/></location></instance>"
        )
    }

    #[test]
    fn instantiate_source_keeps_the_later_of_two_instances_with_one_file() {
        // fontmake writes each instance in turn, so of two with one file the later is left.
        let (dir, ds) = source("one-file");
        let file = " filename=\"instances/Test-Regular.ufo\"";
        set_instances(&ds, &[regular_at(400, file), regular_at(600, file)]);
        let mut s = Session::new();
        let r = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let results = r["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "{r}");
        assert_eq!(results[0]["location"]["wght"], json!(600.0));
        assert_eq!(glyph_a(&dir.join("instances/Test-Regular.ufo")).0, 575.0, "fontmake's width of A at 600");
        let warned = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).any(|w| w.contains("not written") && w.contains("same file"));
        assert!(warned, "{r}");

        // Compiled: no recorded file name, so both are named Test-Regular.
        set_instances(&ds, &[regular_at(400, ""), regular_at(600, "")]);
        let r = s.execute("font.instantiate", &json!({ "input": ds, "to": "ttf", "outputDir": dir.join("ttf"), "timestamp": 0 })).unwrap();
        let results = r["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "{r}");
        assert_eq!(results[0]["location"]["wght"], json!(600.0));
        let font = fontcraft_tables::read(&std::fs::read(results[0]["output"].as_str().unwrap()).unwrap(), 0).unwrap();
        assert_eq!(font.get(*b"OS/2").unwrap()[4..6], 600u16.to_be_bytes(), "the later instance");
        let warned = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).any(|w| w.contains("not kept"));
        assert!(warned, "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Adds `idotless`, `acutecomb` (whose contour starts at `acute_x`) and the composite
    /// `iacute` (the acute placed at 184, -26) to the UFO at `dir`.
    fn add_iacute(dir: &Path, acute_x: i32) {
        let write = |rel: &str, text: String| std::fs::write(dir.join(rel), text).unwrap();
        write(
            "glyphs/contents.plist",
            format!(
                "{PLIST}<dict><key>A</key><string>A_.glif</string><key>B</key><string>B_.glif</string><key>acutecomb</key><string>acutecomb.glif</string><key>iacute</key><string>iacute.glif</string><key>idotless</key><string>idotless.glif</string></dict></plist>\n"
            ),
        );
        write("glyphs/idotless.glif", glif("idotless", "0131", 600, &rect(200, 400)));
        let acute = format!(
            "<contour><point x=\"{acute_x}\" y=\"700\" type=\"line\"/><point x=\"300\" y=\"700\" type=\"line\"/><point x=\"300\" y=\"800\" type=\"line\"/></contour>"
        );
        write("glyphs/acutecomb.glif", glif("acutecomb", "0301", 0, &acute));
        write(
            "glyphs/iacute.glif",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<glyph name=\"iacute\" format=\"2\"><advance width=\"600\"/><unicode hex=\"00ED\"/><outline><component base=\"idotless\"/><component base=\"acutecomb\" xOffset=\"184\" yOffset=\"-26\"/></outline></glyph>\n".into(),
        );
    }

    #[test]
    fn instantiate_source_leaves_components_where_the_file_puts_them() {
        // Rounding the bases (100.5 to 100) must not move the composite's components by the
        // editor's automatic alignment: fontmake keeps them at (184, -26), and the width at 600.
        let (dir, ds) = source("components");
        add_iacute(&dir.join("Light.ufo"), 101);
        add_iacute(&dir.join("Bold.ufo"), 100);
        let mut s = Session::new();
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let (font, _) = crate::io::open(&dir.join("instances/Test-Regular.ufo"), None).unwrap();
        let layer = font.glyph("iacute").unwrap().layers.first().unwrap().clone();
        assert_eq!(layer.width, 600.0);
        let offsets: Vec<(f64, f64)> = layer
            .shapes
            .iter()
            .filter_map(|s| match s {
                fontcraft_model::Shape::Component(c) => {
                    let m = c.transform.affine().as_coeffs();
                    Some((m[4], m[5]))
                }
                _ => None,
            })
            .collect();
        assert_eq!(offsets, [(0.0, 0.0), (184.0, -26.0)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instantiate_source_interpolates_a_number_only_one_source_sets() {
        // Only Bold states a width class (8): fontMath counts the missing one as 0, so fontmake's
        // instance halfway has 4.
        let (dir, ds) = source("width-class");
        let info = dir.join("Bold.ufo/fontinfo.plist");
        let text = std::fs::read_to_string(&info).unwrap();
        std::fs::write(&info, text.replace("<key>unitsPerEm</key>", "<key>openTypeOS2WidthClass</key><integer>8</integer><key>unitsPerEm</key>"))
            .unwrap();
        let mut s = Session::new();
        s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        let out = std::fs::read_to_string(dir.join("instances/Test-Regular.ufo/fontinfo.plist")).unwrap();
        let at = out.find("<key>openTypeOS2WidthClass</key>").unwrap_or_else(|| panic!("{out}"));
        assert!(
            out.get(at..).unwrap().trim_start_matches("<key>openTypeOS2WidthClass</key>").trim_start().starts_with("<integer>4</integer>"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instantiate_source_maps_user_locations_through_the_axis_map() {
        // User 500 is design 600: fontmake's instances at user 600 (design 650) and at design 500
        // (user 433) have A 588 and 550 wide.
        let (dir, ds) = source("axis-map");
        let text = std::fs::read_to_string(&ds).unwrap().replace(
            "<axis tag=\"wght\" name=\"Weight\" minimum=\"300\" default=\"300\" maximum=\"700\"/>",
            "<axis tag=\"wght\" name=\"Weight\" minimum=\"300\" default=\"300\" maximum=\"700\"><map input=\"300\" output=\"300\"/><map input=\"500\" output=\"600\"/><map input=\"700\" output=\"700\"/></axis>",
        );
        std::fs::write(&ds, text).unwrap();
        set_instances(
            &ds,
            &[
                "<instance familyname=\"Test\" stylename=\"Medium\" filename=\"instances/Test-Medium.ufo\"><location><dimension name=\"Weight\" uservalue=\"600\"/></location></instance>".into(),
                "<instance familyname=\"Test\" stylename=\"Regular\" filename=\"instances/Test-Regular.ufo\"><location><dimension name=\"Weight\" xvalue=\"500\"/></location></instance>".into(),
            ],
        );
        let mut s = Session::new();
        let r = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        assert_eq!(r["results"][0]["location"]["wght"], json!(600.0), "{r}");
        assert_eq!(glyph_a(&dir.join("instances/Test-Medium.ufo")).0, 588.0);
        assert_eq!(glyph_a(&dir.join("instances/Test-Regular.ufo")).0, 550.0);
        let info = std::fs::read_to_string(dir.join("instances/Test-Regular.ufo/fontinfo.plist")).unwrap();
        assert!(info.contains("<integer>433</integer>"), "the weight class from the user location: {info}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instantiate_source_applies_the_instance_custom_parameters() {
        // What fontmake's instance UFO takes from the instance's custom parameters and properties
        // (glyphsLib's apply_instance_data_to_ufo).
        let (dir, ds) = source("custom-params");
        add_iacute(&dir.join("Light.ufo"), 100);
        add_iacute(&dir.join("Bold.ufo"), 100);
        let lib = "<lib><dict><key>com.schriftgestaltung.customParameters</key><array>\
            <array><string>typoAscender</string><integer>900</integer></array>\
            <array><string>underlinePosition</string><integer>-150</integer></array>\
            <array><string>vendorID</string><string>ABCD</string></array>\
            <array><string>postscriptFontName</string><string>Test-Custom</string></array>\
            <array><string>weightClass</string><integer>650</integer></array>\
            <array><string>Rename Glyphs</string><array><string>A=acutecomb</string></array></array>\
            <array><string>Reencode Glyphs</string><array><string>idotless=0041</string></array></array>\
            <array><string>Some Unknown Thing</string><string>v</string></array>\
            </array><key>com.schriftgestaltung.properties</key><array><array><string>designers</string><string>Someone</string></array></array></dict></lib>";
        set_instances(&ds, &[regular_at(500, " filename=\"instances/Test-Regular.ufo\"").replace("</location>", &format!("</location>{lib}"))]);
        let mut s = Session::new();
        let r = s.execute("font.instantiate", &json!({ "input": ds, "autohint": false })).unwrap();
        assert_eq!(r["results"][0]["postscriptName"], json!("Test-Custom"), "{r}");
        let out = dir.join("instances/Test-Regular.ufo");
        let info = std::fs::read_to_string(out.join("fontinfo.plist")).unwrap();
        let value = |key: &str| {
            let at = info.find(&format!("<key>{key}</key>")).unwrap_or_else(|| panic!("no {key}: {info}"));
            let rest = info.get(at..).unwrap().split_once("</key>").unwrap().1.trim_start();
            rest.get(..rest.find("</").unwrap()).unwrap().rsplit('>').next().unwrap().to_string()
        };
        assert_eq!(value("openTypeOS2TypoAscender"), "900");
        assert_eq!(value("postscriptUnderlinePosition"), "-150");
        assert_eq!(value("openTypeOS2VendorID"), "ABCD");
        assert_eq!(value("postscriptFontName"), "Test-Custom");
        assert_eq!(value("openTypeOS2WeightClass"), "650", "the parameter wins over the location");
        assert_eq!(value("openTypeNameDesigner"), "Someone");
        let lib = std::fs::read_to_string(out.join("lib.plist")).unwrap();
        assert!(lib.contains("Some Unknown Thing"), "{lib}");
        let (font, _) = crate::io::open(&out, None).unwrap();
        let glyph = |name: &str| font.glyph(name).unwrap().clone();
        // Rename Glyphs swaps the drawings; the code points stay with the names.
        assert_eq!(glyph("acutecomb").layers.first().unwrap().width, 550.0);
        assert_eq!(glyph("A").layers.first().unwrap().width, 0.0);
        // Reencode Glyphs takes U+0041 from A and puts it first on idotless.
        assert!(glyph("A").unicodes.is_empty());
        assert_eq!(glyph("idotless").unicodes, [0x41, 0x131]);
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
