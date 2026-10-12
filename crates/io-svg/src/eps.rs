//! Encapsulated PostScript artwork as glyph paths.
//!
//! An EPS file is a program and the input is untrusted, so this is not a PostScript interpreter.
//! It runs a fixed subset of the language: the part drawing applications use to write outlines.
//!
//! | Group | Operators |
//! |---|---|
//! | Path construction | `newpath moveto rmoveto lineto rlineto curveto rcurveto closepath` |
//! | Painting | `fill eofill stroke rectfill rectstroke`; `clip eoclip rectclip initclip` |
//! | Graphics state | `gsave grestore save restore`; colour and line style operators, read and dropped |
//! | Transforms | `translate scale rotate concat matrix currentmatrix setmatrix` |
//! | Stack and arithmetic | `pop exch dup copy index roll clear mark cleartomark [ ] add sub mul div neg abs` |
//! | Definitions | `def bind load array astore dict begin end userdict globaldict`; `showpage` |
//!
//! There are no conditionals, loops, `exec`, files, fonts, text or images. A file that uses any
//! other operator is refused with its name rather than imported wrongly.
//!
//! - **Sections.** The prolog, the setup, procedure sets, resources, fonts, features and defaults
//!   (`%%BeginProlog` … `%%EndProlog` and the like) are not run. They are scanned for
//!   definitions of the form `/name { … } bind def`, `/name /operator load def` and
//!   `/name number def`; a procedure is kept when every name in it is an operator of the subset
//!   or a name kept before it. The page description runs; it may call kept procedures and define
//!   its own, which run in the subset too. Reading stops at `%%PageTrailer`, `%%Trailer` or
//!   `%%EOF`.
//! - **Abbreviations.** A file in the drawing-application format that marks itself with `%AI`
//!   comments may use that format's documented one- and two-letter operators when its prolog
//!   defines them with procedures beyond the subset: `m l L c C v V y Y` build paths, `f F s S b B
//!   n N` paint (the lower-case forms close the path first), `W` clips, and the colour, line
//!   style, text, group and layer operators are dropped together with their operands.
//! - **Painting.** A filled path gives closed contours; a stroked one gives its centre line, open
//!   unless closed with `closepath`. A path painted twice (`gsave fill grestore stroke`) is read
//!   once. A path used only to clip, or ended with `newpath` unpainted, is left out and counted in
//!   [`EpsOutlines::dropped`].
//! - **Placement.** PostScript's y axis points up, like a glyph's. The frame is the
//!   `%%HiResBoundingBox`, else the `%%BoundingBox`, else the bounds of the drawing.
//!   [`Placement::Unscaled`] keeps one point per font unit and moves the frame's top edge to
//!   `top`; [`Placement::FitEm`] scales the frame's height to the em, with its top edge at the
//!   ascender and its left edge at x = 0.
//!
//! DOS EPS files (a binary header with a preview) are read through the header's PostScript
//! section. Every count read from the file has a limit below.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fontcraft_model::{MAX_COORDINATE, Path};

use crate::read::{Collected, Contour, Pt, round};
use crate::{Dropped, MAX_SVG_NODES, MAX_SVG_PATHS, Placement, SvgError};

/// Largest EPS input, in bytes.
pub const MAX_EPS_BYTES: usize = 16 << 20;
/// Most tokens read from the file.
pub const MAX_EPS_TOKENS: usize = 8_000_000;
/// Most tokens held in procedures, over the whole file.
pub const MAX_EPS_STORED_TOKENS: usize = 2_000_000;
/// Most bytes in a name.
pub const MAX_EPS_NAME_BYTES: usize = 255;
/// Deepest nesting of procedures in the text, and of skipped sections.
pub const MAX_EPS_NESTING: usize = 64;
/// Deepest nesting of procedure calls while running.
pub const MAX_EPS_CALL_DEPTH: usize = 32;
/// Most `gsave` and `save` levels open at once.
pub const MAX_EPS_GSAVE_DEPTH: usize = 64;
/// Most values on the operand stack.
pub const MAX_EPS_STACK: usize = 10_000;
/// Most operations run: every token executed, plus the values copied by stack operators.
pub const MAX_EPS_OPERATIONS: usize = 10_000_000;
/// Most array elements created, plus path segments copied away from a saved graphics state,
/// over the whole file.
pub const MAX_EPS_ALLOCATED: usize = 1_000_000;
/// Most names defined.
pub const MAX_EPS_DEFINITIONS: usize = 10_000;
/// Largest absolute value of a number, read or computed, and of a transformed coordinate.
pub const MAX_EPS_NUMBER: f64 = 1e9;
/// Most segments in the path being built.
pub const MAX_EPS_PATH_SEGMENTS: usize = MAX_SVG_NODES;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EpsError {
    #[error("the EPS file is {size} bytes; the limit is {MAX_EPS_BYTES}")]
    TooLarge { size: usize },
    #[error("the file is not PostScript: it does not start with `%!`")]
    NotPostScript,
    #[error("the file's binary header points outside the file")]
    BadBinaryHeader,
    #[error("the PostScript cannot be read: {0}")]
    Syntax(String),
    #[error("the file has more than {MAX_EPS_TOKENS} tokens, or its procedures hold more than {MAX_EPS_STORED_TOKENS}")]
    TooManyTokens,
    #[error("the file nests procedures, sections, calls or saved graphics states more than {MAX_EPS_NESTING} deep")]
    TooDeep,
    #[error("the file needs more than {MAX_EPS_STACK} values on the operand stack")]
    StackOverflow,
    #[error("`{op}` needs more operands than the stack holds")]
    StackUnderflow { op: String },
    #[error("`{op}` was given an operand of the wrong type or value")]
    TypeCheck { op: String },
    #[error("`{op}` divides by zero")]
    DivideByZero { op: String },
    #[error("`{op}` needs a current point, and there is none")]
    NoCurrentPoint { op: String },
    #[error("the file uses `{op}`, which is beyond the outline operators this reader runs; save the artwork as plain outlines and try again")]
    Unsupported { op: String },
    #[error("the file runs more than {MAX_EPS_OPERATIONS} operations")]
    TooMuchWork,
    #[error("the file creates more than {MAX_EPS_ALLOCATED} array elements and copied path segments, or more than {MAX_EPS_DEFINITIONS} definitions")]
    TooMuchData,
    #[error("the file has a number beyond ±{MAX_EPS_NUMBER}, or one that is not a number")]
    NumberOutOfRange,
    #[error("the drawing has more than {MAX_SVG_PATHS} paths")]
    TooManyPaths,
    #[error("the drawing has more than {MAX_SVG_NODES} nodes")]
    TooManyNodes,
    #[error("a point of the drawing lands beyond ±{MAX_COORDINATE} font units")]
    CoordinateOutOfRange,
    #[error("the placement is not usable: {0}")]
    BadPlacement(String),
}

impl From<SvgError> for EpsError {
    fn from(e: SvgError) -> Self {
        match e {
            SvgError::TooManyPaths => EpsError::TooManyPaths,
            SvgError::TooManyNodes => EpsError::TooManyNodes,
            SvgError::CoordinateOutOfRange => EpsError::CoordinateOutOfRange,
            SvgError::BadPlacement(s) => EpsError::BadPlacement(s),
            other => EpsError::Syntax(other.to_string()),
        }
    }
}

