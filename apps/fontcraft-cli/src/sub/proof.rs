//! `fontcraft-cli proof` (M12.35, M12.36). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand { name: "proof", summary: "Make proof sheets", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli proof FONT... [--layout NAME] [--format pdf|svg|html] [--glyphs LIST] [--size PT | --glyphs-per-page N] [-o OUT] [options]
";

/// The options, as data (M12.35, M12.36).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.35, M12.36
    super::not_built(&SUBCOMMAND, args)
}
