//! Behaviour tests of the table checks and `check.outlines.pointsOutsideBBox` on fonts built here,
//! byte by byte.

use super::*;
use crate::{CheckSpec, Config, LayerReport, NoServices, Severity, audit_binary, find, select};

// --- bytes ------------------------------------------------------------------------------------

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
}

/// An sfnt with these tables (sorted by tag, padded to 4 bytes; checksums left at zero).
fn sfnt(version: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(t, _)| *t);
    let mut w = W::default();
    w.u32(version).u16(tables.len() as u16).u16(0).u16(0).u16(0);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        w.bytes(tag).u32(0).u32(offset as u32).u32(data.len() as u32);
        let mut padded = data.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        body.extend(padded);
    }
    w.bytes(&body);
    w.0
}

fn head(mac_style: u16, lowest_ppem: u16) -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0001_0000).u32(0x0001_0000).u32(0).u32(0x5F0F_3CF5).u16(0).u16(1000);
    w.bytes(&[0; 16]);
    w.i16(0).i16(-200).i16(1000).i16(800);
    w.u16(mac_style).u16(lowest_ppem).i16(2).i16(1).i16(0);
    assert_eq!(w.0.len(), 54);
    w.0
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0000_5000).u16(glyphs);
    w.0
}

fn hhea_hmtx(glyphs: u16) -> (Vec<u8>, Vec<u8>) {
    let mut h = W::default();
    h.u32(0x0001_0000).i16(800).i16(-200).i16(0).u16(600);
    for _ in 0..11 {
        h.i16(0);
    }
    h.u16(glyphs);
    let mut m = W::default();
    for _ in 0..glyphs {
        m.u16(600).i16(0);
    }
    (h.0, m.0)
}

fn name(records: &[(u16, &str)]) -> Vec<u8> {
    let mut strings = Vec::new();
    let mut w = W::default();
    w.u16(0).u16(records.len() as u16).u16(6 + 12 * records.len() as u16);
    for (id, s) in records {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        w.u16(3).u16(1).u16(0x409).u16(*id).u16(utf16.len() as u16).u16(strings.len() as u16);
        strings.extend(utf16);
    }
    w.bytes(&strings);
    w.0
}

#[derive(Clone)]
struct Os2 {
    version: u16,
    weight: u16,
    fs_type: u16,
    fs_selection: u16,
    vendor: [u8; 4],
    code_pages: [u32; 2],
    unicode_ranges: [u32; 4],
}

