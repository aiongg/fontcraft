//! The hint settings of a master, or of an instance at a master: zones, stems, TT zones and
//! stems, the blue numbers, the TT numbers and the main master, each read with its scope (font,
//! master, instance) and its default.
//!
//! Defaults follow the handbook (owner decision M10 § 7, question 6): blueScale 0.037 and
//! blueFuzz 1 when the source sets nothing, blueShift 7; TTFBlueFuzz 1, TTFZoneRoundingThreshold
//! 0.09375, TTFMinimumDistance 0.25 px. TT zones default to the master's alignment zones and TT
//! stems to its standard stems when TTFZones / TTFStems are absent. Rows:
//! `hinting.ps.standardStems*`, `hinting.ps.alignmentZones*`, `hinting.ps.blueScale`,
//! `hinting.ps.blueShift`, `hinting.ps.blueFuzz`, `hinting.ps.familyAlignmentZones`,
//! `hinting.ps.masters.*`, `hinting.tt.mainMasterOnly`, `hinting.tt.param.*`,
//! `hinting.tt.zones.defaultFromAlignmentZones`, `hinting.tt.stems.defaultFromStandardStems`,
//! `fontInfo.masters.firstMasterHints`, `fontInfo.stems.autohinterInput` and the M10 parameter
//! rows of `spec/inventory/18-custom-parameters.toml`.
//!
//! Scopes. The handbook's parameter descriptions name no scope for most hinting parameters, so a
//! parameter is read where it is most specific: the instance's (when the settings are for one),
//! else the master's, else the font's, as the compiler reads every other parameter
//! (`fontcraft_model::Font::resolve`; guess on `hinting.ps.blueShift`). The exceptions:
//!
//! | Parameter | Read from |
//! |---|---|
//! | Get Hints From Master | the font (its value is a master id or name) |
//! | TTFZones, TTFStems | the master; TTFZones on the font holds the family zones ([`family_tt_zones`]) |
//! | Autohint | the instance |
//! | blueScale, blueShift, blueFuzz | also under their UFO names (`postscriptBlueScale`…), which the handbook says work alike |
//!
//! TT zones and stems across masters (`hinting.tt.replicateToOtherMasters`; guess on
//! `hinting.tt.zones.name`): the main master's list fixes the order, names, deltas and filters;
//! another master gives each entry its own position and size by name, and an entry it lacks
//! keeps the main master's values.
//!
//! Stored forms of the TT definitions: TTFStems entries are `{horizontal, name, width}` (the
//! corpus); TTFZones entries `{name, position, size, align}` with `pos` also read; both may hold
//! `filter` (predicate text) and `deltas` (`{column: {PPM: shift}}`). No source shows a zone or a
//! delta, so these keys are our choice (guess on `hinting.tt.param.ttfZones`).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::plist::{Dict, Value};
use fontcraft_model::{Font, Instance, Master, MetricDef, metric};

use crate::noderef::{read_flag, read_number, read_numbers};
use crate::{
    HintingError, MAX_CONTROL_TEXT, MAX_CVT_ENTRIES, MAX_DELTA_COLUMNS, MAX_DELTA_PPM, MAX_SOURCE_STEMS, MAX_SOURCE_ZONES, MAX_TT_STEMS, MAX_TT_ZONES,
};

// Parameter names, as the reference spells them.
pub const GET_HINTS_FROM_MASTER: &str = "Get Hints From Master";
pub const FAMILY_ALIGNMENT_ZONES: &str = "Family Alignment Zones";
pub const BLUE_SCALE: &str = "blueScale";
pub const BLUE_SHIFT: &str = "blueShift";
pub const BLUE_FUZZ: &str = "blueFuzz";
pub const AUTOHINT: &str = "Autohint";
pub const DISABLE_AUTOHINTING_FOR_GLYPHS: &str = "Disable autohinting for glyphs";
pub const EXPORT_POSTSCRIPT_HINTS: &str = "Export PostScript Hints";
pub const EXPORT_TRUETYPE_HINTS: &str = "Export TrueType Hints";
pub const TTF_ZONES: &str = "TTFZones";
pub const TTF_STEMS: &str = "TTFStems";
pub const TTF_BLUE_FUZZ: &str = "TTFBlueFuzz";
pub const TTF_OVERSHOOT_SUPPRESSION_BELOW_PPM: &str = "TTFOvershootSuppressionBelowPPM";
pub const TTF_ZONE_ROUNDING_THRESHOLD: &str = "TTFZoneRoundingThreshold";
pub const TTF_MINIMUM_DISTANCE: &str = "TTFMinimumDistance";
pub const TTF_FAMILY_ZONES_THRESHOLD: &str = "TTFFamilyZonesThreshold";
pub const TTF_DONT_PRESERVE_DIAGONALS: &str = "TTFDontPreserveDiagonals";
pub const IGNORE_VERTICAL_HINTS: &str = "Ignore Vertical Hints";
pub const TTF_AUTOHINT_OPTIONS: &str = "TTFAutohint options";
pub const TTF_AUTOHINT_CONTROL_INSTRUCTIONS: &str = "TTFAutohint control instructions";
pub const TTF_AUTOHINT_BINARY_PATH: &str = "TTFAutohint binary path";
pub const CVT_TABLE: &str = "cvt Table";
pub const PREP_TABLE_ASSEMBLY: &str = "prep Table Assembly";
pub const FPGM_TABLE_ASSEMBLY: &str = "fpgm Table Assembly";
pub const GASP_TABLE: &str = "gasp Table";
/// The UFO names of the blue numbers, which the handbook says work beside the short ones.
pub const POSTSCRIPT_BLUE_SCALE: &str = "postscriptBlueScale";
pub const POSTSCRIPT_BLUE_SHIFT: &str = "postscriptBlueShift";
pub const POSTSCRIPT_BLUE_FUZZ: &str = "postscriptBlueFuzz";

