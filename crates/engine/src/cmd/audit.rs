//! The audit commands (M3.17): run the outline checks of `fontcraft-audit` on the open document,
//! list and describe them, return the last run's results, and fix what they report
//! (`docs/spec/21-qa.md` §§ 3–5).
//!
//! A run examines every layer of the selected glyphs except backup layers and returns the result
//! document of § 4. The last run of each document is kept for `audit.results` and for
//! `audit.fix {results}`; a glyph edited since then makes its results `stale`.
//!
//! `audit.fix` works in the stages of `docs/spec/21-qa.md` § 5 "Fix calls". First the Font Info
//! fixes, each run once through its command (`audit_fix_info`), for findings of any scope. Then
//! each layer: every fix with the engine command its check names, in the order of
//! `audit.outlineFixOrder`, re-checking the layer (or glyph) before every step so each command
//! gets the current addresses. Findings of compiled instances are fixed on the master layers of
//! their source glyph, from the selection's findings; the calls of the steps M3 did not have are
//! built in `audit_fix_glyph`. The unfixed layer is first copied to a backup layer of its master
//! (decision D12), and the whole call is one undo step.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_audit::profile::{self as prof, DocumentSettings};
use fontcraft_audit::{self as audit, AuditContext, CHECKS, CheckScope, CheckSpec, Config, CurveType, Finding, FixSpec, Profile, Severity};
use fontcraft_model::{Font, Glyph, Layer, Path, Shape};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::Params;
use crate::{Batch, DocState, EngineError};

const SCHEMA: &str = "fontcraft.audit/1";
/// Most entries in a `checks`, `exclude`, `groups` or `scope` list.
const MAX_PATTERNS: usize = 1024;
/// Longest check id or pattern, in bytes.
const MAX_PATTERN_LEN: usize = 256;
/// Most ids in a `masters` or `layers` list.
const MAX_IDS: usize = 1024;
/// Most result ids one `audit.fix` call takes.
const MAX_RESULT_IDS: usize = 100_000;
/// Most thresholds one `config` object may set.
const MAX_CONFIG_ENTRIES: usize = 1024;
/// Largest `maxResults`.
const MAX_MAX_RESULTS: i64 = 1_000_000;
/// Most results one document lists; the rest are counted in `summary.truncated`.
const MAX_REPORTED: usize = 100_000;
const GROUPS: [&str; 18] = [
    "outlines",
    "components",
    "anchors",
    "glyphset",
    "metrics",
    "names",
    "tables",
    "layout",
    "variations",
    "hinting",
    "kerning",
    "family",
    "files",
    "sources",
    "color",
    "vendor",
    "cid",
    crate::extensions::MODULE_CHECK_GROUP,
];
const SCOPES: [&str; 3] = ["source", "binary", "family"];
/// `audit.outlineFixOrder`, the layer steps `audit.fix` builds calls for, in the order of § 5
/// "Fix calls" (steps 7 to 33): emptied glyphs and component repairs first, then the outline
/// clean-up, overlap removal and clean-up again for what the union left, then direction,
/// extremes, curve type, node types and handles, metric lines, start points, compatibility and
/// rounding, and last anchors and widths.
const FIX_ORDER: [&str; 28] = [
    "clearGlyph",
    "flattenComponents",
    "removeDuplicateComponents",
    "decomposeComponents",
    "realignComponents",
    "closePaths",
    "removeDuplicateContours",
    "eraseOpenCorners",
    "tidyPaths",
    "removeOverlap",
    "deleteTinyContours",
    "tidyPaths",
    "correctPathDirection",
    "addExtremes",
    "addInflections",
    "convertCurveType",
    "setSmooth",
    "alignHandles",
    "harmonize",
    "snapHandles",
    "snapToMetric",
    "setStartPoint",
    "fixCompatibility",
    "roundCoordinates",
    "removeDuplicateAnchors",
    "addAnchors",
    "updateMetrics",
    "zeroMarkWidths",
];
/// The layer steps that are not outline steps (§ 5 "Fix calls" steps 7 to 11 and 30 to 33):
/// `safeOnly` does not skip them on a layer with components.
const NOT_OUTLINE_STEPS: [&str; 9] = [
    "clearGlyph",
    "flattenComponents",
    "removeDuplicateComponents",
    "decomposeComponents",
    "realignComponents",
    "removeDuplicateAnchors",
    "addAnchors",
    "updateMetrics",
    "zeroMarkWidths",
];
/// Why a layer's outline steps are left out under `safeOnly`.
const HAS_COMPONENTS: &str = "the layer has components: its outline fixes need safeOnly: false";
/// Why a fix with no call `audit.fix` can make is left out.
const NO_FIX_CALL: &str = "audit.fix cannot carry this fix out yet";
const NO_FILE_FIX_CALL: &str = "audit.fix cannot carry this fix out on a compiled font: fix the source and export again";
/// Added to the largest deviation the almost-straight check reported, so the reported curves fall
/// strictly inside Tidy up Paths' `straighten` limit.
const STRAIGHTEN_MARGIN: f64 = 1e-6;
const FIX_LABEL: &str = "Fix Audit Results";
const NO_RUN: &str = "no audit has run on this document since it was opened: run audit.run first";

mod cache;
mod font_steps;
mod staged;

pub(crate) use staged::Slot;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = run_specs();
    v.extend(staged::specs());
    v
}

fn run_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "audit.run",
            "Run Audit",
            [],
            None,
            "{target?: \"document\" | path | [paths] (default the open document; several files or a folder of compiled fonts are also one family), profile?: id (default the document's default profile, else the application's, else universal), checks?: [id | prefix*], exclude?: [id | prefix*], groups?: [group], scope?: [\"source\" | \"binary\" | \"family\"], glyphs?: [names, indexes, first-last ranges, /CID] (entries that match nothing are warned about), glyphFile?: path (the same entries in a file), masters?: [ids], layers?: [ids], minSeverity?: \"info\" | \"warn\" | \"fail\", config?: {checkId: {threshold: number}}, maxResults?: count = 100, includePassed?: bool = false, includeIgnored?: bool = false, network?: bool = false, all?: bool = false} runs the checks of every scope and changes nothing: layer checks on every layer of the selected glyphs except backup layers (or the layers in `layers`), glyph and source checks on the source, binary checks on each compiled font (a source's exporting instances and variable font are compiled in memory and their findings name the source field behind a compiled value), family checks on two or more fonts. Turned-on check modules (none in safe mode) run on the same layers under any profile with layer checks; their results have the check id check.plugin.<module>.<name>, group plugin and no fix, and a module that fails is listed once in checks with status error and a message. On a compiled font the outline checks read the decoded file and checks that need source data are skipped. A check whose preconditions do not hold is skipped with the reason. A pattern that matches no check is an error. A capped check with more than maxResults results in one font gives one info result with the count instead. Results on the source's ignore list are counted in summary.ignored and listed only with includeIgnored. A run on the open document is kept for audit.results and audit.fix; the next run on it reuses the layer-check results of each glyph that has not changed since, when the glyphs its components reach, the rest of the font, the layer checks and their settings have not changed either (all: true examines every glyph again). A result's fix gives the audit.fix call that carries it out: {results: [id]} on the open document, {checks, glyphs, layers} on a source file, {target, checks} on one compiled font file. Every run has a new runId, which audit.results, audit.fix and audit.export take to make sure the kept run is still that one → {schema, tool, runId, target: {kind, path, fonts: [{id, name, path, instance, format}]}, document?: {title, revision}, profile, config, summary: {fail, warn, info, passed, skipped, ignored, errors, truncated}, checks: [{id, status, results, reason?, message?}], results: [{id, check, code, severity, message, location: {font, glyph, master, layer, shapes?, points?, segments?, point?, bounds?, table?, field?, sourceField?, instance?, fonts?, masters?}, data, fix: {available, id?, safe?, description?, command?, params?, via?}, ignored, stale}], warnings?: [text]}",
            always,
            run
        )
        .files(const { &[FileParam::read("target").keywords(&["document"]), FileParam::read("glyphFile")] }),
        cmd!(
            query "audit.list",
            "List Checks",
            [],
            None,
            "{profile?: id, groups?: [group], scope?: [\"source\" | \"binary\" | \"family\"], fixable?: bool} the registered checks, in registry order → {checks: [{id, label, group, severity, scope, profile, fixable, fix?, fixAvailable, description}]}",
            always,
            list
        ),
        cmd!(
            query "audit.describe",
            "Describe Check",
            [],
            None,
            "{check: id} everything known about one check → {id, label, description, group, family, severity, scope, profile, profiles: [ids], thresholds: [{name, default, min, max, doc}], codes: [message code], fix: {id, command, available, safe, description} | null, preconditions: [text], capped}",
            always,
            describe
        ),
        cmd!(
            query "audit.results",
            "Get Audit Results",
            [],
            None,
            "{checks?: [id | prefix*], exclude?: [id | prefix*], glyphs?: [names], minSeverity?: \"info\" | \"warn\" | \"fail\", fixable?: bool, includeIgnored?: bool, runId?: id (refused unless the kept run is that one), target?: \"document\"} the result document of the last audit.run on the open document, with its runId, without running anything, narrowed to the matching results. A result whose glyph changed since the run has `stale: true`. An error when no run is kept (none since the document was opened) → the result document of audit.run, with summary.stale",
            has_doc,
            results
        ),
        cmd!(
            "audit.fix",
            "Fix Issues",
            [],
            None,
            "{results?: [result id] (from the last audit.run; not with the other filters), runId?: id (refused unless the document still keeps that run), profile?, checks?: [id | prefix*], exclude?, groups?, glyphs?: [names], masters?: [ids], layers?: [ids], minSeverity?, config?, safeOnly?: bool = true, dryRun?: bool = false, keepOriginal?: \"backupLayer\" | \"none\" = \"backupLayer\", maxResults?: count = 100, target?: \"document\" | path, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} repairs what the selected results report. On a compiled font file (target a path) it runs the checks on the file and then, on a copy, the table repairs in this order, each call reading the previous one's output: tables.delete, tables.recompileCff, tables.recompileLayout, tables.rebuildCmap, tables.cleanNameRecords, tables.cleanInstances, tables.dropAxis (one call per axis), variations.buildHvar, outlines.removeOverlaps (the reported glyphs) and tables.recalculate (the reported table fields); the copy replaces output (default FONT.fixed.EXT, or the input with inPlace) only when every call succeeded, and fixes that edit a source are skipped → {output, applied: [{fix, command, params, checks, result}], skipped, planned? (dryRun), remaining: the result document of the same checks on the output}. On the files of a family (target a list of paths; outputDir or inPlace, not output) each file gets its own table repairs, and syncFamilyValues gives the fonts a result compares the widest vertical metrics and the compared font's underline values through tables.recalculate values; every output replaces its file only when every call on every file succeeded → {outputs, applied, skipped, planned?, remaining}. On the open document: First each selected Font Info fix runs once, whatever the scope of its findings: font.info.syncVerticalMetrics, font.info.syncVersion, font.instance.syncStyleLinking, font.info.syncFamilyValues, font.info.sortZones, font.info.clearGeneratedFields, font.info.cleanNames and font.instance.cleanUp, in that order (with filters, their checks run on the source and its compiled instances). Then each selected layer (a result of a compiled instance on every master layer of its source glyph), by running edit.delete (empty glyphs), component.flatten, edit.delete (repeated components), component.decompose, component.enableAutomaticAlignment, path.closeOpenPaths, edit.delete (duplicate contours), path.eraseOpenCorners, path.tidyUpPaths, path.removeOverlap, edit.delete (tiny contours), path.correctPathDirection, path.addExtremes, path.addInflections, path.convertToCubic or path.convertToQuadratic, path.toggleNodeType, selection.setPosition (kinked smooth nodes), path.harmonizeNodes, selection.setPosition (almost orthogonal handles), selection.move (points near a metric line), path.makeNodeFirst (start points, then start points that differ between masters), path.roundCoordinates, anchor.remove and anchor.add (repeated anchor names), glyph.setAnchors, spacing.setWidth and glyph.updateMetrics (widths from keys or the compared glyph) and spacing.setWidth (mark widths), in that order, on what the checks report then. The unfixed layer is first copied to a backup layer of its master unless keepOriginal is \"none\". All of it is one undo step. safeOnly skips fixes that change the design and the outline fixes of layers with components. A locked glyph, a stale result, a fix audit.fix cannot carry out yet and a step that fails are skipped and left as they were → {applied: [{fix, glyph?, layer?, master?, command, params, checks?, changed?}], backups: [{glyph, layer, backup}], skipped: [{glyph?, layer?, check?, fix?, reason}], planned?: [{fix, glyph?, layer?, master?, command, params, results?, checks?, changed?}] (dryRun), remaining: the result document of the selected checks on the glyphs involved, and of the Font Info fixes' checks on the whole font}",
            always,
            fix
        )
        .files(const { &[FileParam::read("target").keywords(&["document"]), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
        cmd!(
            query "audit.profile.list",
            "List Audit Profiles",
            [],
            None,
            "{} the built-in profiles, then the open document's and the user's, each with the number of registered checks it runs (null when it cannot be resolved) and its store; a user profile hidden by a project profile with the same id is shadowed; default marks the profile audit.run uses when the call names none → {profiles: [{id, label, checks, builtIn, default, store: \"builtIn\" | \"project\" | \"user\", description?, shadowed?}], problems?: [text]}",
            always,
            profile_list
        ),
    ]
}

// ── Reading a selection ─────────────────────────────────────────────────────────────────────────

/// What one run examines, with which settings.
#[derive(Clone)]
struct Run {
    profile: String,
    /// The resolved profile, for its severity overrides.
    resolved: prof::Resolved,
    checks: Vec<&'static CheckSpec>,
    glyphs: Option<HashSet<String>>,
    masters: Option<HashSet<String>>,
    layers: Option<HashSet<String>>,
    config: Config,
    config_json: Value,
    min_severity: Severity,
    max_results: usize,
    include_passed: bool,
    include_ignored: bool,
    network: bool,
    /// Glyph selection entries that matched nothing.
    warnings: Vec<String>,
    /// The check modules that run on the same layers as the layer checks.
    modules: crate::extensions::ModuleChecks,
    /// The layer reports of the document's kept run, reused where nothing they read changed.
    cache: Option<Arc<cache::LayerCache>>,
    /// Whether the run records layer reports for the next one: only a run on the open document.
    keeps_cache: bool,
}

/// Only the open document can be audited so far.
fn document_target(p: &Params) -> Result<()> {
    match p.get("target") {
        None => Ok(()),
        Some(Value::String(s)) if s == "document" => Ok(()),
        Some(_) => Err(EngineError::Unsupported("auditing a file or a family of files; audit the open document".into())),
    }
}

/// A list of check ids and `prefix*` patterns. Each must match a registered check.
fn patterns<'a>(p: &Params<'a>, key: &str) -> Result<Vec<&'a str>> {
    let Some(list) = p.opt_strings(key)? else { return Ok(Vec::new()) };
    if list.len() > MAX_PATTERNS {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {MAX_PATTERNS} are allowed", list.len())));
    }
    for pat in &list {
        if pat.is_empty() || pat.len() > MAX_PATTERN_LEN {
            return Err(p.bad(format!("`{key}`: a check id or pattern has 1 to {MAX_PATTERN_LEN} bytes")));
        }
        if !CHECKS.iter().any(|c| audit::matches_pattern(c.id, pat)) && !crate::extensions::names_module_checks(pat) {
            return Err(p.bad(format!("`{key}`: no check matches `{pat}` (a pattern is an id, or a prefix followed by `*`)")));
        }
    }
    Ok(list)
}

