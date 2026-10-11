//! `fontcraft-cli dump` (M12.4, M12.10). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "dump", summary: "Dump tables as TTX, JSON, hex or text views", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli dump FONT... [-t TAG[=VIEW]]... [-x TAG]... [--format ttx|json|text|hex] [--split tables|glyphs] [-o PATH]
";

/// The options, as data (M12.4, M12.10).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.4, M12.10
    super::not_built(&SUBCOMMAND, args)
}
