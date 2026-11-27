//! Recovery of unsaved work.
//!
//! The desktop app supplies a [`RecoveryHost`], which keeps copies of open documents in a folder of its own and writes
//! them off the window's thread. Each frame [`tick`] hands it a snapshot of every document with unsaved changes that has
//! changed since its last copy, at most every [`INTERVAL`] and when the window loses the focus, and takes away the
//! copies of documents that were saved or closed. The snapshot is the document's own `Arc`, so it costs the window
//! nothing. No copy ever goes into the user's own file.
//!
//! After a run that ended without closing its documents, [`start`] lists the copies it left, and the Recover dialog asks
//! about them one at a time: Recover opens the copy as a document with unsaved changes ([`fontcraft_engine`]'s
//! `file.openRecovered`), Discard removes it, Escape keeps it for the next start. A failed write says so in the status
//! line once. `app.recoverDocuments` reports and answers the same, so the control channel can drive it.
//!
//! Projects have no copies. Rows: `app.recoverDocuments`, `app.crashReportOnNextLaunch`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use craft::ui::dialog::{DialogHost, DialogResult, RecoveredDocument, RecoveryAnswer, RecoveryChoice, RecoveryDialog};
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::Font;
use serde_json::{Map, Value, json};

use super::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::params;

/// The shortest time between two copies of one document.
pub const INTERVAL: Duration = Duration::from_secs(30);
/// Most documents copied, and most copies offered.
pub const MAX_DOCUMENTS: usize = 32;
/// The command that reports and answers.
pub const RECOVER: &str = "app.recoverDocuments";

/// The revision of a copy that has to be written again; no document has it.
const NONE: u64 = u64::MAX;

/// A snapshot of a document for the host to write.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The document's id in this session.
    pub document: u64,
    pub title: String,
    /// The file it was opened from or last saved to.
    pub original: Option<PathBuf>,
    pub revision: u64,
    pub font: Arc<Font>,
}

/// A copy an earlier run left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    /// Names the copy to the host.
    pub key: String,
    pub title: String,
    pub original: Option<PathBuf>,
    /// The copy.
    pub path: PathBuf,
    /// How long before the list was made it was written.
    pub age: Duration,
}

/// What went wrong with the copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The first failure.
    pub text: String,
    /// The documents whose copy was not written.
    pub documents: Vec<u64>,
}

/// Where the desktop app keeps the copies. Every call returns at once but [`finish`](Self::finish); the writing happens
/// on another thread, in the order of the calls.
pub trait RecoveryHost: Send {
    /// The copies of runs that are gone, newest first.
    fn leftovers(&mut self) -> Vec<Leftover>;
    /// The leftover `key` is being asked about.
    fn offered(&mut self, key: &str);
    /// Writes a copy of a document, in place of its last one.
    fn write(&mut self, snapshot: Snapshot);
    /// Removes a document's copy: it was saved or closed.
    fn discard(&mut self, document: u64);
    /// The document was recovered from the leftover `key`: its copies replace that one, unless another run took it
    /// meanwhile.
    fn adopt(&mut self, document: u64, key: &str);
    /// Removes a leftover.
    fn remove(&mut self, key: &str);
    /// What went wrong since the last call.
    fn problem(&mut self) -> Option<Problem>;
    /// The window closes and the user has answered for every document: waits until what was asked is done.
    fn finish(&mut self);
}

/// A document's last copy.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Copied {
    revision: u64,
    /// The input time of the copy, in seconds.
    at: f64,
}

