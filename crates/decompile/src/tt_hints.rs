//! TrueType hinting of an opened font: `cvt ` is stored as the cvt Table parameter and `prep`
//! and `fpgm` as assembly text in the prep and fpgm Table Assembly parameters on every master;
//! glyph programs go to layer user data with an outline fingerprint
//! (`fontcraft_hinting::imported`) and are written back at export while the outline is
//! unchanged; `gasp` becomes the gasp Table parameter of the font (`docs/plans/M10.md` § 3 ›
//! Imported hinting).
//!
//! Rows: `hinting.tt.import.glyphPrograms`, `hinting.tt.param.controlValueProgram` (import).
//!
//! | What the font holds | What the document gets |
//! |---|---|
//! | `cvt ` | the cvt Table parameter: the values as text, separated by commas |
//! | `fpgm`, `prep` | the fpgm and prep Table Assembly parameters ([`fontcraft_ttasm::disassemble`], which [`fontcraft_ttasm::assemble`] turns back into the same bytes) |
//! | a glyph program (simple or composite glyph) | the layer's imported program and outline fingerprint |
//! | `gasp` | the gasp Table parameter of the font, {largest PPM: flags} |
//!
//! The three tables go together: when one cannot be read or is past a limit, none of them and
//! no glyph program is kept, since the programs call each other's functions and values. Glyph
//! programs are kept only when the font has at least one of the three tables, which is the only
//! case export writes them back. A glyph whose outline was not read exactly (a component placed
//! by point matching, a contour the model could not draw) loses its program, with a warning.

use std::sync::Arc;

use fontcraft_hinting::MAX_CVT_ENTRIES;
use fontcraft_hinting::imported::set_imported_program;
use fontcraft_hinting::settings::{CVT_TABLE, FPGM_TABLE_ASSEMBLY, GASP_TABLE, MAX_GASP_RANGES, PREP_TABLE_ASSEMBLY};
use fontcraft_model::plist::{Dict, Number, Str, Value};
use fontcraft_model::{Font, Layer, Shape};
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::raw::types::{GlyphId, Tag};
use skrifa::raw::{FontRef, TableProvider};

use crate::DecompileError;
use crate::type1::list;

/// The tables [`import`] reads, which `lib.rs` leaves out of its "tables not read" warning.
pub const TABLES: &[&str] = &["cvt ", "fpgm", "prep", "gasp"];

/// Most assembly text kept for all glyph programs together. Past it the remaining programs are
/// dropped with a warning.
pub const MAX_IMPORTED_PROGRAM_TEXT: usize = 64 << 20;

/// The font-wide programs, as the parameters hold them.
#[derive(Default)]
struct FontPrograms {
    cvt: Option<String>,
    fpgm: Option<String>,
    prep: Option<String>,
}

impl FontPrograms {
    fn is_empty(&self) -> bool {
        self.cvt.is_none() && self.fpgm.is_none() && self.prep.is_none()
    }
}

/// Adds the TrueType hinting of `sfnt` to `font`. A font without hinting tables is left alone.
/// Programs that do not disassemble go to `warnings` and are dropped; only a model error fails.
pub fn import(sfnt: &FontRef, font: &mut Font, warnings: &mut Vec<String>) -> Result<(), DecompileError> {
    import_gasp(sfnt, font, warnings);
    let programs = match font_programs(sfnt) {
        Ok(p) => p,
        Err(why) => {
            warnings.push(format!("the TrueType hinting was not kept: {why}"));
            return Ok(());
        }
    };
    let has_tables = !programs.is_empty();
    if has_tables {
        let masters = Arc::make_mut(&mut font.masters);
        for m in masters.iter_mut() {
            for (name, text) in [(CVT_TABLE, &programs.cvt), (FPGM_TABLE_ASSEMBLY, &programs.fpgm), (PREP_TABLE_ASSEMBLY, &programs.prep)] {
                if let Some(text) = text {
                    m.params.set(name, Value::String(Str::new(text.as_str())));
                }
            }
        }
    }
    import_glyph_programs(sfnt, font, has_tables, warnings);
    Ok(())
}

