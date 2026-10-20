//! Subsetting `SVG `, ported from fontTools' `subset.svg` (MIT, Copyright 2021 Google LLC).
//!
//! A document stays when it covers a kept glyph. Inside it, the elements with the id
//! `glyph<N>` of the kept glyphs stay, with every element they reference (through
//! `xlink:href="#…"`, `fill="url(#…)"`, `clip-path="url(#…)"` or the same in `style`), the
//! ancestors of those, and the whole subtree of each kept element; every other element,
//! comment and processing instruction goes, with the text after it. Unless glyph ids are
//! retained, `glyph<N>` ids follow the output numbering (ids of glyphs not in the subset get a
//! leading dot) and `xlink:href="#glyph…"` links follow them. The document is written from its
//! root element; what is kept keeps its source text, except text that is only whitespace.
//! One record is written per run of consecutive output glyphs.
//!
//! The XML reader is a small one of our own: elements, attributes, text, CDATA, comments,
//! processing instructions and a doctype; nesting is capped by [`MAX_SVG_DEPTH`]. Each document
//! is read once however many records point to it, and its parse tree is dropped after the last
//! record that needs it. The bytes of all documents read are capped by [`MAX_SVG_PARSE_RATIO`]
//! times the table's length, and the work of all records together by [`MAX_SVG_WORK`].
//! Documents compressed with gzip are refused. A record whose first glyph comes after its last
//! covers no glyph, as in fontTools.

use std::collections::{HashMap, HashSet};

use super::super::Plan;
use crate::reorder::be::{Data, put_u16, put_u32, to_u16, to_u32};
use crate::{Result, TablesError};

/// Deepest element nesting read.
pub const MAX_SVG_DEPTH: usize = 256;
/// Most node visits the subsetting of the whole table may make: each record's pass over its
/// document plus its reference closure.
pub const MAX_SVG_WORK: usize = 1 << 26;
/// Most bytes the document reads of one table may scan together, as a multiple of the table's
/// length. Documents that do not overlap scan at most the table once; records whose documents
/// overlap (one start, many lengths) would otherwise scan the same bytes once per record.
pub const MAX_SVG_PARSE_RATIO: usize = 4;
/// Most bytes of subset documents the output may hold.
pub const MAX_SVG_OUTPUT: usize = 1 << 27;

fn bad(what: &str) -> TablesError {
    TablesError::invalid(format!("an SVG document cannot be read: {what}"))
}

/// A node of the document, as byte ranges of its source.
#[derive(Debug)]
enum XNode {
    Elem(Elem),
    /// Text or CDATA.
    Text,
    /// A comment or processing instruction.
    Other,
}

#[derive(Debug, Default)]
struct Elem {
    /// The start tag, `<` to `>`.
    start: (usize, usize),
    /// The end tag, empty for a self-closing element.
    close: (usize, usize),
    /// Attribute name and value ranges (the value without its quotes).
    attrs: Vec<((usize, usize), (usize, usize))>,
    children: Vec<usize>,
}

struct Doc<'a> {
    src: &'a [u8],
    nodes: Vec<(XNode, (usize, usize))>,
    root: usize,
}

