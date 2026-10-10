//! `cargo xtask glyphsets`: the sources of the built-in language and glyph-set data.
//!
//! The pins are in `xtask/glyphsets.toml`. `--fetch` downloads the Unicode CLDR locale files and
//! the Google Fonts glyph sets at their commits (a sparse, blob-less git fetch of the listed
//! paths, as `glyphdata --fetch` does) into `corpus/glyphsets/` (git-ignored). `--generate`
//! builds `crates/glyphdata/data/sets.bin` and writes the licence notices beside it.
//!
//! Languages: every CLDR locale whose file has its own main exemplar set (`<exemplarCharacters>`
//! without a `type` or `alt`), parsed as a UnicodeSet. Strings such as `{dž}` add each of their
//! characters. CLDR lists letters in lowercase, so each character's uppercase form is added when it
//! is a single character (the Georgian Mtavruli capitals excepted: Georgian text does not use
//! them as an uppercase). A language is listed under the glyph database's script that most of its
//! characters belong to (characters of the Unicode scripts Common and Inherited do not count). Its
//! English name comes from `en.xml`: the longest leading run of subtags with a language name, then
//! the script, region and variant names in brackets.
//!
//! Glyph sets: each `data/results/glyphs/<file>.glyphs` listed in the pins, read for every glyph's
//! name and code point.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use fontcraft_glyphdata::GlyphData;
use fontcraft_glyphdata::sets::{GlyphSet, Language, SetGlyph, Sets};
use toml::Table;

use crate::glyphdata::GitPin;

pub struct SetPin {
    pub file: String,
    pub id: String,
    pub name: String,
    pub script: String,
    pub default: bool,
}

pub struct Pins {
    pub cldr: GitPin,
    pub cldr_release: String,
    pub glyphsets: GitPin,
    pub sets: Vec<SetPin>,
}

const TOML: &str = "xtask/glyphsets.toml";

fn git_pin(t: &Table, key: &str) -> Result<GitPin, String> {
    let s = t.get(key).and_then(|v| v.as_table()).ok_or_else(|| format!("{TOML}: no [{key}]"))?;
    let get = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let paths = s
        .get("paths")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>())
        .unwrap_or_default();
    let pin = GitPin { repo: get("repo"), commit: get("commit"), paths };
    if pin.commit.len() != 40 || !pin.commit.chars().all(|c| c.is_ascii_hexdigit()) || !pin.repo.starts_with("https://") || pin.paths.is_empty() {
        return Err(format!("{TOML}: [{key}] needs an https repo, a 40-digit commit and paths"));
    }
    Ok(pin)
}

pub fn pins(root: &Path) -> Result<Pins, String> {
    let path = root.join(TOML);
    let t: Table =
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let cldr_release = t.get("cldr").and_then(|c| c.get("release")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    if cldr_release.is_empty() {
        return Err(format!("{TOML}: [cldr] needs a release"));
    }
    let mut sets = Vec::new();
    for (i, s) in t.get("set").and_then(|v| v.as_array()).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        let get = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let pin = SetPin {
            file: get("file"),
            id: get("id"),
            name: get("name"),
            script: get("script"),
            default: s.get("default").and_then(|v| v.as_bool()).unwrap_or(false),
        };
        if pin.file.is_empty() || pin.id.is_empty() || pin.name.is_empty() || !is_script_id(&pin.script) {
            return Err(format!("{TOML}: [[set]] {} needs a file, an id, a name and a script id", i + 1));
        }
        if sets.iter().any(|p: &SetPin| p.id == pin.id || p.file == pin.file) {
            return Err(format!("{TOML}: [[set]] {} repeats an id or a file", pin.id));
        }
        sets.push(pin);
    }
    if sets.is_empty() {
        return Err(format!("{TOML}: no [[set]]"));
    }
    Ok(Pins { cldr: git_pin(&t, "cldr")?, cldr_release, glyphsets: git_pin(&t, "glyphsets")?, sets })
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("corpus/glyphsets")
}

