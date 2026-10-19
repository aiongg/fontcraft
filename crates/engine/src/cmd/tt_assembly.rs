//! TrueType instructions as assembly text: storing the raw program of a glyph or of a font-level
//! program, and what the TrueType Instructions dialog reads.
//!
//! Rows: `hint.tt.setAssembly`; the reading side of `dialog.ttInstructions`.
//!
//! Where the programs live:
//!
//! | Target | Stored as | Read at export |
//! |---|---|---|
//! | `{glyph}` | the layer's glyph program (`fontcraft_hinting::imported`), with a fingerprint of the outline it was written for | while the outline keeps that fingerprint; a program that calls functions or uses control values or storage only when the master carries a cvt Table, prep Table Assembly or fpgm Table Assembly parameter (the command warns otherwise, and also when those parameters do not define the functions the program calls) |
//! | `"fpgm"` | the master's fpgm Table Assembly parameter | instance, then master, then font |
//! | `"prep"` | the master's prep Table Assembly parameter | instance, then master, then font |
//!
//! The text syntax is the assembler's (`fontcraft_ttasm`), plus one shorthand: numbers written
//! after an instruction that is not a push become a `PUSH` of those numbers just before it, in
//! the order written, so the last number ends up on top of the stack (`MDAP[1] 3` is
//! `PUSH 3` then `MDAP[1]`). The text is stored with the shorthand written out, comments kept,
//! so export reads it with the plain assembler. Text that assembles to no bytes removes the
//! program. A parse error, or a glyph program longer than the `glyf` length field allows,
//! changes nothing.

use std::sync::Arc;

use fontcraft_hinting::imported::{ImportedProgram, imported_program, remove_imported_program, set_imported_program};
use fontcraft_hinting::settings::{CVT_TABLE, FPGM_TABLE_ASSEMBLY, IMPORTED_GLYPH_PROGRAM_KEY, PREP_TABLE_ASSEMBLY, main_master};
use fontcraft_hinting::tt_instructions;
use fontcraft_hinting::ttasm::{self, AsmError, MAX_GLYPH_PROGRAM_BYTES, MAX_PROGRAM_TEXT};
use fontcraft_model::Font;
use fontcraft_model::plist::{Str, Value as PValue};
use serde_json::{Value, json};

use super::{CommandSpec, cmd, has_doc};
use crate::params::{Params, Targets};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "hint.tt.setAssembly",
        "Set TrueType Instructions",
        [],
        None,
        "{target: {glyph: name} | \"fpgm\" | \"prep\", master?: id (a glyph: the layer of that master, default the main master; fpgm and prep: that master's parameter, default every master, and the font-level parameter is removed), assembly: string} assemble the text and store it as the glyph's own TrueType program or as the fpgm or prep Table Assembly parameter. One instruction per line as the disassembler writes it (`SVTCA[0]`, `PUSHB[ ] 1 2`, `PUSH 1 1000`); numbers after any other instruction are pushed before it, the last on top. Text that assembles to nothing removes the program. A parse error changes nothing and names the line → {target, master, bytes, instructions, assembly: the stored text, changed: bool, warnings: [text] (a glyph program: the layer also has Instructor hints; the program needs font programs the master lacks, so export leaves it out; or it calls functions that the fpgm and prep Table Assembly parameters do not define)}",
        has_doc,
        run
    )]
}

/// What a TrueType program belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgramTarget {
    Glyph(String),
    Fpgm,
    Prep,
}

impl ProgramTarget {
    /// Reads `{glyph: name}`, `"fpgm"` or `"prep"`.
    pub fn from_json(v: &Value) -> std::result::Result<Self, String> {
        match v {
            Value::String(s) if s == "fpgm" => Ok(Self::Fpgm),
            Value::String(s) if s == "prep" => Ok(Self::Prep),
            Value::Object(m) => match m.get("glyph") {
                Some(Value::String(name)) if !name.is_empty() => Ok(Self::Glyph(name.clone())),
                _ => Err("`target` as an object needs `glyph`: a glyph name".into()),
            },
            _ => Err("`target` must be {glyph: name}, \"fpgm\" or \"prep\"".into()),
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Self::Glyph(name) => json!({ "glyph": name }),
            Self::Fpgm => json!("fpgm"),
            Self::Prep => json!("prep"),
        }
    }

