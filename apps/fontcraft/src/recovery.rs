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
//! folder, so a run never offers or removes the copies of another that is still open. Copies older than [`MAX_AGE`] are
//! removed when the window opens; past [`MAX_BYTES`] in the folder nothing more is written.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use craft::platform::autosave::{RecoveryEntry, RecoveryMeta, RecoveryStore, SettingsMode, recovery_dir};
use fontcraft_engine::guard::catch_panic;
use fontcraft_engine::recovery;
use fontcraft_ui::app::recovery::{Leftover, MAX_DOCUMENTS, RecoveryHost, Snapshot};

/// Copies older than this are removed unasked.
const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
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
    /// Answers once everything before it is done.
    Done(std::sync::mpsc::Sender<()>),
}

/// The copies of one run.
pub struct Copies {
    store: Arc<RecoveryStore>,
    jobs: Option<SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
    problem: Arc<Mutex<Option<String>>>,
    /// What the last [`RecoveryHost::leftovers`] listed.
    listed: Vec<RecoveryEntry>,
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
        Ok(Copies { store, jobs: Some(jobs), thread: Some(thread), problem, listed: Vec::new() })
    }

    /// Asks the thread for `job`. One that is not a copy waits for room; a copy is left out when there is none.
    fn ask(&self, job: Job) {
        let Some(jobs) = &self.jobs else { return };
        let lost = match jobs.try_send(job) {
            Ok(()) => false,
            Err(TrySendError::Full(Job::Write(_))) => false,
            Err(TrySendError::Full(job)) => jobs.send(job).is_err(),
            Err(TrySendError::Disconnected(_)) => true,
        };
        if lost {
            report(&self.problem, "the thread that writes them ended".into());
        }
    }

    fn entry(&self, key: &str) -> Option<RecoveryEntry> {
        self.listed.iter().find(|e| e.key == key).cloned()
    }
}

impl Drop for Copies {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            // The thread ends once it has done what was asked; its result says nothing more.
            let _ = thread.join();
        }
    }
}

/// Keeps the first failure until it is read.
fn report(problem: &Mutex<Option<String>>, text: String) {
    problem.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(text);
}

fn work(store: &RecoveryStore, jobs: &Receiver<Job>, problem: &Mutex<Option<String>>) {
    while let Ok(job) = jobs.recv() {
        let done = match job {
            Job::Write(snapshot) => write(store, &snapshot),
            Job::Discard(document) => store.discard(document).map_err(|e| e.to_string()),
            Job::Remove(entry) => store.remove(&entry).map_err(|e| e.to_string()),
            Job::Done(answer) => {
                let _ = answer.send(());
                Ok(())
            }
        };
        if let Err(e) = done {
            report(problem, e);
        }
    }
}

fn write(store: &RecoveryStore, snapshot: &Snapshot) -> Result<(), String> {
    let bytes = catch_panic(|| recovery::encode(&snapshot.font))??;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if folder_size(store.dir()).saturating_add(size) > MAX_BYTES {
        return Err(format!("{} holds more than {} GB", store.dir().display(), MAX_BYTES >> 30));
    }
    let meta = RecoveryMeta {
        name: snapshot.title.clone(),
        original: snapshot.original.clone(),
        extension: recovery::EXTENSION.into(),
        revision: snapshot.revision,
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
            let age = entry.saved.elapsed().unwrap_or_default();
            if age > MAX_AGE || kept.len() >= MAX_DOCUMENTS {
                // Past the age it is removed; past the count it stays for the next start.
                if age > MAX_AGE {
                    self.ask(Job::Remove(Box::new(entry)));
                }
                continue;
            }
            kept.push((entry, age));
        }
        self.listed = kept.iter().map(|(entry, _)| entry.clone()).collect();
        kept.into_iter()
            .map(|(entry, age)| Leftover { key: entry.key, title: entry.meta.name, original: entry.meta.original, path: entry.path, age })
            .collect()
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

    fn problem(&mut self) -> Option<String> {
        self.problem.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn finish(&mut self) {
        let (answer, done) = channel();
        self.ask(Job::Done(answer));
        let _ = done.recv_timeout(FINISH);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
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
}
