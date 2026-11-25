//! What stops an export, found all at once: `file.exportCheck`, and the problems a failing export
//! command lists in its error. Rows: `file.exportCheck`, `glyphName.invalidBlocksExport`,
//! `compatibility.exportBlocked`, `compile.cmapConflict`.
//!
//! The compiler decides whether a font is written. A check only names every problem where the
//! compiler stops at its first: each job goes through the stages that decide the font's shape
//! ([`fontcraft_compile::plan`]), with its feature code, and writes nothing.
//!
//! A problem is `{kind, message, glyph?, layer?, character?, position?, codepoint?, glyphs?,
//! entry?, line?, column?, instances?, document?}`; `kind` is `glyphName`, `codepoint`,
//! `incompatible`, `features`, `glyph` or `font`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use fontcraft_compile::CompileError;
use fontcraft_features::params::apply_instance_params;
use fontcraft_features::{EntryDiagnostic, EntryKind, MasterValues};
use fontcraft_interp::{InterpError, Mismatch};
use fontcraft_model::glyphdata::{GlyphData, validate};
use fontcraft_model::{Features, Font, Target};
use serde_json::{Map, Value, json};

use super::compile::{self, Job, Report, Request, Sink};
use super::features_check::{CheckContext, compile_entries};
use super::features_compile::{entry_json, group, positions, snippet_of};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most problems one check lists.
pub const MAX_PROBLEMS: usize = 1_000;
/// Most problems the message of a failed export spells out.
const MAX_IN_MESSAGE: usize = 50;
/// Most glyphs a skipped instance lists.
const MAX_SKIPPED_GLYPHS: usize = 256;
/// Most jobs of one document a check looks at.
const MAX_JOBS: usize = 64;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "file.exportCheck",
        "Check Export",
        [],
        None,
        "{document?: title or path, tab?: \"otf\" (default)|\"variable\", params?: {…}} what would stop an export of the document (default: the active one) or keep one of its fonts from being written, all at once and without writing anything. tab and params name the export: the parameters of file.exportStatic (otf) or file.exportVariable (variable). Checked: the names of the exported glyphs, code points given to two glyphs, layers that are not compatible, the feature code of each font, and everything else that decides a font's glyphs and tables; outlines are not prepared. A failing export lists its errors the same way in its message → {tab, document, ok, errors: [{kind: \"glyphName\"|\"codepoint\"|\"incompatible\"|\"features\"|\"glyph\"|\"font\", message, glyph?, layer?, character?, position? (from 1), codepoint?, glyphs?, entry?: {kind, name, index?}, line?, column?, instances?: [name]}], more?: errors not listed, skipped: [{instance, source, reason, glyphs?: [{glyph, layer?, message}]}], warnings: [text], files: how many font files it would write (otf)}",
        has_doc,
        export_check
    )]
}

/// The problems found, each once.
#[derive(Default)]
pub(super) struct Problems {
    list: Vec<Map<String, Value>>,
    /// Problems past [`MAX_PROBLEMS`].
    more: usize,
    /// Where the problems of the document being checked start.
    from: usize,
}

impl Problems {
    pub(super) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn has(&self, kind: &str) -> bool {
        self.list.iter().any(|p| p.get("kind").and_then(Value::as_str) == Some(kind))
    }

    /// Adds a problem of `kind`, or `instance` to the problem already listed.
    fn add(&mut self, kind: &str, message: String, instance: Option<&str>, details: Value) {
        let mut problem = Map::new();
        problem.insert("kind".into(), json!(kind));
        problem.insert("message".into(), json!(message));
        if let Value::Object(details) = details {
            problem.extend(details.into_iter().filter(|(_, v)| !v.is_null()));
        }
        let same = |p: &Map<String, Value>| problem.iter().all(|(k, v)| p.get(k) == Some(v));
        let at = match self.list.iter().skip(self.from).position(same) {
            Some(at) => at.saturating_add(self.from),
            None if self.list.len() >= MAX_PROBLEMS => {
                self.more = self.more.saturating_add(1);
                return;
            }
            None => {
                self.list.push(problem);
                self.list.len().saturating_sub(1)
            }
        };
        let Some((instance, known)) = instance.zip(self.list.get_mut(at)) else { return };
        match known.get_mut("instances").and_then(Value::as_array_mut) {
            Some(names) if names.iter().any(|n| n.as_str() == Some(instance)) => {}
            Some(names) => names.push(json!(instance)),
            None => {
                known.insert("instances".into(), json!([instance]));
            }
        }
    }

