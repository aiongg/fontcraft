//! The last feature compile of each open document (M8.15; `docs/plans/M8.md` § 3, The preview
//! font): the preview font, its feature list and the diagnostics, for the Features menu, Edit
//! View shaping, inline errors and `font.features.listErrors`.
//!
//! Rows: `features.menuNeedsCompile` (the state), `fontInfo.features.inlineErrors`,
//! `textPreview.shaping` (the shared build).
//!
//! Session state only: not undoable, never saved, dropped when the document closes. A compile
//! records the document revision it read and a hash of what it compiled (the assembled code,
//! the glyph order with the code points, the master), so `font.features.compile` asked again for
//! the same code reuses the result instead of compiling it again. The result describes the font
//! while that hash and the hash of the entries' stored code are unchanged
//! ([`FeatureState::is_current`]): moving a node or editing a width keeps the diagnostics, an
//! edit to the code or to the glyph set does not. One compile is kept per open document.
//!
//! The code is compiled once per document revision and master: `font.features.compile` and
//! Text Preview both build through [`build_previews`], which gives Edit View's font, Text
//! Preview's font and the diagnostics from one compile, and each leaves its build in
//! [`PreviewBuilds`] for the other to take.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fontcraft_compile::preview::{PreviewBuild, PreviewFont};
use fontcraft_compile::{CompileError, Options};
use fontcraft_fea::FeatureSummary;
use fontcraft_features::EntryDiagnostic;
use fontcraft_model::Font;

use crate::cmd::registry::SlotInit;
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
    /// The hash of the stored code of every entry, which the diagnostics' places are ranges of.
    pub code_hash: u64,
}

impl FeatureState {
    /// The compile describes the document as it is now: it read this revision, or the compiler's input is the one it
    /// read ([`FeatureState::source_hash`]). The answer for a later revision is kept per document, so asking again
    /// each frame assembles the code once per edit.
    pub fn is_current(&self, s: &Session, doc: &DocState) -> bool {
        if self.revision == doc.revision {
            return true;
        }
        // Zero: the code did not assemble, and nothing was hashed.
        if self.source_hash == 0 {
            return false;
        }
        let answers = s.slot::<FeaturesSlot>().map(|f| f.answers.clone()).unwrap_or_default();
        let key = (doc.revision, self.source_hash);
        if let Some(known) = answers.lock().ok().and_then(|a| a.get(&doc.id).filter(|(k, _)| *k == key).map(|(_, same)| *same)) {
            return known;
        }
        let same = crate::cmd::features_compile::input_hash(s, doc, &self.master) == Some((self.source_hash, self.code_hash));
        if let Ok(mut a) = answers.lock() {
            a.insert(doc.id, (key, same));
        }
        same
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.diagnostic.is_error())
    }
}

/// One preview build of a document revision and master, or why it failed.
#[derive(Clone, Debug)]
pub struct SharedBuild {
    pub doc: u64,
    /// The document revision ([`DocState::revision`]) the build read.
    pub revision: u64,
    pub master: String,
    /// The hash `font.features.compile` keys its compiles with (see [`FeatureState::source_hash`]);
    /// `None` for a build Text Preview made, which reads included files without hashing them.
    pub source_hash: Option<u64>,
    pub result: Result<Arc<PreviewBuild>, Arc<CompileError>>,
}

/// The last preview build of each document, shared by `font.features.compile` and Text Preview,
/// which builds on a thread of its own: a handle to one store that every clone sees.
#[derive(Clone, Debug, Default)]
pub struct PreviewBuilds(Arc<Mutex<HashMap<u64, Arc<SharedBuild>>>>);

impl PreviewBuilds {
    /// The build of `doc` at `revision` for `master`, if one was kept.
    pub fn get(&self, doc: u64, revision: u64, master: &str) -> Option<Arc<SharedBuild>> {
        let builds = self.0.lock().ok()?;
        builds.get(&doc).filter(|b| b.revision == revision && b.master == master).cloned()
    }

    /// Keeps `build` as its document's last build, unless a build of a later revision is kept.
    pub fn put(&self, build: SharedBuild) {
        // A poisoned store is only a cache: the build is not kept.
        let Ok(mut builds) = self.0.lock() else { return };
        if builds.get(&build.doc).is_some_and(|b| b.revision > build.revision) {
            return;
        }
        builds.insert(build.doc, Arc::new(build));
    }

    /// Drops the builds of documents not in `open`.
    fn retain(&self, open: &[u64]) {
        if let Ok(mut builds) = self.0.lock() {
            builds.retain(|id, _| open.contains(id));
        }
    }
}

/// Both preview fonts of `master` and the code's diagnostics, from one compile of the feature
/// code (`fontcraft_compile::preview::layout_fonts_with`).
pub fn build_previews(font: &Font, master: &str, options: &Options) -> Result<PreviewBuild, CompileError> {
    fontcraft_compile::preview::layout_fonts_with(font, master, options)
}

