//! The options of `fontcraft-cli compile`, as data. Each row names its spellings, the path of the
//! `font.compile` parameter it sets (`outlines.removeOverlaps`; `cli.*` for what the subcommand
//! reads itself) and what it does with its value ([`Kind`]). [`parse`] reads a command line
//! against the rows, left to right, a later option overriding an earlier one; a new option is a
//! new row.
//!
//! Long options take their value as `--opt VALUE` or `--opt=VALUE`; the single-dash spellings of
//! the reference's project tool (`-ff`, `-osbOn`) take it as the next argument. A long option may
//! be given in another letter case or shortened to a prefix of one spelling only.

use serde_json::{Map, Value, json};

use crate::args;
use crate::io::{Failure, usage};

/// What an option does with its value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Takes no value; sets the parameter to this.
    Flag(bool),
    /// Takes no value; appends the spelling given, without its dashes, to the list parameter
    /// (the project options, applied in order).
    Word,
    /// Takes no value; sets the parameter to this and appends the spelling given to
    /// `projectOptions`, so a project compile reads it in order with the other project options.
    FlagAndWord(bool),
    /// Takes no value; appends this item to the list parameter (`--variable`).
    Item(&'static str),
    /// One value, as text.
    Text,
    /// One of these words.
    Choice(&'static [&'static str]),
    /// One whole number from `.0` to `.1`.
    Int(i64, i64),
    /// One finite number.
    Number,
    /// Comma-separated items, appended to the list parameter; the option may be repeated.
    List,
    /// A whole number from `.0` to `.1`, appended to the list parameter; the option may be
    /// repeated.
    IntItem(i64, i64),
    /// Whole seconds since 1970, or `now`.
    Timestamp,
    /// `--instances[=VALUE]`: bare, every exportable instance; with a value that holds a regular
    /// expression character (`.*+?()[]{}|^$\`), that expression, matched in full; otherwise a
    /// comma-separated list of names.
    Instances,
    /// An optional folder: `--opt=DIR`, or the next argument when it is neither an option nor a
    /// source; bare, `true`.
    OptionalFolder,
}

/// One option of `compile`.
#[derive(Clone, Copy, Debug)]
pub struct CompileOpt {
    /// Every spelling, the long one first (`--feature-file`, `-ff`).
    pub flags: &'static [&'static str],
    /// The parameter set, dot-separated for a group (`outlines.curves`); `cli.*` for what the
    /// subcommand reads itself.
    pub path: &'static str,
    pub kind: Kind,
    /// What the value is, for messages (`FILE`, `N`); empty for an option without value.
    pub value: &'static str,
}

const fn row(flags: &'static [&'static str], path: &'static str, kind: Kind, value: &'static str) -> CompileOpt {
    CompileOpt { flags, path, kind, value }
}

/// The project options list that [`Kind::FlagAndWord`] appends to.
pub const PROJECT_WORDS: &str = "projectOptions";

/// The parameters that make a compile a project compile, besides project words other than the
/// subroutinize switches.
pub const PROJECT_KEYS: &[&str] =
    &["projectFile", "menuNames", "goadb", "fontinfo", "featureFile", "licenseCode", "os2BitsOn", "os2BitsOff", "os2Version", "projectDefaults"];

