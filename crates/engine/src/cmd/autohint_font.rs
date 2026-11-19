//! PostScript autohinting of a compiled font file (`autohint.ps.runFont`): the charstrings of its
//! `CFF ` table hinted by the autohinter, and the font written again with every other table as it
//! was (`fontcraft_tables::patch::autohint_cff`).
//!
//! Row: `autohint.ps.runFont`.
//!
//! Each glyph is hinted as a CFF export hints it (`fontcraft_compile::cff_hints::autohinted`),
//! on the outline its charstring draws, against the zones and stems of its own Private DICT read
//! as otfautohint reads them from a compiled font (`autohint_dict`). Override sets from fontinfo
//! text are read against each Private DICT's set and the font's vertical extent (the FontBBox),
//! as otfautohint reads them for a compiled font. As in otfautohint, every glyph is hinted again
//! by default and its old hints replaced.
//!
//! A whole-file command as `super::file_io` describes: the input never changes and the output is
//! written atomically; a refusal writes nothing.

use std::collections::HashSet;

use fontcraft_autohint::fontinfo::{self, FontInfo};
use fontcraft_autohint::{FdDict, MAX_SEGMENTS, Options};
use fontcraft_compile::cff_hints::{autohint_dict, autohinted};
use fontcraft_tables::TableOrder;
use fontcraft_tables::patch::{self, CffAutohintOptions};
use serde_json::{Value, json};

use super::file_io::{self, OutputRequest};
use super::tables_file::{load, write_font};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// Inserted between the input's name and its extension when no output is named.
const SUFFIX: &str = ".autohinted";
/// What the shared hint writer adds to a warning; here the glyph is left as it was.
const EXPORT_TAIL: &str = "; it was exported without hints";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "autohint.ps.runFont",
            "Autohint PostScript Font File",
            [],
            None,
            "{font: path, face?: number|name, glyphs?: [names] (default: every glyph), excludeGlyphs?: [names], hintAll?: bool (default true: every glyph is hinted again and its hints replaced; false leaves glyphs that have hints), flex?: bool (default true), hintSubstitution?: bool (default true), maxSegments?: number (a direction with more segments is left unhinted unless glyphs are named; default and at most 100), allowChanges?: bool (let the autohinter reorder contours and treat nearly straight curves as lines while it works; the outlines written are not changed), decimal?: bool (keep fractional stem positions; default rounds them), fontinfo?: text (override sets: FDDict and GlyphSet blocks), dryRun?: bool (hint and report without writing), output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} runs the PostScript autohinter on the charstrings of a compiled font's CFF table, each glyph against the zones and stems of its Private DICT, and writes the font with the hinted charstrings, subroutinised again, every other table unchanged (default output: FONT.autohinted.EXT). Glyphs drawn with seac, glyphs that draw nothing and glyphs the autohinter finds no stems in keep their charstrings; a variable font's CFF2 table is refused → {output, written, hinted: [{glyph, horizontal, vertical}], skipped: [{glyph, reason}], warnings, bytesBefore, bytesAfter}",
            always,
            run_font
        )
        .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
    ]
}

fn tables_err(e: fontcraft_tables::TablesError) -> EngineError {
    EngineError::Tables(e)
}

/// One Private DICT's set in the autohinter's form, with the override sets read against it.
type FontDictSets = Option<(FdDict, Option<FontInfo>)>;

fn run_font(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("autohint.ps.runFont", v)?;
    if p.opt_strings("overlapGlyphs")?.is_some_and(|g| !g.is_empty()) || matches!(p.opt_str("overlap")?, Some("all")) {
        return Err(p.bad("hinting a compiled font on its overlap-free outline is not built yet"));
    }
    let loaded = load(&p, "font", "face")?;
    let setup = patch::cff_hint_setup(&loaded.font).map_err(tables_err)?;

    let known: HashSet<&str> = setup.names.iter().map(String::as_str).collect();
    let named = p.opt_strings("glyphs")?;
    if let Some(missing) = named.iter().flatten().find(|n| !known.contains(**n)) {
        return Err(p.bad(format!("the font has no glyph `{missing}`")));
    }
    let selected: Option<HashSet<&str>> = named.as_ref().map(|n| n.iter().copied().collect());
    let excluded: HashSet<&str> = p.opt_strings("excludeGlyphs")?.unwrap_or_default().into_iter().collect();
    let select = |name: &str| selected.as_ref().is_none_or(|s| s.contains(name)) && !excluded.contains(name);

    let max_segments = p.opt_int("maxSegments", 1, MAX_SEGMENTS as i64)?.map_or(MAX_SEGMENTS, |n| usize::try_from(n).unwrap_or(MAX_SEGMENTS));
    let decimal = p.bool_or("decimal", false)?;
    let options = Options {
        no_flex: !p.bool_or("flex", true)?,
        no_hint_sub: !p.bool_or("hintSubstitution", true)?,
        max_segments,
        explicit_glyphs: named.is_some(),
        allow_changes: p.bool_or("allowChanges", false)?,
        round_coords: !decimal,
        ..Options::default()
    };
    let run_options = CffAutohintOptions { hint_all: p.bool_or("hintAll", true)?, round_stems: !decimal };
    let dry_run = p.bool_or("dryRun", false)?;

    let mut warnings: Vec<String> = Vec::new();
    let fontinfo_text = p.opt_str("fontinfo")?;
    let mut sets: Vec<FontDictSets> = Vec::with_capacity(setup.privates.len());
    for (fd, private) in setup.privates.iter().enumerate() {
        let name = if setup.privates.len() > 1 { format!("Font DICT {fd}") } else { "Private DICT".to_string() };
        let entry = match (autohint_dict(private, &name, &mut warnings), fontinfo_text) {
            (Some(base), Some(text)) => {
                let (min_y, max_y) = setup.y_range;
                let info = fontinfo::parse(text, &base, &setup.names, max_y, min_y).map_err(|e| p.bad(format!("`fontinfo`: {e}")))?;
                Some((base, Some(info)))
            }
            (Some(base), None) => Some((base, None)),
            (None, _) => None,
        };
        sets.push(entry);
    }

    let r = patch::autohint_cff(&loaded.font, &setup, &run_options, &select, &mut |g| {
        let Some(Some((base, info))) = sets.get(g.fd) else {
            return Err("the alignment zones and standard stems of its Private DICT cannot be used".to_string());
        };
        let dict = info.as_ref().and_then(|i| i.dict_for(g.name)).unwrap_or(base);
        let mut found = Vec::new();
        match autohinted(g.name, g.path, dict, &options, &mut found) {
            None => match found.first() {
                Some(why) => Err(why.replace(EXPORT_TAIL, "")),
                None => Ok(None),
            },
            hints => Ok(hints),
        }
    })
    .map_err(tables_err)?;

    let output = if dry_run {
        file_io::output_path(&loaded.path, &OutputRequest::from_params(&p)?, SUFFIX, None)?
    } else {
        write_font(&p, &loaded, &r.font, TableOrder::Keep, SUFFIX)?
    };
    let hinted: Vec<Value> = r.hinted.iter().map(|h| json!({"glyph": h.glyph, "horizontal": h.horizontal, "vertical": h.vertical})).collect();
    let skipped: Vec<Value> = r.skipped.iter().map(|s| json!({"glyph": s.glyph, "reason": s.reason})).collect();
    Ok(json!({
        "output": output,
        "written": !dry_run,
        "hinted": hinted,
        "skipped": skipped,
        "warnings": warnings,
        "bytesBefore": r.bytes_before,
        "bytesAfter": r.bytes_after,
    }))
}
