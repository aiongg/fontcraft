//! `cargo xtask testfonts [--out DIR]`: the synthetic test fonts for the checks in the reference
//! app (each plan's § 8 "To check in the reference app", and `docs/observe.md` for the built
//! milestones), written as format version 3 `.glyphs` files with a README that lists them.
//!
//! Every font is built here with the model API, from simple geometric outlines; nothing comes
//! from an existing font or from the reference app. The output folder defaults to the git-ignored
//! `plan/local/testfonts/`.

mod draw;
mod early;
mod images;
mod late;
mod middle;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_engine::model::Font;

/// One generated item: a font (or other test material) with what the README says about it.
pub struct Built {
    /// Path relative to the output folder.
    pub file: String,
    /// The plan section whose checks use it.
    pub plan: &'static str,
    /// The name the plan gives it.
    pub label: &'static str,
    pub contents: String,
    pub checks: &'static str,
    /// What the font should hold but does not, and why.
    pub gaps: Vec<String>,
    pub font: Option<Font>,
    /// Extra files (images, scripts) relative to the output folder.
    pub files: Vec<(String, Vec<u8>)>,
}

impl Built {
    pub fn font(file: &str, plan: &'static str, label: &'static str, font: Font) -> Self {
        Built { file: file.into(), plan, label, contents: String::new(), checks: "", gaps: Vec::new(), font: Some(font), files: Vec::new() }
    }
    pub fn material(file: &str, plan: &'static str, label: &'static str) -> Self {
        Built { file: file.into(), plan, label, contents: String::new(), checks: "", gaps: Vec::new(), font: None, files: Vec::new() }
    }
    pub fn about(mut self, text: &str) -> Self {
        self.contents = text.into();
        self
    }
    pub fn checks(mut self, text: &'static str) -> Self {
        self.checks = text;
        self
    }
    pub fn gap(mut self, text: &str) -> Self {
        self.gaps.push(text.into());
        self
    }
    pub fn file(mut self, path: &str, bytes: Vec<u8>) -> Self {
        self.files.push((path.into(), bytes));
        self
    }
}

/// Every test font and its material, in README order.
pub fn all() -> Vec<Built> {
    let mut out = Vec::new();
    out.extend(early::fonts());
    out.extend(middle::fonts());
    out.extend(late::fonts());
    out
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut out_dir = root.join("plan/local/testfonts");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--out" => out_dir = PathBuf::from(it.next().ok_or("--out needs a folder")?),
            other => return Err(format!("testfonts: unknown argument `{other}`")),
        }
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let built = all();
    let mut total = 0usize;
    for b in &built {
        if let Some(font) = &b.font {
            let text = fontcraft_io_glyphs::write(font).map_err(|e| format!("{}: {e}", b.file))?;
            // The file must load again: a font this reader refuses would fail the check too.
            fontcraft_io_glyphs::read(&text).map_err(|e| format!("{}: reading it back: {e}", b.file))?;
            total += write(&out_dir, &b.file, text.as_bytes())?;
        }
        for (path, bytes) in &b.files {
            total += write(&out_dir, path, bytes)?;
        }
    }
    total += write(&out_dir, "README.md", readme(&built).as_bytes())?;
    println!("testfonts: {} items, {} bytes, in {}", built.len(), total, out_dir.display());
    Ok(())
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) -> Result<usize, String> {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(bytes.len())
}

fn readme(built: &[Built]) -> String {
    let mut s = String::from(
        "# Test fonts for the reference-app checks\n\n\
         Made by `cargo xtask testfonts`. Every font is built in code from simple geometric outlines \
         (rectangles, polygons, ovals of four curves); none comes from an existing font or from the \
         reference app. Each `.glyphs` file is format version 3, which the reference app opens.\n\n\
         Rerunning the command overwrites these files: save edited copies under another name.\n\n\
         ## Naming\n\n\
         Each file starts with its milestone: `m8-F1.glyphs` is the font M8 § 8 calls F1, and \
         `m9-F1.glyphs` the one M9 § 8 calls F1. An `observe` step of an M<n> row that names a font \
         by its plan label (\"F1\", \"T3\", \"font A\") means the file with that milestone's prefix. \
         `m35-general.glyphs` serves the rows of the built milestones M3 to M5 in `docs/observe.md`.\n\n\
         Outlines are placeholders: letters without a drawing of their own are rectangular rings sized \
         by script and case, and a suffixed variant (`.sc`, `.ss01`, `.alt`…) is the base drawing \
         scaled, moved or given a small square marker so that variants can be told apart.\n\n",
    );
    s.push_str("## Files\n\n| File | Plan | Font | Contents | Checks |\n|---|---|---|---|---|\n");
    for b in built {
        s.push_str(&format!("| `{}` | {} | {} | {} | {} |\n", b.file, b.plan, b.label, cell(&b.contents), cell(b.checks)));
    }
    s.push_str("\n## Gaps\n\nWhat a plan asks for that these files do not hold, and why.\n\n");
    let mut any = false;
    for b in built {
        for g in &b.gaps {
            s.push_str(&format!("- `{}`: {}\n", b.file, g));
            any = true;
        }
    }
    if !any {
        s.push_str("None.\n");
    }
    s
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// Sets a kerning value in the left-to-right table.
pub fn kern(font: &mut Font, master: &str, first: &str, second: &str, value: f64) {
    Arc::make_mut(&mut font.kerning).ltr.set(master, first, second, value);
}

/// Sets a glyph's kerning groups (`left`: the group used when the glyph is on the right of a
/// pair).
pub fn groups(font: &mut Font, names: &[&str], left: Option<&str>, right: Option<&str>) {
    for n in names {
        if let Ok(mut g) = font.glyph_mut(n) {
            if let Some(l) = left {
                g.kern_groups.left = Some(l.into());
            }
            if let Some(r) = right {
                g.kern_groups.right = Some(r.into());
            }
        }
    }
}

/// Sets a glyph's metrics keys: left, right and width (`None` leaves a key alone).
pub fn keys(font: &mut Font, name: &str, left: Option<&str>, right: Option<&str>, width: Option<&str>) {
    if let Ok(mut g) = font.glyph_mut(name) {
        let k = &mut g.info.metrics_keys;
        if let Some(v) = left {
            k.left = Some(v.into());
        }
        if let Some(v) = right {
            k.right = Some(v.into());
        }
        if let Some(v) = width {
            k.width = Some(v.into());
        }
    }
}

#[cfg(test)]
mod tests {
    /// Every generated font is written and read back by the `.glyphs` reader without error, with
    /// the same glyphs and masters.
    #[test]
    fn every_test_font_round_trips_through_the_glyphs_reader() {
        for b in super::all() {
            let Some(font) = &b.font else { continue };
            let text = fontcraft_io_glyphs::write(font).unwrap_or_else(|e| panic!("{}: write: {e}", b.file));
            let back = fontcraft_io_glyphs::read(&text).unwrap_or_else(|e| panic!("{}: read: {e}", b.file));
            let names = |f: &fontcraft_engine::model::Font| f.glyphs.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
            assert_eq!(names(&back), names(font), "{}", b.file);
            assert_eq!(back.masters.len(), font.masters.len(), "{}", b.file);
        }
    }
}