/// `cvt `, `fpgm` and `prep` as parameter text, or why they cannot be kept.
fn font_programs(sfnt: &FontRef) -> Result<FontPrograms, String> {
    let mut out = FontPrograms::default();
    if sfnt.table_data(Tag::new(b"cvt ")).is_some() {
        let values = sfnt.cvt().map_err(|e| format!("the cvt table cannot be read ({e})"))?;
        if values.len() > MAX_CVT_ENTRIES {
            return Err(format!("the cvt table has {} values, more than the {MAX_CVT_ENTRIES} a master keeps", values.len()));
        }
        out.cvt = Some(values.iter().map(|v| v.get().to_string()).collect::<Vec<_>>().join(", "));
    }
    for (tag, slot) in [(b"fpgm", &mut out.fpgm), (b"prep", &mut out.prep)] {
        let Some(data) = sfnt.table_data(Tag::new(tag)) else { continue };
        let name = String::from_utf8_lossy(tag);
        let text = fontcraft_ttasm::disassemble(data.as_bytes()).map_err(|e| format!("the {name} table cannot be read ({e})"))?;
        if text.len() > fontcraft_ttasm::MAX_PROGRAM_TEXT {
            return Err(format!("the {name} table is longer than a parameter holds"));
        }
        *slot = Some(text);
    }
    Ok(out)
}

/// `gasp` as the gasp Table parameter of the font.
fn import_gasp(sfnt: &FontRef, font: &mut Font, warnings: &mut Vec<String>) {
    if sfnt.table_data(Tag::new(b"gasp")).is_none() {
        return;
    }
    let gasp = match sfnt.gasp() {
        Ok(g) => g,
        Err(e) => {
            warnings.push(format!("the gasp table cannot be read ({e}), so it was not kept"));
            return;
        }
    };
    let ranges = gasp.gasp_ranges();
    if ranges.is_empty() {
        return;
    }
    if ranges.len() > MAX_GASP_RANGES {
        warnings.push(format!("the gasp table has {} ranges; only the first {MAX_GASP_RANGES} were kept", ranges.len()));
    }
    let mut d = Dict::new();
    for r in ranges.iter().take(MAX_GASP_RANGES) {
        d.insert(Str::new(r.range_max_ppem().to_string()), Value::Number(Number::int(i64::from(r.range_gasp_behavior().bits()))));
    }
    Arc::make_mut(&mut font.info).params.set(GASP_TABLE, Value::Dict(d));
}

/// The number of nodes on the layer's paths and of its components, or `None` when it holds
/// another kind of shape.
fn shape_counts(layer: &Layer) -> Option<(usize, usize)> {
    let (mut nodes, mut components) = (0usize, 0usize);
    for s in &layer.shapes {
        match s {
            Shape::Path(p) => nodes = nodes.saturating_add(p.nodes.len()),
            Shape::Component(_) => components += 1,
            _ => return None,
        }
    }
    Some((nodes, components))
}