/// Ranges of a gasp Table parameter.
pub const MAX_GASP_RANGES: usize = 256;
/// Length of a stored assembly program, a control-instruction text or an option string.
const MAX_TEXT: usize = MAX_CONTROL_TEXT;

/// Layer user-data key of an imported glyph program (assembly text), written when a hinted
/// TrueType font is opened and written back at export while the outline is unchanged.
pub const IMPORTED_GLYPH_PROGRAM_KEY: &str = "com.fontcraft.ttGlyphProgram";
/// Layer user-data key of the outline fingerprint the imported program belongs to.
pub const IMPORTED_OUTLINE_KEY: &str = "com.fontcraft.ttGlyphProgramOutline";

pub const DEFAULT_BLUE_SCALE: f64 = 0.037;
pub const DEFAULT_BLUE_SHIFT: f64 = 7.0;
pub const DEFAULT_BLUE_FUZZ: f64 = 1.0;
pub const DEFAULT_TT_BLUE_FUZZ: f64 = 1.0;
pub const DEFAULT_ZONE_ROUNDING_THRESHOLD: f64 = 0.09375;
pub const DEFAULT_MINIMUM_DISTANCE: f64 = 0.25;

/// A delta cell's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeltaDir {
    Up,
    Down,
}

/// Deltas: master or instance id → PPM (1 to [`crate::MAX_DELTA_PPM`]) → direction. At most
/// [`crate::MAX_DELTA_COLUMNS`] ids.
pub type Deltas = BTreeMap<String, BTreeMap<u16, DeltaDir>>;

/// A PostScript alignment zone: flat edge and signed size (positive: top zone).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PsZone {
    pub position: f64,
    pub size: f64,
}

/// A TT zone (TTFZones entry, or one derived from an alignment zone).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TtZone {
    /// Unique among the master's TT zones. `xHeight` gets the rounding threshold twice.
    pub name: String,
    pub position: f64,
    /// Positive: top zone; negative: bottom zone.
    pub size: f64,
    /// The zone this one is aligned to, by name.
    pub align: Option<String>,
    /// Main master only.
    pub deltas: Deltas,
    /// Main master only: the glyphs the zone applies to, as a glyph predicate.
    pub filter: Option<String>,
}

/// A TT stem (TTFStems entry, or one derived from a standard stem).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TtStem {
    pub name: String,
    /// Measured vertically (the thickness of a horizontal stroke; the editor's Y direction).
    pub horizontal: bool,
    pub size: f64,
    /// Main master only.
    pub deltas: Deltas,
    /// Main master only.
    pub filter: Option<String>,
}

/// Everything hinting reads from a font for one master, or for an instance at a master.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintSettings {
    /// The master whose hints count (Get Hints From Master, else the first master).
    pub main_master: String,
    /// The master these settings are for.
    pub master: String,
    /// The instance, when the settings are for one (its index in `Font::instances`).
    pub instance: Option<usize>,

    // PostScript
    /// Alignment zones (metric overshoots plus the master's extra zones), as written to the
    /// Private dict before the CFF limits are applied.
    pub zones: Vec<PsZone>,
    pub family_zones: Vec<PsZone>,
    /// Standard stems, by direction, in source order.
    pub stems_h: Vec<f64>,
    pub stems_v: Vec<f64>,
    /// `None`: the source sets none; export writes [`DEFAULT_BLUE_SCALE`] and the others.
    pub blue_scale: Option<f64>,
    pub blue_shift: Option<f64>,
    pub blue_fuzz: Option<f64>,
    /// Autohint at export (instance parameter, else the export option).
    pub autohint: Option<bool>,
    pub disable_autohint_glyphs: Vec<String>,
    pub export_ps_hints: bool,

    // TrueType
    pub tt_zones: Vec<TtZone>,
    pub tt_stems: Vec<TtStem>,
    pub tt_blue_fuzz: f64,
    pub tt_overshoot_suppression_below_ppm: Option<u16>,
    pub tt_zone_rounding_threshold: f64,
    pub tt_minimum_distance: f64,
    pub tt_family_zones_threshold: Option<u16>,
    pub tt_dont_preserve_diagonals: bool,
    pub ignore_vertical_hints: bool,
    pub export_tt_hints: bool,
    /// TTFAutohint options and control instructions as stored (parsed by [`crate::tt_auto`] and
    /// [`crate::control`]); the binary path (desktop only).
    pub tt_autohint_options: Option<String>,
    pub tt_autohint_control: Option<String>,
    pub tt_autohint_binary: Option<String>,
    /// Imported hinting kept on the master (cvt Table, prep and fpgm Table Assembly).
    pub cvt: Option<Vec<i16>>,
    pub prep_assembly: Option<String>,
    pub fpgm_assembly: Option<String>,
    /// gasp Table: (maximum PPM, flags), sorted by PPM.
    pub gasp: Option<Vec<(u16, u16)>>,
}

impl HintSettings {
    /// The settings of master `master`.
    pub fn for_master(font: &Font, master: &str) -> Result<Self, HintingError> {
        let m = font.master(master).map_err(|_| HintingError::MissingMaster(master.to_string()))?;
        Self::read(font, m, None)
    }

    /// The settings of instance `instance` built at master `master` (instance parameters
    /// override font and master ones).
    pub fn for_instance(font: &Font, instance: usize, master: &str) -> Result<Self, HintingError> {
        let m = font.master(master).map_err(|_| HintingError::MissingMaster(master.to_string()))?;
        let inst = font
            .instances
            .get(instance)
            .ok_or_else(|| HintingError::BadParameter { name: "instance".to_string(), message: format!("the font has no instance {instance}") })?;
        let mut s = Self::read(font, m, Some(inst))?;
        s.instance = Some(instance);
        Ok(s)
    }

