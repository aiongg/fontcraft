//! CID-keyed CFF output (M12.41; rows `compile.cidKeyedFonts`, `compile.cidCmapResources`,
//! `export.cidMapping`, the ROS custom parameter).
//!
//! A source is CID-keyed when it holds a character collection (`fontcraft_model::cid`: the ROS,
//! the glyph-to-CID map, the sub-fonts and their `FDArraySelect` groups), or when its export has
//! the `ROS` custom parameter. The parameter names the collection; a glyph named `cid<number>`
//! takes that CID, another glyph the CID the Unicode CMap file gives its code point, and the rest
//! are appended, as the CID map's left-out glyphs are. Its CFF build then
//! writes a CID-keyed table ([`fontcraft_cff::cid`]) as AFDKO's makeotf 5.0.1 builds one from a
//! CID-keyed UFO (observed on synthetic sources):
//!
//! | Part | What is written |
//! |---|---|
//! | glyph order | `.notdef` as CID 0, then the glyphs in CID order; glyphs the map leaves out, and a second glyph on a CID already taken, follow the highest CID in glyph order, with a warning |
//! | font name | the CID font name, else the PostScript name |
//! | sub-fonts | one Font DICT per sub-font, named by its `FontName`; a glyph in no group goes to sub-font 0, with a warning; a source without sub-fonts has one, named after the font |
//! | Private DICTs | a sub-font's own hinting values under their UFO font-info names, each as given (stem snaps sorted, no StdHW or StdVW), CFF defaults for the rest; a sub-font without a `PrivateDict` takes the font's values; `defaultWidthX` and `nominalWidthX` chosen per sub-font |
//! | hints | each glyph's manual hints, else the autohinter's with its sub-font's zones and stems |
//! | `cmap` | with a Unicode CMap file (makeotf's `-ch`), each glyph gets the code points the file maps to its CID and nothing else; without one, the source's own code points |
//! | Macintosh `cmap` | with a Macintosh CMap file (`-cm`), a format 6 subtable (platform 1) for one-byte codes, or a format 2 subtable laid out as makeotf lays it out for two-byte codes, with the script `-cs` names, else the ordering's (Japan 1, CNS 2, GB 25, Korea 3, else Roman 0), and the language `-cl` names, else 0; without one, a stub format 6 subtable that maps code 0 to glyph 0, as makeotf writes it |
//!
//! The CMap options and the script heuristic follow AFDKO's `python/afdko/makeotf.py` 5.0.1
//! (Apache-2.0, see `NOTICE`).
//!
//! FontCraft ships no CMap files (`docs/plans/M12.md` § 7, question 6 (a)): the caller reads them
//! from the paths the user gives and hands over their bytes in [`CidOptions`]. makeotf refuses
//! to build a CID-keyed font without a Unicode CMap; FontCraft encodes the source's code points
//! instead.

pub mod cmap;
pub mod uvs;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use fontcraft_cff::{self as cff, Number, PrivateDict, Widths};
use fontcraft_model::cid::{CidFont, MAX_CID, Ros};
use fontcraft_model::plist::Dict;
use fontcraft_model::{Font, Params, Resolved, ResolvedGlyph};
use read_fonts::FontRef;
use write_fonts::FontBuilder;
use write_fonts::types::Tag;

pub use cmap::{CMap, CodeKind, MAX_CMAP_BYTES, MAX_CMAP_CODES, parse_cmap};

use crate::cff::{ROUND_TOLERANCE, hint_settings, numbers, path_ops, table_error};
use crate::info::{Info, boolean, float};
use crate::outline::Prepared;
use crate::{CompileError, Compiled, HintingOptions, cff_hints, ot_round};

/// Most names one warning lists.
const LISTED: usize = 5;

/// The custom parameter that makes an export CID-keyed and names its character collection.
pub const ROS_PARAM: &str = "ROS";

