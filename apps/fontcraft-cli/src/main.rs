//! `fontcraft-cli`: FontCraft from the command line.
//!
//! ```text
//! fontcraft-cli commands [PATTERN] [--cli [--json]]
//! fontcraft-cli run [--in FILE] [--cmd ID [--params JSON | --params-file FILE]]... [--script FILE [--keep-going]] [--save [FILE]] [--export FILE]...
//! fontcraft-cli convert IN OUT [--smart-components components|outlines]
//! fontcraft-cli compile SOURCE... [--format ttf|otf]... [--container plain|woff|woff2]... [-o FILE | --output-dir DIR] …
//! fontcraft-cli info INPUT... [--section NAME,...] [--all-names] [--face N] [--json]
//! fontcraft-cli tables list FONT... [--sort file|tag|size] [--face N|all] [--json]
//! fontcraft-cli glyphs list FONT [--one-per-line | --columns COLS] [--glyphs SELECTOR] [--json]
//! fontcraft-cli mcp [--connect ADDR | --headless] [--transport stdio|http] [--listen ADDR] [--automation-read-root DIR] [--automation-write-root DIR] [--read-only] [--allow P]... [--deny P]...
//! fontcraft-cli oracle (SOURCE... | --corpus [DIR]) [--against fontc|fontmake] [--format ttf|otf] [--variable | --instances] [--json] …
//! ```
//! Every subcommand is a thin shell over engine commands (`spec/inventory/20-cli.toml` lists the
//! full planned set and the reference tools each one replaces). Results are JSON on stdout (the
//! listings `info`, `tables list` and `glyphs list` print text unless `--json` is given) and
//! messages on stderr. Exit status, shared by every subcommand (`cli.global.exitCodes`): 0 success,
//! 2 usage error, 3 an input could not be read, 4 the operation failed, 5 some inputs of a batch
//! failed. (1 is reserved for "it ran and found something": differences, failed checks.) Every
//! subcommand takes the common options of [`args::GLOBAL`], before or after its name.
#![forbid(unsafe_code)]

mod args;
mod io;
mod sub;

use std::process::ExitCode;
use std::time::Instant;

use io::{out, usage};

const USAGE: &str = "\
fontcraft-cli: FontCraft from the command line

USAGE:
  fontcraft-cli commands [PATTERN] [--cli [--json]]
      Print the command catalogue as JSON (only commands whose id or label contains PATTERN).
      --cli lists the subcommands instead, with the engine command each one runs.

  fontcraft-cli run [--in FILE] [--cmd ID [--params JSON | --params-file FILE]]...
                    [--script FILE [--keep-going]] [--save [FILE]] [--export FILE]...
      Headless batch: open FILE (a source) or start a new font, run the commands in order and
      print one JSON result per line. Stops at the first command that fails. --script runs a file
      of steps. --save writes the document (to FILE, or back to the file it was opened from);
      --export compiles it to a font.

  fontcraft-cli convert IN OUT [--smart-components components|outlines]
      Open IN (a source) and write OUT in the format its extension names (.fontcraft, .glyphs,
      .glyphspackage, .ufo for one master, .designspace with a UFO per master), or compile it to
      one static font (.ttf, .otf with CFF outlines, .woff, .woff2) of its first exportable
      instance. --smart-components: a UFO or designspace keeps smart components as components
      with their values (default) or holds them drawn as outlines.

  fontcraft-cli compile SOURCE... [--format ttf|otf]... [--container plain|woff|woff2]...
                        [-o FILE | --output-dir DIR] [--instances[=NAME,...]] [--masters-as-instances]
                        [--keep-overlaps] [--keep-direction] [--conversion-error EM] [--no-subroutinize]
                        [--timestamp SECS|now] [--ttfautohint PATH] [--cbdt] [--plan]
      Compile each SOURCE to static fonts: every exportable instance (or the named ones; a font
      without instances builds its first master), plus each master with --masters-as-instances,
      once per format (default ttf) and container (default plain). Files are named by PostScript
      name in DIR (default: the current folder); with both formats each goes into a ttf or otf
      subfolder. -o names the one file a single SOURCE builds. Instances between masters are
      skipped until interpolation exists. Overlapping contours are merged unless --keep-overlaps is
      given. CFF charstrings are subroutinised unless --no-subroutinize is given or the source
      sets Disable Subroutines. --ttfautohint PATH names the trusted ttfautohint program: a source
      whose TTFAutohint binary path parameter names that same file is autohinted by it (default:
      none runs). --cbdt also writes CBDT and CBLC bitmap tables from the iColor layers.
      --plan prints what would be built, without building.
      One JSON line per SOURCE.

  fontcraft-cli info INPUT... [--section NAME,...] [--all-names] [--face N] [--json]
      Describe each INPUT (a compiled font or a source) without opening it: format, names,
      metrics, style, tables, cmap, layout, axes, instances, masters, color, hinting, faces.
  fontcraft-cli tables list FONT... [--sort file|tag|size] [--face N|all] [--json]
      Print the table directory of each compiled font, collection, WOFF or WOFF2 file.
  fontcraft-cli glyphs list FONT [--one-per-line | --columns COLS] [--glyphs SELECTOR] [--json]
      List the glyphs of FONT (a compiled font or a source) in glyph order.

  fontcraft-cli mcp [--connect ADDR | --headless] [--transport stdio|http] [--listen ADDR]
                    [--automation-read-root DIR] [--automation-write-root DIR] [--read-only]
                    [--allow PATTERN]... [--deny PATTERN]...
      Run the MCP server on stdio. Default: attach to a running app at 127.0.0.1:7985
      (fontcraft --control 7985), falling back to a headless session with a new font.
      --transport http --listen ADDR serves HTTP instead (default 127.0.0.1:7986).
      --automation-read-root and --automation-write-root confine the files commands read and
      write (headless only); --read-only, --allow and --deny restrict the commands an agent may run.

  fontcraft-cli oracle (SOURCE... | --corpus [DIR]) [--against fontc|fontmake] [--format ttf|otf]
                      [--reference-bin PATH] [--output-dir DIR] [--timestamp SECS] [--timeout SECS]
                      [--differences N] [--match TEXT] [--variable | --instances] [--json]
      Development aid: compile each source with FontCraft and with a reference compiler (fontc
      for .ttf, fontmake for .otf), read both fonts back and report per table whether they are
      identical, equivalent, allowed or different. --corpus runs over the comparison set in DIR
      (default ./corpus). Exit status 1 when a table differs or a build fails. --variable
      compares variable fonts at locations, --instances static instances against fontmake -i.

  fontcraft-cli help [SUBCOMMAND [ACTION]] [--all] | --version
      The list of subcommands, the reference of one, or with --all the reference of every one.