/// A script id as the sidebar accepts it: lowercase letters, digits, `_` or `-`.
fn is_script_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").current_dir(dir).args(args).output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn fetch_git(dir: &Path, pin: &GitPin) -> Result<(), String> {
    if dir.is_dir() && git(dir, &["rev-parse", "HEAD"]).ok().as_deref() == Some(pin.commit.as_str()) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !dir.join(".git").exists() {
        git(dir, &["init", "-q"])?;
        git(dir, &["remote", "add", "origin", &pin.repo])?;
    }
    eprintln!("fetching {} at {}…", pin.repo, &pin.commit[..12]);
    git(dir, &["fetch", "-q", "--depth", "1", "--filter=blob:none", "origin", &pin.commit])?;
    let mut args = vec!["sparse-checkout", "set", "--no-cone"];
    args.extend(pin.paths.iter().map(String::as_str));
    git(dir, &args)?;
    git(dir, &["checkout", "-q", "--detach", "FETCH_HEAD"])?;
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e} (run `cargo xtask glyphsets --fetch`)", path.display()))
}

fn check_head(d: &Path, name: &str, pin: &GitPin) -> Result<(), String> {
    if git(&d.join(name), &["rev-parse", "HEAD"]).unwrap_or_default() != pin.commit {
        return Err(format!("corpus/glyphsets/{name} is not at the pinned commit (run `cargo xtask glyphsets --fetch`)"));
    }
    Ok(())
}

// ── A little XML, enough for CLDR's locale files ────────────────────────────────────────────────

/// `text` with `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and numeric references decoded.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        match rest[at..].find("-->") {
            Some(end) => rest = &rest[at + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Every `<tag attr="…">text</tag>` (or `<tag …/>`) in `xml`: its attributes and its text, decoded.
fn elements(xml: &str, tag: &str) -> Vec<(BTreeMap<String, String>, String)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let after = &rest[at + open.len()..];
        // `<tag` must not be the start of a longer name.
        if !after.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let head = &after[..gt];
        let attrs = attributes(head.trim_end_matches('/'));
        if head.ends_with('/') {
            out.push((attrs, String::new()));
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push((attrs, unescape(&body[..end])));
        rest = &body[end + close.len()..];
    }
    out
}

fn attributes(head: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut rest = head;
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let Some(end) = after[1..].find(quote) else { break };
        out.insert(name, unescape(&after[1..end + 1]));
        rest = &after[end + 2..];
    }
    out
}

/// The text of the elements `<tag type="…">` inside `<section>` that have no `alt`, by type.
fn display_names(xml: &str, section: &str, tag: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (_, body) in elements(xml, section) {
        for (attrs, text) in elements(&body, tag) {
            if let Some(t) = attrs.get("type")
                && !attrs.contains_key("alt")
                && !text.trim().is_empty()
            {
                out.entry(t.clone()).or_insert_with(|| text.trim().to_string());
            }
        }
    }
    out
}

/// English display names from CLDR's `en.xml`.
struct Names {
    languages: BTreeMap<String, String>,
    scripts: BTreeMap<String, String>,
    territories: BTreeMap<String, String>,
    variants: BTreeMap<String, String>,
}

impl Names {
    fn read(en_xml: &str) -> Names {
        let xml = strip_comments(en_xml);
        Names {
            languages: display_names(&xml, "languages", "language"),
            scripts: display_names(&xml, "scripts", "script"),
            territories: display_names(&xml, "territories", "territory"),
            variants: display_names(&xml, "variants", "variant"),
        }
    }

    fn qualifier(&self, subtag: &str) -> String {
        let is_script = subtag.len() == 4 && subtag.starts_with(|c: char| c.is_ascii_uppercase());
        let is_region = (subtag.len() == 2 && subtag.chars().all(|c| c.is_ascii_uppercase()))
            || (subtag.len() == 3 && subtag.chars().all(|c| c.is_ascii_digit()));
        let found = if is_script {
            self.scripts.get(subtag)
        } else if is_region {
            self.territories.get(subtag)
        } else {
            self.variants.get(&subtag.to_ascii_uppercase())
        };
        found.cloned().unwrap_or_else(|| subtag.to_string())
    }

