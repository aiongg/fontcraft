//! `fontcraft-cli outlines` (M12.28, M12.29). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "outlines",
    summary: "Repair or convert the outlines of a whole font",
    usage: USAGE,
    options: OPTIONS,
    actions: &["remove-overlaps", "direction", "extremes", "round", "to-quadratic", "to-cubic", "componentize", "decompose"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli outlines remove-overlaps|direction|extremes|round|to-quadratic|to-cubic|componentize|decompose INPUT... [args]
  fontcraft-cli outlines remove-overlaps INPUT [GLYPH...] [--keep-hinting] [--ignore-errors] [--keep-unused-subroutines] [-o OUT]
  fontcraft-cli outlines direction INPUT... [--flavour postscript|truetype|reverse] [-o OUT]
  fontcraft-cli outlines extremes INPUT... [--min-segment N] [-o OUT]
  fontcraft-cli outlines round INPUT... [--grid N] [-o OUT]
  fontcraft-cli outlines to-quadratic INPUT... [--max-error EM] [--mixed] [--interpolatable] [--keep-direction] [-o OUT | --output-dir DIR]
  fontcraft-cli outlines to-cubic INPUT... [--max-error EM] [--all-cubic] [-o OUT | --output-dir DIR]
  fontcraft-cli outlines componentize FONT.ttf [--reference UFO | --detect] [-o OUT]
  fontcraft-cli outlines decompose INPUT... [--nested-only] [--transformed-only] [-o OUT | --output-dir DIR]
";

/// The options, as data (M12.28, M12.29).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.28, M12.29
    super::not_built(&SUBCOMMAND, args)
}
