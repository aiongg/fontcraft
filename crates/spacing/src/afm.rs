//! AFM (Adobe Font Metrics, Adobe Technical Note 5004): one master's widths, bounding boxes and
//! kerning as flat glyph pairs, for exchange with other font editors. Row `afm` in
//! `spec/inventory/30-formats.toml`.
//!
//! The format cannot hold kerning groups or metrics keys. [`from_master`] expands every stored
//! pair of a master to the glyph pairs it covers and writes each with the value that applies to
//! it (exceptions included), rounded half up; widths and bounding boxes are rounded half up too.
//! Values stay in the font's own units: nothing is scaled to 1,000 units per em.
//!
//! The file is lines of a keyword and its values. [`write`] gives:
//!
//! ```text
//! StartFontMetrics 4.1
//! FontName, FullName, FamilyName, Weight, ItalicAngle, IsFixedPitch, FontBBox,
//! CapHeight, XHeight, Ascender, Descender   (each when known)
//! StartCharMetrics n
//! C 65 ; WX 600 ; N A ; B 10 0 590 700 ;
//! EndCharMetrics
//! StartKernData
//! StartKernPairs n
//! KPX A V -80
//! EndKernPairs
//! EndKernData
//! EndFontMetrics
//! ```
//!
//! [`read`] takes `C` and `CH` lines (`WX`, `W0X`, `W`, `W0`, `N`, `B`), `KPX` and `KP` lines,
//! and the global keywords above; other keywords (composites, track kerning, `KPY`, `KPH`,
//! comments) are skipped, as the note asks of readers. Lines may end in `\n`, `\r\n` or `\r`.
//!
//! Untrusted input: the text, each line, the number of glyphs and of pairs are capped
//! ([`MAX_TEXT_BYTES`], [`MAX_LINE_BYTES`], [`MAX_CHARS`], [`MAX_PAIRS`]); numbers must be
//! finite and in range. Expanding groups on export is capped by [`MAX_EXPANSION`]. An error names
//! the line (from 1). [`write`] refuses what [`read`] would refuse or read differently, so what it
//! writes always reads back equal.
//!
//! fontTools' `afmLib` (MIT) served as a reference for what common readers expect: whole
//! numbers, a `B` field on every `C` line, and `KPX` pairs.

use std::collections::HashSet;
use std::ops::RangeInclusive;

use fontcraft_geom::Shape as _;
use fontcraft_model::{Font, MAX_COORDINATE, MAX_GLYPHS, MAX_NAME_LEN, metric};

use crate::kerning::pairs::stored_pairs;
use crate::kerning::{Direction, KernIndex, Position, Side, facing};
use crate::{WIDTH_RANGE, kerning_text, metrics_source, round_half_up};

/// Largest AFM text, in bytes: room for [`MAX_PAIRS`] pair lines of about 30 bytes.
pub const MAX_TEXT_BYTES: usize = 64 * 1024 * 1024;
/// Longest line, in bytes, without the line break.
pub const MAX_LINE_BYTES: usize = kerning_text::MAX_LINE_BYTES;
/// Most kerning pairs one AFM text may hold: AFM lists every covered glyph pair, so a large
/// font needs many more than kerning-text does. Opening a compiled font takes as many.
pub const MAX_PAIRS: usize = 2_000_000;
/// Most glyphs one AFM text may hold.
pub const MAX_CHARS: usize = MAX_GLYPHS;
/// Most glyph pairs [`covered_pairs`] looks at (counting a pair once for every stored pair that
/// covers it) before it gives up.
pub const MAX_EXPANSION: usize = 50_000_000;
/// The kerning values AFM pairs may have.
pub const VALUE_RANGE: RangeInclusive<f64> = kerning_text::VALUE_RANGE;
/// The coordinates bounding boxes and global metrics may have.
pub const COORDINATE_RANGE: RangeInclusive<f64> = -MAX_COORDINATE..=MAX_COORDINATE;
/// The character codes a `C` line may give (`-1`: not encoded).
pub const CODE_RANGE: RangeInclusive<i64> = -1..=0x10_FFFF;
/// The first keyword of an AFM text.
pub const START: &str = "StartFontMetrics";
/// The format version [`write`] states.
pub const VERSION: &str = "4.1";

