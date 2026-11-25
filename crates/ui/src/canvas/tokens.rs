//! The canvas's colours and sizes: the one place that holds their values.
//!
//! The colours and the handle size that Settings › Appearance lists come from the user's settings
//! (`appearance.color.<item>.light`/`.dark`, `appearance.handleSize`); their defaults are the setting defaults in
//! `fontcraft_engine::settings`. The set used is the one of the appearance the canvas is drawn in
//! (`app::appearance`); the colours no setting covers have a light and a dark value here. Node, handle and anchor sizes scale with the handle-size step.
//! `appearance.guideOpacity` fades metric lines, their names, alignment zones and guides.

use egui::Color32;
use fontcraft_engine::color::export;
use fontcraft_engine::model::Master;
use fontcraft_engine::settings::UserSettings;

/// The master custom parameters that colour the canvas while their master is the active one.
pub const MASTER_BACKGROUND: &str = "Master Background Color";
pub const MASTER_BACKGROUND_DARK: &str = "Master Background Color Dark";

/// The canvas's colours and sizes for one appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    /// The canvas behind the glyphs (`appearance.color.canvas`).
    pub canvas: Color32,
    /// The active master's background (Master Background Color, or its Dark form on a dark canvas), painted over
    /// `canvas`; `None` when the master sets none it can be read from.
    pub master_canvas: Option<Color32>,
    /// The edited layer's outline (`appearance.color.foregroundStroke`).
    pub outline: Color32,
    /// The paths as drawn under a composite that strokes or masks change.
    pub source_outline: Color32,
    pub metric_line: Color32,
    pub metric_name: Color32,
    /// Alignment zones, as bands (`appearance.color.alignmentZones`).
    pub zone_fill: Color32,
    /// Corner on-curve nodes (`appearance.color.cornerNodes`).
    pub node_corner: Color32,
    /// Smooth on-curve nodes (`appearance.color.smoothNodes`).
    pub node_smooth: Color32,
    pub node_handle: Color32,
    pub handle_line: Color32,
    /// The strokes across the ends of an open path, and the triangle at its first node.
    pub open_end: Color32,
    pub open_end_start: Color32,
    pub duplicate_node: Color32,
    /// Behind a node on a metric line (diamond) or in an alignment zone (circle).
    pub metric_highlight: Color32,
    /// Extra Nodes: where paths cross.
    pub extra_node: Color32,
    pub component_aligned: Color32,
    pub component_manual: Color32,
    pub component_shiftable: Color32,
    pub component_placeholder: Color32,
    pub anchor: Color32,
    /// The faint band along a corner or cap component's fitted outline.
    pub attachment_band: Color32,
    /// The band of a selected corner component.
    pub attachment_band_selected: Color32,
    /// The other side of the layer under Show Background (`appearance.color.backgroundStroke`).
    pub background_outline: Color32,
    /// Another visible layer of the glyph (`appearance.color.otherLayersStroke`).
    pub visible_layer_outline: Color32,
    /// The reference font's glyph behind the outline (`view.setReferenceFont`).
    pub reference_outline: Color32,
    /// Laid over the canvas while the background is the side being edited.
    pub background_tint: Color32,
    /// Text mode's tint of a gap the kerning closes (`appearance.color.kerningNegative`).
    pub kerning_negative: Color32,
    /// Text mode's tint of a gap the kerning opens (`appearance.color.kerningPositive`).
    pub kerning_positive: Color32,
    /// How opaque guides are drawn, from 0 to 1 (`appearance.guideOpacity`); the metric line and zone colours above
    /// already carry it.
    pub guide_opacity: f32,

    pub outline_width: f32,
    pub source_outline_width: f32,
    pub metric_line_width: f32,
    pub handle_line_width: f32,
    /// Side of a corner node.
    pub node_size: f32,
    pub smooth_node_radius: f32,
    pub handle_radius: f32,
    pub start_arrow_size: f32,
    pub open_end_length: f32,
    pub duplicate_node_radius: f32,
    pub metric_highlight_size: f32,
    pub extra_node_radius: f32,
    pub component_outline_width: f32,
    /// The outline of a selected component.
    pub component_selected_width: f32,
    /// How opaque the fill of a component in its state's colour is, from 0 to 1.
    pub component_fill_opacity: f32,
    /// How opaque the mark cloud is, from 0 to 1, and the outlines of its marks on the edited glyph.
    pub mark_cloud_opacity: f32,
    pub mark_cloud_outline_opacity: f32,
    /// Width and height of an anchor marker.
    pub anchor_size: f32,
    /// Width of the band along a corner or cap component.
    pub attachment_band_width: f32,
    /// Text size of metric names, anchor names and placeholder labels.
    pub label_size: f32,
    /// Room between a line or a marker and its label.
    pub label_gap: f32,
    pub background_outline_width: f32,
    pub visible_layer_outline_width: f32,
    pub reference_outline_width: f32,
    /// Share of the canvas left free on each side when a glyph is fitted into it.
    pub fit_margin: f64,
    /// The Master Compatibility view (`canvas::compatibility`).
    pub compat: CompatTokens,
}

