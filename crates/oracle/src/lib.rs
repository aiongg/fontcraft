//! The oracle harness (L3): FontCraft's compiled font compared, table by table, with a reference
//! compiler's build of the same source.
//!
//! A run ([`check`]) builds the source both ways with the same options and a fixed timestamp
//! (FontCraft through a closure the caller supplies, the reference through [`tools`]), reads both
//! fonts back and compares them normalised ([`compare`]). Each table comes out identical,
//! equivalent (it differs only where normalisation allows), coverage-equal (outline tables only:
//! some glyphs differ in their points but cover the same pixels), allowed (every difference is on
//! the allow-list, [`allow`]), different, or skipped (outside the comparison for now: layout tables
//! until M8, variation tables until M9). Differences in how the two programs read the source, such
//! as a different glyph set or order, are reported apart as source-reading findings.
//!
//! The reference compilers are optional: when one is missing, [`tools::locate`] says so and the
//! caller skips. Both fonts are untrusted input to the comparison; every count read from them is
//! capped and nothing here panics.
#![forbid(unsafe_code)]

pub mod allow;
pub mod compare;
pub mod corpus;
pub mod outline;
pub mod tools;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

pub use allow::AllowList;
pub use tools::{Built, Flavour, Hint, Oracle};

/// The default `SOURCE_DATE_EPOCH` of a comparison (2023-11-14). Timestamps are removed before
/// comparing; a fixed value keeps both builds reproducible.
pub const DEFAULT_TIMESTAMP: i64 = 1_700_000_000;
/// The default time limit for one reference build.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// How many differences per table a report shows by default.
pub const DEFAULT_SHOWN: usize = 5;

/// How a table compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The same bytes (for `head`, apart from the checksum and the dates).
    Identical,
    /// Different bytes, the same content once normalised.
    Equivalent,
    /// An outline table whose glyphs are all equivalent or, where their points differ, cover
    /// the same pixels ([`outline::coverage_equal`]). Used where both sides removed overlaps.
    #[serde(rename = "coverage-equal")]
    CoverageEqual,
    /// Different, and every difference is on the allow-list.
    Allowed,
    Different,
    /// Not compared; the note says why.
    Skipped,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Identical => "identical",
            Status::Equivalent => "equivalent",
            Status::CoverageEqual => "coverage-equal",
            Status::Allowed => "allowed",
            Status::Different => "different",
            Status::Skipped => "skipped",
        }
    }
}

/// One normalised difference: a key naming the field, record or glyph, and the two values.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Difference {
    pub key: String,
    pub ours: String,
    pub theirs: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TableResult {
    pub tag: String,
    pub status: Status,
    /// The first differences not on the allow-list.
    pub differences: Vec<Difference>,
    /// How many differences there were before the allow-list.
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Reasons of the allow-list entries that matched.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,
    /// For an outline table (`glyf`, `CFF `): how the glyphs both fonts have compared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<GlyphCounts>,
}

/// How the outlines of the glyphs both fonts have compared, before the allow-list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlyphCounts {
    pub compared: usize,
    /// The same segments within half a unit, start points free.
    pub same: usize,
    /// Different segments, the same rendered coverage.
    pub coverage_equal: usize,
    pub different: usize,
}

impl GlyphCounts {
    fn add(&mut self, o: &GlyphCounts) {
        self.compared += o.compared;
        self.same += o.same;
        self.coverage_equal += o.coverage_equal;
        self.different += o.different;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingKind {
    /// The fonts have different glyphs.
    GlyphSet,
    /// The same glyphs in a different order.
    GlyphOrder,
}

/// A difference in how the two programs read the source, not in how they compile it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Compared,
    /// The reference compiler failed; FontCraft's build is not compared.
    ReferenceFailed,
    FontcraftFailed,
    BothFailed,
    /// FontCraft cannot build this kind of font yet; the reference build was still tried.
    NotCompared,
}

/// Everything a comparison of one source found.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReport {
    pub source: String,
    pub oracle: Oracle,
    pub flavour: Flavour,
    pub outcome: Outcome,
    /// Why a build failed or was not made.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Built>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_reading: Vec<Finding>,
    pub tables: Vec<TableResult>,
}

