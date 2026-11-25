//! The user's settings and the window's layout kept between runs.
//!
//! The settings live in the session's settings store (`settings.get`, `settings.set`, the Settings window, every
//! switch that is a setting). The desktop app supplies a [`UserStore`] that reads them, with the user's custom glyph
//! data, from the settings folder when the window opens ([`load`]), and writes them back whenever they change
//! ([`save`], once per frame, and [`flush`] as the window closes). The UI crate reads and writes no files itself;
//! without a store nothing is kept. View state kept between runs lives in the settings too (the reporter switches,
//! `view.shownReporters`); [`load`] puts it back in the view. The dock's layout is kept beside the settings
//! (`shell::kept_layout`).

use std::time::Duration;

use craft::behavior::dock::Workspace;
use fontcraft_engine::Session;
use serde_json::Value;

use super::FontCraftApp;
use crate::shell::dock::PanelId;
use crate::shell::kept_layout::{self, ClosedPalette};

/// Where the desktop app keeps the user's settings.
pub trait UserStore: Send {
    /// Reads the stored settings (and what else the settings folder holds for the session) into `session`; the problems
    /// found, one line each. Nothing here may stop the app: a file that cannot be read leaves the defaults.
    fn load(&mut self, session: &mut Session) -> Vec<String>;

    /// The window layout given to [`keep_layout`](Self::keep_layout) last, and the problems found reading it.
    fn load_layout(&mut self) -> (Option<Value>, Vec<String>) {
        (None, Vec::new())
    }

    /// Takes the settings when they changed since the last read or call. [`tick`](Self::tick) writes them.
    fn save_if_changed(&mut self, session: &Session) -> Option<String>;

    /// Takes the window's layout, which changed. [`tick`](Self::tick) writes it.
    fn keep_layout(&mut self, layout: Value) {
        let _ = layout;
    }

    /// Writes what has been unchanged long enough; a failure as a message, and the time until it has more to write.
    fn tick(&mut self) -> (Option<String>, Option<Duration>) {
        (None, None)
    }

    /// Writes everything taken and not yet written; a failure as a message.
    fn flush(&mut self) -> Option<String> {
        None
    }
}

/// The dock and the closed Palette as the store has them.
pub type KeptLayout = (Workspace<PanelId>, ClosedPalette);

/// Reads the store into the session and the window, before the window first asks a setting (the Start Window's Show at
/// Launch). The last problem goes to the status line, every one to the log.
pub fn load(app: &mut FontCraftApp) {
    let Some(store) = app.user_store.as_mut() else { return };
    let mut problems = store.load(&mut app.session);
    let (layout, unread) = store.load_layout();
    problems.extend(unread);
    match layout.as_ref().map(kept_layout::restore) {
        Some(Ok((workspace, palette))) => {
            app.view.workspace.replace(workspace);
            app.view.palette.hidden = palette;
        }
        Some(Err(problem)) => problems.push(problem),
        None => {}
    }
    app.kept_layout = Some((app.view.workspace.clone(), app.view.palette.hidden.clone()));
    for problem in &problems {
        log::warn!("{problem}");
    }
    if let Some(last) = problems.last() {
        app.status = last.clone();
    }
    // View state kept in the settings.
    app.view.extensions.reporters = crate::shell::extensions::stored_reporters(&app.session);
    // Loaded settings may carry the same revision as the defaults: the projection reads them again.
    app.catalog.reread_shortcuts();
    app.view_changed();
    app.project();
}

/// Hands the store the settings and the layout when they changed, and lets it write; a failure goes to the status line
/// and is tried again on the next change. The time until the store has more to write.
pub fn save(app: &mut FontCraftApp) -> Option<Duration> {
    take_changes(app)?;
    let (problem, again) = app.user_store.as_mut()?.tick();
    report(app, problem);
    again
}

/// Writes what the store has not written yet, as the window closes.
pub fn flush(app: &mut FontCraftApp) {
    if take_changes(app).is_none() {
        return;
    }
    let problem = app.user_store.as_mut().and_then(|store| store.flush());
    report(app, problem);
}

/// Hands the store the settings and the layout when they changed; `None` without a store.
fn take_changes(app: &mut FontCraftApp) -> Option<()> {
    let store = app.user_store.as_mut()?;
    let view = &app.view;
    if let Some((workspace, palette)) = &mut app.kept_layout
        && (*workspace != view.workspace || *palette != view.palette.hidden)
    {
        workspace.clone_from(&view.workspace);
        palette.clone_from(&view.palette.hidden);
        if let Some(layout) = kept_layout::to_value(workspace, palette) {
            store.keep_layout(layout);
        }
    }
    let problem = store.save_if_changed(&app.session);
    report(app, problem);
    Some(())
}

fn report(app: &mut FontCraftApp, problem: Option<String>) {
    if let Some(problem) = problem {
        log::error!("{problem}");
        app.status = problem;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use serde_json::{Value, json};

    use super::*;
    use crate::app::{APP, Msg};

    const SETTING: &str = "appearance.displayUnicodeValue";

    /// A store holding one value of [`SETTING`] and a layout, recording what it is given.
    #[derive(Clone, Default)]
    struct Memory {
        written: Arc<Mutex<Vec<Value>>>,
        layout: Arc<Mutex<Option<Value>>>,
        revision: u64,
    }

    impl UserStore for Memory {
        fn load(&mut self, session: &mut Session) -> Vec<String> {
            session.execute("settings.set", &json!({"id": SETTING, "value": true})).unwrap();
            self.revision = session.settings().revision();
            vec!["a stored file was left out".into()]
        }

        fn load_layout(&mut self) -> (Option<Value>, Vec<String>) {
            (self.layout.lock().unwrap().clone(), Vec::new())
        }

        fn keep_layout(&mut self, layout: Value) {
            *self.layout.lock().unwrap() = Some(layout);
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
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
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

    /// The layout is handed to the store when it changes, and the next window opens with it.
    #[test]
    fn the_layout_is_handed_to_the_store_and_comes_back() {
        let open = |store: &Memory| {
            let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
            app.user_store = Some(Box::new(store.clone()));
            load(&mut app);
            app
        };
        let store = Memory::default();
        let mut app = open(&store);
        save(&mut app);
        assert_eq!(*store.layout.lock().unwrap(), None, "nothing changed, nothing kept");
        app.fire(CommandRef::new("window.palette"));
        crate::control::drain(&mut app);
        flush(&mut app);
        assert!(store.layout.lock().unwrap().is_some());
        assert!(!crate::shell::window::palette_shows(&open(&store)), "the Palette stays hidden");
    }
}
