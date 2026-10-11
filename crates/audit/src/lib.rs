//! FontCraft font audit (L2): the check registry and the checks.
//!
//! A check is a pure function from a target to a list of [`Finding`]s. Its [`CheckScope`] says
//! which target: one layer ([`audit_layer`]), every master of a glyph ([`audit_glyph`]), the
//! source ([`audit_source`]), one compiled font ([`audit_binary`]) or several ([`audit_family`]).
//! What a check cannot compute at this layer (an in-memory compile, a TTX round trip, shaping,
//! the master-compatibility report, rasterising) it asks of [`AuditServices`], which the engine
//! implements; an unavailable service makes the check skip with the reason.
//! [`CHECKS`] lists every check with its id, label, severity, home profile,
//! thresholds and fix. The `audit.*` engine commands pick checks with [`select`], run them with
//! [`audit_layer`], apply the result cap with [`collapse`] and build the result document of
//! `docs/spec/21-qa.md` § 4 from the findings. Checks never change the font: a fix is carried out
//! by the engine command its [`FixSpec`] names.
//!
//! Addresses follow decision 0004 ("How commands name their targets"): `shapes` are indices in
//! the layer's shape list, `points` are `[shape, node]` pairs (off-curve nodes included) and
//! `segments` name a segment by the `[shape, node]` address of the on-curve node that ends it.
#![forbid(unsafe_code)]

mod checks;
mod context;
mod contour;
mod contours;
pub mod data;
mod handles;
mod metrics;
mod points;
pub mod profile;
mod registry;
mod scope;
pub mod services;

use std::collections::BTreeMap;

use fontcraft_geom::{Point, Rect};
use fontcraft_model::{Font, Glyph, Layer, Master, NodeKind, Shape};
use serde::Serialize;

pub use checks::compat::structural_report;
pub use context::{BinaryCtx, FamilyCtx, FontFile, GlyphCtx, SourceCtx, SourceMap};
pub use registry::CHECKS;
pub use scope::{CheckGroup, CheckScope, Precondition};
pub use services::{AuditServices, CompatibilityReport, NoServices, Unavailable};

/// Most nodes (on- and off-curve, every path of the layer together) a layer may have to be
/// audited. Far beyond a drawn glyph; a larger layer is skipped by every check with a reason.
pub const MAX_LAYER_NODES: usize = 20_000;
/// Most shapes a layer may have to be audited.
pub const MAX_LAYER_SHAPES: usize = 4_096;
/// Most findings one check keeps for one layer. The rest are counted in
/// [`LayerReport::truncated`].
pub const MAX_FINDINGS_PER_CHECK: usize = 1_000;
/// Default of the `maxResults` run option (`audit.resultCap`).
pub const DEFAULT_MAX_RESULTS: usize = 100;
/// Most nodes the font-wide survey (the font's main curve type) reads.
pub const MAX_SURVEY_NODES: usize = 5_000_000;
/// Grid steps below this many units count as no grid: coordinates that fine are not rounded.
pub const MIN_GRID_STEP: f64 = 1.0 / 1024.0;

/// How serious a finding is. Sorted most serious first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Fail,
    Warn,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Fail => "fail",
            Severity::Warn => "warn",
            Severity::Info => "info",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fail" => Some(Severity::Fail),
            "warn" => Some(Severity::Warn),
            "info" => Some(Severity::Info),
            _ => None,
        }
    }
}

/// The built-in profiles (`docs/spec/21-qa.md` § 2). Each check names the smallest one that
/// contains it, its home profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Outlines,
    Opentype,
    Universal,
    Strict,
}

impl Profile {
    pub const ALL: [Profile; 4] = [Profile::Outlines, Profile::Opentype, Profile::Universal, Profile::Strict];

    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Outlines => "outlines",
            Profile::Opentype => "opentype",
            Profile::Universal => "universal",
            Profile::Strict => "strict",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Profile::ALL.into_iter().find(|p| p.as_str() == s)
    }

    /// Does this profile run the checks whose home profile is `home`? `outlines` and `opentype`
    /// are disjoint, `universal` holds both, `strict` holds everything.
    pub fn includes(self, home: Profile) -> bool {
        match self {
            Profile::Outlines => home == Profile::Outlines,
            Profile::Opentype => home == Profile::Opentype,
            Profile::Universal => home != Profile::Strict,
            Profile::Strict => true,
        }
    }
}

/// The kind of an outline check of the layer scope, which is also the module it lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckFamily {
    PointsAndSegments,
    Contours,
    HandlesAndCurvature,
    MetricsAndGrid,
}

