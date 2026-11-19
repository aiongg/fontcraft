//! The options of `fontcraft-cli compile`, as data. Each row names its spellings, the path of the
//! `font.compile` parameter it sets (`outlines.removeOverlaps`; `cli.*` for what the subcommand
//! reads itself) and what it does with its value ([`Kind`]). [`parse`] reads a command line
//! against the rows, left to right, a later option overriding an earlier one; a new option is a
//! new row.
//!
//! Long options take their value as `--opt VALUE` or `--opt=VALUE`; the single-dash spellings of
//! the reference's project tool (`-ff`, `-osbOn`) take it as the next argument.

use serde_json::{Map, Value, json};

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
    row(&["--flatten-components"], "outlines.flattenComponents", Kind::Flag(true), ""),
    row(&["--no-flatten-components"], "outlines.flattenComponents", Kind::Flag(false), ""),
    row(&["--decompose-components"], "outlines.decomposeComponents", Kind::Flag(true), ""),
    row(&["--decompose-transformed-components"], "outlines.decomposeTransformedComponents", Kind::Flag(true), ""),
    row(&["--prefer-simple-glyphs"], "outlines.preferSimpleGlyphs", Kind::Flag(true), ""),
    row(&["--no-prefer-simple-glyphs"], "outlines.preferSimpleGlyphs", Kind::Flag(false), ""),
    row(&["--erase-open-corners"], "outlines.eraseOpenCorners", Kind::Flag(true), ""),
    row(&["--no-erase-open-corners"], "outlines.eraseOpenCorners", Kind::Flag(false), ""),
    row(&["--propagate-anchors"], "outlines.propagateAnchors", Kind::Flag(true), ""),
    row(&["--no-propagate-anchors"], "outlines.propagateAnchors", Kind::Flag(false), ""),
    row(&["--auto-use-my-metrics"], "outlines.autoUseMyMetrics", Kind::Flag(true), ""),
    row(&["--no-auto-use-my-metrics"], "outlines.autoUseMyMetrics", Kind::Flag(false), ""),
    row(&["--filter"], "outlines.filters", Kind::List, "NAME"),
    // CFF.
    row(&["--cff-round-tolerance"], "cff.roundTolerance", Kind::Number, "F"),
    row(&["--optimize-cff"], "cff.optimize", Kind::Int(0, 2), "0|1|2"),
    row(&["--max-subrs"], "cff.maxSubrs", Kind::Int(0, 65_535), "N"),
    row(&["--optimize-widths"], "cff.optimizeWidths", Kind::Flag(true), ""),
    row(&["--no-optimize-widths"], "cff.optimizeWidths", Kind::Flag(false), ""),
    row(&["--cid-keyed"], "cff.cidKeyed", Kind::OptionalFolder, "R-O-S"),
    row(&["--warn-unhinted"], "cff.warnUnhinted", Kind::Flag(true), ""),
    row(&["--no-warn-unhinted"], "cff.warnUnhinted", Kind::Flag(false), ""),
    // Glyph set, names and version.
    row(&["--glyphs"], "glyphs", Kind::List, "NAME,..."),
    row(&["--notdef"], "notdef", Kind::Choice(&["auto", "replace", "allow-missing"]), "auto|replace|allow-missing"),
    row(&["--glyph-data"], "glyphData", Kind::List, "FILE"),
    row(&["--production-names"], "productionNames", Kind::Flag(true), ""),
    row(&["--no-production-names"], "productionNames", Kind::Flag(false), ""),
    row(&["--bump-version"], "bumpVersion", Kind::OptionalFolder, "N|X.YYY"),
    // Features.
    row(&["--skip-features"], "skipFeatures", Kind::Flag(true), ""),
    row(&["--features"], "features.file", Kind::Text, "FILE"),
    row(&["--include-dir", "--fea-include-dir"], "features.includeDir", Kind::Text, "DIR"),
    row(&["--mti-source"], "features.mtiSource", Kind::Text, "FILE"),
    row(&["--feature-writer"], "features.writers", Kind::List, "NAME[(mode=append)],...|none|..."),
    row(&["--generate-gdef"], "features.generateGdef", Kind::Flag(true), ""),
    row(&["--no-generate-gdef", "--no-generate-GDEF"], "features.generateGdef", Kind::Flag(false), ""),
    row(&["--variable-features"], "features.variable", Kind::Flag(true), ""),
    row(&["--no-variable-features"], "features.variable", Kind::Flag(false), ""),
    row(&["--expand-features-to-instances"], "features.expandToInstances", Kind::Flag(true), ""),
    row(&["--debug-feature-file"], "features.debugFile", Kind::Text, "FILE"),
    row(&["--stub-gsub"], "features.stubGsub", Kind::Flag(true), ""),
    row(&["--no-stub-gsub"], "features.stubGsub", Kind::Flag(false), ""),
    row(&["--feature-names-override"], "features.namesOverride", Kind::Flag(true), ""),
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
    row(&["--post-format"], "postFormat", Kind::Int(2, 3), "2|3"),
    row(&["--exclude-table"], "excludeTables", Kind::List, "TAG"),
    row(&["--tables"], "tables", Kind::List, "TAG,..."),
    row(&["--autohint"], "autohint", Kind::OptionalFolder, "OPTIONS"),
    row(&["--no-autohint"], "autohint", Kind::Flag(false), ""),
    row(&["--warnings"], "warnings", Kind::Choice(&["default", "all", "none"]), "default|all|none"),
    // Variable fonts.
    row(&["--optimize-gvar"], "optimizeDeltas", Kind::Flag(true), ""),
    row(&["--no-optimize-gvar"], "optimizeDeltas", Kind::Flag(false), ""),
    row(&["--colr-layer-reuse"], "colrLayerReuse", Kind::Flag(true), ""),
    row(&["--no-colr-layer-reuse"], "colrLayerReuse", Kind::Flag(false), ""),
    row(&["--check-compatibility"], "checkCompatibility", Kind::OptionalFolder, "fix"),
    row(&["--no-check-compatibility"], "checkCompatibility", Kind::Flag(false), ""),
    // Options with no counterpart: font.compile refuses their parameters with the reason.
    row(&["--master-finder"], "masterFinder", Kind::Text, "TEMPLATE"),
    row(&["--sparse-glyphs"], "sparseGlyphs", Kind::Text, "FILE"),
    row(&["--emit-designspace"], "emitDesignspace", Kind::Text, "PATH"),
    row(&["--ufo-structure"], "ufoStructure", Kind::Text, "package|zip|json"),
    row(&["--build-dir"], "buildDir", Kind::Text, "DIR"),
    row(&["--emit-debug"], "emitDebug", Kind::Flag(true), ""),
    row(&["--validate-sources"], "validateSources", Kind::Flag(true), ""),
    row(&["--keep-mvar"], "keepMvar", Kind::Flag(true), ""),
    row(&["--designspace"], "designspace", Kind::Text, "FILE"),
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
    fn feature_options_fill_the_features_group() {
        let (_, p) = parsed(&[
            "--features",
            "f.fea",
            "--fea-include-dir=inc",
            "--feature-writer",
            "kern,...",
            "--feature-writer=mark(mode=append)",
            "--no-generate-GDEF",
            "--debug-feature-file",
            "debug.fea",
            "--stub-gsub",
            "--skip-features",
        ]);
        let f = &p["features"];
        assert_eq!((&f["file"], &f["includeDir"], &f["debugFile"]), (&json!("f.fea"), &json!("inc"), &json!("debug.fea")));
        assert_eq!(f["writers"], json!(["kern", "...", "mark(mode=append)"]));
        assert_eq!((&f["generateGdef"], &f["stubGsub"], &p["skipFeatures"]), (&json!(false), &json!(true), &json!(true)));
        assert!(!is_project(p.as_object().unwrap()));
    }

    #[test]
    fn outline_cff_and_glyph_set_options_fill_their_groups() {
        let (_, p) = parsed(&[
            "--flatten-components",
            "--no-auto-use-my-metrics",
            "--cff-round-tolerance",
            "0.25",
            "--optimize-cff=1",
            "--max-subrs",
            "0",
            "--glyphs",
            "A,Aacute",
            "--notdef=replace",
            "--post-format",
            "3",
            "--exclude-table",
            "DSIG",
            "--exclude-table=kern",
            "--bump-version",
            "--no-optimize-gvar",
            "--warnings",
            "none",
        ]);
        assert_eq!((&p["outlines"]["flattenComponents"], &p["outlines"]["autoUseMyMetrics"]), (&json!(true), &json!(false)));
        assert_eq!((&p["cff"]["roundTolerance"], &p["cff"]["optimize"], &p["cff"]["maxSubrs"]), (&json!(0.25), &json!(1), &json!(0)));
        assert_eq!((&p["glyphs"], &p["notdef"], &p["postFormat"]), (&json!(["A", "Aacute"]), &json!("replace"), &json!(3)));
        assert_eq!((&p["excludeTables"], &p["bumpVersion"]), (&json!(["DSIG", "kern"]), &json!(true)));
        assert_eq!((&p["optimizeDeltas"], &p["warnings"]), (&json!(false), &json!("none")));
        let (sources, p) = parsed(&["--bump-version=1.250", "--check-compatibility", "F.glyphs", "--emit-debug"]);
        assert_eq!(sources, ["F.glyphs"]);
        assert_eq!((&p["bumpVersion"], &p["checkCompatibility"], &p["emitDebug"]), (&json!("1.250"), &json!(true), &json!(true)));
        assert!(!is_project(p.as_object().unwrap()));
    }

    #[test]
    fn a_bad_value_or_an_unknown_option_is_a_usage_error() {
        for args in [
            &["--output-layout", "nested"][..],
            &["--os2-version", "0"],
            &["--plan=yes"],
            &["--frobnicate"],
            &["--format"],
            &["--post-format", "4"],
            &["--notdef", "keep"],
        ] {
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