/// What [`read_eps`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct EpsOutlines {
    /// The contours, in painting order: closed or open, lines and cubic curves, on-curve nodes
    /// marked smooth where the curve runs through them without a corner. Coordinates are rounded
    /// to 0.001 units.
    pub paths: Vec<Path>,
    /// The file's bounding box as `[llx, lly, urx, ury]` in points, the high-resolution one when
    /// the file has it.
    pub bounding_box: Option<[f64; 4]>,
    /// What was left out because it is not geometry: clipping and unpainted paths, text and
    /// images (as elements), colour and line style (as styling).
    pub dropped: Dropped,
}

/// Does `data` look like PostScript (a `%!` header, or a DOS EPS binary header)?
pub fn looks_like_eps(data: &[u8]) -> bool {
    data.starts_with(&DOS_MAGIC) || after_leading_space(data).starts_with(b"%!")
}

/// The outlines of an EPS file, placed by `placement`. See the module documentation for the
/// subset of PostScript that runs.
pub fn read_eps(data: &[u8], placement: Placement) -> Result<EpsOutlines, EpsError> {
    if data.len() > MAX_EPS_BYTES {
        return Err(EpsError::TooLarge { size: data.len() });
    }
    let src = after_leading_space(postscript_section(data)?);
    if !src.starts_with(b"%!") {
        return Err(EpsError::NotPostScript);
    }
    let header = Header::read(src);
    let mut run = Interp::new(header.marked_abbreviations);
    let mut scanner = Scanner { src, pos: 0, tokens: 0, stored: 0 };
    let mut skipping = 0usize;
    let mut window: Vec<Pending> = Vec::new();
    while let Some(raw) = scanner.next_raw()? {
        match raw {
            Raw::Dsc(Dsc::Stop) => break,
            Raw::Dsc(Dsc::Begin) => {
                skipping = skipping.saturating_add(1);
                if skipping > MAX_EPS_NESTING {
                    return Err(EpsError::TooDeep);
                }
                window.clear();
            }
            Raw::Dsc(Dsc::End) => {
                skipping = skipping.saturating_sub(1);
                window.clear();
            }
            Raw::Token(t) if skipping > 0 => run.scan_definition(&mut window, t)?,
            Raw::Token(t) => run.exec(&t, 0)?,
            // The scanner gives `}` only inside a procedure.
            Raw::Close => {}
        }
    }
    let bounding_box = header.hires_box.or(header.bounding_box);
    run.finish(bounding_box, placement)
}

// ───────────────────────────── Input framing ─────────────────────────────

const DOS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];

/// The PostScript section of a DOS EPS file (a binary header giving its offset and length, little
/// endian, after the magic number), or the whole input.
fn postscript_section(data: &[u8]) -> Result<&[u8], EpsError> {
    if !data.starts_with(&DOS_MAGIC) {
        return Ok(data);
    }
    let word = |at: usize| data.get(at..at.saturating_add(4)).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes);
    let (Some(offset), Some(length)) = (word(4), word(8)) else {
        return Err(EpsError::BadBinaryHeader);
    };
    let start = usize::try_from(offset).map_err(|_| EpsError::BadBinaryHeader)?;
    let length = usize::try_from(length).map_err(|_| EpsError::BadBinaryHeader)?;
    let end = start.checked_add(length).ok_or(EpsError::BadBinaryHeader)?;
    data.get(start..end).ok_or(EpsError::BadBinaryHeader)
}

/// `data` without leading white space and the end-of-job character some spoolers put first.
fn after_leading_space(data: &[u8]) -> &[u8] {
    let first = data.iter().position(|&c| !(c.is_ascii_whitespace() || c == 0x04)).unwrap_or(data.len());
    data.get(first..).unwrap_or_default()
}

/// What the comments say about the file before it runs.
struct Header {
    bounding_box: Option<[f64; 4]>,
    hires_box: Option<[f64; 4]>,
    /// The file marks itself with `%AI` comments, so the drawing-application abbreviations apply.
    marked_abbreviations: bool,
}

impl Header {
    /// The first usable bounding box of each kind, wherever it is (`(atend)` defers it to the
    /// trailer).
    fn read(src: &[u8]) -> Header {
        let mut h = Header { bounding_box: None, hires_box: None, marked_abbreviations: false };
        for line in src.split(|&c| c == b'\n' || c == b'\r') {
            if line.starts_with(b"%AI") || line.starts_with(b"%%AI") {
                h.marked_abbreviations = true;
            } else if let Some(rest) = line.strip_prefix(b"%%HiResBoundingBox:") {
                h.hires_box = h.hires_box.or_else(|| parse_box(rest));
            } else if let Some(rest) = line.strip_prefix(b"%%BoundingBox:") {
                h.bounding_box = h.bounding_box.or_else(|| parse_box(rest));
            }
        }
        h
    }
}

fn parse_box(rest: &[u8]) -> Option<[f64; 4]> {
    let text = std::str::from_utf8(rest).ok()?;
    let mut values = text.split_ascii_whitespace().map(|w| number(w.as_bytes()));
    let mut b = [0.0; 4];
    for slot in &mut b {
        *slot = values.next()??;
    }
    let [llx, lly, urx, ury] = b;
    let usable = b.iter().all(|v| v.is_finite() && v.abs() <= MAX_EPS_NUMBER) && urx >= llx && ury >= lly;
    usable.then_some(b)
}

// ───────────────────────────── Scanner ─────────────────────────────

#[derive(Clone, Debug)]
enum Token {
    Num(f64),
    /// `/name`
    Lit(Rc<str>),
    /// An executable name, `[`, `]`, `<<`, `>>`, or `//name`.
    Name(Rc<str>),
    /// A string; its contents are never needed.
    Str,
    Proc(Rc<[Token]>),
}

/// A document structuring comment that changes how the file is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dsc {
    /// A section that is scanned for definitions but not run begins.
    Begin,
    End,
    /// The page description is over.
    Stop,
}

enum Raw {
    Token(Token),
    /// `}` inside a procedure.
    Close,
    Dsc(Dsc),
}

struct Scanner<'a> {
    src: &'a [u8],
    pos: usize,
    tokens: usize,
    stored: usize,
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0)
}

