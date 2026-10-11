//! The custom-parameter catalogue: every parameter FontCraft knows, generated from the
//! inventory's `[[parameter]]` rows by `cargo xtask paramcatalog` into `catalog_data.rs` (checked
//! in; the gate's spec step runs `--check`).
//!
//! An entry gives what the Custom Parameter picker lists and what the value checks
//! ([`crate::value::check`]) need: the name, its other spellings, the value kind, where the
//! parameter may be set, what it affects, the milestone that makes it work, whether it is
//! measured in font units (scaled by [`crate::scale`]) and whether it may repeat.
//!
//! Name properties (`familyNames`, `designers`…) are entries too: commands that add one through
//! `font.customParameters.add` write the property instead (`docs/plans/M7.md` § 3).
//!
//! Rows: `font.customParameters.describe`, `customParameters.nameCaseConvention`,
//! `customParameters.shortAndLongNames` (M7.2).

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
    /// One line for the picker, in our own words, naming no vendor.
    pub summary: &'static str,
}

/// The kind of value a parameter takes. The inventory's `choice` rows are [`ParamKind::String`]
/// with their choices checked by [`crate::value::check`].
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

/// The entry `name` or one of its aliases names. Exact spelling: names that differ only by case are
/// different parameters (`customParameters.nameCaseConvention`).
pub fn lookup(name: &str) -> Option<&'static ParamSpec> {
    // M7.2: an index over names and aliases built once.
    PARAMS.iter().find(|p| p.name == name || p.aliases.contains(&name))
}

/// The canonical spelling of `name` (itself, or the name its alias stands for).
pub fn canonical(name: &str) -> Option<&'static str> {
    lookup(name).map(|p| p.name)
}
