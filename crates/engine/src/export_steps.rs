//! The engine's export stage (`docs/plans/M7.md` § 3 › Export-time parameters): the export-time
//! parameters that need the engine, run on a copy of the font before the compiler resolves it. In
//! order:
//!
//! 1. linked sources: Import Font and Import Master refreshed from their files ([`crate::linked`]),
//!    once per export ([`linked`]);
//! 2. PreFilter strings on the target master's layers, components still in place;
//! 3. Scale to UPM, through `fontcraft_fontinfo::scale`;
//! 4. Filter strings, after every composite of the target master's layers is decomposed.
//!
//! Every job built here is a static font, so PreFilter always runs. Filter and PreFilter strings
//! come from the font, the master and the instance, in that order, and all of them run; at most
//! [`fontcraft_fontinfo::MAX_FILTER_STRINGS`] of both together. Scale to UPM is one value: the
//! instance's, else the master's, else the font's. Each string that names a built-in filter runs
//! through `filter.apply` in a session of its own that holds the copy, so export and the Filter
//! menu share one implementation per filter. Locked glyphs and paths are unlocked in that copy
//! first: a lock guards editing, not export.
//!
//! A string whose name is no built-in filter names an installed filter module, by id or by name
//! (`fontcraft_plugins::export_filter`). The module runs on the target master's layer of every
//! glyph that `include:` and `exclude:` let through, through the same
//! `fontcraft_plugins::export_filter::run_on_layer` as `plugin.runFilter`, and all its layers
//! change in one edit. A name that is neither a built-in filter nor an installed module, or a
//! module that is turned off or not loaded (safe mode), refuses the export with a message naming
//! it: the font would come out without a filter its source asks for. Parameters the module cannot
//! take, and a module that fails on a glyph, are warnings, and that string changes nothing. The
//! session's modules reach this stage through [`ModuleScope`], which `Session::execute` enters
//! for each top-level command.
//!
//! Before Filter decomposes, each composite of the target master is given the anchors it would
//! have inherited from its components, so mark attachment survives the decomposition (not for
//! UFO-family sources, whose propagation follows their own filter list).
//!
//! `Autohinting`, the app function the handbook lists among the filter strings, is not a filter
//! of the Filter menu and changes no outline: a Filter or PreFilter string that names it, at any
//! level, turns autohinting on for an instance's export as the instance's Autohint parameter
//! would, whatever that parameter or the export option says, and so applies to the outlines
//! every other filter string leaves ([`AUTOHINTING`]). A master has no Autohint parameter, so a
//! master's export reports the string as not used.
//!
//! A parameter that cannot be used is reported as a warning and skipped; a Scale to UPM that
//! cannot be applied refuses the export, since the font would come out at the wrong size. Export
//! Folder ([`export_folder`]) names the sub-folder of the destination the job's files go to.
//! The compiler's stage (glyph sets, names, the `kern` table) runs after resolving, in
//! `fontcraft_compile::export_params`.
//!
//! File › Export, `font.compile`, the CLI, the control channel and the oracle harness all build
//! through `cmd::compile::build`, which calls [`linked`] once and [`prepare`] for every job.
//!
//! Rows: `Filter`, `PreFilter`, the `filter.customParameter.*` rows (`.autohinting`: M10.31),
//! `export.instanceExportFolder` (M7.10); `Import Font`, `Import Master` (M7.12); `Scale to UPM`
//! (M7.4 scales, M7.10 runs it); `filter.customParameter.thirdParty`,
//! `filter.menu.pluginFiltersListed` (M13.4, module filters).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use fontcraft_compile::Options;
use fontcraft_geom::{Affine, Point};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::plist::{Num, Number, Value as PValue};
use fontcraft_model::{Anchor, Font, Layer, MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, Params, Resolved, Shape, Target, Transform};
use fontcraft_plugins::export_filter::{self as module_filter, FilterCall};
use fontcraft_plugins::{Kind, Plugin};
use serde_json::{Map, Value};

