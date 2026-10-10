//! The audit commands (M3.17): run the outline checks of `fontcraft-audit` on the open document,
//! list and describe them, return the last run's results, and fix what they report
//! (`docs/spec/21-qa.md` §§ 3–5).
//!
//! A run examines every layer of the selected glyphs except backup layers and returns the result
//! document of § 4. The last run of each document is kept for `audit.results` and for
//! `audit.fix {results}`; a glyph edited since then makes its results `stale`.
//!
//! `audit.fix` carries each fix out with the engine command its check names, in the order of
//! `audit.outlineFixOrder`, re-checking the layer before every step so each command gets the
//! current addresses. The unfixed layer is first copied to a backup layer of its master
//! (decision D12), and the whole call is one undo step.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use fontcraft_audit::{self as audit, AuditContext, CHECKS, CheckSpec, Config, CurveType, Finding, Profile, Severity};
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
/// Most documents whose last run is kept.
const MAX_CACHED: usize = 32;
const GROUPS: [&str; 16] = [
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
];
const SCOPES: [&str; 3] = ["source", "binary", "family"];
/// `audit.outlineFixOrder`: clean-up first, then overlap removal, then clean-up again for what the
/// union left, then direction, extremes, start points, and rounding last.
const FIX_ORDER: [&str; 9] = [
    "closePaths",
    "tidyPaths",
    "removeOverlap",
    "tidyPaths",
    "correctPathDirection",
    "addExtremes",
    "addInflections",
    "setStartPoint",
    "roundCoordinates",
];
const FIX_LABEL: &str = "Fix Audit Results";
const NO_RUN: &str = "no audit has run on this document since it was opened or last saved: run audit.run first";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "audit.run",
            "Run Audit",
            [],
            None,
            "{profile?: \"outlines\" | \"opentype\" | \"universal\" | \"strict\" (default universal), checks?: [id | prefix*], exclude?: [id | prefix*], groups?: [group], scope?: [\"source\" | \"binary\" | \"family\"], glyphs?: [names], masters?: [ids], layers?: [ids], minSeverity?: \"info\" | \"warn\" | \"fail\", config?: {checkId: {threshold: number}}, maxResults?: count = 100, includePassed?: bool = false, target?: \"document\"} runs the checks on the open document: every layer of the glyphs except backup layers (or the layers in `layers`), and changes nothing. A pattern that matches no check is an error. A capped check with more than maxResults results in the font gives one info result with the count instead. The run is kept for audit.results and audit.fix → {schema, tool, target, document: {title, revision}, profile, config, summary: {fail, warn, info, passed, skipped, ignored, errors, truncated}, checks: [{id, status, results, reason?}], results: [{id, check, code, severity, message, location: {font, glyph, master, layer, shapes?, points?, segments?, point?, bounds?}, data, fix: {available, id?, safe?, description?, command?, params?, via?}, ignored, stale}]}",
            has_doc,
            run
        ),
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
            "{check: id} everything known about one check → {id, label, description, group, family, severity, scope, profile, profiles: [ids], thresholds: [{name, default, min, max, doc}], fix: {id, command, available, safe, description} | null, preconditions: [text], capped}",
            always,
            describe
        ),
        cmd!(
            query "audit.results",
            "Get Audit Results",
            [],
            None,
            "{checks?: [id | prefix*], exclude?: [id | prefix*], glyphs?: [names], minSeverity?: \"info\" | \"warn\" | \"fail\", fixable?: bool, includeIgnored?: bool, target?: \"document\"} the result document of the last audit.run on the open document, without running anything, narrowed to the matching results. A result whose glyph changed since the run has `stale: true`. An error when no run is kept (none since the document was opened or last saved) → the result document of audit.run, with summary.stale",
            has_doc,
            results
        ),
        cmd!(
            "audit.fix",
            "Fix Issues",
            [],
            None,
            "{results?: [result id] (from the last audit.run; not with the other filters), profile?, checks?: [id | prefix*], exclude?, groups?, glyphs?: [names], masters?: [ids], layers?: [ids], minSeverity?, config?, safeOnly?: bool = true, dryRun?: bool = false, keepOriginal?: \"backupLayer\" | \"none\" = \"backupLayer\", maxResults?: count = 100, target?: \"document\"} repairs each selected layer that a check with a fix command reports, by running the commands path.closeOpenPaths, path.tidyUpPaths, path.removeOverlap, path.correctPathDirection, path.addExtremes, path.makeNodeFirst and path.roundCoordinates, in that order, on the paths involved. The unfixed layer is first copied to a backup layer of its master unless keepOriginal is \"none\". All of it is one undo step. safeOnly skips fixes that change the design and layers with components. A locked glyph, a stale result and a layer whose fix fails are skipped and left as they were → {applied: [{fix, glyph, layer, master, command, params}], backups: [{glyph, layer, backup}], skipped: [{glyph?, layer?, check?, fix?, reason}], planned?: [{fix, glyph, layer, master, command, params, results}] (dryRun), remaining: the result document of the selected checks on the glyphs involved}",
            has_doc,
            fix
        ),
        cmd!(
            query "audit.profile.list",
            "List Audit Profiles",
            [],
            None,
            "{} the built-in profiles with the number of registered checks each runs → {profiles: [{id, checks, builtIn, default, description}]}",
            always,
            profile_list
        ),
    ]
}

