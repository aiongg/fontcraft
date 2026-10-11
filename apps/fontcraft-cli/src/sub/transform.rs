//! `fontcraft-cli transform` (M12.24). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "transform", summary: "Transform every glyph or change units per em", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli transform FONT [--rotate DEG] [--translate X,Y] [--scale SX[,SY]] [--slant DEG] [--matrix A,B,C,D,E,F] [--upem N] [--glyph-table FILE] [--widths auto|keep|transform|em] [-o OUT]
";

/// The options, as data (M12.24).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.24
    super::not_built(&SUBCOMMAND, args)
}
