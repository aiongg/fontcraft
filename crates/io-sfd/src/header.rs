//! The file's top level: the font header's keywords, lookups and class kerning, the blocks that
//! are skipped, and the dispatch of `StartChar` blocks to [`crate::glyph`].

use std::collections::BTreeMap;

use crate::glyph::{self, RawGlyph};
use crate::lines::{self, A85Count, Lines, Tok};
use crate::{MAX_KERN_CELLS, MAX_KERN_CLASSES, MAX_LAYERS, Report, Result, SfdError};

/// A layer the font declares (`Layer: 1 1 "Fore" 0`).
#[derive(Clone, Debug)]
pub(crate) struct FontLayer {
    pub(crate) quadratic: bool,
    pub(crate) name: String,
}

/// A lookup subtable, as far as kerning needs it.
#[derive(Clone, Debug)]
pub(crate) struct Subtable {
    /// Pair positioning in a lookup for the horizontal kerning feature (or for no feature).
    pub(crate) kern: bool,
    pub(crate) vertical: bool,
}

/// One class kerning table (`KernClass2`).
#[derive(Clone, Debug)]
pub(crate) struct KernClass {
    pub(crate) subtable: String,
    /// Glyph names per class; class 0 of a side is "every other glyph" unless it was listed.
    pub(crate) firsts: Vec<Vec<String>>,
    pub(crate) first_zero_listed: bool,
    pub(crate) seconds: Vec<Vec<String>>,
    /// `firsts.len() × seconds.len()` values, row by row.
    pub(crate) values: Vec<f64>,
    pub(crate) at: String,
}

/// Everything read, before it becomes a model font.
#[derive(Default)]
pub(crate) struct RawFont {
    /// Header keywords with one value; the first occurrence wins (CID subfonts repeat them).
    pub(crate) header: BTreeMap<String, String>,
    /// `LangName`: Windows language id → strings by name id, first occurrence per language.
    pub(crate) lang_names: BTreeMap<u16, Vec<String>>,
    pub(crate) layers: BTreeMap<usize, FontLayer>,
    pub(crate) subtables: BTreeMap<String, Subtable>,
    pub(crate) kern_classes: Vec<KernClass>,
    pub(crate) glyphs: Vec<RawGlyph>,
    pub(crate) seen_magic: bool,
    /// Points read so far, against [`crate::MAX_TOTAL_POINTS`].
    pub(crate) points: usize,
}

impl RawFont {
    /// Whether layer `n` holds quadratic curves.
    pub(crate) fn quadratic(&self, n: usize) -> bool {
        match self.layers.get(&n) {
            Some(l) => l.quadratic,
            None => self.header.get("Order2").is_some_and(|v| v.trim() == "1"),
        }
    }
}

/// Header keywords kept as plain values for [`crate::build`].
const VALUES: &[&str] = &[
    "FontName",
    "FullName",
    "FamilyName",
    "Weight",
    "Copyright",
    "Comments",
    "UComments",
    "Version",
    "ItalicAngle",
    "UnderlinePosition",
    "UnderlineWidth",
    "Ascent",
    "Descent",
    "Order2",
    "CreationTime",
    "FSType",
    "Panose",
    "LineGap",
    "OS2LineGap",
    "VLineGap",
    "OS2Vendor",
    "OS2_UseTypoMetrics",
    "HheadAscent",
    "HheadAOffset",
    "HheadDescent",
    "HheadDOffset",
    "OS2TypoAscent",
    "OS2TypoAOffset",
    "OS2TypoDescent",
    "OS2TypoDOffset",
    "OS2WinAscent",
    "OS2WinAOffset",
    "OS2WinDescent",
    "OS2WinDOffset",
    "OS2SubXSize",
    "OS2SubYSize",
    "OS2SubXOff",
    "OS2SubYOff",
    "OS2SupXSize",
    "OS2SupYSize",
    "OS2SupXOff",
    "OS2SupYOff",
    "OS2StrikeYSize",
    "OS2StrikeYPos",
];

