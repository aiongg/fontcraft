//! `cargo xtask cli-options`: how many of the reference tools' options the command line takes.
//!
//! Reads the mapping tables of `docs/spec/20-cli.md` (one row per reference option, with the
//! option of ours in the **Ours** cell) and its table of options deliberately not carried over,
//! checks every option an **Ours** cell names against the options each subcommand accepts, as
//! `fontcraft-cli commands --cli --json` lists them, and writes the counts to
//! `docs/cli-options.md`. It reports and fails nothing. The spec's "Subcommand reference" says
//! how a cell is read.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

const SPEC: &str = "docs/spec/20-cli.md";
const OUT: &str = "docs/cli-options.md";

/// `help` is answered by the binary itself, not by a subcommand; the listing gives its options apart.
const HELP: &str = "help";

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let listing = match args {
        [] => listing_from_cli()?,
        ["--listing", file] => std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?,
        _ => return Err("usage: cargo xtask cli-options [--listing FILE]".into()),
    };
    let json: Value = serde_json::from_str(&listing).map_err(|e| format!("the subcommand listing is not JSON: {e}"))?;
    let listing = Listing::from_json(&json)?;
    let spec = std::fs::read_to_string(root.join(SPEC)).map_err(|e| format!("{SPEC}: {e}"))?;
    let rows = parse_spec(&spec);
    let report = Report::build(&rows, &listing);
    std::fs::write(root.join(OUT), report.render()).map_err(|e| format!("{OUT}: {e}"))?;
    for (name, c) in &report.subcommands {
        println!("{name:<12} {:>4} of {:<4} present, {:>3} unchecked", c.present, c.present + c.missing, c.unchecked);
    }
    let t = report.total();
    println!(
        "cli-options: {} mapped rows, {} present, {} missing, {} unchecked, {} refused; wrote {OUT}",
        t.mapped(),
        t.present,
        t.missing,
        t.unchecked,
        report.refused_total()
    );
    Ok(())
}

