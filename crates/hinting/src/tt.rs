//! TrueType instructions on a layer: Snap, Stem, Shift, Interpolate and Delta, parsed from and
//! written back to the stored hint values. List order is execution order.
//!
//! Forms read (`docs/plans/M10.md` § 2): `TTStem` (origin and target as node references, as
//! 4-tuples, or `lsb`/`rsb`; `stem` index; `options`), `TTSnap`, `TTShift`, `TTInterpolate`
//! (with `other1`), `TTDelta` (`settings` = `deltaV` per column and PPM); format 2's names
//! `Anchor` (Snap), `Align` (Shift), `Interpolate` and `Delta`; glyphsLib's dictionaries in a UFO
//! glyph's lib. `TTDiagonal` and other types are not instructions here and are kept as stored.
//! Rows: `hinting.tt.hintOrder`, `hinting.tt.rounding.modes`, `hinting.tt.snap.*`,
//! `hinting.tt.stem.*`, `hinting.tt.shift.*`, `hinting.tt.interpolate.*`,
//! `hinting.tt.delta.restrictions`, `hinting.tt.mainMasterOnly`.
//!
//! How the fields are stored (the GlyphsSDK's constants and the format samples):
//!
//! | Field | Stored as |
//! |---|---|
//! | direction | `horizontal = 1` for [`Direction::Vertical`] (the instruction moves points along y, as a horizontal PostScript hint measures y); absent for [`Direction::Horizontal`] |
//! | rounding | the low bits of `options`: 0 round (absent), 1 up, 2 down, 4 no rounding; 3 for "round only in GDI ClearType" is our choice, no source shows it (guess on `hinting.tt.mainMasterOnly`) |
//! | triple | bit 128 of `options`; the flagged Stem instructions form groups of three in list order (guess on `hinting.tt.mainMasterOnly`) |
//! | stem of a Stem, zone of a Snap | `stem`: −2 automatic (absent), −1 none, else the index; that a Snap keeps its zone in `stem` is our choice (guess on `hinting.tt.mainMasterOnly`) |
//! | moved point | `target`; for Snap and Delta, which name one point, `origin` |
//! | reference points | `origin` (Stem, Shift, Interpolate) and `other1` (Interpolate) |
//! | deltas | `settings.deltaV` = {column → {PPM → shift}}; the column key is kept as stored (the format sample uses an instance location, `"{0, 0, 0, 0, 0, 0}"`), a positive shift moves up and a negative one down, `8` and `-8` for new cells (one pixel in eighths; guess on `hinting.tt.delta.restrictions`) |
//!
//! Writing back changes only the keys whose meaning changed, so unknown keys, key order and the
//! source's spellings survive a read and write of an unchanged instruction byte for byte.

use fontcraft_model::plist::{Dict, Str, Value};
use fontcraft_model::{Family, Glyph, Hint, Layer, LayerInfo};

use crate::noderef::{Syntax, number, put, put_if_changed, read_flag, read_int, read_number, remove, type_is};
use crate::ps::splice;
use crate::settings::{DeltaDir, Deltas};
use crate::{HintSettings, HintingError, MAX_DELTA_COLUMNS, MAX_DELTA_PPM, MAX_LAYER_HINTS, TtPoint};

/// The `options` bit of a Stem instruction that belongs to a triple hint.
pub const TRIPLE_FLAG: i64 = 128;
/// The `options` bits that hold the rounding mode.
const ROUNDING_BITS: i64 = 7;
/// Size of a new delta cell: one pixel, in eighths.
const DELTA_STEP: i64 = 8;
/// Columns the format's delta key lists at least (its six axis slots).
const DELTA_KEY_SLOTS: usize = 6;

/// The hinting direction of an instruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Moves points along y (the Instructor's default).
    #[default]
    Vertical,
    /// Moves points along x.
    Horizontal,
}

/// How an instruction rounds (`hinting.tt.rounding.modes`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rounding {
    /// To the nearest pixel edge.
    #[default]
    Round,
    /// To the edge above (vertical) or to the right (horizontal).
    Up,
    /// To the edge below or to the left.
    Down,
    /// No rounding; the point still counts as touched.
    NoRounding,
    /// Round in the GDI ClearType intent, no rounding in the others.
    GdiOnly,
}

/// The zone a Snap uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ZoneRef {
    /// The zone the point lies in, if any.
    #[default]
    Auto,
    NoZone,
    /// Index into the main master's TT zones ([`crate::HintSettings::tt_zones`]).
    Index(usize),
}

/// The stem a Stem instruction uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum StemRef {
    /// The closest TT stem of the direction.
    #[default]
    Auto,
    NoStem,
    /// Index into the main master's TT stems ([`crate::HintSettings::tt_stems`]).
    Index(usize),
}

