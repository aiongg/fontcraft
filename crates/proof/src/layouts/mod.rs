//! The layouts ported from the reference proof script (rows `proof`, `font.proof`): the drawing
//! settings every tile layout reads, and the layouts themselves.
//!
//! - [`tiles`]: `boxes`, `hints` and `digi`, glyphs in tiles with em boxes, advance boxes, centre
//!   marks, stem hints, alignment zones, point marks and labels, and a block of metrics text.
//! - [`hints`]: the stem hints, hint-replacement groups and alignment zones of a `CFF ` font.
//! - [`compare`]: a font set, one row per font, glyphs of the same index in one column.
//! - [`linked`]: the glyph grid whose cells link to the outline pages that follow it.
//! - [`spot`]: the sheets of the reference table dumper: glyph synopsis, `cmap`, vertical origins,
//!   and text set as outlines; [`features`] draws the rules of `GSUB` and `GPOS` features.
//!
//! Settings and presets follow `fontpdf.py` and `proofpdf.py` of AFDKO 5.0.1 (Apache License 2.0,
//! Copyright 2014 Adobe; see `NOTICE`): the setting names, their defaults and the elements each
//! layout turns on. Positions inside a tile are our own. The elements `show` and `hide` switch
//! ([`ELEMENTS`]) are names of ours for groups of those settings; the dumper's sheets read the
//! same [`Display`], so its proof policies (no names, no numbers, em box, bounding box) are
//! `hide name`, `hide numeric-labels`, `show em-box` and `show bbox`.

pub(crate) mod compare;
pub(crate) mod features;
pub(crate) mod hints;
pub(crate) mod linked;
pub(crate) mod spot;
pub(crate) mod tiles;

#[cfg(test)]
mod layout_tests;

use crate::page::TITLE_SIZE;
use crate::{Panel, ProofError, ProofOptions, Sheet};

/// Most fonts one font-set proof compares.
pub const MAX_SET_FONTS: usize = 256;
/// Largest number a size or padding setting takes, in tile units (the glyph square is 1000).
const MAX_SETTING: f64 = 10_000.0;

/// Which side bearings a tile prints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Metrics {
    /// Left and right side bearings and the advance width.
    #[default]
    Horizontal,
    /// Top and bottom side bearings and the vertical advance.
    Vertical,
    Both,
}

/// The value of one drawing setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingValue {
    /// A number; for a switch, 0 is off and anything else on.
    Number(f64),
    /// A colour, each part from 0 to 1.
    Rgb([f64; 3]),
}

impl SettingValue {
    /// Reads `12`, `0`, `true`, `false`, or a colour `(0, 0.75, 0)` (the brackets are optional).
    pub fn parse(text: &str) -> Option<SettingValue> {
        let t = text.trim();
        match t {
            "true" | "on" => return Some(SettingValue::Number(1.0)),
            "false" | "off" => return Some(SettingValue::Number(0.0)),
            _ => {}
        }
        let inner = t.strip_prefix('(').and_then(|r| r.strip_suffix(')')).unwrap_or(t);
        let parts: Vec<f64> = inner.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|v| v.is_finite())).collect::<Option<Vec<f64>>>()?;
        match parts.as_slice() {
            [v] => Some(SettingValue::Number(*v)),
            [r, g, b] => Some(SettingValue::Rgb([*r, *g, *b])),
            _ => None,
        }
    }
}

/// One drawing setting as `--list-settings` prints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub name: &'static str,
    /// The default before a layout's own choices.
    pub default: &'static str,
    pub about: &'static str,
}

