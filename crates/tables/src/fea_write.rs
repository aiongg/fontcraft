//! Decompiling layout tables to feature code (M12.12; rows `features decompile`,
//! `layout.decompileFeatures`). The text compiled back by feaLib gives tables that normalise equal
//! to the original.
//!
//! Layout of the text (`docs/spec/20-cli.md` › features decompile):
//!
//! 1. `languagesystem` statements, `DFLT dflt` first.
//! 2. Named glyph classes: glyph sets used more than once, named from their first and last
//!    member (`@a_z`), with members in between added when two names would collide.
//! 3. `table GDEF` and `table BASE` blocks.
//! 4. Every lookup as a top-level `lookup` block, `GSUB` first, in lookup-list order, except that
//!    a lookup a contextual rule calls is written before the rule that calls it (the compiler
//!    resolves names as it reads). A lookup a feature uses is named
//!    `feature_script_language_index` after the first language system that uses it; the others
//!    `GSUB_index` or `GPOS_index`. Mark-attachment lookups are preceded by their `markClass`
//!    statements, one class name per lookup, subtable and class.
//! 5. One `feature` block per feature tag: for each script, `script`, the default language
//!    system's lookups, then each language with `exclude_dflt` unless it uses all of the default
//!    lookups.
//!
//! Rules are written so the compiler builds the same kind of subtable: glyph pairs as glyph
//! pairs, class pairs with bracketed classes, original subtable boundaries as `subtable;`.
//! Whatever has no feature syntax (variation indices, feature variations, most feature
//! parameters, lookups that call themselves) is written as a comment starting `# not written:`
//! and listed in [`Decompiled::unsupported`], so nothing is dropped silently.
//!
//! The rules, blocks and notes come from the one feature-code writer,
//! [`fontcraft_decompile::fea::write`], which also rebuilds the feature entries of a compiled font
//! when it is opened; this module lays its lines out as one file.
//!
//! Input is untrusted: every record, glyph and rule read is charged to a work budget
//! ([`crate::features::MAX_LAYOUT_WORK`]), the text is capped at
//! [`crate::limits::MAX_FEATURE_SOURCE`] bytes, and lookup nesting is followed at most
//! [`fontcraft_decompile::fea::MAX_NESTING`] deep.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fontcraft_decompile::fea::write::{FeaError, Line, Piece, ReadOptions, Writer};
use read_fonts::TableProvider;

use crate::features::{BASE, Budget, DFLT_LANG, GDEF, GPOS, GSUB, MAX_LAYOUT_WORK, Provider, glyph_names, read_err};
use crate::limits::MAX_FEATURE_SOURCE;
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What [`decompile`] writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecompileOptions {
    /// `GSUB`, `GPOS`, `GDEF`; empty decompiles every layout table the font has.
    pub tables: Vec<Tag>,
    /// Write glyph classes as named classes when a set of glyphs repeats.
    pub named_classes: bool,
    /// Only these features and the lookups they reach; empty writes every feature and lookup.
    pub features: Vec<Tag>,
    /// Write `useExtension` on lookups the font stores through Extension lookups.
    pub show_extensions: bool,
    /// Glyph names to write in place of the font's: font name → written name.
    pub rename: BTreeMap<String, String>,
}

/// The text and what it could not express.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decompiled {
    pub text: String,
    /// One sentence per construct without feature syntax; each is also a comment in `text`.
    pub unsupported: Vec<String>,
}

/// The font's layout as feature code.
pub fn decompile(font: &SfntFont, options: &DecompileOptions) -> Result<String> {
    decompile_report(font, options).map(|d| d.text)
}

