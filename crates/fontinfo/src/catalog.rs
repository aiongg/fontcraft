//! The custom-parameter catalogue: every parameter FontCraft knows, generated from the
//! inventory's `[[parameter]]` rows by `cargo xtask paramcatalog` into `catalog_data.rs` (checked
//! in; the gate's spec step runs `--check`).
//!
//! An entry gives what the Custom Parameter picker lists and what the value checks
//! ([`crate::value::check`]) need: the name, its other spellings, the value kind, where the
//! parameter may be set, what it affects, the milestone that makes it work, whether it is
//! measured in font units (scaled by [`crate::scale`]), whether it may repeat and the values it
//! takes when the row lists them.
//!
//! Name properties (`familyNames`, `designers`…) are entries too: commands that add one through
//! `font.customParameters.add` write the property instead (`docs/plans/M7.md` § 3).
//!
//! Rows: `font.customParameters.describe`, `customParameters.nameCaseConvention`,
//! `customParameters.shortAndLongNames` (M7.2).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::catalog_data::PARAMS;
use crate::target::Scope;

/// One known custom parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    /// The file-format key, as the reference spells it (`Name Table Entry`, `typoAscender`).
    pub name: &'static str,
    /// Other spellings read as the same parameter: the UFO long names of camelCase keys
    /// (`openTypeOS2TypoAscender`) and format 2's singular name properties (`designer`).
    pub aliases: &'static [&'static str],
    pub kind: ParamKind,
    /// Where it may be set; [`Scopes::ANY`] when the row states no scope.
    pub scopes: Scopes,
    pub effect: Effect,
    /// The milestone whose package makes the parameter take effect (`M7`, `M8`…).
    pub milestone: &'static str,
    /// Measured in font units: scaled with the font.
    pub units: bool,
    /// May appear more than once in one list (Name Table Entry, Filter…).
    pub repeatable: bool,
    /// The values the parameter takes, from the row's `values`: the one value of a number or text,
    /// each item of a list, the `tag` of each meta Table entry. Empty for a free value.
    pub choices: &'static [&'static str],
    /// One line for the picker, in our own words, naming no vendor.
    pub summary: &'static str,
}

/// The kind of value a parameter takes. The inventory's `choice` rows are [`ParamKind::String`];
/// a row's `values` become [`ParamSpec::choices`], checked by [`crate::value::check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamKind {
    Bool,
    Number,
    String,
    /// A list of values (most often strings, one per line in the editor).
    List,
    Dict,
    /// One string per language.
    Localized,
}

impl ParamKind {
    /// The kind as messages name it: "a boolean", "a number"…
    pub fn describe(self) -> &'static str {
        match self {
            ParamKind::Bool => "a boolean",
            ParamKind::Number => "a number",
            ParamKind::String => "a text",
            ParamKind::List => "a list",
            ParamKind::Dict => "a dictionary",
            ParamKind::Localized => "a text per language",
        }
    }
}

/// What a parameter affects (the inventory's `effect` field). A row without one is
/// [`Effect::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    /// A field of a compiled table.
    TableValue,
    /// The glyph set or outlines at export (Remove Glyphs, Filter…).
    ExportFilter,
    /// The `name` table and other name strings.
    Naming,
    Interpolation,
    Hinting,
    Features,
    /// The editor only.
    Ui,
    Other,
}

/// A set of [`Scope`]s. The empty set means every scope (a parameter whose row states none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Scopes(u8);

impl Scopes {
    /// No stated scope: allowed everywhere.
    pub const ANY: Scopes = Scopes(0);
    pub const FONT: Scopes = Scopes(1);
    pub const MASTER: Scopes = Scopes(2);
    pub const INSTANCE: Scopes = Scopes(4);

    /// The union of two sets.
    pub const fn with(self, other: Scopes) -> Scopes {
        Scopes(self.0 | other.0)
    }

    /// The set holding one scope.
    pub const fn of(scope: Scope) -> Scopes {
        match scope {
            Scope::Font => Scopes::FONT,
            Scope::Master => Scopes::MASTER,
            Scope::Instance => Scopes::INSTANCE,
        }
    }

    /// Whether a parameter with these scopes may be set at `scope`.
    pub const fn allows(self, scope: Scope) -> bool {
        self.0 == 0 || self.0 & Scopes::of(scope).0 != 0
    }

    /// Whether the row stated no scope.
    pub const fn is_any(self) -> bool {
        self.0 == 0
    }
}

