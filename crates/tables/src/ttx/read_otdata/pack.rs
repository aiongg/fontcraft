//! The bytes of a compiled table as fontTools assembles them: a tree of writers, one per
//! table, holding data, counts filled in later and offsets to other writers; equal subtables
//! shared; then the tables laid out depth first, in reverse at each level, so that a shared
//! table comes after the last table that points to it. Ported from fontTools'
//! `ttLib.tables.otBase.OTTableWriter` (`_doneWriting`, `_gatherTables`, `getAllData`) and
//! `CountReference` (MIT).
//!
//! Sharing compares tables by content, recursively, as Python's `==` on writers does. Each
//! table gets a content class (two tables are equal exactly when their classes are), and each
//! scope keeps the first table of every class. Tables under an extension subtable are shared
//! only among themselves, and the children of a writer marked `dont_share` (the top of GSUB and
//! GPOS) are never shared.

use std::collections::{HashMap, HashSet};

use crate::limits::{MAX_SFNT, MAX_TTX_DEPTH};
use crate::{Result, TablesError};

/// How deep writers may nest. The tree follows the TTX elements, which nest at most
/// [`MAX_TTX_DEPTH`] levels; the margin covers the tables a summarised form expands into.
const MAX_WRITER_DEPTH: usize = MAX_TTX_DEPTH + 8;

/// One thing a writer holds.
#[derive(Clone, Debug)]
pub(crate) enum Item {
    Data(Vec<u8>),
    /// An offset of `size` bytes (2, 3 or 4) to another writer.
    Offset {
        size: u8,
        node: usize,
    },
    /// A count of `size` bytes (1, 2 or 4), filled from a slot when the tree is done.
    Count {
        slot: usize,
        size: u8,
    },
}

/// One table being written (fontTools' `OTTableWriter`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    pub items: Vec<Item>,
    /// The writer of an extension subtable: what it points to is laid out after every other
    /// table and shared only within its own tree.
    pub extension: bool,
    /// Its children are never shared (the top of GSUB and GPOS).
    pub dont_share: bool,
    /// Its coverage table is laid out after its other subtables.
    pub sort_coverage_last: bool,
    /// The field that points to it is named `Coverage`.
    pub is_coverage: bool,
}

/// The writers of one table and the values of its counts.
#[derive(Debug, Default)]
pub(crate) struct Graph {
    pub nodes: Vec<Node>,
    slots: Vec<Option<i64>>,
}

/// A content class: a writer's items with each offset replaced by its target's class.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Data(Vec<u8>),
    Offset(u8, usize),
}

fn overflow() -> TablesError {
    TablesError::invalid("an offset does not fit its field; the reference compiler would restructure the lookups, which this reader does not do")
}

impl Graph {
    /// A new, empty writer.
    pub(crate) fn node(&mut self) -> usize {
        self.nodes.push(Node::default());
        self.nodes.len() - 1
    }

    pub(crate) fn get_mut(&mut self, node: usize) -> Result<&mut Node> {
        self.nodes.get_mut(node).ok_or_else(|| TablesError::invalid("a writer that does not exist"))
    }

    pub(crate) fn push(&mut self, node: usize, item: Item) -> Result<()> {
        self.get_mut(node)?.items.push(item);
        Ok(())
    }

    pub(crate) fn data(&mut self, node: usize, bytes: Vec<u8>) -> Result<()> {
        self.push(node, Item::Data(bytes))
    }

    /// A new count, unset or with a value.
    pub(crate) fn slot(&mut self, value: Option<i64>) -> usize {
        self.slots.push(value);
        self.slots.len() - 1
    }

    pub(crate) fn slot_value(&self, slot: usize) -> Option<i64> {
        self.slots.get(slot).copied().flatten()
    }

    /// `CountReference.setValue`: the first value sticks, a different one later is an error.
    pub(crate) fn set_slot(&mut self, slot: usize, value: i64, what: &str) -> Result<()> {
        let s = self.slots.get_mut(slot).ok_or_else(|| TablesError::invalid("a count that does not exist"))?;
        match s {
            None => *s = Some(value),
            Some(v) if *v == value => {}
            Some(v) => return Err(TablesError::invalid(format!("'{what}' is {v} in one place and {value} in another"))),
        }
        Ok(())
    }