/// The type of an instruction and what only that type carries.
#[derive(Clone, Debug, PartialEq)]
pub enum TtKind {
    Snap {
        zone: ZoneRef,
    },
    Stem {
        stem: StemRef,
        /// Part of a triple hint: the id the three linked Stem instructions share.
        triple: Option<u32>,
    },
    Shift,
    Interpolate,
    /// Per-PPM corrections, by master or instance id ([`Deltas`]); vertical only.
    Delta {
        cells: Deltas,
    },
}

/// One TrueType instruction.
#[derive(Clone, Debug, PartialEq)]
pub struct TtInstruction {
    pub kind: TtKind,
    pub direction: Direction,
    /// Not used by Delta.
    pub rounding: Rounding,
    /// The point the instruction moves.
    pub target: TtPoint,
    /// The reference point of Stem and Shift; the first reference of Interpolate.
    pub origin: Option<TtPoint>,
    /// The second reference of Interpolate (`other1`).
    pub other: Option<TtPoint>,
    /// The stored value this instruction was read from; [`set_tt_instructions`] writes only the
    /// keys the typed fields cover over it. `None` for a new instruction.
    pub source: Option<Value>,
}

impl TtInstruction {
    /// The default rounding of each type: Snap rounds, the others do not (the handbook's table).
    pub fn default_rounding(kind: &TtKind) -> Rounding {
        match kind {
            TtKind::Snap { .. } => Rounding::Round,
            _ => Rounding::NoRounding,
        }
    }

    /// Every point the instruction names, references first, target last.
    pub fn points(&self) -> Vec<TtPoint> {
        self.origin.iter().chain(&self.other).copied().chain(std::iter::once(self.target)).collect()
    }

    /// A new instruction with the type's default rounding and no reference points.
    pub fn new(kind: TtKind, direction: Direction, target: TtPoint) -> Self {
        let rounding = Self::default_rounding(&kind);
        Self { kind, direction, rounding, target, origin: None, other: None, source: None }
    }

    /// Reads one stored hint; `None` when it is not a TrueType instruction or is detached from
    /// its points (see `attached_kind`). A Stem instruction
    /// of a triple hint reads with group 0 here; [`tt_instructions`] numbers the groups.
    pub fn from_hint(index: usize, hint: &Hint) -> Option<Result<Self, HintingError>> {
        let d = hint.value.as_dict()?;
        let kind = attached_kind(d)?;
        Some(read(index, d, kind).map(|mut t| {
            t.source = Some(hint.value.clone());
            t
        }))
    }

    /// The stored value of this instruction, over [`TtInstruction::source`].
    pub fn to_hint(&self, family: Family) -> Hint {
        let syntax = match &self.source {
            Some(v) => Syntax::of_hint(family, v),
            None if family == Family::Ufo => Syntax::Ufo,
            None => Syntax::Glyphs3,
        };
        self.to_hint_in(family, syntax)
    }

    /// Whether `self` and `other` say the same thing, their stored values aside. Triple groups
    /// compare by membership only, since the stored form keeps no group number.
    pub fn same_as(&self, other: &TtInstruction) -> bool {
        let kinds = match (&self.kind, &other.kind) {
            (TtKind::Stem { stem: a, triple: ta }, TtKind::Stem { stem: b, triple: tb }) => a == b && ta.is_some() == tb.is_some(),
            (a, b) => a == b,
        };
        kinds
            && self.direction == other.direction
            && (self.rounding == other.rounding || matches!(self.kind, TtKind::Delta { .. }))
            && self.target == other.target
            && self.origin == other.origin
            && self.other == other.other
    }

    /// The points the instruction needs, by type: Stem and Shift a reference, Interpolate two.
    fn check_points(&self, index: usize) -> Result<(), HintingError> {
        let bad = |message: &str| -> Result<(), HintingError> { Err(HintingError::BadHint { index, message: message.to_string() }) };
        match self.kind {
            TtKind::Stem { .. } | TtKind::Shift if self.origin.is_none() => bad("the instruction needs a reference point"),
            TtKind::Interpolate if self.origin.is_none() || self.other.is_none() => bad("an Interpolate instruction needs two reference points"),
            _ => Ok(()),
        }
    }

    pub(crate) fn to_hint_in(&self, family: Family, syntax: Syntax) -> Hint {
        let old = self.source.as_ref().and_then(|v| {
            let d = v.as_dict()?;
            read(0, d, attached_kind(d)?).ok()
        });
        if let (Some(v), Some(old)) = (&self.source, &old)
            && old.same_as(self)
        {
            return Hint { family, value: v.clone() };
        }
        let mut d = self.source.as_ref().and_then(|v| v.as_dict().cloned()).unwrap_or_default();
        self.write(&mut d, old.as_ref(), syntax);
        Hint { family, value: Value::Dict(d) }
    }

