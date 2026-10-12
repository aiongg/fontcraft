//! The TrueType opcode table: mnemonics as fontTools prints them, with the number of flag bits
//! each instruction folds into its opcode. Facts from the TrueType instruction set specification.

use std::sync::OnceLock;

/// One instruction family: `base` is the opcode with every flag bit clear.
pub(crate) struct Op {
    pub name: &'static str,
    pub base: u8,
    pub bits: u8,
}

const fn op(name: &'static str, base: u8, bits: u8) -> Op {
    Op { name, base, bits }
}

/// Every instruction except the four push families (see [`push_form`]).
pub(crate) const OPS: &[Op] = &[
    op("SVTCA", 0x00, 1),
    op("SPVTCA", 0x02, 1),
    op("SFVTCA", 0x04, 1),
    op("SPVTL", 0x06, 1),
    op("SFVTL", 0x08, 1),
    op("SPVFS", 0x0A, 0),
    op("SFVFS", 0x0B, 0),
    op("GPV", 0x0C, 0),
    op("GFV", 0x0D, 0),
    op("SFVTPV", 0x0E, 0),
    op("ISECT", 0x0F, 0),
    op("SRP0", 0x10, 0),
    op("SRP1", 0x11, 0),
    op("SRP2", 0x12, 0),
    op("SZP0", 0x13, 0),
    op("SZP1", 0x14, 0),
    op("SZP2", 0x15, 0),
    op("SZPS", 0x16, 0),
    op("SLOOP", 0x17, 0),
    op("RTG", 0x18, 0),
    op("RTHG", 0x19, 0),
    op("SMD", 0x1A, 0),
    op("ELSE", 0x1B, 0),
    op("JMPR", 0x1C, 0),
    op("SCVTCI", 0x1D, 0),
    op("SSWCI", 0x1E, 0),
    op("SSW", 0x1F, 0),
    op("DUP", 0x20, 0),
    op("POP", 0x21, 0),
    op("CLEAR", 0x22, 0),
    op("SWAP", 0x23, 0),
    op("DEPTH", 0x24, 0),
    op("CINDEX", 0x25, 0),
    op("MINDEX", 0x26, 0),
    op("ALIGNPTS", 0x27, 0),
    op("UTP", 0x29, 0),
    op("LOOPCALL", 0x2A, 0),
    op("CALL", 0x2B, 0),
    op("FDEF", 0x2C, 0),
    op("ENDF", 0x2D, 0),
    op("MDAP", 0x2E, 1),
    op("IUP", 0x30, 1),
    op("SHP", 0x32, 1),
    op("SHC", 0x34, 1),
    op("SHZ", 0x36, 1),
    op("SHPIX", 0x38, 0),
    op("IP", 0x39, 0),
    op("MSIRP", 0x3A, 1),
    op("ALIGNRP", 0x3C, 0),
    op("RTDG", 0x3D, 0),
    op("MIAP", 0x3E, 1),
    op("WS", 0x42, 0),
    op("RS", 0x43, 0),
    op("WCVTP", 0x44, 0),
    op("RCVT", 0x45, 0),
    op("GC", 0x46, 1),
    op("SCFS", 0x48, 0),
    op("MD", 0x49, 1),
    op("MPPEM", 0x4B, 0),
    op("MPS", 0x4C, 0),
    op("FLIPON", 0x4D, 0),
    op("FLIPOFF", 0x4E, 0),
    op("DEBUG", 0x4F, 0),
    op("LT", 0x50, 0),
    op("LTEQ", 0x51, 0),
    op("GT", 0x52, 0),
    op("GTEQ", 0x53, 0),
    op("EQ", 0x54, 0),
    op("NEQ", 0x55, 0),
    op("ODD", 0x56, 0),
    op("EVEN", 0x57, 0),
    op("IF", 0x58, 0),
    op("EIF", 0x59, 0),
    op("AND", 0x5A, 0),
    op("OR", 0x5B, 0),
    op("NOT", 0x5C, 0),
    op("DELTAP1", 0x5D, 0),
    op("SDB", 0x5E, 0),
    op("SDS", 0x5F, 0),
    op("ADD", 0x60, 0),
    op("SUB", 0x61, 0),
    op("DIV", 0x62, 0),
    op("MUL", 0x63, 0),
    op("ABS", 0x64, 0),
    op("NEG", 0x65, 0),
    op("FLOOR", 0x66, 0),
    op("CEILING", 0x67, 0),
    op("ROUND", 0x68, 2),
    op("NROUND", 0x6C, 2),
    op("WCVTF", 0x70, 0),
    op("DELTAP2", 0x71, 0),
    op("DELTAP3", 0x72, 0),
    op("DELTAC1", 0x73, 0),
    op("DELTAC2", 0x74, 0),
    op("DELTAC3", 0x75, 0),
    op("SROUND", 0x76, 0),
    op("S45ROUND", 0x77, 0),
    op("JROT", 0x78, 0),
    op("JROF", 0x79, 0),
    op("ROFF", 0x7A, 0),
    op("RUTG", 0x7C, 0),
    op("RDTG", 0x7D, 0),
    op("SANGW", 0x7E, 0),
    op("AA", 0x7F, 0),
    op("FLIPPT", 0x80, 0),
    op("FLIPRGON", 0x81, 0),
    op("FLIPRGOFF", 0x82, 0),
    op("SCANCTRL", 0x85, 0),
    op("SDPVTL", 0x86, 1),
    op("GETINFO", 0x88, 0),
    op("IDEF", 0x89, 0),
    op("ROLL", 0x8A, 0),
    op("MAX", 0x8B, 0),
    op("MIN", 0x8C, 0),
    op("SCANTYPE", 0x8D, 0),
    op("INSTCTRL", 0x8E, 0),
    op("GETVARIATION", 0x91, 0),
    op("MDRP", 0xC0, 5),
    op("MIRP", 0xE0, 5),
];

