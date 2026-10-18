//! Running a charstring with its subroutine calls followed, into a flat list of tokens.
//!
//! Only what the token boundaries need is tracked: the operand stack (for the subroutine number
//! of a call, the operand count of `blend` and the stem count), the stem count (for the length of
//! a hint mask) and the `vsindex` in force. A call's subroutine number disappears from the flat
//! output together with the call; everything else keeps its bytes.

use super::{CharstringSet, MAX_CALL_DEPTH, MAX_CHARSTRING, MAX_OPS_PER_GLYPH, SubrError, bias};

/// One token of a flat charstring: a number, an operator, or `hintmask`/`cntrmask` with its mask
/// bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Token {
    /// Where its bytes start in [`Flat::bytes`].
    pub start: u32,
    pub len: u32,
    /// Operands on the stack just before the token.
    pub depth: u16,
    /// An operator that leaves the stack empty.
    pub clears: bool,
    /// `CFF2` `blend`: the `vsindex` in force, which decides how many operands it takes.
    pub blend_vsindex: Option<u16>,
}

/// A charstring without subroutine calls, and its tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Flat {
    pub bytes: Vec<u8>,
    pub tokens: Vec<Token>,
}

impl Flat {
    /// The bytes of token `t`.
    pub fn token_bytes(&self, t: &Token) -> &[u8] {
        let start = t.start as usize;
        self.bytes.get(start..start + t.len as usize).unwrap_or_default()
    }
}

/// An operand on the stack: the output token that pushed it (none for a `blend` result) and its
/// value when it is an integer.
#[derive(Clone, Copy)]
struct Slot {
    token: Option<usize>,
    int: Option<i64>,
}

enum Flow {
    Go,
    Return,
    End,
}

struct Run<'a, 'b> {
    set: &'a CharstringSet,
    local: &'a [Vec<u8>],
    bytes: Vec<u8>,
    tokens: Vec<Token>,
    deleted: Vec<bool>,
    stack: Vec<Slot>,
    stems: usize,
    vsindex: usize,
    ops: usize,
    budget: &'b mut usize,
}

/// Glyph `gid` of `set`, its calls expanded. `budget` counts operators down across glyphs.
pub(super) fn flatten(set: &CharstringSet, gid: usize, budget: &mut usize) -> Result<Flat, SubrError> {
    let code = set.charstrings.get(gid).ok_or_else(|| SubrError::Invalid(format!("there is no glyph {gid}")))?;
    let fd = set.fd_of(gid);
    let mut run = Run {
        set,
        local: set.local_subrs.get(fd).map(Vec::as_slice).unwrap_or_default(),
        bytes: Vec::with_capacity(code.len()),
        tokens: Vec::new(),
        deleted: Vec::new(),
        stack: Vec::new(),
        stems: 0,
        vsindex: set.fd_vsindex.get(fd).map_or(0, |&v| usize::from(v)),
        ops: 0,
        budget,
    };
    run.run(code, 0)?;
    run.finish()
}

impl Run<'_, '_> {
    fn finish(self) -> Result<Flat, SubrError> {
        let mut flat = Flat { bytes: Vec::with_capacity(self.bytes.len()), tokens: Vec::with_capacity(self.tokens.len()) };
        for (t, gone) in self.tokens.iter().zip(&self.deleted) {
            if *gone {
                continue;
            }
            let start = t.start as usize;
            let src = self.bytes.get(start..start + t.len as usize).unwrap_or_default();
            let at = u32::try_from(flat.bytes.len()).map_err(|_| too_long())?;
            flat.bytes.extend_from_slice(src);
            flat.tokens.push(Token { start: at, ..*t });
        }
        if flat.bytes.len() > MAX_CHARSTRING {
            return Err(too_long());
        }
        Ok(flat)
    }

    fn tick(&mut self) -> Result<(), SubrError> {
        self.ops += 1;
        if self.ops > MAX_OPS_PER_GLYPH {
            return Err(SubrError::TooLarge { what: "operators run by one glyph", limit: MAX_OPS_PER_GLYPH });
        }
        *self.budget =
            self.budget.checked_sub(1).ok_or(SubrError::TooLarge { what: "operators run by all glyphs", limit: super::MAX_OPS_PER_SET })?;
        Ok(())
    }

    /// Appends a token and returns its index.
    fn emit(&mut self, bytes: &[u8], clears: bool, blend_vsindex: Option<u16>) -> Result<usize, SubrError> {
        // The flat output may hold subroutine numbers that are deleted later, hence the slack.
        if self.bytes.len() + bytes.len() > 2 * MAX_CHARSTRING {
            return Err(too_long());
        }
        let start = self.bytes.len() as u32;
        let depth = u16::try_from(self.stack.len()).map_err(|_| SubrError::Invalid("the operand stack overflows".into()))?;
        self.bytes.extend_from_slice(bytes);
        self.tokens.push(Token { start, len: bytes.len() as u32, depth, clears, blend_vsindex });
        self.deleted.push(false);
        if clears {
            self.stack.clear();
        }
        Ok(self.tokens.len() - 1)
    }

    fn pop(&mut self, what: &str) -> Result<Slot, SubrError> {
        self.stack.pop().ok_or_else(|| SubrError::Invalid(format!("{what} with an empty operand stack")))
    }

