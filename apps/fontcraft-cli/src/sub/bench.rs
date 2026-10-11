//! `fontcraft-cli bench` (M12.43). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "bench", summary: "Time subcommands on a font", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli bench SUBCOMMAND [ARGS...] [--runs N] [--warmup N] [--compare TOOL...] [--json]
";

/// The options, as data (M12.43).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.43
    super::not_built(&SUBCOMMAND, args)
}
