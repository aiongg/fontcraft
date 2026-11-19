//! `fontcraft-cli subset` (M12.17–M12.19b, M12.18b, M12.18c, M12.73): one `font.subset` run per input, with the
//! option names of the reference subsetter and the glyph selection of the reference converter.

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec, OptValue, Parsed};
use crate::io::{self, Failure, Outcome, execute, out, out_json_line, usage};

use super::Subcommand;

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "subset", summary: "Reduce a font to chosen glyphs", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli subset FONT... [GLYPH...] [--unicodes LIST] [--text STRING] [--glyphs LIST] [--gids LIST] [options] [-o OUT]
      Keep the chosen glyphs and every glyph they use (components, accent parts, .notdef), and
      remove the rest from every table. Glyphs the kept layout features can substitute in are
      kept too. The output is FONT.subset.EXT unless -o, --output-dir or --in-place says
      otherwise, in the input's container unless --flavor says otherwise. Unused CFF and CFF2
      subroutines are removed; --json lists every dropped table and why.
      A list option replaces its list with --opt=a,b (--opt= empties it), adds to it with
      --opt+=a,b and removes from it with --opt-=a,b; each list starts as its default.
      --opt? prints the value an option has at that point of the line; with no FONT nothing
      else is done.
      --exclude-glyphs, --fd, --exclude-fd, --goadb and --random narrow the glyphs the other
      selections choose, or every glyph when none is given; the closure runs after them.
      Arguments after FONT that end in a font file extension are further inputs, subset with
      the same options, up to --jobs at once; they need --output-dir or no output option.
";

