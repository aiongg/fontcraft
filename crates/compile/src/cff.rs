//! The `CFF ` table of a static font with PostScript outlines ([`crate::Flavour::Cff`],
//! `docs/spec/40-compile.md` › CFF).
//!
//! What goes into the table is decided as ufo2ft 3.x decides it (`outlineCompiler.py`,
//! `OutlineOTFCompiler.setupTable_CFF` and `getCharStringForGlyph`, with the fallbacks of
//! `fontInfoData.py`; MIT, Copyright (c) 2015-2016 Tal Leming, Ben Kiel, Cosimo Lupo and
//! contributors, see `NOTICE`); [`fontcraft_cff`] writes it byte for byte as fontTools does.
//!
//! | Entry | Value |
//! |---|---|
//! | font name | name ID 6, so the two agree; the family and subfamily as a PostScript name when the `name` table has none |
//! | `version` | `major.minor`, the minor version as a plain number |
//! | `Notice`, `Copyright` | the trademark and the copyright, `©` spelt `Copyright`, then reduced to printable ASCII ([`postscript_string`]); empty when the source has none |
//! | `FullName`, `FamilyName` | the PostScript full name, else the typographic family and subfamily of the `name` table; the typographic family |
//! | `Weight` | the PostScript weight name, only when the source sets it |
//! | `isFixedPitch`, `ItalicAngle`, `UnderlinePosition`, `UnderlineThickness` | as `post` writes them; the angle always as a real number |
//! | `FontMatrix` | `1/upem` (left out at 1000 units per em, where it is the default) |
//! | blue zones | the master's alignment zones as blue values, rounded; Family Alignment Zones (a UFO's family blues) as family blues by the same rule; with any of them, and only then, `BlueFuzz` (1 by default), `BlueShift` (7), `BlueScale` (0.037) and `ForceBold`. The two defaults are the handbook's; the reference compiler writes BlueFuzz 0 and `3 / (4 × tallest zone)` (`docs/spec/40-compile.md` § 4a) |
//! | hints | each glyph's manual PostScript hints, else with Autohint on the autohinter's ([`cff_hints`]), unless Export PostScript Hints is off |
//! | stems | `StemSnapH`/`StemSnapV` sorted and `StdHW`/`StdVW` the first of each list, only when both directions have stems |
//! | widths | `defaultWidthX` and `nominalWidthX` from the source when it sets either, else optimised |
//!
//! Outlines arrive decomposed and in their drawn direction ([`crate::outline`]). Each closed
//! contour loses its closing line back to the start, unless the point before it is already at
//! the start (the reference's segment pens keep that one to keep the duplicate point), and
//! quadratic segments become cubic ones by the exact two-thirds rule of fontTools' `BasePen`.

use fontcraft_cff::{self as cff, Number, PathOp, PrivateDict, TopDict, Widths};
use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_hinting::HintSettings;
use fontcraft_hinting::settings::{DEFAULT_BLUE_FUZZ, DEFAULT_BLUE_SCALE};
use fontcraft_model::plist::{Num, Value};
use fontcraft_model::{AlignmentZone, Font, Resolved, ResolvedGlyph, blue_values, is_ufo_family};
use unicode_normalization::UnicodeNormalization;

use crate::info::{Info, boolean, float, param, text};
use crate::outline::Prepared;
use crate::{CompileError, HintingOptions, cff_hints, ot_round};

/// The rounding tolerance of charstring coordinates: every coordinate becomes an integer, as the
/// reference compiles by default (`compile.cffRoundTolerance`).
pub const ROUND_TOLERANCE: f64 = 0.5;

/// The fraction fontTools' `BasePen` uses to turn a quadratic segment into a cubic one. It is
/// written out, not `2.0 / 3.0`, so the arithmetic is the reference's to the last bit.
const TWO_THIRDS: f64 = 0.666_666_666_666_666_7;

fn table_error(e: cff::Error) -> CompileError {
    CompileError::Table { table: "CFF ", message: e.to_string() }
}

