//! `fontcraft-cli dehint` (M12.27). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "dehint", summary: "Remove hints and instructions", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli dehint INPUT... [--glyphs LIST] [--keep-font-hints] [--hinting-tables TAG,...] [-o OUT]
";

/// The options, as data (M12.27).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.27
    super::not_built(&SUBCOMMAND, args)
}
