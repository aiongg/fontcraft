//! `fontcraft-cli metrics` (M12.16). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "metrics", summary: "Report glyph metrics and kerning", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli metrics FONT... [--format text|json|csv|afm] [--precision integer|real] [--aggregate-bbox] [--kerning] [--statistics] [--glyphs SELECTOR]
";

/// The options, as data (M12.16).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.16
    super::not_built(&SUBCOMMAND, args)
}
