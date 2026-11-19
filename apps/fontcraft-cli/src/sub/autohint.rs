//! `fontcraft-cli autohint` (M12.39): a shell over the autohinters' document commands. Each input
//! opens in a session of its own, `autohint.ps.run` (or `autohint.tt.run` for TrueType work)
//! stores the hints on the master layers, and the document is saved to the output. The options
//! are the reference tool's; those the autohinter does not offer yet are refused, never ignored.
//! Override sets come from `--fontinfo-file`, else from a file named `fontinfo` beside each input,
//! as the reference reads them for each font.
//!
//! Glyph selectors follow otfautohint's rules (AFDKO 5.0.1 `__main__.py` and `autohint.py`, Apache-2.0, Copyright 2014, 2016, 2022 Adobe; see
//! `NOTICE`): a number is a glyph index, `/N` is CID N (`cid0000N`, `/0` is `.notdef`), a name
//! the font has is that glyph even when it holds a hyphen, `A-B` is the glyphs from A to B in
//! glyph order (each end a name or an index), and an entry the font cannot resolve is a warning.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{self, Failure, Outcome, batch, batch_outcome, detail, execute, note, out, out_json_line, usage, warn};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "autohint", summary: "Add PostScript or TrueType hints", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli autohint INPUT... [-o OUT...] [--glyphs LIST] [--all] [--allow-changes] [--report-only] [options]
      Run the PostScript autohinter on each source and store its stems on the master layers,
      the masters of one glyph hinted together. Override sets in a file named fontinfo beside
      an input apply unless --ignore-fontinfo or --fontinfo-file. With --tt-options or --tt-control the TrueType
      autohinter stores its instructions on the main master instead. Layers that already have
      hints are left alone unless --all. Each output is INPUT.autohinted.EXT unless -o (one
      per input, comma-separated), --output-dir or --in-place says otherwise. A compiled font
      can only be analysed (--report-only): to hint one, autohint its source, or export the
      source with autohinting on.
  fontcraft-cli autohint INPUT --print-list-fddict | --print-all-fddict [--fontinfo-file FILE]
      List the zone-and-stem sets in effect and the glyphs each one hints, without hinting.
";

/// Largest glyph list, fontinfo or control-instruction file read, in bytes.
pub(super) const MAX_TEXT_FILE: usize = 16 << 20;
/// Most unresolved glyph selector entries reported one by one.
const MAX_SELECTOR_WARNINGS: usize = 20;
/// The file of override sets the reference reads from the folder of each font.
const FONTINFO_FILE_NAME: &str = "fontinfo";
/// Inserted between the input's name and its extension when no output is named.
const OUTPUT_SUFFIX: &str = ".autohinted";
/// The options of the reference's overlap group, of which one may be given.
const OVERLAP_OPTIONS: [&str; 5] = ["overlap-glyphs", "overlaps-file", "force-overlap", "force-no-overlap", "loose-overlap-mapping"];
/// The options only the PostScript autohinter takes.
const POSTSCRIPT_OPTIONS: [&str; 11] = [
    "flex",
    "hint-sub",
    "max-segments",
    "fontinfo-file",
    "overlap-glyphs",
    "overlaps-file",
    "force-overlap",
    "loose-overlap-mapping",
    "keep-conflicts",
    "allow-changes",
    "decimal",
];

