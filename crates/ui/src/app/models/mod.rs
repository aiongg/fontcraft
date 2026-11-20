//! View models: what each panel shows, copied out of the engine into `Tracked` values.
//!
//! One file per area. Each file owns its model types and a `project` function; [`Models::project`] calls every
//! one of them with a one-line arm, and only when the [`Stamp`] moved since the last call.

pub mod canvas;
pub mod color;
pub mod compare;
pub mod compatibility;
pub mod components;
pub mod console;
pub mod font_audit;
pub mod font_exports;
pub mod font_features;
pub mod font_info;
pub mod font_masters;
pub mod font_view;
pub mod glyph;
pub mod glyph_info;
pub mod glyph_props;
pub mod glyph_sections;
pub mod hint_canvas;
pub mod hinting;
pub mod hoi;
pub mod kerning_review;
pub mod kerning_tools;
pub mod kerning_window;
pub mod layers;
pub mod palette;
pub mod preview;
pub mod project;
pub mod shape_order;
pub mod sidebar;
pub mod space_center;
pub mod table_viewer;
pub mod text;
pub mod text_preview;

use craft::ui::contract::Tracked;
use fontcraft_engine::Session;

use super::ViewState;

/// Every view model the panels read.
#[derive(Default)]
pub struct Models {
    /// The stamp the models were last projected at.
    seen: Option<Stamp>,
    /// The shown glyph's width and sidebearings (`panels::glyph_metrics`).
    pub glyph_metrics: Tracked<glyph::GlyphMetrics>,
    /// The edited glyph as the canvas draws it (`canvas`).
    pub canvas: Tracked<canvas::CanvasModel>,
    /// The canvas's colours and sizes, from the appearance settings.
    pub canvas_tokens: crate::canvas::tokens::Tokens,
    /// Font View's glyphs, their order, the selection and the cell outlines (`panels::font_view`).
    pub font_view: Tracked<font_view::FontViewModel>,
    font_view_outlines: font_view::OutlineCache,
    /// The Font Info panel (`panels::font_info`).
    pub font_info: Tracked<font_info::FontInfoModel>,
    /// What is connected to the shown glyph (`panels::related_glyphs`).
    pub related_glyphs: Tracked<glyph_sections::RelatedModel>,
    /// The shown glyph's stored selections (`panels::selections`).
    pub selections: Tracked<glyph_sections::SelectionsModel>,
    /// The shown master's metrics and stems (`panels::measurements`).
    pub measurements: Tracked<glyph_sections::MeasurementsModel>,
    /// The reference fonts (`panels::reference_font`).
    pub reference_font: Tracked<glyph_sections::ReferenceFontModel>,
    /// The Pairs & Phrases work list (`panels::pairs_phrases`).
    pub pairs_phrases: Tracked<kerning_review::PairsModel>,
    /// Glyph Info's search results (`panels::glyph_info`).
    pub glyph_info: Tracked<glyph_info::GlyphInfoModel>,
    glyph_info_search: glyph_info::SearchCache,
    /// The Hinting Preview's rows (`panels::hinting_waterfall`).
    pub hinting_waterfall: Tracked<hinting::Waterfall>,
    /// What the hint layers draw (`canvas::hinting`).
    pub hint_canvas: Tracked<hint_canvas::HintCanvas>,
    hint_cache: hint_canvas::HintCache,
    /// The files colour glyphs draw from (`canvas::color::files`), shared by the canvas, its text and Font View.
    color_files: crate::canvas::color::ColorFiles,
    /// The hint layers' colours and sizes.
    pub hint_tokens: crate::canvas::hinting::tokens::HintTokens,
    /// The shown glyph's layers (`panels::layers`).
    pub layers: Tracked<layers::LayersModel>,
    /// The masters and the current one (`panels::master_switcher`).
    pub masters: Tracked<layers::MastersModel>,
    /// The Command Console's pane and input line (`panels::command_console`); projected outside the stamp.
    pub console: Tracked<console::ConsoleModel>,
    /// The Kerning window (`panels::kerning_window`).
    pub kerning_window: Tracked<kerning_window::KerningWindowModel>,
    /// The Kerning Pairs Preview (`panels::kerning_tools::pairs_preview`).
    pub kerning_pairs_preview: Tracked<kerning_tools::PairsPreviewModel>,
    /// The Kerning Groups Editor (`panels::kerning_tools::groups_editor`).
    pub kerning_groups_editor: Tracked<kerning_tools::GroupsEditorModel>,
    /// The sidebar's entries (`panels::font_view::sidebar`).
    pub sidebar: Tracked<sidebar::SidebarModel>,
    /// The glyph inspector's fields (`panels::font_view::inspector`).
    pub inspector: Tracked<glyph_props::GlyphPropsModel>,
    /// The Selection Info panel's fields (`panels::selection_info`).
    pub selection_info: Tracked<glyph_props::GlyphPropsModel>,
    /// The Master Compatibility view (`canvas::compatibility`).
    pub compatibility: Tracked<compatibility::CompatibilityModel>,
    /// The Info box and the Palette sections (`panels::info_box`, `panels::transformations`, …); projected outside the
    /// stamp, since they read the canvas's selection.
    pub palette: palette::PaletteModels,
    pub text: Tracked<text::TextModel>,
    /// The Space Center (`panels::space_center`).
    pub space_center: Tracked<space_center::SpaceCenterModel>,
    /// The Table Viewer (`panels::table_viewer`).
    pub table_viewer: Tracked<table_viewer::TableViewerModel>,
    /// Compare Fonts (`panels::compare_fonts`); projected outside the stamp, since it reads two documents.
    pub compare_fonts: Tracked<compare::CompareFontsModel>,
    /// Compare Family (`panels::compare_family`).
    pub compare_family: Tracked<compare::CompareFamilyModel>,
    /// The Text Preview (`panels::text_preview`); projected outside the stamp, since its font builds land between
    /// frames.
    pub text_preview: text_preview::TextPreviewModels,
    /// The Attributes panel (`panels::attributes`); projected outside the stamp, since it reads the canvas's selection.
    pub color: color::AttributesModels,
    pub font_audit: Tracked<font_audit::FontAuditModel>,
    /// The Features menu.
    pub features: Tracked<text::FeaturesModel>,
    pub project: project::ProjectModels,
    /// The Preview Area's and the Preview Panel's lines (`panels::preview`).
    pub preview_area: Tracked<preview::PreviewModel>,
    pub preview_panel: Tracked<preview::PreviewModel>,
    /// The Shape Order panel (`panels::shape_order`).
    pub shape_order: Tracked<shape_order::ShapeOrderModel>,
}

