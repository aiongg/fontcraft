//! FontCraft engine (L5): a [`Session`] of open fonts, their undo history, and the command
//! registry. **Every user-visible action is a command** ([`cmd::CommandSpec`]): the UI, the CLI,
//! the control channel and the MCP server all call [`Session::execute`] with a command id and JSON
//! parameters, and get JSON back.
//!
//! Three properties hold for every command, and `tests/engine/command_sweep.rs` checks them:
//! - it never panics, whatever the parameters and the document state;
//! - when it returns `Err`, the session is exactly as it was before the call;
//! - when it returns `Ok`, every open font still passes [`fontcraft_model::Font::check`].
//!
//! How they are kept: fonts change only through [`Session::edit`], which works on a copy and
//! swaps it in after the checks pass; around that, [`Session::execute`] snapshots the session
//! before a command that is not a query and restores it on any error or internal failure. A
//! command that calls other commands and ignores an inner error keeps the inner successes: the
//! outer command decides what its transaction is.
//!
//! A locked glyph refuses every edit of its drawing (decision 0004 item 7): [`Session::edit`]
//! rejects a change to the layers or metrics keys of a glyph that was locked, whichever command
//! makes it, and [`check_unlocked`] lets a command refuse early with the same message.
//!
//! A drag or a dialog preview is one *interaction* ([`Session::begin`], [`Session::preview`],
//! [`Session::commit`], [`Session::cancel`]): previews run on the snapshot taken at the start
//! without history, and the commit makes one undo step and one journal entry.
#![forbid(unsafe_code)]

pub mod assets;
pub mod cmd;
pub(crate) mod export_steps;
pub mod extension_index;
pub mod extensions;
pub mod external_autohint;
pub mod feature_state;
pub mod file_access;
pub mod glyph_data;
pub mod interp_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod io;
pub(crate) mod linked;
pub mod log;
pub mod macros;
pub mod project;
pub mod slots;
pub mod spacing_state;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use fontcraft_color as color;
pub use fontcraft_compile as compile;
pub use fontcraft_components as components;
pub use fontcraft_container as container;
pub use fontcraft_engine_core::{Clock, EngineError, Result, check_unlocked, clock, guard, params, settings, work};
pub use fontcraft_fea as fea;
pub use fontcraft_features as features;
pub use fontcraft_fontinfo as fontinfo;
pub use fontcraft_format as format;
pub use fontcraft_hinting as hinting;
pub use fontcraft_imagery as imagery;
pub use fontcraft_interp as interp;
pub use fontcraft_model as model;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, PropsCache};
pub use fontcraft_pathops as pathops;
pub use fontcraft_plugins as plugins;
use serde_json::{Value, json};

/// The extension traits through which the command areas reach their state on a [`Session`] or a
/// [`DocState`] ([`slots`]): `use fontcraft_engine::prelude::*;`.
pub mod prelude {
    pub(crate) use crate::cmd::audit::DocAuditExt;
    pub use crate::feature_state::SessionFeaturesExt;
    pub use crate::interp_cache::SessionInterpExt;
    pub use crate::spacing_state::SessionSpacingExt;
}

/// Most undo steps kept per document. A step is a pointer to the font as it was (glyphs it shares
/// with its neighbours are not copied) plus a label.
pub const UNDO_LIMIT: usize = 10_000;
/// Most journal entries kept; older ones are dropped.
pub const JOURNAL_LIMIT: usize = 10_000;
/// How deeply commands may call other commands.
const MAX_DEPTH: u32 = 16;

/// `new` with the automatic layers, anchor expressions and composites that follow its changes
/// since `old` brought up to date (`fontcraft_components::expressions::follow_changes`, which
/// ends in `fontcraft_components::metrics::realign`), checked as every edit is; `None` when nothing
/// moves. `what` names the command or interaction in the error. The glyphs re-aligned get
/// `stamp` as their last change, as edited glyphs do.
fn realigned(what: &str, old: &Arc<Font>, new: &Arc<Font>, stamp: Option<&str>) -> Result<Option<Arc<Font>>> {
    if Arc::ptr_eq(old, new) {
        return Ok(None);
    }
    let mut next = fontcraft_components::expressions::follow_changes(old, Font::clone(new))
        .map_err(|e| EngineError::Disabled(what.to_string(), format!("the composites that follow this change cannot be aligned: {e}")))?;
    if next.same_as(new) {
        return Ok(None);
    }
    if let Some(stamp) = stamp
        && clock::writes_last_change(&next)
    {
        clock::stamp_changed(&mut next, new, stamp);
    }
    next.check_changed(new)?;
    Ok(Some(Arc::new(next)))
}

