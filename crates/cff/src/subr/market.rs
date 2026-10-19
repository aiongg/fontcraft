//! Choosing the subroutines: the iterative market of compreffor's `pyCompressor` (Apache-2.0,
//! Copyright 2015 Google Inc.), ported over the suffix tree of [`super::suffix`].
//!
//! Terms: a *candidate* is a repeated token sequence (a node of the tree) that could become a
//! subroutine; its *cost* is its bytes; its *price* is what using it costs in the current round;
//! its *usages* are how many encodings (of glyphs and of other candidates) call it. A candidate
//! that ends up unused, or whose calls cost more than they save, is *flattened*: written out
//! wherever it was chosen, with its own calls kept.

use std::collections::HashMap;

use super::flat::Flat;
use super::suffix::{self, NONE};
use super::{MAX_CALL_DEPTH, MAX_CANDIDATE_TOKENS, MAX_CANDIDATES_PER_POSITION, MAX_TOKENS, SubrError, bias, push_int};

/// Weight of the newest marginal cost in a price (compreffor's `ALPHA`).
const ALPHA: f64 = 0.1;
/// Added to the usages before dividing by them (compreffor's `K`).
const K: f64 = 0.1;
/// Bytes a call is assumed to cost when estimating savings (compreffor's choice; a real call
/// costs two to four).
const CALL_COST: i64 = 5;
/// Bytes a subroutine costs beyond its body: the `return` and its INDEX offset.
const SUBR_OVERHEAD: i64 = 3;
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
}

/// One call in an encoding: where in the encoded sequence it starts, and the candidate.
type Call = (u32, u32);

struct Cand {
    len: u32,
    start: u32,
    cost: i64,
    /// Largest stack depth at the start of any occurrence, up to its first stack-clearing operator.
    entry: u16,
    /// Tokens before its first stack-clearing operator.
    first_clear: u32,
    price: f64,
    adjusted: f64,
    usages: i64,
    enc: Vec<Call>,
    alive: bool,
    flatten: bool,
    global: bool,
    /// Index in its INDEX, once placed, and the bias of that INDEX.
    position: usize,
    call_bias: i64,
    /// Its bytes once written: a subroutine body, or what replaces it when flattened.
    program: Vec<u8>,
}

impl Cand {
    /// Bytes saved by making this a subroutine, by compreffor's estimate.
    fn saving(&self, amount: i64, cost: i64) -> i64 {
        cost * amount - cost - CALL_COST * amount - SUBR_OVERHEAD
    }
}

struct Market<'p> {
    p: &'p Params,
    /// Token id of every text position; separators are ids at or above `alphabet`.
    text: Vec<u32>,
    alphabet: u32,
    /// Bytes of each token id, in `arena`.
    spans: Vec<(u32, u32)>,
    arena: Vec<u8>,
    cost: Vec<u32>,
    depth: Vec<u16>,
    glyph_start: Vec<u32>,
    glyph_len: Vec<u32>,
    tree: suffix::Tree,
    cand_of: Vec<u32>,
    cands: Vec<Cand>,
    /// Nodes in order of length, so parents come first.
    order: Vec<u32>,
    /// Nearest strict ancestor that is a live candidate.
    up: Vec<u32>,
    /// Per text position, the tokens before the next stack-clearing one (dropped once the
    /// candidates are found).
    next_clear: Vec<u32>,
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

pub(super) fn subroutinize(flats: &[Flat], fd_of: &[usize], p: &Params) -> Result<Output, SubrError> {
    let mut m = Market::new(flats, p)?;
    m.find_candidates();
    let glyph_enc = m.run_rounds();
    m.finish(&glyph_enc, fd_of)
}

