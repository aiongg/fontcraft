//! `fontcraft-cli sources` (M12.37). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "sources", summary: "Compare and merge sources", usage: USAGE, options: OPTIONS, actions: &["diff", "merge"], run };

const USAGE: &str = "\
  fontcraft-cli sources (diff | merge) ...
  fontcraft-cli sources diff A B [--parts glyphs,info,masters,instances,axes,kerning,features] [--glyphs LIST] [--tolerance UNITS] [--format text|json]
  fontcraft-cli sources merge BASE OTHER (--take ID,... | --take-all) [--parts LIST] [--glyphs LIST] (-o OUT | --in-place)
";

/// The options, as data (M12.37).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.37
    super::not_built(&SUBCOMMAND, args)
}
