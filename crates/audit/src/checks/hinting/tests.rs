//! Behaviour tests of the hinting checks on fonts built in code.

use std::sync::Arc;

use fontcraft_model::{AlignmentZone, Font, StemDef};

use super::*;
use crate::{AuditContext, CheckSpec, Config, LayerReport, NoServices, Profile, Severity, audit_binary, audit_source, find, select};

// --- font building --------------------------------------------------------------------------

/// An sfnt with the given tables (sorted by tag, padded to 4 bytes; checksums left at zero).
fn sfnt(cff: bool, mut tables: Vec<(&[u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|a, b| a.0.cmp(b.0));
    let head_len = 12 + 16 * tables.len();
    let mut out = Vec::new();
    out.extend_from_slice(if cff { b"OTTO" } else { &[0, 1, 0, 0] });
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&((head_len + body.len()) as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    out.extend(body);
    out
}

fn head(flags: u16, y_min: i16, y_max: i16) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend(0x0001_0000u32.to_be_bytes());
    h.extend(0x0001_0000u32.to_be_bytes());
    h.extend(0u32.to_be_bytes());
    h.extend(0x5F0F_3CF5u32.to_be_bytes());
    h.extend(flags.to_be_bytes());
    h.extend(1000u16.to_be_bytes());
    h.extend([0u8; 16]);
    for v in [0i16, y_min, 1000, y_max] {
        h.extend(v.to_be_bytes());
    }
    h.extend(0u16.to_be_bytes());
    h.extend(8u16.to_be_bytes());
    h.extend(2i16.to_be_bytes());
    h.extend(0i16.to_be_bytes());
    h.extend(0i16.to_be_bytes());
    assert_eq!(h.len(), 54);
    h
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut m = 0x0000_5000u32.to_be_bytes().to_vec();
    m.extend(glyphs.to_be_bytes());
    m
}

/// A `name` table with one Windows version string.
fn name(version: &str) -> Vec<u8> {
    let text: Vec<u8> = version.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut n = Vec::new();
    for v in [0u16, 1, 6 + 12, 3, 1, 0x409, 5, text.len() as u16, 0] {
        n.extend(v.to_be_bytes());
    }
    n.extend(text);
    n
}

fn gasp_table(ranges: &[(u16, u16)]) -> Vec<u8> {
    let mut g = Vec::new();
    g.extend(1u16.to_be_bytes());
    g.extend((ranges.len() as u16).to_be_bytes());
    for (max, flags) in ranges {
        g.extend(max.to_be_bytes());
        g.extend(flags.to_be_bytes());
    }
    g
}

/// `glyf` and short `loca` for an empty `.notdef` and a triangle with three instruction bytes.
fn glyf_loca() -> (Vec<u8>, Vec<u8>) {
    let mut g = Vec::new();
    for v in [1i16, 0, 0, 100, 100] {
        g.extend(v.to_be_bytes());
    }
    g.extend(2u16.to_be_bytes()); // end point of the contour
    g.extend(3u16.to_be_bytes()); // instruction length
    g.extend([0xB0, 0x01, 0x5D]);
    g.extend([0x01, 0x01, 0x01]); // on-curve, coordinates as 16-bit deltas
    for v in [0i16, 100, -100, 0, 0, 100] {
        g.extend(v.to_be_bytes());
    }
    assert_eq!(g.len(), 32);
    let loca: Vec<u8> = [0u16, 0, 16].iter().flat_map(|v| v.to_be_bytes()).collect();
    (g, loca)
}

/// A hinted TrueType font: `flags` in head, `prep` as given, and the other tables a hinted font
/// has.
fn truetype(flags: u16, prep: &[u8], gasp: Option<&[(u16, u16)]>, version: Option<&str>) -> Vec<u8> {
    let (glyf, loca) = glyf_loca();
    let mut tables = vec![
        (b"head", head(flags, 0, 100)),
        (b"maxp", maxp(2)),
        (b"glyf", glyf),
        (b"loca", loca),
        (b"fpgm", vec![0xB0, 0x00, 0x2C, 0x2D, 0x00]),
        (b"prep", prep.to_vec()),
    ];
    if let Some(ranges) = gasp {
        tables.push((b"gasp", gasp_table(ranges)));
    }
    if let Some(v) = version {
        tables.push((b"name", name(v)));
    }
    sfnt(false, tables)
}

fn dict_int(v: i32) -> Vec<u8> {
    match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => {
            let w = v - 108;
            vec![(w / 256 + 247) as u8, (w % 256) as u8]
        }
        -1131..=-108 => {
            let w = -v - 108;
            vec![(w / 256 + 251) as u8, (w % 256) as u8]
        }
        _ => {
            let mut o = vec![29];
            o.extend(v.to_be_bytes());
            o
        }
    }
}