    fn write(&self, d: &mut Dict, old: Option<&TtInstruction>, syntax: Syntax) {
        let ufo = syntax == Syntax::Ufo;
        if ufo && d.get("name").is_none() {
            put(d, "name", Value::String("".into()));
        }
        let vertical = self.direction == Direction::Vertical;
        match syntax.flag(vertical) {
            Some(v) => put_if_changed(d, "horizontal", v, |x| read_flag(x) == Some(vertical)),
            None => remove(d, "horizontal"),
        }
        let same_type = old.is_some_and(|o| std::mem::discriminant(&o.kind) == std::mem::discriminant(&self.kind));
        if !same_type || d.get("type").is_none() {
            let v2 = syntax == Syntax::Glyphs2;
            let name = match self.kind {
                TtKind::Snap { .. } if v2 => "Anchor",
                TtKind::Snap { .. } => "TTSnap",
                TtKind::Stem { .. } => "TTStem",
                TtKind::Shift if v2 => "Align",
                TtKind::Shift => "TTShift",
                TtKind::Interpolate if v2 => "Interpolate",
                TtKind::Interpolate => "TTInterpolate",
                TtKind::Delta { .. } if v2 => "Delta",
                TtKind::Delta { .. } => "TTDelta",
            };
            put(d, "type", Value::String(name.into()));
        }

        // options: rounding and the triple bit; other bits are kept.
        let stored = d.get("options").and_then(read_int).unwrap_or(0);
        let options = match &self.kind {
            TtKind::Delta { .. } => stored,
            kind => {
                let triple = matches!(kind, TtKind::Stem { triple: Some(_), .. });
                (stored & !(ROUNDING_BITS | TRIPLE_FLAG)) | rounding_code(self.rounding) | if triple { TRIPLE_FLAG } else { 0 }
            }
        };
        if options == 0 && !ufo {
            remove(d, "options");
        } else {
            put_if_changed(d, "options", number(options as f64), |x| read_int(x) == Some(options));
        }

        // stem: the TT stem of a Stem, the zone of a Snap.
        let reference = match &self.kind {
            TtKind::Stem { stem, .. } => Some(match stem {
                StemRef::Auto => -2,
                StemRef::NoStem => -1,
                StemRef::Index(i) => i64::try_from(*i).unwrap_or(-2),
            }),
            TtKind::Snap { zone } => Some(match zone {
                ZoneRef::Auto => -2,
                ZoneRef::NoZone => -1,
                ZoneRef::Index(i) => i64::try_from(*i).unwrap_or(-2),
            }),
            _ => None,
        };
        match reference {
            Some(-2) if !ufo => remove(d, "stem"),
            Some(r) => put_if_changed(d, "stem", number(r as f64), |x| read_int(x) == Some(r)),
            None if ufo && d.get("stem").is_none() => put(d, "stem", number(-2.0)),
            None => {}
        }

        // Points.
        let point = |d: &mut Dict, key: &str, p: Option<TtPoint>| match p {
            Some(p) => {
                let like = d.get(key).cloned().or_else(|| syntax.example());
                let v = p.to_value(like.as_ref());
                put_if_changed(d, key, v, |x| TtPoint::from_value(x) == Some(p));
            }
            None => remove(d, key),
        };
        match self.kind {
            TtKind::Snap { .. } | TtKind::Delta { .. } => {
                point(d, "origin", Some(self.target));
                point(d, "target", None);
                point(d, "other1", None);
            }
            TtKind::Stem { .. } | TtKind::Shift => {
                point(d, "origin", self.origin);
                point(d, "target", Some(self.target));
                point(d, "other1", None);
            }
            TtKind::Interpolate => {
                point(d, "origin", self.origin);
                point(d, "other1", self.other);
                point(d, "target", Some(self.target));
            }
        }

        if let TtKind::Delta { cells } = &self.kind {
            let unchanged = matches!(old.map(|o| &o.kind), Some(TtKind::Delta { cells: c }) if c == cells);
            if !unchanged {
                write_deltas(d, cells);
            }
        }
    }
}

fn rounding_code(r: Rounding) -> i64 {
    match r {
        Rounding::Round => 0,
        Rounding::Up => 1,
        Rounding::Down => 2,
        Rounding::GdiOnly => 3,
        Rounding::NoRounding => 4,
    }
}

