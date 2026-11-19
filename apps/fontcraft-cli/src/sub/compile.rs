//! `fontcraft-cli compile`.

use super::{Subcommand, compile_opts};
use crate::io::{Failure, Outcome, batch_outcome, error, execute, open, out, stage, usage, with_input};
use fontcraft_engine::Session;
use serde_json::{Value, json};

/// The user setting that opens UFO and designspace sources without aligning their composites.
const DISABLE_ALIGNMENT_FOR_IMPORTED: &str = "user.disableAutomaticAlignmentForImportedFiles";

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "compile", summary: "Compile sources to static or variable fonts", usage: USAGE, options: &[], actions: &[], run: compile };

const USAGE: &str = "\
  fontcraft-cli compile SOURCE... [--format FMT]... [--variable] [--container plain|woff|woff2]...
                        [-o FILE | --output-dir DIR] [--output-layout flat|by-format]
                        [--name-by filename|postscript-name] [--instances[=NAME,...|=REGEX]]
                        [--masters-as-instances] [--variable-fonts REGEX | --settings NAME,...]
                        [--family-name NAME] [--keep-overlaps] [--keep-direction] [--conversion-error EM]
                        [--no-subroutinize] [--ttf-curves cu2qu|keep-quad]
                        [--drop-implied-oncurves | --keep-implied-oncurves]
                        [--subset | --no-subset] [--dsig] [--pass-through-tables] [--collection]
                        [--timestamp SECS|now] [--ttfautohint PATH] [--cbdt] [--plan]
                        [--project-file FILE] [--project-defaults] [--menu-names FILE] [--goadb FILE]
                        [--fontinfo FILE] [--feature-file FILE] [--license-code CODE]
                        [--fs-selection-on N]... [--fs-selection-off N]... [--os2-version N] [project options]
                        [--cid-cmap FILE] [--mac-cmap FILE] [--mac-script N] [--mac-language N] [--uvs FILE]
                        [--interpolate-binary-layout [DIR]]
  fontcraft-cli compile --project-file FILE [options]
      Compile each SOURCE to fonts, once per format and container (default plain). --format
      (repeatable, comma-separated): ttf and otf build static fonts with TrueType or CFF outlines;
      variable and variable-cff2 build variable fonts with glyf and gvar or with CFF2 outlines;
      otf-cff2, ttf-interpolatable and otf-interpolatable are not built yet. --variable is
      --format variable. Without --format a SOURCE with several masters and an axis builds a
      variable TTF, unless --instances, --masters-as-instances, --collection,
      --interpolate-binary-layout, --cbdt, --subset, --dsig, --pass-through-tables or a project
      option asks for static fonts; any other SOURCE a static TTF.
      Static fonts: every exportable instance (a font without instances builds its first master),
      or with --instances=NAME,... the named ones, or with --instances=REGEX the exportable ones
      whose name, or family and style name, the expression matches in full (a value holding any
      of . * + ? ( ) [ ] { } | ^ $ or a backslash is an expression); plus each master with
      --masters-as-instances.
      Variable fonts: one per variable font setting, or one for the whole designspace when the
      source has none; --variable-fonts REGEX keeps those whose setting name or file name it
      matches in full, --settings NAME,... the named settings. --family-name NAME builds the
      masters and variable fonts with that family name and only the instances of that family.
      Files go into DIR (default: the current folder), named by the instance's fileName
      parameter, else by PostScript name (--name-by postscript-name: always by PostScript name);
      variable fonts by the setting's file name. --output-layout flat (default) puts them in DIR,
      with both TrueType and CFF outlines each in a ttf or otf subfolder; by-format puts them in
      the reference compiler's folders: master_ttf or master_otf for a font built from its first
      master, instance_ttf or instance_otf for instances and masters built as instances,
      variable_ttf or variable_otf for variable fonts. -o names the one file a single SOURCE
      builds. Overlapping contours are merged in static fonts unless --keep-overlaps is given.
      CFF charstrings are subroutinised unless --no-subroutinize is given or the source
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
      With only variable formats the static options (--instances, --masters-as-instances, --cbdt,
      --subset, --dsig, --pass-through-tables) are refused; a variable format does not go with
      --collection, --interpolate-binary-layout or a project compile.
      --plan prints what would be built, without building.
      --interpolate-binary-layout [DIR] gives each instance the GPOS interpolated from the
      compiled masters in DIR (default master_ttf, or master_otf for otf, beside the source),
      named as the source's master UFOs are, and the GDEF and GSUB of the default master; masters
      built with --masters-as-instances keep their own layout. The source is a designspace or a
      .glyphs file or package. A master that cannot be found stops the build, and nothing is written.
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
      Long options also take their value as --opt=VALUE and may be shortened to a prefix of one
      option only. One JSON line per SOURCE.
