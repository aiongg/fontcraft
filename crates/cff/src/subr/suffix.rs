//! The repeated token sequences of a text: suffix array, LCP array and a walk over the tree of
//! LCP intervals.
//!
//! The text is every glyph's tokens followed by a separator of its own, so no repeat crosses a
//! glyph. Each internal node of the tree is a token sequence that occurs at least twice and is
//! followed by different tokens in different places; its occurrences are the suffixes of its
//! interval. The suffix array is built by prefix doubling with counting sorts (`O(n log n)`; the
//! number of rounds is the logarithm of the longest repeat), the LCP array by Kasai's method, the
//! intervals by the usual bottom-up stack traversal.
//!
//! The tree is not stored: [`walk`] reports each node as it opens and closes, its final parent
//! and the deepest node at each text position, and the caller keeps only what it needs. Every
//! array here holds 4 bytes per token.

use super::SubrError;

/// "No node".
pub(super) const NONE: u32 = u32::MAX;

/// A node of the tree, reported once its interval is complete. Node 0 is the root (the empty
/// sequence) and is never reported; the others are numbered from 1 in the order they open.
#[derive(Clone, Copy, Debug)]
pub(super) struct Node {
    pub id: u32,
    /// Length of the node's sequence, in tokens.
    pub len: u32,
    /// A text position where the sequence starts.
    pub start: u32,
    /// Occurrences of the sequence, overlapping ones included.
    pub freq: u32,
    /// The largest `entry` value at the start of any occurrence.
    pub entry: u16,
}

/// What [`walk`] reports.
pub(super) trait Visit {
    /// Node `id` opens; ids count up by one from 1.
    fn open(&mut self, id: u32) -> Result<(), SubrError>;
    /// `node` closes: everything in it is final.
    fn close(&mut self, node: &Node) -> Result<(), SubrError>;
    /// The enclosing node of `child` is `parent` (0 for the root). Reported once for every node,
    /// after it closes.
    fn parent(&mut self, child: u32, parent: u32);
    /// The deepest node whose sequence starts at text position `pos` (0 for none).
    fn deepest(&mut self, pos: u32, node: u32);
}

fn get(v: &[u32], i: usize) -> u32 {
    v.get(i).copied().unwrap_or(0)
}

fn put(v: &mut [u32], i: usize, x: u32) {
    if let Some(s) = v.get_mut(i) {
        *s = x;
    }
}

/// Suffix array of `s` and its inverse, the rank of every suffix. Every suffix meets a symbol
/// that occurs only once before the end of the text (the separators), so sorting the cyclic
/// rotations sorts the suffixes. Takes four arrays of `n` entries and one of `max(alphabet, n)`;
/// the caller keeps `s.len()` below `u32::MAX`.
pub(super) fn suffix_array(s: &[u32], alphabet: usize) -> (Vec<u32>, Vec<u32>) {
    let n = s.len();
    let mut sa = vec![0u32; n];
    let mut class = vec![0u32; n];
    if n == 0 {
        return (sa, class);
    }
    let mut cnt = vec![0u32; alphabet.max(n) + 1];
    for &c in s {
        if let Some(x) = cnt.get_mut(c as usize) {
            *x += 1;
        }
    }
    let mut run = 0u32;
    for x in cnt.iter_mut() {
        run += *x;
        *x = run;
    }
    for i in (0..n).rev() {
        let c = get(s, i) as usize;
        let k = get(&cnt, c).saturating_sub(1);
        put(&mut cnt, c, k);
        put(&mut sa, k as usize, i as u32);
    }
    let mut classes = 1usize;
    for i in 1..n {
        if get(s, get(&sa, i) as usize) != get(s, get(&sa, i - 1) as usize) {
            classes += 1;
        }
        put(&mut class, get(&sa, i) as usize, (classes - 1) as u32);
    }
    // Each round sorts by the second half into `sa` through `pn`, then writes the new classes
    // into `pn`, which is free again, and swaps it with `class`.
    let mut pn = vec![0u32; n];
    let mut h = 1usize;
    while classes < n && h < n {
        for (slot, &p) in pn.iter_mut().zip(&sa) {
            *slot = ((p as usize + n - h) % n) as u32;
        }
        for x in cnt.iter_mut().take(classes) {
            *x = 0;
        }
        for &p in &pn {
            if let Some(x) = cnt.get_mut(get(&class, p as usize) as usize) {
                *x += 1;
            }
        }
        let mut run = 0u32;
        for x in cnt.iter_mut().take(classes) {
            run += *x;
            *x = run;
        }
        for i in (0..n).rev() {
            let p = get(&pn, i);
            let c = get(&class, p as usize) as usize;
            let k = get(&cnt, c).saturating_sub(1);
            put(&mut cnt, c, k);
            put(&mut sa, k as usize, p);
        }
        let key = |class: &[u32], p: u32| (get(class, p as usize), get(class, (p as usize + h) % n));
        put(&mut pn, get(&sa, 0) as usize, 0);
        classes = 1;
        for i in 1..n {
            if key(&class, get(&sa, i)) != key(&class, get(&sa, i - 1)) {
                classes += 1;
            }
            put(&mut pn, get(&sa, i) as usize, (classes - 1) as u32);
        }
        std::mem::swap(&mut class, &mut pn);
        h *= 2;
    }
    drop(pn);
    drop(cnt);
    for (r, &p) in sa.iter().enumerate() {
        put(&mut class, p as usize, r as u32);
    }
    (sa, class)
}

