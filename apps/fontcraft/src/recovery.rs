//! The folder of copies of unsaved work: the window's `RecoveryHost` over the suite's recovery store.
//!
//! | Platform | Folder |
//! |---|---|
//! | Linux and others | `$XDG_STATE_HOME/fontcraft/Recovery`, else `~/.local/state/fontcraft/Recovery` |
//! | macOS | `Recovery` in the settings folder |
//! | Windows | `%LOCALAPPDATA%\FontCraft\Recovery` |
//!
//! With `FONTCRAFT_SETTINGS_DIR` the folder is `Recovery` in it; set but empty, nothing is copied.
//!
//! A copy is the document in the native format with a small file beside it that says what it is; both are written
//! atomically, by one thread that takes the window's requests in their order ([`Copies`]). Each run holds a lock in the
//! folder, so a run never offers or removes the copies of another that is still open. Only plain files this app names
//! are offered, removed or written: never a link, nor anything else in the folder. A copy that was asked about more than
//! [`MAX_AGE`] ago and is still there is removed when the window opens; one that was never asked about is kept whatever
//! its age. Past [`MAX_BYTES`] in the folder nothing more is written.

use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use craft::platform::autosave::{RecoveryEntry, RecoveryMeta, RecoveryStore, SettingsMode, recovery_dir};
use fontcraft_engine::guard::catch_panic;
use fontcraft_engine::recovery;
use fontcraft_ui::app::recovery::{Leftover, MAX_DOCUMENTS, Problem, RecoveryHost, Snapshot};

/// Copies first asked about longer ago than this, and not written since, are removed.
const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// The extension of the file beside a copy that says it was asked about.
const OFFERED: &str = "offered";
/// Most bytes the folder may hold.
const MAX_BYTES: u64 = 2 << 30;
/// Most files of the folder counted.
const MAX_FILES: usize = 100_000;
/// Most requests waiting for the thread. A copy that does not fit is written with the document's next change.
const MAX_WAITING: usize = 4 * MAX_DOCUMENTS;
/// Longest wait for the thread as the window closes.
const FINISH: Duration = Duration::from_secs(10);

enum Job {
    Write(Snapshot),
    Discard(u64),
    Remove(Box<RecoveryEntry>),
    /// The copy of this key was asked about.
    Offered(String),
    /// Answers once everything before it is done.
    Done(std::sync::mpsc::Sender<()>),
}

/// The copies of one run.
pub struct Copies {
    store: Arc<RecoveryStore>,
    jobs: Option<SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
    problem: Arc<Mutex<Option<Problem>>>,
    /// What the last [`RecoveryHost::leftovers`] listed.
    listed: Vec<RecoveryEntry>,
    /// The thread did not finish in [`FINISH`]: the window closes without it.
    stuck: bool,
}

impl Copies {
    /// The copies in the folder that goes with the settings folder; `None` when everything is kept in memory or the
    /// folder cannot be made.
    pub fn open(settings_dir: Option<&Path>) -> Option<Copies> {
        let settings_dir = settings_dir?;
        let own = std::env::var_os(crate::user_files::DIR_VARIABLE).is_some_and(|v| !v.is_empty());
        let mode = if own { SettingsMode::SelfContained } else { SettingsMode::Platform };
        let dir = recovery_dir("FontCraft", "fontcraft", Some(settings_dir), mode, |name| std::env::var_os(name))?;
        match Copies::at(&dir) {
            Ok(copies) => Some(copies),
            Err(e) => {
                eprintln!("fontcraft: no copies of unsaved work are kept: {}: {e}", dir.display());
                None
            }
        }
    }

    fn at(dir: &Path) -> std::io::Result<Copies> {
        let store = Arc::new(RecoveryStore::open(dir)?);
        let problem = Arc::new(Mutex::new(None));
        let (jobs, waiting) = sync_channel(MAX_WAITING);
        let thread = {
            let (store, problem) = (Arc::clone(&store), Arc::clone(&problem));
            std::thread::Builder::new().name("recovery copies".into()).spawn(move || work(&store, &waiting, &problem))?
        };
        Ok(Copies { store, jobs: Some(jobs), thread: Some(thread), problem, listed: Vec::new(), stuck: false })
    }

