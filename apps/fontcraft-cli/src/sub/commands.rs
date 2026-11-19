//! `fontcraft-cli commands`: the engine's command catalogue, or with `--cli` the subcommands.

use super::Subcommand;
use super::completion;
use crate::args::{self, GLOBAL, OptSpec};
use crate::io::{Outcome, execute, out, out_json, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "commands", summary: "Print the command catalogue as JSON", usage: USAGE, options: OPTIONS, actions: &[], run: commands };

const USAGE: &str = "\
  fontcraft-cli commands [PATTERN] [--cli [--json]]
      Print the command catalogue as JSON (only commands whose id or label contains PATTERN).
      --cli lists the subcommands instead (only those whose name or summary contains PATTERN):
      each one's engine command and actions as text, or with --json also their options, the
      common options and the options of `help`, as one JSON document.
";

const OPTIONS: &[OptSpec] = &[
    OptSpec::switch("cli", false, "list the subcommands and the engine command each one runs"),
    OptSpec::switch("json", false, "with --cli, print JSON instead of text (the catalogue is always JSON)"),
];

/// The engine command behind each subcommand and action that runs one: the `command` field of
/// its row in `spec/inventory/20-cli.toml`, in the order of the rows.
pub(super) const ENGINE_COMMANDS: &[(&str, &str)] = &[
    ("compile", "font.compile"),
    ("dump", "font.dumpTables"),
    ("info", "font.inspect"),
    ("tables list", "tables.list"),
    ("tables extract", "tables.extract"),
    ("tables add", "tables.add"),
    ("tables delete", "tables.delete"),
    ("tables edit", "tables.edit"),
    ("tables copy", "tables.add"),
    ("tables checksum", "tables.checksum"),
    ("tables import", "tables.importTtx"),
    ("tables reorder", "tables.reorder"),
    ("glyphs list", "font.listGlyphs"),
    ("glyphs rename", "font.renameGlyphs"),
    ("glyphs reorder", "font.reorderGlyphs"),
    ("metrics", "font.reportMetrics"),
    ("convert", "font.convert"),
    ("subset", "font.subset"),
    ("instance", "font.instantiate"),
    ("merge", "font.merge"),
    ("transform", "font.transform"),
    ("optimize", "font.optimize"),
    ("collection list", "collection.list"),
    ("collection pack", "collection.pack"),
    ("collection unpack", "collection.unpack"),
    ("woff compress", "woff.compress"),
    ("woff decompress", "woff.decompress"),
    ("diff", "font.diff"),
    ("outlines remove-overlaps", "outlines.removeOverlaps"),
    ("outlines direction", "outlines.correctDirection"),
    ("outlines extremes", "outlines.addExtremes"),
    ("outlines round", "outlines.roundCoordinates"),
    ("outlines to-quadratic", "outlines.convertCurves"),
    ("outlines to-cubic", "outlines.convertCurves"),
    ("outlines componentize", "outlines.componentize"),
    ("outlines decompose", "outlines.decompose"),
    ("autohint", "autohint.ps.run"),
    ("stems", "autohint.ps.reportStems"),
    ("dehint", "hints.remove"),
    ("check", "audit.run"),
    ("check outlines", "audit.checkOutlines"),
    ("check family", "audit.compareFamily"),
    ("check interpolation", "audit.checkInterpolation"),
    ("proof", "font.proof"),
    ("features list", "layout.listFeatures"),
    ("features compile", "layout.compileFeatures"),
    ("features decompile", "layout.decompileFeatures"),
    ("features lint", "layout.lintFeatures"),
    ("features convert", "layout.convertSyntax"),
    ("features normalize", "layout.normalize"),
    ("variations build-avar", "variations.buildAvar"),
    ("variations plan-avar", "variations.planAvar"),
    ("variations map", "variations.mapCoordinates"),
    ("variations build-hvar", "variations.buildHvar"),
    ("variations model", "variations.describeModel"),
    ("variations plot", "variations.plot"),
    ("commands", "commands.list"),
    ("tables show", "tables.show"),
    ("sources diff", "source.diff"),
    ("sources merge", "source.merge"),
];

/// The engine command the subcommand or action `id` (`tables list`) runs.
fn engine_command(id: &str) -> Option<&'static str> {
    ENGINE_COMMANDS.iter().find(|(i, _)| *i == id).map(|(_, command)| *command)
}