/// The options, as data.
const OPTIONS: &[OptSpec] = &[
    OptSpec::list("glyphs", "LIST", "", "glyph names to keep; '*' keeps every glyph"),
    OptSpec::value("glyphs-file", "FILE", "", "a file of glyph names to keep"),
    OptSpec::list("gids", "LIST", "", "glyph indices and ranges to keep (10-12,14)"),
    OptSpec::value("gids-file", "FILE", "", "a file of glyph indices and ranges to keep"),
    OptSpec::list("unicodes", "LIST", "", "code points and ranges in hex, with or without U+; '*' keeps every mapped character").hex(),
    OptSpec::value("unicodes-file", "FILE", "", "a file of code points and ranges to keep"),
    OptSpec::value("text", "STRING", "", "keep the characters of this text"),
    OptSpec::value("text-file", "FILE", "", "keep the characters of this UTF-8 file; line breaks are not added"),
    OptSpec::list("exclude-glyphs", "LIST", "", "glyphs to leave out: names, indices, ranges (3-9), U+XXXX, /CID; .notdef stays"),
    OptSpec::value("exclude-glyphs-file", "FILE", "", "a file of glyphs to leave out"),
    OptSpec::list("fd", "LIST", "", "keep only the glyphs of these Font DICTs of a CID-keyed or CFF2 font (from 0)"),
    OptSpec::list("exclude-fd", "LIST", "", "leave out the glyphs of these Font DICTs"),
    OptSpec::value("goadb", "FILE", "", "keep only the glyphs a GlyphOrderAndAliasDB names in its first two columns"),
    OptSpec::switch("subset-to-goadb", false, "accepted with --goadb, which always subsets to the file"),
    OptSpec::value("random", "PERCENT", "", "keep a random PERCENT of the glyphs (at least one); the same font gives the same choice").short('p'),
    OptSpec::value("random-new", "PERCENT", "", "the same as --random PERCENT --seed random").short('P'),
    OptSpec::value("seed", "N|random", "", "the seed of --random; random draws a new one, which the summary reports"),
    OptSpec::switch("ignore-missing-glyphs", false, "skip requested glyphs the font lacks instead of failing"),
    OptSpec::switch("ignore-missing-unicodes", true, "skip requested characters the font does not map instead of failing"),
    OptSpec::switch("retain-gids", false, "keep glyph indices by leaving dropped glyphs in place, empty"),
    OptSpec::switch("notdef-glyph", true, "keep .notdef"),
    OptSpec::switch("notdef-outline", false, "keep the outline of .notdef"),
    OptSpec::switch("recommended-glyphs", false, "also keep glyphs 1 to 3 of a TrueType font"),
    OptSpec::switch("layout-closure", true, "also keep the glyphs the kept GSUB features can substitute in"),
    OptSpec::list("layout-features", "TAG", DEFAULT_LAYOUT_FEATURES, "layout features to keep; '*' keeps all, an empty list none"),
    OptSpec::list("layout-scripts", "TAG", "*", "layout scripts to keep, as script or script.LANG; '*' keeps all"),
    OptSpec::switch("bidi-closure", true, "also keep the mirrored partner of each requested character"),
    OptSpec::switch("hinting", true, "keep hints and instructions"),
    OptSpec::list("hinting-tables", "TAG", "cvt,cvar,fpgm,prep,hdmx,VDMX", "tables --no-hinting removes"),
    OptSpec::switch("desubroutinize", false, "expand every CFF and CFF2 subroutine instead of removing only the unused ones"),
    OptSpec::list("drop-tables", "TAG", DEFAULT_DROP_TABLES, "tables removed before subsetting"),
    OptSpec::list(
        "no-subset-tables",
        "TAG",
        DEFAULT_NO_SUBSET_TABLES,
        "tables copied without subsetting; an unknown table listed is kept, a default one left out is dropped",
    ),
    OptSpec::switch("passthrough-tables", false, "keep tables the subsetter does not know"),
    OptSpec::switch("legacy-kern", false, "keep the kern table when GPOS exists"),
    OptSpec::list("name-ids", "LIST", "0,1,2,3,4,5,6", "name IDs to keep; '*' keeps all (also spelled --name-IDs)"),
    OptSpec::list("name-languages", "LIST", "0x409", "name language IDs to keep; '*' keeps all"),
    OptSpec::switch("name-legacy", false, "keep name records that are not Unicode"),
    OptSpec::switch("obfuscate-names", false, "replace the names that install the font with placeholders"),
    OptSpec::switch("glyph-names", false, "keep the glyph names of a TrueType font"),
    OptSpec::switch("legacy-cmap", false, "keep cmap subtables that are neither Unicode nor symbol"),
    OptSpec::switch("symbol-cmap", false, "keep symbol cmap subtables"),
    OptSpec::switch("recalc-bounds", false, "recompute the font bounding box, maxp maxima and hhea/vhea extremes"),
    OptSpec::switch("recalc-timestamp", false, "set the modification time to now (SOURCE_DATE_EPOCH when set)"),
    OptSpec::switch("prune-unicode-ranges", true, "clear the OS/2 Unicode ranges no kept character falls in"),
    OptSpec::switch("prune-codepage-ranges", true, "clear the OS/2 code pages the kept characters no longer cover"),
    OptSpec::switch("recalc-average-width", false, "recompute OS/2 xAvgCharWidth from the kept glyphs"),
    OptSpec::switch("recalc-max-context", false, "recompute OS/2 usMaxContext from the kept layout rules"),
    OptSpec::switch("canonical-order", false, "write the tables in the recommended order instead of the input's"),
    OptSpec::value("flavor", "woff|woff2|none", "", "the container of the output (default: the input's)"),
    OptSpec::switch("with-zopfli", false, "compress WOFF with zopfli (not available: refused with --flavor woff)"),
    OptSpec::switch("harfbuzz-repacker", true, "accepted for compatibility; the layout tables are packed the same either way"),
    OptSpec::switch("lazy", true, "accepted for compatibility; has no effect"),
    OptSpec::value("output", "OUT", "", "the output file").short('o'),
    OptSpec::value("output-file", "OUT", "", "the same as --output"),
    OptSpec::value("output-dir", "DIR", "", "the folder the output goes into"),
    OptSpec::switch("in-place", false, "replace the input"),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing output"),
    OptSpec::value("face", "N|NAME", "", "the face of a collection"),
    OptSpec::value("font-number", "N", "", "the same as --face N"),
    OptSpec::switch("json", false, "print the result as JSON"),
    OptSpec::switch("verbose", false, "also print the dropped tables and the warnings on stderr").short('v'),
    OptSpec::switch("timing", false, "accepted for compatibility; has no effect"),
    OptSpec::switch("pretty-svg", false, "write subset SVG documents again with two-space indentation"),
    OptSpec::switch("xml", false, "print the subset font as TTX instead of the one-line summary"),
    OptSpec::switch("show-defaults", false, "print the value of every option after reading the command line, and exit"),
];

