//! The user's settings: setting ids to JSON values, kept for every document and every session
//! (`settings.get`, `settings.set`; `docs/plans/M4.md` § 3, the owner's decision on where filters
//! and recent documents live).
//!
//! Two kinds of id:
//! - **Live settings** ([`SETTINGS`]): ids of `[[setting]]` rows that FontCraft implements, each
//!   with a kind that `settings.set` checks and a default that `settings.get` returns while
//!   nothing is stored.
//! - **Reserved ids** ([`Reserved`]): structured data that belongs to other commands (filters,
//!   visible scripts, recent documents, the Start Window's picks, sample strings).
//!   `settings.get` reads them; only their own commands write them, through
//!   [`UserSettings::set_reserved`].
//!
//! The store is untrusted input twice over: from the settings file and from command parameters.
//! Every id, value and file is checked against the limits below; a corrupt file loads as the
//! defaults with a problem report ([`UserSettings::load`]), never a crash.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

/// Most ids one store holds.
pub const MAX_ENTRIES: usize = 1024;
/// Longest setting id, in bytes.
pub const MAX_ID_BYTES: usize = 128;
/// Largest value, as compact JSON.
pub const MAX_VALUE_BYTES: usize = 1 << 20;
/// Largest settings file.
pub const MAX_FILE_BYTES: usize = 8 << 20;
/// Deepest nesting of arrays and objects in one value.
pub const MAX_DEPTH: usize = 64;
/// Longest value of a text setting, in bytes.
pub const MAX_TEXT_BYTES: usize = 4096;
/// The settings file's format version.
pub const FILE_VERSION: u64 = 1;
/// Most documents kept in [`Reserved::RecentDocuments`].
pub const MAX_RECENT_DOCUMENTS: usize = 10;
/// Longest path kept in the recent documents, in bytes.
pub const MAX_RECENT_PATH_BYTES: usize = 4096;

/// The colour settings of Settings › Appearance start with this; each item has a `.light` and a `.dark` id.
pub const COLOR_PREFIX: &str = "appearance.color.";

/// The ids of the colour settings of Settings › Appearance, both sets.
pub fn color_ids() -> impl Iterator<Item = &'static str> {
    SETTINGS.iter().filter(|d| d.kind == Kind::Color && d.id.starts_with(COLOR_PREFIX)).map(|d| d.id)
}

/// What values a live setting takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bool,
    /// One of these strings.
    Choice(&'static [&'static str]),
    /// A finite number in this range.
    Number {
        min: f64,
        max: f64,
    },
    /// A string of at most [`MAX_TEXT_BYTES`].
    Text,
    /// A colour as `#rrggbb` or `#rrggbbaa` (hexadecimal, alpha last).
    Color,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Bool => "true or false",
            Kind::Choice(_) => "one of the listed choices",
            Kind::Number { .. } => "a number",
            Kind::Text => "text",
            Kind::Color => "a colour as #rrggbb or #rrggbbaa",
        }
    }
}

/// A live setting's default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DefaultValue {
    Bool(bool),
    Text(&'static str),
    Number(f64),
}

impl DefaultValue {
    fn value(self) -> Value {
        match self {
            DefaultValue::Bool(b) => json!(b),
            DefaultValue::Text(s) => json!(s),
            DefaultValue::Number(n) => json!(n),
        }
    }
}

/// A live setting: an id from a `[[setting]]` row of `spec/inventory`, its kind and default.
#[derive(Clone, Copy, Debug)]
pub struct SettingDef {
    pub id: &'static str,
    pub kind: Kind,
    pub default: DefaultValue,
}

