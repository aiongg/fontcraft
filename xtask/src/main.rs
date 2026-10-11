//! Workspace tooling: `cargo xtask <command>`.
//!
//! Rust only. External tools (`cargo`, `git`) are run through `std::process::Command`.

mod assets;
mod audit;
mod brands;
mod codecs;
mod corpus;
mod glyphdata;
mod glyphdata_check;
mod glyphsets;
mod guesses;
mod layers;
mod observe;
mod overlap;
mod parity;
mod roundtrip;
mod sha256;
mod spec;
mod testfonts;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci              the gate: fmt, clippy -D warnings, test, spec, parity --check, guesses --check,
                  observe --check, assets, brands, layers, wasm
                  (stops at the first failure; run it before every commit)
  spec            check spec/*.toml and spec/inventory/*.toml against the rules in spec/README.md
  parity [--check]
                  compare the inventory with the live command registry and write docs/parity.md
                  (--check: fail if the file is out of date)
  guesses [--check]
                  list every row's `guesses` in docs/guesses.md; fail when a live command whose status
                  is inferred or memory records none (--check: fail if the file is out of date)
  observe [--check]
                  list every row the running reference app could settle (guesses, memory and inferred
                  rows, `observe` steps) in docs/observe.md (--check: fail if the file is out of date)
  assets          check that every asset has a row in ASSETS.md and that no font binary is in the repository
  brands          check that user-visible text names no other vendor's products
  layers          enforce the crate dependency layering (docs/architecture.md)
  wasm            cargo check --target wasm32-unknown-unknown for every crate that must run in the browser
  corpus [--fetch]
                  list the pinned real-file test corpora; --fetch downloads them into corpus/ (git-ignored)
  glyphdata [--fetch] [--generate] [--check [-v]]
                  --fetch downloads the pinned glyph database sources into corpus/glyphdata/ (git-ignored);
                  --generate rebuilds crates/glyphdata/data/glyphdata.bin; --check compares the database
                  with fontc's glyphs-reader and glyphsLib (known differences: crates/glyphdata/crosscheck/allow.toml)
  glyphsets [--fetch] [--generate]
                  --fetch downloads the pinned Unicode CLDR locale files and Google Fonts glyph sets into
                  data-sources/glyphsets/ (git-ignored); --generate rebuilds crates/glyphdata/data/sets.bin
  roundtrip [--mode plist|model|native] [--kind glyphs|glyphspackage|ufo|ufoz|designspace] [-v]
                  read every corpus source and write it back; count byte-identical results
                  (known differences with a reason: xtask/roundtrip-allow.toml)
  audit [--match TEXT]
                  run the outline audit (`outlines` profile) on every source of the corpus comparison set
  overlap [--out DIR] [--reference FILE] [--match TEXT] [--quadratic]
                  remove overlaps on every glyph layer of the corpus comparison set and measure it
                  (--quadratic: convert the input to quadratic curves first)
                  (inputs for crates/pathops/crosscheck/reference.py go to DIR/layers.jsonl)
  testfonts [--out DIR]
                  build the synthetic test fonts for the checks in the reference app and write them
                  as .glyphs files with a README (default DIR: plan/local/testfonts/)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => cmd_ci(),
        Some("spec") => spec::run(&root()),
        Some("parity") => parity::run(&root(), rest.contains(&"--check")),
        Some("guesses") => guesses::run(&root(), rest.contains(&"--check")),
        Some("observe") => observe::run(&root(), rest.contains(&"--check")),
        Some("assets") => assets::run(&root()),
        Some("brands") => brands::run(&root()),
        Some("layers") => cmd_layers(),
        Some("wasm") => cmd_wasm(),
        Some("corpus") => corpus::run(&root(), rest.contains(&"--fetch")),
        Some("glyphdata") => glyphdata::run(&root(), &rest),
        Some("glyphsets") => glyphsets::run(&root(), &rest),
        Some("roundtrip") => roundtrip::run(&root(), &rest),
        Some("audit") => audit::run(&root(), &rest),
        Some("overlap") => overlap::run(&root(), &rest),
        Some("testfonts") => testfonts::run(&root(), &rest),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (the parent of the xtask crate).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has a parent directory").to_path_buf()
}

/// Files in the repository that exist: tracked ones and untracked ones that are not ignored,
/// relative to `root`. Outside a git checkout (an unpacked archive), every file except build output.
pub fn repo_files(root: &Path) -> Result<Vec<String>, String> {
    // `-z`: names are separated by NUL and never quoted, so a file with an accent or a space in
    // its name is not lost to the checks.
    let git = Command::new("git").current_dir(root).args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).output();
    if let Ok(out) = git
        && out.status.success()
    {
        return Ok(String::from_utf8_lossy(&out.stdout).split('\0').filter(|l| !l.is_empty() && root.join(l).is_file()).map(str::to_owned).collect());
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let rel = path.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            if !(rel == ".git"
                || rel.starts_with("target")
                || rel == "corpus"
                || rel == "data-sources"
                || rel == "dist"
                || rel == "log"
                || rel == "plan/local")
            {
                walk(root, &path, files)?;
            }
        } else {
            files.push(rel);
        }
    }
    Ok(())
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to start: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (docs/architecture.md)\n");
    println!("{:<24} {:<12} workspace dependencies", "crate", "layer");
    for c in &crates {
        let deps: Vec<String> =
            c.deps.iter().filter(|d| d.workspace).map(|d| format!("{}{}", layers::short_name(&d.name), d.kind.suffix())).collect();
        println!("{:<24} {:<12} {}", c.name, layers::describe(layers::classify(&c.name)), deps.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("layers: {} crates, no violations", crates.len());
        Ok(())
    } else {
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

fn cmd_wasm() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    let set: Vec<String> = crates
        .into_iter()
        .filter(|c| matches!(layers::classify(&c.name), Some(layers::Class::Layer(_) | layers::Class::Standalone)))
        .map(|c| c.name)
        .collect();
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown"]);
    for pkg in &set {
        c.args(["-p", pkg]);
    }
    run(c, &format!("cargo check --target wasm32-unknown-unknown ({} crates)", set.len()))?;
    println!("wasm: {} crates build for the browser ({})", set.len(), set.iter().map(|n| layers::short_name(n)).collect::<Vec<_>>().join(", "));
    Ok(())
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let cargo_step = |args: &'static [&'static str]| -> Box<dyn Fn() -> Result<(), String>> {
        Box::new(move || {
            let mut c = cargo();
            c.args(args);
            run(c, &format!("cargo {}", args.join(" ")))
        })
    };
    let steps: Vec<Step> = vec![
        ("fmt", cargo_step(&["fmt", "--all", "--", "--check"])),
        ("clippy", cargo_step(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])),
        ("test", cargo_step(&["test", "--workspace"])),
        ("spec", Box::new(|| spec::run(&root()))),
        ("parity", Box::new(|| parity::run(&root(), true))),
        ("guesses", Box::new(|| guesses::run(&root(), true))),
        ("observe", Box::new(|| observe::run(&root(), true))),
        ("assets", Box::new(|| assets::run(&root()))),
        ("brands", Box::new(|| brands::run(&root()))),
        ("layers", Box::new(cmd_layers)),
        ("wasm", Box::new(cmd_wasm)),
    ];
    let mut done = Vec::new();
    for (name, step) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = step() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}
