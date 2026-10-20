//! Choosing the subroutines: the iterative market of compreffor's `pyCompressor` (Apache-2.0,
//! Copyright 2015 Google Inc.), ported over the candidates [`super::find`] takes from the tree of
//! repeats.
//!
//! Terms: a *candidate* is a repeated token sequence (a node of the tree) that could become a
//! subroutine; its *cost* is its bytes; its *price* is what using it costs in the current round;
//! its *usages* are how many encodings (of glyphs and of other candidates) call it. A candidate
//! that ends up unused, or whose calls cost more than they save, is *flattened*: written out
//! wherever it was chosen, with its own calls kept.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::find::{self, Text};
use super::flat::Flat;
use super::memory::Memory;
use super::suffix::NONE;
use super::{MAX_CALL_DEPTH, MAX_CANDIDATES_PER_POSITION, SubrError, bias, push_int};

/// Weight of the newest marginal cost in a price (compreffor's `ALPHA`).
const ALPHA: f64 = 0.1;
/// Added to the usages before dividing by them (compreffor's `K`).
const K: f64 = 0.1;
/// Bytes a call is assumed to cost when estimating savings (compreffor's choice; a real call
/// costs two to four).
pub(super) const CALL_COST: i64 = 5;
/// Bytes a subroutine costs beyond its body: the `return` and its INDEX offset.
pub(super) const SUBR_OVERHEAD: i64 = 3;
/// A reached candidate's state when glyphs of more than one Font DICT reach it.
const MULTI: u32 = u32::MAX - 1;

pub(super) struct Params {
    pub fd_count: usize,
    pub cff2: bool,
    pub max_stack: usize,
    pub max_subrs: usize,
    pub rounds: usize,
}

pub(super) struct Output {
    pub charstrings: Vec<Vec<u8>>,
    pub global: Vec<Vec<u8>>,
    pub local: Vec<Vec<Vec<u8>>>,
    /// Tokens of the text, separators included, and candidates found.
    pub tokens: usize,
    pub candidates: usize,
}

/// One call in an encoding: where in the encoded sequence it starts, and the candidate.
type Call = (u32, u32);

pub(super) struct Cand {
    pub len: u32,
    pub start: u32,
    pub cost: i64,
    pub price: f64,
    pub adjusted: f64,
    pub usages: i64,
    /// Its encoding: `(first, count)` in [`Market::calls`].
    pub enc: (u32, u32),
    /// The nearest candidate above it in the tree of repeats, and the nearest live one.
    pub parent: u32,
    pub up: u32,
    /// Tokens before its first stack-clearing operator (at most its length).
    pub first_clear: u32,
    /// Index in its INDEX, once placed, and the bias of that INDEX.
    pub position: u32,
    pub call_bias: u16,
    /// Largest stack depth at the start of any occurrence, up to its first stack-clearing operator.
    pub entry: u16,
    pub alive: bool,
    pub flatten: bool,
    pub global: bool,
}

impl Cand {
    pub(super) fn new(len: u32) -> Self {
        Cand {
            len,
            start: 0,
            cost: 0,
            price: 0.0,
            adjusted: 0.0,
            usages: 0,
            enc: (0, 0),
            parent: NONE,
            up: NONE,
            first_clear: 0,
            position: 0,
            call_bias: 0,
            entry: 0,
            alive: true,
            flatten: false,
            global: false,
        }
    }

    /// Bytes saved by making this a subroutine, by compreffor's estimate.
    fn saving(&self, amount: i64, cost: i64) -> i64 {
        cost * amount - cost - CALL_COST * amount - SUBR_OVERHEAD
    }
}

struct Market<'a> {
    p: &'a Params,
    t: Text,
    /// Per text position, the deepest candidate starting there.
    deep: Vec<u32>,
    cands: Vec<Cand>,
    /// Candidates in order of length, so that the enclosing ones come first.
    order: Vec<u32>,
    /// The encodings of the live candidates in the latest round.
    calls: Vec<Call>,
    mem: &'a mut Memory,
}

/// Where calls may start within the sequence being encoded.
#[derive(Clone, Copy)]
enum Site {
    /// A glyph: the stack depth at each position is known.
    Glyph,
    /// A candidate: before its first stack-clearing operator the depth depends on the caller,
    /// so the largest over its occurrences counts.
    Subr { entry: u16, first_clear: u32 },
}

