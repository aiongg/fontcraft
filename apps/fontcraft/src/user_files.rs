//! The user's files in FontCraft's settings folder: the settings store and custom glyph data. [`UserFiles`] is the
//! window's `UserStore`: it reads them when the window opens and writes the settings whenever they change.
//!
//! | Platform | Folder |
//! |---|---|
//! | Linux and others | `$XDG_CONFIG_HOME/fontcraft`, else `~/.config/fontcraft` |
//! | macOS | `~/Library/Application Support/FontCraft` |
//! | Windows | `%APPDATA%\FontCraft` |
//!
//! `FONTCRAFT_SETTINGS_DIR` replaces the folder (a test or a second profile).
//!
//! In it: `settings.json` (the store of `settings.get` and `settings.set`), and `Info/` with the
//! user's `GlyphData*.xml` files and `Groups*.plist` files (extra Categories and Languages entries
//! for Font View's sidebar). A missing settings file means the defaults.
//! A settings file that cannot be read is reported, moved aside as `settings.json.bad` so that
//! saving does not overwrite it, and replaced by the defaults. Nothing here may stop the app.

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::settings::UserSettings;

/// The settings file's name in the settings folder.
const SETTINGS_FILE: &str = "settings.json";
/// Most files of one kind read from `Info/`.
const MAX_INFO_FILES: usize = fontcraft_engine::glyph_data::MAX_FILES;
/// Largest custom glyph data file read (the database's own limit per file).
const MAX_GLYPH_DATA_BYTES: u64 = 16 << 20;
/// Largest groups file read (the sidebar's own limit per file).
const MAX_GROUPS_BYTES: u64 = fontcraft_engine::glyph_data::MAX_GROUPS_FILE_BYTES as u64;