    /// The parameter that holds a font-level program.
    pub fn parameter(&self) -> Option<&'static str> {
        match self {
            Self::Glyph(_) => None,
            Self::Fpgm => Some(FPGM_TABLE_ASSEMBLY),
            Self::Prep => Some(PREP_TABLE_ASSEMBLY),
        }
    }
}

/// A program as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredProgram {
    pub text: String,
    /// A glyph program whose outline changed since it was stored: export leaves it out.
    pub stale: bool,
}

/// The master a target is read or written at: `master` when given, else the main master.
pub fn master_for(font: &Font, master: Option<&str>) -> std::result::Result<String, String> {
    match master {
        Some(id) => font.masters.iter().find(|m| m.id == id).map(|m| m.id.clone()).ok_or_else(|| format!("there is no master `{id}`")),
        None if font.masters.is_empty() => Err("the font has no master".into()),
        None => Ok(main_master(font)),
    }
}

/// The program `target` has at `master` (the main master when `None`); `None` when it has none.
/// A font-level program is the master's parameter, else the font's.
pub fn stored_program(font: &Font, target: &ProgramTarget, master: Option<&str>) -> std::result::Result<Option<StoredProgram>, String> {
    let master = master_for(font, master)?;
    match target {
        ProgramTarget::Glyph(name) => {
            let glyph = font.glyph(name).ok_or_else(|| format!("there is no glyph `{name}`"))?;
            let layer = glyph.layer(&master).ok_or_else(|| format!("glyph `{name}` has no layer for master `{master}`"))?;
            Ok(match imported_program(layer) {
                ImportedProgram::Absent => None,
                ImportedProgram::Current(text) => Some(StoredProgram { text: text.to_string(), stale: false }),
                ImportedProgram::Stale => layer
                    .info
                    .user_data
                    .get(IMPORTED_GLYPH_PROGRAM_KEY)
                    .and_then(PValue::as_str)
                    .map(|text| StoredProgram { text: text.to_string(), stale: true }),
            })
        }
        ProgramTarget::Fpgm | ProgramTarget::Prep => {
            let Some(name) = target.parameter() else { return Ok(None) };
            let m = font.master(&master).map_err(|e| e.to_string())?;
            let value = m.params.get(name).or_else(|| font.info.params.get(name));
            Ok(value.and_then(PValue::as_str).map(|text| StoredProgram { text: text.to_string(), stale: false }))
        }
    }
}

/// Assembly text with the argument shorthand written out, and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assembled {
    pub text: String,
    pub bytes: Vec<u8>,
}

/// Assembles `text` (the module's syntax). Syntax errors name the line of `text`.
pub fn assemble(text: &str) -> std::result::Result<Assembled, AsmError> {
    if text.len() > MAX_PROGRAM_TEXT {
        return Err(AsmError::TooLarge { what: "bytes of assembly text", limit: MAX_PROGRAM_TEXT });
    }
    let (expanded, lines) = expand_arguments(text);
    if expanded.len() > MAX_PROGRAM_TEXT {
        return Err(AsmError::TooLarge { what: "bytes of assembly text", limit: MAX_PROGRAM_TEXT });
    }
    match ttasm::assemble(&expanded) {
        Ok(bytes) => {
            check_blocks(&bytes)?;
            Ok(Assembled { text: expanded, bytes })
        }
        Err(AsmError::Syntax { line, message }) => {
            let original = line.checked_sub(1).and_then(|i| lines.get(i)).copied().unwrap_or(line);
            Err(AsmError::Syntax { line: original, message })
        }
        Err(e) => Err(e),
    }
}

const IF: u8 = 0x58;
const ELSE: u8 = 0x1B;
const EIF: u8 = 0x59;
const FDEF: u8 = 0x2C;
const ENDF: u8 = 0x2D;
const IDEF: u8 = 0x89;

