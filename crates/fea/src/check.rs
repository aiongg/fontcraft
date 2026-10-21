//! Checks on fea-rs's parse tree before `validate` and `compile` run.
//!
//! Two kinds of check. Growth: glyph classes, ligature sequences and enumerated pairs expand
//! to the product or sum of their members, and fea-rs builds them all; the sizes are added up
//! here with saturation and refused over [`Limits`]. Known panics: input fea-rs's validation
//! lets through and its compiler then panics on (a number outside 16 bits where it reads a
//! signed 16-bit value, a CID over 65,535, an empty `table head`, a `size` feature whose
//! subfamily and menu names disagree, a character variant character beyond Unicode, a Windows
//! name string whose escapes are not valid UTF-16, a contextual rule that calls a named lookup
//! with no rules, more lookups, features, mark filtering sets or character variant entries
//! than a 16-bit index holds). Each has a test in `rows_tests.rs`.
//!
//! The tree is one tree for all files, so its offsets are not file offsets; a refusal is placed
//! by finding the statement's text in the files.
//!
//! `range_size` follows fea-rs 1.0.0's `compile/glyph_range.rs` (MIT OR Apache-2.0), which
//! decides which part of two glyph names a range runs over.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use fea_rs::{Kind, Node, NodeOrToken, Token};

use crate::{Diagnostic, Limits, Severity};

/// Most refusals reported from one walk.
const MAX_REFUSALS: usize = 20;
/// Deepest tree the walk descends; the nesting check keeps real trees far shallower.
const MAX_TREE_DEPTH: usize = 1_024;
/// Longest piece of statement text used to place a refusal.
const SNIPPET_BYTES: usize = 160;
/// Most lookups in one table, features, mark filtering sets, and named parameters or characters
/// in one `cvParameters` block: fea-rs stores each index or count in 16 bits and panics past it.
const MAX_INDEX: usize = 65_535;

pub(crate) fn tree(root: &Node, glyph_count: usize, limits: &Limits, files: &BTreeMap<String, Arc<str>>) -> Vec<Diagnostic> {
    let mut walk = Walk {
        limits,
        glyph_count,
        classes: HashMap::new(),
        class_total: 0,
        rules: 0,
        name_ids: HashSet::new(),
        elided_names: Vec::new(),
        empty_lookups: HashSet::new(),
        lookups: [0; 2],
        features: 0,
        filter_sets: 0,
        nested: false,
        flag_ids: HashMap::new(),
        flag_count: 0,
        refusals: Vec::new(),
    };
    walk.node(root, 0);
    // fea-rs panics when STAT's fallback name id is not one the code itself defines.
    for (node, id) in std::mem::take(&mut walk.elided_names) {
        if !walk.name_ids.contains(&id) {
            walk.refuse(&node, format!("ElidedFallbackNameID {id} must name a record of the code's `table name` block"));
        }
    }
    walk.refusals.into_iter().map(|(snippet, message)| place(files, &snippet, message)).collect()
}

struct Walk<'a> {
    limits: &'a Limits,
    glyph_count: usize,
    classes: HashMap<String, usize>,
    /// Glyphs all class literals and class references expand to, saturating.
    class_total: usize,
    /// Rules after expansion, saturating.
    rules: usize,
    /// Name ids the code's `table name` block defines.
    name_ids: HashSet<u32>,
    /// `ElidedFallbackNameID` statements and the id each names.
    elided_names: Vec<(Node, u32)>,
    /// Named lookup blocks with no rule: fea-rs gives them no lookup index, and a contextual
    /// rule that calls one makes its compiler panic.
    empty_lookups: HashSet<String>,
    /// At least as many lookups as fea-rs builds, in GSUB and in GPOS.
    lookups: [usize; 2],
    /// At least as many feature records as fea-rs builds.
    features: usize,
    /// `lookupflag` statements with a mark filtering set.
    filter_sets: usize,
    /// Set while the walk enters a lookup block written directly in a feature block: the
    /// feature's count covers its rules, with the flags and language system it inherits.
    nested: bool,
    /// A number for each distinct `flag_key`; flags the tree does not show get a number of
    /// their own each time.
    flag_ids: HashMap<String, usize>,
    flag_count: usize,
    refusals: Vec<(String, String)>,
}