    fn read(font: &Font, m: &Master, inst: Option<&Instance>) -> Result<Self, HintingError> {
        let scope = Scope { font, master: m, instance: inst };
        let main_master = main_master(font);

        let zones = font.alignment_zones(m);
        if zones.len() > MAX_SOURCE_ZONES {
            return Err(HintingError::LimitExceeded { what: "alignment zones in a master", limit: MAX_SOURCE_ZONES });
        }
        let zones: Vec<PsZone> = zones.iter().map(|z| PsZone { position: z.pos, size: z.size }).collect();
        if zones.iter().any(|z| !z.position.is_finite() || !z.size.is_finite()) {
            return Err(HintingError::BadNumber { what: format!("an alignment zone of master `{}`", m.name) });
        }
        if font.stems.len() > MAX_SOURCE_STEMS {
            return Err(HintingError::LimitExceeded { what: "stem definitions", limit: MAX_SOURCE_STEMS });
        }
        let (stems_h, stems_v) = (font.stem_widths(m, true), font.stem_widths(m, false));
        if stems_h.iter().chain(&stems_v).any(|v| !v.is_finite()) {
            return Err(HintingError::BadNumber { what: format!("a stem of master `{}`", m.name) });
        }
        let family_zones = match scope.get(FAMILY_ALIGNMENT_ZONES) {
            Some(v) => read_ps_zones(v).ok_or_else(|| bad_param(FAMILY_ALIGNMENT_ZONES, "it is not a list of zones"))?,
            None => Vec::new(),
        };
        if family_zones.len() > MAX_SOURCE_ZONES {
            return Err(HintingError::LimitExceeded { what: "family alignment zones", limit: MAX_SOURCE_ZONES });
        }

        let (tt_zones, tt_stems) = tt_definitions(font, m, &main_master)?;

        Ok(Self {
            main_master,
            master: m.id.clone(),
            instance: None,
            zones,
            family_zones,
            stems_h,
            stems_v,
            blue_scale: scope.number_of(&[BLUE_SCALE, POSTSCRIPT_BLUE_SCALE])?,
            blue_shift: scope.number_of(&[BLUE_SHIFT, POSTSCRIPT_BLUE_SHIFT])?,
            blue_fuzz: scope.number_of(&[BLUE_FUZZ, POSTSCRIPT_BLUE_FUZZ])?,
            autohint: match inst.and_then(|i| i.params.get(AUTOHINT)) {
                Some(v) => Some(read_flag(v).ok_or_else(|| bad_param(AUTOHINT, "it is not on or off"))?),
                None => None,
            },
            disable_autohint_glyphs: match scope.get(DISABLE_AUTOHINTING_FOR_GLYPHS) {
                Some(v) => glyph_names(v).ok_or_else(|| bad_param(DISABLE_AUTOHINTING_FOR_GLYPHS, "it is not a list of glyph names"))?,
                None => Vec::new(),
            },
            export_ps_hints: scope.flag(EXPORT_POSTSCRIPT_HINTS)?.unwrap_or(true),

            tt_zones,
            tt_stems,
            tt_blue_fuzz: scope.number_of(&[TTF_BLUE_FUZZ])?.unwrap_or(DEFAULT_TT_BLUE_FUZZ),
            tt_overshoot_suppression_below_ppm: scope.ppm(TTF_OVERSHOOT_SUPPRESSION_BELOW_PPM)?,
            tt_zone_rounding_threshold: scope.number_of(&[TTF_ZONE_ROUNDING_THRESHOLD])?.unwrap_or(DEFAULT_ZONE_ROUNDING_THRESHOLD),
            tt_minimum_distance: scope.number_of(&[TTF_MINIMUM_DISTANCE])?.unwrap_or(DEFAULT_MINIMUM_DISTANCE),
            tt_family_zones_threshold: scope.ppm(TTF_FAMILY_ZONES_THRESHOLD)?,
            tt_dont_preserve_diagonals: scope.flag(TTF_DONT_PRESERVE_DIAGONALS)?.unwrap_or(false),
            ignore_vertical_hints: scope.flag(IGNORE_VERTICAL_HINTS)?.unwrap_or(false),
            export_tt_hints: scope.flag(EXPORT_TRUETYPE_HINTS)?.unwrap_or(true),
            tt_autohint_options: scope.text(TTF_AUTOHINT_OPTIONS)?,
            tt_autohint_control: scope.text(TTF_AUTOHINT_CONTROL_INSTRUCTIONS)?,
            tt_autohint_binary: scope.text(TTF_AUTOHINT_BINARY_PATH)?,
            cvt: match scope.get(CVT_TABLE) {
                Some(v) => Some(read_cvt(v)?),
                None => None,
            },
            prep_assembly: scope.text(PREP_TABLE_ASSEMBLY)?,
            fpgm_assembly: scope.text(FPGM_TABLE_ASSEMBLY)?,
            gasp: match scope.get(GASP_TABLE) {
                Some(v) => Some(read_gasp(v)?),
                None => None,
            },
        })
    }

    /// The TT zones that apply to `glyph` (filters evaluated with `fontcraft-glyphquery`). A
    /// filter that cannot be read applies to no glyph; an unknown glyph gets the zones without a
    /// filter.
    pub fn tt_zones_for(&self, font: &Font, glyph: &str) -> Vec<usize> {
        self.in_scope(font, glyph, self.tt_zones.iter().map(|z| z.filter.as_deref()))
    }

    /// The TT stems that apply to `glyph`.
    pub fn tt_stems_for(&self, font: &Font, glyph: &str) -> Vec<usize> {
        self.in_scope(font, glyph, self.tt_stems.iter().map(|z| z.filter.as_deref()))
    }