    /// `sr_Latn_BA` → `Serbian (Latin, Bosnia & Herzegovina)`; `zh_Hant` → `Traditional Chinese`.
    fn english(&self, locale: &str) -> String {
        let parts: Vec<&str> = locale.split('_').collect();
        for k in (1..=parts.len()).rev() {
            if let Some(base) = self.languages.get(&parts[..k].join("_")) {
                let quals: Vec<String> = parts[k..].iter().map(|p| self.qualifier(p)).collect();
                return if quals.is_empty() { base.clone() } else { format!("{base} ({})", quals.join(", ")) };
            }
        }
        locale.replace('_', "-")
    }
}

// ── UnicodeSet, the subset CLDR's exemplar sets use ─────────────────────────────────────────────

/// Largest range `a-z` accepted in a set.
const MAX_RANGE: u32 = 0x1_0000;

/// One character at `i`, with escapes (`\uXXXX`, `\UXXXXXXXX`, `\x{…}`, `\` before any other
/// character); returns it and the position after it.
fn set_char(chars: &[char], i: usize) -> Result<(u32, usize), String> {
    let c = *chars.get(i).ok_or("a set ends early")?;
    if c != '\\' {
        return Ok((c as u32, i + 1));
    }
    let next = *chars.get(i + 1).ok_or("a set ends with `\\`")?;
    let hex = |from: usize, len: usize| -> Result<u32, String> {
        let digits: String = chars.get(from..from + len).ok_or("a short escape")?.iter().collect();
        u32::from_str_radix(&digits, 16).map_err(|_| format!("a bad escape `{digits}`"))
    };
    match next {
        'u' => Ok((hex(i + 2, 4)?, i + 6)),
        'U' => Ok((hex(i + 2, 8)?, i + 10)),
        'x' if chars.get(i + 2) == Some(&'{') => {
            let end = (i + 3..chars.len()).find(|&j| chars[j] == '}').ok_or("an unclosed `\\x{`")?;
            Ok((hex(i + 3, end - i - 3)?, end + 1))
        }
        'x' => Ok((hex(i + 2, 2)?, i + 4)),
        other => Ok((other as u32, i + 2)),
    }
}

/// The code points of a UnicodeSet such as `[a b {dž} c-e ́]`.
fn unicode_set(text: &str) -> Result<BTreeSet<u32>, String> {
    let inner = text.trim().strip_prefix('[').and_then(|t| t.strip_suffix(']')).ok_or("a set is not in brackets")?;
    let chars: Vec<char> = inner.chars().collect();
    let mut out = BTreeSet::new();
    let mut last: Option<u32> = None;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        match c {
            c if c.is_whitespace() => i += 1,
            '{' => {
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err("an unclosed `{`".into()),
                        Some('}') => {
                            i += 1;
                            break;
                        }
                        Some(_) => {
                            let (cp, next) = set_char(&chars, i)?;
                            out.insert(cp);
                            i = next;
                        }
                    }
                }
                last = None;
            }
            '-' if last.is_some() && chars.get(i + 1).is_some() => {
                // Whitespace inside a set means nothing, also around `-`.
                let from = (i + 1..chars.len()).find(|&j| !chars[j].is_whitespace()).ok_or("a range has no end")?;
                let (end, next) = set_char(&chars, from)?;
                let start = last.unwrap_or(end);
                if end < start || end - start > MAX_RANGE {
                    return Err(format!("a bad range {start:X}-{end:X}"));
                }
                out.extend(start..=end);
                last = None;
                i = next;
            }
            '[' | ']' | '^' | '$' | '&' => return Err(format!("set syntax `{c}` is not supported")),
            _ => {
                let (cp, next) = set_char(&chars, i)?;
                out.insert(cp);
                last = Some(cp);
                i = next;
            }
        }
    }
    if out.iter().any(|&cp| char::from_u32(cp).is_none()) {
        return Err("a set holds something that is not a character".into());
    }
    Ok(out)
}

/// The set with the single-character uppercase form of each character added.
fn with_uppercase(set: &BTreeSet<u32>) -> Vec<u32> {
    let mut out = set.clone();
    for &cp in set {
        let Some(c) = char::from_u32(cp) else { continue };
        let mut up = c.to_uppercase();
        if let (Some(u), None) = (up.next(), up.next())
            && u != c
            && !(0x1C90..=0x1CBF).contains(&(u as u32))
        {
            out.insert(u as u32);
        }
    }
    out.into_iter().collect()
}

