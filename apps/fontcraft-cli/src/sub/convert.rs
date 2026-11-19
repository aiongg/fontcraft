//! `fontcraft-cli convert`: sources through `file.saveAs` and `file.exportStatic`, compiled fonts
//! through `font.convert` (M12.30).

use super::Subcommand;
use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, execute, open, out, out_json_line, usage};
use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "convert",
    summary: "Convert a font between formats or outline flavours, or a source to another format",
    usage: USAGE,
    options: OPTIONS,
    actions: &[],
    run: convert,
};

const USAGE: &str = "\
  fontcraft-cli convert IN OUT [--smart-components components|outlines]
      Open IN (a source) and write OUT in the format its extension names (.fontcraft, .glyphs,
      .glyphspackage, .ufo for one master, .designspace with a UFO per master), or compile it to
      one static font (.ttf, .otf with CFF outlines, .woff, .woff2) of its first exportable
      instance. --smart-components: a UFO or designspace keeps smart components as components
      with their values (default) or holds them drawn as outlines.
  fontcraft-cli convert FONT OUT | convert FONT... --to ttf|otf|otf-cff2|cff|cff2|ufo [options]
      Rewrite compiled fonts, TTX text, Type 1 fonts (.pfa, .pfb) and bare CFF tables with other
      outlines: CFF or CFF2 to TrueType (cubic curves become quadratic within one font unit or
      --max-error em, contours reversed), TrueType to CFF or
      CFF2 (components decomposed, instructions dropped), CFF to CFF2 and back with hints kept,
      only the CFF or CFF2 table (cff, cff2), or a UFO of the font as the editor opens it. With
      IN OUT the extension of OUT says what to write; .woff and .woff2 keep the input's outlines.
      Each output is INPUT.EXT unless -o, --output-dir or --in-place says otherwise.
      A variable font written as UFO or with other outlines is snapshotted at its default
      location unless --location says where. --cid-keyed makes a CFF result CID-keyed.
      Before converting: --location snapshots a variable font, --glyphs and --exclude-glyphs
      keep part of the glyph set (names, glyph ids, /CID in a CID-keyed font), --fd-select and
      --exclude-fd keep the glyphs of some Font DICTs, --goadb renames, subsets or orders the
      glyphs, and --remove-overlaps merges overlapping contours.
      CFF and CFF2 results take the reference converter's writing switches: --charstring-optimize,
      --[no-]optimize-widths, --flatten-seac, --cff-embed-minimal, --[no-]dedupe-family-blues,
      --standard-encoding, --[no-]keep-unique-id and --[no-]warn-hints (on by default: glyphs
      drawn without stem hints are reported). --keep-glyph-order and --allow-missing-notdef shape
      a Type 1 input. --decid makes a CID-keyed font name-keyed (glyphs named cidN), with the
      hint values of the one Font DICT its glyphs come from, or of --fd N for all of them.
      --name-by postscript-name names each output after the font's PostScript name.
";

