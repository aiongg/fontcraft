//! Static analysis of a program: the stack depth it reaches, the functions, storage cells and
//! twilight points it uses (crate docs, [`crate::analyse`]).
//!
//! The walk is linear in the program: it keeps an abstract stack of values that are known where
//! they were pushed as constants, follows both branches of every `IF` and merges them at `EIF`,
//! and accounts for a `CALL` through the callee's [`FunctionInfo`] instead of walking its body
//! again. Function bodies start with the glyph zone in every zone pointer and a loop count of 1.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::table::{self, EIF, ELSE, ENDF, FDEF, IDEF, IF};
use crate::{AsmError, FunctionInfo, FunctionTable, Instruction, MAX_CALL_DEPTH, MAX_FUNCTIONS, ProgramLimits};

/// Known values kept on the abstract stack; deeper ones are counted but forgotten.
const TRACK_LIMIT: usize = 1024;
/// Values copied when saving the state at `IF`, for the whole analysis; beyond it the values are
/// forgotten before each copy (and the result marked uncertain), so hostile input stays linear.
const COPY_BUDGET: usize = 1 << 24;

fn to_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

#[derive(Clone, Debug)]
struct State {
    /// Values above `below`, top last; `None` where the value is not known.
    vals: Vec<Option<i32>>,
    /// Stack entries beneath `vals`, relative to the depth on entry (negative after popping
    /// values the caller pushed).
    below: i64,
    /// Lowest depth reached relative to entry, by pops and moves.
    low: i64,
    low_known: bool,
    /// Whether the depth relative to entry is still known.
    effect_known: bool,
    /// Zone pointers zp0, zp1, zp2: 0 twilight, 1 glyph, `None` unknown.
    zp: [Option<u8>; 3],
    loop_n: Option<i64>,
}

impl State {
    fn new() -> Self {
        State { vals: Vec::new(), below: 0, low: 0, low_known: true, effect_known: true, zp: [Some(1); 3], loop_n: Some(1) }
    }

    fn depth(&self) -> i64 {
        self.below.saturating_add(to_i64(self.vals.len()))
    }

    fn pop(&mut self) -> Option<i32> {
        match self.vals.pop() {
            Some(v) => v,
            None => {
                self.below = self.below.saturating_sub(1);
                self.low = self.low.min(self.below);
                None
            }
        }
    }

    fn pop_n(&mut self, n: i64) {
        let n = n.max(0);
        let tracked = usize::try_from(n).unwrap_or(usize::MAX).min(self.vals.len());
        self.vals.truncate(self.vals.len().saturating_sub(tracked));
        let rest = n.saturating_sub(to_i64(tracked));
        if rest > 0 {
            self.below = self.below.saturating_sub(rest);
            self.low = self.low.min(self.below);
        }
    }

    fn push(&mut self, v: Option<i32>) {
        self.vals.push(v);
        self.trim();
    }

    fn push_unknown(&mut self, n: i64) {
        if n <= 0 {
            return;
        }
        match usize::try_from(n) {
            Ok(k) if k < TRACK_LIMIT => {
                self.vals.resize(self.vals.len() + k, None);
                self.trim();
            }
            _ => {
                self.forget_values();
                self.below = self.below.saturating_add(n);
            }
        }
    }

    /// Keeps at most [`TRACK_LIMIT`] values, forgetting the deepest.
    fn trim(&mut self) {
        if self.vals.len() > TRACK_LIMIT {
            let k = self.vals.len() - TRACK_LIMIT / 2;
            self.vals.drain(..k);
            self.below = self.below.saturating_add(to_i64(k));
        }
    }

    /// Changes the depth by `n` (pushing unknown values or popping).
    fn shift(&mut self, n: i64) {
        if n < 0 { self.pop_n(n.saturating_neg()) } else { self.push_unknown(n) }
    }

    fn invalidate(&mut self) {
        self.vals.iter_mut().for_each(|v| *v = None);
    }

    fn forget_values(&mut self) {
        self.below = self.below.saturating_add(to_i64(self.vals.len()));
        self.vals.clear();
    }

    /// The stack after something the analysis cannot follow.
    fn lose_track(&mut self) {
        self.invalidate();
        self.effect_known = false;
        self.low_known = false;
    }
}