/// Builds the `CFF ` table. `names` are the glyph names in order (the same the `post` table
/// would hold); `synthesised_notdef` says glyph 0 was drawn by the compiler, whose box keeps its
/// explicit closing lines as the reference draws it. `source` is the font `font` was resolved
/// from, which holds the hints and the hint settings; without it the glyphs are unhinted.
/// `hinting` holds the export's Autohint option and whether every hint is left out.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    source: Option<&Font>,
    font: &Resolved,
    info: &Info,
    order: &[ResolvedGlyph],
    outlines: &[Prepared],
    names: &[String],
    synthesised_notdef: bool,
    hinting: &HintingOptions,
    warnings: &mut Vec<String>,
) -> Result<cff::Built, CompileError> {
    let settings = source.and_then(|s| hint_settings(s, font, warnings));
    let private = private_dict(font, settings.as_ref(), warnings);
    // The instance's Autohint parameter, else the export option. Without autohinting the
    // autohinter still finds flex for hand-hinted glyphs, so its zone warnings are dropped.
    let autohint = settings.as_ref().is_some_and(|s| s.autohint.unwrap_or(hinting.autohint));
    let mut dict_warnings = Vec::new();
    let dict = settings
        .as_ref()
        .filter(|s| !hinting.strip && s.export_ps_hints)
        .and_then(|_| cff_hints::autohint_dict(&private, &font.master, &mut dict_warnings));
    if autohint {
        warnings.append(&mut dict_warnings);
    }
    let mut glyphs = Vec::with_capacity(order.len());
    for (gid, ((g, prepared), name)) in order.iter().zip(outlines).zip(names).enumerate() {
        let mut path = match prepared {
            Prepared::Empty => Vec::new(),
            Prepared::Simple(path) => path_ops(path, !(gid == 0 && synthesised_notdef)),
            Prepared::Composite(_) | Prepared::Points(_) => {
                return Err(CompileError::Glyph { glyph: g.name.clone(), message: "a CFF outline cannot hold components or TrueType points".into() });
            }
        };
        let hints = match (source, &settings) {
            (Some(source), Some(settings)) if !hinting.strip => {
                cff_hints::glyph_hints(source, settings, g, &mut path, dict.as_ref(), autohint, warnings)?
            }
            _ => None,
        };
        glyphs.push(cff::Glyph { name: name.clone(), advance: g.width, path, hints });
    }
    let cff_font = cff::Font {
        name: font_name(info, warnings),
        top: top_dict(font, info, warnings),
        private,
        glyphs,
        widths: widths(font),
        round_tolerance: ROUND_TOLERANCE,
        specialise: true,
    };
    cff::build(&cff_font).map_err(table_error)
}

/// The hint settings of the master or instance `font` was resolved at, or `None` with a warning
/// when its hinting parameters cannot be read.
fn hint_settings(source: &Font, font: &Resolved, warnings: &mut Vec<String>) -> Option<HintSettings> {
    let read = match font.instance.filter(|i| source.instances.get(*i).is_some()) {
        Some(i) => HintSettings::for_instance(source, i, &font.master),
        None => HintSettings::for_master(source, &font.master),
    };
    match read {
        Ok(s) => {
            warnings.extend(s.warnings.iter().cloned());
            Some(s)
        }
        Err(e) => {
            warnings.push(format!("the hinting parameters cannot be read ({e}); the CFF table was written without hints"));
            None
        }
    }
}

/// The font name: name ID 6, or, when the `name` table has none, the typographic family and
/// subfamily joined by a hyphen and reduced to what a PostScript name may hold (ufo2ft's
/// `postscriptFontNameFallback`).
fn font_name(info: &Info, warnings: &mut Vec<String>) -> String {
    match info.name(6).filter(|n| !n.is_empty()) {
        Some(n) => latin1(&n, "the PostScript name", warnings),
        None => {
            let (family, subfamily) = typographic_names(info);
            postscript_string(&format!("{family}-{subfamily}")).replace(' ', "")
        }
    }
}

