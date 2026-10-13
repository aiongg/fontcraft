//! The composite outline of a glyph (`compositing.shapesCombineInOrder`): its shapes in stored
//! order, stroked paths expanded and masks applied, as `fontcraft_engine::cmd::stroke` builds it.
//! The canvas fills it and draws it in black.
//!
//! Strokes and masks take boolean operations and the canvas draws every frame, so composites are
//! cached per glyph and layer. An entry stays valid while the document revision is unchanged,
//! and across edits to other glyphs while its own layer's shapes are unchanged and hold no
//! component (a component's outline depends on another glyph).

use std::collections::HashMap;

use fontcraft_engine::cmd::stroke::{format_version, layer_outline, mask_mode, stroke_style};
use fontcraft_geom::BezPath;
use fontcraft_model::{Font, Layer, Shape};

/// Most glyphs the cache holds; past this it starts again empty.
const MAX_ENTRIES: usize = 1024;

/// One glyph's composite in one layer.
#[derive(Debug)]
pub struct Composite {
    pub outline: BezPath,
    /// The layer has a stroked path or a mask, so the composite differs from the paths as drawn.
    pub styled: bool,
}

struct Entry {
    /// Document id and revision the entry was built at.
    doc: (u64, u64),
    version: u32,
    shapes: Vec<Shape>,
    components: bool,
    composite: Composite,
}

#[derive(Default)]
pub struct CompositeCache {
    entries: HashMap<(String, String), Entry>,
}

impl CompositeCache {
    /// The composite of `layer` of `glyph` in document `doc` (id, revision); `None` when its
    /// outline cannot be built. A composite that fails (a stroke size given as an expression,
    /// say) falls back to the plain outline.
    pub fn get(&mut self, doc: (u64, u64), font: &Font, glyph: &str, layer: &Layer) -> Option<&Composite> {
        let version = format_version(font);
        let key = (glyph.to_string(), layer.id.clone());
        let fresh = self
            .entries
            .get(&key)
            .is_some_and(|e| e.doc == doc || (e.doc.0 == doc.0 && e.version == version && !e.components && e.shapes == layer.shapes));
        if !fresh {
            let outline = match layer_outline(font, layer) {
                Ok(parts) => parts.into_iter().fold(BezPath::new(), |mut all, p| {
                    all.extend(p.elements().iter().copied());
                    all
                }),
                Err(_) => plain_outline(font, glyph, layer)?,
            };
            if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
                self.entries.clear();
            }
            let components = layer.shapes.iter().any(|s| matches!(s, Shape::Component(_)));
            let composite = Composite { outline, styled: styled(layer, version) };
            self.entries.insert(key.clone(), Entry { doc, version, shapes: layer.shapes.clone(), components, composite });
        }
        let entry = self.entries.get_mut(&key)?;
        entry.doc = doc;
        Some(&entry.composite)
    }
}

/// The outline of `layer` of `glyph` with its components resolved, without strokes or masks.
pub fn plain_outline(font: &Font, glyph: &str, layer: &Layer) -> Option<BezPath> {
    if layer.is_master_layer() {
        return font.outline(glyph, &layer.master).ok();
    }
    Some(super::background::outline(font, &layer.shapes, &layer.master))
}

/// Whether `layer` has a stroked path or a mask shape.
fn styled(layer: &Layer, version: u32) -> bool {
    layer.shapes.iter().any(|shape| match shape {
        Shape::Path(p) => mask_mode(&p.info.attrs, version).is_some() || !matches!(stroke_style(&p.info.attrs), Ok(None)),
        Shape::Component(c) => mask_mode(&c.info.attrs, version).is_some(),
        Shape::Image(_) | Shape::Raw(_) => false,
    })
}
