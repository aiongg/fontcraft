//! The engine's export stage (`docs/plans/M7.md` § 3 › Export-time parameters): the export-time
//! parameters that need the engine, run on a copy of the font for one job before the compiler
//! resolves it. In order:
//!
//! 1. linked sources: Import Font and Import Master refreshed from their files ([`crate::linked`]);
//! 2. PreFilter strings (static exports only) on the target master's layers;
//! 3. Scale to UPM, through `fontcraft_fontinfo::scale`;
//! 4. Filter strings, after every composite of the target layers is decomposed.
//!
//! Parameters come from the font, the master and the instance, in that order; at most
//! [`fontcraft_fontinfo::MAX_FILTER_STRINGS`] Filter and PreFilter strings together. Export Folder
//! ([`export_folder`]) names the sub-folder of the destination the job's files go to. The
//! compiler's stage (glyph sets, names, the `kern` table) runs after resolving, in
//! `fontcraft_compile::export_params`.
//!
//! File › Export, `font.compile`, the CLI, the control channel and the oracle harness all build
//! through `cmd::compile::build`, which calls [`prepare`] for every job.
//!
//! Rows: `Filter`, `PreFilter`, the `filter.customParameter.*` rows, `export.instanceExportFolder`
//! (M7.10); `Import Font`, `Import Master` (M7.12); `Scale to UPM` (M7.4 scales, M7.10 runs it).

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_compile::Options;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Resolved};

use crate::Result;
use crate::cmd::compile::Job;

/// What the steps may read besides the font.
pub(crate) struct StepContext<'a> {
    /// The document's glyph database (filters that look up glyph data, decomposition).
    pub data: &'a Arc<GlyphData>,
    /// The document's file, which relative linked-source paths start from; `None` for a document
    /// never saved.
    pub path: Option<&'a Path>,
}

/// The font `job` is built from: `font` itself when no step applies, else a changed copy.
/// Warnings name the parameters that could not be used.
pub(crate) fn prepare<'f>(font: &'f Font, job: &Job, ctx: &StepContext, warnings: &mut Vec<String>) -> Result<Cow<'f, Font>> {
    let mut font = Cow::Borrowed(font);
    if let Some(linked) = crate::linked::resolve(&font, ctx.path, warnings)? {
        font = Cow::Owned(linked);
    }
    // M7.10: PreFilter, Scale to UPM, Filter, for `job.target`.
    let _ = (job, ctx.data);
    Ok(font)
}

/// The sub-folder of the destination the files of `resolved` go to (its Export Folder
/// parameter), relative, or `None` for the destination itself. An absolute path, more than
/// [`fontcraft_fontinfo::MAX_EXPORT_FOLDER_SEGMENTS`] segments or
/// [`fontcraft_fontinfo::MAX_EXPORT_FOLDER_BYTES`] bytes is refused.
pub(crate) fn export_folder(resolved: &Resolved, options: &Options) -> Result<Option<PathBuf>> {
    // M7.10: the parameter, `..` and the guess's rules.
    let _ = (resolved, options);
    Ok(None)
}