/// The typographic family and subfamily as the `name` table holds them (IDs 16 and 17, or 1 and
/// 2 where the typographic ones equal them and are left out), so the two tables agree.
fn typographic_names(info: &Info) -> (String, String) {
    (info.name(16).or_else(|| info.name(1)).unwrap_or_default(), info.name(17).or_else(|| info.name(2)).unwrap_or_default())
}

/// The drawing steps of `path` as a segment pen receives them (see the module notes).
/// `drop_closing_lines`: leave out each closed contour's line back to its start.
pub fn path_ops(path: &BezPath, drop_closing_lines: bool) -> Vec<PathOp> {
    let mut ops: Vec<PathOp> = Vec::with_capacity(path.elements().len());
    let (mut current, mut start) = (Point::ZERO, Point::ZERO);
    // Where the open contour's steps begin in `ops`.
    let mut contour_at = 0;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                ops.push(PathOp::MoveTo((p.x, p.y)));
                contour_at = ops.len();
                (current, start) = (p, p);
            }
            PathEl::LineTo(p) => {
                ops.push(PathOp::LineTo((p.x, p.y)));
                current = p;
            }
            PathEl::QuadTo(c, p) => {
                let c1 = (current.x + TWO_THIRDS * (c.x - current.x), current.y + TWO_THIRDS * (c.y - current.y));
                let c2 = (p.x + TWO_THIRDS * (c.x - p.x), p.y + TWO_THIRDS * (c.y - p.y));
                ops.push(PathOp::CurveTo(c1, c2, (p.x, p.y)));
                current = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                ops.push(PathOp::CurveTo((c1.x, c1.y), (c2.x, c2.y), (p.x, p.y)));
                current = p;
            }
            PathEl::ClosePath => {
                if drop_closing_lines && ops.len() > contour_at {
                    let before = ops.len().checked_sub(2).and_then(|i| ops.get(i)).map(end_point);
                    let closing = matches!(ops.last(), Some(PathOp::LineTo(p)) if *p == (start.x, start.y));
                    if closing && before != Some((start.x, start.y)) {
                        ops.pop();
                    }
                }
                current = start;
            }
        }
    }
    ops
}

fn end_point(op: &PathOp) -> (f64, f64) {
    match *op {
        PathOp::MoveTo(p) | PathOp::LineTo(p) | PathOp::CurveTo(_, _, p) => p,
    }
}

/// `s` with every character Latin-1 cannot hold replaced by `?`, as a CFF string must be; a
/// warning names what was replaced.
fn latin1(s: &str, what: &str, warnings: &mut Vec<String>) -> String {
    if s.chars().all(|c| u32::from(c) <= 0xFF) {
        return s.to_string();
    }
    warnings.push(format!("{what} `{s}` has characters a CFF table cannot store; they were written as `?`"));
    s.chars().map(|c| if u32::from(c) <= 0xFF { c } else { '?' }).collect()
}

/// `s` with every character outside ASCII replaced by `?`, for the CFF strings that must be
/// ASCII; a warning names what was replaced.
fn ascii(s: &str, what: &str, warnings: &mut Vec<String>) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    warnings.push(format!("{what} `{s}` has characters a CFF table cannot store there; they were written as `?`"));
    s.chars().map(|c| if c.is_ascii() { c } else { '?' }).collect()
}

/// A string reduced to what PostScript strings conventionally hold (ufo2ft's
/// `normalizeStringForPostscript`): the characters `[](){}<>/%` dropped; other characters
/// outside printable ASCII replaced by their compatibility decomposition when that is printable
/// ASCII, else spelt with `?` for each non-ASCII character of it (so `é` becomes `e?`).
pub fn postscript_string(s: &str) -> String {
    let printable = |c: char| (33..127).contains(&u32::from(c));
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "[](){}<>/%".contains(c) {
            continue;
        }
        if printable(c) {
            out.push(c);
            continue;
        }
        let decomposed: String = std::iter::once(c).nfkd().collect();
        if decomposed.chars().all(printable) {
            out.push_str(&decomposed);
        } else {
            out.extend(decomposed.chars().map(|d| if d.is_ascii() { d } else { '?' }));
        }
    }
    out
}