/// A named number a check reads, with its default and the range a run may set it to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Threshold {
    pub name: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub doc: &'static str,
}

/// The repair attached to a check (`docs/spec/21-qa.md` § 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FixSpec {
    /// The fix id of § 5.
    pub id: &'static str,
    /// The engine command that carries the fix out. `None`: no command exists yet, so the
    /// result is report-only.
    pub command: Option<&'static str>,
    /// The fix does not change the rendered shape, the metrics or the names users see.
    pub safe: bool,
    pub description: &'static str,
}

pub(crate) type CheckFn = fn(&LayerInput<'_>, &Params<'_>, &mut Sink);
pub(crate) type GlyphFn = fn(&GlyphCtx<'_>, &Params<'_>, &mut Sink);
pub(crate) type SourceFn = fn(&SourceCtx<'_>, &Params<'_>, &mut Sink);
pub(crate) type BinaryFn = fn(&BinaryCtx<'_>, &Params<'_>, &mut Sink);
pub(crate) type FamilyFn = fn(&FamilyCtx<'_>, &Params<'_>, &mut Sink);

/// A check's code, one function type per scope.
#[derive(Clone, Copy)]
#[allow(dead_code)] // The registry files of M12.46–M12.58 add checks of the other scopes.
pub(crate) enum Runner {
    Layer(CheckFn),
    Glyph(GlyphFn),
    Source(SourceFn),
    Binary(BinaryFn),
    Family(FamilyFn),
}

/// One check of the registry.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub group: CheckGroup,
    /// What the runner reads; matches [`CheckSpec::run`].
    pub scope: CheckScope,
    /// The kind of an outline check of the layer scope; `None` for every other check.
    pub family: Option<CheckFamily>,
    /// The most severe result the check emits by default.
    pub severity: Severity,
    /// The home profile.
    pub profile: Profile,
    pub thresholds: &'static [Threshold],
    pub fix: Option<FixSpec>,
    /// Not run on a source with more than one master, where the heuristic misfires on outlines
    /// that must stay compatible.
    pub skip_variable: bool,
    /// Heuristic: more than `maxResults` results in one font collapse into one summary
    /// (`audit.resultCap`, [`collapse`]).
    pub capped: bool,
    /// The message codes the check emits, besides `too-many-results` when it is capped.
    pub codes: &'static [&'static str],
    /// Message codes whose default severity differs from [`CheckSpec::severity`]
    /// (`audit.severityOverrides`); every other code has the check's severity.
    pub code_severities: &'static [(&'static str, Severity)],
    /// Facts that must hold for the check to run (`audit.preconditions`).
    pub preconditions: &'static [Precondition],
    #[serde(skip)]
    pub(crate) run: Runner,
}

impl std::fmt::Debug for CheckSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckSpec").field("id", &self.id).field("severity", &self.severity).field("profile", &self.profile).finish()
    }
}

impl CheckSpec {
    pub fn threshold(&self, name: &str) -> Option<&'static Threshold> {
        self.thresholds.iter().find(|t| t.name == name)
    }

    /// The default severity of one of the check's message codes.
    pub fn code_severity(&self, code: &str) -> Severity {
        self.code_severities.iter().find(|(c, _)| *c == code).map(|(_, s)| *s).unwrap_or(self.severity)
    }
}

/// The code of the one result a capped check collapses into (`audit.resultCap`).
pub const TOO_MANY_RESULTS: &str = "too-many-results";

/// The check with this id.
pub fn find(id: &str) -> Option<&'static CheckSpec> {
    CHECKS.iter().find(|c| c.id == id)
}

/// Does `id` match `pattern`: the same text, or a prefix followed by `*` (`check.outlines.*`)?
pub fn matches_pattern(id: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => id.starts_with(prefix),
        None => id == pattern,
    }
}

/// The checks a run selects, in registry order: those of `profile` (every check when `None`),
/// narrowed to the ids matching one of `patterns` when any are given, minus the ids matching one
/// of `exclude`.
pub fn select(profile: Option<Profile>, patterns: &[&str], exclude: &[&str]) -> Vec<&'static CheckSpec> {
    CHECKS
        .iter()
        .filter(|c| profile.is_none_or(|p| p.includes(c.profile)))
        .filter(|c| patterns.is_empty() || patterns.iter().any(|p| matches_pattern(c.id, p)))
        .filter(|c| !exclude.iter().any(|p| matches_pattern(c.id, p)))
        .collect()
}

