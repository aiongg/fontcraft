//! `fontcraft-cli oracle`.

use super::Subcommand;
use crate::io::{Failure, Outcome, out, usage};
use serde_json::json;

pub const SUBCOMMAND: Subcommand = Subcommand {
    name: "oracle",
    summary: "Compare compiled fonts with a reference compiler (development aid)",
    usage: USAGE,
    options: &[],
    actions: &[],
    run: oracle,
};

/// The option spellings of `oracle`, which reads its own arguments.
pub const SPELLINGS: &[&str] = &[
    "--corpus",
    "--against",
    "--format",
    "--reference-bin",
    "--output-dir",
    "--match",
    "--timestamp",
    "--timeout",
    "--differences",
    "--json",
    "--variable",
    "--instances",
];

const USAGE: &str = "\
  fontcraft-cli oracle (SOURCE... | --corpus [DIR]) [--against fontc|fontmake] [--format ttf|otf]
                      [--reference-bin PATH] [--output-dir DIR] [--timestamp SECS] [--timeout SECS]
                      [--differences N] [--match TEXT] [--variable | --instances] [--json]
      Development aid: compile each source with FontCraft and with a reference compiler (fontc
      for .ttf, fontmake for .otf), read both fonts back and report per table whether they are
      identical, equivalent, allowed or different. --corpus runs over the comparison set in DIR
      (default ./corpus). Exit status 1 when a table differs or a build fails.
      --variable compares the variable fonts of sources with two masters or more (fontc or
      fontmake): outlines, advances, MVAR, GSUB feature variations, GPOS kerning and marks at the
      masters, virtual masters, instances and midpoints between masters; fvar, avar and STAT as
      structures; with --format otf, CFF2 variable fonts against fontmake's -o variable-cff2.
      --instances compares each exported static instance with fontmake's (-i) by the
      static comparison. In both, a font FontCraft does not build yet counts as not built and
      does not set the exit status.
";

