//! Reading the glyphs into one text of token ids, and finding the candidate subroutines in it.
//!
//! The work runs in phases so that only a few arrays of one entry per token are held at once:
//! the text (7 bytes per token), then the suffix array with its working arrays (16), then the
//! suffix and LCP arrays (8) walked twice. The first walk prices every repeated sequence and
//! keeps those that save bytes; the second records, for the chosen ones only, what the market
//! needs, and links every text position and every candidate to the nearest chosen sequence
//! above it in the tree. The tree itself is never stored.

use std::collections::HashMap;

use super::flat::Flat;
use super::market::{CALL_COST, Cand, SUBR_OVERHEAD};
use super::memory::Memory;
use super::suffix::{self, NONE, Node, Visit};
use super::{MAX_CANDIDATE_TOKENS, MAX_TOKENS, SubrError};

/// Bytes charged for each distinct token's entry in the lookup used while reading.
const KEY_BYTES: usize = 64;
/// Marks a link that names a chosen candidate instead of a parent node.
const CHOSEN: u32 = 1 << 31;

/// Every glyph's tokens in one text, each glyph followed by a separator of its own.
#[derive(Default)]
pub(super) struct Text {
    /// Token id of every position; separators are ids at or above `alphabet`.
    pub text: Vec<u32>,
    pub alphabet: u32,
    /// Bytes of each token id, in `arena`.
    pub spans: Vec<(u32, u32)>,
    pub arena: Vec<u8>,
    /// Operands on the stack before each token.
    pub depth: Vec<u16>,
    /// Whether each token leaves the stack empty (separators do); emptied once the candidates
    /// are found.
    pub clears: Vec<bool>,
    pub glyph_start: Vec<u32>,
    pub glyph_len: Vec<u32>,
}

impl Text {
    /// Bytes of the token at text position `at` (0 for a separator).
    pub fn cost(&self, at: usize) -> u32 {
        self.text.get(at).and_then(|&t| self.spans.get(t as usize)).map_or(0, |s| s.1)
    }

    pub fn token(&self, id: u32) -> Option<&[u8]> {
        let &(start, len) = self.spans.get(id as usize)?;
        self.arena.get(start as usize..(start + len) as usize)
    }

    /// Bytes held by the arrays that grow with the text.
    fn held(&self) -> usize {
        self.text.capacity() * 4
            + self.depth.capacity() * 2
            + self.clears.capacity()
            + (self.glyph_start.capacity() + self.glyph_len.capacity()) * 4
            + self.spans.capacity() * 8
            + self.arena.capacity()
    }
}

/// Reads glyphs `0..glyphs`, each produced by `next`, into one text. A glyph is dropped as soon
/// as it is read.
pub(super) fn read(glyphs: usize, mut next: impl FnMut(usize) -> Result<Flat, SubrError>, mem: &mut Memory) -> Result<Text, SubrError> {
    let mut ids: HashMap<Vec<u8>, u32> = HashMap::new();
    let mut t = Text::default();
    let mut held = 0usize;
    for gid in 0..glyphs {
        let flat = next(gid)?;
        let total = t.text.len().saturating_add(flat.tokens.len()).saturating_add(1);
        if total > MAX_TOKENS {
            return Err(SubrError::TooLarge { what: "charstring tokens to subroutinise", limit: MAX_TOKENS });
        }
        t.glyph_start.push(t.text.len() as u32);
        t.glyph_len.push(flat.tokens.len() as u32);
        for tok in &flat.tokens {
            let bytes = flat.token_bytes(tok);
            let blend_key;
            let key: &[u8] = match tok.blend_vsindex {
                Some(v) => {
                    blend_key = [16, (v >> 8) as u8, v as u8];
                    &blend_key
                }
                None => bytes,
            };
            let id = match ids.get(key) {
                Some(&id) => id,
                None => {
                    mem.charge(KEY_BYTES + key.len())?;
                    let id = t.spans.len() as u32;
                    t.spans.push((t.arena.len() as u32, bytes.len() as u32));
                    t.arena.extend_from_slice(bytes);
                    ids.insert(key.to_vec(), id);
                    id
                }
            };
            t.text.push(id);
            t.depth.push(tok.depth);
            t.clears.push(tok.clears);
        }
        t.text.push(NONE);
        t.depth.push(0);
        t.clears.push(true);
        let now = t.held();
        if now > held {
            mem.charge(now - held)?;
            held = now;
        }
    }
    let keys: usize = ids.keys().map(|k| KEY_BYTES + k.len()).sum();
    drop(ids);
    mem.release(keys);
    t.text.shrink_to_fit();
    t.depth.shrink_to_fit();
    t.clears.shrink_to_fit();
    mem.release(held.saturating_sub(t.held()));
    t.alphabet = t.spans.len() as u32;
    let base = t.alphabet;
    for (id, g) in t.text.iter_mut().filter(|id| **id == NONE).zip(0u32..) {
        *id = base + g;
    }
    Ok(t)
}

