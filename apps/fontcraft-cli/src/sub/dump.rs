//! `fontcraft-cli dump` (M12.4, M12.10, M12.76): a shell over `font.dumpTables`: TTX, JSON and
//! hex, the text views (`-t TAG=VIEW`, `-t @font`, `-t @glyphs`) and the normalised dump
//! (`--normalize`).

use fontcraft_engine::{EngineError, Session};
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, note, out, out_json, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "dump", summary: "Dump tables as TTX, JSON, hex or text views", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli dump FONT... [-t TAG[=VIEW]]... [-x TAG]... [--format ttx|json|text|hex] [--split tables|glyphs] [-o PATH]
      Write the tables of a compiled font, collection face, WOFF or WOFF2 file to standard output, or
      to -o PATH or --output-dir DIR (one FONT.ttx, .json or .txt file per font). Tables that cannot
      be read are written as hex unless --strict.
      Text views: -t GPOS=fea, -t GSUB=fea, -t GDEF=fea (feature code), -t GPOS=normalized,
      -t GDEF=normalized (flat rules), -t @font (the font program's dictionaries), -t @glyphs (one
      record per glyph; --glyph-detail medium or long adds the path with its hints).
      Table listings: -t cmap=hex-gid|dec-gid|hex-name|dec-name|summary, -t hmtx=advances|advances-lsb,
      -t name=raw|lines|described, -t GSUB=features, -t GPOS=features, -t glyf=points,
      -t VORG=names, -t kern=pairs; -t CFF=literal lists the CFF structures as stored;
      -t avar=designspace writes the axes and their maps as a designspace snippet.
      @glyphs takes --fd and --exclude-fd (font dicts of a CID-keyed font), --flatten-seac (draw
      accented glyphs out) and --goadb FILE with --names source|final (the names shown).
      --unicode-data FILE writes character names as comments after the cmap mappings.
      --normalize writes the normalised dump for comparing two versions of a font.
";

/// The options, as data (M12.4, M12.10).
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("table", "TAG", "", "dump only these tables, in this order; repeatable").short('t').repeat(),
    OptSpec::list("exclude-table", "TAG", "", "dump every table except these; repeatable").short('x').repeat(),
    OptSpec::value("format", "FORMAT", "ttx", "ttx, json, hex or text"),
    OptSpec::value("split", "WHAT", "", "tables: one file per table beside the main file (needs -o or --output-dir)"),
    OptSpec::value("output", "PATH", "", "the file to write; - or none for standard output").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder to write FONT.ttx (or .json, .txt) into"),
    OptSpec::value("face", "N", "0", "the font of a collection"),
    OptSpec::value(
        "bitmap-format",
        "raw|row|bitwise|extfile",
        "raw",
        "how bitmap images are written: hex, hex per pixel row, a character per pixel, or files in bitmaps/ beside the dump (needs -o or --output-dir)",
    )
    .short('z'),
    OptSpec::switch("disassemble", true, "write TrueType programs as assembly (--no-disassemble: as hex)"),
    OptSpec::switch("strict", false, "stop at the first table that cannot be read"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output file"),
    OptSpec::value("newline", "lf|cr|crlf", "lf", "line ending of the output"),
    OptSpec::switch("normalize", false, "write the normalised dump: sorted tables, no values that always differ, outlines and layout as text"),
    OptSpec::switch("version", true, "with --normalize, keep the version fields (--no-version blanks them)"),
    OptSpec::switch("hints", true, "write stem hints in outline dumps (--no-hints leaves them out)"),
    OptSpec::switch("show-extensions", false, "with --normalize, mark lookups stored as extension lookups"),
    OptSpec::value("glyphs", "LIST", "", "the glyphs @glyphs lists, comma-separated; default every glyph"),
    OptSpec::value("exclude-glyphs", "LIST", "", "the glyphs @glyphs leaves out, comma-separated"),
    OptSpec::value("glyph-detail", "short|medium|long", "short", "what @glyphs writes per glyph"),
    OptSpec::value("cmap-subtable", "N", "", "the cmap encoding record the cmap listings write; default all"),
    OptSpec::value("units", "font|1000", "font", "the units of the hmtx listings: the font's own or 1000 per em"),
    OptSpec::value("cff-parts", "LETTERS", "all", "the structures of -t CFF=literal: h n t s g e C f F c p l, or all"),
    OptSpec::value("cff-level", "offsets|bytes|values", "values", "the detail of -t CFF=literal"),
    OptSpec::switch("wrap", true, "break the flowed data of -t CFF=literal into lines (--no-wrap: one line)"),
    OptSpec::switch("allow-virtual-gids", false, "accept glyph indices past the glyph count (always written as glyphNNNNN)"),
    OptSpec::value("unicode-data", "FILE", "", "a UnicodeData.txt file whose names the TTX dump writes after cmap mappings"),
    OptSpec::value("fd", "LIST", "", "the font dicts whose glyphs @glyphs lists, comma-separated indexes from 0"),
    OptSpec::value("exclude-fd", "LIST", "", "the font dicts whose glyphs @glyphs leaves out"),
    OptSpec::switch("flatten-seac", false, "draw accented seac glyphs out in @glyphs"),
    OptSpec::value("names", "source|final", "", "with --goadb, the names @glyphs shows (default source)"),
    OptSpec::value("goadb", "FILE", "", "a glyph name database (final name, source name) for --names"),
];