/// The CMap files of a CID-keyed build, read by the caller, and the Macintosh subtable's ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CidOptions {
    /// A UTF-32 (or other Unicode) CMap for horizontal glyphs (makeotf's `-ch`).
    pub unicode_cmap: Option<Vec<u8>>,
    /// A Macintosh CMap (`-cm`).
    pub mac_cmap: Option<Vec<u8>>,
    /// The Macintosh subtable's script (`-cs`); `None`: from the ordering.
    pub mac_script: Option<u16>,
    /// The Macintosh subtable's language (`-cl`); `None`: 0.
    pub mac_language: Option<u16>,
    /// A Unicode Variation Sequences file (`-ci`), written as a format 14 subtable ([`uvs`]);
    /// used by every font, CID-keyed or not.
    pub uvs: Option<Vec<u8>>,
}

impl CidOptions {
    fn is_empty(&self) -> bool {
        *self == CidOptions { uvs: self.uvs.clone(), ..CidOptions::default() }
    }
}

/// What the CID stage decided, for the table build and the `cmap` finish.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CidPlan {
    ros: Ros,
    font_name: Option<String>,
    /// Glyph name → CID, every glyph of the ordered list.
    cids: HashMap<String, u32>,
    /// Glyph name → sub-font index.
    fds: HashMap<String, usize>,
    /// Name and stored `PrivateDict` of each sub-font; empty: one sub-font named after the font.
    subfonts: Vec<(String, Option<Dict>)>,
    mac: Option<MacPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct MacPlan {
    script: u16,
    language: u16,
    /// Code (one or two bytes) → CID; empty without a Macintosh CMap, which gives the stub
    /// subtable.
    codes: BTreeMap<u32, u32>,
    /// First bytes of two-byte codes; with any, or with a two-byte code, the subtable is format 2.
    lead_bytes: BTreeSet<u8>,
}

fn list<S: AsRef<str>>(names: &[S]) -> String {
    let shown: Vec<String> = names.iter().take(LISTED).map(|n| format!("`{}`", n.as_ref())).collect();
    let more = names.len().saturating_sub(LISTED);
    if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
}

fn cid_error(message: impl Into<String>) -> CompileError {
    CompileError::Cid(message.into())
}

/// The Macintosh script makeotf picks from an ordering, with a warning for one it does not know.
fn mac_script(ordering: &str, warnings: &mut Vec<String>) -> u16 {
    if ordering.starts_with("Japan") {
        1
    } else if ordering.starts_with("CN") {
        2
    } else if ordering.starts_with("GB") {
        25
    } else if ordering.starts_with("Korea") {
        3
    } else {
        warnings.push(format!("the ordering `{ordering}` names no Macintosh script; the Macintosh cmap subtable uses Roman (0)"));
        0
    }
}

/// Reads a CMap the caller gave and checks it belongs to the font's character collection.
fn read_cmap(bytes: &[u8], kind: CodeKind, what: &str, ros: &Ros, warnings: &mut Vec<String>) -> Result<CMap, CompileError> {
    let cmap = parse_cmap(bytes, kind).map_err(|e| cid_error(format!("the {what} cannot be read: {e}")))?;
    let same = |a: &Option<String>, b: &str| a.as_deref().is_none_or(|a| a == b);
    if !same(&cmap.registry, &ros.registry) || !same(&cmap.ordering, &ros.ordering) {
        return Err(cid_error(format!(
            "the {what} is for {}-{}, not for the font's {}-{}",
            cmap.registry.as_deref().unwrap_or("?"),
            cmap.ordering.as_deref().unwrap_or("?"),
            ros.registry,
            ros.ordering
        )));
    }
    if let Some(u) = &cmap.uses {
        warnings.push(format!("the {what} uses the CMap `{u}`, which is not read; codes only it maps are left out"));
    }
    Ok(cmap)
}

/// The `ROS` parameter of the export, read; a value that is not `Registry-Ordering-Supplement` is
/// reported and left out.
fn ros_param(params: &Params, warnings: &mut Vec<String>) -> Option<Ros> {
    let value = params.get(ROS_PARAM)?;
    let ros = value.as_str().and_then(Ros::parse);
    if ros.is_none() {
        warnings.push(format!("`{ROS_PARAM}` is not Registry-Ordering-Supplement (such as Adobe-Identity-0), so it was left out"));
    }
    ros
}

