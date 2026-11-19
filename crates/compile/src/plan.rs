//! What a compile would produce, without building it (`font.compilePlan`).
//!
//! [`plan`] runs the stages that decide the font's shape (resolve, glyph data, preflight, glyph
//! order with a CID-keyed font's CID order, the names) and stops before outline preparation and the table builders. It fails where the build
//! would fail in those stages, so a plan that succeeds names the glyphs and tables the build
//! writes.

use std::borrow::Cow;

use fontcraft_model::{Font, PropertyValue, Resolved, Target};

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
    /// The feature code the build would compile
    /// ([`crate::layout::LayoutTables::assembled_code`]).
    pub features: Option<String>,
}

/// Plans the compile of `font` at `target`.
pub fn plan(font: &Font, target: &Target, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    font.check()?;
    let mut notes = Vec::new();
    let (source, prepared) = crate::sources_at(font, target, &mut notes)?;
    let mut plan = plan_stages(Some(&source), crate::drawing::resolve(&prepared, target)?, flavour, options)?;
    notes.append(&mut plan.warnings);
    plan.warnings = notes;
    Ok(plan)
}

/// [`plan`] on a resolved font, without kerning: the resolved font holds none.
pub fn plan_resolved(resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    plan_stages(None, resolved, flavour, options)
}

/// [`plan`] on `resolved`, which was resolved from `font` after [`crate::prepare_source`]; `font`
/// is the source as stored ([`crate::compile_resolved_from`]).
pub fn plan_resolved_from(font: &Font, resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    plan_stages(Some(font), resolved, flavour, options)
}