    fn run(&mut self, code: &[u8], level: usize) -> Result<Flow, SubrError> {
        let set: &CharstringSet = self.set;
        let cff2 = set.cff2;
        let mut i = 0usize;
        while let Some(&b0) = code.get(i) {
            self.tick()?;
            if b0 == 28 || b0 >= 32 {
                let (len, int) = number(code, i)?;
                if self.stack.len() >= set.max_stack {
                    return Err(SubrError::Invalid(format!("more than {} operands on the stack", set.max_stack)));
                }
                let bytes = code.get(i..i + len).unwrap_or_default();
                let token = self.emit(bytes, false, None)?;
                self.stack.push(Slot { token: Some(token), int });
                i += len;
                continue;
            }
            match b0 {
                10 | 29 => {
                    let slot = self.pop("a subroutine call")?;
                    let n = slot.int.ok_or_else(|| SubrError::Invalid("a subroutine number that is not an integer".into()))?;
                    let token = slot.token.ok_or_else(|| SubrError::Unsupported("a computed subroutine number".into()))?;
                    if let Some(d) = self.deleted.get_mut(token) {
                        *d = true;
                    }
                    let subrs: &[Vec<u8>] = if b0 == 10 { self.local } else { &set.global_subrs };
                    let index = n.checked_add(bias(subrs.len())).and_then(|v| usize::try_from(v).ok());
                    let what = if b0 == 10 { "local" } else { "global" };
                    let subr = index
                        .and_then(|k| subrs.get(k))
                        .ok_or_else(|| SubrError::Invalid(format!("a call to {what} subroutine {n}, which does not exist")))?;
                    if level + 1 > MAX_CALL_DEPTH {
                        return Err(SubrError::Invalid(format!("subroutines nest more than {MAX_CALL_DEPTH} deep")));
                    }
                    if let Flow::End = self.run(subr, level + 1)? {
                        return Ok(Flow::End);
                    }
                    i += 1;
                }
                11 if !cff2 => return Ok(Flow::Return),
                14 if !cff2 => {
                    self.emit(&[14], true, None)?;
                    return Ok(Flow::End);
                }
                1 | 3 | 18 | 23 => {
                    self.stems += self.stack.len() / 2;
                    self.emit(&[b0], true, None)?;
                    i += 1;
                }
                19 | 20 => {
                    // Operands before the first mask are vertical stems.
                    self.stems += self.stack.len() / 2;
                    let n = self.stems.div_ceil(8);
                    let bytes = code.get(i..i + 1 + n).ok_or_else(|| SubrError::Invalid("a hint mask is cut short".into()))?;
                    self.emit(bytes, true, None)?;
                    i += 1 + n;
                }
                4..=8 | 21 | 22 | 24..=27 | 30 | 31 => {
                    self.emit(&[b0], true, None)?;
                    i += 1;
                }
                15 if cff2 => {
                    let slot = *self.stack.last().ok_or_else(|| SubrError::Invalid("vsindex with an empty operand stack".into()))?;
                    let v = slot.int.and_then(|v| usize::try_from(v).ok()).filter(|&v| v < set.regions.len().max(1));
                    self.vsindex = v.ok_or_else(|| SubrError::Invalid("a vsindex that names no item variation data".into()))?;
                    self.emit(&[15], true, None)?;
                    i += 1;
                }
                16 if cff2 => {
                    let slot = *self.stack.last().ok_or_else(|| SubrError::Invalid("blend with an empty operand stack".into()))?;
                    let n = slot
                        .int
                        .and_then(|v| usize::try_from(v).ok())
                        .ok_or_else(|| SubrError::Invalid("a blend count that is not a whole number".into()))?;
                    let k = set.regions.get(self.vsindex).copied().unwrap_or(0);
                    let need = n.checked_mul(k + 1).and_then(|v| v.checked_add(1)).filter(|&v| v <= self.stack.len());
                    let need = need.ok_or_else(|| SubrError::Invalid("blend has too few operands".into()))?;
                    let vs = u16::try_from(self.vsindex).map_err(|_| SubrError::Invalid("a vsindex beyond 65,535".into()))?;
                    self.emit(&[16], false, Some(vs))?;
                    let keep = self.stack.len() - need;
                    self.stack.truncate(keep);
                    self.stack.extend(std::iter::repeat_n(Slot { token: None, int: None }, n));
                    i += 1;
                }
                12 => {
                    let b1 = *code.get(i + 1).ok_or_else(|| SubrError::Invalid("an escaped operator is cut short".into()))?;
                    match b1 {
                        0 if !cff2 => {}
                        34..=37 => {}
                        _ => return Err(SubrError::Unsupported(format!("charstring operator 12 {b1}"))),
                    }
                    self.emit(&[12, b1], true, None)?;
                    i += 2;
                }
                _ => return Err(SubrError::Invalid(format!("reserved charstring operator {b0}"))),
            }
        }
        Ok(Flow::Go)
    }
}

/// The length of the number at `i` and its value when it is an integer.
fn number(code: &[u8], i: usize) -> Result<(usize, Option<i64>), SubrError> {
    let cut = || SubrError::Invalid("a number is cut short".into());
    let b0 = i64::from(*code.get(i).ok_or_else(cut)?);
    let byte = |k: usize| code.get(i + k).map(|&b| i64::from(b)).ok_or_else(cut);
    Ok(match b0 {
        28 => (3, Some(i64::from(((byte(1)? << 8) | byte(2)?) as u16 as i16))),
        32..=246 => (1, Some(b0 - 139)),
        247..=250 => (2, Some((b0 - 247) * 256 + byte(1)? + 108)),
        251..=254 => (2, Some(-(b0 - 251) * 256 - byte(1)? - 108)),
        _ => {
            let v = ((byte(1)? << 24) | (byte(2)? << 16) | (byte(3)? << 8) | byte(4)?) as u32 as i32;
            (5, (v & 0xFFFF == 0).then_some(i64::from(v >> 16)))
        }
    })
}

fn too_long() -> SubrError {
    SubrError::TooLarge { what: "bytes in one expanded charstring", limit: MAX_CHARSTRING }
}