/// Threshold values set for a run, per check id and threshold name. Unset values take the
/// registry's defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    thresholds: BTreeMap<String, BTreeMap<String, f64>>,
}

impl Config {
    /// Sets one threshold. Fails, changing nothing, on an unknown check or threshold, or on a
    /// value that is not a number within the threshold's range.
    pub fn set(&mut self, check: &str, name: &str, value: f64) -> Result<(), String> {
        let spec = find(check).ok_or_else(|| format!("there is no check `{check}`"))?;
        let t = spec.threshold(name).ok_or_else(|| format!("check `{check}` has no threshold `{name}`"))?;
        if !value.is_finite() || value < t.min || value > t.max {
            return Err(format!("`{name}` of `{check}` must be a number from {} to {}", t.min, t.max));
        }
        self.thresholds.entry(check.to_string()).or_default().insert(name.to_string(), value);
        Ok(())
    }

    /// The value set for one threshold, if any.
    pub fn get(&self, check: &str, name: &str) -> Option<f64> {
        self.thresholds.get(check).and_then(|m| m.get(name)).copied()
    }
}

/// The thresholds one check runs with.
pub(crate) struct Params<'a> {
    overrides: Option<&'a BTreeMap<String, f64>>,
}

impl Params<'_> {
    pub(crate) fn get(&self, t: &Threshold) -> f64 {
        self.overrides.and_then(|m| m.get(t.name)).copied().filter(|v| v.is_finite()).map(|v| v.max(t.min).min(t.max)).unwrap_or(t.default)
    }
}

/// The curve type of a contour or a font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CurveType {
    Cubic,
    Quadratic,
}

/// Font-wide facts the checks need, computed once per run.
pub struct AuditContext<'a> {
    pub font: &'a Font,
    /// The grid step in font units (spacing ÷ subdivision); `None` when coordinates are not
    /// rounded (spacing 0).
    pub grid_step: Option<f64>,
    /// The curve type most of the font's curve nodes use; `None` when it has no curves.
    pub main_curve: Option<CurveType>,
    /// The source has more than one master.
    pub variable: bool,
}

