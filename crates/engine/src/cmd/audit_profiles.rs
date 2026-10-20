//! Audit profiles, settings, the ignore list and the result cache: `audit.clearCache`, `audit.profile.show`, `.save`, `.delete`, `.setDefault`, `audit.config.get`, `.set`, `audit.ignoreIssue`, `.unignoreIssue`, `.listIgnored` (M12.59).
//!
//! Where things live (plan M12 § 3, "Audit settings storage"):
//! - user profiles and the application's default profile: the user settings, reserved id
//!   `audit.userProfiles`, as `{default?, profiles: [profile]}`;
//! - project profiles, the document's default profile, its threshold overrides, project values
//!   and ignore list: the source's font user data under `com.fontcraft.audit`
//!   ([`fontcraft_audit::profile::DocumentSettings`]), so they travel with the source in every
//!   format. Changing them is one undo step.
//!
//! Parsing, limits and profile resolution live in [`fontcraft_audit::profile`]; parameters are
//! turned into property-list values and read by the same code that reads the files, so a value
//! a command accepts is a value the file reader accepts. The runner (`audit.run`) reads the same
//! settings through [`user_store`] and [`default_profile`].

use std::collections::{BTreeSet, HashMap};

use fontcraft_audit::profile::{
    self as prof, DocumentSettings, IgnoreEntry, Location, PROJECT_VALUES, ProfileDef, Profiles, ProjectValue, Store, location_from_plist,
    project_value_def,
};
use fontcraft_audit::{self as audit, CHECKS, Config, Profile};
use fontcraft_model::plist::{Array, Dict, Num, Number, Str, Value as PValue};
use serde_json::{Map, Value, json};

use super::*;
use crate::clock;
use crate::params::Params;
use crate::settings::Reserved;