    fn in_scope<'a>(&self, font: &Font, glyph: &str, filters: impl Iterator<Item = Option<&'a str>>) -> Vec<usize> {
        let g = font.glyph(glyph);
        let env = glyph_data().and_then(|data| fontcraft_glyphquery::Env::new(font, data, Some(&self.master)).ok());
        filters
            .enumerate()
            .filter(|(_, f)| match f {
                None => true,
                Some(text) => match (&g, &env) {
                    (Some(g), Some(env)) => {
                        fontcraft_glyphquery::parse_predicate(text).map(|e| e.matches(&fontcraft_glyphquery::Subject::new(g), env)).unwrap_or(false)
                    }
                    _ => false,
                },
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The BlueScale the export writes: the source's, else [`DEFAULT_BLUE_SCALE`].
    pub fn effective_blue_scale(&self) -> f64 {
        self.blue_scale.unwrap_or(DEFAULT_BLUE_SCALE)
    }

    /// The BlueShift the export writes: the source's, else [`DEFAULT_BLUE_SHIFT`].
    pub fn effective_blue_shift(&self) -> f64 {
        self.blue_shift.unwrap_or(DEFAULT_BLUE_SHIFT)
    }

    /// The BlueFuzz the export writes: the source's, else [`DEFAULT_BLUE_FUZZ`].
    pub fn effective_blue_fuzz(&self) -> f64 {
        self.blue_fuzz.unwrap_or(DEFAULT_BLUE_FUZZ)
    }
}

/// The master whose hints count: Get Hints From Master on the font (a master id, else a master
/// name), else the first master (`fontInfo.masters.firstMasterHints`).
pub fn main_master(font: &Font) -> String {
    let first = font.masters.first().map(|m| m.id.clone()).unwrap_or_default();
    let Some(named) = font.info.params.get(GET_HINTS_FROM_MASTER).and_then(Value::as_str) else { return first };
    font.masters.iter().find(|m| m.id == named).or_else(|| font.masters.iter().find(|m| m.name == named)).map(|m| m.id.clone()).unwrap_or(first)
}

/// The family TT zones: a TTFZones parameter on the font (`hinting.tt.param.ttfZones`); empty
/// when there is none.
pub fn family_tt_zones(font: &Font) -> Result<Vec<TtZone>, HintingError> {
    match font.info.params.get(TTF_ZONES) {
        Some(v) => read_tt_zones(v),
        None => Ok(Vec::new()),
    }
}

/// The TT zones a master without TTFZones uses: its alignment zones, named after their metric
/// (`xHeight` for the x-height, `capHeight`, `ascender`, `baseline`, `descender`, a named
/// metric's name; its other zones `zone 1`, `zone 2`…), with the metric's filter. The names are
/// our choice (guess on `hinting.tt.zones.defaultFromAlignmentZones`).
pub fn derived_tt_zones(font: &Font, master: &Master) -> Vec<TtZone> {
    let mut out: Vec<TtZone> = Vec::new();
    for (def, v) in font.metrics.iter().zip(master.metric_values.iter()) {
        if def.kind == metric::ITALIC_ANGLE || v.over == 0.0 {
            continue;
        }
        let name = unique(&out, zone_name(def), |z| &z.name);
        out.push(TtZone { name, position: v.pos, size: v.over, filter: def.filter.clone(), ..TtZone::default() });
    }
    for (i, z) in master.zones.iter().enumerate() {
        let name = unique(&out, format!("zone {}", i + 1), |z| &z.name);
        out.push(TtZone { name, position: z.pos, size: z.size, ..TtZone::default() });
    }
    out
}

/// The TT stems a master without TTFStems uses: its standard stems, with the stem definitions'
/// names (`stem 1`, `stem 2`… for unnamed ones, as format 2 has), directions and filters.
pub fn derived_tt_stems(font: &Font, master: &Master) -> Vec<TtStem> {
    let mut out: Vec<TtStem> = Vec::new();
    for (i, (def, size)) in font.stems.iter().zip(master.stem_values.iter()).enumerate() {
        let base = if def.name.is_empty() { format!("stem {}", i + 1) } else { def.name.clone() };
        let name = unique(&out, base, |s| &s.name);
        out.push(TtStem { name, horizontal: def.horizontal, size: *size, filter: def.filter.clone(), ..TtStem::default() });
    }
    out
}

fn zone_name(def: &MetricDef) -> String {
    if let Some(n) = def.name.as_deref().filter(|n| !n.is_empty()) {
        return n.to_string();
    }
    match def.kind.as_str() {
        metric::X_HEIGHT => "xHeight".to_string(),
        metric::CAP_HEIGHT => "capHeight".to_string(),
        other => other.to_string(),
    }
}

/// `base`, or `base 2`, `base 3`… when an earlier entry has that name.
fn unique<T>(taken: &[T], base: String, name: impl Fn(&T) -> &String) -> String {
    if !taken.iter().any(|t| *name(t) == base) {
        return base;
    }
    (2..=taken.len() + 1).map(|n| format!("{base} {n}")).find(|c| !taken.iter().any(|t| name(t) == c)).unwrap_or(base)
}

/// A master's TT zones and stems, laid out on the main master's lists.
fn tt_definitions(font: &Font, m: &Master, main: &str) -> Result<(Vec<TtZone>, Vec<TtStem>), HintingError> {
    let own = |m: &Master| -> Result<(Vec<TtZone>, Vec<TtStem>), HintingError> {
        let zones = match m.params.get(TTF_ZONES) {
            Some(v) => read_tt_zones(v)?,
            None => derived_tt_zones(font, m),
        };
        let stems = match m.params.get(TTF_STEMS) {
            Some(v) => read_tt_stems(v)?,
            None => derived_tt_stems(font, m),
        };
        if zones.len() > MAX_TT_ZONES {
            return Err(HintingError::LimitExceeded { what: "TT zones in a master", limit: MAX_TT_ZONES });
        }
        if stems.len() > MAX_TT_STEMS {
            return Err(HintingError::LimitExceeded { what: "TT stems in a master", limit: MAX_TT_STEMS });
        }
        Ok((zones, stems))
    };
    let (zones, stems) = own(m)?;
    let Some(main_m) = font.masters.iter().find(|x| x.id == main).filter(|x| x.id != m.id) else { return Ok((zones, stems)) };
    let (main_zones, main_stems) = own(main_m)?;
    let zones = main_zones
        .into_iter()
        .map(|z| match zones.iter().find(|o| o.name == z.name) {
            Some(o) => TtZone { position: o.position, size: o.size, align: o.align.clone(), ..z },
            None => z,
        })
        .collect();
    let stems = main_stems
        .into_iter()
        .map(|s| match stems.iter().find(|o| o.name == s.name) {
            Some(o) => TtStem { size: o.size, ..s },
            None => s,
        })
        .collect();
    Ok((zones, stems))
}

/// Where one master's or instance's parameters are looked up.
struct Scope<'a> {
    font: &'a Font,
    master: &'a Master,
    instance: Option<&'a Instance>,
}

impl Scope<'_> {
    /// The most specific enabled value: instance, master, font.
    fn get(&self, name: &str) -> Option<&Value> {
        self.instance.and_then(|i| i.params.get(name)).or_else(|| self.master.params.get(name)).or_else(|| self.font.info.params.get(name))
    }

    /// The first of `names` set at the most specific scope.
    fn get_any<'n>(&self, names: &[&'n str]) -> Option<(&'n str, &Value)> {
        let scopes = [self.instance.map(|i| &i.params), Some(&self.master.params), Some(&self.font.info.params)];
        scopes.into_iter().flatten().find_map(|p| names.iter().find_map(|n| p.get(n).map(|v| (*n, v))))
    }

    fn number_of(&self, names: &[&str]) -> Result<Option<f64>, HintingError> {
        match self.get_any(names) {
            Some((name, v)) => read_number(v).map(Some).ok_or_else(|| bad_param(name, "it is not a number")),
            None => Ok(None),
        }
    }

    fn flag(&self, name: &str) -> Result<Option<bool>, HintingError> {
        match self.get(name) {
            Some(v) => read_flag(v).map(Some).ok_or_else(|| bad_param(name, "it is not on or off")),
            None => Ok(None),
        }
    }

    fn ppm(&self, name: &str) -> Result<Option<u16>, HintingError> {
        match self.number_of(&[name])? {
            Some(f) if (0.0..=f64::from(u16::MAX)).contains(&f) => Ok(Some(f.round() as u16)),
            Some(_) => Err(bad_param(name, "it is not a size from 0 to 65535")),
            None => Ok(None),
        }
    }

    fn text(&self, name: &str) -> Result<Option<String>, HintingError> {
        match self.get(name) {
            Some(Value::String(s)) if s.as_str().len() <= MAX_TEXT => Ok(Some(s.as_str().to_string())),
            Some(Value::String(_)) => Err(HintingError::LimitExceeded { what: "bytes of parameter text", limit: MAX_TEXT }),
            Some(_) => Err(bad_param(name, "it is not text")),
            None => Ok(None),
        }
    }
}