use crate::cmd::compile::Job;
use crate::extensions::ExtensionState;
use crate::{EngineError, Result};

const FILTER: &str = "Filter";
const PRE_FILTER: &str = "PreFilter";
const SCALE_TO_UPM: &str = "Scale to UPM";
const EXPORT_FOLDER: &str = "Export Folder";
/// The filter string of the app function that runs the autohinter (`filter.customParameter.autohinting`).
const AUTOHINTING: &str = "Autohinting";
/// The instance parameter Autohinting turns on (`fontcraft_hinting::settings::AUTOHINT`).
const AUTOHINT: &str = "Autohint";

/// Most nodes and components decomposition for Filter may write for one job, over all glyphs.
/// Each layer has [`MAX_OUTLINE_ELEMENTS`] of its own; glyphs past this keep their components.
const MAX_DECOMPOSED_ELEMENTS: usize = 20_000_000;

/// Where two design locations count as the same (as the model's resolution).
const SAME_LOCATION: f64 = 1e-9;

/// What the steps may read besides the font.
pub(crate) struct StepContext<'a> {
    /// The document's glyph database (filters that look up glyph data, decomposition).
    pub data: &'a Arc<GlyphData>,
}

/// `font` with its linked sources read from their files, once for a whole export: every job
/// shares the result. `path` is the document's file, which relative paths start from; `None` for
/// a document never saved.
pub(crate) fn linked<'f>(font: &'f Font, path: Option<&Path>, warnings: &mut Vec<String>) -> Result<Cow<'f, Font>> {
    Ok(match crate::linked::resolve(font, path, warnings)? {
        Some(linked) => Cow::Owned(linked),
        None => Cow::Borrowed(font),
    })
}

/// The font `job` is built from: `font` (already [`linked`]) itself when no step applies, else a
/// changed copy. Warnings name the parameters that could not be used.
pub(crate) fn prepare<'f>(font: &'f Font, job: &Job, ctx: &StepContext, warnings: &mut Vec<String>) -> Result<Cow<'f, Font>> {
    let mut font = Cow::Borrowed(font);
    // A target that is not a static instance at a master is left to the compiler to report.
    let Some(master) = target_master(&font, &job.target) else { return Ok(font) };
    let (pre, post, upm, autohinting) = {
        let levels = levels(&font, &master, &job.target);
        let autohinting = autohinting_asked(&levels, warnings);
        let mut pre = filter_strings(&levels, PRE_FILTER, warnings);
        let mut post = filter_strings(&levels, FILTER, warnings);
        let total = pre.len().saturating_add(post.len());
        if total > fontcraft_fontinfo::MAX_FILTER_STRINGS {
            pre.truncate(fontcraft_fontinfo::MAX_FILTER_STRINGS);
            post.truncate(fontcraft_fontinfo::MAX_FILTER_STRINGS.saturating_sub(pre.len()));
            warnings.push(format!(
                "{} has {total} Filter and PreFilter parameters; only the first {} were run",
                job.label,
                fontcraft_fontinfo::MAX_FILTER_STRINGS
            ));
        }
        (usable(pre, warnings)?, usable(post, warnings)?, scale_target(&levels, warnings), autohinting)
    };
    if autohinting {
        font = Cow::Owned(autohint_on(font.into_owned(), job, warnings));
    }

    if !pre.is_empty() {
        font = Cow::Owned(run_filters(font.into_owned(), &master, &pre, warnings)?);
    }
    if let Some(upm) = upm.filter(|u| *u != font.units_per_em) {
        let scaled = fontcraft_fontinfo::scale::scale_font(&font, upm).map_err(|e| EngineError::BadParams {
            command: SCALE_TO_UPM.into(),
            message: format!("{} cannot be scaled to {upm} units per em: {e}", job.label),
        })?;
        font = Cow::Owned(scaled);
    }
    if !post.is_empty() {
        let decomposed = decompose_master(&font, &master, ctx.data, warnings);
        font = Cow::Owned(run_filters(decomposed, &master, &post, warnings)?);
    }
    Ok(font)
}