/// The handle-size steps of `appearance.handleSize` and how much each scales nodes, handles and anchors.
pub const HANDLE_SIZES: [(&str, f32); 3] = [("small", 0.75), ("regular", 1.0), ("large", 1.5)];

impl Default for Tokens {
    /// The tokens with every setting at its default.
    fn default() -> Self {
        Tokens::from_settings(&UserSettings::new(), false)
    }
}

impl Tokens {
    /// The tokens for the settings `settings`, in the dark appearance when `dark`.
    pub fn from_settings(settings: &UserSettings, dark: bool) -> Tokens {
        let color = |item: &str, standard: Color32| {
            let id = format!("appearance.color.{item}.{}", if dark { "dark" } else { "light" });
            settings.get(&id).ok().and_then(|v| v.as_str().and_then(parse_color)).unwrap_or(standard)
        };
        let step = settings.get("appearance.handleSize").ok().and_then(|v| v.as_str().map(str::to_owned));
        let scale = HANDLE_SIZES.iter().find(|(name, _)| step.as_deref() == Some(*name)).map_or(1.0, |(_, s)| *s);
        let mut tokens = Tokens::standard(dark);
        tokens.canvas = color("canvas", tokens.canvas);
        tokens.outline = color("foregroundStroke", tokens.outline);
        tokens.zone_fill = color("alignmentZones", tokens.zone_fill);
        tokens.node_corner = color("cornerNodes", tokens.node_corner);
        tokens.node_smooth = color("smoothNodes", tokens.node_smooth);
        tokens.background_outline = color("backgroundStroke", tokens.background_outline);
        tokens.visible_layer_outline = color("otherLayersStroke", tokens.visible_layer_outline);
        tokens.kerning_negative = color("kerningNegative", tokens.kerning_negative);
        tokens.kerning_positive = color("kerningPositive", tokens.kerning_positive);
        let percent = settings.get("appearance.guideOpacity").ok().and_then(|v| v.as_f64()).filter(|p| p.is_finite()).unwrap_or(100.0);
        #[allow(clippy::cast_possible_truncation)]
        let opacity = (percent.clamp(0.0, 100.0) / 100.0) as f32;
        tokens.guide_opacity = opacity;
        tokens.metric_line = tokens.metric_line.gamma_multiply(opacity);
        tokens.metric_name = tokens.metric_name.gamma_multiply(opacity);
        tokens.zone_fill = tokens.zone_fill.gamma_multiply(opacity);
        tokens.scale_handles(scale);
        tokens
    }

    /// Multiplies every node, handle and anchor size by `scale`.
    fn scale_handles(&mut self, scale: f32) {
        for size in [
            &mut self.node_size,
            &mut self.smooth_node_radius,
            &mut self.handle_radius,
            &mut self.start_arrow_size,
            &mut self.duplicate_node_radius,
            &mut self.metric_highlight_size,
            &mut self.extra_node_radius,
            &mut self.anchor_size,
        ] {
            *size *= scale;
        }
    }