/// A repeated sequence that saves bytes, found by the first walk.
struct Saving {
    saving: i64,
    node: u32,
    len: u32,
}

/// The bytes of the `len` tokens at `start`, from the prefix sums `pre` (kept modulo 2^32: a
/// sequence lies within one glyph, so its bytes are far fewer).
fn seq_cost(pre: &[u32], start: u32, len: u32) -> i64 {
    let at = |k: usize| pre.get(k).copied().unwrap_or(0);
    i64::from(at(start as usize + len as usize).wrapping_sub(at(start as usize)))
}

/// First walk: every sequence whose estimated saving is positive.
struct Savings<'a> {
    pre: &'a [u32],
    found: Vec<Saving>,
    mem: &'a mut Memory,
}

impl Visit for Savings<'_> {
    fn open(&mut self, _: u32) -> Result<(), SubrError> {
        Ok(())
    }

    fn close(&mut self, node: &Node) -> Result<(), SubrError> {
        let cost = seq_cost(self.pre, node.start, node.len);
        let freq = i64::from(node.freq);
        let saving = cost * freq - cost - CALL_COST * freq - SUBR_OVERHEAD;
        if node.len > 0 && freq >= 2 && saving > 0 {
            self.mem.charge_for::<Saving>(1)?;
            self.found.push(Saving { saving, node: node.id, len: node.len });
        }
        Ok(())
    }

    fn parent(&mut self, _: u32, _: u32) {}

    fn deepest(&mut self, _: u32, _: u32) {}
}

/// Second walk: the chosen candidates' details, and links up the tree.
struct Shape<'a> {
    /// (node, candidate) of every chosen node, by node.
    chosen: &'a [(u32, u32)],
    next: usize,
    /// Per node: `CHOSEN | candidate` for a chosen node, else its parent node; [`NONE`] for the
    /// root.
    link: Vec<u32>,
    /// Per candidate: its parent node.
    parent_node: Vec<u32>,
    /// Per text position: the deepest node starting there.
    deep: Vec<u32>,
    cands: &'a mut [Cand],
    pre: &'a [u32],
    mem: &'a mut Memory,
}

impl Shape<'_> {
    fn chosen(&self, node: u32) -> Option<usize> {
        let v = self.link.get(node as usize).copied().unwrap_or(NONE);
        (v != NONE && v & CHOSEN != 0).then_some((v & !CHOSEN) as usize)
    }
}

impl Visit for Shape<'_> {
    fn open(&mut self, id: u32) -> Result<(), SubrError> {
        self.mem.charge_for::<u32>(1)?;
        let link = match self.chosen.get(self.next) {
            Some(&(node, c)) if node == id => {
                self.next += 1;
                CHOSEN | c
            }
            // Replaced by the parent once it is known.
            _ => 0,
        };
        self.link.push(link);
        Ok(())
    }

    fn close(&mut self, node: &Node) -> Result<(), SubrError> {
        let Some(c) = self.chosen(node.id) else { return Ok(()) };
        let cost = seq_cost(self.pre, node.start, node.len);
        if let Some(cand) = self.cands.get_mut(c) {
            cand.start = node.start;
            cand.cost = cost;
            cand.entry = node.entry;
            cand.usages = i64::from(node.freq);
            cand.price = cost as f64;
            cand.adjusted = cost as f64;
        }
        Ok(())
    }

    fn parent(&mut self, child: u32, parent: u32) {
        match self.chosen(child) {
            Some(c) => {
                if let Some(slot) = self.parent_node.get_mut(c) {
                    *slot = parent;
                }
            }
            None => {
                if let Some(slot) = self.link.get_mut(child as usize) {
                    *slot = parent;
                }
            }
        }
    }

    fn deepest(&mut self, pos: u32, node: u32) {
        if let Some(slot) = self.deep.get_mut(pos as usize) {
            *slot = node;
        }
    }
}

/// The nearest chosen node at or above `x`, as a candidate ([`NONE`] for none). The links
/// passed on the way are pointed at the answer, so later lookups are short.
fn nearest(link: &mut [u32], x: u32) -> u32 {
    let mut r = x;
    let mut found = NONE;
    // Links lead strictly up the tree, so the walk ends within one step per node.
    for _ in 0..=link.len() {
        let v = link.get(r as usize).copied().unwrap_or(NONE);
        if v == NONE {
            break;
        }
        if v & CHOSEN != 0 {
            found = v & !CHOSEN;
            break;
        }
        r = v;
    }
    let mut y = x;
    for _ in 0..=link.len() {
        if y == r {
            break;
        }
        let Some(slot) = link.get_mut(y as usize) else { break };
        let up = *slot;
        if up == NONE || up & CHOSEN != 0 {
            break;
        }
        *slot = r;
        y = up;
    }
    found
}