/// What the window keeps of the recovery.
#[derive(Default)]
pub struct Recovery {
    pub host: Option<Box<dyn RecoveryHost>>,
    /// The documents with a copy, by id. A recovered document has one from the start.
    copied: BTreeMap<u64, Copied>,
    /// When each changed document without a copy was first seen changed, in seconds.
    changed: BTreeMap<u64, f64>,
    focused: bool,
    /// The time of the last failure, in seconds. While failures follow one another only the first is reported.
    failed: Option<f64>,
    /// The copies still to ask about.
    pub pending: Vec<Leftover>,
    /// The copy the open dialog asks about: its answer is for that one, whatever was answered meanwhile.
    asked: Option<String>,
}

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: RECOVER,
    label: "Recover Unsaved Work",
    menu: &[],
    shortcut: None,
    params: "{key?: string (default: the copy asked about), answer?: \"recover\" | \"discard\" | \"later\"} the copies of unsaved work an earlier run left behind. With answer, answers for one as the dialog's buttons do: recover opens it as a document with unsaved changes, discard removes it, later keeps it for the next start. ui.inspect reports them as recovery: {pending: [{key, title, original, ageSeconds}]}",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Lists what an earlier run left, when the window opens.
pub fn start(app: &mut FontCraftApp, host: Option<Box<dyn RecoveryHost>>) {
    app.recovery.host = host;
    if let Some(host) = app.recovery.host.as_mut() {
        let mut found = host.leftovers();
        found.truncate(MAX_DOCUMENTS);
        app.recovery.pending = found;
    }
}

/// Hands the host what changed. Returns how long until it has more to write.
pub fn tick(app: &mut FontCraftApp, ctx: &egui::Context) -> Option<Duration> {
    let (now, focused) = ctx.input(|i| (i.time, i.focused));
    tick_at(app, now, focused)
}

fn tick_at(app: &mut FontCraftApp, now: f64, focused: bool) -> Option<Duration> {
    let recovery = &mut app.recovery;
    let host = recovery.host.as_mut()?;
    let lost_focus = std::mem::replace(&mut recovery.focused, focused) && !focused;
    let documents = app.session.documents();
    let unsaved = |id: u64| documents.iter().any(|d| d.id == id && d.modified());
    let done: Vec<u64> = recovery.copied.keys().copied().filter(|id| !unsaved(*id)).collect();
    for id in done {
        recovery.copied.remove(&id);
        host.discard(id);
    }
    recovery.changed.retain(|id, _| unsaved(*id));

    let mut next: Option<Duration> = None;
    for doc in documents.iter().filter(|d| d.modified()).take(MAX_DOCUMENTS) {
        let since = match recovery.copied.get(&doc.id) {
            Some(copied) if copied.revision == doc.revision => continue,
            Some(copied) => copied.at,
            None => *recovery.changed.entry(doc.id).or_insert(now),
        };
        let left = INTERVAL.as_secs_f64() - (now - since);
        // A clock that went back counts as due.
        if left > 0.0 && left <= INTERVAL.as_secs_f64() && !lost_focus {
            let left = Duration::from_secs_f64(left);
            next = Some(next.map_or(left, |n| n.min(left)));
            continue;
        }
        host.write(Snapshot {
            document: doc.id,
            title: doc.title.clone(),
            original: doc.path.clone(),
            revision: doc.revision,
            font: Arc::clone(&doc.font),
        });
        recovery.copied.insert(doc.id, Copied { revision: doc.revision, at: now });
        recovery.changed.remove(&doc.id);
    }
    if let Some(problem) = host.problem() {
        // Written again after the interval.
        for id in problem.documents {
            if let Some(copied) = recovery.copied.get_mut(&id) {
                *copied = Copied { revision: NONE, at: now };
                next = Some(next.map_or(INTERVAL, |n| n.min(INTERVAL)));
            }
        }
        let quiet = 3.0 * INTERVAL.as_secs_f64();
        if recovery.failed.replace(now).is_none_or(|at| !(0.0..=quiet).contains(&(now - at))) {
            log::warn!("recovery copies: {}", problem.text);
            app.status = format!("Unsaved work could not be copied for recovery: {}", problem.text);
        }
    }
    next
}

/// The window closes and every document was saved or left unsaved on purpose: no copy of this run stays.
pub fn finish(app: &mut FontCraftApp) {
    let recovery = &mut app.recovery;
    // Taken: a frame after this one copies nothing.
    let Some(mut host) = recovery.host.take() else { return };
    for id in std::mem::take(&mut recovery.copied).into_keys() {
        host.discard(id);
    }
    host.finish();
}