impl Walk<'_> {
    fn refuse(&mut self, node: &Node, message: String) {
        if self.refusals.len() < MAX_REFUSALS {
            self.refusals.push((snippet(node), message));
        }
    }

    fn node(&mut self, node: &Node, depth: usize) {
        let nested = std::mem::take(&mut self.nested);
        if depth > MAX_TREE_DEPTH {
            self.refuse(node, "the feature code is nested too deeply".into());
            return;
        }
        match node.kind() {
            Kind::GlyphClassDefNode => return self.class_def(node),
            Kind::MarkClassNode => return self.mark_class(node),
            Kind::GlyphClass => {
                let size = self.size(&NodeOrToken::Node(node.clone()));
                self.add_class_total(node, size.unwrap_or(0));
                self.tokens_in(node);
                return;
            }
            Kind::HeadTableNode => {
                if !node.iter_children().any(|c| c.kind() == Kind::HeadFontRevisionNode) {
                    self.refuse(node, "a `table head` block needs a FontRevision statement".into());
                }
            }
            Kind::FeatureNode => {
                self.size_feature(node);
                self.count_features(node);
            }
            Kind::AaltFeatureNode => {
                self.count_features(node);
                // aalt builds a single and an alternate substitution lookup at most, and its
                // rules go into those, so they are not counted as rules of a block.
                let holds_rule = node.iter_children().filter_map(NodeOrToken::as_node).any(|n| is_rule(n.kind()));
                self.add_lookups(node, false, usize::from(holds_rule));
            }
            Kind::ScriptNode | Kind::LanguageNode => self.count_features(node),
            Kind::LookupFlagNode if node.iter_tokens().any(|t| t.kind == Kind::UseMarkFilteringSetKw) => {
                self.filter_sets = self.filter_sets.saturating_add(1);
                if self.filter_sets == MAX_INDEX + 1 {
                    self.refuse(node, format!("the feature code uses more than {MAX_INDEX} mark filtering sets"));
                }
            }
            Kind::CvParametersKw => self.cv_parameters(node),
            Kind::NameSpecNode => self.name_spec(node),
            Kind::CharacterKw => self.character(node),
            Kind::ParametersNode => self.size_parameters(node),
            Kind::NameRecordNode => {
                if let Some(id) = node.iter_tokens().find_map(number) {
                    self.name_ids.insert(id);
                }
            }
            Kind::StatElidedFallbackNameNode => {
                if let Some(id) = node.iter_tokens().find_map(number) {
                    self.elided_names.push((node.clone(), id));
                }
            }
            Kind::LookupBlockNode => {
                let holds_rule = node.iter_children().filter_map(NodeOrToken::as_node).any(|n| has_rule(n, 0));
                let label = node.iter_children().find(|c| c.kind() == Kind::Label).and_then(NodeOrToken::token_text);
                if let (false, Some(label)) = (holds_rule, label) {
                    self.empty_lookups.insert(label.to_string());
                }
            }
            kind if is_rule(kind) => {
                self.rule(node);
                self.calls(node);
            }
            _ => {}
        }
        if !is_rule(node.kind()) && node.kind() != Kind::AaltFeatureNode && !nested {
            self.count_lookups(node);
        }
        for child in node.iter_children() {
            match child {
                NodeOrToken::Node(n) => {
                    self.nested = node.kind() == Kind::FeatureNode && n.kind() == Kind::LookupBlockNode;
                    self.node(n, depth + 1);
                }
                NodeOrToken::Token(t) => self.token(node, t),
            }
        }
    }

    fn token(&mut self, parent: &Node, token: &Token) {
        match token.kind {
            Kind::NamedGlyphClass => {
                let size = self.classes.get(token.text.as_str()).copied().unwrap_or(0);
                self.add_class_total(parent, size);
            }
            // fea-rs reads a cvParameters character as 32 bits (`character` bounds it) and an
            // OS/2 number field as unsigned 16 bits; every other number as signed 16 bits.
            Kind::Number => match parent.kind() {
                Kind::CharacterKw => {}
                Kind::NumberValueNode if token.text.parse::<u16>().is_err() => {
                    self.refuse(parent, format!("the number {} is outside 0 to 65535", token.text));
                }
                Kind::NumberValueNode => {}
                _ if token.text.parse::<i16>().is_err() => {
                    self.refuse(parent, format!("the number {} is outside -32768 to 32767, the range the feature compiler reads here", token.text));
                }
                _ => {}
            },
            Kind::Cid if token.text.parse::<u16>().is_err() => {
                self.refuse(parent, format!("the CID {} is over 65535", token.text));
            }
            _ => {}
        }
    }

    /// Number and CID checks for the tokens below a node the walk does not descend into.
    fn tokens_in(&mut self, node: &Node) {
        for token in node.iter_tokens() {
            if matches!(token.kind, Kind::Number | Kind::Cid) {
                self.token(node, token);
            }
        }
    }

    fn add_class_total(&mut self, node: &Node, size: usize) {
        let before = self.class_total;
        self.class_total = self.class_total.saturating_add(size);
        if before <= self.limits.class_members && self.class_total > self.limits.class_members {
            self.refuse(node, format!("glyph classes expand to more than {} glyphs in all", self.limits.class_members));
        }
    }

    fn class_def(&mut self, node: &Node) {
        self.tokens_in(node);
        let mut children = node.iter_children().filter(|c| c.kind() != Kind::Whitespace && c.kind() != Kind::Comment);
        let Some(name) = children.next().and_then(NodeOrToken::token_text).map(str::to_string) else {
            return;
        };
        let size = children.filter_map(|c| self.size(c)).fold(0usize, usize::saturating_add);
        self.add_class_total(node, size);
        self.classes.insert(name, size);
    }

    fn mark_class(&mut self, node: &Node) {
        self.tokens_in(node);
        let items: Vec<&NodeOrToken> = node.iter_children().filter(|c| is_glyph_item(c.kind())).collect();
        let (Some(glyphs), Some(name)) = (items.first(), items.last().and_then(|c| c.token_text())) else {
            return;
        };
        if items.len() < 2 {
            return;
        }
        let size = self.size(glyphs).unwrap_or(0);
        self.add_class_total(node, size);
        let entry = self.classes.entry(name.to_string()).or_insert(0);
        *entry = entry.saturating_add(size);
    }

    /// How many glyphs an item expands to; `None` for anything that is not a glyph item.
    fn size(&self, item: &NodeOrToken) -> Option<usize> {
        match item {
            NodeOrToken::Token(t) => match t.kind {
                Kind::GlyphName | Kind::Cid => Some(1),
                Kind::NamedGlyphClass => Some(self.classes.get(t.text.as_str()).copied().unwrap_or(0)),
                _ => None,
            },
            NodeOrToken::Node(n) => match n.kind() {
                Kind::GlyphClass => Some(n.iter_children().filter_map(|c| self.size(c)).fold(0usize, usize::saturating_add)),
                Kind::GlyphRange => Some(range_size(n)),
                Kind::GlyphsPredicateNode => Some(self.glyph_count),
                _ => None,
            },
        }
    }

    fn rule(&mut self, node: &Node) {
        let expansion = match node.kind() {
            Kind::GsubType4 => {
                let items: Vec<&NodeOrToken> =
                    node.iter_children().take_while(|c| c.kind() != Kind::ByKw).filter(|c| is_glyph_item(c.kind())).collect();
                self.sequence(node, &items)
            }
            Kind::GsubType6 if node.iter_children().any(|c| c.kind() == Kind::InlineSubNode) => {
                let marked: Vec<&NodeOrToken> = node
                    .iter_children()
                    .filter_map(NodeOrToken::as_node)
                    .filter(|n| n.kind() == Kind::ContextSequence)
                    .flat_map(|n| n.iter_children().filter_map(NodeOrToken::as_node))
                    .filter_map(|g| g.iter_children().find(|c| is_glyph_item(c.kind())))
                    .collect();
                self.sequence(node, &marked)
            }
            Kind::GposType2 if node.iter_children().any(|c| c.kind() == Kind::EnumKw) => {
                let items: Vec<&NodeOrToken> = node.iter_children().filter(|c| is_glyph_item(c.kind())).take(2).collect();
                self.sequence(node, &items)
            }
            _ => node.iter_children().find_map(|c| self.size(c)).unwrap_or(1),
        };
        let before = self.rules;
        self.rules = self.rules.saturating_add(expansion.max(1));
        if before <= self.limits.rules && self.rules > self.limits.rules {
            self.refuse(node, format!("the feature code expands to more than {} rules", self.limits.rules));
        }
    }

    /// Counts the lookups a block's own rules start, from above, as fea-rs 1.0.0's
    /// `compile_ctx.rs` groups them. A rule joins the current lookup only when it is of the
    /// same kind (an `ignore` rule is contextual; single and multiple substitutions share one)
    /// under lookup flags known to be the same. The current lookup ends at a `language`
    /// statement, a `script` statement that changes the language system, and a nested lookup
    /// block; a lookup reference, a `subtable` break or a class definition does not end it.
    /// The inline substitutions and value records of a contextual lookup share anonymous
    /// lookups until a `subtable` break, so each part with one counts once. A lookup block
    /// written in a feature block is counted with the feature: it starts with the feature's
    /// flags and language system, and the flags it sets stay in force after it. Where the tree
    /// does not show fea-rs's flags (a `script` statement outside a feature, a flag number that
    /// does not parse), they count as different from all others: the count may then exceed
    /// fea-rs's by a lookup there, but is never below it, so fea-rs never passes a 16-bit index.
    fn count_lookups(&mut self, block: &Node) {
        let (flags, system) = match block.kind() {
            Kind::FeatureNode => (self.known_flags("0".into()), System::Default),
            // fea-rs clears the flags when a lookup block outside a feature starts.
            Kind::LookupBlockNode => (self.known_flags("0".into()), System::Unknown),
            _ => (self.unknown_flags(), System::Unknown),
        };
        self.count_block(block, flags, system);
    }

    /// The number of the flags a `flag_key` stands for.
    fn known_flags(&mut self, key: String) -> usize {
        if let Some(&id) = self.flag_ids.get(&key) {
            return id;
        }
        let id = self.unknown_flags();
        self.flag_ids.insert(key, id);
        id
    }

    /// A number for flags the tree does not show, equal to no other.
    fn unknown_flags(&mut self) -> usize {
        self.flag_count = self.flag_count.saturating_add(1);
        self.flag_count
    }

    /// Counts a block's lookups from the flags and language system in force when it starts,
    /// and returns those in force at its end.
    fn count_block(&mut self, block: &Node, mut flags: usize, mut system: System) -> (usize, System) {
        let in_feature = block.kind() == Kind::FeatureNode;
        // The current lookup's kind and flags, and whether its part since the last `subtable`
        // break has an anonymous lookup.
        let mut current: Option<(Kind, usize)> = None;
        let mut anonymous = false;
        for child in block.iter_children().filter_map(NodeOrToken::as_node) {
            let kind = child.kind();
            match kind {
                Kind::LookupFlagNode => {
                    flags = match flag_key(child) {
                        Some(key) => self.known_flags(key),
                        None => self.unknown_flags(),
                    };
                    continue;
                }
                Kind::SubtableNode => {
                    anonymous = false;
                    continue;
                }
                Kind::LanguageNode => {
                    current = None;
                    let dflt = tag_text(child).is_some_and(|t| t == "dflt");
                    system = match system {
                        System::Unknown => System::Unknown,
                        System::Default => System::Set("DFLT".into(), dflt),
                        System::Set(script, _) => System::Set(script, dflt),
                    };
                    continue;
                }
                Kind::ScriptNode => {
                    let script = tag_text(child).unwrap_or_default();
                    // fea-rs ignores `script X` while the system is already X with the default
                    // language, and otherwise ends the lookup and clears the flags.
                    match &system {
                        System::Set(s, true) if *s == script => {}
                        System::Unknown => {
                            current = None;
                            flags = self.unknown_flags();
                        }
                        _ => {
                            current = None;
                            flags = self.known_flags("0".into());
                        }
                    }
                    system = System::Set(script, true);
                    continue;
                }
                Kind::LookupBlockNode => {
                    current = None;
                    if in_feature {
                        (flags, system) = self.count_block(child, flags, system);
                    } else {
                        flags = self.unknown_flags();
                    }
                    continue;
                }
                _ if !is_rule(kind) => continue,
                _ => {}
            }
            let group = match kind {
                Kind::GsubIgnore => Kind::GsubType6,
                Kind::GposIgnore => Kind::GposType8,
                Kind::GsubType1 => Kind::GsubType2,
                other => other,
            };
            let joins = current.is_some_and(|(k, f)| k == group && f == flags);
            let mut added = 0;
            if !joins {
                added += 1;
                current = Some((group, flags));
                anonymous = false;
            }
            let inline = match kind {
                Kind::GsubType6 => child.iter_children().any(|c| c.kind() == Kind::InlineSubNode),
                Kind::GposType8 => has_kind(child, Kind::ValueRecordNode, 0),
                _ => false,
            };
            if inline && !anonymous {
                added += 1;
                anonymous = true;
            }
            self.add_lookups(child, is_gpos(kind), added);
        }
        (flags, system)
    }

    fn add_lookups(&mut self, node: &Node, gpos: bool, added: usize) {
        let Some(count) = self.lookups.get_mut(usize::from(gpos)) else {
            return;
        };
        let before = *count;
        *count = count.saturating_add(added);
        if before <= MAX_INDEX && *count > MAX_INDEX {
            let table = if gpos { "GPOS" } else { "GSUB" };
            self.refuse(node, format!("the feature code makes more than {MAX_INDEX} {table} lookups"));
        }
    }

    fn count_features(&mut self, node: &Node) {
        self.features = self.features.saturating_add(1);
        if self.features == MAX_INDEX + 1 {
            self.refuse(node, format!("the feature code makes more than {MAX_INDEX} features"));
        }
    }

    fn cv_parameters(&mut self, node: &Node) {
        let blocks = || node.iter_children().filter_map(NodeOrToken::as_node);
        let labels = blocks().filter(|n| n.kind() == Kind::CvParamsNameNode && n.iter_tokens().any(|t| t.kind == Kind::ParamUiLabelNameIdKw)).count();
        let characters = blocks().filter(|n| n.kind() == Kind::CharacterKw).count();
        if labels > MAX_INDEX || characters > MAX_INDEX {
            self.refuse(node, format!("a `cvParameters` block may have at most {MAX_INDEX} named parameters and {MAX_INDEX} characters"));
        }
    }

    /// Refuses a rule's `lookup NAME` calls to a lookup block with no rules.
    fn calls(&mut self, node: &Node) {
        let mut tokens = node.iter_tokens().filter(|t| !matches!(t.kind, Kind::Whitespace | Kind::Comment));
        while let Some(token) = tokens.next() {
            if token.kind != Kind::LookupKw {
                continue;
            }
            let Some(name) = tokens.next().filter(|t| t.kind == Kind::Ident) else {
                continue;
            };
            if self.empty_lookups.contains(name.text.as_str()) {
                let message = format!("this rule calls the lookup `{}`, which has no rules; add a rule to it or remove the call", name.text);
                self.refuse(node, message);
            }
        }
    }

    /// A sequence whose members' classes multiply: a ligature or an enumerated pair.
    fn sequence(&mut self, node: &Node, items: &[&NodeOrToken]) -> usize {
        if items.len() > self.limits.ligature_components {
            self.refuse(node, format!("a ligature may have at most {} components", self.limits.ligature_components));
            return 1;
        }
        let product = items.iter().map(|c| self.size(c).unwrap_or(1)).fold(1usize, usize::saturating_mul);
        if product > self.limits.rule_sequences {
            self.refuse(node, format!("this rule expands to more than {} sequences", self.limits.rule_sequences));
        }
        product
    }

    /// fea-rs requires menu names exactly when the subfamily identifier is not zero.
    fn size_feature(&mut self, node: &Node) {
        let is_size = node.iter_children().find(|c| c.kind() == Kind::Tag).and_then(NodeOrToken::token_text) == Some("size");
        if !is_size {
            return;
        }
        let blocks = || node.iter_children().filter_map(NodeOrToken::as_node);
        let names = blocks().filter(|n| n.kind() == Kind::SizeMenuNameNode).count();
        for params in blocks().filter(|n| n.kind() == Kind::ParametersNode) {
            let subfamily = params.iter_tokens().filter(|t| matches!(t.kind, Kind::Number | Kind::Float)).nth(1);
            let nonzero = subfamily.is_some_and(|t| t.text.parse::<f64>().is_ok_and(|v| v != 0.0));
            if nonzero && names == 0 {
                self.refuse(node, "a `size` feature with a subfamily identifier needs a sizemenuname".into());
            } else if !nonzero && names > 0 {
                self.refuse(node, "a `size` feature with subfamily identifier 0 may not have a sizemenuname".into());
            }
        }
    }

    /// Windows name strings are UTF-16 written as `\XXXX` escapes; fea-rs panics on a lone
    /// surrogate.
    fn name_spec(&mut self, node: &Node) {
        let platform = node.iter_children().find(|c| matches!(c.kind(), Kind::Number | Kind::Hex | Kind::Octal)).and_then(NodeOrToken::token_text);
        if platform == Some("1") {
            return;
        }
        for token in node.iter_tokens().filter(|t| t.kind == Kind::String) {
            let mut units: Vec<u16> = Vec::new();
            let mut rest = token.text.as_str().trim_matches('"');
            while let Some(pos) = rest.find('\\') {
                units.extend(rest.get(..pos).unwrap_or_default().encode_utf16());
                let Some(unit) = rest.get(pos + 1..pos + 5).and_then(|h| u16::from_str_radix(h, 16).ok()) else {
                    // fea-rs's validation refuses a malformed escape itself.
                    return;
                };
                units.push(unit);
                rest = rest.get(pos + 5..).unwrap_or_default();
            }
            if char::decode_utf16(units).any(|c| c.is_err()) {
                self.refuse(node, "this name string's escapes are not valid UTF-16 (a surrogate without its pair)".into());
            }
        }
    }

    /// fea-rs reads the design size and range as unsigned decipoints and panics on a negative
    /// one or on a missing range end.
    fn size_parameters(&mut self, node: &Node) {
        let values: Vec<f64> =
            node.iter_tokens().filter(|t| matches!(t.kind, Kind::Number | Kind::Float)).filter_map(|t| t.text.parse::<f64>().ok()).collect();
        if values.iter().any(|v| *v < 0.0) {
            self.refuse(node, "the size parameters may not be negative".into());
        }
        if values.get(1).is_some_and(|v| *v != 0.0) && values.len() < 4 {
            self.refuse(node, "size parameters with a subfamily identifier need the range start and end".into());
        }
    }

    fn character(&mut self, node: &Node) {
        for token in node.iter_tokens().filter(|t| matches!(t.kind, Kind::Number | Kind::Hex | Kind::Octal)) {
            if number(token).is_none_or(|v| v > 0x10_FFFF) {
                self.refuse(node, format!("the character {} is beyond Unicode", token.text));
            }
        }
    }
}

