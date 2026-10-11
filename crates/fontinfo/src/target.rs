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
    // M7.3: the object forms; numbers that are not whole or not positive refused.
    match value {
        Json::String(s) => Ok(TargetRef::Key(s.clone())),
        _ => Err(TargetError::BadReference("a target is an id, a name or an index".into())),
    }
}

/// Reads a `targets` parameter: one target or a list of them, at most [`crate::MAX_INSTANCES`].
pub fn parse_list(value: &Json) -> Result<Vec<TargetRef>, TargetError> {
    // M7.3: lists, with the limit.
    parse(value).map(|t| vec![t])
}

/// The index of the master `target` names in `font.masters`.
pub fn master(font: &Font, target: &TargetRef) -> Result<usize, TargetError> {
    // M7.3: id, name (ambiguity listed), index, key.
    let _ = font;
    Err(TargetError::NotFound { scope: "master", reference: describe(target) })
}

/// The index of the instance `target` names in `font.instances`.
pub fn instance(font: &Font, target: &TargetRef) -> Result<usize, TargetError> {
    // M7.3: id, name (ambiguity listed), index, key.
    let _ = font;
    Err(TargetError::NotFound { scope: "instance", reference: describe(target) })
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
