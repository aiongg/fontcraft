//! Icons drawn with egui shapes: the Edit View tools and the Palette buttons.
//!
//! Each icon is a simple geometric design made for FontCraft (`ASSETS.md`), laid out on a unit
//! square with y pointing down and scaled to the rect it is drawn in. They need no font, so a
//! button never shows an empty box when the interface font lacks a symbol.

use std::f32::consts::{PI, TAU};

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, vec2};

/// Every icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    // Edit View tools.
    Select,
    Lasso,
    Draw,
    Pencil,
    Rectangle,
    Ellipse,
    Hand,
    Zoom,
    Knife,
    Erase,
    Rotate,
    Scale,
    Measure,
    Text,
    AnnotationSelect,
    AnnotationText,
    AnnotationArrow,
    AnnotationCircle,
    AnnotationPlus,
    AnnotationMinus,
    // Palette › Transformations.
    ReferencePoint,
    MirrorHorizontal,
    MirrorVertical,
    ScaleUp,
    ReverseScale,
    Locked,
    Unlocked,
    RotateCounterclockwise,
    RotateClockwise,
    SlantLeft,
    SlantRight,
    SlantUp,
    SlantDown,
    AlignLeft,
    AlignCenterHorizontal,
    AlignRight,
    AlignTop,
    AlignCenterVertical,
    AlignBottom,
    Union,
    Subtract,
    Intersect,
    // Palette › Stroke.
    JoinMiter,
    JoinBevel,
    JoinRound,
    JoinRoundFull,
    CapButt,
    CapSquare,
    CapRound,
    CapRoundInset,
    CapAxisAligned,
    // Font View's mode buttons.
    GridView,
    ListView,
    // Edit View's bottom bar: writing direction and kerning mode.
    LeftToRight,
    RightToLeft,
    TopToBottom,
    KerningOff,
    KerningOn,
    KerningLocked,
    // The Info box's update button for a metrics key out of sync.
    Update,
    // The bottom bar's Preview Area button: an eye.
    Preview,
}

/// Every icon, for tests and previews.
pub const ALL: &[Icon] = &[
    Icon::Select,
    Icon::Lasso,
    Icon::Draw,
    Icon::Pencil,
    Icon::Rectangle,
    Icon::Ellipse,
    Icon::Hand,
    Icon::Zoom,
    Icon::Knife,
    Icon::Erase,
    Icon::Rotate,
    Icon::Scale,
    Icon::Measure,
    Icon::Text,
    Icon::AnnotationSelect,
    Icon::AnnotationText,
    Icon::AnnotationArrow,
    Icon::AnnotationCircle,
    Icon::AnnotationPlus,
    Icon::AnnotationMinus,
    Icon::ReferencePoint,
    Icon::MirrorHorizontal,
    Icon::MirrorVertical,
    Icon::ScaleUp,
    Icon::ReverseScale,
    Icon::Locked,
    Icon::Unlocked,
    Icon::RotateCounterclockwise,
    Icon::RotateClockwise,
    Icon::SlantLeft,
    Icon::SlantRight,
    Icon::SlantUp,
    Icon::SlantDown,
    Icon::AlignLeft,
    Icon::AlignCenterHorizontal,
    Icon::AlignRight,
    Icon::AlignTop,
    Icon::AlignCenterVertical,
    Icon::AlignBottom,
    Icon::Union,
    Icon::Subtract,
    Icon::Intersect,
    Icon::JoinMiter,
    Icon::JoinBevel,
    Icon::JoinRound,
    Icon::JoinRoundFull,
    Icon::CapButt,
    Icon::CapSquare,
    Icon::CapRound,
    Icon::CapRoundInset,
    Icon::CapAxisAligned,
    Icon::GridView,
    Icon::ListView,
    Icon::LeftToRight,
    Icon::RightToLeft,
    Icon::TopToBottom,
    Icon::KerningOff,
    Icon::KerningOn,
    Icon::KerningLocked,
    Icon::Update,
    Icon::Preview,
];

