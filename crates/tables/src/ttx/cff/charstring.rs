//! Type 2 charstrings as `ttx` writes them: decompiled into operators and operands by fontTools'
//! `SimpleT2Decompiler`, then written one operator a line (`T2CharString.toXML`). Ported from
//! fontTools' `misc.psCharStrings` (MIT).
//!
//! Decompiling walks a glyph's charstring and every subroutine it calls, with the operand stack
//! the subroutines share, so that hint masks get their length from the stems counted so far. A
//! subroutine keeps the program of its first call; a later call walks that program and must
//! find masks of the same length. The arithmetic operators are refused, as fontTools refuses
//! them.

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::limits::{MAX_CFF2_STACK, MAX_SUBR_NESTING};
use crate::{Result, TablesError};

use super::super::{XmlWriter, fixed_to_str, num2binary};
use super::varstore::VarStore;
use super::{Index, Num, bad};

/// Charstring tokens decompiled for one table, at most.
const MAX_STEPS: usize = 100_000_000;

/// A token of a decompiled charstring.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok {
    Num(Num),
    Op(&'static str),
    /// The bytes after `hintmask` or `cntrmask`.
    Mask(Vec<u8>),
}

pub(super) type Program = Vec<Tok>;

/// Global or local subroutines, with the programs of those decompiled so far.
pub(super) struct SubrSet<'a> {
    data: &'a [u8],
    index: Option<Index>,
    programs: BTreeMap<usize, Rc<Program>>,
}

impl<'a> SubrSet<'a> {
    pub(super) fn new(data: &'a [u8], index: Index) -> Self {
        SubrSet { data, index: Some(index), programs: BTreeMap::new() }
    }

    /// No subroutines: what a Private DICT without `Subrs` gives.
    pub(super) fn empty(data: &'a [u8]) -> Self {
        SubrSet { data, index: None, programs: BTreeMap::new() }
    }

    pub(super) fn count(&self) -> usize {
        self.index.map_or(0, |i| i.count())
    }

    pub(super) fn item(&self, i: usize) -> Result<&'a [u8]> {
        self.index.ok_or_else(|| bad("a subroutine is out of range"))?.item(self.data, i)
    }

    /// The program of subroutine `i`, when a charstring called it.
    pub(super) fn program(&self, i: usize) -> Option<&Program> {
        self.programs.get(&i).map(|p| p.as_ref())
    }

    /// The subroutine number bias of the CFF specification.
    fn bias(&self) -> i64 {
        match self.count() {
            n if n < 1240 => 107,
            n if n < 33900 => 1131,
            _ => 32768,
        }
    }
}

/// What `blend` and `vsindex` need: the variation store, and the Private DICT's `vsindex`
/// (`Err` when it is not an integer).
pub(super) struct Env<'v> {
    pub(super) vstore: Option<&'v VarStore>,
    pub(super) private_vsindex: Option<std::result::Result<i64, ()>>,
}

impl Env<'_> {
    fn regions(&self, vsindex: i64) -> Result<usize> {
        self.vstore.ok_or_else(|| bad("a charstring blends without a variation store"))?.regions(vsindex)
    }

    fn default_regions(&self) -> Result<usize> {
        match self.private_vsindex {
            None => self.regions(0),
            Some(Ok(v)) => self.regions(v),
            Some(Err(())) => Err(bad("vsindex is not an integer")),
        }
    }
}

/// The tokens one table may still decompile.
pub(super) struct Budget {
    left: usize,
}

impl Budget {
    pub(super) fn new() -> Self {
        Budget { left: MAX_STEPS }
    }

    fn spend(&mut self) -> Result<()> {
        self.left = self.left.checked_sub(1).ok_or(TablesError::TooLarge { what: "the charstrings of one table", limit: MAX_STEPS })?;
        Ok(())
    }
}

/// Decompiles a glyph's charstring, and the subroutines it calls into their sets.
pub(super) fn decompile<'a>(code: &'a [u8], locals: &mut SubrSet<'a>, globals: &mut SubrSet<'a>, env: &Env, budget: &mut Budget) -> Result<Program> {
    let mut d = Decompiler { env, budget, stack: Vec::new(), hint_count: 0, mask_bytes: 0, num_regions: 0, depth: 0 };
    d.execute(Source::Bytes(code), locals, globals)
}

enum Source<'c> {
    Bytes(&'c [u8]),
    Program(Rc<Program>),
}

struct Decompiler<'e, 'v> {
    env: &'e Env<'v>,
    budget: &'e mut Budget,
    stack: Vec<Num>,
    hint_count: usize,
    mask_bytes: usize,
    num_regions: usize,
    depth: usize,
}