fn dict_int5(v: i32) -> Vec<u8> {
    let mut o = vec![29];
    o.extend(v.to_be_bytes());
    o
}

fn dict_real(s: &str) -> Vec<u8> {
    let mut nibbles: Vec<u8> = s
        .chars()
        .map(|c| match c {
            '0'..='9' => c as u8 - b'0',
            '.' => 0x0A,
            '-' => 0x0E,
            _ => panic!("{c}"),
        })
        .collect();
    nibbles.push(0x0F);
    if nibbles.len() % 2 != 0 {
        nibbles.push(0x0F);
    }
    let mut o = vec![30];
    o.extend(nibbles.chunks(2).map(|p| p[0] << 4 | p[1]));
    o
}

/// A zone array, delta-encoded, with its operator.
fn blues(op: &[u8], values: &[i32]) -> Vec<u8> {
    let mut o = Vec::new();
    let mut prev = 0;
    for &v in values {
        o.extend(dict_int(v - prev));
        prev = v;
    }
    o.extend(op);
    o
}

fn cs_int(v: i32) -> Vec<u8> {
    match v {
        -1131..=1131 => dict_int(v),
        _ => {
            let mut o = vec![28];
            o.extend((v as i16).to_be_bytes());
            o
        }
    }
}

/// A charstring from `(operands, operator)` steps.
fn charstring(steps: &[(&[i32], u8)]) -> Vec<u8> {
    let mut o = Vec::new();
    for (args, op) in steps {
        for a in *args {
            o.extend(cs_int(*a));
        }
        o.push(*op);
    }
    o
}

/// A CFF INDEX with 4-byte offsets.
fn index(items: &[Vec<u8>]) -> Vec<u8> {
    if items.is_empty() {
        return vec![0, 0];
    }
    let mut o = (items.len() as u16).to_be_bytes().to_vec();
    o.push(4);
    let mut offset = 1u32;
    o.extend(offset.to_be_bytes());
    for item in items {
        offset += item.len() as u32;
        o.extend(offset.to_be_bytes());
    }
    for item in items {
        o.extend(item);
    }
    o
}

/// A CFF table (ISOAdobe charset, so glyph 1 is `space`, 2 `exclam`, 3 `quotedbl`).
fn cff(charstrings: &[Vec<u8>], private: Vec<u8>) -> Vec<u8> {
    let top = |cs_off: i32, priv_off: i32| {
        let mut t = dict_int5(cs_off);
        t.push(17);
        t.extend(dict_int5(private.len() as i32));
        t.extend(dict_int5(priv_off));
        t.push(18);
        t
    };
    let names = index(&[b"Test".to_vec()]);
    let prefix = 4 + names.len() + index(&[top(0, 0)]).len() + 2 + 2;
    let cs = index(charstrings);
    let (cs_off, priv_off) = (prefix as i32, (prefix + cs.len()) as i32);
    let mut o = vec![1, 0, 4, 4];
    o.extend(names);
    o.extend(index(&[top(cs_off, priv_off)]));
    o.extend([0, 0, 0, 0]); // empty String and Global Subr INDEXes
    assert_eq!(o.len(), prefix);
    o.extend(cs);
    o.extend(&private);
    o
}

/// A 100..400 × 0..700 rectangle after the given hint steps.
fn rect_glyph(stems: &[(&[i32], u8)]) -> Vec<u8> {
    const MOVE: &[i32] = &[100, 0];
    const LINES: &[i32] = &[300, 0, 0, 700, -300, 0];
    const NONE: &[i32] = &[];
    let mut steps: Vec<(&[i32], u8)> = stems.to_vec();
    steps.extend([(MOVE, 21), (LINES, 5), (NONE, 14)]);
    charstring(&steps)
}