/// Notice and Copyright: `©` spelt out, then [`postscript_string`]; empty when missing.
fn notice(s: Option<&str>) -> String {
    s.map(|s| postscript_string(&s.replace('\u{a9}', "Copyright"))).unwrap_or_default()
}

fn top_dict(font: &Resolved, info: &Info, warnings: &mut Vec<String>) -> TopDict {
    let (family, subfamily) = typographic_names(info);
    // A static instance also reads the plural key, as the handbook describes it.
    let full = text(font, "postscriptFullName", &["postscriptFullName"])
        .or_else(|| font.style.as_ref().and_then(|_| text(font, "postscriptFullNames", &[])))
        .map_or_else(|| format!("{family} {subfamily}"), str::to_string);
    let weight = param(&font.params, &["postscriptWeightName"], |v| v.as_str().map(str::to_string));
    let m = &info.metrics;
    let upm = f64::from(font.units_per_em.max(1));
    let scale = Number::Real(1.0 / upm);
    TopDict {
        version: Some(format!("{}.{}", info.version_major, info.version_minor)),
        notice: Some(notice(text(font, "trademarks", &["trademark"]))),
        copyright: Some(notice(text(font, "copyrights", &["copyright"]))),
        full_name: Some(latin1(&full, "the full name", warnings)),
        family_name: Some(latin1(&family, "the family name", warnings)),
        weight: weight.map(|w| ascii(&w, "the weight name", warnings)),
        is_fixed_pitch: Number::Int(i64::from(info.fixed_pitch)),
        italic_angle: Number::Real(info.italic_angle),
        underline_position: Number::Int(ot_round(m.underline_position) as i64),
        underline_thickness: Number::Int(ot_round(m.underline_thickness) as i64),
        font_matrix: [scale, Number::Int(0), Number::Int(0), scale, Number::Int(0), Number::Int(0)],
        ..TopDict::default()
    }
}

/// Most values read from one list parameter (a Private DICT entry holds at most 48 operands).
const MAX_LIST: usize = 96;

/// A list of finite numbers; a value that is not one is no list.
fn numbers(v: &Value) -> Option<Vec<f64>> {
    v.as_array()?.items.iter().take(MAX_LIST).map(float).collect()
}

/// A number as the source spells it: an integer stays one.
fn number(v: &Value) -> Option<Number> {
    match v {
        Value::Number(n) => match n.value() {
            Num::Int(i) => Some(Number::Int(i)),
            Num::Float(f) if f.is_finite() => Some(Number::Real(f)),
            Num::Float(_) => None,
        },
        Value::String(s) => {
            let s = s.as_str().trim();
            s.parse::<i64>().ok().map(Number::Int).or_else(|| s.parse::<f64>().ok().filter(|f| f.is_finite()).map(Number::Real))
        }
        _ => None,
    }
}

fn rounded(v: &[f64]) -> Vec<Number> {
    v.iter().map(|x| Number::Int(ot_round(*x) as i64)).collect()
}