fn oracle(args: &[String]) -> Outcome {
    use fontcraft_oracle::{Flavour, Oracle, OursError, Settings};
    use std::path::{Path, PathBuf};

    let mut sources: Vec<String> = Vec::new();
    let mut corpus: Option<PathBuf> = None;
    let (mut against, mut flavour) = (None, None);
    let (mut tool, mut keep, mut filter) = (None, None, None);
    let mut timestamp = fontcraft_oracle::DEFAULT_TIMESTAMP;
    let mut timeout = fontcraft_oracle::DEFAULT_TIMEOUT;
    let mut shown = fontcraft_oracle::DEFAULT_SHOWN;
    let mut as_json = false;
    let (mut variable, mut instances) = (false, false);
    let mut it = args.iter().peekable();
    let number = |opt: &str, v: Option<&String>, max: u64| -> Result<u64, Failure> {
        let v = v.ok_or_else(|| usage(format!("{opt} needs a number")))?;
        v.parse::<u64>().ok().filter(|n| *n <= max).ok_or_else(|| usage(format!("{opt} must be a whole number up to {max}, got `{v}`")))
    };
    while let Some(arg) = it.next() {
        let arg = crate::args::spell_long(SPELLINGS, arg)?;
        match arg.as_str() {
            "--corpus" => corpus = Some(PathBuf::from(it.next_if(|n| !n.starts_with("--")).map(String::as_str).unwrap_or("corpus"))),
            "--against" => {
                let v = it.next().ok_or_else(|| usage("--against needs fontc or fontmake"))?;
                against = Some(Oracle::parse(v).ok_or_else(|| usage(format!("--against must be fontc or fontmake, got `{v}`")))?);
            }
            "--format" => {
                let v = it.next().ok_or_else(|| usage("--format needs ttf or otf"))?;
                flavour = Some(Flavour::parse(v).ok_or_else(|| usage(format!("--format must be ttf or otf, got `{v}`")))?);
            }
            "--reference-bin" => tool = Some(PathBuf::from(it.next().ok_or_else(|| usage("--reference-bin needs a path"))?)),
            "--output-dir" => keep = Some(PathBuf::from(it.next().ok_or_else(|| usage("--output-dir needs a folder"))?)),
            "--match" => filter = Some(it.next().ok_or_else(|| usage("--match needs a text"))?.clone()),
            // Years 1970 to 9999, which the head table and SOURCE_DATE_EPOCH both hold.
            "--timestamp" => timestamp = number("--timestamp", it.next(), 253_402_300_799)? as i64,
            "--timeout" => timeout = std::time::Duration::from_secs(number("--timeout", it.next(), 86_400)?.max(1)),
            "--differences" => shown = number("--differences", it.next(), 10_000)? as usize,
            "--json" => as_json = true,
            "--variable" => variable = true,
            "--instances" => instances = true,
            other if other.starts_with("--") => return Err(usage(format!("unknown oracle option `{other}`"))),
            source => sources.push(source.to_string()),
        }
    }
    if variable && instances {
        return Err(usage("--variable and --instances are separate runs; give one"));
    }
    if instances && flavour == Some(Flavour::Cff) {
        return Err(usage("--instances compares TrueType fonts; leave out --format otf"));
    }
    if variable && flavour == Some(Flavour::Cff) && against == Some(Oracle::Fontc) {
        return Err(usage("fontc builds no CFF2 font; --variable --format otf compares with fontmake"));
    }
    let flavour = flavour.unwrap_or(Flavour::TrueType);
    let against = against.unwrap_or(if flavour == Flavour::Cff || instances { Oracle::Fontmake } else { Oracle::Fontc });
    if instances && against != Oracle::Fontmake {
        return Err(usage("--instances compares with fontmake; fontc builds no static instance between masters"));
    }
    if !against.builds(flavour) {
        return Err(usage(format!("{} cannot build .{} fonts; use --against fontmake", against.name(), flavour.extension())));
    }
    // (path, label) pairs.
    let mut jobs: Vec<(PathBuf, String)> = sources.iter().map(|s| (PathBuf::from(s), s.clone())).collect();
    if let Some(dir) = &corpus {
        let found = fontcraft_oracle::corpus::discover(dir).map_err(Failure::Input)?;
        jobs.extend(found.into_iter().map(|p| {
            let label = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().into_owned();
            (p, label)
        }));
    }
    if let Some(f) = &filter {
        jobs.retain(|(_, label)| label.contains(f.as_str()));
    }
    if jobs.is_empty() {
        return Err(usage("oracle needs SOURCE... or --corpus (and --match must leave something)"));
    }
    for (path, _) in &jobs {
        if !path.exists() {
            return Err(Failure::Input(format!("{}: no such file or folder", path.display())));
        }
    }
    let Some(tool) = fontcraft_oracle::tools::locate(against, tool.as_deref()) else {
        return Err(Failure::Failed(format!(
            "{} not found ({}, put it on PATH, set {} or pass --reference-bin); nothing was compared",
            against.name(),
            against.install_hint(),
            against.env_var()
        )));
    };
    let allow = fontcraft_oracle::AllowList::builtin().map_err(Failure::Failed)?;
    let settings = Settings { oracle: against, flavour, tool, timestamp, timeout, keep, shown, allow };
    if variable || instances {
        return varrun::run(&jobs, &settings, instances, as_json);
    }

    let ours = |path: &Path| -> Result<fontcraft_oracle::Ours, OursError> {
        let built = fontcraft_engine::guard::catch_panic(|| -> Result<Result<fontcraft_oracle::Ours, OursError>, String> {
            let (font, warnings) = fontcraft_engine::io::open(path, None).map_err(|e| e.to_string())?;
            // The compiler builds one master. For a .glyphs source the reference compilers take
            // the variable font origin as the default: compile that master, so both sides build
            // the same one. Otherwise the first master (a designspace's first source).
            let glyphs = matches!(
                fontcraft_oracle::tools::SourceKind::of(path),
                Some(fontcraft_oracle::tools::SourceKind::Glyphs | fontcraft_oracle::tools::SourceKind::GlyphsPackage)
            );
            let origin = fontcraft_engine::model::axes::origin(&font).filter(|_| glyphs);
            let master = match origin {
                Some(m) => m,
                None => font.default_master().map_err(|e| e.to_string())?,
            };
            let mut warnings = warnings;
            if font.masters.first().is_some_and(|first| first.id != master.id) {
                warnings.push(format!("compiled master `{}`, the variable font origin, instead of the first", master.name));
            }
            let hint = fontcraft_oracle::Hint { family: font.family_name.clone(), style: master.name.clone() };
            let options = fontcraft_engine::compile::Options {
                timestamp,
                remove_overlap: against.removes_overlaps(),
                feature_dir: fontcraft_engine::io::feature_dir(path),
                ..Default::default()
            };
            let target = fontcraft_engine::model::Target::Master(master.id.clone());
            let outlines = match flavour {
                Flavour::TrueType => fontcraft_engine::compile::Flavour::TrueType,
                Flavour::Cff => fontcraft_engine::compile::Flavour::Cff,
            };
            let compiled = fontcraft_engine::compile::compile(&font, &target, outlines, &options).map_err(|e| e.to_string())?;
            warnings.extend(compiled.warnings);
            Ok(Ok(fontcraft_oracle::Ours { bytes: compiled.bytes, warnings, hint }))
        });
        match built {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(OursError::Failed(e)),
            Err(panic) => Err(OursError::Failed(format!("internal error: {panic}"))),
        }
    };

    let mut reports = Vec::with_capacity(jobs.len());
    for (path, label) in &jobs {
        let report = fontcraft_oracle::check(path, label, &settings, || ours(path));
        if !as_json {
            out(fontcraft_oracle::render(&report).trim_end())?;
        }
        reports.push(report);
    }
    let summary = fontcraft_oracle::summarise(&reports);
    if as_json {
        let doc = json!({ "sources": reports, "summary": summary });
        out(&serde_json::to_string_pretty(&doc).map_err(|e| Failure::Failed(e.to_string()))?)?;
    } else if reports.len() > 1 {
        out("")?;
        out(fontcraft_oracle::render_summary(&summary).trim_end())?;
    }
    let differing = reports.iter().filter(|r| r.has_differences() || r.outcome != fontcraft_oracle::Outcome::Compared).count();
    if differing > 0 { Err(Failure::Found(format!("{differing} of {} sources differ or did not build", reports.len()))) } else { Ok(()) }
}