impl<'a> Doc<'a> {
    fn parse(src: &'a [u8]) -> Result<Self> {
        let mut nodes: Vec<(XNode, (usize, usize))> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut root = None;
        let mut i = 0usize;
        let find = |from: usize, pat: &[u8]| -> Result<usize> {
            src.get(from..)
                .and_then(|rest| rest.windows(pat.len()).position(|w| w == pat))
                .map(|p| from + p)
                .ok_or_else(|| bad("a construct is not closed"))
        };
        while i < src.len() {
            let rest = src.get(i..).unwrap_or_default();
            let (node, end) = if rest.starts_with(b"<!--") {
                (XNode::Other, find(i + 4, b"-->")? + 3)
            } else if rest.starts_with(b"<![CDATA[") {
                (XNode::Text, find(i + 9, b"]]>")? + 3)
            } else if rest.starts_with(b"<?") {
                (XNode::Other, find(i + 2, b"?>")? + 2)
            } else if rest.starts_with(b"<!") {
                // A doctype, possibly with an internal subset.
                let mut j = i + 2;
                let mut depth = 0i32;
                loop {
                    match src.get(j) {
                        Some(b'[') => depth += 1,
                        Some(b']') => depth -= 1,
                        Some(b'>') if depth <= 0 => break,
                        None => return Err(bad("a doctype is not closed")),
                        _ => {}
                    }
                    j += 1;
                }
                if !stack.is_empty() {
                    return Err(bad("a doctype inside an element"));
                }
                i = j + 1;
                continue;
            } else if rest.starts_with(b"</") {
                let end = find(i, b">")? + 1;
                let open = stack.pop().ok_or_else(|| bad("an end tag without a start tag"))?;
                if let Some((XNode::Elem(e), range)) = nodes.get_mut(open) {
                    e.close = (i, end);
                    range.1 = end;
                }
                i = end;
                continue;
            } else if rest.starts_with(b"<") {
                let (elem, end, empty) = start_tag(src, i)?;
                let idx = nodes.len();
                nodes.push((XNode::Elem(elem), (i, end)));
                attach(&mut nodes, &stack, idx, &mut root)?;
                if !empty {
                    if stack.len() >= MAX_SVG_DEPTH {
                        return Err(TablesError::invalid(format!("an SVG document nests elements deeper than {MAX_SVG_DEPTH}")));
                    }
                    stack.push(idx);
                } else if let Some((XNode::Elem(e), _)) = nodes.get_mut(idx) {
                    e.close = (end, end);
                }
                i = end;
                continue;
            } else {
                let end = src.get(i..).and_then(|r| r.iter().position(|b| *b == b'<')).map_or(src.len(), |p| i + p);
                (XNode::Text, end)
            };
            let idx = nodes.len();
            nodes.push((node, (i, end)));
            if !stack.is_empty() {
                attach(&mut nodes, &stack, idx, &mut root)?;
            }
            i = end;
        }
        if !stack.is_empty() {
            return Err(bad("an element is not closed"));
        }
        let root = root.ok_or_else(|| bad("there is no root element"))?;
        Ok(Doc { src, nodes, root })
    }

    fn elem(&self, i: usize) -> Option<&Elem> {
        match self.nodes.get(i) {
            Some((XNode::Elem(e), _)) => Some(e),
            _ => None,
        }
    }

    fn text(&self, r: (usize, usize)) -> &'a [u8] {
        self.src.get(r.0..r.1).unwrap_or_default()
    }

    fn attr(&self, e: &Elem, name: &[u8]) -> Option<&'a [u8]> {
        e.attrs.iter().find(|(n, _)| self.text(*n) == name).map(|(_, v)| self.text(*v))
    }

    fn id(&self, i: usize) -> Option<&'a [u8]> {
        self.elem(i).and_then(|e| self.attr(e, b"id"))
    }

    /// Every element below `i`, depth first.
    fn descendants(&self, i: usize, out: &mut Vec<usize>) {
        let mut stack = vec![i];
        while let Some(n) = stack.pop() {
            if let Some(e) = self.elem(n) {
                for c in e.children.iter().rev() {
                    if self.elem(*c).is_some() {
                        out.push(*c);
                        stack.push(*c);
                    }
                }
            }
        }
    }

    /// `e` has an attribute that can refer to an id, as the selection of `iter_referenced_ids`
    /// tests it.
    fn has_reference(&self, e: &Elem) -> bool {
        self.attr(e, b"xlink:href").is_some_and(|v| v.starts_with(b"#"))
            || self.attr(e, b"fill").is_some_and(|v| v.starts_with(b"url(#"))
            || self.attr(e, b"clip-path").is_some_and(|v| v.starts_with(b"url(#"))
            || self.attr(e, b"style").is_some_and(|v| v.windows(6).any(|w| w == b":url(#"))
    }

    /// The ids element `e` refers to (`iter_referenced_ids` on one element).
    fn references(&self, e: &Elem, out: &mut Vec<Vec<u8>>) {
        if let Some(href) = self.attr(e, b"xlink:href")
            && href.len() > 1
            && let Some(target) = href.strip_prefix(b"#")
        {
            out.push(target.to_vec());
        }
        let mut fill = self.attr(e, b"fill").map(<[u8]>::to_vec);
        let mut clip = self.attr(e, b"clip-path").map(<[u8]>::to_vec);
        if let Some(style) = self.attr(e, b"style") {
            for decl in style.split(|b| *b == b';') {
                if decl.iter().filter(|b| **b == b':').count() != 1 {
                    continue;
                }
                let mut parts = decl.splitn(2, |b| *b == b':');
                let (Some(k), Some(v)) = (parts.next(), parts.next()) else { continue };
                match k.trim_ascii() {
                    b"fill" => fill = Some(v.trim_ascii().to_vec()),
                    b"clip-path" => clip = Some(v.trim_ascii().to_vec()),
                    _ => {}
                }
            }
        }
        for v in [fill, clip].into_iter().flatten() {
            if let Some(inner) = v.strip_prefix(b"url(#").and_then(|r| r.strip_suffix(b")"))
                && !inner.is_empty()
            {
                out.push(inner.to_vec());
            }
        }
    }
}