/// Every option of `compile`.
pub const OPTIONS: &[CompileOpt] = &[
    // What is built.
    row(&["--format"], "formats", Kind::List, "FMT"),
    row(&["--variable"], "formats", Kind::Item("variable"), ""),
    row(&["--container"], "containers", Kind::List, "plain|woff|woff2"),
    row(&["--instances"], "instances", Kind::Instances, "NAME,...|REGEX"),
    row(&["--masters-as-instances"], "mastersAsInstances", Kind::Flag(true), ""),
    row(&["--variable-fonts"], "variableFonts", Kind::Text, "REGEX"),
    row(&["--settings"], "variableFonts", Kind::List, "NAME,..."),
    row(&["--family-name"], "familyName", Kind::Text, "NAME"),
    // Where the files go.
    row(&["-o", "--output"], "cli.output", Kind::Text, "FILE"),
    row(&["--output-dir"], "cli.outputDir", Kind::Text, "DIR"),
    row(&["--output-layout"], "outputLayout", Kind::Choice(&["flat", "by-format"]), "flat|by-format"),
    row(&["--name-by"], "nameBy", Kind::Choice(&["filename", "postscript-name"]), "filename|postscript-name"),
    row(&["--plan"], "cli.plan", Kind::Flag(true), ""),
    row(&["--timestamp"], "timestamp", Kind::Timestamp, "SECS|now"),
    // Outlines.
    row(&["--keep-overlaps"], "outlines.removeOverlaps", Kind::Flag(false), ""),
    row(&["--remove-overlaps"], "outlines.removeOverlaps", Kind::Flag(true), ""),
    row(&["--keep-direction"], "outlines.keepDirection", Kind::Flag(true), ""),
    row(&["--ttf-curves"], "outlines.curves", Kind::Text, "cu2qu|keep-quad"),
    row(&["--drop-implied-oncurves"], "outlines.dropImpliedOnCurves", Kind::Flag(true), ""),
    row(&["--keep-implied-oncurves"], "outlines.dropImpliedOnCurves", Kind::Flag(false), ""),
    row(&["--conversion-error"], "outlines.conversionError", Kind::Number, "EM"),
    // Tables and files of static fonts.
    row(&["--subset"], "subset", Kind::Flag(true), ""),
    row(&["--no-subset"], "subset", Kind::Flag(false), ""),
    row(&["--dsig"], "stubDsig", Kind::Flag(true), ""),
    row(&["--pass-through-tables"], "passThroughTables", Kind::Flag(true), ""),
    row(&["--collection"], "collection", Kind::Flag(true), ""),
    row(&["--cbdt"], "cbdt", Kind::Flag(true), ""),
    row(&["--ttfautohint"], "cli.ttfautohint", Kind::Text, "PATH"),
    row(&["--interpolate-binary-layout"], "interpolateBinaryLayout", Kind::OptionalFolder, "DIR"),
    row(&["--subroutinize"], "subroutinize", Kind::FlagAndWord(true), ""),
    row(&["--no-subroutinize"], "subroutinize", Kind::FlagAndWord(false), ""),
    // The project compile.
    row(
        &[
            "--release",
            "--no-release",
            "--apply-goadb",
            "--no-apply-goadb",
            "--subset-to-goadb",
            "--no-subset-to-goadb",
            "--stub-dsig",
            "--no-stub-dsig",
            "--bold",
            "--no-bold",
            "--italic",
            "--no-italic",
            "--mac-names",
            "--no-mac-names",
            "--stub-cmap4",
            "--legacy-double-mapping",
            "--kern-class0-optimization",
            "--no-kern-class0-optimization",
            "--synthetic-glyphs",
        ],
        PROJECT_WORDS,
        Kind::Word,
        "",
    ),
    row(&["--project-file"], "projectFile", Kind::Text, "FILE"),
    row(&["--project-defaults"], "projectDefaults", Kind::Flag(true), ""),
    row(&["--menu-names"], "menuNames", Kind::Text, "FILE"),
    row(&["--goadb"], "goadb", Kind::Text, "FILE"),
    row(&["--fontinfo"], "fontinfo", Kind::Text, "FILE"),
    row(&["--feature-file", "-ff"], "featureFile", Kind::Text, "FILE"),
    row(&["--license-code", "-lic"], "licenseCode", Kind::Text, "CODE"),
    row(&["--fs-selection-on", "-osbOn"], "os2BitsOn", Kind::IntItem(0, 15), "BIT"),
    row(&["--fs-selection-off", "-osbOff"], "os2BitsOff", Kind::IntItem(0, 15), "BIT"),
    row(&["--os2-version", "-osv"], "os2Version", Kind::Int(1, 65_535), "N"),
    // Encodings.
    row(&["--uvs", "-ci"], "uvsFile", Kind::Text, "FILE"),
    row(&["--cid-cmap", "-ch"], "cidCmap", Kind::Text, "FILE"),
    row(&["--mac-cmap", "-cm"], "cidMacCmap", Kind::Text, "FILE"),
    row(&["--mac-script", "-cs"], "cidMacScript", Kind::Int(0, 32_767), "N"),
    row(&["--mac-language", "-cl"], "cidMacLanguage", Kind::Int(0, 32_767), "N"),
];

/// The option spelled `name` (without an `=VALUE`).
pub fn find(name: &str) -> Option<&'static CompileOpt> {
    OPTIONS.iter().find(|o| o.flags.contains(&name))
}

/// The extensions of the files and folders `compile` reads as a SOURCE.
const SOURCE_EXTENSIONS: &[&str] =
    &["fontcraft", "glyphs", "glyphspackage", "designspace", "ufo", "ufoz", "ttf", "otf", "ttc", "otc", "woff", "woff2", "cff", "pfa", "pfb", "ttx"];

