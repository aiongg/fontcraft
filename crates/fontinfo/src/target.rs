//! Where a Font Info command acts: a [`Scope`] (the font, masters, instances) and, below the font,
//! the masters or instances it names ([`TargetRef`]).
//!
//! Commands take `scope` and `targets` parameters (`docs/plans/M7.md` § 3 › Commands address
//! targets). Each target is an id, a name or an index; a name two entries share is refused with
//! the ids to choose from ([`TargetError::Ambiguous`]). Several targets get the same change in one
//! undo step, so the resolvers return every index or nothing.
//!
//! Rows: `font.info.get`, `font.info.set` and the other commands with a `scope` parameter
//! (M7.2, M7.3, M7.5, M7.7).

use fontcraft_model::Font;
use serde_json::Value as Json;

use crate::error::TargetError;

/// The level of a font's information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    Font,
    Master,
    Instance,
}

impl Scope {
    /// Every scope, in order.
    pub const ALL: [Scope; 3] = [Scope::Font, Scope::Master, Scope::Instance];

    /// The parameter spelling: `font`, `master` or `instance`.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Font => "font",
            Scope::Master => "master",
            Scope::Instance => "instance",
        }
    }

    /// The scope a `scope` parameter names (`font`, `master`, `instance`; the plurals `masters`
    /// and `instances` too).
    pub fn from_name(name: &str) -> Option<Scope> {
        match name {
            "font" => Some(Scope::Font),
            "master" | "masters" => Some(Scope::Master),
            "instance" | "instances" | "export" | "exports" => Some(Scope::Instance),
            _ => None,
        }
    }
}

/// One master or instance, as a command names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetRef {
    /// `{"id": "…"}`: the master's id, or the instance's (format version 4).
    Id(String),
    /// `{"name": "…"}`: the master's name, or the instance's style name.
    Name(String),
    /// `{"index": n}` or a bare number: the position in the font's list, from 0.
    Index(usize),
    /// A bare string: an id when one matches, else a name.
    Key(String),
}

/// Reads one target: a bare number (index), a bare string ([`TargetRef::Key`]), or an object with
/// exactly one of `id`, `name`, `index`.
pub fn parse(value: &Json) -> Result<TargetRef, TargetError> {
    let index = |v: &Json| -> Result<usize, TargetError> {
        v.as_f64()
            .filter(|n| n.is_finite() && n.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(n))
            .map(|n| n as usize)
            .ok_or_else(|| TargetError::BadReference(format!("an index is a whole number from 0, got {v}")))
    };
    let text = |key: &str, v: &Json| -> Result<String, TargetError> {
        match v.as_str() {
            Some(s) if !s.is_empty() && s.len() <= MAX_REFERENCE_BYTES => Ok(s.to_string()),
            _ => Err(TargetError::BadReference(format!("`{key}` is a non-empty text of at most {MAX_REFERENCE_BYTES} bytes"))),
        }
    };
    match value {
        Json::Number(_) => index(value).map(TargetRef::Index),
        Json::String(_) => text("target", value).map(TargetRef::Key),
        Json::Object(map) => {
            let mut entries = map.iter().filter(|(_, v)| !v.is_null());
            match (entries.next(), entries.next()) {
                (Some((k, v)), None) => match k.as_str() {
                    "id" => text(k, v).map(TargetRef::Id),
                    "name" => text(k, v).map(TargetRef::Name),
                    "index" => index(v).map(TargetRef::Index),
                    other => Err(TargetError::BadReference(format!("a target object has `id`, `name` or `index`, not `{other}`"))),
                },
                _ => Err(TargetError::BadReference("a target object has exactly one of `id`, `name` and `index`".into())),
            }
        }
        _ => Err(TargetError::BadReference("a target is an id, a name or an index".into())),
    }
}

/// Longest id or name a target may give.
const MAX_REFERENCE_BYTES: usize = 1_024;

/// Reads a `targets` parameter: one target or a list of them, at most [`crate::MAX_INSTANCES`].
pub fn parse_list(value: &Json) -> Result<Vec<TargetRef>, TargetError> {
    match value {
        Json::Array(items) => {
            if items.len() > crate::MAX_INSTANCES {
                return Err(TargetError::LimitExceeded { what: "targets", limit: crate::MAX_INSTANCES });
            }
            if items.is_empty() {
                return Err(TargetError::BadReference("`targets` is empty".into()));
            }
            items.iter().map(parse).collect()
        }
        other => parse(other).map(|t| vec![t]),
    }
}

/// The index of the master `target` names in `font.masters`.
pub fn master(font: &Font, target: &TargetRef) -> Result<usize, TargetError> {
    let entries: Vec<(Option<&str>, &str)> = font.masters.iter().map(|m| (Some(m.id.as_str()), m.name.as_str())).collect();
    resolve("master", &entries, target)
}