fn otf(head_y: (i16, i16), charstrings: &[Vec<u8>], private: Vec<u8>) -> Vec<u8> {
    sfnt(true, vec![(b"head", head(0, head_y.0, head_y.1)), (b"maxp", maxp(charstrings.len() as u16)), (b"CFF ", cff(charstrings, private))])
}

// --- running ----------------------------------------------------------------------------------

fn run(bytes: &[u8], pattern: &str) -> LayerReport {
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    let checks = select(None, &[pattern], &[]);
    assert!(!checks.is_empty(), "{pattern}");
    let r = audit_binary(&ctx, &checks, &Config::default());
    for f in &r.findings {
        assert!(find(f.check).is_some_and(|c| c.codes.contains(&f.code)), "{} emits unlisted `{}`", f.check, f.code);
        assert_eq!(f.location.font.as_deref(), Some("f0"));
        assert!(f.data.values().all(|v| v.is_finite()), "{f:?}");
    }
    r
}

fn codes<'a>(r: &'a LayerReport, check: &str) -> Vec<&'a str> {
    r.findings.iter().filter(|f| f.check == check).map(|f| f.code).collect()
}

const SMART: [u8; 7] = [0xb8, 0x01, 0xff, 0x85, 0xb0, 0x04, 0x8d];

// --- tests ------------------------------------------------------------------------------------

#[test]
fn truetype_hinting_faults_are_reported() {
    let font = truetype(0, &[0xB0, 0x00], Some(&[(8, 0x02), (0xFFFF, 0x0A)]), Some("Version 1.000; ttfautohint (v1.6)"));
    let r = run(&font, "check.hinting.*");
    assert_eq!(codes(&r, "check.hinting.integerPpem"), ["bad-flags"]);
    assert_eq!(codes(&r, "check.hinting.smartDropout"), ["lacks-smart-dropout"]);
    assert_eq!(codes(&r, "check.hinting.gasp"), ["ranges", "non-ffff-range", "unset-flags"]);
    // "ttfautohint (v1.6)" has no options part at all, so the options test passes; 1.6 is old.
    assert_eq!(codes(&r, "check.hinting.autohinterTraces"), ["old-ttfa"]);
    let flags = r.findings.iter().find(|f| f.code == "bad-flags").unwrap();
    assert_eq!((flags.location.table.as_deref(), flags.location.field.as_deref()), (Some("head"), Some("flags")));
    let size = r.findings.iter().find(|f| f.code == "size-impact").unwrap();
    // fpgm (5 bytes, padded to 8) and prep (2, padded to 4) with their directory entries, and
    // the triangle's 3 instruction bytes.
    assert_eq!(size.data["increase"], (8.0 + 16.0) + (4.0 + 16.0) + 3.0);
    assert_eq!(size.data["hinted"], font.len() as f64);
    // Nothing about CFF applies.
    for f in &r.findings {
        assert!(!f.check.contains("blue") && !f.check.contains("Stem") && f.check != "check.hinting.forceBold", "{f:?}");
    }
}

#[test]
fn a_well_hinted_truetype_font_passes() {
    let mut prep = vec![0xB0, 0x00];
    prep.extend(SMART);
    let font = truetype(0x0008, &prep, Some(&[(0xFFFF, 0x0F)]), Some("Version 1.000; ttfautohint (v1.8.4) -l 8 -r 50 -G 200"));
    let r = run(&font, "check.hinting.*");
    let codes: Vec<&str> = r.findings.iter().map(|f| f.code).collect();
    assert_eq!(codes, ["size-impact", "ranges"], "{:#?}", r.findings);
    assert!(r.findings.iter().all(|f| f.severity == Severity::Info));
}