/// The CID a glyph name states: `cid` and a decimal number, as CID-keyed fonts name their glyphs
/// (`cid01234`).
fn cid_in_name(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("cid")?;
    if digits.is_empty() || digits.len() > 10 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|c| *c <= MAX_CID)
}

/// The CID map of a font that the `ROS` parameter makes CID-keyed: a glyph named `cid<number>`
/// takes that CID; another glyph takes the CID the Unicode CMap gives its first code point the
/// CMap maps. The others are left out, so the CID stage appends them.
fn cids_from_names(glyphs: &[ResolvedGlyph], unicode: Option<&CMap>) -> Vec<(String, u32)> {
    glyphs
        .iter()
        .filter_map(|g| {
            let cid = cid_in_name(&g.name).or_else(|| unicode.and_then(|m| g.unicodes.iter().find_map(|cp| m.cids.get(cp).copied())))?;
            Some((g.name.clone(), cid))
        })
        .collect()
}

/// The CID stage: for a CID-keyed `font`, `glyphs` (in their final order, `.notdef` first) put
/// into CID order, their code points taken from the Unicode CMap when there is one, and the plan
/// the table build follows. The font is CID-keyed when it holds CID-keyed data or the export's
/// `ROS` parameter (`params`) names a collection; the parameter's collection replaces the stored
/// one. An ordinary font keeps its glyphs and has no plan.
pub(crate) fn plan(
    font: &Font,
    params: &Params,
    glyphs: Vec<ResolvedGlyph>,
    options: &CidOptions,
    warnings: &mut Vec<String>,
) -> Result<(Vec<ResolvedGlyph>, Option<CidPlan>), CompileError> {
    let stored = CidFont::read(font).map_err(|e| cid_error(format!("the CID-keyed data of the font cannot be read: {e}")))?;
    let param = ros_param(params, warnings);
    let mut from_param = false;
    let mut state = match (stored, param) {
        (Some(mut state), Some(ros)) => {
            if state.ros != ros {
                warnings.push(format!("`{ROS_PARAM}` names {}, so the font's CID-keyed data for {} is written as {0}", ros.text(), state.ros.text()));
                state.ros = ros;
            }
            state
        }
        (Some(state), None) => state,
        (None, Some(ros)) => {
            from_param = true;
            CidFont { ros, font_name: None, cids: Vec::new(), subfonts: Vec::new() }
        }
        (None, None) => {
            if !options.is_empty() {
                warnings.push("the font is not CID-keyed, so the CMap files and Macintosh cmap ids are not used".into());
            }
            return Ok((glyphs, None));
        }
    };
    let unicode = match &options.unicode_cmap {
        Some(bytes) => Some(read_cmap(bytes, CodeKind::Unicode, "Unicode CMap", &state.ros, warnings)?),
        None => None,
    };
    if from_param {
        state.cids = cids_from_names(&glyphs, unicode.as_ref());
    }
    // The Identity ordering covers no glyph, so appending every glyph is what it asks for.
    let quiet_append = from_param && state.ros.is_identity();
    let map = state.cid_map();
    let mut taken: HashSet<u32> = HashSet::new();
    let mut mapped: Vec<(u32, ResolvedGlyph)> = Vec::with_capacity(glyphs.len());
    let mut unmapped: Vec<ResolvedGlyph> = Vec::new();
    let (mut twice, mut zero) = (Vec::new(), Vec::new());
    for g in glyphs {
        let stored = map.get(g.name.as_str()).copied();
        let cid = if g.name == crate::order::NOTDEF {
            if stored.is_some_and(|c| c != 0) {
                warnings.push(format!("`.notdef` is always CID 0, not CID {}", stored.unwrap_or(0)));
            }
            Some(0)
        } else if stored == Some(0) {
            zero.push(g.name.clone());
            None
        } else {
            stored
        };
        match cid {
            Some(c) if taken.insert(c) => mapped.push((c, g)),
            Some(_) => {
                twice.push(g.name.clone());
                unmapped.push(g);
            }
            None => unmapped.push(g),
        }
    }
    if !zero.is_empty() {
        warnings.push(format!("CID 0 is `.notdef`'s, so {} follow the highest CID", list(&zero)));
    }
    if !twice.is_empty() {
        warnings.push(format!("{} take a CID another glyph has, so they follow the highest CID", list(&twice)));
    }
    let left: Vec<&str> = unmapped.iter().filter(|g| !twice.contains(&g.name) && !zero.contains(&g.name)).map(|g| g.name.as_str()).collect();
    if !left.is_empty() && !quiet_append {
        warnings.push(format!("the CID map leaves out {}, so they follow the highest CID", list(&left)));
    }
    mapped.sort_by_key(|(c, _)| *c);
    let first = mapped.last().map_or(1, |(c, _)| c + 1);
    for (next, g) in (first..).zip(unmapped) {
        if next > MAX_CID {
            return Err(cid_error(format!("`{}` would be CID {next}, above the highest CID, {MAX_CID}", g.name)));
        }
        mapped.push((next, g));
    }

    let mut fds = HashMap::with_capacity(mapped.len());
    let mut homeless = Vec::new();
    for (_, g) in &mapped {
        let fd = state.subfont_of(&g.name).unwrap_or_else(|| {
            if !state.subfonts.is_empty() && g.name != crate::order::NOTDEF {
                homeless.push(g.name.clone());
            }
            0
        });
        fds.insert(g.name.clone(), fd);
    }
    if !homeless.is_empty() {
        warnings.push(format!("{} belong to no sub-font, so they are written with sub-font 0", list(&homeless)));
    }
    let subfonts = state.subfonts.iter().map(|s| (s.name.clone(), s.private().cloned())).collect();

    let mut glyphs: Vec<ResolvedGlyph> = Vec::with_capacity(mapped.len());
    let mut cids = HashMap::with_capacity(mapped.len());
    for (c, g) in mapped {
        cids.insert(g.name.clone(), c);
        glyphs.push(g);
    }
    if let Some(cmap) = &unicode {
        let by_cid = cmap.codes_by_cid();
        for g in &mut glyphs {
            g.unicodes = cids.get(&g.name).and_then(|c| by_cid.get(c)).cloned().unwrap_or_default();
        }
    }
    let mac = match &options.mac_cmap {
        Some(bytes) => {
            let cmap = read_cmap(bytes, CodeKind::Bytes, "Macintosh CMap", &state.ros, warnings)?;
            if cmap.longest_code > 2 {
                warnings.push("the Macintosh CMap has codes longer than two bytes, which no Macintosh cmap subtable holds; the font has none".into());
                None
            } else {
                let script = options.mac_script.unwrap_or_else(|| mac_script(&state.ros.ordering, warnings));
                let lead_bytes = if cmap.longest_code > 1 || !cmap.lead_bytes.is_empty() { cmap.lead_bytes } else { BTreeSet::new() };
                Some(MacPlan { script, language: options.mac_language.unwrap_or(0), codes: cmap.cids, lead_bytes })
            }
        }
        None => {
            // The heuristic's warning is for a CMap the user gave; the stub needs no warning.
            let script = options.mac_script.unwrap_or_else(|| mac_script(&state.ros.ordering, &mut Vec::new()));
            Some(MacPlan { script, language: options.mac_language.unwrap_or(0), codes: BTreeMap::new(), lead_bytes: BTreeSet::new() })
        }
    };
    let plan = CidPlan { ros: state.ros.clone(), font_name: state.font_name.clone(), cids, fds, subfonts, mac };
    Ok((glyphs, Some(plan)))
}