/// Refuses an `IF` without `EIF`, an `FDEF` or `IDEF` without `ENDF`, and the closing
/// instructions without their opening one. The assembler leaves this to its analysis.
fn check_blocks(bytes: &[u8]) -> std::result::Result<(), AsmError> {
    // `decode` caps the nesting at `MAX_NESTING`.
    let mut open: Vec<(u8, usize)> = Vec::new();
    for ins in ttasm::decode(bytes)? {
        let offset = ins.offset;
        match ins.opcode {
            IF | FDEF | IDEF => open.push((ins.opcode, offset)),
            ELSE if open.last().map(|o| o.0) != Some(IF) => return Err(AsmError::Unbalanced { what: "ELSE outside an IF", offset }),
            EIF if open.pop().map(|o| o.0) != Some(IF) => return Err(AsmError::Unbalanced { what: "EIF without its IF", offset }),
            ENDF if !matches!(open.pop().map(|o| o.0), Some(FDEF | IDEF)) => {
                return Err(AsmError::Unbalanced { what: "ENDF without its FDEF or IDEF", offset });
            }
            _ => {}
        }
    }
    match open.last() {
        Some(&(IF, offset)) => Err(AsmError::Unbalanced { what: "IF without its EIF", offset }),
        Some(&(_, offset)) => Err(AsmError::Unbalanced { what: "FDEF or IDEF without its ENDF", offset }),
        None => Ok(()),
    }
}

/// The push instructions, which take their values on their own line.
const PUSHES: [&str; 5] = ["PUSH", "PUSHB", "PUSHW", "NPUSHB", "NPUSHW"];

/// `text` with every `INSTR[flags] n…` line (not a push) split into `PUSH n…` and `INSTR[flags]`,
/// and for each output line the input line (from 1) it came from. Lines the shorthand does not
/// fit are kept as they are, so the assembler reports them.
pub fn expand_arguments(text: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        if let Some((indent, head, args, comment)) = split_arguments(raw) {
            out.push_str(indent);
            out.push_str("PUSH ");
            out.push_str(args);
            out.push('\n');
            lines.push(line);
            out.push_str(indent);
            out.push_str(head);
            out.push_str(comment);
        } else {
            out.push_str(raw);
        }
        out.push('\n');
        lines.push(line);
    }
    // Text typed without a final line break is stored without one.
    if !text.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    (out, lines)
}

/// `(indentation, instruction with its flags, arguments, comment)` of a line in the shorthand.
fn split_arguments(raw: &str) -> Option<(&str, &str, &str, &str)> {
    // A block comment may sit anywhere on the line; such lines are left to the assembler.
    if raw.contains("/*") {
        return None;
    }
    let (code, comment) = match raw.find(';') {
        Some(i) => (raw.get(..i)?, raw.get(i..)?),
        None => (raw, ""),
    };
    let start = code.len() - code.trim_start().len();
    let indent = code.get(..start)?;
    let body = code.get(start..)?.trim_end();
    let name_end = body.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(body.len());
    let name = body.get(..name_end)?;
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) || PUSHES.iter().any(|p| p.eq_ignore_ascii_case(name)) {
        return None;
    }
    let after = body.get(name_end..)?;
    let head_end = match after.trim_start().strip_prefix('[') {
        Some(inner) => {
            let close = inner.find(']')?;
            // name, the spaces before `[`, `[`, the flags and `]`.
            name_end + (after.len() - after.trim_start().len()) + 1 + close + 1
        }
        None => name_end,
    };
    let head = body.get(..head_end)?;
    let args = body.get(head_end..)?.trim();
    if args.is_empty() || !args.split_whitespace().all(|t| t.parse::<i64>().is_ok()) {
        return None;
    }
    Some((indent, head, args, comment))
}

/// One line of the dialog's listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedInstruction {
    /// Byte offset of the instruction in the program.
    pub offset: usize,
    /// The instruction as the disassembler writes it, nesting shown by indentation.
    pub text: String,
    /// What the instruction does, in one sentence.
    pub gloss: &'static str,
}

