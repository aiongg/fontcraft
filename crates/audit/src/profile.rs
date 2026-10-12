//! Audit profiles, project values and the ignore list (`docs/spec/21-qa.md` § 2 and § 3).
//!
//! A [`ProfileDef`] is a user or project profile, built the way FontBakery builds its profiles:
//! profiles it includes, checks it adds and excludes (ids or `prefix*` patterns), severity
//! overrides per check or per message code, and threshold defaults. [`Profiles::resolve`] turns a
//! profile id (built-in, project or user) into the checks a run selects, with the overrides and
//! thresholds that apply.
//!
//! [`DocumentSettings`] is what a source keeps under the font user-data key [`USER_DATA_KEY`]:
//! its default profile, threshold overrides, project values ([`PROJECT_VALUES`]), ignore list and
//! project profiles. User data round-trips through `.glyphs`, UFO and the native format, so the
//! settings travel with the source. User profiles live in the application settings; the engine
//! stores them as the same structure in JSON.
//!
//! Everything here comes from files or command parameters, so it is untrusted: every list and
//! text is capped by the limits below. Decoding a whole document's settings drops the entries it
//! cannot read and names each in the problems it returns; decoding one profile or entry fails with
//! the reason.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Number, Str, Value as PValue};
use fontcraft_model::{Font, fc_keys};

use crate::{CHECKS, CheckSpec, Config, Profile, Severity, TOO_MANY_RESULTS, find, matches_pattern};

/// The font user-data key that holds a source's audit settings.
pub const USER_DATA_KEY: &str = fc_keys::AUDIT;
/// Deepest chain of included profiles below the one resolved.
pub const MAX_INCLUDE_DEPTH: usize = 8;
/// Most profile visits one resolution makes (includes reached along several paths count each
/// time), so a lattice of includes cannot make resolution exponential.
pub const MAX_RESOLVE_VISITS: usize = 256;
/// Most entries in a document's ignore list.
pub const MAX_IGNORED: usize = 100_000;
/// Most profiles one store (the user's, or one document's) holds.
pub const MAX_PROFILES: usize = 256;
/// Most profiles one profile includes.
pub const MAX_INCLUDES: usize = 32;
/// Longest profile id, in bytes.
pub const MAX_ID_LEN: usize = 64;
/// Longest profile label, ignore reason or other free text, in bytes.
pub const MAX_TEXT_LEN: usize = 2048;
/// Most entries in a profile's `checks` or `exclude` list.
pub const MAX_PATTERNS: usize = 1024;
/// Longest check id, pattern, message code, glyph name or master id, in bytes.
pub const MAX_NAME_LEN: usize = 256;
/// Most severity overrides in one profile.
pub const MAX_OVERRIDES: usize = 4096;
/// Most threshold values one profile or document sets.
pub const MAX_THRESHOLDS: usize = 4096;
/// Most keys in an ignore entry's location.
pub const MAX_LOCATION_KEYS: usize = 32;
/// Most items in one list value of a location.
pub const MAX_LOCATION_LIST: usize = 64;
/// Most tags in the `allowedTables` project value.
pub const MAX_TAGS: usize = 1024;
/// Largest `warnSize` or `failSize`, in bytes.
pub const MAX_FILE_SIZE: i64 = 1 << 40;

/// Threshold values per check id and threshold name.
pub type ThresholdMap = BTreeMap<String, BTreeMap<String, f64>>;

/// Where a profile is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Store {
    BuiltIn,
    /// The application settings.
    User,
    /// The source's font user data.
    Project,
}

impl Store {
    pub fn as_str(self) -> &'static str {
        match self {
            Store::BuiltIn => "builtIn",
            Store::User => "user",
            Store::Project => "project",
        }
    }
}

/// A severity override: for one message code of a check, or for every code when `code` is `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct SeverityOverride {
    pub check: String,
    pub code: Option<String>,
    pub severity: Severity,
}

/// A user or project profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileDef {
    pub id: String,
    pub label: String,
    /// Profiles whose checks, overrides and thresholds this one starts from, in order.
    pub include: Vec<String>,
    /// Check ids or `prefix*` patterns added.
    pub checks: Vec<String>,
    /// Check ids or `prefix*` patterns removed, after the includes and the additions.
    pub exclude: Vec<String>,
    /// Applied after the included profiles' overrides.
    pub overrides: Vec<SeverityOverride>,
    /// Threshold defaults, over the included profiles' values.
    pub thresholds: ThresholdMap,
}

/// Is `id` usable as the id of a user or project profile? Ids are 1 to [`MAX_ID_LEN`] bytes of
/// ASCII letters, digits, `.`, `_` and `-`, and never a built-in profile's id.
pub fn check_profile_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > MAX_ID_LEN || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
        return Err(format!("a profile id has 1 to {MAX_ID_LEN} letters, digits, `.`, `_` or `-`; got `{}`", clip(id)));
    }
    if Profile::parse(id).is_some() {
        return Err(format!("`{id}` is a built-in profile and cannot be changed"));
    }
    Ok(())
}

/// The first 64 characters of `s`, for messages.
fn clip(s: &str) -> String {
    s.chars().take(64).collect()
}

fn check_text(what: &str, s: &str, max: usize) -> Result<(), String> {
    if s.len() > max {
        return Err(format!("{what} has {} bytes; at most {max} are allowed", s.len()));
    }
    if s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(format!("{what} holds a control character"));
    }
    Ok(())
}

fn check_name(what: &str, s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err(format!("{what} is empty"));
    }
    if s.len() > MAX_NAME_LEN || s.chars().any(char::is_control) {
        return Err(format!("{what} must be 1 to {MAX_NAME_LEN} bytes without control characters"));
    }
    Ok(())
}

/// Do the values of `t` fit the registry: known checks and thresholds, numbers within range?
pub fn check_thresholds(t: &ThresholdMap) -> Result<(), String> {
    to_config(t).map(|_| ())
}

