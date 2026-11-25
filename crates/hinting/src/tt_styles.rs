//! The autohinter's scripts: which zones a glyph is hinted with and which x-height scales them.
//!
//! Row: `autohint.tt.option.scripts`. ttfautohint measures zones per script and scales each script
//! by its own x-height; what follows is fitted to ttfautohint 1.8.4's output on synthetic fonts
//! (`docs/plans/M10.md`, M10.52), its source not read.
//!
//! The zones of a build form groups. Group 0 holds the master's TT zones, for Latin. A script
//! whose x-height lies in the master's `xHeight` zone joins it; any other script whose letters the
//! font draws has a group of its own, with zones measured on its letters. A group's zones scale
//! by the group's x-height, or only round ([`Script::scales`] off, or no x-height letter drawn).
//! Group [`NO_ZONES`] is for the glyphs of no script: their stems are hinted without zones.
//!
//! The heights of a script: the tops and bottoms of its encoded letters, lower case, capitals and
//! caseless letters apart, gathered into clusters of [`CLUSTER_EM`] em. A cluster of at least a
//! fifth of those letters is a zone from the median flat edge to the median round one. The
//! x-height is the lowest such top of the lower case, else the commonest top of caseless letters.
//! Which letters make a cluster is decided on the main master, so every master of a variable font
//! gets the same zones.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, glyph_props};

use crate::{HintSettings, MAX_TT_ZONES, TtAutohintOptions, TtZone};

/// A script the autohinter measures zones for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Script {
    /// Its tag in the TTFAutohint options.
    pub tag: &'static str,
    /// Its name, the Unicode Script property value in lower case.
    pub name: &'static str,
    /// Its zones scale by its x-height, as ttfautohint's output on a font of one script's letters
    /// shows.
    pub scales: bool,
}

const fn script(tag: &'static str, name: &'static str, scales: bool) -> Script {
    Script { tag, name, scales }
}

/// Latin first; its zones are the master's.
pub const SCRIPTS: [Script; 25] = [
    script("latn", "latin", true),
    script("cyrl", "cyrillic", true),
    script("grek", "greek", true),
    script("armn", "armenian", true),
    script("geor", "georgian", true),
    script("copt", "coptic", true),
    script("glag", "glagolitic", true),
    script("deva", "devanagari", true),
    script("beng", "bengali", true),
    script("guru", "gurmukhi", true),
    script("gujr", "gujarati", true),
    script("thai", "thai", true),
    script("lao", "lao", true),
    script("mymr", "myanmar", true),
    script("khmr", "khmer", true),
    script("hebr", "hebrew", false),
    script("arab", "arabic", false),
    script("taml", "tamil", false),
    script("telu", "telugu", false),
    script("knda", "kannada", false),
    script("mlym", "malayalam", false),
    script("sinh", "sinhala", false),
    script("ethi", "ethiopic", false),
    script("cher", "cherokee", false),
    script("tfng", "tifinagh", false),
];

/// The group of glyphs hinted without zones.
pub const NO_ZONES: usize = 1;
/// The first group of a script with zones of its own.
const FIRST_OWN: usize = 2;
/// Most letters of a script measured.
const MAX_LETTERS: usize = 512;
/// Most zones measured per case and side of a script.
const MAX_CLUSTERS: usize = 4;
/// Heights within this part of the em form one cluster.
const CLUSTER_EM: f64 = 0.03;
/// Heights closer than this are one, in font units.
const SAME_HEIGHT: f64 = 0.5;
/// Suffixes taken off an unencoded glyph's name to find the glyph it is an alternate of.
const MAX_SUFFIXES: usize = 8;

/// The zones of one group (indexes into the settings' list) and the one that scales them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneGroup {
    pub zones: Vec<usize>,
    /// The x-height zone; `None`: the zones only round.
    pub x_height: Option<usize>,
}

/// How a glyph is hinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphStyle {
    /// With the zones of this group.
    Group(usize),
    /// Not at all.
    Unhinted,
}

