//! The out-of-sync scan, its cache, and the order Update Metrics sets metrics in.
//!
//! Scanned layers: master layers and the layers with attributes (intermediate, alternate, colour,
//! smart); backup layers are not. Layers of a master that takes its metrics from another master
//! ([`crate::metrics_source`]) are not scanned either. A key is out of sync when its value differs
//! from the layer's metric rounded half up. A sidebearing key on an empty layer has nothing to
//! compare with and is not listed, nor is a left, right or width key on a layer whose metrics
//! automatic alignment derives ([`crate::aligned`]).

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Weak};

use fontcraft_model::{Font, Glyph, Layer, MAX_COMPONENT_DEPTH, Master, MetricDef, Shape};

use crate::aligned::{alignment_off, derives_metrics, is_derived_field};
use crate::eval::{Ctx, EvalError, effective_key};
use crate::keys::{Body, Key, Scope, parse_stored};
use crate::{Field, metrics_source, referenced_layer, round_half_up};

#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// The key gives `value`; the layer's metric is `actual`.
    OutOfSync { value: f64, actual: f64 },
    /// The key names a glyph the font does not have.
    Unlinked { glyph: String },
    /// The key cannot be read or evaluated.
    Error(EvalError),
}

/// One key that needs attention.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub glyph: String,
    pub layer: String,
    pub field: Field,
    /// Where the key is stored: on the glyph, or on the layer.
    pub scope: Scope,
    /// The key as stored.
    pub key: String,
    pub problem: Problem,
}

fn scanned(font: &Font, layer: &Layer) -> bool {
    (layer.is_master_layer() || !layer.info.attrs.is_empty()) && metrics_source(font, &layer.master).is_some_and(|m| m.id == layer.master)
}

/// One metric a glyph's scan measured on its own layers: (layer index, field, result).
type Measurement = (usize, Field, Result<Option<f64>, EvalError>);
/// The metrics a glyph's scan measured on its own layers.
type Measured = Vec<Measurement>;

/// The own-layer measurements of one glyph's scan: those of an earlier scan that still hold, and
/// those of this one.
struct Own<'p> {
    prior: &'p [Measurement],
    measured: Measured,
}

impl Own<'_> {
    fn none() -> Own<'static> {
        Own { prior: &[], measured: Vec::new() }
    }

    /// The metric of layer `index` of `glyph`, from the earlier scan when it has it.
    fn metric(&mut self, ctx: &mut Ctx<'_>, glyph: &Glyph, layer: &Layer, index: usize, field: Field) -> Result<Option<f64>, EvalError> {
        let result = match self.prior.iter().find(|(i, f, _)| *i == index && *f == field) {
            Some((_, _, r)) => r.clone(),
            None => ctx.metric(glyph, layer, field),
        };
        self.measured.push((index, field, result.clone()));
        result
    }
}

fn scan_into(ctx: &mut Ctx<'_>, glyph: &Glyph, out: &mut Vec<Finding>, own: &mut Own<'_>) {
    for (index, layer) in glyph.layers.iter().enumerate().filter(|(_, l)| scanned(ctx.font, l)) {
        let aligned = derives_metrics(ctx.font, layer);
        for field in Field::ALL {
            // Alignment sets these metrics and applies their offset keys itself.
            if aligned && is_derived_field(field) {
                continue;
            }
            let Some((scope, text)) = effective_key(glyph, layer, field) else { continue };
            let problem = match check(ctx, own, glyph, (index, layer), field, scope, text) {
                Ok(None) => continue,
                Ok(Some(p)) => p,
                Err(EvalError::Unlinked(name)) => Problem::Unlinked { glyph: name },
                Err(e) => Problem::Error(e),
            };
            out.push(Finding { glyph: glyph.name.clone(), layer: layer.id.clone(), field, scope, key: text.to_string(), problem });
        }
    }
}

fn check(
    ctx: &mut Ctx<'_>,
    own: &mut Own<'_>,
    glyph: &Glyph,
    (index, layer): (usize, &Layer),
    field: Field,
    scope: Scope,
    text: &str,
) -> Result<Option<Problem>, EvalError> {
    let key = parse_stored(text, scope, field, |n| ctx.exists(n))?;
    let value = ctx.value(glyph, layer, field, &key)?;
    let Some(actual) = own.metric(ctx, glyph, layer, index, field)? else { return Ok(None) };
    Ok((round_half_up(actual) != value).then_some(Problem::OutOfSync { value, actual }))
}

/// Every key of the font that is out of sync, unlinked or in error, in glyph order.
pub fn scan(font: &Font) -> Vec<Finding> {
    let mut ctx = Ctx::new(font);
    let mut out = Vec::new();
    for g in font.glyphs.as_arcs() {
        scan_into(&mut ctx, g, &mut out, &mut Own::none());
    }
    out
}