fn attach(nodes: &mut [(XNode, (usize, usize))], stack: &[usize], idx: usize, root: &mut Option<usize>) -> Result<()> {
    match stack.last() {
        Some(parent) => {
            if let Some((XNode::Elem(p), _)) = nodes.get_mut(*parent) {
                p.children.push(idx);
            }
        }
        None => {
            if root.is_some() {
                return Err(bad("more than one root element"));
            }
            *root = Some(idx);
        }
    }
    Ok(())
}

/// The start tag at `at`: the element, the end of the tag, and whether it closes itself.
fn start_tag(src: &[u8], at: usize) -> Result<(Elem, usize, bool)> {
    let mut e = Elem::default();
    let mut i = at + 1;
    let name_end = |mut j: usize| {
        while src.get(j).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'>' | b'/' | b'=')) {
            j += 1;
        }
        j
    };
    i = name_end(i);
    loop {
        while src.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        match src.get(i) {
            None => return Err(bad("a start tag is not closed")),
            Some(b'>') => {
                e.start = (at, i + 1);
                return Ok((e, i + 1, false));
            }
            Some(b'/') => {
                if src.get(i + 1) != Some(&b'>') {
                    return Err(bad("a stray '/' in a start tag"));
                }
                e.start = (at, i + 2);
                return Ok((e, i + 2, true));
            }
            Some(_) => {
                let n0 = i;
                i = name_end(i);
                if i == n0 {
                    return Err(bad("an attribute without a name"));
                }
                let name = (n0, i);
                while src.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                if src.get(i) != Some(&b'=') {
                    return Err(bad("an attribute without a value"));
                }
                i += 1;
                while src.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                let quote = *src.get(i).filter(|q| matches!(q, b'"' | b'\'')).ok_or_else(|| bad("an attribute value without quotes"))?;
                let v0 = i + 1;
                let v1 = src
                    .get(v0..)
                    .and_then(|r| r.iter().position(|b| *b == quote))
                    .map(|p| v0 + p)
                    .ok_or_else(|| bad("an attribute value is not closed"))?;
                e.attrs.push((name, (v0, v1)));
                i = v1 + 1;
            }
        }
    }
}

/// The glyph number of an id `glyph<N>`.
fn glyph_number(id: &[u8]) -> Option<u32> {
    let digits = id.strip_prefix(b"glyph")?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let trimmed = std::str::from_utf8(digits).ok()?.trim_start_matches('0');
    if trimmed.is_empty() {
        return Some(0);
    }
    trimmed.parse().ok()
}