/// A list of names, each one of `allowed`.
fn names_from<'a>(p: &Params<'a>, key: &str, allowed: &[&str]) -> Result<Option<Vec<&'a str>>> {
    let Some(list) = p.opt_strings(key)? else { return Ok(None) };
    if list.len() > MAX_PATTERNS {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {MAX_PATTERNS} are allowed", list.len())));
    }
    if let Some(bad) = list.iter().find(|n| !allowed.contains(*n)) {
        return Err(p.bad(format!("`{key}`: `{bad}` is not one of {}", allowed.join(", "))));
    }
    Ok(Some(list))
}

/// The check group of an id: `check.<group>.<name>`.
fn group_of(id: &str) -> &str {
    id.strip_prefix("check.").and_then(|rest| rest.split('.').next()).unwrap_or("")
}

fn profile_param(p: &Params) -> Result<Option<Profile>> {
    match p.opt_str("profile")? {
        None => Ok(None),
        Some(id) => Profile::parse(id).map(Some).ok_or_else(|| p.bad(format!("there is no profile `{id}`: outlines, opentype, universal or strict"))),
    }
}

fn severity_param(p: &Params) -> Result<Severity> {
    match p.opt_str("minSeverity")? {
        None => Ok(Severity::Info),
        Some(s) => Severity::parse(s).ok_or_else(|| p.bad("`minSeverity` must be \"info\", \"warn\" or \"fail\"")),
    }
}

/// Ids that must each exist: masters of the font, or layers of some glyph.
fn id_set(p: &Params, key: &str, exists: impl Fn(&str) -> bool) -> Result<Option<HashSet<String>>> {
    if let Some(Value::Array(a)) = p.get(key)
        && a.len() > MAX_IDS
    {
        return Err(p.bad(format!("`{key}` holds {} ids; at most {MAX_IDS} are allowed", a.len())));
    }
    let Some(list) = p.opt_strings(key)? else { return Ok(None) };
    if let Some(missing) = list.iter().find(|id| !exists(id)) {
        return Err(p.bad(format!("`{key}`: there is no `{missing}`")));
    }
    Ok(Some(list.into_iter().map(str::to_string).collect()))
}

fn config_param(p: &Params) -> Result<(Config, Value)> {
    let mut config = Config::default();
    let Some(map) = p.opt_object("config")? else { return Ok((config, json!({}))) };
    let mut count = 0usize;
    for (check, values) in map {
        let Value::Object(values) = values else {
            return Err(p.bad(format!("`config.{check}` must be an object of threshold values")));
        };
        for (name, value) in values {
            count += 1;
            if count > MAX_CONFIG_ENTRIES {
                return Err(p.bad(format!("`config` sets more than {MAX_CONFIG_ENTRIES} thresholds")));
            }
            let v = value.as_f64().ok_or_else(|| p.bad(format!("`config.{check}.{name}` must be a number")))?;
            config.set(check, name, v).map_err(|e| p.bad(e))?;
        }
    }
    Ok((config, Value::Object(map.clone())))
}

/// The profile a run uses: the one named, else the document's default, else the application's,
/// else `universal`; resolved through the document's and the user's profiles.
fn resolve_profile(p: &Params, s: &Session, settings: &DocumentSettings) -> Result<(String, prof::Resolved, Option<String>)> {
    let (user, _) = super::audit_profiles::user_store(s);
    let (id, note) = match p.opt_str("profile")? {
        Some(id) => (id.to_string(), None),
        None => {
            let (id, _, note) = super::audit_profiles::default_profile(settings, &user);
            (id, note)
        }
    };
    let profiles = prof::Profiles { project: &settings.profiles, user: &user.profiles };
    let resolved = profiles.resolve(&id).map_err(|e| p.bad(e))?;
    Ok((id, resolved, note))
}

/// Reads the selection keys shared by `audit.run` and `audit.fix`. `settings` are the target
/// source's audit settings (empty for compiled fonts): its profiles, default profile and
/// threshold overrides.
fn read_run(p: &Params, s: &Session, font: &Font, settings: &DocumentSettings) -> Result<Run> {
    let (profile, resolved, note) = resolve_profile(p, s, settings)?;
    let include = patterns(p, "checks")?;
    let exclude = patterns(p, "exclude")?;
    let groups = names_from(p, "groups", &GROUPS)?;
    let scopes = names_from(p, "scope", &SCOPES)?;
    let checks = resolved
        .checks
        .iter()
        .copied()
        .filter(|c| include.is_empty() || include.iter().any(|pat| audit::matches_pattern(c.id, pat)))
        .filter(|c| !exclude.iter().any(|pat| audit::matches_pattern(c.id, pat)))
        .filter(|c| groups.as_ref().is_none_or(|g| g.contains(&group_of(c.id))))
        .filter(|c| scopes.as_ref().is_none_or(|s| s.contains(&c.scope.row_scope())))
        .collect();
    // Check modules are layer checks of the source scope: they run under any profile that runs
    // layer checks.
    let modules = if resolved.checks.iter().any(|c| c.scope == CheckScope::Layer)
        && groups.as_ref().is_none_or(|g| g.contains(&crate::extensions::MODULE_CHECK_GROUP))
        && scopes.as_ref().is_none_or(|s| s.contains(&CheckScope::Layer.row_scope()))
    {
        crate::extensions::ModuleChecks::select(s.extensions(), &include, &exclude)
    } else {
        crate::extensions::ModuleChecks::default()
    };
    let mut warnings: Vec<String> = note.into_iter().collect();
    let glyphs = glyph_param(p, font, &mut warnings)?;
    let masters = id_set(p, "masters", |id| font.master(id).is_ok())?;
    let layers = id_set(p, "layers", |id| font.glyphs.as_arcs().iter().any(|g| g.layers.iter().any(|l| l.id == id)))?;
    let (mut config, config_json) = config_param(p)?;
    // Profile thresholds, then the document's overrides, then the call's: the last set wins.
    let mut merged = resolved.config();
    for (check, values) in &settings.thresholds {
        for (name, v) in values {
            let _ = merged.set(check, name, *v);
        }
    }
    for c in CHECKS {
        for t in c.thresholds {
            if let Some(v) = config.get(c.id, t.name) {
                let _ = merged.set(c.id, t.name, v);
            }
        }
    }
    config = merged;
    let max =
        p.opt_int("maxResults", 0, MAX_MAX_RESULTS)?.map_or(audit::DEFAULT_MAX_RESULTS, |n| usize::try_from(n).unwrap_or(audit::DEFAULT_MAX_RESULTS));
    Ok(Run {
        profile,
        resolved,
        checks,
        glyphs,
        masters,
        layers,
        config,
        config_json,
        min_severity: severity_param(p)?,
        max_results: max,
        include_passed: p.bool_or("includePassed", false)?,
        include_ignored: p.bool_or("includeIgnored", false)?,
        network: p.bool_or("network", false)?,
        warnings,
        modules,
        cache: None,
        keeps_cache: false,
    })
}

/// `glyphs` (a list of selection entries) and `glyphFile` (a path to a file of them), read with
/// the glyph selection syntax; entries that match nothing become warnings.
fn glyph_param(p: &Params, font: &Font, warnings: &mut Vec<String>) -> Result<Option<HashSet<String>>> {
    let mut entries: Vec<String> = Vec::new();
    match p.get("glyphs") {
        None => {}
        Some(Value::String(s)) => entries.push(s.clone()),
        Some(Value::Array(a)) if a.len() > fontcraft_model::MAX_GLYPHS => {
            return Err(p.bad(format!("`glyphs` holds {} entries; at most {} are allowed", a.len(), fontcraft_model::MAX_GLYPHS)));
        }
        Some(_) => entries.extend(p.strings("glyphs")?.into_iter().map(str::to_string)),
    }
    if let Some(path) = p.opt_str("glyphFile")? {
        entries.push(read_glyph_file(std::path::Path::new(path))?);
    }
    if p.get("glyphs").is_none() && p.get("glyphFile").is_none() {
        return Ok(None);
    }
    let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
    let sel = audit::run::select_glyphs(font, &refs);
    warnings.extend(sel.warnings);
    Ok(Some(sel.glyphs.into_iter().collect()))
}

/// Largest glyph list file read, in bytes.
#[cfg(not(target_arch = "wasm32"))]
const MAX_GLYPH_FILE: u64 = 16 << 20;

#[cfg(not(target_arch = "wasm32"))]
fn read_glyph_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_GLYPH_FILE + 1).read_to_string(&mut text))
        .map_err(|e| EngineError::Io(format!("cannot read the glyph list {}: {e}", path.display())))?;
    if text.len() as u64 > MAX_GLYPH_FILE {
        return Err(EngineError::Io(format!("the glyph list {} is larger than {MAX_GLYPH_FILE} bytes", path.display())));
    }
    Ok(text)
}

#[cfg(target_arch = "wasm32")]
fn read_glyph_file(_: &std::path::Path) -> Result<String> {
    Err(EngineError::Unsupported("reading a glyph list file in the browser".into()))
}

// ── Running the checks ──────────────────────────────────────────────────────────────────────────

fn is_backup(l: &Layer) -> bool {
    !l.is_master_layer() && l.info.attrs.is_empty()
}