#[test]
fn gasp_and_version_string_edge_cases() {
    let r = run(&truetype(0x0008, &SMART, None, None), "check.hinting.*");
    assert_eq!(codes(&r, "check.hinting.gasp"), ["lacks-gasp"]);
    assert_eq!(codes(&r, "check.hinting.autohinterTraces"), ["lacks-version-strings"]);

    let r = run(&truetype(0x0008, &SMART, Some(&[(8, 0x0F)]), Some("Version 2.1")), "check.hinting.*");
    assert_eq!(codes(&r, "check.hinting.gasp"), ["lacks-ffff-range"]);
    assert_eq!(r.findings.iter().find(|f| f.code == "lacks-ffff-range").unwrap().severity, Severity::Warn);
    assert_eq!(codes(&r, "check.hinting.autohinterTraces"), ["version-not-detected"]);

    // The hinter is named but its options part is empty.
    let r = run(&truetype(0x0008, &SMART, Some(&[(0xFFFF, 0x0F)]), Some("Version 1.0; ttfautohint (v1.8.4) ")), "check.hinting.*");
    assert_eq!(codes(&r, "check.hinting.autohinterTraces"), ["lacks-ttfa-params"]);

    assert!(older("1.8.3", "1.8.4") && older("1.8", "1.8.4") && !older("1.10", "1.8.4") && !older("1.8.4", "1.8.4"));
    assert_eq!(autohinter_version("Version 1.0; ttfautohint (v1.8.2) -l 8"), Some("1.8.2"));
    assert_eq!(autohinter_params("Version 1.0; ttfautohint (v1.8.2) -l 8 -r 50; more"), Some(("1.8.2", "-l 8 -r 50")));
}

#[test]
fn smart_dropout_skips_fonts_hinted_with_a_visual_tool() {
    let mut font = truetype(0x0008, &[0xB0, 0x00], Some(&[(0xFFFF, 0x0F)]), None);
    let r = run(&font, "check.hinting.smartDropout");
    assert_eq!(codes(&r, "check.hinting.smartDropout"), ["lacks-smart-dropout"]);
    let (glyf, loca) = glyf_loca();
    font = sfnt(false, vec![(b"head", head(8, 0, 100)), (b"maxp", maxp(2)), (b"glyf", glyf), (b"loca", loca), (b"TSI5", vec![0, 0])]);
    let r = run(&font, "check.hinting.smartDropout");
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1);
}

#[test]
fn faulty_cff_alignment_zones_are_reported() {
    let mut private = blues(&[6], &[-12, 0, 500, 526, 700, 690]);
    private.extend(blues(&[7], &[-250, -240, -238, -230]));
    private.extend(blues(&[8], &[0, 1, 2]));
    private.extend(dict_real("0.04"));
    private.extend([12, 9]);
    private.extend(dict_int(1));
    private.extend([12, 11]);
    private.extend(dict_int(1));
    private.extend([12, 14]);
    let font = otf((-200, 650), &[charstring(&[(&[], 14)])], private);
    let r = run(&font, "check.hinting.*");
    assert_eq!(codes(&r, "check.hinting.blueValuesCount"), ["odd-count"]);
    assert_eq!(codes(&r, "check.hinting.blueValuesOrder"), ["unordered-pair"]);
    assert_eq!(codes(&r, "check.hinting.blueZoneSeparation"), ["zones-too-close"]);
    assert_eq!(codes(&r, "check.hinting.blueZoneHeight"), ["zone-too-tall"]);
    let mut outside = codes(&r, "check.hinting.blueZonesOutsideBBox");
    outside.sort_unstable();
    assert_eq!(outside, ["above-bbox", "below-bbox"]);
    assert_eq!(codes(&r, "check.hinting.forceBold"), ["force-bold"]);
    let tall = r.findings.iter().find(|f| f.code == "zone-too-tall").unwrap();
    assert_eq!(tall.data["height"], 26.0);
    assert_eq!(tall.location.field.as_deref(), Some("Private.BlueValues"));
    let close = r.findings.iter().find(|f| f.code == "zones-too-close").unwrap();
    assert_eq!((close.data["gap"], close.data["needed"]), (2.0, 3.0));
    // TrueType-only checks skip; an empty glyph set has nothing to hint.
    assert!(codes(&r, "check.hinting.integerPpem").is_empty());
    assert!(codes(&r, "check.hinting.missingStemHints").is_empty());
}