/// The options, as data (M12.39).
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("glyphs", "SELECTOR", "", "hint only these glyphs: names, indexes, /CIDs and A-B ranges").short('g'),
    OptSpec::value("glyphs-file", "FILE", "", "read the glyphs to hint from FILE (comma or whitespace separated)"),
    OptSpec::list("exclude-glyphs", "SELECTOR", "", "hint every glyph but these").short('x'),
    OptSpec::value("exclude-glyphs-file", "FILE", "", "read the glyphs not to hint from FILE"),
    OptSpec::switch("all", false, "also hint layers that already have hints, replacing them").short('a'),
    OptSpec::value("reference", "FONT", "", "hint compatibly with this reference master (not built yet: hint one source with several masters)")
        .short('r'),
    OptSpec::value("reference-out", "PATH", "", "where to write the hinted reference (not built yet)").short('b'),
    OptSpec::list("overlap-glyphs", "SELECTOR", "", "hint these glyphs as if their overlapping contours were merged").short('m'),
    OptSpec::value("overlaps-file", "FILE", "", "read the overlap glyphs from FILE"),
    OptSpec::switch("force-overlap", false, "account for overlaps in every glyph"),
    OptSpec::switch("force-no-overlap", false, "account for overlaps in no glyph (default: those hinted in several masters)"),
    OptSpec::switch("loose-overlap-mapping", false, "match the overlap-free outline to the original loosely"),
    OptSpec::value("write-layer", "NAME|processed", "", "the UFO layer that receives the hints (not built yet: hints go on the master layers)"),
    OptSpec::switch("write-to-default-layer", false, "write the hints to the default layer, where they always go").short('w'),
    OptSpec::switch("keep-conflicts", false, "keep a stem whose order against another changes between masters").short('k'),
    OptSpec::switch(
        "allow-changes",
        false,
        "let the autohinter reorder contours and straighten nearly straight curves while it works (outlines are kept)",
    )
    .short('c'),
    OptSpec::switch("report-only", false, "hint and report without writing anything"),
    OptSpec::switch("decimal", false, "keep fractional coordinates instead of rounding them before hinting").short('d'),
    OptSpec::switch("flex", true, "generate flex at export; --no-flex turns it off (PostScript)"),
    OptSpec::switch("hint-sub", true, "use hint substitution; --no-hint-sub keeps one hint set per glyph (PostScript)"),
    OptSpec::switch("zones-stems", true, "--no-zones-stems: hint without alignment zones or stem widths (the autohinter always may, with a warning)"),
    OptSpec::value("fontinfo-file", "FILE", "", "hint with the override sets (FDDict and GlyphSet blocks) in FILE"),
    OptSpec::switch("ignore-fontinfo", false, "do not read the file named fontinfo beside each input"),
    OptSpec::switch("print-list-fddict", false, "list the zone-and-stem sets of the default master and their glyphs, then stop"),
    OptSpec::switch("print-all-fddict", false, "the same for every master"),
    OptSpec::value("max-segments", "N", "", "leave a direction with more segments unhinted (default and at most 100; PostScript)"),
    OptSpec::value("tt-options", "STRING", "", "run the TrueType autohinter with these options (long command-line options)"),
    OptSpec::value("tt-control", "FILE", "", "run the TrueType autohinter with the control instructions in FILE"),
    OptSpec::list("output", "OUT", "", "the outputs, one per input").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder outputs go into"),
    OptSpec::switch("in-place", false, "replace each input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N", "", "the font of a collection to read"),
    OptSpec::switch("dry-run", false, "hint and report what would be written, without writing"),
    OptSpec::switch("json", false, "print one JSON result per input"),
];

/// Which autohinter a run uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hinter {
    PostScript,
    TrueType,
}

impl Hinter {
    fn command(self) -> &'static str {
        match self {
            Hinter::PostScript => "autohint.ps.run",
            Hinter::TrueType => "autohint.tt.run",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Hinter::PostScript => "postscript",
            Hinter::TrueType => "truetype",
        }
    }
}

/// Where one input's result goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// Back into the input's own file (`file.save`).
    InPlace,
    /// A new file (`file.saveAs`), its format from the extension.
    File(PathBuf),
}

/// One input done.
struct Done {
    input: String,
    output: Option<String>,
    written: bool,
    hinter: Hinter,
    result: Value,
}

fn run(raw: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, raw)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    let selection = Selection::from_options(&parsed)?;
    let fontinfo = Fontinfo::from_options(&parsed)?;
    if parsed.switch("print-list-fddict") || parsed.switch("print-all-fddict") {
        return print_fd_dicts(&parsed, selection.as_ref(), &fontinfo);
    }
    refuse_unbuilt(&parsed)?;
    refuse_conflicts(&parsed)?;
    let hinter = choose_hinter(&parsed)?;
    let params = hint_params(&parsed, hinter)?;
    let overlaps = Selection::overlaps(&parsed)?;
    let outputs = parsed.list("output");
    refuse_repeats(&parsed.positional)?;
    if !outputs.is_empty() && outputs.len() != parsed.positional.len() {
        return Err(usage(format!("{} inputs and {} outputs: give one output per input", parsed.positional.len(), outputs.len())));
    }
    let write = !parsed.switch("report-only") && !parsed.switch("dry-run");
    let jobs = parsed.jobs()?;
    let as_json = parsed.switch("json");
    let extra = Extra { selection: selection.as_ref(), overlaps: overlaps.as_ref(), fontinfo: (hinter == Hinter::PostScript).then_some(&fontinfo) };

    let results = batch(&parsed.positional, jobs, |input| {
        let index = parsed.positional.iter().position(|p| p == input).unwrap_or(0);
        let target = target(&parsed, input, outputs.get(index).map(String::as_str), OUTPUT_SUFFIX)?;
        hint_one(&parsed, input, &target, hinter, &params, &extra, write)
    })?;
    for r in &results {
        match r {
            Ok(done) if as_json => out_json_line(&json!({
                "input": done.input,
                "output": done.output,
                "written": done.written,
                "hinter": done.hinter.name(),
                "result": done.result,
            }))?,
            Ok(done) => print_done(done, parsed.switch("report-only"))?,
            Err(e) => eprintln!("fontcraft-cli: {}", e.message()),
        }
    }
    batch_outcome(&results)
}