/// Every drawing setting, by its name in the reference proof script. Sizes and paddings are in
/// tile units, where the glyph square is 1000; the page title size is in points.
pub const SETTINGS: &[Setting] = &[
    Setting { name: "drawGlyph_EMBox", default: "1", about: "draw the em box" },
    Setting { name: "drawGlyph_Outline", default: "1", about: "draw the glyph" },
    Setting { name: "drawGlyph_HHints", default: "1", about: "draw horizontal stem hints (CFF fonts)" },
    Setting { name: "drawGlyph_VHints", default: "1", about: "draw vertical stem hints (CFF fonts)" },
    Setting { name: "drawGlyph_BlueZones", default: "1", about: "draw the alignment zones of BlueValues (CFF fonts)" },
    Setting { name: "drawGlyph_Baseline", default: "1", about: "draw the baseline" },
    Setting { name: "drawGlyph_CenterMarks", default: "0", about: "draw marks at the middle of the advance and of the em height" },
    Setting { name: "drawGlyph_CenterMarksWithBox", default: "0", about: "draw the centre marks and the advance-width box" },
    Setting { name: "drawGlyph_GlyphBox", default: "0", about: "draw a box of advance width and em height" },
    Setting { name: "drawGlyph_XAdvance", default: "0", about: "draw ticks at the origin and the advance" },
    Setting { name: "drawGlyph_ContourLabels", default: "1", about: "label the start of each contour (P 0, P 1 …)" },
    Setting { name: "drawPoint_PointMarks", default: "1", about: "mark on-curve points" },
    Setting { name: "drawPoint_BCPMarks", default: "1", about: "mark off-curve points and draw their handles" },
    Setting { name: "drawPoint_PointLabels", default: "1", about: "label on-curve points" },
    Setting { name: "pointLabel_doPointType", default: "1", about: "a point label holds the point type (m, l, c, q)" },
    Setting { name: "pointLabel_doPointIndex", default: "1", about: "a point label holds the point index" },
    Setting { name: "pointLabel_doPosition", default: "1", about: "a point label holds the point position" },
    Setting { name: "drawMeta_Outline", default: "1", about: "show a small filled copy of the glyph under it" },
    Setting { name: "drawMeta_Name", default: "1", about: "show the glyph name" },
    Setting { name: "drawMeta_BBox", default: "1", about: "show the glyph's bounding box" },
    Setting { name: "drawMeta_SideBearings", default: "1", about: "show the left and right side bearings and the advance width" },
    Setting { name: "drawMeta_V_SideBearings", default: "1", about: "show the top and bottom side bearings and the vertical advance" },
    Setting { name: "drawMeta_Parts", default: "1", about: "show the number of moves, lines and curves" },
    Setting { name: "drawMeta_Paths", default: "1", about: "show the number of contours" },
    Setting { name: "drawMeta_Hints", default: "1", about: "show the number of stem hints" },
    Setting { name: "DrawFilledOutline", default: "0", about: "fill the glyph instead of outlining it" },
    Setting { name: "pageIncludeTitle", default: "1", about: "print the title line at the top of each page" },
    Setting { name: "pageTitleSize", default: "9", about: "size of the title line, in points (at most 24)" },
    Setting { name: "pointLabelSize", default: "16", about: "size of point, contour, hint and zone labels" },
    Setting { name: "metaDataTextSize", default: "24", about: "size of the metrics text under a glyph" },
    Setting { name: "metaDataNameSize", default: "48", about: "size of the glyph name under a glyph" },
    Setting { name: "metaDataFilledGlyphSize", default: "250", about: "size of the small filled copy of the glyph" },
    Setting { name: "glyphHPadding", default: "150", about: "space left and right of the glyph square" },
    Setting { name: "glyphVPadding", default: "150", about: "space above and below the glyph square" },
    Setting { name: "hhintXOffset", default: "10", about: "how far horizontal hints reach past the em box, in font units" },
    Setting { name: "vhintYOffset", default: "10", about: "how far vertical hints reach past the em box, in font units" },
    Setting { name: "fontsetGroupPtSize", default: "14", about: "size of the group titles and font names of the compare layout, in points" },
    Setting { name: "hintColorRGB", default: "(0, 0.75, 0)", about: "colour of stem hints" },
    Setting { name: "hintColorOverlapRGB", default: "(0.6, 0.6, 0)", about: "colour of a stem hint that overlaps the one before it" },
    Setting { name: "alignmentZoneColorRGB", default: "(0, 0, 0.75)", about: "colour of alignment zones" },
    Setting { name: "emboxColorRGB", default: "(0.25, 0.25, 0.5)", about: "colour of the em box and the advance-width box" },
    Setting { name: "baselineColorRGB", default: "(0.5, 0.25, 0.25)", about: "colour of the baseline" },
    Setting { name: "xAdvanceColorRGB", default: "(0.5, 0, 0)", about: "colour of advance ticks and centre marks" },
    Setting { name: "contourLabelColorRGB", default: "(0.5, 0, 0)", about: "colour of contour labels" },
    Setting { name: "pointLabelColorRGB", default: "(0.5, 0, 0)", about: "colour of point marks and labels" },
    Setting { name: "MetaDataGlyphColorRGB", default: "(0, 0, 0)", about: "colour of the metrics text" },
];