/// The thresholds as a run's [`Config`]. Fails on the first unknown check or threshold, or value
/// out of range.
pub fn to_config(t: &ThresholdMap) -> Result<Config, String> {
    let mut config = Config::default();
    for (check, values) in t {
        for (name, value) in values {
            config.set(check, name, *value)?;
        }
    }
    Ok(config)
}

fn threshold_count(t: &ThresholdMap) -> usize {
    t.values().map(BTreeMap::len).sum()
}

fn check_patterns(what: &str, list: &[String]) -> Result<(), String> {
    if list.len() > MAX_PATTERNS {
        return Err(format!("`{what}` holds {} entries; at most {MAX_PATTERNS} are allowed", list.len()));
    }
    for pat in list {
        check_name(&format!("an entry of `{what}`"), pat)?;
        if !CHECKS.iter().any(|c| matches_pattern(c.id, pat)) {
            return Err(format!("`{what}`: no check matches `{}` (a pattern is an id, or a prefix followed by `*`)", clip(pat)));
        }
    }
    Ok(())
}

impl ProfileDef {
    /// Checks the profile against the limits and the registry: a valid id, known checks, codes
    /// and thresholds, and at least one include or added check. Whether the included profiles
    /// exist is for [`Profiles::resolve`].
    pub fn check(&self) -> Result<(), String> {
        check_profile_id(&self.id)?;
        check_text("`label`", &self.label, MAX_TEXT_LEN)?;
        if self.include.len() > MAX_INCLUDES {
            return Err(format!("`include` holds {} profiles; at most {MAX_INCLUDES} are allowed", self.include.len()));
        }
        let mut seen = BTreeSet::new();
        for inc in &self.include {
            check_name("an entry of `include`", inc)?;
            if *inc == self.id {
                return Err(format!("profile `{}` cannot include itself", self.id));
            }
            if !seen.insert(inc.as_str()) {
                return Err(format!("`include` names `{}` twice", clip(inc)));
            }
        }
        check_patterns("checks", &self.checks)?;
        check_patterns("exclude", &self.exclude)?;
        if self.include.is_empty() && self.checks.is_empty() {
            return Err("a profile must include another profile or add checks".into());
        }
        if self.overrides.len() > MAX_OVERRIDES {
            return Err(format!("`overrides` holds {} entries; at most {MAX_OVERRIDES} are allowed", self.overrides.len()));
        }
        for o in &self.overrides {
            let spec = find(&o.check).ok_or_else(|| format!("`overrides`: there is no check `{}`", clip(&o.check)))?;
            if let Some(code) = &o.code
                && !spec.codes.contains(&code.as_str())
                && !(spec.capped && code == TOO_MANY_RESULTS)
            {
                return Err(format!("`overrides`: check `{}` has no message code `{}`; its codes: {}", spec.id, clip(code), spec.codes.join(", ")));
            }
        }
        if threshold_count(&self.thresholds) > MAX_THRESHOLDS {
            return Err(format!("`config` sets more than {MAX_THRESHOLDS} thresholds"));
        }
        check_thresholds(&self.thresholds)
    }

    /// The profile as a plist dictionary (keys `id`, `label`, `include`, `checks`, `exclude`,
    /// `overrides`, `config`; empty ones left out).
    pub fn to_plist(&self) -> PValue {
        let mut d = Dict::new();
        d.insert("id", text_value(&self.id));
        if !self.label.is_empty() {
            d.insert("label", text_value(&self.label));
        }
        for (key, list) in [("include", &self.include), ("checks", &self.checks), ("exclude", &self.exclude)] {
            if !list.is_empty() {
                d.insert(key, string_array(list));
            }
        }
        if !self.overrides.is_empty() {
            let items = self
                .overrides
                .iter()
                .map(|o| {
                    let mut e = Dict::new();
                    e.insert("check", text_value(&o.check));
                    if let Some(code) = &o.code {
                        e.insert("code", text_value(code));
                    }
                    e.insert("severity", text_value(o.severity.as_str()));
                    PValue::Dict(e)
                })
                .collect();
            d.insert("overrides", array(items));
        }
        if !self.thresholds.is_empty() {
            d.insert("config", thresholds_to_plist(&self.thresholds));
        }
        PValue::Dict(d)
    }

    /// Reads a profile written by [`ProfileDef::to_plist`] (or the same structure converted from
    /// JSON). Fails on a wrong type or a list over its limit; registry checks are
    /// [`ProfileDef::check`]'s.
    pub fn from_plist(v: &PValue) -> Result<ProfileDef, String> {
        let d = v.as_dict().ok_or("a profile must be an object")?;
        let id = req_text(d, "id")?;
        let mut p = ProfileDef { id, ..ProfileDef::default() };
        if let Some(v) = d.get("label") {
            p.label = text(v).ok_or("`label` must be text")?;
            check_text("`label`", &p.label, MAX_TEXT_LEN)?;
        }
        p.include = opt_texts(d, "include", MAX_INCLUDES)?;
        p.checks = opt_texts(d, "checks", MAX_PATTERNS)?;
        p.exclude = opt_texts(d, "exclude", MAX_PATTERNS)?;
        if let Some(v) = d.get("overrides") {
            let items = v.as_array().ok_or("`overrides` must be a list")?;
            if items.items.len() > MAX_OVERRIDES {
                return Err(format!("`overrides` holds {} entries; at most {MAX_OVERRIDES} are allowed", items.items.len()));
            }
            for item in &items.items {
                let e = item.as_dict().ok_or("each entry of `overrides` must be an object {check, code?, severity}")?;
                let check = req_text(e, "check").map_err(|m| format!("`overrides`: {m}"))?;
                check_name("`overrides`: `check`", &check)?;
                let code = match e.get("code") {
                    None => None,
                    Some(c) => {
                        let c = text(c).ok_or("`overrides`: `code` must be text")?;
                        check_name("`overrides`: `code`", &c)?;
                        Some(c)
                    }
                };
                let sev = req_text(e, "severity").map_err(|m| format!("`overrides`: {m}"))?;
                let severity = Severity::parse(&sev).ok_or_else(|| format!("`overrides`: severity `{}` is not fail, warn or info", clip(&sev)))?;
                p.overrides.push(SeverityOverride { check, code, severity });
            }
        }
        if let Some(v) = d.get("config") {
            p.thresholds = thresholds_from_plist(v)?;
        }
        Ok(p)
    }
}

