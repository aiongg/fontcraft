//! The reverse component index (M6.1): for each glyph, the glyphs that use it as a component,
//! directly and through other composites; a dependency order; the cycles.
//!
//! Rows: `components.liveUpdateFromBaseGlyph` (which glyphs follow an edit),
//! `components.nestingAllowed`, `components.placeholderCircularReference` (detection).
//!
//! Contract:
//! - Built from a whole [`Font`]; the components of every layer count, master layers and others.
//!   A component whose base glyph does not exist is listed in
//!   [`ComponentIndex::missing_bases`] and adds no edge.
//! - [`ComponentIndex::dependency_order`] lists every glyph that is not in a cycle, each after
//!   all its bases; glyphs without components keep their order in the font. Glyphs in a cycle,
//!   and the glyphs that use them, are left out of it and named in [`ComponentIndex::cycles`].
//! - The index is a snapshot. [`ComponentIndex::is_current`] tells whether a font still has the
//!   same glyphs (by `Arc` pointer); the engine rebuilds the index when it does not.
//! - Work is linear in glyphs plus components (the order uses a heap: times a logarithm);
//!   nothing recurses.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use fontcraft_model::{Font, Glyph, Shape};

/// See the module doc.
#[derive(Clone, Debug, Default)]
pub struct ComponentIndex {
    /// The glyph pointers the index was built from, in font order.
    glyphs: Vec<Arc<Glyph>>,
    /// Base glyph → the glyphs with a component of it, each once, in font order.
    users: HashMap<String, Vec<String>>,
    order: Vec<String>,
    cycles: Vec<Vec<String>>,
    /// (glyph, base name) for each component whose base glyph is missing.
    missing: Vec<(String, String)>,
    /// Position of each glyph in `order`; glyphs in or above a cycle are absent.
    rank: HashMap<String, usize>,
    /// Position of each glyph in the font.
    font_pos: HashMap<String, usize>,
}

impl ComponentIndex {
    /// The index of `font`.
    pub fn new(font: &Font) -> Self {
        let glyphs: Vec<Arc<Glyph>> = font.glyphs.iter().cloned().collect();
        let font_pos: HashMap<String, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        // bases[i]: the distinct glyphs glyph i uses, by font position, in first-use order.
        let mut bases: Vec<Vec<usize>> = vec![Vec::new(); glyphs.len()];
        let mut missing = Vec::new();
        let mut missing_seen: HashSet<(usize, &str)> = HashSet::new();
        for (i, g) in glyphs.iter().enumerate() {
            let mut seen: HashSet<usize> = HashSet::new();
            for layer in &g.layers {
                for shape in &layer.shapes {
                    let Shape::Component(c) = shape else { continue };
                    match font_pos.get(&c.glyph) {
                        Some(&b) => {
                            if seen.insert(b)
                                && let Some(list) = bases.get_mut(i)
                            {
                                list.push(b);
                            }
                        }
                        None => {
                            if missing_seen.insert((i, c.glyph.as_str())) {
                                missing.push((g.name.clone(), c.glyph.clone()));
                            }
                        }
                    }
                }
            }
        }
        // users_of[b]: the glyphs that use b, by font position, ascending.
        let mut users_of: Vec<Vec<usize>> = vec![Vec::new(); glyphs.len()];
        for (i, list) in bases.iter().enumerate() {
            for &b in list {
                if let Some(u) = users_of.get_mut(b) {
                    u.push(i);
                }
            }
        }
        let name = |i: usize| glyphs.get(i).map(|g| g.name.clone()).unwrap_or_default();
        let mut users = HashMap::new();
        for (b, list) in users_of.iter().enumerate() {
            if !list.is_empty() {
                users.insert(name(b), list.iter().map(|&u| name(u)).collect::<Vec<_>>());
            }
        }

        // Kahn's algorithm, always taking the ready glyph earliest in the font.
        let mut pending: Vec<usize> = bases.iter().map(Vec::len).collect();
        let mut ready: BinaryHeap<Reverse<usize>> = pending.iter().enumerate().filter(|(_, n)| **n == 0).map(|(i, _)| Reverse(i)).collect();
        let mut order = Vec::with_capacity(glyphs.len());
        let mut placed = vec![false; glyphs.len()];
        while let Some(Reverse(i)) = ready.pop() {
            order.push(name(i));
            if let Some(p) = placed.get_mut(i) {
                *p = true;
            }
            for &u in users_of.get(i).map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(n) = pending.get_mut(u) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        ready.push(Reverse(u));
                    }
                }
            }
        }
        let rank = order.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
        let cycles = find_cycles(&bases, &placed).into_iter().map(|c| c.into_iter().map(name).collect()).collect();
        ComponentIndex { glyphs, users, order, cycles, missing, rank, font_pos }
    }

    /// Whether `font` has exactly the glyph pointers this index was built from.
    pub fn is_current(&self, font: &Font) -> bool {
        self.glyphs.len() == font.glyphs.len() && self.glyphs.iter().zip(font.glyphs.iter()).all(|(a, b)| Arc::ptr_eq(a, b))
    }

    /// No glyph uses another.
    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    /// The glyphs with a component of `glyph`, in font order.
    pub fn direct_users(&self, glyph: &str) -> &[String] {
        self.users.get(glyph).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every glyph that uses `glyph`, directly or through other composites, each once, in
    /// dependency order (glyphs in or above a cycle last, in font order).
    pub fn users(&self, glyph: &str) -> Vec<String> {
        self.dependents([glyph])
    }

    /// The transitive users of all of `glyphs` together, each once, in dependency order. The
    /// glyphs themselves are not included unless one uses another.
    pub fn dependents<'a>(&self, glyphs: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut found: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = glyphs.into_iter().collect();
        while let Some(g) = queue.pop_front() {
            for u in self.direct_users(g) {
                if found.insert(u.as_str()) {
                    queue.push_back(u.as_str());
                }
            }
        }
        let mut out: Vec<String> = found.into_iter().map(str::to_string).collect();
        out.sort_by_key(|n| self.sort_key(n));
        out
    }

    /// Every glyph not in or above a cycle, each after its bases.
    pub fn dependency_order(&self) -> &[String] {
        &self.order
    }

    /// Each cycle as the glyph names along it (each uses the next, the last uses the first),
    /// starting from the one earliest in the font.
    pub fn cycles(&self) -> &[Vec<String>] {
        &self.cycles
    }

    /// Whether `glyph` is on a cycle or uses a glyph that is.
    pub fn in_cycle(&self, glyph: &str) -> bool {
        self.font_pos.contains_key(glyph) && !self.rank.contains_key(glyph)
    }

    /// (glyph, missing base name) for each component whose base glyph does not exist, each pair
    /// once, in font order.
    pub fn missing_bases(&self) -> &[(String, String)] {
        &self.missing
    }

    fn sort_key(&self, name: &str) -> (usize, usize) {
        match self.rank.get(name) {
            Some(&r) => (0, r),
            None => (1, self.font_pos.get(name).copied().unwrap_or(usize::MAX)),
        }
    }
}