/// The index of the instance `target` names in `font.instances`. An instance without a style name
/// (format version 4 keeps it among the properties) is named by its `styleNames` default.
pub fn instance(font: &Font, target: &TargetRef) -> Result<usize, TargetError> {
    let entries: Vec<(Option<&str>, &str)> = font.instances.iter().map(|i| (i.id.as_deref(), instance_name(i))).collect();
    resolve("instance", &entries, target)
}

/// The name an instance is addressed and shown by.
pub fn instance_name(instance: &fontcraft_model::Instance) -> &str {
    if instance.name.is_empty() { instance.properties.value("styleNames").unwrap_or("") } else { &instance.name }
}

/// The index `target` names among `entries` (id, name).
fn resolve(scope: &'static str, entries: &[(Option<&str>, &str)], target: &TargetRef) -> Result<usize, TargetError> {
    let not_found = || TargetError::NotFound { scope, reference: describe(target) };
    let by_id = |id: &str| entries.iter().position(|(i, _)| *i == Some(id));
    let by_name = |name: &str| -> Result<usize, TargetError> {
        let found: Vec<usize> = entries.iter().enumerate().filter(|(_, (_, n))| *n == name).map(|(i, _)| i).collect();
        match found.as_slice() {
            [] => Err(not_found()),
            [one] => Ok(*one),
            many => Err(TargetError::Ambiguous {
                scope,
                name: name.to_string(),
                ids: many
                    .iter()
                    .map(|i| match entries.get(*i).and_then(|(id, _)| *id) {
                        Some(id) if !id.is_empty() => id.to_string(),
                        _ => format!("#{i}"),
                    })
                    .collect(),
            }),
        }
    };
    match target {
        TargetRef::Id(id) => by_id(id).ok_or_else(not_found),
        TargetRef::Name(name) => by_name(name),
        TargetRef::Index(i) => (*i < entries.len()).then_some(*i).ok_or_else(not_found),
        TargetRef::Key(key) => match by_id(key) {
            Some(i) => Ok(i),
            None => by_name(key),
        },
    }
}

/// The masters `targets` name, each once, in the order first named. One failing target fails all.
pub fn masters(font: &Font, targets: &[TargetRef]) -> Result<Vec<usize>, TargetError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(targets.len());
    for t in targets {
        let i = master(font, t)?;
        if seen.insert(i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// The instances `targets` name, each once, in the order first named. One failing target fails all.
pub fn instances(font: &Font, targets: &[TargetRef]) -> Result<Vec<usize>, TargetError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(targets.len());
    for t in targets {
        let i = instance(font, t)?;
        if seen.insert(i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// A target as error messages quote it: `` `Bold` ``, `` id `m01` ``, `#2`.
pub fn describe(target: &TargetRef) -> String {
    match target {
        TargetRef::Id(id) => format!("with id `{id}`"),
        TargetRef::Name(n) | TargetRef::Key(n) => format!("`{n}`"),
        TargetRef::Index(i) => format!("#{i}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_model::{Instance, Master};
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn targets_by_id_name_index_and_ambiguity() {
        let mut font = Font::new("T");
        let master = |id: &str, name: &str| Master { id: id.into(), name: name.into(), ..Default::default() };
        font.masters = Arc::new(vec![master("m1", "Light"), master("m2", "Bold"), master("m3", "Bold")]);
        font.instances = Arc::new(vec![Instance { name: "Regular".into(), ..Default::default() }]);
        let one = |v| parse(&v).and_then(|t| super::master(&font, &t));
        assert_eq!(one(json!("m2")), Ok(1));
        assert_eq!(one(json!("Light")), Ok(0));
        assert_eq!(one(json!({"index": 2})), Ok(2));
        assert_eq!(one(json!(0)), Ok(0));
        assert_eq!(one(json!({"id": "Light"})), Err(TargetError::NotFound { scope: "master", reference: "with id `Light`".into() }));
        match one(json!({"name": "Bold"})) {
            Err(TargetError::Ambiguous { ids, .. }) => assert_eq!(ids, ["m2", "m3"]),
            other => panic!("{other:?}"),
        }
        assert!(one(json!(3)).is_err() && one(json!(-1)).is_err() && one(json!(1.5)).is_err() && one(json!({"id": "m1", "name": "x"})).is_err());
        let list = parse_list(&json!(["m1", {"index": 1}, "m1"])).unwrap();
        assert_eq!(masters(&font, &list), Ok(vec![0, 1]));
        assert!(parse_list(&json!([])).is_err());
        assert_eq!(parse(&json!("Regular")).and_then(|t| instance(&font, &t)), Ok(0));
    }
}
