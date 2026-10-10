//! FontCraft engine (L5): a [`Session`] of open fonts, their undo history, and the command
//! registry. **Every user-visible action is a command** ([`cmd::CommandSpec`]): the UI, the CLI,
//! the control channel and the MCP server all call [`Session::execute`] with a command id and JSON
//! parameters, and get JSON back.
//!
//! Three properties hold for every command, and `tests/command_sweep.rs` checks them:
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

pub mod cmd;
pub mod guard;
#[cfg(not(target_arch = "wasm32"))]
pub mod io;
pub mod params;

use std::path::PathBuf;
use std::sync::Arc;

pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use fontcraft_compile as compile;
pub use fontcraft_container as container;
pub use fontcraft_format as format;
pub use fontcraft_model as model;
use fontcraft_model::{Font, Glyph};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("`{0}` is not available: {1}")]
    Disabled(String, String),
    #[error("{command}: {message}")]
    BadParams { command: String, message: String },
    #[error("glyph `{0}` is locked: unlock it to change it")]
    Locked(String),
    #[error("{0}")]
    Interaction(String),
    #[error("no document open")]
    NoDocument,
    #[error(transparent)]
    Model(#[from] fontcraft_model::ModelError),
    #[error(transparent)]
    Format(#[from] fontcraft_format::FormatError),
    #[error(transparent)]
    Compile(#[from] fontcraft_compile::CompileError),
    #[error(transparent)]
    Glyphs(#[from] fontcraft_io_glyphs::GlyphsError),
    #[error(transparent)]
    Ufo(#[from] fontcraft_io_ufo::UfoError),
    #[error(transparent)]
    Decompile(#[from] fontcraft_decompile::DecompileError),
    #[error("{0}")]
    Io(String),
    #[error("not supported yet: {0}")]
    Unsupported(String),
    #[error("internal error (please report it): {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Most undo steps kept per document. A step is a pointer to the font as it was (glyphs it shares
/// with its neighbours are not copied) plus a label.
pub const UNDO_LIMIT: usize = 10_000;
/// Most journal entries kept; older ones are dropped.
pub const JOURNAL_LIMIT: usize = 10_000;
/// How deeply commands may call other commands.
const MAX_DEPTH: u32 = 16;

/// Records `before` as the step that undoes the document's last change, and drops the redo steps.
pub(crate) fn push_undo(doc: &mut DocState, label: Arc<str>, before: Arc<Font>) {
    doc.history.undo.push(HistoryEntry { label, font: before });
    if doc.history.undo.len() > UNDO_LIMIT {
        let extra = doc.history.undo.len() - UNDO_LIMIT;
        doc.history.undo.drain(..extra);
    }
    doc.history.redo.clear();
}

/// Refuses a locked glyph (decision 0004 item 7). Every editing command goes through it, by way
/// of [`params::Targets::edit_layer`] or directly; [`Session::edit`] enforces the same rule on
/// whatever a command changes.
pub fn check_unlocked(glyph: &Glyph) -> Result<()> {
    if glyph.info.locked { Err(EngineError::Locked(glyph.name.clone())) } else { Ok(()) }
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
    /// Bumped by every change; agents compare it to notice edits.
    pub revision: u64,
    /// The font as it is on disk (for a new document: as it was created). Undoing back to it
    /// makes the document unmodified again.
    pub saved: Arc<Font>,
    /// What the file on disk looked like when it was last read or written, so a save notices a
    /// change made by something else in between.
    #[cfg(not(target_arch = "wasm32"))]
    pub stamp: Option<io::Stamp>,
    /// The last `audit.run` on this document, which `audit.results` and `audit.fix` read.
    pub(crate) audit: Option<Arc<cmd::audit::KeptRun>>,
}

impl DocState {
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
    untitled: u32,
    next_id: u64,
    /// While `command.batch` runs: the font of each document it changed, as it was before.
    pub(crate) batch: Option<Batch>,
    interaction: Option<Interaction>,
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
            audit: None,
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
        let doc = self.docs.get_mut(i).ok_or(EngineError::NoDocument)?;
        let mut next = Font::clone(&doc.font);
        let out = f(&mut next)?;
        if next.same_as(&doc.font) {
            return Ok(out);
        }
        if let Some(name) = locked_glyph_changed(&next, &doc.font) {
            return Err(EngineError::Locked(name));
        }
        next.check_changed(&doc.font)?;
        let before = std::mem::replace(&mut doc.font, Arc::new(next));
        doc.revision += 1;
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
        if self.depth == 0 && self.interaction.is_some() && find_command(id).is_some_and(|c| c.journal) {
            return Err(EngineError::Disabled(id.to_string(), "an interaction is running: commit or cancel it first".into()));
        }
        self.run_command(id, params, true, |_| Ok(()))
    }

    /// [`Session::execute`] without the interaction rule. `record`: journal the command when it
    /// succeeds. `accept` judges the outcome before it is kept; its error rolls the command back.
    fn run_command(&mut self, id: &str, params: &Value, record: bool, accept: impl Fn(&Session) -> Result<()>) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        if let Err(why) = (spec.enabled)(self) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        if self.depth >= MAX_DEPTH {
            return Err(EngineError::Internal(format!("commands nested more than {MAX_DEPTH} deep at `{id}`")));
        }
        if self.depth > 0 {
            // A step of another command: the outer call owns the snapshot and the journal.
            self.depth += 1;
            let r = (spec.run)(self, params);
            self.depth -= 1;
            return r;
        }
        // Snapshot for rollback. It copies pointers and labels, no glyph data, but it does walk
        // each document's history, so queries (which change nothing) skip it.
        let snapshot = spec.journal.then(|| (self.docs.clone(), self.active, self.untitled));
        self.depth = 1;
        let outcome = guard::catch_panic(|| (spec.run)(self, params));
        self.depth = 0;
        let result = match outcome {
            Ok(r) => r.and_then(|v| accept(self).map(|()| v)),
            Err(panic) => Err(EngineError::Internal(format!("`{id}` failed unexpectedly: {panic}"))),
        };
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
                if let Some((docs, active, untitled)) = snapshot {
                    (self.docs, self.active, self.untitled) = (docs, active, untitled);
                }
                Err(e)
            }
        }
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

    /// Runs an editing command on the interaction's snapshot, replacing the previous preview,
    /// without history or journal. A preview that fails changes nothing: the previous one stays
    /// shown. Undo, redo and commands that open, close or switch documents cannot be previewed.
    pub fn preview(&mut self, id: &str, params: &Value) -> Result<Value> {
        let (doc_id, before) = match &self.interaction {
            Some(i) => (i.doc, i.before.clone()),
            None => return Err(EngineError::Interaction("no interaction is running: begin one first".into())),
        };
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
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
                doc.revision = if Arc::ptr_eq(&doc.font, &shown) { shown_revision } else { shown_revision + 1 };
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
        let doc = self.docs.get_mut(i).ok_or(EngineError::NoDocument)?;
        if Arc::ptr_eq(&doc.font, &it.before) || doc.font.same_as(&it.before) {
            doc.font = it.before;
            return Ok(false);
        }
        push_undo(doc, it.label, it.before);
        if let Some(entry) = it.last {
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
            doc.revision += 1;
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