/// Subroutinises glyphs `0..glyphs`, each produced by `next` as it is needed.
pub(super) fn subroutinize(
    glyphs: usize,
    next: impl FnMut(usize) -> Result<Flat, SubrError>,
    fd_of: &[usize],
    p: &Params,
    mem: &mut Memory,
) -> Result<Output, SubrError> {
    let mut t = find::read(glyphs, next, mem)?;
    let (cands, deep) = find::find(&mut t, mem)?;
    let mut order: Vec<u32> = (0..cands.len() as u32).collect();
    order.sort_by_key(|&c| cands.get(c as usize).map_or(0, |x| x.len));
    mem.charge_for::<u32>(order.len())?;
    let mut m = Market { p, t, deep, cands, order, calls: Vec::new(), mem };
    m.refresh_up();
    let glyph_enc = m.run_rounds()?;
    m.finish(&glyph_enc, fd_of)
}

impl Market<'_> {
    fn cand(&self, c: u32) -> Option<&Cand> {
        self.cands.get(c as usize)
    }

    fn alive(&self, c: u32) -> bool {
        self.cand(c).is_some_and(|c| c.alive)
    }

    /// The deepest live candidate at or above `c`.
    fn first(&self, c: u32) -> u32 {
        if self.alive(c) { c } else { self.cand(c).map_or(NONE, |x| x.up) }
    }

    /// The encoding of candidate `c` in the latest round.
    fn enc(&self, c: usize) -> &[Call] {
        let (from, count) = self.cands.get(c).map_or((0, 0), |x| x.enc);
        self.calls.get(from as usize..(from + count) as usize).unwrap_or_default()
    }

    fn callees(&self, c: usize) -> Vec<u32> {
        self.enc(c).iter().map(|e| e.1).collect()
    }

    fn refresh_up(&mut self) {
        for k in 0..self.order.len() {
            let c = self.order.get(k).copied().unwrap_or(NONE);
            let parent = self.cand(c).map_or(NONE, |x| x.parent);
            let up = if parent == NONE {
                NONE
            } else if self.alive(parent) {
                parent
            } else {
                self.cand(parent).map_or(NONE, |x| x.up)
            };
            if let Some(x) = self.cands.get_mut(c as usize) {
                x.up = up;
            }
        }
    }

    /// The cheapest encoding of the `len` tokens at `from` with the current prices, and its cost.
    /// Candidate `skip` (the sequence being encoded) is not used.
    fn encode(&self, from: usize, len: usize, skip: u32, site: Site) -> (f64, Vec<Call>) {
        let mut results = vec![0f64; len + 1];
        let mut choice = vec![NONE; len];
        let max_stack = self.p.max_stack;
        for j in (0..len).rev() {
            let at = from + j;
            let tail = |k: usize| results.get(k).copied().unwrap_or(f64::INFINITY);
            let mut best = f64::from(self.t.cost(at)) + tail(j + 1);
            let mut pick = NONE;
            let depth = match site {
                Site::Subr { entry, first_clear } if j <= first_clear as usize => entry,
                _ => self.t.depth.get(at).copied().unwrap_or(u16::MAX),
            };
            // The call pushes the subroutine number.
            if usize::from(depth) < max_stack {
                let mut c = self.first(self.deep.get(at).copied().unwrap_or(NONE));
                let mut seen = 0usize;
                while seen < MAX_CANDIDATES_PER_POSITION {
                    let Some(cand) = self.cand(c) else { break };
                    seen += 1;
                    let clen = cand.len as usize;
                    if c != skip && clen <= len - j {
                        let option = cand.price + tail(j + clen);
                        if option < best {
                            best = option;
                            pick = c;
                        }
                    }
                    c = cand.up;
                }
            }
            if let (Some(r), Some(ch)) = (results.get_mut(j), choice.get_mut(j)) {
                (*r, *ch) = (best, pick);
            }
        }
        let mut enc = Vec::new();
        let mut j = 0usize;
        while j < len {
            let c = choice.get(j).copied().unwrap_or(NONE);
            match self.cand(c) {
                Some(cand) => {
                    enc.push((j as u32, c));
                    j += (cand.len as usize).max(1);
                }
                None => j += 1,
            }
        }
        (results.first().copied().unwrap_or(0.0), enc)
    }

    fn encode_glyphs(&self) -> Vec<Vec<Call>> {
        let t = &self.t;
        t.glyph_start.iter().zip(&t.glyph_len).map(|(&start, &len)| self.encode(start as usize, len as usize, NONE, Site::Glyph).1).collect()
    }

    fn live(&self) -> Vec<usize> {
        (0..self.cands.len()).filter(|&c| self.cands.get(c).is_some_and(|c| c.alive)).collect()
    }

    /// The rounds of the market; returns the glyph encodings of the last one.
    fn run_rounds(&mut self) -> Result<Vec<Vec<Call>>, SubrError> {
        let mut glyph_enc: Vec<Vec<Call>> = Vec::new();
        let mut glyph_bytes = 0usize;
        for round in 0..self.p.rounds {
            let live = self.live();
            self.mem.charge_for::<usize>(live.len())?;
            for &c in &live {
                if let Some(cand) = self.cands.get_mut(c) {
                    let marginal = cand.adjusted / (cand.usages as f64 + K);
                    cand.price = marginal * ALPHA + cand.price * (1.0 - ALPHA);
                }
            }
            // The encodings of this round go into a new list; the old one is read by nothing
            // until it is replaced.
            let mut calls: Vec<Call> = Vec::new();
            let mut charged = 0usize;
            for &c in &live {
                let Some(cand) = self.cands.get(c) else { continue };
                let site = Site::Subr { entry: cand.entry, first_clear: cand.first_clear };
                let (cost, enc) = self.encode(cand.start as usize, cand.len as usize, c as u32, site);
                let from = calls.len() as u32;
                calls.extend_from_slice(&enc);
                let held = calls.capacity() * size_of::<Call>();
                if held > charged {
                    self.mem.charge(held - charged)?;
                    charged = held;
                }
                if let Some(cand) = self.cands.get_mut(c) {
                    cand.adjusted = cost;
                    cand.enc = (from, enc.len() as u32);
                }
            }
            let old = std::mem::replace(&mut self.calls, calls);
            self.mem.release_for::<Call>(old.capacity());
            drop(old);

            let new_enc = self.encode_glyphs();
            let new_bytes = new_enc.iter().map(|e| e.capacity() * size_of::<Call>() + size_of::<Vec<Call>>()).sum::<usize>();
            self.mem.charge(new_bytes)?;
            self.mem.release(glyph_bytes);
            (glyph_enc, glyph_bytes) = (new_enc, new_bytes);

            self.mem.charge_for::<i64>(self.cands.len())?;
            let mut counts = vec![0i64; self.cands.len()];
            let calls = live.iter().flat_map(|&c| self.enc(c).iter()).chain(glyph_enc.iter().flatten());
            for &(_, e) in calls {
                if let Some(n) = counts.get_mut(e as usize) {
                    *n += 1;
                }
            }
            for (cand, n) in self.cands.iter_mut().zip(counts) {
                cand.usages = n;
            }
            self.mem.release_for::<i64>(self.cands.len());

            if round + 2 <= self.p.rounds {
                let bad: Vec<usize> = live.iter().copied().filter(|&c| self.cands.get(c).is_some_and(|x| x.saving(x.usages, x.cost) <= 0)).collect();
                // compreffor's heuristic: what a dropped candidate called stays attractive.
                for &b in &bad {
                    let Some(cand) = self.cands.get(b) else { continue };
                    let u = cand.usages;
                    for e in self.callees(b) {
                        if let Some(callee) = self.cands.get_mut(e as usize) {
                            callee.usages += u - 1;
                        }
                    }
                }
                for &b in &bad {
                    if let Some(cand) = self.cands.get_mut(b) {
                        cand.alive = false;
                        cand.enc = (0, 0);
                    }
                }
                self.refresh_up();
            }
            self.mem.release_for::<usize>(live.len());
        }
        Ok(glyph_enc)
    }
}