/// One cycle through each strongly connected group of the glyphs Kahn's algorithm could not
/// place (`placed[i]` false), as font positions, starting from the group's earliest glyph.
/// `bases[i]` lists the glyphs glyph `i` uses.
fn find_cycles(bases: &[Vec<usize>], placed: &[bool]) -> Vec<Vec<usize>> {
    let left: Vec<usize> = (0..bases.len()).filter(|&i| !placed.get(i).copied().unwrap_or(true)).collect();
    if left.is_empty() {
        return Vec::new();
    }
    let groups = strongly_connected(bases, &left);
    let mut cycles = Vec::new();
    for group in groups {
        let members: HashSet<usize> = group.iter().copied().collect();
        let Some(&start) = group.iter().min() else { continue };
        let self_loop = bases.get(start).is_some_and(|b| b.contains(&start));
        if group.len() == 1 && !self_loop {
            continue;
        }
        if let Some(path) = shortest_cycle(bases, &members, start) {
            cycles.push(path);
        }
    }
    cycles.sort_by_key(|c| c.first().copied().unwrap_or(usize::MAX));
    cycles
}

/// Tarjan's strongly connected components over `nodes`, without recursion.
fn strongly_connected(bases: &[Vec<usize>], nodes: &[usize]) -> Vec<Vec<usize>> {
    let in_set: HashSet<usize> = nodes.iter().copied().collect();
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut low: HashMap<usize, usize> = HashMap::new();
    let mut on_stack: HashSet<usize> = HashSet::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut groups = Vec::new();
    let mut next = 0usize;
    for &root in nodes {
        if index.contains_key(&root) {
            continue;
        }
        // (node, position of the next edge to look at)
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        index.insert(root, next);
        low.insert(root, next);
        next += 1;
        stack.push(root);
        on_stack.insert(root);
        while let Some(top) = work.last_mut() {
            let (v, edge) = *top;
            top.1 += 1;
            let succ = bases.get(v).and_then(|b| b.get(edge)).copied();
            match succ {
                Some(w) if in_set.contains(&w) => {
                    if let std::collections::hash_map::Entry::Vacant(slot) = index.entry(w) {
                        slot.insert(next);
                        low.insert(w, next);
                        next += 1;
                        stack.push(w);
                        on_stack.insert(w);
                        work.push((w, 0));
                    } else if on_stack.contains(&w) {
                        let lw = index.get(&w).copied().unwrap_or(usize::MAX);
                        if let Some(lv) = low.get_mut(&v) {
                            *lv = (*lv).min(lw);
                        }
                    }
                }
                Some(_) => {}
                None => {
                    work.pop();
                    let lv = low.get(&v).copied().unwrap_or(usize::MAX);
                    if let Some(&(parent, _)) = work.last()
                        && let Some(lp) = low.get_mut(&parent)
                    {
                        *lp = (*lp).min(lv);
                    }
                    if Some(&lv) == index.get(&v) {
                        let mut group = Vec::new();
                        while let Some(w) = stack.pop() {
                            on_stack.remove(&w);
                            group.push(w);
                            if w == v {
                                break;
                            }
                        }
                        groups.push(group);
                    }
                }
            }
        }
    }
    groups
}

/// The shortest path from `start` back to itself inside `members`, as the glyphs along it.
fn shortest_cycle(bases: &[Vec<usize>], members: &HashSet<usize>, start: usize) -> Option<Vec<usize>> {
    let mut prev: HashMap<usize, usize> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    while let Some(v) = queue.pop_front() {
        for &w in bases.get(v).map(Vec::as_slice).unwrap_or(&[]) {
            if !members.contains(&w) {
                continue;
            }
            if w == start {
                let mut path = vec![v];
                let mut at = v;
                while at != start {
                    at = *prev.get(&at)?;
                    path.push(at);
                }
                path.reverse();
                return Some(path);
            }
            if let std::collections::hash_map::Entry::Vacant(e) = prev.entry(w) {
                e.insert(v);
                queue.push_back(w);
            }
        }
    }
    None
}
