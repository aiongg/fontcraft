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
mod compile_opts;
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

use fontcraft_engine::format::output;

use crate::args::{self, OptSpec};
use crate::io::{self, Failure, Outcome, out, usage};

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
pub const PLANNED: &[&str] = &[];

/// Subcommands that read their arguments without [`args::parse`] and do not answer `--help`
/// themselves: [`run`] answers it and takes the common options out first.
const OWN_PARSERS: &[&str] = &["compile", "mcp", "oracle"];

/// The subcommand named `name`.
pub fn find(name: &str) -> Option<&'static Subcommand> {
    ALL.iter().find(|s| s.name == name)
}

/// The name `given` stands for among the subcommands and `extra` (`help`): the name itself, or
/// in another letter case, or the one name it begins (`conv` → `convert`). `None` when it names
/// none; a prefix of several names is a usage error that lists them.
pub fn full_name(given: &str, extra: &[&'static str]) -> Result<Option<&'static str>, Failure> {
    let names: Vec<&'static str> = ALL.iter().map(|s| s.name).chain(extra.iter().copied()).collect();
    args::pick(&names, given).map_err(|e| usage(format!("subcommand {}", e.message())))
}

/// The subcommand `given` names, in full or by a prefix ([`full_name`]).
pub fn lookup(given: &str) -> Result<Option<&'static Subcommand>, Failure> {
    Ok(full_name(given, &[])?.and_then(find))
}

/// The option spellings of a subcommand in [`OWN_PARSERS`], so that [`args::take_globals`]
/// leaves a prefix of one of them to the subcommand.
fn own_spellings(name: &str) -> Vec<&'static str> {
    match name {
        "compile" => compile_opts::OPTIONS.iter().flat_map(|o| o.flags.iter().copied()).collect(),
        "mcp" => mcp::SPELLINGS.to_vec(),
        "oracle" => oracle::SPELLINGS.to_vec(),
        _ => Vec::new(),
    }
}

/// Runs `sub` on its arguments.
pub fn run(sub: &Subcommand, args: &[String]) -> Outcome {
    // A line of a script leaves the next one its own switches.
    let before = (output::dry_run(), output::no_clobber(), io::layer(), args::set_current(Some(sub.name)));
    let refused = [(before.0, "dry-run"), (before.1, "no-clobber")];
    let result = match refused.iter().find(|(on, option)| *on && crate::behaviours::refuses(sub.name, option)) {
        Some((_, option)) => Err(usage(format!("--{option} does not apply to `{}`", sub.name))),
        None => run_current(sub, args),
    };
    let result = result.and_then(|()| io::open_log_file());
    let (dry_run, no_clobber, layer, current) = before;
    if current.is_some() {
        output::set_dry_run(dry_run);
        output::set_no_clobber(no_clobber);
        io::set_layer(layer);
    }
    args::set_current(current);
    result
}

fn run_current(sub: &Subcommand, args: &[String]) -> Outcome {
    if !OWN_PARSERS.contains(&sub.name) {
        return (sub.run)(args);
    }
    if args.iter().take_while(|a| *a != "--").any(|a| a == "-h" || a == "--help") {
        return out(&help(sub));
    }
    let args = if sub.name == "compile" { compile_opts::expand(args)? } else { args.to_vec() };
    (sub.run)(&args::take_globals(&args, &own_spellings(sub.name))?)
}

/// The help text of a subcommand: its usage and options, the engine command it runs, and a
/// pointer to the common options.
pub fn help(sub: &Subcommand) -> String {
    let mut text = format!("fontcraft-cli {}: {}\n\nUSAGE:\n{}", sub.name, sub.summary, args::help(sub.usage, sub.options));
    let commands: Vec<String> = commands::ENGINE_COMMANDS
        .iter()
        .filter(|(id, _)| *id == sub.name || id.strip_prefix(sub.name).is_some_and(|rest| rest.starts_with(' ')))
        .map(|(id, command)| if *id == sub.name { (*command).to_string() } else { format!("{command} ({id})") })
        .collect();
    if !commands.is_empty() {
        text.push_str(&format!("\n\nENGINE COMMANDS: {}", commands.join(", ")));
    }
    text.push_str(
        "\n\nCOMMON OPTIONS: -q, -v, -vv, --log LEVEL, --log-file FILE, --color auto|always|never, --timing,\n\
         --backtrace, -j/--jobs N, --dry-run, --no-clobber, --options FILE, --save-options FILE, --trace FILE,\n\
         --layer NAME, -V/--version (`fontcraft-cli help` describes them)",
    );
    text
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
    fn a_subcommand_may_be_named_by_a_unique_prefix() {
        assert_eq!(full_name("conv", &[]).unwrap(), Some("convert"));
        assert_eq!(full_name("transform", &[]).unwrap(), Some("transform"));
        assert_eq!(full_name("he", &["help"]).unwrap(), Some("help"));
        assert_eq!(full_name("nosuch", &[]).unwrap(), None);
        assert_eq!(lookup("orac").unwrap().map(|s| s.name), Some("oracle"));
        let e = full_name("co", &[]).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.message().contains("compile") && e.message().contains("convert"), "{e:?}");
    }

    #[test]
    fn stubs_refuse_with_exit_status_4() {
        let e = not_built(find("optimize").unwrap(), &["in.ttf".to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), 4);
        assert!(e.message().contains("not built"));
        let e = (find("collection").unwrap().run)(&["bogus".to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), 2, "an unknown action is a usage error");
    }
}
