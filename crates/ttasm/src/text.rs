//! Assembly text: the assembler and the disassembler (syntax in the crate docs).

use std::fmt::Write as _;

use crate::table::{self, EIF, ELSE, ENDF, FDEF, Head, IDEF, IF, NPUSHB, NPUSHW, PUSHB, PUSHW, PushForm};
use crate::{AsmError, Instruction, MAX_NESTING, MAX_PROGRAM_BYTES, MAX_PROGRAM_TEXT};

/// Longest run one push instruction carries.
const MAX_RUN: usize = 255;

fn syntax(line: usize, message: impl Into<String>) -> AsmError {
    AsmError::Syntax { line, message: message.into() }
}

/// A push whose values may continue on the following lines.
struct Pending {
    line: usize,
    head: Head,
    values: Vec<i64>,
}

pub(crate) fn assemble(text: &str) -> Result<Vec<u8>, AsmError> {
    let mut out = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut depth = 0usize;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let code = strip_comments(raw).map_err(|m| syntax(line, m))?;
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        if code.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+') {
            let Some(p) = pending.as_mut() else {
                return Err(syntax(line, "values outside a push instruction"));
            };
            parse_values(code, &mut p.values).map_err(|m| syntax(line, m))?;
            continue;
        }
        if let Some(p) = pending.take() {
            flush(p, &mut out)?;
        }
        let (head, rest) = table::parse_head(code).map_err(|m| syntax(line, m))?;
        match head {
            Head::Op(op) => {
                if !rest.trim().is_empty() {
                    return Err(syntax(line, format!("one instruction per line; found “{}” after it", rest.trim())));
                }
                match op {
                    IF | FDEF | IDEF => {
                        depth += 1;
                        if depth > MAX_NESTING {
                            return Err(syntax(line, format!("IF, FDEF and IDEF are nested more than {MAX_NESTING} deep")));
                        }
                    }
                    EIF | ENDF => depth = depth.saturating_sub(1),
                    _ => {}
                }
                out.push(op);
            }
            Head::Push { .. } | Head::Auto => {
                let mut values = Vec::new();
                parse_values(rest, &mut values).map_err(|m| syntax(line, m))?;
                pending = Some(Pending { line, head, values });
            }
        }
        if out.len() > MAX_PROGRAM_BYTES {
            return Err(AsmError::TooLarge { what: "bytes in a program", limit: MAX_PROGRAM_BYTES });
        }
    }
    if let Some(p) = pending.take() {
        flush(p, &mut out)?;
    }
    if out.len() > MAX_PROGRAM_BYTES {
        return Err(AsmError::TooLarge { what: "bytes in a program", limit: MAX_PROGRAM_BYTES });
    }
    Ok(out)
}

/// Removes `; …` and `/* … */` comments from one line.
fn strip_comments(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    loop {
        let semi = rest.find(';');
        let block = rest.find("/*");
        match (semi, block) {
            (Some(s), b) if b.is_none_or(|b| s < b) => {
                out.push_str(rest.get(..s).unwrap_or_default());
                return Ok(out);
            }
            (_, Some(b)) => {
                out.push_str(rest.get(..b).unwrap_or_default());
                out.push(' ');
                let after = rest.get(b + 2..).unwrap_or_default();
                let end = after.find("*/").ok_or("a “/*” comment is not closed on its line")?;
                rest = after.get(end + 2..).unwrap_or_default();
            }
            _ => {
                out.push_str(rest);
                return Ok(out);
            }
        }
    }
}

fn parse_values(text: &str, values: &mut Vec<i64>) -> Result<(), String> {
    for token in text.split_whitespace() {
        let v = token.parse::<i64>().map_err(|_| format!("“{token}” is not a whole number"))?;
        values.push(v);
    }
    Ok(())
}

fn flush(p: Pending, out: &mut Vec<u8>) -> Result<(), AsmError> {
    let n = p.values.len();
    match p.head {
        Head::Auto => auto_push(&p.values, out).map_err(|m| syntax(p.line, m)),
        Head::Push { words, counted, count } => {
            let op = if counted {
                if words { NPUSHW } else { NPUSHB }
            } else {
                let family = if words { "PUSHW" } else { "PUSHB" };
                if n == 0 || n > 8 {
                    return Err(syntax(p.line, format!("{family} carries 1 to 8 values, found {n}; use N{family} or PUSH")));
                }
                if let Some(c) = count
                    && c != n
                {
                    return Err(syntax(p.line, format!("{family}'s flags say {c} values, found {n}")));
                }
                let base = if words { PUSHW } else { PUSHB };
                base.saturating_add(u8::try_from(n - 1).unwrap_or(0))
            };
            let values = to_i32(&p.values).map_err(|m| syntax(p.line, m))?;
            encode_one(op, &values, out).map_err(|m| syntax(p.line, m))
        }
        Head::Op(_) => Ok(()),
    }
}

fn to_i32(values: &[i64]) -> Result<Vec<i32>, String> {
    values.iter().map(|v| i32::try_from(*v).map_err(|_| format!("{v} does not fit a push"))).collect()
}

fn is_byte(v: i64) -> bool {
    (0..=255).contains(&v)
}

