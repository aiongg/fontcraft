//! PostScript autohinting of a compiled font file (`autohint.ps.runFont`): the charstrings of its
//! `CFF ` or `CFF2` table hinted by the autohinter, and the font written again with every other
//! table as it was (`fontcraft_tables::patch::autohint_cff` and `autohint_cff2`).
//!
//! Row: `autohint.ps.runFont`.
//!
//! In a `CFF ` table each glyph is hinted as a CFF export hints it
//! (`fontcraft_compile::cff_hints::autohinted`), on the outline its charstring draws, against the
//! zones and stems of its own Private DICT read as otfautohint reads them from a compiled font
//! (`autohint_dict`). In a `CFF2` table the masters of each glyph (the default location and the
//! peak of each region its blends use) are hinted together as a CFF2 export hints a glyph's
//! sources (`fontcraft_compile::var::cff2::autohint_masters`), each against its Private DICT
//! evaluated at that master, and the stems are written back as blends. Override sets from
//! fontinfo text are read against each Private DICT's set and the font's vertical extent (the
//! FontBBox, or in a `CFF2` font `head`'s box), as otfautohint reads them for a compiled font. As
//! in otfautohint, every glyph is hinted again by default and its old hints replaced. In a `CFF `
//! table a glyph the overlap options select is hinted against its outline with the overlaps
//! removed, while its charstring keeps the outline it has; the table has one master, so
//! `overlap: "auto"` selects only the glyphs in `overlapGlyphs`. A `CFF2` table's masters are
//! hinted on their own outlines, and `overlap: "all"` or `overlapGlyphs` is refused for it.
//!
//! A whole-file command as `super::file_io` describes: the input never changes and the output is
//! written atomically; a refusal writes nothing.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};