/// The state after one of two branches; the deeper one is kept for the depth.
fn merge(a: State, b: State, uncertain: &mut bool) -> State {
    let (mut keep, other) = if a.depth() >= b.depth() { (a, b) } else { (b, a) };
    if keep.depth() == other.depth() {
        let lk = keep.vals.len();
        let lo = other.vals.len();
        for (idx, v) in keep.vals.iter_mut().enumerate() {
            let from_top = lk - idx;
            let theirs = lo.checked_sub(from_top).and_then(|i| other.vals.get(i)).copied().flatten();
            if theirs != *v {
                *v = None;
            }
        }
    } else {
        *uncertain = true;
        keep.invalidate();
        keep.effect_known = false;
    }
    keep.low = keep.low.min(other.low);
    keep.low_known &= other.low_known;
    keep.effect_known &= other.effect_known;
    for (z, o) in keep.zp.iter_mut().zip(other.zp) {
        if *z != o {
            *z = None;
        }
    }
    if keep.loop_n != other.loop_n {
        keep.loop_n = None;
    }
    keep
}

/// What one walk over a range of instructions found.
struct Walk {
    /// Deepest stack relative to entry, callees included.
    max: i64,
    end: State,
    uncertain: bool,
}

#[derive(Default)]
struct Resources {
    /// Storage cells used (highest index + 1).
    storage: i64,
    /// Twilight points used (highest index + 1).
    twilight: i64,
    /// Highest function number defined or called.
    function: Option<i64>,
    idefs: BTreeSet<i32>,
    unknown_idefs: usize,
    twilight_zone: bool,
}

pub(crate) struct Analyser<'a> {
    ins: &'a [Instruction],
    external: Option<&'a FunctionTable>,
    /// Functions this program defines: number → (`FDEF` offset, body range).
    bodies: BTreeMap<u32, (usize, Range<usize>)>,
    memo: BTreeMap<u32, FunctionInfo>,
    visiting: BTreeSet<u32>,
    res: Resources,
    copied: usize,
    /// Some function body could not be followed in full.
    bodies_uncertain: bool,
}

impl<'a> Analyser<'a> {
    pub(crate) fn new(ins: &'a [Instruction], external: Option<&'a FunctionTable>) -> Self {
        Analyser {
            ins,
            external,
            bodies: BTreeMap::new(),
            memo: BTreeMap::new(),
            visiting: BTreeSet::new(),
            res: Resources::default(),
            copied: 0,
            bodies_uncertain: false,
        }
    }

    /// The functions defined by the program walked so far, analysed.
    pub(crate) fn functions(mut self) -> Result<FunctionTable, AsmError> {
        self.cover_bodies()?;
        Ok(FunctionTable { functions: self.memo })
    }

    /// Walks the whole program at top level, then every function it defines.
    pub(crate) fn limits(&mut self, program_len: usize) -> Result<ProgramLimits, AsmError> {
        let walk = self.run(0..self.ins.len(), false, 0)?;
        self.cover_bodies()?;
        let mut uncertain = walk.uncertain || self.bodies_uncertain || walk.end.low < 0;
        let mut clamp = |v: i64| -> u16 {
            if v > i64::from(u16::MAX) {
                uncertain = true;
            }
            u16::try_from(v.max(0)).unwrap_or(u16::MAX)
        };
        let max_stack_elements = clamp(walk.max);
        let max_function_defs = clamp(self.res.function.map_or(0, |f| f.saturating_add(1)));
        let max_instruction_defs = clamp(to_i64(self.res.idefs.len().saturating_add(self.res.unknown_idefs)));
        let max_storage = clamp(self.res.storage);
        let max_twilight_points = clamp(self.res.twilight);
        let max_size_of_instructions = clamp(to_i64(program_len));
        Ok(ProgramLimits {
            max_stack_elements,
            max_function_defs,
            max_instruction_defs,
            max_storage,
            max_twilight_points,
            max_zones: if self.res.twilight_zone { 2 } else { 1 },
            max_size_of_instructions,
            uncertain,
        })
    }

    pub(crate) fn define_all(&mut self) -> Result<(), AsmError> {
        let walk = self.run(0..self.ins.len(), false, 0)?;
        self.bodies_uncertain |= walk.uncertain;
        Ok(())
    }

    fn cover_bodies(&mut self) -> Result<(), AsmError> {
        let numbers: Vec<u32> = self.bodies.keys().copied().collect();
        for n in numbers {
            if let Some(info) = self.callee(n, 0)? {
                self.bodies_uncertain |= info.uncertain;
            }
        }
        Ok(())
    }