/// Every entry, in the generator's order (by name).
pub fn all() -> &'static [ParamSpec] {
    PARAMS
}

/// Every spelling (names and aliases) and the index of its entry in [`PARAMS`].
fn index() -> &'static HashMap<&'static str, usize> {
    static INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = HashMap::new();
        for (i, p) in PARAMS.iter().enumerate() {
            for s in std::iter::once(p.name).chain(p.aliases.iter().copied()) {
                map.entry(s).or_insert(i);
            }
        }
        map
    })
}

/// The entry `name` or one of its aliases names. Exact spelling: names that differ only by case are
/// different parameters (`customParameters.nameCaseConvention`).
pub fn lookup(name: &str) -> Option<&'static ParamSpec> {
    index().get(name).and_then(|i| PARAMS.get(*i))
}

/// The canonical spelling of `name` (itself, or the name its alias stands for).
pub fn canonical(name: &str) -> Option<&'static str> {
    lookup(name).map(|p| p.name)
}

/// The name properties among the entries that hold one text for every language; the entries of
/// kind [`ParamKind::Localized`] are the others (`designers`, `familyNames`…). The format keeps
/// these as properties, not as custom parameters (`docs/plans/M7.md` § 3).
const SINGLE_PROPERTIES: &[&str] = &[
    "designerURL",
    "manufacturerURL",
    "licenseURL",
    "versionString",
    "vendorID",
    "uniqueID",
    "postscriptFontName",
    "WWSFamilyName",
    "WWSSubfamilyName",
    "variationsPostScriptNamePrefix",
];

/// Whether `spec` is a name property, and if so whether it holds one text per language
/// (`Some(true)`) or one text for all (`Some(false)`). Adding one through the custom-parameter
/// commands writes the property of that key.
pub fn property(spec: &ParamSpec) -> Option<bool> {
    if spec.kind == ParamKind::Localized {
        Some(true)
    } else if SINGLE_PROPERTIES.contains(&spec.name) {
        Some(false)
    } else {
        None
    }
}

/// The entries whose name, aliases or summary hold `query` (any case), names before summaries,
/// each in catalogue order; with `scope`, only the entries allowed there. An empty query matches
/// every entry.
pub fn search(query: &str, scope: Option<Scope>) -> Vec<&'static ParamSpec> {
    let q = query.trim().to_lowercase();
    let allowed = |p: &&'static ParamSpec| scope.is_none_or(|s| p.scopes.allows(s));
    let in_name = |p: &ParamSpec| std::iter::once(p.name).chain(p.aliases.iter().copied()).any(|s| s.to_lowercase().contains(&q));
    let mut out: Vec<&'static ParamSpec> = PARAMS.iter().filter(allowed).filter(|p| in_name(p)).collect();
    out.extend(PARAMS.iter().filter(allowed).filter(|p| !in_name(p) && p.summary.to_lowercase().contains(&q)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_scopes_and_search() {
        // Filled by `cargo xtask paramcatalog`; an empty table has nothing to check.
        if PARAMS.is_empty() {
            return;
        }
        // Short and long spellings read as one parameter; case matters.
        assert_eq!(canonical("openTypeNameDescription"), Some("descriptions"));
        assert_eq!(canonical("description"), Some("descriptions"));
        assert_eq!(canonical("openTypeOS2TypoAscender"), Some("typoAscender"));
        assert_eq!(canonical("TypoAscender"), None);
        assert_eq!(lookup("Name Table Entry").map(|p| p.repeatable), Some(true));
        assert_eq!(lookup("designers").and_then(property), Some(true));
        assert_eq!(lookup("vendorID").and_then(property), Some(false));
        assert_eq!(lookup("typoAscender").and_then(property), None);
        // Every spelling finds its own entry.
        for p in all() {
            for s in std::iter::once(p.name).chain(p.aliases.iter().copied()) {
                assert_eq!(lookup(s).map(|q| q.name), Some(p.name), "{s}");
            }
        }
        let found = search("openTypeNameDescription", None);
        assert_eq!(found.iter().map(|p| p.name).collect::<Vec<_>>(), ["descriptions"]);
        assert!(search("glyph", Some(Scope::Master)).iter().all(|p| p.scopes.allows(Scope::Master)));
        assert!(search("", Some(Scope::Instance)).len() < all().len());
        assert_eq!(search("", None).len(), all().len());
    }
}