/// The settings FontCraft implements. Add a row here when its `[[setting]]` row's behaviour
/// lands; the test `live_settings_have_rows` checks that the row exists.
pub const SETTINGS: &[SettingDef] = &[
    SettingDef { id: "addons.alternatePluginRepos", kind: Kind::Text, default: DefaultValue::Text("") },
    SettingDef { id: "addons.useSystemConsoleForScriptOutput", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "appearance.alwaysLightModeEditView", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "appearance.alwaysLightModeFontView", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "appearance.color.alignmentZones.dark", kind: Kind::Color, default: DefaultValue::Text("#9ab8dc38") },
    SettingDef { id: "appearance.color.alignmentZones.light", kind: Kind::Color, default: DefaultValue::Text("#9ab8dc38") },
    SettingDef { id: "appearance.color.backgroundStroke.dark", kind: Kind::Color, default: DefaultValue::Text("#e0404090") },
    SettingDef { id: "appearance.color.backgroundStroke.light", kind: Kind::Color, default: DefaultValue::Text("#e0404090") },
    SettingDef { id: "appearance.color.canvas.dark", kind: Kind::Color, default: DefaultValue::Text("#1e1e1e") },
    SettingDef { id: "appearance.color.canvas.light", kind: Kind::Color, default: DefaultValue::Text("#f4f4f4") },
    SettingDef { id: "appearance.color.cornerNodes.dark", kind: Kind::Color, default: DefaultValue::Text("#2d6cdf") },
    SettingDef { id: "appearance.color.cornerNodes.light", kind: Kind::Color, default: DefaultValue::Text("#2d6cdf") },
    SettingDef { id: "appearance.color.foregroundStroke.dark", kind: Kind::Color, default: DefaultValue::Text("#e6e6e6") },
    SettingDef { id: "appearance.color.foregroundStroke.light", kind: Kind::Color, default: DefaultValue::Text("#202020") },
    SettingDef { id: "appearance.color.kerningGlyphText.dark", kind: Kind::Color, default: DefaultValue::Text("#e0b445") },
    SettingDef { id: "appearance.color.kerningGlyphText.light", kind: Kind::Color, default: DefaultValue::Text("#8f6500") },
    SettingDef { id: "appearance.color.kerningGroupText.dark", kind: Kind::Color, default: DefaultValue::Text("#6ea8ff") },
    SettingDef { id: "appearance.color.kerningGroupText.light", kind: Kind::Color, default: DefaultValue::Text("#1c5fd0") },
    SettingDef { id: "appearance.color.kerningNegative.dark", kind: Kind::Color, default: DefaultValue::Text("#8cc8f590") },
    SettingDef { id: "appearance.color.kerningNegative.light", kind: Kind::Color, default: DefaultValue::Text("#8cc8f590") },
    SettingDef { id: "appearance.color.kerningPositive.dark", kind: Kind::Color, default: DefaultValue::Text("#f5dc46a0") },
    SettingDef { id: "appearance.color.kerningPositive.light", kind: Kind::Color, default: DefaultValue::Text("#f5dc46a0") },
    SettingDef { id: "appearance.color.otherLayersStroke.dark", kind: Kind::Color, default: DefaultValue::Text("#2d8cebb0") },
    SettingDef { id: "appearance.color.otherLayersStroke.light", kind: Kind::Color, default: DefaultValue::Text("#2d8cebb0") },
    SettingDef { id: "appearance.color.smoothNodes.dark", kind: Kind::Color, default: DefaultValue::Text("#2fa84f") },
    SettingDef { id: "appearance.color.smoothNodes.light", kind: Kind::Color, default: DefaultValue::Text("#2fa84f") },
    SettingDef { id: "appearance.displayUnicodeValue", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "appearance.guideOpacity", kind: Kind::Number { min: 0.0, max: 100.0 }, default: DefaultValue::Number(100.0) },
    SettingDef { id: "appearance.handleSize", kind: Kind::Choice(&["small", "regular", "large"]), default: DefaultValue::Text("regular") },
    SettingDef { id: "appearance.textViewWidth", kind: Kind::Number { min: 1.0, max: 100_000.0 }, default: DefaultValue::Number(10_000.0) },
    SettingDef { id: "editing.leverRatio", kind: Kind::Number { min: 1.0, max: 100.0 }, default: DefaultValue::Number(6.0) },
    SettingDef { id: "editing.powerNudge", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "export.ttfAutohintBinary", kind: Kind::Text, default: DefaultValue::Text("") },
    SettingDef { id: "sharing.enableExternalPreview", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "start.showAtLaunch", kind: Kind::Bool, default: DefaultValue::Bool(true) },
    SettingDef { id: "user.disableAutomaticAlignmentForImportedFiles", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "user.crashReportEmail", kind: Kind::Text, default: DefaultValue::Text("") },
    SettingDef { id: "user.crashReportName", kind: Kind::Text, default: DefaultValue::Text("") },
    SettingDef { id: "user.disableLocalization", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "user.keepGlyphNamesFromImportedFiles", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "user.sendCrashReports", kind: Kind::Bool, default: DefaultValue::Bool(false) },
    SettingDef { id: "user.useVersions", kind: Kind::Bool, default: DefaultValue::Bool(false) },
];