/// One finding with where it sorts: font, glyph and layer positions.
struct Hit {
    finding: Finding,
    /// The font's index in the target's font list.
    font: usize,
    glyph: usize,
    layer: usize,
    master: String,
    /// On the target source's ignore list.
    ignored: bool,
}

/// What one run found.
#[derive(Default)]
struct Examined {
    hits: Vec<Hit>,
    tally: audit::run::Tally,
    modules: crate::extensions::ModuleRun,
    /// The layer reports of the target's first font, for the next run.
    cache: cache::LayerCache,
}

/// The layers a run examines, in font order.
fn layers_of<'f>(font: &'f Font, run: &Run) -> Vec<(usize, &'f Arc<Glyph>, usize, &'f Layer)> {
    let mut out = Vec::new();
    for (gi, g) in font.glyphs.as_arcs().iter().enumerate() {
        if run.glyphs.as_ref().is_some_and(|set| !set.contains(&g.name)) {
            continue;
        }
        for (li, l) in g.layers.iter().enumerate() {
            let chosen = match &run.layers {
                Some(set) => set.contains(&l.id),
                None => !is_backup(l),
            };
            if chosen && run.masters.as_ref().is_none_or(|set| set.contains(&l.master)) {
                out.push((gi, g, li, l));
            }
        }
    }
    out
}

/// Where a font's findings sort: glyph index by name, and glyph, layer and master by
/// `(glyph, layer)`.
#[derive(Default)]
struct Places {
    glyphs: HashMap<String, usize>,
    layers: HashMap<(String, String), (usize, String)>,
}

/// Findings before the result cap, ignore list and sorting, with the index of their font.
type Raw = Vec<(usize, Finding)>;

/// Runs the layer, glyph and source checks of `run` on `font`, the font at `fi` in the target.
fn examine_font(font: &Font, fi: usize, run: &Run, facts: &audit::run::Facts, ex: &mut Examined, raw: &mut Raw, places: &mut HashMap<usize, Places>) {
    let layer_checks = start_font(font, fi, run, facts, ex, places);
    let chosen = layers_of(font, run);
    examine_layers(&AuditContext::new(font), fi, run, &layer_checks, &chosen, ex, raw, places);
    examine_rest(font, fi, run, facts, ex, raw);
}

/// Before a font's layers: records where its glyphs sort and returns the layer checks whose
/// preconditions hold, counting the others as skipped.
fn start_font(
    font: &Font,
    fi: usize,
    run: &Run,
    facts: &audit::run::Facts,
    ex: &mut Examined,
    places: &mut HashMap<usize, Places>,
) -> Vec<&'static CheckSpec> {
    let mut layer_checks = Vec::new();
    for &c in run.checks.iter().filter(|c| c.scope == CheckScope::Layer) {
        match audit::run::unmet(c, facts, None) {
            Some(reason) => ex.tally.skip(c.id, reason),
            None => layer_checks.push(c),
        }
    }
    let place = places.entry(fi).or_default();
    place.glyphs = font.glyphs.as_arcs().iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
    layer_checks
}