// ── Plist helpers ───────────────────────────────────────────────────────────────────────────────

fn text_value(s: &str) -> PValue {
    PValue::String(Str::new(s))
}

/// Whole numbers are written as integers, as a person would type them.
#[allow(clippy::cast_possible_truncation)]
fn number_value(f: f64) -> PValue {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        return PValue::Number(Number::int(f as i64));
    }
    PValue::Number(Number::float(f).unwrap_or_else(|| Number::int(0)))
}

fn array(items: Vec<PValue>) -> PValue {
    PValue::Array(Array { items, ..Array::default() })
}

fn string_array(list: &[String]) -> PValue {
    array(list.iter().map(|s| text_value(s)).collect())
}

/// Text, also from a bare number token (an OpenStep file can spell a number-like string that
/// way when edited by hand).
fn text(v: &PValue) -> Option<String> {
    match v {
        PValue::String(s) => Some(s.as_str().to_string()),
        PValue::Number(n) => Some(n.spelling().map_or_else(|| n.value().as_f64().to_string(), str::to_string)),
        _ => None,
    }
}

/// A finite number, also from text that reads as one.
fn number(v: &PValue) -> Option<f64> {
    match v {
        PValue::Number(n) => Some(n.value().as_f64()),
        PValue::String(s) => s.as_str().trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

/// A boolean; the OpenStep writer spells one as `1` or `0`.
fn boolean(v: &PValue) -> Option<bool> {
    match v {
        PValue::Bool(b) => Some(*b),
        PValue::Number(n) => match n.value().as_f64() {
            1.0 => Some(true),
            0.0 => Some(false),
            _ => None,
        },
        PValue::String(s) => match s.as_str() {
            "1" | "true" | "YES" => Some(true),
            "0" | "false" | "NO" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn req_text(d: &Dict, key: &str) -> Result<String, String> {
    let v = d.get(key).ok_or_else(|| format!("`{key}` is required"))?;
    text(v).ok_or_else(|| format!("`{key}` must be text"))
}

fn opt_texts(d: &Dict, key: &str, max: usize) -> Result<Vec<String>, String> {
    let Some(v) = d.get(key) else { return Ok(Vec::new()) };
    let items: Vec<&PValue> = match v {
        PValue::Array(a) => a.items.iter().collect(),
        // One string stands for a list of one.
        PValue::String(_) => vec![v],
        _ => return Err(format!("`{key}` must be a list of text")),
    };
    if items.len() > max {
        return Err(format!("`{key}` holds {} entries; at most {max} are allowed", items.len()));
    }
    items
        .into_iter()
        .map(|i| {
            let s = text(i).ok_or_else(|| format!("`{key}` must be a list of text"))?;
            check_name(&format!("an entry of `{key}`"), &s)?;
            Ok(s)
        })
        .collect()
}

fn thresholds_to_plist(t: &ThresholdMap) -> PValue {
    let mut d = Dict::new();
    for (check, values) in t {
        let mut inner = Dict::new();
        for (name, v) in values {
            inner.insert(name.as_str(), number_value(*v));
        }
        d.insert(check.as_str(), PValue::Dict(inner));
    }
    PValue::Dict(d)
}

/// Reads `{checkId: {threshold: number}}`. Fails on a wrong type or more than
/// [`MAX_THRESHOLDS`] values; whether the checks and thresholds exist is [`to_config`]'s.
pub fn thresholds_from_plist(v: &PValue) -> Result<ThresholdMap, String> {
    let d = v.as_dict().ok_or("`config` must be an object {checkId: {threshold: number}}")?;
    let mut out = ThresholdMap::new();
    let mut count = 0usize;
    for (check, values) in &d.entries {
        check_name("a check id in `config`", check.as_str())?;
        let values = values.as_dict().ok_or_else(|| format!("`config.{}` must be an object of threshold values", clip(check.as_str())))?;
        let slot = out.entry(check.as_str().to_string()).or_default();
        for (name, value) in &values.entries {
            count += 1;
            if count > MAX_THRESHOLDS {
                return Err(format!("`config` sets more than {MAX_THRESHOLDS} thresholds"));
            }
            check_name("a threshold name in `config`", name.as_str())?;
            let n = number(value).ok_or_else(|| format!("`config.{}.{}` must be a finite number", clip(check.as_str()), clip(name.as_str())))?;
            slot.insert(name.as_str().to_string(), n);
        }
    }
    Ok(out)
}

// ── Resolution ──────────────────────────────────────────────────────────────────────────────────

/// A profile resolved for a run.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub id: String,
    pub store: Store,
    /// The checks the profile runs, in registry order.
    pub checks: Vec<&'static CheckSpec>,
    /// Every override that applies, the included profiles' first.
    pub overrides: Vec<SeverityOverride>,
    /// The threshold defaults the profile sets, an includer's values over the included ones'.
    pub thresholds: ThresholdMap,
}

impl Resolved {
    /// The severity of a result with message `code` of `check` under this profile: the last
    /// override for that code, else the last override for the whole check, else the registry's.
    pub fn severity(&self, check: &CheckSpec, code: &str) -> Severity {
        let id = check.id;
        let mine: Vec<&SeverityOverride> = self.overrides.iter().filter(|o| o.check == id).collect();
        mine.iter()
            .rev()
            .find(|o| o.code.as_deref() == Some(code))
            .or_else(|| mine.iter().rev().find(|o| o.code.is_none()))
            .map_or_else(|| check.code_severity(code), |o| o.severity)
    }

    /// The profile's thresholds as a run's [`Config`]. Values the registry no longer accepts (a
    /// check removed, a range narrowed) are left out.
    pub fn config(&self) -> Config {
        let mut config = Config::default();
        for (check, values) in &self.thresholds {
            for (name, v) in values {
                let _ = config.set(check, name, *v);
            }
        }
        config
    }
}

/// The user and project profiles a resolution can see. A project profile includes only built-in
/// and project profiles, a user profile only built-in and user profiles, so a source's rules
/// never depend on one person's settings. A project profile hides a user profile of the same id.
#[derive(Clone, Copy, Debug, Default)]
pub struct Profiles<'a> {
    pub project: &'a [ProfileDef],
    pub user: &'a [ProfileDef],
}

struct Walk {
    visits: usize,
    stack: Vec<String>,
    checks: BTreeSet<usize>,
    overrides: Vec<SeverityOverride>,
    thresholds: ThresholdMap,
}

impl<'a> Profiles<'a> {
    fn in_store(&self, store: Store, id: &str) -> Option<&'a ProfileDef> {
        let list = match store {
            Store::Project => self.project,
            Store::User => self.user,
            Store::BuiltIn => return None,
        };
        list.iter().find(|p| p.id == id)
    }

    /// The user or project profile with this id, and where it is kept.
    pub fn find(&self, id: &str) -> Option<(Store, &'a ProfileDef)> {
        self.in_store(Store::Project, id).map(|p| (Store::Project, p)).or_else(|| self.in_store(Store::User, id).map(|p| (Store::User, p)))
    }

    /// Where the profile `id` is kept; `None` when there is none.
    pub fn store_of(&self, id: &str) -> Option<Store> {
        if Profile::parse(id).is_some() { Some(Store::BuiltIn) } else { self.find(id).map(|(s, _)| s) }
    }

    /// Resolves profile `id`: built-in, project or user. Fails on an unknown id, an include that
    /// does not exist where the profile may look, an include cycle, includes nested deeper than
    /// [`MAX_INCLUDE_DEPTH`] or more than [`MAX_RESOLVE_VISITS`] visits.
    pub fn resolve(&self, id: &str) -> Result<Resolved, String> {
        let store = self.store_of(id).ok_or_else(|| format!("there is no profile `{}`", clip(id)))?;
        let mut walk = Walk { visits: 0, stack: Vec::new(), checks: BTreeSet::new(), overrides: Vec::new(), thresholds: ThresholdMap::new() };
        self.walk(id, store, 0, &mut walk)?;
        let checks = CHECKS.iter().enumerate().filter(|(i, _)| walk.checks.contains(i)).map(|(_, c)| c).collect();
        Ok(Resolved { id: id.to_string(), store, checks, overrides: walk.overrides, thresholds: walk.thresholds })
    }

    fn walk(&self, id: &str, store: Store, depth: usize, w: &mut Walk) -> Result<(), String> {
        w.visits += 1;
        if w.visits > MAX_RESOLVE_VISITS {
            return Err(format!("resolving the profile visits more than {MAX_RESOLVE_VISITS} profiles"));
        }
        if let Some(builtin) = Profile::parse(id) {
            w.checks.extend(CHECKS.iter().enumerate().filter(|(_, c)| builtin.includes(c.profile)).map(|(i, _)| i));
            return Ok(());
        }
        if depth > MAX_INCLUDE_DEPTH {
            return Err(format!("profile includes nest deeper than {MAX_INCLUDE_DEPTH}: {} → {id}", w.stack.join(" → ")));
        }
        if w.stack.iter().any(|s| s == id) {
            return Err(format!("profile `{id}` includes itself: {} → {id}", w.stack.join(" → ")));
        }
        let def = self.in_store(store, id).ok_or_else(|| match w.stack.last() {
            Some(parent) => format!("profile `{parent}` includes `{}`, which is not a built-in or {} profile", clip(id), store.as_str()),
            None => format!("there is no profile `{}`", clip(id)),
        })?;
        w.stack.push(id.to_string());
        for inc in &def.include {
            self.walk(inc, store, depth + 1, w)?;
        }
        w.stack.pop();
        for pat in &def.checks {
            w.checks.extend(CHECKS.iter().enumerate().filter(|(_, c)| matches_pattern(c.id, pat)).map(|(i, _)| i));
        }
        for pat in &def.exclude {
            for (i, c) in CHECKS.iter().enumerate() {
                if matches_pattern(c.id, pat) {
                    w.checks.remove(&i);
                }
            }
        }
        w.overrides.extend(def.overrides.iter().cloned());
        for (check, values) in &def.thresholds {
            let slot = w.thresholds.entry(check.clone()).or_default();
            for (name, v) in values {
                slot.insert(name.clone(), *v);
            }
        }
        Ok(())
    }

    /// The profiles of `store` that include `id` directly.
    pub fn includers(&self, store: Store, id: &str) -> Vec<&'a str> {
        let list = match store {
            Store::Project => self.project,
            Store::User => self.user,
            Store::BuiltIn => &[],
        };
        list.iter().filter(|p| p.include.iter().any(|i| i == id)).map(|p| p.id.as_str()).collect()
    }
}

// ── Project values ──────────────────────────────────────────────────────────────────────────────

/// What a project value holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// 1 to 4 printable ASCII characters (OS/2 `achVendID`).
    VendorId,
    /// Text of at most this many bytes.
    Text(usize),
    /// A whole number in this range.
    Integer(i64, i64),
    Bool,
    /// A list of table tags, 1 to 4 printable ASCII characters each.
    TagList,
}