impl<'a> AuditContext<'a> {
    pub fn new(font: &'a Font) -> Self {
        let settings = &font.info.settings;
        let length = settings.grid_length.unwrap_or(1.0);
        let sub = settings.grid_sub_division.filter(|s| s.is_finite() && *s >= 1.0).unwrap_or(1.0);
        let step = length / sub;
        let grid_step = (length.is_finite() && step.is_finite() && step >= MIN_GRID_STEP).then_some(step);
        let (mut cubic, mut quad, mut seen) = (0usize, 0usize, 0usize);
        'survey: for g in font.glyphs.as_arcs() {
            for layer in &g.layers {
                for shape in &layer.shapes {
                    let Shape::Path(p) = shape else { continue };
                    for n in &p.nodes {
                        seen += 1;
                        if seen > MAX_SURVEY_NODES {
                            break 'survey;
                        }
                        match n.kind {
                            NodeKind::Curve => cubic += 1,
                            NodeKind::QCurve => quad += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
        let main_curve = match (cubic, quad) {
            (0, 0) => None,
            (c, q) if q > c => Some(CurveType::Quadratic),
            _ => Some(CurveType::Cubic),
        };
        Self { font, grid_step, main_curve, variable: font.masters.len() > 1 }
    }
}

/// Where a finding is. Only the keys that apply are set (`docs/spec/21-qa.md` § 4): glyph-level
/// keys for source and glyph findings, table-level keys for compiled fonts, `fonts` for family
/// findings, `masters` for interpolation findings.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub glyph: String,
    pub layer: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[usize; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<[usize; 2]>,
    /// The spot a marker points at, in font units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<[f64; 2]>,
    /// `[x0, y0, x1, y1]` in font units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
    /// The font's id in the result document (`f0`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The master id, when it differs from the layer id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master: Option<String>,
    /// Component indices in the layer's shape list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<usize>,
    /// Anchor names.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<String>,
    /// Table tag text (`OS/2`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Field name in the table (`usWinAscent`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_id: Option<u16>,
    /// Name record platform.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<u16>,
    /// Name record language id as decimal text, or a layout language tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// Lookup index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup: Option<usize>,
    /// Axis tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    /// Named instance or source instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// A kerning pair, two glyph names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<[String; 2]>,
    /// Ids of every font a family finding involves.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<String>,
    /// The two masters an interpolation finding compares.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub masters: Vec<String>,
    /// The Font Info field or custom parameter that produces a compiled value, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_field: Option<String>,
}

impl Location {
    /// Takes every key `other` sets, keeping the others.
    pub fn merge(&mut self, other: Location) {
        fn take<T>(slot: &mut Option<T>, v: Option<T>) {
            if v.is_some() {
                *slot = v;
            }
        }
        fn take_vec<T>(slot: &mut Vec<T>, v: Vec<T>) {
            if !v.is_empty() {
                *slot = v;
            }
        }
        let Location {
            glyph,
            layer,
            shapes,
            points,
            segments,
            point,
            bounds,
            font,
            master,
            components,
            anchors,
            table,
            field,
            name_id,
            platform,
            language,
            feature,
            script,
            lookup,
            axis,
            instance,
            pair,
            fonts,
            masters,
            source_field,
        } = other;
        if !glyph.is_empty() {
            self.glyph = glyph;
        }
        if !layer.is_empty() {
            self.layer = layer;
        }
        take_vec(&mut self.shapes, shapes);
        take_vec(&mut self.points, points);
        take_vec(&mut self.segments, segments);
        take(&mut self.point, point);
        take(&mut self.bounds, bounds);
        take(&mut self.font, font);
        take(&mut self.master, master);
        take_vec(&mut self.components, components);
        take_vec(&mut self.anchors, anchors);
        take(&mut self.table, table);
        take(&mut self.field, field);
        take(&mut self.name_id, name_id);
        take(&mut self.platform, platform);
        take(&mut self.language, language);
        take(&mut self.feature, feature);
        take(&mut self.script, script);
        take(&mut self.lookup, lookup);
        take(&mut self.axis, axis);
        take(&mut self.instance, instance);
        take(&mut self.pair, pair);
        take_vec(&mut self.fonts, fonts);
        take_vec(&mut self.masters, masters);
        take(&mut self.source_field, source_field);
    }
}

/// One thing a check reports.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub check: &'static str,
    /// Which of the check's messages this is (kebab-case).
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub location: Location,
    /// The numbers behind the message, for agents.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<&'static str, f64>,
    /// The repair for this finding; `None` when the check has none or this finding cannot be
    /// repaired automatically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<FixSpec>,
}

/// A check that did not run on a layer, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Skipped {
    pub check: &'static str,
    pub reason: String,
}

/// A check that found more than [`MAX_FINDINGS_PER_CHECK`] things on one layer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Truncated {
    pub check: &'static str,
    pub dropped: usize,
}

/// What a run of checks found on one layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LayerReport {
    pub findings: Vec<Finding>,
    pub skipped: Vec<Skipped>,
    pub truncated: Vec<Truncated>,
}

/// What a check reads: the layer, its glyph and font, and its paths as segments.
pub(crate) struct LayerInput<'a> {
    pub ctx: &'a AuditContext<'a>,
    pub glyph: &'a Glyph,
    pub layer: &'a Layer,
    pub master: Option<&'a Master>,
    pub contours: Vec<contour::Contour<'a>>,
}

impl<'a> LayerInput<'a> {
    /// The contours whose coordinates are all usable numbers.
    pub(crate) fn finite(&self) -> impl Iterator<Item = &contour::Contour<'a>> {
        self.contours.iter().filter(|c| c.finite)
    }
}

/// Runs `checks` on one layer of `glyph`. Never fails: a check that cannot run on this layer is
/// listed in [`LayerReport::skipped`].
pub fn audit_layer(ctx: &AuditContext<'_>, glyph: &Glyph, layer: &Layer, checks: &[&'static CheckSpec], config: &Config) -> LayerReport {
    let mut report = LayerReport::default();
    let nodes = layer
        .shapes
        .iter()
        .map(|s| match s {
            Shape::Path(p) => p.nodes.len(),
            _ => 1,
        })
        .fold(0usize, usize::saturating_add);
    if layer.shapes.len() > MAX_LAYER_SHAPES || nodes > MAX_LAYER_NODES {
        let reason = format!("the layer has more than {MAX_LAYER_SHAPES} shapes or {MAX_LAYER_NODES} nodes");
        report.skipped = checks.iter().map(|c| Skipped { check: c.id, reason: reason.clone() }).collect();
        return report;
    }
    let input = LayerInput { ctx, glyph, layer, master: ctx.font.master(&layer.master).ok(), contours: contour::contours(layer) };
    for &spec in checks {
        if spec.skip_variable && ctx.variable {
            report.skipped.push(Skipped { check: spec.id, reason: "the source has more than one master".into() });
            continue;
        }
        let Runner::Layer(run) = spec.run else { continue };
        let params = Params { overrides: config.thresholds.get(spec.id) };
        let mut sink = Sink::new(spec, &glyph.name, &layer.id);
        run(&input, &params, &mut sink);
        sink.finish(&mut report);
    }
    report
}