/// Takes every `-t X`, `--table X` and `--table=X` (or the forms of `long`) out of `args`, so the
/// option can be repeated; the parser keeps only the last value of a list option.
fn take_repeated(args: &[String], short: &str, long: &str) -> (Vec<String>, Vec<String>) {
    let mut values = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    let prefix = format!("{long}=");
    while let Some(a) = it.next() {
        if a == "--" {
            rest.push(a.clone());
            rest.extend(it.by_ref().cloned());
            break;
        }
        if a == short || a == long {
            match it.next() {
                Some(v) => values.push(v.clone()),
                None => rest.push(a.clone()),
            }
        } else if let Some(v) = a.strip_prefix(&prefix) {
            values.push(v.to_string());
        } else {
            rest.push(a.clone());
        }
    }
    (values, rest)
}

/// The `tables` items: `TAG` as a tag, `TAG=VIEW` as `{tag, view}`.
fn tags(given: &[String], parsed: &[String]) -> Vec<Value> {
    given
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .chain(parsed.iter().map(String::as_str))
        .map(|t| match t.split_once('=') {
            Some((tag, view)) => json!({"tag": tag, "view": view}),
            None => json!(t),
        })
        .collect()
}

fn run(args: &[String]) -> Outcome {
    let args = &args::spell_out(OPTIONS, args)?;
    let (tables, rest) = take_repeated(args, "-t", "--table");
    let (excluded, rest) = take_repeated(&rest, "-x", "--exclude-table");
    let parsed = args::parse(OPTIONS, &rest)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    let output = parsed.value("output").filter(|o| !o.is_empty() && *o != "-");
    if output.is_some() && parsed.positional.len() > 1 {
        return Err(usage("-o names one file: give --output-dir to dump several fonts"));
    }
    let face: u64 = parsed.value("face").unwrap_or("0").parse().map_err(|_| usage("--face takes a number"))?;
    let mut base = Map::new();
    base.insert("tables".into(), Value::Array(tags(&tables, parsed.list("table"))));
    base.insert("excludeTables".into(), Value::Array(tags(&excluded, parsed.list("exclude-table"))));
    base.insert("format".into(), json!(parsed.value("format").unwrap_or("ttx")));
    base.insert("face".into(), json!(face));
    base.insert("disassemble".into(), json!(parsed.switch("disassemble")));
    base.insert("bitmapFormat".into(), json!(parsed.value("bitmap-format").unwrap_or("raw")));
    base.insert("strict".into(), json!(parsed.switch("strict")));
    base.insert("noClobber".into(), json!(parsed.switch("no-clobber")));
    base.insert("newline".into(), json!(parsed.value("newline").unwrap_or("lf")));
    base.insert("normalize".into(), json!(parsed.switch("normalize")));
    base.insert("version".into(), json!(parsed.switch("version")));
    base.insert("hints".into(), json!(parsed.switch("hints")));
    base.insert("showExtensions".into(), json!(parsed.switch("show-extensions")));
    base.insert("glyphDetail".into(), json!(parsed.value("glyph-detail").unwrap_or("short")));
    if let Some(n) = parsed.value("cmap-subtable").filter(|n| !n.is_empty()) {
        let n: u64 = n.parse().map_err(|_| usage("--cmap-subtable takes a number"))?;
        base.insert("cmapSubtable".into(), json!(n));
    }
    base.insert("units".into(), json!(parsed.value("units").unwrap_or("font")));
    base.insert("cffParts".into(), json!(parsed.value("cff-parts").unwrap_or("all")));
    base.insert("cffLevel".into(), json!(parsed.value("cff-level").unwrap_or("values")));
    base.insert("wrap".into(), json!(parsed.switch("wrap")));
    if parsed.switch("allow-virtual-gids") {
        base.insert("allowVirtualGids".into(), json!(true));
    }
    if parsed.switch("flatten-seac") {
        base.insert("flattenSeac".into(), json!(true));
    }
    for (option, key) in [("unicode-data", "unicodeData"), ("fd", "fd"), ("exclude-fd", "excludeFd"), ("names", "names"), ("goadb", "goadb")] {
        if let Some(v) = parsed.value(option).filter(|v| !v.is_empty()) {
            base.insert(key.into(), json!(v));
        }
    }
    if let Some(glyphs) = parsed.value("exclude-glyphs").filter(|g| !g.is_empty()) {
        base.insert("excludeGlyphs".into(), json!(glyphs));
    }
    if let Some(glyphs) = parsed.value("glyphs").filter(|g| !g.is_empty()) {
        base.insert("glyphs".into(), json!(glyphs));
    }
    if let Some(split) = parsed.value("split").filter(|s| !s.is_empty()) {
        base.insert("split".into(), json!(split));
    }
    if let Some(o) = output {
        base.insert("output".into(), json!(o));
    }
    if let Some(dir) = parsed.value("output-dir").filter(|d| !d.is_empty()) {
        base.insert("outputDir".into(), json!(dir));
    }
    let mut session = Session::new();
    for font in &parsed.positional {
        let mut params = base.clone();
        params.insert("font".into(), json!(font));
        let result = session.execute("font.dumpTables", &Value::Object(params)).map_err(|e| match e {
            EngineError::Io(m) => Failure::Input(m),
            EngineError::BadParams { message, .. } => usage(message),
            other => Failure::Failed(other.to_string()),
        })?;
        if let Some(text) = result.get("text").and_then(Value::as_str) {
            out(text.strip_suffix('\n').unwrap_or(text))?;
        } else if let Some(tree) = result.get("json") {
            out_json(tree)?;
        } else if let Some(paths) = result.get("paths").and_then(Value::as_array) {
            for p in paths.iter().filter_map(Value::as_str) {
                note(&format!("wrote {p}"));
            }
            if let Some(images) = result.get("images").and_then(Value::as_u64).filter(|n| *n > 0) {
                note(&format!("wrote {images} bitmap image files"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_table_options_are_all_kept() {
        let args: Vec<String> = ["A.ttf", "-t", "head", "--table=name", "-x", "glyf", "--table", "OS/2,cmap"].iter().map(|s| s.to_string()).collect();
        let (tables, rest) = take_repeated(&args, "-t", "--table");
        assert_eq!(tables, ["head", "name", "OS/2,cmap"]);
        assert_eq!(tags(&tables, &[]), vec![json!("head"), json!("name"), json!("OS/2"), json!("cmap")]);
        let (excluded, rest) = take_repeated(&rest, "-x", "--exclude-table");
        assert_eq!(excluded, ["glyf"]);
        assert_eq!(rest, ["A.ttf"]);
        assert_eq!(tags(&["GPOS=fea,@font".to_string()], &[]), vec![json!({"tag": "GPOS", "view": "fea"}), json!("@font")]);
        let e = run(&["missing.ttf".to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), 3, "an unreadable input is exit status 3");
    }
}
