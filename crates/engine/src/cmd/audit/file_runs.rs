//! The last `audit.run` of each file target (M12.84): what `audit.results` returns for a path and
//! what a `runId` is checked against.
//!
//! The runs live in a session slot, keyed by the resolved path, at most [`MAX_FILE_RUNS`] of
//! them. They hold the result document only; a file that changed on disk since makes every
//! result stale.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_model::{Font, SourceFormat};
use serde_json::Value;

use super::super::file_io::resolved;
use crate::Session;
#[cfg(not(target_arch = "wasm32"))]
use crate::io::Stamp;

/// No file is read in the browser, so no run of one is kept there.
#[cfg(target_arch = "wasm32")]
type Stamp = ();

/// Most file targets whose last run is kept; the one audited longest ago is dropped first.
const MAX_FILE_RUNS: usize = 8;

/// The kept runs, the newest last.
#[derive(Default)]
struct FileRuns(Vec<FileRun>);

/// The last run of one file target.
pub(super) struct FileRun {
    path: PathBuf,
    stamp: Option<Stamp>,
    /// The font read, when its files lie beside the one the target names.
    designspace: Option<Arc<Font>>,
    pub(super) report: Value,
    run_id: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn stamp_of(path: &Path, designspace: Option<&Font>) -> Option<Stamp> {
    match designspace {
        Some(font) => crate::io::document_stamp(path, font, font.source.format),
        None => crate::io::stamp(path),
    }
}

#[cfg(target_arch = "wasm32")]
fn stamp_of(_: &Path, _: Option<&Font>) -> Option<Stamp> {
    None
}

impl FileRun {
    /// A run of the target `path`, whose first font is `font`, as the files are now; its results
    /// come with [`FileRun::set_report`].
    pub(super) fn new(path: &str, font: &Arc<Font>) -> FileRun {
        let path = resolved(Path::new(path));
        let designspace = (font.source.format == SourceFormat::Designspace).then(|| font.clone());
        let stamp = stamp_of(&path, designspace.as_deref());
        FileRun { path, stamp, designspace, report: Value::Null, run_id: String::new() }
    }

    pub(super) fn set_report(&mut self, report: Value, run_id: String) {
        self.report = report;
        self.run_id = run_id;
    }

    /// Have the files changed since the run?
    pub(super) fn stale(&self) -> bool {
        stamp_of(&self.path, self.designspace.as_deref()) != self.stamp
    }
}

/// Keeps `run` in place of the one kept for the same path.
pub(super) fn keep(s: &mut Session, run: FileRun) {
    let runs = &mut s.slot_mut::<FileRuns>().0;
    runs.retain(|r| r.path != run.path);
    if runs.len() >= MAX_FILE_RUNS {
        runs.drain(..runs.len().saturating_sub(MAX_FILE_RUNS).saturating_add(1));
    }
    runs.push(run);
}

/// The run kept for `path`.
pub(super) fn find<'s>(s: &'s Session, path: &str) -> Option<&'s FileRun> {
    let path = resolved(Path::new(path));
    s.slot::<FileRuns>()?.0.iter().find(|r| r.path == path)
}

/// Drops the run kept for `path`; whether there was one.
pub(super) fn clear(s: &mut Session, path: &str) -> bool {
    let path = resolved(Path::new(path));
    let runs = &mut s.slot_mut::<FileRuns>().0;
    let before = runs.len();
    runs.retain(|r| r.path != path);
    runs.len() != before
}

/// The id of the run kept for `path`.
pub(super) fn run_id<'s>(s: &'s Session, path: &str) -> Option<&'s str> {
    find(s, path).map(|r| r.run_id.as_str())
}

/// The message when no run of `path` is kept.
pub(super) fn no_run(path: &str) -> String {
    format!("no audit of {path} is kept: run audit.run on it first")
}
