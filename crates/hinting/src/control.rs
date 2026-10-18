//! TTFAutohint control instructions: per-glyph corrections in ttfautohint's control syntax, as
//! the handbook lists it (`left`, `right`, `nodir`, `touch`, `point`, one-letter abbreviations).
//!
//! `left`, `right` and `nodir` feed our autohinter's analysis; `touch` and `point` become
//! Delta-like corrections. Text over [`MAX_CONTROL_TEXT`] bytes or [`MAX_CONTROL_LINES`] lines is
//! refused. Rows: `hinting.tt.param.ttfAutohintControlInstructions`,
//! `autohint.tt.controlInstructions.syntax`.
//!
//! The grammar is the one ttfautohint's manual gives (read as user documentation): entries end at
//! a semicolon or a line end, a backslash at the end of a line continues it, `#` starts a
//! comment. An entry is an optional font index, one or more comma-separated glyph names or
//! indexes, then one of
//!
//! | Form | Meaning |
//! |---|---|
//! | `left` or `right` points [`(` left offset `,` right offset `)`] | a segment of that direction |
//! | `nodir` points | weak points |
//! | `touch` or `point` points [`xshift` x] [`yshift` y] `@` sizes | a correction at those sizes |
//! | script feature `@` glyphs, script feature `width` widths | style forms: read and left out |
//!
//! Each keyword may be shortened to its first letter. Points and sizes are comma-separated
//! numbers or `a-b` ranges; shifts are pixels from −1 to 1.

use crate::{HintingError, MAX_CONTROL_LINES, MAX_CONTROL_TEXT, MAX_DELTA_PPM};

/// Points one entry may name, ranges expanded.
pub const MAX_CONTROL_POINTS: usize = 65_536;
/// Highest point index an entry may name (the TrueType point limit).
const MAX_POINT_INDEX: u32 = 65_535;

/// What one line asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlKind {
    /// Points belong to a segment's left (or right) edge.
    Left,
    Right,
    /// Points are weak (no direction).
    NoDir,
    /// Shift the points at the given sizes, after hinting.
    Touch {
        x_shift: f64,
        y_shift: f64,
    },
    /// Shift the points at the given sizes, before IUP.
    Point {
        x_shift: f64,
        y_shift: f64,
    },
}

/// One control instruction.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlLine {
    /// 1-based line in the text, for messages.
    pub line: usize,
    pub font_index: u32,
    pub glyph: String,
    pub kind: ControlKind,
    pub points: Vec<u32>,
    pub ppems: Vec<u16>,
}

/// Parsed control instructions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlInstructions {
    pub lines: Vec<ControlLine>,
    /// Lines of the style forms, which the built-in autohinter does not use.
    pub ignored: Vec<usize>,
}

impl ControlInstructions {
    /// The instructions split by glyph name, so a run over many glyphs looks each one up once.
    pub fn by_glyph(&self) -> std::collections::HashMap<&str, ControlInstructions> {
        let mut out: std::collections::HashMap<&str, ControlInstructions> = std::collections::HashMap::new();
        for l in &self.lines {
            out.entry(l.glyph.as_str()).or_default().lines.push(l.clone());
        }
        out
    }
}

/// Parses control-instruction text.
pub fn parse(text: &str) -> Result<ControlInstructions, HintingError> {
    if text.len() > MAX_CONTROL_TEXT {
        return Err(HintingError::LimitExceeded { what: "bytes of control instructions", limit: MAX_CONTROL_TEXT });
    }
    if text.lines().count() > MAX_CONTROL_LINES {
        return Err(HintingError::LimitExceeded { what: "lines of control instructions", limit: MAX_CONTROL_LINES });
    }
    let mut out = ControlInstructions::default();
    // Entries: (first line, text), comments removed and continued lines joined.
    let mut entry = String::new();
    let mut entry_line = 1;
    let mut continued = false;
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        if !continued {
            entry_line = line_no;
        }
        let code = raw.split('#').next().unwrap_or("");
        let (code, more) = match code.trim_end().strip_suffix('\\') {
            Some(c) => (c, true),
            None => (code, false),
        };
        for (k, part) in code.split(';').enumerate() {
            if k > 0 {
                parse_entry(&entry, entry_line, &mut out)?;
                entry.clear();
                entry_line = line_no;
            }
            entry.push(' ');
            entry.push_str(part);
        }
        continued = more;
        if !more {
            parse_entry(&entry, entry_line, &mut out)?;
            entry.clear();
        }
    }
    parse_entry(&entry, entry_line, &mut out)?;
    Ok(out)
}

