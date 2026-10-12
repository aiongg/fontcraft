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

const USAGE: &str = "\
  fontcraft-cli oracle (SOURCE... | --corpus [DIR]) [--against fontc|fontmake] [--format ttf|otf]
                      [--reference-bin PATH] [--output-dir DIR] [--timestamp SECS] [--timeout SECS]
                      [--differences N] [--match TEXT] [--json]
      Development aid: compile each source with FontCraft and with a reference compiler (fontc
      for .ttf, fontmake for .otf), read both fonts back and report per table whether they are
      identical, equivalent, allowed or different. --corpus runs over the comparison set in DIR
      (default ./corpus). Exit status 1 when a table differs or a build fails.
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
    let mut it = args.iter().peekable();
    let number = |opt: &str, v: Option<&String>, max: u64| -> Result<u64, Failure> {
        let v = v.ok_or_else(|| usage(format!("{opt} needs a number")))?;
        v.parse::<u64>().ok().filter(|n| *n <= max).ok_or_else(|| usage(format!("{opt} must be a whole number up to {max}, got `{v}`")))
    };
    while let Some(arg) = it.next() {
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
            other if other.starts_with("--") => return Err(usage(format!("unknown oracle option `{other}`"))),
            source => sources.push(source.to_string()),
        }
    }
    let flavour = flavour.unwrap_or(Flavour::TrueType);
    let against = against.unwrap_or(if flavour == Flavour::Cff { Oracle::Fontmake } else { Oracle::Fontc });
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