fn is_delimiter(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn syntax(what: &str) -> EpsError {
    EpsError::Syntax(what.into())
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<u8> {
        self.src.get(self.pos.saturating_add(1)).copied()
    }

    fn next_raw(&mut self) -> Result<Option<Raw>, EpsError> {
        self.raw(0)
    }

    /// The next token at procedure nesting `depth` (0 outside procedures). Comments are skipped;
    /// structuring comments that start a line are reported outside procedures.
    fn raw(&mut self, depth: usize) -> Result<Option<Raw>, EpsError> {
        loop {
            while self.peek().is_some_and(is_space) {
                self.pos += 1;
            }
            let Some(c) = self.peek() else { return Ok(None) };
            if c == b'%' {
                let line_start = self.pos == 0 || matches!(self.pos.checked_sub(1).and_then(|i| self.src.get(i)), Some(b'\n' | b'\r'));
                let start = self.pos;
                while self.peek().is_some_and(|c| c != b'\n' && c != b'\r') {
                    self.pos += 1;
                }
                if line_start
                    && depth == 0
                    && let Some(d) = self.src.get(start..self.pos).and_then(dsc)
                {
                    return Ok(Some(Raw::Dsc(d)));
                }
                continue;
            }
            self.tokens = self.tokens.saturating_add(1);
            if self.tokens > MAX_EPS_TOKENS {
                return Err(EpsError::TooManyTokens);
            }
            let token = match c {
                b'(' => {
                    self.string()?;
                    Token::Str
                }
                b')' => return Err(syntax("a `)` closes no string")),
                b'<' => match self.peek_next() {
                    Some(b'<') => {
                        self.pos = self.pos.saturating_add(2);
                        Token::Name("<<".into())
                    }
                    Some(b'~') => {
                        self.skip_past(b"~>", "an ASCII85 string is not closed")?;
                        Token::Str
                    }
                    _ => {
                        self.skip_past(b">", "a hexadecimal string is not closed")?;
                        Token::Str
                    }
                },
                b'>' => {
                    if self.peek_next() != Some(b'>') {
                        return Err(syntax("a `>` closes no string"));
                    }
                    self.pos = self.pos.saturating_add(2);
                    Token::Name(">>".into())
                }
                b'[' | b']' => {
                    self.pos += 1;
                    Token::Name(if c == b'[' { "[".into() } else { "]".into() })
                }
                b'{' => {
                    self.pos += 1;
                    Token::Proc(self.procedure(depth.saturating_add(1))?)
                }
                b'}' => {
                    self.pos += 1;
                    if depth > 0 {
                        return Ok(Some(Raw::Close));
                    }
                    return Err(syntax("a `}` closes no procedure"));
                }
                b'/' => {
                    self.pos += 1;
                    let immediate = self.peek() == Some(b'/');
                    if immediate {
                        self.pos += 1;
                    }
                    let word = self.word()?;
                    // `//name` is looked up when it runs, which is the same for this subset.
                    if immediate { Token::Name(name(word)) } else { Token::Lit(name(word)) }
                }
                _ => {
                    let word = self.word()?;
                    match number(word) {
                        Some(v) => Token::Num(v),
                        None => Token::Name(name(word)),
                    }
                }
            };
            return Ok(Some(Raw::Token(token)));
        }
    }

    /// The tokens up to the matching `}`; the `{` has been read.
    fn procedure(&mut self, depth: usize) -> Result<Rc<[Token]>, EpsError> {
        if depth > MAX_EPS_NESTING {
            return Err(EpsError::TooDeep);
        }
        let mut body = Vec::new();
        loop {
            match self.raw(depth)? {
                Some(Raw::Token(t)) => {
                    self.stored = self.stored.saturating_add(1);
                    if self.stored > MAX_EPS_STORED_TOKENS {
                        return Err(EpsError::TooManyTokens);
                    }
                    body.push(t);
                }
                Some(Raw::Close) => return Ok(body.into()),
                Some(Raw::Dsc(_)) => {}
                None => return Err(syntax("a procedure is not closed")),
            }
        }
    }

    /// Skips a `( … )` string with balanced parentheses and backslash escapes.
    fn string(&mut self) -> Result<(), EpsError> {
        self.pos += 1;
        let mut open = 1usize;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'\\' => self.pos = self.pos.saturating_add(1),
                b'(' => open = open.saturating_add(1),
                b')' => {
                    open -= 1;
                    if open == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(syntax("a string is not closed"))
    }

    fn skip_past(&mut self, end: &[u8], what: &str) -> Result<(), EpsError> {
        let rest = self.src.get(self.pos..).unwrap_or_default();
        let at = rest.windows(end.len()).position(|w| w == end).ok_or_else(|| syntax(what))?;
        self.pos = self.pos.saturating_add(at).saturating_add(end.len());
        Ok(())
    }

    /// A run of regular characters.
    fn word(&mut self) -> Result<&[u8], EpsError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| !is_space(c) && !is_delimiter(c)) {
            self.pos += 1;
        }
        if self.pos - start > MAX_EPS_NAME_BYTES {
            return Err(EpsError::Syntax(format!("a name is longer than {MAX_EPS_NAME_BYTES} bytes")));
        }
        Ok(self.src.get(start..self.pos).unwrap_or_default())
    }
}

fn name(word: &[u8]) -> Rc<str> {
    String::from_utf8_lossy(word).as_ref().into()
}

/// The structuring comments that matter, by their keyword.
fn dsc(line: &[u8]) -> Option<Dsc> {
    const SKIPPED: [&str; 7] = ["Prolog", "Setup", "ProcSet", "Resource", "Font", "Feature", "Defaults"];
    let rest = line.strip_prefix(b"%%")?;
    let keyword_end = rest.iter().position(|&c| c == b':' || is_space(c)).unwrap_or(rest.len());
    let keyword = std::str::from_utf8(rest.get(..keyword_end)?).ok()?;
    if let Some(section) = keyword.strip_prefix("Begin")
        && SKIPPED.iter().any(|s| s.eq_ignore_ascii_case(section))
    {
        return Some(Dsc::Begin);
    }
    if let Some(section) = keyword.strip_prefix("End")
        && SKIPPED.iter().any(|s| s.eq_ignore_ascii_case(section))
    {
        return Some(Dsc::End);
    }
    matches!(keyword, "PageTrailer" | "Trailer" | "EOF").then_some(Dsc::Stop)
}

/// A PostScript number: an integer, a real (`-.5`, `1.`, `2e-3`) or a radix number (`16#FF`).
/// Words like `inf` and `nan` are names.
fn number(w: &[u8]) -> Option<f64> {
    if let Some(hash) = w.iter().position(|&c| c == b'#') {
        let base: u32 = std::str::from_utf8(w.get(..hash)?).ok()?.parse().ok()?;
        let digits = w.get(hash.saturating_add(1)..)?;
        if !(2..=36).contains(&base) || digits.is_empty() {
            return None;
        }
        let mut v = 0.0f64;
        for &d in digits {
            v = v * f64::from(base) + f64::from(char::from(d).to_digit(base)?);
        }
        return Some(v);
    }
    let text = std::str::from_utf8(w).ok()?;
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (body.get(..i)?, Some(body.get(i.saturating_add(1)..)?)),
        None => (body, None),
    };
    let digits = mantissa.bytes().filter(u8::is_ascii_digit).count();
    let dots = mantissa.bytes().filter(|&c| c == b'.').count();
    if digits == 0 || dots > 1 || digits + dots != mantissa.len() {
        return None;
    }
    if let Some(e) = exponent {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        if e.is_empty() || !e.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    text.parse::<f64>().ok()
}

// ───────────────────────────── Operators ─────────────────────────────

/// Styling read and dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Styling {
    Colour,
    LineWidth,
    LineStyle,
    /// Not reported (overprint and the like).
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Newpath,
    Moveto,
    Rmoveto,
    Lineto,
    Rlineto,
    Curveto,
    Rcurveto,
    Closepath,
    Fill,
    Stroke,
    Clip,
    Rectfill,
    Rectstroke,
    Rectclip,
    Nothing,
    Gsave,
    Grestore,
    Save,
    Restore,
    /// Pops this many operands and drops the styling.
    Style(usize, Styling),
    Setcolor,
    Translate,
    Scale,
    Rotate,
    Concat,
    Matrix,
    Currentmatrix,
    Setmatrix,
    Pop,
    Exch,
    Dup,
    Copy,
    Index,
    Roll,
    Clear,
    Mark,
    Cleartomark,
    ArrayEnd,
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Abs,
    Def,
    Bind,
    Load,
    Array,
    Astore,
    Dict,
    Begin,
    PushNull,
}

