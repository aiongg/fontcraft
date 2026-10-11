//! `fontcraft-cli optimize` (M12.26). Until its package lands it prints its usage with `--help`
//! and exits 4 with "not built yet" otherwise.

use crate::args::OptSpec;
use crate::io::Outcome;

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "optimize", summary: "Make a font smaller without changing what it does", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli optimize FONT... [--for size|speed] [--subroutinize | --desubroutinize] [--cff-widths] [--charstrings specialize|generalize] [--gpos-compaction N] [--varstore] [--gvar-iup] [options] [-o OUT]
";

/// The options, as data (M12.26).
const OPTIONS: &[OptSpec] = &[];

fn run(args: &[String]) -> Outcome {
    // M12.26
    super::not_built(&SUBCOMMAND, args)
}
