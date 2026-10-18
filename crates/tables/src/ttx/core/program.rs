//! TrueType programs (`fpgm`, `prep`, glyph instructions) as `ttx` writes them: assembly with
//! one instruction per line, push values on the following lines (25 per line), the bodies of
//! `FDEF`, `IDEF`, `IF` and `ELSE` indented, opcodes without a mnemonic as `INSTRnn[ ]`. A
//! program that cannot be decoded (push data cut short, `NPUSHB` of zero values) is written as
//! hex in `<bytecode>`, as is every program when disassembly is turned off. The opcode table is
//! fontTools' `ttProgram` (MIT).

use super::super::{XmlWriter, num2binary};

/// Opcode, mnemonic, the number of bits the opcode carries, and the descriptive name.
pub(crate) const OPCODES: &[(u8, &str, u32, &str)] = &[
    (0x00, "SVTCA", 1, "SetFPVectorToAxis"),
    (0x02, "SPVTCA", 1, "SetPVectorToAxis"),
    (0x04, "SFVTCA", 1, "SetFVectorToAxis"),
    (0x06, "SPVTL", 1, "SetPVectorToLine"),
    (0x08, "SFVTL", 1, "SetFVectorToLine"),
    (0x0a, "SPVFS", 0, "SetPVectorFromStack"),
    (0x0b, "SFVFS", 0, "SetFVectorFromStack"),
    (0x0c, "GPV", 0, "GetPVector"),
    (0x0d, "GFV", 0, "GetFVector"),
    (0x0e, "SFVTPV", 0, "SetFVectorToPVector"),
    (0x0f, "ISECT", 0, "MovePtToIntersect"),
    (0x10, "SRP0", 0, "SetRefPoint0"),
    (0x11, "SRP1", 0, "SetRefPoint1"),
    (0x12, "SRP2", 0, "SetRefPoint2"),
    (0x13, "SZP0", 0, "SetZonePointer0"),
    (0x14, "SZP1", 0, "SetZonePointer1"),
    (0x15, "SZP2", 0, "SetZonePointer2"),
    (0x16, "SZPS", 0, "SetZonePointerS"),
    (0x17, "SLOOP", 0, "SetLoopVariable"),
    (0x18, "RTG", 0, "RoundToGrid"),
    (0x19, "RTHG", 0, "RoundToHalfGrid"),
    (0x1a, "SMD", 0, "SetMinimumDistance"),
    (0x1b, "ELSE", 0, "Else"),
    (0x1c, "JMPR", 0, "Jump"),
    (0x1d, "SCVTCI", 0, "SetCVTCutIn"),
    (0x1e, "SSWCI", 0, "SetSingleWidthCutIn"),
    (0x1f, "SSW", 0, "SetSingleWidth"),
    (0x20, "DUP", 0, "DuplicateTopStack"),
    (0x21, "POP", 0, "PopTopStack"),
    (0x22, "CLEAR", 0, "ClearStack"),
    (0x23, "SWAP", 0, "SwapTopStack"),
    (0x24, "DEPTH", 0, "GetDepthStack"),
    (0x25, "CINDEX", 0, "CopyXToTopStack"),
    (0x26, "MINDEX", 0, "MoveXToTopStack"),
    (0x27, "ALIGNPTS", 0, "AlignPts"),
    (0x29, "UTP", 0, "UnTouchPt"),
    (0x2a, "LOOPCALL", 0, "LoopAndCallFunction"),
    (0x2b, "CALL", 0, "CallFunction"),
    (0x2c, "FDEF", 0, "FunctionDefinition"),
    (0x2d, "ENDF", 0, "EndFunctionDefinition"),
    (0x2e, "MDAP", 1, "MoveDirectAbsPt"),
    (0x30, "IUP", 1, "InterpolateUntPts"),
    (0x32, "SHP", 1, "ShiftPointByLastPoint"),
    (0x34, "SHC", 1, "ShiftContourByLastPt"),
    (0x36, "SHZ", 1, "ShiftZoneByLastPoint"),
    (0x38, "SHPIX", 0, "ShiftZoneByPixel"),
    (0x39, "IP", 0, "InterpolatePts"),
    (0x3a, "MSIRP", 1, "MoveStackIndirRelPt"),
    (0x3c, "ALIGNRP", 0, "AlignRelativePt"),
    (0x3d, "RTDG", 0, "RoundToDoubleGrid"),
    (0x3e, "MIAP", 1, "MoveIndirectAbsPt"),
    (0x42, "WS", 0, "WriteStore"),
    (0x43, "RS", 0, "ReadStore"),
    (0x44, "WCVTP", 0, "WriteCVTInPixels"),
    (0x45, "RCVT", 0, "ReadCVT"),
    (0x46, "GC", 1, "GetCoordOnPVector"),
    (0x48, "SCFS", 0, "SetCoordFromStackFP"),
    (0x49, "MD", 1, "MeasureDistance"),
    (0x4b, "MPPEM", 0, "MeasurePixelPerEm"),
    (0x4c, "MPS", 0, "MeasurePointSize"),
    (0x4d, "FLIPON", 0, "SetAutoFlipOn"),
    (0x4e, "FLIPOFF", 0, "SetAutoFlipOff"),
    (0x4f, "DEBUG", 0, "DebugCall"),
    (0x50, "LT", 0, "LessThan"),
    (0x51, "LTEQ", 0, "LessThenOrEqual"),
    (0x52, "GT", 0, "GreaterThan"),
    (0x53, "GTEQ", 0, "GreaterThanOrEqual"),
    (0x54, "EQ", 0, "Equal"),
    (0x55, "NEQ", 0, "NotEqual"),
    (0x56, "ODD", 0, "Odd"),
    (0x57, "EVEN", 0, "Even"),
    (0x58, "IF", 0, "If"),
    (0x59, "EIF", 0, "EndIf"),
    (0x5a, "AND", 0, "LogicalAnd"),
    (0x5b, "OR", 0, "LogicalOr"),
    (0x5c, "NOT", 0, "LogicalNot"),
    (0x5d, "DELTAP1", 0, "DeltaExceptionP1"),
    (0x5e, "SDB", 0, "SetDeltaBaseInGState"),
    (0x5f, "SDS", 0, "SetDeltaShiftInGState"),
    (0x60, "ADD", 0, "Add"),
    (0x61, "SUB", 0, "Subtract"),
    (0x62, "DIV", 0, "Divide"),
    (0x63, "MUL", 0, "Multiply"),
    (0x64, "ABS", 0, "Absolute"),
    (0x65, "NEG", 0, "Negate"),
    (0x66, "FLOOR", 0, "Floor"),
    (0x67, "CEILING", 0, "Ceiling"),
    (0x68, "ROUND", 2, "Round"),
    (0x6c, "NROUND", 2, "NoRound"),
    (0x70, "WCVTF", 0, "WriteCVTInFUnits"),
    (0x71, "DELTAP2", 0, "DeltaExceptionP2"),
    (0x72, "DELTAP3", 0, "DeltaExceptionP3"),
    (0x73, "DELTAC1", 0, "DeltaExceptionC1"),
    (0x74, "DELTAC2", 0, "DeltaExceptionC2"),
    (0x75, "DELTAC3", 0, "DeltaExceptionC3"),
    (0x76, "SROUND", 0, "SuperRound"),
    (0x77, "S45ROUND", 0, "SuperRound45Degrees"),
    (0x78, "JROT", 0, "JumpRelativeOnTrue"),
    (0x79, "JROF", 0, "JumpRelativeOnFalse"),
    (0x7a, "ROFF", 0, "RoundOff"),
    (0x7c, "RUTG", 0, "RoundUpToGrid"),
    (0x7d, "RDTG", 0, "RoundDownToGrid"),
    (0x7e, "SANGW", 0, "SetAngleWeight"),
    (0x7f, "AA", 0, "AdjustAngle"),
    (0x80, "FLIPPT", 0, "FlipPoint"),
    (0x81, "FLIPRGON", 0, "FlipRangeOn"),
    (0x82, "FLIPRGOFF", 0, "FlipRangeOff"),
    (0x85, "SCANCTRL", 0, "ScanConversionControl"),
    (0x86, "SDPVTL", 1, "SetDualPVectorToLine"),
    (0x88, "GETINFO", 0, "GetInfo"),
    (0x89, "IDEF", 0, "InstructionDefinition"),
    (0x8a, "ROLL", 0, "RollTopThreeStack"),
    (0x8b, "MAX", 0, "Maximum"),
    (0x8c, "MIN", 0, "Minimum"),
    (0x8d, "SCANTYPE", 0, "ScanType"),
    (0x8e, "INSTCTRL", 0, "SetInstrExecControl"),
    (0x91, "GETVARIATION", 0, "GetVariation"),
    (0xc0, "MDRP", 5, "MoveDirectRelPt"),
    (0xe0, "MIRP", 5, "MoveIndirectRelPt"),
];

