//! Checks on fea-rs's parse tree before `validate` and `compile` run.
//!
//! Two kinds of check. Growth: glyph classes, ligature sequences and enumerated pairs expand
//! to the product or sum of their members, and fea-rs builds them all; the sizes are added up
//! here with saturation and refused over [`Limits`]. Known panics: input fea-rs's validation
//! lets through and its compiler then panics on (a number outside 16 bits where it reads a
//! signed 16-bit value, a CID over 65,535, an empty `table head`, a `size` feature whose
//! subfamily and menu names disagree, a character variant character beyond Unicode, a Windows
//! name string whose escapes are not valid UTF-16). Each has a test in `rows_tests.rs`.
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

pub(crate) fn tree(root: &Node, glyph_count: usize, limits: &Limits, files: &BTreeMap<String, Arc<str>>) -> Vec<Diagnostic> {
    let mut walk = Walk {
        limits,
        glyph_count,
        classes: HashMap::new(),
        class_total: 0,
        rules: 0,
        name_ids: HashSet::new(),
        elided_names: Vec::new(),
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
    refusals: Vec<(String, String)>,
}

impl Walk<'_> {
    fn refuse(&mut self, node: &Node, message: String) {
        if self.refusals.len() < MAX_REFUSALS {
            self.refusals.push((snippet(node), message));
        }
    }

    fn node(&mut self, node: &Node, depth: usize) {
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
            Kind::FeatureNode => self.size_feature(node),
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
            kind if is_rule(kind) => self.rule(node),
            _ => {}
        }
        for child in node.iter_children() {
            match child {
                NodeOrToken::Node(n) => self.node(n, depth + 1),
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
            Kind::Number if token.text.parse::<i16>().is_err() => {
                self.refuse(parent, format!("the number {} is outside -32768 to 32767, the range the feature compiler reads here", token.text));
            }
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

fn is_rule(kind: Kind) -> bool {
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