/// A parsed AFM text: the parts FontCraft reads and writes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Afm {
    pub font_name: Option<String>,
    pub full_name: Option<String>,
    pub family_name: Option<String>,
    pub weight: Option<String>,
    pub italic_angle: Option<f64>,
    pub is_fixed_pitch: Option<bool>,
    /// `[x0, y0, x1, y1]`.
    pub font_bbox: Option<[f64; 4]>,
    pub cap_height: Option<f64>,
    pub x_height: Option<f64>,
    pub ascender: Option<f64>,
    pub descender: Option<f64>,
    /// In text order.
    pub chars: Vec<CharMetric>,
    /// In text order.
    pub pairs: Vec<KernPair>,
}

/// One glyph's metrics.
#[derive(Clone, Debug, PartialEq)]
pub struct CharMetric {
    /// The character code, `-1` when the glyph is not encoded.
    pub code: i64,
    pub name: String,
    pub width: f64,
    /// `[x0, y0, x1, y1]`; all zero for a glyph without an outline.
    pub bbox: Option<[f64; 4]>,
}

impl CharMetric {
    /// The left and right sidebearings the bounding box gives; `None` without a box or for an
    /// all-zero box (a glyph without an outline).
    pub fn sidebearings(&self) -> Option<(f64, f64)> {
        let b = self.bbox?;
        if b == [0.0; 4] {
            return None;
        }
        let [x0, _, x1, _] = b;
        Some((x0, self.width - x1))
    }
}