#[test]
fn clean_cff_alignment_zones_pass() {
    let mut private = blues(&[6], &[-12, 0, 500, 512, 700, 712]);
    private.extend(blues(&[7], &[-250, -240]));
    let font = otf((-250, 800), &[charstring(&[(&[], 14)])], private);
    let r = run(&font, "check.hinting.blue*");
    assert!(r.findings.is_empty(), "{:#?}", r.findings);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    let r = run(&otf((-250, 800), &[charstring(&[(&[], 14)])], Vec::new()), "check.hinting.blueValuesCount");
    assert_eq!(codes(&r, "check.hinting.blueValuesCount"), ["no-blue-values"]);
}

#[test]
fn cff_stem_hints_are_checked_against_the_glyph_box() {
    let glyphs = [
        charstring(&[(&[], 14)]),
        // space: a 100..400 × 0..700 box hinted inside it.
        rect_glyph(&[(&[0, 700], 1), (&[100, 50], 3)]),
        // exclam: the same box without hints.
        rect_glyph(&[]),
        // quotedbl: a stem below the box, a top ghost hint above it, a negative vertical stem.
        rect_glyph(&[(&[-50, 100, 750, -20], 1), (&[350, -30], 3)]),
    ];
    let font = otf((0, 700), &glyphs, blues(&[6], &[-12, 0, 700, 712]));
    let r = run(&font, "check.hinting.*");
    let missing: Vec<&str> = r.findings.iter().filter(|f| f.code == "missing-hints").map(|f| f.location.glyph.as_str()).collect();
    assert_eq!(missing, ["exclam"]);
    let mut sanity = codes(&r, "check.hinting.stemHintSanity");
    sanity.sort_unstable();
    assert_eq!(sanity, ["below-glyph", "edge-outside-glyph", "negative-width"], "{:#?}", r.findings);
    assert!(r.findings.iter().filter(|f| f.check == "check.hinting.stemHintSanity").all(|f| f.location.glyph == "quotedbl"));
    let size = r.findings.iter().find(|f| f.code == "size-impact").unwrap();
    assert!(size.data["increase"] > 0.0);
    assert_eq!(size.data["hinted"], font.len() as f64);
}

#[test]
fn private_dict_numbers_are_read_exactly() {
    let mut bytes = blues(&[6], &[-1200, -1131, -108, 108, 1131, 40_000]);
    bytes.extend(dict_real("0.039625"));
    bytes.extend([12, 9]);
    let d = parse_private(&bytes, None);
    assert_eq!(d.blue_values.as_deref(), Some(&[-1200.0, -1131.0, -108.0, 108.0, 1131.0, 40_000.0][..]));
    assert_eq!(d.blue_scale, Some(0.039625));
    let mut e = Vec::new();
    e.extend([30, 0x1C, 0x2F]); // 1E-2
    e.extend([12, 9]);
    assert_eq!(parse_private(&e, None).blue_scale, Some(0.01));
}

#[test]
fn hostile_fonts_do_not_panic() {
    let good = otf((0, 700), &[charstring(&[(&[], 14)]), rect_glyph(&[(&[0, 700], 1)])], blues(&[6], &[-12, 0]));
    let mut seed = 11u64;
    let mut noise = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u8
    };
    let mut fonts = vec![
        // Truncated CFF tables.
        otf((0, 700), &[charstring(&[(&[], 14)])], vec![12]),
        sfnt(true, vec![(b"head", head(0, 0, 700)), (b"CFF ", vec![1, 0, 4])]),
        // A private dictionary of unterminated numbers and stray escapes.
        otf((0, 700), &[charstring(&[(&[], 14)])], vec![30, 0x11, 0x11, 0x11, 28, 1]),
        // A charstring calling a missing subroutine, and one that never ends.
        otf((0, 700), &[charstring(&[(&[5], 10)]), charstring(&[(&[1, 2, 3], 21)])], Vec::new()),
        // TrueType tables of the wrong size.
        sfnt(
            false,
            vec![(b"head", vec![0; 10]), (b"glyf", vec![0; 3]), (b"loca", vec![0; 1]), (b"gasp", vec![0, 1, 0, 9]), (b"name", vec![0, 0, 0, 200])],
        ),
    ];
    for _ in 0..40 {
        let mut f = good.clone();
        for _ in 0..8 {
            let i = 12 + usize::from(noise()) % (f.len() - 12);
            f[i] = noise();
        }
        fonts.push(f);
    }
    for font in &fonts {
        let services = NoServices;
        let Ok(ctx) = BinaryCtx::new("f0", font, 0, &services) else { continue };
        let checks = select(None, &["check.hinting.*"], &[]);
        let r = audit_binary(&ctx, &checks, &Config::default());
        for f in &r.findings {
            assert!(find(f.check).is_some_and(|c| c.codes.contains(&f.code)));
            assert!(f.data.values().all(|v| v.is_finite()));
        }
    }
}