fn plan_stages(font: Option<&Font>, mut resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Plan, CompileError> {
    let mut warnings = std::mem::take(&mut resolved.notes);
    fill_glyph_data(&mut resolved, options, &mut warnings);
    info::expand_tokens(font, &mut resolved, &mut warnings);
    crate::export_params::without_subset_lists(&mut resolved, options.subset);
    let edits = crate::export_params::apply(font, &mut resolved, &mut warnings)?;
    let glyphs = preflight::run(std::mem::take(&mut resolved.glyphs), &mut warnings)?;
    let glyphs = order::final_order(glyphs, &resolved, &mut warnings)?;
    let glyphs = match font {
        Some(font) if flavour == Flavour::Cff => crate::cid::plan(font, &resolved.params, glyphs, &options.cid, &mut warnings)?.0,
        _ => glyphs,
    };
    preflight::check_cmap(&glyphs)?;
    let glyph_data = font.and_then(|_| crate::compile_glyph_data(options));
    let mut color_plan = crate::color::plan(font, &resolved, options);
    let (glyphs, _) = crate::color::append_layer_glyphs(font, &mut color_plan, &resolved, glyphs, &mut warnings)?;
    let layout = crate::layout_tables(font, glyph_data.as_deref(), &resolved, &glyphs, &edits, options, None, &mut warnings)?;
    let mut layout_tags: Vec<String> = layout.tables().iter().map(|(tag, _)| tag.to_string()).collect();
    if crate::math::plan(font, &resolved, &edits).is_some() {
        layout_tags.push("MATH".into());
    }
    if crate::extras::meta_table(&resolved.params, &mut Vec::new())?.is_some() {
        layout_tags.push("meta".into());
    }
    let final_names = if flavour == Flavour::Cff || glyphs.len() <= crate::MAX_POST_NAMES {
        names::final_names(&resolved, &glyphs, &mut warnings)
    } else {
        Vec::new()
    };
    let names = info::derive(&resolved, &options.style_name, &mut warnings);
    let style_name = if options.style_name.is_empty() { resolved.style_name.clone() } else { options.style_name.clone() };
    let glyph_order: Vec<String> = glyphs.into_iter().map(|g| g.name).collect();
    if flavour == Flavour::Cff && names.vertical {
        layout_tags.push("VORG".into());
    }
    if options.stub_dsig || options.project.as_ref().is_some_and(|p| p.stub_dsig) {
        layout_tags.push("DSIG".into());
    }
    if options.pass_through_tables
        && let Some(font) = font
    {
        layout_tags.extend(crate::extras::passed_through(font, &glyph_order));
    }
    Ok(Plan {
        glyph_order,
        final_names,
        tables: tables(flavour, names.vertical, &layout_tags, &color_plan.tables()),
        postscript_name: names.name(6),
        style_name,
        warnings,
        features: layout.code.clone(),
    })
}

/// The PostScript name (name ID 6) a compile of `resolved` alone writes, its names taken as they
/// stand; `None` when it writes none. [`postscript_name_from`] expands their text tokens.
pub fn postscript_name(resolved: &Resolved, options: &Options) -> Option<String> {
    postscript_name_from(None, resolved, options)
}

/// The PostScript name (name ID 6) a compile of `resolved` writes; `None` when it writes none.
/// `font` is the source `resolved` was resolved from: the text tokens of the names are expanded
/// in its context, as the compile expands them ([`with_expanded_names`]).
pub fn postscript_name_from(font: Option<&Font>, resolved: &Resolved, options: &Options) -> Option<String> {
    let named = with_expanded_names(font, resolved);
    info::derive(&named, &options.style_name, &mut Vec::new()).name(6)
}

/// The PostScript name an exported file of `resolved` is named by, its tokens expanded as in
/// [`postscript_name_from`]. A font with no family name takes `source` (the source file's name
/// without its extension) as the family here, so its files are not called `-Bold.ttf`; the
/// `name` table keeps the empty family.
pub fn file_stem(font: Option<&Font>, resolved: &Resolved, options: &Options, source: &str) -> Option<String> {
    let named = with_expanded_names(font, resolved);
    let family = if named.family_name.trim().is_empty() { source.trim() } else { named.family_name.as_str() };
    info::derive_with_family(&named, family, &options.style_name, &mut Vec::new()).name(6)
}

/// `resolved` with the text tokens (`{{{key}}}`) of its family and style names, properties and
/// text parameters expanded in the context of `font`, the source it was resolved from, as the
/// compile expands them. Borrowed when there is no source or nothing to expand. The copy holds
/// no glyphs: no name depends on them.
pub fn with_expanded_names<'a>(font: Option<&Font>, resolved: &'a Resolved) -> Cow<'a, Resolved> {
    use fontcraft_fontinfo::tokens::has_tokens;
    let in_properties = || {
        resolved.properties.0.iter().any(|p| match &p.value {
            PropertyValue::Single(v) => has_tokens(v),
            PropertyValue::Localized(values) => values.iter().any(|l| has_tokens(&l.value)),
        })
    };
    let in_params = || resolved.params.0.iter().any(|p| p.value.as_str().is_some_and(has_tokens));
    let tokens = has_tokens(&resolved.family_name) || has_tokens(&resolved.style_name) || in_properties() || in_params();
    if font.is_none() || !tokens {
        return Cow::Borrowed(resolved);
    }
    let mut named = Resolved {
        format: resolved.format,
        units_per_em: resolved.units_per_em,
        family_name: resolved.family_name.clone(),
        style_name: resolved.style_name.clone(),
        version_major: resolved.version_major,
        version_minor: resolved.version_minor,
        master: resolved.master.clone(),
        location: resolved.location.clone(),
        axes: resolved.axes.clone(),
        metrics: resolved.metrics,
        zones: resolved.zones.clone(),
        stems_h: resolved.stems_h.clone(),
        stems_v: resolved.stems_v.clone(),
        properties: resolved.properties.clone(),
        params: resolved.params.clone(),
        date: resolved.date.clone(),
        style: resolved.style.clone(),
        instance: resolved.instance,
        instance_classes: resolved.instance_classes,
        production_names: resolved.production_names,
        glyphs: Vec::new(),
        notes: Vec::new(),
    };
    info::expand_tokens(font, &mut named, &mut Vec::new());
    Cow::Owned(named)
}

/// The tables `build::assemble` writes for `flavour`, sorted by tag as the table directory is.
fn tables(flavour: Flavour, vertical: bool, layout: &[String], color: &[&'static str]) -> Vec<String> {
    let mut tags: Vec<&str> = vec!["OS/2", "cmap", "head", "hhea", "hmtx", "maxp", "name", "post"];
    match flavour {
        Flavour::TrueType => tags.extend(["glyf", "loca"]),
        Flavour::Cff => tags.push("CFF "),
    }
    if vertical {
        tags.extend(["vhea", "vmtx"]);
    }
    tags.extend(layout.iter().map(String::as_str));
    tags.extend_from_slice(color);
    tags.sort_unstable();
    tags.dedup();
    tags.into_iter().map(str::to_string).collect()
}
