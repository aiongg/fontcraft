//! The user's files in FontCraft's settings folder: the settings store, the window's layout and custom glyph data.
//! [`UserFiles`] is the window's `UserStore`: it reads them when the window opens and writes the settings and the layout
//! whenever they change.
//!
//! | Platform | Folder |
//! |---|---|
//! | Linux and others | `$XDG_CONFIG_HOME/fontcraft`, else `~/.config/fontcraft` |
//! | macOS | `~/Library/Application Support/FontCraft` |
//! | Windows | `%APPDATA%\FontCraft` |
//!
//! `FONTCRAFT_SETTINGS_DIR` replaces the folder (a test or a second profile). Set but empty, it keeps everything in
//! memory: nothing is read or written.
//!
//! In it: `config.toml` (the store of `settings.get` and `settings.set`, [`settings_file`](crate::settings_file)),
//! `ui.json` (the window's layout, [`layout_file`](crate::layout_file)), `window.json` (the window's size and place,
//! which `fontcraft_ui::run` keeps), and `Info/` with the user's `GlyphData*.xml` files and `Groups*.plist` files (extra
//! Categories and Languages entries for Font View's sidebar). A missing file means the defaults; one that cannot be read
//! is reported and kept. Nothing here may stop the app.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fontcraft_engine::Session;
use serde_json::Value;

use crate::layout_file::LayoutFile;
use crate::settings_file::SettingsFile;

/// Replaces the settings folder.
const DIR_VARIABLE: &str = "FONTCRAFT_SETTINGS_DIR";
/// Most files of one kind read from `Info/`.
const MAX_INFO_FILES: usize = fontcraft_engine::glyph_data::MAX_FILES;
/// Largest custom glyph data file read (the database's own limit per file).
const MAX_GLYPH_DATA_BYTES: u64 = 16 << 20;
/// Largest groups file read (the sidebar's own limit per file).
const MAX_GROUPS_BYTES: u64 = fontcraft_engine::glyph_data::MAX_GROUPS_FILE_BYTES as u64;

/// Whether the settings are kept in memory only, as an empty `FONTCRAFT_SETTINGS_DIR` asks.
fn in_memory() -> bool {
    std::env::var_os(DIR_VARIABLE).is_some_and(|v| v.is_empty())
}

/// The settings folder, when the platform names one and the settings are not kept in memory.
pub fn settings_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if in_memory() {
        return None;
    }
    if let Some(dir) = env(DIR_VARIABLE) {
        return Some(dir);
    }
    if cfg!(target_os = "macos") {
        return env("HOME").map(|h| h.join("Library").join("Application Support").join("FontCraft"));
    }
    if cfg!(windows) {
        return env("APPDATA").map(|d| d.join("FontCraft"));
    }
    env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config"))).map(|d| d.join("fontcraft"))
}

/// The settings folder's files, open once the window has asked for them.
pub struct UserFiles {
    dir: Option<PathBuf>,
    /// The session's settings store; `None` until it is read.
    settings: Option<SettingsFile>,
    /// The store's revision when it was last read or taken.
    saved_revision: u64,
    layout: LayoutFile,
}

impl UserFiles {
    /// The files of the settings folder; nothing is read until the window asks.
    pub fn in_settings_folder() -> UserFiles {
        Self::at(settings_dir())
    }

    fn at(dir: Option<PathBuf>) -> UserFiles {
        UserFiles { dir, settings: None, saved_revision: 0, layout: LayoutFile::default() }
    }

    /// Reads the settings and the user's glyph data into `session`; the problems found, one line each.
    fn load_into(&mut self, session: &mut Session) -> Vec<String> {
        let (file, settings, mut problems) = SettingsFile::open(self.dir.as_deref());
        self.settings = Some(file);
        *session.settings_mut() = settings;
        match &self.dir {
            Some(folder) => load_info(session, folder, &mut problems),
            None if in_memory() => {}
            None => problems.push("no settings folder (HOME is not set): settings are not kept".to_string()),
        }
        self.saved_revision = session.settings().revision();
        problems
    }
}

