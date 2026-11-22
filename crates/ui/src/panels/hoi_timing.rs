//! The Info box's timing graph (M9.32, row `hoi.timingGraph`), under the interpolation curve part (`info_box_hoi`):
//! position along the span across, share of the curve's length travelled up, for the one selected node. The diagonal
//! of constant speed is drawn faint behind it; a linear node's graph is that diagonal. Read-only: the curve is edited
//! in the part above and with the `hoi.*` commands.

use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::runtime::env::Env;
use craft::ui::sections::Section;
use egui::{Color32, Pos2, Rect, Shape, Stroke, Ui};

use crate::app::models::hoi::TimingGraph;

craft::ui::words! {
    /// The words of the timing graph.
    pub struct TimingWords {
        title: "fontcraftHoiTiming.title" = "Interpolation Timing";
        graph: "fontcraftHoiTiming.graph" = "Timing graph: position along the span across, share of the path travelled up";
        axis: "fontcraftHoiTiming.axis" = "Axis";
    }
}

/// The diagonal's ink, as a share of the graph's.
const DIAGONAL_SHARE: f32 = 0.3;

pub fn view(ui: &mut Ui, graph: &TimingGraph) {
    let words = Env::of(ui).words::<TimingWords>();
    Section::new(words.title.clone()).id_salt("fontcraftHoiTiming").show(ui, |ui, _| {
        Text::new(format!("{} {}", words.axis.as_str(), graph.axis)).show(ui);
        let points = graph.points.clone();
        Picture::painter(words.graph.clone(), ThumbAspect::SQUARE, move |cx, rect| paint(&cx.painter, cx.ink(), rect, &points))
            .extent(PictureSize::Thumb(ThumbSize::Large))
            .show(ui);
    });
}

/// Paints the frame, the diagonal and the graph into `rect`.
fn paint(painter: &egui::Painter, ink: Color32, rect: Rect, points: &[[f64; 2]]) {
    let hairline = 1.0 / painter.pixels_per_point().max(0.01);
    let at =
        |[s, t]: [f64; 2]| Pos2::new(rect.left() + s.clamp(0.0, 1.0) as f32 * rect.width(), rect.bottom() - t.clamp(0.0, 1.0) as f32 * rect.height());
    let faint = ink.gamma_multiply(DIAGONAL_SHARE);
    painter.rect_stroke(rect, 0.0, Stroke::new(hairline, faint), egui::StrokeKind::Inside);
    painter.line_segment([at([0.0, 0.0]), at([1.0, 1.0])], Stroke::new(hairline, faint));
    let line: Vec<Pos2> = points.iter().copied().filter(|[s, t]| s.is_finite() && t.is_finite()).map(at).collect();
    if line.len() > 1 {
        painter.add(Shape::line(line, Stroke::new(hairline * 2.0, ink)));
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::testing::Fixture;

    use super::view;
    use crate::app::models::hoi::TimingGraph;

    /// A node at constant speed gives the diagonal; the graph shows under its axis.
    #[test]
    fn the_timing_graph_shows_for_a_curve() {
        let graph = TimingGraph { axis: "wght".into(), points: (0..=4).map(|i| [f64::from(i) / 4.0, f64::from(i) / 4.0]).collect() };
        let mut fixture = Fixture::new(move |ui| view(ui, &graph));
        fixture.settle();
        assert!(fixture.by_label("Axis wght").is_some(), "the axis line");
        assert!(fixture.by_label("Timing graph: position along the span across, share of the path travelled up").is_some(), "the graph");
    }
}