// ── Reading a selection ─────────────────────────────────────────────────────────────────────────

/// What one run examines, with which settings.
struct Run {
    profile: Profile,
    checks: Vec<&'static CheckSpec>,
    glyphs: Option<HashSet<String>>,
    masters: Option<HashSet<String>>,
    layers: Option<HashSet<String>>,
    config: Config,
    config_json: Value,
    min_severity: Severity,
    max_results: usize,
    include_passed: bool,
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
        if !CHECKS.iter().any(|c| audit::matches_pattern(c.id, pat)) {
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

/// Every check in the registry reads the editable document.
const CHECK_SCOPE: &str = "source";

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
    if let Some(missing) = list.iter().find(|id| !exists(**id)) {
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

/// Reads the selection keys shared by `audit.run` and `audit.fix`.
fn read_run(p: &Params, font: &Font) -> Result<Run> {
    document_target(p)?;
    let profile = profile_param(p)?.unwrap_or(Profile::Universal);
    let include = patterns(p, "checks")?;
    let exclude = patterns(p, "exclude")?;
    let groups = names_from(p, "groups", &GROUPS)?;
    let scopes = names_from(p, "scope", &SCOPES)?;
    let checks = audit::select(Some(profile), &include, &exclude)
        .into_iter()
        .filter(|c| groups.as_ref().is_none_or(|g| g.contains(&group_of(c.id))))
        .filter(|_| scopes.as_ref().is_none_or(|s| s.contains(&CHECK_SCOPE)))
        .collect();
    let glyphs = p.opt_glyphs("glyphs", font)?.map(|v| v.into_iter().collect());
    let masters = id_set(p, "masters", |id| font.master(id).is_ok())?;
    let layers = id_set(p, "layers", |id| font.glyphs.as_arcs().iter().any(|g| g.layers.iter().any(|l| l.id == id)))?;
    let (config, config_json) = config_param(p)?;
    let max =
        p.opt_int("maxResults", 0, MAX_MAX_RESULTS)?.map_or(audit::DEFAULT_MAX_RESULTS, |n| usize::try_from(n).unwrap_or(audit::DEFAULT_MAX_RESULTS));
    Ok(Run {
        profile,
        checks,
        glyphs,
        masters,
        layers,
        config,
        config_json,
        min_severity: severity_param(p)?,
        max_results: max,
        include_passed: p.bool_or("includePassed", false)?,
    })
}

// ── Running the checks ──────────────────────────────────────────────────────────────────────────

fn is_backup(l: &Layer) -> bool {
    !l.is_master_layer() && l.info.attrs.is_empty()
}

/// One finding with where it sorts: glyph and layer positions in the font.
struct Hit {
    finding: Finding,
    glyph: usize,
    layer: usize,
    master: String,
}

/// What one run found.
#[derive(Default)]
struct Examined {
    hits: Vec<Hit>,
    /// Per check: the layers it ran on.
    ran: HashMap<&'static str, usize>,
    /// Per check: the layers it skipped, and the first reason.
    skipped: HashMap<&'static str, (usize, String)>,
    /// Findings dropped: over a check's per-layer limit, or over [`MAX_REPORTED`].
    truncated: usize,
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

/// Runs the checks, applies `minSeverity` and, with `cap`, the result cap and [`MAX_REPORTED`],
/// and sorts the results: by severity, check id, glyph order and layer order.
fn examine(font: &Font, run: &Run, cap: bool) -> Examined {
    let ctx = AuditContext::new(font);
    let mut ex = Examined::default();
    let mut places: HashMap<(String, String), (usize, usize, String)> = HashMap::new();
    let mut findings = Vec::new();
    for (gi, g, li, layer) in layers_of(font, run) {
        let report = audit::audit_layer(&ctx, g, layer, &run.checks, &run.config);
        for c in &run.checks {
            match report.skipped.iter().find(|s| s.check == c.id) {
                Some(s) => {
                    let e = ex.skipped.entry(c.id).or_insert_with(|| (0, s.reason.clone()));
                    e.0 += 1;
                }
                None => *ex.ran.entry(c.id).or_default() += 1,
            }
        }
        ex.truncated = report.truncated.iter().fold(ex.truncated, |n, t| n.saturating_add(t.dropped));
        places.insert((g.name.clone(), layer.id.clone()), (gi, li, layer.master.clone()));
        findings.extend(report.findings);
    }
    let findings = if cap { audit::collapse(findings, run.max_results) } else { findings };
    let mut hits: Vec<Hit> = findings
        .into_iter()
        .filter(|f| f.severity <= run.min_severity)
        .map(|f| {
            let (glyph, layer, master) = places.get(&(f.location.glyph.clone(), f.location.layer.clone())).cloned().unwrap_or((0, 0, String::new()));
            Hit { finding: f, glyph, layer, master }
        })
        .collect();
    hits.sort_by(|a, b| (a.finding.severity, a.finding.check, a.glyph, a.layer).cmp(&(b.finding.severity, b.finding.check, b.glyph, b.layer)));
    if cap && hits.len() > MAX_REPORTED {
        ex.truncated = ex.truncated.saturating_add(hits.len() - MAX_REPORTED);
        hits.truncate(MAX_REPORTED);
    }
    ex.hits = hits;
    ex
}

// ── The result document ─────────────────────────────────────────────────────────────────────────

/// How a result names itself to `audit.fix`.
#[derive(Clone, Copy, PartialEq)]
enum FixParams {
    /// `{results: [id]}`: the document is kept for `audit.fix`.
    ById,
    /// `{checks, glyphs, layers}`: the document is not kept.
    BySelection,
}

fn fix_json(f: &Finding, id: &str, how: FixParams) -> Value {
    let Some(fx) = f.fix else { return json!({"available": false}) };
    let mut m = Map::new();
    let via = fx.command.filter(|c| find_command(c).is_some());
    m.insert("available".into(), json!(via.is_some()));
    m.insert("id".into(), json!(fx.id));
    m.insert("safe".into(), json!(fx.safe));
    m.insert("description".into(), json!(fx.description));
    match via {
        Some(via) => {
            let params = match how {
                FixParams::ById => json!({"results": [id]}),
                FixParams::BySelection => json!({"checks": [f.check], "glyphs": [f.location.glyph], "layers": [f.location.layer]}),
            };
            m.insert("command".into(), json!("audit.fix"));
            m.insert("params".into(), params);
            m.insert("via".into(), json!(via));
        }
        None => {
            m.insert("reason".into(), json!("no command carries this fix out yet"));
        }
    }
    Value::Object(m)
}

fn result_json(hit: &Hit, id: &str, how: FixParams) -> Result<Value> {
    let f = &hit.finding;
    let mut location = serde_json::to_value(&f.location).map_err(|e| EngineError::Internal(e.to_string()))?;
    if let Some(o) = location.as_object_mut() {
        o.insert("font".into(), json!("f0"));
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
        "ignored": false,
        "stale": false,
    }))
}

/// The id a result gets: its place in the sorted list.
fn result_id(i: usize) -> String {
    format!("r{}", i.saturating_add(1))
}

/// Builds the result document of `docs/spec/21-qa.md` § 4.
fn document(doc: &DocState, run: &Run, ex: &Examined, how: FixParams) -> Result<Value> {
    let mut results = Vec::with_capacity(ex.hits.len());
    let mut per_check: HashMap<&str, (usize, Severity)> = HashMap::new();
    let mut counts = [0usize; 3];
    for (i, hit) in ex.hits.iter().enumerate() {
        results.push(result_json(hit, &result_id(i), how)?);
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
        let ran = ex.ran.get(c.id).copied().unwrap_or(0);
        let skip = ex.skipped.get(c.id);
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
    let [fail, warn, info] = counts;
    Ok(json!({
        "schema": SCHEMA,
        "tool": {"name": "fontcraft", "version": env!("CARGO_PKG_VERSION")},
        "target": {
            "kind": "source",
            "path": doc.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "fonts": [{"id": "f0", "name": doc.font.family_name, "path": doc.path.as_ref().map(|p| p.to_string_lossy().into_owned()), "instance": null, "format": "source"}],
        },
        "document": {"title": doc.title, "revision": doc.revision},
        "profile": run.profile.as_str(),
        "config": run.config_json,
        "summary": {"fail": fail, "warn": warn, "info": info, "passed": passed, "skipped": skipped, "ignored": 0, "errors": 0, "truncated": ex.truncated},
        "checks": checks,
        "results": results,
    }))
}

// ── The kept runs ───────────────────────────────────────────────────────────────────────────────
//
// The session has no slot for audit results, so the last run of each document is kept here,
// keyed by the document's id and its saved font (unique while the document is open, also across
// sessions). Saving the document starts afresh.

/// Where a kept result points.
#[derive(Clone)]
struct ResultRef {
    id: String,
    check: &'static str,
    glyph: String,
    layer: String,
}

struct Kept {
    doc: u64,
    saved: Weak<Font>,
    /// The font the run examined, to tell which glyphs changed since.
    font: Arc<Font>,
    report: Value,
    refs: Vec<ResultRef>,
}

static KEPT: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

fn same_doc(k: &Kept, d: &DocState) -> bool {
    k.doc == d.id && k.saved.upgrade().is_some_and(|s| Arc::ptr_eq(&s, &d.saved))
}

fn keep(d: &DocState, report: Value, refs: Vec<ResultRef>) {
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    kept.retain(|k| k.saved.strong_count() > 0 && !same_doc(k, d));
    if kept.len() >= MAX_CACHED {
        let extra = kept.len() + 1 - MAX_CACHED;
        kept.drain(..extra);
    }
    kept.push(Kept { doc: d.id, saved: Arc::downgrade(&d.saved), font: d.font.clone(), report, refs });
}

fn kept_run(d: &DocState) -> Option<(Arc<Font>, Value, Vec<ResultRef>)> {
    let kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    kept.iter().find(|k| same_doc(k, d)).map(|k| (k.font.clone(), k.report.clone(), k.refs.clone()))
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
    let doc = s.doc()?;
    let run = read_run(&p, &doc.font)?;
    let ex = examine(&doc.font, &run, true);
    let report = document(doc, &run, &ex, FixParams::ById)?;
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
    keep(doc, report.clone(), refs);
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
    p.bool_or("includeIgnored", false)?;
    let doc = s.doc()?;
    let (then, mut report, _) = kept_run(doc).ok_or_else(|| p.bad(NO_RUN))?;
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
        let stale = !glyph.is_empty() && *changed.entry(glyph.clone()).or_insert_with(|| glyph_changed(&then, &doc.font, &glyph));
        if let Some(o) = r.as_object_mut() {
            o.insert("stale".into(), json!(stale));
        }
        stale_count += usize::from(stale);
        if let Some(n) = counts.get_mut(severity as usize) {
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
    document_target(&p)?;
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
    let (mut run, groups) = match p.opt_array("results")? {
        Some(ids) => {
            if let Some(key) = FILTER_KEYS.iter().find(|k| p.get(k).is_some()) {
                return Err(p.bad(format!("give `results` or the filters (`{key}`…), not both")));
            }
            if ids.len() > MAX_RESULT_IDS {
                return Err(p.bad(format!("`results` holds {} ids; at most {MAX_RESULT_IDS} are allowed", ids.len())));
            }
            let ids = p.strings("results")?;
            let mut run = read_run(&p, s.font()?)?;
            let groups = groups_from_ids(s, &p, &ids, &mut run, &mut report)?;
            (run, groups)
        }
        None => {
            let doc = s.doc()?;
            let run = read_run(&p, &doc.font)?;
            // Every finding counts here: a check over the result cap is still fixed.
            let ex = examine(&doc.font, &run, false);
            let groups = groups_from_hits(&doc.font, &ex.hits);
            (run, groups)
        }
    };
    let touched: HashSet<String> = groups.iter().map(|g| g.glyph.clone()).collect();
    let facts = {
        let ctx = AuditContext::new(s.font()?);
        (ctx.grid_step, ctx.main_curve, ctx.variable)
    };
    let how = How { safe_only, dry, backup, facts };
    one_step(s, |s| {
        for group in &groups {
            fix_layer(s, &run, group, &how, &mut report)?;
        }
        Ok(())
    })?;
    // `remaining`: the same checks on the glyphs involved, as they are now.
    run.glyphs = Some(touched);
    let doc = s.doc()?;
    let ex = examine(&doc.font, &run, true);
    let remaining = document(doc, &run, &ex, FixParams::BySelection)?;
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
fn groups_from_ids(s: &Session, p: &Params, ids: &[&str], run: &mut Run, report: &mut FixReport) -> Result<Vec<Group>> {
    let doc = s.doc()?;
    let (then, _, refs) = kept_run(doc).ok_or_else(|| p.bad(NO_RUN))?;
    let index: HashMap<&str, &ResultRef> = refs.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut groups: BTreeMap<(usize, usize), Group> = BTreeMap::new();
    let mut checks: Vec<&'static CheckSpec> = Vec::new();
    for id in ids {
        let r = index.get(id).ok_or_else(|| p.bad(format!("`results`: the last audit has no result `{id}`")))?;
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
        let place = doc
            .font
            .glyphs
            .as_arcs()
            .iter()
            .enumerate()
            .find(|(_, g)| g.name == r.glyph)
            .and_then(|(gi, g)| g.layers.iter().enumerate().find(|(_, l)| l.id == r.layer).map(|(li, l)| (gi, li, l.master.clone())));
        let Some((gi, li, master)) = place else {
            report.skip(Some(&r.glyph), Some(&r.layer), Some(r.check), None, format!("{id}: the layer no longer exists"));
            continue;
        };
        let Some(spec) = audit::find(r.check) else { continue };
        if !checks.iter().any(|c| c.id == spec.id) {
            checks.push(spec);
        }
        groups
            .entry((gi, li))
            .or_insert_with(|| Group { glyph: r.glyph.clone(), layer: r.layer.clone(), master, checks: BTreeSet::new() })
            .checks
            .insert(spec.id);
    }
    run.checks = CHECKS.iter().filter(|c| checks.iter().any(|k| k.id == c.id)).collect();
    run.layers = Some(groups.values().map(|g| g.layer.clone()).collect());
    Ok(groups.into_values().collect())
}

/// The layers with findings that have a fix, in font order, each with the checks that reported
/// it. Findings without a fix stay in `remaining` and are not listed as skipped.
fn groups_from_hits(font: &Font, hits: &[Hit]) -> Vec<Group> {
    let mut groups: BTreeMap<(usize, usize), Group> = BTreeMap::new();
    for h in hits.iter().filter(|h| !h.finding.location.glyph.is_empty() && h.finding.fix.is_some()) {
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
            })
            .checks
            .insert(h.finding.check);
    }
    groups.into_values().collect()
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
fn calls_for(
    fix: &str,
    command: &'static str,
    glyph: &Glyph,
    layer: &Layer,
    findings: &[Finding],
    report: &mut FixReport,
) -> Vec<(&'static str, Value)> {
    let (g, l) = (glyph.name.as_str(), layer.id.as_str());
    let whole = || vec![(command, json!({"glyph": g, "layer": l}))];
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
        "tidyPaths" | "addExtremes" | "addInflections" => some(paths_of(layer, findings, |_| true), None),
        "removeOverlap" => some(paths_of(layer, findings, |_| true), Some(("keepOpenPaths", json!(true)))),
        "correctPathDirection" | "roundCoordinates" => whole(),
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
        _ => Vec::new(),
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
    let Some((glyph, layer)) = find_layer(font, g, l) else { return Ok(()) };
    let mut fixes: Vec<&'static CheckSpec> = Vec::new();
    for &id in &group.checks {
        let Some(spec) = run.checks.iter().find(|c| c.id == id) else { continue };
        let reason = match spec.fix {
            None => Some("the check has no automatic fix"),
            Some(fx) if !fx.command.is_some_and(|c| find_command(c).is_some()) => Some("no command carries this fix out yet"),
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
    if safe_only && layer.shapes.iter().any(|x| matches!(x, Shape::Component(_))) {
        report.skip(Some(g), Some(l), None, None, "the layer has components: fix it with safeOnly: false");
        return Ok(());
    }
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
                let ctx = context(font, facts);
                let found = audit::audit_layer(&ctx, glyph, layer, &checks, &run.config).findings;
                if found.is_empty() {
                    continue;
                }
                let calls = calls_for(fix, command, glyph, layer, &found, report);
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

/// An audit context for `font` with the font-wide facts of the run, so a step does not survey the
/// whole font again.
fn context(font: &Font, (grid_step, main_curve, variable): Facts) -> AuditContext<'_> {
    AuditContext { font, grid_step, main_curve, variable }
}

// ── audit.list, audit.describe, audit.profile.list ─────────────────────────────────────────────

fn fix_available(c: &CheckSpec) -> bool {
    c.fix.and_then(|f| f.command).is_some_and(|cmd| find_command(cmd).is_some())
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
        .filter(|_| scopes.as_ref().is_none_or(|s| s.contains(&CHECK_SCOPE)))
        .filter(|c| fixable.is_none_or(|f| f == c.fix.is_some()))
        .map(|c| {
            json!({
                "id": c.id,
                "label": c.label,
                "group": group_of(c.id),
                "severity": c.severity.as_str(),
                "scope": CHECK_SCOPE,
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
        "scope": CHECK_SCOPE,
        "profile": c.profile.as_str(),
        "profiles": profiles,
        "thresholds": thresholds,
        "fix": fix,
        "preconditions": preconditions,
        "capped": c.capped,
    }))
}

fn profile_list(_: &mut Session, _: &Value) -> Result<Value> {
    let about = |p: Profile| match p {
        Profile::Outlines => "Per-glyph outline checks of a source; fast enough to run after every edit",
        Profile::Opentype => "Conformance of a compiled font with the OpenType specification",
        Profile::Universal => "Outlines and OpenType, plus practices that hold for any professional font",
        Profile::Strict => "Every check, house-style and heuristic ones included",
    };
    let profiles: Vec<Value> = Profile::ALL
        .into_iter()
        .map(|p| {
            json!({
                "id": p.as_str(),
                "checks": CHECKS.iter().filter(|c| p.includes(c.profile)).count(),
                "builtIn": true,
                "default": p == Profile::Universal,
                "description": about(p),
            })
        })
        .collect();
    Ok(json!({ "profiles": profiles }))
}
