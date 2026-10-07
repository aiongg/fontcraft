//! `cargo xtask corpus`: real-file test corpora, pinned by commit and never committed here.
//!
//! The pins are in `xtask/corpus.toml`. `--fetch` downloads each repository at exactly its pinned
//! commit into `corpus/<name>` (git-ignored). A git commit id names its content, so a fetched
//! corpus is verified by construction. Tests that use a corpus skip with a message when it is
//! absent; `docs/testing.md` explains which tests use which corpus.

use std::path::Path;
use std::process::Command;

use toml::Table;

pub struct Pin {
    pub name: String,
    pub repo: String,
    pub commit: String,
    pub licence: String,
    pub holds: String,
}

pub fn pins(root: &Path) -> Result<Vec<Pin>, String> {
    let path = root.join("xtask/corpus.toml");
    let table: Table =
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let rows = table.get("corpus").and_then(|v| v.as_array()).ok_or("xtask/corpus.toml: expected [[corpus]] rows")?;
    let mut out = Vec::new();
    for r in rows {
        let get = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let pin = Pin { name: get("name"), repo: get("repo"), commit: get("commit"), licence: get("licence"), holds: get("holds") };
        let hex = pin.commit.len() == 40 && pin.commit.chars().all(|c| c.is_ascii_hexdigit());
        let plain = !pin.name.is_empty() && pin.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !hex || !plain || !pin.repo.starts_with("https://") || pin.licence.is_empty() {
            return Err(format!("xtask/corpus.toml: `{}` needs a plain name, an https repo, a 40-digit commit and a licence", pin.name));
        }
        out.push(pin);
    }
    Ok(out)
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").current_dir(dir).args(args).output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

pub fn run(root: &Path, fetch: bool) -> Result<(), String> {
    let pins = pins(root)?;
    let corpus = root.join("corpus");
    println!("Test corpora live under {} (git-ignored, never committed).\n", corpus.display());
    for pin in &pins {
        let dir = corpus.join(&pin.name);
        let have = dir.is_dir().then(|| git(&dir, &["rev-parse", "HEAD"]).ok()).flatten();
        if fetch && have.as_deref() != Some(pin.commit.as_str()) {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            if !dir.join(".git").exists() {
                git(&dir, &["init", "-q"])?;
                git(&dir, &["remote", "add", "origin", &pin.repo])?;
            }
            eprintln!("fetching {} at {}…", pin.name, &pin.commit[..12]);
            git(&dir, &["fetch", "-q", "--depth", "1", "origin", &pin.commit])?;
            git(&dir, &["checkout", "-q", "--detach", "FETCH_HEAD"])?;
        }
        let state = match git(&dir, &["rev-parse", "HEAD"]).ok() {
            Some(head) if dir.is_dir() && head == pin.commit => "present",
            Some(_) if dir.is_dir() => "present at another commit (run with --fetch)",
            _ => "absent (run with --fetch)",
        };
        println!("  {:<12} {state}\n               {} @ {}\n               {} · {}\n", pin.name, pin.repo, &pin.commit[..12], pin.licence, pin.holds);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_pins_are_well_formed() {
        let pins = super::pins(&crate::root()).unwrap();
        assert!(pins.len() >= 3);
    }
}