/// One line of assembly: an instruction, or a push instruction with the values it pushes.
enum Line {
    Instr(String),
    Push(String, Vec<String>),
}

fn decode(code: &[u8]) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let cut = |at: usize| format!("the program ends inside the push instruction at byte {at}");
    while let Some(&op) = code.get(i) {
        let known = OPCODES.iter().find(|(base, _, bits, _)| op >= *base && u32::from(op - base) < (1u32 << bits));
        if let Some((base, mnemonic, bits, name)) = known {
            out.push(Line::Instr(if *bits > 0 {
                format!("{mnemonic}[{}]\t/* {name} */", num2binary(u64::from(op - base), *bits))
            } else {
                format!("{mnemonic}[ ]\t/* {name} */")
            }));
            i = i.saturating_add(1);
            continue;
        }
        let (mnemonic, words) = match op {
            0x40 => ("NPUSHB", false),
            0x41 => ("NPUSHW", true),
            0xB0..=0xB7 => ("PUSHB", false),
            0xB8..=0xBF => ("PUSHW", true),
            _ => {
                out.push(Line::Instr(format!("INSTR{op}[ ]")));
                i = i.saturating_add(1);
                continue;
            }
        };
        let start = i;
        let count = if op >= 0xB0 {
            i = i.saturating_add(1);
            usize::from(op & 7).saturating_add(1)
        } else {
            let n = usize::from(*code.get(i.saturating_add(1)).ok_or_else(|| cut(start))?);
            i = i.saturating_add(2);
            n
        };
        if count == 0 {
            return Err(format!("the push instruction at byte {start} pushes no values"));
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            if words {
                let hi = *code.get(i).ok_or_else(|| cut(start))?;
                let lo = *code.get(i.saturating_add(1)).ok_or_else(|| cut(start))?;
                values.push(i16::from_be_bytes([hi, lo]).to_string());
                i = i.saturating_add(2);
            } else {
                values.push(code.get(i).ok_or_else(|| cut(start))?.to_string());
                i = i.saturating_add(1);
            }
        }
        let header = if count == 1 { format!("{mnemonic}[ ]\t/* 1 value pushed */") } else { format!("{mnemonic}[ ]\t/* {count} values pushed */") };
        out.push(Line::Push(header, values));
    }
    Ok(out)
}

