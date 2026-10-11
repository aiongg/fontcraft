//! `fontcraft-cli woff` (M12.3). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "woff",
    summary: "Compress fonts to WOFF or WOFF2 and back",
    usage: USAGE,
    options: OPTIONS,
    actions: &["compress", "decompress"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli woff compress|decompress FONT... [options]
  fontcraft-cli woff compress FONT... [--format woff2|woff] [--no-glyf-transform] [--hmtx-transform] [--metadata FILE] [--private-data FILE] [-o OUT]
  fontcraft-cli woff decompress FONT.woff2|FONT.woff... [-o OUT]
";

/// The options, as data (M12.3).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.3
    super::not_built(&SUBCOMMAND, args)
}