/// The font's layout as feature code, with the list of constructs written as comments.
pub fn decompile_report(font: &SfntFont, options: &DecompileOptions) -> Result<Decompiled> {
    let all = [GSUB, GPOS, GDEF, BASE];
    for t in &options.tables {
        if !all.contains(t) {
            return Err(TablesError::BadOptions(format!("`{}` is not a layout table: give GSUB, GPOS, GDEF or BASE", fea_tag(*t))));
        }
    }
    let wanted = |t: Tag| (options.tables.is_empty() || options.tables.contains(&t)) && font.get(t).is_some();
    let provider = Provider(font);
    let mut budget = Budget::new();
    let raw = glyph_names(&provider, &mut budget)?;
    let raw: Vec<String> = raw.into_iter().map(|n| options.rename.get(&n).cloned().unwrap_or(n)).collect();
    // The writer reads GDEF for lookup flags even when the block is not written, and skips a
    // table it cannot read; a damaged GDEF, or a damaged BASE that is asked for, fails here.
    if font.get(GDEF).is_some() {
        provider.gdef().map_err(|e| read_err("GDEF", e))?;
    }
    if wanted(BASE) {
        provider.base().map_err(|e| read_err("BASE", e))?;
    }
    let tables: Vec<Tag> = all.into_iter().filter(|t| wanted(*t)).collect();
    let mut w = Writer::new(&raw);
    let read = ReadOptions { tables: &tables, exclude: &[], only: &options.features, set_names: false };
    let layout = w.read(&provider, &read).map_err(tables_error)?;

    // Notes and feature parameters sit one level in.
    let mut head = layout.head;
    let mut params = layout.params;
    for line in head.iter_mut().chain(params.values_mut().flatten()) {
        line.indent = 1;
    }

    // GDEF and BASE.
    let mut blocks: Vec<Line> = Vec::new();
    for block in [layout.gdef, layout.base] {
        if !block.is_empty() {
            blocks.extend(block);
            blocks.push(Line::text(0, String::new()));
        }
    }

    // Lookups, commenting out calls to lookups not yet defined.
    let mut body: Vec<Line> = Vec::new();
    let mut defined: HashSet<(Tag, u16)> = HashSet::new();
    for out in layout.lookups {
        body.extend(out.pre);
        let ext = if out.extension && options.show_extensions { " useExtension" } else { "" };
        body.push(Line::text(0, format!("lookup {}{ext} {{", out.name)));
        if let Some(flag) = out.flag {
            body.push(flag);
        }
        for mut line in out.body {
            if let Some(missing) = line.refs.iter().find(|r| **r == out.index || !defined.contains(&(out.table, **r))) {
                let name = w.lookup_name(out.table, *missing);
                let mut note = w.note(&format!("a rule of lookup {} calls lookup {name}, which cannot be defined before it", out.name));
                note.indent = 1;
                body.push(note);
                line.pieces.insert(0, Piece::Text("# ".into()));
                line.refs.clear();
            }
            body.push(line);
        }
        body.push(Line::text(0, format!("}} {};", out.name)));
        body.push(Line::text(0, String::new()));
        defined.insert((out.table, out.index));
    }

    // Feature blocks.
    for (tag, systems) in &layout.features {
        let name = fea_tag(*tag);
        body.push(Line::text(0, format!("feature {name} {{")));
        if let Some(lines) = params.remove(tag) {
            body.extend(lines);
        }
        let special = tag == b"aalt" || tag == b"size";
        if special {
            if tag == b"aalt" {
                body.push(Line::text(1, "# the compiler rebuilds this feature from the lookups it names".into()));
            }
            let mut seen = HashSet::new();
            for (_, ids) in systems.values() {
                for id in ids {
                    if seen.insert(*id) {
                        body.push(Line::text(1, format!("lookup {};", w.lookup_name(id.0, id.1))));
                    }
                }
            }
        } else {
            type LangEntry<'a> = (Tag, bool, &'a Vec<(Tag, u16)>);
            let mut scripts: BTreeMap<(bool, Tag), Vec<LangEntry<'_>>> = BTreeMap::new();
            for ((nd, script, _, lang), (required, ids)) in systems {
                scripts.entry((*nd, *script)).or_default().push((*lang, *required, ids));
            }
            for ((_, script), langs) in scripts {
                body.push(Line::text(1, format!("script {};", fea_tag(script))));
                let dflt: Vec<(Tag, u16)> = langs.iter().find(|(l, _, _)| *l == DFLT_LANG).map(|(_, _, ids)| (*ids).clone()).unwrap_or_default();
                for (lang, required, ids) in &langs {
                    let req = if *required { " required" } else { "" };
                    let list: Vec<(Tag, u16)> = if *lang == DFLT_LANG {
                        if *required {
                            body.push(Line::text(2, "language dflt required;".into()));
                        }
                        (*ids).clone()
                    } else if !dflt.is_empty() && dflt.iter().all(|d| ids.contains(d)) {
                        body.push(Line::text(2, format!("language {}{req};", fea_tag(*lang))));
                        ids.iter().filter(|i| !dflt.contains(*i)).copied().collect()
                    } else {
                        let ex = if dflt.is_empty() { "" } else { " exclude_dflt" };
                        body.push(Line::text(2, format!("language {}{ex}{req};", fea_tag(*lang))));
                        (*ids).clone()
                    };
                    for (t, li) in list {
                        body.push(Line::text(3, format!("lookup {};", w.lookup_name(t, li))));
                    }
                }
            }
        }
        body.push(Line::text(0, format!("}} {name};")));
        body.push(Line::text(0, String::new()));
    }
    // Features with parameters but no lookups (`size`).
    for (tag, lines) in params {
        let name = fea_tag(tag);
        body.push(Line::text(0, format!("feature {name} {{")));
        body.extend(lines);
        body.push(Line::text(0, format!("}} {name};")));
        body.push(Line::text(0, String::new()));
    }

    let mut lines: Vec<Line> = vec![Line::text(0, "# Feature code decompiled from the font's layout tables.".into()), Line::text(0, String::new())];
    for (_, script, _, lang) in &layout.systems {
        lines.push(Line::text(0, format!("languagesystem {} {};", fea_tag(*script), fea_tag(*lang))));
    }
    if !layout.systems.is_empty() {
        lines.push(Line::text(0, String::new()));
    }
    lines.extend(head);
    let rest: Vec<Line> = blocks.into_iter().chain(body).collect();

    // The named classes go between the language systems and the rest.
    let rendered = w.render(&[lines, rest], options.named_classes, MAX_FEATURE_SOURCE).map_err(tables_error)?;
    let mut texts = rendered.texts.into_iter();
    let mut text = texts.next().unwrap_or_default();
    for (name, members) in &rendered.classes {
        text.push_str(&format!("@{name} = [{members}];\n"));
    }
    if !rendered.classes.is_empty() {
        text.push('\n');
    }
    text.push_str(&texts.next().unwrap_or_default());
    if text.len() > MAX_FEATURE_SOURCE {
        return Err(TablesError::TooLarge { what: "the feature code", limit: MAX_FEATURE_SOURCE });
    }
    let mut unsupported = rendered.unsupported;
    let bad: BTreeSet<String> = rendered.bad_names.into_iter().collect();
    if !bad.is_empty() {
        let list: Vec<String> = bad.into_iter().take(20).collect();
        unsupported.push(format!("glyph names feature syntax cannot hold: {}", list.join(", ")));
    }
    Ok(Decompiled { text, unsupported })
}

fn tables_error(e: FeaError) -> TablesError {
    match e {
        FeaError::Invalid(m) => TablesError::Invalid(m),
        FeaError::TooMuchWork => TablesError::TooLarge { what: "the layout tables (units of work to read them)", limit: MAX_LAYOUT_WORK },
        FeaError::TooLong(limit) => TablesError::TooLarge { what: "the feature code", limit },
    }
}

/// A tag as feature code writes it: trailing spaces dropped (`TRK ` is `TRK`).
fn fea_tag(tag: Tag) -> String {
    crate::sfnt::tag_str(tag).trim_end().to_string()
}

#[cfg(test)]
mod tests;