/// A kerning pair of two glyphs.
#[derive(Clone, Debug, PartialEq)]
pub struct KernPair {
    pub left: String,
    pub right: String,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ReadError {
    #[error("the AFM text is {0} bytes long; the limit is {MAX_TEXT_BYTES}")]
    TooLarge(usize),
    #[error("line {line} is {len} bytes long; the limit is {MAX_LINE_BYTES}")]
    LineTooLong { line: usize, len: usize },
    #[error("line {line}: {message}")]
    Bad { line: usize, message: String },
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum WriteError {
    #[error("{what} `{name}` cannot be written to AFM: {problem}")]
    Name { what: &'static str, name: String, problem: String },
    #[error("{what} is {value}, which AFM cannot hold here (a finite number from {} to {})", .range.start(), .range.end())]
    Number { what: String, value: f64, range: RangeInclusive<f64> },
    #[error("there are {0} pairs; an AFM text holds at most {MAX_PAIRS}")]
    TooManyPairs(usize),
    #[error("there are {0} glyphs; an AFM text holds at most {MAX_CHARS}")]
    TooManyChars(usize),
    #[error("{0} would make a line longer than {MAX_LINE_BYTES} bytes")]
    LineTooLong(String),
    #[error("the AFM text would be longer than {MAX_TEXT_BYTES} bytes")]
    TooLarge,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ExportError {
    #[error("the font has no master `{0}`")]
    NoMaster(String),
    #[error("the stored pairs cover more than {MAX_PAIRS} glyph pairs; AFM export takes at most that many")]
    TooManyPairs,
    #[error("the stored pairs cover too many glyph pairs to expand (more than {MAX_EXPANSION} looked at)")]
    TooMuchWork,
}

// ---------- from a font ----------

/// Every pair of glyphs a stored pair of `master` covers in `direction` (groups expanded to
/// their members on the side that faces the pair), each once, with the value that applies to it
/// as [`KernIndex`] looks it up. A linked master gives the pairs of the master it follows. In
/// glyph order of the first glyph, then of the second.
pub fn covered_pairs(font: &Font, direction: Direction, master: &str) -> Result<Vec<KernPair>, ExportError> {
    let index = KernIndex::new(font);
    let table = index.table(direction, master).map_err(|_| ExportError::NoMaster(master.to_string()))?;
    let source = table.master();
    let expand = |side: Side, position: Position| -> Vec<usize> {
        match side {
            Side::Glyph(g) => index.glyph_id(&g).into_iter().collect(),
            Side::Group(g) => index.members(facing(direction, position), &g).filter_map(|n| index.glyph_id(n)).collect(),
        }
    };
    let mut covered: HashSet<(usize, usize)> = HashSet::new();
    let mut work = 0usize;
    for pair in stored_pairs(font, direction, source) {
        let firsts = expand(pair.left(), Position::First);
        let seconds = expand(pair.right(), Position::Second);
        work = work.saturating_add(firsts.len().saturating_mul(seconds.len()));
        if work > MAX_EXPANSION {
            return Err(ExportError::TooMuchWork);
        }
        for a in &firsts {
            for b in &seconds {
                covered.insert((*a, *b));
            }
        }
        if covered.len() > MAX_PAIRS {
            return Err(ExportError::TooManyPairs);
        }
    }
    let mut covered: Vec<(usize, usize)> = covered.into_iter().collect();
    covered.sort_unstable();
    let glyphs = font.glyphs.as_arcs();
    let name = |i: usize| glyphs.get(i).map(|g| g.name.clone());
    Ok(covered
        .into_iter()
        .filter_map(|(a, b)| {
            let applied = table.lookup(a, b)?;
            Some(KernPair { left: name(a)?, right: name(b)?, value: applied.value })
        })
        .collect())
}

/// `master`'s metrics as AFM: the names from the font and master, the master's vertical metrics,
/// every glyph with a layer of the master (width and bounding box, rounded half up; the code of
/// its first Unicode value up to 255, else `-1`) and its left-to-right kerning expanded by
/// [`covered_pairs`], values rounded half up. A linked master gives the widths and kerning of the
/// master it follows.
pub fn from_master(font: &Font, master: &str) -> Result<Afm, ExportError> {
    let own = font.masters.iter().find(|m| m.id == master).ok_or_else(|| ExportError::NoMaster(master.to_string()))?;
    let source = metrics_source(font, master).unwrap_or(own);
    let mut chars = Vec::new();
    let mut font_bbox: Option<[f64; 4]> = None;
    for g in font.glyphs.iter() {
        let Some(layer) = g.layer(&source.id) else { continue };
        let bbox = crate::measure::outline(font, layer)
            .ok()
            .filter(|p| !p.elements().is_empty())
            .map(|p| p.bounding_box())
            .map(|r| [r.x0, r.y0, r.x1, r.y1].map(round_half_up))
            .filter(|b| b.iter().all(|v| v.is_finite() && COORDINATE_RANGE.contains(v)));
        if let Some(b) = bbox {
            font_bbox = Some(match font_bbox {
                None => b,
                Some([f0, f1, f2, f3]) => {
                    let [b0, b1, b2, b3] = b;
                    [f0.min(b0), f1.min(b1), f2.max(b2), f3.max(b3)]
                }
            });
        }
        let code = g.unicodes.first().map(|u| i64::from(*u)).filter(|u| *u <= 255).unwrap_or(-1);
        chars.push(CharMetric { code, name: g.name.clone(), width: round_half_up(layer.width), bbox: Some(bbox.unwrap_or([0.0; 4])) });
    }
    let pairs = covered_pairs(font, Direction::Ltr, &source.id)?.into_iter().map(|p| KernPair { value: round_half_up(p.value), ..p }).collect();
    let vertical = |kind: &str| font.metric(own, kind).map(|v| round_half_up(v.pos)).filter(|v| COORDINATE_RANGE.contains(v));
    let no_space = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let family = font.family_name.trim();
    let style = own.name.trim();
    let text = |s: String| (!s.trim().is_empty()).then_some(s);
    Ok(Afm {
        font_name: text(match (no_space(family).is_empty(), no_space(style).is_empty()) {
            (false, false) => format!("{}-{}", no_space(family), no_space(style)),
            (false, true) => no_space(family),
            (true, _) => no_space(style),
        }),
        full_name: text(format!("{family} {style}").trim().to_string()),
        family_name: text(family.to_string()),
        weight: text(style.to_string()),
        italic_angle: font.metric(own, metric::ITALIC_ANGLE).map(|v| v.pos).filter(|v| v.is_finite() && (-90.0..=90.0).contains(v)),
        is_fixed_pitch: None,
        font_bbox,
        cap_height: vertical(metric::CAP_HEIGHT),
        x_height: vertical(metric::X_HEIGHT),
        ascender: vertical(metric::ASCENDER),
        descender: vertical(metric::DESCENDER),
        chars,
        pairs,
    })
}

// ---------- writing ----------

/// Writes `doc` as AFM, each line ending in `\n`.
pub fn write(doc: &Afm) -> Result<String, WriteError> {
    if doc.pairs.len() > MAX_PAIRS {
        return Err(WriteError::TooManyPairs(doc.pairs.len()));
    }
    if doc.chars.len() > MAX_CHARS {
        return Err(WriteError::TooManyChars(doc.chars.len()));
    }
    let mut out = String::new();
    let line = |out: &mut String, text: &str, what: &dyn Fn() -> String| -> Result<(), WriteError> {
        if text.len() > MAX_LINE_BYTES {
            return Err(WriteError::LineTooLong(what()));
        }
        if out.len() + text.len() + 1 > MAX_TEXT_BYTES {
            return Err(WriteError::TooLarge);
        }
        out.push_str(text);
        out.push('\n');
        Ok(())
    };
    line(&mut out, &format!("{START} {VERSION}"), &|| "the header".into())?;
    let strings = [
        ("FontName", &doc.font_name, true),
        ("FullName", &doc.full_name, false),
        ("FamilyName", &doc.family_name, false),
        ("Weight", &doc.weight, false),
    ];
    for (key, value, one_word) in strings {
        let Some(v) = value else { continue };
        check_text(key, v, one_word)?;
        line(&mut out, &format!("{key} {v}"), &|| key.to_string())?;
    }
    if let Some(a) = doc.italic_angle {
        line(&mut out, &format!("ItalicAngle {}", number("ItalicAngle", a, &(-90.0..=90.0))?), &|| "ItalicAngle".into())?;
    }
    if let Some(f) = doc.is_fixed_pitch {
        line(&mut out, &format!("IsFixedPitch {f}"), &|| "IsFixedPitch".into())?;
    }
    if let Some(b) = doc.font_bbox {
        line(&mut out, &format!("FontBBox {}", bbox("FontBBox", &b)?), &|| "FontBBox".into())?;
    }
    for (key, value) in [("CapHeight", doc.cap_height), ("XHeight", doc.x_height), ("Ascender", doc.ascender), ("Descender", doc.descender)] {
        if let Some(v) = value {
            line(&mut out, &format!("{key} {}", number(key, v, &COORDINATE_RANGE)?), &|| key.to_string())?;
        }
    }
    line(&mut out, &format!("StartCharMetrics {}", doc.chars.len()), &|| "StartCharMetrics".into())?;
    for c in &doc.chars {
        check_name("glyph", &c.name)?;
        if !CODE_RANGE.contains(&c.code) {
            return Err(WriteError::Number { what: format!("the code of `{}`", clip(&c.name)), value: c.code as f64, range: -1.0..=1_114_111.0 });
        }
        let width = number(&format!("the width of `{}`", clip(&c.name)), c.width, &WIDTH_RANGE)?;
        let mut text = format!("C {} ; WX {width} ; N {} ;", c.code, c.name);
        if let Some(b) = &c.bbox {
            text.push_str(&format!(" B {} ;", bbox(&format!("the bounding box of `{}`", clip(&c.name)), b)?));
        }
        line(&mut out, &text, &|| format!("the metrics of `{}`", clip(&c.name)))?;
    }
    line(&mut out, "EndCharMetrics", &|| "EndCharMetrics".into())?;
    line(&mut out, "StartKernData", &|| "StartKernData".into())?;
    line(&mut out, &format!("StartKernPairs {}", doc.pairs.len()), &|| "StartKernPairs".into())?;
    for p in &doc.pairs {
        check_name("glyph", &p.left)?;
        check_name("glyph", &p.right)?;
        let value = number(&format!("the pair {} {}", clip(&p.left), clip(&p.right)), p.value, &VALUE_RANGE)?;
        line(&mut out, &format!("KPX {} {} {value}", p.left, p.right), &|| format!("the pair {} {}", clip(&p.left), clip(&p.right)))?;
    }
    for end in ["EndKernPairs", "EndKernData", "EndFontMetrics"] {
        line(&mut out, end, &|| end.into())?;
    }
    Ok(out)
}

/// A glyph name as AFM spells it: one word without `;` or control characters.
fn check_name(what: &'static str, name: &str) -> Result<(), WriteError> {
    let problem = if name.is_empty() {
        "it is empty"
    } else if name.len() > MAX_NAME_LEN {
        "it is longer than 255 bytes"
    } else if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        "it holds a space or a control character"
    } else if name.contains(';') {
        "it holds a semicolon, which separates the fields of a line"
    } else {
        return Ok(());
    };
    Err(WriteError::Name { what, name: clip(name), problem: problem.into() })
}

fn check_text(key: &'static str, text: &str, one_word: bool) -> Result<(), WriteError> {
    let problem = if text.trim().is_empty() || text.trim() != text {
        "it is empty or starts or ends with a space"
    } else if text.chars().any(char::is_control) {
        "it holds a control character"
    } else if one_word && text.chars().any(char::is_whitespace) {
        "it holds a space"
    } else {
        return Ok(());
    };
    Err(WriteError::Name { what: key, name: clip(text), problem: problem.into() })
}

/// The shortest decimal text that reads back as `v`.
fn number(what: &str, v: f64, range: &RangeInclusive<f64>) -> Result<String, WriteError> {
    if v.is_finite() && range.contains(&v) {
        Ok(format!("{v}"))
    } else {
        Err(WriteError::Number { what: what.to_string(), value: v, range: range.clone() })
    }
}

fn bbox(what: &str, b: &[f64; 4]) -> Result<String, WriteError> {
    let parts: Result<Vec<String>, WriteError> = b.iter().map(|v| number(what, *v, &COORDINATE_RANGE)).collect();
    Ok(parts?.join(" "))
}

// ---------- reading ----------

/// Whether `text` looks like AFM: its first word, after blank space and a byte-order mark, is
/// `StartFontMetrics`.
pub fn sniff(text: &str) -> bool {
    text.trim_start_matches('\u{feff}').trim_start().starts_with(START)
}

/// The lines of `text` with their numbers (from 1); `\r\n`, `\n` and `\r` each end a line.
fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut rest = text;
    let mut number = 0usize;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        number += 1;
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let line = rest.get(..end).unwrap_or_default();
        let after = rest.get(end..).unwrap_or_default();
        rest = after.strip_prefix("\r\n").or_else(|| after.strip_prefix('\r')).or_else(|| after.strip_prefix('\n')).unwrap_or(after);
        Some((number, line))
    })
}