fn keyword(token: &str) -> Option<&'static str> {
    let t = token.to_ascii_lowercase();
    ["left", "right", "nodir", "touch", "point", "xshift", "yshift", "width"]
        .into_iter()
        .find(|k| t == *k || (t.len() == 1 && k.starts_with(t.as_str())))
}

/// Splits an entry into words, numbers, `@`, `(`, `)`, `,` and `-`.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_whitespace() || matches!(c, '@' | '(' | ')' | ',') {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn parse_entry(text: &str, line: usize, out: &mut ControlInstructions) -> Result<(), HintingError> {
    let err = |message: String| HintingError::Control { line, message };
    let t = tokens(text);
    if t.is_empty() {
        return Ok(());
    }
    // Style forms: script feature @ glyphs, script feature width widths.
    if t.len() >= 3
        && t.get(2).is_some_and(|w| w == "@" || keyword(w) == Some("width"))
        && t.first().is_some_and(|w| keyword(w).is_none() && w.parse::<u32>().is_err())
    {
        out.ignored.push(line);
        return Ok(());
    }
    // An optional font index, then glyphs joined by commas, then the instruction: a glyph may be
    // called like a keyword, so position decides.
    let instruction = |w: Option<&String>| w.is_some_and(|w| matches!(keyword(w), Some("left" | "right" | "nodir" | "touch" | "point")));
    let glyph_list = |from: usize| -> Option<(Vec<String>, usize)> {
        let mut glyphs = Vec::new();
        let mut i = from;
        loop {
            let g = t.get(i).filter(|w| !matches!(w.as_str(), "," | "@" | "(" | ")"))?;
            glyphs.push(g.clone());
            i += 1;
            if t.get(i).is_some_and(|w| w == ",") {
                i += 1;
            } else {
                return instruction(t.get(i)).then_some((glyphs, i));
            }
        }
    };
    let (font_index, glyphs, at) = match glyph_list(0) {
        Some((g, at)) => (0, g, at),
        None => match (t.first().and_then(|w| w.parse::<u32>().ok()), glyph_list(1)) {
            (Some(f), Some((g, at))) => (f, g, at),
            _ => return Err(err(format!("expected glyphs and then left, right, nodir, touch or point in `{}`", text.trim()))),
        },
    };
    let kind_word = keyword(t.get(at).map(String::as_str).unwrap_or("")).unwrap_or("");
    let rest = t.get(at + 1..).unwrap_or_default();
    let (points, rest) = numbers(rest, MAX_CONTROL_POINTS, MAX_POINT_INDEX).map_err(|m| err(format!("points: {m}")))?;
    if points.is_empty() {
        return Err(err("no point is named".into()));
    }
    let kind = match kind_word {
        "left" | "right" => {
            // Optional (left offset, right offset): font units, read and checked only.
            if let Some(open) = rest.first().filter(|w| *w == "(") {
                let _ = open;
                let inner: Vec<&String> = rest.iter().skip(1).take_while(|w| *w != ")").filter(|w| *w != ",").collect();
                if inner.len() != 2 || inner.iter().any(|w| w.parse::<i32>().is_err()) || rest.iter().filter(|w| *w == ")").count() != 1 {
                    return Err(err("the offsets must be `(left, right)` in whole font units".into()));
                }
                if rest.last().is_some_and(|w| w != ")") {
                    return Err(err("text after the offsets".into()));
                }
            } else if !rest.is_empty() {
                return Err(err(format!("unexpected `{}`", rest.join(" "))));
            }
            if kind_word == "left" { ControlKind::Left } else { ControlKind::Right }
        }
        "nodir" => {
            if !rest.is_empty() {
                return Err(err(format!("unexpected `{}`", rest.join(" "))));
            }
            ControlKind::NoDir
        }
        _ => {
            let (mut x_shift, mut y_shift) = (0.0, 0.0);
            let mut i = 0;
            while let Some(w) = rest.get(i) {
                match keyword(w) {
                    Some(k @ ("xshift" | "yshift")) => {
                        let v: f64 = rest
                            .get(i + 1)
                            .and_then(|n| n.parse().ok())
                            .filter(|v: &f64| v.is_finite() && (-1.0..=1.0).contains(v))
                            .ok_or_else(|| err(format!("{k} needs a number of pixels from -1 to 1")))?;
                        if k == "xshift" {
                            x_shift = v
                        } else {
                            y_shift = v
                        }
                        i += 2;
                    }
                    _ => break,
                }
            }
            if rest.get(i).is_none_or(|w| w != "@") {
                return Err(err("a correction needs `@` and its sizes".into()));
            }
            let (ppems, after) = numbers(rest.get(i + 1..).unwrap_or_default(), usize::from(MAX_DELTA_PPM), u32::from(MAX_DELTA_PPM))
                .map_err(|m| err(format!("sizes: {m}")))?;
            if ppems.is_empty() || !after.is_empty() || ppems.contains(&0) {
                return Err(err(format!("the sizes must be pixel sizes from 1 to {MAX_DELTA_PPM}")));
            }
            let ppems: Vec<u16> = ppems.into_iter().filter_map(|p| u16::try_from(p).ok()).collect();
            let kind = if kind_word == "touch" { ControlKind::Touch { x_shift, y_shift } } else { ControlKind::Point { x_shift, y_shift } };
            if out.lines.len() >= MAX_CONTROL_LINES {
                return Err(HintingError::LimitExceeded { what: "control instructions", limit: MAX_CONTROL_LINES });
            }
            for g in &glyphs {
                out.lines.push(ControlLine { line, font_index, glyph: g.clone(), kind: kind.clone(), points: points.clone(), ppems: ppems.clone() });
            }
            return Ok(());
        }
    };
    if out.lines.len() >= MAX_CONTROL_LINES {
        return Err(HintingError::LimitExceeded { what: "control instructions", limit: MAX_CONTROL_LINES });
    }
    for g in &glyphs {
        out.lines.push(ControlLine { line, font_index, glyph: g.clone(), kind: kind.clone(), points: points.clone(), ppems: Vec::new() });
    }
    Ok(())
}