/// A decimal, hexadecimal or octal number token's value.
fn number(token: &Token) -> Option<u32> {
    match token.kind {
        Kind::Number => token.text.parse().ok(),
        Kind::Hex => token.text.get(2..).and_then(|h| u32::from_str_radix(h, 16).ok()),
        Kind::Octal => u32::from_str_radix(&token.text, 8).ok(),
        _ => None,
    }
}

pub(crate) fn is_rule(kind: Kind) -> bool {
    use Kind::*;
    matches!(
        kind,
        GsubType1
            | GsubType2
            | GsubType3
            | GsubType4
            | GsubType5
            | GsubType6
            | GsubType7
            | GsubType8
            | GsubIgnore
            | GposType1
            | GposType2
            | GposType3
            | GposType4
            | GposType5
            | GposType6
            | GposType7
            | GposType8
            | GposIgnore
    )
}

/// The language system a block's `script` and `language` statements have set, as far as the
/// tree shows it.
enum System {
    /// Inherited from outside the block.
    Unknown,
    /// None set yet in a feature block.
    Default,
    /// A script and whether the language is the default one.
    Set(String, bool),
}

/// A key for the lookup flags a `lookupflag` statement sets: the low bits (0x100 standing for
/// a mark attachment class), then the text of each class it names. Statements that set
/// different flags get different keys; statements with different keys may still set the same
/// flags (a class named once by name and once by its glyphs), which only makes the lookup
/// count higher.
fn flag_key(node: &Node) -> Option<String> {
    let mut key = 0u16;
    let mut classes = String::new();
    for child in node.iter_children() {
        let Some(token) = child.as_token() else {
            if let Some(class) = child.as_node() {
                classes.push('|');
                for t in class.iter_tokens().filter(|t| !matches!(t.kind, Kind::Whitespace | Kind::Comment)) {
                    classes.push_str(&t.text);
                    classes.push(' ');
                }
            }
            continue;
        };
        match token.kind {
            // fea-rs keeps the low byte of a number, as fontTools does.
            Kind::Number => return token.text.parse::<u32>().ok().map(|n| (n & 0xff).to_string()),
            Kind::RightToLeftKw => key |= 1,
            Kind::IgnoreBaseGlyphsKw => key |= 2,
            Kind::IgnoreLigaturesKw => key |= 4,
            Kind::IgnoreMarksKw => key |= 8,
            Kind::UseMarkFilteringSetKw => key |= 0x10,
            Kind::MarkAttachmentTypeKw => key |= 0x100,
            Kind::NamedGlyphClass => {
                classes.push('|');
                classes.push_str(&token.text);
            }
            _ => {}
        }
    }
    Some(format!("{key}{classes}"))
}