/// Reads an AFM text.
pub fn read(text: &str) -> Result<Afm, ReadError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ReadError::TooLarge(text.len()));
    }
    let mut doc = Afm::default();
    let mut started = false;
    for (line_no, raw) in lines(text.strip_prefix('\u{feff}').unwrap_or(text)) {
        if raw.len() > MAX_LINE_BYTES {
            return Err(ReadError::LineTooLong { line: line_no, len: raw.len() });
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let bad = |message: String| ReadError::Bad { line: line_no, message };
        let (key, rest) = match line.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None => (line, ""),
        };
        if !started {
            if key != START {
                return Err(bad(format!("an AFM text starts with {START}, not `{}`", clip(key))));
            }
            started = true;
            continue;
        }
        match key {
            "C" | "CH" => {
                if doc.chars.len() >= MAX_CHARS {
                    return Err(bad(format!("more than {MAX_CHARS} glyphs")));
                }
                doc.chars.push(read_char(line).map_err(bad)?);
            }
            "KPX" | "KP" => {
                if doc.pairs.len() >= MAX_PAIRS {
                    return Err(bad(format!("more than {MAX_PAIRS} pairs")));
                }
                let words: Vec<&str> = rest.split_whitespace().collect();
                let (left, right, value) = match (key, words.as_slice()) {
                    ("KPX", [l, r, v]) | ("KP", [l, r, v, _]) => (*l, *r, *v),
                    _ => return Err(bad(format!("a {key} line is `{key} left right {}`", if key == "KP" { "x y" } else { "value" }))),
                };
                read_name(left).map_err(|m| bad(format!("left glyph: {m}")))?;
                read_name(right).map_err(|m| bad(format!("right glyph: {m}")))?;
                let value = read_number(value, &VALUE_RANGE).map_err(bad)?;
                doc.pairs.push(KernPair { left: left.to_string(), right: right.to_string(), value });
            }
            "FontName" | "FullName" | "FamilyName" | "Weight" => {
                if rest.chars().any(char::is_control) {
                    return Err(bad(format!("{key} holds a control character")));
                }
                let value = (!rest.is_empty()).then(|| rest.to_string());
                match key {
                    "FontName" => doc.font_name = value,
                    "FullName" => doc.full_name = value,
                    "FamilyName" => doc.family_name = value,
                    _ => doc.weight = value,
                }
            }
            "ItalicAngle" => doc.italic_angle = Some(read_number(rest, &(-90.0..=90.0)).map_err(bad)?),
            "IsFixedPitch" => {
                doc.is_fixed_pitch = Some(match rest {
                    "true" => true,
                    "false" => false,
                    other => return Err(bad(format!("IsFixedPitch is true or false, not `{}`", clip(other)))),
                })
            }
            "FontBBox" => doc.font_bbox = Some(read_box(rest).map_err(bad)?),
            "CapHeight" | "XHeight" | "Ascender" | "Descender" => {
                let v = Some(read_number(rest, &COORDINATE_RANGE).map_err(bad)?);
                match key {
                    "CapHeight" => doc.cap_height = v,
                    "XHeight" => doc.x_height = v,
                    "Ascender" => doc.ascender = v,
                    _ => doc.descender = v,
                }
            }
            _ => {}
        }
    }
    if !started {
        return Err(ReadError::Bad { line: 1, message: format!("the text is empty; an AFM text starts with {START}") });
    }
    Ok(doc)
}