/// The CID of each glyph of `names`: its planned CID, and for a glyph the plan does not know (one
/// the compiler made after the CID stage) the next CID above all before it.
fn final_cids<'a>(plan: &CidPlan, names: impl Iterator<Item = &'a str>) -> Vec<u32> {
    let mut out = Vec::new();
    let mut highest = 0u32;
    for name in names {
        let cid = match plan.cids.get(name) {
            Some(c) if out.is_empty() || *c > highest => *c,
            _ if out.is_empty() => 0,
            _ => highest.saturating_add(1),
        };
        highest = highest.max(cid);
        out.push(cid);
    }
    out
}

/// A sub-font's Private DICT from its stored values under UFO font-info names; each value as
/// given, the CFF defaults for the rest.
fn private_from(dict: &Dict, warnings: &mut Vec<String>, sub: &str) -> (PrivateDict, Widths) {
    let get = |k: &str| dict.get(k);
    let round = |v: Vec<f64>| -> Vec<Number> { v.into_iter().map(|x| Number::Int(ot_round(x) as i64)).collect() };
    let mut p = PrivateDict::default();
    let list = |k: &str| get(k).and_then(numbers).map(round).unwrap_or_default();
    p.blue_values = list("postscriptBlueValues");
    p.other_blues = list("postscriptOtherBlues");
    p.family_blues = list("postscriptFamilyBlues");
    p.family_other_blues = list("postscriptFamilyOtherBlues");
    // StdHW and StdVW have no font-info name, and makeotf writes none for a sub-font.
    let mut h: Vec<i64> = get("postscriptStemSnapH").and_then(numbers).unwrap_or_default().into_iter().map(|x| ot_round(x) as i64).collect();
    let mut v: Vec<i64> = get("postscriptStemSnapV").and_then(numbers).unwrap_or_default().into_iter().map(|x| ot_round(x) as i64).collect();
    h.sort_unstable();
    v.sort_unstable();
    p.stem_snap_h = h.into_iter().map(Number::Int).collect();
    p.stem_snap_v = v.into_iter().map(Number::Int).collect();
    if let Some(n) = get("postscriptBlueScale").and_then(crate::cff::number) {
        p.blue_scale = n;
    }
    if let Some(n) = get("postscriptBlueShift").and_then(float) {
        p.blue_shift = Number::Int(ot_round(n) as i64);
    }
    if let Some(n) = get("postscriptBlueFuzz").and_then(float) {
        p.blue_fuzz = Number::Int(ot_round(n) as i64);
    }
    if let Some(b) = get("postscriptForceBold").and_then(boolean) {
        p.force_bold = Number::Int(i64::from(b));
    }
    match get("LanguageGroup").and_then(float) {
        Some(g) if g == 0.0 || g == 1.0 => p.language_group = Number::Int(g as i64),
        Some(g) => warnings.push(format!("sub-font `{sub}`: LanguageGroup {g} is neither 0 nor 1 and is left out")),
        None => {}
    }
    let default = get("postscriptDefaultWidthX").and_then(float);
    let nominal = get("postscriptNominalWidthX").and_then(float);
    let widths = if default.is_none() && nominal.is_none() {
        Widths::Optimise
    } else {
        let clamp = |v: f64| ot_round(v).clamp(-cff::MAX_ADVANCE, cff::MAX_ADVANCE) as i64;
        Widths::Fixed { default_width_x: clamp(default.unwrap_or(0.0)), nominal_width_x: clamp(nominal.unwrap_or(0.0)) }
    };
    (p, widths)
}