impl ValueKind {
    pub fn name(self) -> &'static str {
        match self {
            ValueKind::VendorId => "vendor id",
            ValueKind::Text(_) => "text",
            ValueKind::Integer(..) => "whole number",
            ValueKind::Bool => "true or false",
            ValueKind::TagList => "list of table tags",
        }
    }
}

/// A house value that checks compare against (`audit.projectValues`).
#[derive(Clone, Copy, Debug)]
pub struct ProjectValueDef {
    pub name: &'static str,
    pub kind: ValueKind,
    pub doc: &'static str,
}

/// The project values a document can set (`docs/spec/21-qa.md` § 2). A check that needs one
/// names it in a `ProjectValue` precondition and is skipped while it is unset.
pub const PROJECT_VALUES: &[ProjectValueDef] = &[
    ProjectValueDef { name: "vendorId", kind: ValueKind::VendorId, doc: "The foundry's vendor id, as OS/2 achVendID must hold it." },
    ProjectValueDef {
        name: "defaultUrl",
        kind: ValueKind::Text(MAX_TEXT_LEN),
        doc: "The foundry's URL, as the vendor URL name record must hold it.",
    },
    ProjectValueDef { name: "fsType", kind: ValueKind::Integer(0, 0xFFFF), doc: "The embedding permissions OS/2 fsType must hold." },
    ProjectValueDef { name: "allowedTables", kind: ValueKind::TagList, doc: "The tables a compiled font may contain; any other table is reported." },
    ProjectValueDef {
        name: "glyphSet",
        kind: ValueKind::Text(MAX_TEXT_LEN),
        doc: "The reference glyph set coverage is compared against: a glyph set name or a file path.",
    },
    ProjectValueDef {
        name: "keepMacNames",
        kind: ValueKind::Bool,
        doc: "Macintosh name records are kept and checked for completeness instead of reported.",
    },
    ProjectValueDef {
        name: "warnSize",
        kind: ValueKind::Integer(0, MAX_FILE_SIZE),
        doc: "A compiled font larger than this many bytes gives a warning.",
    },
    ProjectValueDef { name: "failSize", kind: ValueKind::Integer(0, MAX_FILE_SIZE), doc: "A compiled font larger than this many bytes fails." },
];

