//! Each font-wide metrics check on a small TrueType font built here, byte by byte.

use crate::{BinaryCtx, CHECKS, CheckScope, Config, Finding, NoServices, Severity, audit_binary, find};

/// One glyph: its code point, advance and polygon contours (all points on the curve).
#[derive(Clone)]
struct G {
    cp: Option<u32>,
    advance: u16,
    contours: Vec<Vec<(i16, i16)>>,
}

fn rect(x0: i16, y0: i16, x1: i16, y1: i16) -> Vec<(i16, i16)> {
    vec![(x0, y0), (x0, y1), (x1, y1), (x1, y0)]
}

fn g(cp: u32, advance: u16, y0: i16, y1: i16) -> G {
    G { cp: Some(cp), advance, contours: vec![rect(50, y0, i16::try_from(advance).unwrap() - 50, y1)] }
}

/// The fields of the font the tests change; `None` fields are computed from the glyphs.
#[derive(Clone)]
struct Spec {
    upem: u16,
    mac_style: u16,
    head_bbox: Option<[i16; 4]>,
    hhea: [i16; 3],
    caret: (i16, i16),
    advance_width_max: Option<u16>,
    h_metrics: Option<u16>,
    os2_version: u16,
    x_avg: Option<i16>,
    fs_selection: u16,
    typo: [i16; 3],
    win: [u16; 2],
    x_height: i16,
    cap_height: i16,
    strikeout: [i16; 2],
    sub: [i16; 4],
    sup: [i16; 4],
    panose: [u8; 10],
    italic_angle: f64,
    underline: [i16; 2],
    fixed_pitch: u32,
    glyphs: Vec<G>,
}

