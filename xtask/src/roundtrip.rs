//! `cargo xtask roundtrip`: read every corpus source and write it back, and count how many come
//! back byte for byte.
//!
//! Each source kind has a codec: a function that reads a file (or a directory, for packages and
//! UFOs) and returns what the writer produces for it. The harness compares that with the file on
//! disk and reports identical, different (with the first differing line) or failed. A codec that
//! panics is reported as failed; the run goes on. Known differences with an accepted reason are
//! listed in `xtask/roundtrip-allow.toml` and reported as allowed.
//!
//! The codecs arrive with M1.1 (generic property lists) to M1.4 (UFO and designspace); see
//! `docs/plans/M1.md`. Until a kind has one, its samples are counted and listed as waiting.

use std::collections::BTreeMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use toml::Table;

/// What a codec writes back: one file, or a directory tree as relative path → bytes.
// Constructed by the codecs, the first of which lands with M1.1.
#[cfg_attr(not(test), allow(dead_code))]
pub enum Written {
    File(Vec<u8>),
    Tree(BTreeMap<String, Vec<u8>>),
}

pub type Codec = fn(&Path) -> Result<Written, String>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    Glyphs,
    GlyphsPackage,
    Ufo,
    Ufoz,
    Designspace,
}

impl Kind {
    const ALL: [Kind; 5] = [Kind::Glyphs, Kind::GlyphsPackage, Kind::Ufo, Kind::Ufoz, Kind::Designspace];

    fn of(path: &Path) -> Option<Kind> {
        match path.extension()?.to_str()? {
            "glyphs" => Some(Kind::Glyphs),
            "glyphspackage" => Some(Kind::GlyphsPackage),
            "ufo" => Some(Kind::Ufo),
            "ufoz" => Some(Kind::Ufoz),
            "designspace" => Some(Kind::Designspace),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Glyphs => "glyphs",
            Kind::GlyphsPackage => "glyphspackage",
            Kind::Ufo => "ufo",
            Kind::Ufoz => "ufoz",
            Kind::Designspace => "designspace",
        }
    }

    fn is_dir(self) -> bool {
        matches!(self, Kind::GlyphsPackage | Kind::Ufo)
    }
}

/// One corpus source.
pub struct Sample {
    /// Relative to `corpus/`, with forward slashes.
    pub path: String,
    pub kind: Kind,
    /// Format version as the file states it ("2", "3", "4" for `.glyphs`; "2", "3" for UFO; the
    /// `format` attribute for designspace). `None` when it could not be read.
    pub version: Option<String>,
    /// Saved by the reference tool for that format: the reference app for `.glyphs` (it writes
    /// `.appVersion`), fontTools ufoLib for UFO. Only these are held to byte identity.
    pub reference_written: bool,
}

/// Every source under `corpus/`. Packages and UFOs are one sample each; nothing inside them is
/// a sample of its own.
pub fn discover(corpus: &Path) -> Result<Vec<Sample>, String> {
    let mut out = Vec::new();
    walk(corpus, corpus, &mut out)?;
    out.sort_by(|a, b| (a.kind, &a.path).cmp(&(b.kind, &b.path)));
    Ok(out)
}

fn walk(corpus: &Path, dir: &Path, out: &mut Vec<Sample>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> =
        std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        let kind = Kind::of(&path).filter(|k| k.is_dir() == path.is_dir());
        if let Some(kind) = kind {
            let rel = path.strip_prefix(corpus).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            let (version, reference_written) = sniff(&path, kind);
            out.push(Sample { path: rel, kind, version, reference_written });
        } else if path.is_dir() {
            walk(corpus, &path, out)?;
        }
    }
    Ok(())
}