/// The listing of the binary in this workspace, built when needed.
fn listing_from_cli() -> Result<String, String> {
    eprintln!("$ cargo run --quiet -p fontcraft-cli -- commands --cli --json");
    let out = crate::cargo()
        .args(["run", "--quiet", "-p", "fontcraft-cli", "--", "commands", "--cli", "--json"])
        .output()
        .map_err(|e| format!("cargo run: failed to start: {e}"))?;
    if !out.status.success() {
        return Err(format!("fontcraft-cli commands --cli --json failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("the subcommand listing is not UTF-8: {e}"))
}

/// One option the binary accepts.
#[derive(Debug, Default)]
struct Accepted {
    long: Option<String>,
    short: Option<char>,
    /// A switch, so `--no-<long>` is accepted too.
    negatable: bool,
}

impl Accepted {
    fn from_json(v: &Value) -> Self {
        Accepted {
            long: v["long"].as_str().map(str::to_owned),
            short: v["short"].as_str().and_then(|s| s.chars().next()),
            negatable: v["negatable"].as_bool().unwrap_or(false),
        }
    }

    /// Whether it takes `opt`, written `--name` or `-x`.
    fn takes(&self, opt: &str) -> bool {
        if let Some(name) = opt.strip_prefix("--") {
            let Some(long) = self.long.as_deref() else { return false };
            name == long || (self.negatable && name.strip_prefix("no-") == Some(long))
        } else {
            let mut chars = opt.chars().skip(1);
            matches!((chars.next(), chars.next()), (Some(c), None) if self.short == Some(c))
        }
    }
}

/// What `commands --cli --json` says the binary accepts.
#[derive(Debug, Default)]
struct Listing {
    subcommands: BTreeMap<String, Vec<Accepted>>,
    /// Each subcommand's shared behaviours, in the listing's order.
    behaviours: Vec<(String, BTreeMap<String, bool>)>,
    common: Vec<Accepted>,
    help: Vec<Accepted>,
}

impl Listing {
    fn from_json(v: &Value) -> Result<Self, String> {
        let options = |v: &Value| -> Vec<Accepted> { v.as_array().map(|a| a.iter().map(Accepted::from_json).collect()).unwrap_or_default() };
        let subs = v["subcommands"].as_array().ok_or("the subcommand listing has no `subcommands` array")?;
        let mut subcommands = BTreeMap::new();
        let mut behaviours = Vec::new();
        for sub in subs {
            let name = sub["name"].as_str().ok_or("a subcommand in the listing has no name")?;
            subcommands.insert(name.to_owned(), options(&sub["options"]));
            let row = sub["behaviours"].as_object().into_iter().flatten();
            behaviours.push((name.to_owned(), row.map(|(k, v)| (k.clone(), v.as_bool().unwrap_or(false))).collect()));
        }
        Ok(Listing { subcommands, behaviours, common: options(&v["common_options"]), help: options(&v["help_options"]) })
    }

    /// Whether `word` names a subcommand (or `help`).
    fn knows(&self, word: &str) -> bool {
        word == HELP || self.subcommands.contains_key(word)
    }

    /// Whether subcommand `target` accepts `opt`, on its own or as a common option.
    fn accepts(&self, target: &str, opt: &str) -> bool {
        if matches!(opt, "-h" | "--help") {
            return true;
        }
        let own = if target == HELP { Some(&self.help) } else { self.subcommands.get(target) };
        own.is_some_and(|own| own.iter().chain(&self.common).any(|o| o.takes(opt)))
    }
}

/// Which table of the spec a row comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Table {
    /// A subcommand's mapping: reference tool, reference option, ours, note.
    Mapping,
    /// Deliberately not carried over: reference tool, option or behaviour, reason.
    NotCarried,
}

/// One row of a mapping table or of the table of options not carried over.
#[derive(Debug)]
struct SpecRow {
    table: Table,
    /// The section's heading inside its backticks (`tables list`); empty for [`Table::NotCarried`].
    section: String,
    tool: String,
    reference: String,
    ours: String,
}

impl SpecRow {
    /// The subcommand of the section: the first word of its heading.
    fn subcommand(&self) -> &str {
        self.section.split_whitespace().next().unwrap_or("")
    }
}

/// The rows of every mapping table and of the table of options not carried over, in file order.
fn parse_spec(text: &str) -> Vec<SpecRow> {
    let mut rows = Vec::new();
    let mut section: Option<String> = None;
    let mut table: Option<Table> = None;
    for line in text.lines() {
        if line.starts_with('#') {
            table = None;
            let level = line.chars().take_while(|&c| c == '#').count();
            if level <= 2 {
                section = None;
            } else if let Some(name) = heading_name(line) {
                section = Some(name);
            }
            continue;
        }
        if !line.starts_with('|') {
            table = None;
            continue;
        }
        let cells = cells(line);
        let cell = |i: usize| cells.get(i).cloned().unwrap_or_default();
        let Some(kind) = table else {
            table = match (cell(0).as_str(), cell(1).as_str(), cell(2).as_str()) {
                ("Reference tool", "Reference option", "Ours") => Some(Table::Mapping),
                ("Reference tool", "Option or behaviour", _) => Some(Table::NotCarried),
                _ => None,
            };
            continue;
        };
        if line.starts_with("|-") {
            continue;
        }
        match kind {
            Table::Mapping => {
                if let Some(section) = &section {
                    rows.push(SpecRow { table: kind, section: section.clone(), tool: cell(0), reference: cell(1), ours: cell(2) });
                }
            }
            Table::NotCarried => rows.push(SpecRow { table: kind, section: String::new(), tool: cell(0), reference: cell(1), ours: String::new() }),
        }
    }
    rows
}

/// `tables list` from "#### `tables list`"; `None` for a heading that names no subcommand.
fn heading_name(line: &str) -> Option<String> {
    let rest = line.trim_start_matches('#').trim().strip_prefix('`')?;
    let name = &rest[..rest.find('`')?];
    let first = name.split_whitespace().next()?;
    first.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-').then(|| name.to_owned())
}

/// The cells of a table line, trimmed; `\|` inside a cell is a literal bar.
fn cells(line: &str) -> Vec<String> {
    let body = line.trim().strip_prefix('|').unwrap_or(line.trim());
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cell).trim().to_owned()),
            _ => cell.push(c),
        }
    }
    if !cell.trim().is_empty() {
        cells.push(cell.trim().to_owned());
    }
    cells
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    /// An option, normalised to `--name` or `-x`.
    Opt(String),
    Word(String),
    /// The end of a clause: `;`, `,`, a parenthesis, `or`, `and`.
    Break,
}