    /// Asks the thread for `job`. One that is not a copy waits for room; a copy is refused when there is none.
    fn ask(&self, job: Job) {
        let Some(jobs) = &self.jobs else { return };
        let lost = match jobs.try_send(job) {
            Ok(()) => false,
            Err(TrySendError::Full(Job::Write(snapshot))) => {
                report(&self.problem, Some(snapshot.document), "too many copies are waiting to be written".into());
                false
            }
            Err(TrySendError::Full(job)) => jobs.send(job).is_err(),
            Err(TrySendError::Disconnected(_)) => true,
        };
        if lost {
            report(&self.problem, None, "the thread that writes them ended".into());
        }
    }

    /// The listed copy `key`, if it is still one to answer for: another run may have recovered it, or something put
    /// another file in its place.
    fn entry(&self, key: &str) -> Option<RecoveryEntry> {
        let listed = self.listed.iter().find(|e| e.key == key)?;
        self.store.leftovers().into_iter().find(|e| e.key == key && e.path == listed.path && ours(self.store.dir(), e))
    }
}

impl Drop for Copies {
    fn drop(&mut self) {
        self.jobs = None;
        if self.stuck {
            return;
        }
        if let Some(thread) = self.thread.take() {
            // The thread ends once it has done what was asked; its result says nothing more.
            let _ = thread.join();
        }
    }
}

/// Keeps the first failure, and every document without its copy, until they are read.
fn report(problem: &Mutex<Option<Problem>>, document: Option<u64>, text: String) {
    let mut kept = problem.lock().unwrap_or_else(PoisonError::into_inner);
    let kept = kept.get_or_insert_with(|| Problem { text, documents: Vec::new() });
    if let Some(id) = document
        && !kept.documents.contains(&id)
    {
        kept.documents.push(id);
    }
}

/// Whether `key` is a name this app gives its copies: `doc-<run>-<document>`.
fn our_key(key: &str) -> bool {
    key.strip_prefix("doc-").and_then(|rest| rest.rsplit_once('-')).is_some_and(|(run, id)| {
        !run.is_empty() && run.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
    })
}

fn plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