/// [`scan`] for one glyph.
pub fn scan_glyph(font: &Font, glyph: &Glyph) -> Vec<Finding> {
    let mut out = Vec::new();
    scan_into(&mut Ctx::new(font), glyph, &mut out, &mut Own::none());
    out
}

/// The scan's result kept across edits. Copy-on-write keeps the `Arc` of every glyph an edit did
/// not touch, so [`SyncCache::update`] recomputes only the glyphs whose `Arc` changed, the glyphs
/// that appeared, and the glyphs whose results read a changed, added or removed glyph name: the
/// glyphs a key reads (and the chains behind them), the glyphs every measured layer shows through
/// components, and every name a key looked up, found or not. A renamed glyph counts as one name
/// removed and one added. A glyph scanned again reuses its own measured metrics when neither it
/// nor any glyph its components show changed. A change to the masters, the metrics or the units
/// per em, or of the font's Disable Automatic Alignment option, recomputes everything.
#[derive(Default)]
pub struct SyncCache {
    /// The font of the last update; the `Weak` keeps its address from being reused.
    font: Weak<Font>,
    masters: Option<Arc<Vec<Master>>>,
    metrics: Option<Arc<Vec<MetricDef>>>,
    units_per_em: u16,
    /// The font's Disable Automatic Alignment option at the last update.
    alignment_off: bool,
    entries: HashMap<String, Kept>,
    /// Glyph name → the glyphs whose results read it or looked it up.
    readers: HashMap<String, HashSet<String>>,
    computed: u64,
}

struct Kept {
    glyph: Arc<Glyph>,
    findings: Vec<Finding>,
    /// The names this glyph's results depend on (see [`SyncCache`]).
    deps: Vec<String>,
    /// The glyphs this glyph's layers show through components, all the way down.
    shows: Vec<String>,
    measured: Measured,
}

/// The names that parsing `glyph`'s keys looks up (found or not), with the glyph each key names.
/// Every key of every layer is read, so this covers the keys a chain through `glyph` parses.
fn lookups(font: &Font, glyph: &Glyph, out: &mut HashSet<String>) {
    let sets = glyph.layers.iter().map(|l| (Scope::Local, &l.info.metrics_keys)).chain(std::iter::once((Scope::Glyph, &glyph.info.metrics_keys)));
    let asked: Cell<Vec<String>> = Cell::new(Vec::new());
    for (scope, keys) in sets {
        for field in Field::ALL {
            let Some(text) = field.key_in(keys).filter(|t| !t.trim().is_empty()) else { continue };
            let parsed = parse_stored(text, scope, field, |n| {
                let mut v = asked.take();
                v.push(n.to_string());
                asked.set(v);
                font.glyph(n).is_some()
            });
            if let Some(name) = parsed.as_ref().ok().and_then(Key::glyph) {
                out.insert(name.to_string());
            }
            out.extend(asked.take());
        }
    }
}

/// The glyphs `glyph`'s layers show through components (each layer reading its bases' layers of
/// its master), named whether or not they exist. One level deeper than outlines go, and each
/// base once per layer.
fn shows(font: &Font, glyph: &Glyph) -> Vec<String> {
    let mut out: HashSet<String> = HashSet::new();
    for layer in &glyph.layers {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack: Vec<(&Layer, usize)> = vec![(layer, 0)];
        while let Some((l, depth)) = stack.pop() {
            for shape in &l.shapes {
                let Shape::Component(c) = shape else { continue };
                if !seen.insert(c.glyph.as_str()) {
                    continue;
                }
                if !out.contains(&c.glyph) {
                    out.insert(c.glyph.clone());
                }
                if depth <= MAX_COMPONENT_DEPTH
                    && let Some(base) = font.glyph(&c.glyph).and_then(|g| g.layer(&layer.master))
                {
                    stack.push((base, depth + 1));
                }
            }
        }
    }
    out.into_iter().collect()
}