/// Writes `SVG ` subset to the kept glyphs; `None` when no document is left.
pub(crate) fn subset(plan: &Plan, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let d = Data::new(*b"SVG ", bytes);
    let list = d.u32_usize(2)?;
    let count = usize::from(d.u16(list)?);
    let mut records: Vec<(u16, u16, usize)> = Vec::new();
    let mut docs: Vec<Vec<u8>> = Vec::new();
    let mut doc_index: HashMap<Vec<u8>, usize> = HashMap::new();
    // The records that cover a kept glyph, and the last of them to use each document.
    let mut uses: Vec<(u16, u16, (usize, usize))> = Vec::new();
    let mut last_use: HashMap<(usize, usize), usize> = HashMap::new();
    for i in 0..count {
        let r = list + 2 + 12 * i;
        let (start, end) = (d.u16(r)?, d.u16(r + 2)?);
        if start > end {
            continue;
        }
        let place = (list + d.u32_usize(r + 4)?, d.u32_usize(r + 8)?);
        d.slice(place.0, place.1)?;
        if plan.glyphs.range(u32::from(start)..=u32::from(end)).next().is_none() {
            continue;
        }
        last_use.insert(place, uses.len());
        uses.push((start, end, place));
    }
    let mut parsed: HashMap<(usize, usize), Parsed<'_>> = HashMap::new();
    let mut work = MAX_SVG_WORK;
    let parse_limit = bytes.len().saturating_mul(MAX_SVG_PARSE_RATIO);
    let mut parse_left = parse_limit;
    let mut written = 0usize;
    for (u, (start, end, place)) in uses.into_iter().enumerate() {
        let doc = d.slice(place.0, place.1)?;
        let gids: Vec<u32> = plan.glyphs.range(u32::from(start)..=u32::from(end)).copied().collect();
        if doc.starts_with(&[0x1F, 0x8B]) {
            return Err(TablesError::Unsupported("subsetting gzip-compressed SVG documents".into()));
        }
        let parsed_doc = match parsed.entry(place) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                parse_left = parse_left
                    .checked_sub(doc.len())
                    .ok_or(TablesError::TooLarge { what: "the bytes of the SVG documents read", limit: parse_limit })?;
                e.insert(Parsed::new(doc)?)
            }
        };
        let text = subset_doc(plan, parsed_doc, &gids, &mut work)?;
        if last_use.get(&place) == Some(&u) {
            parsed.remove(&place);
        }
        let Some(text) = text else { continue };
        let index = match doc_index.get(&text) {
            Some(i) => *i,
            None => {
                written = written.saturating_add(text.len());
                if written > MAX_SVG_OUTPUT {
                    return Err(TablesError::TooLarge { what: "the subset SVG documents", limit: MAX_SVG_OUTPUT });
                }
                doc_index.insert(text.clone(), docs.len());
                docs.push(text);
                docs.len() - 1
            }
        };
        let mut new: Vec<u16> = Vec::with_capacity(gids.len());
        for g in &gids {
            let n = plan.glyph_map.get(g).ok_or_else(|| TablesError::invalid("an SVG glyph left the subset"))?;
            new.push(to_u16(*n as usize, "a glyph id")?);
        }
        new.sort_unstable();
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for g in new {
            match runs.last_mut() {
                Some((_, e)) if u32::from(*e) + 1 == u32::from(g) => *e = g,
                _ => runs.push((g, g)),
            }
        }
        records.extend(runs.into_iter().map(|(s, e)| (s, e, index)));
    }
    if records.is_empty() {
        return Ok(None);
    }
    let list_len = 2 + 12 * records.len();
    let mut doc_at = Vec::with_capacity(docs.len());
    let mut at = list_len;
    for doc in &docs {
        doc_at.push(at);
        at += doc.len();
    }
    let mut out = Vec::new();
    put_u16(&mut out, d.u16(0)?);
    put_u32(&mut out, 10);
    put_u32(&mut out, 0);
    put_u16(&mut out, to_u16(records.len(), "the SVG document records")?);
    for (s, e, index) in &records {
        put_u16(&mut out, *s);
        put_u16(&mut out, *e);
        put_u32(&mut out, to_u32(doc_at.get(*index).copied().unwrap_or(0), "the SVG table")?);
        put_u32(&mut out, to_u32(docs.get(*index).map_or(0, Vec::len), "an SVG document")?);
    }
    for doc in docs {
        out.extend(doc);
    }
    Ok(Some(out))
}