impl SourceReport {
    /// Does any table differ beyond the allow-list?
    pub fn has_differences(&self) -> bool {
        self.tables.iter().any(|t| t.status == Status::Different)
    }
}

/// FontCraft's side of a comparison.
#[derive(Clone, Debug, Default)]
pub struct Ours {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
    pub hint: Hint,
}

/// Why FontCraft's side has no font.
#[derive(Clone, Debug, PartialEq)]
pub enum OursError {
    /// FontCraft does not build this yet (the message says when it will). The hint still helps
    /// pick the reference build.
    Unsupported {
        message: String,
        hint: Hint,
    },
    Failed(String),
}

/// How to run a comparison.
#[derive(Clone, Debug)]
pub struct Settings {
    pub oracle: Oracle,
    pub flavour: Flavour,
    /// The reference compiler's executable ([`tools::locate`]).
    pub tool: PathBuf,
    pub timestamp: i64,
    pub timeout: Duration,
    /// Keep both builds in a folder per source under this one; without it they go to a
    /// temporary folder that is removed afterwards.
    pub keep: Option<PathBuf>,
    /// Differences shown per table.
    pub shown: usize,
    pub allow: AllowList,
}

/// Compares `source` built by FontCraft (`ours`) and by the reference compiler. `label` names
/// the source in the report and in allow-list matches.
pub fn check(source: &Path, label: &str, settings: &Settings, ours: impl FnOnce() -> Result<Ours, OursError>) -> SourceReport {
    let mut report = SourceReport {
        source: label.to_string(),
        oracle: settings.oracle,
        flavour: settings.flavour,
        outcome: Outcome::Compared,
        messages: Vec::new(),
        reference: None,
        notes: Vec::new(),
        source_reading: Vec::new(),
        tables: Vec::new(),
    };
    let ours = ours();
    let hint = match &ours {
        Ok(o) => o.hint.clone(),
        Err(OursError::Unsupported { hint, .. }) => hint.clone(),
        Err(OursError::Failed(_)) => Hint::default(),
    };
    let work = match work_dir(settings, label) {
        Ok(w) => w,
        Err(e) => {
            report.outcome = Outcome::ReferenceFailed;
            report.messages.push(e);
            return report;
        }
    };
    let built = tools::build(&tools::Request {
        oracle: settings.oracle,
        tool: &settings.tool,
        source,
        flavour: settings.flavour,
        work_dir: &work.path,
        timestamp: settings.timestamp,
        timeout: settings.timeout,
        hint: &hint,
    });
    let mut built = built;
    if let (Ok(o), Ok(b)) = (&ours, &mut built)
        && !b.alternatives.is_empty()
    {
        closest_master(&o.bytes, b, compare::Outlines::of(settings.oracle));
    }
    let reference = built.as_ref().ok().and_then(|b| std::fs::read(&b.path).ok());
    report.outcome = match (&ours, &built) {
        (Err(OursError::Unsupported { .. }), _) => Outcome::NotCompared,
        (Ok(_), Ok(_)) => Outcome::Compared,
        (Ok(_), Err(_)) => Outcome::ReferenceFailed,
        (Err(_), Ok(_)) => Outcome::FontcraftFailed,
        (Err(_), Err(_)) => Outcome::BothFailed,
    };
    match &ours {
        Ok(o) => report.notes.extend(o.warnings.iter().map(|w| format!("FontCraft: {w}"))),
        Err(OursError::Unsupported { message: m, .. } | OursError::Failed(m)) => report.messages.push(format!("FontCraft: {m}")),
    }
    match built {
        Ok(b) => {
            report.notes.extend(b.notes.iter().cloned());
            report.reference = Some(b);
        }
        Err(e) => report.messages.push(e),
    }
    // FontCraft's font is kept whenever it was built, compared or not.
    if let (Ok(o), Some(_)) = (&ours, &settings.keep) {
        let path = work.path.join(format!("fontcraft.{}", settings.flavour.extension()));
        if let Err(e) = std::fs::write(&path, &o.bytes) {
            report.notes.push(format!("cannot keep FontCraft's font at {}: {e}", path.display()));
        }
    }
    if let (Ok(o), Some(theirs)) = (&ours, &reference) {
        match compare::compare_with(&o.bytes, theirs, compare::Outlines::of(settings.oracle)) {
            Ok(c) => {
                report.notes.extend(c.notes);
                report.source_reading = c.source_reading;
                report.tables = c.tables;
                apply_allow(&mut report.tables, &settings.allow, settings.oracle.name(), label);
                for t in &mut report.tables {
                    t.differences.truncate(settings.shown);
                }
            }
            Err(e) => {
                report.outcome = Outcome::BothFailed;
                report.messages.push(e);
            }
        }
    } else if report.outcome == Outcome::Compared {
        report.outcome = Outcome::ReferenceFailed;
        report.messages.push("the reference font cannot be read back".into());
    }
    report
}