/// Header keywords of editor state or of values derived on export: read and dropped silently.
const IGNORED: &[&str] = &[
    "DefaultBaseFilename",
    "DisplaySize",
    "DisplayLayer",
    "DisplayFont",
    "AntiAlias",
    "FitToEm",
    "WinInfo",
    "Encoding",
    "BeginChars",
    "EndChars",
    "EndSplineFont",
    "LayerCount",
    "UseUniqueID",
    "UseXUID",
    "UniqueID",
    "XUID",
    "ModificationTime",
    "DEI",
    "ExtremaBound",
    "WidthSeparation",
    "sfntRevision",
    "OnlyBitmaps",
    "MacStyle",
    "PfmFamily",
    "PFMFamily",
    "OS2Version",
    "OS2_WeightWidthSlopeOnly",
    "OS2FamilyClass",
    "OS2CodePages",
    "OS2UnicodeRanges",
    "TTFWidth",
    "Compacted",
    "NeedsXUIDChange",
    "AnchorClass2",
    "AnchorClass",
    "GridOrder2",
    "UFOAscent",
    "UFODescent",
    "Registry",
    "Ordering",
    "Supplement",
    "CIDVersion",
    "StyleMapFamilyName",
    "UnicodeInterp",
    "NameList",
];

/// Header keywords whose data is skipped and reported, with the words the report uses.
const REPORTED: &[(&str, &str)] = &[
    ("TeXData", "TeX font parameters"),
    ("GaspTable", "the gasp table"),
    ("FontLog", "the font log"),
    ("woffMajor", "WOFF version and metadata"),
    ("woffMinor", "WOFF version and metadata"),
    ("woffMetadata", "WOFF version and metadata"),
    ("PickledData", "script data"),
    ("OtfFeatName", "stylistic set names"),
    ("BaseHoriz", "BASE table data"),
    ("BaseVert", "BASE table data"),
    ("BaseScript", "BASE table data"),
    ("MacFeat", "AAT feature names"),
    ("MacName", "AAT feature names"),
    ("MacSetting", "AAT feature names"),
    ("TTFWeight", "the OS/2 weight class"),
    ("BeginSubFonts", "the CID subfont structure (glyphs are read into one glyph set)"),
    ("KernClass", "kerning classes in the format before lookups"),
    ("VKernClass2", "vertical kerning classes"),
    ("VKernClass", "vertical kerning classes"),
];

/// Multi-line header blocks that are skipped up to their end line, with the words the report uses.
const BLOCKS: &[(&str, &str, &str)] = &[
    ("BeginPrivate", "EndPrivate", "the PostScript private dictionary"),
    ("Grid", "EndSplineSet", "font-wide guides"),
    ("ContextPos2", "EndFPST", "contextual lookups"),
    ("ContextSub2", "EndFPST", "contextual lookups"),
    ("ChainPos2", "EndFPST", "contextual lookups"),
    ("ChainSub2", "EndFPST", "contextual lookups"),
    ("ReverseChain2", "EndFPST", "contextual lookups"),
    ("MacIndic2", "EndASM", "AAT state machines"),
    ("MacContext2", "EndASM", "AAT state machines"),
    ("MacInsert2", "EndASM", "AAT state machines"),
    ("MacKern2", "EndASM", "AAT state machines"),
    ("Justify", "EndJustify", "JSTF data"),
    ("BitmapFont", "EndBitmapFont", "bitmap strikes"),
];