/// The drawing elements `show` and `hide` take, with what each one switches.
pub const ELEMENTS: &[(&str, &str)] = &[
    ("em-box", "the em box"),
    ("bbox", "the glyph's bounding box, drawn around it"),
    ("advance", "ticks at the origin and the advance"),
    ("baseline", "the baseline"),
    ("zones", "the alignment zones (CFF fonts)"),
    ("hhints", "horizontal stem hints (CFF fonts)"),
    ("vhints", "vertical stem hints (CFF fonts)"),
    ("points", "on-curve point marks"),
    ("handles", "off-curve point marks and their handles"),
    ("point-labels", "point labels"),
    ("contour-labels", "contour start labels"),
    ("center-marks", "marks at the middle of the advance and of the em height"),
    ("name", "the glyph name"),
    ("numeric-labels", "glyph ids, code points, advances and adjustment values next to glyphs"),
    ("sidebearings", "the side bearings and advances in the metrics text"),
    ("hint-counts", "the number of stem hints in the metrics text"),
    ("path-counts", "the numbers of contours, moves, lines and curves in the metrics text"),
    ("filled-thumbnail", "the small filled copy of the glyph under it"),
];

/// What a tile draws: the settings of [`SETTINGS`], after a layout's choices and the caller's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Display {
    pub em_box: bool,
    pub outline: bool,
    pub hhints: bool,
    pub vhints: bool,
    pub blue_zones: bool,
    pub baseline: bool,
    pub center_marks: bool,
    pub center_marks_with_box: bool,
    pub glyph_box: bool,
    /// A box around the glyph's bounds (element `bbox`; no setting of the reference script).
    pub glyph_bbox: bool,
    pub x_advance: bool,
    pub contour_labels: bool,
    pub point_marks: bool,
    pub bcp_marks: bool,
    pub point_labels: bool,
    pub label_type: bool,
    pub label_index: bool,
    pub label_position: bool,
    pub meta_outline: bool,
    pub meta_name: bool,
    pub meta_bbox: bool,
    pub meta_side_bearings: bool,
    pub meta_v_side_bearings: bool,
    pub meta_parts: bool,
    pub meta_paths: bool,
    pub meta_hints: bool,
    /// Glyph ids, code points and values beside the glyphs of the dumper's sheets (element
    /// `numeric-labels`).
    pub numeric_labels: bool,
    pub filled: bool,
    pub page_title: bool,
    pub page_title_size: f64,
    pub point_label_size: f64,
    pub meta_text_size: f64,
    pub meta_name_size: f64,
    pub meta_glyph_size: f64,
    pub h_padding: f64,
    pub v_padding: f64,
    pub hhint_offset: f64,
    pub vhint_offset: f64,
    pub group_title_size: f64,
    pub hint_colour: [u8; 3],
    pub hint_overlap_colour: [u8; 3],
    pub zone_colour: [u8; 3],
    pub embox_colour: [u8; 3],
    pub baseline_colour: [u8; 3],
    pub x_advance_colour: [u8; 3],
    pub contour_label_colour: [u8; 3],
    pub point_label_colour: [u8; 3],
    pub meta_colour: [u8; 3],
}