fn os2(o: &Os2) -> Vec<u8> {
    let mut w = W::default();
    w.u16(o.version).i16(500).u16(o.weight).u16(5).u16(o.fs_type);
    for _ in 0..10 {
        w.i16(0);
    }
    w.i16(0).bytes(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for r in o.unicode_ranges {
        w.u32(r);
    }
    w.bytes(&o.vendor).u16(o.fs_selection).u16(0x20).u16(0x41);
    w.i16(800).i16(-200).i16(0).u16(800).u16(200);
    w.u32(o.code_pages[0]).u32(o.code_pages[1]);
    w.i16(500).i16(700).u16(0).u16(0x20).u16(1);
    assert_eq!(w.0.len(), 96);
    w.0
}

/// `post` with this version; version 2 names the two glyphs `.notdef` and `A` from the standard
/// Macintosh set.
fn post(version: u32) -> Vec<u8> {
    let mut w = W::default();
    w.u32(version).u32(0).i16(-100).i16(50).u32(0).u32(0).u32(0).u32(0).u32(0);
    if version == 0x0002_0000 {
        w.u16(2).u16(0).u16(36);
    }
    w.0
}

/// A format 4 subtable with one segment per mapping.
fn cmap4(map: &[(u16, u16)]) -> Vec<u8> {
    let seg_count = map.len() as u16 + 1;
    let mut ends: Vec<u16> = map.iter().map(|m| m.0).collect();
    ends.push(0xFFFF);
    let mut w = W::default();
    let length = 16 + 8 * seg_count;
    let mut search = 1u16;
    let mut selector = 0u16;
    while search * 2 <= seg_count {
        search *= 2;
        selector += 1;
    }
    w.u16(4).u16(length).u16(0).u16(seg_count * 2).u16(search * 2).u16(selector).u16(seg_count * 2 - search * 2);
    for e in &ends {
        w.u16(*e);
    }
    w.u16(0);
    for e in &ends {
        w.u16(*e);
    }
    for (cp, g) in map {
        w.u16(g.wrapping_sub(*cp));
    }
    w.u16(1);
    for _ in 0..seg_count {
        w.u16(0);
    }
    w.0
}

fn cmap12(map: &[(u32, u32)]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(12).u16(0).u32(16 + 12 * map.len() as u32).u32(0).u32(map.len() as u32);
    for (cp, g) in map {
        w.u32(*cp).u32(*cp).u32(*g);
    }
    w.0
}

fn cmap(subtables: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(0).u16(subtables.len() as u16);
    let mut offset = 4 + 8 * subtables.len() as u32;
    for (p, e, data) in subtables {
        w.u16(*p).u16(*e).u32(offset);
        offset += data.len() as u32;
    }
    for (_, _, data) in subtables {
        w.bytes(data);
    }
    w.0
}

/// A one-contour simple glyph: (x, y, on-curve) points and the stored box.
fn simple_glyph(bbox: [i16; 4], points: &[(i16, i16, bool)]) -> Vec<u8> {
    let mut w = W::default();
    w.i16(1).i16(bbox[0]).i16(bbox[1]).i16(bbox[2]).i16(bbox[3]);
    w.u16(points.len() as u16 - 1).u16(0);
    for p in points {
        w.bytes(&[u8::from(p.2)]);
    }
    let (mut x, mut y) = (0i16, 0i16);
    for p in points {
        w.i16(p.0 - x);
        x = p.0;
    }
    for p in points {
        w.i16(p.1 - y);
        y = p.1;
    }
    if w.0.len() % 2 == 1 {
        w.0.push(0);
    }
    w.0
}

/// `glyf` and a long `loca` for these glyphs.
fn glyf_loca(glyphs: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
    let mut glyf = Vec::new();
    let mut loca = W::default();
    for g in glyphs {
        loca.u32(glyf.len() as u32);
        glyf.extend(g);
    }
    loca.u32(glyf.len() as u32);
    (glyf, loca.0)
}

fn base(tags: &[&[u8; 4]], script: &[u8; 4], default: u16, coords: &[i16]) -> Vec<u8> {
    let mut tag_list = W::default();
    tag_list.u16(tags.len() as u16);
    for t in tags {
        tag_list.bytes(*t);
    }
    // BaseScriptList: one record, then BaseScript, then BaseValues, then the coordinates.
    let mut values = W::default();
    values.u16(default).u16(coords.len() as u16);
    let first_coord = 4 + 2 * coords.len() as u16;
    for i in 0..coords.len() as u16 {
        values.u16(first_coord + 4 * i);
    }
    for c in coords {
        values.u16(1).i16(*c);
    }
    let mut script_list = W::default();
    script_list.u16(1).bytes(script).u16(8);
    script_list.u16(6).u16(0).u16(0);
    script_list.bytes(&values.0);
    let mut axis = W::default();
    axis.u16(4).u16(4 + tag_list.0.len() as u16);
    axis.bytes(&tag_list.0).bytes(&script_list.0);
    let mut w = W::default();
    w.u16(1).u16(0).u16(8).u16(0);
    w.bytes(&axis.0);
    w.0
}

fn meta(entries: &[(&[u8; 4], &str)]) -> Vec<u8> {
    let mut w = W::default();
    w.u32(1).u32(0).u32(0).u32(entries.len() as u32);
    let mut offset = 16 + 12 * entries.len() as u32;
    for (tag, text) in entries {
        w.bytes(*tag).u32(offset).u32(text.len() as u32);
        offset += text.len() as u32;
    }
    for (_, text) in entries {
        w.bytes(text.as_bytes());
    }
    w.0
}

/// STAT with one `wght` axis and these axis values (format 1).
fn stat(values: &[i32]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(1).u16(1).u16(8).u16(1).u32(20).u16(values.len() as u16).u32(28).u16(2);
    w.bytes(b"wght").u16(256).u16(0);
    for i in 0..values.len() as u16 {
        w.u16(2 * values.len() as u16 + 12 * i);
    }
    for (i, v) in values.iter().enumerate() {
        w.u16(1).u16(0).u16(0).u16(257 + i as u16).u32((*v as u32) << 16);
    }
    w.0
}

/// GSUB with a `zz01` feature and a `zzzz` language system under `latn`, no lookups.
fn gsub_zz() -> Vec<u8> {
    let mut w = W::default();
    w.u16(1).u16(0).u16(10).u16(34).u16(46);
    w.u16(1).bytes(b"latn").u16(8);
    w.u16(0).u16(1).bytes(b"zzzz").u16(10);
    w.u16(0).u16(0xFFFF).u16(0);
    w.u16(1).bytes(b"zz01").u16(8);
    w.u16(0).u16(0);
    w.u16(0);
    assert_eq!(w.0.len(), 48);
    w.0
}

// --- a TrueType font --------------------------------------------------------------------------

fn notdef() -> Vec<u8> {
    simple_glyph([50, 0, 450, 700], &[(50, 0, true), (50, 700, true), (450, 700, true), (450, 0, true)])
}

fn letter_a() -> Vec<u8> {
    simple_glyph([0, 0, 600, 700], &[(0, 0, true), (300, 700, true), (600, 0, true)])
}

/// The parts of a TrueType font the tests change.
struct Tt {
    names: Vec<(u16, &'static str)>,
    os2: Os2,
    mac_style: u16,
    lowest_ppem: u16,
    post: u32,
    cmap: Vec<(u16, u16, Vec<u8>)>,
    glyphs: Vec<Vec<u8>>,
    maxp_glyphs: Option<u16>,
    glyf_tail: usize,
    glyf_cut: usize,
    extra: Vec<([u8; 4], Vec<u8>)>,
    drop: Vec<[u8; 4]>,
}

/// A clean static TrueType font: upright Regular, weight 400, registered vendor, Latin 1 code
/// page, BASE with the ideographic baseline centring the cap height, meta with dlng and slng.
fn clean() -> Tt {
    Tt {
        names: vec![(1, "Test"), (2, "Regular")],
        os2: Os2 { version: 4, weight: 400, fs_type: 0, fs_selection: 0x40, vendor: *b"1ASC", code_pages: [1, 0], unicode_ranges: [1, 0, 0, 0] },
        mac_style: 0,
        lowest_ppem: 9,
        post: 0x0002_0000,
        cmap: vec![(0, 3, cmap4(&[(0x41, 1)])), (3, 1, cmap4(&[(0x41, 1)]))],
        glyphs: vec![notdef(), letter_a()],
        maxp_glyphs: None,
        glyf_tail: 0,
        glyf_cut: 0,
        extra: vec![
            (*b"gasp", vec![0, 1, 0, 1, 0xFF, 0xFF, 0, 0x0F]),
            (*b"BASE", base(&[b"romn", b"ideo"], b"latn", 0, &[0, -150])),
            (*b"meta", meta(&[(b"dlng", "Latn"), (b"slng", "Latn")])),
        ],
        drop: vec![],
    }
}

fn build(t: &Tt) -> Vec<u8> {
    let count = t.glyphs.len() as u16;
    let (mut glyf, loca) = glyf_loca(&t.glyphs);
    glyf.extend(vec![0; t.glyf_tail]);
    glyf.truncate(glyf.len() - t.glyf_cut);
    let (hhea, hmtx) = hhea_hmtx(count);
    let mut tables = vec![
        (*b"head", head(t.mac_style, t.lowest_ppem)),
        (*b"maxp", maxp(t.maxp_glyphs.unwrap_or(count))),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"name", name(&t.names)),
        (*b"OS/2", os2(&t.os2)),
        (*b"post", post(t.post)),
        (*b"cmap", cmap(&t.cmap)),
        (*b"glyf", glyf),
        (*b"loca", loca),
    ];
    tables.extend(t.extra.iter().cloned());
    tables.retain(|(tag, _)| !t.drop.contains(tag));
    sfnt(TRUETYPE, tables)
}

// --- a CFF font -------------------------------------------------------------------------------

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
        _ => int5(v),
    }
}