/// A document read once, with its elements by id.
struct Parsed<'a> {
    doc: Doc<'a>,
    elements: HashMap<&'a [u8], usize>,
}

impl<'a> Parsed<'a> {
    fn new(src: &'a [u8]) -> Result<Self> {
        let doc = Doc::parse(src)?;
        let mut elements: HashMap<&'a [u8], usize> = HashMap::new();
        let mut all = vec![doc.root];
        doc.descendants(doc.root, &mut all);
        for e in &all {
            if let Some(id) = doc.id(*e) {
                elements.insert(id, *e);
            }
        }
        Ok(Parsed { doc, elements })
    }
}

fn charge(work: &mut usize, visits: usize) -> Result<()> {
    *work = work.checked_sub(visits).ok_or(TablesError::TooLarge { what: "the SVG subsetting work", limit: MAX_SVG_WORK })?;
    Ok(())
}

/// The document of `parsed` keeping glyphs `gids` (input ids); `None` when nothing is left.
fn subset_doc(plan: &Plan, parsed: &Parsed<'_>, gids: &[u32], work: &mut usize) -> Result<Option<Vec<u8>>> {
    let Parsed { doc, elements } = parsed;
    let src = doc.src;
    // The pruning, renaming and writing passes each visit the nodes once.
    charge(work, doc.nodes.len())?;

    // The ids to keep: the glyphs', then everything they reference.
    let mut keep: HashSet<Vec<u8>> = gids.iter().map(|g| format!("glyph{g}").into_bytes()).collect();
    let mut todo: Vec<Vec<u8>> = keep.iter().cloned().collect();
    while let Some(id) = todo.pop() {
        let Some(e) = elements.get(id.as_slice()) else { continue };
        let mut under = vec![*e];
        doc.descendants(*e, &mut under);
        charge(work, under.len())?;
        let mut refs = Vec::new();
        for (k, n) in under.into_iter().enumerate() {
            if let Some(el) = doc.elem(n)
                && (k == 0 || doc.has_reference(el))
            {
                doc.references(el, &mut refs);
            }
        }
        for r in refs {
            if keep.insert(r.clone()) {
                todo.push(r);
            }
        }
    }

    // Mark what goes: every node not on the way to a kept id, outside a kept subtree.
    let mut removed = vec![false; doc.nodes.len()];
    if !prune(doc, doc.root, &keep, &mut removed, 0) {
        return Ok(None);
    }

    // The ids of glyph elements follow the output numbering, and so do links to them.
    let mut edits: HashMap<(usize, usize), Vec<u8>> = HashMap::new();
    if !plan.options.retain_gids {
        let mut kept = vec![doc.root];
        descendants_kept(doc, doc.root, &removed, &mut kept);
        let ids: HashSet<&[u8]> = kept.iter().filter_map(|e| doc.id(*e)).collect();
        let mut renamed: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        for e in &kept {
            let Some(el) = doc.elem(*e) else { continue };
            let Some((_, value)) = el.attrs.iter().find(|(n, _)| doc.text(*n) == b"id") else { continue };
            let id = doc.text(*value);
            let Some(old) = glyph_number(id) else { continue };
            let new_id = match plan.glyph_map.get(&old) {
                Some(new) if *new == old => continue,
                Some(new) => format!("glyph{new}").into_bytes(),
                None => {
                    let mut candidate = [b".", id].concat();
                    let mut k = 1u32;
                    while ids.contains(candidate.as_slice()) {
                        candidate = [candidate.as_slice(), format!(".{k}").as_bytes()].concat();
                        k += 1;
                    }
                    candidate
                }
            };
            edits.insert(*value, new_id.clone());
            renamed.insert(id.to_vec(), new_id);
        }
        for e in &kept {
            let Some(el) = doc.elem(*e) else { continue };
            let Some((_, value)) = el.attrs.iter().find(|(n, _)| doc.text(*n) == b"xlink:href") else { continue };
            let href = doc.text(*value);
            if !href.starts_with(b"#glyph") {
                continue;
            }
            if let Some(new_id) = href.get(1..).and_then(|old| renamed.get(old)) {
                edits.insert(*value, [b"#", new_id.as_slice()].concat());
            }
        }
    }

    let mut out = Vec::with_capacity(src.len());
    write(doc, doc.root, &removed, &edits, &mut out);
    Ok(Some(out))
}

