//! `font.convert` (M12.30; `fontcraft_tables::convert`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! Compiled fonts (TrueType, OpenType CFF or CFF2, one face of a collection, WOFF, WOFF2), TTX
//! text, Type 1 fonts (PFA, PFB) and bare CFF tables are converted between outline flavours, to
//! bare CFF or CFF2 tables and to UFO, a CFF result can be made CID-keyed, and a result can be
//! wrapped in WOFF or WOFF2. A snapshot at a design location, a glyph subset (by name, glyph id,
//! CID or Font DICT), a GOADB and overlap removal run around the conversion
//! (`fontcraft_tables::convert::prepare`). A UFO is the font as `file.open` reads it, written by
//! the UFO writer, as a folder or a UFOZ; an SVG font is the same font written by the SVG font
//! writer, glyphs without a character given private-use ones. A result can have tables compiled
//! again (`rebuildTables`) or be written for speed (`optimizeFor`); `recalcBounds` false keeps
//! the bounds a TTX dump or a rebuilt table states. Each output is written to a temporary file
//! (a folder, for a UFO) beside its target as soon as its input is converted, so memory holds
//! one font at a time; only when all of them are there are they moved into place, so a failing
//! conversion writes nothing.

use std::path::{Path, PathBuf};

use std::collections::{BTreeSet, HashSet};

use fontcraft_io_svg::svg_font::{SvgFontOptions, SvgGlyphNames, write_svg_font_with};
use fontcraft_tables::convert::{
    CffWriteOptions, CharstringForm, CidCollection, ConvertOptions, ConvertTarget, Decid, FontDictFilter, GoadbUse, PostFormat, PrepareOptions,
    PsReadOptions, PsSource, QuadToCubic, RebuildTables, convert_postscript, convert_prepared, glyph_cids, postscript_name, prepare, read_postscript,
    read_postscript_with,
};
use fontcraft_tables::sfnt::parse_tag;
use fontcraft_tables::ttx::{ImportOptions, import_tree};
use fontcraft_tables::{Flavour, SfntFont, WriteOptions};
use serde_json::{Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most inputs one call converts.
const MAX_INPUTS: usize = 100_000;
/// Seconds between 1904-01-01 (the `head` epoch) and 1970-01-01.
const EPOCH_1904: i64 = 2_082_844_800;
/// Largest GOADB file read.
pub(super) const MAX_GOADB_BYTES: usize = 64 << 20;
/// Most axes a location names.
const MAX_LOCATION_AXES: usize = fontcraft_tables::limits::MAX_AXES;
/// Most Font DICTs a CFF table holds (an FDSelect index is one byte).
const MAX_FONT_DICTS: usize = 256;
/// Most items a glyph selector lists, the bound `font.dehint` and the other selectors use.
const MAX_SELECTOR_ITEMS: usize = 65_536;

/// Parameters of the row this version does not act on yet; naming one is refused, so a call
/// never silently does less than it asks.
const NOT_YET: [&str; 3] = ["type1", "validateSources", "smartComponents"];
/// Most tables `rebuildTables` names (an sfnt directory holds at most this many).
const MAX_REBUILD_TABLES: usize = 4_096;
/// The first and last code points given to glyphs without one in an SVG font.
const PRIVATE_USE: (u32, u32) = (0xE000, 0xF8FF);

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.convert",
            "Convert Font Format",
            [],
            None,
            "{inputs: [path] | input: path, to: \"ttf\"|\"otf\"|\"otf-cff2\"|\"cff\"|\"cff2\"|\"ufo\"|\"ufoz\"|\"svg-font\", output?: path (one input), outputDir?: path, inPlace?: bool, noClobber?: bool, face?: int|name, container?: \"plain\"|\"woff\"|\"woff2\", nameBy?: \"filename\"|\"postscript-name\", curves?: {maxError?: em fraction (default one font unit), conversion?: \"optimized\"|\"exact\", keepDirection?: bool}, postFormat?: 2|3, cff?: {subroutinize?: bool, maxSubrs?: int, charstringOptimize?: \"specialize\"|\"none\", optimizeWidths?: bool, keepGlyphOrder?: bool, flattenSeac?: bool, embedMinimal?: bool, dedupeFamilyBlues?: bool (default true), standardEncoding?: bool, allowMissingNotdef?: bool, keepUniqueId?: bool (default true), warnHints?: bool (default true)}, hints?: bool, cid?: {keyed?: bool|\"REGISTRY-ORDERING-SUPPLEMENT\", decid?: bool, fd?: int}, glyphs?: selector, excludeGlyphs?: selector, fdSelect?: [int], excludeFd?: [int], goadb?: path, goadbRename?: bool (default true), goadbSubset?: bool, goadbOrder?: bool, location?: {axis: number}, clamp?: bool (default true), removeOverlaps?: bool, ignoreOverlapErrors?: bool, timestamp?: \"keep\"|\"now\"|int (seconds since 1970), recalcBounds?: bool (default true), rebuildTables?: [tag]|\"all\", optimizeFor?: \"size\"|\"speed\", ufo?: {structure?: \"package\"|\"zip\", skipExportKey?: \"public\"|\"glyphs\"}, svg?: {glyphNames?: \"none\"|\"non-ascii\"|\"all\", absolute?: bool, standalone?: bool}, newline?: \"lf\"|\"cr\"|\"crlf\"}. Inputs: compiled fonts, TTX text (compiled first), Type 1 fonts (PFA, PFB) and bare CFF tables (wrapped in an OpenType CFF font first; bare tables are written without subroutines unless cff.subroutinize says otherwise). CFF or static CFF2 to TrueType (cubic to quadratic within maxError, contours reversed), TrueType to CFF or CFF2 (components decomposed, instructions dropped), CFF to CFF2 and back with hints kept, the bare outline table (cff, cff2), a UFO of the font as file.open reads it (a folder, or with ufo.structure zip or to ufoz a UFOZ archive; ufo.skipExportKey names where non-exporting glyphs would be listed, and a compiled font has none), or an SVG font of it (svg-font: the font element alone unless svg.standalone; svg.glyphNames chooses which glyphs carry glyph-name, default none; path data relative unless svg.absolute; glyphs without a character get private-use ones from U+E000 in glyph order; newline sets the line ending). curves, postFormat, cff, hints, timestamp, rebuildTables and optimizeFor shape compiled output only and are refused with ufo and svg-font. recalcBounds false keeps the bounding boxes, maxp maxima and hhea extremes a TTX input states and those of rebuilt tables. rebuildTables compiles the named tables of the result (all: every table) again from their parsed form instead of copying their bytes. optimizeFor speed writes CFF and CFF2 charstrings without subroutines and every gvar delta (cff.subroutinize and cff.maxSubrs are refused with it). A variable font converted to UFO or to another outline flavour is snapshotted at its default location unless location is given. cid.keyed makes a name-keyed CFF result CID-keyed, CID n for glyph n (true: the Identity ordering, supplement 0; or a REGISTRY-ORDERING-SUPPLEMENT string). cid.decid makes a CID-keyed CFF result name-keyed, glyphs named cid and their CID, with the Private DICT of the one Font DICT the glyphs come from, or of cid.fd for all. The cff keys shape a CFF or CFF2 result: charstringOptimize rewrites every charstring in the short operator forms or with rmoveto, rlineto and rrcurveto only; optimizeWidths chooses defaultWidthX and nominalWidthX again (false leaves both 0 in a table built from TrueType); flattenSeac draws accented seac glyphs out and drops dotsection; embedMinimal drops the descriptive Top DICT strings; dedupeFamilyBlues drops FamilyBlues and FamilyOtherBlues equal to BlueValues and OtherBlues; standardEncoding drops a custom Encoding; keepUniqueId false drops UniqueID and XUID; warnHints reports glyphs drawn without stem hints and repeated hint masks; keepGlyphOrder keeps a Type 1 font's glyph order (.notdef first) instead of the standard-string order; allowMissingNotdef lets the first glyph of a Type 1 font without .notdef take glyph 0. nameBy postscript-name names each output after the font's PostScript name. Before converting: location snapshots a variable font (axes not named at their default; clamped to the axis ranges unless clamp is false), glyphs and excludeGlyphs keep a subset (names, glyph ids, ranges such as 3-9, U+0041, and in a CID-keyed font /CID or /CID-CID; .notdef always stays), fdSelect and excludeFd keep the glyphs of some Font DICTs of a CID-keyed CFF or a CFF2 font (indexes from 0), goadb renames glyphs from its second column to its first (goadbRename), keeps only the glyphs it lists (goadbSubset) and orders them as it does (goadbOrder); removeOverlaps merges overlapping contours (TrueType outlines before converting a TrueType font or after converting to TrueType; CFF and CFF2 outlines before converting, a merged glyph losing its stem hints), and ignoreOverlapErrors keeps a glyph that cannot be merged. Without output the result goes beside the input with the new extension → {results: [{input, output, bytes, glyphs, leftBehind: [text], warnings: [text]}]}",
            always,
            convert_fonts
        )
        .files(const {
            &[FileParam::read("input"), FileParam::read("inputs"), FileParam::read("goadb"), FileParam::write("output"), FileParam::write("outputDir")]
        })
        .writes_files(),
    ]
}