/// The operators of the subset, by name.
fn builtin(name: &str) -> Option<Op> {
    Some(match name {
        "newpath" => Op::Newpath,
        "moveto" => Op::Moveto,
        "rmoveto" => Op::Rmoveto,
        "lineto" => Op::Lineto,
        "rlineto" => Op::Rlineto,
        "curveto" => Op::Curveto,
        "rcurveto" => Op::Rcurveto,
        "closepath" => Op::Closepath,
        "fill" | "eofill" => Op::Fill,
        "stroke" => Op::Stroke,
        "clip" | "eoclip" => Op::Clip,
        "rectfill" => Op::Rectfill,
        "rectstroke" => Op::Rectstroke,
        "rectclip" => Op::Rectclip,
        "initclip" | "showpage" | "end" => Op::Nothing,
        "gsave" => Op::Gsave,
        "grestore" => Op::Grestore,
        "save" => Op::Save,
        "restore" => Op::Restore,
        "setgray" | "setcolorspace" => Op::Style(1, Styling::Colour),
        "setrgbcolor" | "sethsbcolor" => Op::Style(3, Styling::Colour),
        "setcmykcolor" => Op::Style(4, Styling::Colour),
        "setcolor" => Op::Setcolor,
        "setlinewidth" => Op::Style(1, Styling::LineWidth),
        "setlinecap" | "setlinejoin" | "setmiterlimit" | "setflat" | "setstrokeadjust" => Op::Style(1, Styling::LineStyle),
        "setdash" => Op::Style(2, Styling::LineStyle),
        "setoverprint" => Op::Style(1, Styling::Quiet),
        "translate" => Op::Translate,
        "scale" => Op::Scale,
        "rotate" => Op::Rotate,
        "concat" => Op::Concat,
        "matrix" => Op::Matrix,
        "currentmatrix" => Op::Currentmatrix,
        "setmatrix" => Op::Setmatrix,
        "pop" => Op::Pop,
        "exch" => Op::Exch,
        "dup" => Op::Dup,
        "copy" => Op::Copy,
        "index" => Op::Index,
        "roll" => Op::Roll,
        "clear" => Op::Clear,
        "mark" | "[" => Op::Mark,
        "cleartomark" => Op::Cleartomark,
        "]" => Op::ArrayEnd,
        "add" => Op::Add,
        "sub" => Op::Sub,
        "mul" => Op::Mul,
        "div" => Op::Div,
        "neg" => Op::Neg,
        "abs" => Op::Abs,
        "def" => Op::Def,
        "bind" => Op::Bind,
        "load" => Op::Load,
        "array" => Op::Array,
        "astore" => Op::Astore,
        "dict" => Op::Dict,
        "begin" => Op::Begin,
        "userdict" | "globaldict" => Op::PushNull,
        _ => return None,
    })
}

/// How a drawing-application abbreviation paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Paint {
    Fill,
    Stroke,
    None,
}

/// What an abbreviation drops along with every operand on the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dropping {
    Styling(Styling),
    Text,
    Image,
    Structure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Abbrev {
    Move,
    Line,
    Curve,
    /// `x2 y2 x3 y3 v`: the first control point is the current point.
    CurveV,
    /// `x1 y1 x3 y3 y`: the second control point is the end point.
    CurveY,
    Close,
    Paint {
        close: bool,
        paint: Paint,
    },
    Clip,
    Drop(Dropping),
}

/// The documented operators of the drawing-application format.
fn abbreviation(name: &str) -> Option<Abbrev> {
    let paint = |close, paint| Abbrev::Paint { close, paint };
    Some(match name {
        "m" => Abbrev::Move,
        "l" | "L" => Abbrev::Line,
        "c" | "C" => Abbrev::Curve,
        "v" | "V" => Abbrev::CurveV,
        "y" | "Y" => Abbrev::CurveY,
        "h" | "H" => Abbrev::Close,
        "f" => paint(true, Paint::Fill),
        "F" => paint(false, Paint::Fill),
        "s" => paint(true, Paint::Stroke),
        "S" => paint(false, Paint::Stroke),
        "b" => paint(true, Paint::Fill),
        "B" => paint(false, Paint::Fill),
        "n" => paint(true, Paint::None),
        "N" => paint(false, Paint::None),
        "W" => Abbrev::Clip,
        "g" | "G" | "k" | "K" | "x" | "X" | "Xa" | "XA" | "Xx" | "XX" | "Xk" | "XK" | "p" | "P" => Abbrev::Drop(Dropping::Styling(Styling::Colour)),
        "w" => Abbrev::Drop(Dropping::Styling(Styling::LineWidth)),
        "j" | "J" | "M" | "d" | "i" | "D" => Abbrev::Drop(Dropping::Styling(Styling::LineStyle)),
        "To" | "TO" | "Tp" | "TP" | "Tm" | "Td" | "T*" | "Tr" | "TR" | "Tf" | "Ta" | "Tq" | "Tl" | "Tt" | "Tw" | "Tc" | "Ti" | "Tz" | "TA" | "Ts"
        | "Tx" | "TX" | "Tj" | "Tk" | "Tu" | "TV" | "Tv" => Abbrev::Drop(Dropping::Text),
        "XI" | "XG" | "XF" => Abbrev::Drop(Dropping::Image),
        "u" | "U" | "*u" | "*U" | "q" | "Q" | "A" | "O" | "R" | "XR" | "Ln" | "Lb" | "LB" | "Ar" | "XW" | "Xw" | "Xy" | "XY" | "Ap" => {
            Abbrev::Drop(Dropping::Structure)
        }
        _ => return None,
    })
}

/// A name as shown in a message: no control characters, at most 64 characters.
fn shown(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(64).collect()
}

// ───────────────────────────── Interpreter ─────────────────────────────

#[derive(Clone, Debug)]
enum Value {
    Num(f64),
    Lit(Rc<str>),
    Str,
    Null,
    Mark,
    Array(Rc<Vec<Value>>),
    Proc(Rc<[Token]>),
    /// An operator obtained with `load`, with its name.
    Op(Op, Rc<str>),
    /// What `save` returns: the number of graphics states saved before it.
    Save(usize),
}

/// A path segment in page space (the default coordinate system, in points).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Seg {
    Move(Pt),
    Line(Pt),
    Curve(Pt, Pt, Pt),
    Close,
}

