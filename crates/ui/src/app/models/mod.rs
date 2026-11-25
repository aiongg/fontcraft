//! View models: what each panel shows, copied out of the engine into `Tracked` values.
//!
//! One file per area. Each file owns its model types and a `project` function; [`Models::project`] calls every
//! one of them with a one-line arm, and only when the [`Stamp`] moved since the last call.

pub mod bitmap_editor;
pub mod canvas;
pub mod cell_marks;
pub mod color;
pub mod compare;
pub mod compare_list;
pub mod compatibility;
pub mod components;
pub mod console;
pub mod designspace;
pub mod dimensions;
pub mod exported_names;
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
pub mod glyph_sources;
pub mod guide_box;
pub mod hint_canvas;
pub mod hinting;
pub mod hoi;
pub mod image_box;
pub mod incompatibility;
pub mod info_box_kerning;
pub mod instance_preview;
pub mod kerning_review;
pub mod kerning_tools;
pub mod kerning_window;
pub mod layers;
pub mod palette;
pub mod palette_modules;
pub mod preview;
pub mod project;
pub mod shape_order;
pub mod sidebar;
pub mod space_center;
pub mod table_viewer;
pub mod tabs;
pub mod text;
pub mod text_preview;

use craft::ui::contract::Tracked;
use fontcraft_engine::Session;

use super::ViewState;
use crate::shell::axis_sweep::{AxisSweep, STEPS};

