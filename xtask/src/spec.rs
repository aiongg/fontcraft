//! `cargo xtask spec`: load and check the feature inventory (`spec/README.md` has the rules).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use toml::{Table, Value};

pub const KINDS: &[&str] = &["command", "tool", "panel", "setting", "parameter", "behavior", "cli", "format", "check", "table"];
const STATUSES: &[&str] = &["handbook", "docs", "memory", "inferred", "observed"];
const ORIGINS: &[&str] = &["fontlab", "robofont", "fontforge", "fontra"]; // brand-ok: the inventory's `origin` values, not user-visible text
const MODIFIERS: &[&str] = &["Cmd", "Ctrl", "Alt", "Shift"];
/// Focus scopes a `when` may name besides `tool:<id>` and `panel:<id>` (decision 0004).
const SCOPES: &[&str] = &["editView", "textMode", "fontView", "dialog"];
/// Context menus a command's `context` may list.
const CONTEXTS: &[&str] =
    &["canvas", "canvas.node", "canvas.segment", "canvas.path", "canvas.guide", "canvas.anchor", "canvas.component", "layers.row", "toolbar"];

pub struct Area {
    pub id: String,
    pub title: String,
    pub weight: u32,
}

pub struct Milestone {
    pub id: String,
    pub title: String,
}

/// One inventory row.
pub struct Row {
    pub kind: &'static str,
    /// `id`, or `name` for parameters, or `tag` for tables.
    pub key: String,
    pub file: String,
    pub area: String,
    pub milestone: String,
    pub status: String,
    pub fields: Table,
}

impl Row {
    pub fn str(&self, field: &str) -> &str {
        self.fields.get(field).and_then(Value::as_str).unwrap_or("")
    }
}

pub struct Inventory {
    pub areas: Vec<Area>,
    pub milestones: Vec<Milestone>,
    pub rows: Vec<Row>,
}

fn key_field(kind: &str) -> &'static str {
    match kind {
        "parameter" => "name",
        "table" => "tag",
        _ => "id",
    }
}

fn read_table(path: &Path) -> Result<Table, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.parse::<Table>().map_err(|e| format!("{}: {e}", path.display()))
}

fn list<'a>(table: &'a Table, key: &str, file: &str) -> Result<&'a Vec<Value>, String> {
    table.get(key).and_then(Value::as_array).ok_or_else(|| format!("{file}: expected [[{key}]] rows"))
}

