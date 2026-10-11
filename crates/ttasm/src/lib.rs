//! TrueType bytecode (L0, standalone): an assembler, a disassembler, a decoder and a static
//! analysis of stack depth and resource use. Plan: `docs/plans/M10.md` § 3 and § 4 › M10.16.
//! Rows: `compile.instructionBytes`, the `fpgm` and `prep` table rows (assembly half),
//! `hinting.tt.param.controlValueProgram` (the text form).
//!
//! | Item | Package | What it gives |
//! |---|---|---|
//! | [`decode`], [`encode`] | M10.16 | bytes ↔ [`Instruction`]s, inline push data included |
//! | [`disassemble`], [`assemble`] | M10.16 | bytes ↔ assembly text |
//! | [`mnemonic`], [`opcode`] | M10.16 | the opcode table |
//! | [`FunctionTable::from_fpgm`], [`analyse`] | M10.16 | the `maxp` limits a program needs |
//!
//! Contract:
//! - The text syntax is our own: one instruction per line, mnemonics as fontTools prints them
//!   (`SVTCA[0]`, `MIRP[01101]`, `PUSHB[ ]` followed by its values on the same line), `;` starts
//!   a comment, blank lines are ignored. No corpus source holds a Table Assembly parameter that
//!   would show the reference's syntax (guess on `hinting.tt.param.controlValueProgram`).
//! - [`assemble`] picks the shortest push form for a run of values; [`disassemble`] followed by
//!   [`assemble`] gives back the same bytes for every program, including push forms that are
//!   not the shortest, because the text keeps the form it was decoded from.
//! - Every input is untrusted. Text over [`MAX_PROGRAM_TEXT`] bytes, programs over
//!   [`MAX_PROGRAM_BYTES`], `IF`/`FDEF`/`IDEF` nesting deeper than [`MAX_NESTING`] and truncated
//!   push data are errors with a line number or byte offset, never a panic.
//! - [`analyse`] is static: it follows both branches of every `IF`, counts function calls through
//!   the [`FunctionTable`] (recursion stops at [`MAX_CALL_DEPTH`]) and reports the largest stack,
//!   storage index, twilight point and function number it can see. Values computed at run time
//!   that it cannot follow make it report [`ProgramLimits::uncertain`].
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// Longest assembly text accepted, in bytes (1 MB, `docs/plans/M10.md` § 3 › Limits).
pub const MAX_PROGRAM_TEXT: usize = 1 << 20;
/// Longest program accepted or produced, in bytes (`fpgm`, `prep`).
pub const MAX_PROGRAM_BYTES: usize = 1 << 20;
/// Longest glyph program: the `glyf` instruction length field is 16 bits.
pub const MAX_GLYPH_PROGRAM_BYTES: usize = 65_535;
/// Deepest nesting of `IF`, `FDEF` and `IDEF`.
pub const MAX_NESTING: usize = 64;
/// Deepest chain of `CALL`/`LOOPCALL` that [`analyse`] follows.
pub const MAX_CALL_DEPTH: usize = 64;
/// Most functions a [`FunctionTable`] holds (`maxp.maxFunctionDefs` is 16 bits).
pub const MAX_FUNCTIONS: usize = 65_535;