/// `new` with the hint node references of each layer whose nodes changed since `old` kept on
/// the same nodes (`fontcraft_hinting::remap`); `None` when none needs to change. Remove Overlap
/// moves TrueType instructions on removed nodes to the nearest node it created.
fn hint_upkeep(
    what: &str,
    old: &Arc<Font>,
    new: &Arc<Font>,
    skip: &fontcraft_hinting::Renumbered,
    report: &mut fontcraft_hinting::RemapReport,
) -> Result<Option<Arc<Font>>> {
    if Arc::ptr_eq(old, new) {
        return Ok(None);
    }
    let mode = match what {
        "path.removeOverlap" | "path.removeOverlapForAllMasters" => fontcraft_hinting::RemapMode::OverlapRemoval,
        _ => fontcraft_hinting::RemapMode::Edit,
    };
    let (next, found) = fontcraft_hinting::remap_after_edit(old, Font::clone(new), mode, skip)
        .map_err(|e| EngineError::Disabled(what.to_string(), format!("the hints on the changed nodes cannot be kept: {e}")))?;
    if found.is_empty() && next.glyphs.as_arcs().iter().zip(new.glyphs.as_arcs()).all(|(a, b)| Arc::ptr_eq(a, b)) {
        return Ok(None);
    }
    next.check_changed(new)?;
    report.removed.extend(found.removed);
    report.unlinked += found.unlinked;
    report.moved += found.moved;
    Ok(Some(Arc::new(next)))
}

/// A command's result with what the hint upkeep did added as `hintUpkeep` (`removed`: one
/// message per removed TrueType instruction; `unlinked`: PostScript ends turned into numbers;
/// `moved`: TrueType points moved to a node Remove Overlap created). A `null` result becomes an
/// object holding only the report. An array or scalar result is returned as it is and the report
/// is dropped: wrapping it in an object would change the result type its callers parse, and the
/// upkeep itself is already done and in the undo step.
fn with_hint_upkeep(v: Value, upkeep: fontcraft_hinting::RemapReport) -> Value {
    if upkeep.is_empty() {
        return v;
    }
    let report = serde_json::json!({"removed": upkeep.removed, "unlinked": upkeep.unlinked, "moved": upkeep.moved});
    match v {
        Value::Object(mut map) => {
            map.insert("hintUpkeep".to_string(), report);
            Value::Object(map)
        }
        Value::Null => serde_json::json!({ "hintUpkeep": report }),
        other => other,
    }
}

/// Records `before` as the step that undoes the document's last change, and drops the redo steps.
pub(crate) fn push_undo(doc: &mut DocState, label: Arc<str>, before: Arc<Font>) {
    doc.history.undo.push(HistoryEntry { label, font: before, path: doc.path.clone() });
    if doc.history.undo.len() > UNDO_LIMIT {
        let extra = doc.history.undo.len() - UNDO_LIMIT;
        doc.history.undo.drain(..extra);
    }
    doc.history.redo.clear();
}

/// The first glyph that was locked in `before` and whose drawing (layers or metrics keys)
/// differs in `next`. Unlocking, renaming, re-encoding or removing a locked glyph is allowed;
/// kerning is font data. Only glyphs `next` no longer shares with `before` are looked into.
fn locked_glyph_changed(next: &Font, before: &Font) -> Option<String> {
    let (new, old) = (next.glyphs.as_arcs(), before.glyphs.as_arcs());
    if std::ptr::eq(new, old) {
        return None;
    }
    for (i, g) in new.iter().enumerate() {
        let was = match old.get(i) {
            Some(b) if Arc::ptr_eq(b, g) => continue,
            Some(b) if b.name == g.name => b,
            _ => match before.glyph(&g.name) {
                Some(b) if Arc::ptr_eq(b, g) => continue,
                Some(b) => b,
                None => continue,
            },
        };
        if was.info.locked && (was.layers != g.layers || was.info.metrics_keys != g.info.metrics_keys) {
            return Some(was.name.clone());
        }
    }
    None
}

/// One undo step: the font as it was before the change.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub label: Arc<str>,
    pub font: Arc<Font>,
    /// Where the document was saved while `font` was current. Its relative image paths start
    /// from that file's folder; undo and redo rewrite them when the document was saved into
    /// another folder since.
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub undo: Vec<HistoryEntry>,
    pub redo: Vec<HistoryEntry>,
}

/// One open font.
#[derive(Clone, Debug)]
pub struct DocState {
    /// Stable for the session, unlike the document's position, which closing another document
    /// changes.
    pub id: u64,
    pub font: Arc<Font>,
    pub history: History,
    /// Where the document is saved (`None`: never saved).
    pub path: Option<PathBuf>,
    pub title: String,
    /// Set from [`next_revision`] by every change; agents and caches compare it to notice edits.
    /// A command that fails puts the document's revision back with its font, so numbers are
    /// never reused: a later change cannot get the number of a font that was rolled back.
    pub revision: u64,
    /// The font as it is on disk (for a new document: as it was created). Undoing back to it
    /// makes the document unmodified again.
    pub saved: Arc<Font>,
    /// What the file on disk looked like when it was last read or written, so a save notices a
    /// change made by something else in between.
    #[cfg(not(target_arch = "wasm32"))]
    pub stamp: Option<io::Stamp>,
    /// The command areas' state for this document ([`slots`]).
    pub(crate) exts: slots::DocExts,
    /// The glyph data files beside the source, merged with the user's.
    pub(crate) glyph_data: glyph_data::CustomData,
}