/// The zone groups of a build ([`crate::prepare_settings`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Styles {
    /// The groups of their own, from group [`FIRST_OWN`].
    pub own: Vec<ZoneGroup>,
    /// The group of each script that has zones, by its place in [`SCRIPTS`].
    scripts: BTreeMap<usize, usize>,
}

impl Styles {
    /// The group of zone `index`.
    pub fn zone_group(&self, index: usize) -> usize {
        self.own.iter().position(|g| g.zones.contains(&index)).map_or(0, |g| g.saturating_add(FIRST_OWN))
    }

    /// The groups of their own, with their numbers.
    pub fn own_groups(&self) -> impl Iterator<Item = (usize, &ZoneGroup)> {
        self.own.iter().enumerate().map(|(i, g)| (i.saturating_add(FIRST_OWN), g))
    }

    /// Moves the zones of the own groups `by` places down the list.
    pub(crate) fn shift(&mut self, by: usize) {
        for g in &mut self.own {
            g.zones.iter_mut().chain(g.x_height.as_mut()).for_each(|z| *z = z.saturating_add(by));
        }
    }

    /// The zones of `all` a glyph of `group` is hinted with.
    pub fn zones_of(&self, group: usize, all: &[usize]) -> Vec<usize> {
        if group == NO_ZONES {
            return Vec::new();
        }
        all.iter().copied().filter(|z| self.zone_group(*z) == group).collect()
    }

    /// How `glyph` is hinted: its script's group, an unencoded glyph of unknown script as the
    /// default script, any other by the fallback options.
    pub fn of(&self, font: &Font, glyph: &str, options: &TtAutohintOptions) -> GlyphStyle {
        let tagged = |tag: &str| SCRIPTS.iter().position(|s| s.tag == tag);
        let script = font.glyph(glyph).map_or(Lookup::None, |g| script_of(font, g, MAX_SUFFIXES));
        let script = match script {
            Lookup::Script(s) => Some(s),
            Lookup::Unencoded => tagged(&options.default_script),
            Lookup::None => None,
        };
        if let Some(group) = script.and_then(|s| self.scripts.get(&s)) {
            return GlyphStyle::Group(*group);
        }
        if options.fallback_scaling {
            return GlyphStyle::Unhinted;
        }
        GlyphStyle::Group(tagged(&options.fallback_script).and_then(|s| self.scripts.get(&s)).copied().unwrap_or(NO_ZONES))
    }
}

fn glyph_data() -> Option<&'static GlyphData> {
    static DATA: OnceLock<Option<GlyphData>> = OnceLock::new();
    DATA.get_or_init(|| GlyphData::new().ok()).as_ref()
}

/// Characters of no script that are hinted as Latin: the ones of ASCII and Latin-1, the general
/// punctuation and the currency signs up to the euro's column, as ttfautohint covers them.
fn common_latin(cp: u32) -> bool {
    matches!(cp, 0x21..=0xFF | 0x2010..=0x205E | 0x20A0..=0x20B8)
}

/// What [`script_of`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lookup {
    /// A script, by its place in [`SCRIPTS`].
    Script(usize),
    /// No script of [`SCRIPTS`].
    None,
    /// Nothing says: no character, and a name the glyph data gives no script.
    Unencoded,
}

/// The script of `glyph`: the one it stores, else that of its first character with one, else the
/// glyph data's for its name; an unencoded glyph without one has the script of the glyph its name
/// without the last suffix names.
fn script_of(font: &Font, glyph: &Glyph, suffixes: usize) -> Lookup {
    let Some(data) = glyph_data() else { return Lookup::None };
    let named = |name: &str| SCRIPTS.iter().position(|s| s.name.eq_ignore_ascii_case(name));
    let found = |s: Option<usize>| s.map_or(Lookup::None, Lookup::Script);
    if let Some(stored) = glyph.info.script.as_deref() {
        return found(named(stored));
    }
    if !glyph.unicodes.is_empty() {
        let own = glyph.unicodes.iter().find_map(|cp| data.unicode_script(*cp).and_then(named));
        return found(own.or_else(|| glyph.unicodes.iter().any(|cp| common_latin(*cp)).then_some(0)));
    }
    if let Some(name) = glyph_props(glyph, data, false).script.value.as_deref() {
        return found(named(name));
    }
    let base = glyph.name.rsplit_once('.').filter(|(base, _)| !base.is_empty()).and_then(|(base, _)| font.glyph(base));
    match (base, suffixes.checked_sub(1)) {
        (Some(base), Some(left)) => script_of(font, base, left),
        _ => Lookup::Unencoded,
    }
}

