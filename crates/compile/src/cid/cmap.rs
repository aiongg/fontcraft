//! CMap resource files (row `compile.cidCmapResources`): the PostScript text that maps character
//! codes to CIDs (Adobe Technical Note #5099, "Building CMap Files for CID-Keyed Fonts").
//!
//! [`parse_cmap`] reads `CIDSystemInfo`'s `/Registry`, `/Ordering` and `/Supplement`,
//! `/CMapName`, and the `begincidrange` and `begincidchar` blocks. `usecmap` is reported, not
//! followed; `notdef` blocks and the code space are not needed and are skipped. The text is read
//! as a stream of tokens, never held as a token list.
//!
//! The file is untrusted: at most [`MAX_CMAP_BYTES`] bytes and [`MAX_CMAP_CODES`] codes (a range
//! counts every code it covers), codes of 1 to 4 bytes, CIDs up to
//! [`fontcraft_model::cid::MAX_CID`]. A malformed file is refused with a message.

use std::collections::BTreeMap;

use fontcraft_model::cid::MAX_CID;

/// Largest CMap file read, in bytes. Adobe's largest UTF-32 CMaps are a few megabytes.
pub const MAX_CMAP_BYTES: usize = 16 << 20;
/// Most codes one CMap file may map.
pub const MAX_CMAP_CODES: usize = 1 << 20;
/// Longest name, string or hex string token, in bytes.
const MAX_TOKEN: usize = 4_096;
/// Deepest nesting of parentheses inside a string.
const MAX_PAREN_DEPTH: usize = 64;

/// How the codes of a CMap are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeKind {
    /// Unicode: the CMap must be named `Uni…` (or be unnamed); codes are UTF-8 when the name says
    /// `UTF8`, UTF-16 when it says `UTF16`, and the code point itself otherwise (`UTF32`).
    Unicode,
    /// Codes of a legacy encoding (a Macintosh CMap), kept as the byte values.
    Bytes,
}

/// What a CMap file says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CMap {
    pub name: Option<String>,
    pub registry: Option<String>,
    pub ordering: Option<String>,
    pub supplement: Option<u32>,
    /// Code (a code point for [`CodeKind::Unicode`]) → CID; the first entry for a code counts.
    pub cids: BTreeMap<u32, u32>,
    /// The length in bytes of the longest code.
    pub longest_code: usize,
    /// The CMap this one names with `usecmap`, which is not read.
    pub uses: Option<String>,
}