/// What a projection reads: the session, for its data and its queries, and the view state.
pub struct ProjectCx<'a> {
    /// The session. Projections read it and run queries through `Session::query`, which refuses edits.
    pub session: &'a mut Session,
    pub view: &'a ViewState,
}

/// When the models need projecting again: the open documents, the active one and its revision, the user settings'
/// revision and the view state's revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    documents: usize,
    active: Option<(u64, u64)>,
    settings: u64,
    view: u64,
    /// The active document's last feature compile.
    compile: Option<(u64, u64, bool)>,
}

impl Stamp {
    /// The stamp of `session` and the view state at `view_revision`.
    pub fn of(session: &Session, view_revision: u64) -> Stamp {
        Stamp {
            documents: session.documents().len(),
            active: session.active().map(|d| (d.id, d.revision)),
            settings: session.settings().revision(),
            view: view_revision,
            compile: crate::canvas::text::features::compile_stamp(session),
        }
    }
}

impl Models {
    /// Projects every model when `stamp` differs from the last projection; returns whether it did.
    pub fn project(&mut self, stamp: Stamp, cx: &mut ProjectCx<'_>) -> bool {
        if self.seen == Some(stamp) {
            return false;
        }
        self.seen = Some(stamp);
        glyph::project(&mut self.glyph_metrics, cx);
        canvas::project(&mut self.canvas, &mut self.color_files, cx);
        self.canvas_tokens = crate::canvas::tokens::Tokens::from_settings(cx.session.settings(), false);
        font_view::project(&mut self.font_view, &mut self.font_view_outlines, &mut self.color_files, cx);
        font_info::project(&mut self.font_info, cx);
        glyph_sections::project_related(&mut self.related_glyphs, cx);
        glyph_sections::project_selections(&mut self.selections, cx);
        glyph_sections::project_measurements(&mut self.measurements, cx);
        glyph_sections::project_reference_font(&mut self.reference_font, cx);
        kerning_review::project(&mut self.pairs_phrases, cx);
        glyph_info::project(&mut self.glyph_info, &mut self.glyph_info_search, cx);
        hinting::project(&mut self.hinting_waterfall, cx);
        hint_canvas::project(&mut self.hint_canvas, &mut self.hint_cache, cx);
        layers::project(&mut self.layers, cx);
        layers::project_masters(&mut self.masters, cx);
        kerning_window::project(&mut self.kerning_window, cx);
        kerning_tools::project_preview(&mut self.kerning_pairs_preview, cx);
        kerning_tools::project_groups(&mut self.kerning_groups_editor, cx);
        sidebar::project(&mut self.sidebar, cx);
        glyph_props::project_inspector(&mut self.inspector, cx);
        glyph_props::project_selection_info(&mut self.selection_info, cx);
        text::project(&mut self.text, &mut self.color_files, cx);
        space_center::project(&mut self.space_center, cx);
        table_viewer::project(&mut self.table_viewer, cx);
        compare::project_family(&mut self.compare_family, cx);
        font_audit::project(&mut self.font_audit, cx);
        compatibility::project(&mut self.compatibility, cx);
        text::project_features(&mut self.features, &self.text, cx);
        preview::project_both(&mut self.preview_area, &mut self.preview_panel, cx);
        shape_order::project(&mut self.shape_order, cx);
        true
    }
}

/// Replaces `model`'s value with `value` only when they differ, so an unchanged model keeps its revision.
pub fn set<T: craft::ui::contract::PlainData + PartialEq>(model: &mut Tracked<T>, value: T) {
    if **model != value {
        model.edit(|m| *m = value);
    }
}
