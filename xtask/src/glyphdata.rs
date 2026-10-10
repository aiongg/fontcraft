//! `cargo xtask glyphdata`: the sources of the built-in glyph database.
//!
//! The pins are in `xtask/glyphdata.toml`. `--fetch` downloads GlyphsInfo and the Adobe Glyph List
//! at their commits (a sparse, blob-less git fetch of the listed paths, as `corpus --fetch` does)
//! and the Unicode files by URL, checked against their SHA-256, into `corpus/glyphdata/`
//! (git-ignored). `--generate` builds `crates/glyphdata/data/glyphdata.bin` and copies the
//! licence notices beside it. `--check` compares the database with the oracles
//! (`glyphdata_check.rs`).

use std::path::{Path, PathBuf};
use std::process::Command;

use toml::Table;

pub struct GitPin {
    pub repo: String,
    pub commit: String,
    pub paths: Vec<String>,
}

pub struct UcdPin {
    pub version: String,
    pub url: String,
    pub files: Vec<(String, String)>,
}

pub struct Pins {
    pub glyphsinfo: GitPin,
    pub agl: GitPin,
    pub ucd: UcdPin,
}

fn strings(v: Option<&toml::Value>) -> Vec<String> {
    v.and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn git_pin(t: &Table, key: &str) -> Result<GitPin, String> {
    let s = t.get(key).and_then(|v| v.as_table()).ok_or_else(|| format!("xtask/glyphdata.toml: no [{key}]"))?;
    let get = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let pin = GitPin { repo: get("repo"), commit: get("commit"), paths: strings(s.get("paths")) };
    if pin.commit.len() != 40 || !pin.commit.chars().all(|c| c.is_ascii_hexdigit()) || !pin.repo.starts_with("https://") || pin.paths.is_empty() {
        return Err(format!("xtask/glyphdata.toml: [{key}] needs an https repo, a 40-digit commit and paths"));
    }
    Ok(pin)
}

pub fn pins(root: &Path) -> Result<Pins, String> {
    let path = root.join("xtask/glyphdata.toml");
    let t: Table =
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let u = t.get("ucd").and_then(|v| v.as_table()).ok_or("xtask/glyphdata.toml: no [ucd]")?;
    let get = |k: &str| u.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let files = u
        .get("files")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|f| Some((f.get("path")?.as_str()?.to_string(), f.get("sha256")?.as_str()?.to_string()))).collect::<Vec<_>>())
        .unwrap_or_default();
    let ucd = UcdPin { version: get("version"), url: get("url"), files };
    if !ucd.url.starts_with("https://") || ucd.files.len() != 3 || ucd.version.is_empty() {
        return Err("xtask/glyphdata.toml: [ucd] needs a version, an https url and three files with their sha256".into());
    }
    Ok(Pins { glyphsinfo: git_pin(&t, "glyphsinfo")?, agl: git_pin(&t, "agl")?, ucd })
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("corpus/glyphdata")
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

fn ucd_file(root: &Path, ucd: &UcdPin, path: &str) -> PathBuf {
    dir(root).join(format!("ucd-{}", ucd.version)).join(path.rsplit('/').next().unwrap_or(path))
}