/// Push cost in bytes of a run of `len` values.
fn run_cost(len: usize, words: bool) -> usize {
    let data = if words { 2 * len } else { len };
    if len <= 8 { 1 + data } else { 2 + data }
}

/// `PUSH`: splits the values into runs and gives each the shortest form, minimising the total
/// length (a run of bytes, or of words when a value does not fit a byte).
fn auto_push(values: &[i64], out: &mut Vec<u8>) -> Result<(), String> {
    if values.is_empty() {
        return Err("PUSH needs at least one value".into());
    }
    if let Some(v) = values.iter().find(|v| i16::try_from(**v).is_err()) {
        return Err(format!("{v} does not fit a push word (−32768 to 32767)"));
    }
    let n = values.len();
    // best[i]: (cost of the first i values, start of the last run, the last run is words).
    let mut best: Vec<(usize, usize, bool)> = vec![(usize::MAX, 0, false); n + 1];
    if let Some(first) = best.first_mut() {
        *first = (0, 0, false);
    }
    for i in 1..=n {
        let mut all_bytes = true;
        let mut choice = (usize::MAX, 0, false);
        for j in (i.saturating_sub(MAX_RUN)..i).rev() {
            all_bytes &= values.get(j).is_some_and(|v| is_byte(*v));
            let Some(&(before, _, _)) = best.get(j) else { continue };
            if before == usize::MAX {
                continue;
            }
            let len = i - j;
            if all_bytes {
                let c = before.saturating_add(run_cost(len, false));
                if c < choice.0 {
                    choice = (c, j, false);
                }
            }
            let c = before.saturating_add(run_cost(len, true));
            if c < choice.0 {
                choice = (c, j, true);
            }
        }
        if let Some(slot) = best.get_mut(i) {
            *slot = choice;
        }
    }
    let mut runs = Vec::new();
    let mut i = n;
    while i > 0 {
        let Some(&(_, start, words)) = best.get(i) else { break };
        if start >= i {
            break;
        }
        runs.push((start, i, words));
        i = start;
    }
    for (start, end, words) in runs.into_iter().rev() {
        let run = to_i32(values.get(start..end).unwrap_or_default())?;
        let len = run.len();
        let op = match (words, len <= 8) {
            (false, true) => PUSHB.saturating_add(u8::try_from(len - 1).unwrap_or(0)),
            (true, true) => PUSHW.saturating_add(u8::try_from(len - 1).unwrap_or(0)),
            (false, false) => NPUSHB,
            (true, false) => NPUSHW,
        };
        encode_one(op, &run, out)?;
    }
    Ok(())
}

/// Appends one instruction, checking that its inline data fits its form.
pub(crate) fn encode_one(op: u8, data: &[i32], out: &mut Vec<u8>) -> Result<(), String> {
    let name = || table::names().get(usize::from(op)).cloned().flatten().unwrap_or_else(|| format!("INSTR{op}[ ]"));
    let Some(PushForm { words, count }) = table::push_form(op) else {
        if !data.is_empty() {
            return Err(format!("{} carries no values", name()));
        }
        out.push(op);
        return Ok(());
    };
    match count {
        Some(c) if c != data.len() => return Err(format!("{} carries {c} values, found {}", name(), data.len())),
        None if data.len() > MAX_RUN => return Err(format!("{} carries at most {MAX_RUN} values, found {}", name(), data.len())),
        _ => {}
    }
    out.push(op);
    if count.is_none() {
        out.push(u8::try_from(data.len()).unwrap_or(u8::MAX));
    }
    for &v in data {
        if words {
            let w = i16::try_from(v).map_err(|_| format!("{v} does not fit a push word (−32768 to 32767)"))?;
            out.extend_from_slice(&w.to_be_bytes());
        } else {
            let b = u8::try_from(v).map_err(|_| format!("{v} does not fit a push byte (0 to 255)"))?;
            out.push(b);
        }
    }
    Ok(())
}

pub(crate) fn disassemble(instructions: &[Instruction]) -> Result<String, AsmError> {
    let mut text = String::new();
    let mut depth = 0usize;
    for ins in instructions {
        if matches!(ins.opcode, ELSE | EIF | ENDF) {
            depth = depth.saturating_sub(1);
        }
        for _ in 0..depth.min(MAX_NESTING) {
            text.push_str("  ");
        }
        match table::push_form(ins.opcode) {
            Some(form) => {
                let family = if form.words { "PUSHW" } else { "PUSHB" };
                text.push_str(if form.count.is_none() { "N" } else { "" });
                text.push_str(family);
                text.push_str("[ ]");
                for v in &ins.data {
                    let _ = write!(text, " {v}");
                }
            }
            None => match table::names().get(usize::from(ins.opcode)).and_then(Option::as_deref) {
                Some(name) => text.push_str(name),
                None => {
                    let _ = write!(text, "INSTR{}[ ]", ins.opcode);
                }
            },
        }
        text.push('\n');
        if matches!(ins.opcode, IF | FDEF | IDEF | ELSE) {
            depth += 1;
        }
        if text.len() > MAX_PROGRAM_TEXT {
            return Err(AsmError::TooLarge { what: "bytes of assembly text", limit: MAX_PROGRAM_TEXT });
        }
    }
    Ok(text)
}