use fontcraft_autohint::fontinfo::{self, FontInfo};
use fontcraft_autohint::{FdDict, MAX_SEGMENTS, Options};
use fontcraft_compile::cff_hints::{autohint_dict, autohinted};
use fontcraft_compile::var::cff2::autohint_masters;
use fontcraft_tables::TableOrder;
use fontcraft_tables::patch::{self, CffAutohint, CffAutohintOptions};
use fontcraft_tables::sfnt::SfntFont;
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
            "{font: path, face?: number|name, glyphs?: [names] (default: every glyph), excludeGlyphs?: [names], hintAll?: bool (default true: every glyph is hinted again and its hints replaced; false leaves glyphs that have hints), flex?: bool (default true), hintSubstitution?: bool (default true), maxSegments?: number (a direction with more segments is left unhinted unless glyphs are named; default and at most 100), allowChanges?: bool (let the autohinter reorder contours and treat nearly straight curves as lines while it works; the outlines written are not changed), overlap?: \"auto\"|\"all\"|\"none\" (in a CFF table, hint against the outline with overlaps removed, the charstring keeping the outline it has: auto for the glyphs in overlapGlyphs; default auto; all is refused for a CFF2 table), overlapGlyphs?: [names] (refused for a CFF2 table), looseOverlapMapping?: bool (match the overlap-free outline to the original loosely), decimal?: bool (keep fractional stem positions; default rounds them), fontinfo?: text (override sets: FDDict and GlyphSet blocks), dryRun?: bool (hint and report without writing), output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} runs the PostScript autohinter on the charstrings of a compiled font's CFF or CFF2 table, each glyph against the zones and stems of its Private DICT, and writes the font with the hinted charstrings, subroutinised again, every other table unchanged (default output: FONT.autohinted.EXT). In a variable font's CFF2 table the masters of each glyph (the default location and the peak of each region its blends use) are hinted together, each against the Private DICT at that master, and the stems are written as blends; the outlines keep every blend, and flex is not written. Glyphs drawn with seac, glyphs that draw nothing, glyphs the autohinter finds no stems in and glyphs whose masters cannot be hinted together keep their charstrings, with the reason → {output, written, hinted: [{glyph, horizontal, vertical}], skipped: [{glyph, reason}], warnings, bytesBefore, bytesAfter}",
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
    let loaded = load(&p, "font", "face")?;
    // A font with both tables is read as a CFF font, as a renderer reads it.
    let setup = if loaded.font.get(*b"CFF ").is_none() && loaded.font.get(*b"CFF2").is_some() {
        Setup::Cff2(patch::cff2_hint_setup(&loaded.font).map_err(tables_err)?)
    } else {
        Setup::Cff(patch::cff_hint_setup(&loaded.font).map_err(tables_err)?)
    };
    let names = setup.names();

    let known: HashSet<&str> = names.iter().map(String::as_str).collect();
    let named = p.opt_strings("glyphs")?;
    if let Some(missing) = named.iter().flatten().find(|n| !known.contains(**n)) {
        return Err(p.bad(format!("the font has no glyph `{missing}`")));
    }
    let selected: Option<HashSet<&str>> = named.as_ref().map(|n| n.iter().copied().collect());
    let excluded: HashSet<&str> = p.opt_strings("excludeGlyphs")?.unwrap_or_default().into_iter().collect();
    let select = |name: &str| selected.as_ref().is_none_or(|s| s.contains(name)) && !excluded.contains(name);

    let max_segments = p.opt_int("maxSegments", 1, MAX_SEGMENTS as i64)?.map_or(MAX_SEGMENTS, |n| usize::try_from(n).unwrap_or(MAX_SEGMENTS));
    let decimal = p.bool_or("decimal", false)?;
    let overlap_forcing = match p.opt_str("overlap")? {
        None | Some("auto") => None,
        Some("all") => Some(true),
        Some("none") => Some(false),
        Some(other) => return Err(p.bad(format!("`overlap` is \"auto\", \"all\" or \"none\", not `{other}`"))),
    };
    let overlap_list: BTreeSet<String> = p.opt_strings("overlapGlyphs")?.unwrap_or_default().into_iter().map(str::to_string).collect();
    let options = Options {
        no_flex: !p.bool_or("flex", true)?,
        no_hint_sub: !p.bool_or("hintSubstitution", true)?,
        max_segments,
        explicit_glyphs: named.is_some(),
        allow_changes: p.bool_or("allowChanges", false)?,
        round_coords: !decimal,
        overlap_forcing,
        overlap_list,
        loose_overlap_mapping: p.bool_or("looseOverlapMapping", false)?,
        ..Options::default()
    };
    if matches!(setup, Setup::Cff2(_)) && (options.overlap_forcing == Some(true) || !options.overlap_list.is_empty()) {
        return Err(p.bad("hinting a CFF2 font on its overlap-free outline is not built yet"));
    }
    let run_options = CffAutohintOptions { hint_all: p.bool_or("hintAll", true)?, round_stems: !decimal };
    let dry_run = p.bool_or("dryRun", false)?;

    let mut warnings: Vec<String> = Vec::new();
    let run = Run { p: &p, options: &options, cff: &run_options, select: &select, fontinfo: p.opt_str("fontinfo")? };
    let r = match &setup {
        Setup::Cff(setup) => hint_cff(&run, &loaded.font, setup, &mut warnings)?,
        Setup::Cff2(setup) => hint_cff2(&run, &loaded.font, setup, &mut warnings)?,
    };

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

/// What the font's table gives the hinter.
enum Setup {
    Cff(patch::CffHintSetup),
    Cff2(patch::Cff2HintSetup),
}

impl Setup {
    fn names(&self) -> &[String] {
        match self {
            Setup::Cff(s) => &s.names,
            Setup::Cff2(s) => &s.names,
        }
    }
}

/// The parameters of one run, shared by both tables.
struct Run<'a, 'p> {
    p: &'a Params<'p>,
    options: &'a Options,
    cff: &'a CffAutohintOptions,
    select: &'a dyn Fn(&str) -> bool,
    fontinfo: Option<&'a str>,
}

/// One Private DICT's set (`base`, `None` when its zones and stems cannot be used) with the
/// override sets read against it.
fn with_overrides(base: Option<FdDict>, fontinfo: Option<&str>, names: &[String], y_range: (f64, f64)) -> std::result::Result<FontDictSets, String> {
    match (base, fontinfo) {
        (Some(base), Some(text)) => {
            let (min_y, max_y) = y_range;
            let info = fontinfo::parse(text, &base, names, max_y, min_y).map_err(|e| format!("`fontinfo`: {e}"))?;
            Ok(Some((base, Some(info))))
        }
        (Some(base), None) => Ok(Some((base, None))),
        (None, _) => Ok(None),
    }
}