    /// Says that the problems added since the last call are those of the document `title`.
    fn in_document(&mut self, title: &str) {
        for problem in self.list.iter_mut().skip(self.from) {
            problem.insert("document".into(), json!(title));
        }
        self.from = self.list.len();
    }

    fn json(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("errors".into(), json!(self.list));
        if self.more > 0 {
            out.insert("more".into(), json!(self.more));
        }
        out
    }

    /// One text for all problems.
    fn message(&self) -> String {
        let texts: Vec<&str> = self.list.iter().filter_map(|p| p.get("message").and_then(Value::as_str)).collect();
        let count = texts.len().saturating_add(self.more);
        let listed = texts.iter().take(MAX_IN_MESSAGE).copied().collect::<Vec<_>>().join("; ");
        match count {
            0 | 1 => listed,
            n if n > MAX_IN_MESSAGE => format!("{n} problems stop the export: {listed}; and {} more", n - MAX_IN_MESSAGE),
            n => format!("{n} problems stop the export: {listed}"),
        }
    }
}

// ── The single checks ───────────────────────────────────────────────────────────────────────────

/// The exported glyphs whose names break the naming rules, with the position of the bad character
/// (`glyphName.invalidBlocksExport`). File › Export refuses such a font; `font.compile` writes a
/// valid name with a warning instead.
pub(super) fn glyph_names(font: &Font, found: &mut Problems) {
    for g in font.glyphs.iter().filter(|g| g.export) {
        if let Err(e) = validate(&g.name, true) {
            let at = e.character.map(|_| e.position.saturating_add(1));
            found.add(
                "glyphName",
                format!("glyph `{}`: illegal glyph name: {e}", g.name),
                None,
                json!({ "glyph": g.name, "character": e.character.map(String::from), "position": at }),
            );
        }
    }
}

/// Whether every exported glyph has a name an export may write.
pub(super) fn glyph_names_ok(font: &Font) -> bool {
    font.glyphs.iter().filter(|g| g.export).all(|g| validate(&g.name, true).is_ok())
}

fn codepoint(found: &mut Problems, codepoint: u32, first: &str, second: &str) {
    found.add(
        "codepoint",
        CompileError::DuplicateCodepoint { codepoint, first: first.to_string(), second: second.to_string() }.to_string(),
        None,
        json!({ "codepoint": codepoint, "glyph": second, "glyphs": [first, second] }),
    );
}

/// The code points two exported glyphs share.
fn codepoints(font: &Font, found: &mut Problems) {
    let mut owners: BTreeMap<u32, &str> = BTreeMap::new();
    for g in font.glyphs.iter().filter(|g| g.export) {
        for &cp in &g.unicodes {
            match owners.get(&cp) {
                Some(first) if *first != g.name => codepoint(found, cp, first, &g.name),
                Some(_) => {}
                None => {
                    owners.insert(cp, &g.name);
                }
            }
        }
    }
}

/// `glyph` as an incompatible one: the text, and the layer that differs.
fn incompatible_text(glyph: &str, mismatches: &[Mismatch]) -> (String, Option<String>) {
    match mismatches.first() {
        Some(m) => (format!("glyph `{glyph}` has layers that are not compatible: {m}"), Some(m.layer_b.clone())),
        None => (format!("glyph `{glyph}` has layers that are not compatible"), None),
    }
}

fn incompatible(found: &mut Problems, instance: Option<&str>, glyph: &str, mismatches: &[Mismatch]) {
    let (message, layer) = incompatible_text(glyph, mismatches);
    found.add("incompatible", message, instance, json!({ "glyph": glyph, "layer": layer }));
}