/// A clean upright font: units per em 1000, typo and hhea 800 / -200 / 0, win 800 / 200,
/// USE_TYPO_METRICS set, capitals 0 to 700, Agrave to 780, p down to -200.
fn base() -> Spec {
    let mut glyphs = vec![G { cp: None, advance: 500, contours: vec![rect(50, 0, 450, 700)] }, G { cp: Some(0x20), advance: 250, contours: vec![] }];
    let caps = [
        ('A', 650),
        ('B', 600),
        ('C', 640),
        ('D', 660),
        ('E', 560),
        ('H', 700),
        ('I', 300),
        ('M', 820),
        ('O', 720),
        ('S', 560),
        ('T', 580),
        ('X', 620),
    ];
    glyphs.extend(caps.iter().map(|(c, w)| g(u32::from(*c), *w, 0, 700)));
    glyphs.push(g(0xC0, 650, 0, 780));
    glyphs.push(g(u32::from('p'), 560, -200, 500));
    Spec {
        upem: 1000,
        mac_style: 0,
        head_bbox: None,
        hhea: [800, -200, 0],
        caret: (1, 0),
        advance_width_max: None,
        h_metrics: None,
        os2_version: 4,
        x_avg: None,
        fs_selection: 0xC0,
        typo: [800, -200, 0],
        win: [800, 200],
        x_height: 500,
        cap_height: 700,
        strikeout: [50, 300],
        sub: [650, 600, 0, 75],
        sup: [650, 600, 0, 350],
        panose: [2, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        italic_angle: 0.0,
        underline: [-100, 50],
        fixed_pitch: 0,
        glyphs,
    }
}

fn be16(out: &mut Vec<u8>, v: impl Into<i32>) {
    let v: i32 = v.into();
    out.extend_from_slice(&(v as u16).to_be_bytes());
}

fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn glyph_bytes(glyph: &G) -> (Vec<u8>, [i16; 4]) {
    let points: Vec<(i16, i16)> = glyph.contours.iter().flatten().copied().collect();
    if points.is_empty() {
        return (Vec::new(), [0; 4]);
    }
    let bb = [
        points.iter().map(|p| p.0).min().unwrap(),
        points.iter().map(|p| p.1).min().unwrap(),
        points.iter().map(|p| p.0).max().unwrap(),
        points.iter().map(|p| p.1).max().unwrap(),
    ];
    let mut out = Vec::new();
    be16(&mut out, glyph.contours.len() as i32);
    for v in bb {
        be16(&mut out, v);
    }
    let mut end = 0usize;
    for c in &glyph.contours {
        end += c.len();
        be16(&mut out, (end - 1) as i32);
    }
    be16(&mut out, 0);
    out.extend(std::iter::repeat_n(1u8, points.len()));
    let mut last = (0i16, 0i16);
    for p in &points {
        be16(&mut out, p.0 - last.0);
        last.0 = p.0;
    }
    for p in &points {
        be16(&mut out, p.1 - last.1);
        last.1 = p.1;
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    (out, bb)
}

fn build(s: &Spec) -> Vec<u8> {
    let n = s.glyphs.len();
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    let mut bbox = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
    for glyph in &s.glyphs {
        be32(&mut loca, glyf.len() as u32);
        let (bytes, bb) = glyph_bytes(glyph);
        if !bytes.is_empty() {
            bbox = [bbox[0].min(bb[0]), bbox[1].min(bb[1]), bbox[2].max(bb[2]), bbox[3].max(bb[3])];
        }
        glyf.extend(bytes);
    }
    be32(&mut loca, glyf.len() as u32);
    let bbox = s.head_bbox.unwrap_or(bbox);

    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, s.upem);
    head.extend([0u8; 16]);
    for v in bbox {
        be16(&mut head, v);
    }
    be16(&mut head, s.mac_style);
    be16(&mut head, 8);
    be16(&mut head, 2);
    be16(&mut head, 1);
    be16(&mut head, 0);

    let advances: Vec<u16> = s.glyphs.iter().map(|g| g.advance).collect();
    let h_metrics = s.h_metrics.unwrap_or(n as u16);
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for v in s.hhea {
        be16(&mut hhea, v);
    }
    be16(&mut hhea, s.advance_width_max.unwrap_or(*advances.iter().max().unwrap()));
    be16(&mut hhea, 0);
    be16(&mut hhea, 0);
    be16(&mut hhea, 0);
    be16(&mut hhea, s.caret.0);
    be16(&mut hhea, s.caret.1);
    hhea.extend([0u8; 10]);
    be16(&mut hhea, 0);
    be16(&mut hhea, h_metrics);

    let mut hmtx = Vec::new();
    for (i, a) in advances.iter().enumerate() {
        if i < usize::from(h_metrics) {
            be16(&mut hmtx, *a);
        }
        be16(&mut hmtx, 50);
    }

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0000_5000);
    be16(&mut maxp, n as i32);

    let nonzero: Vec<f64> = advances.iter().filter(|a| **a > 0).map(|a| f64::from(*a)).collect();
    let x_avg = s.x_avg.unwrap_or((nonzero.iter().sum::<f64>() / nonzero.len() as f64).round_ties_even() as i16);
    let mut os2 = Vec::new();
    be16(&mut os2, s.os2_version);
    be16(&mut os2, x_avg);
    be16(&mut os2, 400);
    be16(&mut os2, 5);
    be16(&mut os2, 0);
    for v in [s.sub[0], s.sub[1], s.sub[2], s.sub[3], s.sup[0], s.sup[1], s.sup[2], s.sup[3], s.strikeout[0], s.strikeout[1], 0] {
        be16(&mut os2, v);
    }
    os2.extend(s.panose);
    os2.extend([0u8; 16]);
    os2.extend(*b"NONE");
    be16(&mut os2, s.fs_selection);
    be16(&mut os2, 0x20);
    be16(&mut os2, 0xC0);
    for v in s.typo {
        be16(&mut os2, v);
    }
    be16(&mut os2, s.win[0]);
    be16(&mut os2, s.win[1]);
    if s.os2_version >= 1 {
        os2.extend([0u8; 8]);
    }
    if s.os2_version >= 2 {
        be16(&mut os2, s.x_height);
        be16(&mut os2, s.cap_height);
        be16(&mut os2, 0);
        be16(&mut os2, 0x20);
        be16(&mut os2, 0);
    }

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, (s.italic_angle * 65536.0).round() as i32 as u32);
    be16(&mut post, s.underline[0]);
    be16(&mut post, s.underline[1]);
    be32(&mut post, s.fixed_pitch);
    post.extend([0u8; 16]);

    let mut groups: Vec<(u32, u32)> = s.glyphs.iter().enumerate().filter_map(|(i, g)| g.cp.map(|c| (c, i as u32))).collect();
    groups.sort();
    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 3);
    be16(&mut cmap, 10);
    be32(&mut cmap, 12);
    be16(&mut cmap, 12);
    be16(&mut cmap, 0);
    be32(&mut cmap, 16 + 12 * groups.len() as u32);
    be32(&mut cmap, 0);
    be32(&mut cmap, groups.len() as u32);
    for (c, gid) in groups {
        be32(&mut cmap, c);
        be32(&mut cmap, c);
        be32(&mut cmap, gid);
    }

    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"post", post),
    ];
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, tables.len() as i32);
    be16(&mut out, 128);
    be16(&mut out, 3);
    be16(&mut out, 16);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        be32(&mut out, 0);
        be32(&mut out, offset as u32);
        be32(&mut out, data.len() as u32);
        let mut padded = data.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        body.extend(padded);
    }
    out.extend(body);
    out
}