/// Is `id` written `noun.camelCase` (ASCII letters, digits and dots; no empty part)?
fn is_command_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|p| p.starts_with(|c: char| c.is_ascii_lowercase()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// `Cmd+Ctrl+Alt+Shift+Key`, modifiers in that order, then exactly one key.
fn is_shortcut(sc: &str) -> bool {
    let parts: Vec<&str> = sc.split('+').collect();
    let Some((key, mods)) = parts.split_last() else { return false };
    let positions: Vec<usize> = mods.iter().filter_map(|m| MODIFIERS.iter().position(|o| o == m)).collect();
    !key.is_empty() && !MODIFIERS.contains(key) && positions.len() == mods.len() && positions.windows(2).all(|w| w[0] < w[1])
}

/// Does the shortcut fire while a text field has focus? Only Cmd and Ctrl keep a key out of text
/// entry; a plain key, Shift or Alt types or edits text.
fn types_into_text(sc: &str) -> bool {
    !sc.is_empty() && !sc.split('+').rev().skip(1).any(|m| m == "Cmd" || m == "Ctrl")
}

/// Loads `spec/` and returns the inventory, or every problem found.
pub fn load(root: &Path) -> Result<Inventory, Vec<String>> {
    let mut errors = Vec::new();
    let spec = root.join("spec");
    let mut areas = Vec::new();
    let mut milestones = Vec::new();
    match read_table(&spec.join("areas.toml")).and_then(|t| list(&t, "area", "spec/areas.toml").cloned()) {
        Ok(rows) => {
            for r in rows {
                let get = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let weight = r.get("weight").and_then(Value::as_integer).and_then(|w| u32::try_from(w).ok()).unwrap_or(0);
                if get("id").is_empty() || get("title").is_empty() || weight == 0 {
                    errors.push(format!("spec/areas.toml: an area needs id, title and a positive weight ({})", get("id")));
                }
                areas.push(Area { id: get("id"), title: get("title"), weight });
            }
        }
        Err(e) => errors.push(e),
    }
    match read_table(&spec.join("milestones.toml")).and_then(|t| list(&t, "milestone", "spec/milestones.toml").cloned()) {
        Ok(rows) => {
            for r in rows {
                let get = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                if get("id").is_empty() || get("title").is_empty() {
                    errors.push("spec/milestones.toml: a milestone needs id and title".to_string());
                }
                milestones.push(Milestone { id: get("id"), title: get("title") });
            }
        }
        Err(e) => errors.push(e),
    }
    let area_ids: HashSet<&str> = areas.iter().map(|a| a.id.as_str()).collect();
    let milestone_ids: HashSet<&str> = milestones.iter().map(|m| m.id.as_str()).collect();

    let mut files: Vec<_> = match std::fs::read_dir(spec.join("inventory")) {
        Ok(dir) => dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect(),
        Err(e) => {
            errors.push(format!("spec/inventory: {e}"));
            Vec::new()
        }
    };
    files.sort();
    let mut rows: Vec<Row> = Vec::new();
    let mut seen: BTreeMap<(&'static str, String), String> = BTreeMap::new();
    for path in &files {
        let file = format!("spec/inventory/{}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let table = match read_table(path) {
            Ok(t) => t,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        for (kind_name, value) in &table {
            let Some(kind) = KINDS.iter().find(|k| *k == kind_name).copied() else {
                errors.push(format!("{file}: unknown row kind [[{kind_name}]]"));
                continue;
            };
            let Some(entries) = value.as_array() else {
                errors.push(format!("{file}: `{kind_name}` must be [[{kind_name}]] rows"));
                continue;
            };
            for entry in entries {
                let Some(fields) = entry.as_table() else { continue };
                let s = |k: &str| fields.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let key = s(key_field(kind));
                let at = format!("{file}: {kind} `{key}`");
                let mut err = |m: String| errors.push(format!("{at}: {m}"));
                if key.is_empty() {
                    err(format!("`{}` is required", key_field(kind)));
                    continue;
                }
                if let Some(first) = seen.insert((kind, key.clone()), file.clone()) {
                    err(format!("duplicate (also in {first})"));
                }
                if !area_ids.contains(s("area").as_str()) {
                    err(format!("unknown area `{}`", s("area")));
                }
                if !milestone_ids.contains(s("milestone").as_str()) {
                    err(format!("unknown milestone `{}`", s("milestone")));
                }
                if !STATUSES.contains(&s("status").as_str()) {
                    err(format!("status must be one of {STATUSES:?}, got `{}`", s("status")));
                }
                if s("summary").trim().is_empty() {
                    err("`summary` is empty".into());
                }
                if fields.contains_key("origin") && !ORIGINS.contains(&s("origin").as_str()) {
                    err(format!("origin must be one of {ORIGINS:?}, got `{}`", s("origin")));
                }
                let has_list = |k: &str| fields.get(k).is_some_and(Value::is_array);
                let one_of = |k: &str, allowed: &[&str]| allowed.contains(&s(k).as_str());
                let shortcut_ok = s("shortcut").is_empty() || is_shortcut(&s("shortcut"));
                let scopes_ok = fields
                    .get("scope")
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.iter().all(|v| matches!(v.as_str(), Some("font" | "master" | "instance"))));
                let flag = |k: &str| fields.get(k).is_some_and(Value::is_bool);
                let is_str = |k: &str| fields.get(k).is_some_and(Value::is_str);
                let absent_or_str = |k: &str| !fields.contains_key(k) || is_str(k);
                let when_ok = !fields.contains_key("when") || (is_str("when") && !s("when").is_empty());
                let when_needed = !types_into_text(&s("shortcut")) || fields.contains_key("when");
                let context_ok = fields.get("context").is_none_or(|v| {
                    v.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|c| c.as_str().is_some_and(|c| CONTEXTS.contains(&c))))
                });
                // One message per broken rule; a row may break several.
                let rules: &[(&str, bool, &str)] = &[
                    ("command", is_command_id(&key), "a command id is noun.camelCase: ASCII letters, digits and dots"),
                    (
                        "command",
                        !s("label").is_empty() && has_list("menu") && is_str("shortcut") && is_str("params"),
                        "a command needs label, menu, shortcut and params",
                    ),
                    ("command", one_of("kind", &["engine", "ui"]), "kind must be engine or ui"),
                    ("command", shortcut_ok, "the shortcut is not written Cmd+Ctrl+Alt+Shift+Key"),
                    ("command", when_ok, "`when` must be a non-empty string"),
                    ("tool", when_ok, "`when` must be a non-empty string"),
                    ("command", when_needed, "a shortcut without Cmd or Ctrl needs `when`, so it never fires while a text field has focus"),
                    ("command", context_ok, "`context` must be a non-empty list of known context menus (spec/README.md)"),
                    ("tool", !s("label").is_empty() && has_list("gestures"), "a tool needs label and gestures"),
                    ("panel", !s("label").is_empty() && has_list("controls"), "a panel needs label and controls"),
                    (
                        "panel",
                        one_of("location", &["palette", "sidebar", "inspector", "window", "dialog", "toolbar", "statusbar"]),
                        "unknown location",
                    ),
                    (
                        "setting",
                        !s("label").is_empty() && one_of("kind", &["bool", "choice", "number", "string", "color", "shortcut", "path"]),
                        "a setting needs label and a kind (bool, choice, number, string, color, shortcut, path)",
                    ),
                    (
                        "parameter",
                        scopes_ok && one_of("kind", &["bool", "number", "string", "list", "dict", "choice"]),
                        "a parameter needs scope (font, master, instance; may be empty) and a kind",
                    ),
                    (
                        "cli",
                        !s("usage").is_empty() && has_list("options") && has_list("equivalents") && is_str("command"),
                        "a cli row needs usage, options, command and equivalents",
                    ),
                    (
                        "format",
                        !s("label").is_empty() && has_list("extensions") && flag("read") && flag("write"),
                        "a format needs label, extensions, read and write",
                    ),
                    (
                        "check",
                        one_of("severity", &["fail", "warn", "info"]) && one_of("scope", &["source", "binary", "family"]) && flag("fixable"),
                        "a check needs severity (fail, warn, info), scope (source, binary, family) and fixable",
                    ),
                    ("table", has_list("flavour") && !s("read").is_empty() && !s("write").is_empty(), "a table needs flavour, read and write"),
                ];
                for (_, _, message) in rules.iter().filter(|(k, ok, _)| *k == kind && !ok) {
                    err((*message).to_string());
                }
                if !absent_or_str("notes") {
                    err("`notes` must be a string".into());
                }
                if fields.contains_key("context") && kind != "command" {
                    err("only a command has `context`".into());
                }
                if fields.contains_key("when") && !matches!(kind, "command" | "tool") {
                    err("only a command or a tool has `when`".into());
                }
                rows.push(Row {
                    kind,
                    key,
                    file: file.clone(),
                    area: s("area"),
                    milestone: s("milestone"),
                    status: s("status"),
                    fields: fields.clone(),
                });
            }
        }
    }
    // A cli row's command must be a command in the inventory.
    let commands: HashSet<&str> = rows.iter().filter(|r| r.kind == "command").map(|r| r.key.as_str()).collect();
    // A `when` names a scope, or a tool or panel that has a row.
    let ids = |kind: &str| -> HashSet<&str> { rows.iter().filter(|r| r.kind == kind).map(|r| r.key.as_str()).collect() };
    let (tools, panels) = (ids("tool"), ids("panel"));
    for r in rows.iter().filter(|r| r.fields.contains_key("when")) {
        let when = r.str("when");
        let known = SCOPES.contains(&when)
            || when.strip_prefix("tool:").is_some_and(|t| tools.contains(t))
            || when.strip_prefix("panel:").is_some_and(|p| panels.contains(p));
        if !known {
            errors.push(format!(
                "{}: {} `{}`: `when` must be one of {SCOPES:?}, tool:<tool id> or panel:<panel id>, got `{when}`",
                r.file, r.kind, r.key
            ));
        }
    }
    for r in rows.iter().filter(|r| r.kind == "cli") {
        let target = r.str("command");
        if !target.is_empty() && !commands.contains(target) {
            errors.push(format!("{}: cli `{}`: its command `{target}` is not a [[command]] row", r.file, r.key));
        }
    }
    if errors.is_empty() { Ok(Inventory { areas, milestones, rows }) } else { Err(errors) }
}