/// The settings folder, when the platform names one.
pub fn settings_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = env("FONTCRAFT_SETTINGS_DIR") {
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

/// What loading found, for saving later.
pub struct UserFiles {
    dir: Option<PathBuf>,
    /// The store's revision when it was last read or written.
    saved_revision: u64,
}

impl UserFiles {
    /// The store of the settings folder; nothing is read until the window asks.
    pub fn in_settings_folder() -> UserFiles {
        UserFiles { dir: settings_dir(), saved_revision: 0 }
    }

    /// Reads the settings and the user's glyph data into `session`. Returns what to save to and
    /// the problems found, one line each.
    fn load_from(session: &mut Session, dir: Option<PathBuf>) -> (UserFiles, Vec<String>) {
        let mut problems = Vec::new();
        let Some(folder) = dir.clone() else {
            problems.push("no settings folder (HOME is not set): settings are not kept".to_string());
            return (UserFiles { dir, saved_revision: session.settings().revision() }, problems);
        };
        load_settings(session, &folder.join(SETTINGS_FILE), &mut problems);
        let info = folder.join("Info");
        let glyph_data = info_files(&info, "GlyphData", ".xml", &mut problems);
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
        for path in info_files(&info, "Groups", ".plist", &mut problems) {
            match read_capped(&path, MAX_GROUPS_BYTES)
                .and_then(|b| String::from_utf8(b).map_err(|_| std::io::Error::other("the file is not UTF-8 text")))
            {
                Ok(text) => groups.push((file_name(&path), text)),
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        if !groups.is_empty() {
            let named: Vec<(&str, &str)> = groups.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            problems.extend(session.set_user_groups(&named).into_iter().map(|p| format!("{}: {p}", info.display())));
        }
        (UserFiles { dir, saved_revision: session.settings().revision() }, problems)
    }

    /// Writes the settings when they changed since the last read or write. A failure is
    /// returned as a message and tried again on the next change.
    pub fn save_if_changed(&mut self, settings: &UserSettings) -> Option<String> {
        if settings.revision() == self.saved_revision {
            return None;
        }
        self.saved_revision = settings.revision();
        let dir = self.dir.as_ref()?;
        let result = std::fs::create_dir_all(dir)
            .map_err(|e| e.to_string())
            .and_then(|()| fontcraft_engine::format::write_atomic(&dir.join(SETTINGS_FILE), &settings.to_json()).map_err(|e| e.to_string()));
        result.err().map(|e| format!("cannot save the settings in {}: {e}", dir.display()))
    }
}

fn load_settings(session: &mut Session, path: &Path, problems: &mut Vec<String>) {
    let bytes = match read_capped(path, fontcraft_engine::settings::MAX_FILE_BYTES as u64) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            problems.push(format!("{}: {e}; the default settings are used", path.display()));
            set_aside(path, problems);
            return;
        }
    };
    match UserSettings::from_json(&bytes) {
        Ok((settings, dropped)) => {
            problems.extend(dropped.into_iter().map(|p| format!("{}: {p}", path.display())));
            *session.settings_mut() = settings;
        }
        Err(e) => {
            problems.push(format!("{}: {e}; the default settings are used", path.display()));
            set_aside(path, problems);
        }
    }
}

/// Moves an unreadable settings file aside, so the next save does not replace it.
fn set_aside(path: &Path, problems: &mut Vec<String>) {
    let mut aside = path.as_os_str().to_owned();
    aside.push(".bad");
    let aside = PathBuf::from(aside);
    match std::fs::rename(path, &aside) {
        Ok(()) => problems.push(format!("the unreadable file was kept as {}", aside.display())),
        Err(e) => problems.push(format!("cannot move {} aside: {e}", path.display())),
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
        let (files, problems) = Self::load_from(session, self.dir.clone());
        *self = files;
        problems
    }

    fn save_if_changed(&mut self, session: &Session) -> Option<String> {
        UserFiles::save_if_changed(self, session.settings())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-user-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Info")).unwrap();
        dir
    }

    #[test]
    fn settings_and_glyph_data_load_and_settings_save_on_change() {
        let dir = folder("load");
        std::fs::write(dir.join("Info/GlyphData.xml"), br#"<glyphData><glyph name="zz" unicode="E100" category="Symbol"/></glyphData>"#).unwrap();
        std::fs::write(dir.join("Info/GlyphData-broken.xml"), b"<glyphData><glyph").unwrap();
        let mut s = Session::new();
        let (mut files, problems) = UserFiles::load_from(&mut s, Some(dir.clone()));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("GlyphData-broken.xml"), "{problems:?}");
        assert_eq!(s.glyph_data().unwrap().record("zz").map(|r| r.unicode), Some(Some(0xE100)));
        assert_eq!(files.save_if_changed(s.settings()), None);
        assert!(!dir.join(SETTINGS_FILE).exists(), "nothing changed, nothing written");
        s.execute("settings.set", &serde_json::json!({"id": "appearance.displayUnicodeValue", "value": true})).unwrap();
        assert_eq!(files.save_if_changed(s.settings()), None);
        let mut again = Session::new();
        let (_, problems) = UserFiles::load_from(&mut again, Some(dir.clone()));
        assert_eq!(problems.len(), 1);
        assert_eq!(again.settings().get_bool("appearance.displayUnicodeValue"), Some(true));
    }

    #[test]
    fn groups_files_load_and_a_broken_one_is_reported() {
        let dir = folder("groups");
        std::fs::write(dir.join("Info/Groups.plist"), "{languages = ({name = Mine; list = (a, \"\u{e4}\");});}").unwrap();
        std::fs::write(dir.join("Info/Groups-broken.plist"), "{languages = (").unwrap();
        let mut s = Session::new();
        let (_, problems) = UserFiles::load_from(&mut s, Some(dir.clone()));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("Groups-broken.plist"), "{problems:?}");
        let groups = s.user_groups().unwrap();
        assert_eq!(groups.languages.len(), 1);
        assert_eq!(groups.languages[0].name, "Mine");
    }

    #[test]
    fn a_corrupt_settings_file_gives_the_defaults_and_is_kept_aside() {
        let dir = folder("corrupt");
        std::fs::write(dir.join(SETTINGS_FILE), b"{\"version\": 1, \"settings\": [").unwrap();
        let mut s = Session::new();
        let (_, problems) = UserFiles::load_from(&mut s, Some(dir.clone()));
        assert!(problems.iter().any(|p| p.contains("the default settings are used")), "{problems:?}");
        assert!(dir.join("settings.json.bad").exists());
        assert!(!dir.join(SETTINGS_FILE).exists());
        assert_eq!(s.settings().revision(), Session::new().settings().revision());
    }
}
