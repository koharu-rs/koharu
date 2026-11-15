//! A text layout drawn: its glyph runs filled, over their strokes, with a
//! face's synthetic bold and slant.

use anyhow::Result;
use glifo::FontEmbolden;
use kurbo::{Affine, Diagonal2, Join};
use vello_gpu::peniko::Color;

use super::TextLayout;
use crate::Drawing;

/// Draws `layout`'s glyph runs onto `drawing` through `transform`, its
/// strokes too when `strokes` says so. Strokes draw first, under every fill,
/// as Photoshop draws an Outside stroke under its type: as the glyphs grown
/// by the stroke's width with round joins.
pub(crate) fn draw(
    drawing: &mut Drawing,
    layout: &TextLayout,
    transform: Affine,
    strokes: bool,
) -> Result<()> {
    drawing.scene.set_transform(transform);
    let passes: &[bool] = if strokes { &[true, false] } else { &[false] };
    for &stroking in passes {
        for (glyphs, run) in layout.runs.iter().zip(&layout.resolved.runs) {
            let (paint, width, join) = match (stroking, run.stroke) {
                (true, Some((color, width))) => (color, f64::from(width), Join::Round),
                (true, None) => continue,
                (false, _) => (run.fill, 0.0, Join::Miter),
            };
            let font = &glyphs.font;
            // A synthetic bold grows each side by a fiftieth of the size.
            let bold = match font.synthesis.embolden() {
                true => f64::from(run.size) * 0.02,
                false => 0.0,
            };
            // In the glyphs' own space, where y runs up: a quarter turn
            // clockwise on the page for sideways text, which maps (x, y) to
            // (y, -x), after a synthetic slant.
            let skew = font
                .synthesis
                .skew()
                .map(|angle| Affine::skew(-f64::from(angle.to_radians().tan()), 0.0));
            let turn = match glyphs.sideways {
                true => Some(
                    Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]) * skew.unwrap_or(Affine::IDENTITY),
                ),
                false => skew,
            };
            drawing.scene.set_paint(Color::from(paint));
            drawing.glyphs(
                font,
                run.size,
                turn,
                FontEmbolden::new(Diagonal2::new(bold + width, bold + width)).with_join(join),
                &glyphs.glyphs,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use image::RgbaImage;
    use koharu_scene::{CharStyle, Rgba, Text, TextStyle};

    use super::*;
    use crate::{Gpu, fonts::Library};

    #[test]
    fn strokes_widen_a_text_and_draw_under_its_fill() -> Result<()> {
        let mut library = Library::new();
        let font = ["Arial", "DejaVu Sans", "Helvetica"]
            .into_iter()
            .find_map(|family| library.family(family).ok())
            .expect("an installed sans-serif");
        let laid = |stroke_width| -> Result<TextLayout> {
            let text = Text {
                content: "Stroked".into(),
                style: TextStyle {
                    chars: CharStyle {
                        stroke: Some(Rgba([1.0, 0.0, 0.0, 1.0])),
                        stroke_width,
                        ..CharStyle::default()
                    },
                    ..TextStyle::default()
                },
                ..Text::default()
            };
            Ok(crate::text::layout(&text, |_, _| Ok(vec![font.clone()]))?.expect("a layout"))
        };
        let (plain, stroked) = (laid(None)?, laid(Some(3.0))?);
        assert_eq!(stroked.bounds, plain.bounds.inflate(3.0, 3.0));
        let Ok(gpu) = Gpu::get() else {
            eprintln!("skipped drawing without a GPU");
            return Ok(());
        };
        // Drawn on white, the stroked text is red around a black fill.
        let draw = |layout: &TextLayout| -> Result<RgbaImage> {
            let size = layout.bounds.size();
            let mut drawing = gpu.drawing(size.width.ceil() as u16, size.height.ceil() as u16);
            draw(
                &mut drawing,
                layout,
                Affine::translate((-layout.bounds.x0, -layout.bounds.y0)),
                true,
            )?;
            drawing.read()
        };
        let red = |image: &RgbaImage| {
            image
                .pixels()
                .filter(|pixel| pixel.0[0] > 200 && pixel.0[1] < 60 && pixel.0[3] > 200)
                .count()
        };
        let (plain, stroked) = (draw(&plain)?, draw(&stroked)?);
        assert_eq!(red(&plain), 0);
        assert!(red(&stroked) > 100, "{} red pixels", red(&stroked));
        assert!(
            stroked
                .pixels()
                .any(|pixel| pixel.0[..3] == [0, 0, 0] && pixel.0[3] == 255)
        );
        Ok(())
    }
}