/// The project value named `name`.
pub fn project_value_def(name: &str) -> Option<&'static ProjectValueDef> {
    PROJECT_VALUES.iter().find(|d| d.name == name)
}

/// A project value's setting.
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<String>),
}

impl ProjectValue {
    pub fn to_plist(&self) -> PValue {
        match self {
            ProjectValue::Text(s) => text_value(s),
            ProjectValue::Number(n) => number_value(*n),
            ProjectValue::Bool(b) => PValue::Bool(*b),
            ProjectValue::List(l) => string_array(l),
        }
    }
}

fn check_tag(what: &str, s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return Err(format!("{what} must be 1 to 4 printable ASCII characters, got `{}`", clip(s)));
    }
    Ok(())
}

impl ProjectValueDef {
    /// Reads and checks a value of this kind.
    pub fn read(&self, v: &PValue) -> Result<ProjectValue, String> {
        let name = self.name;
        let wrong = || format!("`{name}` must be a {}", self.kind.name());
        match self.kind {
            ValueKind::VendorId => {
                let s = text(v).ok_or_else(wrong)?;
                check_tag(&format!("`{name}`"), &s)?;
                Ok(ProjectValue::Text(s))
            }
            ValueKind::Text(max) => {
                let s = text(v).ok_or_else(wrong)?;
                check_text(&format!("`{name}`"), &s, max)?;
                Ok(ProjectValue::Text(s))
            }
            ValueKind::Integer(min, max) => {
                let n = number(v).ok_or_else(wrong)?;
                #[allow(clippy::cast_precision_loss)]
                let (lo, hi) = (min as f64, max as f64);
                if n.fract() != 0.0 || n < lo || n > hi {
                    return Err(format!("`{name}` must be a whole number from {min} to {max}"));
                }
                Ok(ProjectValue::Number(n))
            }
            ValueKind::Bool => boolean(v).map(ProjectValue::Bool).ok_or_else(wrong),
            ValueKind::TagList => {
                let items = v.as_array().ok_or_else(wrong)?;
                if items.items.len() > MAX_TAGS {
                    return Err(format!("`{name}` holds {} tags; at most {MAX_TAGS} are allowed", items.items.len()));
                }
                let mut out = Vec::with_capacity(items.items.len());
                for i in &items.items {
                    let s = text(i).ok_or_else(wrong)?;
                    check_tag(&format!("a tag of `{name}`"), &s)?;
                    out.push(s);
                }
                Ok(ProjectValue::List(out))
            }
        }
    }
}

// ── The ignore list ─────────────────────────────────────────────────────────────────────────────

/// A value in an ignore entry's location: text, a number, or a list of those (a kerning pair).
#[derive(Clone, Debug, PartialEq)]
pub enum LocValue {
    Text(String),
    Number(f64),
    List(Vec<LocValue>),
}

impl LocValue {
    pub fn to_plist(&self) -> PValue {
        match self {
            LocValue::Text(s) => text_value(s),
            LocValue::Number(n) => number_value(*n),
            LocValue::List(l) => array(l.iter().map(LocValue::to_plist).collect()),
        }
    }

    fn scalar(v: &PValue) -> Option<LocValue> {
        match v {
            PValue::String(s) => Some(LocValue::Text(s.as_str().to_string())),
            PValue::Number(n) => n.value().as_f64().is_finite().then(|| LocValue::Number(n.value().as_f64())),
            PValue::Bool(b) => Some(LocValue::Number(if *b { 1.0 } else { 0.0 })),
            _ => None,
        }
    }