/// `--variable` and `--instances` (M9.10).
mod varrun {
    use crate::io::{Failure, Outcome, out};
    use fontcraft_engine::compile::{self, CompileError};
    use fontcraft_engine::interp::{Designspace, InterpError};
    use fontcraft_engine::model::{Font, Target};
    use fontcraft_oracle::variable::{self, InstanceJob, Space};
    use fontcraft_oracle::{Hint, Ours, OursError, Settings, SourceReport};
    use serde_json::json;
    use std::path::{Path, PathBuf};

    fn guarded<T>(f: impl FnOnce() -> Result<T, OursError>) -> Result<T, OursError> {
        match fontcraft_engine::guard::catch_panic(f) {
            Ok(r) => r,
            Err(panic) => Err(OursError::Failed(format!("internal error: {panic}"))),
        }
    }

    /// The designspace of the variable font FontCraft builds first, in user coordinates.
    fn space_of(font: &Font, setting: Option<usize>) -> Result<Space, InterpError> {
        let ds = match setting {
            Some(s) => Designspace::for_setting(font, s)?,
            None => Designspace::new(font)?,
        };
        let axes = ds
            .axes()
            .iter()
            .map(|a| variable::Axis { tag: a.tag.clone(), minimum: a.external.minimum, default: a.external.default, maximum: a.external.maximum })
            .collect();
        let masters = ds.masters().iter().filter_map(|m| Some((m.name.clone(), ds.to_external(&m.location).ok()?.0))).collect();
        let virtual_masters = ds.virtual_masters().iter().filter_map(|m| Some((m.name.clone(), ds.to_external(&m.location).ok()?.0))).collect();
        let instances = ds
            .named_instances(font)
            .into_iter()
            .filter_map(|i| {
                let name = font.instances.get(i)?.name.clone();
                Some((name, ds.to_external(&ds.instance_location(font, i).ok()?).ok()?.0))
            })
            .collect();
        Ok(Space { axes, masters, virtual_masters, instances })
    }

    fn options(path: &Path, settings: &Settings, remove_overlap: bool) -> compile::Options {
        compile::Options { timestamp: settings.timestamp, remove_overlap, feature_dir: fontcraft_engine::io::feature_dir(path), ..Default::default() }
    }

    fn unsupported(what: &str, hint: Hint) -> OursError {
        OursError::Unsupported { message: format!("{what}: not built yet"), hint }
    }