/// A subroutine INDEX being filled: its members in the order they are placed, and the usages of
/// its 215 and 2,263 most used members, which decide what a call to one more would cost. Sorting
/// the members by usages, stably, gives the order compreffor's insertion gives.
#[derive(Clone, Default)]
struct Filling {
    members: Vec<usize>,
    top_215: BinaryHeap<Reverse<i64>>,
    top_2263: BinaryHeap<Reverse<i64>>,
}

impl Filling {
    fn len(&self) -> usize {
        self.members.len()
    }

    /// What a call to one more subroutine, used `usages` times, would cost by its usage rank
    /// (compreffor's `test_call_cost`).
    fn call_cost(&self, usages: i64) -> u8 {
        let reaches = |top: &BinaryHeap<Reverse<i64>>, rank: usize| top.len() >= rank && top.peek().is_some_and(|least| least.0 >= usages);
        if reaches(&self.top_2263, 2263) {
            3
        } else if reaches(&self.top_215, 215) {
            2
        } else {
            1
        }
    }

    fn push(&mut self, c: usize, usages: i64) {
        self.members.push(c);
        for (top, rank) in [(&mut self.top_215, 215), (&mut self.top_2263, 2263)] {
            top.push(Reverse(usages));
            if top.len() > rank {
                top.pop();
            }
        }
    }
}