fn fetch_ucd(root: &Path, ucd: &UcdPin) -> Result<(), String> {
    for (path, sha) in &ucd.files {
        let target = ucd_file(root, ucd, path);
        if std::fs::read(&target).is_ok_and(|b| crate::sha256::hex(&b) == *sha) {
            continue;
        }
        std::fs::create_dir_all(target.parent().ok_or("no parent")?).map_err(|e| e.to_string())?;
        let url = format!("{}{path}", ucd.url);
        eprintln!("fetching {url}…");
        let partial = target.with_extension("part");
        let status = Command::new("curl").args(["-fsSL", "-o"]).arg(&partial).arg(&url).status().map_err(|e| format!("curl: {e}"))?;
        if !status.success() {
            return Err(format!("curl {url}: {status}"));
        }
        let bytes = std::fs::read(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
        let got = crate::sha256::hex(&bytes);
        if got != *sha {
            return Err(format!("{url}: SHA-256 {got}, pinned {sha}"));
        }
        std::fs::rename(&partial, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e} (run `cargo xtask glyphdata --fetch`)", path.display()))
}

/// The parsed sources, ready for the builder.
pub fn sources(root: &Path, pins: &Pins) -> Result<fontcraft_glyphdata::build::Sources, String> {
    let d = dir(root);
    for (name, pin) in [("glyphsinfo", &pins.glyphsinfo), ("agl-aglfn", &pins.agl)] {
        let head = git(&d.join(name), &["rev-parse", "HEAD"]).unwrap_or_default();
        if head != pin.commit {
            return Err(format!("corpus/glyphdata/{name} is not at the pinned commit (run `cargo xtask glyphdata --fetch`)"));
        }
    }
    for (path, sha) in &pins.ucd.files {
        let p = ucd_file(root, &pins.ucd, path);
        let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e} (run `cargo xtask glyphdata --fetch`)", p.display()))?;
        if crate::sha256::hex(&bytes) != *sha {
            return Err(format!("{} does not match its pin (run `cargo xtask glyphdata --fetch`)", p.display()));
        }
    }
    let xml = |file: &str| -> Result<Vec<fontcraft_glyphdata::Record>, String> {
        let p = d.join("glyphsinfo").join(file);
        let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let parsed = fontcraft_glyphdata::xml::parse(&bytes).map_err(|e| format!("{file}: {e}"))?;
        if let Some(w) = parsed.warnings.first() {
            return Err(format!("{file}: {w}"));
        }
        Ok(parsed.records)
    };
    let ucd = |path: &str| read(&ucd_file(root, &pins.ucd, path));
    Ok(fontcraft_glyphdata::build::Sources {
        meta: vec![
            ("glyphsinfo".into(), format!("{} {}", pins.glyphsinfo.repo, pins.glyphsinfo.commit)),
            ("agl".into(), format!("{} {}", pins.agl.repo, pins.agl.commit)),
            ("unicode".into(), pins.ucd.version.clone()),
        ],
        main: xml("GlyphData.xml")?,
        ideographs: xml("GlyphData_Ideographs.xml")?,
        agl: read(&d.join("agl-aglfn/glyphlist.txt"))?,
        aglfn: read(&d.join("agl-aglfn/aglfn.txt"))?,
        unicode_data: ucd("UnicodeData.txt")?,
        bidi: ucd("extracted/DerivedBidiClass.txt")?,
        scripts: ucd("Scripts.txt")?,
    })
}

/// The licence notices that travel with the data: GlyphsInfo's LICENSE with the AGL notice of its
/// README, and the AGL's own header notice.
fn notices(root: &Path, pins: &Pins) -> Result<Vec<(&'static str, String)>, String> {
    let d = dir(root);
    let licence = read(&d.join("glyphsinfo/LICENSE"))?;
    let readme = read(&d.join("glyphsinfo/README.md"))?;
    let agl_notice = readme.split_once("## Notices").map(|(_, n)| n.trim()).ok_or("GlyphsInfo README.md: no `## Notices` section")?;
    let glyphsinfo = format!(
        "GlyphsInfo ({}), GlyphData.xml and GlyphData_Ideographs.xml.\n\n{}\n\nNotices from its README.md:\n\n{}\n",
        pins.glyphsinfo.repo,
        licence.trim(),
        agl_notice
    );
    let glyphlist = read(&d.join("agl-aglfn/glyphlist.txt"))?;
    let header: Vec<&str> = glyphlist
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with("# ----"))
        .map(|l| l.strip_prefix("# ").or_else(|| l.strip_prefix('#')).unwrap_or(l))
        .collect();
    let agl = format!("AGL and AGLFN ({}), glyphlist.txt and aglfn.txt.\n\n{}\n", pins.agl.repo, header.join("\n"));
    Ok(vec![("LICENSE-GlyphsInfo.txt", glyphsinfo), ("LICENSE-AGL.txt", agl)])
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let pins = pins(root)?;
    let mut did = false;
    if args.contains(&"--fetch") {
        fetch_git(&dir(root).join("glyphsinfo"), &pins.glyphsinfo)?;
        fetch_git(&dir(root).join("agl-aglfn"), &pins.agl)?;
        fetch_ucd(root, &pins.ucd)?;
        println!("glyph data sources are in {}", dir(root).display());
        did = true;
    }
    if args.contains(&"--generate") {
        let built = fontcraft_glyphdata::build::build(&sources(root, &pins)?)?;
        let data = root.join("crates/glyphdata/data");
        std::fs::write(data.join("glyphdata.bin"), &built.blob).map_err(|e| format!("glyphdata.bin: {e}"))?;
        for (file, text) in notices(root, &pins)? {
            std::fs::write(data.join(file), text).map_err(|e| format!("{file}: {e}"))?;
        }
        for (what, n) in &built.stats {
            println!("  {what:<28} {n}");
        }
        for note in &built.notes {
            println!("  note: {note}");
        }
        println!("wrote crates/glyphdata/data/glyphdata.bin ({} bytes)", built.blob.len());
        did = true;
    }
    if args.contains(&"--check") {
        crate::glyphdata_check::run(root, args)?;
        did = true;
    }
    if !did {
        return Err("usage: cargo xtask glyphdata [--fetch] [--generate] [--check]".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_pins_are_well_formed() {
        let p = super::pins(&crate::root()).unwrap();
        assert_eq!(p.ucd.files.len(), 3);
    }
}
