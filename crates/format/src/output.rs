//! What this process may put on disk (rows `cli.global.dryRun`, `.overwrite`, `.stdinStdout`).
//! [`write_atomic`](crate::write_atomic), the staging of compiled fonts and the file helpers here
//! all ask this module, so one switch covers every writing path:
//!
//! - In a dry run ([`set_dry_run`]) nothing is created, changed or removed. What would be is
//!   recorded and [`planned`] lists it. A write that could not succeed (its folder is missing, a
//!   folder has its name, the file is read-only) still fails.
//! - With [`set_no_clobber`], a file or folder that exists and that this process did not write is
//!   not replaced.
//! - A folder or file that is *held* ([`hold`]) never reaches the disk: what is written there stays in
//!   memory, and [`held`] gives it back. It carries working files through a dry run, and the
//!   command line's standard input and output through commands that read and write paths.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::FormatError;

/// Most entries [`planned`] lists; more are only counted.
pub const MAX_PLANNED: usize = 100_000;
/// Most bytes held in memory at once.
pub const MAX_HELD: usize = 1 << 30;

/// What a dry run left undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Write,
    CreateFolder,
    Remove,
}

/// One change a dry run left undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub action: Action,
    pub path: PathBuf,
    /// The size of a file that would be written, where it is known.
    pub bytes: Option<usize>,
}

struct State {
    /// `None`: taken back.
    planned: Vec<Option<Planned>>,
    more: usize,
    /// Where each write is in `planned`.
    writes: BTreeMap<PathBuf, usize>,
    /// Where each folder that would be created is in `planned` (`None` past [`MAX_PLANNED`]).
    folders: BTreeMap<PathBuf, Option<usize>>,
    roots: Vec<PathBuf>,
    held: BTreeMap<PathBuf, Vec<u8>>,
    held_bytes: usize,
    /// What this process wrote, while no-clobber is on.
    own: BTreeSet<PathBuf>,
}

