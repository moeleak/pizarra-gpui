//! PNG export, rendered on the CPU with tiny-skia.
//!
//! Like the GTK frontend, the image covers the bounding box of the whole
//! drawing plus the configured `export_padding`, at one pixel per world unit.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use pizarra::{Pizarra, color::Color, point::Unit};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::geometry::{self, Affine, Primitive, Seg};

/// Refuse absurdly large exports instead of trying to allocate them.
const MAX_SIDE: f64 = 16_384.0;

/// Everything needed to rasterize the drawing. Plain data, so it can be sent
/// to a background thread while the UI stays responsive.
pub struct ExportJob {
    width: u32,
    height: u32,
    background: Color,
    primitives: Vec<Primitive>,
}

impl ExportJob {
    pub fn new(controller: &Pizarra) -> Result<Self> {
        let Some([a, b]) = controller.get_bounds() else {
            bail!("el dibujo está vacío");
        };

        let padding = controller.config().export_padding.val();
        let (min_x, min_y) = (a.x.val().min(b.x.val()) - padding, a.y.val().min(b.y.val()) - padding);
        let (max_x, max_y) = (a.x.val().max(b.x.val()) + padding, a.y.val().max(b.y.val()) + padding);
        let (width, height) = ((max_x - min_x).ceil(), (max_y - min_y).ceil());

        if !(width.is_finite() && height.is_finite()) || width > MAX_SIDE || height > MAX_SIDE {
            bail!("el dibujo es demasiado grande para exportarlo ({width}×{height} px)");
        }

        let world = Affine::translation(-min_x, -min_y);
        let commands = controller.draw_commands_for_drawing();

        Ok(Self {
            width: width.max(1.0) as u32,
            height: height.max(1.0) as u32,
            background: controller.bgcolor(),
            primitives: geometry::primitives(&commands, world, Affine::IDENTITY),
        })
    }

    fn render(&self) -> Result<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, self.height).context("could not allocate the image")?;
        pixmap.fill(skia_color(self.background));
        for primitive in &self.primitives {
            paint(&mut pixmap, primitive);
        }
        Ok(pixmap)
    }

    pub fn render_png(&self) -> Result<Vec<u8>> {
        self.render()?.encode_png().context("encoding the PNG")
    }

    pub fn write_png(&self, path: &Path) -> Result<()> {
        let png = self.render_png()?;
        crate::files::write_atomically(path, &png)
    }
}

fn paint(pixmap: &mut Pixmap, primitive: &Primitive) {
    let Some(path) = skia_path(&primitive.segments) else { return };

    if let Some(color) = primitive.fill {
        pixmap.fill_path(&path, &skia_paint(color), FillRule::Winding, Transform::identity(), None);
    }

    if let Some(stroke) = primitive.stroke {
        let style = Stroke {
            width: stroke.width as f32,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &skia_paint(stroke.color), &style, Transform::identity(), None);
    }
}

fn skia_path(segments: &[Seg]) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    for seg in segments {
        match *seg {
            Seg::MoveTo(p) => builder.move_to(p.x as f32, p.y as f32),
            Seg::LineTo(p) => builder.line_to(p.x as f32, p.y as f32),
            Seg::CubicTo { c1, c2, to } => {
                builder.cubic_to(c1.x as f32, c1.y as f32, c2.x as f32, c2.y as f32, to.x as f32, to.y as f32)
            }
            Seg::Close => builder.close(),
        }
    }
    builder.finish()
}

fn skia_paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(skia_color(color));
    paint.anti_alias = true;
    paint
}

fn skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pizarra::{MouseButton, Vec2D, config::Config};

    fn drawing_with_a_stroke() -> Pizarra {
        let config = Config { background_color: Color::black(), stroke_color: Color::white(), ..Config::default() };
        let mut p = Pizarra::new(Vec2D::new_screen(400.0, 300.0), config);
        p.set_stroke(4.0.into());
        p.handle_mouse_button_pressed(MouseButton::Left, Vec2D::new_screen(100.0, 100.0));
        for i in 1..=20 {
            p.handle_mouse_move(Vec2D::new_screen(100.0 + i as f64 * 5.0, 100.0 + i as f64 * 2.0));
        }
        p.handle_mouse_button_released(MouseButton::Left, Vec2D::new_screen(200.0, 140.0));
        p
    }

    #[test]
    fn empty_drawings_cannot_be_exported() {
        let p = Pizarra::new(Vec2D::new_screen(100.0, 100.0), Config::default());
        assert!(ExportJob::new(&p).is_err());
    }

    #[test]
    fn exports_the_bounding_box_plus_padding() {
        let p = drawing_with_a_stroke();
        let job = ExportJob::new(&p).unwrap();
        let pixmap = job.render().unwrap();
        let padding = p.config().export_padding.val();

        let [a, b] = p.get_bounds().unwrap();
        let expected_w = ((a.x.val() - b.x.val()).abs() + 2.0 * padding).ceil() as u32;
        assert_eq!(pixmap.width(), expected_w);

        // Corners are background, the stroke is somewhere in the middle.
        let corner = pixmap.pixel(0, 0).unwrap();
        assert_eq!((corner.red(), corner.green(), corner.blue()), (0, 0, 0));
        let has_white = pixmap.pixels().iter().any(|px| px.red() > 200 && px.green() > 200 && px.blue() > 200);
        assert!(has_white, "the stroke should be painted");

        let png = job.render_png().unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