/// Deepest nesting of a JSON parameter turned into a property-list value.
const MAX_JSON_DEPTH: usize = 16;
/// Most names one `audit.config.set` call changes.
const MAX_SET_VALUES: usize = 1024;
/// The keys of a result's location an ignore entry made from it keeps: the table-level keys of
/// `docs/spec/21-qa.md` § 4. Glyph-level addresses (shapes, points, bounds) change with every
/// edit and are not kept; font ids (`font`, `fonts`) belong to one run.
const LOCATION_KEYS: [&str; 12] =
    ["table", "field", "nameId", "platform", "encoding", "language", "feature", "script", "lookup", "axis", "instance", "pair"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "audit.clearCache",
            "Clear Audit Cache",
            [],
            None,
            "{target?: \"document\" (default) | path} discards the results and the per-glyph layer-check results kept from the last audit.run on the open document, so the next run examines every glyph again and audit.results has nothing until then. A file target keeps no cache: nothing is cleared → {cleared: bool}",
            always,
            clear_cache
        ),
        cmd!(
            query "audit.profile.show",
            "Show Audit Profile",
            [],
            None,
            "{profile: id} a profile in full: the profiles it includes, the checks it adds and excludes, its severity overrides and threshold defaults, and the checks it runs once resolved. Built-in profiles, the user's profiles (application settings) and the open document's project profiles; a project profile hides a user profile of the same id → {id, label, store: \"builtIn\" | \"user\" | \"project\", builtIn, include: [ids], checks: [id | prefix*], exclude: [id | prefix*], overrides: [{check, code?, severity}], config: {checkId: {threshold: number}}, default: bool, resolved: {count, checks: [ids]}, problems?: [text]}",
            always,
            profile_show
        ),
        cmd!(
            "audit.profile.save",
            "Save Audit Profile",
            [],
            None,
            "{profile: id (letters, digits, `.`, `_`, `-`; not a built-in id), label?: string, include?: [profile ids], checks?: [id | prefix*], exclude?: [id | prefix*], overrides?: [{check, code?: message code, severity: \"fail\" | \"warn\" | \"info\"}], config?: {checkId: {threshold: number}}, store?: \"user\" (default) | \"project\"} creates or replaces a profile. The checks are the included profiles' checks plus `checks`, minus `exclude`; overrides and thresholds apply over the included profiles' own. A user profile is kept in the application settings and includes built-in and user profiles; a project profile is kept in the open document (one undo step) and includes built-in and project profiles. Refused: a built-in id, an id the other store holds, unknown checks, codes or thresholds, an include that does not exist, a cycle, includes nested deeper than 8 → {profile: the profile as audit.profile.show gives it, created: bool, checks: count}",
            always,
            profile_save
        ),
        cmd!(
            "audit.profile.delete",
            "Delete Audit Profile",
            [],
            None,
            "{profile: id, store?: \"user\" | \"project\" (needed only when both hold the id)} deletes a user or project profile. Refused while another profile of its store includes it. A document or application default that named it is cleared: the document falls back to the application default, else universal → {deleted, store, defaultCleared: bool}",
            always,
            profile_delete
        ),
        cmd!(
            "audit.profile.setDefault",
            "Set Default Audit Profile",
            [],
            None,
            "{profile: id | null, target?: \"document\" (default) | \"application\"} sets the profile audit.run uses when the call names none: for the open document (kept in the source, one undo step) or for the application (the user settings; a project profile cannot be the application default). null clears it. Without a document default (or when it no longer exists) the application default applies, else universal → {profile: id | null, target, effective: id}",
            always,
            profile_set_default
        ),
        cmd!(
            query "audit.config.get",
            "Get Audit Settings",
            [],
            None,
            "{check?: id, profile?: id (default: the effective default profile)} the thresholds a run uses, each with where its value comes from (the document's override, the profile's default, or the check's built-in default), for one check or every check that has thresholds, and the open document's project values (unset ones are null) → {profile, profileFrom: \"call\" | \"document\" | \"application\" | \"builtIn\", note?, checks: {checkId: {threshold: {value, from: \"document\" | \"profile\" | \"default\", default, min, max}}}, values: {name: {value, kind, doc}}, problems?: [text]}",
            always,
            config_get
        ),
        cmd!(
            "audit.config.set",
            "Change Audit Settings",
            [],
            None,
            "{check?: id, values: {name: value | null}} stores overrides in the open document, as one undo step: with `check`, threshold values of that check (numbers within each threshold's range); without, project values: vendorId (1 to 4 ASCII characters), defaultUrl, fsType (0 to 65535), allowedTables (list of tags), glyphSet, keepMacNames (bool), warnSize and failSize (bytes). null removes an override. Unknown names and values out of range are refused and nothing changes → {check?, values: {name: value | null}}",
            has_doc,
            config_set
        ),
        cmd!(
            "audit.ignoreIssue",
            "Ignore Issue",
            [],
            None,
            "{result?: result id (from the last audit.run), check?: id, glyph?: name, master?: id, location?: {key: text | number | [text | number]}, reason?: string} adds an entry to the open document's ignore list, as one undo step: from a result (its check, glyph, master and table-level location) or from `check` with an optional glyph, master and location. An entry without a glyph covers the check in the whole font; one without a master covers every master. Matching results are hidden and do not count toward the exit status. Ignoring the same issue again replaces its reason and date → {entry: {check, glyph?, master?, location?, reason?, date?}, added: bool, entries: count}",
            has_doc,
            ignore_issue
        ),
        cmd!(
            "audit.unignoreIssue",
            "Stop Ignoring Issue",
            [],
            None,
            "{check: id, glyph?: name, master?: id, location?: object} removes from the open document's ignore list the entries of `check` that name this glyph, master and location; a key left out matches any entry. Refused when no entry matches → {removed: count, entries: count}",
            has_doc,
            unignore_issue
        ),
        cmd!(
            query "audit.listIgnored",
            "List Ignored Issues",
            [],
            None,
            "{} the open document's ignore list with the reason and date of each entry and whether the issue still occurs in the last audit.run: true, false, or null when no run is kept or the run did not include the check → {entries: [{check, glyph?, master?, location?, reason?, date?, occurs: bool | null}], checkedAgainstRun: bool, problems?: [text]}",
            has_doc,
            list_ignored
        ),
    ]
}

// ── JSON and property lists ─────────────────────────────────────────────────────────────────────

/// A JSON parameter as a property-list value. `null` is dropped (inside a list or object too);
/// numbers must be finite.
fn to_plist(v: &Value, depth: usize) -> std::result::Result<Option<PValue>, String> {
    if depth > MAX_JSON_DEPTH {
        return Err(format!("a value nests deeper than {MAX_JSON_DEPTH}"));
    }
    Ok(Some(match v {
        Value::Null => return Ok(None),
        Value::Bool(b) => PValue::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => PValue::Number(Number::int(i)),
            (None, Some(f)) => PValue::Number(Number::float(f).ok_or("numbers must be finite")?),
            (None, None) => return Err("numbers must be finite".into()),
        },
        Value::String(s) => PValue::String(Str::new(s.as_str())),
        Value::Array(a) => {
            let mut items = Vec::with_capacity(a.len());
            for x in a {
                if let Some(p) = to_plist(x, depth + 1)? {
                    items.push(p);
                }
            }
            PValue::Array(Array { items, ..Array::default() })
        }
        Value::Object(o) => {
            let mut d = Dict::new();
            for (k, x) in o {
                if let Some(p) = to_plist(x, depth + 1)? {
                    d.insert(k.as_str(), p);
                }
            }
            PValue::Dict(d)
        }
    }))
}