/// The icon of the pointer tool `id` (an id of `fontcraft_tools::TOOLS`).
pub fn for_tool(id: &str) -> Option<Icon> {
    Some(match id {
        "select" => Icon::Select,
        "lasso" => Icon::Lasso,
        "draw" => Icon::Draw,
        "pencil" => Icon::Pencil,
        "rectangle" => Icon::Rectangle,
        "ellipse" => Icon::Ellipse,
        "hand" => Icon::Hand,
        "zoom" => Icon::Zoom,
        "knife" => Icon::Knife,
        "erase" => Icon::Erase,
        "rotate" => Icon::Rotate,
        "scale" => Icon::Scale,
        "measurement" => Icon::Measure,
        "text" => Icon::Text,
        "annotationSelect" => Icon::AnnotationSelect,
        "annotationText" => Icon::AnnotationText,
        "annotationArrow" => Icon::AnnotationArrow,
        "annotationCircle" => Icon::AnnotationCircle,
        "annotationPlus" => Icon::AnnotationPlus,
        "annotationMinus" => Icon::AnnotationMinus,
        _ => return None,
    })
}

/// Draws `icon` centred in `rect`, as large as fits.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    painter.extend(shapes(icon, rect, color));
}

/// The shapes of `icon` centred in `rect`.
pub fn shapes(icon: Icon, rect: Rect, color: Color32) -> Vec<Shape> {
    let side = rect.width().min(rect.height());
    if !rect.is_finite() || side <= 0.0 {
        return Vec::new();
    }
    let min = rect.center() - Vec2::splat(side / 2.0);
    let mut pen = Pen { min, side, color, width: (side * 0.075).max(1.0), out: Vec::new() };
    draw(&mut pen, icon);
    pen.out
}

/// A point on the icon's unit square.
type P = (f32, f32);

struct Pen {
    min: Pos2,
    side: f32,
    color: Color32,
    width: f32,
    out: Vec<Shape>,
}

impl Pen {
    fn p(&self, (x, y): P) -> Pos2 {
        self.min + vec2(x, y) * self.side
    }

    fn pts(&self, pts: &[P]) -> Vec<Pos2> {
        pts.iter().map(|&q| self.p(q)).collect()
    }

    fn stroke(&self) -> Stroke {
        Stroke::new(self.width, self.color)
    }

    fn line(&mut self, pts: &[P]) {
        let shape = Shape::line(self.pts(pts), self.stroke());
        self.out.push(shape);
    }

    /// A segment `factor` times the normal line width.
    fn thick(&mut self, a: P, b: P, factor: f32) {
        let shape = Shape::line_segment([self.p(a), self.p(b)], Stroke::new(self.width * factor, self.color));
        self.out.push(shape);
    }

    fn closed(&mut self, pts: &[P]) {
        let shape = Shape::closed_line(self.pts(pts), self.stroke());
        self.out.push(shape);
    }

    /// A filled convex polygon.
    fn fill(&mut self, pts: &[P]) {
        let shape = Shape::convex_polygon(self.pts(pts), self.color, Stroke::NONE);
        self.out.push(shape);
    }

    fn dashed(&mut self, pts: &[P]) {
        let dash = (self.side * 0.08).max(1.5);
        let shapes = Shape::dashed_line(&self.pts(pts), Stroke::new(self.width * 0.75, self.color), dash, dash);
        self.out.extend(shapes);
    }

    fn rect(&self, (x0, y0): P, (x1, y1): P) -> Rect {
        Rect::from_two_pos(self.p((x0, y0)), self.p((x1, y1)))
    }

    fn rect_stroke(&mut self, a: P, b: P, radius: f32) {
        let shape = Shape::rect_stroke(self.rect(a, b), CornerRadius::from(radius * self.side), self.stroke(), StrokeKind::Middle);
        self.out.push(shape);
    }

    fn rect_fill(&mut self, a: P, b: P, radius: f32) {
        let shape = Shape::rect_filled(self.rect(a, b), CornerRadius::from(radius * self.side), self.color);
        self.out.push(shape);
    }