/// How a result is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Plain,
    Woff,
    Woff2,
}

/// One converted input.
struct Converted {
    output: Output,
    glyphs: usize,
    left_behind: Vec<String>,
    warnings: Vec<String>,
}

/// What one input becomes.
enum Output {
    File(Vec<u8>),
    /// A UFO: its files by relative path.
    Folder(fontcraft_io_ufo::Files),
}

impl Output {
    fn len(&self) -> usize {
        match self {
            Output::File(bytes) => bytes.len(),
            Output::Folder(files) => files.values().map(Vec::len).sum(),
        }
    }
}

fn convert_fonts(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.convert", v)?;
    for key in NOT_YET {
        if p.get(key).is_some() {
            return Err(EngineError::Unsupported(format!("`{key}` in font.convert")));
        }
    }
    let name_by_font = match p.opt_str("nameBy")? {
        None | Some("filename") => false,
        Some("postscript-name") => true,
        Some(other) => return Err(p.bad(format!("`nameBy` must be filename or postscript-name, not `{other}`"))),
    };
    let inputs: Vec<&str> = match (p.opt_strings("inputs")?, p.opt_str("input")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `inputs` or `input`, not both")),
        (Some(list), None) => list,
        (None, Some(one)) => vec![one],
        (None, None) => return Err(p.bad("`inputs` is required")),
    };
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(p.bad(format!("give 1 to {MAX_INPUTS} inputs")));
    }
    if inputs.len() > 1 && p.get("output").is_some() {
        return Err(p.bad("`output` names one file: give `outputDir` or `inPlace` for several inputs"));
    }
    let to = p.str("to")?;
    let container = match p.opt_str("container")?.unwrap_or("plain") {
        "plain" => Container::Plain,
        "woff" => Container::Woff,
        "woff2" => Container::Woff2,
        other => return Err(p.bad(format!("`container` must be plain, woff or woff2, not `{other}`"))),
    };
    let request = OutputRequest::from_params(&p)?;
    let settings = Settings::read(&p, to)?;
    if container != Container::Plain && matches!(to, "cff" | "cff2" | "ufo" | "ufoz" | "svg-font") {
        return Err(p.bad("a bare CFF or CFF2 table, a UFO or an SVG font cannot be wrapped in WOFF"));
    }
    if matches!(to, "ufo" | "ufoz" | "svg-font") && request.in_place {
        return Err(p.bad("`inPlace` cannot replace a font with a UFO or an SVG font"));
    }

    let mut outputs = Vec::with_capacity(inputs.len());
    for input in &inputs {
        let path = Path::new(input);
        let output = if name_by_font && request.output.is_none() && !request.in_place {
            // The default name comes from the font's PostScript name, as `tx -A` names it.
            let name = font_file_name(&p, path).map_err(|e| in_input(e, input))?;
            let named = path.with_file_name(format!("{name}.font"));
            let out = file_io::output_path(&named, &request, "", Some(settings.extension(container))).map_err(|e| in_input(e, input))?;
            if out == path {
                return Err(in_input(
                    EngineError::Io(format!("the output {} would replace the input: give output or outputDir", out.display())),
                    input,
                ));
            }
            out
        } else {
            file_io::output_path(path, &request, "", Some(settings.extension(container))).map_err(|e| in_input(e, input))?
        };
        outputs.push(output);
    }
    // Two inputs written to one file, or an output that replaces another input, would lose a
    // result: refused before anything is written.
    let resolved = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let sources: Vec<PathBuf> = inputs.iter().map(|i| resolved(Path::new(i))).collect();
    let mut targets: Vec<PathBuf> = Vec::with_capacity(outputs.len());
    for (input, output) in inputs.iter().zip(&outputs) {
        let target = resolved(output);
        if targets.contains(&target) {
            return Err(p.bad(format!("two inputs would both be written to {}", output.display())));
        }
        if sources.contains(&target) && resolved(Path::new(input)) != target {
            return Err(p.bad(format!("the output {} would replace another input", output.display())));
        }
        targets.push(target);
    }
    for output in &outputs {
        file_io::check_write(output)?;
        if request.no_clobber && !request.in_place && output.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", output.display())));
        }
    }
    let mut staging = Staging::open(&request)?;
    let mut results = Vec::with_capacity(inputs.len());
    for (n, (input, output)) in inputs.iter().zip(&outputs).enumerate() {
        let mut c = convert_one(&p, Path::new(input), &settings, container).map_err(|e| in_input(e, input))?;
        if settings.ufo_zip
            && let Output::Folder(files) = &c.output
        {
            // The archive's one folder is named after the output, as UFO writers name it.
            let stem = output.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "font".into());
            c.output = Output::File(fontcraft_io_ufo::zip::write_ufoz(files, &format!("{stem}.ufo"))?);
        }
        staging.stage(output, &c.output, n)?;
        results.push(json!({
            "input": input,
            "output": output.display().to_string(),
            "bytes": c.output.len(),
            "glyphs": c.glyphs,
            "leftBehind": c.left_behind,
            "warnings": c.warnings,
        }));
    }
    staging.commit()?;
    Ok(json!({ "results": results }))
}