/// Opcodes of the instructions that open and close blocks.
pub(crate) const IF: u8 = 0x58;
pub(crate) const ELSE: u8 = 0x1B;
pub(crate) const EIF: u8 = 0x59;
pub(crate) const FDEF: u8 = 0x2C;
pub(crate) const IDEF: u8 = 0x89;
pub(crate) const ENDF: u8 = 0x2D;
pub(crate) const NPUSHB: u8 = 0x40;
pub(crate) const NPUSHW: u8 = 0x41;
pub(crate) const PUSHB: u8 = 0xB0;
pub(crate) const PUSHW: u8 = 0xB8;

/// The shape of a push instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PushForm {
    /// Two bytes per value, sign-extended.
    pub words: bool,
    /// The value count folded into the opcode (`PUSHB`, `PUSHW`); `None` when a count byte follows
    /// the opcode (`NPUSHB`, `NPUSHW`).
    pub count: Option<usize>,
}

/// The push form of `opcode`, or `None` for every other instruction.
pub(crate) fn push_form(opcode: u8) -> Option<PushForm> {
    match opcode {
        NPUSHB => Some(PushForm { words: false, count: None }),
        NPUSHW => Some(PushForm { words: true, count: None }),
        0xB0..=0xB7 => Some(PushForm { words: false, count: Some(usize::from(opcode - PUSHB) + 1) }),
        0xB8..=0xBF => Some(PushForm { words: true, count: Some(usize::from(opcode - PUSHW) + 1) }),
        _ => None,
    }
}

/// The family `opcode` belongs to, with its flag value.
pub(crate) fn lookup(opcode: u8) -> Option<(&'static Op, u8)> {
    OPS.iter().find_map(|o| {
        let flags = opcode.checked_sub(o.base)?;
        (u16::from(flags) < (1u16 << o.bits)).then_some((o, flags))
    })
}

