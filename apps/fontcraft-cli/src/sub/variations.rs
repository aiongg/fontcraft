//! `fontcraft-cli variations` (M12.33, M12.34). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "variations",
    summary: "Variation utilities: coordinates, avar, HVAR, models",
    usage: USAGE,
    options: OPTIONS,
    actions: &["build-avar", "plan-avar", "map", "build-hvar", "model", "plot"],
    run,
};

const USAGE: &str = "\
  fontcraft-cli variations build-avar|plan-avar|map|build-hvar|model|plot INPUT [args]
  fontcraft-cli variations build-avar FONT [DESIGNSPACE] [-o OUT]
  fontcraft-cli variations plan-avar FONT [--weights LIST] [--widths LIST] [--slants LIST] [--sizes LIST] [--samples N] [--glyphs LIST] [--pins AXIS=FROM:TO,...] [--design-limits AXIS=MIN:DEF:MAX] [--sanitize] [--plot FILE] [-o OUT]
  fontcraft-cli variations map FONT [AXIS=VALUE...] [--from user|normalized] [--to design|normalized|user] [--all-axes]
  fontcraft-cli variations build-hvar FONT [--no-vvar] [--replace] [-o OUT]
  fontcraft-cli variations model (DESIGNSPACE | --locations LOC...) [--json]
  fontcraft-cli variations plot (DESIGNSPACE | FONT | --locations LOC...) [-o OUT.svg]
";

/// The options, as data (M12.33, M12.34).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.33, M12.34
    super::not_built(&SUBCOMMAND, args)
}
