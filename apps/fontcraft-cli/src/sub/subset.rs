//! `fontcraft-cli subset` (M12.17): one `font.subset` run per input, with the option names of
//! the reference subsetter.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, execute, out, out_json_line, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "subset", summary: "Reduce a font to chosen glyphs", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli subset FONT [GLYPH...] [--unicodes LIST] [--text STRING] [--glyphs LIST] [--gids LIST] [options] [-o OUT]
      Keep the chosen glyphs and every glyph they use (components, accent parts, .notdef), and
      remove the rest from every table. The output is FONT.subset.EXT unless -o, --output-dir
      or --in-place says otherwise. Layout, variation and colour tables are dropped for now;
      --json lists every dropped table and why.
";

/// The options, as data.
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("glyphs", "LIST", "", "glyph names to keep; '*' keeps every glyph"),
    OptSpec::value("glyphs-file", "FILE", "", "a file of glyph names to keep"),
    OptSpec::list("gids", "LIST", "", "glyph indices and ranges to keep (10-12,14)"),
    OptSpec::value("gids-file", "FILE", "", "a file of glyph indices and ranges to keep"),
    OptSpec::list("unicodes", "LIST", "", "code points and ranges in hex, with or without U+; '*' keeps every mapped character"),
    OptSpec::value("unicodes-file", "FILE", "", "a file of code points and ranges to keep"),
    OptSpec::value("text", "STRING", "", "keep the characters of this text"),
    OptSpec::value("text-file", "FILE", "", "keep the characters of this UTF-8 file; line breaks are not added"),
    OptSpec::switch("ignore-missing-glyphs", false, "skip requested glyphs the font lacks instead of failing"),
    OptSpec::switch("ignore-missing-unicodes", true, "skip requested characters the font does not map instead of failing"),
    OptSpec::switch("retain-gids", false, "keep glyph indices by leaving dropped glyphs in place, empty"),
    OptSpec::switch("notdef-glyph", true, "keep .notdef"),
    OptSpec::switch("notdef-outline", false, "keep the outline of .notdef"),
    OptSpec::switch("recommended-glyphs", false, "also keep glyphs 1 to 3 of a TrueType font"),
    OptSpec::switch("bidi-closure", true, "also keep the mirrored partner of each requested character"),
    OptSpec::switch("hinting", true, "keep hints and instructions"),
    OptSpec::list("hinting-tables", "TAG", "cvt,cvar,fpgm,prep,hdmx,VDMX", "tables --no-hinting removes"),
    OptSpec::list("drop-tables", "TAG", "", "more tables to remove"),
    OptSpec::switch("passthrough-tables", false, "keep tables the subsetter does not know"),
    OptSpec::switch("legacy-kern", false, "keep the kern table when GPOS exists"),
    OptSpec::list("name-ids", "LIST", "0,1,2,3,4,5,6", "name IDs to keep; '*' keeps all"),
    OptSpec::list("name-languages", "LIST", "0x409", "name language IDs to keep; '*' keeps all"),
    OptSpec::switch("name-legacy", false, "keep name records that are not Unicode"),
    OptSpec::switch("glyph-names", false, "keep the glyph names of a TrueType font"),
    OptSpec::switch("legacy-cmap", false, "keep cmap subtables that are neither Unicode nor symbol"),
    OptSpec::switch("symbol-cmap", false, "keep symbol cmap subtables"),
    OptSpec::switch("prune-unicode-ranges", true, "clear the OS/2 Unicode ranges no kept character falls in"),
    OptSpec::switch("prune-codepage-ranges", true, "clear the OS/2 code pages the kept characters no longer cover"),
    OptSpec::value("output", "OUT", "", "the output file").short('o'),
    OptSpec::value("output-dir", "DIR", "", "the folder the output goes into"),
    OptSpec::switch("in-place", false, "replace the input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection"),
    OptSpec::switch("json", false, "print the result as JSON"),
];

/// Largest list file read for `--glyphs-file`, `--gids-file`, `--unicodes-file` and `--text-file`.
const MAX_LIST_FILE: u64 = 16 << 20;

fn read_file(path: &str) -> Result<String, Failure> {
    let meta = std::fs::metadata(path).map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))?;
    if meta.len() > MAX_LIST_FILE {
        return Err(Failure::Input(format!("{path} is larger than {MAX_LIST_FILE} bytes")));
    }
    std::fs::read_to_string(path).map_err(|e| Failure::Input(format!("cannot read {path}: {e}")))
}

