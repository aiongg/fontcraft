//! The dock: every panel's id, title and place, and what the dock shows in each.
//!
//! The app layer owns the [`Workspace`] (view state); the dock gets it as a given binding and reports the user's
//! moves as actions, which [`show`] applies. All dock code stays in this module.

use craft::behavior::Axis;
use craft::behavior::dock::{Column, DockEdge, Group, Layout, Node, PanelState, Permissions, SplitSize, Workspace};
use craft::ui::contract::{Bind, BodyCx};
use craft::ui::docking::{DockConfig, DockPanels, GroupAt, PanelSpec};
use craft::ui::patterns::dock::DockArea;
use craft::ui::runtime::env::{Env, Label};
use egui::Ui;

use crate::app::models::Models;
use crate::app::{FontCraftApp, Msg};
use crate::{canvas, panels};

/// Every panel of the window. Add a variant, its id in [`PanelId::id`], its title in [`PanelWords`] and its arm in
/// [`Panels::body`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelId {
    /// The edit view's canvas: the document area.
    Document,
    FontView,
    Sidebar,
    /// The glyph part of the Info box: name, width and sidebearings.
    GlyphMetrics,
    InfoBox,
    Kerning,
    Dimensions,
    FitCurve,
    Layers,
    Transformations,
    RelatedGlyphs,
    ReferenceFont,
    Selections,
    Measurements,
    PairsAndPhrases,
    Stroke,
    SmartComponent,
    Attributes,
    AnchorContext,
    Modules,
    SelectionInfo,
    CommandConsole,
    HintingWaterfall,
    TextPreview,
    Preview,
    SpaceCenter,
    KerningWindow,
    KerningReview,
    GlyphInfo,
    FontAudit,
    TableViewer,
    CompareFonts,
    CompareFamily,
    Project,
    FontInfo,
}

impl PanelId {
    /// Every panel, in the Window menu's order.
    pub const ALL: &[PanelId] = &[
        PanelId::Document,
        PanelId::FontView,
        PanelId::Sidebar,
        PanelId::GlyphMetrics,
        PanelId::InfoBox,
        PanelId::Kerning,
        PanelId::Dimensions,
        PanelId::FitCurve,
        PanelId::Layers,
        PanelId::Transformations,
        PanelId::RelatedGlyphs,
        PanelId::ReferenceFont,
        PanelId::Selections,
        PanelId::Measurements,
        PanelId::PairsAndPhrases,
        PanelId::Stroke,
        PanelId::SmartComponent,
        PanelId::Attributes,
        PanelId::AnchorContext,
        PanelId::Modules,
        PanelId::SelectionInfo,
        PanelId::CommandConsole,
        PanelId::HintingWaterfall,
        PanelId::TextPreview,
        PanelId::Preview,
        PanelId::SpaceCenter,
        PanelId::KerningWindow,
        PanelId::KerningReview,
        PanelId::GlyphInfo,
        PanelId::FontAudit,
        PanelId::TableViewer,
        PanelId::CompareFonts,
        PanelId::CompareFamily,
        PanelId::Project,
        PanelId::FontInfo,
    ];