";

fn main() -> ExitCode {
    let started = Instant::now();
    io::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = dispatch(&args);
    let code = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            // An empty message: the subcommand has already said everything (`check --quiet`).
            if !failure.message().is_empty() {
                io::error(failure.message());
            }
            ExitCode::from(failure.exit_code())
        }
    };
    io::report_timing(started);
    code
}

/// Runs a command line: the common options before the subcommand's name, then the subcommand.
fn dispatch(args: &[String]) -> io::Outcome {
    let leading = args::leading_globals(args);
    let (globals, rest) = args.split_at(leading);
    if !globals.is_empty() {
        args::parse(&[], globals)?;
    }
    let tail = rest.get(1..).unwrap_or_default();
    match rest.first().map(String::as_str) {
        Some("-h" | "--help") | None => print_usage(),
        Some("help") => help(tail),
        Some(name) => match sub::find(name) {
            Some(sub) => sub::run(sub, tail),
            None => Err(usage(format!("unknown subcommand `{name}`\n\n{USAGE}"))),
        },
    }
}

/// `fontcraft-cli help [SUBCOMMAND [ACTION]] [--all]`.
fn help(args: &[String]) -> io::Outcome {
    const OPTIONS: &[args::OptSpec] = &[args::OptSpec::switch("all", false, "print the reference of every subcommand")];
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.switch("all") {
        print_usage()?;
        for sub in sub::ALL {
            out(&format!("\n{}", sub::help(sub)))?;
        }
        return Ok(());
    }
    match parsed.positional.as_slice() {
        [] => print_usage(),
        [name, action @ ..] if action.len() <= 1 => {
            let sub = sub::find(name).ok_or_else(|| usage(format!("unknown subcommand `{name}`")))?;
            let mut rest: Vec<String> = action.to_vec();
            rest.push("--help".to_string());
            // An action that reads its own arguments may not know --help: the general text then.
            match sub::run(sub, &rest) {
                Err(io::Failure::Usage(_)) => out(&sub::help(sub)),
                other => other,
            }
        }
        _ => Err(usage("help takes a subcommand and at most one action")),
    }
}

/// The general usage, then the other subcommands with their summaries: built ones first, then
/// the ones whose packages have not landed.
fn print_usage() -> io::Outcome {
    out(USAGE.trim_end())?;
    let rest = || sub::ALL.iter().filter(|s| !USAGE.contains(&format!("fontcraft-cli {} ", s.name)));
    out("\nMORE SUBCOMMANDS (each prints its usage with --help):")?;
    for sub in rest().filter(|s| !sub::PLANNED.contains(&s.name)) {
        out(&format!("  {:<12} {}", sub.name, sub.summary))?;
    }
    if rest().any(|s| sub::PLANNED.contains(&s.name)) {
        out("\nPLANNED (each prints its usage with --help; not built yet):")?;
        for sub in rest().filter(|s| sub::PLANNED.contains(&s.name)) {
            out(&format!("  {:<12} {}", sub.name, sub.summary))?;
        }
    }
    out("\nCOMMON OPTIONS (every subcommand; one of its own with the same name takes precedence):")?;
    out(args::option_lines(args::GLOBAL).trim_end())
}