/// Runs the glyph-scope checks among `checks` on every master of one glyph. Checks of other
/// scopes are left out.
pub fn audit_glyph(ctx: &GlyphCtx<'_>, checks: &[&'static CheckSpec], config: &Config) -> LayerReport {
    let base = Location { glyph: ctx.glyph.name.clone(), ..Location::default() };
    run_scope(checks, config, &base, |run, params, sink| match run {
        Runner::Glyph(f) => {
            f(ctx, params, sink);
            true
        }
        _ => false,
    })
}

/// Runs the source-scope checks among `checks` on the whole source.
pub fn audit_source(ctx: &SourceCtx<'_>, checks: &[&'static CheckSpec], config: &Config) -> LayerReport {
    run_scope(checks, config, &Location::default(), |run, params, sink| match run {
        Runner::Source(f) => {
            f(ctx, params, sink);
            true
        }
        _ => false,
    })
}

/// Runs the binary-scope checks among `checks` on one compiled font. Findings carry the font's
/// id. Preconditions are the caller's to evaluate (M12.44's runner).
pub fn audit_binary(ctx: &BinaryCtx<'_>, checks: &[&'static CheckSpec], config: &Config) -> LayerReport {
    let base = Location { font: Some(ctx.id.to_string()), ..Location::default() };
    run_scope(checks, config, &base, |run, params, sink| match run {
        Runner::Binary(f) => {
            f(ctx, params, sink);
            true
        }
        _ => false,
    })
}

/// Runs the family-scope checks among `checks` on several compiled fonts. Findings carry the ids
/// of every font in `fonts` unless the check names fewer.
pub fn audit_family(ctx: &FamilyCtx<'_>, checks: &[&'static CheckSpec], config: &Config) -> LayerReport {
    let base = Location { fonts: ctx.fonts.iter().map(|f| f.id.to_string()).collect(), ..Location::default() };
    run_scope(checks, config, &base, |run, params, sink| match run {
        Runner::Family(f) => {
            f(ctx, params, sink);
            true
        }
        _ => false,
    })
}

/// Runs every check `call` accepts (it returns false for a runner of another scope) with a sink
/// whose findings start from `base`.
fn run_scope(
    checks: &[&'static CheckSpec],
    config: &Config,
    base: &Location,
    mut call: impl FnMut(&Runner, &Params<'_>, &mut Sink) -> bool,
) -> LayerReport {
    let mut report = LayerReport::default();
    for &spec in checks {
        let params = Params { overrides: config.thresholds.get(spec.id) };
        let mut sink = Sink::at(spec, base.clone());
        if call(&spec.run, &params, &mut sink) {
            sink.finish(&mut report);
        }
    }
    report
}

/// Applies `audit.resultCap` to the findings of one font: every capped check with more than
/// `max_results` findings keeps none of them and gets one informational summary with the count
/// in their place.
pub fn collapse(findings: Vec<Finding>, max_results: usize) -> Vec<Finding> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for f in &findings {
        *counts.entry(f.check).or_default() += 1;
    }
    let over = |check: &str| find(check).is_some_and(|s| s.capped) && counts.get(check).is_some_and(|n| *n > max_results);
    let mut out = Vec::with_capacity(findings.len().min(max_results.saturating_mul(4)));
    let mut summarised: Vec<&'static str> = Vec::new();
    for f in findings {
        if !over(f.check) {
            out.push(f);
            continue;
        }
        if summarised.contains(&f.check) {
            continue;
        }
        summarised.push(f.check);
        let n = counts.get(f.check).copied().unwrap_or(0);
        out.push(Finding {
            check: f.check,
            code: TOO_MANY_RESULTS,
            severity: Severity::Info,
            message: format!("{n} results, more than {max_results}: this check probably does not suit the design."),
            location: Location::default(),
            data: BTreeMap::from([("count", n as f64)]),
            fix: None,
        });
    }
    out
}

/// A finding under construction, before the check and the layer are filled in.
pub(crate) struct Hit {
    code: &'static str,
    message: String,
    severity: Option<Severity>,
    shapes: Vec<usize>,
    points: Vec<[usize; 2]>,
    segments: Vec<[usize; 2]>,
    point: Option<Point>,
    bounds: Option<Rect>,
    data: Vec<(&'static str, f64)>,
    no_fix: bool,
    place: Option<Location>,
}

impl Hit {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            severity: None,
            shapes: Vec::new(),
            points: Vec::new(),
            segments: Vec::new(),
            point: None,
            bounds: None,
            data: Vec::new(),
            no_fix: false,
            place: None,
        }
    }
    pub(crate) fn severity(mut self, s: Severity) -> Self {
        self.severity = Some(s);
        self
    }
    pub(crate) fn shapes(mut self, v: Vec<usize>) -> Self {
        self.shapes = v;
        self
    }
    pub(crate) fn points(mut self, v: Vec<[usize; 2]>) -> Self {
        self.points = v;
        self
    }
    pub(crate) fn segments(mut self, v: Vec<[usize; 2]>) -> Self {
        self.segments = v;
        self
    }
    pub(crate) fn at(mut self, p: Point) -> Self {
        self.point = Some(p);
        self
    }
    pub(crate) fn bounds(mut self, r: Rect) -> Self {
        self.bounds = Some(r);
        self
    }
    pub(crate) fn data(mut self, key: &'static str, value: f64) -> Self {
        self.data.push((key, value));
        self
    }
    pub(crate) fn without_fix(mut self) -> Self {
        self.no_fix = true;
        self
    }
    /// Sets the location keys `place` sets (table, field, fonts…) on top of the sink's base.
    #[allow(dead_code)] // M12.46–M12.58: the binary and family checks set table-level keys.
    pub(crate) fn place(mut self, place: Location) -> Self {
        self.place = Some(place);
        self
    }
}