/// A property-list value as JSON. Values nested deeper than the limit become `null`.
fn to_json(v: &PValue, depth: usize) -> Value {
    if depth > MAX_JSON_DEPTH {
        return Value::Null;
    }
    match v {
        PValue::Dict(d) => Value::Object(d.entries.iter().map(|(k, x)| (k.as_str().to_string(), to_json(x, depth + 1))).collect()),
        PValue::Array(a) => Value::Array(a.items.iter().map(|x| to_json(x, depth + 1)).collect()),
        PValue::String(s) | PValue::Date(s) => json!(s.as_str()),
        PValue::Number(n) => match n.value() {
            Num::Int(i) => json!(i),
            Num::Float(f) => json!(f),
        },
        PValue::Bool(b) => json!(b),
        PValue::Data(d) => json!(format!("<{} bytes>", d.bytes().len())),
    }
}

fn param_plist(p: &Params, key: &str) -> Result<Option<PValue>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => to_plist(v, 0).map_err(|e| p.bad(format!("`{key}`: {e}"))),
    }
}

// ── Where the settings are kept ─────────────────────────────────────────────────────────────────

/// The user's audit profiles and default profile (application settings).
#[derive(Clone, Debug, Default)]
pub(crate) struct UserStore {
    pub default: Option<String>,
    pub profiles: Vec<ProfileDef>,
}

/// The user's store, and what of it could not be read (dropped).
pub(crate) fn user_store(s: &Session) -> (UserStore, Vec<String>) {
    let mut out = UserStore::default();
    let mut problems = Vec::new();
    let stored = match s.settings().reserved::<Value>(Reserved::AuditProfiles) {
        Ok(v) => v,
        Err(e) => {
            problems.push(e.to_string());
            None
        }
    };
    let Some(Value::Object(o)) = stored else { return (out, problems) };
    if let Some(d) = o.get("default").and_then(Value::as_str) {
        out.default = Some(d.to_string());
    }
    if let Some(list) = o.get("profiles").and_then(Value::as_array) {
        for (i, item) in list.iter().take(prof::MAX_PROFILES).enumerate() {
            let read = to_plist(item, 0)
                .and_then(|v| v.ok_or_else(|| "empty".to_string()))
                .and_then(|v| ProfileDef::from_plist(&v))
                .and_then(|def| prof::check_profile_id(&def.id).map(|()| def));
            match read {
                Ok(def) if out.profiles.iter().any(|q| q.id == def.id) => problems.push(format!("a second user profile `{}` was dropped", def.id)),
                Ok(def) => out.profiles.push(def),
                Err(e) => problems.push(format!("user profile {} was dropped: {e}", i + 1)),
            }
        }
    }
    (out, problems)
}

/// How refusals name the two stores.
const USER_STORE: &str = "the user's audit profiles (application settings)";
const DOC_STORE: &str = "the open document's audit settings";

fn save_user_store(s: &mut Session, store: &UserStore) -> Result<()> {
    let profiles: Vec<Value> = store.profiles.iter().map(|d| to_json(&d.to_plist(), 0)).collect();
    let mut o = Map::new();
    if let Some(d) = &store.default {
        o.insert("default".into(), json!(d));
    }
    o.insert("profiles".into(), Value::Array(profiles));
    s.settings_mut().set_reserved(Reserved::AuditProfiles, &Value::Object(o))?;
    Ok(())
}

/// Refuses to write `store` back when part of it could not be read: the write would drop that
/// part without a word.
fn refuse_unread(p: &Params, store: &str, problems: &[String]) -> Result<()> {
    if problems.is_empty() {
        return Ok(());
    }
    Err(p.bad(format!(
        "{store} could not be read in full ({}); saving would drop what could not be read, so nothing was changed: repair or remove those entries first",
        problems.join("; ")
    )))
}

/// The open document's settings (empty without a document) and what could not be read.
fn doc_settings(s: &Session) -> (DocumentSettings, Vec<String>) {
    s.active().map_or_else(|| (DocumentSettings::default(), Vec::new()), |d| DocumentSettings::read(&d.font))
}

/// The open document's and the user's profiles as `audit.profile.list` lists them, the effective
/// default profile, and what of the stores could not be read.
pub(crate) fn stored_profiles(s: &Session) -> (Vec<Value>, String, Vec<String>) {
    let (doc, mut problems) = doc_settings(s);
    let (user, more) = user_store(s);
    problems.extend(more);
    let (default, _, _) = default_profile(&doc, &user);
    let profiles = Profiles { project: &doc.profiles, user: &user.profiles };
    let mut out = Vec::with_capacity(doc.profiles.len() + user.profiles.len());
    for (store, defs) in [(Store::Project, &doc.profiles), (Store::User, &user.profiles)] {
        for def in defs {
            // A project profile hides a user profile with the same id.
            let shadowed = store == Store::User && doc.profiles.iter().any(|d| d.id == def.id);
            let checks = if shadowed { Value::Null } else { profiles.resolve(&def.id).map_or(Value::Null, |r| json!(r.checks.len())) };
            out.push(json!({
                "id": def.id,
                "label": def.label,
                "checks": checks,
                "builtIn": false,
                "default": !shadowed && def.id == default,
                "store": store.as_str(),
                "shadowed": shadowed,
            }));
        }
    }
    (out, default, problems)
}