/// A letter of a script, as measured.
#[derive(Clone, Copy, Debug)]
struct Extreme {
    y: f64,
    round: bool,
}

/// One zone found: the letters whose extreme lies in it.
struct Cluster {
    letters: Vec<String>,
}

/// Case classes of letters, each measured apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Lower,
    Upper,
    Caseless,
}

/// The letters of each script but Latin that the main master draws, by class.
fn letters(font: &Font, master: &str) -> BTreeMap<usize, Vec<(Class, String)>> {
    let mut out: BTreeMap<usize, Vec<(Class, String)>> = BTreeMap::new();
    let Some(data) = glyph_data() else { return out };
    for g in font.glyphs.iter().filter(|g| !g.unicodes.is_empty()) {
        let Lookup::Script(s) = script_of(font, g, 0) else { continue };
        if s == 0 {
            continue;
        }
        let props = glyph_props(g, data, false);
        if props.category.value.as_deref() != Some("Letter") || g.layer(master).is_none_or(|l| l.shapes.is_empty()) {
            continue;
        }
        let class = match props.case.value.as_deref() {
            Some("lower") => Class::Lower,
            Some("upper") => Class::Upper,
            _ => Class::Caseless,
        };
        let list = out.entry(s).or_default();
        if list.len() < MAX_LETTERS {
            list.push((class, g.name.clone()));
        }
    }
    out
}

/// The top or bottom of `outline`: the highest (lowest) on-curve point, flat when a straight
/// segment runs along it, else round when a curve's handle beside it lies at its height.
fn extreme(outline: &BezPath, top: bool) -> Option<Extreme> {
    let better = |a: f64, b: f64| if top { a > b } else { a < b };
    let mut best: Option<f64> = None;
    let mut see = |p: Point| {
        if p.y.is_finite() && best.is_none_or(|b| better(p.y, b)) {
            best = Some(p.y);
        }
    };
    for el in outline.elements() {
        match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => see(p),
            PathEl::ClosePath => {}
        }
    }
    let y = best?;
    let at = |p: Point| (p.y - y).abs() <= SAME_HEIGHT;
    let (mut flat, mut round) = (false, false);
    let (mut from, mut start) = (Point::ZERO, Point::ZERO);
    for el in outline.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                (from, start) = (p, p);
            }
            PathEl::LineTo(p) => {
                flat |= at(from) && at(p) && (from.x - p.x).abs() > SAME_HEIGHT;
                from = p;
            }
            PathEl::QuadTo(c, p) => {
                round |= at(c) && (at(from) || at(p));
                from = p;
            }
            PathEl::CurveTo(a, b, p) => {
                round |= (at(a) && at(from)) || (at(b) && at(p));
                from = p;
            }
            PathEl::ClosePath => {
                flat |= at(from) && at(start) && (from.x - start.x).abs() > SAME_HEIGHT;
                from = start;
            }
        }
    }
    Some(Extreme { y, round: round && !flat })
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// The flat and the round edge of `letters` on `master`: medians, one standing for both when the
/// other has no letter or lies on the wrong side.
fn edges(font: &Font, master: &str, letters: &[String], top: bool) -> Option<(f64, f64)> {
    let found: Vec<Extreme> =
        letters.iter().filter_map(|name| font.glyph(name)).filter_map(|g| font.outline_of(g, master).ok()).filter_map(|o| extreme(&o, top)).collect();
    let of = |round: bool| median(found.iter().filter(|e| e.round == round).map(|e| e.y).collect());
    let (flat, round) = match (of(false), of(true)) {
        (Some(f), Some(r)) => (f, r),
        (Some(v), None) | (None, Some(v)) => (v, v),
        (None, None) => return None,
    };
    let overshoot = if top { round - flat } else { flat - round };
    if overshoot < 0.0 { Some(((flat + round) / 2.0, (flat + round) / 2.0)) } else { Some((flat, round)) }
}