/// The default of `--drop-tables`.
const DEFAULT_DROP_TABLES: &str = "JSTF,DSIG,EBDT,EBLC,EBSC,PCLT,LTSH,Feat,Glat,Gloc,Silf,Sill";

/// The default of `--no-subset-tables`.
const DEFAULT_NO_SUBSET_TABLES: &str = "avar,BASE,fvar,gasp,head,hhea,maxp,vhea,OS/2,loca,name,cvt,fpgm,prep,VDMX,DSIG,CPAL,MVAR,cvar,STAT";

/// The default of `--layout-features`, as the subsetter's default list.
const DEFAULT_LAYOUT_FEATURES: &str = "abvf,abvm,abvs,akhn,blwf,blwm,blws,calt,ccmp,cfar,chws,cjct,clig,cswh,curs,dist,dnom,fin2,fin3,fina,frac,half,haln,halt,init,isol,jalt,kern,liga,ljmo,locl,ltra,ltrm,mark,med2,medi,mkmk,mset,nukt,numr,palt,pref,pres,pstf,psts,rand,rclt,rkrf,rlig,rphf,rtla,rtlm,rvrn,stch,tjmo,valt,vatu,vchw,vert,vhal,vjmo,vkrn,vpal,vrt2";

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

/// One option's value, as the reference subsetter prints it for `--opt?`: Python's text of the
/// value, list items that start with a digit read as numbers.
fn python_value(name: &str, value: Option<&OptValue>) -> String {
    match value {
        None if name == "font-number" => "-1".into(),
        None => "None".into(),
        // The reference's `--no-canonical-order` and its default are both None.
        Some(OptValue::Switch(false)) if name == "canonical-order" => "None".into(),
        Some(OptValue::Switch(on)) => (if *on { "True" } else { "False" }).into(),
        Some(OptValue::Value(v)) => v.clone(),
        Some(OptValue::List(items)) => {
            let items: Vec<String> = items.iter().map(String::as_str).map(python_item).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// A list item as Python prints it inside a list: a number in decimal, other text quoted.
fn python_item(item: &str) -> String {
    if item.starts_with(|c: char| c.is_ascii_digit()) {
        let lower = item.to_ascii_lowercase();
        let number = match lower.get(..2) {
            Some("0x") => lower.get(2..).and_then(|d| u64::from_str_radix(d, 16).ok()),
            Some("0o") => lower.get(2..).and_then(|d| u64::from_str_radix(d, 8).ok()),
            Some("0b") => lower.get(2..).and_then(|d| u64::from_str_radix(d, 2).ok()),
            _ => lower.parse().ok(),
        };
        if let Some(n) = number {
            return n.to_string();
        }
    }
    if item.contains('\'') && !item.contains('"') {
        return format!("\"{}\"", item.replace('\\', "\\\\"));
    }
    format!("'{}'", item.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// The line the reference subsetter prints for `--opt?`.
fn query_line(given: &str, value: &str) -> String {
    format!("Current setting for '{given}' is: {value}")
}

/// `--show-defaults`: every option's value after the whole line, one query line each, or one
/// JSON object.
fn show_defaults(parsed: &Parsed) -> Outcome {
    let shown = OPTIONS.iter().filter(|o| o.name != "show-defaults");
    if parsed.switch("json") {
        let mut map = Map::new();
        for o in shown {
            let v = match parsed.values.get(o.name) {
                None => Value::Null,
                Some(OptValue::Switch(on)) => json!(on),
                Some(OptValue::Value(v)) => json!(v),
                Some(OptValue::List(items)) => json!(items),
            };
            map.insert(o.name.into(), v);
        }
        return out_json_line(&Value::Object(map));
    }
    let lines: Vec<String> = shown.map(|o| query_line(o.name, &python_value(o.name, parsed.values.get(o.name)))).collect();
    out(&lines.join("\n"))
}

fn run(args: &[String]) -> Outcome {
    let parsed = args::parse_with_queries(OPTIONS, args)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    for q in &parsed.queries {
        out(&query_line(&q.given, &python_value(q.name, q.value.as_ref())))?;
    }
    if parsed.switch("show-defaults") {
        return show_defaults(&parsed);
    }
    let Some(font) = parsed.positional.first() else {
        if !parsed.queries.is_empty() {
            return Ok(());
        }
        return Err(usage(super::help(&SUBCOMMAND)));
    };
    if parsed.switch("xml") && parsed.switch("json") {
        return Err(usage("give --xml or --json, not both"));
    }
    let (inputs, _) = split_inputs(&parsed.positional);
    if inputs.len() > 1 {
        if parsed.value("output").is_some() || parsed.value("output-file").is_some() {
            return Err(usage("several inputs need --output-dir or no output file"));
        }
        if parsed.switch("xml") {
            return Err(usage("--xml takes one input"));
        }
        let results = io::batch(inputs, parsed.jobs()?, |input| Ok((input.to_string(), subset_one(&parsed, input)?)))?;
        for r in &results {
            match r {
                Ok((input, result)) => report(&parsed, input, result)?,
                Err(e) => io::error(e.message()),
            }
        }
        return io::batch_outcome(&results);
    }
    let result = subset_one(&parsed, font)?;
    report(&parsed, font, &result)
}

/// One `font.subset` run on `input`.
fn subset_one(parsed: &Parsed, input: &str) -> Result<Value, Failure> {
    let params = params_for(parsed, input)?;
    execute(&mut Session::new(), "font.subset", &params).map_err(|f| match f {
        Failure::Failed(m) if m.contains("cannot open") || m.contains("cannot read") => Failure::Input(m),
        other => other,
    })
}

/// Prints the result of one run.
fn report(parsed: &Parsed, font: &str, result: &Value) -> Outcome {
    if parsed.switch("verbose") {
        io::set_verbosity(2);
        for d in result.get("dropped").and_then(Value::as_array).into_iter().flatten() {
            let field = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("?").to_string();
            io::detail(&format!("dropped '{}': {}", field("tag"), field("reason")));
        }
        for w in result.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            io::detail(w);
        }
    }
    if parsed.switch("json") {
        return out_json_line(result);
    }
    if parsed.switch("xml") {
        let output = result.get("output").and_then(Value::as_str).unwrap_or_default();
        let dump = execute(&mut Session::new(), "font.dumpTables", &json!({ "font": output, "format": "ttx" }))?;
        let text = dump.get("text").and_then(Value::as_str).unwrap_or_default();
        return out(text.strip_suffix('\n').unwrap_or(text));
    }
    let dropped: Vec<&str> = result
        .get("dropped")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|d| d.get("tag").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    let seed = result.get("seed").and_then(Value::as_str).map(|s| format!("; random seed {s}")).unwrap_or_default();
    out(&format!(
        "{font} -> {}: {} glyphs kept; tables dropped: {}{seed}",
        result.get("output").and_then(Value::as_str).unwrap_or("?"),
        result.get("glyphCount").and_then(Value::as_u64).unwrap_or(0),
        if dropped.is_empty() { "none".to_string() } else { dropped.join(" ") }
    ))
}

/// The file extensions that mark an argument after the first as another input.
const FONT_EXTENSIONS: [&str; 6] = ["ttf", "otf", "woff", "woff2", "ttc", "otc"];

/// The inputs and the glyph names among the positional arguments: the first argument, and each
/// one right after it that ends in a font file extension, are inputs.
fn split_inputs(positional: &[String]) -> (&[String], &[String]) {
    let is_font = |a: &str| {
        std::path::Path::new(a).extension().and_then(|e| e.to_str()).is_some_and(|e| FONT_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
    };
    let n = positional.iter().enumerate().take_while(|(i, a)| *i == 0 || is_font(a.as_str())).count();
    positional.split_at_checked(n).unwrap_or((positional, Default::default()))
}

/// The `font.subset` parameters of a parsed command line, for its first input.
#[cfg(test)]
fn params(parsed: &Parsed) -> Result<Value, Failure> {
    let Some(font) = parsed.positional.first() else { return Err(usage(super::help(&SUBCOMMAND))) };
    params_for(parsed, font)
}

/// The `font.subset` parameters of a parsed command line, for the input `font`.
fn params_for(parsed: &Parsed, font: &str) -> Result<Value, Failure> {
    let (_, glyph_args) = split_inputs(&parsed.positional);

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
    let mut exclude: Vec<String> = parsed.list("exclude-glyphs").to_vec();
    if let Some(path) = parsed.value("exclude-glyphs-file") {
        exclude.extend(file_words(path)?);
    }
    let fd_list = |name: &str| -> Result<Vec<u16>, Failure> {
        parsed
            .list(name)
            .iter()
            .map(|i| i.trim().parse::<u16>().map_err(|_| usage(format!("--{name} needs Font DICT indexes: `{i}` is not one"))))
            .collect()
    };
    let (fd, exclude_fd) = (fd_list("fd")?, fd_list("exclude-fd")?);
    let random = match (parsed.value("random"), parsed.value("random-new")) {
        (Some(_), Some(_)) => return Err(usage("give --random (-p) or --random-new (-P), not both")),
        (Some(p), None) => Some((p, parsed.value("seed"))),
        (None, Some(p)) => {
            if parsed.value("seed").is_some() {
                return Err(usage("--random-new (-P) draws its own seed: give --random with --seed instead"));
            }
            Some((p, Some("random")))
        }
        (None, None) => {
            if parsed.value("seed").is_some() {
                return Err(usage("--seed needs --random"));
            }
            None
        }
    };
    if parsed.switch("subset-to-goadb") && parsed.value("goadb").is_none() {
        return Err(usage("--subset-to-goadb needs --goadb FILE"));
    }
    let narrowed = !exclude.is_empty() || !fd.is_empty() || !exclude_fd.is_empty() || random.is_some() || parsed.value("goadb").is_some();
    if glyphs.is_empty() && gids.is_empty() && unicodes.is_empty() && text.is_empty() && !narrowed {
        return Err(usage("give glyphs, --gids, --unicodes or --text to keep ('*' keeps all)"));
    }
    if !exclude.is_empty() {
        keep.insert("excludeGlyphs".into(), json!(exclude));
    }
    if !fd.is_empty() {
        keep.insert("fd".into(), json!(fd));
    }
    if !exclude_fd.is_empty() {
        keep.insert("excludeFd".into(), json!(exclude_fd));
    }
    if let Some((percent, seed)) = random {
        let share = percent.trim().trim_end_matches('%').parse::<f64>().ok().filter(|v| v.is_finite() && (0.0..=100.0).contains(v));
        let share = share.ok_or_else(|| usage(format!("--random needs a percentage from 0 to 100, not `{percent}`")))?;
        let mut random = Map::new();
        random.insert("percent".into(), json!(share));
        match seed {
            None => {}
            Some("random") => {
                random.insert("seed".into(), json!("random"));
            }
            Some(n) => {
                let n = n.trim().parse::<u64>().map_err(|_| usage(format!("--seed needs a whole number or `random`, not `{n}`")))?;
                random.insert("seed".into(), json!(n));
            }
        }
        keep.insert("random".into(), Value::Object(random));
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
    if let Some(goadb) = parsed.value("goadb") {
        params.insert("goadb".into(), json!(goadb));
    }
    for (key, name) in [
        ("ignoreMissingGlyphs", "ignore-missing-glyphs"),
        ("ignoreMissingUnicodes", "ignore-missing-unicodes"),
        ("retainGids", "retain-gids"),
        ("notdefGlyph", "notdef-glyph"),
        ("notdefOutline", "notdef-outline"),
        ("recommendedGlyphs", "recommended-glyphs"),
        ("bidiClosure", "bidi-closure"),
        ("layoutClosure", "layout-closure"),
        ("hinting", "hinting"),
        ("desubroutinize", "desubroutinize"),
        ("passthroughTables", "passthrough-tables"),
        ("legacyKern", "legacy-kern"),
        ("nameLegacy", "name-legacy"),
        ("obfuscateNames", "obfuscate-names"),
        ("glyphNames", "glyph-names"),
        ("legacyCmap", "legacy-cmap"),
        ("symbolCmap", "symbol-cmap"),
        ("recalcBounds", "recalc-bounds"),
        ("pruneUnicodeRanges", "prune-unicode-ranges"),
        ("pruneCodepageRanges", "prune-codepage-ranges"),
        ("recalcAverageWidth", "recalc-average-width"),
        ("recalcMaxContext", "recalc-max-context"),
        ("canonicalOrder", "canonical-order"),
        ("prettySvg", "pretty-svg"),
        ("inPlace", "in-place"),
        ("noClobber", "no-clobber"),
    ] {
        params.insert(key.into(), json!(parsed.switch(name)));
    }
    for (key, name) in [("layoutFeatures", "layout-features"), ("layoutScripts", "layout-scripts")] {
        let items = parsed.list(name);
        params.insert(key.into(), if items.iter().any(|i| i == "*") { json!("*") } else { json!(items) });
    }
    for (key, name) in [("hintingTables", "hinting-tables"), ("dropTables", "drop-tables"), ("noSubsetTables", "no-subset-tables")] {
        params.insert(key.into(), json!(parsed.list(name)));
    }
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
    if parsed.switch("recalc-timestamp") {
        params.insert("timestamp".into(), json!("now"));
    }
    match parsed.value("flavor") {
        None => {}
        Some("" | "none") => {
            params.insert("container".into(), json!("plain"));
        }
        Some(flavor @ ("woff" | "woff2")) => {
            params.insert("container".into(), json!(flavor));
        }
        Some(other) => return Err(usage(format!("--flavor is woff, woff2 or none, not `{other}`"))),
    }
    if parsed.switch("with-zopfli") {
        params.insert("zopfli".into(), json!(true));
    }
    if parsed.value("output").is_some() && parsed.value("output-file").is_some() {
        return Err(usage("give --output or --output-file, not both"));
    }
    for (key, name) in [("output", "output"), ("output", "output-file"), ("outputDir", "output-dir")] {
        if let Some(v) = parsed.value(name) {
            params.insert(key.into(), json!(v));
        }
    }
    let face = match (parsed.value("face"), parsed.value("font-number")) {
        (Some(_), Some(_)) => return Err(usage("give --face or --font-number, not both")),
        (face, number) => face.or(number),
    };
    if let Some(face) = face {
        params.insert("face".into(), face.parse::<u64>().map(Value::from).unwrap_or_else(|_| json!(face)));
    }
    Ok(Value::Object(params))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn params_of(line: &[&str]) -> Value {
        let args: Vec<String> = line.iter().map(|s| s.to_string()).collect();
        params(&args::parse(OPTIONS, &args).unwrap()).unwrap()
    }

    #[test]
    fn list_options_take_the_reference_subsetter_syntax() {
        let p = params_of(&["f.ttf", "A"]);
        assert_eq!(p["dropTables"], json!(DEFAULT_DROP_TABLES.split(',').collect::<Vec<_>>()), "the default list is sent in full");
        assert_eq!(p["noSubsetTables"].as_array().unwrap().len(), 20);
        assert_eq!(p["nameIds"], json!([0, 1, 2, 3, 4, 5, 6]));

        let p = params_of(&["f.ttf", "A", "--drop-tables="]);
        assert_eq!(p["dropTables"], json!([]), "`=` with nothing empties the list: EBDT and the rest are kept");
        let p = params_of(&["f.ttf", "A", "--drop-tables+=BASE", "--drop-tables-=DSIG,LTSH"]);
        assert_eq!(p["dropTables"], json!(["JSTF", "EBDT", "EBLC", "EBSC", "PCLT", "Feat", "Glat", "Gloc", "Silf", "Sill", "BASE"]));
        let p = params_of(&["f.ttf", "A", "--name-IDs+=7,8", "--name-languages-=1033", "--name-languages+=0x412"]);
        assert_eq!(p["nameIds"], json!([0, 1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(p["nameLanguages"], json!([0x412]), "1033 removes 0x409");
        let p = params_of(&["f.ttf", "A", "--layout-features-=kern", "--layout-scripts=latn", "--hinting-tables+=gasp"]);
        assert!(!p["layoutFeatures"].as_array().unwrap().contains(&json!("kern")));
        assert_eq!(p["layoutScripts"], json!(["latn"]));
        assert_eq!(p["hintingTables"], json!(["cvt", "cvar", "fpgm", "prep", "hdmx", "VDMX", "gasp"]));
        let p = params_of(&["f.ttf", "A", "--layout-features=*", "--name-IDs=*", "--no-subset-tables+=LTSH"]);
        assert_eq!((p["layoutFeatures"].clone(), p["nameIds"].clone()), (json!("*"), json!("*")));
        assert_eq!(p["noSubsetTables"].as_array().unwrap().last(), Some(&json!("LTSH")));
    }

    #[test]
    fn reference_option_names_reach_the_command() {
        let p = params_of(&[
            "f.ttf",
            "--text=A",
            "--recalc-bounds",
            "--recalc-timestamp",
            "--recalc-average-width",
            "--recalc-max-context",
            "--canonical-order",
            "--obfuscate-names",
            "--flavor=woff2",
            "--font-number=1",
            "--output-file=o.woff2",
            "--no-harfbuzz-repacker",
            "--timing",
            "--no-lazy",
        ]);
        for key in ["recalcBounds", "recalcAverageWidth", "recalcMaxContext", "canonicalOrder", "obfuscateNames"] {
            assert_eq!(p[key], json!(true), "{key}");
        }
        assert_eq!(p["timestamp"], json!("now"));
        assert_eq!((p["container"].clone(), p["face"].clone(), p["output"].clone()), (json!("woff2"), json!(1), json!("o.woff2")));
        assert_eq!(params_of(&["f.ttf", "A", "--flavor="])["container"], json!("plain"));
        assert!(params_of(&["f.ttf", "A"]).get("container").is_none(), "the input's container by default");
        let bad: Vec<String> = ["f.ttf", "A", "--flavor=zip"].iter().map(|s| s.to_string()).collect();
        assert!(params(&args::parse(OPTIONS, &bad).unwrap()).is_err());
    }

    #[test]
    fn converter_selection_options_reach_the_command() {
        let p = params_of(&["f.ttf", "--exclude-glyphs=A,3-9,/17", "--fd=1", "--exclude-fd=0,2", "--goadb", "GOADB", "--subset-to-goadb"]);
        assert_eq!(p["keep"], json!({"excludeGlyphs": ["A", "3-9", "/17"], "fd": [1], "excludeFd": [0, 2]}), "no other selection is needed");
        assert_eq!(p["goadb"], json!("GOADB"));
        assert_eq!(params_of(&["f.ttf", "-p", "12.5"])["keep"]["random"], json!({"percent": 12.5}));
        assert_eq!(params_of(&["f.ttf", "A", "--random=50", "--seed=7"])["keep"]["random"], json!({"percent": 50.0, "seed": 7}));
        assert_eq!(params_of(&["f.ttf", "-P", "1"])["keep"]["random"], json!({"percent": 1.0, "seed": "random"}));
        for bad in [
            &["f.ttf", "--seed=1"][..],
            &["f.ttf", "-p", "1", "-P", "1"],
            &["f.ttf", "-p", "101"],
            &["f.ttf", "--fd=x"],
            &["f.ttf", "A", "--subset-to-goadb"],
            &["f.ttf"],
        ] {
            let args: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
            assert!(params(&args::parse(OPTIONS, &args).unwrap()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn leading_font_files_are_inputs_and_the_rest_glyphs() {
        let line: Vec<String> = ["a.ttf", "b.OTF", "c.woff2", "A", "d.ttf"].iter().map(|s| s.to_string()).collect();
        let (inputs, glyphs) = split_inputs(&line);
        assert_eq!((inputs, glyphs), (&line[..3], &line[3..]), "a font file after a glyph name is a glyph name");
        assert_eq!(params_of(&["a.ttf", "b.ttf", "A"])["keep"]["glyphs"], json!(["A"]));
    }

    #[test]
    fn query_values_print_as_the_reference_prints_them() {
        let list = |items: &[&str]| OptValue::List(items.iter().map(|s| s.to_string()).collect());
        assert_eq!(python_value("name-languages", Some(&list(&["0x409", "0"]))), "[1033, 0]");
        assert_eq!(python_value("layout-scripts", Some(&list(&["*", "it's", "a\\b"]))), r#"['*', "it's", 'a\\b']"#);
        assert_eq!(python_value("flavor", Some(&OptValue::Value("woff2".into()))), "woff2");
        assert_eq!(python_value("canonical-order", Some(&OptValue::Switch(true))), "True");
        assert_eq!(params_of(&["f.ttf", "A", "--pretty-svg"])["prettySvg"], json!(true));
    }
}
