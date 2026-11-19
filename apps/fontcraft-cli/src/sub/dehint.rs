//! `fontcraft-cli dehint` (M12.27, M10.36): one `hints.remove` run per compiled font. A source
//! opens in a session of its own instead: `hint.ps.clearGlyphHints` removes the PostScript hints
//! of its layers, `hint.tt.removeHints` the TrueType instructions of its main master, and the
//! document is saved to the output. Its font-wide hinting data (zones, stems, TrueType font
//! programs and settings) is kept: it belongs to the design as much as to the hints.

use std::path::Path;

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, Parsed};
use crate::io::{Failure, Outcome, batch, batch_outcome, execute, note, out, out_json_line, usage, warn};

use super::Subcommand;
use super::autohint::{self, Selection, Target};

/// Inserted between the input's name and its extension when no output is named.
const OUTPUT_SUFFIX: &str = ".dehinted";
/// The first bytes of the compiled fonts `hints.remove` reads: TrueType, CFF, collections, WOFF
/// and WOFF2.
const SIGNATURES: [&[u8; 4]; 7] = [&[0, 1, 0, 0], b"OTTO", b"true", b"typ1", b"ttcf", b"wOFF", b"wOF2"];

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "dehint", summary: "Remove hints and instructions", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli dehint INPUT... [--glyphs LIST] [--keep-font-hints] [--hinting-tables TAG,...] [-o OUT]
      Remove CFF stems, hint masks and flex, or TrueType instructions, from compiled fonts.
      Without a glyph selection the font-wide hinting data goes too: the CFF zones and stems,
      or the TrueType font programs, cvt, hdmx, VDMX and LTSH, the maxp hinting limits and the
      GPOS device tables. From a source, remove the PostScript hints of every layer and the
      TrueType instructions of the main master; its zones, stems and font-wide TrueType data
      stay. Each output is INPUT.dehinted.EXT unless -o, --output-dir or --in-place says
      otherwise.
";

/// The options, as data (M12.27).
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("glyphs", "SELECTOR", "", "remove hints only from these glyphs (font-wide data is then kept)"),
    OptSpec::list("exclude-glyphs", "SELECTOR", "", "leave these glyphs' hints alone (font-wide data is then kept)"),
    OptSpec::switch("keep-font-hints", false, "keep CFF zones and stems, or the TrueType fpgm, prep, cvt and cvar (compiled fonts)"),
    OptSpec::list("hinting-tables", "TAG", DEFAULT_HINTING_TABLES, "font-wide tables to delete (compiled fonts)"),
    OptSpec::switch("keep-flex", false, "keep flex in CFF outlines (compiled fonts)"),
    OptSpec::value(
        "gasp",
        "keep|smooth|remove",
        "keep",
        "leave gasp alone, replace it with one smoothing range for all sizes, or delete it (compiled fonts)",
    ),
    OptSpec::value("output", "OUT", "", "the output file (one input only)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder outputs go into"),
    OptSpec::switch("in-place", false, "replace each input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection"),
    OptSpec::switch("json", false, "print one JSON result per input"),
];

/// The tables `--hinting-tables` names when not given.
const DEFAULT_HINTING_TABLES: &str = "cvt,cvar,fpgm,prep,hdmx,VDMX";

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    if parsed.value("output").is_some() && parsed.positional.len() > 1 {
        return Err(usage("-o names one output: give one input, or use --output-dir"));
    }
    let mut params = Map::new();
    for (key, name) in [("glyphs", "glyphs"), ("excludeGlyphs", "exclude-glyphs")] {
        if !parsed.list(name).is_empty() {
            params.insert(key.into(), json!(parsed.list(name)));
        }
    }
    params.insert("keepFontHints".into(), json!(parsed.switch("keep-font-hints")));
    params.insert("hintingTables".into(), json!(parsed.list("hinting-tables")));
    params.insert("keepFlex".into(), json!(parsed.switch("keep-flex")));
    if let Some(gasp) = parsed.value("gasp") {
        params.insert("gasp".into(), json!(gasp));
    }
    for (key, name) in [("output", "output"), ("outputDir", "output-dir")] {
        if let Some(v) = parsed.value(name) {
            params.insert(key.into(), json!(v));
        }
    }
    params.insert("inPlace".into(), json!(parsed.switch("in-place")));
    params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    if let Some(face) = parsed.value("face") {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
    let jobs = parsed.jobs()?;
    let as_json = parsed.switch("json");
    // Read for sources only: `hints.remove` reads the selectors of compiled fonts itself.
    let selection = Selection::from_options(&parsed);

    let results = batch(&parsed.positional, jobs, |input| {
        if !is_compiled_font(Path::new(input))
            && let Some(result) = dehint_source(&parsed, input, selection.as_ref().map_err(|e| usage(e.message()))?.as_ref())?
        {
            return Ok((input.to_string(), result));
        }
        let mut p = params.clone();
        p.insert("input".into(), json!(input));
        let result = execute(&mut Session::new(), "hints.remove", &Value::Object(p)).map_err(|f| match f {
            Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
            other => other,
        })?;
        Ok((input.to_string(), result))
    })?;
    for r in &results {
        match r {
            Ok((input, result)) if as_json => out_json_line(&json!({"input": input, "result": result}))?,
            Ok((input, result)) => {
                let locked = result.get("locked").and_then(Value::as_array).map_or(0, Vec::len);
                if locked > 0 {
                    note(&format!("{input}: {locked} locked glyphs are left as they are"));
                }
                let removed: Vec<&str> =
                    result.get("tablesRemoved").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                out(&format!(
                    "{input} -> {}: {} glyphs changed; tables removed: {}",
                    result.get("output").and_then(Value::as_str).unwrap_or("?"),
                    result.get("glyphsChanged").and_then(Value::as_u64).unwrap_or(0),
                    if removed.is_empty() { "none".to_string() } else { removed.join(" ") }
                ))?;
            }
            Err(e) => eprintln!("fontcraft-cli: {}", e.message()),
        }
    }
    batch_outcome(&results)
}