/// Writes a program as `<assembly>` (or `<bytecode>`); an empty program writes nothing.
pub(crate) fn write(code: &[u8], w: &mut XmlWriter) {
    if code.is_empty() {
        return;
    }
    if !w.disassemble() {
        w.open("bytecode", &[]);
        w.hex(code);
        w.close("bytecode");
        return;
    }
    let lines = match decode(code) {
        Ok(lines) => lines,
        Err(message) => {
            w.open("bytecode", &[]);
            w.comment(&format!("An exception occurred during the decompilation of glyph program:\n\n{message}"));
            w.hex(code);
            w.close("bytecode");
            return;
        }
    };
    w.open("assembly", &[]);
    let mut indent: i64 = 0;
    for line in &lines {
        let head = match line {
            Line::Instr(s) | Line::Push(s, _) => s.as_str(),
        };
        if head.starts_with("ELSE") || head.starts_with("ENDF") || head.starts_with("EIF[ ]\t") {
            indent -= 1;
        }
        let pad = "  ".repeat(usize::try_from(indent).unwrap_or(0));
        w.text(&format!("{pad}{head}"));
        if let Line::Push(_, values) = line {
            for chunk in values.chunks(25) {
                w.text(&format!("{pad}{}", chunk.join(" ")));
            }
        }
        if head.starts_with("FDEF") || head.starts_with("IDEF") || head.starts_with("IF") || head.starts_with("ELSE[ ]\t") {
            indent += 1;
        }
    }
    w.close("assembly");
}