impl Decompiler<'_, '_> {
    /// Runs `src`; the program it read when `src` is bytecode (empty for a program).
    fn execute<'a>(&mut self, src: Source<'a>, locals: &mut SubrSet<'a>, globals: &mut SubrSet<'a>) -> Result<Program> {
        // The glyph is level 1; each subroutine call one more.
        if self.depth > MAX_SUBR_NESTING {
            return Err(bad("subroutines nest too deep"));
        }
        self.depth += 1;
        let result = self.run(src, locals, globals);
        self.depth = self.depth.saturating_sub(1);
        result
    }

    fn run<'a>(&mut self, src: Source<'a>, locals: &mut SubrSet<'a>, globals: &mut SubrSet<'a>) -> Result<Program> {
        let from_bytes = matches!(src, Source::Bytes(_));
        let mut program = Vec::new();
        let mut i = 0usize;
        loop {
            let (tok, next) = match &src {
                Source::Bytes(b) => match next_token(b, i)? {
                    Some(t) => t,
                    None => break,
                },
                Source::Program(p) => match p.get(i) {
                    Some(t) => (t.clone(), i + 1),
                    None => break,
                },
            };
            i = next;
            self.budget.spend()?;
            if from_bytes {
                program.push(tok.clone());
            }
            match tok {
                Tok::Num(n) => self.push(n)?,
                Tok::Mask(_) => {}
                Tok::Op("hintmask" | "cntrmask") => {
                    if self.mask_bytes == 0 {
                        self.count_hints();
                        self.mask_bytes = self.hint_count.div_ceil(8);
                    }
                    let n = self.mask_bytes;
                    let mask = match &src {
                        Source::Bytes(b) => {
                            let m = b.get(i..i + n).ok_or_else(|| bad("a hint mask is cut short"))?.to_vec();
                            i += n;
                            m
                        }
                        Source::Program(p) => match p.get(i) {
                            Some(Tok::Mask(m)) if m.len() == n => {
                                i += 1;
                                m.clone()
                            }
                            _ => return Err(bad("a subroutine's hint mask has another length on a later call")),
                        },
                    };
                    if from_bytes {
                        program.push(Tok::Mask(mask));
                    }
                }
                Tok::Op("callsubr") => self.call(true, locals, globals)?,
                Tok::Op("callgsubr") => self.call(false, locals, globals)?,
                Tok::Op(op) => self.operator(op)?,
            }
        }
        Ok(program)
    }

    fn call<'a>(&mut self, local: bool, locals: &mut SubrSet<'a>, globals: &mut SubrSet<'a>) -> Result<()> {
        let n = self.pop_int()?;
        let set: &SubrSet<'a> = if local { &*locals } else { &*globals };
        let index = n
            .checked_add(set.bias())
            .and_then(|i| usize::try_from(i).ok())
            .filter(|&i| i < set.count())
            .ok_or_else(|| bad("a charstring calls a subroutine that does not exist"))?;
        let cached = set.programs.get(&index).cloned();
        if let Some(program) = cached {
            self.execute(Source::Program(program), locals, globals)?;
            return Ok(());
        }
        let code = set.item(index)?;
        let program = self.execute(Source::Bytes(code), locals, globals)?;
        let set = if local { locals } else { globals };
        set.programs.insert(index, Rc::new(program));
        Ok(())
    }

    fn operator(&mut self, op: &'static str) -> Result<()> {
        match op {
            "return" | "endchar" | "ignore" => {}
            "hstem" | "vstem" | "hstemhm" | "vstemhm" => self.count_hints(),
            "and" | "or" | "not" | "store" | "abs" | "add" | "sub" | "div" | "load" | "neg" | "eq" | "drop" | "put" | "get" | "ifelse" | "random"
            | "mul" | "sqrt" | "dup" | "exch" | "index" | "roll" => {
                return Err(TablesError::Unsupported(format!("the charstring operator '{op}'")));
            }
            "blend" => {
                if self.num_regions == 0 {
                    self.num_regions = self.env.default_regions()?;
                }
                let blends = self.pop_int()?;
                let regions = i64::try_from(self.num_regions).map_err(|_| bad("too many regions"))?;
                let deltas = blends.checked_mul(regions).ok_or_else(|| bad("a blend count is too large"))?;
                // Python's `del stack[-deltas:]`: the deltas go, the defaults stay.
                let len = self.stack.len() as i64;
                let start = match deltas.checked_neg() {
                    Some(s) if s < 0 => (len + s).max(0),
                    Some(s) => s.min(len),
                    None => 0,
                };
                self.stack.truncate(usize::try_from(start).unwrap_or(0));
            }
            "vsindex" => {
                let vsindex = self.pop_int()?;
                self.num_regions = self.env.regions(vsindex)?;
            }
            _ => self.stack.clear(),
        }
        Ok(())
    }

    fn push(&mut self, n: Num) -> Result<()> {
        if self.stack.len() >= MAX_CFF2_STACK {
            return Err(bad("a charstring's operand stack overflows"));
        }
        self.stack.push(n);
        Ok(())
    }

    fn pop_int(&mut self) -> Result<i64> {
        match self.stack.pop() {
            Some(Num::Int(v)) => Ok(v),
            Some(Num::Real(_)) => Err(bad("a charstring operator needs an integer")),
            None => Err(bad("a charstring operator has too few operands")),
        }
    }

    fn count_hints(&mut self) {
        self.hint_count += self.stack.len() / 2;
        self.stack.clear();
    }
}