fn rounding_of(code: i64) -> Option<Rounding> {
    match code & ROUNDING_BITS {
        0 => Some(Rounding::Round),
        1 => Some(Rounding::Up),
        2 => Some(Rounding::Down),
        3 => Some(Rounding::GdiOnly),
        4 => Some(Rounding::NoRounding),
        _ => None,
    }
}

/// The instruction type a stored hint's `type` names.
fn tt_kind(d: &Dict) -> Option<TtKind> {
    let t = d.get("type")?;
    if type_is(t, &["TTSnap", "TTAnchor", "Anchor"], &[3]) {
        Some(TtKind::Snap { zone: ZoneRef::Auto })
    } else if type_is(t, &["TTStem"], &[4]) {
        Some(TtKind::Stem { stem: StemRef::Auto, triple: None })
    } else if type_is(t, &["TTShift", "TTAlign", "Align"], &[5]) {
        Some(TtKind::Shift)
    } else if type_is(t, &["TTInterpolate", "Interpolate"], &[6]) {
        Some(TtKind::Interpolate)
    } else if type_is(t, &["TTDelta", "Delta"], &[9]) {
        Some(TtKind::Delta { cells: Deltas::new() })
    } else {
        None
    }
}

/// The instruction type of a stored hint that names every point its type needs. A hint of an
/// instruction type without them (the corpus has `place`-only Snap and Stem hints and a Stem
/// without `target`, left when their nodes were deleted) is detached: not an instruction, kept as
/// stored and never compiled (guess on `hinting.tt.mainMasterOnly`).
fn attached_kind(d: &Dict) -> Option<TtKind> {
    let kind = tt_kind(d)?;
    let has = |k: &str| d.get(k).is_some();
    let attached = match kind {
        TtKind::Snap { .. } | TtKind::Delta { .. } => has("origin") || has("target"),
        TtKind::Stem { .. } | TtKind::Shift => has("origin") && has("target"),
        TtKind::Interpolate => has("origin") && has("other1") && has("target"),
    };
    attached.then_some(kind)
}

/// Whether a stored hint is a TrueType instruction of one of the five types, attached to its
/// points (by its keys alone).
pub(crate) fn is_tt(hint: &Hint) -> bool {
    hint.value.as_dict().and_then(attached_kind).is_some()
}

fn read(index: usize, d: &Dict, kind: TtKind) -> Result<TtInstruction, HintingError> {
    let bad = |message: String| HintingError::BadHint { index, message };
    let vertical = match d.get("horizontal") {
        Some(v) => read_flag(v).ok_or_else(|| bad("`horizontal` is not a flag".into()))?,
        None => false,
    };
    let direction = if vertical { Direction::Vertical } else { Direction::Horizontal };
    let options = match d.get("options") {
        Some(v) => read_int(v).ok_or_else(|| bad("`options` is not a whole number".into()))?,
        None => 0,
    };
    let rounding = rounding_of(options).ok_or_else(|| bad(format!("`options` {options} names no rounding mode")))?;
    let reference = match d.get("stem") {
        Some(v) => read_int(v).ok_or_else(|| bad("`stem` is not a whole number".into()))?,
        None => -2,
    };
    let point = |key: &str| -> Result<Option<TtPoint>, HintingError> {
        match d.get(key) {
            Some(v) => TtPoint::from_value(v).map(Some).ok_or_else(|| bad(format!("`{key}` is not a point"))),
            None => Ok(None),
        }
    };
    let need = |p: Option<TtPoint>, key: &str| p.ok_or_else(|| bad(format!("the instruction has no `{key}`")));
    let kind = match kind {
        TtKind::Snap { .. } => TtKind::Snap {
            zone: match reference {
                -1 => ZoneRef::NoZone,
                r => usize::try_from(r).map(ZoneRef::Index).unwrap_or(ZoneRef::Auto),
            },
        },
        TtKind::Stem { .. } => TtKind::Stem {
            stem: match reference {
                -1 => StemRef::NoStem,
                r => usize::try_from(r).map(StemRef::Index).unwrap_or(StemRef::Auto),
            },
            triple: (options & TRIPLE_FLAG != 0).then_some(0),
        },
        TtKind::Delta { .. } => TtKind::Delta { cells: read_deltas(d).map_err(bad)? },
        k => k,
    };
    let (target, origin, other) = match kind {
        TtKind::Snap { .. } | TtKind::Delta { .. } => (need(point("origin")?.or(point("target")?), "origin")?, None, None),
        TtKind::Stem { .. } | TtKind::Shift => (need(point("target")?, "target")?, Some(need(point("origin")?, "origin")?), None),
        TtKind::Interpolate => (need(point("target")?, "target")?, Some(need(point("origin")?, "origin")?), Some(need(point("other1")?, "other1")?)),
    };
    Ok(TtInstruction { kind, direction, rounding, target, origin, other, source: None })
}