/// The id of the master whose layers `target` draws: the master itself, or the one a static
/// instance sits at. `None` for anything else.
fn target_master(font: &Font, target: &Target) -> Option<String> {
    match target {
        Target::Master(id) => font.master(id).ok().map(|m| m.id.clone()),
        Target::Instance(i) => {
            let inst = font.instances.get(*i).filter(|inst| inst.kind.is_none())?;
            let at = |m: &&fontcraft_model::Master| {
                let n = font.axes.len().max(m.location.len()).max(inst.location.len());
                (0..n).all(|a| {
                    let (x, y) = (fontcraft_model::axes::design_value(&m.location, a), fontcraft_model::axes::design_value(&inst.location, a));
                    (x - y).abs() <= SAME_LOCATION
                })
            };
            font.masters.iter().find(at).map(|m| m.id.clone())
        }
    }
}

/// The parameter lists of the font, the target master and the instance, in that order.
fn levels<'a>(font: &'a Font, master: &str, target: &Target) -> [Option<&'a Params>; 3] {
    let instance = match target {
        Target::Instance(i) => font.instances.get(*i).map(|inst| &inst.params),
        Target::Master(_) => None,
    };
    [Some(&font.info.params), font.masters.iter().find(|m| m.id == master).map(|m| &m.params), instance]
}

/// Every enabled `name` string of `levels`, in order. A value that is not text is skipped with a
/// warning; an empty one, and an [`AUTOHINTING`] one, silently.
fn filter_strings(levels: &[Option<&Params>; 3], name: &'static str, warnings: &mut Vec<String>) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for params in levels.iter().flatten() {
        for value in params.all(name) {
            match value.as_str() {
                Some(text) if text.trim().is_empty() || autohinting_name(text) => {}
                Some(text) => out.push((name, text.to_string())),
                None => warnings.push(format!("a {name} parameter that is not text was not used")),
            }
        }
    }
    out
}

/// The Scale to UPM value that applies: the instance's, else the master's, else the font's.
fn scale_target(levels: &[Option<&Params>; 3], warnings: &mut Vec<String>) -> Option<u16> {
    let value = levels.iter().rev().flatten().find_map(|p| p.get(SCALE_TO_UPM))?;
    let upm = match value {
        PValue::Number(n) => match n.value() {
            Num::Int(i) => u16::try_from(i).ok(),
            Num::Float(f) if f.is_finite() && f.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(&f) => {
                // In range and whole, checked above.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Some(f as u16)
            }
            Num::Float(_) => None,
        },
        PValue::String(s) => s.as_str().trim().parse::<u16>().ok(),
        _ => None,
    };
    match upm.filter(|u| fontcraft_fontinfo::UPM_RANGE.contains(u)) {
        Some(u) => Some(u),
        None => {
            warnings.push(format!(
                "the {SCALE_TO_UPM} parameter was not used: its value must be a whole number from {} to {}",
                fontcraft_fontinfo::UPM_RANGE.start(),
                fontcraft_fontinfo::UPM_RANGE.end()
            ));
            None
        }
    }
}

// ── Autohinting ─────────────────────────────────────────────────────────────────────────────────

/// Whether the filter string `text` names [`AUTOHINTING`].
fn autohinting_name(text: &str) -> bool {
    text.split(';').next().is_some_and(|name| name.trim() == AUTOHINTING)
}

/// Whether a Filter or PreFilter string of `levels` names [`AUTOHINTING`]. It takes no
/// arguments; any it is given are ignored with a warning.
fn autohinting_asked(levels: &[Option<&Params>; 3], warnings: &mut Vec<String>) -> bool {
    let mut asked = false;
    for params in levels.iter().flatten() {
        for name in [PRE_FILTER, FILTER] {
            for text in params.all(name).filter_map(|v| v.as_str()).filter(|t| autohinting_name(t)) {
                asked = true;
                if text.split(';').skip(1).any(|item| !item.trim().is_empty()) {
                    warnings.push(format!("the {name} parameter \"{text}\" takes no arguments; they were ignored"));
                }
            }
        }
    }
    asked
}

