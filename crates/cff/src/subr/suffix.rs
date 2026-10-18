//! The repeated token sequences of a text: suffix array, LCP array and the tree of LCP intervals.
//!
//! The text is every glyph's tokens followed by a separator of its own, so no repeat crosses a
//! glyph. Each internal node of the tree is a token sequence that occurs at least twice and is
//! followed by different tokens in different places; its occurrences are the suffixes of its
//! interval. The suffix array is built by prefix doubling with counting sorts (`O(n log n)`; the
//! number of rounds is the logarithm of the longest repeat), the LCP array by Kasai's method, the
//! intervals by the usual bottom-up stack traversal.

/// "No node".
pub(super) const NONE: u32 = u32::MAX;

/// The tree of repeated sequences. Node 0 is the root (the empty sequence).
#[derive(Debug, Default)]
pub(super) struct Tree {
    /// Length of the node's sequence, in tokens.
    pub len: Vec<u32>,
    /// A text position where the sequence starts.
    pub start: Vec<u32>,
    /// Occurrences of the sequence, overlapping ones included.
    pub freq: Vec<u32>,
    /// The enclosing node; [`NONE`] for the root.
    pub parent: Vec<u32>,
    /// The largest `entry` value at the start of any occurrence.
    pub entry: Vec<u16>,
    /// For every text position, the deepest node whose sequence starts there (0 for none).
    pub deep: Vec<u32>,
}

fn get(v: &[u32], i: usize) -> u32 {
    v.get(i).copied().unwrap_or(0)
}

fn put(v: &mut [u32], i: usize, x: u32) {
    if let Some(s) = v.get_mut(i) {
        *s = x;
    }
}

/// The tree for `text`, whose symbols are below `alphabet`, with `entry` given per position.
/// The caller keeps `text.len()` below `u32::MAX`.
pub(super) fn build(text: &[u32], alphabet: usize, entry: &[u16]) -> Tree {
    let n = text.len();
    if n == 0 {
        return Tree { len: vec![0], start: vec![0], freq: vec![0], parent: vec![NONE], entry: vec![0], deep: Vec::new() };
    }
    let sa = suffix_array(text, alphabet);
    let lcp = lcp_array(text, &sa);
    intervals(&sa, &lcp, entry)
}

/// Suffix array of `s`. Every suffix meets a symbol that occurs only once before the end of the
/// text (the separators), so sorting the cyclic rotations sorts the suffixes.
fn suffix_array(s: &[u32], alphabet: usize) -> Vec<u32> {
    let n = s.len();
    let mut sa = vec![0u32; n];
    let mut class = vec![0u32; n];
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
    let mut pn = vec![0u32; n];
    let mut cn = vec![0u32; n];
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
        put(&mut cn, get(&sa, 0) as usize, 0);
        classes = 1;
        for i in 1..n {
            if key(&class, get(&sa, i)) != key(&class, get(&sa, i - 1)) {
                classes += 1;
            }
            put(&mut cn, get(&sa, i) as usize, (classes - 1) as u32);
        }
        std::mem::swap(&mut class, &mut cn);
        h *= 2;
    }
    sa
}

/// `lcp[r]`: the length of the common prefix of suffixes `sa[r - 1]` and `sa[r]` (0 for `r = 0`).
fn lcp_array(s: &[u32], sa: &[u32]) -> Vec<u32> {
    let n = s.len();
    let mut rank = vec![0u32; n];
    for (r, &p) in sa.iter().enumerate() {
        put(&mut rank, p as usize, r as u32);
    }
    let mut lcp = vec![0u32; n];
    let mut h = 0usize;
    for i in 0..n {
        let r = get(&rank, i) as usize;
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

fn intervals(sa: &[u32], lcp: &[u32], entry: &[u16]) -> Tree {
    let n = sa.len();
    let mut t = Tree { len: vec![0], start: vec![0], freq: vec![n as u32], parent: vec![NONE], entry: vec![0], deep: vec![0; n] };
    let bump = |t: &mut Tree, node: u32, v: u16| {
        if let Some(e) = t.entry.get_mut(node as usize) {
            *e = (*e).max(v);
        }
    };
    let node_len = |t: &Tree, node: u32| t.len.get(node as usize).copied().unwrap_or(0);
    // (node, left bound of its interval)
    let mut stack: Vec<(u32, usize)> = vec![(0, 0)];
    for i in 1..=n {
        let l = if i < n { get(lcp, i) } else { 0 };
        let leaf = get(sa, i - 1) as usize;
        let leaf_entry = entry.get(leaf).copied().unwrap_or(0);
        let left_top = stack.last().map_or(0, |s| s.0);
        bump(&mut t, left_top, leaf_entry);
        let mut lb = i - 1;
        let mut last_popped = NONE;
        while let Some(&(top, top_lb)) = stack.last() {
            if node_len(&t, top) <= l || top == 0 {
                break;
            }
            stack.pop();
            let below = stack.last().map_or(0, |s| s.0);
            put(&mut t.freq, top as usize, (i - top_lb) as u32);
            put(&mut t.parent, top as usize, below);
            let e = t.entry.get(top as usize).copied().unwrap_or(0);
            bump(&mut t, below, e);
            lb = top_lb;
            last_popped = top;
        }
        let top = stack.last().map_or(0, |s| s.0);
        if l > node_len(&t, top) {
            let id = t.len.len() as u32;
            t.len.push(l);
            t.start.push(get(sa, lb));
            t.freq.push(0);
            t.parent.push(top);
            t.entry.push(0);
            if last_popped != NONE {
                put(&mut t.parent, last_popped as usize, id);
                let e = t.entry.get(last_popped as usize).copied().unwrap_or(0);
                bump(&mut t, id, e);
            }
            stack.push((id, lb));
        }
        let right_top = stack.last().map_or(0, |s| s.0);
        bump(&mut t, right_top, leaf_entry);
        let deepest = if node_len(&t, left_top) >= node_len(&t, right_top) { left_top } else { right_top };
        put(&mut t.deep, leaf, deepest);
    }
    t
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

    #[test]
    fn the_tree_lists_every_branching_repeat_with_its_count() {
        // Three "glyphs" separated by unique symbols 10, 11, 12.
        let text = [1, 2, 3, 1, 2, 10, 2, 3, 1, 2, 3, 11, 1, 2, 1, 2, 12];
        let entry: Vec<u16> = (0..text.len() as u16).collect();
        let t = build(&text, 13, &entry);
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