    /// The bytes of the tree under `root` (fontTools' `getAllData`).
    pub(crate) fn assemble(mut self, root: usize) -> Result<Vec<u8>> {
        let mut classes: HashMap<Vec<Key>, usize> = HashMap::new();
        let mut scope: HashMap<usize, usize> = HashMap::new();
        self.done(root, &mut scope, &mut classes, 0)?;
        let mut tables = Vec::new();
        let mut ext = Vec::new();
        let mut seen = HashSet::new();
        self.gather(root, &mut tables, Some(&mut ext), &mut seen, 0)?;
        tables.reverse();
        ext.reverse();
        let mut pos: HashMap<usize, usize> = HashMap::new();
        let mut at = 0usize;
        for &id in tables.iter().chain(&ext) {
            pos.insert(id, at);
            let node = self.nodes.get(id).ok_or_else(overflow)?;
            let len: usize = node
                .items
                .iter()
                .map(|i| match i {
                    Item::Data(d) => d.len(),
                    Item::Offset { size, .. } | Item::Count { size, .. } => usize::from(*size),
                })
                .sum();
            at = at.saturating_add(len);
            if at > MAX_SFNT {
                return Err(TablesError::TooLarge { what: "a compiled table", limit: MAX_SFNT });
            }
        }
        let mut out = Vec::with_capacity(at);
        for &id in tables.iter().chain(&ext) {
            let node = self.nodes.get(id).ok_or_else(overflow)?;
            let here = pos.get(&id).copied().unwrap_or(0);
            for item in &node.items {
                match item {
                    Item::Data(d) => out.extend_from_slice(d),
                    Item::Count { .. } => return Err(TablesError::invalid("a count left unfilled")),
                    Item::Offset { size, node } => {
                        let there = pos.get(node).copied().ok_or_else(overflow)?;
                        let v = there.checked_sub(here).ok_or_else(overflow)?;
                        let v = u32::try_from(v).map_err(|_| overflow())?;
                        match size {
                            2 => out.extend_from_slice(&u16::try_from(v).map_err(|_| overflow())?.to_be_bytes()),
                            3 if v < 1 << 24 => out.extend_from_slice(v.to_be_bytes().get(1..).unwrap_or_default()),
                            4 => out.extend_from_slice(&v.to_be_bytes()),
                            _ => return Err(overflow()),
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// `_doneWriting`: fills the counts and shares equal subtables. Returns the content class of
    /// `id`.
    fn done(&mut self, id: usize, scope: &mut HashMap<usize, usize>, classes: &mut HashMap<Vec<Key>, usize>, depth: usize) -> Result<usize> {
        if depth > MAX_WRITER_DEPTH {
            return Err(TablesError::TooLarge { what: "the nesting of subtables", limit: MAX_WRITER_DEPTH });
        }
        let node = self.nodes.get(id).ok_or_else(overflow)?;
        let (extension, dont_share) = (node.extension, node.dont_share);
        let count = node.items.len();
        let mut fresh = HashMap::new();
        let scope = if extension { &mut fresh } else { scope };
        let mut key = Vec::with_capacity(count);
        for i in 0..count {
            let item = self.nodes.get(id).and_then(|n| n.items.get(i)).cloned().ok_or_else(overflow)?;
            match item {
                Item::Data(d) => key.push(Key::Data(d)),
                Item::Count { slot, size } => {
                    let v = self.slot_value(slot).unwrap_or(0);
                    let bytes = match size {
                        1 => u8::try_from(v).map(|v| vec![v]),
                        2 => u16::try_from(v).map(|v| v.to_be_bytes().to_vec()),
                        _ => u32::try_from(v).map(|v| v.to_be_bytes().to_vec()),
                    }
                    .map_err(|_| TablesError::invalid(format!("a count of {v} does not fit its field")))?;
                    key.push(Key::Data(bytes.clone()));
                    if let Some(slot) = self.nodes.get_mut(id).and_then(|n| n.items.get_mut(i)) {
                        *slot = Item::Data(bytes);
                    }
                }
                Item::Offset { size, node: child } => {
                    let class = self.done(child, scope, classes, depth + 1)?;
                    let target = if dont_share { child } else { *scope.entry(class).or_insert(child) };
                    if let Some(Item::Offset { node, .. }) = self.nodes.get_mut(id).and_then(|n| n.items.get_mut(i)) {
                        *node = target;
                    }
                    key.push(Key::Offset(size, class));
                }
            }
        }
        let next = classes.len();
        Ok(*classes.entry(key).or_insert(next))
    }

    /// `_gatherTables`: the tables in the order they are laid out, reversed.
    fn gather(&self, id: usize, tables: &mut Vec<usize>, ext: Option<&mut Vec<usize>>, seen: &mut HashSet<usize>, depth: usize) -> Result<()> {
        if depth > MAX_WRITER_DEPTH {
            return Err(TablesError::TooLarge { what: "the nesting of subtables", limit: MAX_WRITER_DEPTH });
        }
        seen.insert(id);
        let node = self.nodes.get(id).ok_or_else(overflow)?;
        let mut fresh = HashSet::new();
        let (inner, mut inner_ext, inner_seen): (&mut Vec<usize>, Option<&mut Vec<usize>>, &mut HashSet<usize>) = if node.extension {
            let ext = ext.ok_or_else(|| TablesError::invalid("an extension subtable inside an extension subtable"))?;
            (ext, None, &mut fresh)
        } else {
            (&mut *tables, ext, &mut *seen)
        };
        let child = |item: &Item| match item {
            Item::Offset { node, .. } => Some(*node),
            _ => None,
        };
        let is_coverage = |n: usize| self.nodes.get(n).is_some_and(|n| n.is_coverage);
        let mut coverage_first = false;
        if node.sort_coverage_last {
            // As in fontTools: the first offset to a coverage table, or failing that the last item.
            let found = node.items.iter().find_map(|i| child(i).filter(|&n| is_coverage(n)));
            coverage_first = found.is_some();
            let target =
                found.or_else(|| node.items.last().and_then(child)).ok_or_else(|| TablesError::invalid("a subtable without a coverage table"))?;
            if !inner_seen.contains(&target) {
                self.gather(target, inner, inner_ext.as_deref_mut(), inner_seen, depth + 1)?;
            }
        }
        for (i, item) in node.items.iter().enumerate().rev() {
            let Some(target) = child(item) else { continue };
            if coverage_first && i == 1 && is_coverage(target) {
                continue;
            }
            if !inner_seen.contains(&target) {
                self.gather(target, inner, inner_ext.as_deref_mut(), inner_seen, depth + 1)?;
            }
        }
        tables.push(id);
        Ok(())
    }
}