/// The glyphs of a skipped instance that cannot be interpolated, for its entry in `skipped`.
pub(super) fn skipped_glyphs(font: &Font) -> Vec<Value> {
    let list = fontcraft_compile::var::incompatible_glyphs(font, None).unwrap_or_default();
    list.iter()
        .take(MAX_SKIPPED_GLYPHS)
        .map(|(glyph, mismatches)| {
            let (message, layer) = incompatible_text(glyph, mismatches);
            let mut out = Map::new();
            out.insert("glyph".into(), json!(glyph));
            out.insert("message".into(), json!(message));
            if let Some(layer) = layer {
                out.insert("layer".into(), json!(layer));
            }
            Value::Object(out)
        })
        .collect()
}

/// The errors among `list`, the diagnostics of the feature code as `job` compiles it. The entry
/// is the font's own; an entry that only the job's parameters write has no index.
fn features(found: &mut Problems, font: &Font, job: Option<&Job>, list: &[EntryDiagnostic]) {
    let params = job.and_then(|j| match j.target {
        Target::Instance(i) => font.instances.get(i).map(|i| &i.params),
        _ => None,
    });
    let edited = params.and_then(|p| apply_instance_params(&font.features, p).ok());
    let code: &Features = edited.as_ref().unwrap_or(&font.features);
    for d in list.iter().filter(|d| d.diagnostic.is_error()) {
        let place = d.place.as_ref().and_then(|place| Some((place, snippet_of(code, place.entry)?)));
        let Some((place, snippet)) = place else {
            found.add("features", format!("feature code: {}", d.message()), job.map(|j| j.label.as_str()), Value::Null);
            continue;
        };
        let (line, column) = positions(&snippet.code, &[place.range.start]).first().copied().unwrap_or((1, 1));
        let own = group(&font.features, place.entry.kind);
        let same = |e: &fontcraft_model::Snippet| e.name == snippet.name && e.code == snippet.code;
        let index = if own.get(place.entry.index).is_some_and(same) { Some(place.entry.index) } else { own.iter().position(same) };
        let mut entry = entry_json(code, place.entry);
        if let Some(entry) = entry.as_object_mut() {
            match index {
                Some(index) => entry.insert("index".into(), json!(index)),
                None => entry.remove("index"),
            };
        }
        let kind = match place.entry.kind {
            EntryKind::Prefix => "prefix",
            EntryKind::Class => "class",
            EntryKind::Feature => "feature",
        };
        found.add(
            "features",
            format!("{kind} {}, line {line}: {}", snippet.name, d.message()),
            job.map(|j| j.label.as_str()),
            json!({ "entry": entry, "line": line, "column": column }),
        );
    }
}

/// Whether `e` is the compiler refusing the font, so that a check can say what else it would
/// refuse.
fn refuses_the_font(e: &EngineError) -> bool {
    use CompileError as C;
    matches!(
        e,
        EngineError::Compile(
            C::Glyph { .. } | C::DuplicateCodepoint { .. } | C::Features(_) | C::Incompatible(_) | C::Interp(InterpError::Incompatible { .. })
        )
    )
}

/// Adds the error `e` met while building `job`.
pub(super) fn add_error(found: &mut Problems, font: &Font, job: Option<&Job>, e: &EngineError) {
    let instance = job.map(|j| j.label.as_str());
    match e {
        EngineError::Compile(CompileError::Features(list)) => features(found, font, job, list),
        EngineError::Compile(CompileError::DuplicateCodepoint { codepoint: cp, first, second }) => {
            codepoint(found, *cp, first, second);
            codepoints(font, found);
        }
        EngineError::Compile(CompileError::Interp(InterpError::Incompatible { glyph, mismatches })) => {
            incompatible(found, instance, glyph, mismatches);
        }
        EngineError::Compile(CompileError::Glyph { glyph, .. }) => found.add("glyph", e.to_string(), instance, json!({ "glyph": glyph })),
        other => found.add("font", other.to_string(), instance, Value::Null),
    }
}

// ── Checking an export ──────────────────────────────────────────────────────────────────────────