impl SyncCache {
    /// Brings the cache up to date with `font`.
    pub fn update(&mut self, font: &Arc<Font>) {
        if std::ptr::eq(self.font.as_ptr(), Arc::as_ptr(font)) {
            return;
        }
        let same = self.masters.as_ref().is_some_and(|m| Arc::ptr_eq(m, &font.masters))
            && self.metrics.as_ref().is_some_and(|m| Arc::ptr_eq(m, &font.metrics))
            && self.units_per_em == font.units_per_em
            && self.alignment_off == alignment_off(font);
        if !same {
            self.entries.clear();
            self.readers.clear();
        }
        // The names whose glyph is new, edited or gone.
        let mut touched: HashSet<String> = HashSet::new();
        let mut matched = 0usize;
        for g in font.glyphs.as_arcs() {
            match self.entries.get(&g.name) {
                Some(k) => {
                    matched += 1;
                    if !Arc::ptr_eq(&k.glyph, g) {
                        touched.insert(g.name.clone());
                    }
                }
                None => {
                    touched.insert(g.name.clone());
                }
            }
        }
        let mut dirty = touched.clone();
        if matched != self.entries.len() {
            let gone: Vec<String> = self.entries.keys().filter(|n| font.glyph(n).is_none()).cloned().collect();
            for name in gone {
                if let Some(old) = self.entries.remove(&name) {
                    self.unlink(&name, &old.deps);
                }
                touched.insert(name);
            }
        }
        for name in &touched {
            if let Some(readers) = self.readers.get(name) {
                dirty.extend(readers.iter().cloned());
            }
        }
        let mut ctx = Ctx::new(font);
        let mut looked_up: HashMap<String, HashSet<String>> = HashMap::new();
        for g in font.glyphs.as_arcs().iter().filter(|g| dirty.contains(&g.name)) {
            let old = self.entries.remove(&g.name);
            let prior = match &old {
                Some(k) if Arc::ptr_eq(&k.glyph, g) && !k.shows.iter().any(|s| touched.contains(s)) => k.measured.as_slice(),
                _ => &[],
            };
            ctx.forget_reads();
            let mut own = Own { prior, measured: Vec::new() };
            let mut findings = Vec::new();
            scan_into(&mut ctx, g, &mut findings, &mut own);
            let measured = own.measured;
            let mut names: HashSet<String> = std::mem::take(&mut ctx.deps);
            // A reused measurement read the glyph and what it shows without telling `ctx`.
            let shown = if measured.is_empty() { Vec::new() } else { shows(font, g) };
            if !measured.is_empty() {
                names.insert(g.name.clone());
                names.extend(shown.iter().cloned());
            }
            let mut read: Vec<String> = names.iter().cloned().collect();
            read.push(g.name.clone());
            for r in &read {
                let Some(rg) = font.glyph(r) else { continue };
                let found = looked_up.entry(r.clone()).or_insert_with(|| {
                    let mut out = HashSet::new();
                    lookups(font, rg, &mut out);
                    out
                });
                names.extend(found.iter().cloned());
            }
            if let Some(old) = &old {
                self.unlink(&g.name, &old.deps);
            }
            let deps: Vec<String> = names.into_iter().collect();
            for d in &deps {
                self.readers.entry(d.clone()).or_default().insert(g.name.clone());
            }
            self.entries.insert(g.name.clone(), Kept { glyph: g.clone(), findings, deps, shows: shown, measured });
            self.computed = self.computed.saturating_add(1);
        }
        self.masters = Some(font.masters.clone());
        self.metrics = Some(font.metrics.clone());
        self.units_per_em = font.units_per_em;
        self.alignment_off = alignment_off(font);
        self.font = Arc::downgrade(font);
    }

    /// Takes `reader` off the reader lists of `deps`.
    fn unlink(&mut self, reader: &str, deps: &[String]) {
        for d in deps {
            if let Some(r) = self.readers.get_mut(d) {
                r.remove(reader);
                if r.is_empty() {
                    self.readers.remove(d);
                }
            }
        }
    }

    /// The findings of one glyph as of the last update.
    pub fn glyph(&self, name: &str) -> &[Finding] {
        self.entries.get(name).map_or(&[], |k| k.findings.as_slice())
    }

    /// Every finding as of the last update, in `font`'s glyph order.
    pub fn all<'a>(&'a self, font: &'a Font) -> impl Iterator<Item = &'a Finding> + 'a {
        font.glyphs.as_arcs().iter().flat_map(move |g| self.glyph(&g.name).iter())
    }

    /// How many glyphs were scanned since the cache was made.
    pub fn computed(&self) -> u64 {
        self.computed
    }
}

/// One metric of one layer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Slot {
    pub glyph: String,
    pub layer: String,
    pub field: Field,
}

/// The fields whose update changes `field` as measured: setting the left sidebearing moves the
/// outline and keeps the right one (the width follows); setting the right sidebearing or the
/// width moves the right edge.
fn writers(field: Field) -> &'static [Field] {
    match field {
        Field::Left => &[Field::Left],
        Field::Right => &[Field::Right, Field::Width],
        Field::Width => &[Field::Left, Field::Right, Field::Width],
        Field::Top => &[Field::Top],
        Field::Bottom => &[Field::Bottom],
    }
}