/// The instructions of `bytes`, one per line, with their offsets and glosses.
pub fn listing(bytes: &[u8]) -> std::result::Result<Vec<ListedInstruction>, AsmError> {
    let decoded = ttasm::decode(bytes)?;
    let text = ttasm::disassemble(bytes)?;
    Ok(decoded
        .iter()
        .zip(text.lines())
        .map(|(ins, line)| ListedInstruction { offset: ins.offset, text: line.to_string(), gloss: gloss(line.trim()).unwrap_or("") })
        .collect())
}

/// What the instruction named by `mnemonic` (`MIRP[01101]`, `PUSHB[ ]`, `svtca`) does, in one
/// sentence; `None` for no instruction.
pub fn gloss(mnemonic: &str) -> Option<&'static str> {
    let end = mnemonic.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(mnemonic.len());
    let base = mnemonic.get(..end)?.to_ascii_uppercase();
    let g = match base.as_str() {
        "PUSH" => "Pushes the values that follow, in the shortest push form.",
        "PUSHB" | "NPUSHB" => "Pushes the bytes that follow it in the program.",
        "PUSHW" | "NPUSHW" => "Pushes the signed 16-bit words that follow it in the program.",
        "SVTCA" => "Sets both the projection and the freedom vector to an axis (0: y, 1: x).",
        "SPVTCA" => "Sets the projection vector to an axis (0: y, 1: x).",
        "SFVTCA" => "Sets the freedom vector to an axis (0: y, 1: x).",
        "SPVTL" => "Sets the projection vector parallel (0) or perpendicular (1) to the line through two points.",
        "SFVTL" => "Sets the freedom vector parallel (0) or perpendicular (1) to the line through two points.",
        "SDPVTL" => "Sets the dual projection vector from the original positions of two points, and the projection vector from their current ones.",
        "SPVFS" => "Sets the projection vector from two values on the stack.",
        "SFVFS" => "Sets the freedom vector from two values on the stack.",
        "GPV" => "Pushes the projection vector's two components.",
        "GFV" => "Pushes the freedom vector's two components.",
        "SFVTPV" => "Makes the freedom vector equal to the projection vector.",
        "ISECT" => "Moves a point to the intersection of two lines.",
        "SRP0" => "Sets reference point 0.",
        "SRP1" => "Sets reference point 1.",
        "SRP2" => "Sets reference point 2.",
        "SZP0" => "Sets zone pointer 0 (0: twilight zone, 1: glyph zone).",
        "SZP1" => "Sets zone pointer 1 (0: twilight zone, 1: glyph zone).",
        "SZP2" => "Sets zone pointer 2 (0: twilight zone, 1: glyph zone).",
        "SZPS" => "Sets all three zone pointers (0: twilight zone, 1: glyph zone).",
        "SLOOP" => "Sets how many times the next looping instruction repeats.",
        "RTG" => "Rounds distances to the pixel grid.",
        "RTHG" => "Rounds distances to half pixels.",
        "RTDG" => "Rounds distances to half or whole pixels.",
        "RDTG" => "Rounds distances down to the pixel grid.",
        "RUTG" => "Rounds distances up to the pixel grid.",
        "ROFF" => "Turns rounding off.",
        "SROUND" => "Sets a custom rounding state with a period of one pixel.",
        "S45ROUND" => "Sets a custom rounding state with a period suited to 45-degree lines.",
        "SMD" => "Sets the minimum distance.",
        "SCVTCI" => "Sets the control value cut-in.",
        "SSWCI" => "Sets the single width cut-in.",
        "SSW" => "Sets the single width value.",
        "SDB" => "Sets the delta base: the size the delta instructions count from.",
        "SDS" => "Sets the delta shift: the step size of the delta instructions.",
        "FLIPON" => "Turns the auto flip of MIRP and MIAP on.",
        "FLIPOFF" => "Turns the auto flip of MIRP and MIAP off.",
        "SANGW" => "Sets the angle weight; it has no effect.",
        "SCANCTRL" => "Sets when dropout control applies.",
        "SCANTYPE" => "Sets the dropout control mode.",
        "INSTCTRL" => "Sets instruction control flags (prep only).",
        "IF" => "Runs what follows when the value popped is not zero.",
        "ELSE" => "Starts the part of an IF that runs when its value is zero.",
        "EIF" => "Ends an IF.",
        "JMPR" => "Jumps by a number of bytes popped from the stack.",
        "JROT" => "Jumps by a number of bytes when a popped value is true.",
        "JROF" => "Jumps by a number of bytes when a popped value is false.",
        "FDEF" => "Starts the definition of a numbered function.",
        "ENDF" => "Ends a function or instruction definition.",
        "IDEF" => "Starts the definition of an instruction for an unused opcode.",
        "CALL" => "Calls a function.",
        "LOOPCALL" => "Calls a function a number of times.",
        "DUP" => "Duplicates the top of the stack.",
        "POP" => "Drops the top of the stack.",
        "CLEAR" => "Empties the stack.",
        "SWAP" => "Swaps the two values on top of the stack.",
        "DEPTH" => "Pushes the number of values on the stack.",
        "CINDEX" => "Copies the value at a depth to the top of the stack.",
        "MINDEX" => "Moves the value at a depth to the top of the stack.",
        "ROLL" => "Moves the third value on the stack to the top.",
        "MDAP" => "Touches a point and makes it reference points 0 and 1, rounding its position when flagged.",
        "MIAP" => "Moves a point to a control value's position along the projection vector, rounding when flagged.",
        "MDRP" => {
            "Moves a point to keep its original distance from reference point 0 (flags: new reference, minimum distance, rounding, distance type)."
        }
        "MIRP" => {
            "Moves a point to a control value's distance from reference point 0 (flags: new reference, minimum distance, rounding, distance type)."
        }
        "MSIRP" => "Moves a point to a given distance from reference point 0.",
        "SHP" => "Shifts points by the move of a reference point.",
        "SHC" => "Shifts a contour by the move of a reference point.",
        "SHZ" => "Shifts a zone by the move of a reference point.",
        "SHPIX" => "Shifts points along the freedom vector by a distance in pixels.",
        "IP" => "Interpolates points between reference points 1 and 2.",
        "IUP" => "Interpolates the untouched points of every contour (0: in y, 1: in x).",
        "ALIGNRP" => "Moves points onto reference point 0 along the projection vector.",
        "ALIGNPTS" => "Moves two points to the middle between them.",
        "UTP" => "Untouches a point.",
        "FLIPPT" => "Flips points between on-curve and off-curve.",
        "FLIPRGON" => "Makes a range of points on-curve.",
        "FLIPRGOFF" => "Makes a range of points off-curve.",
        "WS" => "Writes a value to a storage location.",
        "RS" => "Reads a value from a storage location.",
        "WCVTP" => "Writes a control value, in pixels.",
        "WCVTF" => "Writes a control value, in font units.",
        "RCVT" => "Reads a control value.",
        "GC" => "Pushes a point's coordinate along the projection vector (0: current, 1: original).",
        "SCFS" => "Moves a point to a coordinate along the projection vector.",
        "MD" => "Pushes the distance between two points (0: current, 1: original).",
        "MPPEM" => "Pushes the current size in pixels per em.",
        "MPS" => "Pushes the current point size.",
        "GETINFO" => "Pushes facts about the rasterizer: version, rotation, stretching, variations and more.",
        "GETVARIATION" => "Pushes the normalized coordinate of each axis of a variable font.",
        "DEBUG" => "Pops a value; for debugging only.",
        "AA" => "Does nothing but pop a value.",
        "LT" => "Pushes 1 when the second value is less than the top one, else 0.",
        "LTEQ" => "Pushes 1 when the second value is less than or equal to the top one, else 0.",
        "GT" => "Pushes 1 when the second value is greater than the top one, else 0.",
        "GTEQ" => "Pushes 1 when the second value is greater than or equal to the top one, else 0.",
        "EQ" => "Pushes 1 when the two values are equal, else 0.",
        "NEQ" => "Pushes 1 when the two values differ, else 0.",
        "ODD" => "Pushes 1 when the value, rounded, is an odd number of pixels.",
        "EVEN" => "Pushes 1 when the value, rounded, is an even number of pixels.",
        "AND" => "Pushes 1 when both values are true, else 0.",
        "OR" => "Pushes 1 when either value is true, else 0.",
        "NOT" => "Pushes 1 when the value is zero, else 0.",
        "ADD" => "Adds the two values on top of the stack.",
        "SUB" => "Subtracts the top value from the one below it.",
        "MUL" => "Multiplies two 26.6 values.",
        "DIV" => "Divides the second 26.6 value by the top one.",
        "ABS" => "Replaces the top value by its absolute value.",
        "NEG" => "Negates the top value.",
        "FLOOR" => "Rounds the top value down to a whole pixel.",
        "CEILING" => "Rounds the top value up to a whole pixel.",
        "MAX" => "Pushes the larger of two values.",
        "MIN" => "Pushes the smaller of two values.",
        "ROUND" => "Rounds a distance with the current rounding state.",
        "NROUND" => "Applies engine compensation to a distance without rounding it.",
        "DELTAP1" => "Moves points by small amounts at chosen sizes (delta base to base + 15).",
        "DELTAP2" => "Moves points by small amounts at chosen sizes (delta base + 16 to + 31).",
        "DELTAP3" => "Moves points by small amounts at chosen sizes (delta base + 32 to + 47).",
        "DELTAC1" => "Changes control values by small amounts at chosen sizes (delta base to base + 15).",
        "DELTAC2" => "Changes control values by small amounts at chosen sizes (delta base + 16 to + 31).",
        "DELTAC3" => "Changes control values by small amounts at chosen sizes (delta base + 32 to + 47).",
        _ if base.starts_with("INSTR") => "A byte that is no instruction, or one an IDEF defines.",
        _ => return None,
    };
    Some(g)
}