/// `list` reordered so that the most used subroutines get the shortest numbers for its bias.
fn arrange(list: &[usize]) -> Vec<usize> {
    let seg = |a: usize, b: usize| list.get(a.min(list.len())..b.min(list.len())).unwrap_or_default();
    match bias(list.len()) {
        1131 => [seg(216, 1240), seg(0, 216), seg(1240, usize::MAX)].concat(),
        32_768 => [seg(2264, 33_901), seg(216, 1240), seg(0, 216), seg(1240, 2264), seg(33_901, usize::MAX)].concat(),
        _ => list.to_vec(),
    }
}

impl Market<'_> {
    fn usages(&self, c: usize) -> i64 {
        self.cands.get(c).map_or(0, |x| x.usages)
    }

    fn flattened(&self, c: u32) -> bool {
        self.cand(c).is_some_and(|x| x.flatten)
    }

    fn set_flatten(&mut self, c: usize) {
        if let Some(x) = self.cands.get_mut(c) {
            x.flatten = true;
        }
    }

    /// Places the subroutines, writes every program and the glyphs.
    fn finish(mut self, glyph_enc: &[Vec<Call>], fd_of: &[usize]) -> Result<Output, SubrError> {
        let n = self.cands.len();
        let fd_count = self.p.fd_count.max(1);

        // Per candidate: the Font DICT state, the real cost, the nesting level, the program and
        // two lists of the live ones.
        self.mem.charge(n.saturating_mul(4 + 8 + 8 + size_of::<Vec<u8>>() + 2 * size_of::<usize>()))?;
        let mut programs: Vec<Vec<u8>> = vec![Vec::new(); n];
        // Which Font DICTs reach each candidate.
        let mut state = vec![NONE; n];
        let mut work: Vec<(u32, u32)> = Vec::new();
        for (g, enc) in glyph_enc.iter().enumerate() {
            let fd = fd_of.get(g).copied().unwrap_or(0) as u32;
            work.extend(enc.iter().map(|&(_, c)| (c, fd)));
        }
        while let Some((c, fd)) = work.pop() {
            let Some(s) = state.get_mut(c as usize) else { continue };
            let new = if *s == NONE {
                fd
            } else if *s == fd || *s == MULTI {
                continue;
            } else {
                MULTI
            };
            *s = new;
            work.extend(self.enc(c as usize).iter().map(|&(_, e)| (e, new)));
        }

        // Keep what pays for itself, counting the calls inside each candidate.
        let live = self.live();
        let mut by_len = live.clone();
        by_len.sort_by_key(|&c| self.cands.get(c).map_or(0, |x| x.len));
        let mut real = vec![0i64; n];
        let callee_cost = |cands: &[Cand], e: u32| cands.get(e as usize).map_or(0, |x| x.cost);
        for &c in &live {
            let Some(cand) = self.cands.get(c) else { continue };
            let r = cand.cost + self.enc(c).iter().map(|&(_, e)| CALL_COST - callee_cost(&self.cands, e)).sum::<i64>();
            if let Some(slot) = real.get_mut(c) {
                *slot = r;
            }
        }
        for &c in &live {
            let keep = self.cands.get(c).is_some_and(|x| {
                x.usages > 0 && state.get(c).is_some_and(|&s| s != NONE) && x.saving(x.usages, real.get(c).copied().unwrap_or(0)) > 0
            });
            if !keep {
                self.set_flatten(c);
            }
        }
        // compreffor adds a flattened callee's whole real cost here; only its own calls change
        // the caller's size, so that is what counts.
        for &c in &by_len {
            let Some(cand) = self.cands.get(c) else { continue };
            let mut r = cand.cost;
            for &(_, e) in self.enc(c) {
                let ec = callee_cost(&self.cands, e);
                r += if self.flattened(e) { real.get(e as usize).copied().unwrap_or(ec) - ec } else { CALL_COST - ec };
            }
            if let Some(slot) = real.get_mut(c) {
                *slot = r;
            }
        }

        let mut subrs: Vec<usize> = live.iter().copied().filter(|&c| !self.flattened(c as u32)).collect();
        let saving = |m: &Self, c: usize| m.cands.get(c).map_or(0, |x| x.saving(x.usages, real.get(c).copied().unwrap_or(0)));
        subrs.sort_by(|&a, &b| saving(&self, a).cmp(&saving(&self, b)).then(b.cmp(&a)));
        let limit = self.p.max_subrs;
        let mut gsubrs = Filling::default();
        let mut lsubrs = vec![Filling::default(); fd_count];
        let mut leftover = Vec::new();
        while !subrs.is_empty() && (gsubrs.len() < limit || lsubrs.iter().any(|l| l.len() < limit)) {
            let Some(c) = subrs.pop() else { break };
            let s = state.get(c).copied().unwrap_or(NONE);
            let u = self.usages(c);
            let global = if s != MULTI {
                let Some(local) = lsubrs.get_mut(s as usize) else {
                    leftover.push(c);
                    continue;
                };
                let room_g = gsubrs.len() < limit;
                let room_l = local.len() < limit;
                if room_g && room_l {
                    let global = gsubrs.call_cost(u) < local.call_cost(u);
                    (if global { &mut gsubrs } else { local }).push(c, u);
                    global
                } else if room_g {
                    gsubrs.push(c, u);
                    true
                } else if room_l {
                    local.push(c, u);
                    false
                } else {
                    leftover.push(c);
                    continue;
                }
            } else if gsubrs.len() < limit {
                gsubrs.push(c, u);
                true
            } else {
                leftover.push(c);
                continue;
            };
            if let Some(x) = self.cands.get_mut(c) {
                x.global = global;
            }
        }
        let mut gsubrs = gsubrs.members;
        let mut lsubrs: Vec<Vec<usize>> = lsubrs.into_iter().map(|l| l.members).collect();
        leftover.extend(subrs);
        for c in leftover {
            self.set_flatten(c);
        }

        // With several Font DICTs a global subroutine cannot know whose local ones to call.
        if fd_count > 1 {
            let mut seen = vec![false; n];
            let mut stack = gsubrs.clone();
            while let Some(s) = stack.pop() {
                let callees = self.callees(s);
                for e in callees {
                    let global = self.cand(e).is_some_and(|x| x.global);
                    if let Some(flag) = seen.get_mut(e as usize).filter(|f| !**f && !global) {
                        *flag = true;
                        stack.push(e as usize);
                        self.set_flatten(e as usize);
                    }
                }
            }
        }

        // Nesting: the call depth at which each candidate's body runs, glyphs being depth 0.
        let mut level = vec![-1i64; n];
        for &(_, e) in glyph_enc.iter().flatten() {
            let lv = if self.flattened(e) { 0 } else { 1 };
            if let Some(l) = level.get_mut(e as usize) {
                *l = (*l).max(lv);
            }
        }
        for &c in by_len.iter().rev() {
            let lc = level.get(c).copied().unwrap_or(-1);
            if lc < 0 {
                continue;
            }
            let callees = self.callees(c);
            for e in callees {
                let lv = lc + if self.flattened(e) { 0 } else { 1 };
                if let Some(l) = level.get_mut(e as usize) {
                    *l = (*l).max(lv);
                }
            }
        }
        for c in 0..n {
            if !self.flattened(c as u32) && level.get(c).copied().unwrap_or(0) > MAX_CALL_DEPTH as i64 {
                self.set_flatten(c);
            }
        }
        let cands = &self.cands;
        let kept = |c: &usize| cands.get(*c).is_some_and(|x| !x.flatten);
        gsubrs.retain(kept);
        for l in &mut lsubrs {
            l.retain(kept);
        }

        // Numbers: most used first, then arranged for the bias.
        let mut lists: Vec<Vec<usize>> = std::iter::once(gsubrs).chain(lsubrs).collect();
        for list in &mut lists {
            list.sort_by_key(|&a| std::cmp::Reverse(self.usages(a)));
            *list = arrange(list);
            let b = bias(list.len());
            for (pos, &c) in list.iter().enumerate() {
                if let Some(x) = self.cands.get_mut(c) {
                    x.position = pos as u32;
                    x.call_bias = u16::try_from(b).unwrap_or(u16::MAX);
                }
            }
        }

        // Programs: flattened candidates shortest first (each may contain shorter ones), then
        // the subroutines.
        for &c in &by_len {
            if self.flattened(c as u32) && state.get(c).is_some_and(|&s| s != NONE) {
                let program = self.program(c, &programs)?;
                self.mem.charge(program.len())?;
                if let Some(slot) = programs.get_mut(c) {
                    *slot = program;
                }
            }
        }
        for &c in lists.iter().flatten() {
            let mut program = self.program(c, &programs)?;
            let last = self.cands.get(c).and_then(|x| self.t.text.get(((x.start + x.len) as usize).saturating_sub(1))).copied();
            let ends = last.and_then(|t| self.t.token(t)) == Some(&[14][..]);
            if !self.p.cff2 && !ends {
                program.push(11);
            }
            if program.len() > super::MAX_CHARSTRING {
                return Err(SubrError::TooLarge { what: "bytes in one subroutine", limit: super::MAX_CHARSTRING });
            }
            self.mem.charge(program.len())?;
            if let Some(slot) = programs.get_mut(c) {
                *slot = program;
            }
        }
        let mut charstrings = Vec::with_capacity(glyph_enc.len());
        for (g, enc) in glyph_enc.iter().enumerate() {
            let start = self.t.glyph_start.get(g).copied().unwrap_or(0) as usize;
            let len = self.t.glyph_len.get(g).copied().unwrap_or(0) as usize;
            let charstring = self.write(start, len, enc, &programs)?;
            self.mem.charge(charstring.len())?;
            charstrings.push(charstring);
        }
        let mut take = |c: &usize| programs.get_mut(*c).map(std::mem::take).unwrap_or_default();
        let mut lists = lists.into_iter();
        let global = lists.next().unwrap_or_default().iter().map(&mut take).collect();
        let local = lists.map(|l| l.iter().map(&mut take).collect()).collect();
        Ok(Output { charstrings, global, local, tokens: self.t.text.len(), candidates: n })
    }

    /// The body of candidate `c` with its encoding applied.
    fn program(&self, c: usize, programs: &[Vec<u8>]) -> Result<Vec<u8>, SubrError> {
        let cand = self.cands.get(c).ok_or_else(|| SubrError::Invalid("a lost candidate".into()))?;
        self.write(cand.start as usize, cand.len as usize, self.enc(c), programs)
    }

    /// The `len` tokens at `from` with the calls of `enc`: a flattened candidate is written out
    /// (its program from `programs`), a subroutine is called.
    fn write(&self, from: usize, len: usize, enc: &[Call], programs: &[Vec<u8>]) -> Result<Vec<u8>, SubrError> {
        let mut out = Vec::new();
        let mut calls = enc.iter().peekable();
        let mut j = 0usize;
        while j < len {
            if let Some(&&(at, c)) = calls.peek()
                && at as usize == j
            {
                calls.next();
                let cand = self.cand(c).ok_or_else(|| SubrError::Invalid("a call to a lost candidate".into()))?;
                if cand.flatten {
                    out.extend_from_slice(programs.get(c as usize).map(Vec::as_slice).unwrap_or_default());
                } else {
                    push_int(i64::from(cand.position) - i64::from(cand.call_bias), &mut out)?;
                    out.push(if cand.global { 29 } else { 10 });
                }
                j += (cand.len as usize).max(1);
                continue;
            }
            let id = self.t.text.get(from + j).copied().unwrap_or(NONE);
            out.extend_from_slice(self.t.token(id).unwrap_or_default());
            j += 1;
        }
        Ok(out)
    }
}