pub fn run(root: &Path) -> Result<(), String> {
    match load(root) {
        Ok(inv) => {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for r in &inv.rows {
                *counts.entry(r.kind).or_default() += 1;
            }
            let by_kind: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
            println!("spec: {} rows in {} areas and {} milestones ({})", inv.rows.len(), inv.areas.len(), inv.milestones.len(), by_kind.join(", "));
            Ok(())
        }
        Err(errors) => {
            for e in errors.iter().take(60) {
                eprintln!("  {e}");
            }
            Err(format!("{} problem(s) in spec/", errors.len()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_shortcuts_are_recognised() {
        for good in ["file.new", "font.info.get", "path.addPath", "a.b2", "hint.tt.addStemHint"] {
            assert!(is_command_id(good), "{good}");
        }
        for bad in ["new", "file.", ".new", "file.new glyph", "file.añadir", "1.x", "file..new", "File.New", "a.1x", "file.New"] {
            assert!(!is_command_id(bad), "{bad}");
        }
        for good in ["P", "Cmd+N", "Cmd+Ctrl+Alt+Shift+N", "Shift+B", "Cmd+Plus", "Ctrl+Tab"] {
            assert!(is_shortcut(good), "{good}");
        }
        for bad in ["Shift+Cmd+N", "Cmd+", "Cmd+Cmd+N", "Option+N", "Cmd+Shift", ""] {
            assert!(!is_shortcut(bad), "{bad}");
        }
        for text in ["Tab", "Shift+Tab", "Alt+Delete", "Alt+Shift+Return", "A"] {
            assert!(types_into_text(text), "{text}");
        }
        for global in ["Cmd+A", "Ctrl+Tab", "Cmd+Alt+Shift+R", ""] {
            assert!(!types_into_text(global), "{global}");
        }
    }

    #[test]
    fn the_repository_inventory_loads() {
        let inv = load(&crate::root()).unwrap_or_else(|e| panic!("{}", e.join("\n")));
        assert!(inv.rows.len() > 3000 && inv.areas.len() > 15);
    }
}
