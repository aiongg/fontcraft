//! One lookup per rule type ([`crate::CompileRequest::lookup_per_rule_type`]).
//!
//! The feature file specification groups a feature block's rules into lookups of one type each,
//! and AFDKO's feature compiler starts a new anonymous lookup whenever the type changes
//! (`FeatCtx::prepRule` in `addfeatures/hotconv/FeatCtx.cpp`). fea-rs, like feaLib, instead lets
//! a single substitution join the lookup of a neighbouring multiple or ligature substitution,
//! which it rewrites as a one-glyph sequence or ligature. [`split`] writes the code out again
//! with an empty named lookup block before each rule that would join that way: the block ends
//! the current lookup and adds none of its own.

use fea_rs::{Kind, Node, NodeOrToken};

use crate::check::{is_rule, tag_text};

/// The most characters tried in front of the break lookups' names to make them unique.
const MAX_PREFIX: usize = 32;

/// The code of `root`, with its included files written in place and a lookup break before each
/// substitution that fea-rs would merge into a lookup of another type; `None` when there is no
/// such rule.
pub(crate) fn split(root: &Node) -> Option<String> {
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
        let mut at = start;
        for child in feature.iter_children() {
            let child_at = at;
            at = at.saturating_add(child.text_len());
            let Some(child) = child.as_node() else { continue };
            match child.kind() {
                Kind::LanguageNode | Kind::ScriptNode | Kind::LookupBlockNode => current = None,
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
    if breaks.is_empty() {
        return None;
    }
    let text: String = root.iter_tokens().map(|t| t.as_str()).collect::<String>();
    if text.len() != root.text_len() {
        return None;
    }
    let mut prefix = String::from("fcTypeBreak");
    while text.contains(&prefix) {
        if prefix.len() >= MAX_PREFIX {
            return None;
        }
        prefix.push('_');
    }
    let mut out = String::with_capacity(text.len().saturating_add(breaks.len().saturating_mul(prefix.len() * 2 + 32)));
    let mut from = 0;
    for (n, at) in breaks.into_iter().enumerate() {
        out.push_str(text.get(from..at)?);
        out.push_str(&format!("lookup {prefix}{n} {{ }} {prefix}{n}; "));
        from = at;
    }
    out.push_str(text.get(from..)?);
    Some(out)
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