/// The token at `i` and the position after it; `None` at the end, and at an operator the
/// format does not define (fontTools stops there).
fn next_token(b: &[u8], i: usize) -> Result<Option<(Tok, usize)>> {
    let Some(&b0) = b.get(i) else { return Ok(None) };
    let i = i + 1;
    let byte = |at: usize| b.get(at).copied().map(i64::from).ok_or_else(|| bad("a charstring is cut short"));
    let int = |v: i64| Tok::Num(Num::Int(v));
    let token = match b0 {
        28 => {
            let v = (byte(i)? << 8 | byte(i + 1)?) as u16 as i16;
            (int(i64::from(v)), i + 2)
        }
        32..=246 => (int(i64::from(b0) - 139), i),
        247..=250 => (int((i64::from(b0) - 247) * 256 + byte(i)? + 108), i + 1),
        251..=254 => (int(-(i64::from(b0) - 251) * 256 - byte(i)? - 108), i + 1),
        255 => {
            let v = b.get(i..i + 4).ok_or_else(|| bad("a charstring is cut short"))?;
            let v = v.iter().fold(0u32, |acc, x| acc << 8 | u32::from(*x)) as i32;
            (Tok::Num(Num::Real(f64::from(v) / 65536.0)), i + 4)
        }
        12 => {
            let b1 = byte(i)?;
            match escaped_operator(b1) {
                Some(op) => (Tok::Op(op), i + 1),
                None => return Ok(None),
            }
        }
        _ => match operator(b0) {
            Some(op) => (Tok::Op(op), i),
            None => return Ok(None),
        },
    };
    Ok(Some(token))
}

fn operator(b0: u8) -> Option<&'static str> {
    Some(match b0 {
        1 => "hstem",
        3 => "vstem",
        4 => "vmoveto",
        5 => "rlineto",
        6 => "hlineto",
        7 => "vlineto",
        8 => "rrcurveto",
        10 => "callsubr",
        11 => "return",
        14 => "endchar",
        15 => "vsindex",
        16 => "blend",
        18 => "hstemhm",
        19 => "hintmask",
        20 => "cntrmask",
        21 => "rmoveto",
        22 => "hmoveto",
        23 => "vstemhm",
        24 => "rcurveline",
        25 => "rlinecurve",
        26 => "vvcurveto",
        27 => "hhcurveto",
        29 => "callgsubr",
        30 => "vhcurveto",
        31 => "hvcurveto",
        _ => return None,
    })
}

fn escaped_operator(b1: i64) -> Option<&'static str> {
    Some(match b1 {
        0 => "ignore",
        3 => "and",
        4 => "or",
        5 => "not",
        8 => "store",
        9 => "abs",
        10 => "add",
        11 => "sub",
        12 => "div",
        13 => "load",
        14 => "neg",
        15 => "eq",
        18 => "drop",
        20 => "put",
        21 => "get",
        22 => "ifelse",
        23 => "random",
        24 => "mul",
        26 => "sqrt",
        27 => "dup",
        28 => "exch",
        29 => "index",
        30 => "roll",
        34 => "hflex",
        35 => "flex",
        36 => "hflex1",
        37 => "flex1",
        _ => return None,
    })
}

/// Writes a program one operator a line, its operands before it and a hint mask after it as
/// bits. Returns the operands left after the last operator, which go on the closing line.
pub(super) fn write_program(w: &mut XmlWriter, program: &Program) -> Option<String> {
    let mut args: Vec<String> = Vec::new();
    let mut i = 0usize;
    while let Some(tok) = program.get(i) {
        i += 1;
        match tok {
            Tok::Num(Num::Int(v)) => args.push(v.to_string()),
            Tok::Num(Num::Real(v)) => args.push(fixed_to_str(*v, 16)),
            Tok::Mask(_) => {}
            Tok::Op(op) => {
                args.push((*op).to_string());
                if matches!(*op, "hintmask" | "cntrmask") {
                    let mut bits = String::new();
                    if let Some(Tok::Mask(mask)) = program.get(i) {
                        i += 1;
                        for b in mask {
                            bits.push_str(&num2binary(u64::from(*b), 8));
                        }
                    }
                    args.push(bits);
                }
                w.text(&args.join(" "));
                args.clear();
            }
        }
    }
    (!args.is_empty()).then(|| args.join(" "))
}