/// Collects one check's findings on one layer, up to [`MAX_FINDINGS_PER_CHECK`].
pub(crate) struct Sink {
    spec: &'static CheckSpec,
    base: Location,
    findings: Vec<Finding>,
    dropped: usize,
    skipped: Option<String>,
}

impl Sink {
    fn new(spec: &'static CheckSpec, glyph: &str, layer: &str) -> Self {
        Self::at(spec, Location { glyph: glyph.to_string(), layer: layer.to_string(), ..Location::default() })
    }

    fn at(spec: &'static CheckSpec, base: Location) -> Self {
        Self { spec, base, findings: Vec::new(), dropped: 0, skipped: None }
    }

    fn finish(self, report: &mut LayerReport) {
        if let Some(reason) = self.skipped {
            report.skipped.push(Skipped { check: self.spec.id, reason });
        }
        if self.dropped > 0 {
            report.truncated.push(Truncated { check: self.spec.id, dropped: self.dropped });
        }
        report.findings.extend(self.findings);
    }

    pub(crate) fn push(&mut self, hit: Hit) {
        if self.findings.len() >= MAX_FINDINGS_PER_CHECK {
            self.dropped += 1;
            return;
        }
        let mut location = Location {
            shapes: hit.shapes,
            points: hit.points,
            segments: hit.segments,
            point: hit.point.map(|p| [p.x, p.y]).filter(|v| all_finite(v)),
            bounds: hit.bounds.map(|r| [r.x0, r.y0, r.x1, r.y1]).filter(|v| all_finite(v)),
            ..self.base.clone()
        };
        if let Some(place) = hit.place {
            location.merge(place);
        }
        self.findings.push(Finding {
            check: self.spec.id,
            code: hit.code,
            severity: hit.severity.unwrap_or_else(|| self.spec.code_severity(hit.code)),
            message: hit.message,
            location,
            data: hit.data.into_iter().filter(|(_, v)| v.is_finite()).collect(),
            fix: if hit.no_fix { None } else { self.spec.fix },
        });
    }

    /// Records that the check could not run (or finish) on this layer.
    pub(crate) fn skip(&mut self, reason: impl Into<String>) {
        self.skipped = Some(reason.into());
    }
}

fn all_finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// A node, handle or path index as messages show it, counted from 1. Result addresses
/// (`points`, `shapes`, `segments`) stay 0-based.
pub(crate) struct Nth(pub usize);

impl std::fmt::Display for Nth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.saturating_add(1))
    }
}

/// A number for a message: whole numbers without decimals, others with up to two.
pub(crate) fn num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        // Adding zero turns -0 into 0.
        format!("{}", v.round() + 0.0)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests;