/// Whether the feature code uses the variable syntax: a `variation` block or an axis location.
fn varies_features(font: &Font) -> bool {
    let axes: Vec<String> = font.axes.iter().map(|a| format!("{}=", a.tag)).collect();
    let f = &font.features;
    f.prefixes
        .iter()
        .chain(&f.classes)
        .chain(&f.features)
        .any(|e| e.code.contains("variation ") || axes.iter().any(|axis| e.code.contains(axis.as_str())))
}

/// What an export builds from a font.
pub(super) enum Build<'a> {
    Static {
        jobs: &'a [Job],
        req: &'a Request,
    },
    /// The variable fonts of these settings (`None`: the whole designspace).
    Variable {
        settings: &'a [Option<usize>],
    },
}

/// One font of an export, as its checks read it.
pub(super) struct Subject<'a> {
    pub title: &'a str,
    pub path: Option<&'a Path>,
    pub font: &'a Font,
    pub data: Arc<GlyphData>,
    pub build: Build<'a>,
    /// Files may land outside the destination ([`Session::export_outside`]).
    pub outside: bool,
}

/// What a check of one font found besides its problems.
#[derive(Default)]
struct Checked {
    skipped: Vec<Value>,
    warnings: Vec<String>,
    files: usize,
}

impl Subject<'_> {
    /// Adds everything that stops this export.
    fn check(&self, found: &mut Problems) -> Result<Checked> {
        glyph_names(self.font, found);
        match &self.build {
            Build::Static { jobs, req } => {
                let mut report = Report::default();
                let sink = Sink::new(&req.dest, false, self.outside)?;
                let jobs = jobs.get(..MAX_JOBS).unwrap_or(jobs);
                compile::check(self.font, self.title, self.path, &self.data, jobs, req, sink, &mut report, found)?;
                // A font refused for its glyphs never reached its feature code.
                if !found.is_empty() && !found.has("features") {
                    self.feature_code(found);
                }
                // Each job reports what preparing the font reports.
                let mut seen = BTreeSet::new();
                report.warnings.retain(|w| seen.insert(w.clone()));
                Ok(Checked { skipped: report.skipped, warnings: report.warnings, files: report.planned })
            }
            Build::Variable { settings } => {
                for &setting in settings.iter().take(MAX_JOBS) {
                    crate::work::check()?;
                    let label = setting.and_then(|i| self.font.instances.get(i)).map(|i| i.name.as_str());
                    match fontcraft_compile::var::incompatible_glyphs(self.font, setting) {
                        Ok(list) => list.iter().for_each(|(glyph, mismatches)| incompatible(found, label, glyph, mismatches)),
                        Err(e) => found.add("font", e.to_string(), label, Value::Null),
                    }
                }
                codepoints(self.font, found);
                // The check below compiles one master's code, which variable syntax is no part of.
                if !varies_features(self.font) {
                    self.feature_code(found);
                }
                Ok(Checked::default())
            }
        }
    }

    /// The font's feature code as Font Info compiles it, for the first master. What the code
    /// cannot be compiled for (a value missing in the master) is the export's to report.
    fn feature_code(&self, found: &mut Problems) {
        let Ok(master) = self.font.default_master() else { return };
        let raw = json!({});
        let Ok(p) = Params::new("file.exportCheck", &raw) else { return };
        let Ok(values) = MasterValues::new(self.font, &self.data, &master.id) else { return };
        let cx = CheckContext { glyph_data: self.data.clone(), include_dir: self.path.and_then(super::shared::source_feature_dir) };
        if let Ok(list) = compile_entries(self.font, &self.font.features, &master.id, &cx, &values, &p) {
            features(found, self.font, None, &list);
        }
    }
}

/// Document `doc` as an export of `build` reads it.
pub(super) fn subject<'a>(s: &Session, doc: &'a crate::DocState, build: Build<'a>) -> Result<Subject<'a>> {
    Ok(Subject { title: &doc.title, path: doc.path.as_deref(), font: &doc.font, data: s.glyph_data_of(doc)?, build, outside: s.export_outside() })
}

