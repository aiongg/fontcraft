//! The settings store in `config.toml` of the settings folder, through the suite's settings library.
//!
//! The file starts with `schema_version` and holds the store of `settings.get` and `settings.set` in one `[settings]`
//! table, a key per setting id. A value TOML cannot hold as it is (one with a null in it) is written as
//! `{ json = "…" }`. The library writes it atomically, keeps the previous version as `config.toml.bak`, and leaves
//! comments and keys of other versions in place. A file that is not TOML is kept as `config.broken-<time>.toml` and the
//! backup takes its place; without a usable backup the defaults do.
//!
//! The first start that finds no `config.toml` imports `settings.json`, the settings file of earlier versions, which is
//! left as it is.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use craft::settings::store::CONFIG_FILE;
use craft::settings::{
    DiskFiles, LegacyFile, LoadReport, Location, OpenOptions, SaveStatus, SectionError, Settings, SettingsFiles, SettingsStore, Source, document,
    recovery,
};
use fontcraft_engine::settings::{MAX_DEPTH, MAX_FILE_BYTES, UserSettings};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The settings file of earlier versions.
pub const LEGACY_FILE: &str = "settings.json";
/// Largest `config.toml` read; its text can be longer than the same settings as JSON.
const MAX_CONFIG_BYTES: u64 = 2 * MAX_FILE_BYTES as u64;
/// The key of a value written as JSON text.
const JSON_KEY: &str = "json";
/// What [`CappedFiles`] reads where the file is not UTF-8: not TOML either, so the store moves the file aside.
const NOT_TEXT: &str = "this file is not UTF-8 text";

/// What `config.toml` holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    settings: toml::Table,
}

impl Settings for Config {
    const APP_ID: &'static str = "fontcraft";
    const SCHEMA_VERSION: u32 = 1;
}

/// The file system, with a limit on what is read, and so on what is written.
struct CappedFiles;

impl SettingsFiles for CappedFiles {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        if let Ok(meta) = std::fs::metadata(path)
            && meta.len() > MAX_CONFIG_BYTES
        {
            return Err(io::Error::other(format!("{} bytes is over the limit of {MAX_CONFIG_BYTES}", meta.len())));
        }
        match DiskFiles.read(path) {
            Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(Some(NOT_TEXT.to_string())),
            read => read,
        }
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        if text.len() as u64 > MAX_CONFIG_BYTES {
            return Err(io::Error::other(format!("the settings take {} bytes, over the limit of {MAX_CONFIG_BYTES}", text.len())));
        }
        DiskFiles.write(path, text)
    }

    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        DiskFiles.move_aside(path)
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        DiskFiles.write_backup(path, text)
    }

    fn create_dir(&self, dir: &Path) -> io::Result<()> {
        DiskFiles.create_dir(dir)
    }

    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        DiskFiles.check_writable(dir)
    }
}

/// The open settings file.
pub struct SettingsFile {
    store: SettingsStore<Config>,
    /// The session's settings as they were last put in the store. Only what differs from them is put there again, so
    /// another window's changes to other settings stay.
    taken: toml::Table,
}

impl SettingsFile {
    /// Opens the settings in `dir`, or in memory without one: the store, the settings and the problems found, one line
    /// each.
    pub fn open(dir: Option<&Path>) -> (SettingsFile, UserSettings, Vec<String>) {
        let mut problems = Vec::new();
        let store = match dir {
            Some(dir) => open_in(dir, &mut problems),
            None => SettingsStore::open(Location::Memory).0,
        };
        let mut entries = Map::new();
        for (id, value) in &store.get().settings {
            match from_toml(value, MAX_DEPTH + 1) {
                Some(value) => {
                    entries.insert(id.clone(), value);
                }
                None => problems.push(format!("{CONFIG_FILE}: the value of `{id}` cannot be read; the default is used")),
            }
        }
        let settings = match UserSettings::from_entries(&entries) {
            Ok((settings, dropped)) => {
                problems.extend(dropped.into_iter().map(|p| format!("{CONFIG_FILE}: {p}")));
                settings
            }
            Err(e) => {
                problems.push(format!("{CONFIG_FILE}: {e}; the default settings are used"));
                UserSettings::new()
            }
        };
        let taken = table(&settings);
        (SettingsFile { store, taken }, settings, problems)
    }

