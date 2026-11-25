//! `fontcraft-cli stems` (M12.39): a shell over `autohint.ps.reportStems` and
//! `autohint.ps.reportZones`. The report files are laid out as otfstemhist writes them (AFDKO
//! 5.0.1 `otfautohint/report.py`, Apache-2.0, Copyright 2016, 2022 Adobe; see `NOTICE`): one file per
//! list, a title naming the base path and the time, a header line, then one row per value; an
//! empty list writes no file.

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::format::write_atomic;
use serde_json::{Map, Value, json};

use crate::args::{self, OptSpec};
use crate::io::{Failure, Outcome, batch, batch_outcome, execute, out, out_json_line, usage};

use super::Subcommand;
use super::autohint::{Selection, jobs, not_built, open, set_verbosity};

pub const SUBCOMMAND: Subcommand =
    Subcommand { name: "stems", summary: "Report stem widths and alignment zones", usage: USAGE, options: OPTIONS, actions: &[], run };

const USAGE: &str = "\
  fontcraft-cli stems INPUT... [--zones] [--all-stems] [--glyphs LIST] [-o BASEPATH]
      Report the stem widths the autohinter finds in the first master, most frequent first, in
      BASEPATH.hstm.txt and BASEPATH.vstm.txt; with --zones the heights of glyph tops and
      bottoms, in BASEPATH.top.txt and BASEPATH.bot.txt. BASEPATH is the input's path unless -o
      gives one per input; -o - prints the report instead. The input is not changed.
";

/// The options, as data (M12.39).
const OPTIONS: &[OptSpec] = &[
    OptSpec::switch("zones", false, "report the tops and bottoms of glyphs and stems instead of stems").short('z'),
    OptSpec::switch("all-stems", false, "also count stems formed by curves").short('a'),
    OptSpec::list("glyphs", "SELECTOR", "", "measure only these glyphs: names, indexes, /CIDs and A-B ranges").short('g'),
    OptSpec::value("glyphs-file", "FILE", "", "read the glyphs to measure from FILE"),
    OptSpec::list("exclude-glyphs", "SELECTOR", "", "measure every glyph but these").short('x'),
    OptSpec::value("exclude-glyphs-file", "FILE", "", "read the glyphs not to measure from FILE"),
    OptSpec::list("overlap-glyphs", "SELECTOR", "", "glyphs whose overlaps are removed first (not built yet)").short('m'),
    OptSpec::value("overlaps-file", "FILE", "", "read the overlap glyphs from FILE (not built yet)"),
    OptSpec::switch("force-overlap", false, "remove overlaps in every glyph first (not built yet)"),
    OptSpec::switch("force-no-overlap", false, "remove overlaps in no glyph (what the report does)"),
    OptSpec::switch("loose-overlap-mapping", false, "relax the overlap mapping (not built yet)"),
    OptSpec::list("output", "BASEPATH", "", "the base path of each input's report files, one per input; - prints the report").short('o'),
    OptSpec::switch("no-clobber", false, "refuse to replace an existing report file"),
    OptSpec::value("face", "N", "", "the font of a collection to read"),
    OptSpec::switch("json", false, "print each report as one JSON line instead of writing files"),
    OptSpec::switch("quiet", false, "print nothing but errors").short('q'),
    OptSpec::switch("verbose", false, "print more detail on stderr").short('v'),
    OptSpec::value("jobs", "N", "1", "inputs processed at once").short('j'),
];

/// Where one input's report goes.
enum Destination {
    Stdout,
    Files(String),
}

/// One input measured.
struct Measured {
    input: String,
    destination: Destination,
    result: Value,
}

fn run(raw: &[String]) -> Outcome {
    let parsed = args::parse(OPTIONS, raw)?;
    if parsed.help {
        return out(&super::help(&SUBCOMMAND));
    }
    set_verbosity(&parsed);
    if parsed.positional.is_empty() {
        return Err(usage(super::help(&SUBCOMMAND)));
    }
    if parsed.value("overlaps-file").is_some() {
        return Err(not_built("overlaps-file"));
    }
    if !parsed.list("overlap-glyphs").is_empty() {
        return Err(not_built("overlap-glyphs"));
    }
    for name in ["force-overlap", "loose-overlap-mapping"] {
        if parsed.switch(name) {
            return Err(not_built(name));
        }
    }
    let selection = Selection::from_options(&parsed)?;
    let outputs = parsed.list("output");
    if !outputs.is_empty() && outputs.len() != parsed.positional.len() {
        return Err(usage(format!("{} inputs and {} base paths: give one per input", parsed.positional.len(), outputs.len())));
    }
    let zones = parsed.switch("zones");
    let (command, mut shared) = if zones { ("autohint.ps.reportZones", Map::new()) } else { ("autohint.ps.reportStems", Map::new()) };
    if !zones {
        shared.insert("includeCurved".into(), json!(parsed.switch("all-stems")));
    }
    let as_json = parsed.switch("json");
    let write = !as_json;

    let inputs: Vec<String> = (0..parsed.positional.len()).map(|i| i.to_string()).collect();
    let results = batch(&inputs, jobs(&parsed)?, |key| {
        let i: usize = key.parse().map_err(|_| Failure::Failed("bad input index".into()))?;
        let input = parsed.positional.get(i).cloned().unwrap_or_default();
        let base = outputs.get(i).cloned().unwrap_or_else(|| input.trim_end_matches(['/', '\\']).to_string());
        let destination = if base == "-" { Destination::Stdout } else { Destination::Files(base) };
        let mut session = Session::new();
        open(&mut session, &input, parsed.value("face"))?;
        let mut params = shared.clone();
        if let Some(sel) = &selection {
            let (key, names) = sel.resolve(&mut session, &input)?;
            params.insert(key.into(), json!(names));
        }
        let result = execute(&mut session, command, &Value::Object(params))?;
        if write && let Destination::Files(base) = &destination {
            write_reports(base, &result, zones, parsed.switch("no-clobber"))?;
        }
        Ok(Measured { input, destination, result })
    })?;
    for r in &results {
        match r {
            Ok(m) if as_json => out_json_line(&json!({"input": m.input, "result": m.result}))?,
            Ok(m) => match &m.destination {
                Destination::Stdout => out(m.result.get("text").and_then(Value::as_str).unwrap_or_default().trim_end())?,
                Destination::Files(base) => {
                    let files: Vec<String> =
                        report_files(base, &m.result, zones).into_iter().map(|(p, _)| p.to_string_lossy().into_owned()).collect();
                    let verb = if write { "wrote" } else { "would write" };
                    let list = if files.is_empty() { "nothing to report".to_string() } else { format!("{verb} {}", files.join(", ")) };
                    out(&format!("{}: {list}", m.input))?;
                }
            },
            Err(e) => eprintln!("fontcraft-cli: {}", e.message()),
        }
    }
    batch_outcome(&results)
}