/// Reads one file (an `.sfd`, a `font.props` or a `.glyph`) into `raw`.
pub(crate) fn read_file(lines: &mut Lines<'_>, raw: &mut RawFont, report: &mut Report, magic: bool) -> Result<()> {
    if magic {
        let first = lines.next()?.unwrap_or("");
        let (k, v) = lines::keyword(first);
        if k != "SplineFontDB" {
            return Err(SfdError::NotSfd("the first line is not `SplineFontDB: <version>`".into()));
        }
        if lines::num(v).is_none() {
            return Err(lines.err(format!("`{v}` is not a format version")));
        }
        raw.seen_magic = true;
    }
    while let Some(line) = lines.next()? {
        let (k, v) = lines::keyword(line);
        if k.is_empty() {
            continue;
        }
        if k == "StartChar" {
            let g = glyph::read_char(lines, raw, report, v)?;
            raw.glyphs.push(g);
            if raw.glyphs.len() > fontcraft_model::MAX_GLYPHS {
                return Err(SfdError::Model(fontcraft_model::ModelError::TooManyGlyphs));
            }
            continue;
        }
        if let Some((_, end, what)) = BLOCKS.iter().find(|(b, _, _)| *b == k) {
            report.skip(what, &lines.at());
            skip_to(lines, end, k)?;
            continue;
        }
        match k {
            "MMCounts" => return Err(SfdError::Unsupported("multiple master sources".into())),
            "SplineFontDB" => {}
            "LangName" => lang_name(lines, raw, report, v)?,
            "Layer" => font_layer(lines, raw, v)?,
            "Lookup" => lookup(raw, v),
            "KernClass2" => kern_class(lines, raw, v)?,
            "TtTable" => {
                report.skip("TrueType programs", &lines.at());
                skip_instructions(lines)?;
            }
            "ShortTable" => {
                report.skip("TrueType tables", &lines.at());
                let n = v.split_whitespace().nth(1).and_then(lines::int).unwrap_or(0);
                skip_lines(lines, n, k)?;
                if lines.peek()?.is_some_and(|l| l.trim().starts_with("EndShort")) {
                    lines.next()?;
                }
            }
            "TtfTable" => {
                report.skip("TrueType tables", &lines.at());
                let n = v.split_whitespace().nth(1).and_then(lines::int).unwrap_or(0);
                skip_a85(lines, n)?;
            }
            "MarkAttachClasses" | "MarkAttachSets" => {
                report.skip("mark attachment classes and sets", &lines.at());
                let n = lines::int(v).unwrap_or(0);
                skip_lines(lines, if k == "MarkAttachClasses" { n.saturating_sub(1) } else { n }, k)?;
            }
            _ if k.starts_with("MATH:") => report.skip("MATH table constants", &lines.at()),
            _ if k.starts_with("Jstf") => report.skip("JSTF data", &lines.at()),
            _ if VALUES.contains(&k) => {
                let value = lines.value(v)?;
                raw.header.entry(k.to_string()).or_insert_with(|| value.into_owned());
            }
            _ if IGNORED.contains(&k) => {
                lines.value(v)?;
            }
            _ => {
                lines.value(v)?;
                match REPORTED.iter().find(|(r, _)| *r == k) {
                    Some((_, what)) => report.skip(what, &lines.at()),
                    None if is_private_key(k) => report.skip("the PostScript private dictionary", &lines.at()),
                    None => report.skip(&format!("unknown keyword `{}`", short(k)), &lines.at()),
                }
            }
        }
    }
    Ok(())
}

fn is_private_key(k: &str) -> bool {
    ["BlueValues", "OtherBlues", "StdHW", "StdVW", "StemSnapH", "StemSnapV", "BlueScale", "BlueShift", "BlueFuzz", "ForceBold"].contains(&k)
}

/// At most 64 characters of a keyword, for the report.
pub(crate) fn short(k: &str) -> String {
    k.chars().take(64).collect()
}

/// Skips lines up to and including `end`.
pub(crate) fn skip_to(lines: &mut Lines<'_>, end: &str, block: &str) -> Result<()> {
    loop {
        let l = lines.need(&format!("a `{block}` block (no `{end}` line)"))?;
        if l.trim() == end {
            return Ok(());
        }
    }
}