/// `slots` ordered so that a metric comes after the metrics in `slots` its key reads, and a
/// composite's sidebearings come after the left sidebearings of the glyphs its components show
/// (updating those moves their outlines, and so the composite's), so that Update Metrics settles
/// a chain in one run. Otherwise the given order is kept; slots in a cycle come last, in the
/// given order. A slot given twice appears once.
pub fn update_order(font: &Font, slots: &[Slot]) -> Vec<Slot> {
    let mut index: HashMap<(&str, &str, Field), usize> = HashMap::new();
    let mut unique: Vec<&Slot> = Vec::new();
    for s in slots {
        if let std::collections::hash_map::Entry::Vacant(e) = index.entry((s.glyph.as_str(), s.layer.as_str(), s.field)) {
            e.insert(unique.len());
            unique.push(s);
        }
    }
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); unique.len()];
    let mut waiting: Vec<usize> = vec![0; unique.len()];
    for (i, s) in unique.iter().enumerate() {
        let Some((r, rl, f)) = reads(font, s) else { continue };
        for w in writers(f) {
            if let Some(&j) = index.get(&(r.name.as_str(), rl.id.as_str(), *w))
                && j != i
                && let (Some(list), Some(n)) = (after.get_mut(j), waiting.get_mut(i))
            {
                list.push(i);
                *n += 1;
            }
        }
    }
    let mut shown: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (i, s) in unique.iter().enumerate() {
        if !matches!(s.field, Field::Left | Field::Right) {
            continue;
        }
        let deps = shown.entry((s.glyph.as_str(), s.layer.as_str())).or_insert_with(|| component_slots(font, s, &index));
        for &j in deps.iter() {
            if j != i
                && let (Some(list), Some(n)) = (after.get_mut(j), waiting.get_mut(i))
            {
                list.push(i);
                *n += 1;
            }
        }
    }
    let mut ready: BTreeSet<usize> = waiting.iter().enumerate().filter(|(_, n)| **n == 0).map(|(i, _)| i).collect();
    let mut done = vec![false; unique.len()];
    let mut out = Vec::with_capacity(unique.len());
    while let Some(i) = ready.pop_first() {
        if let Some(d) = done.get_mut(i) {
            *d = true;
        }
        if let Some(s) = unique.get(i) {
            out.push((*s).clone());
        }
        for &k in after.get(i).map_or(&[][..], Vec::as_slice) {
            if let Some(n) = waiting.get_mut(k) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    ready.insert(k);
                }
            }
        }
    }
    for (i, s) in unique.iter().enumerate() {
        if !done.get(i).copied().unwrap_or(true) {
            out.push((*s).clone());
        }
    }
    out
}

/// The left-sidebearing slots in `index` of the glyphs `s`'s layer shows through components.
/// The walk stops at a base glyph that has such a slot (that slot waits for what lies below it)
/// and at [`MAX_COMPONENT_DEPTH`]; each base layer is visited once.
fn component_slots(font: &Font, s: &Slot, index: &HashMap<(&str, &str, Field), usize>) -> Vec<usize> {
    let mut out = Vec::new();
    let Some(layer) = font.glyph(&s.glyph).and_then(|g| g.layers.iter().find(|l| l.id == s.layer)) else { return out };
    let master = layer.master.as_str();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<(&Layer, usize)> = vec![(layer, 0)];
    while let Some((l, depth)) = stack.pop() {
        for shape in &l.shapes {
            let Shape::Component(c) = shape else { continue };
            let Some(base) = font.glyph(&c.glyph) else { continue };
            let Some(bl) = base.layer(master) else { continue };
            if !seen.insert(base.name.as_str()) {
                continue;
            }
            match index.get(&(base.name.as_str(), bl.id.as_str(), Field::Left)) {
                Some(&j) => out.push(j),
                None if depth < MAX_COMPONENT_DEPTH => stack.push((bl, depth + 1)),
                None => {}
            }
        }
    }
    out
}

/// The glyph, layer and field a slot's key reads.
fn reads<'a>(font: &'a Font, s: &Slot) -> Option<(&'a Glyph, &'a Layer, Field)> {
    let g = font.glyph(&s.glyph)?;
    let l = g.layers.iter().find(|l| l.id == s.layer)?;
    let (scope, text) = effective_key(g, l, s.field)?;
    let key = parse_stored(text, scope, s.field, |n| font.glyph(n).is_some()).ok()?;
    match &key.body {
        Body::SelfMirror => Some((g, l, s.field.opposite())),
        Body::Name { name, mirrored, .. } => {
            let r = font.glyph(name)?;
            Some((r, referenced_layer(r, l)?, if *mirrored { s.field.opposite() } else { s.field }))
        }
        Body::Constant(_) | Body::Signed(_) => None,
    }
}