fn int5(v: i32) -> Vec<u8> {
    let mut o = vec![29];
    o.extend(v.to_be_bytes());
    o
}

/// A charstring from `(operands, operator bytes)` steps.
fn cs(steps: &[(&[i32], &[u8])]) -> Vec<u8> {
    let mut o = Vec::new();
    for (args, op) in steps {
        for a in *args {
            o.extend(dict_int(*a));
        }
        o.extend(*op);
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

#[derive(Default)]
struct Cff {
    charstrings: Vec<Vec<u8>>,
    global: Vec<Vec<u8>>,
    local: Vec<Vec<u8>>,
    top_extra: Vec<u8>,
    strings: Vec<Vec<u8>>,
}

fn cff_table(c: &Cff) -> Vec<u8> {
    let private = if c.local.is_empty() {
        Vec::new()
    } else {
        let mut p = int5(6);
        p.push(19);
        p
    };
    let top = |cs_off: i32, priv_off: i32| {
        let mut t = c.top_extra.clone();
        t.extend(int5(cs_off));
        t.push(17);
        t.extend(int5(private.len() as i32));
        t.extend(int5(priv_off));
        t.push(18);
        t
    };
    let names = index(&[b"Test".to_vec()]);
    let strings = index(&c.strings);
    let global = index(&c.global);
    let prefix = 4 + names.len() + index(&[top(0, 0)]).len() + strings.len() + global.len();
    let charstrings = index(&c.charstrings);
    let (cs_off, priv_off) = (prefix as i32, (prefix + charstrings.len()) as i32);
    let mut o = vec![1, 0, 4, 4];
    o.extend(names);
    o.extend(index(&[top(cs_off, priv_off)]));
    o.extend(strings);
    o.extend(global);
    assert_eq!(o.len(), prefix);
    o.extend(charstrings);
    o.extend(&private);
    if !c.local.is_empty() {
        o.extend(index(&c.local));
    }
    o
}

const ENDCHAR: &[u8] = &[14];

fn plain_glyph() -> Vec<u8> {
    cs(&[(&[0, 0], &[21]), (&[100, 0], &[5]), (&[], ENDCHAR)])
}

fn otf(c: &Cff, post_version: u32, weight: u16) -> Vec<u8> {
    let o = Os2 { weight, ..clean().os2 };
    sfnt(
        OTTO,
        vec![
            (*b"head", head(0, 3)),
            (*b"maxp", maxp(c.charstrings.len() as u16)),
            (*b"name", name(&[(1, "Test"), (2, "Regular")])),
            (*b"OS/2", os2(&o)),
            (*b"post", post(post_version)),
            (*b"CFF ", cff_table(c)),
        ],
    )
}

/// Global subroutines nested `n` deep: each calls the next, the last returns.
fn chain(n: usize, calls_each: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| {
            let mut s = Vec::new();
            if i + 1 < n {
                for _ in 0..calls_each {
                    s.extend(cs(&[(&[-107 + i as i32 + 1], &[29])]));
                }
            }
            s.push(11);
            s
        })
        .collect()
}

// --- running ----------------------------------------------------------------------------------

fn run_with(bytes: &[u8], pattern: &str, config: &Config) -> LayerReport {
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    let checks = select(None, &[pattern], &[]);
    assert!(!checks.is_empty(), "{pattern}");
    let r = audit_binary(&ctx, &checks, config);
    for f in &r.findings {
        assert!(find(f.check).is_some_and(|c| c.codes.contains(&f.code)), "{} emits unlisted `{}`", f.check, f.code);
        assert_eq!(f.location.font.as_deref(), Some("f0"));
        assert!(f.data.values().all(|v| v.is_finite()), "{f:?}");
    }
    r
}

fn run(bytes: &[u8], pattern: &str) -> LayerReport {
    run_with(bytes, pattern, &Config::default())
}

fn codes(bytes: &[u8], check: &str) -> Vec<&'static str> {
    run(bytes, check).findings.iter().map(|f| f.code).collect()
}