/// The reference refuses an input given twice; here the outputs are matched to inputs by name.
fn refuse_repeats(inputs: &[String]) -> Outcome {
    let mut seen = std::collections::HashSet::new();
    match inputs.iter().find(|i| !seen.insert(i.as_str())) {
        Some(again) => Err(usage(format!("{again} is given twice"))),
        None => Ok(()),
    }
}

/// The message level from `stems`' own -q and -v.
pub(super) fn set_verbosity(parsed: &Parsed) {
    io::set_verbosity(if parsed.switch("quiet") {
        0
    } else if parsed.switch("verbose") {
        2
    } else {
        1
    });
}

/// `stems`' own --jobs.
pub(super) fn jobs(parsed: &Parsed) -> Result<usize, Failure> {
    parsed.value("jobs").map_or(Ok(1), str::parse::<usize>).map_err(|_| usage("--jobs needs a number"))
}

/// The reference tool's options the autohinter does not offer yet.
fn refuse_unbuilt(parsed: &Parsed) -> Outcome {
    match ["reference", "reference-out", "write-layer"].into_iter().find(|name| parsed.value(name).is_some()) {
        Some(name) => Err(not_built(name)),
        None => Ok(()),
    }
}

pub(super) fn not_built(option: &str) -> Failure {
    let hint = match option {
        "reference" | "reference-out" => ": hint one source with several masters, whose masters are hinted together",
        "write-layer" => ": hints are stored on the master layers, which a UFO keeps in its default layer",
        _ => "",
    };
    Failure::Failed(format!("--{option} is not built yet{hint}"))
}

/// Whether option `name` is on the command line: a switch turned on, a value or a list given.
fn given(parsed: &Parsed, name: &str) -> bool {
    parsed.switch(name) || parsed.value(name).is_some() || !parsed.list(name).is_empty()
}

/// The combinations the reference refuses: more than one option of the overlap group, and
/// --allow-changes with --report-only.
fn refuse_conflicts(parsed: &Parsed) -> Outcome {
    let overlap: Vec<&str> = OVERLAP_OPTIONS.into_iter().filter(|name| given(parsed, name)).collect();
    if overlap.len() > 1 {
        return Err(usage(format!("give one of --{}", overlap.join(", --"))));
    }
    if parsed.switch("allow-changes") && parsed.switch("report-only") {
        return Err(usage("--allow-changes changes what is written, and --report-only writes nothing: give one"));
    }
    Ok(())
}

/// The TrueType autohinter when its options or control instructions are given, or when the
/// first input is a compiled TrueType font; the PostScript one otherwise.
fn choose_hinter(parsed: &Parsed) -> Result<Hinter, Failure> {
    let tt_options = parsed.value("tt-options").is_some() || parsed.value("tt-control").is_some();
    let tt_input = parsed.positional.first().is_some_and(|p| is_truetype_font(Path::new(p)));
    if !(tt_options || tt_input) {
        return Ok(Hinter::PostScript);
    }
    // --flex and --hint-sub are on by default: only their --no- forms count as given.
    let ps_only: Vec<&str> = POSTSCRIPT_OPTIONS
        .into_iter()
        .filter(|name| match *name {
            "flex" | "hint-sub" => !parsed.switch(name),
            _ => given(parsed, name),
        })
        .collect();
    if let Some(name) = ps_only.first() {
        let shown = if matches!(*name, "flex" | "hint-sub") { format!("no-{name}") } else { name.to_string() };
        return Err(usage(format!("--{shown} is an option of the PostScript autohinter; the TrueType autohinter takes --tt-options")));
    }
    Ok(Hinter::TrueType)
}

/// A compiled font with TrueType outlines, plain or in WOFF or WOFF2, by its first bytes.
fn is_truetype_font(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 8];
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    if file.read_exact(&mut head).is_err() {
        return false;
    }
    let (signature, flavour) = head.split_at(4);
    let tag = if signature == b"wOFF" || signature == b"wOF2" { flavour } else { signature };
    tag == [0, 1, 0, 0] || tag == b"true"
}