/// The script most of the characters belong to: the glyph database's id, or the Unicode script
/// name in lowercase when the database has none. Ties go to the first in alphabetical order.
fn script_of(data: &GlyphData, characters: &[u32]) -> Option<String> {
    let mut votes: BTreeMap<String, usize> = BTreeMap::new();
    for &cp in characters {
        let Some(unicode) = data.unicode_script(cp) else { continue };
        if matches!(unicode, "Common" | "Inherited" | "Unknown") {
            continue;
        }
        let id = match data.script_for_code_point(cp) {
            Some(s) => s.to_string(),
            None => unicode.to_lowercase().replace([' ', '-'], "_"),
        };
        *votes.entry(id).or_default() += 1;
    }
    let best = votes.values().copied().max()?;
    votes.into_iter().find(|(_, n)| *n == best).map(|(s, _)| s)
}

/// The main exemplar set of a locale file: `None` when it has none of its own.
fn main_exemplars(xml: &str) -> Result<Option<BTreeSet<u32>>, String> {
    let xml = strip_comments(xml);
    let Some((_, text)) = elements(&xml, "exemplarCharacters").into_iter().find(|(a, _)| !a.contains_key("type") && !a.contains_key("alt")) else {
        return Ok(None);
    };
    // `↑↑↑` means "inherited from the parent locale".
    if text.trim() == "\u{2191}\u{2191}\u{2191}" {
        return Ok(None);
    }
    let set = unicode_set(&text)?;
    Ok(if set.is_empty() { None } else { Some(set) })
}

fn languages(root: &Path, data: &GlyphData, notes: &mut Vec<String>) -> Result<Vec<Language>, String> {
    let main = dir(root).join("cldr/common/main");
    let names = Names::read(&read(&main.join("en.xml"))?);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&main)
        .map_err(|e| format!("{}: {e} (run `cargo xtask glyphsets --fetch`)", main.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "xml"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Some(locale) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        if locale == "root" {
            continue;
        }
        let Some(set) = main_exemplars(&read(&path)?).map_err(|e| format!("{locale}.xml: {e}"))? else { continue };
        let characters = with_uppercase(&set);
        let Some(script) = script_of(data, &characters).filter(|s| is_script_id(s)) else {
            notes.push(format!("{locale}: no script for its characters, left out"));
            continue;
        };
        out.push(Language { id: locale.replace('_', "-"), name: names.english(&locale), script, characters });
    }
    out.sort_by(|a, b| (&a.script, &a.name, &a.id).cmp(&(&b.script, &b.name, &b.id)));
    Ok(out)
}

// ── Glyph sets ──────────────────────────────────────────────────────────────────────────────────

fn plist_text(v: &fontcraft_plist::Value) -> Option<String> {
    match v {
        fontcraft_plist::Value::String(s) => Some(s.as_str().to_string()),
        fontcraft_plist::Value::Number(n) => n.spelling().map(str::to_string),
        _ => None,
    }
}

/// A `unicode` value of a format-3 source: a decimal number, or a list of them.
fn plist_code_points(v: &fontcraft_plist::Value) -> Result<Vec<u32>, String> {
    let one = |v: &fontcraft_plist::Value| match v.as_num() {
        Some(fontcraft_plist::Num::Int(i)) => {
            u32::try_from(i).ok().filter(|cp| char::from_u32(*cp).is_some()).ok_or(format!("{i} is not a code point"))
        }
        _ => Err("a unicode value is not a number".to_string()),
    };
    match v {
        fontcraft_plist::Value::Array(a) => a.items.iter().map(one).collect(),
        other => one(other).map(|cp| vec![cp]),
    }
}

