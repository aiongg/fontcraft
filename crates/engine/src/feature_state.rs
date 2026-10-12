//! The last feature compile of each open document (M8.15; `docs/plans/M8.md` § 3, The preview
//! font): the preview font, its feature list and the diagnostics, for the Features menu, Edit
//! View shaping, inline errors and `font.features.listErrors`.
//!
//! Rows: `features.menuNeedsCompile` (the state), `fontInfo.features.inlineErrors`.
//!
//! Session state only: not undoable, never saved, dropped when the document closes. A compile
//! records the document revision it read, so readers can tell whether the font changed since
//! ([`FeatureState::is_current`]), and a hash of what it compiled (the assembled code, the
//! glyph order, the master), so `font.features.compile` asked again for the same code reuses
//! the result instead of compiling it again. One compile is kept per open document.

use std::sync::Arc;

use fontcraft_compile::preview::PreviewFont;
use fontcraft_fea::FeatureSummary;
use fontcraft_features::EntryDiagnostic;

use crate::{DocState, Session};

/// The result of one `font.features.compile`.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureState {
    /// The document revision ([`DocState::revision`]) the compile read.
    pub revision: u64,
    /// The master compiled.
    pub master: String,
    /// The preview font; `None` when the compile failed.
    pub preview: Option<Arc<PreviewFont>>,
    /// The features of the preview font, in feature-list order (empty when it failed).
    pub features: Vec<FeatureSummary>,
    /// Errors and warnings, mapped to the entries. A diagnostic's index here is its id in
    /// `font.features.listErrors` and `applyFix`.
    pub diagnostics: Vec<EntryDiagnostic>,
    /// The hash of the compiler's input: the assembled files, the glyph order with the code
    /// points, the master and the include folder.
    pub source_hash: u64,
}

impl FeatureState {
    /// The compile read the document as it is now.
    pub fn is_current(&self, doc: &DocState) -> bool {
        self.revision == doc.revision
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.diagnostic.is_error())
    }
}

impl Session {
    /// The active document's last feature compile, if it has one.
    pub fn feature_state(&self) -> Option<Arc<FeatureState>> {
        self.active().and_then(|doc| self.feature_state_of(doc.id))
    }

    /// The last feature compile of the document with this id.
    pub fn feature_state_of(&self, doc_id: u64) -> Option<Arc<FeatureState>> {
        self.feature_state.get(&doc_id).cloned()
    }

    /// Keeps `state` as the document's last compile, replacing the one before; states of
    /// documents no longer open are dropped.
    pub fn set_feature_state(&mut self, doc_id: u64, state: FeatureState) {
        let open: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.feature_state.retain(|id, _| open.contains(id));
        if open.contains(&doc_id) {
            self.feature_state.insert(doc_id, Arc::new(state));
        }
    }

    /// Forgets the document's last compile.
    pub fn clear_feature_state(&mut self, doc_id: u64) {
        self.feature_state.remove(&doc_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compile_is_kept_per_document_and_goes_stale_with_an_edit() {
        let mut s = Session::with_document();
        let Some(id) = s.active().map(|d| d.id) else { panic!("no document") };
        let revision = s.doc().map(|d| d.revision).unwrap_or_default();
        s.set_feature_state(
            id,
            FeatureState { revision, master: String::new(), preview: None, features: Vec::new(), diagnostics: Vec::new(), source_hash: 0 },
        );
        let Some(state) = s.feature_state() else { panic!("no state kept") };
        assert!(state.is_current(s.doc().unwrap()));
        s.edit("Rename", |f| {
            f.family_name = "Other".into();
            Ok(())
        })
        .unwrap();
        assert!(!state.is_current(s.doc().unwrap()));
        s.set_feature_state(id + 1000, state.as_ref().clone());
        assert!(s.feature_state_of(id + 1000).is_none());
    }
}
