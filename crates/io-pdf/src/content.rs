//! Running a page's content stream: the path, painting and graphics state operators, and form
//! XObjects. Everything else is skipped, and what is not geometry is counted in [`Dropped`].

use std::collections::HashMap;
use std::rc::Rc;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::load::{Budget, deref, dict, number};
use crate::{Dropped, MAX_PDF_DEPTH, MAX_PDF_NUMBER, MAX_PDF_OPERANDS, MAX_PDF_OPERATORS, MAX_PDF_SCANNED_BYTES, MAX_PDF_SEGMENTS, PdfError};

pub(crate) type Pt = (f64, f64);
/// `[a b c d e f]`: a point `(x, y)` maps to `(a x + c y + e, b x + d y + f)`.
pub(crate) type Matrix = [f64; 6];
pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Seg {
    Move(Pt),
    Line(Pt),
    Curve(Pt, Pt, Pt),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FillRule {
    NonZero,
    EvenOdd,
}

/// One painted path, in the page's shown space.
#[derive(Clone, Debug)]
pub(crate) struct Painted {
    pub(crate) segs: Vec<Seg>,
    pub(crate) fill: Option<FillRule>,
    /// The line width in the shown space when the path is stroked.
    pub(crate) stroke: Option<f64>,
    pub(crate) fill_rgb: [f64; 3],
    pub(crate) stroke_rgb: [f64; 3],
}

/// The first page, ready to be placed or drawn.
pub(crate) struct PageDrawing {
    pub(crate) painted: Vec<Painted>,
    /// The page's box in the shown space, when it has one.
    pub(crate) frame: Option<[f64; 4]>,
    pub(crate) page_box: Option<[f64; 4]>,
    pub(crate) dropped: Dropped,
}

pub(crate) fn concat(m: &Matrix, n: &Matrix) -> Matrix {
    let [a, b, c, d, e, f] = *m;
    let [na, nb, nc, nd, ne, nf] = *n;
    [a * na + b * nc, a * nb + b * nd, c * na + d * nc, c * nb + d * nd, e * na + f * nc + ne, e * nb + f * nd + nf]
}

pub(crate) fn apply(m: &Matrix, (x, y): Pt) -> Pt {
    let [a, b, c, d, e, f] = *m;
    (a * x + c * y + e, b * x + d * y + f)
}

fn usable(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_PDF_NUMBER
}

// ───────────────────────────── Tokens ─────────────────────────────

enum Token<'a> {
    Num(f64),
    Name(Vec<u8>),
    /// Strings, arrays and dictionary brackets, booleans and null: operands this reader never
    /// uses.
    Other,
    Op(&'a [u8]),
}

fn is_space(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_delimiter(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn is_regular(b: u8) -> bool {
    !is_space(b) && !is_delimiter(b)
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn regular_run(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(is_regular) {
            self.pos += 1;
        }
        self.src.get(start..self.pos).unwrap_or_default()
    }

    fn next(&mut self) -> Result<Option<Token<'a>>, PdfError> {
        loop {
            let Some(b) = self.peek() else { return Ok(None) };
            if is_space(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.peek().is_some_and(|c| c != b'\n' && c != b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        let Some(b) = self.peek() else { return Ok(None) };
        match b {
            b'/' => {
                self.pos += 1;
                Ok(Some(Token::Name(decode_name(self.regular_run()))))
            }
            b'(' => {
                self.skip_literal_string();
                Ok(Some(Token::Other))
            }
            b'<' if self.src.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Ok(Some(Token::Other))
            }
            b'>' if self.src.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Ok(Some(Token::Other))
            }
            b'<' => {
                while self.peek().is_some_and(|c| c != b'>') {
                    self.pos += 1;
                }
                self.pos += 1;
                Ok(Some(Token::Other))
            }
            b'[' | b']' | b'{' | b'}' | b')' | b'>' => {
                self.pos += 1;
                Ok(Some(Token::Other))
            }
            _ => {
                let word = self.regular_run();
                match word.first() {
                    Some(c) if c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.') => {
                        let v = std::str::from_utf8(word).ok().and_then(|s| s.parse::<f64>().ok());
                        match v {
                            Some(v) if usable(v) => Ok(Some(Token::Num(v))),
                            Some(_) => Err(PdfError::NumberOutOfRange),
                            None => Ok(Some(Token::Other)),
                        }
                    }
                    _ if matches!(word, b"true" | b"false" | b"null") => Ok(Some(Token::Other)),
                    _ => Ok(Some(Token::Op(word))),
                }
            }
        }
    }

    fn skip_literal_string(&mut self) {
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'\\' => self.pos += 1,
                b'(' => depth += 1,
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Skips an inline image after `BI`: its dictionary up to `ID`, then its data up to an `EI`
    /// standing alone between white space (or the end).
    fn skip_inline_image(&mut self) -> Result<(), PdfError> {
        while let Some(t) = self.next()? {
            if let Token::Op(b"ID") = t {
                break;
            }
        }
        self.pos += 1;
        while self.pos < self.src.len() {
            let here = self.pos;
            let ei = self.src.get(here..here + 2) == Some(b"EI".as_slice());
            let before = here > 0 && self.src.get(here - 1).copied().is_some_and(is_space);
            let after = self.src.get(here + 2).is_none_or(|c| is_space(*c) || is_delimiter(*c));
            if ei && before && after {
                self.pos = here + 2;
                return Ok(());
            }
            self.pos += 1;
        }
        Ok(())
    }
}

/// A name with its `#xx` escapes decoded.
fn decode_name(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while let Some(&c) = raw.get(i) {
        let hex = |j: usize| raw.get(j).and_then(|h| (*h as char).to_digit(16));
        if c == b'#'
            && let (Some(hi), Some(lo)) = (hex(i + 1), hex(i + 2))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

// ───────────────────────────── Running ─────────────────────────────

#[derive(Clone)]
struct GState {
    ctm: Matrix,
    line_width: f64,
    fill_rgb: [f64; 3],
    stroke_rgb: [f64; 3],
}

pub(crate) struct Interp<'d> {
    doc: &'d Document,
    budget: Budget,
    gs: GState,
    saved: Vec<GState>,
    path: Vec<Seg>,
    current: Option<Pt>,
    start: Option<Pt>,
    /// The path ends with `W` or `W*` pending: it clips (which is left out) as well as paints.
    clipping: bool,
    painted: Vec<Painted>,
    segments: usize,
    operators: usize,
    scanned: usize,
    operands: Vec<Token<'static>>,
    dropped: Dropped,
    forms: HashMap<ObjectId, Rc<Vec<u8>>>,
}

fn operand_number(t: &Token<'_>) -> Option<f64> {
    match t {
        Token::Num(v) => Some(*v),
        _ => None,
    }
}

impl<'d> Interp<'d> {
    pub(crate) fn new(doc: &'d Document, budget: Budget, ctm: Matrix) -> Self {
        Interp {
            doc,
            budget,
            gs: GState { ctm, line_width: 1.0, fill_rgb: [0.0; 3], stroke_rgb: [0.0; 3] },
            saved: Vec::new(),
            path: Vec::new(),
            current: None,
            start: None,
            clipping: false,
            painted: Vec::new(),
            segments: 0,
            operators: 0,
            scanned: 0,
            operands: Vec::new(),
            dropped: Dropped::default(),
            forms: HashMap::new(),
        }
    }

    pub(crate) fn decode(&mut self, stream: &Stream) -> Result<Vec<u8>, PdfError> {
        self.budget.decode(stream)
    }

    pub(crate) fn finish(self) -> (Vec<Painted>, Dropped) {
        (self.painted, self.dropped)
    }

    pub(crate) fn run_page(&mut self, content: &[u8], resources: Option<&'d Dictionary>) -> Result<(), PdfError> {
        self.run(content, resources)
    }

    fn run(&mut self, content: &[u8], resources: Option<&'d Dictionary>) -> Result<(), PdfError> {
        self.scanned = self.scanned.saturating_add(content.len());
        if self.scanned > MAX_PDF_SCANNED_BYTES {
            return Err(PdfError::TooMuchWork);
        }
        let mut lexer = Lexer { src: content, pos: 0 };
        while let Some(token) = lexer.next()? {
            match token {
                Token::Op(op) => {
                    self.operators = self.operators.saturating_add(1);
                    if self.operators > MAX_PDF_OPERATORS {
                        return Err(PdfError::TooMuchWork);
                    }
                    if op == b"BI" {
                        lexer.skip_inline_image()?;
                        self.dropped.element("image");
                    } else {
                        self.operator(op, resources)?;
                    }
                    self.operands.clear();
                }
                Token::Num(v) => self.push(Token::Num(v))?,
                Token::Name(n) => self.push(Token::Name(n))?,
                Token::Other => self.push(Token::Other)?,
            }
        }
        Ok(())
    }

    fn push(&mut self, t: Token<'static>) -> Result<(), PdfError> {
        if self.operands.len() >= MAX_PDF_OPERANDS {
            return Err(PdfError::TooMuchData);
        }
        self.operands.push(t);
        Ok(())
    }

    /// The last `N` operands as numbers, or `None` when there are fewer or they are not numbers.
    fn nums<const N: usize>(&self) -> Option<[f64; N]> {
        let first = self.operands.len().checked_sub(N)?;
        let mut out = [0.0; N];
        for (slot, t) in out.iter_mut().zip(self.operands.get(first..)?) {
            *slot = operand_number(t)?;
        }
        Some(out)
    }

    /// Every trailing numeric operand (for `sc` and `scn`, whose count depends on the colour
    /// space).
    fn trailing_nums(&self) -> Vec<f64> {
        let mut out: Vec<f64> = self.operands.iter().rev().map_while(operand_number).collect();
        out.reverse();
        out
    }

    fn last_name(&self) -> Option<&[u8]> {
        match self.operands.last() {
            Some(Token::Name(n)) => Some(n),
            _ => None,
        }
    }

    fn point(&self, x: f64, y: f64) -> Result<Pt, PdfError> {
        let p = apply(&self.gs.ctm, (x, y));
        if usable(p.0) && usable(p.1) { Ok(p) } else { Err(PdfError::NumberOutOfRange) }
    }

    fn add(&mut self, seg: Seg) -> Result<(), PdfError> {
        self.segments = self.segments.saturating_add(1);
        if self.segments > MAX_PDF_SEGMENTS {
            return Err(PdfError::TooMuchData);
        }
        self.path.push(seg);
        Ok(())
    }

    fn operator(&mut self, op: &[u8], resources: Option<&'d Dictionary>) -> Result<(), PdfError> {
        match op {
            // ── Graphics state ──
            b"q" => {
                if self.saved.len() >= MAX_PDF_DEPTH {
                    return Err(PdfError::TooDeep);
                }
                self.saved.push(self.gs.clone());
            }
            b"Q" => {
                if let Some(gs) = self.saved.pop() {
                    self.gs = gs;
                }
            }
            b"cm" => {
                if let Some(m) = self.nums::<6>() {
                    self.set_ctm(concat(&m, &self.gs.ctm))?;
                }
            }
            b"w" => {
                if let Some([w]) = self.nums::<1>() {
                    self.gs.line_width = w.abs();
                }
            }
            b"gs" => {
                self.dropped.attribute("graphics state");
                let width = self.last_name().and_then(|name| {
                    let states = dict(self.doc, resources?.get(b"ExtGState").ok()?)?;
                    let state = dict(self.doc, states.get(name).ok()?)?;
                    number(self.doc, state.get(b"LW").ok()?)
                });
                if let Some(w) = width {
                    self.gs.line_width = w.abs();
                }
            }
            b"d" | b"j" | b"J" | b"M" => self.dropped.attribute("line style"),
            // ── Colour: kept for drawing, left out of outlines ──
            b"g" | b"rg" | b"k" | b"sc" | b"scn" => {
                self.dropped.attribute("colour");
                if let Some(rgb) = colour(&self.trailing_nums()) {
                    self.gs.fill_rgb = rgb;
                }
            }
            b"G" | b"RG" | b"K" | b"SC" | b"SCN" => {
                self.dropped.attribute("colour");
                if let Some(rgb) = colour(&self.trailing_nums()) {
                    self.gs.stroke_rgb = rgb;
                }
            }
            b"cs" | b"CS" => self.dropped.attribute("colour"),
            // ── Path construction ──
            b"m" => {
                if let Some([x, y]) = self.nums::<2>() {
                    let p = self.point(x, y)?;
                    self.add(Seg::Move(p))?;
                    self.current = Some(p);
                    self.start = Some(p);
                }
            }
            b"l" => {
                if let (Some([x, y]), Some(_)) = (self.nums::<2>(), self.current) {
                    let p = self.point(x, y)?;
                    self.add(Seg::Line(p))?;
                    self.current = Some(p);
                }
            }
            b"c" => {
                if let (Some([x1, y1, x2, y2, x, y]), Some(_)) = (self.nums::<6>(), self.current) {
                    let (c1, c2, p) = (self.point(x1, y1)?, self.point(x2, y2)?, self.point(x, y)?);
                    self.add(Seg::Curve(c1, c2, p))?;
                    self.current = Some(p);
                }
            }
            b"v" => {
                if let (Some([x2, y2, x, y]), Some(c1)) = (self.nums::<4>(), self.current) {
                    let (c2, p) = (self.point(x2, y2)?, self.point(x, y)?);
                    self.add(Seg::Curve(c1, c2, p))?;
                    self.current = Some(p);
                }
            }
            b"y" => {
                if let (Some([x1, y1, x, y]), Some(_)) = (self.nums::<4>(), self.current) {
                    let (c1, p) = (self.point(x1, y1)?, self.point(x, y)?);
                    self.add(Seg::Curve(c1, p, p))?;
                    self.current = Some(p);
                }
            }
            b"h" => self.close()?,
            b"re" => {
                if let Some([x, y, w, h]) = self.nums::<4>() {
                    let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
                    let mut pts = [(0.0, 0.0); 4];
                    for (slot, (cx, cy)) in pts.iter_mut().zip(corners) {
                        *slot = self.point(cx, cy)?;
                    }
                    let [p0, p1, p2, p3] = pts;
                    for seg in [Seg::Move(p0), Seg::Line(p1), Seg::Line(p2), Seg::Line(p3), Seg::Close] {
                        self.add(seg)?;
                    }
                    self.current = Some(p0);
                    self.start = Some(p0);
                }
            }
            // ── Painting ──
            b"f" | b"F" => self.paint(Some(FillRule::NonZero), false),
            b"f*" => self.paint(Some(FillRule::EvenOdd), false),
            b"S" => self.paint(None, true),
            b"s" => {
                self.close()?;
                self.paint(None, true);
            }
            b"B" => self.paint(Some(FillRule::NonZero), true),
            b"B*" => self.paint(Some(FillRule::EvenOdd), true),
            b"b" => {
                self.close()?;
                self.paint(Some(FillRule::NonZero), true);
            }
            b"b*" => {
                self.close()?;
                self.paint(Some(FillRule::EvenOdd), true);
            }
            b"n" => {
                if self.path.iter().any(|s| !matches!(s, Seg::Move(_) | Seg::Close)) && !self.clipping {
                    self.dropped.element("unpainted path");
                }
                self.end_path();
            }
            b"W" | b"W*" => {
                self.clipping = true;
                self.dropped.element("clipping path");
            }
            // ── Other content ──
            b"BT" => self.dropped.element("text"),
            b"sh" => self.dropped.element("shading"),
            b"Do" => {
                if let Some(name) = self.last_name().map(<[u8]>::to_vec) {
                    self.xobject(&name, resources)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn set_ctm(&mut self, m: Matrix) -> Result<(), PdfError> {
        if m.iter().all(|v| usable(*v)) {
            self.gs.ctm = m;
            Ok(())
        } else {
            Err(PdfError::NumberOutOfRange)
        }
    }

    fn close(&mut self) -> Result<(), PdfError> {
        if self.current.is_some() {
            self.add(Seg::Close)?;
            self.current = self.start;
        }
        Ok(())
    }

    fn end_path(&mut self) {
        self.path.clear();
        self.current = None;
        self.start = None;
        self.clipping = false;
    }

    fn paint(&mut self, fill: Option<FillRule>, stroke: bool) {
        let segs = std::mem::take(&mut self.path);
        self.end_path();
        if !segs.iter().any(|s| !matches!(s, Seg::Move(_) | Seg::Close)) {
            return;
        }
        let [a, b, c, d, ..] = self.gs.ctm;
        let scale = (a * d - b * c).abs().sqrt();
        let width = self.gs.line_width * scale;
        self.painted.push(Painted {
            segs,
            fill,
            stroke: (stroke && width.is_finite()).then_some(width),
            fill_rgb: self.gs.fill_rgb,
            stroke_rgb: self.gs.stroke_rgb,
        });
    }

    fn xobject(&mut self, name: &[u8], resources: Option<&'d Dictionary>) -> Result<(), PdfError> {
        let doc = self.doc;
        let Some(entry) = resources.and_then(|r| r.get(b"XObject").ok()).and_then(|x| dict(doc, x)).and_then(|x| x.get(name).ok()) else {
            return Ok(());
        };
        let id = entry.as_reference().ok();
        let Object::Stream(stream) = deref(doc, entry)? else { return Ok(()) };
        let subtype = stream.dict.get(b"Subtype").ok().and_then(|s| s.as_name().ok());
        match subtype {
            Some(b"Form") => {}
            Some(b"Image") => {
                self.dropped.element("image");
                return Ok(());
            }
            _ => return Ok(()),
        }
        if self.saved.len() >= MAX_PDF_DEPTH {
            return Err(PdfError::TooDeep);
        }
        let content = match id.and_then(|id| self.forms.get(&id).cloned()) {
            Some(cached) => cached,
            None => {
                let decoded = Rc::new(self.budget.decode(stream)?);
                if let Some(id) = id {
                    self.forms.insert(id, Rc::clone(&decoded));
                }
                decoded
            }
        };
        let matrix = match stream.dict.get(b"Matrix").ok().map(|m| deref(doc, m)).transpose()? {
            Some(Object::Array(items)) if items.len() == 6 => {
                let mut m = IDENTITY;
                for (slot, item) in m.iter_mut().zip(items) {
                    *slot = number(doc, item).ok_or(PdfError::NumberOutOfRange)?;
                }
                m
            }
            _ => IDENTITY,
        };
        let form_resources = stream.dict.get(b"Resources").ok().and_then(|r| dict(doc, r)).or(resources);
        let outer_path = (std::mem::take(&mut self.path), self.current.take(), self.start.take(), std::mem::take(&mut self.clipping));
        self.saved.push(self.gs.clone());
        let ran = self.set_ctm(concat(&matrix, &self.gs.ctm)).and_then(|()| self.run(&content, form_resources));
        if let Some(gs) = self.saved.pop() {
            self.gs = gs;
        }
        (self.path, self.current, self.start, self.clipping) = outer_path;
        ran
    }
}

/// A colour from its components: grey, RGB or CMYK by their number, each clamped to 0–1.
fn colour(c: &[f64]) -> Option<[f64; 3]> {
    let k = |v: f64| v.clamp(0.0, 1.0);
    match *c {
        [g] => Some([k(g); 3]),
        [r, g, b] => Some([k(r), k(g), k(b)]),
        [c, m, y, black] => {
            let w = 1.0 - k(black);
            Some([(1.0 - k(c)) * w, (1.0 - k(m)) * w, (1.0 - k(y)) * w])
        }
        _ => None,
    }
}
