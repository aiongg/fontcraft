//! The source's feature-writer setting (the `com.github.googlei18n.ufo2ft.featureWriters` key of
//! its user data or UFO lib): which generated features run ([`kern_tags`] here, the mark,
//! cursive and `GDEF` writers' in [`crate::marks::mark_tags`]) and which writers run in append
//! mode ([`append_tags`]).
//!
//! Follows fontc's reading of the key (`fontir/src/ir/feature_writers.rs`, ported from fontc at
//! commit 4504578c3f74c2739f7e55cae133bbfe773f6d8c, Apache-2.0), which mirrors ufo2ft: without
//! the key every writer runs; with it only the writers it lists, so an empty list turns the
//! generated kerning off. An entry that is malformed, or has an option the writer does not take,
//! is dropped. Where fontc refuses to build (a value that is not a list, a kern option it does not
//! implement), FontCraft builds and warns.

use fontcraft_model::plist::{Num, Value};

/// The key that holds the setting.
pub const FEATURE_WRITERS_KEY: &str = "com.github.googlei18n.ufo2ft.featureWriters";

const DEFAULT_MODULE: &str = "ufo2ft.featureWriters";
const KERN_MODULE: &str = "ufo2ft.featureWriters.kernFeatureWriter";
const KERN2_MODULE: &str = "ufo2ft.featureWriters.kernFeatureWriter2";
const KERN_CLASS: &str = "KernFeatureWriter";

/// Which of the kern writer's features the source asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernTags {
    pub kern: bool,
    pub dist: bool,
}

impl KernTags {
    pub const ALL: KernTags = KernTags { kern: true, dist: true };
    pub const NONE: KernTags = KernTags { kern: false, dist: false };

    pub fn any(self) -> bool {
        self.kern || self.dist
    }
}

/// The kern writer's features under `setting` (the key's value, `None` without the key), and
/// what the user should know about it.
pub fn kern_tags(setting: Option<&Value>, warnings: &mut Vec<String>) -> KernTags {
    let Some(setting) = setting else { return KernTags::ALL };
    let Some(entries) = setting.as_array() else {
        warnings.push(format!("the feature-writer setting ({FEATURE_WRITERS_KEY}) is not a list, so no feature writer runs"));
        return KernTags::NONE;
    };
    // A later entry for the same writer replaces an earlier one, as fontc's plan does.
    entries.items.iter().filter_map(|entry| kern_entry(entry, warnings)).last().unwrap_or(KernTags::NONE)
}

/// The tags of one entry when it is a valid kern writer entry.
fn kern_entry(entry: &Value, warnings: &mut Vec<String>) -> Option<KernTags> {
    let dict = entry.as_dict()?;
    let class = dict.get("class")?.as_str()?;
    let module = match dict.get("module") {
        None => DEFAULT_MODULE,
        Some(m) => m.as_str()?,
    };
    let legacy = module == KERN2_MODULE;
    if class != KERN_CLASS || !(legacy || module == DEFAULT_MODULE || module == KERN_MODULE) {
        return None;
    }
    let mut tags = KernTags::ALL;
    let mut unapplied = Vec::new();
    if legacy {
        unapplied.push("the legacy kern writer (kernFeatureWriter2)".to_string());
    }
    if let Some(options) = dict.get("options") {
        for (key, value) in &options.as_dict()?.entries {
            match key.as_str() {
                "mode" => {
                    // Append mode is read by `append_tags`.
                    if !matches!(value.as_str(), Some("skip" | "append")) {
                        return None;
                    }
                }
                "features" => tags = features(value)?,
                "ignoreMarks" => match flag(value)? {
                    true => {}
                    false => unapplied.push("ignoreMarks off".to_string()),
                },
                "quantization" => match value.as_num()? {
                    Num::Int(1) => {}
                    Num::Int(q) => unapplied.push(format!("quantization {q}")),
                    Num::Float(_) => return None,
                },
                _ => return None,
            }
        }
    }
    for what in unapplied {
        warnings
            .push(format!("the feature-writer setting asks for {what}, which is not supported: the kerning is written with the standard settings"));
    }
    Some(tags)
}

/// A yes-or-no option: a property-list boolean, or 0 or 1 in a format without booleans.
fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.value() {
            Num::Int(0) => Some(false),
            Num::Int(1) => Some(true),
            _ => None,
        },
        _ => None,
    }
}

/// The `features` option: a non-empty list of the kern writer's tags.
fn features(value: &Value) -> Option<KernTags> {
    let items = &value.as_array()?.items;
    if items.is_empty() {
        return None;
    }
    let mut tags = KernTags::NONE;
    for item in items {
        match item.as_str()? {
            "kern" => tags.kern = true,
            "dist" => tags.dist = true,
            _ => return None,
        }
    }
    Some(tags)
}

/// The generated features whose writer the setting puts in append mode (`mode = append`): their
/// lookups go after every hand-written lookup, `# Automatic Code` markers ignored, and a feature
/// the code declares does not keep them out (`compile.featureWriterModes`). A writer's
/// `features` option limits its tags; the last entry of a writer decides.
pub fn append_tags(setting: Option<&Value>) -> Vec<&'static str> {
    let Some(entries) = setting.and_then(Value::as_array) else { return Vec::new() };
    const KERN: &[&str] = &["kern", "dist"];
    const MARK: &[&str] = &["mark", "mkmk", "abvm", "blwm"];
    const CURS: &[&str] = &["curs"];
    let mut by_writer: Vec<(&'static [&'static str], Vec<&'static str>)> = Vec::new();
    for entry in &entries.items {
        let Some(dict) = entry.as_dict() else { continue };
        let all: &'static [&'static str] = match dict.get("class").and_then(Value::as_str) {
            Some(KERN_CLASS) => KERN,
            Some("MarkFeatureWriter" | "ContextualMarkFeatureWriter") => MARK,
            Some("CursFeatureWriter") => CURS,
            _ => continue,
        };
        let options = dict.get("options").and_then(Value::as_dict);
        let append = options.and_then(|o| o.get("mode")).and_then(Value::as_str) == Some("append");
        let wanted: Option<Vec<String>> = options
            .and_then(|o| o.get("features"))
            .and_then(Value::as_array)
            .map(|a| a.items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
        let tags: Vec<&'static str> =
            if append { all.iter().copied().filter(|t| wanted.as_ref().is_none_or(|w| w.iter().any(|x| x == t))).collect() } else { Vec::new() };
        by_writer.retain(|(w, _)| !std::ptr::eq(*w, all));
        by_writer.push((all, tags));
    }
    by_writer.into_iter().flat_map(|(_, tags)| tags).collect()
}