    /// Reads a location value: a string, a number, or a list of at most [`MAX_LOCATION_LIST`]
    /// of those.
    pub fn from_plist(v: &PValue) -> Result<LocValue, String> {
        if let Some(s) = LocValue::scalar(v) {
            if let LocValue::Text(t) = &s {
                check_text("a location value", t, MAX_NAME_LEN)?;
            }
            return Ok(s);
        }
        let a = v.as_array().ok_or("a location value must be text, a number or a list of those")?;
        if a.items.len() > MAX_LOCATION_LIST {
            return Err(format!("a location list holds {} items; at most {MAX_LOCATION_LIST} are allowed", a.items.len()));
        }
        a.items
            .iter()
            .map(|i| match LocValue::scalar(i) {
                Some(LocValue::Text(t)) => check_text("a location value", &t, MAX_NAME_LEN).map(|()| LocValue::Text(t)),
                Some(s) => Ok(s),
                None => Err("a location list holds only text and numbers".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(LocValue::List)
    }
}

/// A location: key (`table`, `field`, `nameId`, `pair`…) to value.
pub type Location = BTreeMap<String, LocValue>;

/// Reads a location object.
pub fn location_from_plist(v: &PValue) -> Result<Location, String> {
    let d = v.as_dict().ok_or("`location` must be an object")?;
    if d.entries.len() > MAX_LOCATION_KEYS {
        return Err(format!("`location` holds {} keys; at most {MAX_LOCATION_KEYS} are allowed", d.entries.len()));
    }
    let mut out = Location::new();
    for (k, v) in &d.entries {
        check_name("a location key", k.as_str())?;
        out.insert(k.as_str().to_string(), LocValue::from_plist(v).map_err(|e| format!("`location.{}`: {e}", clip(k.as_str())))?);
    }
    Ok(out)
}

pub fn location_to_plist(l: &Location) -> PValue {
    let mut d = Dict::new();
    for (k, v) in l {
        d.insert(k.as_str(), v.to_plist());
    }
    PValue::Dict(d)
}

/// One entry of a document's ignore list (`audit.ignoreListStorage`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IgnoreEntry {
    pub check: String,
    /// `None`: every glyph, and results that name no glyph.
    pub glyph: Option<String>,
    /// `None`: every master.
    pub master: Option<String>,
    /// Keys a matching result's location must hold with these values.
    pub location: Location,
    pub reason: Option<String>,
    /// When the entry was made, as text.
    pub date: Option<String>,
}

impl IgnoreEntry {
    /// The same check, glyph, master and location: the entry names the same issue.
    pub fn same_target(&self, other: &IgnoreEntry) -> bool {
        self.check == other.check && self.glyph == other.glyph && self.master == other.master && self.location == other.location
    }

    /// Does the entry hide a result of `check` at this glyph, master and location?
    pub fn covers(&self, check: &str, glyph: Option<&str>, master: Option<&str>, location: &Location) -> bool {
        self.check == check
            && self.glyph.as_deref().is_none_or(|g| glyph == Some(g))
            && self.master.as_deref().is_none_or(|m| master == Some(m))
            && self.location.iter().all(|(k, v)| location.get(k) == Some(v))
    }

    pub fn to_plist(&self) -> PValue {
        let mut d = Dict::new();
        d.insert("check", text_value(&self.check));
        for (key, v) in [("glyph", &self.glyph), ("master", &self.master)] {
            if let Some(v) = v {
                d.insert(key, text_value(v));
            }
        }
        if !self.location.is_empty() {
            d.insert("location", location_to_plist(&self.location));
        }
        for (key, v) in [("reason", &self.reason), ("date", &self.date)] {
            if let Some(v) = v {
                d.insert(key, text_value(v));
            }
        }
        PValue::Dict(d)
    }

    /// Reads an entry. Fails on a missing check id, a wrong type or text over its limit.
    pub fn from_plist(v: &PValue) -> Result<IgnoreEntry, String> {
        let d = v.as_dict().ok_or("an ignore entry must be an object")?;
        let check = req_text(d, "check")?;
        check_name("`check`", &check)?;
        let mut e = IgnoreEntry { check, ..IgnoreEntry::default() };
        if let Some(v) = d.get("glyph") {
            let g = text(v).ok_or("`glyph` must be text")?;
            check_name("`glyph`", &g)?;
            e.glyph = Some(g);
        }
        if let Some(v) = d.get("master") {
            let m = text(v).ok_or("`master` must be text")?;
            check_name("`master`", &m)?;
            e.master = Some(m);
        }
        if let Some(v) = d.get("location") {
            e.location = location_from_plist(v)?;
        }
        if let Some(v) = d.get("reason") {
            let r = text(v).ok_or("`reason` must be text")?;
            check_text("`reason`", &r, MAX_TEXT_LEN)?;
            e.reason = Some(r);
        }
        if let Some(v) = d.get("date") {
            let t = text(v).ok_or("`date` must be text")?;
            check_text("`date`", &t, MAX_NAME_LEN)?;
            e.date = Some(t);
        }
        Ok(e)
    }
}

// ── A document's settings ───────────────────────────────────────────────────────────────────────

/// The audit settings a source keeps in its font user data under [`USER_DATA_KEY`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentSettings {
    /// The profile `audit.run` uses when the call names none.
    pub default_profile: Option<String>,
    /// Threshold overrides, over the profile's values.
    pub thresholds: ThresholdMap,
    pub values: BTreeMap<String, ProjectValue>,
    pub ignored: Vec<IgnoreEntry>,
    pub profiles: Vec<ProfileDef>,
}

impl DocumentSettings {
    pub fn is_empty(&self) -> bool {
        self.default_profile.is_none() && self.thresholds.is_empty() && self.values.is_empty() && self.ignored.is_empty() && self.profiles.is_empty()
    }

    /// The settings `font` keeps, and what could not be read (dropped, one line each).
    pub fn read(font: &Font) -> (DocumentSettings, Vec<String>) {
        match font.info.user_data.get(USER_DATA_KEY) {
            None => (DocumentSettings::default(), Vec::new()),
            Some(v) => DocumentSettings::from_plist(v),
        }
    }

    /// Stores the settings in `font`'s user data; empty settings remove the key.
    pub fn write(&self, font: &mut Font) {
        let root = &mut Arc::make_mut(&mut font.info).user_data.0;
        if self.is_empty() {
            if let Some(i) = root.entries.iter().position(|(k, _)| k.as_str() == USER_DATA_KEY) {
                root.remove_at(i);
            }
        } else {
            root.insert(USER_DATA_KEY, self.to_plist());
        }
    }

    pub fn to_plist(&self) -> PValue {
        let mut d = Dict::new();
        if let Some(p) = &self.default_profile {
            d.insert("defaultProfile", text_value(p));
        }
        if !self.thresholds.is_empty() {
            d.insert("config", thresholds_to_plist(&self.thresholds));
        }
        if !self.values.is_empty() {
            let mut v = Dict::new();
            for (name, value) in &self.values {
                v.insert(name.as_str(), value.to_plist());
            }
            d.insert("values", PValue::Dict(v));
        }
        if !self.ignored.is_empty() {
            d.insert("ignored", array(self.ignored.iter().map(IgnoreEntry::to_plist).collect()));
        }
        if !self.profiles.is_empty() {
            d.insert("profiles", array(self.profiles.iter().map(ProfileDef::to_plist).collect()));
        }
        PValue::Dict(d)
    }

