//! `fontcraft-cli check` (M12.44). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "check",
    summary: "Run quality checks on fonts and sources",
    usage: USAGE,
    options: OPTIONS,
    actions: &["outlines", "family", "interpolation"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli check [outlines|family|interpolation] INPUT... [--profile ID] [--check ID]... [--exclude ID]... [--fix] [--report PATH] [options]
  fontcraft-cli check outlines INPUT... [--fix] [--no-overlap-checks] [--no-basic-checks] [--min-area N] [--tolerance N] [--glyphs LIST] [--all] [-o OUT]
  fontcraft-cli check family DIR|FONT... [--tolerance N] [--menu-names] [--metrics] [--panose] [--layout] [--single LIST] [--family LIST] [--no-hints]
  fontcraft-cli check interpolation INPUT... [--glyphs LIST] [--tolerance F] [--kinkiness F] [--show-all] [--ignore-missing] [--master NAME]... [--report PATH]
";

/// The options, as data (M12.44).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.44
    super::not_built(&SUBCOMMAND, args)
}
