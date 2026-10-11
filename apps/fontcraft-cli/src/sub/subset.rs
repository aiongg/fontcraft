//! `fontcraft-cli subset` (M12.17). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "subset", summary: "Reduce a font to chosen glyphs", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli subset FONT [GLYPH...] [--unicodes LIST] [--text STRING] [--glyphs LIST] [--gids LIST] [options] [-o OUT]
";

/// The options, as data (M12.17).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.17
    super::not_built(&SUBCOMMAND, args)
}