/// The first `limit` bytes of a file as text (lossy), or empty when it cannot be read.
fn head(path: &Path, limit: usize) -> String {
    use std::io::Read;
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(limit as u64).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn sniff(path: &Path, kind: Kind) -> (Option<String>, bool) {
    match kind {
        Kind::Glyphs | Kind::GlyphsPackage => {
            let text = if kind == Kind::Glyphs { head(path, 4096) } else { head(&path.join("fontinfo.plist"), 4096) };
            if text.is_empty() {
                return (None, false);
            }
            // The version keys come first because `.glyphs` keys are sorted and `.` sorts early.
            let version = text
                .lines()
                .find_map(|l| l.strip_prefix(".formatVersion = ").map(|v| v.trim_end_matches(';').trim().to_string()))
                .unwrap_or_else(|| "2".to_string());
            let app = text.lines().any(|l| l.starts_with(".appVersion = "));
            (Some(version), app)
        }
        Kind::Ufo => {
            let meta = head(&path.join("metainfo.plist"), 4096);
            let version = plist_value_after(&meta, "formatVersion");
            let creator = plist_value_after(&meta, "creator");
            (version, creator.as_deref() == Some("com.github.fonttools.ufoLib"))
        }
        Kind::Ufoz => (None, false),
        Kind::Designspace => {
            let text = head(path, 4096);
            let version = text.split("format=\"").nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
            (version, false)
        }
    }
}

/// In an XML property list, the text of the element after `<key>{key}</key>`.
fn plist_value_after(text: &str, key: &str) -> Option<String> {
    let after = text.split(&format!("<key>{key}</key>")).nth(1)?;
    let open = after.find('>')?;
    let rest = after.get(open + 1..)?;
    let close = rest.find('<')?;
    Some(rest.get(..close)?.trim().to_string())
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Identical,
    /// The first difference: file (for trees) and line, with both versions of the line.
    Different(String),
    Allowed(String),
    Failed(String),
    NoCodec,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Identical => write!(f, "identical"),
            Outcome::Different(d) => write!(f, "different: {d}"),
            Outcome::Allowed(r) => write!(f, "allowed: {r}"),
            Outcome::Failed(e) => write!(f, "failed: {e}"),
            Outcome::NoCodec => write!(f, "no codec yet"),
        }
    }
}

/// The first line where `written` differs from `original`, or `None` when the bytes are equal.
pub fn first_difference(original: &[u8], written: &[u8]) -> Option<String> {
    if original == written {
        return None;
    }
    let a = String::from_utf8_lossy(original);
    let b = String::from_utf8_lossy(written);
    let mut la = a.split_inclusive('\n');
    let mut lb = b.split_inclusive('\n');
    let mut n = 1usize;
    loop {
        match (la.next(), lb.next()) {
            (Some(x), Some(y)) if x == y => n += 1,
            (x, y) => {
                let show = |s: Option<&str>| s.map_or("<end of file>".to_string(), |s| format!("{:?}", s.trim_end_matches('\n')));
                return Some(format!("line {n}: was {}, wrote {}", clip(&show(x)), clip(&show(y))));
            }
        }
    }
}

fn clip(s: &str) -> String {
    const MAX: usize = 120;
    if s.chars().count() <= MAX { s.to_string() } else { format!("{}…", s.chars().take(MAX).collect::<String>()) }
}