/// Builds the CID-keyed `CFF ` table for `order` as `plan` says; the arguments are those of
/// [`crate::cff::build`]. The returned widths are sub-font 0's.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    plan: &CidPlan,
    source: Option<&Font>,
    font: &Resolved,
    info: &Info,
    order: &[ResolvedGlyph],
    outlines: &[Prepared],
    synthesised_notdef: bool,
    hinting: &HintingOptions,
    warnings: &mut Vec<String>,
) -> Result<cff::Built, CompileError> {
    let settings = source.and_then(|s| hint_settings(s, font, warnings));
    let font_private = crate::cff::private_dict(font, settings.as_ref(), warnings);
    let ps_name = crate::cff::font_name(info, warnings);
    let name = plan.font_name.clone().unwrap_or_else(|| ps_name.clone());
    let mut font_dicts = Vec::with_capacity(plan.subfonts.len().max(1));
    if plan.subfonts.is_empty() {
        font_dicts.push(cff::cid::FontDict { name: name.clone(), private: font_private.clone(), widths: Widths::Optimise });
    }
    for (sub, dict) in &plan.subfonts {
        let (private, widths) = match dict {
            Some(d) => private_from(d, warnings, sub),
            None => (font_private.clone(), Widths::Optimise),
        };
        font_dicts.push(cff::cid::FontDict { name: sub.clone(), private, widths });
    }
    let autohint = settings.as_ref().is_some_and(|s| s.autohint.unwrap_or(hinting.autohint));
    let mut dict_warnings = Vec::new();
    let dicts: Vec<Option<fontcraft_autohint::FdDict>> = font_dicts
        .iter()
        .map(|fd| {
            settings
                .as_ref()
                .filter(|s| !hinting.strip && s.export_ps_hints)
                .and_then(|_| cff_hints::autohint_dict(&fd.private, &font.master, &mut dict_warnings))
        })
        .collect();
    if autohint {
        warnings.append(&mut dict_warnings);
    }
    let cids = final_cids(plan, order.iter().map(|g| g.name.as_str()));
    let mut glyphs = Vec::with_capacity(order.len());
    for (gid, ((g, prepared), cid)) in order.iter().zip(outlines).zip(&cids).enumerate() {
        let mut path = match prepared {
            Prepared::Empty => Vec::new(),
            Prepared::Simple(path) => path_ops(path, !(gid == 0 && synthesised_notdef)),
            Prepared::Composite(_) | Prepared::Points(_) => {
                return Err(CompileError::Glyph { glyph: g.name.clone(), message: "a CFF outline cannot hold components or TrueType points".into() });
            }
        };
        let fd = plan.fds.get(&g.name).copied().filter(|fd| *fd < font_dicts.len()).unwrap_or(0);
        let hints = match (source, &settings) {
            (Some(source), Some(settings)) if !hinting.strip => {
                cff_hints::glyph_hints(source, settings, g, &mut path, dicts.get(fd).and_then(Option::as_ref), autohint, warnings)?
            }
            _ => None,
        };
        glyphs.push(cff::cid::CidGlyph { cid: *cid, fd, glyph: cff::Glyph { name: g.name.clone(), advance: g.width, path, hints } });
    }
    let supplement = i64::from(plan.ros.supplement);
    let cid_font = cff::cid::CidFont {
        name,
        top: crate::cff::top_dict(font, info, warnings),
        ros: cff::cid::Ros { registry: plan.ros.registry.clone(), ordering: plan.ros.ordering.clone(), supplement },
        cid_count: None,
        font_dicts,
        glyphs,
        round_tolerance: ROUND_TOLERANCE,
        specialise: true,
    };
    let built = cff::cid::build(&cid_font).map_err(table_error)?;
    let (default_width_x, nominal_width_x) = built.widths.first().copied().unwrap_or((0, 0));
    Ok(cff::Built { table: built.table, font_bbox: built.font_bbox, default_width_x, nominal_width_x, glyph_bounds: built.glyph_bounds })
}