fn run(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.setAssembly", v)?;
    let target = ProgramTarget::from_json(p.get("target").ok_or_else(|| p.bad("`target` is required"))?).map_err(|e| p.bad(e))?;
    let master = p.opt_str("master")?;
    let text = p.str("assembly")?;
    let assembled = assemble(text).map_err(|e| p.bad(format!("the instructions cannot be assembled: {e}")))?;
    let instructions = ttasm::decode(&assembled.bytes).map_err(|e| p.bad(e.to_string()))?.len();
    let remove = assembled.bytes.is_empty();
    let font = s.font()?;
    let mut warnings: Vec<String> = Vec::new();
    let master_out: Value;
    let changed = match &target {
        ProgramTarget::Glyph(name) => {
            if assembled.bytes.len() > MAX_GLYPH_PROGRAM_BYTES {
                return Err(p.bad(format!("a glyph program holds at most {MAX_GLYPH_PROGRAM_BYTES} bytes; this one has {}", assembled.bytes.len())));
            }
            let id = master_for(font, master).map_err(|e| p.bad(e))?;
            let glyph = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
            let layer = glyph.layer(&id).ok_or_else(|| p.bad(format!("glyph `{name}` has no layer for master `{id}`")))?;
            let current = match imported_program(layer) {
                ImportedProgram::Current(t) => Some(t),
                ImportedProgram::Absent | ImportedProgram::Stale => None,
            };
            let absent = matches!(imported_program(layer), ImportedProgram::Absent);
            let changed = if remove { !absent } else { current != Some(assembled.text.as_str()) };
            if !remove && tt_instructions(layer).is_ok_and(|list| !list.is_empty()) {
                warnings.push("the layer also has TrueType Instructor hints: export writes these instructions in their place".into());
            }
            if !remove && ttasm::uses_font_programs(&assembled.bytes) && !has_font_programs(font, &id) {
                warnings.push(format!(
                    "the program calls functions or uses control values or storage, and master `{id}` has no cvt Table, prep Table Assembly or fpgm Table Assembly parameter, so export leaves it out"
                ));
            } else if !remove && let Some(warning) = undefined_calls(font, &id, &assembled.bytes) {
                warnings.push(warning);
            }
            if changed {
                let targets = Targets::new(p.command(), name.clone(), Some(id.clone()));
                let label = if remove { "Remove TrueType Instructions" } else { "Set TrueType Instructions" };
                s.edit(label, |font| {
                    targets.edit_layer(font, |layer| {
                        if remove {
                            remove_imported_program(layer);
                        } else {
                            set_imported_program(layer, assembled.text.clone());
                        }
                        Ok(())
                    })
                })?;
            }
            master_out = json!(id);
            changed
        }
        ProgramTarget::Fpgm | ProgramTarget::Prep => {
            let name = target.parameter().ok_or_else(|| p.bad("no parameter for this target"))?;
            let only = match master {
                Some(_) => Some(master_for(font, master).map_err(|e| p.bad(e))?),
                None if font.masters.is_empty() => return Err(p.bad("the font has no master")),
                None => None,
            };
            let wanted = (!remove).then(|| assembled.text.clone());
            let differs = |params: &fontcraft_model::Params| params.get(name).and_then(PValue::as_str) != wanted.as_deref();
            let changed = match &only {
                Some(id) => font.master(id).is_ok_and(|m| differs(&m.params)),
                None => font.masters.iter().any(|m| differs(&m.params)) || font.info.params.get(name).is_some(),
            };
            if changed {
                let label = if remove { "Remove TrueType Instructions" } else { "Set TrueType Instructions" };
                s.edit(label, |font| {
                    let write = |params: &mut fontcraft_model::Params| match &wanted {
                        Some(text) => params.set(name, PValue::String(Str::new(text.as_str()))),
                        None => _ = params.remove(name),
                    };
                    for m in Arc::make_mut(&mut font.masters).iter_mut() {
                        if only.as_ref().is_none_or(|id| *id == m.id) {
                            write(&mut m.params);
                        }
                    }
                    if only.is_none() {
                        Arc::make_mut(&mut font.info).params.remove(name);
                    }
                    Ok(())
                })?;
            }
            if s.font()?.instances.iter().any(|i| i.params.get(name).is_some()) {
                warnings.push(format!("an instance has its own {name} parameter, which its export uses instead"));
            }
            master_out = only.map_or(Value::Null, |id| json!(id));
            changed
        }
    };
    Ok(json!({
        "target": target.to_json(),
        "master": master_out,
        "bytes": assembled.bytes.len(),
        "instructions": instructions,
        "assembly": if remove { Value::Null } else { json!(assembled.text) },
        "changed": changed,
        "warnings": warnings,
    }))
}