/// Runs one check on the font with the given config.
fn run_with(s: &Spec, id: &str, config: &Config) -> (Vec<Finding>, Vec<String>) {
    let bytes = build(s);
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let spec = find(id).unwrap_or_else(|| panic!("no check {id}"));
    let r = audit_binary(&ctx, &[spec], config);
    for f in &r.findings {
        assert!(spec.codes.contains(&f.code), "{id} emits `{}`, which its registry entry does not list", f.code);
        assert_eq!(f.location.font.as_deref(), Some("f0"));
        assert!(f.data.values().all(|v| v.is_finite()));
    }
    (r.findings, r.skipped.into_iter().map(|s| s.reason).collect())
}

fn codes(s: &Spec, id: &str) -> Vec<&'static str> {
    let (findings, skipped) = run_with(s, id, &Config::default());
    assert!(skipped.is_empty(), "{id} skipped: {skipped:?}");
    findings.iter().map(|f| f.code).collect()
}

const IDS: [&str; 17] = [
    "check.metrics.unitsPerEm",
    "check.metrics.italicAngle",
    "check.metrics.italicBitVsAngle",
    "check.metrics.caretSlope",
    "check.metrics.maxAdvanceWidth",
    "check.metrics.xAvgCharWidth",
    "check.metrics.monospaceFlags",
    "check.metrics.fontBBox",
    "check.metrics.typoMetricsVsEm",
    "check.metrics.hheaMatchesTypo",
    "check.metrics.winAscentDescent",
    "check.metrics.lineGaps",
    "check.metrics.useTypoMetrics",
    "check.metrics.typoAscenderVsAgrave",
    "check.metrics.capsVerticallyCentred",
    "check.metrics.verticalMetricsScheme",
    "check.metrics.strikeoutAndScripts",
];

#[test]
fn every_font_wide_metrics_check_is_registered_as_a_binary_check() {
    for id in IDS {
        let spec = find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(spec.scope, CheckScope::Binary, "{id}");
        for (code, _) in spec.code_severities {
            assert!(spec.codes.contains(code), "{id}: severity for unlisted `{code}`");
        }
    }
    assert_eq!(CHECKS.iter().filter(|c| IDS.contains(&c.id)).count(), IDS.len());
}