/// A `C` or `CH` line: fields separated by `;`, each a key and its values.
fn read_char(line: &str) -> Result<CharMetric, String> {
    let mut code = None;
    let mut width = None;
    let mut name = None;
    let mut bbox = None;
    for field in line.split(';') {
        let mut words = field.split_whitespace();
        let Some(key) = words.next() else { continue };
        let values: Vec<&str> = words.collect();
        let one = || one_value(key, &values);
        match key {
            "C" => {
                let v = one()?;
                let n: i64 = v.parse().map_err(|_| format!("the code `{}` is not a whole number", clip(v)))?;
                code = Some(n);
            }
            "CH" => {
                let v = one()?;
                let hex = v.strip_prefix('<').and_then(|h| h.strip_suffix('>')).ok_or_else(|| format!("the code `{}` is not <hex>", clip(v)))?;
                if hex.is_empty() || hex.len() > 8 {
                    return Err(format!("the code `{}` is not <hex>", clip(v)));
                }
                code = Some(i64::from_str_radix(hex, 16).map_err(|_| format!("the code `{}` is not <hex>", clip(v)))?);
            }
            "WX" | "W0X" => width = Some(read_number(one()?, &WIDTH_RANGE)?),
            "W" | "W0" => match values.as_slice() {
                [x, _] => width = Some(read_number(x, &WIDTH_RANGE)?),
                _ => return Err(format!("`{key}` takes two values")),
            },
            "N" => {
                let v = one()?;
                read_name(v)?;
                name = Some(v.to_string());
            }
            "B" => bbox = Some(read_box(&values.join(" "))?),
            _ => {}
        }
    }
    let code = code.ok_or("a glyph line needs a code (C or CH)")?;
    if !CODE_RANGE.contains(&code) {
        return Err(format!("the code {code} is outside -1 to {}", CODE_RANGE.end()));
    }
    let name = name.ok_or("a glyph line needs a name (N)")?;
    let width = width.ok_or_else(|| format!("the line of `{}` gives no width (WX)", clip(&name)))?;
    Ok(CharMetric { code, name, width, bbox })
}

