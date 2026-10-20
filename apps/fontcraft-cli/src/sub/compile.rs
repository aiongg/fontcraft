//! `fontcraft-cli compile`.

use super::Subcommand;
use crate::io::{Failure, Outcome, execute, open, out, usage};
use fontcraft_engine::Session;
use serde_json::{Value, json};

/// The user setting that opens UFO and designspace sources without aligning their composites.
const DISABLE_ALIGNMENT_FOR_IMPORTED: &str = "user.disableAutomaticAlignmentForImportedFiles";

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "compile", summary: "Compile sources to static or variable fonts", usage: USAGE, options: &[], actions: &[], run: compile };

const USAGE: &str = "\
  fontcraft-cli compile SOURCE... [--format ttf|otf]... [--container plain|woff|woff2]...
                        [-o FILE | --output-dir DIR] [--instances[=NAME,...]] [--masters-as-instances]
                        [--keep-overlaps] [--keep-direction] [--conversion-error EM] [--no-subroutinize]
                        [--ttf-curves cu2qu|keep-quad] [--drop-implied-oncurves | --keep-implied-oncurves]
                        [--subset | --no-subset] [--dsig] [--pass-through-tables] [--collection]
                        [--timestamp SECS|now] [--ttfautohint PATH] [--cbdt] [--plan]
                        [--project-file FILE] [--project-defaults] [--menu-names FILE] [--goadb FILE]
                        [--fontinfo FILE] [--feature-file FILE] [--license-code CODE]
                        [--fs-selection-on N]... [--fs-selection-off N]... [--os2-version N] [project options]
                        [--cid-cmap FILE] [--mac-cmap FILE] [--mac-script N] [--mac-language N] [--uvs FILE]
  fontcraft-cli compile --project-file FILE [options]
  fontcraft-cli compile SOURCE... --variable [--format ttf|otf] [--settings NAME,...] [--container plain|woff|woff2]...
                        [-o FILE | --output-dir DIR] [--timestamp SECS|now]
      Compile each SOURCE to static fonts: every exportable instance (or the named ones; a font
      without instances builds its first master), plus each master with --masters-as-instances,
      once per format (default ttf) and container (default plain). Files are named by PostScript
      name in DIR (default: the current folder); with both formats each goes into a ttf or otf
      subfolder. -o names the one file a single SOURCE builds. Instances between masters are
      skipped until interpolation exists. Overlapping contours are merged unless --keep-overlaps is
      given. CFF charstrings are subroutinised unless --no-subroutinize is given or the source
      sets Disable Subroutines. --ttfautohint PATH names the trusted ttfautohint program: a source whose TTFAutohint
      binary path parameter names that same file is autohinted by it (default: none runs).
      --cbdt also writes CBDT and CBLC bitmap tables from the iColor layers.
      --ttf-curves: cu2qu (default) converts cubic curves to quadratic ones for TrueType;
      keep-quad converts nothing, and a glyph with a cubic curve stops the build.
      On-curve points halfway between two off-curve points are left out of TrueType glyphs;
      --keep-implied-oncurves keeps every on-curve node and leaves out only the points the
      conversion and quadratic splines imply (--drop-implied-oncurves: the default).
      --no-subset ignores the Keep Glyphs and Remove Glyphs parameters (--subset: the default).
      --dsig adds an empty DSIG table. --pass-through-tables copies the legacy, bitmap and Apple
      tables a compiled SOURCE kept when it was opened, while its glyph order is unchanged.
      --collection packs the fonts of each format into one collection that stores identical
      tables once (SOURCE.ttc, or .otc for CFF); -o FILE.ttc or FILE.otc does the same.
      --plan prints what would be built, without building.
      A project compile builds a font as the reference compiler's project workflow does (default
      format otf): composites and overlaps stay as the source has them, coordinates keep two
      decimals, and the metrics come from the glyphs and the full name. --project-file FILE reads a current.fpr project file first; the project
      options then apply left to right, a later one overriding an earlier: --release /
      --no-release (subroutinize, rename and order by the GlyphOrderAndAliasDB, stub DSIG),
      --subroutinize / --no-subroutinize, --apply-goadb / --no-apply-goadb, --subset-to-goadb /
      --no-subset-to-goadb, --stub-dsig / --no-stub-dsig, --bold / --no-bold, --italic /
      --no-italic, --mac-names / --no-mac-names, --stub-cmap4, --legacy-double-mapping,
      --kern-class0-optimization / --no-kern-class0-optimization; --fontinfo FILE fills what is
      still unset. --menu-names FILE is a FontMenuNameDB, --goadb FILE a GlyphOrderAndAliasDB
      (its code points always apply). --project-defaults finds fontinfo and a features or
      features.fea file beside the source and FontMenuNameDB and GlyphOrderAndAliasDB beside it
      or up to 4 folders above. The paths a project file names are relative to it; without
      SOURCE its InputFontPath is built, and without -o or --output-dir the font goes where its
      OutputFontPath says. --feature-file FILE (-ff) replaces the source's feature code.
      --license-code CODE (-lic) is appended to the unique font identifier (name ID 3).
      --fs-selection-on N (-osbOn) and --fs-selection-off N (-osbOff) turn an OS/2 fsSelection
      bit on or off (each may be given more than once; bits 7, 8 and 9 all need a value when one
      is turned on); --os2-version N (-osv) sets the OS/2 version where the table needs no
      higher one (default 3, or 4 with bits 7 to 9).
      A CID-keyed SOURCE builds a CID-keyed CFF. --cid-cmap FILE (-ch) is a Unicode (UTF-32)
      CMap of its character collection, which then gives every code point of the cmap table;
      --mac-cmap FILE (-cm) a one-byte Macintosh CMap, written as a Macintosh cmap subtable with
      the script --mac-script N (-cs; default: from the ordering) and the language
      --mac-language N (-cl; default 0). --uvs FILE (-ci) adds the Unicode Variation Sequences
      the file lists, for any SOURCE, as a cmap format 14 subtable.
      --variable builds variable fonts instead: one per variable font setting (or the settings
      --settings names), or one for the whole designspace when the source has none, with
      TrueType outlines, or CFF2 outlines with --format otf; files are named by the setting's file
      name. `--format variable` is the same as --variable, `--format variable-cff2` the same as
      --variable --format otf.
      One JSON line per SOURCE.
