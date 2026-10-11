//! Which glyphs Font View shows (L2): the search field, smart-filter rule trees, the glyph
//! predicate language of the Custom rule and of feature-code tokens, list filters and folders,
//! the sidebar's entries and how a selection of them combines, and the glyphs an entry expects
//! that the font lacks (`docs/spec/05-font-view.md` § 6, `docs/spec/12-feature-code.md` § 3).
//!
//! Everything here reads a font and the glyph data and changes nothing. Rule trees, predicates,
//! search terms, filter trees and `Groups*.plist` files are untrusted input: each is read within
//! the limits below and fails with a [`QueryError`], never a panic.
#![forbid(unsafe_code)]

mod filters;
mod predicate;
mod replace;
mod rules;
mod search;
mod sidebar;
#[cfg(test)]
mod tests;

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, GlyphProps, Layer, PropsCache, glyph_props, keeps_alternates};

pub use filters::{
    Filter, add_filter, check_filter_name, filter_at, list_names, move_filter, remove_filter, rename_filter, replace_filter, validate_filters,
};
pub use predicate::{Attr, Expr, LayerAttr, Literal, Op, Pattern, Test, parse_predicate};
pub use replace::Replacement;
pub use rules::{GroupKind, Property, Rule, RuleNode, RuleOp, RuleTree, RuleValue};
pub use search::{Matcher, Search, SearchField};
pub use sidebar::{
    Catalog, DEFAULT_VISIBLE_SCRIPTS, Entry, Expected, GroupEntry, GroupRule, Groups, LanguageIndex, LanguageSet, Section, Selection, SidebarItem,
    check_script_id, parse_groups, parse_path,
};

/// Longest predicate text accepted, in bytes.
pub const MAX_PREDICATE_BYTES: usize = 16 * 1024;
/// Deepest nesting of parentheses and `NOT` in a predicate.
pub const MAX_PREDICATE_DEPTH: usize = 64;
/// Most tokens in one predicate.
pub const MAX_PREDICATE_TOKENS: usize = 4096;
/// Longest regular expression accepted, in bytes.
pub const MAX_PATTERN_BYTES: usize = 1024;
/// Largest compiled regular expression, in bytes (the `regex` crate's size limits).
pub const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Deepest nesting of groups and repetitions in a regular expression.
pub const MAX_REGEX_NESTING: u32 = 64;
/// Longest search term, in bytes.
pub const MAX_SEARCH_BYTES: usize = 1024;
/// Longest text value of a rule (a name, tag or script to compare with), in bytes.
pub const MAX_VALUE_BYTES: usize = 1024;
/// Most rules in one rule tree, groups included.
pub const MAX_RULES: usize = 512;
/// Deepest nesting of rule groups.
pub const MAX_GROUP_DEPTH: usize = 16;
/// Most filters and folders in the Filters section.
pub const MAX_FILTERS: usize = 1024;
/// Deepest nesting of filter folders.
pub const MAX_FOLDER_DEPTH: usize = 8;
/// Longest filter or folder name, in bytes.
pub const MAX_FILTER_NAME_BYTES: usize = 256;
/// Most names in one list filter.
pub const MAX_LIST_NAMES: usize = 20_000;
/// Most segments in a sidebar path.
pub const MAX_PATH_SEGMENTS: usize = 16;
/// Most sidebar entries one query selects.
pub const MAX_ENTRIES: usize = 256;
/// Most language entries a caller may pass in.
pub const MAX_LANGUAGES: usize = 4096;
/// Most characters one language entry may require.
pub const MAX_LANGUAGE_CHARACTERS: usize = 65_536;
/// Most scripts the Languages section may list by choice.
pub const MAX_VISIBLE_SCRIPTS: usize = 256;
/// Largest `Groups*.plist` file read, in bytes.
pub const MAX_GROUPS_BYTES: usize = 4 << 20;
/// Most entries, at all levels, one `Groups*.plist` file may define.
pub const MAX_GROUP_ENTRIES: usize = 4096;
/// Deepest nesting of `subGroup` lists in a `Groups*.plist` file.
pub const MAX_GROUP_NESTING: usize = 8;

/// Why a query, rule, predicate, filter or path was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError(pub String);

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for QueryError {}

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, QueryError> {
    Err(QueryError(message.into()))
}