static DRY_RUN: AtomicBool = AtomicBool::new(false);
static NO_CLOBBER: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<State> = Mutex::new(State {
    planned: Vec::new(),
    more: 0,
    writes: BTreeMap::new(),
    folders: BTreeMap::new(),
    roots: Vec::new(),
    held: BTreeMap::new(),
    held_bytes: 0,
    own: BTreeSet::new(),
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One spelling per place, so that a path given relative and absolute meet.
fn key(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl State {
    fn push(&mut self, entry: Planned) -> Option<usize> {
        if self.planned.len() >= MAX_PLANNED {
            self.more = self.more.saturating_add(1);
            return None;
        }
        self.planned.push(Some(entry));
        Some(self.planned.len().saturating_sub(1))
    }

    fn forget(&mut self, at: Option<usize>) {
        if let Some(slot) = at.and_then(|i| self.planned.get_mut(i)) {
            *slot = None;
        }
    }

    fn is_held(&self, key: &Path) -> bool {
        self.roots.iter().any(|r| key.starts_with(r))
    }

    fn drop_held(&mut self, under: &Path) {
        let gone: Vec<PathBuf> = self.held.keys().filter(|k| k.starts_with(under)).cloned().collect();
        for k in gone {
            let bytes = self.held.remove(&k).map_or(0, |b| b.len());
            self.held_bytes = self.held_bytes.saturating_sub(bytes);
        }
    }

    fn is_dir(&self, key: &Path) -> bool {
        self.folders.contains_key(key) || self.is_held(key) || key.is_dir()
    }
}

/// Turns the dry run on or off for the whole process.
pub fn set_dry_run(on: bool) {
    DRY_RUN.store(on, Ordering::Relaxed);
}

/// Whether the process is in a dry run.
pub fn dry_run() -> bool {
    DRY_RUN.load(Ordering::Relaxed)
}

/// Refuses, for the whole process, to replace a file or folder that another process wrote.
pub fn set_no_clobber(on: bool) {
    NO_CLOBBER.store(on, Ordering::Relaxed);
}

/// Whether [`set_no_clobber`] is on.
pub fn no_clobber() -> bool {
    NO_CLOBBER.load(Ordering::Relaxed)
}

/// What the dry run left undone, in order, and how many entries past [`MAX_PLANNED`] there were.
pub fn planned() -> (Vec<Planned>, usize) {
    let s = state();
    (s.planned.iter().flatten().cloned().collect(), s.more)
}

/// Whether what is written to `path` stays off the disk: a dry run, or a held place. The code that
/// stages files asks this before it creates a temporary one, and calls [`plan_write`] instead.
pub fn diverted(path: &Path) -> bool {
    dry_run() || (has_roots() && state().is_held(&key(path)))
}

static HAS_ROOTS: AtomicBool = AtomicBool::new(false);

fn has_roots() -> bool {
    HAS_ROOTS.load(Ordering::Relaxed)
}

/// Keeps what is written to `place` (a file, or anything under a folder) in memory until
/// [`release`].
pub fn hold(place: &Path) {
    HAS_ROOTS.store(true, Ordering::Relaxed);
    let key = key(place);
    let mut s = state();
    if !s.roots.contains(&key) {
        s.roots.push(key);
    }
}

/// Puts `bytes` at the held `place`, as a write there would.
pub fn put_held(place: &Path, bytes: Vec<u8>) -> Result<(), FormatError> {
    hold(place);
    store(&mut state(), &key(place), bytes)
}

/// Ends [`hold`] and drops what it kept.
pub fn release(place: &Path) {
    let key = key(place);
    let mut s = state();
    s.roots.retain(|r| *r != key);
    s.drop_held(&key);
}

/// The bytes written to the held `path`.
pub fn held(path: &Path) -> Option<Vec<u8>> {
    if !has_roots() {
        return None;
    }
    state().held.get(&key(path)).cloned()
}

fn store(s: &mut State, key: &Path, bytes: Vec<u8>) -> Result<(), FormatError> {
    let before = s.held.get(key).map_or(0, Vec::len);
    let total = s.held_bytes.saturating_sub(before).saturating_add(bytes.len());
    if total > MAX_HELD {
        return Err(FormatError::TooLarge(total));
    }
    s.held_bytes = total;
    s.held.insert(key.to_path_buf(), bytes);
    Ok(())
}

/// Records that `bytes` would be written to `path` (or keeps them, for a held path). Fails where
/// the write would: the folder is missing, a folder has that name, the file is read-only.
pub fn plan_write(path: &Path, bytes: &[u8]) -> Result<(), FormatError> {
    let key = key(path);
    let mut s = state();
    if s.is_held(&key) {
        return store(&mut s, &key, bytes.to_vec());
    }
    let cannot = |why: &str| FormatError::Io(format!("cannot write {}: {why}", path.display()));
    if path.is_dir() {
        return Err(cannot("it is a folder"));
    }
    if std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
        return Err(cannot("it is read-only"));
    }
    record_write(&mut s, path, key, Some(bytes.len()))
}

/// Records that `path`, a folder (a UFO) with `bytes` of files, or a file whose size is not
/// known, would be written as a whole.
pub fn plan_whole(path: &Path, bytes: Option<usize>) -> Result<(), FormatError> {
    record_write(&mut state(), path, key(path), bytes)
}

fn record_write(s: &mut State, path: &Path, key: PathBuf, bytes: Option<usize>) -> Result<(), FormatError> {
    if let Some(folder) = key.parent()
        && !s.is_dir(folder)
    {
        return Err(FormatError::Io(format!("cannot write {}: the folder {} does not exist", path.display(), folder.display())));
    }
    let entry = Planned { action: Action::Write, path: path.to_path_buf(), bytes };
    let known = s.writes.get(&key).copied();
    if let Some(slot) = known.and_then(|i| s.planned.get_mut(i)) {
        *slot = Some(entry);
        return Ok(());
    }
    if let Some(i) = s.push(entry) {
        s.writes.insert(key, i);
    }
    Ok(())
}

/// Takes back what [`plan_write`] or [`plan_whole`] recorded for `path`: the run that would have
/// written it failed.
pub fn unplan(path: &Path) {
    let key = key(path);
    let mut s = state();
    let at = s.writes.remove(&key);
    s.forget(at);
    if let Some(bytes) = s.held.remove(&key) {
        s.held_bytes = s.held_bytes.saturating_sub(bytes.len());
    }
}

/// Notes that this process wrote `path`, so that no-clobber lets it write there again.
pub(crate) fn note_written(path: &Path) {
    if NO_CLOBBER.load(Ordering::Relaxed) {
        state().own.insert(key(path));
    }
}

/// With [`set_no_clobber`], refuses a `path` that exists and that this process did not write.
pub fn check_clobber(path: &Path) -> Result<(), FormatError> {
    if !NO_CLOBBER.load(Ordering::Relaxed) || std::fs::symlink_metadata(path).is_err() || state().own.contains(&key(path)) {
        return Ok(());
    }
    Err(FormatError::Io(format!("{} exists and noClobber is set", path.display())))
}

/// Whether `path` is a folder, or would be one after the dry run.
pub fn is_dir(path: &Path) -> bool {
    path.is_dir() || ((dry_run() || has_roots()) && state().is_dir(&key(path)))
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{} does not exist", path.display()))
}

/// [`std::fs::create_dir`], which a dry run records; a folder already there or recorded is
/// accepted in a dry run.
pub fn create_dir(path: &Path) -> io::Result<()> {
    if !diverted(path) {
        return std::fs::create_dir(path);
    }
    if path.parent().is_some_and(|p| !p.as_os_str().is_empty() && !is_dir(p)) {
        return Err(not_found(path));
    }
    create_dir_all(path)
}

/// [`std::fs::create_dir_all`], which a dry run records.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    if !diverted(path) {
        return std::fs::create_dir_all(path);
    }
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    let key = key(path);
    let mut s = state();
    if s.is_dir(&key) {
        return Ok(());
    }
    let missing: Vec<PathBuf> = key.ancestors().take_while(|a| !s.is_dir(a)).map(Path::to_path_buf).collect();
    if let Some(file) = missing.iter().find(|m| m.exists()) {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} is a file", file.display())));
    }
    for folder in missing.into_iter().rev() {
        let at = s.push(Planned { action: Action::CreateFolder, path: folder.clone(), bytes: None });
        s.folders.insert(folder, at);
    }
    Ok(())
}

