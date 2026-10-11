//! `fontcraft-cli autohint` (M12.39). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "autohint", summary: "Add PostScript hints", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli autohint INPUT... [-o OUT...] [--glyphs LIST] [--all] [--reference FONT] [--allow-changes] [--report-only] [options]
";

/// The options, as data (M12.39).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.39
    super::not_built(&SUBCOMMAND, args)
}
