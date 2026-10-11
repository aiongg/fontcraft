//! `fontcraft-cli completion` (M12.43). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "completion", summary: "Print a shell completion script", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli completion bash|zsh|fish|powershell
";

/// The options, as data (M12.43).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.43
    super::not_built(&SUBCOMMAND, args)
}