/// Runs the layer checks and the check modules on `chosen`, a stretch of [`layers_of`] of the
/// font of `ctx`.
#[allow(clippy::too_many_arguments)]
fn examine_layers(
    ctx: &AuditContext<'_>,
    fi: usize,
    run: &Run,
    layer_checks: &[&'static CheckSpec],
    chosen: &[(usize, &Arc<Glyph>, usize, &Layer)],
    ex: &mut Examined,
    raw: &mut Raw,
    places: &mut HashMap<usize, Places>,
) {
    let font = ctx.font;
    let place = places.entry(fi).or_default();
    if !layer_checks.is_empty() {
        // Only the open document (the first and only font of its target) keeps a cache.
        let keeps = run.keeps_cache && fi == 0;
        let cached = run.cache.as_deref().filter(|c| keeps && c.key_matches(ctx, run, layer_checks));
        if keeps {
            ex.cache.start(ctx, run, layer_checks);
        }
        for (_, g, li, layer) in chosen {
            let kept = cached.and_then(|c| c.reports(ctx, g)).and_then(|r| r.iter().find(|(id, _)| *id == layer.id)).map(|(_, r)| r.clone());
            let report = kept.unwrap_or_else(|| audit::audit_layer(ctx, g, layer, layer_checks, &run.config));
            if keeps {
                ex.cache.record(font, g, &layer.id, &report);
            }
            place.layers.insert((g.name.clone(), layer.id.clone()), (*li, layer.master.clone()));
            ex.tally.absorb(layer_checks, report);
        }
    }
    if !run.modules.is_empty() {
        for (_, g, li, layer) in chosen {
            place.layers.insert((g.name.clone(), layer.id.clone()), (*li, layer.master.clone()));
            run.modules.run_layer(font, g, layer, &mut ex.modules, &mut ex.tally.findings);
        }
        ex.tally.truncated = ex.tally.truncated.saturating_add(std::mem::take(&mut ex.modules.truncated));
    }
    raw.extend(ex.tally.findings.drain(..).map(|f| (fi, f)));
}

/// Runs the glyph and source checks of `run` on `font`.
fn examine_rest(font: &Font, fi: usize, run: &Run, facts: &audit::run::Facts, ex: &mut Examined, raw: &mut Raw) {
    let ctx = AuditContext::new(font);
    let services = super::audit_services::EngineServices;
    audit::run::run_glyphs(&ctx, &services, &run.checks, &run.config, facts, run.glyphs.as_ref(), &mut ex.tally);
    audit::run::run_source(&ctx, &services, &run.checks, &run.config, facts, &mut ex.tally);
    raw.extend(ex.tally.findings.drain(..).map(|f| (fi, f)));
}

/// Runs the binary checks on each compiled font and the family checks on all of them, with the
/// target source's project values.
fn examine_binaries(
    bins: &[Bin],
    ids: &[String],
    values: &BTreeMap<String, prof::ProjectValue>,
    run: &Run,
    facts: &audit::run::Facts,
    ex: &mut Examined,
    raw: &mut Raw,
) {
    let services = super::audit_services::EngineServices;
    let mut ctxs = Vec::with_capacity(bins.len());
    let mut at = Vec::with_capacity(bins.len());
    for b in bins {
        let id = ids.get(b.font).map_or("", String::as_str);
        match audit::BinaryCtx::new(id, &b.bytes, b.face, &services) {
            Ok(ctx) => {
                let mut ctx = ctx.with_values(values);
                ctx.source = b.map.as_ref();
                if let Some(path) = &b.path {
                    ctx = ctx.with_file(path.clone(), b.size);
                }
                ctxs.push(ctx);
                at.push(b.font);
            }
            Err(e) => {
                ex.tally.notes.push(format!("{id}: {e}"));
                for c in run.checks.iter().filter(|c| c.scope == CheckScope::Binary) {
                    ex.tally.skip(c.id, e.clone());
                }
            }
        }
    }
    for (ctx, &fi) in ctxs.iter().zip(&at) {
        audit::run::run_binary(ctx, &run.checks, &run.config, facts, &mut ex.tally);
        raw.extend(ex.tally.findings.drain(..).map(|f| (fi, f)));
    }
    audit::run::run_family(&ctxs, &services, &run.checks, &run.config, facts, &mut ex.tally);
    raw.extend(ex.tally.findings.drain(..).map(|f| (usize::MAX, f)));
}

/// Applies the result cap (per font, with `cap`), the profile's severity overrides,
/// `minSeverity`, the master filter and the ignore list, then sorts the results: by severity,
/// group, check id, font, glyph order and layer order.
fn finish(raw: Raw, run: &Run, places: &HashMap<usize, Places>, ignore: &prof::IgnoreIndex<'_>, cap: bool, ex: &mut Examined) {
    let mut per_font: BTreeMap<usize, Vec<Finding>> = BTreeMap::new();
    for (fi, f) in raw {
        per_font.entry(fi).or_default().push(f);
    }
    let mut hits = Vec::new();
    for (fi, findings) in per_font {
        let findings = if cap { audit::collapse(findings, run.max_results) } else { findings };
        let place = places.get(&fi);
        for mut f in findings {
            if f.code != audit::TOO_MANY_RESULTS
                && run.resolved.overrides.iter().any(|o| o.check == f.check)
                && let Some(spec) = audit::find(f.check)
            {
                f.severity = run.resolved.severity(spec, f.code);
            }
            if f.severity > run.min_severity {
                continue;
            }
            if let Some(set) = &run.masters
                && !f.location.masters.is_empty()
                && !f.location.masters.iter().all(|m| set.contains(m))
            {
                continue;
            }
            let glyph = place.and_then(|p| p.glyphs.get(&f.location.glyph)).copied().unwrap_or(0);
            let (layer, master) = place
                .and_then(|p| p.layers.get(&(f.location.glyph.clone(), f.location.layer.clone())))
                .cloned()
                .unwrap_or_else(|| (0, f.location.master.clone().unwrap_or_default()));
            let ignored = is_ignored(ignore, &f, &master);
            hits.push(Hit { finding: f, font: fi, glyph, layer, master, ignored });
        }
    }
    hits.sort_by(|a, b| {
        (a.finding.severity, group_of(a.finding.check), a.finding.check, a.font, a.glyph, a.layer).cmp(&(
            b.finding.severity,
            group_of(b.finding.check),
            b.finding.check,
            b.font,
            b.glyph,
            b.layer,
        ))
    });
    if cap && hits.len() > MAX_REPORTED {
        ex.tally.truncated = ex.tally.truncated.saturating_add(hits.len() - MAX_REPORTED);
        hits.truncate(MAX_REPORTED);
    }
    ex.hits = hits;
}

/// Is `f` on the ignore list? An entry matches the check, the glyph (none for a result without
/// one), the master and the location keys it holds.
fn is_ignored(ignore: &prof::IgnoreIndex<'_>, f: &Finding, master: &str) -> bool {
    let Ok(Value::Object(loc)) = serde_json::to_value(&f.location) else { return false };
    let location = super::audit_profiles::result_location(&loc);
    let glyph = (!f.location.glyph.is_empty()).then_some(f.location.glyph.as_str());
    let master = (!master.is_empty()).then_some(master);
    ignore.is_ignored(f.check, glyph, master, &location)
}

/// Runs the layer checks of `run` on the open document only: what `audit.fix` repairs and
/// re-checks.
fn examine(font: &Font, run: &Run, cap: bool) -> Examined {
    let mut ex = Examined::default();
    let (mut raw, mut places) = (Raw::new(), HashMap::new());
    let facts = audit::run::Facts { source: true, variable: font.masters.len() > 1, network: run.network, ..Default::default() };
    let layer_run = Run {
        checks: run.checks.iter().copied().filter(|c| c.scope == CheckScope::Layer).collect(),
        modules: crate::extensions::ModuleChecks::default(),
        ..run.clone()
    };
    examine_font(font, 0, &layer_run, &facts, &mut ex, &mut raw, &mut places);
    let none = prof::IgnoreIndex::new(&[]);
    finish(raw, run, &places, &none, cap, &mut ex);
    ex
}

// ── Targets ─────────────────────────────────────────────────────────────────────────────────────

/// A compiled font of the target: a file, a face of a collection, or an instance compiled in
/// memory from a source.
struct Bin {
    /// Index in the target's font list.
    font: usize,
    /// The sfnt (after any WOFF unwrapping), or the whole collection; the faces of one
    /// collection share it.
    bytes: Arc<[u8]>,
    face: u32,
    path: Option<String>,
    size: u64,
    map: Option<audit::SourceMap>,
}

/// A font the layer, glyph and source checks read: a source, or a compiled font decoded.
struct Src {
    font: usize,
    model: Arc<Font>,
    /// A source (true) or a decoded compiled font.
    source: bool,
}

/// What a run examines.
#[derive(Default)]
struct Target {
    /// `source`, `binary` or `family`.
    kind: &'static str,
    path: Option<String>,
    /// One entry per font: `{id, name, path, instance, format}`.
    fonts: Vec<Value>,
    srcs: Vec<Src>,
    bins: Vec<Bin>,
    /// The first source's audit settings: profiles, thresholds, project values, ignore list.
    settings: DocumentSettings,
    /// The open document's title and revision, for a document target.
    document: Option<Value>,
    notes: Vec<String>,
    /// Bytes of the compiled font files read so far, as read and unwrapped.
    #[cfg(not(target_arch = "wasm32"))]
    bytes_read: u64,
    /// Glyphs decoded from compiled fonts so far.
    #[cfg(not(target_arch = "wasm32"))]
    glyphs_decoded: usize,
}

impl Target {
    fn add_font(&mut self, name: &str, path: Option<&str>, instance: Option<&str>, format: &str) -> usize {
        let i = self.fonts.len();
        self.fonts.push(json!({"id": format!("f{i}"), "name": name, "path": path, "instance": instance, "format": format}));
        i
    }

    fn ids(&self) -> Vec<String> {
        (0..self.fonts.len()).map(|i| format!("f{i}")).collect()
    }
}

/// The open document as a target.
fn document_target_of(doc: &DocState) -> Target {
    let mut t = Target { kind: "source", path: doc.path.as_ref().map(|p| p.to_string_lossy().into_owned()), ..Target::default() };
    let i = t.add_font(&doc.font.family_name, t.path.clone().as_deref(), None, "source");
    t.srcs.push(Src { font: i, model: doc.font.clone(), source: true });
    t.settings = DocumentSettings::read(&doc.font).0;
    t.document = Some(json!({"title": doc.title, "revision": doc.revision}));
    t
}

/// The paths of `target`: one text or a list.
fn target_paths(p: &Params) -> Result<Option<Vec<String>>> {
    match p.get("target") {
        None => Ok(None),
        Some(Value::String(s)) if s == "document" => Ok(None),
        Some(Value::String(s)) => Ok(Some(vec![s.clone()])),
        Some(Value::Array(a)) if a.is_empty() || a.len() > audit::run::MAX_TARGET_FONTS => {
            Err(p.bad(format!("`target` lists 1 to {} paths", audit::run::MAX_TARGET_FONTS)))
        }
        Some(Value::Array(_)) => Ok(Some(p.strings("target")?.into_iter().map(str::to_string).collect())),
        Some(_) => Err(p.bad("`target` is \"document\", a path or a list of paths")),
    }
}

/// Reads the files of a path target. A folder that is not a source package contributes the
/// compiled fonts directly inside it.
#[cfg(not(target_arch = "wasm32"))]
fn load_paths(paths: &[String]) -> Result<Target> {
    use std::path::Path;
    let mut t = Target::default();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for raw in paths {
        let path = Path::new(raw);
        let package = ["metainfo.plist", "fontinfo.plist", "font.props"].iter().any(|f| path.join(f).is_file());
        if path.is_dir() && !package {
            let mut entries: Vec<std::path::PathBuf> = Vec::new();
            for e in std::fs::read_dir(path).map_err(|e| EngineError::Io(format!("cannot read the folder {raw}: {e}")))? {
                let Ok(e) = e else { continue };
                if entries.len() >= MAX_FOLDER_ENTRIES {
                    return Err(EngineError::Io(format!("the folder {raw} holds more than {MAX_FOLDER_ENTRIES} entries")));
                }
                entries.push(e.path());
            }
            entries.sort();
            let before = files.len();
            let mut skipped = 0usize;
            for e in entries {
                // The entry itself, links followed, must lie in the read root: the check on the
                // folder does not cover a link inside it. One that does not is skipped with a note,
                // so the rest of the folder is still audited.
                if let Err(err) = crate::file_access::check_read(&e) {
                    skipped += 1;
                    if skipped <= MAX_SKIP_NOTES {
                        t.notes.push(format!("skipped {}: {err}", e.display()));
                    }
                    continue;
                }
                if std::fs::metadata(&e).is_ok_and(|m| m.is_file()) && starts_as_font(&e) {
                    files.push(e);
                }
                if files.len() > audit::run::MAX_TARGET_FONTS {
                    return Err(EngineError::Io(format!("{raw} holds more than {} fonts", audit::run::MAX_TARGET_FONTS)));
                }
            }
            if skipped > MAX_SKIP_NOTES {
                t.notes.push(format!("skipped {} more entries of {raw} outside the read root", skipped - MAX_SKIP_NOTES));
            }
            if files.len() == before {
                t.notes.push(format!("the folder {raw} holds no compiled fonts"));
            }
        } else {
            files.push(path.to_path_buf());
        }
    }
    if files.is_empty() {
        return Err(EngineError::Io("the target holds no fonts or sources".into()));
    }
    for path in &files {
        load_one(path, &mut t)?;
        if t.fonts.len() > audit::run::MAX_TARGET_FONTS {
            return Err(EngineError::Io(format!("the target holds more than {} fonts", audit::run::MAX_TARGET_FONTS)));
        }
    }
    let first_source = t.srcs.iter().find(|s| s.source).map(|s| s.model.clone());
    if let Some(font) = first_source {
        t.settings = DocumentSettings::read(&font).0;
    }
    t.path = paths.first().filter(|_| paths.len() == 1).cloned();
    t.kind = match (t.fonts.len(), t.srcs.iter().any(|s| s.source)) {
        (1, true) => "source",
        (1, false) => "binary",
        _ => "family",
    };
    Ok(t)
}

/// Whether the file at `path` starts with the signature of a compiled font; only its first
/// bytes are read.
#[cfg(not(target_arch = "wasm32"))]
fn starts_as_font(path: &std::path::Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && fontcraft_container::sniff(&head).is_some()
}

#[cfg(target_arch = "wasm32")]
fn load_paths(_: &[String]) -> Result<Target> {
    Err(EngineError::Unsupported("auditing files by path in the browser; audit the open document".into()))
}

/// Adds one file of a path target: each face of a compiled font, or a source.
#[cfg(not(target_arch = "wasm32"))]
fn load_one(path: &std::path::Path, t: &mut Target) -> Result<()> {
    let text = path.to_string_lossy().into_owned();
    if let Ok(meta) = std::fs::metadata(path)
        && meta.is_file()
    {
        charge_bytes(t, meta.len(), &text)?;
        let input = super::file_io::read_input(path)?;
        if input.kind.is_font() {
            let size = input.bytes.len() as u64;
            // A WOFF or WOFF2 states the size of the font it wraps; one that would take the run
            // past its byte limit is refused before any table is inflated.
            if let Some(declared) = declared_sfnt_size(&input.bytes) {
                over_limit(t.bytes_read.saturating_add(declared), &text)?;
            }
            let unwrapped = match fontcraft_container::to_sfnt(&input.bytes).map_err(|e| EngineError::Io(format!("cannot read {text}: {e}")))? {
                std::borrow::Cow::Borrowed(_) => None,
                std::borrow::Cow::Owned(sfnt) => Some(sfnt),
            };
            let sfnt: Arc<[u8]> = match unwrapped {
                Some(sfnt) => {
                    charge_bytes(t, sfnt.len() as u64, &text)?;
                    sfnt.into()
                }
                None => input.bytes.into(),
            };
            let faces = audit::export::face_count(&sfnt).map_err(|e| EngineError::Io(format!("cannot read {text}: {e}")))?;
            let faces = faces.min(u32::try_from(audit::run::MAX_TARGET_FONTS).unwrap_or(u32::MAX));
            for face in 0..faces {
                let glyphs = face_glyph_count(&sfnt, face);
                t.glyphs_decoded = t.glyphs_decoded.saturating_add(glyphs);
                if t.glyphs_decoded > MAX_RUN_GLYPHS {
                    return Err(EngineError::Io(format!(
                        "{text}: the target's compiled fonts hold more than {MAX_RUN_GLYPHS} glyphs; audit fewer fonts at a time"
                    )));
                }
                let decoded = fontcraft_decompile::decompile(&sfnt, Some(face));
                let (model, warnings) = match decoded {
                    Ok(d) => (Some(d.font), d.warnings),
                    Err(e) => (None, vec![format!("its outlines cannot be decoded, so the outline checks skip it: {e}")]),
                };
                let name = model.as_ref().map_or_else(|| file_stem(path), |m| m.family_name.clone());
                let i = t.add_font(&name, Some(&text), None, input.kind.name());
                t.notes.extend(warnings.into_iter().take(MAX_NOTES_PER_FONT).map(|w| format!("f{i}: {w}")));
                if let Some(model) = model {
                    t.srcs.push(Src { font: i, model: Arc::new(model), source: false });
                }
                t.bins.push(Bin { font: i, bytes: Arc::clone(&sfnt), face, path: Some(text.clone()), size, map: None });
            }
            return Ok(());
        }
    }
    let (font, _) = crate::io::open(path, None)?;
    let i = t.add_font(&font.family_name, Some(&text), None, "source");
    t.srcs.push(Src { font: i, model: Arc::new(font), source: true });
    Ok(())
}

/// Most decoding warnings kept per font.
#[cfg(not(target_arch = "wasm32"))]
const MAX_NOTES_PER_FONT: usize = 5;
/// Most notes on folder entries skipped because they lie outside the read root.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SKIP_NOTES: usize = 64;
/// Most entries of a folder target looked at.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FOLDER_ENTRIES: usize = 4096;
/// Most bytes the files of one path target hold in memory, compiled fonts counted as read and
/// again unwrapped from WOFF.
#[cfg(not(target_arch = "wasm32"))]
const MAX_RUN_BYTES: u64 = 1 << 30;
/// Most glyphs decoded from the compiled fonts of one path target.
#[cfg(not(target_arch = "wasm32"))]
const MAX_RUN_GLYPHS: usize = 1 << 19;

/// Counts `len` more bytes read for `t`, refused past [`MAX_RUN_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
fn charge_bytes(t: &mut Target, len: u64, text: &str) -> Result<()> {
    t.bytes_read = t.bytes_read.saturating_add(len);
    over_limit(t.bytes_read, text)
}

/// Refuses a run that would hold `total` bytes, past [`MAX_RUN_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
fn over_limit(total: u64, text: &str) -> Result<()> {
    if total > MAX_RUN_BYTES {
        return Err(EngineError::Io(format!("{text}: the target's files hold more than {MAX_RUN_BYTES} bytes; audit fewer fonts at a time")));
    }
    Ok(())
}

/// The size of the font a WOFF or WOFF2 file says it wraps (`totalSfntSize`, at the same offset
/// in both headers); `None` for other files.
#[cfg(not(target_arch = "wasm32"))]
fn declared_sfnt_size(bytes: &[u8]) -> Option<u64> {
    use fontcraft_container::Kind;
    if !matches!(fontcraft_container::sniff(bytes), Some(Kind::Woff | Kind::Woff2)) {
        return None;
    }
    let field: [u8; 4] = bytes.get(16..20)?.try_into().ok()?;
    Some(u64::from(u32::from_be_bytes(field)))
}

/// The glyph count of face `face` of `sfnt` (0 when it cannot be read).
#[cfg(not(target_arch = "wasm32"))]
fn face_glyph_count(sfnt: &[u8], face: u32) -> usize {
    use skrifa::raw::TableProvider as _;
    skrifa::FontRef::from_index(sfnt, face).ok().and_then(|f| f.maxp().ok()).map_or(0, |m| usize::from(m.num_glyphs()))
}

#[cfg(not(target_arch = "wasm32"))]
fn file_stem(path: &std::path::Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Compiles each source of the target in memory for the binary and family checks.
fn compile_sources(t: &mut Target, run: &Run, ex: &mut Examined) {
    let wanted = audit::run::any_of(&run.checks, CheckScope::Binary) || audit::run::any_of(&run.checks, CheckScope::Family);
    if !wanted {
        return;
    }
    let sources: Vec<Arc<Font>> = t.srcs.iter().filter(|s| s.source).map(|s| s.model.clone()).collect();
    for font in sources {
        let (instances, notes) = super::audit_services::EngineServices.compile_all(&font);
        t.notes.extend(notes);
        if instances.is_empty() {
            let reason = "no instance of the source could be compiled".to_string();
            for c in run.checks.iter().filter(|c| c.scope == CheckScope::Binary) {
                ex.tally.skip(c.id, reason.clone());
            }
        }
        for inst in instances {
            if t.fonts.len() >= audit::run::MAX_TARGET_FONTS {
                t.notes.push(format!("the run reads at most {} fonts; further instances were not compiled", audit::run::MAX_TARGET_FONTS));
                return;
            }
            let instance = inst.map.instance.clone();
            let i = t.add_font(&inst.name, None, Some(instance.as_deref().unwrap_or(&inst.name)), inst.format);
            let size = inst.bytes.len() as u64;
            t.bins.push(Bin { font: i, bytes: inst.bytes.into(), face: 0, path: None, size, map: Some(inst.map) });
        }
    }
}

/// Runs every check of `run` on `t`.
fn examine_target(t: &mut Target, run: &Run) -> Examined {
    let mut ex = Examined::default();
    compile_sources(t, run, &mut ex);
    let (mut raw, mut places) = (Raw::new(), HashMap::new());
    let values = t.settings.values.keys().cloned().collect();
    let facts = |source: bool, font: &Font| audit::run::Facts {
        source,
        variable: font.masters.len() > 1,
        network: run.network,
        values: BTreeSet::clone(&values),
    };
    for src in &t.srcs {
        examine_font(&src.model, src.font, run, &facts(src.source, &src.model), &mut ex, &mut raw, &mut places);
    }
    let base = audit::run::Facts { source: false, variable: false, network: run.network, values: values.clone() };
    examine_binaries(&t.bins, &t.ids(), &t.settings.values, run, &base, &mut ex, &mut raw);
    let ignore = t.settings.ignore_index();
    finish(raw, run, &places, &ignore, true, &mut ex);
    t.notes.append(&mut ex.tally.notes);
    ex
}

// ── The result document ─────────────────────────────────────────────────────────────────────────

/// How a result names itself to `audit.fix`.
#[derive(Clone, Copy, PartialEq)]
enum FixParams<'a> {
    /// `{results: [id]}`: the document is kept for `audit.fix`.
    ById,
    /// `{checks, glyphs, layers}`: the document is not kept.
    BySelection,
    /// `{target, checks}`: one compiled font file, repaired by the table commands.
    File(&'a str),
}

fn fix_json(f: &Finding, id: &str, how: FixParams) -> Value {
    let Some(fx) = f.fix else { return json!({"available": false}) };
    let mut m = Map::new();
    let via = match how {
        FixParams::File(_) => super::audit_fix_binary::file_fix_command(fx.id),
        _ => fix_via(&fx),
    };
    m.insert("available".into(), json!(via.is_some()));
    m.insert("id".into(), json!(fx.id));
    m.insert("safe".into(), json!(fx.safe));
    m.insert("description".into(), json!(fx.description));
    match via {
        Some(via) => {
            let params = match how {
                FixParams::ById => json!({"results": [id]}),
                FixParams::File(path) => json!({"target": path, "checks": [f.check]}),
                FixParams::BySelection if f.location.glyph.is_empty() => json!({"checks": [f.check]}),
                FixParams::BySelection => json!({"checks": [f.check], "glyphs": [f.location.glyph], "layers": [f.location.layer]}),
            };
            m.insert("command".into(), json!("audit.fix"));
            m.insert("params".into(), params);
            m.insert("via".into(), json!(via));
        }
        None if matches!(how, FixParams::File(_)) => {
            m.insert("reason".into(), json!(NO_FILE_FIX_CALL));
        }
        None => {
            m.insert("reason".into(), json!(NO_FIX_CALL));
        }
    }
    Value::Object(m)
}

fn result_json(hit: &Hit, id: &str, ids: &[String], how: FixParams) -> Result<Value> {
    let f = &hit.finding;
    let mut location = serde_json::to_value(&f.location).map_err(|e| EngineError::Internal(e.to_string()))?;
    if let Some(o) = location.as_object_mut()
        && let Some(font) = ids.get(hit.font)
    {
        o.insert("font".into(), json!(font));
        if !hit.master.is_empty() {
            o.insert("master".into(), json!(hit.master));
        }
    }
    let data = serde_json::to_value(&f.data).map_err(|e| EngineError::Internal(e.to_string()))?;
    Ok(json!({
        "id": id,
        "check": f.check,
        "code": f.code,
        "severity": f.severity.as_str(),
        "message": f.message,
        "location": location,
        "data": data,
        "fix": fix_json(f, id, how),
        "ignored": hit.ignored,
        "stale": false,
    }))
}

/// The id a result gets: its place in the sorted list.
fn result_id(i: usize) -> String {
    format!("r{}", i.saturating_add(1))
}

/// Builds the result document of `docs/spec/21-qa.md` § 4. Ignored results are counted in
/// `summary.ignored`, never in the other counts or a check's status, and listed only with
/// `includeIgnored` (or `all_results`, for the kept run).
fn document(t: &Target, run: &Run, ex: &Examined, how: FixParams, all_results: bool) -> Result<Value> {
    let ids = t.ids();
    let mut results = Vec::with_capacity(ex.hits.len());
    let mut per_check: HashMap<&str, (usize, Severity)> = HashMap::new();
    let mut counts = [0usize; 3];
    let mut ignored = 0usize;
    for (i, hit) in ex.hits.iter().enumerate() {
        if hit.ignored {
            ignored += 1;
            if run.include_ignored || all_results {
                results.push(result_json(hit, &result_id(i), &ids, how)?);
            }
            continue;
        }
        results.push(result_json(hit, &result_id(i), &ids, how)?);
        let e = per_check.entry(hit.finding.check).or_insert((0, Severity::Info));
        e.0 += 1;
        e.1 = e.1.min(hit.finding.severity);
        if let Some(n) = counts.get_mut(hit.finding.severity as usize) {
            *n += 1;
        }
    }
    let (mut passed, mut skipped) = (0usize, 0usize);
    let mut checks = Vec::new();
    for c in &run.checks {
        let ran = ex.tally.ran.get(c.id).copied().unwrap_or(0);
        let skip = ex.tally.skipped.get(c.id);
        let mut m = Map::new();
        m.insert("id".into(), json!(c.id));
        let status = match (per_check.get(c.id), skip) {
            (Some((n, worst)), _) => {
                m.insert("results".into(), json!(n));
                worst.as_str()
            }
            (None, Some((_, reason))) if ran == 0 => {
                m.insert("results".into(), json!(0));
                m.insert("reason".into(), json!(reason));
                skipped += 1;
                "skipped"
            }
            (None, _) => {
                m.insert("results".into(), json!(0));
                passed += 1;
                "pass"
            }
        };
        if let Some((n, reason)) = skip
            && ran > 0
        {
            m.insert("skippedLayers".into(), json!(n));
            m.insert("reason".into(), json!(reason));
        }
        m.insert("status".into(), json!(status));
        if status != "pass" || run.include_passed {
            checks.push(Value::Object(m));
        }
    }
    let (module_checks, module_passed, errors) = ex.modules.entries(&per_check, run.include_passed);
    passed += module_passed;
    checks.extend(module_checks);
    let [fail, warn, info] = counts;
    let mut out = json!({
        "schema": SCHEMA,
        "tool": {"name": "fontcraft", "version": env!("CARGO_PKG_VERSION")},
        "target": {"kind": t.kind, "path": t.path, "fonts": t.fonts},
        "profile": run.profile,
        "config": run.config_json,
        "summary": {"fail": fail, "warn": warn, "info": info, "passed": passed, "skipped": skipped, "ignored": ignored, "errors": errors, "truncated": ex.tally.truncated},
        "checks": checks,
        "results": results,
    });
    if let Some(o) = out.as_object_mut() {
        if let Some(d) = &t.document {
            o.insert("document".into(), d.clone());
        }
        let warnings: Vec<&String> = run.warnings.iter().chain(&t.notes).collect();
        if !warnings.is_empty() {
            o.insert("warnings".into(), json!(warnings));
        }
    }
    Ok(out)
}

// ── The kept run ────────────────────────────────────────────────────────────────────────────────
//
// The last run of a document lives on its `DocState` until the next run or until the document
// closes; saving keeps it.

/// Where a kept result points.
#[derive(Clone, Debug)]
struct ResultRef {
    id: String,
    check: &'static str,
    glyph: String,
    layer: String,
}

/// The last `audit.run` of a document.
#[derive(Debug)]
pub struct KeptRun {
    /// The font the run examined, to tell which glyphs changed since.
    font: Arc<Font>,
    report: Value,
    refs: Vec<ResultRef>,
    run_id: String,
    /// The layer reports, for the next run (`audit.glyphHashCache`).
    cache: Option<Arc<cache::LayerCache>>,
}

fn kept_run(d: &DocState) -> Option<&KeptRun> {
    d.audit.as_deref()
}

/// Has `glyph` changed between the kept font and `now`?
fn glyph_changed(then: &Font, now: &Font, glyph: &str) -> bool {
    match (then.glyph(glyph), now.glyph(glyph)) {
        (Some(a), Some(b)) => !Arc::ptr_eq(a, b) && a.layers != b.layers,
        _ => true,
    }
}

// ── audit.run, audit.results ────────────────────────────────────────────────────────────────────

fn run(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.run", v)?;
    let all = p.bool_or("all", false)?;
    if let Some(paths) = target_paths(&p)? {
        let mut t = load_paths(&paths)?;
        let first = t.srcs.first().map(|s| s.model.clone()).unwrap_or_else(|| Arc::new(Font::new("")));
        let run = read_run(&p, s, &first, &t.settings)?;
        let ex = examine_target(&mut t, &run);
        let how = match (t.kind, t.path.as_deref()) {
            ("binary", Some(path)) => FixParams::File(path),
            _ => FixParams::BySelection,
        };
        let mut report = document(&t, &run, &ex, how, false)?;
        set_run_id(&mut report, &next_run_id());
        return Ok(report);
    }
    let doc = s.doc()?;
    let mut t = document_target_of(doc);
    let font = doc.font.clone();
    let mut run = read_run(&p, s, &font, &t.settings)?;
    run.keeps_cache = true;
    if !all {
        run.cache = kept_run(doc).and_then(|k| k.cache.clone());
    }
    let mut ex = examine_target(&mut t, &run);
    keep_run(s, &t, &run, &mut ex, font, next_run_id())
}

/// Ids of runs: unique for the life of the process, so a result list can tell whether the
/// engine's kept run is still the one it shows.
static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A new run id.
pub(super) fn next_run_id() -> String {
    let n = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed).saturating_add(1);
    format!("run{n}")
}

fn set_run_id(report: &mut Value, id: &str) {
    if let Some(o) = report.as_object_mut() {
        o.insert("runId".into(), json!(id));
    }
}

/// The id of the run the open document keeps, if any.
pub(super) fn kept_run_id(d: &DocState) -> Option<&str> {
    kept_run(d).map(|k| k.run_id.as_str())
}

/// Refuses a call whose `runId` names a run other than the one the document keeps.
pub(super) fn check_run_id(p: &Params, d: &DocState) -> Result<()> {
    let Some(asked) = p.opt_str("runId")? else { return Ok(()) };
    match kept_run_id(d) {
        Some(kept) if kept == asked => Ok(()),
        Some(kept) => Err(p.bad(format!("the results of run `{asked}` are no longer kept: the document keeps run `{kept}`; run the audit again"))),
        None => Err(p.bad(format!("the results of run `{asked}` are no longer kept: {NO_RUN}"))),
    }
}

/// Keeps the run `ex` of the open document (which examined `font`) for `audit.results` and
/// `audit.fix` under `run_id`, and returns its result document.
fn keep_run(s: &mut Session, t: &Target, run: &Run, ex: &mut Examined, font: Arc<Font>, run_id: String) -> Result<Value> {
    let mut kept = document(t, run, ex, FixParams::ById, true)?;
    set_run_id(&mut kept, &run_id);
    let report = if run.include_ignored {
        kept.clone()
    } else {
        let mut r = document(t, run, ex, FixParams::ById, false)?;
        set_run_id(&mut r, &run_id);
        r
    };
    let refs = ex
        .hits
        .iter()
        .enumerate()
        .map(|(i, h)| ResultRef {
            id: result_id(i),
            check: h.finding.check,
            glyph: h.finding.location.glyph.clone(),
            layer: h.finding.location.layer.clone(),
        })
        .collect();
    let cache = Some(Arc::new(std::mem::take(&mut ex.cache)));
    s.doc_mut()?.audit = Some(Arc::new(KeptRun { font, report: kept, refs, run_id, cache }));
    Ok(report)
}

fn results(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.results", v)?;
    document_target(&p)?;
    let include = patterns(&p, "checks")?;
    let exclude = patterns(&p, "exclude")?;
    let glyphs: Option<HashSet<&str>> = match p.opt_array("glyphs")? {
        Some(a) if a.len() > fontcraft_model::MAX_GLYPHS => {
            return Err(p.bad(format!("`glyphs` holds {} names; at most {} are allowed", a.len(), fontcraft_model::MAX_GLYPHS)));
        }
        _ => p.opt_strings("glyphs")?.map(|v| v.into_iter().collect()),
    };
    let min = severity_param(&p)?;
    let fixable = match p.get("fixable") {
        None => None,
        Some(_) => Some(p.bool_or("fixable", false)?),
    };
    let include_ignored = p.bool_or("includeIgnored", false)?;
    let doc = s.doc()?;
    check_run_id(&p, doc)?;
    let last = kept_run(doc).ok_or_else(|| p.bad(NO_RUN))?;
    // The ignore list as it is now: entries added or removed since the run apply.
    let settings = DocumentSettings::read(&doc.font).0;
    let ignore = settings.ignore_index();
    let mut ignored_count = 0usize;
    let (then, mut report) = (last.font.clone(), last.report.clone());
    let mut changed: HashMap<String, bool> = HashMap::new();
    let mut kept = Vec::new();
    let (mut counts, mut stale_count) = ([0usize; 3], 0usize);
    let list = report.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
    for mut r in list {
        let check = r.get("check").and_then(Value::as_str).unwrap_or("");
        let glyph = r.pointer("/location/glyph").and_then(Value::as_str).unwrap_or("").to_string();
        let severity = r.get("severity").and_then(Value::as_str).and_then(Severity::parse).unwrap_or(Severity::Info);
        let available = r.pointer("/fix/available").and_then(Value::as_bool).unwrap_or(false);
        let wanted = (include.is_empty() || include.iter().any(|pat| audit::matches_pattern(check, pat)))
            && !exclude.iter().any(|pat| audit::matches_pattern(check, pat))
            && glyphs.as_ref().is_none_or(|set| set.contains(glyph.as_str()))
            && severity <= min
            && fixable.is_none_or(|f| f == available);
        if !wanted {
            continue;
        }
        let master = r.pointer("/location/master").and_then(Value::as_str);
        let location = r.get("location").and_then(Value::as_object).map(super::audit_profiles::result_location).unwrap_or_default();
        let ignored = ignore.is_ignored(check, (!glyph.is_empty()).then_some(glyph.as_str()), master, &location);
        if let Some(o) = r.as_object_mut() {
            o.insert("ignored".into(), json!(ignored));
        }
        if ignored {
            ignored_count += 1;
            if !include_ignored {
                continue;
            }
        }
        let stale = !glyph.is_empty() && *changed.entry(glyph.clone()).or_insert_with(|| glyph_changed(&then, &doc.font, &glyph));
        if let Some(o) = r.as_object_mut() {
            o.insert("stale".into(), json!(stale));
        }
        stale_count += usize::from(stale);
        if !ignored && let Some(n) = counts.get_mut(severity as usize) {
            *n += 1;
        }
        kept.push(r);
    }
    if let Some(o) = report.as_object_mut() {
        o.insert("results".into(), Value::Array(kept));
        if let Some(Value::Object(summary)) = o.get_mut("summary") {
            let [fail, warn, info] = counts;
            summary.insert("fail".into(), json!(fail));
            summary.insert("warn".into(), json!(warn));
            summary.insert("info".into(), json!(info));
            summary.insert("stale".into(), json!(stale_count));
            summary.insert("ignored".into(), json!(ignored_count));
        }
    }
    Ok(report)
}

// ── audit.fix ───────────────────────────────────────────────────────────────────────────────────

/// One layer to repair and the checks whose findings select it.
struct Group {
    glyph: String,
    layer: String,
    master: String,
    checks: BTreeSet<&'static str>,
    /// The findings of the selected binary and family checks about this layer's glyph, which
    /// `audit.fix` cannot check again layer by layer.
    outside: Vec<Finding>,
}

/// The groups of a call, by glyph and layer index.
type Groups = BTreeMap<(usize, usize), Group>;

fn group_at<'g>(groups: &'g mut Groups, gi: usize, li: usize, glyph: &str, layer: &Layer) -> &'g mut Group {
    groups.entry((gi, li)).or_insert_with(|| Group {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        master: layer.master.clone(),
        checks: BTreeSet::new(),
        outside: Vec::new(),
    })
}