impl<'p> Market<'p> {
    fn new(flats: &[Flat], p: &'p Params) -> Result<Self, SubrError> {
        let total: usize = flats.iter().map(|f| f.tokens.len() + 1).sum();
        if total > MAX_TOKENS {
            return Err(SubrError::TooLarge { what: "charstring tokens to subroutinise", limit: MAX_TOKENS });
        }
        let mut ids: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut m = Market {
            p,
            text: Vec::with_capacity(total),
            alphabet: 0,
            spans: Vec::new(),
            arena: Vec::new(),
            cost: Vec::new(),
            depth: Vec::with_capacity(total),
            glyph_start: Vec::with_capacity(flats.len()),
            glyph_len: Vec::with_capacity(flats.len()),
            tree: suffix::Tree::default(),
            cand_of: Vec::new(),
            cands: Vec::new(),
            order: Vec::new(),
            up: Vec::new(),
            next_clear: Vec::new(),
        };
        let mut clears = Vec::with_capacity(total);
        for flat in flats {
            m.glyph_start.push(m.text.len() as u32);
            m.glyph_len.push(flat.tokens.len() as u32);
            for t in &flat.tokens {
                let bytes = flat.token_bytes(t);
                let blend_key;
                let key: &[u8] = match t.blend_vsindex {
                    Some(v) => {
                        blend_key = [16, (v >> 8) as u8, v as u8];
                        &blend_key
                    }
                    None => bytes,
                };
                let id = match ids.get(key) {
                    Some(&id) => id,
                    None => {
                        let id = m.spans.len() as u32;
                        m.spans.push((m.arena.len() as u32, bytes.len() as u32));
                        m.arena.extend_from_slice(bytes);
                        ids.insert(key.to_vec(), id);
                        id
                    }
                };
                m.text.push(id);
                m.depth.push(t.depth);
                clears.push(t.clears);
            }
            m.text.push(NONE);
            m.depth.push(0);
            clears.push(true);
        }
        m.alphabet = m.spans.len() as u32;
        let mut g = 0u32;
        for t in m.text.iter_mut().filter(|t| **t == NONE) {
            *t = m.alphabet + g;
            g += 1;
        }
        m.cost = m.text.iter().map(|&t| m.spans.get(t as usize).map_or(0, |s| s.1)).collect();

        // Per position: the largest depth up to and including the next stack-clearing token, and
        // the tokens before that token.
        let n = m.text.len();
        let mut entry = vec![0u16; n];
        let mut next_clear = vec![0u32; n];
        let (mut run_max, mut run_len) = (0u16, 0u32);
        for i in (0..n).rev() {
            let d = m.depth.get(i).copied().unwrap_or(0);
            if clears.get(i).copied().unwrap_or(true) {
                (run_max, run_len) = (d, 0);
            } else {
                (run_max, run_len) = (run_max.max(d), run_len + 1);
            }
            if let (Some(e), Some(c)) = (entry.get_mut(i), next_clear.get_mut(i)) {
                (*e, *c) = (run_max, run_len);
            }
        }
        m.tree = suffix::build(&m.text, (m.alphabet + g) as usize, &entry);
        m.cand_of = vec![NONE; m.tree.len.len()];
        m.up = vec![NONE; m.tree.len.len()];
        let mut order: Vec<u32> = (0..m.tree.len.len() as u32).collect();
        order.sort_by_key(|&node| m.tree.len.get(node as usize).copied().unwrap_or(0));
        m.order = order;
        m.next_clear = next_clear;
        Ok(m)
    }
}

