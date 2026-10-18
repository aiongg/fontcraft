//! What a check reads and when it can run: scope, group and preconditions
//! (`docs/spec/21-qa.md` § 1; rows `audit.preconditions`, `audit.severityOverrides`).

use serde::Serialize;

/// What a check's runner reads. The inventory's `scope` column has three values; the first three
/// scopes here are all `source` there ([`CheckScope::row_scope`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckScope {
    /// One layer of one glyph ([`crate::audit_layer`]).
    Layer,
    /// Every master of one glyph ([`crate::GlyphCtx`]).
    Glyph,
    /// The whole source ([`crate::SourceCtx`]).
    Source,
    /// One compiled font ([`crate::BinaryCtx`]).
    Binary,
    /// Two or more compiled fonts ([`crate::FamilyCtx`]).
    Family,
}

impl CheckScope {
    /// The value of the inventory's `scope` column: `source`, `binary` or `family`.
    pub fn row_scope(self) -> &'static str {
        match self {
            CheckScope::Layer | CheckScope::Glyph | CheckScope::Source => "source",
            CheckScope::Binary => "binary",
            CheckScope::Family => "family",
        }
    }
}

/// The group of a check: the second part of its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckGroup {
    Outlines,
    Components,
    Anchors,
    Glyphset,
    Metrics,
    Names,
    Tables,
    Layout,
    Variations,
    Hinting,
    Kerning,
    Family,
    Files,
    Sources,
    Color,
    Vendor,
    Cid,
}

impl CheckGroup {
    /// Every group, in the order of `docs/spec/21-qa.md` § 1.
    pub const ALL: [CheckGroup; 17] = [
        CheckGroup::Outlines,
        CheckGroup::Components,
        CheckGroup::Anchors,
        CheckGroup::Glyphset,
        CheckGroup::Metrics,
        CheckGroup::Names,
        CheckGroup::Tables,
        CheckGroup::Layout,
        CheckGroup::Variations,
        CheckGroup::Hinting,
        CheckGroup::Kerning,
        CheckGroup::Family,
        CheckGroup::Files,
        CheckGroup::Sources,
        CheckGroup::Color,
        CheckGroup::Vendor,
        CheckGroup::Cid,
    ];

    /// The name in ids and filters (`check.<name>.*`).
    pub fn as_str(self) -> &'static str {
        match self {
            CheckGroup::Outlines => "outlines",
            CheckGroup::Components => "components",
            CheckGroup::Anchors => "anchors",
            CheckGroup::Glyphset => "glyphset",
            CheckGroup::Metrics => "metrics",
            CheckGroup::Names => "names",
            CheckGroup::Tables => "tables",
            CheckGroup::Layout => "layout",
            CheckGroup::Variations => "variations",
            CheckGroup::Hinting => "hinting",
            CheckGroup::Kerning => "kerning",
            CheckGroup::Family => "family",
            CheckGroup::Files => "files",
            CheckGroup::Sources => "sources",
            CheckGroup::Color => "color",
            CheckGroup::Vendor => "vendor",
            CheckGroup::Cid => "cid",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.as_str() == s)
    }
}

/// A fact that must hold for a check to run; otherwise the check is recorded as skipped with
/// [`Precondition::describe`] as the reason. The runner of M12.44 evaluates them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum Precondition {
    /// The compiled font has `glyf` outlines.
    TrueTypeOutlines,
    /// The compiled font has `CFF ` or `CFF2` outlines.
    CffOutlines,
    /// The font is variable (a compiled font with `fvar`, or a source with several masters).
    Variable,
    /// The font is not variable.
    Static,
    /// The compiled font has hints or instructions.
    Hinted,
    /// The compiled font has this table (tag text, `OS/2`).
    Table(&'static str),
    /// The font covers this script (ISO 15924 code).
    ScriptCovered(&'static str),
    /// The project sets this value (`audit.projectValues`).
    ProjectValue(&'static str),
    /// The run allows network access (`audit.networkOptIn`).
    Network,
    /// The target is a source: the check needs data a compiled font does not keep (smooth flags,
    /// metrics keys, alignment zones of a master, backup layers).
    SourceData,
}

impl Precondition {
    /// The skip reason when the fact does not hold.
    pub fn describe(self) -> String {
        match self {
            Precondition::TrueTypeOutlines => "the font has no TrueType outlines".into(),
            Precondition::CffOutlines => "the font has no CFF outlines".into(),
            Precondition::Variable => "the font is not variable".into(),
            Precondition::Static => "the font is variable".into(),
            Precondition::Hinted => "the font has no hints".into(),
            Precondition::Table(tag) => format!("the font has no '{tag}' table"),
            Precondition::ScriptCovered(script) => format!("the font does not cover the {script} script"),
            Precondition::ProjectValue(name) => format!("the project does not set `{name}`"),
            Precondition::Network => "network checks are not allowed in this run".into(),
            Precondition::SourceData => "the check needs data only a source keeps".into(),
        }
    }
}