/// Does `c` read more than one layer, with a fix that is a layer step `audit.fix` can make?
fn outside_step(c: &CheckSpec) -> bool {
    c.scope != CheckScope::Layer && c.fix.is_some_and(|f| FIX_ORDER.contains(&f.id) && fix_via(&f).is_some())
}

/// Adds the findings of glyph, binary and family checks to the layers they are fixed on: the
/// layer a finding names, else every master layer of its glyph (a glyph of a compiled font is
/// named after its source glyph). With `create` false only layers already in `groups` that
/// selected the finding's check take it.
fn add_outside(font: &Font, findings: Vec<Finding>, run: &Run, create: bool, groups: &mut Groups) {
    let index: HashMap<&str, usize> = font.glyphs.as_arcs().iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    for f in findings {
        if f.location.glyph.is_empty() || run.glyphs.as_ref().is_some_and(|set| !set.contains(&f.location.glyph)) {
            continue;
        }
        let Some((gi, g)) = index.get(f.location.glyph.as_str()).and_then(|&i| font.glyphs.as_arcs().get(i).map(|g| (i, g))) else { continue };
        for (li, layer) in g.layers.iter().enumerate() {
            let named = if f.location.layer.is_empty() { layer.is_master_layer() } else { layer.id == f.location.layer };
            if !named || run.layers.as_ref().is_some_and(|set| !set.contains(&layer.id)) {
                continue;
            }
            if !create && !groups.get(&(gi, li)).is_some_and(|x| x.checks.contains(f.check)) {
                continue;
            }
            let group = group_at(groups, gi, li, &g.name, layer);
            group.checks.insert(f.check);
            group.outside.push(f.clone());
        }
    }
}