/// The candidates of `t`, numbered by estimated saving, best first, with each one's nearest
/// candidate ancestor in [`Cand::parent`]; and per text position the deepest candidate starting
/// there ([`NONE`] for none). Empties `t.clears`.
pub(super) fn find(t: &mut Text, mem: &mut Memory) -> Result<(Vec<Cand>, Vec<u32>), SubrError> {
    let n = t.text.len();
    let symbols = t.alphabet as usize + t.glyph_start.len();

    // The suffix array takes `sa`, the ranks, one more array of `n` and the counts.
    let sa_work = 3 * n + symbols.max(n) + 1;
    mem.charge_for::<u32>(sa_work)?;
    let (sa, rank) = suffix::suffix_array(&t.text, symbols);
    mem.release_for::<u32>(sa_work - 2 * n);
    mem.charge_for::<u32>(n)?;
    let lcp = suffix::lcp_array(&t.text, &sa, &rank);
    drop(rank);
    mem.release_for::<u32>(n);

    // Per position: the largest depth up to and including the next stack-clearing token.
    mem.charge_for::<u16>(n)?;
    let mut entry = vec![0u16; n];
    let mut run_max = 0u16;
    for i in (0..n).rev() {
        let d = t.depth.get(i).copied().unwrap_or(0);
        run_max = if t.clears.get(i).copied().unwrap_or(true) { d } else { run_max.max(d) };
        if let Some(e) = entry.get_mut(i) {
            *e = run_max;
        }
    }
    mem.charge_for::<u32>(n + 1)?;
    let mut pre = Vec::with_capacity(n + 1);
    let mut sum = 0u32;
    pre.push(sum);
    for at in 0..n {
        sum = sum.wrapping_add(t.cost(at));
        pre.push(sum);
    }

    let mut savings = Savings { pre: &pre, found: Vec::new(), mem: &mut *mem };
    suffix::walk(&sa, &lcp, &entry, &mut savings)?;
    let mut found = savings.found;
    found.sort_unstable_by(|a, b| b.saving.cmp(&a.saving).then(a.node.cmp(&b.node)));

    // Best first, within the token limit; the limit grows with the text so that the work stays
    // proportional to it.
    let cap = MAX_CANDIDATE_TOKENS.max(n);
    let fits = |tokens: &mut usize, len: u32| {
        let ok = *tokens + len as usize <= cap;
        if ok {
            *tokens += len as usize;
        }
        ok
    };
    let mut tokens = 0usize;
    let count = found.iter().filter(|f| fits(&mut tokens, f.len)).count();
    mem.charge_for::<Cand>(count)?;
    mem.charge_for::<(u32, u32)>(count)?;
    let mut cands = Vec::with_capacity(count);
    let mut chosen = Vec::with_capacity(count);
    tokens = 0;
    for f in &found {
        if fits(&mut tokens, f.len) {
            chosen.push((f.node, cands.len() as u32));
            cands.push(Cand::new(f.len));
        }
    }
    mem.release_for::<Saving>(found.len());
    drop(found);
    chosen.sort_unstable();

    mem.charge_for::<u32>(n + count + 1)?;
    let mut shape = Shape {
        chosen: &chosen,
        next: 0,
        link: vec![NONE],
        parent_node: vec![0; count],
        deep: vec![0; n],
        cands: &mut cands,
        pre: &pre,
        mem: &mut *mem,
    };
    suffix::walk(&sa, &lcp, &entry, &mut shape)?;
    let Shape { mut link, parent_node, mut deep, .. } = shape;
    let nodes = link.len();
    drop((sa, lcp, entry, pre));
    mem.release_for::<u32>(3 * n + 1);
    mem.release_for::<u16>(n);
    drop(chosen);
    mem.release_for::<(u32, u32)>(count);

    for (cand, &p) in cands.iter_mut().zip(&parent_node) {
        cand.parent = nearest(&mut link, p);
    }
    for d in deep.iter_mut() {
        *d = nearest(&mut link, *d);
    }
    drop((link, parent_node));
    mem.release_for::<u32>(nodes + count);

    // Tokens before the first stack-clearing one, counted only as far as the candidate reaches:
    // beyond its end the count makes no difference.
    for cand in &mut cands {
        let from = cand.start as usize;
        let clears = t.clears.get(from..from + cand.len as usize).unwrap_or_default();
        cand.first_clear = clears.iter().take_while(|&&c| !c).count() as u32;
    }
    mem.release(t.clears.capacity());
    t.clears = Vec::new();
    Ok((cands, deep))
}
