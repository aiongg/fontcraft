//! `fontcraft-cli merge` (M12.23). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand { name: "merge", summary: "Merge fonts into one", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli merge FONT [--map FILE] [--glyphs LIST] FONT [--map FILE] ... -o OUT [options]
";

/// The options, as data (M12.23).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.23
    super::not_built(&SUBCOMMAND, args)
}