    fn circle(&mut self, c: P, r: f32) {
        let shape = Shape::circle_stroke(self.p(c), r * self.side, self.stroke());
        self.out.push(shape);
    }

    fn dot(&mut self, c: P, r: f32) {
        let shape = Shape::circle_filled(self.p(c), r * self.side, self.color);
        self.out.push(shape);
    }

    fn ellipse(&mut self, (cx, cy): P, rx: f32, ry: f32) {
        let pts: Vec<P> = (0..32).map(|i| i as f32 / 32.0 * TAU).map(|a| (cx + rx * a.cos(), cy + ry * a.sin())).collect();
        self.closed(&pts);
    }

    /// A filled arrowhead with its tip at `tip`, pointing along the unit vector `dir`.
    fn head(&mut self, (tx, ty): P, (dx, dy): P, size: f32) {
        let (nx, ny) = (-dy, dx);
        let (bx, by) = (tx - dx * size, ty - dy * size);
        let half = size * 0.55;
        self.fill(&[(tx, ty), (bx + nx * half, by + ny * half), (bx - nx * half, by - ny * half)]);
    }

    /// A horizontal band from the left edge, ending in `end` (from its top edge to its bottom
    /// edge), and its end node at (0.6, 0.5).
    fn cap(&mut self, end: &[P]) {
        let mut outline = vec![(0.08, 0.64), (0.08, 0.36)];
        outline.extend_from_slice(end);
        self.closed(&outline);
        self.dot((0.6, 0.5), 0.06);
    }

    /// A straight arrow from `a` to `b`.
    fn arrow(&mut self, a: P, b: P) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f32::EPSILON {
            return;
        }
        let dir = (dx / len, dy / len);
        let size = 0.2;
        self.line(&[a, (b.0 - dir.0 * size * 0.8, b.1 - dir.1 * size * 0.8)]);
        self.head(b, dir, size);
    }

    /// A circular arc from angle `a0` turning `sweep` radians (positive: clockwise on screen)
    /// with an arrowhead at its end.
    fn arc_arrow(&mut self, (cx, cy): P, r: f32, a0: f32, sweep: f32) {
        let size = 0.2;
        // Stop the line short of the tip so the head covers its end.
        let trimmed = sweep - sweep.signum() * (size * 0.7 / r);
        let steps = 24;
        let pts: Vec<P> = (0..=steps).map(|i| a0 + trimmed * i as f32 / steps as f32).map(|a| (cx + r * a.cos(), cy + r * a.sin())).collect();
        self.line(&pts);
        let a1 = a0 + sweep;
        let s = sweep.signum();
        self.head((cx + r * a1.cos(), cy + r * a1.sin()), (-a1.sin() * s, a1.cos() * s), size);
    }

    /// The selection arrow with its tip at `(x, y)`, `scale` times full size.
    fn pointer(&mut self, (x, y): P, scale: f32) {
        let at = |(px, py): P| (x + (px - 0.30) * scale, y + (py - 0.12) * scale);
        self.fill(&[at((0.30, 0.12)), at((0.30, 0.74)), at((0.72, 0.58))]);
        self.fill(&[at((0.43, 0.60)), at((0.55, 0.86)), at((0.65, 0.81)), at((0.53, 0.56))]);
    }

    /// The outline of the square [0.28, 0.72]² sheared by `sx` (top to the right) and `sy`
    /// (right side up), over the dashed upright square.
    fn slant(&mut self, sx: f32, sy: f32) {
        let square = [(0.28, 0.28), (0.72, 0.28), (0.72, 0.72), (0.28, 0.72), (0.28, 0.28)];
        self.dashed(&square);
        let sheared: Vec<P> = square.iter().take(4).map(|&(x, y)| (x + sx * (0.5 - y), y - sy * (x - 0.5))).collect();
        self.closed(&sheared);
    }

    /// Two bars aligned on an axis at `at` (0 left or top, 0.5 centre, 1 right or bottom);
    /// `vertical` lays the picture on its side for the vertical alignments.
    /// Three lines of text set from one edge with an arrow along them: left to right, right to
    /// left, or (vertical) columns from the top.
    fn writing(&mut self, rtl: bool, vertical: bool) {
        let place = |(x, y): P| {
            let x = if rtl { 1.0 - x } else { x };
            if vertical { (1.0 - y, x) } else { (x, y) }
        };
        for (y, len) in [(0.2, 0.7), (0.4, 0.5), (0.6, 0.62)] {
            self.line(&[place((0.14, y)), place((0.14 + len, y))]);
        }
        self.line(&[place((0.14, 0.82)), place((0.7, 0.82))]);
        let dir = if vertical {
            (0.0, 1.0)
        } else if rtl {
            (-1.0, 0.0)
        } else {
            (1.0, 0.0)
        };
        self.head(place((0.88, 0.82)), dir, 0.18);
    }

    /// A V and an A between heights `top` and `bottom`, their boxes apart (a positive `gap`) or
    /// overlapping (a negative one).
    fn kern_pair(&mut self, gap: f32, top: f32, bottom: f32) {
        let (v0, a1) = (0.5 - gap / 2.0, 0.5 + gap / 2.0);
        let bar = top + (bottom - top) * 0.62;
        self.line(&[(v0 - 0.4, top), (v0 - 0.2, bottom), (v0, top)]);
        self.line(&[(a1, bottom), (a1 + 0.2, top), (a1 + 0.4, bottom)]);
        self.line(&[(a1 + 0.08, bar), (a1 + 0.32, bar)]);
    }

    fn align(&mut self, at: f32, vertical: bool) {
        let place = |(x, y): P| if vertical { (y, x) } else { (x, y) };
        let axis = 0.16 + at * 0.68;
        let bar = |len: f32| match at {
            a if a < 0.25 => (axis, axis + len),
            a if a > 0.75 => (axis - len, axis),
            _ => (axis - len / 2.0, axis + len / 2.0),
        };
        let ((a0, a1), (b0, b1)) = (bar(0.6), bar(0.38));
        self.rect_fill(place((a0, 0.24)), place((a1, 0.42)), 0.0);
        self.rect_fill(place((b0, 0.58)), place((b1, 0.76)), 0.0);
        self.line(&[place((axis, 0.1)), place((axis, 0.9))]);
    }
}