/// `font` with the Autohint parameter of `job`'s instance on. A master's export has no such
/// parameter: the request is reported and `font` returned as it was.
fn autohint_on(mut font: Font, job: &Job, warnings: &mut Vec<String>) -> Font {
    let Target::Instance(i) = &job.target else {
        warnings.push(format!(
            "the {AUTOHINTING} filter string was not used for {}: it turns on the Autohint parameter of an instance, and a master has none; turn on Autohint in the export options instead",
            job.label
        ));
        return font;
    };
    if let Some(instance) = Arc::make_mut(&mut font.instances).get_mut(*i) {
        instance.params.set(AUTOHINT, PValue::Number(Number::int(1)));
    }
    font
}

// ── running filter strings ──────────────────────────────────────────────────────────────────────

/// One Filter or PreFilter string to run.
enum FilterStep {
    /// A built-in filter, run through `filter.apply`.
    BuiltIn { param: &'static str, text: String },
    /// An installed filter module.
    Module(ModuleStep),
}

/// A filter module named by a filter string, with its parameters resolved.
struct ModuleStep {
    param: &'static str,
    text: String,
    plugin: Arc<Plugin>,
    call: FilterCall,
    params: Map<String, Value>,
}

/// The strings of `strings` that can run, in order. A string naming a built-in filter that cannot
/// be read, or one without an export string form (Extrude), is skipped with a warning, and so is
/// a module string with parameters the module cannot take. A string whose name is neither a
/// built-in filter nor an installed filter module that may run is an error.
fn usable(strings: Vec<(&'static str, String)>, warnings: &mut Vec<String>) -> Result<Vec<FilterStep>> {
    let modules = ModuleScope::current();
    let mut out = Vec::with_capacity(strings.len());
    for (param, text) in strings {
        let builtin_error = match crate::cmd::filter::parse_filter_string(&text) {
            Ok(_) => {
                out.push(FilterStep::BuiltIn { param, text });
                continue;
            }
            Err(e) => e,
        };
        let call = match module_filter::parse_filter_string(&text) {
            Ok(call) if !is_builtin(&call.name) => call,
            // Unreadable, or a built-in filter's string with a problem: the built-in reading
            // says what is wrong.
            _ => {
                warnings.push(format!("the {param} parameter \"{text}\" was not used: {builtin_error}"));
                continue;
            }
        };
        let plugin = module_for(param, &text, &call.name, modules.as_deref())?;
        match call.params_for(plugin.manifest()) {
            Ok(params) => out.push(FilterStep::Module(ModuleStep { param, text, plugin, call, params })),
            Err(e) => warnings.push(format!("the {param} parameter \"{text}\" was not used: {e}")),
        }
    }
    Ok(out)
}

/// Whether `name` is a built-in filter, with an export string form or without one, or the
/// [`AUTOHINTING`] app function.
fn is_builtin(name: &str) -> bool {
    autohinting_name(name) || crate::cmd::filter::parse_filter_string(name).is_ok() || fontcraft_pathops::filters::find(name).is_some()
}

/// The installed filter module `name` names (its id, else its name exactly, else its name with
/// ASCII case ignored), when it may run. Every other case refuses the export with a message
/// naming it.
fn module_for(param: &'static str, text: &str, name: &str, modules: Option<&ModuleSet>) -> Result<Arc<Plugin>> {
    let refuse = |message: String| EngineError::BadParams { command: param.into(), message: format!("the {param} parameter \"{text}\" {message}") };
    let filters = modules.map(|m| m.filters.as_slice()).unwrap_or_default();
    let found = match filters.iter().find(|m| m.plugin.id() == name) {
        Some(m) => Some(m),
        None => {
            let exact: Vec<&ModuleEntry> = filters.iter().filter(|m| m.plugin.manifest().name == name).collect();
            let named =
                if exact.is_empty() { filters.iter().filter(|m| m.plugin.manifest().name.eq_ignore_ascii_case(name)).collect() } else { exact };
            match named.as_slice() {
                [] => None,
                [one] => Some(*one),
                several => {
                    let ids: Vec<&str> = several.iter().map(|m| m.plugin.id()).collect();
                    return Err(refuse(format!("names `{name}`, which several filter modules are called: give the id of one ({})", ids.join(", "))));
                }
            }
        }
    };
    let Some(module) = found else {
        return Err(refuse(format!("names `{name}`, which is neither a built-in filter nor an installed filter module")));
    };
    match &module.runnable {
        Ok(()) => Ok(Arc::clone(&module.plugin)),
        Err(e) => Err(refuse(format!("names a filter module that cannot run: {e}"))),
    }
}

/// `font` with each of `steps` run on the master layers of `master`, in order. A string that
/// cannot be run is skipped with a warning.
fn run_filters(mut font: Font, master: &str, steps: &[FilterStep], warnings: &mut Vec<String>) -> Result<Font> {
    unlock(&mut font, master);
    let mut s = crate::Session::new();
    s.push_document(font, None);
    let layers = [master.to_string()];
    for step in steps {
        match step {
            FilterStep::BuiltIn { param, text } => match crate::cmd::filter::run_filter_string(&mut s, text, &layers) {
                Ok(result) => left_as_they_were(&result, param, text, warnings),
                Err(e) => warnings.push(format!("the {param} parameter \"{text}\" was not used: {e}")),
            },
            FilterStep::Module(m) => run_module(&mut s, master, m, warnings)?,
        }
    }
    Ok(s.font()?.clone())
}

/// Runs the module of `step` on the `master` layer of every glyph its scope lets through, and
/// applies every new layer in one edit. A module that fails on any glyph changes nothing, with a
/// warning naming the glyph.
fn run_module(s: &mut crate::Session, master: &str, step: &ModuleStep, warnings: &mut Vec<String>) -> Result<()> {
    let mut changed: Vec<(String, Layer)> = Vec::new();
    {
        let font = s.font()?;
        for glyph in font.glyphs.iter() {
            if !step.call.applies_to(&glyph.name) {
                continue;
            }
            let Some(layer) = glyph.layer(master) else { continue };
            match module_filter::run_on_layer(&step.plugin, font, glyph, layer, &step.params) {
                Ok((new, _log)) => changed.push((glyph.name.clone(), new)),
                Err(e) => {
                    warnings.push(format!("the {} parameter \"{}\" was not used: on `{}`, {e}", step.param, step.text, glyph.name));
                    return Ok(());
                }
            }
        }
    }
    if changed.is_empty() {
        return Ok(());
    }
    s.edit(&step.plugin.manifest().name, |font| {
        for (name, new) in changed {
            let mut glyph = font.glyph_mut(&name)?;
            if let Some(layer) = glyph.layer_mut(master) {
                *layer = new;
            }
        }
        Ok(())
    })
}

// ── the session's modules ───────────────────────────────────────────────────────────────────────

/// One installed filter module as the export filter step sees it.
struct ModuleEntry {
    plugin: Arc<Plugin>,
    /// What [`ExtensionState::runnable`] said when the scope was entered: why it cannot run
    /// (turned off, safe mode), if it cannot.
    runnable: std::result::Result<(), String>,
}

/// The filter modules of the session whose command runs on this thread.
struct ModuleSet {
    filters: Vec<ModuleEntry>,
}

thread_local! {
    /// The modules in force on this thread ([`ModuleScope`]).
    static MODULES: RefCell<Option<Rc<ModuleSet>>> = const { RefCell::new(None) };
}

/// Makes a session's filter modules visible to the export filter step while one top-level
/// command runs, and puts back what it replaced when dropped (also when the command panics).
/// Export builds run deep inside commands that have no other way to the session's
/// `ExtensionState`; the sessions the steps open for built-in filters enter their own, empty,
/// scope and restore this one.
pub(crate) struct ModuleScope(Option<Rc<ModuleSet>>);

impl ModuleScope {
    /// Enters the filter modules of `ext`; none when it has none.
    pub(crate) fn enter(ext: &ExtensionState) -> ModuleScope {
        let filters: Vec<ModuleEntry> = ext
            .registry
            .list()
            .iter()
            .filter(|m| m.plugin.kind() == Kind::Filter)
            .map(|m| ModuleEntry { plugin: Arc::clone(&m.plugin), runnable: ext.runnable(m.plugin.id(), Kind::Filter).map(|_| ()) })
            .collect();
        let set = (!filters.is_empty()).then(|| Rc::new(ModuleSet { filters }));
        ModuleScope(MODULES.with(|m| m.replace(set)))
    }

    /// The modules in force on this thread.
    fn current() -> Option<Rc<ModuleSet>> {
        MODULES.with(|m| m.borrow().clone())
    }
}

impl Drop for ModuleScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        MODULES.with(|m| *m.borrow_mut() = previous);
    }
}

