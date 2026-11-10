//! Renderer-agnostic geometry.
//!
//! The pizarra engine describes what to draw as [`DrawCommand`]s: some in
//! world coordinates (finished shapes) and some in screen coordinates (visual
//! helpers such as snapping circles). This module flattens those commands into
//! [`Primitive`]s whose segments are already in the target pixel space, so the
//! same code feeds both the GPUI canvas and the PNG exporter. The canvas then
//! joins the pieces of pressure strokes with [`join_pen_strokes`].

use pizarra::{
    color::Color,
    draw_commands::DrawCommand,
    geom::Ellipse,
    path_command::{CubicBezierCurve, PathCommand},
    point::{Unit, Vec2D, WorldUnit},
    style::Style,
    transform::Transform,
};

/// Control-point distance that best approximates a quarter circle with a cubic
/// Bézier curve: `4/3 * tan(π/8)`.
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// Long paths are split into chunks of at most this many segments. GPUI
/// tessellates paths into `u16`-indexed buffers, so a single huge stroke could
/// otherwise overflow and silently disappear; chunks also allow culling the
/// off-screen parts of a long stroke.
const MAX_SEGMENTS_PER_CHUNK: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    fn distance(self, other: Pt) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    MoveTo(Pt),
    LineTo(Pt),
    CubicTo { c1: Pt, c2: Pt, to: Pt },
    Close,
}

impl Seg {
    fn points(&self) -> impl Iterator<Item = Pt> {
        let (a, b, c) = match *self {
            Seg::MoveTo(p) | Seg::LineTo(p) => (Some(p), None, None),
            Seg::CubicTo { c1, c2, to } => (Some(c1), Some(c2), Some(to)),
            Seg::Close => (None, None, None),
        };
        a.into_iter().chain(b).chain(c)
    }