/// Stores `settings` in the open document as one undo step.
fn write_settings(s: &mut Session, label: &str, settings: &DocumentSettings) -> Result<()> {
    s.edit(label, |font| {
        settings.write(font);
        Ok(())
    })
}

/// The profile `audit.run` uses when the call names none, where the choice comes from
/// (`document`, `application` or `builtIn`) and a note when a stored default no longer exists.
pub(crate) fn default_profile(doc: &DocumentSettings, user: &UserStore) -> (String, &'static str, Option<String>) {
    let profiles = Profiles { project: &doc.profiles, user: &user.profiles };
    let mut missing = Vec::new();
    if let Some(id) = &doc.default_profile {
        if profiles.store_of(id).is_some() {
            return (id.clone(), "document", None);
        }
        missing.push(format!("the document's default profile `{id}` no longer exists"));
    }
    if let Some(id) = &user.default {
        if profiles.store_of(id).is_some() {
            let note = (!missing.is_empty()).then(|| format!("{}; the application default is used", missing.join("; ")));
            return (id.clone(), "application", note);
        }
        missing.push(format!("the application's default profile `{id}` no longer exists"));
    }
    let note = (!missing.is_empty()).then(|| format!("{}; universal is used", missing.join("; ")));
    (Profile::Universal.as_str().to_string(), "builtIn", note)
}

fn with_problems(mut out: Value, problems: Vec<String>) -> Value {
    if !problems.is_empty()
        && let Value::Object(o) = &mut out
    {
        o.insert("problems".into(), json!(problems));
    }
    out
}

fn profile_json(def: &ProfileDef, store: Store) -> Value {
    let overrides: Vec<Value> = def
        .overrides
        .iter()
        .map(|o| {
            let mut m = Map::new();
            m.insert("check".into(), json!(o.check));
            if let Some(c) = &o.code {
                m.insert("code".into(), json!(c));
            }
            m.insert("severity".into(), json!(o.severity.as_str()));
            Value::Object(m)
        })
        .collect();
    json!({
        "id": def.id,
        "label": def.label,
        "store": store.as_str(),
        "builtIn": false,
        "include": def.include,
        "checks": def.checks,
        "exclude": def.exclude,
        "overrides": overrides,
        "config": def.thresholds,
    })
}

fn builtin_json(p: Profile) -> Value {
    let include: &[&str] = match p {
        Profile::Outlines | Profile::Opentype => &[],
        Profile::Universal => &["outlines", "opentype"],
        Profile::Strict => &["universal"],
    };
    json!({
        "id": p.as_str(),
        "label": p.as_str(),
        "store": Store::BuiltIn.as_str(),
        "builtIn": true,
        "include": include,
        "checks": [],
        "exclude": [],
        "overrides": [],
        "config": {},
    })
}

fn entry_json(e: &IgnoreEntry) -> Value {
    to_json(&e.to_plist(), 0)
}

// ── audit.clearCache ────────────────────────────────────────────────────────────────────────────

fn clear_cache(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.clearCache", v)?;
    match p.opt_str("target")? {
        None | Some("document") => {}
        Some(path) => {
            if path.is_empty() {
                return Err(p.bad("`target` is \"document\" or a file path"));
            }
            return Ok(json!({"cleared": false}));
        }
    }
    let doc = s.doc_mut()?;
    let cleared = doc.audit.take().is_some();
    Ok(json!({"cleared": cleared}))
}

// ── audit.profile.* ─────────────────────────────────────────────────────────────────────────────

fn profile_show(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.profile.show", v)?;
    let id = p.str("profile")?;
    let (doc, mut problems) = doc_settings(s);
    let (user, more) = user_store(s);
    problems.extend(more);
    let profiles = Profiles { project: &doc.profiles, user: &user.profiles };
    let mut out = match Profile::parse(id) {
        Some(b) => builtin_json(b),
        None => {
            let (store, def) = profiles.find(id).ok_or_else(|| p.bad(format!("there is no profile `{id}`")))?;
            profile_json(def, store)
        }
    };
    let resolved = match profiles.resolve(id) {
        Ok(r) => {
            let ids: Vec<&str> = r.checks.iter().map(|c| c.id).collect();
            json!({"count": ids.len(), "checks": ids})
        }
        Err(e) => {
            problems.push(format!("the profile cannot be resolved: {e}"));
            Value::Null
        }
    };
    let (effective, _, _) = default_profile(&doc, &user);
    if let Value::Object(o) = &mut out {
        o.insert("default".into(), json!(effective == id));
        o.insert("resolved".into(), resolved);
    }
    Ok(with_problems(out, problems))
}

