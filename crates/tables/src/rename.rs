//! Renaming the glyphs of a compiled font (M12.15; rows `glyphs rename`, `font.renameGlyphs`).
//! Oracle: `tx -gf`. Renaming a source is the engine's job (components, kerning, groups and
//! feature code follow there).
//!
//! Names live in two places in a compiled font: the charset of a `CFF ` table and the `post`
//! table. A CFF font gets its charset rewritten (strings are kept and new names appended), and
//! its `post` table too when it is version 2 or when [`RenameOptions::post_names`] asks for it.
//! Any other font gets a version 2 `post` table. The glyph order and every other table stay as
//! they are. CID-keyed fonts have no glyph names and are refused.

use std::collections::{HashMap, HashSet};

use crate::goadb::Goadb;
use crate::reorder::{cff, names};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// What [`rename_glyphs`] renames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameOptions {
    /// Old name → new name pairs.
    pub map: Vec<(String, String)>,
    /// A GOADB to rename by (source name → final name); applied before `map`.
    pub goadb: Option<Goadb>,
    /// Write `post` version 2 names even for a CFF font (which keeps names in its charset).
    pub post_names: bool,
}

/// The renamed font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameResult {
    pub font: SfntFont,
    /// How many glyphs changed name.
    pub renamed: usize,
    /// Names the options mention that the font does not have.
    pub missing: Vec<String>,
}

/// Longest name a `post` table can store.
const MAX_POST_NAME: usize = 255;

/// The new name of every glyph: the GOADB first, then the map. Names the options mention that
/// the font lacks are returned as missing; a GOADB usually lists more glyphs than one font has.
pub fn new_names(old: &[String], options: &RenameOptions) -> (Vec<String>, Vec<String>) {
    let mut missing = Vec::new();
    let present: HashSet<&str> = old.iter().map(String::as_str).collect();
    let mut names: Vec<String> = old.to_vec();
    if let Some(goadb) = &options.goadb {
        let by_source: HashMap<&str, &str> = goadb.entries.iter().map(|e| (e.source_name.as_str(), e.final_name.as_str())).collect();
        for entry in &goadb.entries {
            if !present.contains(entry.source_name.as_str()) {
                missing.push(entry.source_name.clone());
            }
        }
        for name in &mut names {
            if let Some(new) = by_source.get(name.as_str()) {
                *name = (*new).to_string();
            }
        }
    }
    if !options.map.is_empty() {
        let map: HashMap<&str, &str> = options.map.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let current: HashSet<String> = names.iter().cloned().collect();
        for (from, _) in &options.map {
            if !current.contains(from) {
                missing.push(from.clone());
            }
        }
        for name in &mut names {
            if let Some(new) = map.get(name.as_str()) {
                *name = (*new).to_string();
            }
        }
    }
    (names, missing)
}

/// A name `post` and CFF can store: printable ASCII without spaces, at most `max` bytes.
fn check_name(name: &str, max: usize) -> Result<()> {
    if name.is_empty() || name.len() > max || !name.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(TablesError::BadOptions(format!("`{name}` is not a usable glyph name (printable ASCII without spaces, 1 to {max} bytes)")));
    }
    Ok(())
}

/// Renames glyphs in `post` and, for CFF fonts, the charset.
pub fn rename_glyphs(font: SfntFont, options: &RenameOptions) -> Result<RenameResult> {
    let n = names::num_glyphs(&font)?;
    let old = names::glyph_names(&font)?;
    let (new, missing) = new_names(&old, options);
    let renamed = old.iter().zip(&new).filter(|(a, b)| a != b).count();
    if renamed == 0 {
        return Ok(RenameResult { font, renamed, missing });
    }
    let cff_bytes = font.get(*b"CFF ");
    let max = if cff_bytes.is_some() { cff::MAX_CFF_NAME } else { MAX_POST_NAME };
    let mut seen = HashSet::with_capacity(new.len());
    for (i, (a, b)) in old.iter().zip(&new).enumerate() {
        if a != b {
            if i == 0 && cff_bytes.is_some() {
                return Err(TablesError::BadOptions(format!("glyph 0 (`{a}`) of a CFF font cannot be renamed")));
            }
            check_name(b, max)?;
        }
        if !seen.insert(b.as_str()) {
            return Err(TablesError::BadOptions(format!("two glyphs would be named `{b}`")));
        }
    }

    let mut out = font.clone();
    let identity: Vec<u16> = (0..n).map(|g| g as u16).collect();
    let mut write_post = true;
    if let Some(bytes) = cff_bytes {
        let table = cff::parse(bytes, n)?;
        if table.is_cid() {
            return Err(TablesError::Unsupported("renaming the glyphs of a CID-keyed font".into()));
        }
        out.insert(*b"CFF ", table.rebuild(&identity, Some(&new))?);
        let post_has_names = font.get(*b"post").map(|p| names::post_names(p, n)).transpose()?.flatten().is_some();
        write_post = options.post_names || post_has_names;
    }
    if write_post {
        let post = font.get(*b"post").ok_or_else(|| TablesError::MissingTable("post".into()))?;
        out.insert(*b"post", names::write_post_v2(post, &new)?);
    }
    Ok(RenameResult { font: out, renamed, missing })
}