/// `settings.deltaV` as cells; an empty table when there is none.
fn read_deltas(d: &Dict) -> Result<Deltas, String> {
    let mut cells = Deltas::new();
    let Some(table) = d.get("settings").and_then(Value::as_dict).and_then(|s| s.get("deltaV")) else { return Ok(cells) };
    let table = table.as_dict().ok_or("`deltaV` is not a dictionary")?;
    if table.len() > MAX_DELTA_COLUMNS {
        return Err(format!("the delta table has more than {MAX_DELTA_COLUMNS} columns"));
    }
    for (column, rows) in &table.entries {
        let rows = rows.as_dict().ok_or("a delta column is not a dictionary")?;
        if rows.len() > usize::from(MAX_DELTA_PPM) {
            return Err(format!("a delta column has more than {MAX_DELTA_PPM} rows"));
        }
        let mut out = std::collections::BTreeMap::new();
        for (ppm, shift) in &rows.entries {
            let ppm = ppm
                .as_str()
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| (1..=MAX_DELTA_PPM).contains(p))
                .ok_or_else(|| format!("delta size `{}` is not a PPM from 1 to {MAX_DELTA_PPM}", ppm.as_str()))?;
            let shift = read_number(shift).ok_or("a delta is not a number")?;
            if shift > 0.0 {
                out.insert(ppm, DeltaDir::Up);
            } else if shift < 0.0 {
                out.insert(ppm, DeltaDir::Down);
            }
        }
        cells.insert(column.as_str().to_string(), out);
    }
    Ok(cells)
}

/// Writes `cells` into `settings.deltaV`, keeping the stored shift of cells whose direction did
/// not change and every other key of `settings`.
fn write_deltas(d: &mut Dict, cells: &Deltas) {
    let mut settings = d.get("settings").and_then(Value::as_dict).cloned().unwrap_or_default();
    let old = settings.get("deltaV").and_then(Value::as_dict).cloned().unwrap_or_default();
    let mut table = Dict::new();
    for (column, rows) in cells {
        let old_rows = old.get(column).and_then(Value::as_dict);
        let mut out = Dict::new();
        for (ppm, dir) in rows {
            let key = ppm.to_string();
            let kept = old_rows.and_then(|r| r.get(&key)).and_then(read_number).filter(|s| (*s > 0.0) == (*dir == DeltaDir::Up) && *s != 0.0);
            let shift = kept.unwrap_or(if *dir == DeltaDir::Up { DELTA_STEP as f64 } else { -(DELTA_STEP as f64) });
            out.entries.push((Str::new(key), number(shift)));
        }
        if !out.is_empty() {
            table.entries.push((Str::new(column.clone()), Value::Dict(out)));
        }
    }
    if table.is_empty() {
        remove(&mut settings, "deltaV");
    } else {
        put(&mut settings, "deltaV", Value::Dict(table));
    }
    if settings.is_empty() {
        remove(d, "settings");
    } else {
        put(d, "settings", Value::Dict(settings));
    }
}

/// The column key of a Delta instruction for an instance at `location`, in the form of the format
/// sample: the axis values in braces, padded with zeros to six (guess on
/// `hinting.tt.mainMasterOnly`).
pub fn delta_column_key(location: &[f64]) -> String {
    let mut values: Vec<String> = location.iter().map(|v| if v.is_finite() { crate::noderef::number_text(*v) } else { "0".to_string() }).collect();
    while values.len() < DELTA_KEY_SLOTS {
        values.push("0".to_string());
    }
    format!("{{{}}}", values.join(", "))
}

/// The layer's TrueType instructions in execution order, each with its index in
/// [`LayerInfo::hints`].
///
/// Stem instructions flagged as part of a triple hint get group ids 0, 1, 2… by threes in list
/// order.
pub fn tt_instructions(info: &LayerInfo) -> Result<Vec<(usize, TtInstruction)>, HintingError> {
    if info.hints.len() > MAX_LAYER_HINTS {
        return Err(HintingError::LimitExceeded { what: "hints on a layer", limit: MAX_LAYER_HINTS });
    }
    let mut out: Vec<(usize, TtInstruction)> =
        info.hints.iter().enumerate().filter_map(|(i, h)| TtInstruction::from_hint(i, h).map(|r| r.map(|t| (i, t)))).collect::<Result<_, _>>()?;
    let mut flagged = 0u32;
    for (_, t) in &mut out {
        if let TtKind::Stem { triple: Some(group), .. } = &mut t.kind {
            *group = flagged / 3;
            flagged = flagged.saturating_add(1);
        }
    }
    Ok(out)
}