#[test]
fn the_clean_font_passes_every_font_wide_metrics_check() {
    let mut microsoft = Config::default();
    microsoft.set("check.metrics.verticalMetricsScheme", "scheme", 1.0).unwrap();
    for id in IDS {
        let (findings, skipped) = run_with(&base(), id, &microsoft);
        assert!(findings.is_empty(), "{id}: {findings:#?}");
        assert!(skipped.is_empty(), "{id}: {skipped:?}");
    }
}

#[test]
fn units_per_em_out_of_range_fails_and_unusual_warns() {
    let s = Spec { upem: 8, ..base() };
    assert_eq!(codes(&s, "check.metrics.unitsPerEm"), ["out-of-range"]);
    let s = Spec { upem: 1200, ..base() };
    let (f, _) = run_with(&s, "check.metrics.unitsPerEm", &Config::default());
    assert_eq!((f[0].code, f[0].severity), ("suboptimal", Severity::Warn));
    assert_eq!(f[0].location.field.as_deref(), Some("unitsPerEm"));
    assert!(codes(&Spec { upem: 2048, ..base() }, "check.metrics.unitsPerEm").is_empty());
}

/// A base font whose H and I lean right by about 12 degrees.
fn slanted(angle: f64) -> Spec {
    let mut s = base();
    for glyph in &mut s.glyphs {
        if glyph.cp == Some(u32::from('H')) || glyph.cp == Some(u32::from('I')) {
            glyph.contours = vec![vec![(50, 0), (200, 700), (300, 700), (150, 0)]];
        }
    }
    s.italic_angle = angle;
    s
}

#[test]
fn italic_angle_rules() {
    // Upright style with an angle: the angle disagrees with the style.
    assert_eq!(codes(&slanted(-12.0), "check.metrics.italicAngle"), ["non-zero-upright"]);
    // Italic style, right lean, negative angle: clean.
    let italic = Spec { fs_selection: 0xC1, ..slanted(-12.0) };
    assert!(codes(&italic, "check.metrics.italicAngle").is_empty());
    // A positive angle on right-leaning outlines.
    let positive = Spec { fs_selection: 0xC1, ..slanted(12.0) };
    assert_eq!(codes(&positive, "check.metrics.italicAngle"), ["positive"]);
    // Steep angles.
    let steep = Spec { fs_selection: 0xC1, ..slanted(-35.0) };
    assert_eq!(codes(&steep, "check.metrics.italicAngle"), ["over-30-degrees"]);
    let zero = Spec { fs_selection: 0xC1, ..base() };
    let (f, _) = run_with(&zero, "check.metrics.italicAngle", &Config::default());
    assert_eq!((f[0].code, f[0].severity), ("zero-italic", Severity::Fail));
}

#[test]
fn italic_bit_and_angle_must_agree() {
    let s = Spec { mac_style: 2, ..base() };
    assert_eq!(codes(&s, "check.metrics.italicBitVsAngle"), ["bit-without-angle"]);
    assert_eq!(codes(&slanted(-12.0), "check.metrics.italicBitVsAngle"), ["angle-without-bit"]);
    assert!(codes(&Spec { mac_style: 2, ..slanted(-12.0) }, "check.metrics.italicBitVsAngle").is_empty());
}

#[test]
fn caret_slope_follows_the_italic_angle() {
    assert_eq!(codes(&Spec { caret: (0, 0), ..base() }, "check.metrics.caretSlope"), ["zero-rise"]);
    let (f, _) = run_with(&slanted(-12.0), "check.metrics.caretSlope", &Config::default());
    assert_eq!((f[0].code, f[0].severity), ("mismatch", Severity::Warn));
    // tan(12°) × 1000 = 212.56: the expected run is 213.
    assert_eq!(f[0].data.get("expectedRun"), Some(&213.0));
    assert!(codes(&Spec { caret: (1000, 213), ..slanted(-12.0) }, "check.metrics.caretSlope").is_empty());
}