fn one_value<'a>(key: &str, values: &[&'a str]) -> Result<&'a str, String> {
    match values {
        [v] => Ok(v),
        _ => Err(format!("`{key}` takes one value")),
    }
}

fn read_box(text: &str) -> Result<[f64; 4], String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let [a, b, c, d] = words.as_slice() else { return Err("a bounding box is four numbers".into()) };
    Ok([
        read_number(a, &COORDINATE_RANGE)?,
        read_number(b, &COORDINATE_RANGE)?,
        read_number(c, &COORDINATE_RANGE)?,
        read_number(d, &COORDINATE_RANGE)?,
    ])
}

fn read_number(text: &str, range: &RangeInclusive<f64>) -> Result<f64, String> {
    let t = text.trim();
    let ok = !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'));
    let v = if ok { t.parse::<f64>().ok() } else { None };
    match v {
        Some(v) if v.is_finite() && range.contains(&v) => Ok(v),
        Some(v) if v.is_finite() => Err(format!("{v} is outside {} to {}", range.start(), range.end())),
        Some(_) => Err(format!("`{}` is too large", clip(t))),
        None => Err(format!("`{}` is not a number", clip(t))),
    }
}

fn read_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("the name is empty".into())
    } else if name.len() > MAX_NAME_LEN {
        Err(format!("the name is longer than {MAX_NAME_LEN} bytes"))
    } else if name.chars().any(char::is_control) {
        Err("the name holds a control character".into())
    } else {
        Ok(())
    }
}

