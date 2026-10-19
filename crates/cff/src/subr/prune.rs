//! Removing the subroutines no glyph calls, and renumbering the calls that remain (M12.19b; row
//! `subset`). A port of `remove_unused_subroutines` in fontTools' `cffLib/transforms.py` (MIT
//! License, Copyright fontTools authors), which pyftsubset runs after a subset.
//!
//! Every glyph is run with its calls followed, as the reference's marking decompiler runs it, and
//! the subroutines reached are kept in their old order. Each call's subroutine number is then
//! written again for the new, smaller INDEX and its bias. The rest of every charstring keeps its
//! bytes, code after an `endchar` or `return` included.
//!
//! The reference renumbers the local calls inside a global subroutine through the one Private
//! DICT of the font. Here a global subroutine is renumbered in the Font DICT of each glyph that
//! calls it, so a font with several Font DICTs works as long as those numbers agree. Where they do
//! not, or where a call's number is not the literal written just before it, or a subroutine with a
//! hint mask is reached with different stem counts, nothing is renumbered: the function returns
//! [`SubrError::Unsupported`] and the caller keeps the subroutines as they were.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{CharstringSet, MAX_CALL_DEPTH, MAX_CHARSTRING, MAX_OPS_PER_GLYPH, MAX_OPS_PER_SET, SubrError, bias, push_int};

/// A charstring body: a glyph's own, a local subroutine (Font DICT, index) or a global one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Body {
    Glyph(usize),
    Local(usize, usize),
    Global(usize),
}

/// A subroutine a call reaches: local in a Font DICT, or global.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Local(usize, usize),
    Global(usize),
}

/// A call site: the subroutine number's bytes, and every subroutine it reached.
#[derive(Default)]
struct Site {
    len: usize,
    targets: BTreeSet<Target>,
}

#[derive(Default)]
struct BodyMarks {
    /// By the position of the number in the body.
    sites: BTreeMap<usize, Site>,
    /// The length of every hint mask, by the position of its operator.
    masks: BTreeMap<usize, usize>,
}

/// An operand: where its literal lies when it is one (body, start, length), and its integer
/// value.
#[derive(Clone, Copy)]
struct Slot {
    literal: Option<(Body, usize, usize)>,
    int: Option<i64>,
}

enum Flow {
    Go,
    Return,
    End,
}

struct Marker<'a> {
    set: &'a CharstringSet,
    bodies: HashMap<Body, BodyMarks>,
    global_used: BTreeSet<usize>,
    local_used: Vec<BTreeSet<usize>>,
    stack: Vec<Slot>,
    stems: usize,
    vsindex: usize,
    ops: usize,
    budget: usize,
}