/// The engine parameters every input shares.
fn hint_params(parsed: &Parsed, hinter: Hinter) -> Result<Map<String, Value>, Failure> {
    let mut params = Map::new();
    match hinter {
        Hinter::PostScript => {
            params.insert("flex".into(), json!(parsed.switch("flex")));
            params.insert("hintSubstitution".into(), json!(parsed.switch("hint-sub")));
            params.insert("hintAll".into(), json!(parsed.switch("all")));
            params.insert("allowChanges".into(), json!(parsed.switch("allow-changes")));
            params.insert("keepConflicts".into(), json!(parsed.switch("keep-conflicts")));
            params.insert("decimal".into(), json!(parsed.switch("decimal")));
            params.insert("looseOverlapMapping".into(), json!(parsed.switch("loose-overlap-mapping")));
            let overlap = if parsed.switch("force-overlap") {
                "all"
            } else if parsed.switch("force-no-overlap") {
                "none"
            } else {
                "auto"
            };
            params.insert("overlap".into(), json!(overlap));
            if let Some(text) = parsed.value("max-segments") {
                let n: u32 = text.parse().map_err(|_| usage(format!("--max-segments takes a whole number, not `{text}`")))?;
                params.insert("maxSegments".into(), json!(n));
            }
        }
        Hinter::TrueType => {
            if let Some(options) = parsed.value("tt-options") {
                params.insert("options".into(), json!(options));
            }
            if let Some(file) = parsed.value("tt-control") {
                params.insert("controlInstructions".into(), json!(read_text(file)?));
            }
        }
    }
    Ok(params)
}

/// Where `input`'s result goes: `-o`'s entry, else `--in-place`, else the input's name with
/// `suffix` before its extension, in `--output-dir` or beside it.
pub(super) fn target(parsed: &Parsed, input: &str, named: Option<&str>, suffix: &str) -> Result<Target, Failure> {
    let input_path = Path::new(input.trim_end_matches(['/', '\\']));
    if parsed.switch("in-place") {
        if named.is_some() || parsed.value("output-dir").is_some() {
            return Err(usage("--in-place writes over the inputs: leave out -o and --output-dir"));
        }
        return Ok(Target::InPlace);
    }
    let path = match named {
        Some(o) => PathBuf::from(o),
        None => {
            let stem = input_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
            let name = match input_path.extension() {
                Some(ext) => format!("{stem}{suffix}.{}", ext.to_string_lossy()),
                None => format!("{stem}{suffix}"),
            };
            let dir = parsed.value("output-dir").map(PathBuf::from).or_else(|| input_path.parent().map(Path::to_path_buf)).unwrap_or_default();
            dir.join(name)
        }
    };
    if same_path(&path, input_path) {
        return Err(usage(format!("the output {} is the input: give --in-place to replace it", path.display())));
    }
    // A designspace writes a UFO per master beside itself, under names that can be the input's.
    let is_designspace = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("designspace"));
    if is_designspace(input_path) && is_designspace(&path) && same_path(&folder(&path), &folder(input_path)) {
        return Err(usage(format!("{} would write its UFOs over the input's: give --output-dir with another folder, or --in-place", path.display())));
    }
    Ok(Target::File(path))
}

fn folder(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The file of override sets beside `input`, when there is one.
fn fontinfo_beside(input: &str) -> Option<PathBuf> {
    let path = folder(Path::new(input.trim_end_matches(['/', '\\']))).join(FONTINFO_FILE_NAME);
    path.is_file().then_some(path)
}

/// Where the override sets come from.
enum Fontinfo {
    /// The text of `--fontinfo-file`, for every input.
    File(String),
    /// The file named fontinfo beside each input, when there is one.
    Beside,
    /// `--ignore-fontinfo`.
    Ignored,
}

impl Fontinfo {
    fn from_options(parsed: &Parsed) -> Result<Fontinfo, Failure> {
        Ok(match parsed.value("fontinfo-file") {
            Some(file) => Fontinfo::File(read_text(file)?),
            None if parsed.switch("ignore-fontinfo") => Fontinfo::Ignored,
            None => Fontinfo::Beside,
        })
    }

    /// The override sets for `input`.
    fn text_for(&self, input: &str) -> Result<Option<String>, Failure> {
        match self {
            Fontinfo::File(text) => Ok(Some(text.clone())),
            Fontinfo::Ignored => Ok(None),
            Fontinfo::Beside => match fontinfo_beside(input) {
                Some(path) => {
                    detail(&format!("{input}: override sets from {}", path.display()));
                    read_text(&path.to_string_lossy()).map(Some)
                }
                None => Ok(None),
            },
        }
    }
}

/// What each input resolves for itself.
struct Extra<'a> {
    selection: Option<&'a Selection>,
    overlaps: Option<&'a Selection>,
    /// `None` for the TrueType autohinter, which takes no override sets.
    fontinfo: Option<&'a Fontinfo>,
}