type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` then `n`: the matrix that applies `m` first.
fn multiply(m: Matrix, n: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = m;
    let [na, nb, nc, nd, ne, nf] = n;
    [a * na + b * nc, a * nb + b * nd, c * na + d * nc, c * nb + d * nd, e * na + f * nc + ne, e * nb + f * nd + nf]
}

/// Refuses an array among the elements of a new array. Arrays here only hold matrices and
/// similar flat lists; refusing nesting keeps every value shallow, so dropping one never
/// recurses deeply.
fn flat(op: &str, items: &[Value]) -> Result<()> {
    if items.iter().any(|v| matches!(v, Value::Array(_))) { Err(EpsError::TypeCheck { op: shown(op) }) } else { Ok(()) }
}

fn usable(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_EPS_NUMBER
}

#[derive(Clone)]
struct GState {
    ctm: Matrix,
    /// Shared with saved states until either side changes it.
    path: Rc<Vec<Seg>>,
    /// Changes with every edit, so a path restored by `grestore` is known to be painted already.
    path_id: u64,
    point: Option<Pt>,
    subpath_start: Option<Pt>,
}

struct Painted {
    segs: Vec<Seg>,
    fill: bool,
}

/// A definition seen in a skipped section, waiting for its `def`.
enum Pending {
    Lit(Rc<str>),
    Proc(Rc<[Token]>),
    Num(f64),
    Load(Rc<str>),
}

struct Interp {
    stack: Vec<Value>,
    defs: HashMap<Rc<str>, Value>,
    gs: GState,
    saved: Vec<GState>,
    work: usize,
    allocated: usize,
    next_id: u64,
    emitted: HashSet<u64>,
    clipped: HashSet<u64>,
    painted: Vec<Painted>,
    painted_nodes: usize,
    painted_paths: usize,
    dropped: Dropped,
    abbreviations: bool,
}

type Result<T, E = EpsError> = std::result::Result<T, E>;

impl Interp {
    fn new(abbreviations: bool) -> Interp {
        Interp {
            stack: Vec::new(),
            defs: HashMap::new(),
            gs: GState { ctm: IDENTITY, path: Rc::default(), path_id: 0, point: None, subpath_start: None },
            saved: Vec::new(),
            work: 0,
            allocated: 0,
            next_id: 0,
            emitted: HashSet::new(),
            clipped: HashSet::new(),
            painted: Vec::new(),
            painted_nodes: 0,
            painted_paths: 0,
            dropped: Dropped::default(),
            abbreviations,
        }
    }

    fn charge(&mut self, n: usize) -> Result<()> {
        self.work = self.work.saturating_add(n);
        if self.work > MAX_EPS_OPERATIONS { Err(EpsError::TooMuchWork) } else { Ok(()) }
    }

    fn allocate(&mut self, n: usize) -> Result<()> {
        self.allocated = self.allocated.saturating_add(n);
        if self.allocated > MAX_EPS_ALLOCATED || n > MAX_EPS_STACK { Err(EpsError::TooMuchData) } else { Ok(()) }
    }

    fn drop_element(&mut self, what: &str) {
        let n = self.dropped.elements.entry(what.into()).or_default();
        *n = n.saturating_add(1);
    }

    fn drop_styling(&mut self, kind: Styling) {
        let what = match kind {
            Styling::Colour => "colour",
            Styling::LineWidth => "line width",
            Styling::LineStyle => "line style",
            Styling::Quiet => return,
        };
        let n = self.dropped.attributes.entry(what.into()).or_default();
        *n = n.saturating_add(1);
    }

    // ── The skipped sections ──

    fn scan_definition(&mut self, window: &mut Vec<Pending>, t: Token) -> Result<()> {
        self.charge(1)?;
        fn keep(window: &mut Vec<Pending>, p: Pending) {
            window.push(p);
            if window.len() > 2 {
                window.remove(0);
            }
        }
        match t {
            Token::Lit(n) => keep(window, Pending::Lit(n)),
            Token::Proc(b) => keep(window, Pending::Proc(b)),
            Token::Num(v) => keep(window, Pending::Num(v)),
            Token::Name(n) if &*n == "bind" => {
                if !matches!(window.last(), Some(Pending::Proc(_))) {
                    window.clear();
                }
            }
            Token::Name(n) if &*n == "load" => match window.pop() {
                Some(Pending::Lit(b)) => window.push(Pending::Load(b)),
                _ => window.clear(),
            },
            Token::Name(n) if &*n == "def" => {
                if let [Pending::Lit(key), value] = window.as_slice() {
                    let value = match value {
                        Pending::Proc(b) if self.runs_in_subset(b, 0)? => Some(Value::Proc(b.clone())),
                        Pending::Load(b) => match self.defs.get(b) {
                            Some(v) => Some(v.clone()),
                            None => builtin(b).map(|op| Value::Op(op, b.clone())),
                        },
                        Pending::Num(v) if usable(*v) => Some(Value::Num(*v)),
                        Pending::Lit(b) => Some(Value::Lit(b.clone())),
                        _ => None,
                    };
                    match value {
                        Some(v) => self.define(key.clone(), v)?,
                        // A definition beyond the subset hides nothing kept before it.
                        None => {
                            self.defs.remove(key);
                        }
                    }
                }
                window.clear();
            }
            _ => window.clear(),
        }
        Ok(())
    }

    /// Does every name in `body` (and in the procedures inside it) name an operator of the
    /// subset or a definition already kept?
    fn runs_in_subset(&mut self, body: &[Token], depth: usize) -> Result<bool> {
        if depth > MAX_EPS_NESTING {
            return Err(EpsError::TooDeep);
        }
        self.charge(body.len())?;
        for t in body {
            let ok = match t {
                Token::Name(n) => builtin(n).is_some() || self.defs.contains_key(n),
                Token::Proc(inner) => self.runs_in_subset(inner, depth.saturating_add(1))?,
                _ => true,
            };
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn define(&mut self, key: Rc<str>, value: Value) -> Result<()> {
        if !self.defs.contains_key(&key) && self.defs.len() >= MAX_EPS_DEFINITIONS {
            return Err(EpsError::TooMuchData);
        }
        self.defs.insert(key, value);
        Ok(())
    }

    // ── Running ──

    fn exec(&mut self, t: &Token, depth: usize) -> Result<()> {
        self.charge(1)?;
        match t {
            Token::Num(v) => self.push(Value::Num(*v)),
            Token::Lit(n) => self.push(Value::Lit(n.clone())),
            Token::Str => self.push(Value::Str),
            Token::Proc(b) => self.push(Value::Proc(b.clone())),
            Token::Name(n) => self.call(n, depth),
        }
    }

    fn call(&mut self, name: &Rc<str>, depth: usize) -> Result<()> {
        match self.defs.get(name).cloned() {
            Some(Value::Proc(body)) => {
                if depth >= MAX_EPS_CALL_DEPTH {
                    return Err(EpsError::TooDeep);
                }
                for t in body.iter() {
                    self.exec(t, depth.saturating_add(1))?;
                }
                Ok(())
            }
            Some(Value::Op(op, n)) => self.run(op, &n),
            Some(v) => self.push(v),
            None => match builtin(name) {
                Some(op) => self.run(op, name),
                None => match abbreviation(name).filter(|_| self.abbreviations) {
                    Some(a) => self.run_abbreviation(a, name),
                    None => Err(EpsError::Unsupported { op: shown(name) }),
                },
            },
        }
    }

    fn push(&mut self, v: Value) -> Result<()> {
        if let Value::Num(n) = v
            && !usable(n)
        {
            return Err(EpsError::NumberOutOfRange);
        }
        if self.stack.len() >= MAX_EPS_STACK {
            return Err(EpsError::StackOverflow);
        }
        self.stack.push(v);
        Ok(())
    }

    fn pop(&mut self, op: &str) -> Result<Value> {
        self.stack.pop().ok_or_else(|| EpsError::StackUnderflow { op: shown(op) })
    }

    fn num(&mut self, op: &str) -> Result<f64> {
        match self.pop(op)? {
            Value::Num(v) => Ok(v),
            _ => Err(EpsError::TypeCheck { op: shown(op) }),
        }
    }

    /// `N` numbers, in the order they were pushed.
    fn nums<const N: usize>(&mut self, op: &str) -> Result<[f64; N]> {
        let mut out = [0.0; N];
        for slot in out.iter_mut().rev() {
            *slot = self.num(op)?;
        }
        Ok(out)
    }

    /// A non-negative whole number used as a count.
    fn count(&mut self, op: &str) -> Result<usize> {
        let v = self.num(op)?;
        if v < 0.0 || v.fract() != 0.0 {
            return Err(EpsError::TypeCheck { op: shown(op) });
        }
        // `push` keeps numbers within MAX_EPS_NUMBER, so the conversion is exact.
        Ok(v as usize)
    }

    fn matrix_operand(&mut self, op: &str) -> Result<Matrix> {
        let Value::Array(a) = self.pop(op)? else {
            return Err(EpsError::TypeCheck { op: shown(op) });
        };
        matrix_of(&a).ok_or_else(|| EpsError::TypeCheck { op: shown(op) })
    }

    fn push_matrix(&mut self, m: Matrix) -> Result<()> {
        self.allocate(6)?;
        self.push(Value::Array(Rc::new(m.iter().map(|&v| Value::Num(v)).collect())))
    }

    /// Applies `m` before the current transformation (`concat`).
    fn concat(&mut self, m: Matrix) -> Result<()> {
        let ctm = multiply(m, self.gs.ctm);
        if !ctm.iter().all(|&v| usable(v)) {
            return Err(EpsError::NumberOutOfRange);
        }
        self.gs.ctm = ctm;
        Ok(())
    }

    /// A transform operator: with a matrix on top it fills that matrix with `m` (the
    /// matrix form), otherwise it applies `m`.
    fn transform(&mut self, op: &str, operands: usize, make: impl Fn(&[f64]) -> Matrix) -> Result<()> {
        let matrix_form = matches!(self.stack.last(), Some(Value::Array(_)));
        if matrix_form {
            self.matrix_operand(op)?;
        }
        let mut values = [0.0; 2];
        let slots = values.get_mut(..operands).unwrap_or_default();
        for slot in slots.iter_mut().rev() {
            *slot = self.num(op)?;
        }
        let m = make(values.get(..operands).unwrap_or_default());
        if matrix_form { self.push_matrix(m) } else { self.concat(m) }
    }

    /// User space to page space.
    fn to_page(&self, x: f64, y: f64) -> Result<Pt> {
        let [a, b, c, d, e, f] = self.gs.ctm;
        let p = (a * x + c * y + e, b * x + d * y + f);
        if usable(p.0) && usable(p.1) { Ok(p) } else { Err(EpsError::NumberOutOfRange) }
    }

    /// A user-space displacement to the current point, in page space.
    fn relative(&self, op: &str, dx: f64, dy: f64) -> Result<Pt> {
        let (x, y) = self.gs.point.ok_or_else(|| EpsError::NoCurrentPoint { op: shown(op) })?;
        let [a, b, c, d, _, _] = self.gs.ctm;
        let p = (x + a * dx + c * dy, y + b * dx + d * dy);
        if usable(p.0) && usable(p.1) { Ok(p) } else { Err(EpsError::NumberOutOfRange) }
    }

    fn edit_path(&mut self, edit: impl FnOnce(&mut Vec<Seg>)) -> Result<()> {
        let len = self.gs.path.len();
        if len >= MAX_EPS_PATH_SEGMENTS {
            return Err(EpsError::TooManyNodes);
        }
        if Rc::strong_count(&self.gs.path) > 1 {
            // The edit copies a path shared with a saved state. Each saved state can keep its own
            // copy alive, so the copy counts as allocated memory as well as work.
            self.charge(len)?;
            self.allocated = self.allocated.saturating_add(len);
            if self.allocated > MAX_EPS_ALLOCATED {
                return Err(EpsError::TooMuchData);
            }
        }
        edit(Rc::make_mut(&mut self.gs.path));
        self.next_id = self.next_id.wrapping_add(1);
        self.gs.path_id = self.next_id;
        Ok(())
    }

    fn move_to(&mut self, p: Pt) -> Result<()> {
        let replace = matches!(self.gs.path.last(), Some(Seg::Move(_)));
        self.edit_path(|path| {
            if replace {
                path.pop();
            }
            path.push(Seg::Move(p));
        })?;
        self.gs.point = Some(p);
        self.gs.subpath_start = Some(p);
        Ok(())
    }

    /// Appends a segment from the current point, starting a new subpath there after a
    /// `closepath`.
    fn append(&mut self, op: &str, seg: Seg, end: Pt) -> Result<()> {
        let from = self.gs.point.ok_or_else(|| EpsError::NoCurrentPoint { op: shown(op) })?;
        let restart = matches!(self.gs.path.last(), None | Some(Seg::Close));
        self.edit_path(|path| {
            if restart {
                path.push(Seg::Move(from));
            }
            path.push(seg);
        })?;
        if restart {
            self.gs.subpath_start = Some(from);
        }
        self.gs.point = Some(end);
        Ok(())
    }

    fn line_to(&mut self, op: &str, p: Pt) -> Result<()> {
        self.append(op, Seg::Line(p), p)
    }

    fn curve_to(&mut self, op: &str, c1: Pt, c2: Pt, p: Pt) -> Result<()> {
        self.append(op, Seg::Curve(c1, c2, p), p)
    }

    fn close_path(&mut self) -> Result<()> {
        if !matches!(self.gs.path.last(), None | Some(Seg::Close)) {
            self.edit_path(|path| path.push(Seg::Close))?;
            self.gs.point = self.gs.subpath_start;
        }
        Ok(())
    }

    fn reset_path(&mut self) {
        self.gs.path = Rc::default();
        self.gs.point = None;
        self.gs.subpath_start = None;
    }

    /// Ends the path without painting it.
    fn discard(&mut self) {
        let id = self.gs.path_id;
        if !self.gs.path.is_empty() && !self.emitted.contains(&id) {
            self.drop_element(if self.clipped.contains(&id) { "clipping path" } else { "unpainted path" });
        }
        self.reset_path();
    }

    fn paint(&mut self, fill: bool) -> Result<()> {
        let id = self.gs.path_id;
        if !self.gs.path.is_empty() && !self.emitted.contains(&id) {
            let segs = self.gs.path.to_vec();
            self.emit(segs, fill)?;
            self.emitted.insert(id);
        }
        self.reset_path();
        Ok(())
    }

    fn emit(&mut self, segs: Vec<Seg>, fill: bool) -> Result<()> {
        self.charge(segs.len())?;
        for s in &segs {
            match s {
                Seg::Move(_) => {
                    self.painted_paths = self.painted_paths.saturating_add(1);
                    self.painted_nodes = self.painted_nodes.saturating_add(1);
                }
                Seg::Line(_) => self.painted_nodes = self.painted_nodes.saturating_add(1),
                Seg::Curve(..) => self.painted_nodes = self.painted_nodes.saturating_add(3),
                Seg::Close => {}
            }
        }
        if self.painted_paths > MAX_SVG_PATHS {
            return Err(EpsError::TooManyPaths);
        }
        if self.painted_nodes > MAX_SVG_NODES {
            return Err(EpsError::TooManyNodes);
        }
        self.painted.push(Painted { segs, fill });
        Ok(())
    }

    fn rect(&mut self, op: &str, fill: bool) -> Result<()> {
        let [x, y, w, h] = self.nums::<4>(op)?;
        let corners = [self.to_page(x, y)?, self.to_page(x + w, y)?, self.to_page(x + w, y + h)?, self.to_page(x, y + h)?];
        let [p0, p1, p2, p3] = corners;
        self.emit(vec![Seg::Move(p0), Seg::Line(p1), Seg::Line(p2), Seg::Line(p3), Seg::Close], fill)
    }

    fn gsave(&mut self) -> Result<()> {
        if self.saved.len() >= MAX_EPS_GSAVE_DEPTH {
            return Err(EpsError::TooDeep);
        }
        self.saved.push(self.gs.clone());
        Ok(())
    }

    fn grestore(&mut self) {
        if let Some(g) = self.saved.pop() {
            self.gs = g;
        }
    }

    fn run(&mut self, op: Op, name: &str) -> Result<()> {
        match op {
            Op::Newpath => self.discard(),
            Op::Moveto => {
                let [x, y] = self.nums::<2>(name)?;
                let p = self.to_page(x, y)?;
                self.move_to(p)?;
            }
            Op::Rmoveto => {
                let [dx, dy] = self.nums::<2>(name)?;
                let p = self.relative(name, dx, dy)?;
                self.move_to(p)?;
            }
            Op::Lineto => {
                let [x, y] = self.nums::<2>(name)?;
                let p = self.to_page(x, y)?;
                self.line_to(name, p)?;
            }
            Op::Rlineto => {
                let [dx, dy] = self.nums::<2>(name)?;
                let p = self.relative(name, dx, dy)?;
                self.line_to(name, p)?;
            }
            Op::Curveto => {
                let [x1, y1, x2, y2, x3, y3] = self.nums::<6>(name)?;
                let (c1, c2, p) = (self.to_page(x1, y1)?, self.to_page(x2, y2)?, self.to_page(x3, y3)?);
                self.curve_to(name, c1, c2, p)?;
            }
            Op::Rcurveto => {
                let [x1, y1, x2, y2, x3, y3] = self.nums::<6>(name)?;
                let (c1, c2, p) = (self.relative(name, x1, y1)?, self.relative(name, x2, y2)?, self.relative(name, x3, y3)?);
                self.curve_to(name, c1, c2, p)?;
            }
            Op::Closepath => self.close_path()?,
            Op::Fill => self.paint(true)?,
            Op::Stroke => self.paint(false)?,
            Op::Clip => {
                if !self.gs.path.is_empty() {
                    self.clipped.insert(self.gs.path_id);
                }
            }
            Op::Rectfill => self.rect(name, true)?,
            Op::Rectstroke => self.rect(name, false)?,
            Op::Rectclip => {
                self.nums::<4>(name)?;
                self.drop_element("clipping path");
            }
            Op::Nothing => {}
            Op::Gsave => self.gsave()?,
            Op::Grestore => self.grestore(),
            Op::Save => {
                self.push(Value::Save(self.saved.len()))?;
                self.gsave()?;
            }
            Op::Restore => {
                let Value::Save(level) = self.pop(name)? else {
                    return Err(EpsError::TypeCheck { op: shown(name) });
                };
                if self.saved.len() > level {
                    self.saved.truncate(level.saturating_add(1));
                    self.grestore();
                }
            }
            Op::Style(operands, kind) => {
                for _ in 0..operands {
                    self.pop(name)?;
                }
                self.drop_styling(kind);
            }
            Op::Setcolor => {
                for _ in 0..4 {
                    if !matches!(self.stack.last(), Some(Value::Num(_))) {
                        break;
                    }
                    self.stack.pop();
                }
                self.drop_styling(Styling::Colour);
            }
            Op::Translate => self.transform(name, 2, |v| match v {
                [tx, ty] => [1.0, 0.0, 0.0, 1.0, *tx, *ty],
                _ => IDENTITY,
            })?,
            Op::Scale => self.transform(name, 2, |v| match v {
                [sx, sy] => [*sx, 0.0, 0.0, *sy, 0.0, 0.0],
                _ => IDENTITY,
            })?,
            Op::Rotate => self.transform(name, 1, |v| match v {
                [angle] => {
                    let (sin, cos) = angle.to_radians().sin_cos();
                    [cos, sin, -sin, cos, 0.0, 0.0]
                }
                _ => IDENTITY,
            })?,
            Op::Concat => {
                let m = self.matrix_operand(name)?;
                self.concat(m)?;
            }
            Op::Matrix => self.push_matrix(IDENTITY)?,
            Op::Currentmatrix => {
                self.matrix_operand(name)?;
                self.push_matrix(self.gs.ctm)?;
            }
            Op::Setmatrix => {
                let m = self.matrix_operand(name)?;
                self.gs.ctm = m;
            }
            Op::Pop => {
                self.pop(name)?;
            }
            Op::Exch => {
                let b = self.pop(name)?;
                let a = self.pop(name)?;
                self.push(b)?;
                self.push(a)?;
            }
            Op::Dup => {
                let v = self.stack.last().cloned().ok_or_else(|| EpsError::StackUnderflow { op: shown(name) })?;
                self.push(v)?;
            }
            Op::Copy => {
                let n = self.count(name)?;
                let len = self.stack.len();
                let from = len.checked_sub(n).ok_or_else(|| EpsError::StackUnderflow { op: shown(name) })?;
                self.charge(n)?;
                let items = self.stack.get(from..).map(<[Value]>::to_vec).unwrap_or_default();
                for v in items {
                    self.push(v)?;
                }
            }
            Op::Index => {
                let n = self.count(name)?;
                let at = self.stack.len().checked_sub(n.saturating_add(1));
                let v = at.and_then(|i| self.stack.get(i)).cloned().ok_or_else(|| EpsError::StackUnderflow { op: shown(name) })?;
                self.push(v)?;
            }
            Op::Roll => {
                let j = self.num(name)?;
                if j.fract() != 0.0 {
                    return Err(EpsError::TypeCheck { op: shown(name) });
                }
                let n = self.count(name)?;
                let from = self.stack.len().checked_sub(n).ok_or_else(|| EpsError::StackUnderflow { op: shown(name) })?;
                self.charge(n)?;
                if n > 0 {
                    // Both are within MAX_EPS_NUMBER, so the conversions are exact.
                    let k = (j as i64).rem_euclid(n as i64) as usize;
                    if let Some(top) = self.stack.get_mut(from..) {
                        top.rotate_right(k);
                    }
                }
            }
            Op::Clear => self.stack.clear(),
            Op::Mark => self.push(Value::Mark)?,
            Op::Cleartomark => {
                let at = self.stack.iter().rposition(|v| matches!(v, Value::Mark)).ok_or_else(|| EpsError::StackUnderflow { op: shown(name) })?;
                self.stack.truncate(at);
            }
            Op::ArrayEnd => {
                let at = self.stack.iter().rposition(|v| matches!(v, Value::Mark)).ok_or_else(|| EpsError::StackUnderflow { op: shown(name) })?;
                let items = self.stack.split_off(at.saturating_add(1));
                self.stack.truncate(at);
                flat(name, &items)?;
                self.charge(items.len())?;
                self.allocate(items.len())?;
                self.push(Value::Array(Rc::new(items)))?;
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div => {
                let [a, b] = self.nums::<2>(name)?;
                let v = match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    _ if b == 0.0 => return Err(EpsError::DivideByZero { op: shown(name) }),
                    _ => a / b,
                };
                self.push(Value::Num(v))?;
            }
            Op::Neg => {
                let v = self.num(name)?;
                self.push(Value::Num(-v))?;
            }
            Op::Abs => {
                let v = self.num(name)?;
                self.push(Value::Num(v.abs()))?;
            }
            Op::Def => {
                let value = self.pop(name)?;
                let Value::Lit(key) = self.pop(name)? else {
                    return Err(EpsError::TypeCheck { op: shown(name) });
                };
                self.define(key, value)?;
            }
            Op::Bind => {
                if !matches!(self.stack.last(), Some(Value::Proc(_))) {
                    return Err(EpsError::TypeCheck { op: shown(name) });
                }
            }
            Op::Load => {
                let Value::Lit(key) = self.pop(name)? else {
                    return Err(EpsError::TypeCheck { op: shown(name) });
                };
                let value = match self.defs.get(&key) {
                    Some(v) => v.clone(),
                    None => match builtin(&key) {
                        Some(op) => Value::Op(op, key.clone()),
                        None => return Err(EpsError::Unsupported { op: shown(&key) }),
                    },
                };
                self.push(value)?;
            }
            Op::Array => {
                let n = self.count(name)?;
                self.charge(n)?;
                self.allocate(n)?;
                self.push(Value::Array(Rc::new(vec![Value::Null; n])))?;
            }
            Op::Astore => {
                let Value::Array(a) = self.pop(name)? else {
                    return Err(EpsError::TypeCheck { op: shown(name) });
                };
                let from = self.stack.len().checked_sub(a.len()).ok_or_else(|| EpsError::StackUnderflow { op: shown(name) })?;
                let items = self.stack.split_off(from);
                flat(name, &items)?;
                self.charge(items.len())?;
                self.allocate(items.len())?;
                self.push(Value::Array(Rc::new(items)))?;
            }
            Op::Dict => {
                self.count(name)?;
                self.push(Value::Null)?;
            }
            Op::Begin => {
                self.pop(name)?;
            }
            Op::PushNull => self.push(Value::Null)?,
        }
        Ok(())
    }

    fn run_abbreviation(&mut self, a: Abbrev, name: &str) -> Result<()> {
        match a {
            Abbrev::Move => self.run(Op::Moveto, name),
            Abbrev::Line => self.run(Op::Lineto, name),
            Abbrev::Curve => self.run(Op::Curveto, name),
            Abbrev::CurveV => {
                let [x2, y2, x3, y3] = self.nums::<4>(name)?;
                let c1 = self.gs.point.ok_or_else(|| EpsError::NoCurrentPoint { op: shown(name) })?;
                let (c2, p) = (self.to_page(x2, y2)?, self.to_page(x3, y3)?);
                self.curve_to(name, c1, c2, p)
            }
            Abbrev::CurveY => {
                let [x1, y1, x3, y3] = self.nums::<4>(name)?;
                let (c1, p) = (self.to_page(x1, y1)?, self.to_page(x3, y3)?);
                self.curve_to(name, c1, p, p)
            }
            Abbrev::Close => self.close_path(),
            Abbrev::Paint { close, paint } => {
                if close {
                    self.close_path()?;
                }
                match paint {
                    Paint::Fill => self.paint(true),
                    Paint::Stroke => self.paint(false),
                    Paint::None => {
                        self.discard();
                        Ok(())
                    }
                }
            }
            Abbrev::Clip => self.run(Op::Clip, name),
            Abbrev::Drop(what) => {
                self.stack.clear();
                match what {
                    Dropping::Styling(kind) => self.drop_styling(kind),
                    Dropping::Text => {
                        // A text object's own paths (text on a path) are not outlines.
                        self.reset_path();
                        self.drop_element("text");
                    }
                    Dropping::Image => self.drop_element("image"),
                    Dropping::Structure => {}
                }
                Ok(())
            }
        }
    }

    // ── The result ──

    fn finish(self, bounding_box: Option<[f64; 4]>, placement: Placement) -> Result<EpsOutlines> {
        let Interp { painted, dropped, .. } = self;
        let frame = match bounding_box.or_else(|| bounds(&painted)) {
            Some(frame) if !painted.is_empty() => frame,
            _ => return Ok(EpsOutlines { paths: Vec::new(), bounding_box, dropped }),
        };
        let map = PageMap::new(placement, frame)?;
        let mut out = Collected::default();
        for p in &painted {
            build(p, &map, &mut out)?;
        }
        Ok(EpsOutlines { paths: out.paths, bounding_box, dropped })
    }
}

fn matrix_of(a: &[Value]) -> Option<Matrix> {
    let mut m = [0.0; 6];
    if a.len() != m.len() {
        return None;
    }
    for (slot, v) in m.iter_mut().zip(a) {
        match v {
            Value::Num(n) if usable(*n) => *slot = *n,
            _ => return None,
        }
    }
    Some(m)
}

/// The bounds of every point of the painted paths, control points included, as
/// `[llx, lly, urx, ury]`.
fn bounds(painted: &[Painted]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut add = |(x, y): Pt| {
        b = Some(match b {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    };
    for s in painted.iter().flat_map(|p| p.segs.iter()) {
        match *s {
            Seg::Move(p) | Seg::Line(p) => add(p),
            Seg::Curve(c1, c2, p) => {
                add(c1);
                add(c2);
                add(p);
            }
            Seg::Close => {}
        }
    }
    b
}

/// Page space to glyph coordinates: `x' = (x - x0) * scale`, `y' = top - (y1 - y) * scale`,
/// where `y1` is the frame's top edge.
struct PageMap {
    scale: f64,
    x0: f64,
    y1: f64,
    top: f64,
}

impl PageMap {
    fn new(placement: Placement, [llx, lly, _, ury]: [f64; 4]) -> Result<PageMap> {
        let fits = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
        match placement {
            Placement::Unscaled { top } => {
                if !fits(top) {
                    return Err(EpsError::BadPlacement(format!("the top {top} is not a number or is beyond ±{MAX_COORDINATE}")));
                }
                Ok(PageMap { scale: 1.0, x0: 0.0, y1: ury, top })
            }
            Placement::FitEm { units_per_em, ascender } => {
                if !(fits(units_per_em) && units_per_em > 0.0 && fits(ascender)) {
                    return Err(EpsError::BadPlacement(format!(
                        "units per em {units_per_em} and ascender {ascender} must be finite, with a positive em"
                    )));
                }
                let height = ury - lly;
                if !(height.is_finite() && height > 0.0) {
                    return Err(EpsError::BadPlacement("the drawing has no height to scale to the em".into()));
                }
                Ok(PageMap { scale: units_per_em / height, x0: llx, y1: ury, top: ascender })
            }
        }
    }

    fn apply(&self, (x, y): Pt) -> Result<Pt> {
        let fx = round((x - self.x0) * self.scale);
        let fy = round(self.top - (self.y1 - y) * self.scale);
        if fx.is_finite() && fy.is_finite() && fx.abs() <= MAX_COORDINATE && fy.abs() <= MAX_COORDINATE {
            Ok((fx, fy))
        } else {
            Err(EpsError::CoordinateOutOfRange)
        }
    }
}

/// Adds the contours of one painted path to `out`. A fill closes every subpath.
fn build(p: &Painted, map: &PageMap, out: &mut Collected) -> Result<()> {
    let end = |c: &mut Contour, out: &mut Collected| -> Result<()> {
        if p.fill && c.nodes.len() > 1 {
            c.close();
        }
        c.finish(out).map_err(EpsError::from)
    };
    let mut c = Contour::default();
    for s in &p.segs {
        match *s {
            Seg::Move(pt) => {
                end(&mut c, out)?;
                c.start(map.apply(pt)?);
            }
            Seg::Line(pt) => c.line(map.apply(pt)?),
            Seg::Curve(c1, c2, pt) => c.cubic(map.apply(c1)?, map.apply(c2)?, map.apply(pt)?),
            Seg::Close => {
                c.close();
                c.finish(out)?;
            }
        }
    }
    end(&mut c, out)
}

#[cfg(test)]
mod tests;
