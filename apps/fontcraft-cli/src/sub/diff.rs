//! `fontcraft-cli diff` (M12.32). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "diff", summary: "Compare two fonts table by table", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli diff A B [--summary | --detail LEVEL] [-t TAG]... [-x TAG]... [--normalize none|dump|compiler] [--context N] [--output-dir DIR]
";

/// The options, as data (M12.32).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.32
    super::not_built(&SUBCOMMAND, args)
}