/// Opens `input`, hints it and, when `write`, saves it to `target`.
fn hint_one(
    parsed: &Parsed,
    input: &str,
    target: &Target,
    hinter: Hinter,
    shared: &Map<String, Value>,
    extra: &Extra,
    write: bool,
) -> Result<Done, Failure> {
    let output = match target {
        Target::InPlace => input.to_string(),
        Target::File(p) => p.to_string_lossy().into_owned(),
    };
    if write && parsed.switch("no-clobber") && matches!(target, Target::File(p) if p.exists()) {
        return Err(Failure::Failed(format!("{output} exists and --no-clobber is set")));
    }
    let mut session = Session::new();
    let opened = open(&mut session, input, parsed.value("face"))?;
    if write && opened.get("viewOnly").and_then(Value::as_bool) == Some(true) {
        return Err(Failure::Input(format!(
            "{input} is a compiled font, which autohint can only analyse (--report-only): autohint its source, or export the source with autohinting on"
        )));
    }
    let mut params = shared.clone();
    if let Some(sel) = extra.selection {
        let (key, names) = sel.resolve(&mut session, input)?;
        params.insert(key.into(), json!(names));
    }
    if let Some(sel) = extra.overlaps {
        let (_, names) = sel.resolve(&mut session, input)?;
        params.insert("overlapGlyphs".into(), json!(names));
    }
    if let Some(text) = extra.fontinfo.map(|f| f.text_for(input)).transpose()?.flatten() {
        params.insert("fontinfo".into(), json!(text));
    }
    let result = execute(&mut session, hinter.command(), &Value::Object(params))?;
    if !write {
        return Ok(Done { input: input.to_string(), output: Some(output), written: false, hinter, result });
    }
    save(&mut session, target)?;
    Ok(Done { input: input.to_string(), output: Some(output), written: true, hinter, result })
}

/// Saves the session's document to `target`, making the output's folder when it is missing.
pub(super) fn save(session: &mut Session, target: &Target) -> Outcome {
    match target {
        Target::InPlace => execute(session, "file.save", &json!({}))?,
        Target::File(path) => {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
                std::fs::create_dir_all(dir).map_err(|e| Failure::Failed(format!("cannot create {}: {e}", dir.display())))?;
            }
            execute(session, "file.saveAs", &json!({"path": path}))?
        }
    };
    Ok(())
}

/// Opens `input` (a collection member with `face`) as the session's document.
pub(super) fn open(session: &mut Session, input: &str, face: Option<&str>) -> Result<Value, Failure> {
    let mut params = Map::new();
    params.insert("path".into(), json!(input));
    // The hinted font keeps the glyph names the input gives (see `io::open`).
    params.insert("keepGlyphNames".into(), json!(true));
    if let Some(face) = face {
        let n: u32 = face.parse().map_err(|_| usage(format!("--face takes a font number, not `{face}`")))?;
        params.insert("face".into(), json!(n));
    }
    session.execute("file.open", &Value::Object(params)).map_err(|e| Failure::Input(format!("{input}: {e}")))
}

fn print_done(done: &Done, list_glyphs: bool) -> Outcome {
    let entries = |key: &str| done.result.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let hinted = entries("hinted");
    let skipped = entries("skipped");
    for w in entries("warnings") {
        note(&format!("{}: {}", done.input, w.as_str().map_or_else(|| w.to_string(), str::to_string)));
    }
    for s in &skipped {
        detail(&format!("{}: {} skipped: {}", done.input, entry_name(s), s.get("reason").and_then(Value::as_str).unwrap_or("no reason given")));
    }
    if list_glyphs {
        for h in &hinted {
            let counts = match (h.get("horizontal").and_then(Value::as_u64), h.get("vertical").and_then(Value::as_u64)) {
                (Some(hz), Some(v)) => format!(": {hz} horizontal, {v} vertical"),
                _ => String::new(),
            };
            out(&format!("{}{counts}", entry_name(h)))?;
        }
    }
    let counts =
        format!("{} {} hinted, {} skipped", hinted.len(), if done.hinter == Hinter::PostScript { "layers" } else { "glyphs" }, skipped.len());
    match (&done.output, done.written) {
        (Some(output), true) => out(&format!("{} -> {output}: {counts}", done.input)),
        (Some(output), false) if !list_glyphs => out(&format!("{}: {counts}; would write {output}", done.input)),
        _ => out(&format!("{}: {counts}; nothing written", done.input)),
    }
}

/// `glyph` or `glyph [master]` of a result entry.
fn entry_name(entry: &Value) -> String {
    let glyph = entry.get("glyph").and_then(Value::as_str).unwrap_or("?");
    match entry.get("master").and_then(Value::as_str) {
        Some(m) => format!("{glyph} [{m}]"),
        None => glyph.to_string(),
    }
}

/// `--print-list-fddict` and `--print-all-fddict`: `autohint.ps.listFdDicts` on each input.
fn print_fd_dicts(parsed: &Parsed, selection: Option<&Selection>, fontinfo: &Fontinfo) -> Outcome {
    if selection.is_some() {
        note("the glyph selection does not apply to the hint dictionary list");
    }
    let results = batch(&parsed.positional, parsed.jobs()?, |input| {
        let mut params = Map::new();
        params.insert("allInstances".into(), json!(parsed.switch("print-all-fddict")));
        if let Some(text) = fontinfo.text_for(input)? {
            params.insert("fontinfo".into(), json!(text));
        }
        let mut session = Session::new();
        open(&mut session, input, parsed.value("face"))?;
        Ok((input.to_string(), execute(&mut session, "autohint.ps.listFdDicts", &Value::Object(params))?))
    })?;
    for r in &results {
        match r {
            Ok((input, result)) if parsed.switch("json") => out_json_line(&json!({"input": input, "result": result}))?,
            Ok((_, result)) => out(&fd_dict_text(result))?,
            Err(e) => eprintln!("fontcraft-cli: {}", e.message()),
        }
    }
    batch_outcome(&results)
}

