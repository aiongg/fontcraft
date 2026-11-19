//! The spacing area's session state: each open document's kerning prepared for lookups and its
//! metrics keys that need attention. Both are caches kept per document id and brought up to
//! date with the font on every call, so a failing command leaves nothing stale behind.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_spacing::SyncCache;
use fontcraft_spacing::kerning::KernIndex;

use crate::{Result, Session};

#[derive(Default)]
struct SpacingState {
    metrics_sync: HashMap<u64, SyncCache>,
    kern_index: HashMap<u64, Arc<KernIndex>>,
}

/// The spacing caches of the active document.
pub trait SessionSpacingExt {
    /// The active document's metrics keys that are out of sync, unlinked or in error, up to
    /// date with its font. While the font is the one last scanned this returns at once; the first
    /// call after an edit scans again the glyphs that changed and the glyphs whose keys read them.
    fn metrics_sync(&mut self) -> Result<&SyncCache>;

    /// The active document's kerning prepared for lookups. The index is kept while it describes
    /// the font ([`KernIndex::is_current`]) and built again on the first call after an edit to
    /// kerning, masters or groups.
    fn kern_index(&mut self) -> Result<Arc<KernIndex>>;
}

impl SessionSpacingExt for Session {
    fn metrics_sync(&mut self) -> Result<&SyncCache> {
        let doc = self.doc()?;
        let (id, font) = (doc.id, doc.font.clone());
        let open: Vec<u64> = self.documents().iter().map(|d| d.id).collect();
        let state = self.slot_mut::<SpacingState>();
        state.metrics_sync.retain(|id, _| open.contains(id));
        let cache = state.metrics_sync.entry(id).or_default();
        cache.update(&font);
        Ok(cache)
    }

    fn kern_index(&mut self) -> Result<Arc<KernIndex>> {
        let doc = self.doc()?;
        let (id, font) = (doc.id, doc.font.clone());
        let open: Vec<u64> = self.documents().iter().map(|d| d.id).collect();
        let state = self.slot_mut::<SpacingState>();
        state.kern_index.retain(|id, _| open.contains(id));
        let index = match state.kern_index.get(&id) {
            Some(index) if index.is_current(&font) => index.clone(),
            _ => {
                let index = Arc::new(KernIndex::new(&font));
                state.kern_index.insert(id, index.clone());
                index
            }
        };
        Ok(index)
    }
}