/// The clusters of one class and side in order of height: those of at least a fifth of the
/// letters, the [`MAX_CLUSTERS`] largest.
fn clusters(font: &Font, master: &str, letters: &[&String], top: bool) -> Vec<Cluster> {
    let mut heights: Vec<(f64, &String)> =
        letters.iter().filter_map(|name| Some((extreme(&font.outline_of(font.glyph(name)?, master).ok()?, top)?.y, *name))).collect();
    heights.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    let reach = f64::from(font.units_per_em) * CLUSTER_EM;
    let mut found: Vec<(f64, Cluster)> = Vec::new();
    for (y, name) in heights {
        match found.last_mut() {
            Some((lowest, c)) if y - *lowest <= reach => c.letters.push(name.clone()),
            _ => found.push((y, Cluster { letters: vec![name.clone()] })),
        }
    }
    let least = letters.len().div_ceil(5);
    found.retain(|(_, c)| c.letters.len() >= least);
    let mut sizes: Vec<usize> = found.iter().map(|(_, c)| c.letters.len()).collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let smallest = sizes.get(MAX_CLUSTERS.saturating_sub(1)).copied().unwrap_or(0);
    found.retain(|(_, c)| c.letters.len() >= smallest);
    found.truncate(MAX_CLUSTERS);
    found.into_iter().map(|(_, c)| c).collect()
}

/// A measured zone of a script, on the main master and on the settings' master.
struct Measured {
    top: bool,
    /// (flat edge, round edge) on the main master, which decides what the zone is.
    main: (f64, f64),
    here: (f64, f64),
    letters: usize,
    class: Class,
}

fn measure(font: &Font, main: &str, here: &str, list: &[(Class, String)]) -> Vec<Measured> {
    let mut out = Vec::new();
    for class in [Class::Lower, Class::Upper, Class::Caseless] {
        let names: Vec<&String> = list.iter().filter(|(c, _)| *c == class).map(|(_, n)| n).collect();
        if names.is_empty() {
            continue;
        }
        for top in [false, true] {
            for c in clusters(font, main, &names, top) {
                let Some(on_main) = edges(font, main, &c.letters, top) else { continue };
                let on_here = if here == main { Some(on_main) } else { edges(font, here, &c.letters, top) };
                out.push(Measured { top, main: on_main, here: on_here.unwrap_or(on_main), letters: c.letters.len(), class });
            }
        }
    }
    out
}

/// The measured zone that is the script's x-height.
fn x_height(zones: &[Measured]) -> Option<usize> {
    let tops = || zones.iter().enumerate().filter(|(_, z)| z.top);
    let lower = tops().filter(|(_, z)| z.class == Class::Lower).min_by(|a, b| a.1.main.0.total_cmp(&b.1.main.0));
    lower.or_else(|| tops().filter(|(_, z)| z.class == Class::Caseless).max_by_key(|(_, z)| z.letters)).map(|(i, _)| i)
}

/// The zone of `zones` that holds both edges of `m`, on its side.
fn held(zones: &[TtZone], fuzz: f64, m: &Measured) -> Option<usize> {
    zones.iter().position(|z| {
        let (lo, hi) = (z.position.min(z.position + z.size) - fuzz, z.position.max(z.position + z.size) + fuzz);
        z.size != 0.0 && (z.size > 0.0) == m.top && [m.main.0, m.main.1].iter().all(|y| *y >= lo && *y <= hi)
    })
}