/// The dictionary list as text: each set's font name, name, keys and values, then its glyphs.
fn fd_dict_text(result: &Value) -> String {
    let mut text = String::from("Private Dictionaries:\n\n");
    let dicts = result.get("dicts").and_then(Value::as_array).cloned().unwrap_or_default();
    let all = dicts.iter().chain(result.get("final").filter(|f| f.is_object()));
    for d in all {
        let field = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(master) = d.get("master").and_then(Value::as_str) {
            text.push_str(&format!("master {master}\n"));
        }
        text.push_str(&format!("{}\n{}\n", field("fontName"), field("name")));
        for (key, label) in
            [("blueValues", "BlueValues"), ("otherBlues", "OtherBlues"), ("familyBlues", "FamilyBlues"), ("familyOtherBlues", "FamilyOtherBlues")]
        {
            let values: Vec<String> =
                d.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_array).flatten().map(number).collect();
            if !values.is_empty() {
                text.push_str(&format!("{label} [{}]\n", values.join(" ")));
            }
        }
        for (key, label) in [("blueFuzz", "BlueFuzz"), ("languageGroup", "LanguageGroup")] {
            if let Some(v) = d.get(key).filter(|v| v.is_number()) {
                text.push_str(&format!("{label} {}\n", number(v)));
            }
        }
        for (key, label) in [("dominantV", "DominantV"), ("dominantH", "DominantH")] {
            let values: Vec<String> = d.get(key).and_then(Value::as_array).into_iter().flatten().map(number).collect();
            if !values.is_empty() {
                text.push_str(&format!("{label} [{}]\n", values.join(" ")));
            }
        }
        if let Some(flex) = d.get("flexOK").and_then(Value::as_bool) {
            text.push_str(&format!("FlexOK {flex}\n"));
        }
        for (key, label) in [("vCounterChars", "VCounterChars"), ("hCounterChars", "HCounterChars")] {
            let values: Vec<&str> = d.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            if !values.is_empty() {
                text.push_str(&format!("{label} [{}]\n", values.join(" ")));
            }
        }
        if let Some(glyphs) = d.get("glyphs").and_then(Value::as_array) {
            let names: Vec<&str> = glyphs.iter().filter_map(Value::as_str).collect();
            text.push_str(&format!("{} glyphs:\n{}\n", names.len(), if names.is_empty() { "None".to_string() } else { names.join(" ") }));
        }
        text.push('\n');
    }
    text.trim_end().to_string()
}

/// A JSON number as the reference prints it: whole numbers without a fraction.
fn number(v: &Value) -> String {
    match v.as_f64() {
        Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
        Some(f) => format!("{f}"),
        None => v.to_string(),
    }
}

/// The comma- or whitespace-separated entries of a glyph list file.
fn read_entries(path: &str) -> Result<Vec<String>, Failure> {
    let text = read_text(path)?;
    Ok(text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(str::to_string).collect())
}

/// A text file named by an option, at most [`MAX_TEXT_FILE`] bytes.
pub(super) fn read_text(path: &str) -> Result<String, Failure> {
    let bytes = io::read_input(path)?;
    if bytes.len() > MAX_TEXT_FILE {
        return Err(Failure::Input(format!("{path} is larger than {MAX_TEXT_FILE} bytes")));
    }
    String::from_utf8(bytes).map_err(|_| Failure::Input(format!("{path} is not UTF-8 text")))
}

/// A glyph selection from the four mutually exclusive options.
pub(super) struct Selection {
    exclude: bool,
    entries: Vec<String>,
}

impl Selection {
    /// The selection the command line gives, if any; more than one of the four forms is a usage
    /// error.
    pub(super) fn from_options(parsed: &Parsed) -> Result<Option<Selection>, Failure> {
        let given: Vec<&str> = [
            ("glyphs", !parsed.list("glyphs").is_empty()),
            ("exclude-glyphs", !parsed.list("exclude-glyphs").is_empty()),
            ("glyphs-file", parsed.value("glyphs-file").is_some()),
            ("exclude-glyphs-file", parsed.value("exclude-glyphs-file").is_some()),
        ]
        .into_iter()
        .filter_map(|(name, set)| set.then_some(name))
        .collect();
        let name = match given.as_slice() {
            [] => return Ok(None),
            [one] => *one,
            many => return Err(usage(format!("give one of --{}", many.join(", --")))),
        };
        let exclude = name.starts_with("exclude");
        let entries = match name {
            "glyphs" | "exclude-glyphs" => parsed.list(name).to_vec(),
            _ => read_entries(parsed.value(name).unwrap_or_default())?,
        };
        Ok(Some(Selection { exclude, entries }))
    }