/// Warns about the glyphs a filter run left as they were for a reason other than having no
/// layer for the master.
fn left_as_they_were(result: &Value, param: &str, text: &str, warnings: &mut Vec<String>) {
    let Some(skipped) = result.get("skipped").and_then(Value::as_array) else { return };
    let failed: Vec<(&str, &str)> = skipped
        .iter()
        .filter_map(|v| Some((v.get("glyph")?.as_str()?, v.get("reason").and_then(Value::as_str).unwrap_or(""))))
        .filter(|(_, reason)| !reason.contains("no such layer"))
        .collect();
    let Some((first, reason)) = failed.first() else { return };
    let more = failed.len().saturating_sub(1);
    let others = if more == 0 { String::new() } else { format!(" and {more} more glyphs") };
    warnings.push(format!("the {param} parameter \"{text}\" left `{first}`{others} as they were: {reason}"));
}

/// Unlocks every glyph, and the locked paths of the master layers of `master`.
fn unlock(font: &mut Font, master: &str) {
    let locked: Vec<usize> = font
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, g)| g.info.locked || g.layer(master).is_some_and(|l| l.shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.info.locked))))
        .map(|(i, _)| i)
        .collect();
    for i in locked {
        let Some(mut g) = font.glyphs.get_mut_at(i) else { continue };
        if g.info.locked {
            g.info.locked = false;
        }
        if let Some(layer) = g.layer_mut(master) {
            for shape in &mut layer.shapes {
                if let Shape::Path(p) = shape
                    && p.info.locked
                {
                    p.info.locked = false;
                }
            }
        }
    }
}