/// Whether nothing is at `path`, or a plain file is: what may be written.
fn writable(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(m) => m.is_file(),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Whether `entry` is a copy this app wrote: its name, its extension, and plain files.
fn ours(dir: &Path, entry: &RecoveryEntry) -> bool {
    our_key(&entry.key)
        && entry.path.extension().is_some_and(|x| x == recovery::EXTENSION)
        && plain_file(&entry.path)
        && writable(&dir.join(format!("{}.json", entry.key)))
}

/// How long ago the copy was first asked about, if it was and has not been written since. A copy without its sidecar
/// was never recorded as asked about.
fn offered(dir: &Path, entry: &RecoveryEntry) -> Option<Duration> {
    let mark = std::fs::symlink_metadata(dir.join(format!("{}.{OFFERED}", entry.key))).ok()?;
    let at = mark.modified().ok().filter(|at| mark.is_file() && *at >= entry.saved)?;
    at.elapsed().ok()
}

/// Records that the copy `key` was asked about, unless a mark since its last writing says so already.
fn mark_offered(store: &RecoveryStore, key: &str) -> std::io::Result<()> {
    let Some(entry) = store.leftovers().into_iter().find(|e| e.key == key && ours(store.dir(), e)) else { return Ok(()) };
    let sidecar = store.dir().join(format!("{key}.json"));
    let mark = store.dir().join(format!("{key}.{OFFERED}"));
    if !plain_file(&sidecar) || !writable(&mark) || offered(store.dir(), &entry).is_some() {
        return Ok(());
    }
    std::fs::write(mark, b"asked")
}

fn work(store: &RecoveryStore, jobs: &Receiver<Job>, problem: &Mutex<Option<Problem>>) {
    while let Ok(job) = jobs.recv() {
        let document = if let Job::Write(snapshot) = &job { Some(snapshot.document) } else { None };
        let done = match job {
            Job::Offered(key) => mark_offered(store, &key).map_err(|e| e.to_string()),
            Job::Write(snapshot) => write(store, &snapshot),
            Job::Discard(document) => store.discard(document).map_err(|e| e.to_string()),
            Job::Remove(entry) => store.remove(&entry).map_err(|e| e.to_string()),
            Job::Done(answer) => {
                let _ = answer.send(());
                Ok(())
            }
        };
        if let Err(e) = done {
            report(problem, document, e);
        }
    }
}

fn write(store: &RecoveryStore, snapshot: &Snapshot) -> Result<(), String> {
    let bytes = catch_panic(|| recovery::encode(&snapshot.font))??;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if folder_size(store.dir()).saturating_add(size) > MAX_BYTES {
        return Err(format!("{} holds more than {} GB", store.dir().display(), MAX_BYTES >> 30));
    }
    let key = store.key(snapshot.document);
    for extension in [recovery::EXTENSION, "json"] {
        // Writing follows a link, which could lead to a file of the user's.
        let path = store.dir().join(format!("{key}.{extension}"));
        if !writable(&path) {
            return Err(format!("{} is not a plain file", path.display()));
        }
    }
    let meta = RecoveryMeta {
        name: snapshot.title.clone(),
        // The file beside the copy is text.
        original: snapshot.original.clone().filter(|p| p.to_str().is_some()),
        extension: recovery::EXTENSION.into(),
        revision: snapshot.revision,
        // FontCraft's copies open without a password.
        protected: false,
    };
    store.write(snapshot.document, &meta, &bytes).map_err(|e| e.to_string())
}

/// The bytes of the files in `dir`; one past the limit of files counts as full.
fn folder_size(dir: &Path) -> u64 {
    let Ok(read) = std::fs::read_dir(dir) else { return 0 };
    let mut size = 0u64;
    for (at, file) in read.flatten().enumerate() {
        if at >= MAX_FILES {
            return u64::MAX;
        }
        size = size.saturating_add(file.metadata().map_or(0, |m| m.len()));
    }
    size
}

impl RecoveryHost for Copies {
    fn leftovers(&mut self) -> Vec<Leftover> {
        let mut kept = Vec::new();
        for entry in self.store.leftovers() {
            if !ours(self.store.dir(), &entry) {
                continue;
            }
            if offered(self.store.dir(), &entry).is_some_and(|ago| ago > MAX_AGE) {
                self.ask(Job::Remove(Box::new(entry)));
            } else if kept.len() < MAX_DOCUMENTS {
                // Past the count it stays for the next start.
                let age = entry.saved.elapsed().unwrap_or_default();
                kept.push((entry, age));
            }
        }
        self.listed = kept.iter().map(|(entry, _)| entry.clone()).collect();
        kept.into_iter()
            .map(|(entry, age)| Leftover { key: entry.key, title: entry.meta.name, original: entry.meta.original, path: entry.path, age })
            .collect()
    }

    fn offered(&mut self, key: &str) {
        self.ask(Job::Offered(key.to_string()));
    }

    fn write(&mut self, snapshot: Snapshot) {
        self.ask(Job::Write(snapshot));
    }

    fn discard(&mut self, document: u64) {
        self.ask(Job::Discard(document));
    }

    fn adopt(&mut self, document: u64, key: &str) {
        if let Some(entry) = self.entry(key) {
            self.store.adopt(document, &entry);
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entry(key) {
            self.ask(Job::Remove(Box::new(entry)));
        }
    }

    fn problem(&mut self) -> Option<Problem> {
        self.problem.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn finish(&mut self) {
        let (answer, done) = channel();
        self.ask(Job::Done(answer));
        self.stuck = done.recv_timeout(FINISH).is_err();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::path::PathBuf;

    use fontcraft_engine::Session;
    use serde_json::json;

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-recovery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn edited() -> Session {
        let mut s = Session::with_document();
        s.execute("glyph.newGlyph", &json!({"name": "recoveredGlyph"})).unwrap();
        s
    }

    fn snapshot(s: &Session, document: u64) -> Snapshot {
        let doc = s.active().unwrap();
        Snapshot { document, title: "Work".into(), original: None, revision: doc.revision, font: Arc::clone(&doc.font) }
    }

    /// A run that ends without closing its document leaves the copy, and the next run opens it equal; two runs at once
    /// see nothing of each other; a document's save takes the copy away.
    #[test]
    fn a_copy_outlives_its_run_and_comes_back_equal() {
        let dir = folder("kill");
        let s = edited();
        let mut first = Copies::at(&dir).unwrap();
        first.write(snapshot(&s, 1));
        first.finish();
        let mut second = Copies::at(&dir).unwrap();
        assert!(second.leftovers().is_empty(), "the copies of a run that is still open are its own");
        drop(first);

        let found = second.leftovers();
        assert_eq!((found.len(), found[0].title.as_str()), (1, "Work"));
        let mut back = Session::new();
        back.execute("file.openRecovered", &json!({"path": found[0].path.to_string_lossy(), "title": found[0].title})).unwrap();
        let doc = back.active().unwrap();
        assert!(doc.modified() && doc.path.is_none());
        assert_eq!(*doc.font, *s.active().unwrap().font);
        second.adopt(doc.id, &found[0].key);
        second.discard(doc.id);
        second.finish();
        assert_eq!(second.problem(), None);
        assert!(!found[0].path.exists(), "saved or closed: no copy");
        drop(second);
        assert!(Copies::at(&dir).unwrap().leftovers().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder something else wrote into lists nothing that opens as a font, and discarding removes it.
    #[test]
    fn a_hostile_copy_is_listed_refused_and_removed() {
        let dir = folder("hostile");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("doc-x-1.fontcraft"), b"\x1f\x8bnot a font").unwrap();
        std::fs::write(dir.join("doc-x-1.json"), b"{\"name\": \"../../etc\", \"original\": null, \"extension\": \"fontcraft\", \"revision\": 1}")
            .unwrap();
        let mut copies = Copies::at(&dir).unwrap();
        let found = copies.leftovers();
        assert_eq!(found.len(), 1);
        assert!(found[0].path.starts_with(&dir));
        assert!(Session::new().execute("file.openRecovered", &json!({"path": found[0].path.to_string_lossy()})).is_err());
        copies.remove(&found[0].key);
        copies.remove("../../etc");
        copies.finish();
        assert!(!found[0].path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nothing the app did not write is listed or removed, and a copy is removed for its age only once it was asked
    /// about.
    #[test]
    fn only_copies_of_ours_that_were_asked_about_grow_old() {
        let dir = folder("age");
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::time::SystemTime::now() - 3 * MAX_AGE;
        let touch = |path: &Path, at: std::time::SystemTime| std::fs::File::options().write(true).open(path).unwrap().set_modified(at).unwrap();
        let bytes = recovery::encode(&edited().active().unwrap().font).unwrap();
        for name in ["MyFont.fontcraft", "MyFont.glyphs", "doc-x-2.glyphs", "doc-x-1.fontcraft"] {
            std::fs::write(dir.join(name), &bytes).unwrap();
            touch(&dir.join(name), old);
        }
        std::fs::write(dir.join("doc-x-1.json"), b"{\"name\": \"Work\", \"original\": null, \"extension\": \"fontcraft\", \"revision\": 1}").unwrap();
        touch(&dir.join("doc-x-1.json"), old);
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("MyFont.fontcraft"), dir.join("doc-x-3.fontcraft")).unwrap();

        let keys = |copies: &mut Copies| copies.leftovers().into_iter().map(|l| l.key).collect::<Vec<_>>();
        let mut copies = Copies::at(&dir).unwrap();
        assert_eq!(keys(&mut copies), ["doc-x-1"], "never asked about: kept, whatever its age");
        copies.offered("doc-x-1");
        copies.finish();
        drop(copies);
        let mut copies = Copies::at(&dir).unwrap();
        assert_eq!(keys(&mut copies), ["doc-x-1"], "asked about just now");
        drop(copies);
        touch(&dir.join("doc-x-1.offered"), old + MAX_AGE);
        let mut copies = Copies::at(&dir).unwrap();
        assert!(keys(&mut copies).is_empty());
        copies.finish();
        assert!(!dir.join("doc-x-1.fontcraft").exists());
        for name in ["MyFont.fontcraft", "MyFont.glyphs", "doc-x-2.glyphs"] {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), bytes, "{name}");
        }
        drop(copies);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