/// Marks the children of `i` that go; returns whether `i` stays (`subset_elements`).
fn prune(doc: &Doc<'_>, i: usize, keep: &HashSet<Vec<u8>>, removed: &mut [bool], depth: usize) -> bool {
    if doc.id(i).is_some_and(|id| keep.contains(id)) {
        return true;
    }
    let Some(e) = doc.elem(i) else { return false };
    let mut any = false;
    let mut tail_of_removed = false;
    for c in &e.children {
        match doc.nodes.get(*c) {
            Some((XNode::Text, _)) => {
                if tail_of_removed && let Some(r) = removed.get_mut(*c) {
                    *r = true;
                }
            }
            Some((XNode::Elem(_), _)) => {
                let stays = depth < MAX_SVG_DEPTH && prune(doc, *c, keep, removed, depth + 1);
                if let Some(r) = removed.get_mut(*c) {
                    *r = !stays;
                }
                tail_of_removed = !stays;
                any |= stays;
            }
            _ => {
                if let Some(r) = removed.get_mut(*c) {
                    *r = true;
                }
                tail_of_removed = true;
            }
        }
    }
    any
}

fn descendants_kept(doc: &Doc<'_>, i: usize, removed: &[bool], out: &mut Vec<usize>) {
    let mut stack = vec![i];
    while let Some(n) = stack.pop() {
        if let Some(e) = doc.elem(n) {
            for c in e.children.iter().rev() {
                if doc.elem(*c).is_some() && !removed.get(*c).copied().unwrap_or(true) {
                    out.push(*c);
                    stack.push(*c);
                }
            }
        }
    }
}

/// Writes element `i` and what stays below it, with the attribute value `edits` applied.
fn write(doc: &Doc<'_>, i: usize, removed: &[bool], edits: &HashMap<(usize, usize), Vec<u8>>, out: &mut Vec<u8>) {
    // An explicit stack: (node, next child to write).
    let mut stack: Vec<(usize, usize)> = vec![(i, 0)];
    while let Some((n, next)) = stack.pop() {
        let Some((node, range)) = doc.nodes.get(n) else { continue };
        let XNode::Elem(e) = node else {
            // Whitespace-only text goes, as the reference's parser drops blank text.
            let text = doc.text(*range);
            if !text.iter().all(u8::is_ascii_whitespace) {
                out.extend_from_slice(text);
            }
            continue;
        };
        if next == 0 {
            let mut at = e.start.0;
            let mut values: Vec<&(usize, usize)> = e.attrs.iter().map(|(_, v)| v).filter(|v| edits.contains_key(*v)).collect();
            values.sort();
            for v in values {
                out.extend_from_slice(doc.text((at, v.0)));
                out.extend_from_slice(edits.get(v).map_or(&[][..], Vec::as_slice));
                at = v.1;
            }
            out.extend_from_slice(doc.text((at, e.start.1)));
        }
        let child = e.children.iter().enumerate().skip(next).find(|(_, c)| !removed.get(**c).copied().unwrap_or(true));
        match child {
            Some((k, c)) => {
                stack.push((n, k + 1));
                stack.push((*c, 0));
            }
            None => out.extend_from_slice(doc.text(e.close)),
        }
    }
}