/// The set without the subroutines no glyph reaches, every call renumbered for the INDEXes that
/// remain. A local Subrs INDEX left empty is dropped with its Private DICT entry when the table
/// is written.
pub fn prune_unused(set: &CharstringSet) -> Result<CharstringSet, SubrError> {
    set.validate()?;
    let fds = set.fd_count();
    let mut m = Marker {
        set,
        bodies: HashMap::new(),
        global_used: BTreeSet::new(),
        local_used: vec![BTreeSet::new(); fds],
        stack: Vec::new(),
        stems: 0,
        vsindex: 0,
        ops: 0,
        budget: MAX_OPS_PER_SET,
    };
    for (gid, code) in set.charstrings.iter().enumerate() {
        let fd = set.fd_of(gid);
        m.stack.clear();
        m.stems = 0;
        m.ops = 0;
        m.vsindex = set.fd_vsindex.get(fd).map_or(0, |&v| usize::from(v));
        m.run(code, Body::Glyph(gid), fd, 0).map_err(|e| in_glyph(e, gid))?;
    }

    let global_order: Vec<usize> = m.global_used.iter().copied().collect();
    let local_order: Vec<Vec<usize>> = m.local_used.iter().map(|u| u.iter().copied().collect()).collect();
    let global_new: HashMap<usize, usize> = global_order.iter().enumerate().map(|(new, old)| (*old, new)).collect();
    let local_new: Vec<HashMap<usize, usize>> = local_order.iter().map(|o| o.iter().enumerate().map(|(new, old)| (*old, new)).collect()).collect();
    let number = |t: &Target| -> Option<i64> {
        match *t {
            Target::Global(k) => Some(*global_new.get(&k)? as i64 - bias(global_order.len())),
            Target::Local(fd, k) => Some(*local_new.get(fd)?.get(&k)? as i64 - bias(local_order.get(fd)?.len())),
        }
    };
    let empty = BodyMarks::default();
    let rewrite = |body: Body, code: &[u8]| -> Result<Vec<u8>, SubrError> {
        let marks = m.bodies.get(&body).unwrap_or(&empty);
        let mut out = Vec::with_capacity(code.len());
        let mut copied = 0usize;
        for (&at, site) in &marks.sites {
            let mut numbers = site.targets.iter().map(number);
            let first = numbers.next().flatten().ok_or_else(|| SubrError::Invalid("a call site without a subroutine".into()))?;
            if numbers.any(|n| n != Some(first)) {
                return Err(SubrError::Unsupported(
                    "a global subroutine calls local subroutines that would be renumbered differently in different Font DICTs".into(),
                ));
            }
            out.extend_from_slice(code.get(copied..at).ok_or_else(|| SubrError::Invalid("a call site lies outside its charstring".into()))?);
            push_int(first, &mut out)?;
            copied = at + site.len;
        }
        out.extend_from_slice(code.get(copied..).unwrap_or_default());
        if out.len() > MAX_CHARSTRING {
            return Err(SubrError::TooLarge { what: "bytes in one charstring", limit: MAX_CHARSTRING });
        }
        Ok(out)
    };

    let charstrings = set.charstrings.iter().enumerate().map(|(g, c)| rewrite(Body::Glyph(g), c)).collect::<Result<Vec<_>, _>>()?;
    let global_subrs = global_order
        .iter()
        .map(|&k| rewrite(Body::Global(k), set.global_subrs.get(k).map(Vec::as_slice).unwrap_or_default()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut local_subrs = Vec::with_capacity(fds);
    for (fd, order) in local_order.iter().enumerate() {
        let subrs = set.local_subrs.get(fd).map(Vec::as_slice).unwrap_or_default();
        local_subrs.push(
            order.iter().map(|&k| rewrite(Body::Local(fd, k), subrs.get(k).map(Vec::as_slice).unwrap_or_default())).collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(CharstringSet { charstrings, global_subrs, local_subrs, ..set.clone() })
}

fn in_glyph(e: SubrError, gid: usize) -> SubrError {
    match e {
        SubrError::Invalid(m) => SubrError::Invalid(format!("glyph {gid}: {m}")),
        SubrError::Unsupported(m) => SubrError::Unsupported(format!("glyph {gid}: {m}")),
        other => other,
    }
}

fn unsupported(what: &str) -> SubrError {
    SubrError::Unsupported(format!("{what}, so the subroutines were kept as they were"))
}

impl Marker<'_> {
    fn tick(&mut self) -> Result<(), SubrError> {
        self.ops += 1;
        if self.ops > MAX_OPS_PER_GLYPH {
            return Err(SubrError::TooLarge { what: "operators run by one glyph", limit: MAX_OPS_PER_GLYPH });
        }
        self.budget = self.budget.checked_sub(1).ok_or(SubrError::TooLarge { what: "operators run by all glyphs", limit: MAX_OPS_PER_SET })?;
        Ok(())
    }

    fn mark_mask(&mut self, body: Body, at: usize, len: usize) -> Result<(), SubrError> {
        let marks = self.bodies.entry(body).or_default();
        match marks.masks.get(&at) {
            Some(&seen) if seen != len => Err(unsupported("a subroutine with a hint mask is called with different stem counts")),
            Some(_) => Ok(()),
            None => {
                marks.masks.insert(at, len);
                Ok(())
            }
        }
    }

    fn run(&mut self, code: &[u8], body: Body, fd: usize, level: usize) -> Result<Flow, SubrError> {
        let set: &CharstringSet = self.set;
        let cff2 = set.cff2;
        let mut i = 0usize;
        while let Some(&b0) = code.get(i) {
            self.tick()?;
            if b0 == 28 || b0 >= 32 {
                let (len, int) = number(code, i)?;
                if self.stack.len() >= set.max_stack {
                    return Err(SubrError::Invalid(format!("more than {} operands on the stack", set.max_stack)));
                }
                self.stack.push(Slot { literal: Some((body, i, len)), int });
                i += len;
                continue;
            }
            match b0 {
                10 | 29 => {
                    let slot = self.stack.pop().ok_or_else(|| SubrError::Invalid("a subroutine call with an empty operand stack".into()))?;
                    let n = slot.int.ok_or_else(|| SubrError::Invalid("a subroutine number that is not an integer".into()))?;
                    let (at, len) = match slot.literal {
                        Some((b, at, len)) if b == body && at + len == i => (at, len),
                        _ => return Err(unsupported("a subroutine number is not written just before its call")),
                    };
                    let local = b0 == 10;
                    let subrs: &[Vec<u8>] = if local { set.local_subrs.get(fd).map(Vec::as_slice).unwrap_or_default() } else { &set.global_subrs };
                    let index = n.checked_add(bias(subrs.len())).and_then(|v| usize::try_from(v).ok());
                    let what = if local { "local" } else { "global" };
                    let (k, subr) = index
                        .and_then(|k| subrs.get(k).map(|s| (k, s)))
                        .ok_or_else(|| SubrError::Invalid(format!("a call to {what} subroutine {n}, which does not exist")))?;
                    if level + 1 > MAX_CALL_DEPTH {
                        return Err(SubrError::Invalid(format!("subroutines nest more than {MAX_CALL_DEPTH} deep")));
                    }
                    let (target, callee) = if local {
                        if let Some(used) = self.local_used.get_mut(fd) {
                            used.insert(k);
                        }
                        (Target::Local(fd, k), Body::Local(fd, k))
                    } else {
                        self.global_used.insert(k);
                        (Target::Global(k), Body::Global(k))
                    };
                    let site = self.bodies.entry(body).or_default().sites.entry(at).or_default();
                    site.len = len;
                    site.targets.insert(target);
                    if let Flow::End = self.run(subr, callee, fd, level + 1)? {
                        return Ok(Flow::End);
                    }
                    i += 1;
                }
                11 if !cff2 => return Ok(Flow::Return),
                14 if !cff2 => return Ok(Flow::End),
                1 | 3 | 18 | 23 => {
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    i += 1;
                }
                19 | 20 => {
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    let n = self.stems.div_ceil(8);
                    if code.get(i + n).is_none() {
                        return Err(SubrError::Invalid("a hint mask is cut short".into()));
                    }
                    self.mark_mask(body, i, n)?;
                    i += 1 + n;
                }
                4..=8 | 21 | 22 | 24..=27 | 30 | 31 => {
                    self.stack.clear();
                    i += 1;
                }
                15 if cff2 => {
                    let slot = self.stack.last().copied().ok_or_else(|| SubrError::Invalid("vsindex with an empty operand stack".into()))?;
                    let v = slot.int.and_then(|v| usize::try_from(v).ok()).filter(|&v| v < set.regions.len().max(1));
                    self.vsindex = v.ok_or_else(|| SubrError::Invalid("a vsindex that names no item variation data".into()))?;
                    self.stack.clear();
                    i += 1;
                }
                16 if cff2 => {
                    let slot = self.stack.last().copied().ok_or_else(|| SubrError::Invalid("blend with an empty operand stack".into()))?;
                    let n = slot
                        .int
                        .and_then(|v| usize::try_from(v).ok())
                        .ok_or_else(|| SubrError::Invalid("a blend count that is not a whole number".into()))?;
                    let k = set.regions.get(self.vsindex).copied().unwrap_or(0);
                    let need = n.checked_mul(k + 1).and_then(|v| v.checked_add(1)).filter(|&v| v <= self.stack.len());
                    let need = need.ok_or_else(|| SubrError::Invalid("blend has too few operands".into()))?;
                    let keep = self.stack.len() - need;
                    self.stack.truncate(keep);
                    self.stack.extend(std::iter::repeat_n(Slot { literal: None, int: None }, n));
                    i += 1;
                }
                12 => {
                    let b1 = *code.get(i + 1).ok_or_else(|| SubrError::Invalid("an escaped operator is cut short".into()))?;
                    match b1 {
                        0 if !cff2 => {}
                        34..=37 => {}
                        _ => return Err(unsupported(&format!("charstring operator 12 {b1} is not followed"))),
                    }
                    self.stack.clear();
                    i += 2;
                }
                _ => return Err(unsupported(&format!("charstring operator {b0} is not followed"))),
            }
        }
        Ok(Flow::Go)
    }
}

/// The length of the number at `i` and its value when it is an integer.
fn number(code: &[u8], i: usize) -> Result<(usize, Option<i64>), SubrError> {
    let cut = || SubrError::Invalid("a number is cut short".into());
    let b0 = i64::from(*code.get(i).ok_or_else(cut)?);
    let byte = |k: usize| code.get(i + k).map(|&b| i64::from(b)).ok_or_else(cut);
    Ok(match b0 {
        28 => (3, Some(i64::from(((byte(1)? << 8) | byte(2)?) as u16 as i16))),
        32..=246 => (1, Some(b0 - 139)),
        247..=250 => (2, Some((b0 - 247) * 256 + byte(1)? + 108)),
        251..=254 => (2, Some(-(b0 - 251) * 256 - byte(1)? - 108)),
        _ => {
            let v = ((byte(1)? << 24) | (byte(2)? << 16) | (byte(3)? << 8) | byte(4)?) as u32 as i32;
            (5, (v & 0xFFFF == 0).then_some(i64::from(v >> 16)))
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn int(v: i64) -> Vec<u8> {
        let mut out = Vec::new();
        push_int(v, &mut out).unwrap();
        out
    }

    fn call(n: i64, op: u8) -> Vec<u8> {
        [int(n), vec![op]].concat()
    }

    /// A subroutine that draws one line and returns.
    fn line(dx: i64) -> Vec<u8> {
        [int(dx), int(0), vec![5, 11]].concat()
    }

    #[test]
    fn unused_subroutines_go_and_calls_are_renumbered() {
        // Local subroutines 0..4 and global 0..2; the glyphs reach local 1 and 3 and global 2,
        // and global 2 calls local 3.
        let locals: Vec<Vec<u8>> = (0..5).map(|k| line(10 + k)).collect();
        let mut locals = locals;
        let gsub2 = [call(3 - 107, 10), vec![11]].concat();
        let globals = vec![line(1), line(2), gsub2];
        let glyph0 = [int(0), int(0), vec![21], call(1 - 107, 10), vec![14]].concat();
        let glyph1 = [int(0), int(0), vec![21], call(2 - 107, 29), vec![14]].concat();
        locals[4] = [call(0 - 107, 10), vec![11]].concat(); // never reached: local 0 stays unused
        let set = CharstringSet {
            charstrings: vec![glyph0, glyph1],
            global_subrs: globals.clone(),
            local_subrs: vec![locals.clone()],
            ..CharstringSet::default()
        };
        let pruned = prune_unused(&set).unwrap();
        assert_eq!(pruned.global_subrs.len(), 1);
        assert_eq!(pruned.local_subrs, vec![vec![locals[1].clone(), locals[3].clone()]]);
        // The old global 2 is the new global 0; it calls the new local 1 (the old local 3).
        assert_eq!(pruned.global_subrs[0], [call(1 - 107, 10), vec![11]].concat());
        assert_eq!(pruned.charstrings[0], [int(0), int(0), vec![21], call(-107, 10), vec![14]].concat());
        assert_eq!(pruned.charstrings[1], [int(0), int(0), vec![21], call(-107, 29), vec![14]].concat());
        // Expanding both gives the same charstrings.
        assert_eq!(super::super::desubroutinize(&pruned).unwrap().charstrings, super::super::desubroutinize(&set).unwrap().charstrings);
    }

    #[test]
    fn a_bias_change_rewrites_the_numbers() {
        // 1,300 local subroutines (bias 1131), of which glyph 0 calls the last; after pruning one
        // is left (bias 107).
        let locals: Vec<Vec<u8>> = (0..1300).map(|k| line(k % 100)).collect();
        let glyph = [int(0), int(0), vec![21], call(1299 - 1131, 10), vec![14]].concat();
        let set = CharstringSet { charstrings: vec![glyph], local_subrs: vec![locals.clone()], ..CharstringSet::default() };
        let pruned = prune_unused(&set).unwrap();
        assert_eq!(pruned.local_subrs, vec![vec![locals[1299].clone()]]);
        assert_eq!(pruned.charstrings[0], [int(0), int(0), vec![21], call(-107, 10), vec![14]].concat());
    }

    #[test]
    fn a_computed_subroutine_number_keeps_the_subroutines() {
        // The number is pushed by the glyph and the call made inside subroutine 0.
        let locals = vec![vec![10, 11], line(5)];
        let glyph = [int(1 - 107), call(-107, 10), vec![14]].concat();
        let set = CharstringSet { charstrings: vec![glyph], local_subrs: vec![locals], ..CharstringSet::default() };
        assert!(matches!(prune_unused(&set), Err(SubrError::Unsupported(_))));
    }

    #[test]
    fn cff2_blends_and_hint_masks_are_followed() {
        // One region: `a b 1 blend` takes two operands and leaves one. The mask after two stems
        // is one byte long.
        let sub = [int(5), int(1), int(1), vec![16], int(0), vec![5]].concat();
        let glyph = [int(10), int(20), int(30), int(40), vec![18], vec![19, 0xC0], call(1 - 107, 10)].concat();
        let set = CharstringSet {
            cff2: true,
            charstrings: vec![glyph],
            local_subrs: vec![vec![line(1), sub.clone()]],
            regions: vec![1],
            max_stack: 193,
            ..CharstringSet::default()
        };
        let pruned = prune_unused(&set).unwrap();
        assert_eq!(pruned.local_subrs, vec![vec![sub]]);
        assert_eq!(pruned.charstrings[0], [int(10), int(20), int(30), int(40), vec![18], vec![19, 0xC0], call(-107, 10)].concat());
    }
}