/// Whether the argument after an [`Kind::OptionalFolder`] option is its folder: not an option and
/// not a SOURCE, so `--interpolate-binary-layout F.glyphs` still compiles F.glyphs.
pub fn is_folder_value(arg: &str) -> bool {
    let ext = std::path::Path::new(arg).extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    !arg.starts_with('-') && !ext.is_some_and(|e| SOURCE_EXTENSIONS.contains(&e.as_str()))
}

/// Reads `args` into the sources and the `font.compile` parameters, with the subcommand's own
/// settings under `cli`.
pub fn parse(args: &[String]) -> Result<(Vec<String>, Map<String, Value>), Failure> {
    let long: Vec<&'static str> = OPTIONS.iter().flat_map(|o| o.flags.iter().copied()).filter(|f| f.starts_with("--")).collect();
    let mut sources = Vec::new();
    let mut params = Map::new();
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        if arg == "--" {
            sources.extend(it.by_ref().cloned());
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            sources.push(arg.clone());
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (arg.as_str(), None),
        };
        let name = match name.strip_prefix("--") {
            Some(rest) if !rest.is_empty() => args::pick(&long, name)?.unwrap_or(name),
            _ => name,
        };
        let opt = find(name).ok_or_else(|| usage(format!("unknown compile option `{name}`")))?;
        let value = match opt.kind {
            Kind::Flag(_) | Kind::Word | Kind::FlagAndWord(_) | Kind::Item(_) => {
                if inline.is_some() {
                    return Err(usage(format!("{name} takes no value")));
                }
                None
            }
            Kind::Instances => inline,
            Kind::OptionalFolder => inline.or_else(|| it.next_if(|next| is_folder_value(next)).map(String::as_str)),
            _ => Some(match inline {
                Some(v) => v,
                None => it.next().map(String::as_str).ok_or_else(|| usage(format!("{name} needs {}", opt.value)))?,
            }),
        };
        apply(opt, name, value, &mut params)?;
    }
    Ok((sources, params))
}

/// Whether the parameters ask for a project compile.
pub fn is_project(params: &Map<String, Value>) -> bool {
    let words = params.get(PROJECT_WORDS).and_then(Value::as_array);
    PROJECT_KEYS.iter().any(|k| params.contains_key(*k))
        || words.is_some_and(|w| w.iter().any(|w| !matches!(w.as_str(), Some("subroutinize" | "no-subroutinize"))))
}

/// Whether a `--instances` value is a regular expression rather than a list of names.
fn is_pattern(value: &str) -> bool {
    value.chars().any(|c| matches!(c, '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\'))
}

fn items(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').map(str::trim).filter(|s| !s.is_empty())
}

/// The parameter at the dot-separated `path`, created as `null` when missing.
fn slot<'m>(params: &'m mut Map<String, Value>, path: &str) -> &'m mut Value {
    let mut parts = path.split('.');
    let first = parts.next().unwrap_or(path);
    let mut slot = params.entry(first.to_string()).or_insert(Value::Null);
    for part in parts {
        if !slot.is_object() {
            *slot = Value::Object(Map::new());
        }
        slot = match slot {
            Value::Object(m) => m.entry(part.to_string()).or_insert(Value::Null),
            other => other,
        };
    }
    slot
}

fn set(params: &mut Map<String, Value>, path: &str, value: Value) {
    *slot(params, path) = value;
}

fn push(params: &mut Map<String, Value>, path: &str, value: Value) {
    let list = slot(params, path);
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    if let Value::Array(items) = list {
        items.push(value);
    }
}

fn int(given: &str, v: &str, min: i64, max: i64) -> Result<i64, Failure> {
    v.parse::<i64>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| usage(format!("{given} must be a whole number from {min} to {max}, got `{v}`")))
}

