//! Evaluating metrics keys.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_geom::Rect;
use fontcraft_model::{Font, Glyph, Layer, MAX_COMPONENT_DEPTH, Shape};

use crate::aligned::{derived_field, derives_metrics, is_derived_field};
use crate::keys::{Body, Key, KeyError, Op, Scope, parse_stored};
use crate::measure::{at_height, bounds_with, master_slant, metric_from, outline};
use crate::{Field, MAX_CHAIN, metrics_source, referenced_layer, round_half_up};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum EvalError {
    #[error("{0}")]
    Key(#[from] KeyError),
    /// The key names a glyph the font does not have.
    #[error("no glyph is called `{0}`: the key is unlinked")]
    Unlinked(String),
    #[error("no glyph is called `{0}`")]
    NoSuchGlyph(String),
    #[error("glyph `{glyph}` has no layer `{layer}`")]
    NoSuchLayer { glyph: String, layer: String },
    #[error("glyph `{0}` has no layer for this master")]
    NoLayer(String),
    #[error("glyph `{0}` is empty here, so it has no sidebearing")]
    Empty(String),
    #[error("the key divides by zero")]
    DivisionByZero,
    #[error("the key's references lead back to it")]
    Cycle,
    #[error("the key's references run deeper than {MAX_CHAIN} steps")]
    TooDeep,
    #[error("the result {value} is outside {min} to {max}")]
    OutOfRange { value: f64, min: f64, max: f64 },
    #[error("the result is not a number")]
    NotFinite,
    #[error("the outline cannot be measured: {0}")]
    Measure(String),
}

/// The key that applies to one field of a layer, with where it is stored: the layer's own key,
/// else the glyph's. An empty text is no key.
pub fn effective_key<'a>(glyph: &'a Glyph, layer: &'a Layer, field: Field) -> Option<(Scope, &'a str)> {
    let some = |k: Option<&'a str>| k.filter(|t| !t.trim().is_empty());
    some(field.key_in(&layer.info.metrics_keys))
        .map(|k| (Scope::Local, k))
        .or_else(|| some(field.key_in(&glyph.info.metrics_keys)).map(|k| (Scope::Glyph, k)))
}

/// The value of the key that applies to `field` of layer `layer` of `glyph`; `None` when no key
/// applies, which includes a field that automatic alignment derives ([`crate::aligned`]): there
/// alignment sets the metric, and applies an offset key itself.
pub fn evaluate(font: &Font, glyph: &str, layer: &str, field: Field) -> Result<Option<f64>, EvalError> {
    let (g, l) = find(font, glyph, layer)?;
    if derived_field(font, l, field) {
        return Ok(None);
    }
    let Some((scope, text)) = effective_key(g, l, field) else { return Ok(None) };
    let key = parse_stored(text, scope, field, |n| font.glyph(n).is_some())?;
    Ctx::new(font).value(g, l, field, &key).map(Some)
}

/// The value `key` gives in `field` of layer `layer` of `glyph`, whatever key is stored there.
pub fn evaluate_key(font: &Font, glyph: &str, layer: &str, field: Field, key: &Key) -> Result<f64, EvalError> {
    let (g, l) = find(font, glyph, layer)?;
    Ctx::new(font).value(g, l, field, key)
}

fn find<'a>(font: &'a Font, glyph: &str, layer: &str) -> Result<(&'a Glyph, &'a Layer), EvalError> {
    let g = font.glyph(glyph).ok_or_else(|| EvalError::NoSuchGlyph(glyph.to_string()))?;
    let l = g.layers.iter().find(|l| l.id == layer).ok_or_else(|| EvalError::NoSuchLayer { glyph: glyph.into(), layer: layer.into() })?;
    Ok((g, l))
}

fn address<T>(t: &T) -> usize {
    std::ptr::from_ref(t) as usize
}

/// The measured bounds of layers, by the layer's address in memory: they hold while the layer's
/// glyph is kept alive and neither it nor what it shows through components changes.
#[derive(Clone, Debug, Default)]
pub(crate) struct LayerBounds {
    by_layer: HashMap<usize, Result<Option<Rect>, EvalError>>,
    /// The layers of `by_layer` that hold components.
    composites: Vec<usize>,
}

