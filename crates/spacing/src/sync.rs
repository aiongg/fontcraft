//! The out-of-sync scan, its cache, and the order Update Metrics sets metrics in.
//!
//! Scanned layers: master layers and the layers with attributes (intermediate, alternate, colour,
//! smart); backup layers are not. Layers of a master that takes its metrics from another master
//! ([`crate::metrics_source`]) are not scanned either. A key is out of sync when its value differs
//! from the layer's metric rounded half up. A sidebearing key on an empty layer has nothing to
//! compare with and is not listed.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

use fontcraft_model::{Font, Glyph, Layer, MAX_COMPONENT_DEPTH, Master, MetricDef, Shape};

use crate::eval::{Ctx, EvalError, effective_key};
use crate::keys::{Body, Scope, parse_stored};
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

fn scan_into(ctx: &mut Ctx<'_>, glyph: &Glyph, out: &mut Vec<Finding>) {
    for layer in glyph.layers.iter().filter(|l| scanned(ctx.font, l)) {
        for field in Field::ALL {
            let Some((scope, text)) = effective_key(glyph, layer, field) else { continue };
            let problem = match check(ctx, glyph, layer, field, scope, text) {
                Ok(None) => continue,
                Ok(Some(p)) => p,
                Err(EvalError::Unlinked(name)) => Problem::Unlinked { glyph: name },
                Err(e) => Problem::Error(e),
            };
            out.push(Finding { glyph: glyph.name.clone(), layer: layer.id.clone(), field, scope, key: text.to_string(), problem });
        }
    }
}

fn check(ctx: &mut Ctx<'_>, glyph: &Glyph, layer: &Layer, field: Field, scope: Scope, text: &str) -> Result<Option<Problem>, EvalError> {
    let key = parse_stored(text, scope, field, |n| ctx.exists(n))?;
    let value = ctx.value(glyph, layer, field, &key)?;
    let Some(actual) = ctx.metric(glyph, layer, field)? else { return Ok(None) };
    Ok((round_half_up(actual) != value).then_some(Problem::OutOfSync { value, actual }))
}

/// Every key of the font that is out of sync, unlinked or in error, in glyph order.
pub fn scan(font: &Font) -> Vec<Finding> {
    let mut ctx = Ctx::new(font);
    let mut out = Vec::new();
    for g in font.glyphs.as_arcs() {
        scan_into(&mut ctx, g, &mut out);
    }
    out
}

/// [`scan`] for one glyph.
pub fn scan_glyph(font: &Font, glyph: &Glyph) -> Vec<Finding> {
    let mut out = Vec::new();
    scan_into(&mut Ctx::new(font), glyph, &mut out);
    out
}

/// The scan's result kept across edits. Copy-on-write keeps the `Arc` of every glyph an edit did
/// not touch, so [`SyncCache::update`] recomputes only the glyphs whose `Arc` changed and the
/// glyphs whose results read one of them (keys, the chains behind them, and the components of
/// every layer measured). A change to the masters, the metrics, the units per em or the glyph
/// names recomputes everything.
#[derive(Default)]
pub struct SyncCache {
    /// The font of the last update; the `Weak` keeps its address from being reused.
    font: Weak<Font>,
    masters: Option<Arc<Vec<Master>>>,
    metrics: Option<Arc<Vec<MetricDef>>>,
    units_per_em: u16,
    names: u64,
    entries: HashMap<String, Kept>,
    /// Glyph → the glyphs whose results read it.
    readers: HashMap<String, HashSet<String>>,
    computed: u64,
}

struct Kept {
    glyph: Arc<Glyph>,
    findings: Vec<Finding>,
    deps: Vec<String>,
}

fn names_hash(font: &Font) -> u64 {
    let mut h = DefaultHasher::new();
    for g in font.glyphs.as_arcs() {
        g.name.hash(&mut h);
    }
    h.finish()
}

impl SyncCache {
    /// Brings the cache up to date with `font`.
    pub fn update(&mut self, font: &Arc<Font>) {
        if std::ptr::eq(self.font.as_ptr(), Arc::as_ptr(font)) {
            return;
        }
        let names = names_hash(font);
        let same = self.masters.as_ref().is_some_and(|m| Arc::ptr_eq(m, &font.masters))
            && self.metrics.as_ref().is_some_and(|m| Arc::ptr_eq(m, &font.metrics))
            && self.units_per_em == font.units_per_em
            && self.names == names;
        let dirty: HashSet<String> = if same {
            let mut dirty = HashSet::new();
            for g in font.glyphs.as_arcs() {
                if self.entries.get(&g.name).is_some_and(|k| Arc::ptr_eq(&k.glyph, g)) {
                    continue;
                }
                if let Some(readers) = self.readers.get(&g.name) {
                    dirty.extend(readers.iter().cloned());
                }
                dirty.insert(g.name.clone());
            }
            dirty
        } else {
            self.entries.clear();
            self.readers.clear();
            font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect()
        };
        let mut ctx = Ctx::new(font);
        for g in font.glyphs.as_arcs().iter().filter(|g| dirty.contains(&g.name)) {
            ctx.forget_reads();
            let mut findings = Vec::new();
            scan_into(&mut ctx, g, &mut findings);
            let deps: Vec<String> = ctx.deps.drain().collect();
            if let Some(old) = self.entries.remove(&g.name) {
                for d in &old.deps {
                    if let Some(r) = self.readers.get_mut(d) {
                        r.remove(&g.name);
                    }
                }
            }
            for d in &deps {
                self.readers.entry(d.clone()).or_default().insert(g.name.clone());
            }
            self.entries.insert(g.name.clone(), Kept { glyph: g.clone(), findings, deps });
            self.computed = self.computed.saturating_add(1);
        }
        self.masters = Some(font.masters.clone());
        self.metrics = Some(font.metrics.clone());
        self.units_per_em = font.units_per_em;
        self.names = names;
        self.font = Arc::downgrade(font);
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