/// [`std::fs::remove_file`]; a dry run takes back a planned write, or records the removal.
pub fn remove_file(path: &Path) -> io::Result<()> {
    if !diverted(path) {
        return std::fs::remove_file(path);
    }
    let key = key(path);
    let mut s = state();
    if let Some(bytes) = s.held.remove(&key) {
        s.held_bytes = s.held_bytes.saturating_sub(bytes.len());
        return Ok(());
    }
    if let Some(at) = s.writes.remove(&key) {
        s.forget(Some(at));
        return Ok(());
    }
    if std::fs::symlink_metadata(path).is_err() {
        return Err(not_found(path));
    }
    s.push(Planned { action: Action::Remove, path: path.to_path_buf(), bytes: None });
    Ok(())
}

/// [`std::fs::remove_dir`]; a dry run takes back a planned folder that nothing would be written
/// into, or records the removal.
pub fn remove_dir(path: &Path) -> io::Result<()> {
    if !diverted(path) {
        return std::fs::remove_dir(path);
    }
    let key = key(path);
    let mut s = state();
    if s.writes.range(key.clone()..).next().is_some_and(|(k, _)| k.starts_with(&key)) {
        return Err(io::Error::other(format!("{} is not empty", path.display())));
    }
    if let Some(at) = s.folders.remove(&key) {
        s.forget(at);
        return Ok(());
    }
    if !path.is_dir() {
        return Err(not_found(path));
    }
    s.push(Planned { action: Action::Remove, path: path.to_path_buf(), bytes: None });
    Ok(())
}

/// [`std::fs::rename`], which a dry run records as a removal and a write.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if !diverted(from) && !diverted(to) {
        return std::fs::rename(from, to);
    }
    let bytes = std::fs::metadata(from).ok().and_then(|m| usize::try_from(m.len()).ok());
    remove_file(from)?;
    plan_whole(to, bytes).map_err(|e| io::Error::other(e.to_string()))
}