/// Does the export of `master` get font programs from a parameter (the master's or the font's),
/// which a glyph program that calls functions or reads control values or storage needs?
fn has_font_programs(font: &Font, master: &str) -> bool {
    let names = [CVT_TABLE, PREP_TABLE_ASSEMBLY, FPGM_TABLE_ASSEMBLY];
    let master_has = font.master(master).is_ok_and(|m| names.iter().any(|n| m.params.get(n).is_some()));
    master_has || names.iter().any(|n| font.info.params.get(n).is_some())
}

/// Most function numbers a warning about undefined calls names.
const MAX_NAMED_FUNCTIONS: usize = 16;

/// A warning when a glyph program calls functions (CALL, LOOPCALL) that the export's fpgm and
/// prep Table Assembly parameters (the master's, else the font's) do not define. A call whose
/// number is pushed just before it is checked by number; any other call only needs some function
/// to exist.
fn undefined_calls(font: &Font, master: &str, program: &[u8]) -> Option<String> {
    let list = ttasm::decode(program).ok()?;
    let mut any_call = false;
    let mut numbers: Vec<u32> = Vec::new();
    let mut seen: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut previous: Option<&ttasm::Instruction> = None;
    for ins in &list {
        if matches!(ins.opcode, 0x2A | 0x2B) {
            any_call = true;
            // The function number is on top of the stack for both calls.
            if let Some(n) = previous.and_then(|p| p.data.last()).and_then(|n| u32::try_from(*n).ok())
                && seen.insert(n)
            {
                numbers.push(n);
            }
        }
        previous = Some(ins);
    }
    if !any_call {
        return None;
    }
    let text = |name: &str| {
        let own = font.master(master).ok().and_then(|m| m.params.get(name).and_then(PValue::as_str).map(str::to_string));
        own.or_else(|| font.info.params.get(name).and_then(PValue::as_str).map(str::to_string))
    };
    let mut defined: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for name in [FPGM_TABLE_ASSEMBLY, PREP_TABLE_ASSEMBLY] {
        // Export leaves out a parameter that does not assemble, so its functions do not count.
        let table = text(name).and_then(|t| ttasm::assemble(&t).ok()).and_then(|b| ttasm::FunctionTable::from_fpgm(&b).ok());
        defined.extend(table.iter().flat_map(|t| t.functions.keys().copied()));
    }
    if defined.is_empty() {
        return Some(format!(
            "the program calls functions (CALL, LOOPCALL), and the fpgm and prep Table Assembly parameters of master `{master}` define none, so its hinting fails when the font is rendered"
        ));
    }
    let missing: Vec<String> = numbers.iter().filter(|n| !defined.contains(n)).map(u32::to_string).collect();
    if missing.is_empty() {
        return None;
    }
    let more = missing.len().saturating_sub(MAX_NAMED_FUNCTIONS);
    let shown = missing.iter().take(MAX_NAMED_FUNCTIONS).cloned().collect::<Vec<_>>().join(", ");
    let named = if more > 0 { format!("{shown} and {more} more") } else { shown };
    Some(format!(
        "the program calls function {named}, which the fpgm and prep Table Assembly parameters of master `{master}` do not define, so its hinting fails when the font is rendered"
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn arguments_after_an_instruction_become_a_push_before_it() {
        let a = assemble("SVTCA[0]\n  MDAP[1] 3 ; the stem\nMIRP[01101] 4 7").unwrap();
        assert_eq!(a.text, "SVTCA[0]\n  PUSH 3\n  MDAP[1]; the stem\nPUSH 4 7\nMIRP[01101]");
        assert_eq!(a.bytes, ttasm::assemble("SVTCA[0]\nPUSHB[ ] 3\nMDAP[1]\nPUSHB[ ] 4 7\nMIRP[01101]").unwrap());
        // Pushes keep their own values; a line the shorthand does not fit goes to the assembler.
        assert_eq!(assemble("PUSHB[ ] 1 2").unwrap().text, "PUSHB[ ] 1 2");
        assert_eq!(assemble("POP[ ]\n").unwrap().text, "POP[ ]\n");
        assert!(matches!(assemble("SVTCA[0]\nMDAP[1] x"), Err(AsmError::Syntax { line: 2, .. })));
    }

    #[test]
    fn syntax_errors_name_the_line_that_was_typed() {
        // Line 3 of the typed text is line 4 of the expanded text.
        assert!(matches!(assemble("MDAP[1] 1\nPOP[ ]\nBOGUS[ ]"), Err(AsmError::Syntax { line: 3, .. })));
    }

    #[test]
    fn the_listing_has_one_line_per_instruction_with_its_offset_and_gloss() {
        let bytes = ttasm::assemble("PUSHB[ ] 0\nFDEF[ ]\nPOP[ ]\nENDF[ ]").unwrap();
        let list = listing(&bytes).unwrap();
        assert_eq!(list.iter().map(|l| l.offset).collect::<Vec<_>>(), [0, 2, 3, 4]);
        assert_eq!(list[2].text, "  POP[ ]");
        assert_eq!(list[2].gloss, gloss("POP[ ]").unwrap());
        assert!(list.iter().all(|l| !l.gloss.is_empty()));
    }

    #[test]
    fn every_opcode_has_a_gloss() {
        for op in 0..=255u8 {
            if let Some(name) = ttasm::mnemonic(op) {
                assert!(gloss(name).is_some(), "{name} has no gloss");
            }
        }
    }
}