/// Runs a check's inner step with a sink of that check.
fn with_sink(check: &str, bytes: &[u8], step: impl FnOnce(&BinaryCtx<'_>, &mut Sink)) -> Vec<&'static str> {
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    let spec: &'static CheckSpec = find(check).unwrap();
    let mut sink = Sink::at(spec, Location::default());
    step(&ctx, &mut sink);
    let mut report = LayerReport::default();
    sink.finish(&mut report);
    report.findings.iter().map(|f| f.code).collect()
}

fn ours(id: &str) -> bool {
    id.starts_with("check.tables.") || id == "check.outlines.pointsOutsideBBox"
}

// --- tests ------------------------------------------------------------------------------------

#[test]
fn a_clean_truetype_font_passes_every_table_check() {
    let bytes = build(&clean());
    let mut r = run(&bytes, "check.tables.*");
    let extra = run(&bytes, "check.outlines.pointsOutsideBBox");
    r.findings.extend(extra.findings);
    let loud: Vec<_> = r.findings.iter().filter(|f| ours(f.check) && f.severity >= Severity::Warn).collect();
    assert!(loud.is_empty(), "{loud:#?}");
    let info: BTreeSet<&str> = r.findings.iter().map(|f| f.code).collect();
    assert_eq!(info, BTreeSet::from(["optional-tables", "missing-recommended-table", "dlng-tag", "slng-tag"]));
    // The checks that need a project value say why they did not run.
    for id in ["check.tables.unsupportedTables", "check.tables.fsType"] {
        assert!(r.skipped.iter().any(|s| s.check == id && s.reason.contains("project value")), "{id}");
    }
}

#[test]
fn missing_and_unwanted_tables_are_reported() {
    let mut t = clean();
    t.drop.push(*b"name");
    t.extra.extend([
        (*b"FFTM", vec![0; 8]),
        (*b"TSI1", vec![0; 4]),
        (*b"morx", vec![0; 4]),
        (*b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]),
        (*b"GSUB", gsub_zz()),
    ]);
    let bytes = build(&t);
    let r = run(&bytes, "check.tables.requiredTables");
    let missing: Vec<_> = r.findings.iter().filter(|f| f.code == "required-tables").collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].location.table.as_deref(), Some("name"));
    assert_eq!(missing[0].severity, Severity::Fail);

    let r = run(&bytes, "check.tables.unwantedTables");
    let mut found: Vec<(&str, String)> = r.findings.iter().map(|f| (f.code, f.location.table.clone().unwrap_or_default())).collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            ("unwanted-table", "FFTM".to_string()),
            ("unwanted-table", "TSI1".to_string()),
            ("zz-feature", "GSUB".to_string()),
            ("zz-language-system", "GSUB".to_string()),
        ]
    );
    assert!(r.findings.iter().filter(|f| f.code.starts_with("zz-")).all(|f| f.fix.is_none()), "zz entries cannot be dropped as tables");
    assert!(r.findings.iter().filter(|f| f.code == "unwanted-table").all(|f| f.fix.is_some_and(|x| x.id == "dropTables")));

    assert_eq!(codes(&bytes, "check.tables.unwantedAatTables"), vec!["unwanted-aat-table"]);
    assert_eq!(codes(&bytes, "check.tables.dsig"), vec!["found-dsig"]);
}

#[test]
fn the_allow_list_reports_tables_outside_it() {
    let bytes = build(&clean());
    let present = |ctx: &BinaryCtx<'_>| table_tags(ctx);
    let allowed: Vec<[u8; 4]> =
        [b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"glyf", b"loca", b"gasp", b"BASE"].map(|t| *t).to_vec();
    let found = with_sink("check.tables.unsupportedTables", &bytes, |ctx, sink| report_unsupported(&present(ctx), &allowed, sink));
    assert_eq!(found, vec!["unsupported-table"], "meta is not on the list");
    assert_eq!(padded_tag("CFF"), Some(*b"CFF "));
    assert_eq!(padded_tag("toolong"), None);
}

