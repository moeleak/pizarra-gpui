//! Renderer-agnostic geometry.
//!
//! The pizarra engine describes what to draw as [`DrawCommand`]s: some in
//! world coordinates (finished shapes) and some in screen coordinates (visual
//! helpers such as snapping circles). This module flattens those commands into
//! [`Primitive`]s whose segments are already in the target pixel space, so the
//! same code feeds both the GPUI canvas and the PNG exporter.

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
        let mut points = self.segments.iter().flat_map(Seg::points);
        let first = points.next()?;
        let (mut min, mut max) = (first, first);
        for p in points {
            min = Pt::new(min.x.min(p.x), min.y.min(p.y));
            max = Pt::new(max.x.max(p.x), max.y.max(p.y));
        }
        let pad = self.stroke.map_or(0.0, |s| s.width / 2.0);
        Some((Pt::new(min.x - pad, min.y - pad), Pt::new(max.x + pad, max.y + pad)))
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
    use pizarra::{geom::Angle, point::ScreenUnit, style::Stroke};

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