fn store_param(p: &Params) -> Result<Option<Store>> {
    match p.opt_str("store")? {
        None => Ok(None),
        Some("user") => Ok(Some(Store::User)),
        Some("project") => Ok(Some(Store::Project)),
        Some(other) => Err(p.bad(format!("`store` must be \"user\" or \"project\", got `{other}`"))),
    }
}

fn profile_save(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.profile.save", v)?;
    let id = p.str("profile")?;
    prof::check_profile_id(id).map_err(|e| p.bad(e))?;
    let store = store_param(&p)?.unwrap_or(Store::User);
    let mut d = Dict::new();
    d.insert("id", PValue::String(Str::new(id)));
    for key in ["label", "include", "checks", "exclude", "overrides", "config"] {
        if let Some(value) = param_plist(&p, key)? {
            d.insert(key, value);
        }
    }
    let def = ProfileDef::from_plist(&PValue::Dict(d)).map_err(|e| p.bad(e))?;
    def.check().map_err(|e| p.bad(e))?;

    let (mut user, user_problems) = user_store(s);
    let (mut doc, doc_problems) = doc_settings(s);
    if store == Store::Project {
        s.doc()?;
    }
    let (mine, other) = match store {
        Store::Project => (&mut doc.profiles, &user.profiles),
        _ => (&mut user.profiles, &doc.profiles),
    };
    if other.iter().any(|q| q.id == id) {
        let there = if store == Store::Project { "the user's profiles hold" } else { "the open document holds a project profile" };
        return Err(p.bad(format!("{there} `{id}`: use another id, or delete that profile first")));
    }
    let created = match mine.iter_mut().find(|q| q.id == id) {
        Some(slot) => {
            *slot = def.clone();
            false
        }
        None => {
            if mine.len() >= prof::MAX_PROFILES {
                return Err(p.bad(format!("the {} store holds {} profiles already", store.as_str(), prof::MAX_PROFILES)));
            }
            mine.push(def.clone());
            true
        }
    };
    let profiles = Profiles { project: &doc.profiles, user: &user.profiles };
    let resolved = profiles.resolve(id).map_err(|e| p.bad(e))?;
    for includer in profiles.includers(store, id) {
        profiles.resolve(includer).map_err(|e| p.bad(format!("profile `{includer}` includes `{id}` and would break: {e}")))?;
    }
    let count = resolved.checks.len();
    match store {
        Store::Project => {
            refuse_unread(&p, DOC_STORE, &doc_problems)?;
            write_settings(s, "Save Audit Profile", &doc)?
        }
        _ => {
            refuse_unread(&p, USER_STORE, &user_problems)?;
            save_user_store(s, &user)?
        }
    }
    Ok(json!({"profile": profile_json(&def, store), "created": created, "checks": count}))
}

fn profile_delete(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.profile.delete", v)?;
    let id = p.str("profile")?;
    if Profile::parse(id).is_some() {
        return Err(p.bad(format!("`{id}` is a built-in profile and cannot be deleted")));
    }
    let (mut user, user_problems) = user_store(s);
    let (mut doc, doc_problems) = doc_settings(s);
    let in_doc = doc.profiles.iter().any(|q| q.id == id);
    let in_user = user.profiles.iter().any(|q| q.id == id);
    let store = match (store_param(&p)?, in_doc, in_user) {
        (Some(Store::Project), true, _) | (None, true, false) => Store::Project,
        (Some(Store::User), _, true) | (None, false, true) => Store::User,
        (None, true, true) => return Err(p.bad(format!("both the open document and the user's profiles hold `{id}`: give `store`"))),
        _ => return Err(p.bad(format!("there is no user or project profile `{id}`"))),
    };
    let includers = Profiles { project: &doc.profiles, user: &user.profiles }.includers(store, id);
    if !includers.is_empty() {
        return Err(p.bad(format!("profiles {} include `{id}`: change them first", includers.join(", "))));
    }
    let default_cleared = match store {
        Store::Project => {
            refuse_unread(&p, DOC_STORE, &doc_problems)?;
            doc.profiles.retain(|q| q.id != id);
            let cleared = doc.default_profile.as_deref() == Some(id);
            if cleared {
                doc.default_profile = None;
            }
            write_settings(s, "Delete Audit Profile", &doc)?;
            cleared
        }
        _ => {
            refuse_unread(&p, USER_STORE, &user_problems)?;
            user.profiles.retain(|q| q.id != id);
            let cleared = user.default.as_deref() == Some(id);
            if cleared {
                user.default = None;
            }
            save_user_store(s, &user)?;
            cleared
        }
    };
    Ok(json!({"deleted": id, "store": store.as_str(), "defaultCleared": default_cleared}))
}

