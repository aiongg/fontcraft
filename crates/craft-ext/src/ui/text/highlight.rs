//! Colour runs, underlines and fills over the text of a field: syntax colouring, error underlines and the current find
//! match on a code field.
//!
//! Meant for craft-ui-contract, `src/primitives/text_input.rs` and `src/runtime/egui/text_edit.rs`: there it is a
//! `highlight` option on `TextOptions` (a function from the text to its runs) that the text widget's layouter turns into
//! sections of the field's one layout, and a `TextField::highlight` prop. The layouter is private to craft, so this
//! stand-in works on what the field painted: after the field has shown, it finds the field's laid-out text among the
//! pass's shapes, lays the same text out again with its sections cut at the runs' edges and each run's colour and
//! underline, paints the selection into it the way the widget does, and swaps the new layout in. Colour and underline do
//! not change where lines break, so the caret and the selection stay where the widget put them.
//!
//! It covers a field whose text is the only text painted inside its rect: no prefix, and no placeholder (the runs would
//! colour the placeholder of an empty field).

use std::ops::Range;
use std::sync::Arc;

use craft::theme::Srgba;
use craft::ui::contract::EditResponse;
use craft::ui::runtime::env::Env;
use egui::epaint::TextShape;
use egui::layers::ShapeIdx;
use egui::text::{ByteIndex, LayoutJob, LayoutSection};
use egui::text_edit::TextEditState;
use egui::text_selection::visuals::paint_text_selection;
use egui::{Color32, Galley, Response, Shape, Stroke, Ui};

/// Above this many bytes the text shows plain: colouring is a convenience, and a long text is laid out twice with it.
pub const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;
/// Most runs applied to one text; the rest are dropped.
pub const MAX_HIGHLIGHTS: usize = 50_000;

/// One run over a field's text: the bytes it covers, the colour they take and the underline they get.
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    /// Bytes of the text; edges inside a character move back to its start.
    pub range: Range<usize>,
    /// The text colour; `None` keeps the field's.
    pub ink: Option<Color32>,
    /// An underline in this colour, as thick as the theme's thin stroke.
    pub underline: Option<Color32>,
    /// A fill behind the text in this colour.
    pub background: Option<Color32>,
}

impl Highlight {
    /// `range` in `ink`.
    pub fn ink(range: Range<usize>, ink: Color32) -> Self {
        Highlight { range, ink: Some(ink), underline: None, background: None }
    }

    /// `range` underlined in `color` (an error).
    pub fn underline(range: Range<usize>, color: Color32) -> Self {
        Highlight { range, ink: None, underline: Some(color), background: None }
    }

    /// `range` on a fill of `color` (a found match the field does not show selected).
    pub fn background(range: Range<usize>, color: Color32) -> Self {
        Highlight { range, ink: None, underline: None, background: Some(color) }
    }
}