    /// The stable id commands and saved layouts use.
    pub fn id(self) -> &'static str {
        match self {
            PanelId::Document => "document",
            PanelId::FontView => "fontView",
            PanelId::Sidebar => "sidebar",
            PanelId::GlyphMetrics => "glyphMetrics",
            PanelId::InfoBox => "infoBox",
            PanelId::Kerning => "kerning",
            PanelId::Dimensions => "dimensions",
            PanelId::FitCurve => "fitCurve",
            PanelId::Layers => "layers",
            PanelId::Transformations => "transformations",
            PanelId::RelatedGlyphs => "relatedGlyphs",
            PanelId::ReferenceFont => "referenceFont",
            PanelId::Selections => "selections",
            PanelId::Measurements => "measurements",
            PanelId::PairsAndPhrases => "pairsAndPhrases",
            PanelId::Stroke => "stroke",
            PanelId::SmartComponent => "smartComponent",
            PanelId::Attributes => "attributes",
            PanelId::AnchorContext => "anchorContext",
            PanelId::Modules => "modules",
            PanelId::SelectionInfo => "selectionInfo",
            PanelId::CommandConsole => "commandConsole",
            PanelId::HintingWaterfall => "hintingWaterfall",
            PanelId::TextPreview => "textPreview",
            PanelId::Preview => "preview",
            PanelId::SpaceCenter => "spaceCenter",
            PanelId::KerningWindow => "kerningWindow",
            PanelId::KerningReview => "kerningReview",
            PanelId::GlyphInfo => "glyphInfo",
            PanelId::FontAudit => "fontAudit",
            PanelId::TableViewer => "tableViewer",
            PanelId::CompareFonts => "compareFonts",
            PanelId::CompareFamily => "compareFamily",
            PanelId::Project => "project",
            PanelId::FontInfo => "fontInfo",
        }
    }

    pub fn from_id(id: &str) -> Option<PanelId> {
        PanelId::ALL.iter().copied().find(|p| p.id() == id)
    }

    /// The panel's title in the user's language.
    pub fn title(self, words: &PanelWords) -> Label {
        let w = words;
        match self {
            PanelId::Document => w.document.clone(),
            PanelId::FontView => w.font_view.clone(),
            PanelId::Sidebar => w.sidebar.clone(),
            PanelId::GlyphMetrics => w.glyph_metrics.clone(),
            PanelId::InfoBox => w.info_box.clone(),
            PanelId::Kerning => w.kerning.clone(),
            PanelId::Dimensions => w.dimensions.clone(),
            PanelId::FitCurve => w.fit_curve.clone(),
            PanelId::Layers => w.layers.clone(),
            PanelId::Transformations => w.transformations.clone(),
            PanelId::RelatedGlyphs => w.related_glyphs.clone(),
            PanelId::ReferenceFont => w.reference_font.clone(),
            PanelId::Selections => w.selections.clone(),
            PanelId::Measurements => w.measurements.clone(),
            PanelId::PairsAndPhrases => w.pairs_and_phrases.clone(),
            PanelId::Stroke => w.stroke.clone(),
            PanelId::SmartComponent => w.smart_component.clone(),
            PanelId::Attributes => w.attributes.clone(),
            PanelId::AnchorContext => w.anchor_context.clone(),
            PanelId::Modules => w.modules.clone(),
            PanelId::SelectionInfo => w.selection_info.clone(),
            PanelId::CommandConsole => w.command_console.clone(),
            PanelId::HintingWaterfall => w.hinting_waterfall.clone(),
            PanelId::TextPreview => w.text_preview.clone(),
            PanelId::Preview => w.preview.clone(),
            PanelId::SpaceCenter => w.space_center.clone(),
            PanelId::KerningWindow => w.kerning_window.clone(),
            PanelId::KerningReview => w.kerning_review.clone(),
            PanelId::GlyphInfo => w.glyph_info.clone(),
            PanelId::FontAudit => w.font_audit.clone(),
            PanelId::TableViewer => w.table_viewer.clone(),
            PanelId::CompareFonts => w.compare_fonts.clone(),
            PanelId::CompareFamily => w.compare_family.clone(),
            PanelId::Project => w.project.clone(),
            PanelId::FontInfo => w.font_info.clone(),
        }
    }

    /// What the user may do with the panel: the document area stays where it is.
    pub fn permissions(self) -> Permissions {
        match self {
            PanelId::Document => Permissions::PROTECTED,
            _ => Permissions::ALL,
        }
    }
}