fn profile_set_default(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.profile.setDefault", v)?;
    let clear = matches!(v.get("profile"), Some(Value::Null));
    let id = if clear { None } else { Some(p.str("profile")?) };
    let target = match p.opt_str("target")? {
        None | Some("document") => "document",
        Some("application") => "application",
        Some(other) => return Err(p.bad(format!("`target` must be \"document\" or \"application\", got `{other}`"))),
    };
    if target == "document" {
        s.doc()?;
    }
    let (mut user, user_problems) = user_store(s);
    let (mut doc, doc_problems) = doc_settings(s);
    if let Some(id) = id {
        let profiles = Profiles { project: &doc.profiles, user: &user.profiles };
        let store = profiles.store_of(id).ok_or_else(|| p.bad(format!("there is no profile `{id}`")))?;
        if target == "application" && store == Store::Project {
            return Err(
                p.bad(format!("`{id}` is a project profile of the open document; only built-in and user profiles can be the application default"))
            );
        }
        profiles.resolve(id).map_err(|e| p.bad(e))?;
    }
    if target == "document" {
        refuse_unread(&p, DOC_STORE, &doc_problems)?;
        doc.default_profile = id.map(str::to_string);
        write_settings(s, "Set Default Audit Profile", &doc)?;
    } else {
        refuse_unread(&p, USER_STORE, &user_problems)?;
        user.default = id.map(str::to_string);
        save_user_store(s, &user)?;
    }
    let (effective, _, _) = default_profile(&doc, &user);
    Ok(json!({"profile": id, "target": target, "effective": effective}))
}

// ── audit.config.* ──────────────────────────────────────────────────────────────────────────────

fn value_json(v: &ProjectValue) -> Value {
    to_json(&v.to_plist(), 0)
}

fn config_get(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.config.get", v)?;
    let only = match p.opt_str("check")? {
        None => None,
        Some(id) => Some(audit::find(id).ok_or_else(|| p.bad(format!("there is no check `{id}`")))?),
    };
    let (doc, mut problems) = doc_settings(s);
    let (user, more) = user_store(s);
    problems.extend(more);
    let profiles = Profiles { project: &doc.profiles, user: &user.profiles };
    let (id, from, note) = match p.opt_str("profile")? {
        Some(id) => {
            profiles.store_of(id).ok_or_else(|| p.bad(format!("there is no profile `{id}`")))?;
            (id.to_string(), "call", None)
        }
        None => default_profile(&doc, &user),
    };
    let resolved = profiles.resolve(&id).map_err(|e| p.bad(e))?;
    let mut checks = Map::new();
    for c in CHECKS.iter().filter(|c| only.is_none_or(|o| o.id == c.id) && !c.thresholds.is_empty()) {
        let mut ts = Map::new();
        for t in c.thresholds {
            let in_range = |x: &f64| x.is_finite() && *x >= t.min && *x <= t.max;
            let doc_value = doc.thresholds.get(c.id).and_then(|m| m.get(t.name)).copied().filter(in_range);
            let profile_value = resolved.thresholds.get(c.id).and_then(|m| m.get(t.name)).copied().filter(in_range);
            let (value, from) = match (doc_value, profile_value) {
                (Some(x), _) => (x, "document"),
                (None, Some(x)) => (x, "profile"),
                (None, None) => (t.default, "default"),
            };
            ts.insert(t.name.into(), json!({"value": value, "from": from, "default": t.default, "min": t.min, "max": t.max}));
        }
        checks.insert(c.id.into(), Value::Object(ts));
    }
    let mut values = Map::new();
    for d in PROJECT_VALUES {
        let value = doc.values.get(d.name).map_or(Value::Null, value_json);
        values.insert(d.name.into(), json!({"value": value, "kind": d.kind.name(), "doc": d.doc}));
    }
    let mut out = json!({"profile": id, "profileFrom": from, "checks": checks, "values": values});
    if let (Some(n), Value::Object(o)) = (note, &mut out) {
        o.insert("note".into(), json!(n));
    }
    Ok(with_problems(out, problems))
}