    /// Takes the settings that changed since the last call; [`tick`](Self::tick) writes them.
    pub fn take(&mut self, settings: &UserSettings) {
        let now = table(settings);
        let before = std::mem::replace(&mut self.taken, now);
        let now = &self.taken;
        self.store.update(|config| {
            for id in before.keys().filter(|id| !now.contains_key(id.as_str())) {
                config.settings.remove(id);
            }
            for (id, value) in now.iter().filter(|(id, value)| before.get(id.as_str()) != Some(*value)) {
                config.settings.insert(id.clone(), value.clone());
            }
        });
    }

    /// Writes what has been unchanged long enough; a failure as a message, and the time until there is more to write.
    pub fn tick(&mut self) -> (Option<String>, Option<Duration>) {
        let now = Instant::now();
        let tick = self.store.tick(now);
        let failed = match tick.status {
            SaveStatus::Failed(e) => Some(e.to_string()),
            SaveStatus::Idle | SaveStatus::Pending | SaveStatus::Saved => None,
        };
        (failed.or_else(|| self.moved_aside()), tick.next_deadline.map(|due| due.saturating_duration_since(now)))
    }

    /// Writes what is not written yet; a failure as a message.
    pub fn flush(&mut self) -> Option<String> {
        if !self.store.is_dirty() {
            return None;
        }
        let failed = self.store.flush().err().map(|e| e.to_string());
        failed.or_else(|| self.moved_aside())
    }

    /// The file another program damaged while the app ran.
    fn moved_aside(&mut self) -> Option<String> {
        let moved = self.store.take_moved_aside().pop()?;
        Some(format!("{CONFIG_FILE} was not a settings file any more; it was kept as {}", moved.display()))
    }
}

/// Opens the store in `dir`, recovering from a damaged file, with what went wrong in `problems`.
fn open_in(dir: &Path, problems: &mut Vec<String>) -> SettingsStore<Config> {
    let legacy = dir.join(LEGACY_FILE);
    let legacy_size = std::fs::metadata(&legacy).map(|m| m.len()).ok();
    let importable = legacy_size.is_some_and(|size| size <= MAX_FILE_BYTES as u64);
    let open = || {
        let files = if importable { vec![LegacyFile { path: legacy.clone(), parse: import }] } else { Vec::new() };
        let options = OpenOptions::new(Location::Dir(dir.to_path_buf())).legacy(files).files(Arc::new(CappedFiles));
        SettingsStore::<Config>::open_with(options)
    };
    let (mut store, mut report) = open();
    if let Some(moved) = report.broken_moved_to.take() {
        let path = dir.join(CONFIG_FILE);
        problems.push(format!("{}: not a settings file; it was kept as {}", path.display(), moved.display()));
        let restored = restore_backup(&path);
        (store, report) = open();
        problems.extend(restored.as_ref().err().cloned());
        problems.push(match (&restored, &report.source) {
            (Ok(backup), _) => format!("the settings of {} are used", backup.display()),
            (Err(_), Source::Legacy(from)) => format!("the settings of {} are used", from.display()),
            (Err(_), _) => "the default settings are used".to_string(),
        });
    }
    describe(dir, &report, problems);
    match &report.source {
        // Written at once, so later starts read `config.toml`.
        Source::Legacy(_) => {
            if let Err(e) = store.flush() {
                problems.push(e.to_string());
            }
        }
        Source::Defaults if legacy_size.is_some() => problems.push(match read_legacy(&legacy) {
            Err(why) => format!("{}: {why}; it was not imported and stays as it is", legacy.display()),
            Ok(_) => format!("{}: it was not imported and stays as it is", legacy.display()),
        }),
        Source::Defaults | Source::File => {}
    }
    store
}

/// Adds what the report says went wrong.
fn describe(dir: &Path, report: &LoadReport, problems: &mut Vec<String>) {
    problems.extend(report.section_errors.iter().map(|e| format!("{}: {}: {}; the default is used", dir.display(), e.section, e.message)));
    if let Some(why) = &report.read_only {
        problems.push(format!("the settings in {} are not saved: {why}", dir.display()));
    }
}

/// Puts the backup of the settings file at `path` in its place. The backup's path, or why it was not used.
fn restore_backup(path: &Path) -> Result<PathBuf, String> {
    let backup = recovery::backup_path(path);
    let refused = |why: &str| format!("{}: {why}", backup.display());
    let text = CappedFiles.read(&backup).map_err(|e| refused(&e.to_string()))?.ok_or_else(|| refused("there is no backup"))?;
    if text.trim().is_empty() || document::parse(&text).is_err() {
        return Err(refused("not a settings file"));
    }
    CappedFiles.write(path, &text).map_err(|e| refused(&e.to_string()))?;
    Ok(backup)
}