/// Ids that hold other commands' data. `settings.set` refuses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reserved {
    /// The Filters section of Font View's sidebar: smart filters, list filters and folders.
    Filters,
    /// The scripts the Languages section of the sidebar shows.
    VisibleScripts,
    /// Recently opened or saved documents, newest first.
    RecentDocuments,
    /// The Start Window's last choice of scripts and glyph sets.
    ChooserPicks,
    /// The groups of sample strings Edit View offers, `[{name, strings}]`.
    SampleStrings,
    /// The user's audit profiles and the application's default audit profile,
    /// `{default?, profiles: [profile]}` (`audit.profile.*`).
    AuditProfiles,
    /// The user's export profiles, `[{name, params}]` (`file.saveExportProfile`).
    ExportProfiles,
    /// The devices trusted to see External Preview, `[IP address]`, oldest first
    /// (`sharing.answerDevice`, `settings.resetTrustedDevices`).
    TrustedDevices,
    /// The reporter modules Edit View draws, `[module id]`, in the order they were switched on
    /// (`view.showReporter`).
    ShownReporters,
    /// The Export dialog's settings of each saved document's last successful export,
    /// `[{path, otf?, variable?}]`, newest first (`file.exportSettings`).
    ExportSettings,
}

impl Reserved {
    pub const ALL: [Reserved; 10] = [
        Reserved::Filters,
        Reserved::VisibleScripts,
        Reserved::RecentDocuments,
        Reserved::ChooserPicks,
        Reserved::SampleStrings,
        Reserved::AuditProfiles,
        Reserved::ExportProfiles,
        Reserved::TrustedDevices,
        Reserved::ShownReporters,
        Reserved::ExportSettings,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Reserved::Filters => "fontView.filters",
            Reserved::VisibleScripts => "fontView.visibleScripts",
            Reserved::RecentDocuments => "file.recentDocuments",
            Reserved::ChooserPicks => "start.chooserPicks",
            Reserved::SampleStrings => "sampleStrings.groups",
            Reserved::AuditProfiles => "audit.userProfiles",
            Reserved::ExportProfiles => fontcraft_model::fc_keys::EXPORT_PROFILES,
            Reserved::TrustedDevices => "sharing.trustedDevices",
            Reserved::ShownReporters => "view.shownReporters",
            Reserved::ExportSettings => "export.keptSettings",
        }
    }

    /// What writes it, for the error `settings.set` gives.
    fn owner(self) -> &'static str {
        match self {
            Reserved::Filters => "the fontView filter commands",
            Reserved::VisibleScripts => "fontView.setVisibleScripts",
            Reserved::RecentDocuments => "opening and saving documents",
            Reserved::ChooserPicks => "the Start Window",
            Reserved::SampleStrings => "the Sample Strings settings",
            Reserved::AuditProfiles => "the audit.profile commands",
            Reserved::ExportProfiles => "file.saveExportProfile and file.removeExportProfile",
            Reserved::TrustedDevices => "sharing.answerDevice and settings.resetTrustedDevices",
            Reserved::ShownReporters => "view.showReporter",
            Reserved::ExportSettings => "file.exportStatic and file.exportVariable when they succeed",
        }
    }

    pub fn of_id(id: &str) -> Option<Reserved> {
        Reserved::ALL.into_iter().find(|r| r.id() == id)
    }
}

/// The live setting with this id.
pub fn setting(id: &str) -> Option<&'static SettingDef> {
    SETTINGS.iter().find(|d| d.id == id)
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SettingsError {
    #[error("there is no setting `{0}`")]
    UnknownId(String),
    #[error("setting `{id}` is written by {owner}, not by settings.set")]
    ReservedId { id: String, owner: &'static str },
    #[error("setting `{id}` must be {expected}, got {got}")]
    BadValue { id: String, expected: String, got: String },
    #[error("{what} is too large: {size} is over the limit of {limit}")]
    TooLarge { what: String, size: usize, limit: usize },
    #[error("setting `{id}` holds a value that cannot be read: {message}")]
    Unreadable { id: String, message: String },
    #[error("the settings file cannot be read: {0}")]
    File(String),
}

/// The store. Cloning it copies one pointer, so a command's rollback snapshot is cheap.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct UserSettings {
    values: Arc<BTreeMap<String, Value>>,
    revision: u64,
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Whether `v` nests no deeper than `limit`.
fn depth_ok(v: &Value, limit: usize) -> bool {
    match v {
        Value::Array(a) => limit > 0 && a.iter().all(|x| depth_ok(x, limit - 1)),
        Value::Object(o) => limit > 0 && o.values().all(|x| depth_ok(x, limit - 1)),
        _ => true,
    }
}

/// Checks a value against the size and depth limits.
fn check_size(id: &str, v: &Value) -> Result<(), SettingsError> {
    if !depth_ok(v, MAX_DEPTH) {
        return Err(SettingsError::TooLarge { what: format!("setting `{id}`'s nesting"), size: MAX_DEPTH + 1, limit: MAX_DEPTH });
    }
    let size = serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX);
    if size > MAX_VALUE_BYTES {
        return Err(SettingsError::TooLarge { what: format!("setting `{id}`"), size, limit: MAX_VALUE_BYTES });
    }
    Ok(())
}