#[test]
fn post_formats_follow_the_outline_kind() {
    let tt = |version| build(&Tt { post: version, ..clean() });
    assert_eq!(codes(&tt(0x0003_0000), "check.tables.postVersion"), vec!["format-3"]);
    assert_eq!(run(&tt(0x0003_0000), "check.tables.postVersion").findings[0].severity, Severity::Warn);
    assert_eq!(codes(&tt(0x0002_5000), "check.tables.postVersion"), vec!["format-2-5"]);
    assert_eq!(codes(&tt(0x0004_0000), "check.tables.postVersion"), vec!["format-4"]);
    assert!(codes(&tt(0x0002_0000), "check.tables.postVersion").is_empty());
    let c = Cff { charstrings: vec![plain_glyph()], ..Cff::default() };
    assert_eq!(codes(&otf(&c, 0x0002_0000, 400), "check.tables.postVersion"), vec!["cff-not-format-3"]);
    assert!(codes(&otf(&c, 0x0003_0000, 400), "check.tables.postVersion").is_empty());
}

#[test]
fn damaged_glyf_and_loca_are_reported() {
    let short = build(&Tt { maxp_glyphs: Some(3), ..clean() });
    assert_eq!(codes(&short, "check.tables.glyfIntegrity"), vec!["loca-too-short"]);
    let tail = build(&Tt { glyf_tail: 8, ..clean() });
    assert_eq!(codes(&tail, "check.tables.glyfIntegrity"), vec!["unreachable-data"]);
    let padded = build(&Tt { glyf_tail: 2, ..clean() });
    assert!(codes(&padded, "check.tables.glyfIntegrity").is_empty(), "up to 3 bytes of padding are fine");
    let cut = build(&Tt { glyf_cut: 4, ..clean() });
    assert_eq!(codes(&cut, "check.tables.glyfIntegrity"), vec!["missing-data"]);
    // A trailing empty glyph is never missing data.
    let empty_last = build(&Tt { glyphs: vec![notdef(), letter_a(), Vec::new()], ..clean() });
    assert!(codes(&empty_last, "check.tables.glyfIntegrity").is_empty());
}

#[test]
fn cmap_subtables_are_judged() {
    let both = Tt { cmap: vec![(3, 1, cmap4(&[(0x41, 1), (0x42, 1)])), (3, 10, cmap12(&[(0x41, 1)]))], ..clean() };
    let mut found = codes(&build(&both), "check.tables.cmapFormat12");
    found.sort_unstable();
    assert_eq!(found, vec!["pointless-format-12", "unmapped-from-4"]);
    let r = run(&build(&both), "check.tables.cmapFormat12");
    assert_eq!(r.findings.iter().find(|f| f.code == "unmapped-from-4").map(|f| f.severity), Some(Severity::Warn));

    let alone = Tt { cmap: vec![(3, 10, cmap12(&[(0x41, 1), (0x1F600, 1)]))], ..clean() };
    assert_eq!(codes(&build(&alone), "check.tables.cmapFormat12"), vec!["no-cmap-4"]);
    let good = Tt { cmap: vec![(3, 1, cmap4(&[(0x41, 1)])), (3, 10, cmap12(&[(0x41, 1), (0x1F600, 1)]))], ..clean() };
    assert!(codes(&build(&good), "check.tables.cmapFormat12").is_empty());
    assert!(codes(&build(&good), "check.tables.cmapSubtables").is_empty());

    let symbol = Tt { cmap: vec![(3, 1, cmap4(&[(0x41, 1)])), (3, 0, cmap4(&[(0xF041, 1)]))], ..clean() };
    let r = run(&build(&symbol), "check.tables.cmapSubtables");
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.platform)).collect::<Vec<_>>(), vec![("unexpected-subtable", Some(3))]);
}

#[test]
fn style_bits_follow_the_names_and_agree_with_mac_style() {
    let bold_named = Tt { names: vec![(1, "Test"), (2, "Bold")], ..clean() };
    let mut found = codes(&build(&bold_named), "check.tables.fsSelection");
    found.sort_unstable();
    assert_eq!(found, vec!["bad-bold", "bad-regular"]);

    let mac = Tt { mac_style: 0x02, ..clean() };
    let r = run(&build(&mac), "check.tables.fsSelection");
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.field.as_deref())).collect::<Vec<_>>(), vec![("macstyle-italic", Some("macStyle"))]);

    let italic = Tt { names: vec![(1, "Test"), (2, "Italic")], os2: Os2 { fs_selection: 0x01, ..clean().os2 }, mac_style: 0x02, ..clean() };
    assert!(codes(&build(&italic), "check.tables.fsSelection").is_empty());

    let nameless = Tt { names: vec![(1, "Test")], ..clean() };
    let r = run(&build(&nameless), "check.tables.fsSelection");
    assert!(r.findings.is_empty() && r.skipped.len() == 1);
}