/// `ui.inspect`'s `recovery`.
pub fn inspect(app: &FontCraftApp, out: &mut Map<String, Value>) {
    let pending: Vec<Value> = app
        .recovery
        .pending
        .iter()
        .map(|l| json!({"key": l.key, "title": l.title, "original": l.original.as_ref().map(|p| p.to_string_lossy()), "ageSeconds": l.age.as_secs()}))
        .collect();
    out.insert("recovery".into(), json!({ "pending": pending }));
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    let key = params::string(p, "key")?;
    let Some(answer) = params::string(p, "answer")? else { return Ok(()) };
    if !matches!(answer, "recover" | "discard" | "later") {
        return Err(format!("`answer` is recover, discard or later, not `{}`", params::quoted(answer)));
    }
    let at = match key {
        None => (!app.recovery.pending.is_empty()).then_some(0),
        Some(key) => app.recovery.pending.iter().position(|l| l.key == key),
    };
    let leftover = at.and_then(|at| app.recovery.pending.get(at)).cloned().ok_or("there is no such copy of unsaved work")?;
    // A copy that cannot be opened stays on disk and is not asked about again in this run.
    app.recovery.pending.retain(|l| l.key != leftover.key);
    match answer {
        "recover" => recover(app, &leftover),
        "discard" => {
            if let Some(host) = app.recovery.host.as_mut() {
                host.remove(&leftover.key);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn recover(app: &mut FontCraftApp, leftover: &Leftover) -> Result<(), String> {
    let mut given = json!({"path": leftover.path.to_string_lossy(), "title": leftover.title});
    if let (Some(original), Some(map)) = (&leftover.original, given.as_object_mut()) {
        // A name that is not text would name another file.
        if let Some(original) = original.to_str() {
            map.insert("original".into(), json!(original));
        }
    }
    let reply = app.run_command("file.openRecovered", given)?;
    if let (Some(id), Some(host)) = (reply.get("document").and_then(Value::as_u64), app.recovery.host.as_mut()) {
        host.adopt(id, &leftover.key);
        // Written at once, so the copy names this run and no other window offers it.
        app.recovery.copied.insert(id, Copied { revision: NONE, at: f64::NEG_INFINITY });
    }
    app.status = format!("Recovered `{}`: it is not saved yet", leftover.title);
    app.view.start.open = false;
    Ok(())
}

/// Asks about the first copy left; the next one once it is answered.
pub fn run_dialog(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(result) = host.take::<RecoveryDialog>() {
        let answer = match result {
            DialogResult::Ok(RecoveryChoice { answer: RecoveryAnswer::Recover, .. }) => "recover",
            DialogResult::Ok(RecoveryChoice { answer: RecoveryAnswer::Discard, .. }) => "discard",
            _ => "later",
        };
        // A copy the control channel answered for meanwhile is no longer pending.
        if let Some(key) = app.recovery.asked.take().filter(|key| app.recovery.pending.iter().any(|l| l.key == *key)) {
            app.sent.push(Msg::Shell(CommandRef::with_params(RECOVER, json!({ "key": key, "answer": answer }))));
        }
        return;
    }
    let answering = app.sent.iter().any(|m| m.command_id() == Some(RECOVER));
    if answering || host.is_open::<RecoveryDialog>() {
        return;
    }
    if let Some(first) = app.recovery.pending.first() {
        app.recovery.asked = Some(first.key.clone());
        if let Some(copies) = app.recovery.host.as_mut() {
            copies.offered(&first.key);
        }
        host.open(RecoveryDialog::new(RecoveredDocument::new(first.key.clone(), first.title.clone(), first.age)));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::testing::TestApp;

    #[derive(Default)]
    struct Log {
        calls: Vec<String>,
        leftovers: Vec<Leftover>,
        problem: Option<Problem>,
    }

    struct Fake(Arc<Mutex<Log>>);

    impl Fake {
        fn call(&self, text: String) {
            self.0.lock().unwrap().calls.push(text);
        }
    }

    impl RecoveryHost for Fake {
        fn leftovers(&mut self) -> Vec<Leftover> {
            self.0.lock().unwrap().leftovers.clone()
        }
        fn offered(&mut self, key: &str) {
            self.call(format!("offered {key}"));
        }
        fn write(&mut self, snapshot: Snapshot) {
            self.call(format!("write {}", snapshot.document));
        }
        fn discard(&mut self, document: u64) {
            self.call(format!("discard {document}"));
        }
        fn adopt(&mut self, document: u64, key: &str) {
            self.call(format!("adopt {document} {key}"));
        }
        fn remove(&mut self, key: &str) {
            self.call(format!("remove {key}"));
        }
        fn problem(&mut self) -> Option<Problem> {
            self.0.lock().unwrap().problem.take()
        }
        fn finish(&mut self) {
            self.call("finish".into());
        }
    }

    fn calls(log: &Arc<Mutex<Log>>) -> Vec<String> {
        std::mem::take(&mut log.lock().unwrap().calls)
    }

    /// A changed document is copied after the interval, at once when the window loses the focus, and not again until it
    /// changes; undoing to the saved state, saving or closing takes the copy away, and so does the window's close.
    #[test]
    fn changed_documents_are_copied_and_saved_ones_are_not() {
        let mut t = TestApp::new();
        let log = Arc::new(Mutex::new(Log::default()));
        start(&mut t.app, Some(Box::new(Fake(Arc::clone(&log)))));
        let id = t.app.session.active().unwrap().id;
        assert_eq!(tick_at(&mut t.app, 0.0, true), None);
        t.ok("engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "recoverA"}}));
        calls(&log);
        t.app.recovery.changed.clear();
        assert_eq!(tick_at(&mut t.app, 100.0, true), Some(INTERVAL));
        assert!(calls(&log).is_empty());
        tick_at(&mut t.app, 131.0, true);
        assert_eq!(calls(&log), [format!("write {id}")]);
        tick_at(&mut t.app, 200.0, true);
        assert!(calls(&log).is_empty(), "no change, no copy");
        t.ok("engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "recoverB"}}));
        calls(&log);
        t.app.recovery.copied.insert(id, Copied { revision: 0, at: 200.0 });
        tick_at(&mut t.app, 205.0, true);
        assert!(calls(&log).is_empty(), "within the interval");
        tick_at(&mut t.app, 206.0, false);
        assert_eq!(calls(&log), [format!("write {id}")], "the window lost the focus");

        t.ok("engine.execute", json!({"command": "edit.undo", "params": {}}));
        t.ok("engine.execute", json!({"command": "edit.undo", "params": {}}));
        assert!(!t.app.session.active().unwrap().modified());
        t.frame();
        assert!(calls(&log).contains(&format!("discard {id}")), "an unchanged document has no copy");

        t.ok("engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "recoverC"}}));
        tick_at(&mut t.app, 1000.0, true);
        tick_at(&mut t.app, 1001.0, false);
        calls(&log);
        finish(&mut t.app);
        assert_eq!(calls(&log), [format!("discard {id}"), "finish".into()]);
        tick_at(&mut t.app, 2000.0, false);
        assert!(calls(&log).is_empty(), "nothing is copied once the window closes");
    }

    #[test]
    fn a_failed_copy_is_reported_once_and_written_again() {
        let mut t = TestApp::new();
        let log = Arc::new(Mutex::new(Log::default()));
        start(&mut t.app, Some(Box::new(Fake(Arc::clone(&log)))));
        let id = t.app.session.active().unwrap().id;
        t.ok("engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "recoverA"}}));
        t.app.recovery.changed.clear();
        tick_at(&mut t.app, 1000.0, true);
        tick_at(&mut t.app, 1031.0, true);
        calls(&log);
        for (now, shown) in [(1032.0, true), (1063.0, false), (2000.0, true)] {
            log.lock().unwrap().problem = Some(Problem { text: "the disk is full".into(), documents: vec![id] });
            t.app.status.clear();
            let next = tick_at(&mut t.app, now, true);
            assert_eq!((!t.app.status.is_empty(), next), (shown, Some(INTERVAL)), "{now}");
        }
        calls(&log);
        tick_at(&mut t.app, 2031.0, true);
        assert_eq!(calls(&log), [format!("write {id}")], "the unsaved work has no copy yet");
    }

    /// The copy of an earlier run opens as a document with unsaved changes; saving over its file asks first. A hostile
    /// copy is refused with a message, and another is discarded.
    #[test]
    fn leftovers_are_recovered_or_discarded() {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let font = Font::new("Recovered");
        let original = dir.join("Original.fontcraft");
        std::fs::write(&original, fontcraft_engine::recovery::encode(&Font::new("Original")).unwrap()).unwrap();
        let before = std::fs::read(&original).unwrap();
        let copy = dir.join("copy.fontcraft");
        std::fs::write(&copy, fontcraft_engine::recovery::encode(&font).unwrap()).unwrap();
        let hostile = dir.join("hostile.fontcraft");
        std::fs::write(&hostile, "{\"format\": \"nothing of ours\"}").unwrap();
        let leftover = |key: &str, path: &PathBuf, original: Option<PathBuf>| Leftover {
            key: key.into(),
            title: "Work".into(),
            original,
            path: path.clone(),
            age: Duration::from_secs(90),
        };
        let log = Arc::new(Mutex::new(Log {
            leftovers: vec![leftover("a", &copy, Some(original.clone())), leftover("b", &hostile, None), leftover("c", &copy, None)],
            ..Log::default()
        }));
        let mut t = TestApp::new();
        start(&mut t.app, Some(Box::new(Fake(Arc::clone(&log)))));
        t.frames(2);
        assert!(calls(&log).contains(&"offered a".to_string()));
        t.ok("engine.execute", json!({"command": RECOVER, "params": {"answer": "recover"}}));
        t.frame();
        let doc = t.app.session.active().unwrap();
        assert!(doc.modified(), "unsaved");
        assert_eq!((doc.title.as_str(), doc.path.as_deref(), doc.font.family_name.as_str()), ("Work", Some(original.as_path()), "Recovered"));
        let id = doc.id;
        let done = calls(&log);
        let at = |call: String| done.iter().position(|c| *c == call).unwrap_or_else(|| panic!("{call}: {done:?}"));
        assert!(at(format!("adopt {id} a")) < at(format!("write {id}")), "the copy is this run's at once");
        // The window gets the focus back: the file is as it was when the work was recovered.
        t.frame_with(vec![egui::Event::WindowFocused(false)]);
        t.frame_with(vec![egui::Event::WindowFocused(true)]);
        t.frames(2);
        assert!(!t.app.view.extensions.external.open, "the file's old state is not offered over the recovered work");
        assert!(fontcraft_engine::cmd::external_changes::changed_on_disk(&t.app.session).is_empty());
        let review = t.ok("engine.execute", json!({"command": "file.reviewExternalChanges", "params": {}}));
        assert_eq!((review["changed"].as_bool(), review["saveNeedsForce"].as_bool()), (Some(false), Some(true)), "{review}");
        assert_eq!(t.app.session.font().unwrap().family_name, "Recovered");
        let error = t.error("engine.execute", json!({"command": "file.save", "params": {}}));
        assert!(error.contains("changed on disk"), "{error}");
        assert_eq!(std::fs::read(&original).unwrap(), before, "the user's file is untouched");

        let documents = t.app.session.documents().len();
        assert!(!t.error("engine.execute", json!({"command": RECOVER, "params": {"key": "b", "answer": "recover"}})).is_empty());
        assert_eq!(t.app.session.documents().len(), documents, "nothing opens");
        t.ok("engine.execute", json!({"command": RECOVER, "params": {"answer": "discard"}}));
        assert!(calls(&log).contains(&"remove c".to_string()));
        assert!(t.app.recovery.pending.is_empty());
        assert!(!t.error("engine.execute", json!({"command": RECOVER, "params": {"answer": "recover"}})).is_empty());

        t.ok("engine.execute", json!({"command": "file.save", "params": {"force": true}}));
        t.frame();
        assert!(calls(&log).contains(&format!("discard {id}")), "a save takes the copy away");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