fn draw(pen: &mut Pen, icon: Icon) {
    match icon {
        Icon::Select => pen.pointer((0.30, 0.12), 1.0),
        Icon::Lasso => {
            pen.ellipse((0.54, 0.38), 0.32, 0.2);
            pen.line(&[(0.32, 0.53), (0.27, 0.66), (0.30, 0.78), (0.40, 0.88)]);
            pen.dot((0.32, 0.54), 0.045);
        }
        Icon::Draw => {
            pen.closed(&[(0.5, 0.9), (0.26, 0.52), (0.36, 0.14), (0.64, 0.14), (0.74, 0.52)]);
            pen.line(&[(0.5, 0.9), (0.5, 0.58)]);
            pen.dot((0.5, 0.52), 0.055);
        }
        Icon::Pencil => {
            let (tip, b1, b2, f1, f2) = ((0.16, 0.84), (0.23, 0.63), (0.37, 0.77), (0.65, 0.21), (0.79, 0.35));
            pen.closed(&[tip, b2, f2, f1, b1]);
            pen.line(&[b1, b2]);
            pen.fill(&[tip, (0.21, 0.74), (0.26, 0.79)]);
        }
        Icon::Rectangle => pen.closed(&[(0.18, 0.26), (0.82, 0.26), (0.82, 0.74), (0.18, 0.74)]),
        Icon::Ellipse => pen.ellipse((0.5, 0.5), 0.34, 0.26),
        Icon::Hand => {
            for (x0, top) in [(0.30, 0.26), (0.41, 0.14), (0.52, 0.17), (0.63, 0.28)] {
                pen.rect_fill((x0, top), (x0 + 0.09, 0.62), 0.045);
            }
            pen.rect_fill((0.30, 0.50), (0.72, 0.88), 0.1);
            pen.fill(&[(0.20, 0.50), (0.30, 0.60), (0.30, 0.78), (0.13, 0.57)]);
        }
        Icon::Zoom => {
            pen.circle((0.42, 0.42), 0.25);
            pen.thick((0.60, 0.60), (0.85, 0.85), 2.2);
        }
        Icon::Knife => {
            pen.fill(&[(0.14, 0.86), (0.56, 0.36), (0.66, 0.46)]);
            pen.thick((0.63, 0.39), (0.85, 0.17), 2.6);
        }
        Icon::Erase => {
            pen.closed(&[(0.18, 0.62), (0.50, 0.30), (0.74, 0.54), (0.42, 0.86)]);
            pen.fill(&[(0.18, 0.62), (0.34, 0.46), (0.58, 0.70), (0.42, 0.86)]);
            pen.line(&[(0.42, 0.86), (0.86, 0.86)]);
        }
        Icon::Rotate => {
            pen.arc_arrow((0.5, 0.5), 0.32, PI * 0.9, PI * 1.45);
            pen.rect_fill((0.42, 0.42), (0.58, 0.58), 0.0);
        }
        Icon::Scale | Icon::ScaleUp => {
            pen.closed(&[(0.14, 0.14), (0.86, 0.14), (0.86, 0.86), (0.14, 0.86)]);
            pen.rect_fill((0.14, 0.58), (0.42, 0.86), 0.0);
            pen.arrow((0.46, 0.54), (0.76, 0.24));
        }
        Icon::ReverseScale => {
            pen.closed(&[(0.14, 0.14), (0.86, 0.14), (0.86, 0.86), (0.14, 0.86)]);
            pen.rect_fill((0.14, 0.58), (0.42, 0.86), 0.0);
            pen.arrow((0.78, 0.22), (0.48, 0.52));
        }
        Icon::Measure => {
            pen.closed(&[(0.08, 0.36), (0.92, 0.36), (0.92, 0.64), (0.08, 0.64)]);
            for (i, x) in [0.2, 0.32, 0.44, 0.56, 0.68, 0.8].into_iter().enumerate() {
                let depth = if i % 2 == 0 { 0.46 } else { 0.52 };
                pen.line(&[(x, 0.36), (x, depth)]);
            }
        }
        Icon::Text => {
            pen.line(&[(0.22, 0.3), (0.22, 0.18), (0.78, 0.18), (0.78, 0.3)]);
            pen.thick((0.5, 0.18), (0.5, 0.84), 1.4);
            pen.line(&[(0.38, 0.84), (0.62, 0.84)]);
        }
        Icon::AnnotationSelect => {
            pen.pointer((0.12, 0.08), 0.75);
            pen.rect_stroke((0.54, 0.56), (0.9, 0.86), 0.04);
        }
        Icon::AnnotationText => {
            pen.rect_stroke((0.12, 0.16), (0.88, 0.66), 0.06);
            pen.fill(&[(0.28, 0.66), (0.28, 0.86), (0.46, 0.66)]);
            pen.line(&[(0.24, 0.33), (0.76, 0.33)]);
            pen.line(&[(0.24, 0.49), (0.60, 0.49)]);
        }
        Icon::AnnotationArrow => pen.arrow((0.16, 0.84), (0.84, 0.16)),
        Icon::AnnotationCircle => pen.circle((0.5, 0.5), 0.32),
        Icon::AnnotationPlus => {
            pen.thick((0.5, 0.18), (0.5, 0.82), 1.6);
            pen.thick((0.18, 0.5), (0.82, 0.5), 1.6);
        }
        Icon::AnnotationMinus => pen.thick((0.18, 0.5), (0.82, 0.5), 1.6),
        Icon::ReferencePoint => {
            pen.circle((0.5, 0.5), 0.25);
            pen.line(&[(0.5, 0.1), (0.5, 0.9)]);
            pen.line(&[(0.1, 0.5), (0.9, 0.5)]);
        }
        Icon::MirrorHorizontal => {
            pen.dashed(&[(0.5, 0.08), (0.5, 0.92)]);
            pen.fill(&[(0.42, 0.22), (0.42, 0.78), (0.12, 0.78)]);
            pen.closed(&[(0.58, 0.22), (0.58, 0.78), (0.88, 0.78)]);
        }
        Icon::MirrorVertical => {
            pen.dashed(&[(0.08, 0.5), (0.92, 0.5)]);
            pen.fill(&[(0.22, 0.42), (0.78, 0.42), (0.78, 0.12)]);
            pen.closed(&[(0.22, 0.58), (0.78, 0.58), (0.78, 0.88)]);
        }
        Icon::Locked | Icon::Unlocked => {
            let lift = if icon == Icon::Locked { 0.0 } else { 0.12 };
            let top = 0.38 - lift;
            let arc: Vec<P> = (0..=16).map(|i| PI + PI * i as f32 / 16.0).map(|a| (0.5 + 0.17 * a.cos(), top + 0.17 * a.sin())).collect();
            pen.line(&arc);
            pen.line(&[(0.33, top), (0.33, 0.48)]);
            pen.line(&[(0.67, top), (0.67, if lift > 0.0 { top + 0.06 } else { 0.48 })]);
            pen.rect_fill((0.24, 0.48), (0.76, 0.86), 0.06);
        }
        Icon::RotateCounterclockwise => pen.arc_arrow((0.5, 0.52), 0.3, 0.0, -PI * 1.5),
        Icon::RotateClockwise => pen.arc_arrow((0.5, 0.52), 0.3, PI, PI * 1.5),
        Icon::SlantLeft => pen.slant(-0.5, 0.0),
        Icon::SlantRight => pen.slant(0.5, 0.0),
        Icon::SlantUp => pen.slant(0.0, 0.5),
        Icon::SlantDown => pen.slant(0.0, -0.5),
        Icon::AlignLeft => pen.align(0.0, false),
        Icon::AlignCenterHorizontal => pen.align(0.5, false),
        Icon::AlignRight => pen.align(1.0, false),
        Icon::AlignTop => pen.align(0.0, true),
        Icon::AlignCenterVertical => pen.align(0.5, true),
        Icon::AlignBottom => pen.align(1.0, true),
        Icon::Union => {
            pen.rect_fill((0.14, 0.14), (0.62, 0.62), 0.0);
            pen.rect_fill((0.38, 0.38), (0.86, 0.86), 0.0);
        }
        Icon::Subtract => {
            pen.rect_fill((0.14, 0.14), (0.62, 0.38), 0.0);
            pen.rect_fill((0.14, 0.38), (0.38, 0.62), 0.0);
            pen.dashed(&[(0.38, 0.38), (0.86, 0.38), (0.86, 0.86), (0.38, 0.86), (0.38, 0.38)]);
        }
        Icon::Intersect => {
            pen.closed(&[(0.14, 0.14), (0.62, 0.14), (0.62, 0.62), (0.14, 0.62)]);
            pen.closed(&[(0.38, 0.38), (0.86, 0.38), (0.86, 0.86), (0.38, 0.86)]);
            pen.rect_fill((0.38, 0.38), (0.62, 0.62), 0.0);
        }
        // Joins: the outline of a wide band turning a corner, and the corner node, as the caps
        // draw a band's end. The band fills the icon's square like the other icons' drawings:
        // outer edge at 0.04, inner edge at 0.54, the path along 0.29.
        Icon::JoinMiter | Icon::JoinBevel | Icon::JoinRound | Icon::JoinRoundFull => {
            let outer: Vec<P> = match icon {
                Icon::JoinMiter => vec![(0.04, 0.98), (0.04, 0.04), (0.98, 0.04)],
                Icon::JoinBevel => vec![(0.04, 0.98), (0.04, 0.29), (0.29, 0.04), (0.98, 0.04)],
                _ => [(0.04, 0.98)].into_iter().chain(quarter_arc((0.29, 0.29), 0.25)).chain([(0.98, 0.04)]).collect(),
            };
            let inner: Vec<P> = match icon {
                Icon::JoinRoundFull => [(0.54, 0.98)].into_iter().chain(quarter_arc((0.79, 0.79), 0.25)).chain([(0.98, 0.54)]).collect(),
                _ => vec![(0.54, 0.98), (0.54, 0.54), (0.98, 0.54)],
            };
            let mut outline = outer;
            outline.extend(inner.into_iter().rev());
            pen.closed(&outline);
            pen.dot((0.29, 0.29), 0.07);
        }
        // Caps: the outline of a band's end, with its end node.
        Icon::CapButt => pen.cap(&[(0.6, 0.36), (0.6, 0.64)]),
        Icon::CapSquare => pen.cap(&[(0.74, 0.36), (0.74, 0.64)]),
        Icon::CapRound => pen.cap(&half_circle((0.6, 0.5), 0.14)),
        Icon::CapRoundInset => pen.cap(&half_circle((0.46, 0.5), 0.14)),
        Icon::CapAxisAligned => {
            pen.closed(&[(0.05, 0.75), (0.42, 0.38), (0.82, 0.38), (0.25, 0.95)]);
            pen.dot((0.62, 0.38), 0.06);
        }
        Icon::GridView => {
            for y in [0.14, 0.56] {
                for x in [0.14, 0.56] {
                    pen.rect_fill((x, y), (x + 0.3, y + 0.3), 0.0);
                }
            }
        }
        Icon::LeftToRight => pen.writing(false, false),
        Icon::RightToLeft => pen.writing(true, false),
        Icon::TopToBottom => pen.writing(false, true),
        Icon::KerningOff => pen.kern_pair(0.12, 0.22, 0.78),
        Icon::KerningOn => pen.kern_pair(-0.16, 0.22, 0.78),
        Icon::Update => {
            pen.arc_arrow((0.5, 0.5), 0.3, PI * 0.1, PI * 0.8);
            pen.arc_arrow((0.5, 0.5), 0.3, PI * 1.1, PI * 0.8);
        }
        Icon::KerningLocked => {
            pen.kern_pair(-0.16, 0.06, 0.5);
            let arc: Vec<P> = (0..=12).map(|i| PI + PI * i as f32 / 12.0).map(|a| (0.5 + 0.11 * a.cos(), 0.7 + 0.11 * a.sin())).collect();
            pen.line(&arc);
            pen.rect_fill((0.33, 0.7), (0.67, 0.95), 0.04);
        }
        Icon::Preview => {
            // Two arcs meeting at the corners of the eye, and the pupil.
            let lid =
                |sign: f32| -> Vec<P> { (0..=16).map(|i| i as f32 / 16.0).map(|t| (0.1 + 0.8 * t, 0.5 + sign * 0.28 * (PI * t).sin())).collect() };
            pen.line(&lid(-1.0));
            pen.line(&lid(1.0));
            pen.circle((0.5, 0.5), 0.13);
            pen.dot((0.5, 0.5), 0.05);
        }
        Icon::ListView => {
            for y in [0.2, 0.5, 0.8] {
                pen.dot((0.16, y), 0.06);
                pen.line(&[(0.32, y), (0.88, y)]);
            }
        }
    }
}

/// The upper-left quarter of a circle around `c`, from its left point to its top point.
fn quarter_arc((cx, cy): P, r: f32) -> Vec<P> {
    (0..=8).map(|i| PI + PI / 2.0 * i as f32 / 8.0).map(|a| (cx + r * a.cos(), cy + r * a.sin())).collect()
}

/// The right half of a circle around `c`, from top to bottom.
fn half_circle((cx, cy): P, r: f32) -> Vec<P> {
    (0..=12).map(|i| -PI / 2.0 + PI * i as f32 / 12.0).map(|a| (cx + r * a.cos(), cy + r * a.sin())).collect()
}

/// A small filled triangle in the bottom-right corner of `rect`: the mark of a button that
/// opens a menu when held.
pub fn menu_mark(rect: Rect, size: f32, color: Color32) -> Shape {
    let corner = rect.right_bottom() - Vec2::splat(2.0);
    Shape::convex_polygon(vec![corner, corner - vec2(size, 0.0), corner - vec2(0.0, size)], color, Stroke::NONE)
}