    fn end(&self) -> Option<Pt> {
        match *self {
            Seg::MoveTo(p) | Seg::LineTo(p) => Some(p),
            Seg::CubicTo { to, .. } => Some(to),
            Seg::Close => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    pub width: f64,
    pub color: Color,
}

/// A path in target pixel space plus how to paint it. When both are present the
/// fill is painted first and the stroke on top, like SVG does.
#[derive(Debug, Clone, PartialEq)]
pub struct Primitive {
    pub segments: Vec<Seg>,
    pub fill: Option<Color>,
    pub stroke: Option<StrokeStyle>,
}

impl Primitive {
    /// Axis-aligned bounds of every point, including control points (a cubic
    /// curve always lies inside the hull of its control points), grown by half
    /// the stroke width.
    pub fn bounds(&self) -> Option<(Pt, Pt)> {
        padded_bounds(&self.segments, self.stroke.map_or(0.0, |s| s.width / 2.0))
    }
}

fn padded_bounds(segments: &[Seg], pad: f64) -> Option<(Pt, Pt)> {
    let mut points = segments.iter().flat_map(Seg::points);
    let first = points.next()?;
    let (mut min, mut max) = (first, first);
    for p in points {
        min = Pt::new(min.x.min(p.x), min.y.min(p.y));
        max = Pt::new(max.x.max(p.x), max.y.max(p.y));
    }
    Some((Pt::new(min.x - pad, min.y - pad), Pt::new(max.x + pad, max.y + pad)))
}

/// A stroke whose width changes along its length.
#[derive(Debug, Clone, PartialEq)]
pub struct PenStroke {
    /// A `MoveTo` followed by lines and curves.
    pub segments: Vec<Seg>,
    /// Width of the stroke at the end of each of `segments`.
    pub widths: Vec<f64>,
    pub color: Color,
}

impl PenStroke {
    fn start(from: Pt, start_width: f64, seg: Seg, width: f64, color: Color) -> Self {
        Self { segments: vec![Seg::MoveTo(from), seg], widths: vec![start_width, width], color }
    }

    fn continues_at(&self, from: Pt, color: Color) -> bool {
        self.color == color && self.segments.last().and_then(Seg::end) == Some(from)
    }

    fn bounds(&self) -> Option<(Pt, Pt)> {
        padded_bounds(&self.segments, self.widths.iter().copied().fold(0.0, f64::max) / 2.0)
    }
}

/// What the canvas paints.
#[derive(Debug, Clone, PartialEq)]
pub enum Drawable {
    Primitive(Primitive),
    PenStroke(PenStroke),
}

impl Drawable {
    pub fn bounds(&self) -> Option<(Pt, Pt)> {
        match self {
            Drawable::Primitive(primitive) => primitive.bounds(),
            Drawable::PenStroke(stroke) => stroke.bounds(),
        }
    }
}

/// The engine draws a pressure stroke as one path per segment, each with its
/// own width, which is far too many paths to tessellate and paint every frame.
/// This turns runs of connected one-segment strokes of the same color back into
/// [`PenStroke`]s, keeping the painting order.
pub fn join_pen_strokes(primitives: Vec<Primitive>) -> Vec<Drawable> {
    let mut out = Vec::with_capacity(primitives.len());
    for primitive in primitives {
        let Some((from, seg, StrokeStyle { width, color })) = pen_segment(&primitive) else {
            out.push(Drawable::Primitive(primitive));
            continue;
        };
        match out.last_mut() {
            Some(Drawable::PenStroke(stroke)) if stroke.continues_at(from, color) => {
                if stroke.segments.len() > MAX_SEGMENTS_PER_CHUNK {
                    let end_width = stroke.widths.last().copied().unwrap_or(width);
                    out.push(Drawable::PenStroke(PenStroke::start(from, end_width, seg, width, color)));
                } else {
                    stroke.segments.push(seg);
                    stroke.widths.push(width);
                }
            }
            _ => out.push(Drawable::PenStroke(PenStroke::start(from, width, seg, width, color))),
        }
    }
    out
}

/// The start, the segment and the style of a stroke of exactly one segment.
fn pen_segment(primitive: &Primitive) -> Option<(Pt, Seg, StrokeStyle)> {
    match (primitive.fill, primitive.stroke, primitive.segments.as_slice()) {
        (None, Some(stroke), &[Seg::MoveTo(from), seg @ (Seg::LineTo(_) | Seg::CubicTo { .. })]) => {
            Some((from, seg, stroke))
        }
        _ => None,
    }
}

/// A 2D affine transform using the same convention as cairo and pizarra:
/// `x' = xx*x + xy*y + x0`, `y' = yx*x + yy*y + y0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub xx: f64,
    pub yx: f64,
    pub xy: f64,
    pub yy: f64,
    pub x0: f64,
    pub y0: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { xx: 1.0, yx: 0.0, xy: 0.0, yy: 1.0, x0: 0.0, y0: 0.0 };

    pub fn translation(dx: f64, dy: f64) -> Self {
        Affine { x0: dx, y0: dy, ..Self::IDENTITY }
    }

    /// Applies `self` first and then translates the result.
    pub fn then_translate(self, dx: f64, dy: f64) -> Self {
        Affine { x0: self.x0 + dx, y0: self.y0 + dy, ..self }
    }

    pub fn apply(&self, x: f64, y: f64) -> Pt {
        Pt::new(self.xx * x + self.xy * y + self.x0, self.yx * x + self.yy * y + self.y0)
    }

    fn apply_vec<T: Unit>(&self, p: Vec2D<T>) -> Pt {
        self.apply(p.x.val(), p.y.val())
    }

    /// How much lengths grow under this transform. Pizarra only ever zooms
    /// uniformly, rotates and translates, so this is exact for stroke widths.
    pub fn length_scale(&self) -> f64 {
        (self.xx * self.yy - self.xy * self.yx).abs().sqrt()
    }
}

impl From<Transform> for Affine {
    fn from(t: Transform) -> Self {
        Affine { xx: t.xx, yx: t.yx, xy: t.xy, yy: t.yy, x0: t.x0, y0: t.y0 }
    }
}

/// Converts commands to primitives.
///
/// * `world` maps world coordinates to target pixels (used by finished shapes).
/// * `screen` maps pizarra's screen coordinates to target pixels (used by
///   visual helpers); usually just a translation to the canvas origin.
pub fn primitives(commands: &[DrawCommand], world: Affine, screen: Affine) -> Vec<Primitive> {
    let mut out = Vec::with_capacity(commands.len());
    for command in commands {
        push_command(&mut out, command, world, screen);
    }
    out
}

fn push_command(out: &mut Vec<Primitive>, command: &DrawCommand, world: Affine, screen: Affine) {
    match command {
        DrawCommand::Path { commands, style } => {
            push_path(out, commands, style, world);
        }
        DrawCommand::ScreenPath { commands, style } => {
            push_path(out, commands, style, screen);
        }
        DrawCommand::Ellipse { ellipse, style } => {
            if let Some(segments) = ellipse_segments(ellipse, world) {
                push_primitive(out, segments, style, world);
            }
        }
        DrawCommand::ScreenCircle { center, radius, style } => {
            let segments = conic_segments(screen, center.x.val(), center.y.val(), radius.val(), radius.val(), 0.0);
            if let Some(segments) = segments {
                push_primitive(out, segments, style, screen);
            }
        }
    }
}

fn push_path<T: Unit + 'static>(out: &mut Vec<Primitive>, commands: &[PathCommand<T>], style: &Style<T>, t: Affine) {
    let mut segments = Vec::with_capacity(commands.len());
    for command in commands {
        if command.is_nan() {
            continue;
        }
        let seg = match *command {
            PathCommand::MoveTo(p) => Seg::MoveTo(t.apply_vec(p)),
            PathCommand::LineTo(p) => Seg::LineTo(t.apply_vec(p)),
            PathCommand::CurveTo(CubicBezierCurve { pt1, pt2, to }) => {
                Seg::CubicTo { c1: t.apply_vec(pt1), c2: t.apply_vec(pt2), to: t.apply_vec(to) }
            }
        };
        // Every path must start with a MoveTo for the tessellators.
        let seg = match (segments.is_empty(), seg) {
            (true, Seg::LineTo(p)) | (true, Seg::CubicTo { to: p, .. }) => Seg::MoveTo(p),
            (_, seg) => seg,
        };
        segments.push(seg);
    }