/// Of masters that share a name, the file whose font differs least from ours (fewest differing
/// tables, then fewest differences) becomes `built.path`. The note names the choice.
fn closest_master(ours: &[u8], built: &mut tools::Built, outlines: compare::Outlines) {
    let candidates: Vec<PathBuf> = std::iter::once(built.path.clone()).chain(built.alternatives.iter().cloned()).collect();
    let distance = |path: &PathBuf| {
        let theirs = std::fs::read(path).ok()?;
        let c = compare::compare_with(ours, &theirs, outlines).ok()?;
        let different: Vec<&TableResult> = c.tables.iter().filter(|t| t.status == Status::Different).collect();
        Some((different.len(), different.iter().map(|t| t.count).sum::<usize>()))
    };
    // Ties keep the earlier file, the first master of the name.
    let best = candidates.iter().filter_map(|p| distance(p).map(|d| (d, p))).min_by_key(|(d, _)| *d).map(|(_, p)| p.clone());
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(best) = best {
        built.notes.push(format!("{} masters share the name of {}; compared with the closest, {}", candidates.len(), name(&built.path), name(&best)));
        built.path = best;
    }
}

/// Marks a different table "allowed" when the allow-list accepts every difference, and drops
/// the accepted ones from the rest.
pub fn apply_allow(tables: &mut [TableResult], allow: &AllowList, oracle: &str, source: &str) {
    for t in tables.iter_mut().filter(|t| t.status == Status::Different) {
        let mut reasons: Vec<String> = Vec::new();
        let tag = t.tag.trim_end().to_string();
        t.differences.retain(|d| match allow.find(oracle, source, &tag, d) {
            Some(e) => {
                if !reasons.contains(&e.reason) {
                    reasons.push(e.reason.clone());
                }
                false
            }
            None => true,
        });
        // A table with more differences than were kept cannot be shown to be fully allowed.
        if t.differences.is_empty() && t.count <= compare::MAX_KEPT {
            t.status = Status::Allowed;
        }
        t.allowed = reasons;
    }
}

/// A work folder, removed on drop unless it is kept.
struct Work {
    path: PathBuf,
    remove: bool,
}