/// Whether `s` reads as `#rrggbb` or `#rrggbbaa`.
fn is_color(s: &str) -> bool {
    s.strip_prefix('#').is_some_and(|hex| (hex.len() == 6 || hex.len() == 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Checks a value against a live setting's kind.
fn check_kind(def: &SettingDef, v: &Value) -> Result<(), SettingsError> {
    let bad = |expected: String| SettingsError::BadValue { id: def.id.to_string(), expected, got: kind_of(v).to_string() };
    match (def.kind, v) {
        (Kind::Bool, Value::Bool(_)) => Ok(()),
        (Kind::Choice(choices), Value::String(s)) if choices.contains(&s.as_str()) => Ok(()),
        (Kind::Choice(choices), Value::String(s)) => Err(SettingsError::BadValue {
            id: def.id.to_string(),
            expected: format!("one of {}", choices.join(", ")),
            got: format!("`{}`", s.chars().take(64).collect::<String>()),
        }),
        (Kind::Number { min, max }, Value::Number(n)) => match n.as_f64() {
            Some(x) if x.is_finite() && (min..=max).contains(&x) => Ok(()),
            _ => Err(bad(format!("a number from {min} to {max}"))),
        },
        (Kind::Text, Value::String(s)) if s.len() <= MAX_TEXT_BYTES => Ok(()),
        (Kind::Text, Value::String(s)) => {
            Err(SettingsError::TooLarge { what: format!("setting `{}`", def.id), size: s.len(), limit: MAX_TEXT_BYTES })
        }
        (Kind::Color, Value::String(s)) if is_color(s) => Ok(()),
        (kind, _) => Err(bad(kind.name().to_string())),
    }
}

impl UserSettings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bumped by every change; the app saves the store when it differs from the saved one.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// A live setting's value (its default while none is stored), or a reserved id's stored
    /// value (`null` while none is stored).
    pub fn get(&self, id: &str) -> Result<Value, SettingsError> {
        if let Some(def) = setting(id) {
            return Ok(self.values.get(id).cloned().unwrap_or_else(|| def.default.value()));
        }
        if Reserved::of_id(id).is_some() {
            return Ok(self.values.get(id).cloned().unwrap_or(Value::Null));
        }
        Err(SettingsError::UnknownId(id.chars().take(MAX_ID_BYTES).collect()))
    }

    /// A live setting's value as a boolean (its default when the stored value is not one).
    pub fn get_bool(&self, id: &str) -> Option<bool> {
        self.get(id).ok().and_then(|v| v.as_bool())
    }

    /// Every live setting and every stored reserved id, by id.
    pub fn all(&self) -> Map<String, Value> {
        let mut out = Map::new();
        for def in SETTINGS {
            out.insert(def.id.to_string(), self.values.get(def.id).cloned().unwrap_or_else(|| def.default.value()));
        }
        for r in Reserved::ALL {
            if let Some(v) = self.values.get(r.id()) {
                out.insert(r.id().to_string(), v.clone());
            }
        }
        out
    }

    /// Sets a live setting; `null` returns it to its default. Reserved and unknown ids are
    /// refused. A refused value changes nothing.
    pub fn set(&mut self, id: &str, value: Value) -> Result<(), SettingsError> {
        if let Some(r) = Reserved::of_id(id) {
            return Err(SettingsError::ReservedId { id: id.to_string(), owner: r.owner() });
        }
        let def = setting(id).ok_or_else(|| SettingsError::UnknownId(id.chars().take(MAX_ID_BYTES).collect()))?;
        if value.is_null() {
            self.remove(id);
            return Ok(());
        }
        check_kind(def, &value)?;
        self.put(id, value)
    }

    /// A reserved id's value, read as `T`. `None` while nothing is stored; an error names the id
    /// when the stored value is not a `T` (a file from another version, or edited by hand).
    pub fn reserved<T: DeserializeOwned>(&self, r: Reserved) -> Result<Option<T>, SettingsError> {
        match self.values.get(r.id()) {
            None => Ok(None),
            Some(v) => T::deserialize(v).map(Some).map_err(|e| SettingsError::Unreadable { id: r.id().to_string(), message: e.to_string() }),
        }
    }

    /// Stores a reserved id's value. Only the commands that own the id call this. Checked
    /// against the size and depth limits; a refused value changes nothing.
    pub fn set_reserved<T: Serialize + ?Sized>(&mut self, r: Reserved, value: &T) -> Result<(), SettingsError> {
        let v = serde_json::to_value(value).map_err(|e| SettingsError::Unreadable { id: r.id().to_string(), message: e.to_string() })?;
        check_size(r.id(), &v)?;
        self.put(r.id(), v)
    }

    /// Removes a reserved id's value.
    pub fn clear_reserved(&mut self, r: Reserved) {
        self.remove(r.id());
    }

    /// The stored recent documents, newest first, without entries no command would have written.
    pub fn stored_recent_documents(&self) -> Vec<String> {
        let list = self.reserved::<Vec<String>>(Reserved::RecentDocuments).ok().flatten().unwrap_or_default();
        list.into_iter().filter(|p| !p.is_empty() && p.len() <= MAX_RECENT_PATH_BYTES).take(MAX_RECENT_DOCUMENTS).collect()
    }

    /// The recent documents whose files still exist, newest first.
    pub fn recent_documents(&self) -> Vec<String> {
        self.stored_recent_documents().into_iter().filter(|p| std::path::Path::new(p).exists()).collect()
    }

    /// Puts `path` first in the recent documents, as an absolute path, once, keeping at most
    /// [`MAX_RECENT_DOCUMENTS`]. `file.open`, `file.save` and `file.saveAs` call it.
    pub fn remember_document(&mut self, path: &std::path::Path) -> Result<(), SettingsError> {
        let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned();
        if full.is_empty() || full.len() > MAX_RECENT_PATH_BYTES {
            return Err(SettingsError::TooLarge { what: "a recent document's path".into(), size: full.len(), limit: MAX_RECENT_PATH_BYTES });
        }
        let mut list = self.stored_recent_documents();
        list.retain(|p| *p != full);
        list.insert(0, full);
        list.truncate(MAX_RECENT_DOCUMENTS);
        self.set_reserved(Reserved::RecentDocuments, &list)
    }

    fn put(&mut self, id: &str, value: Value) -> Result<(), SettingsError> {
        check_size(id, &value)?;
        if self.values.get(id) == Some(&value) {
            return Ok(());
        }
        if !self.values.contains_key(id) && self.values.len() >= MAX_ENTRIES {
            return Err(SettingsError::TooLarge { what: "the number of settings".into(), size: self.values.len() + 1, limit: MAX_ENTRIES });
        }
        Arc::make_mut(&mut self.values).insert(id.to_string(), value);
        self.revision += 1;
        Ok(())
    }

    fn remove(&mut self, id: &str) {
        if self.values.contains_key(id) {
            Arc::make_mut(&mut self.values).remove(id);
            self.revision += 1;
        }
    }

    /// The stored ids and values, without the defaults: what a settings file keeps.
    pub fn stored(&self) -> &BTreeMap<String, Value> {
        &self.values
    }

    /// The settings file: `{"version": 1, "settings": {id: value…}}`, ids sorted.
    pub fn to_json(&self) -> Vec<u8> {
        let file = json!({ "version": FILE_VERSION, "settings": self.values.as_ref() });
        serde_json::to_vec_pretty(&file).unwrap_or_default()
    }

    /// Reads a settings file. The file as a whole must be well-formed and within the limits;
    /// single entries that are not are dropped and reported, so one bad value does not lose the
    /// others. Ids FontCraft does not know are kept as they are (a newer version's settings
    /// survive a session of an older one).
    pub fn from_json(bytes: &[u8]) -> Result<(UserSettings, Vec<String>), SettingsError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(SettingsError::TooLarge { what: "the settings file".into(), size: bytes.len(), limit: MAX_FILE_BYTES });
        }
        let file: Value = serde_json::from_slice(bytes).map_err(|e| SettingsError::File(e.to_string()))?;
        let Some(obj) = file.as_object() else { return Err(SettingsError::File(format!("expected an object, found {}", kind_of(&file)))) };
        match obj.get("version").and_then(Value::as_u64) {
            Some(v) if v <= FILE_VERSION => {}
            Some(v) => return Err(SettingsError::File(format!("version {v} is newer than this FontCraft reads ({FILE_VERSION})"))),
            None => return Err(SettingsError::File("it has no version".into())),
        }
        let entries = match obj.get("settings") {
            Some(Value::Object(m)) => m,
            None => return Ok((UserSettings::new(), Vec::new())),
            Some(other) => return Err(SettingsError::File(format!("`settings` must be an object, found {}", kind_of(other)))),
        };
        UserSettings::from_entries(entries)
    }

    /// The store of the ids and values another file layout holds (`config.toml`), checked as [`UserSettings::from_json`]
    /// checks them.
    pub fn from_entries(entries: &Map<String, Value>) -> Result<(UserSettings, Vec<String>), SettingsError> {
        if entries.len() > MAX_ENTRIES {
            return Err(SettingsError::TooLarge { what: "the number of settings".into(), size: entries.len(), limit: MAX_ENTRIES });
        }
        let mut values = BTreeMap::new();
        let mut problems = Vec::new();
        for (id, value) in entries {
            if id.len() > MAX_ID_BYTES {
                problems.push(SettingsError::TooLarge { what: "a setting id".into(), size: id.len(), limit: MAX_ID_BYTES }.to_string());
                continue;
            }
            let checked = check_size(id, value).and_then(|()| match setting(id) {
                Some(def) => check_kind(def, value),
                None => Ok(()),
            });
            match checked {
                Ok(()) => {
                    values.insert(id.clone(), value.clone());
                }
                Err(e) => problems.push(format!("{e}; the default is used")),
            }
        }
        Ok((UserSettings { values: Arc::new(values), revision: 0 }, problems))
    }

    /// [`UserSettings::from_json`] that always gives a store: the defaults when the file cannot
    /// be read, with the reason among the problems.
    pub fn load(bytes: &[u8]) -> (UserSettings, Vec<String>) {
        match UserSettings::from_json(bytes) {
            Ok(loaded) => loaded,
            Err(e) => (UserSettings::new(), vec![format!("{e}; the default settings are used")]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_reload() {
        let mut s = UserSettings::new();
        assert_eq!(s.get("appearance.displayUnicodeValue").unwrap(), json!(false));
        s.set("appearance.displayUnicodeValue", json!(true)).unwrap();
        s.set_reserved(Reserved::VisibleScripts, &["latin", "greek"]).unwrap();
        let (back, problems) = UserSettings::load(&s.to_json());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back.get("appearance.displayUnicodeValue").unwrap(), json!(true));
        assert_eq!(back.reserved::<Vec<String>>(Reserved::VisibleScripts).unwrap(), Some(vec!["latin".to_string(), "greek".to_string()]));
        // null returns a setting to its default.
        let mut back = back;
        back.set("appearance.displayUnicodeValue", Value::Null).unwrap();
        assert_eq!(back.get("appearance.displayUnicodeValue").unwrap(), json!(false));
    }

    #[test]
    fn colour_and_handle_size_settings_check_their_values() {
        let mut s = UserSettings::new();
        assert_eq!(s.get("appearance.color.cornerNodes.light").unwrap(), json!("#2d6cdf"));
        s.set("appearance.color.cornerNodes.light", json!("#FF000080")).unwrap();
        assert_eq!(s.get("appearance.color.cornerNodes.light").unwrap(), json!("#FF000080"));
        for bad in [json!("red"), json!("#12345"), json!("#gg0000"), json!(3)] {
            let e = s.set("appearance.color.cornerNodes.dark", bad).unwrap_err().to_string();
            assert!(e.contains("#rrggbb"), "{e}");
        }
        assert_eq!(s.get("appearance.handleSize").unwrap(), json!("regular"));
        s.set("appearance.handleSize", json!("large")).unwrap();
        assert!(s.set("appearance.handleSize", json!("huge")).is_err());
    }

    #[test]
    fn kerning_colours_and_guide_opacity_have_defaults_and_separate_light_and_dark_values() {
        let mut s = UserSettings::new();
        assert_eq!(s.get("appearance.color.kerningNegative.light").unwrap(), json!("#8cc8f590"));
        assert_eq!(s.get("appearance.color.kerningPositive.dark").unwrap(), json!("#f5dc46a0"));
        assert_eq!(s.get("appearance.guideOpacity").unwrap(), json!(100.0));
        s.set("appearance.color.kerningNegative.dark", json!("#102030")).unwrap();
        assert_eq!(s.get("appearance.color.kerningNegative.dark").unwrap(), json!("#102030"));
        assert_eq!(s.get("appearance.color.kerningNegative.light").unwrap(), json!("#8cc8f590"), "the light value stays");
        let before = s.clone();
        for bad in [json!("#1020"), json!("#10203g"), json!(" #102030"), json!("#102030ff00")] {
            let e = s.set("appearance.color.kerningPositive.light", bad).unwrap_err().to_string();
            assert!(e.contains("appearance.color.kerningPositive.light") && e.contains("#rrggbb"), "{e}");
        }
        for bad in [json!(-1), json!(100.5), json!("50")] {
            assert!(s.set("appearance.guideOpacity", bad).is_err());
        }
        assert_eq!(s, before, "refused values change nothing");
        s.set("appearance.guideOpacity", json!(40)).unwrap();
        assert_eq!(s.get("appearance.guideOpacity").unwrap(), json!(40));
    }

    #[test]
    fn bad_ids_and_values_are_named() {
        let mut s = UserSettings::new();
        let e = s.set("no.suchSetting", json!(1)).unwrap_err().to_string();
        assert!(e.contains("no.suchSetting"), "{e}");
        let e = s.set("appearance.displayUnicodeValue", json!("yes")).unwrap_err().to_string();
        assert!(e.contains("appearance.displayUnicodeValue") && e.contains("true or false") && e.contains("text"), "{e}");
        let e = s.set("fontView.filters", json!([])).unwrap_err().to_string();
        assert!(e.contains("fontView.filters"), "{e}");
        let deep = (0..100).fold(json!(0), |v, _| json!([v]));
        assert!(matches!(s.set_reserved(Reserved::Filters, &deep), Err(SettingsError::TooLarge { .. })));
        let big = "x".repeat(MAX_VALUE_BYTES + 1);
        assert!(matches!(s.set_reserved(Reserved::Filters, &big), Err(SettingsError::TooLarge { .. })));
        assert_eq!(s, UserSettings::new(), "refused values change nothing");
        assert!(matches!(s.reserved::<Vec<String>>(Reserved::Filters), Ok(None)));
        s.set_reserved(Reserved::Filters, &json!({"not": "a list"})).unwrap();
        assert!(matches!(s.reserved::<Vec<String>>(Reserved::Filters), Err(SettingsError::Unreadable { .. })));
    }

    #[test]
    fn corrupt_files_give_defaults_and_a_report() {
        for bytes in [&b"{not json"[..], b"[]", b"{\"settings\": {}}", b"{\"version\": 99}", &[0xff, 0xfe, 0x00][..], &vec![b'['; 10_000][..]] {
            let (s, problems) = UserSettings::load(bytes);
            assert_eq!(s, UserSettings::new());
            assert_eq!(problems.len(), 1, "{problems:?}");
        }
        // One bad entry is dropped; the others and unknown ids are kept.
        let file = br#"{"version": 1, "settings": {"appearance.displayUnicodeValue": 3, "start.showAtLaunch": false, "later.setting": [1]}}"#;
        let (s, problems) = UserSettings::load(file);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("appearance.displayUnicodeValue"), "{problems:?}");
        assert_eq!(s.get("appearance.displayUnicodeValue").unwrap(), json!(false));
        assert_eq!(s.get_bool("start.showAtLaunch"), Some(false));
        assert!(String::from_utf8(s.to_json()).unwrap().contains("later.setting"));
    }

    #[test]
    fn live_settings_have_rows() {
        let spec = [
            include_str!("../../../spec/inventory/16-settings.toml"),
            include_str!("../../../spec/inventory/03a-edit-view-drawing.toml"),
            include_str!("../../../spec/inventory/90-secondary.toml"),
        ]
        .concat();
        for def in SETTINGS {
            assert!(spec.contains(&format!("[[setting]]\nid = \"{}\"", def.id)), "{} has no [[setting]] row", def.id);
        }
    }
}
