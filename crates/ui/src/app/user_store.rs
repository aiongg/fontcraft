//! The user's settings kept between runs.
//!
//! The settings live in the session's settings store (`settings.get`, `settings.set`, the Settings window, every
//! switch that is a setting). The desktop app supplies a [`UserStore`] that reads them, with the user's custom glyph
//! data, from the settings folder when the window opens ([`load`]), and writes them back whenever they change
//! ([`save`], once per frame). The UI crate reads and writes no files itself; without a store nothing is kept. View
//! state kept between runs lives in the settings too (the reporter switches, `view.shownReporters`); [`load`] puts it
//! back in the view.

use fontcraft_engine::Session;

use super::FontCraftApp;

/// Where the desktop app keeps the user's settings.
pub trait UserStore: Send {
    /// Reads the stored settings (and what else the settings folder holds for the session) into `session`; the problems
    /// found, one line each. Nothing here may stop the app: a file that cannot be read leaves the defaults.
    fn load(&mut self, session: &mut Session) -> Vec<String>;
    /// Writes the settings when they changed since the last read or write; a failure as a message.
    fn save_if_changed(&mut self, session: &Session) -> Option<String>;
}

/// Reads the store into the session, before the window first asks a setting (the Start Window's Show at Launch). The
/// last problem goes to the status line, every one to the log.
pub fn load(app: &mut FontCraftApp) {
    let Some(store) = app.user_store.as_mut() else { return };
    let problems = store.load(&mut app.session);
    for problem in &problems {
        log::warn!("{problem}");
    }
    if let Some(last) = problems.last() {
        app.status = last.clone();
    }
    // View state kept in the settings.
    app.view.extensions.reporters = crate::shell::extensions::stored_reporters(&app.session);
    // Loaded settings may carry the same revision as the defaults: the projection reads them again.
    app.view_changed();
    app.project();
}

/// Writes the settings when they changed; a failure goes to the status line and is tried again on the next change.
pub fn save(app: &mut FontCraftApp) {
    let Some(store) = app.user_store.as_mut() else { return };
    if let Some(problem) = store.save_if_changed(&app.session) {
        log::error!("{problem}");
        app.status = problem;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use craft::host::Host;
    use serde_json::{Value, json};

    use super::*;
    use crate::app::{APP, Msg};

    const SETTING: &str = "appearance.displayUnicodeValue";

    /// A store holding one value of [`SETTING`], recording what it writes.
    #[derive(Clone, Default)]
    struct Memory {
        written: Arc<Mutex<Vec<Value>>>,
        revision: u64,
    }

    impl UserStore for Memory {
        fn load(&mut self, session: &mut Session) -> Vec<String> {
            session.execute("settings.set", &json!({"id": SETTING, "value": true})).unwrap();
            self.revision = session.settings().revision();
            vec!["a stored file was left out".into()]
        }

        fn save_if_changed(&mut self, session: &Session) -> Option<String> {
            let revision = session.settings().revision();
            if revision == self.revision {
                return None;
            }
            self.revision = revision;
            self.written.lock().unwrap().push(session.settings().get(SETTING).unwrap_or(Value::Null));
            None
        }
    }

    /// The stored settings are in the session once the window opens; a change is written once, and nothing is written
    /// while nothing changes.
    #[test]
    fn stored_settings_load_at_open_and_changes_are_written() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let store = Memory::default();
        app.user_store = Some(Box::new(store.clone()));
        load(&mut app);
        assert_eq!(app.session.settings().get_bool(SETTING), Some(true));
        assert_eq!(app.status, "a stored file was left out");

        save(&mut app);
        assert!(store.written.lock().unwrap().is_empty(), "nothing changed, nothing written");
        app.run_now(Msg::run("settings.set", json!({"id": SETTING, "value": false})), false).unwrap();
        save(&mut app);
        save(&mut app);
        assert_eq!(*store.written.lock().unwrap(), vec![json!(false)]);
    }
}