/// The largest character boundary of `text` at or before `at`.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// `job` with its sections cut at the edges of `runs`: each piece keeps its section's format (font, line height, axes)
/// and takes the colour of the run that covers it and the underline of every underlined run over it.
pub fn recoloured(job: &LayoutJob, runs: &[Highlight], underline_width: f32) -> LayoutJob {
    let text = job.text.as_str();
    let clamp = |r: &Range<usize>| floor_boundary(text, r.start)..floor_boundary(text, r.end);
    let mut inks: Vec<(Range<usize>, Color32)> =
        runs.iter().take(MAX_HIGHLIGHTS).filter_map(|h| Some((clamp(&h.range), h.ink?))).filter(|(r, _)| r.start < r.end).collect();
    inks.sort_by_key(|(r, _)| r.start);
    let mut lines: Vec<(Range<usize>, Color32)> =
        runs.iter().take(MAX_HIGHLIGHTS).filter_map(|h| Some((clamp(&h.range), h.underline?))).filter(|(r, _)| r.start < r.end).collect();
    lines.sort_by_key(|(r, _)| r.start);
    let fills: Vec<(Range<usize>, Color32)> =
        runs.iter().take(MAX_HIGHLIGHTS).filter_map(|h| Some((clamp(&h.range), h.background?))).filter(|(r, _)| r.start < r.end).collect();
    // Every place the format may change, in order.
    let mut cuts: Vec<usize> = Vec::with_capacity(2 * (job.sections.len() + inks.len() + lines.len() + fills.len()) + 2);
    cuts.push(0);
    cuts.push(text.len());
    for section in &job.sections {
        cuts.push(floor_boundary(text, section.byte_range.start.0));
        cuts.push(floor_boundary(text, section.byte_range.end.0));
    }
    for (r, _) in inks.iter().chain(&lines).chain(&fills) {
        cuts.push(r.start);
        cuts.push(r.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut sections: Vec<LayoutSection> = Vec::with_capacity(cuts.len());
    let (mut section, mut ink) = (0usize, 0usize);
    for pair in cuts.windows(2) {
        let [from, to] = pair else { continue };
        if from >= to {
            continue;
        }
        while job.sections.get(section).is_some_and(|s| s.byte_range.end.0 <= *from) {
            section += 1;
        }
        let Some(base) = job.sections.get(section).filter(|s| s.byte_range.start.0 <= *from) else { continue };
        while inks.get(ink).is_some_and(|(r, _)| r.end <= *from) {
            ink += 1;
        }
        let mut format = base.format.clone();
        if let Some((_, color)) = inks.get(ink).filter(|(r, _)| r.start <= *from) {
            format.color = *color;
        }
        // Underlines are few (errors): the last one over the piece gives the colour.
        if let Some((_, color)) = lines.iter().rev().find(|(r, _)| r.start <= *from && *from < r.end) {
            format.underline = Stroke::new(underline_width, *color);
        }
        // Fills are few (the current match): the last one over the piece gives the colour.
        if let Some((_, color)) = fills.iter().rev().find(|(r, _)| r.start <= *from && *from < r.end) {
            format.background = *color;
        }
        // Only the first piece of a section keeps its leading space (a first-row indent).
        let leading_space = if base.byte_range.start.0 == *from { base.leading_space } else { 0.0 };
        sections.push(LayoutSection { leading_space, byte_range: ByteIndex(*from)..ByteIndex(*to), format });
    }
    let mut out = job.clone();
    out.sections = sections;
    out
}

/// Colours a field's text after the field has shown.
pub trait HighlightText {
    /// Colours and underlines the text the field shows (the draft being typed, else its value) with the runs `runs`
    /// gives for that text. Call it right after showing the field, every pass. Returns whether the field's text was
    /// found among the pass's shapes; an empty field or a text past [`MAX_HIGHLIGHT_BYTES`] shows plain.
    fn highlight_text(&self, ui: &Ui, runs: impl FnOnce(&str) -> Vec<Highlight>) -> bool;
}

impl<T> HighlightText for EditResponse<T> {
    fn highlight_text(&self, ui: &Ui, runs: impl FnOnce(&str) -> Vec<Highlight>) -> bool {
        highlight_response(ui, &self.response, runs)
    }
}

impl HighlightText for Response {
    fn highlight_text(&self, ui: &Ui, runs: impl FnOnce(&str) -> Vec<Highlight>) -> bool {
        highlight_response(ui, self, runs)
    }
}

/// Where the field of `response` painted its text in this pass, and how it laid the text out: the text shape's
/// position and layout (the coloured one once [`HighlightText::highlight_text`] ran). Rows sit at `pos + row.pos`.
/// `None` for an empty field. A gutter beside a code field reads its line positions from it.
pub fn field_layout(ui: &Ui, response: &Response) -> Option<(egui::Pos2, Arc<Galley>)> {
    field_text(ui, response).map(|(_, shape)| (shape.pos, shape.galley))
}

/// The last text shape painted inside `response`'s rect on its layer, and its place in the layer's list.
fn field_text(ui: &Ui, response: &Response) -> Option<(usize, TextShape)> {
    let rect = response.rect;
    ui.ctx().graphics(|layers| {
        let list = layers.get(response.layer_id)?;
        // The paint list iterates forward only; the last match is the field's text.
        list.all_entries()
            .enumerate()
            .filter_map(|(at, clipped)| match &clipped.shape {
                Shape::Text(text) if rect.contains(text.pos) && !text.galley.job.text.is_empty() => Some((at, text)),
                _ => None,
            })
            .last()
            .map(|(at, text)| (at, text.clone()))
    })
}

fn highlight_response(ui: &Ui, response: &Response, runs: impl FnOnce(&str) -> Vec<Highlight>) -> bool {
    let Some((at, shape)) = field_text(ui, response) else { return false };
    let text = shape.galley.job.text.as_str();
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return true;
    }
    let runs = runs(text);
    if runs.is_empty() {
        return true;
    }
    let env = Env::of(ui);
    let color = |c| Color32::from(Srgba::from(c));
    let job = recoloured(&shape.galley.job, &runs, env.tokens.stroke.thin);
    let ctx = ui.ctx();
    let mut galley: Arc<Galley> = ctx.fonts_mut(|fonts| fonts.layout_job(job));
    // The widget painted its selection into the layout it showed; the new layout gets it the same way, in the colours
    // craft's text widget sets (`text_edit::show`).
    let widget = super::text_widget(response.id);
    if super::text_focused(ctx, response.id)
        && let Some(range) = TextEditState::load(ctx, widget).and_then(|state| state.cursor.range(&galley))
        && !range.is_empty()
    {
        let mut visuals = ui.visuals().clone();
        visuals.selection.bg_fill = color(env.tokens.color.accent.soft);
        visuals.selection.stroke.color = color(env.tokens.color.text.normal);
        paint_text_selection(&mut galley, &visuals, &range, None);
    }
    let old = shape.galley;
    ctx.graphics_mut(|layers| {
        let list = layers.entry(response.layer_id);
        // Swapped only if the shape is still the one found: nothing painted in between.
        let same = list.all_entries().nth(at).is_some_and(|clipped| matches!(&clipped.shape, Shape::Text(t) if Arc::ptr_eq(&t.galley, &old)));
        if same {
            list.mutate_shape(ShapeIdx(at), |clipped| {
                if let Shape::Text(text) = &mut clipped.shape {
                    text.galley = galley;
                }
            });
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use craft::ui::contract::{Bind, Show as _};
    use craft::ui::testing::Fixture;
    use craft::ui::text::TextField;
    use egui::text::TextFormat;
    use egui::{FontId, Shape};

    use super::*;

    fn job(text: &str) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.append(text, 0.0, TextFormat::simple(FontId::monospace(12.0), Color32::GRAY));
        job
    }

    fn pieces(job: &LayoutJob) -> Vec<(Range<usize>, Color32, bool)> {
        job.sections.iter().map(|s| (s.byte_range.start.0..s.byte_range.end.0, s.format.color, s.format.underline.width > 0.0)).collect()
    }

    /// Runs cut the section at their edges, an edge inside a character moves to its start, and runs past the text or
    /// empty are dropped.
    #[test]
    fn runs_cut_the_text_on_character_boundaries() {
        let code = "sub é by f;";
        let runs = [
            Highlight::ink(0..3, Color32::RED),
            // Byte 5 is inside "é" (bytes 4..6): the run starts at 4.
            Highlight::underline(5..6, Color32::BLUE),
            Highlight::ink(9..9, Color32::GREEN),
            Highlight::ink(40..50, Color32::GREEN),
        ];
        let out = recoloured(&job(code), &runs, 1.0);
        assert_eq!(out.text, code);
        assert_eq!(
            pieces(&out),
            vec![(0..3, Color32::RED, false), (3..4, Color32::GRAY, false), (4..6, Color32::GRAY, true), (6..code.len(), Color32::GRAY, false)]
        );
    }

    /// The field's painted text takes the runs' colours and underlines, and keeps its words.
    #[test]
    fn a_highlighted_field_paints_its_runs() {
        let mut fixture = Fixture::new(|ui| {
            let out = TextField::new("Code", Bind::given("feature liga {".to_string())).mono().lines(4).show(ui);
            let found = out.highlight_text(ui, |text| {
                assert_eq!(text, "feature liga {");
                vec![Highlight::ink(0..7, Color32::RED), Highlight::underline(8..12, Color32::BLUE)]
            });
            assert!(found, "the field's text is found");
        });
        fixture.settle();
        let shapes = fixture.frame_shapes();
        let text = shapes
            .iter()
            .find_map(|shape| match shape {
                Shape::Text(t) if t.galley.job.text == "feature liga {" => Some(t.galley.clone()),
                _ => None,
            })
            .expect("the field's text is painted");
        let sections = pieces(&text.job);
        assert!(sections.contains(&(0..7, Color32::RED, false)), "{sections:?}");
        assert!(sections.iter().any(|(r, _, line)| *r == (8..12) && *line), "{sections:?}");
    }
}