// ── decomposition before Filter ─────────────────────────────────────────────────────────────────

/// `font` as Filter strings see it: corner and smart components drawn, then every component of
/// the master layers of `master` replaced by the outlines it stands for, each composite keeping
/// the anchors it inherited. A glyph whose components cannot be resolved keeps them, with a
/// warning.
fn decompose_master(font: &Font, master: &str, data: &GlyphData, warnings: &mut Vec<String>) -> Font {
    let inherited = inherited_anchors(font, master, data, warnings);
    let mut out = fontcraft_compile::prepare_source(font, warnings).into_owned();
    let mut budget = MAX_DECOMPOSED_ELEMENTS;
    let mut changes: Vec<(usize, Vec<Shape>)> = Vec::new();
    let mut kept: Vec<(String, String)> = Vec::new();
    for (i, g) in out.glyphs.iter().enumerate() {
        let Some(layer) = g.layer(master) else { continue };
        if !layer.shapes.iter().any(|s| matches!(s, Shape::Component(_))) {
            continue;
        }
        if budget == 0 {
            kept.push((g.name.clone(), format!("the decomposed composites hold more than {MAX_DECOMPOSED_ELEMENTS} nodes in all")));
            continue;
        }
        let start = MAX_OUTLINE_ELEMENTS.min(budget);
        let mut layer_budget = start;
        let mut shapes = Vec::with_capacity(layer.shapes.len());
        let result = decompose(&out, &layer.shapes, master, Affine::IDENTITY, 0, &mut layer_budget, &mut shapes);
        budget = budget.saturating_sub(start.saturating_sub(layer_budget));
        match result {
            Ok(()) => changes.push((i, shapes)),
            Err(why) => kept.push((g.name.clone(), why)),
        }
    }
    if let Some((first, why)) = kept.first() {
        let more = kept.len().saturating_sub(1);
        let others = if more == 0 { String::new() } else { format!(" and {more} more glyphs") };
        warnings.push(format!("`{first}`{others} kept their components for the Filter parameters: {why}"));
    }
    for (i, shapes) in changes {
        let Some(mut g) = out.glyphs.get_mut_at(i) else { continue };
        let anchors = inherited.as_ref().and_then(|map| map.get(&g.name)).cloned();
        if let Some(layer) = g.layer_mut(master) {
            layer.shapes = shapes;
            if let Some(anchors) = anchors {
                layer.anchors = anchors;
            }
        }
    }
    out
}