/// Every file under `dir`, as relative path → bytes. Hidden files are included: a writer that
/// drops one has lost data.
pub fn read_tree(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    fn go(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                go(root, &path, out)?;
            } else {
                let rel = path.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    go(dir, dir, &mut out)?;
    Ok(out)
}

/// The first difference between two trees: a missing or extra file, or the first differing line.
pub fn tree_difference(original: &BTreeMap<String, Vec<u8>>, written: &BTreeMap<String, Vec<u8>>) -> Option<String> {
    for (path, bytes) in original {
        match written.get(path) {
            None => return Some(format!("{path}: not written")),
            Some(w) => {
                if let Some(d) = first_difference(bytes, w) {
                    return Some(format!("{path}: {d}"));
                }
            }
        }
    }
    written.keys().find(|p| !original.contains_key(*p)).map(|p| format!("{p}: written but not in the original"))
}

/// Run `codec` on one sample and compare its output with the original.
pub fn check(corpus: &Path, sample: &Sample, codec: Codec, allow: &BTreeMap<String, String>) -> Outcome {
    let path = corpus.join(&sample.path);
    let result = catch_unwind(AssertUnwindSafe(|| codec(&path)));
    let difference = match result {
        Err(_) => return Outcome::Failed("the codec panicked".to_string()),
        Ok(Err(e)) => return Outcome::Failed(e),
        Ok(Ok(Written::File(bytes))) => match std::fs::read(&path) {
            Ok(original) => first_difference(&original, &bytes),
            Err(e) => return Outcome::Failed(format!("reading the original: {e}")),
        },
        Ok(Ok(Written::Tree(tree))) => match read_tree(&path) {
            Ok(original) => tree_difference(&original, &tree),
            Err(e) => return Outcome::Failed(format!("reading the original: {e}")),
        },
    };
    match (difference, allow.get(&sample.path)) {
        (None, _) => Outcome::Identical,
        (Some(_), Some(reason)) => Outcome::Allowed(reason.clone()),
        (Some(d), None) => Outcome::Different(d),
    }
}

/// The codec for each kind, for the given mode. Empty until M1.1.
fn codec(_mode: &str, _kind: Kind) -> Option<Codec> {
    None
}

const MODES: [&str; 2] = ["plist", "model"];

fn allow_list(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = root.join("xtask/roundtrip-allow.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let table: Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = BTreeMap::new();
    for row in table.get("allow").and_then(|v| v.as_array()).map(Vec::as_slice).unwrap_or_default() {
        let get = |k: &str| row.get(k).and_then(|v| v.as_str()).map(str::to_string);
        match (get("path"), get("reason")) {
            (Some(p), Some(r)) if !r.trim().is_empty() => {
                out.insert(p, r);
            }
            _ => return Err(format!("{}: every [[allow]] row needs a `path` and a non-empty `reason`", path.display())),
        }
    }
    Ok(out)
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut mode = "model";
    let mut kinds: Vec<Kind> = Kind::ALL.to_vec();
    let mut verbose = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--mode" => {
                let m = it.next().ok_or("--mode needs a value")?;
                mode = MODES.iter().find(|x| *x == m).ok_or_else(|| format!("unknown mode `{m}` (modes: {})", MODES.join(", ")))?;
            }
            "--kind" => {
                let k = it.next().ok_or("--kind needs a value")?;
                let kind = Kind::ALL.into_iter().find(|x| x.name() == *k).ok_or_else(|| format!("unknown kind `{k}`"))?;
                kinds = vec![kind];
            }
            "-v" | "--verbose" => verbose = true,
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    let corpus = root.join("corpus");
    if !corpus.is_dir() {
        return Err("corpus/ is missing: run `cargo xtask corpus --fetch` first".to_string());
    }
    let allow = allow_list(root)?;
    let samples = discover(&corpus)?;

    // (kind, version, reference_written) → outcome counts
    type Key = (Kind, String, bool);
    let mut table: BTreeMap<Key, [usize; 5]> = BTreeMap::new();
    let mut details = Vec::new();
    for s in samples.iter().filter(|s| kinds.contains(&s.kind)) {
        let outcome = match codec(mode, s.kind) {
            Some(c) => check(&corpus, s, c, &allow),
            None => Outcome::NoCodec,
        };
        let slot = match outcome {
            Outcome::Identical => 0,
            Outcome::Allowed(_) => 1,
            Outcome::Different(_) => 2,
            Outcome::Failed(_) => 3,
            Outcome::NoCodec => 4,
        };
        let key = (s.kind, s.version.clone().unwrap_or_else(|| "?".into()), s.reference_written);
        if let Some(cell) = table.entry(key).or_default().get_mut(slot) {
            *cell += 1;
        }
        if verbose || matches!(outcome, Outcome::Different(_) | Outcome::Failed(_)) {
            details.push(format!("  {}  {outcome}", s.path));
        }
    }

    println!("Round trip, mode `{mode}`: read each corpus source and write it back.\n");
    println!(
        "{:<14} {:>7} {:>9}  {:>9} {:>7} {:>9} {:>6} {:>8}",
        "kind", "version", "reference", "identical", "allowed", "different", "failed", "no codec"
    );
    let mut total = [0usize; 5];
    for ((kind, version, reference), counts) in &table {
        let [a, b, c, d, e] = *counts;
        println!("{:<14} {:>7} {:>9}  {a:>9} {b:>7} {c:>9} {d:>6} {e:>8}", kind.name(), version, if *reference { "yes" } else { "no" });
        for (t, n) in total.iter_mut().zip(counts) {
            *t += n;
        }
    }
    let [a, b, c, d, e] = total;
    println!("{:<14} {:>7} {:>9}  {a:>9} {b:>7} {c:>9} {d:>6} {e:>8}", "total", "", "");
    println!("\n`reference` = saved by the format's reference tool; only those files are held to byte identity.");
    if !details.is_empty() {
        println!();
        for d in &details {
            println!("{d}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_bytes_have_no_difference() {
        assert_eq!(first_difference(b"a\nb\n", b"a\nb\n"), None);
    }

    #[test]
    fn the_first_differing_line_is_reported() {
        let d = first_difference(b"a\nb\nc\n", b"a\nB\nc\n").unwrap();
        assert!(d.starts_with("line 2:"), "{d}");
        assert!(d.contains("\"b\"") && d.contains("\"B\""), "{d}");
    }

    #[test]
    fn a_shorter_output_is_reported_at_its_end() {
        let d = first_difference(b"a\nb\n", b"a\n").unwrap();
        assert!(d.contains("line 2") && d.contains("<end of file>"), "{d}");
    }

    #[test]
    fn trees_report_missing_and_extra_files() {
        let orig: BTreeMap<String, Vec<u8>> = [("a".to_string(), b"1".to_vec())].into();
        let none = BTreeMap::new();
        assert_eq!(tree_difference(&orig, &none).unwrap(), "a: not written");
        let extra: BTreeMap<String, Vec<u8>> = [("a".to_string(), b"1".to_vec()), ("b".to_string(), b"2".to_vec())].into();
        assert_eq!(tree_difference(&orig, &extra).unwrap(), "b: written but not in the original");
        assert_eq!(tree_difference(&orig, &orig.clone()), None);
    }

    #[test]
    fn plist_values_are_found_after_their_key() {
        let meta = "<dict>\n<key>creator</key>\n<string>com.github.fonttools.ufoLib</string>\n<key>formatVersion</key>\n<integer>3</integer>";
        assert_eq!(plist_value_after(meta, "creator").as_deref(), Some("com.github.fonttools.ufoLib"));
        assert_eq!(plist_value_after(meta, "formatVersion").as_deref(), Some("3"));
        assert_eq!(plist_value_after(meta, "missing"), None);
    }

    #[test]
    fn a_panicking_codec_is_a_failure() {
        let dir = std::env::temp_dir().join(format!("fontcraft-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.glyphs"), b"{}").unwrap();
        let sample = Sample { path: "x.glyphs".into(), kind: Kind::Glyphs, version: None, reference_written: false };
        fn boom(_: &Path) -> Result<Written, String> {
            panic!("boom")
        }
        fn same(p: &Path) -> Result<Written, String> {
            std::fs::read(p).map(Written::File).map_err(|e| e.to_string())
        }
        let allow = BTreeMap::new();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let failed = check(&dir, &sample, boom, &allow);
        std::panic::set_hook(prev);
        assert_eq!(failed, Outcome::Failed("the codec panicked".into()));
        assert_eq!(check(&dir, &sample, same, &allow), Outcome::Identical);
        let _ = std::fs::remove_file(dir.join("x.glyphs"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_tree_codec_is_compared_file_by_file() {
        let dir = std::env::temp_dir().join(format!("fontcraft-roundtrip-tree-{}", std::process::id()));
        let pkg = dir.join("x.glyphspackage");
        std::fs::create_dir_all(pkg.join("glyphs")).unwrap();
        std::fs::write(pkg.join("fontinfo.plist"), b"{\n}\n").unwrap();
        std::fs::write(pkg.join("glyphs/A_.glyph"), b"{\nglyphname = A;\n}\n").unwrap();
        let sample = Sample { path: "x.glyphspackage".into(), kind: Kind::GlyphsPackage, version: None, reference_written: false };
        fn same(p: &Path) -> Result<Written, String> {
            read_tree(p).map(Written::Tree)
        }
        fn drops_a_glyph(p: &Path) -> Result<Written, String> {
            let mut t = read_tree(p)?;
            t.remove("glyphs/A_.glyph");
            Ok(Written::Tree(t))
        }
        let allow = BTreeMap::new();
        assert_eq!(check(&dir, &sample, same, &allow), Outcome::Identical);
        assert_eq!(check(&dir, &sample, drops_a_glyph, &allow), Outcome::Different("glyphs/A_.glyph: not written".into()));
        let allowed: BTreeMap<String, String> = [("x.glyphspackage".to_string(), "test".to_string())].into();
        assert_eq!(check(&dir, &sample, drops_a_glyph, &allowed), Outcome::Allowed("test".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