/// Why a program could not be assembled, decoded or analysed.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum AsmError {
    /// Assembly text that cannot be read; `line` counts from 1.
    #[error("line {line}: {message}")]
    Syntax { line: usize, message: String },
    /// Bytes that end inside an instruction's inline data.
    #[error("the program ends inside the instruction at byte {offset}")]
    Truncated { offset: usize },
    /// A byte that is no TrueType instruction.
    #[error("byte {offset} holds 0x{opcode:02X}, which is no instruction")]
    UnknownOpcode { offset: usize, opcode: u8 },
    /// `IF` without `EIF`, `ENDF` without `FDEF` and the like.
    #[error("unbalanced {what} at byte {offset}")]
    Unbalanced { what: &'static str, offset: usize },
    #[error("more than {limit} {what}")]
    TooLarge { what: &'static str, limit: usize },
    /// A function of this crate whose M10 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}

/// One decoded instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    /// Byte offset of the opcode in the program.
    pub offset: usize,
    pub opcode: u8,
    /// The values a push instruction carries inline (`NPUSHB`, `NPUSHW`, `PUSHB[n]`, `PUSHW[n]`),
    /// sign-extended for words; empty for every other instruction.
    pub data: Vec<i32>,
}

impl Instruction {
    /// The instruction's length in bytes, inline data included.
    pub fn len(&self) -> usize {
        // M10.16: count the count byte of NPUSHB/NPUSHW and two bytes per word.
        1
    }

    /// Always false: an instruction has at least its opcode byte.
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// The mnemonic of `opcode` as fontTools prints it (`MDRP[10110]`), or `None` for an unused byte.
pub fn mnemonic(opcode: u8) -> Option<&'static str> {
    // M10.16: the opcode table.
    let _ = opcode;
    None
}

/// The opcode of a mnemonic as [`mnemonic`] prints it.
pub fn opcode(mnemonic: &str) -> Option<u8> {
    // M10.16: the opcode table.
    let _ = mnemonic;
    None
}

/// Decodes a program into instructions, inline data attached.
pub fn decode(bytes: &[u8]) -> Result<Vec<Instruction>, AsmError> {
    if bytes.len() > MAX_PROGRAM_BYTES {
        return Err(AsmError::TooLarge { what: "bytes in a program", limit: MAX_PROGRAM_BYTES });
    }
    // M10.16: decode.
    Err(AsmError::NotYetImplemented("decoding TrueType instructions"))
}

/// Encodes instructions as bytes, each push in the form its opcode names.
pub fn encode(instructions: &[Instruction]) -> Result<Vec<u8>, AsmError> {
    // M10.16: encode.
    let _ = instructions;
    Err(AsmError::NotYetImplemented("encoding TrueType instructions"))
}

/// Assembles text (syntax in the crate docs) into bytes.
pub fn assemble(text: &str) -> Result<Vec<u8>, AsmError> {
    if text.len() > MAX_PROGRAM_TEXT {
        return Err(AsmError::TooLarge { what: "bytes of assembly text", limit: MAX_PROGRAM_TEXT });
    }
    // M10.16: the assembler.
    Err(AsmError::NotYetImplemented("the TrueType assembler"))
}

/// Disassembles bytes into text that [`assemble`] turns back into the same bytes.
pub fn disassemble(bytes: &[u8]) -> Result<String, AsmError> {
    // M10.16: the disassembler.
    let _ = decode(bytes)?;
    Err(AsmError::NotYetImplemented("the TrueType disassembler"))
}

/// What one function defined in `fpgm` needs when called.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FunctionInfo {
    /// Byte offset of the function's `FDEF` in `fpgm`.
    pub offset: usize,
    /// Values the function pops beyond those it pushes, when known.
    pub stack_effect: Option<i32>,
    /// The deepest stack the function reaches above its entry depth.
    pub max_stack: u32,
}

/// The functions `fpgm` defines, by number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunctionTable {
    pub functions: BTreeMap<u32, FunctionInfo>,
}

impl FunctionTable {
    /// Reads the `FDEF`s of an `fpgm` program (at most [`MAX_FUNCTIONS`]).
    pub fn from_fpgm(fpgm: &[u8]) -> Result<Self, AsmError> {
        // M10.16: walk the FDEFs and analyse each body.
        let _ = decode(fpgm)?;
        Ok(Self::default())
    }
}

/// The `maxp` values a program needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProgramLimits {
    pub max_stack_elements: u16,
    pub max_function_defs: u16,
    pub max_instruction_defs: u16,
    pub max_storage: u16,
    pub max_twilight_points: u16,
    /// 2 when the program uses the twilight zone, else 1.
    pub max_zones: u16,
    pub max_size_of_instructions: u16,
    /// A value the analysis could not follow (a computed index or call); the limits are a lower
    /// bound then.
    pub uncertain: bool,
}

impl ProgramLimits {
    /// The larger of each value, for combining the limits of several programs.
    pub fn merge(&mut self, other: &ProgramLimits) {
        self.max_stack_elements = self.max_stack_elements.max(other.max_stack_elements);
        self.max_function_defs = self.max_function_defs.max(other.max_function_defs);
        self.max_instruction_defs = self.max_instruction_defs.max(other.max_instruction_defs);
        self.max_storage = self.max_storage.max(other.max_storage);
        self.max_twilight_points = self.max_twilight_points.max(other.max_twilight_points);
        self.max_zones = self.max_zones.max(other.max_zones);
        self.max_size_of_instructions = self.max_size_of_instructions.max(other.max_size_of_instructions);
        self.uncertain |= other.uncertain;
    }
}

/// Analyses one program (`fpgm`, `prep` or a glyph program) against the functions `fpgm`
/// defines: stack depth, function and storage use, twilight points.
pub fn analyse(program: &[u8], functions: &FunctionTable) -> Result<ProgramLimits, AsmError> {
    // M10.16: the static analysis.
    let _ = (decode(program)?, functions);
    Err(AsmError::NotYetImplemented("TrueType program analysis"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_text_is_refused_before_any_work() {
        let text = " ".repeat(MAX_PROGRAM_TEXT + 1);
        assert!(matches!(assemble(&text), Err(AsmError::TooLarge { .. })));
    }
}
