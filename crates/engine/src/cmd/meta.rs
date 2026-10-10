//! Commands about commands: what agents call first.

use serde_json::{Value, json};

use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "commands.list",
            "List Commands",
            [],
            None,
            "{pattern?: string} every command (or those whose id or label contains `pattern`, ignoring case) → {commands: [{id, label, menu, shortcut, params, query, targets?: [\"glyphs\"|\"layer\"|\"points\"|\"segments\"|\"shapes\"|\"anchors\"|\"guides\"|\"annotations\"], when?, context?: [menus], enabled, disabled_reason?}]}",
            always,
            list
        ),
        cmd!(
            query "document.inspect",
            "Inspect Documents",
            [],
            None,
            "{} the open documents → {documents: [{index, active, title, path, modified, revision, familyName, glyphs, masters, undo: [labels], redo: [labels]}], active}",
            always,
            inspect
        ),
    ]
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("commands.list", p)?;
    let pattern = p.opt_str("pattern")?.map(str::to_lowercase);
    let commands: Vec<CommandInfo> = command_specs()
        .iter()
        .filter(|c| pattern.as_deref().is_none_or(|pat| c.id.to_lowercase().contains(pat) || c.label.to_lowercase().contains(pat)))
        .map(|c| c.info(s))
        .collect();
    Ok(json!({ "commands": commands }))
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    Params::new("document.inspect", p)?;
    Ok(s.inspect())
}
