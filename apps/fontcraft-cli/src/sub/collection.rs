//! `fontcraft-cli collection` (M12.3). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "collection",
    summary: "List, pack and unpack font collections",
    usage: USAGE,
    options: OPTIONS,
    actions: &["list", "pack", "unpack"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli collection list|pack|unpack FILE... [args]
  fontcraft-cli collection list FILE.ttc [--json]
  fontcraft-cli collection pack FONT FONT... -o OUT.ttc [--share-table TAG=INDEX]... [--no-share]
  fontcraft-cli collection unpack FILE.ttc [--face N|NAME] [--output-dir DIR]
";

/// The options, as data (M12.3).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.3
    super::not_built(&SUBCOMMAND, args)
}