craft::ui::words! {
    /// The panels' titles.
    pub struct PanelWords {
        /// The dock's own name.
        dock: "fontcraftPanel.dock" = "Panels";
        document: "fontcraftPanel.document" = "Edit View";
        font_view: "fontcraftPanel.fontView" = "Font View";
        sidebar: "fontcraftPanel.sidebar" = "Sidebar";
        glyph_metrics: "fontcraftPanel.glyphMetrics" = "Metrics";
        info_box: "fontcraftPanel.infoBox" = "Info";
        kerning: "fontcraftPanel.kerning" = "Kerning";
        dimensions: "fontcraftPanel.dimensions" = "Dimensions";
        fit_curve: "fontcraftPanel.fitCurve" = "Fit Curve";
        layers: "fontcraftPanel.layers" = "Layers";
        transformations: "fontcraftPanel.transformations" = "Transformations";
        related_glyphs: "fontcraftPanel.relatedGlyphs" = "Related Glyphs & Characters";
        reference_font: "fontcraftPanel.referenceFont" = "Reference Font";
        selections: "fontcraftPanel.selections" = "Selections";
        measurements: "fontcraftPanel.measurements" = "Measurements";
        pairs_and_phrases: "fontcraftPanel.pairsAndPhrases" = "Pairs & Phrases";
        stroke: "fontcraftPanel.stroke" = "Stroke";
        smart_component: "fontcraftPanel.smartComponent" = "Smart Component";
        attributes: "fontcraftPanel.attributes" = "Attributes";
        anchor_context: "fontcraftPanel.anchorContext" = "Anchor Context";
        modules: "fontcraftPanel.modules" = "Modules";
        selection_info: "fontcraftPanel.selectionInfo" = "Selection Info";
        command_console: "fontcraftPanel.commandConsole" = "Command Console";
        hinting_waterfall: "fontcraftPanel.hintingWaterfall" = "Hinting Preview";
        text_preview: "fontcraftPanel.textPreview" = "Text Preview";
        preview: "fontcraftPanel.preview" = "Preview";
        space_center: "fontcraftPanel.spaceCenter" = "Space Center";
        kerning_window: "fontcraftPanel.kerningWindow" = "Kerning Window";
        kerning_review: "fontcraftPanel.kerningReview" = "Kerning Review";
        glyph_info: "fontcraftPanel.glyphInfo" = "Glyph Info";
        font_audit: "fontcraftPanel.fontAudit" = "Font Audit";
        table_viewer: "fontcraftPanel.tableViewer" = "Table Viewer";
        compare_fonts: "fontcraftPanel.compareFonts" = "Compare Fonts";
        compare_family: "fontcraftPanel.compareFamily" = "Compare Family";
        project: "fontcraftPanel.project" = "Project";
        font_info: "fontcraftPanel.fontInfo" = "Font Info";
    }
}

/// The layout a new window starts with: the sidebar at the start, the edit view in the middle, the glyph panels at
/// the end. Every other panel starts hidden.
pub fn default_workspace() -> Workspace<PanelId> {
    let start = Column::new(vec![Group::new(vec![PanelId::Sidebar], 0)]);
    let end = Column::new(vec![
        Group::new(vec![PanelId::GlyphMetrics, PanelId::InfoBox, PanelId::Kerning], 0),
        Group::new(vec![PanelId::Layers], 0),
        Group::new(vec![PanelId::Dimensions, PanelId::Transformations, PanelId::FitCurve], 0),
    ]);
    let centre = Node::split(Axis::X, SplitSize::Ratio(0.8), Node::tabs(vec![PanelId::Document], 0), Node::dock(DockEdge::End, vec![end]));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.15), Node::dock(DockEdge::Start, vec![start]), centre);
    Workspace::new(Layout::new(Some(root), Vec::new()))
}

/// Shows `panel` when it is hidden, hides it when it shows.
pub fn toggle(workspace: &mut Workspace<PanelId>, panel: PanelId) {
    let shown = workspace.layout.panel_state(&panel) != PanelState::Hidden;
    // A refused move (a protected panel) leaves the layout as it was.
    let _ = workspace.set_visible(panel, !shown, None, false, |p| p.permissions());
}

/// What the dock reads for each panel.
struct Panels<'a> {
    models: &'a Models,
    canvas: canvas::CanvasProps<'a>,
    sent: &'a mut Vec<Msg>,
    words: std::sync::Arc<PanelWords>,
}