    if segments.is_empty() {
        return;
    }

    push_primitive(out, segments, style, t);
}

fn push_primitive<T: Unit>(out: &mut Vec<Primitive>, segments: Vec<Seg>, style: &Style<T>, t: Affine) {
    if segments.iter().flat_map(Seg::points).any(|p| !p.is_finite()) {
        log::warn!("skipping a shape with non-finite coordinates");
        return;
    }

    let fill = style.fill.filter(|c| c.a > 0);
    let stroke = style
        .stroke
        .map(|s| StrokeStyle { width: s.size.val() * t.length_scale(), color: s.color })
        .filter(|s| s.width > 0.0 && s.color.a > 0);

    if fill.is_some() {
        out.push(Primitive { segments: segments.clone(), fill, stroke: None });
    }

    let Some(stroke) = stroke else { return };

    // A click without dragging produces a zero-length path. Cairo and SVG draw
    // a round dot for it, but stroke tessellators produce nothing, so draw the
    // dot ourselves.
    let first = segments.iter().flat_map(Seg::points).next();
    if let Some(first) = first
        && segments.iter().flat_map(Seg::points).all(|p| p.distance(first) < 1e-6)
    {
        if let Some(dot) =
            conic_segments(Affine::IDENTITY, first.x, first.y, stroke.width / 2.0, stroke.width / 2.0, 0.0)
        {
            out.push(Primitive { segments: dot, fill: Some(stroke.color), stroke: None });
        }
        return;
    }

    for chunk in split_into_chunks(segments) {
        out.push(Primitive { segments: chunk, fill: None, stroke: Some(stroke) });
    }
}