/// Adds the Macintosh subtable the plan asks for to the built font's `cmap`. Every other table
/// is copied as it is.
pub(crate) fn finish(compiled: &mut Compiled, plan: &CidPlan) -> Result<(), CompileError> {
    let Some(mac) = &plan.mac else { return Ok(()) };
    let table_error = |e: String| CompileError::Table { table: "cmap", message: e };
    let cids = final_cids(plan, compiled.glyph_order.iter().map(String::as_str));
    let gid_of: HashMap<u32, u16> = cids.iter().enumerate().filter_map(|(gid, c)| u16::try_from(gid).ok().map(|g| (*c, g))).collect();
    let font = FontRef::new(&compiled.bytes).map_err(|e| table_error(e.to_string()))?;
    let old = font.table_data(Tag::new(b"cmap")).map(|d| d.as_bytes().to_vec()).ok_or_else(|| table_error("the font has no cmap".into()))?;
    let two_bytes = !mac.lead_bytes.is_empty() || mac.codes.keys().any(|c| *c > 0xFF);
    let subtable = if two_bytes { format2(mac, &gid_of) } else { format6(mac, &gid_of) };
    let Some(subtable) = subtable else {
        // The subtables leave out only codes past two bytes; with a code that reaches a glyph,
        // the subtable did not fit its 16-bit length.
        let reached = mac.codes.iter().any(|(code, cid)| *code <= 0xFFFF && gid_of.get(cid).is_some_and(|g| *g != 0));
        compiled.warnings.push(if reached {
            "the Macintosh cmap subtable would be larger than the 65,535 bytes its format allows, so the font has none".into()
        } else {
            "no code of the Macintosh CMap reaches a glyph of the font, so the font has no Macintosh cmap subtable".into()
        });
        return Ok(());
    };
    let cmap = with_subtable(&old, 1, mac.script, &subtable).map_err(table_error)?;
    let mut builder = FontBuilder::new();
    for record in font.table_directory().table_records() {
        let tag = record.tag();
        if tag == Tag::new(b"cmap") {
            continue;
        }
        if let Some(data) = font.table_data(tag) {
            builder.add_raw(tag, data.as_bytes().to_vec());
        }
    }
    builder.add_raw(Tag::new(b"cmap"), cmap);
    compiled.bytes = builder.build();
    Ok(())
}

