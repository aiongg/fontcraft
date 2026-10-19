//! TrueType assembly as `ttx` writes it, assembled as fontTools' `ttProgram._assemble` does
//! (MIT): instructions `NAME[bits]`, numbers and `/* comments */`; a push writes the form its
//! mnemonic names, `PUSH` the shortest mix of forms, `INSTRnn` the byte `nn`.

use crate::{Result, TablesError};

use super::super::core::program::OPCODES;

enum Token<'a> {
    Instr(&'a str, &'a str),
    Number(i64),
    Comment,
}

fn syntax(at: &str) -> TablesError {
    let near: String = at.chars().take(15).collect();
    TablesError::invalid(format!("syntax error in TrueType assembly ({near})"))
}

/// The token at the start of `s` and the text after it.
fn token(s: &str) -> Result<(Token<'_>, &str)> {
    let b = s.as_bytes();
    if s.starts_with("/*") {
        let end = s.get(2..).and_then(|r| r.find("*/")).ok_or_else(|| syntax(s))?;
        return Ok((Token::Comment, s.get(end + 4..).unwrap_or_default()));
    }
    match b.first() {
        Some(c) if c.is_ascii_uppercase() => {
            let name_len = b.iter().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit()).count();
            let name = s.get(..name_len).unwrap_or_default();
            let after = s.get(name_len..).unwrap_or_default().trim_start();
            let args = after.strip_prefix('[').ok_or_else(|| syntax(s))?;
            let close = args.find(']').ok_or_else(|| syntax(s))?;
            Ok((Token::Instr(name, args.get(..close).unwrap_or_default()), args.get(close + 1..).unwrap_or_default()))
        }
        Some(c) if c.is_ascii_digit() || *c == b'-' => {
            let digits_from = usize::from(*c == b'-');
            let digits = b.iter().skip(digits_from).take_while(|c| c.is_ascii_digit()).count();
            if digits == 0 {
                return Err(syntax(s));
            }
            let end = digits_from + digits;
            let text = s.get(..end).unwrap_or_default();
            let n = text.parse::<i64>().map_err(|_| TablesError::invalid(format!("the number {text} in TrueType assembly is too large")))?;
            Ok((Token::Number(n), s.get(end..).unwrap_or_default()))
        }
        _ => Err(syntax(s)),
    }
}

fn mnemonic(name: &str) -> Option<(u8, u32)> {
    OPCODES.iter().find(|(_, m, _, _)| *m == name).map(|(op, _, bits, _)| (*op, *bits))
}

fn value_error(what: &str, v: i64) -> TablesError {
    TablesError::invalid(format!("{what} value out of range {v}"))
}

fn push_words(out: &mut Vec<u8>, values: &[i64]) -> Result<()> {
    for v in values {
        if !(-32768..32768).contains(v) {
            return Err(value_error("PUSHW", *v));
        }
        out.extend_from_slice(&(*v as i16).to_be_bytes());
    }
    Ok(())
}

fn push_bytes(out: &mut Vec<u8>, values: &[i64]) -> Result<()> {
    for v in values {
        out.push(u8::try_from(*v).map_err(|_| value_error("PUSHB", *v))?);
    }
    Ok(())
}

/// The bytecode of an assembly text.
pub(crate) fn assemble(text: &str) -> Result<Vec<u8>> {
    let joined = text.lines().collect::<Vec<_>>().join(" ");
    let mut out: Vec<u8> = Vec::new();
    let mut rest = joined.trim_start();
    while !rest.is_empty() {
        let (tok, after) = token(rest)?;
        rest = after.trim_start();
        let (name, arg) = match tok {
            Token::Comment => continue,
            Token::Number(_) => return Err(syntax(rest)),
            Token::Instr(name, arg) => (name, arg.trim()),
        };
        if let Some(code) = name.strip_prefix("INSTR") {
            let op = code.parse::<u8>().map_err(|_| TablesError::invalid(format!("the instruction {name} names no byte")))?;
            out.push(op);
            continue;
        }
        if !matches!(name, "PUSH" | "NPUSHB" | "NPUSHW" | "PUSHB" | "PUSHW") {
            let (op, bits) = mnemonic(name).ok_or_else(|| TablesError::invalid(format!("unknown TrueType instruction {name}")))?;
            if arg.len() != bits as usize {
                return Err(TablesError::invalid(format!("incorrect number of argument bits ({name}[{arg}])")));
            }
            let add = super::super::read::binary2num(arg);
            out.push(u8::try_from(u64::from(op) + add).map_err(|_| TablesError::invalid(format!("{name}[{arg}] is not an instruction")))?);
            continue;
        }
        let mut args: Vec<i64> = Vec::new();
        while !rest.is_empty() {
            match token(rest)? {
                (Token::Instr(..), _) => break,
                (Token::Comment, after) => rest = after.trim_start(),
                (Token::Number(n), after) => {
                    if args.len() >= crate::limits::MAX_SFNT {
                        return Err(TablesError::TooLarge { what: "the push values", limit: crate::limits::MAX_SFNT });
                    }
                    args.push(n);
                    rest = after.trim_start();
                }
            }
        }
        if name == "PUSH" {
            push_shortest(&mut out, &args)?;
            continue;
        }
        let n = args.len();
        let (base, words) = match name {
            "NPUSHB" => (0x40u8, false),
            "NPUSHW" => (0x41, true),
            "PUSHB" => (0xb0, false),
            _ => (0xb8, true),
        };
        if name.starts_with('N') {
            if n >= 256 {
                return Err(TablesError::invalid(format!("{name} with {n} values")));
            }
            out.push(base);
            out.push(n as u8);
        } else {
            if n > 8 {
                return Err(TablesError::invalid(format!("{name} with {n} values")));
            }
            // fontTools writes the form for zero values as the opcode before it.
            out.push((u16::from(base) + n as u16).wrapping_sub(1) as u8);
        }
        if words { push_words(&mut out, &args)? } else { push_bytes(&mut out, &args)? }
    }
    Ok(out)
}

/// `PUSH`: runs of byte values as `PUSHB`/`NPUSHB`, the rest as words, a lone byte between
/// words pushed as a word.
fn push_shortest(out: &mut Vec<u8>, values: &[i64]) -> Result<()> {
    let is_byte = |v: i64| (0..=255).contains(&v);
    let mut args = values;
    let mut n_words = 0usize;
    while !args.is_empty() {
        let n = args.len();
        while n_words < n && n_words < 255 && !args.get(n_words).is_some_and(|v| is_byte(*v)) {
            n_words += 1;
        }
        let mut n_bytes = 0usize;
        while n_words + n_bytes < n && n_bytes < 255 && args.get(n_words + n_bytes).is_some_and(|v| is_byte(*v)) {
            n_bytes += 1;
        }
        if n_bytes < 2 && n_words + n_bytes < 255 && n_words + n_bytes != n {
            n_words += n_bytes;
            continue;
        }
        if n_words > 0 {
            if n_words <= 8 {
                out.push(0xb8 + (n_words - 1) as u8);
            } else {
                out.push(0x41);
                out.push(n_words as u8);
            }
            push_words(out, args.get(..n_words).unwrap_or_default())?;
        }
        if n_bytes > 0 {
            if n_bytes <= 8 {
                out.push(0xb0 + (n_bytes - 1) as u8);
            } else {
                out.push(0x40);
                out.push(n_bytes as u8);
            }
            push_bytes(out, args.get(n_words..n_words + n_bytes).unwrap_or_default())?;
        }
        args = args.get(n_words + n_bytes..).unwrap_or_default();
        n_words = 0;
    }
    Ok(())
}