fn glyph_set(path: &Path, pin: &SetPin, data: &GlyphData, notes: &mut Vec<String>) -> Result<GlyphSet, String> {
    let text = read(path)?;
    let doc = fontcraft_plist::openstep::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let version = doc.root.as_dict().and_then(|d| d.get(".formatVersion")).and_then(|v| v.as_num());
    if !matches!(version, Some(fontcraft_plist::Num::Int(3))) {
        return Err(format!("{}: expected a format-3 source", path.display()));
    }
    let items = doc.root.as_dict().and_then(|d| d.get("glyphs")).and_then(|v| v.as_array()).ok_or(format!("{}: no glyphs", path.display()))?;
    let mut glyphs: Vec<SetGlyph> = Vec::new();
    let mut scripts: BTreeMap<String, usize> = BTreeMap::new();
    for item in &items.items {
        let d = item.as_dict().ok_or(format!("{}: a glyph is not a dictionary", path.display()))?;
        let name = d.get("glyphname").and_then(plist_text).filter(|n| !n.is_empty()).ok_or(format!("{}: a glyph has no name", path.display()))?;
        let codes = match d.get("unicode") {
            Some(v) => plist_code_points(v).map_err(|e| format!("{}: {name}: {e}", path.display()))?,
            None => Vec::new(),
        };
        if codes.len() > 1 {
            notes.push(format!("{}: {name} has {} code points; the first is kept", pin.id, codes.len()));
        }
        let code_point = codes.first().copied();
        if let Some(s) = code_point.and_then(|cp| data.script_for_code_point(cp)) {
            *scripts.entry(s.to_string()).or_default() += 1;
        }
        if glyphs.iter().any(|g| g.name == name) {
            notes.push(format!("{}: {name} is listed twice; the first is kept", pin.id));
            continue;
        }
        glyphs.push(SetGlyph { name, code_point });
    }
    if let Some((most, _)) = scripts.iter().max_by_key(|(_, n)| **n)
        && *most != pin.script
    {
        notes.push(format!("{}: listed under {}, most of its encoded glyphs are {most}", pin.id, pin.script));
    }
    Ok(GlyphSet { id: pin.id.clone(), name: pin.name.clone(), script: pin.script.clone(), default: pin.default, glyphs })
}