impl DockPanels<PanelId> for Panels<'_> {
    fn spec(&self, panel: &PanelId) -> PanelSpec {
        let spec = PanelSpec::new(panel.title(&self.words));
        if *panel == PanelId::Document { spec.canvas() } else { spec }
    }

    fn permissions(&self, panel: &PanelId) -> Permissions {
        panel.permissions()
    }

    fn body(&mut self, panel: &PanelId, ui: &mut Ui, _: &mut BodyCx<'_>) {
        match panel {
            PanelId::GlyphMetrics => {
                panels::glyph_metrics::show(ui, panels::glyph_metrics::Props { model: &self.models.glyph_metrics, sent: self.sent });
            }
            PanelId::FontView => panels::font_view::show(ui, panels::font_view::Props { model: &self.models.font_view, sent: self.sent }),
            PanelId::FontInfo => panels::font_info::show(ui, panels::font_info::Props { model: &self.models.font_info, sent: self.sent }),
            PanelId::RelatedGlyphs => {
                panels::related_glyphs::show(ui, panels::related_glyphs::Props { model: &self.models.related_glyphs, sent: self.sent });
            }
            PanelId::ReferenceFont => {
                panels::reference_font::show(ui, panels::reference_font::Props { model: &self.models.reference_font, sent: self.sent });
            }
            PanelId::Selections => panels::selections::show(ui, panels::selections::Props { model: &self.models.selections, sent: self.sent }),
            PanelId::Measurements => {
                panels::measurements::show(ui, panels::measurements::Props { model: &self.models.measurements, sent: self.sent });
            }
            PanelId::PairsAndPhrases => {
                panels::pairs_phrases::show(ui, panels::pairs_phrases::Props { model: &self.models.pairs_phrases, sent: self.sent });
            }
            PanelId::GlyphInfo => panels::glyph_info::show(ui, panels::glyph_info::Props { model: &self.models.glyph_info, sent: self.sent }),
            PanelId::HintingWaterfall => {
                panels::hinting_waterfall::show(ui, panels::hinting_waterfall::Props { model: &self.models.hinting_waterfall, sent: self.sent });
            }
            PanelId::Layers => panels::layers::show(ui, panels::layers::Props { model: &self.models.layers, sent: self.sent }),
            PanelId::CommandConsole => {
                panels::command_console::show(ui, panels::command_console::Props { model: &self.models.console, sent: self.sent });
            }
            PanelId::KerningWindow => {
                panels::kerning_window::show(ui, panels::kerning_window::Props { model: &self.models.kerning_window, sent: self.sent });
            }
            PanelId::Sidebar => panels::font_view::sidebar::show(
                ui,
                panels::font_view::sidebar::Props { model: &self.models.sidebar, inspector: &self.models.inspector, sent: self.sent },
            ),
            PanelId::SelectionInfo => {
                panels::selection_info::show(ui, panels::selection_info::Props { model: &self.models.selection_info, sent: self.sent });
            }
            PanelId::InfoBox => panels::info_box::show(ui, panels::info_box::Props { model: &self.models.palette.info_box, sent: self.sent }),
            PanelId::Transformations => {
                panels::transformations::show(ui, panels::transformations::Props { model: &self.models.palette.transform, sent: self.sent });
            }
            PanelId::FitCurve => panels::fit_curve::show(ui, panels::fit_curve::Props { model: &self.models.palette.fit_curve, sent: self.sent }),
            PanelId::AnchorContext => {
                panels::anchor_context::show(ui, panels::anchor_context::Props { model: &self.models.palette.anchor_context, sent: self.sent });
            }
            PanelId::Stroke => panels::stroke::show(ui, panels::stroke::Props { model: &self.models.palette.stroke, sent: self.sent }),
            PanelId::TextPreview => {
                panels::text_preview::show(ui, panels::text_preview::Props { model: &self.models.text_preview.view, sent: self.sent });
            }
            // Drawn by `group_view`.
            PanelId::Document => {}
            other => panels::stub::show(ui, panels::stub::Props { title: other.title(&self.words) }),
        }
    }

    fn group_view(&mut self, panels: &[PanelId], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&PanelId::Document) {
            return None;
        }
        match at {
            GroupAt::Show(rect) => {
                canvas::show(ui, &mut self.canvas, self.sent);
                Some(rect.height())
            }
            // The protected document group never sits in a flyout, which is what measures.
            GroupAt::Measure(_) => None,
        }
    }
}

/// Shows the dock and applies what the user did to it.
pub fn show(ui: &mut Ui, app: &mut FontCraftApp) {
    let words = Env::of(ui).words::<PanelWords>();
    let FontCraftApp { view, models, sent, status, active_tool, tools, .. } = app;
    let given = view.workspace.clone();
    let label = words.dock.clone();
    let mut panels = Panels {
        models,
        canvas: canvas::CanvasProps {
            view: &mut view.canvas,
            model: &models.canvas,
            tokens: &models.canvas_tokens,
            display: view.display,
            edit_background: view.edit_background,
            tools: &tools.view,
            tool: active_tool.as_mut(),
        },
        sent,
        words,
    };
    let out = DockArea::new(label, Bind::given(given), &mut panels, DockConfig::default()).show(ui);
    let actions: Vec<_> = out.actions.into_iter().chain(out.requested).collect();
    if !actions.is_empty()
        && let Err(error) = view.workspace.apply_all(actions, false, |p| p.permissions())
    {
        *status = format!("{error:?}");
    }
}