/// The set in force for glyph `name`: its override set, else the Private DICT's.
fn dict_for<'s>(sets: &'s FontDictSets, name: &str) -> Option<&'s FdDict> {
    let (base, info) = sets.as_ref()?;
    Some(info.as_ref().and_then(|i| i.dict_for(name)).unwrap_or(base))
}

/// Appends the warnings of `notes` that `warnings` does not hold yet.
fn push_unique(warnings: &mut Vec<String>, notes: Vec<String>) {
    for n in notes {
        if !warnings.contains(&n) {
            warnings.push(n);
        }
    }
}

fn hint_cff(run: &Run<'_, '_>, font: &SfntFont, setup: &patch::CffHintSetup, warnings: &mut Vec<String>) -> Result<CffAutohint> {
    let mut sets: Vec<FontDictSets> = Vec::with_capacity(setup.privates.len());
    for (fd, private) in setup.privates.iter().enumerate() {
        let name = if setup.privates.len() > 1 { format!("Font DICT {fd}") } else { "Private DICT".to_string() };
        let base = autohint_dict(private, &name, warnings);
        sets.push(with_overrides(base, run.fontinfo, &setup.names, setup.y_range).map_err(|e| run.p.bad(e))?);
    }
    patch::autohint_cff(font, setup, run.cff, run.select, &mut |g| {
        let Some(dict) = sets.get(g.fd).and_then(|s| dict_for(s, g.name)) else {
            return Err("the alignment zones and standard stems of its Private DICT cannot be used".to_string());
        };
        let mut found = Vec::new();
        let hints = autohinted(g.name, g.path, dict, run.options, &mut found);
        // A failure leaves the glyph as it was; the other notes (overlaps that could not be
        // removed) go with the run's warnings.
        if let Some(why) = found.iter().find(|w| w.ends_with(EXPORT_TAIL)) {
            return Err(why.replace(EXPORT_TAIL, ""));
        }
        warnings.extend(found);
        Ok(hints)
    })
    .map_err(tables_err)
}

/// Hints the masters of each glyph of a `CFF2` table together. The sets of each Font DICT at the
/// masters of each region list are made once, when a glyph first needs them.
fn hint_cff2(run: &Run<'_, '_>, font: &SfntFont, setup: &patch::Cff2HintSetup, warnings: &mut Vec<String>) -> Result<CffAutohint> {
    let several = setup.privates.len() > 1;
    let dict_name = |fd: usize| if several { format!("Font DICT {fd}") } else { "Private DICT".to_string() };
    // Override sets that cannot be read are refused before anything is hinted.
    for (fd, private) in setup.privates.iter().enumerate() {
        let base = autohint_dict(private, &dict_name(fd), warnings);
        with_overrides(base, run.fontinfo, &setup.names, setup.y_range).map_err(|e| run.p.bad(e))?;
    }
    let mut notes: Vec<String> = Vec::new();
    let mut cache: HashMap<(usize, u16), Vec<FontDictSets>> = HashMap::new();
    let r = patch::autohint_cff2(font, setup, run.cff, run.select, &mut |g| {
        if let Entry::Vacant(slot) = cache.entry((g.fd, g.vsindex)) {
            let mut list = Vec::with_capacity(g.privates.len());
            for (m, private) in g.privates.iter().enumerate() {
                // The default location's warnings were given above.
                let mut dict_notes = Vec::new();
                let name = format!("{} at master {m}", dict_name(g.fd));
                let base = autohint_dict(private, &name, &mut dict_notes);
                if m > 0 {
                    notes.append(&mut dict_notes);
                }
                list.push(with_overrides(base, run.fontinfo, &setup.names, setup.y_range)?);
            }
            slot.insert(list);
        }
        let sets = cache.get(&(g.fd, g.vsindex)).map_or(&[][..], Vec::as_slice);
        let dicts: Vec<&FdDict> = sets
            .iter()
            .map(|s| dict_for(s, g.name))
            .collect::<Option<Vec<_>>>()
            .ok_or("the alignment zones and standard stems of its Private DICT at one of its masters cannot be used")?;
        autohint_masters(g.name, g.paths, &dicts, run.options).map_err(|e| e.replace(EXPORT_TAIL, ""))
    })
    .map_err(tables_err)?;
    push_unique(warnings, notes);
    Ok(r)
}