/// The findings of `checks` on the open document, compiling it when they read compiled fonts.
fn outside_findings(doc: &DocState, run: &Run, checks: Vec<&'static CheckSpec>) -> Vec<Finding> {
    let run = Run { checks, modules: crate::extensions::ModuleChecks::default(), ..run.clone() };
    let ex = examine_target(&mut document_target_of(doc), &run);
    ex.hits.into_iter().filter(|h| !h.ignored).map(|h| h.finding).collect()
}

/// The Font Info fixes a call selected, by fix id, with the checks that asked for each.
type InfoFixes = BTreeMap<&'static str, Vec<&'static CheckSpec>>;

/// Is `fix` one of the Font Info fixes of the first stage?
fn is_info_fix(fix: &str) -> bool {
    super::audit_fix_info::source_command(fix).is_some()
}

/// The command `audit.fix` runs for `fx` on the open document: the Font Info command of a fix
/// of the first stage, or the check's command for a layer step `audit.fix` builds calls for.
/// `None` when the call cannot be made yet.
fn fix_via(fx: &FixSpec) -> Option<&'static str> {
    let command = match super::audit_fix_info::source_command(fx.id) {
        Some(command) => command,
        None if font_steps::is_font_step(fx.id) => return font_steps::command(fx.id),
        None if FIX_ORDER.contains(&fx.id) => fx.command?,
        None => return None,
    };
    find_command(command).is_some().then_some(command)
}

/// What `audit.fix` reports.
#[derive(Default)]
struct FixReport {
    applied: Vec<Value>,
    backups: Vec<Value>,
    skipped: Vec<Value>,
    planned: Vec<Value>,
}

impl FixReport {
    fn skip(&mut self, glyph: Option<&str>, layer: Option<&str>, check: Option<&str>, fix: Option<&str>, reason: impl Into<String>) {
        let mut m = Map::new();
        for (key, value) in [("glyph", glyph), ("layer", layer), ("check", check), ("fix", fix)] {
            if let Some(value) = value {
                m.insert(key.into(), json!(value));
            }
        }
        m.insert("reason".into(), json!(reason.into()));
        self.skipped.push(Value::Object(m));
    }
}

/// The selection keys `results` cannot be combined with.
const FILTER_KEYS: [&str; 7] = ["profile", "checks", "exclude", "groups", "glyphs", "masters", "layers"];

fn fix(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.fix", v)?;
    if super::audit_fix_binary::is_file_target(&p) {
        return super::audit_fix_binary::fix_file(s, v);
    }
    document_target(&p)?;
    check_run_id(&p, s.doc()?)?;
    if p.get("output").is_some() {
        return Err(EngineError::Unsupported("writing a fixed copy to `output`; fix the open document and save it".into()));
    }
    let safe_only = p.bool_or("safeOnly", true)?;
    let dry = p.bool_or("dryRun", false)?;
    let backup = match p.opt_str("keepOriginal")? {
        None | Some("backupLayer") => true,
        Some("none") => false,
        Some(_) => return Err(p.bad("`keepOriginal` must be \"backupLayer\" or \"none\"")),
    };
    let mut report = FixReport::default();
    let mut info = InfoFixes::new();
    let mut steps = font_steps::Selected::default();
    let (mut run, groups) = match p.opt_array("results")? {
        Some(ids) => {
            if let Some(key) = FILTER_KEYS.iter().find(|k| p.get(k).is_some()) {
                return Err(p.bad(format!("give `results` or the filters (`{key}`…), not both")));
            }
            if ids.len() > MAX_RESULT_IDS {
                return Err(p.bad(format!("`results` holds {} ids; at most {MAX_RESULT_IDS} are allowed", ids.len())));
            }
            let ids = p.strings("results")?;
            let font = s.font()?;
            let settings = DocumentSettings::read(font).0;
            let mut run = read_run(&p, s, font, &settings)?;
            let groups = groups_from_ids(s, &p, &ids, &mut run, &mut info, &mut steps, &mut report)?;
            (run, groups)
        }
        None => {
            let doc = s.doc()?;
            let settings = DocumentSettings::read(&doc.font).0;
            let run = read_run(&p, s, &doc.font, &settings)?;
            // Every finding counts here: a check over the result cap is still fixed.
            let ex = examine(&doc.font, &run, false);
            let mut groups = groups_from_hits(&doc.font, &ex.hits);
            let outside: Vec<&'static CheckSpec> = run.checks.iter().copied().filter(|c| outside_step(c)).collect();
            if !outside.is_empty() {
                add_outside(&doc.font, outside_findings(doc, &run, outside), &run, true, &mut groups);
            }
            // The Font Info fixes answer findings of every scope, so those checks run in full,
            // compiling the source when they read compiled fonts.
            // So do the font-wide steps; each of them runs its checks again before it acts.
            let whole: Vec<&'static CheckSpec> =
                run.checks.iter().copied().filter(|c| c.fix.is_some_and(|f| is_info_fix(f.id) || font_steps::is_font_step(f.id))).collect();
            if !whole.is_empty() {
                let whole_run = Run { checks: whole, modules: crate::extensions::ModuleChecks::default(), ..run.clone() };
                let ex = examine_target(&mut document_target_of(doc), &whole_run);
                for h in ex.hits.iter().filter(|h| !h.ignored) {
                    if let (Some(fx), Some(spec)) = (h.finding.fix, audit::find(h.finding.check)) {
                        if is_info_fix(fx.id) {
                            add_info(&mut info, fx.id, spec);
                        } else {
                            add_info(&mut steps.fixes, fx.id, spec);
                        }
                    }
                }
            }
            steps.glyphs = run.glyphs.clone();
            (run, groups.into_values().collect::<Vec<Group>>())
        }
    };
    let touched: HashSet<String> = groups.iter().map(|g| g.glyph.clone()).collect();
    let facts = {
        let ctx = AuditContext::new(s.font()?);
        (ctx.grid_step, ctx.main_curve, ctx.variable)
    };
    let how = How { safe_only, dry, backup, facts };
    one_step(s, |s| {
        fix_info(s, &info, &how, &mut report);
        font_steps::run_steps(s, &run, &steps.fixes, steps.glyphs.as_ref(), &font_steps::DATA_STEPS, &how, &mut report)?;
        for group in &groups {
            fix_layer(s, &run, group, &how, &mut report)?;
        }
        font_steps::run_steps(s, &run, &steps.fixes, steps.glyphs.as_ref(), &font_steps::LATE_STEPS, &how, &mut report)?;
        Ok(())
    })?;
    // `remaining`: the same checks on the glyphs involved, as they are now, and the checks of
    // the Font Info fixes on the whole font; the font-wide steps' checks on the glyphs they
    // were given.
    run.glyphs = match (steps.fixes.is_empty(), steps.glyphs.take()) {
        (true, _) => Some(touched),
        (false, None) => None,
        (false, Some(named)) => Some(named.into_iter().chain(touched).collect()),
    };
    let doc = s.doc()?;
    let mut target = document_target_of(doc);
    let used_outside: Vec<&'static CheckSpec> =
        run.checks.iter().copied().filter(|c| c.scope != CheckScope::Layer && groups.iter().any(|g| g.checks.contains(c.id))).collect();
    let ex = if info.is_empty() && used_outside.is_empty() && steps.fixes.is_empty() {
        examine(&doc.font, &run, true)
    } else {
        let mut checks: Vec<&'static CheckSpec> = run.checks.iter().copied().filter(|c| c.scope == CheckScope::Layer).collect();
        for &spec in info.values().flatten().chain(&used_outside).chain(steps.fixes.values().flatten()) {
            if !checks.iter().any(|c| c.id == spec.id) {
                checks.push(spec);
            }
        }
        run.checks = CHECKS.iter().filter(|c| checks.iter().any(|k| k.id == c.id)).collect();
        examine_target(&mut target, &run)
    };
    let remaining = document(&target, &run, &ex, FixParams::BySelection, false)?;
    let mut out = json!({
        "applied": report.applied,
        "backups": report.backups,
        "skipped": report.skipped,
        "remaining": remaining,
        "dryRun": dry,
    });
    if dry && let Some(o) = out.as_object_mut() {
        o.insert("planned".into(), Value::Array(report.planned));
    }
    Ok(out)
}