/// Each glyph's program as the layer's imported program.
fn import_glyph_programs(sfnt: &FontRef, font: &mut Font, has_tables: bool, warnings: &mut Vec<String>) {
    let (Ok(glyf), Ok(loca)) = (sfnt.glyf(), sfnt.loca(None)) else { return };
    let count = font.glyphs.as_arcs().len();
    let (mut unreadable, mut inexact) = (Vec::new(), Vec::new());
    let mut text_total = 0usize;
    let mut found = 0usize;
    for gid in 0..count {
        let Ok(id) = u32::try_from(gid) else { break };
        // The program and the outline it numbers: points of a simple glyph, components of a
        // composite.
        let (program, expected) = match loca.get_glyf(GlyphId::new(id), &glyf) {
            Ok(Some(RawGlyph::Simple(s))) => (s.instructions(), (s.num_points(), 0)),
            Ok(Some(RawGlyph::Composite(c))) => (c.instructions().unwrap_or_default(), (0, c.components().count())),
            _ => continue,
        };
        if program.is_empty() {
            continue;
        }
        found += 1;
        if !has_tables {
            continue;
        }
        let Some(name) = font.glyphs.as_arcs().get(gid).map(|g| g.name.clone()) else { continue };
        let text = match fontcraft_ttasm::disassemble(program) {
            Ok(t) => t,
            Err(_) => {
                unreadable.push(name);
                continue;
            }
        };
        text_total = text_total.saturating_add(text.len());
        if text_total > MAX_IMPORTED_PROGRAM_TEXT {
            warnings.push(format!(
                "the glyph programs hold more than {} MB of instructions; those from glyph `{name}` on were not kept",
                MAX_IMPORTED_PROGRAM_TEXT >> 20
            ));
            break;
        }
        let Some(mut glyph) = font.glyphs.get_mut_at(gid) else { continue };
        let Some(layer) = glyph.layers.first_mut() else { continue };
        if shape_counts(layer) != Some(expected) {
            inexact.push(name);
            continue;
        }
        set_imported_program(layer, text);
    }
    if found > 0 && !has_tables {
        warnings.push(format!("{found} glyph programs were not kept: the font has no cvt, fpgm or prep table for them to work with"));
    }
    if !unreadable.is_empty() {
        warnings.push(format!("the TrueType programs of these glyphs cannot be read and were not kept: {}", list(&unreadable)));
    }
    if !inexact.is_empty() {
        warnings
            .push(format!("these glyphs were not read exactly as the font has them, so their TrueType programs were not kept: {}", list(&inexact)));
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_compile::{Compiled, Options, compile_ttf};
    use fontcraft_hinting::imported::{ImportedProgram, imported_program, set_imported_program};
    use fontcraft_model::{Component, Path, Transform};

    use super::*;

    const FPGM: &str = "PUSHB[ ] 0\nFDEF[ ]\nPOP[ ]\nENDF[ ]";
    const PREP: &str = "PUSHB[ ] 1\nPUSHB[ ] 0\nCALL[ ]";
    const H_PROGRAM: &str = "SVTCA[0]\nPUSHB[ ] 0\nMDAP[1]";
    const COMPOSITE_PROGRAM: &str = "SVTCA[1]\nPUSHB[ ] 2\nMDAP[0]";

    /// A document as an opened hinted font leaves it: the three tables as master parameters, a
    /// gasp Table, and imported programs on `H` (two boxes) and on the composite `Hcomp` (`H`
    /// moved right).
    fn opened() -> Font {
        let mut f = Font::new("Hint Again");
        {
            let mut h = f.glyph_mut("H").unwrap();
            h.layers[0].shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0)), Shape::Path(Path::rect(450.0, 0.0, 550.0, 700.0))];
            set_imported_program(&mut h.layers[0], H_PROGRAM.into());
        }
        f.add_glyph("Hcomp", vec![], 600.0).unwrap();
        {
            let mut c = f.glyph_mut("Hcomp").unwrap();
            c.layers[0].shapes = vec![Shape::Component(Component {
                glyph: "H".into(),
                transform: Transform::Matrix([1.0, 0.0, 0.0, 1.0, 20.0, 0.0]),
                info: Default::default(),
                extra: Default::default(),
            })];
            set_imported_program(&mut c.layers[0], COMPOSITE_PROGRAM.into());
        }
        let params = &mut Arc::make_mut(&mut f.masters)[0].params;
        params.set(CVT_TABLE, Value::String(Str::new("10, -20")));
        params.set(FPGM_TABLE_ASSEMBLY, Value::String(Str::new(FPGM)));
        params.set(PREP_TABLE_ASSEMBLY, Value::String(Str::new(PREP)));
        let mut gasp = Dict::new();
        gasp.insert("8", Value::Number(Number::int(2)));
        gasp.insert("65535", Value::Number(Number::int(15)));
        Arc::make_mut(&mut f.info).params.set(GASP_TABLE, Value::Dict(gasp));
        f
    }

    fn table(out: &Compiled, tag: &[u8; 4]) -> Vec<u8> {
        FontRef::new(&out.bytes).unwrap().table_data(Tag::new(tag)).map(|d| d.as_bytes().to_vec()).unwrap_or_default()
    }

    /// The program of glyph `name`, simple or composite.
    fn program(out: &Compiled, name: &str) -> Vec<u8> {
        let font = FontRef::new(&out.bytes).unwrap();
        let gid = GlyphId::new(u32::try_from(out.glyph_order.iter().position(|n| n == name).unwrap()).unwrap());
        match font.loca(None).unwrap().get_glyf(gid, &font.glyf().unwrap()).unwrap() {
            Some(RawGlyph::Simple(s)) => s.instructions().to_vec(),
            Some(RawGlyph::Composite(c)) => c.instructions().unwrap_or_default().to_vec(),
            None => Vec::new(),
        }
    }

    #[test]
    fn an_opened_hinted_font_reexports_its_hinting_byte_for_byte() {
        let first = compile_ttf(&opened(), &Options::default()).unwrap();
        let asm = |t: &str| fontcraft_ttasm::assemble(t).unwrap();
        assert_eq!(program(&first, "H"), asm(H_PROGRAM));
        assert_eq!(program(&first, "Hcomp"), asm(COMPOSITE_PROGRAM));
        assert_eq!(table(&first, b"cvt "), [0, 10, 0xFF, 0xEC]);
        // The program numbers the layer's nodes, so they are written in their order.
        let font = FontRef::new(&first.bytes).unwrap();
        let gid = GlyphId::new(u32::try_from(first.glyph_order.iter().position(|n| n == "H").unwrap()).unwrap());
        let Some(RawGlyph::Simple(h)) = font.loca(None).unwrap().get_glyf(gid, &font.glyf().unwrap()).unwrap() else { panic!("H is simple") };
        let points: Vec<(i16, i16)> = h.points().map(|p| (p.x, p.y)).collect();
        assert_eq!(&points[..4], &[(50, 0), (150, 0), (150, 700), (50, 700)]);
        assert!(font.maxp().unwrap().max_size_of_instructions().unwrap() >= u16::try_from(asm(COMPOSITE_PROGRAM).len()).unwrap());

        let d = crate::decompile(&first.bytes, None).unwrap();
        assert!(!d.warnings.iter().any(|w| w.contains("tables not read") && (w.contains("fpgm") || w.contains("gasp"))), "{:?}", d.warnings);
        let f = d.font;
        let m = f.default_master().unwrap();
        assert_eq!(m.params.get(CVT_TABLE).and_then(Value::as_str), Some("10, -20"));
        for (name, text) in [(FPGM_TABLE_ASSEMBLY, FPGM), (PREP_TABLE_ASSEMBLY, PREP)] {
            assert_eq!(asm(m.params.get(name).and_then(Value::as_str).unwrap()), asm(text), "{name}");
        }
        let gasp = f.info.params.get(GASP_TABLE).and_then(Value::as_dict).unwrap();
        let ranges: Vec<(&str, Option<f64>)> = gasp.entries.iter().map(|(k, v)| (k.as_str(), v.as_num().map(|n| n.as_f64()))).collect();
        assert_eq!(ranges, vec![("8", Some(2.0)), ("65535", Some(15.0))]);
        for (glyph, text) in [("H", H_PROGRAM), ("Hcomp", COMPOSITE_PROGRAM)] {
            let ImportedProgram::Current(kept) = imported_program(&f.glyph(glyph).unwrap().layers[0]) else { panic!("{glyph} keeps its program") };
            assert_eq!(asm(kept), asm(text), "{glyph}");
        }

        let second = compile_ttf(&f, &Options::default()).unwrap();
        assert!(!second.warnings.iter().any(|w| w.contains("imported TrueType programs")), "{:?}", second.warnings);
        for tag in [b"cvt ", b"fpgm", b"prep", b"gasp"] {
            assert_eq!(table(&second, tag), table(&first, tag), "{}", String::from_utf8_lossy(tag));
        }
        for name in &first.glyph_order {
            assert_eq!(program(&second, name), program(&first, name), "{name}");
        }
    }

    #[test]
    fn a_changed_outline_leaves_its_imported_program_out() {
        let first = compile_ttf(&opened(), &Options::default()).unwrap();
        let mut f = crate::decompile(&first.bytes, None).unwrap().font;
        {
            let mut h = f.glyph_mut("H").unwrap();
            let Shape::Path(p) = &mut h.layers[0].shapes[0] else { panic!() };
            p.nodes[2].y += 10.0;
        }
        let out = compile_ttf(&f, &Options::default()).unwrap();
        assert!(program(&out, "H").is_empty());
        assert!(out.warnings.iter().any(|w| w.contains("`H`") && w.contains("outlines changed")), "{:?}", out.warnings);
        assert_eq!(program(&out, "Hcomp"), fontcraft_ttasm::assemble(COMPOSITE_PROGRAM).unwrap());
        assert_eq!(table(&out, b"fpgm"), table(&first, b"fpgm"));
    }

    #[test]
    fn a_font_without_hinting_tables_brings_no_parameters() {
        let mut plain = opened();
        Arc::make_mut(&mut plain.masters)[0].params.0.clear();
        Arc::make_mut(&mut plain.info).params.0.clear();
        let out = compile_ttf(&plain, &Options::default()).unwrap();
        let f = crate::decompile(&out.bytes, None).unwrap().font;
        let m = f.default_master().unwrap();
        assert!([CVT_TABLE, FPGM_TABLE_ASSEMBLY, PREP_TABLE_ASSEMBLY].iter().all(|n| m.params.get(n).is_none()));
        assert_eq!(imported_program(&f.glyph("H").unwrap().layers[0]), ImportedProgram::Absent);
    }

    #[test]
    fn damaged_hinting_tables_are_warnings_not_panics() {
        let good = compile_ttf(&opened(), &Options::default()).unwrap().bytes;
        for i in (0..good.len()).step_by(7) {
            let mut bad = good.clone();
            bad[i] = bad[i].wrapping_add(113);
            let _ = crate::decompile(&bad, None);
        }
    }
}