fn commands(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let pattern = match parsed.positional.as_slice() {
        [] => None,
        [pattern] => Some(pattern.as_str()),
        _ => return Err(usage("commands takes at most one PATTERN")),
    };
    if parsed.switch("cli") {
        return if parsed.switch("json") { out_json(&cli_list(pattern)) } else { print_cli(pattern) };
    }
    let list = execute(&mut Session::new(), "commands.list", &json!({"pattern": pattern}))?;
    out_json(&list)
}

/// Whether the subcommand matches `pattern`: its name or summary contains it, ignoring case.
fn matches(sub: &Subcommand, pattern: Option<&str>) -> bool {
    pattern.map(str::to_lowercase).is_none_or(|p| sub.name.contains(&p) || sub.summary.to_lowercase().contains(&p))
}

/// The subcommands as JSON: name, summary, engine command, actions with theirs, and the options
/// each accepts (from its option table and its usage text); then the common options and the
/// options of `help`.
fn cli_list(pattern: Option<&str>) -> Value {
    let model = completion::model(super::ALL);
    let subcommands: Vec<Value> = model
        .iter()
        .zip(super::ALL)
        .filter(|(_, sub)| matches(sub, pattern))
        .map(|(entry, sub)| {
            let actions: Vec<Value> =
                sub.actions.iter().map(|a| json!({"name": a, "command": engine_command(&format!("{} {a}", sub.name))})).collect();
            json!({
                "name": sub.name,
                "summary": sub.summary,
                "command": engine_command(sub.name),
                "actions": actions,
                "options": entry.opts.iter().map(completion::Opt::to_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    let common: Vec<Value> = GLOBAL.iter().map(|spec| completion::from_spec(spec).to_json()).collect();
    let help: Vec<Value> = crate::HELP_OPTIONS.iter().map(|spec| completion::from_spec(spec).to_json()).collect();
    json!({"subcommands": subcommands, "common_options": common, "help_options": help})
}

/// The subcommands as text: name, engine command (`(shell)` for none) and summary, then each
/// action with its engine command.
fn print_cli(pattern: Option<&str>) -> Outcome {
    for sub in super::ALL.iter().filter(|sub| matches(sub, pattern)) {
        out(&format!("{:<12} {:<28} {}", sub.name, engine_command(sub.name).unwrap_or("(shell)"), sub.summary))?;
        for action in sub.actions {
            out(&format!("  {action:<10} {}", engine_command(&format!("{} {action}", sub.name)).unwrap_or("(shell)")))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table follows the inventory, so `help` and `commands --cli` name the right commands.
    #[test]
    fn engine_commands_match_the_inventory() {
        let text = include_str!("../../../../spec/inventory/20-cli.toml");
        let mut expected = Vec::new();
        for block in text.split("\n[[").filter(|b| b.starts_with("cli]]")) {
            let field = |key: &str| block.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(" = \"")?.strip_suffix('"'));
            let (Some(id), Some(command)) = (field("id"), field("command")) else { continue };
            if !command.is_empty() {
                expected.push((id, command));
            }
        }
        assert_eq!(ENGINE_COMMANDS, expected.as_slice());
    }

    #[test]
    fn the_cli_listing_names_subcommands_actions_and_options() {
        let list = cli_list(None);
        let subs = list["subcommands"].as_array().unwrap();
        assert_eq!(subs.len(), super::super::ALL.len());
        let compile = subs.iter().find(|s| s["name"] == "compile").unwrap();
        assert_eq!(compile["command"], "font.compile");
        assert!(compile["options"].as_array().unwrap().iter().any(|o| o["long"] == "format"), "options come from the usage text too");
        let tables = subs.iter().find(|s| s["name"] == "tables").unwrap();
        assert!(tables["command"].is_null(), "a multi-action subcommand runs its actions' commands");
        assert!(tables["actions"].as_array().unwrap().iter().any(|a| a["name"] == "list" && a["command"] == "tables.list"));
        assert!(list["common_options"].as_array().unwrap().iter().any(|o| o["long"] == "jobs" && o["short"] == "j"));
        assert!(list["help_options"].as_array().unwrap().iter().any(|o| o["long"] == "all"));
        let narrowed = cli_list(Some("TABLES"));
        assert!(narrowed["subcommands"].as_array().unwrap().iter().all(|s| s["name"] != "compile"), "PATTERN narrows the list");
    }
}
