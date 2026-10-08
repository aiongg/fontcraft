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
pub use fontcraft_format as format;
pub use fontcraft_model as model;
use fontcraft_model::Font;
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("`{0}` is not available: {1}")]
    Disabled(String, String),
    #[error("{command}: {message}")]
    BadParams { command: String, message: String },
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

    /// Runs the command `id`. See the crate docs for what every command guarantees.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
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
            Ok(r) => r,
            Err(panic) => Err(EngineError::Internal(format!("`{id}` failed unexpectedly: {panic}"))),
        };
        match result {
            Ok(value) => {
                if spec.journal {
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