/// Every view model the panels read.
#[derive(Default)]
pub struct Models {
    /// The stamp the models were last projected at.
    seen: Option<Stamp>,
    /// The shown glyph's width and sidebearings (`panels::glyph_metrics`).
    pub glyph_metrics: Tracked<glyph::GlyphMetrics>,
    /// The edited glyph as the canvas draws it (`canvas`).
    pub canvas: Tracked<canvas::CanvasModel>,
    /// The canvas's colours and sizes, from the appearance settings, in the colour set of the appearance the canvas
    /// is drawn in.
    pub canvas_tokens: crate::canvas::tokens::Tokens,
    /// The appearance of the window and of the two document views (`app::appearance`).
    pub appearance: crate::app::appearance::Appearance,
    /// Font View's glyphs, their order, the selection and the cell outlines (`panels::font_view`).
    pub font_view: Tracked<font_view::FontViewModel>,
    /// The placed images Font View cells draw (`canvas::images::cells`).
    pub cell_images: Tracked<crate::canvas::images::cells::CellImages>,
    /// The document area's tab strip (`panels::tab_strip`).
    pub tabs: Tracked<tabs::TabsModel>,
    font_view_outlines: font_view::OutlineCache,
    /// The Font Info panel (`panels::font_info`).
    pub font_info: Tracked<font_info::FontInfoModel>,
    /// What is connected to the shown glyph (`panels::related_glyphs`).
    pub related_glyphs: Tracked<glyph_sections::RelatedModel>,
    /// The shown glyph's stored selections (`panels::selections`).
    pub selections: Tracked<glyph_sections::SelectionsModel>,
    selections_seen: Option<glyph_sections::SelectionsStamp>,
    /// The shown master's metrics and stems (`panels::measurements`).
    pub measurements: Tracked<glyph_sections::MeasurementsModel>,
    /// The shown master's Dimensions notepad (`panels::dimensions`).
    pub dimensions: Tracked<dimensions::DimensionsModel>,
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
    /// The files colour glyphs draw from (`canvas::color::files`), shared by the canvas, its text, Font View and the
    /// Space Center.
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
    /// Where the compatibility checks behind the indicators stand.
    pub compat_scan: incompatibility::Scan,
    compat_probe: compatibility::ProbeSeen,
    /// The Info box and the Palette sections (`panels::info_box`, `panels::transformations`, …); projected outside the
    /// stamp, since they read the canvas's selection.
    pub palette: palette::PaletteModels,
    pub modules: palette_modules::ModulesModels,
    /// The Glyph Sources panel (`panels::glyph_sources`).
    pub glyph_sources: Tracked<glyph_sources::GlyphSourcesModel>,
    pub text: Tracked<text::TextModel>,
    /// The Space Center (`panels::space_center`).
    pub space_center: Tracked<space_center::SpaceCenterModel>,
    /// The placed images the Space Center draws behind its glyphs (`canvas::images::cells::space_center`).
    pub space_images: Tracked<crate::canvas::images::cells::CellImages>,
    /// The Table Viewer (`panels::table_viewer`).
    pub table_viewer: Tracked<table_viewer::TableViewerModel>,
    /// Compare Fonts (`panels::compare_fonts`); projected outside the stamp, since it reads two documents.
    pub compare_fonts: Tracked<compare::CompareFontsModel>,
    /// Compare Family (`panels::compare_family`).
    pub compare_family: Tracked<compare::CompareFamilyModel>,
    /// Compare Fonts in List (`panels::compare_in_list`).
    pub compare_list: Tracked<compare_list::CompareListModel>,
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
    /// The step of the axis sweep the previews were last projected at.
    swept: Option<u32>,
    /// The Shape Order panel (`panels::shape_order`).
    pub shape_order: Tracked<shape_order::ShapeOrderModel>,
    /// The Bitmap Glyph Editor (`panels::bitmap_editor`).
    pub bitmap_editor: Tracked<bitmap_editor::BitmapEditorModel>,
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
        self.appearance = crate::app::appearance::Appearance::of(cx.session.settings(), cx.view.system_dark);
        let dark = self.appearance.edit_view_dark();
        self.canvas_tokens = crate::canvas::tokens::Tokens::from_settings(cx.session.settings(), dark);
        self.canvas_tokens.master_canvas = cx
            .session
            .font()
            .ok()
            .and_then(|font| crate::shell::masters::current_master(font, &cx.view.layers))
            .and_then(|master| crate::canvas::tokens::master_background(master, dark));
        self.hint_tokens = crate::canvas::hinting::tokens::HintTokens::with_opacity(self.canvas_tokens.guide_opacity);
        font_view::project(&mut self.font_view, &mut self.font_view_outlines, &mut self.color_files, cx);
        tabs::project(&mut self.tabs, cx);
        font_info::project(&mut self.font_info, cx);
        glyph_sections::project_related(&mut self.related_glyphs, cx);
        glyph_sections::project_measurements(&mut self.measurements, cx);
        dimensions::project(&mut self.dimensions, cx);
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
        space_center::project(&mut self.space_center, &mut self.color_files, cx);
        table_viewer::project(&mut self.table_viewer, cx);
        compare::project_family(&mut self.compare_family, cx);
        compare_list::project(&mut self.compare_list, cx);
        font_audit::project(&mut self.font_audit, cx);
        compatibility::project(&mut self.compatibility, cx);
        text::project_features(&mut self.features, &self.text, cx);
        preview::project_both(&mut self.preview_area, &mut self.preview_panel, cx);
        glyph_sources::project(&mut self.glyph_sources, cx);
        self.swept = cx.view.preview.sweep.as_ref().map(AxisSweep::step);
        shape_order::project(&mut self.shape_order, cx);
        bitmap_editor::project(&mut self.bitmap_editor, cx);
        true
    }

    /// Brings the incompatibility indicators up to date; `projected` is [`Self::project`]'s answer on this pass.
    pub fn project_indicators(&mut self, projected: bool, cx: &mut ProjectCx<'_>) {
        incompatibility::project(&mut self.compat_scan, &mut self.font_view, &mut self.compatibility, &self.text, projected, cx);
    }

    /// Brings the Master Compatibility view's lines of the selected nodes up to date.
    pub fn project_probe(&mut self, selection: &fontcraft_tools::Selection, stamp: Stamp, cx: &ProjectCx<'_>) {
        compatibility::project_probe(&mut self.compatibility, &mut self.compat_probe, selection, stamp, cx);
    }

    /// Projects the Selections panel; outside [`Models::project`], since it reads the canvas's selection.
    pub fn project_selections(&mut self, cx: &mut ProjectCx<'_>, tools: &Tools, stamp: Stamp) {
        glyph_sections::project_selections(&mut self.selections, &mut self.selections_seen, cx, tools, stamp);
    }

    /// Where the axis sweep is along its axis, 0 to 1: the clock of the timing graph's dots. `None` while no axis is
    /// animated.
    pub fn sweep_position(&self) -> Option<f64> {
        self.swept.map(|step| f64::from(step) / f64::from(STEPS))
    }

    /// Projects what follows the axis sweep when its step moved: a step changes neither the engine nor the rest of
    /// the view state, so the other models stay.
    pub fn project_sweep(&mut self, cx: &mut ProjectCx<'_>) {
        let step = cx.view.preview.sweep.as_ref().map(AxisSweep::step);
        if step == self.swept {
            return;
        }
        self.swept = step;
        preview::project_both(&mut self.preview_area, &mut self.preview_panel, cx);
        glyph_sources::project(&mut self.glyph_sources, cx);
        canvas::project_location(&mut self.canvas, cx);
    }
}

/// Replaces `model`'s value with `value` only when they differ, so an unchanged model keeps its revision.
pub fn set<T: craft::ui::contract::PlainData + PartialEq>(model: &mut Tracked<T>, value: T) {
    if **model != value {
        model.edit(|m| *m = value);
    }
}
