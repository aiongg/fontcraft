//! `cargo xtask glyphdata --check`: the glyph database against fontc and glyphsLib.
//!
//! The name set is the measure of `docs/plans/M4.md` § 5: every record of the main GlyphsInfo
//! file, and a derived set (every record name with `.sc`, `.ss01` and `.001`; 10,000 two- and
//! three-part ligatures drawn with a fixed seed from the letters and marks of one script;
//! `uniXXXX` for every BMP code point with a record; 1,000 `_` names). Each name is looked up in
//! FontCraft's database, in fontc's `glyphs-reader` 1.0.0 through `xtask/glyphdata-oracle`
//! (category, subcategory, production name, code point) and in glyphsLib through `crates/glyphdata/crosscheck/glyphslib.py` (the same and
//! script, case and description). Differences must match an entry of
//! `crates/glyphdata/crosscheck/allow.toml`, which gives the cause and the number of names it
//! covers; the check fails on any other difference and on an entry whose count is off.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use fontcraft_glyphdata::{GlyphData, Info, Source};

const SEED: u64 = 0x4D34_2E31;
const LIGATURES: usize = 10_000;
const UNDERSCORE_NAMES: usize = 1_000;
const GLYPHSLIB_VERSION: &str = "6.15.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Set {
    Records,
    Suffix,
    Ligature,
    Uni,
    Underscore,
}

impl Set {
    fn as_str(self) -> &'static str {
        match self {
            Set::Records => "records",
            Set::Suffix => "suffix",
            Set::Ligature => "ligature",
            Set::Uni => "uni",
            Set::Underscore => "underscore",
        }
    }
}

/// splitmix64: a fixed-seed generator, so the derived names are the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn name_set(root: &Path) -> Result<Vec<(Set, String)>, String> {
    let d = crate::glyphdata::dir(root).join("glyphsinfo");
    let read = |f: &str| -> Result<Vec<fontcraft_glyphdata::Record>, String> {
        let bytes = std::fs::read(d.join(f)).map_err(|e| format!("{f}: {e} (run `cargo xtask glyphdata --fetch`)"))?;
        Ok(fontcraft_glyphdata::xml::parse(&bytes).map_err(|e| format!("{f}: {e}"))?.records)
    };
    let main = read("GlyphData.xml")?;
    let ideographs = read("GlyphData_Ideographs.xml")?;
    let mut out: Vec<(Set, String)> = main.iter().map(|r| (Set::Records, r.name.clone())).collect();
    for suffix in [".sc", ".ss01", ".001"] {
        out.extend(main.iter().map(|r| (Set::Suffix, format!("{}{suffix}", r.name))));
    }
    // Ligatures: parts are letters and marks of one script, without dots or underscores. A
    // script suffix the parts share is written once, at the end.
    let mut by_script: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for r in &main {
        let part = !r.name.contains(['.', '_']) && matches!(r.category.map(|c| c.as_str()), Some("Letter" | "Mark"));
        if part {
            by_script.entry(r.script.clone().unwrap_or_default()).or_default().push(&r.name);
        }
    }
    let scripts: Vec<&Vec<&str>> = by_script.values().filter(|v| v.len() >= 2).collect();
    let mut rng = Rng(SEED);
    let mut ligatures = BTreeSet::new();
    while ligatures.len() < LIGATURES {
        let Some(pool) = scripts.get(rng.below(scripts.len())) else { break };
        let n = 2 + rng.below(2);
        let parts: Vec<&str> = (0..n).filter_map(|_| pool.get(rng.below(pool.len())).copied()).collect();
        let suffix = parts.first().and_then(|p| p.rsplit_once('-')).map(|(_, s)| s);
        let name = match suffix.filter(|s| parts.iter().all(|p| p.ends_with(&format!("-{s}")))) {
            Some(s) => format!("{}-{s}", parts.iter().map(|p| p.strip_suffix(&format!("-{s}")).unwrap_or(p)).collect::<Vec<_>>().join("_")),
            None => parts.join("_"),
        };
        ligatures.insert(name);
    }
    out.extend(ligatures.into_iter().map(|n| (Set::Ligature, n)));
    let bmp: BTreeSet<u32> = main.iter().chain(&ideographs).filter_map(|r| r.unicode).filter(|cp| *cp <= 0xFFFF).collect();
    out.extend(bmp.into_iter().map(|cp| (Set::Uni, format!("uni{cp:04X}"))));
    let suffixes = ["", ".part", ".a", ".alt"];
    for _ in 0..UNDERSCORE_NAMES {
        let base = main.get(rng.below(main.len())).map_or("x", |r| r.name.as_str());
        let suffix = suffixes.get(rng.below(suffixes.len())).copied().unwrap_or("");
        out.push((Set::Underscore, format!("_{base}{suffix}")));
    }
    Ok(out)
}