fn in_input(e: EngineError, input: &str) -> EngineError {
    match e {
        EngineError::Io(m) => EngineError::Io(m),
        EngineError::Unsupported(m) => EngineError::Unsupported(format!("{input}: {m}")),
        other => other,
    }
}

/// The conversion parameters, read once for every input.
struct Settings {
    to: &'static str,
    /// `None`: one font unit, the reference converters' bound.
    max_error_em: Option<f64>,
    quad_to_cubic: QuadToCubic,
    reverse_direction: bool,
    post_format: PostFormat,
    subroutinize: Option<bool>,
    max_subrs: Option<usize>,
    hints: bool,
    timestamp: Timestamp,
    /// `cid.keyed`: the character collection of a CID-keyed result.
    cid_keyed: Option<CidCollection>,
    /// `cid.decid` and `cid.fd`.
    decid: Option<Decid>,
    /// The CFF-writing keys of `cff`.
    cff_write: CffWriteOptions,
    /// `cff.flattenSeac`.
    flatten_seac: bool,
    /// `cff.keepGlyphOrder` and `cff.allowMissingNotdef`, for Type 1 inputs.
    ps_read: PsReadOptions,
    /// The steps around the conversion; the glyph selection is resolved for each input.
    prepare: PrepareOptions,
    /// `recalcBounds` false: TTX inputs and rebuilt tables keep the bounds they state.
    keep_bounds: bool,
    rebuild_tables: RebuildTables,
    /// `optimizeFor: speed`.
    optimize_for_speed: bool,
    /// `ufo.structure: zip`: a UFOZ archive instead of a folder.
    ufo_zip: bool,
    /// The `svg` keys.
    svg: SvgFontOptions,
    /// The line ending of an SVG font.
    newline: &'static str,
}

enum Timestamp {
    Keep,
    Now,
    At(i64),
}