    /// The glyphs `--overlap-glyphs` or `--overlaps-file` names, if either is given (the two
    /// exclude each other, see [`refuse_conflicts`]).
    fn overlaps(parsed: &Parsed) -> Result<Option<Selection>, Failure> {
        let entries = match parsed.value("overlaps-file") {
            Some(file) => read_entries(file)?,
            None if !parsed.list("overlap-glyphs").is_empty() => parsed.list("overlap-glyphs").to_vec(),
            None => return Ok(None),
        };
        Ok(Some(Selection { exclude: false, entries }))
    }

    /// The glyph names the selection resolves to in `order` (a font's glyph order): the entries
    /// in the order given without repeats, or for an exclusion every other glyph in glyph
    /// order. Entries the font cannot resolve are reported for `input` and left out.
    pub(super) fn names_in(&self, order: &[String], input: &str) -> Vec<String> {
        let (names, unresolved) = resolve_entries(&self.entries, order);
        for warning in unresolved.iter().take(MAX_SELECTOR_WARNINGS) {
            warn(&format!("{input}: {warning}"));
        }
        if unresolved.len() > MAX_SELECTOR_WARNINGS {
            warn(&format!("{input}: {} more glyph selector entries not in the font", unresolved.len() - MAX_SELECTOR_WARNINGS));
        }
        if !self.exclude {
            return names;
        }
        let excluded: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
        order.iter().filter(|n| !excluded.contains(n.as_str())).cloned().collect()
    }

    /// The parameter (`glyphs` or `excludeGlyphs`) and the glyph names the selection resolves
    /// to in the open document, in the order given, without repeats. Entries the font cannot
    /// resolve are reported and left out.
    pub(super) fn resolve(&self, session: &mut Session, input: &str) -> Result<(&'static str, Vec<String>), Failure> {
        let listed = execute(session, "font.listGlyphs", &json!({}))?;
        let order: Vec<String> = listed
            .get("glyphs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|g| g.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        let (names, unresolved) = resolve_entries(&self.entries, &order);
        for warning in unresolved.iter().take(MAX_SELECTOR_WARNINGS) {
            warn(&format!("{input}: {warning}"));
        }
        if unresolved.len() > MAX_SELECTOR_WARNINGS {
            warn(&format!("{input}: {} more glyph selector entries not in the font", unresolved.len() - MAX_SELECTOR_WARNINGS));
        }
        Ok((if self.exclude { "excludeGlyphs" } else { "glyphs" }, names))
    }
}

/// Resolves selector entries against the glyph order: the names, and a warning per entry that
/// names nothing.
fn resolve_entries(entries: &[String], order: &[String]) -> (Vec<String>, Vec<String>) {
    let index: HashMap<&str, usize> = order.iter().enumerate().rev().map(|(i, n)| (n.as_str(), i)).collect();
    let mut seen = vec![false; order.len()];
    let mut names = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        match resolve_entry(entry, order, &index) {
            Ok(gids) => {
                for gid in gids {
                    if let (Some(flag), Some(name)) = (seen.get_mut(gid), order.get(gid))
                        && !*flag
                    {
                        *flag = true;
                        names.push(name.clone());
                    }
                }
            }
            Err(w) => warnings.push(w),
        }
    }
    (names, warnings)
}

/// The glyph indexes one entry stands for.
fn resolve_entry(entry: &str, order: &[String], index: &HashMap<&str, usize>) -> Result<Vec<usize>, String> {
    let spec = match entry.split('-').collect::<Vec<_>>().as_slice() {
        [one] => cid_name(one),
        [a, b] if !a.is_empty() && !b.is_empty() => format!("{}-{}", cid_name(a), cid_name(b)),
        _ => entry.to_string(),
    };
    let gid = |s: &str| -> Option<usize> {
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse::<usize>().ok().filter(|&g| g < order.len())
        } else {
            index.get(s).copied()
        }
    };
    if let Some(g) = gid(&spec) {
        return Ok(vec![g]);
    }
    let parts: Vec<&str> = spec.split('-').collect();
    match parts.as_slice() {
        [_] => Err(format!("glyph `{entry}` is not in the font")),
        [a, b] => match (gid(a), gid(b)) {
            // A range written backwards is empty, as the reference reads it.
            (Some(x), Some(y)) => Ok((x..=y).collect()),
            _ => Err(format!("glyph range `{entry}` names a glyph that is not in the font")),
        },
        _ => Err(format!("glyph range `{entry}` has too many hyphens")),
    }
}