/// The printed mnemonic of every byte, built once.
pub(crate) fn names() -> &'static [Option<String>] {
    static NAMES: OnceLock<Vec<Option<String>>> = OnceLock::new();
    NAMES.get_or_init(|| {
        (0..=255u8)
            .map(|b| {
                if let Some(form) = push_form(b) {
                    let family = if form.words { "PUSHW" } else { "PUSHB" };
                    return Some(match form.count {
                        None => format!("N{family}[ ]"),
                        Some(n) => format!("{family}[{:03b}]", n.saturating_sub(1)),
                    });
                }
                let (o, flags) = lookup(b)?;
                Some(if o.bits == 0 { format!("{}[ ]", o.name) } else { format!("{}[{:0width$b}]", o.name, flags, width = usize::from(o.bits)) })
            })
            .collect()
    })
}

/// What a line's leading token names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Head {
    /// A single opcode, flags included.
    Op(u8),
    /// A push of the given form; for `PUSHB`/`PUSHW` the count comes from the values unless the
    /// brackets give it.
    Push { words: bool, counted: bool, count: Option<usize> },
    /// `PUSH`: the assembler picks the shortest forms.
    Auto,
}

/// Reads the instruction at the start of `code` (already free of comments and leading space)
/// and returns it with the rest of the line.
pub(crate) fn parse_head(code: &str) -> Result<(Head, &str), String> {
    let end = code.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(code.len());
    let raw_name = code.get(..end).unwrap_or_default();
    if raw_name.is_empty() || !raw_name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        let shown: String = code.chars().take(20).collect();
        return Err(format!("expected an instruction, found “{shown}”"));
    }
    let name = raw_name.to_ascii_uppercase();
    let after = code.get(end..).unwrap_or_default().trim_start();
    let (flags, rest) = match after.strip_prefix('[') {
        Some(inner) => {
            let close = inner.find(']').ok_or_else(|| format!("{name}: the flags have no closing “]”"))?;
            (inner.get(..close).unwrap_or_default().trim(), inner.get(close + 1..).unwrap_or_default())
        }
        None => ("", after),
    };
    let no_flags = |head: Head| if flags.is_empty() { Ok((head, rest)) } else { Err(format!("{name} takes no flags, found “{flags}”")) };
    match name.as_str() {
        "PUSH" => no_flags(Head::Auto),
        "NPUSHB" => no_flags(Head::Push { words: false, counted: true, count: None }),
        "NPUSHW" => no_flags(Head::Push { words: true, counted: true, count: None }),
        "PUSHB" | "PUSHW" => {
            let words = name == "PUSHW";
            if flags.is_empty() {
                return Ok((Head::Push { words, counted: false, count: None }, rest));
            }
            let n = parse_bits(flags, 3).ok_or_else(|| format!("{name}: the flags must be 3 binary digits, found “{flags}”"))?;
            Ok((Head::Push { words, counted: false, count: Some(usize::from(n) + 1) }, rest))
        }
        _ => {
            if let Some(digits) = name.strip_prefix("INSTR")
                && !digits.is_empty()
                && digits.chars().all(|c| c.is_ascii_digit())
            {
                let byte = digits.parse::<u8>().map_err(|_| format!("{name}: {digits} is not a byte"))?;
                if lookup(byte).is_some() || push_form(byte).is_some() {
                    return Err(format!("{name}: byte {byte} is a named instruction; use its mnemonic"));
                }
                return no_flags(Head::Op(byte));
            }
            let o = OPS.iter().find(|o| o.name == name).ok_or_else(|| format!("unknown instruction “{raw_name}”"))?;
            if o.bits == 0 {
                return no_flags(Head::Op(o.base));
            }
            let value = parse_bits(flags, o.bits)
                .ok_or_else(|| format!("{name} needs {} binary digit{} of flags, found “{flags}”", o.bits, if o.bits == 1 { "" } else { "s" }))?;
            Ok((Head::Op(o.base.saturating_add(value)), rest))
        }
    }
}

/// A string of exactly `bits` binary digits.
fn parse_bits(flags: &str, bits: u8) -> Option<u8> {
    if flags.len() != usize::from(bits) || !flags.chars().all(|c| c == '0' || c == '1') {
        return None;
    }
    u8::from_str_radix(flags, 2).ok()
}