/// A revision number no document or project has had: the counter is shared by the whole
/// process and only grows, so rollback never makes two different fonts share one revision.
pub(crate) fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl DocState {
    /// Marks the document as changed: it gets a revision number not used before.
    pub(crate) fn bump_revision(&mut self) {
        self.revision = next_revision();
    }

    pub fn modified(&self) -> bool {
        !Arc::ptr_eq(&self.font, &self.saved)
    }
}

/// The open documents and everything commands act on.
#[derive(Default)]
pub struct Session {
    docs: Vec<DocState>,
    active: Option<usize>,
    journal: Vec<(String, Value)>,
    depth: u32,
    /// The layer sides the running command renumbered the hints of itself
    /// ([`Session::hints_renumbered`]); the hint upkeep leaves them alone.
    renumbered: fontcraft_hinting::Renumbered,
    untitled: u32,
    next_id: u64,
    /// While `command.batch` runs: the font of each document it changed, as it was before.
    pub(crate) batch: Option<Batch>,
    interaction: Option<Interaction>,
    settings: settings::UserSettings,
    /// The user's custom glyph data files.
    user_glyph_data: glyph_data::CustomData,
    /// The entries of the user's `Groups*.plist` files.
    user_groups: Option<Arc<fontcraft_glyphquery::Groups>>,
    /// Counts the changes to the user's glyph data, the glyph data beside a source and the
    /// user's groups, which change what the sidebar lists without a new document revision.
    glyph_data_revision: u64,
    /// What [`Session::edit`] stamps on changed glyphs; `None` stamps nothing.
    clock: Option<Clock>,
    /// Each open document's effective glyph properties, by document id.
    props: HashMap<u64, Arc<PropsCache>>,
    /// The language sets named through the glyph data last asked for.
    language_index: Option<Arc<fontcraft_glyphquery::LanguageIndex>>,
    /// The command areas' session state ([`slots`]).
    slots: slots::SessionSlots,
    /// The file cache and in-memory files behind [`Session::assets`].
    assets: assets::SessionAssets,
    /// Installed modules, macros, recording, the command log (`extensions`).
    extensions: extensions::ExtensionState,
    /// Exports may write where an Export Folder parameter climbs out of the destination folder.
    export_outside: bool,
    /// The user's feature code snippet folder ([`Session::set_snippets_dir`]).
    snippets_dir: Option<PathBuf>,
    /// The open projects (`project`).
    projects: project::Projects,
    /// External Preview while the app serves it (`cmd::sharing`).
    sharing: cmd::sharing::SharingState,
    /// Creating a session installs the command tables.
    _tables: TablesInstalled,
}

/// Installs the command tables ([`cmd::install`]) when a session is created, so every way of
/// making one finds the commands.
struct TablesInstalled;

impl Default for TablesInstalled {
    fn default() -> Self {
        cmd::ensure_installed();
        TablesInstalled
    }
}

/// A running interaction (see [`Session::begin`]).
struct Interaction {
    label: Arc<str>,
    /// The document it edits.
    doc: u64,
    /// The font when it began; every preview starts from it.
    before: Arc<Font>,
    /// The last preview that succeeded: what the commit journals.
    last: Option<(String, Value)>,
}

/// The state of a running `command.batch` (see [`Session::edit`]).
pub(crate) struct Batch {
    pub(crate) label: Arc<str>,
    pub(crate) before: Vec<(u64, Arc<Font>)>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// A session with one new, untitled font open.
    pub fn with_document() -> Self {
        let mut s = Self::new();
        s.push_document(Font::new("Untitled"), None);
        s
    }

    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn doc(&self) -> Result<&DocState> {
        self.active().ok_or(EngineError::NoDocument)
    }

    pub(crate) fn doc_mut(&mut self) -> Result<&mut DocState> {
        self.active.and_then(|i| self.docs.get_mut(i)).ok_or(EngineError::NoDocument)
    }