    /// Reads settings written by [`DocumentSettings::to_plist`], dropping what it cannot read.
    pub fn from_plist(v: &PValue) -> (DocumentSettings, Vec<String>) {
        let mut out = DocumentSettings::default();
        let mut problems = Vec::new();
        let Some(d) = v.as_dict() else {
            problems.push(format!("the `{USER_DATA_KEY}` user data is not an object and was ignored"));
            return (out, problems);
        };
        if let Some(v) = d.get("defaultProfile") {
            match text(v) {
                Some(p) if check_name("`defaultProfile`", &p).is_ok() => out.default_profile = Some(p),
                _ => problems.push("`defaultProfile` is not a profile id and was ignored".into()),
            }
        }
        if let Some(v) = d.get("config") {
            match thresholds_from_plist(v) {
                Ok(t) => out.thresholds = t,
                Err(e) => problems.push(format!("the threshold overrides were ignored: {e}")),
            }
        }
        if let Some(v) = d.get("values") {
            match v.as_dict() {
                Some(values) => {
                    for (name, value) in &values.entries {
                        match project_value_def(name.as_str())
                            .ok_or_else(|| format!("there is no project value `{}`", clip(name.as_str())))
                            .and_then(|def| def.read(value))
                        {
                            Ok(pv) => {
                                out.values.insert(name.as_str().to_string(), pv);
                            }
                            Err(e) => problems.push(format!("a project value was ignored: {e}")),
                        }
                    }
                }
                None => problems.push("`values` is not an object and was ignored".into()),
            }
        }
        if let Some(v) = d.get("ignored") {
            match v.as_array() {
                Some(list) => {
                    if list.items.len() > MAX_IGNORED {
                        problems.push(format!("the ignore list holds {} entries; the first {MAX_IGNORED} were kept", list.items.len()));
                    }
                    for (i, item) in list.items.iter().take(MAX_IGNORED).enumerate() {
                        match IgnoreEntry::from_plist(item) {
                            Ok(e) => out.ignored.push(e),
                            Err(e) => problems.push(format!("ignore entry {} was dropped: {e}", i + 1)),
                        }
                    }
                }
                None => problems.push("`ignored` is not a list and was ignored".into()),
            }
        }
        if let Some(v) = d.get("profiles") {
            match v.as_array() {
                Some(list) => {
                    if list.items.len() > MAX_PROFILES {
                        problems.push(format!("the document holds {} profiles; the first {MAX_PROFILES} were kept", list.items.len()));
                    }
                    for (i, item) in list.items.iter().take(MAX_PROFILES).enumerate() {
                        match ProfileDef::from_plist(item).and_then(|p| check_profile_id(&p.id).map(|()| p)) {
                            Ok(p) if out.profiles.iter().any(|q| q.id == p.id) => problems.push(format!("a second profile `{}` was dropped", p.id)),
                            Ok(p) => out.profiles.push(p),
                            Err(e) => problems.push(format!("profile {} was dropped: {e}", i + 1)),
                        }
                    }
                }
                None => problems.push("`profiles` is not a list and was ignored".into()),
            }
        }
        (out, problems)
    }

    /// Is a result of `check` at this glyph, master and location on the ignore list?
    pub fn is_ignored(&self, check: &str, glyph: Option<&str>, master: Option<&str>, location: &Location) -> bool {
        self.ignored.iter().any(|e| e.covers(check, glyph, master, location))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn def(id: &str, include: &[&str], checks: &[&str], exclude: &[&str]) -> ProfileDef {
        ProfileDef {
            id: id.into(),
            include: include.iter().map(|s| s.to_string()).collect(),
            checks: checks.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            ..ProfileDef::default()
        }
    }

    fn ids(r: &Resolved) -> Vec<&'static str> {
        r.checks.iter().map(|c| c.id).collect()
    }

    #[test]
    fn includes_additions_and_exclusions_combine_in_order() {
        let base = def("base", &["outlines"], &[], &["check.outlines.overlap"]);
        let mut top = def("top", &["base"], &["check.outlines.overlap"], &["check.outlines.tiny*"]);
        top.overrides.push(SeverityOverride { check: "check.outlines.collinearLines".into(), code: None, severity: Severity::Fail });
        top.thresholds.entry("check.outlines.tinyContour".into()).or_default().insert("minArea".into(), 40.0);
        let mut base = base;
        base.overrides.push(SeverityOverride {
            check: "check.outlines.collinearLines".into(),
            code: Some("nearly-collinear".into()),
            severity: Severity::Info,
        });
        base.thresholds.entry("check.outlines.tinyContour".into()).or_default().insert("minArea".into(), 10.0);
        let user = [base, top];
        let profiles = Profiles { project: &[], user: &user };

        let r = profiles.resolve("top").unwrap();
        let got = ids(&r);
        let outlines: Vec<&str> = CHECKS.iter().filter(|c| c.profile == Profile::Outlines).map(|c| c.id).collect();
        // Excluded by `base`, added back by `top`.
        assert!(got.contains(&"check.outlines.overlap"));
        assert!(!got.contains(&"check.outlines.tinyContour"));
        assert_eq!(got.len(), outlines.len() - 1);
        // Registry order is kept.
        let order: Vec<&str> = outlines.into_iter().filter(|id| *id != "check.outlines.tinyContour").collect();
        assert_eq!(got, order);

        let collinear = find("check.outlines.collinearLines").unwrap();
        // The code-specific override of the included profile beats the includer's check-wide one.
        assert_eq!(r.severity(collinear, "nearly-collinear"), Severity::Info);
        assert_eq!(r.severity(collinear, "collinear"), Severity::Fail);
        // The includer's threshold wins.
        assert_eq!(r.config().get("check.outlines.tinyContour", "minArea"), Some(40.0));
        assert_eq!(profiles.store_of("top"), Some(Store::User));
        assert_eq!(profiles.store_of("strict"), Some(Store::BuiltIn));
    }