/// A format 6 subtable from the first to the last code that reaches a glyph; `None` when no code
/// does. Without codes, the stub: code 0 to glyph 0.
fn format6(mac: &MacPlan, gid_of: &HashMap<u32, u16>) -> Option<Vec<u8>> {
    if mac.codes.is_empty() {
        return Some([6u16, 12, mac.language, 0, 1, 0].iter().flat_map(|v| v.to_be_bytes()).collect());
    }
    let reached: BTreeMap<u32, u16> = mac.codes.iter().filter_map(|(code, cid)| gid_of.get(cid).filter(|g| **g != 0).map(|g| (*code, *g))).collect();
    let (&first, _) = reached.iter().next()?;
    let (&last, _) = reached.iter().next_back()?;
    let count = last.checked_sub(first)?.checked_add(1)?;
    let first = u16::try_from(first).ok()?;
    let count16 = u16::try_from(count).ok()?;
    let length = u16::try_from(10 + 2 * count).ok()?;
    let mut out = Vec::with_capacity(usize::from(length));
    for v in [6, length, mac.language, first, count16] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for code in u32::from(first)..u32::from(first) + count {
        out.extend_from_slice(&reached.get(&code).copied().unwrap_or(0).to_be_bytes());
    }
    Some(out)
}

/// A format 2 subtable for a CMap with two-byte codes, laid out as makeotf 5.0.1 lays it out:
/// sub-header 0 maps the one-byte codes (256 entries); each first byte with codes that reach a
/// glyph has its own sub-header, in ascending order, from its lowest to its highest second byte,
/// with `idDelta` one less than its lowest glyph; the other first bytes of the code space share
/// an empty last sub-header. `None` when no code reaches a glyph or the subtable would be too
/// large.
fn format2(mac: &MacPlan, gid_of: &HashMap<u32, u16>) -> Option<Vec<u8>> {
    let mut single = [0u16; 256];
    let mut leads: BTreeMap<u8, BTreeMap<u8, u16>> = BTreeMap::new();
    for (code, cid) in &mac.codes {
        let Some(&gid) = gid_of.get(cid).filter(|g| **g != 0) else { continue };
        let [_, _, high, low] = code.to_be_bytes();
        if *code <= 0xFF {
            if let Some(slot) = single.get_mut(usize::from(low)) {
                *slot = gid;
            }
        } else if *code <= 0xFFFF {
            leads.entry(high).or_default().insert(low, gid);
        }
    }
    if single.iter().all(|g| *g == 0) && leads.is_empty() {
        return None;
    }
    let empty = mac.lead_bytes.iter().any(|b| !leads.contains_key(b));
    let count = 1 + leads.len() + usize::from(empty);
    let header = 6 + 512;
    let array_start = header + 8 * count;
    let mut keys = [0u16; 256];
    for b in &mac.lead_bytes {
        if let Some(k) = keys.get_mut(usize::from(*b)) {
            *k = u16::try_from(8 * (count - 1)).ok()?;
        }
        if let Some(slot) = single.get_mut(usize::from(*b)) {
            *slot = 0;
        }
    }
    let mut array: Vec<u16> = single.to_vec();
    // (first code, entry count, idDelta, index into the glyph array) per sub-header.
    let mut subs: Vec<(u16, u16, u16, usize)> = vec![(0, 256, 0, 0)];
    for (j, (lead, lows)) in leads.iter().enumerate() {
        // A first byte of two-byte codes is no one-byte code.
        if let Some(a) = array.get_mut(usize::from(*lead)) {
            *a = 0;
        }
        if let Some(k) = keys.get_mut(usize::from(*lead)) {
            *k = u16::try_from(8 * (j + 1)).ok()?;
        }
        let (&first, _) = lows.iter().next()?;
        let (&last, _) = lows.iter().next_back()?;
        let delta = lows.values().min()?.checked_sub(1)?;
        let at = array.len();
        for low in first..=last {
            array.push(lows.get(&low).map_or(0, |g| g.saturating_sub(delta)));
        }
        subs.push((u16::from(first), u16::from(last - first) + 1, delta, at));
    }
    if empty {
        subs.push((0, 0, 0, 0));
    }
    let length = u16::try_from(array_start + 2 * array.len()).ok()?;
    let mut out = Vec::with_capacity(usize::from(length));
    for v in [2, length, mac.language] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for k in keys {
        out.extend_from_slice(&k.to_be_bytes());
    }
    for (j, (first, entries, delta, index)) in subs.iter().enumerate() {
        // idRangeOffset counts from the field itself to the sub-header's first glyph entry.
        let field = header + 8 * j + 6;
        let offset = u16::try_from((array_start + 2 * index).checked_sub(field)?).ok()?;
        for v in [*first, *entries, *delta, offset] {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    for g in array {
        out.extend_from_slice(&g.to_be_bytes());
    }
    Some(out)
}

/// `old` (a `cmap` table) with one more encoding record and its subtable appended. The records
/// stay sorted by platform and encoding; a record with the same ids is an error.
fn with_subtable(old: &[u8], platform: u16, encoding: u16, subtable: &[u8]) -> Result<Vec<u8>, String> {
    let u16_at = |at: usize| old.get(at..at + 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(u16::from_be_bytes);
    let u32_at = |at: usize| old.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes);
    let cut = || "the cmap is cut short".to_string();
    let count = usize::from(u16_at(2).ok_or_else(cut)?);
    let body_at = 4 + 8 * count;
    let body = old.get(body_at..).ok_or_else(cut)?;
    let mut records: Vec<(u16, u16, u32)> = Vec::with_capacity(count + 1);
    for i in 0..count {
        let at = 4 + 8 * i;
        let (p, e, o) = (u16_at(at).ok_or_else(cut)?, u16_at(at + 2).ok_or_else(cut)?, u32_at(at + 4).ok_or_else(cut)?);
        if (p, e) == (platform, encoding) {
            return Err(format!("the cmap already has a subtable for platform {platform}, encoding {encoding}"));
        }
        records.push((p, e, o.checked_add(8).ok_or_else(cut)?));
    }
    let new_at = u32::try_from(body_at + 8 + body.len()).map_err(|_| "the cmap is too large".to_string())?;
    records.push((platform, encoding, new_at));
    records.sort_by_key(|(p, e, _)| (*p, *e));
    let new_count = u16::try_from(records.len()).map_err(|_| "the cmap has too many subtables".to_string())?;
    let mut out = Vec::with_capacity(old.len() + 8 + subtable.len());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&new_count.to_be_bytes());
    for (p, e, o) in records {
        out.extend_from_slice(&p.to_be_bytes());
        out.extend_from_slice(&e.to_be_bytes());
        out.extend_from_slice(&o.to_be_bytes());
    }
    out.extend_from_slice(body);
    out.extend_from_slice(subtable);
    Ok(out)
}

#[cfg(test)]
#[path = "cid/tests.rs"]
mod tests;