/// The compared fields of one answer.
#[derive(Clone, Debug, Default)]
struct Answer {
    fields: BTreeMap<&'static str, Option<String>>,
    /// Our source of each field.
    sources: BTreeMap<&'static str, Source>,
}

const FONTC_FIELDS: &[&str] = &["category", "subCategory", "production", "unicode"];
const GLYPHSLIB_FIELDS: &[&str] = &["category", "subCategory", "production", "unicode", "script", "case", "description"];

fn ours(info: &Info) -> Answer {
    let mut a = Answer::default();
    let mut put = |k: &'static str, v: Option<String>, s: Option<Source>| {
        a.fields.insert(k, v);
        if let Some(s) = s {
            a.sources.insert(k, s);
        }
    };
    put("category", info.category.as_ref().map(|c| c.value.to_string()), info.category.as_ref().map(|c| c.source));
    put("subCategory", info.sub_category.as_ref().map(|c| c.value.to_string()), info.sub_category.as_ref().map(|c| c.source));
    put("production", Some(info.production_name.value.clone()), Some(info.production_name.source));
    put("unicode", info.unicode.as_ref().map(|u| format!("{:04X}", u.value)), info.unicode.as_ref().map(|u| u.source));
    put("script", info.script.as_ref().map(|s| s.value.clone()), info.script.as_ref().map(|s| s.source));
    put("case", info.case.as_ref().map(|c| c.value.to_string()), info.case.as_ref().map(|c| c.source));
    put("description", info.description.as_ref().map(|d| d.value.clone()), info.description.as_ref().map(|d| d.source));
    a
}

