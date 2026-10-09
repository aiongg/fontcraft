//! What a compile would produce, without building it (`font.compilePlan`).
//!
//! [`plan`] runs the stages that decide the font's shape (resolve, preflight, glyph order, the
//! names) and stops before outline preparation and the table builders. It fails where the build
//! would fail in those stages, so a plan that succeeds names the glyphs and tables the build
//! writes.

use fontcraft_model::{Font, Resolved, Target};

use crate::{CompileError, Flavour, Options, info, order, preflight};

/// A compile, described.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Final glyph order (`.notdef` first), by source glyph name.
    pub glyph_order: Vec<String>,
    /// The tables the font file holds, by tag, in table-directory order.
    pub tables: Vec<String>,
    /// The PostScript name (name ID 6) the font gets.
    pub postscript_name: Option<String>,
    /// The style name written to the `name` table.
    pub style_name: String,
    /// Things the user should know that the build would also report.
    pub warnings: Vec<String>,
}

/// Plans the compile of `font` at `target`.
pub fn plan(font: &Font, target: &Target, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    font.check()?;
    plan_resolved(font.resolve(target)?, flavour, options)
}

/// [`plan`] on a resolved font.
pub fn plan_resolved(mut resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    let mut warnings = std::mem::take(&mut resolved.notes);
    let glyphs = preflight::run(std::mem::take(&mut resolved.glyphs), &mut warnings)?;
    let glyphs = order::final_order(glyphs, &resolved, &mut warnings)?;
    preflight::check_cmap(&glyphs)?;
    let names = info::derive(&resolved, &options.style_name, &mut warnings);
    let style_name = if options.style_name.is_empty() { resolved.style_name.clone() } else { options.style_name.clone() };
    Ok(Plan {
        glyph_order: glyphs.into_iter().map(|g| g.name).collect(),
        tables: tables(flavour, names.vertical),
        postscript_name: names.name(6),
        style_name,
        warnings,
    })
}

/// The PostScript name (name ID 6) a compile of `resolved` writes; `None` when it writes none.
pub fn postscript_name(resolved: &Resolved, options: &Options) -> Option<String> {
    info::derive(resolved, &options.style_name, &mut Vec::new()).name(6)
}

/// The tables `build::assemble` writes for `flavour`, sorted by tag as the table directory is.
fn tables(flavour: Flavour, vertical: bool) -> Vec<String> {
    let mut tags: Vec<&str> = vec!["OS/2", "cmap", "head", "hhea", "hmtx", "maxp", "name", "post"];
    match flavour {
        Flavour::TrueType => tags.extend(["glyf", "loca"]),
        Flavour::Cff => tags.push("CFF "),
    }
    if vertical {
        tags.extend(["vhea", "vmtx"]);
    }
    tags.sort_unstable();
    tags.into_iter().map(str::to_string).collect()
}
