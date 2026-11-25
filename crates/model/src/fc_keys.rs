//! The `com.fontcraft.*` user-data keys: where FontCraft keeps the data of features that neither
//! `.glyphs` nor UFO has a field for, on the object the data belongs to. Both formats keep user
//! data they do not know, so these survive a round trip. Plan: `docs/plans/M13.md` § 3 (secondary
//! features' stored data). This list is fixed; a package that needs another key adds it here in
//! its own change.
//!
//! Values are plist values (dictionaries, arrays, strings, numbers); each constant's doc gives
//! the shape and the package that reads and writes it.

/// The object a key is stored on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    Font,
    Glyph,
    Layer,
    Path,
    Node,
    Anchor,
    Component,
    Guide,
    /// The user settings store, not a document.
    Settings,
}

/// An anchor's position expressions: `{"x"?: "<expr>", "y"?: "<expr>"}` in `fontcraft_geom::expr`
/// syntax (M13.16, `anchor.positionExpressions`).
pub const ANCHOR_EXPRESSIONS: &str = "com.fontcraft.anchorExpressions";
/// A glyph's or layer's automatic layer recipe: a recipe string with directives (M13.16,
/// `glyph.setAutoLayerRecipe`).
pub const AUTO_LAYER_RECIPE: &str = "com.fontcraft.autoLayerRecipe";
/// A glyph's stored selections: an array of `{"name", "nodes": [[shape, node]…], "anchors": […],
/// "components": […]}` (M13.32, `edit.storeSelection`).
pub const STORED_SELECTIONS: &str = "com.fontcraft.storedSelections";
/// A path's contour name: a string (M13.27, `path.nameContour`).
pub const CONTOUR_NAME: &str = "com.fontcraft.contourName";
/// A node's servant link: `{"axis": "x"|"y"|"both", "masters": [node indices]}` (M13.33,
/// `path.setServantNode`).
pub const SERVANT: &str = "com.fontcraft.servant";
/// A node's stroke width for the Thickness tool: `[left, right]` in font units (M13.38).
pub const NODE_WIDTH: &str = "com.fontcraft.nodeWidth";
/// A guide's thickness, in font units (M13.36, `guide.setThickness`).
pub const GUIDE_THICKNESS: &str = "com.fontcraft.guideThickness";
/// A guide's markers: an array of positions along the guide, in font units (M13.36,
/// `guide.addMarkers`).
pub const GUIDE_MARKERS: &str = "com.fontcraft.guideMarkers";
/// A guide's appearance: `{"style"?: "solid"|"dashed"}`. Not written: a guide's colour is its own
/// `color` field (M13.36, `guide.setAppearance`).
pub const GUIDE_APPEARANCE: &str = "com.fontcraft.guideAppearance";
/// How strongly a guide pulls dragged points: the snapping distance in screen points (M13.36,
/// `guide.setAppearance`; `crate::guide_data`).
pub const GUIDE_MAGNETIC: &str = "com.fontcraft.guideMagnetic";
/// A power guide's links: an array of `[shape, node]` it moves, `[glyph, shape, node]` for a
/// global guide (M13.36, `guide.makePowerGuide`; `crate::guide_data`).
pub const POWER_GUIDE_LINKS: &str = "com.fontcraft.powerGuideLinks";
/// A glyph's status: the integer code of one of the font's status definitions (M13.39,
/// `glyph.setStatus`).
pub const GLYPH_STATUS: &str = "com.fontcraft.status";
/// The font's status definitions: an array of `{"code", "color": [r, g, b, a], "isDefault"?: 1,
/// "name"}` (M13.39, `font.setStatusDefinitions`).
pub const STATUS_DEFINITIONS: &str = "com.fontcraft.statusDefinitions";
/// The user's export profiles: an array of `{"name", "params": {…}}` (M13.43,
/// `file.saveExportProfile`).
pub const EXPORT_PROFILES: &str = "com.fontcraft.exportProfiles";
/// A glyph or master added from a linked source (Import Font, Import Master): the parameter's path as
/// written (M7.12). Never written to a file: saving leaves linked glyphs and masters out.
pub const LINKED_SOURCE: &str = "com.fontcraft.linkedSource";
/// The font's audit settings: `{"defaultProfile"?, "config"?: {check: {threshold: number}},
/// "values"?: {name: value}, "ignored"?: [{"check", "glyph"?, "master"?, "location"?, "reason"?,
/// "date"?}], "profiles"?: [profile]}` (M12.59, `fontcraft_audit::profile`).
pub const AUDIT: &str = "com.fontcraft.audit";
/// The font's cross-axis mappings: `{"description"?, "mappings": [{"description"?, "input": {tag:
/// number}, "output": {tag: number}}]}` in design coordinates (M13.48, `font.setCrossAxisMappings`).
pub const CROSS_AXIS_MAPPINGS: &str = "com.fontcraft.crossAxisMappings";
/// A component's own place in the design space: `{tag: number}` in design coordinates (M6.24,
/// `component.setInstanceLocation`; `crate::pinned`).
pub const COMPONENT_LOCATION: &str = "com.fontcraft.componentLocation";

/// A source layer left out of interpolation: the integer 1 (M9.37, `glyph.setSourceActive`,
/// `fontcraft_interp::sources`). A layer without the key takes part.
pub const SOURCE_INACTIVE: &str = "com.fontcraft.sourceInactive";

/// Every key with the object it is stored on.
pub const ALL: &[(&str, Owner)] = &[
    (ANCHOR_EXPRESSIONS, Owner::Anchor),
    (AUTO_LAYER_RECIPE, Owner::Layer),
    (STORED_SELECTIONS, Owner::Glyph),
    (CONTOUR_NAME, Owner::Path),
    (SERVANT, Owner::Node),
    (NODE_WIDTH, Owner::Node),
    (GUIDE_THICKNESS, Owner::Guide),
    (GUIDE_MARKERS, Owner::Guide),
    (GUIDE_APPEARANCE, Owner::Guide),
    (GUIDE_MAGNETIC, Owner::Guide),
    (POWER_GUIDE_LINKS, Owner::Guide),
    (GLYPH_STATUS, Owner::Glyph),
    (STATUS_DEFINITIONS, Owner::Font),
    (EXPORT_PROFILES, Owner::Settings),
    (LINKED_SOURCE, Owner::Glyph),
    (AUDIT, Owner::Font),
    (CROSS_AXIS_MAPPINGS, Owner::Font),
    (COMPONENT_LOCATION, Owner::Component),
    (SOURCE_INACTIVE, Owner::Layer),
];

/// Whether `key` is one of FontCraft's own keys.
pub fn is_fontcraft_key(key: &str) -> bool {
    key.starts_with("com.fontcraft.")
}
