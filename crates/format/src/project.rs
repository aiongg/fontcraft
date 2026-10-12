//! The native project file (`.fontcraftproject`): a list of export instances linked to one font
//! file, kept apart from the font so that other versions of it can be exported without changing
//! the font (`project.separateFromFont`).
//!
//! A project file is UTF-8 JSON:
//! ```json
//! { "format": "fontcraft-project", "version": 1, "generator": "FontCraft 0.0.1",
//!   "font": "../Sources/Family.glyphs", "destination": "fonts",
//!   "instances": [ { …fontcraft_model::Instance… } ] }
//! ```
//! `font` and `destination` are stored as written by the engine: relative to the project file's
//! folder (with `/` between segments) or absolute. Instances use the same shape as in a
//! `.fontcraft` file. Keys this version does not know are kept and written back.
//!
//! The file is untrusted input: [`load_project`] refuses files over [`MAX_PROJECT_BYTES`], more
//! than [`MAX_PROJECT_INSTANCES`] instances, paths over [`MAX_PROJECT_PATH_BYTES`], and
//! instances whose counts or numbers are out of range, each with a message.

use fontcraft_model::{Instance, StyleClass};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::FormatError;

pub const PROJECT_FORMAT: &str = "fontcraft-project";
pub const PROJECT_EXTENSION: &str = "fontcraftproject";
/// The version this build writes and the newest it reads.
pub const PROJECT_VERSION: u32 = 1;
/// Largest project file [`load_project`] accepts.
pub const MAX_PROJECT_BYTES: usize = 64 << 20;
/// Most instances a project holds (as `fontcraft_fontinfo::MAX_PROJECT_INSTANCES`).
pub const MAX_PROJECT_INSTANCES: usize = 4_096;
/// Longest stored path (the font link, the export destination), in bytes.
pub const MAX_PROJECT_PATH_BYTES: usize = 4_096;
/// Longest instance name, in bytes (as `fontcraft_fontinfo::MAX_FIELD_BYTES`).
const MAX_INSTANCE_NAME_BYTES: usize = 65_535;
/// Most axis coordinates of one instance.
const MAX_LOCATION: usize = 256;
/// Most custom parameters of one instance (as `fontcraft_fontinfo::MAX_PARAMS_PER_SCOPE`).
const MAX_INSTANCE_PARAMS: usize = 4_096;
/// Most properties of one instance (as `fontcraft_fontinfo::MAX_PROPERTIES_PER_SCOPE`).
const MAX_INSTANCE_PROPERTIES: usize = 1_024;
/// Most hand-set master weights of one instance (as `fontcraft_fontinfo::MAX_MASTERS`).
const MAX_INSTANCE_WEIGHTS: usize = 256;
/// Largest stored coordinate or weight (as `fontcraft_model::MAX_COORDINATE`).
const MAX_NUMBER: f64 = 1_000_000.0;

/// A project as stored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// The linked font file, as stored (relative to the project's folder, or absolute). `None`:
    /// no font is linked yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The folder the project exports into, as stored. `None`: not chosen yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    /// The project's own export instances, in list order.
    #[serde(default)]
    pub instances: Vec<Instance>,
    /// Keys this version does not know, written back as read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The header keys, which [`Project::extra`] never holds.
const HEADER: [&str; 3] = ["format", "version", "generator"];

/// The bytes of a project file holding `project`.
pub fn save_project(project: &Project) -> Result<Vec<u8>, FormatError> {
    check_project(project)?;
    let mut value = serde_json::to_value(project).map_err(|e| FormatError::Encode(e.to_string()))?;
    if let Value::Object(map) = &mut value {
        for key in HEADER {
            map.remove(key);
        }
    }
    let generator = Value::String(format!("FontCraft {}", env!("CARGO_PKG_VERSION")));
    let mut out = format!("{{\n  \"format\": \"{PROJECT_FORMAT}\",\n  \"version\": {PROJECT_VERSION},\n  \"generator\": {generator}");
    if let Value::Object(map) = &value {
        for (key, v) in map {
            out.push_str(",\n  ");
            out.push_str(&Value::String(key.clone()).to_string());
            out.push_str(": ");
            crate::layout::write(&mut out, v, 1);
        }
    }
    out.push_str("\n}\n");
    Ok(out.into_bytes())
}

/// Reads a project file and checks what it holds.
pub fn load_project(bytes: &[u8]) -> Result<Project, FormatError> {
    if bytes.len() > MAX_PROJECT_BYTES {
        return Err(FormatError::TooLarge(bytes.len()));
    }
    let mut map: Map<String, Value> = serde_json::from_slice(bytes).map_err(|e| FormatError::NotFontcraft(format!("not a project file: {e}")))?;
    match map.get("format").and_then(Value::as_str) {
        Some(PROJECT_FORMAT) => {}
        Some(other) => return Err(FormatError::NotFontcraft(format!("the `format` key is `{other}`, expected `{PROJECT_FORMAT}`"))),
        None => return Err(FormatError::NotFontcraft(format!("the file has no `format` key: it is not a {PROJECT_EXTENSION} file"))),
    }
    let version = map.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version > u64::from(PROJECT_VERSION) {
        return Err(FormatError::TooNew { found: u32::try_from(version).unwrap_or(u32::MAX), supported: PROJECT_VERSION });
    }
    // Counted before the instances are built, so a hostile list costs no more than its JSON.
    if let Some(list) = map.get("instances") {
        let n = list.as_array().map(Vec::len).ok_or_else(|| FormatError::NotFontcraft("`instances` is not a list".into()))?;
        if n > MAX_PROJECT_INSTANCES {
            return Err(FormatError::NotFontcraft(format!("the project holds {n} instances; at most {MAX_PROJECT_INSTANCES} are read")));
        }
    }
    for key in HEADER {
        map.remove(key);
    }
    let project: Project = serde_json::from_value(Value::Object(map)).map_err(|e| FormatError::NotFontcraft(e.to_string()))?;
    check_project(&project)?;
    Ok(project)
}