/// The options, words and clause ends of an **Ours** cell. `-xx` (a repeated letter, as `-vv`)
/// reads as `-x`; other words of one dash and several letters are reference spellings, not ours,
/// and are skipped, as is `--name?` (a query form, not an option).
fn tokens(text: &str) -> Vec<Token> {
    const BREAKS: &[char] = &[';', ',', '(', ')'];
    const SPACES: &[char] = &['/', '[', ']', '`', '|'];
    fn flush(word: &mut String, out: &mut Vec<Token>) {
        if let Some(token) = token(word) {
            out.push(token);
        }
        word.clear();
    }
    let mut out = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if BREAKS.contains(&c) {
            flush(&mut word, &mut out);
            out.push(Token::Break);
        } else if c.is_whitespace() || SPACES.contains(&c) {
            flush(&mut word, &mut out);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn token(word: &str) -> Option<Token> {
    if let Some(rest) = word.strip_prefix("--") {
        if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return None;
        }
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(rest.len());
        if rest[end..].starts_with('?') {
            return None;
        }
        return Some(Token::Opt(format!("--{}", rest[..end].trim_end_matches('-'))));
    }
    if let Some(rest) = word.strip_prefix('-') {
        let end = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
        let (run, after) = rest.split_at(end);
        let mut letters = run.chars();
        let first = letters.next().filter(char::is_ascii_alphabetic)?;
        return (letters.all(|c| c == first) && !after.starts_with('-')).then(|| Token::Opt(format!("-{first}")));
    }
    let word = word.trim_end_matches(['.', ':']);
    match word {
        "" | "also" => None,
        "or" | "and" => Some(Token::Break),
        _ => Some(Token::Word(word.to_owned())),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Present,
    /// The options not accepted, each as `subcommand --option`.
    Missing(Vec<String>),
    Unchecked,
    Refused,
}

/// Reads one mapping row's **Ours** cell against the listing.
fn classify(listing: &Listing, row: &SpecRow) -> Verdict {
    let ours = row.ours.trim();
    let lower = ours.to_lowercase();
    if lower.starts_with("refused") {
        return Verdict::Refused;
    }
    let text = if lower == "same" { row.reference.as_str() } else { ours };
    let section = row.subcommand();
    let tokens = tokens(text);
    let mut target = section;
    let mut option_seen = false;
    let mut named = false;
    let mut checked = 0;
    let mut missing: Vec<String> = Vec::new();
    for t in &tokens {
        match t {
            Token::Break => {
                target = section;
                option_seen = false;
            }
            Token::Word(w) => {
                if !option_seen && listing.knows(w) {
                    target = w.as_str();
                    named = true;
                }
            }
            Token::Opt(o) => {
                option_seen = true;
                checked += 1;
                let what = format!("{target} {o}");
                if !listing.accepts(target, o) && !missing.contains(&what) {
                    missing.push(what);
                }
            }
        }
    }
    if !missing.is_empty() {
        Verdict::Missing(missing)
    } else if checked > 0 || named {
        Verdict::Present
    } else {
        Verdict::Unchecked
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    present: usize,
    missing: usize,
    unchecked: usize,
    refused: usize,
}

impl Counts {
    fn add(&mut self, verdict: &Verdict) {
        match verdict {
            Verdict::Present => self.present += 1,
            Verdict::Missing(_) => self.missing += 1,
            Verdict::Unchecked => self.unchecked += 1,
            Verdict::Refused => self.refused += 1,
        }
    }

    /// Rows of the mapping tables (refusals listed apart are not among them).
    fn mapped(&self) -> usize {
        self.present + self.missing + self.unchecked + self.refused
    }

    /// `P of N (x%)` over the rows that name an option.
    fn coverage(&self) -> String {
        let named = self.present + self.missing;
        if named == 0 {
            return "–".into();
        }
        format!("{} of {named} ({:.1}%)", self.present, 100.0 * self.present as f64 / named as f64)
    }
}

struct MissingRow<'a> {
    row: &'a SpecRow,
    options: Vec<String>,
}

struct Report<'a> {
    /// Per subcommand, in the order of the spec.
    subcommands: Vec<(String, Counts)>,
    /// Per reference tool: the mapping rows, and apart the rows not carried over.
    tools: BTreeMap<String, (Counts, usize)>,
    missing: Vec<MissingRow<'a>>,
    behaviours: &'a [(String, BTreeMap<String, bool>)],
}

impl<'a> Report<'a> {
    fn build(rows: &'a [SpecRow], listing: &'a Listing) -> Self {
        let mut report = Report { subcommands: Vec::new(), tools: BTreeMap::new(), missing: Vec::new(), behaviours: &listing.behaviours };
        for row in rows {
            let tool = report.tools.entry(row.tool.clone()).or_default();
            if row.table == Table::NotCarried {
                tool.1 += 1;
                continue;
            }
            let verdict = classify(listing, row);
            tool.0.add(&verdict);
            let sub = row.subcommand();
            match report.subcommands.iter_mut().find(|(name, _)| name == sub) {
                Some((_, counts)) => counts.add(&verdict),
                None => {
                    let mut counts = Counts::default();
                    counts.add(&verdict);
                    report.subcommands.push((sub.to_owned(), counts));
                }
            }
            if let Verdict::Missing(options) = verdict {
                report.missing.push(MissingRow { row, options });
            }
        }
        report
    }

    fn total(&self) -> Counts {
        self.subcommands.iter().fold(Counts::default(), |mut t, (_, c)| {
            t.present += c.present;
            t.missing += c.missing;
            t.unchecked += c.unchecked;
            t.refused += c.refused;
            t
        })
    }

    /// Refused rows in the mapping tables plus the rows not carried over.
    fn refused_total(&self) -> usize {
        self.tools.values().map(|(c, apart)| c.refused + apart).sum()
    }

    fn render(&self) -> String {
        let mut out = String::new();
        let t = self.total();
        let _ = writeln!(out, "# Option coverage\n");
        let _ = writeln!(
            out,
            "Generated by `cargo xtask cli-options` from the mapping tables of `{SPEC}` and the options each subcommand accepts (`fontcraft-cli commands --cli --json`); do not edit.\n"
        );
        let _ = writeln!(
            out,
            "A mapping row pairs a reference tool's option with ours. It is **present** when every option its Ours cell names is accepted, **missing** when one is not, **unchecked** when the cell names no option (a positional argument, a default or a behaviour), and **refused** when the cell says so. The rows of *Deliberately not carried over* count as refused for their reference tool. Coverage is present out of present and missing. The spec's \"Subcommand reference\" says how a cell is read.\n"
        );
        let _ = writeln!(
            out,
            "**{} mapping rows: {} present, {} missing, {} unchecked, {} refused; coverage {}. {} reference options refused in all.**\n",
            t.mapped(),
            t.present,
            t.missing,
            t.unchecked,
            t.refused,
            t.coverage(),
            self.refused_total()
        );

        let _ = writeln!(out, "## By subcommand\n");
        let _ = writeln!(out, "| Subcommand | Rows | Present | Missing | Unchecked | Refused | Coverage |\n|---|---|---|---|---|---|---|");
        for (name, c) in &self.subcommands {
            let _ =
                writeln!(out, "| `{name}` | {} | {} | {} | {} | {} | {} |", c.mapped(), c.present, c.missing, c.unchecked, c.refused, c.coverage());
        }

        let _ = writeln!(out, "\n## By reference tool\n");
        let _ =
            writeln!(out, "| Reference tool | Rows | Present | Missing | Unchecked | Refused | Not carried over |\n|---|---|---|---|---|---|---|");
        for (tool, (c, apart)) in &self.tools {
            let _ = writeln!(out, "| {} | {} | {} | {} | {} | {} | {apart} |", cell(tool), c.mapped(), c.present, c.missing, c.unchecked, c.refused);
        }

        let columns: Vec<&String> = self.behaviours.first().map(|(_, row)| row.keys().collect()).unwrap_or_default();
        if !columns.is_empty() {
            let _ = writeln!(out, "\n## Shared behaviours\n");
            let _ = writeln!(out, "Which of the behaviours every subcommand should share each one gives (`apps/fontcraft-cli/src/behaviours.rs`).\n");
            let names: Vec<String> = columns.iter().map(|c| format!("`{c}`")).collect();
            let _ = writeln!(out, "| Subcommand | {} |\n|---{}|", names.join(" | "), "|---".repeat(names.len()));
            for (name, row) in self.behaviours {
                let cells: Vec<&str> = columns.iter().map(|c| if row.get(*c).copied().unwrap_or(false) { "yes" } else { "" }).collect();
                let _ = writeln!(out, "| `{name}` | {} |", cells.join(" | "));
            }
        }

        let _ = writeln!(out, "\n## Missing options\n");
        if self.missing.is_empty() {
            let _ = writeln!(out, "None.");
        } else {
            let _ = writeln!(out, "| Section | Reference tool | Reference option | Ours | Not accepted |\n|---|---|---|---|---|");
            for m in &self.missing {
                let _ = writeln!(
                    out,
                    "| `{}` | {} | {} | {} | {} |",
                    m.row.section,
                    cell(&m.row.tool),
                    cell(&m.row.reference),
                    cell(&m.row.ours),
                    cell(&m.options.join(", "))
                );
            }
        }
        out
    }
}

/// Text for a table cell: bars escaped.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_TEXT: &str = "\
# The command line

## Subcommand reference

### `shrink`

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| reftool | `--keep / --no-keep` | same |  |
| reftool | `-o` | -o / --output |  |
| reftool | `-W` | --width a\\|b |  |
| reftool | `-vv` | -vv |  |
| reftool | `--zap` | --zap |  |
| reftool | `-s` | @FILE for arguments; grow --script for lines |  |
| reftool | `-l` | tables list |  |
| reftool | font | FONT... |  |
| reftool | `-x` | refused: no such thing here |  |
| reftool | `--mode` | --layout features --features X |  |

#### `grow more`

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| other | `-a` | --all (or shrink --zap) |  |
| other | `-h` | -h / --help / --version |  |

### `help`

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| other | `-a` | help --all |  |

## Deliberately not carried over

| Reference tool | Option or behaviour | Why it is left out |
|---|---|---|
| reftool | -q | Printer only. |
";

    fn listing() -> Listing {
        let json = serde_json::json!({
            "subcommands": [
                {"name": "shrink", "options": [
                    {"long": "keep", "short": null, "negatable": true},
                    {"long": "output", "short": "o", "negatable": false},
                    {"long": "width", "short": null, "negatable": false},
                    {"long": "layout", "short": null, "negatable": false},
                    {"long": "features", "short": null, "negatable": false}
                ]},
                {"name": "grow", "options": [{"long": "script", "short": null, "negatable": false}, {"long": "all", "short": null, "negatable": true}]},
                {"name": "tables", "options": []}
            ],
            "common_options": [{"long": "verbose", "short": "v", "negatable": true}, {"long": "version", "short": "V", "negatable": true}],
            "help_options": [{"long": "all", "short": null, "negatable": true}]
        });
        Listing::from_json(&json).unwrap()
    }

    #[test]
    fn cells_are_read_against_the_subcommand_they_name() {
        let rows = parse_spec(SPEC_TEXT);
        let listing = listing();
        let verdicts: Vec<(&str, Verdict)> =
            rows.iter().filter(|r| r.table == Table::Mapping).map(|r| (r.ours.as_str(), classify(&listing, r))).collect();
        let expected = [
            ("same", Verdict::Present),
            ("-o / --output", Verdict::Present),
            ("--width a|b", Verdict::Present),
            ("-vv", Verdict::Present),
            ("--zap", Verdict::Missing(vec!["shrink --zap".into()])),
            ("@FILE for arguments; grow --script for lines", Verdict::Present),
            ("tables list", Verdict::Present),
            ("FONT...", Verdict::Unchecked),
            ("refused: no such thing here", Verdict::Refused),
            ("--layout features --features X", Verdict::Present),
            ("--all (or shrink --zap)", Verdict::Missing(vec!["shrink --zap".into()])),
            ("-h / --help / --version", Verdict::Present),
            ("help --all", Verdict::Present),
        ];
        assert_eq!(verdicts, expected.into_iter().collect::<Vec<_>>());

        let report = Report::build(&rows, &listing);
        let names: Vec<&str> = report.subcommands.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["shrink", "grow", "help"], "per subcommand, in the order of the spec");
        assert_eq!(report.subcommands[0].1, Counts { present: 7, missing: 1, unchecked: 1, refused: 1 });
        assert_eq!(report.tools["reftool"], (Counts { present: 7, missing: 1, unchecked: 1, refused: 1 }, 1));
        assert_eq!(report.refused_total(), 2);
        let text = report.render();
        assert!(text.contains("| `shrink` | 10 | 7 | 1 | 1 | 1 | 7 of 8 (87.5%) |"), "{text}");
        assert!(text.contains("| `grow more` | other | `-a` | --all (or shrink --zap) | shrink --zap |"), "{text}");
    }
}
