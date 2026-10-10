//! Every glyph's effective properties, kept across edits. Copy-on-write keeps the `Arc` of each
//! glyph an edit did not touch, so a glyph whose `Arc` is the same has the same properties as
//! long as the glyph data and the *Keep Alternates Next to Base Glyph* setting are the same too.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Weak};

use fontcraft_glyphdata::GlyphData;

use crate::{Font, Glyph, GlyphProps, glyph_props, keeps_alternates};

/// The effective properties of one font's glyphs for one glyph data. [`PropsCache::update`]
/// recomputes only the glyphs that changed and drops the glyphs the font no longer has, so the
/// cache never holds more entries than the font has glyphs.
#[derive(Clone, Default)]
pub struct PropsCache {
    /// Held so its address cannot be reused while the entries depend on it.
    data: Option<Arc<GlyphData>>,
    keep_alternates: bool,
    /// The font of the last update; the `Weak` keeps its address from being reused.
    font: Weak<Font>,
    /// By the address of each glyph.
    entries: HashMap<usize, Kept>,
    /// Marks the entries the last update saw.
    stamp: u64,
    computed: u64,
}

#[derive(Clone)]
struct Kept {
    /// Held so the address that keys the entry stays unique.
    _glyph: Arc<Glyph>,
    props: Arc<GlyphProps>,
    stamp: u64,
}

fn address(glyph: &Glyph) -> usize {
    std::ptr::from_ref(glyph) as usize
}

impl PropsCache {
    /// Brings the cache up to date with `font` and `data`.
    pub fn update(&mut self, font: &Arc<Font>, data: &Arc<GlyphData>) {
        let keep = keeps_alternates(font);
        let same_data = self.data.as_ref().is_some_and(|d| Arc::ptr_eq(d, data)) && self.keep_alternates == keep;
        if same_data && std::ptr::eq(self.font.as_ptr(), Arc::as_ptr(font)) {
            return;
        }
        if !same_data {
            self.entries.clear();
        }
        self.stamp = self.stamp.wrapping_add(1);
        for glyph in font.glyphs.as_arcs() {
            match self.entries.entry(address(glyph)) {
                Entry::Occupied(mut e) => e.get_mut().stamp = self.stamp,
                Entry::Vacant(e) => {
                    self.computed = self.computed.saturating_add(1);
                    e.insert(Kept { _glyph: glyph.clone(), props: Arc::new(glyph_props(glyph, data, keep)), stamp: self.stamp });
                }
            }
        }
        if self.entries.len() > font.glyphs.len() {
            let stamp = self.stamp;
            self.entries.retain(|_, e| e.stamp == stamp);
        }
        self.data = Some(data.clone());
        self.keep_alternates = keep;
        self.font = Arc::downgrade(font);
    }

    /// The properties of `glyph`, if it is one of the font's glyphs as of the last update.
    pub fn get(&self, glyph: &Glyph) -> Option<&Arc<GlyphProps>> {
        self.entries.get(&address(glyph)).map(|e| &e.props)
    }

    /// Whether the cache was computed from `data` with this *Keep Alternates* setting.
    pub fn is_for(&self, data: &GlyphData, keep_alternates: bool) -> bool {
        self.keep_alternates == keep_alternates && self.data.as_ref().is_some_and(|d| std::ptr::eq(Arc::as_ptr(d), data))
    }

    /// How many glyphs' properties were computed since the cache was made.
    pub fn computed(&self) -> u64 {
        self.computed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Arc<Font> {
        let mut f = Font::new("Test");
        f.glyphs.retain(|g| ["A", "a", "one"].contains(&g.name.as_str()));
        Arc::new(f)
    }

    #[test]
    fn only_changed_glyphs_and_new_data_are_computed_again() {
        let data = Arc::new(GlyphData::new().unwrap());
        let mut font = font();
        let n = font.glyphs.len() as u64;
        assert!(n >= 2, "the test font has glyphs");
        let mut cache = PropsCache::default();
        cache.update(&font, &data);
        assert_eq!(cache.computed(), n);
        let a = font.glyph("A").unwrap();
        assert_eq!(cache.get(a).unwrap().category.as_deref(), Some("Letter"));

        // Another font value with the same glyphs: nothing is computed.
        let copy = Arc::new(Font::clone(&font));
        cache.update(&copy, &data);
        assert_eq!(cache.computed(), n);

        // One glyph changed: only it is computed, with its new properties.
        Arc::make_mut(&mut font).glyph_mut("A").unwrap().info.category = Some("Symbol".into());
        cache.update(&font, &data);
        assert_eq!(cache.computed(), n + 1);
        assert_eq!(cache.get(font.glyph("A").unwrap()).unwrap().category.as_deref(), Some("Symbol"));

        // New glyph data (a custom file added): every glyph is computed from it.
        let custom = br#"<glyphData><glyph name="a" category="Mark"/></glyphData>"#;
        let merged = Arc::new(GlyphData::with_custom_files(&[custom]).unwrap());
        assert!(!cache.is_for(&merged, false));
        cache.update(&font, &merged);
        assert_eq!(cache.computed(), 2 * n + 1);
        assert!(cache.is_for(&merged, false));
        assert_eq!(cache.get(font.glyph("a").unwrap()).unwrap().category.as_deref(), Some("Mark"));

        // A removed glyph is dropped.
        Arc::make_mut(&mut font).glyphs.retain(|g| g.name != "A");
        cache.update(&font, &merged);
        assert_eq!(cache.entries.len() as u64, n - 1);
    }
}
