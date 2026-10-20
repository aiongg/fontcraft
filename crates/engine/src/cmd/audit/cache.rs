//! The per-glyph cache of the layer checks (row `audit.glyphHashCache`).
//!
//! A run on the open document keeps, with its results, what the layer checks reported on each
//! layer of each glyph. The next run reuses a glyph's reports, without running the checks again,
//! when nothing the layer checks read has changed:
//!
//! - the glyph, and every glyph its components reach on any layer (a missing one stays missing);
//! - the font apart from its glyphs (masters, metrics, info, settings, kerning, features …);
//! - the facts the runner surveys over all glyphs (grid step, main curve type, more than one
//!   master);
//! - the layer checks that run, their thresholds and the layer and master filters.
//!
//! Reports are replayed in the order a full run makes them, so the result document is the one a
//! full run gives. Glyph, source, binary and family checks and the check modules always run:
//! several glyph checks read glyphs no component names (kerning groups, metrics keys).
//! `all: true` ignores the cache.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use super::Run;
use fontcraft_audit::{AuditContext, CheckSpec, Config, CurveType, LayerReport};
use fontcraft_model::{Font, Glyph, GlyphTable, Shape};

/// Most glyphs one glyph's components may reach for its reports to be kept; a deeper or wider
/// tree is examined on every run.
const MAX_DEPENDENCIES: usize = 256;

/// What every kept report depends on besides its glyphs.
#[derive(Debug, PartialEq)]
struct Key {
    context: Font,
    grid_step: Option<u64>,
    main_curve: Option<CurveType>,
    variable: bool,
    checks: Vec<&'static str>,
    /// The thresholds as the run resolved them: profile, document overrides, then the call.
    config: Config,
    layers: Option<Vec<String>>,
    masters: Option<Vec<String>>,
}

impl Key {
    fn new(ctx: &AuditContext<'_>, run: &Run, layer_checks: &[&'static CheckSpec]) -> Self {
        let mut context = ctx.font.clone();
        context.glyphs = GlyphTable::default();
        // Edit-tab texts and the note change while the person types; no layer check reads them.
        context.display_strings = Arc::default();
        context.note = String::new();
        let sorted = |set: &Option<HashSet<String>>| {
            set.as_ref().map(|s| {
                let mut v: Vec<String> = s.iter().cloned().collect();
                v.sort();
                v
            })
        };
        Key {
            context,
            grid_step: ctx.grid_step.map(f64::to_bits),
            main_curve: ctx.main_curve,
            variable: ctx.variable,
            checks: layer_checks.iter().map(|c| c.id).collect(),
            config: run.config.clone(),
            layers: sorted(&run.layers),
            masters: sorted(&run.masters),
        }
    }
}

/// One glyph's kept reports.
#[derive(Debug)]
struct Entry {
    /// The glyph first, then every glyph its components reach, as the run saw them.
    deps: Vec<(String, Option<Arc<Glyph>>)>,
    /// By layer id, in the order the run examined them.
    reports: Vec<(String, LayerReport)>,
}

/// The layer reports of one run, by glyph name.
#[derive(Debug, Default)]
pub(crate) struct LayerCache {
    key: Option<Key>,
    glyphs: HashMap<String, Entry>,
}

/// The glyph and the glyphs its components reach, or `None` past [`MAX_DEPENDENCIES`].
fn dependencies(font: &Font, glyph: &Arc<Glyph>) -> Option<Vec<(String, Option<Arc<Glyph>>)>> {
    let mut out = vec![(glyph.name.clone(), Some(glyph.clone()))];
    let mut seen: HashSet<String> = HashSet::from([glyph.name.clone()]);
    let mut queue: VecDeque<Arc<Glyph>> = VecDeque::from([glyph.clone()]);
    while let Some(g) = queue.pop_front() {
        for layer in &g.layers {
            for shape in &layer.shapes {
                let Shape::Component(c) = shape else { continue };
                if !seen.insert(c.glyph.clone()) {
                    continue;
                }
                if out.len() >= MAX_DEPENDENCIES {
                    return None;
                }
                let base = font.glyph(&c.glyph).cloned();
                if let Some(b) = &base {
                    queue.push_back(b.clone());
                }
                out.push((c.glyph.clone(), base));
            }
        }
    }
    Some(out)
}

fn unchanged(font: &Font, deps: &[(String, Option<Arc<Glyph>>)]) -> bool {
    deps.iter().all(|(name, then)| match (then, font.glyph(name)) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
        _ => false,
    })
}

impl LayerCache {
    /// The kept reports of `glyph`'s layers, when nothing the glyph's layer checks read has
    /// changed. The caller has checked the key ([`LayerCache::key_matches`]).
    pub(crate) fn reports(&self, ctx: &AuditContext<'_>, glyph: &Glyph) -> Option<&[(String, LayerReport)]> {
        let entry = self.glyphs.get(&glyph.name)?;
        unchanged(ctx.font, &entry.deps).then_some(entry.reports.as_slice())
    }

    /// Whether this cache was made under the key of `run` on the font of `ctx`.
    pub(crate) fn key_matches(&self, ctx: &AuditContext<'_>, run: &Run, layer_checks: &[&'static CheckSpec]) -> bool {
        self.key.as_ref().is_some_and(|k| *k == Key::new(ctx, run, layer_checks))
    }

    /// Starts recording a run's reports under its key, unless already started.
    pub(crate) fn start(&mut self, ctx: &AuditContext<'_>, run: &Run, layer_checks: &[&'static CheckSpec]) {
        if self.key.is_none() {
            self.key = Some(Key::new(ctx, run, layer_checks));
        }
    }

    /// Records one layer's report.
    pub(crate) fn record(&mut self, font: &Font, glyph: &Arc<Glyph>, layer: &str, report: &LayerReport) {
        if let Some(entry) = self.glyphs.get_mut(&glyph.name) {
            entry.reports.push((layer.to_string(), report.clone()));
            return;
        }
        let Some(deps) = dependencies(font, glyph) else { return };
        self.glyphs.insert(glyph.name.clone(), Entry { deps, reports: vec![(layer.to_string(), report.clone())] });
    }
}