pub(crate) fn private_dict(font: &Resolved, settings: Option<&HintSettings>, warnings: &mut Vec<String>) -> PrivateDict {
    let params = &font.params;
    let ufo = is_ufo_family(font.format);
    // A UFO's blue values that its reader could not turn into zones are kept as written.
    let (zone_blues, zone_other) = blue_values(&font.zones);
    let blues = param(params, &["postscriptBlueValues"], numbers).unwrap_or(zone_blues);
    let other = param(params, &["postscriptOtherBlues"], numbers).unwrap_or(zone_other);
    let family_zones: Vec<AlignmentZone> =
        settings.map(|s| s.family_zones.iter().map(|z| AlignmentZone { pos: z.position, size: z.size }).collect()).unwrap_or_default();
    let (zone_family, zone_family_other) = blue_values(&family_zones);
    let ufo_list = |key: &str| if ufo { param(params, &[key], numbers) } else { None };
    let family = ufo_list("postscriptFamilyBlues").unwrap_or(zone_family);
    let family_other = ufo_list("postscriptFamilyOtherBlues").unwrap_or(zone_family_other);
    let mut p = PrivateDict::default();
    if [&blues, &other, &family, &family_other].iter().any(|l| !l.is_empty()) {
        p.blue_fuzz = Number::Int(ot_round(param(params, &["blueFuzz", "postscriptBlueFuzz"], float).unwrap_or(DEFAULT_BLUE_FUZZ)) as i64);
        p.blue_shift = Number::Int(ot_round(param(params, &["blueShift", "postscriptBlueShift"], float).unwrap_or(7.0)) as i64);
        p.blue_scale = param(params, &["blueScale", "postscriptBlueScale"], number).unwrap_or(Number::Real(DEFAULT_BLUE_SCALE));
        p.force_bold = Number::Int(i64::from(param(params, &["postscriptForceBold"], boolean).unwrap_or(false)));
        p.blue_values = rounded(&blues);
        p.other_blues = rounded(&other);
        p.family_blues = rounded(&family);
        p.family_other_blues = rounded(&family_other);
        blue_scale_check(&p, warnings);
    }
    let stems_h = param(params, &["postscriptStemSnapH"], numbers).unwrap_or_else(|| font.stems_h.clone());
    let stems_v = param(params, &["postscriptStemSnapV"], numbers).unwrap_or_else(|| font.stems_v.clone());
    if !stems_h.is_empty() && !stems_v.is_empty() {
        let round = |v: &[f64]| -> Vec<i64> { v.iter().map(|x| ot_round(*x) as i64).collect() };
        let (h, v) = (round(&stems_h), round(&stems_v));
        p.std_hw = h.first().map(|n| Number::Int(*n));
        p.std_vw = v.first().map(|n| Number::Int(*n));
        p.stem_snap_h = sorted(h);
        p.stem_snap_v = sorted(v);
    }
    p
}

/// Warns when BlueScale is too large for the tallest alignment zone: the handbook requires
/// (largest point size at 300 dpi − 0.49) × largest zone < 240, and BlueScale is that point size
/// less 0.49, over 240, so BlueScale × largest zone must stay below 1.
fn blue_scale_check(p: &PrivateDict, warnings: &mut Vec<String>) {
    let value = |n: &Number| match *n {
        Number::Int(i) => i as f64,
        Number::Real(r) => r,
    };
    let largest = [&p.blue_values, &p.other_blues, &p.family_blues, &p.family_other_blues]
        .into_iter()
        .flat_map(|list| list.as_chunks::<2>().0.iter())
        .map(|[a, b]| (value(b) - value(a)).abs())
        .fold(0.0, f64::max);
    let scale = value(&p.blue_scale);
    if scale * largest >= 1.0 {
        warnings.push(format!(
            "BlueScale {scale} is too large for the largest alignment zone ({largest} units): BlueScale times the zone must stay below 1, so \
             BlueScale below {:.6}; overshoots in that zone may not be suppressed as intended",
            1.0 / largest
        ));
    }
}

fn sorted(mut v: Vec<i64>) -> Vec<Number> {
    v.sort_unstable();
    v.into_iter().map(Number::Int).collect()
}

/// The widths the source fixes, else optimised (`compile.cffWidthOptimisation`).
fn widths(font: &Resolved) -> Widths {
    let default = param(&font.params, &["postscriptDefaultWidthX"], float);
    let nominal = param(&font.params, &["postscriptNominalWidthX"], float);
    if default.is_none() && nominal.is_none() {
        return Widths::Optimise;
    }
    let clamp = |v: f64| ot_round(v).clamp(-cff::MAX_ADVANCE, cff::MAX_ADVANCE) as i64;
    Widths::Fixed { default_width_x: clamp(default.unwrap_or(200.0)), nominal_width_x: clamp(nominal.unwrap_or(0.0)) }
}
