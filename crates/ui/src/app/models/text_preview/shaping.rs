//! The preview font Text Preview shapes with (`textPreview.shaping`): a full preview font of the shown master
//! (`GSUB` and `GPOS` with the generated kerning and marks), built through the host's jobs.
//!
//! A font is built at once when the preview first needs one for a document and master, then each time the document
//! changes and editing has paused for [`REBUILD_PAUSE`] seconds; until then the last font of the same document and
//! master shapes the text. A build that `font.features.compile` or an earlier build left in the session's
//! `PreviewBuilds` is taken instead of compiling again.

use std::sync::{Arc, Mutex};

use craft::host::Host;
use fontcraft_engine::Session;
use fontcraft_engine::compile::CompileError;
use fontcraft_engine::compile::preview::PreviewBuild;
use fontcraft_engine::feature_state::{PreviewBuilds, SharedBuild};
use fontcraft_engine::prelude::*;
use fontcraft_model::Font;
use fontcraft_textlayout::PreviewShaper;

/// How long editing must pause, in seconds, before the preview font is built again.
pub const REBUILD_PAUSE: f64 = 0.5;
/// How often a frame looks for a finished build, in seconds.
const BUILD_POLL: f64 = 0.03;

/// What a preview font is built for: the document's id and revision, and the master.
pub type FontKey = (u64, u64, String);
/// A built preview font's shaper, or why it could not be built.
type FontResult = Result<Arc<PreviewShaper>, String>;
/// Where a build left its result.
type Landing = Arc<Mutex<Option<FontResult>>>;

/// The preview font: the last build, the key asked for and the build under way.
#[derive(Default)]
pub struct FontSlot {
    built: Option<(FontKey, FontResult)>,
    /// The key asked for last, and the time it was first asked for.
    wanted: Option<(FontKey, f64)>,
    running: Option<(FontKey, Landing)>,
}

impl FontSlot {
    /// Takes the result of a finished build.
    fn collect(&mut self) {
        let Some((key, landing)) = &self.running else { return };
        let done = landing.lock().ok().and_then(|mut l| l.take());
        if let Some(result) = done {
            self.built = Some((key.clone(), result));
            self.running = None;
        }
    }

    /// Starts a build when `key` has no font yet: at once for a document or master without one, else once `key` has
    /// been asked for since [`REBUILD_PAUSE`] seconds (each edit makes a new key). Returns the shaper to draw with
    /// (the last font built for the same document and master, even when edits have made it older than the
    /// document; `None` while there is none) and how many seconds until a frame should look again.
    pub fn step(&mut self, key: &FontKey, now: f64, session: &Session, host: &Host) -> (Option<Arc<PreviewShaper>>, Option<f64>) {
        self.collect();
        let mut wake = None;
        let fresh = self.built.as_ref().is_some_and(|(k, _)| k == key);
        let under_way = self.running.as_ref().is_some_and(|(k, _)| k == key);
        let kept = (!fresh && !under_way && self.running.is_none()).then(|| session.preview_builds().get(key.0, key.1, &key.2)).flatten();
        if let Some(kept) = kept {
            self.running = Some((key.clone(), start(host, key.clone(), Source::Kept(kept))));
            self.collect();
        } else if !fresh && !under_way {
            if self.wanted.as_ref().is_none_or(|(k, _)| k != key) {
                self.wanted = Some((key.clone(), now));
            }
            let since = self.wanted.as_ref().map_or(0.0, |(_, at)| now - at);
            let editing = self.built.as_ref().is_some_and(|(k, _)| k.0 == key.0 && k.2 == key.2);
            let wait = if editing { REBUILD_PAUSE - since } else { 0.0 };
            if self.running.is_none() && wait <= 0.0 {
                if let Some(doc) = session.active() {
                    let source =
                        Source::Build { font: doc.font.clone(), options: Box::new(build_options(session)), builds: session.preview_builds() };
                    self.running = Some((key.clone(), start(host, key.clone(), source)));
                    self.collect();
                }
            } else if wait > 0.0 {
                wake = Some(wait.clamp(BUILD_POLL, REBUILD_PAUSE));
            }
        }
        if self.running.is_some() {
            wake = Some(BUILD_POLL);
        }
        let shaper =
            self.built.as_ref().and_then(|(built, result)| (built.0 == key.0 && built.2 == key.2).then(|| result.as_ref().ok().cloned()).flatten());
        (shaper, wake)
    }