    pub fn font(&self) -> Result<&Font> {
        self.doc().map(|d| d.font.as_ref())
    }

    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }

    /// Top-level commands that ran and changed something, oldest first, with their parameters
    /// (for actions, replays and bug reports). Queries are not recorded.
    pub fn journal(&self) -> &[(String, Value)] {
        &self.journal
    }

    /// Opens `font` as a new, unmodified document and makes it the active one, if it passes the
    /// model's checks.
    pub fn add_document(&mut self, font: Font, path: Option<PathBuf>) -> Result<()> {
        font.check()?;
        self.push_document(font, path);
        Ok(())
    }

    /// [`Session::add_document`] for a font already known to be valid.
    pub(crate) fn push_document(&mut self, font: Font, path: Option<PathBuf>) {
        let title = match &path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            None => {
                self.untitled += 1;
                if self.untitled == 1 { "Untitled".to_string() } else { format!("Untitled {}", self.untitled) }
            }
        };
        let font = Arc::new(font);
        #[cfg(not(target_arch = "wasm32"))]
        let stamp = path.as_deref().and_then(|p| io::document_stamp(p, &font));
        self.next_id += 1;
        self.docs.push(DocState {
            id: self.next_id,
            saved: font.clone(),
            font,
            history: History::default(),
            path,
            title,
            revision: 0,
            #[cfg(not(target_arch = "wasm32"))]
            stamp,
            exts: slots::DocExts::default(),
            glyph_data: glyph_data::CustomData::default(),
        });
        self.active = Some(self.docs.len() - 1);
    }

    /// Closes the active document; the one that followed it (else the one before) becomes active.
    pub(crate) fn close_active(&mut self) -> Result<()> {
        let i = self.active.filter(|i| *i < self.docs.len()).ok_or(EngineError::NoDocument)?;
        self.docs.remove(i);
        self.active = if self.docs.is_empty() { None } else { Some(i.min(self.docs.len() - 1)) };
        Ok(())
    }

    /// Changes the active font as one undo step. `f` edits a private copy (glyphs it does not
    /// touch stay shared with the snapshot). The copy replaces the font only if `f` succeeds and
    /// the result passes the model's checks, so a failed edit leaves nothing behind. An edit that
    /// leaves the font as it was makes no undo step and does not count as a change. Inside
    /// `command.batch` the step is not pushed: the batch makes one step of all its edits.
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Font) -> Result<T>) -> Result<T> {
        let i = self.active.filter(|i| *i < self.docs.len()).ok_or(EngineError::NoDocument)?;
        let stamp = self.stamp();
        let doc = self.docs.get_mut(i).ok_or(EngineError::NoDocument)?;
        let mut next = Font::clone(&doc.font);
        let out = f(&mut next)?;
        cmd::color_layers_follow_master(&mut next, &doc.font);
        if next.same_as(&doc.font) {
            return Ok(out);
        }
        if let Some(name) = locked_glyph_changed(&next, &doc.font) {
            return Err(EngineError::Locked(name));
        }
        if let Some(stamp) = stamp
            && clock::writes_last_change(&next)
        {
            clock::stamp_changed(&mut next, &doc.font, &stamp);
        }
        next.check_changed(&doc.font)?;
        let before = std::mem::replace(&mut doc.font, Arc::new(next));
        doc.bump_revision();
        match &mut self.batch {
            Some(batch) => {
                if !batch.before.iter().any(|(id, _)| *id == doc.id) {
                    batch.before.push((doc.id, before));
                }
            }
            None => push_undo(doc, label.into(), before),
        }
        Ok(out)
    }

    /// No `command.batch` is running (for tests and the command sweep).
    pub fn batch_is_idle(&self) -> bool {
        self.batch.is_none()
    }

    /// Runs `f` as one undo step named `label`, as `command.batch` does: the documents `f`
    /// changes get one step each, and only when `f` succeeds. Inside a running batch `f` is part
    /// of that batch.
    pub fn one_step<T>(&mut self, label: &str, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
        let outer = self.batch.is_none();
        if outer {
            self.batch = Some(Batch { label: label.into(), before: Vec::new() });
        }
        let out = f(self);
        if outer {
            let finished = self.batch.take();
            if out.is_ok()
                && let Some(batch) = finished
            {
                for (id, before) in batch.before {
                    let Some(i) = self.position_of(id) else { continue };
                    if let Some(doc) = self.docs.get_mut(i)
                        && !doc.font.same_as(&before)
                    {
                        push_undo(doc, batch.label.clone(), before);
                    }
                }
            }
        }
        out
    }

    /// The last-change text for an edit made now; `None` without a clock.
    fn stamp(&self) -> Option<String> {
        self.clock.as_ref().and_then(|c| c()).and_then(clock::last_change_text)
    }

    /// Seconds since the Unix epoch from the session's clock; `None` without one.
    pub(crate) fn now(&self) -> Option<i64> {
        self.clock.as_ref().and_then(|c| c())
    }

    /// Sets the clock [`Session::edit`] reads to stamp Last Changed on the glyphs an edit
    /// changes (`fontView.column.lastChanged`). `None`, the default, stamps nothing.
    pub fn set_clock(&mut self, clock: Option<Clock>) {
        self.clock = clock;
    }

    /// Lets exports write files outside their destination folder when an Export Folder parameter
    /// climbs out of it with `..`. Off by default: headless, MCP and control-channel exports skip
    /// such a font with a message. The desktop app turns it on for what the user runs there.
    pub fn set_export_outside(&mut self, allow: bool) {
        self.export_outside = allow;
    }

    /// Whether exports may write outside their destination folder ([`Session::set_export_outside`]).
    pub fn export_outside(&self) -> bool {
        self.export_outside
    }

    /// The folder of the user's feature code snippets, which the snippet commands use when they
    /// are not given a `folder`. `None`, the default, means none: the snippet list has the
    /// built-in snippets only and Show Snippet Folder refuses. The desktop app sets it to the
    /// `snippets` folder in its settings folder.
    pub fn set_snippets_dir(&mut self, dir: Option<PathBuf>) {
        self.snippets_dir = dir;
    }

    /// The folder set by [`Session::set_snippets_dir`].
    pub fn snippets_dir(&self) -> Option<&Path> {
        self.snippets_dir.as_deref()
    }

    /// The user's settings (`settings.get`).
    pub fn settings(&self) -> &settings::UserSettings {
        &self.settings
    }

    /// Installed modules, macros, the recording and the command log.
    pub fn extensions(&self) -> &extensions::ExtensionState {
        &self.extensions
    }

    /// The extension state, for the commands and the app (folder sync, safe mode). It is not
    /// part of the rollback snapshot: a command that changes it does so only after it succeeds.
    pub fn extensions_mut(&mut self) -> &mut extensions::ExtensionState {
        &mut self.extensions
    }

    /// External Preview's address and the devices waiting or refused (`cmd::sharing`).
    pub fn sharing(&self) -> &cmd::sharing::SharingState {
        &self.sharing
    }

    /// For the app that serves External Preview, and for the sharing commands.
    pub fn sharing_mut(&mut self) -> &mut cmd::sharing::SharingState {
        &mut self.sharing
    }

    /// The user's settings, for the app (loading) and for the commands that own reserved ids.
    /// A command that fails after changing them is rolled back with the rest of the session.
    pub fn settings_mut(&mut self) -> &mut settings::UserSettings {
        &mut self.settings
    }

    /// The glyph data the active document's commands read: the built-in database with the
    /// user's custom files and the files beside the source on top. Without a document, the
    /// built-in database with the user's files.
    pub fn glyph_data(&self) -> Result<Arc<GlyphData>> {
        match self.active() {
            Some(doc) => self.glyph_data_of(doc),
            None => self.user_glyph_data.merged.clone().map_or_else(glyph_data::builtin, Ok),
        }
    }

    /// The active document's effective glyph properties, up to date with its font and glyph
    /// data. Only the glyphs that changed since the last call are computed again.
    pub fn effective_props(&mut self) -> Result<Arc<PropsCache>> {
        let doc = self.doc()?;
        let (id, font) = (doc.id, doc.font.clone());
        let data = self.glyph_data()?;
        let open: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.props.retain(|id, _| open.contains(id));
        let cache = self.props.entry(id).or_default();
        Arc::make_mut(cache).update(&font, &data);
        Ok(cache.clone())
    }

    /// Sets the active document's display strings (the texts of its Edit View tabs). This is
    /// view state, not an edit: it adds no undo step, and an unmodified document stays
    /// unmodified (`text.displayStrings`).
    pub fn set_display_strings(&mut self, strings: Vec<String>) -> Result<()> {
        let doc = self.doc_mut()?;
        if *doc.font.display_strings == strings {
            return Ok(());
        }
        let unchanged = !doc.modified();
        let mut font = Font::clone(&doc.font);
        font.display_strings = Arc::new(strings);
        let font = Arc::new(font);
        doc.font = font.clone();
        if unchanged {
            doc.saved = font;
        }
        Ok(())
    }

    /// [`Session::glyph_data`] for one document.
    pub fn glyph_data_of(&self, doc: &DocState) -> Result<Arc<GlyphData>> {
        if !glyph_data::ignores_source_files(&doc.font)
            && let Some(d) = &doc.glyph_data.merged
        {
            return Ok(d.clone());
        }
        self.user_glyph_data.merged.clone().map_or_else(glyph_data::builtin, Ok)
    }

    /// Sets the user's custom glyph data files (name, bytes), in order: the app reads them from
    /// its settings folder. Malformed files are skipped; the problems are returned.
    pub fn set_user_glyph_data(&mut self, files: &[(&str, &[u8])]) -> Vec<String> {
        let mut problems = Vec::new();
        let parsed = glyph_data::parse_files(files, &mut problems);
        let merged = glyph_data::merge(&[&parsed], &mut problems);
        self.user_glyph_data = glyph_data::CustomData { files: Arc::new(parsed), merged };
        self.glyph_data_revision += 1;
        for i in 0..self.docs.len() {
            self.remerge_doc(i, &mut problems);
        }
        problems
    }

    /// Sets the glyph data files that sit beside the active document's source (name, bytes).
    /// `file.open` reads them from disk; tests and the browser build hand them in. Malformed
    /// files are skipped; the problems are returned.
    pub fn set_source_glyph_data(&mut self, files: &[(&str, &[u8])]) -> Result<Vec<String>> {
        let i = self.active.filter(|i| *i < self.docs.len()).ok_or(EngineError::NoDocument)?;
        let mut problems = Vec::new();
        let parsed = glyph_data::parse_files(files, &mut problems);
        if let Some(doc) = self.docs.get_mut(i) {
            doc.glyph_data.files = Arc::new(parsed);
        }
        self.glyph_data_revision += 1;
        self.remerge_doc(i, &mut problems);
        Ok(problems)
    }

    /// Rebuilds a document's merged glyph data from the user's files and its own.
    fn remerge_doc(&mut self, i: usize, problems: &mut Vec<String>) {
        let user = self.user_glyph_data.files.clone();
        let Some(doc) = self.docs.get_mut(i) else { return };
        let own = doc.glyph_data.files.clone();
        doc.glyph_data.merged = if own.is_empty() { None } else { glyph_data::merge(&[&user, &own], problems) };
    }

    /// The position of the document with this id.
    pub(crate) fn position_of(&self, id: u64) -> Option<usize> {
        self.docs.iter().position(|d| d.id == id)
    }

    pub(crate) fn docs_mut(&mut self) -> &mut [DocState] {
        &mut self.docs
    }

    pub(crate) fn set_active(&mut self, i: usize) -> Result<()> {
        if i >= self.docs.len() {
            return Err(EngineError::NoDocument);
        }
        self.active = Some(i);
        Ok(())
    }

    /// Runs the command `id`. See the crate docs for what every command guarantees. While an
    /// interaction runs, only queries run; editing commands wait for its commit or cancel.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        if self.depth == 0 && self.interaction.is_some() && cmd::registry::find(id).is_some_and(|c| c.journal) {
            return Err(EngineError::Disabled(id.to_string(), "an interaction is running: commit or cancel it first".into()));
        }
        self.run_command(id, params, true, |_| Ok(()))
    }

    /// Runs the query `id` without writing it to the command log, for views that read engine
    /// state on every frame. A command that is not a query is refused.
    pub fn query(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = cmd::registry::lookup(id)?;
        if spec.journal {
            return Err(EngineError::Disabled(id.to_string(), "it changes the document: run it as a command".into()));
        }
        self.run_command(id, params, false, |_| Ok(()))
    }

    /// [`Session::execute`] without the interaction rule. `record`: journal the command when it
    /// succeeds. `accept` judges the outcome before it is kept; its error rolls the command back.
    fn run_command(&mut self, id: &str, params: &Value, record: bool, accept: impl Fn(&Session) -> Result<()>) -> Result<Value> {
        let spec = cmd::registry::lookup(id)?;
        if let Err(why) = (spec.enabled)(self) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        if self.depth >= MAX_DEPTH {
            return Err(EngineError::Internal(format!("commands nested more than {MAX_DEPTH} deep at `{id}`")));
        }
        // Steps a command runs itself are its own business; what a request names is checked.
        let checked = match (self.depth, file_access::current()) {
            (0, Some(roots)) => Some(file_access::command::check(self, spec, params, &roots)?),
            _ => None,
        };
        let params = checked.as_ref().unwrap_or(params);
        work::check()?;
        let _level = work::enter();
        if self.depth > 0 {
            // A step of another command: the outer call owns the snapshot and the journal. The
            // modules are read again, so a step sees what an earlier step installed or removed.
            let _modules = extensions::ModuleScope::enter(&self.extensions);
            self.depth += 1;
            let r = (spec.run)(self, params);
            self.depth -= 1;
            return r;
        }
        // Snapshot for rollback. It copies pointers and labels, no glyph data, but it does walk
        // each document's history, so queries (which change nothing) skip it.
        let snapshot = spec.journal.then(|| (self.docs.clone(), self.active, self.untitled, self.settings.clone(), self.projects.clone()));
        let _modules = extensions::ModuleScope::enter(&self.extensions);
        self.depth = 1;
        self.renumbered = fontcraft_hinting::Renumbered::default();
        let outcome = guard::catch_panic(|| (spec.run)(self, params));
        self.depth = 0;
        let result = match outcome {
            Ok(r) => r
                .and_then(|v| self.realign_after(id, snapshot.as_ref().map(|s| s.0.as_slice())).map(|upkeep| with_hint_upkeep(v, upkeep)))
                .and_then(|v| accept(self).map(|()| v)),
            Err(panic) => Err(EngineError::Internal(format!("`{id}` failed unexpectedly: {panic}"))),
        };
        // What an edit created that a UFO document's file cannot hold (previews and queries skip it).
        let result = match (result, &snapshot) {
            (Ok(v), Some((before, ..))) if record => Ok(cmd::format_warnings::attach(&self.docs, before, id, v)),
            (r, _) => r,
        };
        // A cancel the command did not stop at undoes it here; from here on it is too late. Files already written
        // stand, so a command that writes some is kept.
        let result = result.and_then(|v| {
            if spec.files.iter().any(|f| f.access == cmd::FileAccess::Write) {
                work::keep();
            } else if spec.journal {
                work::seal()?;
            }
            Ok(v)
        });
        if record {
            self.extensions.after_command(id, params, spec.journal, &result);
        }
        match result {
            Ok(value) => {
                if spec.journal && record {
                    self.journal.push((id.to_string(), params.clone()));
                    if self.journal.len() > JOURNAL_LIMIT {
                        let extra = self.journal.len() - JOURNAL_LIMIT;
                        self.journal.drain(..extra);
                    }
                }
                Ok(value)
            }
            Err(e) => {
                if let Some((docs, active, untitled, settings, projects)) = snapshot {
                    (self.docs, self.active, self.untitled, self.settings, self.projects) = (docs, active, untitled, settings, projects);
                }
                Err(e)
            }
        }
    }

    /// Re-aligns the composites that follow what command `id` changed in each open document,
    /// in place, as part of the command's own undo step. `before` is the documents as they were
    /// (`None` for a query). Undo and redo put back fonts as they were and are left alone; a
    /// document the command opened has no `before` and is not aligned. Previews run through
    /// [`Session::run_command`], so each preview is aligned too.
    ///
    /// Then the hint node references of every layer whose nodes the command changed are kept on
    /// the same nodes ([`hint_upkeep`]); what that removed or unlinked is returned.
    fn realign_after(&mut self, id: &str, before: Option<&[DocState]>) -> Result<fontcraft_hinting::RemapReport> {
        let skip = std::mem::take(&mut self.renumbered);
        let mut upkeep = skip.report.clone();
        let Some(before) = before else { return Ok(upkeep) };
        if matches!(id, "edit.undo" | "edit.redo") {
            return Ok(upkeep);
        }
        let stamp = self.stamp();
        for doc in &mut self.docs {
            let Some(old) = before.iter().find(|d| d.id == doc.id) else { continue };
            if let Some(font) = realigned(id, &old.font, &doc.font, stamp.as_deref())? {
                doc.font = font;
            }
            if let Some(font) = hint_upkeep(id, &old.font, &doc.font, &skip, &mut upkeep)? {
                doc.font = font;
            }
        }
        Ok(upkeep)
    }

    /// Tells the session that the running command renumbered the hints of the layer sides in
    /// `done` itself, for the edit it made: the hint upkeep after the command leaves them alone,
    /// and `done.report` is returned with its own. Steps of a command add to the same list.
    pub(crate) fn hints_renumbered(&mut self, done: fontcraft_hinting::Renumbered) {
        self.renumbered.extend(done);
    }

    /// Starts an interaction on the active document: a drag, or a dialog with a live preview.
    /// The font as it is now is the snapshot every [`Session::preview`] starts from. One
    /// interaction runs at a time, outside commands and batches.
    pub fn begin(&mut self, label: &str) -> Result<()> {
        if self.interaction.is_some() {
            return Err(EngineError::Interaction("an interaction is already running: commit or cancel it first".into()));
        }
        if self.depth > 0 || self.batch.is_some() {
            return Err(EngineError::Interaction("an interaction cannot begin inside a command".into()));
        }
        let doc = self.doc()?;
        self.interaction = Some(Interaction { label: label.into(), doc: doc.id, before: doc.font.clone(), last: None });
        Ok(())
    }

    /// The label of the running interaction, if one runs.
    pub fn interaction(&self) -> Option<&str> {
        self.interaction.as_ref().map(|i| i.label.as_ref())
    }

    /// The font the running interaction began from, which cancelling it gives back.
    pub fn interaction_start(&self) -> Option<&Font> {
        self.interaction.as_ref().map(|i| i.before.as_ref())
    }

    /// Runs an editing command on the interaction's snapshot, replacing the previous preview,
    /// without history or journal. A preview that fails changes nothing: the previous one stays
    /// shown. Undo, redo and commands that open, close or switch documents cannot be previewed.
    pub fn preview(&mut self, id: &str, params: &Value) -> Result<Value> {
        let (doc_id, before) = match &self.interaction {
            Some(i) => (i.doc, i.before.clone()),
            None => return Err(EngineError::Interaction("no interaction is running: begin one first".into())),
        };
        let spec = cmd::registry::lookup(id)?;
        if !spec.journal || matches!(id, "edit.undo" | "edit.redo") {
            return Err(EngineError::Interaction(format!("`{id}` cannot be previewed: only editing commands can")));
        }
        let i = self.interaction_doc(doc_id)?;
        let doc = self.docs.get_mut(i).ok_or(EngineError::NoDocument)?;
        let (shown, shown_revision) = (std::mem::replace(&mut doc.font, before), doc.revision);
        let count = self.docs.len();
        // The batch state keeps `edit` from recording undo steps; it is discarded afterwards.
        self.batch = Some(Batch { label: "".into(), before: Vec::new() });
        let result = self.run_command(id, params, false, |s| {
            if s.docs.len() == count && s.active == Some(i) {
                Ok(())
            } else {
                Err(EngineError::Interaction(format!("`{id}` cannot be previewed: it opens, closes or switches documents")))
            }
        });
        self.batch = None;
        let doc = self.docs.get_mut(i).ok_or(EngineError::NoDocument)?;
        match result {
            Ok(value) => {
                doc.revision = if Arc::ptr_eq(&doc.font, &shown) { shown_revision } else { next_revision() };
                if let Some(it) = &mut self.interaction {
                    it.last = Some((id.to_string(), params.clone()));
                }
                Ok(value)
            }
            Err(e) => {
                (doc.font, doc.revision) = (shown, shown_revision);
                Err(e)
            }
        }
    }

    /// Ends the interaction, keeping the last preview as one undo step labelled with the
    /// interaction's label, and journals the last previewed command. Returns whether the font
    /// changed (an interaction that changed nothing leaves no undo step and no journal entry).
    pub fn commit(&mut self) -> Result<bool> {
        let it = self.interaction.take().ok_or_else(|| EngineError::Interaction("no interaction is running".into()))?;
        let Some(i) = self.position_of(it.doc) else { return Ok(false) };
        let stamp = self.stamp();
        let doc = self.docs.get_mut(i).ok_or(EngineError::NoDocument)?;
        if Arc::ptr_eq(&doc.font, &it.before) || doc.font.same_as(&it.before) {
            doc.font = it.before;
            return Ok(false);
        }
        // Each preview was aligned as it ran (`run_command`); this catches a font changed by
        // other means meanwhile. A failure ends the interaction as a cancel would.
        match realigned(&it.label, &it.before, &doc.font, stamp.as_deref()) {
            Ok(Some(font)) => {
                doc.font = font;
                doc.bump_revision();
            }
            Ok(None) => {}
            Err(e) => {
                doc.font = it.before;
                doc.bump_revision();
                return Err(e);
            }
        }
        push_undo(doc, it.label, it.before);
        if let Some(entry) = it.last {
            // The interaction counts as one top-level command, its last preview's: the command
            // log and a running macro recording see it once.
            self.extensions.after_command(&entry.0, &entry.1, true, &Ok(Value::Null));
            self.journal.push(entry);
            if self.journal.len() > JOURNAL_LIMIT {
                let extra = self.journal.len() - JOURNAL_LIMIT;
                self.journal.drain(..extra);
            }
        }
        Ok(true)
    }

    /// Ends the interaction and puts the snapshot back: the document's font is the very font it
    /// was when the interaction began.
    pub fn cancel(&mut self) -> Result<()> {
        let it = self.interaction.take().ok_or_else(|| EngineError::Interaction("no interaction is running".into()))?;
        if let Some(doc) = self.position_of(it.doc).and_then(|i| self.docs.get_mut(i))
            && !Arc::ptr_eq(&doc.font, &it.before)
        {
            doc.font = it.before;
            doc.bump_revision();
        }
        Ok(())
    }

    /// The position of the interaction's document; it must still be the active one.
    fn interaction_doc(&mut self, id: u64) -> Result<usize> {
        match self.position_of(id) {
            Some(i) if self.active == Some(i) => Ok(i),
            Some(_) => Err(EngineError::Interaction("the interaction's document is no longer the active one".into())),
            None => {
                self.interaction = None;
                Err(EngineError::Interaction("the interaction's document was closed".into()))
            }
        }
    }

    /// A summary of the session for agents: `document.inspect` on the control channel and MCP.
    pub fn inspect(&self) -> Value {
        let docs: Vec<Value> = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                json!({
                    "index": i,
                    "active": self.active == Some(i),
                    "title": d.title,
                    "path": d.path.as_ref().map(|p| p.to_string_lossy()),
                    "modified": d.modified(),
                    "revision": d.revision,
                    "familyName": d.font.family_name,
                    "glyphs": d.font.glyphs.len(),
                    "masters": d.font.masters.iter().map(|m| json!({"id": m.id, "name": m.name})).collect::<Vec<_>>(),
                    "undo": d.history.undo.iter().map(|h| h.label.as_ref()).collect::<Vec<_>>(),
                    "redo": d.history.redo.iter().map(|h| h.label.as_ref()).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "documents": docs, "active": self.active })
    }
}