#[test]
fn advance_width_max_and_average_are_recomputed() {
    let s = Spec { advance_width_max: Some(900), ..base() };
    let (f, _) = run_with(&s, "check.metrics.maxAdvanceWidth", &Config::default());
    assert_eq!(f[0].data.get("expected"), Some(&820.0));
    let base_avg = {
        let a: Vec<f64> = base().glyphs.iter().map(|g| f64::from(g.advance)).collect();
        (a.iter().sum::<f64>() / a.len() as f64).round_ties_even() as i16
    };
    assert!(codes(&Spec { x_avg: Some(base_avg + 1), ..base() }, "check.metrics.xAvgCharWidth").is_empty());
    let (f, _) = run_with(&Spec { x_avg: Some(base_avg + 5), ..base() }, "check.metrics.xAvgCharWidth", &Config::default());
    assert_eq!((f[0].code, f[0].severity), ("close", Severity::Info));
    assert_eq!(codes(&Spec { x_avg: Some(base_avg + 40), ..base() }, "check.metrics.xAvgCharWidth"), ["wrong"]);
}

/// Every printable ASCII character at width 600, with three glyphs per tens of widths.
fn monospaced(fixed_pitch: u32, proportion: u8) -> Spec {
    let mut s = base();
    s.glyphs = vec![G { cp: None, advance: 600, contours: vec![rect(50, 0, 550, 700)] }];
    s.glyphs.extend((0x21..0x7F).map(|c| g(c, 600, 0, 700)));
    s.glyphs.push(G { cp: Some(0x20), advance: 600, contours: vec![] });
    s.glyphs.push(g(0xC0, 600, 0, 780));
    s.glyphs.push(g(0xE9, 600, -200, 700));
    s.fixed_pitch = fixed_pitch;
    s.panose[3] = proportion;
    s.h_metrics = Some(3);
    s
}

#[test]
fn monospace_markers_follow_the_advances() {
    assert!(codes(&monospaced(1, 9), "check.metrics.monospaceFlags").is_empty());
    assert_eq!(codes(&monospaced(0, 9), "check.metrics.monospaceFlags"), ["mono-bad-is-fixed-pitch"]);
    assert_eq!(codes(&monospaced(1, 0), "check.metrics.monospaceFlags"), ["mono-bad-panose"]);
    let mut outlier = monospaced(1, 9);
    outlier.glyphs.push(g(0x2014, 1000, 300, 400));
    outlier.h_metrics = None;
    assert_eq!(codes(&outlier, "check.metrics.monospaceFlags"), ["number-of-hmetrics", "mono-outliers"]);
    assert_eq!(codes(&Spec { fixed_pitch: 1, ..base() }, "check.metrics.monospaceFlags"), ["bad-is-fixed-pitch"]);
}

#[test]
fn improbable_font_box_warns() {
    let s = Spec { head_bbox: Some([0, -1600, 820, 780]), ..base() };
    let (f, _) = run_with(&s, "check.metrics.fontBBox", &Config::default());
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].location.field.as_deref(), Some("yMin"));
}

#[test]
fn vertical_metrics_checks_fire_on_their_faults() {
    let typo = Spec { typo: [900, -200, 0], ..base() };
    assert_eq!(codes(&typo, "check.metrics.typoMetricsVsEm"), ["typo-height"]);
    assert_eq!(codes(&typo, "check.metrics.hheaMatchesTypo"), ["ascender"]);

    let (f, _) = run_with(&Spec { win: [700, 200], ..base() }, "check.metrics.winAscentDescent", &Config::default());
    assert_eq!(f.len(), 1);
    assert_eq!((f[0].code, f[0].data.get("expected")), ("ascent", Some(&780.0)));
    assert_eq!(f[0].location.field.as_deref(), Some("usWinAscent"));
    assert_eq!(codes(&Spec { win: [800, 500], ..base() }, "check.metrics.winAscentDescent"), ["descent"]);

    let gaps = Spec { hhea: [800, -200, 90], typo: [800, -200, 90], ..base() };
    assert_eq!(codes(&gaps, "check.metrics.lineGaps"), ["hhea-line-gap", "typo-line-gap"]);

    assert_eq!(codes(&Spec { fs_selection: 0x40, ..base() }, "check.metrics.useTypoMetrics"), ["not-set"]);
    assert_eq!(codes(&Spec { os2_version: 3, ..base() }, "check.metrics.useTypoMetrics"), ["set-below-version-4"]);

    assert_eq!(codes(&Spec { typo: [750, -250, 0], hhea: [750, -250, 0], ..base() }, "check.metrics.typoAscenderVsAgrave"), ["below-agrave"]);
    assert_eq!(codes(&Spec { hhea: [1300, -200, 0], ..base() }, "check.metrics.capsVerticallyCentred"), ["not-centred"]);
}