    fn callee(&mut self, n: u32, depth: usize) -> Result<Option<FunctionInfo>, AsmError> {
        if let Some(info) = self.memo.get(&n) {
            return Ok(Some(*info));
        }
        if let Some((offset, body)) = self.bodies.get(&n).cloned() {
            if depth >= MAX_CALL_DEPTH || self.visiting.contains(&n) {
                return Ok(None);
            }
            self.visiting.insert(n);
            let walk = self.run(body, true, depth + 1);
            self.visiting.remove(&n);
            let walk = walk?;
            let clamp_u32 = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
            let info = FunctionInfo {
                offset,
                stack_effect: walk.end.effect_known.then(|| i32::try_from(walk.end.depth().saturating_neg()).ok()).flatten(),
                max_stack: clamp_u32(walk.max),
                consumes: walk.end.low_known.then(|| clamp_u32(walk.end.low.saturating_neg())),
                uncertain: walk.uncertain,
            };
            self.memo.insert(n, info);
            return Ok(Some(info));
        }
        Ok(self.external.and_then(|t| t.functions.get(&n).copied()))
    }

    fn note_function(&mut self, n: u32) {
        let n = i64::from(n);
        self.res.function = Some(self.res.function.map_or(n, |f| f.max(n)));
    }

    /// Saves the state at `IF`, within the copy budget.
    fn save(&mut self, st: &mut State, uncertain: &mut bool) -> State {
        if self.copied.saturating_add(st.vals.len()) > COPY_BUDGET {
            st.forget_values();
            *uncertain = true;
        }
        self.copied = self.copied.saturating_add(st.vals.len());
        st.clone()
    }

    fn run(&mut self, range: Range<usize>, in_function: bool, call_depth: usize) -> Result<Walk, AsmError> {
        let mut st = State::new();
        let mut max = 0i64;
        let mut uncertain = false;
        // Per open IF: the state on entry (or, after ELSE, a placeholder), the state at the end of
        // the THEN branch once ELSE is seen, and the IF's offset.
        let mut frames: Vec<(State, Option<State>, usize)> = Vec::new();
        let mut i = range.start;
        while i < range.end {
            let Some(ins) = self.ins.get(i) else { break };
            match ins.opcode {
                FDEF | IDEF => {
                    let what = if ins.opcode == FDEF { "FDEF" } else { "IDEF" };
                    if in_function {
                        return Err(AsmError::Unbalanced { what, offset: ins.offset });
                    }
                    let n = st.pop();
                    let end = (i + 1..range.end)
                        .find(|&j| self.ins.get(j).is_some_and(|x| x.opcode == ENDF))
                        .ok_or(AsmError::Unbalanced { what, offset: ins.offset })?;
                    if ins.opcode == FDEF {
                        match n.and_then(|n| u32::try_from(n).ok()) {
                            Some(n) => {
                                self.note_function(n);
                                if !self.bodies.contains_key(&n) && self.bodies.len() >= MAX_FUNCTIONS {
                                    return Err(AsmError::TooLarge { what: "functions in a program", limit: MAX_FUNCTIONS });
                                }
                                self.bodies.entry(n).or_insert((ins.offset, i + 1..end));
                            }
                            None => uncertain = true,
                        }
                    } else {
                        match n {
                            Some(op) => {
                                self.res.idefs.insert(op);
                            }
                            None => self.res.unknown_idefs = self.res.unknown_idefs.saturating_add(1),
                        }
                    }
                    max = max.max(st.depth());
                    i = end + 1;
                    continue;
                }
                ENDF => return Err(AsmError::Unbalanced { what: "ENDF", offset: ins.offset }),
                IF => {
                    st.pop();
                    let entry = self.save(&mut st, &mut uncertain);
                    frames.push((entry, None, ins.offset));
                }
                ELSE => {
                    let Some(frame) = frames.last_mut() else {
                        return Err(AsmError::Unbalanced { what: "ELSE", offset: ins.offset });
                    };
                    if frame.1.is_some() {
                        return Err(AsmError::Unbalanced { what: "ELSE", offset: ins.offset });
                    }
                    let entry = std::mem::replace(&mut frame.0, State::new());
                    frame.1 = Some(std::mem::replace(&mut st, entry));
                }
                EIF => {
                    let Some((entry, then_end, _)) = frames.pop() else {
                        return Err(AsmError::Unbalanced { what: "EIF", offset: ins.offset });
                    };
                    let other = then_end.unwrap_or(entry);
                    st = merge(st, other, &mut uncertain);
                }
                _ => self.step(ins, &mut st, &mut uncertain, call_depth, &mut max)?,
            }
            max = max.max(st.depth());
            i += 1;
        }
        if let Some((_, _, offset)) = frames.last() {
            return Err(AsmError::Unbalanced { what: "IF", offset: *offset });
        }
        Ok(Walk { max, end: st, uncertain })
    }