/// The tag of a `script` or `language` statement.
pub(crate) fn tag_text(node: &Node) -> Option<String> {
    node.iter_children().filter_map(NodeOrToken::as_token).find(|t| t.kind == Kind::Tag).map(|t| t.text.to_string())
}

/// Whether a node is or holds a rule; lookup blocks hold statements only a few levels deep.
fn has_rule(node: &Node, depth: usize) -> bool {
    is_rule(node.kind()) || (depth < MAX_TREE_DEPTH && node.iter_children().filter_map(NodeOrToken::as_node).any(|n| has_rule(n, depth + 1)))
}

/// Whether a node holds a node of the given kind.
fn has_kind(node: &Node, kind: Kind, depth: usize) -> bool {
    depth < MAX_TREE_DEPTH && node.iter_children().filter_map(NodeOrToken::as_node).any(|n| n.kind() == kind || has_kind(n, kind, depth + 1))
}

fn is_gpos(kind: Kind) -> bool {
    use Kind::*;
    matches!(kind, GposType1 | GposType2 | GposType3 | GposType4 | GposType5 | GposType6 | GposType7 | GposType8 | GposIgnore)
}

fn is_glyph_item(kind: Kind) -> bool {
    matches!(kind, Kind::GlyphName | Kind::Cid | Kind::NamedGlyphClass | Kind::GlyphClass | Kind::GlyphRange | Kind::GlyphsPredicateNode)
}