/// Refuses a project whose counts, paths or numbers are out of range.
pub fn check_project(project: &Project) -> Result<(), FormatError> {
    let bad = |m: String| FormatError::NotFontcraft(m);
    for (what, path) in [("font link", &project.font), ("export destination", &project.destination)] {
        if let Some(path) = path {
            check_path(what, path).map_err(bad)?;
        }
    }
    if project.instances.len() > MAX_PROJECT_INSTANCES {
        return Err(bad(format!("a project holds at most {MAX_PROJECT_INSTANCES} instances")));
    }
    for (i, inst) in project.instances.iter().enumerate() {
        check_instance(inst).map_err(|m| bad(format!("instance #{i}: {m}")))?;
    }
    Ok(())
}

/// A stored path: not empty, within [`MAX_PROJECT_PATH_BYTES`], without NUL characters.
pub fn check_path(what: &str, path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(format!("the {what} is empty"));
    }
    if path.len() > MAX_PROJECT_PATH_BYTES {
        return Err(format!("the {what} is longer than {MAX_PROJECT_PATH_BYTES} bytes"));
    }
    if path.contains('\0') {
        return Err(format!("the {what} holds a NUL character"));
    }
    Ok(())
}

fn check_instance(inst: &Instance) -> Result<(), String> {
    let number = |n: f64| n.is_finite() && n.abs() <= MAX_NUMBER;
    if inst.name.len() > MAX_INSTANCE_NAME_BYTES {
        return Err(format!("the name is longer than {MAX_INSTANCE_NAME_BYTES} bytes"));
    }
    if inst.location.len() > MAX_LOCATION {
        return Err(format!("more than {MAX_LOCATION} axis coordinates"));
    }
    if !inst.location.iter().all(|&v| number(v)) {
        return Err("an axis coordinate is not a number or is out of range".into());
    }
    if inst.params.0.len() > MAX_INSTANCE_PARAMS {
        return Err(format!("more than {MAX_INSTANCE_PARAMS} custom parameters"));
    }
    if inst.properties.0.len() > MAX_INSTANCE_PROPERTIES {
        return Err(format!("more than {MAX_INSTANCE_PROPERTIES} properties"));
    }
    if inst.interpolation.len() > MAX_INSTANCE_WEIGHTS {
        return Err(format!("more than {MAX_INSTANCE_WEIGHTS} master weights"));
    }
    if !inst.interpolation.iter().all(|(_, w)| number(*w)) {
        return Err("a master weight is not a number or is out of range".into());
    }
    for class in [&inst.weight_class, &inst.width_class].into_iter().flatten() {
        if let StyleClass::Number(n) = class
            && !number(*n)
        {
            return Err("a weight or width class is not a number or is out of range".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Project {
        let bold = Instance { name: "Bold".into(), location: vec![700.0], ..Instance::default() };
        Project { font: Some("../Family.fontcraft".into()), destination: Some("fonts".into()), instances: vec![bold], extra: Map::new() }
    }

    #[test]
    fn a_project_round_trips_with_unknown_keys() {
        let mut project = sample();
        project.extra.insert("laterKey".into(), serde_json::json!({"a": 1}));
        let bytes = save_project(&project).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with("{\n  \"format\": \"fontcraft-project\",\n  \"version\": 1,"), "{text}");
        assert_eq!(load_project(&bytes).unwrap(), project);
    }

    #[test]
    fn hostile_files_are_refused_with_a_message() {
        assert!(load_project(b"[]").is_err());
        assert!(load_project(br#"{"format": "fontcraft", "version": 2}"#).is_err());
        assert!(matches!(load_project(br#"{"format": "fontcraft-project", "version": 99}"#), Err(FormatError::TooNew { .. })));
        let many = format!(r#"{{"format": "fontcraft-project", "instances": [{}]}}"#, vec!["{}"; MAX_PROJECT_INSTANCES + 1].join(","));
        let err = load_project(many.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("at most 4096"), "{err}");
        let long = format!(r#"{{"format": "fontcraft-project", "font": "{}"}}"#, "a".repeat(MAX_PROJECT_PATH_BYTES + 1));
        assert!(load_project(long.as_bytes()).is_err());
        let huge = br#"{"format": "fontcraft-project", "instances": [{"name": "X", "location": [1e300]}]}"#;
        assert!(load_project(huge).is_err());
        assert!(load_project(br#"{"format": "fontcraft-project", "instances": {}}"#).is_err());
    }

    #[test]
    fn an_empty_project_reads_back_empty() {
        let bytes = save_project(&Project::default()).unwrap();
        assert_eq!(load_project(&bytes).unwrap(), Project::default());
    }
}