impl CMap {
    /// CID → its codes, ascending.
    pub fn codes_by_cid(&self) -> BTreeMap<u32, Vec<u32>> {
        let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for (code, cid) in &self.cids {
            out.entry(*cid).or_default().push(*code);
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    Hex(Vec<u8>),
    Text(String),
    Int(i64),
    Word(String),
}

/// The tokens of PostScript text, one at a time; `%` comments are skipped.
struct Tokens<'a> {
    s: &'a [u8],
    at: usize,
}

fn is_delimiter(b: u8) -> bool {
    b.is_ascii_whitespace() || b"()<>[]{}/%".contains(&b)
}

impl Tokens<'_> {
    fn next_token(&mut self) -> Result<Option<Token>, String> {
        loop {
            let Some(&b) = self.s.get(self.at) else { return Ok(None) };
            if b.is_ascii_whitespace() || b == 0 {
                self.at += 1;
            } else if b == b'%' {
                while self.s.get(self.at).is_some_and(|c| *c != b'\n' && *c != b'\r') {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        let start = self.at;
        let Some(&b) = self.s.get(start) else { return Ok(None) };
        self.at += 1;
        match b {
            b'/' => {
                let word = self.word();
                Ok(Some(Token::Name(word)))
            }
            b'<' if self.s.get(self.at) == Some(&b'<') => {
                self.at += 1;
                Ok(Some(Token::Word("<<".into())))
            }
            b'>' if self.s.get(self.at) == Some(&b'>') => {
                self.at += 1;
                Ok(Some(Token::Word(">>".into())))
            }
            b'<' => {
                let mut digits = Vec::new();
                loop {
                    let Some(&c) = self.s.get(self.at) else { return Err("a hex string is not closed".into()) };
                    self.at += 1;
                    match c {
                        b'>' => break,
                        c if c.is_ascii_whitespace() => {}
                        c if c.is_ascii_hexdigit() => {
                            if digits.len() >= 2 * MAX_TOKEN {
                                return Err("a hex string is too long".into());
                            }
                            digits.push(c);
                        }
                        c => return Err(format!("a hex string holds `{}`", char::from(c))),
                    }
                }
                if digits.len() % 2 == 1 {
                    digits.push(b'0');
                }
                let value = |c: u8| char::from(c).to_digit(16).unwrap_or(0) as u8;
                Ok(Some(Token::Hex(digits.chunks(2).map(|p| p.iter().fold(0u8, |v, c| (v << 4) | value(*c))).collect())))
            }
            b'(' => {
                let mut depth = 1usize;
                let mut text = Vec::new();
                loop {
                    let Some(&c) = self.s.get(self.at) else { return Err("a string is not closed".into()) };
                    self.at += 1;
                    match c {
                        b'\\' => {
                            if let Some(&e) = self.s.get(self.at) {
                                self.at += 1;
                                text.push(e);
                            }
                        }
                        b'(' => {
                            depth += 1;
                            if depth > MAX_PAREN_DEPTH {
                                return Err("a string nests its parentheses too deeply".into());
                            }
                            text.push(c);
                        }
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            text.push(c);
                        }
                        c => text.push(c),
                    }
                    if text.len() > MAX_TOKEN {
                        return Err("a string is too long".into());
                    }
                }
                Ok(Some(Token::Text(String::from_utf8_lossy(&text).into_owned())))
            }
            b'[' | b']' | b'{' | b'}' | b'>' | b')' => Ok(Some(Token::Word(char::from(b).to_string()))),
            _ => {
                self.at = start;
                let word = self.word();
                Ok(Some(match word.parse::<i64>() {
                    Ok(i) => Token::Int(i),
                    Err(_) => Token::Word(word),
                }))
            }
        }
    }

    /// Characters up to the next delimiter, at most [`MAX_TOKEN`] of them kept.
    fn word(&mut self) -> String {
        let start = self.at;
        while self.s.get(self.at).is_some_and(|c| !is_delimiter(*c)) {
            self.at += 1;
        }
        let end = self.at.min(start + MAX_TOKEN);
        String::from_utf8_lossy(self.s.get(start..end).unwrap_or_default()).into_owned()
    }
}

/// How a Unicode CMap's codes become code points.
#[derive(Clone, Copy)]
enum Codes {
    Utf8,
    Utf16,
    Plain,
    Bytes,
}

fn value(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |v, x| (v << 8) | u32::from(*x))
}

fn decode(codes: Codes, bytes: &[u8]) -> Option<u32> {
    let cp = match codes {
        Codes::Bytes => return Some(value(bytes)),
        Codes::Utf8 => std::str::from_utf8(bytes).ok().and_then(|s| {
            let mut it = s.chars();
            let c = it.next()?;
            it.next().is_none().then_some(u32::from(c))
        })?,
        Codes::Utf16 => match bytes {
            [_, _] => value(bytes),
            [a, b, c, d] => {
                let (hi, lo) = (value(&[*a, *b]), value(&[*c, *d]));
                if !(0xD800..0xDC00).contains(&hi) || !(0xDC00..0xE000).contains(&lo) {
                    return None;
                }
                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
            }
            _ => return None,
        },
        Codes::Plain => value(bytes),
    };
    char::from_u32(cp).map(u32::from)
}

/// Which block the parser is in.
enum Block {
    None,
    Range(Vec<Token>),
    Char(Vec<Token>),
}

struct Collect {
    codes: Option<Codes>,
    kind: CodeKind,
    count: usize,
    out: CMap,
}

impl Collect {
    /// How codes are read, fixed by the name once the first mapping is met.
    fn codes(&mut self) -> Result<Codes, String> {
        if let Some(c) = self.codes {
            return Ok(c);
        }
        let c = match (self.kind, &self.out.name) {
            (CodeKind::Bytes, _) => Codes::Bytes,
            (CodeKind::Unicode, Some(n)) if !n.starts_with("Uni") => {
                return Err(format!("`{n}` is not a Unicode CMap; a Unicode CMap is named Uni…"));
            }
            (CodeKind::Unicode, Some(n)) if n.contains("UTF8") => Codes::Utf8,
            (CodeKind::Unicode, Some(n)) if n.contains("UTF16") => Codes::Utf16,
            (CodeKind::Unicode, _) => Codes::Plain,
        };
        self.codes = Some(c);
        Ok(c)
    }