";

/// `compile`: each source is opened as a document and built by `font.compile` (or described by
/// `font.compilePlan` with `--plan`).
fn compile(args: &[String]) -> Outcome {
    let mut sources: Vec<&String> = Vec::new();
    let (mut formats, mut containers): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let (mut output, mut output_dir): (Option<&String>, Option<&String>) = (None, None);
    let mut params = serde_json::Map::new();
    let mut outlines = serde_json::Map::new();
    let mut plan = false;
    let mut variable = false;
    let mut settings: Option<Vec<String>> = None;
    let mut autohinter: Option<&String> = None;
    let mut project_words: Vec<String> = Vec::new();
    let mut project = serde_json::Map::new();
    let mut project_file: Option<&String> = None;
    let mut it = args.iter();
    let list = |v: &str| -> Vec<String> { v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect() };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--format" => formats.extend(list(it.next().ok_or_else(|| usage("--format needs ttf or otf"))?)),
            "--container" => containers.extend(list(it.next().ok_or_else(|| usage("--container needs plain, woff or woff2"))?)),
            "-o" | "--output" => output = Some(it.next().ok_or_else(|| usage("-o needs a file"))?),
            "--output-dir" => output_dir = Some(it.next().ok_or_else(|| usage("--output-dir needs a folder"))?),
            "--instances" => {
                params.insert("instances".into(), json!(true));
            }
            "--masters-as-instances" => {
                params.insert("mastersAsInstances".into(), json!(true));
            }
            "--keep-overlaps" => {
                outlines.insert("removeOverlaps".into(), json!(false));
            }
            "--remove-overlaps" => {
                outlines.insert("removeOverlaps".into(), json!(true));
            }
            "--subroutinize" | "--no-subroutinize" => {
                params.insert("subroutinize".into(), json!(arg == "--subroutinize"));
                project_words.push(arg.trim_start_matches('-').to_string());
            }
            "--release"
            | "--no-release"
            | "--apply-goadb"
            | "--no-apply-goadb"
            | "--subset-to-goadb"
            | "--no-subset-to-goadb"
            | "--stub-dsig"
            | "--no-stub-dsig"
            | "--bold"
            | "--no-bold"
            | "--italic"
            | "--no-italic"
            | "--mac-names"
            | "--no-mac-names"
            | "--stub-cmap4"
            | "--legacy-double-mapping"
            | "--kern-class0-optimization"
            | "--no-kern-class0-optimization"
            | "--synthetic-glyphs" => {
                project_words.push(arg.trim_start_matches('-').to_string());
                project.insert("projectOptions".into(), Value::Null);
            }
            "--project-file" | "--menu-names" | "--goadb" | "--fontinfo" | "--feature-file" | "-ff" => {
                let key = match arg.as_str() {
                    "--project-file" => "projectFile",
                    "--menu-names" => "menuNames",
                    "--goadb" => "goadb",
                    "--fontinfo" => "fontinfo",
                    _ => "featureFile",
                };
                let file = it.next().ok_or_else(|| usage(format!("{arg} needs a file")))?;
                if key == "projectFile" {
                    project_file = Some(file);
                }
                project.insert(key.into(), json!(file));
            }
            "--license-code" | "-lic" => {
                let code = it.next().ok_or_else(|| usage(format!("{arg} needs a licence code")))?;
                project.insert("licenseCode".into(), json!(code));
            }
            "--fs-selection-on" | "-osbOn" | "--fs-selection-off" | "-osbOff" => {
                let key = if matches!(arg.as_str(), "--fs-selection-on" | "-osbOn") { "os2BitsOn" } else { "os2BitsOff" };
                let v = it.next().ok_or_else(|| usage(format!("{arg} needs a bit number")))?;
                let bit = v.parse::<i64>().map_err(|_| usage(format!("{arg} must be a bit number from 0 to 15, got `{v}`")))?;
                if let Some(Value::Array(bits)) = project.get_mut(key) {
                    bits.push(json!(bit));
                } else {
                    project.insert(key.into(), json!([bit]));
                }
            }
            "--os2-version" | "-osv" => {
                let v = it.next().ok_or_else(|| usage(format!("{arg} needs a number")))?;
                let n = v
                    .parse::<u16>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| usage(format!("{arg} must be a whole number from 1 to 65535, got `{v}`")))?;
                project.insert("os2Version".into(), json!(n));
            }
            "--uvs" | "-ci" => {
                let file = it.next().ok_or_else(|| usage(format!("{arg} needs a Unicode Variation Sequences file")))?;
                params.insert("uvsFile".into(), json!(file));
            }
            "--cid-cmap" | "-ch" | "--mac-cmap" | "-cm" => {
                let key = if matches!(arg.as_str(), "--cid-cmap" | "-ch") { "cidCmap" } else { "cidMacCmap" };
                let file = it.next().ok_or_else(|| usage(format!("{arg} needs a CMap file")))?;
                params.insert(key.into(), json!(file));
            }
            "--mac-script" | "-cs" | "--mac-language" | "-cl" => {
                let key = if matches!(arg.as_str(), "--mac-script" | "-cs") { "cidMacScript" } else { "cidMacLanguage" };
                let v = it.next().ok_or_else(|| usage(format!("{arg} needs a number")))?;
                let n = v
                    .parse::<u16>()
                    .ok()
                    .filter(|n| *n <= 32_767)
                    .ok_or_else(|| usage(format!("{arg} must be a whole number from 0 to 32767, got `{v}`")))?;
                params.insert(key.into(), json!(n));
            }
            "--project-defaults" => {
                project.insert("projectDefaults".into(), json!(true));
            }
            "--keep-direction" => {
                outlines.insert("keepDirection".into(), json!(true));
            }
            "--ttf-curves" => {
                let v = it.next().ok_or_else(|| usage("--ttf-curves needs cu2qu or keep-quad"))?;
                outlines.insert("curves".into(), json!(v));
            }
            "--drop-implied-oncurves" | "--keep-implied-oncurves" => {
                outlines.insert("dropImpliedOnCurves".into(), json!(arg == "--drop-implied-oncurves"));
            }
            "--subset" | "--no-subset" => {
                params.insert("subset".into(), json!(arg == "--subset"));
            }
            "--dsig" => {
                params.insert("stubDsig".into(), json!(true));
            }
            "--pass-through-tables" => {
                params.insert("passThroughTables".into(), json!(true));
            }
            "--collection" => {
                params.insert("collection".into(), json!(true));
            }
            "--conversion-error" => {
                let v = it.next().ok_or_else(|| usage("--conversion-error needs a number"))?;
                let e = v
                    .parse::<f64>()
                    .ok()
                    .filter(|e| e.is_finite())
                    .ok_or_else(|| usage(format!("--conversion-error must be a number, got `{v}`")))?;
                outlines.insert("conversionError".into(), json!(e));
            }
            "--timestamp" => {
                let v = it.next().ok_or_else(|| usage("--timestamp needs seconds since 1970 or `now`"))?;
                let t = if v == "now" {
                    json!("now")
                } else {
                    json!(v.parse::<i64>().map_err(|_| usage(format!("--timestamp must be whole seconds or `now`, got `{v}`")))?)
                };
                params.insert("timestamp".into(), t);
            }
            "--ttfautohint" => autohinter = Some(it.next().ok_or_else(|| usage("--ttfautohint needs the path of a ttfautohint program"))?),
            "--cbdt" => {
                params.insert("cbdt".into(), json!(true));
            }
            "--plan" => plan = true,
            "--variable" => variable = true,
            "--settings" => settings = Some(list(it.next().ok_or_else(|| usage("--settings needs setting names"))?)),
            other => match other.strip_prefix("--instances=") {
                Some(names) => {
                    params.insert("instances".into(), json!(list(names)));
                }
                None if other.starts_with('-') => return Err(usage(format!("unknown compile option `{other}`"))),
                None => sources.push(arg),
            },
        }
    }
    let (input, project_output) = match project_file {
        Some(file) => project_paths(file)?,
        None => (None, None),
    };
    let input = input.map(|p| p.to_string_lossy().into_owned());
    if sources.is_empty() {
        match &input {
            Some(path) => sources.push(path),
            None => return Err(usage("compile needs at least one SOURCE, or a --project-file whose InputFontPath names one")),
        }
    }
    // A project compile: the subroutinize switches count among its ordered options.
    let is_project = !project.is_empty();
    if is_project {
        if variable || formats.iter().any(|f| f.starts_with("variable")) {
            return Err(usage("a project compile builds static fonts: leave out --variable"));
        }
        params.remove("subroutinize");
        project.insert("projectOptions".into(), json!(project_words));
        if formats.is_empty() {
            formats.push("otf".into());
        }
        params.extend(project);
    }
    let destination = match (output, output_dir) {
        (Some(_), Some(_)) => return Err(usage("use either -o or --output-dir")),
        (Some(_), None) if sources.len() > 1 => return Err(usage("-o names one file; with several sources use --output-dir")),
        (Some(file), None) => Some(file.clone()),
        (None, Some(dir)) => Some(dir.clone()),
        // The compile writes where the project file's OutputFontPath says.
        (None, None) if is_project && project_output.is_some() => None,
        (None, None) => Some(".".to_string()),
    };
    if let Some(destination) = destination {
        params.insert("destination".into(), json!(destination));
    }
    if !formats.is_empty() {
        params.insert("formats".into(), json!(formats));
    }
    if !containers.is_empty() {
        params.insert("containers".into(), json!(containers));
    }
    if !outlines.is_empty() {
        params.insert("outlines".into(), Value::Object(outlines));
    }
    // `--format variable` is the reference's spelling of `--variable`.
    if formats.iter().any(|f| f == "variable") {
        formats.retain(|f| f != "variable");
        variable = true;
    }
    let mut cff2 = false;
    if formats.iter().any(|f| f == "variable-cff2") {
        formats.retain(|f| f != "variable-cff2");
        variable = true;
        cff2 = true;
    }
    if variable {
        match formats.as_slice() {
            [] => {}
            [f] if f == "ttf" => formats.clear(),
            [f] if f == "otf" => {
                formats.clear();
                cff2 = true;
            }
            _ => return Err(usage("--variable takes one --format: ttf (TrueType outlines) or otf (CFF2 outlines)")),
        }
    }
    if settings.is_some() && !variable {
        return Err(usage("--settings goes with --variable"));
    }
    if variable {
        if plan
            || !formats.is_empty()
            || params.contains_key("outlines")
            || params.contains_key("instances")
            || params.contains_key("mastersAsInstances")
            || params.contains_key("subroutinize")
            || params.contains_key("cbdt")
            || ["subset", "stubDsig", "passThroughTables", "collection", "uvsFile"].iter().any(|k| params.contains_key(*k))
        {
            return Err(usage("--variable takes only --format, --settings, --container, -o, --output-dir and --timestamp"));
        }
        params.remove("formats");
        if cff2 {
            params.insert("outlines".into(), json!("cff2"));
        }
        if let Some(names) = settings {
            params.insert("settings".into(), json!(names));
        }
    }
    let command = if variable {
        "file.exportVariable"
    } else if plan {
        "font.compilePlan"
    } else {
        "font.compile"
    };
    if plan {
        params.remove("timestamp");
    }
    let params = Value::Object(params);
    let build = |source: &str| -> Result<Value, Failure> {
        let mut session = Session::new();
        if let Some(path) = autohinter {
            session
                .settings_mut()
                .set(fontcraft_engine::external_autohint::SETTING, json!(path))
                .map_err(|e| usage(format!("--ttfautohint: {e}")))?;
        }
        // The reference compiler builds composites as the file places them, so a project
        // compile opens the source without aligning them.
        if is_project {
            session.settings_mut().set(DISABLE_ALIGNMENT_FOR_IMPORTED, json!(true)).map_err(|e| usage(e.to_string()))?;
        }
        open(&mut session, source)?;
        execute(&mut session, command, &params)
    };
    if let [source] = sources.as_slice() {
        let result = build(source)?;
        return out(&json!({"source": source, "result": result}).to_string());
    }
    // Several sources: one failing does not stop the others.
    let mut failed = 0usize;
    for source in &sources {
        match build(source) {
            Ok(result) => out(&json!({"source": source, "result": result}).to_string())?,
            Err(e) => {
                eprintln!("fontcraft-cli: {source}: {}", e.message());
                failed += 1;
            }
        }
    }
    if failed > 0 { Err(Failure::Failed(format!("{failed} of {} sources did not build", sources.len()))) } else { Ok(()) }
}

/// The `InputFontPath` and `OutputFontPath` a project file names, relative to its folder: one
/// `Key value` per line, `#` lines skipped. The compile reads the rest of the file.
fn project_paths(file: &str) -> Result<(Option<std::path::PathBuf>, Option<std::path::PathBuf>), Failure> {
    let text = std::fs::read_to_string(file).map_err(|e| usage(format!("--project-file: cannot read {file}: {e}")))?;
    let folder = std::path::Path::new(file).parent().unwrap_or_else(|| std::path::Path::new(""));
    let (mut input, mut output) = (None, None);
    for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
        let Some((key, value)) = line.split_once(char::is_whitespace) else { continue };
        let slot = match key {
            "InputFontPath" => &mut input,
            "OutputFontPath" => &mut output,
            _ => continue,
        };
        *slot = Some(folder.join(value.trim()));
    }
    Ok((input, output))
}