const OPTIONS: &[OptSpec] = &[
    OptSpec::value("to", "FORMAT", "", "ttf, otf, otf-cff2, cff, cff2 or ufo"),
    OptSpec::value("container", "plain|woff|woff2", "plain", "wrap the result"),
    OptSpec::value("max-error", "EM", "", "largest curve conversion error, as a fraction of the em (default: one font unit)"),
    OptSpec::value(
        "curve-conversion",
        "optimized|exact",
        "",
        "quadratic to cubic: merge curves within the error (optimized, the default), or raise each one exactly",
    ),
    OptSpec::switch("keep-direction", false, "do not reverse contours when the curve degree changes"),
    OptSpec::value("post-format", "2|3", "", "keep glyph names in post (2, the default) or drop them (3)"),
    OptSpec::switch("subroutinize", false, "subroutinise CFF and CFF2 results (default: sfnt results yes, bare tables no)"),
    OptSpec::value("max-subrs", "N", "", "most subroutines per INDEX"),
    OptSpec::switch("no-hints", false, "drop stem hints when CFF outlines stay CFF"),
    OptSpec::optional_value(
        "cid-keyed",
        "R-O-S",
        "make a name-keyed CFF result CID-keyed, CID n for glyph n (default Identity ordering, supplement 0); give a collection as --cid-keyed=R-O-S",
    ),
    OptSpec::value(
        "charstring-optimize",
        "specialize|none",
        "",
        "CFF charstrings in the compact operator forms, or with rmoveto, rlineto and rrcurveto only",
    ),
    OptSpec::switch(
        "optimize-widths",
        false,
        "choose defaultWidthX and nominalWidthX again (--no-optimize-widths: leave them 0 in a table built from TrueType)",
    ),
    OptSpec::switch("flatten-seac", false, "draw seac accented glyphs out and drop dotsection"),
    OptSpec::switch("cff-embed-minimal", false, "drop the descriptive Top DICT strings (version, Notice, Copyright, FullName, FamilyName, Weight)"),
    OptSpec::switch("dedupe-family-blues", false, "drop FamilyBlues and FamilyOtherBlues equal to BlueValues and OtherBlues (default on)"),
    OptSpec::switch("standard-encoding", false, "drop a custom Encoding: the font has StandardEncoding"),
    OptSpec::switch("keep-unique-id", false, "keep UniqueID and XUID (default on; --no-keep-unique-id drops them)"),
    OptSpec::switch("warn-hints", false, "report glyphs drawn without stem hints and repeated hint masks (default on)"),
    OptSpec::switch("keep-glyph-order", false, "Type 1 input: keep the file's glyph order (.notdef first)"),
    OptSpec::switch("allow-missing-notdef", false, "Type 1 input without .notdef: its first glyph becomes glyph 0 instead of an added empty .notdef"),
    OptSpec::switch("decid", false, "make a CID-keyed CFF result name-keyed, glyphs named cidN"),
    OptSpec::value("fd", "N", "", "with --decid: every glyph takes the hint values of Font DICT N"),
    OptSpec::value("name-by", "filename|postscript-name", "", "name each output after the input file (default) or the font's PostScript name"),
    OptSpec::value("timestamp", "keep|now|SECONDS", "", "head.modified: keep it, the current time, or seconds since 1970"),
    OptSpec::value("glyphs", "SELECTOR", "", "keep only these glyphs and .notdef: names, glyph ids, ranges (3-9), U+0041"),
    OptSpec::value("exclude-glyphs", "SELECTOR", "", "leave these glyphs out"),
    OptSpec::list("fd-select", "N,...", "", "keep only the glyphs of these Font DICTs (CID-keyed CFF or CFF2, numbered from 0)"),
    OptSpec::list("exclude-fd", "N,...", "", "leave out the glyphs of these Font DICTs"),
    OptSpec::value("goadb", "FILE", "", "rename glyphs from the GOADB's second column to its first"),
    OptSpec::switch("no-rename", false, "use the GOADB without renaming"),
    OptSpec::switch("subset-to-goadb", false, "keep only the glyphs the GOADB lists"),
    OptSpec::switch("goadb-order", false, "order the glyphs as the GOADB lists them"),
    OptSpec::list("location", "AXIS=VALUE", "", "snapshot a variable font at this location first (axis tags; others at their default)"),
    OptSpec::switch("no-clamp", false, "refuse a location outside the axis ranges instead of clamping it"),
    OptSpec::switch("remove-overlaps", false, "merge overlapping contours (a merged CFF glyph loses its stem hints)"),
    OptSpec::switch("ignore-overlap-errors", false, "keep a glyph whose overlaps cannot be merged as it is"),
    OptSpec::value("smart-components", "components|outlines", "", "sources: how a UFO or designspace holds smart components"),
    OptSpec::value("output", "OUT", "", "the output file (one input only)").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder outputs go into"),
    OptSpec::switch("in-place", false, "replace each input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection"),
    OptSpec::switch("json", false, "print one JSON result per input"),
];