#[test]
fn vendor_ids_are_compared_with_the_project_or_the_registry() {
    let with = |vendor: [u8; 4]| build(&Tt { os2: Os2 { vendor, ..clean().os2 }, ..clean() });
    assert_eq!(codes(&with(*b"UKWN"), "check.tables.vendorId"), vec!["placeholder"]);
    assert_eq!(codes(&with(*b"    "), "check.tables.vendorId"), vec!["not-set"]);
    assert_eq!(codes(&with([0; 4]), "check.tables.vendorId"), vec!["not-set"]);
    assert_eq!(codes(&with(*b"Q~Q~"), "check.tables.vendorId"), vec!["unregistered"]);
    assert_eq!(run(&with(*b"Q~Q~"), "check.tables.vendorId").findings[0].severity, Severity::Warn);

    let bytes = with(*b"AB  ");
    let found = with_sink("check.tables.vendorId", &bytes, |ctx, sink| report_vendor_id(ctx, *b"AB  ", padded_tag("ABCD"), sink));
    assert_eq!(found, vec!["bad-vendor-id"]);
    let found = with_sink("check.tables.vendorId", &bytes, |ctx, sink| report_vendor_id(ctx, *b"AB\0\0", padded_tag("AB"), sink));
    assert!(found.is_empty(), "NUL padding equals space padding");
}

#[test]
fn code_page_and_range_bits() {
    let empty = Tt { os2: Os2 { code_pages: [0, 0], ..clean().os2 }, ..clean() };
    assert_eq!(codes(&build(&empty), "check.tables.codePageRanges"), vec!["no-code-pages"]);
    let mut no_os2 = clean();
    no_os2.drop.push(*b"OS/2");
    assert_eq!(codes(&build(&no_os2), "check.tables.codePageRanges"), vec!["no-os2-table"]);

    // Cyrillic declared for a font that maps only A; Basic Latin and bit 57 as stored.
    let wrong = Tt { os2: Os2 { code_pages: [1 << 2, 0], unicode_ranges: [0, 1 << 25, 0, 0], ..clean().os2 }, ..clean() };
    assert!(codes(&build(&wrong), "check.tables.codePageRanges").is_empty(), "only empty fields count unless strict");
    let mut config = Config::default();
    config.set("check.tables.codePageRanges", "strict", 1.0).unwrap();
    let r = run_with(&build(&wrong), "check.tables.codePageRanges", &config);
    let bits: Vec<(&str, i64)> = r.findings.iter().map(|f| (f.code, f.data.get("bit").map_or(-1, |b| *b as i64))).collect();
    assert_eq!(bits, vec![("bad-code-page-bit", 2), ("bad-range-bit", 0), ("bad-range-bit", 57), ("non-plane-0-without-format-12", -1)]);
    assert!(r.findings.iter().all(|f| f.severity == Severity::Warn));
}

#[test]
fn os2_fields_and_lowest_size() {
    let old = Tt { os2: Os2 { version: 3, ..clean().os2 }, ..clean() };
    assert_eq!(codes(&build(&old), "check.tables.os2Version"), vec!["old-version"]);
    assert_eq!(codes(&build(&Tt { lowest_ppem: 3, ..clean() }), "check.tables.lowestRecPpem"), vec!["small-lowest-ppem"]);
    let mut config = Config::default();
    config.set("check.tables.lowestRecPpem", "minimum", 2.0).unwrap();
    assert!(run_with(&build(&Tt { lowest_ppem: 3, ..clean() }), "check.tables.lowestRecPpem", &config).findings.is_empty());

    let bytes = build(&clean());
    let found = with_sink("check.tables.fsType", &bytes, |ctx, sink| report_fs_type(ctx, 0, 8, sink));
    assert_eq!(found, vec!["unexpected-value"]);
    assert!(with_sink("check.tables.fsType", &bytes, |ctx, sink| report_fs_type(ctx, 8, 8, sink)).is_empty());
}

#[test]
fn weight_classes_follow_the_style_and_outlines() {
    let weight = |w: u16, style: &'static str, fs: u16| {
        build(&Tt { names: vec![(1, "Test"), (2, style)], os2: Os2 { weight: w, fs_selection: fs, ..clean().os2 }, ..clean() })
    };
    assert_eq!(codes(&weight(450, "Regular", 0x40), "check.tables.weightClassRange"), vec!["bad-value"]);
    assert_eq!(codes(&weight(400, "Bold", 0x20), "check.tables.weightClassRange"), vec!["bold-below-500", "bad-value"]);
    assert!(codes(&weight(700, "Bold", 0x20), "check.tables.weightClassRange").is_empty());
    assert_eq!(codes(&weight(400, "Hairline", 0x40), "check.tables.weightClassRange"), vec!["no-value"]);
    let semi = build(&Tt {
        names: vec![(1, "Test"), (2, "Regular"), (16, "Test"), (17, "Semi Bold Italic")],
        os2: Os2 { weight: 600, ..clean().os2 },
        ..clean()
    });
    assert!(codes(&semi, "check.tables.weightClassRange").is_empty(), "Semi Bold reads as SemiBold");
    assert!(codes(&weight(100, "Thin", 0x40), "check.tables.weightClassRange").is_empty(), "a TrueType Thin may be 100");

    let c = Cff { charstrings: vec![plain_glyph()], ..Cff::default() };
    assert_eq!(codes(&otf(&c, 0x0003_0000, 200), "check.tables.weightClassRange"), vec!["cff-below-250", "bad-value"]);
}

