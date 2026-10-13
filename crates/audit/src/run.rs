//! The runner (M12.44): runs the checks of every scope on one target and tallies what ran, what
//! was skipped and why, and what was found (`docs/spec/21-qa.md` § 1 and § 3).
//!
//! A target is a source, one or more compiled fonts, or both:
//!
//! - On a source, layer checks run on every layer (the engine's loop over [`crate::audit_layer`]),
//!   glyph checks on every glyph ([`run_glyphs`]), source checks once ([`run_source`]), binary
//!   checks on each instance compiled in memory ([`run_binary`] with a [`crate::SourceMap`]), and
//!   family checks on those instances ([`run_family`]).
//! - On a compiled font, the layer and glyph checks run on the decoded file; checks with the
//!   [`Precondition::SourceData`] precondition are skipped there.
//! - Several fonts are also one family: every family check receives all of them and groups them
//!   itself by name ID 1 or name ID 16 (`audit.familyGrouping`, see `checks/family.rs`).
//!
//! A check whose preconditions do not hold is skipped with the reason ([`unmet`]). A binary
//! finding that names a table field gets the source field behind it from the font's source map
//! when the check did not set one (`audit.scopeResolution`).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fontcraft_model::Font;
use read_fonts::{FontRef, TableProvider};

use crate::checks::family::MAX_FACES;
use crate::{
    AuditContext, BinaryCtx, CheckScope, CheckSpec, Config, FamilyCtx, Finding, GlyphCtx, LayerReport, Precondition, SourceCtx, SourceMap,
    audit_binary, audit_family, audit_glyph, audit_source,
};

/// Most fonts one run reads: files given, faces of collections and instances compiled from
/// sources together. The family checks read at most as many.
pub const MAX_TARGET_FONTS: usize = MAX_FACES;
/// Most instances of one source compiled in memory for the binary checks.
pub const MAX_SOURCE_INSTANCES: usize = 32;
/// Most entries one glyph selection holds, after splitting.
pub const MAX_SELECTION_ENTRIES: usize = 100_000;
/// Most warnings a glyph selection reports; the rest are counted.
pub const MAX_SELECTION_WARNINGS: usize = 100;

/// What is known about the target, for the preconditions.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// The target is a source (not a decoded compiled font).
    pub source: bool,
    /// A source with more than one master, or a compiled font with `fvar`.
    pub variable: bool,
    /// The run allows network access (`audit.networkOptIn`).
    pub network: bool,
    /// The project values the source sets (`audit.projectValues`).
    pub values: BTreeSet<String>,
}

/// Why `spec` cannot run on this target, or `None` when every precondition holds. `font` is the
/// compiled font a binary or family check reads; facts about compiled tables are not tested
/// without one.
pub fn unmet(spec: &CheckSpec, facts: &Facts, font: Option<&FontRef<'_>>) -> Option<String> {
    spec.preconditions.iter().find(|p| !holds(**p, facts, font)).map(|p| p.describe())
}

fn holds(p: Precondition, facts: &Facts, font: Option<&FontRef<'_>>) -> bool {
    let has = |tag: &[u8; 4]| font.is_none_or(|f| f.table_data(read_fonts::types::Tag::new(tag)).is_some());
    match p {
        Precondition::TrueTypeOutlines => has(b"glyf"),
        Precondition::CffOutlines => font.is_none_or(|_| has(b"CFF ") || has(b"CFF2")),
        Precondition::Variable => font.map_or(facts.variable, |_| has(b"fvar")),
        Precondition::Static => font.map_or(!facts.variable, |f| f.fvar().is_err()),
        Precondition::Hinted => font.is_none_or(|_| has(b"fpgm") || has(b"prep") || has(b"cvt ") || has(b"CFF ") || has(b"CFF2")),
        Precondition::Table(tag) => {
            let mut t = [b' '; 4];
            for (slot, b) in t.iter_mut().zip(tag.bytes()) {
                *slot = b;
            }
            has(&t)
        }
        // No check needs a script's coverage tested before it runs: each one reads the cmap
        // itself and reports nothing for a script the font does not cover.
        Precondition::ScriptCovered(_) => true,
        Precondition::ProjectValue(name) => facts.values.contains(name),
        Precondition::Network => facts.network,
        Precondition::SourceData => facts.source,
    }
}

/// What a run found and did, summed over every font and glyph it examined.
#[derive(Debug, Default)]
pub struct Tally {
    pub findings: Vec<Finding>,
    /// Per check: how many targets (layers, glyphs, fonts, families) it ran on.
    pub ran: BTreeMap<&'static str, usize>,
    /// Per check: how many targets it was skipped on, and the first reason.
    pub skipped: BTreeMap<&'static str, (usize, String)>,
    /// Findings dropped over a check's per-target limit.
    pub truncated: usize,
    /// Things the run could not do, for the result document (`notes`).
    pub notes: Vec<String>,
}

impl Tally {
    pub fn skip(&mut self, check: &'static str, reason: impl Into<String>) {
        let e = self.skipped.entry(check).or_insert_with(|| (0, reason.into()));
        e.0 = e.0.saturating_add(1);
    }