/// `lcp[r]`: the length of the common prefix of suffixes `sa[r - 1]` and `sa[r]` (0 for `r = 0`).
/// `rank` is the inverse of `sa`.
pub(super) fn lcp_array(s: &[u32], sa: &[u32], rank: &[u32]) -> Vec<u32> {
    let n = s.len();
    let mut lcp = vec![0u32; n];
    let mut h = 0usize;
    for i in 0..n {
        let r = get(rank, i) as usize;
        if r == 0 {
            h = 0;
            continue;
        }
        let j = get(sa, r - 1) as usize;
        while i + h < n && j + h < n && s.get(i + h) == s.get(j + h) {
            h += 1;
        }
        put(&mut lcp, r, h as u32);
        h = h.saturating_sub(1);
    }
    lcp
}

/// An interval on the traversal stack.
struct Open {
    id: u32,
    lb: usize,
    len: u32,
    start: u32,
    entry: u16,
}

/// Walks the tree of LCP intervals of the suffix array `sa` with its `lcp`, reporting to `v`.
/// `entry` is given per text position; a node's entry is the largest at its occurrences.
pub(super) fn walk(sa: &[u32], lcp: &[u32], entry: &[u16], v: &mut impl Visit) -> Result<(), SubrError> {
    let n = sa.len();
    let mut stack = vec![Open { id: 0, lb: 0, len: 0, start: 0, entry: 0 }];
    let mut next_id = 1u32;
    for i in 1..=n {
        let l = if i < n { get(lcp, i) } else { 0 };
        let leaf = get(sa, i - 1);
        let leaf_entry = entry.get(leaf as usize).copied().unwrap_or(0);
        let (left_top, left_len) = match stack.last_mut() {
            Some(top) => {
                top.entry = top.entry.max(leaf_entry);
                (top.id, top.len)
            }
            None => (0, 0),
        };
        let mut lb = i - 1;
        // The node popped last, with its entry: its parent is known only after the loop.
        let mut popped: Option<(u32, u16)> = None;
        while stack.last().is_some_and(|top| top.len > l && top.id != 0) {
            let Some(top) = stack.pop() else { break };
            v.close(&Node { id: top.id, len: top.len, start: top.start, freq: (i - top.lb) as u32, entry: top.entry })?;
            if let Some(below) = stack.last_mut() {
                below.entry = below.entry.max(top.entry);
            }
            if let Some((child, _)) = popped {
                v.parent(child, top.id);
            }
            lb = top.lb;
            popped = Some((top.id, top.entry));
        }
        let (top_id, top_len) = stack.last().map_or((0, 0), |s| (s.id, s.len));
        if l > top_len {
            let id = next_id;
            next_id += 1;
            v.open(id)?;
            let mut e = 0;
            if let Some((child, child_entry)) = popped {
                v.parent(child, id);
                e = child_entry;
            }
            stack.push(Open { id, lb, len: l, start: get(sa, lb), entry: e });
        } else if let Some((child, _)) = popped {
            v.parent(child, top_id);
        }
        let (right_top, right_len) = match stack.last_mut() {
            Some(top) => {
                top.entry = top.entry.max(leaf_entry);
                (top.id, top.len)
            }
            None => (0, 0),
        };
        v.deepest(leaf, if left_len >= right_len { left_top } else { right_top });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every repeated, right-branching substring of a small text, by brute force.
    fn brute(text: &[u32]) -> Vec<(Vec<u32>, u32)> {
        let n = text.len();
        let mut found = std::collections::BTreeMap::new();
        for len in 1..n {
            for a in 0..=n - len {
                let s = &text[a..a + len];
                let occ: Vec<usize> = (0..=n - len).filter(|&b| &text[b..b + len] == s).collect();
                let followers: std::collections::BTreeSet<Option<u32>> = occ.iter().map(|&b| text.get(b + len).copied()).collect();
                if occ.len() >= 2 && followers.len() >= 2 {
                    found.insert(s.to_vec(), occ.len() as u32);
                }
            }
        }
        found.into_iter().collect()
    }

    /// The whole tree, indexed by node id, as the walk reports it.
    #[derive(Default)]
    struct Tree {
        len: Vec<u32>,
        start: Vec<u32>,
        freq: Vec<u32>,
        parent: Vec<u32>,
        entry: Vec<u16>,
        deep: Vec<u32>,
        opened: u32,
    }

    impl Tree {
        fn of(text: &[u32], alphabet: usize, entry: &[u16]) -> Tree {
            let (sa, rank) = suffix_array(text, alphabet);
            let lcp = lcp_array(text, &sa, &rank);
            let mut t = Tree { deep: vec![NONE; text.len()], ..Tree::default() };
            walk(&sa, &lcp, entry, &mut t).unwrap();
            assert!(t.deep.iter().all(|&d| d != NONE), "every position gets its deepest node");
            t
        }

        fn grow(&mut self, id: u32) {
            let n = id as usize + 1;
            if self.len.len() < n {
                self.len.resize(n, 0);
                self.start.resize(n, 0);
                self.freq.resize(n, 0);
                self.parent.resize(n, NONE);
                self.entry.resize(n, 0);
            }
        }
    }

    impl Visit for Tree {
        fn open(&mut self, id: u32) -> Result<(), SubrError> {
            assert_eq!(id, self.opened + 1, "ids count up by one");
            self.opened = id;
            self.grow(id);
            Ok(())
        }
        fn close(&mut self, node: &Node) -> Result<(), SubrError> {
            let k = node.id as usize;
            assert!(node.id <= self.opened, "a node closes after it opens");
            (self.len[k], self.start[k], self.freq[k], self.entry[k]) = (node.len, node.start, node.freq, node.entry);
            Ok(())
        }
        fn parent(&mut self, child: u32, parent: u32) {
            assert_eq!(self.parent[child as usize], NONE, "one parent per node");
            self.parent[child as usize] = parent;
        }
        fn deepest(&mut self, pos: u32, node: u32) {
            self.deep[pos as usize] = node;
        }
    }

    #[test]
    fn the_tree_lists_every_branching_repeat_with_its_count() {
        // Three "glyphs" separated by unique symbols 10, 11, 12.
        let text = [1, 2, 3, 1, 2, 10, 2, 3, 1, 2, 3, 11, 1, 2, 1, 2, 12];
        let entry: Vec<u16> = (0..text.len() as u16).collect();
        let t = Tree::of(&text, 13, &entry);
        let mut got: Vec<(Vec<u32>, u32)> = (1..t.len.len())
            .map(|node| {
                let (s, l) = (t.start[node] as usize, t.len[node] as usize);
                (text[s..s + l].to_vec(), t.freq[node])
            })
            .collect();
        got.sort();
        assert_eq!(got, brute(&text));
        // Each node's entry is the largest over the starts of its occurrences.
        for node in 1..t.len.len() {
            let (s, l) = (t.start[node] as usize, t.len[node] as usize);
            let max = (0..=text.len() - l).filter(|&b| text[b..b + l] == text[s..s + l]).map(|b| entry[b]).max().unwrap();
            assert_eq!(t.entry[node], max, "node {node}");
        }
        // A node's parent is a shorter prefix of it that occurs at least as often.
        for node in 1..t.len.len() {
            let p = t.parent[node] as usize;
            let (s, l, ps, pl) = (t.start[node] as usize, t.len[node] as usize, t.start[p] as usize, t.len[p] as usize);
            assert!(pl < l && text[s..s + pl] == text[ps..ps + pl] && (p == 0 || t.freq[p] >= t.freq[node]), "node {node}");
        }
        // The deepest node at a position is its longest repeat starting there.
        for p in 0..text.len() {
            let node = t.deep[p] as usize;
            let l = t.len[node] as usize;
            if l > 0 {
                let s = t.start[node] as usize;
                assert_eq!(text[p..p + l], text[s..s + l]);
            }
        }
    }
}