/// Replaces the layer's TrueType instructions by `instructions`, in order, at the place of the
/// first old one (at the end when there was none); other stored hints keep their order. More
/// than [`MAX_LAYER_HINTS`] in all is an error and changes nothing.
///
/// Checked first: the reference points each type needs, delta cells at PPM 1 to
/// [`MAX_DELTA_PPM`] in at most [`MAX_DELTA_COLUMNS`] columns. New instructions take the family and
/// syntax of the layer's stored hints, else format 3.
pub fn set_tt_instructions(info: &mut LayerInfo, instructions: &[TtInstruction]) -> Result<(), HintingError> {
    for (i, t) in instructions.iter().enumerate() {
        t.check_points(i)?;
        if let TtKind::Delta { cells } = &t.kind {
            if cells.len() > MAX_DELTA_COLUMNS {
                return Err(HintingError::LimitExceeded { what: "delta columns", limit: MAX_DELTA_COLUMNS });
            }
            if cells.values().flat_map(|rows| rows.keys()).any(|p| !(1..=MAX_DELTA_PPM).contains(p)) {
                return Err(HintingError::BadHint { index: i, message: format!("a delta size is not a PPM from 1 to {MAX_DELTA_PPM}") });
            }
        }
    }
    let (family, syntax) = Syntax::of_layer(info);
    let new: Vec<Hint> =
        instructions.iter().map(|t| t.to_hint_in(family, t.source.as_ref().map(|v| Syntax::of_hint(family, v)).unwrap_or(syntax))).collect();
    splice(info, is_tt, new)
}

/// The layer whose TrueType instructions `glyph` uses where `built` (one of its layers) is
/// exported: a special layer its own (`hinting.ps.masters.specialLayers` applied to TrueType, a
/// guess on `hinting.tt.export.instanceValues`), every master layer the main master's
/// (`hinting.tt.mainMasterOnly`). `None` when that layer has no instruction.
pub fn tt_hint_layer<'a>(glyph: &'a Glyph, settings: &HintSettings, built: &'a Layer) -> Option<&'a Layer> {
    let layer = if built.is_master_layer() { glyph.layer(&settings.main_master)? } else { built };
    layer.info.hints.iter().any(is_tt).then_some(layer)
}

#[cfg(test)]
mod tests {
    use fontcraft_model::plist::openstep::{self, Dialect};
    use fontcraft_model::{Family, Hint, LayerInfo};

    use super::*;
    use crate::NodeRef;

    fn hints(text: &str) -> Vec<Hint> {
        let v = openstep::parse_value(text).unwrap();
        v.as_array().unwrap().items.iter().map(|value| Hint { family: Family::Glyphs, value: value.clone() }).collect()
    }

    fn text(v: &Value) -> String {
        let mut out = String::new();
        openstep::write_value(&mut out, v, Dialect::Glyphs3);
        out
    }

    fn info(hints: Vec<Hint>) -> LayerInfo {
        LayerInfo { hints, ..LayerInfo::default() }
    }

    fn node(p: usize, n: usize) -> TtPoint {
        TtPoint::Node(NodeRef::new(p, n))
    }