#[test]
fn cff_subroutine_depth_and_recursion() {
    let call_first = || cs(&[(&[-107], &[29]), (&[], ENDCHAR)]);
    let ten = Cff { charstrings: vec![call_first()], global: chain(10, 1), ..Cff::default() };
    assert!(codes(&otf(&ten, 0x0003_0000, 400), "check.tables.cffCallDepth").is_empty(), "ten levels are allowed");
    let eleven = Cff { charstrings: vec![call_first()], global: chain(11, 1), ..Cff::default() };
    assert_eq!(codes(&otf(&eleven, 0x0003_0000, 400), "check.tables.cffCallDepth"), vec!["max-depth"]);

    let recursive = Cff { charstrings: vec![cs(&[(&[-107], &[10]), (&[], ENDCHAR)])], local: vec![cs(&[(&[-107], &[10, 11])])], ..Cff::default() };
    let r = run(&otf(&recursive, 0x0003_0000, 400), "check.tables.cffCallDepth");
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.table.as_deref())).collect::<Vec<_>>(), vec![("recursion", Some("CFF "))]);
}

#[test]
fn deprecated_cff_operators_are_reported() {
    let seac = cs(&[(&[0, 0, 0, 65, 97], ENDCHAR)]);
    let dot = cs(&[(&[0, 0], &[21]), (&[], &[12, 0]), (&[100, 0], &[5]), (&[], ENDCHAR)]);
    // An endchar inside a subroutine counts too.
    let seac_in_subr = cs(&[(&[0, 0, 0, 65, 97, -107], &[29])]);
    let c = Cff { charstrings: vec![plain_glyph(), seac, dot, seac_in_subr], global: vec![vec![14]], ..Cff::default() };
    let r = run(&otf(&c, 0x0003_0000, 400), "check.tables.cffDeprecatedOperators");
    let found: Vec<(&str, Severity)> = r.findings.iter().map(|f| (f.code, f.severity)).collect();
    assert_eq!(found, vec![("endchar-seac", Severity::Fail), ("dotsection", Severity::Warn), ("endchar-seac", Severity::Fail)]);
}

#[test]
fn cff_dictionary_and_charstring_limits() {
    let mut long = cs(&[(&[0, 0], &[21])]);
    for _ in 0..11_000 {
        long.extend(cs(&[(&[1, 1], &[5])]));
    }
    long.push(14);
    let mut top = dict_int(1234);
    top.push(13);
    top.extend(dict_int(1));
    top.extend(dict_int(2));
    top.push(14);
    top.extend(dict_int(5));
    top.extend([12, 35]);
    top.extend(dict_int(391));
    top.push(1); // Notice: the first custom string
    top.extend(dict_int(395));
    top.push(0x0C); // escape …
    top.push(0); // … 0: Copyright, a string that does not exist
    let c = Cff { charstrings: vec![plain_glyph(), long], top_extra: top, strings: vec!["Café".as_bytes().to_vec()], ..Cff::default() };
    let bytes = otf(&c, 0x0003_0000, 400);
    assert_eq!(codes(&bytes, "check.tables.cffCharstringSize"), vec!["charstring-too-long"]);
    assert_eq!(codes(&bytes, "check.tables.cffUniqueIds"), vec!["unique-id", "xuid", "uid-base"]);
    let mut strings = codes(&bytes, "check.tables.cffAsciiStrings");
    strings.sort_unstable();
    assert_eq!(strings, vec!["cannot-decode", "not-ascii"]);
    let clean_cff = otf(&Cff { charstrings: vec![plain_glyph()], ..Cff::default() }, 0x0003_0000, 400);
    for id in [
        "check.tables.cffCharstringSize",
        "check.tables.cffUniqueIds",
        "check.tables.cffAsciiStrings",
        "check.tables.cffDeprecatedOperators",
        "check.tables.cffCallDepth",
    ] {
        assert!(codes(&clean_cff, id).is_empty(), "{id}");
    }
}