/// At most 64 characters of untrusted text, for messages.
pub(crate) fn short(text: &str) -> String {
    let mut out: String = text.chars().take(64).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

/// What a query is evaluated against: the font, the glyph data, and the master whose layers the
/// layer rules read (Font View's master). The two count rules read the first master instead.
pub struct Env<'a> {
    pub font: &'a Font,
    pub data: &'a GlyphData,
    master: Option<&'a str>,
    first_master: Option<&'a str>,
    keep_alternates: bool,
    classes: HashMap<String, HashSet<String>>,
    props: Option<&'a PropsCache>,
    out_of_sync: Option<&'a HashSet<String>>,
}

impl<'a> Env<'a> {
    /// `master` is the shown master's id; `None` means the first master.
    pub fn new(font: &'a Font, data: &'a GlyphData, master: Option<&'a str>) -> Result<Env<'a>, QueryError> {
        let first_master = font.masters.first().map(|m| m.id.as_str());
        let master = match master {
            Some(id) => match font.masters.iter().find(|m| m.id == id) {
                Some(m) => Some(m.id.as_str()),
                None => return fail(format!("there is no master `{}`", short(id))),
            },
            None => first_master,
        };
        Ok(Env { font, data, master, first_master, keep_alternates: keeps_alternates(font), classes: HashMap::new(), props: None, out_of_sync: None })
    }

    /// Gives `name in class(X)` the members of each feature class.
    pub fn with_classes(mut self, classes: HashMap<String, HashSet<String>>) -> Self {
        self.classes = classes;
        self
    }

    /// Reads effective properties from `cache` where it has them. A cache made from other glyph
    /// data or another *Keep Alternates* setting is not used.
    pub fn with_props(mut self, cache: &'a PropsCache) -> Self {
        if cache.is_for(self.data, self.keep_alternates) {
            self.props = Some(cache);
        }
        self
    }

    /// Names the glyphs whose metrics keys are out of sync, which `metricsKeysOutOfSync` reads.
    /// The caller computes the set (with `fontcraft_spacing`, a crate beside this one); without
    /// it, no glyph is out of sync.
    pub fn with_out_of_sync(mut self, glyphs: &'a HashSet<String>) -> Self {
        self.out_of_sync = Some(glyphs);
        self
    }

    pub(crate) fn is_out_of_sync(&self, glyph: &str) -> bool {
        self.out_of_sync.is_some_and(|set| set.contains(glyph))
    }

    /// The master the layer rules read.
    pub fn master(&self) -> Option<&'a str> {
        self.master
    }

    pub(crate) fn shown_layer<'g>(&self, glyph: &'g Glyph) -> Option<&'g Layer> {
        self.master.and_then(|m| glyph.layer(m))
    }

    pub(crate) fn first_layer<'g>(&self, glyph: &'g Glyph) -> Option<&'g Layer> {
        self.first_master.and_then(|m| glyph.layer(m))
    }

    pub(crate) fn in_class(&self, class: &str, name: &str) -> bool {
        self.classes.get(class).is_some_and(|members| members.contains(name))
    }
}

/// One glyph under evaluation. Its effective properties are read from the environment's cache,
/// or computed at most once, when a rule first needs them.
pub struct Subject<'a> {
    pub glyph: &'a Glyph,
    props: OnceCell<Arc<GlyphProps>>,
}

impl<'a> Subject<'a> {
    pub fn new(glyph: &'a Glyph) -> Self {
        Subject { glyph, props: OnceCell::new() }
    }

    pub(crate) fn props(&self, env: &Env) -> &GlyphProps {
        self.props.get_or_init(|| {
            env.props.and_then(|c| c.get(self.glyph)).cloned().unwrap_or_else(|| Arc::new(glyph_props(self.glyph, env.data, env.keep_alternates)))
        })
    }
}

/// Whether a global guide shows in `glyph`. `scope` is the guide's rules (built like a smart
/// filter's, compiled with [`RuleTree::compile`]); a guide without rules shows in every glyph
/// (`guides.globalScopeFilter`).
pub fn guide_in_scope(scope: Option<&Expr>, glyph: &Glyph, env: &Env) -> bool {
    scope.is_none_or(|e| e.matches(&Subject::new(glyph), env))
}