/// The layers and checks named by result ids of the kept run.
fn groups_from_ids(
    s: &Session,
    p: &Params,
    ids: &[&str],
    run: &mut Run,
    info: &mut InfoFixes,
    steps: &mut font_steps::Selected,
    report: &mut FixReport,
) -> Result<Vec<Group>> {
    let doc = s.doc()?;
    let last = kept_run(doc).ok_or_else(|| p.bad(NO_RUN))?;
    let (then, refs) = (last.font.clone(), last.refs.clone());
    let index: HashMap<&str, &ResultRef> = refs.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut groups = Groups::new();
    let mut checks: Vec<&'static CheckSpec> = Vec::new();
    for id in ids {
        let r = index.get(id).ok_or_else(|| p.bad(format!("`results`: the last audit has no result `{id}`")))?;
        if let Some(spec) = audit::find(r.check)
            && let Some(fx) = spec.fix
            && is_info_fix(fx.id)
        {
            add_info(info, fx.id, spec);
            continue;
        }
        if let Some(spec) = audit::find(r.check)
            && let Some(fx) = spec.fix
            && font_steps::is_font_step(fx.id)
        {
            add_info(&mut steps.fixes, fx.id, spec);
            steps.name(&r.glyph);
            continue;
        }
        if r.glyph.is_empty() {
            report.skip(None, None, Some(r.check), None, format!("{id} sums up many results and names no layer: fix by check instead"));
            continue;
        }
        if glyph_changed(&then, &doc.font, &r.glyph) {
            report.skip(
                Some(&r.glyph),
                Some(&r.layer),
                Some(r.check),
                None,
                format!("{id}: the glyph changed since the audit ran; run audit.run again"),
            );
            continue;
        }
        let Some((gi, g)) = doc.font.glyphs.as_arcs().iter().enumerate().find(|(_, g)| g.name == r.glyph) else {
            report.skip(Some(&r.glyph), Some(&r.layer), Some(r.check), None, format!("{id}: the glyph no longer exists"));
            continue;
        };
        // A result of a compiled font names no layer: it is fixed on every master layer.
        let places: Vec<(usize, &Layer)> =
            g.layers.iter().enumerate().filter(|(_, l)| if r.layer.is_empty() { l.is_master_layer() } else { l.id == r.layer }).collect();
        if places.is_empty() {
            report.skip(Some(&r.glyph), Some(&r.layer), Some(r.check), None, format!("{id}: the layer no longer exists"));
            continue;
        }
        let Some(spec) = audit::find(r.check) else { continue };
        if !checks.iter().any(|c| c.id == spec.id) {
            checks.push(spec);
        }
        for (li, layer) in places {
            group_at(&mut groups, gi, li, &g.name, layer).checks.insert(spec.id);
        }
    }
    run.checks = CHECKS.iter().filter(|c| checks.iter().any(|k| k.id == c.id)).collect();
    run.glyphs = Some(groups.values().map(|g| g.glyph.clone()).collect());
    run.layers = Some(groups.values().map(|g| g.layer.clone()).collect());
    let outside: Vec<&'static CheckSpec> = run.checks.iter().copied().filter(|c| outside_step(c)).collect();
    if !outside.is_empty() {
        add_outside(&doc.font, outside_findings(doc, run, outside), run, false, &mut groups);
    }
    Ok(groups.into_values().collect())
}

/// The layers with findings that have a fix, in font order, each with the checks that reported
/// it. Findings without a fix stay in `remaining` and are not listed as skipped.
fn groups_from_hits(font: &Font, hits: &[Hit]) -> Groups {
    let mut groups = Groups::new();
    for h in hits
        .iter()
        .filter(|h| !h.finding.location.glyph.is_empty() && h.finding.fix.is_some_and(|f| !is_info_fix(f.id) && !font_steps::is_font_step(f.id)))
    {
        if font.glyph(&h.finding.location.glyph).is_none() {
            continue;
        }
        groups
            .entry((h.glyph, h.layer))
            .or_insert_with(|| Group {
                glyph: h.finding.location.glyph.clone(),
                layer: h.finding.location.layer.clone(),
                master: h.master.clone(),
                checks: BTreeSet::new(),
                outside: Vec::new(),
            })
            .checks
            .insert(h.finding.check);
    }
    groups
}

fn add_info(info: &mut InfoFixes, fix: &'static str, spec: &'static CheckSpec) {
    let checks = info.entry(fix).or_default();
    if !checks.iter().any(|c| c.id == spec.id) {
        checks.push(spec);
    }
}

/// The first stage: each selected Font Info fix runs once, in the order of § 5, through its
/// command with that command's defaults. With `dryRun` the command runs as a dry run too and its
/// planned edits are listed. Each command is one edit, so one that fails changes nothing and is
/// listed in `skipped`.
fn fix_info(s: &mut Session, info: &InfoFixes, how: &How, report: &mut FixReport) {
    for (fix, command) in super::audit_fix_info::SOURCE_FIXES {
        let Some(checks) = info.get(fix) else { continue };
        let mut allowed = Vec::new();
        for c in checks {
            if how.safe_only && !c.fix.is_some_and(|f| f.safe) {
                report.skip(None, None, Some(c.id), Some(fix), "the fix changes the design: give safeOnly: false");
            } else {
                allowed.push(c.id);
            }
        }
        if allowed.is_empty() {
            continue;
        }
        if find_command(command).is_none() {
            report.skip(None, None, None, Some(fix), NO_FIX_CALL);
            continue;
        }
        let params = json!({});
        let call = if how.dry { json!({"dryRun": true}) } else { params.clone() };
        match s.execute(command, &call) {
            Ok(r) => {
                let changed = r.get("changed").cloned().unwrap_or_else(|| json!([]));
                if how.dry {
                    report.planned.push(json!({"fix": fix, "command": command, "params": params, "checks": allowed, "changed": changed}));
                } else if changed.as_array().is_some_and(|a| !a.is_empty()) {
                    report.applied.push(json!({"fix": fix, "command": command, "params": params, "checks": allowed, "changed": changed}));
                }
            }
            Err(e) => report.skip(None, None, None, Some(fix), format!("the fix failed and changed nothing: {e}")),
        }
    }
}

/// Runs `f` as one undo step, as `command.batch` does; inside a batch it is part of that batch.
fn one_step<T>(s: &mut Session, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    let outer = s.batch.is_none();
    if outer {
        s.batch = Some(Batch { label: FIX_LABEL.into(), before: Vec::new() });
    }
    let out = f(s);
    if outer {
        let finished = s.batch.take();
        if out.is_ok()
            && let Some(batch) = finished
        {
            for (id, before) in batch.before {
                let Some(i) = s.position_of(id) else { continue };
                if let Some(doc) = s.docs_mut().get_mut(i)
                    && !doc.font.same_as(&before)
                {
                    crate::push_undo(doc, batch.label.clone(), before);
                }
            }
        }
    }
    out
}

/// The layer `id` of `glyph`.
fn find_layer<'f>(font: &'f Font, glyph: &str, id: &str) -> Option<(&'f Arc<Glyph>, &'f Layer)> {
    let g = font.glyph(glyph)?;
    g.layers.iter().find(|l| l.id == id).map(|l| (g, l))
}

/// The paths a set of findings touches, by shape index.
fn paths_of(layer: &Layer, findings: &[Finding], keep: impl Fn(&Path) -> bool) -> Vec<usize> {
    let mut set = BTreeSet::new();
    for f in findings {
        let l = &f.location;
        set.extend(l.shapes.iter().copied());
        set.extend(l.points.iter().map(|[s, _]| *s));
        set.extend(l.segments.iter().map(|[s, _]| *s));
    }
    set.into_iter().filter(|&i| matches!(layer.shapes.get(i), Some(Shape::Path(p)) if keep(p))).collect()
}

/// The lowest, then leftmost, on-curve node of a path.
fn lowest_leftmost(path: &Path) -> Option<usize> {
    let mut best: Option<(usize, f64, f64)> = None;
    for (i, n) in path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()) {
        if best.is_none_or(|(_, x, y)| n.y < y || (n.y == y && n.x < x)) {
            best = Some((i, n.x, n.y));
        }
    }
    best.map(|(i, _, _)| i)
}

/// The command calls that carry out `fix` for these findings on one layer. Contours that cannot
/// be done are listed in `report`.
#[allow(clippy::too_many_arguments)]
fn calls_for(
    fix: &str,
    command: &'static str,
    font: &Font,
    glyph: &Glyph,
    layer: &Layer,
    findings: &[Finding],
    main_curve: Option<CurveType>,
    report: &mut FixReport,
) -> Vec<(&'static str, Value)> {
    let (g, l) = (glyph.name.as_str(), layer.id.as_str());
    let whole = || vec![(command, json!({"glyph": g, "layer": l}))];
    let delete = |shapes: BTreeSet<usize>| {
        let shapes: Vec<usize> = shapes.into_iter().filter(|&i| matches!(layer.shapes.get(i), Some(Shape::Path(_)))).collect();
        if shapes.is_empty() { Vec::new() } else { vec![(command, json!({"glyph": g, "layer": l, "shapes": shapes}))] }
    };
    let points = || {
        let set: BTreeSet<[usize; 2]> = findings.iter().flat_map(|f| f.location.points.iter().copied()).collect();
        set.into_iter().collect::<Vec<_>>()
    };
    let some = |shapes: Vec<usize>, extra: Option<(&str, Value)>| {
        if shapes.is_empty() {
            return Vec::new();
        }
        let mut params = json!({"glyph": g, "layer": l, "shapes": shapes});
        if let (Some((key, value)), Some(o)) = (extra, params.as_object_mut()) {
            o.insert(key.into(), value);
        }
        vec![(command, params)]
    };
    match fix {
        "closePaths" => some(paths_of(layer, findings, |p| !p.closed && p.nodes.iter().filter(|n| n.on_curve()).count() >= 2), None),
        "tidyPaths" => {
            // Curves the almost-straight check reported are straightened as far as it reported.
            let straighten = findings.iter().filter_map(|f| f.data.get("deviation")).fold(0.0, |a: f64, &d| a.max(d));
            some(paths_of(layer, findings, |_| true), (straighten > 0.0).then(|| ("straighten", json!(straighten + STRAIGHTEN_MARGIN))))
        }
        "addExtremes" | "addInflections" => some(paths_of(layer, findings, |_| true), None),
        "removeOverlap" => some(paths_of(layer, findings, |_| true), Some(("keepOpenPaths", json!(true)))),
        "correctPathDirection" | "roundCoordinates" => whole(),
        // Each finding names the first path and its copy; the copies go.
        "removeDuplicateContours" => delete(findings.iter().flat_map(|f| f.location.shapes.iter().skip(1).copied()).collect()),
        "deleteTinyContours" => delete(findings.iter().flat_map(|f| f.location.shapes.iter().copied()).collect()),
        "convertCurveType" => match main_curve {
            Some(CurveType::Quadratic) => vec![("path.convertToQuadratic", json!({"glyph": g, "layer": l}))],
            Some(CurveType::Cubic) => vec![("path.convertToCubic", json!({"glyph": g, "layer": l}))],
            None => Vec::new(),
        },
        "setSmooth" => {
            let points = points();
            if points.is_empty() {
                Vec::new()
            } else {
                vec![(command, json!({"glyph": g, "layer": l, "points": points, "to": "smooth", "keepOutline": true}))]
            }
        }
        "harmonize" => {
            let points = points();
            if points.is_empty() { Vec::new() } else { vec![(command, json!({"glyph": g, "layer": l, "points": points}))] }
        }
        "setStartPoint" => {
            let masters: Vec<&Layer> = glyph.layers.iter().filter(|x| x.is_master_layer()).collect();
            let mut calls = Vec::new();
            let mut done = BTreeSet::new();
            for f in findings {
                let Some(&[shape, _]) = f.location.points.first() else { continue };
                let Some(Shape::Path(path)) = layer.shapes.get(shape) else { continue };
                let Some(node) = lowest_leftmost(path) else { continue };
                if !done.insert(shape) {
                    continue;
                }
                let agree = masters.len() < 2
                    || masters.iter().all(|m| matches!(m.shapes.get(shape), Some(Shape::Path(mp)) if mp.closed && lowest_leftmost(mp) == Some(node)));
                if agree {
                    calls.push((command, json!({"glyph": g, "layer": l, "point": [shape, node]})));
                } else {
                    report.skip(Some(g), Some(l), Some(f.check), Some(fix), format!("path {shape}: the masters disagree on its start node"));
                }
            }
            calls
        }
        _ => match super::audit_fix_glyph::step_calls(fix, font, glyph, layer, findings) {
            Some(step) => {
                for (check, reason) in step.skipped {
                    report.skip(Some(g), Some(l), Some(check), Some(fix), reason);
                }
                step.calls
            }
            None => Vec::new(),
        },
    }
}

