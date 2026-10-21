//! One lookup per rule type ([`crate::CompileRequest::lookup_per_rule_type`]).
//!
//! The feature file specification groups a feature block's rules into lookups of one type each,
//! and AFDKO's feature compiler starts a new anonymous lookup whenever the type changes
//! (`FeatCtx::prepRule` in `addfeatures/hotconv/FeatCtx.cpp`). fea-rs, like feaLib, instead lets
//! a single substitution join the lookup of a neighbouring multiple or ligature substitution,
//! which it rewrites as a one-glyph sequence or ligature. [`split`] writes the code out again
//! with an empty named lookup block before each rule that would join that way: the block ends
//! the current lookup and adds none of its own.
//!
//! The code is written out as one file, with every included file in place. An included file
//! that ends in a comment without a line break would then comment out the code after its
//! include statement, so comments are written as spaces of the same length; the insertion
//! markers fea-rs reads (`# Automatic Code`) stay, each followed by a line break.

use fea_rs::{Kind, Node, NodeOrToken};

use crate::check::{is_rule, tag_text};

/// The most characters tried in front of the break lookups' names to make them unique.
const MAX_PREFIX: usize = 32;

/// The code with its lookup breaks.
pub(crate) struct Split {
    pub(crate) text: String,
    /// Where text was added, in order: the offset in the original code (the root node's text,
    /// included files in place) and the number of bytes added there.
    pub(crate) inserts: Vec<(usize, usize)>,
}

/// The code of `root`, with its included files written in place and a lookup break before each
/// substitution that fea-rs would merge into a lookup of another type; `None` when there is no
/// such rule, and why the code cannot be written out when it cannot.
pub(crate) fn split(root: &Node) -> Result<Option<Split>, String> {
    let breaks = breaks(root);
    if breaks.is_empty() {
        return Ok(None);
    }
    let mut prefix = String::from("fcTypeBreak");
    while root.iter_tokens().any(|t| t.text.contains(prefix.as_str())) {
        if prefix.len() >= MAX_PREFIX {
            return Err("the code uses every name tried for the lookups that keep rule types apart".into());
        }
        prefix.push('_');
    }
    let capacity = root.text_len().saturating_add(breaks.len().saturating_mul(prefix.len().saturating_mul(2).saturating_add(32)));
    let mut out = String::with_capacity(capacity);
    let mut inserts = Vec::new();
    let mut pending = breaks.into_iter().enumerate().peekable();
    let mut at = 0usize;
    for token in root.iter_tokens() {
        while let Some((n, _)) = pending.next_if(|(_, b)| *b <= at) {
            let block = format!("lookup {prefix}{n} {{ }} {prefix}{n}; ");
            inserts.push((at, block.len()));
            out.push_str(&block);
        }
        let text = token.as_str();
        at = at.saturating_add(text.len());
        if token.kind != Kind::Comment {
            out.push_str(text);
        } else if is_marker(text) {
            out.push_str(text);
            out.push('\n');
            inserts.push((at, 1));
        } else {
            out.extend(std::iter::repeat_n(' ', text.len()));
        }
    }
    if at != root.text_len() || pending.next().is_some() {
        return Err("the code could not be written out with its included files in place".into());
    }
    Ok(Some(Split { text: out, inserts }))
}

/// A comment fea-rs keeps as a statement: a place for generated lookups.
fn is_marker(comment: &str) -> bool {
    comment.trim_start().starts_with("# Automatic Code")
}

/// The offsets, in ascending order, of the rules that need a lookup break before them.
fn breaks(root: &Node) -> Vec<usize> {
    // Offsets are counted from the text lengths: the tree's own positions are those of the
    // file each node came from.
    let mut breaks = Vec::new();
    let mut feature_at = 0usize;
    for item in root.iter_children() {
        let item_len = item.text_len();
        let start = feature_at;
        feature_at = feature_at.saturating_add(item_len);
        let Some(feature) = item.as_node().filter(|n| n.kind() == Kind::FeatureNode) else { continue };
        if tag_text(feature).as_deref() == Some("aalt") {
            continue;
        }
        let mut current = None;
        let mut system = System::Default;
        let mut at = start;
        for child in feature.iter_children() {
            let child_at = at;
            at = at.saturating_add(child.text_len());
            let Some(child) = child.as_node() else { continue };
            match child.kind() {
                Kind::LanguageNode => {
                    current = None;
                    let dflt = tag_text(child).is_some_and(|t| t == "dflt");
                    system = match system {
                        System::Unknown => System::Unknown,
                        System::Default => System::Set("DFLT".into(), dflt),
                        System::Set(script, _) => System::Set(script, dflt),
                    };
                }
                Kind::ScriptNode => {
                    // fea-rs ignores `script X` while the system is already X with the default
                    // language (as `check::count_block` counts it).
                    let script = tag_text(child).unwrap_or_default();
                    if !matches!(&system, System::Set(s, true) if *s == script) {
                        current = None;
                    }
                    system = System::Set(script, true);
                }
                Kind::LookupBlockNode => {
                    current = None;
                    let sets_system =
                        child.iter_children().filter_map(NodeOrToken::as_node).any(|n| matches!(n.kind(), Kind::ScriptNode | Kind::LanguageNode));
                    if sets_system {
                        system = System::Unknown;
                    }
                }
                kind => {
                    let joins = joinable(child);
                    if let (Some(a), Some(b)) = (current, joins)
                        && a != b
                        && (a == 1 || b == 1)
                    {
                        breaks.push(child_at);
                    }
                    if joins.is_some() || is_rule(kind) {
                        current = joins;
                    }
                }
            }
        }
    }
    breaks
}

/// The language system a feature block's `script` and `language` statements have set.
enum System {
    /// Set inside a lookup block, where this pass does not follow it.
    Unknown,
    /// None set yet.
    Default,
    /// A script and whether the language is the default one.
    Set(String, bool),
}

/// The lookup type a substitution that fea-rs can merge across types starts: 1 for a single
/// substitution, 2 for a multiple one (a deletion by `NULL` is one), 4 for a ligature.
fn joinable(rule: &Node) -> Option<u8> {
    match rule.kind() {
        Kind::GsubType1 if rule.iter_children().filter_map(NodeOrToken::as_token).any(|t| t.kind == Kind::NullKw) => Some(2),
        Kind::GsubType1 => Some(1),
        Kind::GsubType2 => Some(2),
        Kind::GsubType4 => Some(4),
        _ => None,
    }
}