/// The anchors each glyph of `master` has once its components pass theirs on, by glyph name;
/// `None` for a UFO-family source or when they cannot be worked out (with a warning).
fn inherited_anchors(font: &Font, master: &str, data: &GlyphData, warnings: &mut Vec<String>) -> Option<HashMap<String, Vec<Anchor>>> {
    if fontcraft_model::is_ufo_family(font.source.format) {
        return None;
    }
    match fontcraft_components::propagate::effective_anchors_for_master_with(font, Some(data), master) {
        Ok(m) => Some(m.anchors.into_iter().map(|(name, list)| (name, list.into_iter().map(|a| Anchor::new(a.name, a.x, a.y)).collect())).collect()),
        Err(e) => {
            warnings.push(format!("composites decomposed for the Filter parameters keep only their own anchors: {e}"));
            None
        }
    }
}

/// Adds `shapes` to `out` with every component replaced by its base glyph's shapes in `master`,
/// moved by `affine` (a path under a mirroring transform is reversed, as export decomposition
/// does). A component whose base is missing stays a component. Each node and each component takes
/// one from `budget`.
fn decompose(
    font: &Font,
    shapes: &[Shape],
    master: &str,
    affine: Affine,
    depth: usize,
    budget: &mut usize,
    out: &mut Vec<Shape>,
) -> std::result::Result<(), String> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(format!("components are nested more than {MAX_COMPONENT_DEPTH} deep, or in a cycle"));
    }
    let too_big = || format!("the outline would have more than {MAX_OUTLINE_ELEMENTS} nodes");
    for shape in shapes {
        match shape {
            Shape::Path(path) => {
                *budget = budget.checked_sub(path.nodes.len()).ok_or_else(too_big)?;
                let mut path = path.clone();
                if depth > 0 {
                    for n in &mut path.nodes {
                        let at = affine * Point::new(n.x, n.y);
                        n.x = at.x;
                        n.y = at.y;
                    }
                    if affine.determinant() < 0.0 {
                        path.reverse();
                    }
                }
                out.push(Shape::Path(path));
            }
            Shape::Component(c) => {
                *budget = budget.checked_sub(1).ok_or_else(too_big)?;
                match font.glyph(&c.glyph).and_then(|g| g.layer(master)) {
                    Some(base) => decompose(font, &base.shapes, master, affine * c.affine(), depth + 1, budget, out)?,
                    None => {
                        let mut c = c.clone();
                        if depth > 0 {
                            c.transform = Transform::Matrix((affine * c.affine()).as_coeffs());
                        }
                        out.push(Shape::Component(c));
                    }
                }
            }
            // A base glyph's images are not part of the composite's outline.
            other if depth == 0 => out.push(other.clone()),
            _ => {}
        }
    }
    Ok(())
}