fn convert(raw: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, raw)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let Some(first) = parsed.positional.first() else { return Err(usage(super::help(&SUBCOMMAND))) };
    let second = parsed.positional.get(1).map(String::as_str);
    let compiled = parsed.value("to").is_some() || is_compiled(first) || (program_kind(first).is_some() && second.is_some_and(writes_binary));
    if compiled {
        return convert_fonts(&parsed, raw);
    }
    let [input, output] = parsed.positional.as_slice() else { return Err(usage("convert needs IN and OUT")) };
    let given = ["glyphs", "exclude-glyphs", "goadb"].into_iter().find(|name| parsed.value(name).is_some_and(|v| !v.is_empty()));
    let given = given.or_else(|| ["location", "fd-select", "exclude-fd"].into_iter().find(|name| !parsed.list(name).is_empty()));
    let given = given.or_else(|| parsed.value("cid-keyed").is_some().then_some("cid-keyed"));
    let given = given.or_else(|| ["charstring-optimize", "fd", "name-by"].into_iter().find(|name| parsed.value(name).is_some_and(|v| !v.is_empty())));
    let given = given.or_else(|| {
        [
            "no-rename",
            "subset-to-goadb",
            "goadb-order",
            "no-clamp",
            "remove-overlaps",
            "ignore-overlap-errors",
            "optimize-widths",
            "flatten-seac",
            "cff-embed-minimal",
            "dedupe-family-blues",
            "standard-encoding",
            "keep-unique-id",
            "warn-hints",
            "keep-glyph-order",
            "allow-missing-notdef",
            "decid",
        ]
        .into_iter()
        .find(|name| parsed.switch(name))
    });
    if let Some(name) = given {
        return Err(usage(format!("--{name} applies to compiled fonts; a source is converted as it is (see instance and subset)")));
    }
    let smart = parsed.value("smart-components").filter(|v| !v.is_empty());
    if let Some(v) = smart
        && !matches!(v, "components" | "outlines")
    {
        return Err(usage("--smart-components needs components or outlines"));
    }
    let mut session = Session::new();
    open(&mut session, input)?;
    let extension = std::path::Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if smart.is_some() && !matches!(extension.as_str(), "ufo" | "designspace") {
        return Err(usage("--smart-components applies to .ufo and .designspace output"));
    }
    let result = match extension.as_str() {
        "ttf" | "otf" | "woff" | "woff2" => execute(&mut session, "file.exportStatic", &json!({"destination": output}))?,
        "fontcraft" | "glyphs" | "glyphspackage" | "ufo" | "designspace" => {
            let mut params = json!({"path": output});
            if let (Some(v), Value::Object(m)) = (smart, &mut params) {
                m.insert("smartComponents".into(), json!(v));
            }
            execute(&mut session, "file.saveAs", &params)?
        }
        other => {
            return Err(usage(format!(
                "cannot write `.{other}` yet (writable: .ttf, .otf, .woff, .woff2, .fontcraft, .glyphs, .glyphspackage, .ufo, .designspace; see ROADMAP.md)"
            )));
        }
    };
    out(&result.to_string())
}

/// A file whose first bytes say it is a compiled font (sfnt, collection, WOFF, WOFF2).
fn is_compiled(path: &str) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head));
    read.is_ok() && matches!(&head, [0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf" | b"wOFF" | b"wOF2")
}

/// Inputs `font.convert` reads besides compiled fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Program {
    /// A PFA or PFB font, or a bare CFF table.
    PostScript,
    Ttx,
}

/// Whether the file at `path` starts like a Type 1 font, a bare CFF table or TTX text.
fn program_kind(path: &str) -> Option<Program> {
    use std::io::Read as _;
    let mut head = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(4096).read_to_end(&mut head)).ok()?;
    if head.starts_with(&[0x80, 0x01]) || head.starts_with(b"%!PS-AdobeFont") || head.starts_with(b"%!FontType1") {
        return Some(Program::PostScript);
    }
    if matches!(head.get(..4), Some(&[1, 0, h, off]) if h >= 4 && (1..=4).contains(&off)) {
        return Some(Program::PostScript);
    }
    let text = String::from_utf8_lossy(&head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with('<') && text.contains("<ttFont")).then_some(Program::Ttx)
}

/// Whether `output` names a file `font.convert` writes.
fn writes_binary(output: &str) -> bool {
    let extension = std::path::Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(extension.as_str(), "ttf" | "otf" | "cff" | "cff2" | "woff" | "woff2")
}

