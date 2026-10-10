//! Contours as linked on-curve nodes, for operations that rewire them: Reconnect Nodes and Cut
//! with Line. Each node keeps the off-curve nodes of the segment that ends at it, so changing a
//! node's successor never splits a segment's data.

use fontcraft_geom::Vec2;

use crate::contour::{Contour, DIRECTION_EPSILON, Node, NodeKind, RNode, Ring};

/// Where a node sorts among the result contours: input contour, ring index, and 0 for an input
/// node or a larger number for a copy.
pub(crate) type Key = (usize, usize, usize);

/// A closed result contour with less area than this, in square units, is a sliver the rewiring
/// left behind.
const MIN_AREA: f64 = 1e-9;

pub(crate) struct GNode {
    pub r: RNode,
    pub key: Key,
    pub succ: Option<usize>,
    pub pred: Option<usize>,
    pub alive: bool,
}

#[derive(Default)]
pub(crate) struct Graph {
    pub nodes: Vec<GNode>,
}

/// The result of an operation that rewires contours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rewired {
    pub contours: Vec<Contour>,
    /// For each result contour, the input contour it starts in. Contours the operation did not
    /// touch are returned as they were, in their places.
    pub sources: Vec<usize>,
    /// The nodes the operation made or joined, as `(contour, node)` in the result.
    pub selection: Vec<(usize, usize)>,
}

impl Rewired {
    pub(crate) fn unchanged(contours: &[Contour]) -> Self {
        Self { contours: contours.to_vec(), sources: (0..contours.len()).collect(), selection: Vec::new() }
    }
}

impl Graph {
    /// Adds the nodes of `ring`, which is input contour `contour`, linked in path order. Returns
    /// the id of its first node; the others follow in ring order.
    pub fn add_ring(&mut self, contour: usize, ring: &Ring) -> usize {
        let base = self.nodes.len();
        let m = ring.len();
        for (k, r) in ring.nodes.iter().enumerate() {
            let succ = if k + 1 < m {
                Some(base + k + 1)
            } else if ring.closed {
                Some(base)
            } else {
                None
            };
            let pred = if k > 0 {
                Some(base + k - 1)
            } else if ring.closed {
                Some(base + m - 1)
            } else {
                None
            };
            self.nodes.push(GNode { r: r.clone(), key: (contour, k, 0), succ, pred, alive: true });
        }
        base
    }

    pub fn get(&self, id: usize) -> Option<&GNode> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut GNode> {
        self.nodes.get_mut(id)
    }

    fn set_succ(&mut self, id: usize, succ: Option<usize>) {
        if let Some(g) = self.nodes.get_mut(id) {
            g.succ = succ;
        }
    }

    fn set_pred(&mut self, id: Option<usize>, pred: Option<usize>) {
        if let Some(g) = id.and_then(|id| self.nodes.get_mut(id)) {
            g.pred = pred;
        }
    }

    /// A new node at the place of `id`, reached by a straight segment, with no links yet.
    fn copy_of(&mut self, id: usize) -> Option<usize> {
        let g = self.nodes.get(id)?;
        let node = Node::new(g.r.node.point, NodeKind::Line);
        let key = (g.key.0, g.key.1, self.nodes.len() + 1);
        let mark = g.r.mark;
        let new = self.nodes.len();
        self.nodes.push(GNode { r: RNode { node, ctrl: Vec::new(), src: None, mark }, key, succ: None, pred: None, alive: true });
        Some(new)
    }

    /// Swaps what follows `p` and `q`: the path that reached `p` goes on with a straight segment
    /// to a copy of `q` and then the rest of `q`'s path, and the path that reached `q` goes on to
    /// a copy of `p` and the rest of `p`'s path. Two contours become one, or one becomes two.
    pub fn rewire(&mut self, p: usize, q: usize) -> Option<(usize, usize)> {
        if p == q {
            return None;
        }
        let sp = self.nodes.get(p)?.succ;
        let sq = self.nodes.get(q)?.succ;
        let p_copy = self.copy_of(p)?;
        let q_copy = self.copy_of(q)?;
        self.set_succ(p, Some(q_copy));
        self.set_succ(q, Some(p_copy));
        self.set_succ(p_copy, sp);
        self.set_succ(q_copy, sq);
        if let Some(g) = self.nodes.get_mut(p_copy) {
            g.pred = Some(q);
        }
        if let Some(g) = self.nodes.get_mut(q_copy) {
            g.pred = Some(p);
        }
        self.set_pred(sp, Some(p_copy));
        self.set_pred(sq, Some(q_copy));
        Some((p_copy, q_copy))
    }

    /// Ends the path at `x`; what followed `x` starts at a copy of it.
    pub fn split(&mut self, x: usize) -> Option<usize> {
        let g = self.nodes.get(x)?;
        let (s, _) = (g.succ?, g.pred?);
        let copy = self.copy_of(x)?;
        self.set_succ(x, None);
        self.set_succ(copy, Some(s));
        self.set_pred(Some(s), Some(copy));
        Some(copy)
    }