fn config_set(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.config.set", v)?;
    let values = p.opt_object("values")?.ok_or_else(|| p.bad("`values` is required"))?;
    if values.is_empty() {
        return Err(p.bad("`values` names nothing to change"));
    }
    if values.len() > MAX_SET_VALUES {
        return Err(p.bad(format!("`values` holds {} names; at most {MAX_SET_VALUES} are allowed", values.len())));
    }
    let (mut doc, doc_problems) = doc_settings(s);
    refuse_unread(&p, DOC_STORE, &doc_problems)?;
    let mut echo = Map::new();
    let check = p.opt_str("check")?;
    match check {
        Some(id) => {
            let spec = audit::find(id).ok_or_else(|| p.bad(format!("there is no check `{id}`")))?;
            for (name, value) in values {
                if spec.threshold(name).is_none() {
                    let known: Vec<&str> = spec.thresholds.iter().map(|t| t.name).collect();
                    let list = if known.is_empty() { "it has none".to_string() } else { format!("its thresholds: {}", known.join(", ")) };
                    return Err(p.bad(format!("check `{id}` has no threshold `{name}`; {list}")));
                }
                if value.is_null() {
                    if let Some(m) = doc.thresholds.get_mut(id) {
                        m.remove(name);
                    }
                    echo.insert(name.clone(), Value::Null);
                    continue;
                }
                let n = value.as_f64().ok_or_else(|| p.bad(format!("`values.{name}` must be a number or null")))?;
                Config::default().set(id, name, n).map_err(|e| p.bad(e))?;
                doc.thresholds.entry(id.to_string()).or_default().insert(name.clone(), n);
                echo.insert(name.clone(), json!(n));
            }
            if doc.thresholds.get(id).is_some_and(std::collections::BTreeMap::is_empty) {
                doc.thresholds.remove(id);
            }
        }
        None => {
            for (name, value) in values {
                let def = project_value_def(name).ok_or_else(|| {
                    let known: Vec<&str> = PROJECT_VALUES.iter().map(|d| d.name).collect();
                    p.bad(format!("there is no project value `{name}`; known: {} (give `check` to set a check's threshold)", known.join(", ")))
                })?;
                match to_plist(value, 0).map_err(|e| p.bad(format!("`values.{name}`: {e}")))? {
                    None => {
                        doc.values.remove(name);
                        echo.insert(name.clone(), Value::Null);
                    }
                    Some(pv) => {
                        let read = def.read(&pv).map_err(|e| p.bad(e))?;
                        echo.insert(name.clone(), value_json(&read));
                        doc.values.insert(name.clone(), read);
                    }
                }
            }
        }
    }
    write_settings(s, "Change Audit Settings", &doc)?;
    let mut out = Map::new();
    if let Some(id) = check {
        out.insert("check".into(), json!(id));
    }
    out.insert("values".into(), Value::Object(echo));
    Ok(Value::Object(out))
}

// ── The ignore list ─────────────────────────────────────────────────────────────────────────────

/// The table-level part of a result's location.
pub(super) fn result_location(loc: &Map<String, Value>) -> Location {
    let mut out = Location::new();
    for key in LOCATION_KEYS {
        let Some(v) = loc.get(key) else { continue };
        if let Ok(Some(pv)) = to_plist(v, 0)
            && let Ok(lv) = prof::LocValue::from_plist(&pv)
        {
            out.insert(key.to_string(), lv);
        }
    }
    out
}

/// One result of a result document: its check, glyph, master and table-level location.
fn result_target(r: &Value) -> Option<(String, Option<String>, Option<String>, Location)> {
    let check = r.get("check")?.as_str()?.to_string();
    let loc = r.get("location").and_then(Value::as_object);
    let field = |k: &str| loc.and_then(|l| l.get(k)).and_then(Value::as_str).map(str::to_string);
    Some((check, field("glyph"), field("master"), loc.map(result_location).unwrap_or_default()))
}

/// The result document of the kept run, if there is one.
fn kept_results(s: &mut Session) -> Option<Value> {
    s.execute("audit.results", &json!({"includeIgnored": true})).ok()
}