impl LayerBounds {
    pub(crate) fn clear(&mut self) {
        self.by_layer.clear();
        self.composites.clear();
    }

    /// Forgets the layers of `glyph`.
    pub(crate) fn forget(&mut self, glyph: &Glyph) {
        for layer in &glyph.layers {
            self.by_layer.remove(&address(layer));
        }
    }

    /// Forgets every layer that shows other glyphs.
    pub(crate) fn forget_composites(&mut self) {
        for at in self.composites.drain(..) {
            self.by_layer.remove(&at);
        }
    }
}

/// One metric a key's value was read from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Read {
    pub(crate) glyph: String,
    /// Index of the layer in the glyph.
    layer: usize,
    field: Field,
    /// The height of an `@` key.
    at: Option<f64>,
    /// The layer shows other glyphs.
    pub(crate) composite: bool,
    value: Result<Option<f64>, EvalError>,
}

/// What a run keeps of the results it computes, for the out-of-sync cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Collect {
    Nothing,
    /// The metrics read.
    Reads,
    /// The metrics read and the names of the glyphs the results depend on.
    ReadsAndNames,
}

/// A key's text by its place in memory, with what decides how it parses.
type KeyAt = (usize, usize, Scope, Field);

/// One evaluation run over one font: measured bounds and parsed keys are kept, and what the
/// results read is collected for the out-of-sync cache.
pub(crate) struct Ctx<'a> {
    pub(crate) font: &'a Font,
    pub(crate) bounds: LayerBounds,
    /// The keys parsed, by the place of their text, which lives as long as the font.
    keys: HashMap<KeyAt, Result<Arc<Key>, KeyError>>,
    /// Whether alignment derives a layer's metrics, by layer address.
    derived: HashMap<usize, bool>,
    /// The slant of the masters met.
    slants: Vec<(&'a str, Option<(f64, f64)>)>,
    /// The masters that show their own metrics.
    own_metrics: Option<Vec<&'a str>>,
    collect: Collect,
    /// The metrics the results read.
    pub(crate) reads: Vec<Read>,
    /// Glyphs whose data the results read.
    pub(crate) deps: HashSet<String>,
    /// Layers whose components were added to `deps`.
    walked: HashSet<usize>,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(font: &'a Font) -> Self {
        Self::with_bounds(font, LayerBounds::default())
    }

    /// A run that starts from the bounds of an earlier one.
    pub(crate) fn with_bounds(font: &'a Font, bounds: LayerBounds) -> Self {
        Self {
            font,
            bounds,
            keys: HashMap::new(),
            derived: HashMap::new(),
            slants: Vec::new(),
            own_metrics: None,
            collect: Collect::Nothing,
            reads: Vec::new(),
            deps: HashSet::new(),
            walked: HashSet::new(),
        }
    }

    /// Starts collecting what another result reads.
    pub(crate) fn start(&mut self, collect: Collect) {
        self.collect = collect;
        self.reads.clear();
        self.deps.clear();
        self.walked.clear();
    }

    /// The stored key `text`, parsed once however many layers and chains read it.
    pub(crate) fn parse(&mut self, text: &'a str, scope: Scope, field: Field) -> Result<Arc<Key>, KeyError> {
        let font = self.font;
        self.keys
            .entry((text.as_ptr() as usize, text.len(), scope, field))
            .or_insert_with(|| parse_stored(text, scope, field, |n| font.glyph(n).is_some()).map(Arc::new))
            .clone()
    }

    /// [`derives_metrics`], looked up once per layer.
    pub(crate) fn derives(&mut self, layer: &Layer) -> bool {
        let font = self.font;
        *self.derived.entry(address(layer)).or_insert_with(|| derives_metrics(font, layer))
    }

    fn derived_field(&mut self, layer: &Layer, field: Field) -> bool {
        is_derived_field(field) && self.derives(layer)
    }

    /// Whether `layer`'s master shows its own metrics ([`metrics_source`]).
    pub(crate) fn own_metrics(&mut self, layer: &Layer) -> bool {
        let font = self.font;
        let own = self.own_metrics.get_or_insert_with(|| {
            font.masters.iter().filter(|m| metrics_source(font, &m.id).is_some_and(|s| s.id == m.id)).map(|m| m.id.as_str()).collect()
        });
        own.iter().any(|id| *id == layer.master)
    }

    fn slant(&mut self, layer: &Layer) -> Option<(f64, f64)> {
        if let Some((_, slant)) = self.slants.iter().find(|(id, _)| *id == layer.master) {
            return *slant;
        }
        let slant = master_slant(self.font, &layer.master);
        // A layer of a master the font lacks has no slant and nothing to keep it under.
        if let Some(m) = self.font.masters.iter().find(|m| m.id == layer.master) {
            self.slants.push((m.id.as_str(), slant));
        }
        slant
    }

    /// Whether the metric of `read` is another one now.
    pub(crate) fn differs(&mut self, read: &Read) -> bool {
        let font = self.font;
        let Some((g, l)) = font.glyph(&read.glyph).and_then(|g| Some((g, g.layers.get(read.layer)?))) else { return true };
        let now = match read.at {
            Some(y) => self.metric_at(g, l, read.field, y),
            None => self.metric(g, l, read.field),
        };
        now != read.value
    }

    /// Records that a result read `value`, the metric `field` of `layer` of `glyph`.
    fn record(&mut self, glyph: &Glyph, layer: &Layer, field: Field, at: Option<f64>, value: &Result<Option<f64>, EvalError>) {
        if self.collect == Collect::Nothing {
            return;
        }
        let Some(index) = glyph.layers.iter().position(|l| std::ptr::eq(l, layer)) else { return };
        let composite = layer.shapes.iter().any(|s| matches!(s, Shape::Component(_)));
        self.reads.push(Read { glyph: glyph.name.clone(), layer: index, field, at, composite, value: value.clone() });
    }

    fn read(&mut self, glyph: &Glyph, layer: &Layer) {
        if self.collect != Collect::ReadsAndNames {
            return;
        }
        if !self.deps.contains(&glyph.name) {
            self.deps.insert(glyph.name.clone());
        }
        self.walk(layer, 0);
    }

    /// Adds the glyphs `layer`'s components come from, all the way down.
    fn walk(&mut self, layer: &Layer, depth: usize) {
        if depth > MAX_COMPONENT_DEPTH || !self.walked.insert(address(layer)) {
            return;
        }
        for shape in &layer.shapes {
            let Shape::Component(c) = shape else { continue };
            if !self.deps.contains(&c.glyph) {
                self.deps.insert(c.glyph.clone());
            }
            let font = self.font;
            if let Some(base) = font.glyph(&c.glyph).and_then(|g| g.layer(&layer.master)) {
                self.walk(base, depth + 1);
            }
        }
    }

    fn bounds(&mut self, glyph: &Glyph, layer: &Layer) -> Result<Option<Rect>, EvalError> {
        self.read(glyph, layer);
        let at = address(layer);
        if let Some(known) = self.bounds.by_layer.get(&at) {
            return known.clone();
        }
        let slant = self.slant(layer);
        let measured = bounds_with(self.font, layer, slant);
        if layer.shapes.iter().any(|s| matches!(s, Shape::Component(_))) {
            self.bounds.composites.push(at);
        }
        self.bounds.by_layer.insert(at, measured.clone());
        measured
    }

    /// A layer's current metric (`None`: a sidebearing of an empty layer).
    pub(crate) fn metric(&mut self, glyph: &Glyph, layer: &Layer, field: Field) -> Result<Option<f64>, EvalError> {
        let b = if field.is_sidebearing() {
            self.bounds(glyph, layer)?
        } else {
            self.read(glyph, layer);
            None
        };
        Ok(metric_from(self.font, layer, field, b))
    }

    /// The left or right sidebearing at height `y`, else at the bounds.
    fn metric_at(&mut self, glyph: &Glyph, layer: &Layer, field: Field, y: f64) -> Result<Option<f64>, EvalError> {
        self.read(glyph, layer);
        let path = outline(self.font, layer)?;
        match (at_height(self.font, layer, &path, y), field) {
            (Some((first, _)), Field::Left) => Ok(Some(first)),
            (Some((_, last)), Field::Right) => Ok(Some(layer.width - last)),
            _ => self.metric(glyph, layer, field),
        }
    }

    /// The value of `key` in `field` of `layer`: checked, rounded and range-checked.
    pub(crate) fn value(&mut self, glyph: &Glyph, layer: &Layer, field: Field, key: &Key) -> Result<f64, EvalError> {
        self.check_chain(glyph, layer, field, key)?;
        let raw = match &key.body {
            Body::Constant(v) | Body::Signed(v) => *v,
            Body::SelfMirror => {
                let v = self.metric(glyph, layer, field.opposite());
                self.record(glyph, layer, field.opposite(), None, &v);
                v?.ok_or_else(|| EvalError::Empty(glyph.name.clone()))?
            }
            Body::Name { name, mirrored, op, .. } => {
                let font = self.font;
                let r = font.glyph(name).ok_or_else(|| EvalError::Unlinked(name.clone()))?;
                let rl = referenced_layer(r, layer).ok_or_else(|| EvalError::NoLayer(name.clone()))?;
                let f = if *mirrored { field.opposite() } else { field };
                let at = key.at.filter(|_| matches!(f, Field::Left | Field::Right));
                let v = match at {
                    Some(y) => self.metric_at(r, rl, f, y),
                    None => self.metric(r, rl, f),
                };
                self.record(r, rl, f, at, &v);
                let v = v?.ok_or_else(|| EvalError::Empty(name.clone()))?;
                match op {
                    None => v,
                    Some(o) => match o.op {
                        Op::Add => v + o.operand,
                        Op::Sub => v - o.operand,
                        Op::Mul => v * o.operand,
                        Op::Div if o.operand == 0.0 => return Err(EvalError::DivisionByZero),
                        Op::Div => v / o.operand,
                    },
                }
            }
        };
        if !raw.is_finite() {
            return Err(EvalError::NotFinite);
        }
        let value = round_half_up(raw);
        let range = field.range();
        if !range.contains(&value) {
            return Err(EvalError::OutOfRange { value, min: *range.start(), max: *range.end() });
        }
        Ok(value)
    }

    /// The metric `key` reads, one step: a glyph's layer and a field.
    fn step(&self, layer: &Layer, field: Field, key: &Key, own: &'a Glyph) -> Option<(&'a Glyph, Field)> {
        match &key.body {
            Body::SelfMirror => Some((own, field.opposite())),
            Body::Name { name, mirrored, .. } => {
                let r = self.font.glyph(name)?;
                referenced_layer(r, layer)?;
                Some((r, if *mirrored { field.opposite() } else { field }))
            }
            Body::Constant(_) | Body::Signed(_) => None,
        }
    }

    /// Follows the references from `key`: an error when they come back to where they started or
    /// run deeper than [`MAX_CHAIN`] steps. A loop that does not pass the start belongs to other
    /// keys, which report it themselves.
    fn check_chain(&mut self, glyph: &Glyph, layer: &Layer, field: Field, key: &Key) -> Result<(), EvalError> {
        let font = self.font;
        let Some(own) = font.glyph(&glyph.name) else { return Ok(()) };
        let start = (address(layer), field);
        let mut seen = vec![start];
        let mut at_layer: &Layer = layer;
        let mut next = self.step(layer, field, key, own);
        let mut steps = 0usize;
        while let Some((g, f)) = next {
            let Some(l) = referenced_layer(g, at_layer) else { break };
            steps += 1;
            if steps > MAX_CHAIN {
                return Err(EvalError::TooDeep);
            }
            let node = (address(l), f);
            if node == start {
                return Err(EvalError::Cycle);
            }
            if seen.contains(&node) {
                break;
            }
            seen.push(node);
            if self.collect == Collect::ReadsAndNames && !self.deps.contains(&g.name) {
                self.deps.insert(g.name.clone());
            }
            // Alignment sets this metric: the key stored there does not continue the chain.
            if self.derived_field(l, f) {
                break;
            }
            let Some((scope, text)) = effective_key(g, l, f) else { break };
            let Ok(k) = self.parse(text, scope, f) else { break };
            next = self.step(l, f, &k, g);
            at_layer = l;
        }
        Ok(())
    }
}