fn bad_param(name: &str, message: &str) -> HintingError {
    HintingError::BadParameter { name: name.to_string(), message: message.to_string() }
}

/// Glyph names: a list of strings, or one string of names separated by spaces, commas or lines.
fn glyph_names(v: &Value) -> Option<Vec<String>> {
    let names: Vec<String> = match v {
        Value::Array(a) => a.items.iter().map(|i| i.as_str().map(str::to_string)).collect::<Option<_>>()?,
        Value::String(s) => s.as_str().split(|c: char| c.is_whitespace() || c == ',').filter(|n| !n.is_empty()).map(str::to_string).collect(),
        _ => return None,
    };
    (names.len() <= fontcraft_model::MAX_GLYPHS).then_some(names)
}

/// Zones as dictionaries `{pos, size}` (`position` also read), pairs, or brace strings.
fn read_ps_zones(v: &Value) -> Option<Vec<PsZone>> {
    let items = &v.as_array()?.items;
    if items.len() > MAX_SOURCE_ZONES {
        return None;
    }
    items
        .iter()
        .map(|z| match z {
            Value::Dict(d) => {
                let position = d.get("pos").or_else(|| d.get("position")).and_then(read_number)?;
                let size = d.get("size").and_then(read_number)?;
                Some(PsZone { position, size })
            }
            other => match read_numbers(other).as_deref() {
                Some([position, size]) => Some(PsZone { position: *position, size: *size }),
                _ => None,
            },
        })
        .collect()
}

fn read_tt_zones(v: &Value) -> Result<Vec<TtZone>, HintingError> {
    let bad = |m: &str| bad_param(TTF_ZONES, m);
    let items = &v.as_array().ok_or_else(|| bad("it is not a list"))?.items;
    if items.len() > MAX_TT_ZONES {
        return Err(HintingError::LimitExceeded { what: "TT zones in a master", limit: MAX_TT_ZONES });
    }
    items
        .iter()
        .enumerate()
        .map(|(i, z)| {
            let d = z.as_dict().ok_or_else(|| bad("an entry is not a dictionary"))?;
            Ok(TtZone {
                name: entry_name(d, i, "zone").ok_or_else(|| bad("a zone name is not text"))?,
                position: d
                    .get("position")
                    .or_else(|| d.get("pos"))
                    .map(read_number)
                    .unwrap_or(Some(0.0))
                    .ok_or_else(|| bad("a zone position is not a number"))?,
                size: d.get("size").map(read_number).unwrap_or(Some(0.0)).ok_or_else(|| bad("a zone size is not a number"))?,
                align: d.get("align").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
                deltas: read_definition_deltas(d).ok_or_else(|| bad("the deltas of a zone cannot be read"))?,
                filter: d.get("filter").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
            })
        })
        .collect()
}

