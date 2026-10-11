//! What a compile would produce, without building it (`font.compilePlan`).
//!
//! [`plan`] runs the stages that decide the font's shape (resolve, glyph data, preflight, glyph
//! order, the names) and stops before outline preparation and the table builders. It fails where the build
//! would fail in those stages, so a plan that succeeds names the glyphs and tables the build
//! writes.

use fontcraft_model::{Font, Resolved, Target};

use crate::{CompileError, Flavour, Options, fill_glyph_data, info, names, order, preflight};

/// A compile, described.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Final glyph order (`.notdef` first), by source glyph name.
    pub glyph_order: Vec<String>,
    /// The name each glyph of [`Plan::glyph_order`] is written under, in the same order: its
    /// production name, made valid ([`names`]). Empty when the font is written without glyph
    /// names (`post` 3.0 with too many glyphs for names).
    pub final_names: Vec<String>,
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
    let mut notes = Vec::new();
    let font = crate::prepare_source(font, &mut notes);
    let mut plan = plan_stages(Some(&font), crate::drawing::resolve(&font, target)?, flavour, options)?;
    notes.append(&mut plan.warnings);
    plan.warnings = notes;
    Ok(plan)
}

/// [`plan`] on a resolved font, without kerning: the resolved font holds none.
pub fn plan_resolved(resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    plan_stages(None, resolved, flavour, options)
}

/// [`plan`] on `resolved`, which was resolved from `font`; `font` gives the kerning.
pub fn plan_resolved_from(font: &Font, resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    plan_stages(Some(font), resolved, flavour, options)
}

fn plan_stages(font: Option<&Font>, mut resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    let mut warnings = std::mem::take(&mut resolved.notes);
    fill_glyph_data(&mut resolved, options, &mut warnings);
    let glyphs = preflight::run(std::mem::take(&mut resolved.glyphs), &mut warnings)?;
    let glyphs = order::final_order(glyphs, &resolved, &mut warnings)?;
    preflight::check_cmap(&glyphs)?;
    let glyph_data = font.and_then(|_| crate::compile_glyph_data(options));
    let kern = match font {
        Some(font) => crate::kern::generate(font, &resolved, &glyphs, &mut warnings),
        None => None,
    };
    let layout = crate::marks::generate(font, glyph_data.as_deref(), &resolved, &glyphs, kern, &mut warnings);
    let final_names = if flavour == Flavour::Cff || glyphs.len() <= crate::MAX_POST_NAMES {
        names::final_names(&resolved, &glyphs, &mut warnings)
    } else {
        Vec::new()
    };
    let names = info::derive(&resolved, &options.style_name, &mut warnings);
    let style_name = if options.style_name.is_empty() { resolved.style_name.clone() } else { options.style_name.clone() };
    Ok(Plan {
        glyph_order: glyphs.into_iter().map(|g| g.name).collect(),
        final_names,
        tables: tables(flavour, names.vertical, layout.gpos.is_some(), layout.gdef.is_some()),
        postscript_name: names.name(6),
        style_name,
        warnings,
    })
}

/// The PostScript name (name ID 6) a compile of `resolved` writes; `None` when it writes none.
pub fn postscript_name(resolved: &Resolved, options: &Options) -> Option<String> {
    info::derive(resolved, &options.style_name, &mut Vec::new()).name(6)
}

/// The PostScript name an exported file of `resolved` is named by. A font with no family name
/// takes `source` (the source file's name without its extension) as the family here, so its
/// files are not called `-Bold.ttf`; the `name` table keeps the empty family.
pub fn file_stem(resolved: &Resolved, options: &Options, source: &str) -> Option<String> {
    let family = if resolved.family_name.trim().is_empty() { source.trim() } else { resolved.family_name.as_str() };
    info::derive_with_family(resolved, family, &options.style_name, &mut Vec::new()).name(6)
}

/// The tables `build::assemble` writes for `flavour`, sorted by tag as the table directory is.
fn tables(flavour: Flavour, vertical: bool, gpos: bool, gdef: bool) -> Vec<String> {
    let mut tags: Vec<&str> = vec!["OS/2", "cmap", "head", "hhea", "hmtx", "maxp", "name", "post"];
    match flavour {
        Flavour::TrueType => tags.extend(["glyf", "loca"]),
        Flavour::Cff => tags.push("CFF "),
    }
    if vertical {
        tags.extend(["vhea", "vmtx"]);
    }
    if gpos {
        tags.push("GPOS");
    }
    if gdef {
        tags.push("GDEF");
    }
    tags.sort_unstable();
    tags.into_iter().map(str::to_string).collect()
}