/// Splits a long open path into several shorter ones that share their end
/// points. With round caps and joins the result looks the same.
fn split_into_chunks(segments: Vec<Seg>) -> Vec<Vec<Seg>> {
    if segments.len() <= MAX_SEGMENTS_PER_CHUNK || segments.contains(&Seg::Close) {
        return vec![segments];
    }

    let mut chunks = Vec::new();
    let mut current: Vec<Seg> = Vec::with_capacity(MAX_SEGMENTS_PER_CHUNK + 1);
    let mut last_point = None;

    for seg in segments {
        if current.len() >= MAX_SEGMENTS_PER_CHUNK && !matches!(seg, Seg::MoveTo(_)) {
            chunks.push(std::mem::take(&mut current));
            if let Some(p) = last_point {
                current.push(Seg::MoveTo(p));
            }
        }
        if let Some(p) = seg.end() {
            last_point = Some(p);
        }
        current.push(seg);
    }

    if current.iter().any(|s| !matches!(s, Seg::MoveTo(_))) {
        chunks.push(current);
    }

    chunks
}

fn ellipse_segments(e: &Ellipse<WorldUnit>, t: Affine) -> Option<Vec<Seg>> {
    conic_segments(t, e.center.x.val(), e.center.y.val(), e.semimajor.val(), e.semiminor.val(), e.angle.radians())
}