    /// Why the font for `key`'s document and master could not be built.
    pub fn error(&self, key: &FontKey) -> Option<&str> {
        match &self.built {
            Some((k, Err(e))) if k.0 == key.0 && k.2 == key.2 => Some(e),
            _ => None,
        }
    }

    /// A build is under way.
    pub fn building(&self) -> bool {
        self.running.is_some()
    }
}

/// The compile options a preview build reads: the session's glyph data and the folder the source's feature code
/// includes files from, as `font.features.compile` reads them.
pub fn build_options(session: &Session) -> fontcraft_engine::compile::Options {
    let path = session.active().and_then(|d| d.path.clone());
    fontcraft_engine::compile::Options {
        glyph_data: fontcraft_engine::compile::DataRef(session.glyph_data().ok()),
        feature_dir: feature_dir(path.as_deref()),
        include_check: fontcraft_engine::file_access::include_check(),
        ..fontcraft_engine::compile::Options::default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn feature_dir(path: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    path.and_then(fontcraft_engine::io::feature_dir).filter(|d| fontcraft_engine::file_access::check_read(d).is_ok())
}

/// The web build has no file system to read includes from.
#[cfg(target_arch = "wasm32")]
fn feature_dir(_path: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    None
}

/// What a build starts from.
enum Source {
    /// A build of the key that is already kept: only its shaper is made.
    Kept(Arc<SharedBuild>),
    /// The font to build the preview fonts of, and the store the build is left in for `font.features.compile`.
    Build { font: Arc<Font>, options: Box<fontcraft_engine::compile::Options>, builds: PreviewBuilds },
}

/// The shaper of a build's full preview font, or why there is none.
fn shaper_of(result: &Result<Arc<PreviewBuild>, Arc<CompileError>>) -> FontResult {
    let built = result.as_ref().map_err(|e| format!("The text is not shaped: {e}"))?;
    let full = &built.full;
    let tags: Vec<fontcraft_textlayout::shaping::Tag> = full.features.iter().map(|f| f.tag).collect();
    PreviewShaper::new(Arc::<[u8]>::from(full.bytes.as_slice()), &full.glyph_order, &full.private_use, &tags)
        .map(Arc::new)
        .map_err(|e| format!("The text is not shaped: {e}"))
}

/// The full preview font of `key` and its shaper: from the kept build, or from a new build (one compile of the
/// feature code, `fontcraft_engine::feature_state::build_previews`) that is kept in turn.
fn build_font(key: &FontKey, source: &Source) -> FontResult {
    match source {
        Source::Kept(kept) => shaper_of(&kept.result),
        Source::Build { font, options, builds } => {
            let result = fontcraft_engine::feature_state::build_previews(font, &key.2, options).map(Arc::new).map_err(Arc::new);
            builds.put(SharedBuild { doc: key.0, revision: key.1, master: key.2.clone(), source_hash: None, result: result.clone() });
            shaper_of(&result)
        }
    }
}

/// Builds the preview font through the host's jobs (a thread of its own on the desktop, at once on the web and in
/// tests with inline jobs); the result lands in the returned slot.
fn start(host: &Host, key: FontKey, source: Source) -> Landing {
    let landing: Landing = Arc::default();
    let out = landing.clone();
    let job = move || {
        let result = fontcraft_engine::guard::catch_panic(|| build_font(&key, &source))
            .unwrap_or_else(|e| Err(format!("The text is not shaped: the preview font could not be built ({e})")));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result);
        }
    };
    host.jobs().spawn("text preview font", Box::new(job));
    landing
}