/// A compiled font `hints.remove` reads, by its first bytes. Folders and source files are not.
fn is_compiled_font(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 4];
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    if file.read_exact(&mut head).is_err() {
        return false;
    }
    SIGNATURES.contains(&&head)
}

/// Clears the stored hints of the source `input` and saves it; `None` when it opens as a
/// compiled font after all (a format the signatures miss), which `hints.remove` then takes.
fn dehint_source(parsed: &Parsed, input: &str, selection: Option<&Selection>) -> Result<Option<Value>, Failure> {
    let mut session = Session::new();
    let opened = autohint::open(&mut session, input, parsed.value("face"))?;
    if opened.get("viewOnly").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let font_wide = parsed.switch("keep-font-hints")
        || parsed.switch("keep-flex")
        || parsed.value("gasp").is_some_and(|g| g != "keep")
        || parsed.list("hinting-tables").join(",") != DEFAULT_HINTING_TABLES;
    if font_wide {
        warn(&format!(
            "{input}: --keep-font-hints, --hinting-tables, --keep-flex and --gasp apply to compiled fonts; a source keeps its font-wide data"
        ));
    }
    let target = autohint::target(parsed, input, parsed.value("output"), OUTPUT_SUFFIX)?;
    let output = match &target {
        Target::InPlace => input.to_string(),
        Target::File(p) => p.to_string_lossy().into_owned(),
    };
    if parsed.switch("no-clobber") && matches!(&target, Target::File(p) if p.exists()) {
        return Err(Failure::Failed(format!("{output} exists and --no-clobber is set")));
    }
    let (order, locked): (Vec<String>, Vec<String>) = {
        let font = session.font().map_err(|e| Failure::Failed(format!("{input}: {e}")))?;
        (font.glyphs.iter().map(|g| g.name.clone()).collect(), font.glyphs.iter().filter(|g| g.info.locked).map(|g| g.name.clone()).collect())
    };
    let chosen = selection.map(|s| s.names_in(&order, input));
    // Without a selection, the PostScript command skips locked glyphs itself; a named locked
    // glyph is refused, as in the editor.
    let ps = match &chosen {
        Some(names) => execute(&mut session, "hint.ps.clearGlyphHints", &json!({"glyphs": names}))?,
        None => execute(&mut session, "hint.ps.clearGlyphHints", &json!({}))?,
    };
    let tt_glyphs: Vec<&String> = match &chosen {
        Some(names) => names.iter().collect(),
        None => order.iter().filter(|n| !locked.contains(n)).collect(),
    };
    let tt = execute(&mut session, "hint.tt.removeHints", &json!({"glyphs": tt_glyphs}))?;
    autohint::save(&mut session, &target)?;

    let mut changed: Vec<&str> = ps
        .get("cleared")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(tt.get("glyphs").and_then(Value::as_array).into_iter().flatten())
        .filter_map(|e| e.get("glyph").and_then(Value::as_str).or_else(|| e.as_str()))
        .collect();
    changed.sort_unstable();
    changed.dedup();
    let layers = ps.get("cleared").and_then(Value::as_array).map_or(0, Vec::len);
    let instructions = tt.get("removed").and_then(Value::as_u64).unwrap_or(0);
    // Locked glyphs are only left out when no glyph was named.
    let locked = if chosen.is_some() { Vec::new() } else { locked };
    Ok(Some(json!({
        "output": output,
        "glyphsChanged": changed.len(),
        "layersCleared": layers,
        "instructionsRemoved": instructions,
        "tablesRemoved": [],
        "locked": locked,
    })))
}