impl Settings {
    fn read(p: &Params<'_>, to: &str) -> Result<Self> {
        // `ufoz` is a UFO written as a UFOZ.
        let zip_target = to == "ufoz";
        let to = if zip_target { "ufo" } else { to };
        let to: &'static str = match to {
            "ttf" => "ttf",
            "otf" => "otf",
            "otf-cff2" => "otf-cff2",
            "cff" => "cff",
            "cff2" => "cff2",
            "ufo" => "ufo",
            "ttx" => return Err(EngineError::Unsupported("`to: ttx`: write TTX with font.dumpTables".into())),
            "pfa" | "pfb" | "t1-text" => return Err(EngineError::Unsupported(format!("`to: {to}`: Type 1 fonts are read, not written"))),
            "designspace" | "glyphs" | "native" => {
                return Err(EngineError::Unsupported(format!("`to: {to}` from a compiled font: open it (file.open) and save it with file.saveAs")));
            }
            "svg-font" => "svg-font",
            other => return Err(p.bad(format!("`to` must be ttf, otf, otf-cff2, cff, cff2, ufo or svg-font, not `{other}`"))),
        };
        // A UFO or SVG font holds the outlines as drawn: these keys shape a compiled result only.
        if matches!(to, "ufo" | "svg-font") {
            for key in ["curves", "postFormat", "cff", "hints", "timestamp", "rebuildTables", "optimizeFor"] {
                if p.get(key).is_some() {
                    return Err(p.bad(format!("`{key}` has no effect on a UFO or an SVG font")));
                }
            }
        }
        let curves = p.opt_object("curves")?;
        let curve = |key: &str| curves.and_then(|c| c.get(key));
        for key in curves.map(|c| c.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            if !matches!(key.as_str(), "maxError" | "conversion" | "keepDirection") {
                return Err(p.bad(format!("`curves.{key}` is not a curve setting")));
            }
        }
        let max_error_em = match curve("maxError") {
            None => None,
            Some(v) => Some(
                v.as_f64()
                    .filter(|e| e.is_finite() && *e > 0.0 && *e <= 1.0)
                    .ok_or_else(|| p.bad("`curves.maxError` must be above 0 and at most 1 (em)"))?,
            ),
        };
        let quad_to_cubic = match curve("conversion").map(|v| v.as_str()) {
            None | Some(Some("optimized")) => QuadToCubic::Optimized,
            Some(Some("exact")) => QuadToCubic::Exact,
            _ => return Err(p.bad("`curves.conversion` must be optimized or exact")),
        };
        let keep_direction = match curve("keepDirection") {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| p.bad("`curves.keepDirection` must be true or false"))?,
        };
        let post_format = match p.opt_int("postFormat", 2, 3)? {
            None | Some(2) => PostFormat::Names,
            Some(_) => PostFormat::NoNames,
        };
        let cff = p.opt_object("cff")?;
        let mut subroutinize = None;
        let mut max_subrs = None;
        let mut cff_write = CffWriteOptions::default();
        let mut flatten_seac = false;
        let mut ps_read = PsReadOptions::default();
        if let Some(cff) = cff {
            for (key, value) in cff {
                let flag = || value.as_bool().ok_or_else(|| p.bad(format!("`cff.{key}` must be true or false")));
                match key.as_str() {
                    "subroutinize" => subroutinize = Some(flag()?),
                    "maxSubrs" => {
                        let n = value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad("`cff.maxSubrs` must be a whole number"))?;
                        max_subrs = Some(n);
                    }
                    "charstringOptimize" => {
                        cff_write.charstrings = match value.as_str() {
                            Some("specialize") => Some(CharstringForm::Specialize),
                            Some("none") => Some(CharstringForm::General),
                            _ => return Err(p.bad("`cff.charstringOptimize` must be specialize or none")),
                        }
                    }
                    "optimizeWidths" => cff_write.optimize_widths = Some(flag()?),
                    "embedMinimal" => cff_write.embed_minimal = flag()?,
                    "dedupeFamilyBlues" => cff_write.dedupe_family_blues = flag()?,
                    "standardEncoding" => cff_write.standard_encoding = flag()?,
                    "keepUniqueId" => cff_write.keep_unique_id = flag()?,
                    "warnHints" => cff_write.warn_hints = flag()?,
                    "flattenSeac" => flatten_seac = flag()?,
                    "keepGlyphOrder" => ps_read.keep_glyph_order = flag()?,
                    "allowMissingNotdef" => ps_read.allow_missing_notdef = flag()?,
                    other => return Err(p.bad(format!("`cff.{other}` is not a CFF setting"))),
                }
            }
        }
        if !matches!(to, "otf" | "otf-cff2" | "cff" | "cff2") {
            let cff_only = [
                "charstringOptimize",
                "optimizeWidths",
                "embedMinimal",
                "dedupeFamilyBlues",
                "standardEncoding",
                "keepUniqueId",
                "warnHints",
                "flattenSeac",
            ];
            if let Some(key) = cff_only.iter().find(|k| cff.is_some_and(|c| c.contains_key(**k))) {
                return Err(p.bad(format!("`cff.{key}` shapes CFF and CFF2 output (otf, otf-cff2, cff, cff2)")));
            }
        }
        if matches!(to, "otf-cff2" | "cff2") {
            for key in ["embedMinimal", "standardEncoding", "keepUniqueId", "flattenSeac"] {
                if cff.is_some_and(|c| c.contains_key(key)) {
                    return Err(p.bad(format!("`cff.{key}` shapes CFF output (otf, cff); CFF2 has no such entries")));
                }
            }
        }
        let timestamp = match p.get("timestamp") {
            None => Timestamp::Keep,
            Some(Value::String(s)) if s == "keep" => Timestamp::Keep,
            Some(Value::String(s)) if s == "now" => Timestamp::Now,
            Some(Value::Number(n)) => Timestamp::At(n.as_i64().ok_or_else(|| p.bad("`timestamp` must be keep, now or whole seconds"))?),
            Some(_) => return Err(p.bad("`timestamp` must be keep, now or whole seconds since 1970")),
        };
        let rebuild_tables = Self::rebuild_tables(p)?;
        let optimize_for_speed = match p.opt_str("optimizeFor")? {
            None | Some("size") => false,
            Some("speed") => true,
            Some(other) => return Err(p.bad(format!("`optimizeFor` must be size or speed, not `{other}`"))),
        };
        if optimize_for_speed && (subroutinize == Some(true) || max_subrs.is_some()) {
            return Err(p.bad("`optimizeFor` speed expands CFF subroutines: leave out `cff.subroutinize` and `cff.maxSubrs`"));
        }
        let ufo_zip = match (zip_target, Self::ufo(p, to)?) {
            (false, zip) => zip,
            (true, true) => true,
            (true, false) if p.opt_object("ufo")?.is_some_and(|u| u.contains_key("structure")) => {
                return Err(p.bad("`to: ufoz` writes a UFOZ: leave out `ufo.structure` or give zip"));
            }
            (true, false) => true,
        };
        let (svg, newline) = Self::svg(p, to)?;
        let prepare = Self::prepare(p)?;
        let (cid_keyed, decid) = Self::cid(p)?;
        if cid_keyed.is_some() && !matches!(to, "otf" | "cff") {
            return Err(p.bad("`cid.keyed` needs CFF output (otf or cff)"));
        }
        if decid.is_some() && (!matches!(to, "otf" | "cff") || cid_keyed.is_some()) {
            return Err(p.bad("`cid.decid` needs CFF output (otf or cff) and no `cid.keyed`"));
        }
        Ok(Settings {
            prepare,
            cid_keyed,
            decid,
            cff_write,
            flatten_seac,
            ps_read,
            to,
            max_error_em,
            quad_to_cubic,
            reverse_direction: !keep_direction,
            post_format,
            subroutinize,
            max_subrs,
            hints: p.bool_or("hints", true)?,
            timestamp,
            keep_bounds: !p.bool_or("recalcBounds", true)?,
            rebuild_tables,
            optimize_for_speed,
            ufo_zip,
            svg,
            newline,
        })
    }

    /// `rebuildTables`: table tags, or `"all"`.
    fn rebuild_tables(p: &Params<'_>) -> Result<RebuildTables> {
        match p.get("rebuildTables") {
            None => Ok(RebuildTables::None),
            Some(Value::String(s)) if s == "all" => Ok(RebuildTables::All),
            Some(Value::Array(items)) => {
                if items.len() > MAX_REBUILD_TABLES {
                    return Err(p.bad(format!("`rebuildTables` names more than {MAX_REBUILD_TABLES} tables")));
                }
                let mut seen = BTreeSet::new();
                let mut tags = Vec::with_capacity(items.len());
                for item in items {
                    let name = item.as_str().ok_or_else(|| p.bad("`rebuildTables` items must be table tags"))?;
                    let tag = parse_tag(name).map_err(|e| p.bad(format!("`rebuildTables`: {e}")))?;
                    if seen.insert(tag) {
                        tags.push(tag);
                    }
                }
                Ok(RebuildTables::These(tags))
            }
            Some(_) => Err(p.bad("`rebuildTables` must be a list of table tags or \"all\"")),
        }
    }

    /// `ufo`: `structure` (whether the result is a UFOZ) and `skipExportKey`. A compiled font
    /// exports every glyph, so the key has nothing to list; it is checked all the same.
    fn ufo(p: &Params<'_>, to: &str) -> Result<bool> {
        let Some(ufo) = p.opt_object("ufo")? else { return Ok(false) };
        if to != "ufo" {
            return Err(p.bad("`ufo` shapes UFO output (to: ufo)"));
        }
        let mut zip = false;
        for (key, value) in ufo {
            match (key.as_str(), value.as_str()) {
                ("structure", Some("package")) => zip = false,
                ("structure", Some("zip")) => zip = true,
                ("structure", Some("json")) => return Err(EngineError::Unsupported("`ufo.structure` json in font.convert".into())),
                ("structure", _) => return Err(p.bad("`ufo.structure` must be package or zip")),
                ("skipExportKey", Some("public" | "glyphs")) => {}
                ("skipExportKey", _) => return Err(p.bad("`ufo.skipExportKey` must be public or glyphs")),
                (other, _) => return Err(EngineError::Unsupported(format!("`ufo.{other}` in font.convert"))),
            }
        }
        Ok(zip)
    }

    /// `svg` and `newline`: how an SVG font is written, and its line ending.
    fn svg(p: &Params<'_>, to: &str) -> Result<(SvgFontOptions, &'static str)> {
        let mut svg = SvgFontOptions::default();
        if let Some(options) = p.opt_object("svg")? {
            if to != "svg-font" {
                return Err(p.bad("`svg` shapes SVG font output (to: svg-font)"));
            }
            for (key, value) in options {
                let flag = || value.as_bool().ok_or_else(|| p.bad(format!("`svg.{key}` must be true or false")));
                match key.as_str() {
                    "glyphNames" => {
                        svg.glyph_names = match value.as_str() {
                            Some("none") => SvgGlyphNames::None,
                            Some("non-ascii") => SvgGlyphNames::NonAscii,
                            Some("all") => SvgGlyphNames::All,
                            _ => return Err(p.bad("`svg.glyphNames` must be none, non-ascii or all")),
                        }
                    }
                    "absolute" => svg.absolute = flag()?,
                    "standalone" => svg.standalone = flag()?,
                    other => return Err(p.bad(format!("`svg.{other}` is not an SVG font setting"))),
                }
            }
        }
        let newline = match p.opt_str("newline")? {
            None => "\n",
            Some(_) if to != "svg-font" => return Err(p.bad("`newline` sets the line ending of text output (to: svg-font)")),
            Some("lf") => "\n",
            Some("cr") => "\r",
            Some("crlf") => "\r\n",
            Some(other) => return Err(p.bad(format!("`newline` must be lf, cr or crlf, not `{other}`"))),
        };
        Ok((svg, newline))
    }

    /// `cid`: `keyed` (`true` is Adobe-Identity-0, a string names
    /// registry-ordering-supplement), `decid` and `fd`. `info` is refused.
    fn cid(p: &Params<'_>) -> Result<(Option<CidCollection>, Option<Decid>)> {
        let Some(cid) = p.opt_object("cid")? else { return Ok((None, None)) };
        let mut keyed = None;
        let mut decid = false;
        let mut fd = None;
        for (key, value) in cid {
            match (key.as_str(), value) {
                ("decid", Value::Bool(b)) => decid = *b,
                ("decid", _) => return Err(p.bad("`cid.decid` must be true or false")),
                ("fd", v) => {
                    let n = v
                        .as_u64()
                        .and_then(|n| u16::try_from(n).ok())
                        .filter(|n| usize::from(*n) < MAX_FONT_DICTS)
                        .ok_or_else(|| p.bad(format!("`cid.fd` must be a Font DICT index from 0 to {}", MAX_FONT_DICTS - 1)))?;
                    fd = Some(n);
                }
                ("keyed", Value::Bool(false)) => {}
                ("keyed", Value::Bool(true)) => keyed = Some(("Adobe", "Identity", 0)), // brand-ok: the CFF registry value
                ("keyed", Value::String(ros)) => {
                    let bad = || {
                        p.bad(format!("`cid.keyed` `{ros}` is not REGISTRY-ORDERING-SUPPLEMENT (three parts joined by hyphens, the last a number)"))
                    };
                    let (rest, supplement) = ros.rsplit_once('-').ok_or_else(bad)?;
                    let (registry, ordering) = rest.split_once('-').ok_or_else(bad)?;
                    let supplement: i64 = supplement.parse().map_err(|_| bad())?;
                    keyed = Some((registry, ordering, supplement));
                }
                ("keyed", _) => return Err(p.bad("`cid.keyed` must be true, false or REGISTRY-ORDERING-SUPPLEMENT")),
                (other, _) => return Err(EngineError::Unsupported(format!("`cid.{other}` in font.convert"))),
            }
        }
        if fd.is_some() && !decid {
            return Err(p.bad("`cid.fd` forces a Font DICT on a font made name-keyed: it needs `cid.decid`"));
        }
        let keyed = keyed.map(|(registry, ordering, supplement)| CidCollection {
            registry: registry.to_string(),
            ordering: ordering.to_string(),
            supplement,
        });
        Ok((keyed, decid.then_some(Decid { fd })))
    }

    /// `fdSelect` and `excludeFd`: Font DICT indexes.
    fn font_dicts(p: &Params<'_>) -> Result<Option<FontDictFilter>> {
        let list = |key: &str| -> Result<Option<BTreeSet<u16>>> {
            let Some(value) = p.get(key) else { return Ok(None) };
            let items = value.as_array().ok_or_else(|| p.bad(format!("`{key}` must be a list of Font DICT indexes")))?;
            if items.len() > MAX_FONT_DICTS {
                return Err(p.bad(format!("`{key}` lists more than {MAX_FONT_DICTS} Font DICTs")));
            }
            items
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u16::try_from(n).ok())
                        .filter(|n| usize::from(*n) < MAX_FONT_DICTS)
                        .ok_or_else(|| p.bad(format!("`{key}` items must be Font DICT indexes from 0 to {}", MAX_FONT_DICTS - 1)))
                })
                .collect::<Result<BTreeSet<u16>>>()
                .map(Some)
        };
        let keep = list("fdSelect")?;
        let exclude = list("excludeFd")?;
        if keep.is_none() && exclude.is_none() {
            return Ok(None);
        }
        Ok(Some(FontDictFilter { keep, exclude: exclude.unwrap_or_default() }))
    }

    /// `location`, `clamp`, the GOADB keys, the Font DICT keys and the overlap keys.
    fn prepare(p: &Params<'_>) -> Result<PrepareOptions> {
        let location = match p.opt_object("location")? {
            None => None,
            Some(axes) => {
                if axes.len() > MAX_LOCATION_AXES {
                    return Err(p.bad(format!("`location` names more than {MAX_LOCATION_AXES} axes")));
                }
                let mut list = Vec::with_capacity(axes.len());
                for (axis, value) in axes {
                    let tag = parse_tag(axis).map_err(|e| p.bad(format!("`location`: {e}")))?;
                    let v = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad(format!("`location.{axis}` must be a number")))?;
                    list.push((tag, v));
                }
                Some(list)
            }
        };
        let goadb = match p.opt_str("goadb")? {
            None => {
                for key in ["goadbRename", "goadbSubset", "goadbOrder"] {
                    if p.get(key).is_some() {
                        return Err(p.bad(format!("`{key}` needs `goadb`")));
                    }
                }
                None
            }
            Some(path) => {
                let text = goadb_text(Path::new(path), MAX_GOADB_BYTES)?;
                let goadb = fontcraft_tables::goadb::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
                    .map_err(|e| p.bad(format!("`goadb` {path}: {e}")))?;
                Some(GoadbUse {
                    goadb,
                    rename: p.bool_or("goadbRename", true)?,
                    subset: p.bool_or("goadbSubset", false)?,
                    order: p.bool_or("goadbOrder", false)?,
                })
            }
        };
        Ok(PrepareOptions {
            location,
            clamp: p.bool_or("clamp", true)?,
            glyphs: None,
            goadb,
            font_dicts: Self::font_dicts(p)?,
            remove_overlaps: p.bool_or("removeOverlaps", false)?,
            ignore_overlap_errors: p.bool_or("ignoreOverlapErrors", false)?,
        })
    }

    /// Converts one font: the steps of [`Settings::prepare`] with the glyphs `glyphs` and
    /// `excludeGlyphs` select in it, then the conversion.
    fn run(&self, p: &Params<'_>, font: &SfntFont) -> Result<Converted> {
        let mut steps = self.prepare.clone();
        steps.glyphs = match cid_selectors(p, font)? {
            Some(rewritten) => super::dehint::selection(&Params::new("font.convert", &rewritten)?, font)?,
            None => super::dehint::selection(p, font)?,
        };
        if self.to == "ufo" {
            return ufo(font, steps);
        }
        if self.to == "svg-font" {
            return self.svg_font(font, steps);
        }
        let c = convert_prepared(font, &self.options(units_per_em(font))?, &steps)?;
        Ok(Converted { output: Output::File(c.bytes), glyphs: c.glyphs, left_behind: c.left_behind, warnings: c.warnings })
    }

    fn options(&self, units_per_em: f64) -> Result<ConvertOptions> {
        let max_error = self.max_error_em.map_or(1.0, |e| e * units_per_em);
        let target = match self.to {
            "ttf" => ConvertTarget::TrueType { max_error },
            "otf" => ConvertTarget::Cff,
            "otf-cff2" => ConvertTarget::Cff2,
            "cff" => ConvertTarget::BareCff,
            _ => ConvertTarget::BareCff2,
        };
        let mut o = ConvertOptions::new(target);
        o.reverse_direction = self.reverse_direction;
        o.quad_to_cubic = self.quad_to_cubic;
        o.cubic_error = max_error;
        o.post_format = self.post_format;
        o.subroutinize = self.subroutinize;
        o.max_subrs = self.max_subrs;
        o.hints = self.hints;
        o.cid_keyed = self.cid_keyed.clone();
        o.decid = self.decid;
        o.cff_write = self.cff_write.clone();
        o.flatten_seac = self.flatten_seac;
        o.rebuild_tables = self.rebuild_tables.clone();
        o.keep_bounds = self.keep_bounds;
        o.optimize_for_speed = self.optimize_for_speed;
        o.modified = match self.timestamp {
            Timestamp::Keep => None,
            Timestamp::At(unix) => Some(unix.saturating_add(EPOCH_1904)),
            Timestamp::Now => Some(now()?.saturating_add(EPOCH_1904)),
        };
        Ok(o)
    }

    fn extension(&self, container: Container) -> &'static str {
        match (container, self.to) {
            (Container::Woff, _) => "woff",
            (Container::Woff2, _) => "woff2",
            (_, "ttf") => "ttf",
            (_, "cff") => "cff",
            (_, "cff2") => "cff2",
            (_, "ufo") if self.ufo_zip => "ufoz",
            (_, "ufo") => "ufo",
            (_, "svg-font") => "svg",
            _ => "otf",
        }
    }

    /// An SVG font of `font` after the steps, from the font as `file.open` reads it; a variable
    /// font without a location is snapshotted at its default location. Glyphs without a
    /// character (`.notdef` aside) get private-use ones from U+E000 on, in glyph order, as the
    /// reference converter gives them, so that text can reach every glyph; code points the font
    /// already maps are skipped.
    fn svg_font(&self, font: &SfntFont, mut steps: PrepareOptions) -> Result<Converted> {
        if steps.location.is_none() && font.get(*b"fvar").is_some() {
            steps.location = Some(Vec::new());
        }
        let (font, mut left_behind) = prepare(font, &steps, true)?;
        let bytes = fontcraft_tables::write(&font, &WriteOptions::default())?;
        let mut decompiled = fontcraft_decompile::decompile(&bytes, None)?;
        left_behind.extend(decompiled.warnings);
        let model = &mut decompiled.font;
        let used: HashSet<u32> = model.glyphs.iter().flat_map(|g| g.unicodes.iter().copied()).collect();
        let mut next = PRIVATE_USE.0;
        for i in 0..model.glyphs.len() {
            if !model.glyphs.as_arcs().get(i).is_some_and(|g| g.unicodes.is_empty() && g.name != ".notdef") {
                continue;
            }
            while used.contains(&next) {
                next += 1;
            }
            if next > PRIVATE_USE.1 {
                left_behind.push("characters for every glyph: the private use area U+E000 to U+F8FF ran out".into());
                break;
            }
            if let Some(mut g) = model.glyphs.get_mut_at(i) {
                g.unicodes.push(next);
            }
            next += 1;
        }
        let master =
            model.masters.first().map(|m| m.id.clone()).ok_or_else(|| EngineError::Unsupported("an SVG font of a font without a master".into()))?;
        let text = write_svg_font_with(model, &master, &self.svg).map_err(|e| EngineError::Io(format!("cannot write the SVG font: {e}")))?;
        let text = if self.newline == "\n" { text } else { text.replace('\n', self.newline) };
        Ok(Converted { glyphs: model.glyphs.len(), output: Output::File(text.into_bytes()), left_behind, warnings: Vec::new() })
    }
}

