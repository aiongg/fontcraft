//! `fontcraft-cli features` (M12.11–M12.14). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "features",
    summary: "List, compile, decompile, check, convert and normalise layout features",
    usage: USAGE,
    options: OPTIONS,
    actions: &["list", "compile", "decompile", "lint", "convert", "normalize"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli features list|compile|decompile|lint|convert|normalize INPUT [args]
  fontcraft-cli features list FONT [--table GSUB|GPOS] [--lookups] [--json]
  fontcraft-cli features compile FEATURES --font FONT [-o OUT] [--tables TAG,...] [--debug-lookups] [--include-dir DIR] [--syntax fea|mti|volt]
  fontcraft-cli features decompile FONT [--table GSUB|GPOS|GDEF|BASE,...] [--features TAG,...] [--show-extensions] [-o OUT.fea]
  fontcraft-cli features lint FEATURES [--font FONT | --glyph-order FILE] [--include-dir DIR] [--warnings default|all]
  fontcraft-cli features convert INPUT --to fea|ttx [-o OUT] [--table GSUB|GPOS|GDEF,...]
  fontcraft-cli features normalize FONT [--table gpos|gsub|gdef|all] [-o OUT]
";

/// The options, as data (M12.11–M12.14).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.11–M12.14
    super::not_built(&SUBCOMMAND, args)
}