// ── Export Folder ───────────────────────────────────────────────────────────────────────────────

/// The sub-folder of the destination the files of `resolved` go to (its Export Folder
/// parameter), relative, or `None` for the destination itself. An absolute path, more than
/// [`fontcraft_fontinfo::MAX_EXPORT_FOLDER_SEGMENTS`] segments or
/// [`fontcraft_fontinfo::MAX_EXPORT_FOLDER_BYTES`] bytes is refused.
pub(crate) fn export_folder(resolved: &Resolved, options: &Options) -> Result<Option<PathBuf>> {
    let _ = options;
    let Some(value) = resolved.params.get(EXPORT_FOLDER) else { return Ok(None) };
    let refused = |message: String| EngineError::BadParams { command: EXPORT_FOLDER.into(), message };
    let text = value.as_str().ok_or_else(|| refused("the value must be text".into()))?;
    folder_path(text).map_err(refused)
}

/// Reads an Export Folder value: `/`-separated folder names, `..` climbing out of the
/// destination, `.` and one trailing `/` ignored.
fn folder_path(text: &str) -> std::result::Result<Option<PathBuf>, String> {
    if text.len() > fontcraft_fontinfo::MAX_EXPORT_FOLDER_BYTES {
        return Err(format!("the value is longer than {} bytes", fontcraft_fontinfo::MAX_EXPORT_FOLDER_BYTES));
    }
    if text.trim().is_empty() {
        return Ok(None);
    }
    if text.starts_with('/') || text.starts_with('\\') || Path::new(text).is_absolute() {
        return Err(format!("\"{text}\" is an absolute path; give a folder relative to the export destination"));
    }
    let text = text.strip_suffix('/').unwrap_or(text);
    let segments: Vec<&str> = text.split('/').collect();
    if segments.len() > fontcraft_fontinfo::MAX_EXPORT_FOLDER_SEGMENTS {
        return Err(format!("\"{text}\" has more than {} folder levels", fontcraft_fontinfo::MAX_EXPORT_FOLDER_SEGMENTS));
    }
    let mut out = PathBuf::new();
    for segment in segments {
        if segment.is_empty() {
            return Err(format!("\"{text}\" has an empty folder name between two slashes"));
        }
        if let Some(c) = segment.chars().find(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
            return Err(format!("the folder name \"{segment}\" holds {c:?}, which folder names cannot hold"));
        }
        if segment != "." {
            out.push(segment);
        }
    }
    Ok(if out.as_os_str().is_empty() { None } else { Some(out) })
}

/// `name` (a `/`-separated path relative to the destination) inside `folder`.
pub(crate) fn in_folder(folder: &Path, name: &str) -> String {
    let mut out: Vec<String> = folder.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    out.push(name.to_string());
    out.join("/")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// `export.instanceExportFolder`: nested folders, `..` kept, absolute paths refused.
    #[test]
    fn export_folder_values_are_read_as_relative_paths() {
        let path = |t: &str| folder_path(t).unwrap().map(|p| in_folder(&p, "F.ttf"));
        assert_eq!(path("Trial Versions/Webfonts"), Some("Trial Versions/Webfonts/F.ttf".into()));
        assert_eq!(path("../Trial Versions/"), Some("../Trial Versions/F.ttf".into()));
        assert_eq!(path("./web"), Some("web/F.ttf".into()));
        assert_eq!(path(""), None);
        assert_eq!(path("."), None);
        for bad in ["/tmp/x", "\\x", "a//b", "a:b", "C:/fonts"] {
            assert!(folder_path(bad).is_err(), "{bad} is refused");
        }
        assert!(folder_path(&"a/".repeat(40)).is_err(), "too many levels");
        assert!(folder_path(&"a".repeat(2_000)).is_err(), "too long");
    }
}