fn run_source(font: &Font) -> LayerReport {
    let audit = AuditContext::new(font);
    let ctx = SourceCtx { audit: &audit, services: &NoServices };
    let checks = select(None, &["check.hinting.*"], &[]);
    audit_source(&ctx, &checks, &Config::default())
}

#[test]
fn source_zones_and_stems_are_checked_per_master() {
    let mut font = Font::new("Test");
    let r = run_source(&font);
    assert!(codes(&r, "check.hinting.zonesOverlapInSource").is_empty(), "{:#?}", r.findings);
    assert_eq!(codes(&r, "check.hinting.stemsUndefined"), ["no-horizontal-stems", "no-vertical-stems"]);
    assert!(r.findings.iter().all(|f| f.location.master.as_deref() == Some("m01")));

    {
        let master = Arc::make_mut(&mut font.masters).get_mut(0).unwrap();
        // Overlaps the x-height zone (500 to 516) and holds no metric line.
        master.zones.push(AlignmentZone { pos: 510.0, size: 20.0 });
        // Clear of every zone, but holds no metric line either.
        master.zones.push(AlignmentZone { pos: 300.0, size: 10.0 });
        master.stem_values = vec![80.0, 90.0];
    }
    font.stems = Arc::new(vec![StemDef { horizontal: true, ..StemDef::default() }, StemDef::default()]);
    let r = run_source(&font);
    let mut zones = codes(&r, "check.hinting.zonesOverlapInSource");
    zones.sort_unstable();
    assert_eq!(zones, ["zone-without-metric", "zone-without-metric", "zones-overlap"]);
    assert!(codes(&r, "check.hinting.stemsUndefined").is_empty());

    // A zone that only touches another one is reported; with no minimum gap it is not.
    let mut font = Font::new("Test");
    Arc::make_mut(&mut font.masters).get_mut(0).unwrap().zones.push(AlignmentZone { pos: 516.0, size: 4.0 });
    assert_eq!(codes(&run_source(&font), "check.hinting.zonesOverlapInSource"), ["zones-overlap", "zone-without-metric"]);
    let audit = AuditContext::new(&font);
    let ctx = SourceCtx { audit: &audit, services: &NoServices };
    let mut config = Config::default();
    config.set("check.hinting.zonesOverlapInSource", "minGap", 0.0).unwrap();
    let r = audit_source(&ctx, &[find("check.hinting.zonesOverlapInSource").unwrap()], &config);
    assert_eq!(codes(&r, "check.hinting.zonesOverlapInSource"), ["zone-without-metric"]);
}

#[test]
fn hinting_registry_matches_the_specification() {
    let all: Vec<&CheckSpec> = select(None, &["check.hinting.*"], &[]);
    assert_eq!(all.len(), 15);
    assert_eq!(select(Some(Profile::Universal), &["check.hinting.*"], &[]).len(), 8);
    assert_eq!(select(Some(Profile::Strict), &["check.hinting.*"], &[]).len(), 15);
    assert!(select(Some(Profile::Opentype), &["check.hinting.*"], &[]).is_empty());
    for spec in all {
        assert!(!spec.codes.is_empty(), "{}", spec.id);
        for (code, _) in spec.code_severities {
            assert!(spec.codes.contains(code), "{} {code}", spec.id);
        }
    }
}