";

/// `compile`: each source is opened as a document and built by `font.compile` (or described by
/// `font.compilePlan` with `--plan`). The options are read from the table in `compile_opts`.
fn compile(args: &[String]) -> Outcome {
    let (mut sources, mut params) = compile_opts::parse(args)?;
    let cli = match params.remove("cli") {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    let text = |key: &str| cli.get(key).and_then(Value::as_str).map(str::to_string);
    let (output, output_dir, autohinter) = (text("output"), text("outputDir"), text("ttfautohint"));
    let plan = cli.get("plan") == Some(&Value::Bool(true));
    let (input, project_output) = match params.get("projectFile").and_then(Value::as_str) {
        Some(file) => project_paths(file)?,
        None => (None, None),
    };
    if sources.is_empty() {
        match input {
            Some(path) => sources.push(path.to_string_lossy().into_owned()),
            None => return Err(usage("compile needs at least one SOURCE, or a --project-file whose InputFontPath names one")),
        }
    }
    // A project compile: the subroutinize switches count among its ordered options.
    let is_project = compile_opts::is_project(&params);
    if is_project {
        let formats = params.get("formats").and_then(Value::as_array);
        if formats.is_some_and(|f| f.iter().any(|f| f.as_str().is_some_and(|f| f.starts_with("variable")))) {
            return Err(usage("a project compile builds static fonts: leave out --variable"));
        }
        if formats.is_none() {
            params.insert("formats".into(), json!(["otf"]));
        }
        params.remove("subroutinize");
    } else {
        params.remove(compile_opts::PROJECT_WORDS);
    }
    let destination = match (output, output_dir) {
        (Some(_), Some(_)) => return Err(usage("use either -o or --output-dir")),
        (Some(_), None) if sources.len() > 1 => return Err(usage("-o names one file; with several sources use --output-dir")),
        (Some(file), None) => Some(file),
        (None, Some(dir)) => Some(dir),
        // The compile writes where the project file's OutputFontPath says.
        (None, None) if is_project && project_output.is_some() => None,
        (None, None) => Some(".".to_string()),
    };
    if let Some(destination) = destination {
        params.insert("destination".into(), json!(destination));
    }
    let command = if plan { "font.compilePlan" } else { "font.compile" };
    if plan {
        params.remove("timestamp");
    }
    let params = Value::Object(params);
    let build = |source: &str| -> Result<Value, Failure> {
        let mut session = Session::new();
        if let Some(path) = &autohinter {
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
        stage("compile", || execute(&mut session, command, &params))
    };
    if let [source] = sources.as_slice() {
        let result = build(source)?;
        return out(&json!({"source": source, "result": result}).to_string());
    }
    // Several sources: one failing does not stop the others.
    let mut results = Vec::with_capacity(sources.len());
    for source in &sources {
        let result = with_input(source, || build(source));
        match &result {
            Ok(result) => out(&json!({"source": source, "result": result}).to_string())?,
            Err(e) => with_input(source, || error(e.message())),
        }
        results.push(result);
    }
    batch_outcome(&results)
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