/// Refuses an export before it builds when a font has a glyph name no export may write, listing
/// everything else that would stop it.
pub(super) fn refuse_names(subjects: &[Subject<'_>]) -> Result<()> {
    if subjects.iter().all(|s| glyph_names_ok(s.font)) { Ok(()) } else { Err(refusal(subjects, None)) }
}

/// The error of an export of `subjects` stopped by `cause` (by its glyph names when there is
/// none), listing everything that stops it. An error that is not about the font is returned as it
/// came.
pub(super) fn refusal(subjects: &[Subject<'_>], cause: Option<EngineError>) -> EngineError {
    let cause = match cause {
        Some(e) if !refuses_the_font(&e) => return e,
        cause => cause,
    };
    let mut found = Problems::default();
    for subject in subjects {
        match subject.check(&mut found) {
            Ok(_) => {}
            Err(EngineError::Cancelled) => return EngineError::Cancelled,
            Err(e) => found.add("font", e.to_string(), None, Value::Null),
        }
        if subjects.len() > 1 {
            found.in_document(subject.title);
        }
    }
    if let Some((e, subject)) = cause.as_ref().zip(subjects.first()) {
        let listed = match e {
            EngineError::Compile(CompileError::Features(_)) => found.has("features"),
            EngineError::Compile(CompileError::Incompatible(_)) => found.has("incompatible"),
            EngineError::Compile(CompileError::DuplicateCodepoint { .. }) => found.has("codepoint"),
            _ => false,
        };
        // The check may have stopped before the stage the build stopped in.
        if !listed && !(subjects.len() > 1 && !found.is_empty()) {
            add_error(&mut found, subject.font, None, e);
        }
    }
    let message = match cause {
        Some(e) if found.list.len() <= 1 => e.to_string(),
        _ => found.message(),
    };
    EngineError::Problems { message, data: Box::new(Value::Object(found.json())) }
}

/// The error of an export none of whose fonts could be built: why each was skipped.
pub(super) fn nothing_built(skipped: &[Value]) -> EngineError {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("?").to_string();
    let reasons: Vec<String> = skipped.iter().map(|v| format!("{}: {}", text(v, "instance"), text(v, "reason"))).collect();
    let message = format!("nothing could be built ({})", reasons.join("; "));
    EngineError::Problems { message, data: Box::new(json!({ "errors": [], "skipped": skipped })) }
}

// ── file.exportCheck ────────────────────────────────────────────────────────────────────────────

fn export_check(s: &mut Session, v: &Value) -> Result<Value> {
    let s: &Session = s;
    let p = Params::new("file.exportCheck", v)?;
    let doc = match p.opt_str("document")? {
        None => s.doc()?,
        Some(name) => s
            .documents()
            .iter()
            .find(|d| d.title == name || d.path.as_deref().is_some_and(|path| path == Path::new(name)))
            .ok_or_else(|| p.bad(format!("no open document is called or saved as `{name}`")))?,
    };
    let given = match p.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(Value::Object(m)) => Value::Object(m.clone()),
        Some(_) => return Err(p.bad("`params` must be a JSON object")),
    };
    let tab = p.opt_str("tab")?.unwrap_or("otf");
    let mut found = Problems::default();
    let checked = match tab {
        "otf" => {
            let p = Params::new("file.exportStatic", &given)?;
            let plan = super::compile_export::static_plan(s, &p, &doc.font)?;
            subject(s, doc, Build::Static { jobs: &plan.jobs, req: &plan.req })?.check(&mut found)?
        }
        "variable" => {
            let p = Params::new("file.exportVariable", &given)?;
            let settings = compile::picked_settings(&p, &doc.font)?;
            subject(s, doc, Build::Variable { settings: &settings })?.check(&mut found)?
        }
        other => return Err(p.bad(format!("`tab` is \"otf\" or \"variable\", got \"{other}\""))),
    };
    let mut out = found.json();
    out.insert("tab".into(), json!(tab));
    out.insert("document".into(), json!(doc.title));
    out.insert("ok".into(), json!(found.is_empty() && (tab != "otf" || checked.files > 0)));
    out.insert("skipped".into(), json!(checked.skipped));
    out.insert("warnings".into(), json!(checked.warnings));
    if tab == "otf" {
        out.insert("files".into(), json!(checked.files));
    }
    Ok(Value::Object(out))
}