#[test]
fn base_meta_and_stat_tables() {
    let with_base = |b: Vec<u8>| {
        let mut t = clean();
        t.extra.retain(|(tag, _)| tag != b"BASE");
        t.extra.push((*b"BASE", b));
        build(&t)
    };
    let mut no_base = clean();
    no_base.drop.push(*b"BASE");
    assert_eq!(codes(&build(&no_base), "check.tables.baseTable"), vec!["missing-base"]);
    assert_eq!(codes(&with_base(base(&[b"romn", b"ideo"], b"latn", 0, &[0, -100])), "check.tables.baseTable"), vec!["ideo-not-centred"]);
    let mut config = Config::default();
    config.set("check.tables.baseTable", "tolerance", 60.0).unwrap();
    assert!(run_with(&with_base(base(&[b"romn", b"ideo"], b"latn", 0, &[0, -100])), "check.tables.baseTable", &config).findings.is_empty());
    assert_eq!(codes(&with_base(base(&[b"romn", b"ideo"], b"latn", 0, &[5, -150])), "check.tables.baseTable"), vec!["default-baseline-offset"]);
    assert_eq!(codes(&with_base(base(&[b"romn", b"ideo"], b"latn", 0, &[0])), "check.tables.baseTable"), vec!["coord-count-mismatch"]);
    let mut found = codes(&with_base(base(&[b"romn", b"xyzw"], b"qqqq", 0, &[0, 0])), "check.tables.baseTable");
    found.sort_unstable();
    assert_eq!(found, vec!["unregistered-baseline-tag", "unregistered-script-tag"]);

    let with_meta = |m: Option<Vec<u8>>| {
        let mut t = clean();
        t.extra.retain(|(tag, _)| tag != b"meta");
        t.extra.extend(m.map(|m| (*b"meta", m)));
        build(&t)
    };
    let r = run(&with_meta(None), "check.tables.metaScriptLangTags");
    assert_eq!(r.findings.iter().map(|f| (f.code, f.severity)).collect::<Vec<_>>(), vec![("lacks-meta-table", Severity::Warn)]);
    let r = run(&with_meta(Some(meta(&[(b"dlng", "Latn, Grek")]))), "check.tables.metaScriptLangTags");
    assert_eq!(
        r.findings.iter().map(|f| (f.code, f.severity)).collect::<Vec<_>>(),
        vec![("dlng-tag", Severity::Info), ("missing-slng-tag", Severity::Fail)]
    );
    assert!(r.findings.first().is_some_and(|f| f.message.contains("Latn, Grek")));

    let with_stat = |values: &[i32]| {
        let mut t = clean();
        t.extra.push((*b"STAT", stat(values)));
        build(&t)
    };
    let r = run(&with_stat(&[400, 700]), "check.tables.statInStatics");
    assert_eq!(r.findings.iter().map(|f| (f.code, f.location.axis.as_deref())).collect::<Vec<_>>(), vec![("multiple-stat-entries", Some("wght"))]);
    assert!(run(&with_stat(&[400]), "check.tables.statInStatics").findings.is_empty());
}

#[test]
fn points_outside_the_stored_box_are_reported() {
    // A's off-curve point at the top pokes 50 units above the stored box.
    let a = simple_glyph([0, 0, 600, 700], &[(0, 0, true), (300, 750, false), (600, 0, true)]);
    let bytes = build(&Tt { glyphs: vec![notdef(), a], ..clean() });
    let r = run(&bytes, "check.outlines.pointsOutsideBBox");
    assert_eq!(r.findings.len(), 1);
    let f = &r.findings[0];
    assert_eq!((f.code, f.location.glyph.as_str(), f.location.point), ("points-out-of-bounds", "A", Some([300.0, 750.0])));
    assert_eq!(f.location.bounds, Some([0.0, 0.0, 600.0, 700.0]));
    assert_eq!(f.fix.map(|x| x.id), Some("patchTable"));
}

#[test]
fn every_entry_matches_its_inventory_row_and_profile() {
    let ids: Vec<&str> = crate::CHECKS.iter().filter(|c| ours(c.id)).map(|c| c.id).collect();
    assert_eq!(ids.len(), 26);
    let count = |p| select(Some(p), &["check.tables.*"], &[]).len();
    assert_eq!(count(crate::Profile::Opentype), 10);
    assert_eq!(count(crate::Profile::Universal), 14, "4 universal and 10 opentype table checks");
    assert_eq!(count(crate::Profile::Strict), 25);
    for spec in crate::CHECKS.iter().filter(|c| ours(c.id)) {
        for (code, _) in spec.code_severities {
            assert!(spec.codes.contains(code), "{} {code}", spec.id);
        }
        assert!(matches!(spec.run, crate::Runner::Binary(_)), "{}", spec.id);
    }
}

#[test]
fn hostile_fonts_do_not_panic() {
    let fanout = Cff { charstrings: vec![cs(&[(&[-107], &[29]), (&[], ENDCHAR)])], global: chain(10, 40), ..Cff::default() };
    let r = run(&otf(&fanout, 0x0003_0000, 400), "check.tables.cffCallDepth");
    assert!(r.skipped.iter().any(|s| s.reason.contains("operations")), "an exponential call tree stops at the budget");

    let mut fonts = vec![
        build(&clean()),
        otf(&Cff { charstrings: vec![plain_glyph()], global: chain(3, 2), local: vec![vec![11]], ..Cff::default() }, 0x0003_0000, 400),
    ];
    // Charstrings that end inside a number, call without operands, and overflow the stack.
    let mut overflow = Vec::new();
    for _ in 0..600 {
        overflow.extend(dict_int(1));
    }
    overflow.push(14);
    fonts.push(otf(&Cff { charstrings: vec![vec![28, 1], vec![10, 29, 19, 19], overflow, vec![255, 0]], ..Cff::default() }, 0x0003_0000, 400));
    let checks: Vec<_> = crate::CHECKS.iter().filter(|c| ours(c.id)).collect();
    let services = NoServices;
    for bytes in &fonts {
        for cut in (12..bytes.len()).step_by(7) {
            let Ok(ctx) = BinaryCtx::new("f0", &bytes[..cut], 0, &services) else { continue };
            let _ = audit_binary(&ctx, &checks, &Config::default());
        }
        let mut flipped = bytes.clone();
        for (i, b) in flipped.iter_mut().enumerate().skip(12) {
            if i % 5 == 0 {
                *b ^= 0xA5;
            }
        }
        if let Ok(ctx) = BinaryCtx::new("f0", &flipped, 0, &services) {
            let _ = audit_binary(&ctx, &checks, &Config::default());
        }
    }
}