fn read_tt_stems(v: &Value) -> Result<Vec<TtStem>, HintingError> {
    let bad = |m: &str| bad_param(TTF_STEMS, m);
    let items = &v.as_array().ok_or_else(|| bad("it is not a list"))?.items;
    if items.len() > MAX_TT_STEMS {
        return Err(HintingError::LimitExceeded { what: "TT stems in a master", limit: MAX_TT_STEMS });
    }
    items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let d = s.as_dict().ok_or_else(|| bad("an entry is not a dictionary"))?;
            Ok(TtStem {
                name: entry_name(d, i, "stem").ok_or_else(|| bad("a stem name is not text"))?,
                horizontal: d.get("horizontal").map(read_flag).unwrap_or(Some(false)).ok_or_else(|| bad("a stem orientation is not a flag"))?,
                size: d
                    .get("width")
                    .or_else(|| d.get("size"))
                    .map(read_number)
                    .unwrap_or(Some(0.0))
                    .ok_or_else(|| bad("a stem width is not a number"))?,
                deltas: read_definition_deltas(d).ok_or_else(|| bad("the deltas of a stem cannot be read"))?,
                filter: d.get("filter").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
            })
        })
        .collect()
}

fn entry_name(d: &Dict, index: usize, what: &str) -> Option<String> {
    match d.get("name") {
        Some(v) => v.as_str().map(|n| n.chars().take(fontcraft_model::MAX_NAME_LEN).collect()),
        None => Some(format!("{what} {}", index + 1)),
    }
}

/// `deltas` of a TT zone or stem: {column: {PPM: shift}}, a positive shift up.
fn read_definition_deltas(d: &Dict) -> Option<Deltas> {
    let mut out = Deltas::new();
    let Some(table) = d.get("deltas") else { return Some(out) };
    let table = table.as_dict()?;
    if table.len() > MAX_DELTA_COLUMNS {
        return None;
    }
    for (column, rows) in &table.entries {
        let rows = rows.as_dict()?;
        let mut cells = BTreeMap::new();
        for (ppm, shift) in rows.entries.iter().take(usize::from(MAX_DELTA_PPM) + 1) {
            let ppm = ppm.as_str().trim().parse::<u16>().ok().filter(|p| (1..=MAX_DELTA_PPM).contains(p))?;
            let shift = read_number(shift)?;
            if shift != 0.0 {
                cells.insert(ppm, if shift > 0.0 { DeltaDir::Up } else { DeltaDir::Down });
            }
        }
        out.insert(column.as_str().to_string(), cells);
    }
    Some(out)
}

/// The cvt Table parameter: a list of numbers, or text of numbers separated by commas or spaces,
/// optionally in brackets; each a whole number in the `cvt ` table's range.
fn read_cvt(v: &Value) -> Result<Vec<i16>, HintingError> {
    let bad = |m: &str| bad_param(CVT_TABLE, m);
    let numbers: Vec<f64> = match v {
        Value::Array(a) => {
            if a.items.len() > MAX_CVT_ENTRIES {
                return Err(HintingError::LimitExceeded { what: "cvt entries", limit: MAX_CVT_ENTRIES });
            }
            a.items.iter().map(read_number).collect::<Option<_>>().ok_or_else(|| bad("an entry is not a number"))?
        }
        Value::String(s) => {
            if s.as_str().len() > MAX_TEXT {
                return Err(HintingError::LimitExceeded { what: "bytes of parameter text", limit: MAX_TEXT });
            }
            let mut out = Vec::new();
            for t in s.as_str().split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '[' | ']' | '{' | '}')).filter(|t| !t.is_empty()) {
                if out.len() == MAX_CVT_ENTRIES {
                    return Err(HintingError::LimitExceeded { what: "cvt entries", limit: MAX_CVT_ENTRIES });
                }
                out.push(t.parse::<f64>().ok().filter(|f| f.is_finite()).ok_or_else(|| bad("an entry is not a number"))?);
            }
            out
        }
        _ => return Err(bad("it is neither a list nor text")),
    };
    numbers
        .into_iter()
        .map(|f| (f.fract() == 0.0 && (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&f)).then_some(f as i16))
        .collect::<Option<_>>()
        .ok_or_else(|| bad("an entry is not a whole number from -32768 to 32767"))
}

/// The gasp Table parameter: {maximum PPM: flags}, sorted by PPM. A range above 65535 is left
/// out, as fontc does; the compiler reports it from the parameter.
fn read_gasp(v: &Value) -> Result<Vec<(u16, u16)>, HintingError> {
    let bad = |m: &str| bad_param(GASP_TABLE, m);
    let d = v.as_dict().ok_or_else(|| bad("it is not a dictionary from sizes to flags"))?;
    if d.len() > MAX_GASP_RANGES {
        return Err(HintingError::LimitExceeded { what: "gasp ranges", limit: MAX_GASP_RANGES });
    }
    let mut out = Vec::new();
    for (ppm, flags) in &d.entries {
        let ppm = ppm.as_str().trim().parse::<u64>().map_err(|_| bad("a range size is not a whole number"))?;
        let flags = crate::noderef::read_int(flags)
            .filter(|f| (0..=i64::from(u16::MAX)).contains(f))
            .ok_or_else(|| bad("a range's flags are not a number from 0 to 65535"))?;
        if let (Ok(ppm), Ok(flags)) = (u16::try_from(ppm), u16::try_from(flags)) {
            out.push((ppm, flags));
        }
    }
    out.sort_unstable();
    out.dedup_by_key(|r| r.0);
    Ok(out)
}