    #[test]
    fn cycles_depth_and_foreign_includes_are_refused() {
        let user = [def("a", &["b"], &[], &[]), def("b", &["a"], &[], &[])];
        let err = Profiles { project: &[], user: &user }.resolve("a").unwrap_err();
        assert!(err.contains("includes itself"), "{err}");

        let chain: Vec<ProfileDef> = (0..=MAX_INCLUDE_DEPTH + 1)
            .map(|i| {
                if i == MAX_INCLUDE_DEPTH + 1 {
                    def(&format!("p{i}"), &["outlines"], &[], &[])
                } else {
                    def(&format!("p{i}"), &[&format!("p{}", i + 1)], &[], &[])
                }
            })
            .collect();
        let err = Profiles { project: &[], user: &chain }.resolve("p0").unwrap_err();
        assert!(err.contains("deeper"), "{err}");
        assert!(Profiles { project: &[], user: &chain }.resolve("p1").is_ok());

        // A project profile cannot include a user profile.
        let user = [def("mine", &["outlines"], &[], &[])];
        let project = [def("house", &["mine"], &[], &[])];
        let err = Profiles { project: &project, user: &user }.resolve("house").unwrap_err();
        assert!(err.contains("not a built-in or project profile"), "{err}");
    }

    #[test]
    fn settings_round_trip_through_plist_and_drop_what_they_cannot_read() {
        let mut s = DocumentSettings { default_profile: Some("house".into()), ..DocumentSettings::default() };
        s.thresholds.entry("check.outlines.tinyContour".into()).or_default().insert("minArea".into(), 12.5);
        s.values.insert("vendorId".into(), ProjectValue::Text("ABCD".into()));
        s.values.insert("keepMacNames".into(), ProjectValue::Bool(true));
        s.values.insert("allowedTables".into(), ProjectValue::List(vec!["glyf".into(), "OS/2".into()]));
        s.ignored.push(IgnoreEntry {
            check: "check.outlines.overlap".into(),
            glyph: Some("A".into()),
            master: Some("m01".into()),
            location: [
                ("pair".to_string(), LocValue::List(vec![LocValue::Text("A".into()), LocValue::Text("V".into())])),
                ("nameId".to_string(), LocValue::Number(1.0)),
            ]
            .into_iter()
            .collect(),
            reason: Some("intended".into()),
            date: Some("2026-10-11 12:00:00 +0000".into()),
        });
        let mut house = def("house", &["universal"], &[], &["check.outlines.overlap"]);
        house.overrides.push(SeverityOverride { check: "check.outlines.overlap".into(), code: Some("crossing".into()), severity: Severity::Fail });
        s.profiles.push(house);

        let (back, problems) = DocumentSettings::from_plist(&s.to_plist());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back, s);

        // A boolean written as 1 by the OpenStep writer reads back as true.
        let def = project_value_def("keepMacNames").unwrap();
        assert_eq!(def.read(&PValue::Number(Number::int(1))).unwrap(), ProjectValue::Bool(true));

        let mut d = s.to_plist().as_dict().unwrap().clone();
        d.insert("ignored", array(vec![text_value("not an entry"), s.ignored[0].to_plist()]));
        d.insert("values", {
            let mut v = Dict::new();
            v.insert("vendorId", text_value("TOO LONG"));
            v.insert("nope", text_value("x"));
            PValue::Dict(v)
        });
        let (back, problems) = DocumentSettings::from_plist(&PValue::Dict(d));
        assert_eq!(back.ignored.len(), 1);
        assert!(back.values.is_empty());
        assert_eq!(problems.len(), 3, "{problems:?}");
    }

    #[test]
    fn ignore_entries_cover_matching_results() {
        let whole_font = IgnoreEntry { check: "check.outlines.overlap".into(), ..IgnoreEntry::default() };
        let glyph = IgnoreEntry { glyph: Some("A".into()), ..whole_font.clone() };
        let field = IgnoreEntry {
            check: "check.names.x".into(),
            location: [("nameId".to_string(), LocValue::Number(1.0))].into_iter().collect(),
            ..IgnoreEntry::default()
        };
        let none = Location::new();
        assert!(whole_font.covers("check.outlines.overlap", Some("B"), Some("m01"), &none));
        assert!(whole_font.covers("check.outlines.overlap", None, None, &none));
        assert!(glyph.covers("check.outlines.overlap", Some("A"), Some("m02"), &none));
        assert!(!glyph.covers("check.outlines.overlap", Some("B"), None, &none));
        assert!(!glyph.covers("check.outlines.tinyContour", Some("A"), None, &none));
        let at = |n: f64| -> Location {
            [("nameId".to_string(), LocValue::Number(n)), ("platform".to_string(), LocValue::Number(3.0))].into_iter().collect()
        };
        assert!(field.covers("check.names.x", None, None, &at(1.0)));
        assert!(!field.covers("check.names.x", None, None, &at(2.0)));
        assert!(!field.covers("check.names.x", None, None, &none));
    }

    #[test]
    fn profile_checks_refuse_what_the_registry_does_not_know() {
        assert!(def("ok", &["universal"], &["check.outlines.*"], &[]).check().is_ok());
        assert!(def("strict", &["universal"], &[], &[]).check().unwrap_err().contains("built-in"));
        assert!(def("bad id", &["universal"], &[], &[]).check().is_err());
        assert!(def("empty", &[], &[], &[]).check().unwrap_err().contains("include"));
        assert!(def("x", &[], &["check.nothing.here"], &[]).check().unwrap_err().contains("no check matches"));
        let mut p = def("x", &["universal"], &[], &[]);
        p.overrides.push(SeverityOverride { check: "check.outlines.overlap".into(), code: Some("no-such-code".into()), severity: Severity::Info });
        assert!(p.check().unwrap_err().contains("message code"));
        let mut p = def("x", &["universal"], &[], &[]);
        p.thresholds.entry("check.outlines.tinyContour".into()).or_default().insert("minArea".into(), -1.0);
        assert!(p.check().is_err());
    }
}