fn skip_lines(lines: &mut Lines<'_>, n: i64, block: &str) -> Result<()> {
    for _ in 0..n.max(0) {
        lines.need(&format!("a `{block}` block"))?;
    }
    Ok(())
}

/// Skips ASCII85 lines until they hold `n` bytes, then an `EndTtf` line if one follows.
pub(crate) fn skip_a85(lines: &mut Lines<'_>, n: i64) -> Result<()> {
    let mut count = A85Count::default();
    while (count.bytes() as i64) < n {
        let Some(l) = lines.peek()? else { break };
        if !count.feed(l) {
            break;
        }
        lines.next()?;
    }
    if lines.peek()?.is_some_and(|l| l.trim().starts_with("EndTtf")) {
        lines.next()?;
    }
    Ok(())
}

/// Skips a block of TrueType instructions written as mnemonics and numbers. The format page
/// names no end line for it, so the block ends at a line starting `EndTt` (consumed), or before
/// the first line that is neither a mnemonic nor a number (left for the caller).
pub(crate) fn skip_instructions(lines: &mut Lines<'_>) -> Result<()> {
    while let Some(l) = lines.peek()? {
        let t = l.trim();
        if t.starts_with("EndTt") || t.starts_with("EndTT") {
            lines.next()?;
            return Ok(());
        }
        let mnemonic = t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && !t.contains(':')
            && t.split('[').next().is_some_and(|m| m.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            && !matches!(t, "EndChar" | "EndSplineSet" | "SplineSet" | "Fore" | "Back" | "EndChars" | "EndSplineFont");
        let number = t.split_whitespace().all(|w| lines::int(w).is_some()) && !t.is_empty();
        if !(mnemonic || number) {
            return Ok(());
        }
        lines.next()?;
    }
    Ok(())
}

fn lang_name(lines: &mut Lines<'_>, raw: &mut RawFont, report: &mut Report, v: &str) -> Result<()> {
    let value = lines.value(v)?;
    let toks = lines::tokens(&value);
    let Some(Tok::Word(lang)) = toks.first() else {
        return Err(lines.err("`LangName` has no language number"));
    };
    let Some(lang) = lines::int(lang).and_then(|l| u16::try_from(l).ok()) else {
        report.skip("name table entries with a bad language number", &lines.at());
        return Ok(());
    };
    let strings = toks.iter().skip(1).map(|t| match t {
        Tok::Quoted(q) => lines::utf7(q),
        Tok::Word(w) | Tok::Group(w) => lines::utf7(w),
    });
    raw.lang_names.entry(lang).or_insert_with(|| strings.collect());
    Ok(())
}

fn font_layer(lines: &mut Lines<'_>, raw: &mut RawFont, v: &str) -> Result<()> {
    let toks = lines::tokens(v);
    let word = |i: usize| match toks.get(i) {
        Some(Tok::Word(w)) => lines::int(w),
        _ => None,
    };
    let Some(n) = word(0).and_then(|n| usize::try_from(n).ok()) else {
        return Err(lines.err("`Layer` needs a layer number"));
    };
    if n >= crate::MAX_LAYERS {
        return Err(lines.err(format!("layer {n} is beyond the limit of {MAX_LAYERS} layers")));
    }
    let name = match toks.get(2) {
        Some(Tok::Quoted(q)) => lines::utf7(q),
        Some(Tok::Word(w)) => lines::utf7(w),
        _ => format!("Layer {n}"),
    };
    let quadratic = word(1) == Some(1);
    raw.layers.entry(n).or_insert(FontLayer { quadratic, name });
    Ok(())
}

/// `Lookup: <type> <flags> <afm> "<name>" { "<subtable>" … } [ '<feature>' ( … ) … ]`. Pair
/// positioning is type 0x102 (the format numbers GPOS lookup types from 0x101).
fn lookup(raw: &mut RawFont, v: &str) {
    let toks = lines::tokens(v);
    let kind = match toks.first() {
        Some(Tok::Word(w)) => lines::int(w).unwrap_or(-1),
        _ => -1,
    };
    let Some(group_at) = toks.iter().position(|t| matches!(t, Tok::Group(_))) else { return };
    let Some(Tok::Group(group)) = toks.get(group_at) else { return };
    let features = v.find(group).and_then(|i| v.get(i + group.len()..)).unwrap_or("");
    let horizontal_kern = features.contains("'kern'") || !features.contains('\'');
    let vertical_lookup = features.contains("'vkrn'");
    let inner = group.strip_prefix('{').and_then(|g| g.strip_suffix('}')).unwrap_or(group);
    let sub = lines::tokens(inner);
    for (i, t) in sub.iter().enumerate() {
        let Tok::Quoted(name) = t else { continue };
        let vertical_mark = matches!(sub.get(i + 1), Some(Tok::Word("(1)")));
        raw.subtables.entry(name.clone()).or_insert(Subtable {
            kern: kind == 0x102 && horizontal_kern && !vertical_lookup && !vertical_mark,
            vertical: vertical_lookup || vertical_mark,
        });
    }
}

/// `KernClass2: <n1>[+] <n2> … "<subtable>"`, then the first classes (from class 1, or from
/// class 0 after a `+`), the second classes from class 1, then `n1 × n2` values that may carry
/// device tables in braces.
fn kern_class(lines: &mut Lines<'_>, raw: &mut RawFont, v: &str) -> Result<()> {
    let at = lines.at();
    let toks = lines::tokens(v);
    let (Some(Tok::Word(a)), Some(Tok::Word(b))) = (toks.first(), toks.get(1)) else {
        return Err(lines.err("`KernClass2` needs two class counts"));
    };
    let plus = a.ends_with('+');
    let count = |s: &str| lines::int(s.trim_end_matches('+')).and_then(|n| usize::try_from(n).ok());
    let (Some(n1), Some(n2)) = (count(a), count(b)) else {
        return Err(lines.err("`KernClass2` needs two class counts"));
    };
    if n1 == 0 || n2 == 0 || n1 > MAX_KERN_CLASSES || n2 > MAX_KERN_CLASSES || n1.saturating_mul(n2) > MAX_KERN_CELLS {
        return Err(lines.err(format!(
            "a class kerning table of {n1} × {n2} classes is empty or beyond the limits ({MAX_KERN_CLASSES} classes, {MAX_KERN_CELLS} values)"
        )));
    }
    let subtable = toks.iter().find_map(|t| if let Tok::Quoted(q) = t { Some(q.clone()) } else { None }).unwrap_or_default();
    let class_line = |l: &str| -> Vec<String> { l.split_whitespace().skip(1).map(lines::utf7).collect() };
    let mut firsts = if plus { Vec::new() } else { vec![Vec::new()] };
    while firsts.len() < n1 {
        let l = lines.need("a class kerning table")?;
        firsts.push(class_line(l));
    }
    let mut seconds = vec![Vec::new()];
    while seconds.len() < n2 {
        let l = lines.need("a class kerning table")?;
        seconds.push(class_line(l));
    }
    let cells = n1 * n2;
    let mut values = Vec::new();
    while values.len() < cells {
        let l = lines.need("the values of a class kerning table")?;
        for t in lines::tokens(l) {
            match t {
                Tok::Word(w) => match lines::num(w) {
                    Some(x) if values.len() < cells => values.push(x),
                    Some(_) => {}
                    None => return Err(lines.err(format!("`{}` is not a kerning value", short(w)))),
                },
                Tok::Group(_) => {}
                Tok::Quoted(_) => return Err(lines.err("a class kerning table holds a string among its values")),
            }
        }
    }
    raw.kern_classes.push(KernClass { subtable, firsts, first_zero_listed: plus, seconds, values, at });
    Ok(())
}