/// The flavour already in a compiled font, for `IN OUT.woff`: from the sfnt version and, for
/// `OTTO`, whether the table directory lists `CFF2`. A Type 1 font or bare CFF table keeps CFF
/// outlines; TTX text the flavour its `sfntVersion` and tables name.
fn flavour_of(path: &str) -> Result<&'static str, Failure> {
    match program_kind(path) {
        Some(Program::PostScript) => return Ok("otf"),
        Some(Program::Ttx) => {
            let bytes = crate::io::read_input(path)?;
            let text = String::from_utf8_lossy(&bytes);
            return Ok(if !text.contains("sfntVersion=\"OTTO\"") {
                "ttf"
            } else if text.contains("<CFF2>") {
                "otf-cff2"
            } else {
                "otf"
            });
        }
        None => {}
    }
    let bytes = crate::io::read_input(path)?;
    let at = |i: usize| bytes.get(i..i + 4);
    let version = match at(0) {
        Some(b"wOFF" | b"wOF2") => at(4),
        other => other,
    };
    if version != Some(b"OTTO".as_slice()) {
        return Ok("ttf");
    }
    // A plain sfnt: 12-byte header, then 16-byte table records; WOFF keeps its own directory,
    // where CFF2 is rare enough to be read as CFF.
    let tables = bytes.get(4..6).map_or(0, |n| usize::from(u16::from_be_bytes([n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(0)])));
    let cff2 = at(0) == Some(b"OTTO".as_slice()) && (0..tables.min(1024)).any(|i| at(12 + 16 * i) == Some(b"CFF2".as_slice()));
    Ok(if cff2 { "otf-cff2" } else { "otf" })
}

fn convert_fonts(parsed: &args::Parsed, raw: &[String]) -> Outcome {
    let params = font_params(parsed, raw)?;
    let result = execute(&mut Session::new(), "font.convert", &Value::Object(params)).map_err(|f| match f {
        Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
        other => other,
    })?;
    let results = result.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
    for r in &results {
        if parsed.switch("json") {
            out_json_line(r)?;
        } else {
            let input = r.get("input").and_then(Value::as_str).unwrap_or("?");
            for w in r.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                crate::io::warn(&format!("{input}: {w}"));
            }
            let left: Vec<&str> =
                r.get("leftBehind").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            out(&format!(
                "{} -> {}{}",
                r.get("input").and_then(Value::as_str).unwrap_or("?"),
                r.get("output").and_then(Value::as_str).unwrap_or("?"),
                if left.is_empty() { String::new() } else { format!(" (left behind: {})", left.join("; ")) }
            ))?;
        }
    }
    Ok(())
}