    // Every instruction form of the format 3 samples (`A.ss01`, `uni56FD`, `alef-ar`), plus a
    // PostScript hint, which is not an instruction.
    const SAMPLES: &str = r#"(
{
horizontal = 1;
options = 4;
origin = (0,1,1,1);
stem = 1;
target = (0,1,1,3);
type = TTStem;
},
{
horizontal = 1;
origin = (0,1);
type = TTSnap;
},
{
horizontal = 1;
origin = (0,1);
settings = {
deltaV = {
"{0, 0, 0, 0, 0, 0}" = {
"100" = 8;
};
};
};
type = TTDelta;
},
{
options = 4;
origin = (0,0);
other1 = (0,2);
target = (0,1);
type = TTInterpolate;
},
{
horizontal = 1;
options = 2;
origin = (2,3,3,0);
target = (2,3,3,2);
type = TTStem;
},
{
horizontal = 1;
options = 4;
origin = (3,3);
target = (3,0);
type = TTShift;
},
{
horizontal = 1;
options = 4;
origin = (1,3);
stem = -1;
target = (0,3);
type = TTStem;
},
{
options = 4;
origin = lsb;
target = (0,0);
type = TTStem;
},
{
horizontal = 1;
origin = (0,3);
target = down;
type = TopGhost;
}
)"#;

    #[test]
    fn format3_instructions_read_with_points_rounding_and_references_and_write_back_unchanged() {
        let stored = hints(SAMPLES);
        let typed = tt_instructions(&info(stored.clone())).unwrap();
        assert_eq!(typed.len(), 8, "the ghost hint is not an instruction");
        let t: Vec<&TtInstruction> = typed.iter().map(|(_, t)| t).collect();

        assert_eq!(t[0].kind, TtKind::Stem { stem: StemRef::Index(1), triple: None });
        assert_eq!((t[0].direction, t[0].rounding), (Direction::Vertical, Rounding::NoRounding));
        assert_eq!(t[0].origin, Some(TtPoint::Intersection { a: NodeRef::new(0, 1), b: NodeRef::new(1, 1) }));

        assert_eq!((&t[1].kind, t[1].target, t[1].origin, t[1].rounding), (&TtKind::Snap { zone: ZoneRef::Auto }, node(0, 1), None, Rounding::Round));

        let mut cells = Deltas::new();
        cells.insert("{0, 0, 0, 0, 0, 0}".to_string(), [(100, DeltaDir::Up)].into_iter().collect());
        assert_eq!(t[2].kind, TtKind::Delta { cells });
        assert_eq!(t[2].target, node(0, 1));

        assert_eq!((&t[3].kind, t[3].direction), (&TtKind::Interpolate, Direction::Horizontal));
        assert_eq!(t[3].points(), [node(0, 0), node(0, 2), node(0, 1)]);
        assert_eq!(t[4].rounding, Rounding::Down);
        assert_eq!((&t[5].kind, t[5].origin, t[5].target), (&TtKind::Shift, Some(node(3, 3)), node(3, 0)));
        assert_eq!(t[6].kind, TtKind::Stem { stem: StemRef::NoStem, triple: None });
        assert_eq!((t[7].origin, t[7].direction), (Some(TtPoint::Lsb), Direction::Horizontal));

        for (i, inst) in &typed {
            assert_eq!(text(&inst.to_hint(Family::Glyphs).value), text(&stored[*i].value), "instruction {i} is written back byte for byte");
        }
        let mut layer = info(stored.clone());
        let list: Vec<TtInstruction> = typed.into_iter().map(|(_, t)| t).collect();
        set_tt_instructions(&mut layer, &list).unwrap();
        assert_eq!(layer.hints, stored);
    }

    #[test]
    fn new_instructions_read_back_as_written() {
        let mut snap = TtInstruction::new(TtKind::Snap { zone: ZoneRef::Index(2) }, Direction::Vertical, node(0, 4));
        snap.rounding = Rounding::GdiOnly;
        let mut stem = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: Some(0) }, Direction::Horizontal, TtPoint::Rsb);
        stem.origin = Some(node(1, 0));
        let mut interp = TtInstruction::new(TtKind::Interpolate, Direction::Vertical, node(0, 2));
        interp.origin = Some(node(0, 0));
        interp.other = Some(TtPoint::Intersection { a: NodeRef::new(0, 1), b: NodeRef::new(1, 2) });
        let mut cells = Deltas::new();
        cells.insert("{100, 0, 0, 0, 0, 0}".to_string(), [(12, DeltaDir::Down), (13, DeltaDir::Up)].into_iter().collect());
        let delta = TtInstruction::new(TtKind::Delta { cells }, Direction::Vertical, node(0, 3));
        let list = [snap, stem, interp, delta];
        let mut layer = LayerInfo::default();
        set_tt_instructions(&mut layer, &list).unwrap();

        let snap = layer.hints[0].value.as_dict().unwrap();
        assert_eq!(snap.get("options").and_then(read_int), Some(3));
        assert_eq!(snap.get("stem").and_then(read_int), Some(2));
        assert_eq!(text(snap.get("origin").unwrap()), "(0,4)");
        let stem = layer.hints[1].value.as_dict().unwrap();
        assert_eq!(stem.get("options").and_then(read_int), Some(4 | TRIPLE_FLAG));
        assert_eq!(stem.get("target").and_then(Value::as_str), Some("rsb"));
        assert!(stem.get("horizontal").is_none() && stem.get("stem").is_none());
        let delta = layer.hints[3].value.as_dict().unwrap();
        assert_eq!(text(delta.get("settings").unwrap()), "{\ndeltaV = {\n\"{100, 0, 0, 0, 0, 0}\" = {\n\"12\" = -8;\n\"13\" = 8;\n};\n};\n}");

        let reread: Vec<TtInstruction> = tt_instructions(&layer).unwrap().into_iter().map(|(_, t)| t).collect();
        assert_eq!(reread.len(), 4);
        assert!(reread.iter().zip(&list).all(|(a, b)| a.same_as(b)));
    }

    #[test]
    fn changing_the_rounding_keeps_other_option_bits_and_other_keys() {
        let stored = hints("({horizontal = 1; options = 132; origin = (0,1); target = (0,2); type = TTStem; zzz = 1;})");
        let mut t = TtInstruction::from_hint(0, &stored[0]).unwrap().unwrap();
        assert_eq!((t.rounding, &t.kind), (Rounding::NoRounding, &TtKind::Stem { stem: StemRef::Auto, triple: Some(0) }));
        t.rounding = Rounding::Up;
        let d = t.to_hint(Family::Glyphs).value.as_dict().cloned().unwrap();
        assert_eq!(d.get("options").and_then(read_int), Some(129));
        assert!(d.get("zzz").is_some());
    }

    #[test]
    fn triple_stems_are_grouped_by_threes_in_list_order() {
        let one = "{horizontal = 1; options = 132; origin = (0,1); target = (0,2); type = TTStem;}";
        let plain = "{horizontal = 1; options = 4; origin = (0,1); target = (0,2); type = TTStem;}";
        let stored = hints(&format!("({one}, {one}, {plain}, {one}, {one}, {one}, {one})"));
        let groups: Vec<Option<u32>> = tt_instructions(&info(stored))
            .unwrap()
            .into_iter()
            .map(|(_, t)| match t.kind {
                TtKind::Stem { triple, .. } => triple,
                _ => None,
            })
            .collect();
        assert_eq!(groups, [Some(0), Some(0), None, Some(0), Some(1), Some(1), Some(1)]);
    }

    #[test]
    fn set_tt_instructions_refuses_missing_points_and_bad_deltas_and_changes_nothing() {
        let stored = hints("({horizontal = 1; origin = (0,1); type = TTSnap;}, {horizontal = 1; place = (0,10); type = Stem;})");
        let mut layer = info(stored.clone());
        let stem = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Vertical, node(0, 2));
        assert!(matches!(set_tt_instructions(&mut layer, &[stem]), Err(HintingError::BadHint { index: 0, .. })));
        let mut cells = Deltas::new();
        cells.insert("x".to_string(), [(300, DeltaDir::Up)].into_iter().collect());
        let delta = TtInstruction::new(TtKind::Delta { cells }, Direction::Vertical, node(0, 2));
        assert!(set_tt_instructions(&mut layer, &[delta]).is_err());
        assert_eq!(layer.hints, stored);

        let bad = hints("({origin = (0,1); options = 5; type = TTSnap;})");
        assert!(matches!(tt_instructions(&info(bad)), Err(HintingError::BadHint { .. })));
    }

    #[test]
    fn format2_names_and_brace_points_are_read() {
        let stored = hints("({horizontal = 1; origin = \"{0, 1}\"; type = Anchor;}, {origin = \"{lsb}\"; target = \"{0, 3}\"; type = Align;})");
        let typed = tt_instructions(&info(stored.clone())).unwrap();
        assert_eq!(typed[0].1.kind, TtKind::Snap { zone: ZoneRef::Auto });
        assert_eq!((&typed[1].1.kind, typed[1].1.origin), (&TtKind::Shift, Some(TtPoint::Lsb)));
        let mut layer = info(stored);
        let mut list: Vec<TtInstruction> = typed.into_iter().map(|(_, t)| t).collect();
        list.push(TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, node(1, 1)));
        set_tt_instructions(&mut layer, &list).unwrap();
        let new = layer.hints[2].value.as_dict().unwrap();
        assert_eq!((new.get("type").and_then(Value::as_str), new.get("origin").and_then(Value::as_str)), (Some("Anchor"), Some("{1, 1}")));
    }

    #[test]
    fn detached_instructions_are_kept_as_stored() {
        // As the corpus holds them: a Snap and a Stem by position only, a Stem without target.
        let stored = hints(
            "({horizontal = 1; options = 4; place = (-20,0); type = TTSnap;}, {horizontal = 1; options = 4; origin = (1,3); type = TTStem;}, {horizontal = 1; origin = (0,1); type = TTSnap;})",
        );
        let mut layer = info(stored.clone());
        let typed = tt_instructions(&layer).unwrap();
        assert_eq!(typed.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [2]);
        set_tt_instructions(&mut layer, &[]).unwrap();
        assert_eq!(layer.hints, stored[..2].to_vec(), "only the attached instruction is replaced");
    }

    #[test]
    fn delta_column_keys_list_six_axis_values() {
        assert_eq!(delta_column_key(&[]), "{0, 0, 0, 0, 0, 0}");
        assert_eq!(delta_column_key(&[400.0, 87.5]), "{400, 87.5, 0, 0, 0, 0}");
    }
}