/// The font-wide facts of a run, as [`AuditContext`] holds them.
type Facts = (Option<f64>, Option<CurveType>, bool);

/// How `audit.fix` repairs a layer.
#[derive(Clone, Copy)]
struct How {
    safe_only: bool,
    dry: bool,
    backup: bool,
    facts: Facts,
}

/// Repairs one layer: checks it before each fix step and runs that fix's command on what the
/// check reports now. On failure the layer is left as it was.
fn fix_layer(s: &mut Session, run: &Run, group: &Group, how: &How, report: &mut FixReport) -> Result<()> {
    let How { safe_only, dry, backup, facts } = *how;
    let (g, l) = (group.glyph.as_str(), group.layer.as_str());
    let font = s.font()?;
    let Some((glyph, _)) = find_layer(font, g, l) else { return Ok(()) };
    let mut fixes: Vec<&'static CheckSpec> = Vec::new();
    for &id in &group.checks {
        let Some(spec) = run.checks.iter().find(|c| c.id == id) else { continue };
        let reason = match spec.fix {
            None => Some("the check has no automatic fix"),
            Some(fx) if fix_via(&fx).is_none() => Some(NO_FIX_CALL),
            Some(fx) if safe_only && !fx.safe => Some("the fix changes the design: give safeOnly: false"),
            Some(_) => None,
        };
        match reason {
            Some(reason) => report.skip(Some(g), Some(l), Some(id), spec.fix.map(|f| f.id), reason),
            None => fixes.push(spec),
        }
    }
    if fixes.is_empty() {
        return Ok(());
    }
    if glyph.info.locked {
        report.skip(Some(g), Some(l), None, None, EngineError::Locked(g.to_string()).to_string());
        return Ok(());
    }
    let mut told_components = false;
    let before = s.doc()?.font.clone();
    let mut steps = Vec::new();
    let mut made_backup = None;
    let mut seen = HashSet::new();
    let outcome = (|| -> Result<()> {
        for (n, fix) in FIX_ORDER.iter().enumerate() {
            let checks: Vec<&'static CheckSpec> = fixes.iter().copied().filter(|c| c.fix.is_some_and(|f| f.id == *fix)).collect();
            // The second clean-up pass runs only after an overlap removal.
            let again = FIX_ORDER.iter().take(n).any(|f| f == fix);
            if checks.is_empty() || (again && !seen.contains("removeOverlap")) || (dry && again) {
                continue;
            }
            let Some(command) = checks.iter().find_map(|c| c.fix.and_then(|f| f.command)) else { continue };
            let calls = {
                let font = s.font()?;
                let Some((glyph, layer)) = find_layer(font, g, l) else { return Ok(()) };
                // The outline steps leave a layer with components alone under safeOnly; the
                // component steps before them may have removed the components.
                if safe_only && !NOT_OUTLINE_STEPS.contains(fix) && layer.shapes.iter().any(|x| matches!(x, Shape::Component(_))) {
                    if !told_components {
                        report.skip(Some(g), Some(l), None, Some(fix), HAS_COMPONENTS);
                        told_components = true;
                    }
                    continue;
                }
                let found = check_again(font, facts, glyph, layer, &checks, run, group);
                if found.is_empty() {
                    continue;
                }
                let calls = calls_for(fix, command, font, glyph, layer, &found, facts.1, report);
                if dry {
                    for (command, params) in &calls {
                        report.planned.push(json!({"fix": fix, "glyph": g, "layer": l, "master": group.master, "command": command, "params": params, "results": found.len()}));
                    }
                    continue;
                }
                calls
            };
            if calls.is_empty() {
                continue;
            }
            if backup && made_backup.is_none() {
                let r = s.execute("layers.addBackupLayer", &json!({"glyphs": [g], "master": group.master, "from": l}))?;
                made_backup = r.get("layer").and_then(Value::as_str).map(str::to_string);
            }
            for (command, params) in calls {
                s.execute(command, &params)?;
                steps.push(json!({"fix": fix, "glyph": g, "layer": l, "master": group.master, "command": command, "params": params}));
            }
            seen.insert(*fix);
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        s.doc_mut()?.font = before;
        report.skip(Some(g), Some(l), None, None, format!("the fix failed and the layer was left as it was: {e}"));
        return Ok(());
    }
    let unchanged = match (find_layer(&before, g, l), find_layer(s.font()?, g, l)) {
        (Some((_, a)), Some((_, b))) => a == b,
        _ => false,
    };
    if unchanged {
        // Nothing changed but perhaps a backup was made: drop it too.
        s.doc_mut()?.font = before;
        return Ok(());
    }
    report.applied.extend(steps);
    if let Some(id) = made_backup {
        report.backups.push(json!({"glyph": g, "layer": l, "backup": id}));
    }
    Ok(())
}

/// What `checks` report on one layer now: layer checks run on the layer, glyph checks on the glyph
/// (their findings about this layer), and the findings of binary and family checks are those of
/// the selection, since the layer cannot be compiled on its own.
fn check_again(font: &Font, facts: Facts, glyph: &Glyph, layer: &Layer, checks: &[&'static CheckSpec], run: &Run, group: &Group) -> Vec<Finding> {
    let ctx = context(font, facts);
    let of = |scope: CheckScope| -> Vec<&'static CheckSpec> { checks.iter().copied().filter(|c| c.scope == scope).collect() };
    let mut found = Vec::new();
    let on_layer = of(CheckScope::Layer);
    if !on_layer.is_empty() {
        found.extend(audit::audit_layer(&ctx, glyph, layer, &on_layer, &run.config).findings);
    }
    let on_glyph = of(CheckScope::Glyph);
    if !on_glyph.is_empty() {
        let services = super::audit_services::EngineServices;
        let gctx = audit::GlyphCtx::new(&ctx, glyph, &services);
        let report = audit::audit_glyph(&gctx, &on_glyph, &run.config);
        found.extend(report.findings.into_iter().filter(|f| f.location.layer.is_empty() || f.location.layer == layer.id));
    }
    found.extend(
        group.outside.iter().filter(|f| checks.iter().any(|c| c.id == f.check && !matches!(c.scope, CheckScope::Layer | CheckScope::Glyph))).cloned(),
    );
    found
}

/// An audit context for `font` with the font-wide facts of the run, so a step does not survey the
/// whole font again.
fn context(font: &Font, (grid_step, main_curve, variable): Facts) -> AuditContext<'_> {
    AuditContext::with_facts(font, grid_step, main_curve, variable)
}

// ── audit.list, audit.describe, audit.profile.list ─────────────────────────────────────────────

/// audit.fix carries the check's fix out on the open document or on a compiled font file.
fn fix_available(c: &CheckSpec) -> bool {
    c.fix.as_ref().is_some_and(|f| fix_via(f).is_some() || super::audit_fix_binary::file_fix_command(f.id).is_some())
}

fn list(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.list", v)?;
    let profile = profile_param(&p)?;
    let groups = names_from(&p, "groups", &GROUPS)?;
    let scopes = names_from(&p, "scope", &SCOPES)?;
    let fixable = match p.get("fixable") {
        None => None,
        Some(_) => Some(p.bool_or("fixable", false)?),
    };
    let checks: Vec<Value> = CHECKS
        .iter()
        .filter(|c| profile.is_none_or(|pr| pr.includes(c.profile)))
        .filter(|c| groups.as_ref().is_none_or(|g| g.contains(&group_of(c.id))))
        .filter(|c| scopes.as_ref().is_none_or(|s| s.contains(&c.scope.row_scope())))
        .filter(|c| fixable.is_none_or(|f| f == c.fix.is_some()))
        .map(|c| {
            json!({
                "id": c.id,
                "label": c.label,
                "group": group_of(c.id),
                "severity": c.severity.as_str(),
                "scope": c.scope.row_scope(),
                "profile": c.profile.as_str(),
                "fixable": c.fix.is_some(),
                "fix": c.fix.map(|f| f.id),
                "fixAvailable": fix_available(c),
                "description": c.description,
            })
        })
        .collect();
    Ok(json!({ "checks": checks }))
}

fn describe(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.describe", v)?;
    let id = p.str("check")?;
    let c = audit::find(id).ok_or_else(|| p.bad(format!("there is no check `{id}`")))?;
    let profiles: Vec<&str> = Profile::ALL.into_iter().filter(|pr| pr.includes(c.profile)).map(Profile::as_str).collect();
    let family = serde_json::to_value(c.family).map_err(|e| EngineError::Internal(e.to_string()))?;
    let thresholds = serde_json::to_value(c.thresholds).map_err(|e| EngineError::Internal(e.to_string()))?;
    let fix = c.fix.map(|f| json!({"id": f.id, "command": f.command, "available": fix_available(c), "safe": f.safe, "description": f.description}));
    let mut codes = c.codes.to_vec();
    if c.capped {
        codes.push(audit::TOO_MANY_RESULTS);
    }
    let mut preconditions = vec!["the source is read as it is: no compile"];
    if c.skip_variable {
        preconditions.push("the source has one master: on several masters the heuristic misfires on outlines that must stay compatible");
    }
    Ok(json!({
        "id": c.id,
        "label": c.label,
        "description": c.description,
        "group": group_of(c.id),
        "family": family,
        "severity": c.severity.as_str(),
        "scope": c.scope.row_scope(),
        "profile": c.profile.as_str(),
        "profiles": profiles,
        "thresholds": thresholds,
        "codes": codes,
        "fix": fix,
        "preconditions": preconditions,
        "capped": c.capped,
    }))
}

fn profile_list(s: &mut Session, _: &Value) -> Result<Value> {
    let (stored, default, problems) = super::audit_profiles::stored_profiles(s);
    let about = |p: Profile| match p {
        Profile::Outlines => "Per-glyph outline checks of a source; fast enough to run after every edit",
        Profile::Opentype => "Conformance of a compiled font with the OpenType specification",
        Profile::Universal => "Outlines and OpenType, plus practices that hold for any professional font",
        Profile::Strict => "Every check, house-style and heuristic ones included",
    };
    let mut profiles: Vec<Value> = Profile::ALL
        .into_iter()
        .map(|p| {
            json!({
                "id": p.as_str(),
                "label": p.as_str(),
                "checks": CHECKS.iter().filter(|c| p.includes(c.profile)).count(),
                "builtIn": true,
                "default": p.as_str() == default,
                "description": about(p),
                "store": prof::Store::BuiltIn.as_str(),
            })
        })
        .collect();
    profiles.extend(stored);
    let mut out = json!({ "profiles": profiles });
    if !problems.is_empty()
        && let Value::Object(o) = &mut out
    {
        o.insert("problems".into(), json!(problems));
    }
    Ok(out)
}