/// The PostScript name of the font in `input`, for `nameBy: postscript-name`.
fn font_file_name(p: &Params<'_>, input: &Path) -> Result<String> {
    let file = file_io::read_input(input)?;
    let font = match file.kind {
        k if k.is_font() => fontcraft_tables::read(&file.bytes, super::dehint::face_index(p, &file.bytes)?)?,
        InputKind::Ttx => import_tree(SfntFont::new(Flavour::TrueType), &super::tables_import::read_dump(input)?, &ImportOptions::default())?,
        _ if PsSource::sniff(&file.bytes).is_some() => read_postscript(&file.bytes)?.font,
        _ => return Err(EngineError::Unsupported("nameBy postscript-name needs a compiled font, TTX, a Type 1 font or a bare CFF table".into())),
    };
    // The name is printable ASCII without PostScript delimiters; a backslash or colon would
    // still be a path on some systems.
    let name = postscript_name(&font);
    if name.contains(['\\', ':']) || name.starts_with('.') {
        return Err(EngineError::Unsupported(format!("the PostScript name `{name}` cannot be a file name: give output or use nameBy filename")));
    }
    Ok(name)
}

/// `font` converted to `to` (`ttf`, `otf`, `otf-cff2`, `cff` or `cff2`) with the default
/// settings: the bytes and what was left behind. For commands that hand their result to the
/// converter (`font.merge` with `to`).
pub(super) fn convert_to(font: &SfntFont, to: &str) -> Result<(Vec<u8>, Vec<String>)> {
    if matches!(to, "ufo" | "svg-font") {
        return Err(EngineError::Unsupported(format!("`to: {to}` here: convert the result with font.convert")));
    }
    let v = json!({ "to": to });
    let p = Params::new("font.convert", &v)?;
    let settings = Settings::read(&p, to)?;
    let c = settings.run(&p, font)?;
    match c.output {
        Output::File(bytes) => Ok((bytes, c.left_behind)),
        Output::Folder(_) => Err(EngineError::Unsupported(format!("`to: {to}` writes a folder"))),
    }
}