/// The built-in glyph data, read once, for the glyph filters of TT zones and stems.
fn glyph_data() -> Option<&'static GlyphData> {
    static DATA: OnceLock<Option<GlyphData>> = OnceLock::new();
    DATA.get_or_init(|| GlyphData::new().ok()).as_ref()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::openstep;
    use fontcraft_model::{Font, Instance, Master, Param, StemDef};

    use super::*;

    fn value(text: &str) -> Value {
        openstep::parse_value(text).unwrap()
    }

    fn two_masters() -> Font {
        let mut font = Font::new("T");
        let mut bold = font.masters[0].clone();
        bold.id = "m02".into();
        bold.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(bold);
        font
    }

    fn font_param(font: &mut Font, name: &str, v: Value) {
        Arc::make_mut(&mut font.info).params.set(name, v);
    }

    fn master_mut(font: &mut Font, i: usize) -> &mut Master {
        &mut Arc::make_mut(&mut font.masters)[i]
    }

    #[test]
    fn defaults_follow_the_handbook() {
        let font = Font::new("T");
        let s = HintSettings::for_master(&font, "m01").unwrap();
        assert_eq!(s.main_master, "m01");
        assert_eq!((s.blue_scale, s.blue_shift, s.blue_fuzz), (None, None, None));
        assert_eq!((s.effective_blue_scale(), s.effective_blue_shift(), s.effective_blue_fuzz()), (0.037, 7.0, 1.0));
        assert_eq!((s.tt_blue_fuzz, s.tt_zone_rounding_threshold, s.tt_minimum_distance), (1.0, 0.09375, 0.25));
        assert!(s.export_ps_hints && s.export_tt_hints && !s.ignore_vertical_hints && !s.tt_dont_preserve_diagonals);
        assert_eq!((s.autohint, s.gasp.as_ref(), s.cvt.as_ref()), (None, None, None));
        assert_eq!(s.zones.len(), 5);
        let names: Vec<&str> = s.tt_zones.iter().map(|z| z.name.as_str()).collect();
        assert_eq!(names, ["ascender", "capHeight", "xHeight", "baseline", "descender"]);
        assert_eq!((s.tt_zones[2].position, s.tt_zones[2].size), (500.0, 16.0));
        assert!(matches!(HintSettings::for_master(&font, "nope"), Err(HintingError::MissingMaster(_))));
    }

    #[test]
    fn a_parameter_is_read_from_the_instance_then_the_master_then_the_font() {
        let mut font = Font::new("T");
        font_param(&mut font, TTF_BLUE_FUZZ, value("2"));
        assert_eq!(HintSettings::for_master(&font, "m01").unwrap().tt_blue_fuzz, 2.0);
        master_mut(&mut font, 0).params.set(TTF_BLUE_FUZZ, value("3"));
        assert_eq!(HintSettings::for_master(&font, "m01").unwrap().tt_blue_fuzz, 3.0);
        let mut inst = Instance::default();
        inst.params.set(TTF_BLUE_FUZZ, value("4"));
        inst.params.set(AUTOHINT, value("1"));
        Arc::make_mut(&mut font.instances).push(inst);
        let s = HintSettings::for_instance(&font, 0, "m01").unwrap();
        assert_eq!((s.tt_blue_fuzz, s.instance, s.autohint), (4.0, Some(0), Some(true)));
        assert!(HintSettings::for_instance(&font, 5, "m01").is_err());

        // Autohint is an instance parameter only; the UFO name of a blue number works too.
        font_param(&mut font, AUTOHINT, value("1"));
        font_param(&mut font, POSTSCRIPT_BLUE_SCALE, value("0.0625"));
        let s = HintSettings::for_master(&font, "m01").unwrap();
        assert_eq!((s.autohint, s.blue_scale), (None, Some(0.0625)));
    }

    #[test]
    fn get_hints_from_master_names_the_main_master_by_id_or_name() {
        let mut font = two_masters();
        assert_eq!(main_master(&font), "m01");
        font_param(&mut font, GET_HINTS_FROM_MASTER, value("m02"));
        assert_eq!(HintSettings::for_master(&font, "m01").unwrap().main_master, "m02");
        font_param(&mut font, GET_HINTS_FROM_MASTER, value("Bold"));
        assert_eq!(main_master(&font), "m02");
        font_param(&mut font, GET_HINTS_FROM_MASTER, value("Missing"));
        assert_eq!(main_master(&font), "m01", "an unknown master falls back to the first");
    }

    #[test]
    fn other_masters_lay_their_tt_definitions_on_the_main_masters_list_by_name() {
        let mut font = two_masters();
        let main_zones = "({name = xHeight; position = 500; size = 12; deltas = {m01 = {\"9\" = 8;};}; filter = \"case == lower\";}, {name = capHeight; pos = 700; size = 14;})";
        master_mut(&mut font, 0).params.set(TTF_ZONES, value(main_zones));
        master_mut(&mut font, 1)
            .params
            .set(TTF_ZONES, value("({name = capHeight; position = 720; size = 18;}, {name = extra; position = 1; size = 1;})"));
        master_mut(&mut font, 0).params.set(TTF_STEMS, value("({horizontal = 1; name = Thin; width = 16;}, {name = Wide; width = 80;})"));
        master_mut(&mut font, 1).params.set(TTF_STEMS, value("({horizontal = 1; name = Thin; width = 30;})"));

        let main = HintSettings::for_master(&font, "m01").unwrap();
        assert_eq!(main.tt_zones[0].deltas.get("m01").and_then(|r| r.get(&9)), Some(&DeltaDir::Up));
        assert_eq!(main.tt_zones[0].filter.as_deref(), Some("case == lower"));
        assert_eq!((main.tt_zones[1].position, main.tt_stems[0].horizontal, main.tt_stems[1].size), (700.0, true, 80.0));

        let bold = HintSettings::for_master(&font, "m02").unwrap();
        let zones: Vec<(&str, f64)> = bold.tt_zones.iter().map(|z| (z.name.as_str(), z.position)).collect();
        assert_eq!(zones, [("xHeight", 500.0), ("capHeight", 720.0)], "the main master's list; own values by name");
        assert_eq!(bold.tt_zones[0].filter.as_deref(), Some("case == lower"), "filters come from the main master");
        let stems: Vec<(&str, f64)> = bold.tt_stems.iter().map(|s| (s.name.as_str(), s.size)).collect();
        assert_eq!(stems, [("Thin", 30.0), ("Wide", 80.0)]);
    }

    #[test]
    fn tt_stems_default_to_the_standard_stems() {
        let mut font = Font::new("T");
        Arc::make_mut(&mut font.stems).extend([
            StemDef { name: "hStem".into(), horizontal: true, ..StemDef::default() },
            StemDef { name: String::new(), horizontal: false, ..StemDef::default() },
        ]);
        master_mut(&mut font, 0).stem_values = vec![20.0, 80.0];
        let s = HintSettings::for_master(&font, "m01").unwrap();
        assert_eq!((s.stems_h.as_slice(), s.stems_v.as_slice()), ([20.0].as_slice(), [80.0].as_slice()));
        let stems: Vec<(&str, bool, f64)> = s.tt_stems.iter().map(|t| (t.name.as_str(), t.horizontal, t.size)).collect();
        assert_eq!(stems, [("hStem", true, 20.0), ("stem 2", false, 80.0)]);
    }

    #[test]
    fn zone_filters_choose_the_glyphs_a_zone_applies_to() {
        let mut font = Font::new("T");
        master_mut(&mut font, 0).params.set(TTF_ZONES, value("({name = a; position = 0; size = -10;}, {name = b; position = 500; size = 10; filter = \"name beginswith \\\"a\\\"\";}, {name = c; position = 700; size = 10; filter = \"((\";})"));
        let s = HintSettings::for_master(&font, "m01").unwrap();
        assert_eq!(s.tt_zones_for(&font, "a"), [0, 1]);
        assert_eq!(s.tt_zones_for(&font, "b"), [0]);
        assert_eq!(s.tt_zones_for(&font, "missing"), [0]);
    }

    #[test]
    fn gasp_and_cvt_tables_are_read_with_their_limits() {
        let mut font = Font::new("T");
        font_param(&mut font, GASP_TABLE, value("{\"20\" = 7; \"65535\" = 15; \"8\" = 10; \"65536\" = 1;}"));
        master_mut(&mut font, 0).params.set(CVT_TABLE, value("\"[12, -3, 40]\""));
        let s = HintSettings::for_master(&font, "m01").unwrap();
        assert_eq!(s.gasp, Some(vec![(8, 10), (20, 7), (65535, 15)]), "a size above 65535 is left out, as fontc does");
        assert_eq!(s.cvt, Some(vec![12, -3, 40]));
        master_mut(&mut font, 0).params.set(CVT_TABLE, value("(1, 40000)"));
        assert!(matches!(HintSettings::for_master(&font, "m01"), Err(HintingError::BadParameter { .. })));
    }

    #[test]
    fn unreadable_or_oversized_parameters_fail_with_a_message() {
        let cases: [(&str, &str); 6] = [
            (BLUE_SCALE, "abc"),
            (TTF_ZONES, "({name = a; position = x;})"),
            (GASP_TABLE, "{abc = 1;}"),
            (TTF_OVERSHOOT_SUPPRESSION_BELOW_PPM, "70000"),
            (EXPORT_POSTSCRIPT_HINTS, "(1)"),
            (DISABLE_AUTOHINTING_FOR_GLYPHS, "{a = b;}"),
        ];
        for (name, text) in cases {
            let mut font = Font::new("T");
            master_mut(&mut font, 0).params.0.push(Param::new(name, value(text)));
            assert!(HintSettings::for_master(&font, "m01").is_err(), "{name} = {text}");
        }
        let mut font = Font::new("T");
        let many = format!("({})", vec!["{name = z; position = 1; size = 1;}"; MAX_TT_ZONES + 1].join(","));
        master_mut(&mut font, 0).params.set(TTF_ZONES, value(&many));
        assert!(matches!(HintSettings::for_master(&font, "m01"), Err(HintingError::LimitExceeded { .. })));
        let mut font = Font::new("T");
        master_mut(&mut font, 0).zones = vec![fontcraft_model::AlignmentZone { pos: 1.0, size: 1.0 }; 1_000];
        assert!(matches!(HintSettings::for_master(&font, "m01"), Err(HintingError::LimitExceeded { .. })));
        let mut font = Font::new("T");
        master_mut(&mut font, 0).zones = vec![fontcraft_model::AlignmentZone { pos: 1.0, size: f64::INFINITY }];
        assert!(matches!(HintSettings::for_master(&font, "m01"), Err(HintingError::BadNumber { .. })));
    }

    #[test]
    fn family_tt_zones_come_from_the_font() {
        let mut font = Font::new("T");
        assert!(family_tt_zones(&font).unwrap().is_empty());
        font_param(&mut font, TTF_ZONES, value("({name = xHeight; position = 500; size = 10; align = baseline;})"));
        let zones = family_tt_zones(&font).unwrap();
        assert_eq!((zones[0].name.as_str(), zones[0].align.as_deref()), ("xHeight", Some("baseline")));
        let s = HintSettings::for_master(&font, "m01").unwrap();
        assert_eq!(s.tt_zones.len(), 5, "the font's TTFZones are family zones, not the master's");
        font_param(&mut font, DISABLE_AUTOHINTING_FOR_GLYPHS, value("\"a b,c\""));
        assert_eq!(HintSettings::for_master(&font, "m01").unwrap().disable_autohint_glyphs, ["a", "b", "c"]);
    }
}
