//! Turns geometry primitives into GPUI paths and keeps the finished shapes
//! tessellated between frames.
//!
//! The GTK frontend rendered stored shapes into an offscreen cairo surface and
//! only repainted it when pizarra answered `ShouldRedraw::All`. The equivalent
//! here is [`ShapeLayer`]: the tessellated paths of the stored shapes, rebuilt
//! only when invalidated or when the canvas moves or resizes. The shape being
//! drawn and the tool helpers are cheap and rebuilt every frame.

use gpui::{Bounds, Hsla, Path, PathBuilder, PathStyle, Pixels, Window, point, px};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, LineCap, LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use pizarra::draw_commands::DrawCommand;

use crate::geometry::{self, Affine, Drawable, PenStroke, Primitive, Seg};
use crate::theme;

/// Maximum distance, in logical pixels, between a flattened curve and the real
/// one.
const TOLERANCE: f32 = 0.1;

pub type PaintedPaths = Vec<(Path<Pixels>, Hsla)>;

/// Maps pizarra's world and screen coordinates to window coordinates for a
/// canvas whose top-left corner is at `bounds.origin`.
pub fn transforms(controller: &pizarra::app::Pizarra, bounds: Bounds<Pixels>) -> (Affine, Affine) {
    let (ox, oy) = (f64::from(bounds.origin.x), f64::from(bounds.origin.y));
    let world = Affine::from(controller.get_transform()).then_translate(ox, oy);
    let screen = Affine::translation(ox, oy);
    (world, screen)
}

pub fn build(commands: &[DrawCommand], world: Affine, screen: Affine, viewport: Bounds<Pixels>) -> PaintedPaths {
    let mut out = Vec::new();
    for drawable in geometry::join_pen_strokes(geometry::primitives(commands, world, screen)) {
        if is_visible(&drawable, viewport) {
            match &drawable {
                Drawable::Primitive(primitive) => push_paths(&mut out, primitive),
                Drawable::PenStroke(stroke) => push_pen_stroke(&mut out, stroke),
            }
        }
    }
    out
}

pub fn paint(window: &mut Window, paths: &PaintedPaths) {
    for (path, color) in paths {
        window.paint_path(path.clone(), *color);
    }
}

fn is_visible(drawable: &Drawable, viewport: Bounds<Pixels>) -> bool {
    let Some((min, max)) = drawable.bounds() else { return false };
    let (left, top) = (f64::from(viewport.origin.x), f64::from(viewport.origin.y));
    let (right, bottom) = (left + f64::from(viewport.size.width), top + f64::from(viewport.size.height));
    max.x >= left && min.x <= right && max.y >= top && min.y <= bottom
}

fn push_paths(out: &mut PaintedPaths, primitive: &Primitive) {
    if let Some(color) = primitive.fill {
        let style = PathStyle::Fill(FillOptions::non_zero().with_tolerance(TOLERANCE));
        if let Some(path) = build_path(PathBuilder::fill().with_style(style), &primitive.segments) {
            out.push((path, theme::to_hsla(color)));
        }
    }

    if let Some(stroke) = primitive.stroke {
        let width = stroke.width as f32;
        let options = StrokeOptions::default()
            .with_line_width(width)
            .with_line_cap(LineCap::Round)
            .with_line_join(LineJoin::Round)
            .with_tolerance(TOLERANCE);
        let builder = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options));
        if let Some(path) = build_path(builder, &primitive.segments) {
            out.push((path, theme::to_hsla(stroke.color)));
        }
    }
}

/// Tessellates with lyon directly because `PathBuilder` has no per-point width.
fn push_pen_stroke(out: &mut PaintedPaths, stroke: &PenStroke) {
    const WIDTH: usize = 0;

    let p = |pt: geometry::Pt| lyon::math::point(pt.x as f32, pt.y as f32);
    let mut builder = lyon::path::Path::builder_with_attributes(1);
    for (seg, width) in stroke.segments.iter().zip(&stroke.widths) {
        let attributes = [*width as f32];
        match *seg {
            Seg::MoveTo(to) => {
                builder.begin(p(to), &attributes);
            }
            Seg::LineTo(to) => {
                builder.line_to(p(to), &attributes);
            }
            Seg::CubicTo { c1, c2, to } => {
                builder.cubic_bezier_to(p(c1), p(c2), p(to), &attributes);
            }
            Seg::Close => {}
        }
    }
    builder.end(false);

    let options = StrokeOptions::default()
        .with_variable_line_width(WIDTH)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round)
        .with_tolerance(TOLERANCE);
    let mut buffers: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
    let mut output = BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex| vertex.position());
    match StrokeTessellator::new().tessellate_path(&builder.build(), &options, &mut output) {
        Ok(_) => out.push((PathBuilder::build_path(buffers), theme::to_hsla(stroke.color))),
        Err(err) => log::warn!("could not tessellate a pen stroke: {err}"),
    }
}

fn build_path(mut builder: PathBuilder, segments: &[Seg]) -> Option<Path<Pixels>> {
    let p = |pt: geometry::Pt| point(px(pt.x as f32), px(pt.y as f32));

    for seg in segments {
        match *seg {
            Seg::MoveTo(to) => builder.move_to(p(to)),
            Seg::LineTo(to) => builder.line_to(p(to)),
            Seg::CubicTo { c1, c2, to } => builder.cubic_bezier_to(p(to), p(c1), p(c2)),
            Seg::Close => builder.close(),
        }
    }

    match builder.build() {
        Ok(path) => Some(path),
        Err(err) => {
            log::warn!("could not tessellate a shape: {err}");
            None
        }
    }
}

/// Tessellated stored shapes, valid for one canvas position, size and view
/// transform.
#[derive(Default)]
pub struct ShapeLayer {
    built_for: Option<Bounds<Pixels>>,
    paths: PaintedPaths,
}

impl ShapeLayer {
    pub fn invalidate(&mut self) {
        self.built_for = None;
    }

    pub fn paths(&mut self, controller: &pizarra::app::Pizarra, bounds: Bounds<Pixels>) -> &PaintedPaths {
        if self.built_for != Some(bounds) {
            let (world, screen) = transforms(controller, bounds);
            self.paths = build(&controller.draw_commands_for_screen(), world, screen, bounds);
            self.built_for = Some(bounds);
        }
        &self.paths
    }
}
