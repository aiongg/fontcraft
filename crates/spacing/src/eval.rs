//! Evaluating metrics keys.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::Rect;
use fontcraft_model::{Font, Glyph, Layer, MAX_COMPONENT_DEPTH, Shape};

use crate::aligned::derived_field;
use crate::keys::{Body, Key, KeyError, Op, Scope, parse_stored};
use crate::measure::{at_height, bounds, metric_from, outline};
use crate::{Field, MAX_CHAIN, referenced_layer, round_half_up};

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

/// One evaluation run over one font: measured bounds are kept, and the names of the glyphs read
/// are collected for the out-of-sync cache.
pub(crate) struct Ctx<'a> {
    pub(crate) font: &'a Font,
    /// By layer address.
    bounds: HashMap<usize, Result<Option<Rect>, EvalError>>,
    /// Glyphs whose data the results read.
    pub(crate) deps: HashSet<String>,
    /// Layers whose components were added to `deps`.
    walked: HashSet<usize>,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(font: &'a Font) -> Self {
        Self { font, bounds: HashMap::new(), deps: HashSet::new(), walked: HashSet::new() }
    }

    /// Starts collecting the reads of another result.
    pub(crate) fn forget_reads(&mut self) {
        self.deps.clear();
        self.walked.clear();
    }

    pub(crate) fn exists(&self, name: &str) -> bool {
        self.font.glyph(name).is_some()
    }

    fn read(&mut self, glyph: &Glyph, layer: &Layer) {
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
        let font = self.font;
        self.bounds.entry(address(layer)).or_insert_with(|| bounds(font, layer)).clone()
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
            Body::SelfMirror => self.metric(glyph, layer, field.opposite())?.ok_or_else(|| EvalError::Empty(glyph.name.clone()))?,
            Body::Name { name, mirrored, op, .. } => {
                let font = self.font;
                let r = font.glyph(name).ok_or_else(|| EvalError::Unlinked(name.clone()))?;
                let rl = referenced_layer(r, layer).ok_or_else(|| EvalError::NoLayer(name.clone()))?;
                let f = if *mirrored { field.opposite() } else { field };
                let v = match key.at {
                    Some(y) if matches!(f, Field::Left | Field::Right) => self.metric_at(r, rl, f, y)?,
                    _ => self.metric(r, rl, f)?,
                };
                let v = v.ok_or_else(|| EvalError::Empty(name.clone()))?;
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
            if !self.deps.contains(&g.name) {
                self.deps.insert(g.name.clone());
            }
            // Alignment sets this metric: the key stored there does not continue the chain.
            if derived_field(font, l, f) {
                break;
            }
            let Some((scope, text)) = effective_key(g, l, f) else { break };
            let Ok(k) = parse_stored(text, scope, f, |n| self.exists(n)) else { break };
            next = self.step(l, f, &k, g);
            at_layer = l;
        }
        Ok(())
    }
}