    /// The values the settings do not cover, and the fallbacks for those they do, in the dark appearance when `dark`.
    fn standard(dark: bool) -> Tokens {
        let light = Tokens::standard_light();
        if !dark {
            return light;
        }
        Tokens {
            canvas: Color32::from_rgb(0x1e, 0x1e, 0x1e),
            outline: Color32::from_rgb(0xe6, 0xe6, 0xe6),
            source_outline: Color32::from_rgb(0x5a, 0x5a, 0x5a),
            metric_line: Color32::from_rgb(0x4c, 0x56, 0x64),
            metric_name: Color32::from_rgb(0x8c, 0x96, 0xa4),
            node_handle: Color32::from_rgb(0xa0, 0xa0, 0xa0),
            handle_line: Color32::from_rgb(0x70, 0x70, 0x70),
            component_manual: Color32::from_rgb(0xa0, 0xa0, 0xa0),
            // Editing the background lifts the dark canvas a little, where a darker tint would not show.
            background_tint: Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x10),
            ..light
        }
    }

    fn standard_light() -> Tokens {
        Tokens {
            canvas: Color32::from_rgb(0xf4, 0xf4, 0xf4),
            master_canvas: None,
            outline: Color32::from_rgb(0x20, 0x20, 0x20),
            source_outline: Color32::from_rgb(0xb4, 0xb4, 0xb4),
            metric_line: Color32::from_rgb(0xb0, 0xb8, 0xc4),
            metric_name: Color32::from_rgb(0x80, 0x88, 0x94),
            zone_fill: Color32::from_rgba_unmultiplied(0x9a, 0xb8, 0xdc, 0x38),
            node_corner: Color32::from_rgb(0x2d, 0x6c, 0xdf),
            node_smooth: Color32::from_rgb(0x2f, 0xa8, 0x4f),
            node_handle: Color32::from_rgb(0x88, 0x88, 0x88),
            handle_line: Color32::from_rgb(0xa8, 0xa8, 0xa8),
            open_end: Color32::from_rgb(0x1e, 0x7b, 0xe6),
            open_end_start: Color32::from_rgb(0x8c, 0xc4, 0xf5),
            duplicate_node: Color32::from_rgb(0xe0, 0x30, 0x30),
            metric_highlight: Color32::from_rgb(0xdc, 0xc4, 0x8c),
            extra_node: Color32::from_rgb(0x9a, 0x5c, 0xd0),
            component_aligned: Color32::from_rgb(0x3c, 0xa0, 0x50),
            component_manual: Color32::from_rgb(0x8c, 0x8c, 0x8c),
            component_shiftable: Color32::from_rgb(0x3c, 0x78, 0xd8),
            component_placeholder: Color32::from_rgb(0xd0, 0x40, 0x40),
            anchor: Color32::from_rgb(0xe8, 0x8a, 0x1a),
            attachment_band: Color32::from_rgba_unmultiplied(0x50, 0x96, 0xf0, 0x40),
            attachment_band_selected: Color32::from_rgba_unmultiplied(0x50, 0x96, 0xf0, 0xb0),
            background_outline: Color32::from_rgba_unmultiplied(0xe0, 0x40, 0x40, 0x90),
            visible_layer_outline: Color32::from_rgba_unmultiplied(0x2d, 0x8c, 0xeb, 0xb0),
            reference_outline: Color32::from_rgba_unmultiplied(0x30, 0xa0, 0x60, 0xa0),
            background_tint: Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0x18),
            kerning_negative: Color32::from_rgba_unmultiplied(0x8c, 0xc8, 0xf5, 0x90),
            kerning_positive: Color32::from_rgba_unmultiplied(0xf5, 0xdc, 0x46, 0xa0),
            guide_opacity: 1.0,
            outline_width: 1.5,
            source_outline_width: 1.0,
            metric_line_width: 1.0,
            handle_line_width: 1.0,
            node_size: 7.0,
            smooth_node_radius: 4.0,
            handle_radius: 2.5,
            start_arrow_size: 11.0,
            open_end_length: 16.0,
            duplicate_node_radius: 7.5,
            metric_highlight_size: 18.0,
            extra_node_radius: 2.5,
            component_outline_width: 2.0,
            component_selected_width: 3.0,
            component_fill_opacity: 0.14,
            mark_cloud_opacity: 0.16,
            mark_cloud_outline_opacity: 0.4,
            anchor_size: 9.0,
            attachment_band_width: 10.0,
            label_size: 10.0,
            label_gap: 4.0,
            background_outline_width: 1.0,
            visible_layer_outline_width: 0.75,
            reference_outline_width: 1.0,
            fit_margin: 0.08,
            compat: CompatTokens::standard(),
        }
    }
}