    /// Records a point index taken from the stack in zone pointer `zone` (0 to 2).
    fn point(&mut self, st: &State, zone: usize, p: Option<i32>, uncertain: &mut bool) {
        match st.zp.get(zone).copied().flatten() {
            Some(1) => {}
            Some(0) => match p {
                Some(p) if p >= 0 => self.res.twilight = self.res.twilight.max(i64::from(p) + 1),
                _ => *uncertain = true,
            },
            _ => *uncertain = true,
        }
    }

    fn storage(&mut self, index: Option<i32>, uncertain: &mut bool) {
        match index {
            Some(i) if i >= 0 => self.res.storage = self.res.storage.max(i64::from(i) + 1),
            _ => *uncertain = true,
        }
    }

    /// Pops `SLOOP`'s count of points in zone `zone` and resets the loop count.
    fn loop_points(&mut self, st: &mut State, zone: usize, uncertain: &mut bool) {
        let n = match st.loop_n {
            Some(n) => n.max(0),
            None => {
                *uncertain = true;
                1
            }
        };
        st.loop_n = Some(1);
        let tracked = n.min(to_i64(st.vals.len()));
        for _ in 0..tracked {
            let p = st.pop();
            self.point(st, zone, p, uncertain);
        }
        let rest = n.saturating_sub(tracked);
        if rest > 0 {
            st.pop_n(rest);
            self.point(st, zone, None, uncertain);
        }
    }

    /// `DELTAP*` (`points`) and `DELTAC*`: a count, then that many pairs.
    fn delta(&mut self, st: &mut State, points: bool, uncertain: &mut bool) {
        let Some(n) = st.pop() else {
            *uncertain = true;
            st.lose_track();
            return;
        };
        let pairs = i64::from(n.max(0));
        if points {
            let tracked = pairs.min(to_i64(st.vals.len() / 2));
            for _ in 0..tracked {
                let p = st.pop();
                self.point(st, 0, p, uncertain);
                st.pop();
            }
            let rest = pairs.saturating_sub(tracked);
            if rest > 0 {
                st.pop_n(rest.saturating_mul(2));
                self.point(st, 0, None, uncertain);
            }
        } else {
            st.pop_n(pairs.saturating_mul(2));
        }
    }

    fn set_zone(&mut self, st: &mut State, zones: Range<usize>) {
        let v = st.pop();
        let z = match v {
            Some(0) => Some(0),
            Some(1) => Some(1),
            _ => None,
        };
        if z != Some(1) {
            self.res.twilight_zone = true;
        }
        for k in zones {
            if let Some(slot) = st.zp.get_mut(k) {
                *slot = z;
            }
        }
    }

    fn call(
        &mut self,
        st: &mut State,
        f: Option<i32>,
        count: Option<i32>,
        call_depth: usize,
        max: &mut i64,
        uncertain: &mut bool,
    ) -> Result<(), AsmError> {
        let calls = match count {
            Some(c) => i64::from(c.max(0)),
            None => {
                *uncertain = true;
                1
            }
        };
        let Some(f) = f.and_then(|f| u32::try_from(f).ok()) else {
            *uncertain = true;
            st.lose_track();
            return Ok(());
        };
        self.note_function(f);
        if calls == 0 {
            return Ok(());
        }
        let Some(info) = self.callee(f, call_depth)? else {
            *uncertain = true;
            st.lose_track();
            return Ok(());
        };
        *uncertain |= info.uncertain;
        let base = st.depth();
        let Some(effect) = info.stack_effect else {
            *max = (*max).max(base.saturating_add(i64::from(info.max_stack)));
            *uncertain = true;
            st.lose_track();
            return Ok(());
        };
        let net = i64::from(effect).saturating_neg();
        let growth = if net > 0 { (calls - 1).saturating_mul(net) } else { 0 };
        *max = (*max).max(base.saturating_add(i64::from(info.max_stack)).saturating_add(growth));
        match info.consumes {
            Some(c) if calls == 1 => {
                let c = i64::from(c);
                st.pop_n(c);
                st.shift(c.saturating_add(net));
            }
            consumes => {
                st.invalidate();
                let shrink = if net < 0 { (calls - 1).saturating_mul(net) } else { 0 };
                match consumes {
                    Some(c) => st.low = st.low.min(base.saturating_sub(i64::from(c)).saturating_add(shrink)),
                    None => st.low_known = false,
                }
                st.shift(calls.saturating_mul(net));
            }
        }
        Ok(())
    }