/// Reads comma-separated numbers and `a-b` ranges from the start of `t`, at most `limit` in
/// all, each at most `max`; returns them and the tokens after them.
fn numbers(t: &[String], limit: usize, max: u32) -> Result<(Vec<u32>, &[String]), String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(w) = t.get(i) {
        let (a, b) = match w.split_once('-') {
            Some((a, b)) => (a, b),
            None => (w.as_str(), w.as_str()),
        };
        let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) else {
            if out.is_empty() && i == 0 {
                return Err(format!("`{w}` is not a number or a range"));
            }
            break;
        };
        if a > b || b > max {
            return Err(format!("`{w}` is not a range from 0 to {max}"));
        }
        let count = usize::try_from(b - a).unwrap_or(usize::MAX).saturating_add(1);
        if out.len().saturating_add(count) > limit {
            return Err(format!("more than {limit} numbers"));
        }
        out.extend(a..=b);
        i += 1;
        match t.get(i) {
            Some(c) if c == "," => i += 1,
            _ => break,
        }
    }
    Ok((out, t.get(i..).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manual_s_forms_parse() {
        let text = "# comment\na, b touch 3-5 yshift 0.5 @ 12, 14-15; l n 1\n0 d left 2, 4 (-10, 20)\nlatn dflt @ e\ne p 7 x -0.25 \\\n  @ 9";
        let c = parse(text).unwrap();
        let kinds: Vec<(&str, &ControlKind, &[u32], &[u16])> =
            c.lines.iter().map(|l| (l.glyph.as_str(), &l.kind, l.points.as_slice(), l.ppems.as_slice())).collect();
        assert_eq!(kinds.len(), 5, "{kinds:?}");
        assert_eq!(kinds[0], ("a", &ControlKind::Touch { x_shift: 0.0, y_shift: 0.5 }, &[3, 4, 5][..], &[12, 14, 15][..]));
        assert_eq!(kinds[1].0, "b");
        assert_eq!((kinds[2].0, kinds[2].1), ("l", &ControlKind::NoDir));
        assert_eq!((kinds[3].0, kinds[3].1, kinds[3].2), ("d", &ControlKind::Left, &[2, 4][..]));
        assert_eq!((kinds[4].1, kinds[4].3), (&ControlKind::Point { x_shift: -0.25, y_shift: 0.0 }, &[9][..]));
        assert_eq!(c.lines[4].line, 5);
        assert_eq!(c.ignored, vec![4]);
    }

    #[test]
    fn malformed_text_is_refused_with_its_line() {
        for (text, line) in [
            ("a frob 1", 1),
            ("\na touch 1 @", 2),
            ("a touch 1 yshift 3 @ 12", 1),
            ("a touch 1 @ 0", 1),
            ("a left", 1),
            ("a left 1 (5)", 1),
            ("a nodir 1-4000000000", 1),
            ("a nodir 9-3", 1),
            ("touch 1 @ 12", 1),
            ("a touch 1 @ 12-99999", 1),
        ] {
            match parse(text) {
                Err(HintingError::Control { line: l, .. }) => assert_eq!(l, line, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
        assert!(matches!(parse(&"a n 1\n".repeat(MAX_CONTROL_LINES + 1)), Err(HintingError::LimitExceeded { .. })));
    }
}