/// The colours and sizes of the Master Compatibility view.
#[derive(Clone, Debug, PartialEq)]
pub struct CompatTokens {
    /// Segments that match, turn more than the warning angle, or do not match.
    pub ok: Color32,
    pub warn: Color32,
    pub bad: Color32,
    /// The lines that join corresponding points across the layers.
    pub line: Color32,
    /// The tints of the shapes, by shape index in turn.
    pub tints: [Color32; 6],
    pub segment_width: f32,
    pub tint_width: f32,
    pub line_width: f32,
    pub label_size: f32,
    /// How far a label sits from its point, in screen points.
    pub label_gap: f32,
    /// Dash and gap lengths of component outlines and anchor lines.
    pub dash: f32,
    pub dash_gap: f32,
}

impl CompatTokens {
    fn standard() -> CompatTokens {
        CompatTokens {
            ok: Color32::from_rgb(0x2f, 0xa8, 0x4f),
            warn: Color32::from_rgb(0xe0, 0xb0, 0x20),
            bad: Color32::from_rgb(0xe0, 0x30, 0x30),
            line: Color32::from_rgba_unmultiplied(0x60, 0x60, 0x60, 0xa0),
            tints: [
                Color32::from_rgba_unmultiplied(0x2d, 0x6c, 0xdf, 0x70),
                Color32::from_rgba_unmultiplied(0xe8, 0x8a, 0x1a, 0x70),
                Color32::from_rgba_unmultiplied(0x9a, 0x5c, 0xd0, 0x70),
                Color32::from_rgba_unmultiplied(0x1a, 0xa0, 0xa0, 0x70),
                Color32::from_rgba_unmultiplied(0xd0, 0x40, 0x90, 0x70),
                Color32::from_rgba_unmultiplied(0x80, 0x80, 0x20, 0x70),
            ],
            segment_width: 2.0,
            tint_width: 4.0,
            line_width: 1.0,
            label_size: 10.0,
            label_gap: 4.0,
            dash: 4.0,
            dash_gap: 3.0,
        }
    }

    /// The tint of shape `index`, opaque when `solid`.
    pub fn tint(&self, index: usize, solid: bool) -> Color32 {
        let base = self.tints.get(index % self.tints.len()).copied().unwrap_or(self.line);
        if solid { Color32::from_rgb(base.r(), base.g(), base.b()) } else { base }
    }
}

/// The canvas colour `master` sets for the dark appearance when `dark`, else for the light one: its Master Background
/// Color Dark or Master Background Color, read as `R,G,B[,A]` from 0 to 255 or `#RRGGBB[AA]`. A value that cannot be
/// read sets nothing, and the dark canvas does not fall back to the light value.
pub fn master_background(master: &Master, dark: bool) -> Option<Color32> {
    let name = if dark { MASTER_BACKGROUND_DARK } else { MASTER_BACKGROUND };
    let c = master.params.get(name).and_then(export::master_color)?;
    Some(Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a))
}