/// Four cubic Béziers approximating the ellipse centered at `(cx, cy)` with
/// semi-axes `a` (along `angle`) and `b`, mapped through `t`. An affine map of a
/// Bézier curve is the Bézier curve of the mapped control points, so this stays
/// exact under pizarra's rotations and zooms.
fn conic_segments(t: Affine, cx: f64, cy: f64, a: f64, b: f64, angle: f64) -> Option<Vec<Seg>> {
    if a == 0.0 || b == 0.0 || !a.is_finite() || !b.is_finite() {
        return None;
    }

    let (sin, cos) = angle.sin_cos();
    // Maps a point of the unit circle to the ellipse and then through `t`.
    let map = |u: f64, v: f64| {
        let (x, y) = (u * a, v * b);
        t.apply(cx + x * cos - y * sin, cy + x * sin + y * cos)
    };
    let k = KAPPA;

    Some(vec![
        Seg::MoveTo(map(1.0, 0.0)),
        Seg::CubicTo { c1: map(1.0, k), c2: map(k, 1.0), to: map(0.0, 1.0) },
        Seg::CubicTo { c1: map(-k, 1.0), c2: map(-1.0, k), to: map(-1.0, 0.0) },
        Seg::CubicTo { c1: map(-1.0, -k), c2: map(-k, -1.0), to: map(0.0, -1.0) },
        Seg::CubicTo { c1: map(k, -1.0), c2: map(1.0, -k), to: map(1.0, 0.0) },
        Seg::Close,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pizarra::{
        app::{MouseButton, Pizarra, SelectedTool},
        geom::Angle,
        point::ScreenUnit,
        shape::ShapeTool,
        style::Stroke,
        ui::Flags,
    };

    fn approx(a: Pt, b: Pt) -> bool {
        a.distance(b) < 1e-9
    }

    fn white_stroke<T: Unit>(size: f64) -> Style<T> {
        Style { fill: None, stroke: Some(Stroke { size: size.into(), color: Color::white() }) }
    }

    #[test]
    fn affine_matches_pizarra_transform() {
        let t = Transform::default_for_viewport(Vec2D::new_screen(800.0, 600.0))
            .zoom(2.0, Vec2D::new_screen(10.0, 20.0))
            .turn(Angle::from_degrees(30.0), Vec2D::new_screen(400.0, 300.0));
        let a = Affine::from(t);

        for (x, y) in [(0.0, 0.0), (12.5, -7.0), (-300.0, 42.0)] {
            let expected = t.to_screen_coordinates(Vec2D::new_world(x, y));
            assert!(approx(a.apply(x, y), Pt::new(expected.x.val(), expected.y.val())));
        }

        assert!((a.length_scale() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn world_paths_are_transformed_and_strokes_scaled() {
        let t = Affine { xx: 2.0, yy: 2.0, ..Affine::translation(10.0, 20.0) };
        let cmd = DrawCommand::Path {
            commands: vec![
                PathCommand::MoveTo(Vec2D::new_world(0.0, 0.0)),
                PathCommand::LineTo(Vec2D::new_world(5.0, 5.0)),
            ],
            style: white_stroke(3.0),
        };

        let prims = primitives(&[cmd], t, Affine::IDENTITY);
        assert_eq!(prims.len(), 1);
        assert_eq!(prims[0].segments, vec![Seg::MoveTo(Pt::new(10.0, 20.0)), Seg::LineTo(Pt::new(20.0, 30.0))]);
        assert_eq!(prims[0].stroke.unwrap().width, 6.0);
        assert_eq!(prims[0].fill, None);
    }

    #[test]
    fn screen_helpers_ignore_the_world_transform() {
        let world = Affine { xx: 3.0, yy: 3.0, ..Affine::IDENTITY };
        let screen = Affine::translation(0.0, 40.0);
        let cmd = DrawCommand::ScreenPath {
            commands: vec![
                PathCommand::MoveTo(Vec2D::new_screen(1.0, 1.0)),
                PathCommand::LineTo(Vec2D::new_screen(2.0, 2.0)),
            ],
            style: white_stroke::<ScreenUnit>(1.0),
        };

        let prims = primitives(&[cmd], world, screen);
        assert_eq!(prims[0].segments[0], Seg::MoveTo(Pt::new(1.0, 41.0)));
        assert_eq!(prims[0].stroke.unwrap().width, 1.0);
    }

    #[test]
    fn ellipse_points_lie_on_the_rotated_ellipse() {
        let e = Ellipse {
            center: Vec2D::new_world(5.0, -3.0),
            semimajor: 4.0.into(),
            semiminor: 2.0.into(),
            angle: Angle::from_degrees(90.0),
        };
        let segs = ellipse_segments(&e, Affine::IDENTITY).unwrap();

        // Rotated 90°: the major axis is vertical.
        assert!(approx(segs[0].end().unwrap(), Pt::new(5.0, 1.0)));
        assert!(approx(segs[1].end().unwrap(), Pt::new(3.0, -3.0)));
        assert!(approx(segs[2].end().unwrap(), Pt::new(5.0, -7.0)));
        assert!(approx(segs[3].end().unwrap(), Pt::new(7.0, -3.0)));
        assert_eq!(*segs.last().unwrap(), Seg::Close);
    }

    #[test]
    fn degenerate_ellipses_are_skipped() {
        let e = Ellipse {
            center: Vec2D::new_world(0.0, 0.0),
            semimajor: 4.0.into(),
            semiminor: 0.0.into(),
            angle: Angle::from_degrees(0.0),
        };
        let cmd = DrawCommand::Ellipse { ellipse: e, style: white_stroke(1.0) };
        assert!(primitives(&[cmd], Affine::IDENTITY, Affine::IDENTITY).is_empty());
    }

    #[test]
    fn a_single_click_becomes_a_dot() {
        let cmd = DrawCommand::Path {
            commands: vec![
                PathCommand::MoveTo(Vec2D::new_world(3.0, 3.0)),
                PathCommand::LineTo(Vec2D::new_world(3.0, 3.0)),
            ],
            style: white_stroke(4.0),
        };
        let prims = primitives(&[cmd], Affine::IDENTITY, Affine::IDENTITY);

        assert_eq!(prims.len(), 1);
        assert_eq!(prims[0].stroke, None);
        assert_eq!(prims[0].fill, Some(Color::white()));
        let (min, max) = prims[0].bounds().unwrap();
        assert!((max.x - min.x - 4.0).abs() < 0.5);
    }

    #[test]
    fn fill_and_stroke_produce_two_primitives_fill_first() {
        let style =
            Style { fill: Some(Color::red()), stroke: Some(Stroke { size: 1.0.into(), color: Color::white() }) };
        let cmd = DrawCommand::ScreenCircle { center: Vec2D::new_screen(0.0, 0.0), radius: 5.0.into(), style };
        let prims = primitives(&[cmd], Affine::IDENTITY, Affine::IDENTITY);

        assert_eq!(prims.len(), 2);
        assert!(prims[0].fill.is_some() && prims[0].stroke.is_none());
        assert!(prims[1].stroke.is_some() && prims[1].fill.is_none());
    }

    #[test]
    fn long_paths_are_chunked_without_gaps() {
        let mut commands = vec![PathCommand::MoveTo(Vec2D::new_world(0.0, 0.0))];
        for i in 1..=1000 {
            commands.push(PathCommand::LineTo(Vec2D::new_world(i as f64, (i % 7) as f64)));
        }
        let cmd = DrawCommand::Path { commands, style: white_stroke(2.0) };
        let prims = primitives(&[cmd], Affine::IDENTITY, Affine::IDENTITY);

        assert!(prims.len() > 1);
        for pair in prims.windows(2) {
            let prev_end = pair[0].segments.last().unwrap().end().unwrap();
            assert_eq!(pair[1].segments[0], Seg::MoveTo(prev_end));
            assert!(pair[0].segments.len() <= MAX_SEGMENTS_PER_CHUNK + 1);
        }
        let drawn: usize =
            prims.iter().map(|p| p.segments.iter().filter(|s| matches!(s, Seg::LineTo(_))).count()).sum();
        assert_eq!(drawn, 1000);
    }

    fn segment(from: (f64, f64), to: (f64, f64), width: f64, color: Color) -> Primitive {
        Primitive {
            segments: vec![Seg::MoveTo(Pt::new(from.0, from.1)), Seg::LineTo(Pt::new(to.0, to.1))],
            fill: None,
            stroke: Some(StrokeStyle { width, color }),
        }
    }

    fn pen_strokes(drawables: &[Drawable]) -> Vec<&PenStroke> {
        drawables
            .iter()
            .map(|d| match d {
                Drawable::PenStroke(stroke) => stroke,
                other => panic!("expected a pen stroke, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_pressure_stroke_from_the_engine_becomes_one_pen_stroke() {
        let mut controller = Pizarra::new(Vec2D::new_screen(800.0, 600.0), Default::default());
        controller.set_tool(SelectedTool::Shape(ShapeTool::Path));
        let at = |i: u32| Vec2D::new_screen(50.0 + f64::from(i) * 3.0, 100.0 + f64::from(i % 5));

        let moves = 100;
        controller.handle_mouse_button_pressed_pressure(MouseButton::Left, at(0), 0.1, None);
        for i in 1..=moves {
            let pressure = f64::from(i) / f64::from(moves);
            controller.handle_mouse_move_pressure(at(i), pressure, Flags::default(), None);
        }
        controller.handle_mouse_button_released_pressure(MouseButton::Left, at(moves), 0.0, Flags::default(), None);

        let commands = controller.draw_commands_for_screen();
        assert_eq!(commands.len(), moves as usize, "the engine draws one path per segment");

        // Seams must still meet exactly after a transform.
        let world = Affine { xx: 1.7, yx: 0.3, xy: -0.3, yy: 1.7, x0: 13.1, y0: 7.7 };
        let drawables = join_pen_strokes(primitives(&commands, world, Affine::IDENTITY));
        let strokes = pen_strokes(&drawables);
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].segments.len(), moves as usize + 1);
        assert_eq!(strokes[0].widths.len(), strokes[0].segments.len());
        let (first, last) = (strokes[0].widths[0], *strokes[0].widths.last().unwrap());
        assert!(first < last, "pressure grows along the stroke: {first} -> {last}");
    }

    #[test]
    fn only_connected_segments_of_one_color_are_joined() {
        let (white, black) = (Color::white(), Color::black());
        let drawables = join_pen_strokes(vec![
            segment((0.0, 0.0), (1.0, 0.0), 1.0, white),
            segment((1.0, 0.0), (2.0, 0.0), 3.0, white),
            // Same color, but it starts somewhere else.
            segment((5.0, 5.0), (6.0, 5.0), 1.0, white),
            // Connected, but in another color.
            segment((6.0, 5.0), (7.0, 5.0), 1.0, black),
        ]);

        let strokes = pen_strokes(&drawables);
        assert_eq!(strokes.len(), 3);
        assert_eq!(strokes[0].widths, [1.0, 1.0, 3.0]);
        assert_eq!(strokes[1].segments.len(), 2);
        assert_eq!(strokes[2].color, black);
    }

    #[test]
    fn other_primitives_stay_as_they_are_and_in_order() {
        let white = Color::white();
        let filled = Primitive {
            segments: vec![Seg::MoveTo(Pt::new(0.0, 0.0)), Seg::LineTo(Pt::new(1.0, 0.0))],
            fill: Some(white),
            stroke: None,
        };
        let long = Primitive {
            segments: vec![
                Seg::MoveTo(Pt::new(0.0, 0.0)),
                Seg::LineTo(Pt::new(1.0, 0.0)),
                Seg::LineTo(Pt::new(1.0, 1.0)),
            ],
            fill: None,
            stroke: Some(StrokeStyle { width: 2.0, color: white }),
        };

        let drawables = join_pen_strokes(vec![
            segment((0.0, 0.0), (1.0, 0.0), 1.0, white),
            filled.clone(),
            segment((1.0, 0.0), (2.0, 0.0), 1.0, white),
            long.clone(),
        ]);

        assert_eq!(drawables.len(), 4);
        assert!(matches!(drawables[0], Drawable::PenStroke(_)));
        assert_eq!(drawables[1], Drawable::Primitive(filled));
        assert!(matches!(drawables[2], Drawable::PenStroke(_)));
        assert_eq!(drawables[3], Drawable::Primitive(long));
    }

    #[test]
    fn long_pen_strokes_are_chunked_without_gaps_in_position_or_width() {
        let count = 1000;
        let width = |i: usize| 1.0 + (i % 11) as f64;
        let drawables = join_pen_strokes(
            (0..count).map(|i| segment((i as f64, 0.0), ((i + 1) as f64, 0.0), width(i), Color::white())).collect(),
        );

        let strokes = pen_strokes(&drawables);
        assert!(strokes.len() > 1);
        for pair in strokes.windows(2) {
            assert!(pair[0].segments.len() <= MAX_SEGMENTS_PER_CHUNK + 1);
            let end = pair[0].segments.last().unwrap().end().unwrap();
            assert_eq!(pair[1].segments[0], Seg::MoveTo(end));
            assert_eq!(pair[1].widths[0], *pair[0].widths.last().unwrap());
        }
        let drawn: usize = strokes.iter().map(|s| s.segments.len() - 1).sum();
        assert_eq!(drawn, count);
    }

    #[test]
    fn pen_stroke_bounds_fit_its_widest_point() {
        let (white, width) = (Color::white(), 6.0);
        let drawables = join_pen_strokes(vec![
            segment((0.0, 0.0), (10.0, 0.0), width, white),
            segment((10.0, 0.0), (20.0, 0.0), 1.0, white),
        ]);

        let (min, max) = drawables[0].bounds().unwrap();
        assert_eq!((min, max), (Pt::new(-3.0, -3.0), Pt::new(23.0, 3.0)));
    }

    #[test]
    fn non_finite_paths_are_dropped_and_leading_lines_become_moves() {
        let cmd = DrawCommand::Path {
            commands: vec![
                PathCommand::LineTo(Vec2D::new_world(f64::NAN, 0.0)),
                PathCommand::LineTo(Vec2D::new_world(1.0, 1.0)),
                PathCommand::LineTo(Vec2D::new_world(2.0, 1.0)),
            ],
            style: white_stroke(1.0),
        };
        let prims = primitives(&[cmd], Affine::IDENTITY, Affine::IDENTITY);
        assert_eq!(prims[0].segments[0], Seg::MoveTo(Pt::new(1.0, 1.0)));
    }
}