    fn step(&mut self, ins: &Instruction, st: &mut State, u: &mut bool, call_depth: usize, max: &mut i64) -> Result<(), AsmError> {
        let op = ins.opcode;
        if table::push_form(op).is_some() {
            for v in &ins.data {
                st.push(Some(*v));
            }
            return Ok(());
        }
        match op {
            // No stack effect.
            0x00..=0x05 | 0x0E | 0x18 | 0x19 | 0x30 | 0x31 | 0x3D | 0x4D | 0x4E | 0x7A | 0x7C | 0x7D => {}
            // Pop one.
            0x10..=0x12 | 0x1A | 0x1D..=0x1F | 0x21 | 0x34..=0x37 | 0x4F | 0x5E | 0x5F | 0x76 | 0x77 | 0x7E | 0x7F | 0x85 | 0x8D => {
                st.pop();
            }
            // Pop two.
            0x0A | 0x0B | 0x44 | 0x70 | 0x81 | 0x82 | 0x8E => st.pop_n(2),
            // Pop one, push an unknown value.
            0x45 | 0x56 | 0x57 | 0x5C | 0x66..=0x6F | 0x88 => {
                st.pop();
                st.push(None);
            }
            // Pop two, push an unknown value.
            0x50..=0x55 | 0x5A | 0x5B | 0x62 | 0x63 | 0x8B | 0x8C => {
                st.pop_n(2);
                st.push(None);
            }
            // ADD, SUB.
            0x60 | 0x61 => {
                let b = st.pop();
                let a = st.pop();
                st.push(a.zip(b).map(|(a, b)| if op == 0x60 { a.wrapping_add(b) } else { a.wrapping_sub(b) }));
            }
            // ABS, NEG.
            0x64 | 0x65 => {
                let a = st.pop();
                st.push(a.map(|a| if op == 0x64 { a.wrapping_abs() } else { a.wrapping_neg() }));
            }
            // DEPTH, MPPEM, MPS.
            0x24 | 0x4B | 0x4C => st.push(None),
            // GPV, GFV.
            0x0C | 0x0D => st.push_unknown(2),
            // SPVTL, SFVTL, SDPVTL: p1 in zp2, p2 in zp1.
            0x06..=0x09 | 0x86 | 0x87 => {
                let p1 = st.pop();
                self.point(st, 2, p1, u);
                let p2 = st.pop();
                self.point(st, 1, p2, u);
            }
            // ISECT: b1, b0 in zp0; a1, a0 in zp1; p in zp2.
            0x0F => {
                for zone in [0, 0, 1, 1, 2] {
                    let p = st.pop();
                    self.point(st, zone, p, u);
                }
            }
            0x13 => self.set_zone(st, 0..1),
            0x14 => self.set_zone(st, 1..2),
            0x15 => self.set_zone(st, 2..3),
            0x16 => self.set_zone(st, 0..3),
            // SLOOP.
            0x17 => st.loop_n = st.pop().map(i64::from),
            // JMPR, JROT, JROF: jumps are not followed.
            0x1C => {
                st.pop();
                *u = true;
            }
            0x78 | 0x79 => {
                st.pop_n(2);
                *u = true;
            }
            // DUP.
            0x20 => {
                let v = st.pop();
                st.push(v);
                st.push(v);
            }
            // CLEAR: the depth relative to a caller is lost.
            0x22 => {
                st.vals.clear();
                st.below = 0;
                st.effect_known = false;
                st.low_known = false;
            }
            // SWAP.
            0x23 => {
                let a = st.pop();
                let b = st.pop();
                st.push(a);
                st.push(b);
            }
            // CINDEX.
            0x25 => {
                let k = st.pop();
                let len = st.vals.len();
                let v =
                    k.and_then(|k| usize::try_from(k).ok()).filter(|k| (1..=len).contains(k)).and_then(|k| st.vals.get(len - k).copied().flatten());
                st.push(v);
            }
            // MINDEX.
            0x26 => {
                let k = st.pop();
                let len = st.vals.len();
                match k.and_then(|k| usize::try_from(k).ok()) {
                    Some(k) if (1..=len).contains(&k) => {
                        let v = st.vals.remove(len - k);
                        st.push(v);
                    }
                    Some(k) if k > len => {
                        st.below = st.below.saturating_sub(1);
                        st.low = st.low.min(st.depth().saturating_sub(to_i64(k)).saturating_add(1));
                        st.push(None);
                    }
                    _ => {
                        *u = true;
                        st.invalidate();
                        st.low_known = false;
                    }
                }
            }
            // ALIGNPTS: p2 in zp0, p1 in zp1.
            0x27 => {
                let p2 = st.pop();
                self.point(st, 0, p2, u);
                let p1 = st.pop();
                self.point(st, 1, p1, u);
            }
            // UTP, MDAP: zp0.
            0x29 | 0x2E | 0x2F => {
                let p = st.pop();
                self.point(st, 0, p, u);
            }
            // LOOPCALL.
            0x2A => {
                let f = st.pop();
                let count = st.pop();
                self.call(st, f, count, call_depth, max, u)?;
            }
            // CALL.
            0x2B => {
                let f = st.pop();
                self.call(st, f, Some(1), call_depth, max, u)?;
            }
            // SHP, IP: zp2; ALIGNRP: zp1.
            0x32 | 0x33 | 0x39 => self.loop_points(st, 2, u),
            0x3C => self.loop_points(st, 1, u),
            // SHPIX: a distance, then points in zp2.
            0x38 => {
                st.pop();
                self.loop_points(st, 2, u);
            }
            // MSIRP: a distance, a point in zp1.
            0x3A | 0x3B => {
                st.pop();
                let p = st.pop();
                self.point(st, 1, p, u);
            }
            // MIAP: a cvt entry, a point in zp0.
            0x3E | 0x3F => {
                st.pop();
                let p = st.pop();
                self.point(st, 0, p, u);
            }
            // WS.
            0x42 => {
                st.pop();
                let i = st.pop();
                self.storage(i, u);
            }
            // RS.
            0x43 => {
                let i = st.pop();
                self.storage(i, u);
                st.push(None);
            }
            // GC: a point in zp2.
            0x46 | 0x47 => {
                let p = st.pop();
                self.point(st, 2, p, u);
                st.push(None);
            }
            // SCFS: a value, a point in zp2.
            0x48 => {
                st.pop();
                let p = st.pop();
                self.point(st, 2, p, u);
            }
            // MD: p2 in zp1, p1 in zp0.
            0x49 | 0x4A => {
                let p2 = st.pop();
                self.point(st, 1, p2, u);
                let p1 = st.pop();
                self.point(st, 0, p1, u);
                st.push(None);
            }
            0x5D | 0x71 | 0x72 => self.delta(st, true, u),
            0x73..=0x75 => self.delta(st, false, u),
            // FLIPPT: points of the glyph zone.
            0x80 => {
                let n = match st.loop_n {
                    Some(n) => n.max(0),
                    None => {
                        *u = true;
                        1
                    }
                };
                st.loop_n = Some(1);
                st.pop_n(n);
            }
            // ROLL: (c b a) → (b a c).
            0x8A => {
                let a = st.pop();
                let b = st.pop();
                let c = st.pop();
                st.push(b);
                st.push(a);
                st.push(c);
            }
            // MDRP: a point in zp1.
            0xC0..=0xDF => {
                let p = st.pop();
                self.point(st, 1, p, u);
            }
            // MIRP: a cvt entry, a point in zp1.
            0xE0..=0xFF => {
                st.pop();
                let p = st.pop();
                self.point(st, 1, p, u);
            }
            // GETVARIATION pushes one value per axis; an unused opcode may be an IDEF.
            _ => {
                *u = true;
                st.lose_track();
            }
        }
        Ok(())
    }
}