/// The `font.convert` parameters of a command line. An option left out is left out of the
/// parameters too, so the command's own defaults apply.
fn font_params(parsed: &args::Parsed, raw: &[String]) -> Result<Map<String, Value>, Failure> {
    let mut params = Map::new();
    let mut inputs: Vec<String> = parsed.positional.clone();
    let mut container = parsed.value("container").unwrap_or("plain").to_string();
    let to = match parsed.value("to") {
        Some(to) => to.to_string(),
        None => {
            // IN OUT: the output's extension decides.
            let [input, output] = parsed.positional.as_slice() else { return Err(usage("convert needs IN and OUT, or --to FORMAT")) };
            if parsed.value("output").is_some() {
                return Err(usage("give OUT or -o, not both"));
            }
            let extension = std::path::Path::new(output).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let to = match extension.as_str() {
                "ttf" => "ttf",
                "otf" => "otf",
                "cff" => "cff",
                "cff2" => "cff2",
                "ufo" => "ufo",
                "woff" | "woff2" => {
                    container = extension.clone();
                    flavour_of(input)?
                }
                other => {
                    return Err(usage(format!(
                        "cannot write `.{other}` from a compiled font (writable: .ttf, .otf, .cff, .cff2, .woff, .woff2, .ufo)"
                    )));
                }
            };
            params.insert("output".into(), json!(output));
            inputs = vec![input.clone()];
            to.to_string()
        }
    };
    params.insert("inputs".into(), json!(inputs));
    params.insert("to".into(), json!(to));
    params.insert("container".into(), json!(container));
    let mut curves = Map::new();
    if let Some(v) = parsed.value("max-error").filter(|v| !v.is_empty()) {
        let max_error = v.parse::<f64>().map_err(|_| usage("--max-error needs a number"))?;
        curves.insert("maxError".into(), json!(max_error));
    }
    if let Some(v) = parsed.value("curve-conversion").filter(|v| !v.is_empty()) {
        curves.insert("conversion".into(), json!(v));
    }
    if parsed.switch("keep-direction") {
        curves.insert("keepDirection".into(), json!(true));
    }
    if !curves.is_empty() {
        params.insert("curves".into(), Value::Object(curves));
    }
    if let Some(v) = parsed.value("post-format").filter(|v| !v.is_empty()) {
        params.insert("postFormat".into(), json!(v.parse::<u64>().map_err(|_| usage("--post-format needs 2 or 3"))?));
    }
    let mut cff = Map::new();
    if raw.iter().any(|a| a == "--no-subroutinize") {
        cff.insert("subroutinize".into(), json!(false));
    } else if parsed.switch("subroutinize") {
        cff.insert("subroutinize".into(), json!(true));
    }
    if let Some(n) = parsed.value("max-subrs").filter(|v| !v.is_empty()) {
        cff.insert("maxSubrs".into(), json!(n.parse::<u64>().map_err(|_| usage("--max-subrs needs a number"))?));
    }
    // A switch the command defaults on or off is sent only when given, either way.
    let given = |name: &str| -> Option<bool> {
        if raw.iter().any(|a| a == &format!("--no-{name}")) { Some(false) } else { parsed.switch(name).then_some(true) }
    };
    if let Some(v) = parsed.value("charstring-optimize").filter(|v| !v.is_empty()) {
        if !matches!(v, "specialize" | "none") {
            return Err(usage("--charstring-optimize needs specialize or none"));
        }
        cff.insert("charstringOptimize".into(), json!(v));
    }
    for (key, name) in [
        ("optimizeWidths", "optimize-widths"),
        ("flattenSeac", "flatten-seac"),
        ("embedMinimal", "cff-embed-minimal"),
        ("dedupeFamilyBlues", "dedupe-family-blues"),
        ("standardEncoding", "standard-encoding"),
        ("keepUniqueId", "keep-unique-id"),
        ("warnHints", "warn-hints"),
        ("keepGlyphOrder", "keep-glyph-order"),
        ("allowMissingNotdef", "allow-missing-notdef"),
    ] {
        if let Some(v) = given(name) {
            cff.insert(key.into(), json!(v));
        }
    }
    if !cff.is_empty() {
        params.insert("cff".into(), Value::Object(cff));
    }
    if let Some(v) = parsed.value("name-by").filter(|v| !v.is_empty()) {
        if !matches!(v, "filename" | "postscript-name") {
            return Err(usage("--name-by needs filename or postscript-name"));
        }
        params.insert("nameBy".into(), json!(v));
    }
    if parsed.switch("no-hints") {
        params.insert("hints".into(), json!(false));
    }
    if let Some(ros) = parsed.value("cid-keyed") {
        let keyed = if ros.is_empty() {
            json!(true)
        } else {
            let well_formed = ros.rsplit_once('-').is_some_and(|(rest, s)| rest.contains('-') && s.parse::<u32>().is_ok());
            if !well_formed {
                return Err(usage(format!(
                    "--cid-keyed takes REGISTRY-ORDERING-SUPPLEMENT, not `{ros}` (write --cid-keyed=R-O-S, or put --cid-keyed after the inputs)"
                )));
            }
            json!(ros)
        };
        params.insert("cid".into(), json!({ "keyed": keyed }));
    }
    let fd = match parsed.value("fd").filter(|v| !v.is_empty()) {
        Some(v) => Some(v.parse::<u64>().map_err(|_| usage(format!("--fd takes a Font DICT number, not {v}")))?),
        None => None,
    };
    if parsed.switch("decid") {
        let cid = params.entry("cid").or_insert_with(|| json!({}));
        if let Value::Object(m) = cid {
            m.insert("decid".into(), json!(true));
            if let Some(fd) = fd {
                m.insert("fd".into(), json!(fd));
            }
        }
    } else if fd.is_some() {
        return Err(usage("--fd needs --decid"));
    }
    for (key, name) in [("fdSelect", "fd-select"), ("excludeFd", "exclude-fd")] {
        let items = parsed.list(name);
        if items.is_empty() {
            continue;
        }
        let mut fds = Vec::new();
        for item in items.iter().flat_map(|l| l.split([' ', ','])).filter(|w| !w.is_empty()) {
            fds.push(item.parse::<u64>().map_err(|_| usage(format!("--{name} takes Font DICT numbers, not {item}")))?);
        }
        params.insert(key.into(), json!(fds));
    }
    if let Some(t) = parsed.value("timestamp").filter(|v| !v.is_empty()) {
        params.insert("timestamp".into(), t.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(t)));
    }
    for (key, name) in [("glyphs", "glyphs"), ("excludeGlyphs", "exclude-glyphs"), ("goadb", "goadb")] {
        if let Some(v) = parsed.value(name).filter(|v| !v.is_empty()) {
            params.insert(key.into(), json!(v));
        }
    }
    if params.contains_key("goadb") {
        params.insert("goadbRename".into(), json!(!parsed.switch("no-rename")));
        params.insert("goadbSubset".into(), json!(parsed.switch("subset-to-goadb")));
        params.insert("goadbOrder".into(), json!(parsed.switch("goadb-order")));
    } else if ["no-rename", "subset-to-goadb", "goadb-order"].iter().any(|name| parsed.switch(name)) {
        return Err(usage("--no-rename, --subset-to-goadb and --goadb-order need --goadb FILE"));
    }
    let location = parsed.list("location");
    if !location.is_empty() {
        let mut axes = Map::new();
        for item in location.iter().flat_map(|l| l.split([' ', ','])).filter(|w| !w.is_empty()) {
            let (axis, value) = item.split_once('=').ok_or_else(|| usage(format!("--location takes AXIS=VALUE, not {item}")))?;
            let value: f64 =
                value.parse().ok().filter(|v: &f64| v.is_finite()).ok_or_else(|| usage(format!("--location: {value} is not a number")))?;
            if axes.insert(axis.to_string(), json!(value)).is_some() {
                return Err(usage(format!("--location names the axis {axis} twice")));
            }
        }
        params.insert("location".into(), Value::Object(axes));
        params.insert("clamp".into(), json!(!parsed.switch("no-clamp")));
    } else if parsed.switch("no-clamp") {
        return Err(usage("--no-clamp needs --location"));
    }
    if parsed.switch("remove-overlaps") {
        params.insert("removeOverlaps".into(), json!(true));
        params.insert("ignoreOverlapErrors".into(), json!(parsed.switch("ignore-overlap-errors")));
    } else if parsed.switch("ignore-overlap-errors") {
        return Err(usage("--ignore-overlap-errors needs --remove-overlaps"));
    }
    for (key, name) in [("output", "output"), ("outputDir", "output-dir")] {
        if let Some(v) = parsed.value(name).filter(|v| !v.is_empty()) {
            params.insert(key.into(), json!(v));
        }
    }
    params.insert("inPlace".into(), json!(parsed.switch("in-place")));
    params.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    if let Some(face) = parsed.value("face").filter(|v| !v.is_empty()) {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(list: &[&str]) -> Result<Map<String, Value>, Failure> {
        let raw: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        font_params(&args::parse(OPTIONS, &raw).unwrap(), &raw)
    }

    #[test]
    fn convert_options_become_engine_parameters() {
        let p = params(&["A.ttf", "--to", "ufo"]).unwrap();
        for key in ["curves", "postFormat", "hints", "cff", "timestamp"] {
            assert!(!p.contains_key(key), "{key}: options left out leave the command's defaults");
        }
        let p = params(&["A.ttf", "--to", "otf", "--no-hints", "--post-format", "3", "--keep-direction", "--location", "wght=300,wdth=80"]).unwrap();
        assert_eq!(p["hints"], json!(false));
        assert_eq!(p["postFormat"], json!(3));
        assert_eq!(p["curves"], json!({ "keepDirection": true }));
        assert_eq!(p["location"], json!({ "wght": 300.0, "wdth": 80.0 }));
        assert!(params(&["A.ttf", "--to", "otf", "--location", "wght=1,wght=2"]).is_err(), "an axis given twice is refused");
        let p = params(&[
            "A.otf",
            "--to",
            "cff",
            "--charstring-optimize",
            "none",
            "--no-warn-hints",
            "--flatten-seac",
            "--decid",
            "--fd",
            "2",
            "--name-by",
            "postscript-name",
        ])
        .unwrap();
        assert_eq!(p["cff"], json!({ "charstringOptimize": "none", "warnHints": false, "flattenSeac": true }));
        assert_eq!(p["cid"], json!({ "decid": true, "fd": 2 }));
        assert_eq!(p["nameBy"], json!("postscript-name"));
        assert!(params(&["A.otf", "--to", "cff", "--fd", "1"]).is_err(), "--fd needs --decid");
    }
}