/// Whether a document's last compile still describes it, by document id: for (revision, source hash).
type Answers = Arc<Mutex<HashMap<u64, ((u64, u64), bool)>>>;

/// Builds the preview fonts of document `doc` at `revision` for `master` and keeps the build in `builds` under the
/// input hash `font.features.compile` keys its compiles with, so a compile of the same revision that follows takes it
/// instead of compiling: the Features tab runs this on a worker thread after the code changes. `options` are the ones
/// the compile reads (the glyph data and the include folder of `crate::cmd::features_check::check_context`). Nothing is
/// kept when the code does not assemble or includes files the hash cannot cover.
pub fn prebuild(font: &Font, doc: u64, revision: u64, master: &str, options: &Options, builds: &PreviewBuilds) {
    let Some(data) = options.glyph_data.0.as_ref() else { return };
    let Ok(values) = fontcraft_features::MasterValues::new(font, data, master) else { return };
    let Ok(prepared) = crate::cmd::features_compile::prepare(font, &font.features, master, options.feature_dir.as_deref(), &values) else {
        return;
    };
    if !prepared.keyed {
        return;
    }
    let result = build_previews(font, master, options).map(Arc::new).map_err(Arc::new);
    builds.put(SharedBuild { doc, revision, master: master.to_string(), source_hash: Some(prepared.hash), result });
}

/// The features area's session slot.
#[derive(Default)]
struct FeaturesSlot {
    states: HashMap<u64, Arc<FeatureState>>,
    preview_builds: PreviewBuilds,
    /// [`FeatureState::is_current`]'s answers, kept through `&Session`.
    answers: Answers,
}

/// Every session starts with the features slot, so the preview builds store exists before
/// Text Preview, which reads it through `&Session`, first asks for it.
pub(crate) fn slot_init() -> SlotInit {
    SlotInit::of::<FeaturesSlot>()
}

/// The last feature compile and the preview builds of each open document.
pub trait SessionFeaturesExt {
    /// The store of preview builds that `font.features.compile` and Text Preview share.
    fn preview_builds(&self) -> PreviewBuilds;

    /// The active document's last feature compile, if it has one.
    fn feature_state(&self) -> Option<Arc<FeatureState>>;

    /// The last feature compile of the document with this id.
    fn feature_state_of(&self, doc_id: u64) -> Option<Arc<FeatureState>>;

    /// Keeps `state` as the document's last compile, replacing the one before; states of
    /// documents no longer open are dropped.
    fn set_feature_state(&mut self, doc_id: u64, state: FeatureState);

    /// Forgets the document's last compile.
    fn clear_feature_state(&mut self, doc_id: u64);
}

impl SessionFeaturesExt for Session {
    fn preview_builds(&self) -> PreviewBuilds {
        self.slot::<FeaturesSlot>().map(|f| f.preview_builds.clone()).unwrap_or_default()
    }

    fn feature_state(&self) -> Option<Arc<FeatureState>> {
        self.active().and_then(|doc| self.feature_state_of(doc.id))
    }

    fn feature_state_of(&self, doc_id: u64) -> Option<Arc<FeatureState>> {
        self.slot::<FeaturesSlot>()?.states.get(&doc_id).cloned()
    }

    fn set_feature_state(&mut self, doc_id: u64, state: FeatureState) {
        let open: Vec<u64> = self.documents().iter().map(|d| d.id).collect();
        let slot = self.slot_mut::<FeaturesSlot>();
        slot.states.retain(|id, _| open.contains(id));
        slot.preview_builds.retain(&open);
        if let Ok(mut answers) = slot.answers.lock() {
            answers.retain(|id, _| open.contains(id));
        }
        if open.contains(&doc_id) {
            slot.states.insert(doc_id, Arc::new(state));
        }
    }

    fn clear_feature_state(&mut self, doc_id: u64) {
        self.slot_mut::<FeaturesSlot>().states.remove(&doc_id);
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
            FeatureState {
                revision,
                master: String::new(),
                preview: None,
                features: Vec::new(),
                diagnostics: Vec::new(),
                source_hash: 0,
                code_hash: 0,
            },
        );
        let Some(state) = s.feature_state() else { panic!("no state kept") };
        assert!(state.is_current(&s, s.doc().unwrap()));
        s.edit("Rename", |f| {
            f.family_name = "Other".into();
            Ok(())
        })
        .unwrap();
        // Zero: a state that hashed nothing describes only the revision it read.
        assert!(!state.is_current(&s, s.doc().unwrap()));
        s.set_feature_state(id + 1000, state.as_ref().clone());
        assert!(s.feature_state_of(id + 1000).is_none());
    }
}