/// The settings of `settings.json`.
fn import(bytes: &[u8]) -> Result<(Config, Vec<SectionError>), String> {
    let (settings, dropped) = UserSettings::from_json(bytes).map_err(|e| e.to_string())?;
    let dropped = dropped.into_iter().map(|message| SectionError { section: LEGACY_FILE.to_string(), message }).collect();
    Ok((Config { settings: table(&settings) }, dropped))
}

/// Why the `settings.json` at `path` cannot be imported, if it cannot.
fn read_legacy(path: &Path) -> Result<Config, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE_BYTES as u64 {
        return Err(format!("{size} bytes is over the limit of {MAX_FILE_BYTES}"));
    }
    import(&std::fs::read(path).map_err(|e| e.to_string())?).map(|(config, _)| config)
}

/// The `[settings]` table of `settings`.
fn table(settings: &UserSettings) -> toml::Table {
    settings.stored().iter().map(|(id, value)| (id.clone(), to_toml(value))).collect()
}

/// `value` in TOML, as JSON text where TOML cannot hold it as it is.
fn to_toml(value: &Value) -> toml::Value {
    exact(value).unwrap_or_else(|| {
        let text = toml::Value::String(value.to_string());
        toml::Value::Table([(JSON_KEY.to_string(), text)].into_iter().collect())
    })
}

/// The TOML value that reads back as `value`; `None` when there is none (a null, an integer past 64 bits signed, an
/// object that looks like JSON text).
fn exact(value: &Value) -> Option<toml::Value> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None if n.is_f64() => toml::Value::Float(n.as_f64()?),
            None => return None,
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(items) => toml::Value::Array(items.iter().map(exact).collect::<Option<_>>()?),
        Value::Object(fields) if fields.len() == 1 && fields.contains_key(JSON_KEY) => return None,
        Value::Object(fields) => {
            toml::Value::Table(fields.iter().map(|(key, value)| Some((key.clone(), exact(value)?))).collect::<Option<toml::Table>>()?)
        }
    })
}