    /// Removes the node after `id`; `id` takes its place in the path.
    pub fn drop_next(&mut self, id: usize) -> Option<()> {
        let next = self.nodes.get(id)?.succ?;
        let after = self.nodes.get(next)?.succ;
        if let Some(g) = self.nodes.get_mut(next) {
            g.alive = false;
        }
        if after == Some(next) {
            return None;
        }
        self.set_succ(id, after.map(|a| if a == next { id } else { a }));
        self.set_pred(after, Some(id));
        Some(())
    }

    /// The direction in which the path arrives at `id`.
    pub fn tangent_in(&self, id: usize) -> Option<Vec2> {
        let g = self.nodes.get(id)?;
        let from = self.nodes.get(g.pred?)?.r.node.point;
        let to = g.r.node.point;
        let q = g.r.ctrl.iter().rev().map(|c| c.point).chain(std::iter::once(from)).find(|q| q.distance(to) > DIRECTION_EPSILON)?;
        Some((to - q).normalize())
    }

    /// The direction in which the path leaves `id`.
    pub fn tangent_out(&self, id: usize) -> Option<Vec2> {
        let g = self.nodes.get(id)?;
        let next = self.nodes.get(g.succ?)?;
        let from = g.r.node.point;
        let q = next.r.ctrl.iter().map(|c| c.point).chain(std::iter::once(next.r.node.point)).find(|q| q.distance(from) > DIRECTION_EPSILON)?;
        Some((q - from).normalize())
    }

    /// The paths through the live nodes, each with the smallest key among its nodes. Open paths
    /// start at a node nothing leads to; closed ones start at their node with the smallest key.
    /// Straight segments of zero length are merged away, and slivers of no area and open paths of
    /// one node are dropped.
    pub fn trace(&self) -> Vec<(Key, Ring)> {
        let n = self.nodes.len();
        let mut visited = vec![false; n];
        let mut out = Vec::new();
        let alive = |i: usize| self.nodes.get(i).is_some_and(|g| g.alive);
        let starts_chain = |i: usize| self.nodes.get(i).is_some_and(|g| g.pred.is_none_or(|p| !alive(p)));
        for pass in 0..2 {
            for start in 0..n {
                if !alive(start) || visited.get(start).copied().unwrap_or(true) || (pass == 0 && !starts_chain(start)) {
                    continue;
                }
                let mut ids = Vec::new();
                let mut at = Some(start);
                let mut closed = false;
                // At most `n` steps: every node is visited once.
                for _ in 0..n {
                    let Some(i) = at else { break };
                    if i == start && !ids.is_empty() {
                        closed = true;
                        break;
                    }
                    let Some(v) = visited.get_mut(i) else { break };
                    if *v || !alive(i) {
                        break;
                    }
                    *v = true;
                    ids.push(i);
                    at = self.nodes.get(i).and_then(|g| g.succ);
                }
                if pass == 1 && !closed {
                    // A cycle broken by a bad link; it is still drawn, as a closed path.
                    closed = true;
                }
                if let Some(traced) = self.ring_of(&ids, closed) {
                    out.push(traced);
                }
            }
        }
        out
    }

    fn ring_of(&self, ids: &[usize], closed: bool) -> Option<(Key, Ring)> {
        let mut nodes: Vec<(Key, RNode)> = ids.iter().filter_map(|&i| self.nodes.get(i)).map(|g| (g.key, g.r.clone())).collect();
        if closed {
            let at = nodes.iter().enumerate().min_by_key(|(_, (k, _))| *k).map_or(0, |(i, _)| i);
            nodes.rotate_left(at);
        } else if let Some((_, first)) = nodes.first_mut() {
            first.ctrl.clear();
            first.node.kind = NodeKind::Line;
        }
        let key = nodes.iter().map(|(k, _)| *k).min()?;
        let mut ring = Ring { closed, nodes: nodes.into_iter().map(|(_, r)| r).collect() };
        ring.remove_zero_length(0.0, |_, b| b.ctrl.is_empty());
        for r in ring.nodes.iter_mut().filter(|r| r.mark) {
            r.node.smooth = false;
        }
        let keep = if closed { ring.area().abs() >= MIN_AREA } else { ring.len() > 1 };
        keep.then_some((key, ring))
    }
}

/// The result contours: the untouched ones in their places, the traced ones where their smallest
/// key puts them.
pub(crate) fn assemble(contours: &[Contour], involved: &[bool], traced: Vec<(Key, Ring)>) -> Rewired {
    let mut all: Vec<(Key, Ring, Option<Contour>)> = Vec::new();
    for (i, c) in contours.iter().enumerate() {
        if !involved.get(i).copied().unwrap_or(false) {
            all.push(((i, 0, 0), Ring::default(), Some(c.clone())));
        }
    }
    for (key, ring) in traced {
        all.push((key, ring, None));
    }
    all.sort_by_key(|(k, _, _)| *k);
    let mut out = Rewired::default();
    for (index, (key, ring, kept)) in all.into_iter().enumerate() {
        out.sources.push(key.0);
        match kept {
            Some(c) => out.contours.push(c),
            None => {
                for (k, r) in ring.nodes.iter().enumerate() {
                    if r.mark {
                        out.selection.push((index, ring.list_index(k)));
                    }
                }
                out.contours.push(ring.to_contour());
            }
        }
    }
    out
}
