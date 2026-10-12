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
//!   (`SVTCA[0]`, `MIRP[01101]`, `ADD[ ]`), `;` and `/* … */` start comments, blank lines and
//!   indentation are ignored, mnemonics may be in any case. A push names its form and lists its
//!   values after it: `PUSHB[ ] 1 2`, `NPUSHB[ ]`, `PUSHW[ ] -64`, `NPUSHW[ ] 1000`; values may go
//!   on the following lines, as fontTools writes them. `PUSH 1 2 1000` lets [`assemble`] choose.
//!   A byte that is no instruction is written `INSTR<byte>[ ]`. No corpus source holds a Table
//!   Assembly parameter that would show the reference's syntax (guess on
//!   `hinting.tt.param.controlValueProgram`).
//! - `PUSH` takes the shortest sequence of push forms for its values; [`disassemble`] followed by
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

mod analysis;
mod table;
mod text;

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
        match table::push_form(self.opcode) {
            None => 1,
            Some(form) => {
                let count_byte = usize::from(form.count.is_none());
                let per_value = if form.words { 2 } else { 1 };
                1 + count_byte + self.data.len().saturating_mul(per_value)
            }
        }
    }

    /// Always false: an instruction has at least its opcode byte.
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// The mnemonic of `opcode` as fontTools prints it (`MDRP[10110]`), or `None` for an unused byte.
/// The fixed-count pushes print their count less one as flags (`PUSHB[010]` pushes 3 bytes); in
/// assembly text they are written `PUSHB[ ]` and the count comes from the values.
pub fn mnemonic(opcode: u8) -> Option<&'static str> {
    table::names().get(usize::from(opcode)).and_then(Option::as_deref)
}

/// The opcode of a mnemonic as [`mnemonic`] prints it. Flags must be complete (`SVTCA[1]`);
/// `ADD`, `ADD[]` and `ADD[ ]` are the same. `PUSH` and `PUSHB[ ]`, whose opcode depends on
/// their values, give `None`.
pub fn opcode(mnemonic: &str) -> Option<u8> {
    let (head, rest) = table::parse_head(mnemonic.trim()).ok()?;
    if !rest.trim().is_empty() {
        return None;
    }
    match head {
        table::Head::Op(op) => Some(op),
        table::Head::Push { words, counted: true, .. } => Some(if words { table::NPUSHW } else { table::NPUSHB }),
        table::Head::Push { words, counted: false, count: Some(n) } => {
            let base = if words { table::PUSHW } else { table::PUSHB };
            Some(base.saturating_add(u8::try_from(n.checked_sub(1)?).ok()?))
        }
        table::Head::Push { count: None, .. } | table::Head::Auto => None,
    }
}

/// Decodes a program into instructions, inline data attached. A byte that is no instruction
/// decodes as an instruction without data (an `IDEF` may define it); [`disassemble`] prints it
/// as `INSTR<byte>[ ]`. Blocks need not be balanced here; [`analyse`] checks them.
pub fn decode(bytes: &[u8]) -> Result<Vec<Instruction>, AsmError> {
    if bytes.len() > MAX_PROGRAM_BYTES {
        return Err(AsmError::TooLarge { what: "bytes in a program", limit: MAX_PROGRAM_BYTES });
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut depth = 0usize;
    while let Some(&opcode) = bytes.get(pos) {
        let offset = pos;
        let mut data = Vec::new();
        pos += 1;
        if let Some(form) = table::push_form(opcode) {
            let count = match form.count {
                Some(n) => n,
                None => {
                    let n = *bytes.get(pos).ok_or(AsmError::Truncated { offset })?;
                    pos += 1;
                    usize::from(n)
                }
            };
            let width = if form.words { 2 } else { 1 };
            let raw = bytes.get(pos..pos + count * width).ok_or(AsmError::Truncated { offset })?;
            data = if form.words {
                raw.chunks_exact(2)
                    .map(|w| i32::from(i16::from_be_bytes([w.first().copied().unwrap_or(0), w.get(1).copied().unwrap_or(0)])))
                    .collect()
            } else {
                raw.iter().map(|b| i32::from(*b)).collect()
            };
            pos += count * width;
        }
        match opcode {
            table::IF | table::FDEF | table::IDEF => {
                depth += 1;
                if depth > MAX_NESTING {
                    return Err(AsmError::Unbalanced { what: "nesting: IF, FDEF and IDEF more than 64 deep", offset });
                }
            }
            table::EIF | table::ENDF => depth = depth.saturating_sub(1),
            _ => {}
        }
        out.push(Instruction { offset, opcode, data });
    }
    Ok(out)
}

/// Encodes instructions as bytes, each push in the form its opcode names. A push whose values do
/// not fit its form fails with [`AsmError::Syntax`], whose `line` then counts instructions from 1.
pub fn encode(instructions: &[Instruction]) -> Result<Vec<u8>, AsmError> {
    let mut out = Vec::new();
    for (index, ins) in instructions.iter().enumerate() {
        text::encode_one(ins.opcode, &ins.data, &mut out).map_err(|message| AsmError::Syntax { line: index + 1, message })?;
        if out.len() > MAX_PROGRAM_BYTES {
            return Err(AsmError::TooLarge { what: "bytes in a program", limit: MAX_PROGRAM_BYTES });
        }
    }
    Ok(out)
}

/// Assembles text (syntax in the crate docs) into bytes.
pub fn assemble(text: &str) -> Result<Vec<u8>, AsmError> {
    if text.len() > MAX_PROGRAM_TEXT {
        return Err(AsmError::TooLarge { what: "bytes of assembly text", limit: MAX_PROGRAM_TEXT });
    }
    text::assemble(text)
}

/// Disassembles bytes into text that [`assemble`] turns back into the same bytes: one
/// instruction per line, the bodies of `IF`, `ELSE`, `FDEF` and `IDEF` indented by two spaces.
pub fn disassemble(bytes: &[u8]) -> Result<String, AsmError> {
    text::disassemble(&decode(bytes)?)
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
    /// Values below its entry depth that the function pops or moves (its arguments), when known.
    pub consumes: Option<u32>,
    /// The analysis of the function, or of a function it calls, could not follow everything
    /// (see [`ProgramLimits::uncertain`]).
    pub uncertain: bool,
}

/// The functions `fpgm` defines, by number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunctionTable {
    pub functions: BTreeMap<u32, FunctionInfo>,
}

