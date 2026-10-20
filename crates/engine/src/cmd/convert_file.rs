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
//! the UFO writer. Each output is written to a temporary file (a folder, for a UFO) beside its
//! target as soon as its input is converted, so memory holds one font at a time; only when all
//! of them are there are they moved into place, so a failing conversion writes nothing.

use std::path::{Path, PathBuf};

use std::collections::BTreeSet;

use fontcraft_tables::convert::{
    CidCollection, ConvertOptions, ConvertTarget, FontDictFilter, GoadbUse, PostFormat, PrepareOptions, PsSource, QuadToCubic, convert_postscript,
    convert_prepared, glyph_cids, prepare, read_postscript,
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
const MAX_GOADB_BYTES: usize = 64 << 20;
/// Most axes a location names.
const MAX_LOCATION_AXES: usize = fontcraft_tables::limits::MAX_AXES;
/// Most Font DICTs a CFF table holds (an FDSelect index is one byte).
const MAX_FONT_DICTS: usize = 256;

/// Parameters of the row this version does not act on yet; naming one is refused, so a call
/// never silently does less than it asks.
const NOT_YET: [&str; 9] = ["type1", "newline", "ufo", "svg", "recalcBounds", "rebuildTables", "optimizeFor", "validateSources", "smartComponents"];

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.convert",
            "Convert Font Format",
            [],
            None,
            "{inputs: [path] | input: path, to: \"ttf\"|\"otf\"|\"otf-cff2\"|\"cff\"|\"cff2\"|\"ufo\", output?: path (one input), outputDir?: path, inPlace?: bool, noClobber?: bool, face?: int|name, container?: \"plain\"|\"woff\"|\"woff2\", nameBy?: \"filename\", curves?: {maxError?: em fraction (default one font unit), conversion?: \"optimized\"|\"exact\", keepDirection?: bool}, postFormat?: 2|3, cff?: {subroutinize?: bool, maxSubrs?: int}, hints?: bool, cid?: {keyed?: bool|\"REGISTRY-ORDERING-SUPPLEMENT\"}, glyphs?: selector, excludeGlyphs?: selector, fdSelect?: [int], excludeFd?: [int], goadb?: path, goadbRename?: bool (default true), goadbSubset?: bool, goadbOrder?: bool, location?: {axis: number}, clamp?: bool (default true), removeOverlaps?: bool, ignoreOverlapErrors?: bool, timestamp?: \"keep\"|\"now\"|int (seconds since 1970)}. Inputs: compiled fonts, TTX text (compiled first), Type 1 fonts (PFA, PFB) and bare CFF tables (wrapped in an OpenType CFF font first; bare tables are written without subroutines unless cff.subroutinize says otherwise). CFF or static CFF2 to TrueType (cubic to quadratic within maxError, contours reversed), TrueType to CFF or CFF2 (components decomposed, instructions dropped), CFF to CFF2 and back with hints kept, the bare outline table (cff, cff2), or a UFO of the font as file.open reads it. A variable font converted to UFO or to another outline flavour is snapshotted at its default location unless location is given. cid.keyed makes a name-keyed CFF result CID-keyed, CID n for glyph n (true: the Identity ordering, supplement 0; or a REGISTRY-ORDERING-SUPPLEMENT string). Before converting: location snapshots a variable font (axes not named at their default; clamped to the axis ranges unless clamp is false), glyphs and excludeGlyphs keep a subset (names, glyph ids, ranges such as 3-9, U+0041, and in a CID-keyed font /CID or /CID-CID; .notdef always stays), fdSelect and excludeFd keep the glyphs of some Font DICTs of a CID-keyed CFF or a CFF2 font (indexes from 0), goadb renames glyphs from its second column to its first (goadbRename), keeps only the glyphs it lists (goadbSubset) and orders them as it does (goadbOrder); removeOverlaps merges overlapping contours (TrueType outlines before converting a TrueType font or after converting to TrueType; CFF and CFF2 outlines before converting, a merged glyph losing its stem hints), and ignoreOverlapErrors keeps a glyph that cannot be merged. Without output the result goes beside the input with the new extension → {results: [{input, output, bytes, glyphs, leftBehind: [text]}]}",
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
    if let Some(name_by) = p.opt_str("nameBy")?
        && name_by != "filename"
    {
        return Err(EngineError::Unsupported(format!("nameBy `{name_by}`: outputs are named after their input files")));
    }
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
    if container != Container::Plain && matches!(to, "cff" | "cff2" | "ufo") {
        return Err(p.bad("a bare CFF or CFF2 table or a UFO cannot be wrapped in WOFF"));
    }
    if to == "ufo" && request.in_place {
        return Err(p.bad("`inPlace` cannot replace a font with a UFO folder"));
    }

    let mut outputs = Vec::with_capacity(inputs.len());
    for input in &inputs {
        outputs.push(file_io::output_path(Path::new(input), &request, "", Some(settings.extension(container))).map_err(|e| in_input(e, input))?);
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
        let c = convert_one(&p, Path::new(input), &settings, container).map_err(|e| in_input(e, input))?;
        staging.stage(output, &c.output, n)?;
        results.push(json!({
            "input": input,
            "output": output.display().to_string(),
            "bytes": c.output.len(),
            "glyphs": c.glyphs,
            "leftBehind": c.left_behind,
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
    /// The steps around the conversion; the glyph selection is resolved for each input.
    prepare: PrepareOptions,
}

enum Timestamp {
    Keep,
    Now,
    At(i64),
}

impl Settings {
    fn read(p: &Params<'_>, to: &str) -> Result<Self> {
        let to: &'static str = match to {
            "ttf" => "ttf",
            "otf" => "otf",
            "otf-cff2" => "otf-cff2",
            "cff" => "cff",
            "cff2" => "cff2",
            "ufo" => "ufo",
            "ttx" => return Err(EngineError::Unsupported("`to: ttx`: write TTX with font.dumpTables".into())),
            "pfa" | "pfb" | "t1-text" => return Err(EngineError::Unsupported(format!("`to: {to}`: Type 1 fonts are read, not written"))),
            "ufoz" | "designspace" | "glyphs" | "native" => {
                return Err(EngineError::Unsupported(format!("`to: {to}` from a compiled font: open it (file.open) and save it with file.saveAs")));
            }
            "svg-font" => return Err(EngineError::Unsupported("`to: svg-font`".into())),
            other => return Err(p.bad(format!("`to` must be ttf, otf, otf-cff2, cff, cff2 or ufo, not `{other}`"))),
        };
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
        if let Some(cff) = cff {
            for (key, value) in cff {
                match key.as_str() {
                    "subroutinize" => subroutinize = Some(value.as_bool().ok_or_else(|| p.bad("`cff.subroutinize` must be true or false"))?),
                    "maxSubrs" => {
                        let n = value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad("`cff.maxSubrs` must be a whole number"))?;
                        max_subrs = Some(n);
                    }
                    other => return Err(EngineError::Unsupported(format!("`cff.{other}` in font.convert"))),
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
        let prepare = Self::prepare(p)?;
        let cid_keyed = Self::cid(p)?;
        if cid_keyed.is_some() && !matches!(to, "otf" | "cff") {
            return Err(p.bad("`cid.keyed` needs CFF output (otf or cff)"));
        }
        Ok(Settings {
            prepare,
            cid_keyed,
            to,
            max_error_em,
            quad_to_cubic,
            reverse_direction: !keep_direction,
            post_format,
            subroutinize,
            max_subrs,
            hints: p.bool_or("hints", true)?,
            timestamp,
        })
    }

    /// `cid`: only `keyed` is acted on; `true` is Adobe-Identity-0, a string names
    /// registry-ordering-supplement.
    fn cid(p: &Params<'_>) -> Result<Option<CidCollection>> {
        let Some(cid) = p.opt_object("cid")? else { return Ok(None) };
        let mut keyed = None;
        for (key, value) in cid {
            match (key.as_str(), value) {
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
        Ok(keyed.map(|(registry, ordering, supplement)| CidCollection { registry: registry.to_string(), ordering: ordering.to_string(), supplement }))
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
                let read = file_io::read_input_up_to(Path::new(path), MAX_GOADB_BYTES)?;
                let text = String::from_utf8(read.bytes).map_err(|_| EngineError::Io(format!("{path} is not UTF-8 text")))?;
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
        let c = convert_prepared(font, &self.options(units_per_em(font))?, &steps)?;
        Ok(Converted { output: Output::File(c.bytes), glyphs: c.glyphs, left_behind: c.left_behind })
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
            (_, "ufo") => "ufo",
            _ => "otf",
        }
    }
}

/// `glyphs` and `excludeGlyphs` with the `/CID` items of the reference converter (`/17`,
/// `/100-200`) replaced by glyph id ranges; `None` when neither has one. A CID range keeps the
/// CIDs the font has; a single CID the font lacks is refused, as a missing name is.
fn cid_selectors(p: &Params<'_>, font: &SfntFont) -> Result<Option<Value>> {
    let mut out = serde_json::Map::new();
    let mut by_cid: Option<std::collections::HashMap<u32, u16>> = None;
    for key in ["glyphs", "excludeGlyphs"] {
        let Some(value) = p.get(key) else { continue };
        let items: Vec<&str> = match value {
            Value::String(s) => s.split([',', ' ']).filter(|w| !w.is_empty()).collect(),
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
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
        for item in items {
            let Some(spec) = item.strip_prefix('/') else {
                list.push(Value::String(item.to_string()));
                continue;
            };
            let cid = |t: &str| t.parse::<u32>().ok().filter(|c| *c <= 65_535).ok_or_else(|| p.bad(format!("`{item}` is not a CID or a CID range")));
            let (low, high) = match spec.split_once('-') {
                Some((a, b)) => (cid(a)?, cid(b)?),
                None => {
                    let c = cid(spec)?;
                    let g = map.get(&c).ok_or_else(|| p.bad(format!("the font has no CID {c}")))?;
                    list.push(Value::String(format!("gid{g}")));
                    continue;
                }
            };
            if low > high {
                return Err(p.bad(format!("the range `{item}` runs backwards")));
            }
            // Runs of consecutive glyph ids go in as one range each.
            let mut gids: Vec<u16> = (low..=high).filter_map(|c| map.get(&c).copied()).collect();
            gids.sort_unstable();
            let mut run: Option<(u16, u16)> = None;
            for g in gids.into_iter().map(Some).chain([None]) {
                match (run, g) {
                    (Some((a, b)), Some(g)) if u32::from(b) + 1 == u32::from(g) => run = Some((a, g)),
                    (current, next) => {
                        if let Some((a, b)) = current {
                            list.push(Value::String(format!("gid{a}..gid{b}")));
                        }
                        run = next.map(|g| (g, g));
                    }
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
            let font = import_tree(SfntFont::new(Flavour::TrueType), &root, &ImportOptions::default())?;
            settings.run(p, &font)?
        }
        _ if PsSource::sniff(&file.bytes).is_some() => {
            no_face()?;
            if !settings.prepare.is_empty() || p.get("glyphs").is_some() || p.get("excludeGlyphs").is_some() {
                return Err(EngineError::Unsupported(
                    "glyph selection, a GOADB, a location or overlap removal on a Type 1 font or bare CFF table: convert it to otf first".into(),
                ));
            }
            let ps = read_postscript(&file.bytes)?;
            if settings.to == "ufo" {
                ufo(&ps.font, PrepareOptions::default())?
            } else {
                let c = convert_postscript(&ps, &settings.options(units_per_em(&ps.font))?)?;
                Converted { output: Output::File(c.bytes), glyphs: c.glyphs, left_behind: c.left_behind }
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
    Ok(Converted { output, glyphs: converted.glyphs, left_behind: converted.left_behind })
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
    Ok(Converted { glyphs: decompiled.font.glyphs.len(), output: Output::Folder(files), left_behind })
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
    /// Puts the temporary in place; an existing folder is set aside, then removed.
    fn move_in(&self) -> std::io::Result<()> {
        if !(self.folder && self.existed) {
            return std::fs::rename(&self.temp, &self.target);
        }
        let name = self.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font.ufo".into());
        let aside = self.target.with_file_name(format!(".{name}.{}.old", std::process::id()));
        std::fs::rename(&self.target, &aside)?;
        if let Err(e) = std::fs::rename(&self.temp, &self.target) {
            let _ = std::fs::rename(&aside, &self.target);
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(&aside);
        Ok(())
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
    /// replaced nothing are removed again; an output already replaced keeps its new content, and
    /// the message says how many there are.
    fn commit(mut self) -> Result<()> {
        for (i, s) in self.staged.iter().enumerate() {
            if let Err(e) = s.move_in() {
                let mut replaced = 0;
                for moved in self.staged.iter().take(i) {
                    if moved.existed {
                        replaced += 1;
                    } else {
                        StagedOutput::remove(&moved.target, moved.folder);
                    }
                }
                let kept = if replaced == 0 {
                    "nothing was written".to_string()
                } else {
                    format!("{replaced} existing files before it were already replaced; the new files were removed")
                };
                return Err(EngineError::Io(format!("cannot write {}: {e}; {kept}", s.target.display())));
            }
        }
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