/// Reads the user's glyph data and groups files of `folder` into `session`.
fn load_info(session: &mut Session, folder: &Path, problems: &mut Vec<String>) {
    let info = folder.join("Info");
    let glyph_data = info_files(&info, "GlyphData", ".xml", problems);
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for path in glyph_data {
        match read_capped(&path, MAX_GLYPH_DATA_BYTES) {
            Ok(bytes) => files.push((file_name(&path), bytes)),
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }
    if !files.is_empty() {
        let named: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        problems.extend(session.set_user_glyph_data(&named).into_iter().map(|p| format!("{}: {p}", info.display())));
    }
    // After the glyph data: a character in a groups list is named through it.
    let mut groups: Vec<(String, String)> = Vec::new();
    for path in info_files(&info, "Groups", ".plist", problems) {
        match read_capped(&path, MAX_GROUPS_BYTES).and_then(|b| String::from_utf8(b).map_err(|_| std::io::Error::other("the file is not UTF-8 text")))
        {
            Ok(text) => groups.push((file_name(&path), text)),
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }
    if !groups.is_empty() {
        let named: Vec<(&str, &str)> = groups.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        problems.extend(session.set_user_groups(&named).into_iter().map(|p| format!("{}: {p}", info.display())));
    }
}

/// The files in `dir` named `<prefix>…<suffix>`, sorted by name, at most [`MAX_INFO_FILES`].
fn info_files(dir: &Path, prefix: &str, suffix: &str, problems: &mut Vec<String>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && file_name(p).starts_with(prefix) && file_name(p).ends_with(suffix))
        .collect();
    found.sort();
    if found.len() > MAX_INFO_FILES {
        problems.push(format!("{}: only the first {MAX_INFO_FILES} {prefix}{suffix} files are read", dir.display()));
        found.truncate(MAX_INFO_FILES);
    }
    found
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The file's bytes, refused when it is larger than `limit`.
fn read_capped(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let size = std::fs::metadata(path)?.len();
    if size > limit {
        return Err(std::io::Error::other(format!("{size} bytes is over the limit of {limit}")));
    }
    std::fs::read(path)
}

impl fontcraft_ui::app::user_store::UserStore for UserFiles {
    fn load(&mut self, session: &mut Session) -> Vec<String> {
        self.load_into(session)
    }

    fn load_layout(&mut self) -> (Option<Value>, Vec<String>) {
        let (file, layout, problems) = LayoutFile::open(self.dir.as_deref());
        self.layout = file;
        (layout, problems)
    }

    fn save_if_changed(&mut self, session: &Session) -> Option<String> {
        let settings = session.settings();
        if settings.revision() != self.saved_revision {
            self.saved_revision = settings.revision();
            self.settings.as_mut()?.take(settings);
        }
        None
    }

    fn keep_layout(&mut self, layout: Value) {
        self.layout.take(layout);
    }

    fn tick(&mut self) -> (Option<String>, Option<Duration>) {
        let (settings, settings_due) = self.settings.as_mut().map(SettingsFile::tick).unwrap_or_default();
        let (layout, layout_due) = self.layout.tick();
        (settings.or(layout), [settings_due, layout_due].into_iter().flatten().min())
    }

    fn flush(&mut self) -> Option<String> {
        let settings = self.settings.as_mut().and_then(SettingsFile::flush);
        let layout = self.layout.flush();
        settings.or(layout)
    }
}

impl Drop for UserFiles {
    fn drop(&mut self) {
        // The window writes everything as it closes; this is for an app that ends another way. Nobody is left to tell.
        let _ = fontcraft_ui::app::user_store::UserStore::flush(self);
    }
}

/// An empty settings folder for the test `tag`.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub fn test_folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-user-files-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Info")).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use fontcraft_ui::app::user_store::UserStore as _;

    use super::*;

    const CONFIG_FILE: &str = craft::settings::store::CONFIG_FILE;

    #[test]
    fn settings_and_glyph_data_load_and_settings_save_on_change() {
        let dir = test_folder("load");
        std::fs::write(dir.join("Info/GlyphData.xml"), br#"<glyphData><glyph name="zz" unicode="E100" category="Symbol"/></glyphData>"#).unwrap();
        std::fs::write(dir.join("Info/GlyphData-broken.xml"), b"<glyphData><glyph").unwrap();
        let mut s = Session::new();
        let mut files = UserFiles::at(Some(dir.clone()));
        let problems = files.load_into(&mut s);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("GlyphData-broken.xml"), "{problems:?}");
        assert_eq!(s.glyph_data().unwrap().record("zz").map(|r| r.unicode), Some(Some(0xE100)));
        assert_eq!((files.save_if_changed(&s), files.flush()), (None, None));
        assert!(!dir.join(CONFIG_FILE).exists(), "nothing changed, nothing written");
        s.execute("settings.set", &serde_json::json!({"id": "appearance.displayUnicodeValue", "value": true})).unwrap();
        assert_eq!(files.save_if_changed(&s), None);
        let (problem, wait) = files.tick();
        assert!(problem.is_none() && wait.is_some(), "written once the settings rest");
        assert_eq!(files.flush(), None);
        let mut again = Session::new();
        assert_eq!(UserFiles::at(Some(dir)).load_into(&mut again).len(), 1);
        assert_eq!(again.settings().get_bool("appearance.displayUnicodeValue"), Some(true));
    }

    #[test]
    fn groups_files_load_and_a_broken_one_is_reported() {
        let dir = test_folder("groups");
        std::fs::write(dir.join("Info/Groups.plist"), "{languages = ({name = Mine; list = (a, \"\u{e4}\");});}").unwrap();
        std::fs::write(dir.join("Info/Groups-broken.plist"), "{languages = (").unwrap();
        let mut s = Session::new();
        let problems = UserFiles::at(Some(dir)).load_into(&mut s);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("Groups-broken.plist"), "{problems:?}");
        let groups = s.user_groups().unwrap();
        assert_eq!(groups.languages.len(), 1);
        assert_eq!(groups.languages[0].name, "Mine");
    }
}