fn ignore_issue(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.ignoreIssue", v)?;
    let mut d = Dict::new();
    match p.opt_str("result")? {
        Some(result) => {
            if let Some(key) = ["check", "glyph", "master", "location"].into_iter().find(|k| p.get(k).is_some()) {
                return Err(p.bad(format!("give `result` or `check` with `{key}`, not both")));
            }
            let report = s.execute("audit.results", &json!({"includeIgnored": true}))?;
            let r = report
                .get("results")
                .and_then(Value::as_array)
                .and_then(|list| list.iter().find(|r| r.get("id").and_then(Value::as_str) == Some(result)))
                .ok_or_else(|| p.bad(format!("the last audit has no result `{result}`")))?;
            let (check, glyph, master, location) = result_target(r).ok_or_else(|| p.bad(format!("result `{result}` names no check")))?;
            d.insert("check", PValue::String(Str::new(check)));
            for (key, value) in [("glyph", glyph), ("master", master)] {
                if let Some(value) = value {
                    d.insert(key, PValue::String(Str::new(value)));
                }
            }
            if !location.is_empty() {
                d.insert("location", prof::location_to_plist(&location));
            }
        }
        None => {
            let check = p.str("check").map_err(|_| p.bad("give `result`, or `check` with an optional glyph, master and location"))?;
            if audit::find(check).is_none() {
                return Err(p.bad(format!("there is no check `{check}`")));
            }
            let font = s.font()?;
            d.insert("check", PValue::String(Str::new(check)));
            if let Some(g) = p.opt_str("glyph")? {
                if font.glyph(g).is_none() {
                    return Err(p.bad(format!("there is no glyph `{g}`")));
                }
                d.insert("glyph", PValue::String(Str::new(g)));
            }
            if let Some(m) = p.opt_str("master")? {
                if font.master(m).is_err() {
                    return Err(p.bad(format!("there is no master `{m}`")));
                }
                d.insert("master", PValue::String(Str::new(m)));
            }
            if let Some(loc) = param_plist(&p, "location")? {
                location_from_plist(&loc).map_err(|e| p.bad(e))?;
                d.insert("location", loc);
            }
        }
    }
    if let Some(reason) = p.opt_str("reason")? {
        d.insert("reason", PValue::String(Str::new(reason)));
    }
    if let Some(date) = s.now().and_then(clock::last_change_text) {
        d.insert("date", PValue::String(Str::new(date)));
    }
    let entry = IgnoreEntry::from_plist(&PValue::Dict(d)).map_err(|e| p.bad(e))?;
    let (mut doc, doc_problems) = doc_settings(s);
    refuse_unread(&p, DOC_STORE, &doc_problems)?;
    let added = match doc.ignored.iter_mut().find(|e| e.same_target(&entry)) {
        Some(slot) => {
            *slot = entry.clone();
            false
        }
        None => {
            if doc.ignored.len() >= prof::MAX_IGNORED {
                return Err(p.bad(format!("the ignore list holds {} entries already", prof::MAX_IGNORED)));
            }
            doc.ignored.push(entry.clone());
            true
        }
    };
    write_settings(s, "Ignore Issue", &doc)?;
    Ok(json!({"entry": entry_json(&entry), "added": added, "entries": doc.ignored.len()}))
}

fn unignore_issue(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.unignoreIssue", v)?;
    let check = p.str("check")?;
    let glyph = p.opt_str("glyph")?;
    let master = p.opt_str("master")?;
    let location = match param_plist(&p, "location")? {
        Some(loc) => Some(location_from_plist(&loc).map_err(|e| p.bad(e))?),
        None => None,
    };
    let (mut doc, doc_problems) = doc_settings(s);
    refuse_unread(&p, DOC_STORE, &doc_problems)?;
    let before = doc.ignored.len();
    doc.ignored.retain(|e| {
        let hit = e.check == check
            && glyph.is_none_or(|g| e.glyph.as_deref() == Some(g))
            && master.is_none_or(|m| e.master.as_deref() == Some(m))
            && location.as_ref().is_none_or(|l| e.location == *l);
        !hit
    });
    let removed = before.saturating_sub(doc.ignored.len());
    if removed == 0 {
        return Err(p.bad(format!("no ignored issue of `{check}` matches")));
    }
    write_settings(s, "Stop Ignoring Issue", &doc)?;
    Ok(json!({"removed": removed, "entries": doc.ignored.len()}))
}

fn list_ignored(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("audit.listIgnored", v)?;
    let (doc, problems) = doc_settings(s);
    let report = if doc.ignored.is_empty() { None } else { kept_results(s) };
    let ran: BTreeSet<String> = report
        .as_ref()
        .and_then(|r| r.get("checks"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|c| c.get("status").and_then(Value::as_str) != Some("skipped"))
                .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let found: Vec<(String, Option<String>, Option<String>, Location)> = report
        .as_ref()
        .and_then(|r| r.get("results"))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(result_target).collect())
        .unwrap_or_default();
    // Results by check, and by check and glyph, so each entry reads only the results it could
    // cover.
    let mut by_check: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut by_glyph: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (i, (c, g, _, _)) in found.iter().enumerate() {
        by_check.entry(c.as_str()).or_default().push(i);
        if let Some(g) = g {
            by_glyph.entry((c.as_str(), g.as_str())).or_default().push(i);
        }
    }
    let entries: Vec<Value> = doc
        .ignored
        .iter()
        .map(|e| {
            let occurs = if report.is_none() || !ran.contains(&e.check) {
                Value::Null
            } else {
                let candidates = match &e.glyph {
                    Some(g) => by_glyph.get(&(e.check.as_str(), g.as_str())),
                    None => by_check.get(e.check.as_str()),
                };
                json!(candidates.into_iter().flatten().filter_map(|&i| found.get(i)).any(|(c, g, m, l)| e.covers(c, g.as_deref(), m.as_deref(), l)))
            };
            let mut j = entry_json(e);
            if let Value::Object(o) = &mut j {
                o.insert("occurs".into(), occurs);
            }
            j
        })
        .collect();
    let out = json!({"entries": entries, "checkedAgainstRun": report.is_some()});
    Ok(with_problems(out, problems))
}