fn glyph_sets(root: &Path, pins: &Pins, data: &GlyphData, notes: &mut Vec<String>) -> Result<Vec<GlyphSet>, String> {
    let results = dir(root).join("glyphsets/data/results/glyphs");
    let mut found: Vec<String> = std::fs::read_dir(&results)
        .map_err(|e| format!("{}: {e} (run `cargo xtask glyphsets --fetch`)", results.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "glyphs"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();
    found.sort();
    let unlisted: Vec<&String> = found.iter().filter(|f| !pins.sets.iter().any(|p| &p.file == *f)).collect();
    if !unlisted.is_empty() {
        return Err(format!("glyph set files not listed in {TOML}: {unlisted:?}"));
    }
    pins.sets.iter().map(|pin| glyph_set(&results.join(format!("{}.glyphs", pin.file)), pin, data, notes)).collect()
}

// ── Licences and the command ────────────────────────────────────────────────────────────────────

fn notices(root: &Path, pins: &Pins) -> Result<Vec<(&'static str, String)>, String> {
    let d = dir(root);
    let cldr = format!(
        "Unicode CLDR ({}), {} (commit {}): the exemplar characters and English display names of common/main.\n\n{}\n",
        pins.cldr.repo,
        pins.cldr_release,
        pins.cldr.commit,
        read(&d.join("cldr/LICENSE"))?.trim()
    );
    let glyphsets = format!(
        "Google Fonts glyph sets ({}), data/results/glyphs at commit {}. Their characters come from gflanguages \
         (https://github.com/googlefonts/lang, Apache License 2.0), adapted from Unicode CLDR (see LICENSE-CLDR.txt).\n\n{}\n",
        pins.glyphsets.repo,
        pins.glyphsets.commit,
        read(&d.join("glyphsets/LICENSE"))?.trim()
    );
    Ok(vec![("LICENSE-CLDR.txt", cldr), ("LICENSE-glyphsets.txt", glyphsets)])
}

fn generate(root: &Path, pins: &Pins) -> Result<(), String> {
    let d = dir(root);
    check_head(&d, "cldr", &pins.cldr)?;
    check_head(&d, "glyphsets", &pins.glyphsets)?;
    let data = GlyphData::new().map_err(|e| e.to_string())?;
    let mut notes = Vec::new();
    let languages = languages(root, &data, &mut notes)?;
    let glyph_sets = glyph_sets(root, pins, &data, &mut notes)?;
    let meta =
        format!("cldr={} {} {}\nglyphsets={} {}\n", pins.cldr.repo, pins.cldr_release, pins.cldr.commit, pins.glyphsets.repo, pins.glyphsets.commit);
    let sets = Sets { meta, languages, glyph_sets };
    let blob = sets.write()?;
    // The reader must accept what was written.
    if Sets::open(&blob).map_err(|e| e.to_string())? != sets {
        return Err("the written data does not read back equal".into());
    }
    let out = root.join("crates/glyphdata/data");
    std::fs::write(out.join("sets.bin"), &blob).map_err(|e| format!("sets.bin: {e}"))?;
    for (file, text) in notices(root, pins)? {
        std::fs::write(out.join(file), text).map_err(|e| format!("{file}: {e}"))?;
    }
    let mut per_script: BTreeMap<&str, usize> = BTreeMap::new();
    for l in &sets.languages {
        *per_script.entry(l.script.as_str()).or_default() += 1;
    }
    let characters: usize = sets.languages.iter().map(|l| l.characters.len()).sum();
    let set_glyphs: std::collections::HashSet<&SetGlyph> = sets.glyph_sets.iter().flat_map(|s| s.glyphs.iter()).collect();
    println!("  {:<28} {}", "languages", sets.languages.len());
    println!("  {:<28} {}", "scripts with languages", per_script.len());
    println!("  {:<28} {}", "language characters", characters);
    println!("  {:<28} {}", "glyph sets", sets.glyph_sets.len());
    println!("  {:<28} {}", "distinct glyph-set glyphs", set_glyphs.len());
    for (script, n) in &per_script {
        println!("    {script:<26} {n}");
    }
    for note in &notes {
        println!("  note: {note}");
    }
    println!("wrote crates/glyphdata/data/sets.bin ({} bytes)", blob.len());
    Ok(())
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let pins = pins(root)?;
    let mut did = false;
    if args.contains(&"--fetch") {
        fetch_git(&dir(root).join("cldr"), &pins.cldr)?;
        fetch_git(&dir(root).join("glyphsets"), &pins.glyphsets)?;
        println!("language and glyph-set sources are in {}", dir(root).display());
        did = true;
    }
    if args.contains(&"--generate") {
        generate(root, &pins)?;
        did = true;
    }
    if !did {
        return Err("usage: cargo xtask glyphsets [--fetch] [--generate]".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pins_are_well_formed() {
        let p = pins(&crate::root()).unwrap();
        assert!(p.sets.iter().any(|s| s.default));
    }

    #[test]
    fn unicode_sets_and_names_read_as_cldr_writes_them() {
        let set = unicode_set("[a b {dž} c-e \\u0301 {n\\-} \\:]").unwrap();
        let want: BTreeSet<u32> = ['a', 'b', 'd', 'ž', 'c', 'e', '\u{301}', 'n', '-', ':'].iter().map(|c| *c as u32).collect();
        assert_eq!(set, want);
        assert!(unicode_set("[[:Latin:]]").is_err());
        assert_eq!(with_uppercase(&['a' as u32, 'ß' as u32].into_iter().collect()), vec!['A' as u32, 'a' as u32, 'ß' as u32]);
        let en = r#"<ldml><!-- <languages><language type="xx">No</language></languages> -->
            <localeDisplayNames><languages><language type="sr">Serbian</language><language type="zh_Hant">Traditional Chinese</language>
            <language type="sr" alt="short">Srb</language></languages><scripts><script type="Latn">Latin</script></scripts>
            <territories><territory type="BA">Bosnia &amp; Herzegovina</territory></territories><variants></variants></localeDisplayNames></ldml>"#;
        let names = Names::read(en);
        assert_eq!(names.english("sr_Latn_BA"), "Serbian (Latin, Bosnia & Herzegovina)");
        assert_eq!(names.english("zh_Hant"), "Traditional Chinese");
        assert_eq!(names.english("xx"), "xx");
        let locale = r#"<characters><exemplarCharacters type="auxiliary">[q]</exemplarCharacters><exemplarCharacters draft="contributed">[a ä]</exemplarCharacters></characters>"#;
        assert_eq!(main_exemplars(locale).unwrap(), Some(['a' as u32, 'ä' as u32].into_iter().collect()));
        assert_eq!(main_exemplars("<exemplarCharacters>↑↑↑</exemplarCharacters>").unwrap(), None);
    }
}