    fn add(&mut self, code: &[u8], cid: i64) -> Result<(), String> {
        self.count += 1;
        if self.count > MAX_CMAP_CODES {
            return Err(format!("the CMap maps more than {MAX_CMAP_CODES} codes"));
        }
        let cid = u32::try_from(cid).ok().filter(|c| *c <= MAX_CID).ok_or_else(|| format!("the CID {cid} is not from 0 to {MAX_CID}"))?;
        let codes = self.codes()?;
        if let Some(c) = decode(codes, code) {
            self.out.cids.entry(c).or_insert(cid);
            self.out.longest_code = self.out.longest_code.max(code.len());
        }
        Ok(())
    }

    fn range(&mut self, lo: &[u8], hi: &[u8], start: i64) -> Result<(), String> {
        if lo.len() != hi.len() || lo.is_empty() || lo.len() > 4 {
            return Err("a cidrange's codes must be 1 to 4 bytes, both the same length".into());
        }
        let (a, b) = (value(lo), value(hi));
        if a > b {
            return Err(format!("the cidrange <{a:X}> <{b:X}> runs backwards"));
        }
        let n = lo.len();
        for code in a..=b {
            let bytes = code.to_be_bytes();
            let Some(code_bytes) = bytes.get(4 - n..) else { break };
            self.add(code_bytes, start.saturating_add(i64::from(code - a)))?;
        }
        Ok(())
    }
}

/// Reads a CMap file (see the module notes).
pub fn parse_cmap(bytes: &[u8], kind: CodeKind) -> Result<CMap, String> {
    if bytes.len() > MAX_CMAP_BYTES {
        return Err(format!("the file is {} bytes long; at most {MAX_CMAP_BYTES} are read", bytes.len()));
    }
    let mut toks = Tokens { s: bytes, at: 0 };
    let mut c = Collect { codes: None, kind, count: 0, out: CMap::default() };
    let mut block = Block::None;
    let mut prev: Option<Token> = None;
    while let Some(t) = toks.next_token()? {
        match &mut block {
            Block::Range(buf) => {
                if t == Token::Word("endcidrange".into()) {
                    if !buf.is_empty() {
                        return Err("a cidrange block holds something other than <code> <code> CID".into());
                    }
                    block = Block::None;
                    continue;
                }
                buf.push(t);
                if let [a, b, s] = buf.as_slice() {
                    let (Token::Hex(lo), Token::Hex(hi), Token::Int(start)) = (a, b, s) else {
                        return Err("a cidrange block holds something other than <code> <code> CID".into());
                    };
                    let (lo, hi, start) = (lo.clone(), hi.clone(), *start);
                    buf.clear();
                    c.range(&lo, &hi, start)?;
                }
                continue;
            }
            Block::Char(buf) => {
                if t == Token::Word("endcidchar".into()) {
                    if !buf.is_empty() {
                        return Err("a cidchar block holds something other than <code> CID".into());
                    }
                    block = Block::None;
                    continue;
                }
                buf.push(t);
                if let [a, b] = buf.as_slice() {
                    let (Token::Hex(code), Token::Int(cid)) = (a, b) else {
                        return Err("a cidchar block holds something other than <code> CID".into());
                    };
                    let (code, cid) = (code.clone(), *cid);
                    buf.clear();
                    c.add(&code, cid)?;
                }
                continue;
            }
            Block::None => {}
        }
        match (&prev, &t) {
            (Some(Token::Name(n)), Token::Text(s)) if n == "Registry" => c.out.registry = Some(s.clone()),
            (Some(Token::Name(n)), Token::Text(s)) if n == "Ordering" => c.out.ordering = Some(s.clone()),
            (Some(Token::Name(n)), Token::Int(v)) if n == "Supplement" => c.out.supplement = u32::try_from(*v).ok(),
            (Some(Token::Name(n)), Token::Name(v)) if n == "CMapName" && c.out.name.is_none() => c.out.name = Some(v.clone()),
            (Some(Token::Name(u)), Token::Word(w)) if w == "usecmap" => c.out.uses = Some(u.clone()),
            (_, Token::Word(w)) if w == "begincidrange" => block = Block::Range(Vec::new()),
            (_, Token::Word(w)) if w == "begincidchar" => block = Block::Char(Vec::new()),
            _ => {}
        }
        prev = Some(t);
    }
    if !matches!(block, Block::None) {
        return Err("a cidrange or cidchar block is not closed".into());
    }
    if c.out.cids.is_empty() && c.out.uses.is_none() {
        return Err("the CMap maps no codes to CIDs".into());
    }
    if c.codes.is_none() {
        c.codes()?;
    }
    Ok(c.out)
}