impl Drop for Work {
    fn drop(&mut self) {
        if self.remove {
            // Best effort: a temporary folder left behind is not an error worth reporting.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn work_dir(settings: &Settings, label: &str) -> Result<Work, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let (path, remove) = match &settings.keep {
        Some(root) => (root.join(slug(label)), false),
        None => (std::env::temp_dir().join(format!("fontcraft-oracle-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))), true),
    };
    if path.exists() {
        // Outputs of an earlier run would be mistaken for this one's.
        std::fs::remove_dir_all(&path).map_err(|e| format!("cannot clear {}: {e}", path.display()))?;
    }
    std::fs::create_dir_all(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    Ok(Work { path, remove })
}

/// A folder name for a source label.
fn slug(label: &str) -> String {
    let s: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).take(150).collect();
    if s.trim_matches(['.', '_']).is_empty() { "source".into() } else { s }
}

/// Counts of table outcomes over the compared sources.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Counts {
    pub identical: usize,
    pub equivalent: usize,
    pub coverage_equal: usize,
    pub allowed: usize,
    pub different: usize,
    pub skipped: usize,
}

/// A run over several sources, summed up.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub sources: usize,
    pub compared: usize,
    pub reference_built: usize,
    pub fontcraft_built: usize,
    /// Compared sources with a source-reading finding.
    pub source_reading: usize,
    /// Per table tag, over the compared sources whose fonts have the table.
    pub tables: BTreeMap<String, Counts>,
    /// Per outline table tag, the glyphs compared over the compared sources.
    pub glyphs: BTreeMap<String, GlyphCounts>,
}

pub fn summarise(reports: &[SourceReport]) -> Summary {
    let mut s = Summary { sources: reports.len(), ..Default::default() };
    for r in reports {
        if r.reference.is_some() {
            s.reference_built += 1;
        }
        if matches!(r.outcome, Outcome::Compared | Outcome::ReferenceFailed) {
            s.fontcraft_built += 1;
        }
        if r.outcome != Outcome::Compared {
            continue;
        }
        s.compared += 1;
        if !r.source_reading.is_empty() {
            s.source_reading += 1;
        }
        for t in &r.tables {
            let c = s.tables.entry(t.tag.clone()).or_default();
            match t.status {
                Status::Identical => c.identical += 1,
                Status::Equivalent => c.equivalent += 1,
                Status::CoverageEqual => c.coverage_equal += 1,
                Status::Allowed => c.allowed += 1,
                Status::Different => c.different += 1,
                Status::Skipped => c.skipped += 1,
            }
            if let Some(g) = &t.glyphs {
                s.glyphs.entry(t.tag.clone()).or_default().add(g);
            }
        }
    }
    s
}

/// One source's report as text.
pub fn render(r: &SourceReport) -> String {
    let mut out = String::new();
    let outcome = match r.outcome {
        Outcome::Compared => "compared",
        Outcome::ReferenceFailed => "reference build failed",
        Outcome::FontcraftFailed => "FontCraft build failed",
        Outcome::BothFailed => "both builds failed",
        Outcome::NotCompared => "not compared",
    };
    let _ = writeln!(out, "{}  [{} .{}] {outcome}", r.source, r.oracle.name(), r.flavour.extension());
    for m in &r.messages {
        let _ = writeln!(out, "  ! {m}");
    }
    for n in &r.notes {
        let _ = writeln!(out, "  note: {n}");
    }
    for f in &r.source_reading {
        let _ = writeln!(out, "  source reading: {}", f.message);
    }
    for t in &r.tables {
        let mut line = format!("  {:<5} {}", t.tag.trim_end(), t.status.name());
        if t.status == Status::Different {
            let _ = write!(line, " ({})", t.count);
        }
        if let Some(n) = &t.note {
            let _ = write!(line, "  [{n}]");
        }
        let _ = writeln!(out, "{line}");
        for d in &t.differences {
            let _ = writeln!(out, "        {}: {} | reference: {}", d.key, d.ours, d.theirs);
        }
        if t.status == Status::Allowed {
            for reason in &t.allowed {
                let _ = writeln!(out, "        allowed: {reason}");
            }
        }
    }
    out
}

/// A summary as a text table.
pub fn render_summary(s: &Summary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} sources: reference built {}, FontCraft built {}, compared {}; {} with a source-reading difference",
        s.sources, s.reference_built, s.fontcraft_built, s.compared, s.source_reading
    );
    let _ = writeln!(
        out,
        "{:<6} {:>9} {:>10} {:>14} {:>7} {:>9} {:>7}",
        "table", "identical", "equivalent", "coverage-equal", "allowed", "different", "skipped"
    );
    let mut tags: Vec<&String> = s.tables.keys().collect();
    let rank = |t: &str| compare::CORE.iter().position(|c| *c == t).unwrap_or(compare::CORE.len());
    tags.sort_by_key(|t| (rank(t), (*t).clone()));
    for tag in tags {
        if let Some(c) = s.tables.get(tag) {
            let _ = writeln!(
                out,
                "{:<6} {:>9} {:>10} {:>14} {:>7} {:>9} {:>7}",
                tag.trim_end(),
                c.identical,
                c.equivalent,
                c.coverage_equal,
                c.allowed,
                c.different,
                c.skipped
            );
        }
    }
    for (tag, g) in &s.glyphs {
        let _ = writeln!(
            out,
            "{} glyphs: {} compared, {} same, {} coverage-equal, {} different (before the allow-list)",
            tag.trim_end(),
            g.compared,
            g.same,
            g.coverage_equal,
            g.different
        );
    }
    out
}