#[test]
fn agrave_and_capitals_missing_skip_their_checks() {
    let mut s = base();
    s.glyphs.retain(|g| g.cp != Some(0xC0) && g.cp != Some(u32::from('X')));
    for id in ["check.metrics.typoAscenderVsAgrave", "check.metrics.capsVerticallyCentred"] {
        let (f, skipped) = run_with(&s, id, &Config::default());
        assert!(f.is_empty());
        assert_eq!(skipped.len(), 1, "{id}");
    }
}

#[test]
fn vertical_metrics_schemes() {
    let id = "check.metrics.verticalMetricsScheme";
    // Google: total height 1.0 em is too small.
    assert_eq!(codes(&base(), id), ["hhea-range"]);
    let tall = Spec { typo: [950, -250, 0], hhea: [950, -250, 0], win: [950, 250], ..base() };
    assert!(codes(&tall, id).is_empty());

    let mut ms = Config::default();
    ms.set(id, "scheme", 1.0).unwrap();
    let off = Spec { fs_selection: 0x40, win: [900, 200], ..base() };
    let (f, _) = run_with(&off, id, &ms);
    assert_eq!(f.iter().map(|f| f.code).collect::<Vec<_>>(), ["hhea-win-ascent"]);

    let mut tn = Config::default();
    tn.set(id, "scheme", 2.0).unwrap();
    let (f, _) = run_with(&Spec { cap_height: 600, ..base() }, id, &tn);
    assert!(f.is_empty(), "{f:#?}");
    let (f, _) = run_with(&base(), id, &tn);
    assert_eq!(f.iter().map(|f| f.code).collect::<Vec<_>>(), ["uncentred"]);
}

#[test]
fn strikeout_and_script_fields_follow_the_defaults() {
    let id = "check.metrics.strikeoutAndScripts";
    let zero = Spec { strikeout: [0, 300], underline: [-100, 0], ..base() };
    assert_eq!(codes(&zero, id), ["strikeout-size-zero", "underline-thickness-zero"]);
    let (f, _) = run_with(&Spec { sup: [650, 600, 0, 500], ..base() }, id, &Config::default());
    assert_eq!(f.len(), 1);
    assert_eq!((f[0].code, f[0].severity), ("differs-from-default", Severity::Warn));
    assert_eq!(f[0].location.field.as_deref(), Some("ySuperscriptYOffset"));
    assert_eq!(f[0].data.get("expected"), Some(&350.0));
}

#[test]
fn fonts_without_the_tables_are_skipped_not_crashed() {
    let services = NoServices;
    let empty = [0u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let ctx = BinaryCtx::new("f0", &empty, 0, &services).unwrap();
    let checks: Vec<_> = IDS.iter().filter_map(|id| find(id)).collect();
    let r = audit_binary(&ctx, &checks, &Config::default());
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), IDS.len());
    // Truncated: every table cut short.
    let bytes = build(&base());
    for cut in [40, 200, 400, bytes.len() - 7] {
        let mut cut_bytes = bytes.clone();
        cut_bytes.truncate(cut);
        if let Ok(ctx) = BinaryCtx::new("f0", &cut_bytes, 0, &services) {
            let _ = audit_binary(&ctx, &checks, &Config::default());
        }
    }
}