impl FunctionTable {
    /// Reads the `FDEF`s of an `fpgm` program (at most [`MAX_FUNCTIONS`]).
    pub fn from_fpgm(fpgm: &[u8]) -> Result<Self, AsmError> {
        let instructions = decode(fpgm)?;
        let mut analyser = analysis::Analyser::new(&instructions, None);
        analyser.define_all()?;
        analyser.functions()
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
    /// The program's length in bytes. `maxp` counts glyph programs only, so a caller merging the
    /// limits of `fpgm` and `prep` takes this value from the glyph programs.
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
///
/// Functions the program defines itself (an `fpgm`) take precedence over `functions`, and their
/// bodies count towards storage, twilight points and function numbers, so the limits of a font
/// are those of `fpgm`, `prep` and every glyph program merged.
pub fn analyse(program: &[u8], functions: &FunctionTable) -> Result<ProgramLimits, AsmError> {
    let instructions = decode(program)?;
    analysis::Analyser::new(&instructions, Some(functions)).limits(program.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asm(text: &str) -> Vec<u8> {
        assemble(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
    }

    fn line_of(text: &str) -> usize {
        match assemble(text) {
            Err(AsmError::Syntax { line, .. }) => line,
            other => panic!("{text:?} should be refused with a line, got {other:?}"),
        }
    }

    #[test]
    fn oversized_text_is_refused_before_any_work() {
        let text = " ".repeat(MAX_PROGRAM_TEXT + 1);
        assert!(matches!(assemble(&text), Err(AsmError::TooLarge { .. })));
    }

    #[test]
    fn mnemonics_and_opcodes_are_inverse() {
        assert_eq!(mnemonic(0x00), Some("SVTCA[0]"));
        assert_eq!(mnemonic(0xC0 | 0b10110), Some("MDRP[10110]"));
        assert_eq!(mnemonic(0x60), Some("ADD[ ]"));
        assert_eq!(mnemonic(0xB2), Some("PUSHB[010]"));
        assert_eq!(mnemonic(0x40), Some("NPUSHB[ ]"));
        assert_eq!(mnemonic(0x28), None);
        let mut named = 0;
        for b in 0..=255u8 {
            if let Some(m) = mnemonic(b) {
                named += 1;
                assert_eq!(opcode(m), Some(b), "{m}");
            }
        }
        // 256 bytes less the 36 the instruction set leaves unused (0x28, 0x7B, 0x83, 0x84, 0x8F,
        // 0x90, 0x92 to 0xAF).
        assert_eq!(named, 256 - 36);
        assert_eq!(opcode("add"), Some(0x60));
        assert_eq!(opcode("MIRP[01101]"), Some(0xED));
        assert_eq!(opcode("SVTCA[01]"), None);
        assert_eq!(opcode("PUSHB[ ]"), None);
    }

    #[test]
    fn every_instruction_and_push_form_round_trips() {
        let mut program = Vec::new();
        for b in 0..=255u8 {
            match b {
                0x40 => program.extend([0x40, 0]),
                0x41 => program.extend([0x41, 2, 0xFF, 0xC0, 0x00, 0x05]),
                0xB0..=0xB7 => {
                    program.push(b);
                    program.extend((0..=b - 0xB0).map(|i| i.wrapping_mul(37)));
                }
                0xB8..=0xBF => {
                    program.push(b);
                    for i in 0..=i16::from(b - 0xB8) {
                        program.extend((i * -1000).to_be_bytes());
                    }
                }
                _ => program.push(b),
            }
        }
        // Forms that are not the shortest stay as written.
        program.extend([0x40, 3, 1, 2, 3, 0xB8, 0x00, 0x07]);
        let text = disassemble(&program).unwrap();
        assert!(text.contains("PUSHW[ ] 7\n"), "{text}");
        assert!(text.contains("INSTR40[ ]\n"), "{text}");
        assert!(text.contains("NPUSHW[ ] -64 5\n"), "{text}");
        assert_eq!(assemble(&text).unwrap(), program);
        let decoded = decode(&program).unwrap();
        assert_eq!(decoded.iter().map(Instruction::len).sum::<usize>(), program.len());
        assert_eq!(encode(&decoded).unwrap(), program);
    }

    #[test]
    fn push_picks_the_shortest_forms() {
        assert_eq!(asm("PUSH 1 2 3"), [0xB2, 1, 2, 3]);
        assert_eq!(asm("PUSH -1"), [0xB8, 0xFF, 0xFF]);
        assert_eq!(asm("PUSH 1 2 3 4 1000"), [0xB3, 1, 2, 3, 4, 0xB8, 0x03, 0xE8]);
        let twenty: Vec<String> = (0..20).map(|i| i.to_string()).collect();
        let bytes = asm(&format!("PUSH {}", twenty.join(" ")));
        assert_eq!(bytes.get(..2), Some(&[0x40, 20][..]));
        assert_eq!(bytes.len(), 22);
        assert_eq!(asm("PUSHB[ ] 1 2\nPUSHB[001] 3 4"), [0xB1, 1, 2, 0xB1, 3, 4]);
    }

    #[test]
    fn fonttools_assembly_is_accepted() {
        let text = "PUSHB[ ]\t/* 2 values pushed */\n1 2\nSVTCA[0]\t/* SetFPVectorToAxis */\n  mdap[1] ; round\n";
        assert_eq!(asm(text), [0xB1, 1, 2, 0x00, 0x2F]);
    }

    #[test]
    fn malformed_text_is_refused_with_its_line() {
        assert_eq!(line_of("SVTCA[0]\nFOO[ ]"), 2);
        assert_eq!(line_of("PUSHB[ ] 256"), 1);
        assert_eq!(line_of("PUSHW[ ] 40000"), 1);
        assert_eq!(line_of("\n\nSVTCA[01]"), 3);
        assert_eq!(line_of("ADD[1]"), 1);
        assert_eq!(line_of("  \n; a comment\n12"), 3);
        assert_eq!(line_of("SVTCA[0]\nPUSHB[ ] 1 2 3 4 5 6 7 8 9"), 2);
        assert_eq!(line_of("PUSHB[010] 1 2"), 1);
        assert_eq!(line_of("NPUSHB[ ]\n1\nADD[ ] /* open"), 3);
        assert_eq!(line_of("SVTCA[0] ADD[ ]"), 1);
        assert_eq!(line_of("PUSHB[ ] 1 x"), 1);
        assert_eq!(line_of("PUSH"), 1);
        assert_eq!(line_of("INSTR96[ ]"), 1);
        let many: Vec<String> = (0..256).map(|i| (i % 200).to_string()).collect();
        assert_eq!(line_of(&format!("NPUSHB[ ] {}", many.join(" "))), 1);
    }

    #[test]
    fn deep_nesting_and_truncated_data_are_refused() {
        assert_eq!(line_of(&"PUSHB[ ] 1\nIF[ ]\n".repeat(1000)), 2 * (MAX_NESTING + 1));
        assert!(matches!(decode(&[0x58; 1000]), Err(AsmError::Unbalanced { offset: 64, .. })));
        assert_eq!(decode(&[0x00, 0x40, 5, 1]), Err(AsmError::Truncated { offset: 1 }));
        assert_eq!(decode(&[0xB9, 1, 2, 3]), Err(AsmError::Truncated { offset: 0 }));
        assert_eq!(decode(&[0x41]), Err(AsmError::Truncated { offset: 0 }));
    }

    #[test]
    fn analysis_follows_functions_storage_and_twilight() {
        // Function 0 pushes two values and pops three: it consumes one argument.
        // Function 1 reads storage 5 and touches twilight point 9.
        let fpgm = asm("PUSHB[ ] 0\nFDEF[ ]\n  PUSHB[ ] 7 8\n  POP[ ]\n  POP[ ]\n  POP[ ]\nENDF[ ]\n\
                        PUSHB[ ] 1\nFDEF[ ]\n  PUSHB[ ] 5\n  RS[ ]\n  POP[ ]\n  PUSHB[ ] 0\n  SZP0[ ]\n  PUSHB[ ] 9\n  MDAP[1]\nENDF[ ]");
        let table = FunctionTable::from_fpgm(&fpgm).unwrap();
        let f0 = table.functions.get(&0).copied().unwrap();
        assert_eq!((f0.stack_effect, f0.max_stack, f0.consumes, f0.uncertain), (Some(1), 2, Some(1), false));
        let f1 = table.functions.get(&1).copied().unwrap();
        assert_eq!((f1.stack_effect, f1.max_stack, f1.uncertain), (Some(0), 1, false));

        let font = analyse(&fpgm, &table).unwrap();
        assert_eq!(font.max_function_defs, 2);
        assert_eq!(font.max_storage, 6);
        assert_eq!(font.max_twilight_points, 10);
        assert_eq!(font.max_zones, 2);
        assert!(!font.uncertain);

        // 4 values, CALL 0 peaks at 3 + 2, leaves 2; CALL 1 peaks at 3.
        let glyph = asm("PUSHB[ ] 4 4 4 0\nCALL[ ]\nPUSHB[ ] 1\nCALL[ ]\nPOP[ ]\nPOP[ ]");
        let limits = analyse(&glyph, &table).unwrap();
        assert_eq!(limits.max_stack_elements, 5);
        assert_eq!(limits.max_function_defs, 2);
        assert_eq!(limits.max_zones, 1);
        assert_eq!(usize::from(limits.max_size_of_instructions), glyph.len());
        assert!(!limits.uncertain);

        // A function number below the call's argument stays known after the call.
        let glyph = asm("PUSHB[ ] 1 4 0\nCALL[ ]\nCALL[ ]");
        assert!(!analyse(&glyph, &table).unwrap().uncertain);
    }

    #[test]
    fn analysis_follows_both_branches() {
        let program = asm("PUSHB[ ] 1\nIF[ ]\n  PUSHB[ ] 1 2 3\n  POP[ ]\n  POP[ ]\n  POP[ ]\nELSE[ ]\n  \
                           PUSHB[ ] 1 2 3 4 5\n  CLEAR[ ]\nEIF[ ]\nPUSHB[ ] 2\nPOP[ ]");
        let limits = analyse(&program, &FunctionTable::default()).unwrap();
        assert_eq!(limits.max_stack_elements, 5);
        assert!(!limits.uncertain);
        assert!(matches!(analyse(&asm("PUSHB[ ] 1\nIF[ ]"), &FunctionTable::default()), Err(AsmError::Unbalanced { what: "IF", .. })));
        assert!(matches!(analyse(&asm("EIF[ ]"), &FunctionTable::default()), Err(AsmError::Unbalanced { what: "EIF", .. })));
        assert!(matches!(analyse(&asm("PUSHB[ ] 0\nFDEF[ ]"), &FunctionTable::default()), Err(AsmError::Unbalanced { what: "FDEF", .. })));
    }

    #[test]
    fn analysis_reports_what_it_cannot_follow() {
        let none = FunctionTable::default();
        assert!(analyse(&asm("PUSHB[ ] 1\nJMPR[ ]"), &none).unwrap().uncertain);
        assert!(analyse(&asm("PUSHB[ ] 3\nCALL[ ]"), &none).unwrap().uncertain);
        assert!(analyse(&asm("MPPEM[ ]\nRS[ ]\nPOP[ ]"), &none).unwrap().uncertain);
        // Recursion stops instead of looping.
        let fpgm = asm("PUSHB[ ] 0\nFDEF[ ]\n  PUSHB[ ] 0\n  CALL[ ]\nENDF[ ]");
        let table = FunctionTable::from_fpgm(&fpgm).unwrap();
        assert!(table.functions.get(&0).unwrap().uncertain);
        // A huge loop count is arithmetic, not iteration.
        let fpgm = asm("PUSHB[ ] 0\nFDEF[ ]\n  PUSHB[ ] 1 2 3\nENDF[ ]");
        let table = FunctionTable::from_fpgm(&fpgm).unwrap();
        let limits = analyse(&asm("PUSHW[ ] 32767 0\nLOOPCALL[ ]"), &table).unwrap();
        assert_eq!(limits.max_stack_elements, u16::MAX);
        assert!(limits.uncertain);
    }
}
