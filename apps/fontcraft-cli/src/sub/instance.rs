//! `fontcraft-cli instance` (M12.20–M12.22). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "instance",
    summary: "Make static or partial instances of a variable font or source",
    usage: USAGE,
    options: OPTIONS,
    actions: &[],
    run,
};

const USAGE: &str = "\
  fontcraft-cli instance INPUT [AXIS=LOC...] [--named REGEX | --all | --index LIST] [--static] [--to same|ttf|otf|ufo] [-o OUT | --output-dir DIR]
";

/// The options, as data (M12.20–M12.22).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.20–M12.22
    super::not_built(&SUBCOMMAND, args)
}