/// The words of a list file: separated by commas or white space, `#` starting a comment.
fn file_words(path: &str) -> Result<Vec<String>, Failure> {
    let text = read_file(path)?;
    Ok(text
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default())
        .flat_map(|l| l.split([',', ' ', '\t']).filter(|w| !w.is_empty()).map(str::to_string).collect::<Vec<_>>())
        .collect())
}

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    let Some((font, glyph_args)) = parsed.positional.split_first() else { return Err(usage(super::help(&SUBCOMMAND))) };

    let mut keep = Map::new();
    let mut glyphs: Vec<String> = glyph_args.to_vec();
    glyphs.extend(parsed.list("glyphs").iter().cloned());
    if let Some(path) = parsed.value("glyphs-file") {
        glyphs.extend(file_words(path)?);
    }
    let mut gids: Vec<String> = parsed.list("gids").to_vec();
    if let Some(path) = parsed.value("gids-file") {
        gids.extend(file_words(path)?);
    }
    let mut unicodes: Vec<String> = parsed.list("unicodes").to_vec();
    if let Some(path) = parsed.value("unicodes-file") {
        unicodes.extend(file_words(path)?);
    }
    let mut text = parsed.value("text").unwrap_or_default().to_string();
    if let Some(path) = parsed.value("text-file") {
        text.extend(read_file(path)?.chars().filter(|c| *c != '\n' && *c != '\r'));
    }
    if glyphs.is_empty() && gids.is_empty() && unicodes.is_empty() && text.is_empty() {
        return Err(usage("give glyphs, --gids, --unicodes or --text to keep ('*' keeps all)"));
    }
    if !glyphs.is_empty() {
        keep.insert("glyphs".into(), if glyphs.iter().any(|g| g == "*") { json!("*") } else { json!(glyphs) });
    }
    if !gids.is_empty() {
        keep.insert("gids".into(), json!(gids));
    }
    if !unicodes.is_empty() {
        keep.insert("unicodes".into(), if unicodes.iter().any(|u| u == "*") { json!("*") } else { json!(unicodes) });
    }
    if !text.is_empty() {
        keep.insert("text".into(), json!(text));
    }

    let mut params = Map::new();
    params.insert("font".into(), json!(font));
    params.insert("keep".into(), Value::Object(keep));
    for (key, name) in [
        ("ignoreMissingGlyphs", "ignore-missing-glyphs"),
        ("ignoreMissingUnicodes", "ignore-missing-unicodes"),
        ("retainGids", "retain-gids"),
        ("notdefGlyph", "notdef-glyph"),
        ("notdefOutline", "notdef-outline"),
        ("recommendedGlyphs", "recommended-glyphs"),
        ("bidiClosure", "bidi-closure"),
        ("hinting", "hinting"),
        ("passthroughTables", "passthrough-tables"),
        ("legacyKern", "legacy-kern"),
        ("nameLegacy", "name-legacy"),
        ("glyphNames", "glyph-names"),
        ("legacyCmap", "legacy-cmap"),
        ("symbolCmap", "symbol-cmap"),
        ("pruneUnicodeRanges", "prune-unicode-ranges"),
        ("pruneCodepageRanges", "prune-codepage-ranges"),
        ("inPlace", "in-place"),
        ("noClobber", "no-clobber"),
    ] {
        params.insert(key.into(), json!(parsed.switch(name)));
    }
    params.insert("hintingTables".into(), json!(parsed.list("hinting-tables")));
    params.insert("dropTables".into(), json!(parsed.list("drop-tables")));
    for (key, name) in [("nameIds", "name-ids"), ("nameLanguages", "name-languages")] {
        let items = parsed.list(name);
        if items.iter().any(|i| i == "*") {
            params.insert(key.into(), json!("*"));
            continue;
        }
        let mut numbers = Vec::with_capacity(items.len());
        for item in items {
            let n = match item.strip_prefix("0x").or_else(|| item.strip_prefix("0X")) {
                Some(hex) => u16::from_str_radix(hex, 16),
                None => item.parse::<u16>(),
            };
            numbers.push(n.map_err(|_| usage(format!("--{name} needs numbers: `{item}` is not one")))?);
        }
        params.insert(key.into(), json!(numbers));
    }
    for (key, name) in [("output", "output"), ("outputDir", "output-dir")] {
        if let Some(v) = parsed.value(name) {
            params.insert(key.into(), json!(v));
        }
    }
    if let Some(face) = parsed.value("face") {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }

    let result = execute(&mut Session::new(), "font.subset", &Value::Object(params)).map_err(|f| match f {
        Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
        other => other,
    })?;
    if parsed.switch("json") {
        return out_json_line(&result);
    }
    let dropped: Vec<&str> = result
        .get("dropped")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|d| d.get("tag").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    out(&format!(
        "{font} -> {}: {} glyphs kept; tables dropped: {}",
        result.get("output").and_then(Value::as_str).unwrap_or("?"),
        result.get("glyphCount").and_then(Value::as_u64).unwrap_or(0),
        if dropped.is_empty() { "none".to_string() } else { dropped.join(" ") }
    ))
}