impl Market<'_> {
    fn cand(&self, c: u32) -> Option<&Cand> {
        self.cands.get(c as usize)
    }

    fn alive(&self, node: u32) -> bool {
        let c = self.cand_of.get(node as usize).copied().unwrap_or(NONE);
        self.cand(c).is_some_and(|c| c.alive)
    }

    /// The deepest live candidate at or above `node`.
    fn first(&self, node: u32) -> u32 {
        if self.alive(node) { node } else { self.up.get(node as usize).copied().unwrap_or(NONE) }
    }

    fn refresh_up(&mut self) {
        for k in 0..self.order.len() {
            let node = self.order.get(k).copied().unwrap_or(0);
            let parent = self.tree.parent.get(node as usize).copied().unwrap_or(NONE);
            let up = if parent == NONE {
                NONE
            } else if self.alive(parent) {
                parent
            } else {
                self.up.get(parent as usize).copied().unwrap_or(NONE)
            };
            if let Some(slot) = self.up.get_mut(node as usize) {
                *slot = up;
            }
        }
    }

    /// Keeps the nodes whose estimated saving is positive, best first, within
    /// [`MAX_CANDIDATE_TOKENS`].
    fn find_candidates(&mut self) {
        let mut pre = Vec::with_capacity(self.cost.len() + 1);
        let mut sum = 0i64;
        pre.push(0);
        for &c in &self.cost {
            sum += i64::from(c);
            pre.push(sum);
        }
        let mut found: Vec<(i64, u32)> = Vec::new();
        for node in 1..self.tree.len.len() {
            let len = self.tree.len.get(node).copied().unwrap_or(0) as usize;
            let start = self.tree.start.get(node).copied().unwrap_or(0) as usize;
            let freq = i64::from(self.tree.freq.get(node).copied().unwrap_or(0));
            let cost = pre.get(start + len).copied().unwrap_or(0) - pre.get(start).copied().unwrap_or(0);
            let saving = cost * freq - cost - CALL_COST * freq - SUBR_OVERHEAD;
            if len > 0 && freq >= 2 && saving > 0 {
                found.push((saving, node as u32));
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut tokens = 0usize;
        for (_, node) in found {
            let n = node as usize;
            let len = self.tree.len.get(n).copied().unwrap_or(0);
            if tokens + len as usize > MAX_CANDIDATE_TOKENS {
                continue;
            }
            tokens += len as usize;
            let start = self.tree.start.get(n).copied().unwrap_or(0);
            let cost = pre.get(start as usize + len as usize).copied().unwrap_or(0) - pre.get(start as usize).copied().unwrap_or(0);
            let freq = i64::from(self.tree.freq.get(n).copied().unwrap_or(0));
            if let Some(slot) = self.cand_of.get_mut(n) {
                *slot = self.cands.len() as u32;
            }
            self.cands.push(Cand {
                len,
                start,
                cost,
                entry: self.tree.entry.get(n).copied().unwrap_or(0),
                first_clear: self.next_clear.get(start as usize).copied().unwrap_or(0),
                price: cost as f64,
                adjusted: cost as f64,
                usages: freq,
                enc: Vec::new(),
                alive: true,
                flatten: false,
                global: false,
                position: 0,
                call_bias: 0,
                program: Vec::new(),
            });
        }
        self.next_clear = Vec::new();
        self.refresh_up();
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
            let mut best = f64::from(self.cost.get(at).copied().unwrap_or(0)) + tail(j + 1);
            let mut pick = NONE;
            let depth = match site {
                Site::Subr { entry, first_clear } if j <= first_clear as usize => entry,
                _ => self.depth.get(at).copied().unwrap_or(u16::MAX),
            };
            // The call pushes the subroutine number.
            if usize::from(depth) < max_stack {
                let mut node = self.first(self.tree.deep.get(at).copied().unwrap_or(0));
                let mut seen = 0usize;
                while node != NONE && seen < MAX_CANDIDATES_PER_POSITION {
                    seen += 1;
                    let c = self.cand_of.get(node as usize).copied().unwrap_or(NONE);
                    if let Some(cand) = self.cand(c) {
                        let clen = cand.len as usize;
                        if c != skip && clen <= len - j {
                            let option = cand.price + tail(j + clen);
                            if option < best {
                                best = option;
                                pick = c;
                            }
                        }
                    }
                    node = self.up.get(node as usize).copied().unwrap_or(NONE);
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
        self.glyph_start.iter().zip(&self.glyph_len).map(|(&start, &len)| self.encode(start as usize, len as usize, NONE, Site::Glyph).1).collect()
    }

    fn live(&self) -> Vec<usize> {
        (0..self.cands.len()).filter(|&c| self.cands.get(c).is_some_and(|c| c.alive)).collect()
    }

    /// The rounds of the market; returns the glyph encodings of the last one.
    fn run_rounds(&mut self) -> Vec<Vec<Call>> {
        let mut glyph_enc = Vec::new();
        for round in 0..self.p.rounds {
            let live = self.live();
            for &c in &live {
                if let Some(cand) = self.cands.get_mut(c) {
                    let marginal = cand.adjusted / (cand.usages as f64 + K);
                    cand.price = marginal * ALPHA + cand.price * (1.0 - ALPHA);
                }
            }
            for &c in &live {
                let Some(cand) = self.cands.get(c) else { continue };
                let site = Site::Subr { entry: cand.entry, first_clear: cand.first_clear };
                let (cost, enc) = self.encode(cand.start as usize, cand.len as usize, c as u32, site);
                if let Some(cand) = self.cands.get_mut(c) {
                    cand.adjusted = cost;
                    cand.enc = enc;
                }
            }
            glyph_enc = self.encode_glyphs();

            let mut counts = vec![0i64; self.cands.len()];
            let calls = live.iter().filter_map(|&c| self.cands.get(c)).flat_map(|c| c.enc.iter()).chain(glyph_enc.iter().flatten());
            for &(_, e) in calls {
                if let Some(n) = counts.get_mut(e as usize) {
                    *n += 1;
                }
            }
            for (cand, n) in self.cands.iter_mut().zip(counts) {
                cand.usages = n;
            }

            if round + 2 <= self.p.rounds {
                let bad: Vec<usize> = live.iter().copied().filter(|&c| self.cands.get(c).is_some_and(|x| x.saving(x.usages, x.cost) <= 0)).collect();
                // compreffor's heuristic: what a dropped candidate called stays attractive.
                for &b in &bad {
                    let Some(cand) = self.cands.get(b) else { continue };
                    let (u, callees): (i64, Vec<u32>) = (cand.usages, cand.enc.iter().map(|e| e.1).collect());
                    for e in callees {
                        if let Some(callee) = self.cands.get_mut(e as usize) {
                            callee.usages += u - 1;
                        }
                    }
                }
                for &b in &bad {
                    if let Some(cand) = self.cands.get_mut(b) {
                        cand.alive = false;
                    }
                }
                self.refresh_up();
            }
        }
        glyph_enc
    }
}

/// What a call to a subroutine at the end of `list` would cost, by its usage rank (compreffor's
/// `test_call_cost`).
fn call_cost(usages: i64, list: &[usize], cands: &[Cand]) -> u8 {
    let used = |k: usize| list.get(k).and_then(|&c| cands.get(c)).map_or(i64::MIN, |c| c.usages);
    if list.len() >= 2263 && used(2262) >= usages {
        3
    } else if list.len() >= 215 && used(214) >= usages {
        2
    } else {
        1
    }
}

/// Inserts `c` into `list`, which is ordered by usages, most used first, after its equals.
fn insert_by_usage(c: usize, list: &mut Vec<usize>, cands: &[Cand]) {
    let u = cands.get(c).map_or(0, |x| x.usages);
    let at = list.partition_point(|&x| cands.get(x).map_or(0, |x| x.usages) >= u);
    list.insert(at, c);
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
            if let Some(cand) = self.cands.get(c as usize) {
                work.extend(cand.enc.iter().map(|&(_, e)| (e, new)));
            }
        }

        // Keep what pays for itself, counting the calls inside each candidate.
        let live = self.live();
        let mut by_len = live.clone();
        by_len.sort_by_key(|&c| self.cands.get(c).map_or(0, |x| x.len));
        let mut real = vec![0i64; n];
        let callee_cost = |cands: &[Cand], e: u32| cands.get(e as usize).map_or(0, |x| x.cost);
        for &c in &live {
            let Some(cand) = self.cands.get(c) else { continue };
            let r = cand.cost + cand.enc.iter().map(|&(_, e)| CALL_COST - callee_cost(&self.cands, e)).sum::<i64>();
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
            for &(_, e) in &cand.enc {
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
        let mut gsubrs: Vec<usize> = Vec::new();
        let mut lsubrs: Vec<Vec<usize>> = vec![Vec::new(); fd_count];
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
                    let global = call_cost(u, &gsubrs, &self.cands) < call_cost(u, local, &self.cands);
                    insert_by_usage(c, if global { &mut gsubrs } else { local }, &self.cands);
                    global
                } else if room_g {
                    insert_by_usage(c, &mut gsubrs, &self.cands);
                    true
                } else if room_l {
                    insert_by_usage(c, local, &self.cands);
                    false
                } else {
                    leftover.push(c);
                    continue;
                }
            } else if gsubrs.len() < limit {
                insert_by_usage(c, &mut gsubrs, &self.cands);
                true
            } else {
                leftover.push(c);
                continue;
            };
            if let Some(x) = self.cands.get_mut(c) {
                x.global = global;
            }
        }
        leftover.extend(subrs);
        for c in leftover {
            self.set_flatten(c);
        }

        // With several Font DICTs a global subroutine cannot know whose local ones to call.
        if fd_count > 1 {
            let mut seen = vec![false; n];
            let mut stack = gsubrs.clone();
            while let Some(s) = stack.pop() {
                let callees: Vec<u32> = self.cands.get(s).map(|x| x.enc.iter().map(|e| e.1).collect()).unwrap_or_default();
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
            let callees: Vec<u32> = self.cands.get(c).map(|x| x.enc.iter().map(|e| e.1).collect()).unwrap_or_default();
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
            list.sort_by(|&a, &b| self.usages(b).cmp(&self.usages(a)));
            *list = arrange(list);
            let b = bias(list.len());
            for (pos, &c) in list.iter().enumerate() {
                if let Some(x) = self.cands.get_mut(c) {
                    x.position = pos;
                    x.call_bias = b;
                }
            }
        }

        // Programs: flattened candidates shortest first (each may contain shorter ones), then
        // the subroutines.
        for &c in &by_len {
            if self.flattened(c as u32) && state.get(c).is_some_and(|&s| s != NONE) {
                let program = self.program(c)?;
                if let Some(x) = self.cands.get_mut(c) {
                    x.program = program;
                }
            }
        }
        for &c in lists.iter().flatten() {
            let mut program = self.program(c)?;
            let last = self.cands.get(c).and_then(|x| self.text.get(((x.start + x.len) as usize).saturating_sub(1))).copied();
            let ends = last.and_then(|t| self.token(t)) == Some(&[14][..]);
            if !self.p.cff2 && !ends {
                program.push(11);
            }
            if program.len() > super::MAX_CHARSTRING {
                return Err(SubrError::TooLarge { what: "bytes in one subroutine", limit: super::MAX_CHARSTRING });
            }
            if let Some(x) = self.cands.get_mut(c) {
                x.program = program;
            }
        }
        let mut charstrings = Vec::with_capacity(glyph_enc.len());
        for (g, enc) in glyph_enc.iter().enumerate() {
            let start = self.glyph_start.get(g).copied().unwrap_or(0) as usize;
            let len = self.glyph_len.get(g).copied().unwrap_or(0) as usize;
            charstrings.push(self.write(start, len, enc)?);
        }
        let mut take = |c: &usize| self.cands.get_mut(*c).map(|x| std::mem::take(&mut x.program)).unwrap_or_default();
        let mut lists = lists.into_iter();
        let global = lists.next().unwrap_or_default().iter().map(&mut take).collect();
        let local = lists.map(|l| l.iter().map(&mut take).collect()).collect();
        Ok(Output { charstrings, global, local })
    }

    fn token(&self, id: u32) -> Option<&[u8]> {
        let &(start, len) = self.spans.get(id as usize)?;
        self.arena.get(start as usize..(start + len) as usize)
    }

    /// The body of candidate `c` with its encoding applied.
    fn program(&self, c: usize) -> Result<Vec<u8>, SubrError> {
        let cand = self.cands.get(c).ok_or_else(|| SubrError::Invalid("a lost candidate".into()))?;
        self.write(cand.start as usize, cand.len as usize, &cand.enc)
    }

    /// The `len` tokens at `from` with the calls of `enc`: a flattened candidate is written out,
    /// a subroutine is called.
    fn write(&self, from: usize, len: usize, enc: &[Call]) -> Result<Vec<u8>, SubrError> {
        let mut out = Vec::new();
        let mut calls = enc.iter().peekable();
        let mut j = 0usize;
        while j < len {
            if let Some(&&(at, c)) = calls.peek() {
                if at as usize == j {
                    calls.next();
                    let cand = self.cand(c).ok_or_else(|| SubrError::Invalid("a call to a lost candidate".into()))?;
                    if cand.flatten {
                        out.extend_from_slice(&cand.program);
                    } else {
                        push_int(cand.position as i64 - cand.call_bias, &mut out)?;
                        out.push(if cand.global { 29 } else { 10 });
                    }
                    j += (cand.len as usize).max(1);
                    continue;
                }
            }
            let id = self.text.get(from + j).copied().unwrap_or(NONE);
            out.extend_from_slice(self.token(id).unwrap_or_default());
            j += 1;
        }
        Ok(out)
    }
}