/// The JSON value of `value`; `None` when it has none or nests deeper than `depth`.
fn from_toml(value: &toml::Value, depth: usize) -> Option<Value> {
    let depth = depth.checked_sub(1)?;
    Some(match value {
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(x) => Value::Number(serde_json::Number::from_f64(*x)?),
        toml::Value::String(s) => Value::String(s.clone()),
        // No setting is a date; a hand edit can leave the quotes off.
        toml::Value::Datetime(date) => Value::String(date.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(|item| from_toml(item, depth)).collect::<Option<_>>()?),
        toml::Value::Table(fields) => match fields.get(JSON_KEY) {
            Some(toml::Value::String(text)) if fields.len() == 1 => serde_json::from_str(text).ok()?,
            _ => Value::Object(fields.iter().map(|(key, value)| Some((key.clone(), from_toml(value, depth)?))).collect::<Option<_>>()?),
        },
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::user_files::test_folder;

    const ID: &str = "appearance.displayUnicodeValue";

    /// The settings of `dir` after `change`, opened again.
    fn reopened(dir: &Path, change: impl FnOnce(&mut UserSettings)) -> UserSettings {
        let (mut file, mut settings, problems) = SettingsFile::open(Some(dir));
        assert_eq!(problems, Vec::<String>::new());
        change(&mut settings);
        file.take(&settings);
        assert_eq!(file.flush(), None);
        SettingsFile::open(Some(dir)).1
    }

    #[test]
    fn values_of_every_shape_survive_config_toml() {
        for text in [
            "null",
            "[1, 1.0, -0.5, 9223372036854775807]",
            "18446744073709551615",
            r#"{"a": {"b.c": [true, "x\ny", []]}, "d": {}}"#,
            r#"[{"name": "n", "kept": null}]"#,
            r#"{"json": "{}"}"#,
            r#"{"json": {"json": "1"}}"#,
        ] {
            let value: Value = serde_json::from_str(text).unwrap();
            let written = toml::to_string(&toml::Table::from_iter([("id".to_string(), to_toml(&value))])).unwrap();
            let read: toml::Table = toml::from_str(&written).unwrap();
            assert_eq!(from_toml(&read["id"], MAX_DEPTH + 1), Some(value), "{written}");
        }
        assert_eq!(from_toml(&toml::Value::Float(f64::NAN), 1), None);
    }

    #[test]
    fn settings_json_is_imported_once_and_stays() {
        let dir = test_folder("import");
        let legacy = dir.join(LEGACY_FILE);
        let old = br#"{"version": 1, "settings": {"appearance.displayUnicodeValue": true, "from.a.newer.version": [null]}}"#;
        std::fs::write(&legacy, old).unwrap();
        let (_, settings, problems) = SettingsFile::open(Some(&dir));
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(settings.get_bool(ID), Some(true));
        let text = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(text.contains("[settings]") && text.contains("\"appearance.displayUnicodeValue\" = true"), "{text}");

        let settings = reopened(&dir, |s| s.set(ID, Value::Null).unwrap());
        assert_eq!(settings.get_bool(ID), Some(false), "a removed setting does not come back from settings.json");
        assert_eq!(settings.stored()["from.a.newer.version"], serde_json::json!([null]));
        assert_eq!(std::fs::read(&legacy).unwrap(), old, "the old file stays as it is");
    }

    #[test]
    fn an_unreadable_settings_json_is_reported_and_stays() {
        let dir = test_folder("bad-import");
        std::fs::write(dir.join(LEGACY_FILE), b"{\"version\": 1, \"settings\": [").unwrap();
        let (_, settings, problems) = SettingsFile::open(Some(&dir));
        assert!(problems.iter().any(|p| p.contains("not imported")), "{problems:?}");
        assert_eq!(settings, UserSettings::new());
        assert!(dir.join(LEGACY_FILE).exists());
    }

    #[test]
    fn a_config_too_large_to_read_back_is_not_written() {
        let dir = std::env::temp_dir().join(format!("fontcraft-settings-large-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        CappedFiles.write(&path, "kept = 1\n").unwrap();
        assert!(CappedFiles.write(&path, &"#".repeat(MAX_CONFIG_BYTES as usize + 1)).is_err());
        assert_eq!(CappedFiles.read(&path).unwrap().as_deref(), Some("kept = 1\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_config_is_kept_aside_and_the_backup_is_used() {
        let dir = test_folder("damaged");
        // The second save makes the first one the backup.
        reopened(&dir, |s| s.set(ID, Value::Bool(true)).unwrap());
        reopened(&dir, |s| s.set("editing.leverRatio", Value::from(7)).unwrap());
        for bytes in [b"schema_version = [".as_slice(), b"\xff\xfe".as_slice()] {
            std::fs::write(dir.join(CONFIG_FILE), bytes).unwrap();
            let (_, settings, problems) = SettingsFile::open(Some(&dir));
            assert!(problems.iter().any(|p| p.contains("config.broken-")), "{problems:?}");
            assert_eq!(settings.get_bool(ID), Some(true), "{problems:?}");
        }
        std::fs::write(dir.join(CONFIG_FILE), b"]").unwrap();
        std::fs::write(dir.join("config.toml.bak"), b"]").unwrap();
        let (_, settings, problems) = SettingsFile::open(Some(&dir));
        assert!(problems.iter().any(|p| p.contains("the default settings are used")), "{problems:?}");
        assert_eq!(settings, UserSettings::new());
    }

    #[test]
    fn comments_unknown_keys_and_another_windows_settings_survive_a_save() {
        let dir = test_folder("merge");
        std::fs::write(dir.join(CONFIG_FILE), "# mine\nschema_version = 1\n\n[other]\nkept = 1\n\n[settings]\n\"editing.powerNudge\" = true\n")
            .unwrap();
        let (mut one, mut settings, _) = SettingsFile::open(Some(&dir));
        reopened(&dir, |s| s.set("editing.leverRatio", Value::from(7)).unwrap());
        settings.set(ID, Value::Bool(true)).unwrap();
        one.take(&settings);
        assert_eq!(one.flush(), None);
        let text = std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(text.contains("# mine") && text.contains("kept = 1"), "{text}");
        let (_, settings, _) = SettingsFile::open(Some(&dir));
        assert_eq!(settings.get("editing.leverRatio").unwrap().as_f64(), Some(7.0));
        assert_eq!((settings.get_bool(ID), settings.get_bool("editing.powerNudge")), (Some(true), Some(true)));
    }
}