/// A colour setting's value: `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(text: &str) -> Option<Color32> {
    let hex = text.strip_prefix('#')?;
    if !hex.is_ascii() || !(hex.len() == 6 || hex.len() == 8) {
        return None;
    }
    let byte = |i: usize| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    let alpha = if hex.len() == 8 { byte(6)? } else { 0xff };
    Some(Color32::from_rgba_unmultiplied(byte(0)?, byte(2)?, byte(4)?, alpha))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn stored_colours_and_handle_size_reach_the_tokens() {
        let standard = Tokens::default();
        let mut settings = UserSettings::new();
        settings.set("appearance.color.cornerNodes.light", json!("#ff000080")).unwrap();
        settings.set("appearance.handleSize", json!("large")).unwrap();
        let tokens = Tokens::from_settings(&settings, false);
        assert_eq!(tokens.node_corner, Color32::from_rgba_unmultiplied(0xff, 0, 0, 0x80));
        assert_eq!(tokens.node_size, standard.node_size * 1.5);
        assert_eq!(tokens.node_smooth, standard.node_smooth, "an unset colour keeps its default");
        assert_eq!(Tokens::from_settings(&settings, true).node_corner, standard.node_corner, "the dark value is separate");
    }

    #[test]
    fn kerning_colours_and_guide_opacity_reach_the_tokens() {
        let standard = Tokens::default();
        let mut settings = UserSettings::new();
        settings.set("appearance.color.kerningPositive.dark", json!("#00ff00")).unwrap();
        settings.set("appearance.guideOpacity", json!(50)).unwrap();
        let light = Tokens::from_settings(&settings, false);
        let dark = Tokens::from_settings(&settings, true);
        assert_eq!(dark.kerning_positive, Color32::from_rgb(0, 0xff, 0));
        assert_eq!(light.kerning_positive, standard.kerning_positive, "the light value is separate");
        assert_eq!(light.kerning_negative, standard.kerning_negative);
        assert_eq!(light.guide_opacity, 0.5);
        assert!(light.metric_line.a() < standard.metric_line.a() && light.zone_fill.a() < standard.zone_fill.a());
        let hints = crate::canvas::hinting::tokens::HintTokens::with_opacity(light.guide_opacity);
        assert!(hints.ps_vertical.a() < crate::canvas::hinting::tokens::HintTokens::standard().ps_vertical.a(), "hint bands fade too");
        assert_eq!(standard.guide_opacity, 1.0);
        assert_eq!(standard.metric_line, Tokens::standard(false).metric_line, "full opacity leaves the colours alone");
    }

    /// The two sets have their own canvas and outline defaults, and a stored corner-node colour reaches the tokens in
    /// the set it was stored for.
    #[test]
    fn each_appearance_takes_its_own_colour_set() {
        let standard = UserSettings::new();
        let light = Tokens::from_settings(&standard, false);
        let dark = Tokens::from_settings(&standard, true);
        assert_eq!(light.canvas, Color32::from_rgb(0xf4, 0xf4, 0xf4));
        assert_eq!(dark.canvas, Color32::from_rgb(0x1e, 0x1e, 0x1e));
        assert_eq!(dark.outline, Color32::from_rgb(0xe6, 0xe6, 0xe6));
        assert_eq!(dark.node_corner, light.node_corner, "the node colours share one default");
        let mut settings = UserSettings::new();
        settings.set("appearance.color.cornerNodes.dark", json!("#11223344")).unwrap();
        settings.set("appearance.color.canvas.light", json!("#ffffff")).unwrap();
        assert_eq!(Tokens::from_settings(&settings, true).node_corner, Color32::from_rgba_unmultiplied(0x11, 0x22, 0x33, 0x44));
        assert_eq!(Tokens::from_settings(&settings, false).node_corner, light.node_corner);
        assert_eq!(Tokens::from_settings(&settings, false).canvas, Color32::WHITE);
        assert_eq!(Tokens::from_settings(&settings, true).canvas, dark.canvas);
    }
}