/// The glyphs a range names: the difference of its CIDs, or of the letter or digit run its
/// two names differ in, as fea-rs expands it; 1 when the ends cannot form a range (fea-rs
/// reports that).
fn range_size(node: &Node) -> usize {
    let mut ends = node.iter_children().filter_map(NodeOrToken::as_token).filter(|t| matches!(t.kind, Kind::GlyphName | Kind::Cid));
    let (Some(start), Some(end)) = (ends.next(), ends.next()) else {
        return 1;
    };
    let span = |a: u32, b: u32| usize::try_from(b.abs_diff(a)).unwrap_or(usize::MAX).saturating_add(1).min(1 << 16);
    if start.kind == Kind::Cid {
        return match (start.text.parse::<u32>(), end.text.parse::<u32>()) {
            (Ok(a), Ok(b)) => span(a, b),
            _ => 1,
        };
    }
    let (a, b) = (start.text.as_bytes(), end.text.as_bytes());
    if a.len() != b.len() {
        return 1;
    }
    let front = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let back = a.len() - a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count();
    if back <= front {
        return 1;
    }
    let mut lo = front;
    while lo > 0 && a.get(lo - 1).is_some_and(u8::is_ascii_digit) {
        lo -= 1;
    }
    let mut hi = back;
    while a.get(hi).is_some_and(u8::is_ascii_digit) {
        hi += 1;
    }
    let digits = |s: &[u8]| s.get(lo..hi).and_then(|d| std::str::from_utf8(d).ok()).and_then(|d| d.parse::<u32>().ok());
    match (digits(a), digits(b)) {
        (Some(x), Some(y)) => span(x, y),
        _ if back - front == 1 => match (a.get(front), b.get(front)) {
            (Some(&x), Some(&y)) => span(u32::from(x), u32::from(y)),
            _ => 1,
        },
        _ => 1,
    }
}

/// The node's text, cut to [`SNIPPET_BYTES`].
fn snippet(node: &Node) -> String {
    let mut out = String::new();
    for token in node.iter_tokens() {
        if out.len() + token.text.len() > SNIPPET_BYTES {
            break;
        }
        out.push_str(&token.text);
    }
    out
}

/// A refusal at the first place a file holds the statement's text, or about the request as a
/// whole when none does.
fn place(files: &BTreeMap<String, Arc<str>>, snippet: &str, message: String) -> Diagnostic {
    if !snippet.is_empty() {
        for (path, text) in files {
            if let Some(pos) = text.find(snippet) {
                return Diagnostic::at(path.clone(), pos..pos + snippet.len(), Severity::Error, message);
            }
        }
    }
    Diagnostic::request(Severity::Error, message)
}