/// `/N` and short `cidN` as the CID glyph name `cid0000N`; CID 0 is `.notdef`.
fn cid_name(token: &str) -> String {
    let digits = match token.strip_prefix('/') {
        Some(d) => d,
        None => match token.strip_prefix("cid") {
            Some(d) if token.len() < 8 => d,
            _ => return token.to_string(),
        },
    };
    let name = format!("cid{digits:0>5}");
    if name == "cid00000" { ".notdef".to_string() } else { name }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn selectors_resolve_names_indexes_ranges_and_cids() {
        let order = strings(&[".notdef", "a", "b", "c", "a-b", "cid00012"]);
        let (names, warnings) = resolve_entries(&strings(&["2", "a-b", "a-c", "/12", "/0", "zz", "x-y", "c-a", "a"]), &order);
        // `a-b` exists as a name, so it is that glyph; `a-c` is a range; repeats are dropped.
        assert_eq!(names, strings(&["b", "a-b", "a", "c", "cid00012", ".notdef"]));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        let (names, warnings) = resolve_entries(&strings(&["9", "a-b-c"]), &order);
        assert!(names.is_empty());
        assert_eq!(warnings.len(), 2, "an index past the end and a double hyphen are warnings");
    }

    #[test]
    fn autohint_refuses_unbuilt_options_and_bad_combinations() {
        let args = |list: &[&str]| strings(list);
        let e = run(&args(&["in.ufo", "--reference", "r.ufo"])).unwrap_err();
        assert_eq!(e.exit_code(), 4);
        assert!(e.message().contains("not built"), "{}", e.message());
        let e = run(&args(&["in.ufo", "--glyphs", "a", "--exclude-glyphs", "b"])).unwrap_err();
        assert_eq!(e.exit_code(), 2, "the selection forms exclude each other");
        let e = run(&args(&["in.ufo", "--tt-options", "--hinting-range-min=8", "--no-flex"])).unwrap_err();
        assert_eq!(e.exit_code(), 2, "PostScript options with the TrueType autohinter");
        let e = run(&args(&["a.ufo", "b.ufo", "-o", "x.ufo"])).unwrap_err();
        assert_eq!(e.exit_code(), 2, "one output per input");
        let e = run(&args(&["/nonexistent/in.ufo"])).unwrap_err();
        assert_eq!(e.exit_code(), 3);
        assert!(run(&args(&["--help"])).is_ok());
    }

    #[test]
    fn autohint_refuses_what_the_reference_refuses_together() {
        let code = |list: &[&str]| run(&strings(list)).unwrap_err().exit_code();
        // One option of the overlap group at a time, as the reference's parser has it.
        assert_eq!(code(&["in.ufo", "--force-overlap", "--force-no-overlap"]), 2);
        assert_eq!(code(&["in.ufo", "--force-overlap", "--loose-overlap-mapping"]), 2);
        assert_eq!(code(&["in.ufo", "--overlap-glyphs", "a", "--force-overlap"]), 2);
        assert_eq!(code(&["in.ufo", "--allow-changes", "--report-only"]), 2);
        // PostScript options with the TrueType autohinter.
        assert_eq!(code(&["in.ufo", "--tt-options", "--hinting-range-min=8", "--decimal"]), 2);
        assert_eq!(code(&["in.ufo", "--tt-options", "--hinting-range-min=8", "--force-overlap"]), 2);
        // Built options get past the checks to the missing input.
        assert_eq!(code(&["/nonexistent/in.ufo", "--keep-conflicts", "--decimal", "--allow-changes", "-w", "--force-overlap"]), 3);
        assert_eq!(code(&["/nonexistent/in.ufo", "--write-layer", "processed"]), 4, "a layer other than the default is not built");
    }

    #[test]
    fn outputs_default_beside_the_input_and_never_replace_it() {
        let parsed = args::parse(OPTIONS, &strings(&["dir/Font.ufo"])).unwrap();
        assert_eq!(target(&parsed, "dir/Font.ufo/", None, OUTPUT_SUFFIX).unwrap(), Target::File(PathBuf::from("dir/Font.autohinted.ufo")));
        assert!(target(&parsed, "dir/Font.ufo", Some("dir/Font.ufo"), OUTPUT_SUFFIX).is_err());
        assert!(target(&parsed, "dir/Font.designspace", Some("dir/Other.designspace"), OUTPUT_SUFFIX).is_err(), "its UFOs would land on the input's");
        let parsed = args::parse(OPTIONS, &strings(&["Font.ufo", "--in-place"])).unwrap();
        assert_eq!(target(&parsed, "Font.ufo", None, OUTPUT_SUFFIX).unwrap(), Target::InPlace);
    }
}