/// The text of a GOADB file of at most `limit` bytes; a larger file is refused, not cut.
pub(super) fn goadb_text(path: &Path, limit: usize) -> Result<String> {
    let read = file_io::read_input_up_to(path, limit)?;
    if read.bytes.len() > limit {
        return Err(EngineError::Io(format!("the GOADB {} is larger than {limit} bytes", path.display())));
    }
    String::from_utf8(read.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", path.display())))
}

/// `glyphs` and `excludeGlyphs` with the `/CID` items of the reference converter (`/17`,
/// `/100-200`) replaced by glyph id ranges; `None` when neither has one. A CID range keeps the
/// CIDs the font has; a single CID the font lacks is refused, as a missing name is. The CID
/// ranges are merged before the glyphs are looked up, and the glyphs are written as runs once,
/// so the work and the rewritten list stay within the glyph count whatever the ranges repeat.
pub(super) fn cid_selectors(p: &Params<'_>, font: &SfntFont) -> Result<Option<Value>> {
    let mut out = serde_json::Map::new();
    let mut by_cid: Option<std::collections::BTreeMap<u32, u16>> = None;
    for key in ["glyphs", "excludeGlyphs"] {
        let Some(value) = p.get(key) else { continue };
        let items: Vec<&str> = match value {
            Value::String(s) => s.split([',', ' ']).filter(|w| !w.is_empty()).collect(),
            Value::Array(a) => a.iter().map(|v| v.as_str().ok_or_else(|| p.bad(format!("`{key}` items must be strings")))).collect::<Result<_>>()?,
            _ => Vec::new(),
        };
        if items.len() > MAX_SELECTOR_ITEMS {
            return Err(p.bad(format!("`{key}` lists more than {MAX_SELECTOR_ITEMS} items")));
        }
        if !items.iter().any(|i| i.starts_with('/')) {
            out.insert(key.to_string(), value.clone());
            continue;
        }
        if by_cid.is_none() {
            let cids = glyph_cids(font)?.ok_or_else(|| p.bad(format!("`{key}`: /CID items need a CID-keyed CFF font")))?;
            by_cid = Some(cids.iter().enumerate().filter_map(|(g, c)| u16::try_from(g).ok().map(|g| (u32::from(*c), g))).collect());
        }
        let map = by_cid.as_ref().ok_or_else(|| p.bad("no CID map"))?;
        let mut list = Vec::with_capacity(items.len());
        let mut gids: BTreeSet<u16> = BTreeSet::new();
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for item in items {
            let Some(spec) = item.strip_prefix('/') else {
                list.push(Value::String(item.to_string()));
                continue;
            };
            let cid = |t: &str| t.parse::<u32>().ok().filter(|c| *c <= 65_535).ok_or_else(|| p.bad(format!("`{item}` is not a CID or a CID range")));
            match spec.split_once('-') {
                Some((a, b)) => {
                    let (low, high) = (cid(a)?, cid(b)?);
                    if low > high {
                        return Err(p.bad(format!("the range `{item}` runs backwards")));
                    }
                    ranges.push((low, high));
                }
                None => {
                    let c = cid(spec)?;
                    gids.insert(*map.get(&c).ok_or_else(|| p.bad(format!("the font has no CID {c}")))?);
                }
            }
        }
        ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (low, high) in ranges {
            match merged.last_mut() {
                Some(last) if low <= last.1.saturating_add(1) => last.1 = last.1.max(high),
                _ => merged.push((low, high)),
            }
        }
        for (low, high) in merged {
            gids.extend(map.range(low..=high).map(|(_, g)| *g));
        }
        // Runs of consecutive glyph ids go in as one range each.
        let mut run: Option<(u16, u16)> = None;
        for g in gids.into_iter().map(Some).chain([None]) {
            match (run, g) {
                (Some((a, b)), Some(g)) if u32::from(b) + 1 == u32::from(g) => run = Some((a, g)),
                (current, next) => {
                    match current {
                        Some((a, b)) if a == b => list.push(Value::String(format!("gid{a}"))),
                        Some((a, b)) => list.push(Value::String(format!("gid{a}..gid{b}"))),
                        None => {}
                    }
                    run = next.map(|g| (g, g));
                }
            }
        }
        out.insert(key.to_string(), Value::Array(list));
    }
    Ok(by_cid.is_some().then_some(Value::Object(out)))
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> Result<i64> {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| EngineError::Io(format!("the clock: {e}")))?;
    i64::try_from(since.as_secs()).map_err(|_| EngineError::Io("the clock is out of range".into()))
}

#[cfg(target_arch = "wasm32")]
fn now() -> Result<i64> {
    Err(EngineError::Unsupported("the current time in the browser".into()))
}

fn units_per_em(font: &SfntFont) -> f64 {
    font.get(*b"head").and_then(|h| h.get(18..20)).and_then(|b| <[u8; 2]>::try_from(b).ok()).map_or(1000.0, |b| f64::from(u16::from_be_bytes(b)))
}

fn convert_one(p: &Params<'_>, input: &Path, settings: &Settings, container: Container) -> Result<Converted> {
    let file = file_io::read_input(input)?;
    let no_face = || -> Result<()> {
        if p.get("face").is_some() {
            return Err(p.bad("`face` names a font of a collection; this input holds one font"));
        }
        Ok(())
    };
    let converted = match file.kind {
        k if k.is_font() => {
            let face = super::dehint::face_index(p, &file.bytes)?;
            let font = fontcraft_tables::read(&file.bytes, face)?;
            settings.run(p, &font)?
        }
        // TTX text is compiled as the reference compiles it without a font to merge into.
        InputKind::Ttx => {
            no_face()?;
            let root = super::tables_import::read_dump(input)?;
            let options = ImportOptions { keep_bounds: settings.keep_bounds, ..ImportOptions::default() };
            let font = import_tree(SfntFont::new(Flavour::TrueType), &root, &options)?;
            settings.run(p, &font)?
        }
        _ if PsSource::sniff(&file.bytes).is_some() => {
            no_face()?;
            if !settings.prepare.is_empty() || p.get("glyphs").is_some() || p.get("excludeGlyphs").is_some() {
                return Err(EngineError::Unsupported(
                    "glyph selection, a GOADB, a location or overlap removal on a Type 1 font or bare CFF table: convert it to otf first".into(),
                ));
            }
            let ps = read_postscript_with(&file.bytes, &settings.ps_read)?;
            if settings.to == "ufo" {
                ufo(&ps.font, PrepareOptions::default())?
            } else if settings.to == "svg-font" {
                settings.svg_font(&ps.font, PrepareOptions::default())?
            } else {
                let c = convert_postscript(&ps, &settings.options(units_per_em(&ps.font))?)?;
                Converted { output: Output::File(c.bytes), glyphs: c.glyphs, left_behind: c.left_behind, warnings: c.warnings }
            }
        }
        _ => {
            return Err(EngineError::Unsupported(
                "font.convert takes compiled fonts, TTX, Type 1 fonts and bare CFF tables; a source is converted with file.saveAs or compiled with file.exportStatic".into(),
            ));
        }
    };
    let output = match (container, converted.output) {
        (Container::Plain, output) | (_, output @ Output::Folder(_)) => output,
        (Container::Woff, Output::File(bytes)) => Output::File(
            fontcraft_container::woff::encode(&bytes, &fontcraft_container::Extras::default()).map_err(|e| EngineError::Io(format!("WOFF: {e}")))?,
        ),
        (Container::Woff2, Output::File(bytes)) => Output::File(
            fontcraft_container::woff2::encode(&bytes, &fontcraft_container::woff2::Options::default())
                .map_err(|e| EngineError::Io(format!("WOFF2: {e}")))?,
        ),
    };
    Ok(Converted { output, glyphs: converted.glyphs, left_behind: converted.left_behind, warnings: converted.warnings })
}

/// A UFO of `font` as `file.open` reads a compiled font, after the steps; a variable font
/// without a location is snapshotted at its default location, as the reference converter
/// snapshots one.
fn ufo(font: &SfntFont, mut steps: PrepareOptions) -> Result<Converted> {
    if steps.location.is_none() && font.get(*b"fvar").is_some() {
        steps.location = Some(Vec::new());
    }
    let (font, mut left_behind) = prepare(font, &steps, true)?;
    let bytes = fontcraft_tables::write(&font, &WriteOptions::default())?;
    let decompiled = fontcraft_decompile::decompile(&bytes, None)?;
    left_behind.extend(decompiled.warnings);
    let files = fontcraft_io_ufo::ufo::write(&decompiled.font)?;
    Ok(Converted { glyphs: decompiled.font.glyphs.len(), output: Output::Folder(files), left_behind, warnings: Vec::new() })
}

/// The temporary files of one run. [`Staging::commit`] moves them into place; dropped before
/// that, the run removes them and the output folder it created.
struct Staging {
    #[cfg(not(target_arch = "wasm32"))]
    staged: Vec<StagedOutput>,
    #[cfg(not(target_arch = "wasm32"))]
    created: Vec<PathBuf>,
}

/// One output's temporary file or folder.
#[cfg(not(target_arch = "wasm32"))]
struct StagedOutput {
    temp: PathBuf,
    target: PathBuf,
    /// Whether the target existed before.
    existed: bool,
    /// A UFO folder, swapped in for an existing one.
    folder: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl StagedOutput {
    /// Puts the temporary in place. An existing folder is set aside first, and the path it was
    /// moved to is returned: [`Staging::commit`] removes it once every output is in place, or
    /// puts it back when a later output fails.
    fn move_in(&self) -> std::result::Result<Option<PathBuf>, String> {
        if !(self.folder && self.existed) {
            return std::fs::rename(&self.temp, &self.target).map(|()| None).map_err(|e| e.to_string());
        }
        let aside = self.beside("old");
        std::fs::rename(&self.target, &aside).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(&self.temp, &self.target) {
            if let Err(back) = std::fs::rename(&aside, &self.target) {
                return Err(format!("{e}; the existing UFO could not be put back ({back}) and is at {}", aside.display()));
            }
            return Err(e.to_string());
        }
        Ok(Some(aside))
    }

    /// Puts the folder set aside at `aside` back in place of the new one; false when it cannot,
    /// and then the new folder stays and the old one stays at `aside`.
    fn restore(&self, aside: &Path) -> bool {
        let new = self.beside("new");
        if std::fs::rename(&self.target, &new).is_err() {
            return false;
        }
        if std::fs::rename(aside, &self.target).is_err() {
            let _ = std::fs::rename(&new, &self.target);
            return false;
        }
        let _ = std::fs::remove_dir_all(&new);
        true
    }

    /// A hidden path beside the target, for a folder moved out of the way.
    fn beside(&self, what: &str) -> PathBuf {
        let name = self.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font.ufo".into());
        self.target.with_file_name(format!(".{name}.{}.{what}", std::process::id()))
    }

    fn remove(path: &Path, folder: bool) {
        if folder {
            let _ = std::fs::remove_dir_all(path);
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Staging {
    /// Creates the request's `outputDir` when it is missing.
    fn open(request: &OutputRequest) -> Result<Self> {
        Ok(Staging { staged: Vec::new(), created: file_io::create_output_dir(request)? })
    }

    /// Writes output `n` to its temporary file or folder.
    fn stage(&mut self, out: &Path, output: &Output, n: usize) -> Result<()> {
        let (temp, target, folder) = match output {
            Output::File(bytes) => {
                let (temp, target) = stage(out, bytes, n)?;
                (temp, target, false)
            }
            Output::Folder(files) => {
                let (temp, target) = stage_folder(out, files, n)?;
                (temp, target, true)
            }
        };
        let existed = target.exists();
        self.staged.push(StagedOutput { temp, target, existed, folder });
        Ok(())
    }

    /// Moves every temporary into place. When a move fails, the outputs already moved that
    /// replaced nothing are removed again, and an existing UFO already replaced gets its old
    /// folder back; an existing file already replaced keeps its new content, and the message
    /// says how many there are.
    fn commit(mut self) -> Result<()> {
        // The existing folders set aside so far, by output index.
        let mut asides: Vec<(usize, PathBuf)> = Vec::new();
        for (i, s) in self.staged.iter().enumerate() {
            let e = match s.move_in() {
                Ok(aside) => {
                    asides.extend(aside.map(|a| (i, a)));
                    continue;
                }
                Err(e) => e,
            };
            let mut replaced = 0;
            let mut kept_aside = Vec::new();
            for (j, moved) in self.staged.iter().enumerate().take(i) {
                if !moved.existed {
                    StagedOutput::remove(&moved.target, moved.folder);
                    continue;
                }
                if let Some((_, aside)) = asides.iter().find(|(k, _)| *k == j) {
                    if moved.restore(aside) {
                        continue;
                    }
                    kept_aside.push(format!("{} is at {}", moved.target.display(), aside.display()));
                }
                replaced += 1;
            }
            let mut kept = if replaced == 0 {
                "nothing was written".to_string()
            } else {
                format!("{replaced} existing outputs before it were already replaced; the new files were removed")
            };
            if !kept_aside.is_empty() {
                kept.push_str(&format!("; the previous content of {}", kept_aside.join(", of ")));
            }
            return Err(EngineError::Io(format!("cannot write {}: {e}; {kept}", s.target.display())));
        }
        for (_, aside) in asides {
            let _ = std::fs::remove_dir_all(aside);
        }
        self.staged.iter().for_each(|s| fontcraft_format::written::record(&s.target));
        self.staged.clear();
        self.created.clear();
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Staging {
    fn drop(&mut self) {
        for s in &self.staged {
            StagedOutput::remove(&s.temp, s.folder);
        }
        file_io::remove_created(&self.created);
    }
}

#[cfg(target_arch = "wasm32")]
impl Staging {
    fn open(_: &OutputRequest) -> Result<Self> {
        Ok(Staging {})
    }

    fn stage(&mut self, _: &Path, _: &Output, _: usize) -> Result<()> {
        Err(EngineError::Unsupported("writing files in the browser".into()))
    }

    fn commit(self) -> Result<()> {
        Ok(())
    }
}

/// The file `out` names: the file it links to, for a link.
#[cfg(not(target_arch = "wasm32"))]
fn resolved_target(out: &Path) -> Result<PathBuf> {
    if out.is_symlink() {
        std::fs::canonicalize(out).map_err(|e| EngineError::Io(format!("cannot write {}: {e}", out.display())))
    } else {
        Ok(out.to_path_buf())
    }
}

/// Writes `bytes` to a new temporary file beside `out` (beside the file it links to, for a
/// link), with the permissions of the file it will replace.
#[cfg(not(target_arch = "wasm32"))]
fn stage(out: &Path, bytes: &[u8], n: usize) -> Result<(PathBuf, PathBuf)> {
    use std::io::Write as _;
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    let target = resolved_target(out)?;
    if target.is_dir() {
        return Err(EngineError::Io(format!("cannot write {}: it is a folder", target.display())));
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font".into());
    let temp = target.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| io(&temp, e))?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(io(&temp, e));
    }
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    Ok((temp, target))
}

/// Writes a UFO's files into a new temporary folder beside `out`. An existing `out` must be a
/// UFO (a folder with `metainfo.plist`), which the commit replaces.
#[cfg(not(target_arch = "wasm32"))]
fn stage_folder(out: &Path, files: &fontcraft_io_ufo::Files, n: usize) -> Result<(PathBuf, PathBuf)> {
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    let target = resolved_target(out)?;
    if target.exists() && !(target.is_dir() && target.join("metainfo.plist").is_file()) {
        return Err(EngineError::Io(format!("cannot write {}: something that is not a UFO has that name", target.display())));
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font.ufo".into());
    let temp = target.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    file_io::check_write(&temp)?;
    std::fs::create_dir(&temp).map_err(|e| io(&temp, e))?;
    for (relative, bytes) in files {
        let relative_path = Path::new(relative);
        let written = if relative_path.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
            let path = temp.join(relative_path);
            path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, bytes)).map_err(|e| io(&target, e))
        } else {
            Err(EngineError::Io(format!("cannot write {}: the UFO writer named a file `{relative}`", target.display())))
        };
        if let Err(e) = written {
            let _ = std::fs::remove_dir_all(&temp);
            return Err(e);
        }
    }
    Ok((temp, target))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn a_goadb_over_the_limit_is_refused_not_cut() {
        let path = std::env::temp_dir().join(format!("fontcraft-goadb-limit-{}", std::process::id()));
        std::fs::write(&path, "A a\nB b\n\n").unwrap();
        assert!(goadb_text(&path, 8).is_err(), "9 bytes against a limit of 8");
        assert_eq!(goadb_text(&path, 9).unwrap(), "A a\nB b\n\n");
        let _ = std::fs::remove_file(&path);
    }
}