    fn variable_font(font: &Font, setting: Option<usize>, path: &Path, settings: &Settings, warnings: &[String]) -> Result<Ours, OursError> {
        guarded(|| {
            let hint = Hint { family: font.family_name.clone(), style: String::new() };
            let flavour =
                if settings.flavour == fontcraft_oracle::Flavour::Cff { compile::var::VarFlavour::Cff2 } else { compile::var::VarFlavour::TrueType };
            let var_options = compile::var::VariableOptions { flavour, ..Default::default() };
            match compile::var::compile_variable(font, setting, &options(path, settings, false), &var_options) {
                Ok(v) => {
                    let mut all = warnings.to_vec();
                    all.extend(v.compiled.warnings);
                    Ok(Ours { bytes: v.compiled.bytes, warnings: all, hint: Hint { family: font.family_name.clone(), style: v.file_stem } })
                }
                Err(CompileError::NotYetImplemented(what) | CompileError::Interp(InterpError::NotYetImplemented(what))) => {
                    Err(unsupported(what, hint))
                }
                Err(e) => Err(OursError::Failed(e.to_string())),
            }
        })
    }

    fn instance_font(font: &Font, index: usize, path: &Path, settings: &Settings) -> Result<Ours, OursError> {
        guarded(|| {
            let name = font.instances.get(index).map(|i| i.name.clone()).unwrap_or_default();
            let hint = Hint { family: font.family_name.clone(), style: name };
            let options = options(path, settings, settings.oracle.removes_overlaps());
            let tt = compile::Flavour::TrueType;
            let ours = |c: compile::Compiled| Ours { bytes: c.bytes, warnings: c.warnings, hint: hint.clone() };
            match compile::compile(font, &Target::Instance(index), tt, &options) {
                Ok(c) => Ok(ours(c)),
                Err(CompileError::Interp(InterpError::NotYetImplemented(what))) => Err(unsupported(what, hint.clone())),
                Err(CompileError::NotYetImplemented(what)) => Err(unsupported(what, hint.clone())),
                Err(e) => Err(OursError::Failed(e.to_string())),
            }
        })
    }

    pub(super) fn run(jobs: &[(PathBuf, String)], settings: &Settings, instances: bool, as_json: bool) -> Outcome {
        let mut reports: Vec<SourceReport> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for (path, label) in jobs {
            let opened = fontcraft_engine::guard::catch_panic(|| fontcraft_engine::io::open(path, None).map_err(|e| e.to_string()));
            let (font, warnings) = match opened {
                Ok(Ok(f)) => f,
                Ok(Err(e)) => {
                    out(&format!("{label}  FontCraft cannot read the source: {e}"))?;
                    skipped.push(label.clone());
                    continue;
                }
                Err(panic) => {
                    out(&format!("{label}  FontCraft cannot read the source: internal error: {panic}"))?;
                    skipped.push(label.clone());
                    continue;
                }
            };
            let new: Vec<SourceReport> = if instances {
                let list: Vec<InstanceJob> = font
                    .instances
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.exports && i.kind.as_deref() != Some("variable"))
                    .map(|(index, i)| InstanceJob { index, name: i.name.clone() })
                    .collect();
                if list.is_empty() {
                    skipped.push(label.clone());
                    continue;
                }
                variable::check_instances(path, label, settings, &font.family_name, &list, |i| instance_font(&font, i, path, settings))
            } else {
                if font.masters.len() < 2 {
                    skipped.push(label.clone());
                    continue;
                }
                let setting = compile::var::variable_targets(&font).first().copied().flatten();
                let space = space_of(&font, setting);
                let mut r =
                    variable::check_variable(path, label, settings, space.as_ref().ok(), || variable_font(&font, setting, path, settings, &warnings));
                if let Err(e) = &space {
                    r.notes.push(format!("FontCraft's designspace: {e}; the reference's axes and instances are the locations"));
                }
                vec![r]
            };
            if !as_json {
                for r in &new {
                    out(fontcraft_oracle::render(r).trim_end())?;
                }
            }
            reports.extend(new);
        }
        let summary = fontcraft_oracle::summarise(&reports);
        let measure = variable::measure(&reports);
        let what = if instances { "instances" } else { "variable fonts" };
        let skip_why = if instances { "without an exported static instance" } else { "with one master" };
        if as_json {
            let doc = json!({ "sources": reports, "summary": summary, "measure": measure, "skipped": skipped });
            out(&serde_json::to_string_pretty(&doc).map_err(|e| Failure::Failed(e.to_string()))?)?;
        } else {
            out("")?;
            out(fontcraft_oracle::render_summary(&summary).trim_end())?;
            out(&variable::render_measure(what, &measure))?;
            out(&format!("{} sources skipped ({skip_why}, or unreadable)", skipped.len()))?;
        }
        let failing = measure.different + measure.failed;
        if failing > 0 { Err(Failure::Found(format!("{failing} of {} {what} differ or failed", reports.len()))) } else { Ok(()) }
    }
}