/// At most 40 characters of untrusted text, for a message.
fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(40).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn sample() -> Afm {
        Afm {
            font_name: Some("Test-Regular".into()),
            full_name: Some("Test Regular".into()),
            family_name: Some("Test".into()),
            weight: Some("Regular".into()),
            italic_angle: Some(-9.5),
            is_fixed_pitch: Some(false),
            font_bbox: Some([-20.0, -200.0, 900.0, 800.0]),
            cap_height: Some(700.0),
            x_height: Some(500.0),
            ascender: Some(800.0),
            descender: Some(-200.0),
            chars: vec![
                CharMetric { code: 65, name: "A".into(), width: 600.0, bbox: Some([10.0, 0.0, 590.0, 700.0]) },
                CharMetric { code: -1, name: "Che-cy".into(), width: 512.5, bbox: Some([0.0; 4]) },
                CharMetric { code: 32, name: "space".into(), width: 250.0, bbox: None },
            ],
            pairs: vec![
                KernPair { left: "A".into(), right: "Che-cy".into(), value: -40.0 },
                KernPair { left: "Che-cy".into(), right: "A".into(), value: 0.0 },
            ],
        }
    }

    #[test]
    fn what_write_gives_reads_back_with_any_line_end() {
        let doc = sample();
        let text = write(&doc).unwrap();
        assert!(text.starts_with("StartFontMetrics 4.1\nFontName Test-Regular\n"), "{text}");
        assert!(text.contains("\nC 65 ; WX 600 ; N A ; B 10 0 590 700 ;\n"), "{text}");
        assert!(text.contains("\nKPX A Che-cy -40\n"), "{text}");
        assert!(text.ends_with("EndKernPairs\nEndKernData\nEndFontMetrics\n"));
        assert!(sniff(&text));
        for ending in ["\n", "\r\n", "\r"] {
            assert_eq!(read(&text.replace('\n', ending)).unwrap(), doc, "{ending:?}");
        }
        assert_eq!(doc.chars[0].sidebearings(), Some((10.0, 10.0)));
        assert_eq!(doc.chars[1].sidebearings(), None, "an all-zero box is an empty glyph");
    }

    #[test]
    fn other_spellings_are_read_and_unknown_keywords_skipped() {
        let text = "\u{feff}StartFontMetrics 2.0\nComment anything ; at all\nFontName X\nStartCharMetrics 2\n\
            CH <0041> ; W 600 0 ; N A ; L A V AV ; B 1 2 3 4 ;\nC -1;WX 300;N b;\nEndCharMetrics\n\
            StartKernData\nStartTrackKern 1\nTrackKern 0 6 0 72 0\nEndTrackKern\nStartKernPairs 3\n\
            KP A b -20 0\nKPY A b 5\nKPH <0041> <0042> -3\nEndKernPairs\nEndKernData\nStartComposites 1\n\
            CC Aacute 2 ; PCC A 0 0 ; PCC acute 100 200 ;\nEndComposites\nEndFontMetrics\n";
        let doc = read(text).unwrap();
        assert_eq!(
            doc.chars,
            vec![
                CharMetric { code: 0x41, name: "A".into(), width: 600.0, bbox: Some([1.0, 2.0, 3.0, 4.0]) },
                CharMetric { code: -1, name: "b".into(), width: 300.0, bbox: None },
            ]
        );
        assert_eq!(doc.pairs, vec![KernPair { left: "A".into(), right: "b".into(), value: -20.0 }]);
    }

    #[test]
    fn write_refuses_what_read_would_not_give_back() {
        let with = |f: &dyn Fn(&mut Afm)| {
            let mut doc = sample();
            f(&mut doc);
            write(&doc)
        };
        assert!(with(&|d| d.chars[0].name = "a b".into()).is_err());
        assert!(with(&|d| d.chars[0].name = "a;b".into()).is_err());
        assert!(with(&|d| d.pairs[0].value = f64::NAN).is_err());
        assert!(with(&|d| d.pairs[0].value = 40_000.0).is_err());
        assert!(with(&|d| d.chars[0].width = -1.0).is_err());
        assert!(with(&|d| d.font_name = Some("Test Regular".into())).is_err());
        assert!(with(&|d| d.full_name = Some("a\nb".into())).is_err());
        assert!(with(&|d| d.full_name = Some(" a".into())).is_err());
        assert!(with(&|d| d.chars[0].code = 0x11_0000).is_err());
    }

    #[test]
    fn hostile_text_fails_with_a_message() {
        assert!(matches!(read(&"x".repeat(MAX_TEXT_BYTES + 1)), Err(ReadError::TooLarge(_))));
        assert!(matches!(
            read(&format!("StartFontMetrics 2.0\nComment {}", "x".repeat(MAX_LINE_BYTES))),
            Err(ReadError::LineTooLong { line: 2, .. })
        ));
        let many = format!("StartFontMetrics 2.0\n{}", "KPX a b 1\n".repeat(MAX_PAIRS + 1));
        assert!(read(&many).unwrap_err().to_string().contains("pairs"));
        for (text, says) in [
            ("", "empty"),
            ("C 1 ; WX 1 ; N a ;", "starts with"),
            ("StartFontMetrics\nC 1 ; WX 1e400 ; N a ;", "too large"),
            ("StartFontMetrics\nC 1 ; WX -5 ; N a ;", "outside"),
            ("StartFontMetrics\nC 1 ; WX 5 ;", "name"),
            ("StartFontMetrics\nC x ; WX 5 ; N a ;", "whole number"),
            ("StartFontMetrics\nC 1 ; N a ;", "width"),
            ("StartFontMetrics\nC 1 ; WX 5 ; N a ; B 1 2 3 ;", "four numbers"),
            ("StartFontMetrics\nKPX a b", "KPX"),
            ("StartFontMetrics\nKPX a b NaN", "not a number"),
            ("StartFontMetrics\nCH <123456789> ; WX 1 ; N a ;", "hex"),
            ("StartFontMetrics\nC 1114112 ; WX 1 ; N a ;", "outside"),
            ("StartFontMetrics\nIsFixedPitch maybe", "true or false"),
        ] {
            let e = read(text).unwrap_err().to_string();
            assert!(e.contains(says), "{text:?}: {e}");
        }
    }
}