/// Where a setting's value goes.
enum Slot<'a> {
    Switch(&'a mut bool),
    Number(&'a mut f64, f64),
    Colour(&'a mut [u8; 3]),
}

/// A colour of the reference script (parts from 0 to 1) as bytes.
fn rgb(c: [f64; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

impl Default for Display {
    /// The reference script's defaults, before a layout's choices.
    fn default() -> Self {
        Display {
            em_box: true,
            outline: true,
            hhints: true,
            vhints: true,
            blue_zones: true,
            baseline: true,
            center_marks: false,
            center_marks_with_box: false,
            glyph_box: false,
            glyph_bbox: false,
            x_advance: false,
            contour_labels: true,
            point_marks: true,
            bcp_marks: true,
            point_labels: true,
            label_type: true,
            label_index: true,
            label_position: true,
            meta_outline: true,
            meta_name: true,
            meta_bbox: true,
            meta_side_bearings: true,
            meta_v_side_bearings: true,
            meta_parts: true,
            meta_paths: true,
            meta_hints: true,
            numeric_labels: true,
            filled: false,
            page_title: true,
            page_title_size: TITLE_SIZE,
            point_label_size: 16.0,
            meta_text_size: 24.0,
            meta_name_size: 48.0,
            meta_glyph_size: 250.0,
            h_padding: 150.0,
            v_padding: 150.0,
            hhint_offset: 10.0,
            vhint_offset: 10.0,
            group_title_size: 14.0,
            hint_colour: rgb([0.0, 0.75, 0.0]),
            hint_overlap_colour: rgb([0.6, 0.6, 0.0]),
            zone_colour: rgb([0.0, 0.0, 0.75]),
            embox_colour: rgb([0.25, 0.25, 0.5]),
            baseline_colour: rgb([0.5, 0.25, 0.25]),
            x_advance_colour: rgb([0.5, 0.0, 0.0]),
            contour_label_colour: rgb([0.5, 0.0, 0.0]),
            point_label_colour: rgb([0.5, 0.0, 0.0]),
            meta_colour: [0, 0, 0],
        }
    }
}

impl Display {
    /// The settings of each copy a sheet draws (two for `Digi`, one otherwise): the layout's
    /// choices, then `black`, `show`, `hide`, `metrics`, `settings` and the `panels`.
    pub(crate) fn for_sheet(sheet: &Sheet, options: &ProofOptions) -> Result<Vec<Display>, ProofError> {
        let mut copies = preset(sheet);
        for d in &mut copies {
            d.apply_common(options)?;
        }
        for Panel { index, show, hide, settings } in &options.panels {
            let count = copies.len();
            let Some(d) = copies.get_mut(*index) else {
                return Err(ProofError::BadOptions(format!("panel {index}: this layout draws {count} copy of each glyph, numbered from 0")));
            };
            d.show_all(show, true)?;
            d.show_all(hide, false)?;
            for (name, value) in settings {
                d.set(name, *value)?;
            }
        }
        Ok(copies)
    }

    /// The settings that decide the title line of every page.
    pub(crate) fn for_title(options: &ProofOptions) -> Result<Display, ProofError> {
        let mut d = Display::default();
        d.apply_common(options)?;
        Ok(d)
    }

    fn apply_common(&mut self, options: &ProofOptions) -> Result<(), ProofError> {
        if options.black {
            self.make_black();
        }
        self.show_all(&options.show, true)?;
        self.show_all(&options.hide, false)?;
        if let Some(m) = options.metrics {
            self.meta_side_bearings = m != Metrics::Vertical;
            self.meta_v_side_bearings = m != Metrics::Horizontal;
        }
        for (name, value) in &options.settings {
            self.set(name, *value)?;
        }
        Ok(())
    }

    fn show_all(&mut self, elements: &[String], on: bool) -> Result<(), ProofError> {
        for e in elements {
            self.show(e, on)?;
        }
        Ok(())
    }

    /// Turns the element `name` of [`ELEMENTS`] on or off; an unknown name is refused.
    pub(crate) fn show(&mut self, name: &str, on: bool) -> Result<(), ProofError> {
        let fields: Vec<&mut bool> = match name {
            "em-box" => vec![&mut self.em_box],
            "bbox" => vec![&mut self.glyph_bbox],
            "advance" => vec![&mut self.x_advance],
            "baseline" => vec![&mut self.baseline],
            "zones" => vec![&mut self.blue_zones],
            "hhints" => vec![&mut self.hhints],
            "vhints" => vec![&mut self.vhints],
            "points" => vec![&mut self.point_marks],
            "handles" => vec![&mut self.bcp_marks],
            "point-labels" => vec![&mut self.point_labels],
            "contour-labels" => vec![&mut self.contour_labels],
            "center-marks" => vec![&mut self.center_marks],
            "name" => vec![&mut self.meta_name],
            "numeric-labels" => vec![&mut self.numeric_labels],
            "sidebearings" => vec![&mut self.meta_side_bearings, &mut self.meta_v_side_bearings],
            "hint-counts" => vec![&mut self.meta_hints],
            "path-counts" => vec![&mut self.meta_paths, &mut self.meta_parts],
            "filled-thumbnail" => vec![&mut self.meta_outline],
            _ => {
                let names: Vec<&str> = ELEMENTS.iter().map(|(n, _)| *n).collect();
                return Err(ProofError::BadOptions(format!("there is no drawing element `{name}`; the elements are {}", names.join(", "))));
            }
        };
        for f in fields {
            *f = on;
        }
        Ok(())
    }

    /// The reference's `-black`: marks and text black, hints and zones mid grey.
    fn make_black(&mut self) {
        for c in [
            &mut self.embox_colour,
            &mut self.baseline_colour,
            &mut self.x_advance_colour,
            &mut self.contour_label_colour,
            &mut self.point_label_colour,
            &mut self.meta_colour,
        ] {
            *c = [0, 0, 0];
        }
        for c in [&mut self.hint_colour, &mut self.hint_overlap_colour, &mut self.zone_colour] {
            *c = rgb([0.5, 0.5, 0.5]);
        }
    }

    fn slot(&mut self, name: &str) -> Option<Slot<'_>> {
        use Slot::{Colour, Number, Switch};
        Some(match name {
            "drawGlyph_EMBox" => Switch(&mut self.em_box),
            "drawGlyph_Outline" => Switch(&mut self.outline),
            "drawGlyph_HHints" => Switch(&mut self.hhints),
            "drawGlyph_VHints" => Switch(&mut self.vhints),
            "drawGlyph_BlueZones" => Switch(&mut self.blue_zones),
            "drawGlyph_Baseline" => Switch(&mut self.baseline),
            "drawGlyph_CenterMarks" => Switch(&mut self.center_marks),
            "drawGlyph_CenterMarksWithBox" => Switch(&mut self.center_marks_with_box),
            "drawGlyph_GlyphBox" => Switch(&mut self.glyph_box),
            "drawGlyph_XAdvance" => Switch(&mut self.x_advance),
            "drawGlyph_ContourLabels" => Switch(&mut self.contour_labels),
            "drawPoint_PointMarks" => Switch(&mut self.point_marks),
            "drawPoint_BCPMarks" => Switch(&mut self.bcp_marks),
            "drawPoint_PointLabels" => Switch(&mut self.point_labels),
            "pointLabel_doPointType" => Switch(&mut self.label_type),
            "pointLabel_doPointIndex" => Switch(&mut self.label_index),
            "pointLabel_doPosition" => Switch(&mut self.label_position),
            "drawMeta_Outline" => Switch(&mut self.meta_outline),
            "drawMeta_Name" => Switch(&mut self.meta_name),
            "drawMeta_BBox" => Switch(&mut self.meta_bbox),
            "drawMeta_SideBearings" => Switch(&mut self.meta_side_bearings),
            "drawMeta_V_SideBearings" => Switch(&mut self.meta_v_side_bearings),
            "drawMeta_Parts" => Switch(&mut self.meta_parts),
            "drawMeta_Paths" => Switch(&mut self.meta_paths),
            "drawMeta_Hints" => Switch(&mut self.meta_hints),
            "DrawFilledOutline" => Switch(&mut self.filled),
            "pageIncludeTitle" => Switch(&mut self.page_title),
            "pageTitleSize" => Number(&mut self.page_title_size, crate::page::HEADER),
            "pointLabelSize" => Number(&mut self.point_label_size, MAX_SETTING),
            "metaDataTextSize" => Number(&mut self.meta_text_size, MAX_SETTING),
            "metaDataNameSize" => Number(&mut self.meta_name_size, MAX_SETTING),
            "metaDataFilledGlyphSize" => Number(&mut self.meta_glyph_size, MAX_SETTING),
            "glyphHPadding" => Number(&mut self.h_padding, MAX_SETTING),
            "glyphVPadding" => Number(&mut self.v_padding, MAX_SETTING),
            "hhintXOffset" => Number(&mut self.hhint_offset, MAX_SETTING),
            "vhintYOffset" => Number(&mut self.vhint_offset, MAX_SETTING),
            "fontsetGroupPtSize" => Number(&mut self.group_title_size, crate::page::HEADER * 4.0),
            "hintColorRGB" => Colour(&mut self.hint_colour),
            "hintColorOverlapRGB" => Colour(&mut self.hint_overlap_colour),
            "alignmentZoneColorRGB" => Colour(&mut self.zone_colour),
            "emboxColorRGB" => Colour(&mut self.embox_colour),
            "baselineColorRGB" => Colour(&mut self.baseline_colour),
            "xAdvanceColorRGB" => Colour(&mut self.x_advance_colour),
            "contourLabelColorRGB" => Colour(&mut self.contour_label_colour),
            "pointLabelColorRGB" => Colour(&mut self.point_label_colour),
            "MetaDataGlyphColorRGB" => Colour(&mut self.meta_colour),
            _ => return None,
        })
    }

    /// Sets the setting `name`; an unknown name or a value of the wrong kind is refused.
    pub(crate) fn set(&mut self, name: &str, value: SettingValue) -> Result<(), ProofError> {
        let bad = |what: &str| ProofError::BadOptions(format!("setting `{name}` {what}"));
        match (self.slot(name), value) {
            (None, _) => Err(ProofError::BadOptions(format!("there is no drawing setting `{name}`; the settings list shows them all"))),
            (Some(Slot::Switch(b)), SettingValue::Number(v)) => {
                *b = v != 0.0;
                Ok(())
            }
            (Some(Slot::Number(n, max)), SettingValue::Number(v)) => {
                if !(0.0..=max).contains(&v) {
                    return Err(bad(&format!("takes a number from 0 to {max}")));
                }
                *n = v;
                Ok(())
            }
            (Some(Slot::Colour(c)), SettingValue::Rgb(v)) => {
                if !v.iter().all(|p| (0.0..=1.0).contains(p)) {
                    return Err(bad("takes a colour with parts from 0 to 1, such as (0, 0.75, 0)"));
                }
                *c = rgb(v);
                Ok(())
            }
            (Some(Slot::Colour(_)), SettingValue::Number(_)) => Err(bad("takes a colour such as (0, 0.75, 0)")),
            (Some(_), SettingValue::Rgb(_)) => Err(bad("takes a number")),
        }
    }
}

/// Glyphs per page of a tile layout when neither a size nor a count is given, as the reference.
pub(crate) fn default_per_page(sheet: &Sheet) -> usize {
    match sheet {
        Sheet::Boxes => 156,
        Sheet::Hints => 4,
        Sheet::Synopsis => spot::SYNOPSIS_PER_PAGE,
        _ => 1,
    }
}

/// The reference's choices for each layout, on top of [`Display::default`].
fn preset(sheet: &Sheet) -> Vec<Display> {
    let mut d = Display::default();
    match sheet {
        Sheet::Boxes => {
            d.no_overlays();
            d.center_marks_with_box = true;
            d.filled = true;
            d.only_name_and_side_bearings();
            vec![d]
        }
        Sheet::Hints => {
            d.baseline = false;
            d.contour_labels = false;
            d.point_labels = false;
            d.meta_outline = false;
            d.meta_parts = false;
            d.meta_paths = false;
            d.meta_bbox = false;
            d.meta_v_side_bearings = false;
            d.point_label_size = 32.0;
            vec![d]
        }
        Sheet::Digi => {
            d.hhints = false;
            d.vhints = false;
            d.blue_zones = false;
            d.baseline = false;
            d.x_advance = true;
            d.center_marks = true;
            d.contour_labels = false;
            d.point_marks = false;
            d.bcp_marks = false;
            d.point_labels = false;
            d.h_padding = 0.0;
            d.v_padding = 0.0;
            let mut right = d.clone();
            // The left copy shows no metrics text: the reference prints error-log notes there.
            for m in [
                &mut d.meta_outline,
                &mut d.meta_name,
                &mut d.meta_bbox,
                &mut d.meta_side_bearings,
                &mut d.meta_v_side_bearings,
                &mut d.meta_parts,
                &mut d.meta_paths,
                &mut d.meta_hints,
            ] {
                *m = false;
            }
            right.em_box = false;
            right.x_advance = false;
            right.contour_labels = true;
            right.center_marks = false;
            right.point_marks = true;
            right.bcp_marks = true;
            right.point_labels = true;
            vec![d, right]
        }
        Sheet::Compare => {
            d.no_overlays();
            d.filled = true;
            d.only_name_and_side_bearings();
            d.meta_name = false;
            d.meta_side_bearings = false;
            vec![d]
        }
        // The dumper's sheets: filled glyphs with name and numbers; the em box, bounding box and
        // hints only when asked for. Advance ticks mark each glyph's width on the line-set
        // sheets and the vertical advance on `vorg`.
        Sheet::Synopsis | Sheet::Cmap | Sheet::Features { .. } | Sheet::Vorg | Sheet::Text { .. } => {
            d.no_overlays();
            d.filled = true;
            d.only_name_and_side_bearings();
            d.meta_side_bearings = false;
            match sheet {
                Sheet::Cmap | Sheet::Features { .. } => d.x_advance = true,
                Sheet::Vorg => {
                    d.x_advance = true;
                    d.em_box = true;
                }
                Sheet::Text { .. } => {
                    d.meta_name = false;
                    d.numeric_labels = false;
                }
                _ => {}
            }
            vec![d]
        }
        _ => vec![d],
    }
}

impl Display {
    /// Turns off hints, zones, baseline, em box, advance ticks and point drawing.
    fn no_overlays(&mut self) {
        for b in [
            &mut self.hhints,
            &mut self.vhints,
            &mut self.blue_zones,
            &mut self.baseline,
            &mut self.em_box,
            &mut self.x_advance,
            &mut self.contour_labels,
            &mut self.point_marks,
            &mut self.bcp_marks,
            &mut self.point_labels,
        ] {
            *b = false;
        }
    }

    fn only_name_and_side_bearings(&mut self) {
        for b in [
            &mut self.meta_outline,
            &mut self.meta_bbox,
            &mut self.meta_v_side_bearings,
            &mut self.meta_parts,
            &mut self.meta_paths,
            &mut self.meta_hints,
        ] {
            *b = false;
        }
        self.meta_name = true;
        self.meta_side_bearings = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_setting_is_read_and_lists_its_true_default() {
        for s in SETTINGS {
            let mut d = Display::default();
            let value = SettingValue::parse(s.default).unwrap_or_else(|| panic!("{} default {}", s.name, s.default));
            let before = d.clone();
            d.set(s.name, value).unwrap_or_else(|e| panic!("{}: {e}", s.name));
            assert_eq!(d, before, "{} lists {} but defaults to something else", s.name, s.default);
        }
        let mut d = Display::default();
        assert!(d.set("pageTitleFont", SettingValue::Number(1.0)).is_err(), "a setting we do not draw is refused");
        assert!(d.set("hintColorRGB", SettingValue::Number(1.0)).is_err());
        assert!(d.set("pointLabelSize", SettingValue::Number(-3.0)).is_err());
        assert_eq!(SettingValue::parse("( 1.0, 0, 0)"), Some(SettingValue::Rgb([1.0, 0.0, 0.0])));
        assert_eq!(SettingValue::parse("1,2"), None);
    }

    #[test]
    fn panels_change_one_copy_of_the_digi_layout() {
        let options = ProofOptions {
            panels: vec![Panel {
                index: 1,
                hide: vec!["handles".into()],
                show: vec!["em-box".into()],
                settings: vec![("drawPoint_PointLabels".into(), SettingValue::Number(0.0))],
            }],
            black: true,
            ..ProofOptions::default()
        };
        let copies = Display::for_sheet(&Sheet::Digi, &options).unwrap();
        assert_eq!(copies.len(), 2);
        assert!(copies[0].em_box && copies[1].em_box, "the right copy shows its em box when asked");
        assert!(!copies[1].point_labels && copies[1].point_marks && !copies[1].bcp_marks);
        assert_eq!(copies[0].hint_colour, [128, 128, 128]);
        let wrong = ProofOptions { panels: vec![Panel { index: 1, ..Panel::default() }], ..ProofOptions::default() };
        assert!(Display::for_sheet(&Sheet::Hints, &wrong).is_err());
    }

    #[test]
    fn show_and_hide_switch_elements_and_settings_come_after_them() {
        let options = ProofOptions {
            show: vec!["em-box".into(), "bbox".into()],
            hide: vec!["name".into(), "numeric-labels".into(), "sidebearings".into()],
            settings: vec![("drawMeta_Name".into(), SettingValue::Number(1.0))],
            ..ProofOptions::default()
        };
        let d = &Display::for_sheet(&Sheet::Synopsis, &options).unwrap()[0];
        assert!(d.em_box && d.glyph_bbox && !d.numeric_labels && !d.meta_side_bearings && !d.meta_v_side_bearings);
        assert!(d.meta_name, "a setting overrides hide");
        let every: Vec<String> = ELEMENTS.iter().map(|(n, _)| n.to_string()).collect();
        assert!(Display::for_sheet(&Sheet::Boxes, &ProofOptions { hide: every, ..ProofOptions::default() }).is_ok());
        let bad = ProofOptions { show: vec!["sparkles".into()], ..ProofOptions::default() };
        assert!(Display::for_sheet(&Sheet::Boxes, &bad).is_err());
    }
}