/// fontc's answers from `xtask/glyphdata-oracle`, its own workspace: glyphs-reader pins an
/// icu_properties the main workspace cannot share.
fn fontc(root: &Path, names: &[(Set, String)]) -> Result<Vec<Answer>, String> {
    let manifest = root.join("xtask/glyphdata-oracle/Cargo.toml");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut child = Command::new(cargo)
        .args(["run", "--quiet", "--release", "--manifest-path"])
        .arg(&manifest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let input: String = names.iter().map(|(_, n)| format!("{n}\n")).collect();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut out = Vec::with_capacity(names.len());
    for line in std::io::BufReader::new(stdout).lines() {
        let v: serde_json::Value = serde_json::from_str(&line.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut a = Answer::default();
        for f in FONTC_FIELDS {
            a.fields.insert(f, v.get(*f).and_then(|x| x.as_str()).map(str::to_string));
        }
        out.push(a);
    }
    writer.join().map_err(|_| "writer thread failed")?.map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() || out.len() != names.len() {
        return Err(format!("glyphdata-oracle failed ({status}; {} of {} answers)", out.len(), names.len()));
    }
    Ok(out)
}

fn python() -> PathBuf {
    if let Some(p) = std::env::var_os("FONTMAKE_PYTHON") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let uv = home.join(".local/share/uv/tools/fontmake/bin/python");
    if uv.exists() { uv } else { PathBuf::from("python3") }
}

/// glyphsLib's answers, or `None` with a message when its Python cannot run.
fn glyphslib(root: &Path, names: &[(Set, String)]) -> Result<(String, Vec<Answer>), String> {
    let script = root.join("crates/glyphdata/crosscheck/glyphslib.py");
    let mut child = Command::new(python())
        .arg("-I")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", python().display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let input: String = names.iter().map(|(_, n)| format!("{n}\n")).collect();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut lines = std::io::BufReader::new(stdout).lines();
    let header: serde_json::Value =
        serde_json::from_str(&lines.next().ok_or("glyphslib.py wrote nothing")?.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let version = header.get("glyphsLib").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let mut out = Vec::with_capacity(names.len());
    for line in lines {
        let v: serde_json::Value = serde_json::from_str(&line.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let mut a = Answer::default();
        for f in GLYPHSLIB_FIELDS {
            a.fields.insert(f, s(f));
        }
        let name = s("name").unwrap_or_default();
        // glyphsLib exports a glyph without a production name under its own name.
        if a.fields.get("production").is_some_and(Option::is_none) {
            a.fields.insert("production", Some(name));
        }
        out.push(a);
    }
    writer.join().map_err(|_| "writer thread failed")?.map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() || out.len() != names.len() {
        return Err(format!("glyphslib.py: {status}, {} of {} answers", out.len(), names.len()));
    }
    Ok((version, out))
}

#[derive(Debug)]
struct Entry {
    oracle: String,
    field: String,
    sets: Vec<String>,
    ours: String,
    theirs: String,
    ours_source: Option<String>,
    names: Vec<String>,
    count: usize,
    cause: String,
    matched: Vec<String>,
}

fn allow_list(root: &Path) -> Result<Vec<Entry>, String> {
    let path = root.join("crates/glyphdata/crosscheck/allow.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let t: toml::Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let rows = t.get("allow").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    rows.iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let list = |k: &str| {
                r.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
            };
            Ok(Entry {
                oracle: s("oracle").ok_or("allow.toml: an entry without `oracle`")?,
                field: s("field").ok_or("allow.toml: an entry without `field`")?,
                sets: list("sets"),
                ours: s("ours").unwrap_or_else(|| "*".into()),
                theirs: s("theirs").unwrap_or_else(|| "*".into()),
                ours_source: s("ours_source"),
                names: list("names"),
                count: r.get("count").and_then(|v| v.as_integer()).ok_or("allow.toml: an entry without `count`")? as usize,
                cause: s("cause").filter(|c| !c.trim().is_empty()).ok_or("allow.toml: an entry without `cause`")?,
                matched: Vec::new(),
            })
        })
        .collect()
}

/// A name with every character other than `A–Z a–z 0–9 _ .` removed, as glyphsLib cleans a
/// production name it cannot build.
fn sanitised(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.')).collect()
}

/// `ours-sanitised`: the value is ours, cleaned; `name-sanitised`: the glyph name, cleaned.
fn value_matches(pattern: &str, value: &Option<String>, ours: &Option<String>, name: &str) -> bool {
    match pattern {
        "*" => true,
        "none" => value.is_none(),
        "some" => value.is_some(),
        "ours-sanitised" => ours.as_deref().map(sanitised) == *value,
        "name-sanitised" => Some(sanitised(name)) == *value,
        literal => value.as_deref() == Some(literal),
    }
}

struct Difference {
    set: Set,
    name: String,
    field: &'static str,
    ours: Option<String>,
    theirs: Option<String>,
    source: Option<Source>,
}

fn compare(
    oracle: &str,
    fields: &[&'static str],
    names: &[(Set, String)],
    ours: &[Answer],
    theirs: &[Answer],
    allow: &mut [Entry],
    verbose: bool,
) -> usize {
    let mut agree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut allowed: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unexplained: Vec<Difference> = Vec::new();
    for (((set, name), o), t) in names.iter().zip(ours).zip(theirs) {
        for field in fields {
            let ov = o.fields.get(field).cloned().flatten();
            let tv = t.fields.get(field).cloned().flatten();
            if ov == tv {
                *agree.entry(field).or_default() += 1;
                continue;
            }
            let source = o.sources.get(field).copied();
            let entry = allow.iter_mut().find(|e| {
                e.oracle == oracle
                    && e.field == *field
                    && (e.sets.is_empty() || e.sets.iter().any(|s| s == set.as_str()))
                    && value_matches(&e.ours, &ov, &ov, name)
                    && value_matches(&e.theirs, &tv, &ov, name)
                    && e.ours_source.as_deref().is_none_or(|s| source.is_some_and(|x| x.as_str() == s))
                    && (e.names.is_empty() || e.names.iter().any(|n| n == name))
            });
            match entry {
                Some(e) => {
                    e.matched.push(format!("{name}: ours {ov:?}, {oracle} {tv:?}"));
                    *allowed.entry(field).or_default() += 1;
                }
                None => unexplained.push(Difference { set: *set, name: name.clone(), field, ours: ov, theirs: tv, source }),
            }
        }
    }
    println!("\nagainst {oracle}: {} names", names.len());
    println!("  {:<12} {:>9} {:>13} {:>12}", "field", "agree", "allow-listed", "unexplained");
    for field in fields {
        let u = unexplained.iter().filter(|d| d.field == *field).count();
        println!("  {:<12} {:>9} {:>13} {:>12}", field, agree.get(field).unwrap_or(&0), allowed.get(field).unwrap_or(&0), u);
    }
    if !unexplained.is_empty() {
        // Grouped by what an allow-list entry would match on.
        let mut groups: BTreeMap<(String, Set, String, String, String), Vec<&Difference>> = BTreeMap::new();
        for d in &unexplained {
            let kind = |v: &Option<String>| if v.is_some() { "some" } else { "none" }.to_string();
            let src = d.source.map_or("-", |s| s.as_str()).to_string();
            groups.entry((d.field.to_string(), d.set, kind(&d.ours), kind(&d.theirs), src)).or_default().push(d);
        }
        println!("  unexplained differences:");
        for ((field, set, o, t, src), ds) in &groups {
            println!("    {field} in {}: ours {o} ({src}), {oracle} {t}: {} names", set.as_str(), ds.len());
            for d in ds.iter().take(if verbose { 40 } else { 6 }) {
                println!("      {}: ours {:?}, {oracle} {:?}", d.name, d.ours, d.theirs);
            }
        }
    }
    unexplained.len()
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let verbose = args.contains(&"-v");
    let start = Instant::now();
    let data = GlyphData::new().map_err(|e| e.to_string())?;
    let _ = data.lookup("A-cy.sc", &[]);
    let first_lookup = start.elapsed();
    let start = Instant::now();
    let _ = data.lookup("lam_alef-ar.fina", &[]);
    let second_lookup = start.elapsed();

    let names = name_set(root)?;
    let start = Instant::now();
    let ours: Vec<Answer> = names.iter().map(|(_, n)| ours(&data.lookup(n, &[]))).collect();
    let ours_time = start.elapsed();
    let mut by_set: BTreeMap<Set, usize> = BTreeMap::new();
    for (s, _) in &names {
        *by_set.entry(*s).or_default() += 1;
    }
    println!(
        "glyph database check: {} names ({})",
        names.len(),
        by_set.iter().map(|(s, n)| format!("{} {n}", s.as_str())).collect::<Vec<_>>().join(", ")
    );
    println!(
        "first lookup (load and query): {:.1} ms; next lookup: {:.3} ms; all names: {:.0} ms",
        first_lookup.as_secs_f64() * 1e3,
        second_lookup.as_secs_f64() * 1e3,
        ours_time.as_secs_f64() * 1e3
    );

    let mut allow = allow_list(root)?;
    let fontc_answers = fontc(root, &names)?;
    let mut failures = compare("fontc", FONTC_FIELDS, &names, &ours, &fontc_answers, &mut allow, verbose);

    match glyphslib(root, &names) {
        Ok((version, answers)) => {
            if version != GLYPHSLIB_VERSION {
                println!("\nnote: glyphsLib is {version}; the allow list was written against {GLYPHSLIB_VERSION}");
            }
            failures += compare("glyphslib", GLYPHSLIB_FIELDS, &names, &ours, &answers, &mut allow, verbose);
        }
        Err(e) => {
            println!("\nglyphsLib skipped: {e}");
            allow.retain(|e| e.oracle != "glyphslib");
        }
    }

    println!("\nallow list (crates/glyphdata/crosscheck/allow.toml):");
    let mut stale = 0;
    for e in &allow {
        let ok = e.matched.len() == e.count;
        if !ok {
            stale += 1;
        }
        println!(
            "  {} {} {}: {} names{}",
            e.oracle,
            e.field,
            if e.sets.is_empty() { "all".to_string() } else { e.sets.join("+") },
            e.matched.len(),
            if ok { String::new() } else { format!(" (listed: {})", e.count) }
        );
        println!("    {}", e.cause);
        for m in e.matched.iter().take(if verbose { 10 } else { 2 }) {
            println!("      {m}");
        }
    }
    if failures > 0 || stale > 0 {
        return Err(format!("{failures} unexplained differences; {stale} allow-list entries with a different count"));
    }
    println!("\nglyph database check passed");
    Ok(())
}
