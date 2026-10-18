//! The subcommands, one file each. A subcommand file declares its usage and options as data
//! ([`Subcommand`]), parses its arguments into one engine command's parameters, and prints the
//! result (`cli.global.commandDispatch`). Files of subcommands whose package has not landed are
//! stubs: `--help` prints the usage, anything else exits 4 with "not built yet".

mod autohint;
mod bench;
mod check;
mod collection;
mod commands;
mod compile;
mod completion;
mod convert;
mod dehint;
mod diff;
mod dump;
mod features;
mod glyphs;
mod info;
mod instance;
mod mcp;
mod merge;
mod metrics;
mod optimize;
mod oracle;
mod outlines;
mod proof;
mod run;
mod sources;
mod stems;
mod subset;
mod tables;
mod transform;
mod variations;
mod woff;

use serde_json::{Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, out, usage};

/// One subcommand.
pub struct Subcommand {
    pub name: &'static str,
    /// One line for the subcommand list.
    pub summary: &'static str,
    /// The synopsis lines, as help prints them.
    pub usage: &'static str,
    /// The options, as data; empty for a subcommand that parses its own arguments.
    pub options: &'static [OptSpec],
    /// The actions of a multi-action subcommand (`tables list`); empty otherwise.
    pub actions: &'static [&'static str],
    pub run: fn(&[String]) -> Outcome,
}

/// Every subcommand, in the order help lists them.
pub const ALL: &[Subcommand] = &[
    commands::SUBCOMMAND,
    run::SUBCOMMAND,
    convert::SUBCOMMAND,
    compile::SUBCOMMAND,
    info::SUBCOMMAND,
    tables::SUBCOMMAND,
    glyphs::SUBCOMMAND,
    mcp::SUBCOMMAND,
    oracle::SUBCOMMAND,
    dump::SUBCOMMAND,
    metrics::SUBCOMMAND,
    subset::SUBCOMMAND,
    instance::SUBCOMMAND,
    merge::SUBCOMMAND,
    transform::SUBCOMMAND,
    optimize::SUBCOMMAND,
    collection::SUBCOMMAND,
    woff::SUBCOMMAND,
    diff::SUBCOMMAND,
    outlines::SUBCOMMAND,
    autohint::SUBCOMMAND,
    stems::SUBCOMMAND,
    dehint::SUBCOMMAND,
    check::SUBCOMMAND,
    proof::SUBCOMMAND,
    features::SUBCOMMAND,
    variations::SUBCOMMAND,
    bench::SUBCOMMAND,
    completion::SUBCOMMAND,
    sources::SUBCOMMAND,
];

/// The subcommands whose packages have not landed: each prints its usage and refuses to run.
pub const PLANNED: &[&str] = &["instance", "merge", "optimize"];

/// The subcommand named `name`.
pub fn find(name: &str) -> Option<&'static Subcommand> {
    ALL.iter().find(|s| s.name == name)
}

/// The help text of a subcommand.
pub fn help(sub: &Subcommand) -> String {
    format!("fontcraft-cli {}: {}\n\nUSAGE:\n{}", sub.name, sub.summary, args::help(sub.usage, sub.options))
}

/// What a stub does: the usage for `--help`, a usage error without arguments, and exit status 4
/// otherwise.
pub fn not_built(sub: &Subcommand, args: &[String]) -> Outcome {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return out(&help(sub));
    }
    let Some(first) = args.first() else { return Err(usage(help(sub))) };
    let what = match sub.actions.iter().find(|a| **a == first.as_str()) {
        Some(action) => format!("{} {action}", sub.name),
        None if !sub.actions.is_empty() => return Err(usage(format!("unknown {} action `{first}` (actions: {})", sub.name, sub.actions.join(", ")))),
        None => sub.name.to_string(),
    };
    Err(Failure::Failed(format!("{what} is not built yet")))
}

/// `--name VALUE` options after the positional arguments, as JSON parameters; `flags` are the
/// options that take no value.
pub(crate) fn options(
    args: &[String],
    names: &[(&str, &str)],
    flags: &[(&str, &str)],
) -> Result<(Vec<String>, serde_json::Map<String, Value>), Failure> {
    let mut positional = Vec::new();
    let mut params = serde_json::Map::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some((_, key)) = flags.iter().find(|(f, _)| f == a) {
            params.insert((*key).to_string(), json!(true));
        } else if let Some((_, key)) = names.iter().find(|(f, _)| f == a) {
            let v = it.next().ok_or_else(|| usage(format!("{a} needs a value")))?;
            let value = match *key {
                "face" => v.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(v)),
                "sections" => json!(v.split(',').collect::<Vec<_>>()),
                _ => json!(v),
            };
            params.insert((*key).to_string(), value);
        } else if a.starts_with("--") {
            return Err(usage(format!("unknown option {a}")));
        } else {
            positional.push(a.clone());
        }
    }
    Ok((positional, params))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every subcommand of the inventory has a file here, so help and completion list them all.
    #[test]
    fn every_inventory_subcommand_is_registered() {
        let text = include_str!("../../../../spec/inventory/20-cli.toml");
        for block in text.split("\n[[cli]]").skip(1) {
            let Some(id) = block.lines().find_map(|l| l.strip_prefix("id = \"").and_then(|r| r.strip_suffix('"'))) else { continue };
            let mut words = id.split(' ');
            let name = words.next().unwrap_or_default();
            if name == "help" {
                continue;
            }
            let sub = find(name).unwrap_or_else(|| panic!("`{name}` has no subcommand file"));
            if let Some(action) = words.next() {
                assert!(sub.actions.contains(&action), "`{id}` is not among the actions of `{name}`");
            }
            assert!(help(sub).contains(&format!("fontcraft-cli {name}")), "{name}");
        }
    }

    #[test]
    fn stubs_refuse_with_exit_status_4() {
        let sub = find("instance").unwrap();
        let e = (sub.run)(&["in.ttf".to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), 4);
        assert!(e.message().contains("not built"));
        let e = (find("collection").unwrap().run)(&["bogus".to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), 2, "an unknown action is a usage error");
    }
}