    pub fn ran(&mut self, check: &'static str) {
        let n = self.ran.entry(check).or_default();
        *n = n.saturating_add(1);
    }

    /// Adds one runner call's report: every check in `run` ran unless the report lists it as
    /// skipped.
    pub fn absorb(&mut self, run: &[&'static CheckSpec], report: LayerReport) {
        for c in run {
            match report.skipped.iter().find(|s| s.check == c.id) {
                Some(s) => self.skip(c.id, s.reason.clone()),
                None => self.ran(c.id),
            }
        }
        self.truncated = report.truncated.iter().fold(self.truncated, |n, t| n.saturating_add(t.dropped));
        self.findings.extend(report.findings);
    }

    /// Splits `checks` of `scope` into those whose preconditions hold, recording the others as
    /// skipped.
    fn ready(&mut self, checks: &[&'static CheckSpec], scope: CheckScope, facts: &Facts, font: Option<&FontRef<'_>>) -> Vec<&'static CheckSpec> {
        let mut out = Vec::new();
        for &c in checks.iter().filter(|c| c.scope == scope) {
            match unmet(c, facts, font) {
                Some(reason) => self.skip(c.id, reason),
                None => out.push(c),
            }
        }
        out
    }
}

/// Does any of `checks` have this scope?
pub fn any_of(checks: &[&'static CheckSpec], scope: CheckScope) -> bool {
    checks.iter().any(|c| c.scope == scope)
}

/// Runs the glyph checks among `checks` on each glyph of the context's font, or on the glyphs
/// in `glyphs`.
pub fn run_glyphs(
    ctx: &AuditContext<'_>,
    services: &dyn crate::AuditServices,
    checks: &[&'static CheckSpec],
    config: &Config,
    facts: &Facts,
    glyphs: Option<&HashSet<String>>,
    tally: &mut Tally,
) {
    let ready = tally.ready(checks, CheckScope::Glyph, facts, None);
    if ready.is_empty() {
        return;
    }
    for g in ctx.font.glyphs.as_arcs() {
        if glyphs.is_some_and(|set| !set.contains(&g.name)) {
            continue;
        }
        let gctx = GlyphCtx::new(ctx, g, services);
        let report = audit_glyph(&gctx, &ready, config);
        tally.absorb(&ready, report);
    }
}

/// Runs the source checks among `checks` once on the whole source.
pub fn run_source(
    ctx: &AuditContext<'_>,
    services: &dyn crate::AuditServices,
    checks: &[&'static CheckSpec],
    config: &Config,
    facts: &Facts,
    tally: &mut Tally,
) {
    let ready = tally.ready(checks, CheckScope::Source, facts, None);
    if ready.is_empty() {
        return;
    }
    let sctx = SourceCtx { audit: ctx, services };
    let report = audit_source(&sctx, &ready, config);
    tally.absorb(&ready, report);
}

/// Runs the binary checks among `checks` on one compiled font. Findings carry the font's id, its
/// source instance and, through its source map, the source fields behind table fields.
pub fn run_binary(ctx: &BinaryCtx<'_>, checks: &[&'static CheckSpec], config: &Config, facts: &Facts, tally: &mut Tally) {
    let font_facts = Facts { source: false, variable: ctx.font.fvar().is_ok(), ..facts.clone() };
    let ready = tally.ready(checks, CheckScope::Binary, &font_facts, Some(&ctx.font));
    if ready.is_empty() {
        return;
    }
    let mut report = audit_binary(ctx, &ready, config);
    if let Some(map) = ctx.source {
        for f in &mut report.findings {
            map_back(f, map);
        }
    }
    tally.absorb(&ready, report);
}

/// Fills what a binary finding leaves out and its source map knows: the instance, and the source
/// field behind `table.field`.
pub fn map_back(f: &mut Finding, map: &SourceMap) {
    if f.location.instance.is_none() {
        f.location.instance.clone_from(&map.instance);
    }
    if f.location.source_field.is_none()
        && let (Some(table), Some(field)) = (&f.location.table, &f.location.field)
    {
        f.location.source_field = map.field(&format!("{}.{field}", table.trim_end())).map(str::to_string);
    }
}

/// Runs the family checks among `checks` on `fonts`. Fewer than two fonts skip them; more than
/// [`MAX_TARGET_FONTS`] are cut to the first ones with a note.
pub fn run_family(
    fonts: &[BinaryCtx<'_>],
    services: &dyn crate::AuditServices,
    checks: &[&'static CheckSpec],
    config: &Config,
    facts: &Facts,
    tally: &mut Tally,
) {
    let family: Vec<&'static CheckSpec> = checks.iter().copied().filter(|c| c.scope == CheckScope::Family).collect();
    if family.is_empty() {
        return;
    }
    if fonts.len() < 2 {
        for c in family {
            tally.skip(c.id, "family checks need at least two fonts: give several files, a folder, or a source with several instances");
        }
        return;
    }
    let fonts = match fonts.get(..MAX_TARGET_FONTS) {
        Some(head) if fonts.len() > MAX_TARGET_FONTS => {
            tally.notes.push(format!("the family checks read the first {MAX_TARGET_FONTS} of {} fonts", fonts.len()));
            head
        }
        _ => fonts,
    };
    let ready = tally.ready(&family, CheckScope::Family, &Facts { source: false, ..facts.clone() }, None);
    if ready.is_empty() {
        return;
    }
    let fctx = FamilyCtx::new(fonts, services);
    let report = audit_family(&fctx, &ready, config);
    tally.absorb(&ready, report);
}

// ── Glyph selection ─────────────────────────────────────────────────────────────────────────────

/// A glyph selection resolved against a font (`audit.glyphSelectionSyntax`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// The glyph names selected, in font order.
    pub glyphs: Vec<String>,
    /// One line per entry that matched no glyph.
    pub warnings: Vec<String>,
}

/// Resolves glyph selection entries against `font`. Each entry is split at commas and white
/// space; a piece is a glyph name, a glyph index, a CID written `/N` (the glyph named `cidNNNNN`,
/// else index N), or a range `first-last` of any two of those (a name holding a hyphen is read
/// as a name first). Pieces that match no glyph are warned about and skipped.
pub fn select_glyphs(font: &Font, entries: &[&str]) -> Selection {
    let names: Vec<&str> = font.glyphs.as_arcs().iter().map(|g| g.name.as_str()).collect();
    let index_of = |name: &str| names.iter().position(|n| *n == name);
    let one = |piece: &str| -> Option<usize> {
        if let Some(i) = index_of(piece) {
            return Some(i);
        }
        if let Some(cid) = piece.strip_prefix('/') {
            let n: usize = cid.parse().ok()?;
            return index_of(&format!("cid{n:05}")).or_else(|| (n < names.len()).then_some(n));
        }
        piece.parse::<usize>().ok().filter(|n| *n < names.len())
    };
    let mut picked = vec![false; names.len()];
    let mut out = Selection::default();
    let mut unmatched = 0usize;
    let pieces = entries.iter().flat_map(|e| e.split(|c: char| c == ',' || c.is_whitespace())).filter(|p| !p.is_empty());
    for (seen, piece) in pieces.enumerate() {
        if seen >= MAX_SELECTION_ENTRIES {
            out.warnings.push(format!("the glyph selection holds more than {MAX_SELECTION_ENTRIES} entries; the rest were ignored"));
            break;
        }
        let range = match one(piece) {
            Some(i) => Some((i, i)),
            None => piece.split_once('-').and_then(|(a, b)| Some((one(a)?, one(b)?))),
        };
        match range {
            Some((a, b)) => {
                for slot in picked.iter_mut().take(a.max(b).saturating_add(1)).skip(a.min(b)) {
                    *slot = true;
                }
            }
            None => {
                unmatched = unmatched.saturating_add(1);
                if out.warnings.len() < MAX_SELECTION_WARNINGS {
                    out.warnings.push(format!("`{piece}` matches no glyph and was skipped"));
                }
            }
        }
    }
    if unmatched > MAX_SELECTION_WARNINGS {
        out.warnings.push(format!("{} more entries match no glyph", unmatched - MAX_SELECTION_WARNINGS));
    }
    out.glyphs = names.iter().zip(&picked).filter(|(_, p)| **p).map(|(n, _)| (*n).to_string()).collect();
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use fontcraft_model::Glyph;

    fn font(names: &[&str]) -> Font {
        let mut f = Font::new("Test");
        for n in names {
            f.glyphs.push(Glyph::new(*n));
        }
        f
    }

    #[test]
    fn glyph_selection_reads_names_indexes_ranges_and_cids() {
        let f = font(&[".notdef", "a", "b", "c", "a-cy", "cid00007", "z"]);
        let names: Vec<&str> = f.glyphs.as_arcs().iter().map(|g| g.name.as_str()).collect();
        let z = names.iter().position(|n| *n == "z").unwrap().to_string();
        let s = select_glyphs(&f, &[&format!("a-c, {z}"), "/7 a-cy\tnope", "0-0"]);
        let mut want = vec![".notdef", "a", "b", "c", "a-cy", "cid00007", "z"];
        want.sort_by_key(|n| names.iter().position(|m| m == n));
        assert_eq!(s.glyphs, want.into_iter().map(String::from).collect::<Vec<_>>());
        assert_eq!(s.warnings, ["`nope` matches no glyph and was skipped"]);
        let s = select_glyphs(&f, &["c-a"]);
        assert_eq!(s.glyphs, ["a", "b", "c"].map(String::from).to_vec());
    }

    #[test]
    fn preconditions_without_a_compiled_font_follow_the_facts() {
        let spec = crate::CHECKS.iter().find(|c| c.preconditions.contains(&Precondition::Variable)).unwrap();
        assert!(unmet(spec, &Facts { variable: true, ..Facts::default() }, None).is_none());
        assert_eq!(unmet(spec, &Facts::default(), None).as_deref(), Some("the font is not variable"));
    }
}