/// The TT zone of `m`, one unit high when its edges are one.
fn zone(name: String, m: &Measured) -> TtZone {
    let one = if m.top { 1.0 } else { -1.0 };
    let (flat, size) = (m.here.0.round(), (m.here.1 - m.here.0).round());
    TtZone { name, position: flat, size: if size == 0.0 { one } else { size }, ..TtZone::default() }
}

/// What [`measure_scripts`] adds.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ScriptZones {
    /// The zones of its groups are numbered from the first of `zones`.
    pub styles: Styles,
    /// A script that shares the master's `xHeight` zone draws an x-height letter.
    pub x_height: bool,
    /// A letter of one of the scripts has its top in the zone: `xHeight`, `capHeight`.
    pub holds: [bool; 2],
    /// Zones to append to the settings' list, in order.
    pub zones: Vec<TtZone>,
}

/// Measures the scripts other than Latin that the font draws; `main_zones` are the main master's
/// TT zones and `settings` the master's being built.
pub(crate) fn measure_scripts(font: &Font, settings: &HintSettings, main_zones: &[TtZone]) -> ScriptZones {
    let mut out = ScriptZones::default();
    out.styles.scripts.insert(0, 0);
    let main = if font.master(&settings.main_master).is_ok() { settings.main_master.as_str() } else { settings.master.as_str() };
    let fuzz = if settings.tt_blue_fuzz.is_finite() { settings.tt_blue_fuzz.max(0.0) } else { 0.0 };
    let named = |n: &str| main_zones.iter().position(|z| z.name == n);
    let (x_zone, cap_zone) = (named("xHeight"), named("capHeight"));
    let room = MAX_TT_ZONES.saturating_sub(settings.tt_zones.len());
    for (s, list) in letters(font, main) {
        let Some(script) = SCRIPTS.get(s) else { continue };
        let zones = measure(font, main, &settings.master, &list);
        if zones.is_empty() {
            continue;
        }
        let x = if script.scales { x_height(&zones) } else { None };
        let shared = x.is_some() && x_zone.is_some() && x.and_then(|x| zones.get(x)).and_then(|m| held(main_zones, fuzz, m)) == x_zone;
        let mut group = ZoneGroup::default();
        for (i, m) in zones.iter().enumerate() {
            if shared && let Some(at) = held(main_zones, fuzz, m) {
                for (hold, named) in out.holds.iter_mut().zip([x_zone, cap_zone]) {
                    *hold |= named == Some(at);
                }
                continue;
            }
            let next = out.zones.len();
            if next >= room {
                break;
            }
            let side = if m.top { "top" } else { "bottom" };
            out.zones.push(zone(format!("{} {side} {}", script.tag, i.saturating_add(1)), m));
            if x == Some(i) {
                group.x_height = Some(next);
            }
            group.zones.push(next);
        }
        if shared {
            out.x_height = true;
            out.styles.scripts.insert(s, 0);
        } else {
            out.styles.scripts.insert(s, out.styles.own.len().saturating_add(FIRST_OWN));
            out.styles.own.push(group);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use fontcraft_model::{Path, Shape};

    use super::*;
    use crate::tt_testfont as tt;

    /// A font with a box `height` units tall for each character.
    fn boxes(letters: &[(char, f64)]) -> Font {
        let mut font = Font::new("Scripts");
        for (c, height) in letters {
            let cp = u32::from(*c);
            let name = match font.glyphs.iter().find(|g| g.unicodes.contains(&cp)) {
                Some(g) => g.name.clone(),
                None => {
                    let name = format!("uni{cp:04X}");
                    let mut g = Glyph::new(&name);
                    g.unicodes.push(cp);
                    g.layers.push(fontcraft_model::Layer::new("m01", 600.0));
                    font.glyphs.push(g);
                    name
                }
            };
            font.glyph_mut(&name).unwrap().layer_mut("m01").unwrap().shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 450.0, *height))];
        }
        font
    }

    fn name_of(font: &Font, c: char) -> String {
        font.glyphs.iter().find(|g| g.unicodes.contains(&u32::from(c))).unwrap().name.clone()
    }

    fn top(font: &Font, c: char, options: &TtAutohintOptions, ppem: u16) -> f32 {
        let built = tt::autohinted(font, &name_of(font, c), options);
        tt::hinted_y(&built, ppem, tt::grayscale()).into_iter().fold(0.0, f32::max)
    }

    /// A Cyrillic-only font keeps the xHeight and capHeight zones and scales by them: ttfautohint
    /// puts a top of 516 on row 7 at 12 PPM (6.19 pixels, rounded up from 3/16).
    #[test]
    fn a_cyrillic_font_keeps_the_masters_zones() {
        let mut font = boxes(&[('п', 516.0), ('н', 516.0), ('Н', 700.0), ('П', 700.0)]);
        let settings = HintSettings::for_master(&font, "m01").unwrap();
        let mut prepared = settings.clone();
        let measured = crate::prepare_settings(&font, &mut prepared, &TtAutohintOptions::default());
        assert!(measured.left_out.is_empty() && measured.x_height, "{measured:?}");
        assert_eq!(prepared.tt_zones.len(), settings.tt_zones.len(), "nothing to add");
        let options = TtAutohintOptions::default();
        assert_eq!(top(&font, 'п', &options, 12), 7.0);
        let capital = name_of(&font, 'П');
        let mut g = font.glyph_mut(&capital).unwrap();
        g.unicodes.clear();
        drop(g);
        assert_eq!(measured.styles.of(&font, &capital, &options), GlyphStyle::Group(0), "by its name");
    }

    /// Two scripts of different x-heights scale apart: ttfautohint gives a Cyrillic x-height of 1100
    /// of 2000 the same rows beside a Latin one of 1000 and alone.
    #[test]
    fn scripts_scale_by_their_own_x_height() {
        let font = boxes(&[('x', 500.0), ('o', 500.0), ('п', 550.0), ('н', 550.0), ('т', 550.0)]);
        let options = TtAutohintOptions::default();
        let mut settings = HintSettings::for_master(&font, "m01").unwrap();
        let measured = crate::prepare_settings(&font, &mut settings, &options);
        assert_eq!(measured.styles.own.len(), 1);
        assert!(measured.styles.own[0].x_height.is_some());
        for (ppem, latin, cyrillic) in [(14, 8.0, 8.0), (16, 8.0, 9.0), (18, 9.0, 10.0), (20, 10.0, 11.0), (27, 14.0, 15.0)] {
            assert_eq!(top(&font, 'x', &options, ppem), latin, "x at {ppem} PPM");
            assert_eq!(top(&font, 'п', &options, ppem), cyrillic, "п at {ppem} PPM");
        }
    }

    /// A glyph of no script is hinted without zones; with a fallback script as one of it; with
    /// Fallback Scaling not at all.
    #[test]
    fn fallback_options_place_glyphs_of_no_script() {
        let font = boxes(&[('x', 500.0), ('o', 500.0), ('→', 500.0), ('«', 500.0)]);
        let mut settings = HintSettings::for_master(&font, "m01").unwrap();
        let styles = crate::prepare_settings(&font, &mut settings, &TtAutohintOptions::default()).styles;
        let style = |c: char, options: &str| styles.of(&font, &name_of(&font, c), &TtAutohintOptions::parse(options).unwrap().0);
        assert_eq!(style('«', ""), GlyphStyle::Group(0));
        assert_eq!(style('→', ""), GlyphStyle::Group(NO_ZONES));
        assert!(styles.zones_of(NO_ZONES, &[0, 1]).is_empty());
        assert_eq!(style('→', "--fallback-script=latn"), GlyphStyle::Group(0));
        assert_eq!(style('→', "--fallback-script=grek"), GlyphStyle::Group(NO_ZONES), "a script the font does not draw");
        assert_eq!(style('→', "--fallback-scaling"), GlyphStyle::Unhinted);
        assert_eq!(style('x', "--fallback-scaling"), GlyphStyle::Group(0));
        assert_eq!(styles.of(&font, ".notdef", &TtAutohintOptions::default()), GlyphStyle::Group(0), "unencoded: the default script");
    }
}