/// The report files of one result and their contents, the empty lists left out.
fn report_files(base: &str, result: &Value, zones: bool) -> Vec<(PathBuf, String)> {
    let time = asctime(report_time());
    let lists: [(&str, &str, &str, &str); 2] = if zones {
        [("top", ".top.txt", "Top Zone List", "count   height    glyphs"), ("bottom", ".bot.txt", "Bottom Zone List", "count   height    glyphs")]
    } else {
        [
            ("horizontal", ".hstm.txt", "Horizontal Stem List", "count    width    glyphs"),
            ("vertical", ".vstm.txt", "Vertical Stem List", "count    width    glyphs"),
        ]
    };
    let value_key = if zones { "height" } else { "width" };
    let mut files = Vec::new();
    for (key, suffix, title, header) in lists {
        let rows = result.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        if rows.is_empty() {
            continue;
        }
        let mut text = format!("{title} for {base} on {time}\n{header}\n");
        for row in &rows {
            let count = row.get("count").and_then(Value::as_u64).unwrap_or(0);
            let value = row.get(value_key).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(0, |v| v.round() as i64);
            let glyphs: Vec<&str> = row.get("glyphs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            text.push_str(&format!("{count:5}    {value:5}    [{}]\n", glyphs.join(" ")));
        }
        files.push((PathBuf::from(format!("{base}{suffix}")), text));
    }
    files
}

/// Writes the report files of one result, each through a temporary file renamed into place.
fn write_reports(base: &str, result: &Value, zones: bool, no_clobber: bool) -> Outcome {
    let files = report_files(base, result, zones);
    if no_clobber && let Some((path, _)) = files.iter().find(|(p, _)| p.exists()) {
        return Err(Failure::Failed(format!("{} exists and --no-clobber is set", path.display())));
    }
    for (path, text) in &files {
        write_atomic(path, text.as_bytes()).map_err(|e| Failure::Failed(e.to_string()))?;
    }
    Ok(())
}

/// The time in the report titles: `SOURCE_DATE_EPOCH` when set, so runs can be compared,
/// otherwise the clock.
fn report_time() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

/// Latest time [`asctime`] formats: the end of year 9999.
const MAX_SECONDS: u64 = 253_402_300_799;

/// `secs` after the epoch, in UTC, in C's `asctime` layout (`Sun Oct  4 07:08:09 2026`).
fn asctime(secs: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let secs = secs.min(MAX_SECONDS);
    let days = secs / 86_400;
    let rest = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let weekday = DAYS.get((days % 7) as usize).copied().unwrap_or("???");
    let month_name = MONTHS.get(month.saturating_sub(1) as usize).copied().unwrap_or("???");
    format!("{weekday} {month_name} {day:2} {:02}:{:02}:{:02} {year}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Year, month (1 to 12) and day of the month of a day count since 1970-01-01, in the proleptic
/// Gregorian calendar (the days-to-civil method of Howard Hinnant's date algorithms).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asctime_matches_the_c_layout() {
        assert_eq!(asctime(0), "Thu Jan  1 00:00:00 1970");
        assert_eq!(asctime(951_782_400), "Tue Feb 29 00:00:00 2000");
        assert_eq!(asctime(1_791_401_889), "Wed Oct  7 19:38:09 2026");
    }

    #[test]
    fn report_files_follow_the_reference_layout() {
        let result = json!({
            "horizontal": [],
            "vertical": [{"width": 80.0, "count": 2, "glyphs": ["I", "l"]}, {"width": 112.0, "count": 1, "glyphs": ["H"]}],
        });
        let files = report_files("Font.ufo", &result, false);
        assert_eq!(files.len(), 1, "an empty list writes no file");
        let (path, text) = &files[0];
        assert_eq!(path, &PathBuf::from("Font.ufo.vstm.txt"));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("Vertical Stem List for Font.ufo on "), "{text}");
        assert_eq!(&lines[1..], ["count    width    glyphs", "    2       80    [I l]", "    1      112    [H]"]);
    }

    #[test]
    fn stems_refuses_overlap_options_and_unreadable_inputs() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(run(&args(&["in.ufo", "--force-overlap"])).unwrap_err().exit_code(), 4);
        assert_eq!(run(&args(&["a.ufo", "b.ufo", "-o", "x"])).unwrap_err().exit_code(), 2);
        assert_eq!(run(&args(&["/nonexistent/in.ufo"])).unwrap_err().exit_code(), 3);
        assert!(run(&args(&["--help"])).is_ok());
    }
}