/// Applies `opt`, spelled `given`, with `value` to `params`.
fn apply(opt: &CompileOpt, given: &str, value: Option<&str>, params: &mut Map<String, Value>) -> Result<(), Failure> {
    let word = || json!(given.trim_start_matches('-'));
    let v = value.unwrap_or_default();
    match opt.kind {
        Kind::Flag(on) => set(params, opt.path, json!(on)),
        Kind::Word => push(params, opt.path, word()),
        Kind::FlagAndWord(on) => {
            set(params, opt.path, json!(on));
            push(params, PROJECT_WORDS, word());
        }
        Kind::Item(item) => push(params, opt.path, json!(item)),
        Kind::Text => set(params, opt.path, json!(v)),
        Kind::Choice(words) => {
            if !words.contains(&v) {
                return Err(usage(format!("{given} takes {}, got `{v}`", words.join(" or "))));
            }
            set(params, opt.path, json!(v));
        }
        Kind::Int(min, max) => set(params, opt.path, json!(int(given, v, min, max)?)),
        Kind::Number => {
            let n = v.parse::<f64>().ok().filter(|n| n.is_finite()).ok_or_else(|| usage(format!("{given} must be a number, got `{v}`")))?;
            set(params, opt.path, json!(n));
        }
        Kind::List => {
            for item in items(v) {
                push(params, opt.path, json!(item));
            }
        }
        Kind::IntItem(min, max) => push(params, opt.path, json!(int(given, v, min, max)?)),
        Kind::Timestamp => {
            let t = if v == "now" {
                json!("now")
            } else {
                json!(v.parse::<i64>().map_err(|_| usage(format!("{given} must be whole seconds or `now`, got `{v}`")))?)
            };
            set(params, opt.path, t);
        }
        Kind::Instances => match value {
            None | Some("") => set(params, opt.path, json!(true)),
            Some(v) if is_pattern(v) => set(params, opt.path, json!(v)),
            Some(v) => set(params, opt.path, json!(items(v).collect::<Vec<_>>())),
        },
        Kind::OptionalFolder => match value {
            None | Some("") => set(params, opt.path, json!(true)),
            Some(dir) => set(params, opt.path, json!(dir)),
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> (Vec<String>, Value) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (sources, params) = parse(&args).unwrap();
        (sources, Value::Object(params))
    }

    #[test]
    fn options_set_the_parameters_their_rows_name() {
        let (sources, p) = parsed(&[
            "F.glyphs",
            "--format",
            "ttf,variable",
            "--output-layout=by-format",
            "--keep-overlaps",
            "-osbOn",
            "8",
            "--fs-selection-on=7",
            "--instances=Reg.*",
            "--interpolate-binary-layout",
            "G.designspace",
            "--subroutinize",
        ]);
        assert_eq!(sources, ["F.glyphs", "G.designspace"]);
        assert_eq!(p["formats"], json!(["ttf", "variable"]));
        assert_eq!(p["outputLayout"], "by-format");
        assert_eq!(p["outlines"]["removeOverlaps"], false);
        assert_eq!(p["os2BitsOn"], json!([8, 7]));
        assert_eq!(p["instances"], "Reg.*");
        assert_eq!(p["interpolateBinaryLayout"], true);
        assert_eq!((&p["subroutinize"], &p["projectOptions"]), (&json!(true), &json!(["subroutinize"])));
        assert!(!is_project(p.as_object().unwrap()));
        let (_, p) = parsed(&["--instances=Bold,Light", "--release", "--variable"]);
        assert_eq!(p["instances"], json!(["Bold", "Light"]));
        assert_eq!(p["formats"], json!(["variable"]));
        assert!(is_project(p.as_object().unwrap()));
    }

    #[test]
    fn long_options_take_a_unique_prefix_or_another_letter_case() {
        let (_, p) = parsed(&["F.glyphs", "--output-l=by-format", "--KEEP-OVERLAPS", "--rel", "--no-subr"]);
        assert_eq!(p["outputLayout"], "by-format");
        assert_eq!(p["outlines"]["removeOverlaps"], false);
        assert_eq!(p["projectOptions"], json!(["release", "no-subroutinize"]), "the words recorded are the full spellings");
        let args: Vec<String> = ["--out", "x"].iter().map(|s| s.to_string()).collect();
        let e = parse(&args).unwrap_err();
        assert!(e.message().contains("--output-dir"), "a prefix of several options lists them: {e:?}");
    }

    #[test]
    fn a_bad_value_or_an_unknown_option_is_a_usage_error() {
        for args in [&["--output-layout", "nested"][..], &["--os2-version", "0"], &["--plan=yes"], &["--frobnicate"], &["--format"]] {
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            assert!(matches!(parse(&args), Err(Failure::Usage(_))), "{args:?}");
        }
    }

    #[test]
    fn the_argument_after_an_optional_folder_is_its_folder_unless_it_is_a_source() {
        assert!(is_folder_value("master_ttf"));
        assert!(is_folder_value("build/masters"));
        assert!(!is_folder_value("F.glyphs"));
        assert!(!is_folder_value("Family.DESIGNSPACE"));
        assert!(!is_folder_value("--format"));
    }
}
